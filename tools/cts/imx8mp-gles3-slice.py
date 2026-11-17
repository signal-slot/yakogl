#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the README.md of one i.MX8MP dEQP-GLES3 board slice from a session's `tools/board.sh imx8mp` output.

  imx8mp-gles3-slice.py --commit HASH40 --rows AA-BB --date YYYY-MM-DD --lines FILE [--lines FILE...]
                        [--records DIR...] [--results results.tsv] [--out DIR]

--rows is the slice's table rows, counted from 1 as the directory name `<date>-imx8mp-gles3-<hash8>-rows<AA>-<BB>`
counts them: rows01-13 are entries 0-12 (`e<NN>` counts from 0). AA-AA is a one-row slice.
--lines is the captured board.sh stdout (`-` is stdin; the run's and the tally's lines may be in one file):
every `board.sh imx8mp e<NN> rc=.. verdict=.. ...` line, verbatim. Page lines (`e<NN>-p<k>`) are counted per
entry, not copied. A line seen twice verbatim counts once; two different lines for one entry, an entry outside
the range, or an entry of the range without a line are refused (exit 2), as is a commit that is not 40 hex
characters. --records names the record directories (`records/imx8mp/...`: run directories `<stamp>-e<NN>-<sha8>`
or a parent holding them); with it, every entry needs a run directory. --results is the tally's results.tsv
(its rows of the slice give the counts and the failing names the README lists per entry).

