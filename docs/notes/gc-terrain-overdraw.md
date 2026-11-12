<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: how many of terrain's fragments early depth rejects (host model)

Plan #355, item terrain-overdraw (issues #1204, #1106; ROADMAP.md stage 2,
"Slow scenes"). Host only, no board number, no `src/` change. It answers the
open point of `gc-tiled-wire.md` ("What this note does not settle": how many
of terrain's fragments early depth rejects) and of `gc-tiled-targets.md`
step 5 ("Gain": terrain 0 to 14 ms, up to 19 at 0.8 ns, "the widest range in
this note"). glmark2 is the pinned `22c527cb` (`$GLMARK2_SRC`). The model is
`tools/vivante/terrain_overdraw.py`; run it with
`python3 -I tools/vivante/terrain_overdraw.py` (a few seconds a height offset; plain Python).

## Answer in short

- **Early depth rejects 6.7 to 7.1 % of terrain's covered fragments, and
  that is also the most any draw order could reject** (the covered fragments
  that never become visible). The saving is **2.8 to 3.8 ms a frame** (2.5 to 3.3 ms
  at 0.6 ns an instruction, 3.7 to 3.8 ms at 0.8 ns), at the 148-instruction
  terrain fragment shader. The earlier range of 0 to 14 (19) ms is
  too wide at the top: its top assumed up to a third of the fragments are
  hidden.
- **glmark2's draw order is already as good as a sorted front-to-back
  draw**: it passes 85 fragments more than the ideal in 432 659 (offset 0).
  The grid is built with the x column outer and the y row inner
  (`mesh.cpp:604-640`), and the camera looks toward +x and -z, so columns
  arrive near to far. Back to front would save only 0.5 %, 0.2 to 0.3 ms.
  So the order does not set the figure; the *hills' height* does.
- **The covered area is 0.94 to 0.97 of 800x600, not 0.8**: the hills lift
  the mesh up the screen. A flat plane covers 0.797 (this matches
  `gc-slow-terrain.md`'s "cover about 0.8"). The terrain pass therefore
  shades 452 000 to 464 000 fragments, 41 to 55 ms at 0.6 to 0.8 ns an
  instruction, against that note's 49 to 66 ms.
- **What the number rests on**: the height map's shape. At half the hill
  height the saving is 0.2 ms, at twice 20 ms, at four times 57 ms
  ("Sensitivity"). The model transcribes the scene's own noise shader, so
  the height is not a free parameter, but the board's pair is the check.

## Method

`terrain_overdraw.py` builds the mesh exactly as glmark2 does and rasterises
it once, then replays the fragments in several triangle orders.

| piece | source | in the model |
|---|---|---|
| mesh | `terrain-renderer.cpp:243` `make_grid(256, 256, 6000, 6000, 0, grid_conf)` | 65 536 cells, 131 072 triangles, 6 vertices a cell as (ll, lr, ur), (ll, ur, ul) (`grid_conf`, `vertex_index {0,1,2,0,2,3}`) |
| draw order | `mesh.cpp:604-640`: `for i (x) { for j (y) }` | one draw call, triangles in that order |
| camera | `scene-terrain.cpp:270-292`: perspective 40 degrees, 800/600, near 2, far 4000; `lookAt(-1200, 800, 1200 -> 0)`; model `translate(0, -125, 0) rotate(-90, x)` | the same, in view space; the near plane is clipped exactly, the far plane per fragment |
| height | `terrain.vert`: the vertex moves `375 * height.x` along the world-up normal (`normalMatrix` of a rotation, `w = 1` adds no translation); `terrain-noise.frag`: `abs(snoise)` over 4 octaves at `(1.5 u + off, 1.5 (1 - v), -1)`, written to an RGBA8 map, so it clamps to 1; `off = 0.05 * seconds` (`scene-terrain.cpp:326`) | the noise shader transcribed (`snoise` range -0.97 to 0.99, mean `abs` 0.31, checked); displacement mean 225 to 227, 5 to 6 % of the vertices clamped at 375; offsets 0 and 0.5 (the scene's first and last second) |
| depth | the terrain target's depth16 renderbuffer, `LESS`, write on (`base-renderer.cpp:74-75`) | 16-bit quantised depth, ties fail |
| rasteriser | | pixel centres, exact per pixel, 800x600 |

Per order it counts:

- **covered**: every fragment a triangle covers inside the clip volume. With
  late depth (every colour target is linear, so `backend.rs` has no early
  depth, `gc-slow-terrain.md` "The 52 kicks") each of these runs the
  148-instruction shader.
- **passing**: the fragments that pass the depth test when they arrive. With
  early depth only these run the shader.
- **visible**: the pixels the mesh covers, the least `passing` can be.

The orders: glmark2's; glmark2's reversed; random (seed 1); sorted front to
back by triangle view distance (the ideal); sorted back to front (the worst).
Saved ms is `(covered - passing) * 148 * ns`, with 0.6, 0.7 and 0.8 ns an
instruction a pixel (`gc-slow-terrain.md`, "The GPU calibration"; the same
model gives its 42.6 ms for 480 000 fragments at 0.6 ns).

## Numbers

Height map at offset 0 and 0.5 (the first and last second of the 10 s
scene). The script prints exactly these:

| offset | order | covered | passing | visible | cover | rejected |
|---:|---|---:|---:|---:|---:|---:|
| 0 | glmark2 | 463 687 | 432 659 | 432 574 | 0.966 | 6.7 % |
| 0 | glmark2 reversed | 463 687 | 460 904 | 432 574 | 0.966 | 0.6 % |
| 0 | random | 463 687 | 444 692 | 432 574 | 0.966 | 4.1 % |
| 0 | front to back | 463 687 | 432 574 | 432 574 | 0.966 | 6.7 % |
| 0 | back to front | 463 687 | 461 196 | 432 574 | 0.966 | 0.5 % |
| 0.5 | glmark2 | 460 242 | 427 778 | 427 739 | 0.959 | 7.1 % |
| 0.5 | glmark2 reversed | 460 242 | 457 873 | 427 739 | 0.959 | 0.5 % |
| 0.5 | random | 460 242 | 441 118 | 427 739 | 0.959 | 4.2 % |
| 0.5 | front to back | 460 242 | 427 739 | 427 739 | 0.959 | 7.1 % |
| 0.5 | back to front | 460 242 | 458 114 | 427 739 | 0.959 | 0.5 % |

Milliseconds a frame the terrain fragment shader costs (148 instructions):

| ns an instruction | late depth (now), offset 0 / 0.5 | early depth, glmark2 order, offset 0 / 0.5 | saves (offset 0) | saves (offset 0.5) |
|---:|---:|---:|---:|---:|
| 0.6 | 41.2 / 40.9 | 38.4 / 38.0 | **2.8** | **2.9** |
| 0.7 | 48.0 / 47.7 | 44.8 / 44.3 | **3.2** | **3.4** |
| 0.8 | 54.9 / 54.5 | 51.2 / 50.6 | **3.7** | **3.8** |

- Front to back saves the same to within 0.1 ms; back to front 0.2 to 0.3 ms
  at 0.7 ns. Those bound it: **0.3 ms at the worst order, 3.4 ms at the best
  (which glmark2's order already is)**.
- The 2.5 ms low end (offset 0.25, 0.6 ns) and the 3.8 ms high end are the
  range this note gives: **2.5 to 3.8 ms**, central 3.2 ms.

### Sensitivity

Offset 0.25, 0.7 ns, glmark2 order; the height multiplied by `--hscale`:

| height | cover | covered | passing | rejected | saves |
|---:|---:|---:|---:|---:|---:|
| flat (`--flat`) | 0.797 | 382 400 | 382 400 | 0.0 % | 0.0 ms |
| 0.5 x | 0.841 | 403 608 | 401 577 | 0.5 % | 0.2 ms |
| 1 x (the scene) | 0.942 | 452 216 | 424 379 | 6.2 % | 2.9 ms |
| 2 x | 1.389 | 666 779 | 477 988 | 28.3 % | 19.6 ms |
| 4 x | 1.878 | 901 226 | 353 445 | 60.8 % | 56.8 ms |

So the saving grows steeply with hill height, and the old 0 to 14 (19) ms
range corresponds to hills about 1.7 to 1.8 times as tall as the scene's. At the
scene's own height the camera (800 above the origin, looking down at
about 25 degrees) rarely sees a hill hide terrain
behind it.

## Assumptions

- **The height is the noise at the vertex's uv**, not the vertex stage's
  bilinear read of the 256x256 RGBA8 map (8-bit height, half a texel
  offset). The map holds the same function at texel centres, so the error
  is a shift of up to one texel (23 world units) and a 1/255 quantisation;
  neither changes the hill height class. Not checked at the pixel.
- **Early depth is per fragment, at once.** The GC rejects per 2x2 quad
  (a quad is shaded if any of its four pixels passes), so a real rejection is
  a little lower: the rejected fragments lie in thin slivers behind ridges,
  where a quad straddling the edge still runs. The model gives no figure for
  that loss; the table above is the upper figure.
- **Rejected fragments cost nothing**, except the 3.9 ns a pixel floor
  (`imx8mp-frame-floor.md` candidate 2), which the model keeps on the
  rejected pixels (it only removes shader instructions). A rejected pixel
  probably costs less than the floor, so up to 3.9 ns * 31 000 = 0.12 ms
  more.
- **The shader is 148 instructions for every fragment**, as
  `gc-slow-terrain.md` counts it; the cost per instruction is its 0.6 to
  0.8 ns, taken from two effect2d scenes, not measured on terrain.
- **Depth16 and `LESS`.** Ties fail. Z-fight between adjacent triangles of
  one mesh does not occur (the mesh is a single surface and coplanar
  triangles share edges).
- **One kick holds the whole mesh** as one draw in this order, as
  `glDrawArrays` of the VBO (`gc-slow-terrain.md`, row 4 of the kicks).
- **No culling**: glmark2 enables none in this scene; both faces draw, but
  the plane is seen from above only.
- **Hierarchical or coarse-Z rejection** (HiZ, a tile's z-range) is not
  modelled and would only add rejections, never remove them.
- The camera is fixed, so the figure moves only with the height map's
  offset: 6.2 to 7.1 % over the scene's 10 s.

## How session A's off/on pair confirms or refutes it

Session A (`gc-tiled-wire.md`) measures one commit with `gc_tiled_rt` off
and on. Terrain's pass is the only scene pass that W5's early depth reaches
(`gc-tiled-wire.md` lines 319-320). The pair's difference in the terrain
scene has more in it than early depth (a tiled shadow rasterises and
resolves differently, `gc-tiled-wire.md` W4), so read it with the kick and
word records:

| X2 record | reading |
|---|---|
| `gc_scene_kicks_tiled` shows the terrain pass drawing into a tiled target, with `PE_DEPTH_CONFIG` early-Z and `RA_EARLY_DEPTH` set on its draw | early depth is on; only then compare ms |
| terrain's frame ms on minus off, **within about -2 to -6 ms** after the RS resolve's cost is added back (W4's resolve is measured by session A) | **confirms** this model (2.5 to 3.8 ms, up to 4 ms with the floor) |
| a gain of 8 ms or more beyond the resolve | **refutes** the 1x height: either the real hills are taller than the model's (the map or vertex read differs), or tiling alone speeds the RA (fill rate); separate it with a run with early depth off and the target tiled (`no_early_z` behaviour, `gc-tiled-targets.md` step 5 "Board check") |
| no change, or the frame gets slower by the resolve's cost and no more | early depth rejected nothing: it is not on (check the words), or the quad granularity and the floor eat the 3 ms; the model's bound is then too high by at most 3.8 ms |
| a picture that differs from the off picture | a defect, not a number: stop and report |

Because the model's figure is about 2 to 4 % of the pass's 41 to 55 ms
shader time, a single pair on the X2 cannot resolve it if the board's frame
ms varies by more than about 1 ms; the session should repeat each side 3
times and compare means. The model's more testable claim is the one on the
draw order: **the same image with the grid built in the reverse order would
save about 0.3 ms instead of 3 ms**, which no board run is needed to
settle.

Reproduce: `python3 -I tools/vivante/terrain_overdraw.py` (offsets 0 and
0.5), `--offsets=0.25`, `--flat`, `--hscale=2`.

## What this does not show

- No board number; the shader cost per instruction is the calibration's.
- The scene's real height map (RGBA8 texels read by the vertex stage) was
  not rendered. A host render of glmark2's terrain pass with
  `tools/hostharness/glmark2/run.sh` could read back the height map's bytes
  and replace the noise function here; not done.
- The GC7000UL's early-depth granularity (quad or tile) and its rate for a
  rejected quad are not measured.
