#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The common set's tally: one block per row of a common run.

    tools/cts/common-tally.py <board> <outdir> [--prev <earlier outdir>]
    tools/cts/common-tally.py <board> --row LABEL=<rundir> ... [--prev-row LABEL=<rundir> ...]
    tools/cts/common-tally.py --self-test
    tools/cts/common-tally.py --check-records <dir of the first i.MX8MP common run>

<outdir> is what `tools/board.sh <board> tree tools/cts/common-tree.txt
<outdir>` wrote: tree.txt (the rows, labelled GM SO SV GT) and one run
directory per row, YYYYMMDD-HHMM-<LABEL>-<sha8>. --row names run directories
directly, for runs made one at a time with `board.sh run` (the rows of the
first i.MX8MP common run, 2026-10-04, were).

The verdict of each run is tools/board.sh's own, recomputed from the run's
files (`board.sh <board> tally <rundir>`, the same call board.sh tree ends
with); a verdict that differs from the one INDEX.txt stored is printed beside
it. This tool adds what board.sh's verdict line does not say, per kind:

  glmark2  the GLMARK2 record: score, scenes ran / failed / unsupported, and
           the record's own status (pass or fail)
  slint    the PROBES record's status (pass or fail)
  gl_test  passed of run and the failing case numbers. From the console:
           "gl_test: all: F of T cases failed (P passed; X timed out; ...)"
           (gl_test.c, AM62P and Pi 4) and each "gl_test: case N <name>: FAIL"
           or "v3d_gl_test: case N FAIL" line. From a CASE record (the
           i.MX8MP has no UART): the regression set's passed of run and its
           FIRST failing case only (gc_gl_test main.c: gc_result_case(7, ...,
           CASE_PASS, passed, total) or (first failure, ..., CASE_SET_FAIL,
           passed, total)); the other failing cases are not in the record and
           are printed as unknown, never as none.

Absent stays absent: a row without a run, a run without a result, a field the
record does not carry. Each line names the record words (or the legacy console
result), the GPU resets as board.sh counts them, the image's sha256 and its
build ids (meta.txt, as board.sh read them from the image's build.ninja).

--prev compares with an earlier common run (a tree outdir, or --prev-row
LABEL=<rundir> ...), row by row by label: the verdict, the record words, and
the fields above (glmark2 score, gl_test failing cases added and removed).

The first line is this tool's own build (git describe of its checkout).
Exit status: 0 when every row has a run and every verdict is done; 1 when a row
has no run or a verdict is anything else; 2 for a usage error.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
BOARD_SH = os.path.join(REPO, "tools", "board.sh")
BOARDS = ("am62p", "pi4", "imx8mp")
RUN_RE = re.compile(r"^[0-9]{8}-[0-9]{4}-(.+)-([0-9a-f]{8}|probe)$")


def describe():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def ytools():
    """yakogl-tools, built once (board.sh's ytools(); exported, so board.sh's children reuse it)"""
    path = os.environ.get("YAKOGL_TOOLS")
    if path:
        return path
    subprocess.run(["cargo", "build", "-q", "-p", "yakogl-tools"], cwd=REPO, check=True)
    meta = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=REPO,
                          capture_output=True, text=True, check=True).stdout
    td = re.search(r'"target_directory":"([^"]*)"', meta).group(1)
    path = os.path.join(td, "debug", "yakogl-tools")
    os.environ["YAKOGL_TOOLS"] = path
    return path


def read_meta(rundir):
    meta = {}
    try:
        with open(os.path.join(rundir, "meta.txt"), encoding="utf-8", errors="replace") as f:
            for line in f:
                k, sep, v = line.rstrip("\n").partition("=")
                if sep:
                    meta[k] = v
    except OSError:
        pass
    return meta


def read_text(path):
    try:
        with open(path, "rb") as f:
            return f.read().decode("utf-8", errors="replace")
    except OSError:
        return None


def board_verdict(board, rundir):
    """board.sh's verdict line for one run, recomputed, and the stored one when it differs"""
    ytools()
    p = subprocess.run([BOARD_SH, board, "tally", rundir], capture_output=True, text=True)
    lines = p.stdout.splitlines()
    vline = next((l for l in lines if l.startswith("board.sh ")), None)
    differs = None
    for l in lines:
        m = re.match(r"\s*DIFFERS from INDEX.txt: (.*)", l)
        if m:
            differs = m.group(1)
    if vline is None:
        return None, None, (p.stderr.strip().splitlines() or ["no output"])[-1]
    return vline, differs, None


