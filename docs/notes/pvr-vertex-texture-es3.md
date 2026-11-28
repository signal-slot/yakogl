<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 89 dEQP-GLES3 texture.vertex fails (T-VERTEX), host cause

Plan #400, item cause-vertex. Host-only work at the integration tip
`ad2b6cc9`: no board run and no source change. The fails are from row 40
of `tools/cts/am62p-gles3-tree.txt` (the specification + swizzle + units +
vertex row). That row was last measured at build `ad2b5ed` (2026-10-13/14,
`docs/es3/am62p-gles3-notpass.tsv` header). The ES 2 side
(`texture.vertex.2d`, an arena refusal fixed in `a6fc6f14`) is
`docs/notes/es2-pvr-vertex-texture.md`. It is not this cause.

## Result in short

- **The vertex stage has no sampler defect of its own.** The GL core binds
  vertex-stage textures in the same per-stage loop as fragment ones
  (`src/gl/draw.rs:4931`, `p.rvs` / `vs_textures`). The native-copy
  decisions read that stage's own record (`sh.native_cube`,
  `draw.rs:5108`, `:5177`). The PowerVR back end builds the vertex
  descriptor block with the same `build_desc_block` as the fragment one
  (`src/pvr/backend.rs:10314-10332`). The online compiler's sampler arms do
  not depend on the stage (`src/shaderc/online/powervr/select.rs:1527-1560`).
  `textureLod` in the vertex shader keeps its explicit level on the native
  paths.
