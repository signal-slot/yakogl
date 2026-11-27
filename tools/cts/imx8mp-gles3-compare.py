#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the i.MX8MP board not-pass list with the HOST-ONLY GC7000UL one, per case.

  imx8mp-gles3-compare.py BOARD_NOTPASS HOST_NOTPASS
      [--causes tools/cts/imx8mp-gles3-causes.tsv] [--table tools/cts/imx8mp-gles3-tree.txt]
      [--examples N] [--tsv] [--gpu NAME] [--verdin VERDIN_NOTPASS]

Inputs: "<outcome>\\t<case>[\\t<message>]" (docs/es3/am62p-gles3-notpass.tsv), the case-first
"[<gpu>\\t]<case>\\t<outcome>[\\t...]" (docs/es3/host-gc-gles3-5454be1b/host-gc-gles3-notpass.tsv;
its 'gpu\\tcase\\tstatus...' column row is skipped) or "[<record>:]<outcome> <case>" (the output of
imx8mp-gles3-merge.py --notpass); '#' lines are headers. Each tab line's layout is told by its fields:
the case starts with 'dEQP-GLES3.' and the outcome is a dEQP status word (STATUS_WORDS); a line that
fits neither layout is refused with its file and line number.

Verdict of each board case:
  host-predicted  the host list has the case with the same outcome, or both outcomes are
                  fail-like (Fail, Crash, Timeout, ResourceError, InternalError, Hang, Watchdog)
  host-other      the host list has the case with another outcome (NotSupported vs Fail, ...)
  board-only      the host list lacks the case
Host-only cases (not-pass on the host, not on the board's list: passed or not run there) are only
counted per cause. Cause: the first matching glob of the cause map, else 'unclassified'. Entry:
the first table entry (counting non-comment lines from 0) with a matching glob, shown as eNN.

--gpu NAME keeps, of a host list with a gpu column (docs/es3/host-gles3-notpass.tsv), only the rows
whose first field is NAME (powervr, v3d, vivante); without it such a list warns on stderr and every
gpu's row counts. Lists without a gpu column are unaffected.

Board name: when BOARD_NOTPASS is `imx8mp-gles3-merge.py --notpass` output of Armadillo-X2 slices it carries the
comment `# measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)` (or `# boards: ...` when slices of both boards
were merged); that line is shown under the markdown heading and as the first line of --tsv. A list without it
is the Verdin's and prints as before. The X2's begun counts are '-' in its records; this tool reads only the
not-pass rows, so they change nothing here.

--verdin FILE (the earlier Verdin 9bc82915 slices' --notpass list, any layout above) adds, only when given, the
columns 'verdin' (Verdin not-pass cases of the cause) and 'x2 not on verdin' (board cases the Verdin list lacks)
to the markdown table, and a trailing 'verdin-same|verdin-other|x2-only' field to each --tsv row.

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
# imx8mp-gles3-merge.py's STATUSES, lower case, plus the fail-like words above
STATUS_WORDS = {"pass", "fail", "notsupported", "qualitywarning", "compatibilitywarning",
                "resourceerror", "internalerror", "crash", "timeout", "noresult"} | FAIL_LIKE
CASE_PREFIX = "dEQP-GLES3."


def parse_tab_row(path, n, line):
    """(outcome, case, message, gpu) of a tab line, outcome-first or case-first; None for a column row.
    gpu is the first field of a case-first line behind a gpu column, else None."""
    p = [x.strip() for x in line.split("\t")]
    if len(p) > 1 and p[0].lower() in STATUS_WORDS and p[1].startswith(CASE_PREFIX):
        return p[0], p[1], p[2] if len(p) > 2 else "", None
    for i in (0, 1):  # case-first, optionally behind a gpu column
        if len(p) > i + 1 and p[i].startswith(CASE_PREFIX) and p[i + 1].lower() in STATUS_WORDS:
            return p[i + 1], p[i], "", p[0] if i else None
    if p[:3] == ["gpu", "case", "status"] or p[:2] == ["case", "status"]:
        return None
    sys.exit(f"{path}:{n}: neither '<outcome>\\t{CASE_PREFIX}...' nor "
             f"'[<gpu>\\t]{CASE_PREFIX}...\\t<outcome>' (outcome a dEQP status word): {line[:100]}")


BOARD_RE = re.compile(r"^#\s*((?:measured on the|boards:).*Armadillo-X2.*)$")


def board_line(path):
    """The '# measured on the Armadillo-X2 ...' (or '# boards: ...') comment of a merge.py --notpass list, else None."""
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            m = BOARD_RE.match(raw.rstrip("\n"))
            if m:
                return m.group(1).strip()
    return None


def parse_notpass(path, gpu=None, warn=False):
    rows = []
    gpu_col = False
    with open(path, encoding="utf-8", errors="replace") as f:
        for n, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            if "\t" in line:
                r = parse_tab_row(path, n, line)
                if r:
                    if r[3] is not None:
                        gpu_col = True
                        if gpu is not None and r[3] != gpu:
                            continue
                    rows.append(r[:3])
            else:
                m = LINE_RE.match(line)
                if not m:
                    sys.exit(f"{path}: unrecognised row: {line[:80]}")
                rows.append((m.group(2), m.group(3), ""))
    if warn and gpu_col and gpu is None:
        print(f"{path}: warning: the list has a gpu column and --gpu is not given: "
              "every gpu's rows count as host not-pass", file=sys.stderr)
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
    ap.add_argument("--verdin", help="the Verdin slices' not-pass list: adds a per-cause Verdin column")
    ap.add_argument("--gpu", help="keep only host rows of this gpu when the host list has a gpu column")
    a = ap.parse_args()

    board = parse_notpass(a.board)
    host = {}
    for out, case, _ in parse_notpass(a.host, a.gpu, warn=True):
        host.setdefault(case, out)
    bl = board_line(a.board)
    verdin = None
    if a.verdin:
        verdin = {}
        for out, case, _ in parse_notpass(a.verdin):
            verdin.setdefault(case, out)
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
        line = f"{v}\t{c}\t{es}\t{out}\t{case}"
        if verdin is not None:
            vo = verdin.get(case)
            vv = "x2-only" if vo is None else ("verdin-same" if verdict(out, vo) == "host-predicted" else "verdin-other")
            line += "\t" + vv
            if vv == "x2-only":
                counts[c]["x2-only"] += 1
        lines.append(line)
    for case in host:
        if case not in seen:
            counts[find_cause(rules, case)]["host-only"] += 1

    if verdin is not None:
        for case in verdin:
            counts[find_cause(rules, case)]["verdin"] += 1
    if a.tsv:
        if bl:
            print("# " + bl)
        print("\n".join(lines))
        return
    order = list(titles) + ["unclassified"]
    order += sorted(c for c in counts if c not in order)
    cols = VERDICTS + ("host-only",) + (("verdin", "x2-only") if verdin is not None else ())
    tot = collections.Counter()
    print("# i.MX8MP board vs HOST-ONLY GC7000UL\n")
    if bl:
        print(bl + "\n")
    if verdin is not None:
        print(f"Verdin not-pass cases: {len(verdin)} (columns 'verdin': per cause; 'x2-only': board cases the "
              "Verdin list lacks)\n")
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