def kv(line):
    out = {}
    for tok in line.split():
        k, sep, v = tok.partition("=")
        if sep and k not in out:
            out[k] = v
    return out


# ---- the details per kind ---------------------------------------------------------

GLMARK2_RE = re.compile(r"score (\d+) status (\d+) \(([^)]*)\) scenes ran (\d+) failed (\d+) unsupported (\d+)")
# a GLMARK2 record with GLMARK2_SCENES (record.rs) also names its entries
GLMARK2_SCENES_RE = re.compile(r"; first failed: (.*?); unsupported: (.*)$")
PROBES_RE = re.compile(r"PROBES \(\d+\) tag \d+: status (\S+)")
CASE_RE = re.compile(r"CASE \(\d+\) seq \d+: case (\d+) check (\d+) status (\d+) \(([^)]*)\) "
                     r"got 0x([0-9a-f]{8}) want 0x([0-9a-f]{8})")
# gl_test.c: pr_info("gl_test: all: %d of %u cases failed (%u passed; %u timed out; ...")
ALL_RE = re.compile(r"gl_test: all: (-?\d+) of (\d+) cases failed \((\d+) passed; (\d+) timed out")
# gl_test.c: "gl_test: case N <name>: PASS/FAIL/TIMEOUT"; v3d_gl_test main.c: "v3d_gl_test: case N <rc_name>"
CASE_LINE_RE = re.compile(r"(?<![A-Za-z0-9_])gl_test: case (\d+) [^:\n]*: (PASS|FAIL|TIMEOUT)\b")
V3D_CASE_RE = re.compile(r"v3d_gl_test: case (\d+) (PASS|FAIL|TIMEOUT|SKIP|no such case|\?)\s*$")
# v3d_gl_test main.c verdicts(): printk("v3d_gl_test: %u of %u cases passed (%lld ms)\n", ...)
V3D_SUM_RE = re.compile(r"v3d_gl_test: (\d+) of (\d+) cases passed \(")


def plain_lines(text):
    """the console's lines without NULs, CRs and ANSI escapes (a leading time stamp stays:
    every pattern is a search, not a match)"""
    text = text.replace("\0", "").replace("\r", "")
    text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text)
    return text.split("\n")


def gl_test_from_console(text):
    """passed of run and the failing case numbers, from the console's own lines. The count comes
    from, in this order: gl_test.c's "gl_test: all:" line (the list walker, every board), else
    v3d_gl_test's "N of M cases passed" (a single case on the Pi 4 prints no "all:" line), else a
    single case's own "gl_test: case N <name>: PASS/FAIL/TIMEOUT" line (one case, no walker)."""
    alls, sums = set(), set()
    seen = {}
    for line in plain_lines(text):
        m = ALL_RE.search(line)
        if m:
            alls.add(tuple(int(x) for x in m.groups()))
        m = V3D_SUM_RE.search(line)
        if m:
            sums.add((int(m.group(1)), int(m.group(2))))
        m = CASE_LINE_RE.search(line) or V3D_CASE_RE.search(line)
        if m:
            seen[int(m.group(1))] = m.group(2)
    failing = {c: st for c, st in seen.items() if st not in ("PASS", "SKIP")}
    d = {"kind": "gl_test", "source": "console", "failing": sorted(failing), "passed": None, "total": None}
    names = ",".join(str(c) if failing[c] == "FAIL" else "%d(%s)" % (c, failing[c]) for c in d["failing"])
    if len(alls) > 1 or (not alls and len(sums) > 1):
        # the Pi 4 prints the verdicts twice (UART and the record): identical, or not a result
        d["text"] = "gl_test: the console's summary lines disagree: %s" % sorted(alls or sums)
        return d
    if alls:
        fails, total, passed, timed_out = alls.pop()
        d.update(passed=passed, total=total)
        d["text"] = "gl_test passed %d of %d (%d failed, %d timed out); failing: %s" % (
            passed, total, fails, timed_out, names or "none")
        if len(failing) != fails:
            d["text"] += " [%d named in the console, %d counted by gl_test]" % (len(failing), fails)
        return d
    if sums:
        passed, total = sums.pop()
        d.update(passed=passed, total=total)
        d["text"] = "gl_test passed %d of %d (v3d_gl_test's count; no 'gl_test: all:' line); failing: %s" % (
            passed, total, names or "none")
        return d
    if len(seen) == 1:
        (case, st), = seen.items()
        d.update(passed=int(st == "PASS"), total=1)
        d["text"] = "gl_test case %d alone: %s" % (case, st)
        return d
    d["text"] = "gl_test: no summary line in the console (no result)"
    return d


