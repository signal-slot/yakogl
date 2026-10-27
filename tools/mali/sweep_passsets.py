#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mechanical check of the T6 sweep PASS sets and of corpus-verdicts.txt.

    python3 -I tools/mali/sweep_passsets.py [--require <case,case,...>] [--delta] <LOG_DIR>
    python3 -I tools/mali/sweep_passsets.py corpus <OLD_REV> [<NEW_REV>]

Sets mode. Each record is a directory of LOG_DIR, laid out like
tools/cts/log/<date>-<item>/. A record holds at least one complete
`panthor-gltest --sweep` log: a `*.verdicts.txt` file (tools/cts/log:
`sweep-N.verdicts.txt`) or a `*.log` file (a raw run log; the retired board
records were of this kind), with `panthor-gltest: verdict case N name: PASS
...` lines and the `256 case(s) run: ...` summary. A log counts as a sweep
only when its summary covers every case of the sweep, i.e. as many cases as
the largest run under LOG_DIR: case-run logs (one case, a few cases) and the
partial `--set` runs (3 or 50 cases) are not sweeps and are skipped, as are
logs without the summary line (a killed run). README.md, predictions.txt and
the like are not logs. Logs under base/, baseline/, main/, first/ and the like
(the tree before the change) and under mesa/ are not the record's result and
are skipped.

A record's PASS set is the cases that PASS in every one of its sweep logs.
A record that holds several sessions (an ours/ and a tip/ours/, several sweep
pairs) is therefore the intersection over all of them, not its final pair:
lenient, as a case must pass everywhere to count. The records are ordered by
the date in their name, then by the commit that first added them. The newest
record's PASS set must contain that of every earlier record: exit 1 and the
missing cases are named otherwise. A log whose PASS count differs from its
summary line is an error (exit 2).

--require 8,28,...  each named case must PASS in every sweep log of the newest
record: exit 1 and one `REQUIRED case N: ...` line per failing case and log,
naming the log and its verdict line (or that it has none).

--delta  print `case N: <old line> -> <new line>` for every case whose verdict
line differs between the previous record and the newest one, in exact strings
(the `panthor-gltest: verdict case N name: ...` lines). The line of a record is
that of its sweep logs; when they differ the distinct lines are joined with
` || `. A case with no line in one record shows `(none)`. It does not change
the exit code.

Corpus mode. Compares tools/mali/corpus-verdicts.txt at two revisions (git
show <rev>:path; NEW_REV defaults to the working tree file). Lines are
`path: verdict`. Exit 1 if a path that was `ok` is now another verdict or is
gone. Paths that were refused may change or go; new paths are free.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = "tools/mali/corpus-verdicts.txt"
SKIP_DIRS = {"base", "baseline", "main", "first", "mesa", "no-helpers", "dropped",
             "in-place", "stride"}
LOG_SUFFIXES = (".log", ".verdicts.txt")
VERDICT = re.compile(r"^panthor-gltest: verdict case (\d+) \S+: (\S+)", re.M)
VERDICT_LINE = re.compile(r"^panthor-gltest: verdict case (\d+) .*$", re.M)
SUMMARY = re.compile(r"^panthor-gltest: (\d+) case\(s\) run: (.*)$", re.M)


def parse_log(text):
    """(passes, number of verdict lines, summary counts, cases run, verdict
    line of each case) of a log that has the summary line, or None."""
    m = SUMMARY.search(text)
    if not m:
        return None
    counts = {}
    for part in m.group(2).split(","):
        k, _, v = part.strip().rpartition(" ")
        counts[k] = int(v)
    verdicts = {int(n): v for n, v in VERDICT.findall(text)}
    passes = {n for n, v in verdicts.items() if v == "PASS"}
    lines = {int(x.group(1)): x.group(0).strip() for x in VERDICT_LINE.finditer(text)}
    return passes, len(verdicts), counts, int(m.group(1)), lines


def log_files(record):
    for root, dirs, files in os.walk(record):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        for f in sorted(files):
            if f.endswith(LOG_SUFFIXES):
                yield os.path.join(root, f)


def read_logs(record):
    """[(path, parse_log result)] of the logs of a record that have a summary."""
    out = []
    for path in log_files(record):
        with open(path, errors="replace") as f:
            r = parse_log(f.read())
        if r is not None:
            out.append((path, r))
    return out


def record_passset(logs, full, problems):
    """(set, [(path, verdict lines)] of the sweep logs) of a record's logs;
    (None, []) without a sweep."""
    sets = []
    sweeps = []
    for path, (passes, total, counts, run, lines) in logs:
        if run != full:
            continue
        if len(passes) != counts.get("PASS", 0) or total != sum(counts.values()):
            problems.append("%s: %d PASS in %d verdict lines, summary says %s"
                            % (path, len(passes), total, counts))
        sets.append(passes)
        sweeps.append((path, lines))
    if not sets:
        return None, []
    return set.intersection(*sets), sweeps


def record_line(sweeps, case):
    """The verdict line of a case in a record's sweep logs; the distinct lines
    joined with ' || ' when the logs differ."""
    seen = []
    for _, lines in sweeps:
        line = lines.get(case)
        if line is not None and line not in seen:
            seen.append(line)
    return " || ".join(seen) if seen else "(none)"


