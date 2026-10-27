<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 `texture.*` on the AM62P: subgroups, failure buckets, sources, plan items

Input for `docs/es3-cts-failures.md` (plan #3, item `es3-inventory`). Docs only.
Everything below comes from the repository or from the pinned VK-GL-CTS
checkout (`93bca01`, `modules/gles3/functional/`, the deps cache). Nothing
here was run. Where the repository has no per-case data the cell says so;
nothing is estimated.

## 0. Data and its limits

| What | Where | Shape |
|---|---|---|
| Mustpass case list | `tools/cts/gles3-mustpass.txt` (`dEQP-GLES3.functional.texture.*`: **5,105** cases) | exact |
| AM62P group totals, 396d667a (2026-09-26) | `tools/cts/RESULTS.md` §1r, group table (line 1437 on, row `texture`): **835 pass / 2,612 fail / 1,658 not supported**, 0 other (835+2,612+1,658 = 5,105) | exact, mustpass-only |
| AM62P per-image summaries | `tools/cts/RESULTS.md` §1r, image table (rows 18, 19a-19g, 191-1910, 20, 21) | **raw**: each log's own summary of its filter pattern. A pattern can match cases outside the mustpass, so a raw count can exceed the mustpass count of the subgroup |
| AM62P failure messages | `tools/cts/RESULTS.md` §1r, "The 8,835 failures by message" (16 largest of 78 buckets) and the `AM62P GLES3 (§1r)` / `(shadow, 15dde808)` / `(depth uploads)` rows (lines ~989-1000) | message totals over all groups, with the groups named, not split per subgroup |
| Status prose | `tools/cts/STATUS.md` lines 9 and 250 (Japanese) | only the group totals; no texture detail |
| Per-case results | none. The UART logs live in a worktree (`build/g3-396d667a/`, RESULTS.md after the image table) that is not in the repository; `tools/cts/testdata` has no GLES3 AM62P files | **absent** |

The raw/mustpass check: three raw summaries equal their mustpass counts
exactly, which shows the others are supersets. `texture.shadow` 0/432/0 (19g)
and `texture.format` 164/334/48 = 546 (19e) and `texture.wrap` 144+1,296 =
1,440 (21) match the mustpass totals below. `texture.filtering.2d` raw is
108+114+12 = 234 against 129 in the mustpass, so there the raw figure is only
an upper bound.

Sum of the raw failing figures: 114 (2d) + 210 (2d_array) + 110 (cube) + 334
(format) + 137 (mipmap+size) + 432 (shadow) + 1,518 (specification, swizzle,
units, vertex) + at least 124 (3d) = at least 2,979, against 2,612 in the
mustpass. So at least 367 raw failures are outside the mustpass; which ones is
not recorded.

The documents named in the task as hardware sources `docs/es3/backend-caps.md`
and `docs/es3/pvr-2darray.md` are **not on the integration branch** (only
`docs/es3/pvr-formats.md`, `es3/occlusion-query.md`, `es3-host-toolchain.md`
are). The hardware facts below are from `docs/es3/pvr-formats.md` and from
RESULTS.md.

## 1. Subgroups (second and third level) with mustpass case counts

Counts come from `tools/cts/gles3-mustpass.txt`; they sum to 5,105.

| Second level | cases | Third level (cases) |
|---|---:|---|
| `specification` | 1,163 | see table 2 (26 families) |
| `wrap` | 1,440 | 40 groups x 36: rgba8, etc1, 11 etc2/eac groups (etc2_rgb8, etc2_srgb8, punchthrough x2, etc2_eac_rgba8, srgb8_alpha8, eac_r11, eac_signed_r11, eac_rg11, eac_signed_rg11), 24 astc_* (12 block sizes, plain and `_srgb`) |
| `filtering` | 540 | 2d 129, 2d_array 134, 3d 153, cube 124 |
| `format` | 546 | sized 438 (2d 112, cube 112, 2d_array 112, 3d 102), unsized 64, compressed 44 (etc1 4, etc2 24, eac 16) |
| `shadow` | 432 | 2d 144, cube 144, 2d_array 144 |
| `compressed` | 266 | astc 266 (10 families x 14 or 28) |
| `mipmap` | 224 | 2d 128, 3d 64, cube 32 |
| `vertex` | 174 | 2d 39, 2d_array 39, cube 39, 3d 57 |
| `units` | 159 | 2_units 50, 4_units 50, 8_units 50, all_units 9 |
| `swizzle` | 83 | single_channel 20, multi_channel 63 |
| `size` | 78 | 2d 36, cube 42 |
| **total** | **5,105** | |

Fourth level that matters for the mapping:

- `filtering.{2d,cube,2d_array,3d}`: `formats`, `sizes`, `combinations` (cube
  also `no_edges_visible`). 2d: 78/28/23; 2d_array: 72/26/36; 3d: 71/26/56;
  cube: 78/26/18/2.
- `mipmap.2d`: affine 12, base_level 4, basic 36, bias 4, generate 48,
  max_level+max_lod 8, min_lod 4, projected 12. `mipmap.3d`: affine 12, basic
  24 (12 of them `_npot`), bias 4, max 8, min 4, projected 12. `mipmap.cube`:
  base 4, basic 4, bias 2, generate 16, max 2, min 4.
- `shadow.*`: 3 formats (depth_component16, depth_component32f,
  depth24_stencil8) x 6 min filters x 8 compare functions = 144 per target
  (`es3fTextureShadowTests.cpp:1035-1037`, filters just below, compare funcs
  after them).

## 2. The failure map

Columns: **fails** is the AM62P figure with the source of the figure;
"exact" = the mustpass tally, "raw ≤" = the log summary of a pattern that
covers more than the subgroup (an upper bound); "pooled" = counted only
together with other subgroups; "none recorded" = the repository has no
number for this subgroup. `RES` = `tools/cts/RESULTS.md`. Sources are
`modules/gles3/functional/` in VK-GL-CTS 93bca01.

### 2.1 Everything but `specification`

| subgroup | mustpass cases | AM62P fails | bucket | source | what the case exercises / suspected cause | plan item |
|---|---:|---|---|---|---|---|
| `wrap.*` (40 groups) | 1,440 | **0** exact: RES §1r image 21, 144 pass / 0 fail / 1,296 n/s, = 1,440 | n/a (1,296 are NotSupported: compressed formats, §3) | `es3fTextureWrapTests.cpp:425-` (rgba8 `:461`, etc1 `:482`, etc2 `:582`, astc `:621`) | wrap modes x filters on RGBA8 and compressed textures | none needed (no failure) |
| `compressed.astc.*` (10 families) | 266 | 0 recorded fails; not supported (per-case split not recorded; image 18 holds them: 1,174 / 516 / 331 with `shaders.texture_functions`, RES §1r) | n/s | `es3fASTCDecompressionCases.cpp` | ASTC decode; reported NotSupported on the AM62P (the per-case split is not recorded) | none (not a failure; counted in the 1,658 n/s) |
| `format.compressed.*` (etc1 4, etc2 24, eac 16) | 44 | none recorded (19e is pooled with sized/unsized, see below) | not split | `es3fTextureFormatTests.cpp` (`Compressed2DFormatCase :818`, `CompressedCubeFormatCase :945`) | ETC1/ETC2/EAC 2D and cube, pot/npot | none recorded |
| `format.unsized.*` | 64 | none recorded; pooled in 19e below | not split | `es3fTextureFormatTests.cpp:1378-1425` (alpha, luminance, luminance_alpha, rgb565, rgb8, rgba4444, rgba5551, rgba8; 2d/cube/2d_array/3d, pot and npot) | ES 2.0 style unsized upload + sampling | none recorded |
| `format.sized.{2d,cube,2d_array,3d}` | 438 (112/112/112/102) | `texture.format` as a whole: **334 of 546, exact** (RES §1r image 19e: 164 / 334 / 48 n/s); sized/unsized/compressed not split | "Texture upload failed: GL_INVALID_OPERATION (gluTexture)" 522 shared with `shaders.texture_functions` and `texture.filtering`; "Image comparison failed" 675 shared with fbo/fragment_out/...; "Image verification failed" 298 shared with filtering/mipmap (RES §1r, 16-largest table) | `es3fTextureFormatTests.cpp:1342-` (sized list from `:1429`: rgba32f/i/ui, rgba16f/i/ui, rgba8/i/ui, srgb8_alpha8, srgb_r8, ... r8; targets `Texture2DFormatCase :85`, cube `:255`, 2d array `:481`, 3d `:649`); the sampler type comes from the format (`getSamplerType`, `:202`, `:418`, `:595`, `:764`), so integer formats are sampled as `isampler`/`usampler` and rendered to the default framebuffer, not to an integer target | ES 3.0 sized formats refused or mis-converted at upload (RES row "ES 3.0 sized texture formats refused at upload", a reading from the message); then sampling of float/packed-float/snorm/sRGB/integer per GPU (PowerVR hardware: `docs/es3/pvr-formats.md` §2.1-2.3, integer formats are not filterable, §1.6) | sized-formats-gl, then formats-pvr (AM62P); formats-v3d / formats-gc for the other boards |
| `filtering.2d.{formats,sizes,combinations}` | 129 (78/28/23) | **raw ≤ 114** (RES §1r image 19a: 108 / 114 / 12 n/s over 234 cases; the mustpass tally is not split) | not split. Candidates by message: upload GL_INVALID_OPERATION (formats), Image verification failed (298, filtering+mipmap) | `es3fTextureFilteringTests.cpp:1382` (formats `:1386`, sizes `:1433` 4x8, 32x64, 128x128, 3x7, 31x55, 127x99, combinations `:1458`); formats list `filterableFormatsByType` `:~1360-1376` (rgba16f, r11f_g11f_b10f, rgb9_e5, rgba8, rgba8_snorm, rgb565, rgba4, rgb5_a1, srgb8_alpha8, srgb_r8, srgb_rg8, rgb10_a2, bgra, bgra8) | 2D filter x wrap on RGBA8 (sizes, combinations) and on 14 filterable ES 3.0 formats (formats) | formats part: sized-formats-gl, formats-pvr; sizes/combinations part: **none** (no failure figure, no cause recorded) |
| `filtering.cube.*` | 124 (78/26/18/2) | **raw ≤ 110** (image 19d: 108 / 110 / 12 n/s) | not split | `es3fTextureFilteringTests.cpp:1490` (formats `:1494`, sizes `:1546` 8,64,128,7,63, combinations `:1571`, no_edges_visible `:1602`) | cube filtering; AM62P uses the native cube (RES row "step C", ES 3.0 seamless LINEAR) | formats part: sized-formats-gl, formats-pvr; rest **none** |
| `filtering.2d_array.*` | 134 (72/26/36) | **raw ≤ 210** (image 19b: 0 / 210 / 12 n/s over 222 cases), i.e. every case that is not n/s. RES row "(shadow, 15dde808)" calls it "the whole of texture.filtering.2d_array (210)" - that is the raw figure, the mustpass tally is ≤ 134 | not split | `es3fTextureFilteringTests.cpp:1619` (formats `:1624`, sizes `:1649` 4x8x8, 32x64x16, 128x32x64, 3x7x5, 63x63x63, combinations `:1675`) | 2D array as `sampler2DArray`, all filters/wraps | the online compiler samples a 2D array as one tall stacked image, so the TPU filters and wraps across layers (RES row "2D-array sampling", "Cause:", a statement in RESULTS.md, step 1 measured on a parked branch: +20 pass); also NPOT levels >= 1 of arrays | pvr-2darray; tex3d-levels (NPOT levels) |
| `filtering.3d.*` | 153 (71/26/56) | **124 to 138** (bounds, not a tally): images 191-196 (mustpass names 1-132): 3 pass, 117 fail, 12 n/s; names 133-153: 14 results with no summary (image 197, unknown), 7 re-run singly all fail (images 198-1910). RES §1r image table | not split | `es3fTextureFilteringTests.cpp:1708` (formats `:1712`, sizes `:1738` 4x8x8, 32x64x16, 128x32x64, 3x7x5, 63x63x63 all RGBA8 x 6 filters, combinations `:1765`) | 3D filter x wrap. **Even the power-of-two `sizes.4x8x8_*` fail (0/5 in image 198)**, and `sizes` are RGBA8, so formats do not explain it | tex3d-levels covers NPOT levels >= 1 only; for the POT cases **none** (cause not investigated; listed in section 4) |
| `mipmap.2d.*` / `mipmap.cube.*` | 128 / 32 | pooled with `size` and `mipmap.3d`: **raw ≤ 137** (image 19f: 183 / 137 / 0 over 320 cases; mipmap 224 + size 78 = 302 in the mustpass) | "Image verification failed" (298, with filtering), "Image comparison failed" (675) | `es3fTextureMipmapTests.cpp:2356-` (2d `:2366`, bias `:2480`, generate `:2492`, min/max lod, base/max level; RGBA4444/5551/8888 sources `:2407-2409`) | mip chain selection, LOD bias, generate | none recorded |
| `mipmap.3d.*` | 64 | 4 are known: `bias.*` "Failed to compile shader program (glsTextureTestUtil)": the online compiler refuses "a bias on a ND texture" (RES §1r row "shadow look-ups", "the same bucket holds 4 texture.mipmap cases, bias on 3D"). The rest is pooled in the 137 above | compile refusal (4); others not split | `es3fTextureMipmapTests.cpp:2368`, 3D: bias `:2656`, min/max lod and levels `:2669-2693`, sizes RGBA8 (`:2647`, `:2662`) | 3D mip selection; `basic.*_npot` (12) needs NPOT levels >= 1 (RES row "depth uploads", level >= 1 of a mipmapped NPOT 3D/array is refused by the GL core) | tex3d-levels for the 12 `_npot`; bias on 3D (4): **none** (shaderc selector; `pvr-shadow` is for shadow look-ups) |
| `shadow.2d` | 144 | **144 of 144 exact** (RES §1r image 19g: 0 / 432 / 0 over all three targets; RES row "shadow look-ups": "All 432 texture.shadow cases are in these two" buckets) | 288 over the group: "Failed to compile shader program (glsTextureTestUtil)" - online compiler refusal of shadow look-ups; 144 over the group: "Texture upload failed: GL_INVALID_ENUM (gluTexture)" - depth formats. The per-target split of 288/144 is not recorded (the 144 is one third of 432, the formats are three; that it is one format is not stated) | `es3fTextureShadowTests.cpp:1021-` (formats `:1035`, 6 filters, 8 compare functions; 2d `:1061`, cube `:1093`, 2d_array `:1124`); verification `:1000-1010` | `sampler2DShadow` with GL_TEXTURE_COMPARE_MODE/FUNC, depth16 / depth32f / depth24_stencil8 | compile refusal: pvr-shadow (online compiler); depth upload INVALID_ENUM: sized-formats-gl. Note RES row "(shadow, 15dde808)": on a parked branch NEAREST passes and LINEAR/mipmap are 18 of 24 QualityWarning (per-texel compare missing) - pvr-shadow's PCF |
| `shadow.cube` | 144 | same, pooled in 432 | same | same, `:1093` | `samplerCubeShadow` | pvr-shadow; sized-formats-gl |
| `shadow.2d_array` | 144 | same, pooled in 432 | same; plus RES row "(shadow, 15dde808)": 2d_array fails 18 of 24 per filter even with NEAREST | same, `:1124` | `sampler2DArrayShadow`: stacked-image sampling | pvr-shadow; pvr-2darray; sized-formats-gl |
| `size.2d` / `size.cube` | 36 / 42 | pooled in image 19f above (≤ 137 with mipmap); no message names `size` | none recorded | `es3fTextureSizeTests.cpp:346-` (formats l8, rgba4444, rgba5551, rgba8888 `:378`) | texture sizes up to 2048 | none recorded |
| `swizzle.single_channel` / `multi_channel` | 20 / 63 | pooled in image 20 (specification+swizzle+units+vertex, **raw 1,518**); RES §1r names `texture.swizzle` among the "Image comparison failed" (675) groups | "Image comparison failed" | `es3fTextureSwizzleTests.cpp:240-` (formats alpha, luminance, luminance_alpha, r8, rg8, rgb8, rgba8 only; `GL_TEXTURE_SWIZZLE_{R,G,B,A}` x red/green/blue/alpha/zero/one) | the sampler swizzle state of 8-bit formats; no ES 3.0 sized format is needed | **none** (not format-bound; cause unknown) |
| `units.{2,4,8,all}_units` | 159 | pooled in image 20; named among the "Image comparison failed" groups | "Image comparison failed" | `es3fTextureUnitTests.cpp:1192-` (format list `:87-93`: **all** ES 3.0 colour formats including rgba32i/ui, r8i, rgb10_a2ui, r11f_g11f_b10f) | several bound textures of mixed type per draw | sized-formats-gl and formats-pvr for the format list; the rest unknown (**none** recorded) |
| `vertex.2d` / `.cube` | 39 / 39 | pooled in image 20; named among the "Image comparison failed" groups | "Image comparison failed" | `es3fVertexTextureTests.cpp:1972-` (groups `:1983-1998`) | `texture2D`/`textureCube` from the vertex shader | none recorded |
| `vertex.2d_array` | 39 | pooled in image 20 | same | `es3fVertexTextureTests.cpp:1363` (`sampler2DArray` in the vertex shader) | array fetch from the vertex shader | pvr-2darray (a reading: the same stacked-image sampling) |
| `vertex.3d` | 57 | pooled in image 20 | same | `es3fVertexTextureTests.cpp:1998` | 3D fetch from the vertex shader | tex3d-levels if NPOT levels; otherwise **none recorded** |

### 2.2 `specification` (1,163 cases, 26 families)

The group's figure: 736 failures carry the message "Runtime check failed:
glGetError() == GL_NO_ERROR (es3fTextureSpecificationTests.cpp)" (RES §1r,
16-largest table, row 3, and the `AM62P GLES3 (§1r)` row at line ~990, which
names `basic_teximage2d.r11f_g11f_b10f_2d`,
`basic_teximage3d.rg8ui_2d_array`, `basic_texsubimage2d.rgba32ui_cube` as
examples). How the 736 spread over the 26 families is **not recorded**;
image 20 gives only the pooled 1,518 raw failures of specification, swizzle,
units and vertex (1,579 cases in the mustpass, 1,723 run). All the
specification cases read back with `glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE)`
from the default framebuffer (`es3fTextureSpecificationTests.cpp:250-253`),
so no integer render target is involved; `int-rt-gl` / `int-rt-pvr` do not
cover this group.

