<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: how much of glmark2 terrain's fragment work early depth removes, modelled from glmark2's draw order

Plan #355, item terrain-overdraw. `gc-tiled-wire.md` ("What this note does
not settle") leaves open how many of terrain's fragments early depth (W5)
rejects. `gc-tiled-targets.md` (step 5, `:651-657`) puts it at 0 to 14 ms a
frame (up to 19 ms at 0.8 ns an instruction), from a cover times overdraw of
0.8 to 1.2 and "0 to a third of 480 000 fragments", and says that the
range's width depends on the order in which `make_grid` draws the grid. This
note narrows that range with a host model of the draw. The model is
`tools/vivante/terrain_overdraw.py`. Every number below is a host model. No
board measured any of it.

## Answer in short

- **Early depth saves about 2.5 % of the terrain pass's fragments: 1.0 to
  1.3 ms a frame** (0.6 to 0.8 ns an instruction). Over the 10 s scene the
  range is 0.75 to 1.57 ms a frame (fractions 0.019 to 0.030).
- **glmark2's order is already at the floor.** In make_grid's order early
  depth rejects all but about 20 of the fragments that any order could
  reject. Sorting the triangles front to back gains nothing more. Back to
  front, early depth rejects nothing (0 ms). So the order is not what limits
  the saving: terrain's overdraw is. The rasteriser makes about 440 000
  fragments a frame for about 429 000 covered pixels (cover 0.88 to 0.90,
  overdraw 1.02 to 1.03).
- **So W5's terrain row is 0 to 14 (19) ms in `gc-tiled-wire.md:130`, `:942`
  and in `gc-tiled-targets.md:65`, `:652`, and the model puts it at about
  1 ms (at most 1.6 ms).** The range's upper end assumed that a third of the
  fragments are hidden. The model finds a fortieth.
- For session A, the model predicts that the knob's net change on terrain is
  set by the RS copies, not by early depth (see "How session A confirms or
  refutes this").

## Method

`python3 -I tools/vivante/terrain_overdraw.py` (pure Python, no third-party
modules, about 50 s on a host core; `--times` picks other moments). It
rebuilds glmark2's terrain pass from glmark2 `22c527cb` (the commit
`driver/docs/glmark2.md` pins) and rasterises it on the CPU:

1. **Height map.** This is the `SimplexNoiseRenderer` target: 256 x 256
   RGBA8, `terrain-noise.frag` (4 octaves of `abs(snoise)`). The fragment
   shader is ported line for line, and is evaluated at each texel centre with
   `terrain-texture.vert`'s `vUv = 1.5 * uv + uvOffset`,
   `coord = (vUv.x, 1.5 - vUv.y, -time)` and `time = 1` (never updated). The
   result is clamped to [0, 1] and rounded to 8 bits. `uvOffset` scrolls with
   the scene's clock (`scene-terrain.cpp`, `update`: `0.05 * seconds`).
2. **Mesh.** `make_grid(256, 256, 6000, 6000, 0, grid_conf)`
   (`terrain-renderer.cpp:243`, `mesh.cpp:604-641`) loops over cell columns
   `i` (model x, -3000 to 3000) outside and over rows `j` (model y, 3000 to
   -3000) inside. Each cell gives two triangles, `(ll, lr, ur)` then
   `(ll, ur, ul)` (`grid_conf`, `terrain-renderer.cpp:220`). That gives
   131 072 triangles, drawn in this order by one `glDrawArrays`.
3. **Vertex stage.** This is `terrain.vert`. The height map is sampled at the
   vertex's `uv` with LINEAR and CLAMP_TO_EDGE (the `BaseRenderer` defaults,
   `base-renderer.cpp:24-27`, no mipmaps). The vertex is displaced by
   `375 * h` along the view-space image of the normal (0, 0, 1). The model
   (`translate(0, -125, 0) * rotate(-90, x)`) makes that world +y. So a
   vertex at model (x, y) lands at world (x, -125 + 375 h, -y).
4. **Matrices.** `lookAt((-1200, 800, 1200), origin, +y)` and
   `perspective(40, 800/600, 2, 4000)` are libmatrix's (`mat.cc:128-169`).
5. **Raster.** The target is 800 x 600 with a `DEPTH_COMPONENT16`
   renderbuffer (`base-renderer.cpp:146-150`), cleared to 1.0. The depth test
   is `GL_LEQUAL` and back faces are culled with CCW front faces
   (`canvas-generic.cpp:77-80`). Triangles are clipped against the near and
   far planes. The x and y planes are a guard band: the bounding box is
   clamped to the screen, and every vertex has w > 0. Samples sit at pixel
   centres, with a tie rule that gives a sample on a shared edge to exactly
   one of the two triangles. Window z is affine in screen space and rounded
   to 16 bits.
