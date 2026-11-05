#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write the AM62P dEQP-GLES3 not-pass TSV (docs/es3/am62p-gles3-notpass.tsv).

  gles3-notpass.py [--commit SHA] [--board NAME] [--date RUN]
                   [--mustpass FILE] [-o OUT] SOURCE...
  gles3-notpass.py --self-test

A SOURCE is read whole, in the order given:
  <dir>       its cases.tsv, if present, then the *.qpa at its top, sorted.
              A directory named like a tree slice
              (<date>-am62p-gles3-<sha8>-rowsAA-BB/, tools/cts/log/) is
              checked as am62p-gles3-merge.py checks one (README.md 'commit:'
              line equal to the name) and its sha8 is the build, unless
              --commit names one. A directory from which no case could be
              read is an error, not an empty source.
  *.qpa       a dEQP log: <TestCaseResult CasePath="..."> blocks with a
              <Result StatusCode="...">, plus the console markers
              #beginTestCaseResult / #terminateTestCaseResult <Status> /
              #endTestCaseResult (the same reading as tools/cts/tree-diff.py:
              a block that closes without a Result — case cut off, board
              reset — records nothing, so the case is NoResult unless
              another source reported it).
  other file  per-case statuses, one per line: <case> TAB <status>
              [TAB <first message line>], the cases.tsv form of the tree's
              slices, e.g. the merged statuses of several slices. '#'
              comments. A status of NoResult is allowed here: it records
              the case as not having run.

A case twice with two different statuses (in one source or across two) is
an error, as in am62p-gles3-merge.py; twice with the same status keeps the
later line's message. Statuses are normalised with am62p-gles3-merge.py's
own norm() (imported from that file, not restated): CompatibilityWarning
counts as QualityWarning and any other non-Pass word (Crash, InternalError,
ResourceError, Timeout) counts as Fail, so the TSV holds exactly the
outcomes Fail, NotSupported, QualityWarning and NoResult — the same run
gives the same TSV whichever of the two tools wrote it.

What differs from am62p-gles3-merge.py: that tool is the tree's merge gate
— slice dirs only, README commit mandatory, totals and a --against delta.
This one is the writer for runs that are not slice dirs (the qpa logs of one
board session or of tools/cts/host-deqp-run.sh --gpu=pvr) and for a merged
status table, and its header names the run date, which merge's lacks.
--commit is mandatory unless a source directory names the build.

