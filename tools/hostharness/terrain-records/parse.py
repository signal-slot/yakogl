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
TERRAIN = re.compile(r'\[terrain\]')
FPS1 = re.compile(r'\[terrain\][^:]*: FPS: (\d+) FrameTime: ([\d.]+) ms')
FPS = re.compile(r'\bFPS: (\d+) FrameTime: ([\d.]+) ms')
OTHER_SCENE = re.compile(r'^\[[\w-]+\] ')
BUILD = re.compile(r'sample: build tree=([0-9a-f]{7,40})(-dirty)?\b')
LABEL = re.compile(r'^board\.sh \S+ glmark2-([0-9a-f]{8})\b')
COUNTERS = re.compile(r'glmark2-counters: (.*?): (absent|frames (\d+)\b(.*))$')
# Cumulative differences are printed per frame; these are levels or causes.
LEVEL_KEYS = {'rt.live', 'rt.bytes', 'tile.untimed', 'fl.stop'}
# "total T render R[~] idle I[ tfu K/Tus][ bin B prep P]": the bin and prep
# pair is last, printed by newer GM images; older images do not print it.
JOBS = re.compile(r'glmark2-jobs: (.*?): frame (\d+): (\d+) job\(s\):(.*)$')
JOB = re.compile(r'\s*total (\d+) render (\d+)(~?) idle (\d+)(?: tfu (\d+)/(\d+)us)?'
                 r'(?: bin (\d+) prep (\d+))?\s*$')
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
    pending = False  # a [terrain] line seen, its FPS line not yet
    for l in L:
        if TERRAIN.search(l) and 'glmark2-frame' not in l and 'glmark2-counters' not in l:
            pending = True
        elif pending and OTHER_SCENE.match(l):
            pending = False
        m = FPS1.search(l) or (FPS.search(l) if pending else None)
        if m:
            r['fps'], r['ms'] = int(m.group(1)), float(m.group(2))
            pending = False
        m = BUILD.search(l)
        if m:
            r['build'] = m.group(1)[:8] + (m.group(2) or '')
        m = COUNTERS.search(l)
        if m and TERRAIN.search(m.group(1)):
            if m.group(2) == 'absent':
                r['counters'] = None
            else:
                n = int(m.group(3))
                w = m.group(4).split()
                c = {}
                for k, v in zip(w[0::2], w[1::2]):
                    try:
                        c[k] = float(v) if re.search(r'[.eE]', v) else int(v)
                    except ValueError:
                        c[k] = v
                r['counters'] = c
                r['counters_frames'] = n
        m = JOBS.search(l)
        if m and TERRAIN.search(m.group(1)):
            jobs = []
            for part in m.group(4).split(';'):
                j = JOB.match(part)
                if not j:
                    jobs = None
                    break
                g = j.groups()
                jobs.append({'total': int(g[0]), 'render': int(g[1]), 'late': g[2] == '~',
                             'idle': int(g[3]), 'tfu_jobs': int(g[4] or 0), 'tfu': int(g[5] or 0),
                             'bin': int(g[6]) if g[6] else None,
                             'prep': int(g[7]) if g[7] else None})
            if jobs:
                r['jobs_frame'], r['jobs'] = int(m.group(2)), jobs
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
            r['image_sha256'] = m.group(3)
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
                if m and 'build' not in r:
                    r['commit'] = m.group(1)
    if 'verdict' not in r:
        for l in L:
            m = VERDICT.match(l)
            if m:
                r['verdict'] = m.group(1)
    # commit: the console's build line, else a stage note, else the label in
    # the verdict line ('board.sh pi4 glmark2-<sha8>'); never the sha256.
    if 'build' in r:
        r['commit'] = r['build']
    elif 'commit' not in r and 'verdict' in r:
        m = LABEL.match(r['verdict'])
        if m:
            r['commit'] = m.group(1)
    return r


def parse(path):
    runs = [parse_one(d, c) for d, c in find_runs(path)]
    runs = [r for r in runs if 'fps' in r or 'frame_line' in r]
    if not runs:
        raise SystemExit('parse.py: no terrain line in %s' % path)
    return runs[-1]  # the last run that has terrain


def per_frame(r):
    """Terrain's counters as {key: value}, rates divided by frames."""
    c, n = r.get('counters'), r.get('counters_frames', 0)
    if not c:
        return {}
    out = {}
    for k, v in c.items():
        if isinstance(v, str) or k in LEVEL_KEYS or n == 0:
            out[k] = v
        else:
            out[k] = round(v / n, 3)
            if out[k] == int(out[k]):
                out[k] = int(out[k])
    return out


def counters_text(r):
    c = r.get('counters')
    if c is None:
        return '- glmark2-counters (terrain, a frame): absent'
    pf = per_frame(r)
    return '- glmark2-counters (terrain, %d frames, a frame): %s' % (
        r['counters_frames'], ' '.join('%s %s' % kv for kv in pf.items()) or 'none')


