#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the one-commit Pi 4 dEQP-GLES3 session record.

Usage:
  tools/cts/check-pi4-gles3-log.py --commit <full sha> \
      [--tree tools/cts/pi4-gles3-tree.txt] [session-dir ...]

The session directories (default: the glob
tools/cts/log/*-pi4-gles3-<sha8>-*) are what the board sessions write,
tools/cts/log/<YYYY-MM>-pi4-gles3-<sha8>-rowsAA-BB/. Exit 1 when:
  a) an image label of the tree ('# image <LABEL>:' lines) has no
     <LABEL>.tally.txt in any directory (missing);
  b) a label has tallies in more than one directory (duplicated);
  c) a README.md has no 'commit <sha>' line, or names another commit;
  d) a tally has no complete deqp summary line (complete=1 and
     'summary: N cases begun, N with a result, 0 without'): aborted;
  e) a file holds an absolute home path (/home/, /Users/), an IPv4
     address, a .ts.net name or a /dev/tty path.
On success prints '<N> images, <N> with a tally, 0 missing, 0 duplicated,
commit <sha8>'.
"""
import argparse
import glob
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SECRET = re.compile(
    r"/home/|/Users/|(?<![\w.-])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])|\.ts\.net|/dev/tty")
SUMMARY = re.compile(
    r"deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")


def labels_of(tree):
    out = []
    with open(tree, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*image\s+([^\s:]+)\s*:", line)
            if m and m.group(1) not in out:
                out.append(m.group(1))
    return out


def complete(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    if "complete=1" not in text:
        return False
    m = SUMMARY.search(text)
    return bool(m) and m.group(3) == "0" and m.group(1) == m.group(2)


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--commit", required=True, help="full sha of the record")
    ap.add_argument("dirs", nargs="*", help="session directories")
    a = ap.parse_args()
    sha = a.commit.lower()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        ap.error("--commit needs a full 40-digit sha")
    dirs = a.dirs or sorted(
        glob.glob(os.path.join(HERE, "log", "*-pi4-gles3-%s-*" % sha[:8])))
    dirs = [d for d in dirs if os.path.isdir(d)]
    problems = []
    if not dirs:
        problems.append("no session directory")

    labels = labels_of(a.tree)
    if not labels:
        problems.append("%s: no '# image <LABEL>:' lines" % a.tree)
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
                shas = re.findall(r"\bcommit\s+([0-9a-f]{7,40})\b", f.read(), re.I)
            if not shas:
                problems.append("%s: README.md has no 'commit <sha>' line" % readme)
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
                    m = SECRET.search(line)
                    if m:
                        problems.append("%s:%d: private detail %r" % (p, n, m.group(0)))
                        break

    missing, aborted, dup = [], [], []
    for lab in labels:
        ps = where.get(lab, [])
        if not ps:
            missing.append(lab)
            continue
        if len(ps) > 1:
            dup.append(lab)
            problems.append("%s: tallies in %d directories: %s" % (lab, len(ps), ", ".join(ps)))
        if not any(complete(p) for p in ps):
            aborted.append(lab)
    if missing:
        problems.append("missing (%d): %s" % (len(missing), " ".join(missing)))
    if aborted:
        problems.append("aborted, no complete deqp summary (%d): %s"
                        % (len(aborted), " ".join(aborted)))
    extra = sorted(set(where) - set(labels))
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


if __name__ == "__main__":
    sys.exit(main())
