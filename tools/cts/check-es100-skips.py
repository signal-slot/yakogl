#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""check-es100-skips.py - the GL_TEST_ES100 skip list, checked against the code.

With GL_TEST_ES100=1 the suite compiles the ES 1.00 shader set, and the cases
whose GLSL is `#version 300 es` have no ES 1.00 form at all. Those are skipped
by name and `gl_test_total` is decremented, so the DENOMINATOR changes -- which
means two mistakes are possible and neither is visible in a score:

  * a case that uses an ES 3.00 source and is NOT skipped feeds `#version 300
    es` to the ES 1.00 front end. It fails at the version line, and the failure
    reads exactly like a compiler defect.
  * a case that is skipped but uses NO ES 3.00 source is a test silently not
    run, and the denominator is wrong in the other direction.

This asks both questions from the sources. It does not take a list from anyone:
the dispatch switch says which function each case runs, the function's own body
says which shader sources it compiles, and the headers say which of those are
`#version 300 es`.

**Map cases by the dispatch, never by the `/* ---- case N` comments.** Those
comments have helper functions between them, so a comment-position heuristic
attributes a helper's sources to whichever case comment precedes it. Doing that
here first produced a false finding -- case 42 appearing to be skipped with no
ES 3.00 source of its own, when `case_es3array()` compiles two of them.

Usage:
  python3 tools/cts/check-es100-skips.py
Exit status is 1 if either question has a bad answer.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GL_TEST = ROOT / "driver/tools/pvrlab/gl_test.c"
RUNNER = ROOT / "driver/tools/pvrlab/runner/src/lib.rs"
HDR_ES100 = ROOT / "driver/tools/pvrlab/gl_test_shaders_es100.h"

# The regression set, SET_ORDER of the gl_test runner
# (driver/tools/pvrlab/runner/src/lib.rs). Read from the source below.
def read_order(src):
    m = re.search(r'static SET_ORDER:\s*\[i32;\s*\d+\]\s*=\s*\[([^\]]*)\]', src)
    if not m:
        sys.exit("check-es100-skips: could not find SET_ORDER in the runner's lib.rs")
    return [int(x) for x in re.findall(r'\d+', m.group(1))]

def read_dispatch(src):
    return {int(m.group(1)): m.group(2)
            for m in re.finditer(r'case (\d+):\s*rc = (case_\w+)\(', src)}

def read_body(src, fn):
    m = re.search(r'^static int ' + re.escape(fn) + r'\(void\)\s*\n\{', src, re.M)
    if not m:
        return ""
    i = src.index('{', m.start())
    depth = 0
    for j in range(i, len(src)):
        if src[j] == '{':
            depth += 1
        elif src[j] == '}':
            depth -= 1
            if depth == 0:
                return src[i:j + 1]
    return src[i:]

def es300_symbols(header_text):
    """Shader symbols in the ES 1.00 header whose source is still ES 3.00.

    Derived from the text, not from the name: a symbol called `es300_*` that
    had been given an ES 1.00 form would otherwise be reported forever.
    """
    out = set()
    for m in re.finditer(r'static const char (\w+)\[\]\s*=\s*(.*?);\s*\n',
                         header_text, re.S):
        body = ''.join(re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(2)))
        if '#version 300' in body:
            out.add(m.group(1))
    return out

def main():
    src = GL_TEST.read_text()
    order = read_order(RUNNER.read_text())
    dispatch = read_dispatch(src)
    es300 = es300_symbols(HDR_ES100.read_text())

    uses = {}
    for n, fn in dispatch.items():
        body = read_body(src, fn)
        u = sorted(es300 & set(re.findall(r'gl_test_\w+_src', body)))
        if u:
            uses[n] = (fn, u)

    # The suite's own skip list. It is NOT an array: the generator
    # declares it as GL_TEST_SHADERS_SKIP_REASON(c) in the generated ES 1.00
    # header, a chain of `(c) == N ||` returning the reason string.
    #
    # The first version of this script looked for `es100_skip[] = {...}`, found
    # nothing once the real hunk landed, and printed "gl_test.c declares no
    # skip list yet" while exiting 0 -- a checker silently not checking, which
    # is the failure it exists to catch. Read the macro, and if the header
    # declares GL_TEST_ES100_SET but no reason macro can be parsed, say so and
    # FAIL rather than report a clean absence.
    hdr = HDR_ES100.read_text()
    m = re.search(r'#define\s+GL_TEST_SHADERS_SKIP_REASON\(c\)(.*?)\n\n', hdr, re.S)
    if m:
        declared = sorted({int(x) for x in re.findall(r'\(c\)\s*==\s*(\d+)', m.group(1))})
    elif 'GL_TEST_ES100_SET' in hdr:
        print()
        print("check-es100-skips: the header defines GL_TEST_ES100_SET but no")
        print("  GL_TEST_SHADERS_SKIP_REASON(c) could be parsed from it. That is a")
        print("  parse failure in this script, not a clean result -- do not read the")
        print("  exit status as agreement.")
        return 1
    else:
        declared = None

    should = sorted(uses)
    print(f"gl_test cases in the dispatch      : {len(dispatch)}")
    print(f"ES 3.00 shader symbols in the header: {len(es300)}")
    print(f"cases compiling one or more of them : {should}")
    for n in should:
        fn, u = uses[n]
        print(f"    case {n:3d}  {fn:22s} {u}")

    in_set = sorted(set(should) & set(order))
    print()
    print(f"regression set                     : {len(order)} cases")
    print(f"  of which use an ES 3.00 source   : {in_set} ({len(in_set)})")
    print(f"  denominator with GL_TEST_ES100=1 : {len(order)} - {len(in_set)} = {len(order) - len(in_set)}")

    if declared is None:
        print()
        print("no GL_TEST_SHADERS_SKIP_REASON in the ES 1.00 header (the wiring has")
        print("not landed). The list above is what it must be when it does:")
        print(f"    {should}")
        return 0

    missing = sorted(set(should) - set(declared))
    extra = sorted(set(declared) - set(should))
    print()
    print(f"declared skip list                 : {declared}")
    rc = 0
    if missing:
        print(f"NOT SKIPPED but uses ES 3.00 source : {missing}")
        print("  each of these feeds `#version 300 es` to the ES 1.00 front end and")
        print("  fails at the version line, which reads exactly like a compiler defect.")
        rc = 1
    if extra:
        print(f"SKIPPED but uses no ES 3.00 source  : {extra}")
        print("  each of these is a test silently not run, and the denominator is wrong.")
        rc = 1
    if rc == 0:
        print("skip list agrees with the sources.")
    return rc

if __name__ == "__main__":
    sys.exit(main())
