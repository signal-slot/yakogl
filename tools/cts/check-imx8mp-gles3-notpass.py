#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Shape check of an i.MX8MP dEQP-GLES3 not-pass TSV (docs/es3/imx8mp-gles3-notpass.tsv).

  check-imx8mp-gles3-notpass.py [--mustpass tools/cts/gles3-mustpass.txt] <tsv>

The shape is that of docs/es3/pi4-gles3-fails.tsv: `# ` comment lines first (at least one, the first
naming the dEQP-GLES3 commit, a `# rows:` line describing the row form; further comments such as
`# measured on the Armadillo-X2 ...` or `# e<i>: ...` notes are allowed anywhere), then one row per case
`<record>:<Status> <case>` (one space, no other column). Checked, one line per problem on stderr:
  - the header: first line a comment, a `# rows:` comment present, no data row before it;
  - every row has exactly the two columns, a record token without blanks, no tab;
  - the status is one of NotSupported, QualityWarning, CompatibilityWarning, ResourceError, InternalError,
    Crash, Timeout, Fail, NoResult; `Pass` is never listed;
  - the case is in the mustpass list;
  - sorted: record `e<i>` (imx8mp-gles3-merge.py --notpass): by entry, then mustpass order; any other
    record (the pi4 file): by case name;
  - no case twice.
Exit 0 when clean, 1 with problems, 2 on a usage or read error.
"""
import os
import re
import sys

STATUSES = ('NotSupported', 'QualityWarning', 'CompatibilityWarning', 'ResourceError', 'InternalError',
            'Crash', 'Timeout', 'Fail', 'NoResult')
ROW = re.compile(r'^([^\s:]+):(\S+) (\S+)$')


def check(path, mustpass):
    probs = []
    order = {}
    for l in open(mustpass):
        l = l.strip()
        if l and not l.startswith('#'):
            order.setdefault(l, len(order))
    lines = open(path).read().split('\n')
    if lines and lines[-1] == '':
        lines.pop()
    if not lines or not lines[0].startswith('# '):
        probs.append('line 1: the first line must be a "# " comment')
    seen = {}
    prev = None
    rows_hdr = False
    n = 0
    for no, l in enumerate(lines, 1):
        if l.startswith('#'):
            if not l.startswith('# '):
                probs.append('line %d: comment must start with "# "' % no)
            if l.startswith('# rows:'):
                rows_hdr = True
            continue
        if not l.strip():
            probs.append('line %d: blank line' % no)
            continue
        n += 1
        if not rows_hdr:
            probs.append('line %d: data row before the "# rows:" header line' % no)
            rows_hdr = True
        m = ROW.match(l)
        if not m:
            probs.append('line %d: not "<record>:<Status> <case>" with two columns: %r' % (no, l[:80]))
            continue
        rec, st, case = m.groups()
        if st == 'Pass':
            probs.append('line %d: Pass row %s' % (no, case))
        elif st not in STATUSES:
            probs.append('line %d: unknown status %r' % (no, st))
        if case not in order:
            probs.append('line %d: case not in the mustpass list: %s' % (no, case))
        if case in seen:
            probs.append('line %d: duplicate of line %d: %s' % (no, seen[case], case))
        else:
            seen[case] = no
        em = re.fullmatch(r'e(\d+)', rec)
        key = ((int(em.group(1)), order.get(case, -1)) if em else (0, case))
        if prev is not None and type(prev[0][1]) is type(key[1]) and key < prev[0]:
            probs.append('line %d: out of order (after line %d): %s' % (no, prev[1], case))
        prev = (key, no)
    if not rows_hdr:
        probs.append('no "# rows:" header line')
    return probs, n


def main(argv):
    mp = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'gles3-mustpass.txt')
    args = argv[1:]
    if args[:1] == ['--mustpass'] and len(args) >= 2:
        mp, args = args[1], args[2:]
    if len(args) != 1:
        sys.stderr.write(__doc__.split('\n\n')[0] + '\n')
        return 2
    try:
        probs, n = check(args[0], mp)
    except OSError as e:
        sys.stderr.write('%s\n' % e)
        return 2
    for p in probs:
        sys.stderr.write(p + '\n')
    if probs:
        sys.stderr.write('%d problem(s) in %d row(s)\n' % (len(probs), n))
        return 1
    print('ok: %d row(s)' % n)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
