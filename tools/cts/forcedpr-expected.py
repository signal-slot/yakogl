#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""forcedpr-expected.py - host cross-check of gl_test case 217's expected image.

Case 217 (driver/tools/pvrlab/gl_test.c, "a forced partial render") draws N
(256) layers of 512x512 rectangles into a 1024x1024 target, depth LESS, and
compares the read-back with an image it computes on the target. This tool
computes that image again, independently (it paints layers in descending
rank, so the least rank is painted last, instead of comparing ranks), so an
expectation bug is found on the host and not in a board session.

  forcedpr-expected.py [--layers N]       sha256, histogram, checks
  forcedpr-expected.py --pixel X Y        the covering layers at a pixel
  forcedpr-expected.py --dump FILE        the RGBA bytes (bottom row first)
  forcedpr-expected.py --explain TEXT     TEXT holds 'got 0x%08x want 0x%08x'
                                          and optionally '(x,y)' (the FAIL line)

Layer k: x0 = 173k mod 512, y0 = 97k mod 512, size 512, colour
(53k+17, 97k+101, 193k+7) & 255 with alpha 255, depth rank 2579k mod 4093 (LESS:
the least rank wins); a pixel no layer covers is the clear, 0xff000000. Pixel
words are the read-back's: R | G << 8 | B << 16 | A << 24, y from the bottom.

