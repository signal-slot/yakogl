#!/usr/bin/env python3
"""am62p-gltest-es3-verdicts.py: PASS/FAIL, verdict and resets for the AM62P gl_test ES 3.0 cases.

usage: am62p-gltest-es3-verdicts.py [--cases 196,197,...] LOG [LOG...]
       am62p-gltest-es3-verdicts.py --selftest      (the synthetic fixtures; see the .sh beside it)

LOG is the serial console of a run of the gl_test image built with
CONFIG_GL_TEST_CASES=196,...,205 (tools/cts/README.md, 'ES 3.0 gl_test cases'). With several
logs a case's line comes from the last log that has it. One line per case:

  case <n> <name> PASS|FAIL|SKIP|MISSING verdict=<done|hang|reset|none> resets=<k> <reading>

verdict: done = the case's own 'gl_test: case N name: PASS|FAIL|SKIP' line, no reset in it;
hang = TIMEOUT, or a fence timeout / stall dump with no PASS; reset = a GPU recovery
(FWCCB CONTEXT_RESET, 'render: HWR during fence') inside the case, or a boot banner that
cut it short; none = no trace of the case. The case's lines are the ones since the previous
case's verdict line (gl_test prints a case's checks first, its verdict last).

PASS needs the verdict line PASS, resets 0 and the expected reading (gl_test.c):
  196 occl      q0..q4 'available 1' with results 0,1,0,1,1 (five lines)
  197 vtxgrid   every 'lod k ... 0 of N pixels wrong', no MISMATCH
  198-201       the case's checks all 'ok' (a2cconst, es3trunc, intrt, usamp)
  202 blitconv  12 of 12 checks ok      203 tfcap  9 of 9      204 srgb  8 of 8
  205 mappbo    15 of 15
A skip by name ('gl_test: case N name: SKIP (...)', an ES 2.0 context) is SKIP, not a pass.
The run's own recovery figure ('render freelists: ... GPU recoveries N') is also printed
and counts: N > 0 fails the run even when no case line shows it.

Exit 0: every listed case PASS with resets=0 and the run's recoveries 0. 1: any other
result. 2: an unreadable log (or no log, or a bad --cases). The patterns come from
tools/cts/pr-verdict.py (case line, reset, HWR, stall, timeout, recoveries figure) and
tools/cts/common-tally.py (plain_lines); the boot banner is board.sh's.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def load(fname, modname):
    spec = importlib.util.spec_from_file_location(modname, os.path.join(HERE, fname))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[modname] = mod
    spec.loader.exec_module(mod)
    return mod


PRV = load("pr-verdict.py", "pr_verdict")
CT = load("common-tally.py", "common_tally")

# the names gl_test.c's case table gives (driver/tools/pvrlab/gl_test.c, [196]..[205])
NAMES = {196: "occl", 197: "vtxgrid", 198: "a2cconst", 199: "es3trunc", 200: "intrt",
         201: "usamp", 202: "blitconv", 203: "tfcap", 204: "srgb", 205: "mappbo"}
# checks each case makes, tools/cts/README.md (others: judged by 'no MISMATCH' only)
EXPECT_CHECKS = {202: 12, 203: 9, 204: 8, 205: 15}
# every expect_*() prints '... ok|MISMATCH  [check N]' (gl_test.c)
CHECK_RE = re.compile(r"\[check (\d+)\]\s*$")
OCCL_RE = re.compile(r"gl_test: occl: .*: available (\d+) result (\d+) \(want 1 (\d+)\)")
LOD_RE = re.compile(r"gl_test: vtxgrid lod \d+ .*: (\d+) of (\d+) pixels wrong")
OCCL_WANT = (0, 1, 0, 1, 1)
# board.sh's boot line (its console grep): a Zephyr banner restarts the image
BOOT_RE = re.compile(r"Booting Zephyr OS")


def read_log(path):
    with open(path, encoding="utf-8", errors="replace") as fh:
        return fh.read()


class Seg:
    def __init__(self):
        self.ok = self.bad = 0
        self.occl = []
        self.lods = []
        self.ctx = self.hwr = 0
        self.hang = False
        self.cut = False

    def touched(self):
        return bool(self.ok or self.bad or self.occl or self.lods or self.ctx or self.hwr or self.hang)

    def resets(self):
        return max(self.ctx, self.hwr)


def segments(text):
    """({case number: (verdict word, name, Seg)}, the run's recoveries figure) for one log"""
    out, seg, carry, figure = {}, Seg(), 0, None
    for line in CT.plain_lines(text):
        if BOOT_RE.search(line):
            if seg.touched():
                carry += seg.resets() + 1  # the image restarted inside a case: its lines are lost
            seg = Seg()
            seg.cut = carry > 0
            continue
        if PRV.RESET_RE.search(line):
            seg.ctx += 1
        if PRV.HWR_FENCE_RE.search(line):
            d = re.search(r"\(counter (\d+) -> (\d+)\)", line)
            seg.hwr += (int(d.group(2)) - int(d.group(1))) if d else 1
        if PRV.TIMEOUT_RE.search(line) or PRV.STALL_RE.search(line):
            seg.hang = True
        if CHECK_RE.search(line):
            if " MISMATCH " in line:
                seg.bad += 1
            else:
                seg.ok += 1
        m = OCCL_RE.search(line)
        if m:
            seg.occl.append(tuple(int(x) for x in m.groups()))
        m = LOD_RE.search(line)
        if m:
            seg.lods.append((int(m.group(1)), int(m.group(2))))
        m = PRV.GROW_RE.search(line)
        if m:
            figure = int(m.group(4))
        m = PRV.CASE_RE.search(line)
        if m:
            case, name, word = int(m.group(1)), m.group(2), m.group(3)
            seg.ctx += carry
            carry = 0
            out[case] = (word, name, seg)
            seg = Seg()
    return out, figure


def reading(case, seg):
    """(reading text, whether it is the expected one) from the case's lines"""
    total = seg.ok + seg.bad
    parts = []
    good = True
    if case == 196:
        avail = "/".join(str(a) for a, _, _ in seg.occl) or "none"
        res = ",".join(str(r) for _, r, _ in seg.occl) or "none"
        parts.append("occl q0-q4 available %s results %s" % (avail, res))
        good &= (len(seg.occl) == 5 and all(a == 1 for a, _, _ in seg.occl)
                 and tuple(r for _, r, _ in seg.occl) == OCCL_WANT)
    if case == 197:
        wrong = sum(b for b, _ in seg.lods)
        parts.append("%d lod lines, %d pixels wrong" % (len(seg.lods), wrong))
        good &= bool(seg.lods) and wrong == 0
    if case not in (196, 197) or total:
        parts.append("%d of %d checks ok" % (seg.ok, total))
    want = EXPECT_CHECKS.get(case)
    if want is not None:
        good &= seg.ok == want and seg.bad == 0
        if total != want:
            parts.append("(want %d)" % want)
    else:
        good &= seg.bad == 0 and (case in (196, 197) or total > 0)
    return "; ".join(parts), good


def judge(cases, logs):
    """(lines, rc) for the case list over the logs' texts"""
    got, figures = {}, []
    for text in logs:
        segs, fig = segments(text)
        got.update(segs)
        if fig is not None:
            figures.append(fig)
    lines, rc = [], 0
    for c in cases:
        name = NAMES.get(c, "?")
        if c not in got:
            lines.append("case %d %s MISSING verdict=none resets=0 no 'gl_test: case %d' line" % (c, name, c))
            rc = 1
            continue
        word, name, seg = got[c]
        resets = seg.resets()
        text, good = reading(c, seg)
        if word == "SKIP":
            status, v, text = "SKIP", "done", "skipped by name (an ES 2.0 context?)"
        elif word == "TIMEOUT" or (seg.hang and word != "PASS"):
            status, v, text = "FAIL", "hang", "TIMEOUT; " + text
        elif resets:
            status, v, text = "FAIL", "reset", "verdict line %s; %s" % (word, text)
        elif word == "PASS" and good:
            status, v = "PASS", "done"
        else:
            status, v = "FAIL", "done"
            if word == "PASS":
                text = "verdict line PASS but the reading is wrong: " + text
        if seg.cut:
            text += "; image restarted before the case"
        lines.append("case %d %s %s verdict=%s resets=%d %s" % (c, name, status, v, resets, text))
        if status != "PASS":
            rc = 1
    if figures:
        lines.append("run: GPU recoveries %d (render freelists line)" % figures[-1])
        if figures[-1]:
            rc = 1
    else:
        lines.append("run: GPU recoveries ? (no 'render freelists' line)")
    return lines, rc


def main(argv):
    args = argv[1:]
    if args == ["--selftest"]:
        return selftest()
    if len(args) == 2 and args[0] == "--fixture":  # the synthetic log, for the shell selftest
        kw = {"clean": {}, "reset": {"reset": 203}, "missing": {"missing": 201}, "bad": {"bad": 204}}
        if args[1] not in kw:
            return 2
        sys.stdout.write(fixture(**kw[args[1]]))
        return 0
    cases = sorted(NAMES)
    if len(args) >= 2 and args[0] == "--cases":
        try:
            cases = [int(x) for x in args[1].split(",") if x]
        except ValueError:
            cases = []
        args = args[2:]
        if not cases:
            print("am62p-gltest-es3-verdicts: bad --cases", file=sys.stderr)
            return 2
    if not args:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    logs = []
    for p in args:
        try:
            logs.append(read_log(p))
        except OSError as e:
            print("am62p-gltest-es3-verdicts: %s" % e.strerror, file=sys.stderr)
            return 2
    lines, rc = judge(cases, logs)
    print("\n".join(lines))
    return rc


SERIAL = "[00:01:02.003,000] <inf> yakogl: "


def fixture(bad=None, skip=None, missing=None, reset=None):
    """a synthetic redacted log; the keywords break one case"""
    out = ["*** Booting Zephyr OS build test ***"]
    for c in sorted(NAMES):
        n = NAMES[c]
        if c == missing:
            continue
        if c == skip:
            out.append("gl_test: case %d %s: SKIP (an ES 2.0 library has no such thing)" % (c, n))
            continue
        if c == reset:
            out.append(SERIAL + "render: HWR during fence 7 (counter 0 -> 1)")
            out.append(SERIAL + "FWCCB: CONTEXT_RESET ctxid=3 reason=1 dm=3 jobref=1 flags=0x0 PC=0x0 MMU=0x0")
        if c == 196:
            for i, w in enumerate(OCCL_WANT):
                r = (1 - w) if bad == c and i == 1 else w
                out.append("gl_test: occl: q%d: available 1 result %d (want 1 %d)" % (i, r, w))
            nchk = 2
        elif c == 197:
            for k in range(4):
                out.append("gl_test: vtxgrid lod %d (level %d, 8x8): %d of 1024 pixels wrong"
                           % (k, k, 5 if bad == c and k == 2 else 0))
            nchk = 0
        else:
            nchk = EXPECT_CHECKS.get(c, 3)
        for k in range(nchk):
            mm = bad == c and k == nchk - 1 and c not in (196, 197)
            out.append("gl_test:   %-28s (%2d,%2d) = 0x%08x  want 0x%08x  %s  [check %d]"
                       % ("what", k, k, 0xffffffff, 0xffffffff, "MISMATCH" if mm else "ok", k + 1))
        out.append("gl_test: case %d %s: %s" % (c, n, "FAIL" if bad == c else "PASS"))
    n = 1 if reset else 0
    out.append(SERIAL + "render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; "
               "GPU recoveries %d (HWR counter %d)" % (n, n))
    return "\n".join(out) + "\n"


def selftest():
    fails = []
    allc = sorted(NAMES)

    def check(label, text, cases, rc, must=()):
        lines, got = judge(cases, [text])
        blob = "\n".join(lines)
        if got != rc or any(m not in blob for m in must):
            fails.append("%s: rc %d (want %d)\n%s" % (label, got, rc, blob))

    check("clean", fixture(), allc, 0, ("case 196 occl PASS verdict=done resets=0",
                                        "case 205 mappbo PASS", "15 of 15"))
    check("reset", fixture(reset=203), allc, 1, ("case 203 tfcap FAIL verdict=reset resets=1",))
    check("missing", fixture(missing=201), allc, 1, ("case 201 usamp MISSING verdict=none",))
    check("bad196", fixture(bad=196), allc, 1, ("case 196 occl FAIL verdict=done",))
    check("bad204", fixture(bad=204), allc, 1, ("case 204 srgb FAIL verdict=done", "7 of 8"))
    check("skip", fixture(skip=205), allc, 1, ("case 205 mappbo SKIP",))
    check("subset", fixture(bad=196), [202, 203], 0)
    for f in fails:
        print("FAIL " + f, file=sys.stderr)
    print("am62p-gltest-es3-verdicts selftest: %s" % ("FAILED" if fails else "ok"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
