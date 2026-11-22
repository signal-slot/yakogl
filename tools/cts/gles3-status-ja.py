#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the Japanese STATUS.md paragraph for the AM62P dEQP-GLES3 whole-tree run.

  gles3-status-ja.py [--tsv docs/es3/am62p-gles3-notpass.tsv]
                     [--tree tools/cts/am62p-gles3-tree.txt]
                     [--mustpass tools/cts/gles3-mustpass.txt]
                     [--commit SHA] [--date MM/DD] [--top N]

Reads the not-pass TSV (with its family column), the image table and the
mustpass list. Pass = mustpass cases minus the non-Pass rows. Prints the
"ES 3.0（段階 4）" bullet text: commit, image count, the five counts (which add
up to the mustpass count) and the largest cause families, counted as
gles3-triage.py --check counts them (its parse_notpass is imported). --commit
defaults to the sha after "build" in the TSV's header comment. Exit 1 when a
row has an unknown outcome, an empty or 'unassigned' family, or the counts do
not add up.
"""
import argparse
import collections
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUTCOMES = [('Fail', '失敗'), ('NotSupported', '未対応'),
            ('QualityWarning', '品質警告'), ('NoResult', '結果なし')]


def load_triage():
    spec = importlib.util.spec_from_file_location(
        'gles3_triage', os.path.join(HERE, 'gles3-triage.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def header_commit(path):
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            if not line.startswith('#'):
                break
            m = re.search(r'\bbuild\s+([0-9a-f]{7,40})\b', line)
            if m:
                return m.group(1)
    return None


def count_lines(path):
    with open(path, encoding='utf-8') as f:
        return [l for l in f if l.strip() and not l.lstrip().startswith('#')]


def build(tsv, tree, mustpass, commit=None, date=None, top=5):
    tri = load_triage()
    rows = tri.parse_notpass(tsv)
    problems = []
    commit = commit or header_commit(tsv)
    if not commit:
        problems.append('no build commit: header has no "build <sha>" and no --commit')
    images = len(count_lines(tree))
    total = len(count_lines(mustpass))
    cnt = collections.Counter(r[0] for r in rows)
    known = [k for k, _ in OUTCOMES]
    for k in cnt:
        if k not in known:
            problems.append('unknown outcome %r (%d rows)' % (k, cnt[k]))
    bad = [r for r in rows if not r[3] or r[3] == 'unassigned']
    if bad:
        problems.append('%d rows with no family (first: %s)' % (len(bad), bad[0][1]))
    nonpass = sum(cnt[k] for k in known)
    npass = total - nonpass
    nums = [npass] + [cnt[k] for k in known]
    if npass < 0 or sum(nums) != total:
        problems.append('counts add up to %d, mustpass has %d' % (sum(nums), total))
    fams = sorted(collections.Counter(r[3] for r in rows).items(),
                  key=lambda kv: (-kv[1], kv[0]))
    f = lambda n: format(n, ',')
    when = '、%s' % date if date else ''
    text = ('- **ES 3.0（段階 4）。** AM62P の dEQP-GLES3 全ツリー（%s%s、%d 画像）は必須 %s 件のうち '
            '%s 合格 / %s。' % (commit, when, images, f(total), f(npass),
                               ' / '.join('%s %s' % (ja, f(cnt[k])) for k, ja in OUTCOMES)))
    if fams:
        parts = ['%s %s 件' % (n, f(c)) for n, c in fams[:top]]
        rest = sum(c for _, c in fams[top:])
        text += '合格以外 %s 件の原因は %d 系統で、大きい順に %s' % (
            f(nonpass), len(fams), '、'.join(parts))
        if rest:
            text += '、残り %d 系統 %s 件' % (len(fams) - top, f(rest))
        text += '。'
    text += '失敗の原因別の一覧は docs/es3-cts-failures.md、全件は docs/es3/am62p-gles3-notpass.tsv。'
    table = '\n'.join([
        '', '| 合格 | 失敗 | 未対応 | 品質警告 | 結果なし | 計 |', '|---|---|---|---|---|---|',
        '| ' + ' | '.join(f(n) for n in nums) + ' | %s |' % f(sum(nums))])
    return text, table, problems


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--tsv', default=os.path.join(ROOT, 'docs/es3/am62p-gles3-notpass.tsv'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'am62p-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--commit')
    ap.add_argument('--date')
    ap.add_argument('--top', type=int, default=5)
    a = ap.parse_args()
    text, table, problems = build(a.tsv, a.tree, a.mustpass, a.commit, a.date, a.top)
    print(text)
    print(table)
    for p in problems:
        print('gles3-status-ja: ' + p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