| subgroup | cases | AM62P fails | bucket | source | what the case exercises | plan item |
|---|---:|---|---|---|---|---|
| `basic_teximage2d` | 98 | pooled in 736 | Runtime check (glGetError) | `:3896` | glTexImage2D with every sized colour format (2D and cube variants) | sized-formats-gl |
| `basic_texsubimage2d` | 98 | pooled in 736 | same | `:4107` | glTexSubImage2D, all formats | sized-formats-gl |
| `basic_teximage3d` | 69 | pooled in 736 | same | `:4405` | glTexImage3D, 3D and 2D array | sized-formats-gl; tex3d-levels (levels >= 1) |
| `basic_texsubimage3d` | 49 | pooled in 736 | same | `:4565` | glTexSubImage3D | sized-formats-gl; tex3d-levels |
| `random_teximage2d` | 20 | none recorded | not split | `:3916` | random-order level upload | none recorded |
| `teximage2d_align` | 34 | none recorded | not split | `:3948` | unpack alignment | none recorded |
| `teximage2d_unpack_params` | 13 | none recorded | not split | `:3991` | row length, skip pixels/rows | none recorded |
| `teximage2d_pbo` | 108 | none recorded | not split | `:4021` | upload from a PBO, many formats | sized-formats-gl if it fails on the format; PBO path itself unknown |
| `teximage2d_depth` | 5 | none recorded (the RES row says client-memory depth uploads pass since eef7a9d2 / e60bb067) | n/a | `:4073` | depth / depth-stencil upload | sized-formats-gl (done per that row) |
| `teximage2d_depth_pbo` | 5 | **5, named**: RES row "(depth uploads)", 10 for the two `_depth_pbo` groups | 'glGetError() == GL_NO_ERROR' | `:4090` | depth upload from a PBO | tex3d-levels (its second half, "depth uploads from a PBO") |
| `teximage3d_depth_pbo` | 4 | 5 named in the RES row (5 each; 4 in the mustpass, so the row counts a case outside the mustpass - the row's own figure is kept) | same | `:4547` | same, 3D / array | tex3d-levels |
| `teximage3d_pbo` | 112 | none recorded | not split | `:4469` | glTexImage3D from a PBO | sized-formats-gl (format part) |
| `teximage3d_unpack_params` | 14 | none recorded | not split | `:4429` | unpack params 3D | none recorded |
| `teximage3d_depth` | 5 | none recorded | not split | `:4529` | depth upload 3D | none recorded |
| `texsubimage2d_align` | 40 | none recorded | not split | `:4148` | alignment | none recorded |
| `texsubimage2d_unpack_params` | 13 | none recorded | not split | `:4220` | params | none recorded |
| `texsubimage2d_empty_tex` | 16 | none recorded | not split | `:4127` | sub-image into storage with no data | none recorded |
| `texsubimage2d_pbo` | 109 | none recorded | not split | `:4262` | PBO sub-image | sized-formats-gl (format part) |
| `texsubimage2d_depth` | 5 | none recorded | not split | `:4338` | depth sub-image | none recorded |
| `texsubimage3d_pbo` | 110 | none recorded | not split | `:4631` | PBO 3D sub-image | sized-formats-gl (format part) |
| `texsubimage3d_unpack_params` | 14 | none recorded | not split | `:4583` | params | none recorded |
| `texsubimage3d_depth` | 5 | named in RES row "(depth uploads)" with texstorage2d/3d: 20 together | glGetError (GL_INVALID_VALUE at level >= 1 of a mipmapped NPOT 2D array, host repro 57x44x5 "teximage3d level 1 28x22: 0x501") | `:4724` | depth sub-image 3D | tex3d-levels |
| `texstorage2d` | 121 | named in the same 20 (117x97 / 57 2D and cube) | same | `:4742` (format subgroup `:4747`) | glTexStorage2D, all formats, mip chain | tex3d-levels (20 shared); sized-formats-gl for the formats |
| `texstorage3d` | 76 | named in the same 20 | same | `:4863` (`:4868`) | glTexStorage3D, all formats | tex3d-levels; sized-formats-gl |
| `basic_copyteximage2d` | 10 | none recorded | not split | `:4355` | copy from the framebuffer | none recorded |
| `basic_copytexsubimage2d` | 10 | none recorded | not split | `:4376` | same | none recorded |

The RES row "(depth uploads)" puts the NPOT-levels failures at 20 across
`texstorage2d` (2D and cube), `texstorage3d` and `texsubimage3d_depth`, and
says "the 2D/cube cases are a reading"; only the 2D-array case is shown on the
host. The note that colour formats of texstorage2d (rgba8, r8, rgb565, r32f,
rgba16f) fail the same way means these 20 are **not** part of the "ES 3.0
sized formats refused" cause.

## 3. Not supported (1,658) is not a failure

1,658 of the group's cases are NotSupported on the AM62P (RES §1r group
table). The only recorded part: `texture.wrap` 1,296 (image 21, compressed
formats) and 331 n/s in image 18 (`shaders.texture_functions` and
`texture.compressed`, not split) plus 12 each in the filtering images 19a, 19b,
19d and 12 in 196 (3D). The remaining split is not recorded. These do not count
toward 2,612 and are not given a plan item here; ETC2/EAC support is described
in `docs/es3/pvr-formats.md` §2.4.

