#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Re-split tools/cts/pi4-gles3-tree.txt after dEQP aborted images.

  tools/cts/pi4-gles3-resplit.py [options] TREE --tally FILE... [-o OUT]
  tools/cts/pi4-gles3-resplit.py [options] TREE --refuse 'LABEL CASE'... [-o OUT]
  tools/cts/pi4-gles3-resplit.py [options] TREE --refuse-file FILE [-o OUT]

docs/es3/board-recipe-pi4.md section 6 ("An abort (at the refusing case)"),
as a program. Each aborted label L and its refusing case R (the case dEQP
stopped at) are split, the label's cases taken in mustpass order:

  L     the cases before R (they ran and have their result); the label is
        dropped when R was its first case (nothing ran before it)
  L1    R alone, a one-case image
  L2..  the cases after R (never begun), in as few images as the section 6
        sizes allow, of equal size

Input, one source or both together:
  --tally FILE   a deqp-uart-tally.py output (tools/cts/log/<session>/<LABEL>
                 .tally.txt); read only if it says STOPPED, R being its last
                 "<Status> <case>" line, L the "image <LABEL>:" of its header
                 comment (else the file name up to the first '.')
  --refuse 'LABEL CASE'   the same, given directly (repeatable)
  --refuse-file FILE      one 'LABEL CASE' per line, '#' lines and blanks skipped
A label refused twice, an unknown label, and a case the label does not select
(in the mustpass list) are errors; nothing is written then.

