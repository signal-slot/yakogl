<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 `texture.filtering.*` and `texture.swizzle.*`: host reproduction, causes, GL-core fixes

Plan #19, item `int-rt-pvr`. The two groups run on the host against the
three GPU libraries (the in-process compiler, the Linux OSAL that executes
nothing), every failing case put to a cause with its code site, the GL-core
causes fixed (`src/backend.rs`, `src/gl/draw.rs`, `src/gl/es3.rs`; the
tests in `tests/es3_tex_filtering.rs`), the rest listed for the items that
own them. Integration tip `70142d35`, VK-GL-CTS `93bca01`, 2026-10-22.

## 0. What the host can show, and what it cannot

`tools/cts/host-deqp-run.sh` links a library whose kicks are never
executed: every `glReadPixels` reads black. So a **render verdict from the
host is meaningless** (the script says so in its header) and only these are
real: whether a case begins, its compile and link outcome, any GL error it
hits, a crash or a lost chunk, and the `NotSupported` verdicts dEQP gives
before it draws. Both groups here are picture comparisons, so the host can
rule causes *out* (no refusal, no API error, no crash on any GPU) but cannot
show a wrong picture turning right. What it does show after the fix is the
binding the back end receives, which `tests/es3_tex_filtering.rs` pins.

Per-case board data: the AM62P (PowerVR) run of build `ad2b5ed`
(2026-10-13/14, `docs/es3/am62p-gles3-notpass.tsv`). The Pi 4 list
(`docs/es3/pi4-gles3-fails.tsv`) holds images A1, D1 and FB1-FB3 only, none
of the texture group; the i.MX8MP has no dEQP-GLES3 run. So the GC7000 and
V3D rows below come from reading the back ends, not from a board.

## 1. The host runs, before and after

Six runs per tree: `texture.filtering.` (540 cases) and `texture.swizzle.`
(83) on each GPU's library. "Before" is a clean copy of `70142d35` (the
working tree with this branch's files put back from HEAD), "after" is this
branch. Every case began on every GPU; no crash. HOST-ONLY throughout.

| GPU | group | cases | began | picture Fail | "Pass" | NotSupported | compile refused | upload refused | draw refused (ResourceError) | before = after |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| PowerVR | filtering | 540 | 540 | 492 | 0 | 48 | 0 | 0 | 0 | yes |
| PowerVR | swizzle | 83 | 83 | 70 | 13 | 0 | 0 | 0 | 0 | yes |
| GC7000 | filtering | 540 | 540 | 492 | 0 | 48 | 0 | 0 | 0 | yes |
| GC7000 | swizzle | 83 | 83 | 70 | 13 | 0 | 0 | 0 | 0 | yes |
| V3D | filtering | 540 | 540 | 169 | 0 | 48 | 227 | 72 | 24 | yes |
| V3D | swizzle | 83 | 83 | 70 | 13 | 0 | 0 | 0 | 0 | yes |

"Picture Fail" is "Image verification failed" / "Image comparison failed"
against a black picture: not a verdict, and the same before and after, which
is the expected answer of a harness that renders nothing. The 48
`NotSupported` are the 12 `srgb_r8_*` and `srgb_rg8_*` cases of each of
the four `formats` subgroups: `GL_EXT_texture_sRGB_R8` / `_RG8` are not
advertised (the AM62P says the same, 24 of them in the mustpass). The 13
swizzle "passes" are the cases whose reference picture is black too
(`*_all_zero` of the seven formats, `alpha_all_{red,green,blue}`,
`red_all_{green,blue}`, `rg_all_blue`): a black result equals a black
reference. They are not passes of the swizzle.

The V3D library is the one GPU where the host shows real causes in
`filtering`, all three in `formats` or in the array and 3D targets:

- 227 "Failed to compile shader program": the V3D code generator refuses
  `Program(NotYet("a texture look-up other than 2D or cube"))` for every
  `sampler2DArray` and `sampler3D` program -- all of `2d_array` (104: 36
  combinations, 42 formats, 26 sizes) and `3d` (123: 56, 41, 26).
