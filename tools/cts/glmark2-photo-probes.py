#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Probe colours of a glmark2 picture from a camera screenshot of the i.MX8MP panel.

    tools/cts/glmark2-photo-probes.py <screenshot.png> [--rotation 0|90|180|270]
        [--window x,y,w,h] [--tolerance N] [--frame panel|x2-kvm]
        [--kvm [--rect x,y,w,h]]
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

--frame x2-kvm: the screenshot is a whole-frame capture of the Armadillo-X2's
display through the NanoKVM (1920x1080, any size of that aspect, rotation 0),
not the Verdin panel. The X2 scans the 800x600 glmark2 window at (0,0) of a
1920x1080 frame and the rest is black (copy path, SCANOUT_VIEW=n;
docs/notes/x2-lcdif-1080.md, board_imx8mp.c scanout code), so the window is
cropped at (0,0,800,600) of the 1920x1080 frame. Because a crop of the window
alone reads plausible probes from a wrong framing, the area OUTSIDE the window
is checked too: it must be black (every sampled pixel <= BLACK_LEVEL, at most
OUTSIDE_BAD_FRACTION of the samples allowed above it for JPEG noise, a margin of
OUTSIDE_MARGIN frame pixels around the window left out for edge blur). If it is
not (a shifted window, a wrong frame, a desktop behind), no probes are printed
and the exit is 2 with the share of non-black samples and the first place. The
default frame is `panel` (the Verdin, above), where there is no such check.

--kvm: the screenshot is an upright, straight (no perspective) HDMI capture, of any
size, with the 800x600 scene (4:3) somewhere inside it, possibly scaled, the rest
black (Armadillo-X2 through the NanoKVM). The scene rectangle is found in capture
pixels as the bounding box of the non-black content (a row or column counts when
at least KVM_MIN_HITS sampled pixels of it exceed BLACK_LEVEL, so JPEG noise does
not widen it), then it must be 4:3 within KVM_ASPECT_TOL and at least 40x30; else
exit 2 and --rect x,y,w,h (capture pixels) is the manual override. The five
probes and the mean are taken at the scene's fractions of that rectangle
(1/2, 1/4, 3/4 as above), so a scaled scene needs no scale option. A scene whose
border rows or columns are black itself shrinks the box: the aspect check then
refuses, use --rect. --kvm excludes --frame, --window and --rotation. The output
line is the same, marked "approximate: KVM capture", with the rectangle shown in
a trailing "[kvm rect x,y,w,h]".

JPEG input (the NanoKVM capture is one; recognised by its FF D8 FF header, not the
file name): there is no JPEG decoder here. The file is converted to an 8-bit PNG
first, by Pillow if it imports, else by ffmpeg, else by ImageMagick (magick, then
convert) found on PATH, into a directory under $TMPDIR (removed afterwards). With
none of them the exit is 2 and the message says so. A JPEG's probes differ from the
PNG's by compression noise (a few levels), far inside the tolerance.

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
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

PANEL = (1280, 800)  # verdin_imx8mp_display.dtsi:157-158
FRAMES = {"panel": PANEL, "x2-kvm": (1920, 1080)}  # x2-kvm: NanoKVM whole frame
OUTSIDE_STEP = 8  # x2-kvm outside check: sample every 8th frame pixel
OUTSIDE_MARGIN = 6  # frame pixels around the window not checked (edge blur)
OUTSIDE_BAD_FRACTION = 0.005  # of the samples may be above BLACK_LEVEL (JPEG noise)
WINDOW = (0, 0, 800, 600)  # board_imx8mp.c:310-355, 403-410: the panel's top-left
NAMES = ("c", "tl", "tr", "bl", "br")
BLACK_LEVEL = 16
TOLERANCE = 48
HALF = 3  # probe patch half-width, panel pixels
KVM_STEP = 2  # --kvm detection: sample every 2nd capture pixel
KVM_MIN_HITS = 3  # sampled pixels above BLACK_LEVEL for a row/column to count
KVM_ASPECT_TOL = 0.04  # the found rectangle's w/h may differ from 4:3 by this fraction
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


