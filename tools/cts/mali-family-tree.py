#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""A tree table of the compile-refused cases of NanoPC-T6 dEQP-GLES2 slices, by family.

  python3 -I tools/cts/mali-family-tree.py [--mustpass FILE] [--max N]
                                           [--families FILE] [--refusal TEXT]...
                                           SLICE... > family.txt

A slice is a tools/cts/log/<date>-t6-gles2-<session>-<sha8>/ directory with the
refusals.tsv that mali-deqp-tally.py wrote (case<TAB>refusal). Which refusal
belongs to which family is tools/cts/mali-family-refusals.txt (--families), one
<family><TAB><text> line each; a case goes to the first line whose text its
refusal contains, and a case with none is left out. --refusal TEXT (the flag
repeats, once per text) keeps only the cases whose refusal contains one of the
texts.

The table has the form tools/cts/mali-deqp-run.sh reads with --tree (that of
mali-gles2-tree.txt, checked by check-gles3-tree.py):

  # image T6RF<nnn>: <cases>
  <case>,<case>,...

Rows are filled family by family, in the order of the families file, each
family's cases in mustpass order, at most --max (default 400) per row; a row
holds one family. The header comment lists the rows of each family. Labels
T6RF001.. share no label with mali-gles2-tree.txt (T6S<s>R<r>) or
mali-rerun-tree.py (T6RR<n>). A case in several slices is listed once. A case
missing from the mustpass list is an error (exit 1), an unreadable file or a
usage error exits 2.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
USAGE = ("usage: mali-family-tree.py [--mustpass FILE] [--max N] [--families FILE] "
         "[--refusal TEXT]... SLICE...")


class Fatal(Exception):
    pass


def read_families(path):
    """[(family, text)] of the families file, in order."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    out = []
    for n, ln in enumerate(lines, 1):
        if not ln or ln.startswith("#"):
            continue
        f = ln.split("\t")
        if len(f) != 2 or not f[0] or not f[1]:
            raise Fatal("%s:%d: not a family<TAB>text line" % (path, n))
        out.append((f[0], f[1]))
    return out


def read_refusals(slice_dir):
    """{case: refusal} of one slice's refusals.tsv."""
    path = os.path.join(slice_dir.rstrip("/"), "refusals.tsv")
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    if not lines or lines[0].split("\t")[:2] != ["case", "refusal"]:
        raise Fatal("%s: header is not case<TAB>refusal" % path)
    out = {}
    for n, ln in enumerate(lines[1:], 2):
        if not ln:
            continue
        f = ln.split("\t")
        if len(f) < 2:
            raise Fatal("%s:%d: not a case<TAB>refusal line" % (path, n))
        out[f[0]] = f[1]
    return out


def classify(refusals, families, texts=None):
    """{family: [case]} of the cases of {case: refusal}, first matching line."""
    out = {}
    for case, refusal in refusals.items():
        if texts and not any(t in refusal for t in texts):
            continue
        for fam, text in families:
            if text in refusal:
                out.setdefault(fam, []).append(case)
                break
    return out


def build(slices, mustpass, families, max_cases=400, texts=None):
    """The table's text for the refused cases of the slices (list of dicts)."""
    merged = {}
    for s in slices:
        merged.update(s)
    index = {name: i for i, name in enumerate(mustpass)}
    by_family = classify(merged, families, texts)
    for cases in by_family.values():
        for case in cases:
            if case not in index:
                raise ValueError("refused case %s is not in the mustpass list" % case)
            if any(c in case for c in ", \t*"):
                raise ValueError("case '%s' cannot be written in a filter line" % case)
    order = []
    for fam, _ in families:
        if fam not in order:
            order.append(fam)
    rows, ranges = [], []
    for fam in order:
        cases = sorted(by_family.get(fam, []), key=index.get)
        first = len(rows) + 1
        for i in range(0, len(cases), max_cases):
            rows.append(cases[i:i + max_cases])
        if cases:
            ranges.append("# %s: T6RF%03d-T6RF%03d, %d case(s)"
                          % (fam, first, len(rows), len(cases)))
    total = sum(len(r) for r in rows)
    out = ["# The compile-refused cases of %d slice(s) by family: %d case(s) in %d row(s)."
           % (len(slices), total, len(rows)),
           "# One family per row, mustpass order within a family, at most %d per row." % max_cases]
    out += ranges
    for n, row in enumerate(rows, 1):
        out.append("# image T6RF%03d: %d" % (n, len(row)))
        out.append(",".join(row))
    return "\n".join(out) + "\n"


def main(argv):
    mustpass = os.path.join(HERE, "gles2-mustpass.txt")
    families = os.path.join(HERE, "mali-family-refusals.txt")
    max_cases, texts, slices = 400, [], []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--mustpass", "--max", "--families", "--refusal"):
            if i + 1 >= len(argv):
                print("mali-family-tree: %s wants a value" % a, file=sys.stderr)
                return 2
            v = argv[i + 1]
            i += 2
            if a == "--mustpass":
                mustpass = v
            elif a == "--families":
                families = v
            elif a == "--refusal":
                texts.append(v)
            else:
                if not v.isdigit() or int(v) < 1:
                    print("mali-family-tree: --max wants a positive number", file=sys.stderr)
                    return 2
                max_cases = int(v)
        elif a.startswith("-"):
            print(USAGE, file=sys.stderr)
            return 2
        else:
            slices.append(a)
            i += 1
    if not slices:
        print("mali-family-tree: no slice given", file=sys.stderr)
        return 2
    try:
        with open(mustpass, encoding="utf-8") as f:
            names = [l.strip() for l in f if l.strip() and not l.startswith("#")]
        fams = read_families(families)
        data = [read_refusals(s) for s in slices]
    except OSError as e:
        print("mali-family-tree: %s: %s" % (e.filename, e.strerror), file=sys.stderr)
        return 2
    except Fatal as e:
        print("mali-family-tree: %s" % e, file=sys.stderr)
        return 2
    try:
        text = build(data, names, fams, max_cases, texts)
    except ValueError as e:
        print("mali-family-tree: %s" % e, file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
