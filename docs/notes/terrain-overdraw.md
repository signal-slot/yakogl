<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain: overdraw of the terrain draw

Plan #329, item `terrain-overdraw` (Pi 4; #1209). `v3d-early-z.md` gives the
early-Z gain on terrain's job 3 as `4.3 ms x (overdraw - 1)` and says nobody
has measured the overdraw.

**Status: the measurement was written but not run.** The tool and method are
in `tools/hostharness/terrain-overdraw/`; the sandbox that ran this item
refused to execute any program it had built (every run of the compiled binary
or of `run.sh` came back "requires approval"; compiling with g++ worked, with
no warnings). So this note has **no overdraw figure, no mean and no gain in
ms**, and none is invented here. Run the command below and fill the table.

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
   early Z still shades beyond one fragment per pixel, so the exact gain
   is `4.3 ms x (overdraw - passed_per_covered) / 1`-scaled by the same
   rule: report both and take the formula's figure as the ceiling.

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

Not measured (above). Fill in from `$TMPDIR/terrain-overdraw/overdraw.txt`:

| t (s) | rasterized | covered | overdraw | passed / covered |
|---|---|---|---|---|
| 0 | | | | |
| 1 | | | | |
| 2.5 | | | | |
| 5 | | | | |
| 7.5 | | | | |
| 10 | | | | |
| mean | | | | |

Implied Pi 4 gain: `4.3 ms x (mean overdraw - 1)`. For scale, `v3d-early-z.md`
gives 0 at overdraw 1.0, 1.3 ms at 1.3 and 1.7 ms at 1.4.

## AM62P

Not moot by measurement here. PowerVR does hidden-surface removal per tile,
so opaque overlapped fragments are rejected before shading whatever the draw
order and the shader cost of the hidden ones is not paid; an early-Z change
would have no counterpart there. That is the architecture's behaviour, not
something this item measured.