Options:
  --mustpass FILE   the case list (default tools/cts/gles3-mustpass.txt, in
                    dEQP's run order, which is the order the split follows)
  --at SHA8         the commit the abort was seen at, named in the notes
  --size N          cases an image may hold (default by group, section 6:
                    builtin_functions.precision 200, texture.filtering.3d 15,
                    shaders 1200, fragment_ops / ubo / uniform_api /
                    state_query 1000, the rest 700)
  -o OUT            write the tree there (default stdout)

New labels: a label ending in a digit gets a letter appended (S03 -> S03a,
as the b1b5b845 round did), one ending in a letter a number (S03a ->
S03a1, S03a2, ...), so a second round gives S03a1 -> S03a1a and the name
never reads two ways. A taken name is skipped. The image count in the
header ("in 114 images") is rewritten, every other line outside the
split labels is kept byte for byte, and each new filter line is as short as
the case set allows: "<prefix>.*" where every mustpass case under the
prefix is in the image, else the case name. Check with
tools/cts/check-gles3-tree.py <OUT>.
"""
import math
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LABEL_RE = re.compile(r'^# image ([^:]*):(.*)$')
FUNC = 'dEQP-GLES3.functional.'
# section 6 sizes: (case-name prefix, cases an image holds); first match wins
SIZES = [
    (FUNC + 'shaders.builtin_functions.precision.', 200),
    (FUNC + 'texture.filtering.3d.', 15),
    (FUNC + 'shaders.', 1200),
    (FUNC + 'fragment_ops.', 1000),
    (FUNC + 'ubo.', 1000),
    (FUNC + 'uniform_api.', 1000),
    (FUNC + 'state_query.', 1000),
]
DEFAULT_SIZE = 700


class Error(Exception):
    pass


def read_mustpass(path):
    return [l.strip() for l in open(path) if l.strip() and not l.startswith('#')]


def expand(filt, mustpass):
    """The mustpass cases (in list order) a CONFIG_DEQP_CASE filter selects."""
    alts = ['[^\n]*'.join(re.escape(p) for p in pat.split('*')) for pat in filt.split(',')]
    rx = re.compile('^(?:%s)$' % '|'.join(alts))
    return [c for c in mustpass if rx.match(c)]


def size_for(case, override=None):
    if override:
        return override
    for pre, n in SIZES:
        if case.startswith(pre):
            return n
    return DEFAULT_SIZE


def compress(chunk, mustpass, prefix_total):
    """A filter selecting exactly the mustpass cases of chunk: for each case
    the shortest dotted prefix X whose every mustpass case is in chunk gives
    'X.*', else the case's own name."""
    count = {}
    for c in chunk:
        parts = c.split('.')
        for i in range(1, len(parts)):
            p = '.'.join(parts[:i])
            count[p] = count.get(p, 0) + 1
    pats, done = [], set()
    for c in chunk:
        parts = c.split('.')
        pat = c
        for i in range(1, len(parts)):
            p = '.'.join(parts[:i])
            if count[p] == prefix_total[p]:
                pat = p + '.*'
                break
        if pat not in done:
            done.add(pat)
            pats.append(pat)
    return ','.join(pats)


def prefix_totals(mustpass):
    t = {}
    for c in mustpass:
        parts = c.split('.')
        for i in range(1, len(parts)):
            p = '.'.join(parts[:i])
            t[p] = t.get(p, 0) + 1
    return t


def next_label(label, taken):
    """label + a letter if it ends in a digit, + a number if it ends in a
    letter; the first suffix not taken."""
    if label[-1].isdigit():
        for ch in 'abcdefghijklmnopqrstuvwxyz':
            if label + ch not in taken:
                return label + ch
        raise Error('no free letter after label %s' % label)
    n = 1
    while label + str(n) in taken:
        n += 1
    return label + str(n)


def read_rows(text):
    """(lines, {label: (label_line_index, filter_line_index)})."""
    lines = text.split('\n')
    rows, lab = {}, None
    for i, l in enumerate(lines):
        m = LABEL_RE.match(l)
        if m:
            lab = (m.group(1), i)
            continue
        if re.match(r'\s*(#|$)', l):
            continue
        if lab is not None:
            rows[lab[0]] = (lab[1], i)
        lab = None
    return lines, rows


def parse_tally(path):
    text = open(path).read()
    if 'STOPPED' not in text:
        return None
    m = re.search(r'^# Pi 4 dEQP-GLES3 image ([A-Za-z0-9]+):', text, re.M)
    label = m.group(1) if m else os.path.basename(path).split('.')[0]
    cases = re.findall(r'^[A-Za-z]+ (dEQP-\S+)$', text, re.M)
    if not cases:
        raise Error('%s: STOPPED but no "<Status> <case>" line to name the refusing case' % path)
    return label, cases[-1]


def short(case):
    return case[len(FUNC):] if case.startswith(FUNC) else case


def resplit(tree_text, mustpass, refusals, at=None, size=None):
    """The re-split tree text; refusals is [(label, case)]."""
    lines, rows = read_rows(tree_text)
    seen = set()
    for lab, case in refusals:
        if lab in seen:
            raise Error('label %s is refused twice' % lab)
        seen.add(lab)
        if lab not in rows:
            raise Error('label %s is not in the tree' % lab)
    taken = set(rows)
    totals = prefix_totals(mustpass)
    where = ' at ' + at if at else ''
    repl = {}  # label line index -> [new lines]
    for lab, case in refusals:
        li, fi = rows[lab]
        cases = expand(lines[fi].strip(), mustpass)
        if case not in cases:
            raise Error('%s does not select %s (a mustpass case of its filter)' % (lab, case))
        i = cases.index(case)
        before, after = cases[:i], cases[i + 1:]
        ref = next_label(lab, taken)
        taken.add(ref)
        names = [ref]
        rest_out = []
        if after:
            limit = size_for(after[0], size)
            k = math.ceil(len(after) / limit)
            each = math.ceil(len(after) / k)
            for j in range(0, len(after), each):
                chunk = after[j:j + each]
                nl = next_label(lab, taken)
                taken.add(nl)
                names.append(nl)
                rest_out += ['# image %s: %d (the rest of %s after %s, never begun%s)'
                             % (nl, len(chunk), lab, short(case), where),
                             compress(chunk, mustpass, totals)]
        out = []
        if before:
            rest = names[1:]
            tail = ''
            if rest:
                tail = ', the rest in ' + (rest[0] if len(rest) == 1 else '%s-%s' % (rest[0], rest[-1]))
            out += ['# image %s: %d (the cases of %s that ran before %s%s; %s alone in %s%s)'
                    % (lab, len(before), lab, short(case), where, short(case), ref, tail),
                    compress(before, mustpass, totals)]
        out += ['# image %s: 1 (%s alone: dEQP stopped at it in %s%s)' % (ref, short(case), lab, where), case]
        out += rest_out
        repl[li] = out
        repl[fi] = None  # the old filter line goes
    new = []
    for i, l in enumerate(lines):
        if i in repl:
            if repl[i] is not None:
                new += repl[i]
            continue
        new.append(l)
    text = '\n'.join(new)
    n_images = len(read_rows(text)[1])
    head, sep, tail = text.partition('\n# image ')
    head, subs = re.subn(r'\bin \d+ images', 'in %d images' % n_images, head, count=1)
    if not subs:
        raise Error('the tree header names no "in <N> images" to update')
    return head + sep + tail


def main(argv):
    mustpass_path = os.path.join(HERE, 'gles3-mustpass.txt')
    at = out_path = tree = None
    size = None
    refusals = []
    tallies = []
    i = 1
    try:
        while i < len(argv):
            a = argv[i]
            if a in ('--mustpass', '--at', '--size', '-o', '--tally', '--refuse', '--refuse-file') and i + 1 < len(argv):
                v = argv[i + 1]
                i += 2
                if a == '--mustpass':
                    mustpass_path = v
                elif a == '--at':
                    at = v
                elif a == '--size':
                    size = int(v)
                elif a == '-o':
                    out_path = v
                elif a == '--tally':
                    tallies.append(v)
                elif a == '--refuse':
                    f = v.split()
                    if len(f) != 2:
                        raise Error("--refuse wants 'LABEL CASE', got '%s'" % v)
                    refusals.append((f[0], f[1]))
                else:
                    for l in open(v):
                        if l.strip() and not l.startswith('#'):
                            f = l.split()
                            if len(f) != 2:
                                raise Error("%s: want 'LABEL CASE', got '%s'" % (v, l.strip()))
                            refusals.append((f[0], f[1]))
            elif a.startswith('-') or tree is not None:
                print(__doc__, file=sys.stderr)
                return 2
            else:
                tree = a
                i += 1
        if tree is None or not (tallies or refusals):
            print(__doc__, file=sys.stderr)
            return 2
        for t in tallies:
            r = parse_tally(t)
            if r:
                refusals.append(r)
        text = resplit(open(tree).read(), read_mustpass(mustpass_path), refusals, at, size)
    except (Error, OSError, ValueError) as e:
        print('pi4-gles3-resplit: %s' % e, file=sys.stderr)
        return 1
    if out_path:
        with open(out_path, 'w') as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
