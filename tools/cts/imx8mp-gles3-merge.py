#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Merge i.MX8MP dEQP-GLES3 tree records into a not-pass list and totals.

  imx8mp-gles3-merge.py [--table T] [--mustpass M] (--notpass | --totals | --check) DIR...
  imx8mp-gles3-merge.py --selftest        (synthetic records under $TMPDIR)

Each DIR is a `tools/board.sh imx8mp tree` record (results.tsv: index, filter, begun, passed, failed, notsup,
other, noresult, failing names; rows.md; docs/es3/board-recipe-imx8mp.md sections 3-4) or a committed slice
under tools/cts/log/*-imx8mp-gles3-*/ (a results.tsv, or one *results.tsv, plus a README naming the
origin/main commit). The commit is the first 40-hex string of the README, else the hash after "main" or
"commit" in it, else the 8-hex tail of a directory named es3-<hash>. Two commits agree when one is a prefix
of the other (8 characters at least).

The table (tools/cts/imx8mp-gles3-tree.txt) and the mustpass list give each entry's cases, matched as
tools/cts/check-imx8mp-gles3-tree.py does (dEQP's filter: comma-separated patterns, '*' = any run, whole name).

--notpass  `e<i>:<status> <case>` per mustpass case a record names as failing (Fail), and `e<i>:NoResult
           <case>` for every case of an entry whose row has no usable record. A record names only the failing
           cases, not the n.s./other/none ones, so each row also gets a `# e<i>: unresolved N ...` comment with
           the cases it cannot place (mustpass - pass - named fails; non-mustpass cases a wildcard also begins
           can hide some).
--totals   per entry and overall: mustpass, begun, pass, fail, n.s., other, none (mustpass minus the
           others, at least 0), then the entries with no usable record.
--check    exit 1 when two inputs cover the same entry, the inputs name different commits (or none), a row
           does not fit the table (index, filter), or a case is listed twice. The other modes print the same
           problems on stderr and go on.
"""
import argparse, contextlib, io, os, re, shutil, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def pattern_regexes(entry):
    return [re.compile('^' + '.*'.join(re.escape(x) for x in p.split('*')) + '$') for p in entry.split(',') if p]


def load_table(path):
    return [l.rstrip('\n') for l in open(path) if l.strip() and not l.lstrip().startswith('#')]


def entry_cases(table, mustpass_path):
    """Per entry, its mustpass cases in mustpass order (a case matching several entries goes to each)."""
    cases = [l.strip() for l in open(mustpass_path) if l.strip() and not l.startswith('#')]
    pats = [pattern_regexes(e) for e in table]
    per = [[] for _ in table]
    for c in cases:
        for i, rs in enumerate(pats):
            if any(r.match(c) for r in rs):
                per[i].append(c)
    return per


def find_commit(d):
    for name in sorted(os.listdir(d)):
        if name.lower().startswith('readme'):
            txt = open(os.path.join(d, name), errors='replace').read()
            m = re.search(r'\b[0-9a-f]{40}\b', txt)
            if m:
                return m.group(0)
            m = re.search(r'(?i)(?:main|commit)\W+`?([0-9a-f]{8,40})\b', txt)
            if m:
                return m.group(1).lower()
    m = re.search(r'-([0-9a-f]{8,40})$', os.path.basename(os.path.normpath(d)))
    return m.group(1) if m else None


def same_commit(a, b):
    n = min(len(a), len(b))
    return n >= 8 and a[:n] == b[:n]


def find_results(d):
    p = os.path.join(d, 'results.tsv')
    if os.path.isfile(p):
        return p
    found = sorted(f for f in os.listdir(d) if f.endswith('results.tsv'))
    return os.path.join(d, found[0]) if len(found) == 1 else None


def parse_int(s):
    return int(s) if re.fullmatch(r'\d+', s) else None


def read_rows(path):
    """[(index, filter, [begun, passed, failed, notsup, other, noresult] or None, [names])]"""
    rows = []
    for l in open(path):
        l = l.rstrip('\n')
        if not l.strip() or l.startswith('#'):
            continue
        f = l.split('\t')
        f += ['-'] * (9 - len(f))
        idx = parse_int(f[0])
        if idx is None:
            continue
        nums = [parse_int(x) for x in f[2:8]]
        tally = None if any(n is None for n in nums[:5]) else [n or 0 for n in nums]
        names = [] if f[8] in ('-', '') else [n for part in f[8].split(';') for n in part.split(',') if n]
        rows.append((idx, f[1], tally, names))
    return rows


class Merge:
    def __init__(self, table, per, dirs):
        self.table, self.per = table, per
        self.problems = []
        self.commits = []      # (dir, commit)
        self.rec = {}          # entry -> dict
        self.info = []         # per-row comments
        for d in dirs:
            self.add(d)
        known = [c for _, c in self.commits if c]
        for d, c in self.commits:
            if not c:
                self.problems.append('%s names no commit' % d)
        for d, c in self.commits:
            if c and known and not same_commit(known[0], c):
                self.problems.append('different commits: %s (%s) and %s (%s)' % (self.commits[0][0], known[0], d, c))
        self.listed = {}
        for i in sorted(self.rec):
            r = self.rec[i]
            seen = set()
            for n in r['names']:
                if n in seen or n in self.listed:
                    self.problems.append('case listed twice: %s (e%d%s)' % (
                        n, i, '' if n in seen else ', also e%d' % self.listed[n]))
                seen.add(n)
                self.listed.setdefault(n, i)

    def add(self, d):
        if not os.path.isdir(d):
            self.problems.append('%s: not a directory' % d)
            return
        self.commits.append((d, find_commit(d)))
        path = find_results(d)
        if not path:
            self.problems.append('%s: no results.tsv' % d)
            return
        for idx, filt, tally, names in read_rows(path):
            if not 0 <= idx < len(self.table):
                self.problems.append('%s: entry %d is not in the table (%d entries)' % (d, idx, len(self.table)))
                continue
            if filt != self.table[idx]:
                self.problems.append('%s: entry %d filter differs from the table' % (d, idx))
                continue
            if idx in self.rec:
                self.problems.append('entry %d is in both %s and %s' % (idx, self.rec[idx]['dir'], d))
                continue
            must = set(self.per[idx])
            placed, outside, crc = [], 0, 0
            for n in names:
                if n in must:
                    placed.append(n)
                elif n.startswith('crc:') or n.startswith('PAGE'):
                    crc += 1
                else:
                    outside += 1
            self.rec[idx] = dict(dir=d, tally=tally, names=placed, outside=outside, crc=crc)

    def counts(self, i):
        """(mustpass, begun, pass, fail, ns, other, none) of an entry, or None without a record."""
        r = self.rec.get(i)
        m = len(self.per[i])
        if r is None or r['tally'] is None:
            return None
        b, p, f, s, o, _ = r['tally']
        return m, b, p, f, s, o, max(0, m - p - f - s - o)


def span(ids):
    ids, out, k = sorted(ids), [], 0
    while k < len(ids):
        j = k
        while j + 1 < len(ids) and ids[j + 1] == ids[j] + 1:
            j += 1
        out.append(str(ids[k]) if j == k else '%d-%d' % (ids[k], ids[j]))
        k = j + 1
    return ','.join(out) or 'none'


def do_notpass(m, out):
    commits = sorted({c for _, c in m.commits if c})
    out.write('# i.MX8MP dEQP-GLES3 at %s: every case that is not Pass, per entry of tools/cts/imx8mp-gles3-tree.txt\n'
              % (' and '.join(commits) or 'an unnamed commit'))
    out.write('# rows: e<i>:<status> <case>, in entry then mustpass order (NoResult = the entry has no usable record; '
              'a record names only the failing cases, so n.s./other/none cases show in the unresolved counts)\n')
    run = sorted(m.rec)
    out.write('# entries run (%d of %d): %s\n' % (len(run), len(m.table), span(run)))
    for i in run:
        r = m.rec[i]
        c = m.counts(i)
        if c is None:
            out.write('# e%d: no usable record, %d mustpass cases without a result\n' % (i, len(m.per[i])))
            for case in m.per[i]:
                out.write('e%d:NoResult %s\n' % (i, case))
            continue
        mp, b, p, f, s, o, _ = c
        unresolved = max(0, mp - p - len(r['names']))
        if unresolved or r['crc'] or r['outside'] or f != len(r['names']) or b < mp:
            out.write('# e%d: unresolved %d (mustpass %d, begun %d, pass %d, fail %d of which %d named, n.s. %d, other %d%s%s)\n'
                      % (i, unresolved, mp, b, p, f, len(r['names']), s, o,
                         ', %d names unresolved crc' % r['crc'] if r['crc'] else '',
                         ', %d names outside the mustpass' % r['outside'] if r['outside'] else ''))
        order = {n: k for k, n in enumerate(m.per[i])}
        for n in sorted(set(r['names']), key=order.get):
            out.write('e%d:Fail %s\n' % (i, n))


def do_totals(m, out):
    out.write('%-6s %8s %8s %8s %8s %8s %8s %8s\n' % ('entry', 'mustpass', 'begun', 'pass', 'fail', 'n.s.', 'other', 'none'))
    tot = [0] * 7
    for i in sorted(m.rec):
        c = m.counts(i)
        if c is None:
            c = (len(m.per[i]), 0, 0, 0, 0, 0, len(m.per[i]))
        out.write('e%-5d %8d %8d %8d %8d %8d %8d %8d\n' % ((i,) + c))
        tot = [a + b for a, b in zip(tot, c)]
    out.write('%-6s %8d %8d %8d %8d %8d %8d %8d\n' % (('total',) + tuple(tot)))
    missing = [i for i in range(len(m.table)) if m.counts(i) is None]
    out.write('no record: %d of %d entries (%s), %d mustpass cases\n' % (
        len(missing), len(m.table), span(missing), sum(len(m.per[i]) for i in missing)))


def run(argv, out=None):
    out = out or sys.stdout
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument('--notpass', action='store_true')
    g.add_argument('--totals', action='store_true')
    g.add_argument('--check', action='store_true')
    ap.add_argument('dirs', nargs='+')
    a = ap.parse_args(argv)
    table = load_table(a.table)
    m = Merge(table, entry_cases(table, a.mustpass), a.dirs)
    if a.check:
        for p in m.problems:
            print('FAIL:', p)
        print('checked %d inputs, %d entries with a record: %s' % (len(a.dirs), len(m.rec), 'FAILED' if m.problems else 'OK'))
        return 1 if m.problems else 0
    for p in m.problems:
        print('warning:', p, file=sys.stderr)
    (do_notpass if a.notpass else do_totals)(m, out)
    return 0


# ---- self-test --------------------------------------------------------------------------------

def selftest():
    base = os.environ.get('TMPDIR')
    if not base or not os.path.isdir(base):
        print('selftest: set TMPDIR to a directory (synthetic records are never written to /tmp)', file=sys.stderr)
        return 2
    root = tempfile.mkdtemp(prefix='imx8mp-merge-', dir=base)
    fails = []

    def expect(what, ok):
        if not ok:
            fails.append(what)
        print('%-4s %s' % ('ok' if ok else 'FAIL', what))

    def slice_(name, commit, rows):
        d = os.path.join(root, name)
        os.makedirs(d)
        with open(os.path.join(d, 'README.md'), 'w') as f:
            f.write('# slice at origin/main %s\n' % commit)
        with open(os.path.join(d, 'results.tsv'), 'w') as f:
            f.write('# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names\n')
            for r in rows:
                f.write('\t'.join(str(x) for x in r) + '\n')
        return d

    def call(*argv):
        buf, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(buf):
            rc = run(list(argv), buf)
        return rc, buf.getvalue(), err.getvalue()

    try:
        table = os.path.join(root, 'table.txt')
        must = os.path.join(root, 'mustpass.txt')
        entries = ['dEQP-GLES3.a.*', 'dEQP-GLES3.b.*,dEQP-GLES3.c.x', 'dEQP-GLES3.d.*', 'dEQP-GLES3.e.*']
        open(table, 'w').write('# synthetic\n' + '\n'.join(entries) + '\n')
        cases = (['dEQP-GLES3.a.%d' % k for k in range(4)] + ['dEQP-GLES3.b.%d' % k for k in range(3)] +
                 ['dEQP-GLES3.c.x'] + ['dEQP-GLES3.d.%d' % k for k in range(5)] + ['dEQP-GLES3.e.0'])
        open(must, 'w').write('# synthetic\n' + '\n'.join(cases) + '\n')
        c1, c2 = 'a' * 40, 'b' * 40
        A = slice_('a', c1, [
            (0, entries[0], 4, 4, 0, 0, 0, 0, '-'),                                    # all pass
            (1, entries[1], 4, 1, 1, 1, 0, 1, 'dEQP-GLES3.b.1;dEQP-GLES3.nonmust.z'),  # failing names
        ])
        B = slice_('b', c1[:8], [
            (2, entries[2], 3, 3, 0, 0, 0, 0, '-'),                                    # ended early: 3 of 5 begun
            (3, entries[3], '-', '-', '-', '-', '-', '-', '-'),                        # no usable record
        ])
        D = slice_('dup', c1, [(1, entries[1], 4, 4, 0, 0, 0, 0, '-')])               # entry 1 again
        X = slice_('mixed', c2, [(2, entries[2], 5, 5, 0, 0, 0, 0, '-')])             # other commit
        base_args = ['--table', table, '--mustpass', must]

        rc, out, _ = call(*base_args, '--check', A, B)
        expect('check accepts disjoint slices of one commit (full and 8-char hash)', rc == 0)
        rc, out, _ = call(*base_args, '--check', A, D)
        expect('check flags a duplicate entry', rc == 1 and 'entry 1 is in both' in out)
        rc, out, _ = call(*base_args, '--check', A, X)
        expect('check flags mixed commits', rc == 1 and 'different commits' in out)

        rc, out, _ = call(*base_args, '--notpass', A, B)
        lines = [l for l in out.splitlines() if not l.startswith('#')]
        expect('notpass: the named failing case is Fail', 'e1:Fail dEQP-GLES3.b.1' in lines)
        expect('notpass: all-pass row lists nothing', not any(l.startswith('e0:') for l in lines))
        expect('notpass: a name outside the mustpass is dropped', not any('nonmust' in l for l in lines))
        expect('notpass: the no-record entry is NoResult per case', lines.count('e3:NoResult dEQP-GLES3.e.0') == 1)
        expect('notpass: the early row has its unresolved count', '# e2: unresolved 2 ' in out)
        expect('notpass: entries run are named', '(4 of 4): 0-3' in out)

        rc, out, _ = call(*base_args, '--totals', A, B)
        expect('totals: overall row', re.search(r'^total\s+14\s+11\s+8\s+1\s+1\s+0\s+4$', out, re.M) is not None)
        expect('totals: the unusable record counts as no record', 'no record: 1 of 4 entries (3), 1 mustpass' in out)
        rc, out, _ = call(*base_args, '--totals', A)
        expect('totals: entries without a record', 'no record: 2 of 4 entries (2-3), 6 mustpass cases' in out)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print('selftest: %s' % ('FAILED (%d)' % len(fails) if fails else 'OK'))
    return 1 if fails else 0


def main():
    if '--selftest' in sys.argv[1:]:
        return selftest()
    return run(sys.argv[1:])


if __name__ == '__main__':
    sys.exit(main())
