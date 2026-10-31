#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""pr-verdict.py - the partial-render verdict lines from an AM62P serial log.

    python3 -I tools/cts/pr-verdict.py [--expect pr|stop] LOG CASE [CASE...]
    python3 -I tools/cts/pr-verdict.py --selftest

CASE is a gl_test case name (refusehang) or number (175); the forced-PR case
is named by the caller. One line per case:

    pr-verdict: case refusehang PASS partial_renders=1 oom=3 hwr=0 stopped=no

then 'pr-verdict: OK' or 'pr-verdict: FAIL <reason>'; exit 0 / 1. Exit 2: the
log or the arguments are unusable.

--expect pr    (default) the knob on, criteria c1/c2: every case PASS, the
               firmware's partial renders >= 1, GPU recoveries 0, no stop.
On a failure (and on a pass under --expect pr) one more line names the class
of the partial-render failure table, the first row that matches:

    pr-verdict: class 3 F1 page fault

Rows 1 success, 2 F8 corrupt frame, 3 F1 page fault, 4 F9 (CONTEXT_RESET
reason 16), 5 F2, 6 F3, 7 F7/F5 (the stop with no partial render; FENCE_PR
MET or UNMET), 8 stop cause 2 (knob probably off), 9 short-timeout stall, 10
F10 (bad cmd_size), 11 hang ended by the watchdog; 0 no row matches.

--expect stop  the knob-off negative: a 'render: stopped' line with cause 2
               (out of memory at the maximum) or 4 (a kick that did not
               complete) is recorded and is the expected outcome; the case
               verdict is printed, not judged.

The figures are the run's: the last 'render freelists' lines of the log (the
run-end report). A log with several runs should be cut first.
"""
import re
import sys

# 'gl_test: case N name: PASS|FAIL|TIMEOUT|SKIP (why)' - the runner's verdict
# line, driver/tools/pvrlab/runner/src/lib.rs (the writeln! with {verdict}).
CASE_RE = re.compile(r"gl_test: case (\d+) (\S+): (PASS|FAIL|TIMEOUT|SKIP)\b")
# 'render freelists: R grow requests, S served, U unserved, K KiB taken on
# demand; GPU recoveries N (HWR counter C)' -
# driver/rust/src/gpu/pvr/render.rs, grow_report().
GROW_RE = re.compile(
    r"render freelists: (\d+) grow requests, (\d+) served, (\d+) unserved, \d+ KiB taken on demand; "
    r"GPU recoveries (\d+) \(HWR counter (\d+)\)")
# '... firmware UPDATE_STATS: partial renders P, out of memory M, ...' -
# driver/rust/src/gpu/pvr/render.rs, grow_report() (names: ccb.rs STATS_NAMES).
STATS_RE = re.compile(r"firmware UPDATE_STATS: partial renders (\d+), out of memory (\d+)")
# 'render: stopped: <cause words> (list L, K KiB); no kick is waited ...' -
# driver/rust/src/gpu/pvr/freelist.rs, stop(); words from stop_name().
STOP_RE = re.compile(r"render: stopped: (.*?) \(list (\d+), (\d+) KiB\)")
# The cause numbers are board::record GPU_STOP_* (driver/rust/src/board/record.rs).
STOP_CAUSES = (
    ("the parameter manager out of memory with its freelist at the maximum", 2),
    ("a freelist grow past the host's table of grows", 3),
    ("a kick that did not complete in the stop's time", 4),
)
# 'GPU_STOP (20) tag 2: cause 2 (...) fence F GPU recoveries N ...' - the board
# record's decode, driver/rust/src/board/record.rs (used when the stop line is
# lost from the capture).
RECORD_RE = re.compile(r"GPU_STOP \(\d+\) tag \d+: cause (\d+) ")


def parse(text):
    """The log's facts: case verdicts, run-end figures, stop cause."""
    f = {"cases": {}, "pr": None, "oom": None, "hwr": None, "grow": None, "stop": None}
    for line in text.splitlines():
        m = CASE_RE.search(line)
        if m:
            f["cases"][m.group(1)] = f["cases"][m.group(2)] = m.group(3)
        m = GROW_RE.search(line)
        if m:
            f["grow"] = tuple(int(x) for x in m.groups()[:3])
            f["hwr"] = int(m.group(4))
        m = STATS_RE.search(line)
        if m:
            f["pr"], f["oom"] = int(m.group(1)), int(m.group(2))
        m = STOP_RE.search(line)
        if m and f["stop"] is None:  # the first cause stays, as in the driver
            f["stop"] = next((c for w, c in STOP_CAUSES if m.group(1) == w), 0)
        m = RECORD_RE.search(line)
        if m and f["stop"] is None:
            f["stop"] = int(m.group(1))
    return f


