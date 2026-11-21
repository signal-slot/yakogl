#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Does the AM62P ES 3.0 whole-tree row of RESULTS.md say what a run must?

  tools/cts/check-gles3-results.py [--results FILE] [--tree FILE]
      [--mustpass FILE] [--tsv FILE] [--git [--ref REF]]

Reads the section "## ES 3.0: dEQP-GLES3 (stage 4)" of tools/cts/RESULTS.md,
the paragraph that starts "**AM62P, whole tree**", and checks:

  1. it names the build commit (first backticked hex word of 7 to 40 digits
     after the marker); with --git that commit must be an ancestor of REF
     (default origin/main, `git merge-base --is-ancestor`);
  2. every "<N> images" / "<N>-image" it states equals the image count of
     tools/cts/am62p-gles3-tree.txt (filter lines, as check-gles3-tree.py
     counts them), and it states one;
  3. the table Pass | Fail | NotSupported | QualityWarning | No result has
     five numbers that add up to the line count of tools/cts/gles3-mustpass.txt
     (computed, not hard-coded);
  4. the section has no "31 images" / "31-image" text left (the retired table);
  5. with --tsv (docs/es3/am62p-gles3-notpass.tsv): the TSV's Fail,
     NotSupported, QualityWarning and NoResult row counts equal the table's.

One line per problem on stdout; exit 0 when there is none, else 1 (2 on a
file that cannot be read or a section not found).
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
HEAD = '## ES 3.0: dEQP-GLES3 (stage 4)'
MARK = '**AM62P, whole tree**'
COLS = ['Pass', 'Fail', 'NotSupported', 'QualityWarning', 'No result']
TSV_COLS = {'Fail': 'Fail', 'NotSupported': 'NotSupported',
            'QualityWarning': 'QualityWarning', 'No result': 'NoResult'}


def section(text):
    out, on, found = [], False, False
    for l in text.splitlines():
        if l.startswith('## '):
            if on:
                break
            on = l.strip() == HEAD
            found = found or on
            continue
        if on:
            out.append(l)
    return out if found else None


def whole_tree_block(lines):
    out, on = [], False
    for l in lines:
        if l.startswith(MARK):
            on = True
        elif on and l.startswith('**'):
            break
        if on:
            out.append(l)
    return out


def tree_images(text):
    return sum(1 for l in text.splitlines() if not re.match(r'\s*(#|$)', l))


def table_counts(block):
    """{column: number} of the first Pass | Fail ... table, or None."""
    for i, l in enumerate(block):
        cells = [c.strip() for c in l.strip().strip('|').split('|')]
        if cells == COLS and l.lstrip().startswith('|'):
            for r in block[i + 1:i + 5]:
                v = [c.strip().replace(',', '') for c in r.strip().strip('|').split('|')]
                if len(v) == len(COLS) and all(x.isdigit() for x in v):
                    return dict(zip(COLS, map(int, v)))
            return None
    return None


def tsv_counts(text):
    c = {}
    for l in text.splitlines():
        if not l.strip() or l.startswith('#'):
            continue
        k = l.split('\t', 1)[0]
        c[k] = c.get(k, 0) + 1
    return c


def check(results, tree, mustpass, tsv=None, git_ref=None, root=None):
    """(problems, fatal)."""
    P = []
    sec = section(results)
    if sec is None:
        return ['no section "%s" in RESULTS.md' % HEAD], True
    block = whole_tree_block(sec)
    if not block:
        return ['section has no paragraph "%s"' % MARK], True
    btext = '\n'.join(block)
    stext = '\n'.join(sec)
    after = btext.split(MARK, 1)[1]
    m = re.search(r'`([0-9a-f]{7,40})`', after)
    if not m:
        P.append('whole-tree row names no build commit (a backticked hex sha)')
    elif git_ref:
        r = subprocess.run(['git', 'merge-base', '--is-ancestor', m.group(1), git_ref],
                           cwd=root, capture_output=True)
        if r.returncode != 0:
            P.append('build commit %s is not an ancestor of %s' % (m.group(1), git_ref))
    want = tree_images(tree)
    nums = [int(x) for x in re.findall(r'\b(\d+)[ -]images?\b', btext)]
    if not nums:
        P.append('whole-tree row states no image count (expected %d images)' % want)
    for n in sorted(set(nums)):
        if n != want:
            P.append('whole-tree row says %d images, am62p-gles3-tree.txt has %d' % (n, want))
    for l in stext.splitlines():
        if re.search(r'\b31[ -]images?\b', l):
            P.append('stale "31 images" text: %s' % l.strip()[:100])
    cnt = table_counts(block)
    total = len([l for l in mustpass.splitlines() if l.strip()])
    if cnt is None:
        P.append('no table "%s" with five numbers in the whole-tree row' % ' | '.join(COLS))
    else:
        s = sum(cnt.values())
        if s != total:
            P.append('counts add up to %d, mustpass has %d cases (%s)' % (
                s, total, ' + '.join(str(cnt[c]) for c in COLS)))
        if tsv is not None:
            t = tsv_counts(tsv)
            for c, k in TSV_COLS.items():
                if t.get(k, 0) != cnt[c]:
                    P.append('%s: table says %d, TSV has %d %s rows' % (c, cnt[c], t.get(k, 0), k))
    return P, False


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--results', default=os.path.join(HERE, 'RESULTS.md'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'am62p-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--tsv')
    ap.add_argument('--git', action='store_true')
    ap.add_argument('--ref', default='origin/main')
    a = ap.parse_args()
    try:
        def rd(f):
            with open(f, encoding='utf-8') as fh:
                return fh.read()
        res, tree, mp = rd(a.results), rd(a.tree), rd(a.mustpass)
        tsv = rd(a.tsv) if a.tsv else None
    except OSError as e:
        print('check-gles3-results: %s' % e)
        return 2
    probs, fatal = check(res, tree, mp, tsv, a.ref if a.git else None,
                         os.path.dirname(os.path.abspath(a.results)))
    for p in probs:
        print('check-gles3-results: %s' % p)
    return 2 if fatal else (1 if probs else 0)


if __name__ == '__main__':
    sys.exit(main())
