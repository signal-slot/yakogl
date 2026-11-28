#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare a dEQP-GLES3 results file with docs/es3/mrt4-expected.tsv (RC14).

  tools/cts/mrt4-verdict.py --gpu v3d|pvr RESULTS [--expected FILE]

RESULTS is a TestResults.qpa (#beginTestCaseResult ... <Result StatusCode=..>)
or a plain list of 'case<TAB>status' lines (further columns and '#' lines are
ignored). Only cases named by the expected file's rows of the chosen GPU
(v3d -> v3d42, pvr -> bxs) are looked at; all other cases are ignored.

Reading the expected file:
  * the first whitespace-separated word of the case column is the pattern;
    '{a,b,n-m}' alternation/ranges and '*' wildcards are expanded;
  * the expected outcome is the status words of the column, split at '|' and
    stripped of '(...)' and of '; ...' notes: 'Pass|Fail(x); unmeasured' is
    {Pass, Fail}. 'unmeasured' alone, or a case that no results line names
    while the row is 'unmeasured', is skipped (not counted);
  * a row whose case column has prose after the pattern is a description of
    a group, not a pattern, and is skipped, except '(the other N)' rows,
    which apply to the cases no earlier row of that GPU claimed;
  * the first row that names a case wins.

Output, one line per differing expected case, in file order, cases sorted:
  MRT4 <gpu> <case> expected=<x> got=<y> REGRESSION|IMPROVEMENT|MISSING
then
  MRT4 VERDICT <gpu> PASS|FAIL n_match=N n_regress=N n_missing=N
(n_improve=N is not part of the line; improvements do not fail). FAIL when a
case regressed or is missing. Exit 0 on PASS, 1 on FAIL, 2 on unreadable input.
"""
import argparse
import fnmatch
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_EXPECTED = os.path.join(HERE, '..', '..', 'docs', 'es3', 'mrt4-expected.tsv')
GPU = {'v3d': 'v3d42', 'pvr': 'bxs'}
# lower is better
RANK = {'Pass': 0, 'CompatibilityWarning': 1, 'QualityWarning': 1, 'Waiver': 1,
        'NotSupported': 2, 'Fail': 3, 'ResourceError': 4, 'InternalError': 4,
        'Timeout': 4, 'Crash': 4, 'Pending': 5, 'No result': 5}
STATUS = set(RANK)


def parse_results(text):
    """{case: status}. qpa when it has #beginTestCaseResult, else plain."""
    out = {}
    if '#beginTestCaseResult' in text:
        case = None
        for line in text.splitlines():
            m = re.match(r'#beginTestCaseResult\s+(\S+)', line)
            if m:
                case = m.group(1)
                out.setdefault(case, 'No result')
                continue
            m = re.search(r'<Result\s+StatusCode="([^"]+)"', line)
            if m and case:
                out[case] = m.group(1)
            elif line.startswith('#endTestCaseResult'):
                case = None
        return out
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        f = line.split('\t') if '\t' in line else line.split()
        if len(f) >= 2 and f[1].strip() in STATUS:
            out[f[0].strip()] = f[1].strip()
    return out


def expand(pat):
    m = re.search(r'\{([^{}]*)\}', pat)
    if not m:
        return [pat]
    alts = []
    for a in m.group(1).split(','):
        r = re.fullmatch(r'(\d+)-(\d+)', a)
        alts += [str(i) for i in range(int(r.group(1)), int(r.group(2)) + 1)] if r else [a]
    res = []
    for a in alts:
        res += expand(pat[:m.start()] + a + pat[m.end():])
    return res


def expected_set(col):
    """Set of statuses, or None when the row gives no outcome."""
    s = set()
    for alt in col.split(';')[0].split('|'):
        w = re.sub(r'\(.*', '', alt).strip().split()
        if w and w[0] in STATUS:
            s.add(w[0])
    return s or None


def read_expected(path, gpu):
    rows = []
    with open(path, encoding='utf-8') as f:
        for line in f:
            if line.startswith('#') or not line.strip():
                continue
            c = line.rstrip('\n').split('\t')
            if len(c) < 4 or c[0] == 'case' or c[1] != GPU[gpu]:
                continue
            words = c[0].split(None, 1)
            prose = words[1].strip() if len(words) > 1 else ''
            rows.append((words[0], prose, expected_set(c[3]), c[3].strip()))
    return rows


def compare(rows, results):
    """(lines, n_match, n_regress, n_improve, n_missing)."""
    claimed, diffs = set(), []
    n = {'match': 0, 'regress': 0, 'improve': 0, 'missing': 0}
    for pat, prose, exp, raw in rows:
        if prose and not prose.startswith('(the other'):
            continue
        names = []
        for p in expand(pat):
            if '*' in p:
                names += sorted(k for k in results if fnmatch.fnmatchcase(k, p))
                if not any(fnmatch.fnmatchcase(k, p) for k in results) and exp:
                    names.append(p)
            else:
                names.append(p)
        for case in names:
            if case in claimed:
                continue
            claimed.add(case)
            if exp is None:
                continue
            got = results.get(case)
            label = '|'.join(sorted(exp, key=RANK.get))
            if got is None:
                diffs.append((case, label, 'missing', 'MISSING'))
                n['missing'] += 1
            elif got in exp:
                n['match'] += 1
            elif RANK.get(got, 9) < min(RANK[e] for e in exp):
                diffs.append((case, label, got, 'IMPROVEMENT'))
                n['improve'] += 1
            else:
                diffs.append((case, label, got, 'REGRESSION'))
                n['regress'] += 1
    return diffs, n


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0],
                                 epilog=__doc__.split('\n\n', 1)[1],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--gpu', required=True, choices=sorted(GPU))
    ap.add_argument('--expected', default=DEFAULT_EXPECTED)
    ap.add_argument('results')
    a = ap.parse_args(argv)
    try:
        with open(a.results, encoding='utf-8', errors='replace') as f:
            results = parse_results(f.read())
        rows = read_expected(a.expected, a.gpu)
    except OSError as e:
        print('mrt4-verdict: %s' % e, file=sys.stderr)
        return 2
    diffs, n = compare(rows, results)
    for case, exp, got, kind in diffs:
        print('MRT4 %s %s expected=%s got=%s %s' % (a.gpu, case, exp, got, kind))
    ok = n['regress'] == 0 and n['missing'] == 0
    print('MRT4 VERDICT %s %s n_match=%d n_regress=%d n_missing=%d' % (
        a.gpu, 'PASS' if ok else 'FAIL', n['match'], n['regress'], n['missing']))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
