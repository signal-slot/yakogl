#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""dump2png: scanout readback bytes to PNG (standard library only).

Run it as `python3 -I dump2png.py ...`.

    dump2png.py DUMP -o out.png --width W --height H --format FMT
                [--pitch BYTES] [--input auto|bin|hex]
                [--crop x,y,w,h] [--stats] [--grid COLSxROWS]

DUMP is a file, or '-' for standard input.

Input forms (--input, default auto):

  bin   the raw bytes of the framebuffer, as in memory.
  hex   text. Per line, leading tokens that end in ':' are dropped (a 'tag:'
        prefix, an address column such as '0x1000:' or '00001000:'; any
        number of them). The rest of the line is hex tokens separated by
        whitespace or commas, each with an optional 0x prefix. A '#' starts a
        comment; lines with no hex tokens (headings, blank lines) are skipped.
        A token's length gives its meaning:
          2 digits   one byte
          4 digits   a 16-bit value, stored little-endian (as a halfword read)
          8 digits   a 32-bit value, stored little-endian (as a word read)
          other even lengths (a row as one run of digits): bytes in file
                     order, first digit pair first
        The bytes of all lines are joined in order. Line ends do not cut
        rows; rows are cut by --pitch.
  auto  hex when the input is all printable ASCII/whitespace and is not
        exactly the raw size of the picture, else bin.

Formats (DRM names, little-endian words; bytes in memory in brackets):

  XRGB8888 [B G R X]   ARGB8888 [B G R A]   XBGR8888 [R G B X]
  RGB565   (16-bit LE, red in the top bits)
  BGR565   (16-bit LE, blue in the top bits)

--pitch is the row stride in bytes (default width * bytes per pixel). Padding
bytes at the end of a row are ignored. Fewer bytes than
pitch*(height-1) + width*bpp is an error (exit 2); extra bytes are ignored
with a warning on stderr.

--crop x,y,w,h cuts the picture before output and before --stats.
--stats prints, for the whole picture and for each cell of a --grid
(default 1x2: top half, bottom half), the mean colour, the number of distinct
colours and the share of the commonest colour, plus a verdict word per cell:
  flat     one colour
  text     few colours (<= 8) and the commonest one covers >= 50 percent
  pattern  anything else (many colours or a gradient)
