#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tally one STM32MP257F-DK glmark2-es2 console log: per-scene FPS and frame
time, failed / unsupported / hung scenes, and the score.

  python3 -I tools/cts/mp2-glmark2-tally.py [--json] [--strict] LOG|-

LOG is the console text of driver/samples/zephyr/glmark2 on the DK (the record
tools/board-stm32mp2.sh keeps), or - for stdin. The lines it parses, all of
driver/samples/zephyr/glmark2/src/main.cpp:

  glmark2: upstream <commit> (<ver>), size WxH, args "..."   main.cpp:1527
  glmark2-mem: entry N before setup: ...    scene started   main.cpp:1153, 1281
  glmark2-mem: entry N end: ...             scene finished  main.cpp:1198, 1281
  [scene] opts: FPS: N FrameTime: X ms      upstream's result line, via
                                            Log::info (main.cpp:1232-1233
                                            cites main-loop.cpp:178)
  [scene] opts: Set up failed | Unsupported upstream's failure forms of it
  glmark2: entry N <scene>: unsupported     main.cpp:1210
  glmark2: entry N <scene>: failed, why W   main.cpp:1218
  glmark2 Score: N                          main.cpp:1646
  glmark2: score N: R scenes ran, F failed, U unsupported (L list entries);
      window WxH..., shown yes|no, GPU resets G           main.cpp:1653
  glmark2: GL errors raised N, draws that sampled an incomplete texture M
                                                           main.cpp:1665

Scene verdicts: ok (an FPS), failed, unsupported, hung (the scene started
- "before setup" - and the log ends with no result and no score), no-result
(started, never finished, but the run went on). The run is complete (the
"glmark2: score" line is there) or truncated (it is not). The text prints
one row per scene (scene, verdict, ms/frame, FPS) and the summary; --json
the same as one object. The score is the run's own (main.cpp:1646/1653), never
recomputed here; when only one of the two lines is there it is used, and a
disagreement is noted.

