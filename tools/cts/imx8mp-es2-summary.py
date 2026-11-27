#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Summarise the i.MX8MP ES 2.0 regression rows: totals per board.

    python3 -I tools/cts/imx8mp-es2-summary.py [--baseline FILE] PATH...

PATH is read exactly as imx8mp-es2-regress.py reads it (README.md, verdicts.txt,
results.tsv or a directory; the verdict lines of board.sh imx8mp or board.sh x2,
the Armadillo-X2 with the same i.MX 8M Plus and GC7000UL). Output:
    es2 summary: board <name(s)> (baseline: Verdin i.MX8MP stage 1)
    es2 summary: <n>/29 entries, passed <p>, failed <f>, notsup <s>, other <o>,
        noresult <r> (baseline passed <bp>, bad <bb>)
    es2 summary: measured on the Armadillo-X2   (only when an x2 line was read)
The record is judged by imx8mp-es2-regress.py, not here. Exit 0 if all 29
entries are present, 1 otherwise, 2 bad input.
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "es2regress", os.path.join(HERE, "imx8mp-es2-regress.py"))
R = importlib.util.module_from_spec(spec)
spec.loader.exec_module(R)


def main(argv):
    args = argv[1:]
    base = R.BASELINE
    if args[:1] == ["--baseline"] and len(args) > 1:
        base, args = args[1], args[2:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    try:
        bl = R.read_tsv(base)
        got = {}
        for p in args:
            got.update(R.read_path(p, bl))
    except (OSError, ValueError) as e:
        print("es2 summary: " + str(e), file=sys.stderr)
        return 2
    names = sorted({r["board"] for r in got.values() if "board" in r})
    print("es2 summary: board %s (baseline: Verdin i.MX8MP stage 1)" %
          (", ".join(names) if names else "not named by the records"))
    tot = dict(passed=0, failed=0, notsup=0, other=0, noresult=0)
    for r in got.values():
        for k in tot:
            tot[k] += r[k]
    bp = sum(b["passed"] for b in bl.values())
    bb = sum(R.bad(b) for b in bl.values())
    print("es2 summary: %d/29 entries, passed %d, failed %d, notsup %d, other %d, "
          "noresult %d (baseline passed %d, bad %d)" % (
              len(got), tot["passed"], tot["failed"], tot["notsup"],
              tot["other"], tot["noresult"], bp, bb))
    if "Armadillo-X2" in names:
        print("es2 summary: measured on the Armadillo-X2 (same SoC and GPU as "
              "the Verdin i.MX8MP)")
    return 0 if len(got) == 29 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
