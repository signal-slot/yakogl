#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Arm A / arm E confirmation of three mip levels a render (AM62P, arm 7).

    tools/cts/mip3-ab-verdict.py [--c <ms>] <arm A record dir> <arm E record dir>

Each argument is a record directory (records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/)
or one console file. A directory's *.txt and *.log files are read together
(console.txt is the console). The lines read, per arm:

  gl_test: mipchains: 20 frames, X ms a frame (...)        case 216 ms
  gl_test:   per frame: N.NN kicks (M over 20 frames)      case 216 kicks
  gl_test: case 216 mipchains: PASS                         case 216 verdict
  gl_test: kickpair: two|one: R reps, X ms a rep, ...       c = two - one (case 219)
  [terrain] ... FPS: n FrameTime: X ms  (and [refract])     glmark2-ms.parse
  glmark2-counters: [terrain] ...: frames N ... kicks K     kicks a frame = K / N
  glmark2 pictures against column am62p: 33 scenes, match M, mismatch K ...
                                                            the picture split

c is taken from arm A's case 219 (arm E's when A has none), or --c.

Prints (a) the arm-7 table of docs/notes/pvr-mip-three-levels.md ("The board
A/B") with A and E measured, (b) one ok/miss line per confirmation criterion
(E's case 216 at 21.00 kicks and PASS; case 216 ms fall >= 19c - 2; terrain
ms fall >= 23c - 3; the same picture split), (c) a kick-bound line (am62p-
pvr-mipmap-kicks.md: the kicks fell and the milliseconds did not fall in
proportion: the frame is not bound by the kick count), (d) the last line
"mip3-ab: confirmed" or "mip3-ab: missed (<criteria>)".

Exit 0 confirmed, 1 missed, 2 unreadable (a record or a line missing). Stdlib
only.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

KICKS_PREDICTED = 21.00
CASE216_FALL_KICKS = 19  # 40 -> 21
TERRAIN_FALL_KICKS = 23  # 51 -> 28
REFRACT_FALL_KICKS = 6


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), os.path.join(HERE, name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


MS_RE = re.compile(r"gl_test: mipchains: \d+ frames, (\d+(?:\.\d+)?) ms a frame")
KICKS_RE = re.compile(r"gl_test:\s+per frame: (\d+(?:\.\d+)?) kicks \(")
PASS_RE = re.compile(r"gl_test: case 216 \S+: (PASS|FAIL|TIMEOUT|SKIP)\b")
PAIR_RE = re.compile(r"gl_test: kickpair: (two|one): \d+ reps, (\d+(?:\.\d+)?) ms a rep")
CTR_RE = re.compile(r"glmark2-counters: (\[(\w+)\] .*?): frames (\d+)\b(.*)$")
KICK_WORD_RE = re.compile(r"(?:^|\s)kicks (\d+)(?:\s|$)")
PIC_RE = re.compile(r"glmark2 pictures against column (\S+): (\d+) scenes, match (\d+)((?:, [a-z-]+ \d+)*)")


class Unreadable(Exception):
    pass


def read_record(path):
    if os.path.isdir(path):
        names = sorted(n for n in os.listdir(path) if n.endswith((".txt", ".log")))
        if not names:
            raise Unreadable("%s: no console.txt or *.log in the record" % path)
        parts = []
        for n in names:
            with open(os.path.join(path, n), encoding="utf-8", errors="replace") as f:
                parts.append(f.read())
        return "\n".join(parts)
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError as e:
        raise Unreadable("%s: %s" % (path, e.strerror))


def last(rx, text):
    found = rx.findall(text)
    return found[-1] if found else None


def parse_arm(label, path, glmark2_ms):
    text = read_record(path)
    miss = []
    d = {"label": label}
    v = last(MS_RE, text)
    d["ms216"] = float(v) if v else None
    if v is None:
        miss.append("case 216 'mipchains: 20 frames, X ms a frame'")
    v = last(KICKS_RE, text)
    d["kicks216"] = float(v) if v else None
    if v is None:
        miss.append("case 216 'per frame: N kicks'")
    v = last(PASS_RE, text)
    d["pass216"] = v
    if v is None:
        miss.append("'case 216 mipchains: PASS'")
    pair = {}
    for k, ms in PAIR_RE.findall(text):
        pair[k] = float(ms)
    d["c"] = pair["two"] - pair["one"] if "two" in pair and "one" in pair else None
    rows = glmark2_ms.parse(text)[0]
    for scene in ("terrain", "refract"):
        r = next((r for r in rows if r[0].startswith("[%s] " % scene) and r[1] is not None), None)
        d[scene + "_ms"] = r[1] if r else None
        if r is None:
            miss.append("[%s] FrameTime" % scene)
        d[scene + "_kicks"] = None
        for line in text.splitlines():
            m = CTR_RE.search(line)
            if m and m.group(2) == scene:
                k = KICK_WORD_RE.search(m.group(4))
                frames = int(m.group(3))
                if k and frames:
                    d[scene + "_kicks"] = int(k.group(1)) / frames
        if d[scene + "_kicks"] is None:
            miss.append("glmark2-counters [%s] kicks" % scene)
    pic = last(PIC_RE, text)
    if pic:
        split = {"match": int(pic[2])}
        for w in re.findall(r"([a-z-]+) (\d+)", pic[3]):
            split[w[0]] = int(w[1])
        d["pic"] = split
        d["pic_scenes"] = int(pic[1])
    else:
        d["pic"] = None
        miss.append("'glmark2 pictures against column ...' (the picture verdict)")
    d["missing"] = miss
    return d


def fmt(v, spec="%.2f"):
    return "-" if v is None else spec % v


def split_text(p):
    return "-" if p is None else ", ".join("%s %d" % (k, p[k]) for k in sorted(p, key=lambda k: (k != "match", k)))


def main(argv):
    c_arg = None
    args = list(argv)
    if len(args) >= 2 and args[0] == "--c":
        try:
            c_arg = float(args[1])
        except ValueError:
            print("mip3-ab: --c wants a number of ms", file=sys.stderr)
            return 2
        args = args[2:]
    if len(args) != 2:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    glmark2_ms = load("glmark2-ms")
    try:
        a = parse_arm("A", args[0], glmark2_ms)
        e = parse_arm("E", args[1], glmark2_ms)
    except Unreadable as ex:
        print("mip3-ab: unreadable: %s" % ex, file=sys.stderr)
        return 2
    c = c_arg if c_arg is not None else (a["c"] if a["c"] is not None else e["c"])
    problems = ["arm %s: no %s" % (x["label"], m) for x in (a, e) for m in x["missing"]]
    if c is None:
        problems.append("no case 219 'kickpair: two' and 'one' lines (c) in either arm, and no --c")
    if problems:
        for p in problems:
            print("mip3-ab: unreadable: %s" % p, file=sys.stderr)
        return 2

    print("| record line | A (off) | E (on), measured |")
    print("|---|---|---|")
    print("| gl_test 216 `per frame: N kicks` | %s | %s |" % (fmt(a["kicks216"]), fmt(e["kicks216"])))
    print("| gl_test 216 `mipchains: 20 frames, X ms a frame` | %s | %s |" % (fmt(a["ms216"]), fmt(e["ms216"])))
    print("| gl_test 216 `case 216 mipchains: PASS` | %s | %s |" % (a["pass216"], e["pass216"]))
    print("| `glmark2-counters: ... [terrain] ... kicks K` a frame | %s | %s |" % (fmt(a["terrain_kicks"]), fmt(e["terrain_kicks"])))
    print("| terrain `FrameTime` ms | %s | %s |" % (fmt(a["terrain_ms"]), fmt(e["terrain_ms"])))
    print("| refract `FrameTime` ms | %s | %s |" % (fmt(a["refract_ms"]), fmt(e["refract_ms"])))
    print("| `glmark2-counters: ... [refract] ... kicks K` a frame | %s | %s |" % (fmt(a["refract_kicks"]), fmt(e["refract_kicks"])))
    print("| pictures (column am62p) | %s | %s |" % (split_text(a["pic"]), split_text(e["pic"])))
    print()
    print("c = %.2f ms (%s)" % (c, "--c" if c_arg is not None else "case 219 two - one"))
    print()

    miss = []

    def crit(name, ok, text):
        print("%s: %s: %s" % (name, "ok" if ok else "miss", text))
        if not ok:
            miss.append(name)

    kick_ok = abs(e["kicks216"] - KICKS_PREDICTED) < 0.005
    crit("case216-kicks-pass", kick_ok and e["pass216"] == "PASS",
         "E %.2f kicks (want %.2f, off by %+.2f), E case 216 %s" % (
             e["kicks216"], KICKS_PREDICTED, e["kicks216"] - KICKS_PREDICTED, e["pass216"]))
    fall216 = a["ms216"] - e["ms216"]
    need216 = CASE216_FALL_KICKS * c - 2
    crit("case216-ms", fall216 >= need216,
         "fell %.2f ms, want >= 19c - 2 = %.2f ms (%s by %.2f ms)" % (
             fall216, need216, "met" if fall216 >= need216 else "missed", abs(fall216 - need216)))
    fallt = a["terrain_ms"] - e["terrain_ms"]
    needt = TERRAIN_FALL_KICKS * c - 3
    crit("terrain-ms", fallt >= needt,
         "terrain fell %.2f ms (E %.2f), want >= 23c - 3 = %.2f ms (%s by %.2f ms)" % (
             fallt, e["terrain_ms"], needt, "met" if fallt >= needt else "missed", abs(fallt - needt)))
    if a["pic"] == e["pic"]:
        crit("pictures", True, "same split: %s" % split_text(e["pic"]))
    else:
        keys = sorted(set(a["pic"]) | set(e["pic"]))
        diff = ", ".join("%s %d -> %d" % (k, a["pic"].get(k, 0), e["pic"].get(k, 0))
                         for k in keys if a["pic"].get(k, 0) != e["pic"].get(k, 0))
        crit("pictures", False, "split moved: A %s, E %s (%s)" % (split_text(a["pic"]), split_text(e["pic"]), diff))
    rf = a["refract_ms"] - e["refract_ms"]
    print("info: refract fell %.2f ms (about 6c = %.2f ms predicted); terrain kicks %.2f -> %.2f a frame (28-29 predicted)" % (
        rf, REFRACT_FALL_KICKS * c, a["terrain_kicks"], e["terrain_kicks"]))

    # (c) kick-bound: the kicks fell, did the ms fall in proportion?
    parts = []
    notbound = False
    for name, kd, fall, need in (("case 216", a["kicks216"] - e["kicks216"], fall216, need216),
                                 ("terrain", a["terrain_kicks"] - e["terrain_kicks"], fallt, needt)):
        if kd <= 0:
            parts.append("%s: kicks did not fall" % name)
            continue
        per = fall / kd
        if fall < need:
            notbound = True
        parts.append("%s: kicks -%.2f, ms -%.2f = %.2f ms a kick (c %.2f)" % (name, kd, fall, per, c))
    print("kick-bound: %s: %s" % (
        "no, the kicks fell and the ms did not fall in proportion" if notbound else "yes, the ms fell with the kicks",
        "; ".join(parts)))

    if miss:
        print("mip3-ab: missed (%s)" % ", ".join(miss))
        return 1
    print("mip3-ab: confirmed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
