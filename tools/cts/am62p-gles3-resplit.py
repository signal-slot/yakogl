#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Turn an aborted or overrun AM62P dEQP-GLES3 row into a generator change.

  tools/cts/am62p-gles3-resplit.py [options] (--tally FILE | --refuse 'ROW CASE' | --overrun 'ROW CASE')...

The AM62P tree is generated (tools/cts/gen-am62p-gles3-tree.py), so a slow or
refusing group is split there (ISOLATE, WEIGHT, SPLIT, OWN), never in the
table (tools/cts/README.md, "The AM62P dEQP-GLES3 tree"). This reads what a
board row did and prints, on stdout, for each affected row:

  1. the cases that have a result, the refusing or overrunning case, and the
     mustpass cases never begun (dEQP runs a row's cases in mustpass order)
  2. the proposed generator change, a unified diff of the generator's source
     (the exact ISOLATE / OWN / SPLIT entry or WEIGHT value; the generator
     file is never written)
  3. TREE_ROWS for the rerun: the rows (0-based, as am62p-deqp-images.sh and
     board.sh read TREE_ROWS) of the table the changed generator would write
     that select the never-begun cases, computed by running the changed
     generator in memory; and the same with the refusing cases added

ROW is the row's number in the table, 1-based, as the images are named
(img22-<tag>, or just 22, or img22). Inputs:
  --tally FILE        a deqp-uart-tally.py output saying STOPPED (or FATAL);
                      the refusing case is its last "<Status> <case>" line,
                      the row comes from its "image <ROW>:" header comment or
                      from the file name (img22-x.tally.txt); any other tally
                      is left alone
  --refuse 'ROW CASE'   dEQP stopped at CASE (an abort)
  --overrun 'ROW CASE'  the 600 s watchdog reset the board while CASE ran
  --log FILE          a console log: per-case spans from its "Test case '..'"
                      lines (their HH:MM:SS stamps if they have any)
  --tree FILE, --mustpass FILE, --generator FILE   defaults: the files next to
                      this script
An unknown row, a case the row does not select (in the mustpass list) and a
row named twice exit 2; so does a bad command line.
"""
import difflib
import fnmatch
import importlib.util
import math
import os
import re
import sys
import types

HERE = os.path.dirname(os.path.abspath(__file__))
SAFE = 0.8  # share of the cases that ran in the time that the next image may hold


class Error(Exception):
    pass


def read_lines(path):
    with open(path) as f:
        return [l.rstrip('\n') for l in f if l.strip() and not l.startswith('#')]


def select(filt, mustpass):
    pats = filt.split(',')
    return [c for c in mustpass if any(fnmatch.fnmatchcase(c, p) for p in pats)]


def row_number(label):
    m = re.match(r'^(?:img)?(\d+)(?:[-.].*)?$', label)
    if not m:
        raise Error("unknown row '%s' (want the row number, 1-based: 22, img22, img22-tag)" % label)
    return int(m.group(1))


def load_generator(source, path):
    mod = types.ModuleType('gen_am62p_gles3_tree')
    mod.__file__ = path
    exec(compile(source, path, 'exec'), mod.__dict__)
    return mod


def parse_tally(path):
    text = open(path, errors='replace').read()
    if 'STOPPED' not in text and 'FATAL' not in text:
        return None
    m = re.search(r'image ([A-Za-z0-9-]+):', text)
    label = m.group(1) if m else os.path.basename(path).split('.')[0]
    cases = re.findall(r'^(?:\d\d:\d\d:\d\d )?(?:deqp: )?[A-Za-z]+ (dEQP-\S+)\s*$', text, re.M)
    if not cases:
        raise Error('%s: stopped, but no "<Status> <case>" line names the refusing case' % path)
    return label, cases[-1], 'abort'


def parse_log(path):
    """[(case, seconds or None)] in log order: the span runs to the next Test case line."""
    seen = []
    for l in open(path, errors='replace'):
        m = re.search(r"(?:^|\s)(?:(\d\d):(\d\d):(\d\d)\s)?.*?Test case '([^']+)'", l)
        if m:
            t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3)) if m.group(1) else None
            seen.append((m.group(4), t))
    spans = []
    for i, (c, t) in enumerate(seen):
        nxt = seen[i + 1][1] if i + 1 < len(seen) else None
        spans.append((c, (nxt - t) % 86400 if t is not None and nxt is not None else None))
    return spans


# ---- the proposal: edits of the generator's sets and WEIGHT ----

def edit_source(src, edits):
    """The generator source with edits applied: ('add', SETNAME, unit) or
    ('weight', prefix, value)."""
    for op, name, val in edits:
        if op == 'add':
            m = re.search(r'^%s = \{\n(.*?)^\}' % name, src, re.M | re.S)
            if not m:
                raise Error('the generator has no %s set to edit' % name)
            if "'%s'," % val in m.group(1):
                continue
            src = src[:m.end(1)] + "    '%s',\n" % val + src[m.end(1):]
        else:
            m = re.search(r'^WEIGHT = \{\n(.*?)^\}', src, re.M | re.S)
            if not m:
                raise Error('the generator has no WEIGHT table to edit')
            body, line = m.group(1), "    '%s': %d,\n" % (name, val)
            old = re.search(r"^    '%s': \d+,\n" % re.escape(name), body, re.M)
            body = body[:old.start()] + line + body[old.end():] if old else body + line
            src = src[:m.start(1)] + body + src[m.end(1):]
    return src


def unit_of(gen, cases, case):
    for u, _ in gen.units(cases):
        if case.startswith(gen.ROOT + u + '.'):
            return u
    raise Error('no unit of the generator holds %s' % case)


def propose(gen, cases, case, kind, ran, rowcases):
    """([edits], [explanation lines]) for the refusing case."""
    unit = unit_of(gen, cases, case)
    why = []
    prefix = next((p for p in gen.WEIGHT if unit.startswith(p)), None)
    unit_cases = [c for c in cases if c.startswith(gen.ROOT + unit + '.')]
    rowunits = {unit_of(gen, cases, c) for c in rowcases}
    ran_unit = [c for c in ran if c.startswith(gen.ROOT + unit + '.')]
    safe = max(1, int(len(ran_unit) * SAFE))
    if unit in gen.ISOLATE and case.endswith(gen.ISOLATE_SUFFIX):
        if len(rowcases) == 1:
            return [], ['%s is already an image of its own (ISOLATE, ISOLATE_SUFFIX %s): the generator has nothing'
                        % (case, gen.ISOLATE_SUFFIX),
                        'finer to give; the case itself hangs or is slow (docs/notes/pvr-3d-mipmap-hang.md)']
    if unit in gen.ISOLATE:
        return [], ['row of %s: the unit is ISOLATE d and the slow case is not a %s one, so no entry of the'
                    % (unit, gen.ISOLATE_SUFFIX),
                    'generator isolates it; change ISOLATE_SUFFIX (one tail for all units) or add a NARROW list by hand']
    if prefix:
        w = gen.WEIGHT[prefix]
        want = max(math.ceil(gen.MAX_CASES / safe), w + 1)
        edits = [('weight', prefix, want)]
        why.append('%s is weighted %d (WEIGHT[%r]): %d of its cases ran in the row, so an image should hold about %d of them'
                   % (unit, w, prefix, len(ran_unit), safe))
        why.append('raise WEIGHT[%r] %d -> %d (MAX_CASES %d / %d cases an image may hold)' % (prefix, w, want, gen.MAX_CASES, safe))
        if len(rowunits) > 1:
            why.append('or, a split by function: add %r to OWN (an image for %s alone, %d cases)' % (unit, unit, len(unit_cases)))
        else:
            why.append('or, a split by function: %s is already the whole row, so OWN gains nothing; the weight is the knob'
                       % unit)
        return edits, why
    deeper = any(len(c[len(gen.ROOT):].split('.')) > len(unit.split('.')) + 1 for c in unit_cases)
    if unit not in gen.SPLIT and deeper:
        return [('add', 'SPLIT', unit)], ['%s (%d cases) shares the row with %d other unit(s) and has name levels below it:'
                                         % (unit, len(unit_cases), len(rowunits) - 1),
                                         'divide it by its next name level (SPLIT), so the slow subgroup is packed on its own']
    if len(rowunits) > 1 and unit not in gen.OWN:
        return [('add', 'OWN', unit)], ['%s (%d cases) shares the row with %d other unit(s): give it an image of its own (OWN)'
                                        % (unit, len(unit_cases), len(rowunits) - 1)]
    return [], ['%s is already the whole row and cannot be divided further (SPLIT / OWN do nothing): no generator entry helps;'
                % unit, 'a NARROW list or ISOLATE-style entry for the slow case is a code change, not a value']


def ranges(nums):
    out, nums = [], sorted(nums)
    i = 0
    while i < len(nums):
        j = i
        while j + 1 < len(nums) and nums[j + 1] == nums[j] + 1:
            j += 1
        out.append(str(nums[i]) if i == j else '%d-%d' % (nums[i], nums[j]))
        i = j + 1
    return ','.join(out)


def new_rows(gen, cases, wanted):
    """0-based indexes of the rows of gen's table that select any case of wanted."""
    lines = [l for l in gen.render(cases).splitlines() if l and not l.startswith('#')]
    hit = [i for i, l in enumerate(lines) if any(fnmatch.fnmatchcase(c, p) for c in wanted for p in l.split(','))]
    return hit, len(lines)


def run(args, out):
    opt = {'--tree': os.path.join(HERE, 'am62p-gles3-tree.txt'), '--mustpass': os.path.join(HERE, 'gles3-mustpass.txt'),
           '--generator': os.path.join(HERE, 'gen-am62p-gles3-tree.py')}
    refs, logs, tallies = [], [], []
    i = 0
    while i < len(args):
        a = args[i]
        if a in ('--tree', '--mustpass', '--generator', '--tally', '--refuse', '--overrun', '--log') and i + 1 < len(args):
            v = args[i + 1]
            i += 2
            if a in opt:
                opt[a] = v
            elif a == '--tally':
                tallies.append(v)
            elif a == '--log':
                logs.append(v)
            else:
                f = v.split()
                if len(f) != 2:
                    raise Error("%s wants 'ROW CASE', got '%s'" % (a, v))
                refs.append((f[0], f[1], 'abort' if a == '--refuse' else 'overrun'))
        else:
            raise UsageError(a)
    for t in tallies:
        r = parse_tally(t)
        if r:
            refs.append(r)
        else:
            print('%s: not stopped, left alone' % t, file=out)
    if not refs:
        if tallies:
            return 0
        raise UsageError('')
    mustpass = read_lines(opt['--mustpass'])
    rows = read_lines(opt['--tree'])
    path = opt['--generator']
    src = open(path).read()
    gen = load_generator(src, path)
    spans = {}
    for lf in logs:
        for c, s in parse_log(lf):
            spans[c] = s
    seen, plan = set(), []
    for lab, case, kind in refs:
        n = row_number(lab)
        if not 1 <= n <= len(rows):
            raise Error("unknown row '%s' (the table has rows 1-%d)" % (lab, len(rows)))
        if n in seen:
            raise Error('row %d is named twice' % n)
        seen.add(n)
        rowcases = select(rows[n - 1], mustpass)
        if case not in rowcases:
            raise Error('row %d does not select %s (a mustpass case of its filter)' % (n, case))
        plan.append((n, case, kind, rowcases))
    edits, never_all, refusing = [], [], []
    for n, case, kind, rowcases in plan:
        k = rowcases.index(case)
        ran, after = rowcases[:k], rowcases[k + 1:]
        print('== row %d (%s at case %d of %d)' % (n, 'abort' if kind == 'abort' else 'overrun of the 600 s watchdog',
                                                           k + 1, len(rowcases)), file=out)
        print('   with a result (%d): %s' % (len(ran), ('%s .. %s' % (ran[0], ran[-1])) if ran else '-'), file=out)
        print('   %s: %s' % ('refusing case' if kind == 'abort' else 'overrunning case', case), file=out)
        print('   never begun (%d): %s' % (len(after), ('%s .. %s' % (after[0], after[-1])) if after else '-'), file=out)
        timed = [(c, spans[c]) for c in rowcases if spans.get(c) is not None]
        if timed:
            tot = sum(s for _, s in timed)
            print('   log: %d timed cases, %d s in all, %.2f s a case' % (len(timed), tot, tot / len(timed)), file=out)
            for c, s in sorted(timed, key=lambda x: -x[1])[:3]:
                print('        slowest %4d s  %s' % (s, c), file=out)
        elif spans and any(c in spans for c in rowcases):
            print('   log: %d "Test case" lines of this row, no time stamps' % sum(c in spans for c in rowcases), file=out)
        e, why = propose(gen, mustpass, case, kind, ran, rowcases)
        for w in why:
            print('   proposal: ' + w, file=out)
        edits += e
        never_all += after
        refusing.append(case)
    print('== proposed generator change (tools/cts/gen-am62p-gles3-tree.py; not written)', file=out)
    new_src = edit_source(src, edits)
    if new_src == src:
        print('   none: the generator already gives these cases the finest split it has', file=out)
    else:
        for l in difflib.unified_diff(src.splitlines(True), new_src.splitlines(True),
                                      'a/tools/cts/gen-am62p-gles3-tree.py', 'b/tools/cts/gen-am62p-gles3-tree.py', n=2):
            out.write(l if l.endswith('\n') else l + '\n')
    new = load_generator(new_src, path)
    hit_n, total = new_rows(new, mustpass, never_all)
    hit_r, _ = new_rows(new, mustpass, never_all + refusing)
    cur, _ = new_rows(gen, mustpass, never_all + refusing)
    print('== rerun rows of the table that generator writes (%d rows; 0-based, TREE_ROWS of board.sh am62p tree)' % total, file=out)
    print('TREE_ROWS=%s' % (ranges(hit_n) if hit_n else '(none: no case is never begun)'), file=out)
    print('TREE_ROWS=%s   # with the refusing case(s) added, which has no result either' % ranges(hit_r), file=out)
    if new_src != src:
        print('   (regenerate tools/cts/am62p-gles3-tree.txt with the change first; today the same cases are in 0-based rows %s)'
              % ranges(cur), file=out)
    return 0


class UsageError(Exception):
    pass


def main(argv):
    try:
        return run(argv[1:], sys.stdout)
    except UsageError:
        print(__doc__, file=sys.stderr)
        return 2
    except (Error, OSError, ValueError) as e:
        print('am62p-gles3-resplit: %s' % e, file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv))
