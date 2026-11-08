#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cover, tally and doc checks over NanoPC-T6 dEQP-GLES2 slices.

  python3 -I tools/cts/mali-deqp-tally.py --check-cover MUSTPASS SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --tally [--markdown] SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --check-doc DOC SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --self-test

A slice is tools/cts/log/<YYYY-MM-DD>-t6-gles2-<session>-<sha8>/ with

  meta.txt     key=value: commit (40 hex), session, rows (<a>-<b>), tree,
               boot_id_before, boot_id_after, kernel, lib, started, ended
  chunks.tsv   row label cases status seconds kmsg resets ssh_after boot_same
               exit_line   (status done|crash|hang|fault|lost; ssh_after
               yes|no: did the board answer ssh after the chunk; boot_same
               yes|no; exit_line is the library's yakogl-mali-exit line or -)
               A row the runner never ran has status lost and '-' for kmsg,
               resets, ssh_after and boot_same (nothing was measured); '-' is
               accepted there and nowhere else, so a done/crash/hang/fault
               row with '-' or an empty field is rejected.
  results.tsv  case status detail   (a dEQP status, or Lost with a reason:
               chunk-crash, chunk-hang, board-lost, not-run)
  kmsg/<label>.txt   the redacted kernel lines of the chunk

results.tsv lists the cases in mustpass order, chunk after chunk, so the
chunks' `cases` counts split it; a chunk whose exit line names a refusal
gives that refusal as the cause of its non-Pass cases.

Every rule failure prints "RULE <name>: ..." and the mode exits 1; a slice
that cannot be read exits 2. --no-git skips the ancestor rule (fixtures);
--main-ref REF names the ref the commits must be ancestors of (origin/main).
"""
import os
import re
import subprocess
import sys
import tempfile

STATUSES = ("Pass", "Fail", "NotSupported", "QualityWarning",
            "CompatibilityWarning", "InternalError", "ResourceError",
            "Crash", "Timeout", "Lost")
CHUNK_STATUS = ("done", "crash", "hang", "fault", "lost")
CHUNK_COLS = ("row", "label", "cases", "status", "seconds", "kmsg",
              "resets", "ssh_after", "boot_same", "exit_line")
META_KEYS = ("commit", "session", "rows", "tree", "boot_id_before",
             "boot_id_after", "kernel", "lib", "started", "ended")
LOST_REASONS = ("chunk-crash", "chunk-hang", "board-lost", "not-run")
BEGIN = "<!-- mali-deqp-tally begin -->"
END = "<!-- mali-deqp-tally end -->"
# A back end refusal in a detail or an exit line: "refused <name>",
# "refused: <name>" or "refusal=<name>".
REFUSED = re.compile(r"\brefus(?:ed|al)\b[\s:=]+([A-Za-z_][A-Za-z0-9_.:-]*)")
HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.join(HERE, "testdata", "mali-deqp")


class Fatal(Exception):
    pass


def read_tsv(path, header):
    try:
        with open(path, encoding="utf-8", newline="") as f:
            lines = f.read().split("\n")
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))
    if lines and lines[-1] == "":
        lines.pop()
    if not lines or tuple(lines[0].split("\t")) != header:
        raise Fatal("%s: header is not %s" % (path, "\t".join(header)))
    rows = []
    for n, ln in enumerate(lines[1:], 2):
        f = ln.split("\t")
        if len(f) > len(header):
            f = f[:len(header) - 1] + ["\t".join(f[len(header) - 1:])]
        rows.append((n, f + [""] * (len(header) - len(f)), len(f) >= len(header)))
    return rows


def load_slice(path):
    path = path.rstrip("/")
    meta = {}
    try:
        with open(os.path.join(path, "meta.txt"), encoding="utf-8") as f:
            for ln in f.read().splitlines():
                if "=" in ln:
                    k, v = ln.split("=", 1)
                    meta[k.strip()] = v.strip()
    except OSError as e:
        raise Fatal("%s/meta.txt: %s" % (path, e.strerror))
    chunks = read_tsv(os.path.join(path, "chunks.tsv"), CHUNK_COLS)
    results = read_tsv(os.path.join(path, "results.tsv"),
                       ("case", "status", "detail"))
    return {"path": path, "name": os.path.basename(path), "meta": meta,
            "chunks": chunks, "results": results}


def load_mustpass(path):
    try:
        with open(path, encoding="utf-8") as f:
            return [ln.strip() for ln in f if ln.strip()]
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))


def is_ancestor(commit, ref, cwd):
    r = subprocess.run(["git", "merge-base", "--is-ancestor", commit, ref],
                       cwd=cwd, capture_output=True)
    return r.returncode == 0


def check_cover(mustpass, slices, git=True, main_ref="origin/main", cwd=None):
    """Returns the list of 'RULE name: text' errors."""
    errs = []
    want = set(mustpass)
    seen = {}
    for s in slices:
        n = s["name"]
        for k in META_KEYS:
            if not s["meta"].get(k):
                errs.append("RULE meta-field: %s: meta.txt has no %s" % (n, k))
        c = s["meta"].get("commit", "")
        if c and not re.fullmatch(r"[0-9a-f]{40}", c):
            errs.append("RULE meta-field: %s: commit %r is not 40 hex" % (n, c))
        elif c and git and not is_ancestor(c, main_ref, cwd):
            errs.append("RULE commit-ancestor: %s: commit %s is not an "
                        "ancestor of %s" % (n, c[:8], main_ref))
        for ln, f, whole in s["chunks"]:
            row = "%s chunks.tsv:%d" % (n, ln)
            if not whole:
                errs.append("RULE chunk-field: %s: too few fields" % row)
            for i, col in enumerate(CHUNK_COLS):
                if not f[i]:
                    errs.append("RULE chunk-field: %s: %s is empty" % (row, col))
            unrun = f[CHUNK_COLS.index("status")] == "lost"
            for col in ("kmsg", "resets", "cases", "seconds"):
                v = f[CHUNK_COLS.index(col)]
                if unrun and v == "-" and col in ("kmsg", "resets"):
                    continue
                if v and not v.isdigit():
                    errs.append("RULE chunk-field: %s: %s %r is not a count"
                                % (row, col, v))
            for col, ok in (("ssh_after", ("yes", "no")),
                            ("boot_same", ("yes", "no")),
                            ("status", CHUNK_STATUS)):
                v = f[CHUNK_COLS.index(col)]
                if unrun and v == "-" and col != "status":
                    continue
                if v and v not in ok:
                    errs.append("RULE chunk-field: %s: %s %r is not %s"
                                % (row, col, v, "|".join(ok)))
        for ln, f, whole in s["results"]:
            case, st, detail = f
            where = "%s results.tsv:%d" % (n, ln)
            if not whole or not st:
                errs.append("RULE result-status: %s: %s has no status"
                            % (where, case))
            elif st not in STATUSES:
                errs.append("RULE result-status: %s: %s status %r unknown"
                            % (where, case, st))
            elif st == "Lost" and not detail.strip():
                errs.append("RULE lost-reason: %s: %s is Lost without a reason"
                            % (where, case))
            if case not in want:
                errs.append("RULE cover-unknown: %s: %s is not in the mustpass "
                            "list" % (where, case))
            elif case in seen:
                errs.append("RULE cover-duplicate: %s: %s is also in %s"
                            % (where, case, seen[case]))
            else:
                seen[case] = where
    miss = [c for c in mustpass if c not in seen]
    for c in miss[:20]:
        errs.append("RULE cover-missing: %s is in no slice" % c)
    if len(miss) > 20:
        errs.append("RULE cover-missing: ... and %d more" % (len(miss) - 20))
    return errs


def normalize(detail):
    d = re.sub(r"dEQP-[A-Za-z0-9_.]+", "<case>", detail)
    d = re.sub(r"'[^']*'|\"[^\"]*\"", "<name>", d)
    d = re.sub(r"0x[0-9a-fA-F]+", "<hex>", d)
    d = re.sub(r"-?\d+(?:\.\d+)?", "#", d)
    return re.sub(r"\s+", " ", d).strip() or "-"


def chunk_exit_lines(s):
    """Per result index, the exit line of the chunk it belongs to (or '')."""
    out = [""] * len(s["results"])
    pos = 0
    counts = []
    for _, f, _w in s["chunks"]:
        counts.append((int(f[2]) if f[2].isdigit() else 0, f[9]))
    if sum(c for c, _ in counts) != len(out):
        return out
    for c, ex in counts:
        for i in range(pos, pos + c):
            out[i] = "" if ex == "-" else ex
        pos += c
    return out


def cause(status, detail, exit_line):
    m = REFUSED.search(detail) or REFUSED.search(exit_line)
    if m:
        return "refused:" + m.group(1)
    return normalize(detail)


def tally(slices):
    by_status = {}
    by_cause = {}
    for s in slices:
        ex = chunk_exit_lines(s)
        for i, (_, f, _w) in enumerate(s["results"]):
            st = f[1]
            by_status[st] = by_status.get(st, 0) + 1
            if st != "Pass":
                k = (st, cause(st, f[2], ex[i]))
                by_cause[k] = by_cause.get(k, 0) + 1
    return by_status, by_cause


def status_order(by_status):
    return [s for s in STATUSES if s in by_status] + \
        sorted(s for s in by_status if s not in STATUSES)


def render_text(slices):
    by_status, by_cause = tally(slices)
    out = ["slices: %d" % len(slices)]
    for st in status_order(by_status):
        out.append("%-22s %d" % (st, by_status[st]))
    out.append("%-22s %d" % ("TOTAL", sum(by_status.values())))
    for (st, c), n in sorted(by_cause.items(), key=lambda x: (-x[1], x[0])):
        out.append("cause %-14s %6d  %s" % (st, n, c))
    return "\n".join(out) + "\n"


def render_markdown(slices):
    by_status, by_cause = tally(slices)
    commits = sorted({s["meta"].get("commit", "")[:8] for s in slices})
    out = [BEGIN, "",
           "Slices: " + ", ".join("`%s`" % s["name"] for s in
                                  sorted(slices, key=lambda x: x["name"])),
           "(commits " + ", ".join("`%s`" % c for c in commits) + ").", "",
           "| Status | Cases |", "| --- | ---: |"]
    for st in status_order(by_status):
        out.append("| %s | %d |" % (st, by_status[st]))
    out.append("| **Total** | **%d** |" % sum(by_status.values()))
    out += ["", "| Status | Cause | Cases |", "| --- | --- | ---: |"]
    for (st, c), n in sorted(by_cause.items(), key=lambda x: (-x[1], x[0])):
        out.append("| %s | %s | %d |" % (st, c.replace("|", "\\|"), n))
    out += ["", END]
    return "\n".join(out) + "\n"


def doc_block(text):
    a = text.find(BEGIN)
    b = text.find(END)
    if a < 0 or b < a:
        return None
    return text[a:b + len(END)] + "\n"


def check_doc(doc, slices):
    try:
        with open(doc, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        raise Fatal("%s: %s" % (doc, e.strerror))
    blk = doc_block(text)
    if blk is None:
        return ["RULE doc-block: %s has no %s ... %s block" % (doc, BEGIN, END)]
    if blk != render_markdown(slices):
        return ["RULE doc-stale: %s: the mali-deqp-tally block differs from "
                "--markdown of the slices" % doc]
    return []


def report(errs):
    for e in errs:
        print(e, file=sys.stderr)
    return 1 if errs else 0


def self_test():
    n_ok = 0
    n_bad = 0
    me = os.path.abspath(__file__)

    def run(*args, cwd=None):
        r = subprocess.run([sys.executable, "-I", me] + list(args),
                           capture_output=True, text=True, cwd=cwd)
        return r.returncode, r.stdout, r.stderr

    def chk(label, cond):
        nonlocal n_ok, n_bad
        if cond:
            n_ok += 1
            print("  ok    " + label)
        else:
            n_bad += 1
            print("  FAIL  " + label)

    def must(label, rule, *args, cwd=None):
        rc, _o, e = run(*args, cwd=cwd)
        chk(label, rc == 1 and ("RULE " + rule) in e)

    mp = os.path.join(FIXTURES, "mustpass.txt")
    g = os.path.join(FIXTURES, "good")
    good = sorted(os.path.join(g, d) for d in os.listdir(g))
    bad = os.path.join(FIXTURES, "bad")

    def b(name):
        return os.path.join(bad, name)

    rc, _o, e = run("--check-cover", "--no-git", mp, *good)
    chk("good slices cover the mustpass list", rc == 0)
    rc, o, _e = run("--tally", "--no-git", *good)
    chk("tally: total 8", rc == 0 and re.search(r"^TOTAL +8$", o, re.M))
    chk("tally: 3 Pass", re.search(r"^Pass +3$", o, re.M) is not None)
    chk("tally: refused cause counted",
        re.search(r"InternalError +1 +refused:blend_equation_separate", o)
        is not None)
    chk("tally: numbers stripped from Fail cause",
        "Image comparison failed: max difference = (#, #, #)" in o)
    chk("tally: Lost reason is its cause", "chunk-crash" in o)
    rc, o, _e = run("--tally", "--markdown", *good)
    chk("markdown: marked block", o.startswith(BEGIN) and o.endswith(END + "\n"))
    chk("markdown: matches the pinned fixture",
        o == open(os.path.join(FIXTURES, "tally.md"), encoding="utf-8").read())
    chk("check-doc: current block", run("--check-doc",
        os.path.join(FIXTURES, "doc-current.md"), *good)[0] == 0)
    must("duplicate case", "cover-duplicate", "--check-cover", "--no-git", mp,
         *good, b("dup"))
    must("missing case", "cover-missing", "--check-cover", "--no-git", mp,
         good[0])
    must("unknown case", "cover-unknown", "--check-cover", "--no-git", mp,
         *good, b("unknown"))
    must("missing chunk field", "chunk-field", "--check-cover", "--no-git",
         mp, good[0], b("nofield"))
    chk("never-run row ('-' fields, status lost) is accepted",
        run("--check-cover", "--no-git", mp, good[0],
            os.path.join(FIXTURES, "lost"))[0] == 0)
    must("ssh_after 'ok' (the old vocabulary)", "chunk-field", "--check-cover",
         "--no-git", mp, good[0], b("sshold"))
    must("ssh_after '-' on a done row", "chunk-field", "--check-cover",
         "--no-git", mp, good[0], b("sshdash"))
    must("Lost without a reason", "lost-reason", "--check-cover", "--no-git",
         mp, good[0], b("lostnoreason"))
    must("unknown status", "result-status", "--check-cover", "--no-git", mp,
         good[0], b("badstatus"))
    must("meta key missing", "meta-field", "--check-cover", "--no-git", mp,
         *good, b("nometa"))
    must("stale doc block", "doc-stale", "--check-doc",
         os.path.join(FIXTURES, "doc-stale.md"), *good)
    must("doc without block", "doc-block", "--check-doc",
         os.path.join(FIXTURES, "doc-noblock.md"), *good)
    rc, _o, e = run("--check-cover", "--no-git", mp, os.path.join(FIXTURES, "nope"))
    chk("unreadable slice exits 2", rc == 2)
    # ancestor rule, in a throwaway repository
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as t:
        def git(*a):
            return subprocess.run(
                ["git", "-c", "user.name=t", "-c", "user.email=t@t",
                 "-c", "commit.gpgsign=false"] + list(a), cwd=t,
                capture_output=True, text=True, check=True).stdout.strip()
        git("init", "-q", "-b", "main")
        git("commit", "-q", "--allow-empty", "-m", "a")
        a = git("rev-parse", "HEAD")
        git("checkout", "-q", "-b", "side")
        git("commit", "-q", "--allow-empty", "-m", "b")
        side = git("rev-parse", "HEAD")
        git("checkout", "-q", "main")

        def with_commit(src, commit, dst):
            os.makedirs(dst)
            for dp, _dn, fns in os.walk(src):
                od = os.path.join(dst, os.path.relpath(dp, src))
                os.makedirs(od, exist_ok=True)
                for fn in fns:
                    with open(os.path.join(dp, fn), encoding="utf-8",
                              newline="") as f:
                        txt = f.read()
                    if fn == "meta.txt":
                        txt = re.sub(r"(?m)^commit=.*$", "commit=" + commit, txt)
                    with open(os.path.join(od, fn), "w", encoding="utf-8",
                              newline="") as f:
                        f.write(txt)
        s1 = os.path.join(t, "anc", "s1")
        s2 = os.path.join(t, "anc", "s2")
        with_commit(good[0], a, s1)
        with_commit(good[1], a, s2)
        rc, _o, e = run("--check-cover", "--main-ref", "main", mp, s1, s2, cwd=t)
        chk("ancestor commit passes", rc == 0)
        n1 = os.path.join(t, "non", "s1")
        n2 = os.path.join(t, "non", "s2")
        with_commit(good[0], side, n1)
        with_commit(good[1], a, n2)
        must("non-ancestor commit", "commit-ancestor", "--check-cover",
             "--main-ref", "main", mp, n1, n2, cwd=t)
        chk("--no-git skips the ancestor rule",
            run("--check-cover", "--no-git", "--main-ref", "main", mp, n1, n2,
                cwd=t)[0] == 0)
    print("%d checks passed" % n_ok if not n_bad else
          "%d checks passed, %d FAILED" % (n_ok, n_bad))
    return 1 if n_bad else 0


def main(argv):
    git = True
    markdown = False
    main_ref = "origin/main"
    args = []
    mode = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--no-git":
            git = False
        elif a == "--markdown":
            markdown = True
        elif a == "--main-ref":
            i += 1
            if i >= len(argv):
                print("--main-ref needs a ref", file=sys.stderr)
                return 2
            main_ref = argv[i]
        elif a in ("--check-cover", "--tally", "--check-doc", "--self-test"):
            mode = a
        elif a.startswith("--"):
            print("unknown option " + a, file=sys.stderr)
            return 2
        else:
            args.append(a)
        i += 1
    try:
        if mode == "--self-test":
            return self_test()
        if mode == "--check-cover" and len(args) >= 2:
            s = [load_slice(p) for p in args[1:]]
            return report(check_cover(load_mustpass(args[0]), s, git,
                                      main_ref))
        if mode == "--tally" and args:
            s = [load_slice(p) for p in args]
            sys.stdout.write(render_markdown(s) if markdown else render_text(s))
            return 0
        if mode == "--check-doc" and len(args) >= 2:
            return report(check_doc(args[0], [load_slice(p) for p in args[1:]]))
    except Fatal as e:
        print("mali-deqp-tally: %s" % e, file=sys.stderr)
        return 2
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
