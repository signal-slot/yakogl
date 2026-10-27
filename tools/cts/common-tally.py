#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The common set's tally: one block per row of a common run.

    tools/cts/common-tally.py <board> <outdir> [--prev <earlier outdir>]
    tools/cts/common-tally.py <board> --row LABEL=<rundir> ... [--prev-row LABEL=<rundir> ...]
    tools/cts/common-tally.py --self-test
    tools/cts/common-tally.py --check-records <dir of the first i.MX8MP common run>
    tools/cts/common-tally.py --gm-table <glmark2 console file or run directory>
    tools/cts/common-tally.py --gm-diff <A> <B>
    tools/cts/common-tally.py --gm-pictures <glmark2 console or run directory> <column> [--table <file>]
    tools/cts/common-tally.py --gm-picture-lines <glmark2 console or run directory> <column>

<outdir> is what `tools/board.sh <board> tree tools/cts/common-tree.txt
<outdir>` wrote: tree.txt (the rows, labelled GM GT) and one run
directory per row, YYYYMMDD-HHMM-<LABEL>-<sha8>. --row names run directories
directly, for runs made one at a time with `board.sh run` (the rows of the
first i.MX8MP common run, 2026-10-04, were).

The verdict of each run is tools/board.sh's own, recomputed from the run's
files (`board.sh <board> tally <rundir>`, the same call board.sh tree ends
with); a verdict that differs from the one INDEX.txt stored is printed beside
it. This tool adds what board.sh's verdict line does not say, per kind:

  glmark2  the GLMARK2 record: score, scenes ran / failed / unsupported, and
           the record's own status (pass or fail); then, from the run's
           console.txt (the AM62P and the Pi 4), the per-scene verdict: each
           line of GLMARK2_BAD_LINES (a GL error, an incomplete texture, a
           refused or unrecorded draw, out of GPU memory, a refused tiled
           copy) given to its scene by the "glmark2-mem:" lines (or, in a
           console without them, to the next scene's result line), and the
           cell: amber when a scene has such a line and the verdict is done,
           failed when the verdict is, else green; "green (record only)"
           without a console (the i.MX8MP); and the per-scene picture, from the
           console's "glmark2-picture:" lines against the board's column of
           tools/cts/glmark2-pictures.txt (tools/cts/README.md): a probe or the
           mean off by more than the tolerance, a missing readback or a scene
           with no reference makes the cell amber, an all-black scene failed
  gl_test  passed of run and the failing case numbers. From the console:
           "gl_test: all: F of T cases failed (P passed; X timed out; ...)"
           (gl_test.c, AM62P and Pi 4) and each "gl_test: case N <name>: FAIL"
           or "v3d_gl_test: case N FAIL" line. From a CASE record (the
           i.MX8MP has no UART): the set's summary (status CASE_SET,
           gc_gl_test main.c gc_result_set): passed of run (skips not run),
           the skipped count and first skip, and the FIRST TWO failing cases;
           past two the others are not in the record and are printed as
           unknown, never as none. The earlier form (CASE_PASS of case 7, or
           CASE_SET_FAIL) names the first failing case only and counted a
           by-name skip as a failure.

