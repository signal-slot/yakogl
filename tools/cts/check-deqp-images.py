#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Do the dEQP-GLES2 image recipes still build what conformance needs?

  tools/cts/check-deqp-images.py                  the recipes in this checkout
  tools/cts/check-deqp-images.py --config <.config> [--ms4] [--gles3]
                                                  one built image's Kconfig result
  tools/cts/check-deqp-images.py --self-test

The first form needs no Zephyr workspace: it reads the sample configs
(driver/samples/zephyr/deqp, the per-board files merged the way
Zephyr merges them), the Kconfig files and the tables, and fails when

  - a CONFIG_ symbol of this project that a recipe, table or builder script
    names is not defined by any Kconfig file (a symbol renamed or removed);
  - a board's merged configuration does not say the in-process compiler, or
    sets a precompiled-only / precompiled / offline-table back end, ES 3.0
    reporting, or leaves the deqp Kconfig's REPORT_ES2 off;
  - the library's own defaults moved away from conformance (REPORT_ES2 not
    default y, the compiler not the choice's default, a precompiled tier or
    test knob on by default);
  - the sample's sources call a test knob (gc_/v3d_/pvr_debug_*) outside an
    `#if defined(CONFIG_X)` block of a deqp Kconfig probe symbol (default n,
    depending on DEQP_TEST_KNOBS), or a board's configuration sets X=y;
  - the memory carve-outs differ from the ones of the last passing sweeps
    (Pi 4: 48 MiB / 16 MiB of tiled copies, and texture-2048.conf's 72 / 24
    MiB for image T5, 64 in sweep 28 and 72 since 6d2e7d0d's tile memory,
    tests/v3d_es2ctx_t5_mem.rs; i.MX8MP: 160 MiB heap; AM62P: 256 MiB GPU carve-out),
    or the T6's is under 64 MiB (no passing sweep yet: the size the 2048x2048
    textures need, as image T5);
  - a table no longer selects every case of gles2-mustpass.txt, or the AM62P
    multisample table is not the rgba8888d24s8ms4 image;
  - a dEQP-GLES3 table (am62p-gles3-tree.txt, pi4-gles3-tree.txt) does not
    select every case of gles3-mustpass.txt exactly once (check-gles3-tree.py,
    whose verdict line is printed as a note).