# --- the failure table's patterns; each cites the code that prints it. ---
# render: HWR during fence N (counter A -> B)   - render.rs, wait_for().
HWR_FENCE_RE = re.compile(r"render: HWR during fence \d+ ")
# '  HWR[e] dm=.. type=.. ... BIF read|write VA=0x...' - render.rs, wait_for(),
# the per-entry dump of the HWR info buffer that follows the line above.
HWR_BIF_RE = re.compile(r"HWR\[\d+\] .* BIF (?:read|write) VA=0x")
# 'FWCCB: CONTEXT_RESET ctxid=.. reason=N dm=..' - ccb.rs, the
# FWCCB_CMD_CONTEXT_RESET_NOTIFICATION arm of the FWCCB service.
RESET_RE = re.compile(r"FWCCB: CONTEXT_RESET .*\breason=(\d+)\b")
# 'FWCCB: cmd type=0x%08x (unhandled, skipping)' - ccb.rs, the last arm of the
# FWCCB service; 0x2abc0070 and 0x2abc006d are the types the earlier runs
# showed (F2, F3). The dedicated page-fault / GPU-restart lines of pr-diag-fwccb
# are not in ccb.rs yet: add their exact text here when they land.
UNHANDLED_F2 = "cmd type=0x2abc0070 (unhandled"
UNHANDLED_F3 = "cmd type=0x2abc006d (unhandled"
# 'render: stall: frag[n] off=.. FENCE_PR ufo 0x.. needs V, is W  (MET - ...)'
# or '(UNMET - ...)' - render.rs, the CCB dump (stall_dump) with the FENCE_PR kind.
FENCE_PR_RE = re.compile(r"stall: frag\[\d+\] off=\d+ FENCE_PR .*\((UNMET|MET) -")
# 'render: fence N timeout (ufo=U) waiting on the W' - render.rs, wait_for().
TIMEOUT_RE = re.compile(r"render: fence \d+ timeout ")
# 'render: stall: ' - render.rs, the stall dump's lines (all start so).
STALL_RE = re.compile(r"render: stall: ")
# 'render: stall: what[n] off=O bad cmd_size=S (type=..)' - render.rs, the CCB dump.
BAD_SIZE_RE = re.compile(r"stall: .*bad cmd_size=")
# 'gl_test: forcedpr: B of N pixels differ; first at ...' - gl_test.c,
# case_forcedpr(): the read-back differs.
PIXELS_RE = re.compile(r"gl_test: \w+: \d+ of \d+ pixels differ")


