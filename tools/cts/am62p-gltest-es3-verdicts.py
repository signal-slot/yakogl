#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""am62p-gltest-es3-verdicts.py: PASS/FAIL, verdict and resets of the AM62P gl_test ES 3.0 cases.

usage: am62p-gltest-es3-verdicts.py [--cases 196,197,...] LOG [LOG...]

LOG is the serial console of the AM62P gl_test image built with
CONFIG_GL_TEST_CASES=196,...,205 (board.sh / board-stage.sh am62p common; tools/cts/README.md,
'ES 3.0 gl_test cases'). With several logs (a rerun) a case is judged from the last log that has
it, and the resets an earlier log saw in it are added. One line per listed case, in list order:

  case <n> <name> PASS|FAIL|SKIP|MISSING verdict=<done|hang|reset|none> resets=<k> <reading>

then one '# run: ...' line per log (the run's own figures: 'gl_test: all:', 'GPU recoveries N',
board.sh's verdict line when the log holds it; its resets= above 0 fails the run, absent
does not).

A case's lines are those since the previous case's verdict line: gl_test prints a case's checks
first and the runner its 'gl_test: case N name: PASS|FAIL|TIMEOUT' line last
(driver/tools/pvrlab/runner/src/lib.rs).
  verdict=done   the case's verdict or skip line is there and no GPU reset fell inside the case
  verdict=hang   TIMEOUT, a fence timeout, a stall dump, or 'not run' after a second timeout
  verdict=reset  a GPU recovery in the case's lines ('FWCCB: CONTEXT_RESET', 'render: HWR during
                 fence ... (counter A -> B)'), or a Zephyr boot banner that cut the case short
                 (the next listed case after the last verdict line; the restart counts 1)
  verdict=none   no trace of the case
resets is the larger of the CONTEXT_RESET count and the HWR counter's rise in the case, plus the
restarts. PASS needs the verdict line PASS, resets 0 and the reading gl_test.c's checks make
(every expect_*() prints '... ok|MISMATCH  [check N]'):
  196 occl      'gl_test: occl: qK: available 1 result R' for q0..q4 with R = 0,1,0,1,1; 2/2 checks
  197 vtxgrid   'gl_test: vtxgrid lod K ...: 0 of N pixels wrong' for lod 0..3 (no check lines)
  198 a2cconst  pictures 1 and 2 '0 of N pixels hold green', '0 of N pixels differ'; 2/2 checks
  199 es3trunc  2/2 checks      200 intrt  16/16      201 usamp  8/8
  202 blitconv  12/12           203 tfcap  9/9        204 srgb   8/8     205 mappbo  15/15
A skip ('gl_test: case N name: SKIP (...)', the runner's 'gl_test: case N skipped: ...'; each case
skips by name in an ES 2.0 context) is SKIP, not a pass.

Exit 0: every listed case PASS with resets=0, and no log's run figure ('GPU recoveries N',
board.sh's resets=) above 0. 1: any other result. 2: an unreadable log, no log, or a bad --cases.
The patterns are imported: the verdict line, CONTEXT_RESET, HWR, fence timeout, stall and the
recoveries figure from tools/cts/pr-verdict.py; plain_lines and the 'gl_test: all:' line from
tools/cts/common-tally.py; board.sh's verdict line and its fields from tools/cts/check-tree-log.py.
The selftest is tools/cts/am62p-gltest-es3-verdicts-selftest.sh.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def load(fname, modname):
    spec = importlib.util.spec_from_file_location(modname, os.path.join(HERE, fname))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


PRV = load("pr-verdict.py", "pr_verdict")
CT = load("common-tally.py", "common_tally")
CTL = load("check-tree-log.py", "check_tree_log")

# driver/tools/pvrlab/gl_test.c, the case-name table: [196] = "occl" ... [205] = "mappbo"
NAMES = {196: "occl", 197: "vtxgrid", 198: "a2cconst", 199: "es3trunc", 200: "intrt",
         201: "usamp", 202: "blitconv", 203: "tfcap", 204: "srgb", 205: "mappbo"}
# the '[check N]' lines each case prints when it runs to its end (gl_test.c, counted per body)
CHECKS = {196: 2, 197: 0, 198: 2, 199: 2, 200: 16, 201: 8, 202: 12, 203: 9, 204: 8, 205: 15}
OCCL_WANT = (0, 1, 0, 1, 1)
LODS = 4

