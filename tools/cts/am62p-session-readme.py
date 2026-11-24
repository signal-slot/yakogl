#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the README.md of an AM62P dEQP-GLES3 board record.

Usage:
  tools/cts/am62p-session-readme.py --commit <40-hex sha> [--table FILE]
      [--tree-rows SPEC] [--run-bound S] [--return-bound S] [--native-3d 0|1]
      [--env NAME]... [--note TEXT]... RECORD_DIR

RECORD_DIR is records/am62p/<YYYYMMDD-HHMM>-<name>-<sha8>/, the OUTDIR of
`tools/board.sh am62p tree TABLE OUTDIR`: board.sh's tree.txt (one
"name<TAB>image<TAB>selector<TAB>filter" line per row, in table order) and
one run directory per run, <YYYYMMDD>-<HHMM>-<name>-<sha8>/, each with
INDEX.txt (its "verdict: board.sh am62p <name> rc=.. verdict=.. ... resets=.."
line) and console.txt. The README goes to stdout. It holds:

  - "commit: <sha>", the line pi4-session-readme.py writes (console-redact.py
    --check-readme and check-tree-log.py read it);
  - TREE_ROWS (board.sh's 0-based row indexes, as given), the table file and
    its sha256;
  - RUN_BOUND, RETURN_BOUND and the DEQP_PVR_NATIVE_3D arm;
  - each env name of the session as "set" or "unset", never its value;
  - per row (1-based table position) the verdict and resets of its last run,
    as a table and as "row NN: board.sh ..." lines (check-tree-log.py's form);
  - gl_test cases 196-205 (the ES 3.0 image) when a run's console.txt has
    "gl_test: case N <name>: PASS|FAIL|TIMEOUT" lines for them.

--tree-rows, --run-bound, --return-bound and --native-3d default to the
environment's TREE_ROWS, RUN_BOUND, RETURN_BOUND and DEQP_PVR_NATIVE_3D
("unset" when absent). Every line read (tree.txt, INDEX.txt, the consoles
read for gl_test, the arguments) and the README itself are checked against
console-redact.py's address, UART and home patterns: any match prints the
file and line number (not the match) on stderr and exits 1 with nothing on
stdout. Redact the record (tools/cts/console-redact.py) first.
"""
import argparse
import hashlib
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
RUNDIR = re.compile(r"^(\d{8})-(\d{4})-(.+)-([0-9a-f]{8})$")
VLINE = re.compile(r"^verdict: (board\.sh am62p (\S+) .*)$", re.M)
GLCASE = re.compile(r"gl_test: case (\d+) ([^:\s]*): (PASS|FAIL|TIMEOUT)")
GL_ES3 = range(196, 206)
ENV_NAMES = ["BOARD_HOST", "BOARD_UART", "TREE_IMAGE_DIR", "TREE_ROWS", "TREE_RETRIES",
             "RUN_BOUND", "RETURN_BOUND", "HOLD_MAX", "DEQP_PVR_NATIVE_3D",
             "DEQP_PVR_INSTANCE_DIVISOR", "GLES3_WORK"]


def redact_patterns():
    """console-redact.py's ADDR, UART and HOME patterns (bytes regexes)."""
    spec = importlib.util.spec_from_file_location("console_redact", os.path.join(HERE, "console-redact.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return [("an IPv4-like address", mod.ADDR), ("a UART path", mod.UART), ("a home path", mod.HOME)]


PATTERNS = redact_patterns()


class Private(Exception):
    pass


def check_private(where, text):
    """Raise Private naming where (file:line) the first private-looking line is; never the match."""
    for n, line in enumerate(text.splitlines(), 1):
        b = line.encode("utf-8", "replace")
        for what, rx in PATTERNS:
            if rx.search(b):
                raise Private("%s:%d: looks like %s" % (where, n, what))


def read(path, rel):
    with open(path, "rb") as f:
        text = f.read().decode("utf-8", "replace")
    check_private(rel, text)
    return text


def gather(rec):
    """(row names in tree order, {name: [(stamp, rundir), ...]})"""
    names = []
    tp = os.path.join(rec, "tree.txt")
    if os.path.isfile(tp):
        for line in read(tp, "tree.txt").splitlines():
            if line.strip():
                names.append(line.split("\t")[0])
    runs = {}
    for e in sorted(os.listdir(rec)):
        m = RUNDIR.match(e)
        if m and os.path.isdir(os.path.join(rec, e)):
            runs.setdefault(m.group(3), []).append((m.group(1) + m.group(2), e))
    return names, runs


def run_verdict(rec, rundir):
    """(board.sh verdict line, verdict, resets) of one run directory; '-' when absent."""
    ip = os.path.join(rec, rundir, "INDEX.txt")
    if not os.path.isfile(ip):
        return None, "no INDEX.txt", "-"
    m = VLINE.search(read(ip, rundir + "/INDEX.txt"))
    if not m:
        return None, "no verdict line", "-"
    line = m.group(1)
    v = re.search(r" verdict=(\S+)", line)
    r = re.search(r" resets=(\S+)", line)
    return line, v.group(1) if v else "-", r.group(1) if r else "-"


def gl_cases(rec, runs):
    """{case: (name, result, rundir)} for gl_test cases 196-205 in any run's console.txt (last wins)."""
    out = {}
    for name in sorted(runs):
        for stamp, rd in sorted(runs[name]):
            cp = os.path.join(rec, rd, "console.txt")
            if not os.path.isfile(cp):
                continue
            with open(cp, "rb") as f:
                raw = f.read().decode("utf-8", "replace")
            if "gl_test: case" not in raw:
                continue
            check_private(rd + "/console.txt", raw)
            for m in GLCASE.finditer(raw):
                n = int(m.group(1))
                if n in GL_ES3:
                    out[n] = (m.group(2), m.group(3), rd)
    return out


def shown_path(p):
    a = os.path.abspath(p)
    if a.startswith(REPO + os.sep):
        return os.path.relpath(a, REPO)
    return os.path.basename(a)


def build(rec, commit, table, tree_rows, run_bound, return_bound, native3d, env_names, notes, environ):
    for i, s in enumerate([commit, tree_rows, run_bound, return_bound, native3d] + list(notes), 1):
        check_private("argument %d" % i, s)
    with open(table, "rb") as f:
        tsha = hashlib.sha256(f.read()).hexdigest()
    names, runs = gather(rec)
    order = names + sorted(n for n in runs if n not in names)
    name = os.path.basename(os.path.normpath(rec))
    out = [
        "<!-- SPDX-License-Identifier: Apache-2.0 -->",
        "<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->",
        "# AM62P dEQP-GLES3 record %s at %s" % (name, commit[:8]),
        "",
        "commit: %s" % commit,
        "",
        "- table: `%s` sha256 %s" % (shown_path(table), tsha),
        "- TREE_ROWS: %s (board.sh's 0-based row indexes; the row numbers below are 1-based)" % tree_rows,
        "- RUN_BOUND: %s" % run_bound,
        "- RETURN_BOUND: %s" % return_bound,
        "- DEQP_PVR_NATIVE_3D: %s" % native3d,
        "- env: " + ", ".join("%s %s" % (n, "set" if environ.get(n) else "unset") for n in env_names),
        "",
        "Per row, the last run of its name (run directories relative to this one, "
        "`<YYYYMMDD>-<HHMM>-<name>-<sha8>/`); '-' is unknown.",
        "",
        "| row | image | verdict | resets | runs | last run |",
        "|---|---|---|---|---|---|",
    ]
    lines = []
    for n in order:
        row = "%02d" % (names.index(n) + 1) if n in names else "-"
        if n not in runs:
            continue  # a row of the table not run in this session
        rs = sorted(runs[n])
        line, v, r = run_verdict(rec, rs[-1][1])
        out.append("| %s | %s | %s | %s | %d | `%s` |" % (row, n, v, r, len(rs), rs[-1][1]))
        if line and row != "-":
            lines.append("row %s: %s" % (row, line))
    if lines:
        out += ["", "board.sh verdict lines (check-tree-log.py's form):", "", "```"] + lines + ["```"]
    gl = gl_cases(rec, runs)
    if gl:
        out += ["", "gl_test ES 3.0 cases 196-205:", "", "| case | name | result | run |", "|---|---|---|---|"]
        for c in GL_ES3:
            nm, res, rd = gl.get(c, ("-", "no result", "-"))
            out.append("| %d | %s | %s | %s |" % (c, nm, res, "`%s`" % rd if rd != "-" else "-"))
    if notes:
        out += [""] + list(notes)
    text = "\n".join(out) + "\n"
    check_private("README", text)
    return text


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--commit", required=True, help="full 40-digit sha of the session's build")
    ap.add_argument("--table", default=os.path.join(HERE, "am62p-gles3-tree.txt"))
    ap.add_argument("--tree-rows", default=os.environ.get("TREE_ROWS") or "all")
    ap.add_argument("--run-bound", default=os.environ.get("RUN_BOUND") or "unset")
    ap.add_argument("--return-bound", default=os.environ.get("RETURN_BOUND") or "unset")
    ap.add_argument("--native-3d", default=os.environ.get("DEQP_PVR_NATIVE_3D") or "unset (library default)")
    ap.add_argument("--env", action="append", default=[], metavar="NAME",
                    help="an env name to report as set/unset (default: the session's list)")
    ap.add_argument("--note", action="append", default=[], help="a line of text at the end")
    ap.add_argument("dir")
    a = ap.parse_args()
    sha = a.commit.lower()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        ap.error("--commit needs a full 40-digit sha")
    if not os.path.isdir(a.dir):
        ap.error("no record directory %s" % a.dir)
    if not os.path.isfile(a.table):
        ap.error("no table %s" % a.table)
    try:
        text = build(a.dir, sha, a.table, a.tree_rows, a.run_bound, a.return_bound, a.native_3d,
                     a.env or ENV_NAMES, a.note, os.environ)
    except Private as e:
        sys.stderr.write("am62p-session-readme: refused: %s\n" % e)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
