<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the 85 AM62P dEQP-GLES3 `texture.mipmap` fails (T-MIPMAP)

Roadmap stage 4 (ES 3.0), plan #400. This note splits cause T-MIPMAP of
`docs/es3/am62p-gles3-notpass.tsv` into sub-causes. For each one it says
whether origin/main fixes it, and gives a fix design with a host test for
those still open. Nothing here was run on a board. Code references are at
**`2340445f0`**, the origin/main tip when this was written. Its `src/pvr`,
`src/shaderc`, `src/gl` and `driver/src/pvr` are byte-identical to
`14e7a742`: none of the 40 commits between the two touches them.

## 1. Where the 85 come from

`python3 -I tools/cts/am62p-notpass-causes.py docs/es3/am62p-gles3-notpass.tsv`
gives `T-MIPMAP 85 known` (rule `^texture\.mipmap\.`, Fail only). Every
`texture.mipmap` line of the TSV is in it. No other cause (K3, K4, K5 ...)
claims a `texture.mipmap` line. All 85 are in tree row 39 of
`tools/cts/am62p-gles3-tree.txt` (`...texture.mipmap.*` with the
filtering.cube, format, shadow ... groups). The TSV header puts that row in
the **`ad2b5ed` build, run 2026-10-13/14**. All 85 have the same message,
"Image verification failed".

dEQP source: `modules/gles3/functional/es3fTextureMipmapTests.cpp` of
VK-GL-CTS `93bca01` (the commit `DEQP_WORK` pins). The mustpass list has 224
`texture.mipmap` cases.

| target | sub-group | mustpass | Fail | POT / NPOT of the failing cases | texture |
|---|---|---|---|---|---|
| 2d | basic, affine, projected, bias, generate, max_level | 36+12+12+4+48+4 | 0 | | |
| 2d | base_level | 4 | **4** | POT | 64x64 RGBA8 |
| 2d | min_lod | 4 | **4** | POT | 64x64 RGBA8 |
| 2d | max_lod | 4 | **4** | POT | 64x64 RGBA8 |
| cube | basic, bias, generate | 4+2+16 | 0 | | |
| cube | base_level | 4 | **4** | POT | 64x64 faces RGBA8 |
| cube | min_lod | 4 | **4** | POT | 64x64 faces RGBA8 |
| cube | max_level | 2 | **1** (`linear_linear`; `nearest_linear` passes) | POT | 64x64 faces RGBA8 |
| 3d | basic | 24 | **24** | 12 POT (32x32x32), 12 `_npot` (33x29x27) | RGBA8 |
| 3d | affine | 12 | **12** | POT | 32x32x32 RGBA8 |
| 3d | projected | 12 | **12** | POT | 32x32x32 RGBA8 |
| 3d | bias | 4 | **4** | POT | 32x32x32 RGBA8 |
| 3d | min_lod | 4 | **4** | POT | 32x32x32 RGBA8 |
| 3d | max_lod | 4 | **4** | POT | 32x32x32 RGBA8 |
| 3d | max_level | 4 | **4** | POT | 32x32x32 RGBA8 |
| | **total** | 224 | **85** | 73 POT, 12 NPOT | |

`generate` (2d 48, cube 16) has no fail. `3d.base_level`, `cube.max_lod` and
`cube.projected` are not in the mustpass. ES 3.0 dEQP has no 2D-array
mipmap group.

## 2. Sub-causes and their state at the tip

