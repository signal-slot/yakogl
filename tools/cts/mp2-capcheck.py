#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check a copied STM32MP257F-DK dEQP console capture against its shard.

    python3 -I tools/cts/mp2-capcheck.py CONSOLE --cases shard.txt
    python3 -I tools/cts/mp2-capcheck.py CONSOLE --list expanded.txt [--filter 'P1,P2']
        [--clean-out cleaned.txt]

The DK's console is read through the farm's serial tool, and a board item
copies it into a file in byte ranges. A range boundary can repeat lines (two
ranges overlap) or lose them (a gap). mp2-capcheck says whether one copied
capture of one shard run is whole before mp2-case-times.py or mp2-p9-diff.py
read it:

1. Boundary repeats. A block of lines that equals the block just before it
   (one line or many, exactly, time stamp included) is an overlap of two
   copied ranges: the second copy is dropped and counted. --clean-out writes
   the capture without them.
2. Each case of the shard has exactly one 'deqp: <Status> <name>' line (the
   format of driver/samples/zephyr/deqp/src/deqp_log.c, which
   mp2-case-times.py and mp2-p9-diff.py parse). The shard is --cases (one
   name per line, '#' starts a comment) or the names of --list that the
   --filter patterns match ('*' matches anything, dots too; patterns split by
   ','); without --filter the capture's own "deqp: case filter '...'" line is
   used. A shard name with no line is missing, one with two or more lines is
   a duplicate, a line whose name is not in the shard is unexpected.
3. The tally of the result lines matches the image's own counts:
   'deqp: summary: B cases begun, R with a result, W without' (deqp_log.c)
   and 'deqp: done: P passed, F failed, N not supported, W warnings,
   complete=C' (main.cpp; Pass, NotSupported and Quality/Compatibility
   Warning are counted by name, every other status is a failure, as dEQP's
   TestRunStatus does). A capture without either line (cut tail, a fatal
   end) is a problem too: its counts cannot be checked.

