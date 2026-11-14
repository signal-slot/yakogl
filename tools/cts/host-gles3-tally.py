#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""HOST-ONLY tip table and counting not-pass list of three host-deqp runs.

  python3 -I tools/cts/host-gles3-tally.py RUN RUN RUN
        [--tip FILE] [--notpass FILE]
  python3 -I tools/cts/host-gles3-tally.py --selftest

Each RUN is a run directory tools/cts/host-deqp-run.sh wrote (README 2b):
c_*.qpa (a resumed c_NNN.rK.qpa too), all.txt and the commit file
"<sha> <gpu> <module> ...". The three runs must be GPU powervr, vivante and
v3d (one each, read from the commit file) of ONE commit; runs of different
commits are refused. The .qpa parsing is copied from deqp-limits-tally.py
(stdlib only; under -I the script's directory is not on sys.path, so nothing
is imported from it).

Output (a), --tip FILE (default stdout): the per-group table in the format of
docs/es3/host-gles3-tip.tsv: a '# host dEQP-GLES3 (MODULE=m, N cases) at
<commit>, HOST-ONLY: API, compile and refusal verdicts only, render verdicts
meaningless' line, then the tab separated columns
'gpu group pass fail notsupported crash other', rows sorted by gpu (powervr,
v3d, vivante) then group. group = the first two path parts after dEQPGLESn.
(functional.buffer, info.version). pass = Pass; crash = Crash plus a case
that began and was cut off or never began; other = any other status
(QualityWarning, Timeout, ResourceError, ...).

Output (b), --notpass FILE: the counting not-pass list, a '#' header line then
'gpu case status kind message rc owner' (tab separated; rc and owner are left
empty for the triage). kind:
  refusal   a back end or the GL core refused by name ('online shaderc',
            'refused', 'not supported', 'unimplemented'; a NotSupported case)
  gl-error  a wrong GL error ('Got invalid error', 'Unexpected GL error',
            'Texture upload failed: GL_...')
  crash     Crash, or no result in a chunk that died (Incomplete, NotRun)
  compile   'Compile failed', 'Failed to compile'
A Fail whose message matches none of these is a render/value verdict and is
not listed (render verdicts are meaningless on the host); nor are Pass and
QualityWarning.

