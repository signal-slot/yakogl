<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 CTS failure inventory

dEQP-GLES3, the mustpass list `tools/cts/gles3-mustpass.txt` (42,810 cases).
This file says what fails now, why, and what fixes it. The numbers are those
of `tools/cts/RESULTS.md`; earlier runs and their analysis are in git history.
Layers: **GL core** = `src/gl/**`; **shaderc** = `src/shaderc/**`;
**PowerVR** = `src/pvr/**` (AM62P); **V3D** = `src/v3d/**` (Pi 4);
**GC7000UL** = `src/vivante/**` (i.MX8MP).

## Current state

**AM62P**, whole tree on `ad2b5ed` (same `src/`, `driver/` and `capi/` as main
`a63fcc5b`), 2026-10-14: **36,552 pass / 2,880 fail / 2,195 NotSupported /
185 QualityWarning / 998 without a result**. Every case that is not Pass, with
its first message: `docs/es3/am62p-gles3-notpass.tsv`. The counts below are
read from that file.

Rows re-run later (2026-10-14, 0 resets) pass in full and have no failure in
the file either: `transform_feedback` 882 / 882, `ubo` 1,099 / 1,099,
`occlusion_query` 62 / 62, `fragment_ops.scissor` 26 / 26,
`fragment_ops.blend.fbo_srgb` 1,152 / 1,152; `fbo.blit.conversion` 325 pass /
0 fail / 112 NotSupported (every NotSupported one has an `r32f`, `rg32f`,
`rgba32f` or `r11f_g11f_b10f` end).

**Pi 4**: 9 of the 91 images of `tools/cts/pi4-gles3-tree.txt` on `925dbca9`
(4,917 cases: 3,943 pass / 643 fail / 331 NotSupported, 0 resets; commit
`96810ab2`, `tools/cts/RESULTS.md`). Every case that is not Pass:
`docs/es3/pi4-gles3-fails.tsv` (974 rows). By subgroup: `fbo.render` 305,
`fbo.color` 206, `fbo.blit` 194, `draw_buffers_indexed` 76 (NotSupported),
`fbo.msaa` 66, `fbo.completeness` 45, `fbo.invalidate` 38,
`attribute_location.mixed_max_attributes` 20, `fbo.depth` 10,
`implementation_limits` 11. Against the earlier seven images on `cd738b1a`:
`attribute_location` 48 -> 20, `draw.random` 14 -> 0, `fbo.blit` 308 -> 48 Fail,
`fbo.completeness` 134 -> 38. Rows 9-90 of the tree have not run on this commit.

**Host (HOST-ONLY: API and compile verdicts, never pictures)**: per group on
the three GPUs at `70142d35`: `docs/es3/host-gles3-tip.tsv`. Since then the
GC7000UL library samples eight more texture classes (`541975a2`). Measured at
`541975a2`, HOST-ONLY: `texture.format` 546 cases, 498 fail / 48 NotSupported,
upload refusals 200 -> 64 (compile 0, image 434); `texture.specification`
1,163 cases, 6 pass / 1,157 fail, upload refusals 448 -> 141 (compile 0,
image 1,013, framebuffer incomplete 3). `docs/es3/gc-formats.md` section 5.

**i.MX8MP**: whole tree on `d1b24389`, 2026-10-31 (counts only, case names not yet recorded): 37,140 pass / 5,274 fail / 2,197 NotSupported / 24 other / 0 without a result over 44,635 cases begun (`tools/cts/RESULTS.md`, `tools/cts/log/2026-10-31-imx8mp-es3/`). Largest groups: `shaders` about 2,000, `fbo_srgb` blend 1,104 (RC7), `fbo` 494, `texture.shadow` 432, `fragment_out` 263.

## Feature state

Host numbers are HOST-ONLY: the Linux OSAL runs no command stream, so only API errors and compile and link outcomes mean anything there (`tools/cts/README.md` section 2b). Under `report-es2` every ES 2.0 answer is kept, pinned by `tests/es2_report_guard.rs`.

