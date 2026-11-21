#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cover, tally and doc checks over NanoPC-T6 dEQP-GLES2 slices.

  python3 -I tools/cts/mali-deqp-tally.py --check-cover MUSTPASS SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --tally [--markdown] SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --check-doc DOC SLICE...
  python3 -I tools/cts/mali-deqp-tally.py --check-doc DOC --log LOGDIR
  python3 -I tools/cts/mali-deqp-tally.py --self-test

--per-case (with --tally, text or --markdown) takes the cause of a non-Pass
case from the refusal line the Mali backend logged inside that case's own
output, not from the chunk's exit line. It reads the optional slice file
refusals/<label>.txt: a chunk's stdout and stderr merged in the order the
process wrote them, holding dEQP's "Test case '<case>'.." lines and the
backend's per-draw line (src/mali/backend.rs refusal_line)

  yakogl-mali-refusal: <errno> <reason>

A refusal belongs to the last Test case line before it; the case's first
such line is its cause. The cause is "refused:<normalized reason>", or
"errno:<errno>" (for example "errno:-12") when the reason is the bare word
"errno" (cause already logged elsewhere), so ResourceError cases split by
errno. The "yakogl: a draw was refused:" line (first refusal of the whole
process only) is ignored. A case's refusals.tsv text, if its slice has one,
still comes first; a case without a line, and a slice or chunk without the
file, keep the default cause. Without --per-case nothing reads those
files, so --tally, --check-doc and --check-cover are unchanged.
dEQP stdout buffering into a file can reorder lines: check the raw order.

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
  refusals.tsv case refusal   (optional; the runner writes it since
               2026-11-15: for each case whose log holds one, the first info
               log in which the library refuses a shader or a program)
  kmsg/<label>.txt   the redacted kernel lines of the chunk

A session the runner ends on board-lost or a wedged process marks the rest
Lost (board-lost, not-run); those cases are rerun in a new slice whose
meta.txt carries one more, optional key

  reruns=<dir name>[,<dir name>...]   (one or more earlier slices, each
               among the slices given)

and a slice of a board other than the NanoPC-T6 one more, optional key,
board=<MALI_BOARD> (tools/mali/panthor/board-profile.sh), which the tally
does not read.

A case may then appear in the rerunning slice and in an earlier slice, and
only so: that earlier slice is one of the names in reruns and has the case as
Lost. The later status wins: the earlier Lost row is dropped from the tally
and the cover. One rerun slice can thus take over the Lost cases of several
slices. Chains work (C reruns B reruns A), each link by the same rule. Any
other duplicate is RULE cover-duplicate; a reruns naming a slice not given,
the slice itself, or forming a cycle is RULE rerun-target, for every name in
the list.

A second optional key replaces a whole chunk, whatever its status:

  replaces=<dir name>[,<dir name>...]   (one or more earlier slices, each
               among the slices given)

Every chunk label the slice ran replaces the chunk of the same label in each
named slice: all that chunk's cases (Pass, Fail, Lost, ...) drop out of the
tally and the cover, and the later slice's cases stand in their place. It is
how a partial rerun of some rows of a tree (from a newer commit) is combined
with the whole tree. A case of a replaced chunk that the replacing slice's
chunk lacks is RULE replace-cover; a label the named slice has no chunk for
replaces nothing; the chunks' `cases` counts must add up to the results (they
map chunks to cases). A replaces naming a slice not given, or the slice
itself, is RULE replace-target. The key combines with reruns= in one slice
(replaced rows first, then the Lost rows of the reruns); a case in a chunk
neither key reaches is still RULE cover-duplicate. The markdown block's
commit list holds the commit of every slice given, replaced or not.

The cause of a non-Pass case, in a slice with refusals.tsv: its own refusal
when the file names the case ("refused, <vertex|fragment> shader: <why>"),
else dEQP's Result text with names and numbers folded. In a slice without
the file the tally can only guess: results.tsv lists the cases in mustpass
order, chunk after chunk, so the chunks' `cases` counts split it, and a chunk
whose exit line names a refusal gives that refusal as the cause of all its
non-Pass cases.

--check-doc DOC --log LOGDIR checks the doc's block against the slices the
block itself names (its "Slices:" line), read from LOGDIR: the form the gate
runs on docs/es2-cts-failures.md and tools/cts/log/.

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
REFUSAL_COLS = ("case", "refusal")
META_KEYS = ("commit", "session", "rows", "tree", "boot_id_before",
             "boot_id_after", "kernel", "lib", "started", "ended")
