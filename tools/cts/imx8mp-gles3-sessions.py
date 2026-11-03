#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Plan the i.MX8MP dEQP-GLES3 board sessions: cases and estimated board seconds per table entry, a watchdog-risk flag, row ranges.

  imx8mp-gles3-sessions.py [--table T] [--mustpass M] [--record DIR] [--margin 0.8] [--window 600] [--scale 1.0]
                           [--sessions N | --session-budget SECONDS] [--overhead SECONDS]

No option: one TSV row per table entry: index, cases (mustpass cases the entry matches), estimated seconds,
rate source, risk (RISK when the estimate exceeds margin x window), first pattern. Lines starting with '#'
document the model and the sources of every rate. --sessions N or --session-budget S: instead print
contiguous row ranges (start..end inclusive, covering every entry once) with each range's estimated total;
--sessions minimises the largest total, --session-budget fills each range up to S seconds. --overhead adds a
fixed per-entry boot/reboot cost to the totals (default 0).

Model: seconds = sum over the entry's cases of a seconds-per-case rate by group. Measured rate: the Pi 4
GLES3 record (log/2026-10-31-pi4-gles3-a, cycles.txt: cases and run ms per image, images = rows 9-20 of
pi4-gles3-tree.txt), times --scale (the GC7000UL/V3D ratio is unmeasured; the GC7000UL GLES2 results carry
no times). Assumed rate (marked 'assumed'): the table header's figures: AM62P texture.filtering.3d 22 s a case,
shaders 0.3 s (2,000 per window); otherwise half of window/budget of the group's table budget.
Matching is dEQP's filter as in check-imx8mp-gles3-tree.py (whole name, '*' any run, comma-separated); that
checker has no importable function, so its one-line pattern rule is repeated in compile_entry().
"""
import argparse, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
PI4_LABELS = 'FO2 FO3 FO4 FO5 FO6 FR1 FR2 LT M1 PR R1 SQ'.split()
PI4_FIRST_ROW = 9

# (prefix after "dEQP-GLES3.", seconds per case, why): first match wins; the rest is budget-derived
ASSUMED = [
    ('functional.texture.filtering.3d', 22.0, 'AM62P 19-25 s a case (table header)'),
    ('functional.shaders', 0.3, 'AM62P 2,000 shader cases per 600 s (table header)'),
]
# (prefix, cases per entry) as in imx8mp-gles3-gen.py BUDGET / DEFAULT
BUDGET = [('functional.texture.filtering.3d', 20), ('functional.texture.size', 30),
          ('functional.texture.format', 40), ('functional.texture.shadow', 80), ('functional.texture', 150),
          ('functional.fbo', 400), ('functional.fragment_out', 400), ('functional.fragment_ops', 500),
          ('functional.transform_feedback', 300), ('functional.ubo', 600),
          ('functional.shaders.random', 600), ('functional.shaders', 1000)]
DEFAULT_BUDGET = 800


def compile_entry(e):
    """dEQP filter -> list of regexes (same rule as check-imx8mp-gles3-tree.py)"""
    return [re.compile('^' + '.*'.join(re.escape(x) for x in p.split('*')) + '$') for p in e.split(',') if p]


def read_table(path):
    return [l.rstrip('\n') for l in open(path) if l.strip() and not l.lstrip().startswith('#')]


def read_cases(path):
    return [l.strip() for l in open(path) if l.strip() and not l.startswith('#')]


def prefix_hit(n, p):
    return n == p or n.startswith(p + '.')


def group_key(name):
    p = name.split('.', 1)[1]
    if prefix_hit(p, 'functional.fragment_ops.blend'):
        return 'functional.fragment_ops.blend'
    q = p.split('.')
    return '.'.join(q[:2]) if q[0] == 'functional' and len(q) > 1 else q[0]


def assumed_rate(name):
    n = name.split('.', 1)[1]
    for p, r, why in ASSUMED:
        if prefix_hit(n, p):
            return r, 'assumed:%s' % why.split(' (')[0]
    b = DEFAULT_BUDGET
    for p, v in BUDGET:
        if prefix_hit(n, p):
            b = v
            break
    return 0.5 * 600.0 / b, 'assumed:half of 600 s/%d' % b


def pi4_rates(rec, cases, pi4_table):
    """group key -> (s/case, source) from the Pi 4 record, and notes"""
    cyc = os.path.join(rec, 'cycles.txt')
    if not os.path.exists(cyc) or not os.path.exists(pi4_table):
        return {}, ['Pi 4 record not found (%s): every rate is assumed' % rec]
    imgs, lab = {}, None
    for l in open(cyc):
        m = re.match(r'^(\w+) sha256=', l)
        if m:
            lab = m.group(1)
            continue
        m = re.match(r'^\s+(\d+) cases in (\d+) ms', l)
        if m and lab:
            imgs[lab] = (int(m.group(1)), int(m.group(2)))
    rows = read_table(pi4_table)
    pats = {}
    for i, lab in enumerate(PI4_LABELS):
        if lab in imgs and PI4_FIRST_ROW + i < len(rows):
            pats[lab] = compile_entry(rows[PI4_FIRST_ROW + i])
    keys = {lab: set() for lab in pats}
    for c in cases:
        for lab, rs in pats.items():
            if any(r.match(c) for r in rs):
                keys[lab].add(group_key(c))
    out = {}
    # pass 1: images of a single group, pooled; pass 2: mixed images for groups still without a rate
    pool = {}
    for lab, ks in keys.items():
        if len(ks) == 1:
            pool.setdefault(next(iter(ks)), []).append(lab)
    for k, labs in pool.items():
        cs = sum(imgs[l][0] for l in labs)
        ms = sum(imgs[l][1] for l in labs)
        out[k] = (ms / 1000.0 / cs, 'pi4:' + '+'.join(labs))
    for lab, ks in keys.items():
        if len(ks) > 1:
            for k in sorted(ks):
                if k not in out:
                    out[k] = (imgs[lab][1] / 1000.0 / imgs[lab][0], 'pi4:' + lab)
    return out, []


def time_column_note(results):
    if not os.path.exists(results):
        return 'GC7000UL GLES2 results: file not found'
    hdr = open(results).readline().lower()
    if re.search(r'second|time|\bms\b', hdr):
        return 'GC7000UL GLES2 results carry a time column, but this script does not read it yet'
    return 'GC7000UL GLES2 results (%s) carry no times: not used' % os.path.basename(results)


def split_budget(sec, budget):
    r, s, acc = [], 0, 0.0
    for i, x in enumerate(sec):
        if acc and acc + x > budget:
            r.append((s, i - 1))
            s, acc = i, 0.0
        acc += x
    r.append((s, len(sec) - 1))
    return r


def split_min_max(sec, n):
    """contiguous partition of sec into n ranges (fewer if n > len) minimising the largest sum"""
    n = max(1, min(n, len(sec)))
    lo, hi = max(sec), sum(sec)
    for _ in range(100):
        mid = (lo + hi) / 2
        if len(split_budget(sec, mid)) <= n:
            hi = mid
        else:
            lo = mid
    r = split_budget(sec, hi)
    while len(r) < n:   # split the largest multi-entry range so that exactly n come out
        cand = [k for k, (s, e) in enumerate(r) if e > s]
        if not cand:
            break
        j = max(cand, key=lambda k: sum(sec[r[k][0]:r[k][1] + 1]))
        s, e = r[j]
        best = min(range(s, e), key=lambda c: max(sum(sec[s:c + 1]), sum(sec[c + 1:e + 1])))
        r[j:j + 1] = [(s, best), (best + 1, e)]
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--table', default=os.path.join(HERE, 'imx8mp-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--record', default=os.path.join(HERE, 'log', '2026-10-31-pi4-gles3-a'))
    ap.add_argument('--pi4-table', default=os.path.join(HERE, 'pi4-gles3-tree.txt'))
    ap.add_argument('--gles2-results', default=os.path.join(HERE, 'imx8mp-full-tree-29460f96.results.tsv'))
    ap.add_argument('--margin', type=float, default=0.8, help='risk when the estimate exceeds margin x window (default 0.8)')
    ap.add_argument('--window', type=float, default=600.0, help='WDOG1 window in seconds (default 600)')
    ap.add_argument('--scale', type=float, default=1.0, help='multiplier on the measured Pi 4 rates (GC7000UL/V3D ratio, unmeasured; default 1)')
    ap.add_argument('--overhead', type=float, default=0.0, help='fixed seconds per entry added to range totals (default 0)')
    g = ap.add_mutually_exclusive_group()
    g.add_argument('--sessions', type=int, help='print N contiguous row ranges with balanced totals')
    g.add_argument('--session-budget', type=float, help='print row ranges filled up to this many seconds')
    a = ap.parse_args()

    cases = read_cases(a.mustpass)
    entries = read_table(a.table)
    pats = [compile_entry(e) for e in entries]
    hdr = ''.join(l for l in open(a.table) if l.startswith('#'))
    m = re.search(r'\((\d+) cases\)', hdr)
    declared = int(m.group(1)) if m else None
    measured, notes = pi4_rates(a.record, cases, a.pi4_table)
    notes.append(time_column_note(a.gles2_results))

    n = len(entries)
    cnt, sec, srcs = [0] * n, [0.0] * n, [dict() for _ in range(n)]
    problems = []
    for c in cases:
        hit = [i for i, rs in enumerate(pats) if any(r.match(c) for r in rs)]
        if len(hit) != 1:
            problems.append('%s matched by %d entries' % (c, len(hit)))
            continue
        i = hit[0]
        k = group_key(c)
        if k in measured:
            rate, src = measured[k][0] * a.scale, measured[k][1]
        else:
            rate, src = assumed_rate(c)
        cnt[i] += 1
        sec[i] += rate
        srcs[i][src] = srcs[i].get(src, 0) + 1
    if declared is not None and sum(cnt) != declared:
        problems.append('cases sum %d, the table header says %d' % (sum(cnt), declared))
    for p in problems[:10]:
        print('WARNING:', p, file=sys.stderr)

    def srctext(i):
        multi = len(srcs[i]) > 1
        return ';'.join(('%s x%d' % (s, k)) if multi else s
                        for s, k in sorted(srcs[i].items(), key=lambda t: -t[1])) or '-'

    out = sys.stdout
    out.write('# i.MX8MP dEQP-GLES3 sessions: %d entries, %d mustpass cases, window %g s, risk above %g%% (%g s)\n'
              % (n, sum(cnt), a.window, a.margin * 100, a.margin * a.window))
    out.write('# rates: Pi 4 record %s (x%g scale) where measured, else assumed from the table header\n'
              % (os.path.relpath(a.record, HERE), a.scale))
    for k, (r, s) in sorted(measured.items()):
        out.write('# rate %s = %.4f s/case (%s)\n' % (k, r * a.scale, s))
    for p, r, why in ASSUMED:
        out.write('# rate %s = %.2f s/case (assumed: %s)\n' % (p, r, why))
    out.write('# other unmeasured groups: assumed 0.5 x 600 s / budget s/case (budgets of imx8mp-gles3-gen.py)\n')
    for t in notes:
        out.write('# note: %s\n' % t)
    if a.sessions or a.session_budget:
        tot = [s + a.overhead for s in sec]
        rng = split_min_max(tot, a.sessions) if a.sessions else split_budget(tot, a.session_budget)
        out.write('session\tstart\tend\tentries\tcases\test_seconds\trisky_entries\n')
        for k, (s, e) in enumerate(rng):
            risky = [str(i) for i in range(s, e + 1) if sec[i] > a.margin * a.window]
            out.write('%d\t%d\t%d\t%d\t%d\t%.0f\t%s\n' % (k, s, e, e - s + 1, sum(cnt[s:e + 1]), sum(tot[s:e + 1]),
                                                           ','.join(risky) or '-'))
        return 1 if problems else 0
    out.write('index\tcases\test_seconds\trate_source\trisk\tfirst_pattern\n')
    for i, e in enumerate(entries):
        out.write('%d\t%d\t%.1f\t%s\t%s\t%s\n' % (i, cnt[i], sec[i], srctext(i),
                                                  'RISK' if sec[i] > a.margin * a.window else '-', e.split(',')[0]))
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