| Feature | State | Design |
|---|---|---|
| Transform feedback | GL core on the CPU for the three GPUs: the vertex shader runs for every captured vertex, with its texture look-ups on 2D, cube, 3D and 2D-array samplers (`src/shaderc/online/eval.rs` `TexelSource`, `src/gl/tf_capture.rs` `GlTexels`). Host `transform_feedback` 882 / 882; board: AM62P rows pass, gl_test 203 on the AM62P and the Pi 4 | `docs/es3/transform-feedback.md` |
| Uniform blocks | host `tests/es3_ubo.rs` (dEQP `functional.ubo`, 2,032 cases); the GC7000 refuses four at link (`KNOWN_LIMITS`) | `docs/notes/gc-ubo-const-limit.md` |
| Sized internal formats | the 35 colour formats of ES 3.0 table 3.2, converted at upload; the PowerVR and the V3D (`cc1c6ede`) sample every class but `RGB10A2`; the GC7000UL samples eight more classes (`dc83be67`, `541975a2`: R11G11B10F, RGB9E5, RGB10A2UI, RGBA8_SNORM, RGBA8UI/I, RGBA16UI/I) and refuses or keeps in a fallback class RGBA32F/UI/I, R32I/UI, RG32I/UI and ETC2; host only, no board has run the V3D or GC7000UL classes | `docs/es3/sized-formats.md`, `docs/es3/pvr-formats.md`, `docs/es3/gc-formats.md` |
| Occlusion queries | counted on the three back ends; board: AM62P passes, Pi 4 reads 0 (gl_test 196) | `docs/es3/occlusion-query.md` |
| Integer colour render targets | PowerVR: all nineteen as a draw's one colour attachment (board: gl_test 200); V3D: all nineteen, the float ones and the ES 2.0 formats on any attachment, host only (`src/v3d/state.rs` `rt_format`, `tests/es3_int_rt_v3d.rs`); GC7000UL: 13 of the 19 (`1324a160`, `docs/es3/int-rt-gc.md`: RGBA16I/UI and RG32I/UI mapped but held back, RGBA32I/UI have no hardware format), host only | `docs/es3/pvr-formats.md` section 10, `docs/es3/int-rt-gc.md`, `docs/online-shaderc/v3d-backend.md` |
| `glBlitFramebuffer` | colour on the CPU, shared by the three back ends; depth on the CPU into a depth texture from a depth texture or renderbuffer (host only, `tests/es3_blit_ds.rs`); depth into a renderbuffer or the window, depth from the window, and all stencil refused by name | `src/gl/blit.rs`, RC8 |
| sRGB | PowerVR renders and samples (board: gl_test 204); the GC7000 and V3D sample and refuse a draw into sRGB by name | `docs/es3/srgb.md` |
| Buffer mapping and pixel buffers | host; board: gl_test 205 on the AM62P and the Pi 4 | |
| NPOT levels of arrays and storage textures | valid at every level | |
| Framebuffer completeness | host `fbo.completeness`: 17 cases Fail to Pass on the PowerVR and the GC7000UL (`src/gl/framebuffer.rs` `check_status` under `!REPORT_ES2`, `tests/es3_fbo_fragops.rs`) | RC17 |
| Stage-interface link rules | interpolation and block mismatches refused | |
| GLSL ES 3.00 in the compiler | 3,457 of 3,461 expected-build dEQP programs build on each GPU (`tests/es3_shaderc_corpus.rs`) | `docs/es3/shaderc-coverage.md`, RC13 |
| Vertex array conversion | a buffer array the GPU cannot fetch as it lies (packed 2_10_10_10, `GL_HALF_FLOAT`, `GL_INT` / `GL_UNSIGNED_INT` through `glVertexAttribPointer`) is converted to float32 rows like a client array (`src/gl/draw.rs` `attrib_in_place_layout`); host only, `tests/es3_vertex_arrays.rs` | |
| `layout(location)` on vertex inputs | `glBindAttribLocation` does not move a qualified input (`src/gl/program.rs` `ExplicitAttribs`); host only, `tests/es3_attrib_location.rs` | |
| Multiple render targets | GL core: the draw-buffer count is the back end's `DRAW_TARGETS` (3 on every GPU); the framebuffer stores 4 colour points | RC14 |
| PowerVR 2D-array and shadow sampling | built, host only: 2D arrays sampled by layer address (`e160bafb`, switch `pvr_native_array`, default off; the dEQP image flips it with `CONFIG_DEQP_PVR_NATIVE_ARRAY`, `1ddb0cf3`); every ES 3.00 shadow look-up form compiles and a LINEAR sampler over one level compares per texel (`d7b2cf8b`). Not done: mipmap and cube PCF, mirrored-repeat weights, ETC arrays | `docs/es3/pvr-2darray.md`, RC5, RC6 |
| Texture swizzle and wrap R | the GL core hands both to the back end (`GlesTextureBind::swizzle`, `::wrap_r`, `e4f58cbe`); no back end fills them yet | `docs/es3/texture-host-repro.md` |
| ES 3.0 limits | `tests/es3_caps.rs` against the spec minimum and the hardware ceilings | `docs/es3/backend-caps.md` |