Exit status: 0, or 1 when a check fails (coverage, ranks, depth order).
"""
import argparse
import hashlib
import re
import struct
import sys

SIZE, RECT, P, P2 = 1024, 512, 4093, 2579
CLEAR = 0xFF000000


def rank(k):
    return (k * P2) % P


def x0(k):
    return (k * 173) % RECT


def y0(k):
    return (k * 97) % RECT


def rgba(k):
    return ((53 * k + 17) & 255) | (((97 * k + 101) & 255) << 8) | \
        (((193 * k + 7) & 255) << 16) | CLEAR


def covers(k, x, y):
    return x0(k) <= x < x0(k) + RECT and y0(k) <= y < y0(k) + RECT


def covering(n, x, y):
    return sorted((k for k in range(n) if covers(k, x, y)), key=rank)


def winners(n):
    """Per row, the winning layer of every pixel (None: clear)."""
    order = sorted(range(n), key=rank, reverse=True)  # least rank painted last
    img = []
    for y in range(SIZE):
        row = [None] * SIZE
        for k in order:
            if y0(k) <= y < y0(k) + RECT:
                row[x0(k):x0(k) + RECT] = [k] * RECT
        img.append(row)
    return img


def image_bytes(n, img):
    tab = {k: struct.pack("<I", rgba(k)) for k in range(n)}
    tab[None] = struct.pack("<I", CLEAR)
    return b"".join(b"".join(tab[w] for w in row) for row in img)


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def check_coverage(n):
    """The vertices as the C float code computes them, through the viewport."""
    bad = []
    for k in range(n):
        for name, lo in (("x", x0(k)), ("y", y0(k))):
            for edge in (lo, lo + RECT):
                ndc = f32(f32(f32(edge) / f32(512.0)) - 1.0)
                win = (ndc + 1.0) * 512.0  # viewport 1024: (ndc + 1) * 1024 / 2
                if win != edge:
                    bad.append((k, name, edge, win))
    return bad


def depth16(k):
    z = f32(-0.9 + 1.8 * rank(k) / P)
    return int((z + 1.0) / 2.0 * 65535.0 + 0.5)


def depth_steps(n):
    ks = sorted(range(n), key=rank)
    return [(a, b, depth16(b) - depth16(a)) for a, b in zip(ks, ks[1:])]


def report(n):
    fail = 0
    print("forcedpr-expected: %d layers, %dx%d" % (n, SIZE, SIZE))
    img = winners(n)
    print("sha256 %s" % hashlib.sha256(image_bytes(n, img)).hexdigest())
    hist = {}
    for row in img:
        for w in set(row):
            hist[w] = hist.get(w, 0) + row.count(w)
    clear = hist.pop(None, 0)
    assert clear + sum(hist.values()) == SIZE * SIZE
    print("clear (no layer): %d pixels" % clear)
    print("layer-winner histogram (layer: winning pixels):")
    for i in range(0, n, 8):
        print("  " + "  ".join("%3d:%6d" % (k, hist.get(k, 0)) for k in range(i, min(i + 8, n))))
    zero = [k for k in range(n) if hist.get(k, 0) == 0]
    print("layers winning no pixel: %d %s" % (len(zero), zero[:16]))
    distinct = len({rank(k) for k in range(n)}) == n
    print("ranks distinct: %s" % ("yes" if distinct else "NO"))
    fail |= not distinct
    bad = check_coverage(n)
    if bad:
        fail = 1
        print("coverage: DISCREPANCY in %d edges, first %s" % (len(bad), bad[0]))
    else:
        print("coverage: exact whole-pixel coverage for all %d layers (xa..xb, ya..yb are "
              "multiples of 1/512 in float32 and land on whole pixels of a %d viewport)" % (n, SIZE))
    steps = depth_steps(n)
    low = [s for s in steps if s[2] < 1]
    if low:
        fail = 1
        print("depth: 16-bit order broken for %d rank pairs, first %s" % (len(low), low[0]))
    else:
        print("depth: float32 z keeps the %d ranks distinct and ordered in 16-bit unorm "
              "(smallest step %d)" % (n, min([s[2] for s in steps] or [0])))
    return fail


def fmt(w):
    return "0x%08x" % w


def near(a, b, tol):
    return all(abs(((a >> s) & 255) - ((b >> s) & 255)) <= tol for s in (0, 8, 16, 24))


def explain(text, n):
    m = re.search(r"got\s+(0x[0-9a-fA-F]+)\s+want\s+(0x[0-9a-fA-F]+)", text)
    if not m:
        sys.exit("forcedpr-expected: no 'got 0x... want 0x...' in the text")
    got, want = int(m.group(1), 16), int(m.group(2), 16)
    p = re.search(r"\((\d+),\s*(\d+)\)", text)
    pos = (int(p.group(1)), int(p.group(2))) if p else None
    print("got %s want %s%s" % (fmt(got), fmt(want), " at (%d,%d)" % pos if pos else ""))
    w = None
    if pos:
        cov = covering(n, *pos)
        w = cov[0] if cov else None
        exp = CLEAR if w is None else rgba(w)
        print("host: %d layer(s) cover it; winner %s; expected %s"
              % (len(cov), "none (clear)" if w is None else "layer %d (rank %d)" % (w, rank(w)), fmt(exp)))
        if not near(exp, want, 1):
            print("EXPECTATION MISMATCH: gl_test wants %s, the host computes %s" % (fmt(want), fmt(exp)))
    else:
        lw = [k for k in range(min(n, 256)) if rgba(k) == want]
        print("want is %s" % ("the clear" if want == CLEAR else "layer %s" % lw if lw else "no layer's colour"))
    if near(got, want, 1):
        print("got is within one count of want per channel: not a mismatch for gl_test")
        return
    if got == CLEAR:
        print("got is the clear colour: the layer was never drawn, or the clear was reloaded "
              "over it (a lost tile, or a partial render that reloaded the clear)")
    elif got == 0:
        print("got is 0: transparent black, memory never written (an unwritten or unstored tile)")
    else:
        ls = []
        for tol in (0, 1):
            ls = [k for k in range(min(n, 256)) if near(got, rgba(k), tol)]
            if ls:
                break
        if not ls:
            print("got is no layer's colour and not the clear: a blend, garbage or a wrong store")
        for j in ls:
            msg = "got is layer %d's colour (rank %d%s)" % (j, rank(j), ", within 1" if tol else "")
            if not pos:
                msg += "; give the '(x,y)' of the FAIL line to compare against the covering layers"
            elif not covers(j, *pos):
                msg += "; layer %d does NOT cover the pixel (a misplaced tile or a wrong address)" % j
            elif w is not None and rank(j) > rank(w):
                msg += "; it is BEHIND the winner (rank %d): the depth of an earlier pass was lost" % rank(w)
            elif w is not None and rank(j) < rank(w):
                msg += "; it is in front of the winner: the expectation or the depth compare is wrong"
            print(msg)


def main():
    ap = argparse.ArgumentParser(description="host cross-check of gl_test case 217's expected image")
    ap.add_argument("--layers", type=int, default=256)
    ap.add_argument("--pixel", nargs=2, type=int, metavar=("X", "Y"))
    ap.add_argument("--dump", metavar="FILE")
    ap.add_argument("--explain", metavar="TEXT")
    a = ap.parse_args()
    n = a.layers
    if not 1 <= n <= P:
        sys.exit("forcedpr-expected: --layers must be 1..%d" % P)
    if a.explain is not None:
        explain(a.explain, n)
        return 0
    if a.pixel:
        cov = covering(n, *a.pixel)
        w = cov[0] if cov else None
        print("pixel (%d,%d): covered by %d layer(s) %s" % (a.pixel[0], a.pixel[1], len(cov), cov))
        print("winner %s word %s" % (w, fmt(CLEAR if w is None else rgba(w))))
        return 0
    if a.dump:
        with open(a.dump, "wb") as f:
            f.write(image_bytes(n, winners(n)))
    return report(n)


if __name__ == "__main__":
    sys.exit(main())
