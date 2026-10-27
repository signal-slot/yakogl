#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The programs gl_test links, as the `pair` lines of tools/shaders/precompiled.list.

gl_test.c is the one sample whose programs can be read out of the repository
mechanically, and tools/v3d/glsl2v3d.py already does it (`gl_test_pairs`, its
pairing rule 1: the link_program calls, the sites that link through a
variable, the parameters resolved at their call sites, and the symbols mapped
to files through gen_gl_test_shaders.py's ENTRIES). This script reuses that
reader rather than keeping a second one, and turns its answer into the list's
two gl_test apps:

  app gl_test         the default build: each program with the sources
                      gl_test_shaders.h carries (thirteen are #version 450,
                      which only the table images can serve);
  app gl_test_es100   the GL_TEST_ES100 build, which every image with the
                      in-process compiler takes (gl_test.c: CONFIG_YAKOGL_
                      SHADERC_ONLINE selects it): each program with its
                      sources swapped for their ES 1.00 twins (ES100_TWINS).

Each app's pairs are in the order the reader finds them, first occurrence
only.

  gl_test_pairs.py            print both apps' pair lines
  gl_test_pairs.py --check    exit 1 unless precompiled.list holds exactly them

One shape glsl2v3d's reader does not follow is followed here: fragment
shaders handed to the linking function in a table of structs (gl_test 193
and 194, offarm_run; `table_arms`). The sites still unresolved are printed
(on stderr, and as the list's comments say): the three that link a named
source beside a buffer the case fills with GLSL while it runs -- case_vsunits
(149), shp_arm (150 shprobe, 166 fshprobe) and fshp_arm_units (166 arm C).
No pair names those programs and none can: half of each exists only after the
case runs, so an image with no shader compiler cannot build them at all.

The generated headers are the only GLSL text gl_test compiles, and this
script says so: it stops if gl_test.c declares a GLSL literal of its own,
because a source the reader cannot see is a source no list can name. That is
how gl_test 181, 182, 183 and 188 to 192 came to have no precompiled row --
their sources were C strings inside gl_test.c until they became
tools/shaders/gltest/.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
LIST = os.path.join(REPO, "tools", "shaders", "precompiled.list")
sys.path.insert(0, os.path.join(REPO, "tools", "v3d"))
import glsl2v3d  # noqa: E402  (gl_test_pairs, gen_gl_test_tables)

APPS = ("gl_test", "gl_test_es100")


def table_arms(unresolved):
    """The sites glsl2v3d's reader leaves with a vertex shader and no fragment
    shader because the fragment shaders come in a table of structs the
    function is handed (gl_test 193 and 194: offarm_run(const struct
    offarm_arm *arm, ...), its callers' static arrays, 194's built by a
    macro, `OFFARMN(8)` for `gl_test_offarm##n##_fs_src`): each caller's
    `_fs_src` symbols, macros expanded, paired with the site's vertex
    shader. [(vs_sym, fs_sym, how)], and the sites still unresolved."""
    text = open(glsl2v3d.GL_TEST_C).read()
    funcs = glsl2v3d.c_functions(text)
    pairs, left = [], []
    for u in unresolved:
        m = re.match(r"(\w+): links \d+, direct 0, left over vs=\['(\w+)'\] fs=\[\]$", u)
        callers = [] if not m else [(n, b) for n, _, b in funcs
                                    if n != m.group(1) and re.search(r"\b%s\s*\(" % m.group(1), b)]
        if not callers:
            left.append(u)
            continue
        for name, body in callers:
            syms = re.findall(r"\b%s\b" % glsl2v3d.SYM, body)
            for mac, pre, post in re.findall(r"#define (\w+)\(n\)[^\n]*\b(gl_test_\w*?)##n##(_fs_src)\b", body):
                syms += [pre + n + post for n in re.findall(r"\b%s\((\d+)\)" % mac, body)]
            for fs in [x for x in dict.fromkeys(syms) if x.endswith("_fs_src")]:
                pairs.append((m.group(2), fs, "gl_test.c %s via %s (a table of arms)" % (name, m.group(1))))
    return pairs, left


def derived():
    """{app: [(vs, fs)]} from gl_test.c, and the unresolved sites."""
    sym2file, twins = glsl2v3d.gen_gl_test_tables()
    pairs, unresolved = glsl2v3d.gl_test_pairs()
    arms, unresolved = table_arms(unresolved)
    for vs, fs, how in arms:
        if vs not in sym2file or fs not in sym2file:
            unresolved.append("%s: symbol not in gen_gl_test_shaders.py ENTRIES: %s" % (how, [vs, fs]))
            continue
        pairs.append((sym2file[vs], sym2file[fs], how))
    out = {a: [] for a in APPS}
    for vs, fs, how in pairs:
        # gl_test_pairs lists each twin pair right after its original; the
        # GL_TEST_ES100 build is derived here from the originals instead, so
        # that a program with no twin appears in both apps.
        if how.endswith("GL_TEST_ES100 twins"):
            continue
        for app, p in (("gl_test", (vs, fs)), ("gl_test_es100", (twins.get(vs, vs), twins.get(fs, fs)))):
            if p not in out[app]:
                out[app].append(p)
    return out, unresolved


def listed():
    """{app: [(vs, fs)]} as precompiled.list has them."""
    out = {a: [] for a in APPS}
    app = None
    for line in open(LIST):
        f = line.split("#", 1)[0].split()
        if not f:
            continue
        if f[0] == "app":
            app = f[1]
        elif f[0] == "pair" and app in out:
            out[app].append((f[1], f[2]))
    return out


def inline_literals():
    """Symbols gl_test.c declares as its own GLSL string literal.

    A source written as a C literal is invisible to the reader, so it can
    never become a `pair` line: the cases whose sources were literals had no
    precompiled row, and a draw asked for a program the image never built.
    Every GLSL source gl_test compiles is in a generated header instead.
    """
    lines = open(glsl2v3d.GL_TEST_C).read().split("\n")
    out, i = [], 0
    while i < len(lines):
        m = re.match(r"^static const char \*?(\w*_src)\[\]\s*=\s*(.*)$", lines[i])
        if not m:
            i += 1
            continue
        body, j = m.group(2), i
        while not body.rstrip().endswith(";"):
            j += 1
            body += "\n" + lines[j]
        if '"' in body:
            out.append(m.group(1))
        i = j + 1
    return sorted(out)


def main():
    literals = inline_literals()
    if literals:
        print("gl_test_pairs: gl_test.c declares %d GLSL literal(s) of its own: %s; "
              "a source the reader cannot see is a source no list can name -- move it to "
              "tools/shaders/ and name it in gen_gl_test_shaders.py's ENTRIES"
              % (len(literals), ", ".join(literals[:6])), file=sys.stderr)
        return 1
    want, unresolved = derived()
    for u in unresolved:
        print("gl_test_pairs: unresolved: %s" % u, file=sys.stderr)
    if "--check" not in sys.argv[1:]:
        for app in APPS:
            print("app %s" % app)
            for vs, fs in want[app]:
                print("pair %s %s" % (vs, fs))
        return 0
    have = listed()
    bad = [a for a in APPS if have[a] != want[a]]
    for app in bad:
        extra = [p for p in have[app] if p not in want[app]]
        missing = [p for p in want[app] if p not in have[app]]
        print("gl_test_pairs: app %s: listed %d, gl_test.c links %d; not linked: %s; not listed: %s%s"
              % (app, len(have[app]), len(want[app]), extra[:4], missing[:4],
                 "" if extra or missing else " (the order differs)"))
    if bad:
        print("gl_test_pairs: precompiled.list is not what gl_test.c links; "
              "regenerate its gl_test apps with tools/precompile/gl_test_pairs.py")
        return 1
    print("gl_test_pairs: precompiled.list holds the %s pairs gl_test.c links (%s)"
          % ("+".join(str(len(want[a])) for a in APPS), ", ".join(APPS)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
