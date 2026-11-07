#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft the Pi 4 dEQP-GLES3 results lines from the session directories.

  pi4-gles3-report.py [--table T] [--mustpass M] [--date YYYY-MM-DD] [--commit SHA]
                      [--root R] [--apply R] <session dir>...
  pi4-gles3-report.py --self-test

The session directories are what the board sessions write,
tools/cts/log/<date>-pi4-gles3-<sha8>-rowsAA-BB/ (README.md with a 'commit' line,
<LABEL>.console.txt and <LABEL>.tally.txt per image). Runs check-pi4-gles3-log.py
(completeness, one commit) and gles3-notpass.py --pi4 (the not-pass rows) as
subprocesses, reads the per-image totals from the tally files and prints, to stdout,
four headed blocks: the RESULTS.md entry, the STATUS.md line (Japanese), the ROADMAP
stage 4 Pi 4 line and the text of the status comment for GitHub issue #2. It edits
no file. When the completeness check fails the drafts are still printed, marked
PARTIAL, the problems are named and the exit status is 1.

After the drafts a block "anchors" names, for RESULTS.md, STATUS.md and ROADMAP.md of
the tree R (default: this checkout), the current Pi 4 ES 3.0 text each draft
replaces, found by matching in the file, with its line number. --apply R rewrites
exactly those texts in R. It writes nothing and exits 1 when an anchor is missing or
matches more than once, or when the drafts are PARTIAL.
"""
import argparse
import datetime
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CHECK = os.path.join(HERE, 'check-pi4-gles3-log.py')
NOTPASS = os.path.join(HERE, 'gles3-notpass.py')
COMMIT_LINE = re.compile(r'^commit:?[ \t]+`?([0-9a-f]{7,40})\b', re.M | re.I)
HEX40 = re.compile(r'\b[0-9a-f]{40}\b')
TOTAL_ROW = re.compile(r'^TOTAL\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$', re.M)
RECOVERIES = re.compile(r'(\d+) GPU recover', re.I)
PRIVATE = re.compile(r'/(?:home|Users)/[^\s:\'")]*|/dev/tty\S*')


def n(x):
    return f'{x:,}'


def scrub(text, dirs):
    """No absolute worker path in any output: session paths relative, the rest masked."""
    for d in dirs:
        text = text.replace(os.path.abspath(d), rel(d)).replace(os.path.realpath(d), rel(d))
    return PRIVATE.sub('<path>', text)


def rel(d):
    """A session directory as 'tools/cts/log/...' (relative to the repository root) or its basename."""
    r = os.path.relpath(os.path.abspath(d), ROOT)
    return r.rstrip('/') if not r.startswith('..') else os.path.basename(os.path.normpath(d))


def read(path):
    with open(path, encoding='utf-8', errors='replace') as f:
        return f.read()


def labels_of(table):
    out = []
    for line in read(table).splitlines():
        m = re.match(r'#\s*image\s+([^\s:]+)\s*:', line)
        if m and m.group(1) not in out:
            out.append(m.group(1))
    return out


def mustpass_count(path):
    return sum(1 for l in read(path).splitlines() if l.strip() and not l.startswith('#'))


def commit_of(d):
    """The commit a session names: a README 'commit' line, else a 40-hex in it, else the -<hex> suffix."""
    readme = os.path.join(d, 'README.md')
    if os.path.isfile(readme):
        txt = read(readme)
        m = COMMIT_LINE.search(txt)
        if m:
            return m.group(1).lower()
        m = HEX40.search(txt)
        if m:
            return m.group(0)
    m = re.search(r'-([0-9a-f]{8})-rows', os.path.basename(os.path.normpath(d)))
    return m.group(1) if m else None


def expand(c):
    """A short hash as the full one when this checkout knows it."""
    if c and len(c) < 40:
        r = subprocess.run(['git', '-C', HERE, 'rev-parse', '--verify', '--quiet', c + '^{commit}'],
                           capture_output=True, text=True)
        if r.returncode == 0 and re.fullmatch(r'[0-9a-f]{40}', r.stdout.strip()):
            return r.stdout.strip()
    return c


def run(cmd):
    p = subprocess.run([sys.executable, '-I'] + cmd, capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def read_tallies(dirs):
    """{label: (pass, fail, ns, other)} from the <LABEL>.tally.txt TOTAL rows; and the labels without a TOTAL row."""
    res, bad = {}, []
    for d in dirs:
        if not os.path.isdir(d):
            continue
        for fn in sorted(os.listdir(d)):
            if fn.endswith('.tally.txt'):
                lab = fn[:-len('.tally.txt')]
                m = TOTAL_ROW.search(read(os.path.join(d, fn)))
                if m:
                    res[lab] = tuple(int(x) for x in m.groups())
                else:
                    bad.append(lab)
    return res, bad


def recoveries(dirs):
    tot, seen = 0, False
    for d in dirs:
        r = os.path.join(d, 'README.md')
        if os.path.isfile(r):
            for m in RECOVERIES.finditer(read(r)):
                tot, seen = tot + int(m.group(1)), True
                break
    return tot if seen else None


# (file under the repo root, regex of the current Pi 4 text, what it is). Matched in the file, not by line number.
ANCHORS = [
    ('tools/cts/RESULTS.md', re.compile(r'^\*\*Pi 4, first nine images\*\*.*?(?=\n\n)', re.M | re.S),
     'RESULTS.md Pi 4 ES 3.0 entry'),
    ('tools/cts/STATUS.md', re.compile(r'Pi 4 は 91 画像のうち 9 画像だけ[^。\n]*。'), 'STATUS.md Pi 4 sentence'),
    ('ROADMAP.md', re.compile(r'^- Raspberry Pi 4: 9 of its 91 images[^\n]*(?:\n  [^\n]*)*', re.M),
     'ROADMAP.md stage 4 Pi 4 bullet'),
]


def find_anchors(root, repl):
    """[(label, path, text, match-or-None, problem-or-None, replacement)] for the three docs under root."""
    res = []
    for (rel_path, rx, label), new in zip(ANCHORS, repl):
        try:
            text = open(os.path.join(root, rel_path), encoding='utf-8', newline='').read()
        except OSError as e:
            res.append((label, rel_path, None, None, 'cannot read %s: %s' % (rel_path, e.strerror), new))
            continue
        ms = list(rx.finditer(text))
        if len(ms) != 1:
            res.append((label, rel_path, text, None, 'anchor %s in %s: %s' % (
                'missing' if not ms else 'matches %d times' % len(ms), rel_path, rx.pattern), new))
        else:
            res.append((label, rel_path, text, ms[0], None, new))
    return res


def report(args, out):
    dirs = args.dirs
    problems = []
    commits = []
    for d in dirs:
        c = commit_of(d) if os.path.isdir(d) else None
        if c and not any(c.startswith(x) or x.startswith(c) for x in commits):
            commits.append(c)
    if args.commit:
        sha = args.commit.lower()
    else:
        sha = expand(commits[0]) if commits else None
    if len(commits) > 1:
        problems.append('the session directories name different commits: %s' % ', '.join(c[:8] for c in commits))

    labels = labels_of(args.table)
    mustpass = mustpass_count(args.mustpass)

    ok = True
    aborted = []
    if sha and re.fullmatch(r'[0-9a-f]{40}', sha):
        rc, o, e = run([CHECK, '--commit', sha, '--tree', args.table] + dirs)
        for l in o.splitlines():
            if l.startswith('FAIL:'):
                p = l[5:].strip()
                problems.append(p)
                m = re.match(r'aborted, no complete deqp summary \(\d+\): (.*)', p)
                if m:
                    aborted += m.group(1).split()
        if rc not in (0, 1):
            problems.append('check-pi4-gles3-log.py could not run: %s' % (e.strip() or 'exit %d' % rc))
        elif rc == 1 and not any(l.startswith('FAIL:') for l in o.splitlines()):
            problems.append('check-pi4-gles3-log.py failed: %s' % (e.strip() or o.strip()))
    else:
        problems.append('no full 40-digit commit sha known (README.md "commit" line, or --commit)')

    tallies, no_total = read_tallies(dirs)
    for lab in no_total:
        problems.append('%s.tally.txt has no TOTAL row' % lab)

    none_by = {}
    rc, o, e = run([NOTPASS, '--pi4', '--mustpass', args.mustpass, '--tree', args.table] + dirs)
    if rc != 0:
        problems.append('gles3-notpass.py --pi4 could not run: %s' % (e.strip() or o.strip() or 'exit %d' % rc))
    else:
        for l in o.splitlines():
            m = re.match(r'(\S+)\.console\.txt:NoResult ', l)
            if m:
                none_by[m.group(1)] = none_by.get(m.group(1), 0) + 1

    ok = not problems
    npass = sum(t[0] for t in tallies.values())
    nfail = sum(t[1] for t in tallies.values())
    nns = sum(t[2] for t in tallies.values())
    nother = sum(t[3] for t in tallies.values())
    nnone = sum(none_by.values())
    in_run = npass + nfail + nns + nother + nnone
    run_labels = [l for l in labels if l in tallies]
    img_run, img_all = len(run_labels), len(labels)
    not_run = mustpass - in_run
    aborted = [l for l in labels if l in aborted]
    rec = recoveries(dirs)
    rec_txt = 'GPU recoveries: %s' % ('%d' % rec if rec is not None else 'not stated in the READMEs')
    full = sha or 'an unnamed commit'
    short = full[:8]
    dirs_txt = ', '.join('`%s/`' % rel(d) for d in dirs)
    partial = '' if ok else 'PARTIAL: '
    five = tuple(n(x) for x in (npass, nfail, nns, nother, nnone))
    ab_txt = ('%d aborted (%s)' % (len(aborted), ' '.join(aborted))) if aborted else 'none aborted'
    ab_ja = ('%d 画像が途中で止まった（%s）' % (len(aborted), ' '.join(aborted))) if aborted else '途中で止まった画像なし'
    imgs = '%d of %d images' % (img_run, img_all)

    P = lambda s='': print(scrub(s, dirs), file=out)
    P('=== pi4-gles3-report: Pi 4 dEQP-GLES3 drafts (%s) ===' % args.date)
    if not ok:
        P('*** PARTIAL: the completeness check failed; these drafts are not final ***')
        for p in problems:
            P('  problem: %s' % p)
    P()
    P('--- RESULTS.md (ES 3.0 section, replaces the Pi 4 entry "**Pi 4, first nine images**") ---')
    results_txt = (
        '%s**Pi 4, whole tree** on `%s` (origin/main, tryboot, console through the UART bridge in `$BOARD_UART`;\n'
        '`tools/cts/pi4-gles3-tree.txt`, %d images; recipe `docs/es3/board-recipe-pi4.md`), %s: %s run, %s '
        'mustpass cases of %s in them, %s pass / %s fail / %s NotSupported / %s other / %s without a result, %s; %s. '
        'Records: %s. Every case that is not Pass: `docs/es3/pi4-gles3-fails.tsv`.' % (
            partial, full, img_all, args.date, imgs, n(in_run), n(mustpass), *five, ab_txt, rec_txt, dirs_txt))
    if not_run:
        results_txt += ' %s mustpass cases are in images not run.' % n(not_run)
    for p in problems:
        results_txt += '\n- PARTIAL, not final: %s.' % p
    P(results_txt)
    P()
    P('--- STATUS.md (Japanese, current state only) ---')
    status_txt = ('Pi 4 は%s全 %d 画像のうち %d 画像を %s（%s）で実行し（全ツリー）、これらの画像の必須 %s 件中 合格 %s / 失敗 %s / 未対応 %s / その他 %s / 結果なし %s、%s、'
                  '%s（失敗の一覧は docs/es3/pi4-gles3-fails.tsv）。' % (
                      '（途中、記録が不完全）' if not ok else '', img_all, img_run, short, args.date, n(in_run), *five, ab_ja,
                      'GPU 復旧 %d' % rec if rec is not None else 'GPU 復旧は README に記載なし'))
    P(status_txt)
    P()
    P('--- ROADMAP.md (stage 4, Raspberry Pi 4 line) ---')
    roadmap_txt = ('- Raspberry Pi 4: %s%s of the dEQP-GLES3 tree at `%s` (%s): %s mustpass cases, %s Pass / %s Fail / '
                   '%s NotSupported / %s other / %s without a result, %s (#2).' % (
                       partial, imgs, short, args.date, n(in_run), *five, ab_txt))
    P(roadmap_txt)
    P()
    P('--- GitHub issue #2 comment ---')
    P('%s**Pi 4 dEQP-GLES3, %s**' % (partial, args.date))
    P()
    P('- Commit: `%s`.' % full)
    P('- Images run: %s (table `tools/cts/pi4-gles3-tree.txt`); %s.' % (imgs, ab_txt))
    P('- Records: %s.' % dirs_txt)
    P('- %s.' % rec_txt)
    P('- Totals (mustpass %s; %s in the images run): %s Pass / %s Fail / %s NotSupported / %s other / %s without a result.'
      % (n(mustpass), n(in_run), *five))
    left = [p for p in problems]
    left.append('triage of the not-pass groups into issues (`docs/es3/pi4-gles3-fails.tsv`)')
    P('- Remains: %s.' % '; '.join(left))

    repl = [results_txt, status_txt, roadmap_txt]
    root = args.apply or args.root
    anchors = find_anchors(root, repl)
    P()
    P('--- anchors (the current text each draft replaces, matched in the file; tree %s) ---'
      % ('given by --apply' if args.apply else 'this checkout' if root == ROOT else 'given by --root'))
    for label, path, text, m, prob, new in anchors:
        if prob:
            P('%s: %s' % (label, prob))
            continue
        P('%s: %s line %d, replaces' % (label, path, text.count('\n', 0, m.start()) + 1))
        for l in m.group(0).split('\n'):
            P('  - %s' % l)
        P('  with the %s draft above' % path.rsplit('/', 1)[-1])
    if args.apply:
        bad = [a[4] for a in anchors if a[4]]
        if not ok:
            bad.append('the drafts are PARTIAL')
        if bad:
            P('--apply: refused, nothing written: %s' % '; '.join(bad))
            return 1
        for label, path, text, m, prob, new in anchors:
            with open(os.path.join(args.apply, path), 'w', encoding='utf-8', newline='') as f:
                f.write(text[:m.start()] + new + text[m.end():])
        P('--apply: rewrote %s' % ', '.join(a[1] for a in anchors))
    return 0 if ok else 1


def self_test():
    fails = []
    cases = []

    def expect(name, cond, detail=''):
        cases.append(name)
        print('%s %s%s' % ('ok  ' if cond else 'FAIL', name, '' if cond else ' ' + detail))
        if not cond:
            fails.append(name)

    sha = 'a' * 40
    me = os.path.abspath(__file__)

    def rep(td, *extra):
        p = subprocess.run([sys.executable, '-I', me, '--table', os.path.join(td, 'tree.txt'),
                            '--mustpass', os.path.join(td, 'mustpass.txt'), '--date', '2026-11-08',
                            '--root', os.path.join(td, 'docs')] + list(extra),
                           capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr

    with tempfile.TemporaryDirectory() as td:
        with open(os.path.join(td, 'tree.txt'), 'w') as f:
            f.write('# image A1: a\ndEQP-GLES3.a.*\n# image B1: b\ndEQP-GLES3.b.*\n# image C1: c\ndEQP-GLES3.c.*\n')
        with open(os.path.join(td, 'mustpass.txt'), 'w') as f:
            f.write('\n'.join(['dEQP-GLES3.a.pass', 'dEQP-GLES3.a.fail', 'dEQP-GLES3.b.zeta', 'dEQP-GLES3.b.alpha',
                               'dEQP-GLES3.c.unsupported', 'dEQP-GLES3.c.pass']) + '\n')

        def session(name, labels, consoles, readme_extra=''):
            d = os.path.join(td, name)
            os.makedirs(d)
            with open(os.path.join(d, 'README.md'), 'w') as f:
                f.write('# synthetic\n\ncommit: %s\n%s' % (sha, readme_extra))
            for lab in labels:
                with open(os.path.join(d, lab + '.console.txt'), 'w') as f:
                    f.write(consoles[lab])
                tot = {'A1': (1, 1, 0, 0), 'B1': (0, 1, 0, 1), 'C1': (1, 0, 1, 0)}[lab]
                with open(os.path.join(d, lab + '.tally.txt'), 'w') as f:
                    f.write('%s.console.txt: deqp: done: complete=1 | deqp: summary: 2 cases begun, 2 with a result, 0 without\n'
                            'group pass fail n/s other\nTOTAL %d %d %d %d\n' % ((lab,) + tot))
            return d

        con = {
            'A1': 'deqp: Pass dEQP-GLES3.a.pass\ndeqp: Fail dEQP-GLES3.a.fail\ndeqp: summary: 2 cases begun, 2 with a result, 0 without\n',
            'B1': 'deqp: Fail dEQP-GLES3.b.zeta\ndeqp: Crash dEQP-GLES3.b.alpha\ndeqp: summary: 2 cases begun, 2 with a result, 0 without\n',
            'C1': 'deqp: NotSupported dEQP-GLES3.c.unsupported\ndeqp: Pass dEQP-GLES3.c.pass\ndeqp: summary: 2 cases begun, 2 with a result, 0 without\n',
        }
        s1 = session('s1-rows0-1', ['A1', 'B1'], con, '2 GPU recoveries\n')
        s2 = session('s2-rows2-2', ['C1'], con)

        docs = os.path.join(td, 'docs')
        os.makedirs(os.path.join(docs, 'tools', 'cts'))
        pi4_results = '**Pi 4, first nine images** on `925dbca9` (old\ntext): 9 of 91 images.'
        pi4_status = 'Pi 4 は 91 画像のうち 9 画像だけ（925dbca9）で、4,917 件中 3,943 合格（失敗の一覧は x）。'
        pi4_road = '- Raspberry Pi 4: 9 of its 91 images at `925dbca9`: 4,917\n  cases.\n  more.\n- i.MX8MP: next'
        files = {
            'tools/cts/RESULTS.md': 'head\n\n%s\n\n**next entry**\n' % pi4_results,
            'tools/cts/STATUS.md': 'x ES 3.0。 %s 残り。\n' % pi4_status,
            'ROADMAP.md': '## stage 4\n%s\n' % pi4_road,
        }

        def write_docs(over=None):
            for p, t in files.items():
                with open(os.path.join(docs, p), 'w', encoding='utf-8', newline='') as f:
                    f.write((over or {}).get(p, t))

        write_docs()
        rc, out = rep(td, s1, s2)
        expect('complete record: exit 0', rc == 0, out)
        expect('totals: 6 mustpass, 2 pass / 2 fail / 1 NotSupported / 1 other / 0 without',
               'mustpass 6' in out and '2 Pass / 2 Fail / 1 NotSupported / 1 other / 0 without a result' in out, out)
        expect('commit, images run, recoveries and the four blocks named',
               sha in out and '3 of 3 images' in out and 'GPU recoveries: 2' in out and 'PARTIAL' not in out
               and all(h in out for h in ('--- RESULTS.md', '--- STATUS.md', '--- ROADMAP.md', '--- GitHub issue #2', '--- anchors')), out)
        expect('anchors found with their line numbers',
               'RESULTS.md Pi 4 ES 3.0 entry: tools/cts/RESULTS.md line 3' in out
               and 'ROADMAP.md stage 4 Pi 4 bullet: ROADMAP.md line 2' in out, out)
        expect('no absolute path in the output', td not in out and '/home/' not in out, out)

        os.remove(os.path.join(s2, 'C1.tally.txt'))
        rc, out = rep(td, s1, s2)
        expect('missing tally: PARTIAL, exit 1, names the label',
               rc == 1 and 'PARTIAL' in out and 'missing (1): C1' in out and '--- GitHub issue #2' in out, out)
        rc, out = rep(td, '--apply', docs, s1, s2)
        expect('--apply on PARTIAL drafts is refused', rc == 1 and 'refused' in out, out)
        with open(os.path.join(s2, 'C1.tally.txt'), 'w') as f:
            f.write('C1.console.txt: deqp: done: complete=1 | deqp: summary: 2 cases begun, 2 with a result, 0 without\n'
                    'TOTAL 1 0 1 0\n')

        rc, out = rep(td, '--apply', docs, s1, s2)
        new = {p: open(os.path.join(docs, p), encoding='utf-8').read() for p in files}
        expect('--apply: exit 0, one replacement in each of the three docs',
               rc == 0 and all('925dbca9' not in t for t in new.values())
               and new['tools/cts/RESULTS.md'].count('**Pi 4, whole tree**') == 1
               and new['tools/cts/RESULTS.md'].endswith('\n\n**next entry**\n')
               and new['tools/cts/STATUS.md'].count('Pi 4 は全 3 画像のうち 3 画像') == 1
               and new['tools/cts/STATUS.md'].endswith('。 残り。\n')
               and new['ROADMAP.md'].count('- Raspberry Pi 4: 3 of 3 images') == 1
               and new['ROADMAP.md'].endswith('(#2).\n- i.MX8MP: next\n'), out + repr(new))

        doubled = dict(files)
        doubled['ROADMAP.md'] = files['ROADMAP.md'] + pi4_road + '\n'
        write_docs(doubled)
        rc, out = rep(td, '--apply', docs, s1, s2)
        same = all(open(os.path.join(docs, p), encoding='utf-8').read() == doubled[p] for p in files)
        expect('--apply: a doubled anchor is refused and nothing is written',
               rc == 1 and 'matches 2 times' in out and same, out)

    print('pi4-gles3-report self-test: %d of %d cases ok' % (len(cases) - len(fails), len(cases)))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--table', default=os.path.join(HERE, 'pi4-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--date', default=datetime.date.today().isoformat(), help='date of the drafts (default: today)')
    ap.add_argument('--commit', help='full sha of the record (default: from the session READMEs)')
    ap.add_argument('--root', default=ROOT, help='tree whose docs the anchors are read from')
    ap.add_argument('--apply', metavar='ROOT', help='rewrite the anchored texts in the tree ROOT (all or nothing)')
    ap.add_argument('--self-test', action='store_true')
    ap.add_argument('dirs', nargs='*', help='session directories (tools/cts/log/*-pi4-gles3-*/)')
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.dirs:
        ap.error('at least one session directory is required')
    return report(args, sys.stdout)


if __name__ == '__main__':
    sys.exit(main())
