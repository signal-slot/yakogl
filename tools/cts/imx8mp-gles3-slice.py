#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the README.md of one i.MX8MP dEQP-GLES3 board slice from a session's `tools/board.sh imx8mp` (or x2) output.

  imx8mp-gles3-slice.py --commit HASH40 --rows AA-BB --date YYYY-MM-DD --lines FILE [--lines FILE...]
                        [--records DIR...] [--results results.tsv] [--out DIR]
                        [--artefacts FILE] [--preflight FILE] [--window START/END] [--run-date YYYY-MM-DD]

--rows is the slice's table rows, counted from 1 as the directory name `<date>-imx8mp-gles3-<hash8>-rows<AA>-<BB>`
counts them: rows01-13 are entries 0-12 (`e<NN>` counts from 0). AA-AA (or AA) is a one-row slice.
--lines is the captured board.sh stdout (`-` is stdin; the run's and the tally's lines may be in one file):
every `board.sh imx8mp e<NN> rc=.. verdict=.. ...` line, verbatim, or every `board.sh x2 e<NN> ...` line of an
Armadillo-X2 session (tools/cts/imx8mp-gles3-board.sh --board x2: boot=kexec-dev, DEV, "never a row" in board.sh's
words; by the owner's decision of 2026-11-27 they are rows of this campaign, docs/es3/board-recipe-imx8mp.md,
"Running a GLES3 slice on the Armadillo-X2"). The board is read from the lines; one slice is one board's session,
so lines of both boards are refused (exit 2). An X2 slice's README says "measured on the Armadillo-X2 (i.MX 8M Plus,
GC7000UL) with tools/board.sh x2 (crash-route kexec)", names its records under records/x2/ and its image under
$IMX8MP_OUT/x2/, and names the slice after --out when that is `<YYYY-MM[-DD]>-imx8mp-gles3-[x2-]<hash8>-rowsAA-BB`
of the commit and rows (the X2 campaign's `2026-11-imx8mp-gles3-x2-<hash8>-rowsAA-BB`); a Verdin slice's README
is byte for byte what it was before.
--run-date is the day the session ran when it is not the slice's --date (the campaign's slice names keep
2026-11-16, docs/es3/board-recipe-imx8mp.md): it goes into the "Run ..." lines, --date stays in the name. Page lines (`e<NN>-p<k>`) are counted per
entry, not copied. A line seen twice verbatim counts once; two different lines for one entry (a rerun: pass
the line that counts), an entry outside the range, or an entry of the range without a line are refused
(exit 2), as is a commit that is not 40 hex characters. --records names the record directories
(`records/imx8mp/...`: run directories `<YYYYMMDD-HHMM>-e<NN>-<sha8>` or a parent holding them); with it, every
entry needs a run directory, and the README names the given directories by their last component only.
--scan is the stdout of imx8mp-console-scan.py (repeatable): its `!! NO FINISH row N (hang|watchdog end|reset) ...;
last result NAME;` lines give an entry's kind and last case. For every row recorded as reset or hung the README gets
a "## Resets and hangs" section with one marker-led line per event, which imx8mp-gles3-report.py counts: `WDOG1 eNN
...` (a WDOG1 reset: verdict done_with_resets or a numeric resets= above 0, or a scan NO FINISH of kind reset or
watchdog end) or `Hang: eNN ...` (verdict no_record, failed or died_after=..., or a scan NO FINISH of kind hang),
`; last case NAME` appended when the scan names one. A row of neither kind adds nothing (no section).
--results is the tally's results.tsv (its rows of the slice give the counts and the failing names the README
lists per entry); it must have a row for every entry of the slice.

