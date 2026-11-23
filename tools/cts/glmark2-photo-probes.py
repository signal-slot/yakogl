#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Probe colours of a glmark2 picture from a camera screenshot of the i.MX8MP panel.

    tools/cts/glmark2-photo-probes.py <screenshot.png> [--rotation 0|90|180|270]
        [--window x,y,w,h] [--tolerance N]
        [--compare <tools/cts/glmark2-pictures.txt> <column> <scene info string>]

The screenshot is the tailvision camera's take_screenshot: a perspective-corrected
PNG of the whole panel at some output size (any size, e.g. 1280x720 or 800x500; the
aspect need not be the panel's) and rotation. --rotation is the rotation_degrees that
was passed to take_screenshot: the image is the panel turned that many degrees
clockwise, so the script turns it back (0 = the panel upright). The image is then
scaled to panel coordinates (1280x800) and sampled there.

Where the window is (the sources of the numbers):
  - The panel is 1280x800: driver/samples/zephyr/common/verdin_imx8mp_display.dtsi,
    lcdif1 hactive/vactive (lines 157-158; display0 again at 220-221).
  - The 800x600 glmark2 window is a view into a panel-pitch buffer whose first row
    and column are the panel's: driver/zephyr/lib/board/imx8mp/board_imx8mp.c,
    board_window() (lines 310-355: stride pw*4, the window at the buffer's start)
    and board_show() (lines 403-410: scan = win->phys). So the window is the
    panel's top-left 800x600, at (0, 0), and the rest of the panel is "black, not
    old memory" (board_imx8mp.c:346, memset 0): the right 480 columns and the
    bottom 200 rows are black. --window overrides the default.

The probes are the sample's picture code (driver/samples/zephyr/glmark2/src/main.cpp
lines 386-388): px = {w/2, w/4, 3w/4, w/4, 3w/4}, py = {h/2, h/4, h/4, 3h/4, 3h/4}
(integer division) for c, tl, tr, bl, br, in window pixels, row 0 at the top
(driver/docs/glmark2.md line 176: c (400,300), tl (200,150), tr (600,150),
bl (200,450), br (600,450)). Each probe is the per-channel median of a 7x7 patch of
panel pixels around it, so one noisy or moire pixel does not decide it.

Output, one line in the notation of glmark2-pictures.txt, marked approximate:

    photo: 800x600; probes c RRGGBB00, tl ..., br ...; mean R G B; black no  [approximate ...]

Alpha is not seen by a camera and prints 00, as the i.MX8MP column has it; there is no
crc16 (a photograph has no exact pixels). mean is the mean over the sample's 40x30
grid of cell centres (main.cpp:400), each the median of a 3x3 patch. black is `yes`
when the mean and every probe have all channels <= BLACK_LEVEL (16): a camera sees a
black panel as dark grey, not as 0.

--compare <file> <column> <scene>: finds `<column> | <scene> | <text>` in the
pictures table and prints per probe `near`, `far` or `black where the reference is
not`. A probe is near when every channel is within the tolerance of the reference's.
A reference counts as not black when its largest channel is above the tolerance; a
photographed probe is black when all its channels are <= BLACK_LEVEL (a dark
reference and a black photo are near). Exit 0: every probe near; 1: some probe far
or black where the reference is not; 2: bad input (file, column, scene, image,
option).

The tolerance default (TOLERANCE = 48 levels per channel, about 19%) is a judgement,
not a measurement. The table's board-to-board tolerance is 8, but a camera adds
exposure, white-balance and gamma shifts, backlight bleed and vignetting. 48 passes a
cast of a few tens of levels and still fails a wrong hue or a bright region where a
dark one belongs. Calibrate it on a photograph of a scene with known probes. This is
CONTENT-ONLY evidence: the right kind of picture is there (bright where it should be,
dark where it should be, the hue roughly right). It is not a value, does not replace
the UART probes, and never goes into the table as a probe value. Stdlib only.
"""
import re
import struct
import sys
import zlib

PANEL = (1280, 800)  # verdin_imx8mp_display.dtsi:157-158
WINDOW = (0, 0, 800, 600)  # board_imx8mp.c:310-355, 403-410: the panel's top-left
NAMES = ("c", "tl", "tr", "bl", "br")
BLACK_LEVEL = 16
TOLERANCE = 48
HALF = 3  # probe patch half-width, panel pixels
GRID = (40, 30)  # main.cpp Picture::GRID_X, GRID_Y


class Bad(Exception):
    pass


def decode_png(data):
    """-> (w, h, rows of RGB bytes). 8-bit, not interlaced; gray, RGB, palette, +alpha."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise Bad("not a PNG")
    pos, idat, plte, hdr = 8, [], None, None
    while pos + 8 <= len(data):
        n, tag = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if tag == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif tag == b"PLTE":
            plte = body
        elif tag == b"IDAT":
            idat.append(body)
        elif tag == b"IEND":
            break
    if hdr is None or not idat:
        raise Bad("PNG without IHDR or IDAT")
    w, h, depth, ctype, _, _, interlace = hdr
    if depth != 8 or interlace != 0 or ctype not in (0, 2, 3, 4, 6):
        raise Bad("PNG must be 8-bit and not interlaced (colour type %d, depth %d)" % (ctype, depth))
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    stride = w * ch
    try:
        raw = zlib.decompress(b"".join(idat))
    except zlib.error as e:
        raise Bad("PNG data damaged: %s" % e)
    if len(raw) < h * (stride + 1):
        raise Bad("PNG data too short")
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        cur = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if f > 4:
            raise Bad("PNG filter %d" % f)
        if f:
            for i in range(stride):
                a = cur[i - ch] if i >= ch else 0
                b = prev[i]
                c = prev[i - ch] if i >= ch else 0
                if f == 1:
                    p = a
                elif f == 2:
                    p = b
                elif f == 3:
                    p = (a + b) >> 1
                else:
                    t = a + b - c
                    pa, pb, pc = abs(t - a), abs(t - b), abs(t - c)
                    p = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[i] = (cur[i] + p) & 255
        prev = cur
        if ctype == 2:
            rows.append(bytes(cur))
        elif ctype == 6:
            rows.append(bytes(v for i in range(w) for v in cur[4 * i:4 * i + 3]))
        elif ctype == 0:
            rows.append(bytes(v for g in cur for v in (g, g, g)))
        elif ctype == 4:
            rows.append(bytes(v for i in range(w) for v in (cur[2 * i],) * 3))
        else:
            if plte is None:
                raise Bad("palette PNG without PLTE")
            rows.append(bytes(v for i in cur for v in plte[3 * i:3 * i + 3]))
    return w, h, rows


class Photo:
    """The screenshot seen in panel coordinates."""

    def __init__(self, w, h, rows, rotation):
        self.w, self.h, self.rows, self.rot = w, h, rows, rotation
        # the upright panel's size in image pixels (before the rotation)
        self.uw, self.uh = (h, w) if rotation in (90, 270) else (w, h)

    def pixel(self, px, py):
        px = min(max(px, 0), PANEL[0] - 1)
        py = min(max(py, 0), PANEL[1] - 1)
        ux = (px + 0.5) * self.uw / PANEL[0]
        uy = (py + 0.5) * self.uh / PANEL[1]
        if self.rot == 0:
            x, y = ux, uy
        elif self.rot == 90:  # turned clockwise: upright (ux, uy) is at (uh - uy, ux)
            x, y = self.uh - uy, ux
        elif self.rot == 180:
            x, y = self.uw - ux, self.uh - uy
        else:  # 270: upright (ux, uy) is at (uy, uw - ux)
            x, y = uy, self.uw - ux
        r = self.rows[min(max(int(y), 0), self.h - 1)]
        x = min(max(int(x), 0), self.w - 1)
        return r[3 * x], r[3 * x + 1], r[3 * x + 2]

    def median(self, px, py, half):
        pts = [self.pixel(px + dx, py + dy) for dy in range(-half, half + 1)
               for dx in range(-half, half + 1)]
        return tuple(sorted(p[c] for p in pts)[len(pts) // 2] for c in range(3))


def probe_points(x0, y0, w, h):
    """The sample's probes (main.cpp:386-388) as panel pixels, in NAMES order."""
    px = (w // 2, w // 4, w * 3 // 4, w // 4, w * 3 // 4)
    py = (h // 2, h // 4, h // 4, h * 3 // 4, h * 3 // 4)
    return [(x0 + a, y0 + b) for a, b in zip(px, py)]


def measure(photo, win):
    x0, y0, w, h = win
    probes = [photo.median(x, y, HALF) for x, y in probe_points(x0, y0, w, h)]
    gx, gy = GRID
    sums, n = [0, 0, 0], 0
    for j in range(gy):
        for i in range(gx):
            p = photo.median(x0 + (2 * i + 1) * w // (2 * gx), y0 + (2 * j + 1) * h // (2 * gy), 1)
            for c in range(3):
                sums[c] += p[c]
            n += 1
    mean = [s // n for s in sums]
    black = max(mean) <= BLACK_LEVEL and all(max(p) <= BLACK_LEVEL for p in probes)
    return probes, mean, black


def fmt(win, probes, mean, black):
    pr = ", ".join("%s %02x%02x%02x00" % ((n,) + p) for n, p in zip(NAMES, probes))
    return ("photo: %dx%d; probes %s; mean %d %d %d; black %s  "
            "[approximate: camera photograph, content-only]" % (
                win[2], win[3], pr, mean[0], mean[1], mean[2], "yes" if black else "no"))


def reference(path, column, scene):
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        raise Bad("cannot read %s: %s" % (path, e))
    val = None
    for ln in text.splitlines():
        if ln.startswith("#"):
            continue
        parts = [s.strip() for s in ln.split(" | ", 2)]
        if len(parts) == 3 and parts[0] == column and parts[1] == scene:
            val = parts[2]
            break
    if val is None:
        raise Bad("no line for column %r, scene %r in %s" % (column, scene, path))
    m = re.search(r"probes c ([0-9a-f]{6,8}), tl ([0-9a-f]{6,8}), tr ([0-9a-f]{6,8}), "
                  r"bl ([0-9a-f]{6,8}), br ([0-9a-f]{6,8})", val)
    if not m:
        raise Bad("the %s line has no probes: %s" % (column, val))
    return [tuple(int(h[i:i + 2], 16) for i in (0, 2, 4)) for h in m.groups()]


def compare(probes, ref, tol):
    out, worst = [], 0
    for n, p, r in zip(NAMES, probes, ref):
        p_black = max(p) <= BLACK_LEVEL
        r_black = max(r) <= tol
        if p_black and not r_black:
            v, worst = "black where the reference is not", 1
        elif (p_black and r_black) or max(abs(a - b) for a, b in zip(p, r)) <= tol:
            v = "near"
        else:
            v, worst = "far", 1
        out.append("%s: photo %02x%02x%02x, reference %02x%02x%02x: %s" % ((n,) + p + r + (v,)))
    return out, worst


def parse(argv):
    a = {"rotation": 0, "window": WINDOW, "tol": TOLERANCE, "compare": None}
    pos, i = [], 0
    try:
        while i < len(argv):
            o = argv[i]
            if o == "--rotation":
                a["rotation"] = int(argv[i + 1])
                if a["rotation"] not in (0, 90, 180, 270):
                    raise Bad("--rotation is 0, 90, 180 or 270")
                i += 2
            elif o == "--window":
                v = tuple(int(s) for s in argv[i + 1].split(","))
                if (len(v) != 4 or v[2] < GRID[0] or v[3] < GRID[1] or v[0] < 0 or v[1] < 0
                        or v[0] + v[2] > PANEL[0] or v[1] + v[3] > PANEL[1]):
                    raise Bad("--window x,y,w,h must lie in the %dx%d panel" % PANEL)
                a["window"] = v
                i += 2
            elif o == "--tolerance":
                a["tol"] = int(argv[i + 1])
                if not 0 <= a["tol"] <= 255:
                    raise Bad("--tolerance is 0..255")
                i += 2
            elif o == "--compare":
                a["compare"] = argv[i + 1:i + 4]
                if len(a["compare"]) != 3:
                    raise Bad("--compare needs <file> <column> <scene>")
                i += 4
            elif o.startswith("--"):
                raise Bad("unknown option %s" % o)
            else:
                pos.append(o)
                i += 1
    except (IndexError, ValueError):
        raise Bad("bad or missing value for an option")
    if len(pos) != 1:
        raise Bad("exactly one screenshot expected")
    a["png"] = pos[0]
    return a


def main(argv):
    try:
        a = parse(argv)
        try:
            with open(a["png"], "rb") as f:
                data = f.read()
        except OSError as e:
            raise Bad("cannot read the screenshot: %s" % e)
        w, h, rows = decode_png(data)
        probes, mean, black = measure(Photo(w, h, rows, a["rotation"]), a["window"])
        print(fmt(a["window"], probes, mean, black))
        if not a["compare"]:
            return 0
        res, worst = compare(probes, reference(*a["compare"]), a["tol"])
        for r in res:
            print(r)
        print("compare: %s (tolerance %d; camera evidence, content only)" % (
            "all near" if worst == 0 else "differs", a["tol"]))
        return worst
    except Bad as e:
        print("glmark2-photo-probes: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