LOST_REASONS = ("chunk-crash", "chunk-hang", "board-lost", "not-run")
BEGIN = "<!-- mali-deqp-tally begin -->"
END = "<!-- mali-deqp-tally end -->"
# A back end refusal in a detail or an exit line: "refused <name>",
# "refused: <name>" or "refusal=<name>".
REFUSED = re.compile(r"\brefus(?:ed|al)\b[\s:=]+([A-Za-z_][A-Za-z0-9_.:-]*)")
# A refusals.tsv text: "... refused this <stage> shader: <why>", where <why>
# is the compiler's error, "Mali(<text>)" for this back end.
SHADER_REFUSAL = re.compile(r"\brefused this (\w+) shader: (.*)$")
CASE_LINE = re.compile(r"Test case '([^']+)'")
MALI_REFUSAL = re.compile(r"yakogl-mali-refusal:\s+(-?\d+)\s*(.*\S)?")
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
    refusals = None
    if os.path.exists(os.path.join(path, "refusals.tsv")):
        refusals = read_tsv(os.path.join(path, "refusals.tsv"), REFUSAL_COLS)
    return {"path": path, "name": os.path.basename(path), "meta": meta,
            "chunks": chunks, "results": results, "refusals": refusals}


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


def rerun_names(s):
    """The slice names of s's reruns key (comma-separated), in order."""
    return [n.strip() for n in s["meta"].get("reruns", "").split(",")
            if n.strip()]


def replace_names(s):
    """The slice names of s's replaces key (comma-separated), in order."""
    return [n.strip() for n in s["meta"].get("replaces", "").split(",")
            if n.strip()]


def order_reruns(slices):
    """Slices with every rerun (and replacing slice) after the slice it
    reruns or replaces, plus the errors of a reruns or replaces key that
    names no given slice, itself, or a cycle."""
    errs = []
    by_name = {s["name"]: s for s in slices}
    done = []
    names = set()
    pending = list(slices)
    while pending:
        moved = False
        for s in list(pending):
            rs = rerun_names(s)
            bad = False
            for r in rs:
                if r not in by_name:
                    errs.append("RULE rerun-target: %s: reruns %r is not a "
                                "slice given" % (s["name"], r))
                    bad = True
                elif r == s["name"]:
                    errs.append("RULE rerun-target: %s: reruns itself" % r)
                    bad = True
            for r in replace_names(s):
                if r not in by_name:
                    errs.append("RULE replace-target: %s: replaces %r is not "
                                "a slice given" % (s["name"], r))
                    bad = True
                elif r == s["name"]:
                    errs.append("RULE replace-target: %s: replaces itself" % r)
                    bad = True
                rs = rs + [r]
            if not bad:
                if not all(r in names for r in rs):
                    continue
                done.append(s)
                names.add(s["name"])
            pending.remove(s)
            moved = True
        if not moved:
            for s in pending:
                errs.append("RULE rerun-target: %s: reruns cycle" % s["name"])
            break
    return done, errs


def chunk_rows(s):
    """{label: [result indexes]} from the chunks' `cases` counts, in order,
    or None when the counts do not add up to the results."""
    out = {}
    pos = 0
    for _ln, f, _w in s["chunks"]:
        if not f[2].isdigit():
            return None
        n = int(f[2])
        out.setdefault(f[1], []).extend(range(pos, pos + n))
        pos += n
    return out if pos == len(s["results"]) else None


def replace_rows(s, p, errs):
    """The result indexes of slice p that slice s replaces: every chunk of p
    whose label s ran. Cases of those chunks that s lacks are RULE
    replace-cover errors."""
    mine = chunk_rows(s)
    theirs = chunk_rows(p)
    if mine is None or theirs is None:
        bad = s["name"] if mine is None else p["name"]
        errs.append("RULE replace-cover: %s: the chunks' cases counts do not "
                    "add up to results.tsv (needed by replaces in %s)"
                    % (bad, s["name"]))
        return set()
    out = set()
    for lab, idx in mine.items():
        if lab not in theirs:
            continue
        have = {s["results"][i][1][0] for i in idx}
        for i in theirs[lab]:
            out.add(i)
            c = p["results"][i][1][0]
            if c not in have:
                errs.append("RULE replace-cover: %s: chunk %s replaces %s "
                            "but lacks %s" % (s["name"], lab, p["name"], c))
    return out


