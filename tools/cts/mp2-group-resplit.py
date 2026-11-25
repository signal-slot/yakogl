#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Re-split DK dEQP groups or shards that overran the run lifetime.

    python3 -I tools/cts/mp2-group-resplit.py INPUT --out DIR [--group NAME]
        [--times times.tsv] [--missing missing.txt] [--lifetime 100]
        [--default-ms 100] [--overhead-ms 0]

The STM32MP257F-DK runs dEQP-GLES2 one shot per boot; a run lasts
CONFIG_YAKOGL_STM32MP2_LIFETIME_S (100 s for the sweep images). A group or
shard whose cases do not all finish in that time leaves cases with no result
(docs/notes/stm32mp257f-dk-p9.md, the dEQP sweep section). This tool is the
plan's "re-split any group that overruns the lifetime until every case has a
result" step: it cuts such a group into smaller shards.

INPUT is either
  - a case list for one group (one dEQP case name per line, '#' comments and
    blank lines ignored, a repeated name kept once at its first position, as
    mp2-deqp-shards.py reads it); the group's name is --group, by default the
    file name without its .txt suffix; or
  - a manifest.tsv as mp2-deqp-shards.py writes it (header
    'index<TAB>cases<TAB>est_ms<TAB>file', then one row per shard; the shard
    files are read relative to the manifest). Each shard is re-split on its
    own and its group name is its file name without .txt (shard-0003).

