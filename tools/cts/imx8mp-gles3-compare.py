#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the i.MX8MP board not-pass list with the HOST-ONLY GC7000UL one, per case.

  imx8mp-gles3-compare.py BOARD_NOTPASS HOST_NOTPASS
      [--causes tools/cts/imx8mp-gles3-causes.tsv] [--table tools/cts/imx8mp-gles3-tree.txt]
      [--examples N] [--tsv]

Inputs: "<outcome>\\t<case>[\\t<message>]" (docs/es3/am62p-gles3-notpass.tsv) or
"[<record>:]<outcome> <case>" (the output of imx8mp-gles3-merge.py --notpass); '#' lines are headers.

Verdict of each board case:
  host-predicted  the host list has the case with the same outcome, or both outcomes are
                  fail-like (Fail, Crash, Timeout, ResourceError, InternalError, Hang, Watchdog)
  host-other      the host list has the case with another outcome (NotSupported vs Fail, ...)
  board-only      the host list lacks the case
Host-only cases (not-pass on the host, not on the board's list: passed or not run there) are only
counted per cause. Cause: the first matching glob of the cause map, else 'unclassified'. Entry:
the first table entry (counting non-comment lines from 0) with a matching glob, shown as eNN.

Default output is markdown (summary table, examples per cause and verdict); --tsv prints
"verdict\\tcause\\teNN\\toutcome\\tcase" per board case.
"""
import argparse
import collections
import fnmatch
import re
import sys

LINE_RE = re.compile(r"^(?:(\S+?):)?([A-Za-z]+)\s+(dEQP-\S+)")
FAIL_LIKE = {"fail", "crash", "timeout", "resourceerror", "internalerror", "hang", "watchdog"}
VERDICTS = ("host-predicted", "host-other", "board-only")


def parse_notpass(path):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            if "\t" in line:
                p = line.split("\t")
                rows.append((p[0].strip(), p[1].strip(), p[2].strip() if len(p) > 2 else ""))
            else:
                m = LINE_RE.match(line)
                if not m:
                    sys.exit(f"{path}: unrecognised row: {line[:80]}")
                rows.append((m.group(2), m.group(3), ""))
    return rows


def parse_table(path):
    ents = []
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.strip()
            if line and not line.startswith("#"):
                ents.append([g for g in line.split(",") if g])
    return ents


def find_entry(ents, case):
    for i, pats in enumerate(ents):
        if any(fnmatch.fnmatchcase(case, g) for g in pats):
            return i
    return None


def parse_causes(path):
    rules, titles = [], collections.OrderedDict()
    with open(path, encoding="utf-8") as f:
        for n, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            p = line.split("\t")
            if len(p) < 2 or not p[0] or not p[1]:
                sys.exit(f"{path}:{n}: want '<cause>\\t<glob>\\t<title>'")
            rules.append((p[0], p[1]))
            titles.setdefault(p[0], p[2] if len(p) > 2 else "")
    return rules, titles


def find_cause(rules, case):
    for cid, g in rules:
        if fnmatch.fnmatchcase(case, g):
            return cid
    return "unclassified"


def verdict(board_out, host_out):
    if host_out is None:
        return "board-only"
    b, h = board_out.lower(), host_out.lower()
    if b == h or (b in FAIL_LIKE and h in FAIL_LIKE):
        return "host-predicted"
    return "host-other"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("board")
    ap.add_argument("host")
    ap.add_argument("--causes")
    ap.add_argument("--table")
    ap.add_argument("--examples", type=int, default=3)
    ap.add_argument("--tsv", action="store_true")
    a = ap.parse_args()

    board = parse_notpass(a.board)
    host = {}
    for out, case, _ in parse_notpass(a.host):
        host.setdefault(case, out)
    rules, titles = parse_causes(a.causes) if a.causes else ([], {})
    ents = parse_table(a.table) if a.table else None

    seen = set()
    counts = collections.defaultdict(collections.Counter)
    ex = collections.defaultdict(list)
    lines = []
    for out, case, _ in board:
        v = verdict(out, host.get(case))
        c = find_cause(rules, case)
        e = find_entry(ents, case) if ents is not None else None
        es = f"e{e}" if e is not None else "-"
        seen.add(case)
        counts[c][v] += 1
        ex[(c, v)].append(f"{case} ({out}, {es})")
        lines.append(f"{v}\t{c}\t{es}\t{out}\t{case}")
    for case in host:
        if case not in seen:
            counts[find_cause(rules, case)]["host-only"] += 1

    if a.tsv:
        print("\n".join(lines))
        return
    order = list(titles) + ["unclassified"]
    order += sorted(c for c in counts if c not in order)
    cols = VERDICTS + ("host-only",)
    tot = collections.Counter()
    print("# i.MX8MP board vs HOST-ONLY GC7000UL\n")
    print(f"Board not-pass cases: {len(board)}; host not-pass cases: {len(host)}\n")
    print("| cause | " + " | ".join(cols) + " | board total |")
    print("|---|" + "---:|" * (len(cols) + 1))
    for c in order:
        if c not in counts:
            continue
        r = counts[c]
        tot.update(r)
        bt = sum(r[v] for v in VERDICTS)
        print(f"| {c} | " + " | ".join(str(r[v]) for v in cols) + f" | {bt} |")
    print("| **total** | " + " | ".join(str(tot[v]) for v in cols)
          + f" | {sum(tot[v] for v in VERDICTS)} |")
    for c in order:
        if c not in counts or not any(counts[c][v] for v in VERDICTS):
            continue
        print(f"\n## {c}" + (f": {titles[c]}" if titles.get(c) else ""))
        for v in VERDICTS:
            if not counts[c][v]:
                continue
            print(f"\n- {v}: {counts[c][v]}")
            for s in ex[(c, v)][:a.examples]:
                print(f"  - {s}")


if __name__ == "__main__":
    main()
