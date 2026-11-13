#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tally of the dEQP limit, state-query, ubo and varying groups in a host-deqp run.

  python3 -I tools/cts/deqp-limits-tally.py RUN [RUN ...]
  python3 -I tools/cts/deqp-limits-tally.py --compare OLD NEW
  python3 -I tools/cts/deqp-limits-tally.py --selftest

RUN/OLD/NEW is a run directory tools/cts/host-deqp-run.sh wrote
(tools/cts/README.md 2b): its c_*.qpa logs (a resumed chunk's c_NNN.rK.qpa
too), the case list all.txt it ran, and its commit file ("<sha> <gpu>
<module> [native-3d=v] [lib-cmake-args=...]"); a single .qpa file is
accepted too. GPU and module are read from the commit file. summary.txt and
failures.tsv (cts-summary.py's output, which needs the VK-GL-CTS log
parser) are not read: this parses the .qpa files itself, stdlib only, so
the tallies reproduce on any machine, with or without a CTS checkout.

Tallied groups (a case is dEQP-GLES[23].functional.<group>.<rest>, matched
at a group-name boundary, so state_query.integers never swallows
state_query.integers64), one row each:

  implementation_limits  state_query.integers  state_query.integers64
  state_query.floats  ubo  shaders.linkage.varying  shaders.builtin_variable

The last two are the plan's varying shader groups: the ES 3.0 linkage.varying
cases and the gl_MaxVertexOutputVectors / gl_MaxFragmentInputVectors
builtin_variable cases. --group G adds one more (repeatable; G with or
without the dEQP-GLESx.functional. prefix); the defaults are extended,
never replaced. A group row counts the cases asked for (all.txt) plus any
the logs hold, by status: Pass, Fail, NotSupported, Other (any other status
dEQP wrote: QualityWarning, Crash, Timeout, ResourceError, ...), NoResult
(a case that began and was cut off with no <Result StatusCode= -- the
runner's Incomplete -- or never began). Below the table: the whole run's
status counts, and the cases outside the tallied groups. Every number is
HOST-ONLY (the linux OSAL reads black, host-deqp-run.sh's header): what the
tallied groups decide here is a GL query or a compile/link outcome, which
the host does answer -- never quote a render verdict from these runs.

After the table: every row whose case name contains one of the plan's limit
words (--limit WORD, repeatable; default: max_varying_vectors,
max_varying_components, max_fragment_input_components, max_vertex_output_
components, max_uniform_block_size, max_combined_vertex_uniform_components
and max_combined_fragment_uniform_components), with its message: the Result
element's whole text, dEQP's entities decoded, its lines joined by ' / ' (a
compile refusal names the reason on the line after its first -- cts-summary
.py's lesson). The Fail rows first,
then the same cases in any other non-Pass status (Incomplete, NotRun,
Crash, ...).

--compare OLD NEW prints both runs' full blocks, then one line per case
whose status differs ('case: old -> new', '-' for a case that run's logs do
not hold -- host-deqp-run.sh --compare's reading), then the per-status
totals of both runs and the changed count. A side without a single parsed
result is refused, never a silent 'no changes' (host-deqp-run.sh's
compare_runs rule).

Exit 0: the tallies printed. 1: a run directory without c_*.qpa, a run with
no result at all, a --compare side without a single result. 2: a bad
command line.

--require-pass limits is a mechanical acceptance check instead of the tally:
for each RUN it selects exactly the plan's limit cases -- every case of
gles3-mustpass.txt (next to this script; the names come from that file) in
implementation_limits, state_query.integers, integers64, floats or booleans
whose last name part is one of the seven limit words (the default --limit
list), bare or with a _get<form> suffix (_getboolean, _getfloat,
_getinteger, _getinteger64) -- and prints one line per case, 'status case'
('Pass' or the run's status; 'Missing' for a case the run does not hold,
'Incomplete' for one that began and was cut off), then 'N selected, P pass,
F not-pass'; with several RUNs, one block per run headed '== RUN'. Exit 0
when every selected case of every run is Pass; 3 when any is not (a missing
case counts as not-pass; NotSupported is not Pass); 1 and 2 as above. The
default output, without the option, is unchanged. --compare and
--require-pass together are refused.

--selftest writes tiny run directories inline -- the shapes real logs hold:
unindented, space- and tab-indented <Result> lines, a multi-line message,
&quot; and &amp; entities, a self-closed <Result ... />,
#terminateTestCaseResult inside an open block and after a closed one, a
case that began without a Result, a resumed chunk's retry whose later
message wins -- and checks the tallies, the limit rows, the compare and
every refusal against them.
"""
import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
from xml.sax.saxutils import unescape

REQUIRE_GROUPS = ("implementation_limits", "state_query.integers",
                  "state_query.integers64", "state_query.floats",
                  "state_query.booleans")
MUSTPASS = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'gles3-mustpass.txt')
GROUPS_DEFAULT = ("implementation_limits", "state_query.integers",
                  "state_query.integers64", "state_query.floats", "ubo",
                  "shaders.linkage.varying", "shaders.builtin_variable")
LIMITS_DEFAULT = ("max_varying_vectors", "max_varying_components",
                  "max_fragment_input_components", "max_vertex_output_components",
                  "max_uniform_block_size", "max_combined_vertex_uniform_components",
                  "max_combined_fragment_uniform_components")

CASE_RE = re.compile(r'^dEQP-GLES[23]\.functional\.')
BEGIN = re.compile(r'^#beginTestCaseResult (\S+)')
QPA_OPEN = re.compile(r'<TestCaseResult\b[^>]*\bCasePath="([^"]+)"')
RESULT = re.compile(r'<Result\b[^>]*\bStatusCode="([^"]*)"([^>]*)>')
RESULT_END = '</Result>'
END = re.compile(r'^#endTestCaseResult')
TERMINATE = re.compile(r'^#terminateTestCaseResult (\S+)')
ENTITIES = {"&quot;": '"', "&apos;": "'"}

NAMED = ("Pass", "Fail", "NotSupported")
WIDTH = 34   # the group column: the longest default group name is 24 chars

HEAD = '%-*s %6s %6s %6s %6s %6s %6s %6s %8s' % (
    WIDTH, 'group', 'cases', 'asked', 'began', 'pass', 'fail', 'notsup',
    'other', 'noresult')


class TallyError(Exception):
    """A refused input; main turns it into exit 1."""


def message(msg):
    """One dEQP status message, whole: entities decoded, blank lines and the
    line breaks dropped, the parts joined by ' / '. A multi-line message
    holds the reason on the second line (the first is often just
    'Compile failed:'), and the tally must not drop it (cts-summary.py read
    whole InfoLogs for exactly this reason). None stays None: a self-closed
    <Result ... /> has a status and no message."""
    if msg is None:
        return None
    parts = [p.strip() for p in unescape(msg, ENTITIES).splitlines() if p.strip()]
    return ' / '.join(parts)


def norm_group(g):
    """--group accepts the suffix alone or the full case prefix."""
    g = CASE_RE.sub('', g.strip()).strip('.')
    if not g:
        raise TallyError('--group names no group: %r' % g)
    return g


def group_of(case, groups):
    """The first group whose name the case starts with, at a boundary
    (rest == g or rest.startswith(g + '.')): state_query.integers64.x is
    integers64, not integers."""
    rest = CASE_RE.sub('', case)
    for g in groups:
        if rest == g or rest.startswith(g + '.'):
            return g
    return None


def read_qpa(path, began, res):
    """One .qpa into began (set of cases its logs opened) and res ({}:
    case -> (status, message)). The block and marker reading of
    tools/cts/gles3-notpass.py and tools/cts/tree-diff.py: the
    #beginTestCaseResult markers and the TestCaseResult blocks (either
    opens the case), a <Result StatusCode="..."> whose text may span
    lines, a self-closed Result (a status, no message),
    #terminateTestCaseResult inside an open, result-less block (the
    runner's stop after a ResourceError or a deinit error -- its status
    word is the case's result), #endTestCaseResult, and a block still
    open at the file's end. Where this differs from gles3-notpass.py: a
    message that starts on the <Result> line keeps that line (there it is
    dropped, so 'Compile failed:' vanishes and only the reason survives).
    A case that began is in began even with no Result (the runner's
    Incomplete); the last Result or terminate status of a case wins
    (cts-summary.py's rule: a case retried by hand or by the chunk resume
    keeps the later one)."""
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
                # Continuation lines get a newline before them, including
                # the one that closes the message: without it the lines
                # fuse into one.
                msg += '\n' + line.split(RESULT_END, 1)[0]
                if RESULT_END in line:
                    in_msg = False
                continue
            o, b = QPA_OPEN.search(line), BEGIN.match(line)
            if o or b:
                name = (o or b).group(1)
                if name != case:      # '#beginTestCaseResult X' precedes X's own block
                    close()           # an unclosed previous block: its status so far kept
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
                    # The text starts on this line and runs on: keep the
                    # tail here or the first line is lost (a compile
                    # refusal says 'Compile failed:' there and the reason
                    # on the next -- dropping the first line reads as if
                    # the message said only the reason).
                    msg, in_msg = tail, True
            t = TERMINATE.match(line)
            if t:
                # A healthy chunk ends '#endTestCaseResult' then
                # '#terminateTestCaseResult Passed': nothing to record after
                # the case closed. Inside an open, result-less block it is
                # the case's status (gles3-notpass.py reads it the same way).
                if case is not None and status is None:
                    status, msg = t.group(1), None
                close()
            elif END.match(line):
                close()
    close()


def load_run(path):
    """One host-deqp run directory (or one .qpa file) into
    {dir, gpu, module, commit, extra, asked, began, res}. No c_*.qpa is not
    a run directory; a run with no parsed result at all is refused: an
    empty run or an unreadable log is not an outcome (the rule
    host-deqp-run.sh's compare_runs learned the hard way)."""
    run = {'dir': path, 'gpu': None, 'module': None, 'commit': None,
           'extra': None, 'asked': None, 'began': set(), 'res': {}}
    if os.path.isdir(path):
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
                if len(tok) > 3:
                    run['extra'] = ' '.join(tok[3:])
        else:
            run['commit'] = os.path.basename(os.path.normpath(path))
        af = os.path.join(path, 'all.txt')
        if os.path.isfile(af):
            with open(af, encoding='utf-8', errors='replace') as f:
                run['asked'] = [ln.strip() for ln in f if ln.strip()
                                and not ln.startswith('#')]
    elif os.path.isfile(path) and path.endswith('.qpa'):
        logs = [path]
    else:
        raise TallyError('no such run directory: %s' % path)
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
    """The runner's reading of one case: its logged status; the synthetic
    Incomplete (host-deqp-run.sh's word) for a case that began and was cut
    off; None for a case its logs do not hold."""
    if case in run['res']:
        return run['res'][case][0]
    return 'Incomplete' if case in run['began'] else None


def group_table(run, groups):
    """[(group, cases, asked, began, pass, fail, notsup, other, noresult)],
    in the groups' order, groups with no case omitted. A case belongs to
    the first group it matches, so --group and a default can overlap
    without a case counted twice."""
    members = {}
    for c in universe(run):
        g = group_of(c, groups)
        if g is not None:
            members.setdefault(g, []).append(c)
    asked_set = set(run['asked']) if run['asked'] is not None else universe(run)
    rows = []
    for g in groups:
        ms = members.get(g)
        if not ms:
            continue
        st = [run['res'][c][0] for c in ms if c in run['res']]
        rows.append((g, len(ms),
                     sum(1 for c in ms if c in asked_set),
                     sum(1 for c in ms if c in run['began'] or c in run['res']),
                     st.count('Pass'), st.count('Fail'), st.count('NotSupported'),
                     len(st) - st.count('Pass') - st.count('Fail') - st.count('NotSupported'),
                     len(ms) - len(st)))
    return rows


def total_row(rows):
    return ('TOTAL',) + tuple(sum(r[i] for r in rows) for i in range(1, 9))


def fmt_row(r):
    return '%-*s %6d %6d %6d %6d %6d %6d %6d %8d' % (
        WIDTH, r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8])


def run_header(run):
    h = 'run %s  gpu=%s module=%s commit=%s' % (
        run['dir'], run['gpu'] or '-', run['module'] or '-', run['commit'] or '-')
    if run['extra']:
        h += ' ' + run['extra']
    h += '  asked %d  began %d' % (len(run['asked']) if run['asked'] is not None
                                   else len(run['began']), len(run['began']))
    return h + '  (HOST-ONLY: tools/cts/README.md 2b; never quote a render verdict from these counts)'


def status_line(cases, run):
    counts = {}
    for c in cases:
        st = case_status(run, c) or 'NotRun'
        counts[st] = counts.get(st, 0) + 1
    return '  '.join('%s %d' % (k, counts[k]) for k in sorted(counts))


def run_footer(run, groups):
    uni = universe(run)
    out = ['whole run (%d case(s)): %s' % (len(uni), status_line(uni, run))]
    outside = [c for c in uni if group_of(c, groups) is None]
    if outside:
        out.append('outside the tallied groups (%d case(s)): %s'
                   % (len(outside), status_line(outside, run)))
    return out


def limit_rows(run, limits):
    """([Fail rows], [other non-Pass rows]) of the cases whose name contains
    one of the limit words: (status, case, message). Pass and NotSupported
    rows are not failures and not listed here; NoResult and Incomplete are
    (a limit row lost to a crash or never run is part of the gap)."""
    fails, others = [], []
    for c in sorted(universe(run)):
        if not any(k in c for k in limits):
            continue
        st = case_status(run, c) or 'NotRun'
        if st == 'Pass' or st == 'NotSupported':
            continue
        if st == 'Incomplete':
            msg = 'began and was cut off (no <Result StatusCode=)'
        elif st == 'NotRun':
            msg = 'never began'
        else:
            msg = run['res'][c][1] or '(no message in the log)'
        (fails if st == 'Fail' else others).append((st, c, msg))
    return fails, others


def tally_lines(runs, groups, limits):
    out = []
    for i, run in enumerate(runs):
        if i:
            out.append('')
        out.append(run_header(run))
        rows = group_table(run, groups)
        out.append(HEAD)
        for r in rows:
            out.append(fmt_row(r))
        out.append(fmt_row(total_row(rows)))
        if not rows:
            out.append('(no case of the tallied groups in this run)')
        out.extend(run_footer(run, groups))
        fails, others = limit_rows(run, limits)
        out.append('limit rows (case name contains: %s):' % ' '.join(limits))
        for st, c, msg in fails:
            out.append('  %-13s %s  %s' % (st, c, msg))
        for st, c, msg in others:
            out.append('  %-13s %s  %s' % (st, c, msg))
        if not fails and not others:
            out.append('  (none: no non-Pass row of these limits)')
    return out


def compare_lines(old, new, groups, limits):
    out = ['compare %s vs %s (HOST-ONLY: counts from host-deqp runs)'
           % (new['dir'], old['dir'])]
    if old['gpu'] is not None and new['gpu'] is not None and old['gpu'] != new['gpu']:
        out.append('note: different GPUs: old=%s new=%s' % (old['gpu'], new['gpu']))
    out.extend(tally_lines([old], groups, limits))
    out.append('')
    out.extend(tally_lines([new], groups, limits))
    cases = sorted(set(old['began']) | set(old['res']) | set(new['began']) | set(new['res']))
    changed = []
    for c in cases:
        a, b = case_status(old, c), case_status(new, c)
        if a == b:
            continue
        row = '  %s: %s -> %s' % (c, a or '-', b or '-')
        msg = None
        if b is not None and b != 'Pass':
            msg = new['res'].get(c, (None, None))[1] or old['res'].get(c, (None, None))[1]
        elif b is None:
            msg = old['res'].get(c, (None, None))[1]
        if msg:
            row += '  (%s)' % msg
        changed.append(row)
    out.append('changed cases (%d):' % len(changed))
    out.extend(changed if changed else ['  (none)'])

    def side(run):
        m = {}
        for c in set(run['began']) | set(run['res']):
            st = case_status(run, c)
            m[st] = m.get(st, 0) + 1
        return m
    oc, nc = side(old), side(new)
    out.append('%-20s %10s %10s' % ('status', 'old', 'new'))
    for st in sorted(set(oc) | set(nc)):
        out.append('%-20s %10d %10d' % (st, oc.get(st, 0), nc.get(st, 0)))
    out.append('%d case(s) changed' % len(changed))
    return out


def require_cases(mustpass=MUSTPASS):
    """The selected case names, sorted, read from gles3-mustpass.txt."""
    pat = re.compile(r'^dEQP-GLES3\.functional\.(?:%s)\.(?:%s)(?:_get(?:boolean|float|integer|integer64))?$'
                     % ('|'.join(re.escape(g) for g in REQUIRE_GROUPS),
                        '|'.join(LIMITS_DEFAULT)))
    try:
        with open(mustpass, encoding='utf-8', errors='replace') as f:
            names = sorted({ln.strip() for ln in f if pat.match(ln.strip())})
    except OSError as e:
        raise TallyError('cannot read %s: %s' % (mustpass, e))
    if not names:
        raise TallyError('%s holds no limit case' % mustpass)
    return names


def require_lines(runs, paths):
    """([lines], all_pass) of --require-pass limits."""
    names = require_cases()
    out, ok = [], True
    for i, (run, path) in enumerate(zip(runs, paths)):
        if len(runs) > 1:
            if i:
                out.append('')
            out.append('== %s' % path)
        npass = 0
        for c in names:
            st = case_status(run, c) or 'Missing'
            npass += st == 'Pass'
            out.append('%s %s' % (st, c))
        out.append('%d selected, %d pass, %d not-pass'
                   % (len(names), npass, len(names) - npass))
        ok = ok and npass == len(names)
    return out, ok


def main():
    ap = argparse.ArgumentParser(
        prog='deqp-limits-tally.py',
        description='Tally the dEQP limit/state-query/ubo/varying groups of host-deqp runs.')
    ap.add_argument('runs', nargs='*', metavar='RUN')
    ap.add_argument('--compare', action='store_true',
                    help='the two RUNs are OLD NEW: print the cases whose status changed')
    ap.add_argument('--group', action='append', default=[], metavar='G',
                    help='tally this group too (repeatable)')
    ap.add_argument('--limit', action='append', default=[], metavar='WORD',
                    help='list limit rows named after this word too (repeatable)')
    ap.add_argument('--require-pass', choices=('limits',), metavar='limits',
                    help='acceptance check: the limit cases must all be Pass (exit 3 if not)')
    ap.add_argument('--selftest', action='store_true')
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    groups = GROUPS_DEFAULT + tuple(norm_group(g) for g in a.group)
    limits = LIMITS_DEFAULT + tuple(w for w in a.limit if w)
    try:
        if a.require_pass:
            if a.compare or not a.runs:
                ap.error('--require-pass needs RUN... and no --compare')
            lines, ok = require_lines([load_run(p) for p in a.runs], a.runs)
            sys.stdout.write('\n'.join(lines) + '\n')
            return 0 if ok else 3
        if a.compare:
            if len(a.runs) != 2:
                ap.error('--compare needs exactly two run directories: OLD NEW')
            lines = compare_lines(load_run(a.runs[0]), load_run(a.runs[1]), groups, limits)
        else:
            if not a.runs:
                ap.error('give at least one run directory (or --compare OLD NEW)')
            lines = tally_lines([load_run(p) for p in a.runs], groups, limits)
    except TallyError as e:
        sys.stderr.write('deqp-limits-tally: %s\n' % e)
        return 1
    sys.stdout.write('\n'.join(lines) + '\n')
    return 0


def st_case(name, status=None, msg=None, indent=''):
    """One complete case block of the fixture logs (a begin marker, the
    TestCaseResult block, optionally a Result, the end markers). No status
    is the runner's Incomplete shape: the case began, the log wrote no
    <Result StatusCode=."""
    out = ['#beginTestCaseResult %s' % name,
           '<TestCaseResult CasePath="%s" CaseType="SelfValidate" Version="0.3.4">' % name]
    if status is not None:
        if msg is None:
            out.append('%s<Result StatusCode="%s" />' % (indent, status))
        else:
            out.append('%s<Result StatusCode="%s">%s</Result>' % (indent, status, msg))
    out.extend(['</TestCaseResult>', '#endTestCaseResult'])
    return out


def st_run(d, gpu, module, commit, asked, chunks):
    """Write one tiny run directory the way host-deqp-run.sh writes one:
    commit, all.txt, c_*.qpa."""
    os.makedirs(d)
    with open(os.path.join(d, 'commit'), 'w') as f:
        f.write('%s %s %s\n' % (commit, gpu, module))
    with open(os.path.join(d, 'all.txt'), 'w') as f:
        f.write('\n'.join(asked) + '\n')
    for name, lines in chunks.items():
        with open(os.path.join(d, name), 'w') as f:
            f.write('\n'.join(lines) + '\n')


def selftest():
    tmp = tempfile.mkdtemp(prefix='deqp-limits-tally-selftest.')
    try:
        return selftest_in(tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_in(tmp):
    fails = []

    def check(name, cond, detail=''):
        print('  ok    %s' % name if cond else '  FAIL  %s %s' % (name, detail))
        if not cond:
            fails.append(name)
        return cond

    def run_cli(*args):
        r = subprocess.run([sys.executable, '-I', os.path.abspath(__file__)] + list(args),
                           capture_output=True, text=True)
        return r.returncode, r.stdout, r.stderr

    IL = 'dEQP-GLES3.functional.implementation_limits.'
    IQI = 'dEQP-GLES3.functional.state_query.integers.'
    IQ6 = 'dEQP-GLES3.functional.state_query.integers64.'
    IQF = 'dEQP-GLES3.functional.state_query.floats.'
    UBO = 'dEQP-GLES3.functional.ubo.'
    VAR = 'dEQP-GLES3.functional.shaders.linkage.varying.'
    BV = 'dEQP-GLES3.functional.shaders.builtin_variable.'
    FO = 'dEQP-GLES3.functional.fragment_ops.blend.'
    C = [IL + 'max_varying_vectors', IL + 'max_varying_components',
         IL + 'max_uniform_block_size', IL + 'max_combined_vertex_uniform_components',
         IL + 'max_vertex_output_components', IL + 'max_fragment_input_components',
         IQI + 'max_varying_vectors_getfloat',
         IQI + 'max_combined_fragment_uniform_components_getfloat',
         IQI + 'max_uniform_block_size_getinteger64',
         IQ6 + 'max_vertex_output_components_getinteger64',
         IQF + 'max_texture_lod_bias_getfloat', UBO + 'single_basic_type.ubyte',
         UBO + 'random.float', VAR + 'basic_types.vec4',
         BV + 'max_vertex_output_vectors_vertex', BV + 'max_vertex_output_vectors_fragment',
         FO + 'foo']

    # old/ -- the before run, GPU=v3d, module=gles3. c_000 unindented,
    # c_001 indented (a space per level, and a tab) the way real dEQP logs
    # write them (host-deqp-compare-selftest.sh's lesson: a parse that
    # anchors the Result line reads a real run as all-Incomplete).
    c0 = []
    for n in C[0:4]:
        c0 += st_case(n, 'Fail', 'Requirement not satisfied')
    c0 += st_case(C[4])                                   # began, no Result
    c0 += st_case(C[6], 'Fail', 'Got invalid float value')
    c0 += st_case(C[7], 'Fail', 'Got invalid float value')
    c0 += st_case(C[8], 'Fail', 'Got invalid integer value')
    c0 += st_case(C[11], 'Fail', 'Compile failed: 0:12: not lowered yet\n'
                                 'second line, the reason')  # multi-line message
    c0 += ['#beginTestCaseResult %s' % C[12],
           '<TestCaseResult CasePath="%s" CaseType="SelfValidate" Version="0.3.4">' % C[12],
           '#terminateTestCaseResult InternalError']      # terminate inside an open block
    c0 += st_case(C[16], 'Fail', 'Image compare failed &amp; more')
    c0.append('#terminateTestCaseResult Passed')          # after a closed case: ignored
    r1 = st_case(C[0], 'Fail', 'retried: max_varying_vectors 8 is below the ES 3.0 minimum 15')
    c1 = (st_case(C[9], 'Fail', 'Got invalid integer value', indent=' ')
          + st_case(C[10], 'Pass', 'Pass', indent='\t')
          + st_case(C[13], 'NotSupported', 'Extension &quot;OES_foo&quot; not supported',
                    indent=' ')
          + st_case(C[14], 'Fail', 'Compile failed: 0:12: not lowered yet', indent=' ')
          + st_case(C[15], 'Timeout')                    # self-closed <Result ... />
          + ['#terminateTestCaseResult Passed'])
    old_dir = os.path.join(tmp, 'run-old-v3d-gles3')
    st_run(old_dir, 'v3d', 'gles3', 'aaaaaaa', C,
           {'c_000.qpa': c0, 'c_000.r1.qpa': r1, 'c_001.qpa': c1})

    # new/ -- the after run: the raises landed on several limits; one
    # InternalError became a Fail; the group-outside case became a Crash;
    # max_fragment_input_components still never began.
    n0 = (st_case(C[0], 'Pass', 'Pass')
          + st_case(C[1], 'Fail', 'Requirement not satisfied')
          + st_case(C[2], 'Fail', 'Requirement not satisfied')
          + st_case(C[3], 'Pass', 'Pass')
          + st_case(C[4], 'Pass', 'Pass')
          + st_case(C[6], 'Fail', 'Got invalid float value')
          + st_case(C[7], 'Pass', 'Pass')
          + st_case(C[8], 'Pass', 'Pass')
          + st_case(C[9], 'Pass', 'Pass', indent=' ')
          + st_case(C[10], 'Pass', 'Pass', indent=' ')
          + st_case(C[11], 'Pass', 'Pass')
          + st_case(C[12], 'Fail', 'draw refused: uniform block exceeds the compiled size')
          + st_case(C[13], 'Pass', 'Pass')
          + st_case(C[14], 'Fail', 'Compile failed: 0:12: not lowered yet')
          + st_case(C[15], 'Timeout')
          + st_case(C[16], 'Crash')
          + ['#terminateTestCaseResult Passed'])
    new_dir = os.path.join(tmp, 'run-new-v3d-gles3')
    st_run(new_dir, 'v3d', 'gles3', 'bbbbbbb', C, {'c_000.qpa': n0})

    pvr_dir = os.path.join(tmp, 'run-old-powervr-gles3')
    st_run(pvr_dir, 'powervr', 'gles3', 'ccccccc', [C[0], C[6]],
           {'c_000.qpa': st_case(C[0], 'Fail', 'Requirement not satisfied')
                       + st_case(C[6], 'Fail', 'Got invalid float value')})

    nores_dir = os.path.join(tmp, 'run-nores')          # began, but no Result anywhere
    st_run(nores_dir, 'v3d', 'gles3', 'ddaaaaa', [C[0]], {'c_000.qpa': st_case(C[0])})
    bad_dir = os.path.join(tmp, 'not-a-run')            # a directory without c_*.qpa
    os.makedirs(bad_dir)
    open(os.path.join(bad_dir, 'summary.txt'), 'w').close()

    old = load_run(old_dir)
    new = load_run(new_dir)
    pvr = load_run(pvr_dir)

    check('the commit file names the GPU, module and commit',
          (old['gpu'], old['module'], old['commit']) == ('v3d', 'gles3', 'aaaaaaa'),
          '(%s)' % (old,))
    want_rows = [(g,) + t for g, t in zip(GROUPS_DEFAULT, [
        (6, 6, 5, 0, 4, 0, 0, 2),    # implementation_limits: cut off + never began
        (3, 3, 3, 0, 3, 0, 0, 0),    # state_query.integers
        (1, 1, 1, 0, 1, 0, 0, 0),    # state_query.integers64
        (1, 1, 1, 1, 0, 0, 0, 0),    # state_query.floats
        (2, 2, 2, 0, 1, 0, 1, 0),    # ubo: the InternalError row counts as Other
        (1, 1, 1, 0, 0, 1, 0, 0),    # shaders.linkage.varying
        (2, 2, 2, 0, 1, 0, 1, 0)])]  # shaders.builtin_variable: Timeout as Other
    rows = group_table(old, GROUPS_DEFAULT)
    check('the group table tallies the asked, began and status counts (old run)',
          rows == want_rows, '(got %s)' % (rows,))
    check('state_query.integers did not swallow state_query.integers64',
          rows[1][0] == 'state_query.integers' and rows[2][0] == 'state_query.integers64')
    check('the TOTAL row sums the group rows',
          total_row(rows) == ('TOTAL', 16, 16, 15, 1, 10, 1, 2, 2), '(got %s)' % (total_row(rows),))
    want_new = [(g,) + t for g, t in zip(GROUPS_DEFAULT, [
        (6, 6, 5, 3, 2, 0, 0, 1), (3, 3, 3, 2, 1, 0, 0, 0), (1, 1, 1, 1, 0, 0, 0, 0),
        (1, 1, 1, 1, 0, 0, 0, 0), (2, 2, 2, 1, 1, 0, 0, 0), (1, 1, 1, 1, 0, 0, 0, 0),
        (2, 2, 2, 0, 1, 0, 1, 0)])]
    check('the after run tallies its raised statuses',
          group_table(new, GROUPS_DEFAULT) == want_new,
          '(got %s)' % (group_table(new, GROUPS_DEFAULT),))
    check('the TOTAL of the after run', total_row(group_table(new, GROUPS_DEFAULT))
          == ('TOTAL', 16, 16, 15, 9, 5, 0, 1, 1))

    check('a resumed chunk: the retry kept the case and its later message won',
          old['res'][C[0]] == ('Fail', 'retried: max_varying_vectors 8 is below the ES 3.0 '
                                       'minimum 15'), '(got %s)' % (old['res'][C[0]],))
    check('a multi-line message is kept whole, joined by / (not the first line only)',
          old['res'][C[11]][1] == 'Compile failed: 0:12: not lowered yet / '
                                  'second line, the reason', '(got %r)' % (old['res'][C[11]][1],))
    check('dEQP entities in a message decode (&quot; -> \", &amp; -> &)',
          old['res'][C[13]][1] == 'Extension "OES_foo" not supported'
          and old['res'][C[16]][1] == 'Image compare failed & more',
          '(got %r, %r)' % (old['res'][C[13]][1], old['res'][C[16]][1]))
    check('a self-closed <Result ... /> is a status with no message',
          old['res'][C[15]] == ('Timeout', None), '(got %s)' % (old['res'][C[15]],))
    check('#terminateTestCaseResult inside an open block is the case status; '
          'after a closed case it changes nothing',
          old['res'][C[12]][0] == 'InternalError' and old['res'][C[16]][0] == 'Fail')
    check('a case that began without a Result is Incomplete; an asked case never '
          'logged is not',
          case_status(old, C[4]) == 'Incomplete' and case_status(old, C[5]) is None)

    want_fails = [
        ('Fail', IL + 'max_combined_vertex_uniform_components', 'Requirement not satisfied'),
        ('Fail', IL + 'max_uniform_block_size', 'Requirement not satisfied'),
        ('Fail', IL + 'max_varying_components', 'Requirement not satisfied'),
        ('Fail', IL + 'max_varying_vectors', 'retried: max_varying_vectors 8 is below the '
                                             'ES 3.0 minimum 15'),
        ('Fail', IQI + 'max_combined_fragment_uniform_components_getfloat',
         'Got invalid float value'),
        ('Fail', IQI + 'max_uniform_block_size_getinteger64', 'Got invalid integer value'),
        ('Fail', IQI + 'max_varying_vectors_getfloat', 'Got invalid float value'),
        ('Fail', IQ6 + 'max_vertex_output_components_getinteger64', 'Got invalid integer value')]
    want_others = [
        ('NotRun', IL + 'max_fragment_input_components', 'never began'),
        ('Incomplete', IL + 'max_vertex_output_components',
         'began and was cut off (no <Result StatusCode=)')]
    fails_, others_ = limit_rows(old, LIMITS_DEFAULT)
    check('the limit rows listed are exactly the seven words, Fail first with messages',
          fails_ == want_fails, '(got %s)' % (fails_,))
    check('the same limits in another status are listed too (NotRun, Incomplete)',
          others_ == want_others, '(got %s)' % (others_,))

    footer = run_footer(old, GROUPS_DEFAULT)
    check('the footer counts the whole run and splits the NoResult cases',
          ('Pass 1' in footer[0] and 'Fail 11' in footer[0] and 'Incomplete 1' in footer[0]
           and 'InternalError 1' in footer[0] and 'Timeout 1' in footer[0]
           and 'NotRun 1' in footer[0]), '(got %s)' % (footer,))
    check('the footer names the cases outside the tallied groups',
          footer[-1] == 'outside the tallied groups (1 case(s)): Fail 1', '(got %r)' % (footer[-1],))

    check('--group extends the defaults: fragment_ops.blend got its row',
          [r for r in group_table(old, GROUPS_DEFAULT + ('fragment_ops.blend',))
           if r[0] == 'fragment_ops.blend']
          == [('fragment_ops.blend', 1, 1, 1, 0, 1, 0, 0, 0)],
          '(got %s)' % (group_table(old, GROUPS_DEFAULT + ('fragment_ops.blend',)),))

    tl = tally_lines([old, pvr], GROUPS_DEFAULT, LIMITS_DEFAULT)
    joined = '\n'.join(tl)
    check('two run directories print per GPU, from their commit files',
          'gpu=v3d module=gles3' in joined and 'gpu=powervr' in joined
          and 'asked 17  began 16' in joined and 'HOST-ONLY' in joined)

    out = compare_lines(old, new, GROUPS_DEFAULT, LIMITS_DEFAULT)
    joined = '\n'.join(out)
    trans = set()
    for line in out:
        m = re.match(r'^  (\S+): (\S+) -> (\S+)', line)
        if m:
            trans.add((m.group(1), m.group(2), m.group(3)))
    want_trans = {(C[0], 'Fail', 'Pass'), (C[3], 'Fail', 'Pass'),
                  (C[4], 'Incomplete', 'Pass'), (C[7], 'Fail', 'Pass'),
                  (C[8], 'Fail', 'Pass'), (C[9], 'Fail', 'Pass'),
                  (C[11], 'Fail', 'Pass'), (C[12], 'InternalError', 'Fail'),
                  (C[13], 'NotSupported', 'Pass'), (C[16], 'Fail', 'Crash')}
    check('--compare printed exactly the changed verdicts (10, incl the Incomplete '
          'and the outside-group row)', trans == want_trans, '(got %s)' % (trans,))
    check('--compare carries the new message across a status move, and the old one '
          'when the new status has none',
          'InternalError -> Fail  (draw refused: uniform block exceeds the compiled size)' in joined
          and 'Fail -> Crash  (Image compare failed & more)' in joined)
    def totals_row(word):
        for line in out:
            p = line.split()
            if len(p) == 3 and p[0] == word and p[1].isdigit() and p[2].isdigit():
                return (int(p[1]), int(p[2]))
        return None
    check('the per-status totals name both runs (Pass 1 -> 9, Fail 11 -> 5, '
          'Incomplete 1 -> 0)',
          totals_row('Pass') == (1, 9) and totals_row('Fail') == (11, 5)
          and totals_row('Incomplete') == (1, 0),
          '(got %s %s %s)' % (totals_row('Pass'), totals_row('Fail'), totals_row('Incomplete')))
    check('--compare closes with the changed count', '10 case(s) changed' in joined)

    rc, out_, err_ = run_cli(old_dir, pvr_dir)
    check('CLI tally: two run directories exit 0 and print both GPUs',
          rc == 0 and 'gpu=powervr' in out_, '(rc %d, %s)' % (rc, err_[:200]))
    rc, out_, err_ = run_cli('--compare', old_dir, new_dir)
    check('CLI --compare OLD NEW exits 0 with the changed count',
          rc == 0 and '10 case(s) changed' in out_, '(rc %d, %s)' % (rc, err_[:200]))
    rc, out_, err_ = run_cli('--compare', old_dir, nores_dir)
    check('CLI --compare refuses a side without a single result, printing no comparison',
          rc == 1 and 'no <Result StatusCode=' in err_ and 'case(s) changed' not in out_,
          '(rc %d, %s, %s)' % (rc, out_[:120], err_[:200]))
    rc, out_, err_ = run_cli(nores_dir)
    check('CLI tally refuses a run with no result at all',
          rc == 1 and 'no <Result StatusCode=' in err_, '(rc %d, %s)' % (rc, err_[:200]))
    rc, out_, err_ = run_cli(bad_dir)
    check('CLI refuses a directory that is not a run directory (no c_*.qpa)',
          rc == 1 and 'not a run directory' in err_, '(rc %d, %s)' % (rc, err_[:200]))
    rc, out_, err_ = run_cli(os.path.join(old_dir, 'c_001.qpa'))
    check('a single .qpa file is tallied too',
          rc == 0 and 'state_query.integers64' in out_ and 'gpu=-' in out_,
          '(rc %d, %s)' % (rc, err_[:200]))
    rc, out_, err_ = run_cli('--compare', old_dir)
    check('CLI --compare with one directory is a usage error (exit 2)', rc == 2)

    if fails:
        print('deqp-limits-tally --selftest: %d FAILED: %s' % (len(fails), ', '.join(fails)))
        return 1
    print('deqp-limits-tally --selftest: all checks passed')
    print('selftest ok')
    return 0


if __name__ == '__main__':
    sys.exit(main())
