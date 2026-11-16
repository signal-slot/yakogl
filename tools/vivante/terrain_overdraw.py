#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""terrain_overdraw.py - how many of glmark2 terrain's fragments early depth rejects.

A host model for docs/notes/gc-terrain-overdraw.md. It rebuilds glmark2's
terrain pass (glmark2 22c527cb, src/scene-terrain.cpp,
src/scene-terrain/terrain-renderer.cpp, Mesh::make_grid in src/mesh.cpp,
data/shaders/terrain.vert, terrain-noise.frag, terrain-texture.vert):

  - the 256 x 256 cell grid of 6000 x 6000 units, 2 triangles a cell, in
    make_grid's order (cell column i over x outer, row j inner, triangles
    (ll, lr, ur) then (ll, ur, ul));
  - the height map: the 256 x 256 RGBA8 target of 4 octaves of simplex
    noise, sampled LINEAR and CLAMP_TO_EDGE at each vertex's uv, displacing
    the vertex by 375 units along the plane's normal;
  - the model (translate 0,-125,0; rotate -90 about x), the camera
    (lookAt (-1200, 800, 1200) -> origin) and the projection (40 degrees,
    800/600, near 2, far 4000);
  - the 800 x 600 target with a DEPTH_COMPONENT16 renderbuffer, LEQUAL,
    back faces culled, cleared to 1.0 before the draw.

It rasterises every triangle (near and far clipping, the top-left rule,
pixel centres, depth affine in window space, quantised to 16 bits) and
counts, per draw order:

  covered    every fragment the rasteriser makes: what the fragment shader
             runs on with late depth (the GC7000UL on a linear target today)
  passed     the fragments that pass LEQUAL against the depth written by the
             triangles before them: what it runs on with an ideal early depth
  pixels     distinct pixels covered: the floor no order goes below

and prints the fraction early depth saves, (covered - passed) / covered, and
with gc-slow-terrain.md's calibration (0.6 to 0.8 ns an instruction a pixel,
148 instructions) the expected ms a frame it saves.