6. **Counts per order.** *covered* is every fragment the rasteriser makes:
   what the 148-instruction fragment shader runs on with late depth. That is
   the GC7000UL on a linear target today (`gc-slow-terrain.md:109-112`).
   *passed* is the fragments that pass `LEQUAL` against the depth the earlier
   triangles wrote: what the shader runs on with an ideal early depth.
   *pixels* is the distinct pixels covered, the floor for *passed* in any
   order. The saving is `(covered - passed) / covered`, and in ms it is
   `(covered - passed) * 148 * 0.6 to 0.8 ns` (`gc-slow-terrain.md:15-19`,
   `:235-237`, "The GPU calibration").
7. **Orders.** The orders are glmark2's (make_grid's), and as bounds the
   triangles sorted by the view-space distance of their centroid, front to
   back and back to front. "glmark2 float" is make_grid's order with depth
   compared unrounded. It shows what the 16-bit ties cost.

A depth picture of the modelled frame (a scratch dump, not committed) shows
the scene as glmark2 draws it: rolling hills to the horizon, a sky band in
the top 7 % of the target, and the terrain's top edge at row 559 of 600.

## Numbers

From `python3 -I tools/vivante/terrain_overdraw.py` (default `--times`, every
second of the 10 s scene). Each row is one modelled frame. The ms columns are
the shader time early depth saves, at 148 instructions a fragment.

| t s | covered | passed (glmark2) | pixels | cover | overdraw | saved | ms @0.6 | ms @0.8 |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 445 449 | 432 495 | 432 465 | 0.901 | 1.030 | 0.029 | 1.15 | 1.53 |
| 1 | 442 993 | 432 429 | 432 401 | 0.901 | 1.024 | 0.024 | 0.94 | 1.25 |
| 2 | 440 730 | 431 940 | 431 919 | 0.900 | 1.020 | 0.020 | 0.78 | 1.04 |
| 3 | 438 752 | 430 329 | 430 303 | 0.896 | 1.020 | 0.019 | 0.75 | 1.00 |
| 4 | 435 951 | 427 382 | 427 360 | 0.890 | 1.020 | 0.020 | 0.76 | 1.01 |
| 5 | 434 839 | 424 267 | 424 246 | 0.884 | 1.025 | 0.024 | 0.94 | 1.25 |
| 6 | 437 140 | 424 903 | 424 888 | 0.885 | 1.029 | 0.028 | 1.09 | 1.45 |
| 7 | 439 281 | 426 960 | 426 951 | 0.889 | 1.029 | 0.028 | 1.09 | 1.46 |
| 8 | 442 627 | 429 352 | 429 339 | 0.894 | 1.031 | 0.030 | 1.18 | 1.57 |
| 9 | 443 556 | 430 335 | 430 315 | 0.896 | 1.031 | 0.030 | 1.17 | 1.57 |
| 10 | 439 541 | 427 699 | 427 689 | 0.891 | 1.028 | 0.027 | 1.05 | 1.40 |

Mean over the eleven frames:

| order | covered | passed | saved fraction | saved ms a frame |
|---|---:|---:|---:|---:|
| glmark2 (make_grid) | 440 078 | 428 917 | 0.025 | 1.0 to 1.3 |
| front to back (bound) | 440 078 | 428 914 | 0.025 | 1.0 to 1.3 |
| back to front (bound) | 440 078 | 440 078 | 0.000 | 0 |
| glmark2, unrounded depth | 440 078 | 428 902 | 0.025 | 1.0 to 1.3 |
| per-pixel floor | 440 078 | 428 898 | 0.025 | 1.0 to 1.3 |

Further results:

- **Why make_grid's order reaches the floor.** The camera looks along
  (+x, -y, -z), and the columns are drawn in rising x. A hill hides terrain
  that is further along the view ray, so it hides terrain at larger x, in a
  later column. In the glmark2 order, 0.6 % of the rejected fragments have
  their occluder drawn less than one column (512 triangles) before them. The
  far-to-near order inside a column does not matter, because one cell
  (23.4 units) seldom hides another cell in the same column.
- **Why the overdraw is so small.** Two things keep it small. The camera
  looks down at about 25 degrees from 800 units, and the hills rise 375
  units over noise features about a thousand units wide. And culling removes
  the far slopes of the hills, the faces that would otherwise be drawn twice
  behind a ridge. What is hidden is the thin band of the next front-facing
  slope behind each ridge.
- **16-bit depth costs nothing here.** With near 2 and far 4000 the whole
  terrain lies in about 150 of the 65 536 depth values (65 388 to 65 534 at
  t = 0). So `LEQUAL` ties are common in principle. In practice, unrounded
  depth rejects only about 15 fragments a frame more than 16-bit depth.
- **The terrain pass's fragment work as a whole.** 440 078 fragments x 148
  instructions comes to 39.1 ms at 0.6 ns and 52.1 ms at 0.8 ns.
  `gc-slow-terrain.md:86` gives 42.6 ms at full cover, and `:105-107` gives
  a cover times overdraw of 0.8 to 1.2. The model's 0.917 is inside that
  band. The cover is higher than that note's flat-plane 0.8 because the
  hills raise the horizon.

## Assumptions and approximations

