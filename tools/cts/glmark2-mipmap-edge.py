#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Measure the left edge of glmark2's [texture] texture-filter=mipmap cube and
test the timing explanation of the probe `tl` (issue #17).

    tools/cts/glmark2-mipmap-edge.py <800x600 PNG | console log> [--scene SUBSTRING]

Input: a PNG written by tools/cts/glmark2-fb.py (8-bit RGB or RGBA, not
interlaced), or a console log with a frame dump, which is decoded with
glmark2-fb.py's decode() (imported from this directory; damaged rows that this
script needs make the result inconclusive).

Where the numbers come from. The coefficients are from the host analysis of
plan #397 (item mipmap-cause): the cube rotates, and the frame-time offset
d (ms) moves its left edge to

    row 150:  x = 200.000 + 0.0353 * d
    row 450:  x = 200.000 - 0.212  * d

Pixel x covers [x, x+1), its centre is x+0.5. The probes are the sample's
(driver/samples/zephyr/glmark2, driver/docs/glmark2.md "The picture value"):
tl = (200,150), bl = (200,450). At d = 0 the edge passes through tl's pixel
corner, so tl is covered; once the edge is past the pixel centre, 200.5, i.e.
d > 0.5/0.0353 = 14.2 ms, tl reads the background, 000000 on the board. So a tl
of 000000 is explained by timing if (a) both rows give the same d (within their edge
uncertainty: +-0.5 px for a hard edge, which is what the GPU draws), (b) tl's pixel is uncovered, (c) d > 14.2. Timing is ruled out if tl's
pixel is covered but black, or if the edge has a black rim (a run of 000000
pixels between a non-black background and the texture).

Edge: scan the row from x = 0; the background is pixel (0, row). The first
pixel that differs from it (any channel by more than THRESH) is the edge pixel;
its coverage fraction is its distance from the background relative to the
texture pixel 3 to its right (strongest channel); edge = x0 + 1 - fraction.
Caveats: with a 000000 background a black rim cannot be told from background,
and the sub-pixel estimate on row 150 (0.0353 px per ms) is good to about
+-3 ms only. tl nonblack, a missing edge or rows that disagree are
inconclusive.

Prints a neighbourhood table and one verdict line:

    mipmap-edge: d=<ms> (row150) d=<ms> (row450); tl covered|uncovered <rgb>;
      rim none|black; timing explains tl: yes|no|inconclusive

Exit 0: timing explains tl (yes); 1: it does not (no); 2: inconclusive or the
input cannot be read. Stdlib only.
"""
import importlib.util
import os
import struct
import sys
import zlib

W, H = 800, 600
TL = (200, 150)
ROWS = (150, 450)
K150, K450 = 0.0353, -0.212
THRESH = 2
D_NEEDED = 0.5 / K150  # 14.2 ms


def read_png(data):
    """-> (w, h, rows of w*3 RGB bytes)."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos, idat, ihdr = 8, b"", None
    while pos + 8 <= len(data):
        n, tag = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if tag == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif tag == b"IDAT":
            idat += body
        elif tag == b"IEND":
            break
    if ihdr is None:
        raise ValueError("no IHDR")
    w, h, depth, ctype, _, _, interlace = ihdr
    if depth != 8 or ctype not in (2, 6) or interlace:
        raise ValueError("only 8-bit RGB/RGBA, not interlaced")
    bpp = 3 if ctype == 2 else 4
    raw = zlib.decompress(idat)
    stride = w * bpp
    if len(raw) != h * (stride + 1):
        raise ValueError("PNG data has the wrong length")
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        cur = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = cur[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                cur[i] = (cur[i] + a) & 255
            elif f == 2:
                cur[i] = (cur[i] + b) & 255
            elif f == 3:
                cur[i] = (cur[i] + ((a + b) >> 1)) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[i] = (cur[i] + pr) & 255
            elif f != 0:
                raise ValueError("bad PNG filter %d" % f)
        prev = cur
        if bpp == 4:
            cur = bytearray(v for i, v in enumerate(cur) if i % 4 != 3)
        rows.append(bytes(cur))
    return w, h, rows


def load_log(path, scene):
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("glmark2_fb", os.path.join(here, "glmark2-fb.py"))
    fb = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fb)
    with open(path, errors="replace") as f:
        got = fb.decode(f.read(), scene)
    if got is None:
        raise ValueError("no frame dump in the log")
    name, w, h, rows, problems, _end, _pic = got
    if problems:
        print("mipmap-edge: dump of %s has problems: %s" % (name, "; ".join(problems[:3])),
              file=sys.stderr)
    return w, h, rows


