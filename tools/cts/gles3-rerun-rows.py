#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The TREE_ROWS of the AM62P dEQP-GLES3 tree that hold these cases.

  tools/cts/gles3-rerun-rows.py [options] (--family NAME [--tsv TSV] [--rules RULES] | --case GLOB)...

Prints the TREE_ROWS value of tools/cts/board-stage.sh and
tools/cts/am62p-deqp-images.sh (rows counted from 0 over the tree's
non-comment lines, image N is row N-1, the spec form 3,5-9) for the tree rows
holding the cases named by the arguments, so a board rerun after a host fix
builds and runs exactly the images its family lives in:

  TREE_ROWS=$(python3 tools/cts/gles3-rerun-rows.py --case 'dEQP-GLES3.functional.texture.filtering.3d.*')
  TREE_ROWS=$TREE_ROWS tools/cts/board-stage.sh am62p tree          # rerun-3d
  python3 tools/cts/gles3-rerun-rows.py --family '#1285-watchdog'   # a fixed family

Options (each given at most once; the defaults are the repo's own files):
  --tree FILE     the tree table (default tools/cts/am62p-gles3-tree.txt): one
                  image per non-comment line (the CONFIG_DEQP_CASE value,
                  comma-separated dEQP patterns), read with check-gles3-tree.py's
                  read_table: '# image <LABEL>:' labels the next row, other '#'
                  lines and blank lines are skipped, and an unlabelled row is
                  img<N> (N from 1) as am62p-deqp-images.sh names it.
  --mustpass FILE the mustpass list a --case glob expands against (default
                  tools/cts/gles3-mustpass.txt, 42,810 names).
  --tsv FILE      the not-pass list a --family reads (default
                  docs/es3/am62p-gles3-notpass.tsv): rows
                  "<outcome>\\t<case>[\\t<message>[\\t<family>]]", '#' headers.
  --rules FILE    the family rule file (default tools/cts/am62p-gles3-families.txt)
                  used for a TSV row that has no family column.

--case GLOB    a case name; with '*' it stands for the mustpass names it
               matches (a glob matching none is an error). The value is read as
               a dEQP filter pattern: '?' and '[' are literal characters.
--family NAME  every TSV row whose family is NAME: the row's 4th column when it
               has one (a filled column wins), else the family the rules give
               the row (gles3-triage.py's parse_families and family_of: fnmatch
               globs on the full case name, the first matching rule line wins,
               an optional message regex must match the message too). A NAME no
               TSV row has is an error.

Case-to-row matching is dEQP's own (its CasePaths rule, tcuCommandLine.cpp
without TCU_HIERARCHICAL_CASEPATHS) through check-gles3-tree.py's row_regex: a
filter line splits at ',' (nothing stripped) and a pattern matches the whole
name, '*' standing for any run of characters (dots included) and every other
character for itself. (gles3-triage.py --table reads a table with fnmatch;
here it is the tree the board runs, so the rows must agree with dEQP.)

stdout: one compact TREE_ROWS spec, adjacent rows merged into ranges, e.g.
        21-38 or 3,5-9,21-38, ready for TREE_ROWS=$(...).
stderr: one line per holding row, "row <N>  <label>  <count> case(s)"
        (the count is the distinct selected cases that row holds), and errors.
Exit status: 0 a spec was printed, 1 nothing was selected (a glob matching no
mustpass name, a family no TSV row has, a selected case no tree row holds),
2 usage or a missing file.

