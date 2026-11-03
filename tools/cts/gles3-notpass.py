#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write a board's dEQP-GLES3 not-pass TSV (docs/es3/am62p-gles3-notpass.tsv).

  gles3-notpass.py --commit <sha> [--board NAME] [--date D] [--mustpass FILE]
                   [-o OUT] RESULTS...
  gles3-notpass.py --self-test

RESULTS are, in order (a later source wins for a case it repeats):
  a directory   every *.qpa under it, sorted by path
  *.qpa         a dEQP log: one TestCaseResult per case
  anything else per-case statuses, one per line, tab separated:
                <case> <status> [<message>]  (Pass lines allowed; '#' comments)
                e.g. the merged statuses of the tree's slices.

Output: the SPDX header, a header naming the build, the board and the run
date, then, for every case of the mustpass list (tools/cts/gles3-mustpass.txt)
whose status is not Pass, in mustpass order: <outcome> <case> <first message
line>, tab separated. A mustpass case with no result is NoResult; a log entry
that was begun and cut off (no Result element) is Crash. Cases that are not in
the mustpass list are ignored (reported on stderr).
"""
import argparse
import datetime
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BOARD = 'TI AM62P (Verdin, PowerVR)'
XML = (('&apos;', "'"), ('&quot;', '"'), ('&lt;', '<'), ('&gt;', '>'), ('&amp;', '&'))
BLOCK_RE = re.compile(r'<TestCaseResult\s[^>]*?CasePath="([^"]+)"')
RESULT_RE = re.compile(r'<Result\s+StatusCode="([^"]*)"\s*(?:/>|>(.*?)</Result>)', re.S)


def unescape(s):
    for a, b in XML:
        s = s.replace(a, b)
    return s


def first_line(msg):
    for line in unescape(msg or '').splitlines():
        line = line.strip()
        if line:
            return line
    return ''


def parse_qpa(text, out):
    # split() with one group: [pre, case, body, case, body, ...]. A body ends
    # where the next TestCaseResult starts, so a cut-off case cannot borrow
    # the next case's Result.
    parts = BLOCK_RE.split(text)
    for case, body in zip(parts[1::2], parts[2::2]):
        m = RESULT_RE.search(body)
        if m:
            out[case] = (m.group(1), first_line(m.group(2)))
        else:
            out[case] = ('Crash', 'no result recorded (case cut off)')


def parse_tsv(text, out):
    for line in text.splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        f = line.split('\t')
        if len(f) < 2:
            raise SystemExit('gles3-notpass: bad status line (want case<TAB>status): %r' % line)
        out[f[0]] = (f[1], first_line(f[2]) if len(f) > 2 else '')


def load(paths):
    out = {}
    for p in paths:
        if os.path.isdir(p):
            files = sorted(os.path.join(d, n) for d, _, ns in os.walk(p)
                           for n in ns if n.endswith('.qpa'))
        else:
            files = [p]
        for f in files:
            with open(f, encoding='utf-8', errors='replace') as fh:
                text = fh.read()
            (parse_qpa if f.endswith('.qpa') else parse_tsv)(text, out)
    return out


def render(results, mustpass, commit, board, date):
    lines = [
        '# SPDX-License-Identifier: Apache-2.0',
        '# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.',
        '# dEQP-GLES3 on the %s, build %s, run %s: every mustpass case' % (board, commit, date),
        '# (tools/cts/gles3-mustpass.txt) whose result is not Pass. One case per line, tab separated:',
        '# <outcome> <case> <first message line>. NoResult: the case never ran',
        '# (the run\'s "Not run" list).',
    ]
    for case in mustpass:
        r = results.get(case)
        if r is None:
            lines.append('NoResult\t%s\t' % case)
        elif r[0] != 'Pass':
            lines.append('%s\t%s\t%s' % (r[0], case, r[1]))
    return '\n'.join(lines) + '\n'


def read_mustpass(path):
    with open(path) as f:
        return [l.strip() for l in f if l.strip() and not l.startswith('#')]


def self_test():
    d = os.path.join(HERE, 'testdata', 'gles3-notpass')
    must = read_mustpass(os.path.join(d, 'mustpass.txt'))
    res = load([os.path.join(d, 'logs'), os.path.join(d, 'merged.tsv')])
    got = render(res, must, 'abc1234', BOARD, '2026-11-03')
    with open(os.path.join(d, 'expected.tsv')) as f:
        want = f.read()
    if got != want:
        sys.stderr.write('FAIL: output differs from testdata/gles3-notpass/expected.tsv\n' + got)
        return 1
    print('gles3-notpass.py self-test ok')
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('results', nargs='*')
    ap.add_argument('--commit')
    ap.add_argument('--board', default=BOARD)
    ap.add_argument('--date', default=datetime.date.today().isoformat())
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('-o', '--output')
    ap.add_argument('--self-test', action='store_true')
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.commit or not a.results:
        ap.error('--commit and at least one RESULTS path are required')
    must = read_mustpass(a.mustpass)
    res = load(a.results)
    known = set(must)
    extra = sorted(c for c in res if c not in known)
    if extra:
        sys.stderr.write('gles3-notpass: %d result(s) not in the mustpass list, ignored (first: %s)\n'
                         % (len(extra), extra[0]))
    text = render(res, must, a.commit, a.board, a.date)
    if a.output:
        with open(a.output, 'w') as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
