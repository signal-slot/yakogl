#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""glmark2-results-row.py - one RESULTS.md glmark2 cell from a board's glmark2 run.

    tools/cts/glmark2-results-row.py --board pi4 --commit 0ee17984 \\
        --record 20261124-1200-glmark2-0ee17984 \\
        --verdicts <common-tally.py GM lines> --ms <glmark2-ms.py output> \\
        [--pictures '<summary line>'] [--prior '<text>'] [--allow-no-console]

Prints the board's 'glmark2-es2' cell for tools/cts/RESULTS.md ('Common set'
table) to stdout, in the style of the existing Pi 4 and AM62P rows:

    <score>, 33 scenes, 0 failed, 0 unsupported, 0 GL errors, 0 out-of-memory
    scenes; main `<sha8>`, <date> (`records/<board>/<dir>/`; GM
    `verdict=done`, resets N, record words `...`); **terrain X ms a frame
    (F FPS), shadow Y ms (F FPS), refract Z ms (F FPS)** (slow-scene times
    recorded, not judged); pictures against `tools/cts/glmark2-pictures.txt`
    column `<column>`: <summary>; terrain picture <state>, refract picture
    <state>[; <prior>]

This is exactly the cell readme-glmark2-check.py --scenes terrain,shadow,
refract reads from a RESULTS row: the commit in backticks, '<scene> <n> ms'
for every named scene (markdown bold ignored), and a record directory
records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/ directly under records/<board>/
at that commit (--record is the directory name; anything else, e.g. a run
inside a tree outdir, is refused so the row cites the path the checker reads).

