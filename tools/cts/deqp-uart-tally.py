#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tally dEQP results from Zephyr console logs.

The deqp sample (driver/samples/zephyr/deqp) prints one line per case,

    deqp: <Status> <case name>

with Status one of Pass, Fail, NotSupported, QualityWarning,
CompatibilityWarning, InternalError, ResourceError, Crash, Timeout, and at the
end "deqp: done: ..." and "deqp: summary: N cases begun, M with a result, K
without". This reads any number of captured logs (NULs and CRs of a raw serial
stream are dropped, and so is a leading "HH:MM:SS " time stamp, which the
Raspberry Pi 4's UART recorder puts on every line), keeps each case's last
status, and prints

  - per log: the sample's own done/summary lines, or MISSING when a log has
    none (the image never finished: its count is not a result), FATAL when
    the log holds "deqp: fatal:" (dEQP ended the run with die(): the cases
    after that point never ran, so the image is not a complete result even
    though it printed a summary), and
    INCOMPLETE when the log holds fewer case lines than its own summary
    says had a result (the serial capture lost lines: the image finished,
    but some cases have no name in the log, so they cannot be counted),
    followed by a "Lost in capture" line (see below);
  - per group: pass / fail / not supported / other, over the mustpass list
    when --mustpass is given (the case filter matches dEQP's whole tree,
    which is larger than the mustpass), else over every case seen;
  - every case that is neither Pass nor NotSupported, by name.

    tools/cts/deqp-uart-tally.py [--mustpass tools/cts/gles2-mustpass.txt]
                                 [--depth N] log...

--depth picks the group: 3 (default) groups dEQP-GLES2.functional.<group>,
4 adds the next component (functional.shaders.<subgroup>). dEQP-GLES2.info.*
counts as the group "info". Exit status 1 when a log has no summary line,
is INCOMPLETE or is FATAL. --self-test checks both verdicts on built-in logs.

The mustpass views (tools/board.sh tally and diff; the code was
the old Pi 4 tree runner's "tally", moved here unchanged), each with
--mustpass and instead of the output above:

    --table          "== mustpass: ..." and a per-group table over the
                     mustpass list (pass / fail / n.s. / QW / other / none,
                     "none" = no result line), then "== not Pass, by name"
    --baseline FILE  (after --table, or alone) the mustpass cases whose status
                     differs from FILE's ("<Status> <case>" lines); exit 1
                     when FILE lacks a mustpass case (no diff then)
    --statuses       "<Status> <case>" for every mustpass case (NoResult for
                     none): a baseline for --baseline

The scope of the mustpass views is the case filters the records ran: every
image prints "deqp: case filter '<patterns>'" (comma-separated, '*' matches
anything), and a mustpass case that no record's filter matches was not run,
so it is left out of the table, the statuses and the diff, and counted as
"outside these records' case filters" instead of "none" (a five-group
baseline used to report 16,258 NoResult for the groups it never selected,
2026-10-04). Records with no filter line keep the whole mustpass as scope,
and the table says so.

Lost is not NoResult. A case the image never began has no line and is
NoResult. A case the image counted as run, whose lines the capture lost, is
Lost: the evidence is the image's own counters ("deqp: summary: N cases begun,
M with a result", "deqp: done: P passed, F failed, ..." and, on the Pi 4's
v3d_deqp, "v3d_deqp: build B DONE...: N cases, P p F f ...") against the result
lines seen, and the "v3d_deqp: case #N" numbering, whose gaps (and whose lines
with a name no result line carries, a spliced line) place the loss. A "Lost in
capture" line names the lost ranges, the cases either side of each, the silent
seconds between them when the log is time-stamped, and the counters. The
mustpass views put a Lost case in its own "lost" column (shown only when there
is one) and print "Lost <case>" where they print NoResult; a case is placed by
the order of the mustpass list, which is the image's run order, only when the
window between the cases either side holds exactly the cases the numbering
lost, else the count is said and the cases stay "none". The pass/fail totals
are always the lines seen; the counters are quoted beside them. --lost prints
one number per log (the cases lost, 0 when none) for tools/board.sh's
done_lossy verdict.