| id | cases | count | cause | state at `2340445f0` |
|---|---|---|---|---|
| M1 | `2d.base_level.*` | 4 | GL_TEXTURE_BASE_LEVEL never reached a 2D bind: sampled and sized from level 0 | **fixed at `4c0bcd77` + `349f989f5`** (host-pinned; no board run yet) |
| M2 | `2d.min_lod.*`, `2d.max_lod.*` | 8 | MIN_LOD / MAX_LOD dropped by the GL core: SAMPLER_WORD0 minlod/maxlod always 0..levels-1 | **fixed at `349f989f5`** (host-pinned; no board run yet) |
| M3 | `cube.base_level.*`, `cube.min_lod.*` | 8 | as M1/M2 for the native cube (baselevel 0, LOD range 0..levels-1) | **fixed at `349f989f5`** (host-pinned; no board run yet) |
| M4 | `cube.max_level.linear_linear` | 1 | in one frame, a MAX_LEVEL 0 cell rewrites the native cube's full-chain copy in place as a one-level copy (and the next cell writes it back) while the earlier cells' draws still read it | **needs src/gl (named, section 4)** |
| M5 | `3d.basic.*` POT (12), `3d.affine.*` (12), `3d.bias.*` (4) | 28 | the default arm samples a 3D texture as a level-0 slice grid: no mip levels, REPEAT/MIRROR lost on every axis | fixed on the native-3D arm at `e179ad48`; **default OFF** (`PVR_NATIVE_3D_DEFAULT = 0`). Design: flip the default (section 5) |
| M6 | `3d.basic.*_npot` | 12 | as M5, plus NPOT level layout | native arm, `e179ad48` (NPOT level twiddle); **default OFF**. Design: section 5 |
| M7 | `3d.min_lod.*`, `3d.max_lod.*`, `3d.max_level.*` | 12 | as M5. On the native arm the copy also ignored BASE_LEVEL/LOD range, and a copy rewritten for the next cell changed earlier cells | native arm, **fixed at `e90122b8`** (host-pinned); **default OFF**. Design: section 5 |
| M8 | `3d.projected.*` | 12 | as M5. On the native arm, 5 of 12 still failed: the TPU's per-quad implicit LOD over a perspective varying, because the exact plane gradients (`pvr_proj_lod`) cover 2D and cube but not 3D | **open**: fix design section 6 (src/shaderc/online/powervr), plus the section 5 flip |
| | | **85** | | 20 fixed at the tip (M1-M3), 1 needs src/gl (M4), 64 wait on the native-3D default (M5-M8), of which M8 also needs the section 6 change |

PVR_NATIVE_3D_DEFAULT at the tip: **0** (`src/shaderc/mod.rs:342`). Native
cube and native 2D array are on (`PVR_NATIVE_CUBE_DEFAULT = 1` :282,
`PVR_NATIVE_ARRAY_DEFAULT = 1` :314).

**K3 and K4 are 0 of the 85.** K3 (`docs/notes/pvr-npot-cube-levels.md`,
`texture-failure-map.md` 2.2.1) is an NPOT cube that keeps only its base
level. Every `texture.mipmap.cube` case uses a 64x64 POT face, so K3 cannot
apply. K4 (NPOT 2D array) has no ES 3.0 mipmap case. The only NPOT cases
here are M6 (3D).

**Board pin caveat.** The board sessions of plan #400 build `472630df`. That
commit has `e179ad48` and `e90122b8`, but **not `4c0bcd77`, `349f989f5` or
`d3743fa5e`**. A row-39 result at `472630df` therefore still shows M1-M3
failing (20 cases). That is expected and is not a regression. M1-M3 show on
the board only in a build from `349f989f5` or later.

## 3. M1-M3: fixed at the tip (BASE_LEVEL, MIN_LOD, MAX_LOD)

dEQP's LOD-control cases (`Texture2DLodControlCase`,
`TextureCubeLodControlCase`) draw a grid of cells into one frame. Each cell
sets one parameter with `glTexParameter*` before its draw: 16 cells in a 4x4
grid for 2D, 17 random cells for the cube (`randomPartition`, seed 7, over
128x128). The MIN_LOD values are {1, 3.5, 2, -2, 0, 3, 10, 4.8, 5.8, 5.7, -1.9,
4, 6.5, 7.1, -1e10, 1000}. The MAX_LOD values are {0, 0.2, 0.7, 0.4, 1.3, 0,
0.5, 1.2, -2, 1, 0.1, 0.3, 2.7, 1.2, 10, -1000, 1e10}. BASE_LEVEL and
MAX_LEVEL come from `deInt32Hash(cell) ^ deStringHash(name) ^ k % 7`.

