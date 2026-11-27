#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One issue draft per cause from the i.MX8MP dEQP-GLES3 board vs HOST-ONLY comparison.

  imx8mp-gles3-issues.py COMPARE.tsv [--causes FILE] [--commit SHA8] [--examples N] [--verdict board-only] [--strict]
                         [--slices DIR...]

COMPARE.tsv is `imx8mp-gles3-compare.py --tsv`: one line per board not-pass case,
'verdict<TAB>cause<TAB>eNN<TAB>outcome<TAB>case'. --causes is imx8mp-gles3-causes.tsv ('id<TAB>glob<TAB>title',
default: next to this script). Prints markdown, one block per cause with cases of --verdict (default
board-only), the largest first: proposed title, case count, table entries, example cases, campaign commit and
a Reproduce line. A cause id ending in '-no-record' or starting with 'unexplained-' only fits an entry
without a board record: each such cause with cases prints a stderr WARNING (exit status stays 0; --strict
makes it 1). --slices names the slice directories (tools/cts/log/*-imx8mp-gles3-*, or a board.sh record)
the TSV came from; merge.py's board_of() tells the board of each. When one is an Armadillo-X2 slice
(three-word records, begun '-' allowed; `board.sh x2` lines; a README naming the Armadillo-X2) every draft says
`measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2` and names the 8-hex commit and
the slice directories as the records. Without --slices, or with Verdin slices only, the text is unchanged.
A hang, abort or WDOG1 cause id (gc-gpu-hang-*, gc-wdog1-*, gc-abort-*; "event" ids) is drafted as an event
issue: its title says hang/abort/WDOG1 reset, the body names the cause id, every glob of the id, the table
entries (from the cases, or "see the slice README" when none), the Vivante back-end area (the src/ or driver/
path in the cause's title, else src/vivante and driver) and a 'Record: <slice record> at <commit>' placeholder
line. --events also drafts every event id of --causes that has no case in the TSV (a hang gets no result, so
the compare may not list it). Nothing but the TSV's case names, entry ids and outcomes is printed (no address, no path).
"""
import argparse
import importlib.util
import os
import re
import sys

EVENT_RE = re.compile(r"^gc-(gpu-)?(hang|wdog1|abort)-")
KIND = {"hang": "GPU hang", "wdog1": "WDOG1 reset", "abort": "abort"}
AREA_RE = re.compile(r"((?:src|driver)/[\w./-]*\w)")


def load_board_of(here):
    spec = importlib.util.spec_from_file_location("imx8mp_gles3_merge", os.path.join(here, "imx8mp-gles3-merge.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m.board_of


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("tsv")
    ap.add_argument("--causes", default=os.path.join(here, "imx8mp-gles3-causes.tsv"))
    ap.add_argument("--commit", default="")
    ap.add_argument("--examples", type=int, default=5)
    ap.add_argument("--verdict", default="board-only")
    ap.add_argument("--strict", action="store_true", help="exit 1 if a no-record/unexplained cause has cases")
    ap.add_argument("--events", action="store_true", help="also draft event ids of --causes without cases")
    ap.add_argument("--slices", nargs="+", default=[], metavar="DIR", help="slice directories the TSV came from")
    a = ap.parse_args()

    boards = {}
    if a.slices:
        board_of = load_board_of(here)
        boards = {d: board_of(d) for d in a.slices}
    x2 = "x2" in boards.values()
    records = ", ".join(os.path.basename(d.rstrip("/")) for d in a.slices) if x2 else ""
    if x2 and not (len(a.commit) >= 8 and all(c in "0123456789abcdef" for c in a.commit.lower())):
        print("imx8mp-gles3-issues: an Armadillo-X2 draft needs --commit with the 8-hex campaign commit",
              file=sys.stderr)
        return 2

    titles = {}
    globs = {}
    try:
        with open(a.causes, encoding="utf-8") as f:
            for ln in f:
                p = ln.rstrip("\n").split("\t")
                if len(p) >= 3 and p[0] and not p[0].startswith("#"):
                    titles.setdefault(p[0], p[2])
                    globs.setdefault(p[0], []).append(p[1])
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

    placeholder = sorted(c for c in groups if c.endswith("-no-record") or c.startswith("unexplained-"))
    for c in placeholder:
        print("imx8mp-gles3-issues: WARNING: cause %s has %d case(s): triage must give these cases a real cause"
              % (c, len(groups[c])), file=sys.stderr)

    commit = a.commit or "not given"
    if a.events:
        for c in titles:
            if EVENT_RE.match(c):
                groups.setdefault(c, [])
    order = sorted(groups, key=lambda c: (-len(groups[c]), c))
    for cause in order:
        rows = groups[cause]
        if EVENT_RE.match(cause):
            kind = KIND[EVENT_RE.match(cause).group(2)]
            ents = sorted({r[0] for r in rows}, key=eidx)
            area = sorted(set(AREA_RE.findall(titles.get(cause, "")))) or ["src/vivante", "driver"]
            print("## GC7000UL: %s (cause %s)" % (titles.get(cause, cause).split(" (")[0], cause))
            print()
            print("- Cause id: %s" % cause)
            print("- Kind: %s" % kind)
            print("- Cause: %s" % titles.get(cause, cause))
            print("- Glob: %s" % ", ".join("`%s`" % g for g in globs.get(cause, [])))
            print("- Table entries: %s" % (", ".join(ents) if ents else "see the slice README (markers)"))
            print("- Vivante back-end area: %s" % ", ".join(area))
            print("- Cases: %d" % len(rows))
            if a.examples > 0 and rows:
                print("- Examples:")
                for e, o, c in sorted(rows, key=lambda r: (eidx(r[0]), r[2]))[:a.examples]:
                    print("  - `%s` (%s, %s)" % (c, e, o))
            if x2:
                print("- Board: measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2")
                print("- Campaign commit: %s" % commit[:8])
                print("- Records: %s" % records)
            else:
                print("- Campaign commit: %s" % commit)
                print("- Record: <slice record> at <commit>")
            print("- Reproduce: docs/es3/board-recipe-imx8mp.md ('Running a GLES3 slice session'), "
                  "the table entries above; last case from the console scan")
            print()
            continue
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
        if x2:
            print("- Board: measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2")
            print("- Campaign commit: %s" % commit[:8])
            print("- Records: %s" % records)
        else:
            print("- Campaign commit: %s" % commit)
        print("- Reproduce: docs/es3/board-recipe-imx8mp.md ('Running a GLES3 slice session'), "
              "the table entries above")
        print()
    return 1 if a.strict and placeholder else 0


if __name__ == "__main__":
    sys.exit(main())
