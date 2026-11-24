#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Row specs of the AM62P dEQP-GLES3 not-pass TSV header.

The head of docs/es3/am62p-gles3-notpass.tsv names the build and run date per
row of tools/cts/am62p-gles3-tree.txt (rows counted from 0), as written by
gles3-notpass.py --splice:

    # rows 0-10: build b25997e4, run 2026-11-22
    # rows 11-45: build ad2b5ed, run 2026-10/14

This module (stdlib only, self-contained) reads those lines and also the forms
a re-split needs: a row split into sub-rows ('22a', '22b'), comma lists
('0-10,12') and rows past the old tree (46, 47, ...).

Row keys.  A key is (base, label): (22, '') is row 22, (22, 'a') its sub-row
a.  Keys order 22 < 22a < 22b < 23.  Specs: '0-10' (every whole row 0..10),
'22', '22a', '22a-22c' (sub-rows a..c of one base; both ends must have the
same base), '0-3,22a,22b'.  Ranges mixing a whole row with a sub-row, backwards
ranges, empty items and anything else raise RowsError with a message.

Coverage (coverage(): tree_rows, parsed head).  A row is covered when its whole
key is named, or when it is named only by sub-rows that run a, b, c ... without
a gap (sub-rows of one base that skip a letter leave the base incomplete).
Reported: missing rows, rows past the tree, incomplete bases, rows named twice,
and the distinct builds (builds()).

How the callers use it (the splice-resplit item does this; this module changes
neither file).  gles3-notpass.py: old_ranges() reads the head through
parse_head(lines) -> [(keys, build, run)] and merge_ranges() works on keys
(row_sort_key) instead of ints, so a spliced re-split row keeps its sub-row
label; the header is written with format_spec(keys), which joins neighbouring
keys into 'AA-BB' and prints sub-rows as '22a-22b'.  The line format
'# rows SPEC: build X, run Y' is the one RANGE_LINE matches today, with SPEC
widened from 'AA-BB'.  check-gles3-notpass-run.py: check_c1 calls
coverage(tree_rows, parsed) and fails on missing, past_tree, incomplete or
duplicate rows; builds(parsed) must have one element for c1.