Orders: glmark2 (make_grid's), and as bounds the triangles sorted by
view-space distance of their centroid, front to back and back to front.
These three compare 16-bit depth. "glmark2 float" is make_grid's order with
the depth compared unquantised: what the 16-bit ties cost (with near 2 and
far 4000 the whole terrain spans about 150 of the 65 536 depth values).

The noise scrolls with glmark2's clock (uvOffset = 0.05 * seconds), so the
height map, and with it the hills, is evaluated at each second of the 10 s
scene (--times).

Usage: python3 -I tools/vivante/terrain_overdraw.py [--times 0,5,10]
Pure Python, no third-party modules. About a minute for the default eleven
times, five seconds a time.
"""

import argparse
import math
import sys
from array import array

# --- glmark2's constants -------------------------------------------------

W, H = 800, 600  # terrain_renderer: setup_offscreen(screen_res, true)
GRID = 256  # make_grid(256, 256, 6000, 6000, 0, grid_conf)
SIZE = 6000.0
MAP = 256  # height map 256 x 256 (map_res)
UV_SCALE = 1.5  # SimplexNoiseRenderer::uv_scale_
NOISE_TIME = 1.0  # noise_program "time", never updated
DISP_SCALE = 375.0  # uDisplacementScale
DISP_BIAS = 0.0  # uDisplacementBias
EYE = (-1200.0, 800.0, 1200.0)
CENTER = (0.0, 0.0, 0.0)
UP = (0.0, 1.0, 0.0)
FOVY, NEAR, FAR = 40.0, 2.0, 4000.0
MODEL_Y = -125.0  # model.translate(0, -125, 0)

# --- gc-slow-terrain.md's calibration ------------------------------------

FS_INSTR = 148  # terrain.frag on the GC7000UL ("The 52 kicks")
NS_PER_INSTR = (0.6, 0.8)  # "The GPU calibration": 0.6, upper 0.8

# --- simplex noise, data/shaders/terrain-noise.frag ----------------------


def _floor(x):
    return math.floor(x)


def _mod(x, y):
    return x - y * math.floor(x / y)


def _permute(x):
    return _mod(((x * 34.0) + 1.0) * x, 289.0)


def _step(edge, x):
    return 0.0 if x < edge else 1.0


def snoise(vx, vy, vz):
    cx, cy = 1.0 / 6.0, 1.0 / 3.0
    s = (vx + vy + vz) * cy
    ix, iy, iz = _floor(vx + s), _floor(vy + s), _floor(vz + s)
    t = (ix + iy + iz) * cx
    x0 = (vx - ix + t, vy - iy + t, vz - iz + t)
    # g = step(x0.yzx, x0.xyz); l = 1 - g
    g = (_step(x0[1], x0[0]), _step(x0[2], x0[1]), _step(x0[0], x0[2]))
    l = (1.0 - g[0], 1.0 - g[1], 1.0 - g[2])
    # i1 = min(g.xyz, l.zxy); i2 = max(g.xyz, l.zxy)
    lz = (l[2], l[0], l[1])
    i1 = (min(g[0], lz[0]), min(g[1], lz[1]), min(g[2], lz[2]))
    i2 = (max(g[0], lz[0]), max(g[1], lz[1]), max(g[2], lz[2]))
    x1 = tuple(x0[k] - i1[k] + cx for k in range(3))
    x2 = tuple(x0[k] - i2[k] + 2.0 * cx for k in range(3))
    x3 = tuple(x0[k] - 1.0 + 3.0 * cx for k in range(3))
    ix, iy, iz = _mod(ix, 289.0), _mod(iy, 289.0), _mod(iz, 289.0)
    zs = (0.0, i1[2], i2[2], 1.0)
    ys = (0.0, i1[1], i2[1], 1.0)
    xs = (0.0, i1[0], i2[0], 1.0)
    p = [_permute(_permute(_permute(iz + zs[k]) + iy + ys[k]) + ix + xs[k]) for k in range(4)]
    n_ = 1.0 / 7.0
    # ns = n_ * D.wyz - D.xzx, D = (0, 0.5, 1, 2)
    ns = (n_ * 2.0 - 0.0, n_ * 0.5 - 1.0, n_ * 1.0 - 0.0)
    grads = []
    for k in range(4):
        j = p[k] - 49.0 * _floor(p[k] * ns[2] * ns[2])
        x_ = _floor(j * ns[2])
        y_ = _floor(j - 7.0 * x_)
        gx = x_ * ns[0] + ns[1]
        gy = y_ * ns[0] + ns[1]
        h = 1.0 - abs(gx) - abs(gy)
        sh = -_step(h, 0.0)
        # a = b + s * sh, b = (x, y), s = floor(b) * 2 + 1
        ax = gx + (_floor(gx) * 2.0 + 1.0) * sh
        ay = gy + (_floor(gy) * 2.0 + 1.0) * sh
        grads.append((ax, ay, h))
    xk = (x0, x1, x2, x3)
    total = 0.0
    for k in range(4):
        gx, gy, gz = grads[k]
        norm = 1.79284291400159 - 0.85373472095314 * (gx * gx + gy * gy + gz * gz)
        gx, gy, gz = gx * norm, gy * norm, gz * norm
        x = xk[k]
        m = max(0.6 - (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]), 0.0)
        m = m * m
        total += m * m * (gx * x[0] + gy * x[1] + gz * x[2])
    return 42.0 * total


def surface3(x, y, z):
    return (
        1.0 * abs(snoise(x, y, z))
        + 0.5 * abs(snoise(x * 2.0, y * 2.0, z * 2.0))
        + 0.25 * abs(snoise(x * 4.0, y * 4.0, z * 4.0))
        + 0.125 * abs(snoise(x * 8.0, y * 8.0, z * 8.0))
    )


def height_map(seconds):
    """The RGBA8 height map's red channel, [ty][tx], ty = 0 the bottom row."""
    off = 0.05 * seconds  # SceneTerrain::update: uvOffset = (diff * 0.05, 0)
    rows = []
    for ty in range(MAP):
        row = []
        vy = UV_SCALE * ((ty + 0.5) / MAP)
        for tx in range(MAP):
            vx = UV_SCALE * ((tx + 0.5) / MAP) + off
            n = surface3(vx, UV_SCALE - vy, -NOISE_TIME)
            n = min(max(n, 0.0), 1.0)
            row.append(round(n * 255.0) / 255.0)
        rows.append(row)
    return rows


def sample_linear_clamp(tex, u, v):
    s = u * MAP - 0.5
    t = v * MAP - 0.5
    x0 = math.floor(s)
    y0 = math.floor(t)
    fx, fy = s - x0, t - y0

    def texel(x, y):
        return tex[min(max(y, 0), MAP - 1)][min(max(x, 0), MAP - 1)]

    a = texel(x0, y0) * (1 - fx) + texel(x0 + 1, y0) * fx
    b = texel(x0, y0 + 1) * (1 - fx) + texel(x0 + 1, y0 + 1) * fx
    return a * (1 - fy) + b * fy


# --- matrices (libmatrix, column vectors) --------------------------------


def look_at():
    f = [CENTER[k] - EYE[k] for k in range(3)]
    fl = math.sqrt(sum(c * c for c in f))
    f = [c / fl for c in f]
    s = [f[1] * UP[2] - f[2] * UP[1], f[2] * UP[0] - f[0] * UP[2], f[0] * UP[1] - f[1] * UP[0]]
    u = [s[1] * f[2] - s[2] * f[1], s[2] * f[0] - s[0] * f[2], s[0] * f[1] - s[1] * f[0]]
    sl = math.sqrt(sum(c * c for c in s))
    ul = math.sqrt(sum(c * c for c in u))
    s = [c / sl for c in s]
    u = [c / ul for c in u]
    rows = [s, u, [-c for c in f]]
    return [[rows[r][0], rows[r][1], rows[r][2], -sum(rows[r][k] * EYE[k] for k in range(3))] for r in range(3)] + [
        [0.0, 0.0, 0.0, 1.0]
    ]


def perspective():
    f = 1.0 / math.tan(math.radians(FOVY) / 2.0)
    depth = NEAR - FAR
    return [
        [f / (W / H), 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (FAR + NEAR) / depth, 2.0 * FAR * NEAR / depth],
        [0.0, 0.0, -1.0, 0.0],
    ]


def mat_mul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]


