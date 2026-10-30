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


def selftest():
    cases = [
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
