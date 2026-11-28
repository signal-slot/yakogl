<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR integer cube, 2D-array and 3D samplers (AM62P K1, K1b)

Causes K1 and K1b of `docs/es3/am62p-gles3-notpass.tsv` (texture-failure-map.md
section 2.2.1): 204 dEQP-GLES3 `texture.specification` cases that failed
with "Compile failed at sglrGLContext.cpp:863" because the PowerVR online
compiler refused an integer sampler on a cube, a 2D array or a 3D texture.
This note gives the cases, the cause with file:line references, what the
hardware needs for such a sample, what is still to do and the expected
outcome.

Commits named here:

- `700338349` (2026-10-11): the last commit before the measured build that
  this clone has. The board build `ad2b5ed` (2026-10-13/14, rows 22-45 of the
  TSV) is in none of its refs. The history between 2026-10-11 and the 0.1.0
  root is squashed.
- `381b0f09e` (2026-10-27): the squashed YakoGL 0.1.0 root.
- `14e7a7421`: origin/main when this note was written. `src/`, `driver/` and
  `third_party/` are byte-identical to the integration tip `5f781c491`.
  `src/shaderc/online/powervr/select.rs` is byte-identical to `472630df`,
  the build of the current AM62P board sessions. Every line reference below
  without a commit is at `14e7a7421`.

## 1. The cases

`python3 -I tools/cts/am62p-notpass-causes.py docs/es3/am62p-gles3-notpass.tsv`
gives `K1 200` and `K1b 4`. The rules are at
`tools/cts/am62p-notpass-causes.py:49-54`. All 204 are `Fail`, "Compile
failed at sglrGLContext.cpp:863". They come from TSV rows 22-45 (build
`ad2b5ed`), tree row 40 (`texture.specification.*`).

| family (`texture.specification.`) | target | cases | dEQP shape (es3fTextureSpecificationTests.cpp, VK-GL-CTS 93bca01) |
|---|---|---:|---|
| `basic_teximage2d.<int>_cube` | cube | 25 | 64x64, full chain (7 levels), NEAREST_MIPMAP_NEAREST / NEAREST, MAX_LEVEL 6 (:392-396, :3900-3910) |
| `basic_texsubimage2d.<int>_cube` | cube | 25 | the same, filled by glTexSubImage2D |
| `teximage2d_pbo.<int>_cube` | cube | 25 | 64x64, one level, from a PBO (:2487-2499, :4040-4052) |
| `texsubimage2d_pbo.<int>_cube` | cube | 25 | the same, glTexSubImage2D from a PBO |
| `teximage3d_pbo.<int>_2d_array` | 2D array | 25 | 11x20x8, one level, PBO (:2559-2575, :4500-4508) |
| `teximage3d_pbo.<int>_3d` | 3D | 25 | 11x20x8, one level, PBO (:2634-2650) |
| `texsubimage3d_pbo.<int>_2d_array` | 2D array | 25 | 26x25x10, sub-box 23x19x8 at (1, 2, 0), one level (:4650-4700) |
| `texsubimage3d_pbo.<int>_3d` | 3D | 25 | the same box, 3D |
| **K1** | | **200** | |
| `random_teximage2d.cube_0`, `_1`, `_3`, `_7` (K1b) | cube | 4 | power-of-two side `1 << rnd(2..8)`, full chain, levels uploaded in random order (:946-1000, :3933-3942); the four whose random format is integer |
| **K1 + K1b** | | **204** | |

`<int>` is one of the 25 integer formats `r|rg|rgb|rgba` x `8|16|32` x `i|ui`
plus `rgb10_a2ui` (the rule's `INT`, `am62p-notpass-causes.py:37`). So K1 is
not cubes only: there are 100 cube cases, 50 2D-array cases and 50 3D cases.
The causes table's text ("integer cube sampler") describes only the first
100.

Every case samples through dEQP's sglr shaders (`es3fFboTestUtil.cpp`
`TextureCubeShader`, `Texture2DArrayShader`, `Texture3DShader`): one
`texture()` call on an `isampler*` or `usampler*`, the result converted with
`vec4(...) * u_scale + u_bias` into a `vec4` output. The filters are always
NEAREST (`:392-393`, `:508-509`, `:613-614`): no case filters an integer
texture.

