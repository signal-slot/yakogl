#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""refusal-rank.py - rank the Mali online compiler's refusal texts by the
must-pass dEQP cases whose FIRST refusal each text is.

Usage:
  python3 -I tools/mali/refusal-rank.py [--mustpass FILE] [--threshold N]
                                        RECORD_DIR...
  python3 -I tools/mali/refusal-rank.py --self-test

A RECORD_DIR is a board run's record directory (records/<board>/<name>/) or
one slice; every refusals.tsv below it (the directory itself included) is
read, in sorted path order. Format (written by tools/cts/mali-deqp-run.sh and
tools/cts/imx95-uart-slice.py): a header line "case<TAB>refusal", then one
row per case holding the case's first refusal text. A case in several files
(a rerun) counts once, the later file winning, as refusal-compare.sh does.
Only cases listed in --mustpass (default tools/cts/gles2-mustpass.txt next
to this checkout) count; others are reported as "not must-pass".

The text of a row is the cause as refusal-compare.sh reads it: "...refused
this <stage> shader: Mali(<why>)" gives <why> (stage shown in its own
column), any other text is kept whole. Output is a markdown table: rank,
text, stage (vs/fs/both/-), must-pass cases blocked, share, then a total line.
Exit 1 if any text blocks THRESHOLD (default 100) or more cases, else 0;
exit 2 on a usage or input error.
"""
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MUSTPASS = os.path.join(HERE, "..", "cts", "gles2-mustpass.txt")
REFUSED = re.compile(r"refused this ([A-Za-z0-9_]+) shader: ")


class Fatal(Exception):
    pass


def cause(text):
    """(stage, why) of a refusals.tsv text; stage '-' when it names none."""
    text = text.replace("\t", " ").strip()
    m = REFUSED.search(text)
    if not m:
        return "-", text
    why = text[m.end():]
    if why.startswith("Mali(") :
        why = why[5:]
        if why.endswith(")"):
            why = why[:-1]
    stage = {"vertex": "vs", "fragment": "fs"}.get(m.group(1), m.group(1))
    return stage, why.strip()


def find_files(d):
    if not os.path.isdir(d):
        raise Fatal("%s: not a directory" % d)
    out = []
    for root, dirs, files in os.walk(d):
        dirs.sort()
        if "refusals.tsv" in files:
            out.append(os.path.join(root, "refusals.tsv"))
    if not out:
        raise Fatal("%s: no refusals.tsv below it" % d)
    return out


def read_first(path):
    """{case: text} of one file: the first row of each case."""
    rows = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    if not lines or lines[0].split("\t")[:2] != ["case", "refusal"]:
        raise Fatal("%s: header is not 'case<TAB>refusal'" % path)
    for ln in lines[1:]:
        if not ln.strip():
            continue
        case, _, text = ln.partition("\t")
        if case not in rows:
            rows[case] = text
    return rows


def load_mustpass(path):
    try:
        with open(path, encoding="utf-8") as f:
            return {ln.strip() for ln in f if ln.strip() and not ln.startswith("#")}
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))


def rank(dirs, mustpass):
    """(rows, total, outside): rows = [(why, stages, n)] most blocking first."""
    first = {}
    for d in dirs:
        for p in find_files(d):
            first.update(read_first(p))
    cnt, stages, outside = {}, {}, 0
    for case, text in first.items():
        if case not in mustpass:
            outside += 1
            continue
        st, why = cause(text)
        cnt[why] = cnt.get(why, 0) + 1
        stages.setdefault(why, set()).add(st)
    rows = [(w, "/".join(sorted(stages[w])) if len(stages[w]) == 1 else "both", n)
            for w, n in cnt.items()]
    rows.sort(key=lambda r: (-r[2], r[0]))
    return rows, sum(cnt.values()), outside


def render(rows, total, outside):
    out = ["| rank | text | stage | must-pass cases blocked | share |",
           "|--:|---|---|--:|--:|"]
    for i, (w, st, n) in enumerate(rows, 1):
        out.append("| %d | %s | %s | %d | %.1f%% |" % (
            i, w.replace("|", "\\|"), st, n, 100.0 * n / total if total else 0))
    out.append("| | **total** (%d texts) | | **%d** | 100.0%% |" % (len(rows), total))
    out.append("")
    out.append("%d refused case(s) not in the must-pass list left out." % outside)
    return "\n".join(out)


def main(argv, quiet=False):
    mustpass = DEFAULT_MUSTPASS
    threshold = 100
    dirs = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--mustpass", "--threshold") and i + 1 < len(argv):
            if a == "--mustpass":
                mustpass = argv[i + 1]
            else:
                try:
                    threshold = int(argv[i + 1])
                except ValueError:
                    raise Fatal("--threshold needs an integer")
            i += 2
        elif a in ("-h", "--help"):
            print(__doc__)
            return 0
        elif a.startswith("-"):
            raise Fatal("unknown option %s" % a)
        else:
            dirs.append(a)
            i += 1
    if not dirs:
        raise Fatal("no record directory (see --help)")
    rows, total, outside = rank(dirs, load_mustpass(mustpass))
    if not quiet:
        print(render(rows, total, outside))
    over = [r for r in rows if r[2] >= threshold]
    for w, _s, n in over if not quiet else []:
        print("over threshold %d: %s (%d)" % (threshold, w, n), file=sys.stderr)
    return 1 if over else 0


def self_test():
    fx = os.path.join(HERE, "testdata", "refusal_rank")
    mp = os.path.join(fx, "mustpass.txt")
    a, b = os.path.join(fx, "recA"), os.path.join(fx, "recB")
    fails = []

    def chk(name, ok):
        if not ok:
            fails.append(name)
        print("%s %s" % ("ok  " if ok else "FAIL", name))

    must = load_mustpass(mp)
    rows, total, outside = rank([a], must)
    got = [(w, st, n) for w, st, n in rows]
    # recA: s1 has t1.a (twice, first text wins), t1.b, t1.c, t2.a; s2 reruns
    # t1.c with another text (later wins) and holds t3.a not in must-pass.
    chk("first refusal per case, rerun wins, ranked",
        got == [("alpha is not lowered yet", "fs", 2),
                ("beta cut", "vs", 1),
                ("gamma: no stage word", "-", 1)])
    chk("total and outside", total == 4 and outside == 1)
    rows, total, _ = rank([a, b], must)
    chk("two records add up, text in both stages is 'both'",
        total == 6 and rows[0] == ("alpha is not lowered yet", "both", 4))
    md = render(rows, total, 0)
    chk("markdown table has rank, share and total line",
        md.startswith("| rank |") and "| 1 | alpha" in md and "**6**" in md
        and "66.7%" in md)
    chk("--threshold 5 passes", main([a, b, "--mustpass", mp, "--threshold", "5"], True) == 0)
    chk("--threshold 4 fails (a text blocks exactly 4)",
        main([a, b, "--mustpass", mp, "--threshold", "4"], True) == 1)
    chk("default threshold 100 passes", main([a, "--mustpass", mp], True) == 0)
    with tempfile.TemporaryDirectory() as t:
        try:
            find_files(t)
            chk("empty directory is an error", False)
        except Fatal:
            chk("empty directory is an error", True)
        with open(os.path.join(t, "refusals.tsv"), "w") as f:
            f.write("wrong\theader\n")
        try:
            rank([t], must)
            chk("bad header is an error", False)
        except Fatal:
            chk("bad header is an error", True)
    print("%d failed" % len(fails) if fails else "all ok")
    return 1 if fails else 0


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--self-test"]:
            sys.exit(self_test())
        sys.exit(main(sys.argv[1:]))
    except Fatal as e:
        print("refusal-rank.py: %s" % e, file=sys.stderr)
        sys.exit(2)
