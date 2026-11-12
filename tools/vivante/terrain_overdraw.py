#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Terrain's early-depth saving, modelled from glmark2's draw order.

Run: python3 -I tools/vivante/terrain_overdraw.py [--offsets=0,0.5] [--flat] [--hscale=1.0]

Host only, pure Python (no numpy), about a minute. It rasterises glmark2's
`terrain` mesh at 800x600 with the scene's matrices and a 16-bit depth
buffer, then replays the fragments in several triangle orders and counts, per
order:

  covered   every fragment a triangle covers inside the clip volume. With
            late depth (linear target, `backend.rs` has no early depth) each
            one runs the 148-instruction terrain fragment shader.
  passing   the fragments that pass the depth test (LESS, write on) when
            they arrive. With early depth only these run the shader.
  visible   pixels the mesh covers: the floor of `passing` (one winner each).

Sources, glmark2 at 22c527cb (`$GLMARK2_SRC`):
  src/scene-terrain.cpp:270-292   projection 40 deg, aspect 4/3, near 2, far
                                  4000; camera (-1200,800,1200) -> origin;
                                  model = translate(0,-125,0) rotate(-90,x)
  src/scene-terrain/terrain-renderer.cpp:243 and 134-220
                                  make_grid(256,256,6000,6000), grid_conf:
                                  two triangles a cell (ll,lr,ur),(ll,ur,ul)
  src/mesh.cpp:604-640            make_grid: i (x) outer, j (y) inner
  data/shaders/terrain.vert       vertex moved by 375 * height along the
                                  normal (world up); height = tDisplacement.x
  data/shaders/terrain-noise.frag the height map: |snoise| over 4 octaves at
                                  (1.5u+off, 1.5(1-v), -1), RGBA8 so clamped
                                  to 1; off = 0.05 * seconds into the scene

