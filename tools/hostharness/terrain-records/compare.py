#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the glmark2 jobs, frame and counters lines of two or more records.

  python3 -I compare.py [--scene terrain] [--counters mip.queued,jobs,...]
                        <record dir | console log> <record dir | console log> ...

Prints, for the scene (default terrain), per job entry the render, bin, prep
and idle times (us) of every record with the difference of each later record
from the first, the TFU entries' count and sum, the frame line's gl, swap and
frame times, wait.ns (ms a frame) and the chosen counters a frame. Entries are
named as docs/notes/v3d-blur-jobs.md ("Which entry is which job") names them
for an 8-entry frame; the terrain entry is the first entry with a TFU behind
it in any frame. Reuses parse.py (same directory), which it does not change.
Standard library only; reads files, writes nothing.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parse as P  # noqa: E402

DEFAULT_COUNTERS = ['mip.queued', 'jobs', 'waits', 'tfu.jobs', 'tfu.failed']
ROLES8 = {1: 'tilt-shift V (previous frame)', 2: 'noise (height map)', 3: 'normal map',
          4: 'terrain draw', 5: 'bloom H (blur)', 6: 'bloom V (blur)', 7: 'overlay',
          8: 'tilt-shift H (blur)'}


def parse_scene(path, scene):
    """parse.py's fields for the scene's lines (parse.py reads terrain only)."""
    if scene == 'terrain':
        return P.parse(path)
    runs = P.find_runs(path)
    if not runs:
        raise SystemExit('compare.py: no console in %s' % path)
    d, c = runs[-1]
    L = P.lines_of(c)
    old = P.TERRAIN
    P.TERRAIN = re.compile(re.escape('[%s]' % scene))
    try:
        r = P.parse_one(d, c)
    finally:
        P.TERRAIN = old
    pat = re.compile(r'glmark2-frame: \[%s\].*' % re.escape(scene))
    for l in L:
        m = pat.search(l)
        if m:
            for k in ('gl', 'swap', 'show', 'other', 'frame'):
                mm = re.search(r'\b%s (\d+)' % k, m.group(0))
                if mm:
                    r['us_' + k] = int(mm.group(1))
    return r


def terrain_index(jobs):
    for i, j in enumerate(jobs):
        if j['tfu_jobs']:
            return i + 1
    return None


def role(jobs, n):
    if len(jobs) == 8:
        return ROLES8[n]
    return 'terrain draw' if n == terrain_index(jobs) else ''


def fmt(v, d=0):
    if v is None:
        return 'n/a'
    return ('%.*f' % (d, v)) if d else str(v)


def delta(a, b, d=0):
    if a is None or b is None:
        return 'n/a'
    return '%+.*f' % (d, b - a)


def jobs_section(names, rs):
    o = ['## jobs (us, a frame of the glmark2-jobs line)', '']
    n = max((len(r.get('jobs', [])) for r in rs), default=0)
    if not n:
        return o + ['absent', '']
    for r, nm in zip(rs, names):
        if 'jobs' in r:
            o.append('- %s: frame %d, %d job(s)' % (nm, r['jobs_frame'], len(r['jobs'])))
    o.append('')
    for key in ('render', 'bin', 'prep', 'idle'):
        o.append('### %s' % key)
        o.append('')
        o.append('| entry | job | ' + ' | '.join(names) + ' | ' +
                 ' | '.join('%s - %s' % (x, names[0]) for x in names[1:]) + ' |')
        o.append('|---|---|' + '---|' * (len(names) + len(names) - 1))
        for i in range(1, n + 1):
            vals = [r['jobs'][i - 1][key] if 'jobs' in r and i <= len(r['jobs']) else None for r in rs]
            ref = next((r['jobs'] for r in rs if 'jobs' in r and len(r['jobs']) >= i), [])
            o.append('| %d | %s | %s | %s |' % (
                i, role(ref, i) if ref else '', ' | '.join(fmt(v) for v in vals),
                ' | '.join(delta(vals[0], v) for v in vals[1:])))
        o.append('')
    o.append('### TFU entries (count, sum us)')
    o.append('')
    o.append('| | ' + ' | '.join(names) + ' | ' +
             ' | '.join('%s - %s' % (x, names[0]) for x in names[1:]) + ' |')
    o.append('|---|' + '---|' * (len(names) * 2 - 1))
    cnt = [sum(1 for j in r.get('jobs', []) if j['tfu_jobs']) if 'jobs' in r else None for r in rs]
    tot = [sum(j['tfu'] for j in r['jobs']) if 'jobs' in r else None for r in rs]
    for label, vals in (('entries with a TFU', cnt), ('TFU us', tot)):
        o.append('| %s | %s | %s |' % (label, ' | '.join(fmt(v) for v in vals),
                                       ' | '.join(delta(vals[0], v) for v in vals[1:])))
    o.append('')
    return o


def table(title, rows, names, d=0):
    o = ['## %s' % title, '', '| field | ' + ' | '.join(names) + ' | ' +
         ' | '.join('%s - %s' % (x, names[0]) for x in names[1:]) + ' |',
         '|---|' + '---|' * (len(names) * 2 - 1)]
    for label, vals in rows:
        o.append('| %s | %s | %s |' % (label, ' | '.join(fmt(v, d) for v in vals),
                                         ' | '.join(delta(vals[0], v, d) for v in vals[1:])))
    return o + ['']


def main(argv):
    scene, counters, paths = 'terrain', DEFAULT_COUNTERS, []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == '--scene' and i + 1 < len(argv):
            scene, i = argv[i + 1], i + 2
        elif a == '--counters' and i + 1 < len(argv):
            counters, i = [c for c in argv[i + 1].split(',') if c], i + 2
        elif a.startswith('-'):
            print(__doc__)
            return 2
        else:
            paths.append(a)
            i += 1
    if len(paths) < 2:
        print(__doc__)
        return 2
    names = []
    for p in paths:
        nm = os.path.basename(p.rstrip('/')) or p
        names.append(nm + '#%d' % len(names) if nm in names else nm)
    rs = [parse_scene(p, scene) for p in paths]
    o = ['# compare: %s' % scene, '']
    for nm, r in zip(names, rs):
        o.append('- %s: commit %s, %s ms a frame' % (nm, r.get('commit', 'absent'), r.get('ms', 'absent')))
    o.append('')
    o += jobs_section(names, rs)
    o += table('frame line (us)', [(k, [r.get('us_' + k) for r in rs])
                                   for k in ('gl', 'swap', 'frame')], names)
    pf = [P.per_frame(r) for r in rs]
    wait = [(p['wait.ns'] / 1e6 if isinstance(p.get('wait.ns'), (int, float)) else None) for p in pf]
    o += table('wait.ns (ms a frame)', [('wait.ns', wait)], names, 3)
    o += table('counters (a frame)', [(k, [p.get(k) for p in pf]) for k in counters], names, 3)
    print('\n'.join(o))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
