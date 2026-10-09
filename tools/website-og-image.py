#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write website/og-image.png, the 1200x630 picture link previews show.

Standard library only, deterministic: the same bytes on every run. The
picture is the favicon's triangle on a nacre gradient; it carries no text,
so it never goes stale. The SPDX lines are PNG tEXt chunks.

    python3 -I tools/website-og-image.py
"""
import math
import os
import struct
import zlib

W, H = 1200, 630
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "website", "og-image.png")


def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def inside(px, py, tri):
    (x1, y1), (x2, y2), (x3, y3) = tri
    d1 = (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2)
    d2 = (px - x3) * (y2 - y3) - (x2 - x3) * (py - y3)
    d3 = (px - x1) * (y3 - y1) - (x3 - x1) * (py - y1)
    neg = d1 < 0 or d2 < 0 or d3 < 0
    pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (neg and pos)


def main():
    bg0, bg1 = (18, 20, 24), (29, 26, 46)
    teal, violet, amber = (63, 184, 168), (143, 127, 224), (227, 169, 92)
    cx, cy, s = W / 2, H / 2 + 20, 230
    outer = ((cx, cy - s), (cx + s * 1.13, cy + s * 0.87), (cx - s * 1.13, cy + s * 0.87))
    k = 0.78
    inner = tuple((cx + (x - cx) * k, cy + 18 + (y - cy) * k) for x, y in outer)
    rows = bytearray()
    for y in range(H):
        rows.append(0)
        for x in range(W):
            t = (x / W + y / H) / 2
            c = mix(bg0, bg1, t)
            wave = 0.5 + 0.5 * math.sin(x * 0.006 + y * 0.004)
            c = mix(c, mix(teal, violet, wave), 0.10)
            if inside(x, y, outer) and not inside(x, y, inner):
                g = (x - (cx - s)) / (2 * s)
                c = mix(teal, violet, g * 2) if g < 0.5 else mix(violet, amber, (g - 0.5) * 2)
            elif inside(x, y, inner):
                c = mix(c, violet, 0.12)
            rows += bytes(max(0, min(255, int(round(v)))) for v in c)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
    png += chunk(b"tEXt", b"SPDX-License-Identifier\x00Apache-2.0")
    png += chunk(b"tEXt", b"SPDX-FileCopyrightText\x002026 Signal Slot, Inc.")
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(OUT, "wb") as f:
        f.write(png)
    print("wrote", os.path.normpath(OUT), len(png), "bytes")


if __name__ == "__main__":
    main()
