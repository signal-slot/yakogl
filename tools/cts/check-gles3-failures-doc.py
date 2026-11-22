#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the AM62P family table of docs/es3-cts-failures.md against the triage totals.

  check-gles3-failures-doc.py DOC.md NOTPASS.tsv FAMILIES.txt

DOC.md   docs/es3-cts-failures.md
NOTPASS.tsv  docs/es3/am62p-gles3-notpass.tsv
FAMILIES.txt tools/cts/am62p-gles3-families.txt

Format of the section. A heading line exactly

    ## AM62P failures by cause family

(the section runs to the next '## ' heading) holds one pipe table with the
header "| Family | Count | Sites | Issue |" and one row per family:

    | Family | Count | Sites | Issue |
    |---|---:|---|---|
    | `RC1-int-mix` | 12 | `src/pvr/backend.rs:1234`, `src/gl/draw.rs:88` | #1301 |
    | #1285-watchdog | 1,024 | `src/pvr/texture.rs:77` | #1285 |

Family is the family id as in the TSV's 4th column (backticks optional).
Count is a decimal number (a thousands comma is allowed). Sites and Issue are
free text but must not be empty. A 'total' row (Family 'total') is optional;
when present its count must equal the TSV row count.

Totals come from gles3-triage.py's own functions: a row's family is its 4th
column, or, when that is empty, the first matching rule of FAMILIES.txt
(family_of). Exit 0 when the table matches. Exit 1 listing every difference:
a family with rows but no table row, a table row naming a family with no
rows, a count that differs, an empty Sites or Issue, a duplicate row, or a
table sum that differs from the TSV row count. Exit 2 when the section or
its table is absent, or an input cannot be read.
"""
import collections
import importlib.util
import os
import re
import sys

HEADING = "## AM62P failures by cause family"


def load_triage():
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location(
        "gles3_triage", os.path.join(here, "gles3-triage.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def section(doc):
    out, on = [], False
    with open(doc, encoding="utf-8") as f:
        lines = f.read().split("\n")
    for ln in lines:
        if ln.rstrip() == HEADING:
            on = True
            continue
        if on and ln.startswith("## "):
            break
        if on:
            out.append(ln)
    return out if on else None


def table_rows(sec):
    """[(family, count_text, sites, issue)] of the data rows; None if no table."""
    rows, header = [], False
    for ln in sec:
        s = ln.strip()
        if not s.startswith("|"):
            continue
        cells = [c.strip() for c in s.strip("|").split("|")]
        if not header:
            if [c.lower() for c in cells[:4]] == ["family", "count", "sites", "issue"]:
                header = True
            continue
        if all(re.fullmatch(r":?-+:?", c) for c in cells if c):
            continue
        if len(cells) != 4:
            rows.append((cells[0] if cells else "", "?", "", "?cells"))
            continue
        rows.append(tuple(cells))
    return rows if header else None


def main():
    if len(sys.argv) != 4:
        print("usage: check-gles3-failures-doc.py DOC.md NOTPASS.tsv FAMILIES.txt",
              file=sys.stderr)
        return 2
    doc, tsv, fam = sys.argv[1:]
    try:
        tri = load_triage()
        sec = section(doc)
        rules = tri.parse_families(fam)
        rows = tri.parse_notpass(tsv)
    except OSError as e:
        print(f"check-gles3-failures-doc: {e}", file=sys.stderr)
        return 2
    if sec is None:
        print(f"check-gles3-failures-doc: {doc}: section '{HEADING}' is absent",
              file=sys.stderr)
        return 2
    table = table_rows(sec)
    if table is None:
        print(f"check-gles3-failures-doc: {doc}: section '{HEADING}' has no "
              "'| Family | Count | Sites | Issue |' table", file=sys.stderr)
        return 2

    want = collections.Counter(r[3] or tri.family_of(rules, r[1], r[2]) for r in rows)
    errs, seen, total_row, got_sum = [], set(), None, 0
    for famid, cnt, sites, issue in table:
        famid = famid.strip("`").strip()
        if issue == "?cells":
            errs.append(f"row '{famid}': needs 4 cells (family, count, sites, issue)")
            continue
        if famid in seen:
            errs.append(f"family '{famid}' appears twice in the table")
            continue
        seen.add(famid)
        if not re.fullmatch(r"\d{1,3}(,\d{3})+|\d+", cnt.strip()):
            errs.append(f"family '{famid}': count '{cnt}' is not a number")
            continue
        n = int(cnt.replace(",", ""))
        if famid.lower() == "total":
            total_row = n
            continue
        got_sum += n
        if famid not in want:
            errs.append(f"table row '{famid}' names a family with no rows in {tsv}")
        elif want[famid] != n:
            errs.append(f"family '{famid}': table says {n}, triage counts {want[famid]}")
        if not sites.strip() or not issue.strip():
            errs.append(f"family '{famid}': empty Sites or Issue")
    for famid in sorted(want):
        if famid not in seen:
            errs.append(f"family '{famid}' ({want[famid]} rows) is missing from the table")
    if got_sum != len(rows):
        errs.append(f"table sums to {got_sum}, {tsv} has {len(rows)} rows")
    if total_row is not None and total_row != len(rows):
        errs.append(f"total row says {total_row}, {tsv} has {len(rows)} rows")
    if errs:
        for e in errs:
            print(f"check-gles3-failures-doc: {e}", file=sys.stderr)
        return 1
    print(f"check-gles3-failures-doc: ok, {len(want)} families, {len(rows)} rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
