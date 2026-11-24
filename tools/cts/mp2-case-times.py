#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case times and missing cases from STM32MP257F-DK dEQP console logs.

    python3 -I tools/cts/mp2-case-times.py LOG... [--cases cases.txt]
        [--times-out times.tsv] [--missing-out missing.txt] [--floor-ms 100]

mp2-case-times turns the shard logs of a DK dEQP sweep (shard-NNNN.log, the
runner's output plus the console.txt files, see mp2-deqp-sweep.sh) into (a) a
time table that mp2-deqp-shards.py --times accepts, so the next sweep is
planned from the board's own times, and (b) the list of the cases that have no
result, in --cases order, ready to feed back into mp2-deqp-shards.py for a
re-split of the shards the run lifetime cut.

Log lines: 'deqp: <Status> <case>' result lines (as deqp-uart-tally.py reads
them) and 'deqp: case filter ...'. Time stamps are the board runner's two
console formats: 'HH:MM:SS line' (console.txt, whole seconds, midnight wrap
handled) and '<epoch>.<fraction> line' (console.ts of the serial bridge).
Lines without a stamp are ignored for timing and break the chain.

A case's time is the gap between its start and its result line. dEQP prints no
start line, so the start is the previous result line of the same log or, for
the first case, the 'deqp: case filter' line. A case with no such anchor (no
stamp, other stamp kind) gets a result but no time. With whole-second stamps
the gap is taken as printed (the true time lies within one second of it) and
a case gets at least --floor-ms (default 100 ms, mp2-deqp-shards.py's default
cost): a 0 s gap is a case faster than the stamp can show. A case that appears
in several logs (or twice in one) takes the last occurrence, as mp2-p9-diff.py
does. A last result line without its newline is dropped (the capture was cut
mid-line, the name may be cut).

Output: --times-out is name<TAB>ms, one row per timed case, in first-seen
order. --missing-out holds the --cases names without a result in any log.
stdout ends with 'mp2-case-times: cases C, timed T, missing M, max_ms X'
(C is the --cases count, or the number of cases with a result without --cases).
Exit 0 ok, 2 for usage or input errors.
"""
import argparse
import re
import sys

LINE = re.compile(r"^deqp: (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                  r"InternalError|ResourceError|Crash|Timeout) (\S+)")
FILTER = re.compile(r"^deqp: case filter")
HMS = re.compile(r"^(\d{2}):(\d{2}):(\d{2}) (.*)$", re.S)
EPOCH = re.compile(r"^(\d{9,11}\.\d+) (.*)$", re.S)


def stamp(line):
    """(kind, seconds, rest); kind None when the line has no stamp"""
    m = EPOCH.match(line)
    if m:
        return "e", float(m.group(1)), m.group(2)
    m = HMS.match(line)
    if m:
        return "h", int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3)), m.group(4)
    return None, None, line


def scan(text, floor_ms):
    """{case: ms or None} of one log, the last occurrence of a case winning"""
    out = {}
    raw = text.replace("\0", "").replace("\r", "").split("\n")
    if raw and raw[-1] == "":
        raw.pop()
    elif raw and LINE.match(stamp(raw[-1])[2]):
        raw.pop()  # cut mid-line
    prev = None  # (kind, seconds)
    for l in raw:
        kind, sec, rest = stamp(l)
        m = LINE.match(rest)
        if not m and not FILTER.match(rest):
            continue
        if kind is None:
            prev = None
            if m:
                out.pop(m.group(2), None)
                out[m.group(2)] = None
            continue
        if m:
            ms = None
            if prev and prev[0] == kind:
                gap = sec - prev[1]
                if kind == "h" and gap < 0:
                    gap += 86400
                if gap >= 0:
                    ms = max(int(round(gap * 1000)), floor_ms)
            out.pop(m.group(2), None)
            out[m.group(2)] = ms
        prev = (kind, sec)
    return out


def read(path):
    with open(path, errors="replace") as fh:
        return fh.read()


def names_of(path):
    out, seen = [], set()
    for l in read(path).splitlines():
        l = l.split("#", 1)[0].strip()
        if l and l not in seen:
            seen.add(l)
            out.append(l)
    return out


class Usage(argparse.ArgumentParser):
    def error(self, msg):
        print("mp2-case-times: %s" % msg, file=sys.stderr)
        sys.exit(2)


def main():
    ap = Usage(prog="mp2-case-times", description=__doc__.split("\n")[0])
    ap.add_argument("logs", nargs="+")
    ap.add_argument("--cases")
    ap.add_argument("--times-out")
    ap.add_argument("--missing-out")
    ap.add_argument("--floor-ms", type=int, default=100)
    a = ap.parse_args()
    if a.floor_ms < 0:
        ap.error("--floor-ms is negative")
    try:
        cases = names_of(a.cases) if a.cases else None
        got = {}
        for p in a.logs:
            for k, v in scan(read(p), a.floor_ms).items():
                got.pop(k, None)  # last log wins
                got[k] = v
    except OSError as e:
        print("mp2-case-times: %s" % e, file=sys.stderr)
        return 2
    timed = [(k, v) for k, v in got.items() if v is not None]
    missing = [c for c in cases if c not in got] if cases is not None else []
    try:
        if a.times_out:
            with open(a.times_out, "w") as fh:
                fh.write("# name<TAB>ms, from mp2-case-times.py\n")
                for k, v in timed:
                    fh.write("%s\t%d\n" % (k, v))
        if a.missing_out:
            with open(a.missing_out, "w") as fh:
                fh.writelines(c + "\n" for c in missing)
    except OSError as e:
        print("mp2-case-times: %s" % e, file=sys.stderr)
        return 2
    n = len(cases) if cases is not None else len(got)
    print("mp2-case-times: cases %d, timed %d, missing %d, max_ms %d"
          % (n, len(timed), len(missing), max([v for _, v in timed], default=0)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
