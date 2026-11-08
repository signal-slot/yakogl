#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""A tree table of the Lost cases of NanoPC-T6 dEQP-GLES2 slices, for a rerun.

  python3 -I tools/cts/mali-rerun-tree.py [--mustpass FILE] [--max N]
                                          [--reason R]... SLICE... > rerun.txt

A slice is a tools/cts/log/<date>-t6-gles2-<session>-<sha8>/ directory (format:
the docstring of mali-deqp-tally.py). The table holds every case that has
status Lost in a results.tsv of the slices (once, however many slices list it),
in mustpass order, in the form tools/cts/mali-deqp-run.sh reads with --tree
(the form of mali-gles2-tree.txt, checked by check-gles3-tree.py):

  # image T6RR<nnn>: <cases>
  <case>,<case>,...

Labels are T6RR001, T6RR002, ...: [A-Za-z0-9]+ as the runner demands, and no
label of mali-gles2-tree.txt (T6S<session>R<row>), so a rerun chunk never
shares a kmsg file or a case list name with a first-run chunk. The runner
takes no session from labels; run the rows with its default session (adhoc) or
smoke, since S1..S4 name the first-run tree. A case with reason chunk-crash or
chunk-hang is a row of its own, so it cannot take neighbours down again;
board-lost and not-run cases fill rows of at most --max cases (default 400,
the --max the tree check uses), a row ending where an isolated case stands, so
the table keeps mustpass order. --reason R (repeatable) keeps only the Lost
cases with that reason (chunk-crash, chunk-hang, board-lost, not-run).

Cases are written by name, exactly; a name with a comma, a space or a '*' is
refused (the filter syntax could not carry it). A Lost case missing from the
mustpass list is an error (exit 1), a slice that cannot be read exits 2.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOST_REASONS = ("chunk-crash", "chunk-hang", "board-lost", "not-run")
ALONE = ("chunk-crash", "chunk-hang")


class Fatal(Exception):
    pass


def lost_cases(slice_dir):
    """{case: reason} of one slice's Lost results."""
    path = os.path.join(slice_dir.rstrip("/"), "results.tsv")
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    if not lines or lines[0].split("\t")[:2] != ["case", "status"]:
        raise Fatal("%s: header is not case<TAB>status<TAB>detail" % path)
    out = {}
    for n, ln in enumerate(lines[1:], 2):
        if not ln:
            continue
        f = ln.split("\t")
        if len(f) < 2:
            raise Fatal("%s:%d: not a case<TAB>status line" % (path, n))
        if f[1] != "Lost":
            continue
        reason = f[2] if len(f) > 2 else ""
        if reason not in LOST_REASONS:
            raise Fatal("%s:%d: Lost with reason '%s', not one of %s"
                        % (path, n, reason, ", ".join(LOST_REASONS)))
        out[f[0]] = reason
    return out


def build(slices, mustpass, max_cases=400, reasons=None):
    """The table's text for the Lost cases of the slices (list of dicts)."""
    lost = {}
    for s in slices:
        for case, reason in s.items():
            # A case Lost for different reasons keeps the stricter one.
            if case not in lost or (reason in ALONE and lost[case] not in ALONE):
                lost[case] = reason
    index = {name: i for i, name in enumerate(mustpass)}
    for case in lost:
        if case not in index:
            raise ValueError("Lost case %s is not in the mustpass list" % case)
        if not case or any(c in case for c in ", \t*"):
            raise ValueError("case '%s' cannot be written in a filter line" % case)
    if reasons:
        lost = {c: r for c, r in lost.items() if r in reasons}
    order = sorted(lost, key=index.get)
    rows, run = [], []
    for case in order:
        if lost[case] in ALONE:
            if run:
                rows.append(run)
                run = []
            rows.append([case])
        else:
            run.append(case)
            if len(run) >= max_cases:
                rows.append(run)
                run = []
    if run:
        rows.append(run)
    out = ["# The Lost cases of %d slice(s): %d case(s) in %d row(s), mustpass order."
           % (len(slices), len(order), len(rows)),
           "# One case per row for chunk-crash and chunk-hang; at most %d per row otherwise." % max_cases]
    for n, row in enumerate(rows, 1):
        out.append("# image T6RR%03d: %d" % (n, len(row)))
        out.append(",".join(row))
    return "\n".join(out) + "\n"


def main(argv):
    mustpass = os.path.join(HERE, "gles2-mustpass.txt")
    max_cases, reasons, slices = 400, [], []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--mustpass", "--max", "--reason"):
            if i + 1 >= len(argv):
                print("mali-rerun-tree: %s wants a value" % a, file=sys.stderr)
                return 2
            v = argv[i + 1]
            i += 2
            if a == "--mustpass":
                mustpass = v
            elif a == "--reason":
                if v not in LOST_REASONS:
                    print("mali-rerun-tree: --reason is one of %s" % ", ".join(LOST_REASONS), file=sys.stderr)
                    return 2
                reasons.append(v)
            else:
                if not v.isdigit() or int(v) < 1:
                    print("mali-rerun-tree: --max wants a positive number", file=sys.stderr)
                    return 2
                max_cases = int(v)
        elif a.startswith("-"):
            print("usage: mali-rerun-tree.py [--mustpass FILE] [--max N] [--reason R]... SLICE...", file=sys.stderr)
            return 2
        else:
            slices.append(a)
            i += 1
    if not slices:
        print("mali-rerun-tree: no slice given", file=sys.stderr)
        return 2
    try:
        with open(mustpass, encoding="utf-8") as f:
            names = [l.strip() for l in f if l.strip() and not l.startswith("#")]
        data = [lost_cases(s) for s in slices]
    except OSError as e:
        print("mali-rerun-tree: %s: %s" % (e.filename, e.strerror), file=sys.stderr)
        return 2
    except Fatal as e:
        print("mali-rerun-tree: %s" % e, file=sys.stderr)
        return 2
    try:
        text = build(data, names, max_cases, reasons)
    except ValueError as e:
        print("mali-rerun-tree: %s" % e, file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
