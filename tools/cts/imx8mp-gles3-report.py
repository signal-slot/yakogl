#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft the i.MX8MP dEQP-GLES3 results lines from the tree slice records.

Runs imx8mp-gles3-merge.py --check as a subprocess, takes the per-entry totals from its Merge class and prints, to stdout, four headed
blocks: the RESULTS.md entry, the STATUS.md line (Japanese), the ROADMAP stage 4 line and the text of
the status comment for GitHub issue #24, each in the layout of its target file (RESULTS.md: the board paragraph
and five-column table of the ES 3.0 section; STATUS.md: the Japanese summary sentence of the ES 3.0
bullet; ROADMAP.md: a stage-4 board bullet wrapped at 80 columns). It edits no file. When --check fails (missing entries, reset
verdicts, several commits) the drafts are still printed, marked PARTIAL, the problems are named, and the
exit status is 1.

The drafts count mustpass cases only, with merge's record rule (one record per entry: the first input whose
row is real; an entry in two inputs is a --check problem). A record can begin cases outside the mustpass
list (an entry's wildcards select them too) or begin a case twice; those are left out of every number, so
"begun" is capped per entry at its mustpass cases and the five-number row (Pass, Fail, NotSupported,
Other, No result) sums to the mustpass count (see mustpass_counts). They are named in a note, and they
never make the drafts PARTIAL: a campaign is complete when every entry has a record with a clean verdict
(merge --check), whatever its records begin beyond the mustpass list.

  imx8mp-gles3-report.py [--table T] [--mustpass M] [--date YYYY-MM-DD] [--root R] [--apply R] <slice dir>...

After the drafts a block "anchors" names, for RESULTS.md, STATUS.md and ROADMAP.md of the tree R (default:
this checkout), the current text each draft replaces, found by matching in the file, with its line number
for reference. --apply R rewrites exactly those texts in R. It writes nothing and exits 1 when an anchor is
missing or matches more than once (or when the drafts are PARTIAL).

The board: when every slice is an Armadillo-X2 slice (merge's board_of), each of the four drafts says "measured on
the Armadillo-X2 (i.MX 8M Plus, GC7000UL)" (STATUS.md: "Armadillo-X2（i.MX 8M Plus、GC7000UL）で"); a mix of X2 and
Verdin slices names both with the entries each measured (read from the records: the entries whose record lies in
a slice of that board, e.g. "entries 0-50 on the Verdin iMX8M Plus, 13-15 and 51-132 on the Armadillo-X2"), and the
RESULTS.md and #24 drafts list each slice's board. An X2 slice is a full record, never PARTIAL for being X2; PARTIAL
stays for real gaps (entries without a record). Verdin only: as before.
"""
import argparse
import datetime
import importlib.util
import os
import re
import subprocess
import sys
import textwrap

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


X2_LONG = 'Armadillo-X2 (i.MX 8M Plus, GC7000UL)'


def board_of(d):
    """'x2' or 'verdin': the board a slice was measured on (imx8mp-gles3-merge.py's board_of)."""
    return load_merge().board_of(d)


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


_MERGE_MOD = []


def load_merge():
    if _MERGE_MOD:
        return _MERGE_MOD[0]
    spec = importlib.util.spec_from_file_location('imx8mp_gles3_merge', MERGE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    _MERGE_MOD.append(mod)
    return mod


def mustpass_counts(c, rec):
    """[mustpass, begun, pass, fail, ns, other, none] of one entry over its mustpass cases only, and the
    number of record cases left out.

    Rule (the drafts count mustpass cases only): a record's pass + fail + n.s. + other above the entry's
    mustpass cases m are cases outside the mustpass list (a wildcard of the entry selects them too) or
    cases begun twice. They are taken off so that the five numbers (pass, fail, n.s., other, none) sum
    to m: first the Fail names the record lists outside the mustpass list (merge's `outside`: neither a
    mustpass case nor an unresolved CRC), then Pass, then NotSupported, then Other. A record keeps only
    counts for the other cases (a Fail it does not name may be either), so which of them were out of the
    list cannot be told; Pass first because a wildcard's extra cases are mostly cases of a passing group. begun is
    capped at m. merge's none (m minus the four, at least 0) is then exactly m minus the four."""
    m, b, p, f, s, o, _ = c
    x = max(0, p + f + s + o - m)
    out = x
    k = min(x, f, rec.get('outside', 0))
    f, x = f - k, x - k
    k = min(x, p); p, x = p - k, x - k
    k = min(x, s); s, x = s - k, x - k
    k = min(x, o); o, x = o - k, x - k
    return [m, min(m, b), p, f, s, o, m - p - f - s - o], out


def totals(args):
    """Per-entry rows [i, mustpass, begun, pass, fail, ns, other, none] over mustpass cases only (no-record
    entries: begun 0, none = mustpass), their sum, the raw record sum (merge --totals' total row), the
    no-record tuple (count, entries, span, mustpass cases) and the merge module's problems."""
    mod = load_merge()
    table = mod.load_table(args.table)
    per = mod.entry_cases(table, args.mustpass)
    mg = mod.Merge(table, per, args.dirs)
    rows, raw = [], [0] * 7
    for i in range(len(table)):
        c = mg.counts(i)
        if c is None:
            k = len(per[i])
            rows.append([i, k, 0, 0, 0, 0, 0, k])
            raw = [a + b for a, b in zip(raw, (k, 0, 0, 0, 0, 0, k))]
            continue
        raw = [a + b for a, b in zip(raw, c)]
        rows.append([i] + mustpass_counts(c, mg.rec[i])[0])
    tot = [sum(r[k] for r in rows) for k in range(1, 8)]
    missing = [i for i in range(len(table)) if mg.counts(i) is None]
    miss = (len(missing), len(table), mod.span(missing).replace(',', ', '), sum(len(per[i]) for i in missing))
    # The entries each board measured: the board of the slice that holds the entry's record (a no-record
    # entry belongs to no board).
    boards = {}
    for i in range(len(table)):
        r = mg.rec.get(i)
        if r is not None and r.get('tally') is not None and r.get('dir') is not None:
            boards.setdefault(board_of(r['dir']), []).append(i)
    return rows, tot, raw, miss, {b: mod.span(e).replace(',', ', ') for b, e in boards.items()}


# A README names an event on a line that begins with its marker and names the entries: "Reset: e03 ended on WDOG1",
# "- WDOG1 e03 ...", "Hang: e50 ...", "Watchdog reset e03, e04 ...". Prose that merely mentions the watchdog (the
# slice READMEs explain WDOG1 at length, "Resets and reboots outside the rows") is not an event.
EVENT_RX = re.compile(r'^\s*(?:[-*]\s+)?(?:\*\*)?(?P<k>(?:WDOG\d*|watchdog)(?:\s+resets?)?|resets?|hangs?|NO FINISH)\b(?:\*\*)?\s*[:\-,]?\s',
                      re.I)
ENTRY_RX = re.compile(r'\be(\d+)\b|\bentr(?:y|ies)\s+(\d+)', re.I)


def fault_events(dirs, rows):
    """(resets, hangs, shortfall entries) the slice records carry. The README lines that begin with a marker (WDOG1, watchdog reset, reset, NO FINISH: a reset; hang: a hang) and
    name an entry (`e12`, `entry 12`), as the slice READMEs write them by hand; one event per line. Prose and the
    `board.sh <board> e<NN> ... resets=` verdict lines are not events. And the shortfall: a table entry with a
    record that begun fewer cases than its mustpass count (merge's stderr WARNING ... (shortfall)). A
    shortfall entry no README line names counts as one hang or reset of unknown kind (a hang ends the entry
    at the last case begun, a WDOG1 reset likewise: the records do not tell them apart)."""
    resets, hangs, named = [], [], set()
    for d in dirs:
        try:
            names = sorted(f for f in os.listdir(d) if f.lower().startswith('readme'))
        except OSError:
            continue
        for name in names:
            for l in open(os.path.join(d, name), errors='replace').read().splitlines():
                m = EVENT_RX.match(l)
                es = {int(x) for t in ENTRY_RX.findall(l) for x in t if x}
                if not m or not es:
                    continue
                (hangs if m.group('k').lower().startswith('hang') else resets).append(
                    (os.path.basename(os.path.normpath(d)), sorted(es)))
                named |= es
    short = [r[0] for r in rows if r[2] < r[1] and (r[2] or sum(r[3:7]))]   # no-record entries (all zero) are not shortfalls
    short = sorted(set(short))
    return resets, hangs, short, sorted(set(short) - named)


def mod_span(entries):
    return load_merge().span(entries).replace(',', ', ')


def rel(d):
    """A slice directory as the texts name it: below the repository root, else (a copy elsewhere) as
    tools/cts/log/<name>, never an absolute path of this machine."""
    root = os.path.dirname(os.path.dirname(HERE))
    r = os.path.relpath(os.path.abspath(d), root)
    if r.startswith('..'):
        name = os.path.basename(os.path.normpath(d))
        return ('tools/cts/log/' + name) if re.search(r'-imx8mp-gles3-.*-rows', name) else name
    return r.rstrip('/')


def slice_order(d):
    """Sort key of a slice directory: its first table row as a number (rows14-16 before rows108-122), then the name."""
    name = os.path.basename(os.path.normpath(d))
    m = re.search(r'-rows(\d+)', name)
    return (int(m.group(1)) if m else 10 ** 9, name)


# (file under the repo root, regex of the current text, what it is). Matched in the file, not by line number.
# The i.MX8MP result goes where its neighbours' results are: RESULTS.md's ES 3.0 board paragraphs, the
# Japanese ES 3.0 bullet of STATUS.md (after the Pi 4 sentence), ROADMAP.md's stage-4 "On the boards"
# bullets; the STATUS.md next-steps sentence then drops i.MX8MP.
STATUS_NEXT = 'Pi 4 と i.MX8MP の GLES3 全ツリー。'
STATUS_NEXT_DONE = 'Pi 4 の GLES3 全ツリー。'
ANCHORS = [
    ('tools/cts/RESULTS.md', re.compile(r'^\*\*i\.MX8MP\*\*: not run on the board[^\n]*$', re.M), 'RESULTS.md i.MX8MP line'),
    ('tools/cts/STATUS.md', re.compile(r'i\.MX8MP はまだボードで走らせていない。'), 'STATUS.md ES 3.0 bullet, i.MX8MP sentence'),
    ('tools/cts/STATUS.md', re.compile(re.escape(STATUS_NEXT)), 'STATUS.md next-steps sentence'),
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


def and_join(span):
    """'13-15, 51-132' as '13-15 and 51-132', '1, 5, 9' as '1, 5 and 9'."""
    parts = span.split(', ')
    return parts[0] if len(parts) == 1 else ', '.join(parts[:-1]) + ' and ' + parts[-1]


def n(x):
    return f'{x:,}'


def wrap(text, width=80, indent=''):
    """Markdown prose wrapped as the target files are (no break inside a path or a hyphenated word)."""
    return textwrap.fill(text, width=width, subsequent_indent=indent, break_long_words=False,
                         break_on_hyphens=False)


def nice(problem):
    """A merge problem with the drafts' number format (42,810; spans '13-15, 51-132')."""
    m = re.match(r'no record: (\d+) of (\d+) entries \(([^)]*)\), (\d+) mustpass cases$', problem)
    if m:
        return 'no record: %s of %s entries (%s), %s mustpass cases' % (
            m.group(1), m.group(2), m.group(3).replace(',', ', '), n(int(m.group(4))))
    return problem


def names_block(args):
    """--names: per slice and entry, the record's verdict counts (Pass, Fail, NotSupported, other, no result) and the
    names the record lists as not Pass. results.tsv names failing cases only; a count above the names listed
    (a NotSupported, or Fail beyond the naming cap) is count-only, and the line says so. Same records as merge."""
    mod = load_merge()
    table = mod.load_table(args.table)
    mg = mod.Merge(table, mod.entry_cases(table, args.mustpass), args.dirs)
    print('--- --names: per slice and entry, record verdict counts and the not-pass names the record lists ---')
    for d in args.dirs:
        print('%s:' % rel(d))
        any_row = False
        for i in sorted(mg.rec):
            r = mg.rec[i]
            if r.get('dir') != d or r.get('tally') is None:
                continue
            any_row = True
            b, p, f, s, o, nr = r['tally']
            listed = list(r['names']) + list(r.get('outnames', []))
            notpass = f + s + o
            print('  e%d: begun %d, Pass %d, Fail %d, NotSupported %d, other %d, no result %d; not-pass %d, named %d%s' % (
                i, b, p, f, s, o, nr, notpass, len(listed),
                '' if len(listed) >= notpass else ', %d count-only' % (notpass - len(listed))))
            for nm in listed:
                print('    %s' % nm)
        if not any_row:
            print('  (no entry record)')


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--date', default=datetime.date.today().isoformat(), help='date of the drafts (default: today)')
    ap.add_argument('--root', default=os.path.dirname(os.path.dirname(HERE)), help='tree whose docs the anchors are read from')
    ap.add_argument('--apply', metavar='ROOT', help='rewrite the anchored lines in the tree ROOT (all or nothing)')
    ap.add_argument('--names', action='store_true',
                    help='after the drafts, print per slice and entry the verdict counts and the not-pass names the record lists')
    ap.add_argument('dirs', nargs='+', help='slice record directories (tools/cts/log/*-imx8mp-gles3-*/)')
    args = ap.parse_args()

    rc_c, out_c, err_c = merge('--check', args)
    problems = [nice(l[5:].strip()) for l in out_c.splitlines() if l.startswith('FAIL:')]
    if rc_c not in (0, 1):
        problems.append('merge --check could not run: %s' % (err_c.strip() or out_c.strip() or 'exit %d' % rc_c))
    try:
        rows, tot, raw, miss, board_span = totals(args)
    except (OSError, ValueError) as e:
        problems.append('totals could not be read: %s' % e)
        rows, tot, raw, miss, board_span = [], [0] * 7, [0] * 7, (0, 0, 'none', 0), {}
    mp, begun, npass, nfail, nns, nother, nnone = tot
    # Completeness is judged on mustpass cases only: merge --check (every entry has a record with a clean
    # verdict, one record per entry, one commit). Cases the records begin outside the mustpass list never
    # make the drafts PARTIAL; mustpass_counts leaves them out of every number, so the row sums to mp.
    out_of_list = sum(raw[2:6]) - sum(tot[2:6])
    if npass + nfail + nns + nother + nnone != mp:
        problems.append('the five numbers sum to %s, not to the %s mustpass cases' % (
            n(npass + nfail + nns + nother + nnone), n(mp)))
    ok = rc_c == 0 and not problems

    commits = []
    for d in args.dirs:
        c = commit_of(d) if os.path.isdir(d) else None
        if c and not any(c[:8] == x[:8] for x in commits):
            commits.append(c)
    full = [expand(c) for c in commits]
    short_txt = ' and '.join('`%s`' % c[:8] for c in full) or 'an unnamed commit'
    full_txt = ' and '.join('`%s`' % c for c in full) or 'an unnamed commit'
    short_ja = ' と '.join(c[:8] for c in full) or '名前のないコミット'
    results_commit = ' and '.join('`%s` (`%s`; origin/main' % (c[:8], c) for c in full[:1])
    if len(full) > 1:
        results_commit += '; also %s' % ' and '.join('`%s`' % c for c in full[1:])
    results_commit = results_commit or 'an unnamed commit (origin/main'

    n_entries = miss[1] or 133
    run_n = n_entries - miss[0]
    if miss[0]:
        entries_txt = '%d of %d entries run (no record: %s)' % (run_n, n_entries, miss[2])
    else:
        entries_txt = '%d of %d entries run' % (run_n, n_entries)
    shown = sorted(args.dirs, key=slice_order)   # display only: merge keeps the order given (the first real row of an entry claims it)
    dirs_txt = ', '.join('`%s/`' % rel(d) for d in shown)
    partial = '' if ok else 'PARTIAL: '
    bad_verdict = any('verdict' in p for p in problems)
    ev_resets, ev_hangs, ev_short, ev_unnamed = fault_events(args.dirs, rows)
    n_res, n_hang = len(ev_resets), len(ev_hangs) + len(ev_unnamed)
    n_ev = n_res + n_hang
    ev_parts = []
    if ev_resets:
        ev_parts.append('%d reset line%s' % (n_res, '' if n_res == 1 else 's'))
    if ev_hangs:
        ev_parts.append('%d hang line%s' % (len(ev_hangs), '' if len(ev_hangs) == 1 else 's'))
    if ev_unnamed:
        ev_parts.append('%d shortfall entr%s no README line names (%s)' % (
            len(ev_unnamed), 'y' if len(ev_unnamed) == 1 else 'ies', mod_span(ev_unnamed)))
    if bad_verdict:
        reset_txt = 'reset verdicts NOT CLEAN, see the problems'
    elif n_ev:
        reset_txt = ('%d resets or hangs counted from the records (%s; entries begun short: %s); every entry run '
                     'is `verdict=done`' % (n_ev, ', '.join(ev_parts), mod_span(ev_short) if ev_short else 'none'))
    else:
        reset_txt = '0 resets (every entry run is `verdict=done`, `resets=absent`)'
    ev_roadmap = '%d resets or hangs' % n_ev if n_ev else '0 resets'
    ev_ja = 'リセットまたはハング %d 件' % n_ev if n_ev else 'リセット 0'
    five = tuple(n(x) for x in (npass, nfail, nns, nother, nnone))

    # The board each slice was measured on. Verdin only: every draft is as before. All Armadillo-X2: each draft
    # says so (owner, 2026-11-27: the Verdin is off the farm, the X2 has the same i.MX 8M Plus and GC7000UL).
    boards = {d: board_of(d) for d in args.dirs}
    all_x2 = bool(boards) and set(boards.values()) == {'x2'}
    mixed = not all_x2 and 'x2' in boards.values()
    board_en = (', measured on the %s' % X2_LONG if all_x2 else
                ', measured on the Verdin iMX8M Plus and the Armadillo-X2' if mixed else '')
    board_ja = (' Armadillo-X2（i.MX 8M Plus、GC7000UL）で' if all_x2 else
                ' Verdin iMX8M Plus と Armadillo-X2 で' if mixed else '')
    if mixed and len(board_span) == 2:
        # Both boards hold records: name each board with the entries it measured, read from the records
        # (the slice that holds an entry's record, and that slice's README for the board), never fixed.
        order = sorted(board_span, key=lambda b: int(re.match(r'\d+', board_span[b]).group(0)))
        name = {'x2': 'the Armadillo-X2', 'verdin': 'the Verdin iMX8M Plus'}
        ja = {'x2': 'Armadillo-X2', 'verdin': 'Verdin iMX8M Plus'}
        ranges = ['%s on %s' % (and_join(board_span[b]), name[b]) for b in order]
        board_en = ', measured on entries ' + ', '.join(ranges)
        board_ja = ' ' + 'と '.join('%s（エントリ %s）' % (ja[b], board_span[b].replace(', ', '、')) for b in order) + 'で'
    print('=== imx8mp-gles3-report: i.MX8MP dEQP-GLES3 drafts (%s) ===' % args.date)
    if not ok:
        reason = 'merge --check failed (exit %d)' % rc_c if rc_c else 'the merge totals are inconsistent'
        print('*** PARTIAL: %s; these drafts are not final ***' % reason)
        for p in problems:
            print('  problem: %s' % p)
    if out_of_list:
        print('note: mustpass cases only: %s results the records give beyond their entries\' mustpass cases '
              '(cases outside the mustpass list, cases begun twice; records begin %s cases in all) are not '
              'counted' % (n(out_of_list), n(raw[1])))
    print()
    print('--- RESULTS.md (ES 3.0 section, replaces "**i.MX8MP**: not run on the board") ---')
    # Layout of the section's board paragraphs (AM62P, Pi 4): bold board name, the commit short and full,
    # the table and recipe, the date, then the five-column table and "- " bullets wrapped at 80 columns.
    results_txt = wrap('%s**i.MX8MP, whole tree** on %s; `tools/cts/imx8mp-gles3-tree.txt`, %d entries; '
                       'recipe `docs/es3/board-recipe-imx8mp.md`)%s, %s: %s, %s mustpass cases (each counted '
                       'once, %s begun):' % (partial, results_commit, n_entries, board_en, args.date, entries_txt,
                                             n(mp), n(begun)))
    results_txt += '\n\n| Pass | Fail | NotSupported | Other | No result |\n|---:|---:|---:|---:|---:|\n'
    results_txt += '| %s | %s | %s | %s | %s |\n' % five
    bullets = ['Other is every status but Pass, Fail and NotSupported (QualityWarning, ResourceError, '
               'Crash, ...); No result is a mustpass case that never began or gave no result.']
    if out_of_list:
        bullets.append('Mustpass cases only: %s results the records give beyond their entries\' mustpass '
                       'cases (cases a wildcard selects outside the mustpass list, cases begun twice) are '
                       'not counted, so the five numbers sum to %s.' % (n(out_of_list), n(mp)))
    bullets.append('Records: %s; %s.' % (dirs_txt, reset_txt))
    if mixed:
        bullets.append('Boards: %s.' % '; '.join('`%s/`: %s' % (rel(d), X2_LONG if boards[d] == 'x2' else 'Verdin iMX8M Plus')
                                               for d in shown))
    bullets.append('Every case that is not Pass: `docs/es3/imx8mp-gles3-notpass.tsv`.')
    bullets += ['PARTIAL, not final: %s.' % p for p in problems]
    results_txt += '\n' + '\n'.join(wrap('- ' + b, indent='  ') for b in bullets)
    print(results_txt)
    print()
    print('--- STATUS.md (Japanese, ES 3.0 bullet, replaces "i.MX8MP はまだボードで走らせていない。") ---')
    # As the Pi 4 sentence before it: commit without backticks, the date in full-width parentheses,
    # "必須 N 件中 合格 a / 失敗 b / 未対応 c / その他 d / 結果なし e、リセット 0".
    if miss[0]:
        scope = 'ツリー（%d エントリ、tools/cts/imx8mp-gles3-tree.txt）のうち %d エントリ' % (n_entries, run_n)
        part_ja = '（途中、記録が不完全。記録なし %s）' % miss[2]
    else:
        scope = '全ツリー（%d エントリ、tools/cts/imx8mp-gles3-tree.txt）' % n_entries
        part_ja = '' if ok else '（途中、記録が不完全）'
    status_txt = ('i.MX8MP は%s%sを %s（%s）で実行し%s、必須 %s 件中 合格 %s / 失敗 %s / 未対応 %s / その他 %s / '
                  '結果なし %s、%s（記録は %s。合格でない一覧は docs/es3/imx8mp-gles3-notpass.tsv）。' % (
                      board_ja, scope, short_ja, args.date, part_ja, n(mp), *five,
                      'リセットの確認が未完' if bad_verdict else ev_ja, '、'.join(rel(d) for d in shown)))
    next_txt = STATUS_NEXT_DONE if ok else STATUS_NEXT
    print(status_txt)
    print('次の山: "%s" -> "%s"' % (STATUS_NEXT, next_txt))
    print()
    print('--- ROADMAP.md (stage 4, i.MX8MP line) ---')
    # As the AM62P and Pi 4 bullets: "<board>: the whole tree (...) at `sha` (date): N cases, a pass /
    # b fail / c NotSupported / d other / e without a result", wrapped at 78 columns, 2-space continuation.
    if miss[0]:
        scope = '%d of the tree\'s %d entries (`tools/cts/imx8mp-gles3-tree.txt`; no record: %s)' % (
            run_n, n_entries, miss[2])
    else:
        scope = 'the whole tree (%d entries, `tools/cts/imx8mp-gles3-tree.txt`)' % n_entries
    roadmap_txt = wrap('- i.MX8MP: %s%s%s at %s (%s): %s cases, %s pass / %s fail / %s NotSupported / %s other / '
                       '%s without a result, %s (#24).' % (
                           partial, scope, board_en, short_txt, args.date, n(mp), *five,
                           'reset verdicts not clean' if bad_verdict else ev_roadmap), width=78, indent='  ')
    print(roadmap_txt)
    print()
    print('--- GitHub issue #24 comment ---')
    print('%s**i.MX8MP dEQP-GLES3, %s**' % (partial, args.date))
    print()
    print('- Commit: %s.' % full_txt)
    if all_x2:
        print('- Board: measured on the %s.' % X2_LONG)
    elif mixed:
        print('- Boards: %s.' % '; '.join('`%s/`: %s' % (rel(d), X2_LONG if boards[d] == 'x2' else 'Verdin iMX8M Plus')
                                        for d in shown))
    print('- Entries: %s.' % entries_txt)
    print('- Records: %s.' % dirs_txt)
    print('- Resets: %s.' % reset_txt)
    print('- Totals over the %s mustpass cases (%s begun): %s Pass / %s Fail / %s NotSupported / %s other / '
          '%s without a result.' % (n(mp), n(begun), *five))
    if out_of_list:
        print('- Not counted: %s results beyond the entries\' mustpass cases (outside the mustpass list or '
              'begun twice).' % n(out_of_list))
    left = []
    if miss[0]:
        left.append('entries %s (%s mustpass cases) have no record' % (miss[2], n(miss[3])))
    left += [p for p in problems if not p.startswith('no record:')]
    left.append('triage of the not-pass groups into issues (`docs/es3/imx8mp-gles3-notpass.tsv`)')
    print('- Remains: %s.' % '; '.join(left))

    if args.names:
        print()
        names_block(args)

    repl = [results_txt, status_txt, next_txt, roadmap_txt]
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
        print('  with %s' % ('"%s"' % new if label.endswith('next-steps sentence') else
                             'the %s draft above' % path.rsplit('/', 1)[-1]))
    if args.apply:
        bad = [a[4] for a in anchors if a[4]]
        if not ok:
            bad.append('the drafts are PARTIAL')
        if bad:
            print('--apply: refused, nothing written: %s' % '; '.join(bad))
            return 1
        texts = {}
        for label, path, text, m, prob, new in anchors:
            texts.setdefault(path, []).append((m.start(), m.end(), new))
        for path, edits in texts.items():
            text = [a[2] for a in anchors if a[1] == path][0]
            for start, end, new in sorted(edits, reverse=True):   # from the end: earlier offsets stay valid
                text = text[:start] + new + text[end:]
            with open(os.path.join(args.apply, path), 'w', encoding='utf-8', newline='') as f:
                f.write(text)
        print('--apply: rewrote %s' % ', '.join(texts))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
