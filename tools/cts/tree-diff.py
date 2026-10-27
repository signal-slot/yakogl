#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-case diff of two dEQP-GLES2 result sets against the mustpass list.

    tools/cts/tree-diff.py --baseline A [A...] --new B [B...]
                           [--mustpass tools/cts/gles2-mustpass.txt]
                           [--accepted tools/cts/es2-accepted.txt] [--json]

A is the baseline (an earlier run), B the new run. Each side is one or more
result files, or directories holding them: a board.sh tree or run directory
(its `*/console.txt` and `console.txt`) or a host-deqp-run.sh output directory
(its `*.qpa`). The tool answers, case by case: what passed in A and does not
pass in B (regressed), what passes in B and did not in A (fixed), what fails
in both (still fail), and what has a result in A and none in B (missing from
B). A case that passed in A and has no result in B is in both "regressed" and
"missing from B"; every other case is in at most one list. Changes that are
none of these (NotSupported -> QualityWarning, Missing -> Fail, ...) are
listed as "other status changes".

Input forms. These are the per-case forms this repository's runners write.
The tool works the form out from each file's content (not its name) and
names it in its output:

  deqp-console  the deqp sample's console lines "deqp: <Status> <case>"
                (driver/samples/zephyr/deqp), as tools/board.sh captures them
                into a run's console.txt: the AM62P and Pi 4 trees
                (am62p-deqp-images.sh, pi4-deqp-images.sh) and the Pi 4
                recorder's console.ts. NULs, CRs and a leading time stamp
                ("HH:MM:SS " or epoch "1791625635.017 ", the Pi 4 recorder's
                two forms) are dropped first, as in deqp-uart-tally.py.
                "deqp: case filter '<patterns>'" gives the file's scope.
                Whether a log is MISSING / INCOMPLETE / FATAL / lost lines in
                capture is deqp-uart-tally.py's verdict, not this tool's: this
                one compares the case lines that arrived. A console of an
                image built with CONFIG_DEQP_ECHO_QPA carries both these lines
                and "qpa: " lines; the "deqp:" lines are read.

  qpa           dEQP's TestResults.qpa: "<TestCaseResult CasePath=...>"
                blocks whose "<Result StatusCode=...>" is the status
                (host-deqp-run.sh's c_NNN.qpa and c_NNN.rK.qpa,
                qpa-from-uart.py's rebuild of an echo-qpa console, the
                fixtures in tools/cts/testdata/). The batch markers
                "#beginTestCaseResult <case>" / "#endTestCaseResult" are
                followed, and "#terminateTestCaseResult <Status>" (dEQP's
                crash handler, cts_resume.py --repair) closes the open case
                with that status. A case opened and never closed has no
                result, as for the CTS's own BatchResultParser. A .qpa has no
                case filter: its scope is the whole mustpass.

  statuses      "<Status> <case>" lines, as deqp-uart-tally.py --statuses
                writes them (tools/board.sh diff builds its baseline from
                them) and as the board-session records quote their "not
                Pass, by name" lists. NoResult (the case never began) and
                Lost (the image ran it, the capture lost its line) are no
                result; Lost cases are counted separately. Every name in the
                file, NoResult and Lost included, is the file's scope.

Not accepted, exit 2 with a message saying so: the selector form of the
i.MX8MP tree (tools/board.sh results_tsv: results.tsv, rows.md and the
"TALLY (N entries): begun ... passed ..." line). It is per row, not per case:
the NotSupported cases are counted and not named, and the named ones do not
say which non-Pass status each has. tools/board.sh diff compares two of them
row by row.

Statuses: Pass, Fail, NotSupported, QualityWarning, CompatibilityWarning,
InternalError, ResourceError, Crash, Timeout (the deqp sample's nine). A case
with no result on a side is Missing there. A console or statuses line with
another word is not a result line (a serial capture splices lines; as in
deqp-uart-tally.py it is skipped); an unknown StatusCode or
#terminateTestCaseResult status in a .qpa is bad input. The last result for a name wins
(files in the order given, directories in sorted order, lines in file order),
as in deqp-uart-tally.py.

Scope. With --mustpass the universe is the mustpass cases inside either
side's scope; a side's scope is the union of its files' scopes (a file
without scope information makes it the whole mustpass), as in
deqp-uart-tally.py. Mustpass cases outside both scopes were run by neither
side: counted, left out. A result for a case not in the mustpass is ignored
and counted. Without --mustpass the universe is the union of the cases with
a result in A or B.

