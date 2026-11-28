#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Shape check of the i.MX8MP dEQP-GLES3 not-pass TSV (docs/es3/imx8mp-gles3-notpass.tsv).

  check-imx8mp-gles3-notpass.py [--mustpass M] <tsv>

The file is the output of `imx8mp-gles3-merge.py --notpass`, in the shape of docs/es3/pi4-gles3-fails.tsv:
leading `# ` comment lines (the first names the board and commit, a `# rows:` line gives the row shape),
then one `<record>:<status> <case>` row per case that is not Pass. Here <record> is `e<i>`, the entry of
tools/cts/imx8mp-gles3-tree.txt. Checked: the header comments, one `<record>:<status> <case>` field pair per
row, the status is a known non-Pass value, the case is in the mustpass list, no case twice, no Pass row,
and the order: entry index ascending, mustpass order within an entry (merge.py's order; the pi4 file is
sorted by case name instead). Exit 1 with one line per problem, 0 when clean.
"""
import argparse, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
STATUSES = {'NotSupported', 'QualityWarning', 'CompatibilityWarning', 'ResourceError', 'InternalError',
            'Crash', 'Timeout', 'Fail', 'NoResult'}
ROW = re.compile(r'^e(\d+):(\S+) (\S+)$')


def check(path, mustpass):
    probs = []
    pos = {}
    for l in open(mustpass):
        l = l.strip()
        if l and not l.startswith('#'):
            pos.setdefault(l, len(pos))
    lines = open(path).read().split('\n')
    if lines and lines[-1] == '':
        lines.pop()
    else:
        probs.append('%s: no final newline' % path)
    head = [l for l in lines if l.startswith('#')]
    if not lines or not lines[0].startswith('# i.MX8MP dEQP-GLES3 at ') or not re.match(r'# i\.MX8MP dEQP-GLES3 at [0-9a-f]{8,40}\b', lines[0]):
        probs.append('line 1: header must be `# i.MX8MP dEQP-GLES3 at <commit>...`')
    if not any(l.startswith('# rows: e<i>:<status> <case>') for l in head):
        probs.append('no `# rows: e<i>:<status> <case>` comment line')
    seen = {}
    last = None
    in_rows = False
    for n, l in enumerate(lines, 1):
        if l.startswith('#'):
            if in_rows and not re.match(r'# e\d+: ', l):
                probs.append('line %d: comment after the first row' % n)
            continue
        in_rows = True
        m = ROW.match(l)
        if not m:
            probs.append('line %d: not `e<i>:<status> <case>`: %r' % (n, l))
            continue
        e, st, case = int(m.group(1)), m.group(2), m.group(3)
        if st == 'Pass':
            probs.append('line %d: Pass row: %s' % (n, case))
        elif st not in STATUSES:
            probs.append('line %d: unknown status %r' % (n, st))
        if case not in pos:
            probs.append('line %d: case not in %s: %s' % (n, os.path.basename(mustpass), case))
            continue
        if case in seen:
            probs.append('line %d: duplicate of line %d: %s' % (n, seen[case], case))
        seen[case] = n
        if last is not None:
            le, lp = last
            if e < le or (e == le and pos[case] < lp):
                probs.append('line %d: out of order (entry then mustpass order): %s' % (n, case))
        last = (e, pos[case])
    return probs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('tsv')
    a = ap.parse_args()
    probs = check(a.tsv, a.mustpass)
    for p in probs:
        print('%s: %s' % (a.tsv, p))
    return 1 if probs else 0


if __name__ == '__main__':
    sys.exit(main())