def resolve(slices):
    """(ordered slices, superseded {slice name: set of result indexes},
    {(slice name, result index): earlier 'where'} for the duplicates no
    rerun explains, errors of the reruns keys)."""
    ordered, errs = order_reruns(slices)
    owner = {}
    superseded = {s["name"]: set() for s in ordered}
    dups = {}
    by_name = {s["name"]: s for s in ordered}
    for s in ordered:
        rs = rerun_names(s)
        for pn in replace_names(s):
            p = by_name.get(pn)
            if p is None or p is s:
                continue
            for pi in replace_rows(s, p, errs):
                superseded[pn].add(pi)
                c = p["results"][pi][1][0]
                if owner.get(c, (None, None))[0] is p:
                    del owner[c]
        for i, (_ln, f, _w) in enumerate(s["results"]):
            prev = owner.get(f[0])
            if prev is not None:
                ps, pi = prev
                if ps["name"] in rs and ps["results"][pi][1][1] == "Lost":
                    superseded[ps["name"]].add(pi)
                else:
                    dups[(s["name"], i)] = "%s results.tsv:%d" % (
                        ps["name"], ps["results"][pi][0])
                    continue
            owner[f[0]] = (s, i)
    return ordered, superseded, dups, errs


def check_cover(mustpass, slices, git=True, main_ref="origin/main", cwd=None):
    """Returns the list of 'RULE name: text' errors."""
    slices, _sup, dups, errs = resolve(slices)
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
        for ri, (ln, f, whole) in enumerate(s["results"]):
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
            elif (n, ri) in dups:
                errs.append("RULE cover-duplicate: %s: %s is also in %s"
                            % (where, case, dups[(n, ri)]))
            else:
                seen[case] = where
        status = {f[0]: f[1] for _, f, _w in s["results"]}
        named = set()
        for ln, f, whole in s["refusals"] or ():
            where = "%s refusals.tsv:%d" % (n, ln)
            if not whole or not f[0] or not f[1].strip():
                errs.append("RULE refusal-case: %s: a row needs a case and "
                            "its refusal" % where)
            elif f[0] not in status:
                errs.append("RULE refusal-case: %s: %s is not in the slice's "
                            "results.tsv" % (where, f[0]))
            elif status[f[0]] == "Lost":
                errs.append("RULE refusal-case: %s: %s is Lost (it has no "
                            "log)" % (where, f[0]))
            elif f[0] in named:
                errs.append("RULE refusal-case: %s: %s is named twice"
                            % (where, f[0]))
            named.add(f[0])
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


def case_refusals(path):
    """{case: cause} of a merged chunk log (see --per-case), {} if the file
    is missing."""
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError:
        return {}
    out = {}
    cur = None
    for ln in lines:
        m = CASE_LINE.search(ln)
        if m:
            cur = m.group(1)
            continue
        m = MALI_REFUSAL.search(ln)
        if m and cur is not None and cur not in out:
            why = (m.group(2) or "errno").strip()
            out[cur] = ("errno:" + m.group(1) if why == "errno"
                        else "refused:" + normalize(why))
    return out


def refusal_causes(s):
    """{case: cause} from the slice's refusals/<label>.txt files."""
    out = {}
    for lab in sorted({f[1] for _, f, _w in s["chunks"]}):
        out.update(case_refusals(
            os.path.join(s["path"], "refusals", lab + ".txt")))
    return out


def refusal_cause(text):
    """The cause a refusals.tsv text gives: the shader stage and the
    compiler's own words, without the "Mali(...)" around them."""
    m = SHADER_REFUSAL.search(text)
    if not m:
        return "refused: " + text
    why = m.group(2)
    if why.startswith("Mali("):
        why = why[len("Mali("):]
        if why.endswith(")"):
            why = why[:-1]
    return "refused, %s shader: %s" % (m.group(1), why)


def refusal_map(s):
    """case -> refusal text of a slice with refusals.tsv, else None."""
    if s["refusals"] is None:
        return None
    return {f[0]: f[1] for _, f, _w in s["refusals"]}


