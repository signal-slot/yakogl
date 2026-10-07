# V3D refract: which texture is refused, and what budget removes it

**The three harness runs were not made.** This session could not start
`tools/hostharness/glmark2/run.sh` (nor `cmake`, `cargo`, or a copy of the
script): every one of them was refused as "requires approval", and nobody
could grant it. Only `tools/check.sh --quick` and read-only commands ran. So
this note has **no measured numbers for the three runs**. It has (a) the
numbers already measured in
[FINDINGS-post-npot.md](FINDINGS-post-npot.md) for the 8 MiB default,
(b) the tiled size worked out from the layout code, and (c) a prediction for
16 MiB and 24 MiB from `tiled_copy`'s rules. Predictions are marked. The
commands to make the real runs are in section 5.

## 1. The refused texture

Source: glmark2 `scene-refract.cpp` (lines 177-210), `DistanceRenderTarget`.

| | value |
|---|---|
| role | the FBO's **colour attachment** (`tex_[COLOR]`), a render target |
| texture unit | 1 (the "incomplete texture, unit 1, 1600x1200" line of FINDINGS-post-npot.md) |
| GL name | not known without a run. `glGenTextures(2, tex_)` makes the pair, so it is the depth texture's name + 1 |
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
  This matches FINDINGS-post-npot.md: 1 refusal line, 3 `-22` lines, 3 `0x505`.
* The small NPOT chains of terrain are far below the budget: an 800x600 RGBA8
  chain is 2,780,160 bytes (2.65 MiB).

## 3. The three runs

### 3.1 8,388,608 (the default): measured earlier

From FINDINGS-post-npot.md (tree `fdff1147`, same code as this tree for the
V3D; `v3d` row only). Not re-run here.

| entry | GL errors | refused (scene table) | `-22` lines | stale-tiled-copy lines | GPU peak (of 100,134,912) |
|---|---|---|---|---|---|
| 24 refract | 3 x `0x505 in glDrawArrays` | 0 (the table counts `yakogl_draws_refused`, not the backend's `-22`) | 3 (+ 1 "draw refused" line) | 1 (7,684,096 bytes) | 58,087,424 (58.0%) |
| 22 terrain | 0 | 0 | 0 | 0 | 59,829,248 (59.7%), the run's peak |

### 3.2 16,777,216: prediction, not a run

Rules (`tiled_copy`, `backend.rs:3009-3070`): the colour copy asks for
11,005,952 bytes. Live: the depth copy, 7,684,096. Sum 18,690,048 is over
16,777,216, so the loop picks a victim. The only live copy is the depth one,
sampled by the open job (age <= 1). The working-set guard is skipped because
`image_bytes + 4096 + 64` is over the arena, so the depth copy is evicted
(the OSAL frees it after that job). The colour copy is then made.

| entry | GL errors | refused | `-22` | stale-tiled-copy | GPU peak |
|---|---|---|---|---|---|
| 24 refract | predicted 0 | 0 | predicted 0 | 1 | about 69.1 MB (58,087,424 + 11,005,952 = 69,093,376; 69.0%), predicted |
| 22 terrain | 0 | 0 | 0 | 0 | at most the 8 MiB figure plus extra kept copies; predicted below 76 MB |

The depth copy is dropped and made again, so each frame tiles 7.68 MB of
depth too. That is a cost in time, not a refusal.

### 3.3 25,165,824: prediction, not a run

Sum of live and new, 18,690,048, is under 25,165,824: no eviction, both copies
stay for the frame. Refract: 0 errors, 0 `-22`, peak about 69.1 MB (69.0%),
predicted. Terrain: as above, with room for more kept copies.

## 4. Conclusions

* **A larger budget removes the refusal.** Cause confirmed by the code path:
  the 10.49 MiB mip chain of the colour render target is over the 8 MiB budget
  and the 8 MiB arena. (Confirmed by reading and arithmetic, **not by the
  runs**.)
* **Smallest value.** The copy alone needs 11,005,952 bytes, so any value
  from 11,005,952 up passes `OverBudget`: in the Kconfig's whole MiB, 12 MiB
  (12,582,912). With the depth copy live at the same time, the colour copy
  fits only by evicting the depth copy, which works at 16 MiB but re-tiles
  depth every frame. 24 MiB (the Kconfig help's own advice for a 16.8 MiB
  image) keeps both: 18,690,048 bytes. The probe's candidates 16 MiB and
  24 MiB should both clear the refusal; the run must confirm it.
* **96 MiB carve-out.** Yes, it still holds. The colour copy is about
  11 MB in memory of its own, on top of refract's measured 58.1 MB: about
  69.1 MB of 100,134,912, with the longest free run at exit of 84.7 MB (larger
  than the 11 MB copy). The budget caps only the kept copies, not the carve-out.
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
  `src/v3d/**`); `pi4-refract-draw` should take the budget only if that does
  not remove the cause.

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
only the large allocations), so the harness rows below remain the authority.

**Status of the section 3.2/3.3 predictions: still predictions.** The glmark2
source (pinned commit 22c527cb) was not in the deps cache and the fetch of it
was refused ("requires approval") in the pi4-refract-draw session, so the host
harness runs of section 5 and the 48 MiB control row were **not made**. No
config was changed (`rpi_4b.conf`, `zephyr/Kconfig` untouched): the rule of the
item is to set a budget only from measured rows. Run section 5 on a machine
with the source, add `YAKOGL_V3D_CARVEOUT_BYTES=50331648` with the 8 MiB budget
for the control row (expected: "out of GPU memory" for refract), and give the
harness `YAKOGL_V3D_CARVEOUT_BYTES=100663296` explicitly for the three budgets.

## 5. The runs still to make

```sh
GLMARK2_SRC=/home/tasuku/.local/share/ccfarm/deps/yakogl/glmark2   # at 22c527cb
for b in 8388608 16777216 25165824; do
  GM2_WRITE=0 YAKOGL_V3D_TILED_COPIES_BYTES=$b \
    tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/refract-budget-$b v3d
done
```

`GM2_WRITE=0` leaves `FINDINGS*.md` and `tools/shaders/glmark2/` alone, so
there is nothing to restore (check with `git status`). For each run look at
`build/refract-budget-$b/logs/v3d/{22,24}.log`: count `GL error 0x`,
`draw refused`, `(-22)`, `stale tiled copy`, `tiled copies high-water`, and
read the entry's `high-water` in the report. Expected: `0x505` 3, 0, 0 at
16 MiB and 24 MiB.