**The 119 other compile refusals.** The TSV has 323 "Compile failed" lines
under `texture.specification`. Only 204 are K1/K1b. The other 119 are the
integer formats of K3 (25: `texstorage2d.format.<int>_cube`), K4 (36:
integer `basic_teximage3d.*_2d_array`, `texstorage3d.format.*_2d_array`) and
K5 (58: integer `basic_teximage3d`, `basic_texsubimage3d`,
`texstorage3d.format` `*_3d`). The causes script files them under their
level and layout cause (first match wins). This is the 119-case gap that
texture-failure-map.md section 2.2.1 leaves open: those cases had the same
compiler refusal as K1, plus the K3/K4/K5 defect behind it.

## 2. The cause

dEQP's `sglr::GLContext::createProgram` raises "Compile failed" when
`glu::ShaderProgram::isOk()` is false, that is, when the compile or the link
fails (`framework/opengl/simplereference/sglrGLContext.cpp:859-864`, VK-GL-CTS
93bca01). On the PowerVR the shader is refused by instruction selection in
the online back end:

At `700338349`, `src/shaderc/online/powervr/select.rs`:

- `:1112-1121`, the cube arm (stacked image and the native-cube branch at
  `:1128`) matches only `ret: ir::ScalarType::F32` (`:1116`).
- `:1089-1097`, the stacked 2D-array arm matches only `ret: F32` (`:1093`),
  and so does the 2D-array texelFetch arm (`:931-935`). That commit has no
  native-array arm.
