<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D refract: which texture is refused, and what budget removes it

**Status (2026-10-12): done.** The runs below were made on the integration
tree `bc833b399c5a`. Re-landed on main, the 24 MiB budget was run again
there (origin/main `837b36b9` plus the `rpi_4b.conf` line,
`tools/hostharness/glmark2/run.sh <glmark2> build/gm2-v3d v3d`): refract
peaks at 69,093,376 bytes with a tiled high-water of 19,742,720 and terrain
at 61,208,576, the numbers of section 3.3, with no refused draw and no GL
error ([FINDINGS-v3d.md](FINDINGS-v3d.md)).
No board run yet (section 6).

**The four harness runs are made.** Measured on the yakogl integration tree
`bc833b399c5a` with glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5` (the
GitHub archive in the deps cache; each run's `provenance.txt` records it).
Sections 3.1-3.4 are measured rows, section 4 says where measurement and the
earlier predictions differ, and section 5 lists the exact commands. The
arithmetic of sections 1-2 stands as written. `GM2_WRITE=0` left
`FINDINGS*.md` and `tools/shaders/glmark2/` untouched (`git status` clean
after the runs).

## 1. The refused texture

Source: glmark2 `scene-refract.cpp` (lines 177-210), `DistanceRenderTarget`.

| | value |
|---|---|
| role | the FBO's **colour attachment** (`tex_[COLOR]`), a render target |
| texture unit | 1 (the "incomplete texture, unit 1, 1600x1200" line the harness logged before `2673197d`) |
| GL name | not printed by the runs. `glGenTextures(2, tex_)` makes the pair, so it is the depth texture's name + 1 |
| size | 1600 x 1200: twice the 800x600 canvas (`width_ = canvas_.width() * 2`) |
| format | `GL_RGBA`, `GL_UNSIGNED_BYTE`: RGBA8, 4 bytes a texel |
| min filter | `GL_LINEAR_MIPMAP_LINEAR`; `glGenerateMipmap` is called on it after the depth pass |
| levels | 11 (1600x1200 down to 1x1: `floor(log2 1600) + 1`) |
| render target | yes. level 0 is rendered by the GPU, then sampled by the main pass |

The depth texture (unit 0, `GL_DEPTH_COMPONENT`, R32F, 1600x1200, NEAREST, one
level) is not the refused one. It is the "stale tiled copy of 7684096 bytes"
line: 1600 x 1200 x 4 = 7,680,000, plus 4,096 for the page.

## 2. Its tiled size, with arithmetic

`texture::layout` calls `v3d_tex_layout` (`driver/src/v3d/v3d_tex.c:151`,
Mesa's `v3d_setup_slices`). Utile for 4 bytes a texel is 4x4 texels, UIF block
8x8, a UIF width is aligned to 4 blocks = 32 texels. Levels 0 and 1 have their
own size; levels 2 and up come from the power-of-two size of level 1
(1024 x 1024 doubled to 2048 x 2048, then halved per level). I ported the
function to a short Python script (not committed) and got:

| level | stored size (texels) | tiling | bytes |
|---|---|---|---|
| 0 | 1600 x 1200 | UIF | 7,680,000 |
| 1 | 800 x 600 | UIF | 1,920,000 |
| 2 | 512 x 512 | UIF | 1,048,576 |
| 3 | 256 x 256 | UIF | 262,144 |
| 4 | 128 x 128 | UIF | 65,536 |
| 5 | 64 x 64 | UIF | 16,384 |
| 6 | 32 x 32 | UIF | 4,096 |
| 7 | 16 x 16 | UBLINEAR 2 col | 1,024 |
| 8 | 8 x 8 | UBLINEAR 1 col | 256 |
| 9, 10 | 4 x 4 | LINEARTILE | 64 + 64 |

* Whole chain: **11,001,856 bytes** (10.49 MiB). `image_bytes` in
  `tiled_copy`. Level 0 alone is 7,680,000, which is why it fitted before npot
  (one level, 7.68 MB < 8 MiB = 8,388,608).
* `tiled_copy` asks for `bytes = image_bytes + 4096 = 11,005,952`.
* Refusal test at `src/v3d/backend.rs:2907`: `image_bytes + 4096 + 64`
  = 11,006,016 > `TEX_BYTES` = 8,388,608. True, so the arena fallback is out
  too.
* At the 8 MiB budget: `bytes` 11,005,952 > 8,388,608, so `tiled_copy` returns
  `KeptMiss::OverBudget`, and then the arena test above refuses with the
  message at `backend.rs:2913` (`-22`, GL error 0x505 on the draw).
  This matches the post-npot run (tree `fdff1147`) and [FINDINGS-v3d.md](FINDINGS-v3d.md): 1 refusal line, 3 `-22` lines, 3 `0x505`.
* The small NPOT chains of terrain are far below the budget: an 800x600 RGBA8
  chain is 2,780,160 bytes (2.65 MiB).

## 3. The four runs

### 3.1 8,388,608 (the default): measured here, reproduces the post-npot run

Run `build/refract-budget-8388608`. Every count equals the post-npot row
(tree `fdff1147`), so the refusal is stable across the trees.

| entry | GL errors | `draw refused` lines | `-22` lines | stale-tiled-copy lines | tiled high-water lines | GPU peak (of 100,134,912) |
|---|---|---|---|---|---|---|
| 24 refract | 3 x `0x505 in glDrawArrays` | 1 | 3 | 1 (7,684,096 bytes) | 2: 1,052,672, 7,684,096 | 58,087,424 (58.0%) |
| 22 terrain | 0 | 0 | 0 | 0 | 3: 1,404,928, 3,076,096, 6,152,192 | 59,829,248 (59.7%), the run's peak |

The scene table still says `refused=0` for refract (it counts
`yakogl_draws_refused`, not the backend's `-22`). refract ran at 35 FPS
(score 34). That is an artifact of the refused draw - the frame skips its
work - not a speed advantage of the 8 MiB budget.

### 3.2 16,777,216: measured

Run `build/refract-budget-16777216`. The refusal is gone.

| entry | GL errors | `draw refused` lines | `-22` lines | stale-tiled-copy lines | tiled high-water lines | GPU peak (of 100,134,912) |
|---|---|---|---|---|---|---|
| 24 refract | 0 | 0 | 0 | 0 | 2: 1,052,672, 8,736,768 | 70,146,048 (70.1%) |
| 22 terrain | 0 | 0 | 0 | 0 | 4: 1,404,928, 3,076,096, 6,152,192, 12,433,408 | 61,208,576 (61.1%) |

The eviction rule worked as predicted (`tiled_copy`, `backend.rs:3009-3070`):
the colour copy (11,005,952 with its page) does not fit beside the live depth
copy (7,684,096) under 16,777,216, so the depth copy is evicted (the
working-set guard is skipped: the colour chain is larger than the 8 MiB
arena) and the colour copy is made. Two measured details the prediction got
wrong:

* **0 stale-tiled-copy lines, not 1.** The depth copy is dropped by the
  eviction loop, not by `drop_stale_copies` (`backend.rs:3230`), and only
  the latter logs that line.
* **Peak 70,146,048, not the predicted 69,093,376.** The evicted depth
  copy's free waits for the job that sampled it, so colour and depth memory
  coexist in the allocator: 1,052,672 bytes above the predicted sum.

The last logged high-water line is 8,736,768 (a line logs at each doubling,
`backend.rs:3119`); the true high-water lies between it and the budget - the
colour copy alone is 11,005,952, and depth and colour are never live at once
(18,690,048 > 16,777,216). refract ran at 23 FPS: the refused draw now
runs, so this is its real cost, not a slowdown against the 8 MiB run's 35
(which skipped that draw).

### 3.3 25,165,824: measured

Run `build/refract-budget-25165824`.

| entry | GL errors | `draw refused` lines | `-22` lines | stale-tiled-copy lines | tiled high-water lines | GPU peak (of 100,134,912) |
|---|---|---|---|---|---|---|
| 24 refract | 0 | 0 | 0 | 1 (7,684,096 bytes) | 3: 1,052,672, 8,736,768, 19,742,720 | 69,093,376 (69.0%) |
| 22 terrain | 0 | 0 | 0 | 0 | 4: 1,404,928, 3,076,096, 6,152,192, 12,433,408 | 61,208,576 (61.1%) |

As predicted: 18,690,048 (depth + colour) is under 25,165,824, no eviction,
both copies stay for the frame. The high-water line 19,742,720 = 1,052,672 +
7,684,096 + 11,005,952: a small upload copy is live beside both. The peak
69,093,376 is exactly the predicted 58,087,424 + 11,005,952, and 1,052,672
below the 16 MiB run's: with no eviction, no free waits for a job. The stale
line is back (the depth copy lives across frames and is given back before the
next frame's job opens; the depth texture gets a new stamp every frame at
every budget). refract ran at 23 FPS, the same as at 16 MiB: keeping the
depth copy gave no measured speed gain on the host harness.

### 3.4 48 MiB control: 8,388,608 budget, 50,331,648 carve-out

Run `build/refract-budget-48m` (allocatable 49,803,264). Expected: "out of
GPU memory" for refract. Measured - and terrain fails too:

| entry | GL errors | refused allocations | "out of GPU memory" lines | GPU peak (of 49,803,264) |
|---|---|---|---|---|
| 24 refract | 7: 2 x `0x502` + 1 x `0x505` in `glGenerateMipmap`, 2 x `0x506` in `current_drawable_program`, 2 x `0x506` in `resolve_target` | 2 | 3 | 47,825,664 (96.0%) |
| 22 terrain | 5: 3 x `0x505 in glDrawArrays`, 2 x `0x505 in the pending clear` | 17 | 3 | 49,501,184 (99.4%) |

At 48 MiB the failure is earlier than the tiled copy: the colour texture's
own 11-level linear storage (10,239,836 bytes) and the depth texture
(7,684,096) are refused by the OSAL, so the scene errors before any draw is
refused. The scores (refract 71 at 72 FPS, terrain 52 at 53 FPS) are
meaningless here: refused allocations mean less work per frame.

## 4. Conclusions (measured)

* **A larger budget removes the refusal: confirmed by the runs.** At
  8,388,608 refract still shows 1 `draw refused` line, 3 `(-22)` and 3
  `0x505`; at 16,777,216 and 25,165,824 all three counts are 0 (section 3).
* **Which value.** The copy alone needs 11,005,952 bytes, so any value from
  11,005,952 up passes `OverBudget`: in the Kconfig's whole MiB, 12 MiB
  (12,582,912). But with the depth copy live at the same time, 16 MiB keeps
  both only by evicting the depth copy every frame - measured: a *higher*
  carve-out peak (70,146,048) because the evicted copy's free waits for a
  job. 24 MiB keeps both copies with no per-frame eviction (high-water
  19,742,720) and peaks lower, at 69,093,376. The FPS does not decide it:
  16 and 24 MiB both ran refract at 23 FPS (the 8 MiB run's 35 skipped the
  refused draw). The choice, on the memory evidence, is **24 MiB
  (25,165,824)**: the smallest measured budget that keeps both copies - the
  Kconfig help's own advice - not the smallest passing value. It is set in
  `driver/samples/zephyr/glmark2/boards/rpi_4b.conf` and documented in
  `driver/docs/glmark2.md`.
* **96 MiB carve-out: confirmed.** At the 24 MiB budget the whole default
  list peaks at 69,093,376 of 100,134,912 with 0 refused allocations (all
  33 entries), and the longest free run at exit is 84,758,016. The budget
  caps only the kept copies, not the carve-out.
* **Should it need a tiled copy at all? Not in principle.** It is a
  render target the V3D writes itself. Mesa's V3D renders into a UIF layout
  and the TMU samples that buffer directly, with no copy. Here the GL core
  keeps every texture linear and the V3D renders into the linear storage, so
  the TMU cannot read it ("raster is not valid as texture input",
  `texture.rs` header) and the CPU/TFU makes a UIF copy. Every frame the
  render changes level 0, so the copy is dropped and made again: 11 MB tiled
  per frame, which is also the cost of the chain. A direct sample would need
  the render target to be laid out as the TMU reads it (and the mip levels
  1-10 made by a GPU blit as Mesa does, not by the core's CPU
  `glGenerateMipmap`). That is item `v3d-rt-sample`'s work (it owns
  `src/v3d/**`). The 24 MiB budget is the workaround until then: if that
  item removes the copy, the Pi 4 board file can drop the budget line and
  go back to the 8 MiB default.

## 4a. Host replay of refract's peak (pi4-refract-draw, measured)

`cargo test --test v3d_glmark2_mem` (test
`refract_with_the_colour_chain_copy_fits_the_96_mib_carve_out`, tree
`e8648bac`) replays refract with the 11,005,952-byte colour copy through the
OSAL's real pool under the 96 MiB carve-out (100,663,296), after base, terrain
(npot chains) and shadow, in both orders. This is a pool replay, **not** the
glmark2 harness: it shows the budget fits the carve-out, not that the draw
stops being refused.

| budget | depth copy | peak use (permanent set included) | of 100,663,296 | refused |
|---|---|---|---|---|
| 12 MiB | evicted before the colour copy | 51,883,808 | 51.5% | 0 |
| 16 MiB | evicted before the colour copy | 51,883,808 | 51.5% | 0 |
| 24 MiB | kept with it | 59,567,904 | 59.2% | 0 |

The replay's peak is lower than the harness's 58 MB for refract (it models
only the large allocations), so the harness rows of section 3 remain the
authority.

**Status of the section 3.2/3.3 predictions: measured now** (sections 3.2-3.4
of this note, on tree `bc833b399c5a`). The pool replay's numbers stand; the
harness rows are the authority, and they confirm the replay's point: the
colour copy fits the carve-out at every budget tried. The config **is**
changed from measured rows: `rpi_4b.conf` sets
`CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES=25165824` (section 4; the Kconfig
symbol `YAKOGL_V3D_TILED_COPIES_BYTES` accepts it - range 1 MiB to 256 MiB,
default unchanged for the other boards).

## 5. The runs made

On the node with the glmark2 source in the deps cache (the GitHub archive
`glmark2-22c527cb0556f3a1ac4445aaa52cc532760928d5`, accepted by `run.sh`'s
provenance check; each out directory's `provenance.txt` records it):

```sh
GLMARK2_SRC=$CCFARM_DEPS_DIR/glmark2-22c527cb0556f3a1ac4445aaa52cc532760928d5
for b in 8388608 16777216 25165824; do
  GM2_WRITE=0 YAKOGL_V3D_TILED_COPIES_BYTES=$b YAKOGL_V3D_CARVEOUT_BYTES=100663296 \
    tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/refract-budget-$b v3d
done
GM2_WRITE=0 YAKOGL_V3D_TILED_COPIES_BYTES=8388608 YAKOGL_V3D_CARVEOUT_BYTES=50331648 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/refract-budget-48m v3d
```

`GM2_WRITE=0` left `FINDINGS*.md` and `tools/shaders/glmark2/` alone (`git
status` clean after the runs). Counts read from
`build/refract-budget-*/logs/v3d/{22,24}.log` (`GL error 0x`, `draw refused`,
`(-22)`, `stale tiled copy`, `tiled copies high-water`) and from each run's
report carve-out table; the results are sections 3.1-3.4. The expectation
`0x505` 3, 0, 0 held.

## 6. Board check (Pi 4)

Build and run the glmark2 image on the Pi 4 as `driver/docs/glmark2.md`
section 2 recipes it (the AM62P lines with the Pi 4 board; `rpi_4b` is the
board name `tools/cts/common-images.sh` uses):

```sh
west build -p always -d <ws>/build/gm2-pi4 -b rpi_4b \
    driver/samples/zephyr/glmark2 -- \
    "-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh <ws>)" \
    -DGLMARK2_SRC=$PWD/build/glmark2-upstream   # at 22c527cb
RUN_BOUND=960 tools/board.sh pi4 run <ws>/build/gm2-pi4/zephyr/zephyr.bin <logdir> glmark2
```

What refract and terrain must print: no `draw refused` line, no `out of GPU
memory` line, no `(-22)` and no `GL error 0x505` anywhere in the console, and
the run ends `glmark2: PASS` (board_finish). At the old 48 MiB carve-out the
board console printed "out of GPU memory" for refract and "no GPU memory for
a tiled copy" refusals for terrain (`tools/cts/RESULTS.md`); the 8 MiB default's refract refusal is measured on
the host (section 3.1). With the 24 MiB budget now in
`rpi_4b.conf` neither must appear. Not verified on the board yet - this item
measured on the host only.