## AM62P failures by group

| Group | Fail | Where they are | Causes |
|---|---:|---|---|
| texture | 1,351 | specification 467 (3D and 2D-array uploads and the `*_pbo` cases, many "Compile failed" at `sglrGLContext`), filtering 244 (3d 129, 2d_array 115), format 158 (150 "Failed to compile shader program"), units 120, shadow 108 (all `2d_array`), vertex 89 (3d 56), mipmap 85 (3d 64), swizzle 80 | RC15 3D textures, RC6 2D arrays, RC2 integer samplers, RC9 |
| fragment_out | 583 | basic and array `int` / `uint` 456, array `float` 36, array `fixed` 12, random 79 ("Image comparison failed") | RC1 (integer targets beside another, partial masks), RC14 |
| shaders | 410 | texture_functions 297 (compile refusals: `textureGrad`, `textureProjGrad`, offsets, `texelFetchOffset`; 38 `texturesize` "Got unexpected texture size"), builtin_functions 40 (`common.round` / `roundeven` 16, `modf` 24), precision 24 (`int` / `uint` program does not link), derivate 23 (15 float framebuffer incomplete), fragdepth 15 (compile), qualification_order 7, uniform_block 2, metamorphic 2 | RC13, RC1, RC16 |
| fbo | 367 | color 147, render 68, completeness 56, msaa 34, invalidate 33, blit 19 (16 `blit.depth_stencil`), depth 10. 146 are `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` | RC1, RC8, RC17 |
| attribute_location | 48 | `mixed.*` and `mixed_time.*` 28, `mixed_max_attributes.*` 20 | Appendix A, task T4 |
| vertex_arrays | 41 | packed `int2_10_10_10` in a buffer 20, `components{2,3}_{i,u}vec4` 21 | Appendix A, tasks T2 and T3 |
| state_query | 25 | 64-bit and float reads of ES 3.0 limits, `rbo` component size, `program_binary` | RC10 |
| draw | 14 | `random.*` image comparison | not classified |
| implementation_limits | 11 | limits below the ES 3.0 minimum (`max_draw_buffers`, `max_color_attachments`, varying and uniform components, compressed formats) | RC10, RC14 |
| rasterization | 11 | `flatshading.*` 9, `fbo.rbo_multisample_max` 2 | not classified; RC8 |
| negative_api | 7 | `texsubimage{2d,3d}`, `uniformui{,v}_incompatible_type`, `read_pixels_fbo_format_mismatch`, `blit_framebuffer{,_multisample}` | RC8, not classified |
| instanced | 6 | `{draw_arrays,draw_elements}_instanced.mixed.{2,4,20}_instances` | Appendix A, task T6 |
| multisample | 4 | `fbo_max_samples.*` coverage cases | RC8 |
| uniform_api | 1 | `random.83` (compile) | RC13 |
| lifetime | 1 | `attach.deleted_output.buffer_transform_feedback` | not classified |
| **total** | **2,880** | | |

**NotSupported 2,195**: texture 1,656, fbo 255, fragment_out 115,
`draw_buffers_indexed` 76, multisample 48, shader_api 14, rasterization 12,
state_query 9, pbo 6, negative_api 4. Most are formats a back end refuses by
name (RC2) and the draw-buffer limit (RC14).

**No result 998**: image 7 stops at
`shaders.builtin_functions.precision.clamp.highp_fragment.vec3` with a
`ResourceError` (987 cases after it; RC16), and 11 `texture.filtering.3d`
cases did not start (RC15).

**QualityWarning 185**: `texture.shadow` 180 (compare after filter on the 2D
and cube LINEAR / mipmap cases) and `rasterization` 5. dEQP counts them as
passes.

## Root causes

### RC1. Integer and ES 3.0 colour render targets

