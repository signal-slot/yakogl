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

board.sh prints the words bare (no field names), so the encoding is checked
before it is read; a record that does not fit stops the tool (exit 2, the
entry and the reason on stderr) instead of turning a regression into an
improvement or the reverse:
    - record= is absent, or three or four hex words; anything else is refused
    - w0 must be a record (magic 0x594b) of kind DEQP (8), and agree with the
      line's result=record:<KIND>; on a row that is not clean (the record may
      be a stale or breadcrumb one) a non-DEQP record is read as no result
    - passed+failed+notsup+other <= cases begun (w3) <= the entry's case
      total (the baseline's begun); without w3, the sum <= the case total
    - w1 or w2 with its halves swapped: the fixed reading differs from the
      baseline while the swapped one equals it (a swap keeps every sum, so
      only the baseline can give it away; a swap that also moved counts is
      not seen)

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
REC = re.compile(r"\brecord=(\S*)")
WORDS = re.compile(r"[0-9a-fA-F]{1,8}(?:,[0-9a-fA-F]{1,8}){2,3}$")
KIND = re.compile(r"\bresult=record:(\S+)")
MAGIC, DEQP = 0x594B, 8


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
        bg, p, fl, n, o, nr = (int(x) for x in f[2:8])
        out[int(f[0])] = dict(begun=bg, passed=p, failed=fl, notsup=n, other=o,
                              noresult=nr, clean=True)
    return out


def check_words(where, w, clean, kname, b):
    """The record's counts, or None for a not-clean row's non-DEQP record.
    ValueError for words the DEQP layout cannot explain."""
    where += ": record=%s: " % ",".join("%08x" % x for x in w)
    kind = (w[0] >> 8) & 0xFF if w[0] >> 16 == MAGIC else None
    if kind != DEQP:
        if not clean:
            return None
        raise ValueError(where + "w0 is %s, not a DEQP record (magic 594b, "
                         "kind 8): unknown record encoding" %
                         ("kind %d" % kind if kind is not None else "no record"))
    if kname is not None and kname != "DEQP":
        raise ValueError(where + "w0 says kind DEQP, the line says "
                         "result=record:%s: unknown record encoding" % kname)
    p, f, n, o = w[1] & 0xFFFF, w[1] >> 16, w[2] & 0xFFFF, w[2] >> 16
    total = b["begun"] if b is not None else None
    tb = (w[3] & 0xFFFF) if len(w) > 3 else None
    if tb is not None and p + f + n + o > tb:
        raise ValueError(where + "passed+failed+notsup+other = %d exceeds cases "
                         "begun %d: unknown record encoding" % (p + f + n + o, tb))
    top = tb if tb is not None else p + f + n + o
    if total is not None and top > total:
        raise ValueError(where + "%s %d exceeds the entry's %d cases: unknown "
                         "record encoding" % ("cases begun" if tb is not None
                                              else "passed+failed+notsup+other",
                                              top, total))
    if b is not None:
        for what, got, want in (
                ("w1 (passed | failed << 16)", (p, f), (b["passed"], b["failed"])),
                ("w2 (notsup | other << 16)", (n, o), (b["notsup"], b["other"]))):
            if got != want and got[::-1] == want:
                raise ValueError(where + "%s reads %d,%d where the baseline has "
                                 "%d,%d: halves swapped, unknown record encoding"
                                 % (what, got[0], got[1], want[0], want[1]))
    return counts(tb, p, f, n, o)


def read_text(path, bl=None):
    bl = bl or {}
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
        if not r or r.group(1) == "absent":
            out.pop(i, None)
            continue
        where = "%s e%02d" % (path, i)
        if not WORDS.match(r.group(1)):
            raise ValueError("%s: record=%s is neither 'absent' nor three or four "
                             "hex words: unknown record encoding" %
                             (where, r.group(1)))
        w = [int(x, 16) for x in r.group(1).split(",")]
        k = KIND.search(rest)
        b = bl.get(i)
        c = check_words(where, w, clean, k.group(1) if k else None, b)
        if c is None:
            # a stale or breadcrumb record on a failed row: no tally, all lost
            c = counts(b["begun"] if b else 0, 0, 0, 0, 0)
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