Every problem is one 'capcheck: ...' line; the last line is 'capcheck: ok'
(exit 0) or 'capcheck: N problem(s)' (exit 1). Exit 2: bad arguments or an
unreadable file.
"""

import argparse
import re
import sys

STAMP = re.compile(r"^(?:\d{2}:\d{2}:\d{2}|\d{9,11}\.\d+) ")
LINE = re.compile(r"^deqp: (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                  r"InternalError|ResourceError|Crash|Timeout) (\S+)\s*$")
SUMMARY = re.compile(r"^deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")
DONE = re.compile(r"^deqp: done: (\d+) passed, (\d+) failed, (\d+) not supported, "
                  r"(\d+) warnings, complete=(\d+)")
FILTER_LINE = re.compile(r"^deqp: case filter '([^']*)'")
FATAL = re.compile(r"^deqp: fatal:")
MAX_BLOCK = 512


def drop_repeats(lines):
    """(lines without boundary repeats, number of lines dropped). At each line,
    the longest block (up to MAX_BLOCK lines) that equals the block just kept
    before it is a second copy and is skipped."""
    out = []
    dropped = 0
    i = 0
    n = len(lines)
    while i < n:
        best = 0
        for k in range(min(len(out), n - i, MAX_BLOCK), 0, -1):
            if lines[i] == out[-k] and lines[i:i + k] == out[-k:]:
                best = k
                break
        if best:
            dropped += best
            i += best
            continue
        out.append(lines[i])
        i += 1
    return out, dropped


def read_names(path):
    names = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            s = raw.split("#", 1)[0].strip()
            if s:
                names.append(s)
    return names


def filter_regex(patterns):
    alts = ["".join(".*" if ch == "*" else re.escape(ch) for ch in p) for p in patterns if p]
    return re.compile("^(?:%s)$" % "|".join(alts)) if alts else None


def check(lines, shard):
    """problem lines for one de-duplicated capture against the shard"""
    probs = []
    seen = {}
    order = []
    summaries, dones = [], []
    fatal = False
    for l0 in lines:
        line = STAMP.sub("", l0, count=1)
        m = LINE.match(line)
        if m:
            name = m.group(2)
            if name not in seen:
                order.append(name)
            seen.setdefault(name, []).append(m.group(1))
            continue
        m = SUMMARY.match(line)
        if m:
            summaries.append(tuple(int(x) for x in m.groups()))
        m = DONE.match(line)
        if m:
            dones.append(tuple(int(x) for x in m.groups()))
        if FATAL.match(line):
            fatal = True

    want = set(shard)
    missing = [c for c in shard if c not in seen]
    dups = [c for c in order if len(seen[c]) > 1]
    unexpected = [c for c in order if c not in want]
    for c in missing:
        probs.append(f"missing {c}")
    for c in dups:
        probs.append(f"duplicate {c} ({len(seen[c])} lines: {' '.join(seen[c])})")
    for c in unexpected:
        probs.append(f"unexpected {c}")

    nlines = sum(len(v) for v in seen.values())
    tally = {"pass": 0, "fail": 0, "ns": 0, "warn": 0}
    for v in seen.values():
        for s in v:
            if s == "Pass":
                tally["pass"] += 1
            elif s == "NotSupported":
                tally["ns"] += 1
            elif s in ("QualityWarning", "CompatibilityWarning"):
                tally["warn"] += 1
            else:
                tally["fail"] += 1

    if not summaries:
        probs.append("no 'deqp: summary' line" + (" (deqp: fatal seen)" if fatal else ""))
    elif len(summaries) > 1:
        probs.append(f"{len(summaries)} 'deqp: summary' lines (one run per capture)")
    else:
        b, r, w = summaries[0]
        if r != nlines:
            probs.append(f"summary says {r} with a result, the capture has {nlines} result line(s)")
        if b != r + w:
            probs.append(f"summary does not add up: {b} begun != {r} + {w}")
        if w:
            probs.append(f"summary says {w} case(s) began without a result")
    if not dones:
        probs.append("no 'deqp: done' line" + (" (deqp: fatal seen)" if fatal else ""))
    elif len(dones) > 1:
        probs.append(f"{len(dones)} 'deqp: done' lines (one run per capture)")
    else:
        p, f, ns, w, complete = dones[0]
        got = (tally["pass"], tally["fail"], tally["ns"], tally["warn"])
        if (p, f, ns, w) != got:
            probs.append("done says %d passed, %d failed, %d not supported, %d warnings; "
                         "the capture has %d, %d, %d, %d" % ((p, f, ns, w) + got))
        if complete != 1:
            probs.append(f"done says complete={complete}")
    stats = (f"{len(shard)} shard case(s), {nlines} result line(s): {tally['pass']} Pass, "
             f"{tally['fail']} failed, {tally['ns']} NotSupported, {tally['warn']} warnings")
    return probs, stats


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("console")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--cases", help="the shard's case list, one name per line")
    g.add_argument("--list", help="the expanded case list the filter selects from")
    ap.add_argument("--filter", help="dEQP case patterns, ',' between them (with --list)")
    ap.add_argument("--clean-out", help="write the capture without boundary repeats here")
    a = ap.parse_args(argv)
    if a.filter and not a.list:
        ap.error("--filter needs --list")
    try:
        with open(a.console, "rb") as f:
            text = f.read().decode("utf-8", errors="replace")
        text = text.replace("\0", "").replace("\r", "")
        lines = text.split("\n")
        if lines and lines[-1] == "":
            lines.pop()
        names = read_names(a.cases or a.list)
    except OSError as e:
        print(f"capcheck: {e}", file=sys.stderr)
        return 2

    kept, dropped = drop_repeats(lines)
    if a.cases:
        shard = list(dict.fromkeys(names))
    else:
        pats = a.filter
        if pats is None:
            fl = [m.group(1) for m in (FILTER_LINE.match(STAMP.sub("", l, count=1)) for l in kept) if m]
            if not fl:
                print("capcheck: no --filter and no \"deqp: case filter '...'\" line in the capture",
                      file=sys.stderr)
                return 2
            pats = fl[-1]
        rx = filter_regex(pats.split(","))
        if rx is None:
            print("capcheck: empty filter", file=sys.stderr)
            return 2
        shard = [c for c in dict.fromkeys(names) if rx.match(c)]
        print(f"capcheck: filter '{pats}' selects {len(shard)} of {len(names)} listed case(s)")

    if a.clean_out:
        try:
            with open(a.clean_out, "w", encoding="utf-8") as f:
                f.write("".join(l + "\n" for l in kept))
        except OSError as e:
            print(f"capcheck: {e}", file=sys.stderr)
            return 2

    probs, stats = check(kept, shard)
    print(f"capcheck: {len(lines)} line(s) read, {dropped} boundary-repeat line(s) dropped")
    print(f"capcheck: {stats}")
    for p in probs:
        print(f"capcheck: {p}")
    if probs:
        print(f"capcheck: {len(probs)} problem(s)")
        return 1
    print("capcheck: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
