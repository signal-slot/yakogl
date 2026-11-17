#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the README.md of one i.MX8MP dEQP-GLES3 board slice from a session's `tools/board.sh imx8mp` output.

  imx8mp-gles3-slice.py --commit HASH40 --rows AA-BB --date YYYY-MM-DD --lines FILE [--lines FILE...]
                        [--records DIR...] [--results results.tsv] [--out DIR]

--rows is the slice's table rows, counted from 1 as the directory name `<date>-imx8mp-gles3-<hash8>-rows<AA>-<BB>`
counts them: rows01-13 are entries 0-12 (`e<NN>` counts from 0). AA-AA (or AA) is a one-row slice.
--lines is the captured board.sh stdout (`-` is stdin; the run's and the tally's lines may be in one file):
every `board.sh imx8mp e<NN> rc=.. verdict=.. ...` line, verbatim. Page lines (`e<NN>-p<k>`) are counted per
entry, not copied. A line seen twice verbatim counts once; two different lines for one entry (a rerun: pass
the line that counts), an entry outside the range, or an entry of the range without a line are refused
(exit 2), as is a commit that is not 40 hex characters. --records names the record directories
(`records/imx8mp/...`: run directories `<YYYYMMDD-HHMM>-e<NN>-<sha8>` or a parent holding them); with it, every
entry needs a run directory, and the README names the given directories by their last component only.
--results is the tally's results.tsv (its rows of the slice give the counts and the failing names the README
lists per entry); it must have a row for every entry of the slice.