- 72 "Texture upload failed: GL_INVALID_OPERATION": `r11f_g11f_b10f`,
  `rgb9_e5` and `rgba8_snorm`, six filters each, on all four targets. The
  GL core refuses a sized format none of whose storage classes the back end
  samples (`src/gl/es3_formats.rs` `storage_in`, the sized-formats design);
  the V3D samples RGBA8, SRGB8_ALPHA8, L8, A8, LA8 and R32F only.
- 24 `ResourceError` (`GL_OUT_OF_MEMORY` at the draw, which ends dEQP's
  process; the chunk went on in a second one): `rgba16f_*` (6 of `2d`, 6 of
  `cube`: "draw refused: a texture format other than RGBA8, SRGB8_ALPHA8,
  L8, A8, LA8 and R32F (RGB565, half float, ETC2) is not sampled yet") and
  `srgb8_alpha8_*` (6 + 6: "draw refused: the texture state does not fit
  its record (v3d_tex_pack_state refused a field)" -- a 64x64 sRGB chain
  the V3D back end says it samples but whose texture shader state its
  packer refuses; a V3D defect worth a look before the Pi 4 rerun).

Host figures per subgroup on the PowerVR and GC7000 libraries, either tree
(`Fail` / `NotSupported`): filtering.2d 117 / 12, 2d_array 122 / 12, 3d
141 / 12, cube 124 / 12 (combinations 23, 36, 56, 18; formats 66, 60, 59,
66; sizes 28, 26, 26, 26; cube.no_edges_visible 2); swizzle.single_channel
20 / 0, multi_channel 50 / 0 (13 "pass"). On the V3D library the same
totals split as above (2d: 54 picture, 18 upload, 12 draw; cube: the same;
2d_array: 60 compile, 18 upload; 3d: 123 compile, 18 upload).

## 2. The board figures these cases have (AM62P `ad2b5ed`)

From `docs/es3/am62p-gles3-notpass.tsv` against
`tools/cts/gles3-mustpass.txt`; "pass" is a mustpass case absent from the
not-pass list.

| subgroup | cases | pass | fail | NoResult | NotSupported |
|---|---:|---:|---:|---:|---:|
| filtering.2d (formats, sizes, combinations) | 129 | 117 | 0 | 0 | 12 |
| filtering.cube (formats, sizes, combinations, no_edges_visible) | 124 | 112 | 0 | 0 | 12 |
| filtering.2d_array.combinations | 36 | 7 | 29 | 0 | 0 |
| filtering.2d_array.formats | 72 | 0 | 60 | 0 | 12 |
| filtering.2d_array.sizes | 26 | 0 | 26 | 0 | 0 |
| filtering.3d.combinations | 56 | 3 | 53 | 0 | 0 |
| filtering.3d.formats | 71 | 0 | 54 | 7 | 10 |
| filtering.3d.sizes | 26 | 0 | 22 | 4 | 0 |
| swizzle.single_channel | 20 | 0 | 20 | 0 | 0 |
| swizzle.multi_channel | 63 | 3 | 60 | 0 | 0 |

The passes tell the cause as much as the failures do. The three 3D
combinations that pass are `linear_linear_`, `linear_nearest_` and
`nearest_linear_clamp_clamp_clamp`: no mipmap and every axis clamped. The
three swizzle passes are `alpha_bgra`, `luminance_bgra`,
`luminance_alpha_bgra`: a swizzle that leaves the picture of (0, 0, 0, a)
or (l, l, l, 1) unchanged, so a swizzle that is never applied passes them.
The eleven `NoResult` are every `*_linear_mipmap_linear` 3D case of its
image after the first two: dEQP's check of a wrong trilinear picture takes
minutes per case and the image's 600 s window runs out
(`docs/notes/pvr-3d-mipmap-hang.md`); they are failures of the same cause.

## 3. Every failing case put to a cause

Columns: the count is the AM62P figure where there is one (the host figure
is the same on every GPU and says nothing more); "code site" names the line
that decides; "fix or owner" says what this branch did or which item takes
it. The four causes are **GL core** (fixed here), **back end** (sampler
state the hardware has and the back end does not fill in), **shader
compiler** (a look-up the compiler emulates) and **not reproduced** (no
failure on a board, a black picture on the host).

| subgroup | GPU | count | cause | code site | fix or owner |
|---|---|---:|---|---|---|
| swizzle.single_channel, swizzle.multi_channel | all | 83 (AM62P: 80 fail, 3 pass) | **GL core**: `GL_TEXTURE_SWIZZLE_*` is stored and read back but `GlesTextureBind` had no field for it, so no draw ever handed it to a back end | `src/gl/texture.rs` `tex_parameter` (stored, with the comment "stored and read back, not applied"); `src/gl/draw.rs` texture-binding loop and `texture_bind_2d` (bind built without it); `src/backend.rs` `GlesTextureBind` | **fixed**: `GlesTextureBind::swizzle` (section 4) |
| swizzle.* | PowerVR | 83 | **back end**: TEXSTATE_IMAGE_WORD0's `swiz0..3` come from the texel format alone (`TexFormat`), never from the texture state | `src/pvr/backend.rs` `TexFormat` / `tex_stride_no_alpha` (about line 1045-1180), `tex_image_word0` (1223), `build_tex_desc` (4542) | a PowerVR item (not in the plan yet): compose `b.swizzle` with the format's swizzle, as Mesa does (`pvr_arch_image.c:189-191` `util_format_compose_swizzles`, then `pvr_arch_tex_state.c:40-64` `pvr_get_hw_swizzle`) |
| swizzle.* | GC7000 | 83 (no board run) | **back end**: the descriptor's `TEXDESC_SWIZZLE_{R,G,B,A}` are the format's (luminance, `no_alpha`) only | `src/vivante/texture.rs` lines 202-205 | `formats-gc` (ready) or a GC7000 item: `swizzle_source(k, b.swizzle[k])` through the format's table, as Mesa `etnaviv_format.c:347` `get_texture_swiz` (the view swizzle with the R8-as-L8 and DXT1 fix-ups) |
| swizzle.* | V3D | 83 (no board run) | **back end**: the texture shader state's swizzle is `tmu_format`'s (the format's, `depth_luminance`, `bgra`) only | `src/v3d/texture.rs` `tmu_format` (146-183), `state` (229-240) | `formats-v3d` (running) or a V3D item: compose with `b.swizzle` as Mesa `v3dx_state.c:1059-1073` (`v3d_create_sampler_view`, `util_format_compose_swizzles(fmt_swizzle, view_swizzle)`) |
| filtering.3d.* (all three subgroups) | all | 153 | **GL core**: `GL_TEXTURE_WRAP_R` (and a sampler object's) stored and read back, never in the bind: every back end wraps the third axis however it likes | `src/gl/texture.rs` `tex_parameter` (stored); `src/gl/es3.rs` `sampler_state` (S and T only); `src/gl/draw.rs` binding loop | **fixed**: `GlesTextureBind::wrap_r`, `sampler_state` returns wrap R (section 4) |
| filtering.3d.sizes, .combinations, .formats | PowerVR | AM62P 22 + 4 NoResult, 53 (3 pass), 54 + 7 NoResult (10 n/s) | **shader compiler**: a `sampler3D` look-up goes through the slice grid the GL core keeps a 3D texture in: `vol_uv` clamps s and t inside the slice's cell (`Fmax`/`Fmin` of the half-texel bounds, so REPEAT and MIRRORED_REPEAT on S and T never happen), z is clamped to [0, layers - 1] (no wrap R), the two taps mix slices, and an explicit LOD samples level 0 while a mipmap filter has one level to sample. So `sizes.4x8x8_nearest` (REPEAT, coordinates outside [0, 1]) fails on a power-of-two RGBA8 texture, and only the clamp-everything non-mipmap combinations pass. The sampler word's `addrmode_w` is CLAMP_TO_EDGE by construction | `src/shaderc/online/powervr/select.rs` `ir::Op::Tex { dim: D3 }` (about line 1012-1075), `vol_uv` (2785-2820); `src/gl/texture.rs` `texture_alloc_storage_3d` (the grid is one level to the back end, levels 1.. kept after it); `src/pvr/backend.rs` `build_tex_desc` `addrmode_w` (about 4754) | `pvr-3d` (RC15 in `docs/es3-cts-failures.md`): the TPU's own 3D type as Mesa pvr does it (3D twiddled layout, the full chain, `addrmode_w` from `b.wrap_r`). The 11 NoResult are the same cause plus dEQP's verification time (`docs/notes/pvr-3d-mipmap-hang.md`) |
| filtering.3d.* | GC7000 | 153 (no board run) | **back end**: the sampler's W wrap is copied from the S wrap (`samp_wwrap(uwrap)`); the 3D image itself is native (`volume`, the stacked layout) | `src/vivante/texture.rs` line 388 | `formats-gc` or a GC7000 item: `samp_wwrap(gc_wrap(b.wrap_r, depth <= 1))`, as Mesa `etnaviv_texture_desc.c:108` `SAMP_CTRL0_WWRAP(ss->wrap_r)` |
| filtering.3d.combinations, .sizes, .formats (all but the 18 below) | V3D | 123 (host: compile refused) | **shader compiler**: the V3D code generator refuses every `sampler3D` program, `NotYet("a texture look-up other than 2D or cube")`; behind it the back end refuses the binding too ("a 2D array or 3D texture is not sampled yet") and its sampler packet's `wrap_r` is a constant REPEAT | `src/shaderc/online/v3d/` (the look-up selection); `src/v3d/texture.rs` `refusal_for` (about line 106), `sampler` (214) | `formats-v3d` (running) if it reaches 3D, else a V3D item: the TMU's 3D type with `depth` in the texture shader state and `wrap_r` from `b.wrap_r` (Mesa `v3dx_state.c:595`, `translate_wrap(cso->wrap_r)`) |
| filtering.{2d,cube,2d_array,3d}.formats: `r11f_g11f_b10f_*`, `rgb9_e5_*`, `rgba8_snorm_*` | V3D | 72 (host: upload refused, GL_INVALID_OPERATION) | **back end capability** (the GL core refuses by name, as `docs/es3/sized-formats.md` designs it): the V3D samples none of the storage classes of these three formats | `src/gl/es3_formats.rs` `storage_in`; `src/v3d/texture.rs` `refusal_for` (the format list) | `formats-v3d` (running): the TMU's R11F_G11F_B10F, RGB9_E5 and RGBA8_SNORM types (`docs/es3/backend-caps.md` 1.1) |
| filtering.{2d,cube}.formats: `rgba16f_*` | V3D | 12 (host: draw refused, GL_OUT_OF_MEMORY, a ResourceError) | **back end capability**: the upload is taken (RGBA16F is an original-table format) and the draw refused, "a texture format other than RGBA8, SRGB8_ALPHA8, L8, A8, LA8 and R32F ... is not sampled yet" | `src/v3d/texture.rs` `refusal_for` | `formats-v3d` |
| filtering.{2d,cube}.formats: `srgb8_alpha8_*` | V3D | 12 (host: draw refused, GL_OUT_OF_MEMORY) | **back end defect**: "the texture state does not fit its record (v3d_tex_pack_state refused a field)" for a 64x64 SRGB8_ALPHA8 chain, a format the back end lists as sampled | `src/v3d/texture.rs` `state`, `src/v3d/backend.rs` (the `v3d_tex_pack_state` call) | a V3D item (`formats-v3d` is nearest); not in this item |
| filtering.2d.sizes, .combinations; filtering.cube.sizes, .combinations, .no_edges_visible | PowerVR | 97 | **not reproduced**: all pass on the AM62P at `ad2b5ed`; the host picture is black | - | none |
| filtering.2d.formats, filtering.cube.formats | PowerVR | 156 (66 + 66 pass, 12 + 12 n/s) | **not reproduced**: pass on the AM62P; the 24 `NotSupported` are `GL_EXT_texture_sRGB_R8` / `_RG8`, optional extensions the library does not claim | `src/gl/state.rs` extension string | none (an extension, not a failure) |
| filtering.2d.*, filtering.cube.* | GC7000 | 253 (no board run) | **not reproduced on the host**: no refusal, no error; the picture is the board's to judge | - | the i.MX8MP board (`board-imx8mp` when it runs there) |
| filtering.2d.*, filtering.cube.* (the 205 the V3D rows above do not name) | V3D | 205 (no board run) | **not reproduced on the host**: no refusal, no error | - | the Pi 4 rerun |
| filtering.2d_array.* | PowerVR | 115 (AM62P: 29 + 60 + 26 fail, 7 pass, 12 n/s) | **shader compiler**: a 2D array sampled as one tall stacked image, so the TPU filters and wraps across layers (RC6) | `src/shaderc/online/powervr/select.rs` (the array look-up) | `pvr-2darray` (running; `docs/es3/pvr-2darray.md`) -- outside this item |
| filtering.2d_array.* | GC7000 | 134 (no board run) | native layers; nothing found by reading, no host refusal | - | the i.MX8MP board |
| filtering.2d_array.* (104 compile, 18 upload, 12 n/s) | V3D | 134 (no board run) | **shader compiler** (`sampler2DArray`, the same `NotYet` as 3D) and the three formats above | as the 3D and format rows | `formats-v3d` / a V3D item |

Every one of the 623 cases of the two groups is in a row: 83 swizzle, 153
3D, 253 2D and cube, 134 2D-array.

## 4. The GL-core fix

What the GL core could do for these groups is hand the back ends the state
it already kept. `src/backend.rs` `GlesTextureBind` grows two fields
(`src/backend.h` mirrors them; the ABI pins `src/abi_layout.rs` and
`src/abi_layout_check.c` are regenerated with `yakogl-tools abi-layout`;
the struct is 72 bytes, from 64):

- `wrap_r: u8` (`GlesWrap`, at the byte that was padding, offset 43): the
  third axis. The bound sampler object's when there is one, else the
  texture's, exactly like `wrap_s` and `wrap_t` (`es3::sampler_state` now
  returns the five values; `tf_capture::bound_of` takes the same tuple, so
  the transform-feedback evaluator applies a sampler object's wrap R too,
  which it did not). Every binding carries it; a 2D one has nothing to
  apply it to. GL's default (REPEAT) is 0.
- `swizzle: [u8; 4]` (`GlesSwizzle`, at offset 64): `GL_TEXTURE_SWIZZLE_R`,
  `_G`, `_B`, `_A`, each as `GLES_SWIZZLE_SAME` (0, "the component's own
  channel"), or `_RED`, `_GREEN`, `_BLUE`, `_ALPHA`, `_ZERO`, `_ONE` (1 to
  6). The identity is all zero, so a zeroed bind (and every bind of a
  `report-es2` build, where the parameter is refused) is the identity.
  Texture state, never a sampler object's (ES 3.0.6 3.8.2). It is in the
  *base format's* terms: a `GL_LUMINANCE` texture's green is L and a
  `GL_RGB` texture's alpha is 1 before the swizzle, whatever the storage
  class holds, so a back end composes it with its format's own channel
  mapping (`no_alpha`, `depth_luminance`, an L8/A8/LA8 storage), as Mesa's
  three drivers compose a view's swizzle with the format's. `backend::
  swizzle_source(k, s)` is the one table for that: the channel (0 to 3) or
  constant (4 zero, 5 one) component `k` reads under code `s`.

`src/gl/draw.rs` fills both in the binding loop and in `texture_bind_2d`
(the bind of the upload hook, `generate_mipmap` and `storage_current`, so
a back end sees one set of values everywhere). `bind_swizzle` maps the GL
enums. Nothing else in the GL core changes: the parameters were already
validated, stored and read back; the ES 2.0 surface still refuses them
(`tests/es2_report_guard.rs` unchanged, and `tests/es3_tex_filtering.rs`
pins the ES 2.0 bind as all-zero).

The C mirror also gains `bool uniforms_host_only` in `struct
yakogl_backend`, which `c4af4917` added to the Rust table without the
header: the ABI tool's probe did not compile until it was there, and the
regenerated pins now cover it (offset 212, the size unchanged).

The tests (`tests/es3_tex_filtering.rs`, a stand-in back end that records
every draw's bindings and the upload hook's bind, the in-process compiler):
the swizzle codes for dEQP's `bgra`, `one_one_red_green`, `all_zero`,
`abgr` and single-channel cases; the same codes for all seven 8-bit formats
with `no_alpha` untouched; a sampler object replacing wrap S but not the
swizzle; wrap R from the texture, from the sampler object, from the texture
again; a `sampler3D` binding of a 4x8x8 texture with all three wraps and a
swizzle; the upload hook's bind; `swizzle_source`; and under `report-es2`
the refused parameters and the all-zero bind.

What this does **not** do: make a board pass. A swizzle or wrap R case
passes only when the back end applies the field (section 3's back-end
rows), and the PowerVR 3D cases need `pvr-3d` besides. The host figures of
section 1 cannot move either way.

## 5. What each back end has to do (for the items that own them)

- **PowerVR** (`src/pvr/backend.rs`): in `build_tex_desc`, `swiz0..3` of
  IMAGE_WORD0 = the format's swizzle indexed through `swizzle_source(k,
  b.swizzle[k])` (ZERO and ONE straight to `SWIZ_ZERO` / `SWIZ_ONE`);
  `addrmode_w` = `tpu_addrmode(b.wrap_r)` once the TPU samples a real 3D
  image (`pvr-3d`); until then the grid shader would have to wrap z itself.
  Mesa: `pvr_arch_image.c:166-191`, `pvr_arch_tex_state.c:40-64, 199-205`.
- **GC7000** (`src/vivante/texture.rs`): `texdesc_swizzle_*` from the
  composed table (lines 202-205); `samp_wwrap` from `b.wrap_r` (388), with
  the same one-texel clamp rule as U and V. Mesa: `etnaviv_format.c:347`,
  `etnaviv_texture_desc.c:108, 150`.
- **V3D** (`src/v3d/texture.rs`): `tmu_format`'s swizzle composed with
  `b.swizzle` (Mesa puts the composed swizzle in the texture shader state
  for 16-bit returns and in the shader key for 32-bit ones,
  `v3dx_state.c:1059-1063`); `sampler().wrap_r = wrap(b.wrap_r)`; and 3D
  sampling itself. Mesa: `v3dx_state.c:595, 783, 1031-1034`.
- The transform-feedback evaluator (`src/gl/tf_capture.rs`) already applies
  both from the texture (it reads `GlTexture` directly) and is the
  reference a back end's result can be compared with on the host.

## 6. Reproducing it

```sh
# the host dEQP of one group on one GPU (the deps cache holds the CTS):
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 \
    --cts="$CONNECTED_DEPS_DIR/VK-GL-CTS" --out=build/host-deqp/run-pvr-swizzle texture.swizzle.
# per-subgroup figures: the <Result StatusCode="..."> line of each case in
# build/host-deqp/run-*/c_*.qpa (tools/cts/cts-summary.py stops at the
# third level)
# the GL-core tests, ES 3.0 and ES 2.0:
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tex_filtering
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_tex_filtering
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_report_guard
```

The "before" tree was the working tree copied under `build/` with this
branch's files put back from HEAD (`git show HEAD:<file>`), the dEQP binary
shared; `host-deqp-run.sh` names such a run `70142d35-dirty`.

## 7. The AM62P `texture.units`, `texture.swizzle` and `sglrGLContext.cpp:863` rows

**The compile refusals of `units` and `specification` are one cause.** The
HOST-ONLY run at `be84094b` (`--gpu=powervr --module=gles3`): `texture.units`
159 cases, 103 compile refusals; `texture.specification` 1,163 cases, 323
compile refusals. Both groups' cases have **one** reason, the same text:
`Usc(Select(Unsupported("a texture fetch with an integer result on a cube,
a 3D texture or an array, gradients, a projective cube or array, or an offset
on a stacked texture, is not selected yet")))` -- the last `ir::Op::Tex` arm
of `src/shaderc/online/powervr/select.rs` (line 1294). dEQP records it as
`Compile failed at sglrGLContext.cpp:863`. The 103 and the 323 equal the
AM62P compile-failure counts of `texture.units` and
`texture.specification`, so the compile half of the two groups is this one
selector refusal: their shaders take an integer result from a cube, a 3D
texture or an array, or use one of the other shapes that arm covers --
RC2's "integer cube, 3D and 2D-array samplers", left on the PowerVR, in
`docs/es3-cts-failures.md`.

**The other 67 `sglrGLContext.cpp:863` rows are not claimed here.** The TSV
has 493 of them: the 103 and 323 above, `fbo.color.texcube`,
`fbo.color.tex3d` and `fbo.color.tex2darray` 19 each, and
`fbo.depth.depth_test_clamp` and `fbo.depth.depth_write_clamp` 5 each. Those
67 fbo rows were not run on the host in this session and are not attributed
to this cause; RC9 of `docs/es3-cts-failures.md` does put
`fbo.color.{texcube,tex3d,tex2darray}` among the same family of PowerVR
code-generator refusals, and records nothing about the ten
`fbo.depth.depth_*_clamp` rows.

**The 17 image-failed `texture.units` rows: ten known, seven not.** The ten
`only_2d_array.*` and `only_3d.*` rows compiled (their message is
`Image comparison failed`, not the compile refusal) and sample an array or
a 3D texture, so they fail as `filtering.2d_array` and `filtering.3d` do:
RC6 and RC15. The seven others -- `2_units.mixed.3`, `.5`, `.6`, `.8`,
`4_units.mixed.5`, `.8` and `8_units.only_2d.7` -- are **not yet
explained**: which types the `mixed.*` cases bind per unit, and why
`only_2d.7` fails with only 2D textures, was not established here (no CTS
checkout on this host, and the host renders nothing -- the next step is the
type table of `es3fTextureUnitTests.cpp` on a host that has it, or a board
session).

**`texture.swizzle` is the section 3 back-end gap, not a compiler one.**
The host refuses nothing in it (section 1: swizzle 0 compile, upload and
draw refusals on the PowerVR); all 80 AM62P rows are image failures. The
core delivers the swizzle (`GlesTextureBind::swizzle`, sections 3 and 4)
and the PowerVR back end never reads it: `build_tex_desc`
(`src/pvr/backend.rs`, line 4552) receives the bind, but IMAGE_WORD0's
`swiz0..3` come from the texel format's `TexFormat` table alone
(`tex_image_word0`, line 1224). The three "pass" rows are the black-on-black
cases of section 2.

**Owner.** The selector refusal and the unapplied swizzle each take one
GitHub issue under the ES 3.0 umbrellas #147 and #748 (duplicates checked
first; RC2 and RC9 carry their facts meanwhile). This session had no GitHub
access, so neither issue is cited by a number here.
