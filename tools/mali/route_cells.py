#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The two landing rules of the Mali compiler, main -> tip, on the product route.

    python3 -I tools/mali/route_cells.py MAIN.tsv TIP.tsv

MAIN.tsv and TIP.tsv are `tools/mali/route.sh measure` of one directory from
two trees: one row per program pair, `name verdict digest detail`, the verdict
EQUAL (both stages are the drm-shim reference's), DIFFER (compiled, not the
reference's), REFUSED (refused by name) or NOREF (compiled, no reference), the
digest that of the compiled words and facts (`-` when refused).

It prints the cells (main's verdict -> the tip's, "same words" when both
compiled the same program), the rules and the count of programs compiled with
words that are not Mesa's on each side:

  rule 1: a program main compiles equal to Mesa is refused or different on the
          tip (EQUAL -> DIFFER, REFUSED, or NOREF);
  rule 2: a program main refuses compiles on the tip with words that are not
          Mesa's (REFUSED -> DIFFER);
  needs a reference: a program that moved and has no reference on the tip
          (-> NOREF with other words), so neither rule can be read;
  compiled, not Mesa's: the DIFFER rows of each side.

A program in one file only is listed and counts as a broken rule (the two
files must measure the same directory). Exit 0 when no rule is broken and no
moved program lacks a reference; 1 otherwise; 2 on a malformed file.
"""
import collections
import sys

VERDICTS = ("EQUAL", "DIFFER", "REFUSED", "NOREF")


def load(path):
    rows = {}
    with open(path, encoding="utf-8", errors="replace") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line:
                continue
            parts = line.split("\t")
            if len(parts) != 4 or parts[1] not in VERDICTS:
                raise ValueError("%s:%d: not a route row: %r" % (path, n, line[:120]))
            name, verdict, digest, detail = parts
            if name in rows:
                raise ValueError("%s:%d: %s twice" % (path, n, name))
            rows[name] = (verdict, digest, detail)
    return rows


def cell(a, b):
    """The cell of one program: main's verdict -> the tip's."""
    (va, da, _), (vb, db, _) = a, b
    if va == "REFUSED" and vb == "REFUSED":
        return "REFUSED -> REFUSED"
    if da == db:
        return "%s -> same words" % va
    return "%s -> %s" % (va, vb) + (" (other words)" if va != "REFUSED" and vb != "REFUSED" else "")


def broken(a, b):
    """The rule program (main a, tip b) breaks, or None."""
    (va, da, _), (vb, db, _) = a, b
    if va == "EQUAL" and vb != "EQUAL":
        return "rule 1"
    if va == "REFUSED" and vb == "DIFFER":
        return "rule 2"
    if vb == "NOREF" and da != db:
        return "needs a reference"
    return None


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    try:
        m, t = load(argv[1]), load(argv[2])
    except (OSError, ValueError) as e:
        print("route_cells: %s" % e, file=sys.stderr)
        return 2
    cells = collections.Counter()
    rules = collections.defaultdict(list)
    for name in sorted(set(m) | set(t)):
        if name not in m or name not in t:
            rules["in one file only"].append("%s: only in %s" % (name, argv[1] if name in m else argv[2]))
            continue
        cells[cell(m[name], t[name])] += 1
        r = broken(m[name], t[name])
        if r:
            rules[r].append("%s: %s -> %s: %s" % (name, m[name][0], t[name][0], t[name][2]))
    for k in sorted(cells):
        print("%-36s %6d" % (k, cells[k]))
    differ = lambda rows: sum(1 for v in rows.values() if v[0] == "DIFFER")
    for r in ("rule 1", "rule 2", "needs a reference", "in one file only"):
        print("%s: %d" % (r, len(rules[r])))
    print("compiled, not Mesa's: main %d, tip %d" % (differ(m), differ(t)))
    for r in ("rule 1", "rule 2", "needs a reference", "in one file only"):
        for row in rules[r]:
            print("%s\t%s" % (r, row))
    return 1 if any(rules.values()) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
