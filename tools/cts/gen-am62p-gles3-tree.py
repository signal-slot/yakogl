#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""tools/cts/am62p-gles3-tree.txt from tools/cts/gles3-mustpass.txt.

  tools/cts/gen-am62p-gles3-tree.py            write the table
  tools/cts/gen-am62p-gles3-tree.py --check    the table is what the generator
                                               writes, and every mustpass case
                                               is matched by exactly one filter
  tools/cts/gen-am62p-gles3-tree.py --self-test

One line is one deqp image (CONFIG_DEQP_CASE, comma-separated dEQP globs). The
board's RTI watchdog resets the SoC 600 s after the image booted, so an image
has to finish well inside that: the rows are packed to at most MAX_CASES
mustpass cases, and the groups the board measured as slow are split off or
narrowed (SPLIT, OWN, NARROW below; the measurements are in tools/cts/README.md,
"The AM62P dEQP-GLES3 tree").

Units are the dotted groups of the mustpass list: dEQP-GLES3.functional.<y>
and dEQP-GLES3.info; a unit over MAX_CASES, or named in SPLIT, is divided by
its next name level, recursively. Units are packed in mustpass order, a unit
that does not fit starting a new image; a unit in OWN gets an image of its own.
A unit in NARROW is matched by the listed globs instead of `<unit>.*`: the globs
must match exactly the unit's mustpass cases that the unit-wide glob would, and
leave out the dEQP cases that are not in the mustpass list (the tally ignores
those, and some take minutes each).
"""
import fnmatch
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MUSTPASS = os.path.join(HERE, 'gles3-mustpass.txt')
TABLE = os.path.join(HERE, 'am62p-gles3-tree.txt')
MAX_CASES = 2000
ROOT = 'dEQP-GLES3.'

# Divided by the next name level although small enough: texture.filtering is
# the slowest group on the board (2d_array 252 cases in 100 s, 3d.combinations
# minutes per case outside the mustpass list), and its 3d groups are not
# measured, so they are separated from the cheap ones.
SPLIT = {
    'functional.texture.filtering',
    'functional.texture.filtering.3d',
    'functional.shaders.builtin_functions',
    'functional.shaders.builtin_functions.precision',
}
# Units whose cases are slow, by name prefix: a case counts WEIGHT times in
# the packing. Image 7 of 2026-10-25 (main be84094b) ran 584 of the 1,147
# builtin_functions.precision cases in 420 s, and a second image of them ran 561
# in 600 s (0.7 to 1.1 s a case against about 0.1 s for the rest); the watchdog
# reset the board before either image ended.
WEIGHT = {
    'functional.shaders.builtin_functions.precision': 6,
}
# Units that get an image of their own (a hang or a watchdog reset costs only
# that unit's results, and the unit's measured time is not shared).
OWN = {
    'functional.texture.filtering.3d.formats',
    'functional.texture.filtering.3d.sizes',
}
# 3d.combinations in the mustpass list is only linear_linear_*, linear_nearest_*
# and nearest_*; dEQP also has linear_mipmap_* and nearest_mipmap_* there, of
# which the first took 337 s a case on the board (and the next one never
# finished).
NARROW = {
    'functional.texture.filtering.3d.combinations': [
        'functional.texture.filtering.3d.combinations.linear_linear_*',
        'functional.texture.filtering.3d.combinations.linear_nearest_*',
        'functional.texture.filtering.3d.combinations.nearest_*',
    ],
}
# Each 3D *_linear_mipmap_linear case can hang the GPU and its failing check is
# slow (docs/notes/pvr-3d-mipmap-hang.md), so in these units the case gets an
# image of its own and the other cases are named by mustpass prefix, never by
# a `<unit>.*` glob that would also take dEQP's cases outside the list.
ISOLATE = {
    'functional.texture.filtering.3d.formats',
    'functional.texture.filtering.3d.sizes',
}
ISOLATE_SUFFIX = '_linear_mipmap_linear'


def read_mustpass(path=MUSTPASS):
    with open(path) as f:
        return [l for l in f.read().splitlines() if l]


def units(cases, max_cases=MAX_CASES, split=SPLIT):
    """[(unit, count)] in mustpass order: two name levels under the root,
    divided further while over max_cases or in split."""
    def group(prefix_depth, rows):
        out = {}
        for c in rows:
            p = c[len(ROOT):].split('.')
            d = prefix_depth if p[0] == 'functional' else prefix_depth - 1
            key = '.'.join(p[:max(d, 1)])
            out.setdefault(key, []).append(c)
        return out

    def walk(rows, depth):
        res = []
        for key, sub in group(depth, rows).items():
            if (len(sub) > max_cases or key in split) and len(sub[0][len(ROOT):].split('.')) > len(key.split('.')) + 1:
                res += walk(sub, depth + 1)
            else:
                res.append((key, len(sub)))
        return res

    return walk(cases, 2)


def filters(unit):
    if unit in NARROW:
        return [ROOT + g for g in NARROW[unit]]
    return [ROOT + unit + '.*']


def isolate_lines(unit, cases):
    """[[glob]] for an ISOLATE unit: one line for its cases but the
    *_linear_mipmap_linear ones, named by mustpass prefix, then one line per
    such case."""
    names = [c for c in cases if c.startswith(ROOT + unit + '.')]
    slow = [c for c in names if c.endswith(ISOLATE_SUFFIX)]
    rest = [c for c in names if not c.endswith(ISOLATE_SUFFIX)]
    bases = {}
    for c in rest:
        for tail in ('_linear_mipmap_nearest', '_nearest_mipmap_linear', '_nearest_mipmap_nearest', '_linear', '_nearest'):
            if c.endswith(tail):
                bases.setdefault(c[:-len(tail)], []).append(tail)
                break
    globs = []
    for b, tails in bases.items():
        near = [t for t in tails if t.startswith('_nearest')]
        if len(near) == 3:
            globs.append(b + '_nearest*')
            tails = [t for t in tails if not t.startswith('_nearest')]
        globs += [b + t for t in tails]
    return [globs] + [[c] for c in slow]


def pack(us, max_cases=MAX_CASES, own=OWN):
    rows, cur, n = [], [], 0
    for u, c in us:
        c *= max([w for p, w in WEIGHT.items() if u.startswith(p)] or [1])
        if u in own:
            if cur:
                rows.append(cur)
            rows.append([u])
            cur, n = [], 0
            continue
        if cur and n + c > max_cases:
            rows.append(cur)
            cur, n = [], 0
        cur.append(u)
        n += c
    if cur:
        rows.append(cur)
    return rows


def render(cases):
    rows = pack(units(cases))
    n_images = sum(len(isolate_lines(r[0], cases)) if len(r) == 1 and r[0] in ISOLATE else 1 for r in rows)
    total = len(cases)
    out = [
        '# The dEQP-GLES3 tree for the Verdin AM62P in %d deqp images (CONFIG_DEQP_CASE per line), DEQP_MODULE=gles3.' % n_images,
        '# Generated by tools/cts/gen-am62p-gles3-tree.py from tools/cts/gles3-mustpass.txt (%s): groups over %s mustpass cases'
        % (format(total, ','), format(MAX_CASES, ',')),
        '# split by subgroup, packed in order to at most %s per image; the groups the board measured as slow are split off' % format(MAX_CASES, ','),
        '# (texture.filtering.3d, one image per subgroup, its combinations narrowed to the mustpass names;',
        '# each 3d.formats and 3d.sizes *_linear_mipmap_linear case in an image of its own) so every image',
        '# ends well inside the 600 s watchdog (tools/cts/README.md); every mustpass case is matched by exactly one line.',
        '# The other filters match dEQP\'s whole tree; the tally (deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt)',
        '# restricts to the list.',
    ]
    for r in rows:
        if len(r) == 1 and r[0] in ISOLATE:
            for g in isolate_lines(r[0], cases):
                out.append(','.join(g))
            continue
        out.append(','.join(f for u in r for f in filters(u)))
    return '\n'.join(out) + '\n'


def coverage(cases, table_text):
    """Problems of a table: [str]. Every case matched by exactly one line."""
    lines = [l for l in table_text.splitlines() if l and not l.startswith('#')]
    pats = [[p for p in l.split(',')] for l in lines]
    bad = []
    per = [0] * len(lines)
    for c in cases:
        hit = [i for i, ps in enumerate(pats) if any(fnmatch.fnmatchcase(c, p) for p in ps)]
        if len(hit) != 1:
            bad.append('%s matched by %d lines' % (c, len(hit)))
        for i in hit:
            per[i] += 1
    bad += ['line %d holds %d cases (over %d)' % (i + 1, n, MAX_CASES) for i, n in enumerate(per) if n > MAX_CASES]
    return bad


def check():
    cases = read_mustpass()
    text = open(TABLE).read()
    bad = coverage(cases, text)
    if text != render(cases):
        bad.append('the table is not what the generator writes')
    for b in bad[:20]:
        print(b)
    n = len([l for l in text.splitlines() if l and not l.startswith('#')])
    print('%d images, %d cases, %d problems' % (n, len(cases), len(bad)))
    return 1 if bad else 0


def self_test():
    cases = read_mustpass()
    text = render(cases)
    assert coverage(cases, text) == [], 'the generated table covers the list'
    lines = text.splitlines()
    dup = [l for l in lines if not l.startswith('#')]
    # a case listed twice is reported
    twice = '\n'.join(dup + [dup[0].split(',')[0]]) + '\n'
    assert any('matched by 2' in b for b in coverage(cases, twice)), 'a duplicate line is caught'
    # a case no line matches is reported
    short = '\n'.join(dup[1:]) + '\n'
    assert any('matched by 0' in b for b in coverage(cases, short)), 'a missing line is caught'
    # an unnarrowed slow group is not hidden: 3d.combinations never shares a line with the mipmap cases
    assert not any('3d.combinations.*' in l for l in dup), 'the 3d combinations are narrowed'
    # each slow 3D case runs alone, and no isolated unit is matched by a wildcard
    slow = [l for l in dup if ISOLATE_SUFFIX in l]
    assert slow and all(',' not in l and '*' not in l for l in slow), 'the slow 3d cases are one image each'
    assert not any(u in l for u in ISOLATE for l in dup if '.*' in l), 'isolated units are named by prefix'
    # an image over the cap is caught
    one = ','.join(f for l in dup for f in l.split(',')) + '\n'
    assert any('over %d' % MAX_CASES in b for b in coverage(cases, one)), 'an overfull line is caught'
    print('gen-am62p-gles3-tree self-test ok')
    return 0


if __name__ == '__main__':
    a = sys.argv[1:]
    if a == ['--check']:
        sys.exit(check())
    if a == ['--self-test']:
        sys.exit(self_test())
    if a:
        sys.exit(__doc__)
    open(TABLE, 'w').write(render(read_mustpass()))
