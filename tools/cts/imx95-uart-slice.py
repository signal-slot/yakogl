#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Turn a Verdin iMX95 dEQP-GLES2 session's console logs into a Mali slice.

  python3 -I tools/cts/imx95-uart-slice.py [--tree TREE] [--mustpass FILE]
                                           [--commit SHA] [--session S]
                                           [--started T] [--ended T] SESSION

SESSION is tools/cts/log/<date>-imx95-gles2-<session>-<sha8>/ of a Zephyr session
on the Verdin iMX95 (Mali-G310), every row of tools/cts/mali-gles2-tree.txt
one image started from U-Boot (tools/cts/README.md, the Verdin iMX95
paragraphs). It holds

  console/<label>.log  the row's UART console, as captured (NULs, CRs and a
                       leading "HH:MM:SS " stamp are dropped when read)
  rows.tsv             the session's rows table as tools/cts/imx95-rows-tsv.py
                       writes it: row label cases status seconds lifetime_s
                       bbm exit_line. Read here: label; cases (the summary's
                       "cases begun", or '-'), which may not exceed the row's
                       mustpass count plus its extra cases (extra.tsv); status (done | hang | lifetime |
                       noboot); seconds ('-' allowed); exit_line (used when
                       the console has no end line). row (the label's number
                       within its session) must be a count; chunks.tsv gets
                       the tree's row number, as the T6's slices have it.