Inputs:
  --board    am62p, pi4 or imx8mp (the records/<board>/ directory name).
  --commit   the main commit the run was built from, as a sha8.
  --record   the run directory name <YYYYMMDD-HHMM>-<name>-<sha8>; its sha8
             must equal --commit, and its date becomes the cell's date.
  --verdicts common-tally.py's lines of one GM run: the verdict line
             "GM <name> rc=.. verdict=.. result=.. record=<words>; resets ..;
             image=.. build=..", "glmark2 score N; scenes ran R, failed F,
             unsupported U; ...", the cell line — "glmark2 cell C; scenes in
             the console ...", or without a console "glmark2 cell C; per-scene
             lines absent (no console)" or the glmark2-pages.txt form "glmark2
             cell C; scenes in glmark2-pages.txt N, ..." (the i.MX8MP) — the
             "scene [x] ...: amber, n lines (...)" and "outside any scene:
             ..." lines, and the "glmark2 pictures against column X: N scenes,
             match M..." line with its "scene [x] ...: picture <state> (why)"
             lines. Only one run's block: two GM verdict lines refuse.
  --ms       glmark2-ms.py's output: the Markdown table (every scene's
             FrameTime ms and FPS, then "glmark2 Score: N") or its --cells
             line ("terrain 178.86 ms (6 FPS), shadow ..., refract ...").
  --pictures a picture summary line (same form as common-tally's) used instead
             of the one in --verdicts, for a board whose pictures were judged
             outside common-tally.py.
  --prior    text appended to the cell (the previous-commit comparison, e.g.
             "195 also at `c4af4917`"); it may hold no '|' or newline.
  --allow-no-console  accept a GM row without a console: the cell then says
             GL errors and out-of-memory scenes were not read, naming the
             tally cell, instead of claiming 0.

Refuses (exit 2, a message on stderr, nothing on stdout) when an input is
missing or unreadable, --board or --commit is not a name/sha8, --record is not
<YYYYMMDD-HHMM>-<name>-<sha8> at --commit, the verdict is not done or rc not
0, the counts are not 33 ran / 0 failed / 0 unsupported, the console has a GL
error line, an out-of-memory line (in a scene or outside any scene) or any
other known-bad line (an amber cell), a terrain/shadow/refract time is
missing, the --ms table does not have 33 timed scenes, the --ms score and the
record score disagree, the picture summary is absent, has a black scene, or
does not name every not-match scene it counts, --prior holds a '|' or a
newline, or --allow-no-console was not passed for a console-less row.
Exit 0: the cell printed, one line. Stdlib only.
"""
import argparse
import os
import re
import sys

SCENES = ("terrain", "shadow", "refract")
PIC_SCENES = ("terrain", "refract")
WANT_RAN = 33
BOARDS = ("am62p", "pi4", "imx8mp")

VERDICT_RE = re.compile(r"^\s*GM(?:#\d+)? \S+ rc=(\S+) verdict=(\S+) result=\S+ record=([^;]*); ([^;]*);")
SCORE_RE = re.compile(r"glmark2 score (\d+); scenes ran (\d+), failed (\d+), unsupported (\d+)")
# common-tally.py names the cell green, amber or failed, and without a console
# glmark2_cell gives "green (record only)", and with the glmark2-pages.txt
# pictures "green (record and pages)", whose line (pages_lines) names
# glmark2-pages.txt instead of the console
CELL_RE = re.compile(r"glmark2 cell (\S+(?: \([a-z][a-z ()]+\))?); "
                     r"(?:scenes in (the console|glmark2-pages\.txt) (\d+)|per-scene lines absent \(no console\))")
AMBER_RE = re.compile(r"^\s*(?:scene (\[[\w-]+\] .*?): amber,|outside any scene:) \d+ lines \((.*)\)\s*$")
PIC_SUM_RE = re.compile(r"glmark2 pictures against column (\S+): (\d+) scenes, match (\d+)(.*)$")
PIC_SCENE_RE = re.compile(r"^\s*scene \[([\w-]+)\] .*?: picture (.+?) \((.*)\)\s*$")
CELLS_RE = re.compile(r"\b(%s) (\d+(?:\.\d+)?) ms \((\d+) FPS\)" % "|".join(SCENES))
TABLE_RE = re.compile(r"^\|\s*(\[[\w-]+\] [^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*$")
MS_SCORE_RE = re.compile(r"glmark2 Score: (\d+|-)")
RECORD_RE = re.compile(r"^(?:records/([a-z0-9]+)/)?(\d{4})(\d\d)(\d\d)-\d{4}-[A-Za-z0-9._-]+-([0-9a-f]{8})/?$")


class Refuse(Exception):
    pass


def read(path, what):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError as e:
        raise Refuse("%s %s: %s" % (what, path, e.strerror))


def counts(text):
    """'gl-error 16, incomplete-texture 1' -> {class: n}; prose segments are ignored"""
    out = {}
    for part in text.split(","):
        bits = part.strip().rsplit(" ", 1)
        if len(bits) == 2 and bits[1].isdigit():
            out[bits[0]] = out.get(bits[0], 0) + int(bits[1])
    return out


def parse_verdicts(text, allow_no_console):
    """-> the GM row's fields: verdict, score and counts, cell and console,
    known-bad line totals per class, out-of-memory scenes, picture summary and
    picture scene states"""
    v = {"pictures": None, "picture_scenes": {}, "gl": 0, "oom": 0, "other_bad": 0,
         "oom_scenes": [], "oom_outside": 0}
    for line in text.splitlines():
        m = VERDICT_RE.search(line)
        if m:
            if "rc" in v:
                raise Refuse("--verdicts names two GM runs: pass one run's block")
            v["rc"], v["verdict"], v["words"], v["resets"] = (g.strip() for g in m.groups())
            continue
        m = SCORE_RE.search(line)
        if m:
            v["score"], v["ran"], v["failed"], v["unsupported"] = (int(g) for g in m.groups())
            continue
        m = CELL_RE.search(line)
        if m:
            # only a console carries the GL error and out-of-memory lines: the
            # glmark2-pages.txt form of the line (the i.MX8MP) is console-less
            v["cell"], v["console"] = m.group(1), m.group(2) == "the console"
            continue
        m = AMBER_RE.search(line)
        if m:
            scene, c = m.group(1), counts(m.group(2))
            gl, oom = c.get("gl-error", 0), c.get("out-of-memory", 0)
            v["gl"] += gl
            v["oom"] += oom
            # an amber cell also rests on the classes that are neither GL
            # errors nor out-of-memory: incomplete-texture, draw-refused,
            # draw-not-recorded, tiled-copy lines (common-tally.py
            # GLMARK2_BAD_LINES); a clean row has none of them, in a scene or
            # outside one
            v["other_bad"] += sum(n for k, n in c.items() if k not in ("gl-error", "out-of-memory"))
            if oom:
                if scene:
                    v["oom_scenes"].append(scene)
                else:
                    v["oom_outside"] += oom
            continue
        m = PIC_SUM_RE.search(line)
        if m:
            v["pictures"] = line.strip()
            continue
        m = PIC_SCENE_RE.search(line)
        if m:
            v["picture_scenes"][m.group(1)] = (m.group(2), m.group(3))
    for key, what in (("rc", "the 'GM ... rc=.. verdict=..' line"),
                      ("score", "the 'glmark2 score N; scenes ran R, failed F, unsupported U' line"),
                      ("cell", "the 'glmark2 cell ...' line")):
        if key not in v:
            raise Refuse("--verdicts has no %s" % what)
    if v["rc"] != "0" or v["verdict"] != "done":
        raise Refuse("GM rc=%s verdict=%s, want rc=0 verdict=done" % (v["rc"], v["verdict"]))
    if (v["ran"], v["failed"], v["unsupported"]) != (WANT_RAN, 0, 0):
        raise Refuse("glmark2 scenes ran %d, failed %d, unsupported %d; want %d ran, 0 failed, 0 unsupported"
                     % (v["ran"], v["failed"], v["unsupported"], WANT_RAN))
    if not v["console"] and not allow_no_console:
        raise Refuse("the GM row has no console (GL errors and out-of-memory scenes unread); "
                     "pass --allow-no-console to say so in the cell")
    if v["gl"]:
        raise Refuse("the GM console has %d GL error lines" % v["gl"])
    if v["oom"]:
        noun = "line" if v["oom"] == 1 else "lines"
        where = ", ".join("scene %s" % s for s in v["oom_scenes"])
        if v["oom_outside"]:
            where += ", " if where else ""
            where += "%d %s outside any scene" % (
                v["oom_outside"], "line" if v["oom_outside"] == 1 else "lines")
        raise Refuse("the GM console has %d out-of-GPU-memory %s (%s)" % (v["oom"], noun, where))
    if v["other_bad"]:
        raise Refuse("the GM console has known-bad lines that are neither GL errors nor "
                     "out-of-memory (%d line%s, the cell is amber); name them in the row's "
                     "--prior text if the row still cites them"
                     % (v["other_bad"], "" if v["other_bad"] == 1 else "s"))
    return v


def parse_ms(text):
    """-> ({scene: (ms text, fps)} for SCENES, number of timed table rows or None, score or None)"""
    cells, rows, score = {}, None, None
    for line in text.splitlines():
        m = TABLE_RE.match(line)
        if m:
            rows = rows or []
            rows.append(m.groups())
            continue
        m = MS_SCORE_RE.search(line)
        if m and m.group(1) != "-":
            score = int(m.group(1))
            continue
        for m in CELLS_RE.finditer(line):
            cells.setdefault(m.group(1), (m.group(2), int(m.group(3))))
    timed = None
    if rows is not None:
        timed = 0
        for scene, ms, fps in rows:
            if not re.fullmatch(r"\d+(?:\.\d+)?", ms) or not fps.isdigit():
                raise Refuse("--ms: scene %s has no time (%s)" % (scene, ms))
            timed += 1
            name = scene[1:scene.index("]")]
            if name in SCENES and name not in cells:
                cells[name] = ("%.2f" % float(ms), int(fps))
    for s in SCENES:
        if s not in cells:
            raise Refuse("--ms has no ms a frame for %s" % s)
    return cells, timed, score


def picture_text(v, override):
    """the picture section of the cell: the summary line as common-tally.py
    prints it, then terrain's and refract's picture states; refuses a black
    scene and a summary that does not name every not-match scene it counts"""
    summary = override or v["pictures"]
    if not summary:
        raise Refuse("no picture summary: neither --pictures nor a 'glmark2 pictures against column' line")
    m = PIC_SUM_RE.search(summary)
    if not m:
        raise Refuse("the picture summary is not a 'glmark2 pictures against column X: N scenes, match M...' line")
    col, n, match = m.group(1).strip("`"), int(m.group(2)), m.group(3)
    rest = m.group(4).strip(", ")
    if n != WANT_RAN:
        raise Refuse("the picture summary has %d scenes, want %d" % (n, WANT_RAN))
    extra = counts(rest)
    if "black" in rest:
        raise Refuse("the picture summary has a black scene: %s" % rest)
    states = v["picture_scenes"]
    notmatch = n - int(match) - extra.get("not-in-record", 0)
    named = [s for s, (state, _) in states.items() if state != "match"]
    if notmatch != len(named):
        raise Refuse("the picture summary counts %d not-match scenes, the verdict lines name %d "
                     "(%s): name every scene the summary does not count as a match"
                     % (notmatch, len(named), ", ".join(sorted(states))))
    if extra.get("black"):
        raise Refuse("the picture summary counts %d black scenes" % extra["black"])
    # a scene is named by a picture line when it is not a match or when it was
    # not judged (not in record); with every not-match named, an unnamed
    # terrain/refract is a match, so the cell can say so
    out = []
    for s in PIC_SCENES:
        st = states.get(s)
        out.append("%s picture %s" % (s, "%s (%s)" % st if st else "match"))
    head = "pictures against `tools/cts/glmark2-pictures.txt` column `%s`: %d scenes, match %s%s" % (
        col, n, match, (", " + rest) if rest else "")
    return "%s; %s" % (head, ", ".join(out))


def make_cell(a):
    if a.board not in BOARDS:
        raise Refuse("--board %s, want one of %s" % (a.board, ", ".join(BOARDS)))
    if not re.fullmatch(r"[0-9a-f]{8}", a.commit):
        raise Refuse("--commit %s is not a sha8" % a.commit)
    rec = a.record.rstrip("/")
    m = RECORD_RE.match(rec + "/")
    if not m:
        raise Refuse("--record %s is not <YYYYMMDD-HHMM>-<name>-<sha8>" % a.record)
    if m.group(1) and m.group(1) != a.board:
        raise Refuse("--record %s is not under records/%s/" % (a.record, a.board))
    if m.group(5) != a.commit:
        raise Refuse("--record %s is not at %s" % (a.record, a.commit))
    recdir = "records/%s/%s/" % (a.board, os.path.basename(rec))
    date = "%s-%s-%s" % (m.group(2), m.group(3), m.group(4))
    if a.prior:
        for badch in ("|", "\n"):
            if badch in a.prior:
                raise Refuse("--prior contains %r, which would split the table cell" % badch)

    v = parse_verdicts(read(a.verdicts, "--verdicts"), a.allow_no_console)
    cells, timed, ms_score = parse_ms(read(a.ms, "--ms"))
    if timed is not None and timed != WANT_RAN:
        raise Refuse("--ms table has %d timed scenes, want %d" % (timed, WANT_RAN))
    if ms_score is not None and ms_score != v["score"]:
        raise Refuse("--ms score %d, the GM record's %d" % (ms_score, v["score"]))
    pics = picture_text(v, a.pictures)

    if v["console"]:
        counts = "%d, %d scenes, 0 failed, 0 unsupported, 0 GL errors, 0 out-of-memory scenes" % (
            v["score"], WANT_RAN)
    else:
        # a console-less record (the i.MX8MP): the record's status is pass and
        # its counts are 33/0/0, but GL error lines and out-of-memory lines
        # are only in a console, so the cell says they were not read
        counts = ("%d, %d scenes, 0 failed, 0 unsupported; GL errors and out-of-memory "
                  "not read (no console; tally cell %s)" % (v["score"], WANT_RAN, v["cell"]))
    resets = re.sub(r"^resets \((?:record|console count)\) ", "resets ", v["resets"])
    t, s, r = (cells[x] for x in SCENES)
    out = ("%s; main `%s`, %s (`%s`; GM `verdict=done`, %s, record words `%s`); "
           "**terrain %s ms a frame (%d FPS), shadow %s ms (%d FPS), refract %s ms (%d FPS)** "
           "(slow-scene times recorded, not judged); %s"
           % (counts, a.commit, date, recdir, resets, v["words"],
              t[0], t[1], s[0], s[1], r[0], r[1], pics))
    if a.prior:
        out += "; " + a.prior.strip()
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--board", required=True)
    ap.add_argument("--commit", required=True)
    ap.add_argument("--record", required=True)
    ap.add_argument("--verdicts", required=True)
    ap.add_argument("--ms", required=True)
    ap.add_argument("--pictures")
    ap.add_argument("--prior")
    ap.add_argument("--allow-no-console", action="store_true")
    a = ap.parse_args(argv)  # a missing required option exits 2 by argparse
    try:
        print(make_cell(a))
    except Refuse as e:
        print("glmark2-results-row: refused: %s" % e, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