def detail_from_record(decoded):
    m = GLMARK2_RE.search(decoded)
    if m:
        score, st, stname, ran, failed, unsup = m.groups()
        d = {"kind": "glmark2", "score": int(score), "status": stname, "ran": int(ran),
             "failed": int(failed), "unsupported": int(unsup),
             "text": "glmark2 score %s; scenes ran %s, failed %s, unsupported %s; status %s (%s)"
                     % (score, ran, failed, unsup, st, stname)}
        s = GLMARK2_SCENES_RE.search(decoded)
        if s:
            d["first_failed"], d["unsupported_entries"] = s.groups()
            d["text"] += "; first failed: %s; unsupported: %s" % s.groups()
        return d
    m = PROBES_RE.search(decoded)
    if m:
        return {"kind": "probes", "status": m.group(1),
                "text": "slint probes %s (%s)" % (m.group(1), decoded.strip().split(": ", 1)[-1])}
    m = CASE_RE.search(decoded)
    if m:
        case, check, st, stname, got, want = m.groups()
        case, st, got, want = int(case), int(st), int(got, 16), int(want, 16)
        if st == 1 and case == 7:
            # gc_gl_test main.c: gc_result_case(7, -1, GC_RESULT_CASE_PASS, gl_test_passed, gl_test_total)
            return {"kind": "gl_test", "source": "record", "passed": got, "total": want, "failing": [],
                    "text": "gl_test passed %d of %d (CASE record); failing: none" % (got, want)}
        if st == 6:
            # CASE_SET_FAIL: case = the set's first failure, w2 = passed, w3 = run
            return {"kind": "gl_test", "source": "record", "passed": got, "total": want,
                    "failing": [case], "first_only": True,
                    "text": "gl_test passed %d of %d (CASE record); failing: %d, then unknown "
                            "(the record carries the first failure only)" % (got, want, case)}
        # one case alone (gl_test's CONFIG_*_CASE=N): its own status
        return {"kind": "gl_test", "source": "record", "case": case, "status": stname,
                "passed": 1 if st == 1 else 0, "total": 1, "failing": [] if st == 1 else [case],
                "text": "gl_test case %d alone: %s (check %s, got 0x%08x want 0x%08x)"
                        % (case, stname, check, got, want)}
    return {"kind": "other", "text": decoded.strip().replace("\n", " | ") or "record not decoded"}


def resets_label(record_resets, line_resets):
    """where a reset count comes from: the record's own field when the record carries one
    (yakogl-tools record --line resets=), else board.sh's count of the console's recovery
    lines (its verdict line's resets= without a record value), else absent; never a bare
    number the reader cannot trace"""
    if record_resets not in (None, "absent"):
        return "resets (record) %s" % record_resets
    if line_resets not in (None, "absent"):
        return "resets (console count) %s" % line_resets
    return "resets absent"


def row_summary(board, label, rundir):
    s = {"label": label, "dir": rundir}
    meta = read_meta(rundir)
    s["name"] = meta.get("name", "absent")
    s["image"] = meta.get("image_sha256", "absent")[:16]
    s["build"] = meta.get("image_build", "absent").replace(" ", ",")
    vline, differs, err = board_verdict(board, rundir)
    if vline is None:
        s.update(verdict="no_verdict", rc="absent", result="none", resets="absent",
                 record="absent", differs=None, resets_label="resets absent",
                 detail={"kind": "none", "text": "board.sh tally: %s" % err})
        return s
    v = kv(vline)
    s.update(verdict=v.get("verdict", "absent"), rc=v.get("rc", "absent"),
             result=v.get("result", "none"), resets=v.get("resets", "absent"),
             record=v.get("record", "absent"), differs=differs)
    console = read_text(os.path.join(rundir, "console.txt"))
    record_resets = None
    if s["record"] != "absent":
        p = subprocess.run([ytools(), "record"] + s["record"].split(","), capture_output=True, text=True)
        s["detail"] = detail_from_record(p.stdout + p.stderr)
        p = subprocess.run([ytools(), "record", "--line"] + s["record"].split(","), capture_output=True, text=True)
        record_resets = kv(p.stdout).get("resets")
    elif s["result"].startswith("legacy:gl_test") and console is not None:
        s["detail"] = gl_test_from_console(console)
    else:
        s["detail"] = {"kind": "none", "text": "no result (result=%s)" % s["result"]}
    s["resets_label"] = resets_label(record_resets, s["resets"])
    return s