| | |
|---|---|
| Layer | GL core, then each back end (integer fragment outputs through the PBE and their read-back) |
| Done | GL core accepts the formats; the PowerVR writes all nineteen integer formats as a draw's one colour attachment; the V3D writes them, the float ones and the ES 2.0 formats among any attachments (host only); the GC7000UL draws 13 of the 19 (`1324a160`, `docs/es3/int-rt-gc.md`), host only. Host tests: `tests/es3_int_rt.rs`, `tests/es3_int_rt_v3d.rs` |
| Left on the PowerVR | an integer attachment beside other attachments (this is why `builtin_functions.common.modf` fails: dEQP's executor binds two integer attachments), colour masks that keep some of an integer target's channels, precompiled rows for the integer variants. Most of `fragment_out`'s 456 `int` / `uint` failures and part of `fbo.color` are here |
| Left on the V3D | a board run (gl_test 200 on the Pi 4), an integer clear program for a scissored or masked clear |
| Float targets | `shaders.derivate` 15 fail on an incomplete RGBA32F / RGBA16F framebuffer: float targets need `GL_EXT_color_buffer_float`, which is not claimed. A decision, no item yet |

### RC2. ES 3.0 sized texture formats; integer and packed-float textures

| | |
|---|---|
| Layer | GL core (upload, storage, conversion), then each back end (sampling the format; isampler / usampler) |
| Done | the 35 colour formats of ES 3.0 table 3.2 are converted at upload (`docs/es3/sized-formats.md`); the PowerVR samples every class but `RGB10A2` (`docs/es3/pvr-formats.md`); host test `tests/es3_sized_formats.rs` |
| Done since | V3D: every class but `RGB10A2` (`cc1c6ede`, `docs/es3/backend-caps.md`). GC7000UL: eight classes (`dc83be67`, `541975a2`); integer look-ups select as the float look-up (`docs/es3/gc-formats.md`). Both host only |
| Left | PowerVR: the AM62P run of the integer cube, 3D and 2D-array samplers (they select since the stacked arms take the return type; host-tested only), and `RGB10A2` in its own class (it needs a 10/10/10/2 unorm render-target class first, then the framebuffer guard and `glGenerateMipmap` in the GL core). GC7000UL: RGBA32F/UI/I (Mesa's two-plane path: a second sampler slot and a per-draw shader key), R32I/UI and RG32I/UI (hardware formats, no GL-core class), ETC2 (decoded as ETC1; Mesa uses another format, a board question). V3D: ETC2, `RGB10A2` sampling, and `texelFetch`, offsets, shadow and 2D-array/3D look-ups, all refused by name; vertex-stage look-ups of the 32-bit classes |

### RC3. Transform feedback

Built: the GL core runs the vertex shader on the CPU for every captured
vertex, on the three GPUs (`docs/es3/transform-feedback.md`). The board agrees:
`transform_feedback` 882 / 882 on the AM62P, gl_test 203 on the AM62P and the
Pi 4. Left: `lifetime.attach.deleted_output.buffer_transform_feedback`; a
vertex shader that samples with a shadow look-up, `textureGrad`, a compressed
texture or a base level other than 0 is refused by name.

### RC4. Compile failure with an empty info log

The 48 `transform_feedback` cases that failed with `Compile failed: ''` pass
in the re-run on `cd738b1a`; the refusal now names its step
(`e77a039a`, `tests/es3_shaderc_infolog.rs`). Closed.

### RC5. Shadow samplers

The online compiler takes the shadow, projective and 3D bias forms; the 2D
and cube shadow cases pass or warn (180 QualityWarning). Since the AM62P run (`ad2b5ed`): every shadow look-up form compiles and a
LINEAR sampler over one level compares per texel (`d7b2cf8b`, host). Left: the
108 `texture.shadow.2d_array` failures (RC6) and mipmap and cube PCF.

### RC6. 2D-array textures sampled as one tall image (PowerVR)

The TPU filters and wraps across layers. `texture.filtering.2d_array` 115,
`texture.shadow.2d_array` 108 and the 2D-array cases of `texture.specification`
and `fbo.color.tex2darray` fail on `ad2b5ed`. Built on the host since
(`e160bafb`, `d7b2cf8b`): a derived twiddled copy per array and the TPU's
2D-array type, behind `pvr_native_array` (default 0; `1ddb0cf3` lets the
dEQP image flip it). The board session should run the 2d_array rows with
both arms and, if clean, flip the default. Unverified on silicon, including
the copy's cost. `docs/es3/pvr-2darray.md`.

### RC7. sRGB

The PowerVR encodes on write and blends in linear space
(`docs/es3/srgb.md`); `fragment_ops.blend.fbo_srgb` passes 1,152 / 1,152.
Left: the GC7000UL's and the V3D's sRGB render targets (they sample the class
and refuse a draw into it by name), an sRGB attachment beside another on the
PowerVR, compressed sRGB.

### RC8. glBlitFramebuffer and multisample

Colour blit: a CPU copy shared by the three back ends (`src/gl/blit.rs`);
`fbo.blit.conversion` passes. Depth: copied on the CPU into a 2D depth
texture's texels, from a depth texture or a depth renderbuffer (`blit-ds`,
host). The next draw into that texture (and after a glTexImage2D /
glTexSubImage2D of depth) tests against the new texels: the GL core hands
them to the back end with the draw (`GlesDraw::depth_texels`), and each back
end writes them into its depth buffer first (host-tested on all four, not
yet on a board). Depth into a renderbuffer and stencil between
renderbuffers copy through two back-end entries: a CPU write of a depth or
stencil rectangle (`ds_write`) and a read of a buffer's stencil plane
(`stencil_resolve`). The PowerVR implements both (`pvr_backend_ds_write`,
`pvr_backend_stencil_resolve`); the other back ends leave them out, and a
blit that needs them stays `GL_INVALID_OPERATION`, logged once. Refused
everywhere still: the window as either side of a depth blit, stencil in or
out of a texture, and depth from a cube or array depth texture
(`docs/notes/blit-depth-window.md`). The 16 `fbo.blit.depth_stencil`
failures, which blit between renderbuffers, are for a board run to
re-measure. Multisample
renderbuffers: `fbo.msaa` 34, `multisample.fbo_max_samples` 4,
`rasterization.fbo.rbo_multisample_max` 2; no item yet.

### RC9. "Compile failed" at `sglrGLContext`

The fbo and texture test shaders (isampler / usampler, sampler2DArray,
sampler3D, integer outputs) are refused by the PowerVR code generator:
`fbo.color.{texcube,tex3d,tex2darray}` 19 each, `texture.units.8_units` 40,
the `texture.specification` 3D and `*_pbo` cases. Fixed by RC2 (integer
samplers), RC6 and RC15; no single item.

### RC10. Limits and state queries

`state_query` 25 and `implementation_limits` 11: ES 3.0 limits read as float
or 64-bit integers, uniform-block and varying component limits below the ES
3.0 minimum, the compressed-format list, `max_draw_buffers` and
`max_color_attachments` 3 (RC14). Item `es3-caps` (the ES 3.0 limits test was
reverted for review and is to be redone).

### RC11. Occlusion queries

Counted on the three back ends (`docs/es3/occlusion-query.md`).
`occlusion_query` 62 / 62 and gl_test 196 pass on the AM62P. On the Pi 4 gl_test
196 fails (all five results 0): the V3D counter, open.

### RC12. default_vertex_attrib int / uint current values

Closed: 0 failures. Appendix A has what remains of the vertex groups.

### RC13. GLSL ES 3.00 in the online compiler

Over dEQP's 5,596 `data/gles3/shaders/*.test` programs
(`docs/es3/shaderc-coverage.md`, `tests/es3_shaderc_corpus.rs`), the PowerVR,
GC7000UL and V3D each build 3,457 of the 3,461 that must build. Left on the
PowerVR: 2 `constant_expressions.other.complex_*` (`gl_MaxDrawBuffers`, RC14)
and 2 `uniform_block.valid.long_instance_name_*` (a reflection name of 63
characters or more; needs a decision on the 64-byte field).

| Cause | Layer | PowerVR / GC7000UL / V3D programs | State |
|---|---|---:|---|
| Interpolation and uniform-block link mismatches | shaderc link | 0 / 0 / 0 | fixed |
| Backslash-newline inside a token | harness | 0 / 0 / 0 | fixed (dEQP removes the indentation; the corpus now does too) |
| `centroid in` | shaderc | 0 / 0 / 0 | fixed |
| Indexed element passed as an out/inout argument | shaderc | 0 / 0 / 0 | builds on all three (`docs/notes/shaderc-inout-indexed.md`) |
| `gl_MaxDrawBuffers` (two `complex` programs) | GL core / limits | 2 / 2 / 2 | RC14 |
| Reflection name of 63+ characters | shaderc / GL core | 2 / 2 / 2 | decision needed |
| 128-element constant array | GC7000UL | 0 / 0 / 0 | fixed (register pressure in `sink_definitions`) |

The 645 `shaders` failures that no earlier file tied to a cause are accounted
for in `docs/es3/shader-host-repro.md` (host run of all 20,616 cases at
`70142d35`, HOST-ONLY): 267 refusals on failing cases in 6 reasons, none in the
front end; 395 of the 645 are in its cause rows and 250 cannot be reproduced
(main `396d667a` left no per-case list). The causes are back-end refusals
(PowerVR texture look-ups 226, integer outputs from ALU results 24,
`gl_FragDepth` 15, no owner on any GPU), the second and fourth draw buffer
(RC14), multisample (RC8), the 64-byte reflection name (2) and values only a
board can judge.

The board's `shaders.texture_functions` failures (297) are mostly code
generator refusals outside that corpus: the PowerVR refuses `textureGrad`,
`textureProjGrad`, their offset forms, `texelFetchOffset` and integer results
from 2D-array, cube and 3D samplers. `texturesize` (38) reads a wrong size.
`builtin_functions.common.round` / `roundeven` (16) are fixed on the host
(`tests/es3_shaderc_fold.rs`), not yet run on a board. `precision.{int,uint}`
(24) do not link, `fragdepth` (15) does not compile: not classified further.

### RC14. MRT: three draw buffers

The GL core is done for any count: `GL_MAX_DRAW_BUFFERS`,
`GL_MAX_COLOR_ATTACHMENTS`, `gl_MaxDrawBuffers` and the attachment checks read
the back end's `DRAW_TARGETS`, which is 3 on every GPU; ES 3.0 asks for 4.
Each GPU raises its arm in `src/backend.rs` to 4 once a board draws four
targets. PowerVR: a fourth PBE state and end-of-tile program
(`PVR_RENDER_MAX_RT` 3 in `driver/src/pvr_render.h`). V3D: the TLB, compiler
and back end take 4; a board run is left. GC7000UL: the PE has 4 render
targets; check the fourth output on the board. This explains
`implementation_limits.max_draw_buffers` / `max_color_attachments`, the 76
`draw_buffers_indexed` NotSupported and part of `fragment_out`.

### RC15. PowerVR 3D textures

A 3D texture is emulated as a one-level grid of slices: the TPU samples level
0 only and loses REPEAT on all three axes. `texture.filtering.3d` 129,
`texture.mipmap.3d` 64, `texture.vertex.3d` 56 and the 3D specification
cases fail. dEQP's check of a wrong 3D `linear_mipmap_linear` picture takes
minutes per case, so images 20 and 21 run out of their time and 11 cases do
not start; it is not a GPU hang (`docs/notes/pvr-3d-mipmap-hang.md`). Fix
(item `pvr-3d`): Mesa's way, the TPU's own 3D type (3D-twiddled layout, the
full chain, `addrmode_w`). Built, host only: switch `pvr_native_3d` (default
off, `CONFIG_DEQP_PVR_NATIVE_3D`), `docs/notes/pvr-3d-native.md`; the board
run of `texture.mipmap.3d` and `texture.filtering.3d` is next. Also: `tools/cts/gen-am62p-gles3-tree.py` should
name only the mustpass prefixes in `3d.formats` and give each 3D
`*_linear_mipmap_linear` case an image of its own.

### RC16. Large draws (PowerVR, image 7)

The 24,384-vertex `glDrawArrays` of
`shaders.builtin_functions.precision.clamp.highp_fragment.vec3` (four client
arrays, 1.27 MB) did not fit the 1 MiB general arena and was refused whole.
Fixed on the host: `draw_dispatch` (`src/pvr/backend.rs`) cuts a sequential
draw into pieces as it did an indexed one. On the host the precision group
runs all 1,147 cases (0 `ResourceError`). Not yet run on a board.

### RC17. ES 3.0 framebuffer completeness

Upload refusals in `fbo.completeness` (a case that ends in
`GL_INVALID_OPERATION` at `glTexImage2D`, the refusal by name of
`es3_storage_or_refuse`): the PowerVR and the V3D refuse no format of ES 3.0
table 3.2. The GC7000UL refuses the eight 32-bit integer formats (R/RG/RGB/RGBA
32I/UI): the descriptor has no class for them (`docs/es3/gc-formats.md`
section 3), so the refusal stays and is the one answer for every case that
names them (`renderable.texture.{color0,depth,stencil}.*32*`).
`tests/es3_sized_formats.rs`,
`the_formats_a_back_end_refuses_to_upload_are_listed`, pins the list per back
end. Every other `fbo.completeness` failure is a framebuffer verdict, not an
upload.

Done on the host: depth larger than colour, colour on attachments 1.. alone,
separate depth and stencil with no colour, a `GL_RGBA32F` texture attachment
(the texture remembers its sized format, `GlTexture::rgba32f`; incomplete
while `GL_EXT_color_buffer_float` is not claimed:
`renderable.texture.color0.rgba32f`), the 4 unsized depth textures
(`renderable.texture.depth.*` and `texture.stencil.depth_stencil_unsigned_int_24_8`:
a texture made with `GL_DEPTH_COMPONENT` or `GL_DEPTH_STENCIL` as its
internal format is no attachment under `!REPORT_ES2`, `GlTexture::unsized_depth`;
the ES 3.0 build claims no `GL_OES_depth_texture`, so glmark2's shadow and
refract scenes, which need it, were already skipped there and lose nothing)
(`tests/es3_fbo_fragops.rs`), and
the four-channel integer texture attachments, on attachment 0 and beyond
(`tests/es3_int_rt.rs`; the cases on the board are open). The R and RG integer
textures stay incomplete: they are held as a widened four-channel store, so
they wait for the back end to write the narrow surface into it (RC1).
Left, each in the GL core unless named: separate depth and stencil images
with a colour attachment 0 (12 `attachment_combinations` cases),
`GL_R32F` as a render target (a decision: it wants
`GL_EXT_color_buffer_float`).
Done on the host: a slice of a 3D texture tiled in a grid is a render target
at the grid's row pitch on colour attachment 0 (`completeness.layer.3d_64_15`;
`attachment_storage`, `loc_pitch`). On colour attachments 1.. (drawn at
`tex_pitch(w)`) such a slice is `GL_FRAMEBUFFER_UNSUPPORTED`
(`slot_pitch_ok`). Not yet run on the board.
The 10 `fbo.depth` clamp cases are done on the host: the PowerVR back end now
builds the written `gl_FragDepth` -- a saturating `fadd` clamps it to [0, 1],
`depthf` hands it to the ISP, the draw runs the DEPTH_FEEDBACK pass
(`ShcShader::writes_depth`, `tests/es3_pvr_fragdepth.rs`); a program feeding
depth into a framebuffer with no colour attachment is refused by name. Not
yet run on a board, and the `depthf` encoding is unchecked against a pco
program (the corpus has none that writes `gl_FragDepth`).

