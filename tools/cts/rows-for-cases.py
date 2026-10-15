#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Which rows of a dEQP tree table hold these cases, and the TREE_ROWS to rerun them.

  python3 tools/cts/rows-for-cases.py [options] <table> <case or glob>...
  python3 tools/cts/rows-for-cases.py [options] <table> --cases <file> [<case or glob>...]
  python3 tools/cts/rows-for-cases.py --self-test

<table>   a tree table in the form tools/board.sh tree reads: one dEQP filter
          (dEQP's --deqp-case, comma-separated patterns) per line, blank lines
          and '#' lines skipped. Rows count from 0 over the other lines, the
          TREE_ROWS number of tools/board.sh and am62p-deqp-images.sh. A row's
          label is the "# image <LABEL>:" line above it (pi4-full-tree.txt),
          else img<row+1>-<tag> (am62p-deqp-images.sh, TREE_IMAGE_DIR) or, for
          a selector table (imx8mp-*.txt: one image, TREE_IMAGE), e<NN> with
          selector NN = the row.
<case>    a case name, e.g. dEQP-GLES2.functional.texture.completeness.2d.npot_s_repeat;
          a name without the "dEQP-" prefix gets the table's module in front
          ("dEQP-GLES2." before info.* and functional.*, else
          "dEQP-GLES2.functional."), so texture.completeness.2d.npot_s_repeat
          is the same case.
<glob>    a name with '*' (quote it from the shell) stands for the mustpass
          names it matches (gles2-mustpass.txt or gles3-mustpass.txt by the
          table's module; --mustpass to name another file). A glob matching
          none is an error.
--cases <file>  one case or glob per line ('#' lines and blank lines skipped).

A filter selects a case the way dEQP's CasePaths does (tcuCommandLine.cpp,
without TCU_HIERARCHICAL_CASEPATHS): the line is split at ',' (nothing
stripped), and a pattern matches the whole name, '*' standing for any run of
characters (dots included) and every other character for itself ('?' and
'[' are not special, unlike fnmatch).

Prints, per row holding a case: its index, label (and selector), the filter
line and the cases it holds; then the TREE_ROWS value and the commands that
rerun just those rows for the board the table belongs to (am62p-, pi4- or
imx8mp- at the start of its file name; --board to say). A case no row
selects is printed under "not in any row" and makes the exit status 1.

Options:
  --board am62p|pi4|imx8mp   the board (default: from the table's file name)
  --tag <tag>                the TREE_TAG of an unlabelled table's images
                             (default: the literal $HEAD8)
  --mustpass <file>          the names globs expand against
  --rows-only                print only the TREE_ROWS value (e.g. 7,15)

Exit status: 0 every case is in a row, 1 a case or glob is in none, 2 usage.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FIXTURES = os.path.join(HERE, 'testdata', 'rows-for-cases')
BOARDS = ('am62p', 'pi4', 'imx8mp')


def read_lines(path):
    with open(path, errors='replace') as f:
        return f.read().splitlines()


def table_rows(path):
    """[(index, label or None, filter)] of a tree table, by tools/board.sh tree_rows'
    rules: '# image <LABEL>:' sets the next row's label, blank and '#' lines are
    skipped, every other line is a row, taken as it is."""
    rows, lab = [], None
    for line in read_lines(path):
        m = re.match(r'# image ([A-Za-z0-9]+):', line)
        if m:
            lab = m.group(1)
            continue
        if re.match(r'[ \t]*(#|$)', line):
            continue
        rows.append((len(rows), lab, line))
        lab = None
    return rows


def pattern_re(pat):
    """dEQP's matchWildcards as a regular expression: '*' is any run, the rest literal."""
    return re.compile('.*'.join(re.escape(p) for p in pat.split('*')), re.S)


def filter_matcher(filt):
    pats = [pattern_re(p) for p in filt.split(',')]
    return lambda name: any(p.fullmatch(name) for p in pats)


def module_of(rows):
    """'dEQP-GLES2' (or -GLES3, ...) from the table's first pattern."""
    for _, _, filt in rows:
        m = re.match(r'(dEQP-[A-Z0-9]+)\.', filt)
        if m:
            return m.group(1)
    return 'dEQP-GLES2'


def full_name(case, module):
    if case.startswith('dEQP-'):
        return case
    if case.split('.', 1)[0] in ('info', 'functional'):
        return module + '.' + case
    return module + '.functional.' + case


def board_of(path):
    base = os.path.basename(path)
    for b in BOARDS:
        if base.startswith(b + '-'):
            return b
    return None


def default_mustpass(module):
    n = {'dEQP-GLES2': 'gles2-mustpass.txt', 'dEQP-GLES3': 'gles3-mustpass.txt'}.get(module)
    return os.path.join(HERE, n) if n else None


def expand(cases, module, mustpass):
    """[(given, [full names])]: a glob becomes the mustpass names it matches."""
    names = None
    out = []
    for c in cases:
        f = full_name(c, module)
        if '*' not in f:
            out.append((c, [f]))
            continue
        if names is None:
            if not mustpass or not os.path.isfile(mustpass):
                raise SystemExit('rows-for-cases: a glob (%s) needs a mustpass list (--mustpass)' % c)
            names = [l.strip() for l in read_lines(mustpass) if l.strip() and not l.startswith('#')]
        m = pattern_re(f)
        out.append((c, [n for n in names if m.fullmatch(n)]))
    return out


def label_of(row, board, tag):
    i, lab, _ = row
    if board == 'imx8mp':
        return 'e%02d' % i
    return lab if lab else 'img%d-%s' % (i + 1, tag)


def rows_spec(idx):
    """sorted row indices -> '3,5-9' (tools/board.sh in_rows' form)"""
    parts, idx = [], sorted(set(idx))
    k = 0
    while k < len(idx):
        j = k
        while j + 1 < len(idx) and idx[j + 1] == idx[j] + 1:
            j += 1
        parts.append(str(idx[k]) if j == k else '%d-%d' % (idx[k], idx[j]))
        k = j + 1
    return ','.join(parts)


def shown_path(path):
    """a path inside this checkout as $SW/<path> (the sweep checkout of the recipes), else absolute"""
    a = os.path.abspath(path)
    r = os.path.relpath(a, REPO)
    return a if r.startswith('..') else '$SW/' + r


def commands(board, t, spec, labels, tag):
    """The rerun commands of tools/cts/README.md / docs/HANDOFF.md, for these rows."""
    if board == 'am62p':
        return ['TREE_ROWS=%s $SW/tools/cts/am62p-deqp-images.sh %s <img> %s > <img>/build.out' % (spec, t, tag),
                'TREE_IMAGE_DIR=<img> TREE_TAG=%s TREE_ROWS=%s $SW/tools/board.sh am62p tree %s <rec>' % (tag, spec, t),
                '$SW/tools/board.sh am62p tally <rec>']
    if board == 'pi4':
        return ['$SW/tools/cts/pi4-deqp-images.sh %s <img> %s   # <img> must not exist yet' % (t, ' '.join(labels)),
                'TREE_IMAGE_DIR=<img> TREE_ROWS=%s $SW/tools/board.sh pi4 tree %s <rec>' % (spec, t),
                '$SW/tools/board.sh pi4 tally <rec>']
    if board == 'imx8mp':
        return ["$SW/tools/cts/imx8mp-deqp-build.sh wt-%s 'dEQP-GLES2.info.*' --table %s" % (tag, t),
                'TREE_IMAGE=$IMX8MP_OUT/wt-%s/zephyr/zephyr.bin TREE_ROWS=%s $SW/tools/board.sh imx8mp tree %s <rec>'
                % (tag, spec, t),
                '$SW/tools/board.sh imx8mp tally <rec>']
    return []


def run(table, cases, board=None, tag='$HEAD8', mustpass=None, rows_only=False):
    """(exit status, output lines)"""
    rows = table_rows(table)
    if not rows:
        return 2, ['rows-for-cases: %s has no rows' % table]
    module = module_of(rows)
    board = board or board_of(table)
    mustpass = mustpass or default_mustpass(module)
    matchers = [filter_matcher(f) for _, _, f in rows]
    held = {}          # row index -> [names]
    missing = []       # (given, why)
    for given, names in expand(cases, module, mustpass):
        if not names:
            missing.append((given, 'the glob matches no name of %s' % os.path.basename(mustpass)))
            continue
        for n in names:
            hit = [i for i, m in enumerate(matchers) if m(n)]
            if not hit:
                missing.append((n, 'no row selects it'))
            for i in hit:
                if n not in held.setdefault(i, []):
                    held[i].append(n)
    spec = rows_spec(held)
    if rows_only:
        return (1 if missing else 0), [spec]
    shown = shown_path(table)
    out = ['table %s: %d rows, %s, board %s' % (shown, len(rows), module, board or 'unknown')]
    labels = []
    for i in sorted(held):
        lab = label_of(rows[i], board, tag)
        labels.append(lab)
        sel = ', selector %d' % i if board == 'imx8mp' else ''
        out.append('row %d  %s%s  (%d case%s)' % (i, lab, sel, len(held[i]), '' if len(held[i]) == 1 else 's'))
        out.append('  filter: %s' % rows[i][2])
        out.extend('  %s' % n for n in held[i])
    for n, why in missing:
        out.append('not in any row: %s (%s)' % (n, why))
    out.append('TREE_ROWS=%s' % spec)
    if held and board:
        out.append('commands:')
        out.extend('  ' + c for c in commands(board, shown, spec, labels, tag))
    return (1 if missing else 0), out


def read_cases(path):
    return [l.strip() for l in read_lines(path) if l.strip() and not l.lstrip().startswith('#')]


def self_test():
    fails = 0

    def ok(cond, what):
        nonlocal fails
        print(('  ok    ' if cond else '  FAIL  ') + what)
        if not cond:
            fails += 1

    fx = lambda n: os.path.join(FIXTURES, n)
    mp = fx('mustpass.txt')
    # dEQP's pattern rule, not fnmatch's
    ok(filter_matcher('a.b.*,c.d.*')('c.d.e') and not filter_matcher('a.b.*')('a.c.d'),
       'a comma list selects like dEQP patterns')
    ok(filter_matcher('a.*.z')('a.b.c.z'), "'*' spans dots")
    ok(not filter_matcher('a.b?')('a.bc') and filter_matcher('a.b?')('a.b?'), "'?' is literal")
    ok(not filter_matcher('a.[b]')('a.b'), "'[' is literal")
    ok(not filter_matcher('a.b, a.c')('a.c'), 'nothing is stripped around a comma (dEQP splits only)')
    ok(not filter_matcher('a.b')('a.bc') and not filter_matcher('a.b.*')('a.b'), 'a pattern matches the whole name')
    ok(rows_spec([7, 15]) == '7,15' and rows_spec([3, 5, 6, 7, 9]) == '3,5-7,9' and rows_spec([]) == '',
       "TREE_ROWS spec is board.sh's 3,5-9 form")
    ok(full_name('texture.x', 'dEQP-GLES2') == 'dEQP-GLES2.functional.texture.x'
       and full_name('info.vendor', 'dEQP-GLES2') == 'dEQP-GLES2.info.vendor'
       and full_name('dEQP-GLES3.functional.y', 'dEQP-GLES2') == 'dEQP-GLES3.functional.y',
       'short names get the module prefix')
    # the three table forms: rows counted over non-comment lines, labels as board.sh names them
    r = table_rows(fx('am62p-tree.txt'))
    ok([i for i, _, _ in r] == [0, 1, 2, 3] and all(l is None for _, l, _ in r), 'am62p fixture: 4 unlabelled rows')
    r = table_rows(fx('pi4-tree.txt'))
    ok([l for _, l, _ in r] == ['AL', 'T1', 'NS', None], 'pi4 fixture: labels from "# image" lines, an unlabelled row')
    ok(board_of(fx('imx8mp-tree.txt')) == 'imx8mp' and board_of(fx('mustpass.txt')) is None, 'board from the file name')
    # golden outputs: the npot case list against each fixture table
    cases = read_cases(fx('cases.txt'))
    for t, tag in (('am62p-tree.txt', 'abcd1234'), ('pi4-tree.txt', '$HEAD8'), ('imx8mp-tree.txt', 'abcd1234')):
        rc, out = run(fx(t), cases, tag=tag, mustpass=mp)
        want = read_lines(fx(t.replace('.txt', '.expected')))
        ok(rc == 0 and out == want, 'fixture %s: output == %s' % (t, t.replace('.txt', '.expected')))
        if out != want:
            for l in out:
                print('        | ' + l)
    rc, out = run(fx('am62p-tree.txt'), cases, mustpass=mp, rows_only=True)
    ok(rc == 0 and out == ['1,3'], '--rows-only prints just the spec')
    rc, out = run(fx('am62p-tree.txt'), ['dEQP-GLES2.functional.nowhere.case'], mustpass=mp)
    ok(rc == 1 and 'not in any row: dEQP-GLES2.functional.nowhere.case (no row selects it)' in out,
       'a case no row selects: listed, exit 1')
    rc, out = run(fx('am62p-tree.txt'), ['texture.nosuch*'], mustpass=mp)
    ok(rc == 1 and any(l.startswith('not in any row: texture.nosuch*') for l in out), 'a glob matching nothing: exit 1')
    rc, out = run(fx('am62p-tree.txt'), ['dEQP-GLES2.functional.texture.completeness.2d.npot_s_repeat',
                                         'texture.completeness.2d.npot_s_repeat'], mustpass=mp)
    ok(rc == 0 and out.count('  dEQP-GLES2.functional.texture.completeness.2d.npot_s_repeat') == 1,
       'a case named twice is listed once')
    rc, out = run(fx('overlap-tree.txt'), ['texture.completeness.2d.npot_s_repeat'], mustpass=mp, board='am62p')
    ok(rc == 0 and 'TREE_ROWS=0-1' in out and sum(l.startswith('row ') for l in out) == 2,
       'a case two rows select: both rows')
    # the real tables: every npot case is in exactly one row of each board's tree
    for b in BOARDS:
        t = os.path.join(HERE, '%s-full-tree.txt' % b)
        if not os.path.isfile(t):
            ok(False, '%s missing' % t)
            continue
        rows = table_rows(t)
        ms = [filter_matcher(f) for _, _, f in rows]
        names = [n for _, ns in expand(cases, module_of(rows), default_mustpass('dEQP-GLES2')) for n in ns]
        # 11 named + 2d.npot_size* (3) + cube.npot_size_level_* (4) in gles2-mustpass.txt
        ok(len(names) == 18 and all(sum(m(n) for m in ms) == 1 for n in names),
           '%s-full-tree.txt: each of the 18 npot names is in exactly one row' % b)
    print('rows-for-cases self-test: %d checks failed' % fails)
    return fails


def main(argv):
    args = argv[1:]
    if args == ['--self-test']:
        return 1 if self_test() else 0
    board = tag = mustpass = None
    rows_only = False
    cases, rest = [], []
    k = 0
    try:
        while k < len(args):
            a = args[k]
            if a in ('--board', '--tag', '--mustpass', '--cases'):
                v = args[k + 1]
                k += 2
                if a == '--board':
                    if v not in BOARDS:
                        raise ValueError
                    board = v
                elif a == '--tag':
                    tag = v
                elif a == '--mustpass':
                    mustpass = v
                else:
                    cases.extend(read_cases(v))
                continue
            if a == '--rows-only':
                rows_only = True
            elif a.startswith('--') or a == '-h':
                raise ValueError
            else:
                rest.append(a)
            k += 1
    except (IndexError, ValueError, OSError) as e:
        if isinstance(e, OSError):
            print('rows-for-cases: %s' % e, file=sys.stderr)
        print(__doc__, file=sys.stderr)
        return 2
    if not rest or not os.path.isfile(rest[0]) or not (cases or rest[1:]):
        print(__doc__, file=sys.stderr)
        return 2
    rc, out = run(rest[0], cases + rest[1:], board=board, tag=tag or '$HEAD8', mustpass=mustpass,
                  rows_only=rows_only)
    print('\n'.join(out))
    return rc


if __name__ == '__main__':
    sys.exit(main(sys.argv))
