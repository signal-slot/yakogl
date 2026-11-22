#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the one-commit Pi 4 dEQP-GLES3 session record.

Usage:
  tools/cts/check-pi4-gles3-log.py --commit <full sha> \
      [--tree tools/cts/pi4-gles3-tree.txt] [session-dir ...]
  tools/cts/check-pi4-gles3-log.py --self-test

The session directories (default: the glob
tools/cts/log/*-pi4-gles3-<sha8>-*) are what the board sessions write,
tools/cts/log/<YYYY-MM>-pi4-gles3-<sha8>-rowsAA-BB/. Exit 1 when:
  a) an image label of the tree ('# image <LABEL>:' lines) has no
     <LABEL>.tally.txt in any directory (missing);
  b) a label has tallies in more than one directory (duplicated);
  c) a README.md has no line starting 'commit: <sha>' (or 'commit <sha>'),
     or names another commit;
  d) a tally has no complete deqp summary line (complete=1 and
     'summary: N cases begun, N with a result, 0 without'): aborted;
  e) a file holds an absolute home path (/home/, /Users/), an IPv4
     address, a .ts.net name or a /dev/tty path.
On success prints '<N> images, <N> with a tally, 0 missing, 0 duplicated,
commit <sha8>'.
"""
import argparse
import contextlib
import glob
import io
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
# gles3-notpass.py --pi4 wants 'commit: <sha>'; 'commit <sha>' is accepted too.
# Only line starts count, so prose such as 'fixed in commit <sha>' does not.
COMMIT_LINE = re.compile(r"^commit:?[ \t]+([0-9a-f]{7,40})\b", re.M | re.I)
REDACT = re.compile(
    r"/home/|/Users/|(?<![\w.-])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])|\.ts\.net|/dev/tty")
SUMMARY = re.compile(
    r"deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")


def labels_of(tree, counts=None):
    """Image labels of the tree; counts (a dict), if given, gets the case
    count of each label from its '# image <LABEL>: <N>' header."""
    out = []
    with open(tree, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*image\s+([^\s:]+)\s*:\s*(\d+)?", line)
            if m and m.group(1) not in out:
                out.append(m.group(1))
                if counts is not None and m.group(2):
                    counts[m.group(1)] = int(m.group(2))
    return out


def complete(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    if "complete=1" not in text:
        return False
    m = SUMMARY.search(text)
    return bool(m) and m.group(3) == "0" and m.group(1) == m.group(2)


def refused_one_case(path):
    """An image that ended with its own end line (a summary of 1 case
    begun, 1 with a result, 0 without) but complete=0: a one-case image
    (the tree header says 1 case) whose only case refused. No case of it is left without a result."""
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    m = SUMMARY.search(text)
    return bool(m) and (m.group(1), m.group(2), m.group(3)) == ("1", "1", "0")


def check(sha, tree, dirs):
    """Check the record; print the verdict, return the exit status."""
    dirs = [d for d in dirs if os.path.isdir(d)]
    problems = []
    if not dirs:
        problems.append("no session directory")

    counts = {}
    labels = labels_of(tree, counts)
    if not labels:
        problems.append("%s: no '# image <LABEL>:' lines" % tree)
    where = {}
    for d in dirs:
        for p in sorted(glob.glob(os.path.join(d, "*.tally.txt"))):
            lab = os.path.basename(p)[: -len(".tally.txt")]
            where.setdefault(lab, []).append(p)

        readme = os.path.join(d, "README.md")
        if not os.path.isfile(readme):
            problems.append("%s: no README.md (no commit line)" % d)
        else:
            with open(readme, encoding="utf-8", errors="replace") as f:
                shas = COMMIT_LINE.findall(f.read())
            if not shas:
                problems.append("%s: README.md has no 'commit: <sha>' line" % readme)
            for s in shas:
                s = s.lower()
                if not (sha.startswith(s) or s.startswith(sha)):
                    problems.append("%s: README.md names commit %s, not %s"
                                    % (readme, s, sha[:8]))

        for root, _, files in os.walk(d):
            for fn in sorted(files):
                p = os.path.join(root, fn)
                with open(p, "rb") as f:
                    data = f.read().decode("utf-8", errors="replace")
                for n, line in enumerate(data.splitlines(), 1):
                    m = REDACT.search(line)
                    if m:
                        problems.append("%s:%d: private detail %r" % (p, n, m.group(0)))
                        break

    missing, aborted, dup, info = [], [], [], []
    for lab in labels:
        ps = where.get(lab, [])
        if not ps:
            missing.append(lab)
            continue
        if len(ps) > 1:
            dup.append(lab)
            problems.append("%s: tallies in %d directories: %s" % (lab, len(ps), ", ".join(ps)))
        if any(complete(p) for p in ps):
            continue
        if counts.get(lab) == 1 and any(refused_one_case(p) for p in ps):
            info.append("%s: refused (one-case image)" % lab)
            continue
        aborted.append(lab)
    if missing:
        problems.append("missing (%d): %s" % (len(missing), " ".join(missing)))
    if aborted:
        problems.append("aborted, no complete deqp summary (%d): %s"
                        % (len(aborted), " ".join(aborted)))
    extra = []
    for lab in sorted(set(where) - set(labels)):
        subs = [l for l in labels if l.startswith(lab) and l != lab]
        if subs:
            info.append("superseded by re-split: %s -> %s" % (lab, " ".join(subs)))
        else:
            extra.append(lab)
    for line in info:
        print("info:", line)
    if extra:
        problems.append("tallies not in the tree: %s" % " ".join(extra))

    have = len(labels) - len(missing)
    if problems:
        for p in problems:
            print("FAIL:", p)
        print("%d images, %d with a tally, %d missing, %d duplicated, %d aborted, commit %s"
              % (len(labels), have, len(missing), len(dup), len(aborted), sha[:8]))
        return 1
    print("%d images, %d with a tally, 0 missing, 0 duplicated, commit %s"
          % (len(labels), have, sha[:8]))
    return 0


def self_test():
    """A synthetic record: a complete one passes, a missing tally and a
    mixed commit fail."""
    good = "a" * 40
    other = "b" * 40
    tally = ("deqp: complete=1\n"
             "deqp: summary: 3 cases begun, 3 with a result, 0 without\n")
    fails = []
    cases = []

    def run(tree, dirs):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(good, tree, dirs)
        return rc, buf.getvalue()

    def expect(name, ok):
        cases.append(name)
        print("%s %s" % ("ok  " if ok else "FAIL", name))
        if not ok:
            fails.append(name)

    with tempfile.TemporaryDirectory() as td:
        tree = os.path.join(td, "tree.txt")
        with open(tree, "w", encoding="utf-8") as f:
            f.write("# image A1: dEQP-GLES3.a\n# image B2: dEQP-GLES3.b\n"
                    "# image C3: dEQP-GLES3.c\n")

        def session(name, commit, labels, readme=None, texts=None):
            d = os.path.join(td, name)
            os.makedirs(d)
            with open(os.path.join(d, "README.md"), "w", encoding="utf-8") as f:
                f.write(readme if readme is not None else "commit: %s\n" % commit)
            for lab in labels:
                with open(os.path.join(d, lab + ".tally.txt"), "w",
                          encoding="utf-8") as f:
                    f.write((texts or {}).get(lab, tally))
            return d

        d1 = session("s1-rows1-2", good, ["A1", "B2"])
        d2 = session("s1-rows3-3", good, ["C3"])
        rc, out = run(tree, [d1, d2])
        expect("complete record passes",
               rc == 0 and "3 images, 3 with a tally" in out)

        os.remove(os.path.join(d2, "C3.tally.txt"))
        rc, out = run(tree, [d1, d2])
        expect("missing tally fails and names the label",
               rc == 1 and "missing (1): C3" in out)

        d3 = session("s2-rows3-3", other, ["C3"])
        rc, out = run(tree, [d1, d3])
        expect("session with another commit fails",
               rc == 1 and "names commit " + other in out)

        # re-split / refused rules, on their own tree
        tree2 = os.path.join(td, "tree2.txt")
        with open(tree2, "w", encoding="utf-8") as f:
            f.write("# image S1a: 3\n# image S1b: 2\n# image R1: 1\n"
                    "# image M1: 7\n")
        refused = ("deqp: complete=0\n"
                   "deqp: summary: 1 cases begun, 1 with a result, 0 without\n")
        multi = ("deqp: complete=0\n"
                 "deqp: summary: 5 cases begun, 5 with a result, 0 without\n")

        def run2(labels, texts=None, tag="x"):
            d = session("t2-" + tag, good, labels, None, texts)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = check(good, tree2, [d])
            return rc, buf.getvalue()

        base = ["S1a", "S1b", "R1", "M1"]
        rc, out = run2(base + ["S1"], {"R1": refused}, "a")
        expect("superseded tally ok, one-case refused ok",
               rc == 0 and "superseded by re-split: S1 -> S1a S1b" in out
               and "R1: refused (one-case image)" in out)
        rc, out = run2(base + ["Zzz"], {"R1": refused}, "b")
        expect("unrelated extra tally still fails",
               rc == 1 and "tallies not in the tree: Zzz" in out)
        rc, out = run2(base, {"M1": multi, "R1": refused}, "c")
        expect("multi-case complete=0 still aborted",
               rc == 1 and "aborted, no complete deqp summary (1): M1" in out)
        rc, out = run2(base, {"R1": multi.replace("complete=0", "complete=0")}, "d")
        expect("multi-case image without complete summary fails",
               rc == 1 and "R1" in out)

        rc, out = run2(base, {"M1": refused}, "e")
        expect("one case begun in a multi-case image is aborted, not refused",
               rc == 1 and "aborted, no complete deqp summary (1): M1" in out)

        def readme_case(name, tag, text, want_ok):
            d = session("r-" + tag, good, ["C3"], text)
            rc, out = run(tree, [d1, d])
            expect(name, (rc == 0) == want_ok)

        readme_case("'commit: <sha>' line passes", "colon",
                    "commit: %s\n" % good, True)
        readme_case("'commit <sha>' line passes", "plain",
                    "commit %s\n" % good, True)
        readme_case("prose 'see commit <sha>' only fails", "prose",
                    "see commit %s\n" % good, False)
        readme_case("prose sha of another commit beside a commit: line passes",
                    "both", "fixed in commit %s\ncommit: %s\n" % (other, good),
                    True)

    print("check-pi4-gles3-log self-test: %d of %d cases ok"
          % (len(cases) - len(fails), len(cases)))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--commit", help="full sha of the record")
    ap.add_argument("--self-test", action="store_true",
                    help="check the checker on a synthetic record")
    ap.add_argument("dirs", nargs="*", help="session directories")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.commit:
        ap.error("--commit is required")
    sha = a.commit.lower()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        ap.error("--commit needs a full 40-digit sha")
    dirs = a.dirs or sorted(
        glob.glob(os.path.join(HERE, "log", "*-pi4-gles3-%s-*" % sha[:8])))
    return check(sha, a.tree, dirs)


if __name__ == "__main__":
    sys.exit(main())
