<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D refusals by name, against the Pi 4 dEQP-GLES3 not-pass rows

Every string the V3D back end (`src/v3d`) can refuse a draw with, where it
is, the ES 3.0 feature behind it, who works on it, and the dEQP-GLES3 rows
of `docs/es3/pi4-gles3-fails.tsv` it most likely costs. Read at `a8e79b52`
(code) with the rows of the 925dbca9 / 0ff58329 runs (21 of 91 images, 1,383
rows: 740 Fail, 535 NotSupported, 99 NoResult, 4 QualityWarning, 2
ResourceError).

How to read the row counts:

- The tsv carries no refusal log. Only the first draw of each cause is
  logged per run (`count_refusal`, `backend.rs:1473`), and only the early-stop
  consoles of `tools/cts/log/` keep those lines. A count is a case-name
  prefix, an upper bound on what the refusal can cost, not a measurement.
  Where several refusals could sit behind one prefix, it is counted under
  each and says so.
- The runs are older than e883be8f (the integer clear program). The one
  refusal seen in a console, `a scissored or masked colour clear of an
  integer colour target` (FO6, M1), no longer exists in the code.
- A refusal returns `-EINVAL` (`refuse`) or `-ENOTSUP` (`refuse_unsupported`);
  the GL core turns the former into `GL_OUT_OF_MEMORY` for a pending clear
  (README of the 2026-10-31 run), which dEQP records as ResourceError and
  stops the image.
- `NotSupported` rows are mostly not a refusal at all: dEQP reports it when
  the framebuffer is incomplete or the format is not colour-renderable, which
  is decided in the GL core before the back end is called (section C).
- Owners: items of plan #330 (`mrt-clear` = v3d-mrt-clear, `srgb` =
  v3d-srgb-draw, `rgb10a2` = v3d-rgb10a2-side, `sizes` = v3d-mrt-sizes-note,
  `msaa` = v3d-msaa-es3-note), plan 308 (texture look-ups, `v3d-qpu-array-3d`,
  `v3d-vs-ret32`). "none" = no item and no issue known to this census.

## A. The ten `refuse(g, "...")` literals of `backend.rs`

`grep -c 'refuse(g, "' src/v3d/*.rs` is 10; one row each.

| # | Line | String | ES 3.0 feature | Owner | Rows it likely costs |
| --- | --- | --- | --- | --- | --- |
| A1 | 2607 | a 64- or 128-bpp colour target with more tiles than the job pools hold | RG32/RGBA16/RGBA32 integer and float targets at sizes over `MAX_JOB_TILES` | none | 0 known: the fbo.* cases render 64..256 px. Only a large RGBA32F/UI/I target hits it |
| A2 | 6007 | a draw with no colour target and no depth/stencil buffer | the GL core's framebuffer status already prevents it (an FBO with nothing attached is `GL_FRAMEBUFFER_UNDEFINED`/incomplete) | none, a guard | 0 |
| A3 | 6010 | 4x multisampling with no colour target | depth-only multisampled FBO (`fbo.msaa.*.depth_*`, `multisample.fbo_4_samples.depth`) | `msaa` | 0..10: `fbo.msaa.4_samples.depth*` (5 of the 22 `4_samples` rows), `multisample.fbo_4_samples.depth` and `.stencil` (2), only if the core ever makes the surface |
| A4 | 6016 | the target is outside the V3D's 32-bit address space | any target allocated above 4 GiB | none, a guard | 0 |
| A5 | 6020 | a target larger than the tile memory is sized for (2048x2048), or empty | `GL_MAX_RENDERBUFFER_SIZE` / `GL_MAX_TEXTURE_SIZE` renders; a zero-size target | none | 0 in the 21 images (`implementation_limits.*` rows are limits queries, not draws) |
| A6 | 6046 | 4x multisampling of several colour attachments | multisampled MRT (`glRenderbufferStorageMultisample` on several attachments) | `msaa` | `multisample.fbo_4_samples.*` (16), `fbo.msaa.4_samples.*` (22): only with MRT cases, 0 of them name MRT; so ~0 directly |
| A7 | 6051 | 4x multisampling of an ES 3.0 colour format | 4x RBO of R8, RG8, R16F, RGBA16F, RGB10_A2, sRGB, integers (`typed()` is every format but RGBA8, XRGB8 and RGB565, `state.rs:508`) | `msaa` | up to 22 (`fbo.msaa.4_samples.*`), 12 `rasterization.fbo.rbo_multisample_4`, 8 `fbo.blit.*msaa*`: the core's FBO is single-sampled today, so these rows are NotSupported/Fail before the back end, see C3 |
| A8 | 6057 | a scissored or masked colour clear of several colour attachments | `glClearBufferiv/fv` with scissor test or colour mask on a multi-RT FBO | `mrt-clear` | `fragment_ops.scissor.*` (21 rows: FO6 stops at `clear_int_buffer`, 20 NoResult behind it, one of them the stop itself), plus the several-attachment part of `fbo.color.repeated_clear.*` (55) and `fbo.color.clear.*` (24): the single-attachment integer clear is done (e883be8f), so only the several-attachment rows remain |
| A9 | 6075 | a draw with no linked program | `glUseProgram(0)` with draws | none, a guard | 0 |
| A10 | 6085 | a colour output with no attachment slot | a fragment shader writing `location = k` with fewer than k+1 attachments; also `glDrawBuffers` lists shorter than the program's outputs | none | `fragment_out.random.*` (50, of which 31 Fail, 19 NotSupported), `draw_buffers_indexed.random.*` (40 NotSupported, probably before the draw, C4). Upper bound 50 |