--accepted FILE: one case name or glob per line ('*' and '?' as in fnmatch;
'#' starts a comment, on its own line or after the pattern). A matching case
is still listed, marked [accepted], but neither a regression nor a Fail,
Crash or Timeout of it counts against the exit status.
tools/cts/es2-accepted.txt holds the cases docs/es2-cts-failures.md section C
records as acceptable NotSupported.

Exit status:
  0  B has no Fail, Crash or Timeout outside the accepted list, and no case
     outside it regressed (InternalError and ResourceError are listed under
     "still fail" or "regressed" but do not change the status on their own)
  1  otherwise
  2  bad input: a missing or unreadable file, a directory without result
     files, a file in no per-case form (or in the per-row results.tsv form),
     an unknown status in a .qpa, no case with a result on a side, an empty
     --mustpass or an unreadable --accepted; argparse's usage errors are 2

--json prints the same result as one JSON object instead of the report
(keys: tool, build, a, b, scope, accepted, counts, regressed, fixed,
still_fail, missing_from_b, other_changes, ignored_not_in_mustpass, gate,
exit), with the same exit status.

The fixtures in tools/cts/testdata/tree-diff/ and tools/cts/tree-diff-selftest.sh
check these rules.
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
NO_RESULT = ("NoResult", "Lost")                 # statuses form only: no result
COUNT_COLS = ["Pass", "Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
              "Crash", "Timeout", "Missing"]
EXTRA_COLS = ["InternalError", "ResourceError"]  # shown when nonzero, never gated alone
GATE_BAD = ("Fail", "Crash", "Timeout")          # what the exit status gates on in B
LIST_BAD = GATE_BAD + tuple(EXTRA_COLS)          # "still fail"

STAMP = re.compile(r"^(?:\d{2}:\d{2}:\d{2}|\d{9,11}\.\d+) ")
_ALT = "|".join(STATUSES)
CONSOLE = re.compile(r"^deqp: (%s) (dEQP-\S+)" % _ALT)
FILTER = re.compile(r"^deqp: case filter '([^']*)'")
STATUS_LINE = re.compile(r"^(%s|%s) (dEQP-\S+)\s*$" % (_ALT, "|".join(NO_RESULT)))
QPA_OPEN = re.compile(r"<TestCaseResult\b[^>]*\bCasePath=\"([^\"]+)\"")
QPA_RESULT = re.compile(r"<Result\b[^>]*\bStatusCode=\"([^\"]+)\"")
QPA_CLOSE = re.compile(r"</TestCaseResult>")
BEGIN = re.compile(r"^#beginTestCaseResult (\S+)")
END = re.compile(r"^#endTestCaseResult")
TERMINATE = re.compile(r"^#terminateTestCaseResult (\S+)")
TALLY = re.compile(r"^TALLY \(\d+ entries\): begun ")
TSV_HEAD = re.compile(r"^# index\tfilter\tbegun\tpassed\t")


class InputError(Exception):
    pass


def describe():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


def filter_regex(patterns):
    """dEQP case patterns ('*' matches anything, dots too; the rest literally)
    as one regex, as in deqp-uart-tally.py."""
    alts = ["".join(".*" if ch == "*" else re.escape(ch) for ch in p) for p in patterns if p]
    return re.compile("^(?:%s)$" % "|".join(alts)) if alts else None