So a split-screen check reads "top: text, bottom: pattern".
The PNG is RGB, or RGBA for ARGB8888; the alpha byte of XRGB8888/XBGR8888 is
dropped. 565 components expand by bit replication.
Exit: 0 ok, 2 bad input or arguments.
"""
import argparse
import re
import struct
import sys
import zlib

FORMATS = {
    # name: (bytes per pixel, has alpha)
    "XRGB8888": (4, False),
    "ARGB8888": (4, True),
    "XBGR8888": (4, False),
    "RGB565": (2, False),
    "BGR565": (2, False),
}

HEXTOK = re.compile(r"(0[xX])?[0-9a-fA-F]+")


class Bad(Exception):
    pass


def parse_hex(text):
    out = bytearray()
    for ln, line in enumerate(text.splitlines(), 1):
        line = line.split("#", 1)[0]
        toks = [t for t in re.split(r"[\s,]+", line) if t]
        while toks and toks[0].endswith(":"):
            toks.pop(0)
        if not any(HEXTOK.fullmatch(t) for t in toks):
            continue  # blank or a heading
        for t in toks:
            if not HEXTOK.fullmatch(t):
                raise Bad("line %d: not a hex token: %r" % (ln, t))
            h = t[2:] if t[:2] in ("0x", "0X") else t
            if len(h) % 2:
                raise Bad("line %d: odd digit count in %r" % (ln, t))
            if len(h) in (4, 8):
                out += int(h, 16).to_bytes(len(h) // 2, "little")
            else:
                out += bytes.fromhex(h)
    return bytes(out)


def decode(data, fmt, w, h, pitch):
    bpp, alpha = FORMATS[fmt]
    need = pitch * (h - 1) + w * bpp
    if len(data) < need:
        raise Bad("need %d bytes for %dx%d %s pitch %d, got %d"
                  % (need, w, h, fmt, pitch, len(data)))
    if len(data) > pitch * h:
        sys.stderr.write("dump2png: warning: %d extra bytes ignored\n"
                         % (len(data) - pitch * h))
    rows = []
    for y in range(h):
        r = data[y * pitch:y * pitch + w * bpp]
        px = []
        for x in range(w):
            if bpp == 4:
                b0, b1, b2, b3 = r[4 * x:4 * x + 4]
                if fmt == "XBGR8888":
                    px.append((b0, b1, b2))
                elif alpha:
                    px.append((b2, b1, b0, b3))
                else:
                    px.append((b2, b1, b0))
            else:
                v = r[2 * x] | (r[2 * x + 1] << 8)
                hi, mid, lo = (v >> 11) & 31, (v >> 5) & 63, v & 31
                hi = (hi << 3) | (hi >> 2)
                mid = (mid << 2) | (mid >> 4)
                lo = (lo << 3) | (lo >> 2)
                px.append((hi, mid, lo) if fmt == "RGB565" else (lo, mid, hi))
        rows.append(px)
    return rows, alpha


def write_png(path, rows, alpha):
    h, w = len(rows), len(rows[0])
    raw = bytearray()
    for r in rows:
        raw.append(0)
        for p in r:
            raw += bytes(p)

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6 if alpha else 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def region_stats(rows, x0, y0, w, h):
    cnt = {}
    sr = sg = sb = 0
    for y in range(y0, y0 + h):
        for p in rows[y][x0:x0 + w]:
            c = p[:3]
            cnt[c] = cnt.get(c, 0) + 1
            sr += c[0]
            sg += c[1]
            sb += c[2]
    n = w * h
    share = max(cnt.values()) / n
    if len(cnt) == 1:
        verdict = "flat"
    elif len(cnt) <= 8 and share >= 0.5:
        verdict = "text"
    else:
        verdict = "pattern"
    return ("mean=(%.1f,%.1f,%.1f) colours=%d top=%.1f%% %s"
            % (sr / n, sg / n, sb / n, len(cnt), share * 100, verdict))


def main(argv):
    ap = argparse.ArgumentParser(prog="dump2png", description="scanout dump to PNG")
    ap.add_argument("dump", help="dump file or '-'")
    ap.add_argument("-o", "--output", help="PNG to write")
    ap.add_argument("--width", type=int, required=True)
    ap.add_argument("--height", type=int, required=True)
    ap.add_argument("--pitch", type=int, help="row stride in bytes")
    ap.add_argument("--format", required=True, choices=sorted(FORMATS))
    ap.add_argument("--input", choices=("auto", "bin", "hex"), default="auto")
    ap.add_argument("--crop", help="x,y,w,h")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--grid", default="1x2", help="COLSxROWS for --stats")
    a = ap.parse_args(argv)
    try:
        if a.width < 1 or a.height < 1:
            raise Bad("width and height must be positive")
        if not a.output and not a.stats:
            raise Bad("nothing to do: give -o and/or --stats")
        bpp = FORMATS[a.format][0]
        pitch = a.pitch if a.pitch is not None else a.width * bpp
        if pitch < a.width * bpp:
            raise Bad("pitch %d is less than width*bpp %d" % (pitch, a.width * bpp))
        if a.dump == "-":
            data = sys.stdin.buffer.read()
        else:
            with open(a.dump, "rb") as f:
                data = f.read()
        mode = a.input
        if mode == "auto":
            printable = all(c in b"\t\n\r" or 32 <= c < 127 for c in data)
            mode = "hex" if printable and len(data) != pitch * a.height else "bin"
        if mode == "hex":
            data = parse_hex(data.decode("ascii", "replace"))
        rows, alpha = decode(data, a.format, a.width, a.height, pitch)
        if a.crop:
            try:
                cx, cy, cw, ch = [int(v) for v in a.crop.split(",")]
            except ValueError:
                raise Bad("--crop wants x,y,w,h")
            if cx < 0 or cy < 0 or cw < 1 or ch < 1 or cx + cw > a.width or cy + ch > a.height:
                raise Bad("--crop %s outside the %dx%d picture" % (a.crop, a.width, a.height))
            rows = [r[cx:cx + cw] for r in rows[cy:cy + ch]]
        h, w = len(rows), len(rows[0])
        if a.stats:
            m = re.fullmatch(r"(\d+)x(\d+)", a.grid)
            if not m or int(m[1]) < 1 or int(m[2]) < 1:
                raise Bad("--grid wants COLSxROWS")
            gc, gr = int(m[1]), int(m[2])
            if gc > w or gr > h:
                raise Bad("--grid %s is finer than the %dx%d picture" % (a.grid, w, h))
            print("all: %s" % region_stats(rows, 0, 0, w, h))
            for j in range(gr):
                for i in range(gc):
                    x0, x1 = i * w // gc, (i + 1) * w // gc
                    y0, y1 = j * h // gr, (j + 1) * h // gr
                    print("cell %d,%d (x=%d y=%d w=%d h=%d): %s"
                          % (i, j, x0, y0, x1 - x0, y1 - y0,
                             region_stats(rows, x0, y0, x1 - x0, y1 - y0)))
        if a.output:
            write_png(a.output, rows, alpha)
    except (Bad, OSError) as e:
        sys.stderr.write("dump2png: %s\n" % e)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
