#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Summarise one or more dEQP .qpa logs with the CTS's own log parser.

  cts-summary.py [--mustpass FILE] [--by-group N] [--list-failures N] \
                 [--failures-file F] result.qpa [more.qpa ...]

Prints a Pass / Fail / NotSupported / Crash histogram. With --mustpass it also
reports how many of the expected cases produced no result at all, which is what
a run that was abandoned part-way through looks like. With --by-group it prints
the same counts per test group, which is the table that goes into RESULTS.md.

Reading the numbers:

  Pass / Fail       what the test decided.
  NotSupported      the test asked for something the implementation does not
                    advertise and skipped itself. In an AOSP mustpass run this
                    is an allowed outcome, not a failure.
  QualityWarning /  allowed outcomes too: the result was correct but outside a
  CompatibilityWarning  preferred precision or behaviour.
  Crash / Timeout / the process died in that case, or dEQP's watchdog killed it.
  ResourceError /   Anything here is a driver bug, and it also costs the rest of
  InternalError     that run: the next case starts a new process.
  not executed      no result in any log. With the runner's restart loop this
                    should only ever be cases it never reached.
"""
import argparse
import glob
import os
import re
import sys

CTS = os.environ.get("CTS", os.path.expanduser("~/com/github/KhronosGroup/VK-GL-CTS"))
sys.path.insert(0, os.path.join(CTS, "scripts", "log"))
from log_parser import BatchResultParser, StatusCode  # noqa: E402

# Outcomes that mean the implementation is wrong or took the process down.
FAILING = {StatusCode.FAIL, StatusCode.CRASH, StatusCode.TIMEOUT,
           StatusCode.INTERNAL_ERROR, StatusCode.RESOURCE_ERROR}

# Where a shader toolchain says what actually went wrong. dEQP puts it in the
# case log, not in the one-line status detail, and compiler messages are usually
# several lines -- the first is "X failed for shader.vert:" and the reason is on
# the next one. Reading these with a line-based grep drops the reason and makes
# the tool look like it printed nothing, which is worse than silence: it invites
# a bug report against the compiler's log pass-through, which was working.
INFOLOG_RE = re.compile(rb"<InfoLog>(.*?)</InfoLog>", re.DOTALL)
XML_UNESCAPE = ((b"&apos;", b"'"), (b"&quot;", b'"'), (b"&lt;", b"<"),
                (b"&gt;", b">"), (b"&amp;", b"&"))


def info_logs(blob):
    """Every non-empty <InfoLog> in a case log, whole and unescaped."""
    out = []
    for m in INFOLOG_RE.findall(blob or b""):
        for a, b in XML_UNESCAPE:
            m = m.replace(a, b)
        text = m.decode("utf-8", "replace").strip()
        if text:
            out.append(text)
    return out
# Outcomes that take the whole process with them, so they cost a restart.
FATAL = {StatusCode.CRASH, StatusCode.TIMEOUT, StatusCode.INTERNAL_ERROR}


def group_of(name, depth):
    return ".".join(name.split(".")[:depth])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("qpa", nargs="+")
    ap.add_argument("--mustpass", help="the case list the run was given")
    ap.add_argument("--by-group", type=int, metavar="DEPTH", default=0,
                    help="per-group table at DEPTH dot-separated components "
                         "(3 = dEQP-GLES2.functional.shaders)")
    ap.add_argument("--list-failures", type=int, default=20,
                    help="print at most N failing case names (0 = none)")
    ap.add_argument("--failures-file", help="write every failing case to this file")
    ap.add_argument("--logs-file",
                    help="write the compiler/driver logs of every failing case "
                         "here, whole. This is where a shader toolchain says why.")
    args = ap.parse_args()

    # A run's results live in a directory, and passing that directory instead
    # of its logs used to summarise nothing at all -- "results: 0 distinct
    # cases", which reads as a run that produced no results rather than as a
    # mistake at the command line. Expand a directory to the .qpa files in it,
    # and refuse one that holds none rather than report a zero.
    paths = []
    for path in args.qpa:
        if os.path.isdir(path):
            found = sorted(glob.glob(os.path.join(path, "*.qpa")))
            if not found:
                print("cts-summary: %s holds no .qpa files" % path, file=sys.stderr)
                return 2
            paths.extend(found)
        else:
            paths.append(path)

    # A resumed run repeats nothing, but a case retried by hand would: the last
    # result for a name wins, so keep results keyed by name.
    result_of = {}
    for path in paths:
        try:
            parsed = BatchResultParser().parseFile(path)
        except Exception as exc:
            print("cts-summary: %s: %s" % (path, exc), file=sys.stderr)
            continue
        for r in parsed:
            result_of[r.name] = r

    counts = {}
    for r in result_of.values():
        counts[r.statusCode] = counts.get(r.statusCode, 0) + 1

    print("results: %d distinct cases" % len(result_of))
    for code in StatusCode.STATUS_CODES:
        if counts.get(code):
            print("  %-22s %6d" % (code, counts[code]))

    expected = None
    if args.mustpass:
        with open(args.mustpass) as f:
            expected = [l.strip() for l in f if l.strip()]
        missing = [n for n in expected if n not in result_of]
        print("expected: %d, not executed: %d" % (len(expected), len(missing)))
        if missing:
            print("  first not executed: %s" % missing[0])

    if args.by_group:
        names = expected if expected is not None else sorted(result_of)
        order, groups = [], {}
        for name in names:
            g = group_of(name, args.by_group)
            if g not in groups:
                groups[g] = []
                order.append(g)
            groups[g].append(name)
        print()
        print("%-52s %6s %6s %6s %6s %6s %6s %6s" %
              ("group", "cases", "pass", "fail", "notsup", "crash", "other", "noexec"))
        for g in order:
            n = p = f = ns = cr = ot = ne = 0
            for name in groups[g]:
                n += 1
                r = result_of.get(name)
                if r is None:
                    ne += 1
                elif r.statusCode == StatusCode.PASS:
                    p += 1
                elif r.statusCode == StatusCode.FAIL:
                    f += 1
                elif r.statusCode == StatusCode.NOT_SUPPORTED:
                    ns += 1
                elif r.statusCode in FATAL:
                    cr += 1
                else:
                    ot += 1
            print("%-52s %6d %6d %6d %6d %6d %6d %6d" % (g, n, p, f, ns, cr, ot, ne))

    failures = [(n, r.statusCode, r.statusDetails)
                for n, r in sorted(result_of.items()) if r.statusCode in FAILING]
    if args.failures_file:
        with open(args.failures_file, "w") as f:
            for name, code, detail in failures:
                f.write("%s\t%s\t%s\n" % (name, code, detail))
        print("\nfailures written to %s (%d)" % (args.failures_file, len(failures)))

    if args.logs_file:
        n = 0
        with open(args.logs_file, "w") as f:
            for name, r in sorted(result_of.items()):
                if r.statusCode not in FAILING:
                    continue
                logs = info_logs(r.log)
                if not logs:
                    continue
                n += 1
                f.write("=== %s: %s\n" % (name, r.statusCode))
                for text in logs:
                    f.write("%s\n" % text)
                f.write("\n")
        print("\nlogs of %d failing case(s) written to %s" % (n, args.logs_file))

    if failures and args.list_failures:
        shown = failures[:args.list_failures]
        print("\nfailures (first %d of %d):" % (len(shown), len(failures)))
        for name, code, detail in shown:
            print("  %s: %s (%s)" % (name, code, detail))

    # Exit status is about the run, not the results: a run that produced results
    # succeeded even if cases failed. Only "no results at all" is an error.
    return 0 if result_of else 1


if __name__ == "__main__":
    sys.exit(main())
