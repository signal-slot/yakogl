#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the i.MX8MP ES 2.0 regression rows with the stage-1 baseline.

    python3 -I tools/cts/imx8mp-es2-regress.py [--baseline FILE] PATH...

PATH is a README.md (or any text file) or a directory (its README.md, else
its verdicts.txt, else its results.tsv). A *.tsv PATH is read in the baseline's
own shape. The text files carry the verdict lines tools/board.sh imx8mp prints
(docs/es3/board-recipe-imx8mp.md, "Expected verdict lines"):

    board.sh imx8mp e<NN> rc=<n> verdict=done ... resets=absent|0
        record=<w0>,<w1>,<w2>,<w3> back_after=<n>s ...

The line has no tally of its own; the DEQP record words do: w1 = passed (low
16 bits) and failed (high), w2 = not supported (low) and other (high), w3 low
16 bits = cases begun (as board.sh results_tsv derives them; a three-word
record has no w3 and no-result is then unknown, counted 0). Pages
(e<NN>-p<page>) are ignored; the last line of an entry wins.

The baseline, tools/cts/imx8mp-full-tree-29460f96.results.tsv, holds per-entry
counts (begun passed failed notsup other noresult), so every entry is compared
on counts. Per entry:
    same     the same passed and the same failed+other+noresult
    better   more passed or fewer bad cases, none worse
    WORSE    fewer passed or more failed/other/noresult, or the row is not
             clean (rc != 0, verdict other than done, resets present)
    missing  no verdict line (or no usable record) for the entry
Last line: 'es2 regress: <n>/29 entries, <w> worse, <m> missing'. Exit 0 only
if all 29 entries are present and none is worse; 1 otherwise; 2 bad input.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BASELINE = os.path.join(HERE, "imx8mp-full-tree-29460f96.results.tsv")
LINE = re.compile(r"board\.sh imx8mp e(\d+)(-p\d+)? rc=(-?\d+)(.*)")
REC = re.compile(r"\brecord=([0-9a-fA-F]+(?:,[0-9a-fA-F]+){2,3})\b")


def counts(tb, tp, tf, tn, to):
    nr = tb - tp - tf - tn - to if tb is not None else 0
    return dict(passed=tp, failed=tf, notsup=tn, other=to, noresult=max(nr, 0))


def read_tsv(path):
    out = {}
    for ln in open(path, errors="replace"):
        if ln.startswith("#") or not ln.strip():
            continue
        f = ln.rstrip("\n").split("\t")
        if len(f) < 8 or "-" in f[2:8]:
            continue
        _, p, fl, n, o, nr = (int(x) for x in f[2:8])
        out[int(f[0])] = dict(passed=p, failed=fl, notsup=n, other=o,
                              noresult=nr, clean=True)
    return out


def check_words(name, w, tb, p, f, n, o, b):
    """Refuse a record the fixed word layout cannot explain (ValueError)."""
    where = "%s: record=%s: " % (name, ",".join("%08x" % x for x in w))
    if (w[0] >> 8) & 0xFF != 8:
        raise ValueError(where + "w0 kind byte is %d, not 8 (DEQP): "
                         "unknown record encoding" % ((w[0] >> 8) & 0xFF))
    if tb is not None and p + f + n + o > tb:
        raise ValueError(where + "passed+failed+notsup+other = %d exceeds "
                         "cases begun %d: unknown record encoding" %
                         (p + f + n + o, tb))
    # a halves swap (w1 or w2) keeps every sum; the baseline gives it away:
    # the swapped reading equals the baseline where the fixed one does not
    if b is not None:
        for what, got, swp, want in (
                ("w1 (passed/failed)", (p, f), (f, p), (b["passed"], b["failed"])),
                ("w2 (notsup/other)", (n, o), (o, n), (b["notsup"], b["other"]))):
            if got != want and swp == want:
                raise ValueError(where + "%s halves look swapped (read %s, "
                                 "the baseline has %s): unknown record "
                                 "encoding" % (what, got, want))


def read_text(path, bl=None):
    out = {}
    for ln in open(path, errors="replace"):
        m = LINE.search(ln)
        if not m or m.group(2):
            continue
        rest = m.group(0)
        i = int(m.group(1))
        clean = m.group(3) == "0" and "verdict=done" in rest and \
            re.search(r"resets=(absent|0)\b", rest) is not None
        r = REC.search(rest)
        if not r:
            out.pop(i, None)
            continue
        w = [int(x, 16) for x in r.group(1).split(",")]
        tb = (w[3] & 0xFFFF) if len(w) > 3 else None
        p, f, n, o = w[1] & 0xFFFF, w[1] >> 16, w[2] & 0xFFFF, w[2] >> 16
        check_words("%s e%02d" % (path, i), w, tb, p, f, n, o,
                    (bl or {}).get(i))
        c = counts(tb, p, f, n, o)
        c["clean"] = clean
        out[i] = c
    return out


def read_path(p, bl=None):
    if os.path.isdir(p):
        for n in ("README.md", "verdicts.txt", "results.tsv"):
            if os.path.isfile(os.path.join(p, n)):
                p = os.path.join(p, n)
                break
        else:
            raise OSError("no README.md, verdicts.txt or results.tsv in " + p)
    return read_tsv(p) if p.endswith(".tsv") else read_text(p, bl)


def bad(c):
    return c["failed"] + c["other"] + c["noresult"]


def judge(b, r):
    if not r["clean"] or r["passed"] < b["passed"] or bad(r) > bad(b):
        return "WORSE"
    if r["passed"] > b["passed"] or bad(r) < bad(b):
        return "better"
    return "same"


def main(argv):
    base = BASELINE
    args = argv[1:]
    if args[:1] == ["--baseline"] and len(args) > 1:
        base, args = args[1], args[2:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    try:
        bl = read_tsv(base)
        got = {}
        for p in args:
            got.update(read_path(p, bl))
    except (OSError, ValueError) as e:
        print("es2 regress: " + str(e), file=sys.stderr)
        return 2
    if len(bl) != 29:
        print("es2 regress: baseline has %d entries, not 29" % len(bl),
              file=sys.stderr)
        return 2
    n = w = m = 0
    for i in sorted(bl):
        b, r = bl[i], got.get(i)
        if r is None:
            m += 1
            print("e%02d missing" % i)
            continue
        n += 1
        v = judge(b, r)
        w += v == "WORSE"
        print("e%02d %s passed %d->%d bad %d->%d%s" % (
            i, v, b["passed"], r["passed"], bad(b), bad(r),
            "" if r["clean"] else " (row not clean)"))
    print("es2 regress: %d/29 entries, %d worse, %d missing" % (n, w, m))
    return 0 if n == 29 and w == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
