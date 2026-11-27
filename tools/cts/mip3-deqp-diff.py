#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case diff of dEQP-GLES2 texture.mipmap.2d verdicts, arm A against arm E.

    tools/cts/mip3-deqp-diff.py A E [--prefix P] [--all]

A (knob off) and E (MIP_THREE_LEVELS_DEFAULT true) are result files or record
directories in any form tools/cts/tree-diff.py reads (the deqp console lines
"deqp: <Status> <case>", .qpa, "<Status> <case>" lists; a board.sh run
directory's console.txt). The case set is every case under --prefix (default
dEQP-GLES2.functional.texture.mipmap.2d.) that either side has a result for; a
case one side lacks reads "Missing". Prints a markdown table (case | A | E) of
the differing cases (--all: every case), the families (generate.* is
glGenerateMipmap, the only family the knob can change; basic.*, projected.*,
bias.* are sampling cases it must not change) and ends with
  mipmap.2d: N cases, D differ (new failures F, new passes P, other O)
new failure: Pass on A, not Pass on E; new pass: the reverse; other: both not
Pass with different verdicts. Exit 0 when no case went from Pass to non-Pass
(Missing counts as non-Pass), 1 when one did, 2 on a usage or input error.
Stdlib only.
"""
import argparse
import os
import sys
from importlib.util import module_from_spec, spec_from_file_location

HERE = os.path.dirname(os.path.abspath(__file__))
PREFIX = "dEQP-GLES2.functional.texture.mipmap.2d."
KNOB = "generate."


def tree_diff():
    spec = spec_from_file_location("tree_diff", os.path.join(HERE, "tree-diff.py"))
    mod = module_from_spec(spec)
    sys.modules["tree_diff"] = mod
    spec.loader.exec_module(mod)
    return mod


def family(case, prefix):
    rest = case[len(prefix):] if case.startswith(prefix) else case
    return rest.split(".", 1)[0] or "?"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("a", metavar="A", help="arm A result file or record directory")
    ap.add_argument("e", metavar="E", help="arm E result file or record directory")
    ap.add_argument("--prefix", default=PREFIX, help="case-name prefix (default %(default)s)")
    ap.add_argument("--all", action="store_true", help="print every case, not only differing")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    td = tree_diff()
    try:
        sa = td.load_side([args.a], "A")["status"]
        se = td.load_side([args.e], "E")["status"]
    except td.InputError as e:
        print("mip3-deqp-diff: %s" % e, file=sys.stderr)
        return 2
    cases = sorted(c for c in set(sa) | set(se) if c.startswith(args.prefix))
    if not cases:
        print("mip3-deqp-diff: no case under %s in either side" % args.prefix, file=sys.stderr)
        return 2
    newfail = newpass = other = 0
    rows, fams = [], {}
    for c in cases:
        a, e = sa.get(c, "Missing"), se.get(c, "Missing")
        diff = a != e
        f = family(c, args.prefix)
        d = fams.setdefault(f, [0, 0])
        d[0] += 1
        if diff:
            d[1] += 1
            if a == "Pass":
                newfail += 1
            elif e == "Pass":
                newpass += 1
            else:
                other += 1
        if diff or args.all:
            rows.append((c, a, e, diff))
    ndiff = newfail + newpass + other
    print("| case | A | E |")
    print("|---|---|---|")
    for c, a, e, diff in rows:
        print("| %s | %s | %s |" % (c[len(args.prefix):] if c.startswith(args.prefix) else c, a, e))
    print()
    print("families (cases, differing); %s* is glGenerateMipmap, the only family "
          "the knob can change:" % KNOB)
    for f in sorted(fams):
        tag = " (generate: the knob)" if f + "." == KNOB else ""
        print("- %s.*: %d cases, %d differ%s" % (f, fams[f][0], fams[f][1], tag))
    outside = sorted({family(c, args.prefix) for c, a, e, d in rows
                      if d and family(c, args.prefix) + "." != KNOB})
    if outside:
        print("DIFFERENCE OUTSIDE generate.*: %s (the knob cannot change these; "
              "flaky or a defect)" % ", ".join(x + ".*" for x in outside))
    print("mipmap.2d: %d cases, %d differ (new failures %d, new passes %d, other %d)"
          % (len(cases), ndiff, newfail, newpass, other))
    return 1 if newfail else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