Exit 0 ok, 1 a refused input, 2 a bad command line.
"""
import argparse
import glob
import os
import re
import shutil
import sys
import tempfile
from xml.sax.saxutils import unescape

CASE_RE = re.compile(r'^dEQP-GLES[23]\.functional\.')
BEGIN = re.compile(r'^#beginTestCaseResult (\S+)')
QPA_OPEN = re.compile(r'<TestCaseResult\b[^>]*\bCasePath="([^"]+)"')
RESULT = re.compile(r'<Result\b[^>]*\bStatusCode="([^"]*)"([^>]*)>')
RESULT_END = '</Result>'
END = re.compile(r'^#endTestCaseResult')
TERMINATE = re.compile(r'^#terminateTestCaseResult (\S+)')
ENTITIES = {"&quot;": '"', "&apos;": "'"}

GPUS = ('powervr', 'v3d', 'vivante')
GROUP_RE = re.compile(r'^dEQP-GLES[0-9]+\.([^.]+(?:\.[^.]+)?)')

KINDS = (
    ('compile', re.compile(r'Compile failed|Failed to compile', re.I)),
    ('gl-error', re.compile(r'Got invalid error|Unexpected (?:GL )?error|'
                            r'invalid error|Texture upload failed: GL_|'
                            r'Expected (?:GL )?error|glGetError\(\) returned|'
                            r'\bGL_(?:INVALID|OUT_OF)_[A-Z_]+', re.I)),
    ('refusal', re.compile(r'online shaderc|refused|not supported|unsupported|'
                           r'unimplemented|not implemented', re.I)),
)


class TallyError(Exception):
    """A refused input; main turns it into exit 1."""


def message(msg):
    """One dEQP status message, whole: entities decoded, blank lines and the
    line breaks dropped, the parts joined by ' / ' (a compile refusal names
    the reason on the line after its first). None stays None."""
    if msg is None:
        return None
    parts = [p.strip() for p in unescape(msg, ENTITIES).splitlines() if p.strip()]
    return ' / '.join(parts)


def read_qpa(path, began, res):
    """One .qpa into began (set of cases its logs opened) and res (case ->
    (status, message)). Reads #beginTestCaseResult markers and TestCaseResult
    blocks (either opens the case), a <Result StatusCode="..."> whose text may
    span lines, a self-closed Result, #terminateTestCaseResult inside an open
    result-less block (its status word is the case's result), and a block
    still open at the file's end. A case that began is in began even with no
    Result; the last Result or terminate status of a case wins (a resumed
    chunk's retry)."""
    case = status = msg = None
    in_msg = False

    def close():
        nonlocal case, status, msg
        if case is not None and status is not None:
            res[case] = (status, message(msg))
        case = status = msg = None

    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            line = line.rstrip('\n')
            if in_msg and (QPA_OPEN.search(line) or BEGIN.match(line)
                           or TERMINATE.match(line)):
                in_msg = False   # the Result never closed: the case was cut off
            if in_msg:
                msg += '\n' + line.split(RESULT_END, 1)[0]
                if RESULT_END in line:
                    in_msg = False
                continue
            o, b = QPA_OPEN.search(line), BEGIN.match(line)
            if o or b:
                name = (o or b).group(1)
                if name != case:      # '#beginTestCaseResult X' precedes X's own block
                    close()
                    case, status, msg = name, None, None
                began.add(name)
                continue
            m = RESULT.search(line)
            if m and case is not None:
                status = m.group(1)
                tail = line[m.end():]
                if m.group(2).rstrip().endswith('/'):
                    msg = None                      # <Result ... />: status, no message
                elif RESULT_END in tail:
                    msg = tail.split(RESULT_END, 1)[0]
                else:
                    msg, in_msg = tail, True        # keep the first line: 'Compile failed:'
            t = TERMINATE.match(line)
            if t:
                if case is not None and status is None:
                    status, msg = t.group(1), None
                close()
            elif END.match(line):
                close()
    close()


def load_run(path):
    """One host-deqp run directory into {dir, gpu, module, commit, asked,
    began, res}. A directory without c_*.qpa, or a run with no parsed result,
    is refused: an empty run is not an outcome."""
    run = {'dir': path, 'gpu': None, 'module': None, 'commit': None,
           'asked': None, 'began': set(), 'res': {}}
    if not os.path.isdir(path):
        raise TallyError('no such run directory: %s' % path)
    logs = sorted(glob.glob(os.path.join(path, 'c_*.qpa')))
    if not logs:
        raise TallyError('%s is not a run directory (no c_*.qpa): pass a directory '
                         'tools/cts/host-deqp-run.sh wrote (tools/cts/README.md 2b)' % path)
    cf = os.path.join(path, 'commit')
    if os.path.isfile(cf):
        with open(cf, encoding='utf-8', errors='replace') as f:
            tok = f.readline().split()
        if len(tok) >= 3:
            run['commit'], run['gpu'], run['module'] = tok[0], tok[1], tok[2]
    af = os.path.join(path, 'all.txt')
    if os.path.isfile(af):
        with open(af, encoding='utf-8', errors='replace') as f:
            run['asked'] = [ln.strip() for ln in f if ln.strip() and not ln.startswith('#')]
    for lg in logs:
        read_qpa(lg, run['began'], run['res'])
    if not run['res']:
        raise TallyError('%s: no <Result StatusCode= parsed from %s: an empty run or an '
                         'unreadable log, not an outcome'
                         % (path, ' '.join(os.path.basename(p) for p in logs[:4])))
    return run


def universe(run):
    return set(run['asked'] or ()) | run['began'] | set(run['res'])


def case_status(run, case):
    """Its logged status; the synthetic Incomplete for a case that began and
    was cut off; None for a case its logs do not hold."""
    if case in run['res']:
        return run['res'][case][0]
    return 'Incomplete' if case in run['began'] else None


def group_name(case):
    m = GROUP_RE.match(case)
    return m.group(1) if m else case


def same_commit(a, b):
    return a.startswith(b) or b.startswith(a)


def load_three(paths):
    runs = {}
    for p in paths:
        r = load_run(p)
        if not r['gpu'] or not r['commit']:
            raise TallyError('%s: no commit file ("<sha> <gpu> <module>")' % p)
        if r['gpu'] not in GPUS:
            raise TallyError('%s: gpu %r is not one of %s' % (p, r['gpu'], ' '.join(GPUS)))
        if r['gpu'] in runs:
            raise TallyError('%s: a second run of gpu %s' % (p, r['gpu']))
        runs[r['gpu']] = r
    if sorted(runs) != sorted(GPUS):
        raise TallyError('need one run each of %s, got %s' % (' '.join(GPUS), ' '.join(sorted(runs))))
    first = runs['powervr']
    for g in GPUS:
        if not same_commit(runs[g]['commit'], first['commit']):
            raise TallyError('runs of different commits: powervr %s, %s %s'
                             % (first['commit'], g, runs[g]['commit']))
        if runs[g]['module'] != first['module']:
            raise TallyError('runs of different modules: %s vs %s' % (first['module'], runs[g]['module']))
    return runs


def commit_of(runs):
    return max((runs[g]['commit'] for g in GPUS), key=len)[:12]


def tip_lines(runs):
    first = runs['powervr']
    out = ['# host dEQP-GLES3 (MODULE=%s, %d cases) at %s, HOST-ONLY: API, compile and '
           'refusal verdicts only, render verdicts meaningless'
           % (first['module'], len(universe(first)), commit_of(runs)),
           'gpu\tgroup\tpass\tfail\tnotsupported\tcrash\tother']
    for g in GPUS:
        run = runs[g]
        tab = {}
        for c in universe(run):
            st = case_status(run, c) or 'NotRun'
            row = tab.setdefault(group_name(c), [0, 0, 0, 0, 0])
            if st == 'Pass':
                row[0] += 1
            elif st == 'Fail':
                row[1] += 1
            elif st == 'NotSupported':
                row[2] += 1
            elif st in ('Crash', 'Incomplete', 'NotRun'):
                row[3] += 1
            else:
                row[4] += 1
        for grp in sorted(tab):
            out.append('\t'.join([g, grp] + [str(n) for n in tab[grp]]))
    return out


def classify(status, msg):
    """The kind of one non-Pass case, or None when it is not counted."""
    if status in ('Crash', 'Incomplete', 'NotRun'):
        return 'crash'
    if status in ('Pass', 'QualityWarning', 'CompatibilityWarning'):
        return None
    for kind, rx in KINDS:
        if rx.search(msg):
            return kind
    if status == 'NotSupported':
        return 'refusal'
    if status in ('Timeout', 'ResourceError', 'InternalError'):
        return 'crash'
    return None


def notpass_lines(runs):
    out = ['# host dEQP-GLES3 counting not-pass cases at %s, HOST-ONLY: refusals, wrong GL errors, '
           'crashes and compile failures; render verdicts are not listed' % commit_of(runs),
           'gpu\tcase\tstatus\tkind\tmessage\trc\towner']
    for g in GPUS:
        run = runs[g]
        for c in sorted(universe(run)):
            st = case_status(run, c) or 'NotRun'
            if st == 'Incomplete':
                msg = 'began and was cut off (no <Result StatusCode=)'
            elif st == 'NotRun':
                msg = 'never began'
            else:
                msg = run['res'][c][1] or ''
            kind = classify(st, msg)
            if kind is None:
                continue
            msg = msg.replace('\t', ' ').replace('\n', ' ')
            out.append('\t'.join([g, c, st, kind, msg, '', '']))
    return out


def selftest():
    tmp = tempfile.mkdtemp(prefix='hostgles3tally.')

    def mk(gpu, sha, blocks, asked):
        d = os.path.join(tmp, gpu + sha)
        os.mkdir(d)
        with open(os.path.join(d, 'commit'), 'w') as f:
            f.write('%s %s gles3\n' % (sha, gpu))
        with open(os.path.join(d, 'all.txt'), 'w') as f:
            f.write('\n'.join(asked) + '\n')
        with open(os.path.join(d, 'c_000.qpa'), 'w') as f:
            for case, status, msg in blocks:
                f.write('#beginTestCaseResult %s\n' % case)
                if status is not None:
                    f.write('<Result StatusCode="%s">%s</Result>\n' % (status, msg))
                    f.write('#endTestCaseResult\n')
        return d

    try:
        P = 'dEQP-GLES3.functional.'
        cases = [P + 'buffer.a', P + 'buffer.b', P + 'shader_api.program_binary.x',
                 P + 'negative_api.y', P + 'fbo.z', P + 'fbo.q', P + 'texture.t',
                 'dEQP-GLES3.info.version', P + 'fbo.multi']
        blocks = [
            (cases[0], 'Pass', ''),
            (cases[1], 'Fail', 'Image comparison failed'),
            (cases[2], 'NotSupported', 'Program binaries not supported'),
            (cases[3], 'Fail', 'Got invalid error: GL_INVALID_ENUM'),
            (cases[4], 'Fail', 'Compile failed:\nFailed to compile shader program'),
            (cases[5], 'Fail', 'online shaderc: the code generator refused this fragment shader'),
            (cases[6], None, ''),   # began, cut off
            (cases[7], 'QualityWarning', 'meh'),
            (cases[8], 'Fail', 'Expected error but got none'),
        ]
        paths = [mk(g, 'abc123def456', blocks, cases) for g in GPUS]
        runs = load_three(paths)
        tip = tip_lines(runs)
        assert tip[0] == ('# host dEQP-GLES3 (MODULE=gles3, 9 cases) at abc123def456, HOST-ONLY: '
                          'API, compile and refusal verdicts only, render verdicts meaningless'), tip[0]
        assert tip[1] == 'gpu\tgroup\tpass\tfail\tnotsupported\tcrash\tother', tip[1]
        assert len(tip) == 2 + 3 * 6, len(tip)
        rows = [ln.split('\t') for ln in tip[2:]]
        assert [r[0] for r in rows] == ['powervr'] * 6 + ['v3d'] * 6 + ['vivante'] * 6
        want = {'functional.buffer': '1 1 0 0 0', 'functional.shader_api': '0 0 1 0 0',
                'functional.negative_api': '0 1 0 0 0', 'functional.fbo': '0 3 0 0 0',
                'functional.texture': '0 0 0 1 0', 'info.version': '0 0 0 0 1'}
        for r in rows[:6]:
            assert ' '.join(r[2:]) == want[r[1]], r
        assert [r[1] for r in rows[:6]] == sorted(want)
        body = [ln.split('\t') for ln in notpass_lines(runs)[2:]]
        assert notpass_lines(runs)[1] == 'gpu\tcase\tstatus\tkind\tmessage\trc\towner'
        assert len(body) == 6 * 3, len(body)
        kinds = {r[1]: r[3] for r in body if r[0] == 'v3d'}
        assert kinds == {cases[2]: 'refusal', cases[3]: 'gl-error', cases[4]: 'compile',
                         cases[5]: 'refusal', cases[6]: 'crash', cases[8]: 'gl-error'}, kinds
        assert all(len(r) == 7 and r[5] == '' and r[6] == '' for r in body)
        msgs = {r[1]: r[4] for r in body if r[0] == 'v3d'}
        assert msgs[cases[4]] == 'Compile failed: / Failed to compile shader program', msgs
        bad = [paths[0], paths[2], mk('v3d', 'fff000fff000', blocks, cases)]
        try:
            load_three(bad)
            raise AssertionError('different commits accepted')
        except TallyError as e:
            assert 'different commits' in str(e), e
        try:
            load_three(paths[:2])
            raise AssertionError('two runs accepted')
        except TallyError:
            pass
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print('selftest ok')


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('runs', nargs='*', help='the three run directories (powervr, vivante, v3d)')
    ap.add_argument('--tip', help='write the tip table here (default stdout)')
    ap.add_argument('--notpass', help='write the counting not-pass list here')
    ap.add_argument('--selftest', action='store_true')
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return 0
    if len(a.runs) != 3:
        ap.error('give exactly three run directories')
    try:
        runs = load_three(a.runs)
    except TallyError as e:
        print('host-gles3-tally: %s' % e, file=sys.stderr)
        return 1
    tip = '\n'.join(tip_lines(runs)) + '\n'
    if a.tip:
        with open(a.tip, 'w', encoding='utf-8') as f:
            f.write(tip)
    else:
        sys.stdout.write(tip)
    if a.notpass:
        with open(a.notpass, 'w', encoding='utf-8') as f:
            f.write('\n'.join(notpass_lines(runs)) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
