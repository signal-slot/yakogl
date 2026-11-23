#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""How far a camera-photo probe drifts from the framebuffer probe on real glmark2 frames.

    tools/cts/glmark2-photo-tolerance.py [options] <frame.png | dir> ...
    options: --random N   seeded random camera draws besides the fixed ones (default 6)
             --seed N     seed of those draws (default 1)
             --flag N     edge flag: a probe whose patch changes by more than N levels
                          under a +-4 px shift of the clean frame (default 24)
             --mean       also report the drift of the 40x30 grid mean
             --csv FILE   write one row per scene and probe
             --distinct A B   can a photo tell flat colour A from flat colour B (RRGGBB)?

Input: 800x600 frames (glmark2-host-ref.sh writes DIR/<slug>.png, decoded by
glmark2-fb.py; a directory means every *.png in it). Stdlib only.

Camera model (what a tailvision take_screenshot of the i.MX8MP panel may do), applied
to the frame placed at the top-left of the black 1280x800 panel, in this order:
  1. RGB565 quantisation (the i.MX8MP framebuffer; bit-replicated back to 8 bits);
     this is also the framebuffer probe the photo is compared with: a single pixel
     at the sample's probe point (main.cpp: w/2,h/2 and w/4,3w/4 x h/4,3h/4).
  2. The panel emits light = (v/255)^2.2; the camera blurs it (Gaussian, sigma 0.5-2
     panel px, plus the box of the downscale) and encodes it again with the camera's
     gamma (1.8-2.4): v' = light^(1/gamma). A camera gamma of 2.2 is neutral.
  3. A residual perspective/scale error: the four screenshot corners displaced by up
     to 3 px (bilinear field between them), so the probes sit up to ~3 px off.
  4. The screenshot is W px wide (800-1280), the panel's aspect (W x W*5/8).
  5. A per-channel colour cast of up to +-15 levels, clamped.
Moire and sensor noise are not modelled (the reader's 7x7 median is what absorbs
them). The result is then read by glmark2-photo-probes.py's Photo/probe_points (not
modified), so the patch size and sample points are the reader's.

The configs are a fixed set (each extreme alone, then combinations) plus --random
seeded draws of extremes. Per probe two worst cases (max over configs and channels of
|photo - framebuffer|):
  geo   blur, warp and scale with the colour model neutral (gamma 2.2, no cast):
        drift from misregistration alone, a property of the picture's content
  all   every config, colour model included: what a photo can be trusted to within