## B. The other refusal strings of `src/v3d`

All reach `refuse` through `state::refusal`, `state::unsupported`,
`state::job_state`, `extra_targets`, `texture::refusal_for`, `record` and its
callees (`Attempt::Refused`, `NoMemory`).

### B1. `state.rs`

| # | Line | String | Feature | Owner | Rows |
| --- | --- | --- | --- | --- | --- |
| B1.1 | 35 | a colour target of a format the backend does not know | the colour formats with no `rt_format` row: `GLES_SURF` has no id for RG16F, RG32F, RGBA32F, R11F_G11F_B10F (not even named; the V3D has rows for them, `backend-caps.md:68-69`) | none | the 239 NotSupported rows that name `r11f_g11f_b10f`, `rg32f`, `rgba32f`, or `r32f` (R32F has a row but the core does not offer it): `fragment_out.{basic,array}.float` 96, `fbo.blit.conversion.*` ~100, `fbo.color.*`, `fbo.render.*`, `fbo.invalidate.format.*`, `fbo.msaa.*`, `pbo.renderbuffer.*`. Not this string today; the core refuses first (C1) |
| B1.2 | 41 | a sample count other than 1 and 4 | `glRenderbufferStorageMultisample` with 2 or 8 samples (V3D has 4 only) | `msaa` (caps) | `fbo.msaa.2_samples.*` (22), `fbo.msaa.8_samples.*` (22), `multisample.fbo_8_samples.*` (16), `fbo_max_samples` (4): the core must round to 4 or report the cap; GL-core work |
| B1.3 | 49 | a second index list in one draw is not implemented | one draw with `index_count2` (the GL core's split draws) | none | 0 known |
| B1.4 | 69 | alpha-to-coverage is not implemented | `GL_SAMPLE_ALPHA_TO_COVERAGE` on a 4x target (`-ENOTSUP`) | none ("MSAA step 4, parked") | 7: `multisample.{default_framebuffer,fbo_4_samples,fbo_8_samples}.{constancy,proportionality}_alpha_to_coverage` (all NotSupported today, ahead of the draw) |
| B1.5 | 76 | draw buffer 0 is GL_NONE while a later draw buffer is written | `glDrawBuffers({GL_NONE, COLOR_ATTACHMENT1, ...})` | none | `draw_buffers_indexed.*` and `fragment_out.random.*` cases that use NONE: part of the 50 + 40 of A10; ~0..20 |
| B1.6 | 581 | an unknown cull mode | none reachable | none | 0 |
| B1.7 | 598 | an unknown compare function | none reachable | none | 0 |
| B1.8 | 650 | an unknown blend equation | `glBlendEquation*` the V3D has no row for (GL_MIN/MAX are rows; unknown enums only) | none | 0. The `draw_buffers_indexed.*blend_eq*` rows (advanced equations) are NotSupported by the missing KHR/EXT extension, not this string |

### B2. `backend.rs`, per-target (`extra_targets`)

| # | Line | String | Feature | Owner | Rows |
| --- | --- | --- | --- | --- | --- |
| B2.1 | 5969 | colour attachments of different sizes | MRT with different sizes (ES 3.0 4.4.4: the draw area is the intersection) | `sizes` | `fbo.render.*` / `fbo.completeness.attachment_combinations.*` with two attachments of different sizes: the 94 `fbo.render.resize.*` rows resize one attachment; MRT is a few. `fragment_out.*` attach equal sizes. Likely < 10 |
| B2.2 | 5974 | a colour attachment past the first of a format the backend does not render | an attachment past 0 of a format with no `rt_format` row; stale comment at :5971 says the sRGB surface has none, `rt_format` now has SRGB8_A8 and RGB10_A2 (state.rs:463,468) | `srgb`, `rgb10a2` (the comment) | 0 today; same reach as B1.1 |
| B2.3 | 5976 | a colour attachment outside the V3D's 32-bit address space | guard | none | 0 |

### B3. `backend.rs`, per-draw (`record` and what it calls)

| # | Line | String | Feature | Owner | Rows |
| --- | --- | --- | --- | --- | --- |
| B3.1 | 3665 | the draw reads depth an earlier job wrote, and the target has no depth/stencil buffer to keep it in | depth test across jobs on a target without depth store | none | 0 known |
| B3.2 | 3670 | the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in | same, stencil | none | 0 known; `fragment_ops.stencil` 17 NoResult are behind the FO6 stop (A8) |
| B3.3 | 3727 | an instance drawn on its own whose program reads gl_InstanceID | `gl_InstanceID` with an instance range of one | none | instancing images were not in the 21 |
| B3.4 | 3734 | an attribute location past 15 | `GL_MAX_VERTEX_ATTRIBS` 16 | none | 0: `attribute_location.mixed_max_attributes.*` (20 Fail) use 0..15 and are more likely the integer attribute path (B3.5) |
| B3.5 | 3740 | an attribute format other than float32 reached the backend | integer attributes, `glVertexAttribIPointer` (ES 3.0) | none | `attribute_location.mixed_max_attributes.{int,uint,ivec*,uvec*}` 20 Fail (the array names `.int`, `.ivec2`): it is the one place the code names an ES 3.0 integer attribute; whether it is this string or the compiler needs a V3D-side look at the console |
| B3.6 | 3746, 3764 | a vertex array outside the V3D's 32-bit address space / an attribute the program reads has no array | guards | none | 0 |
| B3.7 | 3775, 3780, 3786, 3787 | an index size other than 1, 2 or 4 bytes; the staged indices are shorter than index_count; an index buffer outside the 32-bit space; an index list too long to address | `glDrawElements` types (UNSIGNED_INT is ES 3.0 core) | none | 0 |
| B3.8 | 3800 | an unknown primitive | none reachable | none | 0 |
| B3.9 | 3836 | the job builder refused the draw (a uniform, program or primitive it does not write) | catch-all of `v3d_job_*` status | none | seen in the FO2-SQ consoles (two logs); unnamed cause. See note below |
| B3.10 | 4117 -> texture.rs | every string of B4, through `tex_state_for` | | | |
| B3.11 | 4155, 5396 | the texture state does not fit its record (v3d_tex_pack_state refused a field) | a field the record cannot hold | none | 0 known |
| B3.12 | 4251, 4257 | the depth buffer a depth texture was to be sampled from holds no stored depth of the texture's size / the TMU layout refused the depth buffer's size | sampling a rendered depth texture (`v3d-shadow.md`) | none | shadow rows (`shadow.*`) are outside the 21 images |
| B3.13 | 4304, 4307, 4309, 4311 | the job that rendered into a sampled texture did not complete / the TMU layout refused the texture's size (twice) / the texture's storage is not in the V3D OSAL's memory | render-to-texture then sample | none | `fbo.render.shared_colorbuffer*` (118) sample the colour buffer of an FBO they rendered into; possible but unconfirmed |
| B3.14 | 4343-4365 | six texture-copy strings: an unstamped (EGLImage) texture, over the tiled-copy budget, no GPU memory, no free slot, tiling refused the image, working set full, all "larger than the texture arena (8 MiB tiled)", and one for the address space | large textures that need the arena | none | 0 (the cases are small) |
| B3.15 | 5229 | the sampler state does not fit its record (v3d_tex_pack_sampler refused a field) | sampler params the packer refuses | none | 0 known |
| B3.16 | 5342, 5381 | an incomplete texture read by a look-up compiled for the 32-bit TMU output | an incomplete 32-bit-class texture | `v3d-vs-ret32` (plan 308) | 0 in the 21 images |
| B3.17 | 5346 | a stage samples more units than the V3D's tables hold | `GL_MAX_TEXTURE_IMAGE_UNITS` 16 | none | 0 |
| B3.18 | 5386 | the incomplete texture's stand-in has no layout | internal | none | 0 |

The 3836 catch-all deserves a split in the job builder's status (an issue
below): it hides the cause for every draw `v3d_job_*` rejects.

