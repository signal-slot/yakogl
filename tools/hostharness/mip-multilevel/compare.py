#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Picture error of three mip levels a render (docs/notes/am62p-slow-terrain.md, candidate 2).

Three chains of an RGBA8 texture are made and compared, level by level, with the
reference:

  ref    the GL core's chain (src/gl/texture.rs, glGenerateMipmap): every level is the 2x2
         box of the one above, (sum + 2) / 4, the side halving by floor (min 1), a
         trailing odd row or column dropped, a side of 1 not halved (box extent 1).
  iter   the chain the PowerVR makes today, one render a level (mip_step_gpu_sample): one
         bilinear CLAMP_TO_EDGE tap at the centre of each destination texel, the
         fraction taken to 8 bits, rounded to nearest. Halving steps are the 2x2 box;
         NPOT odd steps (75 -> 37) are not.
  one    the candidate with the fewest taps: levels l+1..l+3 from level l in one render,
         ONE bilinear tap a target at the destination texel's centre scaled from level l
         (the same program as iter, a bigger ratio). Not a box for 2^j > 2.
  box    the candidate that keeps NPOT exact: target j (2^j) takes the aligned box of
         level l as (2^j / 2)^2 bilinear taps, each centred on a corner shared by four
         texels (fraction exactly 1/2), summed in the shader and rounded to nearest ONCE.
         The box starts at x * 2^j (an axis whose side reached 1 contributes extent 1),
         so it covers exactly the texels the iterated floor halving does.

Groups, as the note: level 1 from 0, then 2-4 from 1, 5-7 from 4, 8-10 from 7.

Per level: max abs error, mean abs error (all channels) and the share of bytes that
differ, against ref. The tolerance is the "tolerance" line of tools/cts/glmark2-pictures.txt
(levels per channel on a probe and on the mean, tools/cts/common-tally.py judge_picture).
A level whose max error is within it cannot move a sample of it by more than that.

usage: python3 -I compare.py [--sizes 800x600,256x256] [--images random,gradient,...]
       [--seed N] [--glmark2-src DIR] [--quick]
