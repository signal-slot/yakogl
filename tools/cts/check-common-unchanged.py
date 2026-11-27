#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Is the ES 2.0 common set on the AM62P unchanged against RESULTS.md?

    tools/cts/check-common-unchanged.py <stage output file | record dir> [RESULTS.md]
    tools/cts/check-common-unchanged.py --board <board> ...   (default am62p)

Input is either the saved output of `tools/cts/board-stage.sh am62p common`
(its common-tally lines: `GM glmark2 rc=0 verdict=done ...`,
`    glmark2 score N; scenes ran R, failed F, unsupported U; ...`,
`GT gl_test rc=0 verdict=done ...`, `    gl_test passed P of T ...; failing: ...`)
or the record directory records/<board>/common-<sha8> (tools/cts/common-tally.py
is run over it; CHECK_COMMON_TALLY names another tally program). RESULTS.md
defaults to the file beside this script. Its common table row for the board
gives the baseline: scenes, glmark2 failed count, gl_test passed of run.

Exit 0: no difference. Exit 1: every difference listed (a missing verdict, a
verdict other than done, glmark2 failed above 0 or fewer scenes than the row,
a gl_test failing case beyond the row's count, passed/run off the row).
Exit 2: the input or the row could not be read. Scores are not compared (a
frame rate is a measurement, not a verdict).
"""
import os
import re
import subprocess
import sys


def die(msg):
    print("check-common-unchanged: " + msg, file=sys.stderr)
    sys.exit(2)


def baseline(results, board):
    """(scenes, failed, passed, run, gt_failed) from the board's row of the common table."""
    want = {"am62p": "AM62P"}.get(board, board)
    for line in open(results, encoding="utf-8"):
        if not line.startswith("| " + want + " |"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 3:
            continue
        gm, gt = cells[1], cells[2]
        m = re.match(r"(\d+), (\d+) scenes, (\d+) failed", gm)
        g = re.match(r"(\d+) of (\d+)", gt)
        if not (m and g):
            continue
        gf = re.search(r"(\d+) failed", gt)
        return int(m.group(2)), int(m.group(3)), int(g.group(1)), int(g.group(2)), int(gf.group(1)) if gf else 0
    die("no '%s' common row in %s (expected '| %s | <score>, <n> scenes, <n> failed ... | <p> of <t> ... |')" % (want, results, want))


def read_text(src, board):
    if os.path.isdir(src):
        tally = os.environ.get("CHECK_COMMON_TALLY") or os.path.join(os.path.dirname(os.path.abspath(__file__)), "common-tally.py")
        p = subprocess.run([sys.executable, tally, board, src], capture_output=True, text=True)
        return p.stdout + p.stderr
    try:
        return open(src, encoding="utf-8", errors="replace").read()
    except OSError as e:
        die("cannot read %s: %s" % (src, e))


def check(text, base):
    scenes0, _failed0, passed0, run0, gtfail0 = base
    diffs = []
    verdicts = {}
    for m in re.finditer(r"^(GM|GT)\s+(glmark2|gl_test)\b.*?\bverdict=(\w+)", text, re.M):
        verdicts[m.group(1)] = m.group(3)
    for lab, name in (("GM", "glmark2"), ("GT", "gl_test")):
        v = verdicts.get(lab)
        if v is None:
            diffs.append("missing verdict: no '%s %s ... verdict=' line" % (lab, name))
        elif v != "done":
            diffs.append("%s %s verdict=%s, the row has done" % (lab, name, v))
    m = re.search(r"^\s+glmark2 score (\S+); scenes ran (\d+), failed (\d+), unsupported (\d+)", text, re.M)
    if not m:
        diffs.append("missing verdict: no 'glmark2 score ...; scenes ran ..., failed ...' line")
    else:
        ran, failed, unsup = int(m.group(2)), int(m.group(3)), int(m.group(4))
        if failed > 0:
            diffs.append("glmark2 failed %d scenes (must be 0)" % failed)
        if unsup > 0:
            diffs.append("glmark2 unsupported %d scenes (must be 0)" % unsup)
        if ran < scenes0:
            diffs.append("glmark2 ran %d scenes, the row has %d" % (ran, scenes0))
    m = re.search(r"^\s+gl_test passed (\d+) of (\d+)\b[^\n]*?failing: ([^\n]*)", text, re.M)
    if not m:
        diffs.append("missing verdict: no 'gl_test passed P of T ... failing: ...' line")
    else:
        p, t, fl = int(m.group(1)), int(m.group(2)), m.group(3).strip()
        cases = [] if fl.startswith("none") else re.findall(r"\d+", fl.split(" [")[0].split(", then")[0])
        nfail = max(len(cases), t - p)
        if nfail > gtfail0:
            diffs.append("gl_test new failing case(s): %s (%d failed, the row has %d)" % (",".join(cases) or "unnamed", nfail, gtfail0))
        if t < run0:
            diffs.append("gl_test ran %d cases, the row has %d" % (t, run0))
        if p < passed0 and nfail <= gtfail0:
            diffs.append("gl_test passed %d of %d, the row has %d of %d" % (p, t, passed0, run0))
    return diffs


def main(argv):
    board = "am62p"
    args = []
    i = 0
    while i < len(argv):
        if argv[i] == "--board" and i + 1 < len(argv):
            board = argv[i + 1]
            i += 2
        else:
            args.append(argv[i])
            i += 1
    if not 1 <= len(args) <= 2 or args[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    results = args[1] if len(args) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "RESULTS.md")
    base = baseline(results, board)
    diffs = check(read_text(args[0], board), base)
    if diffs:
        print("check-common-unchanged: %d difference(s) from the %s common row of %s:" % (len(diffs), board, results))
        for d in diffs:
            print("  - " + d)
        return 1
    print("check-common-unchanged: ok (%s: glmark2 0 failed, %d scenes; gl_test %d of %d)" % (board, base[0], base[2], base[3]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
