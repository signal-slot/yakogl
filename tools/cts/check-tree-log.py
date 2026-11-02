#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check that board slices together record a whole AM62P dEQP-GLES3 tree.

    tools/cts/check-tree-log.py [--rows 46] SLICE_DIR...

Slice directory (tools/cts/log/<YYYY-MM-DD>-am62p-gles3-<sha8>-rows<AA>-<BB>/):

    README.md   a line "commit: <sha8>" (the main commit every image was built from,
                equal to the sha8 in the directory name), and for each row of
                tools/cts/am62p-gles3-tree.txt in AA..BB exactly one verdict line:

                  row <NN>: board.sh am62p <name> rc=0 verdict=done boot=<backend> row
                      boot_verdict=<v> result=<...> resets=0 record=<...> back_after=<s>s

                i.e. "row NN: " followed by the line tools/board.sh prints, unchanged
                (one physical line).
    cases.tsv   <case> TAB <status> [TAB <first message line>] per case, read by
                am62p-gles3-merge.py.

Fails (exit 1, one message per problem) unless the slices together hold exactly one
verdict line per row 1..N from one commit, every verdict is "verdict=done" with
"resets=0", and no file in a slice contains an IPv4 address, a host:port or a
/dev/tty (or /dev/serial) path. Board addresses and UARTs belong in the
environment only; redact them before committing.
"""
import argparse
import os
import re
import sys

SLICE = re.compile(r"^\d{4}-\d{2}-\d{2}-am62p-gles3-([0-9a-f]{8})-rows(\d\d)-(\d\d)$")
ROWLINE = re.compile(r"^row (\d+): (board\.sh .*)$")
OCTET = r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)"
IPV4 = re.compile(r"(?<![\d.])%s(?:\.%s){3}(?![\d]|\.\d)" % (OCTET, OCTET))
HOSTPORT = re.compile(r"(?<![\w.-])([A-Za-z0-9][A-Za-z0-9_.-]*):(\d{2,5})(?!\d)")
SOURCE = re.compile(r"\.(c|cc|cpp|h|hpp|rs|py|sh|md|txt|cmake|s)$", re.I)
TTY = re.compile(r"/dev/(tty|serial|cu\.)")


def scan_text(name, text):
    bad = []
    for n, line in enumerate(text.splitlines(), 1):
        if IPV4.search(line):
            bad.append("%s:%d: contains an IPv4 address" % (name, n))
        if TTY.search(line):
            bad.append("%s:%d: contains a /dev/tty path" % (name, n))
        for m in HOSTPORT.finditer(line):
            host = m.group(1)
            # file.cpp:123 and 12:34:56 stamps are not hosts
            if SOURCE.search(host) or host.isdigit():
                continue
            bad.append("%s:%d: contains a host:port (%s:%s)" % (name, n, host, m.group(2)))
    return bad


def field(v, key):
    m = re.search(r" %s=(\S+)" % key, v)
    return m.group(1) if m else "absent"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("slices", nargs="+", metavar="SLICE_DIR")
    ap.add_argument("--rows", type=int, default=46)
    a = ap.parse_args()

    errs = []
    commits = set()
    rows = {}
    for d in a.slices:
        m = SLICE.match(os.path.basename(os.path.normpath(d)))
        if not m:
            errs.append("%s: name is not <date>-am62p-gles3-<sha8>-rowsAA-BB" % d)
            continue
        sha, lo, hi = m.group(1), int(m.group(2)), int(m.group(3))
        commits.add(sha)
        readme = os.path.join(d, "README.md")
        if not os.path.isfile(readme):
            errs.append("%s: no README.md" % d)
            continue
        if not os.path.isfile(os.path.join(d, "cases.tsv")):
            errs.append("%s: no cases.tsv" % d)
        with open(readme, encoding="utf-8", errors="replace") as f:
            text = f.read()
        said = re.findall(r"^commit: *([0-9a-f]+)\s*$", text, re.M)
        if said != [sha]:
            errs.append("%s: README.md 'commit:' line is %s, the directory name says %s" % (d, said or "missing", sha))
        mine = set()
        for n, line in enumerate(text.splitlines(), 1):
            if not line.startswith("row "):
                continue
            r = ROWLINE.match(line)
            if not r:
                errs.append("%s/README.md:%d: not 'row NN: board.sh ...'" % (d, n))
                continue
            row, v = int(r.group(1)), r.group(2)
            if row in rows or row in mine:
                errs.append("row %d has more than one verdict line (%s, %s)" % (row, rows[row][0] if row in rows else d, d))
            mine.add(row)
            rows[row] = (d, v)
            if not lo <= row <= hi:
                errs.append("%s: row %d is outside the directory's rows %d-%d" % (d, row, lo, hi))
            if field(v, "verdict") != "done":
                errs.append("row %d (%s): verdict is %s, not done" % (row, d, field(v, "verdict")))
            if field(v, "resets") != "0":
                errs.append("row %d (%s): resets is %s, not 0" % (row, d, field(v, "resets")))
        gone = sorted(set(range(lo, hi + 1)) - mine)
        if gone:
            errs.append("%s: no verdict line for row(s) %s of its rows %d-%d" % (d, ", ".join(map(str, gone)), lo, hi))
        for root, _, files in os.walk(d):
            for fn in sorted(files):
                p = os.path.join(root, fn)
                with open(p, "rb") as fh:
                    errs += scan_text(p, fh.read().decode("utf-8", "replace"))

    if len(commits) > 1:
        errs.append("the slices name different commits: %s" % ", ".join(sorted(commits)))
    missing = [r for r in range(1, a.rows + 1) if r not in rows]
    if missing:
        errs.append("no verdict line for row(s) %s of 1..%d" % (", ".join(map(str, missing)), a.rows))
    extra = sorted(r for r in rows if r < 1 or r > a.rows)
    if extra:
        errs.append("verdict line(s) for row(s) %s outside 1..%d" % (", ".join(map(str, extra)), a.rows))

    if errs:
        for e in errs:
            sys.stderr.write("check-tree-log: %s\n" % e)
        sys.stderr.write("check-tree-log: FAIL (%d problem(s))\n" % len(errs))
        sys.exit(1)
    print("check-tree-log: ok, %d rows from commit %s in %d slice(s), all done with 0 resets" % (
        len(rows), commits.pop(), len(a.slices)))


if __name__ == "__main__":
    main()
