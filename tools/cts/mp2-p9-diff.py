#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Diff a STM32MP257F-DK dEQP-GLES2 run against the i.MX8MP baseline (roadmap P9).

    tools/cts/mp2-p9-diff.py [--baseline results.tsv] [--accepted FILE]
                             [--mustpass FILE] [--exclude FILE] [--report-only]
                             RESULT...

RESULT is a DK results.tsv (the per-row form tools/board.sh writes, last
column the ';'-separated failing names) or a UART log (read with
deqp-uart-tally.py's own parser, scan(): "deqp: <Status> <case>" lines).
Several logs are merged, the last status of a case wins. The defaults are the
i.MX8MP files tools/cts/RESULTS.md cites.

A failure is any status but Pass, NotSupported, QualityWarning and
CompatibilityWarning (a log), or a name in the failing column (a results.tsv).
Classes, each with a count line and the sorted names:

  shared    fails on the i.MX8MP and on the DK
  new       fails on the DK only (a regression against the baseline)
  fixed     fails on the i.MX8MP, passes on the DK (a log: has a non-failing
            result; a results.tsv: is not listed, so it may also not have run)
  accepted  an es2-accepted.txt name whose state differs from the baseline.
            The baseline state is Fail when the i.MX8MP lists it, else
            NotSupported (what the file records). A log gives the DK's status;
            a results.tsv only tells Fail (listed), anything else is unknown
            and not reported.
  missing   mustpass names (inside the run's "case filter" lines, if any) with
            no result in a log. A results.tsv lists no passing names, so
            nothing can be missing there (said on the line).

A warning (QualityWarning, CompatibilityWarning) is not a failure on the DK.
A name the i.MX8MP baseline lists in its failing column (its results.tsv
cannot tell a warning from a failure) and the DK log shows as a warning is
therefore "fixed"; a warning on both boards, with the baseline not listing the
name, appears in no class. Exclude such names if they are not wanted.

--exclude FILE drops names before any class or count, from the baseline, the
DK results, the accepted list and the mustpass scope alike. The syntax is
tools/cts/mp2-deqp-gles2-exclude.txt's (as mp2-deqp-sweep.sh reads it): "#"
comment lines, the name up to the first TAB (the reason follows), a name
ending in "*" is a prefix (a group). The summary then ends ", excluded E": E is
the number of distinct names of the baseline, the DK results, the accepted
list and the mustpass list that the file covers.

Last line: "mp2-p9-diff: shared S, new N, fixed F, missing M" (plus ", excluded
E" with --exclude). Exit 1 when N > 0 or M > 0, unless --report-only; 2 on a
usage or input error.
"""
import argparse
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
NONFAIL = ("Pass", "NotSupported", "QualityWarning", "CompatibilityWarning")


def load_tally():
    spec = importlib.util.spec_from_file_location(
        "deqp_uart_tally", os.path.join(HERE, "deqp-uart-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def is_tsv(text):
    for l in text.splitlines():
        if not l.strip():
            continue
        if l.startswith("# index"):
            return True
        f = l.split("\t")
        return len(f) >= 9 and f[0].isdigit()
    return False


def tsv_failures(text):
    out = set()
    for l in text.splitlines():
        f = l.split("\t")
        if l.startswith("#") or len(f) < 9 or not f[0].isdigit():
            continue
        out.update(n for n in f[8].split(";") if n and n != "-")
    return out


def names_of(path):
    out = []
    with open(path) as fh:
        for l in fh:
            l = l.split("#", 1)[0].strip()
            if l:
                out.append(l)
    return out


def exclude_patterns(path):
    out = []
    with open(path) as fh:
        for l in fh:
            l = l.rstrip("\n")
            if l.startswith("#") or not l.strip():
                continue
            out.append(l.split("\t", 1)[0].strip())
    return out


def covered(name, pats):
    for g in pats:
        if g == name or (g.endswith("*") and name.startswith(g[:-1])):
            return True
    return False


def read(path):
    with open(path, errors="replace") as fh:
        return fh.read()


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("result", nargs="+")
    ap.add_argument("--baseline", default=os.path.join(HERE, "imx8mp-full-tree-29460f96.results.tsv"))
    ap.add_argument("--accepted", default=os.path.join(HERE, "es2-accepted.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles2-mustpass.txt"))
    ap.add_argument("--exclude", metavar="FILE")
    ap.add_argument("--report-only", action="store_true")
    a = ap.parse_args()
    try:
        base_fail = tsv_failures(read(a.baseline))
        accepted = names_of(a.accepted)
        mustpass = names_of(a.mustpass)
        texts = [read(p) for p in a.result]
        pats = exclude_patterns(a.exclude) if a.exclude else None
    except OSError as e:
        print("mp2-p9-diff: %s" % e, file=sys.stderr)
        return 2

    tsv = [is_tsv(t) for t in texts]
    if any(tsv) and not all(tsv):
        print("mp2-p9-diff: results.tsv and UART logs cannot be mixed", file=sys.stderr)
        return 2
    from_tsv = all(tsv)

    status = {}   # log: case -> status
    filters = []
    tally = None
    if from_tsv:
        dk_fail = set()
        for t in texts:
            dk_fail |= tsv_failures(t)
    else:
        tally = load_tally()
        for t in texts:
            r = tally.scan(t)
            status.update(r["got"])
            filters.extend(r["filters"])
        dk_fail = {c for c, s in status.items() if s not in NONFAIL}

    excluded = set()
    if pats is not None:
        pool = set(base_fail) | set(dk_fail) | set(status) | set(accepted) | set(mustpass)
        excluded = {n for n in pool if covered(n, pats)}
        base_fail = base_fail - excluded
        dk_fail = dk_fail - excluded
        status = {c: s for c, s in status.items() if c not in excluded}
        accepted = [n for n in accepted if n not in excluded]
        mustpass = [n for n in mustpass if n not in excluded]

    shared = sorted(base_fail & dk_fail)
    new = sorted(dk_fail - base_fail)
    fixed = sorted(n for n in base_fail - dk_fail if from_tsv or n in status)

    changed = []
    for n in sorted(set(accepted)):
        was = "Fail" if n in base_fail else "NotSupported"
        if from_tsv:
            now = "Fail" if n in dk_fail else None
        else:
            now = status.get(n, "NoResult")
        if now is not None and now != was:
            changed.append("%s (%s -> %s)" % (n, was, now))

    if from_tsv:
        missing = []
        mnote = " (a results.tsv lists no passing names: not checkable)"
    else:
        scope = tally.scope_of(mustpass, filters)[0] if filters else mustpass
        missing = sorted(c for c in scope if c not in status)
        mnote = ""

    def show(title, names, note=""):
        print("== %s: %d%s" % (title, len(names), note))
        for n in names:
            print(n)

    show("failures shared with the i.MX8MP", shared)
    show("DK-only failures (new)", new)
    show("i.MX8MP failures the DK passes (fixed)", fixed)
    show("accepted names that changed state", changed)
    show("mustpass names missing from the DK run", missing, mnote)
    print("mp2-p9-diff: shared %d, new %d, fixed %d, missing %d%s"
          % (len(shared), len(new), len(fixed), len(missing),
             ", excluded %d" % len(excluded) if pats is not None else ""))
    return 1 if (new or missing) and not a.report_only else 0


if __name__ == "__main__":
    sys.exit(main())