# ---- the rows of an outdir ----------------------------------------------------------

def tree_rows(outdir):
    """[(label, [run dirs])] in tree.txt's order; the run dirs of a row in name order"""
    tree = os.path.join(outdir, "tree.txt")
    if not os.path.isfile(tree):
        raise SystemExit("common-tally: %s has no tree.txt (not a board.sh tree; use --row)" % outdir)
    names = sorted(os.listdir(outdir))
    rows = []
    with open(tree, encoding="utf-8") as f:
        for line in f:
            label = line.split("\t", 1)[0].strip()
            if not label:
                continue
            dirs = []
            for n in names:
                m = RUN_RE.match(n)
                d = os.path.join(outdir, n)
                if m and m.group(1) == label and os.path.isfile(os.path.join(d, "meta.txt")):
                    dirs.append(d)
            rows.append((label, dirs))
    return rows


def explicit_rows(specs):
    rows = []
    for spec in specs:
        label, sep, d = spec.partition("=")
        if not sep or not label or not os.path.isfile(os.path.join(d, "meta.txt")):
            raise SystemExit("common-tally: --row wants LABEL=<run dir with meta.txt>, not '%s'" % spec)
        rows.append((label, [d.rstrip("/")]))
    return rows


def tally(board, rows):
    """{label: summary of its last run, None without one} and the printed lines"""
    out, lines = {}, []
    for label, dirs in rows:
        if not dirs:
            lines.append("%s no run (absent)" % label)
            out[label] = None
            continue
        for i, d in enumerate(dirs):
            s = row_summary(board, label, d)
            tag = label if len(dirs) == 1 else "%s#%d" % (label, i + 1)
            lines.append("%s %s rc=%s verdict=%s result=%s record=%s; %s; image=%s build=%s"
                         % (tag, s["name"], s["rc"], s["verdict"], s["result"], s["record"],
                            s.get("resets_label", "resets absent"), s["image"], s["build"]))
            lines.append("    %s" % s["detail"]["text"])
            if s["differs"]:
                lines.append("    INDEX.txt stored a different verdict: %s" % s["differs"])
            lines.append("    run %s" % d)
            out[label] = s
    return out, lines


def diff(prev, now):
    lines = []
    for label in list(now) + [l for l in prev if l not in now]:
        a, b = prev.get(label), now.get(label)
        if a is None or b is None:
            lines.append("DIFF %s: %s -> %s" % (label, "run" if a else "no run", "run" if b else "no run"))
            continue
        ch = []
        for k, name in (("verdict", "verdict"), ("result", "result"), ("record", "record"), ("resets_label", "resets")):
            if a.get(k) != b.get(k):
                ch.append("%s: %s -> %s" % (name, a.get(k), b.get(k)))
        da, db = a["detail"], b["detail"]
        if da.get("kind") == db.get("kind") == "glmark2":
            for k in ("score", "ran", "failed", "unsupported", "status"):
                if da[k] != db[k]:
                    ch.append("glmark2 %s %s -> %s" % (k, da[k], db[k]))
        if da.get("kind") == db.get("kind") == "gl_test":
            if (da.get("passed"), da.get("total")) != (db.get("passed"), db.get("total")):
                ch.append("gl_test passed %s of %s -> %s of %s"
                          % tuple("absent" if x is None else x for x in
                                  (da.get("passed"), da.get("total"), db.get("passed"), db.get("total"))))
            fa, fb = set(da.get("failing", [])), set(db.get("failing", []))
            if fa != fb:
                ch.append("failing +%s -%s" % (sorted(fb - fa), sorted(fa - fb)))
                if da.get("first_only") or db.get("first_only"):
                    ch.append("(a CASE record names its first failure only)")
        if a["image"] != b["image"]:
            ch.append("image %s -> %s" % (a["image"], b["image"]))
        lines.append("DIFF %s: %s" % (label, "; ".join(ch) if ch else
                                      "same (verdict %s, record %s)" % (b["verdict"], b["record"])))
    return lines