def transform(m, p):
    return tuple(m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3] for r in range(4))


# --- the mesh ------------------------------------------------------------


def vertices(tex):
    """World-space position and view distance of the (GRID+1)^2 lattice.

    Index [i][j]: i over model x (-3000 .. 3000), j over model y (3000 ..
    -3000), make_grid's a = (-w/2 + i*side, h/2 - j*side). The model maps
    (x, y, 0) to (x, -125, -y); terrain.vert displaces along the view-space
    image of the normal (0, 0, 1), which is world +y.
    """
    side = SIZE / GRID
    world = []
    for i in range(GRID + 1):
        col = []
        x = -SIZE / 2 + i * side
        for j in range(GRID + 1):
            y = SIZE / 2 - j * side
            u = (x + SIZE / 2) / SIZE  # grid_conf: uv_ul.x = i / n_x
            v = (y + SIZE / 2) / SIZE  # grid_conf: uv_ul.y = 1 - j / n_y
            h = DISP_SCALE * sample_linear_clamp(tex, u, v) + DISP_BIAS
            col.append((x, MODEL_Y + h, -y))
        world.append(col)
    return world


def triangles():
    """make_grid's order: (i, j) cells, i outer; grid_conf's two triangles."""
    for i in range(GRID):
        for j in range(GRID):
            ul, ll, ur, lr = (i, j), (i, j + 1), (i + 1, j), (i + 1, j + 1)
            yield (ll, lr, ur)
            yield (ll, ur, ul)