- `:1016-1024`, the 3D grid arm (glsl2usc's `pvr_vol`) matches only
  `ret: F32` (`:1020`), and so does the 3D texelFetch arm (`:974-978`).
- `:1195-1197`, the catch-all: `ir::Op::Tex { .. } => Err(SelectError::Unsupported("a
  texture fetch with an integer result on a cube, a 3D texture or an array,
  ..., is not selected yet"))`.
- `:4991-5011`, a unit test (`each_refusal_names_the_question_behind_it`,
  `:4982`) pins `isamplerCube` as "still refused".

So any `isamplerCube`, `usamplerCube`, `isampler2DArray`, `usampler2DArray`,
`isampler3D` or `usampler3D` look-up fell to the catch-all. The front end and
the hardware were not the problem: only the 2D arms (`:866-870`,
`:898-902`) took `I32 | U32`, and the integer 2D cases of the same families
(`basic_teximage2d.<int>_2d`, ...) are not in the TSV, so they passed on the
board at `ad2b5ed`.

**Fixed at the tip.** At `381b0f09e` the cube arm (`:1535-1543`), the stacked
2D-array arm (`:1512-1517`), the native-array arm (`:1175-1176`) and the 3D
grid arm (`:1415-1419`) already bind any `ret`. The catch-all no longer names
integer results (`:1628-1630`). The native 3D arm (`:1292-1296`) still took
`F32` only. `e179ad482` (2026-11-05, "integer and textureGrad volumes") added
integers there. At `14e7a7421`:

- cube: `select.rs:1858-1867` binds `ret`. On the native cube (the default,
  `src/shaderc/mod.rs:282` `PVR_NATIVE_CUBE_DEFAULT = 1`) `:1877-1892`
  emits `SampleZ` and `Sample { cube: true }` with `ty: smp_type(ret)`. On
  the stacked image `:1893-1909` samples through `cube_uv` (`:4037`).
- 2D array: the native arm `select.rs:1397-1398` (the default,
  `src/shaderc/mod.rs:314` `PVR_NATIVE_ARRAY_DEFAULT = 1`) is an ordinary
  2D `smp` with the layer's address override (`array_layer_addr`, `:4270`).
  The stacked arm is `:1812-1842`.
- 3D: `src/shaderc/mod.rs:342` `PVR_NATIVE_3D_DEFAULT = 0`, so the grid arm
  `select.rs:1678-1760` takes the case. Its integer branch (`:1703-1704`,
  `:1737-1757`) reads the nearest slice whole, with no half-slice shift and
  no mix, through `vol_uv` (`:4434`). The native arm `:1526-1535` takes
  every `ret` when native 3D is on.
- `smp_type` (`select.rs:153-159`) gives `S32` or `U32`.
  `assemble.rs:272-273` turns a non-F32 sample into `SmpForm.integer`, and
  `fragment.rs:328` `fcnorm = !integer` writes the `smp` `fcnorm` bit
  (`isa_tables.rs:347`). This is how Mesa's pco builds an integer
  destination (the comments cite pco_nir_tex.c:236, `.fcnorm =
  base_type(dest_type) == float`. pco is not vendored in `third_party/`).
- The catch-all `select.rs:1985-1987` now refuses only a projective cube or
  array and an offset on a cube. GLSL ES 3.00 has no such functions.

**Measured on the host at `5f781c491`** (sources identical to `14e7a7421`):
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_sglr_shaders --test pvr_int_cube`
passes 1 + 4 tests. `tests/pvr_sglr_shaders.rs` builds dEQP's own
`TextureCubeShader`, `Texture2DArrayShader` and `Texture3DShader` texts for
float, int and uint samplers and every output type, and compiles and links
them (`texture_cube_array_and_3d_shaders_build`, 27 programs). This is
exactly the K1/K1b shader set. `tests/pvr_int_targets_deqp.rs` (`ad589d91f`)
pins the same sampler types under the default switches, with every native
target on and with every one off.

## 3. What the hardware needs

These are the hardware facts the sample relies on, all in place at
`14e7a7421`.

- **An unnormalised (integer) sample.** The USC `smp` with `fcnorm` clear
  returns the texel's integers into the destination registers
  (`fragment.rs:301-303`, `:328`). The TPU filter must not blend integers.
  ES 3.0 3.8.13 makes an integer texture with a LINEAR filter incomplete, and
  every K1 case sets NEAREST. `tex_desc_sampler_lod`
  (`src/pvr/backend.rs:7237`, `:7283-7285`) packs SAMPLER_WORD0
  `magfilter`/`minfilter` POINT for NEAREST and `mipfilter` false for
  `*_MIPMAP_NEAREST` (texstate.xml `SAMPLER_WORD0` `:354-356`, `FILTER` `:89-93`,
  third_party/mesa-26.2.1/src/imagination/csbgen/rogue/texstate.xml). No
  forced POINT for integer formats is needed for these cases (see section 6).
- **Face selection.** On the native cube the TPU selects the face from the
  three-component direction (`smp` `dmn` 3d, `SampleZ` carries z). The image
  is IMAGE_WORD0 `textype = CUBE` (texstate.xml `:252`, `:289`) over the
  twiddled face-major copy (`gl::native_cube`), with IMAGE_WORD1 `depth = 5`
  and the whole chain's level count, so the TPU finds the face stride
  (`src/pvr/backend.rs:7010-7030`). On the stacked image the shader selects
  the face itself (`cube_uv`, `select.rs:4037`). Both are the paths the float
  cubes of the same families already pass on.
- **Integer texel formats.** R, RG and RGB integer formats are stored widened
  to the RGBA class of their element size (`src/backend.rs:105-112`,
  `PVR_SAMPLED_TEXFMTS` `:215`; `is_int_store`, `src/pvr/backend.rs:1457`).
  `RGB10_A2UI` is the RGB10A2 word read without `fcnorm`
  (`src/pvr/backend.rs:1441`, `src/backend.rs:111`). This works on the board:
  `basic_teximage2d.rgb10_a2ui_2d` is not in the TSV.
- **2D array.** The native copy is TEXTYPE 2D per layer, with the layer's
  address in the `tao` data words (`src/pvr/backend.rs:7095-7112`). The byte
  copy in `gl::native_array::copy` works for any format and moves `bpp` bytes
  per texel.
- **3D.** On the grid, one 2D image whose slices sit side by side. A
  one-level integer volume (all K1 3D cases) is read at level 0, slice
  `clamp(floor(w * depth), 0, depth - 1)`, which is GL's NEAREST choice.

## 4. Fix design

**No code change is needed for K1/K1b.** Both the compiler and the descriptor
are in place at `14e7a7421`. What remains is a host test that pins what
`pvr_sglr_shaders` and `pvr_int_targets_deqp` do not: where the texels of an
integer 2D array and 3D texture land, and whether a look-up reads them back.
`tests/pvr_int_cube.rs` already pins this for the cube, both layouts.

If the board run at `472630df` (board-d, tree row 40) still shows K1 cases
failing, look in this order:

1. "Compile failed" remains. The board image does not run the compiler the
   host runs. Check the switches the image boots with
   (`src/shaderc/precompiled.rs:163-165` compares them with the defaults) and
   the image's commit. The selector itself has no K1 refusal left
   (section 2).
2. "Image comparison failed" on the 2D-array cases. Look at
   `gl::native_array::copy` for one-level NPOT layers (11x20: `single`,
   `physical_extent`) and at `select.rs` `array_layer_addr` (`:4270`)
   rounding an `int` layer passed as float. Both are shared with the float
   `*_2d_array` cases, which would fail with them.
3. "Image comparison failed" on the 3D cases. Look at the grid arm's integer
   branch (`select.rs:1737-1757`). Its slice choice is the only
   integer-specific code on the path.

Any such fix goes in `src/pvr/**` or `src/shaderc/online/powervr/**`, with
the host test below extended to the failing shape.

## 5. Host test to add

**`tests/pvr_int_layered.rs`** (features
`host-test,gpu-powervr,shaderc-online`, the `pvr_int_cube.rs` pattern):
integer 2D-array and 3D textures in dEQP's K1 shapes, for R8UI, R32UI,
RG16I, RGBA8I, RGBA16UI, RGBA32I and RGB10_A2UI.

- **Placement, 2D array.** An 11x20x8 one-level texture uploaded with
  glTexImage3D, and a 26x25x10 one with a 23x19x8 glTexSubImage3D at
  (1, 2, 0), each texel distinct per (layer, x, y). Assert that every element
  of the native layer-major copy (`gl::native_array::copy`; offset
  `k * layer_bytes + level_texel_off + twiddle(x, y) * bpp`) equals the
  uploaded one (`TestGl::gpu_bytes`). Also assert this for the stacked
  image with native array off.
- **Placement, 3D.** The same two shapes as a 3D texture. Assert every
  element of the grid image (`vol_grid` cell `(z % cols, z / cols)` times the
  slice size). With native 3D on, assert the twiddled volume
  (`gl::native_vol`) the same way.
- **Look-up.** A CPU model of the program's NEAREST look-up: the native
  layer `clamp(round(layer))`; the grid slice `clamp(floor(w * depth))` and
  `vol_uv`'s cell. Feed it the texel centres of every layer or slice, and
  check that it returns the uploaded value through each layout's placement.
- **Programs.** dEQP's `Texture2DArrayShader` and `Texture3DShader` with
  `isampler*` and `usampler*` compile to `smp` with `fcnorm` clear and four
  integer channels: `dmn` 2d with `tao` for the native array, 2d for the
  grid, 3d for native 3D.
- **Descriptors.** The bind's IMAGE_WORD0 `texformat` is the integer class
  (TEXF_RGBA8UI ... TEXF_RGBA32I, TEXF_RGB10A2 for RGB10_A2UI), and
  SAMPLER_WORD0 has POINT/POINT and `mipfilter` false for the dEQP state.
- **Cube chain.** For a 64x64 seven-level `RGBA16UI` cube with
  NEAREST_MIPMAP_NEAREST and MAX_LEVEL 6 (the `basic_teximage2d` cube shape),
  `maxlod` is `6 * 64` and `mipfilter` false. This adds the mipmapped sampler
  words to `pvr_int_cube.rs`'s one-level focus.

The test adds a `tools/check.sh` row (plan #387 owns that file; name the row
in the report and do not edit it from a docs item).

## 6. Expected outcome

All **204** K1 + K1b cases should turn to **Pass** on the next AM62P run at
`472630df` or later (tree row 40, board-d). The reasoning:

- The shader now compiles: measured on the host, section 2.
- The 100 cube cases use the native-cube path that the float
  `basic_teximage2d.*_cube`, `teximage2d_pbo.*_cube` and `random_teximage2d.cube_*`
  cases of the same build pass on. Those float cases are absent from the TSV,
  except RGBA32F, which is K2. The two paths differ only in `fcnorm` and the
  destination type. `tests/pvr_int_cube.rs` pins placement and look-up.
- The 100 2D-array and 3D cases are one level, NEAREST, with no level choice
  to get wrong. They are not covered by a placement test until
  `pvr_int_layered.rs` exists. This is the remaining risk.

The 119 integer cases under K3/K4/K5 (section 1) also get past compile at the
tip. They then fail or pass by their own cause: K3 NPOT cube levels, K4 NPOT
2D-array levels, K5 mipmapped 3D on the grid. They are not expected to Pass
from this note alone.

This is a host reading. Nothing here was measured on the board.

## 7. Is a src/gl change needed?

**No.** Nothing in K1/K1b needs `src/gl/**`. The fix (integer `ret` in the
select arms) is in `src/shaderc/online/powervr/select.rs`. The state comes
from `src/pvr/backend.rs`. The GL core already hands the backend NEAREST and
the level range (`src/gl/draw.rs:711-735` `sampling_fields`), and
`gl::native_array`, `gl::native_cube` and the 3D grid copy integer formats
byte for byte. An integer texture with a LINEAR filter must be
texture-incomplete (ES 3.0 3.8.13). That rule belongs to the GL core's
completeness check, not to these cases, and no K1/K1b case exercises it.
