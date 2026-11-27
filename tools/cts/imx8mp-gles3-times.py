#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-entry board times of the i.MX8MP dEQP-GLES3 slices against the session estimates.

  python3 -I imx8mp-gles3-times.py [--tsv] [--table T] [--mustpass M] [--record DIR] [--scale S] SLICE_DIR...

SLICE_DIR holds the README.md of a slice (tools/cts/log/<date>-imx8mp-gles3-<slice>/). Its verdict lines
`board.sh imx8mp e<NN> rc=.. verdict=.. ... back_after=<n>s uptime=<n>s` give the time: back_after is reboot
to ssh, uptime is Linux's own boot inside it, so the measured wall time of the entry's run is back_after -
uptime (back_after alone when the line has no uptime or is an x2 line; 'none' when the line has no back_after, e.g. rc=5).
On the Armadillo-X2 (`board.sh x2 e<NN> ...`, slices written by imx8mp-gles3-slice.py: a crash-route kexec, then a wait
of up to 1500 s for Linux) the line has back_after only, the seconds from the jump to ssh answering in the new
Linux, and that is the time used (not U-Boot's; uptime is never subtracted on x2). A slice with x2 lines says so
on a '# ' line under its name (in --tsv: a '# ' comment line before the slice's rows); Verdin output is unchanged.
The failure-name page boots (e<NN>-p<page>) are not entries and are skipped. An entry twice in a README: the
last line counts.

Output, one row per entry in the README, in the order given: entry (eNN), measured seconds, estimate
seconds, ratio measured/estimate ('-' when either is missing or the estimate is 0); then a total per slice
(entries timed of all, measured, estimate over the timed entries, ratio). --tsv: the same as TSV (columns
slice, entry, measured_s, estimate_s, ratio; the slice's last row has entry 'TOTAL').

The estimate is the 'est_seconds' column of imx8mp-gles3-sessions.py, computed with that script's own
functions: it is imported by path (importlib, so the dash in its name and python -I do not matter; it
imports only the standard library). If the import fails, the estimate is 'none' and a warning says why.
A slice directory without a README, or whose README has no verdict lines (the Pi 4 log), prints no entries
and a total of 0.
"""
import argparse, importlib.util, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
LINE = re.compile(r'\bboard\.sh\s+(\S+)\s+e(\d+)\s+rc=(\S+)\s+verdict=(\S+)(.*)$')
X2_NOTE = ('board Armadillo-X2 (i.MX 8M Plus, GC7000UL), tools/board.sh x2 crash-route kexec: time = back_after, the Linux-back '
           'time after the kexec jump (ping lost, ping back, ssh back; not U-Boot), uptime not subtracted')


def load_sessions():
    spec = importlib.util.spec_from_file_location('imx8mp_gles3_sessions', os.path.join(HERE, 'imx8mp-gles3-sessions.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def estimates(S, a):
    """{entry index: estimated seconds}, the per-entry model of imx8mp-gles3-sessions.py main()"""
    cases = S.read_cases(a.mustpass)
    pats = [S.compile_entry(e) for e in S.read_table(a.table)]
    measured, _ = S.pi4_rates(a.record, cases, a.pi4_table)
    sec = [0.0] * len(pats)
    for c in cases:
        hit = [i for i, rs in enumerate(pats) if any(r.match(c) for r in rs)]
        if len(hit) != 1:
            continue
        k = S.group_key(c)
        sec[hit[0]] += measured[k][0] * a.scale if k in measured else S.assumed_rate(c)[0]
    return dict(enumerate(sec))


def read_times(d):
    """([(entry, seconds or None)] sorted by entry, one per entry (last line wins), set of boards named)

    Verdin (imx8mp): back_after - uptime. Armadillo-X2 (x2): back_after alone, the Linux-back time after the
    kexec; an x2 line has no uptime, and one that had it would not be subtracted."""
    got = {}
    boards = set()
    p = os.path.join(d, 'README.md')
    if not os.path.isfile(p):
        return [], boards
    for l in open(p, errors='replace'):
        m = LINE.search(l)
        if not m:
            continue
        kv = dict(re.findall(r'\b(back_after|uptime)=(\d+)s', m.group(5)))
        t = None
        if 'back_after' in kv:
            t = int(kv['back_after']) - (0 if m.group(1) == 'x2' else int(kv.get('uptime', 0)))
        boards.add(m.group(1))
        got[int(m.group(2))] = t
    return sorted(got.items()), boards


def fmt(x, f='%.1f'):
    return 'none' if x is None else f % x


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('slices', nargs='+', metavar='SLICE_DIR')
    ap.add_argument('--tsv', action='store_true')
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--record', default=os.path.join(HERE, 'log', '2026-10-31-pi4-gles3-a'))
    ap.add_argument('--pi4-table', default=os.path.join(HERE, 'pi4-gles3-tree.txt'))
    ap.add_argument('--scale', type=float, default=1.0, help='multiplier on the Pi 4 rates, as in sessions.py')
    a = ap.parse_args()
    try:
        est = estimates(load_sessions(), a)
    except Exception as e:  # any failure leaves the estimate 'none'
        print('WARNING: no estimates (imx8mp-gles3-sessions.py: %s: %s)' % (type(e).__name__, e), file=sys.stderr)
        est = {}
    out = sys.stdout
    if a.tsv:
        out.write('slice\tentry\tmeasured_s\testimate_s\tratio\n')
    else:
        out.write('%-6s %10s %10s %7s\n' % ('entry', 'measured_s', 'estimate_s', 'ratio'))
    for d in a.slices:
        name = os.path.basename(os.path.normpath(d))
        rows, boards = read_times(d)
        note = X2_NOTE if 'x2' in boards else None
        n = tm = te = 0
        if not a.tsv:
            out.write('# %s\n' % name)
            if note:
                out.write('# %s\n' % note)
        elif note:
            out.write('# %s: %s\n' % (name, note))
        for i, t in rows:
            e = est.get(i)
            r = t / e if t is not None and e else None
            if t is not None:
                n += 1
                tm += t
                te += e or 0
            rs = '-' if r is None else '%.2f' % r
            if a.tsv:
                out.write('%s\te%02d\t%s\t%s\t%s\n' % (name, i, fmt(t, '%d'), fmt(e), rs))
            else:
                out.write('e%02d    %10s %10s %7s\n' % (i, fmt(t, '%d'), fmt(e), rs))
        tr = '%.2f' % (tm / te) if te else '-'
        if a.tsv:
            out.write('%s\tTOTAL\t%d\t%.1f\t%s\n' % (name, tm, te, tr))
        else:
            out.write('total  %d of %d entries timed: measured %d s, estimate %.1f s, ratio %s\n' % (n, len(rows), tm, te, tr))
    return 0


if __name__ == '__main__':
    sys.exit(main())
