#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""colorbars.py - judge a KVM screenshot of the DK display probe.

  colorbars.py SHOT.png|SHOT.ppm
  colorbars.py --triangle --apex 0.5,0.15 --apex 0.2,0.8 --apex 0.8,0.8 \
               [--color R,G,B] [--background R,G,B] SHOT.png

Bars mode: the mp2_display probe fills 8 equal vertical bars, left to right
white, yellow, cyan, green, magenta, red, blue, black.  Each pixel of the
middle rows is classified by the nearest of those eight colours (within
--tol), the runs are measured and one line is printed:

  colorbars: PASS: 8 bars white yellow cyan green magenta red blue black
  colorbars: FAIL: <reason>

Triangle mode: every --apex X,Y[:R,G,B] (fractions of the width/height) must
show a non-background colour, the given one when --color or ":R,G,B" is set,
and the triangle's centroid must be non-background too.

Exit 0 on PASS, 1 on FAIL, 2 on an unreadable input.  Standard library only;
input is a PNG (8-bit RGB/RGBA/gray, non-interlaced) or a binary PPM (P6).
"""
import argparse
import struct
import sys
import zlib

BARS = [
    ("white", (255, 255, 255)),
    ("yellow", (255, 255, 0)),
    ("cyan", (0, 255, 255)),
    ("green", (0, 255, 0)),
    ("magenta", (255, 0, 255)),
    ("red", (255, 0, 0)),
    ("blue", (0, 0, 255)),
    ("black", (0, 0, 0)),
]
NAMES = [n for n, _ in BARS]
# The same bars as the display shows them with red and blue exchanged.
SWAP = {"yellow": "cyan", "cyan": "yellow", "red": "blue", "blue": "red"}
SWAPPED = [SWAP.get(n, n) for n in NAMES]
PNG_SIG = b"\x89PNG\r\n\x1a\n"


class ImageError(Exception):
    pass


class Image:
    """width x height, rows of 3 bytes per pixel (RGB)."""

    def __init__(self, width, height, rows):
        self.width, self.height, self.rows = width, height, rows

    def pixel(self, x, y):
        o = 3 * x
        r = self.rows[y]
        return (r[o], r[o + 1], r[o + 2])


# ---------------------------------------------------------------- PNG

def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def _unfilter(raw, height, stride, bpp):
    rows, prev = [], bytearray(stride)
    pos = 0
    for _ in range(height):
        if pos + 1 + stride > len(raw):
            raise ImageError("PNG: truncated image data")
        ft = raw[pos]
        cur = bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        if ft == 1:
            for i in range(bpp, stride):
                cur[i] = (cur[i] + cur[i - bpp]) & 255
        elif ft == 2:
            for i in range(stride):
                cur[i] = (cur[i] + prev[i]) & 255
        elif ft == 3:
            for i in range(stride):
                left = cur[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + ((left + prev[i]) >> 1)) & 255
        elif ft == 4:
            for i in range(stride):
                left = cur[i - bpp] if i >= bpp else 0
                ul = prev[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + _paeth(left, prev[i], ul)) & 255
        elif ft != 0:
            raise ImageError("PNG: bad filter type %d" % ft)
        rows.append(cur)
        prev = cur
    return rows


def decode_png(data):
    if data[:8] != PNG_SIG:
        raise ImageError("not a PNG")
    pos, idat, hdr = 8, [], None
    while pos + 8 <= len(data):
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if typ == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"IDAT":
            idat.append(body)
        elif typ == b"IEND":
            break
    if hdr is None or not idat:
        raise ImageError("PNG: no IHDR/IDAT")
    w, h, depth, ctype, _, _, interlace = hdr
    if depth != 8 or interlace != 0 or ctype not in (0, 2, 6):
        raise ImageError("PNG: only 8-bit gray/RGB/RGBA non-interlaced "
                         "(depth %d, colour type %d, interlace %d)"
                         % (depth, ctype, interlace))
    bpp = {0: 1, 2: 3, 6: 4}[ctype]
    try:
        raw = zlib.decompress(b"".join(idat))
    except zlib.error as e:
        raise ImageError("PNG: %s" % e)
    rows = _unfilter(raw, h, w * bpp, bpp)
    if bpp == 3:
        return Image(w, h, rows)
    out = []
    for r in rows:
        o = bytearray(3 * w)
        if bpp == 4:
            o[0::3], o[1::3], o[2::3] = r[0::4], r[1::4], r[2::4]
        else:
            o[0::3] = o[1::3] = o[2::3] = r
        out.append(o)
    return Image(w, h, out)


def encode_png(width, height, rows, alpha=False):
    """rows: bytes-like, 3 (4 with alpha) bytes per pixel."""
    def chunk(t, b):
        c = struct.pack(">I", len(b)) + t + b
        return c + struct.pack(">I", zlib.crc32(t + b) & 0xffffffff)
    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    return (PNG_SIG
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8,
                                          6 if alpha else 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


# ---------------------------------------------------------------- PPM

def decode_ppm(data):
    pos, fields = 0, []
    while len(fields) < 4:
        while pos < len(data) and data[pos:pos + 1].isspace():
            pos += 1
        if data[pos:pos + 1] == b"#":
            while pos < len(data) and data[pos:pos + 1] != b"\n":
                pos += 1
            continue
        s = pos
        while pos < len(data) and not data[pos:pos + 1].isspace():
            pos += 1
        if s == pos:
            raise ImageError("PPM: truncated header")
        fields.append(data[s:pos])
    pos += 1
    if fields[0] != b"P6":
        raise ImageError("not a binary PPM (P6)")
    try:
        w, h, mx = (int(f) for f in fields[1:])
    except ValueError:
        raise ImageError("PPM: bad header")
    if mx != 255:
        raise ImageError("PPM: only maxval 255")
    if len(data) - pos < 3 * w * h:
        raise ImageError("PPM: truncated pixel data")
    return Image(w, h, [data[pos + 3 * w * y:pos + 3 * w * (y + 1)]
                        for y in range(h)])


def load(path):
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError as e:
        raise ImageError(str(e))
    if data[:8] == PNG_SIG:
        return decode_png(data)
    if data[:2] == b"P6":
        return decode_ppm(data)
    raise ImageError("%s: neither PNG nor PPM (a JPEG screenshot must be "
                     "converted first)" % path)


# ---------------------------------------------------------------- bars

def dist(a, b):
    return sum((x - y) ** 2 for x, y in zip(a, b)) ** 0.5


def nearest(px, tol):
    best = min(BARS, key=lambda nc: dist(px, nc[1]))
    return best[0] if dist(px, best[1]) <= tol else None


def column_classes(img, tol):
    """Per column: the colour name most of the middle rows agree on."""
    y0, y1 = img.height // 4, max(img.height // 4 + 1, 3 * img.height // 4)
    ys = list(range(y0, y1, max(1, (y1 - y0) // 16)))
    out = []
    for x in range(img.width):
        votes = {}
        for y in ys:
            c = nearest(img.pixel(x, y), tol)
            votes[c] = votes.get(c, 0) + 1
        c, n = max(votes.items(), key=lambda kv: kv[1])
        out.append(c if n * 2 > len(ys) else None)
    return out


def runs_of(cols, minlen):
    """[(name, width)] of runs at least minlen wide; unclassified or shorter
    stretches (the edges between bars) are dropped, equal neighbours merged."""
    out, i = [], 0
    while i < len(cols):
        j = i
        while j < len(cols) and cols[j] == cols[i]:
            j += 1
        if cols[i] is not None and j - i >= minlen:
            if out and out[-1][0] == cols[i]:
                out[-1][1] += j - i
            else:
                out.append([cols[i], j - i])
        i = j
    return [(n, w) for n, w in out]


def _subseq(a, b):
    it = iter(b)
    return all(x in it for x in a)


def judge_bars(img, tol=100, uneven=0.25):
    cols = column_classes(img, tol)
    known = sum(1 for c in cols if c is not None)
    lit = sum(1 for c in cols if c not in (None, "black"))
    if lit < img.width * 0.02:
        if known < img.width * 0.5:
            return False, ("no signal: neither black nor colour bars "
                           "(the KVM's no-signal screen?)")
        return False, "no signal: the picture is black"
    runs = runs_of(cols, max(2, img.width // 200))
    # A black border left of the picture is not a bar.
    if len(runs) > 1 and runs[0][0] == "black" and runs[1][0] == "white":
        runs = runs[1:]
    names = [n for n, _ in runs]
    shown = " ".join(names)
    if names != NAMES:
        if names == SWAPPED:
            return False, "red and blue swapped (%s): a pixel-format mistake" % shown
        if len(names) < 8 and _subseq(names, SWAPPED) and \
                not _subseq(names, NAMES):
            return False, "red and blue swapped and partial (%s)" % shown
        if len(names) < 8 and _subseq(names, NAMES):
            return False, "partial: %d of 8 bars (%s)" % (len(names), shown)
        return False, "wrong order: %s" % (shown or "no bars")
    # The black bar may be merged with a border: judge the seven others.
    ws = sorted(w for _, w in runs[:-1])
    med = ws[len(ws) // 2]
    bad = [n for n, w in runs[:-1] if abs(w - med) > uneven * med]
    if bad:
        return False, ("scaled: uneven bar widths (%s off the %d px median)"
                       % (", ".join(bad), med))
    return True, "8 bars " + shown


# ------------------------------------------------------------ triangle

def parse_rgb(s):
    try:
        v = tuple(int(p) for p in s.split(","))
    except ValueError:
        v = ()
    if len(v) != 3 or any(not 0 <= c <= 255 for c in v):
        raise argparse.ArgumentTypeError("RGB as R,G,B (0-255): " + s)
    return v


def parse_apex(s):
    pos, _, col = s.partition(":")
    try:
        x, y = (float(p) for p in pos.split(","))
    except ValueError:
        raise argparse.ArgumentTypeError("apex as X,Y[:R,G,B]: " + s)
    if not (0 <= x <= 1 and 0 <= y <= 1):
        raise argparse.ArgumentTypeError("apex fractions in 0..1: " + s)
    return (x, y, parse_rgb(col) if col else None)


def window_mean(img, fx, fy, half):
    cx = min(img.width - 1, int(fx * img.width))
    cy = min(img.height - 1, int(fy * img.height))
    px = [img.pixel(x, y)
          for y in range(max(0, cy - half), min(img.height, cy + half + 1))
          for x in range(max(0, cx - half), min(img.width, cx + half + 1))]
    return tuple(sum(p[i] for p in px) // len(px) for i in range(3))


def judge_triangle(img, apexes, color=None, background=(0, 0, 0), tol=100,
                   inset=0.2):
    """Samples sit `inset` of the way from each apex to the centroid: the
    exact apex pixel is anti-aliased or clipped."""
    if len(apexes) != 3:
        return False, "the triangle needs exactly three --apex"
    cx = sum(a[0] for a in apexes) / 3
    cy = sum(a[1] for a in apexes) / 3
    half = max(1, min(img.width, img.height) // 200)
    bad, seen = [], []
    for i, (x, y, c) in enumerate(apexes):
        px = window_mean(img, x + (cx - x) * inset, y + (cy - y) * inset, half)
        want = c or color
        seen.append(px)
        if dist(px, background) <= tol / 2:
            bad.append("apex %d (%.2f,%.2f) is background %s" % (i, x, y, px))
        elif want and dist(px, want) > tol:
            bad.append("apex %d (%.2f,%.2f) is %s, want %s"
                       % (i, x, y, px, want))
    mid = window_mean(img, cx, cy, half)
    if dist(mid, background) <= tol / 2:
        bad.append("centroid is background %s" % (mid,))
    if bad:
        return False, "; ".join(bad)
    return True, "triangle at 3 apexes " + " ".join(
        "#%02x%02x%02x" % p for p in seen)


# ---------------------------------------------------------------- main

def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("image")
    ap.add_argument("--triangle", action="store_true",
                    help="check the GPU triangle instead of the colour bars")
    ap.add_argument("--apex", action="append", type=parse_apex, default=[],
                    metavar="X,Y[:R,G,B]")
    ap.add_argument("--color", type=parse_rgb, metavar="R,G,B")
    ap.add_argument("--background", type=parse_rgb, default=(0, 0, 0),
                    metavar="R,G,B")
    ap.add_argument("--tol", type=float, default=100,
                    help="colour distance accepted (default 100 of 441)")
    a = ap.parse_args(argv)
    tag = "triangle" if a.triangle else "colorbars"
    try:
        img = load(a.image)
    except ImageError as e:
        print("%s: FAIL: unreadable input: %s" % (tag, e))
        return 2
    if a.triangle:
        ok, msg = judge_triangle(img, a.apex, a.color, a.background, a.tol)
    else:
        ok, msg = judge_bars(img, a.tol)
    print("%s: %s: %s" % (tag, "PASS" if ok else "FAIL", msg))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
