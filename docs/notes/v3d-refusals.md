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
  stops the image, and the latter into `GL_INVALID_OPERATION`, which fails
  one case. Since v3d-refuse-enotsup every refusal of legal GL state here is
  `-ENOTSUP` (A1, A3-A7, A10, B1.3, B2.1, B2.3, B3.1-B3.3, B3.5, B3.6's and
  B3.7's 32-bit address guards, B3.9, B3.14's fixed limits, B3.16, B4.4-B4.6);
  a malformed draw or a guard keeps `-EINVAL` (A2, A9, B1.1, B1.2, B2.2, the
  rest), and no memory keeps `-EINVAL` (a kept copy) or `-ENOMEM` (a
  depth/stencil buffer, `ds_retention::refuse_no_memory`).
- `NotSupported` rows are mostly not a refusal at all: dEQP reports it when
  the framebuffer is incomplete or the format is not colour-renderable, which
  is decided in the GL core before the back end is called (section C).
- Owners: items of plan #330 (`mrt-clear` = v3d-mrt-clear, `srgb` =
  v3d-srgb-draw, `rgb10a2` = v3d-rgb10a2-side, `sizes` = v3d-mrt-sizes-note,
  `msaa` = v3d-msaa-es3-note), plan 308 (texture look-ups, `v3d-qpu-array-3d`,
  `v3d-vs-ret32`). "none" = no item and no issue known to this census.

## A. The ten draw refusals of `backend.rs`

