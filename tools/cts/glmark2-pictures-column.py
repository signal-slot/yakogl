#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Replace one column of tools/cts/glmark2-pictures.txt from a glmark2 run.

Usage:
  glmark2-pictures-column.py <table> <column> <console log or run dir>
      --commit <8 hex> --record records/<board>/<name> --header-text <text>
      [--out <file> | --in-place] [--check]
  glmark2-pictures-column.py <table> <column> --check

The 33 lines come from `common-tally.py --gm-picture-lines <src> <column>`
(this tool does not parse the console itself). They must be exactly the 33
default scenes, in the order of the table's host column (of the existing
column when the table has no host column), each once. The column's lines and
its single "# <column>:" header are replaced in place; every other line is
kept byte for byte. Without --out and --in-place the new table goes to stdout.

Refused (exit 1): a header text without the commit and the record, a commit
that is not 8 hex digits, a record that is not under records/, a missing,
extra or duplicate scene, a scene out of order, and any text (header or
line) matching the redaction regex
  ([0-9]{1,3}\\.){3}[0-9]{1,3}|/home/|/dev/tty|/Users/

--check validates the column of an existing table only: 33 lines in the host
column's order, one header with an 8-hex commit and a records/ path, nothing
matching the redaction regex. It prints the state; exit 0 when it is sound,
1 when not. Exit 2 for a usage error or a run with no picture lines.
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TALLY = os.path.join(HERE, "common-tally.py")
SCENES = 33
REDACT_RE = re.compile(r"([0-9]{1,3}\.){3}[0-9]{1,3}|/home/|/dev/tty|/Users/")
COMMIT_RE = re.compile(r"(?<![0-9a-f])[0-9a-f]{8}(?![0-9a-f])")
RECORD_RE = re.compile(r"records/[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]*[A-Za-z0-9_-]")


class Refused(Exception):
    pass


def usage(msg=None):
    if msg:
        print("glmark2-pictures-column: %s" % msg, file=sys.stderr)
    print(__doc__.split("\n\n")[1].rstrip(), file=sys.stderr)
    sys.exit(2)


def column_parts(lines, column):
    """(header indexes, line indexes) of a column in a table's lines"""
    head = [i for i, l in enumerate(lines) if l.startswith("# %s:" % column)]
    rows = [i for i, l in enumerate(lines) if l.startswith("%s | " % column)]
    return head, rows


def scene_of(line):
    return line.split(" | ", 2)[1]


def reference_order(lines, column):
    """the scene order the column must have: the host column's, else the column's own"""
    host = [scene_of(lines[i]) for i in column_parts(lines, "host")[1]]
    return host if host else [scene_of(lines[i]) for i in column_parts(lines, column)[1]]


def redaction(text, what):
    m = REDACT_RE.search(text)
    if m:
        raise Refused("%s matches the redaction regex (%r)" % (what, m.group(0)))


def check_header(header, column):
    text = header[len("# %s:" % column):]
    redaction(header, "the header")
    if not COMMIT_RE.search(text):
        raise Refused("the header has no 8-hex commit")
    if not RECORD_RE.search(text):
        raise Refused("the header has no records/ path")


def check_scenes(scenes, order):
    seen = set()
    for s in scenes:
        if s in seen:
            raise Refused("duplicate scene %s" % s)
        seen.add(s)
    missing = [s for s in order if s not in seen]
    extra = [s for s in scenes if s not in order]
    if missing:
        raise Refused("missing scene(s): %s" % "; ".join(missing))
    if extra:
        raise Refused("scene(s) not in the table's order: %s" % "; ".join(extra))
    if scenes != order:
        raise Refused("scenes are not in the host column's order")
    if len(scenes) != SCENES:
        raise Refused("%d scenes, not %d" % (len(scenes), SCENES))


