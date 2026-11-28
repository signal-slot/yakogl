#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Attribute the WDOG1 and Hang marker lines of i.MX8MP dEQP-GLES3 slice READMEs to cause ids.

  imx8mp-gles3-causes.py README|SLICE_DIR... [--causes FILE] [--table FILE]

imx8mp-gles3-slice.py writes one marker-led line per event in a README's "## Resets and hangs" section:
  - WDOG1 eNN: the entry ended on a WDOG1 reset (...)[; last case NAME]
  - Hang: eNN: the entry hung (...)[; last case NAME]
A SLICE_DIR stands for its README.md. --causes is imx8mp-gles3-causes.tsv ('id<TAB>glob<TAB>title'), --table
imx8mp-gles3-tree.txt (entry NN is the NN-th non-comment line, from 0); both default to the files next to this
script. Only the event ids of the map are candidates: an id with a 'wdog1' word (gc-wdog1-*) or a 'hang' word
(gc-gpu-hang-*, gc-hang-*), words split at '-'. A WDOG1 line takes the wdog1 ids and, when none fits, the hang
ids (a reset can be a GPU hang); a Hang line takes the hang ids only. Within that order a line that names its
last case goes to the first id whose glob matches the case (map order, as in imx8mp-gles3-compare.py); a line
without a case, or whose case no id matches, goes to every id whose glob and one of entry NN's globs match one
another (either glob matching the other as a name). A line no id fits is 'unclassified'.
Output, one TSV row per marker line: kind (WDOG1 | Hang), eNN, the cause ids comma-separated or 'unclassified',
the line. Exit 1 on an unreadable file, a malformed map line or an entry outside the table.
"""
import argparse
import fnmatch
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MARK = re.compile(r"^(WDOG1|Hang):? e(\d+)\b:?\s*(.*)$")
LAST = re.compile(r"; last case (\S+)\s*$")


def die(msg):
    print(f"imx8mp-gles3-causes: {msg}", file=sys.stderr)
    sys.exit(1)


def parse_causes(path):
    rules = []
    with open(path, encoding="utf-8") as f:
        for n, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            p = line.split("\t")
            if len(p) < 2 or not p[0] or not p[1]:
                die(f"{path}:{n}: want '<cause>\\t<glob>\\t<title>'")
            rules.append((p[0], p[1]))
    return rules


def parse_table(path):
    ents = []
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.strip()
            if line and not line.startswith("#"):
                ents.append([g for g in line.split(",") if g])
    return ents


def event_pool(rules, word):
    return [(c, g) for c, g in rules if word in c.split("-")]


def overlaps(a, b):
    return a == b or fnmatch.fnmatchcase(a, b) or fnmatch.fnmatchcase(b, a)


def attribute(kind, entry, case, rules, ents):
    if entry >= len(ents):
        die(f"{kind} e{entry:02d}: entry outside the table ({len(ents)} entries)")
    pools = [event_pool(rules, "wdog1"), event_pool(rules, "hang")] if kind == "WDOG1" \
        else [event_pool(rules, "hang")]
    for pool in pools:
        if case:
            for c, g in pool:
                if fnmatch.fnmatchcase(case, g):
                    return [c]
        ids = []
        for c, g in pool:
            if c not in ids and any(overlaps(g, t) for t in ents[entry]):
                ids.append(c)
        if ids:
            return ids
    return []


def marker_lines(path):
    if os.path.isdir(path):
        path = os.path.join(path, "README.md")
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        die(f"{path}: {e.strerror}")
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith(("- ", "* ")):
            line = line[2:].strip()
        m = MARK.match(line)
        if m:
            yield m.group(1), int(m.group(2)), m.group(3), line


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("readme", nargs="+")
    ap.add_argument("--causes", default=os.path.join(HERE, "imx8mp-gles3-causes.tsv"))
    ap.add_argument("--table", default=os.path.join(HERE, "imx8mp-gles3-tree.txt"))
    a = ap.parse_args()
    try:
        rules, ents = parse_causes(a.causes), parse_table(a.table)
    except OSError as e:
        die(f"{e.filename}: {e.strerror}")
    for path in a.readme:
        for kind, entry, rest, line in marker_lines(path):
            m = LAST.search(rest)
            ids = attribute(kind, entry, m.group(1) if m else None, rules, ents)
            print("\t".join([kind, f"e{entry:02d}", ",".join(ids) or "unclassified", line]))


if __name__ == "__main__":
    main()
