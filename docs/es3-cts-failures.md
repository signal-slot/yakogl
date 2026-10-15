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

**Pi 4**: 7 of the 91 images of `tools/cts/pi4-gles3-tree.txt` on `cd738b1a`
(3,807 cases: 2,798 pass / 744 fail / 265 NotSupported). Failing cases:
`docs/es3/pi4-gles3-fails.tsv`. By subgroup: `fbo.blit` 308, `fbo.color` 215,
`fbo.completeness` 134, `attribute_location` 48, `draw.random` 14, `fbo.depth`
10, `implementation_limits` 11. These are the integer, MRT, 2D-array and
layout causes below.

**i.MX8MP**: never run on the board (`docs/es3/board-recipe-imx8mp.md`).

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
| Done | GL core accepts the formats; the PowerVR writes all nineteen integer formats as a draw's one colour attachment; the V3D writes them, the float ones and the ES 2.0 formats among any attachments (host only); the GC7000UL refuses by name (`docs/es3/int-rt-gc.md`). Host tests: `tests/es3_int_rt.rs`, `tests/es3_int_rt_v3d.rs` |
| Left on the PowerVR | an integer attachment beside other attachments (this is why `builtin_functions.common.modf` fails: dEQP's executor binds two integer attachments), colour masks that keep some of an integer target's channels, precompiled rows for the integer variants. Most of `fragment_out`'s 456 `int` / `uint` failures and part of `fbo.color` are here |
| Left on the V3D | a board run (gl_test 200 on the Pi 4), an integer clear program for a scissored or masked clear |
| Float targets | `shaders.derivate` 15 fail on an incomplete RGBA32F / RGBA16F framebuffer: float targets need `GL_EXT_color_buffer_float`, which is not claimed. A decision, no item yet |

### RC2. ES 3.0 sized texture formats; integer and packed-float textures

| | |
|---|---|
| Layer | GL core (upload, storage, conversion), then each back end (sampling the format; isampler / usampler) |
| Done | the 35 colour formats of ES 3.0 table 3.2 are converted at upload (`docs/es3/sized-formats.md`); the PowerVR samples every class but `RGB10A2` (`docs/es3/pvr-formats.md`); host test `tests/es3_sized_formats.rs` |
| Left | PowerVR: integer cube, 3D and 2D-array samplers, and `RGB10A2` in its own class (it needs a 10/10/10/2 unorm render-target class first, then the framebuffer guard and `glGenerateMipmap` in the GL core). GC7000UL and V3D: they refuse the integer classes by name (`formats-gc`, `formats-v3d`), which is most of the 1,656 texture NotSupported on those GPUs once they run |

### RC3. Transform feedback

Built: the GL core runs the vertex shader on the CPU for every captured
vertex, on the three GPUs (`docs/es3/transform-feedback.md`). The board agrees:
`transform_feedback` 882 / 882 on the AM62P, gl_test 203 on the AM62P and the
Pi 4. Left: `lifetime.attach.deleted_output.buffer_transform_feedback`; a
vertex shader that samples with a shadow look-up, `textureGrad`, a compressed
texture or a base level other than 0 is refused by name.

### RC4. Compile failure with an empty info log

The 48 `transform_feedback` cases that failed with `Compile failed: ''` pass
in the re-run on `cd738b1a`. Closed.

### RC5. Shadow samplers

The online compiler takes the shadow, projective and 3D bias forms; the 2D
and cube shadow cases pass or warn (180 QualityWarning). Left: the 108
`texture.shadow.2d_array` failures, which are RC6.

### RC6. 2D-array textures sampled as one tall image (PowerVR)

The TPU filters and wraps across layers. `texture.filtering.2d_array` 115,
`texture.shadow.2d_array` 108 and the 2D-array cases of `texture.specification`
and `fbo.color.tex2darray` fail. Designed, not built:
`docs/es3/pvr-2darray.md` (item `pvr-2darray`).

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
host). Still `GL_INVALID_OPERATION`, each logged once: depth into a
renderbuffer or the window, all stencil, depth from the window or from a
cube or array depth texture. That is all 16 `fbo.blit.depth_stencil`
failures, because dEQP blits between renderbuffers; it needs two back-end
entries, a CPU write of a depth or stencil rectangle (`ds_write`) and the
stencil half of `depth_resolve` (`stencil_resolve`). Multisample
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
full chain, `addrmode_w`). Also: `tools/cts/gen-am62p-gles3-tree.py` should
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

Done on the host: depth larger than colour, colour on attachments 1.. alone,
separate depth and stencil with no colour (`tests/es3_fbo_fragops.rs`).
Left, each in the GL core unless named: separate depth and stencil images
with a colour attachment 0 (12 `attachment_combinations` cases),
`GL_R32F` as a render target (a decision: it wants
`GL_EXT_color_buffer_float`), `renderable.texture.color0.rgba32f` (the texture
must remember its sized format), the 4 unsized depth textures (refuse them
under `!REPORT_ES2`), `completeness.layer.3d_64_15` on the PowerVR (RC15), the
PowerVR's integer texture attachments (RC1), and the 10 `fbo.depth` clamp
cases (`gl_FragDepth` in the PowerVR shader back end).

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
  draw; the PowerVR refuses such a draw, the GC7000UL draws it wrong. The
  design is chosen (`docs/notes/instanceid-per-draw.md`): PowerVR P1, Mesa's
  PDS `instance_id_modifier` set to the draw's instance, then P3, hardware
  divisors; GC7000UL V0, `vs_instance_id_reg1`, and V1, the stream divisor.
  The V3D takes any divisor and is expected to pass.
- **T7, the i.MX8MP and Pi 4 columns**: their first whole dEQP-GLES3 trees.

These verdicts are API-level for `attribute_location` and reproduce on the
host (`tools/cts/host-deqp-run.sh`, `tools/cts/README.md` section 2b);
`vertex_arrays`, `instanced` and `default_vertex_attrib` are picture verdicts
and need the board.
