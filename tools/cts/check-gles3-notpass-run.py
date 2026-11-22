#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check that the AM62P dEQP-GLES3 not-pass TSV is one whole-tree run.

  check-gles3-notpass-run.py [--ref REF] [--repo DIR] NOTPASS.tsv TREE.txt

c1  The header names the build per row range ('# rows AA-BB: build X, run Y',
    or the one-line 'build X, run Y:' form, which covers every row), the ranges
    cover every row of the tree table (read as gles3-notpass.py --splice
    reads it, rows counted from 0) once, all slices come from one build (a
    range of another build is allowed only for single rows: a row rerun later
    is named per row with its own commit), and every build is an ancestor of
    REF (default origin/main: `git merge-base --is-ancestor <sha> REF`).
c2  No non-comment row has an empty outcome (it must be one of Fail,
    NotSupported, QualityWarning, NoResult) or an empty cause-family column.

The parsers are gles3-notpass.py's (header ranges) and check-gles3-tree.py's
(tree table), imported. One line per failed check on stdout. Exit 0 all
checks pass, 1 a check failed, 2 the TSV has no family column yet (the c1
lines are still printed) or an input could not be read.
"""
import argparse
import os
import subprocess
import sys
from importlib.util import module_from_spec, spec_from_file_location

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = spec_from_file_location(name, os.path.join(HERE, file))
    mod = module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


NP = load("gles3_notpass", "gles3-notpass.py")
TREE = load("check_gles3_tree", "check-gles3-tree.py")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo] + list(args), capture_output=True, text=True)


def read_tsv(text):
    """(header lines, [(line number, columns)]) of the TSV."""
    head, rows = [], []
    for k, line in enumerate(text.splitlines(), 1):
        if line.startswith("#"):
            head.append(line)
        elif line.strip():
            rows.append((k, line.split("\t")))
    return head, rows


def check_c1(head, nrows, ref, repo):
    out = []
    ranges = NP.old_ranges(head, nrows)
    if not ranges:
        return ["c1: the header names no build (no '# rows AA-BB: build X, run Y' line)"]
    covered = {}
    for lo, hi, c, d in ranges:
        for r in range(lo, hi + 1):
            if r in covered:
                out.append("c1: row %d is in two header ranges (builds %s and %s)" % (r, covered[r], c))
            covered[r] = c
    miss = [r for r in range(nrows) if r not in covered]
    if miss:
        out.append("c1: the header names no build for %d of the tree's %d rows, first: row %d"
                   % (len(miss), nrows, miss[0]))
    over = sorted(r for r in covered if r >= nrows)
    if over:
        out.append("c1: the header names row %d, but the tree has rows 0-%d" % (over[0], nrows - 1))
    per_build = {}
    for c in covered.values():
        per_build[c] = per_build.get(c, 0) + 1
    base = max(per_build, key=lambda c: (per_build[c], c))
    for lo, hi, c, d in ranges:
        if c != base and lo != hi:
            out.append("c1: rows %d-%d come from build %s, not from %s (the build of %d rows); "
                       "only a single row may be rerun on its own commit"
                       % (lo, hi, c, base, per_build[base]))
    if git(repo, "rev-parse", "--verify", "--quiet", ref + "^{commit}").returncode != 0:
        return out + ["c1: %s is not a commit of %s" % (ref, repo)]
    for c in sorted(per_build):
        full = git(repo, "rev-parse", "--verify", "--quiet", c + "^{commit}")
        if full.returncode != 0:
            out.append("c1: build %s is not a commit of the repository" % c)
        elif git(repo, "merge-base", "--is-ancestor", full.stdout.strip(), ref).returncode != 0:
            out.append("c1: build %s is not an ancestor of %s" % (c, ref))
    return out


def check_c2(rows):
    out = []
    for k, p in rows:
        outcome = p[0].strip()
        if outcome not in NP.COUNTED[1:]:
            out.append("c2: line %d: %s outcome %r" % (k, "empty" if not outcome else "bad", p[0]))
        if len(p) < 4 or not p[3].strip():
            out.append("c2: line %d: empty cause family: %s" % (k, p[1] if len(p) > 1 else "?"))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("tsv")
    ap.add_argument("tree")
    ap.add_argument("--ref", default="origin/main")
    ap.add_argument("--repo", default=os.path.join(HERE, "..", ".."),
                    help="the git repository (default: this one)")
    a = ap.parse_args()
    try:
        with open(a.tsv, encoding="utf-8") as f:
            text = f.read()
        with open(a.tree, encoding="utf-8") as f:
            nrows = len(TREE.read_table(f.read()))
    except OSError as e:
        sys.stderr.write("check-gles3-notpass-run: %s\n" % e)
        return 2
    head, rows = read_tsv(text)
    problems = check_c1(head, nrows, a.ref, a.repo)
    if not any(len(p) >= 4 for _, p in rows):
        for p in problems:
            print(p)
        print("c2: %s has no cause-family column yet (run gles3-triage.py to add it)" % a.tsv)
        return 2
    problems += check_c2(rows)
    for p in problems:
        print(p)
    if not problems:
        print("ok: %d rows, one build per row range of the tree's %d rows, an ancestor of %s, "
              "every row has an outcome and a family" % (len(rows), nrows, a.ref))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