def jpeg_to_png(path, tmpdir):
    """Convert a JPEG to an 8-bit RGB PNG in tmpdir with Pillow or an external tool."""
    out = os.path.join(tmpdir, "capture.png")
    try:
        from PIL import Image
        with Image.open(path) as im:
            im.convert("RGB").save(out, "PNG")
        return out
    except ImportError:
        pass
    except Exception as e:  # Pillow present but the file is damaged
        raise Bad("JPEG cannot be decoded by Pillow: %s" % e)
    cmds = []
    if shutil.which("ffmpeg"):
        cmds.append(["ffmpeg", "-v", "error", "-y", "-i", path, "-pix_fmt", "rgb24", out])
    for tool in ("magick", "convert"):
        if shutil.which(tool):
            cmds.append([tool, path, "-depth", "8", "-interlace", "none", "PNG24:" + out])
    if not cmds:
        raise Bad("the capture is a JPEG and this tool has no JPEG decoder: install Pillow "
                  "(python3 -m pip install pillow), ffmpeg or ImageMagick (convert/magick), "
                  "or give a PNG")
    errs = []
    for cmd in cmds:
        try:
            r = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, timeout=120)
        except (OSError, subprocess.TimeoutExpired) as e:
            errs.append("%s: %s" % (cmd[0], e))
            continue
        if r.returncode == 0 and os.path.exists(out) and os.path.getsize(out) > 0:
            return out
        errs.append("%s exit %d: %s" % (cmd[0], r.returncode, r.stderr.decode("utf-8", "replace").strip()[:200]))
    raise Bad("JPEG conversion failed (%s)" % "; ".join(errs))


class Photo:
    """The screenshot seen in panel coordinates."""

    def __init__(self, w, h, rows, rotation, panel=PANEL):
        self.w, self.h, self.rows, self.rot = w, h, rows, rotation
        self.panel = panel
        # the upright panel's size in image pixels (before the rotation)
        self.uw, self.uh = (h, w) if rotation in (90, 270) else (w, h)

    def pixel(self, px, py):
        px = min(max(px, 0), self.panel[0] - 1)
        py = min(max(py, 0), self.panel[1] - 1)
        ux = (px + 0.5) * self.uw / self.panel[0]
        uy = (py + 0.5) * self.uh / self.panel[1]
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