def px(rows, x, y):
    r = rows[y]
    if r is None:
        raise ValueError("row %d is damaged or missing" % y)
    return tuple(r[x * 3:x * 3 + 3])


def differs(a, b):
    return max(abs(p - q) for p, q in zip(a, b)) > THRESH


def find_edge(rows, w, y):
    """-> dict(edge, x0, bg, first, rim), or None when there is no edge in 100..300."""
    bg = px(rows, 0, y)
    for x0 in range(1, min(w - 4, 300)):
        p = px(rows, x0, y)
        if not differs(p, bg):
            continue
        if x0 < 100:
            return None
        tex = px(rows, x0 + 3, y)
        ch = max(range(3), key=lambda i: abs(tex[i] - bg[i]))
        span = tex[ch] - bg[ch]
        frac = 1.0 if span == 0 else min(1.0, max(0.0, (p[ch] - bg[ch]) / span))
        rim = 0
        if bg != (0, 0, 0):
            while px(rows, x0 + rim, y) == (0, 0, 0):
                rim += 1
        # A hard edge (the GPU does not anti-alias: a pixel is drawn when its
        # centre is inside) is known to +-0.5 px only; a partial pixel to ~0.15.
        return dict(edge=x0 + 1 - frac, x0=x0, bg=bg, first=p, rim=rim,
                    unc=0.5 if frac >= 0.99 else 0.15)
    return None


def analyse(w, h, rows, out=sys.stdout):
    if (w, h) != (W, H):
        raise ValueError("the picture is %dx%d, not %dx%d" % (w, h, W, H))
    e150, e450 = (find_edge(rows, w, y) for y in ROWS)
    ds = []
    for e, k, y in ((e150, K150, ROWS[0]), (e450, K450, ROWS[1])):
        if e is None:
            print("row %d: no edge found" % y, file=out)
            ds.append(None)
        else:
            d = (e["edge"] - 200.0) / k
            e["u"] = e["unc"] / abs(k)
            ds.append(d)
            print("row %d: edge x=%.3f +-%.2f (first pixel %d %s, background %s, black run %d)"
                  " d=%.1f +-%.1f ms" % (y, e["edge"], e["unc"], e["x0"], bytes(e["first"]).hex(),
                                         bytes(e["bg"]).hex(), e["rim"], d, e["u"]), file=out)
    d150, d450 = ds
    # The two rows agree when their d intervals (+- the edge's pixel uncertainty) meet.
    lo = hi = None
    if e150 is not None and e450 is not None:
        lo = max(d150 - e150["u"], d450 - e450["u"])
        hi = min(d150 + e150["u"], d450 + e450["u"])
    bg = px(rows, 0, TL[1])
    print("tl neighbourhood x=198..202, y=148..152 (* = differs from the background %s):"
          % bytes(bg).hex(), file=out)
    for y in range(TL[1] - 2, TL[1] + 3):
        print("  y=%d  " % y + " ".join(
            ("*" if differs(px(rows, x, y), bg) else " ") + bytes(px(rows, x, y)).hex()
            for x in range(TL[0] - 2, TL[0] + 3)), file=out)
    tl = px(rows, *TL)
    covered = differs(tl, bg) or (e150 is not None and e150["edge"] <= TL[0] + 0.5)
    black_tl = tl == (0, 0, 0)
    rim = any(e is not None and e["rim"] > 0 for e in (e150, e450))

    if covered and black_tl:
        why = "no"
    elif rim:
        why = "no"
    elif lo is None or lo > hi or not black_tl:
        why = "inconclusive"
    elif not covered and hi > D_NEEDED:
        why = "yes"
    else:
        why = "inconclusive"

    def fmt(d):
        return "n/a" if d is None else "%.1f" % d

    print("mipmap-edge: d=%s (row150) d=%s (row450); tl %s %s; rim %s; timing explains tl: %s"
          % (fmt(d150), fmt(d450), "covered" if covered else "uncovered", bytes(tl).hex(),
             "black" if rim else "none", why), file=out)
    return {"yes": 0, "no": 1}.get(why, 2)


def main(argv):
    args = argv[1:]
    scene = None
    if "--scene" in args:
        i = args.index("--scene")
        scene = args[i + 1] if i + 1 < len(args) else None
        del args[i:i + 2]
        if scene is None:
            args = []
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        with open(args[0], "rb") as f:
            data = f.read()
        w, h, rows = read_png(data) if data[:4] == b"\x89PNG" else load_log(args[0], scene)
        return analyse(w, h, rows)
    except (OSError, ValueError, zlib.error) as e:
        print("mipmap-edge: %s: %s" % (args[0], e), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
