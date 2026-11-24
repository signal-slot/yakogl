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
-o - for stdout) holds a 'commit: <sha>' line (what check-pi4-gles3-log.py and
gles3-notpass.py --pi4 read), the row range and image labels of the tree
('# image <LABEL>:' lines, rows counted from 1), and a table per image:
the mustpass Pass / Fail / NotSupported / other counts of the tally's TOTAL
line, the image's own verdict, run time, GPU recoveries and hangs.

--times TSV gives what only the session knows, one tab-separated line per
image: LABEL, run time (e.g. 22.0 s), GPU recoveries, hangs. A label or a
field that is missing is '-' (unknown); nothing is made up. Paths in the
README are relative to SESSION_DIR; the text carries no home path, host
name, address or tty device.

A cut (docs/es3/board-recipe-pi4.md section 6): an image dEQP aborted whose
label the tree has since cut after the abort trigger, the cases it never
began moved to later labels with their own tallies. Those later tallies may
be in other session directories of the same commit: the sibling directories
'*-pi4-gles3-<sha8>-*' of SESSION_DIR, and each --with DIR. The cut is
judged by check-pi4-gles3-log.py's own rule and named in its words: the
image keeps its verdict, followed by '; cut after abort trigger <case>
(<N> begun, <K> moved to <LABELS>)', and a line under the table names the
tally each moved case went to, so the K never-begun cases read as moved,
not lost. An aborted image that is not a cut, and every finished image,
reads as before.
"""
import argparse
import glob
import importlib.util
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOTAL = re.compile(r"^TOTAL\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$", re.M)
VERDICT = re.compile(r"\b[0-9a-f]{6,40} ((?:DONE|STOPPED|ABORTED|FATAL|INCOMPLETE)[^:;|]*)")
STOPPED = re.compile(r"STOPPED: dEQP ended the session early")
GROUP = re.compile(r"^(?!TOTAL\b)([A-Za-z_][\w.]*)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$", re.M)
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
    if t:
        # the group rows above TOTAL must add up to it
        rows = [list(map(int, g.groups()[1:])) for g in GROUP.finditer(text)]
        if rows and [sum(c) for c in zip(*rows)] != [int(x) for x in counts]:
            raise SystemExit("%s: the group rows do not add up to TOTAL" % path)
    m = VERDICT.search(text)
    s = SUMMARY.search(text)
    if m:
        verdict = m.group(1).strip()
    elif STOPPED.search(text):
        verdict = "STOPPED after %s cases (dEQP ended the session early)" % (
            s.group(1) if s else "?")
    else:
        if "complete=1" in text and s and s.group(3) == "0":
            verdict = "complete"
        else:
            verdict = "incomplete (no complete deqp summary)"
    return counts + (verdict,)


def logcheck():
    """check-pi4-gles3-log.py as a module: its cut rule and wording."""
    spec = importlib.util.spec_from_file_location(
        "check_pi4_gles3_log", os.path.join(HERE, "check-pi4-gles3-log.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def cuts(sessdir, commit, tree, have, others, mustpass):
    """{label: (why, [relative tally paths of the labels its cases moved to])}
    for each image of the session that check-pi4-gles3-log.py counts as cut."""
    lc = logcheck()
    counts = {}
    lc.labels_of(tree, counts)
    aborted = []
    for lab in have:
        p = os.path.join(sessdir, lab + ".tally.txt")
        if lc.complete(p) or (counts.get(lab) == 1 and lc.refused_one_case(p)):
            continue
        aborted.append(lab)
    if not aborted:
        return {}
    own = os.path.realpath(sessdir)
    dirs = [sessdir]
    sib = os.path.join(os.path.dirname(own), "*-pi4-gles3-%s-*" % commit[:8])
    for d in sorted(glob.glob(sib)) + list(others):
        if os.path.isdir(d) and os.path.realpath(d) not in map(os.path.realpath, dirs):
            dirs.append(d)
    where = {}
    for d in dirs:
        for p in sorted(glob.glob(os.path.join(d, "*.tally.txt"))):
            where.setdefault(os.path.basename(p)[: -len(".tally.txt")], []).append(p)
    try:
        mp = lc.read_mustpass(mustpass)
    except OSError:
        mp = []
    exps = {k: lc.expand(v, mp) for k, v in lc.tree_cases(tree).items()}
    out = {}
    for lab in aborted:
        ok, why = lc.cut_verdict(lab, where[lab], mp, exps, where)
        if not ok:
            continue
        m = re.search(r"moved to (.*)\)$", why)
        subs = m.group(1).split() if m else []
        out[lab] = (why, [os.path.relpath(where[s][0], sessdir)
                          for s in subs if where.get(s)])
    return out


def build(sessdir, commit, tree, times, notes, others=(), mustpass=None):
    labels = tree_labels(tree)
    if not labels:
        raise SystemExit("%s: no '# image <LABEL>:' lines" % tree)
    have = [lab for lab in labels
            if os.path.isfile(os.path.join(sessdir, lab + ".tally.txt"))]
    if not have:
        raise SystemExit("%s: no <LABEL>.tally.txt of the tree" % sessdir)
    extra = sorted(f[:-len(".tally.txt")] for f in os.listdir(sessdir)
                   if f.endswith(".tally.txt") and f[:-len(".tally.txt")] not in labels)
    if extra:
        raise SystemExit("%s: tally of a label not in the tree: %s" % (sessdir, " ".join(extra)))
    rows = [labels.index(lab) + 1 for lab in have]
    lo, hi = min(rows), max(rows)
    name = os.path.basename(os.path.normpath(sessdir))
    out = [
        "<!-- SPDX-License-Identifier: Apache-2.0 -->",
        "<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->",
        "# Pi 4 dEQP-GLES3 rows %d-%d (%s) at %s" % (lo, hi, " ".join(have), commit[:8]),
        "",
        "commit: %s" % commit,
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
    cut = cuts(sessdir, commit, tree, have, others,
               mustpass or os.path.join(HERE, "gles3-mustpass.txt"))
    for lab in have:
        p, f, n, o, v = read_tally(os.path.join(sessdir, lab + ".tally.txt"))
        if lab in cut:
            v += "; " + cut[lab][0]
        run, rec, hang = times.get(lab, ("-", "-", "-"))
        out.append("| %d | %s | %s | %s | %s | %s | %s | %s | %s | %s |"
                   % (labels.index(lab) + 1, lab, p, f, n, o, run, rec, hang, v))
    if cut:
        out.append("")
        out.append("Cut images (`docs/es3/board-recipe-pi4.md` section 6): the label keeps "
                   "its cases up to the abort trigger; the cases dEQP never began after it "
                   "moved to later labels and are counted there, not lost.")
        for lab in have:
            if lab in cut:
                why, paths = cut[lab]
                out.append("- %s: %s; %s %s." % (
                    lab, why, "tallies" if len(paths) > 1 else "tally",
                    ", ".join("`%s`" % p for p in paths) or "-"))
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
              re.search(r"^commit: %s$" % sha, text, re.M) is not None)
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

    # testdata/pi4-session-readme/cut/: K aborted at k.c (its filter k.*),
    # the tree cut K after k.c, its 3 never-begun cases ran as K2 in a
    # later session directory of the same commit.
    cd = os.path.join(HERE, "testdata", "pi4-session-readme", "cut")
    ctree, cmp = os.path.join(cd, "tree.txt"), os.path.join(cd, "mustpass.txt")
    s1 = os.path.join(cd, "2026-11-pi4-gles3-abababab-s1")
    s2 = os.path.join(cd, "2026-11-pi4-gles3-abababab-s2")
    ctimes = read_times(os.path.join(cd, "times.tsv"))
    why = "cut after abort trigger dEQP-GLES3.k.c (3 begun, 3 moved to K2)"
    text = build(s1, sha, ctree, ctimes, [], (), cmp)
    check("cut: finished image unchanged",
          "| 1 | L1 | 2 | 0 | 0 | 0 | 1.2 s | 0 | none | DONE, all passed |\n" in text)
    check("cut: aborted image keeps its verdict and counts, then the cut",
          "| 2 | K | 2 | 0 | 0 | 1 | 0.9 s | 0 | none | "
          "incomplete (no complete deqp summary); %s |\n" % why in text)
    check("cut: the never-begun cases are moved to K2 (its tally named), not lost",
          "- K: %s; tally `../2026-11-pi4-gles3-abababab-s2/K2.tally.txt`." % why in text
          and "not lost" in text)
    check("cut: K2 (another session) is not a row here", "| K2 |" not in text)
    check("cut: no private detail", PRIVATE.search(text) is None)
    with tempfile.TemporaryDirectory() as td:
        # the same wording as check-pi4-gles3-log.py's info line
        for s, lab in ((s1, "K"), (s1, "L1"), (s2, "K2")):
            d = os.path.join(td, os.path.basename(s))
            os.makedirs(d, exist_ok=True)
            for ext in (".tally.txt", ".console.txt"):
                if os.path.isfile(os.path.join(s, lab + ext)):
                    with open(os.path.join(s, lab + ext)) as f, \
                            open(os.path.join(d, lab + ext), "w") as g:
                        g.write(f.read())
        d1 = os.path.join(td, os.path.basename(s1))
        d2 = os.path.join(td, os.path.basename(s2))
        for d in (d1, d2):
            with open(os.path.join(d, "README.md"), "w") as f:
                f.write(build(d, sha, ctree, ctimes, [], (), cmp))
        r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "check-pi4-gles3-log.py"),
                            "--commit", sha, "--tree", ctree, "--mustpass", cmp, d1, d2],
                           capture_output=True, text=True)
        check("cut: check-pi4-gles3-log.py passes the record with the same words",
              r.returncode == 0 and ("info: K: %s" % why) in r.stdout)
        # alone (no K2 tally anywhere) K is an abort, not a cut: as before
        os.rename(d2, os.path.join(td, "elsewhere"))
        text = build(d1, sha, ctree, ctimes, [], (), cmp)
        check("cut: without the remainder's tally K reads as before",
              "| 2 | K | 2 | 0 | 0 | 1 | 0.9 s | 0 | none | "
              "incomplete (no complete deqp summary) |\n" in text and "cut" not in text)
        text = build(d1, sha, ctree, ctimes, [], [os.path.join(td, "elsewhere")], cmp)
        check("cut: --with DIR finds the remainder outside the siblings",
              "moved to K2" in text and "`../elsewhere/K2.tally.txt`" in text)
    print("pi4-session-readme self-test: %d failed" % len(fails))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--commit", help="full 40-digit sha of the session's build")
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--times", help="TSV: LABEL, run time, GPU recoveries, hangs")
    ap.add_argument("--note", action="append", default=[], help="a line of text under the table")
    ap.add_argument("--with", dest="others", action="append", default=[], metavar="DIR",
                    help="another session directory of the commit (for cuts; the "
                    "'*-pi4-gles3-<sha8>-*' siblings are read anyway)")
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"),
                    help="mustpass list the tree globs expand against (for cuts)")
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
    text = build(a.dir, sha, a.tree, read_times(a.times), a.note, a.others, a.mustpass)
    if a.output == "-":
        sys.stdout.write(text)
    else:
        with open(a.output or os.path.join(a.dir, "README.md"), "w", encoding="utf-8") as f:
            f.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