USAGE = ("usage: common-tally.py <am62p|pi4|imx8mp> <outdir> | --row LABEL=<rundir>... "
         "[--prev <outdir> | --prev-row LABEL=<rundir>...] | --self-test")


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if argv[:1] == ["--check-records"] and len(argv) == 2:
        return check_records(argv[1])
    args = list(argv)
    rows_spec, prev_spec, prev_dir, pos = [], [], None, []
    while args:
        a = args.pop(0)
        if a == "--row" and args:
            rows_spec.append(args.pop(0))
        elif a == "--prev" and args:
            prev_dir = args.pop(0)
        elif a == "--prev-row" and args:
            prev_spec.append(args.pop(0))
        elif a.startswith("--"):
            print(USAGE, file=sys.stderr)
            return 2
        else:
            pos.append(a)
    # a board, then exactly one of: an outdir, or --row specs
    if not pos or pos[0] not in BOARDS or len(pos) > 2 or (len(pos) == 2) == bool(rows_spec):
        print(USAGE, file=sys.stderr)
        return 2
    board = pos[0]
    print("common-tally: build %s" % describe())
    rows = tree_rows(pos[1]) if len(pos) == 2 else explicit_rows(rows_spec)
    print("common-tally: %s, %d rows from %s" % (board, len(rows), pos[1] if len(pos) == 2 else "--row"))
    now, lines = tally(board, rows)
    print("\n".join(lines))
    with_run = sum(1 for s in now.values() if s)
    done = sum(1 for s in now.values() if s and s["verdict"] == "done")
    print("common-tally: rows %d, with a run %d, without %d; verdict done %d, other %d"
          % (len(now), with_run, len(now) - with_run, done, with_run - done))
    if prev_dir or prev_spec:
        prows = tree_rows(prev_dir) if prev_dir else explicit_rows(prev_spec)
        prev, _ = tally(board, prows)
        print("common-tally: diff against %s" % (prev_dir or "--prev-row"))
        print("\n".join(diff(prev, now)))
    return 0 if with_run == len(now) and done == with_run else 1


# ---- self-test ----------------------------------------------------------------------

def make_run(outdir, label, name, words, sha):
    """a synthetic i.MX8MP U-Boot run directory, in the files board.sh's verdict_line reads
    (meta.txt, tftp.txt, snvs.txt); no board, no address"""
    d = os.path.join(outdir, "20261004-0000-%s-%s" % (label, sha[:8]))
    os.makedirs(d)
    with open(os.path.join(d, "meta.txt"), "w") as f:
        f.write("board=imx8mp\nboot=uboot-tftp\nname=%s\nprobe=0\nimage=zephyr.bin\nimage_sha256=%s\n"
                "image_build=GC_BUILD_STAMP=synthetic\nrec0=00000000 00000000 00000000 00000000\n"
                "t_reboot=100\nup=16\nt_back=200\nback=1\n" % (name, sha))
    with open(os.path.join(d, "tftp.txt"), "w") as f:
        f.write("1.000 tftp: done peer=<board-host>:1 file=yakogl/x.bin bytes=1000 sha256=%s seconds=0.010\n"
                % sha[:16])
    with open(os.path.join(d, "snvs.txt"), "w") as f:
        f.write(" ".join(words) + "\n")
    return d


