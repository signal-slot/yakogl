#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""imx8mp-gm-perf.py - glmark2 PERF page words to ms a frame and KiB of CPU passes.

  python3 -I tools/cts/imx8mp-gm-perf.py [--board x2|verdin] [--scene NAME] W0 W1 W2 [W3]
  python3 -I tools/cts/imx8mp-gm-perf.py < console.log

Page layout: driver/samples/zephyr/glmark2/src/gm_perf_page.h lines 8-12 (the
record: w0 = 0x594b0c00 | page, w1 = v1<<16 | v0, w2 = v3<<16 | v2,
w3 = v5<<16 | v4), 15-18 (page 0), 29-34 (page 4); docs/notes/
gc-slow-shadow-refract.md lines 224-233 (selectors 0x1000..0x1004, saturation
at 0xffff) and 469-472 (what a session records). Console line
(main.cpp:1075):  glmark2-perf: <scene>: page <n>: <w0> <w1> <w2> <w3>
On the Armadillo-X2 only w0..w2 are stored, so w3 may be left out.

X2 mode (--board x2, or a console line with only w0 w1 w2): decodes what w0..w2
hold (page 0 v0 and v1 so ms a frame, v2, v3; page 4 v0..v3 so KiB of CPU
passes) and prints the cells that need w3 (page 0 v4 and v5, and any v4/v5)
as "not in an X2 record"; each scene line says "measured on the Armadillo-X2".
--board verdin forces the full four-word reading.  Three positional words with
no --board keep the old reading (w3 = 0, v4/v5 left out without a note).

Page 0 (us per frame): v0 frames in the 10 s scene, v1 frame, v2 gl, v3 swap,
v4 show, v5 frame to frame.  ms a frame = 10000 / v0.  Page 4: v0 GPU mipmap
chains / 100 frames, v1 CPU mipmap fallbacks / 100, v2 CPU GPU-waits / 100,
v3 KiB a frame of CPU passes.  Every value saturates at 0xffff: v0 = 65535 is
"at least 65535 frames", so ms a frame is only a bound (<= 0.153 ms);
a us value of 65535 means 65.5 ms or more (a lower bound); page 4 with all of
v0..v3 = 0xffff means the image had no counters ("not available").

Exit: 0 decoded, 1 malformed input (message on stderr), 2 usage.
"""
import re
import sys

MAGIC = 0x594B0C00  # YK_RECORD_MAGIC | YK_RECORD_KIND_PERF << 8
SAT = 0xFFFF
LINE = re.compile(
    r"glmark2-perf:\s*(?P<scene>\S+):\s*page\s+(?P<page>\d+):\s*(?P<w>(?:[0-9a-fA-F]{8}\s*){3,4})\s*$")


class Bad(Exception):
    pass


def parse_words(toks):
    if len(toks) not in (3, 4):
        raise Bad("need w0 w1 w2 [w3] (3 or 4 hex words), got %d" % len(toks))
    ws = []
    for t in toks:
        t = t[2:] if t[:2].lower() == "0x" else t
        if not re.fullmatch(r"[0-9a-fA-F]{1,8}", t):
            raise Bad("not a 32-bit hex word: %r" % t)
        ws.append(int(t, 16))
    if len(ws) == 3:
        ws.append(0)
    return ws


def values(ws):
    w0, w1, w2, w3 = ws
    if w0 & ~0xFF != MAGIC:
        raise Bad("w0 %08x is not a PERF record (want 0x594b0c0p)" % w0)
    page = w0 & 0xFF
    if page > 4:
        raise Bad("page %d is not 0..4" % page)
    return page, [w1 & 0xFFFF, w1 >> 16, w2 & 0xFFFF, w2 >> 16, w3 & 0xFFFF, w3 >> 16]


def us(name, x):
    if x == SAT:
        return "%s us: >= 65535 saturated=yes (65.5 ms or more)" % name
    return "%s us: %d saturated=no" % (name, x)


def decode(ws, x2=False):
    page, v = values(ws)
    out = []
    if page == 0:
        if v[0] == SAT:
            out.append("ms a frame: <= %.3f (bound: page 0 v0 saturated at 65535)" % (10000 / SAT))
            out.append("frames: >= 65535 saturated=yes")
        elif v[0] == 0:
            raise Bad("page 0 v0 = 0 frames: no ms a frame")
        else:
            out.append("ms a frame: %.3f" % (10000 / v[0]))
            out.append("frames: %d saturated=no" % v[0])
        names = [("frame", 1), ("gl", 2), ("swap", 3)]
        if x2:  # w3 is not stored on the X2
            out += [us(n, v[i]) for n, i in names]
            out += ["%s us: not in an X2 record" % n for n in ("show", "frame-to-frame")]
        else:
            if ws[3] != 0:
                names += [("show", 4), ("frame-to-frame", 5)]
            out += [us(n, v[i]) for n, i in names]
    elif page == 4:
        if v[:4] == [SAT] * 4:
            out.append("KiB of CPU passes: not available (v0..v3 all 0xffff: no counters)")
        else:
            for n, i in (("GPU mipmap chains /100", 0), ("CPU mipmap fallbacks /100", 1),
                         ("CPU GPU-waits /100", 2)):
                s = v[i] == SAT
                out.append("%s: %s saturated=%s" % (n, ">= 65535" if s else v[i], "yes" if s else "no"))
            if v[3] == SAT:
                out.append("KiB of CPU passes: >= 65535 saturated=yes")
            else:
                out.append("KiB of CPU passes: %d saturated=no" % v[3])
    else:
        if x2:
            v = [str(x) for x in v[:4]] + ["not in an X2 record"] * 2
        out.append("v0..v5 = %s (not decoded)" % " ".join(map(str, v)))
    return page, out


def main(argv):
    scene, args = "?", argv[1:]
    if args[:1] in (["-h"], ["--help"]):
        sys.stdout.write(__doc__)
        return 0
    board = None
    if args[:1] == ["--board"]:
        if len(args) < 2 or args[1] not in ("x2", "verdin"):
            sys.stderr.write("usage: --board x2|verdin [--scene NAME] W0 W1 W2 [W3]\n")
            return 2
        board, args = args[1], args[2:]
    if args[:1] == ["--scene"]:
        if len(args) < 2:
            sys.stderr.write("usage: --scene NAME W0 W1 W2 [W3]\n")
            return 2
        scene, args = args[1], args[2:]
    recs = []
    try:
        if args:
            recs.append((scene, parse_words(args), None, board == "x2"))
        else:
            for n, line in enumerate(sys.stdin, 1):
                if "glmark2-perf:" not in line:
                    continue
                m = LINE.search(line)
                if not m:
                    raise Bad("line %d: malformed glmark2-perf line: %r" % (n, line.strip()))
                toks = m.group("w").split()
                x2 = board == "x2" or (board is None and len(toks) == 3)
                recs.append((m.group("scene"), parse_words(toks), int(m.group("page")), x2))
            if not recs:
                raise Bad("no glmark2-perf: line on stdin")
        res = []
        for sc, ws, claimed, x2 in recs:
            p, out = decode(ws, x2)
            if claimed is not None and claimed != p:
                raise Bad("scene %s: console says page %d, w0 says page %d" % (sc, claimed, p))
            res.append((sc, p, out, x2))
    except Bad as e:
        sys.stderr.write("imx8mp-gm-perf: %s\n" % e)
        return 1
    for sc, p, out, x2 in res:
        print("scene: %s page %d%s" % (sc, p, " (measured on the Armadillo-X2)" if x2 else ""))
        for o in out:
            print("  " + o)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
