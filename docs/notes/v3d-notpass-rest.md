<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the Pi 4 GLES3 not-pass rows outside fbo.*, fragment_out.* and draw_buffers_indexed.*

Research, no code. `docs/es3/pi4-gles3-fails.tsv` holds 1,380 data rows (1,383
lines with its three `#` header lines). `v3d-fbo-render.md` / `v3d-fbo-blit.md`
cover `fbo.*` (867 rows), `v3d-fragment-out.md` covers `fragment_out.*` (158),
`draw_buffers_indexed.*` (76) has its own item. This note takes the other 279.
Only 21 of the tree's 91 images have run (`texture`, `shaders`,
`transform_feedback`, `vertex_arrays`, ... have no row: no row is not a pass),
so the table below ranks what the tsv shows, not the whole suite.

## 1. Count check

| Part | Rows |
| --- | ---: |
| `fbo.*` (other notes) | 867 |
| `fragment_out.*` (other note) | 158 |
| `draw_buffers_indexed.*` (other item) | 76 |
| this note | 279 |
| total | 1,380 |

This note's groups: `multisample` 52, `pbo.renderbuffer` 45, `state_query` 35,
`fragment_ops` 38 (`scissor` 21, `stencil` 17), `attribute_location` 20,
`polygon_offset` 20, `rasterization` 18 (`fbo` 16, `interpolation` 2),
`samplers` 14, `shader_api` 14, `implementation_limits` 11, `negative_api` 11,
`lifetime` 1 = 279. (Counts by `awk` over the tsv, status split below.)

## 2. Groups, cause, evidence

Statuses are dEQP's. NoResult = the case never began.

### G1. The two early stops: 99 NoResult + 3 rows at the stop (FO6, M1)

`fragment_ops.scissor` 20 NoResult + 1 ResourceError (`clear_int_buffer`),
`fragment_ops.stencil` 17 NoResult, `pbo.renderbuffer` 42 NoResult + 1
ResourceError (`rg16i_clears`), `polygon_offset` 20 NoResult. Total 101 rows
(plus 2 `pbo.renderbuffer` NotSupported, G5).

- **Cause**: a draw the back end refused with `-EINVAL` is reported by
  `submit_draw` (`src/gl/draw.rs:565-600`) as `GL_OUT_OF_MEMORY`; dEQP treats
  that as a fatal resource error and ends the image
  (`docs/es3/board-recipe-pi4.md:555-560`: "the integer clear refusal, RC1").
  `refuse()` (`src/v3d/backend.rs:1461`) still answers `-EINVAL`; only
  `state::unsupported` goes through `refuse_unsupported` (`-ENOTSUP`).
- **Evidence**: both stops were recorded at `0ff58329`, before e883be8f
  (`CLEAR_FS_INT`, a scissored or masked single-attachment integer clear). Both
  cases are single-attachment clears, so they should already pass on main; the
  rows are stale until #320 reruns FO6 and M1. What can still stop an image is
  any other refusal that is legal GL: the one named at
  `src/v3d/backend.rs:6063` (several attachments), the sRGB draw and
  RGB10A2 refusals, and every `-EINVAL` `refuse(` (20 sites, `backend.rs`
  2613-6091). The fix that makes the stop impossible is the item
  `v3d-refuse-enotsup` (inside `src/v3d`).
- **Recovery**: 99 mustpass cases begin. Not all pass: `pbo.renderbuffer`
  has `srgb8_alpha8_*` 2, `rgb10_a2ui_*` 2, `rgb565_clears` 1 and the wide
  32-bit formats that may hit other refusals (RC2/RC1 formats in
  `v3d-format-matrix.md`); `polygon_offset` and `stencil` are unmeasured on
  the V3D (no prior row), so their pass rate is unknown.
- **Host test**: yes, and the plan's tests items cover it
  (`t-fragops-scissor`, `t-pbo-rb-clears`).

### G2. Multisample: 52 + `rasterization.fbo` rbo_multisample 14 = 66 rows (the 14 = 12 NotSupported + 2 Fail)

`multisample.default_framebuffer.*` 16 NotSupported, `fbo_4_samples.*` 16,
`fbo_8_samples.*` 16 (all NotSupported), `fbo_max_samples.*` 4 Fail;
`rasterization.fbo.rbo_multisample_4.*` 12 NotSupported,
`rbo_multisample_max.*lines_wide` 2 Fail.

