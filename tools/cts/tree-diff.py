#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case diff of two dEQP-GLES2 result sets against the mustpass list.

    tools/cts/tree-diff.py --baseline A [A...] --new B [B...]
                           [--mustpass tools/cts/gles2-mustpass.txt]
                           [--accepted tools/cts/es2-accepted.txt] [--json]

A is the baseline (an earlier run), B the new run. Each side is one or more
result files, or a directory holding them (a board.sh tree or run outdir: its
`*.qpa` files and its `*/console.txt` run directories). The tool answers,
case by case: what passed in A stopped passing in B (regressed), what started
passing (fixed), what still fails, and what B has no result for. A case that
passed in A and has no result in B is listed under both "regressed" and
"missing from B"; every other case is in exactly one list.

The per-case forms it reads -- the ones this repository's runners produce. The
tool works the form out per file and names it in its output:

  deqp-console  the deqp sample's console lines "deqp: <Status> <case name>",
                as tools/board.sh captures them into a run's console.txt (the
                AM62P and Pi 4 trees). NULs, CRs and a leading "HH:MM:SS "
                time stamp (the Pi 4's UART recorder) are dropped first, as in
                deqp-uart-tally.py. "deqp: case filter '<patterns>'" lines give
                the scope. "deqp: done/summary/fatal" lines are read by
                deqp-uart-tally.py, not here: whether a log is MISSING,
                INCOMPLETE or FATAL is that tool's verdict; this one compares
                the cases that did arrive.

  qpa           dEQP's own TestResults.qpa XML: a "<TestCaseResult
                CasePath=\"...\">" block whose "<Result StatusCode=\"...\">"
                gives the status (host-deqp-run.sh's c_*.qpa, the .qpa files
                qpa-from-uart.py rebuilds from a CONFIG_DEQP_ECHO_QPA console,
                the fixtures in tools/cts/testdata/). The streaming markers
                "#beginTestCaseResult <case>" / "#endTestCaseResult" are
                followed too, and "#terminateTestCaseResult <Status>" -- what
                dEQP's crash handler and cts_resume.py --repair write --
                closes a case with that status. A case opened and never closed
                has no result, as for the CTS's own BatchResultParser.

  statuses      the "<Status> <case name>" lines deqp-uart-tally.py --statuses
                prints and --baseline reads (tools/board.sh diff builds its
                per-case diff from exactly these). "NoResult" means no result.

Not accepted: the selector form's results.tsv and its "TALLY (N entries):
begun ... passed ..." line (the i.MX8MP tree's per-row counts). They are per
row, not per case: only the failing names are per case, and they do not say
which status each has. tools/board.sh diff compares those.

Statuses: Pass, Fail, NotSupported, QualityWarning, CompatibilityWarning,
InternalError, ResourceError, Crash, Timeout (the deqp sample's nine), plus
NoResult in the statuses form. A case with no result on a side is Missing. The
last result for a name wins (files in the order given, lines in file order),
as in deqp-uart-tally.py and cts-summary.py.

Scope. With --mustpass the universe is the mustpass cases the records could
have run: a case no side's "deqp: case filter" patterns match is "outside both
sides' case filters" (run by neither), counted separately and left out of the
diff; a side with no filter line keeps the whole mustpass as its scope, as
deqp-uart-tally.py does. Without --mustpass the universe is the union of the
cases seen in A and B.

--accepted FILE: one case name or glob per line ('*' matches anything, like a
dEQP case filter; a line starting with '#' is a comment, and a trailing
' # ...' on a pattern line is one too). A case matching it is an accepted
non-Pass outcome: it is still listed, marked [accepted], but neither a
regression nor a Fail/Crash/Timeout counts against the exit status.
tools/cts/es2-accepted.txt seeds the buckets docs/es2-cts-failures.md section
C records as acceptable.

Exit status:
  0  B has no Fail, Crash or Timeout outside the accepted list and no
     regression outside it. (InternalError and ResourceError are counted and
     listed under "still fail" but do not change the exit status; the gate is
     the three the conformance goal names.)
  1  otherwise
  2  bad input: a file that cannot be read, holds no recognized per-case
     form, names no case with a result, carries a status this tool does not
     know, or a --mustpass file that is empty.

--json prints the machine-readable form instead of the report, with the same
exit status.
"""
import argparse
import fnmatch
import glob
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

STATUSES = ("Pass", "Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
            "InternalError", "ResourceError", "Crash", "Timeout")
COUNT_COLS = ["Pass", "Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
              "Crash", "Timeout", "Missing"]
EXTRA_COLS = ["InternalError", "ResourceError"]  # counted when nonzero, never gated
GATE_BAD = ("Fail", "Crash", "Timeout")          # what the exit status gates on
LIST_BAD = ("Fail", "Crash", "Timeout", "InternalError", "ResourceError")  # "still fail"

STAMP = re.compile(r"^\d{2}:\d{2}:\d{2} ")
_ALT = "|".join(STATUSES)
CONSOLE = re.compile(r"^deqp: (%s) (\S+)" % _ALT)
FILTER = re.compile(r"^deqp: case filter '([^']*)'")
STATUS_LINE = re.compile(r"^(%s|NoResult) (\S+)$" % _ALT)
QPA_OPEN = re.compile(r"<TestCaseResult\b[^>]*\bCasePath=\"([^\"]+)\"", re.I)
QPA_RESULT = re.compile(r"<Result\b[^>]*\bStatusCode=\"([^\"]+)\"", re.I)
QPA_CLOSE = re.compile(r"</TestCaseResult>", re.I)
BEGIN = re.compile(r"^#beginTestCaseResult (\S+)")
END = re.compile(r"^#endTestCaseResult")
TERMINATE = re.compile(r"^#terminateTestCaseResult (\S+)")


class InputError(Exception):
    pass


def describe():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


def filter_regex(patterns):
    """One regex for dEQP case patterns ('*' matches anything, also dots;
    everything else literally), as in deqp-uart-tally.py."""
    alts = ["".join(".*" if ch == "*" else re.escape(ch) for ch in p) for p in patterns if p]
    return re.compile("^(?:%s)$" % "|".join(alts)) if alts else None


def detect_form(lines, path):
    for l in lines:
        if QPA_OPEN.search(l) or BEGIN.match(l) or TERMINATE.match(l):
            return "qpa"
    for l in lines:
        if CONSOLE.match(l):
            return "deqp-console"
    for l in lines:
        if STATUS_LINE.match(l):
            return "statuses"
    raise InputError("%s: no recognized per-case form (expected 'deqp: <Status> <case>' console "
                     "lines, '<TestCaseResult CasePath=\"...\">' .qpa blocks, or '<Status> <case>' "
                     "statuses lines)" % path)


def parse_cases(form, lines, path):
    """({case: status}, [case filter patterns]) of one file."""
    got, filters = {}, []
    if form == "deqp-console":
        for l in lines:
            m = CONSOLE.match(l)
            if m:
                got[m.group(2)] = m.group(1)
                continue
            f = FILTER.match(l)
            if f:
                filters.extend(f.group(1).split(","))
    elif form == "statuses":
        for l in lines:
            m = STATUS_LINE.match(l)
            if m and m.group(1) != "NoResult":
                got[m.group(2)] = m.group(1)
    else:  # qpa
        case, status = None, None

        def close():
            nonlocal case, status
            if case is not None and status is not None:
                got[case] = status
            case, status = None, None

        for l in lines:
            o, b = QPA_OPEN.search(l), BEGIN.match(l)
            if o or b:
                close()  # an unclosed block is no result, as for BatchResultParser
                case = (o or b).group(1)
            m = QPA_RESULT.search(l)
            if m and case is not None:
                if m.group(1) not in STATUSES:
                    raise InputError("%s: unknown StatusCode %r on %s" % (path, m.group(1), case))
                status = m.group(1)
            t = TERMINATE.match(l)
            if t:
                if case is None:
                    raise InputError("%s: '#terminateTestCaseResult' with no case open" % path)
                if t.group(1) not in STATUSES:
                    raise InputError("%s: unknown status %r on %s" % (path, t.group(1), case))
                got[case] = t.group(1)
                case, status = None, None
                continue
            if QPA_CLOSE.search(l) or END.match(l):
                close()
        # a case left open at EOF has no result
    return got, filters


def load_side(paths, side):
    files = []
    for p in paths:
        if os.path.isdir(p):
            found = sorted(glob.glob(os.path.join(p, "*.qpa"))) + \
                sorted(glob.glob(os.path.join(p, "*", "console.txt")))
            if not found:
                raise InputError("%s: %s holds no .qpa file and no */console.txt run directory"
                                 % (side, p))
            files.extend(found)
        elif os.path.isfile(p):
            files.append(p)
        else:
            raise InputError("%s: no such file: %s" % (side, p))
    status, filters, forms = {}, [], []
    for path in files:
        try:
            raw = open(path, "rb").read()
        except OSError as e:
            raise InputError("%s: cannot read %s: %s" % (side, path, e))
        lines = [STAMP.sub("", l, count=1) for l in
                 raw.decode("utf-8", "replace").replace("\0", "").replace("\r", "").splitlines()]
        form = detect_form(lines, path)
        if form not in forms:
            forms.append(form)
        got, flt = parse_cases(form, lines, path)
        status.update(got)
        filters.extend(flt)
    if not status:
        raise InputError("%s: these files name no case with a result" % side)
    return {"files": files, "form": "+".join(forms), "status": status, "filters": filters}


def load_mustpass(path):
    try:
        cases = [l.strip() for l in open(path, encoding="utf-8") if l.strip() and not l.startswith("#")]
    except OSError as e:
        raise InputError("cannot read --mustpass %s: %s" % (path, e))
    if not cases:
        raise InputError("--mustpass %s names no case" % path)
    return cases


def load_accepted(path):
    """[pattern] from an accepted list; '#' lines are comments, and so is a
    trailing ' # ...'. A file with only comments is an empty accepted list."""
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as e:
        raise InputError("cannot read --accepted %s: %s" % (path, e))
    pats = []
    for l in text.splitlines():
        l = l.strip()
        if not l or l.startswith("#"):
            continue
        l = l.split(" #", 1)[0].strip()
        if l:
            pats.append(l)
    return pats


def main(argv):
    ap = argparse.ArgumentParser(
        prog="tree-diff.py",
        description="Per-case diff of two dEQP-GLES2 result sets: A (the baseline) against "
                    "B (the new run), over the mustpass list. Reads the per-case forms the "
                    "repository's runners produce: 'deqp: <Status> <case>' console lines, "
                    "TestResults.qpa XML, and '<Status> <case>' statuses lines.")
    ap.add_argument("--baseline", nargs="+", metavar="FILE",
                    help="A: baseline result files, or directories holding them "
                         "(*.qpa and */console.txt)")
    ap.add_argument("--new", nargs="+", metavar="FILE",
                    help="B: new-run result files, or directories holding them")
    ap.add_argument("--mustpass", metavar="FILE",
                    help="the mustpass case list (tools/cts/gles2-mustpass.txt): the diff "
                         "universe, narrowed to the cases the records' 'deqp: case filter' "
                         "patterns could have run; without it the universe is the union of "
                         "the cases seen in A and B")
    ap.add_argument("--accepted", metavar="FILE",
                    help="accepted non-Pass outcomes (tools/cts/es2-accepted.txt): one case "
                         "name or glob per line, a '#' comment gives the reason; matching "
                         "cases are listed but do not count against the exit status")
    ap.add_argument("--json", action="store_true",
                    help="print the machine-readable form instead of the report")
    ap.add_argument("--self-test", action="store_true",
                    help="run the fixture checks in tools/cts/testdata/tree-diff/ "
                         "(the same ones tools/cts/tree-diff-selftest.sh runs)")
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test()
    if not a.baseline or not a.new:
        ap.error("--baseline and --new are required")  # argparse exits 2

    try:
        A = load_side(a.baseline, "A (baseline)")
        B = load_side(a.new, "B (new run)")
        mustpass = load_mustpass(a.mustpass) if a.mustpass else None
        accepted = load_accepted(a.accepted) if a.accepted else []
    except InputError as e:
        print("tree-diff: %s" % e, file=sys.stderr)
        return 2

    def is_acc(c):
        return any(fnmatch.fnmatchcase(c, p) for p in accepted)

    ra, rb = filter_regex(A["filters"]), filter_regex(B["filters"])
    if mustpass is not None:
        universe = [c for c in mustpass if (ra is None or ra.match(c)) or (rb is None or rb.match(c))]
        outside = len(mustpass) - len(universe)
        scope = {"mode": "mustpass", "path": a.mustpass, "total": len(mustpass),
                 "in_scope": len(universe), "outside_case_filters": outside}
    else:
        universe = sorted(set(A["status"]) | set(B["status"]))
        outside = 0
        scope = {"mode": "union", "path": None, "total": None,
                 "in_scope": len(universe), "outside_case_filters": 0}

    def counts(status):
        c = dict.fromkeys(COUNT_COLS + EXTRA_COLS, 0)
        for case in universe:
            s = status.get(case)
            c[s if s else "Missing"] += 1
        return c

    ca, cb = counts(A["status"]), counts(B["status"])
    regressed, fixed, still_fail, missing_b, other = [], [], [], [], []
    for c in universe:
        sa, sb = A["status"].get(c), B["status"].get(c)
        if sb is None and sa is not None:
            missing_b.append((c, sa, sb))  # also a regression when A passed
        if sa == "Pass" and sb != "Pass":
            regressed.append((c, sa, sb))
        elif sa != "Pass" and sb == "Pass":
            fixed.append((c, sa, sb))
        elif sa in LIST_BAD and sb in LIST_BAD:
            still_fail.append((c, sa, sb))
        elif sb is not None and sa != sb:
            other.append((c, sa, sb))  # a change that is none of the above

    bad_b = [c for c in universe if B["status"].get(c) in GATE_BAD and not is_acc(c)]
    reg_out = [e for e in regressed if not is_acc(e[0])]
    extra_b = sum(1 for c in universe if B["status"].get(c) in EXTRA_COLS)
    acc_n = sum(1 for c in universe if is_acc(c))
    clean = not bad_b and not reg_out
    rc = 0 if clean else 1

    if a.json:
        def entries(lst):
            return [{"case": c, "a": sa or "Missing", "b": sb or "Missing", "accepted": is_acc(c)}
                    for c, sa, sb in lst]
        doc = {
            "tool": "tree-diff", "build": describe(),
            "a": {"files": A["files"], "form": A["form"]},
            "b": {"files": B["files"], "form": B["form"]},
            "scope": scope,
            "accepted": {"path": a.accepted, "patterns": len(accepted), "matching_cases": acc_n},
            "counts": {"A": ca, "B": cb},
            "regressed": entries(regressed), "fixed": entries(fixed),
            "still_fail": entries(still_fail), "missing_from_b": entries(missing_b),
            "other_changes": entries(other),
            "gate": {"fail_crash_timeout_in_b_outside_accepted": len(bad_b),
                     "regressions_outside_accepted": len(reg_out),
                     "internal_or_resource_error_in_b": extra_b},
            "exit": rc,
        }
        print(json.dumps(doc, indent=2))
        return rc

    print("tree-diff: build %s" % describe())
    for name, S in (("A (baseline)", A), ("B (new run)", B)):
        print("%s: %d file(s), form %s" % (name, len(S["files"]), S["form"]))
        for f in S["files"]:
            print("    %s" % f)
    if mustpass is not None:
        print("scope: mustpass %s: %d cases, %d in scope, %d outside both sides' case filters "
              "(run by neither)" % (a.mustpass, len(mustpass), len(universe), outside))
    else:
        print("scope: the union of the cases seen in A and B: %d cases" % len(universe))

    cols = COUNT_COLS + [c for c in EXTRA_COLS if ca[c] or cb[c]]
    w = [max(6, len(c)) + 1 for c in cols]  # one space between columns, always
    print("counts:%s" % "".join("%*s" % (wd, c) for c, wd in zip(cols, w)))
    for tag, cnt in (("  A", ca), ("  B", cb)):
        print("%s%s" % (tag, "".join("%*d" % (wd, cnt[c]) for c, wd in zip(cols, w))))

    def entry(c, sa, sb):
        return "  %-20s -> %-20s %s%s" % (sa or "Missing", sb or "Missing", c,
                                          " [accepted]" if is_acc(c) else "")

    print("regressed (Pass in A -> anything else in B): %d" % len(regressed))
    for c, sa, sb in regressed:
        print(entry(c, sa, sb))
    print("fixed (not Pass in A -> Pass in B): %d" % len(fixed))
    for c, sa, sb in fixed:
        print(entry(c, sa, sb))
    print("still fail (Fail/Crash/Timeout/InternalError/ResourceError in A and in B): %d"
          % len(still_fail))
    for c, sa, sb in still_fail:
        print(entry(c, sa, sb))
    print("missing from B (a result in A, none in B): %d" % len(missing_b))
    for c, sa, sb in missing_b:
        print(entry(c, sa, sb))
    print("other status changes (a change that is none of the above): %d" % len(other))
    for c, sa, sb in other:
        print(entry(c, sa, sb))
    if accepted:
        print("accepted: %d case(s) in scope match %s (%d pattern(s))"
              % (acc_n, a.accepted, len(accepted)))
    if extra_b:
        print("note: %d InternalError/ResourceError in B (counted and listed, not gated)" % extra_b)
    if clean:
        print("verdict: PASS (exit 0): no Fail/Crash/Timeout in B outside the accepted list, "
              "no regression outside it")
    else:
        print("verdict: FAIL (exit 1): %d regression(s) outside the accepted list, %d "
              "Fail/Crash/Timeout in B outside it" % (len(reg_out), len(bad_b)))
    return rc


def self_test():
    """The fixture checks tools/cts/tree-diff-selftest.sh runs, in one process."""
    import subprocess
    fix = os.path.join(HERE, "testdata", "tree-diff")
    f = lambda name: os.path.join(fix, name)
    fails = 0

    def run(label, args, want, check=None):
        nonlocal fails
        p = subprocess.run([sys.executable, os.path.abspath(__file__)] + args,
                           capture_output=True, text=True)
        ok = p.returncode == want and (check is None or check(p))
        print("  %s  %s: rc=%d (want %d)" % ("ok  " if ok else "FAIL", label, p.returncode, want))
        fails += not ok

    run("console pair: regression, fix, missing",
        ["--baseline", f("console-a.txt"), "--new", f("console-b.txt"), "--mustpass", f("mustpass.txt")], 1)
    run("console pair + --accepted",
        ["--baseline", f("console-a.txt"), "--new", f("console-b.txt"), "--mustpass", f("mustpass.txt"),
         "--accepted", f("accepted.txt")], 1)
    run("narrow pair without --accepted",
        ["--baseline", f("console-a.txt"), "--new", f("console-b.txt"), "--mustpass", f("mustpass-acc.txt")], 1)
    run("narrow pair + --accepted: exit 0",
        ["--baseline", f("console-a.txt"), "--new", f("console-b.txt"), "--mustpass", f("mustpass-acc.txt"),
         "--accepted", f("accepted.txt")], 0)
    run("statuses pair: fixes only",
        ["--baseline", f("statuses-a.txt"), "--new", f("statuses-b.txt"), "--mustpass", f("mustpass-clean.txt")], 0)
    run("qpa pair: #terminateTestCaseResult Crash",
        ["--baseline", f("a.qpa"), "--new", f("b.qpa"), "--mustpass", f("mustpass-qpa.txt")], 1)
    run("directory input: *.qpa merged, last file wins",
        ["--baseline", f("a.qpa"), "--new", fix, "--mustpass", f("mustpass-qpa.txt")], 1)
    run("bad form", ["--baseline", f("bad-input.txt"), "--new", f("console-b.txt")], 2)
    run("missing file", ["--baseline", f("no-such-file.txt"), "--new", f("console-b.txt")], 2)

    def json_ok(p):
        try:
            d = json.loads(p.stdout)
            return (d["counts"]["A"]["Pass"] == 4 and d["counts"]["A"]["Missing"] == 0
                    and d["counts"]["B"]["Pass"] == 2 and d["counts"]["B"]["NotSupported"] == 2
                    and d["counts"]["B"]["Missing"] == 1 and len(d["regressed"]) == 3
                    and len(d["fixed"]) == 1 and len(d["missing_from_b"]) == 1
                    and d["gate"]["regressions_outside_accepted"] == 3 and d["exit"] == 1)
        except (ValueError, KeyError):
            return False

    run("--json counts, lists and gate",
        ["--baseline", f("console-a.txt"), "--new", f("console-b.txt"), "--mustpass", f("mustpass.txt"),
         "--json"], 1, json_ok)
    print("tree-diff --self-test: %d checks failed" % fails)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
