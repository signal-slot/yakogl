#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check a dEQP table file (one CONFIG_DEQP_CASE_TABLE entry per line) against a mustpass list.

  check-imx8mp-gles3-tree.py [--table tools/cts/imx8mp-gles3-tree.txt] [--mustpass tools/cts/gles3-mustpass.txt]

Every mustpass case must match exactly one entry (dEQP's filter: comma-separated patterns, '*' matches any
run of characters, the match is on the whole case name); an entry is at most 2035 characters (arg[2048] of
driver/samples/zephyr/deqp/src/main.cpp) and there are at most 256 entries (the selector is 8 bits).
Prints one line per entry (index, length, mustpass cases) and the verdict; exit 0 only if all hold.
"""
import argparse, os, re, sys

MAX_LEN, MAX_ENTRIES = 2035, 256

def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser()
    ap.add_argument('--table', default=os.path.join(here, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(here, 'gles3-mustpass.txt'))
    ap.add_argument('-q', '--quiet', action='store_true', help='no per-entry lines')
    a = ap.parse_args()
    cases = [l.strip() for l in open(a.mustpass) if l.strip() and not l.startswith('#')]
    entries = [l.rstrip('\n') for l in open(a.table) if l.strip() and not l.lstrip().startswith('#')]
    bad = []
    pats = []  # (entry index, regex)
    for i, e in enumerate(entries):
        if len(e) > MAX_LEN:
            bad.append('entry %d is %d characters (max %d)' % (i, len(e), MAX_LEN))
        for p in e.split(','):
            if not p:
                bad.append('entry %d has an empty pattern' % i)
                continue
            pats.append((i, re.compile('^' + '.*'.join(re.escape(x) for x in p.split('*')) + '$')))
    if len(entries) > MAX_ENTRIES:
        bad.append('%d entries (max %d)' % (len(entries), MAX_ENTRIES))
    per = [0] * len(entries)
    unmatched, multi = [], []
    for c in cases:
        hit = {i for i, r in pats if r.match(c)}
        if not hit:
            unmatched.append(c)
        elif len(hit) > 1:
            multi.append((c, sorted(hit)))
        else:
            per[hit.pop()] += 1
    if not a.quiet:
        for i, e in enumerate(entries):
            print('entry %3d  %4d chars  %5d cases  %s' % (i, len(e), per[i], e.split(',')[0]))
    for c in unmatched[:10]:
        bad.append('not matched: ' + c)
    for c, h in multi[:10]:
        bad.append('matched by entries %s: %s' % (h, c))
    if unmatched: bad.append('%d mustpass cases matched by no entry' % len(unmatched))
    if multi: bad.append('%d mustpass cases matched by more than one entry' % len(multi))
    print('mustpass %d, entries %d (max %d), longest entry %d chars (max %d), largest entry %d cases, matched exactly once %d'
          % (len(cases), len(entries), MAX_ENTRIES, max(map(len, entries), default=0), MAX_LEN,
             max(per, default=0), sum(per)))
    for b in bad:
        print('FAIL:', b)
    print('OK' if not bad else 'FAILED')
    return 1 if bad else 0

if __name__ == '__main__':
    sys.exit(main())