Why INCOMPLETE exists: on 2026-09-25 the AM62P bridge dropped 14 of image
10's 80 result lines (texture.size, 758a3123); the summary still said 80
with a result, the tally printed its lines, exited 0, and 12 mustpass cases
were silently absent from the total.
"""
import argparse
import os
import re
import sys

# The Pi 4's recorder (uart_record.sh on the build host) starts every line with the
# time it arrived (console.txt: HH:MM:SS; console.ts: epoch seconds with
# milliseconds); without this every Pi record read "MISSING: no summary line".
STAMP = re.compile(r"^(?:\d{2}:\d{2}:\d{2}|\d{9,11}\.\d+) ")
STAMP_HMS = re.compile(r"^(\d{2}):(\d{2}):(\d{2}) ")
STAMP_EPOCH = re.compile(r"^(\d{9,11}\.\d+) ")
SUMMARY = re.compile(r"^deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")
DONE = re.compile(r"^deqp: done: (\d+) passed, (\d+) failed, (\d+) not supported, (\d+) warnings")
V3D_DONE = re.compile(r"^v3d_deqp: build \S+ DONE[^:]*: (\d+) cases, P (\d+) F (\d+) R (\d+) N (\d+) other (\d+)")
V3D_CASE = re.compile(r"^v3d_deqp: case #(\d+) \S .*?(\S+)$")
LINE = re.compile(r"^deqp: (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                  r"InternalError|ResourceError|Crash|Timeout) (\S+)")


HOLE_SECONDS = 5.0  # a silent stretch of the console worth naming (a case takes well under a second)


def group_of(case, depth):
    parts = case.split(".")
    if len(parts) > 1 and parts[1] == "info":
        return "info"
    return ".".join(parts[2:depth]) if len(parts) > depth - 1 else case


def stamp_of(line):
    """seconds of a time-stamped line (None without a stamp)"""
    m = STAMP_EPOCH.match(line)
    if m:
        return float(m.group(1))
    m = STAMP_HMS.match(line)
    return int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3)) if m else None


def scan(text):
    """Everything one log says: {"got": {case: last status}, "tail": the done / summary /
    fatal lines, "filters": the case filter patterns, "cases": {N: (name, stamp)} of the
    v3d_deqp "case #N" lines, "done" / "summary" / "v3d": the image's own counters (None when
    absent), "loss": the capture-loss evidence (None when nothing is lost)}."""
    raw = text.replace("\0", "").replace("\r", "").splitlines()
    r = {"got": {}, "tail": [], "filters": [], "cases": {}, "done": None, "summary": None, "v3d": None}
    stamped = []  # (seconds, last case number seen, line is a result line)
    last_no = None
    for l0 in raw:
        st = stamp_of(l0)
        l = STAMP.sub("", l0, count=1)
        if st is not None:
            stamped.append((st, last_no, LINE.match(l) is not None))
        if l.startswith(("deqp: done", "deqp: summary", "deqp: fatal")):
            r["tail"].append(l)
        m = LINE.match(l)
        if m:
            r["got"][m.group(2)] = m.group(1)
        m = FILTER_LINE.match(l)
        if m:
            r["filters"].extend(m.group(1).split(","))
        m = DONE.match(l)
        if m:
            r["done"] = tuple(int(x) for x in m.groups())
        m = SUMMARY.match(l)
        if m:
            r["summary"] = tuple(int(x) for x in m.groups())
        m = V3D_DONE.match(l)
        if m:
            r["v3d"] = (l, tuple(int(x) for x in m.groups()))
        m = V3D_CASE.match(l)
        if m:
            r["cases"][int(m.group(1))] = (m.group(2), st)
            last_no = int(m.group(1))
    # a result line whose name no case line carries: the capture damaged it (a splice of two lines)
    names = {n for n, _ in r["cases"].values()}
    r["damaged"] = sorted(c for c in r["got"] if c not in names) if names else []
    # holes in the time stamps inside the run of result lines (informational: a bridge that
    # stalls and flushes later shows as a silent stretch; the loss is not at the hole itself)
    res = [i for i, x in enumerate(stamped) if x[2]]
    r["holes"] = []
    if res:
        for i in range(res[0] + 1, res[-1] + 1):
            if stamped[i][0] - stamped[i - 1][0] >= HOLE_SECONDS:
                r["holes"].append((stamped[i][0] - stamped[i - 1][0], stamped[i - 1][1]))
    r["loss"] = capture_loss(r)
    return r


def ranges_of(nums):
    out = []
    for n in sorted(nums):
        if out and out[-1][1] == n - 1:
            out[-1][1] = n
        else:
            out.append([n, n])
    return out


def capture_loss(r):
    """The evidence that the capture lost cases the image ran, or None. The image's counters
    say how many cases had a result (deqp: summary; v3d_deqp's DONE line the same, as "cases");
    the lines seen are fewer. The v3d_deqp case numbers place the loss: a number with no line,
    or whose line names a case no result line carries (a line spliced with the next one)."""
    seen = len(r["got"])
    expected = [r["summary"][1]] if r["summary"] else []
    if r["v3d"]:
        expected.append(r["v3d"][1][0])
    if not expected or max(expected) <= seen:
        return None
    loss = {"seen": seen, "expected": max(expected), "lost": max(expected) - seen, "ranges": []}
    cases = r["cases"]
    if cases:
        top = max(max(cases), (r["v3d"][1][0] - 1) if r["v3d"] else 0)
        bad = [n for n in range(top + 1) if n not in cases or cases[n][0] not in r["got"]]
        for a, b in ranges_of(bad):
            prev = next((n for n in range(a - 1, -1, -1) if n in cases and cases[n][0] in r["got"]), None)
            nxt = next((n for n in range(b + 1, top + 1) if n in cases and cases[n][0] in r["got"]), None)
            loss["ranges"].append({"a": a, "b": b, "prev": prev, "next": nxt,
                                   "prev_name": cases[prev][0] if prev is not None else None,
                                   "next_name": cases[nxt][0] if nxt is not None else None})
    return loss


def counters_text(r):
    """the image's own counters, as the log printed them"""
    parts = []
    if r["summary"]:
        parts.append("deqp: summary %d cases begun, %d with a result" % r["summary"][:2])
    if r["done"]:
        parts.append("deqp: done %d passed, %d failed, %d not supported, %d warnings" % r["done"])
    if r["v3d"]:
        parts.append(r["v3d"][0].split(" ", 2)[2] if r["v3d"][0].startswith("v3d_deqp: build") else r["v3d"][0])
    return "; ".join(parts) or "no counters"


def split_by_status(r, lost):
    """the image's done counters minus the lines seen, per status class, when it adds up to lost"""
    if not r["done"]:
        return None
    seen = {"Pass": 0, "Fail": 0, "NotSupported": 0, "warn": 0, "other": 0}
    for s in r["got"].values():
        seen["warn" if s in ("QualityWarning", "CompatibilityWarning") else s if s in seen else "other"] += 1
    p, f, n, w = r["done"]
    want = {"Pass": p, "Fail": f, "NotSupported": n, "warn": w}
    d = {k: want[k] - seen[k] for k in want}
    if min(d.values()) < 0 or sum(d.values()) != lost:
        return None
    return ", ".join("%s %d" % (k, v) for k, v in d.items() if v)


def lost_lines(name, r):
    """the "Lost in capture" lines of one scanned log (none when nothing is lost)"""
    loss = r["loss"]
    if not loss:
        return []
    out = ["  Lost in capture: %d cases the image ran have no result line here (%s)" % (loss["lost"], name),
           "    the image's counters: %s; result lines seen: %d" % (counters_text(r), loss["seen"])]
    split = split_by_status(r, loss["lost"])
    if split:
        out.append("    by the image's done counters the lost cases were: %s" % split)
    for g in loss["ranges"]:
        where = "#%d-#%d" % (g["a"], g["b"]) if g["a"] != g["b"] else "#%d" % g["a"]
        near = ""
        if g["prev_name"] or g["next_name"]:
            near = " (after %s, before %s)" % (g["prev_name"] or "the start", g["next_name"] or "the end")
        out.append("    case numbers %s, %d without an intact result line%s" % (where, g["b"] - g["a"] + 1, near))
    if not loss["ranges"]:
        out.append("    (this log has no per-case numbering, so the lost cases cannot be placed)")
    if r["damaged"]:
        out.append("    %d result line(s) carry a name no case line has (the capture spliced lines): counted as seen "
                   "above, e.g. %s" % (len(r["damaged"]), r["damaged"][0]))
    for sec, after in r["holes"][:3]:
        out.append("    (time stamps: %.1f s without a console line%s; a stalled bridge flushes late or drops its "
                   "backlog)" % (sec, " after case #%d" % after if after is not None else ""))
    return out


def read_log(text):
    """(verdict, {case: status}, tail) of one log's text. verdict is None for a
    complete log, else the MISSING / INCOMPLETE line to print."""
    r = scan(text)
    got, tail = r["got"], r["tail"]
    if any(l.startswith("deqp: fatal") for l in tail):
        return "FATAL: dEQP ended the run (die()); the cases after it did not run", got, tail
    summary = [SUMMARY.match(l) for l in tail if SUMMARY.match(l)]
    if not summary:
        return "MISSING: no summary line", got, tail
    with_result = int(summary[-1].group(2))
    if len(got) != with_result:
        return ("INCOMPLETE: %d cases have a result line here, the summary says %d had a result"
                % (len(got), with_result)), got, tail
    return None, got, tail


def self_test():
    head = "deqp: Pass dEQP-GLES2.a.b\ndeqp: Fail dEQP-GLES2.a.c\n"
    done = "deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1\n"
    checks = [
        ("complete log", head + done + "deqp: summary: 2 cases begun, 2 with a result, 0 without\n", None),
        ("a result line lost", head + done + "deqp: summary: 3 cases begun, 3 with a result, 0 without\n",
         "INCOMPLETE"),
        ("no summary", head, "MISSING"),
        ("dEQP died after two cases, summary printed",
         head + "FATAL ERROR: Runtime check failed: 'isOk()' at x.cpp:97\n"
         + "deqp: fatal: dEQP ended the run (die()); later cases did not run\n"
         + "deqp: summary: 2 cases begun, 2 with a result, 0 without\n", "FATAL"),
        ("NULs and CRs of a raw stream", head.replace("\n", "\r\n") + "\0" + done
         + "deqp: summary: 2 cases begun, 2 with a result, 0 without\r\n", None),
        ("a time stamp on every line (the Pi 4's recorder)",
         "".join("20:56:%02d %s\r\n" % (i, l) for i, l in enumerate((head + done).splitlines()))
         + "20:56:59 deqp: summary: 2 cases begun, 2 with a result, 0 without\r\n", None),
        ("time-stamped, a result line lost",
         "".join("20:56:%02d %s\n" % (i, l) for i, l in enumerate((head + done).splitlines()))
         + "20:56:59 deqp: summary: 3 cases begun, 3 with a result, 0 without\n", "INCOMPLETE"),
    ]
    bad = 0
    for desc, text, want in checks:
        verdict = read_log(text)[0]
        ok = verdict is None if want is None else (verdict or "").startswith(want)
        print("  %s  %s: %s" % ("ok  " if ok else "FAIL", desc, verdict or "complete"))
        bad += not ok
    return (1 if bad else 0) | scope_self_test() | capture_self_test()


def scope_self_test():
    """The mustpass views' scope: a record that ran two groups leaves the
    others out (not "none"), a whole-tree baseline diffs over those two only,
    and a record with no filter line keeps the whole list."""
    import contextlib
    import io
    import tempfile
    mp = ["dEQP-GLES2.info.vendor", "dEQP-GLES2.functional.a.x", "dEQP-GLES2.functional.a.y",
          "dEQP-GLES2.functional.b.z", "dEQP-GLES2.functional.c.w"]
    log = ("deqp: case filter 'dEQP-GLES2.info.*,dEQP-GLES2.functional.a.*'\n"
           "deqp: Pass dEQP-GLES2.info.vendor\ndeqp: Pass dEQP-GLES2.functional.a.x\n"
           "deqp: NotSupported dEQP-GLES2.functional.a.y\n")
    whole_base = "".join("Pass %s\n" % c for c in mp)
    d = tempfile.mkdtemp()
    paths = {}
    for name, text in (("mp", "\n".join(mp) + "\n"), ("log", log), ("base", whole_base),
                       ("nofilter", log.split("\n", 1)[1])):
        paths[name] = os.path.join(d, name)
        with open(paths[name], "w") as f:
            f.write(text)

    def run(*args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = mustpass_views(paths["mp"], [paths[args[0]]], *args[1:])
        return rc, out.getvalue()
    bad = 0
    checks = []
    rc, out = run("log", True, None, False)
    checks.append(("table: 3 in scope, 2 outside, no NoResult for b and c",
                   "3 cases in these records' case filters (2 outside them" in out
                   and "NoResult" not in out and "TOTAL" in out and out.rstrip().endswith("dEQP-GLES2.functional.a.y")))
    rc, out = run("log", False, paths["base"], False)
    checks.append(("a whole-tree baseline diffs over the scope only: 1 of 3 differ (a.y), 0 lines outside",
                   rc == 0 and "1 of 3 mustpass cases differ" in out and "functional.b." not in out
                   and "functional.c." not in out))
    rc, out = run("log", False, None, True)
    checks.append(("statuses list the scope only", out.count("\n") == 3 and "functional.b.z" not in out))
    rc, out = run("nofilter", True, None, False)
    checks.append(("no filter line: the whole mustpass, b and c NoResult, and the table says so",
                   "no case filter line" in out and "NoResult dEQP-GLES2.functional.b.z" in out))
    for desc, ok in checks:
        print("  %s  scope: %s" % ("ok  " if ok else "FAIL", desc))
        bad += not ok
    return 1 if bad else 0


def capture_self_test():
    """Lost (the image ran the case, the capture lost its lines) against NoResult (the image never
    began it): synthetic v3d_deqp logs, the Pi 4 record of 2026-10-10 (tools/testdata), time stamps."""
    import contextlib
    import io
    import tempfile
    names = ["dEQP-GLES2.functional.a.c%d" % i for i in range(7)]  # c6 is in the filter but never ran

    def v3d_log(keep, stamps=None, total=5):
        out = ["deqp: case filter 'dEQP-GLES2.functional.a.*'"]
        for i in range(total):
            if i in keep:
                out.append("deqp: Pass %s" % names[i])
                out.append("v3d_deqp: case #%d P 40ms k2/8 c1/2 r0 %s" % (i, names[i]))
        out += ["deqp: done: %d passed, 0 failed, 0 not supported, 0 warnings, complete=1" % total,
                "deqp: summary: %d cases begun, %d with a result, 0 without" % (total, total),
                "v3d_deqp: build 0123abcd DONE, all passed: %d cases, P %d F 0 R 0 N 0 other 0" % (total, total)]
        if stamps:
            out = ["%s %s" % (stamps(i), l) for i, l in enumerate(out)]
        return "\n".join(out) + "\n"
    d = tempfile.mkdtemp()

    def write(name, text):
        path = os.path.join(d, name)
        with open(path, "w") as f:
            f.write(text)
        return path
    mp = write("mp", "\n".join(names) + "\n")
    bad = 0
    checks = []
    whole = scan(v3d_log({0, 1, 2, 3, 4}))
    checks.append(("a complete v3d log loses nothing", whole["loss"] is None and not read_log(v3d_log({0, 1, 2, 3, 4}))[0]))
    lossy_text = v3d_log({0, 1, 4})
    lossy = scan(lossy_text)
    lo = lossy["loss"] or {}
    checks.append(("two result lines lost: counters say 2, numbering places them at #2-#3",
                   lo.get("lost") == 2 and [(g["a"], g["b"]) for g in lo.get("ranges", [])] == [(2, 3)]
                   and (read_log(lossy_text)[0] or "").startswith("INCOMPLETE")))
    text = "\n".join(lost_lines("x", lossy))
    checks.append(("the Lost line names the range, the cases either side, and the image's counters",
                   "Lost in capture: 2 cases" in text and "case numbers #2-#3" in text and names[1] in text
                   and names[4] in text and "5 cases begun, 5 with a result" in text and "by the image's done counters"
                   " the lost cases were: Pass 2" in text))
    lp = write("lossy", lossy_text)
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        rc = mustpass_views(mp, [lp], True, None, False)
    t = out.getvalue()
    checks.append(("table: 2 Lost in their own column, c6 (never begun) none, not Lost",
                   "lost   none" in t and "Lost %s" % names[2] in t and "Lost %s" % names[3] in t
                   and "NoResult %s" % names[6] in t and "NoResult %s" % names[2] not in t
                   and "image's own counters" in t and "2 LOST in capture" in t))
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        mustpass_views(mp, [lp], False, None, True)
    checks.append(("--statuses: Lost for the two lost cases, NoResult for the two that never began",
                   out.getvalue().count("Lost ") == 2 and out.getvalue().count("NoResult ") == 2))
    cp = write("complete", v3d_log({0, 1, 2, 3, 4}))
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        mustpass_views(mp, [cp], True, None, False)
    checks.append(("a complete log's table has no lost column and no Lost section",
                   "lost" not in out.getvalue() and "Lost in capture" not in out.getvalue()))
    # no per-case numbering (the AM62P image): the count is known, the cases cannot be placed
    plain = "".join("deqp: Pass %s\n" % names[i] for i in (0, 1, 4)) \
        + "deqp: summary: 5 cases begun, 5 with a result, 0 without\n"
    pl = scan(plain)
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        mustpass_views(mp, [write("plain", "deqp: case filter 'dEQP-GLES2.functional.a.*'\n" + plain)], True, None, False)
    checks.append(("no case numbering: 2 lost by the counters, unplaced, and they stay in none",
                   (pl["loss"] or {}).get("lost") == 2 and "2 more lost cases could not be placed" in out.getvalue()
                   and "Lost %s" % names[2] not in out.getvalue()))
    # a spliced result line (a name no case line has) does not make a case seen
    splice = lossy_text.replace("deqp: Pass %s" % names[4], "deqp: Pass %sXX" % names[4])
    checks.append(("a damaged result line is named and the case stays lost",
                   scan(splice)["damaged"] == [names[4] + "XX"]))
    # a result whose case line was spliced (Pi 4, 2026-10-10 full F1: #379) is a result, not a lost case
    sp = lossy_text.replace("deqp: Pass %s\n" % names[1], "deqp: Pass %s\ndeqp: Pass %s\n" % (names[1], names[2])) \
        .replace("v3d_deqp: case #1 P 40ms k2/8 c1/2 r0 %s\n" % names[1],
                 "v3d_deqp: case #1 P 40ms k2/8 c1/2 r0 %s\nv3d_deqp: case #2 P 40ms k2/8 c1/2 r0 %szz\n" % (names[1], names[2]))
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        mustpass_views(mp, [write("spliced", sp)], False, None, True)
    checks.append(("a result whose case line was spliced stays a result; only the case with no line is Lost",
                   scan(sp)["damaged"] == [names[2]] and "Lost %s\n" % names[3] in out.getvalue()
                   and out.getvalue().count("Lost ") == 1 and "Pass %s\n" % names[2] in out.getvalue()))
    # time stamps: HH:MM:SS and epoch both parse, and a silent stretch is named (informational)
    for label, st in (("HH:MM:SS", lambda i: "10:00:%02d" % (0 if i < 5 else 30)),
                      ("epoch", lambda i: "1791635%03d.%03d" % (0 if i < 5 else 30, i))):
        r = scan(v3d_log({0, 1, 4}, st))
        checks.append(("%s stamps: a 30 s hole is named, the stamps do not break the counters" % label,
                       r["holes"] and r["holes"][0][0] >= 29 and (r["loss"] or {}).get("lost") == 2))
    # the record: Pi 4 F1-rerun2 (a27d69c0), 1923 cases run, 1663 lines through the bridge
    fx = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "testdata",
                      "pi4-tryboot-F1rerun2-a27d69c0.console.txt")
    mustpass = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gles2-mustpass.txt")
    if os.path.exists(fx) and os.path.exists(mustpass):
        r = scan(open(fx, errors="replace").read())
        lo = r["loss"] or {}
        checks.append(("fixture pi4 F1-rerun2: 260 lost by the counters (1923 ran, 1663 seen), #870-#1130 placed",
                       lo.get("lost") == 260 and lo.get("seen") == 1663 and [(g["a"], g["b"]) for g in lo["ranges"]]
                       == [(870, 1130)] and len(r["damaged"]) == 1))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            mustpass_views(mustpass, [fx], True, None, False)
        t = out.getvalue()
        checks.append(("fixture: 1662 Pass, 261 in the lost column (260 counted + the spliced line), none 0, no NoResult",
                       "1662      0      0      0      0    261      0" in t and "NoResult dEQP" not in t))
    for desc, ok in checks:
        print("  %s  capture: %s" % ("ok  " if ok else "FAIL", desc))
        bad += not ok
    return 1 if bad else 0


# The mustpass views, moved unchanged from the old Pi 4 tree runner's "tally"
# (its inline Python), so that tools/board.sh prints what that script printed.
FILTER_LINE = re.compile(r"^(?:\d{2}:\d{2}:\d{2} |\d{9,11}\.\d+ )?deqp: case filter '([^']*)'")


def filter_regex(patterns):
    """One regex for a list of dEQP case patterns ('*' matches anything, also
    dots; everything else literally)."""
    alts = ["".join(".*" if ch == "*" else re.escape(ch) for ch in p) for p in patterns if p]
    return re.compile("^(?:%s)$" % "|".join(alts)) if alts else None


def scope_of(mustpass, filters):
    """(the mustpass cases some filter matches, how many it left out). No
    filter: the whole list."""
    rx = filter_regex(filters)
    if rx is None:
        return mustpass, 0
    inside = [c for c in mustpass if rx.match(c)]
    return inside, len(mustpass) - len(inside)


def place_lost(scans, mustpass, status):
    """({the mustpass cases that are Lost}, how many lost cases could not be placed). The image
    runs the mustpass list in its own order, so the cases between the two either side of a lost
    run of case numbers are the lost ones, when there are exactly as many of them as numbers
    (a window of another size is not trusted: the count stays unplaced)."""
    idx = {c: i for i, c in enumerate(mustpass)}
    lost, unplaced = set(), 0
    for name, r in scans:
        loss = r["loss"]
        if not loss:
            continue
        left = loss["lost"]
        damaged = set(r["damaged"])
        for g in loss["ranges"]:
            prev = g["prev_name"] if g["prev_name"] in idx else None
            nxt = g["next_name"] if g["next_name"] in idx else None
            if (g["prev_name"] and prev is None) or (g["next_name"] and nxt is None):
                continue  # a neighbour outside the mustpass: no window
            lo = idx[prev] + 1 if prev else 0
            hi = idx[nxt] if nxt else len(mustpass)
            whole = mustpass[lo:hi]
            window = [c for c in whole if c not in r["got"]]
            # a result whose case line was spliced is a result (the number is "bad", the case is not lost)
            kept = sum(1 for c in whole if c in damaged)
            if len(window) == g["b"] - g["a"] + 1 - kept:
                lost.update(c for c in window if c not in status)
                left -= len(window)
        unplaced += max(0, left)
    return lost, unplaced


def mustpass_views(mustpass_path, recs, table, base_path, statuses):
    status = {}
    filters = []
    scans = []
    for p in recs:
        r = scan(open(p, "rb").read().decode("utf-8", "replace"))
        status.update(r["got"])
        filters.extend(r["filters"])
        scans.append((os.path.basename(os.path.dirname(os.path.abspath(p))) or p, r))
    whole = [l.strip() for l in open(mustpass_path) if l.strip()]
    mustpass, outside = scope_of(whole, filters)
    lost, unplaced = place_lost(scans, mustpass, status)

    def word(c):
        return status.get(c) or ("Lost" if c in lost else "NoResult")
    if statuses:
        for c in mustpass:
            print("%s %s" % (word(c), c))
        return 0
    if table:
        cols = ["Pass", "Fail", "NotSupported", "QualityWarning", "other"] + (["lost"] if lost else []) + ["none"]

        def kind(c):
            s = status.get(c)
            return "lost" if s is None and c in lost else "none" if s is None else s if s in cols else "other"

        def group(c):
            p = c.split(".")
            return "info" if p[1] == "info" else p[2] if len(p) > 3 else c
        groups = {}
        for c in mustpass:
            g = groups.setdefault(group(c), dict.fromkeys(cols, 0))
            g[kind(c)] += 1
        if filters:
            print("== mustpass: %d cases in these records' case filters (%d outside them, not run, left out); "
                  "%d with a result (%d case lines seen)"
                  % (len(mustpass), outside, sum(c in status for c in mustpass), len(status)))
        else:
            print("== mustpass: %d cases, %d with a result in these records (%d case lines seen; no case filter "
                  "line, so the scope is the whole mustpass)"
                  % (len(mustpass), sum(c in status for c in mustpass), len(status)))
        heads = {"Pass": "pass", "Fail": "fail", "NotSupported": "n.s.", "QualityWarning": "QW", "other": "other",
                 "lost": "lost", "none": "none"}
        fmt = "%-28s %7s" + " %6s" * (len(cols) - 1)
        print(fmt % (("group",) + tuple(heads[k] for k in cols)))
        fmt = fmt.replace("s", "d").replace("%-28d", "%-28s")
        tot = dict.fromkeys(cols, 0)
        for g in sorted(groups):
            v = groups[g]
            for k in cols:
                tot[k] += v[k]
            print(fmt % ((g,) + tuple(v[k] for k in cols)))
        print(fmt % (("TOTAL",) + tuple(tot[k] for k in cols)))
        counted = [(n, r) for n, r in scans if r["summary"] or r["v3d"]]
        if counted:
            print("== the totals above are the result lines seen; the image's own counters, beside them:")
            for n, r in counted:
                print("  %s: %s; %d result lines seen%s" % (n, counters_text(r), len(r["got"]),
                      "" if not r["loss"] else ", %d LOST in capture" % r["loss"]["lost"]))
        if lost or unplaced or any(r["loss"] for _, r in scans):
            print("== Lost in capture (the image ran these; the lines never reached the log; not NoResult)")
            for n, r in scans:
                for l in lost_lines(n, r):
                    print(l)
            counted = sum(r["loss"]["lost"] for _, r in scans if r["loss"])
            print("  placed by the mustpass order: %d cases are in the lost column (the counters say %d%s)%s"
                  % (len(lost), counted, "" if len(lost) == counted else ": a result line the capture spliced is "
                     "counted by them as seen",
                     "; %d more lost cases could not be placed and stay in none" % unplaced if unplaced else ""))
        print("== not Pass, by name")
        for c in mustpass:
            if status.get(c) != "Pass":
                print("%s %s" % (word(c), c))
    if base_path:
        base = {}
        for l in open(base_path):
            f = l.split()
            if len(f) == 2 and not l.startswith("#"):
                base[f[1]] = f[0]
        missing = [c for c in mustpass if c not in base]
        if missing:
            print("== baseline lacks %d mustpass cases (first: %s): no per-case diff" % (len(missing), missing[0]))
            return 1
        diff = [(c, base[c], word(c)) for c in mustpass if base[c] != word(c)]
        print("== per-case diff against %s: %d of %d mustpass cases differ" % (base_path, len(diff), len(mustpass)))
        for c, a, b in diff:
            print("  %s: %s -> %s" % (c, a, b))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("logs", nargs="*")
    ap.add_argument("--mustpass")
    ap.add_argument("--depth", type=int, default=3)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--lost", action="store_true", help="print the cases lost in capture, summed over the logs")
    ap.add_argument("--table", action="store_true")
    ap.add_argument("--baseline")
    ap.add_argument("--statuses", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.logs:
        ap.error("no logs given")
    if a.table or a.baseline or a.statuses:
        if not a.mustpass:
            ap.error("--table, --baseline and --statuses need --mustpass")
        return mustpass_views(a.mustpass, a.logs, a.table, a.baseline, a.statuses)

    status = {}
    missing = False
    lost_total = 0
    scans = []
    for path in a.logs:
        with open(path, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        verdict, got, tail = read_log(text)
        r = scan(text)
        scans.append((path, r))
        lost_total += r["loss"]["lost"] if r["loss"] else 0
        if a.lost:
            continue
        print("%s: %s%s" % (path, " | ".join(tail), (" | " if tail else "") + verdict if verdict else ""))
        for l in lost_lines(path, r):
            print(l)
        missing |= verdict is not None
        status.update(got)
    if a.lost:
        print(lost_total)
        return 0

    cases = status
    if a.mustpass:
        with open(a.mustpass) as f:
            wanted = {l.strip() for l in f if l.strip()}
        cases = {c: s for c, s in status.items() if c in wanted}
        print("mustpass cases with a result: %d (of %d seen)" % (len(cases), len(status)))

    groups = {}
    for c, s in cases.items():
        g = groups.setdefault(group_of(c, a.depth), {"Pass": 0, "Fail": 0, "NotSupported": 0, "other": 0})
        g[s if s in g else "other"] += 1
    tot = {"Pass": 0, "Fail": 0, "NotSupported": 0, "other": 0}
    print("%-40s %7s %6s %6s %6s" % ("group", "pass", "fail", "n/s", "other"))
    for g in sorted(groups):
        v = groups[g]
        for k in tot:
            tot[k] += v[k]
        print("%-40s %7d %6d %6d %6d" % (g, v["Pass"], v["Fail"], v["NotSupported"], v["other"]))
    print("%-40s %7d %6d %6d %6d" % ("TOTAL", tot["Pass"], tot["Fail"], tot["NotSupported"], tot["other"]))
    print("(the pass / fail counts above are the result lines seen; the image's own counters:)")
    for n, r in scans:
        print("  %s: %s; %d result lines seen%s" % (os.path.basename(os.path.dirname(os.path.abspath(n))) or n,
              counters_text(r), len(r["got"]), "" if not r["loss"] else ", %d LOST in capture" % r["loss"]["lost"]))
    for c in sorted(cases):
        if cases[c] not in ("Pass", "NotSupported"):
            print("%s %s" % (cases[c], c))
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