def tally(slices, per_case=False):
    by_status = {}
    by_cause = {}
    ordered, superseded, _d, _e = resolve(slices)
    for s in ordered:
        ex = chunk_exit_lines(s)
        own = refusal_map(s)
        pc = refusal_causes(s) if per_case else {}
        for i, (_, f, _w) in enumerate(s["results"]):
            if i in superseded[s["name"]]:
                continue
            st = f[1]
            by_status[st] = by_status.get(st, 0) + 1
            if st != "Pass":
                if own is not None and f[0] in own:
                    c = refusal_cause(own[f[0]])
                elif f[0] in pc:
                    c = pc[f[0]]
                elif own is None:
                    c = cause(st, f[2], ex[i])
                else:
                    c = cause(st, f[2], "")
                k = (st, c)
                by_cause[k] = by_cause.get(k, 0) + 1
    return by_status, by_cause


def status_order(by_status):
    return [s for s in STATUSES if s in by_status] + \
        sorted(s for s in by_status if s not in STATUSES)


def render_text(slices, per_case=False):
    by_status, by_cause = tally(slices, per_case)
    out = ["slices: %d" % len(slices)]
    for st in status_order(by_status):
        out.append("%-22s %d" % (st, by_status[st]))
    out.append("%-22s %d" % ("TOTAL", sum(by_status.values())))
    for (st, c), n in sorted(by_cause.items(), key=lambda x: (-x[1], x[0])):
        out.append("cause %-14s %6d  %s" % (st, n, c))
    return "\n".join(out) + "\n"


def render_markdown(slices, per_case=False):
    by_status, by_cause = tally(slices, per_case)
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


def block_slices(blk):
    """The slice names of a block's "Slices:" line, or None without one."""
    m = re.search(r"^Slices: (.+)$", blk, re.M)
    if not m:
        return None
    names = re.findall(r"`([^`]*)`", m.group(1))
    if not names or any(not re.fullmatch(r"[A-Za-z0-9._-]+", n) or
                        n in (".", "..") for n in names):
        return None
    return names


