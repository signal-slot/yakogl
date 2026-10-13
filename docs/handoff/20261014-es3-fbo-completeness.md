<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# es3-fbo-completeness, 2026-10-14, plan #19 item fbo-fragops

Base: integration tip `20a4879`. Code: `src/gl/framebuffer.rs` (`check_status`)
and its unit tests. Tests: `tests/es3_fbo_fragops.rs`.

## Open work

### What was run

Host dEQP (VK-GL-CTS `93bca01`, `tools/cts/host-deqp-run.sh --module=gles3`,
`dEQP-GLES3.functional.fbo.completeness.` and `.fbo.`), library from the base
and from this change. **HOST-ONLY**: the LINUX OSAL does not run the command
stream, so every case that reads pixels is meaningless here. What is real: a
framebuffer's status, the GL errors, and shader compile and link. The
completeness cases decide on status and error only, so their verdicts are real.

The AM62P numbers in `docs/es3-cts-failures.md` are from main `396d667a`
(before int-rt-gl, gl-core-misc and the one-sample renderbuffer); this tip is
newer, so the "AM62P" counts there and the host counts here are not the same
population. Nothing below is a board result.

### Cases before and after

| Group (cases) | Cause | Host before | Host after | Left for |
|---|---|---|---|---|
| `completeness.size` (2) | `size.distinct`: depth larger than colour was `INCOMPLETE_DIMENSIONS`, ES 3.0 has no size rule | `distinct` Fail (PowerVR, GC7000); `zero` Pass | `distinct` Pass on all three GPUs | – |
| `completeness.attachment_combinations.none_{rbo,tex}_*` (14) | colour only on attachments 1.. was `UNSUPPORTED`, ES 3.0 asks nothing of attachment 0 | 14 Fail (PowerVR, GC7000UL) | 14 Pass (all three GPUs) | – |
| `attachment_combinations.none_none_{rbo_tex,tex_rbo}` (2) | depth and stencil as two images with no colour: must be `UNSUPPORTED` | 2 Fail | 2 Pass | – |
| `attachment_combinations.*_{rbo_tex,tex_rbo}` with a colour 0 (12) | the same rule with a colour 0 (below) | 12 Fail | 12 Fail | draw.rs / v3d tests, see "Blocked" |
| `completeness.samples` (13) | sample counts | 6 Pass, 7 NotSupported (rbo2, rbo4: `GL_INVALID_OPERATION`, as `GL_MAX_SAMPLES` is 1) | same | – (already right on this tip; the AM62P Fails predate gl-core-misc's one-sample renderbuffer. Pinned by a test) |
| `completeness.layer` (8) | layer vs depth | 2darr x4, 3d_1_0, 3d_1_15, 3d_4_15 Pass on all GPUs; `3d_64_15` Pass on GC7000 and V3D, **Fail on PowerVR** | same | `3d_64_15`: see "Blocked" |
| `fbo.depth` (15) | D16 / D24 / D32F / D24S8 / D32FS8 attach | attach and status Pass everywhere. `basic` x5: Image comparison (host: meaningless). `depth_test_clamp` x5 and `depth_write_clamp` x5: `Compile failed` = `Usc(FragmentOutputsNotBuilt)`, `gl_FragDepth` | same | the 10 clamp cases: PowerVR shader back end (`gl_FragDepth`); the 5 `basic` cases need a board |
| `renderable.*` D32F / D32FS8 rows (rbo and texture) | the "rank 5" depth formats | Pass (every `depth_component32f`, `depth32f_stencil8` row) | same | – (`is_depth_format` has them; attach, sizes and component type are now pinned) |

Totals (cases are `fbo.completeness.*`, 560 on the host list):

| GPU | Pass before | Pass after | Fail before | Fail after | Regressions |
|---|---|---|---|---|---|
| GC7000UL (`vivante`, run on the full `fbo.completeness.`) | 442 | 459 | 111 | 94 | 0 |
| PowerVR (`powervr`, `fbo.` whole group, 2,028) | 517 | 534 | 1,256 | 1,239 | 0 |
| V3D (`v3d`, `fbo.completeness.`) | not measured | 436 | not measured | 117 | not measured |

17 cases moved from Fail to Pass on both GPUs that have a before run
(`comm` of the failure lists: 17 fixed, 0 new). V3D has no before run (its
library was not built from the base); its 117 are the same causes (98 of
them `Expected no error but got GL_INVALID_OPERATION`: integer textures the
back end does not sample, cause 4 below).

### Causes left, with the file that holds each

1. **Separate depth and stencil images with a colour attachment 0** (12
   `attachment_combinations` cases; the other six `none_*` ones are fixed).
   ES 3.0 4.4.4.2: depth and stencil, if both present, are the same image,
   else `GL_FRAMEBUFFER_UNSUPPORTED` (`GL_EXT_separate_depth_stencil` is not
   claimed). The rule is in `check_status` for the case with no colour 0.
   Applying it to every framebuffer breaks unit tests this item must not
   edit, because they build ES 2.0-style separate D16 + S8 renderbuffers
   (or two depth-stencil textures) on an ES 3.0 build and expect `COMPLETE`:
   `src/gl/draw.rs` `fbo_depth_and_stencil_renderbuffers_both_name_the_ds_buffer`
   and `one_depth_stencil_texture_on_both_points_is_one_object`,
   `src/v3d/backend_tests.rs` `fbos_sharing_a_depth_renderbuffer_with_their_own_stencil`
   and `a_recreated_stencil_renderbuffer_keeps_the_fbos_depth`. All four
   pass again with `if crate::REPORT_ES2` on the part that makes the pair
   (draw.rs: `return` early in an ES 3.0 build; v3d: the rigs' second
   framebuffer). Checked on this tip: with the general rule and the draw.rs
   edit, the PowerVR, GC7000UL and ES 2.0 library tests are green and only the
   two V3D rigs fail. Land it by dropping the `fb.color.type_ == GL_NONE`
   term in `check_status`, deleting the ignored-test marker of
   `separate_depth_and_stencil_with_colour_zero_are_unsupported`, and editing
   those four tests. The two V3D rigs exercise the V3D's split Z and S
   buffers, so they have to build their framebuffers with one D24S8
   renderbuffer (or run the separate-image part only with `report-es2`).
2. **`GL_R32F` is a render target here** (`renderable.renderbuffer.color0.r32f`,
   `renderable.texture.color0.r32f`): ES 3.0 makes it colour-renderable only
   with `GL_EXT_color_buffer_float`, which GL_EXTENSIONS does not list, so
   dEQP wants `GL_INVALID_ENUM` at the renderbuffer and an incomplete texture
   attachment. The format has a surface (`GLES_SURF_R32F`) and a blend class
   (`SHC_RT_F32`, class 3), and `src/gl/draw.rs` has a test that draws into it
   (`a_float_render_target_selects_the_variant_packed_for_its_class`). Keeping
   R32F renderable is a decision, not an oversight; making it unrenderable
   (a `is_color_renderable` that asks `extension_listed("GL_EXT_color_buffer_float")`
   at the three completeness sites and in `is_renderbuffer_color_format`) also
   needs that test, and framebuffer.rs's three R32F unit tests, changed. Not
   done here.
3. **`renderable.texture.color0.rgba32f`** (1): `GL_RGBA32F` is stored as
   halves (`texture.rs` `sized_format`) and keeps no record that it was
   32-bit, so `attachment_storage` cannot tell it from `GL_RGBA16F`. Needs
   texture.rs to remember the sized format (e.g. an `es3_fmt` for the float
   class).
4. **Integer texture colour attachments** (PowerVR 19 cases,
   `renderable.texture.color0.{r,rg,rgba}{8,16,32}{i,ui}`, `rgb10_a2ui`):
   `attachment_storage` accepts only RGBA8, sRGB8_A8, RGBA16F, R16F and R32F
   textures. An integer texture is held in an RGBA-class store
   (`GLES_TEXFMT_*I/UI`), which must be rendered as the matching
   `GLES_SURF_*` and read back through `color_kind`. That is the texture half
   of int-rt (`int-rt-pvr`, `-v3d`, `-gc`), not a completeness rule. GC7000UL
   and V3D refuse the upload first, so there the 75 `Expected no error but got
   GL_INVALID_OPERATION` cases are texture.rs / back-end sampling
   (`formats-gc`, `formats-v3d`), not this item.
5. **Unsized depth and depth-stencil textures** (4 cases,
   `renderable.texture.{depth.depth_component_unsigned_int,
   depth_component_unsigned_short, depth_stencil_unsigned_int_24_8;
   stencil.depth_stencil_unsigned_int_24_8}`): ES 3.0 has no unsized
   `GL_DEPTH_COMPONENT` / `GL_DEPTH_STENCIL` internal format for
   `glTexImage2D`, so dEQP expects a GL error at the upload. The texture is
   accepted (ES 2.0's `GL_OES_depth_texture` rule). texture.rs must refuse it
   when `!REPORT_ES2`; the framebuffer is not the place.
6. **`completeness.layer.3d_64_15` on PowerVR**: a 3D texture deeper than one
   slice keeps its slices in a grid (`GlTexture::grid_cols > 1`), and
   `attachment_storage` returns `None` for it. A slice is a sub-rectangle of
   the grid with the grid's row pitch (`grid_cols * width`), so the fix is
   the slice's offset and pitch in `attachment_storage` and `loc_pitch`, and
   every consumer (the draw in draw.rs, the blit in blit.rs, glReadPixels,
   glClear) has to follow. It crosses files this item does not own; a test is
   written for the right answer and ignored
   (`a_3d_slice_below_the_depth_is_complete`). The same `grid_cols > 1` test
   is what ends 13 `fbo.color.tex3d.*` rows (`r8`, `rg8`, `rgb8`, `rgba8`,
   `rgb565`, `rgba4`, `rgb5_a1`, `rgb10_a2`, `srgb8_alpha8`, `r16f`, `rg16f`,
   `rgb16f`, `rgba16f`) in `Framebuffer is not complete` on PowerVR (host).
   Separate and unanalysed: every `rgb16f` colour row (`tex2d`, `tex2darray`,
   `tex3d`) is incomplete the same way, so a `GL_RGB16F` *texture* does not
   attach; that is a texture-format-to-attachment gap in
   `attachment_storage` (its `GL_RGB16F` texture is held as `RGBA16F` or not
   at all; not read here).
7. A **draw into a framebuffer with colour only on attachments 1..** is still
   `GL_INVALID_FRAMEBUFFER_OPERATION` (`target_loc` takes attachment 0).
   It is complete now, as ES 3.0 says. Which attachment a draw writes is
   `glDrawBuffers`' (the MRT item, `mrt4`), and that item has to make
   `target_loc` / `fb_target_drawable` follow `draw_buf`.

### Behaviour changed (ES 3.0 build only; `report-es2` is unchanged)

- `check_status`: depth and stencil images are no longer required to have
  the colour's size (no `GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS`). The back ends
  were already told the depth size apart (`GlesDraw::ds_w`, `ds_h`; PowerVR
  allocates by the target and `depth_resolve` clamps to the buffer; GC7000UL
  takes the larger). Not checked: a draw into a depth image of another size
  than the colour on a board. The PowerVR back end's bounds were read, not
  run; the V3D back end's were not read. The board sessions should run
  `fbo.completeness.size.distinct` and `fbo.depth.*` with a watch for MMU
  faults.
- `check_status`: colour on attachments 1.. alone is complete; with no colour
  at all, depth and stencil as two images is `UNSUPPORTED`.
- Unit tests of `framebuffer.rs` that pinned the ES 2.0 answers on an ES 3.0
  build now ask `crate::REPORT_ES2` (`status_depth_size_mismatch_*`,
  `status_stencil_size_mismatch_*`,
  `status_depth_and_stencil_of_different_sizes_without_colour_*`,
  `status_colour_attachment_1_alone_*`,
  `status_follows_the_texture_storage_being_respecified`,
  `depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say`),
  the same way `status_colour_attachment_1_of_another_size` already did. With
  `report-es2` they assert the same values as before.

### Checks run

- `cargo test --test es3_fbo_fragops` (8 pass, 2 ignored, each ignored one
  written for the right answer and named above) on `host-test,gpu-powervr`,
  `host-test,gpu-vivante` and `host-test,gpu-v3d,shaderc-online,no-shader-table`;
  with `report-es2` it runs `es2_report_is_unchanged` (1 pass).
- Without the change `images_of_different_sizes_are_complete` and
  `colour_attachments_past_zero_complete_a_framebuffer_alone` fail; the rest
  are regression pins (samples, layers, depth formats pass before and after).
- `cargo test --lib` on the three GPUs and on `gpu-powervr,report-es2`: all
  green. `tests/es2_report_guard.rs` (13 pass, 1 ignored) and
  `tests/es2_error_rules_fbo.rs` (6 pass) with `report-es2`, unchanged; both
  run zero tests without it.

## Known defects

- Open, from the table: the 12 colour-0 `attachment_combinations` cases (1),
  R32F (2), RGBA32F (3), the integer texture attachments (4), the unsized depth
  textures (5), the 3D grid slice (6), and the draw into colour-1-only (7).