Run `python3 gles3_rows.py --self-test`, or `python3 gles3_rows.py FILE.tsv
[TREE_ROWS]` to print a TSV head's rows and coverage.
"""
import re
import sys


class RowsError(ValueError):
    pass


KEY = re.compile(r"^(\d+)([a-z]*)$")
HEAD_LINE = re.compile(r"^# rows (\S+): build (\S+), run (\S+)\s*$")


def parse_key(text):
    m = KEY.match(text.strip())
    if not m:
        raise RowsError("bad row key %r (want 22 or 22a)" % text)
    return (int(m.group(1)), m.group(2))


def key_str(key):
    return "%d%s" % key


def row_sort_key(key):
    """Order: 22 < 22a < 22b < 22aa < 23 (shorter label first, then letters)."""
    return (key[0], len(key[1]), key[1])


def base_row(key):
    return key[0]


def _letters(lo, hi):
    if len(lo) != 1 or len(hi) != 1:
        raise RowsError("sub-row range %s-%s: only single-letter labels can be expanded" % (lo, hi))
    if lo > hi:
        raise RowsError("sub-row range runs backwards: %s-%s" % (lo, hi))
    return [chr(c) for c in range(ord(lo), ord(hi) + 1)]


def parse_spec(spec):
    """Row keys of a spec, sorted; a key named twice is an error."""
    if not spec or not spec.strip():
        raise RowsError("empty row spec")
    out = []
    for item in spec.split(","):
        item = item.strip()
        if not item:
            raise RowsError("empty item in row spec %r" % spec)
        parts = item.split("-")
        if len(parts) == 1:
            out.append(parse_key(parts[0]))
        elif len(parts) == 2:
            a, b = parse_key(parts[0]), parse_key(parts[1])
            if a[1] == "" and b[1] == "":
                if a[0] > b[0]:
                    raise RowsError("row range runs backwards: %s" % item)
                out.extend((r, "") for r in range(a[0], b[0] + 1))
            elif a[1] and b[1]:
                if a[0] != b[0]:
                    raise RowsError("sub-row range %s crosses rows (%d and %d)" % (item, a[0], b[0]))
                out.extend((a[0], l) for l in _letters(a[1], b[1]))
            else:
                raise RowsError("range %s mixes a whole row and a sub-row" % item)
        else:
            raise RowsError("bad range %r" % item)
    seen = set()
    for k in out:
        if k in seen:
            raise RowsError("row %s named twice in %r" % (key_str(k), spec))
        seen.add(k)
    return sorted(out, key=row_sort_key)


def _next(a, b):
    if a[1] == "" and b[1] == "":
        return b[0] == a[0] + 1
    return (a[0] == b[0] and len(a[1]) == 1 and len(b[1]) == 1
            and ord(b[1]) == ord(a[1]) + 1)


def format_spec(keys):
    """Inverse of parse_spec: joins neighbours into AA-BB and 22a-22c."""
    keys = sorted(set(keys), key=row_sort_key)
    items, i = [], 0
    while i < len(keys):
        j = i
        while j + 1 < len(keys) and _next(keys[j], keys[j + 1]):
            j += 1
        items.append(key_str(keys[i]) if i == j
                     else "%s-%s" % (key_str(keys[i]), key_str(keys[j])))
        i = j + 1
    return ",".join(items)


def parse_head(lines):
    """[(keys, build, run)] of every '# rows SPEC: build X, run Y' line, in file
    order.  Stops at the first line that is not a '#' comment."""
    out = []
    for line in lines:
        line = line.rstrip("\n")
        if not line.startswith("#"):
            break
        m = HEAD_LINE.match(line)
        if m:
            try:
                keys = parse_spec(m.group(1))
            except RowsError as e:
                raise RowsError("%s (in %r)" % (e, line))
            out.append((keys, m.group(2), m.group(3)))
        elif line.startswith("# rows "):
            raise RowsError("malformed rows line (want '# rows SPEC: build X, run Y'): %r" % line)
    return out


def coverage(tree_rows, parsed):
    """Dict of lists: missing (rows 0..tree_rows-1 not covered), past_tree
    (base rows >= tree_rows), incomplete (bases named only by sub-rows that skip
    a letter or do not start at 'a'), duplicate (keys named twice, or a base
    named whole and by sub-rows; as keys)."""
    whole, subs, dup = set(), {}, []
    for keys, _b, _r in parsed:
        for base, lab in keys:
            if lab == "":
                if base in whole or base in subs:
                    dup.append((base, lab))
                whole.add(base)
            else:
                if base in whole or lab in subs.get(base, ()):
                    dup.append((base, lab))
                subs.setdefault(base, set()).add(lab)
    covered, incomplete = set(whole), set()
    for base, labs in subs.items():
        want = {chr(ord("a") + i) for i in range(len(labs))}
        if labs == want:
            covered.add(base)
        elif base not in whole:
            incomplete.add(base)
    return {
        "missing": [r for r in range(tree_rows) if r not in covered and r not in incomplete],
        "past_tree": sorted(b for b in whole | set(subs) if b >= tree_rows),
        "incomplete": sorted(b for b in incomplete if b < tree_rows),
        "duplicate": sorted(set(dup), key=row_sort_key),
    }


def builds(parsed):
    return sorted({b for _k, b, _r in parsed})


def _self_test():
    fails = []

    def check(name, cond, detail=""):
        print("  ok    %s" % name if cond else "  FAIL  %s %s" % (name, detail))
        if not cond:
            fails.append(name)

    def refuses(spec, word):
        try:
            parse_spec(spec)
            check("refuse %r" % spec, False, "(no error)")
        except RowsError as e:
            check("refuse %r" % spec, word in str(e), "(%s lacks %r)" % (e, word))

    def keys(spec):
        return [key_str(k) for k in parse_spec(spec)]

    clean = {"missing": [], "past_tree": [], "incomplete": [], "duplicate": []}
    check("range", keys("0-3") == ["0", "1", "2", "3"])
    check("single", keys("22") == ["22"])
    check("sub", keys("22a") == ["22a"])
    check("sub range", keys("22a-22c") == ["22a", "22b", "22c"])
    check("list", keys("0-1,5,22a,22b") == ["0", "1", "5", "22a", "22b"])
    check("order", keys("23,22b,22,22a") == ["22", "22a", "22b", "23"])
    check("order aa after z", row_sort_key((22, "z")) < row_sort_key((22, "aa")) < row_sort_key((23, "")))
    check("base", base_row(parse_key("22b")) == 22 and base_row(parse_key("7")) == 7)
    check("format", format_spec(parse_spec("0-10,22a,22b,23")) == "0-10,22a-22b,23")
    check("format roundtrip", format_spec(parse_spec("0-3,5,7a-7c")) == "0-3,5,7a-7c")
    for spec, word in [("", "empty"), ("3-1", "backwards"), ("0-", "bad row key"),
                       ("a", "bad row key"), ("1,,2", "empty item"), ("22a-23", "mixes"),
                       ("22a-23b", "crosses"), ("22c-22a", "backwards"), ("1-2-3", "bad range"),
                       ("5,5", "twice"), ("22,22a-22b,22b", "twice"), ("-1", "bad row key"),
                       ("22A", "bad row key")]:
        refuses(spec, word)

    cur = ["# SPDX-License-Identifier: Apache-2.0\n", "# dEQP-GLES3 on the TI AM62P\n",
           "# rows 0-10: build b25997e4, run 2026-11-22\n",
           "# rows 11-45: build ad2b5ed, run 2026-10/14\n", "NotSupported\tdEQP-GLES3.x\tmsg\n",
           "# rows 0-45: build zzz, run never\n"]
    p = parse_head(cur)
    check("current head", [(format_spec(k), b, r) for k, b, r in p] ==
          [("0-10", "b25997e4", "2026-11-22"), ("11-45", "ad2b5ed", "2026-10/14")])
    check("current covers 46", coverage(46, p) == clean, coverage(46, p))
    check("current has two builds", builds(p) == ["ad2b5ed", "b25997e4"])
    one = parse_head(["# rows 0-45: build 31cdaefc, run 2026-11-25\n"])
    check("one build", builds(one) == ["31cdaefc"])

    re22 = parse_head(["# rows 0-21: build X, run d\n", "# rows 22a-22b: build X, run d\n",
                       "# rows 23-45: build X, run d\n"])
    check("re-split 22a/22b covers 46", coverage(46, re22) == clean, coverage(46, re22))
    t48 = parse_head(["# rows 0-47: build X, run d\n"])
    check("48-row tree", coverage(48, t48) == clean)
    check("rows past a 46-row tree", coverage(46, t48)["past_tree"] == [46, 47])
    check("46-row head on a 48-row tree", coverage(48, one)["missing"] == [46, 47])
    miss = parse_head(["# rows 0-20: build X, run d\n", "# rows 22-45: build X, run d\n"])
    check("missing row", coverage(46, miss)["missing"] == [21])
    gap = parse_head(["# rows 0-21: build X, run d\n", "# rows 22a,22c: build X, run d\n",
                      "# rows 23-45: build X, run d\n"])
    check("sub-row gap is incomplete", coverage(46, gap)["incomplete"] == [22]
          and coverage(46, gap)["missing"] == [])
    dup = parse_head(["# rows 0-45: build X, run d\n", "# rows 22a: build Y, run d\n"])
    check("whole and sub both named", coverage(46, dup)["duplicate"] == [(22, "a")], coverage(46, dup))
    two = parse_head(["# rows 0-10: build A, run d\n", "# rows 11-45: build B, run d\n"])
    check("two builds", builds(two) == ["A", "B"])
    for lines, word in [(["# rows 0-3 build X, run d\n"], "malformed"),
                        (["# rows 3-1: build X, run d\n"], "backwards")]:
        try:
            parse_head(lines)
            check("refuse head %r" % lines[0], False, "(no error)")
        except RowsError as e:
            check("refuse head %r" % lines[0], word in str(e))
    print("%d failed" % len(fails) if fails else "all ok")
    return 1 if fails else 0


def main(argv):
    if argv[1:2] == ["--self-test"]:
        return _self_test()
    if len(argv) not in (2, 3):
        print("usage: gles3_rows.py --self-test | FILE.tsv [TREE_ROWS]", file=sys.stderr)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as f:
            parsed = parse_head(f)
    except (OSError, RowsError) as e:
        print("gles3_rows: %s" % e, file=sys.stderr)
        return 1
    for k, b, r in parsed:
        print("rows %s: build %s, run %s" % (format_spec(k), b, r))
    print("builds: %s" % ", ".join(builds(parsed)))
    if len(argv) == 3:
        print("coverage: %s" % coverage(int(argv[2]), parsed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
