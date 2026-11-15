#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One issue draft per cause from the i.MX8MP dEQP-GLES3 board vs HOST-ONLY comparison.

  imx8mp-gles3-issues.py COMPARE.tsv [--causes FILE] [--commit SHA8] [--examples N] [--verdict board-only]

COMPARE.tsv is `imx8mp-gles3-compare.py --tsv`: one line per board not-pass case,
'verdict<TAB>cause<TAB>eNN<TAB>outcome<TAB>case'. --causes is imx8mp-gles3-causes.tsv ('id<TAB>glob<TAB>title',
default: next to this script). Prints markdown, one block per cause with cases of --verdict (default
board-only), the largest first: proposed title, case count, table entries, example cases, campaign commit and
a Reproduce line. Nothing but the TSV's case names, entry ids and outcomes is printed (no address, no path).
"""
import argparse
import os
import sys


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("tsv")
    ap.add_argument("--causes", default=os.path.join(here, "imx8mp-gles3-causes.tsv"))
    ap.add_argument("--commit", default="")
    ap.add_argument("--examples", type=int, default=5)
    ap.add_argument("--verdict", default="board-only")
    a = ap.parse_args()

    titles = {}
    try:
        with open(a.causes, encoding="utf-8") as f:
            for ln in f:
                p = ln.rstrip("\n").split("\t")
                if len(p) >= 3 and p[0] and not p[0].startswith("#"):
                    titles[p[0]] = p[2]
    except OSError as e:
        print("imx8mp-gles3-issues: cannot read causes: %s" % e.strerror, file=sys.stderr)
        return 2

    groups = {}
    try:
        with open(a.tsv, encoding="utf-8") as f:
            for n, ln in enumerate(f, 1):
                ln = ln.rstrip("\n")
                if not ln or ln.startswith("#"):
                    continue
                p = ln.split("\t")
                if len(p) < 5:
                    print("imx8mp-gles3-issues: line %d: expected 5 tab-separated fields" % n, file=sys.stderr)
                    return 2
                if p[0] != a.verdict:
                    continue
                groups.setdefault(p[1], []).append((p[2], p[3], p[4]))
    except OSError as e:
        print("imx8mp-gles3-issues: cannot read input: %s" % e.strerror, file=sys.stderr)
        return 2

    def eidx(e):
        return (0, int(e[1:]), "") if e[1:].isdigit() else (1, 0, e)

    commit = a.commit or "not given"
    order = sorted(groups, key=lambda c: (-len(groups[c]), c))
    for cause in order:
        rows = groups[cause]
        title = titles.get(cause, "unclassified failures" if cause == "unclassified" else cause)
        ents = sorted({r[0] for r in rows}, key=eidx)
        print("## GC7000UL: %s: %d dEQP-GLES3 cases fail on the board only" % (title, len(rows)))
        print()
        print("- Cause id: %s" % cause)
        print("- Verdict: %s" % a.verdict)
        print("- Cases: %d" % len(rows))
        print("- Table entries: %s" % ", ".join(ents))
        if a.examples > 0:
            print("- Examples:")
            for e, o, c in sorted(rows, key=lambda r: (eidx(r[0]), r[2]))[:a.examples]:
                print("  - `%s` (%s, %s)" % (c, e, o))
        print("- Campaign commit: %s" % commit)
        print("- Reproduce: docs/es3/board-recipe-imx8mp.md ('Running a GLES3 slice session'), "
              "the table entries above")
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
