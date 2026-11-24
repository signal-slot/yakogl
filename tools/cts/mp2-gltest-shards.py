#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Split gl_test's regression set into runs that fit the DK's run lifetime.

    python3 -I tools/cts/mp2-gltest-shards.py --out DIR [--runner lib.rs]
        [--skip 10] [--times times.tsv] [--budget-ms 15000]
        [--default-ms 500] [--overhead-ms 0]

One STM32MP257F-DK run is a one-shot from ST's U-Boot that lasts about 20 s
(the IWDG resets the board after about 32 s; tools/board-stm32mp2.sh). The
whole set on one context (50 cases, SET_ORDER of driver/tools/pvrlab/runner)
may not fit, so it runs as several shards, one image run each.

The case set is read from the `SET_ORDER` array of the runner's lib.rs
(default driver/tools/pvrlab/runner/src/lib.rs, found from this file), minus
--skip (decimal, comma separated; default "10", GL_TEST_MP2_SKIP's default).
--times is a TSV, case<TAB>ms ('#' comments ignored; cases not in the set are
ignored, a case without a row costs --default-ms).

Output in --out: shard-NNNN.txt (case numbers, one per line, in SET_ORDER
order), and manifest.tsv (header, then index, cases, est_ms, list, file per
shard, tab separated; list is the comma-separated case numbers). Shards are
consecutive slices of the set (greedy: a shard closes when the next case would
pass the budget), so cases keep the order the walk has on one context and the
same inputs give the same bytes. A case that cannot fit alone is an error
(exit 1, nothing written).

What the DK's gl_test image can take (main_stm32mp2.c, read only): one case
(GL_TEST_MP2_CASE, or at run time the SNVS selector 1..999) or the head of the
set (GL_TEST_MP2_SET_HEAD_CASES). A shard of one case runs as it is; a shard
that is a later slice of several cases needs a list knob the sample does not
have yet.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_RUNNER = os.path.join(HERE, "..", "..", "driver", "tools", "pvrlab", "runner", "src", "lib.rs")


def set_order(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    m = re.search(r"SET_ORDER\s*:\s*\[\s*i32\s*;\s*(\d+)\s*\]\s*=\s*\[([^\]]*)\]", text)
    if not m:
        raise ValueError("no SET_ORDER array in %s" % path)
    cases = [int(x) for x in re.findall(r"\d+", m.group(2))]
    if len(cases) != int(m.group(1)):
        raise ValueError("SET_ORDER declares %s cases, has %d" % (m.group(1), len(cases)))
    if len(set(cases)) != len(cases):
        raise ValueError("SET_ORDER repeats a case")
    return cases


def parse_skip(s):
    out = set()
    for tok in s.split(","):
        tok = tok.strip()
        if tok:
            if not tok.isdigit():
                raise ValueError("--skip is not a decimal list: %r" % s)
            out.add(int(tok))
    return out


def load_times(path):
    times = {}
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            p = line.split("\t")
            if len(p) != 2 or not p[0].isdigit() or not p[1].isdigit():
                raise ValueError("time table line is not case<TAB>ms: %r" % line)
            times[int(p[0])] = int(p[1])
    return times


def plan(cases, times, budget, default_ms, overhead):
    shards, cur, cur_ms = [], [], overhead
    for c in cases:
        ms = times.get(c, default_ms)
        if overhead + ms > budget:
            raise ValueError("case %d (%d ms + %d ms overhead) exceeds the budget %d ms" % (c, ms, overhead, budget))
        if cur and cur_ms + ms > budget:
            shards.append((cur, cur_ms))
            cur, cur_ms = [], overhead
        cur.append(c)
        cur_ms += ms
    if cur:
        shards.append((cur, cur_ms))
    return shards


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--runner", default=DEFAULT_RUNNER)
    ap.add_argument("--skip", default="10")
    ap.add_argument("--times")
    ap.add_argument("--budget-ms", type=int, default=15000)
    ap.add_argument("--default-ms", type=int, default=500)
    ap.add_argument("--overhead-ms", type=int, default=0)
    a = ap.parse_args(argv)
    if a.budget_ms <= 0 or a.default_ms < 0 or a.overhead_ms < 0:
        ap.error("budget must be positive, default and overhead not negative")
    try:
        skip = parse_skip(a.skip)
        cases = [c for c in set_order(a.runner) if c not in skip]
        times = load_times(a.times) if a.times else {}
        shards = plan(cases, times, a.budget_ms, a.default_ms, a.overhead_ms)
    except (OSError, ValueError) as e:
        print("mp2-gltest-shards: %s" % e, file=sys.stderr)
        return 1
    os.makedirs(a.out, exist_ok=True)
    rows = ["index\tcases\test_ms\tlist\tfile"]
    for i, (names, ms) in enumerate(shards):
        fn = "shard-%04d.txt" % i
        with open(os.path.join(a.out, fn), "w", encoding="utf-8", newline="\n") as f:
            f.write("".join("%d\n" % n for n in names))
        rows.append("%d\t%d\t%d\t%s\t%s" % (i, len(names), ms, ",".join(map(str, names)), fn))
    with open(os.path.join(a.out, "manifest.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    print("mp2-gltest-shards: %d cases, %d shards, budget %d ms" % (len(cases), len(shards), a.budget_ms))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
