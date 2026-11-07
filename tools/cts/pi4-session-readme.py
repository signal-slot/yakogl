#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the README.md of a Pi 4 dEQP-GLES3 board session from its tallies.

Usage:
  tools/cts/pi4-session-readme.py --commit <full sha> [--tree FILE]
      [--times TSV] [--note TEXT]... [-o README.md] SESSION_DIR
  tools/cts/pi4-session-readme.py --self-test

SESSION_DIR is tools/cts/log/<date>-pi4-gles3-<sha8>-rowsAA-BB/ with a
<LABEL>.tally.txt (deqp-uart-tally.py --label LABEL) and a
<LABEL>.console.txt per image. The README (default: SESSION_DIR/README.md,
-o - for stdout) holds a 'commit <sha>' line (what check-pi4-gles3-log.py and
gles3-notpass.py --pi4 read), the row range and image labels of the tree
('# image <LABEL>:' lines, rows counted from 1), and a table per image:
the mustpass Pass / Fail / NotSupported / other counts of the tally's TOTAL
line, the image's own verdict, run time, GPU recoveries and hangs.

--times TSV gives what only the session knows, one tab-separated line per
image: LABEL, run time (e.g. 22.0 s), GPU recoveries, hangs. A label or a
field that is missing is '-' (unknown); nothing is made up. Paths in the
README are relative to SESSION_DIR; the text carries no home path, host
name, address or tty device.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOTAL = re.compile(r"^TOTAL\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$", re.M)
VERDICT = re.compile(r"\b[0-9a-f]{6,40} ((?:DONE|STOPPED|ABORTED|FATAL|INCOMPLETE)[^:;|]*)")
SUMMARY = re.compile(r"deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")
PRIVATE = re.compile(
    r"/home/|/Users/|(?<![\w.-])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])|\.ts\.net|/dev/tty")