def classify(text, f, cases):
    """(class number, mode words) of the first matching row of the table."""
    lines = text.splitlines()
    done = all(f["cases"].get(c) == "PASS" for c in cases)
    differs = any(PIXELS_RE.search(ln) for ln in lines)
    if done and f["pr"] and f["pr"] >= 1 and f["hwr"] == 0 and not differs and f["stop"] is None:
        return 1, "success"
    if f["pr"] is not None and differs and f["hwr"] == 0:
        return 2, "F8 corrupt frame"
    hwr_at = [i for i, ln in enumerate(lines) if HWR_FENCE_RE.search(ln)]
    if hwr_at and any(HWR_BIF_RE.search(ln) for ln in lines[hwr_at[0]:]):
        return 3, "F1 page fault"
    reasons = [int(m.group(1)) for m in map(RESET_RE.search, lines) if m]
    if 16 in reasons:
        return 4, "F9 context reset, reason 16"
    if 13 in reasons or UNHANDLED_F2 in text:
        return 5, "F2 firmware page fault"
    if UNHANDLED_F3 in text:
        return 6, "F3 GPU restart"
    if f["stop"] == 4 and not f["pr"]:
        met = [m.group(1) for m in map(FENCE_PR_RE.search, lines) if m]
        if met:
            return 7, "F5 FENCE_PR unmet" if met[0] == "UNMET" else "F7 FENCE_PR met"
        return 7, "F7/F5 kick did not complete, partial renders 0"
    if f["stop"] == 2:
        return 8, "stop cause 2, not a partial-render failure (knob probably off)"
    if TIMEOUT_RE.search(text) and STALL_RE.search(text):
        return 9, "short-timeout stall"
    if BAD_SIZE_RE.search(text):
        return 10, "F10 bad cmd_size"
    if f["stop"] is None and not any(c in f["cases"] for c in cases):
        return 11, "hang ended by the watchdog"
    return 0, "unclassified"


def fmt(v):
    return "?" if v is None else str(v)


def verdict(text, cases, expect):
    """(lines, failure reasons) for a log."""
    f = parse(text)
    stop = "no" if f["stop"] is None else "cause%d" % f["stop"]
    lines, bad = [], []
    for c in cases:
        v = f["cases"].get(c, "MISSING")
        lines.append("pr-verdict: case %s %s partial_renders=%s oom=%s hwr=%s stopped=%s"
                     % (c, v, fmt(f["pr"]), fmt(f["oom"]), fmt(f["hwr"]), stop))
        if expect == "pr" and v != "PASS":
            bad.append("case %s is %s" % (c, v))
    if f["grow"] is not None:
        lines.append("pr-verdict: grow requests %d served %d unserved %d" % f["grow"])
    else:
        lines.append("pr-verdict: grow requests ? (no 'render freelists' line)")
    if expect == "pr":
        if f["pr"] is None:
            bad.append("no firmware UPDATE_STATS line")
        elif f["pr"] < 1:
            bad.append("partial renders %d (want >= 1)" % f["pr"])
        if f["hwr"] is None:
            bad.append("no GPU recoveries figure")
        elif f["hwr"] != 0:
            bad.append("GPU recoveries %d (want 0)" % f["hwr"])
        if f["stop"] is not None:
            bad.append("render stopped, cause %d" % f["stop"])
    elif f["stop"] is None:
        bad.append("no 'render: stopped' line (want cause 2 or 4)")
    elif f["stop"] not in (2, 4):
        bad.append("stop cause %d (want 2 or 4)" % f["stop"])
    if bad or expect == "pr":
        lines.append("pr-verdict: class %d %s" % classify(text, f, cases))
    return lines, bad


def run(text, cases, expect):
    lines, bad = verdict(text, cases, expect)
    for ln in lines:
        print(ln)
    print("pr-verdict: OK" if not bad else "pr-verdict: FAIL " + "; ".join(bad))
    return 1 if bad else 0


# Synthetic logs, in the formats above, behind a serial log prefix.
SERIAL = "[00:01:02.003,000] <inf> yakogl: "
STATS_LINE = (SERIAL + "render freelists: reserve refills refused 0 (0 KiB); firmware UPDATE_STATS: "
              "partial renders 1, out of memory 3, TA stores 0, 3D stores 1, CDM stores 0, TDM stores 0, other 0\n")