def clip_near_far(poly):
    """Sutherland-Hodgman against -w <= z <= w (x and y: a guard band)."""
    for sign in (1.0, -1.0):  # z <= w, then -z <= w
        out = []
        n = len(poly)
        for k in range(n):
            a, b = poly[k], poly[(k + 1) % n]
            da = a[3] - sign * a[2]
            db = b[3] - sign * b[2]
            if da >= 0:
                out.append(a)
            if (da >= 0) != (db >= 0):
                t = da / (da - db)
                out.append(tuple(a[c] + t * (b[c] - a[c]) for c in range(4)))
        poly = out
        if len(poly) < 3:
            return []
    return poly


def raster(tri, pix, dep):
    """Append the fragments of a window-space triangle (front-facing, CCW)."""
    (x0, y0, z0), (x1, y1, z1), (x2, y2, z2) = tri
    area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
    if area <= 0.0:
        return  # glCullFace(GL_BACK), front face CCW
    minx = max(int(math.floor(min(x0, x1, x2) - 0.5)), 0)
    maxx = min(int(math.ceil(max(x0, x1, x2) - 0.5)), W - 1)
    miny = max(int(math.floor(min(y0, y1, y2) - 0.5)), 0)
    maxy = min(int(math.ceil(max(y0, y1, y2) - 0.5)), H - 1)
    if minx > maxx or miny > maxy:
        return
    inv = 1.0 / area
    # Edge k is opposite vertex k: e(x, y) = a*x + b*y + c, > 0 inside.
    edges = []
    for (ax, ay), (bx, by) in (((x1, y1), (x2, y2)), ((x2, y2), (x0, y0)), ((x0, y0), (x1, y1))):
        a = ay - by
        b = bx - ax
        c = ax * by - ay * bx
        # A tie rule: a sample exactly on an edge belongs to the triangle
        # whose inward normal (a, b) points right, or down if horizontal.
        # The neighbour across a shared edge has (-a, -b), so exactly one of
        # the two owns it: no sample twice, none lost, as the top-left rule.
        owns = a > 0.0 or (a == 0.0 and b < 0.0)
        edges.append((a, b, c, owns))
    (a0, b0, c0, o0), (a1, b1, c1, o1), (a2, b2, c2, o2) = edges
    for py in range(miny, maxy + 1):
        cy = py + 0.5
        row = py * W
        for px in range(minx, maxx + 1):
            cx = px + 0.5
            w0 = a0 * cx + b0 * cy + c0
            if w0 < 0.0 or (w0 == 0.0 and not o0):
                continue
            w1 = a1 * cx + b1 * cy + c1
            if w1 < 0.0 or (w1 == 0.0 and not o1):
                continue
            w2 = a2 * cx + b2 * cy + c2
            if w2 < 0.0 or (w2 == 0.0 and not o2):
                continue
            pix.append(row + px)
            dep.append((w0 * z0 + w1 * z1 + w2 * z2) * inv)


def build(seconds):
    """Per-triangle (view distance, draw index, pixels, window z), make_grid's order."""
    tex = height_map(seconds)
    world = vertices(tex)
    view = look_at()
    mvp = mat_mul(perspective(), view)
    clip = [[transform(mvp, p) for p in col] for col in world]
    dist = [[-transform(view, p)[2] for p in col] for col in world]
    tris = []
    for index, t in enumerate(triangles()):
        poly = clip_near_far([clip[i][j] for (i, j) in t])
        if not poly:
            continue
        pix = array("i")
        dep = array("d")
        win = [
            ((c[0] / c[3] * 0.5 + 0.5) * W, (c[1] / c[3] * 0.5 + 0.5) * H, c[2] / c[3] * 0.5 + 0.5)
            for c in poly
        ]
        for k in range(1, len(win) - 1):
            raster((win[0], win[k], win[k + 1]), pix, dep)
        if pix:
            centroid = sum(dist[i][j] for (i, j) in t) / 3.0
            tris.append((centroid, index, pix, dep))
    return tris