def check_doc(doc, slices, log=None):
    try:
        with open(doc, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        raise Fatal("%s: %s" % (doc, e.strerror))
    blk = doc_block(text)
    if blk is None:
        return ["RULE doc-block: %s has no %s ... %s block" % (doc, BEGIN, END)]
    if log is not None:
        names = block_slices(blk)
        if names is None:
            return ["RULE doc-slices: %s: the block's \"Slices:\" line names "
                    "no slice directory" % doc]
        slices = [load_slice(os.path.join(log, n)) for n in names]
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
    own = os.path.join(FIXTURES, "refusals", "2026-11-09-t6-gles2-S1-00000000")
    chk("refusals.tsv: the slices still cover the mustpass list",
        run("--check-cover", "--no-git", mp, own, good[1])[0] == 0)
    rc, o, _e = run("--tally", "--no-git", own, good[1])
    chk("refusals.tsv: a named case takes its own refusal as the cause",
        rc == 0 and re.search(r"^cause Fail +1  refused, fragment shader: "
                              r"Valhall: an integer or boolean value is not "
                              r"lowered yet$", o, re.M) is not None)
    chk("refusals.tsv: a link error's refusal reads the same way",
        re.search(r"^cause NotSupported +1  refused, vertex shader: Valhall: "
                  r"a loop in a vertex shader is not lowered yet$", o, re.M)
        is not None)
    chk("refusals.tsv: a case it does not name keeps dEQP's text, not the "
        "chunk's first refusal",
        re.search(r"^cause Fail +1  Image comparison failed$", o, re.M)
        is not None and "refused:this" not in o)
    must("refusals.tsv naming a case of no result", "refusal-case",
         "--check-cover", "--no-git", mp, b("refusalunknown"), good[1])
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
    chk("check-doc --log: the block's own slices, read from the directory",
        run("--check-doc", os.path.join(FIXTURES, "doc-current.md"),
            "--log", g)[0] == 0)
    must("check-doc --log: stale doc block", "doc-stale", "--check-doc",
         os.path.join(FIXTURES, "doc-stale.md"), "--log", g)
    must("check-doc --log: a block without a Slices line", "doc-slices",
         "--check-doc", os.path.join(FIXTURES, "doc-noslices.md"), "--log", g)
    rc, _o, e = run("--check-doc", os.path.join(FIXTURES, "doc-current.md"),
                    "--log", bad)
    chk("check-doc --log: a named slice the directory lacks exits 2",
        rc == 2 and "meta.txt" in e)
    # reruns: a later slice replaces the Lost rows of the one it names
    rr = os.path.join(FIXTURES, "rerun", "2026-11-08-t6-gles2-S5-00000000")
    lostd = os.path.join(FIXTURES, "lost")
    rc, _o, _e = run("--check-cover", "--no-git", mp, good[0], lostd, rr)
    chk("rerun pair: Lost rows replaced, cover holds", rc == 0)
    chk("rerun pair: argument order does not matter",
        run("--check-cover", "--no-git", mp, rr, lostd, good[0])[0] == 0)
    rc, o, _e = run("--tally", "--no-git", good[0], lostd, rr)
    chk("rerun tally: Lost rows gone, total 8, later status counted",
        rc == 0 and re.search(r"^TOTAL +8$", o, re.M) is not None
        and re.search(r"^Pass +4$", o, re.M) is not None
        and re.search(r"^Lost ", o, re.M) is None)
    must("rerun of a case that is not Lost", "cover-duplicate",
         "--check-cover", "--no-git", mp, good[0], lostd, b("rerunnonlost"))
    must("rerun of a slice not given", "rerun-target", "--check-cover",
         "--no-git", mp, good[0], lostd, b("reruntarget"))
    must("plain duplicate still fails beside a rerun", "cover-duplicate",
         "--check-cover", "--no-git", mp, good[0], lostd, rr, b("dup"))
    # reruns with a list: one slice takes over the Lost cases of two
    r2 = os.path.join(FIXTURES, "rerun2")
    r2x, r2y, r2z = (os.path.join(r2, "2026-11-08-t6-gles2-%s-00000000" % k)
                     for k in ("S2", "S3", "S5"))
    chk("rerun list: Lost rows of both slices replaced, cover holds",
        run("--check-cover", "--no-git", mp, r2x, r2y, r2z)[0] == 0)
    chk("rerun list: argument order does not matter",
        run("--check-cover", "--no-git", mp, r2z, r2y, r2x)[0] == 0)
    rc, o, _e = run("--tally", "--no-git", r2z, r2x, r2y)
    chk("rerun list tally: total 8, 7 Pass, 1 Fail, no Lost",
        rc == 0 and re.search(r"^TOTAL +8$", o, re.M) is not None
        and re.search(r"^Pass +7$", o, re.M) is not None
        and re.search(r"^Fail +1$", o, re.M) is not None
        and re.search(r"^Lost ", o, re.M) is None)
    must("rerun list that misses a slice: its Lost cases are duplicates",
         "cover-duplicate", "--check-cover", "--no-git", mp, r2x, r2y,
         b("rerun2miss"))
    must("rerun list with a slice not given", "rerun-target", "--check-cover",
         "--no-git", mp, r2x, r2y, b("rerun2unknown"))
    must("rerun list naming the slice itself", "rerun-target",
         "--check-cover", "--no-git", mp, r2x, r2y, b("rerun2self"))
    # replaces: a later slice replaces whole chunks, whatever their status
    rp = os.path.join(FIXTURES, "replaces")

    def rpair(k):
        return [os.path.join(rp, k, "2026-11-08-t6-gles2-%s-00000000" % s)
                for s in ("S1", "S2")]
    rc, _o, _e = run("--check-cover", "--no-git", mp, *rpair("ok"))
    chk("replaces: a chunk of Fail rows replaced, cover holds", rc == 0)
    chk("replaces: argument order does not matter",
        run("--check-cover", "--no-git", mp, *reversed(rpair("ok")))[0] == 0)
    rc, o, _e = run("--tally", "--no-git", *rpair("ok"))
    chk("replaces tally: total 8, 5 Pass, 2 Fail, 1 Lost, replaced rows gone",
        rc == 0 and re.search(r"^TOTAL +8$", o, re.M) is not None
        and re.search(r"^Pass +5$", o, re.M) is not None
        and re.search(r"^Fail +2$", o, re.M) is not None
        and re.search(r"^Lost +1$", o, re.M) is not None)
    rc, o, _e = run("--tally", "--markdown", "--no-git", *rpair("ok"))
    chk("replaces markdown: counts of the replaced tree",
        rc == 0 and "| **Total** | **8** |" in o and "| Pass | 5 |" in o)
    must("replaces: a case the replacing chunk lacks", "replace-cover",
         "--check-cover", "--no-git", mp, *rpair("miss"))
    must("replaces: a slice not given", "replace-target", "--check-cover",
         "--no-git", mp, *rpair("absent"))
    rc, _o, _e = run("--check-cover", "--no-git", mp, *rpair("combo"))
    chk("replaces with reruns: Lost row rerun and a chunk replaced", rc == 0)
    rc, o, _e = run("--tally", "--no-git", *rpair("combo"))
    chk("replaces with reruns tally: total 8, 6 Pass, 2 Fail, no Lost",
        rc == 0 and re.search(r"^TOTAL +8$", o, re.M) is not None
        and re.search(r"^Pass +6$", o, re.M) is not None
        and re.search(r"^Fail +2$", o, re.M) is not None
        and re.search(r"^Lost ", o, re.M) is None)
    rc, _o, e = run("--check-cover", "--no-git", mp, os.path.join(FIXTURES, "nope"))
    chk("unreadable slice exits 2", rc == 2)
    # --per-case: the refusal line inside a case's output is its cause
    pc = os.path.join(FIXTURES, "percase", "2026-11-08-t6-gles2-S1-00000000")
    rc, o, _e = run("--tally", "--no-git", pc)
    chk("per-case fixture, default tally: chunk-wide causes only",
        rc == 0 and "NOT_BUILT" not in o and "errno:" not in o
        and re.search(r"Fail +1 +Image comparison", o) is not None)
    rc, o, _e = run("--tally", "--per-case", "--no-git", pc)
    chk("per-case: a Fail takes its first refusal line, not the decoy",
        re.search(r"Fail +1 +refused:NOT_BUILT_BLEND_SHADER", o) is not None
        and "DECOY" not in o and "NOT_BUILT_OTHER" not in o)
    chk("per-case: a bare errno reason splits by errno",
        re.search(r"ResourceError +1 +errno:-12", o) is not None)
    chk("per-case: a case without a line keeps its cause",
        re.search(r"NotSupported +1 +not supported", o) is not None)
    chk("per-case: a chunk without a refusals file falls back",
        re.search(r"InternalError +1 +refused:blend_equation_separate", o)
        is not None)
    chk("per-case: totals unchanged", re.search(r"^TOTAL +6$", o, re.M)
        is not None)
    rc, o, _e = run("--tally", "--per-case", "--markdown", "--no-git", pc)
    chk("per-case: --markdown takes it too", rc == 0 and "errno:-12" in o)
    chk("per-case: --check-doc is unchanged", run("--check-doc",
        os.path.join(FIXTURES, "doc-current.md"), *good)[0] == 0)
    # lines made from the back end's format string, "yakogl-mali-refusal:
    # {errno} {why}" (src/mali/backend.rs refusal_line): a refusal, the OSAL's
    # own "yakogl: " in front, a -EINVAL non-refusal named by its message, a
    # long reason cut at OSAL_LOG_LINE_MAX
    pr = os.path.join(FIXTURES, "percase-real", "2026-11-08-t6-gles2-S2-00000000")
    rc, o, _e = run("--tally", "--per-case", "--no-git", pr)
    chk("per-case, backend format: a refusal line with a negative errno",
        rc == 0 and re.search(r"Fail +1 +refused:NOT_BUILT_BLEND_SHADER", o)
        is not None)
    chk("per-case, backend format: -EINVAL named by its message, OSAL prefix",
        re.search(r"ResourceError +1 +refused:mali frame: a u# index buffer",
                  o) is not None)
    chk("per-case, backend format: an ENOMEM message cut short is kept",
        re.search(r"Fail +1 +refused:mali arena: out of memory", o) is not None)
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
    per_case = False
    main_ref = "origin/main"
    log = None
    args = []
    mode = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--no-git":
            git = False
        elif a == "--markdown":
            markdown = True
        elif a == "--per-case":
            per_case = True
        elif a == "--main-ref":
            i += 1
            if i >= len(argv):
                print("--main-ref needs a ref", file=sys.stderr)
                return 2
            main_ref = argv[i]
        elif a == "--log":
            i += 1
            if i >= len(argv):
                print("--log needs a directory", file=sys.stderr)
                return 2
            log = argv[i]
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
            sys.stdout.write(render_markdown(s, per_case) if markdown
                             else render_text(s, per_case))
            return 0
        if mode == "--check-doc" and log is not None and len(args) == 1:
            return report(check_doc(args[0], [], log))
        if mode == "--check-doc" and log is None and len(args) >= 2:
            return report(check_doc(args[0], [load_slice(p) for p in args[1:]]))
    except Fatal as e:
        print("mali-deqp-tally: %s" % e, file=sys.stderr)
        return 2
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
