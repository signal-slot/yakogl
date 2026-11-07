#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Terrain's board-run fields from a records directory or a console log.

  python3 -I parse.py <records dir | console log> [<later one>]
  python3 -I parse.py --selftest

Prints a markdown block (commit, verdict line, records path, terrain FPS and
ms a frame, the glmark2-frame line, and the board's counters). With two
arguments it adds the difference (second minus first). README.md has the
formats. Standard library only; reads files, writes nothing.
"""
import os
import re
import sys

TS = re.compile(r'^\d\d:\d\d:\d\d ')
RUN = re.compile(r'^(\d{8}-\d{4})-(.+)-([0-9a-f]{8}|probe)$')
FPS = re.compile(r'\[terrain\][^:]*: FPS: (\d+) FrameTime: ([\d.]+) ms')
FRAME = re.compile(r'glmark2-frame: \[terrain\].*')
TFU = re.compile(r'yakogl/v3d: TFU mipmap at frame (\d+): (\d+) chains')
TILED = re.compile(r'yakogl/v3d: tiled copies for draws at frame (\d+), by reason: (.*)')
REPOST = re.compile(r'render: fence (\d+) .*re-posted (\d+) kick\(s\) \((\d+) so far\)')
KICK = re.compile(r'pvr last completed kick per frame.*')
VERDICT = re.compile(r'^(?:verdict: )?(board\.sh \S+ \S+ rc=\d+ verdict=.*)$')


def lines_of(path):
    with open(path, errors='replace') as f:
        return [TS.sub('', l.rstrip('\n')) for l in f]


def find_runs(path):
    """(run dir or None, console path) for a log, a run dir or a stage dir."""
    if os.path.isfile(path):
        return [(None, path)]
    if os.path.isfile(os.path.join(path, 'console.txt')):
        return [(path, os.path.join(path, 'console.txt'))]
    out = []
    for d in sorted(os.listdir(path)):
        c = os.path.join(path, d, 'console.txt')
        if os.path.isfile(c):
            out.append((os.path.join(path, d), c))
    return out


def parse_one(rundir, console):
    L = lines_of(console)
    r = {'console': console, 'run': rundir}
    for l in L:
        m = FPS.search(l)
        if m:
            r['fps'], r['ms'] = int(m.group(1)), float(m.group(2))
        m = FRAME.search(l)
        if m:
            r['frame_line'] = m.group(0)
            for k in ('gl', 'swap', 'show', 'other', 'frame'):
                mm = re.search(r'\b%s (\d+)' % k, m.group(0))
                if mm:
                    r['us_' + k] = int(mm.group(1))
            mm = re.search(r'end waited for in (\d+) of (\d+)', m.group(0))
            if mm:
                r['end_waited'] = '%s of %s' % mm.groups()
        m = TFU.search(l)
        if m and int(m.group(1)) >= r.get('tfu_frame', 0):
            r['tfu_frame'], r['tfu_chains'] = int(m.group(1)), int(m.group(2))
            r['tfu_line'] = l[l.index('yakogl/v3d'):]
        m = TILED.search(l)
        if m and int(m.group(1)) >= r.get('tiled_frame', 0):
            r['tiled_frame'] = int(m.group(1))
            r['tiled_line'] = l[l.index('yakogl/v3d'):]
            r['tiled'] = {k: int(v) for k, v in re.findall(r'([\w-]+) (\d+)', m.group(2))}
        m = REPOST.search(l)
        if m:
            r['reposts'] = r.get('reposts', 0) + 1
            r['reposted_kicks'] = r.get('reposted_kicks', 0) + int(m.group(2))
            r['reposts_so_far'] = int(m.group(3))
        m = KICK.search(l)
        if m:
            r['kick_line'] = m.group(0)
    if rundir:
        m = RUN.match(os.path.basename(rundir.rstrip('/')))
        if m:
            r['commit'] = m.group(3)
        idx = os.path.join(rundir, 'INDEX.txt')
        if os.path.isfile(idx):
            for l in lines_of(idx):
                m = VERDICT.match(l)
                if m:
                    r['verdict'] = m.group(1)
        notes = os.path.join(os.path.dirname(rundir.rstrip('/')), 'stage-notes.txt')
        if os.path.isfile(notes):
            for l in lines_of(notes):
                m = re.match(r'commit ([0-9a-f]{8})', l)
                if m:
                    r['commit'] = m.group(1)
    if 'verdict' not in r:
        for l in L:
            m = VERDICT.match(l)
            if m:
                r['verdict'] = m.group(1)
    return r


def parse(path):
    runs = [parse_one(d, c) for d, c in find_runs(path)]
    runs = [r for r in runs if 'fps' in r or 'frame_line' in r]
    if not runs:
        raise SystemExit('parse.py: no terrain line in %s' % path)
    return runs[-1]  # the last run that has terrain


def block(r, title):
    def g(k, d='absent'):
        return r.get(k, d)
    o = ['### %s' % title, '']
    o.append('- commit: %s' % g('commit'))
    o.append('- verdict line: `%s`' % g('verdict'))
    o.append('- records path: %s' % (g('run') if r.get('run') else '(console log %s)' % r['console']))
    o.append('- terrain: %s FPS, %s ms a frame' % (g('fps'), g('ms')))
    o.append('- frame line: `%s`' % g('frame_line'))
    if 'tfu_line' in r or 'tiled_line' in r:
        o.append('- Pi 4 TFU mipmap (frame %s): %s chains' % (g('tfu_frame'), g('tfu_chains')))
        o.append('  - `%s`' % g('tfu_line'))
        o.append('  - `%s`' % g('tiled_line'))
    if 'reposts' in r or 'kick_line' in r:
        o.append('- AM62P re-post lines: %s (kicks re-posted %s, last "so far" %s)'
                 % (g('reposts', 0), g('reposted_kicks', 0), g('reposts_so_far', 0)))
        o.append('  - `%s`' % g('kick_line'))
    return '\n'.join(o) + '\n'


NUM = ['fps', 'ms', 'us_gl', 'us_swap', 'us_show', 'us_other', 'us_frame',
       'tfu_chains', 'reposts', 'reposted_kicks']


def diff(a, b):
    o = ['### difference (second - first)', '',
         '| field | first | second | change |', '|---|---|---|---|']
    for k in NUM:
        if k in a or k in b:
            x, y = a.get(k), b.get(k)
            ch = '%+.3f' % (y - x) if x is not None and y is not None else 'n/a'
            o.append('| %s | %s | %s | %s |' % (k, x, y, ch))
    for k in sorted(set(a.get('tiled', {})) | set(b.get('tiled', {}))):
        x, y = a.get('tiled', {}).get(k), b.get('tiled', {}).get(k)
        ch = '%+d' % (y - x) if x is not None and y is not None else 'n/a'
        o.append('| tiled %s | %s | %s | %s |' % (k, x, y, ch))
    return '\n'.join(o) + '\n'


def selftest():
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'testdata')
    p = parse(os.path.join(here, 'pi4'))
    a = parse(os.path.join(here, 'pi4-after'))
    m = parse(os.path.join(here, 'am62p'))
    chk = [
        (p['commit'], '0123abcd'), (p['fps'], 6), (p['ms'], 161.8),
        (p['us_swap'], 154200), (p['tfu_frame'], 461), (p['tfu_chains'], 58),
        (p['tiled']['first-sample'], 77), (p['verdict'].split()[4], 'verdict=done'),
        (a['ms'], 33.3), (a['tfu_frame'], 461),
        (m['reposts'], 2), (m['reposts_so_far'], 2), (m['fps'], 12),
        ('last completed' in m['kick_line'], True), ('reposts' in p, False),
    ]
    bad = [c for c in chk if c[0] != c[1]]
    d = diff(p, a)
    if '-128.500' not in d:
        bad.append(('diff', d))
    for s in (block(p, 'x'), block(m, 'y')):
        if 'records path' not in s:
            bad.append(('block', s))
    if bad:
        print('selftest FAILED: %r' % (bad,))
        return 1
    print('selftest ok: %d checks' % (len(chk) + 3))
    return 0


def main(argv):
    if argv == ['--selftest']:
        return selftest()
    if len(argv) not in (1, 2) or argv[0].startswith('-'):
        print(__doc__)
        return 2
    rs = [parse(x) for x in argv]
    for x, r in zip(argv, rs):
        print(block(r, os.path.basename(x.rstrip('/')) or x))
    if len(rs) == 2:
        print(diff(*rs))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