The README has the 40-hex commit, the verdict lines in a fenced block (what imx8mp-gles3-merge.py --check and
imx8mp-gles3-times.py read), a per-entry table (rc, verdict, resets, back_after, uptime, pages) and, with
--results, the per-entry counts and the not-pass list (failing names). The tally's results.tsv and rows.md
go beside it, as in every committed slice; one slice alone passes `imx8mp-gles3-merge.py --check --partial`.
--artefacts (the stdout of imx8mp-gles3-artefacts.sh), --preflight (the preflight's stdout) and --window
(board-clock UTC, `HH:MM/HH:MM`, or `YYYY-MM-DDTHH:MM/YYYY-MM-DDTHH:MM`) are optional. With any of them the draft
also has a "## What ran" section (window, artefact lines with the path cut to the part below $IMX8MP_OUT, the
preflight result, the commands with knob names only) and a "## Console" section for BOARD_UART unset ("not
captured, recoveries not observed"); without all three the output is exactly as before.
Without --out the README goes to stdout; with --out it is DIR/README.md. Before it is written or printed,
imx8mp-gles3-redact.py --check must find nothing in it (exit 1 on a hit, nothing written). The check's scratch
file is under --out, else $TMPDIR, else the current directory; never /tmp or /var/tmp.
With --out and --records, each entry's record directory (DIR/<stamp>-eNN-<sha8>, the last by name when an entry
has several) that has a meta.txt gets it copied as eNN.meta.txt beside the README (the optional file that
imx8mp-gles3-times.py reads when a verdict line has back_after=?s). The text goes through the redact tool's
--fix; if --check still finds a hit, only the lines whose key a reader uses (META_KEYS) are kept, and if that
is still not clean the slice is refused.
"""
import argparse, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r'^\s*(board\.sh\s+(imx8mp|x2)\s+e(\d+)(?:-p(\d+))?\s+rc=\S+\s+verdict=\S+.*?)\s*$')
X2_MEASURED = 'measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2 (crash-route kexec)'

SCAN_NF = re.compile(r'^!! NO FINISH row (\d+) \((hang|watchdog end|reset)\) at [^;]*; last result ([^;]*);')

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
    rows, pages, boards = {}, {}, set()
    for p in paths:
        f = sys.stdin if p == '-' else open(p, errors='replace')
        for l in f:
            m = LINE.match(l)
            if not m:
                continue
            line, e, pg = m.group(1), int(m.group(3)), m.group(4)
            boards.add(m.group(2))
            if pg is not None:
                pages.setdefault(e, set()).add(int(pg))
            elif e in rows and rows[e] != line:
                raise Refuse('entry e%02d has two different verdict lines (a rerun: pass only the one that counts)' % e)
            else:
                rows[e] = line
        if f is not sys.stdin:
            f.close()
    if len(boards) > 1:
        raise Refuse('verdict lines of two boards (board.sh imx8mp and board.sh x2): one slice is one board\'s session')
    return rows, pages, (boards.pop() if boards else 'imx8mp')


def read_scans(paths):
    """{entry: (kind, last case or None)} from the stdout of imx8mp-console-scan.py (kind: reset | hang)."""
    out = {}
    for p in paths:
        with open(p, errors='replace') as f:
            for l in f:
                m = SCAN_NF.match(l.strip())
                if m:
                    case = m.group(3).strip()
                    out[int(m.group(1))] = ('hang' if m.group(2) == 'hang' else 'reset',
                                            None if case in ('none', '(none)', '') else case)
    return out


def fault_kind(line, scan):
    """'reset', 'hang' or None for a row: the scan's NO FINISH kind if it has one, else the verdict line's."""
    if scan:
        return scan[0]
    v, r = kv(line, 'verdict'), kv(line, 'resets')
    if v == 'done_with_resets' or (r.isdigit() and int(r) > 0):
        return 'reset'
    if v in ('no_record', 'failed') or v.startswith('died_after'):
        return 'hang'
    return None


def fault_lines(want, rows, scans):
    o = []
    for e in want:
        sc = scans.get(e)
        k = fault_kind(rows[e], sc)
        if not k:
            continue
        l = rows[e]
        t = 'rc=%s verdict=%s resets=%s back_after=%s' % (kv(l, 'rc'), kv(l, 'verdict'), kv(l, 'resets'), kv(l, 'back_after'))
        case = '; last case %s' % sc[1] if sc and sc[1] else ''
        o.append(('WDOG1 e%02d: the entry ended on a WDOG1 reset (%s)%s' if k == 'reset' else
                  'Hang: e%02d: the entry hung (%s)%s') % (e, t, case))
    return o


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


META_KEYS = ('jump', 'refused_rc', 'refused', 'lost', 'back', 'back_after', 't_reboot', 't_back', 'rc_cycle',
             'timeline')  # timeline: imx8mp-gles3-times.py reads 'ssh back <n>s' from it


def meta_files(records):
    """{entry: path of the meta.txt of its record directory}, the last directory by name winning."""
    found = {}
    for r in records:
        for root, dirs, files in os.walk(r):
            dirs.sort()
            m = RUN_DIR.search(os.path.basename(os.path.normpath(root)))
            if m and 'meta.txt' in files:
                e = int(m.group(1))
                q = os.path.join(root, 'meta.txt')
                if e not in found or os.path.basename(root) >= os.path.basename(os.path.dirname(found[e])):
                    found[e] = q
    return found


def redact_run(mode, p):
    return subprocess.run([sys.executable, '-I', os.path.join(HERE, 'imx8mp-gles3-redact.py'), mode, p],
                          capture_output=True, text=True)


def redacted_meta(src, scratch):
    """the redacted text of a meta.txt, or None when not even its reader keys can be made clean"""
    with open(src, errors='replace') as f:
        raw = f.read()
    keys = lambda t: ''.join(l + '\n' for l in t.splitlines() if l.partition('=')[0] in META_KEYS)
    for text in (raw, keys(raw)):
        p = os.path.join(scratch, 'meta.txt')
        with open(p, 'w') as f:
            f.write(text)
        redact_run('--fix', p)
        if redact_run('--check', p).returncode == 0:
            with open(p) as f:
                return f.read()
    return None


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


WINDOW = re.compile(r'(?:\d{4}-\d{2}-\d{2}T)?([01]\d|2[0-3]):[0-5]\d')


def parse_window(s):
    p = s.split('/')
    if len(p) != 2 or not all(WINDOW.fullmatch(x) for x in p):
        raise Refuse('--window must be START/END as HH:MM/HH:MM (board-clock UTC), got %r' % s)
    return [x.split('T')[-1] for x in p]


def rel_artefact(path):
    """The part of an artefact path below $IMX8MP_OUT: from es3-<hash8> (x2/es3-<hash8>) or the stage 1 directory on."""
    parts = path.split('/')
    for i, c in enumerate(parts):
        if re.match(r'es3-[0-9a-f]{8}(\.|$)', c) or c.startswith('deqp-stage1'):
            if i > 0 and parts[i - 1] == 'x2' and c.startswith('es3-'):
                i -= 1
            return '/'.join(parts[i:])
    return parts[-1]


def read_artefacts(path):
    out = []
    with open(path, errors='replace') as f:
        for l in f:
            l = l.rstrip('\n')
            m = re.match(r'^(\d+ [0-9a-f]{64}) (\S+)$', l)
            if m:
                out.append('%s %s' % (m.group(1), rel_artefact(m.group(2))))
            elif re.match(r'^\s*(#\s*)?CONFIG_\w+|^\s*(Memory region|RAM:|FLASH:)', l):
                out.append(l)
    if not any(re.match(r'^\d+ [0-9a-f]{64} ', l) for l in out):
        raise Refuse('--artefacts has no "bytes sha256 file" line (the stdout of imx8mp-gles3-artefacts.sh)')
    return out


def read_preflight(path):
    with open(path, errors='replace') as f:
        ls = [l.strip() for l in f]
    nok = sum(1 for l in ls if l.startswith('ok '))
    bad = sum(1 for l in ls if l.startswith('FAIL '))
    last = [l for l in ls if l.startswith('preflight:')]
    if not last:
        raise Refuse('--preflight has no "preflight: ..." result line')
    return nok, bad, last[-1]


def what_ran(a, lo, hi, h8, board='imx8mp'):
    o = ['', '## What ran', '']
    if a.window:
        st, en = parse_window(a.window)
        o += ['Run %s, %s-%s UTC (the board\'s clock): %s of `tools/cts/imx8mp-gles3-tree.txt`'
              % (a.run_date or a.date, st, en, 'row %d' % lo if lo == hi else 'rows %d-%d' % (lo, hi)),
              '(`TREE_ROWS=%d-%d`), one boot per entry.' % (lo - 1, hi - 1), '']
    if a.artefacts:
        o += ['`tools/cts/imx8mp-gles3-artefacts.sh %ses3-%s` (bytes, sha256, file below `$IMX8MP_OUT`):'
              % ('x2/' if board == 'x2' else '', h8), '',
              '```'] + read_artefacts(a.artefacts) + ['```', '']
    if a.preflight:
        nok, bad, last = read_preflight(a.preflight)
        o += ['`tools/cts/imx8mp-gles3-preflight.sh %s--commit %s --out $IMX8MP_OUT`: %d `ok` line%s, %s`%s`.'
              % ('--board x2 ' if board == 'x2' else '', a.commit, nok, '' if nok == 1 else 's', '%d `FAIL`, ' % bad if bad else '', last), '']
    if board == 'x2':
        rec = '$REC'
        o += ['The run, knob names only (values come from the environment; `$REC` is',
              '`records/x2/<YYYYMMDD-HHMM>-es3-%s-rows%02d-%02d-<image sha8>`):' % (h8, lo, hi), '', '```',
              'TREE_ROWS=%d-%d TREE_IMAGE=$IMX8MP_OUT/x2/es3-%s/zephyr/zephyr.bin IMX8MP_BOARD=x2 \\' % (lo - 1, hi - 1, h8),
              '  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt BOARD_HOST=$BOARD_HOST [BOARD_UART=$BOARD_UART] \\',
              '  tools/cts/imx8mp-gles3-board.sh tree %s' % rec,
              'IMX8MP_BOARD=x2 tools/cts/imx8mp-gles3-board.sh tally %s' % rec, '```', '',
              '`BOARD_HOST` and `BOARD_UART` are named, never their values.']
        return o
    o += ['The run, knob names only (values come from the environment):', '', '```',
          'TREE_ROWS=%d-%d TREE_IMAGE=$IMX8MP_OUT/es3-%s/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \\' % (lo - 1, hi - 1, h8),
          '  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=$RUN_BOUND BOARD_UART=$BOARD_UART \\',
          '  NAMES=0 tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-%s-rows%02d-%02d   # pass 1: the rows'
          % (h8, lo, hi),
          '  ...  (NAMES unset) tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-%s-rows%02d-%02d   # pass 2: the name pages'
          % (h8, lo, hi),
          'tools/cts/imx8mp-gles3-board.sh tally records/imx8mp/es3-%s-rows%02d-%02d' % (h8, lo, hi), '```', '',
          '`BOARD_HOST` and `BOARD_UART` are named, never their values.']
    return o


CONSOLE = ['', '## Console', '',
           'not captured, recoveries not observed: `BOARD_UART` was unset, so `tools/board.sh` kept no console',
           'for these boots and GPU recoveries were not observable (`resets=absent` is `board.sh`\'s, it does',
           'not count them on this board). The zero-recovery proof of the recipe is missing for every row; this is',
           'reported, not claimed.']


def x2_name(a, h8, lo, hi):
    """An X2 slice's directory name: the basename of --out when it is `<YYYY-MM[-DD]>-imx8mp-gles3-[x2-]<hash8>-rowsAA-BB`
    for this commit and these rows (the X2 campaign's `2026-11-imx8mp-gles3-x2-<hash8>-rowsAA-BB`), else the
    `<date>-imx8mp-gles3-<hash8>-rowsAA-BB` form."""
    b = os.path.basename(os.path.normpath(a.out)) if a.out else ''
    if re.fullmatch(r'\d{4}-\d{2}(?:-\d{2})?-imx8mp-gles3-(?:x2-)?%s-rows%02d-%02d' % (h8, lo, hi), b):
        return b
    return '%s-imx8mp-gles3-%s-rows%02d-%02d' % (a.date, h8, lo, hi)


def build(a):
    if not re.fullmatch(r'[0-9a-f]{40}', a.commit):
        raise Refuse('--commit must be the full 40-hex commit, got %r' % a.commit)
    for k, v in (('--date', a.date), ('--run-date', a.run_date)):
        if v is not None and not re.fullmatch(r'\d{4}-\d{2}-\d{2}', v):
            raise Refuse('%s must be YYYY-MM-DD, got %r' % (k, v))
    lo, hi = parse_rows(a.rows)
    want = list(range(lo - 1, hi))
    rows, pages, board = read_lines(a.lines)
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
    scans = read_scans(a.scan)
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
         ]
    if board == 'x2':
        o += ['Run %s, %s: the slice' % (a.run_date or a.date, X2_MEASURED),
              '`%s`, %s of `tools/cts/imx8mp-gles3-tree.txt` (%s, counted from 0 as the'
              % (x2_name(a, h8, lo, hi), span, ent),
              'selector does), one crash-route kexec per entry, from the campaign commit above. The Toradex Verdin iMX8M',
              'Plus is off the farm; by the owner\'s decision of 2026-11-27 these kexec-dev runs (board.sh\'s `DEV`, "never',
              'a row") are rows of this campaign. The procedure is `docs/es3/board-recipe-imx8mp.md`, "Running a GLES3',
              'slice on the Armadillo-X2".']
    else:
        o += ['Run %s on the Toradex Verdin iMX8M Plus (GC7000UL): the slice `%s-imx8mp-gles3-%s-rows%02d-%02d`, %s of'
              % (a.run_date or a.date, a.date, h8, lo, hi, span),
              '`tools/cts/imx8mp-gles3-tree.txt` (%s, counted from 0 as the selector does), one boot per entry, from the'
              % ent,
              'campaign commit above. The procedure is `docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice session".']
    if a.records:
        o += ['', 'Records (gitignored): %s.' % ', '.join(
            '`records/%s/%s`' % (board, os.path.basename(os.path.normpath(r))) for r in a.records)]
    new = bool(a.artefacts or a.preflight or a.window)
    if new:
        o += what_ran(a, lo, hi, h8, board)
    o += ['', '## Verdict lines', '',
          ('Verbatim from `tools/board.sh x2` (`back_after` is the jump to ssh; `DEV` and "never a row" are board.sh\'s'
           ' words, rows here by the owner\'s decision of 2026-11-27):' if board == 'x2' else
           'Verbatim from `tools/board.sh imx8mp` (`back_after` is reboot to ssh, `uptime` Linux\'s at that moment):'),
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
    ev = fault_lines(want, rows, scans)
    if ev:
        o += ['', '## Resets and hangs', ''] + ['- ' + x for x in ev]
    if new:
        o += CONSOLE
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
    ap.add_argument('--scan', action='append', default=[], metavar='FILE')
    ap.add_argument('--artefacts', metavar='FILE')
    ap.add_argument('--preflight', metavar='FILE')
    ap.add_argument('--window', metavar='START/END')
    ap.add_argument('--run-date', metavar='YYYY-MM-DD')
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
    if a.out and a.records:
        try:
            lo, hi = parse_rows(a.rows)
            out = {}
            with tempfile.TemporaryDirectory(prefix='.slice-meta.', dir=a.out) as t:
                for e, src in sorted(meta_files(a.records).items()):
                    if lo - 1 <= e < hi:
                        out[e] = redacted_meta(src, t)
                        if out[e] is None:
                            raise Refuse('e%02d meta.txt cannot be made clean for the redact check' % e)
            for e, txt in out.items():
                with open(os.path.join(a.out, 'e%02d.meta.txt' % e), 'w') as f:
                    f.write(txt)
        except (Refuse, OSError) as ex:
            print('slice: refused: %s' % ex, file=sys.stderr)
            return 2
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