--times is the time table mp2-deqp-shards.py --times and mp2-case-times.py
--times-out use: name<TAB>ms, '#' comments ignored. A case without a row costs
--default-ms. --missing is a case list (mp2-case-times.py --missing-out) of
the cases that got no result. In each input shard, a missing case whose
predecessor got a result (or that is the shard's first case) is the case that
was running when the run ended (a hang, a reset or the lifetime): it goes into
a shard of its own, so the cases after it still get a result. The other
missing cases are planned like any case. With no --missing, or an empty one,
no case is put alone for that reason.

The other cases are cut greedily in input order: a shard closes when the next
case would push --overhead-ms plus the summed case times past the lifetime
(--lifetime seconds, as ms). A case that does not fit the lifetime even alone
also gets a shard of its own (warned on stderr); it cannot be cut further.

Output in --out: <group>.<n>.txt (n from 1; case names, one per line, input
order kept) and manifest.tsv in mp2-deqp-shards.py's format, whose index
column is the shard file name without '.txt' and without a leading 'shard-'
(so mp2-deqp-sweep.sh finds shard-0003.1.txt for index 0003.1). The same
inputs give the same bytes. The last stdout line is
'mp2-group-resplit: shards N cases M alone K' (K: shards holding one case
that was put alone, as hung or as over the lifetime).
Exit 0 ok, 1 for input errors, 2 for usage errors.
"""
import argparse
import math
import os
import sys

HEADER = "index\tcases\test_ms\tfile"


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
            ms = float(parts[1])
        except ValueError:
            raise ValueError("time table ms is not a number: %r" % line)
        if ms < 0 or math.isnan(ms) or math.isinf(ms):
            raise ValueError("bad ms: %r" % line)
        times[parts[0]] = int(math.ceil(ms))
    return times


def load_groups(path, group):
    """[(group name, [cases])] from a case list or a manifest.tsv"""
    with open(path, encoding="utf-8") as f:
        first = f.readline().rstrip("\r\n")
    if first == HEADER:
        if group:
            raise ValueError("--group applies to a case list, not to a manifest")
        base = os.path.dirname(path)
        groups = []
        with open(path, encoding="utf-8") as f:
            next(f)
            for raw in f:
                line = raw.rstrip("\r\n")
                if not line:
                    continue
                parts = line.split("\t")
                if len(parts) != 4:
                    raise ValueError("manifest row is not 4 columns: %r" % line)
                fn = parts[3]
                if os.path.basename(fn) != fn:
                    raise ValueError("manifest file is not a plain name: %r" % fn)
                stem = fn[:-4] if fn.endswith(".txt") else fn
                groups.append((stem, load_cases(os.path.join(base, fn))))
        return groups
    if not group:
        group = os.path.basename(path)
        if group.endswith(".txt"):
            group = group[:-4]
    if not group or "/" in group:
        raise ValueError("bad group name: %r" % group)
    return [(group, load_cases(path))]


def plan(cases, times, missing, budget, default_ms, overhead):
    """[(cases, est_ms, alone_reason or None)] in input order"""
    shards = []
    cur, cur_ms = [], overhead

    def close():
        nonlocal cur, cur_ms
        if cur:
            shards.append((cur, cur_ms, None))
        cur, cur_ms = [], overhead

    prev_missing = False
    for i, c in enumerate(cases):
        ms = times.get(c, default_ms)
        is_missing = c in missing
        hung = is_missing and (i == 0 or not prev_missing)
        prev_missing = is_missing
        if hung or overhead + ms > budget:
            close()
            shards.append(([c], overhead + ms, "hung" if hung else "over"))
            continue
        if cur and cur_ms + ms > budget:
            close()
        cur.append(c)
        cur_ms += ms
    close()
    return shards


def main(argv):
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n")[0],
        epilog="See the module docstring (head of the file) for formats.")
    ap.add_argument("input", help="case list of one group, or a mp2-deqp-shards.py manifest.tsv")
    ap.add_argument("--out", required=True, help="output directory")
    ap.add_argument("--group", help="group name for a case-list input (default: file name)")
    ap.add_argument("--times", help="name<TAB>ms time table")
    ap.add_argument("--missing", help="case list of the cases that got no result")
    ap.add_argument("--lifetime", type=float, default=100.0,
                    help="run lifetime in seconds (default 100)")
    ap.add_argument("--default-ms", type=int, default=100,
                    help="cost of a case without a time (default 100)")
    ap.add_argument("--overhead-ms", type=int, default=0,
                    help="per-shard boot overhead (default 0)")
    a = ap.parse_args(argv)
    if a.lifetime <= 0 or a.default_ms < 0 or a.overhead_ms < 0:
        ap.error("lifetime must be positive, default and overhead not negative")
    budget = int(a.lifetime * 1000)
    try:
        groups = load_groups(a.input, a.group)
        times = load_times(a.times) if a.times else {}
        missing = set(load_cases(a.missing)) if a.missing else set()
    except (OSError, ValueError, StopIteration) as e:
        print("mp2-group-resplit: %s" % e, file=sys.stderr)
        return 1
    known = set()
    for _, cases in groups:
        known.update(cases)
    stray = sorted(missing - known)
    if stray:
        print("mp2-group-resplit: %d missing name(s) not in the input, ignored (first %s)"
              % (len(stray), stray[0]), file=sys.stderr)

    out = []  # (stem, cases, est_ms, alone)
    for name, cases in groups:
        for n, (names, ms, alone) in enumerate(
                plan(cases, times, missing, budget, a.default_ms, a.overhead_ms), 1):
            stem = "%s.%d" % (name, n)
            if alone == "over":
                print("mp2-group-resplit: %s: %s alone, %d ms over the lifetime %d ms"
                      % (stem, names[0], ms, budget), file=sys.stderr)
            out.append((stem, names, ms, alone))

    os.makedirs(a.out, exist_ok=True)
    rows = [HEADER]
    for stem, names, ms, _ in out:
        fn = stem + ".txt"
        with open(os.path.join(a.out, fn), "w", encoding="utf-8", newline="\n") as f:
            f.write("".join(c + "\n" for c in names))
        idx = stem[len("shard-"):] if stem.startswith("shard-") else stem
        rows.append("%s\t%d\t%d\t%s" % (idx, len(names), ms, fn))
    with open(os.path.join(a.out, "manifest.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    ncases = sum(len(s[1]) for s in out)
    nalone = sum(1 for s in out if s[3])
    print("mp2-group-resplit: shards %d cases %d alone %d" % (len(out), ncases, nalone))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
