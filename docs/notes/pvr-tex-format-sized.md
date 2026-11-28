<!-- SPDX-License-Identifier: Apache-2.0 -->
# The 158 AM62P `texture.format` fails (T-FORMAT): groups, causes, fixes

Cause row T-FORMAT of `tools/cts/am62p-notpass-causes.py` takes every `Fail`
line of `docs/es3/am62p-gles3-notpass.tsv` under `texture.format.` (158 lines;
`python3 -I tools/cts/am62p-notpass-causes.py --list
docs/es3/am62p-gles3-notpass.tsv`). This note splits them by format and
target, gives each group's cause at the build that failed, and says which
commit on main fixes it. HOST-ONLY: nothing here was run on the board.

**Result.** Each of the 158 lines belongs to one of two causes, and main
fixes both:

- 150 lines are "Failed to compile shader program at
  glsTextureTestUtil.cpp:411". These are the 25 integer formats on the cube,
  2D-array and 3D targets. The PowerVR selector refused integer cube, 3D and
  2D-array samplers. Fixed at or before `381b0f09`.
- 8 lines are "Image comparison failed". These are `rgba32f` on all four
  targets. GL_RGBA32F was stored as halves. Fixed at `ecd9743e`.

No group is left without a fix, and no src/gl change is needed. What is
still open is the board check of tree row 39 at a build that has both
fixes (section 5).

## 1. Data

- **Cases.** The 158 `Fail` lines of `texture.format.sized.{2d,2d_array,3d,cube}`.
  The other 48 `texture.format` lines are `NotSupported`: 32 under
  `compressed` (ETC2/EAC, cause NS-ETC) and 16 `srgb_r8` / `srgb_rg8` (cause
  NS-SRGB-R8RG8). They are not T-FORMAT.
- **Build.** `texture.format.*` is in tree row 39 of
  `tools/cts/am62p-gles3-tree.txt` (counted from 0, the line with
  `texture.filtering.cube.*,texture.format.*,texture.mipmap.*,...`). The
  TSV header puts rows 22-45 at build `ad2b5ed`, run 2026-10-13/14. That
  commit is not in this repository's history. The history's root is
  `381b0f09` (2026-10-27), and `ad2b5ed` is older.
- **Code cited.** Line numbers are at origin/main `3df3adfc` (2026-11-28).
  Its `src/pvr`, `driver/src/pvr`, `src/shaderc` and `src/gl` are byte for
  byte those of the integration tip `5f781c49` (`git diff --stat 5f781c49
  3df3adfc -- src/pvr driver/src/pvr src/shaderc src/gl` is empty).
- **What dEQP's cases do.** The rest of this note relies on this.
  - Upload: each case uploads level 0 only.
  - Sampling: it samples with NEAREST (`docs/notes/etc2-eac.md` section 3:
    `texture.format` uses NEAREST and a 2/255 threshold). The sampler type
    comes from the format (`getSamplerType`; `texture-failure-map.md` 2.1,
    row `format.sized`), so an integer format is read through an
    `isampler*` or `usampler*`.
  - Output: the shader writes `vec4(texel) * u_scale + u_bias` to the RGBA8
    default framebuffer.
  - Levels and filters: there is no mip chain, so the level causes K3, K4
    and K5 (`texture-failure-map.md` 2.2.1) do not apply. No integer format
    is filtered.
  - The level-0 and NEAREST shape is recalled from dEQP's
    `es3fTextureFormatTests.cpp`, not re-read: there is no dEQP source on
    this host. The 2/255 threshold and NEAREST come from `etc2-eac.md`.
- **A control in the same run.** At `ad2b5ed` every integer format on the
  plain **2D** target passed. The TSV has no `texture.format.sized.2d.*`
  integer line; its only `sized.2d` fails are `rgba32f_pot` and
  `rgba32f_npot`. Those 2D cases go through the same upload conversion, the
  same TPU format words and the same output conversion (including the
  32-bit `r32ui` .. `rgba32ui` values through `u2f`). The 150 failures
  differ from them only in the sampler's dimension.

## 2. The per-format table

