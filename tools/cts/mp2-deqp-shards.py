#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Split a dEQP case list into shards that fit the DK's run lifetime.

    python3 -I tools/cts/mp2-deqp-shards.py tools/cts/gles2-mustpass.txt \
        --out DIR [--times times.tsv] [--budget-ms 15000] [--default-ms 100] \
        [--overhead-ms 0]

Each STM32MP257F-DK run is a one-shot from ST's U-Boot that lasts about 20 s
(the IWDG resets the board after about 32 s; docs/notes/stm32mp257f-dk-uboot-boot.md
section 9, tools/board-stm32mp2.sh). A dEQP-GLES2 sweep (roadmap P9) therefore
runs as many shards, one image run each.

Input: a case list (one dEQP case name per line, '#' comments and blank lines
ignored; a repeated name is kept once, at its first position) and optionally a
time table (TSV: name<TAB>ms, '#' comments ignored; names not in the case list
are ignored, a case without a row costs --default-ms). The table can come from
an i.MX8MP results file: any "name<TAB>ms" extract of it will do.

Output in --out: shard-NNNN.txt (case names, one per line, in input order),
and manifest.tsv (header line, then index<TAB>cases<TAB>est_ms<TAB>file per
shard). Shards are consecutive slices of the input in input order (greedy: a
shard closes when the next case would push it past the budget), so the split
is stable and deterministic: the same inputs give the same bytes. A shard's
estimate is --overhead-ms plus the sum of its cases' ms. A case that cannot
fit a budget alone is an error (exit 1, nothing written), not an over-budget
shard.

How the deqp sample takes a case list (driver/samples/zephyr/deqp, read only):
the image runs `--deqp-case=<filter>` with the filter built in at build time
(CONFIG_DEQP_CASE), or picked at run time from CONFIG_DEQP_CASE_TABLE
(';'-separated filters) by the SNVS selector, on boards that have the record.
main.cpp's argument buffer is 2048 bytes, so one table entry holds
comma-separated patterns up to that size. The sample has no case-list file
option, and the DK image has no file system, so a shard reaches it as a
filter or a table entry.

--deqp-caselist-resource: it exists. Checked in a shallow clone of
KhronosGroup/VK-GL-CTS (main), framework/common/tcuCommandLine.cpp:231
declares `Option<CaseListResource>("deqp-caselist-resource", "Read case list
(in trie format) from given file located application's assets")`. It is read
through the platform's tcu::Archive::getResource (:1753-1765) and is one of
the mutually exclusive case-list options (:1202). The file is a trie
({dEQP-GLES2{info{version,renderer}}}), not a flat list, and the resource
comes from the archive the application supplies (the sample's
deqp_archive.cpp), so using it means embedding the shard in the image.
"""
import argparse
import os
import sys


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        for raw in f:
            s = raw.strip()
            if s and not s.startswith("#"):
                yield s


def load_cases(path):
    seen = set()
    cases = []
    for name in read_lines(path):
        if name not in seen:
            seen.add(name)
            cases.append(name)
    return cases


def load_times(path):
    times = {}
    for line in read_lines(path):
        parts = line.split("\t")
        if len(parts) != 2:
            raise ValueError("time table line is not name<TAB>ms: %r" % line)
        try:
            ms = int(parts[1])
        except ValueError:
            raise ValueError("time table ms is not an integer: %r" % line)
        if ms < 0:
            raise ValueError("negative ms: %r" % line)
        times[parts[0]] = ms
    return times


def plan(cases, times, budget, default_ms, overhead):
    shards = []
    cur, cur_ms = [], overhead
    for c in cases:
        ms = times.get(c, default_ms)
        if overhead + ms > budget:
            raise ValueError("case %s (%d ms + %d ms overhead) exceeds the budget %d ms"
                             % (c, ms, overhead, budget))
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
    ap.add_argument("caselist")
    ap.add_argument("--out", required=True)
    ap.add_argument("--times")
    ap.add_argument("--budget-ms", type=int, default=15000)
    ap.add_argument("--default-ms", type=int, default=100)
    ap.add_argument("--overhead-ms", type=int, default=0)
    a = ap.parse_args(argv)
    if a.budget_ms <= 0 or a.default_ms < 0 or a.overhead_ms < 0:
        ap.error("budget must be positive, default and overhead not negative")
    try:
        cases = load_cases(a.caselist)
        times = load_times(a.times) if a.times else {}
        shards = plan(cases, times, a.budget_ms, a.default_ms, a.overhead_ms)
    except (OSError, ValueError) as e:
        print("mp2-deqp-shards: %s" % e, file=sys.stderr)
        return 1
    os.makedirs(a.out, exist_ok=True)
    rows = ["index\tcases\test_ms\tfile"]
    for i, (names, ms) in enumerate(shards):
        fn = "shard-%04d.txt" % i
        with open(os.path.join(a.out, fn), "w", encoding="utf-8", newline="\n") as f:
            f.write("".join(n + "\n" for n in names))
        rows.append("%d\t%d\t%d\t%s" % (i, len(names), ms, fn))
    with open(os.path.join(a.out, "manifest.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    print("mp2-deqp-shards: %d cases, %d shards, budget %d ms" % (len(cases), len(shards), a.budget_ms))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