def run_order(tris, d16=True):
    """Shaded fragments with late depth (covered) and with early depth (passed).

    Also counts the rejected fragments whose occluder (the last triangle to
    write the pixel's depth) was drawn less than one grid column (2 x 256
    triangles) before them: the rejections an early depth that cannot see
    the depth writes still in flight might miss.

    d16: the window z quantised to the DEPTH_COMPONENT16 renderbuffer's
    16 bits (round to nearest), as the PE stores and compares it; otherwise
    compared as a double, to show what the ties of 16 bits cost.
    """
    depth = array("d", [65535.0 if d16 else 1.0]) * (W * H)
    writer = array("i", [0]) * (W * H)
    covered = passed = near = 0
    column = 2 * GRID
    for _, index, pix, dep in tris:
        covered += len(pix)
        for k in range(len(pix)):
            p = pix[k]
            d = dep[k]
            if d16:
                d = float(min(max(int(d * 65535.0 + 0.5), 0), 65535))
            if d <= depth[p]:  # GL_LEQUAL
                depth[p] = d
                writer[p] = index
                passed += 1
            elif abs(index - writer[p]) < column:
                near += 1
    return covered, passed, near


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--times",
        default="0,1,2,3,4,5,6,7,8,9,10",
        help="seconds into the 10 s scene at which to model a frame (default: every second, 0 to 10)",
    )
    args = ap.parse_args(argv)
    times = [float(t) for t in args.times.split(",") if t.strip()]

    print("terrain_overdraw: glmark2 22c527cb terrain pass, 800x600, depth16 LEQUAL, back faces culled")
    print(f"fragment shader {FS_INSTR} instructions; ns an instruction a pixel {NS_PER_INSTR[0]} to {NS_PER_INSTR[1]}")
    print()
    hdr = f"{'t s':>5} {'order':<14} {'covered':>8} {'passed':>8} {'pixels':>7} {'cover':>6} {'overdraw':>8} {'saved':>6} {'ms@0.6':>7} {'ms@0.8':>7}"
    print(hdr)
    sums = {}
    for t in times:
        tris = build(t)
        pixels = len({p for _, _, pix, _ in tris for p in pix})
        orders = (
            ("glmark2", tris),
            ("front-to-back", sorted(tris, key=lambda r: r[0])),
            ("back-to-front", sorted(tris, key=lambda r: -r[0])),
            ("glmark2 float", tris),
        )
        for name, seq in orders:
            covered, passed, near = run_order(seq, d16=name != "glmark2 float")
            saved = covered - passed
            frac = saved / covered
            ms = [saved * FS_INSTR * ns * 1e-6 for ns in NS_PER_INSTR]
            print(
                f"{t:5.1f} {name:<14} {covered:8d} {passed:8d} {pixels:7d} "
                f"{pixels / (W * H):6.3f} {covered / pixels:8.3f} {frac:6.3f} {ms[0]:7.2f} {ms[1]:7.2f}"
            )
            s = sums.setdefault(name, [0, 0, 0, 0])
            s[0] += covered
            s[1] += passed
            s[2] += pixels
            s[3] += near
        sys.stdout.flush()
    print()
    print(f"mean over t = {','.join(f'{t:g}' for t in times)} s:")
    for name, (covered, passed, pixels, _) in sums.items():
        n = len(times)
        saved = (covered - passed) / n
        frac = (covered - passed) / covered
        ms = [saved * FS_INSTR * ns * 1e-6 for ns in NS_PER_INSTR]
        print(
            f"  {name:<14} covered {covered / n:9.0f}  passed {passed / n:9.0f}  "
            f"saved fraction {frac:.3f}  saved {ms[0]:.1f} to {ms[1]:.1f} ms a frame"
        )
    n = len(times)
    pixels = sums["glmark2"][2] / n
    covered = sums["glmark2"][0] / n
    floor = (covered - pixels) / covered
    print(
        f"  {'per-pixel floor':<14} covered {covered:9.0f}  passed {pixels:9.0f}  "
        f"saved fraction {floor:.3f}  (one shaded fragment a covered pixel)"
    )
    covered, passed, _, near = sums["glmark2"]
    print(
        f"  glmark2 order: {near / max(covered - passed, 1):.3f} of the rejected fragments have their "
        f"occluder drawn less than {2 * GRID} triangles (one grid column) before them"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
