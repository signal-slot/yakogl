#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare the gl_test pass sets and glmark2 results of two or more AM62P runs.

    tools/cts/pr-regress-compare.py <log or record dir> <log or record dir> [...]

Acceptance criterion c3 of the PowerVR partial-render plan (issue #3): the
regression set must be unchanged with CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER
on and off. A record directory is read through its console.txt, else through
its *.txt / *.log files. Prints, per gl_test case, the verdict in each run and
the cases that differ; per run the GPU recovery counts and the 'render:
stopped' lines; per glmark2 scene the score and the final score, with the
differences. The last line is 'pr-regress: SAME' (exit 0) or
'pr-regress: DIFFERENT (<n> cases, <m> resets)' (exit 1; a glmark2 difference
or a differing 'render: stopped' count counts in neither number but still
makes the result DIFFERENT). Exit 2: bad usage or an unreadable log.

The line formats are common-tally.py's (CASE_LINE_RE, GM_*_RE, imported when
importable), which cite gl_test.c "gl_test: case N <name>: PASS/FAIL/TIMEOUT"
and glmark2's "[scene] opts: FPS: n". "gl_test: case N ...: SKIP" is a
verdict too. Resets: the OSAL's "GPU reset (recovery" lines (board.sh), and
render.rs's run-end "GPU recoveries N (HWR counter C)"; the larger of the two
is a run's count. "render: stopped" lines are render.rs's refusals.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def _tally():
    try:
        spec = importlib.util.spec_from_file_location(
            "common_tally", os.path.join(HERE, "common-tally.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod
    except Exception:
        return None


_T = _tally()
# common-tally.py CASE_LINE_RE / GM_SCORE_RE / GM_RESULT_RE / GM_HEADER_RE
CASE_LINE_RE = getattr(_T, "CASE_LINE_RE", None) or re.compile(
    r"(?<![A-Za-z0-9_])gl_test: case (\d+) [^:\n]*: (PASS|FAIL|TIMEOUT)\b")
GM_SCORE_RE = getattr(_T, "GM_SCORE_RE", None) or re.compile(r"glmark2: score (\d+)")
GM_RESULT_RE = getattr(_T, "GM_RESULT_RE", None) or re.compile(
    r"(?:^|\s)(\[[\w-]+\] .*?): (?:FPS: |Set up failed|Unsupported)")
GM_HEADER_RE = getattr(_T, "GM_HEADER_RE", None) or re.compile(
    r"^(?:\d\d:\d\d:\d\d )?(\[[\w-]+\] .*)$")
SKIP_RE = re.compile(r"(?<![A-Za-z0-9_])gl_test: case (\d+) [^:\n]*: (SKIP)\b")
FPS_RE = re.compile(r"(?:^|\s)FPS: (\d+)")
UNSUP_RE = re.compile(r"(?:^|\s)(Set up failed|Unsupported)\s*$")
RESET_RE = re.compile(r"GPU reset \(recovery")
RECOV_RE = re.compile(r"GPU recoveries (\d+) \(HWR counter (\d+)\)")
STOP_RE = re.compile(r"render: stopped\b.*")
TS_RE = re.compile(r"^\d\d:\d\d:\d\d ")


def read_run(path):
    if os.path.isdir(path):
        c = os.path.join(path, "console.txt")
        files = [c] if os.path.isfile(c) else sorted(
            os.path.join(path, f) for f in os.listdir(path)
            if f.endswith((".txt", ".log")))
    else:
        files = [path]
    if not files:
        raise OSError("no log in " + path)
    text = ""
    for f in files:
        with open(f, "r", errors="replace") as fh:
            text += fh.read() + "\n"
    return text


def parse(text):
    r = {"cases": {}, "scenes": {}, "score": None, "resets": 0,
         "recov": 0, "hwr": 0, "stops": []}
    last = None
    resets = 0
    for line in text.splitlines():
        line = line.rstrip("\r")
        m = CASE_LINE_RE.search(line) or SKIP_RE.search(line)
        if m:
            r["cases"][int(m.group(1))] = m.group(2)
        m = GM_SCORE_RE.search(line)
        if m:
            r["score"] = int(m.group(1))
        m = GM_RESULT_RE.search(line)
        if m:
            last = m.group(1)
        else:
            h = GM_HEADER_RE.match(line)
            if h and "glmark2-" not in line:
                last = h.group(1).rstrip(":").strip()
        # a scene's result: "[s] opts: FPS: n", or a bare " FPS: n" after its header
        m = FPS_RE.search(line)
        if m and last and "glmark2-frame:" not in line:
            r["scenes"][last] = int(m.group(1))
            last = None
        elif UNSUP_RE.search(line) and last:
            r["scenes"][last] = UNSUP_RE.search(line).group(1).lower()
            last = None
        if RESET_RE.search(line):
            resets += 1
        m = RECOV_RE.search(line)
        if m:
            r["recov"] = max(r["recov"], int(m.group(1)))
            r["hwr"] = max(r["hwr"], int(m.group(2)))
        m = STOP_RE.search(line)
        if m:
            r["stops"].append(TS_RE.sub("", m.group(0)))
    r["resets"] = max(resets, r["recov"])
    return r


def main(argv):
    if len(argv) < 3:
        print("usage: pr-regress-compare.py <run> <run> [<run> ...]", file=sys.stderr)
        return 2
    names = [a.rstrip("/") for a in argv[1:]]
    labels = ["run%d" % (i + 1) for i in range(len(names))]
    runs = []
    for n in names:
        try:
            runs.append(parse(read_run(n)))
        except OSError as e:
            print("pr-regress-compare: %s" % e, file=sys.stderr)
            return 2
    for l, n in zip(labels, names):
        print("%s = %s" % (l, n))

    print("\n== gl_test cases")
    ids = sorted({c for r in runs for c in r["cases"]})
    ndiff = 0
    for c in ids:
        v = [r["cases"].get(c, "missing") for r in runs]
        d = len(set(v)) > 1
        ndiff += d
        print("case %-4d %s%s" % (c, " ".join("%s=%s" % (l, x) for l, x in zip(labels, v)),
                                   "  DIFFERS" if d else ""))
    if not ids:
        print("(no gl_test case lines)")
    print("cases differing: %d of %d" % (ndiff, len(ids)))

    print("\n== resets")
    for l, r in zip(labels, runs):
        print("%s: GPU recoveries %d (HWR counter %d), reset lines %d, render: stopped %d"
              % (l, r["recov"], r["hwr"], r["resets"], len(r["stops"])))
        for s in r["stops"]:
            print("  %s: %s" % (l, s))
    base = runs[0]["resets"]
    nreset = sum(max(0, r["resets"] - base) for r in runs[1:])
    stops_differ = len({len(r["stops"]) for r in runs}) > 1

    print("\n== glmark2")
    gdiff = 0
    for s in sorted({s for r in runs for s in r["scenes"]}):
        v = [r["scenes"].get(s, "missing") for r in runs]
        d = len(set(v)) > 1
        gdiff += d
        print("%s: %s%s" % (s, " ".join("%s=%s" % (l, x) for l, x in zip(labels, v)),
                            "  DIFFERS" if d else ""))
    sc = [r["score"] for r in runs]
    sd = len(set(sc)) > 1
    gdiff += sd
    print("glmark2 score: %s%s" % (
        " ".join("%s=%s" % (l, "missing" if x is None else x) for l, x in zip(labels, sc)),
        "  DIFFERS" if sd else ""))
    print("glmark2 differences: %d" % gdiff)

    if not (ndiff or nreset or gdiff or stops_differ):
        print("pr-regress: SAME")
        return 0
    print("pr-regress: DIFFERENT (%d cases, %d resets)" % (ndiff, nreset))
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