def require_check(sweeps, required):
    """Print one line per required case that does not PASS in a sweep log; return the count."""
    bad = 0
    for path, lines in sweeps:
        for case in required:
            line = lines.get(case)
            if line is None:
                print("REQUIRED case %d: no verdict line in %s" % (case, path))
                bad += 1
            elif not re.match(r"panthor-gltest: verdict case \d+ \S+: PASS\b", line):
                print("REQUIRED case %d: not PASS in %s: %s" % (case, path, line))
                bad += 1
    return bad


def delta(old_sweeps, new_sweeps):
    """(case, old line, new line) of every case whose line differs between two records."""
    cases = set()
    for _, lines in old_sweeps + new_sweeps:
        cases |= set(lines)
    out = []
    for case in sorted(cases):
        a, b = record_line(old_sweeps, case), record_line(new_sweeps, case)
        if a != b:
            out.append((case, a, b))
    return out


def added_time(record):
    try:
        out = subprocess.run(["git", "log", "--format=%ct", "--diff-filter=A", "--",
                              os.path.basename(record)], capture_output=True, text=True,
                             cwd=os.path.dirname(os.path.abspath(record)))
        times = [int(x) for x in out.stdout.split()]
        return min(times) if times else 0
    except (OSError, ValueError):
        return 0


def sets_mode(root, required=None, want_delta=False):
    problems = []
    per_record = []
    for name in sorted(os.listdir(root)):
        path = os.path.join(root, name)
        if os.path.isdir(path):
            per_record.append((name, path, read_logs(path)))
    full = max((r[3] for _, _, logs in per_record for _, r in logs), default=0)
    records = []
    for name, path, logs in per_record:
        s, sweeps = record_passset(logs, full, problems)
        if s is not None:
            records.append((name[:10], added_time(path), name, s, sweeps))
    records.sort(key=lambda r: (r[0], r[1], r[2]))
    if not records:
        print("sweep_passsets: no sweep record under %s" % root)
        return 2
    for _, _, name, s, sweeps in records:
        print("%-32s %3d PASS (%d sweep log(s))" % (name, len(s), len(sweeps)))
    newest = records[-1]
    bad = 0
    for _, _, name, s, _ in records[:-1]:
        missing = sorted(s - newest[3])
        if missing:
            bad += 1
            print("SHRINK %s has %d PASS case(s) the newest (%s) lacks: %s"
                  % (name, len(missing), newest[2], " ".join(map(str, missing))))
    for p in problems:
        print("ERROR " + p)
    if problems:
        return 2
    missed = require_check(newest[4], required) if required else 0
    if want_delta:
        if len(records) < 2:
            print("sweep_passsets: --delta needs two records, found %d" % len(records))
        else:
            old = records[-2]
            changes = delta(old[4], newest[4])
            print("delta %s -> %s: %d case(s) differ" % (old[2], newest[2], len(changes)))
            for case, a, b in changes:
                print("case %d: %s -> %s" % (case, a, b))
    if bad:
        print("sweep_passsets: %d of %d earlier record(s) not a subset of %s"
              % (bad, len(records) - 1, newest[2]))
    if missed:
        print("sweep_passsets: %d required case check(s) failed in %s" % (missed, newest[2]))
    if bad or missed:
        return 1
    print("sweep_passsets: %d record(s), each earlier PASS set is a subset of %s (%d PASS)"
          % (len(records), newest[2], len(newest[3])))
    return 0


def parse_corpus(text):
    d = {}
    for line in text.splitlines():
        path, sep, verdict = line.partition(": ")
        if sep:
            d[path] = verdict.strip()
    return d


def corpus_compare(old, new):
    """The (path, new verdict or None) of every ok line of old that is not ok in new."""
    return [(p, new.get(p)) for p, v in sorted(old.items()) if v == "ok" and new.get(p) != "ok"]


def git_show(rev):
    r = subprocess.run(["git", "show", "%s:%s" % (rev, CORPUS)], capture_output=True,
                       text=True, cwd=HERE)
    if r.returncode:
        raise SystemExit("sweep_passsets: git show %s:%s: %s" % (rev, CORPUS, r.stderr.strip()))
    return r.stdout


def corpus_mode(old_rev, new_rev):
    old = parse_corpus(git_show(old_rev))
    if new_rev:
        new = parse_corpus(git_show(new_rev))
    else:
        top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True,
                             text=True, cwd=HERE).stdout.strip()
        with open(os.path.join(top, CORPUS)) as f:
            new = parse_corpus(f.read())
    lost = corpus_compare(old, new)
    for path, v in lost:
        print("LOST %s: ok -> %s" % (path, v if v is not None else "(gone)"))
    n_ok = sum(1 for v in old.values() if v == "ok")
    if lost:
        print("sweep_passsets: %d of %d ok line(s) lost" % (len(lost), n_ok))
        return 1
    print("sweep_passsets: all %d ok line(s) of %s still ok" % (n_ok, old_rev))
    return 0


def main(argv):
    if argv[:1] == ["corpus"] and 2 <= len(argv) <= 3:
        return corpus_mode(argv[1], argv[2] if len(argv) == 3 else None)
    required, want_delta, rest = None, False, []
    args = list(argv)
    while args:
        a = args.pop(0)
        if a == "--delta":
            want_delta = True
        elif a == "--require" and args:
            try:
                required = [int(x) for x in args.pop(0).split(",") if x.strip()]
            except ValueError:
                print(__doc__)
                return 2
        else:
            rest.append(a)
    if len(rest) == 1 and rest[0] != "corpus":
        return sets_mode(rest[0], required, want_delta)
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
