#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft the i.MX8MP dEQP-GLES3 results lines from the tree slice records.

Runs imx8mp-gles3-merge.py --check and --totals as subprocesses and prints, to stdout, four headed
blocks: the RESULTS.md entry, the STATUS.md line (Japanese), the ROADMAP stage 4 line and the text of
the status comment for GitHub issue #2. It edits no file. When --check fails (missing entries, reset
verdicts, several commits) the drafts are still printed, marked PARTIAL, the problems are named, and the
exit status is 1.

  imx8mp-gles3-report.py [--table T] [--mustpass M] [--date YYYY-MM-DD] [--root R] [--apply R] <slice dir>...

After the drafts a block "anchors" names, for RESULTS.md, STATUS.md and ROADMAP.md of the tree R (default:
this checkout), the current text each draft replaces, found by matching in the file, with its line number
for reference. --apply R rewrites exactly those texts in R. It writes nothing and exits 1 when an anchor is
missing or matches more than once (or when the drafts are PARTIAL).
"""
import argparse
import datetime
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MERGE = os.path.join(HERE, 'imx8mp-gles3-merge.py')
HEX40 = re.compile(r'\b[0-9a-f]{40}\b')
HEXREF = re.compile(r'(?i)(?:main|commit)\W+`?([0-9a-f]{8,40})\b')


def merge(mode, args):
    cmd = [sys.executable, MERGE, '--table', args.table, '--mustpass', args.mustpass, mode, *args.dirs]
    p = subprocess.run(cmd, capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def commit_of(d):
    """The commit a slice names: a README's 40-hex, else main/commit <hex>, else the directory's -<hex> suffix."""
    for name in sorted(os.listdir(d)):
        if name.lower().startswith('readme'):
            txt = open(os.path.join(d, name), errors='replace').read()
            m = HEX40.search(txt)
            if m:
                return m.group(0)
            m = HEXREF.search(txt)
            if m:
                return m.group(1).lower()
    m = re.search(r'-([0-9a-f]{8,40})$', os.path.basename(os.path.normpath(d)))
    return m.group(1) if m else None


def expand(c):
    """A short hash as the full one when this checkout knows it."""
    if len(c) == 40:
        return c
    try:
        r = subprocess.run(['git', '-C', HERE, 'rev-parse', '--verify', '--quiet', c + '^{commit}'],
                           capture_output=True, text=True)
        if r.returncode == 0 and re.fullmatch(r'[0-9a-f]{40}', r.stdout.strip()):
            return r.stdout.strip()
    except OSError:
        pass
    return c


def parse_totals(text):
    tot = miss = None
    for line in text.splitlines():
        f = line.split()
        if f and f[0] == 'total' and len(f) == 8:
            tot = [int(x) for x in f[1:]]
        m = re.match(r'no record: (\d+) of (\d+) entries \(([^)]*)\), (\d+) mustpass cases', line)
        if m:
            miss = (int(m.group(1)), int(m.group(2)), m.group(3), int(m.group(4)))
    return tot, miss


def rel(d):
    root = os.path.dirname(os.path.dirname(HERE))
    r = os.path.relpath(os.path.abspath(d), root)
    return (d if r.startswith('..') else r).rstrip('/')


# (file under the repo root, regex of the current text, what it is). Matched in the file, not by line number.
ANCHORS = [
    ('tools/cts/RESULTS.md', re.compile(r'^\*\*i\.MX8MP\*\*: not run on the board[^\n]*$', re.M), 'RESULTS.md i.MX8MP line'),
    ('tools/cts/STATUS.md', re.compile(r'Pi 4 と i\.MX8MP の GLES3 全ツリー。'), 'STATUS.md next-steps sentence'),
    ('ROADMAP.md', re.compile(r'^- i\.MX8MP: not run on the board yet[^\n]*(?:\n  [^\n]*)*', re.M), 'ROADMAP.md stage 4 i.MX8MP bullet'),
]


def find_anchors(root, repl):
    """[(label, path, text, match-or-None, problem-or-None, replacement)] for the three docs under root."""
    res = []
    for (rel_path, rx, label), new in zip(ANCHORS, repl):
        path = os.path.join(root, rel_path)
        try:
            text = open(path, encoding='utf-8', newline='').read()
        except OSError as e:
            res.append((label, rel_path, None, None, 'cannot read %s: %s' % (path, e.strerror), new))
            continue
        ms = list(rx.finditer(text))
        if len(ms) != 1:
            res.append((label, rel_path, text, None, 'anchor %s in %s: %s' % (
                'missing' if not ms else 'matches %d times' % len(ms), rel_path, rx.pattern), new))
        else:
            res.append((label, rel_path, text, ms[0], None, new))
    return res


def n(x):
    return f'{x:,}'


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--date', default=datetime.date.today().isoformat(), help='date of the drafts (default: today)')
    ap.add_argument('--root', default=os.path.dirname(os.path.dirname(HERE)), help='tree whose docs the anchors are read from')
    ap.add_argument('--apply', metavar='ROOT', help='rewrite the anchored lines in the tree ROOT (all or nothing)')
    ap.add_argument('dirs', nargs='+', help='slice record directories (tools/cts/log/*-imx8mp-gles3-*/)')
    args = ap.parse_args()

    rc_c, out_c, err_c = merge('--check', args)
    rc_t, out_t, err_t = merge('--totals', args)
    problems = [l[5:].strip() for l in out_c.splitlines() if l.startswith('FAIL:')]
    if rc_c not in (0, 1):
        problems.append('merge --check could not run: %s' % (err_c.strip() or out_c.strip() or 'exit %d' % rc_c))
    tot, miss = parse_totals(out_t)
    if rc_t != 0 or tot is None:
        problems.append('merge --totals gave no total row: %s' % (err_t.strip() or 'exit %d' % rc_t))
        tot = [0] * 7
    if miss is None:
        miss = (0, 0, 'none', 0)
    ok = rc_c == 0 and not problems

    commits = []
    for d in args.dirs:
        c = commit_of(d) if os.path.isdir(d) else None
        if c and not any(c[:8] == x[:8] for x in commits):
            commits.append(c)
    full = [expand(c) for c in commits]
    commit_txt = ' and '.join('`%s`' % c for c in full) or 'an unnamed commit'
    short_txt = ' and '.join('`%s`' % c[:8] for c in full) or 'an unnamed commit'

    mp, begun, npass, nfail, nns, nother, nnone = tot
    n_entries = miss[1] or 133
    run_n = n_entries - miss[0]
    if miss[0]:
        entries_txt = '%d of %d entries (no record: %s)' % (run_n, n_entries, miss[2])
    else:
        entries_txt = 'all %d entries (e0..e%d)' % (n_entries, n_entries - 1)
    dirs_txt = ', '.join('`%s/`' % rel(d) for d in args.dirs)
    partial = '' if ok else 'PARTIAL: '
    bad_verdict = any('verdict' in p for p in problems)
    reset_txt = ('reset verdicts NOT CLEAN, see the problems' if bad_verdict else
                 '0 resets (every entry run is `verdict=done`, `resets=absent`)')
    five = tuple(n(x) for x in (npass, nfail, nns, nother, nnone))

    print('=== imx8mp-gles3-report: i.MX8MP dEQP-GLES3 drafts (%s) ===' % args.date)
    if not ok:
        print('*** PARTIAL: merge --check failed (exit %d); these drafts are not final ***' % rc_c)
        for p in problems:
            print('  problem: %s' % p)
    print()
    print('--- RESULTS.md (ES 3.0 section, replaces "**i.MX8MP**: not run on the board") ---')
    results_txt = ('%s**i.MX8MP, whole tree** on %s (tools/cts/imx8mp-gles3-tree.txt, %d entries; recipe\n'
                   '`docs/es3/board-recipe-imx8mp.md`), %s: %s, %s mustpass cases (%s begun):' % (
                       partial, commit_txt, n_entries, args.date, entries_txt, n(mp), n(begun)))
    results_txt += '\n\n| Pass | Fail | NotSupported | Other | No result |\n|---:|---:|---:|---:|---:|\n'
    results_txt += '| %s | %s | %s | %s | %s |\n' % five
    results_txt += '\n- Records: %s; %s.\n' % (dirs_txt, reset_txt)
    results_txt += '- Every case that is not Pass: `docs/es3/imx8mp-gles3-notpass.tsv`.'
    for p in problems:
        results_txt += '\n- PARTIAL, not final: %s.' % p
    print(results_txt)
    print()
    print('--- STATUS.md (Japanese, current state only) ---')
    status_txt = ('i.MX8MP の dEQP-GLES3 は%s 全 %d エントリのうち %d エントリを %s（%s）のボードで実行し、必須 %s 件中 合格 %s / 失敗 %s / 未対応 %s / その他 %s / 結果なし %s、'
          'リセット%s。記録は %s。' % (
              '' if ok else '（途中、記録が不完全）', n_entries, run_n, short_txt, args.date, n(mp), *five,
              'の確認が未完' if bad_verdict else '0', ', '.join(rel(d) for d in args.dirs)))
    print(status_txt)
    print()
    print('--- ROADMAP.md (stage 4, i.MX8MP line) ---')
    roadmap_txt = ('- i.MX8MP: %sdEQP-GLES3 %s on the board at %s (%s): %s Pass / %s Fail / %s NotSupported / %s other / %s without a result '
          'of %s mustpass cases (#24).' % (
              partial, 'whole tree, %d entries,' % n_entries if not miss[0] else '%d of %d entries,' % (run_n, n_entries),
              short_txt, args.date, *five, n(mp)))
    print(roadmap_txt)
    print()
    print('--- GitHub issue #2 comment ---')
    print('%s**i.MX8MP dEQP-GLES3, %s**' % (partial, args.date))
    print()
    print('- Commit: %s.' % commit_txt)
    print('- Entries run: %s.' % entries_txt)
    print('- Records: %s.' % dirs_txt)
    print('- Resets: %s.' % reset_txt)
    print('- Totals (%s of %s mustpass cases begun): %s Pass / %s Fail / %s NotSupported / %s other / %s without a result.'
          % (n(begun), n(mp), *five))
    left = []
    if miss[0]:
        left.append('entries %s (%d mustpass cases) have no record' % (miss[2], miss[3]))
    left += [p for p in problems if not p.startswith('no record:')]
    left.append('triage of the not-pass groups into issues (`docs/es3/imx8mp-gles3-notpass.tsv`)')
    print('- Remains: %s.' % '; '.join(left))

    repl = [results_txt, status_txt + 'Pi 4 の GLES3 全ツリー。', roadmap_txt]
    anchors = find_anchors(args.apply or args.root, repl)
    print()
    print('--- anchors (the current text each draft replaces, matched in the file; tree %s) ---' % (args.apply or args.root))
    for label, path, text, m, prob, new in anchors:
        if prob:
            print('%s: %s' % (label, prob))
            continue
        line = text.count('\n', 0, m.start()) + 1
        print('%s: %s line %d, replaces' % (label, path, line))
        for l in m.group(0).split('\n'):
            print('  - %s' % l)
        print('  with the %s draft above%s' % (path.rsplit('/', 1)[-1], ' (followed by "Pi 4 の GLES3 全ツリー。")' if 'STATUS' in path else ''))
    if args.apply:
        bad = [a[4] for a in anchors if a[4]]
        if not ok:
            bad.append('the drafts are PARTIAL')
        if bad:
            print('--apply: refused, nothing written: %s' % '; '.join(bad))
            return 1
        for label, path, text, m, prob, new in anchors:
            out = text[:m.start()] + new + text[m.end():]
            with open(os.path.join(args.apply, path), 'w', encoding='utf-8', newline='') as f:
                f.write(out)
        print('--apply: rewrote %s' % ', '.join(a[1] for a in anchors))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