# the runner's skip of a walk: 'gl_test: case N skipped[: reason]' (runner/src/lib.rs skip_line)
SKIPPED_RE = re.compile(r"gl_test: case (\d+) skipped\b")
# gl_test.c run_case(): after a second fence timeout the later cases are not run
NOTRUN_RE = re.compile(r"gl_test: case (\d+): not run, the run ended at a second fence timeout")
# gl_test.c run_case(): a fence wait timed out inside the case
CASE_TIMEOUT_RE = re.compile(r"gl_test: case (\d+): \d+ fence wait\(s\) timed out")
# every expect_*() and tfcap_floats(): '...  ok|MISMATCH  [check N]'
CHECK_RE = re.compile(r"gl_test:\s+(.*?)\s+(ok|MISMATCH)\s+\[check (\d+)\]\s*$")
# case_occl(): 'gl_test: occl: <what>: available A result R (want 1 W)'
OCCL_RE = re.compile(r"gl_test: occl: .*: available (\d+) result (\d+) \(want 1 (\d+)\)")
# case_vtxgrid(): 'gl_test: vtxgrid lod K (level K, SxS): B of N pixels wrong'
LOD_RE = re.compile(r"gl_test: vtxgrid lod (\d+) \(.*\): (\d+) of (\d+) pixels wrong")
# case_a2cconst(): 'picture P: G of N pixels hold green', 'a2cconst: D of N pixels differ ...'
GREEN_RE = re.compile(r"gl_test: a2cconst picture (\d+): (\d+) of (\d+) pixels hold green")
DIFF_RE = re.compile(r"gl_test: a2cconst: (\d+) of (\d+) pixels differ between the two pictures")
# render.rs wait_for(): 'render: HWR during fence F (counter A -> B)' (board.sh pvr_resets reads A, B)
HWR_COUNTER_RE = re.compile(r"\(counter (\d+) -> (\d+)\)")
# the Zephyr banner board.sh reads ('Booting Zephyr OS build ...'): the image (re)started
BOOT_RE = re.compile(r"\*\*\* Booting Zephyr OS\b")


class Seg:
    """the lines of one case: from the previous verdict line to its own"""

    def __init__(self):
        self.checks = []  # (number, ok, what)
        self.occl = []
        self.lods = []
        self.green = []
        self.diff = []
        self.ctx = self.hwr = 0
        self.hang = False

    def touched(self):
        return bool(self.checks or self.occl or self.lods or self.green or self.diff
                    or self.ctx or self.hwr or self.hang)

    def resets(self):
        return max(self.ctx, self.hwr)


class Got:
    """one case's outcome in one log"""

    def __init__(self, word, name, seg, restarts=0, note=""):
        self.word, self.name, self.seg, self.restarts, self.note = word, name, seg, restarts, note

    def resets(self):
        return self.seg.resets() + self.restarts


def read_log(path):
    with open(path, "rb") as fh:
        return fh.read().decode("utf-8", errors="replace")


def scan(text, cases):
    """({case: Got}, run figures) for one log"""
    got, seg, figure, alls, board, ended, last = {}, Seg(), None, None, None, False, None
    for line in CT.plain_lines(text):
        if BOOT_RE.search(line):
            if seg.touched() and not ended:
                # the image restarted inside a case: the next listed case after the last verdict
                i = cases.index(last) + 1 if last in cases else 0
                if i < len(cases):
                    c = cases[i]
                    prev = got.get(c)
                    carry = prev.resets() if prev else 0
                    got[c] = Got("CUT", NAMES.get(c, "?"), seg, restarts=1 + carry,
                                 note="the image restarted inside the case")
            seg, ended, last = Seg(), False, None
            continue
        if PRV.RESET_RE.search(line):
            seg.ctx += 1
        if PRV.HWR_FENCE_RE.search(line):
            d = HWR_COUNTER_RE.search(line)
            seg.hwr += max(int(d.group(2)) - int(d.group(1)), 1) if d else 1
        if PRV.TIMEOUT_RE.search(line) or PRV.STALL_RE.search(line) or CASE_TIMEOUT_RE.search(line):
            seg.hang = True
        m = CHECK_RE.search(line)
        if m:
            seg.checks.append((int(m.group(3)), m.group(2) == "ok", " ".join(m.group(1).split())))
        m = OCCL_RE.search(line)
        if m:
            seg.occl.append((int(m.group(1)), int(m.group(2))))
        m = LOD_RE.search(line)
        if m:
            seg.lods.append(int(m.group(2)))
        m = GREEN_RE.search(line)
        if m:
            seg.green.append(int(m.group(2)))
        m = DIFF_RE.search(line)
        if m:
            seg.diff.append(int(m.group(1)))
        m = PRV.GROW_RE.search(line)
        if m:
            figure = int(m.group(4))
        m = CT.ALL_RE.search(line)
        if m:
            alls = line[m.start():].strip()
            ended = True
        m = CTL.VERDICT.search(line)
        if m:
            board = (m.group(2), CTL.field(line, "resets"))
        m = NOTRUN_RE.search(line)
        if m:
            c = int(m.group(1))
            got[c] = Got("NOTRUN", NAMES.get(c, "?"), Seg(), note="not run: the run ended at a second fence timeout")
            continue
        m = PRV.CASE_RE.search(line)
        word = name = None
        if m:
            c, name, word = int(m.group(1)), m.group(2), m.group(3)
        else:
            m = SKIPPED_RE.search(line)
            if m:
                c, word = int(m.group(1)), "SKIP"
        if word:
            prev = got.get(c)
            carry = prev.resets() if prev else 0
            got[c] = Got(word, name or NAMES.get(c, "?"), seg, restarts=carry,
                         note=("reran after %d reset(s) earlier in the log" % carry) if carry else "")
            seg, last = Seg(), c
    return got, (figure, alls, board)