Absent stays absent: a row without a run, a run without a result, a field the
record does not carry. Each line names the record words (or the legacy console
result), the GPU resets as board.sh counts them, the image's sha256 and its
build ids (meta.txt, as board.sh read them from the image's build.ninja).

--prev compares with an earlier common run (a tree outdir, or --prev-row
LABEL=<rundir> ...), row by row by label: the verdict, the record words, and
the fields above (glmark2 score, cell and each scene's known-bad line counts,
gl_test failing cases added and removed).

--gm-pictures judges the pictures of one console against a column (exit 1 unless
every scene matches); --gm-picture-lines prints the console's pictures as table
lines for that column, to review and paste into glmark2-pictures.txt.

--gm-table and --gm-diff read speed, not a verdict (tools/cts/README.md): per
scene the FPS and ms per frame (glmark2's FrameTime; 1000 / FPS only where a
line has none, and the output says which) of a glmark2 console, and the
change between two consoles of one board. They judge nothing; exit 0, or 2
when a console has no scene line.

The first line is this tool's own build (git describe of its checkout).
Exit status: 0 when every row has a run and every verdict is done (an amber
glmark2 cell included: amber is the cell, not the verdict); 1 when a row has no
run or a verdict is anything else; 2 for a usage error.
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

# The glmark2 console lines that say a scene drew wrong while its record stays green: one row
# per class, (name, pattern searched in a console line, where tools/cts/RESULTS.md shows it).
# A GM row whose console has one inside a scene is amber (unless its verdict is failed).
GLMARK2_BAD_LINES = (
    # "Pi 4: the OSAL looks at the job in flight before a free or an allocation, gl_test case
    # 180": terrain logs "GL error 0x502 in glGenerateMipmap" 16 times (context.rs record_error;
    # any code counts: the AM62P's terrain also logged 0x505 and 0x506)
    ("gl-error", re.compile(r"GL error 0x[0-9a-f]+ in ")),
    # the same section: "a texture on unit 0 is incomplete and samples (0,0,0,1) ... 800x600,
    # levels 1 of 1" (draw.rs), terrain sampling black
    ("incomplete-texture", re.compile(r"a texture on unit \d+ is incomplete and samples \(0,0,0,1\)")),
    # "AM62P: the reclaim default, common round ... glmark2 cell AMBER": refract's depth-pass
    # draw refused in every frame, the backend's reason ("yakogl/pvr: draw refused: ...", once)
    # and the GL core's "glDrawArrays was not recorded (-12); the frame is missing it" (each)
    ("draw-refused", re.compile(r"draw refused")),
    ("draw-not-recorded", re.compile(r"was not recorded \(-?\d+\); the frame is missing it")),
    # "Pi 4: the OSAL looks at the job in flight ...": refract's "out of GPU memory for 7684096
    # bytes (1 frees held)", then "no GPU memory for a tiled copy of 7684096 bytes (...)" in
    # each frame; terrain's tiled-copy refusals in "Pi 4: the stale-copy fix in glmark2 and
    # gl_test, and the per-scene memory line"
    ("out-of-memory", re.compile(r"out of GPU memory")),
    ("tiled-copy", re.compile(r"no GPU memory for (?:a [0-9]+-byte |a |the texture's )tiled copy")),
)


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
# record.rs describe(), kind CASE status CASE_SET (the set's summary)
SET_RE = re.compile(r"CASE \(\d+\) seq \d+: set status \d+ \(set (passed|failed)\) passed (\d+) of (\d+) run; "
                    r"skipped (\d+), first skip case (\d+|none); failing (\d+): first case (\d+|none), "
                    r"second case (\d+|none)")
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


# glmark2 sample main.cpp: print_mem() "glmark2-mem: entry N <info>: before setup|end: ...", print_cost()
# "glmark2-frame: <info>: N frames; ..."; upstream's result line "<info>: FPS: ..." (or "Set up failed",
# "Unsupported"). <info> is the scene's info_string(), "[terrain] <default>"; its options hold ':' but
# never ': '
# The sample prints "entry N before setup|end: ..." without <info> (the scene is named by the
# result line that follows); a console of an older image has "entry N <info>: before setup|end: ".
GM_MEM_RE = re.compile(r"glmark2-mem: entry (\d+) (?:(\[[\w-]+\] .*?): )?(before setup|end): ")
GM_FRAME_RE = re.compile(r"glmark2-frame: (\[[\w-]+\] .*?): \d+ frames;")
GM_RESULT_RE = re.compile(r"(?:^|\s)(\[[\w-]+\] .*?): (?:FPS: |Set up failed|Unsupported)")


def glmark2_scenes(text):
    """the known-bad lines of GLMARK2_BAD_LINES in a glmark2 console, by scene.

    With the memory lines a scene is the span from its "before setup" line to its "end" line.
    A console without them (an image before 42bd90ff) gives a line to the next scene that
    prints its result or frame line: glmark2 prints them after the scene's frames. A line in
    no scene is counted under outside. Returns {"order": [info, ...], "scenes": {info: {class:
    count}}, "outside": {class: count}, "mem_lines": bool}; a scene without a bad line has {}."""
    order, scenes, outside = [], {}, {}
    current, pending, mem = None, [], False

    def seen(info):
        if info not in scenes:
            order.append(info)
            scenes[info] = {}
        return scenes[info]

    def add(counts, classes):
        for c in classes:
            counts[c] = counts.get(c, 0) + 1

    for line in plain_lines(text):
        bad = [name for name, rx in GLMARK2_BAD_LINES if rx.search(line)]
        m = GM_MEM_RE.search(line)
        if m:
            mem = True
            info, when = m.group(2), m.group(3)
            if info is None:
                # no scene name: the result line names it, a line goes to that result as without
                continue
            if when == "before setup":
                add(outside, [c for b in pending for c in b])
                pending = []
                current = info
                seen(info)
            else:
                current = None
            continue
        m = GM_FRAME_RE.search(line) or GM_RESULT_RE.search(line)
        if m:
            counts = seen(m.group(1))
            if current is None:
                # no memory lines around this scene: what came since the last scene is its
                add(counts, [c for b in pending for c in b])
                pending = []
            continue
        if bad:
            if current is not None:
                add(scenes[current], bad)
            else:
                pending.append(bad)
    add(outside, [c for b in pending for c in b])
    return {"order": order, "scenes": scenes, "outside": outside, "mem_lines": mem}


# ---- per-scene picture: glmark2-picture: lines against tools/cts/glmark2-pictures.txt ---------
# The glmark2 sample prints after each scene "glmark2-picture: <info>: WxH; probes c RRGGBBAA, tl
# .. tr .. bl .. br ..; crc16 X, crc16q X; mean R G B; black yes|no" (driver/docs/glmark2.md, "The
# picture value"), or "<info>: absent (the readback failed)". The table has one line per scene and
# column (a board, or host): "<column> | <info> | <the text after "<info>: ">".
# COMMON_TALLY_PICTURES names another table (cts-selftest.sh judges fixtures against its own);
# --gm-pictures also takes --table
PICTURE_TABLE = os.environ.get("COMMON_TALLY_PICTURES") or os.path.join(HERE, "glmark2-pictures.txt")
GM_PICTURE_RE = re.compile(r"glmark2-picture: (\[[\w-]+\] .*?): (\d+x\d+;.*|absent.*)$")
PICTURE_VALUE_RE = re.compile(
    r"^(\d+)x(\d+); probes c ([0-9a-f]{8}), tl ([0-9a-f]{8}), tr ([0-9a-f]{8}), bl ([0-9a-f]{8}), "
    r"br ([0-9a-f]{8}); crc16 ([0-9a-f]{4}), crc16q ([0-9a-f]{4}); mean (\d+) (\d+) (\d+); "
    r"black (yes|no)\s*$")
PICTURE_PROBES = ("c", "tl", "tr", "bl", "br")
PICTURE_TOLERANCE = 8  # levels per channel, used when the table has no "tolerance" line


PAGE_MISSING_RE = re.compile(r"absent \((page P[01] of scene \d+ missing)\)")


def parse_picture_value(text):
    """the fields of a picture value's text, or None when it is not one ("absent ..." included)"""
    m = PICTURE_VALUE_RE.match(text)
    if not m:
        return None
    g = m.groups()
    return {"size": (int(g[0]), int(g[1])), "probes": dict(zip(PICTURE_PROBES, g[2:7])),
            "crc16": g[7], "crc16q": g[8], "mean": tuple(int(x) for x in g[9:12]), "black": g[12] == "yes"}


def glmark2_pictures(text):
    """the picture lines of a glmark2 console: {"order": [info, ...], "scenes": {info: value}} where
    value is parse_picture_value()'s dict, or None for an "absent" line or one that does not
    parse; "raw" is {info: the value's text}, which tells the two apart (judge_one_picture); a
    scene printed twice keeps its last line. "missing" is {info: why} for the scenes whose line says a page of
    glmark2-pages.txt is missing"""
    order, scenes, raw, missing = [], {}, {}, {}
    for line in plain_lines(text):
        m = GM_PICTURE_RE.search(line)
        if m:
            if m.group(1) not in scenes:
                order.append(m.group(1))
            scenes[m.group(1)] = parse_picture_value(m.group(2))
            raw[m.group(1)] = m.group(2)
            # glmark2-pages.txt: "absent (page P0 of scene N missing)" is a page that was never run
            # (nothing to judge), not a readback that failed
            pm = PAGE_MISSING_RE.match(m.group(2))
            if pm:
                missing[m.group(1)] = pm.group(1)
            else:
                missing.pop(m.group(1), None)
    return {"order": order, "scenes": scenes, "raw": raw, "missing": missing}


def read_picture_table(path=PICTURE_TABLE):
    """{"tolerance": int, "columns": {column: {info: value}}}; lines starting with # are comments"""
    table = {"tolerance": PICTURE_TOLERANCE, "columns": {}}
    for n, line in enumerate((read_text(path) or "").split("\n"), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("tolerance "):
            table["tolerance"] = int(line.split()[1])
            continue
        parts = [p.strip() for p in line.split(" | ", 2)]
        value = parse_picture_value(parts[2]) if len(parts) == 3 else None
        if value is None:
            raise SystemExit("common-tally: %s:%d is not '<column> | <scene> | <picture value>'" % (path, n))
        table["columns"].setdefault(parts[0], {})[parts[1]] = value
    return table


def judge_picture(value, ref, tol):
    """(state, why) of one scene's picture against its reference value (None: no reference):
    match, mismatch (a probe or the mean off by more than tol in R, G or B, or another size),
    black (all black where the reference is not), absent (no readback), no-reference.
    A None value is a failed readback ("absent"); judge_one_picture() tells it from a line that
    does not parse (unreadable) before it gets here.
    crc16 is informational: it is named when it differs, never judged."""
    if value is None:
        return "absent", "the readback failed"
    if ref is None:
        return "no-reference", "no reference in %s" % os.path.basename(PICTURE_TABLE)
    if value["black"] and not ref["black"]:
        return "black", "all black, the reference is not"
    if value["size"] != ref["size"]:
        return "mismatch", "size %dx%d, reference %dx%d" % (value["size"] + ref["size"])
    off = []
    for p in PICTURE_PROBES:
        a, b = value["probes"][p], ref["probes"][p]
        d = max(abs(int(a[i:i + 2], 16) - int(b[i:i + 2], 16)) for i in (0, 2, 4))
        if d > tol:
            off.append("probe %s %s vs %s (off %d)" % (p, a[:6], b[:6], d))
    d = max(abs(a - b) for a, b in zip(value["mean"], ref["mean"]))
    if d > tol:
        off.append("mean %d %d %d vs %d %d %d (off %d)" % (value["mean"] + ref["mean"] + (d,)))
    if off:
        return "mismatch", "; ".join(off)
    return "match", "" if value["crc16"] == ref["crc16"] else "crc16 %s, reference %s" % (value["crc16"], ref["crc16"])


def judge_one_picture(pictures, info, ref, tol):
    """judge_picture() of one scene of glmark2_pictures(); a line whose value is neither a picture
    value nor "absent ..." is unreadable (the first 80 characters of it), not a failed readback"""
    value = pictures["scenes"][info]
    raw = pictures.get("raw", {}).get(info, "")
    if value is None and not raw.startswith("absent"):
        return "unreadable", "does not parse: %s" % raw[:80]
    return judge_picture(value, ref, tol)


def judge_pictures(pictures, table, column):
    """the judged pictures of a console: {"column", "order", "scenes": {info: (state, why)}};
    None when the console has no picture line (an image from before the picture value)"""
    if pictures is None or not pictures["order"]:
        return None
    refs = table["columns"].get(column, {})
    missing = pictures.get("missing", {})
    return {"column": column, "order": pictures["order"],
            "scenes": {s: ("not-in-record", missing[s]) if s in missing else
                       judge_one_picture(pictures, s, refs.get(s), table["tolerance"])
                       for s in pictures["order"]}}


def picture_states(judged):
    """{state: count} of judge_pictures()"""
    counts = {}
    for state, _ in judged["scenes"].values():
        counts[state] = counts.get(state, 0) + 1
    return counts


def picture_lines(judged):
    """the printed picture verdict of a GM row: a summary, then each scene that is not a match;
    nothing when the console has no picture line"""
    if judged is None:
        return []  # a console from before the picture value: nothing to say
    counts = picture_states(judged)
    out = ["glmark2 pictures against column %s: %d scenes, match %d%s"
           % (judged["column"], len(judged["order"]), counts.get("match", 0),
              "".join(", %s %d" % (k, counts[k]) for k in ("mismatch", "black", "absent", "unreadable", "no-reference", "not-in-record")
                      if k in counts))]
    for s in judged["order"]:
        state, why = judged["scenes"][s]
        if state != "match" or why:
            out.append("scene %s: picture %s (%s)" % (s, state.replace("-", " "), why))
    return out


def picture_cell(judged):
    """what the pictures make of the GM cell: failed for an all-black scene, amber for a mismatch,
    a missing readback or a missing reference, None when every scene matches or none was judged
    (a scene whose page is not in the record is not judged: not-in-record is no failure)"""
    if judged is None:
        return None
    counts = picture_states(judged)
    if counts.get("black"):
        return "failed"
    if any(counts.get(k) for k in ("mismatch", "absent", "unreadable", "no-reference")):
        return "amber"
    return None


def gm_pictures_main(argv):
    """--gm-pictures SRC COLUMN: judge a console's pictures; --gm-picture-lines SRC COLUMN: print it
    as table lines for COLUMN (to review and paste into glmark2-pictures.txt). SRC is a console
    file or a run directory"""
    mode, src, column = argv[:3]
    table_path = argv[4] if len(argv) == 5 else PICTURE_TABLE
    text = read_text(os.path.join(src, "console.txt") if os.path.isdir(src) else src)
    pictures = glmark2_pictures(text) if text is not None else None
    if pictures is None or not pictures["order"]:
        print("common-tally: no glmark2-picture lines in %s" % src, file=sys.stderr)
        return 2
    print("common-tally: build %s" % describe())
    if mode == "--gm-picture-lines":
        for s in pictures["order"]:
            if pictures["scenes"][s] is None:
                kind = "absent" if pictures["raw"][s].startswith("absent") else "unreadable"
                print("common-tally: %s has no picture value (%s), no line for it" % (s, kind), file=sys.stderr)
            else:
                print("%s | %s | %s" % (column, s, pictures["raw"][s]))
        return 0
    judged = judge_pictures(pictures, read_picture_table(table_path), column)
    print("\n".join(picture_lines(judged)))
    return 0 if picture_cell(judged) is None else 1


# ---- per-scene ms/frame: --gm-table and --gm-diff -------------------------------------------
# Speed is read from same-board glmark2 runs, per scene in ms per frame (FrameTime, else 1000 / FPS),
# never from a demo's fps. These read the console lines glmark2_scenes() reads and judge nothing.
GM_SCORE_RE = re.compile(r"glmark2: score (\d+)")
GM_FPS_RE = re.compile(r"FPS: (\d+)")
GM_FRAMETIME_RE = re.compile(r"FrameTime: (\d+(?:\.\d+)?) ms")
GM_GL_RE = re.compile(r"us per frame: gl (\d+)")
# glmark2-pages.txt (tools/board.sh gm-lines): the glmark2-frame: line of a page run says the means
# are not carried; its FrameTime is 1000 / FPS of an integer FPS, not glmark2's own
GM_NOT_CARRIED_RE = re.compile(r"us per frame: not carried \(GM_PAGE\)")
GM_PAGES_HEADER_RE = re.compile(r"^glmark2-pages:", re.M)
NOT_IN_RECORD = "not in record"
GM_WAIT_RE = re.compile(r"end waited for in (\d+) of (\d+)")
# upstream's main-loop.cpp prints "<info>:" when a scene starts and the rest (" FPS: n FrameTime: x
# ms", " Set up failed", " Unsupported") when it ends, with no info in front. Other output of the
# scene (the driver's lines, GL errors) lands between the two, so the result line is then only
# "<time> FPS: 6 FrameTime: 181.654 ms" and the scene is named by the lines around it.
GM_BARE_FPS_RE = re.compile(r"(?:^|\s)FPS: \d+")
GM_BARE_END_RE = re.compile(r"^(?:\d\d:\d\d:\d\d )? ?(?:Set up failed|Unsupported)\s*$")
GM_HEADER_RE = re.compile(r"^(?:\d\d:\d\d:\d\d )?(\[[\w-]+\] .*)$")
GM_MOVED_PCT = 5.0
MS_FRAMETIME, MS_FROM_FPS = "FrameTime", "1000/FPS"


def header_info(raw):
    """the scene info of a header line "[scene] opt=v:opt=w:" with driver text glued after it:
    the scene and its leading options, in full: each is "name=value" or "<default>", and the
    last one may be followed by a space and the driver text"""
    m = re.match(r"(\[[\w-]+\]) (.*)$", raw)
    keep = []
    for part in m.group(2).split(":"):
        if part == "<default>":
            keep.append(part)
            continue
        o = re.match(r"[\w-]+=\S*", part)
        if not o:
            break
        keep.append(o.group(0))
        if o.end() != len(part):  # glued text after a space, not a colon: the options end here
            break
    return "%s %s" % (m.group(1), ":".join(keep) if keep else "?")


def glmark2_speed(text):
    """the speed of a glmark2 console: {"score": int or None, "scenes": [{"name", "fps", "ms",
    "ms_src", "gl_ms", "waited": (n, of)}, ...]} in run order. ms is glmark2's own FrameTime
    (ms_src "FrameTime"), and 1000 / FPS only where the line has no FrameTime (ms_src
    "1000/FPS"). fps, ms and ms_src are None for a scene that set up failed or is unsupported;
    gl_ms and waited are None where the console has no glmark2-frame line. A result line
    without its scene (GM_BARE_FPS_RE) is named by the memory line that opened the scene, else
    by the glmark2-frame line that matches it, else by the header line before it. A
    glmark2-frame line goes to the oldest scene that has an FPS and no frame line yet and
    whose info is the line's; failing that, to the oldest such scene with no name whose header
    the line's info extends, else to the oldest such scene with no name. So a scene run twice
    before its frame line arrives gets one frame line per run, and a scene without an FPS
    (set up failed, unsupported), which prints no frame line, is never given one.
    A scene name seen again gets " #2", " #3", so two runs of one scene stay apart.
    A glmark2-pages.txt (a line starts "glmark2-pages:") is a decode of the SNVS record: "record"
    is True, ms is 1000 / FPS (the FrameTime it prints is derived from the same integer), and a
    scene whose glmark2-frame: line says "not carried" has not_in_record True."""
    record = GM_PAGES_HEADER_RE.search(text) is not None
    score, scenes, last, seen = None, [], None, {}
    current, header = None, None  # the scene the memory lines opened; the last header line

    def name(sc, info):
        seen[info] = seen.get(info, 0) + 1
        sc["info"] = info
        sc["name"] = info if seen[info] == 1 else "%s #%d" % (info, seen[info])

    def frame_target(info):
        """the scene a glmark2-frame line is for (see glmark2_speed)"""
        open_ = [sc for sc in scenes if not sc["framed"] and sc["fps"] is not None]
        for sc in open_:
            if sc["info"] == info:
                return sc
        for sc in open_:
            if sc["info"] is None and sc["header"] and info.startswith(header_info(sc["header"])):
                return sc
        for sc in open_:
            if sc["info"] is None:
                return sc
        return None

    for line in plain_lines(text):
        m = GM_SCORE_RE.search(line)
        if m:
            score = int(m.group(1))
            continue
        m = GM_MEM_RE.search(line)
        if m:
            current = m.group(2) if m.group(3) == "before setup" else None
            continue
        m = GM_FRAME_RE.search(line)
        if m:
            sc = frame_target(m.group(1))
            if sc is not None:
                sc["info"] = m.group(1)
                sc["framed"] = True
                if GM_NOT_CARRIED_RE.search(line):
                    sc["not_in_record"] = True
                else:
                    g, w = GM_GL_RE.search(line), GM_WAIT_RE.search(line)
                    sc["gl_ms"] = int(g.group(1)) / 1000.0 if g else None
                    sc["waited"] = (int(w.group(1)), int(w.group(2))) if w else None
            continue
        m = GM_RESULT_RE.search(line)
        if m:
            info = m.group(1)
        elif GM_BARE_FPS_RE.search(line) or GM_BARE_END_RE.search(line):
            info = current
        else:
            h = GM_HEADER_RE.match(line)
            if h:
                header = h.group(1)
            continue
        f, t = GM_FPS_RE.search(line), GM_FRAMETIME_RE.search(line)
        fps = int(f.group(1)) if f else None
        ms, src = (None, None) if not fps else (
            (float(t.group(1)), MS_FRAMETIME) if t and not record else (1000.0 / fps, MS_FROM_FPS))
        last = {"info": None, "fps": fps, "ms": ms, "ms_src": src, "gl_ms": None,
                "waited": None, "not_in_record": False, "name": "", "header": header,
                "framed": False}
        last["info"] = info
        scenes.append(last)
    # named at the end, in run order, so a frame line can still give a scene its full info
    for sc in scenes:
        name(sc, sc["info"] or (header_info(sc["header"]) if sc["header"] else "[?] ?"))
        del sc["header"], sc["framed"]
    return {"score": score, "scenes": scenes, "record": record}


def read_speed(src):
    """glmark2_speed() of a console file, or of console.txt in a run directory; None if absent"""
    text = read_text(os.path.join(src, "console.txt") if os.path.isdir(src) else src)
    return None if text is None else glmark2_speed(text)


def fmt_ms(v, w=9, absent="-"):
    return ("%*.2f" % (w, v)) if v is not None else "%*s" % (w, absent)


def pct(a, b):
    return (b - a) * 100.0 / a


def score_text(sp):
    """a glmark2_speed() score: the number, "absent", or not in record for a pages file (the full
    run's GLMARK2 record has it)"""
    if sp["score"] is not None:
        return str(sp["score"])
    return NOT_IN_RECORD if sp.get("record") else "absent"


def ms_sources(sp):
    """where the ms/frame of a glmark2_speed() came from, as a phrase"""
    n = [sum(1 for s in sp["scenes"] if s["ms_src"] == k) for k in (MS_FRAMETIME, MS_FROM_FPS)]
    return "%d from FrameTime, %d from 1000/FPS" % tuple(n)


def gm_table(sp):
    """the table lines for a glmark2_speed(): one row per scene"""
    sc = sp["scenes"]
    width = max([len(s["name"]) for s in sc] + [5])
    # a scene of glmark2-pages.txt has no gl / end-waited means: the cell says "not in record"
    wide = len(NOT_IN_RECORD) if any(s["not_in_record"] for s in sc) else 0
    wg, ww = max(8, wide), max(11, wide)
    out = ["%-*s  %5s %9s %*s  %-*s %s" % (width, "scene", "FPS", "ms/frame", wg, "gl ms", ww, "end waited",
                                           "ms from")]
    for s in sc:
        w = s["waited"]
        nr = NOT_IN_RECORD if s["not_in_record"] else "-"
        out.append("%-*s  %5s %s %s  %-*s %s" % (
            width, s["name"], "-" if s["fps"] is None else s["fps"], fmt_ms(s["ms"]),
            fmt_ms(s["gl_ms"], wg, nr), ww, nr if w is None else "%d of %d" % w, s["ms_src"] or "-"))
    ran = [s for s in sc if s["ms"] is not None]
    out.append("score %s; %d scenes, %d with an FPS, %.2f ms/frame summed (%s)"
               % (score_text(sp), len(sc), len(ran), sum(s["ms"] for s in ran), ms_sources(sp)))
    if sp["record"]:
        out.append("from glmark2-pages.txt (the SNVS record): ms/frame is 1000 / the integer FPS "
                   "(a rate rounded up, so a scene is up to one FPS step faster than it reads); "
                   "FrameTime, the gl / swap / show / frame means and the GL error lines are %s"
                   % NOT_IN_RECORD)
    return out


def gm_diff(sa, sb):
    """the diff lines of two glmark2_speed(): per scene the ms/frame of A and B, the delta in ms
    and % (B against A: positive is slower), '*' on a scene that moved more than GM_MOVED_PCT;
    then both scores"""
    ma = {s["name"]: s for s in sa["scenes"]}
    mb = {s["name"]: s for s in sb["scenes"]}
    names = [s["name"] for s in sa["scenes"]] + [n for n in mb if n not in ma]
    width = max([len(n) for n in names] + [5])
    out = ["%-*s  %9s %9s %9s %8s" % (width, "scene", "A ms", "B ms", "delta ms", "delta %")]
    moved = 0
    for n in names:
        a, b = ma.get(n, {}).get("ms"), mb.get(n, {}).get("ms")
        if a is None or b is None:
            note = "only in A" if n not in mb else "only in B" if n not in ma else "no FPS"
            out.append("%-*s  %s %s %9s %8s  %s" % (width, n, fmt_ms(a), fmt_ms(b), "-", "-", note))
            continue
        p = pct(a, b)
        mark = abs(p) > GM_MOVED_PCT
        fb = "".join(" (%s: 1000/FPS)" % w for w, m in (("A", ma[n]), ("B", mb[n]))
                     if m["ms_src"] == MS_FROM_FPS)
        moved += mark
        out.append("%-*s  %s %s %+9.2f %+7.1f%%%s%s" % (
            width, n, fmt_ms(a), fmt_ms(b), b - a, p, "  *" if mark else "", fb))
    out.append("scenes moved by more than %g%%: %d of %d (marked *)" % (GM_MOVED_PCT, moved, len(names)))
    out.append("ms/frame: A %s; B %s (a scene without FrameTime is marked 1000/FPS)"
               % (ms_sources(sa), ms_sources(sb)))
    ca, cb = sa["score"], sb["score"]
    out.append("score A %s, B %s%s" % (
        score_text(sa), score_text(sb),
        "" if not ca or cb is None else " (%+.1f%%)" % pct(ca, cb)))
    return out


def gm_speed_main(mode, srcs):
    """--gm-table SRC / --gm-diff A B; SRC is a console file or a run directory"""
    sps = []
    for src in srcs:
        sp = read_speed(src)
        if sp is None or not sp["scenes"]:
            print("common-tally: no glmark2 scene lines in %s" % src, file=sys.stderr)
            return 2
        sps.append(sp)
    print("common-tally: build %s" % describe())
    if mode == "--gm-table":
        print("\n".join(gm_table(sps[0])))
    else:
        print("A = %s\nB = %s" % tuple(srcs))
        print("\n".join(gm_diff(sps[0], sps[1])))
    return 0


def counts_text(counts):
    return ", ".join("%s %d" % (c, counts[c]) for c, _ in GLMARK2_BAD_LINES if c in counts)


def glmark2_cell(verdict, scenes, pictures=None):
    """the GM cell: failed when the verdict is; amber when a scene has a known-bad line;
    green when none does; without a console (scenes None) the verdict alone. pictures is
    judge_pictures(): an all-black scene makes the cell failed, a mismatch, a missing readback
    or a missing reference amber (picture_cell)"""
    if verdict == "failed":
        return "failed"
    if verdict != "done":
        return verdict
    if scenes is None:
        # no console: the record alone, or with the pictures glmark2-pages.txt gives
        by_picture = picture_cell(pictures)
        if by_picture:
            return by_picture
        return "green (record only)" if pictures is None else "green (record and pages)"
    by_picture = picture_cell(pictures)
    if by_picture == "failed":
        return "failed"
    if by_picture or any(scenes["scenes"][s] for s in scenes["order"]) or scenes["outside"]:
        return "amber"
    return "green"


def glmark2_scene_lines(cell, scenes):
    """the printed per-scene verdict of a GM row"""
    if scenes is None:
        return ["glmark2 cell %s; per-scene lines absent (no console)" % cell]
    bad = [s for s in scenes["order"] if scenes["scenes"][s]]
    out = ["glmark2 cell %s; scenes in the console %d, clean %d, with known-bad lines %d%s"
           % (cell, len(scenes["order"]), len(scenes["order"]) - len(bad), len(bad),
              "" if scenes["mem_lines"] else " (no glmark2-mem lines: a line goes to the next scene's result)")]
    for s in bad:
        c = scenes["scenes"][s]
        out.append("scene %s: amber, %d lines (%s)" % (s, sum(c.values()), counts_text(c)))
    if scenes["outside"]:
        o = scenes["outside"]
        out.append("outside any scene: %d lines (%s)" % (sum(o.values()), counts_text(o)))
    return out


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
        # a PROBES record (a sample's own check; no row of the common set writes one)
        return {"kind": "probes", "status": m.group(1),
                "text": "probes %s (%s)" % (m.group(1), decoded.strip().split(": ", 1)[-1])}
    m = SET_RE.search(decoded)
    if m:
        verdict, passed, run, skipped, skip1, nfail, f1, f2 = m.groups()
        passed, run, skipped, nfail = int(passed), int(run), int(skipped), int(nfail)
        named = [int(c) for c in (f1, f2) if c != "none"]
        d = {"kind": "gl_test", "source": "record", "passed": passed, "total": run, "failing": named,
             "skipped": skipped, "first_skip": None if skip1 == "none" else int(skip1)}
        failing = ", ".join(str(c) for c in named) or "none"
        if nfail > len(named):
            d["first_only"] = True
            failing += ", then %d unknown (the record names the first two failures)" % (nfail - len(named))
        d["text"] = ("gl_test passed %d of %d run (CASE record, set %s); skipped %d%s; failing: %s"
                     % (passed, run, verdict, skipped,
                        "" if skip1 == "none" else " (first case %s)" % skip1, failing))
        return d
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


PAGES_NAME = "glmark2-pages.txt"


def read_pages(rundir):
    """(path, text) of the glmark2-pages.txt `tools/board.sh imx8mp gm-lines <outdir>` wrote, from
    the run directory or else the directory that holds it (the outdir of the page runs, which
    is a tree's outdir when the common set ran the pages too); None when there is none"""
    for d in (rundir, os.path.dirname(os.path.abspath(rundir))):
        path = os.path.join(d, PAGES_NAME)
        text = read_text(path)
        if text is not None:
            return path, text
    return None


def pages_lines(cell, path, speed, pictures):
    """the printed verdict of a GM row read from glmark2-pages.txt: what the record has, what it
    drops (shown as not in record, never as a failure), then the pictures"""
    sc = speed["scenes"]
    ran = [x for x in sc if x["ms"] is not None]
    out = ["glmark2 cell %s; scenes in %s %d, with an FPS %d, %.2f ms/frame summed (1000 / the integer FPS); "
           "per-scene table: common-tally.py --gm-table %s" % (cell, PAGES_NAME, len(sc), len(ran),
                                                              sum(x["ms"] for x in ran), path),
           "not in record: FrameTime, the gl / swap / show / frame means, the glmark2-mem lines, the GL "
           "error lines (known-bad lines), the probes' alpha and low bits (RGB565 probes); the score is "
           "the GLMARK2 record's"]
    return out + picture_lines(pictures)


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
        if s["detail"]["kind"] == "glmark2":
            # the per-scene verdict from the console (the AM62P and the Pi 4; the i.MX8MP has none)
            sc = glmark2_scenes(console) if console is not None else None
            # the pictures are judged against the board's column of glmark2-pictures.txt
            pics = judge_pictures(glmark2_pictures(console), read_picture_table(), board) \
                if console is not None else None
            pages = read_pages(rundir) if console is None else None
            if pages is not None:
                # the i.MX8MP: no console, the speed and the pictures come from the page records
                pics = judge_pictures(glmark2_pictures(pages[1]), read_picture_table(), board)
                s["detail"]["speed"] = glmark2_speed(pages[1])
            s["detail"]["scenes"] = sc
            s["detail"]["pictures"] = pics
            s["detail"]["cell"] = glmark2_cell(s["verdict"], sc, pics)
            s["detail"]["scene_lines"] = glmark2_scene_lines(s["detail"]["cell"], sc)
            if sc is not None:
                s["detail"]["scene_lines"] += picture_lines(pics)
            elif pages is not None:
                s["detail"]["scene_lines"] = pages_lines(s["detail"]["cell"], pages[0], s["detail"]["speed"], pics)
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
            lines.extend("    %s" % l for l in s["detail"].get("scene_lines", ()))
            if s["differs"]:
                lines.append("    INDEX.txt stored a different verdict: %s" % s["differs"])
            lines.append("    run %s" % d)
            out[label] = s
    return out, lines


def scene_diff(da, db):
    """the per-scene changes between two glmark2 details: the cell, then each scene (and the
    outside bucket) whose known-bad counts changed, by class"""
    ch = []
    if da.get("cell") != db.get("cell"):
        ch.append("glmark2 cell %s -> %s" % (da.get("cell") or "absent", db.get("cell") or "absent"))
    pa, pb = da.get("pictures"), db.get("pictures")
    if pa or pb:
        ja, jb = (pa or {}).get("scenes", {}), (pb or {}).get("scenes", {})
        for name in (pb or {}).get("order", []) + [x for x in (pa or {}).get("order", []) if x not in jb]:
            if ja.get(name, ("absent",))[0] != jb.get(name, ("absent",))[0]:
                ch.append("scene %s: picture %s -> %s" % (name, ja.get(name, ("not printed",))[0],
                                                          jb.get(name, ("not printed",))[0]))
    sa, sb = da.get("scenes"), db.get("scenes")
    if sa is None or sb is None:
        if (sa is None) != (sb is None):
            ch.append("glmark2 scene lines %s -> %s" % ("absent" if sa is None else "read",
                                                       "absent" if sb is None else "read"))
        return ch
    names = sb["order"] + [s for s in sa["order"] if s not in sb["scenes"]]
    for name, ca, cb in ([(s, sa["scenes"].get(s, {}), sb["scenes"].get(s, {})) for s in names] +
                         [("outside any scene", sa["outside"], sb["outside"])]):
        if ca != cb:
            ch.append("scene %s: %s" % (name, ", ".join(
                "%s %d -> %d" % (c, ca.get(c, 0), cb.get(c, 0))
                for c, _ in GLMARK2_BAD_LINES if ca.get(c, 0) != cb.get(c, 0))))
    return ch


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
            ch.extend(scene_diff(da, db))
        if da.get("kind") == db.get("kind") == "gl_test":
            if (da.get("passed"), da.get("total")) != (db.get("passed"), db.get("total")):
                ch.append("gl_test passed %s of %s -> %s of %s"
                          % tuple("absent" if x is None else x for x in
                                  (da.get("passed"), da.get("total"), db.get("passed"), db.get("total"))))
            fa, fb = set(da.get("failing", [])), set(db.get("failing", []))
            if fa != fb:
                ch.append("failing +%s -%s" % (sorted(fb - fa), sorted(fa - fb)))
                if da.get("first_only") or db.get("first_only"):
                    ch.append("(a CASE record names its first failure only, or its first two)")
        if a["image"] != b["image"]:
            ch.append("image %s -> %s" % (a["image"], b["image"]))
        lines.append("DIFF %s: %s" % (label, "; ".join(ch) if ch else
                                      "same (verdict %s, record %s)" % (b["verdict"], b["record"])))
    return lines


USAGE = ("usage: common-tally.py <am62p|pi4|imx8mp> <outdir> | --row LABEL=<rundir>... "
         "[--prev <outdir> | --prev-row LABEL=<rundir>...] | --self-test | "
         "--gm-table <console|rundir> | --gm-diff <A> <B> | "
         "--gm-pictures <console|rundir> <column> [--table <file>] | "
         "--gm-picture-lines <console|rundir> <column>")


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if argv[:1] == ["--check-records"] and len(argv) == 2:
        return check_records(argv[1])
    if argv[:1] == ["--gm-table"] and len(argv) == 2:
        return gm_speed_main(argv[0], argv[1:])
    if argv[:1] == ["--gm-diff"] and len(argv) == 3:
        return gm_speed_main(argv[0], argv[1:])
    if argv[:1] in (["--gm-pictures"], ["--gm-picture-lines"]) and (
            len(argv) == 3 or (len(argv) == 5 and argv[3] == "--table")):
        return gm_pictures_main(argv)
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
        # failed 0, unsupported 2), the CASE set failed (passed 3 of 4, first failure 10); a third
        # row, G2, has no run
        out = os.path.join(tmp, "tree")
        os.makedirs(out)
        make_run(out, "GM", "glmark2", ["594b120e", "00000030", "011e0002", "03200258"], "a" * 64)
        make_run(out, "GT", "gl_test", ["594b0101", "000aff06", "00000003", "00000004"], "c" * 64)
        with open(os.path.join(out, "tree.txt"), "w") as f:
            for l, w in (("GM", "glmark2"), ("G2", "glmark2"), ("GT", "gl_test")):
                f.write("%s\t/img/%s/zephyr/zephyr.bin\t-\t%s\n" % (l, l, w))
        rows = tree_rows(out)
        check("tree.txt: three rows, G2 without a run",
              [(l, len(d)) for l, d in rows] == [("GM", 1), ("G2", 0), ("GT", 1)], rows)
        now, lines = tally("imx8mp", rows)
        gm, gt = now["GM"], now["GT"]
        check("GM: glmark2 score 48, ran 30, failed 0, unsupported 2, status pass; verdict done",
              {k: gm["detail"].get(k) for k in ("score", "ran", "failed", "unsupported", "status")} ==
              {"score": 48, "ran": 30, "failed": 0, "unsupported": 2, "status": "pass"} and gm["verdict"] == "done",
              (gm["detail"], gm["verdict"]))
        check("GM: record words, resets absent (never 0), image and build from meta.txt",
              gm["record"] == "594b120e,00000030,011e0002,03200258" and gm["resets"] == "absent" and
              gm["image"] == "a" * 16 and gm["build"] == "GC_BUILD_STAMP=synthetic", gm)
        check("GM: no reset count in a GLMARK2 record and no console: printed 'resets absent', never 0",
              gm["resets_label"] == "resets absent" and any("; resets absent;" in l for l in lines if l.startswith("GM ")),
              (gm["resets_label"], [l for l in lines if l.startswith("GM ")]))
        check("GT: set passed 3 of 4, failing [10] then unknown; verdict failed",
              (gt["detail"].get("passed"), gt["detail"].get("total"), gt["detail"].get("failing")) == (3, 4, [10])
              and "unknown" in gt["detail"]["text"] and gt["verdict"] == "failed", gt)
        check("G2 printed as no run (absent)", now["G2"] is None and "G2 no run (absent)" in lines, lines)
        check("every run's block names its record words and its detail",
              all(any(s["record"] in l for l in lines) and any(s["detail"]["text"] in l for l in lines)
                  for s in now.values() if s), lines)
        rc = subprocess.run([sys.executable, os.path.abspath(__file__), "imx8mp", out],
                            capture_output=True, text=True).returncode
        check("main() exits 1 when a row has no run or a verdict is not done", rc == 1, rc)
        # a GLMARK2 record whose own status is fail (w2 bits 24..27 = 2) is verdict=failed (recordcmd status())
        make_run(out, "G2", "glmark2", ["594b120e", "00000027", "021e0102", "03200258"], "d" * 64)
        g2 = tally("imx8mp", tree_rows(out))[0]["G2"]
        check("a GLMARK2 record with status 2 is verdict failed",
              g2["detail"].get("status") == "fail" and g2["verdict"] == "failed", (g2["detail"], g2["verdict"]))
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

    # the glmark2 per-scene verdict on the committed console snippets (tools/cts/testdata/README.md):
    # clean, terrain's 0x502 and incomplete texture, the AM62P's refused refract draw (a console
    # without glmark2-mem lines), the Pi 4's out of GPU memory and tiled-copy refusals
    def fixture(name):
        return read_text(os.path.join(HERE, "testdata", "glmark2-%s.txt" % name))

    sc = {n: glmark2_scenes(fixture(n)) for n in ("clean", "terrain-0x502", "refract-refused", "pi4-oom")}
    c = sc["clean"]
    check("glmark2 clean: 5 scenes, none with a bad line, nothing outside, read by the memory lines",
          c["order"] == ["[jellyfish] <default>", "[terrain] <default>", "[shadow] <default>", "[refract] <default>",
                         "[conditionals] fragment-steps=0:vertex-steps=0"]
          and not any(c["scenes"].values()) and c["outside"] == {} and c["mem_lines"], c)
    t = sc["terrain-0x502"]
    check("glmark2 terrain: gl-error 16 and incomplete-texture 1 in terrain only",
          t["scenes"]["[terrain] <default>"] == {"gl-error": 16, "incomplete-texture": 1}
          and t["scenes"]["[shadow] <default>"] == {} and t["outside"] == {}, t)
    r = sc["refract-refused"]
    check("glmark2 refract (no memory lines): draw-refused 1 and draw-not-recorded 4 go to refract, not shadow",
          r["scenes"]["[refract] <default>"] == {"draw-refused": 1, "draw-not-recorded": 4}
          and r["scenes"]["[shadow] <default>"] == {} and not r["mem_lines"]
          and r["scenes"]["[conditionals] fragment-steps=0:vertex-steps=0"] == {}, r)
    p = sc["pi4-oom"]
    check("glmark2 Pi 4: refract out-of-memory 1 and tiled-copy 3, terrain tiled-copy 2, shadow clean",
          p["scenes"]["[refract] <default>"] == {"out-of-memory": 1, "tiled-copy": 3}
          and p["scenes"]["[terrain] <default>"] == {"tiled-copy": 2} and p["scenes"]["[shadow] <default>"] == {}, p)
    o = glmark2_scenes(fixture("pi4-oom").replace(
        "15:46:20 glmark2-mem: entry 23 [shadow] <default>: before setup",
        "15:46:20 osal(v3d): out of GPU memory for 4096 bytes (0 frees held; this size refused 1 time)\n"
        "15:46:20 glmark2-mem: entry 23 [shadow] <default>: before setup"))
    check("glmark2: a bad line between one scene's end and the next one's setup is outside any scene",
          o["outside"] == {"out-of-memory": 1} and o["scenes"]["[shadow] <default>"] == {}, o)
    n = glmark2_scenes(fixture("mem-nameless"))
    check("glmark2: the sample's glmark2-mem lines (no scene name) count as memory lines; "
          "the bad lines still go to the next result line",
          n["mem_lines"] and n["order"] == ["[terrain] <default>", "[shadow] <default>"]
          and n["scenes"]["[terrain] <default>"] == {"gl-error": 2} and n["scenes"]["[shadow] <default>"] == {}
          and n["outside"] == {}, n)
    check("glmark2: a console with no glmark2-mem line has mem_lines false",
          not glmark2_scenes("15:00:00 [shadow] <default>: FPS: 5 FrameTime: 200.000 ms\n")["mem_lines"])
    check("glmark2 cell: green, amber, failed over amber, the verdict when not done, record only without a console",
          (glmark2_cell("done", c), glmark2_cell("done", t), glmark2_cell("failed", t), glmark2_cell("timeout", t),
           glmark2_cell("done", None))
          == ("green", "amber", "failed", "timeout", "green (record only)"))

    # the pictures: the committed table's host column has the 33 default scenes and reads back; its
    # one value is all black by design (the host draws nothing), so it matches itself and a coloured
    # picture is a mismatch against it, never an all-black failure
    tab = read_picture_table()
    host = tab["columns"].get("host", {})
    hv = host.get("[terrain] <default>")
    check("glmark2-pictures.txt: tolerance 8 and a host column of 33 scenes, all black",
          tab["tolerance"] == 8 and len(host) == 33 and hv is not None and all(v["black"] for v in host.values()), tab["tolerance"])
    check("glmark2 picture: the host value matches itself, a coloured one is a mismatch, all black on a "
          "coloured reference is black, no reference and no readback are told apart",
          (judge_picture(hv, hv, 8)[0], judge_picture(dict(hv, black=False, mean=(40, 40, 40)), hv, 8)[0],
           judge_picture(hv, dict(hv, black=False), 8)[0], judge_picture(hv, None, 8)[0],
           judge_picture(None, hv, 8)[0]) == ("match", "mismatch", "black", "no-reference", "absent"))
    # a picture line whose value does not parse is unreadable (amber, the raw text), not absent
    vtxt = ("800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; "
            "crc16 7282, crc16q 7282; mean 0 0 0; black yes")
    upics = glmark2_pictures("10:00:00 glmark2-picture: [a] x: %s\n10:00:01 glmark2-picture: [b] x: %sx\n"
                             "10:00:02 glmark2-picture: [c] x: absent (the readback failed)\n" % (vtxt, vtxt))
    uref = parse_picture_value(vtxt)
    ujudged = judge_pictures(upics, {"tolerance": 8, "columns": {"t": {"[a] x": uref, "[b] x": uref, "[c] x": uref}}}, "t")
    check("glmark2 picture: a damaged value is unreadable with its raw text, only a literal absent line is absent, "
          "both make the cell amber",
          ([ujudged["scenes"][k][0] for k in ("[a] x", "[b] x", "[c] x")],
           ujudged["scenes"]["[b] x"][1] == "does not parse: " + (vtxt + "x")[:80], picture_cell(ujudged))
          == (["match", "unreadable", "absent"], True, "amber"), ujudged["scenes"])
    check("glmark2 picture cell: no pictures judged leaves the cell; black is failed, a mismatch amber",
          (picture_cell(None), glmark2_cell("done", c, None),
           glmark2_cell("done", c, {"scenes": {"s": ("black", "")}, "order": ["s"]}),
           glmark2_cell("done", c, {"scenes": {"s": ("mismatch", "")}, "order": ["s"]}),
           glmark2_cell("done", c, {"scenes": {"s": ("match", "")}, "order": ["s"]}))
          == (None, "green", "failed", "amber", "green"))

    # the same snippets as the console of a Pi 4 GM run, through board.sh's verdict: the round-3
    # record (183, 33/0/0, pass; RESULTS.md) and the AM62P 520a6c0f round's failed one (32/1/0)
    gtmp = tempfile.mkdtemp(prefix="common-tally-gm-")
    try:
        def gm_run(name, words):
            d = os.path.join(gtmp, name, "20261004-1520-GM-d0932b24")
            os.makedirs(d)
            with open(os.path.join(d, "meta.txt"), "w") as f:
                f.write("board=pi4\nboot=tryboot\nname=glmark2\nprobe=0\nimage=zephyr.bin\n"
                        "image_sha256=d0932b243a844247%s\nimage_build=YAKOGL_SAMPLE_TREE=42bd90ff\n"
                        "t_reboot=100\nt_back=417\nback=1\n" % ("0" * 48))
            with open(os.path.join(d, "console.txt"), "w") as f:
                f.write(fixture(name) + "15:47:01 yk-record: %s\n" % words)
            return row_summary("pi4", "GM", d)

        green = "594b1201 fc3f00b7 2184003f 03200258"
        rows = {n: gm_run(n, green) for n in ("clean", "terrain-0x502", "pi4-oom")}
        g, a, m = rows["clean"], rows["terrain-0x502"], rows["pi4-oom"]
        check("GM row, clean console: verdict done, cell green, 5 scenes clean",
              g["verdict"] == "done" and g["detail"].get("cell") == "green"
              and "scenes in the console 5, clean 5, with known-bad lines 0" in g["detail"]["scene_lines"][0],
              (g["verdict"], g["detail"]))
        check("GM row, terrain console: verdict done, cell amber, terrain named with 17 lines",
              a["verdict"] == "done" and a["detail"].get("cell") == "amber"
              and "scene [terrain] <default>: amber, 17 lines (gl-error 16, incomplete-texture 1)"
              in a["detail"]["scene_lines"], a["detail"])
        check("GM row, Pi 4 console: cell amber, terrain and refract named",
              m["detail"].get("cell") == "amber" and m["detail"]["scene_lines"][1:] ==
              ["scene [terrain] <default>: amber, 2 lines (tiled-copy 2)",
               "scene [refract] <default>: amber, 4 lines (out-of-memory 1, tiled-copy 3)"], m["detail"])
        _, glines = tally("pi4", [("GM", [m["dir"]])])
        check("GM row printed: the cell line and one line per amber scene under the record's line",
              "    glmark2 cell amber; scenes in the console 3, clean 1, with known-bad lines 2" in glines
              and "    scene [refract] <default>: amber, 4 lines (out-of-memory 1, tiled-copy 3)" in glines, glines)
        fl = gm_run("refract-refused", "594b1201 ffd701d6 2280103f 03200258")
        check("GM row, a failed record with a refused draw: verdict failed, cell failed, refract still named",
              fl["verdict"] == "failed" and fl["detail"].get("cell") == "failed"
              and "scene [refract] <default>: amber, 5 lines (draw-refused 1, draw-not-recorded 4)"
              in fl["detail"]["scene_lines"], (fl["verdict"], fl["detail"]))
        dl = diff({"GM": a}, {"GM": g})[0]
        check("diff: the cell and each scene's changed counts",
              all(x in dl for x in ("glmark2 cell amber -> green",
                                    "scene [terrain] <default>: gl-error 16 -> 0, incomplete-texture 1 -> 0")), dl)
        dl = diff({"GM": g}, {"GM": m})[0]
        check("diff: scenes that gained lines, and a scene only in the earlier run",
              all(x in dl for x in ("glmark2 cell green -> amber", "scene [terrain] <default>: tiled-copy 0 -> 2",
                                    "scene [refract] <default>: out-of-memory 0 -> 1, tiled-copy 0 -> 3"))
              and "jellyfish" not in dl, dl)
        dl = diff({"GM": g}, {"GM": g})[0]
        check("diff: a GM row with itself is the same", ": same (" in dl, dl)
    finally:
        shutil.rmtree(gtmp)

    # the label's three sources, in order (the console count is board.sh's count of recovery lines)
    check("resets label: the record's value first, else the console count, else absent",
          (resets_label("2", "2"), resets_label("absent", "0"), resets_label(None, "3"), resets_label("absent", "absent"))
          == ("resets (record) 2", "resets (console count) 0", "resets (console count) 3", "resets absent"))

    # 2. the diff: identical runs are the same; a changed record names each field
    d_same = diff(now, now)
    check("diff of a run with itself: 'same' for each run, 'no run -> no run' for G2",
          sum(": same (" in l for l in d_same) == 2 and "DIFF G2: no run -> no run" in d_same, d_same)
    alt = dict(now)
    alt["GM"] = dict(gm, detail=dict(gm["detail"], score=41, failed=1), verdict="failed", record="x")
    alt["GT"] = dict(gt, detail=dict(gt["detail"], failing=[], passed=4))
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
    gone = {k: v for k, v in now.items() if k != "GT"}
    dg = {l.split(":")[0]: l for l in diff(now, gone)}
    check("diff: a row missing from the new run", dg.get("DIFF GT") == "DIFF GT: run -> no run", dg.get("DIFF GT"))

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
                               "case 171 alone, check 8 failed: failing [171]"),
                              # CASE_SET (record.rs SetSummary): case 16 skipped, one failure
                              (["594b010e", "002c0107", "ffff002f", "00100030"], (47, 48, [44]),
                               "set: 47 of 48 run, 1 skipped (16), failing [44]"),
                              (["594b010e", "ffff0107", "ffff0030", "00100030"], (48, 48, []),
                               "set: 48 of 48 run, 1 skipped (16), failing none"),
                              (["594b010e", "002c0107", "0002002c", "00100030"], (44, 48, [44, 2]),
                               "set: 44 of 48 run, failing [44, 2] then two unknown")):
        p = subprocess.run([ytools(), "record"] + words, capture_output=True, text=True)
        r = detail_from_record(p.stdout)
        check("CASE " + what, (r.get("passed"), r.get("total"), r.get("failing")) == want, (p.stdout, r))
        if words[1].endswith("07"):
            check("CASE " + what + ": the skip and the unknown failures are said",
                  r.get("skipped") == 1 and r.get("first_skip") == 16 and
                  bool(r.get("first_only")) == (want[0] == 44) and
                  ("2 unknown" in r["text"]) == (want[0] == 44), (p.stdout, r))

    print("common-tally self-test: %d checks failed" % len(fails))
    return 1 if fails else 0


