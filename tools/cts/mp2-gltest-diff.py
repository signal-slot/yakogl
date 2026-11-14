#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Diff a STM32MP257F-DK gl_test run against the i.MX8MP baseline (roadmap P9).

    tools/cts/mp2-gltest-diff.py [--results RESULTS.md] [--set lib.rs]
                                 [--expected FILE] [--report-only] RESULT...

RESULT is a DK gl_test UART log or a result list parsed from one. Read:
  "gl_test: case N <name>: PASS|FAIL|TIMEOUT|SKIP[ (why)]"   (runner verdict line)
  "gl_test: case N skipped[: why]"                            (a walk's skip)
  "N PASS|FAIL|TIMEOUT|SKIP"                                  (a result list)
Several inputs are merged, the last line of a case wins. A UART timestamp
prefix is ignored.

The baseline is the i.MX8MP gl_test cell of tools/cts/RESULTS.md: the regression
set is SET_ORDER of driver/tools/pvrlab/runner/src/lib.rs, "case N skipped" is
the case the i.MX8MP skips, "cases A and B nondeterministic" are its flaky ones.
Classes (a count line and the sorted case numbers each):

  new       fails (FAIL, TIMEOUT) on the DK, not on the expected list and not
            flaky on the i.MX8MP: a regression
  flaky     fails on the DK, nondeterministic on the i.MX8MP (not counted new)
  expected  fails on the DK and is on --expected (default
            tools/cts/mp2-gltest-expected.txt, "N  # reason")
  stale     on the expected list but passes on the DK
  skipped   skipped on the DK but not by the i.MX8MP, or the other way round
  missing   a case of the regression set with no verdict on the DK

Last line: "mp2-gltest-diff: new N, flaky F, expected E, missing M". Exit 1 when
N > 0 or M > 0, unless --report-only; 2 on a usage or input error.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
VERDICT = re.compile(r"gl_test: case (\d+) \S+: (PASS|FAIL|TIMEOUT|SKIP)\b")
SKIPPED = re.compile(r"gl_test: case (\d+) skipped\b")
LIST = re.compile(r"^\s*(\d+)\s+(PASS|FAIL|TIMEOUT|SKIP)\s*$")
FAILS = ("FAIL", "TIMEOUT")


def read(path):
    with open(path, errors="replace") as fh:
        return fh.read()


def scan(text):
    got = {}
    for l in text.splitlines():
        m = VERDICT.search(l) or LIST.match(l)
        if m:
            got[int(m.group(1))] = m.group(2)
            continue
        m = SKIPPED.search(l)
        if m:
            got[int(m.group(1))] = "SKIP"
    return got


def set_order(src):
    m = re.search(r"static SET_ORDER:\s*\[i32;\s*\d+\]\s*=\s*\[([^\]]*)\]", src)
    if not m:
        raise ValueError("no SET_ORDER in the runner's lib.rs")
    return [int(x) for x in re.findall(r"\d+", m.group(1))]


def baseline(text):
    """(skipped, flaky) of the i.MX8MP row's gl_test cell."""
    cell = None
    for l in text.splitlines():
        if l.startswith("| i.MX8MP"):
            for c in l.split("|")[2:]:
                if re.search(r"\d+ of \d+ run, case \d+ skipped", c):
                    cell = c
                    break
        if cell:
            break
    if cell is None:
        raise ValueError("no i.MX8MP gl_test cell ('N of N run, case N skipped') in RESULTS.md")
    m = re.search(r"case (\d+) skipped", cell)
    skipped = {int(m.group(1))}
    flaky = set()
    m = re.search(r"cases? ([\d, and]+?) nondeterministic", cell)
    if m:
        flaky = {int(x) for x in re.findall(r"\d+", m.group(1))}
    return skipped, flaky


def expected_cases(path):
    out = set()
    with open(path) as fh:
        for l in fh:
            l = l.split("#", 1)[0].strip()
            if l:
                out.add(int(l))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("result", nargs="+")
    ap.add_argument("--results", default=os.path.join(HERE, "RESULTS.md"))
    ap.add_argument("--set", dest="setsrc",
                    default=os.path.join(ROOT, "driver/tools/pvrlab/runner/src/lib.rs"))
    ap.add_argument("--expected", default=os.path.join(HERE, "mp2-gltest-expected.txt"))
    ap.add_argument("--report-only", action="store_true")
    a = ap.parse_args()
    try:
        order = set_order(read(a.setsrc))
        base_skip, base_flaky = baseline(read(a.results))
        expected = expected_cases(a.expected)
        got = {}
        for p in a.result:
            got.update(scan(read(p)))
    except (OSError, ValueError) as e:
        print("mp2-gltest-diff: %s" % e, file=sys.stderr)
        return 2

    fail = {c for c, s in got.items() if s in FAILS}
    new = sorted(fail - expected - base_flaky)
    flaky = sorted((fail & base_flaky) - expected)
    exp = sorted(fail & expected)
    stale = sorted(c for c in expected if got.get(c) == "PASS")
    dk_skip = {c for c, s in got.items() if s == "SKIP"}
    skipped = ["%d (DK only)" % c for c in sorted(dk_skip - base_skip)]
    skipped += ["%d (i.MX8MP only)" % c
                for c in sorted(c for c in base_skip if c in got and c not in dk_skip)]
    missing = sorted(c for c in order if c not in got)

    def show(title, items):
        print("== %s: %d" % (title, len(items)))
        for i in items:
            print(i)

    show("new failures (DK only)", new)
    show("failures flaky on the i.MX8MP", flaky)
    show("expected differences failing", exp)
    show("expected-list entries that pass (stale)", stale)
    show("skip differences", skipped)
    show("regression-set cases with no verdict", missing)
    print("mp2-gltest-diff: new %d, flaky %d, expected %d, missing %d"
          % (len(new), len(flaky), len(exp), len(missing)))
    return 1 if (new or missing) and not a.report_only else 0


if __name__ == "__main__":
    sys.exit(main())