1. **Ideal early depth.** Every fragment is tested at the PE's own 16-bit
   precision against all the depth the earlier triangles wrote. A GPU that
   cannot see depth writes still in its pipe rejects fewer. Here this barely
   matters, because 99.4 % of the rejections have an occluder at least 512
   triangles older. The model is therefore an upper bound for make_grid's
   order, and a close one. Any hierarchical depth or a coarser test would
   only lower it.
2. **Only shader time is saved.** A rejected fragment is still rasterised
   and keeps the 3.9 ns floor a pixel (`gc-slow-terrain.md:76-79`). If the
   PE's share of that floor were saved too, it would add at most
   11 000 x 3.9 ns = 0.04 ms.
3. **Calibration.** The ms use 148 instructions a fragment and 0.6 to 0.8 ns
   an instruction a pixel. Both come from `gc-slow-terrain.md`, which
   calibrated them on the Verdin i.MX8MP, the same GC7000UL as the X2. The
   texture loads are inside that per-instruction figure. A different
   instruction count scales the ms in proportion, and the fraction does not
   move.
4. **Height map.** The noise is evaluated in double precision, where the
   GPU uses fp32 or mediump. It is rounded to 8 bits as the RGBA8 target
   stores it. The vertex fetch is an exact bilinear filter, where the GC
   uses fixed-point weights. These move a height by well under one unit of
   the 375, and the overdraw by much less than its frame-to-frame spread
   (1.020 to 1.031).
5. **Time.** glmark2 runs terrain for 10 s, at about 4 frames a second on
   the board. The mean over eleven moments one second apart stands for that
   run. The per-frame spread (0.75 to 1.57 ms) is the range to compare with
   single frames.
6. **The terrain pass only.** The other passes gain nothing from early
   depth. tilt_v's depth-tested full-screen quad covers each pixel once
   (`gc-tiled-targets.md:657-658`). The overlay, blur and copy passes do not
   test depth.
7. **Rasterisation rules.** Pixel-centre samples and one owner for a shared
   edge are the GL rules. The GC's own tie rule can differ only on edge
   samples, and it changes neither the fraction nor the order result.

## How session A confirms or refutes this

Session A (`gc-tiled-wire.md:1007`, "Session A: the run sheet") runs terrain from one commit with
`gc_tiled_rt` off and then on. The knob switches W4 (the tiled shadow with
its RS tile-in and copy-backs) and W5 (early depth) together. So the session
reads only their sum. Let D be terrain's ms a frame with the knob on minus
the ms with it off.

- **The prediction.** `gc-slow-host-counts.md` (`:497-539`, the predicted
  change and "Reading rule for session A, terrain") puts the RS cost at +0.7 to +3 ms (three 1.92 MB RS jobs at
  an unmeasured 2 to 8 GB/s). With early depth at -0.75 to -1.6 ms, D is
  expected between about -0.9 and +2.3 ms. That is "does nothing" or
  "hurts" in that note's rule, inside the spread of repeated runs. The
  likely sign is positive.
- **Refuted** if D <= -3 ms, the reading rule's "Helps". That is more than
  the model allows even with a free RS. Then either the GC's early depth
  saves more than the shader time of the rejected fragments, or the
  calibration (148 instructions, 0.6 to 0.8 ns) is wrong for this pass, or
  something else in the knob helps (fewer CPU tiled copies, a changed kick).
  The PERF pages tell those apart: page 4's tiled copies and fallbacks, and
  page 1's kicks and waits (`gc-slow-host-counts.md`, the terrain table's
  first rows).
- **Consistent, not confirmed** if D is in -3 to +3 ms. A single off/on pair
  cannot isolate a 1 ms gain in a frame of about 250 ms. Write "no net
  effect", not "early depth works" or "early depth does not work".
- **What would confirm it.** Early depth's own share has to be measured
  apart from the RS cost. Either of these does it:
  1. the RS rate, which session A measures for W4 ("What this note does not
     settle", first bullet, `gc-tiled-wire.md:1214`). D minus three RS jobs at that rate is then the
     early-depth gain, to compare with -0.75 to -1.6 ms;
  2. a draw with no tiled-target cost difference, where only early depth
     changes. An example is a gl_test case that draws the same grid with the
     knob on, once in make_grid's order and once in reverse. The model
     predicts the reverse order shades 2.5 % more fragments (back to front
     rejects nothing). That case does not exist (see the report of item
     terrain-overdraw).
- **What a board number does to this note.** If D or the measured
  early-depth share falls outside the bands above, the board row with its
  commit and record goes into `gc-tiled-wire.md` and `gc-slow-terrain.md`
  (owned by the session A and W5 items). This note then gets a line that
  names the assumption the number broke.

## What this note does not settle

- The GC7000UL's early-depth granularity: per pixel, per quad, or per tile,
  and whether it sees in-flight depth writes. The model's 99.4 % "occluder at
  least one column older" makes the answer matter little for terrain.
- The RS rate, which sets the sign of session A's D.
- Shadow and refract. Their early-depth rows (0 to 1 ms and 0 to 2 ms,
  `gc-tiled-targets.md:659-661`) come from other scenes, which this model
  does not draw.