def block(r, title):
    def g(k, d='absent'):
        return r.get(k, d)
    o = ['### %s' % title, '']
    o.append('- commit: %s' % g('commit'))
    if r.get('image_sha256'):
        o.append('- image sha256 (run name): %s' % r['image_sha256'])
    o.append('- verdict line: `%s`' % g('verdict'))
    o.append('- records path: %s' % (g('run') if r.get('run') else '(console log %s)' % r['console']))
    o.append('- terrain: %s FPS, %s ms a frame' % (g('fps'), g('ms')))
    o.append('- frame line: `%s`' % g('frame_line'))
    if 'tfu_line' in r or 'tiled_line' in r:
        o.append('- Pi 4 TFU mipmap (frame %s): %s chains' % (g('tfu_frame'), g('tfu_chains')))
        o.append('  - `%s`' % g('tfu_line'))
        o.append('  - `%s`' % g('tiled_line'))
    o.append(counters_text(r))
    if 'jobs' in r:
        js = r['jobs']
        bp = [j for j in js if j['bin'] is not None]
        o.append('- glmark2-jobs (terrain, frame %d): %d jobs, render %d us, idle %d us'
                 % (r['jobs_frame'], len(js), sum(j['render'] for j in js), sum(j['idle'] for j in js))
                 + ('; bin %d us, prep %d us' % (sum(j['bin'] for j in bp), sum(j['prep'] for j in bp))
                    if bp else '; bin and prep absent (old format)'))
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
    pa, pb = per_frame(a), per_frame(b)
    for k in sorted(set(pa) | set(pb)):
        x, y = pa.get(k), pb.get(k)
        ok = all(isinstance(v, (int, float)) for v in (x, y))
        o.append('| counter %s (a frame) | %s | %s | %s |'
                 % (k, x, y, '%+.3f' % (y - x) if ok else 'n/a'))
    return '\n'.join(o) + '\n'


def parse_text(lines):
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        f = os.path.join(d, 'console.txt')
        with open(f, 'w') as fh:
            fh.write('\n'.join(lines) + '\n')
        return parse_one(None, f)


def selftest():
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'testdata')
    p = parse(os.path.join(here, 'pi4'))
    a = parse(os.path.join(here, 'pi4-after'))
    m = parse(os.path.join(here, 'am62p'))
    chk = [
        (p['commit'], 'a1b2c3d4'), (p['image_sha256'], '0123abcd'),
        (a['commit'], 'e5f6a7b8-dirty'), (m['commit'], '9a8b7c6d'),
        (p['counters']['jobs'], 549), (p['counters_frames'], 61),
        (per_frame(p)['jobs'], 9), (per_frame(p)['rt.live'], 4),
        (per_frame(m)['kicks'], 52.672), (m['counters']['reposts'], 61),
        (parse_text(['[terrain] <d>:', 'x', ' FPS: 5 FrameTime: 9.5 ms'])['fps'], 5),
        (parse_text(['[terrain] <d>: FPS: 5 FrameTime: 9.5 ms'])['ms'], 9.5),
        (parse_text(['glmark2-counters: [terrain] <d>: absent'])['counters'], None),
        (parse_text(['glmark2-counters: [shadow] <d>: frames 2 jobs 4']).get('counters', 'none'), 'none'),
        ('image sha256' in block(p, 'x'), True),
        (p['fps'], 6), (p['ms'], 161.8),
        (p['us_swap'], 154200), (p['tfu_frame'], 461), (p['tfu_chains'], 58),
        (p['tiled']['first-sample'], 77), (p['verdict'].split()[4], 'verdict=done'),
        (a['ms'], 33.3), (a['tfu_frame'], 461),
        (m['reposts'], 2), (m['reposts_so_far'], 2), (m['fps'], 12),
        ('last completed' in m['kick_line'], True), ('reposts' in p, False),
    ]
    old = parse_text(['glmark2-jobs: [terrain] <d>: frame 10: 2 job(s): total 900 render 800~ idle 5 tfu 2/60us;'
                      ' total 700 render 600 idle 0'])
    new = parse_text(['glmark2-jobs: [terrain] <d>: frame 10: 2 job(s): total 900 render 800~ idle 5 tfu 2/60us'
                      ' bin 120 prep 30; total 700 render 600 idle 0 bin 90 prep 25'])
    chk += [
        (len(old['jobs']), 2), (old['jobs'][0]['tfu'], 60), (old['jobs'][0]['late'], True),
        (old['jobs'][0]['bin'], None), (old['jobs'][1]['render'], 600),
        (new['jobs'][0]['bin'], 120), (new['jobs'][0]['prep'], 30), (new['jobs'][0]['tfu_jobs'], 2),
        (new['jobs'][1]['bin'], 90), (new['jobs'][1]['prep'], 25), (new['jobs_frame'], 10),
        ('bin 210 us, prep 55 us' in block(new, 'n'), True), ('old format' in block(old, 'o'), True),
        (parse_text(['glmark2-jobs: [shadow] <d>: frame 10: 1 job(s): total 1 render 1 idle 0']).get('jobs'), None),
        (parse_text(['glmark2-jobs: [terrain] <d>: frame 10: absent (the scene ran fewer frames)']).get('jobs'), None),
        (p['jobs'][0]['bin'], None), (a['jobs'][0]['bin'], 1200),
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