A probe is flagged EDGE when it sits on an edge or a high-gradient area: its 7x7
median patch of the clean frame moves by more than --flag levels under a shift of
+-4 px (or geo is above it). Only a frame dump can decide such a probe's value.
Exit 0; 2 for bad input.
"""
import importlib.util
import itertools
import math
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def load_reader():
    spec = importlib.util.spec_from_file_location(
        "glmark2_photo_probes", os.path.join(HERE, "glmark2-photo-probes.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


R = load_reader()
PW, PH = R.PANEL
FW, FH = R.WINDOW[2], R.WINDOW[3]
DISPLAY_GAMMA = 2.2
SHIFTS = (-4, 0, 4)


class Cfg:
    def __init__(self, name, sigma=0.5, gamma=2.2, cast=(0, 0, 0), warp=((0, 0),) * 4, width=1280):
        self.name, self.sigma, self.gamma, self.cast = name, sigma, gamma, cast
        self.warp, self.width = warp, width

    @property
    def geo_only(self):
        return self.gamma == DISPLAY_GAMMA and self.cast == (0, 0, 0)

    def neutral_colour(self):
        return Cfg(self.name, self.sigma, DISPLAY_GAMMA, (0, 0, 0), self.warp, self.width)


def configs(nrandom, seed):
    z = ((0, 0),) * 4
    c = [Cfg("mild"), Cfg("blur2", sigma=2.0), Cfg("down800", sigma=1.0, width=800),
         Cfg("down1000", sigma=1.0, width=1000)]
    # corners: tl, tr, bl, br, each (dx, dy) in screenshot px
    for nm, w in (("scale-out", ((-3, -3), (3, -3), (-3, 3), (3, 3))),
                  ("scale-in", ((3, 3), (-3, 3), (3, -3), (-3, -3))),
                  ("shear", ((3, 0), (3, 0), (-3, 0), (-3, 0))),
                  ("keystone", ((3, 3), (-3, 3), (0, -3), (0, -3))),
                  ("shift", ((3, 3),) * 4)):
        c.append(Cfg("warp-" + nm, warp=w))
        c.append(Cfg("warp-%s-blur2" % nm, sigma=2.0, warp=w))
    c += [Cfg("gamma1.8", gamma=1.8), Cfg("gamma2.4", gamma=2.4),
          Cfg("cast+", cast=(15, 15, 15)), Cfg("cast-", cast=(-15, -15, -15)),
          Cfg("cast+-+", cast=(15, -15, 15)), Cfg("cast-+-", cast=(-15, 15, -15)),
          Cfg("worst+", sigma=2.0, gamma=1.8, cast=(15, 15, 15), warp=c[4].warp, width=800),
          Cfg("worst-", sigma=2.0, gamma=2.4, cast=(-15, -15, -15), warp=c[6].warp, width=800)]
    rng = random.Random(seed)
    for i in range(nrandom):
        w = tuple((rng.choice((-3, 3)), rng.choice((-3, 3))) for _ in range(4))
        c.append(Cfg("rand%d" % (i + 1), sigma=rng.choice((0.5, 2.0)), gamma=rng.choice((1.8, 2.4)),
                     cast=tuple(rng.choice((-15, 15)) for _ in range(3)), warp=w,
                     width=rng.choice((800, 1280))))
    return c


def q565(v, bits):
    q = v >> (8 - bits)
    return (q << (8 - bits)) | (q >> (2 * bits - 8))


class Panel:
    """The 1280x800 panel showing an RGB565-quantised frame at (0, 0), black elsewhere."""

    def __init__(self, w, h, rows):
        if (w, h) != (FW, FH):
            raise R.Bad("frame is %dx%d, expected %dx%d" % (w, h, FW, FH))
        self.rows = []
        for r in rows:
            self.rows.append(bytes(q565(r[i], 5 if i % 3 != 1 else 6) for i in range(len(r))))

    def px(self, x, y):
        if 0 <= x < FW and 0 <= y < FH:
            r = self.rows[y]
            return r[3 * x], r[3 * x + 1], r[3 * x + 2]
        return 0, 0, 0


LIN = [(v / 255.0) ** DISPLAY_GAMMA for v in range(256)]


class Camera:
    """A lazy screenshot (R.Photo-compatible rows) of a Panel under a Cfg."""

    def __init__(self, panel, cfg):
        self.p, self.cfg = panel, cfg
        self.w, self.h = cfg.width, round(cfg.width * PH / PW)
        self.scale = PW / float(self.w)
        # effective Gaussian sigma in panel px: lens blur + the box of the downscale
        s = cfg.sigma * self.scale
        self.sig = math.sqrt(s * s + self.scale * self.scale / 12.0)
        self.rad = int(math.ceil(3 * self.sig))
        self.cache = {}
        self.enc = [(i / 1023.0) ** (1.0 / cfg.gamma) * 255.0 for i in range(1024)]
        self.rows = _Rows(self)

    def warp(self, x, y):
        fx, fy = x / self.w, y / self.h
        (a, b), (c, d), (e, f), (g, h) = self.cfg.warp  # tl, tr, bl, br
        dx = (1 - fy) * ((1 - fx) * a + fx * c) + fy * ((1 - fx) * e + fx * g)
        dy = (1 - fy) * ((1 - fx) * b + fx * d) + fy * ((1 - fx) * f + fx * h)
        return dx, dy

    def sample(self, x, y):
        k = (x, y)
        v = self.cache.get(k)
        if v is None:
            dx, dy = self.warp(x + 0.5, y + 0.5)
            u = (x + 0.5 + dx) * self.scale
            vv = (y + 0.5 + dy) * self.scale
            cx, cy = int(math.floor(u)), int(math.floor(vv))
            acc, wsum, inv = [0.0, 0.0, 0.0], 0.0, 1.0 / (2 * self.sig * self.sig)
            for j in range(cy - self.rad, cy + self.rad + 1):
                ey = (j + 0.5 - vv) ** 2
                for i in range(cx - self.rad, cx + self.rad + 1):
                    wt = math.exp(-((i + 0.5 - u) ** 2 + ey) * inv)
                    p = self.p.px(i, j)
                    acc[0] += wt * LIN[p[0]]
                    acc[1] += wt * LIN[p[1]]
                    acc[2] += wt * LIN[p[2]]
                    wsum += wt
            out = []
            for ch in range(3):
                lv = acc[ch] / wsum
                e = self.enc[min(1023, int(lv * 1023.0 + 0.5))]
                out.append(min(255, max(0, int(round(e + self.cfg.cast[ch])))))
            v = self.cache[k] = tuple(out)
        return v


class _Row:
    def __init__(self, cam, y):
        self.cam, self.y = cam, y

    def __getitem__(self, i):
        return self.cam.sample(i // 3, self.y)[i % 3]


class _Rows:
    def __init__(self, cam):
        self.cam = cam

    def __getitem__(self, y):
        return _Row(self.cam, y)


def fb_probes(panel):
    return [panel.px(x, y) for x, y in R.probe_points(0, 0, FW, FH)]


def fb_mean(panel):
    gx, gy = R.GRID
    s = [0, 0, 0]
    for j in range(gy):
        for i in range(gx):
            p = panel.px((2 * i + 1) * FW // (2 * gx), (2 * j + 1) * FH // (2 * gy))
            for c in range(3):
                s[c] += p[c]
    return [v // (gx * gy) for v in s]


def read(cam, want_mean):
    photo = R.Photo(cam.w, cam.h, cam.rows, 0)
    probes = [photo.median(x, y, R.HALF) for x, y in R.probe_points(*R.WINDOW)]
    mean = None
    if want_mean:
        mean = R.measure(photo, R.WINDOW)[1]
    return probes, mean


def diff(a, b):
    return max(abs(x - y) for x, y in zip(a, b))


def sensitivity(panel):
    """Per probe: how far the 7x7 median patch of the clean frame moves under +-4 px."""
    out = []
    for (x, y) in R.probe_points(*R.WINDOW):
        base = panel.px(x, y)
        worst = 0
        for dx, dy in itertools.product(SHIFTS, SHIFTS):
            pts = [panel.px(x + dx + i, y + dy + j) for j in range(-R.HALF, R.HALF + 1)
                   for i in range(-R.HALF, R.HALF + 1)]
            med = tuple(sorted(p[c] for p in pts)[len(pts) // 2] for c in range(3))
            worst = max(worst, diff(med, base))
        out.append(worst)
    return out


def analyse(path, cfgs, flag, want_mean):
    with open(path, "rb") as f:
        w, h, rows = R.decode_png(f.read())
    panel = Panel(w, h, rows)
    ref = fb_probes(panel)
    rmean = fb_mean(panel) if want_mean else None
    geo, tot = [0] * 5, [0] * 5
    gmean, tmean = 0, 0
    for cfg in cfgs:
        for neutral in ((True, False) if not cfg.geo_only else (True,)):
            c = cfg.neutral_colour() if neutral else cfg
            probes, mean = read(Camera(panel, c), want_mean)
            for i in range(5):
                d = diff(probes[i], ref[i])
                tot[i] = max(tot[i], d)
                if neutral:
                    geo[i] = max(geo[i], d)
            if want_mean:
                d = diff(mean, rmean)
                tmean = max(tmean, d)
                if neutral:
                    gmean = max(gmean, d)
    sens = sensitivity(panel)
    return {"ref": ref, "geo": geo, "all": tot, "sens": sens,
            "edge": [max(sens[i], geo[i]) > flag for i in range(5)],
            "mean": (rmean, gmean, tmean) if want_mean else None}


def distinct(a, b, cfgs):
    """Flat colours a and b through every colour config: can the photo tell them apart?"""
    res = []
    for hexv in (a, b):
        col = tuple(int(hexv[i:i + 2], 16) for i in (0, 2, 4))
        rows = [bytes(col * FW) for _ in range(FH)]
        panel = Panel(FW, FH, rows)
        res.append([read(Camera(panel, c), False)[0][1] for c in cfgs])
    print("distinct %s vs %s (the tl probe of a flat patch, every camera config):" % (a, b))
    sep, black_a, black_b = True, [], []
    lo = [[min(r[i][ch] for i in range(len(cfgs))) for ch in range(3)] for r in res]
    hi = [[max(r[i][ch] for i in range(len(cfgs))) for ch in range(3)] for r in res]
    for n, r in zip((a, b), res):
        print("  %s reads %s .. %s per channel; reader says black in %d of %d configs" % (
            n, "/".join("%d" % min(p[c] for p in r) for c in range(3)),
            "/".join("%d" % max(p[c] for p in r) for c in range(3)),
            sum(1 for p in r if max(p) <= R.BLACK_LEVEL), len(r)))
    # separable by value for an unknown cast/gamma: disjoint ranges on some channel
    disjoint = any(hi[0][ch] < lo[1][ch] or hi[1][ch] < lo[0][ch] for ch in range(3))
    # same camera state for both: the pair of readings differs by at least this
    same = min(diff(res[0][i], res[1][i]) for i in range(len(cfgs)))
    print("  ranges disjoint on a channel (camera state unknown): %s; smallest difference "
          "under one camera state: %d levels" % ("yes" if disjoint else "NO", same))
    return disjoint


def collect(args):
    out = []
    for a in args:
        if os.path.isdir(a):
            out += [os.path.join(a, n) for n in sorted(os.listdir(a)) if n.endswith(".png")]
        else:
            out.append(a)
    return out


def main(argv):
    nrand, seed, flag, want_mean, csv, dist, files = 6, 1, 24, False, None, None, []
    try:
        i = 0
        while i < len(argv):
            o = argv[i]
            if o == "--random":
                nrand, i = int(argv[i + 1]), i + 2
            elif o == "--seed":
                seed, i = int(argv[i + 1]), i + 2
            elif o == "--flag":
                flag, i = int(argv[i + 1]), i + 2
            elif o == "--mean":
                want_mean, i = True, i + 1
            elif o == "--csv":
                csv, i = argv[i + 1], i + 2
            elif o == "--distinct":
                dist, i = argv[i + 1:i + 3], i + 3
                if len(dist) != 2 or any(len(d) != 6 for d in dist):
                    raise ValueError
                int(dist[0], 16), int(dist[1], 16)
            elif o.startswith("--"):
                raise R.Bad("unknown option %s" % o)
            else:
                files.append(o)
                i += 1
    except (IndexError, ValueError):
        print("glmark2-photo-tolerance: bad or missing value for an option", file=sys.stderr)
        return 2
    except R.Bad as e:
        print("glmark2-photo-tolerance: %s" % e, file=sys.stderr)
        return 2
    cfgs = configs(nrand, seed)
    if dist:
        distinct(dist[0], dist[1], cfgs)
        if not files:
            return 0
    files = collect(files)
    if not files:
        print("glmark2-photo-tolerance: no frames given", file=sys.stderr)
        return 2
    print("# %d camera configs, edge flag > %d levels; "
          "drift = worst channel difference to the framebuffer probe" % (len(cfgs), flag))
    print("%-48s %-3s %-8s %4s %4s %4s  %s" % ("scene", "pr", "fb", "geo", "all", "sens", "flag"))
    flagged, csvrows = [], []
    for path in files:
        slug = os.path.splitext(os.path.basename(path))[0]
        try:
            r = analyse(path, cfgs, flag, want_mean)
        except (OSError, R.Bad) as e:
            print("glmark2-photo-tolerance: %s: %s" % (path, e), file=sys.stderr)
            return 2
        for i, n in enumerate(R.NAMES):
            mark = "EDGE" if r["edge"][i] else ""
            if mark:
                flagged.append("%s %s" % (slug, n))
            print("%-48s %-3s %02x%02x%02x %4d %4d %4d  %s" % (
                slug[:48], n, *r["ref"][i], r["geo"][i], r["all"][i], r["sens"][i], mark))
            csvrows.append("%s,%s,%02x%02x%02x,%d,%d,%d,%d" % (
                slug, n, *r["ref"][i], r["geo"][i], r["all"][i], r["sens"][i], int(r["edge"][i])))
        if r["mean"]:
            rm, gm, tm = r["mean"]
            print("%-48s %-3s %-8s %4d %4d" % (slug[:48], "avg", "%d/%d/%d" % tuple(rm), gm, tm))
    print("# EDGE-flagged probes (only a frame dump can decide): %s" % (", ".join(flagged) or "none"))
    if csv:
        with open(csv, "w", encoding="utf-8") as f:
            f.write("scene,probe,fb,geo,all,sens,edge\n" + "\n".join(csvrows) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