### RC18. Texture swizzle and wrap R

`texture.swizzle` (83) and `texture.filtering.3d` (153) failed on the AM62P
(`ad2b5ed`) partly because the GL core stored `GL_TEXTURE_SWIZZLE_*` and
`GL_TEXTURE_WRAP_R` and never handed them to a back end. Done (`e4f58cbe`):
`GlesTextureBind::swizzle` and `::wrap_r`, `backend::swizzle_source`. Host
dEQP before and after on the three GPUs: identical (a harness that renders
nothing cannot show the fix), `tests/es3_tex_filtering.rs` pins the binds.
Left, each a back-end site: PowerVR `build_tex_desc` (`swiz0..3`,
`addrmode_w`; 3D itself is RC15), GC7000UL `texdesc_swizzle_*` and
`samp_wwrap`, V3D `tmu_format` / `state` and `sampler().wrap_r`, plus V3D 3D
sampling. On the V3D host library `texture.filtering` still refuses 227
look-ups (2D-array and 3D programs) and 72 uploads at `70142d35`, before
`cc1c6ede` (`docs/es3/texture-host-repro.md` section 1).

## Appendix A: the vertex groups

A *current value* is what a vertex input reads while its array is disabled
(`glVertexAttrib*`); a *pure-integer* current value is one set with
`glVertexAttribI4*`; a *packed type* is `GL_INT_2_10_10_10_REV` or
`GL_UNSIGNED_INT_2_10_10_10_REV`. The pure-integer current values are stored
as their bits (`src/gl/vertex.rs`, `set_current_bits`), so
`default_vertex_attrib` passes. `glGetVertexAttribiv(GL_CURRENT_VERTEX_ATTRIB)`
rounds to nearest (ES 3.0 6.1.2). Under `report-es2`, `glVertexAttribPointer`
refuses `GL_HALF_FLOAT` and the packed types, and the getters refuse
`GL_VERTEX_ATTRIB_ARRAY_INTEGER` and `_DIVISOR` (`tests/es3_vertex_api.rs`).