stdout (or -o OUT): the header naming the build, the board and the run date,
then every mustpass case (tools/cts/gles3-mustpass.txt) that is not Pass, in
mustpass order, tab separated: <outcome> <case> <first message line>. A
NoResult has an empty message and a Fail with no message shows the raw
status word, as in docs/es3/am62p-gles3-notpass.tsv. stderr: the totals.
Exit 1 on any refusal, including sources from which nothing was read.
"""
import argparse
import datetime
import glob
import os
import re
import subprocess
import sys
from importlib.util import module_from_spec, spec_from_file_location
from xml.sax.saxutils import unescape

HERE = os.path.dirname(os.path.abspath(__file__))
BOARD = "TI AM62P (Verdin, PowerVR)"
COUNTED = ("Pass", "Fail", "NotSupported", "QualityWarning", "NoResult")


class NotpassError(Exception):
    """A refused input; main turns it into exit 1."""


def _merge_tool():
    path = os.path.join(HERE, "am62p-gles3-merge.py")
    if not os.path.isfile(path):
        sys.exit("gles3-notpass: %s not found: statuses must be normalised exactly "
                 "as the tree's merge tool normalises them" % path)
    spec = spec_from_file_location("am62p_gles3_merge", path)
    mod = module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


MERGE = _merge_tool()


def _merge_refused(msg):
    # merge's slice_commit reports through module-level die(); turn that
    # into our own exception instead of letting it exit for us.
    raise NotpassError(msg)


MERGE.die = _merge_refused

QPA_OPEN = re.compile(r"<TestCaseResult\b[^>]*\bCasePath=\"([^\"]+)\"")
BEGIN = re.compile(r"^#beginTestCaseResult (\S+)")
END = re.compile(r"^#endTestCaseResult")
TERMINATE = re.compile(r"^#terminateTestCaseResult (\S+)")
CLOSE = re.compile(r"</TestCaseResult>")
RESULT = re.compile(r"<Result\b[^>]*\bStatusCode=\"([^\"]*)\"([^>]*)>")
RESULT_END = "</Result>"


def first_line(text):
    """The first non-blank line of a dEQP message, entities decoded (dEQP
    escapes quotes in messages as &quot; / &apos;, cts-refusals.py's
    lesson: a pattern that stops at & captures nothing; and saxutils
    unescapes only &amp; &lt; &gt; unless the quote entities are named)."""
    entities = {"&quot;": '"', "&apos;": "'"}
    for line in unescape(text or "", entities).splitlines():
        line = line.strip()
        if line:
            return line
    return ""


def record(into, case, status, msg, src):
    """One reported status. merge's rule: a case twice with different raw
    statuses is an error; the same status again keeps the later message."""
    old = into.get(case)
    if old is not None and old[0] != status:
        raise NotpassError("%s: %s is %s here and %s in %s" % (src, case, status, old[0], old[2]))
    into[case] = (status, msg, src)


def read_qpa(path, into):
    """The TestCaseResult blocks and #terminateTestCaseResult markers of one
    .qpa, the per-case reading of tools/cts/tree-diff.py, keeping the first
    message line too. Returns the number of records."""
    n = 0
    case = status = msg = None
    in_msg = False
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            if in_msg and (QPA_OPEN.search(line) or BEGIN.match(line) or TERMINATE.match(line)):
                in_msg = False   # the Result never closed: the case was cut off
            if in_msg:
                if RESULT_END in line:
                    msg += line.split(RESULT_END, 1)[0]
                    in_msg = False
                else:
                    msg += line
                continue
            o, b = QPA_OPEN.search(line), BEGIN.match(line)
            if o or b:
                name = (o or b).group(1)
                if name != case:      # "#beginTestCaseResult X" precedes X's own block
                    case, status, msg = name, None, None   # an unclosed block is no result
            m = RESULT.search(line)
            if m and case is not None:
                status = m.group(1)
                tail = line[m.end():]
                if m.group(2).rstrip().endswith("/"):
                    msg = None                       # <Result ... />: a status, no message
                elif RESULT_END in tail:
                    msg = tail.split(RESULT_END, 1)[0]
                else:
                    msg, in_msg = "", True
            t = TERMINATE.match(line)
            if t:
                if case is None:
                    raise NotpassError("%s: '#terminateTestCaseResult' with no case open" % path)
                record(into, case, t.group(1), first_line(msg), path)
                n += 1
                case = status = msg = None
            elif CLOSE.search(line) or END.match(line):
                if case is not None and status is not None:
                    record(into, case, status, first_line(msg), path)
                    n += 1
                if END.match(line):
                    case = status = msg = None
    return n


def read_statuses(path, into):
    """<case> TAB <status> [TAB <message>] lines, the cases.tsv form of the
    tree's slices (merge's read_cases, our own refusal text)."""
    n = 0
    with open(path, encoding="utf-8", errors="replace") as f:
        for k, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            p = line.split("\t")
            if len(p) < 2 or len(p) > 3 or not p[0] or not p[1]:
                raise NotpassError("%s:%d: not <case> TAB <status> [TAB <message>]: %r"
                                   % (path, k, line))
            if p[1].startswith("dEQP-") and p[0] in COUNTED:
                raise NotpassError("%s:%d: outcome first? this reads <case> TAB <status>, "
                                   "the form of the slices, not the not-pass TSV" % (path, k))
            record(into, p[0], p[1], p[2] if len(p) == 3 else "", "%s:%d" % (path, k))
            n += 1
    return n


def read_dir(d, into):
    """cases.tsv of a tree slice (or of any run that keeps one), plus the
    *.qpa at the top of the directory. Nothing readable is an error: an
    empty result here would silently print a TSV of only NoResult."""
    n = 0
    cases = os.path.join(d, "cases.tsv")
    if os.path.isfile(cases):
        n += read_statuses(cases, into)
    for f in sorted(glob.glob(os.path.join(d, "*.qpa"))):
        n += read_qpa(f, into)
    if n == 0:
        raise NotpassError("%s: neither cases.tsv nor *.qpa recorded a case" % d)
    return n


def collect(paths, into):
    """Read every source into into ({}); return the set of commits the
    source directories named."""
    commits = set()
    for p in paths:
        if os.path.isdir(p):
            if MERGE.SLICE.match(os.path.basename(os.path.normpath(p))):
                commits.add(MERGE.slice_commit(p))
            read_dir(p, into)
        elif os.path.isfile(p):
            if p.endswith(".qpa"):
                read_qpa(p, into)
            else:
                read_statuses(p, into)
        else:
            raise NotpassError("no such file or directory: %s" % p)
    return commits


def resolve_commit(given, commits):
    if given:
        if commits and given not in commits:
            raise NotpassError("--commit %s, but the slices name %s"
                               % (given, ", ".join(sorted(commits))))
        return given
    if len(commits) > 1:
        raise NotpassError("the sources name different commits: %s" % ", ".join(sorted(commits)))
    if not commits:
        raise NotpassError("--commit is required: no source names the build")
    return commits.pop()


def read_mustpass(path):
    with open(path, encoding="utf-8") as f:
        return [l.strip() for l in f if l.strip() and not l.startswith("#")]


def render(results, mustpass, commit, board, date):
    """The TSV text and its outcome counts (merge's rules for the raw
    status: norm() to the outcome, and a Fail with no message shows the
    raw status word)."""
    count = dict.fromkeys(COUNTED, 0)
    out = ["# SPDX-License-Identifier: Apache-2.0\n"
           "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n"
           "# dEQP-GLES3 on the %s, build %s, run %s: every mustpass case\n"
           "# (tools/cts/gles3-mustpass.txt) whose result is not Pass. One case per line, tab separated:\n"
           "# <outcome> <case> <first message line>. NoResult: the case never ran\n"
           "# (no status in any source).\n" % (board, commit, date)]
    for case in mustpass:
        s = results.get(case)
        if s is None:
            outcome, msg = "NoResult", ""
        else:
            raw = s[0]
            outcome = "NoResult" if raw == "NoResult" else MERGE.norm(raw)
            msg = "" if outcome == "NoResult" else (s[1] or (raw if outcome == "Fail" else ""))
        count[outcome] += 1
        if outcome != "Pass":
            out.append("%s\t%s\t%s\n" % (outcome, case, msg))
    return "".join(out), count


def self_test():
    d = os.path.join(HERE, "testdata", "gles3-notpass")
    fails = []

    def check(name, cond, detail=""):
        print("  ok    %s" % name if cond else "  FAIL  %s %s" % (name, detail))
        if not cond:
            fails.append(name)

    def refuses(name, paths, word):
        try:
            collect(paths, {})
            check(name, False, "(no error)")
        except NotpassError as e:
            check(name, word.lower() in str(e).lower(), "(%s lacks %r)" % (e, word))

    mustpass = read_mustpass(os.path.join(d, "mustpass.txt"))
    results = {}
    logs = os.path.join(d, "logs")
    merged = os.path.join(d, "merged.tsv")
    slices = os.path.join(d, "slices", "2026-10-27-am62p-gles3-abcdef12-rows01-02")
    commits = collect([logs, merged, slices], results)
    check("the slice directory names the build", commits == {"abcdef12"}, "(%s)" % commits)

    text, count = render(results, mustpass, "abcdef12", BOARD, "2026-10-27")
    with open(os.path.join(d, "expected.tsv"), encoding="utf-8") as f:
        want = f.read()
    check("the TSV matches testdata/gles3-notpass/expected.tsv byte for byte",
          text == want, "(differs)")
    if text != want:
        import difflib
        sys.stderr.write("".join(difflib.unified_diff(
            want.splitlines(True), text.splitlines(True), "expected", "got")))
    got = {r.split("\t")[1]: r.split("\t")[0] for r in text.splitlines() if r and not r.startswith("#")}
    check("only the four outcomes of the TSV appear",
          set(got.values()) <= {"Fail", "NotSupported", "QualityWarning", "NoResult"}, "(%s)" % got)
    check("CompatibilityWarning became QualityWarning, Crash-like statuses Fail",
          got.get("dEQP-GLES3.b.compat") == "QualityWarning"
          and got.get("dEQP-GLES3.b.timeout") == "Fail"
          and got.get("dEQP-GLES3.a.terminated") == "Fail", "(%s)" % got)
    check("a case cut off (no Result) is NoResult, an unreported one too",
          got.get("dEQP-GLES3.a.cut") == "NoResult" and got.get("dEQP-GLES3.b.never") == "NoResult",
          "(%s)" % got)
    check("Pass cases are absent and the totals sum to the mustpass list",
          "dEQP-GLES3.a.pass" not in got and "dEQP-GLES3.b.merged_pass" not in got
          and sum(count.values()) == len(mustpass), "(%s)" % count)
    check("a case in two sources with the same status keeps the later message",
          [l for l in text.splitlines() if "dEQP-GLES3.a.fail" in l][0].endswith("\tretried: mismatch at row 3"))
    check("the #terminateTestCaseResult status won, its raw word is the message",
          [l for l in text.splitlines() if "dEQP-GLES3.a.terminated" in l][0] ==
          "Fail\tdEQP-GLES3.a.terminated\tInternalError")
    check("cases outside the mustpass list are ignored",
          "dEQP-GLES3.not.in.mustpass" not in got)

    bad = os.path.join(d, "bad")
    refuses("a case with two different statuses across sources is refused",
            [logs, merged, slices, os.path.join(bad, "conflict",
                                                "2026-10-27-am62p-gles3-abcdef12-rows08-08")],
            "dEQP-GLES3.a.pass")
    refuses("a directory with no cases.tsv and no qpa is refused",
            [os.path.join(bad, "empty-dir")], "neither cases.tsv nor *.qpa")
    refuses("a slice whose README names another commit is refused",
            [os.path.join(bad, "commit", "2026-10-27-am62p-gles3-abcdef12-rows09-09")], "deadbeef")
    refuses("sources from which nothing at all was read are refused",
            [os.path.join(bad, "noresults.tsv"), os.path.join(bad, "norecords",
                                                              "2026-10-27-am62p-gles3-abcdef12-rows07-07")],
            "recorded a case")
    def run_cli(*args):
        r = subprocess.run([sys.executable, "-I", os.path.abspath(__file__)] + list(args),
                           capture_output=True, text=True)
        return r.returncode, r.stdout, r.stderr

    rc, out, err = run_cli(logs, merged, slices, "--mustpass", os.path.join(d, "mustpass.txt"),
                           "--date", "2026-10-27")
    check("the CLI names the build from the slice and prints the TSV",
          rc == 0 and out == want, "(rc %d, %s)" % (rc, err[:200]))
    rc, _, err = run_cli(os.path.join(d, "bad", "noresults.tsv"), "--commit", "abcdef12")
    check("the CLI refuses a source from which nothing at all was read",
          rc == 1 and "no per-case result" in err, "(rc %d, %s)" % (rc, err[:200]))
    rc, _, err = run_cli(logs, merged, slices, "--commit", "deadbeef", "--date", "2026-10-27")
    check("the CLI refuses --commit disagreeing with the slices",
          rc == 1 and "deadbeef" in err, "(rc %d, %s)" % (rc, err[:200]))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("sources", nargs="*", metavar="SOURCE")
    ap.add_argument("--commit", help="the build the images were made from")
    ap.add_argument("--board", default=BOARD)
    ap.add_argument("--date", default=datetime.date.today().isoformat(), metavar="RUN")
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("-o", "--output", metavar="OUT")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.sources:
        ap.error("at least one SOURCE is required")
    results = {}
    try:
        commits = collect(a.sources, results)
        commit = resolve_commit(a.commit, commits)
        if not results:
            raise NotpassError("no per-case result read from any source")
        mustpass = read_mustpass(a.mustpass)
        text, count = render(results, mustpass, commit, a.board, a.date)
    except NotpassError as e:
        sys.stderr.write("gles3-notpass: %s\n" % e)
        return 1
    extra = sorted(set(results) - set(mustpass))
    if extra:
        sys.stderr.write("gles3-notpass: %d result(s) outside the mustpass list ignored"
                         " (first: %s)\n" % (len(extra), extra[0]))
    if a.output:
        with open(a.output, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    sys.stderr.write("commit %s, %d case(s) with a status\n" % (commit, len(results)))
    sys.stderr.write("Pass %d  Fail %d  NotSupported %d  QualityWarning %d  no result %d"
                     "  = %d (mustpass %d)\n" % (count["Pass"], count["Fail"], count["NotSupported"],
                                                 count["QualityWarning"], count["NoResult"],
                                                 len(mustpass), len(mustpass)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
