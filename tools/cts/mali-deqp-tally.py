#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cover, tally and doc checks over the NanoPC-T6's dEQP-GLES2 slices.

A slice is one directory tools/cts/log/<YYYY-MM-DD>-t6-gles2-<session>-<sha8>/
(session S1..S4 or smoke) holding

  meta.txt      key=value: commit (40 hex), session, rows (<a>-<b>), tree,
                boot_id_before, boot_id_after, kernel, lib, started, ended
  chunks.tsv    row label cases status seconds kmsg resets ssh_after boot_same
                exit_line   (status done|crash|hang|fault|lost; ssh_after
                ok|lost; boot_same yes|no; exit_line is the library's
                yakogl-mali-exit line or -)
  results.tsv   case status detail   (status: a dEQP status, or Lost with a
                reason in detail: chunk-crash, chunk-hang, board-lost, not-run)
  kmsg/<label>.txt   the redacted kernel lines of that chunk

Lines starting with '#' are comments in all four (the fixtures carry the SPDX
header that way); the header line of a .tsv is the first line without one.
results.tsv lists the chunks' cases in the chunks' order, so chunk i owns the
next <cases> rows: that is how a chunk's exit line reaches its cases.

    mali-deqp-tally.py --check-cover MUSTPASS SLICE... [--no-git]
    mali-deqp-tally.py --tally SLICE... [--markdown]
    mali-deqp-tally.py --check-doc DOC SLICE...
    mali-deqp-tally.py --self-test