The counts are lines of the TSV. "pot" and "npot" count as 2 lines per
format and target.

| group | formats | 2d | 2d_array | 3d | cube | lines | message at `ad2b5ed` | state |
|---|---|---:|---:|---:|---:|---:|---|---|
| F1 | 4-component integer: `rgba8i`, `rgba8ui`, `rgba16i`, `rgba16ui`, `rgba32i`, `rgba32ui` | 0 | 12 | 12 | 12 | 36 | Failed to compile (glsTextureTestUtil.cpp:411) | fixed at `381b0f09` (or earlier) |
| F2 | 3-component integer: `rgb8i`, `rgb8ui`, `rgb16i`, `rgb16ui`, `rgb32i`, `rgb32ui` | 0 | 12 | 12 | 12 | 36 | Failed to compile | fixed at `381b0f09` (or earlier) |
| F3 | 1- and 2-component integer: `r8i`, `r8ui`, `r16i`, `r16ui`, `r32i`, `r32ui`, `rg8i`, `rg8ui`, `rg16i`, `rg16ui`, `rg32i`, `rg32ui` | 0 | 24 | 24 | 24 | 72 | Failed to compile | fixed at `381b0f09` (or earlier) |
| F4 | packed integer: `rgb10_a2ui` | 0 | 2 | 2 | 2 | 6 | Failed to compile | fixed at `381b0f09` (or earlier) |
| F5 | `rgba32f` | 2 | 2 | 2 | 2 | 8 | Image comparison failed | fixed at `ecd9743e` |
| | **total** | 2 | 52 | 52 | 52 | **158** | 150 compile + 8 image | |

Checks on the totals:

- By group: 36 + 36 + 72 + 6 + 8 = 158, the T-FORMAT row.
- By target: 2 + 52 + 52 + 52 = 158.
- By message: 150 compile + 8 image. `grep -P
  "^Fail\tdEQP-GLES3.functional.texture.format" docs/es3/am62p-gles3-notpass.tsv`
  gives 158 lines with these two messages and no others.

F1-F4 are split by how the GL core stores the format, which is also the
part a regression would show up in. They share one cause.

## 3. Causes and fixes

### 3.1 F1-F4 (150): the integer cube, 3D and 2D-array sampler was refused

**Cause at `ad2b5ed`.** The message comes from dEQP's program build in
`glsTextureTestUtil.cpp:411`. The PowerVR online compiler refused the
program: the selector turned down `isamplerCube`, `usamplerCube`,
`isampler3D`, `usampler3D`, `isampler2DArray` and `usampler2DArray`
look-ups, and selected an integer `smp` only for the 2D dimension. It is
the same refusal as K1 and K1b of `texture.specification`
(`texture-failure-map.md` 2.2.1). The 2D control in section 1 shows that
nothing past the compiler failed for these formats.

**Fixed at `381b0f09` (or earlier)**, in `src/shaderc/online/powervr/select.rs`.
The integer selection was already at the history's root, so the fixing
commit is between `ad2b5ed` and `381b0f09` and is not in this repository:

- `smp_type` (`select.rs:153`): a sample takes the sampler's return type
  (S32 for `isampler*`, U32 for `usampler*`), and an integer `smp` is built
  without `fcnorm`.
- The 3D slice-grid arm (`select.rs:1700-1745`; `PVR_NATIVE_3D_DEFAULT` is 0,
  `src/shaderc/mod.rs:342`) reads an integer texture at its nearest slice,
  `floor(clamp(r * depth, 0, depth - 1))`. It does not shift by half a slice
  and does not mix two slices (`let integer = ret != ir::ScalarType::F32`,
  `:1703`).
- The 2D-array arm picks the layer in integers (`array_layer_addr`,
  `select.rs:4270`). Native 2D arrays are the default since `02c5c9af`
  (2026-11-27; `PVR_NATIVE_ARRAY_DEFAULT = 1`, `src/shaderc/mod.rs:314`).
- The native cube (`PVR_NATIVE_CUBE_DEFAULT = 1`, `src/shaderc/mod.rs:282`)
  samples integer texels from the copy that `gl::native_cube` makes.