### B4. `texture.rs`

| # | Line | String | Feature | Owner | Rows |
| --- | --- | --- | --- | --- | --- |
| B4.1 | 129 | a cube map whose faces are not square | incomplete cube | none | 0 (a completeness case) |
| B4.2 | 132 | a cube map binding without its six faces | incomplete cube | none | 0 |
| B4.3 | 136 | a 2D array or 3D texture is not sampled yet | `sampler2DArray`, `sampler3D`, shadow arrays | plan 308 (`v3d-qpu-array-3d`) | `samplers.single_tex_3d` 7, `samplers.multi_tex_3d` 7 (14 Fail); the 2D-array rows (`texture.*`, `texel_fetch`) are not in the 21 images |
| B4.4 | 139 | a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here) | ETC2/EAC compressed formats; RGB10_A2 sampling | `rgb10a2` for the second half; ETC2: none | RGB10_A2: 42 rows name `rgb10_a2` (31 Fail, 9 NotSupported, 2 NoResult), mostly the render side; ETC2: `implementation_limits.num_compressed_texture_formats` 1 and the compressed-texture groups (not in the 21 images) |
| B4.5 | 145 | a look-up compiled for the 32-bit TMU output reads a texture of a 16-bit format | the ret32 pairing | `v3d-vs-ret32` | 0 known (the 2026-11 TF1/TF2/S32 consoles show the converse, B4.6) |
| B4.6 | 147 | a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output | 32-bit-class texture look-ups | `v3d-vs-ret32` | seen in three 2026-11 consoles; rows are outside this tsv |
| B4.7 | 152 | a texture size the TMU cannot address (0, or 16384 or more) | `GL_MAX_TEXTURE_SIZE` | none | 0 |
| B4.8 | 160 | a texture whose level 0 pitch is not the GL core's pair-padded width | a GL-core layout mismatch | none | 0 |

