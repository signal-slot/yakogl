#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Attribute the marker lines of imx8mp-gles3 slice READMEs to cause ids.

usage: imx8mp-gles3-causes.py README... [--causes imx8mp-gles3-causes.tsv] [--table imx8mp-gles3-tree.txt]

imx8mp-gles3-slice.py writes one marker-led line per event in a README's "Resets and hangs" section:
  `WDOG1 eNN: the entry ended on a WDOG1 reset (...)[; last case NAME]`
  `Hang: eNN: the entry hung (...)[; last case NAME]`
Each line is mapped to the hang / WDOG1 cause ids of the cause map (ids with a `hang` or `wdog1` word),
by the case the line names, else by table entry NN (a cause line belongs to the entry when its glob
and one of the entry's globs match one another). A WDOG1 line prefers `wdog1` ids, a Hang line only
takes `hang` ids; a WDOG1 line with no `wdog1` id falls back to the `hang` ids (a reset can be a GPU hang).
Output, one TSV row per marker line: kind, entry, cause id(s) comma-separated or `unclassified`, line.
"""
import fnmatch
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MARK = re.compile(r"^(WDOG1|Hang):? e(\d+)\b[: ]*(.*)$")
LAST = re.compile(r"; last case (\S+)\s*$")


def words(cid):
    return set(cid.split("-"))


def load_causes(path):
    rules = []
    for n, raw in enumerate(open(path, encoding="utf-8"), 1):
        line = raw.rstrip("\n")
        if not line.strip() or line.startswith("#"):
            continue
        p = line.split("\t")
        if len(p) < 2 or not p[0] or not p[1]:
            sys.exit(f"{path}:{n}: want '<cause>\\t<glob>\\t<title>'")
        rules.append((p[0], p[1]))
    return rules


def load_table(path):
    return [[g for g in l.strip().split(",") if g] for l in open(path, encoding="utf-8")
            if l.strip() and not l.startswith("#")]


def overlaps(a, b):
    return a == b or fnmatch.fnmatchcase(a, b) or fnmatch.fnmatchcase(b, a)


def attribute(kind, entry, case, rules, table):
    want = "wdog1" if kind == "WDOG1" else "hang"
    pools = [[r for r in rules if want in words(r[0])]]
    if kind == "WDOG1":
        pools.append([r for r in rules if "hang" in words(r[0])])
    for pool in pools:
        if case:
            ids = [c for c, g in pool if fnmatch.fnmatchcase(case, g)][:1]
        else:
            if entry >= len(table):
                sys.exit(f"entry e{entry:02d} is outside the table ({len(table)} entries)")
            ids = []
            for c, g in pool:
                if c not in ids and any(overlaps(g, t) for t in table[entry]):
                    ids.append(c)
        if ids:
            return ids
    return []


def main(argv):
    causes = os.path.join(HERE, "imx8mp-gles3-causes.tsv")
    table = os.path.join(HERE, "imx8mp-gles3-tree.txt")
    files, i = [], 0
    while i < len(argv):
        a = argv[i]
        if a in ("--causes", "--table") and i + 1 < len(argv):
            if a == "--causes":
                causes = argv[i + 1]
            else:
                table = argv[i + 1]
            i += 2
        elif a.startswith("-"):
            sys.exit(__doc__)
        else:
            files.append(a)
            i += 1
    if not files:
        sys.exit(__doc__)
    rules, ents = load_causes(causes), load_table(table)
    for f in files:
        for raw in open(f, encoding="utf-8"):
            line = raw.strip().lstrip("-* ").strip()
            m = MARK.match(line)
            if not m:
                continue
            kind, e, rest = m.group(1), int(m.group(2)), m.group(3)
            lm = LAST.search(rest)
            ids = attribute(kind, e, lm.group(1) if lm else None, rules, ents)
            print("\t".join([kind, "e%02d" % e, ",".join(ids) or "unclassified", line]))


if __name__ == "__main__":
    main(sys.argv[1:])