def tree_labels(tree):
    out = []
    with open(tree, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*image\s+([^\s:]+)\s*:", line)
            if m and m.group(1) not in out:
                out.append(m.group(1))
    return out


def read_times(path):
    """{label: (run, recoveries, hangs)} from a TSV; '-' for blanks."""
    out = {}
    if not path:
        return out
    with open(path, encoding="utf-8") as f:
        for line in f:
            if not line.strip() or line.startswith("#"):
                continue
            c = [x.strip() or "-" for x in line.rstrip("\n").split("\t")]
            c += ["-"] * (4 - len(c))
            out[c[0]] = tuple(c[1:4])
    return out


def read_tally(path):
    """(pass, fail, n/s, other, verdict) of one tally; verdict from its text."""
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    t = TOTAL.search(text)
    counts = t.groups() if t else ("-",) * 4
    m = VERDICT.search(text)
    if m:
        verdict = m.group(1).strip()
    else:
        s = SUMMARY.search(text)
        if "complete=1" in text and s and s.group(3) == "0":
            verdict = "complete"
        else:
            verdict = "incomplete (no complete deqp summary)"
    return counts + (verdict,)


def build(sessdir, commit, tree, times, notes):
    labels = tree_labels(tree)
    if not labels:
        raise SystemExit("%s: no '# image <LABEL>:' lines" % tree)
    have = [lab for lab in labels
            if os.path.isfile(os.path.join(sessdir, lab + ".tally.txt"))]
    if not have:
        raise SystemExit("%s: no <LABEL>.tally.txt of the tree" % sessdir)
    rows = [labels.index(lab) + 1 for lab in have]
    lo, hi = min(rows), max(rows)
    name = os.path.basename(os.path.normpath(sessdir))
    out = [
        "<!-- SPDX-License-Identifier: Apache-2.0 -->",
        "<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->",
        "# Pi 4 dEQP-GLES3 rows %d-%d (%s) at %s" % (lo, hi, " ".join(have), commit[:8]),
        "",
        "commit %s" % commit,
        "",
        "Session `%s`: rows %d-%d of `tools/cts/pi4-gles3-tree.txt` (%d images: %s)."
        % (name, lo, hi, len(have), " ".join(have)),
        "Per-image files, relative to this directory: `<LABEL>.console.txt`, "
        "`<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL "
        "line; '-' is unknown.",
        "",
        "| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for lab in have:
        p, f, n, o, v = read_tally(os.path.join(sessdir, lab + ".tally.txt"))
        run, rec, hang = times.get(lab, ("-", "-", "-"))
        out.append("| %d | %s | %s | %s | %s | %s | %s | %s | %s | %s |"
                   % (labels.index(lab) + 1, lab, p, f, n, o, run, rec, hang, v))
    if notes:
        out.append("")
        out.extend(notes)
    text = "\n".join(out) + "\n"
    m = PRIVATE.search(text)
    if m:
        raise SystemExit("output holds a private detail %r" % m.group(0))
    return text


def self_test():
    sha = "ab" * 20
    fails = []

    def check(name, ok):
        print("%s %s" % ("ok  " if ok else "FAIL", name))
        if not ok:
            fails.append(name)

    with tempfile.TemporaryDirectory() as td:
        tree = os.path.join(td, "tree.txt")
        with open(tree, "w", encoding="utf-8") as f:
            f.write("# image A1: dEQP-GLES3.a\nx\n# image B2: dEQP-GLES3.b\ny\n"
                    "# image C3: dEQP-GLES3.c\nz\n")
        d = os.path.join(td, "2026-11-pi4-gles3-abababab-rows2-3")
        os.makedirs(d)
        tally = ("# t\n%s.console.txt: deqp: done: 5 passed, 1 failed, 0 not supported, "
                 "0 warnings, complete=1 | deqp: summary: 6 cases begun, 6 with a result, "
                 "0 without\nTOTAL    %d   1   0   0\n"
                 "  %s: abababab DONE: 6 cases, P 5 F 1\n")
        with open(os.path.join(d, "B2.tally.txt"), "w") as f:
            f.write(tally % ("B2", 5, "B2"))
        with open(os.path.join(d, "C3.tally.txt"), "w") as f:
            f.write("TOTAL 2 0 0 0\ndeqp: complete=0\n")
        tsv = os.path.join(td, "t.tsv")
        with open(tsv, "w") as f:
            f.write("B2\t12.5 s\t0\t0\n")
        text = build(d, sha, tree, read_times(tsv), [])
        check("commit line with the full sha",
              re.search(r"^commit %s$" % sha, text, re.M) is not None)
        check("row range and labels", "rows 2-3 of" in text and "B2 C3" in text)
        check("counts and verdict of B2",
              "| 2 | B2 | 5 | 1 | 0 | 0 | 12.5 s | 0 | 0 | DONE |" in text)
        check("unknown is '-' and an aborted image is named",
              "| 3 | C3 | 2 | 0 | 0 | 0 | - | - | - | incomplete" in text)
        check("image without a tally is not listed", "A1" not in text.split("\n", 6)[6])
        check("no private detail", PRIVATE.search(text) is None)
        try:
            build(d, sha, tree, {}, ["see " + "/ho" + "me/x"])
            check("a private note is refused", False)
        except SystemExit:
            check("a private note is refused", True)
        out = os.path.join(d, "README.md")
        with open(out, "w") as f:
            f.write(text)
        r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "check-pi4-gles3-log.py"),
                            "--commit", sha, "--tree", tree, d],
                           capture_output=True, text=True)
        check("check-pi4-gles3-log.py accepts the commit and finds no private detail",
              "README.md has no" not in r.stdout and "private detail" not in r.stdout
              and "names commit" not in r.stdout)
    print("pi4-session-readme self-test: %d failed" % len(fails))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--commit", help="full 40-digit sha of the session's build")
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--times", help="TSV: LABEL, run time, GPU recoveries, hangs")
    ap.add_argument("--note", action="append", default=[], help="a line of text under the table")
    ap.add_argument("-o", "--output", help="README path (default SESSION_DIR/README.md; - = stdout)")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("dir", nargs="?")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.dir or not a.commit:
        ap.error("SESSION_DIR and --commit are required")
    sha = a.commit.lower()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        ap.error("--commit needs a full 40-digit sha")
    text = build(a.dir, sha, a.tree, read_times(a.times), a.note)
    if a.output == "-":
        sys.stdout.write(text)
    else:
        with open(a.output or os.path.join(a.dir, "README.md"), "w", encoding="utf-8") as f:
            f.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