Approximations (see docs/notes/gc-terrain-overdraw.md): the height is the
noise at the vertex's uv, not the bilinear read of the 256x256 RGBA8 map;
fragments are exact per pixel, not per 2x2 quad; near-plane clipping is exact
(part of the plane lies behind the camera); the far plane clips per fragment.
"""
import math
import random
import sys
from array import array

W, H = 800, 600
N = 256                      # cells a side
SIZE = 6000.0
HEIGHT_SCALE = 375.0
FOVY, ASPECT, NEAR, FAR = 40.0, W / H, 2.0, 4000.0
EYE = (-1200.0, 800.0, 1200.0)
MODEL_Y = -125.0
DEPTH_MAX = 65535            # the terrain target's depth16
FS_INSNS = 148               # gc-slow-terrain.md: terrain.frag
NS_PER_INSN = (0.6, 0.7, 0.8)  # gc-slow-terrain.md, "The GPU calibration"


# ---- simplex noise, a transcription of data/shaders/terrain-noise.frag ----
def _floor(x):
    return math.floor(x)


def _mod(x, y):
    return x - y * math.floor(x / y)


def _permute(x):
    return _mod((x * 34.0 + 1.0) * x, 289.0)


def snoise(vx, vy, vz):
    c1, c2 = 1.0 / 6.0, 1.0 / 3.0
    s = (vx + vy + vz) * c2
    i = (_floor(vx + s), _floor(vy + s), _floor(vz + s))
    t = (i[0] + i[1] + i[2]) * c1
    x0 = (vx - i[0] + t, vy - i[1] + t, vz - i[2] + t)
    g = (1.0 if x0[0] >= x0[1] else 0.0,   # step(x0.yzx, x0.xyz)
         1.0 if x0[1] >= x0[2] else 0.0,
         1.0 if x0[2] >= x0[0] else 0.0)
    l = (1.0 - g[0], 1.0 - g[1], 1.0 - g[2])
    lz = (l[2], l[0], l[1])                # l.zxy
    i1 = tuple(min(a, b) for a, b in zip(g, lz))
    i2 = tuple(max(a, b) for a, b in zip(g, lz))
    x1 = tuple(x0[k] - i1[k] + c1 for k in range(3))
    x2 = tuple(x0[k] - i2[k] + 2 * c1 for k in range(3))
    x3 = tuple(x0[k] - 1.0 + 3 * c1 for k in range(3))
    i = tuple(_mod(a, 289.0) for a in i)
    pz = [_permute(i[2] + d) for d in (0.0, i1[2], i2[2], 1.0)]
    py = [_permute(pz[k] + i[1] + d)
          for k, d in enumerate((0.0, i1[1], i2[1], 1.0))]
    p = [_permute(py[k] + i[0] + d)
         for k, d in enumerate((0.0, i1[0], i2[0], 1.0))]
    n_ = 1.0 / 7.0
    ns = (n_ * 2.0 - 0.0, n_ * 0.5 - 1.0, n_ * 1.0 - 0.0)   # n_*D.wyz - D.xzx
    xs, ys, hs, grads = [], [], [], []
    for pk in p:
        j = pk - 49.0 * _floor(pk * ns[2] * ns[2])
        x_ = _floor(j * ns[2])
        y_ = _floor(j - 7.0 * x_)
        xs.append(x_ * ns[0] + ns[1])
        ys.append(y_ * ns[0] + ns[1])
        hs.append(1.0 - abs(xs[-1]) - abs(ys[-1]))
    for k in range(4):
        sx = _floor(xs[k]) * 2.0 + 1.0
        sy = _floor(ys[k]) * 2.0 + 1.0
        sh = -(1.0 if hs[k] <= 0.0 else 0.0)
        gx = xs[k] + sx * sh
        gy = ys[k] + sy * sh
        gz = hs[k]
        norm = 1.79284291400159 - 0.85373472095314 * (gx * gx + gy * gy + gz * gz)
        grads.append((gx * norm, gy * norm, gz * norm))
    acc = 0.0
    for k, xk in enumerate((x0, x1, x2, x3)):
        m = max(0.6 - (xk[0] ** 2 + xk[1] ** 2 + xk[2] ** 2), 0.0)
        m = m * m
        acc += m * m * (grads[k][0] * xk[0] + grads[k][1] * xk[1]
                        + grads[k][2] * xk[2])
    return 42.0 * acc


def surface3(cx, cy, cz):
    n = 0.0
    for amp, f in ((1.0, 1.0), (0.5, 2.0), (0.25, 4.0), (0.125, 8.0)):
        n += amp * abs(snoise(cx * f, cy * f, cz * f))
    return n


def height_grid(off, flat, hscale=1.0):
    """Displacement (world units) at the 257x257 grid corners."""
    if flat:
        return [[0.0] * (N + 1) for _ in range(N + 1)]
    rows = []
    for gi in range(N + 1):
        u = gi / N
        col = []
        for gj in range(N + 1):
            v = 1.0 - gj / N
            n = surface3(1.5 * u + off, 1.5 - 1.5 * v, -1.0)
            col.append(HEIGHT_SCALE * hscale * min(n, 1.0))      # RGBA8 clamps
        rows.append(col)
    return rows


# ---- camera ----
def _norm(v):
    d = math.sqrt(sum(a * a for a in v))
    return tuple(a / d for a in v)


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0])


def _dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def make_camera():
    f = _norm((-EYE[0], -EYE[1], -EYE[2]))
    s = _norm(_cross(f, (0.0, 1.0, 0.0)))
    u = _cross(s, f)
    return s, u, f


def to_view(cam, wx, wy, wz):
    s, u, f = cam
    d = (wx - EYE[0], wy - EYE[1], wz - EYE[2])
    return (_dot(d, s), _dot(d, u), -_dot(d, f))     # view space, -z ahead


def to_screen(v):
    """View space (w = -z > 0) -> (sx, sy, depth in 0..1, view distance)."""
    vx, vy, vz = v
    w = -vz
    t = 1.0 / math.tan(math.radians(FOVY) / 2.0)
    # GL: z_clip = -(f+n)/(f-n) * z_eye - 2fn/(f-n), w = -z_eye
    zc = ((FAR + NEAR) / (NEAR - FAR)) * vz + (2.0 * FAR * NEAR) / (NEAR - FAR)
    return ((vx * t / ASPECT / w * 0.5 + 0.5) * W, (vy * t / w * 0.5 + 0.5) * H,
            (zc / w) * 0.5 + 0.5, w)


def clip_near(tri):
    """Sutherland-Hodgman against the near plane; a fan of triangles."""
    out = []
    for k in range(3):
        a, b = tri[k], tri[(k + 1) % 3]
        ina, inb = -a[2] >= NEAR, -b[2] >= NEAR
        if ina:
            out.append(a)
        if ina != inb:
            t = (-NEAR - a[2]) / (b[2] - a[2])
            out.append(tuple(a[i] + t * (b[i] - a[i]) for i in range(3)))
    return [(out[0], out[k], out[k + 1]) for k in range(1, len(out) - 1)]


# ---- mesh and rasteriser ----
def build_triangles(hts, cam):
    """Triangles in glmark2's order: (screen verts, mean view distance)."""
    side = SIZE / N
    pv = {}

    def vert(gi, gj):
        k = gi * (N + 1) + gj
        r = pv.get(k)
        if r is None:
            xm = -SIZE / 2 + gi * side
            ym = SIZE / 2 - gj * side
            # rotate(-90, x): world y = model z (0), world z = -model y
            r = pv[k] = to_view(cam, xm, MODEL_Y + hts[gi][gj], -ym)
        return r

    tris, clipped = [], 0
    for i in range(N):                    # mesh.cpp make_grid: i outer
        for j in range(N):                # j inner
            ul, ll = vert(i, j), vert(i, j + 1)
            ur, lr = vert(i + 1, j), vert(i + 1, j + 1)
            for tri in ((ll, lr, ur), (ll, ur, ul)):     # vertex_index 012 023
                if any(-v[2] < NEAR for v in tri):
                    clipped += 1
                    parts = clip_near(tri)
                else:
                    parts = [tri]
                for part in parts:       # clipped pieces keep the draw slot
                    sv = tuple(to_screen(v) for v in part)
                    tris.append((sv, sum(v[3] for v in sv) / 3.0))
    return tris, clipped