### Tasks

- **T1, typed current value** (open): `GlContext::current_attrib` records a
  value's dwords, not whether a float or an integer command set it. ES 3.0
  2.7 and 6.1.14 leave the mixed reads undefined, and dEQP reads only the
  defined pairs; Mesa converts. Add a per-attribute tag (float, int, uint) in
  `src/gl/context.rs`, keep `current_attrib` for the ABI (`src/abi_layout.rs`
  pins its offset), and convert in `glGetVertexAttribfv/iv` and
  `glGetVertexAttribI*v`.
- **T2, ES 3.0 array formats in a buffer** (fixed on the host, not yet on a
  board): a buffer array of a type the GPU cannot fetch as it lies (packed,
  `GL_HALF_FLOAT`, `GL_INT` / `GL_UNSIGNED_INT` through
  `glVertexAttribPointer`) is converted to float32 rows like a client array
  (`src/gl/draw.rs`, `attrib_in_place_layout`; element size from
  `attrib_elem_bytes`). The 20 `vertex_arrays` packed cases.
  `tests/es3_vertex_arrays.rs`.
- **T3, the integer fill** (fixed on the host, not yet on a board): an
  integer array narrower than its `ivec4` / `uvec4` input fills w with the
  integer 1 (`attrib_row`, ES 3.0 2.8). The 21 `output_types` cases.
