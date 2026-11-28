#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the AM62P ES 3.0 summary of STATUS.md (Japanese, current state) from the TSV.

  python3 -I tools/cts/gles3-status-section.py [--tsv docs/es3/am62p-gles3-notpass.tsv]
      [--tree tools/cts/am62p-gles3-tree.txt] [--mustpass FILE]
      [--families tools/cts/am62p-gles3-families.txt] [--commit SHA] [--top N]
      [--expect-total 42810]

The sibling of gles3-results-section.py (RESULTS.md, English): the TSV reader
(parse, lines_of) is imported from it, the family rules from gles3-triage.py.
Prints the "ES 3.0（段階 4）" bullet: build commit, image count, the five counts
(Pass is the mustpass count minus the TSV rows) and their sum, the per-range
and rerun commits named in the TSV header, and the largest cause families (the
TSV's fourth column, else the rule file). Exit 0; 1 when the counts do not add
to --expect-total (default 42810) or a row has no family; 2 on an unreadable
file, a malformed header or a missing build commit.
"""
import argparse
import collections
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'),
                                                  os.path.join(HERE, name + '.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def check_header(tsv):
    for l in tsv.splitlines():
        h = l.lstrip('#').strip() if l.startswith('#') else ''
        if re.match(r'rows\b', h) and not re.match(
                r'rows \d+-\d+: build `?[0-9a-f]{7,40}`?, run \d{4}-\d{2}-\d{2}(/\d{1,2})?\s*$', h):
            raise ValueError('malformed TSV header line: %s' % l[:100])
        if re.match(r'rerun\b', h) and not re.match(r'rerun\s+`?[0-9a-f]{7,40}`?\b', h):
            raise ValueError('malformed TSV header line: %s' % l[:100])


def families(tsv, rules_path):
    tri = load('gles3-triage')
    rules = None
    out = collections.Counter()
    for l in tsv.splitlines():
        if not l.strip() or l.startswith('#'):
            continue
        p = l.split('\t')
        fam = p[3].strip() if len(p) > 3 else ''
        if not fam:
            if rules is None:
                rules = tri.parse_families(rules_path)
            fam = tri.family_of(rules, p[1].strip() if len(p) > 1 else '',
                                p[2].strip() if len(p) > 2 else '')
        out[fam] += 1
    return out


def build(tsv, tree, mustpass, rules_path, commit=None, top=5, expect=42810):
    rs = load('gles3-results-section')
    check_header(tsv)
    counts, _, header, ranges = rs.parse(tsv)
    problems = []
    total = len(rs.lines_of(mustpass))
    images = len(rs.lines_of(tree))
    rest = sum(counts.values())
    if commit is None:
        if ranges:
            commit = max(ranges, key=lambda r: r[3])[2]
        else:
            m = re.search(r'\bbuild\s+`?([0-9a-f]{7,40})\b', '\n'.join(header))
            if not m:
                raise ValueError('no build commit: pass --commit or name "build <sha>" in the TSV header')
            commit = m.group(1)
    if rest > total:
        raise ValueError('%d non-Pass rows exceed the %d mustpass cases' % (rest, total))
    nums = [total - rest] + [counts[k] for k in rs.OUTCOMES]
    if sum(nums) != expect:
        problems.append('counts add up to %d, expected %d' % (sum(nums), expect))
    fams = families(tsv, rules_path)
    if fams.get('unassigned') or fams.get(''):
        problems.append('%d rows have no family' % (fams.get('unassigned', 0) + fams.get('', 0)))
    f = rs.num
    labels = ['合格', '失敗', '未対応', '品質警告', '結果なし']
    text = ('- **ES 3.0（段階 4）。** AM62P の dEQP-GLES3 全ツリー（%s、%d 画像）は必須 %s 件のうち '
            '%s（計 %s）。' % (commit, images, f(total),
                              ' / '.join('%s %s' % (k, f(n)) for k, n in zip(labels, nums)),
                              f(sum(nums))))
    own = ['行 %s〜%s は %s（%s）' % (a, b, s, d[5:].replace('-', '/')) for a, b, s, d in ranges]
    if len({r[2] for r in ranges}) > 1:
        text += '行ごとのビルド: ' + '、'.join(own) + '。'
    reruns = [m.groups() for m in (re.match(r'rerun\s+`?([0-9a-f]{7,40})`?:?\s*(.*)$', h) for h in header) if m]
    if reruns:
        text += '再実行: ' + '、'.join('%s（%s）' % (s, w or 'TSV の行') for s, w in reruns) + '。'
    if fams:
        ranked = sorted(fams.items(), key=lambda kv: (-kv[1], kv[0]))
        parts = ['%s %s 件' % (n, f(c)) for n, c in ranked[:top]]
        more = sum(c for _, c in ranked[top:])
        text += '合格以外 %s 件の原因は %d 系統で、大きい順に %s' % (f(rest), len(ranked), '、'.join(parts))
        if more:
            text += '、残り %d 系統 %s 件' % (len(ranked) - top, f(more))
        text += '。'
    text += '失敗の原因別の一覧は docs/es3-cts-failures.md、全件は docs/es3/am62p-gles3-notpass.tsv。'
    return text + '\n', problems


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--tsv', default=os.path.join(ROOT, 'docs/es3/am62p-gles3-notpass.tsv'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'am62p-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--families', default=os.path.join(HERE, 'am62p-gles3-families.txt'))
    ap.add_argument('--commit')
    ap.add_argument('--top', type=int, default=5)
    ap.add_argument('--expect-total', type=int, default=42810)
    a = ap.parse_args()
    try:
        def rd(p):
            with open(p, encoding='utf-8') as fh:
                return fh.read()
        tsv, tree, mp = rd(a.tsv), rd(a.tree), rd(a.mustpass)
        text, problems = build(tsv, tree, mp, a.families, a.commit, a.top, a.expect_total)
    except (OSError, ValueError) as e:
        print('gles3-status-section: %s' % e, file=sys.stderr)
        return 2
    sys.stdout.write(text)
    for p in problems:
        print('gles3-status-section: ' + p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