def self_test():
    print("common-tally: build %s (self-test)" % describe())
    fails = []

    def check(name, cond, got=None):
        print("%s %s" % ("ok  " if cond else "FAIL", name))
        if not cond:
            fails.append(name)
            if got is not None:
                print("     got: %r" % (got,))

    tmp = tempfile.mkdtemp(prefix="common-tally-")
    try:
        # 1. a tree of synthetic runs through board.sh's own verdict: GLMARK2 pass (score 48, ran 30,
        # failed 0, unsupported 2), PROBES pass, the CASE set failed (passed 3 of 4, first failure 10);
        # SV has no run
        out = os.path.join(tmp, "tree")
        os.makedirs(out)
        make_run(out, "GM", "glmark2", ["594b120e", "00000030", "011e0002", "03200258"], "a" * 64)
        make_run(out, "SO", "slint", ["594b0601", "08420882", "00210842", "98080883"], "b" * 64)
        make_run(out, "GT", "gl_test", ["594b0101", "000aff06", "00000003", "00000004"], "c" * 64)
        with open(os.path.join(out, "tree.txt"), "w") as f:
            for l, w in (("GM", "glmark2"), ("SO", "slint"), ("SV", "slint-view"), ("GT", "gl_test")):
                f.write("%s\t/img/%s/zephyr/zephyr.bin\t-\t%s\n" % (l, l, w))
        rows = tree_rows(out)
        check("tree.txt: four rows, SV without a run",
              [(l, len(d)) for l, d in rows] == [("GM", 1), ("SO", 1), ("SV", 0), ("GT", 1)], rows)
        now, lines = tally("imx8mp", rows)
        gm, so, gt = now["GM"], now["SO"], now["GT"]
        check("GM: glmark2 score 48, ran 30, failed 0, unsupported 2, status pass; verdict done",
              {k: gm["detail"].get(k) for k in ("score", "ran", "failed", "unsupported", "status")} ==
              {"score": 48, "ran": 30, "failed": 0, "unsupported": 2, "status": "pass"} and gm["verdict"] == "done",
              (gm["detail"], gm["verdict"]))
        check("GM: record words, resets absent (never 0), image and build from meta.txt",
              gm["record"] == "594b120e,00000030,011e0002,03200258" and gm["resets"] == "absent" and
              gm["image"] == "a" * 16 and gm["build"] == "GC_BUILD_STAMP=synthetic", gm)
        check("SO: PROBES pass, verdict done", so["detail"].get("status") == "pass" and so["verdict"] == "done", so)
        check("GM: no reset count in a GLMARK2 record and no console: printed 'resets absent', never 0",
              gm["resets_label"] == "resets absent" and any("; resets absent;" in l for l in lines if l.startswith("GM ")),
              (gm["resets_label"], [l for l in lines if l.startswith("GM ")]))
        check("GT: set passed 3 of 4, failing [10] then unknown; verdict failed",
              (gt["detail"].get("passed"), gt["detail"].get("total"), gt["detail"].get("failing")) == (3, 4, [10])
              and "unknown" in gt["detail"]["text"] and gt["verdict"] == "failed", gt)
        check("SV printed as no run (absent)", now["SV"] is None and "SV no run (absent)" in lines, lines)
        check("every run's block names its record words and its detail",
              all(any(s["record"] in l for l in lines) and any(s["detail"]["text"] in l for l in lines)
                  for s in now.values() if s), lines)
        rc = subprocess.run([sys.executable, os.path.abspath(__file__), "imx8mp", out],
                            capture_output=True, text=True).returncode
        check("main() exits 1 when a row has no run or a verdict is not done", rc == 1, rc)
        # a GLMARK2 record whose own status is fail (w2 bits 24..27 = 2) is verdict=failed (recordcmd status())
        make_run(out, "SV", "slint", ["594b120e", "00000027", "021e0102", "03200258"], "d" * 64)
        sv = tally("imx8mp", tree_rows(out))[0]["SV"]
        check("a GLMARK2 record with status 2 is verdict failed",
              sv["detail"].get("status") == "fail" and sv["verdict"] == "failed", (sv["detail"], sv["verdict"]))
        # a GLMARK2 record with GLMARK2_SCENES (a synthetic outcome): its entries are in the detail
        make_run(out, "GS", "glmark2", ["594b120e", "44960027", "22781093", "03200258"], "f" * 64)
        gs = row_summary("imx8mp", "GS", os.path.join(out, "20261004-0000-GS-ffffffff"))
        check("GLMARK2_SCENES: counts from the new layout, first failed entry and why in the detail",
              (gs["detail"].get("score"), gs["detail"].get("unsupported")) == (39, 2)
              and gs["detail"].get("first_failed") == "entry 22 (default list: terrain), why 2 (fragment shader)"
              and "first failed: entry 22" in gs["detail"]["text"], gs["detail"])
        # a record that carries its own reset count (kind DEQP, w3 bits: 1 reset) is labelled as the record's
        make_run(out, "DQ", "deqp", ["594b0904", "00000096", "00000000", "a1e01001"], "e" * 64)
        dq = row_summary("imx8mp", "DQ", os.path.join(out, "20261004-0000-DQ-eeeeeeee"))
        check("a DEQP record's own reset count: 'resets (record) 1'", dq["resets_label"] == "resets (record) 1",
              (dq["resets"], dq["resets_label"]))
        # a known negative for the run-name match: a run of another label is not GM's
        os.rename(os.path.join(out, "20261004-0000-GM-aaaaaaaa"), os.path.join(out, "20261004-0000-GMX-aaaaaaaa"))
        check("tree.txt: a run named GMX is not GM's", tree_rows(out)[0] == ("GM", []), tree_rows(out)[0])
    finally:
        shutil.rmtree(tmp)

    # the label's three sources, in order (the console count is board.sh's count of recovery lines)
    check("resets label: the record's value first, else the console count, else absent",
          (resets_label("2", "2"), resets_label("absent", "0"), resets_label(None, "3"), resets_label("absent", "absent"))
          == ("resets (record) 2", "resets (console count) 0", "resets (console count) 3", "resets absent"))

    # 2. the diff: identical runs are the same; a changed record names each field
    d_same = diff(now, now)
    check("diff of a run with itself: 'same' for each run, 'no run -> no run' for SV",
          sum(": same (" in l for l in d_same) == 3 and "DIFF SV: no run -> no run" in d_same, d_same)
    alt = dict(now)
    alt["GM"] = dict(gm, detail=dict(gm["detail"], score=41, failed=1), verdict="failed", record="x")
    alt["GT"] = dict(gt, detail=dict(gt["detail"], failing=[], passed=4))
    del alt["SO"]
    d = {l.split(":")[0]: l for l in diff(now, alt)}
    check("diff: glmark2 score and failed, verdict and record changes named",
          all(x in d["DIFF GM"] for x in ("glmark2 score 48 -> 41", "glmark2 failed 0 -> 1",
                                           "verdict: done -> failed", "record: ")), d["DIFF GM"])
    alt2 = {"GM": dict(gm, resets_label="resets (console count) 1")}
    d2 = diff({"GM": gm}, alt2)[0]
    check("diff: a reset change is named 'resets: <from> -> <to>' with both labels",
          "resets: resets absent -> resets (console count) 1" in d2, d2)
    check("diff: a gl_test failure that went away, with the record's first-only caveat",
          all(x in d["DIFF GT"] for x in ("passed 3 of 4 -> 4 of 4", "failing +[] -[10]", "first failure only")),
          d["DIFF GT"])
    check("diff: a row missing from the new run", d.get("DIFF SO") == "DIFF SO: run -> no run", d.get("DIFF SO"))

    # 3. the console form (AM62P and Pi 4 gl_test), in the formats of gl_test.c ("gl_test: all: ...",
    # "gl_test: case N <name>: FAIL") and v3d_gl_test main.c ("v3d_gl_test: case N FAIL"), with the
    # Pi 4 UART recorder's "HH:MM:SS " stamp
    am62p = ("10:00:01 gl_test: case 48 somecase: FAIL\n"
             "10:00:02 gl_test: all: 1 of 38 cases failed (37 passed; 0 timed out; first failure case 48 at index 31)\n")
    g = gl_test_from_console(am62p)
    check("console: passed 37 of 38, failing [48]", (g["passed"], g["total"], g["failing"]) == (37, 38, [48]), g)
    pi4 = ("10:00:01 gl_test: all: 0 of 12 cases failed (12 passed; 0 timed out; first failure case -1 at index -1)\n"
           "10:00:01 v3d_gl_test: case 5 PASS\n10:00:01 v3d_gl_test: 12 of 12 cases passed (2000 ms)\n"
           "10:00:01 gl_test: all: 0 of 12 cases failed (12 passed; 0 timed out; first failure case -1 at index -1)\n")
    g = gl_test_from_console(pi4)
    check("console: the verdicts printed twice, identical: passed 12 of 12, failing none",
          (g["passed"], g["total"], g["failing"]) == (12, 12, []) and "failing: none" in g["text"], g)
    g = gl_test_from_console(pi4.replace("v3d_gl_test: case 5 PASS", "v3d_gl_test: case 5 FAIL")
                             .replace("0 of 12 cases failed (12", "1 of 12 cases failed (11"))
    check("console: a 'v3d_gl_test: case 5 FAIL' line is failing [5]", g["failing"] == [5] and g["passed"] == 11, g)
    g = gl_test_from_console("gl_test: case 9 spin: TIMEOUT\n" + am62p)
    check("console: a TIMEOUT case is named with its status", g["failing"] == [9, 48] and "9(TIMEOUT)" in g["text"], g)
    g = gl_test_from_console(pi4.replace("0 of 12 cases failed (12", "1 of 12 cases failed (11", 1))
    check("console: two disagreeing 'all:' lines are no result", g["passed"] is None and "disagree" in g["text"], g)
    g = gl_test_from_console("gl_test: case 3 tri: PASS\ngl_test: case 4 quad: PASS\n")
    check("console: two case lines and no summary is no result, never a pass",
          g["passed"] is None and "no result" in g["text"], g)
    g = gl_test_from_console("10:00:01 gl_test: case 171 somecase: PASS\n10:00:01 gl_test: case 171 somecase: PASS\n"
                             "10:00:01 v3d_gl_test: case 171 PASS\n10:00:01 v3d_gl_test: 1 of 1 cases passed (400 ms)\n")
    check("console: a single Pi 4 case is v3d_gl_test's count, 1 of 1",
          (g["passed"], g["total"], g["failing"]) == (1, 1, []) and "v3d_gl_test's count" in g["text"], g)
    g = gl_test_from_console("10:00:01 gl_test: case 65 somecase: FAIL\n")
    check("console: a single AM62P case line alone is 0 of 1, failing [65]",
          (g["passed"], g["total"], g["failing"]) == (0, 1, [65]), g)

    # 4. the CASE record's other forms, on the decoder's output
    for words, want, what in ((["594b0101", "00070001", "0000001a", "0000001b"], (26, 27, []),
                               "case 7 pass: the set passed 26 of 27 (w2 of w3)"),
                              (["594b0101", "00ab0802", "ffff0000", "ff0000ff"], (0, 1, [171]),
                               "case 171 alone, check 8 failed: failing [171]")):
        p = subprocess.run([ytools(), "record"] + words, capture_output=True, text=True)
        r = detail_from_record(p.stdout)
        check("CASE " + what, (r.get("passed"), r.get("total"), r.get("failing")) == want, (p.stdout, r))

    print("common-tally self-test: %d checks failed" % len(fails))
    return 1 if fails else 0