- **T4, layout locations at link**: (a) done on the host:
  `glBindAttribLocation` no longer moves an input that has
  `layout(location = N)` (`src/gl/program.rs`, `ExplicitAttribs::scan`,
  `build_attrib_indices`, `link_attrib_layout`; `tests/es3_attrib_location.rs`):
  the 28 `mixed` / `mixed_time` cases. (b) open: the 20
  `mixed_max_attributes.*` cases. An input without a qualifier must take the
  lowest run of free locations that no `layout(location)` uses and no earlier
  input took; `src/shaderc/online/reflect.rs` (`reflect()`, the
  `StorageQualifier::Attribute` branch) gives it the running counter instead,
  which collides with a later qualifier. With that change,
  `mixed_max_attributes_vec4` and
  `unqualified_unbound_inputs_avoid_every_qualifier` in
  `tests/es3_attrib_location.rs` pass: remove their `#[ignore]` in the same
  change. The Pi 4 fails the same 48.
- **T5, the indexed draws under capture** (closed): the 6
  `negative_api.vertex_array` indexed-draw cases need transform feedback,
  which is built; they pass.
- **T6, instancing with `gl_InstanceID` on the PowerVR and the GC7000UL**
  (open): the 6 `instanced.*.mixed.*` cases. Both take divisor 1 only, so the
  GL core draws instance by instance and `gl_InstanceID` starts at 0 in every
  draw; the PowerVR refuses such a draw, and the GC7000UL now refuses it too
  (host-checked): its online compiler records a program that reads
  `gl_InstanceID` in `vs_instance_id_reg1`, the field `gc_draw` tests
  (V0 in `docs/notes/instanceid-per-draw.md`); before that it drew the split
  wrong, with no refusal and no log line. The six cases still fail on the
  boards until `gl_InstanceID` is supplied per draw: PowerVR P1, Mesa's
  PDS `instance_id_modifier` set to the draw's instance, then P3, hardware
  divisors; GC7000UL V1, the stream divisor.
  The V3D takes any divisor and is expected to pass.
- **T7, the i.MX8MP and Pi 4 columns**: their first whole dEQP-GLES3 trees.

These verdicts are API-level for `attribute_location` and reproduce on the
host (`tools/cts/host-deqp-run.sh`, `tools/cts/README.md` section 2b);
`vertex_arrays`, `instanced` and `default_vertex_attrib` are picture verdicts
and need the board.