- **Every fail is a 2D array or a 3D case. 2D and cube have none** (78 of 78
  pass). The cause is the stacked-storage path that every 2D-array and 3D
  look-up took at `ad2b5ed`, in either stage:
  - **2D array (33): the stacked image** (`docs/es3/pvr-2darray.md`
    section 1). This is already documented. **Fixed at origin/main** by the
    native 2D array, default on since `02c5c9af` (2026-11-27). That commit is
    in the board pin `472630df`. The host shows that a vertex
    `sampler2DArray` now binds the twiddled layer-major copy, with
    per-axis wrap and the whole chain (evidence 2). Board confirmation is
    row 40 at `472630df` (item board-d, rows 39-45).
  - **3D (56): the grid of slices.** This is the cause #1285's note gives
    for `texture.filtering.3d` (`docs/notes/pvr-3d-mipmap-hang.md`,
    `docs/notes/pvr-3d-native.md`). **It is not fixed by the default at the
    tip**: `PVR_NATIVE_3D_DEFAULT = 0` (`src/shaderc/mod.rs:342`). A vertex
    `sampler3D` still binds the 512x512 slice grid with one level and
    CLAMP on R (evidence 2). With the switch on (`pvr_set_native_3d(1)`,
    the board's `DEQP_PVR_NATIVE_3D=1`), the vertex stage gets the
    TEXTYPE_3D twiddled copy with 7 levels, mipfilter on, and REPEAT /
    MIRROR on all three axes.
- **Fix:** none is needed in the vertex path. 2D array: already on main.
  3D: turn `PVR_NATIVE_3D_DEFAULT` to 1 (`src/shaderc/mod.rs`; the
  consequences are listed by item n3d-flip-impact and decided by
  n3d-default-decide). The measurement is the native-3D board arm
  (item board-n3d: rows 22-40 with `DEQP_PVR_NATIVE_3D=1`; row 40 holds
  `texture.vertex`). No `src/gl` change and no `src/pvr` change are needed.
  This is not a new defect.

## The case groups

dEQP source: `modules/gles3/functional/es3fVertexTextureTests.cpp` of
VK-GL-CTS `93bca01` (the commit `DEQP_WORK` pins). It was read from the
blobless clone in the dependency cache with
`git show 93bca01:modules/gles3/functional/es3fVertexTextureTests.cpp`.

What every case does (`:1307-1960`, groups `:1980-2122`):

- The vertex shader is
  `v_color = textureLod(u_texture, a_texCoord, u_lod)` with
  `highp sampler2D/samplerCube/sampler2DArray/sampler3D`. The fragment
  shader only writes `v_color`.
- The viewport is split into 4 cells: gradient and grid textures, each in a
  minifying cell (`texMinTransf` 2.1x or 2.2x, offset -0.3) and a magnifying
  cell (0.4x, offset 0.8). Each cell is one `glDrawElements` of a 127 x 127
  quad grid (`GRID_SIZE_2D_ARRAY`, `GRID_SIZE_3D`, `:93-96`).
- `u_lod` is dEQP's own `calculateLod` for the cell (`:1471-1490`), so the
  minifying cell asks for a level above 0 and the magnifying one below 0.
- 2D array: 128x128x8 `GL_RGB`/`GL_UNSIGNED_BYTE`, 8 levels (`:71-73`,
  `:1408-1450`). The coordinate is sheared by the layer count
  (`layerShearTransfData`, `:1517`), so every layer is reached.
- 3D: 64x64x64 `GL_RGB`, 7 levels (`:75-77`). It is sheared so that every
  slice is reached (`depthShearTransfData`, `:1852`).
- Compare: `pixelThresholdCompare` with the format threshold + 15 per channel
  (`compareImages`, `:627`). A wrong level, a wrong wrap or a wrong slice is far
  outside that.
- Groups: `filtering` = 6 min x 2 mag x 3 wraps (the same on every axis) =
  30 per target. `wrap` = `LINEAR_MIPMAP_LINEAR`/`LINEAR` with S x T (x R
  for 3D): 9 per target, 27 for 3D.

The AM62P not-pass lines, by target and group (all "Image comparison
failed"):

| Target | Group | Mustpass | Fail | Pass | The passing cases |
|---|---|---|---|---|---|
| 2d | filtering + wrap | 39 | 0 | 39 | all |
| cube | filtering + wrap | 39 | 0 | 39 | all |
| 2d_array | filtering | 30 | 24 | 6 | `nearest_nearest_clamp`, `linear_linear_clamp`, `nearest_mipmap_{nearest,linear}_{nearest,linear}_clamp` |
| 2d_array | wrap | 9 | 9 | 0 | none (all are LINEAR_MIPMAP_LINEAR) |
| 3d | filtering | 30 | 29 | 1 | `linear_linear_clamp` |
| 3d | wrap | 27 | 27 | 0 | none |
| **total** | | **174** | **89** | **85** | |

The same split by filter and wrap:

- 2D array, failing: every `repeat` and `mirror` case (20 filtering + 8
  wrap), and the `linear_mipmap_*` clamp cases (4 filtering +
  `wrap.clamp_clamp`). Passing: clamp only, with a minifying filter that
  reads level 0 (`nearest`, `linear`) or picks one level per pixel and
  reads it NEAREST on the layer grid (`nearest_mipmap_*`). This is the
  fingerprint of the stacked image: T wraps over the whole stack, not
  inside the layer, and a mip level of the stack is a stack of rounded
  smaller layers that LINEAR taps cross (`docs/es3/pvr-2darray.md`
  section 1).
- 3D, failing: everything but `linear_linear_clamp`. Each failing
  setting goes outside what the grid keeps (below):
  - any mipmap filter: the grid has level 0 only;
  - any `repeat` or `mirror` on S, T or R: the grid clamps all three;
  - NEAREST on min or mag: the grid always mixes two slices on R.

  `linear_linear_clamp` asks only for what the grid does: level 0,
  trilinear, clamp.

## The cause in the code

The 3D grid arm (default at the tip, `select.rs:1661-1760`), which applies
to every stage:

- `TexLod::Lod(_)` becomes `(SmpLod::Replace, level 0)`
  (`select.rs:1713-1716`: "an explicit level samples level 0
  (pvr_volLod)"). The vertex shader's `u_lod` is dropped, so all 25
  mipmapped 3D cases read level 0.
- `vol_uv` (`select.rs:4434-4462`) clamps `(u, v)` to
  `[0.5/sz, 1 - 0.5/sz]` of the slice before it addresses the grid, so S
  and T never wrap. The descriptor's third address mode is CLAMP_TO_EDGE
  for anything but `GLES_TEXTYPE_3D_TWIDDLED` (`src/pvr/backend.rs:7207`),
  and the slice index is clamped to `[0, layers - 1]` in the shader.
- A float look-up always takes the two slices around `c.z * layers - 0.5`
  and mixes them (`select.rs:1703-1760`; only integer textures take the
  nearest slice). A NEAREST filter on R is therefore not honoured.

The 2D-array stacked arm (`array_uv`, the off-arm of `pvr_native_array`) is
`docs/es3/pvr-2darray.md` section 1. Since `02c5c9af` it is reached only with
`pvr_set_native_array(0)` / `DEQP_PVR_NATIVE_ARRAY=0`.

The native arms that replace both are stage-independent: native 3D
`select.rs:1527-1560` (`SampleZ { vol: true }` + `smp` with `smp_level`, so
`textureLod` keeps its level); native 2D array, `tao` layer address
(`docs/es3/pvr-2darray.md`). The draw's copies: `draw.rs:5108-5126` (array)
and `draw.rs:5177-5200` (3D), inside the per-stage loop of `draw.rs:4931`.

## Evidence

### 1. The existing pins pass at the tip

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_array_native_cases
test vertex_2d_array_stacked_control ... ok
test vertex_2d_array_native_looks_up_the_middle_layer ... ok
(+5 more) test result: ok. 7 passed; 0 failed
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_native3d_sampler_words
test the_default_is_the_grid_of_slices_without_wrap_r_or_a_level_range ... ok
test native_3d_puts_wrap_r_in_the_samplers_third_address_mode ... ok
test native_3d_carries_the_level_range_and_the_lod_clamp ... ok
test result: ok. 3 passed; 0 failed
```

`vertex_2d_array_native_looks_up_the_middle_layer` runs a vertex
`textureLod` on a 2D array through the byte model (`tests/usc_exec`) and
reads each level's own layer. The 3D sampler-word pin uses a fragment
stage. Evidence 2 is its vertex counterpart.

### 2. dEQP's vertex shaders through the GL core to the PowerVR descriptor

This was a scratch host test, not committed. It is
`tests/pvr_native3d_sampler_words.rs`'s harness, with a stand-in back end
that keeps `d.vs_textures` (and asserts `num_textures == 0`) and the
vertex record's `native_cube`. It packs each vertex bind with
`pvr::backend::texture_descriptor`. It compiles dEQP's own vertex and
fragment shaders (above), uploads dEQP's textures (2D array 128x128x8,
3D 64x64x64, `GL_RGB`/`GL_UNSIGNED_BYTE`, every level by `glTexImage3D`)
and draws one triangle with `u_lod` set. It ran with the switches at
their boot defaults, both off, and both on:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test scratch_vtx_es3 -- --nocapture
defaults: native_array=1 native_3d=0
```

The output, cut to the fields that matter. `type_` 2 = 2D_ARRAY (stacked),
3 = 3D (grid), 5 = 2D_ARRAY_TWIDDLED, 6 = 3D_TWIDDLED. IW0 textype
(texstate.xml): 1 = 2D, 2 = 3D, 4 = STRIDE. The addrmode values are
0 REPEAT, 1 FLIP (mirror), 2 CLAMP_TO_EDGE. maxlod is u4.6:

| Arm | Case | vs `native_cube` | bind type, w x h, mip_levels, mip_filter | IW0 textype | SW0 mipfilter, maxlod | addr u/v/w |
|---|---|---|---|---|---|---|
| default | 2d_array LML/L repeat, lod 2.3 | 2 | 5, 128x128, 8, 2 | 1 (2D, depth 8) | 1, 448 (7.0) | 0/0/2 |
| default | 2d_array N/N mirror, lod -1 | 2 | 5, 128x128, 8, 0 | 1 | 0, 0 | 1/1/2 |
| default | 3d LML/L repeat, lod 2.3 | **0** | **3, 512x512, 1, 0** | **4 (STRIDE)** | **0, 0** | 0/0/**2** |
| default | 3d N/N clamp, lod -1 | 0 | 3, 512x512, 1, 0 | 4 | 0, 0 | 2/2/2 |
| off | 2d_array LML/L repeat, lod 2.3 | 0 | 2, 128x**1024**, 8, 2 | 4 (STRIDE) | 1, 448 | 0/0/2 |
| off | 3d LML/L repeat, lod 2.3 | 0 | 3, 512x512, 1, 0 | 4 | 0, 0 | 0/0/2 |
| on | 2d_array LML/L repeat, lod 2.3 | 2 | 5, 128x128, 8, 2 | 1 | 1, 448 | 0/0/2 |
| on | 3d LML/L repeat, lod 2.3 | **4** | **6, 64x64, 7, 2** | **2 (3D, depth 64)** | **1, 384 (6.0)** | **0/0/0** |
| on | 3d N/N clamp, lod -1 | 4 | 6, 64x64, 1, 0 | 2 | 0, 0 | 2/2/2 |

The full run also had `2d_array L/L clamp lod 1` and `3d L/L clamp lod 1`
in each arm. They are consistent with the rows above, and the test
passed. The 2D array's third address mode is CLAMP in every arm, which
is right (the layer is clamped, not wrapped).

Reading: at the tip's defaults the vertex 2D array is the native array:
one layer's size, depth 8, the whole chain, and the dEQP wrap on S and T.
That is exactly what the fragment stage gets, so the 33 2D-array fails
are expected to pass at `472630df`. The vertex 3D is the 8 x 8 grid of
64x64 slices (512x512): one level, no mip filter, CLAMP on R. The shader
also forces level 0 and clamps S and T. These are the 56 3D fails. With
native 3D on, the vertex stage gets the full volume with its levels, the
LOD range and every wrap.

The host OSAL renders nothing: these are the words and the records, not
pictures. The picture is the board's (row 40).

### 3. Build note

At `ad2b6cc9` `cargo` refused to load the workspace:
`capi/Cargo.toml:85: duplicate key vivante-st` (lines 41 and 85 both
declare `vivante-st = ["yakogl/vivante-st"]`). The runs above used a local
copy with line 85 removed. That edit was not committed. This is reported
separately. It is not part of this cause.

## Proposed fix and its paths

1. 2D array (33 cases): nothing to do. It is fixed on main by `02c5c9af`
   (`PVR_NATIVE_ARRAY_DEFAULT = 1`, `src/shaderc/mod.rs:314`). Confirm on
   the board with row 40 at `472630df` (board-d).
2. 3D (56 cases): `PVR_NATIVE_3D_DEFAULT = 1` in `src/shaderc/mod.rs:342`.
   The change itself is outside `src/pvr/**` and `src/gl/**`. It moves
   pinned references, fixtures and tests (item n3d-flip-impact),
   including `tests/pvr_native3d_sampler_words.rs`'s
   `the_default_is_the_grid_of_slices_without_wrap_r_or_a_level_range`,
   which says it is "the one to change". Before the flip, the board arm
   `DEQP_PVR_NATIVE_3D=1` on row 40 (item board-n3d) should show the 56
   pass. A 3D vertex case that still fails there would be a new native-3D
   defect: look first at `select.rs:1527-1560` (`smp_level` with an
   explicit level in a vertex `smp`) and the 3D copy `gl::native_vol::copy`
   for `GL_RGB` (3-byte texels).
3. No change to the grid arm is proposed. Honouring `textureLod`, S/T
   wrap and NEAREST-on-R there would mean reproducing the TPU in the
   shader. The native 3D path does this already.
