#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Classify how each Armadillo-X2 dEQP-GLES3 slice entry ended.

  python3 -I imx8mp-gles3-x2class.py [--table T] [--mustpass M] [--scan FILE]... [--expected]
                                     [--early-below S] [--watchdog-from S] SLICE_DIR...

Rule: docs/notes/x2-gc7000ul-board-differences.md, "How a tally classifies a result" (D6: the three-word
record; the riskx2 rule: never take verdict=done as clean; a watchdog end is cases short of the mustpass count
AND back_after of about 680-700 s, a short count with back_after under about 630 s is an early stop).

SLICE_DIR holds a README.md with the slice's `board.sh x2 e<NN> rc=.. verdict=.. ... resets=.. record=w0,w1,w2
back_after=<n>s` lines (tools/cts/imx8mp-gles3-slice.py). Page lines (e<NN>-p<k>) are skipped; an entry twice:
the last line counts. --scan FILE is the output of imx8mp-console-scan.py of the slice's console (rows are table
entries, counted from 0 as the selector is): its recoveries column separates a time overrun (0 recoveries) from a
GPU-driver cause, and its "NO FINISH (watchdog end)" is a watchdog end.

--expected prints `entry<TAB>expected begun` for the 133 entries (mustpass cases per entry, as
check-imx8mp-gles3-tree.py matches them) and the sum, and nothing else.

Output, one TSV line per entry in README order (slice dirs in the order given):
  entry  expected  counted  shortfall  back_after  seq  class  reason
entry is the table index; expected = mustpass cases of the entry; counted = pass+fail+nsup+other of the
record; shortfall = max(expected-counted, 0); back_after in seconds ('-' when absent); seq is the sequence-byte
verdict: ok (every begun case ended), begun-no-end (a case began and has no end), bad (neither) or n/a (no record
or a kind that has none). class is one of:
  clean      counted >= expected, seq ok, verdict done, no resets, no recoveries seen
  watchdog   short, and back_after >= --watchdog-from (default 680) or the scan says watchdog end
  early-stop short, back_after < --early-below (default 630) or verdict done_stopped
  gpu-reset  short or complete, with resets > 0 in the verdict line or recoveries > 0 in the scan
  unproven   anything else (no record, back_after between the two limits, seq bad, ...); the reason says why
Sequence byte (deqp_snvs.c): up to 32 cases the record is kind 9 and its low byte is the begun count, w1/w2 hold
2 bits per case (0 = begun without an end); past 32 it is kind 8 with the low byte ++seq at every begin and end,
counted from the 33rd case's begin, so seq = begun + ended - 64 (mod 256) and (seq - 2*ended + 64) mod 256 is
0 on a clean end and 1 when a case began without an end. (The note's `seq - 2*ended` omits the 64.)
Exit status: 0; 2 on unreadable input (no README, no verdict line, unreadable scan, missing table).
"""
import argparse, importlib.util, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r'\bboard\.sh\s+(\S+)\s+e(\d+)\s+rc=(\S+)\s+verdict=(\S+)(.*)$')
SCAN_ROW = re.compile(r'^row\s+(\d+)\s+\S+\s+\S+\s+results\s+(\d+)\s+recoveries\s+(\d+)\s+(finish|!! NO FINISH \(([^)]*)\))')
KIND_DEQP, KIND_CASES = 8, 9


def merge_mod():
    spec = importlib.util.spec_from_file_location('imx8mp_gles3_merge', os.path.join(HERE, 'imx8mp-gles3-merge.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def expected_counts(table, mustpass):
    m = merge_mod()
    return [len(c) for c in m.entry_cases(m.load_table(table), mustpass)]


def read_slice(d):
    p = os.path.join(d, 'README.md')
    if not os.path.isfile(p):
        raise OSError('%s: no README.md' % d)
    rows = {}
    for l in open(p, errors='replace'):
        m = LINE.search(l)
        if m and re.fullmatch(r'\d+', m.group(2)):
            rest = m.group(5)
            f = dict(re.findall(r'(\w+)=(\S+)', rest))
            rows[int(m.group(2))] = {'verdict': m.group(4), 'resets': f.get('resets', 'absent'),
                                     'record': f.get('record', 'absent'),
                                     'back': re.sub(r's$', '', f.get('back_after', ''))}
    if not rows:
        raise OSError('%s: README.md has no board.sh verdict line' % d)
    return rows


def read_scan(p):
    rows = {}
    for l in open(p, errors='replace'):
        m = SCAN_ROW.match(l.strip())
        if m:
            rows[int(m.group(1))] = {'recoveries': int(m.group(3)), 'how': m.group(5) if m.group(4) != 'finish' else 'finish'}
    return rows


def record_state(rec):
    """(counted, seq verdict, note) of a record=w0,w1,w2[,w3] string; counted None when unreadable."""
    ws = rec.split(',')
    if rec == 'absent' or len(ws) < 3 or not all(re.fullmatch(r'[0-9a-fA-F]{1,8}', w) for w in ws):
        return None, 'n/a', 'no readable record'
    w0, w1, w2 = (int(w, 16) for w in ws[:3])
    kind, low = (w0 >> 8) & 255, w0 & 255
    if kind == KIND_CASES:
        bits = w1 | w2 << 32
        ended = sum(1 for i in range(min(low, 32)) if (bits >> (2 * i)) & 3)
        return ended, ('ok' if low == ended else 'begun-no-end' if low == ended + 1 else 'bad'), ''
    if kind == KIND_DEQP:
        ended = (w1 & 0xffff) + (w1 >> 16) + (w2 & 0xffff) + (w2 >> 16)
        d = (low - 2 * ended + 64) % 256
        return ended, ('ok' if d == 0 else 'begun-no-end' if d == 1 else 'bad'), ''
    return None, 'n/a', 'record kind %d is not a dEQP tally' % kind


def classify(exp, row, scan, early_below, wd_from):
    counted, seq, note = record_state(row['record'])
    back = int(row['back']) if re.fullmatch(r'\d+', row['back']) else None
    verdict = row['verdict']
    resets = int(row['resets']) if re.fullmatch(r'\d+', row['resets']) else 0
    rec_n = scan['recoveries'] if scan else 0
    short = max(exp - counted, 0) if counted is not None else None
    wd_scan = bool(scan) and scan['how'] == 'watchdog end'
    if resets > 0 or rec_n > 0:
        src = 'resets=%d' % resets if resets > 0 else '%d recovery(ies) in the console scan' % rec_n
        return counted, short, back, seq, 'gpu-reset', '%s: a GPU-driver cause, not a time overrun' % src
    if counted is None:
        if wd_scan:
            return counted, short, back, seq, 'watchdog', 'no record (%s); the console scan shows a watchdog end, 0 recoveries' % note
        return counted, short, back, seq, 'unproven', '%s (verdict=%s)' % (note, verdict)
    if short == 0:
        if verdict != 'done':
            return counted, short, back, seq, 'unproven', 'count complete but verdict=%s' % verdict
        if seq != 'ok':
            return counted, short, back, seq, 'unproven', 'count complete but the sequence byte says %s' % seq
        if scan and scan['how'] != 'finish':
            return counted, short, back, seq, 'unproven', 'count complete but the console scan has no finish (%s)' % scan['how']
        return counted, short, back, seq, 'clean', 'all %d cases ended, sequence ok, verdict=done, no resets seen' % exp
    if wd_scan or (back is not None and back >= wd_from):
        return counted, short, back, seq, 'watchdog', '%d short%s; %s, 0 recoveries' % (
            short, ' (a case began without an end)' if seq == 'begun-no-end' else '',
            'console scan: watchdog end' if wd_scan else 'back_after %ds >= %d' % (back, wd_from))
    if verdict == 'done_stopped' or (back is not None and back < early_below):
        return counted, short, back, seq, 'early-stop', '%d short%s; %s' % (
            short, ' (a case began without an end)' if seq == 'begun-no-end' else '',
            'verdict=done_stopped' if back is None or back >= early_below else 'back_after %ds < %d' % (back, early_below))
    if back is None:
        return counted, short, back, seq, 'unproven', '%d short, no back_after' % short
    return counted, short, back, seq, 'unproven', '%d short, back_after %ds is between %d and %d' % (short, back, early_below, wd_from)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('slices', nargs='*', metavar='SLICE_DIR')
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--scan', action='append', default=[], help='imx8mp-console-scan.py output (repeatable)')
    ap.add_argument('--expected', action='store_true', help='print the expected begun count per entry and the sum')
    ap.add_argument('--early-below', type=int, default=630)
    ap.add_argument('--watchdog-from', type=int, default=680)
    a = ap.parse_args()
    try:
        exp = expected_counts(a.table, a.mustpass)
        if a.expected:
            for i, n in enumerate(exp):
                print('%d\t%d' % (i, n))
            print('sum\t%d' % sum(exp))
            return 0
        if not a.slices:
            ap.error('no SLICE_DIR')
        scan = {}
        for p in a.scan:
            scan.update(read_scan(p))
        slices = [read_slice(d) for d in a.slices]
    except (OSError, ValueError) as e:
        print('x2class: %s' % e, file=sys.stderr)
        return 2
    for rows in slices:
        for n, row in rows.items():
            if n >= len(exp):
                print('x2class: entry %d is not in the table' % n, file=sys.stderr)
                return 2
            counted, short, back, seq, cls, why = classify(exp[n], row, scan.get(n), a.early_below, a.watchdog_from)
            print('\t'.join([str(n), str(exp[n]), '-' if counted is None else str(counted), '-' if short is None else str(short),
                             '-' if back is None else str(back), seq, cls, why]))
    return 0


if __name__ == '__main__':
    sys.exit(main())
