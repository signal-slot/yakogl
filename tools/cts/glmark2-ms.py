#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-scene ms a frame from a glmark2-es2 board console log, as a Markdown table.

    tools/cts/glmark2-ms.py <console log>             the table
    tools/cts/glmark2-ms.py --cells <console log>     terrain/shadow/refract cells for RESULTS.md

The log has (driver/docs/glmark2.md, section 1), per scene, upstream's line

    [<scene>] <options>: FPS: <n> FrameTime: <ms> ms     (or "Set up failed" / "Unsupported")

each optionally behind a console timestamp (or cut in two by a library log
line after the scene prefix, the result then on a line of its own), then glmark2's `glmark2 Score: <n>`
banner and the sample's `glmark2: score <n>: <r> scenes ran, <f> failed,
<u> unsupported` line. The table has one row per scene (scene string,
FrameTime ms, FPS) and the final score. `--cells` prints
`terrain 178.86 ms (6 FPS), shadow ..., refract ...`: figures in ms a frame,
never fps alone.

Exit 0: parsed. 1: a scene line damaged (cut off, FPS without FrameTime, a
scene listed twice), a scene that started (`glmark2-mem: entry N [scene]`) and
printed no result, the ran count of the summary line disagreeing with the
lines, no score, or (with --cells) a requested scene without a time. 2: no
glmark2 output in the log. Stdlib only.
"""
import re
import sys

CELL_SCENES = ("terrain", "shadow", "refract")

RESULT_RE = re.compile(r"(?:^|\s)(\[[\w-]+\] .*?): (FPS: .*|Set up failed.*|Unsupported.*)$")
GOOD_RE = re.compile(r"FPS: (\d+) FrameTime: (\d+(?:\.\d+)?) ms\s*$")
START_RE = re.compile(r"glmark2-mem: entry \d+ (\[[\w-]+\] .*?): before setup")
SCORE_RE = re.compile(r"glmark2 Score: (\d+)")
SUMMARY_RE = re.compile(r"glmark2: score (\d+): (\d+) scenes ran, (\d+) failed, (\d+) unsupported")
# A scene's line cut in two by a library log line printed between glmark2's
# scene prefix and its result (the first kick of a new render target logs
# its HWRT on the AM62P): "[shadow] <default>:I: render: ..." and, lines
# later, " FPS: 95 FrameTime: 10.636 ms" on its own. The prefix is the scene
# string (its options have no space) with the log line's own prefix right
# after its colon (a Zephyr level "I: ", the library's "glmark2:", ...); the
# bare result line, with nothing but a console timestamp before it,
# completes it.
PREFIX_RE = re.compile(r"(?:^|\s)(\[[\w-]+\] \S+?):(?=[IWED]: |glmark2|osal|yakogl|shaderc)")
BARE_RE = re.compile(r"^(?:\S+ )?\s*(FPS: .*|Set up failed.*|Unsupported.*)$")


def parse(text):
    """-> (rows [(scene, ms, fps, note)], started scenes, score, summary, problems)"""
    rows, started, problems = [], [], []
    score = summary = None
    seen = set()
    pending = None  # a started scene whose prefix was seen, its result not yet
    for line in text.splitlines():
        m = START_RE.search(line)
        if m:
            started.append(m.group(1))
            continue
        m = SCORE_RE.search(line)
        if m:
            score = int(m.group(1))
            continue
        m = SUMMARY_RE.search(line)
        if m:
            summary = tuple(int(g) for g in m.groups())
            if score is None:
                score = summary[0]
            continue
        m = RESULT_RE.search(line)
        if m:
            scene, rest = m.group(1), m.group(2)
            pending = None
        elif pending is not None and BARE_RE.search(line):
            scene, rest = pending, BARE_RE.search(line).group(1)
            pending = None
        else:
            p = PREFIX_RE.search(line)
            if p and p.group(1) not in seen:
                pending = p.group(1)
            continue
        if scene in seen:
            problems.append("scene listed twice: %s" % scene)
            continue
        seen.add(scene)
        if rest.startswith("FPS:"):
            g = GOOD_RE.match(rest)
            if g:
                rows.append((scene, float(g.group(2)), int(g.group(1)), None))
            else:
                problems.append("damaged scene line: %s" % line.strip())
                rows.append((scene, None, None, "damaged"))
        else:
            rows.append((scene, None, None, rest.split(":")[0].strip().lower()))
    return rows, started, score, summary, problems


def main(argv):
    cells = "--cells" in argv
    args = [a for a in argv if a != "--cells"]
    if len(args) != 1 or args[0].startswith("-"):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    with open(args[0], encoding="utf-8", errors="replace") as f:
        text = f.read()
    rows, started, score, summary, problems = parse(text)
    if not rows and score is None:
        print("error: no glmark2 output in %s" % args[0], file=sys.stderr)
        return 2
    have = {r[0] for r in rows}
    for s in started:
        if s not in have:
            problems.append("scene started, no result line: %s" % s)
    ran = sum(1 for r in rows if r[1] is not None)
    if summary and summary[1] != ran:
        problems.append("summary says %d scenes ran, %d result lines read" % (summary[1], ran))
    if score is None:
        problems.append("no glmark2 score line")

    if cells:
        out = []
        for name in CELL_SCENES:
            r = next((r for r in rows if r[0].startswith("[%s] " % name) and r[1] is not None), None)
            if r is None:
                problems.append("no ms a frame for %s" % name)
            else:
                out.append("%s %.2f ms (%d FPS)" % (name, r[1], r[2]))
        if not problems:
            print(", ".join(out))
    else:
        print("| scene | FrameTime ms | FPS |")
        print("|---|---:|---:|")
        for scene, ms, fps, note in rows:
            if ms is None:
                print("| %s | %s | - |" % (scene, note))
            else:
                print("| %s | %.2f | %d |" % (scene, ms, fps))
        print()
        print("glmark2 Score: %s" % ("-" if score is None else score))
    for p in problems:
        print("FAIL: %s" % p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
