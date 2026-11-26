#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""A tree table of the Lost cases of Mali dEQP-GLES2 slices, for a rerun.

  python3 -I tools/cts/mali-rerun-tree.py [--mustpass FILE] [--max N]
                                          [--prefix P] [--reason R]...
                                          SLICE... > rerun.txt

A slice is a tools/cts/log/<date>-t6-gles2-<session>-<sha8>/ directory (format:
the docstring of mali-deqp-tally.py). The table holds every case that has
status Lost in a results.tsv of the slices (once, however many slices list it),
in mustpass order, in the form tools/cts/mali-deqp-run.sh reads with --tree
(the form of mali-gles2-tree.txt, checked by check-gles3-tree.py):

  # image T6RR<nnn>: <cases>
  <case>,<case>,...

Board and label prefix: the board of a slice is the board= key of its
meta.txt (mali-deqp-tally.py); no meta.txt or no key means the NanoPC-T6
(nanopc-t6). '_' in the key is read as '-': verdin_imx95, the Zephyr
board name the tally and the triage slices of the iMX95 carry
(testdata/mali-fail-triage/imx95), is the same board as verdin-imx95, so
slices mixing the two spellings are one board, not a mix of boards. The
table takes its prefix from it: T6RR for the T6, I95RR for
board=verdin-imx95 (tools/cts/imx95-deqp-images.sh builds one Zephyr
image per row from such a table: its labels are [A-Za-z0-9]+ too, and
neither T6S<s>R<r> of the first-run tree nor T6RR collide with
I95RR<nnn>). Slices of different boards are refused (exit 2), and so is
a board without a prefix of its own unless --prefix P is given; --prefix
P ([A-Za-z0-9]+, not T6S<s>R<r>) sets the prefix whatever the board. On
Zephyr a hang ends the whole image, so on the iMX95 the hang and crash
rows stay one case each, as everywhere. --max on the iMX95 is at most
400 (the default): a row's cases run inside one
image's lifetime (verdin_imx95.conf: 600 s, WDOG3), and the 64 MiB GL
carve-out and the 256 MiB window do not grow with the case count, so the
tree's own row size, which mali-gles2-tree.txt and its check already hold,
is the one measured to fit; a larger --max exits 2.

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
import re
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


BOARD_PREFIX = {"nanopc-t6": "T6RR", "verdin-imx95": "I95RR"}
MAX_ROW = {"verdin-imx95": 400}


def slice_board(slice_dir):
    """The board= of a slice's meta.txt, read with '_' as '-'
    (verdin_imx95 is verdin-imx95); nanopc-t6 when there is none."""
    path = os.path.join(slice_dir.rstrip("/"), "meta.txt")
    try:
        with open(path, encoding="utf-8") as f:
            for ln in f.read().split("\n"):
                if ln.startswith("board="):
                    return ln[6:].strip().replace("_", "-") or "nanopc-t6"
    except FileNotFoundError:
        pass
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    return "nanopc-t6"


def build(slices, mustpass, max_cases=400, reasons=None, prefix="T6RR"):
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
        out.append("# image %s%03d: %d" % (prefix, n, len(row)))
        out.append(",".join(row))
    return "\n".join(out) + "\n"


def main(argv):
    mustpass = os.path.join(HERE, "gles2-mustpass.txt")
    max_cases, reasons, slices, prefix = 400, [], [], None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--mustpass", "--max", "--reason", "--prefix"):
            if i + 1 >= len(argv):
                print("mali-rerun-tree: %s wants a value" % a, file=sys.stderr)
                return 2
            v = argv[i + 1]
            i += 2
            if a == "--mustpass":
                mustpass = v
            elif a == "--prefix":
                if not re.fullmatch(r"[A-Za-z0-9]+", v) or re.fullmatch(r"T6S[0-9]+R[0-9]+", v):
                    print("mali-rerun-tree: --prefix is [A-Za-z0-9]+ and not a T6S<s>R<r> label", file=sys.stderr)
                    return 2
                prefix = v
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
            print("usage: mali-rerun-tree.py [--mustpass FILE] [--max N] [--prefix P] [--reason R]... SLICE...", file=sys.stderr)
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
        boards = sorted({slice_board(s) for s in slices})
    except OSError as e:
        print("mali-rerun-tree: %s: %s" % (e.filename, e.strerror), file=sys.stderr)
        return 2
    except Fatal as e:
        print("mali-rerun-tree: %s" % e, file=sys.stderr)
        return 2
    if len(boards) > 1:
        print("mali-rerun-tree: slices of different boards (%s)" % ", ".join(boards), file=sys.stderr)
        return 2
    board = boards[0]
    if prefix is None:
        prefix = BOARD_PREFIX.get(board)
        if prefix is None:
            print("mali-rerun-tree: board '%s' has no label prefix, give --prefix" % board, file=sys.stderr)
            return 2
    if board in MAX_ROW and max_cases > MAX_ROW[board]:
        print("mali-rerun-tree: --max for %s is at most %d" % (board, MAX_ROW[board]), file=sys.stderr)
        return 2
    try:
        text = build(data, names, max_cases, reasons, prefix)
    except ValueError as e:
        print("mali-rerun-tree: %s" % e, file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