def reading(case, seg):
    """(text, good): what the case's lines read against gl_test.c's checks"""
    parts, good = [], True
    if case == 196:
        av = ",".join(str(a) for a, _ in seg.occl) or "none"
        rs = ",".join(str(r) for _, r in seg.occl) or "none"
        parts.append("occl q0-q4 available %s results %s" % (av, rs))
        good &= (len(seg.occl) == 5 and all(a == 1 for a, _ in seg.occl)
                 and tuple(r for _, r in seg.occl) == OCCL_WANT)
    if case == 197:
        parts.append("%d of %d lods with 0 pixels wrong" % (sum(1 for b in seg.lods if b == 0), LODS))
        good &= len(seg.lods) == LODS and not any(seg.lods)
    if case == 198:
        parts.append("pictures holding green %s, differing pixels %s" % (
            ",".join(map(str, seg.green)) or "none", ",".join(map(str, seg.diff)) or "none"))
        good &= len(seg.green) == 2 and not any(seg.green) and len(seg.diff) == 1 and not seg.diff[0]
    want = CHECKS.get(case)
    ok = sum(1 for _, k, _ in seg.checks if k)
    n = len(seg.checks)
    if want or n:
        t = "checks %d/%d ok" % (ok, n)
        if want is not None and n != want:
            t += " (want %d/%d)" % (want, want)
        bad = [(no, what) for no, k, what in seg.checks if not k]
        if bad:
            t += ", first MISMATCH check %d '%s'" % bad[0]
        parts.append(t)
    good &= ok == n and (want is None or n == want)
    return "; ".join(parts), good


def judge(cases, texts):
    """(lines, rc) for the case list over the logs' texts, oldest first"""
    final, runs = {}, []
    for text in texts:
        got, run = scan(text, cases)
        for c, g in got.items():
            prev = final.get(c)
            if prev and prev.resets() and g.word != "CUT":
                g.restarts += prev.resets()
                g.note = (g.note + "; " if g.note else "") + "%d reset(s) in an earlier log" % prev.resets()
            final[c] = g
        runs.append(run)
    lines, rc = [], 0
    for c in cases:
        name = NAMES.get(c, "?")
        g = final.get(c)
        if g is None:
            lines.append("case %d %s MISSING verdict=none resets=0 no 'gl_test: case %d' line" % (c, name, c))
            rc = 1
            continue
        resets = g.resets()
        text, good = reading(c, g.seg)
        if g.word == "NOTRUN":
            status, v, text = "MISSING", "hang", g.note
        elif g.word == "CUT":
            status, v, text = "FAIL", "reset", "no verdict line, %s; %s" % (g.note, text)
        elif g.word == "SKIP":
            status, v, text = "SKIP", "done", "skipped by name (an ES 2.0 context?)"
        elif g.word == "TIMEOUT" or (g.seg.hang and g.word != "PASS"):
            status, v, text = "FAIL", "hang", "verdict line %s; %s" % (g.word, text)
        elif resets:
            status, v, text = "FAIL", "reset", "verdict line %s; %s" % (g.word, text)
        elif g.word == "PASS" and good:
            status, v = "PASS", "done"
        else:
            status, v = "FAIL", "done"
            text = ("verdict line PASS but the reading is not the expected one: " if g.word == "PASS"
                    else "verdict line %s; " % g.word) + text
        if g.name not in (name, "?"):
            status = "FAIL" if status == "PASS" else status
            text += "; the log names it %s" % g.name
        if g.note and g.word not in ("NOTRUN", "CUT"):
            text += "; " + g.note
        lines.append("case %d %s %s verdict=%s resets=%d %s" % (c, name, status, v, resets, text))
        if status != "PASS":
            rc = 1
    for i, (figure, alls, board) in enumerate(runs):
        t = "# run: log %d of %d: GPU recoveries %s" % (i + 1, len(runs), "?" if figure is None else figure)
        if figure:
            rc = 1
        t += "; " + (alls or "no 'gl_test: all:' line")
        if board:
            t += "; board.sh verdict=%s resets=%s" % board
            if board[1] not in ("0", "absent"):
                rc = 1
        lines.append(t)
    return lines, rc


def main(argv):
    args = argv[1:]
    cases = sorted(NAMES)
    if args and args[0] == "--cases":
        try:
            cases = [int(x) for x in args[1].split(",") if x.strip()]
        except (IndexError, ValueError):
            cases = []
        if not cases or len(set(cases)) != len(cases):
            print("am62p-gltest-es3-verdicts: bad --cases", file=sys.stderr)
            return 2
        args = args[2:]
    if not args or any(a.startswith("-") for a in args):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    texts = []
    for p in args:
        try:
            texts.append(read_log(p))
        except OSError as e:
            print("am62p-gltest-es3-verdicts: cannot read a log: %s" % (e.strerror or e), file=sys.stderr)
            return 2
    lines, rc = judge(cases, texts)
    print("\n".join(lines))
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