- **Cause**: GL core, not `src/v3d`. `GL_MAX_SAMPLES` is 1
  (`src/gl/framebuffer.rs:3381-3390`, `glRenderbufferStorageMultisample`), so
  dEQP reports NotSupported for 4 and 8 samples and its `max_samples` cases
  run with one sample and fail the coverage tests. The default framebuffer has
  no sample count (`src/gl/egl.rs:948-960`, a config without
  `EGL_SAMPLES`). The V3D side also refuses 4x of several attachments and of
  an ES 3.0 format (`backend.rs:6052`, `:6057`) and alpha-to-coverage
  (`state.rs` `unsupported`). Design: `v3d-msaa-es3.md`.
- **Ranking note**: `fbo_8_samples` 16 can never pass on the V3D (4x is its
  only mode; dEQP answers NotSupported when 8 is not offered). The recoverable
  ceiling is 50: 16 + 16 + 4 + 12 + 2 (the `*_max` lines_wide rows also
  want wide lines).
- **Host test**: partly. A host GL test can check `GL_MAX_SAMPLES` and
  `GL_SAMPLES`; coverage proportionality needs the board.

### G3. Limits and state queries: `implementation_limits` 11 + `state_query` 35 = 46 rows

`state_query.indexed.*` 9 (NotSupported: `OES_draw_buffers_indexed`, same
cause as `draw_buffers_indexed`), `state_query.integers.*` 17 and
`integers64.*` 6 (Fail), `rbo.renderbuffer_component_size_color` 1,
`shader.program_active_uniform_types` 1, `shader.program_binary` 1.

