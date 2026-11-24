#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the ROADMAP bullet, the #1285 comment and the new-issue texts for the AM62P dEQP-GLES3 run.

  am62p-gles3-issues.py roadmap    [common options]
  am62p-gles3-issues.py issue-1285 [common options] [--note docs/notes/pvr-3d-mipmap-hang.md]
  am62p-gles3-issues.py new-issues [common options] [--known FILE]

common options: [--tsv docs/es3/am62p-gles3-notpass.tsv] [--tree tools/cts/am62p-gles3-tree.txt]
                [--mustpass tools/cts/gles3-mustpass.txt] [--commit SHA] [--allow-mixed]

Reads the not-pass TSV (parsed by gles3-triage.py's parse_notpass), the image table and the mustpass
list. Pass = mustpass cases minus the non-Pass rows; the five counts must add up to the mustpass count.
--commit defaults to the sha after "build" in the TSV's header; when the header names more than one
build the script exits 2 unless --allow-mixed is given (the texts then name every build).
A row's family is its 4th column, or, when the TSV has none, <group>.<subgroup> of the case name
below dEQP-GLES3.functional.

  roadmap     one Markdown bullet: commit, image count, the five counts.
  issue-1285  the comment for #1285: outcomes of the texture.filtering.3d rows, NoResult named as
              unfinished, whether the issue may be closed (no NoResult and no Fail there), the note.
  new-issues  one title/body per family and outcome (Fail, QualityWarning), '<board> dEQP-GLES3:
              <family> <n> <outcome>', up to 10 cases with the first message line; --known FILE lists
              families (one per line) already tracked, which are skipped.

Exit 0 ok; 1 the counts do not add up, an unknown outcome or no commit (nothing printed); 2 mixed
builds or unreadable input.
"""
import argparse
import collections
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
BOARD = 'AM62P'
OUTCOMES = ['Fail', 'NotSupported', 'QualityWarning', 'NoResult']
P3D = 'dEQP-GLES3.functional.texture.filtering.3d.'
EXAMPLES = 10


def load_triage():
    spec = importlib.util.spec_from_file_location(
        'gles3_triage', os.path.join(HERE, 'gles3-triage.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def header_builds(path):
    seen = []
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            if not line.startswith('#'):
                break
            for m in re.finditer(r'\bbuild\s+([0-9a-f]{7,40})\b', line):
                if m.group(1) not in seen:
                    seen.append(m.group(1))
    return seen


def count_lines(path):
    with open(path, encoding='utf-8') as f:
        return [l for l in f if l.strip() and not l.lstrip().startswith('#')]


def family(row):
    if row[3] and row[3] != 'unassigned':
        return row[3]
    p = row[1].split('.')
    return '.'.join(p[2:4]) if len(p) > 4 else (row[1] or 'unknown')


def load(a):
    """(rows, commit, images, total, npass, counts, problems); exits 2 on mixed builds."""
    tri = load_triage()
    try:
        rows = tri.parse_notpass(a.tsv)
        builds = header_builds(a.tsv)
        images = len(count_lines(a.tree))
        total = len(count_lines(a.mustpass))
    except OSError as e:
        print('am62p-gles3-issues: cannot read %s' % e.filename, file=sys.stderr)
        sys.exit(2)
    problems = []
    if a.commit:
        commit = a.commit
    elif len(builds) > 1 and not a.allow_mixed:
        print('am62p-gles3-issues: the TSV header names %d builds (%s); pass --commit or --allow-mixed'
              % (len(builds), ', '.join(builds)), file=sys.stderr)
        sys.exit(2)
    elif builds:
        commit = ', '.join(builds)
    else:
        commit = None
        problems.append('no build commit: header has no "build <sha>" and no --commit')
    cnt = collections.Counter(r[0] for r in rows)
    for k in cnt:
        if k not in OUTCOMES:
            problems.append('unknown outcome %r (%d rows)' % (k, cnt[k]))
    npass = total - sum(cnt[k] for k in OUTCOMES)
    if npass < 0:
        problems.append('counts do not add up: %d not-pass rows, mustpass has %d' % (len(rows), total))
    if images == 0:
        problems.append('tree names no images')
    return rows, commit, images, total, npass, cnt, problems


def roadmap(commit, images, total, npass, cnt):
    f = lambda n: format(n, ',')
    word = 'builds' if ',' in commit else 'build'
    return ('- **%s, OpenGL ES 3.0 (dEQP-GLES3, whole tree).** %s %s, %d images: of %s mustpass cases '
            '%s Pass / %s Fail / %s NotSupported / %s QualityWarning / %s NoResult. '
            'Every case that is not Pass is listed in docs/es3/am62p-gles3-notpass.tsv.'
            % (BOARD, word, commit, images, f(total), f(npass), f(cnt['Fail']),
               f(cnt['NotSupported']), f(cnt['QualityWarning']), f(cnt['NoResult'])))


def issue_1285(rows, commit, note):
    r3 = [r for r in rows if r[1].startswith(P3D)]
    c = collections.Counter(r[0] for r in r3)
    closable = c['NoResult'] == 0 and c['Fail'] == 0
    lines = ['Outcome of the texture.filtering.3d images (tree rows 22-38, formerly images 20/21) '
             'at %s:' % commit, '']
    if r3:
        lines.append('- %d texture.filtering.3d cases are not Pass: %s.' % (
            len(r3), ', '.join('%d %s' % (c[k], k) for k in OUTCOMES if c[k])))
    else:
        lines.append('- No texture.filtering.3d case is listed as not Pass.')
    for k, what in (('NoResult', 'unfinished (NoResult: the case never reported a status)'),
                    ('Fail', 'Fail')):
        if c[k]:
            lines.append('- %d case(s) %s, first: %s' % (c[k], what, next(r[1] for r in r3 if r[0] == k)))
    lines.append('')
    if closable:
        lines.append('No NoResult and no Fail in texture.filtering.3d: the images finish. '
                     'This issue can be closed.')
    else:
        lines.append('Not closable yet: texture.filtering.3d still has %s.' % ' and '.join(
            '%d %s' % (c[k], k) for k in ('NoResult', 'Fail') if c[k]))
    if note:
        rel = os.path.relpath(note, ROOT) if os.path.abspath(note).startswith(ROOT + os.sep) else note
        lines += ['', 'Analysis and recipe: %s' % rel]
    return '\n'.join(lines)


def new_issues(rows, commit, known):
    groups = {}
    for r in rows:
        if r[0] in ('Fail', 'QualityWarning') and family(r) not in known:
            groups.setdefault((family(r), r[0]), []).append(r)
    out = []
    for (fam, oc), rs in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        body = ['%d dEQP-GLES3 case(s) of %s end %s on the %s at %s.' % (len(rs), fam, oc, BOARD, commit), '']
        for r in rs[:EXAMPLES]:
            body.append('- %s: %s' % (r[1], r[2].splitlines()[0] if r[2] else '(no message)'))
        if len(rs) > EXAMPLES:
            body.append('- ... and %d more, all in docs/es3/am62p-gles3-notpass.tsv' % (len(rs) - EXAMPLES))
        out.append('Title: %s dEQP-GLES3: %s %d %s\n\n%s' % (BOARD, fam, len(rs), oc, '\n'.join(body)))
    return '\n\n---\n\n'.join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('cmd', choices=['roadmap', 'issue-1285', 'new-issues'])
    ap.add_argument('--tsv', default=os.path.join(ROOT, 'docs/es3/am62p-gles3-notpass.tsv'))
    ap.add_argument('--tree', default=os.path.join(HERE, 'am62p-gles3-tree.txt'))
    ap.add_argument('--mustpass', default=os.path.join(HERE, 'gles3-mustpass.txt'))
    ap.add_argument('--commit')
    ap.add_argument('--allow-mixed', action='store_true')
    ap.add_argument('--note')
    ap.add_argument('--known')
    a = ap.parse_args()
    rows, commit, images, total, npass, cnt, problems = load(a)
    if problems:
        for p in problems:
            print('am62p-gles3-issues: ' + p, file=sys.stderr)
        return 1
    if a.cmd == 'roadmap':
        print(roadmap(commit, images, total, npass, cnt))
    elif a.cmd == 'issue-1285':
        print(issue_1285(rows, commit, a.note))
    else:
        known = set()
        if a.known:
            try:
                with open(a.known, encoding='utf-8') as f:
                    known = {l.strip() for l in f if l.strip() and not l.startswith('#')}
            except OSError:
                print('am62p-gles3-issues: cannot read --known', file=sys.stderr)
                return 2
        print(new_issues(rows, commit, known))
    return 0


if __name__ == '__main__':
    sys.exit(main())