The README has the 40-hex commit, the verdict lines in a fenced block (what imx8mp-gles3-merge.py --check and
imx8mp-gles3-times.py read), a per-entry table (back_after, uptime, pages), and, with --results, the per-entry
not-pass counts and failing names. Without --out it goes to stdout; with --out it is DIR/README.md. Before it is
written or printed, imx8mp-gles3-redact.py --check must find nothing in it (exit 1 on a hit).
"""
import argparse, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r'^\s*(board\.sh\s+imx8mp\s+e(\d+)(?:-p(\d+))?\s+rc=\S+\s+verdict=\S+.*?)\s*$')


class Refuse(Exception):
    pass


def parse_rows(s):
    m = re.fullmatch(r'(\d+)-(\d+)', s)
    if not m or not 1 <= int(m.group(1)) <= int(m.group(2)):
        raise Refuse('--rows must be AA-BB with 1 <= AA <= BB (rows counted from 1), got %r' % s)
    return int(m.group(1)), int(m.group(2))


def read_lines(paths):
    rows, pages = {}, {}
    for p in paths:
        f = sys.stdin if p == '-' else open(p, errors='replace')
        for l in f:
            m = LINE.match(l)
            if not m:
                continue
            line, e, pg = m.group(1), int(m.group(2)), m.group(3)
            if pg is not None:
                pages.setdefault(e, set()).add(int(pg))
            elif e in rows and rows[e] != line:
                raise Refuse('entry e%02d has two different verdict lines' % e)
            else:
                rows[e] = line
    return rows, pages


def run_dirs(records):
    seen = set()
    for r in records:
        if not os.path.isdir(r):
            raise Refuse('no such record directory: %s' % r)
        for root, dirs, _ in os.walk(r):
            for d in dirs:
                m = re.search(r'-e(\d+)-[0-9a-f]{8}$', d)
                if m:
                    seen.add(int(m.group(1)))
    return seen


def read_results(path):
    out = {}
    for l in open(path, errors='replace'):
        if l.startswith('#') or not l.strip():
            continue
        f = l.rstrip('\n').split('\t')
        if len(f) >= 9 and f[0].isdigit():
            out[int(f[0])] = f
    return out


def kv(line, k):
    m = re.search(r'\b%s=(\S+)' % k, line)
    return m.group(1) if m else '-'


def build(a):
    if not re.fullmatch(r'[0-9a-f]{40}', a.commit):
        raise Refuse('--commit must be the full 40-hex commit, got %r' % a.commit)
    if not re.fullmatch(r'\d{4}-\d{2}-\d{2}', a.date):
        raise Refuse('--date must be YYYY-MM-DD')
    lo, hi = parse_rows(a.rows)
    want = list(range(lo - 1, hi))
    rows, pages = read_lines(a.lines)
    extra = sorted(set(rows) - set(want))
    if extra:
        raise Refuse('verdict lines for entries outside rows %s: %s' % (a.rows, ' '.join('e%02d' % e for e in extra)))
    miss = [e for e in want if e not in rows]
    if miss:
        raise Refuse('no verdict line for: %s' % ' '.join('e%02d' % e for e in miss))
    if a.records:
        have = run_dirs(a.records)
        miss = [e for e in want if e not in have]
        if miss:
            raise Refuse('no record directory for: %s' % ' '.join('e%02d' % e for e in miss))
    res = read_results(a.results) if a.results else None
    if res is not None:
        miss = [e for e in want if e not in res]
        if miss:
            raise Refuse('results.tsv has no row for: %s' % ' '.join('e%02d' % e for e in miss))
    h8 = a.commit[:8]
    span = 'rows %d' % lo if lo == hi else 'rows %d-%d' % (lo, hi)
    ent = 'entry %d' % want[0] if lo == hi else 'entries %d-%d' % (want[0], want[-1])
    o = ['<!-- SPDX-License-Identifier: Apache-2.0 -->', '<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->',
         '# i.MX8MP dEQP-GLES3 table image es3-%s, %s (%s)' % (h8, span, ent), '',
         'commit %s' % a.commit, '',
         'Run %s, Toradex Verdin iMX8M Plus (GC7000UL): the slice `%s-imx8mp-gles3-%s-rows%02d-%02d`, table rows %s of '
         '`tools/cts/imx8mp-gles3-tree.txt` (%s, counted from 0 as the selector does), one boot per entry, from '
         'the campaign commit above. The procedure is `docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice '
         'session".' % (a.date, a.date, h8, lo, hi, a.rows, ent), '',
         '## Verdict lines', '',
         'Verbatim from `tools/board.sh imx8mp` (`back_after` is reboot to ssh, `uptime` Linux\'s at that moment):',
         '', '```']
    o += [rows[e] for e in want]
    o += ['```', '', '## Per entry', '']
    if res is None:
        o += ['| entry | rc | verdict | resets | back_after | uptime | pages |', '|---|---|---|---|---|---|---|']
        for e in want:
            l = rows[e]
            o.append('| e%d | %s | %s | %s | %s | %s | %d |' % (e, kv(l, 'rc'), kv(l, 'verdict'), kv(l, 'resets'),
                                                          kv(l, 'back_after'), kv(l, 'uptime'), len(pages.get(e, ()))))
    else:
        o += ['| entry | rc | verdict | resets | back_after | uptime | pages | begun | pass | fail | n.s. | other | none |',
              '|---|---|---|---|---|---|---|---|---|---|---|---|---|']
        for e in want:
            l, f = rows[e], res[e]
            o.append('| e%d | %s | %s | %s | %s | %s | %d | %s |' % (
                e, kv(l, 'rc'), kv(l, 'verdict'), kv(l, 'resets'), kv(l, 'back_after'), kv(l, 'uptime'),
                len(pages.get(e, ())), ' | '.join(f[2:8])))
        o += ['', '## Not-pass list', '',
              'The failing names `results.tsv` holds per entry (`imx8mp-gles3-merge.py --notpass` reads them from',
              'there; the other statuses come from the per-case sources):', '']
        for e in want:
            names = [n for n in res[e][8].split(';') if n and n != '-']
            o.append('- e%d: %s' % (e, ('%d failing: ' % len(names) + '; '.join(names)) if names else 'none named'))
    o += ['', '`merge.py --check` reads the verdict lines above; `results.tsv` and `rows.md` of the same tally',
          'stand beside this README.', '']
    return '\n'.join(o)


def redact_ok(text):
    with tempfile.TemporaryDirectory() as t:
        p = os.path.join(t, 'README.md')
        with open(p, 'w') as f:
            f.write(text)
        r = subprocess.run([sys.executable, '-I', os.path.join(HERE, 'imx8mp-gles3-redact.py'), '--check', p],
                           capture_output=True, text=True)
        return r.returncode == 0, (r.stdout + r.stderr).replace(p, 'README.md')


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--commit', required=True)
    ap.add_argument('--rows', required=True)
    ap.add_argument('--date', required=True)
    ap.add_argument('--lines', action='append', required=True, metavar='FILE')
    ap.add_argument('--records', nargs='*', default=[])
    ap.add_argument('--results')
    ap.add_argument('--out')
    a = ap.parse_args()
    try:
        text = build(a)
    except (Refuse, OSError) as e:
        print('slice: refused: %s' % e, file=sys.stderr)
        return 2
    ok, msg = redact_ok(text)
    if not ok:
        print('slice: the README fails the redact check, nothing written:\n%s' % msg, file=sys.stderr)
        return 1
    if a.out:
        os.makedirs(a.out, exist_ok=True)
        with open(os.path.join(a.out, 'README.md'), 'w') as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
