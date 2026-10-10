#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Does a dEQP tree table select every mustpass case exactly once?

  tools/cts/check-gles3-tree.py [--mustpass FILE] [--max N] TABLE...
  tools/cts/check-gles3-tree.py --self-test

A table is one CONFIG_DEQP_CASE filter per line (comma-separated dEQP case
patterns, '*' matching anything), in the form of tools/cts/pi4-full-tree.txt
and am62p-gles3-tree.txt: a line "# image <LABEL>: <note>" labels the filter
line after it (the Pi 4's record names), other '#' lines and blank lines are
skipped, and a filter line without a label is image img<N> (N counting the
filter lines from 1), as am62p-deqp-images.sh and tools/board.sh name them.

For each table it prints one line per image, "<LABEL> <matched>" (the mustpass
cases the image's filter selects), then the verdict line

  check-gles3-tree: <table>: <R> images, <M> matched once, <X> not matched,
  <Y> matched more than once, <Z> empty images

and exits 0 only when X, Y and Z are 0 and nothing else is wrong:

  - a label used twice, or one that is not [A-Za-z0-9]+ (the builders'
    and board.sh's label rule);
  - a pattern that is empty, has a space, or does not start with the
    module's prefix ("dEQP-GLES3." for the default list);
  - a note that starts with a number which is not the image's matched count
    ("# image S01: 861" says what the row selects; a stale count misleads);
  - with --max N, an image selecting more than N mustpass cases (the size
    that fits a board's watchdog window is the table's own business; the
    option only pins what a table claims in its header).

The default list is tools/cts/gles3-mustpass.txt (42,810 cases at VK-GL-CTS
93bca01, tools/cts/gen-gles3-mustpass.sh); --mustpass names another, so the
same check reads a GLES2 table against gles2-mustpass.txt. The filters match
dEQP's whole tree, which is larger than the mustpass: a case outside the list
is neither counted nor reported, as the tally (deqp-uart-tally.py --mustpass)
leaves it out too.

Why: tools/cts/RESULTS.md section 1r's AM62P run had to split line 19 of its
table by hand into 19a-19g and the 3D filtering cases into named lists; a
split by hand drops or doubles a case easily, and a doubled case costs board
time while a dropped one is a silent hole in the total. check-deqp-images.py
runs this on the GLES3 tables as one of its repo rules.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LABEL_RE = re.compile(r'^# image ([^:]*):(.*)$')
LABEL_OK = re.compile(r'^[A-Za-z0-9]+$')


def read_table(text):
    """[(label, note, filter)] of a table's text; unlabelled rows are img<N>."""
    rows, lab, note, n = [], None, '', 0
    for l in text.splitlines():
        m = LABEL_RE.match(l)
        if m:
            lab, note = m.group(1), m.group(2).strip()
            continue
        if re.match(r'\s*(#|$)', l):
            continue
        n += 1
        rows.append((lab if lab is not None else 'img%d' % n, note, l.strip()))
        lab, note = None, ''
    return rows


def row_regex(filt):
    """One regex over a mustpass text (one name per line) for a filter's
    patterns: '*' matches anything but a newline, the rest literally."""
    alts = ['[^\n]*'.join(re.escape(part) for part in p.split('*')) for p in filt.split(',')]
    return re.compile('^(?:%s)$' % '|'.join(alts), re.M)


def check(text, mustpass, prefix='dEQP-GLES3.', max_cases=None):
    """(problems, [(label, matched)], counts) of a table's text against the
    mustpass names; counts = (once, never, more than once, empty rows)."""
    rows = read_table(text)
    bad, per_row = [], []
    index = {name: i for i, name in enumerate(mustpass)}
    hits = [0] * len(mustpass)
    blob = '\n'.join(mustpass)
    seen = set()
    for lab, note, filt in rows:
        if not LABEL_OK.match(lab):
            bad.append("label '%s' is not [A-Za-z0-9]+" % lab)
        if lab in seen:
            bad.append("label '%s' is used twice" % lab)
        seen.add(lab)
        for p in filt.split(','):
            if not p:
                bad.append('%s: an empty pattern (a stray comma?)' % lab)
            elif ' ' in p or '\t' in p:
                bad.append("%s: pattern '%s' has white space" % (lab, p))
            elif not p.startswith(prefix):
                bad.append("%s: pattern '%s' does not start with %s" % (lab, p, prefix))
        n = 0
        for m in row_regex(filt).finditer(blob):
            hits[index[m.group(0)]] += 1
            n += 1
        per_row.append((lab, n))
        mnote = re.match(r'(\d+)', note)
        if mnote and int(mnote.group(1)) != n:
            bad.append('%s: the note says %s, the filter selects %d mustpass cases' % (lab, mnote.group(1), n))
        if max_cases is not None and n > max_cases:
            bad.append('%s selects %d mustpass cases, more than %d' % (lab, n, max_cases))
    once = sum(1 for h in hits if h == 1)
    never = [mustpass[i] for i, h in enumerate(hits) if h == 0]
    twice = [mustpass[i] for i, h in enumerate(hits) if h > 1]
    empty = [lab for lab, n in per_row if n == 0]
    if never:
        bad.append('%d mustpass cases matched by no image, first: %s' % (len(never), never[0]))
    if twice:
        who = [lab for (lab, _, filt) in rows if row_regex(filt).match(twice[0])]
        bad.append('%d mustpass cases matched by more than one image, first: %s (images %s)'
                   % (len(twice), twice[0], ' '.join(who)))
    for lab in empty:
        bad.append('%s selects no mustpass case' % lab)
    return bad, per_row, (once, len(never), len(twice), len(empty))


def check_file(path, mustpass_path, max_cases=None):
    """Print the per-image lines and the verdict line; return the exit code."""
    mustpass = [l.strip() for l in open(mustpass_path) if l.strip() and not l.startswith('#')]
    prefix = mustpass[0].split('.')[0] + '.' if mustpass else 'dEQP-GLES3.'
    bad, per_row, (once, never, twice, empty) = check(open(path).read(), mustpass, prefix, max_cases)
    for lab, n in per_row:
        print('%s %d' % (lab, n))
    for b in bad:
        print('check-gles3-tree: FAIL %s: %s' % (os.path.basename(path), b), file=sys.stderr)
    print('check-gles3-tree: %s: %d images, %d matched once, %d not matched, %d matched more than once, '
          '%d empty images' % (os.path.relpath(path), len(per_row), once, never, twice, empty))
    return 1 if bad else 0


def self_test():
    fails = 0

    def ok(cond, what):
        nonlocal fails
        print(('  ok    ' if cond else '  FAIL  ') + what)
        if not cond:
            fails += 1
    mp = ['dEQP-GLES3.functional.a.x', 'dEQP-GLES3.functional.a.y', 'dEQP-GLES3.functional.b.z.1',
          'dEQP-GLES3.functional.b.z.2', 'dEQP-GLES3.functional.c.w', 'dEQP-GLES3.info.vendor']
    good = ('# a table\n# image AA: 2 of a\ndEQP-GLES3.functional.a.*\n\n'
            '# image BB: the rest\ndEQP-GLES3.functional.b.z.1,dEQP-GLES3.functional.b.z.2,'
            'dEQP-GLES3.functional.c.*,dEQP-GLES3.info.*\n')
    bad, rows, counts = check(good, mp)
    ok(bad == [] and rows == [('AA', 2), ('BB', 4)] and counts == (6, 0, 0, 0),
       'a table covering each case once passes: %s %s' % (rows, counts))
    bad, rows, counts = check(good.replace('functional.c.*', 'functional.c.*,dEQP-GLES3.functional.a.x'), mp)
    ok(counts[2] == 1 and any('more than one image' in b and 'AA BB' in b for b in bad),
       'a case in two images is reported with both labels')
    bad, rows, counts = check(good.replace(',dEQP-GLES3.info.*', ''), mp)
    ok(counts[1] == 1 and any('matched by no image, first: dEQP-GLES3.info.vendor' in b for b in bad),
       'a case no image selects is reported by name')
    bad, rows, counts = check(good + '# image CC: nothing\ndEQP-GLES3.functional.d.*\n', mp)
    ok(counts[3] == 1 and any('CC selects no mustpass case' in b for b in bad), 'an empty image is reported')
    bad, rows, counts = check(good.replace('AA: 2 of a', 'AA: 3 of a'), mp)
    ok(any('AA: the note says 3, the filter selects 2' in b for b in bad), 'a stale count in the note is reported')
    bad, rows, counts = check(good.replace('image BB', 'image AA'), mp)
    ok(any("'AA' is used twice" in b for b in bad), 'a label used twice is reported')
    bad, rows, counts = check(good.replace('image BB', 'image B-2'), mp)
    ok(any("'B-2' is not [A-Za-z0-9]+" in b for b in bad), 'a label with a dash is refused (the builders cannot read it)')
    bad, rows, counts = check(good.replace('functional.c.*,', 'functional.c.*,,'), mp)
    ok(any('BB: an empty pattern' in b for b in bad), 'an empty pattern (a doubled comma) is reported')
    bad, rows, counts = check(good.replace('dEQP-GLES3.info.*', 'dEQP-GLES2.info.*'), mp)
    ok(any("does not start with dEQP-GLES3." in b for b in bad), 'a GLES2 pattern in a GLES3 table is reported')
    bad, rows, counts = check(good, mp, max_cases=3)
    ok(any('BB selects 4 mustpass cases, more than 3' in b for b in bad), '--max reports an image over the size')
    unl = 'dEQP-GLES3.functional.a.*\ndEQP-GLES3.functional.b.*,dEQP-GLES3.functional.c.*,dEQP-GLES3.info.*\n'
    bad, rows, counts = check(unl, mp)
    ok(bad == [] and [r[0] for r in rows] == ['img1', 'img2'], 'an unlabelled table (the AM62P form) names its rows img<N>')
    ok(not row_regex('dEQP-GLES3.functional.a.*').match('dEQP-GLES3.functional.ab.x')
       and row_regex('dEQP-GLES3.functional.a.*').match('dEQP-GLES3.functional.a.x.y')
       and row_regex('dEQP-GLES3.functional.a.x').match('dEQP-GLES3.functional.a.x')
       and not row_regex('dEQP-GLES3.functional.a.x').match('dEQP-GLES3.functional.a.xy'),
       "'*' matches any tail, a dot is literal, a name without '*' matches itself only")
    print('check-gles3-tree self-test: %d checks failed' % fails)
    return fails


def main(argv):
    if argv[1:] == ['--self-test']:
        return 1 if self_test() else 0
    mustpass = os.path.join(HERE, 'gles3-mustpass.txt')
    max_cases = None
    tables = []
    i = 1
    while i < len(argv):
        if argv[i] == '--mustpass' and i + 1 < len(argv):
            mustpass = argv[i + 1]
            i += 2
        elif argv[i] == '--max' and i + 1 < len(argv):
            max_cases = int(argv[i + 1])
            i += 2
        elif argv[i].startswith('-'):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            tables.append(argv[i])
            i += 1
    if not tables:
        print(__doc__, file=sys.stderr)
        return 2
    if not os.path.isfile(mustpass):
        print('check-gles3-tree: no mustpass list %s' % mustpass, file=sys.stderr)
        return 2
    rc = 0
    for t in tables:
        if not os.path.isfile(t):
            print('check-gles3-tree: no table %s' % t, file=sys.stderr)
            return 2
        rc |= check_file(t, mustpass, max_cases)
    return rc


if __name__ == '__main__':
    sys.exit(main(sys.argv))
