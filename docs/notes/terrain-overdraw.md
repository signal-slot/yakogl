<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain: overdraw of the terrain draw

Plan #329, item `terrain-overdraw` (Pi 4; #1209). `v3d-early-z.md` gives the
early-Z gain on terrain's job 3 as `4.3 ms x (overdraw - 1)` and says nobody
has measured the overdraw.

**Result: overdraw 1.030 (mean of six frames, 1.023 to 1.036). Implied
Pi 4 early-Z gain by `v3d-early-z.md`'s formula: 4.3 ms x 0.0302 = 0.13 ms a
frame (0.10 to 0.16), 0.4 % of the 34.8 ms frame. Not worth building for
speed.** Measured on the host with a software rasteriser (below); no board.

## Method

No exact method through a GL driver exists on this host: `ldconfig -p` lists
no libGLES, libEGL, libOSMesa or libGL, and the glmark2 host harness
(`tools/hostharness/glmark2/README.md`, "Limits") renders nothing. So
`overdraw.cpp` is a software rasteriser that re-creates the draw from
glmark2's sources at `22c527cb` (`$CONNECTED_DEPS_DIR/glmark2-src`):

1. Height map: `terrain-noise.frag` ported to float (simplex noise, four
   octaves of `abs(snoise)`), evaluated at each pixel centre of the 256x256
   quad with `vUv = 1.5 * (pos * .5 + .5) + (0.05 * t, 0)` (`update()`:
   `uvOffset = diff * 0.05`; `time` stays 1.0), clamped to 1 and quantised to
   RGBA8 like the render target.
2. Geometry: `make_grid(256, 256, 6000, 6000)` with `grid_conf`'s vertex
   order (ll, lr, ur, ll, ur, ul per cell, cells in i-outer, j-inner order =
   the draw order, 131,072 triangles).
3. `terrain.vert`: `modelViewMatrix` from `scene-terrain.cpp` (camera
   `lookAt(-1200, 800, 1200 -> 0, 0, 0)`, model translate(0, -125, 0) and
   rotate(-90, x), perspective 40 deg, aspect 4:3, near 2, far 4000);
   displacement `375 * texture2D(tDisplacement, uv).x` along the view-space
   normal (the normal map only feeds a varying, not the position), bilinear
   LINEAR / CLAMP_TO_EDGE sampling of the RGBA8 map.
4. Rasterisation: near-plane clip, back-face cull (`GL_CULL_FACE`, BACK, set
   in `canvas-generic.cpp`, as is `glDepthFunc(GL_LEQUAL)`), pixel centres,
   top-left rule, fragments outside the near/far depth range dropped (far
   clips part of the plane: it is 6000 wide), LEQUAL against a 24-bit depth
   buffer cleared to 1.0, draw order as above. Float-depth variant for
   sensitivity.
5. Counts: `rasterized` (fragments made), `covered` (pixels with one),
   `passed` (fragments passing LEQUAL on arrival). `overdraw = rasterized /
   covered`. Early Z skips the `rasterized - passed` fragments. The
   `v3d-early-z.md` formula uses `overdraw - 1` (every non-first fragment
   skipped, the upper bound); `passed_per_covered - 1` is what an exact
   early Z still shades beyond one fragment per pixel. Both are reported;
   the formula's figure is the ceiling.

Limits: float noise and depth are the host's, not the V3D's (a few texels of
height may differ, so read the overdraw to about two digits); window y is
up in the rasteriser, which does not change counts. Not modelled: the tile
binner's own clipping, depth precision of the V3D (24-bit assumed), and the
depth-off fragments of the other jobs.

## Command

```sh
tools/hostharness/terrain-overdraw/run.sh        # frames at t = 0 1 2.5 5 7.5 10 s
```

## Result

`run.sh` output (`overdraw.txt`; the float-depth run is identical to the
digit):

| t (s) | rasterized | covered | overdraw | passed / covered | gain 4.3 x (o - 1) |
|---|---|---|---|---|---|
| 0 | 449,497 | 433,711 | 1.0364 | 1.0000 | 0.157 ms |
| 1 | 446,899 | 432,822 | 1.0325 | 1.0000 | 0.140 ms |
| 2.5 | 443,272 | 431,464 | 1.0274 | 1.0000 | 0.118 ms |
| 5 | 436,650 | 424,169 | 1.0294 | 1.0000 | 0.126 ms |
| 7.5 | 441,576 | 427,845 | 1.0321 | 1.0000 | 0.138 ms |
| 10 | 437,028 | 427,105 | 1.0232 | 1.0000 | 0.100 ms |
| mean | | | **1.0302** | 1.0000 | **0.130 ms** |

Reading it:

* The terrain covers about 430,000 of the 480,000 pixels (the rest is sky);
  only about 3 % of the fragments are second or later fragments of a pixel.
* `passed / covered` is 1.0000: the draw order (i outer, j inner, front to
  back from the camera at (-1200, 800, 1200)) means nearly every later
  fragment fails LEQUAL. Those are exactly the fragments early Z would skip,
  so the formula's `overdraw - 1` is also the exact count here.
* Cost base: `v3d-early-z.md` uses 480,000 px x 9 samples = 4.3 M. With the
  measured ~430,000 covered pixels the base is 3.9 M samples and the gain
  0.12 ms; either way about 0.1 ms.
* The 1.3 to 1.4 overdraw that `v3d-terrain-gpu.md` assumed (1.3 to 1.7 ms) is
  ruled out: the gain sits at the bottom of `v3d-early-z.md`'s 0 to 1.7 ms
  range. Early Z for job 3 stays a legal but ~0.13 ms item; do not build it
  for #1209.

Limits are as in Method: host float noise and 24-bit depth (the float-depth
run gives the same counts), no binner clipping. Read the overdraw to two
digits: 1.03.

## AM62P

Moot by architecture, not measured here. PowerVR does hidden-surface removal per tile,
so opaque overlapped fragments are rejected before shading whatever the draw
order and the shader cost of the hidden ones is not paid; an early-Z change
would have no counterpart there. That is the architecture's behaviour, not
something this item measured.