- Unit test `integer_cube_3d_and_array_lookups_select_as_integer_smp`
  (`select.rs:6574`) checks one `smp` of the integer type per look-up for
  every target. It is present at `381b0f09`.

**Storage at the tip.** No change since `ad2b5ed` is needed for these
cases; this lists why they would draw right:

- F1. RGBA8/16/32 I and UI are their own classes. `es3_formats.rs` rows
  `:167-172` give the classes, and `es3_tex_format` (`src/pvr/backend.rs:1436`)
  maps them to the TPU formats U8U8U8U8 / S8S8S8S8 .. U32U32U32U32 /
  S32S32S32S32 (`TEXF_RGBA8UI` .. at `backend.rs:1412-1417`), all in
  `PVR_SAMPLED_TEXFMTS` (`src/backend.rs:215`).
- F2 and F3. The RGB, R and RG integer formats are stored widened to the
  4-component class of the same width and sign (`U8_ST` .. `I32_ST`,
  `R8U_ST` .. `RG32I_ST`, `es3_formats.rs:83-102`; the 1- and 2-component
  classes are not in `PVR_SAMPLED_TEXFMTS`, so the second, wide entry is
  taken). `masked` (`es3_formats.rs:580`) fills the missing components
  with (0, 0, 0, 1) at upload, so the texel the TPU reads is the GL one.
  `83f267dd` (2026-11-27, `int_store_alpha`, `backend.rs:1480`) adds
  SWIZ.ONE for the alpha of such a store. Cases that only upload and
  sample do not need it, because the upload already wrote 1, but it does
  not hurt them. It is for stores that a draw or a NULL upload fills.
- F4. `rgb10_a2ui` is one 10/10/10/2 dword (`es3_formats.rs:139-147`, encode
  at `:677`). It is read as the TPU format A2R10B10G10 without `fcnorm`
  (`GLES_TEXFMT_RGB10A2UI => (TEXF_RGB10A2, ..)`, `backend.rs:1441`), which
  is in the sampled mask (`src/backend.rs:222`).

**Host evidence at the tip.**

- `tests/pvr_int_targets_deqp.rs` (`ad589d91`, 2026-11-25) compiles and
  links dEQP's exact fragment shader for all six integer sampler types
  under three switch settings: the defaults, every native target on, and
  every native target off. It does the same for the vertex stage with
  `textureLod`. With the native targets on, it checks that the `smp` has
  the native `dmn`.
- `tests/pvr_int_cube.rs` (`0ec67c94`) checks the texel placement and the
  NEAREST look-up of integer cubes (R8UI, R32UI, RG16I, RGBA8I, RGBA16UI,
  RGBA32I) on both the stacked and the native cube.
- The host dEQP-GLES3 run at `9390b7d2` (`docs/es3/host-gles3-part-powervr.tsv`,
  section `notpass`) has no compile line and no `gl-error` line for any
  `texture.format.sized` case. Its only such lines are the 16 sRGB R8/RG8
  `NotSupported`. The host keeps the command stream and reads black pixels,
  so an image verdict there means nothing. What it shows is that the 150
  programs now build.

**Fix design.** None needed. **Host test.** The three pins above.

### 3.2 F5 (8): GL_RGBA32F stored as halves

**Cause at `ad2b5ed`.** The GL core kept GL_RGBA32F as RGBA16F halves on the
PowerVR. `rgba32f_own_class` (`src/gl/texture.rs:6618`) opted in only
`gpu-vivante` (see the diff of `ecd9743e`). dEQP's float values go beyond
the half maximum 65504 (the `ecd9743e` message gives +-1e5), so those
texels became infinity and the image comparison failed on all four
targets.