- `4c0bcd77` (2026-11-27): a 2D bind's base level moves the linear image's
  address, extent and stride (`src/pvr/texture.rs:160` `rebase_stride_bind`;
  `tests/pvr_texsize_base_level.rs`).
- `349f989f5` (2026-11-27): `GlesTextureBind` carries `lod_set`/`min_lod`/
  `max_lod`. `src/pvr/texture.rs:97` `bind_lod` gives every non-3D bind its
  base level and LOD range. `src/pvr/backend.rs:7237`
  `tex_desc_sampler_lod` packs SAMPLER_WORD0 minlod/maxlod (u4.6, counted
  from the base, minlod capped at maxlod :7260-7264). The native cube names
  IMAGE_WORD1.baselevel over its full-chain copy (`:6913`, `:7010`). Pinned
  by `tests/es3_pvr_texlod.rs`. `d3743fa5e` (on main) adds MAX_LEVEL and
  MAX_LOD 0 through a real draw for 2D and the native cube.
- Edge values: MIN_LOD 10 / 1000 is capped at the last level (`minlod =
  lo.min(maxlod)`). MAX_LOD -2 / -1000 gives maxlod 0. The TPU then chooses
  the mag filter on the clamped LOD, as dEQP's reference does (it clamps
  `triLod` to [minLod, maxLod] before choosing). The u4.6 rounding down
  (5.7 -> 5.6875) is Mesa's own packing for the BXS-4-64 (no quirk 51025
  bias). Under trilinear it moves the blend weight by at most 1/64, about
  0.7 of an 8-bit step between two dEQP levels. That is inside the colour
  threshold (pixel bits - 2). With `*_MIPMAP_NEAREST`, none of the values
  sits within 1/64 of a .5 level boundary except 3.5 and 6.5, which u4.6
  holds exactly.
- The host cannot give the verdict: the Linux OSAL reads back black. In
  `349f989f5`'s host dEQP, all 224 cases fail before and after. Board check
  needed: row 39 in a build of `349f989f5` or later.

Nothing is left to design for M1-M3.

## 4. M4: `cube.max_level.linear_linear` (needs src/gl)

**Why this one case.** dEQP's cube MAX_LEVEL for cell `c` is
`(deInt32Hash(c) ^ deStringHash(name) ^ 0x974e21) % 7`. Over the case's 17
cells (computed with dEQP's hashes and `de::Random(7)` partition of the
128x128 viewport):

- `linear_linear`: 5 4 2 3 4 6 5 3 3 **0** 4 2 6 2 5 4 2, so cell 9 has
  MAX_LEVEL 0.
- `nearest_linear`: 6 3 2 6 4 5 4 4 2 3 6 6 5 6 4 5 3, with no 0. This case
  passes.

The two cube.max_level cases outside the mustpass also have zero cells
(`linear_nearest` cells 7 and 13-14, `nearest_nearest` cell 3). The only
mustpass case with a MAX_LEVEL 0 cell is the only one that fails.

**Mechanism.** With MAX_LEVEL 0 the GL core gives the bind `mip_levels` 1
(`src/gl/draw.rs:722-725` `sampling_fields`). The draw then asks for a
one-level copy (`src/gl/draw.rs:5115-5117`, `single = !(mip_filter != 0 &&
mip_levels > 1)`). `src/gl/native_cube.rs:181` `copy` finds its entry by
source alone (`:198`: `src_gpu`, `src_size`). It sees `e.single != single`
(`:207`) and rewrites **the same entry**. It also uses the same memory,
because the one-level copy (6 x 16 KiB) is smaller than the full chain
(6 x 88 KiB). Nothing checks whether a kick still reads the entry.