### B5. Program, clear and internal strings (not ES 3.0 features)

| Where | What | Owner | Rows |
| --- | --- | --- | --- |
| `backend.rs:3507-3525` (`code_refusal`) | 12 strings: the fragment, vertex and coordinate shader code too short, with an unknown QPU word, without the program-end thread switch, or with a uniform stream shorter than the code reads | none, defensive | 0 |
| `backend.rs:3538, 3570, 3575` | the program's code has no CPU copy; the fragment shader is not a program linked for the V3D; a program's code lies outside the uploaded code | none, defensive | 0 |
| `backend.rs:3841-3858` (`clear_program`) | the (integer) clear program did not build | none | 0 |
| `backend.rs:3910-3967` (`build_clear_program`) | no shader back end; cannot compile and link; the vertex/fragment shader did not compile; the program did not link; `u_color` is not the fragment block's dwords 0..3; the program does not read exactly one attribute; no GPU memory for its code | none | 0 |

## C. What the rows are, if not a refusal

The NotSupported and Fail mass of the tsv is mostly decided before the V3D
back end sees a draw. Naming it keeps a later plan from chasing the V3D for
it.

1. **Float32 and R11F colour formats are not colour-renderable names in the GL
   core** (`src/gl/framebuffer.rs` `is_color_format`, `backend-caps.md:241`).
   239 NotSupported rows name `r11f_g11f_b10f`, `rg32f`, `rgba32f` or `r32f`
   (89, 78, 78, 78 by name). They include all of `fragment_out.{basic,array}.float`
   for those four formats (96) and most of `fbo.blit.conversion.*` NotSupported
   (112). The V3D side needs `GLES_SURF` ids and `rt_format` rows for
   RG32F, RGBA32F, R11F_G11F_B10F (and RG16F); that is a GL-core plus V3D
   change (issue draft 1).
