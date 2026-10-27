#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mechanical check of the T6 sweep PASS sets and of corpus-verdicts.txt.

    python3 -I tools/mali/sweep_passsets.py <BOARD_REF_DIR>
    python3 -I tools/mali/sweep_passsets.py corpus <OLD_REV> [<NEW_REV>]

Sets mode. Each record is a directory of BOARD_REF_DIR (the board
records: they are not kept in this tree; until 2026-10-26 they were
0dce654a8:tools/mali/board-ref/ of the non-public working repository)
holding at least one complete `panthor-gltest --sweep` log: a log with
`panthor-gltest: verdict case N name: PASS ...` lines and the `256 case(s)
run: ...` summary. A log counts as a sweep only when its summary covers every
case of the sweep, i.e. as many cases as the largest run under BOARD_REF_DIR:
single-case runs and the partial `--set` runs (3 or 50 cases) are not sweeps
and are skipped, as are logs without the summary line (a killed run). Logs
under base/, baseline/, main/, first/ and the like (the tree before the
change) and under mesa/ are not the record's result and are skipped.

A record's PASS set is the cases that PASS in every one of its sweep logs.
A record that holds several sessions (an ours/ and a tip/ours/, several sweep
pairs) is therefore the intersection over all of them, not its final pair:
lenient, as a case must pass everywhere to count. The records are ordered by
the date in their name, then by the commit that first added them. The newest
record's PASS set must contain that of every earlier record: exit 1 and the
missing cases are named otherwise. A log whose PASS count differs from its
summary line is an error (exit 2).

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
VERDICT = re.compile(r"^panthor-gltest: verdict case (\d+) \S+: (\S+)", re.M)
SUMMARY = re.compile(r"^panthor-gltest: (\d+) case\(s\) run: (.*)$", re.M)


def parse_log(text):
    """(passes, number of verdict lines, summary counts, cases run) of a log
    that has the summary line, or None."""
    m = SUMMARY.search(text)
    if not m:
        return None
    counts = {}
    for part in m.group(2).split(","):
        k, _, v = part.strip().rpartition(" ")
        counts[k] = int(v)
    verdicts = {int(n): v for n, v in VERDICT.findall(text)}
    passes = {n for n, v in verdicts.items() if v == "PASS"}
    return passes, len(verdicts), counts, int(m.group(1))


def log_files(record):
    for root, dirs, files in os.walk(record):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        for f in sorted(files):
            if f.endswith(".log"):
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
    """(set, number of sweep logs) of a record's logs; (None, 0) without a sweep."""
    sets = []
    for path, (passes, total, counts, run) in logs:
        if run != full:
            continue
        if len(passes) != counts.get("PASS", 0) or total != sum(counts.values()):
            problems.append("%s: %d PASS in %d verdict lines, summary says %s"
                            % (path, len(passes), total, counts))
        sets.append(passes)
    if not sets:
        return None, 0
    return set.intersection(*sets), len(sets)


def added_time(record):
    try:
        out = subprocess.run(["git", "log", "--format=%ct", "--diff-filter=A", "--",
                              os.path.basename(record)], capture_output=True, text=True,
                             cwd=os.path.dirname(os.path.abspath(record)))
        times = [int(x) for x in out.stdout.split()]
        return min(times) if times else 0
    except (OSError, ValueError):
        return 0


def sets_mode(root):
    problems = []
    per_record = []
    for name in sorted(os.listdir(root)):
        path = os.path.join(root, name)
        if os.path.isdir(path):
            per_record.append((name, path, read_logs(path)))
    full = max((r[3] for _, _, logs in per_record for _, r in logs), default=0)
    records = []
    for name, path, logs in per_record:
        s, n = record_passset(logs, full, problems)
        if s is not None:
            records.append((name[:10], added_time(path), name, s, n))
    records.sort(key=lambda r: (r[0], r[1], r[2]))
    if not records:
        print("sweep_passsets: no sweep record under %s" % root)
        return 2
    for _, _, name, s, n in records:
        print("%-32s %3d PASS (%d sweep log(s))" % (name, len(s), n))
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
    if bad:
        print("sweep_passsets: %d of %d earlier record(s) not a subset of %s"
              % (bad, len(records) - 1, newest[2]))
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
    if len(argv) == 1 and argv[0] != "corpus":
        return sets_mode(argv[0])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