def rasterise(tris):
    """Per triangle: its fragments as (pixel index, depth16), flat arrays."""
    px, zs, start = array("i"), array("i"), array("i")
    for (a, b, c), _ in tris:
        start.append(len(px))
        area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        if abs(area) < 1e-12:
            continue
        inv = 1.0 / area
        x0 = max(int(math.floor(min(a[0], b[0], c[0]) - 0.5)), 0)
        x1 = min(int(math.ceil(max(a[0], b[0], c[0]) - 0.5)), W - 1)
        y0 = max(int(math.floor(min(a[1], b[1], c[1]) - 0.5)), 0)
        y1 = min(int(math.ceil(max(a[1], b[1], c[1]) - 0.5)), H - 1)
        for y in range(y0, y1 + 1):
            fy = y + 0.5
            for x in range(x0, x1 + 1):
                fx = x + 0.5
                w0 = ((b[0] - fx) * (c[1] - fy) - (b[1] - fy) * (c[0] - fx)) * inv
                w1 = ((c[0] - fx) * (a[1] - fy) - (c[1] - fy) * (a[0] - fx)) * inv
                w2 = 1.0 - w0 - w1
                if w0 < 0.0 or w1 < 0.0 or w2 < 0.0:
                    continue
                z = w0 * a[2] + w1 * b[2] + w2 * c[2]
                if z < 0.0 or z > 1.0:        # near/far clip, per fragment
                    continue
                px.append(y * W + x)
                zs.append(int(z * DEPTH_MAX + 0.5))
    start.append(len(px))
    return px, zs, start