Exit: 0 complete and every scene ok or unsupported; 1 a scene failed, hung or
has no result, or the score is 0 (--strict: an unsupported scene too);
3 truncated; 2 usage.
"""
import json
import re
import sys

RESULT = re.compile(r"^\s*(\[[^\]]*\].*?):?\s+FPS:\s*(\d+)(?:\s+FrameTime:\s*([0-9.]+)\s*ms)?\s*$")
NORESULT = re.compile(r"^\s*(\[[^\]]*\].*?):?\s+(Set up failed|Unsupported)\s*$")
MEM = re.compile(r"^\s*glmark2-mem: entry (\d+) (before setup|end):")
ENTRY = re.compile(r"^\s*glmark2: entry (\d+) (.*?): (unsupported|failed, why (\d+))\s*$")
SCORE_LOG = re.compile(r"^\s*glmark2 Score:\s*(\d+)\s*$")
SCORE = re.compile(r"^\s*glmark2: score (\d+): (\d+) scenes ran, (\d+) failed, (\d+) unsupported "
                   r"\((\d+) list entries\); window (\d+)x(\d+)(?: off-screen)?, shown (yes|no), "
                   r"GPU resets (\d+)")
GLERR = re.compile(r"^\s*glmark2: GL errors raised (\d+), draws that sampled an incomplete "
                   r"texture (\d+)")


def tally(text):
    scenes = {}     # entry -> dict
    order = []
    cur = None      # the entry between "before setup" and "end"
    res = {"score": None, "score_line": None, "summary": None, "gl_errors": None}

    def scene(n):
        if n not in scenes:
            scenes[n] = {"entry": n, "scene": None, "verdict": None, "fps": None,
                         "ms": None, "why": None, "started": False}
            order.append(n)
        return scenes[n]

    for line in text.splitlines():
        m = MEM.match(line)
        if m:
            s = scene(int(m.group(1)))
            if m.group(2) == "end":
                cur = None
            else:
                s["started"] = True
                cur = s
            continue
        m = ENTRY.match(line)
        if m:
            s = scene(int(m.group(1)))
            s["scene"] = s["scene"] or m.group(2)
            if m.group(3) == "unsupported":
                s["verdict"] = "unsupported"
            else:
                s["verdict"] = "failed"
                s["why"] = int(m.group(4))
            continue
        m = RESULT.match(line)
        if m and cur is not None:
            fps = int(m.group(2))
            cur["scene"] = m.group(1)
            cur["fps"] = fps
            cur["ms"] = float(m.group(3)) if m.group(3) else (1000.0 / fps if fps else None)
            cur["verdict"] = "ok"
            continue
        m = NORESULT.match(line)
        if m and cur is not None:
            cur["scene"] = m.group(1)
            if cur["verdict"] is None:
                cur["verdict"] = "failed" if m.group(2) == "Set up failed" else "unsupported"
            continue
        m = SCORE_LOG.match(line)
        if m:
            res["score_line"] = int(m.group(1))
            continue
        m = SCORE.match(line)
        if m:
            g = [int(x) if x.isdigit() else x for x in m.groups()]
            res["score"] = g[0]
            res["summary"] = {"ran": g[1], "failed": g[2], "unsupported": g[3], "entries": g[4],
                              "window": "%dx%d" % (g[5], g[6]), "shown": g[7] == "yes",
                              "gpu_resets": g[8]}
            continue
        m = GLERR.match(line)
        if m:
            res["gl_errors"] = {"raised": int(m.group(1)), "incomplete_draws": int(m.group(2))}

    complete = res["score"] is not None
    rows = []
    for i, n in enumerate(order):
        s = scenes[n]
        if s["verdict"] is None:
            last = i == len(order) - 1
            s["verdict"] = "hung" if (last and not complete and s["started"]) else "no-result"
        s["scene"] = s["scene"] or "entry %d" % n
        del s["started"]
        rows.append(s)

    score = res["score"] if res["score"] is not None else res["score_line"]
    notes = []
    if res["score"] is not None and res["score_line"] is not None and res["score"] != res["score_line"]:
        notes.append("score lines disagree: %d vs %d" % (res["score_line"], res["score"]))
    sm = res["summary"]
    if sm and rows:
        names = {"ran": "ok", "failed": "failed", "unsupported": "unsupported"}
        for k, v in names.items():
            seen = sum(1 for r in rows if r["verdict"] == v)
            if seen != sm[k]:
                notes.append("summary says %d %s, the scene lines show %d" % (sm[k], k, seen))
    count = {}
    for r in rows:
        count[r["verdict"]] = count.get(r["verdict"], 0) + 1
    return {"complete": complete, "score": score, "scenes": rows, "counts": count,
            "summary": sm, "gl_errors": res["gl_errors"], "notes": notes}


def exit_code(t, strict):
    if not t["complete"]:
        return 3
    bad = {"failed", "hung", "no-result"} | ({"unsupported"} if strict else set())
    if any(r["verdict"] in bad for r in t["scenes"]) or not t["score"]:
        return 1
    return 0


def render(t):
    out = ["%-4s %-48s %-12s %9s %5s" % ("#", "scene", "verdict", "ms/frame", "FPS")]
    for r in t["scenes"]:
        out.append("%-4d %-48s %-12s %9s %5s" % (
            r["entry"], r["scene"][:48], r["verdict"],
            "-" if r["ms"] is None else "%.3f" % r["ms"], "-" if r["fps"] is None else r["fps"]))
    out.append("")
    out.append("run: %s; score: %s; %s" % (
        "complete" if t["complete"] else "TRUNCATED (no 'glmark2: score' line)",
        "absent" if t["score"] is None else t["score"],
        ", ".join("%d %s" % (v, k) for k, v in sorted(t["counts"].items())) or "no scenes"))
    if t["summary"]:
        out.append("window %s, shown %s, GPU resets %d" % (
            t["summary"]["window"], "yes" if t["summary"]["shown"] else "no",
            t["summary"]["gpu_resets"]))
    if t["gl_errors"]:
        out.append("GL errors raised %d, incomplete-texture draws %d" % (
            t["gl_errors"]["raised"], t["gl_errors"]["incomplete_draws"]))
    out.extend("note: " + n for n in t["notes"])
    return "\n".join(out)


def main(argv):
    args = [a for a in argv if a not in ("--json", "--strict")]
    if any(a in ("-h", "--help") for a in args):
        print(__doc__)
        return 0
    if len(args) != 1:
        sys.stderr.write("usage: mp2-glmark2-tally.py [--json] [--strict] LOG|-\n")
        return 2
    try:
        if args[0] == "-":
            text = sys.stdin.read()
        else:
            with open(args[0], encoding="utf-8", errors="replace") as f:
                text = f.read()
    except OSError as e:
        sys.stderr.write("mp2-glmark2-tally: %s\n" % e)
        return 2
    t = tally(text)
    print(json.dumps(t, indent=2) if "--json" in argv else render(t))
    return exit_code(t, "--strict" in argv)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
