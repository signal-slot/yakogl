#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the terrain (or another scene's) job entries of two or more board records.

  python3 -I compare.py [--scene NAME] [--counter KEY]... <record> <record> [<record>...]

A <record> is anything parse.py reads: a run directory, a stage directory
(its last run with a terrain line) or a console log. The first record is the
baseline; every later one is shown with its difference to the first.

Prints a markdown table of: the glmark2-jobs entries of the frame (render,
bin, prep, idle in us; entries are numbered from 1 in the line's order), the
TFU entries' count and sum, the glmark2-frame fields (gl, swap, frame, us),
the FrameTime, wait.ns (ms a frame) and the per-frame values of the chosen
counters (default: mip.queued, tfu.jobs, waits). Names as in
docs/notes/pi4-slow-terrain.md and v3d-blur-jobs.md: "noise" is entry 2,
"terrain" is the entry before the first `tfu 1/8xxus` entry (entry 4 if
there is none), "blur" are entries 8 and 1 (tilt-shift H and V).

The scene is read by parse.py, which knows the terrain scene only; --scene
other than terrain is refused. Standard library only; writes nothing.
"""
import argparse
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location('terrain_parse', os.path.join(HERE, 'parse.py'))
parse = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(parse)

DEFAULT_COUNTERS = ['mip.queued', 'tfu.jobs', 'waits']
FIELDS = ['render', 'bin', 'prep', 'idle']


def roles(jobs):
    """{1-based entry: name} for the noise, terrain and blur entries."""
    out = {}
    if len(jobs) >= 2:
        out[2] = 'noise'
    t = None
    for i, j in enumerate(jobs):
        if j['tfu_jobs'] == 1 and 800 <= j['tfu'] < 900:
            t = i  # 0-based index of the tfu entry
            break
    if t is not None and t >= 1:
        out[t] = 'terrain'
    elif len(jobs) >= 4:
        out[4] = 'terrain'
    for n in (8, 1):
        if n <= len(jobs) and n not in out:
            out[n] = 'blur'
    return out


def num(v):
    if v is None:
        return None
    return v


def fmt(v):
    if v is None:
        return 'n/a'
    if isinstance(v, float):
        return ('%.3f' % v)
    return str(v)


def delta(a, b):
    if a is None or b is None:
        return 'n/a'
    d = b - a
    return '%+.3f' % d if isinstance(d, float) else '%+d' % d


def rows_of(r, counters):
    """[(label, value)] for one record, in print order."""
    rows = []
    jobs = r.get('jobs') or []
    names = roles(jobs)
    for i, j in enumerate(jobs, 1):
        tag = ' (%s)' % names[i] if i in names else ''
        for f in FIELDS:
            rows.append(('entry %d%s %s us' % (i, tag, f), j[f]))
    rows.append(('jobs: entries', len(jobs) if jobs else None))
    rows.append(('TFU entries (count)', sum(j['tfu_jobs'] for j in jobs) if jobs else None))
    rows.append(('TFU entries (sum us)', sum(j['tfu'] for j in jobs) if jobs else None))
    rows.append(('render sum us', sum(j['render'] for j in jobs) if jobs else None))
    rows.append(('FrameTime ms', r.get('ms')))
    for k in ('us_frame', 'us_gl', 'us_swap'):
        rows.append(('glmark2-frame %s us' % k[3:], r.get(k)))
    pf = parse.per_frame(r)
    w = pf.get('wait.ns')
    rows.append(('wait.ns ms a frame', round(w / 1e6, 3) if isinstance(w, (int, float)) else None))
    for k in counters:
        v = pf.get(k)
        rows.append(('counter %s (a frame)' % k, v if isinstance(v, (int, float)) else None))
    return rows


def labels_of(rows_list):
    """Row labels in first-seen order, so entries missing in one record still line up."""
    seen = []
    for rows in rows_list:
        for k, _ in rows:
            if k not in seen:
                seen.append(k)
    return seen


def compare(paths, counters):
    recs = [parse.parse(p) for p in paths]
    names = ['%s (%s)' % (r.get('commit', '?'), os.path.basename(p.rstrip('/')) or p)
             for r, p in zip(recs, paths)]
    allrows = [dict(rows_of(r, counters)) for r in recs]
    order = labels_of([rows_of(r, counters) for r in recs])
    o = ['### records', '']
    for i, n in enumerate(names):
        o.append('- %s: %s' % ('first' if i == 0 else 'record %d' % (i + 1), n))
    o.append('')
    head = ['row', 'first'] + [x for i in range(1, len(recs)) for x in ('record %d' % (i + 1), 'change')]
    o.append('| ' + ' | '.join(head) + ' |')
    o.append('|' + '---|' * len(head))
    for k in order:
        a = allrows[0].get(k)
        cells = [k, fmt(a)]
        for rows in allrows[1:]:
            b = rows.get(k)
            cells += [fmt(b), delta(a, b)]
        o.append('| ' + ' | '.join(cells) + ' |')
    return '\n'.join(o) + '\n'


def main(argv):
    ap = argparse.ArgumentParser(
        prog='compare.py', description=__doc__.split('\n\n')[0],
        epilog=__doc__.split('\n\n', 1)[1], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('records', nargs='+', metavar='RECORD',
                    help='run dir, stage dir or console log (two or more; the first is the baseline)')
    ap.add_argument('--scene', default='terrain', help='scene name (default terrain; parse.py reads terrain only)')
    ap.add_argument('--counter', action='append', metavar='KEY',
                    help='a glmark2-counters key to show a frame (repeatable; default %s)'
                    % ', '.join(DEFAULT_COUNTERS))
    a = ap.parse_args(argv)
    if a.scene != 'terrain':
        ap.error('only the terrain scene can be read (parse.py)')
    if len(a.records) < 2:
        ap.error('give at least two records')
    sys.stdout.write(compare(a.records, a.counter or DEFAULT_COUNTERS))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