## 4. Coverage by plan item, and what no item covers

Items with a texture row (in order of the share of the 2,612 they plausibly
hold, from section 2):

| item | rows it fixes |
|---|---|
| sized-formats-gl | `specification` basic_* and PBO format parts (736 pooled); `format.sized` upload; `filtering.*.formats` upload; `shadow` 144 (depth upload INVALID_ENUM); `units` |
| formats-pvr (formats-v3d, formats-gc for the other boards) | `format.sized` and `filtering.*.formats` sampling; `units` |
| pvr-2darray | `filtering.2d_array` (≤ 210 raw), `shadow.2d_array`, `vertex.2d_array` |
| pvr-shadow | `shadow.*` 288 compile refusals and the per-texel compare |
| tex3d-levels | 20 NPOT-levels cases, 10 depth-PBO cases, `mipmap.3d.basic.*_npot` (12), `filtering.3d` NPOT sizes |
| int-rt-gl / int-rt-pvr | **no row of the texture group** (specification verified: default-framebuffer read-back, `:250-253`; the format/filtering renderers were not checked beyond `getSamplerType`) |

### Uncovered causes, ranked by fail count

Counts are the best the repository gives (raw upper bounds unless marked).

1. **Pooled `specification` + `swizzle` + `units` + `vertex` remainder:
   up to 1,518 − 736 = 782 raw failures with no recorded cause.** The 736 is
   attributed to sized-formats-gl; the other (≤ 782) are "Image comparison
   failed" and similar, in swizzle (83 cases, no item), vertex (174), units
   (159, partly the format list) and the specification families marked "none
   recorded" above. Needs the per-case results (host run) before any item.
