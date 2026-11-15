#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One PNG with a board glmark2 picture next to its llvmpipe reference.

    tools/cts/glmark2-sidebyside.py <board picture> <reference picture> <out.png>
        [--label-board TEXT] [--label-ref TEXT]
    tools/cts/glmark2-sidebyside.py --self-test

The board picture is a decoded frame dump (tools/cts/glmark2-fb.py: 800x600
PNG) or a camera photograph of the panel (any size or aspect); the reference is
a picture of tools/cts/glmark2-host-ref.sh (PNG, or PPM P6). Inputs: PNG (8
bit, non-interlaced, grey, grey+alpha, RGB, RGBA or palette) or binary PPM.

Both pictures are scaled to the height of the taller one, aspect ratio kept,
and placed side by side with a thin grey separator and a label bar above each
(built-in 5x7 font, upper case ASCII). The output is for looking at: the
verdict stays by eye. Pixel values are NOT altered beyond scaling:
nearest-neighbour only (no filtering, no colour or brightness change); alpha is
dropped. A side that is already the common height is copied unscaled.
Stdlib only.
"""
import struct
import sys
import zlib

SEP = 4
SEP_COLOUR = (128, 128, 128)
BAR_COLOUR = (32, 32, 32)
TEXT_COLOUR = (255, 255, 255)
SCALE = 2  # label font scale
BAR_H = 7 * SCALE + 8

# 5x7 font, five columns, bit0 = top row; the characters labels need.
FONT = {
    ' ': [0, 0, 0, 0, 0], '-': [8, 8, 8, 8, 8], '.': [0, 96, 96, 0, 0],
    ':': [0, 54, 54, 0, 0], '/': [32, 16, 8, 4, 2], '_': [64, 64, 64, 64, 64],
    '=': [20, 20, 20, 20, 20], '(': [0, 28, 34, 65, 0], ')': [0, 65, 34, 28, 0],
    '0': [62, 81, 73, 69, 62], '1': [0, 66, 127, 64, 0], '2': [66, 97, 81, 73, 70],
    '3': [33, 65, 69, 75, 49], '4': [24, 20, 18, 127, 16], '5': [39, 69, 69, 69, 57],
    '6': [60, 74, 73, 73, 48], '7': [1, 113, 9, 5, 3], '8': [54, 73, 73, 73, 54],
    '9': [6, 73, 73, 41, 30],
    'A': [126, 17, 17, 17, 126], 'B': [127, 73, 73, 73, 54], 'C': [62, 65, 65, 65, 34],
    'D': [127, 65, 65, 34, 28], 'E': [127, 73, 73, 73, 65], 'F': [127, 9, 9, 9, 1],
    'G': [62, 65, 73, 73, 122], 'H': [127, 8, 8, 8, 127], 'I': [0, 65, 127, 65, 0],
    'J': [32, 64, 65, 63, 1], 'K': [127, 8, 20, 34, 65], 'L': [127, 64, 64, 64, 64],
    'M': [127, 2, 12, 2, 127], 'N': [127, 4, 8, 16, 127], 'O': [62, 65, 65, 65, 62],
    'P': [127, 9, 9, 9, 6], 'Q': [62, 65, 81, 33, 94], 'R': [127, 9, 25, 41, 70],
    'S': [70, 73, 73, 73, 49], 'T': [1, 1, 127, 1, 1], 'U': [63, 64, 64, 64, 63],
    'V': [31, 32, 64, 32, 31], 'W': [63, 64, 56, 64, 63], 'X': [99, 20, 8, 20, 99],
    'Y': [7, 8, 112, 8, 7], 'Z': [97, 81, 73, 69, 67],
}


def chunk(tag, data):
    c = tag + data
    return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c))


def write_png(path, w, h, rows):
    """rows: h bytes objects of w*3 RGB bytes."""
    raw = b''.join(b'\0' + r for r in rows)
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
                + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def read_ppm(data):
    pos = 2
    vals = []
    while len(vals) < 3:
        while data[pos:pos + 1].isspace():
            pos += 1
        if data[pos:pos + 1] == b'#':
            while data[pos:pos + 1] not in (b'\n', b''):
                pos += 1
            continue
        s = pos
        while data[pos:pos + 1].isdigit():
            pos += 1
        if s == pos:
            raise ValueError('bad PPM header')
        vals.append(int(data[s:pos]))
    w, h, mx = vals
    if mx != 255:
        raise ValueError('PPM maxval %d, only 255 is supported' % mx)
    pos += 1
    body = data[pos:pos + w * h * 3]
    if len(body) != w * h * 3:
        raise ValueError('PPM pixel data truncated')
    return w, h, [body[y * w * 3:(y + 1) * w * 3] for y in range(h)]


def read_png(data):
    pos = 8
    ihdr = None
    plte = b''
    idat = b''
    while pos + 8 <= len(data):
        n, tag = struct.unpack('>I4s', data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if tag == b'IHDR':
            ihdr = struct.unpack('>IIBBBBB', body)
        elif tag == b'PLTE':
            plte = body
        elif tag == b'IDAT':
            idat += body
        elif tag == b'IEND':
            break
    if ihdr is None:
        raise ValueError('PNG without IHDR')
    w, h, depth, ctype, _, _, interlace = ihdr
    if depth != 8 or interlace != 0 or ctype not in (0, 2, 3, 4, 6):
        raise ValueError('PNG depth %d colour type %d interlace %d not supported (8 bit, '
                         'non-interlaced only)' % (depth, ctype, interlace))
    bpp = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    stride = w * bpp
    raw = zlib.decompress(idat)
    if len(raw) < h * (stride + 1):
        raise ValueError('PNG pixel data truncated')
    prev = bytearray(stride)
    out = []
    for y in range(h):
        ft = raw[y * (stride + 1)]
        cur = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if ft == 1:
            for i in range(bpp, stride):
                cur[i] = (cur[i] + cur[i - bpp]) & 255
        elif ft == 2:
            for i in range(stride):
                cur[i] = (cur[i] + prev[i]) & 255
        elif ft == 3:
            for i in range(stride):
                a = cur[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + ((a + prev[i]) >> 1)) & 255
        elif ft == 4:
            for i in range(stride):
                a = cur[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[i] = (cur[i] + pr) & 255
        elif ft != 0:
            raise ValueError('PNG filter type %d' % ft)
        prev = cur
        r = bytearray(w * 3)
        if ctype == 2:
            r = cur
        elif ctype == 6:
            r[0::3], r[1::3], r[2::3] = cur[0::4], cur[1::4], cur[2::4]
        elif ctype in (0, 4):
            r[0::3] = r[1::3] = r[2::3] = cur[0::bpp]
        else:
            r = bytearray()
            for i in cur:
                r += plte[i * 3:i * 3 + 3]
        out.append(bytes(r))
    return w, h, out


def read_picture(path):
    with open(path, 'rb') as f:
        data = f.read()
    if data[:8] == b'\x89PNG\r\n\x1a\n':
        return read_png(data)
    if data[:2] == b'P6':
        return read_ppm(data)
    raise ValueError('%s: neither PNG nor binary PPM (P6)' % path)


def scale(pic, nh):
    """Nearest-neighbour to height nh, aspect kept (width rounded, at least 1)."""
    w, h, rows = pic
    if h == nh:
        return w, h, rows
    nw = max(1, (w * nh + h // 2) // h)
    xs = [min(w - 1, x * w // nw) for x in range(nw)]
    out = []
    for y in range(nh):
        src = rows[min(h - 1, y * h // nh)]
        r = bytearray(nw * 3)
        for x, sx in enumerate(xs):
            r[x * 3:x * 3 + 3] = src[sx * 3:sx * 3 + 3]
        out.append(bytes(r))
    return nw, nh, out


def label_bar(w, text):
    """BAR_H rows of width w with the text (upper-cased, clipped to w)."""
    rows = [bytearray(bytes(BAR_COLOUR) * w) for _ in range(BAR_H)]
    x0 = 6
    for ch in text.upper():
        glyph = FONT.get(ch, FONT['_'])
        if x0 + 6 * SCALE > w:
            break
        for cx, col in enumerate(glyph):
            for cy in range(7):
                if col >> cy & 1:
                    for dy in range(SCALE):
                        for dx in range(SCALE):
                            o = (x0 + cx * SCALE + dx) * 3
                            rows[4 + cy * SCALE + dy][o:o + 3] = bytes(TEXT_COLOUR)
        x0 += 6 * SCALE
    return [bytes(r) for r in rows]


def compose(board, ref, lab_b='', lab_r=''):
    nh = max(board[1], ref[1])
    bw, _, brows = scale(board, nh)
    rw, _, rrows = scale(ref, nh)
    sep = bytes(SEP_COLOUR) * SEP
    bar_sep = bytes(BAR_COLOUR) * SEP
    rows = [a + bar_sep + b for a, b in zip(label_bar(bw, lab_b), label_bar(rw, lab_r))]
    rows += [a + sep + b for a, b in zip(brows, rrows)]
    return bw + SEP + rw, len(rows), rows


def run(board, ref, out, lab_b, lab_r):
    w, h, rows = compose(read_picture(board), read_picture(ref), lab_b, lab_r)
    write_png(out, w, h, rows)
    print('glmark2-sidebyside: %s: %dx%d' % (out, w, h))


def self_test():
    import tempfile
    fails = []

    def check(name, ok):
        if not ok:
            fails.append(name)
        print('%s %s' % ('ok  ' if ok else 'FAIL', name))

    def px(rows, x, y):
        return tuple(rows[y][x * 3:x * 3 + 3])

    def load(path):
        with open(path, 'rb') as f:
            return read_png(f.read())

    with tempfile.TemporaryDirectory() as d:
        # board: 8x6 PPM (with a comment), left red, right blue, one odd pixel.
        b = [bytes([255, 0, 0] * 4 + [0, 0, 255] * 4) for _ in range(6)]
        b[2] = bytes([255, 0, 0] * 3 + [10, 20, 30] + [0, 0, 255] * 4)
        with open(d + '/b.ppm', 'wb') as f:
            f.write(b'P6\n# c\n8 6\n255\n' + b''.join(b))
        # reference: 4x3 RGB PNG, two green rows over one yellow row.
        write_png(d + '/r.png', 4, 3, [bytes([0, 255, 0] * 4)] * 2 + [bytes([255, 255, 0] * 4)])
        # photograph-like: 16x6 RGBA PNG.
        rgba = b''.join(b'\0' + bytes([1, 2, 3, 255] * 16) for _ in range(6))
        with open(d + '/p.png', 'wb') as f:
            f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 16, 6, 8, 6, 0, 0, 0))
                    + chunk(b'IDAT', zlib.compress(rgba)) + chunk(b'IEND', b''))

        run(d + '/b.ppm', d + '/r.png', d + '/o.png', 'board', 'ref')
        w, h, rows = load(d + '/o.png')
        check('size ppm+png', (w, h) == (8 + SEP + 8, BAR_H + 6))
        check('board pixels unscaled', px(rows, 0, BAR_H) == (255, 0, 0) and px(rows, 7, BAR_H) == (0, 0, 255))
        check('board odd pixel exact', px(rows, 3, BAR_H + 2) == (10, 20, 30))
        check('separator', px(rows, 8, BAR_H + 1) == SEP_COLOUR and px(rows, 11, BAR_H + 1) == SEP_COLOUR)
        check('ref nearest 4x3->8x6', px(rows, 12, BAR_H + 1) == (0, 255, 0)
              and px(rows, 19, BAR_H + 4) == (255, 255, 0))
        wide = (80, 6, [bytes(240)] * 6)
        w, h, rows = compose(wide, wide, 'board', 'ref')
        check('labels drawn', any(px(rows, x, y) == TEXT_COLOUR for y in range(BAR_H) for x in range(80))
              and any(px(rows, x, y) == TEXT_COLOUR for y in range(BAR_H) for x in range(84, 160)))

        run(d + '/p.png', d + '/r.png', d + '/o2.png', '', '')
        w, h, rows = load(d + '/o2.png')
        check('size photo+png', (w, h) == (16 + SEP + 8, BAR_H + 6))
        check('rgba alpha dropped', px(rows, 5, BAR_H + 3) == (1, 2, 3))

        # upscaling uses only source values.
        big = (80, 60, [bytes([7, 8, 9] * 80)] * 60)
        sm = (2, 2, [bytes([1, 1, 1, 2, 2, 2]), bytes([3, 3, 3, 4, 4, 4])])
        w, h, rows = compose(sm, big)
        check('upscale size', (w, h) == (60 + SEP + 80, BAR_H + 60))
        check('upscale values from source only',
              {px(rows, x, BAR_H + y) for x in range(60) for y in range(60)}
              == {(1, 1, 1), (2, 2, 2), (3, 3, 3), (4, 4, 4)})

        # PNG reader: Sub, Up, Average and Paeth rows round trip.
        src = [bytes((x * 37 + y * 11) & 255 for x in range(12)) for y in range(4)]
        raw = b''
        prev = bytes(12)
        for ft, cur in zip((1, 2, 3, 4), src):
            enc = bytearray()
            for i in range(12):
                a = cur[i - 3] if i >= 3 else 0
                up = prev[i]
                c = prev[i - 3] if i >= 3 else 0
                if ft == 1:
                    p = a
                elif ft == 2:
                    p = up
                elif ft == 3:
                    p = (a + up) >> 1
                else:
                    q = a + up - c
                    pa, pb, pc = abs(q - a), abs(q - up), abs(q - c)
                    p = a if pa <= pb and pa <= pc else (up if pb <= pc else c)
                enc.append((cur[i] - p) & 255)
            raw += bytes([ft]) + bytes(enc)
            prev = cur
        with open(d + '/f.png', 'wb') as f:
            f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 4, 4, 8, 2, 0, 0, 0))
                    + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))
        check('png filters', load(d + '/f.png')[2] == src)

        with open(d + '/bad.txt', 'wb') as f:
            f.write(b'nope')
        try:
            read_picture(d + '/bad.txt')
            check('bad input rejected', False)
        except ValueError:
            check('bad input rejected', True)
    print('glmark2-sidebyside self-test: %s' % ('FAILED: ' + ', '.join(fails) if fails else 'ok'))
    return 1 if fails else 0


def main(argv):
    if argv[1:] == ['--self-test']:
        return self_test()
    pos, labs, i = [], {'--label-board': '', '--label-ref': ''}, 1
    while i < len(argv):
        if argv[i] in labs and i + 1 < len(argv):
            labs[argv[i]] = argv[i + 1]
            i += 2
        else:
            pos.append(argv[i])
            i += 1
    if len(pos) != 3 or any(p.startswith('--') for p in pos):
        sys.stderr.write(__doc__)
        return 2
    try:
        run(pos[0], pos[1], pos[2], labs['--label-board'], labs['--label-ref'])
    except (OSError, ValueError, zlib.error) as e:
        sys.stderr.write('glmark2-sidebyside: %s\n' % e)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