All 17 cells are recorded into one open frame and run at the
`glReadPixels`. When the frame runs, the memory holds whatever the last copy
wrote, which is cell 10's full chain. Cells 0-8 and 10-16 read the right
bytes. Cell 9's descriptor is a one-level TEXTYPE_CUBE, whose face stride
is one level's 16 KiB (`face_bytes`, `:137`). So it reads faces 1-5 out of
face 0's chain. The cell is wrong and the case fails.

**No PowerVR-only fix.** The back end cannot describe the full-chain copy as
one level. The TPU's face stride follows the level count (Mesa
`pvr_image_plane_setup_mip_levels`), and maxlod 0 over the full chain
magnifies where GL minifies (LINEAR_MIPMAP_LINEAR at MAX_LEVEL 0 minifies
on level 0). The copy is a GL-core object, so the fix is in `src/gl`.

**The src/gl change, named.** `src/gl/native_cube.rs` `copy` (:181) takes
`gl::native_vol`'s rules (`src/gl/native_vol.rs:283` `in_flight`,
`:305-326` `copy`):

1. Key the entry by `single` too: `&& e.single == single` in the `hit`
   predicate at :198. A frame that draws one cube at one level and at its
   full chain then holds two copies side by side. This is what
   `native_vol` does per `view.base` / `view.held`.
2. Add `Entry::handed` (`backend::kick_id()` when the copy is handed to a
   draw) and `in_flight(e)` (kick `handed` or `handed + 1` pending). An
   entry in flight is never written: a stale one leaves the lookup
   (`stale`), its slot is reused once its kicks are done, and when all 16
   slots are in flight the open frame is flushed and finished first.

`src/gl/native_array.rs:176`/`:194` has the same lookup and the same
in-place rewrite. No `texture.mipmap` case reaches it (there is no
2D-array mipmap group), so it is not counted here. It belongs with the same
change.