2. **`filtering.3d` POT sizes and combinations: ≥ 124 fails, cause unknown**
   (`sizes.4x8x8_*` fail although power-of-two RGBA8). tex3d-levels is about
   NPOT levels and does not cover it. Needs a host repro of one case.
3. **`filtering.2d` / `filtering.cube` outside the format lists: ≤ 114 + 110
   raw**, no cause recorded for `sizes` / `combinations` (the 522 upload and 298
   "Image verification" buckets are shared with other groups).
4. **`mipmap` + `size` pool: ≤ 137 raw**, of which only the 4 `mipmap.3d.bias`
   compile refusals are explained (bias on a 3D texture - no item) and the
   12 `_npot` 3D ones (tex3d-levels). The rest: no cause recorded.
5. **`mipmap.3d.bias`: 4**, the selector refusal "a bias on a ND texture" - no
   item (shaderc; not part of pvr-shadow).
6. **`swizzle`: no item at all** (83 cases; the failures, if any, are inside
   the pool in 1).

The mustpass-exact figures are few: `format` 334, `shadow` 432, `wrap` 0, and
the group total 2,612. Everything else needs the host dEQP run
(`host-gles3-run`) or the AM62P logs from `build/g3-396d667a/` to be split
properly; the two documents named at the top as hardware sources
(`es3/backend-caps.md`, `es3/pvr-2darray.md`) are not yet on the branch.