def outside_check(photo, win):
    """x2-kvm: -> (bad samples, samples, first bad frame pixel or None) outside the window."""
    x0, y0, w, h = win
    fw, fh = photo.panel
    bad, n, first = 0, 0, None
    for y in range(OUTSIDE_STEP // 2, fh, OUTSIDE_STEP):
        for x in range(OUTSIDE_STEP // 2, fw, OUTSIDE_STEP):
            if (x0 - OUTSIDE_MARGIN <= x < x0 + w + OUTSIDE_MARGIN
                    and y0 - OUTSIDE_MARGIN <= y < y0 + h + OUTSIDE_MARGIN):
                continue
            n += 1
            if max(photo.median(x, y, 1)) > BLACK_LEVEL:
                bad += 1
                if first is None:
                    first = (x, y)
    return bad, n, first


def find_scene_rect(w, h, rows):
    """--kvm: bounding box (x, y, w, h) of the non-black content of an upright capture."""
    cols, rws = [0] * w, [0] * h
    for y in range(0, h, KVM_STEP):
        r = rows[y]
        for x in range(0, w, KVM_STEP):
            if max(r[3 * x], r[3 * x + 1], r[3 * x + 2]) > BLACK_LEVEL:
                cols[x] += 1
                rws[y] += 1
    xs = [x for x in range(w) if cols[x] >= KVM_MIN_HITS]
    ys = [y for y in range(h) if rws[y] >= KVM_MIN_HITS]
    if not xs or not ys:
        raise Bad("--kvm: no non-black content found in the capture (black screen?); "
                  "give --rect x,y,w,h")
    x0, y0 = xs[0], ys[0]
    rw, rh = xs[-1] - x0 + KVM_STEP, ys[-1] - y0 + KVM_STEP
    rw, rh = min(rw, w - x0), min(rh, h - y0)
    if rw < GRID[0] or rh < GRID[1] or abs(rw / rh - 4 / 3) > KVM_ASPECT_TOL * 4 / 3:
        raise Bad("--kvm: the non-black area %d,%d,%d,%d is not a 4:3 scene (a desktop or "
                  "OSD in the frame, or a scene with black borders); give --rect x,y,w,h"
                  % (x0, y0, rw, rh))
    return x0, y0, rw, rh


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


def fmt(win, probes, mean, black, note="camera photograph"):
    pr = ", ".join("%s %02x%02x%02x00" % ((n,) + p) for n, p in zip(NAMES, probes))
    return ("photo: %dx%d; probes %s; mean %d %d %d; black %s  "
            "[approximate: %s, content-only]" % (
                win[2], win[3], pr, mean[0], mean[1], mean[2], "yes" if black else "no", note))


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
    a = {"rotation": 0, "window": WINDOW, "tol": TOLERANCE, "compare": None, "frame": "panel",
         "kvm": False, "rect": None}
    win = None
    given = set()
    pos, i = [], 0
    try:
        while i < len(argv):
            o = argv[i]
            if o == "--rotation":
                given.add(o)
                a["rotation"] = int(argv[i + 1])
                if a["rotation"] not in (0, 90, 180, 270):
                    raise Bad("--rotation is 0, 90, 180 or 270")
                i += 2
            elif o == "--window":
                given.add(o)
                v = tuple(int(s) for s in argv[i + 1].split(","))
                if len(v) != 4:
                    raise Bad("--window is x,y,w,h")
                win = v
                i += 2
            elif o == "--kvm":
                a["kvm"] = True
                i += 1
            elif o == "--rect":
                v = tuple(int(s) for s in argv[i + 1].split(","))
                if len(v) != 4 or min(v) < 0 or v[2] < GRID[0] or v[3] < GRID[1]:
                    raise Bad("--rect is x,y,w,h in capture pixels, at least 40x30")
                a["rect"] = v
                i += 2
            elif o == "--frame":
                given.add(o)
                a["frame"] = argv[i + 1]
                if a["frame"] not in FRAMES:
                    raise Bad("--frame is %s" % " or ".join(FRAMES))
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
    if a["rect"] is not None and not a["kvm"]:
        raise Bad("--rect needs --kvm")
    if a["kvm"] and given:
        raise Bad("--kvm excludes %s" % ", ".join(sorted(given)))
    fw, fh = FRAMES[a["frame"]]
    if win is not None:
        if (win[2] < GRID[0] or win[3] < GRID[1] or win[0] < 0 or win[1] < 0
                or win[0] + win[2] > fw or win[1] + win[3] > fh):
            raise Bad("--window x,y,w,h must lie in the %dx%d %s" % (fw, fh, a["frame"]))
        a["window"] = win
    return a


def main(argv):
    try:
        a = parse(argv)
        try:
            with open(a["png"], "rb") as f:
                data = f.read()
        except OSError as e:
            raise Bad("cannot read the screenshot: %s" % e)
        if data[:3] == b"\xff\xd8\xff":
            with tempfile.TemporaryDirectory(prefix="photo-probes-", dir=os.environ.get("TMPDIR") or None) as td:
                with open(jpeg_to_png(a["png"], td), "rb") as f:
                    data = f.read()
        w, h, rows = decode_png(data)
        if a["kvm"]:
            rect = a["rect"] or find_scene_rect(w, h, rows)
            if rect[0] + rect[2] > w or rect[1] + rect[3] > h:
                raise Bad("--rect lies outside the %dx%d capture" % (w, h))
            photo = Photo(w, h, rows, 0, (w, h))
            probes, mean, black = measure(photo, rect)
            print(fmt(rect, probes, mean, black, "KVM capture") +
                  "  [kvm rect %s]" % ",".join(map(str, rect)))
            if not a["compare"]:
                return 0
            res, worst = compare(probes, reference(*a["compare"]), a["tol"])
            for r in res:
                print(r)
            print("compare: %s (tolerance %d; KVM capture, content only)" % (
                "all near" if worst == 0 else "differs", a["tol"]))
            return worst
        photo = Photo(w, h, rows, a["rotation"], FRAMES[a["frame"]])
        if a["frame"] == "x2-kvm":
            bad, n, first = outside_check(photo, a["window"])
            if bad > n * OUTSIDE_BAD_FRACTION:
                raise Bad("x2-kvm framing wrong: %d of %d samples outside the window %s are "
                          "not black (first at frame pixel %s); no probes read" % (
                              bad, n, ",".join(map(str, a["window"])), first))
        probes, mean, black = measure(photo, a["window"])
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