def check_records(base):
    """the first i.MX8MP common run (2026-10-04), where it lies: one board.sh run per row under
    <base>/glmark2 and gltest171 (not in the repository: records stay out of git); its Slint rows
    are not read, the Slint sample being a demo and no row of the common set"""
    print("common-tally: build %s (check-records %s)" % (describe(), base))
    specs = []
    for label, sub in (("GM", "glmark2"), ("GT", "gltest171")):
        runs = sorted(n for n in os.listdir(os.path.join(base, sub)) if RUN_RE.match(n))
        if len(runs) != 1:
            print("FAIL %s: %d run directories in %s" % (label, len(runs), sub))
            return 1
        specs.append("%s=%s" % (label, os.path.join(base, sub, runs[0])))
    now, lines = tally("imx8mp", explicit_rows(specs))
    print("\n".join(lines))
    want = {  # read from each run's INDEX.txt "records:" line (2026-10-04)
        "GM": ("594b120e,00000027,021e0102,03200258", "failed", "glmark2 score 39; scenes ran 30, failed 1, unsupported 2"),
        "GT": ("594b010e,00ab0802,ffff0000,ff0000ff", "failed", "gl_test case 171 alone: check failed"),
    }
    bad = 0
    for label, (rec, verdict, text) in want.items():
        s = now[label]
        ok = s["record"] == rec and s["verdict"] == verdict and s["detail"]["text"].startswith(text)
        print("%s %s: record %s verdict %s, '%s...'" % ("ok  " if ok else "FAIL", label, rec, verdict, text))
        bad += not ok
    print("common-tally check-records: %d of %d rows differ" % (bad, len(want)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
