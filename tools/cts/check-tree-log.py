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

    tools/cts/check-tree-log.py --lossy [--tree TABLE] [--mustpass FILE] RECORD_DIR_OR_LOG...

The lossy check for an AM62P dEQP-GLES2 tree record (records/am62p/<stamp>-deqp-<sha8>/,
the 17 images of am62p-full-tree.txt, or the ms4 image of am62p-ms4-tree.txt). A
record directory holds one run directory per image (<date>-<time>-<name>-<sha8>/
console.txt) and the logs board.sh wrote; a plain file is a log of board.sh verdict
lines. tools/board.sh marks an image whose console lost result lines "verdict=done_lossy
... lost=<n>", yet board-stage.sh still ends "done" with exit 0. Here every image that has
a done_lossy verdict, a lost=<n> above 0, or a console whose own counters say it lost
cases gets one line naming it and its lost-line count. Exit 1 unless every case of
the image's row in TABLE (matched against the mustpass list) still has a result line in
its console ("lossy but complete" is exit 0). No lossy image: one ok line, exit 0.
TABLE is am62p-ms4-tree.txt for a record directory whose name has "ms4", else
am62p-full-tree.txt. Without --lossy nothing above applies.
"""
import argparse
import importlib.util
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


CTS = os.path.dirname(os.path.abspath(__file__))
VERDICT = re.compile(r"\bboard\.sh \S+ (\S+) .*\bverdict=(\S+)")
LOST_FIELD = re.compile(r" lost=(\d+)")
RUNDIR = re.compile(r"^\d{8}-\d{4}-(.+)-(?:[0-9a-f]{8}|probe)$")
IMGNAME = re.compile(r"^img(\d+)-")


def load_tally():
    spec = importlib.util.spec_from_file_location("deqp_uart_tally", os.path.join(CTS, "deqp-uart-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def table_rows(path):
    """[(label, [case pattern...])] per row of an image table, as board.sh's tree_rows names
    them: the last '# image X:' comment, else img<N>-<tag> (the tag is not compared)"""
    rows, label = [], None
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^# image ([A-Za-z0-9]+):", line)
            if m:
                label = m.group(1)
                continue
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            rows.append((label or "img%d" % (len(rows) + 1), line.strip().split(",")))
            label = None
    return rows


def row_of(rows, name):
    """index of the table row an image name belongs to, or None"""
    for i, (lab, _) in enumerate(rows):
        if name == lab:
            return i
    m = IMGNAME.match(name)
    if m and 1 <= int(m.group(1)) <= len(rows):
        return int(m.group(1)) - 1
    return None


def lossy_main(a):
    tally = load_tally()
    mustpass_path = a.mustpass or os.path.join(CTS, "gles2-mustpass.txt")
    with open(mustpass_path, encoding="utf-8") as f:
        mustpass = [l.strip() for l in f if l.strip() and not l.startswith("#")]
    images = {}  # name -> {"verdict": ..., "lost": n, "console": text or None}
    unattributed = []
    tables = set()

    def image(name):
        return images.setdefault(name, {"verdict": None, "lost": 0, "console": None})

    def read_verdicts(text, where):
        for line in text.splitlines():
            m = VERDICT.search(line)
            if m:
                im = image(m.group(1))
                im["verdict"] = m.group(2)
                lf = LOST_FIELD.search(line)
                if lf:
                    im["lost"] = max(im["lost"], int(lf.group(1)))
                continue
            m = re.search(r"Lost in capture: (\d+) cases", line)
            if m and int(m.group(1)) > 0:
                unattributed.append("%s: 'Lost in capture: %s cases' names no image" % (where, m.group(1)))

    def rd(p):
        with open(p, "rb") as fh:
            return fh.read().decode("utf-8", "replace")

    for path in a.slices:
        base = os.path.basename(os.path.normpath(path))
        if os.path.isdir(path):
            tables.add("am62p-ms4-tree.txt" if "ms4" in base else "am62p-full-tree.txt")
            for entry in sorted(os.listdir(path)):
                p = os.path.join(path, entry)
                m = RUNDIR.match(entry)
                if os.path.isdir(p) and m:
                    con = os.path.join(p, "console.txt")
                    if os.path.isfile(con):
                        im = image(m.group(1))
                        im["console"] = rd(con)
                        loss = tally.scan(im["console"])["loss"]
                        if loss:
                            im["lost"] = max(im["lost"], loss["lost"])
                    for f in sorted(os.listdir(p)):
                        fp = os.path.join(p, f)
                        if f != "console.txt" and f.endswith((".txt", ".log", ".out", ".md")) and os.path.isfile(fp):
                            read_verdicts(rd(fp), fp)
                elif os.path.isfile(p) and entry.endswith((".txt", ".log", ".out", ".md")):
                    read_verdicts(rd(p), p)
        elif os.path.isfile(path):
            tables.add("am62p-full-tree.txt")
            read_verdicts(rd(path), path)
        else:
            sys.stderr.write("check-tree-log: %s: no such file or directory\n" % path)
            return 1
    if a.tree:
        tables = {a.tree}
    if len(tables) != 1:
        sys.stderr.write("check-tree-log: --lossy: the records need different tables (%s); pass --tree\n" % ", ".join(sorted(tables)))
        return 1
    table = tables.pop()
    if not os.path.isfile(table):
        table = os.path.join(CTS, table)
    rows = table_rows(table)

    bad = list(unattributed)
    notes = []
    lossy = [n for n, im in images.items() if im["verdict"] == "done_lossy" or im["lost"] > 0]
    for name in sorted(lossy, key=lambda n: (row_of(rows, n) is None, row_of(rows, n) or 0, n)):
        im = images[name]
        idx = row_of(rows, name)
        who = "image %s (%s)" % (idx + 1 if idx is not None else "?", name)
        lost = im["lost"]
        if idx is None:
            bad.append("LOSSY: %s lost %d console line(s); it is not a row of %s, so its cases cannot be checked" % (who, lost, os.path.basename(table)))
            continue
        if im["console"] is None:
            bad.append("LOSSY: %s lost %d console line(s); no console.txt in the record, so its cases cannot be checked" % (who, lost))
            continue
        rx = tally.filter_regex(rows[idx][1])
        want = [c for c in mustpass if rx.match(c)]
        got = tally.scan(im["console"])["got"]
        gone = [c for c in want if c not in got]
        if gone:
            bad.append("LOSSY: %s lost %d console line(s) and %d of its %d expected cases have no result line (first: %s)"
                       % (who, lost, len(gone), len(want), ", ".join(gone[:3])))
        else:
            notes.append("lossy but complete: %s lost %d console line(s), all %d expected cases still have a result line"
                         % (who, lost, len(want)))
    for n in notes:
        print("check-tree-log: %s" % n)
    if bad:
        for e in bad:
            sys.stderr.write("check-tree-log: %s\n" % e)
        sys.stderr.write("check-tree-log: FAIL (%d problem(s))\n" % len(bad))
        return 1
    if not notes:
        print("check-tree-log: ok, no done_lossy image and no lost console line in %d image(s)" % len(images))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("slices", nargs="+", metavar="SLICE_DIR")
    ap.add_argument("--rows", type=int, default=46)
    ap.add_argument("--lossy", action="store_true", help="the lossy-image check of an AM62P dEQP-GLES2 record")
    ap.add_argument("--tree", help="--lossy: the image table (default by the record name)")
    ap.add_argument("--mustpass", help="--lossy: the mustpass list (default gles2-mustpass.txt)")
    a = ap.parse_args()
    if a.lossy:
        sys.exit(lossy_main(a))

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