def check_records(base):
    """the first i.MX8MP common run (2026-10-04), where it lies: one board.sh run per row under
    <base>/glmark2, slint-off, slint-on, gltest171 (not in the repository: records stay out of git)"""
    print("common-tally: build %s (check-records %s)" % (describe(), base))
    specs = []
    for label, sub in (("GM", "glmark2"), ("SO", "slint-off"), ("SV", "slint-on"), ("GT", "gltest171")):
        runs = sorted(n for n in os.listdir(os.path.join(base, sub)) if RUN_RE.match(n))
        if len(runs) != 1:
            print("FAIL %s: %d run directories in %s" % (label, len(runs), sub))
            return 1
        specs.append("%s=%s" % (label, os.path.join(base, sub, runs[0])))
    now, lines = tally("imx8mp", explicit_rows(specs))
    print("\n".join(lines))
    want = {  # read from each run's INDEX.txt "records:" line (2026-10-04)
        "GM": ("594b120e,00000027,021e0102,03200258", "failed", "glmark2 score 39; scenes ran 30, failed 1, unsupported 2"),
        "SO": ("594b0601,08420882,00210842,98080883", "done", "slint probes pass"),
        "SV": ("594b0601,08420882,00210842,98080883", "done", "slint probes pass"),
        "GT": ("594b010e,00ab0802,ffff0000,ff0000ff", "failed", "gl_test case 171 alone: check failed"),
    }
    bad = 0
    for label, (rec, verdict, text) in want.items():
        s = now[label]
        ok = s["record"] == rec and s["verdict"] == verdict and s["detail"]["text"].startswith(text)
        print("%s %s: record %s verdict %s, '%s...'" % ("ok  " if ok else "FAIL", label, rec, verdict, text))
        bad += not ok
    print("common-tally check-records: %d of 4 rows differ" % bad)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