The README has the 40-hex commit, the verdict lines in a fenced block (what imx8mp-gles3-merge.py --check and
imx8mp-gles3-times.py read), a per-entry table (rc, verdict, resets, back_after, uptime, pages) and, with
--results, the per-entry counts and the not-pass list (failing names). The tally's results.tsv and rows.md
go beside it, as in every committed slice; one slice alone passes `imx8mp-gles3-merge.py --check --partial`.
Without --out the README goes to stdout; with --out it is DIR/README.md. Before it is written or printed,
imx8mp-gles3-redact.py --check must find nothing in it (exit 1 on a hit, nothing written). The check's scratch
file is under --out, else $TMPDIR, else the current directory; never /tmp or /var/tmp.
"""
import argparse, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r'^\s*(board\.sh\s+imx8mp\s+e(\d+)(?:-p(\d+))?\s+rc=\S+\s+verdict=\S+.*?)\s*$')
RUN_DIR = re.compile(r'-e(\d+)-[0-9a-f]{8}$')


class Refuse(Exception):
    pass


def parse_rows(s):
    m = re.fullmatch(r'(\d+)(?:-(\d+))?', s)
    lo, hi = (int(m.group(1)), int(m.group(2) or m.group(1))) if m else (0, 0)
    if not m or not 1 <= lo <= hi:
        raise Refuse('--rows must be AA-BB with 1 <= AA <= BB (rows counted from 1), got %r' % s)
    return lo, hi


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
                raise Refuse('entry e%02d has two different verdict lines (a rerun: pass only the one that counts)' % e)
            else:
                rows[e] = line
        if f is not sys.stdin:
            f.close()
    return rows, pages


def run_dirs(records):
    seen = set()
    for r in records:
        if not os.path.isdir(r):
            raise Refuse('no such record directory: %s' % os.path.basename(os.path.normpath(r)))
        m = RUN_DIR.search(os.path.basename(os.path.normpath(r)))
        if m:
            seen.add(int(m.group(1)))
        for _, dirs, _ in os.walk(r):
            for d in dirs:
                m = RUN_DIR.search(d)
                if m:
                    seen.add(int(m.group(1)))
    return seen


def read_results(path):
    out = {}
    with open(path, errors='replace') as f:
        for l in f:
            if l.startswith('#') or not l.strip():
                continue
            c = l.rstrip('\n').split('\t')
            if len(c) >= 9 and c[0].isdigit():
                out[int(c[0])] = c
    return out


def kv(line, k):
    m = re.search(r'\b%s=(\S+)' % k, line)
    return m.group(1) if m else '-'


def build(a):
    if not re.fullmatch(r'[0-9a-f]{40}', a.commit):
        raise Refuse('--commit must be the full 40-hex commit, got %r' % a.commit)
    if not re.fullmatch(r'\d{4}-\d{2}-\d{2}', a.date):
        raise Refuse('--date must be YYYY-MM-DD, got %r' % a.date)
    lo, hi = parse_rows(a.rows)
    want = list(range(lo - 1, hi))
    rows, pages = read_lines(a.lines)
    extra = sorted(set(rows) - set(want))
    if extra:
        raise Refuse('verdict lines for entries outside rows %d-%d: %s' % (lo, hi, ' '.join('e%02d' % e for e in extra)))
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
         'Run %s on the Toradex Verdin iMX8M Plus (GC7000UL): the slice `%s-imx8mp-gles3-%s-rows%02d-%02d`, %s of'
         % (a.date, a.date, h8, lo, hi, span),
         '`tools/cts/imx8mp-gles3-tree.txt` (%s, counted from 0 as the selector does), one boot per entry, from the'
         % ent,
         'campaign commit above. The procedure is `docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice session".']
    if a.records:
        o += ['', 'Records (gitignored): %s.' % ', '.join(
            '`records/imx8mp/%s`' % os.path.basename(os.path.normpath(r)) for r in a.records)]
    o += ['', '## Verdict lines', '',
          'Verbatim from `tools/board.sh imx8mp` (`back_after` is reboot to ssh, `uptime` Linux\'s at that moment):',
          '', '```']
    o += [rows[e] for e in want]
    o += ['```', '', '## Per entry', '']
    head = '| entry | rc | verdict | resets | back_after | uptime | pages |'
    if res is not None:
        head += ' begun | pass | fail | n.s. | other | none |'
    o += [head, '|' + '---|' * (head.count('|') - 1)]
    for e in want:
        l = rows[e]
        cells = ['e%d' % e, kv(l, 'rc'), kv(l, 'verdict'), kv(l, 'resets'), kv(l, 'back_after'), kv(l, 'uptime'),
                 str(len(pages.get(e, ())))]
        if res is not None:
            cells += res[e][2:8]
        o.append('| ' + ' | '.join(cells) + ' |')
    if res is not None:
        o += ['', '## Not-pass list', '',
              'The failing names `results.tsv` holds per entry (`imx8mp-gles3-merge.py --notpass` reads them from',
              'there; the other statuses come from the per-case sources beside it):', '']
        for e in want:
            names = [n for n in res[e][8].split(';') if n and n != '-']
            o.append('- e%d: %s' % (e, ('%d failing: ' % len(names) + '; '.join(names)) if names else 'none named'))
    o += ['', '`imx8mp-gles3-merge.py --check` reads the verdict lines above; `results.tsv` and `rows.md` of the',
          'same tally stand beside this README.', '']
    return '\n'.join(o)


def scratch_parent(out):
    if out:
        return out
    t = os.environ.get('TMPDIR')
    if t and os.path.isdir(t):
        r = os.path.realpath(t)
        if not any(r == p or r.startswith(p + '/') for p in ('/tmp', '/var/tmp')):
            return t
    return os.getcwd()


def redact_ok(text, parent):
    with tempfile.TemporaryDirectory(prefix='.slice-redact.', dir=parent) as t:
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
        if a.out:
            os.makedirs(a.out, exist_ok=True)
        ok, msg = redact_ok(text, scratch_parent(a.out))
    except (Refuse, OSError) as e:
        print('slice: refused: %s' % e, file=sys.stderr)
        return 2
    if not ok:
        print('slice: the README fails the redact check, nothing written:\n%s' % msg, file=sys.stderr)
        return 1
    if a.out:
        p = os.path.join(a.out, 'README.md')
        with open(p + '.part', 'w') as f:
            f.write(text)
        os.replace(p + '.part', p)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