def replay(frags, order):
    """(covered, passing, visible) for the triangles drawn in `order`."""
    px, zs, start = frags
    depth = array("i", [DEPTH_MAX + 1]) * (W * H)
    covered = passing = 0
    for t in order:
        for k in range(start[t], start[t + 1]):
            covered += 1
            p, z = px[k], zs[k]
            if z < depth[p]:                  # LESS, depth write on
                depth[p] = z
                passing += 1
    visible = sum(1 for d in depth if d <= DEPTH_MAX)
    return covered, passing, visible


def run(off, flat, hscale):
    cam = make_camera()
    hts = height_grid(off, flat, hscale)
    flat_h = [h for r in hts for h in r]
    print(f"displacement: mean {sum(flat_h) / len(flat_h):.0f}, max {max(flat_h):.0f}"
          f" of {HEIGHT_SCALE * hscale:.0f}; {100.0 * sum(h >= HEIGHT_SCALE * hscale for h in flat_h) / len(flat_h):.0f}% clamped")
    tris, dropped = build_triangles(hts, cam)
    frags = rasterise(tris)
    n = len(tris)
    dist = [t[1] for t in tris]
    glm = list(range(n))
    rnd = list(range(n))
    random.Random(1).shuffle(rnd)
    near_first = sorted(range(n), key=lambda t: dist[t])
    orders = [
        ("glmark2 (i outer, j inner)", glm),
        ("glmark2 reversed", glm[::-1]),
        ("random (seed 1)", rnd),
        ("front to back (sorted)", near_first),
        ("back to front (sorted)", near_first[::-1]),
    ]
    return n, dropped, [(nm, *replay(frags, o)) for nm, o in orders]


def main(argv):
    offs, flat, hscale = (0.0, 0.5), False, 1.0
    for a in argv:
        if a.startswith("--offsets="):
            offs = tuple(float(x) for x in a.split("=", 1)[1].split(","))
        elif a.startswith("--hscale="):
            hscale = float(a.split("=", 1)[1])
        elif a == "--flat":
            flat = True
        else:
            print(__doc__)
            return 2
    full = W * H
    for off in offs:
        n, dropped, res = run(off, flat, hscale)
        print(f"== height map offset {off} ({off / 0.05:.0f} s into the scene)"
              f"{', FLAT plane' if flat else ''}: {n} triangles,"
              f" {dropped} cut by the near plane ==")
        print(f"{'order':32} {'covered':>8} {'passing':>8} {'visible':>8}"
              f" {'cov/480k':>8} {'saved':>7}")
        for nm, cov, pas, vis in res:
            print(f"{nm:32} {cov:8d} {pas:8d} {vis:8d}"
                  f" {cov / full:8.3f} {1 - pas / cov:7.1%}")
        cov, pas = res[0][1], res[0][2]
        print(f"glmark2 order, {FS_INSNS}-instruction shader, ms a frame the"
              f" early depth saves:")
        for ns in NS_PER_INSN:
            late = cov * FS_INSNS * ns * 1e-6
            early = pas * FS_INSNS * ns * 1e-6
            print(f"  {ns} ns/insn: late {late:5.1f} ms -> early {early:5.1f}"
                  f" ms, saves {late - early:5.1f} ms")
        lo = res[3][1:]      # front to back: the ideal order
        hi = res[4][1:]      # back to front: the worst order
        print(f"  bound at 0.7 ns: front-to-back saves"
              f" {(lo[0] - lo[1]) * FS_INSNS * 0.7e-6:5.1f} ms,"
              f" back-to-front {(hi[0] - hi[1]) * FS_INSNS * 0.7e-6:5.1f} ms")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