- **Cause**: GL core. The limit rows are ES 3.0 minimums the core does not
  reach: `GL_MAX_VARYING_VECTORS` was 8 on the V3D (ES 3.0 asks 15, Mesa's
  v3d has 16; 15 since plan #359, see below), `DRAW_TARGETS` is 3
  (`src/backend.rs:765-775`; ES 3.0 asks 4, the V3D TLB and compiler take 4,
  its comment says only "no board has drawn four"), the uniform component and
  block-size limits (16384 and 50688 on the V3D since plan #359,
  `docs/notes/ubo-limits.md` §9), and the compressed-format list. The `*_getfloat` /
  `*_getinteger64` rows fail on limits whose integer query is right too (e.g.
  `max_samples_getfloat`), so the float and 64-bit paths of
  `glGetFloatv`/`glGetInteger64v` (`src/gl/state.rs:1379-1500`) were
  suspected of a separate defect. Read: there is none. Both widen
  `get_integerv`'s answer (the unsigned stencil masks aside), and
  `tests/es3_query_forms.rs` pins that all three forms agree and write one
  value for every limit; those rows fail with the value, and move with it.
- **V3D varyings, what carries them**: done on the host by plan #359
  (`docs/notes/v3d-varyings.md`, "What was built"): `V3dFsProg` and
  `ShcV3dFs` hold Mesa's three 24-bit flag words per kind, the job builder
  emits one packet per non-zero word, the VPM output holds 64 varying
  components, and the V3D reports 15 / 60 / 60 / 64. The board run of
  es100/varyings15 and of words 1 and 2 is #320's.
- **Cross-effect**: raising `DRAW_TARGETS` to 4 for the V3D also changes
  `fragment_out.*` and `draw_buffers_indexed.*` and the V3D MRT clear; coordinate
  with the notes for those.
- **Recovery**: 45 of the 46 rows: 34 (`implementation_limits` 11 + `integers`
  17 + `integers64` 6) once all values and query forms are right, the 9
  `indexed` rows with the extension (item with `draw_buffers_indexed`), and
  `rbo` and `program_active_uniform_types` (GL core, not read to the line).
  The 46th, `program_binary`, is counted in G7.
- **Host test**: yes, `tests/es3_caps.rs` is the pinned place (it asserts the
  values, and was "reverted for review, to be redone" in RC10).

### G4. `attribute_location.mixed_max_attributes.*`: 20 Fail

- **Cause**: compiler. An unqualified input takes the running counter in
  `src/shaderc/online/reflect.rs` (`reflect()`, `StorageQualifier::Attribute`),
  which collides with a later `layout(location)`; `docs/es3-cts-failures.md`
  T4(b). Same 20 rows on every board.
- **Host test**: exists, ignored: `mixed_max_attributes_vec4` and
  `unqualified_unbound_inputs_avoid_every_qualifier` in
  `tests/es3_attrib_location.rs`.
- **Recovery**: 20, host-checkable, no board needed to confirm.

### G5. `pbo.renderbuffer` NotSupported: 2 rows

`r11f_g11f_b10f_clears` / `_triangles`: R11F_G11F_B10F is not colour-renderable
in ES 3.0 (it is with `EXT_color_buffer_float`, which the V3D does not
offer). Expected NotSupported, not recoverable by a fix.

### G6. `samplers.*_3d`: 14 Fail

`samplers.{single,multi}_tex_3d.diff_*` (7 each).

- **Cause**: compiler. The shader that dEQP builds does a 3D look-up and the
  V3D code generator refuses it: `yakogl: online shaderc: ... NotYet("a texture
  look-up other than 2D or cube")` (`tools/cts/log/2026-10-31-pi4-gles3-a/
  R1.console.txt:1662-1666`; `src/shaderc/online/v3d/prepare.rs:1335`);
  `textureSize` of 3D and array samplers is refused too (`prepare.rs:1494`).
  The same gap sits behind the unrun `texture.filtering.3d`,
  `texture.specification` 3D and 2D-array rows and `texturesize`, so this
  group's 14 are the visible part of a much larger one.
- **Host test**: yes, a compile-only test on the shaderc V3D back end (a
  `texture(sampler3D, vec3)` program must reach QPU code); the texture
  descriptor side is `v3d-tex-layout-es3.md`.

### G7. `shader_api.program_binary.*` 14 + `state_query.shader.program_binary` 1

`glGetProgramBinary` answers `GL_INVALID_OPERATION` and
`GL_NUM_PROGRAM_BINARY_FORMATS` is 0 (`src/gl/es3.rs:1528-1550`), a legal ES
3.0 answer; dEQP reports NotSupported. Recoverable only by implementing
program binaries (store the linked program, load it): design work, no cause
specific to the V3D. Lowest priority; the 15th row is a Fail that probably
expects non-empty formats.

### G8. `negative_api`: 7 Fail + 4 NotSupported = 11

NotSupported 4: `blend_equationi`, `blend_equation_separatei`, `blend_funci`,
`blend_func_separatei` (the indexed-blend entry points, as `draw_buffers_indexed`).
Fail 7 (all "Got invalid error"/"Failed", the wrong GL error code): `buffer.blit_framebuffer`,
`blit_framebuffer_multisample`, `read_pixels_fbo_format_mismatch`,
`shader.uniformui_incompatible_type`, `uniformuiv_incompatible_type`,
`texture.texsubimage2d`, `texsubimage3d`. All in GL core validation
(`src/gl`), independent of the GPU. Host test: yes (`tests/negative_api/`
exists; one case per row needs only the GL API).

### G9. `rasterization` QualityWarning 4 + `lifetime` 1

- `QualityWarning` 4 (`interpolation.basic.lines`, `projected.lines`,
  `fbo.rbo_singlesample.interpolation.lines`, `fbo.texture_2d.interpolation.lines`):
  dEQP's "line rasterization differs from the reference" warning; the result is
  a pass for the mustpass check. No fix needed beyond confirming the status
  in the tally.
- `lifetime.attach.deleted_output.buffer_transform_feedback`: "Writing to
  container with deleted attachment modified a new object." Transform feedback
  capture to a deleted buffer name (`src/gl/tf*.rs`, GL core). Host test: yes
  (`tests/es3_tf_api.rs`).

## 3. Ranking by mustpass rows a fix would recover

| # | Group | Rows | Where | Fix |
| ---: | --- | ---: | --- | --- |
| 1 | G1 early stops (rerun after e883be8f; `-ENOTSUP` for the rest) | 101 | `src/v3d`, rerun #320 | already in; item `v3d-refuse-enotsup` |
| 2 | G3 limits and queries (its `program_binary` row is counted in G7) | 45 | `src/gl`, `src/backend.rs` | caps and query forms |
| 3 | G2 multisample (4x) | 50 of 66 | `src/gl` + `src/v3d` | `v3d-msaa-es3.md` |
| 4 | G4 mixed_max_attributes | 20 | `src/shaderc` | T4(b), tests already written |
| 5 | G6 samplers 3D | 14 (+ unrun) | `src/shaderc/online/v3d` | 3D look-up and `textureSize` |
| 6 | G8 negative_api | 7 (+4) | `src/gl` | error codes |
| 7 | G7 program binary (14 + the G3 row) | 15 | `src/gl` | design |
| 8 | G9 lifetime | 1 | `src/gl` | TF deleted buffer |
| - | G9 QualityWarning 4, G5 2, fbo_8_samples 16 | 22 | none | not recoverable |

Sum of "recoverable": 101 + 45 + 50 + 20 + 14 + 11 + 15 + 1 = 257; with the 22
not recoverable, 279.

## 4. What a host test can reproduce

| Group | Host test |
| --- | --- |
| G1 | yes, V3D host back end through the GL API (`tests/es3_srgb.rs` style); the refusal is observable as `GL_OUT_OF_MEMORY` today |
| G3 | yes, `tests/es3_caps.rs` |
| G4 | yes, ignored tests exist |
| G6 | yes, shaderc compile-only |
| G8, G9 | yes, GL API |
| G2 | partly (queries); coverage needs a board |
| G7 | design first |