--check-cover exits 0 only when every mustpass case appears exactly once over
all slices with a valid status, every chunks.tsv row is complete and has its
kmsg file, and every meta commit is an ancestor of origin/main
(git merge-base --is-ancestor; --no-git skips that, for fixtures).
--tally prints the counts by status and the non-Pass counts by cause. A cause
is the detail with numbers, hex and case-specific names stripped;
`refused:<name>` when the detail or the chunk's exit line names a back-end
refusal ("refused ... : Name(...)" or "refused NAME"); `lost:<reason>` for
Lost. NotSupported is a status, not a cause row.
--markdown prints the block that docs/es2-cts-failures.md keeps between
<!-- mali-deqp-tally begin --> and <!-- mali-deqp-tally end -->;
--check-doc exits 0 only when that block equals it.
"""
import argparse
import collections
import os
import re
import subprocess
import sys
import tempfile

BEGIN = "<!-- mali-deqp-tally begin -->"
END = "<!-- mali-deqp-tally end -->"
STATUSES = ("Pass", "Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
            "InternalError", "ResourceError", "Crash", "Timeout", "Lost")
CHUNK_STATUS = ("done", "crash", "hang", "fault", "lost")
META_KEYS = ("commit", "session", "rows", "tree", "boot_id_before", "boot_id_after", "kernel", "lib",
             "started", "ended")
CHUNK_COLS = ("row", "label", "cases", "status", "seconds", "kmsg", "resets", "ssh_after", "boot_same",
              "exit_line")
RESULT_COLS = ("case", "status", "detail")
SLICE_NAME = re.compile(r"^\d{4}-\d{2}-\d{2}-t6-gles2-(S[1-4]|smoke)-[0-9a-f]{8}$")
REFUSED_NAMED = re.compile(r"refused\s+([A-Z][A-Z0-9_]+)\b")
REFUSED_AFTER = re.compile(r"refus\w*[^:\n]*:\s*([A-Za-z_]\w*)")


def rows_of(path):
    """(header, [fields]) of a tsv, '#' lines skipped; ([], []) when absent."""
    if not os.path.isfile(path):
        return [], []
    lines = [l.rstrip("\n") for l in open(path, encoding="utf-8", errors="replace")
             if l.strip() and not l.startswith("#")]
    if not lines:
        return [], []
    return lines[0].split("\t"), [l.split("\t") for l in lines[1:]]


def load(path):
    """One slice as a dict; "problems" lists what is wrong with its files."""
    s = {"path": path, "name": os.path.basename(os.path.normpath(path)), "meta": {}, "chunks": [],
         "results": [], "problems": []}
    bad = s["problems"].append
    if not SLICE_NAME.match(s["name"]):
        bad("directory name is not <date>-t6-gles2-<S1..S4|smoke>-<sha8>")
    mp = os.path.join(path, "meta.txt")
    if os.path.isfile(mp):
        for l in open(mp, encoding="utf-8", errors="replace"):
            if l.strip() and not l.startswith("#") and "=" in l:
                k, v = l.rstrip("\n").split("=", 1)
                s["meta"][k.strip()] = v.strip()
    else:
        bad("no meta.txt")
    for k in META_KEYS:
        if not s["meta"].get(k):
            bad("meta.txt lacks %s" % k)
    m = s["meta"]
    if m.get("commit") and not re.fullmatch(r"[0-9a-f]{40}", m["commit"]):
        bad("meta commit is not 40 hex")
    if m.get("commit") and SLICE_NAME.match(s["name"]) and not m["commit"].startswith(s["name"][-8:]):
        bad("directory sha8 is not the meta commit's")
    if m.get("session") and SLICE_NAME.match(s["name"]) and m["session"] != s["name"].split("-")[5]:
        bad("meta session differs from the directory name")
    if m.get("rows") and not re.fullmatch(r"\d+-\d+", m["rows"]):
        bad("meta rows is not <a>-<b>")

    head, rows = rows_of(os.path.join(path, "chunks.tsv"))
    if head != list(CHUNK_COLS):
        bad("chunks.tsv header is not %s" % " ".join(CHUNK_COLS))
    else:
        for i, f in enumerate(rows, 1):
            if len(f) != len(CHUNK_COLS) or not all(x != "" for x in f):
                bad("chunks.tsv row %d: %d fields, empty ones not allowed" % (i, len(f)))
                continue
            c = dict(zip(CHUNK_COLS, f))
            if not (c["cases"].isdigit() and c["kmsg"].isdigit() and c["resets"].isdigit()):
                bad("chunks.tsv %s: cases, kmsg and resets must be counts" % c["label"])
            if c["status"] not in CHUNK_STATUS:
                bad("chunks.tsv %s: status %r" % (c["label"], c["status"]))
            if c["ssh_after"] not in ("ok", "lost"):
                bad("chunks.tsv %s: ssh_after %r" % (c["label"], c["ssh_after"]))
            if c["boot_same"] not in ("yes", "no"):
                bad("chunks.tsv %s: boot_same %r" % (c["label"], c["boot_same"]))
            if not os.path.isfile(os.path.join(path, "kmsg", c["label"] + ".txt")):
                bad("chunks.tsv %s: no kmsg/%s.txt" % (c["label"], c["label"]))
            s["chunks"].append(c)
    head, rows = rows_of(os.path.join(path, "results.tsv"))
    if head != list(RESULT_COLS):
        bad("results.tsv header is not %s" % " ".join(RESULT_COLS))
    else:
        for i, f in enumerate(rows, 1):
            if len(f) < 2 or len(f) > 3 or not f[0]:
                bad("results.tsv row %d: need case, status[, detail]" % i)
                continue
            s["results"].append({"case": f[0], "status": f[1], "detail": f[2] if len(f) == 3 else ""})
    if s["chunks"] and s["results"]:
        if sum(int(c["cases"]) for c in s["chunks"] if c["cases"].isdigit()) != len(s["results"]):
            bad("chunks.tsv cases sum differs from the %d rows of results.tsv" % len(s["results"]))
    return s


def merged_ancestor(commit):
    r = subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/main"],
                       capture_output=True, text=True)
    return r.returncode == 0


def check_cover(mustpass, slices, use_git=True):
    want = [l.strip() for l in open(mustpass, encoding="utf-8") if l.strip() and not l.startswith("#")]
    wantset = set(want)
    problems = []
    seen = collections.Counter()
    for s in slices:
        problems += ["%s: %s" % (s["name"], p) for p in s["problems"]]
        if use_git and re.fullmatch(r"[0-9a-f]{40}", s["meta"].get("commit", "")) \
                and not merged_ancestor(s["meta"]["commit"]):
            problems.append("%s: commit %s is not an ancestor of origin/main" % (s["name"], s["meta"]["commit"][:8]))
        for r in s["results"]:
            if r["status"] not in STATUSES:
                problems.append("%s: %s has status %r" % (s["name"], r["case"], r["status"]))
            elif r["status"] == "Lost" and not r["detail"].strip():
                problems.append("%s: %s is Lost without a reason" % (s["name"], r["case"]))
            if r["case"] in wantset:
                seen[r["case"]] += 1
    missing = [c for c in want if seen[c] == 0]
    dup = [c for c in want if seen[c] > 1]
    if missing:
        problems.append("%d mustpass case(s) have no status, first %s" % (len(missing), missing[0]))
    if dup:
        problems.append("%d mustpass case(s) appear more than once, first %s" % (len(dup), dup[0]))
    return problems


def normalize(detail):
    d = re.sub(r"dEQP-GLES2(\.\w+)+", "<case>", detail)
    d = re.sub(r"'[^']*'", "'<name>'", d)
    d = re.sub(r"0x[0-9a-fA-F]+", "N", d)
    d = re.sub(r"\b\d+(\.\d+)?\b", "N", d)
    d = re.sub(r"\s+", " ", d).strip()
    return d or "(no message)"


def refusal(text):
    m = REFUSED_NAMED.search(text) or REFUSED_AFTER.search(text)
    return m.group(1) if m else None


def cause(status, detail, exit_line):
    if status == "Lost":
        return "lost:" + (detail.strip() or "no-reason")
    name = refusal(detail) or (refusal(exit_line) if exit_line not in ("", "-") and status in (
        "Crash", "InternalError", "Timeout") else None)
    return "refused:" + name if name else normalize(detail)


def tally(slices):
    """({case: (status, cause)}, by status Counter, by (status, cause) -> [cases])"""
    per = {}
    for s in slices:
        i = 0
        for c in s["chunks"]:
            n = int(c["cases"])
            for r in s["results"][i:i + n]:
                per[r["case"]] = (r["status"], r["detail"], c["exit_line"])
            i += n
        for r in s["results"][i:]:
            per.setdefault(r["case"], (r["status"], r["detail"], "-"))
    by_status = collections.Counter(v[0] for v in per.values())
    causes = collections.defaultdict(list)
    for case in sorted(per):
        st, detail, ex = per[case]
        if st not in ("Pass", "NotSupported"):
            causes[(st, cause(st, detail, ex))].append(case)
    return per, by_status, causes


def render(slices, markdown):
    per, by_status, causes = tally(slices)
    out = []
    commits = sorted({s["meta"].get("commit", "?")[:8] for s in slices})
    if markdown:
        out.append("NanoPC-T6 (Mali-G610, panthor), dEQP-GLES2 at %s: %d cases in %d slice(s) (%s)."
                   % (", ".join(commits), len(per), len(slices), ", ".join(s["name"] for s in slices)))
        out += ["", "| Status | Cases |", "|---|---:|"]
        out += ["| %s | %d |" % (st, by_status[st]) for st in STATUSES if by_status[st]]
        out += ["", "| Status | Cause | Cases | First case |", "|---|---|---:|---|"]
        for (st, ca), cs in sorted(causes.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            out.append("| %s | %s | %d | `%s` |" % (st, ca.replace("|", "\\|"), len(cs), cs[0]))
        if not causes:
            out.append("| - | none | 0 | - |")
    else:
        out.append("cases %d in %d slice(s), commits %s" % (len(per), len(slices), ", ".join(commits)))
        for st in STATUSES:
            if by_status[st]:
                out.append("  %-22s %6d" % (st, by_status[st]))
        for (st, ca), cs in sorted(causes.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            out.append("  %-12s %5d  %s  (first %s)" % (st, len(cs), ca, cs[0]))
    return "\n".join(out) + "\n"


def doc_block(text):
    a, b = text.find(BEGIN), text.find(END)
    if a < 0 or b < a:
        return None
    return text[a + len(BEGIN):b].strip("\n") + "\n"


def check_doc(doc, slices):
    got = doc_block(open(doc, encoding="utf-8").read())
    if got is None:
        return ["%s has no %s ... %s block" % (doc, BEGIN, END)]
    want = render(slices, True).strip("\n") + "\n"
    return [] if got.strip("\n") + "\n" == want else ["%s: the marked block differs from --markdown" % doc]


# ---------------------------------------------------------------- self-test

TESTDATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata", "mali-deqp")


def self_test():
    import shutil
    bad = 0

    def check(desc, ok):
        nonlocal bad
        print("  %s  %s" % ("ok  " if ok else "FAIL", desc))
        bad += not ok
    mp = os.path.join(TESTDATA, "mustpass.txt")
    good = [os.path.join(TESTDATA, "good", d) for d in sorted(os.listdir(os.path.join(TESTDATA, "good")))]
    sl = [load(p) for p in good]
    check("fixtures: %d good slices, no file problems" % len(sl), len(sl) == 2 and not any(s["problems"] for s in sl))
    check("good slices cover the mustpass exactly once", check_cover(mp, sl, False) == [])
    check("an overlap is named", any("more than once" in p for p in check_cover(mp, [sl[0], sl[0]], False)))
    check("one slice alone leaves cases without status", any("no status" in p for p in check_cover(mp, sl[:1], False)))

    def broken(desc, edit, expect):
        d = tempfile.mkdtemp(dir=os.environ.get("TMPDIR") or None)
        try:
            p = os.path.join(d, os.path.basename(good[1]))
            shutil.copytree(good[1], p)
            edit(p)
            pr = check_cover(mp, [sl[0], load(p)], False)
            check("%s -> %s" % (desc, expect), any(expect in x for x in pr))
        finally:
            shutil.rmtree(d)

    def sub(name, a, b):
        def f(p):
            fp = os.path.join(p, name)
            t = open(fp).read()
            assert a in t, (name, a)
            open(fp, "w").write(t.replace(a, b, 1))
        return f
    broken("Lost without a reason", sub("results.tsv", "\tchunk-crash", "\t"), "Lost without a reason")
    broken("unknown status", sub("results.tsv", "\tPass\t", "\tPassed\t"), "status 'Passed'")
    broken("a chunk row without ssh_after", sub("chunks.tsv", "\tlost\tyes\t", "\t\tyes\t"), "empty ones not allowed")
    broken("bad boot_same", sub("chunks.tsv", "\tyes\t-", "\tmaybe\t-"), "boot_same")
    broken("kmsg file missing", lambda p: os.remove(os.path.join(p, "kmsg", "S2-r1.txt")), "no kmsg/S2-r1.txt")
    broken("meta key missing", sub("meta.txt", "kernel=", "kern="), "lacks kernel")
    broken("short commit", sub("meta.txt", "commit=", "commit=x"), "40 hex")
    broken("cases sum differs", sub("chunks.tsv", "S2-r1\t3", "S2-r1\t4"), "cases sum")
    check("a commit that is no ancestor of origin/main is refused", not merged_ancestor("0" * 40))
    per, by_status, causes = tally(sl)
    check("status counts", dict(by_status) == {"Pass": 5, "Fail": 2, "NotSupported": 1, "Crash": 1, "Lost": 2, "InternalError": 1})
    keys = {c for (_, c) in causes}
    check("number and case names are stripped from a cause",
          "Expected N, got N at 'N' vs '<name>'" not in keys and any(k.startswith("Image comparison failed") for k in keys))
    check("two Fails with different numbers share one cause",
          any(st == "Fail" and len(cs) == 2 for (st, _), cs in causes.items()))
    check("refused:<name> from a detail", ("InternalError", "refused:Select") in causes)
    check("refused:<name> from the chunk's exit line", ("Crash", "refused:NOT_BUILT_MRT") in causes)
    check("Lost causes carry the reason", ("Lost", "lost:chunk-crash") in causes and ("Lost", "lost:not-run") in causes)
    md = render(sl, True)
    check("markdown has both tables", "| Status | Cases |" in md and "| Status | Cause | Cases | First case |" in md)
    d = tempfile.mkdtemp(dir=os.environ.get("TMPDIR") or None)
    try:
        doc = os.path.join(d, "doc.md")
        open(doc, "w").write("# x\n\n%s\n%s%s\ntail\n" % (BEGIN, md, END))
        check("--check-doc accepts the generated block", check_doc(doc, sl) == [])
        open(doc, "w").write("# x\n\n%s\n%s\n%s\n" % (BEGIN, md.replace("| 5 |", "| 6 |"), END))
        check("--check-doc rejects an edited block", check_doc(doc, sl) != [])
        open(doc, "w").write("no markers\n")
        check("--check-doc rejects a doc without markers", check_doc(doc, sl) != [])
    finally:
        shutil.rmtree(d)
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("args", nargs="*")
    ap.add_argument("--check-cover", action="store_true")
    ap.add_argument("--tally", action="store_true")
    ap.add_argument("--check-doc", action="store_true")
    ap.add_argument("--markdown", action="store_true")
    ap.add_argument("--no-git", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if a.check_cover or a.check_doc:
        if len(a.args) < 2:
            ap.error("need a file and at least one slice")
        slices = [load(p) for p in a.args[1:]]
        probs = check_cover(a.args[0], slices, not a.no_git) if a.check_cover else check_doc(a.args[0], slices)
        for p in probs[:50]:
            print("FAIL " + p)
        print("%s: %d problem(s)" % ("cover" if a.check_cover else "doc", len(probs)))
        return 1 if probs else 0
    if a.tally:
        if not a.args:
            ap.error("no slices")
        sys.stdout.write(render([load(p) for p in a.args], a.markdown))
        return 0
    ap.error("give a mode")


if __name__ == "__main__":
    sys.exit(main())

