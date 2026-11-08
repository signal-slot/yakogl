#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""A dumped glmark2 picture beside its reference, with a difference image.

    tools/cts/glmark2-picdiff.py <board.png> <ref.png> <out.png> [--threshold N]
    tools/cts/glmark2-picdiff.py --self-test

<board.png> is a decoded frame dump (tools/cts/glmark2-fb.py), <ref.png> a
reference of the same size (e.g. a host llvmpipe picture). <out.png> is three
panels side by side: the board, the reference and the per-pixel |diff| times 4
(clamped). A row that glmark2-fb.py painted magenta (damaged) is magenta in all
three panels and is left out of every number.

One line is printed: mean |diff| per channel; the share of pixels differing by
more than N levels (default 24) in any channel; the bounding box of those
pixels (x0,y0-x1,y1, or none); the share of black pixels in each picture; and
both pictures' five probes of driver/docs/glmark2.md (c (w/2,h/2),
tl (w/4,h/4), tr (3w/4,h/4), bl (w/4,3h/4), br (3w/4,3h/4); on 800x600
(400,300), (200,150), (600,150), (200,450), (600,450)) as rrggbb, "damaged"
where the probe is in a magenta row.

Exit 0 on any valid input (judging is for a person), 2 on bad input (size
mismatch, unreadable or unsupported file). Stdlib only; reads 8-bit grey,
RGB, grey+alpha and RGBA non-interlaced PNGs (alpha dropped).
"""
import binascii
import struct
import sys
import zlib

MAGENTA = (255, 0, 255)
GAIN = 4


class BadInput(Exception):
    pass


def read_png(path):
    """-> (w, h, rows of w*3 RGB bytes)"""
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError as e:
        raise BadInput("%s: %s" % (path, e.strerror))
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise BadInput("%s: not a PNG" % path)
    pos, idat, hdr = 8, [], None
    while pos + 8 <= len(data):
        n, tag = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if len(body) != n:
            raise BadInput("%s: truncated" % path)
        if tag == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif tag == b"IDAT":
            idat.append(body)
        elif tag == b"IEND":
            break
        pos += 12 + n
    if hdr is None or not idat:
        raise BadInput("%s: no image data" % path)
    w, h, depth, ctype, _, _, interlace = hdr
    chans = {0: 1, 2: 3, 4: 2, 6: 4}.get(ctype)
    if depth != 8 or chans is None or interlace:
        raise BadInput("%s: only 8-bit non-interlaced grey/RGB/RGBA PNGs are read" % path)
    try:
        raw = zlib.decompress(b"".join(idat))
    except zlib.error:
        raise BadInput("%s: bad image data" % path)
    stride = w * chans
    if len(raw) < (stride + 1) * h:
        raise BadInput("%s: image data too short" % path)
    prev = bytearray(stride)
    rows = []
    for y in range(h):
        off = y * (stride + 1)
        f = raw[off]
        cur = bytearray(raw[off + 1:off + 1 + stride])
        if f == 1:
            for i in range(chans, stride):
                cur[i] = (cur[i] + cur[i - chans]) & 255
        elif f == 2:
            for i in range(stride):
                cur[i] = (cur[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = cur[i - chans] if i >= chans else 0
                cur[i] = (cur[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = cur[i - chans] if i >= chans else 0
                c = prev[i - chans] if i >= chans else 0
                b = prev[i]
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else b if pb <= pc else c
                cur[i] = (cur[i] + pr) & 255
        elif f != 0:
            raise BadInput("%s: bad filter %d" % (path, f))
        prev = cur
        if chans == 3:
            rows.append(bytes(cur))
        elif chans == 4:
            rows.append(b"".join(bytes(cur[i * 4:i * 4 + 3]) for i in range(w)))
        else:
            rows.append(b"".join(bytes((cur[i * chans],)) * 3 for i in range(w)))
    return w, h, rows


def write_png(path, w, h, rows):
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(tag, body):
        c = struct.pack(">I", len(body)) + tag + body
        return c + struct.pack(">I", binascii.crc32(tag + body) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def compare(bsize, brows, rsize, rrows, thr):
    """-> (out rows, result line)"""
    if bsize != rsize:
        raise BadInput("size mismatch: board %dx%d, reference %dx%d" % (bsize + rsize))
    w, h = bsize
    mag = bytes(MAGENTA) * w
    damaged = [r == mag for r in brows]
    out = []
    n = nd = nbb = nbr = 0
    ssum = [0, 0, 0]
    x0 = y0 = x1 = y1 = None
    for y in range(h):
        if damaged[y]:
            out.append(mag * 3)
            continue
        b, r = brows[y], rrows[y]
        d = bytes(abs(p - q) for p, q in zip(b, r))
        out.append(b + r + bytes(min(255, v * GAIN) for v in d))
        n += w
        for c in range(3):
            ssum[c] += sum(d[c::3])
        for x in range(w):
            if max(d[x * 3:x * 3 + 3]) > thr:
                nd += 1
                x0 = x if x0 is None else min(x0, x)
                x1 = x if x1 is None else max(x1, x)
                y0 = y if y0 is None else y0
                y1 = y
            nbb += b[x * 3:x * 3 + 3] == b"\0\0\0"
            nbr += r[x * 3:x * 3 + 3] == b"\0\0\0"

    def pct(v):
        return "%.1f%%" % (100.0 * v / n) if n else "n/a"

    def probes(rows):
        pts = (("c", w // 2, h // 2), ("tl", w // 4, h // 4), ("tr", w * 3 // 4, h // 4),
               ("bl", w // 4, h * 3 // 4), ("br", w * 3 // 4, h * 3 // 4))
        return ", ".join("%s %s" % (k, "damaged" if damaged[y] else rows[y][x * 3:x * 3 + 3].hex())
                         for k, x, y in pts)

    mean = " ".join("%.2f" % (s / n) if n else "n/a" for s in ssum)
    bbox = "%d,%d-%d,%d" % (x0, y0, x1, y1) if x0 is not None else "none"
    line = ("mean |diff| %s; %s of pixels differ by more than %d in any channel; bbox %s; "
            "black board %s ref %s; board probes %s; ref probes %s; %d damaged row(s) left out"
            % (mean, pct(nd), thr, bbox, pct(nbb), pct(nbr), probes(brows), probes(rrows),
               sum(damaged)))
    return out, line


def run(board, ref, out, thr):
    try:
        bw, bh, brows = read_png(board)
        rw, rh, rrows = read_png(ref)
        orows, line = compare((bw, bh), brows, (rw, rh), rrows, thr)
        write_png(out, bw * 3, bh, orows)
    except BadInput as e:
        print("glmark2-picdiff: " + str(e))
        return 2
    except OSError as e:
        print("glmark2-picdiff: %s: %s" % (out, e.strerror))
        return 2
    print(line)
    return 0


def self_test():
    import contextlib
    import io
    import os
    import tempfile
    w, h = 80, 60
    d = tempfile.mkdtemp(prefix="glmark2-picdiff.")
    ok = True

    def expect(what, got, want):
        nonlocal ok
        if got != want:
            ok = False
            print("FAIL %s: got %r, want %r" % (what, got, want))

    ref = [bytes([100, 100, 100]) * w for _ in range(h)]
    brd = [bytearray(r) for r in ref]
    for y in range(10, 20):          # a 10x10 block 50 levels off in R only
        for x in range(30, 40):
            brd[y][x * 3] = 150
    brd[0][:] = bytes(w * 3)         # one black row in both pictures
    ref[0] = bytes(w * 3)
    brd[5] = bytearray(bytes(MAGENTA) * w)   # damaged row, left out
    brd = [bytes(r) for r in brd]
    paths = {k: os.path.join(d, k + ".png") for k in ("board", "ref", "out", "small", "junk")}
    write_png(paths["board"], w, h, brd)
    write_png(paths["ref"], w, h, ref)
    write_png(paths["small"], 8, 6, [bytes(24)] * 6)
    with open(paths["junk"], "wb") as f:
        f.write(b"junk")
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = run(paths["board"], paths["ref"], paths["out"], 24)
    line = buf.getvalue()
    expect("exit", rc, 0)
    n = w * (h - 1)
    expect("mean R", "mean |diff| %.2f 0.00 0.00;" % (100 * 50 / n) in line, True)
    expect("share", "%.1f%% of pixels differ by more than 24" % (100.0 * 100 / n) in line, True)
    expect("bbox", "bbox 30,10-39,19;" in line, True)
    expect("black", "black board %.1f%% ref %.1f%%" % (100.0 * w / n, 100.0 * w / n) in line, True)
    expect("probe", "board probes c 646464, tl 646464" in line, True)
    expect("damaged", "1 damaged row(s)" in line, True)
    ow, oh, orows = read_png(paths["out"])
    expect("out size", (ow, oh), (w * 3, h))
    expect("out board panel", orows[10][:w * 3], brd[10])
    expect("out ref panel", orows[10][w * 3:w * 6], ref[10])
    expect("out diff panel", orows[10][w * 6 + 90:w * 6 + 93], bytes([200, 0, 0]))
    expect("out magenta", orows[5], bytes(MAGENTA) * w * 3)
    with contextlib.redirect_stdout(io.StringIO()):
        expect("size mismatch", run(paths["board"], paths["small"], paths["out"], 24), 2)
        expect("missing file", run(os.path.join(d, "none.png"), paths["ref"], paths["out"], 24), 2)
        expect("junk file", run(paths["junk"], paths["ref"], paths["out"], 24), 2)
    print("self-test: " + ("ok" if ok else "FAILED"))
    return 0 if ok else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    thr = 24
    if "--threshold" in argv:
        i = argv.index("--threshold")
        try:
            thr = int(argv[i + 1])
        except (IndexError, ValueError):
            print(__doc__)
            return 2
        del argv[i:i + 2]
    if len(argv) != 4:
        print(__doc__)
        return 2
    return run(argv[1], argv[2], argv[3], thr)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