GROW_LINE = (SERIAL + "render freelists: 4 grow requests, 3 served, 1 unserved, 100 KiB taken on demand; "
             "GPU recoveries 0 (HWR counter 0)\n")
LOG_PASS = ("gl_test: case 175 refusehang: PASS\n"
            "gl_test: case 190 forcedpr: PASS\n" + GROW_LINE + STATS_LINE)
STOP_WORDS = "a kick that did not complete in the stop's time"
STOP_LINE = (SERIAL + "render: stopped: %s (list 1, 0 KiB); no kick is waited for or taken from now on\n")
LOG_STOP = (STOP_LINE % STOP_WORDS + "gl_test: case 175 refusehang: TIMEOUT\n" + GROW_LINE
            + STATS_LINE.replace("partial renders 1", "partial renders 0"))
LOG_HWR = LOG_PASS.replace("GPU recoveries 0 (HWR counter 0)", "GPU recoveries 2 (HWR counter 2)")


HWR_LINES = (SERIAL + "render: HWR during fence 7 (counter 0 -> 1)\n"
             "  HWR[0] dm=3 type=1 evt=0x00000000 rflags=0x00000000 hwrno=1 BIF read VA=0x0000123450 x tag_id=1\n")
RESET_LINE = SERIAL + "FWCCB: CONTEXT_RESET ctxid=3 reason=%d dm=3 jobref=1 flags=0x0 PC=0x0000000000 MMU=0x0000000000\n"
HANG_NOCASE = GROW_LINE.replace("GPU recoveries 0", "GPU recoveries 1")
STALL_UNMET = (SERIAL + "render: stall: frag[0] off=0 FENCE_PR ufo 0x00001000 needs 2, is 1"
               "  (UNMET - this is what it is waiting for)\n")
TIMEOUT_LINES = (SERIAL + "render: fence 9 timeout (ufo=8) waiting on the test\n"
                 + SERIAL + "render: stall: frag ccb has nothing unread\n")
FAIL_CASE = "gl_test: case 175 refusehang: FAIL\n"


def class_tests():
    """(name, log, wanted class line): one synthetic log per class."""
    fail_pr = FAIL_CASE + GROW_LINE + STATS_LINE
    no_pr = STATS_LINE.replace("partial renders 1", "partial renders 0")
    stop4 = STOP_LINE % STOP_WORDS
    return [
        ("class 1", LOG_PASS, "class 1 success"),
        ("class 2", FAIL_CASE + "gl_test: forcedpr: 5 of 65536 pixels differ; first at (0,0)\n"
         + GROW_LINE + STATS_LINE, "class 2 F8"),
        ("class 3", HWR_LINES + fail_pr, "class 3 F1"),
        ("class 4", RESET_LINE % 16 + fail_pr, "class 4 F9"),
        ("class 5 reason 13", RESET_LINE % 13 + fail_pr, "class 5 F2"),
        ("class 5 unhandled", SERIAL + "FWCCB: cmd type=0x2abc0070 (unhandled, skipping)\n" + fail_pr,
         "class 5 F2"),
        ("class 6", SERIAL + "FWCCB: cmd type=0x2abc006d (unhandled, skipping)\n" + fail_pr, "class 6 F3"),
        ("class 7 UNMET", stop4 + STALL_UNMET + FAIL_CASE + GROW_LINE + no_pr, "class 7 F5"),
        ("class 7 MET", stop4 + STALL_UNMET.replace("UNMET", "MET") + FAIL_CASE + GROW_LINE + no_pr,
         "class 7 F7"),
        ("class 7 no stall", stop4 + FAIL_CASE + GROW_LINE + no_pr, "class 7 F7/F5"),
        ("class 8", STOP_LINE % "the parameter manager out of memory with its freelist at the maximum"
         + FAIL_CASE + GROW_LINE + no_pr, "class 8 stop cause 2"),
        ("class 9", TIMEOUT_LINES + FAIL_CASE + GROW_LINE + STATS_LINE, "class 9 short-timeout"),
        ("class 10", SERIAL + "render: stall: frag[0] off=64 bad cmd_size=99999 (type=0x2abc0001)\n"
         + FAIL_CASE + GROW_LINE + STATS_LINE, "class 10 F10"),
        ("class 11", SERIAL + "render: submitted\n", "class 11 hang"),
    ]