With --config it checks the .config a build wrote: the same conformance rules
on the result of Kconfig itself (the builders run it after each image).
--gles3 is a dEQP-GLES3 image's rule (pi4-deqp-images.sh DEQP_MODULE=gles3):
the compiler and no table as before, and both REPORT_ES2 symbols off, since
dEQP-GLES3 judges a context that reports ES 3.0; without it a resolved
configuration that reports ES 3.0 is refused as before.
"""
import fnmatch
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEQP = os.path.join(REPO, 'driver/samples/zephyr/deqp')
OURS = re.compile(r'^(YAKOGL|DEQP|V3D|GC_|PVR|GL_TEST)')
# back ends / modes a conformance image must not select
FORBIDDEN_ON = ('YAKOGL_SHADERC_PRECOMPILED_ONLY', 'YAKOGL_PRECOMPILED_SHADERS', 'YAKOGL_SHADERC_OFFLINE')


def read(p):
    with open(p, errors='replace') as f:
        return f.read()


def parse_conf(path):
    """CONFIG_X=v lines of a prj.conf / fragment -> {X: v}."""
    d = {}
    for l in read(path).splitlines():
        m = re.match(r'\s*CONFIG_([A-Z0-9_]+)\s*=\s*(.*?)\s*$', l)
        if m:
            d[m.group(1)] = m.group(2).strip('"')
    return d


def parse_dotconfig(path):
    d = {}
    for l in read(path).splitlines():
        m = re.match(r'CONFIG_([A-Z0-9_]+)=(.*)$', l)
        if m:
            d[m.group(1)] = m.group(2).strip('"')
    return d


def kconfig_symbols():
    syms = {}
    for dp, dn, fn in os.walk(REPO):
        dn[:] = [x for x in dn if x not in ('.git', 'target', 'build', 'node_modules')]
        for f in fn:
            if f.startswith('Kconfig'):
                p = os.path.join(dp, f)
                for l in read(p).splitlines():
                    m = re.match(r'\s*(?:menu)?config\s+([A-Z0-9_]+)', l)
                    if m:
                        syms.setdefault(m.group(1), os.path.relpath(p, REPO))
    return syms


def kconfig_block(sym):
    """The text of `config SYM` up to the next config/menu/choice line."""
    out, on = [], False
    for p in (os.path.join(REPO, 'zephyr/Kconfig'), os.path.join(DEQP, 'Kconfig')):
        for l in read(p).splitlines():
            if re.match(r'\s*config\s+' + sym + r'\s*$', l):
                on = True
                out.append(l)
                continue
            if on and re.match(r'\s*(config|menuconfig|menu|endmenu|choice|endchoice|comment|if|endif)\b', l):
                on = False
            if on:
                out.append(l)
    return '\n'.join(out)


def conformance_rules(c, name, gles3=False, ms4=None):
    """Rules on one resolved configuration C (dict) -> list of problems."""
    bad = []
    if c.get('YAKOGL_SHADERC_ONLINE') != 'y':
        bad.append("%s: YAKOGL_SHADERC_ONLINE is not y (the compiler is conformance's back end)" % name)
    for s in FORBIDDEN_ON:
        if c.get(s) == 'y':
            bad.append('%s: %s=y (conformance runs the in-process compiler, no table)' % (name, s))
    if not gles3:
        if c.get('YAKOGL_REPORT_ES2') == 'n':
            bad.append('%s: YAKOGL_REPORT_ES2=n (dEQP-GLES2 judges an ES 2.0 context)' % name)
        if c.get('DEQP_REPORT_ES2') == 'n':
            bad.append('%s: DEQP_REPORT_ES2=n' % name)
    elif c.get('YAKOGL_REPORT_ES2') == 'y':
        # DEQP_REPORT_ES2=n only drops the select; the library's own default
        # (y) stays unless the image says -DCONFIG_YAKOGL_REPORT_ES2=n
        bad.append('%s: YAKOGL_REPORT_ES2=y (dEQP-GLES3 needs an ES 3.0 context)' % name)
    gl = c.get('DEQP_GL_CONFIG_NAME', '')
    if ms4 is True and gl != 'rgba8888d24s8ms4':
        bad.append("%s: DEQP_GL_CONFIG_NAME is '%s', the multisample image needs rgba8888d24s8ms4" % (name, gl))
    if ms4 is False and gl:
        bad.append("%s: DEQP_GL_CONFIG_NAME is '%s': a sweep image runs on the default config" % (name, gl))
    return bad


def check_dotconfig(path, ms4, gles3=False):
    c = parse_dotconfig(path)
    bad = conformance_rules(c, path, gles3=gles3, ms4=ms4)
    for s in ('YAKOGL_REPORT_ES2', 'DEQP_REPORT_ES2'):
        if gles3:
            # a dEQP-GLES3 image must report ES 3.0: dEQP's GLES3 context
            # queries GL_NUM_EXTENSIONS at start and dies on INVALID_ENUM
            if c.get(s) == 'y':
                bad.append('%s: CONFIG_%s=y (a dEQP-GLES3 image needs the context to report ES 3.0)' % (path, s))
        elif c.get(s) != 'y':
            bad.append('%s: CONFIG_%s=y is missing from the resolved configuration' % (path, s))
    return bad


def table_lines(path):
    """[(label or None, marker text, filter)] of a tools/cts table."""
    out, lab, mark = [], None, ''
    for l in read(path).splitlines():
        m = re.match(r'# image ([A-Za-z0-9]+):(.*)$', l)
        if m:
            lab, mark = m.group(1), m.group(2)
            continue
        if re.match(r'\s*(#|$)', l):
            continue
        out.append((lab, mark, l.strip()))
        lab, mark = None, ''
    return out


def selects(filt, name):
    for pat in filt.split(','):
        if fnmatch.fnmatchcase(name, pat.strip()):
            return True
    return False


KNOB = re.compile(r'\b(gc_debug_set_\w+|v3d_debug_\w+|gc_debug_\w+|pvr_debug_\w+)\s*\(')


def knob_calls(text):
    """[(guards, name)] for each test-knob call or declaration in C/C++ TEXT.
    guards is the CONFIG_ symbols of the enclosing `#if defined(CONFIG_X)` /
    `#ifdef CONFIG_X` blocks (their #else / #elif branches are not guarded)."""
    out, stack = [], []
    for l in text.splitlines():
        d = re.match(r'\s*#\s*(if|ifdef|ifndef|elif|else|endif)\b(.*)$', l)
        if d:
            kw, rest = d.group(1), d.group(2).strip()
            m = re.fullmatch(r'defined\s*\(\s*CONFIG_(\w+)\s*\)' if kw == 'if' else r'CONFIG_(\w+)', rest)
            if kw in ('if', 'ifdef'):
                stack.append(m.group(1) if m else None)
            elif kw == 'ifndef':
                stack.append(None)
            elif kw in ('elif', 'else') and stack:
                stack[-1] = None
            elif kw == 'endif' and stack:
                stack.pop()
            continue
        for m in KNOB.finditer(l):
            out.append(([g for g in stack if g], m.group(1)))
    return out


def probe_symbol(sym, dq_text, block_of):
    """Is SYM a probe symbol of the deqp sample's Kconfig: defined in that file
    (its text DQ_TEXT), `default n` and depending on DEQP_TEST_KNOBS in its
    Kconfig block BLOCK_OF(SYM)? A test knob called only under such a symbol
    compiles out of every image the sample builds as-is; knob_verdicts still
    refuses it when a conformance configuration turns the symbol on."""
    b = block_of(sym) or ''
    return (re.search(r'default n', b) is not None
            and re.search(r'depends on [^\n]*\bDEQP_TEST_KNOBS\b', b) is not None
            and re.search(r'^config\s+%s\s*$' % sym, dq_text, re.M) is not None)


def knob_verdicts(text, rel, dq_text, block_of, confs):
    """Test-knob calls of one source file TEXT (path REL in the tree) ->
    (problems, accepted [(knob, guard symbol)], each listed once).
    A call is accepted when one of its guard symbols X is a probe symbol of the
    deqp Kconfig (probe_symbol) and no configuration in CONFS (name -> merged
    CONFIG_ dict) sets CONFIG_X=y; anything else stays refused with the
    message of the old blanket grep, or names the configuration that opens it."""
    bad, good = [], []
    for guards, knob in knob_calls(text):
        probe = [g for g in guards if probe_symbol(g, dq_text, block_of)]
        n = len(bad)
        if not probe:
            bad.append('%s calls the test knob %s (a conformance image sets none)' % (rel, knob))
        for g in probe:
            for name, c in confs.items():
                if c.get(g) == 'y':
                    bad.append('%s: %s=y turns on the test knob %s (%s)' % (name, g, knob, rel))
        if probe and len(bad) == n:
            good += [(knob, g) for g in probe]
    return sorted(set(bad), key=bad.index), sorted(set(good))


def check_repo():
    bad, notes = [], []
    syms = kconfig_symbols()

    # 1. every symbol of ours any recipe file names exists
    files = [os.path.join(DEQP, 'prj.conf'), os.path.join(DEQP, 'texture-2048.conf')]
    bd = os.path.join(DEQP, 'boards')
    files += [os.path.join(bd, f) for f in sorted(os.listdir(bd)) if f.endswith('.conf')]
    scripts = [os.path.join(HERE, f) for f in sorted(os.listdir(HERE)) if f.endswith(('.sh', '.txt')) or f == 'README.md']
    scripts += [os.path.join(DEQP, 'README.md')]
    for f in files + scripts:
        seen = set()
        for i, l in enumerate(read(f).splitlines(), 1):
            if f in files and not l.lstrip().startswith(('CONFIG_', '#')):
                continue
            for m in re.finditer(r'CONFIG_([A-Z][A-Z0-9_]+)', l):
                s = m.group(1)
                if OURS.match(s) and s not in syms and s not in seen:
                    seen.add(s)
                    bad.append('%s:%d: CONFIG_%s is in no Kconfig file' % (os.path.relpath(f, REPO), i, s))
    notes.append('%d project symbols named by %d recipe files and %d scripts/tables/readmes checked against %d Kconfig symbols'
                 % (len({s for f in files for s in parse_conf(f) if OURS.match(s)}), len(files), len(scripts), len(syms)))

    # 2. the merged configuration of each board
    base = parse_conf(os.path.join(DEQP, 'prj.conf'))
    boards = {
        'AM62P': dict(base, **parse_conf(os.path.join(DEQP, 'boards/verdin_am62p_am62p54_a53.conf'))),
        'i.MX8MP': dict(base, **parse_conf(os.path.join(DEQP, 'boards/verdin_imx8mp_mimx8ml8_a53.conf'))),
        'X2': dict(base, **parse_conf(os.path.join(DEQP, 'boards/armadillo_x2_mimx8ml8_a53.conf'))),
        'Pi 4':dict(base, **parse_conf(os.path.join(DEQP, 'boards/rpi_4b.conf'))),
        'Pi 4 T5': {**base, **parse_conf(os.path.join(DEQP, 'boards/rpi_4b.conf')),
                    **parse_conf(os.path.join(DEQP, 'texture-2048.conf'))},
        'T6': dict(base, **parse_conf(os.path.join(DEQP, 'boards/nanopc_t6.conf'))),
    }
    dq = read(os.path.join(DEQP, 'Kconfig'))
    for name, c in boards.items():
        bad += conformance_rules(c, name)
        if not name.startswith('Pi') and 'DEQP_REPORT_ES2' not in dq:
            bad.append('deqp/Kconfig lost DEQP_REPORT_ES2')
    if boards['Pi 4'].get('YAKOGL_GPU_V3D') != 'y':
        bad.append('Pi 4: deqp/boards/rpi_4b.conf does not set CONFIG_YAKOGL_GPU_V3D=y')
    if boards['T6'].get('YAKOGL_GPU_MALI') != 'y':
        bad.append('T6: deqp/boards/nanopc_t6.conf does not set CONFIG_YAKOGL_GPU_MALI=y')

    # 3. the library's and the sample's own defaults
    def has(sym, text):
        return re.search(text, kconfig_block(sym) or '') is not None
    if not has('YAKOGL_REPORT_ES2', r'default y'):
        bad.append('zephyr/Kconfig: YAKOGL_REPORT_ES2 is not default y')
    if not has('DEQP_REPORT_ES2', r'default y') or not has('DEQP_REPORT_ES2', r'select YAKOGL_REPORT_ES2'):
        bad.append('deqp/Kconfig: DEQP_REPORT_ES2 is not default y selecting YAKOGL_REPORT_ES2')
    zk = read(os.path.join(REPO, 'zephyr/Kconfig'))
    if not re.search(r'choice YAKOGL_SHADERC\s+prompt[^\n]*\n\s*default YAKOGL_SHADERC_ONLINE', zk):
        bad.append('zephyr/Kconfig: the shader back end choice no longer defaults to YAKOGL_SHADERC_ONLINE')
    for s in ('YAKOGL_PRECOMPILED_SHADERS', 'YAKOGL_TEST_KNOBS'):
        if has(s, r'default y'):
            bad.append('zephyr/Kconfig: %s is on by default' % s)
    if not has('DEQP_TEST_KNOBS', r'select YAKOGL_TEST_KNOBS'):
        notes.append('deqp/Kconfig: DEQP_TEST_KNOBS no longer selects YAKOGL_TEST_KNOBS (fine only if no knob is called)')
    # the sample sets none of the knobs: no call of a test-knob entry point,
    # except under a probe symbol of deqp/Kconfig (default n, depends on
    # DEQP_TEST_KNOBS) that no conformance configuration turns on; every
    # accepted call is a note naming its guard
    # (the six boards above already merge these board files)
    guard_confs = dict(boards)
    merged = {'verdin_am62p_am62p54_a53.conf', 'verdin_imx8mp_mimx8ml8_a53.conf',
              'armadillo_x2_mimx8ml8_a53.conf', 'rpi_4b.conf', 'nanopc_t6.conf'}
    for f in sorted(os.listdir(bd)):
        if f.endswith('.conf') and f not in merged:
            guard_confs[f] = dict(base, **parse_conf(os.path.join(bd, f)))
    for dp, _, fn in os.walk(os.path.join(DEQP, 'src')):
        for f in fn:
            rel = os.path.relpath(os.path.join(dp, f), REPO)
            kb, kg = knob_verdicts(read(os.path.join(dp, f)), rel, dq, kconfig_block, guard_confs)
            bad += kb
            for knob, g in kg:
                notes.append('%s calls the test knob %s only under CONFIG_%s '
                             '(default n, depends on DEQP_TEST_KNOBS, no conformance configuration sets it)'
                             % (rel, knob, g))
    # PowerVR switches the sample passes: their Kconfig defaults must be the library's
    for s, want in (('DEQP_PVR_NATIVE_CUBE', 'default -1'), ('DEQP_PVR_NATIVE_ARRAY', 'default -1'),
                    ('DEQP_PVR_NATIVE_3D', 'default -1')):
        if not has(s, want):
            bad.append('deqp/Kconfig: %s is not "%s" (the A/B switches stay at the library default)' % (s, want))

    # 4. the carve-outs of the last passing sweeps
    p = boards['Pi 4']
    t = boards['Pi 4 T5']
    for k, v in (('YAKOGL_V3D_CARVEOUT_BYTES', '50331648'), ('YAKOGL_V3D_TILED_COPIES_BYTES', '16777216')):
        if p.get(k) != v:
            bad.append('Pi 4 rpi_4b.conf: %s=%s, the sweeps ran %s' % (k, p.get(k), v))
    # T5's carve-out: sweep 28's 64 MiB plus 6d2e7d0d's tile memory, 72 MiB
    # (texture-2048.conf's header, tests/v3d_es2ctx_t5_mem.rs)
    for k, v in (('YAKOGL_V3D_CARVEOUT_BYTES', '75497472'), ('YAKOGL_V3D_TILED_COPIES_BYTES', '25165824')):
        if t.get(k) != v:
            bad.append('Pi 4 texture-2048.conf: %s=%s, T5 is measured at %s' % (k, t.get(k), v))
    if boards['i.MX8MP'].get('COMMON_LIBC_MALLOC_ARENA_SIZE') != '167772160':
        bad.append('i.MX8MP conf: the C heap is not 160 MiB (167772160)')
    if boards['AM62P'].get('KERNEL_VM_SIZE') != '0x20000000':
        bad.append('AM62P conf: CONFIG_KERNEL_VM_SIZE is not 0x20000000')
    if not re.search(r'reg = <0xa0000000 0x10000000>', read(os.path.join(DEQP, 'boards/verdin_am62p_am62p54_a53.overlay'))):
        bad.append('AM62P overlay: the 256 MiB GPU carve-out at 0xa0000000 is gone')
    # the T6 has no passing sweep yet. A 2048x2048 texture's mip chain is
    # 33,546,240 bytes on the Mali, over zephyr/Kconfig's default 32 MiB
    t6 = boards['T6'].get('YAKOGL_MALI_CARVEOUT_BYTES')
    if not (t6 or '').isdigit() or int(t6) < 67108864:
        bad.append('T6 nanopc_t6.conf: YAKOGL_MALI_CARVEOUT_BYTES=%s, under the 64 MiB '
                   'texture.size.2d.2048x2048_* need' % t6)

    # 5. the tables
    must = [l.strip() for l in read(os.path.join(HERE, 'gles2-mustpass.txt')).splitlines()
            if l.strip() and not l.startswith('#')]
    for tname, nimg in (('am62p-full-tree.txt', 17), ('imx8mp-full-tree.txt', 29), ('pi4-full-tree.txt', 29)):
        rows = table_lines(os.path.join(HERE, tname))
        if len(rows) != nimg:
            bad.append('%s: %d images, the recipes say %d' % (tname, len(rows), nimg))
        if any('DEQP_GL_CONFIG_NAME' in m for _, m, _ in rows):
            bad.append('%s: a sweep row names a render config' % tname)
        miss = [n for n in must if not any(selects(f, n) for _, _, f in rows)]
        if miss:
            bad.append('%s selects %d of %d mustpass cases less than the tree needs, first: %s'
                       % (tname, len(miss), len(must), miss[0]))
    pi4 = table_lines(os.path.join(HERE, 'pi4-full-tree.txt'))
    t5 = [l for l, m, f in pi4 if 'with texture-2048.conf' in m]
    if t5 != ['T5']:
        bad.append("pi4-full-tree.txt: texture-2048.conf marks %s, only image T5 may carry it" % t5)
    ms = table_lines(os.path.join(HERE, 'am62p-ms4-tree.txt'))
    if len(ms) != 1 or ms[0][0] != 'MS4' or 'DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4' not in ms[0][1] \
            or ms[0][2] != 'dEQP-GLES2.functional.multisample.*':
        bad.append('am62p-ms4-tree.txt: not one MS4 image of multisample.* on rgba8888d24s8ms4')
    n_ms = len([n for n in must if selects('dEQP-GLES2.functional.multisample.*', n)])
    notes.append('%d multisample.* mustpass cases run on the ms4 image; the 17-image tree covers all %d mustpass cases'
                 % (n_ms, len(must)))

    # 6. the dEQP-GLES3 tables select every case of gles3-mustpass.txt exactly
    # once (check-gles3-tree.py, as a process: under -I a sibling import fails)
    for tname in ('am62p-gles3-tree.txt', 'pi4-gles3-tree.txt', 'imx8mp-gles3-tree.txt'):
        r = subprocess.run([sys.executable, '-I', os.path.join(HERE, 'check-gles3-tree.py'), os.path.join(HERE, tname)],
                           capture_output=True, text=True)
        verdict = [l for l in r.stdout.splitlines() if l.startswith('check-gles3-tree: ')]
        if r.returncode != 0:
            bad.append('%s: %s' % (tname, (r.stderr.strip().splitlines() or verdict or ['check-gles3-tree.py failed'])[0]))
        notes.append(verdict[0] if verdict else '%s: check-gles3-tree.py printed no verdict line' % tname)
    return bad, notes


def self_test():
    fails = 0

    def ok(cond, what):
        nonlocal fails
        print(('  ok    ' if cond else '  FAIL  ') + what)
        if not cond:
            fails += 1
    good = {'YAKOGL_SHADERC_ONLINE': 'y', 'YAKOGL_REPORT_ES2': 'y', 'DEQP_REPORT_ES2': 'y'}
    ok(conformance_rules(good, 'g') == [], 'a conformance configuration passes')
    ok(conformance_rules(dict(good, YAKOGL_SHADERC_PRECOMPILED_ONLY='y'), 'g'), 'precompiled-only is refused')
    ok(conformance_rules(dict(good, YAKOGL_PRECOMPILED_SHADERS='y'), 'g'), 'the precompiled tier is refused')
    ok(conformance_rules(dict(good, YAKOGL_REPORT_ES2='n'), 'g'), 'ES 3.0 reporting is refused')
    ok(conformance_rules({'YAKOGL_REPORT_ES2': 'y'}, 'g'), 'a missing compiler is refused')
    ok(conformance_rules(dict(good, DEQP_GL_CONFIG_NAME='rgba8888d24s8ms4'), 'g', ms4=False), 'a sweep image on the ms4 config is refused')
    ok(conformance_rules(good, 'g', ms4=True), 'the ms4 image without its config is refused')
    ok(conformance_rules(dict(good, DEQP_GL_CONFIG_NAME='rgba8888d24s8ms4'), 'g', ms4=True) == [], 'the ms4 image on its config passes')
    ok(conformance_rules({'YAKOGL_SHADERC_ONLINE': 'y'}, 'g', gles3=True) == [], 'a GLES3 image may report ES 3.0')
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, '.config')
        open(p, 'w').write('CONFIG_YAKOGL_SHADERC_ONLINE=y\nCONFIG_YAKOGL_REPORT_ES2=y\nCONFIG_DEQP_REPORT_ES2=y\n'
                           '# CONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY is not set\nCONFIG_DEQP_GL_CONFIG_NAME=""\n')
        ok(check_dotconfig(p, False) == [], 'a built .config of a sweep image passes')
        open(p, 'w').write('CONFIG_YAKOGL_SHADERC_ONLINE=y\nCONFIG_DEQP_REPORT_ES2=y\n')
        ok(check_dotconfig(p, False), 'a .config without YAKOGL_REPORT_ES2 is refused')
    # the fixture .config files under testdata/deqp-config/ (the form a Zephyr
    # build writes): one good per board plus the ms4 image, one bad per rule.
    # cts-selftest.sh check 17 runs the same matrix through the command line;
    # this keeps it inside the script's own self-test, which
    # tools/board_selftest.sh runs in the pre-landing gate.
    fixtures = os.path.join(HERE, 'testdata', 'deqp-config')
    for f, ms4 in (('am62p-sweep.config', False), ('am62p-ms4.config', True),
                   ('imx8mp-sweep.config', False), ('pi4-sweep.config', False)):
        ok(check_dotconfig(os.path.join(fixtures, f), ms4) == [],
           'fixture %s: a conformance image passes' % f)
    # the dEQP-GLES3 image (pi4-deqp-images.sh DEQP_MODULE=gles3): both REPORT_ES2
    # symbols off, which --gles3 wants and the GLES2 rule refuses; a GLES2 sweep
    # image judged with --gles3 is refused the other way round
    ok(check_dotconfig(os.path.join(fixtures, 'pi4-gles3-sweep.config'), False, gles3=True) == [],
       'fixture pi4-gles3-sweep.config: a dEQP-GLES3 image passes with --gles3')
    ok(any('YAKOGL_REPORT_ES2' in b for b in check_dotconfig(os.path.join(fixtures, 'pi4-gles3-sweep.config'), False)),
       'fixture pi4-gles3-sweep.config: refused as a GLES2 image, the message names YAKOGL_REPORT_ES2')
    ok(any('REPORT_ES2=y (a dEQP-GLES3 image' in b
           for b in check_dotconfig(os.path.join(fixtures, 'pi4-sweep.config'), False, gles3=True)),
       'fixture pi4-sweep.config with --gles3: refused, the message names REPORT_ES2')
    # --gles3: both REPORT_ES2 symbols off passes; DEQP_REPORT_ES2=n alone (YAKOGL_REPORT_ES2 left y) is refused
    ok(check_dotconfig(os.path.join(fixtures, 'bad-report-es3.config'), False, gles3=True) == [],
       'fixture bad-report-es3.config: a GLES3 image (both REPORT_ES2 off) passes --gles3')
    g3 = check_dotconfig(os.path.join(fixtures, 'bad-gles3-report.config'), False, gles3=True)
    ok(any('YAKOGL_REPORT_ES2' in b for b in g3),
       'fixture bad-gles3-report.config: --gles3 refuses it, naming YAKOGL_REPORT_ES2 (still ES 2.0)')
    for f, ms4, sym in (('bad-compiler-missing.config', False, 'YAKOGL_SHADERC_ONLINE'),
                        ('bad-shaderc-offline.config', False, 'YAKOGL_SHADERC_OFFLINE'),
                        ('bad-precompiled-only.config', False, 'YAKOGL_SHADERC_PRECOMPILED_ONLY'),
                        ('bad-precompiled-tier.config', False, 'YAKOGL_PRECOMPILED_SHADERS'),
                        ('bad-report-es3.config', False, 'YAKOGL_REPORT_ES2'),
                        ('bad-gles3-report.config', False, 'DEQP_REPORT_ES2'),
                        ('bad-sweep-ms4-name.config', False, 'DEQP_GL_CONFIG_NAME'),
                        ('bad-ms4-name-missing.config', True, 'DEQP_GL_CONFIG_NAME')):
        bad = check_dotconfig(os.path.join(fixtures, f), ms4)
        ok(any(sym in b for b in bad), 'fixture %s: refused, the message names %s' % (f, sym))
    rows = table_lines(os.path.join(HERE, 'am62p-ms4-tree.txt'))
    ok(rows == [('MS4', ' multisample.* (with DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4)', 'dEQP-GLES2.functional.multisample.*')],
       'the ms4 table is one labelled row with its render config')
    ok(selects('a.b.*,c.d.*', 'c.d.e') and not selects('a.b.*', 'a.c.d'), 'filters select like dEQP patterns')
    src = ('#if defined(CONFIG_A_MRT4)\nint v3d_debug_set_x(int n);\n#endif\n'
           '#ifdef CONFIG_B\n#if 1\npvr_debug_set_y(4);\n#endif\n#else\ngc_debug_set_z(1);\n#endif\n'
           'v3d_debug_set_w(2);\n#ifndef CONFIG_C\npvr_debug_set_v(3);\n#endif\n')
    ok(knob_calls(src) == [(['A_MRT4'], 'v3d_debug_set_x'), (['B'], 'pvr_debug_set_y'), ([], 'gc_debug_set_z'),
                           ([], 'v3d_debug_set_w'), ([], 'pvr_debug_set_v')],
       'a knob call is guarded by its #if defined / #ifdef symbols only, not by #else, #ifndef or a bare call')
    # acceptance (probe_symbol, knob_verdicts) on a synthetic deqp Kconfig and
    # merged configurations: K_N is the probe pattern, K_Y defaults y, K_ND
    # lacks the DEQP_TEST_KNOBS dependency
    dqt = 'config K_N\nconfig K_Y\nconfig K_ND\n'
    dqb = {'K_N': 'config K_N\n\tbool\n\tdefault n\n\tdepends on DEQP_TEST_KNOBS',
           'K_Y': 'config K_Y\n\tbool\n\tdefault y\n\tdepends on DEQP_TEST_KNOBS',
           'K_ND': 'config K_ND\n\tbool\n\tdefault n'}.get
    ksrc = ('#if defined(CONFIG_K_N)\nextern "C" int v3d_debug_set_draw_targets(int n);\n'
            'v3d_debug_set_draw_targets(4);\n#endif\n')
    two = {'Pi 4': {'YAKOGL_GPU_V3D': 'y'}, 'AM62P': {}}
    kb, kg = knob_verdicts(ksrc, 's.cpp', dqt, dqb, two)
    ok(kb == [] and kg == [('v3d_debug_set_draw_targets', 'K_N')],
       'a knob call guarded by a default n DEQP_TEST_KNOBS symbol no configuration sets is accepted, once')
    kb, kg = knob_verdicts('v3d_debug_set_draw_targets(4);\n', 's.cpp', dqt, dqb, two)
    ok(kb == ['s.cpp calls the test knob v3d_debug_set_draw_targets (a conformance image sets none)'] and kg == [],
       'an unguarded knob call is refused with the conformance message')
    ok(knob_verdicts(ksrc.replace('K_N', 'K_Y'), 's.cpp', dqt, dqb, two)[0]
       and knob_verdicts(ksrc.replace('K_N', 'K_ND'), 's.cpp', dqt, dqb, two)[0]
       and knob_verdicts(ksrc.replace('K_N', 'K_UNK'), 's.cpp', dqt, dqb, two)[0],
       'a knob call guarded by a default y symbol, one without DEQP_TEST_KNOBS or an unknown symbol is refused')
    kb, kg = knob_verdicts(ksrc, 's.cpp', dqt, dqb, {'Pi 4': {'K_N': 'y'}, 'AM62P': {}})
    ok(kb == ['Pi 4: K_N=y turns on the test knob v3d_debug_set_draw_targets (s.cpp)'] and kg == [],
       'a knob call guarded by a symbol a conformance configuration sets =y is refused')
    print('check-deqp-images self-test: %d checks failed' % fails)
    return fails


def main(argv):
    if argv[1:] == ['--self-test']:
        return 1 if self_test() else 0
    if argv[1:2] == ['--config']:
        if len(argv) < 3 or not os.path.isfile(argv[2]) or any(a not in ('--ms4', '--gles3') for a in argv[3:]):
            print('usage: check-deqp-images.py --config <.config> [--ms4] [--gles3]', file=sys.stderr)
            return 2
        bad = check_dotconfig(argv[2], '--ms4' in argv[3:], '--gles3' in argv[3:])
    elif len(argv) == 1:
        bad, notes = check_repo()
        for n in notes:
            print('check-deqp-images: ' + n)
    else:
        print(__doc__, file=sys.stderr)
        return 2
    for b in bad:
        print('check-deqp-images: FAIL ' + b, file=sys.stderr)
    if not bad:
        print('check-deqp-images: the dEQP image recipes build what conformance needs')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