Why: the reruns after host fixes (rerun-3d, the c4 reruns) name tree rows, not
cases; the fixed cases are known per family from the not-pass list and the
rule file, and the row numbers can only come out of the tree file itself.
"""
import os
import sys
import importlib.util

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))


def load_helper(mod_name, filename):
    """Reuse a tools/cts helper by path: its file name has a hyphen, so a plain
    import cannot take it and importlib has to (this is the matcher and the
    rule matching of check-gles3-tree.py and gles3-triage.py, not copies)."""
    path = os.path.join(HERE, filename)
    if not os.path.isfile(path):
        raise SystemExit('gles3-rerun-rows: helper %s missing' % path)
    spec = importlib.util.spec_from_file_location(mod_name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cgt = load_helper('check_gles3_tree', 'check-gles3-tree.py')
triage = load_helper('gles3_triage', 'gles3-triage.py')


def rows_spec(indices):
    """[0, 2, 3, 4, 9] -> '0,2-4,9' (the 3,5-9 form of board.sh and
    am62p-deqp-images.sh in_rows, rows counted from 0)."""
    parts = []
    idx = sorted(set(indices))
    k = 0
    while k < len(idx):
        j = k
        while j + 1 < len(idx) and idx[j + 1] == idx[j] + 1:
            j += 1
        parts.append(str(idx[k]) if j == k else '%d-%d' % (idx[k], idx[j]))
        k = j + 1
    return ','.join(parts)


def main(argv):
    args = argv[1:]
    tree = os.path.join(HERE, 'am62p-gles3-tree.txt')
    mustpass = os.path.join(HERE, 'gles3-mustpass.txt')
    tsv = os.path.join(REPO, 'docs', 'es3', 'am62p-gles3-notpass.tsv')
    rules = os.path.join(HERE, 'am62p-gles3-families.txt')
    specs = []  # ('case', glob) / ('family', name), in argument order
    usage = 'usage: gles3-rerun-rows.py [--tree F] [--mustpass F]' \
            ' (--family NAME [--tsv F] [--rules F] | --case GLOB)...'

    def bad_usage(msg):
        print('gles3-rerun-rows: %s\n%s' % (msg, usage), file=sys.stderr)
        return 2

    k = 0
    try:
        while k < len(args):
            a = args[k]
            if a in ('--tree', '--mustpass', '--tsv', '--rules'):
                v = args[k + 1]
                k += 2
                if a == '--tree':
                    tree = v
                elif a == '--mustpass':
                    mustpass = v
                elif a == '--tsv':
                    tsv = v
                else:
                    rules = v
            elif a in ('--case', '--family'):
                specs.append(('case' if a == '--case' else 'family', args[k + 1]))
                k += 2
            else:
                return bad_usage('unknown argument %s' % a)
    except IndexError:
        return bad_usage('%s needs a value' % args[-1])
    if not specs:
        return bad_usage('no --case or --family given')

    for kind, path in (('tree', tree), ('mustpass', mustpass), ('not-pass TSV', tsv), ('rules', rules)):
        want = (kind == 'tree'
                or (kind == 'mustpass' and any(c == 'case' and '*' in v for c, v in specs))
                or (kind in ('not-pass TSV', 'rules') and any(c == 'family' for c, _ in specs)))
        if want and not os.path.isfile(path):
            return bad_usage('no such %s file: %s' % (kind, path))

    table_text = open(tree, errors='replace').read()
    rows = cgt.read_table(table_text)  # [(label, note, filter)], unlabelled rows are img<N>
    if not rows:
        return bad_usage('the tree %s has no filter rows' % tree)
    matchers = [cgt.row_regex(filt) for _lab, _note, filt in rows]

    must_names = blob = None

    def mustpass_names():
        nonlocal must_names, blob
        if must_names is None:
            must_names = [l.strip() for l in open(mustpass, errors='replace')
                          if l.strip() and not l.startswith('#')]
            blob = '\n'.join(must_names)
        return blob

    tsv_rows = rules_list = None

    def family_data():
        nonlocal tsv_rows, rules_list
        if tsv_rows is None:
            tsv_rows = triage.parse_notpass(tsv)     # (outcome, case, message, family)
            rules_list = triage.parse_families(rules)
        return tsv_rows, rules_list

    selected = []   # case names, in argument order, deduplicated below
    for kind, val in specs:
        if kind == 'case':
            if '*' in val:
                hits = [m.group(0) for m in cgt.row_regex(val).finditer(mustpass_names())]
                if not hits:
                    print("gles3-rerun-rows: --case '%s': the glob matches no name of %s"
                          % (val, mustpass), file=sys.stderr)
                    return 1
                selected.extend(hits)
            else:
                selected.append(val)
        else:
            trows, rl = family_data()
            known = {r[0] for r in rl} | {r[3] for r in trows if r[3]}
            hits = [case for _o, case, msg, fam in trows
                    if (fam or (triage.family_of(rl, case, msg) if rl else '')) == val]
            if not hits:
                print("gles3-rerun-rows: --family '%s': no row of %s has this family%s"
                      % (val, tsv,
                         '' if val in known else ' (no rule of %s names it either)' % rules),
                      file=sys.stderr)
                return 1
            selected.extend(hits)

    seen = set()
    selected = [n for n in selected if not (n in seen or seen.add(n))]

    held = {}  # row index -> [selected cases]
    uncovered = []
    for n in selected:
        hit = [i for i, m in enumerate(matchers) if m.match(n)]
        if not hit:
            uncovered.append(n)
        for i in hit:
            held.setdefault(i, []).append(n)
    if uncovered:
        for n in uncovered[:5]:
            print('no tree row holds %s' % n, file=sys.stderr)
        print('gles3-rerun-rows: %d of the %d selected cases are held by no row of %s'
              % (len(uncovered), len(selected), tree), file=sys.stderr)
        return 1

    for i in sorted(held):
        print('row %d  %s  %d case%s' % (i, rows[i][0], len(held[i]),
                                         '' if len(held[i]) == 1 else 's'), file=sys.stderr)
    print(rows_spec(held))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