and gets, in the same directory, the files tools/cts/mali-deqp-tally.py,
mali-family-tree.py and mali-fail-triage.py read (the NanoPC-T6's slice, as
tools/cts/mali-deqp-run.sh writes it):

  results.tsv   case status detail, one line per mustpass case of each row,
                rows in tree order, cases in mustpass order. A case with a
                "deqp: <Status> <case>" line has its last status; detail is
                '-' (the console carries no dEQP Result text). A case without
                one is Lost, with the reason the tally's vocabulary has:
                  chunk-hang   the case in progress when the run ended in a
                               WDOG3 reset (status hang) or the lifetime
                               (status lifetime): it began and has no result
                  chunk-crash  the case in progress when the run ended
                               without the sample's summary (dEQP's fatal),
                               or a case before the last result
                               that began and never had one (a hole the
                               summary's "without" count accounts for)
                  not-run      the cases after the one in progress; and the
                               holes of a log whose summary counts more
                               results than its lines (the capture lost
                               them: rerun them)
                The case in progress is the first case in mustpass order
                after the last one with a result (dEQP runs a filter in tree
                order, which is mustpass order).
  chunks.tsv    row label cases status seconds kmsg resets ssh_after
                boot_same exit_line, the T6's columns. status: done (the
                sample's summary, no fatal), hang (rows.tsv status hang or
                lifetime), crash (status done without the summary, or
                dEQP's fatal), lost (no console, or one without Zephyr
                output: the image never ran; all its cases Lost not-run). seconds: rows.tsv's (0 when
                '-'). kmsg, resets, ssh_after, boot_same: '-' (Zephyr has
                no kernel log, the board is never reached over ssh during a
                row and every row is its own boot; mali-deqp-tally.py
                accepts '-' there in a Verdin iMX95 slice, board=verdin-imx95). exit_line: the
                board's end line (verdin_imx95: ...: PSCI SYSTEM_RESET at N ms
                or the WDOG3 line; else rows.tsv's exit_line; else '-'), then "first_refused=" and the
                row's first refusal (a compile or link refusal's text, or a
                yakogl-mali-refusal line's reason; '-' for none), the field
                the T6's yakogl-mali-exit line has and mali-fail-triage.py
                reads for the chunk's non-Pass cases, as on the T6.
  refusals.tsv  case refusal: for a case with a result, the first
                glCompileShader(N) / glLinkProgram(N) console line in that
                case whose info log says "refused", without the call and the
                "[<stage> stage, N bytes]" tail (the T6's text, which comes
                from the same info log, cut at the OSAL's 256-byte line).
  refusals/<label>.txt  the row's Zephyr console (from its first Zephyr line
                on; U-Boot's lines are left out) with a "Test case '<case>'.."
                line in front of each case's lines: the console prints a
                case's lines before its "deqp: <Status>" line, dEQP's stdout
                on the T6 after its "Test case" line, and
                mali-deqp-tally.py --per-case gives a "yakogl-mali-refusal:"
                line to the Test case line before it. Only a row that ran.
  extra.tsv     label case status, only when there is one: a case the console
                has a result for that the row's filter matches and the
                mustpass list (--mustpass) lacks (the image's dEQP tree has
                it, the T6's case list does not). It is in no other file of
                the slice, so the tally and the T6 diff stay on the mustpass
                cases; its console lines stay in refusals/<label>.txt under
                a "# extra case" line. A console case the row's filter does
                not match is an error (the wrong image).
  meta.txt      only if absent: commit, session, rows, tree,
                boot_id_before=- boot_id_after=- (no Linux boot inside the
                session), kernel=zephyr, lib=<the image's "vi95_deqp: build"
                describe>, started, ended (--started/--ended, else '-'),
                board=verdin-imx95, os=zephyr, route=u-boot-go. The session
                (--session, else the name's) must be one of
                mali-deqp-tally.py's iMX95 sessions (S1a, S1b, S2, S3a, S3b,
                S4): else exit 1, nothing written.
  <file>.license  beside each file written.

The commit is --commit, else the session name's <sha8> or the build line's
-g<hex>, resolved to 40 hex by git in this repository. Every row's build line
must name the same build (one origin/main commit), its hash must be the
commit's, every console case must be one of its row's cases, and rows.tsv's
cases may not exceed the tree's count: else exit 1, nothing written. A
status in rows.tsv that the console contradicts (imx95-rows-tsv.py's own
rule applied to console/<label>.log) is a warning (rows.tsv wins). The console logs must already be redacted (no identifier is taken
from U-Boot's part, which is dropped); run mali-slice-redact.py over the
slice as for the T6's. Exit 0 written, 1 an inconsistent session, 2 usage or
an unreadable file.
"""
import importlib.util
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CHUNK_COLS = ("row", "label", "cases", "status", "seconds", "kmsg", "resets",
              "ssh_after", "boot_same", "exit_line")
BUILD = re.compile(r"\bvi95_deqp: build (\S+)")
ZEPHYR_START = re.compile(r"\bvi95_deqp: build |\*\*\* Booting Zephyr")
END_RUN_OVER = re.compile(r"verdin_imx95: board: run over: PSCI SYSTEM_RESET.*")
END_LIFETIME = re.compile(r"verdin_imx95: lifetime over: PSCI SYSTEM_RESET.*")
END_WDOG3 = re.compile(r"verdin_imx95: WDOG3 feeding stopped at .*")
SHADER_LOG = re.compile(r"\bgl(?:CompileShader|LinkProgram)\(\d+\): (.*)$")
STAGE_TAIL = re.compile(r"\s*\[(?:vertex|fragment) stage, \d+ bytes\]\s*$")
REFUSED_WORD = re.compile(r"\brefused\b")
LICENSE = ("SPDX-License-Identifier: Apache-2.0\n"
           "SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n"
           "Results of our Verdin iMX95 dEQP-GLES2 run, written by "
           "tools/cts/imx95-uart-slice.py.\n")


class Fatal(Exception):
    def __init__(self, msg, code=1):
        Exception.__init__(self, msg)
        self.code = code


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, filename))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# The console parser is deqp-uart-tally.py's (the deqp sample's "deqp: <Status>
# <case>" line, the summary and done lines, the stamp a recorder adds); the
# tree reader is check-gles3-tree.py's, as mali-deqp-run.sh uses it.
uart = load("deqp_uart_tally", "deqp-uart-tally.py")
cgt = load("check_gles3_tree", "check-gles3-tree.py")
tally_mod = load("mali_deqp_tally", "mali-deqp-tally.py")
# rows.tsv's columns, vocabularies and status rule are imx95-rows-tsv.py's.
rows_mod = load("imx95_rows_tsv", "imx95-rows-tsv.py")
ROWS_COLS = tuple(rows_mod.COLUMNS)


def read(path):
    try:
        with open(path, encoding="utf-8", errors="replace", newline="") as f:
            return f.read()
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror), 2)


def field(s, n=240):
    s = re.sub(r"\s+", " ", s).strip()
    return s[:n] if s else "-"


def read_rows(path):
    lines = read(path).replace("\r", "").split("\n")
    while lines and lines[-1] == "":
        lines.pop()
    if not lines or tuple(lines[0].split("\t")) != ROWS_COLS:
        raise Fatal("%s: header is not %s" % (path, "\t".join(ROWS_COLS)))
    out = []
    for n, ln in enumerate(lines[1:], 2):
        f = ln.split("\t")
        where = "%s:%d" % (path, n)
        if len(f) != len(ROWS_COLS):
            raise Fatal("%s: %d fields, not %d" % (where, len(f), len(ROWS_COLS)))
        r = dict(zip(ROWS_COLS, f))
        if not cgt.LABEL_OK.match(r["label"]):
            raise Fatal("%s: label %r is not [A-Za-z0-9]+" % (where, r["label"]))
        for k in ("row", "lifetime_s"):
            if not r[k].isdigit():
                raise Fatal("%s: %s %r is not a count" % (where, k, r[k]))
        for k in ("cases", "seconds"):
            if not (r[k].isdigit() or r[k] == "-"):
                raise Fatal("%s: %s %r is not a count or -" % (where, k, r[k]))
        if r["status"] not in rows_mod.STATUSES:
            raise Fatal("%s: status %r is not %s"
                        % (where, r["status"], "|".join(rows_mod.STATUSES)))
        if r["bbm"] not in rows_mod.BBMS:
            raise Fatal("%s: bbm %r is not %s"
                        % (where, r["bbm"], "|".join(rows_mod.BBMS)))
        if any(o["label"] == r["label"] for o in out):
            raise Fatal("%s: label %s is listed twice" % (where, r["label"]))
        out.append(r)
    if not out:
        raise Fatal("%s: no rows" % path)
    return out


def console_lines(text):
    """The console's lines, cleaned as deqp-uart-tally.py cleans them; a last
    result line the capture cut mid-line is dropped (its name may be cut)."""
    raw = text.replace("\0", "").replace("\r", "").split("\n")
    if raw and raw[-1] == "":
        raw.pop()
    elif raw and uart.LINE.match(uart.STAMP.sub("", raw[-1], count=1)):
        raw.pop()
    return [uart.STAMP.sub("", ln, count=1) for ln in raw]


def console_end(lines):
    """(end, line) the console itself shows: its last board end line."""
    end, line = "none", "-"
    for ln in lines:
        for pat, name in ((END_RUN_OVER, "run-over"), (END_LIFETIME, "lifetime")):
            m = pat.search(ln)
            if m:
                end, line = name, m.group(0)
        m = END_WDOG3.search(ln)
        if m:
            end = "lifetime" if "(lifetime over)" in ln else "wdog3"
            line = m.group(0)
    return end, line


def shader_refusal(line):
    m = SHADER_LOG.search(line)
    if not m or not REFUSED_WORD.search(m.group(1)):
        return None
    return STAGE_TAIL.sub("", m.group(1))


def console_status(path):
    """The status imx95-rows-tsv.py gives the console capture at path."""
    return rows_mod.status_and_exit_line(rows_mod.parse_capture(path))[0]


def convert_row(r, cases, text, cpath=None, rx=None):
    """(chunk fields, [(case, status, detail)], {case: refusal},
    refusal file text or None, build or None, warnings, [(extra case,
    status)]) of one row. rx is the row's filter as a regex: a console case
    it matches that is not one of cases (the mustpass list lacks it) is an
    extra case."""
    warn = []
    begun = int(r["cases"]) if r["cases"].isdigit() else None
    lines = console_lines(text) if text is not None else []
    z = next((i for i, ln in enumerate(lines) if ZEPHYR_START.search(ln)), None)
    seconds = r["seconds"] if r["seconds"].isdigit() else "0"
    lab = r["label"]
    if z is None:
        if r["status"] != "noboot":
            warn.append("%s: rows.tsv says status %s, the console has no Zephyr line"
                        % (lab, r["status"]))
        chunk = [r["row"], lab, str(len(cases)), "lost", seconds, "-", "-", "-", "-", "-"]
        if text is not None and lines:
            warn.append("%s: the console has no Zephyr line; the row is lost" % lab)
        if begun is not None and begun > len(cases):
            raise Fatal("%s has %d mustpass cases in the tree, rows.tsv says %d "
                        "began (the wrong image?)" % (lab, len(cases), begun))
        return chunk, [(c, "Lost", "not-run") for c in cases], {}, None, None, warn, []
    zl = lines[z:]
    build = next((m.group(1) for m in map(BUILD.search, zl) if m), None)
    blocks, pending = [], []
    for ln in zl:
        pending.append(ln)
        m = uart.LINE.match(ln)
        if m:
            blocks.append((m.group(2), m.group(1), pending))
            pending = []
    tail = pending
    got, own = {}, {}
    for case, st, blk in blocks:
        got[case] = st
        own[case] = next((t for t in map(shader_refusal, blk) if t), None)
    pos = {c: i for i, c in enumerate(cases)}
    unknown = [c for c in got if c not in pos and not (rx and rx.fullmatch(c))]
    if unknown:
        raise Fatal("%s: the console has %d case(s) that are not the row's, e.g. %s "
                    "(the wrong image?)" % (lab, len(unknown), unknown[0]))
    extra = [(c, got[c]) for c in got if c not in pos]
    n_got = len(got)
    for c, _st in extra:
        del got[c]
    if extra:
        warn.append("%s: %d case(s) of the row's filter are not in the mustpass list "
                    "and are left out of results.tsv (extra.tsv has them), e.g. %s"
                    % (lab, len(extra), extra[0][0]))
    if begun is not None and begun > len(cases) + len(extra):
        raise Fatal("%s has %d mustpass cases in the tree (and %d extra in the "
                    "console), rows.tsv says %d began (the wrong image?)"
                    % (lab, len(cases), len(extra), begun))
    sc = uart.scan(text)
    summary, tails = sc["summary"], sc["tail"]
    fatal = any(t.startswith("deqp: fatal") for t in tails)
    _end, end_line = console_end(zl)
    if end_line == "-" and r["exit_line"] != "-":
        end_line = r["exit_line"]
    seen = console_status(cpath) if cpath else r["status"]
    if seen != r["status"]:
        warn.append("%s: rows.tsv says status %s, the console says %s"
                    % (lab, r["status"], seen))
    if r["status"] in ("hang", "lifetime"):
        status, reason = "hang", "chunk-hang"
    elif fatal or summary is None:
        status, reason = "crash", "chunk-crash"
    else:
        status, reason = "done", None
    last = max((pos[c] for c in got), default=-1)
    missing = [c for c in cases if c not in got]
    holes = [c for c in missing if pos[c] < last]
    after = [c for c in missing if pos[c] > last]
    lost = {}
    without = summary[2] if summary else 0
    capture_loss = summary is not None and summary[1] > n_got
    for c in holes:
        lost[c] = "not-run" if capture_loss else "chunk-crash"
    if not capture_loss:
        without -= len(holes)
    inprog = None
    if after and (reason or without > 0):
        inprog = after[0]
        lost[inprog] = reason or "chunk-crash"
    for c in after:
        lost.setdefault(c, "not-run")
    res = [(c, got[c], "-") if c in got else (c, "Lost", lost[c]) for c in cases]
    refusals = {c: field(own[c], 600) for c in cases if c in got and own.get(c)}
    out = ["# %s's console from its first Zephyr line, made by "
           "tools/cts/imx95-uart-slice.py: a \"Test case '<case>'..\" line stands "
           "in front of each case's lines" % lab]
    for case, _st, blk in blocks:
        if case in pos:
            out.append("Test case '%s'.." % case)
        else:
            out.append("# extra case (not in the mustpass list): %s" % case)
        out.extend(blk)
    if tail:
        if inprog:
            out.append("Test case '%s'.." % inprog)
        out.extend(tail)
    first = None
    for ln in zl:
        first = shader_refusal(ln)
        if first is None:
            m = tally_mod.MALI_REFUSAL.search(ln)
            first = m.group(2) if m and m.group(2) else None
        if first:
            break
    exit_line = "%s first_refused=%s" % (end_line, field(first or "-", 160))
    chunk = [r["row"], lab, str(len(cases)), status, seconds, "-", "-", "-", "-",
             field(exit_line, 400)]
    return chunk, res, refusals, "\n".join(out) + "\n", build, warn, extra


def git(*args):
    try:
        p = subprocess.run(["git"] + list(args), cwd=ROOT, capture_output=True, text=True)
    except OSError:
        return None
    return p.stdout.strip() if p.returncode == 0 else None


def resolve_commit(given, name, builds):
    """40 hex: --commit, else the first of the build's -g<hex> and the
    name's <sha8> git resolves; each of those must be its prefix."""
    m = re.search(r"-([0-9a-f]{8})$", name)
    prefixes = [("the session name", m.group(1))] if m else []
    for b in builds:
        m = re.search(r"-g([0-9a-f]{7,40})\b", b)
        if m:
            prefixes.append(("the images' build " + b, m.group(1)))
    commit = None
    for c in ([given] if given else []) + [p for _w, p in prefixes]:
        commit = c if re.fullmatch(r"[0-9a-f]{40}", c) else \
            git("rev-parse", "--verify", "--quiet", c + "^{commit}")
        if commit:
            break
    if not commit or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise Fatal("%s: cannot name the commit as 40 hex (give --commit)" % name)
    for what, p in prefixes:
        if not commit.startswith(p):
            raise Fatal("%s: %s is not commit %s" % (name, what, commit[:8]))
    return commit


def rel(path):
    a = os.path.abspath(path)
    return os.path.relpath(a, ROOT) if a.startswith(ROOT + os.sep) else path


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)
    with open(path + ".license", "w", encoding="utf-8", newline="") as f:
        f.write(LICENSE)


def convert(session, tree, mustpass, commit=None, sess=None, started="-", ended="-"):
    session = session.rstrip("/")
    name = os.path.basename(session)
    rows = read_rows(os.path.join(session, "rows.tsv"))
    table = cgt.read_table(read(tree))
    index = {lab: i for i, (lab, _n, _f) in enumerate(table, 1)}
    names = [ln.strip() for ln in read(mustpass).splitlines()
             if ln.strip() and not ln.startswith("#")]
    blob = "\n".join(names)
    warn = []
    out = []
    for r in rows:
        if r["label"] not in index:
            raise Fatal("%s: row %s is not in %s" % (name, r["label"], tree))
        i = index[r["label"]]
        r = dict(r, row=str(i))
        rx = cgt.row_regex(table[i - 1][2])
        cases = [m.group(0) for m in rx.finditer(blob)]
        cpath = os.path.join(session, "console", r["label"] + ".log")
        text = read(cpath) if os.path.exists(cpath) else None
        if text is None:
            warn.append("%s: no console/%s.log; the row is lost" % (r["label"], r["label"]))
        try:
            conv = convert_row(r, cases, text, cpath if text is not None else None, rx)
        except Fatal as e:
            raise Fatal("%s: %s" % (name, e), e.code)
        warn += conv[5]
        out.append((i, conv))
    out.sort(key=lambda x: x[0])
    builds = sorted({c[4] for _i, c in out if c[4]})
    if len(builds) > 1:
        raise Fatal("%s: the rows name %d builds (%s): one session is one commit"
                    % (name, len(builds), ", ".join(builds)))
    meta_path = os.path.join(session, "meta.txt")
    if not os.path.exists(meta_path):
        if sess is None:
            m = re.search(r"-gles2-(S[A-Za-z0-9]+)-", name)
            if not m:
                raise Fatal("%s: no session in the name (give --session)" % name, 2)
            sess = m.group(1)
        if sess not in tally_mod.IMX95_SESSIONS:
            raise Fatal("%s: session %r is not %s (mali-deqp-tally.py's RULE "
                        "session-name)" % (name, sess, "|".join(tally_mod.IMX95_SESSIONS)))
        c = resolve_commit(commit, name, builds)
        meta = ["commit=" + c, "session=" + sess,
                "rows=%d-%d" % (out[0][0], out[-1][0]), "tree=" + rel(tree),
                "boot_id_before=-", "boot_id_after=-", "kernel=zephyr",
                "lib=" + (("vi95_deqp " + builds[0]) if builds else "-"),
                "started=" + started, "ended=" + ended,
                "board=verdin-imx95", "os=zephyr", "route=u-boot-go"]
    else:
        meta = None
    chk = ["\t".join(CHUNK_COLS)]
    res = ["case\tstatus\tdetail"]
    ref = ["case\trefusal"]
    files = []
    ext = ["label\tcase\tstatus"]
    for _i, (chunk, rr, refs, rtext, _b, _w, extra) in out:
        ext.extend("%s\t%s\t%s" % (chunk[1], c, st) for c, st in extra)
        chk.append("\t".join(chunk))
        res.extend("\t".join(x) for x in rr)
        ref.extend("%s\t%s" % (c, refs[c]) for c, _s, _d in rr if c in refs)
        if rtext is not None:
            files.append((chunk[1], rtext))
    if meta is not None:
        write(meta_path, "\n".join(meta) + "\n")
    write(os.path.join(session, "chunks.tsv"), "\n".join(chk) + "\n")
    write(os.path.join(session, "results.tsv"), "\n".join(res) + "\n")
    write(os.path.join(session, "refusals.tsv"), "\n".join(ref) + "\n")
    ext_path = os.path.join(session, "extra.tsv")
    if len(ext) > 1:
        write(ext_path, "\n".join(ext) + "\n")
    else:
        for q in (ext_path, ext_path + ".license"):
            if os.path.exists(q):
                os.remove(q)
    if files:
        os.makedirs(os.path.join(session, "refusals"), exist_ok=True)
    for lab, t in files:
        write(os.path.join(session, "refusals", lab + ".txt"), t)
    tally = {}
    for ln in res[1:]:
        st = ln.split("\t")[1]
        tally[st] = tally.get(st, 0) + 1
    for w in warn:
        print("imx95-uart-slice: warning: " + w, file=sys.stderr)
    print("%s: %d rows, %d cases: %s; %d refusals%s" % (
        name, len(out), len(res) - 1,
        ", ".join("%d %s" % (v, k) for k, v in sorted(tally.items(), key=lambda kv: (-kv[1], kv[0]))),
        len(ref) - 1, "" if meta is not None else " (meta.txt kept)")
        + ("; %d extra case(s) outside the mustpass list in extra.tsv" % (len(ext) - 1)
           if len(ext) > 1 else ""))


USAGE = ("usage: imx95-uart-slice.py [--tree TREE] [--mustpass FILE] [--commit SHA] "
         "[--session S] [--started T] [--ended T] SESSION")


def main(argv):
    opts = {"--tree": os.path.join(HERE, "mali-gles2-tree.txt"),
            "--mustpass": os.path.join(HERE, "gles2-mustpass.txt"),
            "--commit": None, "--session": None, "--started": "-", "--ended": "-"}
    args = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in opts:
            if i + 1 >= len(argv):
                print(USAGE, file=sys.stderr)
                return 2
            opts[a] = argv[i + 1]
            i += 2
            continue
        if a.startswith("-"):
            print(USAGE, file=sys.stderr)
            return 2
        args.append(a)
        i += 1
    if len(args) != 1 or not os.path.isdir(args[0]):
        print(USAGE, file=sys.stderr)
        return 2
    try:
        convert(args[0], opts["--tree"], opts["--mustpass"], opts["--commit"],
                opts["--session"], opts["--started"], opts["--ended"])
    except Fatal as e:
        print("imx95-uart-slice: %s" % e, file=sys.stderr)
        return e.code
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
