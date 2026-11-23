#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Split gl_test's case set into shards that fit the DK's run lifetime.

    python3 -I tools/cts/mp2-gltest-shards.py --out DIR \
        [--source driver/tools/pvrlab/gl_test.c | --list cases.txt] \
        [--times times.tsv] [--budget-ms 15000] [--default-ms 250] \
        [--overhead-ms 2000] [--order set|number] [--exclude 25,26,39]

Each STM32MP257F-DK run is a one-shot of about 20 s (the IWDG and the runner's
timeouts end it; docs/notes/stm32mp257f-dk-bringup.md, mp2-deqp-shards.py).
gl_test's ~235 cases (P9) therefore run as several images, one shard each.

Case list. Default: the source. driver/tools/pvrlab/gl_test.c's `case_names[]`
table (gl_test.c:8671-8686, `[N] = "name"`) holds every case number gl_test
knows; the number is the `gl_case` that gl_test_run dispatches (the switch in
run_case; a gap hits `default: ... return -2`, gl_test.c:~24439). 7 ("all") is
the regression set itself, not a case: always left out. --list reads a file
instead: one case per line, `N` or `N name`, '#' comments and blanks ignored;
a repeated number is kept once. --exclude takes more numbers out (for example
the minutes-long 25 stress, 26 heavy, 39 perf of prj_sweep.conf, or 10 prims,
which the DK's GL_TEST_MP2_SKIP leaves out).

Order. --order set (default) puts the regression set's cases first, in the
runner's SET_ORDER (driver/tools/pvrlab/runner/src/lib.rs:52, read from the
source tree when this script sits in the repository), then every other case by
number. --order number is plain ascending. A shard is a consecutive slice, so
the split is deterministic: same inputs, same bytes.

Time table (optional, TSV `N<TAB>ms`; numbers not in the case list are
ignored). A case without a row costs --default-ms (250: a guess, not a
measurement). A shard's estimate is --overhead-ms (2000: bring-up and the
record, also a guess) plus its cases. A case that cannot fit a budget alone is
an error (exit 1, nothing written).

