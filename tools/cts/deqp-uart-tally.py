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
    but some cases have no name in the log, so they cannot be counted);
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

Why INCOMPLETE exists: on 2026-09-25 the AM62P bridge dropped 14 of image
10's 80 result lines (texture.size, 758a3123); the summary still said 80
with a result, the tally printed its lines, exited 0, and 12 mustpass cases
were silently absent from the total.
"""
import argparse
import re
import sys

# The Pi 4's recorder (uart_record.sh on the build host) starts every line with the
# time it arrived; without this every Pi record read "MISSING: no summary line".
STAMP = re.compile(r"^\d{2}:\d{2}:\d{2} ")
SUMMARY = re.compile(r"^deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")
LINE = re.compile(r"^deqp: (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                  r"InternalError|ResourceError|Crash|Timeout) (\S+)")


def group_of(case, depth):
    parts = case.split(".")
    if len(parts) > 1 and parts[1] == "info":
        return "info"
    return ".".join(parts[2:depth]) if len(parts) > depth - 1 else case


def read_log(text):
    """(verdict, {case: status}, tail) of one log's text. verdict is None for a
    complete log, else the MISSING / INCOMPLETE line to print."""
    lines = [STAMP.sub("", l, count=1) for l in text.replace("\0", "").replace("\r", "").splitlines()]
    tail = [l for l in lines if l.startswith(("deqp: done", "deqp: summary", "deqp: fatal"))]
    got = {}
    for l in lines:
        m = LINE.match(l)
        if m:
            got[m.group(2)] = m.group(1)
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
    return (1 if bad else 0) | scope_self_test()


def scope_self_test():
    """The mustpass views' scope: a record that ran two groups leaves the
    others out (not "none"), a whole-tree baseline diffs over those two only,
    and a record with no filter line keeps the whole list."""
    import contextlib
    import io
    import os
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


# The mustpass views, moved unchanged from the old Pi 4 tree runner's "tally"
# (its inline Python), so that tools/board.sh prints what that script printed.
MP_LINE = re.compile(r"^(?:\d{2}:\d{2}:\d{2} )?deqp: (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                     r"InternalError|ResourceError|Crash|Timeout) (\S+)")
FILTER_LINE = re.compile(r"^(?:\d{2}:\d{2}:\d{2} )?deqp: case filter '([^']*)'")


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


def mustpass_views(mustpass_path, recs, table, base_path, statuses):
    status = {}
    filters = []
    for p in recs:
        for l in open(p, "rb").read().decode("utf-8", "replace").replace("\0", "").replace("\r", "").splitlines():
            m = MP_LINE.match(l)
            if m:
                status[m.group(2)] = m.group(1)
            f = FILTER_LINE.match(l)
            if f:
                filters.extend(f.group(1).split(","))
    whole = [l.strip() for l in open(mustpass_path) if l.strip()]
    mustpass, outside = scope_of(whole, filters)
    if statuses:
        for c in mustpass:
            print("%s %s" % (status.get(c, "NoResult"), c))
        return 0
    if table:
        cols = ["Pass", "Fail", "NotSupported", "QualityWarning", "other", "none"]

        def kind(s):
            return "none" if s is None else s if s in cols else "other"

        def group(c):
            p = c.split(".")
            return "info" if p[1] == "info" else p[2] if len(p) > 3 else c
        groups = {}
        for c in mustpass:
            g = groups.setdefault(group(c), dict.fromkeys(cols, 0))
            g[kind(status.get(c))] += 1
        if filters:
            print("== mustpass: %d cases in these records' case filters (%d outside them, not run, left out); "
                  "%d with a result (%d case lines seen)"
                  % (len(mustpass), outside, sum(c in status for c in mustpass), len(status)))
        else:
            print("== mustpass: %d cases, %d with a result in these records (%d case lines seen; no case filter "
                  "line, so the scope is the whole mustpass)"
                  % (len(mustpass), sum(c in status for c in mustpass), len(status)))
        print("%-28s %7s %6s %6s %6s %6s %6s" % ("group", "pass", "fail", "n.s.", "QW", "other", "none"))
        tot = dict.fromkeys(cols, 0)
        for g in sorted(groups):
            v = groups[g]
            for k in cols:
                tot[k] += v[k]
            print("%-28s %7d %6d %6d %6d %6d %6d" % ((g,) + tuple(v[k] for k in cols)))
        print("%-28s %7d %6d %6d %6d %6d %6d" % (("TOTAL",) + tuple(tot[k] for k in cols)))
        print("== not Pass, by name")
        for c in mustpass:
            if status.get(c) != "Pass":
                print("%s %s" % (status.get(c, "NoResult"), c))
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
        diff = [(c, base[c], status.get(c, "NoResult")) for c in mustpass if base[c] != status.get(c, "NoResult")]
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
    for path in a.logs:
        with open(path, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        verdict, got, tail = read_log(text)
        print("%s: %s%s" % (path, " | ".join(tail), (" | " if tail else "") + verdict if verdict else ""))
        missing |= verdict is not None
        status.update(got)

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
    for c in sorted(cases):
        if cases[c] not in ("Pass", "NotSupported"):
            print("%s %s" % (cases[c], c))
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