def check_table(lines, column):
    head, rows = column_parts(lines, column)
    if len(head) != 1:
        raise Refused("%d '# %s:' header lines, not 1" % (len(head), column))
    if len(rows) != SCENES:
        raise Refused("%d '%s |' lines, not %d" % (len(rows), column, SCENES))
    check_header(lines[head[0]], column)
    for i in rows:
        redaction(lines[i], "line %d" % (i + 1))
    check_scenes([scene_of(lines[i]) for i in rows], reference_order(lines, column))


def run_tally(src, column):
    p = subprocess.run([sys.executable, TALLY, "--gm-picture-lines", src, column],
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stderr)
        sys.exit(2)
    sys.stderr.write(p.stderr)  # its notes on scenes with no value
    return [l for l in p.stdout.splitlines() if l.startswith("%s | " % column)]


def replace(lines, column, new, header):
    head, rows = column_parts(lines, column)
    if len(head) != 1:
        raise Refused("the table has %d '# %s:' header lines, not 1" % (len(head), column))
    if len(rows) != SCENES:
        raise Refused("the table's %s column has %d lines, not %d" % (column, len(rows), SCENES))
    out = list(lines)
    out[head[0]] = header
    for i, l in zip(rows, new):
        out[i] = l
    return out


def main(argv):
    pos, opt = [], {}
    flags = {"--check", "--in-place"}
    withval = {"--commit", "--record", "--header-text", "--out"}
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in flags:
            opt[a] = True
        elif a in withval:
            if i + 1 >= len(argv):
                usage("%s needs a value" % a)
            i += 1
            opt[a] = argv[i]
        elif a.startswith("--"):
            usage("unknown option %s" % a)
        else:
            pos.append(a)
        i += 1
    check = "--check" in opt
    if len(pos) not in ((2, 3) if check else (3,)):
        usage("wrong arguments")
    if "--out" in opt and "--in-place" in opt:
        usage("--out and --in-place exclude each other")
    table, column = pos[0], pos[1]
    with open(table, encoding="utf-8", newline="") as f:
        data = f.read()
    # keep every line's own ending: split on \n only, rejoin the same way
    lines = data.split("\n")
    trailing = lines[-1] == ""
    if trailing:
        lines.pop()

    try:
        if check and len(pos) == 2:
            check_table(lines, column)
            head, _ = column_parts(lines, column)
            print("glmark2-pictures-column: %s: %s ok (%d lines, header %s)"
                  % (table, column, SCENES, lines[head[0]][:60] + "..."))
            return 0

        for k in ("--commit", "--record", "--header-text"):
            if k not in opt:
                usage("%s is required" % k)
        commit, record, text = opt["--commit"], opt["--record"], opt["--header-text"]
        if not re.fullmatch(r"[0-9a-f]{8}", commit):
            raise Refused("--commit is not 8 hex digits")
        if not re.fullmatch(r"records/.+", record):
            raise Refused("--record is not under records/")
        redaction(text, "the header text")
        redaction(record, "the record")
        if commit not in text or record not in text:
            raise Refused("the header text does not name the commit and the record")
        header = "# %s: %s" % (column, text)
        check_header(header, column)

        new = run_tally(pos[2], column)
        for l in new:
            redaction(l, "a picture line")
        check_scenes([scene_of(l) for l in new], reference_order(lines, column))
        out = replace(lines, column, new, header)
        check_table(out, column)
    except Refused as e:
        print("glmark2-pictures-column: refused: %s" % e, file=sys.stderr)
        return 1

    result = "\n".join(out) + ("\n" if trailing else "")
    if check:  # --check with a run: validate only, write nothing
        print("glmark2-pictures-column: the new %s column is sound (%d lines)" % (column, SCENES))
    elif "--in-place" in opt:
        with open(table, "w", encoding="utf-8", newline="") as f:
            f.write(result)
    elif "--out" in opt:
        with open(opt["--out"], "w", encoding="utf-8", newline="") as f:
            f.write(result)
    else:
        sys.stdout.write(result)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
