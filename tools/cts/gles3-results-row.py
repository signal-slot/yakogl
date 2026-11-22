#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the AM62P ES 3.0 whole-tree rows of RESULTS.md from the merged TSV.

  tools/cts/gles3-results-row.py TSV TREE [--mustpass FILE] [--commit SHA]
      [--date YYYY-MM-DD]

TSV is docs/es3/am62p-gles3-notpass.tsv (non-Pass rows only, see
gles3-notpass.py), TREE tools/cts/am62p-gles3-tree.txt. Pass is the mustpass
line count minus the TSV's rows. Prints the paragraph "**AM62P, whole tree**"
and its table in the shape check-gles3-results.py accepts.

The build commit is --commit, else the word after "build" in the TSV's
comment header; the date is --date, else the date of "run YYYY-MM-DD[/DD]" in
that header (a range gives its last day), else the last date named there.
Per-row rerun commits are comment lines of the header:
"# rerun <sha> <what was rerun>" (one bullet each, in order).
Exit 0; 2 on a file that cannot be read, a missing commit or counts that do
not fit (more non-Pass rows than mustpass cases, an unknown outcome).
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUTCOMES = ['Fail', 'NotSupported', 'QualityWarning', 'NoResult']


def tree_images(text):
    return sum(1 for l in text.splitlines() if not re.match(r'\s*(#|$)', l))


def parse_tsv(text):
    counts = dict.fromkeys(OUTCOMES, 0)
    header = []
    for l in text.splitlines():
        if not l.strip():
            continue
        if l.startswith('#'):
            header.append(l.lstrip('#').strip())
            continue
        k = l.split('\t', 1)[0]
        if k not in counts:
            raise ValueError('unknown outcome %r in the TSV' % k)
        counts[k] += 1
    return counts, header


def num(n):
    return format(n, ',')


def render(counts, header, images, total, commit=None, date=None):
    rest = sum(counts.values())
    if rest > total:
        raise ValueError('%d non-Pass rows exceed the %d mustpass cases' % (rest, total))
    htext = '\n'.join(header)
    if commit is None:
        m = re.search(r'\bbuild\s+`?([0-9a-f]{7,40})\b', htext)
        if not m:
            raise ValueError('no build commit: pass --commit or name "build <sha>" in the TSV header')
        commit = m.group(1)
    if date is None:
        m = re.search(r'run\s+(\d{4})-(\d{2})-\d{2}/(\d{1,2})', htext)
        m2 = re.search(r'run\s+(\d{4}-\d{2}-\d{2})', htext)
        ds = re.findall(r'\d{4}-\d{2}-\d{2}', htext)
        if m:
            date = '%s-%s-%02d' % (m.group(1), m.group(2), int(m.group(3)))
        elif m2:
            date = m2.group(1)
        elif ds:
            date = ds[-1]
    on = '`%s`' % commit + (', %s' % date if date else '')
    row = [total - rest] + [counts[k] for k in OUTCOMES]
    out = ['**AM62P, whole tree** on %s, %d images:' % (on, images), '',
           '| Pass | Fail | NotSupported | QualityWarning | No result |',
           '|---:|---:|---:|---:|---:|',
           '| ' + ' | '.join(num(n) for n in row) + ' |', '',
           '- Every case that is not Pass: `docs/es3/am62p-gles3-notpass.tsv`.']
    for h in header:
        m = re.match(r'rerun\s+`?([0-9a-f]{7,40})`?:?\s*(.*)$', h)
        if m:
            out.append('- Rerun on `%s`: %s' % (m.group(1), m.group(2) or 'rows of the TSV'))
    return '\n'.join(out) + '\n'


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('tsv')
    ap.add_argument('tree')
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--commit')
    ap.add_argument('--date')
    a = ap.parse_args()
    try:
        def rd(f):
            with open(f, encoding='utf-8') as fh:
                return fh.read()
        tsv, tree, mp = rd(a.tsv), rd(a.tree), rd(a.mustpass)
    except OSError as e:
        print('gles3-results-row: %s' % e, file=sys.stderr)
        return 2
    try:
        counts, header = parse_tsv(tsv)
        total = len([l for l in mp.splitlines() if l.strip()])
        sys.stdout.write(render(counts, header, tree_images(tree), total, a.commit, a.date))
    except ValueError as e:
        print('gles3-results-row: %s' % e, file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
