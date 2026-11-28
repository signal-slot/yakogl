#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft the #313 / #1285 comments and the per-family issue texts (markdown on stdout).

  gles3-issue-drafts.py [--tsv docs/es3/am62p-gles3-notpass.tsv]
        [--tree tools/cts/am62p-gles3-tree.txt] [--mustpass tools/cts/gles3-mustpass.txt]
        [--families tools/cts/am62p-gles3-families.txt] [--doc docs/es3-cts-failures.md]
        [--issue FAMILY=NUMBER]... [--examples 5]

Three drafts, each under its own '## ' heading:
  1. the #313 comment: build commit(s), images, the five counts (Pass, Fail,
     NotSupported, QualityWarning, NoResult) and their sum, the families with
     counts and issue links, the rerun rows (the header's 'rows A-B: build X'
     lines).
  2. the #1285 comment: the rows whose cause is the watchdog or a hang (family
     or message naming 1285, watchdog, hang, timeout) and the
     texture.filtering.3d not-pass rows, with their outcomes.
  3. one '### title' + body per family that has no issue number: count, first
     case names, and file:line sites (the 'at file.cpp:N' of the first
     messages, and the 'src/...:N' sites of the lines of the doc that name the
     family).

A row's family is its 4th column; if the TSV has none, the first matching rule
of --families (gles3-triage.py's own rule reader); else group.subgroup. A
family has an issue number when its id holds '#NNN' or --issue names it.

Nothing is printed unless the drafts pass the redaction check: the built-in
one refuses IPv4/IPv6 literals, /dev/tty* paths and user@host forms, and
tools/cts/check-redaction.py is run on the text too when it exists. Exit 0 ok;
1 counts do not add up or the redaction check refused; 2 unreadable input.
Python standard library only; gh is never run.
"""
import argparse
import collections
import importlib.util
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUTCOMES = ['Fail', 'NotSupported', 'QualityWarning', 'NoResult']
P3D = 'dEQP-GLES3.functional.texture.filtering.3d.'
HANG_RE = re.compile(r'1285|watchdog|hang|timeout|timed out', re.I)
SITE_MSG_RE = re.compile(r'\bat ([\w./-]+\.(?:cpp|hpp|c|h|rs)):(\d+)')
SITE_DOC_RE = re.compile(r'`?((?:src|driver|tests|tools)/[\w./-]+\.(?:rs|c|h|py|sh)):(\d+)(?:-(\d+))?')
ROWS_RE = re.compile(r'rows\s+(\d+-\d+)\s*:\s*build\s+([0-9a-f]{7,40})(?:,\s*run\s+([\d/-]+))?')

REDACT = [
    ('IPv4 address', re.compile(r'(?<![\w.])(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?![\w.])')),
    ('IPv6 address', re.compile(r'(?<![\w:])(?:[0-9a-fA-F]{1,4}:){2,7}[0-9a-fA-F]{1,4}(?![\w:])|(?<![\w:])(?:[0-9a-fA-F]{1,4})?(?::[0-9a-fA-F]{0,4})*::(?:[0-9a-fA-F]{1,4}(?::[0-9a-fA-F]{1,4})*)?(?![\w:])')),
    ('/dev/tty path', re.compile(r'/dev/tty\w*|/dev/serial/\S+')),
    ('user@host', re.compile(r'(?<![\w.])[A-Za-z_][\w.-]*@[A-Za-z0-9][\w.-]*(?![\w@])')),
]


def load_triage():
    spec = importlib.util.spec_from_file_location('gles3_triage', os.path.join(HERE, 'gles3-triage.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def header_lines(path):
    out = []
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            if not line.startswith('#'):
                break
            out.append(line.rstrip('\n'))
    return out


def count_lines(path):
    with open(path, encoding='utf-8') as f:
        return [l for l in f if l.strip() and not l.lstrip().startswith('#')]


def family_of(row, tri, rules):
    if row[3] and row[3] != 'unassigned':
        return row[3]
    if rules:
        fam = tri.family_of(rules, row[1], row[2])
        if fam:
            return fam
    p = row[1].split('.')
    return '.'.join(p[2:4]) if len(p) > 4 else (row[1] or 'unknown')


def issue_of(fam, known):
    if fam in known:
        return known[fam]
    m = re.search(r'#(\d+)', fam)
    return m.group(1) if m else None


def link(fam, known):
    n = issue_of(fam, known)
    return '#%s' % n if n else 'no issue yet'


def redaction_problems(text):
    probs = []
    for name, rx in REDACT:
        for m in rx.finditer(text):
            probs.append('%s: %r' % (name, m.group(0)))
    ext = os.path.join(HERE, 'check-redaction.py')
    if os.path.exists(ext):
        with tempfile.NamedTemporaryFile('w', suffix='.md', delete=False) as t:
            t.write(text)
            name = t.name
        try:
            r = subprocess.run([sys.executable, '-I', ext, name], capture_output=True, text=True)
            if r.returncode != 0:
                probs.append('check-redaction.py exit %d: %s' % (r.returncode, (r.stdout + r.stderr).strip()[:200]))
        finally:
            os.unlink(name)
    return probs


def doc_sites(doc, fam):
    """src/...:N sites on lines of the doc that name the family id."""
    if not doc or not os.path.exists(doc) or fam.startswith('#'):
        return []
    sites = []
    key = re.compile(r'(?<![\w-])%s(?![\w-])' % re.escape(fam))
    with open(doc, encoding='utf-8', errors='replace') as f:
        for line in f:
            if key.search(line):
                for m in SITE_DOC_RE.finditer(line):
                    s = '%s:%s' % (m.group(1), m.group(2))
                    if s not in sites:
                        sites.append(s)
    return sites[:6]


def msg_sites(rows):
    c = collections.Counter()
    for r in rows:
        m = SITE_MSG_RE.search(r[2])
        if m:
            c[os.path.basename(m.group(1)) + ':' + m.group(2)] += 1
    return ['%s (%d)' % (s, n) for s, n in c.most_common(4)]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    j = lambda p: os.path.join(ROOT, p)
    ap.add_argument('--tsv', default=j('docs/es3/am62p-gles3-notpass.tsv'))
    ap.add_argument('--tree', default=j('tools/cts/am62p-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=j('tools/cts/gles3-mustpass.txt'))
    ap.add_argument('--families', default=j('tools/cts/am62p-gles3-families.txt'))
    ap.add_argument('--doc', default=j('docs/es3-cts-failures.md'))
    ap.add_argument('--issue', action='append', default=[], metavar='FAMILY=NUMBER')
    ap.add_argument('--examples', type=int, default=5)
    a = ap.parse_args()
    known = {}
    for s in a.issue:
        k, _, v = s.partition('=')
        known[k] = v.lstrip('#')
    try:
        tri = load_triage()
        rows = tri.parse_notpass(a.tsv)
        head = header_lines(a.tsv)
        images = len(count_lines(a.tree))
        total = len(count_lines(a.mustpass))
        rules = tri.parse_families(a.families) if os.path.exists(a.families) else None
    except OSError as e:
        print('gles3-issue-drafts: cannot read %s' % e.filename, file=sys.stderr)
        return 2
    cnt = collections.Counter(r[0] for r in rows)
    bad = [k for k in cnt if k not in OUTCOMES]
    npass = total - sum(cnt[k] for k in OUTCOMES)
    if bad or npass < 0:
        print('gles3-issue-drafts: unknown outcome %s or counts exceed the mustpass list (%d not-pass, %d mustpass)'
              % (bad, len(rows), total), file=sys.stderr)
        return 1
    fams = collections.OrderedDict()
    for r in rows:
        fams.setdefault(family_of(r, tri, rules), []).append(r)
    order = sorted(fams, key=lambda k: (-len(fams[k]), k))
    builds, rerun = [], []
    for h in head:
        for m in ROWS_RE.finditer(h):
            rerun.append(m.groups())
            if m.group(2) not in builds:
                builds.append(m.group(2))
    commit = ', '.join(builds) if builds else '(no build named in the TSV header)'
    f = lambda n: format(n, ',')

    out = ['## Draft 1: comment for #313 (AM62P ES 3.0 stage)', '']
    out.append('dEQP-GLES3 whole tree on the AM62P (PowerVR BXS-4-64), build %s, %d images, %s mustpass cases:'
               % (commit, images, f(total)))
    out.append('')
    out.append('| Pass | Fail | NotSupported | QualityWarning | NoResult | sum |')
    out.append('|---|---|---|---|---|---|')
    out.append('| %s | %s | %s | %s | %s | %s |' % (f(npass), f(cnt['Fail']), f(cnt['NotSupported']),
               f(cnt['QualityWarning']), f(cnt['NoResult']), f(npass + sum(cnt[k] for k in OUTCOMES))))
    out += ['', 'Families (every non-Pass case, `docs/es3/am62p-gles3-notpass.tsv`):', '']
    for k in order:
        oc = collections.Counter(r[0] for r in fams[k])
        out.append('- `%s`: %d (%s), %s' % (k, len(fams[k]),
                   ', '.join('%d %s' % (oc[o], o) for o in OUTCOMES if oc[o]), link(k, known)))
    out += ['', 'Rerun rows (rows of `tools/cts/am62p-gles3-tree.txt`, counted from 0):', '']
    if rerun:
        out += ['- rows %s: build %s%s' % (r, b, ', run ' + d if d else '') for r, b, d in rerun]
    else:
        out.append('- (the TSV header names no rows)')

    hang = [r for r in rows if r[0] == 'NoResult' or HANG_RE.search(family_of(r, tri, rules))
            or HANG_RE.search(r[2])]
    hang.sort(key=lambda r: r[0] != 'NoResult')
    p3 = [r for r in rows if r[1].startswith(P3D)]
    out += ['', '## Draft 2: comment for #1285 (PowerVR 3D mipmap hang)', '']
    if hang:
        c = collections.Counter(r[0] for r in hang)
        out.append('%d case(s) have no result or name the watchdog or a hang: %s.'
                   % (len(hang), ', '.join('%d %s' % (c[k], k) for k in OUTCOMES if c[k])))
        for r in hang[:max(a.examples, 5)]:
            out.append('- %s: `%s`%s' % (r[0], r[1], ' (' + r[2] + ')' if r[2] else ''))
    else:
        out.append('No case is without a result or names the watchdog or a hang.')
    c3 = collections.Counter(r[0] for r in p3)
    out.append('')
    out.append('texture.filtering.3d not-pass: %s.' % (
        ', '.join('%d %s' % (c3[k], k) for k in OUTCOMES if c3[k]) if p3 else 'none'))
    out.append('Build %s; rows %s.' % (commit, ', '.join(r for r, _, _ in rerun) or 'not named'))

    out += ['', '## Draft 3: new issues (families without an issue number)']
    new = [k for k in order if not issue_of(k, known)]
    if not new:
        out += ['', 'Every family has an issue number.']
    for k in new:
        rs = fams[k]
        oc = collections.Counter(r[0] for r in rs)
        out += ['', '### AM62P dEQP-GLES3: %s (%d %s)' % (k, len(rs), '/'.join(o for o in OUTCOMES if oc[o])), '',
                'Build %s. %d case(s): %s.' % (commit, len(rs),
                ', '.join('%d %s' % (oc[o], o) for o in OUTCOMES if oc[o])), '', 'First cases:']
        for r in rs[:a.examples]:
            out.append('- `%s`%s' % (r[1], ': ' + r[2] if r[2] else ''))
        sites = msg_sites(rs) + doc_sites(a.doc, k)
        out += ['', 'Sites: ' + (', '.join('`%s`' % s for s in sites) if sites else 'none given by the TSV or the doc')]
    text = '\n'.join(out) + '\n'
    probs = redaction_problems(text)
    if probs:
        print('gles3-issue-drafts: redaction check refused the drafts:', file=sys.stderr)
        for p in probs[:10]:
            print('  ' + p, file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