GLMARK2_SRC set: the PNGs of its data/textures are images too (tiled to the size).
"""
import os
import random
import re
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
PICTURES = os.path.join(HERE, "..", "..", "cts", "glmark2-pictures.txt")
GROUP = 3  # levels a render


def tolerance():
    try:
        for line in open(PICTURES):
            m = re.match(r"tolerance\s+(\d+)", line)
            if m:
                return int(m.group(1))
    except OSError:
        pass
    return 8


# ---- images: planes[c] is a list of h rows of w ints -------------------------------------


def synth(kind, w, h, rng):
    planes = [[[0] * w for _ in range(h)] for _ in range(4)]
    if kind == "random":
        for c in range(4):
            for y in range(h):
                planes[c][y] = [rng.randrange(256) for _ in range(w)]
    elif kind == "gradient":
        for y in range(h):
            for x in range(w):
                planes[0][y][x] = x * 255 // max(1, w - 1)
                planes[1][y][x] = y * 255 // max(1, h - 1)
                planes[2][y][x] = (x + y) * 255 // max(1, w + h - 2)
                planes[3][y][x] = 255 - planes[0][y][x]
    elif kind == "edges":  # hard edges at odd places: the rounding worst case
        for y in range(h):
            for x in range(w):
                v = 255 if ((x // 3) + (y // 5)) % 2 else 0
                planes[0][y][x], planes[1][y][x] = v, 255 - v
                planes[2][y][x] = (x * 7 + y * 13) % 256
                planes[3][y][x] = 255
    elif kind == "noise":  # smooth value noise, 3 octaves, like terrain's scenery
        grids = []
        for oct_ in range(3):
            s = 8 << oct_
            grids.append((s, [[rng.randrange(256) for _ in range(s + 2)] for _ in range(s + 2)]))
        for c in range(3):
            for y in range(h):
                for x in range(w):
                    acc = 0.0
                    for i, (s, g) in enumerate(grids):
                        fx, fy = x * s / w, y * s / h
                        x0, y0 = int(fx), int(fy)
                        tx, ty = fx - x0, fy - y0
                        v = (g[y0][x0] * (1 - tx) + g[y0][x0 + 1] * tx) * (1 - ty) + \
                            (g[y0 + 1][x0] * (1 - tx) + g[y0 + 1][x0 + 1] * tx) * ty
                        acc += v / (2 << i)
                    planes[c][y][x] = min(255, int(acc * 1.14))
        planes[3] = [[255] * w for _ in range(h)]
    else:
        raise SystemExit("unknown image kind %s" % kind)
    return planes


def read_png(path):
    """(w, h, [4 planes of rows]) of an 8-bit non-interlaced PNG, or None"""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    pos, idat, plte, hdr = 8, b"", None, None
    while pos < len(data):
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if typ == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"PLTE":
            plte = body
        elif typ == b"IDAT":
            idat += body
    w, h, depth, ctype, _, _, interlace = hdr
    if depth != 8 or interlace or ctype not in (0, 2, 3, 4, 6):
        return None
    nch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    raw = zlib.decompress(idat)
    stride = w * nch
    rows, prev = [], bytearray(stride)
    p = 0
    for _ in range(h):
        f = raw[p]
        cur = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for i in range(stride):
            a = cur[i - nch] if i >= nch else 0
            b = prev[i]
            c = prev[i - nch] if i >= nch else 0
            if f == 1:
                cur[i] = (cur[i] + a) & 255
            elif f == 2:
                cur[i] = (cur[i] + b) & 255
            elif f == 3:
                cur[i] = (cur[i] + ((a + b) >> 1)) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[i] = (cur[i] + pr) & 255
        rows.append(cur)
        prev = cur
    planes = [[[0] * w for _ in range(h)] for _ in range(4)]
    for y, r in enumerate(rows):
        for x in range(w):
            if ctype == 3:
                i = r[x]
                px = (plte[3 * i], plte[3 * i + 1], plte[3 * i + 2], 255)
            elif ctype == 0:
                px = (r[x], r[x], r[x], 255)
            elif ctype == 4:
                px = (r[2 * x], r[2 * x], r[2 * x], r[2 * x + 1])
            elif ctype == 2:
                px = (r[3 * x], r[3 * x + 1], r[3 * x + 2], 255)
            else:
                px = tuple(r[4 * x:4 * x + 4])
            for c in range(4):
                planes[c][y][x] = px[c]
    return w, h, planes


def tiled(src, w, h):
    sw, sh, sp = src
    return [[[sp[c][y % sh][x % sw] for x in range(w)] for y in range(h)] for c in range(4)]


# ---- chains ------------------------------------------------------------------------------


def sides(w, h):
    out = [(w, h)]
    while out[-1] != (1, 1):
        out.append((max(1, out[-1][0] >> 1), max(1, out[-1][1] >> 1)))
    return out


def rnd(num, den):
    return (2 * num + den) // (2 * den)  # round half up, integers


def apply(plane, taps_x, taps_y, den_x, den_y):
    """sum over the taps of plane (separable, integer weights), / (den_x * den_y), rounded"""
    tmp = []  # horizontal pass
    for row in plane:
        tmp.append([sum(row[i] * wt for i, wt in tx) for tx in taps_x])
    den = den_x * den_y
    out = []
    for ty in taps_y:
        out.append([rnd(sum(tmp[i][x] * wt for i, wt in ty), den) for x in range(len(taps_x))])
    return out


def taps_box(dn, e):
    """aligned box of extent e at x * e: pairs on a shared corner (weight 1 each, den e)"""
    t = []
    for x in range(dn):
        if e == 1:
            t.append([(x, 1)])
        else:
            t.append([(i, 1) for m in range(e // 2) for i in (x * e + 2 * m, x * e + 2 * m + 1)])
    return t, e


def taps_one(n, dn, frac_bits=8):
    """one bilinear CLAMP_TO_EDGE tap at the centre of dest texel x: sx = (x + .5) n / dn - .5,
    the fraction rounded to frac_bits (the sampler's weight precision)"""
    t, one = [], 1 << frac_bits
    for x in range(dn):
        num = (2 * x + 1) * n - dn  # sx * 2dn, >= 0 as n >= dn
        i0 = min(num // (2 * dn), n - 1)
        f = rnd((num % (2 * dn)) * one, 2 * dn)
        i1 = min(i0 + 1, n - 1)
        t.append([(i0, one - f), (i1, f)])
    return t, one


def step_ref(planes, sw, sh):
    dw, dh = max(1, sw >> 1), max(1, sh >> 1)
    tx, dx = taps_box(dw, 2 if sw > 1 else 1)
    ty, dy = taps_box(dh, 2 if sh > 1 else 1)
    return [apply(p, tx, ty, dx, dy) for p in planes]


def chain_ref(planes, w, h):
    out, sd = [planes], sides(w, h)
    for l in range(1, len(sd)):
        out.append(step_ref(out[-1], *sd[l - 1]))
    return out


def one_tap(planes, n, dn):
    tx, dx = taps_one(n[0], dn[0])
    ty, dy = taps_one(n[1], dn[1])
    return [apply(p, tx, ty, dx, dy) for p in planes]


def chain_iter(planes, w, h):
    out, sd = [planes], sides(w, h)
    for l in range(1, len(sd)):
        out.append(one_tap(out[-1], sd[l - 1], sd[l]))
    return out


def extent(sd, l, k, axis):
    e = 1
    for s in range(l, k):
        if sd[s][axis] > 1:
            e *= 2
    return e


def chain_multi(planes, w, h, variant):
    """level 1 from 0, then GROUP levels a render from the first of them"""
    out, sd = [planes], sides(w, h)
    out.append(chain_multi_step(out, sd, 0, 1, variant))
    base = 1
    while base < len(sd) - 1:
        top = min(len(sd) - 1, base + GROUP)
        for k in range(base + 1, top + 1):
            out.append(chain_multi_step(out, sd, base, k, variant))
        base = top
    return out


def chain_multi_step(out, sd, l, k, variant):
    src = out[l]
    if variant == "one":
        return one_tap(src, sd[l], sd[k])
    tx, dx = taps_box(sd[k][0], extent(sd, l, k, 0))
    ty, dy = taps_box(sd[k][1], extent(sd, l, k, 1))
    return [apply(p, tx, ty, dx, dy) for p in src]


def errors(a, b):
    mx = tot = diff = n = 0
    for pa, pb in zip(a, b):
        for ra, rb in zip(pa, pb):
            for x, y in zip(ra, rb):
                d = abs(x - y)
                mx = max(mx, d)
                tot += d
                diff += d != 0
                n += 1
    return mx, tot / n, 100.0 * diff / n


def main(argv):
    opts = {"sizes": "800x600,256x256", "images": "random,gradient,edges,noise", "seed": "1", "glmark2-src": ""}
    quick = False
    i = 1
    while i < len(argv):
        a = argv[i]
        if a == "--quick":
            quick = True
        elif a.startswith("--") and a[2:] in opts and i + 1 < len(argv):
            i += 1
            opts[a[2:]] = argv[i]
        else:
            print(__doc__)
            return 2
        i += 1
    tol = tolerance()
    rng = random.Random(int(opts["seed"]))
    sizes = [tuple(int(v) for v in s.split("x")) for s in opts["sizes"].split(",")]
    kinds = opts["images"].split(",")
    pngs = []
    src = opts["glmark2-src"] or os.environ.get("GLMARK2_SRC")
    if src and not quick:
        d = os.path.join(src, "data", "textures")
        if os.path.isdir(d):
            for n in sorted(os.listdir(d)):
                if n.endswith(".png") and not n.startswith("jellyfish"):
                    img = read_png(os.path.join(d, n))
                    if img:
                        pngs.append((n, img))
    print("tolerance %d levels per channel (tools/cts/glmark2-pictures.txt)" % tol)
    print("variants vs ref (GL core box): iter = today's one render a level, one = 1 tap a target, "
          "box = aligned box taps")
    print("cell: max / mean / %differ")
    worst = {}  # (size, level, variant) -> max err
    for (w, h) in sizes:
        sd = sides(w, h)
        names = [(k, None) for k in kinds] + [(n, im) for n, im in pngs]
        for name, im in names:
            planes = synth(name, w, h, rng) if im is None else tiled(im, w, h)
            ref = chain_ref(planes, w, h)
            chains = {"iter": chain_iter(planes, w, h), "one": chain_multi(planes, w, h, "one"),
                      "box": chain_multi(planes, w, h, "box")}
            print("\n%dx%d %s" % (w, h, name))
            print("%5s %9s | %-22s | %-22s | %-22s" % ("level", "size", "iter", "one", "box"))
            for l in range(1, len(sd)):
                cells = []
                for v in ("iter", "one", "box"):
                    mx, mean, pc = errors(chains[v][l], ref[l])
                    cells.append("%3d / %5.3f / %5.1f%%" % (mx, mean, pc))
                    key = ((w, h), l, v)
                    worst[key] = max(worst.get(key, 0), mx)
                print("%5d %4dx%-4d | %-22s | %-22s | %-22s" % (l, sd[l][0], sd[l][1], *cells))
    print("\nworst max abs error over all images, per level (tolerance %d):" % tol)
    print("%9s %5s %9s | %-14s | %-14s | %-14s" % ("size", "level", "level size", "iter", "one", "box"))
    bad = 0
    for (w, h) in sizes:
        sd = sides(w, h)
        for l in range(1, len(sd)):
            cells = []
            for v in ("iter", "one", "box"):
                m = worst[((w, h), l, v)]
                cells.append("%3d %s" % (m, "within" if m <= tol else "OUT"))
                if v == "box" and m > tol:
                    bad += 1
            print("%4dx%-4d %5d %4dx%-4d | %-14s | %-14s | %-14s" % (w, h, l, *sd[l], *cells))
    print("\nbox variant: %d level(s) out of tolerance" % bad)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
