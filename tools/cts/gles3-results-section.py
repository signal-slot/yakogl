#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the AM62P ES 3.0 whole-tree block of RESULTS.md from the merged TSV.

  tools/cts/gles3-results-section.py [--tsv docs/es3/am62p-gles3-notpass.tsv]
      [--tree tools/cts/am62p-gles3-tree.txt] [--mustpass FILE]
      [--commit SHA] [--date YYYY-MM-DD]

The block replaces the paragraph "**AM62P, whole tree**" (up to the next
"**" paragraph) of the section "## ES 3.0: dEQP-GLES3 (stage 4)"; the final
commit of plan 388 pastes it there, the same way gles3-status-ja.py's output
goes into STATUS.md. The TSV holds only non-Pass rows, so Pass = mustpass
line count minus the TSV rows. The shape is the one check-gles3-results.py
reads: the build commit in backticks, "<N> images" (tree line count), the
table Pass | Fail | NotSupported | QualityWarning | No result.

From the TSV's comment header:
  "# rows A-B: build SHA, run DATE"   one bullet per row range; the main
                                     commit (without --commit) is the build
                                     of the range with the latest run date;
  "# rerun SHA <what>"               one bullet per rerun commit.
The NoResult rows are grouped by cause (the family column, else the first
message line) into a bullet. Exit 0; 1 when a NoResult row has no cause or
the counts do not fit; 2 on an unreadable file or a missing build commit.
"""
import argparse
import collections
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUTCOMES = ['Fail', 'NotSupported', 'QualityWarning', 'NoResult']


def lines_of(text):
    return [l for l in text.splitlines() if l.strip() and not l.lstrip().startswith('#')]


def parse(text):
    counts = dict.fromkeys(OUTCOMES, 0)
    noresult = collections.Counter()
    header, ranges = [], []
    for l in text.splitlines():
        if not l.strip():
            continue
        if l.startswith('#'):
            h = l.lstrip('#').strip()
            header.append(h)
            m = re.match(r'rows (\d+)-(\d+): build `?([0-9a-f]{7,40})`?, run (\d{4}-\d{2}-\d{2})(?:/(\d{1,2}))?', h)
            if m:
                day = m.group(5) or m.group(4)[8:]
                ranges.append((m.group(1), m.group(2), m.group(3),
                               m.group(4)[:8] + '%02d' % int(day)))
            continue
        f = l.split('\t')
        if f[0] not in counts:
            raise ValueError('unknown outcome %r in the TSV' % f[0])
        counts[f[0]] += 1
        if f[0] == 'NoResult':
            cause = f[3].strip() if len(f) > 3 and f[3].strip() else (f[2].strip() if len(f) > 2 else '')
            noresult[cause or ''] += 1
    return counts, noresult, header, ranges


def num(n):
    return format(n, ',')


def build(tsv, tree, mustpass, commit=None, date=None):
    counts, noresult, header, ranges = parse(tsv)
    problems = []
    total = len(lines_of(mustpass))
    images = len(lines_of(tree))
    rest = sum(counts.values())
    if rest > total:
        raise ValueError('%d non-Pass rows exceed the %d mustpass cases' % (rest, total))
    latest = max(ranges, key=lambda r: r[3]) if ranges else None
    if commit is None:
        if latest:
            commit = latest[2]
        else:
            m = re.search(r'\bbuild\s+`?([0-9a-f]{7,40})\b', '\n'.join(header))
            if not m:
                raise ValueError('no build commit: pass --commit or name "build <sha>" in the TSV header')
            commit = m.group(1)
    if date is None and latest:
        date = latest[3]
    if '' in noresult:
        problems.append('%d NoResult rows have no cause' % noresult[''])
    on = '`%s`' % commit + (', %s' % date if date else '')
    row = [total - rest] + [counts[k] for k in OUTCOMES]
    out = ['**AM62P, whole tree** on %s, %d images:' % (on, images), '',
           '| Pass | Fail | NotSupported | QualityWarning | No result |',
           '|---:|---:|---:|---:|---:|',
           '| ' + ' | '.join(num(n) for n in row) + ' |', '',
           '- Every case that is not Pass: `docs/es3/am62p-gles3-notpass.tsv`.']
    if counts['NoResult']:
        causes = '; '.join('%s: %s' % (c or '?', num(n)) for c, n in
                           sorted(noresult.items(), key=lambda kv: (-kv[1], kv[0])))
        out.append('- The %s without a result, by cause: %s.' % (num(counts['NoResult']), causes))
    else:
        out.append('- No case is without a result.')
    for a, b, sha, d in ranges:
        out.append('- Rows %s-%s of the tree: build `%s`, run %s.' % (a, b, sha, d))
    for h in header:
        m = re.match(r'rerun\s+`?([0-9a-f]{7,40})`?:?\s*(.*)$', h)
        if m:
            out.append('- Rerun on `%s`: %s' % (m.group(1), m.group(2) or 'rows of the TSV'))
    return '\n'.join(out) + '\n', problems


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--tsv', default=os.path.join(ROOT, 'docs/es3/am62p-gles3-notpass.tsv'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'am62p-gles3-tree.txt'))
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
        print('gles3-results-section: %s' % e, file=sys.stderr)
        return 2
    try:
        text, problems = build(tsv, tree, mp, a.commit, a.date)
    except ValueError as e:
        print('gles3-results-section: %s' % e, file=sys.stderr)
        return 2
    sys.stdout.write(text)
    for p in problems:
        print('gles3-results-section: ' + p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
