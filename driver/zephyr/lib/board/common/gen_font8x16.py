#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate gc_font8x16.h, the panel console's bitmap font.

Source: the X11 "misc-fixed" 8x13 font, which every distribution ships as
/usr/share/fonts/misc/8x13.pcf.gz (package font-misc-misc). Its PCF COPYRIGHT
property reads "Public domain font.  Share and enjoy.", so the glyph bitmaps
can be pasted into an Apache-2.0 source file without a second licence.

The glyph box is 8x13. Each glyph is placed in an 8x16 cell with one blank row
above and two below, which is the cell height the console lays out with.

Needs Pillow (PIL.PcfFontFile). Run it only when the font has to change:

    python3 gen_font8x16.py /usr/share/fonts/misc/8x13.pcf.gz > gc_font8x16.h
"""
import gzip
import io
import sys

from PIL import PcfFontFile

FIRST, LAST = 0x20, 0x7E
CELL_H = 16
TOP_PAD = 1  # blank rows above the 13-row glyph box; 13 + 1 + 2 == 16


def load(path):
    raw = gzip.open(path, "rb").read() if path.endswith(".gz") else open(path, "rb").read()
    return PcfFontFile.PcfFontFile(io.BytesIO(raw))


def rows(glyph):
    """The 13 rows of one glyph, most significant bit = leftmost pixel."""
    img = glyph[3].convert("1")
    w, h = img.size
    px = img.load()
    out = []
    for y in range(h):
        bits = 0
        for x in range(min(w, 8)):
            if px[x, y]:
                bits |= 0x80 >> x
        out.append(bits)
    return out


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/fonts/misc/8x13.pcf.gz"
    font = load(path)

    w = sys.stdout.write
    w("/* SPDX-License-Identifier: Apache-2.0 */\n")
    w("/*\n")
    w(" * gc_font8x16.h -- the panel console's font. GENERATED, do not edit:\n")
    w(" * driver/zephyr/lib/board/common/gen_font8x16.py regenerates it.\n")
    w(" *\n")
    w(" * Glyphs: the X11 \"misc-fixed\" 8x13 font (/usr/share/fonts/misc/8x13.pcf.gz,\n")
    w(" * package font-misc-misc). Its PCF COPYRIGHT property is \"Public domain font.\n")
    w(" * Share and enjoy.\", so the bitmaps carry no licence of their own.\n")
    w(" *\n")
    w(" * Layout: printable ASCII 0x20..0x7e, one 8x16 cell per character, one byte\n")
    w(" * per row, top row first, bit 7 = leftmost pixel. The 13-row glyph box sits\n")
    w(" * one row down, leaving two blank rows below it as line spacing.\n")
    w(" */\n")
    w("#ifndef GC_FONT8X16_H\n#define GC_FONT8X16_H\n\n#include <stdint.h>\n\n")
    w("#define GC_FONT_W     8\n")
    w("#define GC_FONT_H     %d\n" % CELL_H)
    w("#define GC_FONT_FIRST 0x%02x\n" % FIRST)
    w("#define GC_FONT_LAST  0x%02x\n\n" % LAST)
    w("static const uint8_t gc_font8x16[GC_FONT_LAST - GC_FONT_FIRST + 1][GC_FONT_H] = {\n")
    for code in range(FIRST, LAST + 1):
        cell = [0] * CELL_H
        for i, bits in enumerate(rows(font.glyph[code])):
            if TOP_PAD + i < CELL_H:
                cell[TOP_PAD + i] = bits
        name = chr(code) if code != 0x5C else "\\\\"
        w("\t/* 0x%02x '%s' */ { " % (code, name))
        w(", ".join("0x%02x" % b for b in cell))
        w(" },\n")
    w("};\n\n#endif /* GC_FONT8X16_H */\n")


if __name__ == "__main__":
    main()