def selftest():
    cases = [
        ("recoveries 0 is required for class 1", LOG_HWR, ["refusehang"], "pr", 1, "class 0"),
        ("pass log, expect pr", LOG_PASS, ["refusehang", "forcedpr"], "pr", 0,
         "pr-verdict: case refusehang PASS partial_renders=1 oom=3 hwr=0 stopped=no"),
        ("pass log, case by number", LOG_PASS, ["175"], "pr", 0, "pr-verdict: OK"),
        ("knob-off stop, expect stop", LOG_STOP, ["refusehang"], "stop", 0,
         "pr-verdict: case refusehang TIMEOUT partial_renders=0 oom=3 hwr=0 stopped=cause4"),
        ("knob-off stop, expect pr", LOG_STOP, ["refusehang"], "pr", 1, "pr-verdict: FAIL"),
        ("pass log, expect stop", LOG_PASS, ["refusehang"], "stop", 1, "no 'render: stopped'"),
        ("HWR > 0, expect pr", LOG_HWR, ["refusehang"], "pr", 1, "GPU recoveries 2 (want 0)"),
        ("partial renders 0, expect pr", LOG_PASS.replace("partial renders 1", "partial renders 0"),
         ["refusehang"], "pr", 1, "partial renders 0 (want >= 1)"),
        ("case absent", LOG_PASS, ["nosuch"], "pr", 1, "case nosuch is MISSING"),
        ("stop cause 3 is no expected stop",
         LOG_STOP.replace(STOP_WORDS, "a freelist grow past the host's table of grows"),
         ["refusehang"], "stop", 1, "stop cause 3"),
        ("stop cause 2 from the record only",
         "GPU_STOP (20) tag 2: cause 2 (out of memory with the freelist at its maximum) fence 30871 "
         "GPU recoveries 0 list 1 KiB 0\n" + GROW_LINE + STATS_LINE,
         ["refusehang"], "stop", 0, "stopped=cause2"),
        ("grow line", LOG_PASS, ["refusehang"], "pr", 0, "pr-verdict: grow requests 4 served 3 unserved 1"),
    ]
    cases += [("log for " + n, t, ["refusehang"], "pr", 0 if w.startswith("class 1 ") else 1, w)
              for n, t, w in class_tests()]
    failed = 0
    for name, text, cs, expect, want_rc, want in cases:
        lines, bad = verdict(text, cs, expect)
        out = "\n".join(lines + ["pr-verdict: " + ("OK" if not bad else "FAIL " + "; ".join(bad))])
        rc = 1 if bad else 0
        if rc == want_rc and want in out:
            print("  ok    " + name)
        else:
            failed += 1
            print("  FAIL  %s: rc %d (want %d), want %r in:\n%s" % (name, rc, want_rc, want, out))
    print("pr-verdict selftest: %s" % ("FAILED %d" % failed if failed else "all passed"))
    return 1 if failed else 0


def main(argv):
    expect = "pr"
    args = argv[1:]
    if args == ["--selftest"]:
        return selftest()
    if len(args) >= 2 and args[0] == "--expect":
        expect, args = args[1], args[2:]
    if expect not in ("pr", "stop") or len(args) < 2:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    try:
        with open(args[0], encoding="utf-8", errors="replace") as fh:
            text = fh.read()
    except OSError as e:
        print("pr-verdict: %s" % e, file=sys.stderr)
        return 2
    return run(text, args[1:], expect)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