## 5. What would make this table exact

- The 78-bucket list of RES §1r in full, with the case names per bucket (only
  the 16 largest are in the repository). That gives `specification` per
  family and the unexplained pool of section 4, item 1.
- The AM62P UART logs of images 18, 19a-19g, 20, 21 (`tools/cts/deqp-uart-tally.py
  --mustpass tools/cts/gles3-mustpass.txt` over them gives a mustpass-only
  per-subgroup tally; the host run in plan item `host-gles3-run` cannot
  stand in for it because render verdicts on the host are meaningless, see
  RES "AM62P host pre-run" rows).
- The 14 cases of image 197 with no summary (names 133-153 of the 3D list)
  to turn "at least 124" into an exact figure.

## 6. What has closed or reduced rows since (integration tip `ad2f67d8`)

The tables above are the AM62P run of `ad2b5ed` and stay as written. Later
merged work, by row. Everything is host-side (API and compile verdicts; no
board has run these commits), so none of it is a conformance figure.

| rows | item, commit | effect |
|---|---|---|
| `format.sized`, `filtering.*.formats`, `units`, `specification` (GC7000UL) | formats-gc, `dc83be67` and `541975a2` | eight classes (R11G11B10F, RGB9E5, RGB10A2UI, RGBA8_SNORM, RGBA8UI/I, RGBA16UI/I) sampled natively; integer look-ups select. Host upload refusals `texture.format` 200 -> 64, `texture.specification` 448 -> 141 (measured at `541975a2`, HOST-ONLY; `docs/es3/gc-formats.md` section 5); the rest are the 32-bit integer formats |
| `format.sized`, `filtering.*.formats` (V3D) | formats-v3d, `cc1c6ede` | every class but RGB10A2 sampled, sRGB draws no longer refused; before it the host showed 72 upload refusals and 24 draw refusals in `texture.filtering` (`docs/es3/texture-host-repro.md` section 1, at `70142d35`). Left: ETC2, `texelFetch`, offsets, shadow, 2D-array and 3D look-ups (the 227 `NotYet` compile refusals of the 2D-array and 3D programs) |
| `filtering.2d_array`, `shadow.2d_array`, `vertex.2d_array` (PowerVR) | pvr-2darray, `e160bafb`, `d7b2cf8b`, `1ddb0cf3` | native array sampling behind `pvr_native_array` (default off), every shadow form compiles, PCF for LINEAR over one level. Open: mipmap and cube PCF, mirrored repeat, `textureGrad*` and offset forms on 2D arrays and 3D (no owner) |
| `swizzle.*` (83), `filtering.3d.*` (153) | int-rt-pvr, `e4f58cbe` | the GL core delivers the swizzle and wrap R; no back end applies them yet (`docs/es3/texture-host-repro.md` section 3). Host dEQP before = after |
| `specification` NPOT levels, depth PBO | tex3d-levels | see `docs/es3-cts-failures.md` |
| `shaders.texture_functions` look-ups on arrays and 3D | pvr-shadow, `d7356b65` | 226 PowerVR refusals in one message, `src/shaderc/online/powervr/select.rs`; front end clean (`docs/es3/shader-host-repro.md`) |

Rows nothing has touched: `mipmap.3d.bias` (4), `size`, `vertex.2d/cube`,
`filtering.3d` on the PowerVR (needs pvr-3d), ETC2 on the GC7000UL and V3D.