def detect_form(lines, path):
    if any(TSV_HEAD.match(l) or TALLY.match(l) for l in lines):
        raise InputError("%s: the selector form (results.tsv / TALLY line) is per row, not per "
                         "case: it does not name the NotSupported cases nor say which status a "
                         "failing name has; compare two of them with tools/board.sh diff" % path)
    if any(CONSOLE.match(l) for l in lines):
        return "deqp-console"
    if any(QPA_OPEN.search(l) or BEGIN.match(l) or TERMINATE.match(l) for l in lines):
        return "qpa"
    if any(STATUS_LINE.match(l) for l in lines):
        return "statuses"
    raise InputError("%s: no per-case form found (expected 'deqp: <Status> <case>' console "
                     "lines, .qpa '<TestCaseResult CasePath=...>' blocks or '#beginTestCaseResult' "
                     "markers, or '<Status> <case>' statuses lines)" % path)


def known(status, path, case):
    if status not in STATUSES:
        raise InputError("%s: unknown status %r on %s" % (path, status, case))
    return status


def parse_cases(form, lines, path):
    """({case: status}, scope, {lost case}) of one file. scope is None (the
    whole mustpass), or (patterns, names)."""
    got, lost = {}, set()
    if form == "deqp-console":
        pats = []
        for l in lines:
            m = CONSOLE.match(l)
            if m:
                got[m.group(2)] = known(m.group(1), path, m.group(2))
                continue
            f = FILTER.match(l)
            if f:
                pats.extend(f.group(1).split(","))
        return got, ((pats, set()) if pats else None), lost
    if form == "statuses":
        names = set()
        for l in lines:
            m = STATUS_LINE.match(l)
            if not m:
                continue
            s, c = m.group(1), m.group(2)
            names.add(c)
            if s in NO_RESULT:
                got.pop(c, None)
                if s == "Lost":
                    lost.add(c)
            else:
                got[c] = known(s, path, c)
                lost.discard(c)
        return got, ([], names), lost
    case, status = None, None  # qpa
    for l in lines:
        o, b = QPA_OPEN.search(l), BEGIN.match(l)
        if o or b:
            name = (o or b).group(1)
            if name != case:     # "#beginTestCaseResult X" is followed by X's own block
                case, status = name, None  # an unclosed block is no result
        m = QPA_RESULT.search(l)
        if m and case is not None:
            status = known(m.group(1), path, case)
        t = TERMINATE.match(l)
        if t:
            if case is None:
                raise InputError("%s: '#terminateTestCaseResult' with no case open" % path)
            got[case] = known(t.group(1), path, case)
            case, status = None, None
        elif QPA_CLOSE.search(l) or END.match(l):
            if case is not None and status is not None:
                got[case] = status
            if END.match(l):
                case, status = None, None
    return got, None, lost


def result_files(d):
    """The result files of a directory: a run's console.txt, a tree's
    */console.txt, a host run's *.qpa."""
    found = [os.path.join(d, "console.txt")] if os.path.isfile(os.path.join(d, "console.txt")) else []
    found += sorted(glob.glob(os.path.join(d, "*", "console.txt")))
    found += sorted(glob.glob(os.path.join(d, "*.qpa")))
    return found


def load_side(paths, side):
    files = []
    for p in paths:
        if os.path.isdir(p):
            found = result_files(p)
            if not found:
                raise InputError("%s: %s holds no console.txt, */console.txt or *.qpa" % (side, p))
            files.extend(found)
        elif os.path.isfile(p):
            files.append(p)
        else:
            raise InputError("%s: no such file or directory: %s" % (side, p))
    status, lost, pats, names, whole, forms = {}, set(), [], set(), False, []
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
        got, scope, lst = parse_cases(form, lines, path)
        for c in got:
            lost.discard(c)
        status.update(got)
        lost |= {c for c in lst if c not in status}
        if scope is None:
            whole = True
        else:
            pats.extend(scope[0])
            names |= scope[1]
    if not status:
        raise InputError("%s: no case with a result in %s" % (side, ", ".join(files)))
    rx = filter_regex(pats)
    in_scope = None if whole else (lambda c: c in names or bool(rx and rx.match(c)))
    return {"files": files, "form": "+".join(forms), "status": status, "lost": lost,
            "in_scope": in_scope, "patterns": pats}


def load_mustpass(path):
    try:
        cases = [l.strip() for l in open(path, encoding="utf-8")
                 if l.strip() and not l.lstrip().startswith("#")]
    except (OSError, UnicodeDecodeError) as e:
        raise InputError("cannot read --mustpass %s: %s" % (path, e))
    if not cases:
        raise InputError("--mustpass %s names no case" % path)
    return cases


