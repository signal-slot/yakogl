#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""host-rank.py - rank the Mali online compiler's refusal texts by the
must-pass dEQP cases whose FIRST refusal on the host each text is.

Usage:
  python3 -I tools/mali/host-rank.py [--mustpass FILE] [--threshold N] OUT_DIR
  python3 -I tools/mali/host-rank.py --self-test

OUT_DIR is the output directory of tools/mali/deqp-host-measure.sh: its
measure.tsv (one row per case, columns case, board_status, board_family,
host_status, stage, host_family, shape, host_refusal, next_family) is read;
summary.md must be beside it (its first line is checked, so a wrong
directory is an error). A row with a non-empty stage is a case with a host
refusal; host_refusal is already that first refusal, as the Mali(Valhall: ..)
reason cut at 'is not lowered' (link refusals without the case's own varying),
so it is the text. Rows without a refusal (Pass, NotRun, crash, ...) are
counted apart. Only cases in --mustpass (default tools/cts/gles2-mustpass.txt)
count; others are reported as a count.

Output: a markdown table (rank, text, stage vs/fs/link/be/both, must-pass
cases blocked, share) and a total line. Exit 1 if any text blocks THRESHOLD
(default 100) or more cases, else 0; exit 2 on a usage or input error.
HOST-ONLY: compile and link outcomes of the host library, no render.
"""
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MUSTPASS = os.path.join(HERE, "..", "cts", "gles2-mustpass.txt")
HEADER = ["case", "board_status", "board_family", "host_status", "stage",
          "host_family", "shape", "host_refusal"]


class Fatal(Exception):
    pass


def load_mustpass(path):
    try:
        with open(path, encoding="utf-8") as f:
            return {ln.strip() for ln in f if ln.strip() and not ln.startswith("#")}
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))


def read_measure(d):
    """[(case, host_status, stage, text)] of OUT_DIR/measure.tsv, one per case
    (the first row of a case)."""
    if not os.path.isdir(d):
        raise Fatal("%s: not a directory" % d)
    if not os.path.isfile(os.path.join(d, "summary.md")):
        raise Fatal("%s: no summary.md (not a deqp-host-measure.sh output dir)" % d)
    p = os.path.join(d, "measure.tsv")
    try:
        with open(p, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError as e:
        raise Fatal("%s: %s" % (p, e.strerror))
    if not lines or lines[0].split("\t")[:len(HEADER)] != HEADER:
        raise Fatal("%s: header does not start with %s" % (p, "<TAB>".join(HEADER)))
    seen, rows = set(), []
    for ln in lines[1:]:
        if not ln.strip():
            continue
        f = ln.split("\t")
        f += [""] * (len(HEADER) - len(f))
        if f[0] in seen:
            continue
        seen.add(f[0])
        rows.append((f[0], f[3], f[4], f[7].replace("\t", " ").strip()))
    return rows


def rank(d, mustpass):
    """(rows, total, outside, norefusal): rows = [(text, stage, n)]."""
    cnt, stages, outside, none = {}, {}, 0, 0
    for case, _status, stage, text in read_measure(d):
        if case not in mustpass:
            outside += 1
        elif not stage:
            none += 1
        else:
            text = text or "-"
            cnt[text] = cnt.get(text, 0) + 1
            stages.setdefault(text, set()).add(stage)
    rows = [(w, next(iter(stages[w])) if len(stages[w]) == 1 else "both", n)
            for w, n in cnt.items()]
    rows.sort(key=lambda r: (-r[2], r[0]))
    return rows, sum(cnt.values()), outside, none


def render(rows, total, outside, none):
    out = ["| rank | text | stage | must-pass cases blocked | share |",
           "|--:|---|---|--:|--:|"]
    for i, (w, st, n) in enumerate(rows, 1):
        out.append("| %d | %s | %s | %d | %.1f%% |" % (
            i, w.replace("|", "\\|"), st, n, 100.0 * n / total if total else 0))
    out.append("| | **total** (%d texts) | | **%d** | 100.0%% |" % (len(rows), total))
    out.append("")
    out.append("%d must-pass case(s) without a host refusal (Pass, NotRun, crash) and "
               "%d case(s) not in the must-pass list left out. HOST-ONLY." % (none, outside))
    return "\n".join(out)


def main(argv, quiet=False):
    mustpass, threshold, dirs = DEFAULT_MUSTPASS, 100, []
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
    if len(dirs) != 1:
        raise Fatal("exactly one OUT_DIR needed (see --help)")
    rows, total, outside, none = rank(dirs[0], load_mustpass(mustpass))
    if not quiet:
        print(render(rows, total, outside, none))
    over = [r for r in rows if r[2] >= threshold]
    for w, _s, n in over if not quiet else []:
        print("over threshold %d: %s (%d)" % (threshold, w, n), file=sys.stderr)
    return 1 if over else 0


def self_test():
    fx = os.path.join(HERE, "testdata", "host_rank")
    d, mp = os.path.join(fx, "out"), os.path.join(fx, "mustpass.txt")
    fails = []

    def chk(name, ok):
        if not ok:
            fails.append(name)
        print("%s %s" % ("ok  " if ok else "FAIL", name))

    must = load_mustpass(mp)
    rows, total, outside, none = rank(d, must)
    chk("ranked, first row per case, stage kept, 'both' when mixed",
        rows == [("an integer or boolean value", "both", 3),
                 ("beta cut", "vs", 1),
                 ("gamma: no stage word", "link", 1)])
    chk("total, outside, no-refusal counts", (total, outside, none) == (5, 1, 2))
    md = render(rows, total, outside, none)
    chk("markdown table has rank, share and total line",
        md.startswith("| rank |") and "| 1 | an integer" in md and "**5**" in md
        and "60.0%" in md)
    a = [d, "--mustpass", mp]
    chk("--threshold 4 passes", main(a + ["--threshold", "4"], True) == 0)
    chk("--threshold 3 fails (a text blocks exactly 3)",
        main(a + ["--threshold", "3"], True) == 1)
    chk("default threshold 100 passes", main(a, True) == 0)
    with tempfile.TemporaryDirectory() as t:
        try:
            read_measure(t)
            chk("dir without summary.md is an error", False)
        except Fatal:
            chk("dir without summary.md is an error", True)
        open(os.path.join(t, "summary.md"), "w").close()
        with open(os.path.join(t, "measure.tsv"), "w") as f:
            f.write("wrong\theader\n")
        try:
            read_measure(t)
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
        print("host-rank.py: %s" % e, file=sys.stderr)
        sys.exit(2)