**Host test** (new, `tests/pvr_native_cube_frame.rs`, features
`host-test,gpu-powervr,shaderc-online`, with the stand-in back end and the
PowerVR's kick numbering that `tests/es3_pvr_3d_lod.rs` uses):

- A 64x64 RGBA8 cube, 7 levels, each level and face a distinct colour,
  LINEAR_MIPMAP_LINEAR, a `samplerCube` program (native).
- Replay `linear_linear`'s 17 MAX_LEVEL values, one draw per cell, with no
  kick in between. For each draw, record `base_gpu`, `bytes` and the
  descriptor's level count.
- At the end of the frame, assert for every cell that the bytes at its
  `base_gpu` are the layout its own descriptor describes: a one-level copy
  for cell 9, the full chain for the rest. In particular, cell 9's
  `base_gpu` differs from cell 8's, and cell 8's bytes are unchanged after
  cell 9's copy.
- Then finish the kick and draw MAX_LEVEL 0 again. The kept one-level copy
  is reused (`native_cube::copies()` does not grow).

Today the first assertion fails at cell 9 (one shared address).

## 5. M5-M7: native 3D textures, default OFF at the tip

**On the default arm** (`pvr_native_3d` = 0) a 3D texture is glsl2usc's
slice grid. `Sel::vol_uv` samples level 0 of one 2D image and clamps u, v
and the slice index (`docs/notes/pvr-3d-native.md`, "The defect"). Every
`mipmap.3d` case draws a different colour per level and wraps (clamp,
repeat, mirror), so all 64 fail. This is the same cause as K5 and as the
`filtering.3d` rows of T-FILTER. `3d.bias.*` used to fail to compile ("a
bias on a ND texture", `texture-failure-map.md` 2.1). `6df551233` made it
compile, so at `ad2b5ed` it fails on the picture like the rest.

**On the native arm** (`CONFIG_DEQP_PVR_NATIVE_3D=1`, `DEQP_PVR_NATIVE_3D=1`
for `tools/cts/am62p-deqp-images.sh`):

- `src/shaderc/online/powervr/select.rs:1526-1548` samples a sampler3D as
  pco does: `smp` with `dmn` 3d over TEXTYPE_3D. The TPU picks the level,
  filters across slices and wraps every axis. A bias goes through
  `smp_level` (:3949).
- `e179ad48` (2026-11-05): NPOT level twiddle (the physical power-of-two
  extent per level, `native_vol.rs`). This is M6's 33x29x27.
- `e90122b8` (2026-11-17): the copy starts at BASE_LEVEL, MAX_LEVEL caps the
  descriptor's level count, and MIN/MAX_LOD go into SAMPLER_WORD0. Copies
  are kept per (base, held) and never rewritten while kick k or k+1 may read
  them. Pinned by `tests/es3_pvr_3d_lod.rs` and
  `tests/pvr_native3d_sampler_words.rs`.
- Board evidence: `e90122b8`'s message and `pvr-3d-native.md` ("Base
  level, MAX_LEVEL, MIN_LOD and MAX_LOD") record one AM62P run with native
  3D on. In it, `mipmap.3d` affine, basic and bias were right, and
  base_level, min_lod, max_lod, three max_level and five projected cases
  were wrong. The commit and console of that run are not in the tree. So it
  is not known whether its `basic.*_npot` cases already had `e179ad48`'s
  twiddle. `board-n3d` (plan #400, ready) re-measures `texture.mipmap.3d`
  at `472630df` with `DEQP_PVR_NATIVE_3D=1`.

**Expected on the native arm at the tip:** M5 28 Pass (board-seen), M6 12
Pass (board-n3d confirms the NPOT layout), M7 12 Pass (host-pinned only),
and M8 7 Pass with about 5 Fail until section 6 lands.

**Fix design: make native 3D the default.** In `src/shaderc/mod.rs:342`,
set `PVR_NATIVE_3D_DEFAULT` to 1, and update its doc comment (:329-338).
`pvr_set_native_3d(0)` and `DEQP_PVR_NATIVE_3D=0` keep the grid for A/B.
This is not a src/gl change (`src/gl/native_vol.rs` already exists and is
used behind the switch). Gate: `board-n3d` shows M5-M7 Pass and nothing
that passes on the grid arm fails on the native arm. That is the native
cube's step-C rule of 2026-09-25 (`src/shaderc/mod.rs:269-271`). The
pinned references, fixtures and tests the flip moves are the
`n3d-flip-impact` memo's list. The precompiled table is used only at the
switch's default, so its 3D programs must be rebuilt native in the same
commit, as step C did for the cube.

**Host test for the flip:**

- In `tests/pvr_native3d_sampler_words.rs:363`, change
  `assert_eq!(PVR_NATIVE_3D_DEFAULT, 0, "pvr_native_3d boots off")` to 1, so
  the boot state is pinned on.
- New `#[test]` there: with the switch at its boot value (never set, and
  `YAKOGL_PVR_NATIVE_3D` unset), a `#version 300 es` fragment shader with
  `texture(sampler3D, v)` compiles with record bit 2 (`native_cube & 4`).
  A draw of it binds GLES_TEXTYPE_3D_TWIDDLED with `addrmode_w` from
  GL_TEXTURE_WRAP_R = REPEAT and the 6-level chain of a 32x32x32 volume.
  This is the default-state version of
  `gl::draw::tests::a_native_3d_program_binds_*`.
- In the same test, a 33x29x27 volume's level-1 copy offset is
  `native_vol`'s physical-extent offset (pins M6's layout assumption, which
  board-n3d checks).

## 6. M8: `3d.projected.*` (open, src/shaderc/online/powervr)

**What dEQP draws.** `COORDTYPE_PROJECTED` does not call `textureProj`.
`TextureRenderer::renderQuad` multiplies the quad's positions by a per-vertex
w (`s_projections`, e.g. (1.2, 1.0, 0.7, 1.0)). The fragment shader is the
plain `texture(u_sampler, v_texCoord)` of a perspective-correct varying,
and the reference computes the LOD per pixel from the projected triangle
(`computeProjectedTriLod`, `lodBits` 6).

**Cause on the native arm.** This is the cause `2d.projected.*` had and
`pvr_proj_lod` fixed (`src/shaderc/mod.rs:430-441`; RESULTS.md 1m: one
fixed quad slot got a level the verifier rejects). The TPU's implicit LOD
takes one set of derivatives per 2x2 quad, and for a perspective varying
they change from pixel to pixel. `gradients_for`
(`src/shaderc/online/powervr/select.rs:166`) gives the exact plane
gradients only to `D2` (mode >= 1, :197) and to the native `Cube`
(mode >= 2, :198). `D3` falls to `_ => return None`. The native 3D arm
(:1535-1548) takes `smp_level(Implicit)` and never calls `plane_gradients`
(:660). That 2D and cube projected pass while 3D projected fails in part
(5 of 12 on the native board run, `e90122b8`) fits this. The 3D reference
is the same, with three coordinates. Which 5 failed is not recorded.

**Fix design** (src/shaderc/online/powervr/select.rs only):

1. `gradients_for` (:196-199): add
   `ir::TexDim::D3 if mode >= 2 && crate::shaderc::native_3d_enabled() => 3`.
   The guard is the switch, so the grid arm (which has no gradient form for
   its two-slice mix) is unchanged. So is every default-arm program while
   PVR_NATIVE_3D_DEFAULT is 0. `samples_with_plane_gradients` (:214) then
   also covers 3D, and the constant-W relink (`fs_w_constant`) drops the
   gradients for a program whose vertex shader writes a constant W, as it
   does for 2D and cube.
2. The native 3D arm (:1535): when `lod` is `Implicit` and
   `self.gradients_for(op).is_some()`, call `self.plane_gradients(coord, 3)?`
   and use `SmpLod::Gradients`, before the `SampleZ { vol: true }` group.
   This is the native cube's order at :1878-1885. The three-axis gradient
   layout is the one the arm's `textureGrad` already emits
   (`explicit_gradients(ddx, ddy, 3)`, pco_nir_tex.c:274-283: d/dx, d/dy
   per coordinate).
3. Doc comment of `pvr_proj_lod` (`src/shaderc/mod.rs:430-441`): "2 = for a
   cube and a native 3D texture too". There is no new default: mode 2 is
   already the default, and the 3D form exists only with native 3D on.

**Host test:** extend `tests/usc_limits.rs:6254`
`an_implicit_level_sample_of_a_varying_takes_its_plane_gradients` with two
GLSL ES 3.00 rows:

- `uniform highp sampler3D s; in vec3 v; o = texture(s, v);` compiled under
  `pvr_set_native_3d(1)` (restored to `PVR_NATIVE_3D_DEFAULT` after, as the
  test restores `pvr_proj_lod`). Use 3 coordinates, lowest mode 2. The
  test's model run then checks the six gradient words against
  (A_P - u A_Q)/Q and (B_P - u B_Q)/Q per coordinate, exactly as for the
  cube row.
- The same source under `pvr_set_native_3d(0)`: no gradients in any mode
  (lowest 3), which pins the grid arm as it is.

The test runs in its existing row (`host-test,gpu-powervr,shaderc-online`).
Board check: `3d.projected.*` 12 of 12 in a native-3D image.

## 7. What a run of the tip is expected to give

| build | M1-M3 (20) | M4 (1) | M5-M7 (52) | M8 (12) |
|---|---|---|---|---|
| `472630df`, default (plan #400's pin) | Fail (fixes not in the build) | Fail | Fail (grid) | Fail (grid) |
| `472630df`, `DEQP_PVR_NATIVE_3D=1` (board-n3d) | Fail | Fail | Pass expected | ~7 Pass / ~5 Fail |
| `2340445f0` or later, default | Pass expected | Fail | Fail (grid) | Fail (grid) |
| after section 4 + section 5 + section 6 | Pass | Pass | Pass | Pass |

All of these are predictions. Only the board can give the verdict, because
the host reads back black.