Ten rows (A1-A10), one per refusal. Every `file:line` of this section is
read at `93e9b39f` (after #1261's fix and tile-guard): the Line column and the
`backend.rs`, `backend_tests.rs` and `state.rs` lines in the text. At the
earlier read commit `a8e79b52` the ten
were `grep -c 'refuse(g, "' src/v3d/*.rs` — 10 literals, one row each.
They are not that anymore: b7c1484e moved the refusals of legal GL state to
`refuse_unsupported(g, "...")` with the same words, 7fa02269 removed A8's
string, and #1261 (6d2e7d0d) split A5's one string into `refuse` and
`refuse_unsupported`. bf80c82a (tile-guard) adds no row: `submit_job` clamped
the kick's `tile_state_size` to the pool with `min()`, and bf80c82a made that
clamp a `debug_assert!` (`backend.rs:3345-3356`) naming A1's and A5's
refusals and the pool's sizing as the checks that keep a short tile state
unreachable — clamping instead would be a binner overrun with no message; a
job violating the bound panics in debug and host-test builds and is submitted
unclamped in release builds, and `a_job_at_the_limit_gets_its_whole_tile_state`
(`backend_tests.rs:4593`) pins the jobs at the limit (4096 square, 1x and 4x)
as carrying their whole tile state. The heading counts the ten rows of this
section, not the code's literals.

| # | Line | String | ES 3.0 feature | Owner | Rows it likely costs |
| --- | --- | --- | --- | --- | --- |
| A1 | 2693-2695 | a 64- or 128-bpp colour target with more tiles than the job pools hold: `tiles_x * tiles_y` from `v3d_frame_tiling_bpp` is over `MAX_JOB_TILES` = (4096 / 32)^2 = 16,384, what the pools are sized for at `MAX_TARGET` = 4096 (`refuse_unsupported`, `-ENOTSUP`; was 2048 square, 4,096 tiles, until #1261) | RG32/RGBA16/RGBA32 integer and float targets whose smaller tiles bin past 16,384 tiles: two 128-bpp RTs (32x16 tiles) over 4096x4096 is 32,768, a single 128-bpp RT (32x32 tiles) of 4096x4096 is 16,384 and is taken (`what_a_typed_job_cannot_take_is_refused_by_name`) | none | 0 known: the fbo.* cases render 64..256 px and the shader executor's 4096 x 2-6 targets bin 64 to 256 tiles. Only a multi-RT 4096-square 64/128-bpp target hits it |
| A2 | 6565 | a draw with no colour target and no depth/stencil buffer | the GL core's framebuffer status already prevents it (an FBO with nothing attached is `GL_FRAMEBUFFER_UNDEFINED`/incomplete) | none, a guard | 0 |
| A3 | 6568 | 4x multisampling with no colour target | depth-only multisampled FBO (`fbo.msaa.*.depth_*`, `multisample.fbo_4_samples.depth`) | `msaa` | 0..10: `fbo.msaa.4_samples.depth*` (5 of the 22 `4_samples` rows), `multisample.fbo_4_samples.depth` and `.stencil` (2), only if the core ever makes the surface |
| A4 | 6574 | the target is outside the V3D's 32-bit address space | any target allocated above 4 GiB | none, a guard | 0 |
| A5 | 6582, 6585 (#1261; 6824, 6827, 6835 at `89d3e1c3`, section E) | until #1261 one string, "a target larger than the tile memory is sized for (2048x2048), or empty", for an empty target (`-EINVAL`) and any side past 2048 (`-ENOTSUP`; `-EINVAL` before b7c1484e). Since #1261 (6d2e7d0d) two: "an empty target (0 pixels on a side)" (`refuse`, `-EINVAL`) and "a target larger than the tile memory is sized for (4096x4096)" (`refuse_unsupported`, `-ENOTSUP`; the GL core makes no storage past 4096, so a guard) | `GL_MAX_RENDERBUFFER_SIZE` / `GL_MAX_TEXTURE_SIZE` renders; a zero-size target. The tile memory is sized for 4096 a side, what the GL core reports (docs/notes/v3d-large-targets.md) | fix-v3d (plan #376), done | not 0 as this row said: the dEQP shader executor's 4096 x 2-6 targets fired the 2048 arm in 1,030 of the 1,147 `shaders.builtin_functions.precision.*` cases on the host and aborted Pi 4 images S03-S08 at their first `highp_fragment.scalar` case (1,142 mustpass cases without a result, b1b5b845 and 8990a5cc). Since #1261 those targets draw; the 4096 guard is reached by no GL call |
| A6 | 6612 | 4x multisampling of several colour attachments | multisampled MRT (`glRenderbufferStorageMultisample` on several attachments) | `msaa` | `multisample.fbo_4_samples.*` (16), `fbo.msaa.4_samples.*` (22): only with MRT cases, 0 of them name MRT; so ~0 directly |
| A7 | 6617 | 4x multisampling of an ES 3.0 colour format | 4x RBO of R8, RG8, R16F, RGBA16F, RGB10_A2, sRGB, integers (`typed()` is every format but RGBA8, XRGB8 and RGB565, `state.rs:513`) | `msaa` | up to 22 (`fbo.msaa.4_samples.*`), 12 `rasterization.fbo.rbo_multisample_4`, 8 `fbo.blit.*msaa*`: the core's FBO is single-sampled today, so these rows are NotSupported/Fail before the back end, see C3 |
| A8 | 6051 (`draw_clear_quad`; the string is gone) | a scissored or masked colour clear of several colour attachments | `glClearBufferiv/fv` with scissor test or colour mask on a multi-RT FBO | `mrt-clear`, **done**: the clear is one quad from `src/v3d/clear_mrt.rs` (`draw_clear_quad`, docs/notes/v3d-mrt-clear.md section 0); the string is no longer in the code | `fragment_ops.scissor.*` (21 rows: FO6 stops at `clear_int_buffer`, 20 NoResult behind it, one of them the stop itself), plus the several-attachment part of `fbo.color.repeated_clear.*` (55) and `fbo.color.clear.*` (24): the single-attachment integer clear is done (e883be8f), the several-attachment one with v3d-mrt-clear; the Pi 4 rerun of FO6 and M1 (#1261) says what they cost |
| A9 | 6635 | a draw with no linked program | `glUseProgram(0)` with draws | none, a guard | 0 |
| A10 | 6645 | a colour output with no attachment slot | a fragment shader writing `location = k` with fewer than k+1 attachments; also `glDrawBuffers` lists shorter than the program's outputs | none | `fragment_out.random.*` (50, of which 31 Fail, 19 NotSupported), `draw_buffers_indexed.random.*` (40 NotSupported, probably before the draw, C4). Upper bound 50 |

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
| B3.9 | 3836 (3988-3990 at `89d3e1c3`, section E) | the job builder refused the draw (a uniform, program or primitive it does not write) | catch-all of `v3d_job_*` status | none | seen in the FO2-SQ consoles (two logs); unnamed cause. See note below |
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
| B4.4 | 139 (390 and 439-440 at `89d3e1c3`, section E; the string is now "a texel format the V3D does not sample") | a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here) | ETC2/EAC compressed formats; RGB10_A2 sampling | `rgb10a2` for the second half; ETC2: none | RGB10_A2: 42 rows name `rgb10_a2` (31 Fail, 9 NotSupported, 2 NoResult), mostly the render side; ETC2: `implementation_limits.num_compressed_texture_formats` 1 and the compressed-texture groups (not in the 21 images) |
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
7. **Depth-only / stencil-only 4x, the 64/128-bpp tile cap** (A1, A3): 0
   known. The 2048 cap (A5) was not 0: it aborted Pi 4 images S03-S08 and is
   gone since #1261.

The report of this item holds the issue drafts for 1-3 and 5-6.

## E. The b1b5b845 Pi 4 aborts at 89d3e1c3

The whole-tree Pi 4 run at `b1b5b845` (#1261) has 1,979 cases without a
result, because dEQP aborted in nine images (S03a, S07, T14a, T17a, TF1a,
TF1b, TF2a, TF2b, TF2c). T05ca, T05cb and T05cc also stopped on their one
ETC2 cube case each. The consoles are in
`tools/cts/log/2026-11-pi4-gles3-b1b5b845-aborts/` (S07's is in
`-rows16-30/`). Each console has one `draw refused:` line before the
ResourceError. Code is read at `89d3e1c3`, the integration tip. The
origin/main snapshot `f77b0626` differs in `src/v3d` and `driver/src/v3d`
only by 1ee37fe7, which is V3D 7.1 code (`ffi71.rs`, `v3d_job71.h`), so
every line below holds there too.

At `b1b5b845` every refusal answered `-EINVAL`, because b7c1484e, 6d2e7d0d,
455c3bb5 and 6581f22b are not ancestors of `b1b5b845` (a 2026-11-05
commit). The GL core turns any return other than `-ENOTSUP` into
GL_OUT_OF_MEMORY (`src/gl/draw.rs:683`). dEQP reads that as ResourceError
and ends the image. `-ENOTSUP` becomes GL_INVALID_OPERATION, which fails one
case.

### E1. One row per aborted image

| Image | Case that aborted (`dEQP-GLES3.functional.`) | Console's refusal (`b1b5b845`) | Row | At `89d3e1c3` | Return at the tip | Commit | Host test |
| --- | --- | --- | --- | --- | --- | --- | --- |
| S03a | `shaders.builtin_functions.precision.abs.highp_fragment.vec2` | a target larger than the tile memory is sized for (2048x2048), or empty | A5 | `backend.rs:6826-6827`: refused only past `MAX_TARGET` = 4096 (`backend.rs:447`); the shader executor's 4096 x 2-6 target draws | **gone** (a target over 4096 would get `-ENOTSUP`, but no GL call makes one) | 6d2e7d0d (A5 split, 4096 targets); b7c1484e (`-ENOTSUP`) | `a_target_of_4096_a_side_is_one_job` (`backend_tests.rs:4667`), `a_backend_gap_is_enotsup_and_counted` (`:4580`) |
| S07 | `shaders.builtin_functions.precision.reflect.highp_fragment.scalar` (not in the mustpass; its one mustpass case `reflect.highp_fragment.vec3` never began) | same as S03a | A5 | same as S03a | **gone** | 6d2e7d0d | same as S03a |
| T05ca | `texture.format.compressed.etc2_eac_rgba8_cube_pot` | a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here) | B4.4 | `texture.rs:439-440` (`NOT_SAMPLED`, `:390`): `V3D_SAMPLED_TEXFMTS` (`src/backend.rs:290-309`) now holds `GLES_TEXFMT_ETC2_RGB`, `_RGBA`, `_SRGB_RGB` and `_SRGB_RGBA` | **gone** for the four classes. Any format still outside the set gets `-ENOTSUP` (`texture::GAPS`, `backend.rs:4499`) | 455c3bb5 (ETC2 sampled); b7c1484e | `the_gl_cores_etc2_classes_sample_as_their_rows` (`texture.rs:1581`), `tests/es3_etc2_v3d.rs` |
| T05cb | `texture.format.compressed.etc2_rgb8_cube_npot` | same as T05ca | B4.4 | same as T05ca | **gone** | 455c3bb5 | same as T05ca |
| T05cc | `texture.format.compressed.etc2_rgb8_cube_pot` | same as T05ca | B4.4 | same as T05ca | **gone** | 455c3bb5 | same as T05ca |
| T14a | `texture.units.8_units.only_cube.2` | no empty job holds the draw (its lists, or its client data with no memory of its own); before it, `osal(v3d): out of GPU memory` for 2,117,632, 3,149,824 and 4,206,592 bytes, with 9,502,720 of 16,777,216 bytes of tiled copies live | none in A/B (`draw_in_job`'s lost draw). Not B3.14: the textures are smaller than the 8 MiB arena, so no B3.14 string is reached | `backend.rs:6214-6218`, unchanged since `b1b5b845` (`:5531` there). The kept tiled copy finds no GPU memory (`tiled_copy`, `backend.rs:4880-4892`). The fallback into the job's texture arena (`backend.rs:4768`, `Attempt::Full`) does not fit an empty job either: the 8 cube units need more than the arena. `draw_in_job`'s second attempt breaks out at `:6188` | **`-E2BIG`, still aborts** (GL_OUT_OF_MEMORY, `src/gl/draw.rs:683`) | none (the retry with the open job run first, `:6190-6210`, is in both) | `every_refused_draw_is_counted_and_nothing_else` (`backend_tests.rs:4537`) pins `-E2BIG` for a draw no empty job holds |
| T17a | `texture.wrap.etc2_eac_rgba8.clamp_clamp_linear_pot` (the image also holds `wrap.etc2_rgb8`) | same as T05ca | B4.4 | same as T05ca | **gone** | 455c3bb5 | same as T05ca |
| TF1a | `transform_feedback.array.interleaved.lines.highp_mat3` | the job builder refused the draw (a uniform, program or primitive it does not write) | B3.9 | `backend.rs:3988-3990` (`Attempt::Unsupported`). The job builder's input cap at `b1b5b845`, `d->fs->num_inputs > 24` (`v3d_job.c:775` there), is `> 64` at the tip (`driver/src/v3d/v3d_job.c:876`), and the flag words of inputs 24..63 are written | **gone** on the host. A job-builder refusal that is left would be `-ENOTSUP` (one Fail, no abort) | 6581f22b (inputs 24..63), 218c90ec (flags packet count); b7c1484e (`-ENOTSUP`) | `tests/es3_tf_v3d.rs` (the real `v3d_job.c`): `cases_over_24_inputs_draw_and_capture` and `cases_within_the_input_cap_draw_and_capture` draw all 252 `basic_types`/`array` cases with no GL error |
| TF1b | `transform_feedback.array.separate.lines.highp_mat3` | same as TF1a | B3.9 | same as TF1a | **gone** on the host (`-ENOTSUP` otherwise) | 6581f22b; b7c1484e | same as TF1a |
| TF2a | `transform_feedback.basic_types.interleaved.lines.lowp_mat4` | same as TF1a | B3.9 | same as TF1a | **gone** on the host (`-ENOTSUP` otherwise) | 6581f22b; b7c1484e | same as TF1a |
| TF2b | `transform_feedback.basic_types.separate.lines.highp_mat4` | same as TF1a | B3.9 | same as TF1a | **gone** on the host (`-ENOTSUP` otherwise) | 6581f22b; b7c1484e | same as TF1a |
| TF2c | `transform_feedback.random_full_array_capture.interleaved.lines.1` | same as TF1a | B3.9 | same as TF1a | `-ENOTSUP` at most (fails one case). No host test models `random_full_array_capture` | b7c1484e; 6581f22b likely | none for this group |

What the tip removes:

- S03a and S07 (A5): the tip removes both aborts. Their 1,142 mustpass
  cases now begin; whether they pass is for the board.
- T05ca, T05cb, T05cc and T17a (B4.4, ETC2): the tip removes all four. The
  four ETC2 classes the GL core stores are sampled.
- TF1a, TF1b, TF2a, TF2b and TF2c (B3.9): none of them can abort at the tip,
  because the job builder's catch-all is `-ENOTSUP`. On the host the four
  `array`/`basic_types` cases draw. One caveat: `es3_tf_v3d.rs` models the
  failing cases with a 24-input cap. Its `lowp_mat4` (16 inputs) is under
  that cap, yet TF2a's `lowp_mat4` was refused on the board. Its cause may
  be something other than the input cap, and only a board run shows whether
  these cases pass or fail as one case each.
- T14a still aborts: the lost draw answers `-E2BIG`, which becomes
  GL_OUT_OF_MEMORY.

### E2. The image labels the post-b1b5b845 V3D changes reach

Taken from `git log b1b5b845..f77b0626 -- src/v3d driver/src/v3d` (37
commits besides the V3D 7.1 ones). The groups are mapped to
`tools/cts/pi4-gles3-tree.txt` labels by that file's filters. One line per
change. "Reaches" means the change alters what the V3D does for draws of
that image. It does not mean a row is known to move.

- 6d2e7d0d (A5 split, 4096 targets, `docs/notes/v3d-large-targets.md`): S03,
  S03a, S04, S05, S06, S07, S08 (`shaders.builtin_functions.precision`).
- 0ebbd507 + 48f6017c (a colour write mask per RT; the GL core sets it only
  where the written targets' masks differ, an R/RG integer target beside
  another): FR1, FR2 (`fragment_out.array`, `.random`) and D1
  (`draw_buffers_indexed`, mostly NotSupported in the GL core first, C4).
  It reaches FB3 and FB4 (`fbo.color`, `fbo.render`) only in their
  two-attachment cases.
- 9cccb4ae (per-RT BLEND_ENABLES; integer and 32F RTs unblended; GL_NONE
  RTs masked, several-target draws only): FR1, FR2, D1. Not FO1-FO5:
  one-target draws emit what they did.
- 684ef106 (BLEND_ENABLES names every RT when RT 0 blends; RGBA32F and
  RG32F tile-buffer rows): D1, FR1, FR2 for blending MRT draws. The 32F
  rows matter only with e59119a1's float colour targets, which images get
  only with `YAKOGL_COLOR_BUFFER_FLOAT=ON` (`CMakeLists.txt:113`, OFF by
  default). With it they reach FR1, FR2, FB2, FB3, FB4 and M1
  (`pbo.renderbuffer`).
- 9fd2209e + 8e285d55 (size words 44-47 from `texture::size_words` at
  `base_level`) and 337a9453 (`state_at`): no image at the tip.
  `GlesTextureBind::base_level` stays 0 because the draw does not apply
  GL_TEXTURE_BASE_LEVEL (`src/gl/draw.rs:3611`), and the commit says nothing
  moves. T07 (`texture.mipmap`) and S32a, S32b (`textureSize`) are reached
  once the GL core fills it.
- 298fe11c (uniform words 46/47: 3D depth and array layer count) with
  8ae0a44b (2D-array and 3D look-ups): T02, T03, T04, F3C1-F3C4, F3F1-F3F5,
  F3S1, F3S2 (`texture.filtering` 2d_array/3d), T05a, T06 (`texture.format`
  sized 2d_array/3d), T08, T09 (`texture.shadow` 2d_array), T10-T13
  (`texture.specification`), T14b (`texture.vertex`), S32a, S32b
  (`shaders.texture_functions`), R1 (`samplers`).
- e6d72a23 (colour attachments of different sizes draw and clear at their
  intersection): FB3, FB4 (`fbo.color`, `fbo.render`). 0 to a few rows
  (`docs/notes/v3d-mrt-sizes.md`). FB1's `fbo.completeness.size` only reads
  the status.
- 949f0bef (clears of glDrawBuffers lists whose draw buffer 0 is GL_NONE)
  and 822d8ca9 (draw-buffer order, **not wired**: `state::draw_buffer_order`
  has no caller in `backend.rs`, so such draws are still B1.5): D1, FR2, for
  clears only.
- 7fa02269 + 3c3e5470 (a scissored or masked clear of several attachments
  is one quad, A8): FO6 (`fragment_ops.scissor`, `.stencil` behind it),
  FB3 (`fbo.color.clear`, `.repeated_clear`), D1.
- 455c3bb5 + 333a19f7 + 765cbf81 (ETC2/EAC sampled): T05b, T05c, T05ca,
  T05cb, T05cc, T05d, T17, T17a, T17b.
- 6581f22b + 218c90ec (fragment inputs 24..63, 15 varying vectors): TF1,
  TF1a, TF1b, TF2, TF2a, TF2b, TF2c, and shader cases with more than 24
  fragment input scalars (image not identified here).
- 080240bc (a unit that only `textureSize` reads takes a 32-bit texture):
  S32a, S32b.
- b7c1484e (refusals of legal GL state are `-ENOTSUP`): every image. A
  refusal now fails its case instead of ending the image.

### E3. What is left, and the shape of a fix

- T14a (lost draw, `-E2BIG`): a V3D-only fix is likely, but the code alone
  does not give it. The eight cube units' kept copies (2-4 MiB each) do not
  find memory in the 48 MiB carve-out while 9.5 MiB of copies are live. The
  arena fallback cannot hold them, so the draw is lost. Candidates in
  `src/v3d` are freeing the kept copies no unit of this draw samples before
  the no-memory answer, or a carve-out and budget sizing that fits eight
  cube units. A host reproduction (8 cube units at the case's sizes, with
  `fail_alloc_from`) should come first. Answering `-ENOTSUP` here would
  hide a real shortage. GL_OUT_OF_MEMORY for every non-`-ENOTSUP` code is
  `src/gl/draw.rs:683`, which is outside this plan (named only).
- TF1a, TF1b, TF2a, TF2b, TF2c (B3.9): no abort at the tip. If the board
  still refuses one, the cause is unnamed, because `v3d_job_draw` returns
  one `V3D_JOB_REFUSED` for `draw_is_supported`, `record_draw` and a bad
  field alike (`v3d_job.c:1078`, `:1093`). Splitting that status would be a
  V3D-only change that names the cause; it needs no `src/gl` change.
  TF2a's refused `lowp_mat4` (16 inputs, under the old cap) says a second
  cause is possible.
- S03a, S07, T05ca, T05cb, T05cc, T17a: nothing is left on the V3D side
  for the abort. Their cases now produce results on the board.

## F. The four causes behind the eleven stopped images of the aborts session

Session `tools/cts/log/2026-11-pi4-gles3-b1b5b845-aborts/` (plan #398, issue
#2): eleven of its 14 images stopped again, each at one refused case. Section
E has one row per image; this section groups them by cause and says, for each,
the error kind and whether the limit is the V3D 4.2 hardware or a missing
driver path. Code is read at `35e03a2e`. Four V3D 7.1 commits (1ee37fe7,
7f791cde, 96ade25d, b066d160) lie between `89d3e1c3` and it in `src/v3d` and
`driver/src/v3d`; `git diff 89d3e1c3 35e03a2e` is empty for `backend.rs`,
`texture.rs`, `src/backend.rs`, `v3d_job.c` and `src/gl/draw.rs`, so E's lines hold.
The console strings are the `draw refused:` lines of the console files;
`b1b5b845` answered every one with `-EINVAL`, which the GL core turns into
GL_OUT_OF_MEMORY (`src/gl/draw.rs:683`), dEQP's ResourceError, and the image
ends. At the tip `-ENOTSUP` fails one case instead; `-E2BIG` still aborts.

| Cause | Refusing cases (`dEQP-GLES3.functional.`) | Refusal string | Where at `35e03a2e` | Kind at the tip | Hardware or driver | Issue |
| --- | --- | --- | --- | --- | --- | --- |
| (a) target past the tile memory | S03a `shaders.builtin_functions.precision.abs.highp_fragment.vec2` | a target larger than the tile memory is sized for (2048x2048), or empty (the tip's text: `(4096x4096)`) | `backend.rs:6826-6827` against `MAX_TARGET` = 4096 (`backend.rs:447`) | `-ENOTSUP` (was `-EINVAL`). The shader executor's 4096 x 2-6 targets are no longer refused | driver: the 2048 cap was the init sizing of the tile memory, not a V3D limit (`docs/notes/v3d-large-targets.md`) | #2 |
| (b) ETC2 not sampled | T05ca `texture.format.compressed.etc2_eac_rgba8_cube_pot`; T05cb `...etc2_rgb8_cube_npot`; T05cc `...etc2_rgb8_cube_pot`; T17a `texture.wrap.etc2_eac_rgba8.clamp_clamp_linear_pot` | a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here) | `texture.rs:390` (`NOT_SAMPLED`), `:439-440`; `GAPS` `texture.rs:404`, answered at `backend.rs:4497-4499`; the set is `V3D_SAMPLED_TEXFMTS`, `src/backend.rs:290-309` | `-ENOTSUP` for a format outside the set. The four ETC2 classes are in the set, so these cases are no longer refused | driver: the TMU has the types (below) | drafted: "V3D: ETC2/EAC texture formats outside the four GL-core classes (ETC2 RGB8 A1, EAC R11/RG11) are not sampled" |
| (c) no GPU memory for an empty job | T14a `texture.units.8_units.only_cube.2` | no empty job holds the draw (its lists, or its client data with no memory of its own), after `osal(v3d): out of GPU memory` for 2,117,632, 3,149,824 and 4,206,592 bytes | `backend.rs:6214-6217` (`count_refusal` then `-E2BIG`); the kept copy finds no memory at `backend.rs:4878-4892`; the arena fallback `Attempt::Full` at `backend.rs:4768` | `-E2BIG`, **still aborts** (GL_OUT_OF_MEMORY). Not changed by any commit since `b1b5b845` | driver and board config: eight cube units of 2-4 MiB kept copies against a 48 MiB carve-out with 9.5 MiB live (`tiled_budget`, `backend.rs:1401`, `:4849`); V3D 4.2 itself addresses 32 bits | drafted: "V3D: eight cube units (texture.units.8_units.only_cube.2) lose the draw for lack of GPU memory and abort the image" |
| (d) job builder catch-all | TF1a `transform_feedback.array.interleaved.lines.highp_mat3`; TF1b `...array.separate.lines.highp_mat3`; TF2a `...basic_types.interleaved.lines.lowp_mat4`; TF2b `...basic_types.separate.lines.highp_mat4`; TF2c `...random_full_array_capture.interleaved.lines.1` | the job builder refused the draw (a uniform, program or primitive it does not write) | `backend.rs:3988-3990` (`Attempt::Unsupported`) over `v3d_job_draw`'s one `V3D_JOB_REFUSED` (`driver/src/v3d/v3d_job.c:1077-1078`, `:1093`); `draw_is_supported`, `:868`, with the fragment-input cap `num_inputs > 64` at `:876` | `-ENOTSUP` at the tip (one Fail, no abort). `b1b5b845` capped inputs at 24 | driver for the 24-input cap (Mesa's `V3D_MAX_FS_INPUTS` is 64, `third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h:32`). TF2a's `lowp_mat4` (16 inputs) and TF2c are not explained by the cap: cause unknown | drafted: "V3D: split V3D_JOB_REFUSED so a refused draw names its cause" (first step); TF2a/TF2c cause after it |

Hardware side of (b), read in the vendored Mesa (read only): the v3d format
table maps `ETC2_RGB8`, `ETC2_SRGB8` to `RGB8_ETC2`, `ETC2_RGBA8` and
`ETC2_SRGBA8` to `RGBA8_ETC2_EAC`, and the EAC R11/RG11 types, as sampler
formats (`third_party/mesa-26.2.1/src/gallium/drivers/v3d/v3dx_format_table.c:159-168`),
and `v3d_packet.xml` lists the texture data formats `RGB8_ETC2` (32),
`R11_EAC` (34), `RG11_EAC` (36-37) and `RGBA8_ETC2_EAC` (38)
(`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:371-379`). So the V3D 4.2
TMU does have an ETC2 RGBA8 type. The refusal was a missing driver path, and
the tip removes it for the four classes (455c3bb5).

Which causes can still end an image on the board: only (c). (a), (b) and (d)
cannot abort at the tip by their return codes; whether their cases pass or
fail as one case each is for the board. Board sessions `board1` and `board2` of
plan #398 confirm or move each entry: a case that still refuses there stays on
its issue draft, not reported as fixed.