def load_accepted(path):
    """[pattern] of an accepted list. A file of comments only is an empty list."""
    try:
        text = open(path, encoding="utf-8").read()
    except (OSError, UnicodeDecodeError) as e:
        raise InputError("cannot read --accepted %s: %s" % (path, e))
    pats = []
    for l in text.splitlines():
        l = l.split("#", 1)[0].strip()
        if l:
            pats.append(l)
    return pats


def main(argv):
    ap = argparse.ArgumentParser(
        prog="tree-diff.py",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description="Per-case diff of two dEQP-GLES2 result sets: A (--baseline) against\n"
                    "B (--new), over the mustpass list. Prints counts by status for each\n"
                    "side, then the cases that regressed, got fixed, still fail and are\n"
                    "missing from B.",
        epilog="""inputs (the form is worked out from each file's content and named in the output):
  deqp-console  'deqp: <Status> <case>' lines of a board console (board.sh's
                console.txt; HH:MM:SS or epoch time stamps dropped); its
                'deqp: case filter' line is the file's scope
  qpa           dEQP TestResults.qpa (<TestCaseResult>/<Result StatusCode>,
                #beginTestCaseResult, #terminateTestCaseResult <Status>)
  statuses      '<Status> <case>' lines (deqp-uart-tally.py --statuses;
                NoResult and Lost are no result)
  A directory stands for its console.txt, */console.txt and *.qpa.
  Not accepted: the i.MX8MP selector form (results.tsv, TALLY line): per row,
  not per case.

exit status:
  0  B has no Fail, Crash or Timeout outside the accepted list and no
     regression (Pass in A, anything else in B) outside it
  1  otherwise
  2  bad input (missing/unreadable file, no per-case form or the per-row
     results.tsv form, unknown .qpa status, no case with a result, empty
     --mustpass, unreadable --accepted, a usage error)

The module docstring (tools/cts/tree-diff.py) gives the rules in full.""")
    ap.add_argument("--baseline", nargs="+", metavar="PATH", required=True,
                    help="A, the baseline: result files or directories")
    ap.add_argument("--new", nargs="+", metavar="PATH", required=True,
                    help="B, the new run: result files or directories")
    ap.add_argument("--mustpass", metavar="FILE",
                    help="the mustpass list (tools/cts/gles2-mustpass.txt, one case per line): "
                         "the universe, narrowed to the cases in either side's scope; without "
                         "it the universe is the union of the cases in A and B")
    ap.add_argument("--accepted", metavar="FILE",
                    help="accepted non-Pass outcomes (tools/cts/es2-accepted.txt): one case "
                         "name or glob per line, '#' starts the reason; matching cases are "
                         "listed, marked [accepted], and do not count against the exit status")
    ap.add_argument("--json", action="store_true",
                    help="print the result as one JSON object instead of the report")
    a = ap.parse_args(argv)

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

    if mustpass is not None:
        def scoped(S, c):
            return S["in_scope"] is None or S["in_scope"](c)
        universe = [c for c in mustpass if scoped(A, c) or scoped(B, c)]
        mp = set(mustpass)
        ignored = sorted((set(A["status"]) | set(B["status"])) - mp)
        scope = {"mode": "mustpass", "path": a.mustpass, "total": len(mustpass),
                 "in_scope": len(universe), "outside_both_scopes": len(mustpass) - len(universe)}
    else:
        universe = sorted(set(A["status"]) | set(B["status"]))
        ignored = []
        scope = {"mode": "union", "path": None, "total": None, "in_scope": len(universe),
                 "outside_both_scopes": 0}

    def counts(S):
        c = dict.fromkeys(COUNT_COLS + EXTRA_COLS, 0)
        for case in universe:
            c[S["status"].get(case, "Missing")] += 1
        c["Lost"] = sum(1 for case in universe if case in S["lost"])  # within Missing
        return c

    ca, cb = counts(A), counts(B)
    regressed, fixed, still_fail, missing_b, other = [], [], [], [], []
    for c in universe:
        sa, sb = A["status"].get(c), B["status"].get(c)
        e = (c, sa or "Missing", sb or "Missing")
        if sa is not None and sb is None:
            missing_b.append(e)
        if sa == "Pass" and sb != "Pass":
            regressed.append(e)
        elif sa != "Pass" and sb == "Pass":
            fixed.append(e)
        elif sa in LIST_BAD and sb in LIST_BAD:
            still_fail.append(e)
        elif sa != sb and sb is not None:
            other.append(e)

    bad_b = [c for c in universe if B["status"].get(c) in GATE_BAD and not is_acc(c)]
    reg_out = [e for e in regressed if not is_acc(e[0])]
    acc_n = sum(1 for c in universe if is_acc(c))
    rc = 0 if not bad_b and not reg_out else 1

    if a.json:
        def entries(lst):
            return [{"case": c, "a": sa, "b": sb, "accepted": is_acc(c)} for c, sa, sb in lst]
        print(json.dumps({
            "tool": "tree-diff", "build": describe(),
            "a": {"files": A["files"], "form": A["form"]},
            "b": {"files": B["files"], "form": B["form"]},
            "scope": scope,
            "accepted": {"path": a.accepted, "patterns": len(accepted), "matching_cases": acc_n},
            "counts": {"A": ca, "B": cb},
            "regressed": entries(regressed), "fixed": entries(fixed),
            "still_fail": entries(still_fail), "missing_from_b": entries(missing_b),
            "other_changes": entries(other),
            "ignored_not_in_mustpass": len(ignored),
            "gate": {"fail_crash_timeout_in_b_outside_accepted": len(bad_b),
                     "regressions_outside_accepted": len(reg_out)},
            "exit": rc,
        }, indent=2))
        return rc

    print("tree-diff: build %s" % describe())
    for name, S in (("A (baseline)", A), ("B (new run)", B)):
        print("%s: %d file(s), form %s" % (name, len(S["files"]), S["form"]))
        for f in S["files"]:
            print("    %s" % f)
    if mustpass is not None:
        print("scope: mustpass %s: %d cases, %d in A's or B's scope, %d outside both (run by "
              "neither, left out)" % (a.mustpass, len(mustpass), len(universe),
                                      scope["outside_both_scopes"]))
        if ignored:
            print("scope: %d case(s) with a result are not in the mustpass, ignored (first: %s)"
                  % (len(ignored), ignored[0]))
    else:
        print("scope: no --mustpass, the union of the cases in A and B: %d cases" % len(universe))

    cols = COUNT_COLS + [c for c in EXTRA_COLS if ca[c] or cb[c]]
    w = [len(c) + 1 for c in cols]
    print("counts  %s" % "".join("%*s" % (wd, c) for c, wd in zip(cols, w)))
    for tag, cnt in (("A", ca), ("B", cb)):
        print("  %-5s %s%s" % (tag, "".join("%*d" % (wd, cnt[c]) for c, wd in zip(cols, w)),
                               "   (%d of Missing lost in capture)" % cnt["Lost"] if cnt["Lost"] else ""))

    def section(title, lst):
        print("%s: %d" % (title, len(lst)))
        for c, sa, sb in lst:
            print("  %-20s -> %-20s %s%s" % (sa, sb, c, " [accepted]" if is_acc(c) else ""))

    section("regressed (Pass in A, not Pass in B)", regressed)
    section("fixed (not Pass in A, Pass in B)", fixed)
    section("still fail (Fail/Crash/Timeout/InternalError/ResourceError in A and B)", still_fail)
    section("missing from B (a result in A, none in B)", missing_b)
    section("other status changes", other)
    if a.accepted:
        print("accepted: %d case(s) in scope match %s (%d pattern(s))"
              % (acc_n, a.accepted, len(accepted)))
    if rc == 0:
        print("verdict: PASS (exit 0): no Fail/Crash/Timeout in B and no regression, outside "
              "the accepted list")
    else:
        print("verdict: FAIL (exit 1): %d Fail/Crash/Timeout in B and %d regression(s), "
              "outside the accepted list" % (len(bad_b), len(reg_out)))
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
