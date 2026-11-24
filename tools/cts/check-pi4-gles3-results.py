#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Do the Pi 4 ES 3.0 whole-tree entries agree with the tree and the mustpass?

  tools/cts/check-pi4-gles3-results.py [--results FILE] [--status FILE]
      [--roadmap FILE] [--tree FILE] [--mustpass FILE] [--tsv FILE]
      [--git [--ref REF]]

Reads the paragraph of the section "## ES 3.0: dEQP-GLES3 (stage 4)" of
tools/cts/RESULTS.md that starts "**Pi 4, whole tree**", and checks:

  1. it names the build commit (first backticked hex word of 7 to 40 digits);
     with --git that commit must be an ancestor of REF (default origin/main);
  2. every count it states agrees with tools/cts/pi4-gles3-tree.txt:
       "<N> rows"            must equal the tree's row count (its "# image"
                             labels, read with check-gles3-tree.py's
                             read_table);
       "<N> images"          must equal the original image count, 91, or the
                             row count ("114 of 114 images run" counts rows);
                             a count under 50 is a subset ("9 images
                             stop early") and is not checked;
       "<N> mustpass cases"  must equal the mustpass line count;
     and it must state a row count;
  3. the five counts "<a> Pass / <b> Fail / <c> NotSupported / <d> other /
     <e> without a result" add up to the line count of gles3-mustpass.txt
     (computed, not hard-coded);
  4. with --tsv (docs/es3/pi4-gles3-fails.tsv, rows "<record>:<status>
     <case>" as gles3-notpass.py writes them): Fail, NotSupported and
     NoResult row counts equal b, c and e, all other statuses
     (QualityWarning, ResourceError, ...) together equal d, and the commit
     in its header line, when it has one, is the build commit;
  5. the Pi 4 sentence of STATUS.md (from "Pi 4 は全ツリー") and the ROADMAP.md
     bullet "- Raspberry Pi 4: the whole tree" name the same commit and the
     same five numbers, and their "<N> rows" / "<N> images" obey rule 2.

One line per problem on stdout; exit 0 when there is none, else 1 (2 on a
file that cannot be read or an entry not found).
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
HEAD = '## ES 3.0: dEQP-GLES3 (stage 4)'
MARK = '**Pi 4, whole tree**'
STATUS_MARK = 'Pi 4 は全ツリー'
ROADMAP_MARK = '- Raspberry Pi 4: the whole tree'
ORIGINAL_IMAGES = 91
NAMES = ['pass', 'fail', 'NotSupported', 'other', 'without a result']
NUM = r'(\d[\d,]*)'
EN = re.compile(NUM + r' Pass\s*/\s*' + NUM + r' Fail\s*/\s*' + NUM +
                r' NotSupported\s*/\s*' + NUM + r' other\b[^/]*/\s*' + NUM +
                r' without a result', re.I)
JA = re.compile(r'合格\s*' + NUM + r'\s*/\s*失敗\s*' + NUM + r'\s*/\s*未対応\s*' + NUM +
                r'\s*/\s*その他\s*' + NUM + r'\s*/\s*結果なし\s*' + NUM)


def load_tree_module():
    spec = importlib.util.spec_from_file_location(
        'check_gles3_tree', os.path.join(HERE, 'check-gles3-tree.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


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
    return '\n'.join(out)


def entry(text, mark, stop):
    """The text from MARK to the end of its line (STATUS) or bullet
    (ROADMAP), cut at STOP when given."""
    out, on = [], False
    for l in text.splitlines():
        if not on:
            i = l.find(mark)
            if i < 0:
                continue
            on = True
            out.append(l[i:])
            if not mark.startswith('-'):
                break
            continue
        if re.match(r'\s*(-|$)', l):
            break
        out.append(l.strip())
    s = ' '.join(out)
    i = s.find(stop) if stop else -1
    return s[:i] if i > 0 else s


def counts(text, rx):
    m = rx.search(text)
    return [int(x.replace(',', '')) for x in m.groups()] if m else None


def commit_of(text, backticked):
    if backticked:
        rx = r'`([0-9a-f]{7,40})`'
    else:
        rx = r'(?<![0-9A-Za-z])((?=[0-9a-f]*\d)[0-9a-f]{7,40})(?![0-9A-Za-z])'
    m = re.search(rx, text)
    return m.group(1) if m else None


def same_commit(a, b):
    return a.startswith(b) or b.startswith(a)


def tree_rows_stated(text):
    """The "<N> rows" / "<N> 行" counts that are about the tree: not the
    row count of the TSV ("fails.tsv` (6,888 rows; ...")."""
    out = []
    for m in re.finditer(r'(?<![\d,])(\d[\d,]*) (?:rows|行)', text):
        if 'fails.tsv' in text[max(0, m.start() - 40):m.start()]:
            continue
        out.append(int(m.group(1).replace(',', '')))
    return out


def check_counts_text(what, text, rows, total, P):
    nrows = tree_rows_stated(text)
    if not nrows:
        P.append('%s states no "<N> rows" count (the tree has %d rows)' % (what, rows))
    for n in sorted(set(nrows)):
        if n != rows:
            P.append('%s says %d rows, pi4-gles3-tree.txt has %d' % (what, n, rows))
    for n in sorted(set(int(x) for x in re.findall(r'(?<![\d,])(\d+) (?:images|画像)', text))):
        if n >= 50 and n not in (ORIGINAL_IMAGES, rows):
            P.append('%s says %d images, expected %d (original) or %d (rows)' % (
                what, n, ORIGINAL_IMAGES, rows))
    for n in sorted(set(int(x.replace(',', '')) for x in
                        re.findall(r'\b(\d[\d,]*) mustpass cases\b', text))):
        if n != total:
            P.append('%s says %d mustpass cases, gles3-mustpass.txt has %d' % (what, n, total))


def tsv_counts(text):
    c, head = {}, None
    for l in text.splitlines():
        if l.startswith('#'):
            m = re.search(r'dEQP-GLES3 at ([0-9a-f]{7,40})', l)
            head = head or (m.group(1) if m else None)
            continue
        if not l.strip():
            continue
        st = l.split(' ', 1)[0].split(':', 1)[-1]
        c[st] = c.get(st, 0) + 1
    return c, head


def check(results, status, roadmap, tree, mustpass, tsv=None, git_ref=None, root=None):
    """(problems, fatal)."""
    P = []
    sec = section(results)
    if sec is None:
        return ['no section "%s" in RESULTS.md' % HEAD], True
    block = whole_tree_block(sec)
    if not block:
        return ['section has no paragraph "%s"' % MARK], True
    st = entry(status, STATUS_MARK, 'i.MX8MP')
    rm = entry(roadmap, ROADMAP_MARK, None)
    if not st:
        return ['STATUS.md has no sentence "%s"' % STATUS_MARK], True
    if not rm:
        return ['ROADMAP.md has no bullet "%s"' % ROADMAP_MARK], True
    rows = len(load_tree_module().read_table(tree))
    total = len([l for l in mustpass.splitlines() if l.strip()])
    sha = commit_of(block.split(MARK, 1)[1], True)
    if not sha:
        P.append('RESULTS.md Pi 4 paragraph names no build commit (a backticked hex sha)')
    elif git_ref:
        r = subprocess.run(['git', 'merge-base', '--is-ancestor', sha, git_ref],
                           cwd=root, capture_output=True)
        if r.returncode != 0:
            P.append('build commit %s is not an ancestor of %s' % (sha, git_ref))
    check_counts_text('RESULTS.md Pi 4 paragraph', block, rows, total, P)
    check_counts_text('STATUS.md Pi 4 sentence', st, rows, total, P)
    check_counts_text('ROADMAP.md Pi 4 bullet', rm, rows, total, P)
    cnt = counts(block, EN)
    if cnt is None:
        P.append('RESULTS.md Pi 4 paragraph has no "<a> Pass / <b> Fail / <c> NotSupported '
                 '/ <d> other / <e> without a result" counts')
    elif sum(cnt) != total:
        P.append('RESULTS.md counts add up to %d, mustpass has %d cases (%s)' % (
            sum(cnt), total, ' + '.join(map(str, cnt))))
    for what, text, rx in (('STATUS.md', st, JA), ('ROADMAP.md', rm, EN)):
        c2 = counts(text, rx)
        csha = commit_of(text, False)
        if c2 is None:
            P.append('%s Pi 4 entry has no five counts in the form its language uses' % what)
        elif cnt is not None and c2 != cnt:
            P.append('%s Pi 4 counts %s differ from RESULTS.md %s' % (
                what, ' / '.join(map(str, c2)), ' / '.join(map(str, cnt))))
        if not csha:
            P.append('%s Pi 4 entry names no commit' % what)
        elif sha and not same_commit(csha, sha):
            P.append('%s Pi 4 entry names commit %s, RESULTS.md names %s' % (what, csha, sha))
    if tsv is not None and cnt is not None:
        t, thead = tsv_counts(tsv)
        known = {'Fail': 1, 'NotSupported': 2, 'NoResult': 4}
        for k, i in known.items():
            if t.get(k, 0) != cnt[i]:
                P.append('%s: RESULTS.md says %d, TSV has %d %s rows' % (
                    NAMES[i], cnt[i], t.get(k, 0), k))
        rest = sorted((k, v) for k, v in t.items() if k not in known)
        other = sum(v for _, v in rest)
        if other != cnt[3]:
            P.append('other: RESULTS.md says %d, TSV has %d rows of other statuses (%s)' % (
                cnt[3], other, ', '.join('%s %d' % kv for kv in rest)))
        if thead and sha and not same_commit(thead, sha):
            P.append('TSV header names commit %s, RESULTS.md names %s' % (thead, sha))
    return P, False


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--results', default=os.path.join(HERE, 'RESULTS.md'))
    ap.add_argument('--status', default=os.path.join(HERE, 'STATUS.md'))
    ap.add_argument('--roadmap', default=os.path.join(ROOT, 'ROADMAP.md'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'pi4-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--tsv', default=os.path.join(ROOT, 'docs/es3/pi4-gles3-fails.tsv'))
    ap.add_argument('--git', action='store_true')
    ap.add_argument('--ref', default='origin/main')
    a = ap.parse_args()
    try:
        def rd(f):
            with open(f, encoding='utf-8') as fh:
                return fh.read()
        res, sta, road = rd(a.results), rd(a.status), rd(a.roadmap)
        tree, mp, tsv = rd(a.tree), rd(a.mustpass), rd(a.tsv)
    except OSError as e:
        print('check-pi4-gles3-results: %s' % e)
        return 2
    probs, fatal = check(res, sta, road, tree, mp, tsv, a.ref if a.git else None,
                         os.path.dirname(os.path.abspath(a.results)))
    for p in probs:
        print('check-pi4-gles3-results: %s' % p)
    return 2 if fatal else (1 if probs else 0)


if __name__ == '__main__':
    sys.exit(main())