Output in --out: shard-NNNN.txt (case numbers one per line),
manifest.tsv (index<TAB>cases<TAB>est_ms<TAB>file, the columns of
mp2-deqp-shards.py's), and selections.txt (what each shard needs, below);
stdout repeats the selections.

Selecting a shard in the sample (driver/samples/zephyr/gl_test, read only, plan
#349's). src/main_stm32mp2.c run() takes: CONFIG_GL_TEST_MP2_CASE (Kconfig: one
case number, 7 = the set), or CONFIG_GL_TEST_MP2_SET_HEAD=y with
CONFIG_GL_TEST_MP2_SET_HEAD_CASES=n (1..50: the first n cases of SET_ORDER,
walked through gl_test_set_list on one context). It cannot select an arbitrary
list: the AM62P (GL_TEST_CASES), the Pi 4 (V3D_GL_TEST_CASES) and the NanoPC-T6
(T6_GL_TEST_CASES) have a comma-separated list option and the DK has none,
although gl_test_set_list takes any list (up to 256 cases). So per shard this
tool prints
  exact:   a shard that is one case, or the first n cases of SET_ORDER
  closest: otherwise CONFIG_GL_TEST_MP2_CASE=N per case (one image each), and
           the change the sample needs: a Kconfig string GL_TEST_MP2_CASES
           (comma-separated) and, in main_stm32mp2.c run(), a branch that
           parses it and calls gl_test_set_list(list, n, &stored), as
           main_am62p.c does. The proposed line is printed as
           CONFIG_GL_TEST_MP2_CASES="a,b,...".
Not made here: the sample is #349's.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SOURCE = os.path.join(HERE, "..", "..", "driver", "tools", "pvrlab", "gl_test.c")
DEFAULT_RUNNER = os.path.join(HERE, "..", "..", "driver", "tools", "pvrlab", "runner", "src", "lib.rs")
SET_CASE = 7
HEAD_MAX = 50


def case_table(path):
    """{number: name} from gl_test.c's case_names[] table."""
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    m = re.search(r"case_names\s*\[\s*\]\s*=\s*\{(.*?)\n\};", text, re.S)
    if not m:
        raise ValueError("no case_names[] table in %s" % path)
    body = re.sub(r"/\*.*?\*/", "", m.group(1), flags=re.S)
    out = {}
    for n, name in re.findall(r'\[\s*(\d+)\s*\]\s*=\s*"([^"]*)"', body):
        n = int(n)
        if n in out:
            raise ValueError("case %d listed twice in %s" % (n, path))
        out[n] = name
    if not out:
        raise ValueError("case_names[] of %s is empty" % path)
    return out


def set_order(path):
    """The runner's SET_ORDER, or [] when the file is not there."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return []
    m = re.search(r"SET_ORDER\s*:\s*\[i32;\s*\d+\]\s*=\s*\[(.*?)\];", text, re.S)
    if not m:
        return []
    body = re.sub(r"//[^\n]*", "", m.group(1))
    return [int(x) for x in re.findall(r"\d+", body)]


def list_file(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for raw in f:
            s = raw.strip()
            if not s or s.startswith("#"):
                continue
            tok = s.split()
            if not tok[0].isdigit():
                raise ValueError("case line does not start with a number: %r" % s)
            out.setdefault(int(tok[0]), " ".join(tok[1:]))
    return out


def load_times(path):
    times = {}
    with open(path, encoding="utf-8") as f:
        for raw in f:
            s = raw.rstrip("\n")
            if not s.strip() or s.lstrip().startswith("#"):
                continue
            p = s.split("\t")
            if len(p) != 2 or not p[0].strip().isdigit():
                raise ValueError("time table line is not N<TAB>ms: %r" % s)
            try:
                ms = int(p[1])
            except ValueError:
                raise ValueError("time table ms is not an integer: %r" % s)
            if ms < 0:
                raise ValueError("negative ms: %r" % s)
            times[int(p[0])] = ms
    return times


def ordered(cases, sset, mode):
    nums = sorted(cases)
    if mode == "number":
        return nums
    head = [c for c in sset if c in cases]
    seen = set(head)
    return head + [c for c in nums if c not in seen]


def plan(order, times, budget, default_ms, overhead):
    shards = []
    cur, cur_ms = [], overhead
    for c in order:
        ms = times.get(c, default_ms)
        if overhead + ms > budget:
            raise ValueError("case %d (%d ms + %d ms overhead) exceeds the budget %d ms"
                             % (c, ms, overhead, budget))
        if cur and cur_ms + ms > budget:
            shards.append((cur, cur_ms))
            cur, cur_ms = [], overhead
        cur.append(c)
        cur_ms += ms
    if cur:
        shards.append((cur, cur_ms))
    return shards


def selection(i, cases, sset):
    if len(cases) == 1:
        return "shard %04d: exact: CONFIG_GL_TEST_MP2_CASE=%d" % (i, cases[0])
    if len(cases) <= HEAD_MAX and cases == sset[:len(cases)]:
        return ("shard %04d: exact: CONFIG_GL_TEST_MP2_SET_HEAD=y "
                "CONFIG_GL_TEST_MP2_SET_HEAD_CASES=%d" % (i, len(cases)))
    lst = ",".join(str(c) for c in cases)
    return ("shard %04d: not selectable today; closest: CONFIG_GL_TEST_MP2_CASE=N for each of %s "
            "(one image per case); needs CONFIG_GL_TEST_MP2_CASES=\"%s\" "
            "(new Kconfig string + gl_test_set_list branch in main_stm32mp2.c)" % (i, lst, lst))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True)
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--source", default=DEFAULT_SOURCE)
    src.add_argument("--list")
    ap.add_argument("--runner", default=DEFAULT_RUNNER)
    ap.add_argument("--times")
    ap.add_argument("--budget-ms", type=int, default=15000)
    ap.add_argument("--default-ms", type=int, default=250)
    ap.add_argument("--overhead-ms", type=int, default=2000)
    ap.add_argument("--order", choices=("set", "number"), default="set")
    ap.add_argument("--exclude", default="")
    a = ap.parse_args(argv)
    if a.budget_ms <= 0 or a.default_ms < 0 or a.overhead_ms < 0:
        ap.error("budget must be positive, default and overhead not negative")
    try:
        excl = {SET_CASE}
        for t in a.exclude.split(","):
            if t.strip():
                excl.add(int(t))
        cases = list_file(a.list) if a.list else case_table(a.source)
        cases = {c: n for c, n in cases.items() if c not in excl}
        times = load_times(a.times) if a.times else {}
        sset = set_order(a.runner)
        order = ordered(cases, sset, a.order)
        shards = plan(order, times, a.budget_ms, a.default_ms, a.overhead_ms)
    except (OSError, ValueError) as e:
        print("mp2-gltest-shards: %s" % e, file=sys.stderr)
        return 1
    os.makedirs(a.out, exist_ok=True)
    rows = ["index\tcases\test_ms\tfile"]
    sel = []
    exact_set = sset if a.order == "set" else []
    for i, (nums, ms) in enumerate(shards):
        fn = "shard-%04d.txt" % i
        with open(os.path.join(a.out, fn), "w", encoding="utf-8", newline="\n") as f:
            f.write("".join("%d\n" % n for n in nums))
        rows.append("%d\t%d\t%d\t%s" % (i, len(nums), ms, fn))
        sel.append(selection(i, nums, exact_set))
    with open(os.path.join(a.out, "manifest.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    with open(os.path.join(a.out, "selections.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(sel) + "\n")
    print("\n".join(sel))
    print("mp2-gltest-shards: %d cases, %d shards, budget %d ms" % (len(order), len(shards), a.budget_ms))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