2. **RG16F fails (12 `fragment_out.*.rg16f`, 9 NotSupported elsewhere)**:
   the format has no surface id either (`GLES_SURF` stops at R16F, RGBA16F).
3. **FBO multisampling is absent in the core** (`fbo.msaa.*` 66,
   `multisample.fbo_4_samples.*` 16, `fbo_8_samples` 16, `fbo_max_samples` 4,
   `rasterization.fbo.rbo_multisample_*` 14, `fbo.blit` msaa 8): A3, A6, A7, B1.2
   and B1.4 are the V3D half, `msaa`'s subject.
4. **`draw_buffers_indexed.*` (76 NotSupported)** and
   `fragment_out.random.*` NotSupported (19) are the indexed
   blend/colour-mask entry points of the missing `EXT_draw_buffers_indexed`
   and the same float formats: GL core, not V3D.
5. `fbo.render.*` (305), `fbo.color.*` (206), `fbo.completeness.*` (45),
   `fbo.invalidate.*` (38), `pbo.renderbuffer.*` (45): 275 + 173 Fail rows are
   most likely the integer targets before e883be8f (the clear refused, or the
   integer format) plus sRGB/RGB10_A2: they belong to `v3d-gles3-triage` and
   the 2026-11 rerun (#320), not to a census of strings.

## D. Refusals with no item, ranked by the rows they likely cost

1. **No `GLES_SURF`/`rt_format` for RG32F, RGBA32F, R11F_G11F_B10F, RG16F**
   (B1.1, C1): up to 239 rows. The biggest single bucket, and not a V3D
   refusal; the V3D half is three to four `rt_format` rows.
2. **Fragment outputs without an attachment slot / draw buffer 0 GL_NONE**
   (A10, B1.5): up to 50 `fragment_out.random.*` rows, plus the NONE share of
   `draw_buffers_indexed.random` (40, mostly NotSupported).
3. **Integer attributes** (B3.5): up to 20 `attribute_location.mixed_max_attributes.*`.
   Unconfirmed that it is this string.
4. **Alpha-to-coverage** (B1.4): 7 rows, behind the 4x FBO of `msaa`.
5. **The job builder's catch-all** (B3.9): unknown, it hides the cause.
6. **ETC2/EAC sampling** (B4.4): rows are outside the 21 images.
7. **Depth-only / stencil-only 4x, the 64/128-bpp tile cap, the 2048 cap**
   (A1, A3, A5): 0 known.

The report of this item holds the issue drafts for 1-3 and 5-6.
