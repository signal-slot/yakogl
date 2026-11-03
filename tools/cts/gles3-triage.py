#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Group a dEQP-GLES3 not-pass list into issue drafts (markdown on stdout).

  gles3-triage.py NOTPASS.tsv [--host docs/es3/host-gles3-tip.tsv]
                  [--gpu vivante] [--table tools/cts/imx8mp-gles3-tree.txt]
                  [--depth 4] [--examples 5]

NOTPASS.tsv: one case per line, "<outcome>\\t<case>[\\t<message>]" (the shape of
docs/es3/am62p-gles3-notpass.tsv), or "[<record>:]<outcome> <case>" (the shape
of docs/es3/pi4-gles3-fails.tsv). Lines starting with '#' are headers.

A group is the case's dEQP path under "dEQP-GLES3." cut at --depth levels
(default 4, e.g. functional.texture.filtering.2d).

Host cross-reference: docs/es3/host-gles3-tip.tsv has one row per
(gpu, group) with pass/fail/notsupported/crash/other counts, only for the
two-level groups ("functional.texture"), not per case. Each case takes the
verdict of its host group: "host-also-fails" when the host has fails/crashes
and no pass (likely GL core / shader compiler, not GPU-specific),
"board-only" when the host has passes and no fail/crash/other, "host-mixed"
otherwise, "host-notsupported" / "host-unknown" when it has no data. The
host's render verdicts are meaningless (host-only run): only the non-render
groups are a firm guide.

Table: each case is mapped to the first entry (line, counting from 0, of the
non-comment lines) whose comma-separated glob filters match it, shown as eNN.
"""
import argparse
import collections
import fnmatch
import re
import sys

PREFIX = "dEQP-GLES3."
LINE_RE = re.compile(r"^(?:(\S+?):)?([A-Za-z]+)\s+(dEQP-\S+)")


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


def parse_host(path, gpu):
    """group -> (pass, fail, notsupported, crash, other) for one gpu."""
    out = {}
    cols = None
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            p = line.split("\t")
            if p[0] == "gpu":
                cols = p
                continue
            if cols is None or p[0] != gpu:
                continue
            d = dict(zip(cols, p))
            out[d["group"]] = tuple(int(d[k]) for k in
                                    ("pass", "fail", "notsupported", "crash", "other"))
    if not out:
        sys.exit(f"{path}: no rows for gpu '{gpu}'")
    return out


def host_verdict(counts):
    if counts is None:
        return "host-unknown"
    p, f, _ns, c, o = counts
    if f + c > 0 and p == 0:
        return "host-also-fails"
    if p > 0 and f + c + o == 0:
        return "board-only"
    if p == 0 and f + c + o == 0:
        return "host-notsupported"
    return "host-mixed"


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


def host_group(host, rel):
    """Longest host group that is a dotted prefix of rel."""
    parts = rel.split(".")
    for n in range(len(parts), 0, -1):
        g = ".".join(parts[:n])
        if g in host:
            return host[g]
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("notpass")
    ap.add_argument("--host")
    ap.add_argument("--gpu", default="vivante", help="host gpu column (default vivante)")
    ap.add_argument("--table")
    ap.add_argument("--depth", type=int, default=4)
    ap.add_argument("--examples", type=int, default=5)
    a = ap.parse_args()
    if a.depth < 1:
        sys.exit("--depth must be >= 1")

    rows = parse_notpass(a.notpass)
    host = parse_host(a.host, a.gpu) if a.host else None
    ents = parse_table(a.table) if a.table else None

    groups = collections.OrderedDict()
    for outcome, case, msg in rows:
        rel = case[len(PREFIX):] if case.startswith(PREFIX) else case
        key = ".".join(rel.split(".")[:a.depth])
        g = groups.setdefault(key, {"cases": [], "out": collections.Counter(),
                                    "ent": collections.Counter(),
                                    "host": collections.Counter(),
                                    "msg": collections.Counter()})
        g["cases"].append(case)
        g["out"][outcome] += 1
        if msg and msg != outcome:
            g["msg"][msg[:100]] += 1
        if ents is not None:
            e = find_entry(ents, case)
            g["ent"]["e%02d" % e if e is not None else "none"] += 1
        if host is not None:
            g["host"][host_verdict(host_group(host, rel))] += 1

    order = sorted(groups, key=lambda k: (-len(groups[k]["cases"]), k))
    print(f"# dEQP-GLES3 not-pass triage: {len(rows)} cases in {len(groups)} groups "
          f"(depth {a.depth})\n")
    print(f"Input: `{a.notpass}`" + (f"; host: `{a.host}` (gpu {a.gpu})" if host else "")
          + (f"; table: `{a.table}`" if ents is not None else "") + "\n")
    for k in order:
        g = groups[k]
        n = len(g["cases"])
        mix = ", ".join(f"{o} {c}" for o, c in g["out"].most_common())
        dom = g["out"].most_common(1)[0][0]
        print(f"## {k}\n")
        print(f"- cases: {n}")
        print(f"- verdicts: {mix}")
        if ents is not None:
            print("- table entries: " + ", ".join(f"{e} ({c})" for e, c in sorted(g["ent"].items())))
        if host is not None:
            h = g["host"]
            print(f"- host-also-fails: {h['host-also-fails']}")
            print(f"- board-only: {h['board-only']}")
            rest = ", ".join(f"{v} {c}" for v, c in sorted(h.items())
                             if v not in ("host-also-fails", "board-only"))
            if rest:
                print(f"- other host: {rest}")
        if g["msg"]:
            print("- messages: " + "; ".join(f"`{m}` ({c})" for m, c in g["msg"].most_common(3)))
        print("- examples:")
        for c in sorted(g["cases"])[:a.examples]:
            print(f"  - `{c}`")
        scope = "GL core / shader compiler" if host is not None and \
            g["host"]["host-also-fails"] * 2 > n else "GPU"
        print(f"- suggested title: `dEQP-GLES3 {k}: {n} {dom} cases ({scope})`\n")


if __name__ == "__main__":
    main()