The 2D `rg32f`, `rgb32f` and `r32f` cases passed in the same run because
they take the `es3_formats` row's storage list `F32_ST =
[GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA16F]` (`es3_formats.rs:82`). That
list starts with the 32-bit class, which the PowerVR's sampled mask already
held (`src/backend.rs:230`). Only the GL_RGBA32F name took the halves path
(`ecd9743e`'s rewritten comment above `rgba32f_own_class`).

**Fixed at `ecd9743e`** (2026-11-23). `rgba32f_own_class` now includes
`gpu-powervr` (`texture.rs:6621`), so GL_RGBA32F is stored as
`GLES_TEXFMT_RGBA32F` and read as F32F32F32F32 (`TEXF_RGBA32F`,
`backend.rs:1418`). `ecd9743e` is an ancestor of the board pin `472630df`.

**Host test.** `tests/es3_rgba32f_pvr.rs` (`ecd9743e`, extended in `6c262deb`):

- `rgba32f_samples_full_floats` checks the bound class and the uploaded
  floats (1e5 and -1e5 among them) for 2D, 2D array, stacked cube and 3D.
- `rgba32f_native_cube_copy_carries_full_floats` does the same for the
  native cube's copy.

The 2D-array shape runs at the default switch, which is native since
`02c5c9af`.

**Fix design.** None needed.

## 4. src/gl

None. Both fixes are on main. F5's fix was a src/gl change
(`src/gl/texture.rs` `rgba32f_own_class`, `ecd9743e`), and it has landed.

## 5. What is left: the board check, and what to do if it fails

T-FORMAT is closed once tree row 39 runs on the AM62P at a build that
contains `ecd9743e` (for example the pin `472630df`, item board-d). The
expected result is that the 158 lines become Pass and the 48
`NotSupported` lines stay. Row 39 also holds `texture.filtering.cube`,
`texture.mipmap`, `texture.shadow` and `texture.size`; their causes are
separate.

If some lines still fail on the board, read the group and the message
against this table. Each row is a contingency, not a known defect:

| still failing | most likely place | design | host test |
|---|---|---|---|
| F1-F4 on `cube` only, image | the native cube copy of the 4-, 8- and 16-byte integer stores (`gl::native_cube`; every F1-F3 format is stored 4-component, so those are the only widths) | compare against the stacked cube (`DEQP_PVR_NATIVE_CUBE=0` image, or `pvr_set_native_cube(0)`). If only native fails, fix the copy's texel size and twiddle for that store in the copy code | extend `tests/pvr_int_cube.rs`. It already covers the three store widths (R8UI and RGBA8I 4 bytes, RG16I and RGBA16UI 8, R32UI and RGBA32I 16); add an RGB format (F2) and `rgb10_a2ui` (F4) |
| F1-F4 on `3d` `_npot` only, image | the slice grid of an NPOT volume (`vol_uv` / `vol_cells`, `select.rs` near `:1737`) | run the native-3D image (board-n3d, `DEQP_PVR_NATIVE_3D=1`). If native passes, the fix is the `PVR_NATIVE_3D_DEFAULT` flip (a shared switch in `src/shaderc/mod.rs:342`, decided in n3d-default-decide), not src/pvr | `tests/es3_pvr_3d.rs` with an `isampler3D` 11x31x7 case |
| F1-F4 on `2d_array`, image | the native array copy (default since `02c5c9af`) | compare against the stacked arm (`DEQP_PVR_NATIVE_ARRAY=0`). Fix in the native array copy for that texel width | `tests/pvr_array_native_cases.rs` with integer formats |
| F5 on any target, image | `ecd9743e` is missing from the build, or the TPU's F32F32F32F32 read | check the build's ancestry first. Otherwise compare the board picture with `es3_rgba32f_pvr`'s texels | `tests/es3_rgba32f_pvr.rs` |
| a compile line again | a regression in `select.rs` | `tests/pvr_int_targets_deqp.rs` and the `select.rs:6574` unit test must catch it on the host first | as named |

## 6. Corrections to the documents this note relies on

- `src/backend.rs:207-209` (the doc comment of `PVR_SAMPLED_TEXFMTS`) still
  says that "the other integer sampler dimensions are refused by name at
  compile time". That has not been true since `381b0f09` (section 3.1).
  This note does not edit that comment; the work is in the report.
- `am62p-notpass-causes.py` describes T-FORMAT as "integer / sized formats
  sampled or uploaded wrongly". At `ad2b5ed` none of the 158 lines was an
  upload or sampling error: 150 were compile refusals and 8 were the
  RGBA32F half storage.
