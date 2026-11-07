<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: the `fbo.blit.*` not-pass rows, their causes, and what a V3D-side blit would fix

Research for plan #330, item v3d-fbo-blit-note. Read at main `434d2695`. No
code was changed and nothing ran on a board. Every statement marked **[inferred]**
comes from reading the code against the dEQP source, not from a Pi 4 message:
`docs/es3/pi4-gles3-fails.tsv` keeps a status per row and no message, and
image FB2's console (`build/f-FB2.txt`, the `925dbca9` run) is not in the tree.
The AM62P file `docs/es3/am62p-gles3-notpass.tsv` keeps messages and has
the same blit group (at an older commit), which is how most causes below are
told apart from the V3D's own.

Not repeated here, linked: sRGB render and sample, `docs/es3/srgb.md`;
4x multisample and its resolve, `docs/notes/v3d-msaa-es3.md`; colour
attachments of different sizes, `docs/notes/v3d-mrt-sizes.md`; the window's
and cube/array depth, `docs/notes/blit-depth-window.md`; the TFU,
`docs/notes/v3d-tfu-order.md`, `docs/notes/v3d-tfu-raster-chain.md`; the lazy
raster store, `docs/notes/v3d-lazy-raster.md`.

## 1. The rows

All 194 `dEQP-GLES3.functional.fbo.blit.*` rows of the tsv come from one image,
FB2 (`925dbca9`). Nothing else under `fbo.blit` is in the file, so every other
`fbo.blit` case (every other group, the
`conversion` pairs without an sRGB, 32F or 11F end, the other
`default_framebuffer` cases) passed on the Pi 4.

| group | rows | status | pattern (under `fbo.blit.`) | cause | layer | V3D-side blit fixes it? |
|---|---|---|---|---|---|---|
| **G1** | 112 | NotSupported | `conversion.*` with an `r32f`, `rg32f`, `rgba32f` or `r11f_g11f_b10f` end (every pair that has one of the four names as source or destination, e.g. `r32f_to_r8`, `rgba8_to_rg32f`, `srgb8_alpha8_to_r32f`) | the build does not list `GL_EXT_color_buffer_float`, so dEQP's `preCheck` (`checkFormatSupport`) skips the case; the GL core also has no colour-renderable name for RG32F, RGBA32F, R11F_G11F_B10F (`is_color_format`, `src/gl/framebuffer.rs:602`) | GL core (extension, formats, blit codec) + each back end's render | no |
| **G2** | 23 | Fail | `conversion.*_to_srgb8_alpha8` (11: r16f, r8, rg16f, rg8, rgb10_a2, rgb565, rgb5_a1, rgb8, rgba16f, rgba4, rgba8) and `conversion.srgb8_alpha8_to_*` (12: the same eleven and `srgb8_alpha8`) | every such case **draws** a gradient into an sRGB renderbuffer (src or dst, `BlitColorConversionCase::render`: one `GradientShader` draw per renderbuffer) and reads it back; the blit itself is the CPU codec, which passes the same cases on the AM62P. Only the V3D's sRGB render/read differs [inferred] | V3D back end (sRGB render target) | no: the blit is not at fault |
| **G3** | 22 | NotSupported | `default_framebuffer.{r11f_g11f_b10f,r32f,rg32f,rgba32f}` and their `*_{nearest,linear}_{scale,out_of_bounds}_blit_{from,to}_default` (7 + 5 + 5 + 5) | as G1: `checkFormatSupport` of the FBO texture's format | GL core + back ends | no |
| **G4** | 3 | Fail | `default_framebuffer.r8_linear_out_of_bounds_blit_from_default`, `r8_linear_scale_blit_from_default`, `r8_nearest_scale_blit_from_default` | **GL core, not V3D**: same three rows fail on the AM62P ("Image comparison failed"). Inferred cause in section 3.2: an R8 colour image is stored as RGBA8 bytes and `glReadPixels` returns the stored green left by the clear, where ES wants 0 | GL core | no |
| **G5** | 6 | Fail | `default_framebuffer.srgb8_alpha8_{linear_out_of_bounds,linear_scale,nearest_scale}_blit_{from,to}_default` | the FBO texture is sRGB: cleared to yellow, or the grid drawn into it, then read back. Pass on the AM62P; as G2 [inferred] | V3D back end (sRGB target, texture-backed) | no |
| **G6a** | 8 | Fail | `depth_stencil.depth_component{16,24,32f}_{basic,scale}` (6), `depth_stencil.depth{24_stencil8,32f_stencil8}_depth_only` (2) | the destination is a depth renderbuffer; the V3D table has `ds_write: None` (`src/v3d/backend.rs:6460`), so `blit_framebuffer` sets `REFUSED_DEPTH_DST` (`src/gl/blit.rs:1143-1146`, `rb_dst` needs `backend::can_ds_write()`) and the call is `GL_INVALID_OPERATION`, logged once (`refuse_ds`). The AM62P rows had the same message before its `ds_write` landed (`GL_INVALID_OPERATION at es3fFboTestCase.cpp:90`) | V3D back end | **yes**: `ds_write` (depth plane) |
| **G6b** | 8 | Fail | `depth_stencil.depth{24_stencil8,32f_stencil8}_{basic,scale,stencil_only}` (6), `depth_stencil.stencil_index8_{basic,scale}` (2) | stencil: a renderbuffer destination needs `ds_write` (stencil plane) and a renderbuffer source needs `stencil_resolve` (`src/gl/blit.rs:1119-1129`, `REFUSED_STENCIL`); the V3D has neither (`:6460-6461`) | V3D back end | **yes**: `ds_write` (stencil plane) + `stencil_resolve` |
| **G7** | 12 | NotSupported | `depth_stencil.depth_component{16,24,32f}_stencil_index8_{basic,depth_only,scale,stencil_only}` | "Separate depth and stencil buffers not supported": dEQP wants `GL_EXT_separate_depth_stencil` (ES 3.0 makes a depth and a stencil attachment of different renderbuffers incomplete); the build does not list it. Same on the AM62P | GL core (`check_status`, the extension string) | no |

112 + 23 + 22 + 3 + 6 + 8 + 8 + 12 = 194. The sums per pattern match the tsv:
`conversion` 135 (112 NotSupported, 23 Fail), `default_framebuffer` 31
(22 NotSupported, 9 Fail), `depth_stencil` 28 (12 NotSupported, 16 Fail).

Which rows are **Pi 4 only** (not in the AM62P list): G2 and G5, 29 rows, all
with an sRGB end. G6 is Pi 4 and the other two back ends without `ds_write`
(GC7000UL, Mali); G1, G3, G4, G7 are the same on every back end.

## 2. How `glBlitFramebuffer` runs on the V3D today

`src/gl/blit.rs` is a CPU copy, shared by the PowerVR, V3D, GC7000UL and Mali
back ends. Nothing in `src/v3d` knows about it. The path, and whether the V3D's
images reach it correctly:

1. **The GPU is idle first.** `gpu_idle_for_cpu` (`src/gl/framebuffer.rs:2925`)
   flushes a pending clear (`draw::flush_pending_clear`), `backend::flush`,
   `backend::finish`, and checks the lost-work counter; lost work leaves the
   destination alone with no error.
2. **Render images are linear when the CPU sees them.** A V3D colour image is
   never read tiled by the blit. A renderbuffer or the window is raster memory
   the job stores into (TLB `STORE_TILE_BUFFER_GENERAL`, RGBA8 as R,G,B,A,
   `rt_rgba8888_is_bgra: false`). A texture rendered into has two copies: the
   GL core's raster storage and, for a sampled 2D RGBA8 texture, a UIF render
   image (`RtImage`, `rt_texture`, `src/v3d/backend.rs:1779`). With the lazy
   raster store (`docs/notes/v3d-lazy-raster.md`) the raster storage can be
   stale; `gpu_idle_for_cpu` therefore calls `texture::storage_back` for every
   texture attached to the read and the draw framebuffer, which reaches
   `v3d_storage_current` -> `rt_level0_back` (waits for the job, untiles level
   0 with `v3d_tex_load_box` into the raster rows, `:2075-2117`). Both the
   source and the destination are covered, so a blit reads and writes current
   bytes. The window is raster and stored top-down (`Image::flip`,
   `default_y_up`).
3. **Cache coherence.** `invalidate` before the source is read and before a
   destination row is written, `write_back` (`osal::cache_flush`) after
   (`blit_colour`, `src/gl/blit.rs:437-458, 726-775`). Same discipline as
   `glReadPixels` and `rt_level0_back`.
4. **A destination texture is restamped.** `note_blit_written` takes a new
   `content_gen`, so `rt_sample` sees "a stamp the image was not stored under"
   (`CpuWrite`, `:1918-1925`), takes the tiled-copy path and waits for the next
   render to store the image again. The blit's write is therefore seen by the
   next sample and by the next job that loads the target from its raster
   storage.
5. **Conversions.** One decode and one encode per storage class
   (`decode_float`/`encode_float`/`decode_int`/`encode_int`): unorm per the
   *format's* bits, RGB565 packed, RGBA16F/R16F/R32F, the integer classes
   saturated, sRGB through `gl::srgb`, channels the format lacks read 0
   (alpha 1) and are written 0 (alpha 1). Host test:
   `gl::blit::tests::*` (`float_and_unorm_convert_both_ways`,
   `srgb_buffers_decode_as_the_source_and_encode_as_the_destination`,
   `channels_a_format_lacks_read_and_write_zero_and_alpha_one`,
   `integer_copies_saturate_to_the_destination`). No Pi 4 row fails on the
   arithmetic of a pair it can store.
6. **Filters, flips, out-of-bounds.** `Axis`/`Rect` do the spec's affine map
   of the uncut rectangles, cut to the read buffer, the draw buffer and the
   scissor, NEAREST or four-tap LINEAR with clamp-to-edge in the cut source;
   reversed rectangles flip; a destination pixel whose sample point is
   outside the cut source is left alone. Host tests `reversed_rectangles_flip`,
   `nearest_scales_up_and_down`, `out_of_bounds_rectangles_cut_and_keep_the_scale`,
   `linear_filters_where_the_sizes_differ`, `the_scissor_cuts_the_destination`.
   No row of the 194 is about geometry: the scale and out-of-bounds default
   framebuffer cases that fail do so for a format reason (G4, G5, G3).
7. **Depth and stencil** (`copy_depth`, `src/gl/blit.rs:898`): depth is read
   with `backend::depth_resolve` (the V3D has it: `v3d_depth_resolve`,
   `src/v3d/backend.rs:4786`, untiles the D24S8 buffer the jobs stored) or
   from a depth texture's R32F texels; it is written into a depth texture's
   texels (GL core) or through `backend::ds_write`, which the V3D lacks.
   Stencil needs `stencil_resolve` (source renderbuffer) and `ds_write`
   (destination); the V3D lacks both. That is G6.
8. **Multisample.** A 4x read buffer is resolved only between identical
   rectangles of identical formats, the draw side must be single-sample; the
   V3D leaves the resolved image in the renderbuffer's memory
   (`docs/notes/v3d-msaa-es3.md`, "3.2"). No `fbo.blit` case is multisampled
   (those are `fbo.msaa.*`), so nothing in this note depends on it.

Conclusion: the V3D's raster/tiled storage reaches the CPU blit correctly (the
mechanisms of 2-4 are the ones `glReadPixels` uses and are host-tested in
`tests/v3d_lazy_raster_readers.rs`-style rigs; the blit is not a separate
reader). The 194 rows are not failures of that path. What stands between the
Pi 4 and these rows is a missing extension (G1, G3, G7), the GL core's
treatment of narrow formats (G4), the V3D's sRGB target (G2, G5) and two
missing back-end entries (G6).

## 3. Per group

### 3.1 G1, G3: `GL_EXT_color_buffer_float` (134 rows)

dEQP lists the case only when `checkFormatSupport` passes, and for the 32-bit
float and the 11/11/10 formats it asks for the extension. The 16-bit pairs run
(`GL_EXT_color_buffer_half_float` is listed, `src/gl/state.rs:111`) and pass on
the Pi 4, which is why only pairs with one of the four names are missing.

Exposing the extension is an all-or-nothing claim: it makes R16F, RG16F,
RGBA16F (done), R32F (named renderable today but not through the extension),
RG32F, RGBA32F and R11F_G11F_B10F colour-renderable, for textures and
renderbuffers, and dEQP then runs every pair and `fbo.render.*`/`fragment_out.*`
cases of those names. What it takes, by layer:

- GL core: the extension string and the names in `is_color_format`/
  `is_renderbuffer_color_format`/`renderbuffer_surf_format`
  (`src/gl/framebuffer.rs:602-733`); storage and `GLES_SURF_*` classes for
  RG32F (8 B), RGBA32F (16 B; held as halves today, `t.rgba32f`) and
  R11F_G11F_B10F (4 B); blit codec arms in `decode_float`/`encode_float`
  (R32F exists, the other three do not; the 11/11/10 packing is a new
  decode); `glReadPixels` float paths; the draw's output class
  (`SHC_RT_*`, `src/shaderc`).
- V3D: the hardware renders all of them (`v3dx_format_table.c:96,101,146`:
  RG32F, RGBA32F, R11F_G11F_B10F have RT formats; 32F types are 64/128 bpp
  tile-buffer cases, so tiles shrink, `v3d_frame_tiling_bpp`, `MAX_JOB_TILES`
  at `src/v3d/backend.rs:435`), but no float32 surface class exists in
  `state::rt_format`; Mesa also blends 32F in the shader
  (`v3d_fb_needs_soft_blend`, `v3dx_state.c:517-525`, quoted by
  `docs/notes/v3d-msaa-es3.md`), which yakogl would have to match or refuse.
- Not a blit item: a V3D-side blit changes nothing here. Per-format V3D
  state is the format-matrix note's (plan #330 item v3d-format-matrix-note),
  the `fragment_out` consequences the fragment-out note's.

### 3.2 G4: R8 from the default framebuffer (3 rows) [inferred]

`BlitDefaultFramebufferCase` (`es3fFramebufferBlitTests.cpp`) clears the FBO
texture to yellow (`glClearBufferfv`, 1,1,0,1), blits the window into it, and
reads the FBO texture back with `glReadPixels`. The scaled and out-of-bounds
variants leave part of the destination as the clear left it; the full-size
`r8` case (every pixel blitted) passes, `r8_*_to_default` pass (destination is
the window), and `rg8`, whose stale channels are blue and alpha, written 0
and 1 by the clear, pass. The failing three are exactly the generated
`blit_from_default` variants of the one format whose clear writes a channel
the format lacks (green = 1).

The GL core stores R8 as RGBA8 bytes, the clear writes all four, and
`read_pixels` (`src/gl/framebuffer.rs:2969`) returns the stored bytes; only
the blit codec masks by the format's bits (`decode_float`, `src/gl/blit.rs:511`).
ES wants (r, 0, 0, 1) from an R8 image. The same fault would show in other
narrow-format reads after a clear; it is not V3D-specific (the AM62P has the
same three rows). Fix, in the GL core: mask the lacking channels in the
read-back (or in the clear), per `AttachmentBits`. Host test with the real
GL core: clear an R8 renderbuffer with (1, 1, 0, 1), read RGBA/UNSIGNED_BYTE,
expect (255, 0, 0, 255); the same for a blit into R8 of a part of the image.

### 3.3 G2, G5: sRGB (29 rows)

The conversion cases and the six `default_framebuffer.srgb8_alpha8_*` rows are
the Pi 4's only blit rows the AM62P passes. The blit codec is shared and
passes there, so the V3D's sRGB target is what differs: a draw or a clear of
a `GLES_SURF_SRGB8_A8` target, the read-back of it, or the sampling of an sRGB
texture. `src/v3d/state.rs:463` has the row (TLB `SRGB8_ALPHA8` output, 16F
tile buffer, `NORM` clamp); `src/v3d/backend.rs:5978-5981` refuses an sRGB
colour attachment *past the first* in a multi-target job (the comment says the
surface has no row, the table has one; stale comment). Whether a single sRGB
target renders correctly on the board is unmeasured (`docs/es3/srgb.md`:
"Nothing is measured on a board yet"), and the image that produced these rows
(`925dbca9`) may predate parts of it. The next step is not a blit change:

1. host tests of an sRGB target's draw and clear on the V3D (plan #330 item
   v3d-srgb-draw, `tests/es3_srgb.rs`, `tests/es3_int_rt_v3d.rs`);
2. a Pi 4 rerun of image FB2 at current main, which gives the first failure
   message of each of the 29 rows (plan #320). A "draw refused" message in the
   log would put the cause at `refuse_undrawable_targets`
   (`src/gl/draw.rs:2130`); "Image comparison failed" would put it at the TLB
   conversion, the 16F tile buffer or the clear (`state::clear_words`).

### 3.4 G6: depth and stencil between renderbuffers (16 rows)

Design: give the V3D back end the two missing entries; the GL core does not
change. This is the one group a back-end change fixes, and it is a CPU entry
like the PowerVR's (`pvr_backend_ds_write`, `pvr_backend_stencil_resolve`),
not a GPU blit.

`YakoglBackend::ds_write(id, plane, src, x, y, w, h, pitch) -> i32`
(`src/backend.rs:1592`, contract at `:2056-2093`): write a rectangle of the
depth plane (f32 in [0, 1]) or the stencil plane (u8) the back end keeps for
GL object `id`, rows top-down in the layout `depth_resolve`/`stencil_resolve`
read; 0 written, 1 "the back end holds nothing for the plane", negative errno;
an empty rectangle answers 0 only when a write could be made (the GL core asks
before it copies). `stencil_resolve(id, dst, w, h, pitch)` is `depth_resolve`'s
twin for stencil bytes.

On the V3D (`src/v3d/backend.rs`):

- The data: `ZsBuf` (`:520`), one buffer per id, `owners[0]` the depth id,
  `owners[1]` the stencil id, `z`/`s` a `ZsKnown`. Two layouts: D24S8 (4 B a
  pixel, depth in bits 31..8, stencil in 7..0, Mesa's S8_UINT_Z24_UNORM,
  `v3dx_format_table.c:149`) and the separate S8 buffer (`ZsBuf::s8`, 1 B a
  pixel) the split mode keeps (`g.split_zs && g.s8_stencil && !msaa`,
  `zs_keys`, `:2714-2719`). Both are `rt_layout(w, h, 1)` single-level UIF
  without the XOR.
- Read (`stencil_resolve`): the structure of `v3d_depth_resolve` (`:4786`):
  find the buffer whose stencil half `id` owns (`owners[1]`), `Cleared(v)`
  -> fill, `Stored` and not `msaa` -> `v3d_tex_load_box` the level, take the
  low byte of each word (or the S8 bytes), anything else answers 1.
- Write (`ds_write`): `cache_invd` the buffer, load the rectangle with
  `v3d_tex_load_box`, replace the plane's field (depth: round(v * 0xffffff)
  into bits 31..8, keep the stencil byte; stencil: replace the low byte, keep
  the depth bits), put it back with `v3d_tex_store_box` (`src/v3d/ffi.rs:760`),
  `cache_flush`, and set the written half `Stored` so the next job loads it
  (M6 step C, `ZsKnown::Stored` doc at `:466`). A `Cleared(v)` half is
  expanded to the buffer first (a partial write must not lose the rest of the
  clear). A half that is `Untouched`/`Written`, a 4x buffer or no buffer
  answers 1, which the GL core still refuses as today; a destination
  renderbuffer no frame has drawn into is exactly that case.
- Precision: the buffer is 24-bit. D32F reads back as `n / 16777215`
  (`:4829-4830`); a D32F_S8 or D32F blit is a 24-bit round trip. dEQP's
  comparison has a tolerance but it is untested here: the D32F cases are
  the risk of G6 (6 of the 16 rows: `depth_component32f_{basic,scale}`, `depth32f_stencil8_*`).
- Order of work, by rows: depth plane of `ds_write` first (G6a, 8 rows:
  depth-only renderbuffers and the two `depth_only` combined cases); then the
  stencil plane and `stencil_resolve` (G6b, 8 rows).

Tests: the shape of `tests/es3_blit_ds.rs` (the PowerVR's, on the real GL core)
on the V3D rig of `tests/depth_resolve_v3d.rs`/`tests/v3d_depth_storage.rs`
(real GL core, real V3D back end and layout code, faked job builder, a
platform whose "GPU" stores known words): render depth/stencil into a source
renderbuffer, blit, `depth_resolve`/`stencil_resolve` the destination and
compare; a blit into a buffer in each `ZsKnown` state; a 4x buffer. The check
row for a new `tests/es3_*_v3d.rs` file is `tools/check.sh`'s (plan #316's).

Not done by this entry, still `GL_INVALID_OPERATION` as
`docs/notes/blit-depth-window.md` says: the window as either side, a cube or
array depth source, stencil into or out of a texture. None of the 194 rows
needs them.

### 3.5 G7: separate depth and stencil renderbuffers (12 rows)

Allowed by the spec as NotSupported. Making them run means exposing
`GL_EXT_separate_depth_stencil` (a depth renderbuffer and a stencil
renderbuffer completing together), which `check_status` refuses today, and
every back end then holds depth and stencil under two ids (the V3D already
can: `owners: [u32; 2]`, `ZsBuf::s8`). Lowest value of the groups; do it
with, not before, G6.

## 4. Mesa's v3d blit paths against these rows

`v3d_blit.c`, `v3dx_tfu.c` and `v3d_context.c` are not in `third_party/`
(only the format tables are, `docs/notes/v3d-sampled-copies.md:64-71`);
a `MESA_V3D_TREE` is not set here, GitLab refused a fetch (Anubis) and the
GitHub mirror had no such path, so **the statements about the file below that
carry no line number are from memory of Mesa 26.2.1 and unchecked [U]**; the
line numbers are those other notes of this repository already quote.

- `v3d_blit()` tries, in order, the TFU (`v3d_tfu_blit`, `v3d_blit.c:279-320`),
  the TLB (`v3d_tlb_blit`: a multisample resolve or a same-format copy, its
  gate for the resolve is `v3d_format_supports_tlb_resolve_and_blend`, types
  `8` and `16F` only, `v3d_blit.c:382-387` / `v3d_formats.c:148-163`, quoted
  in `docs/notes/v3d-msaa-es3.md`) and finally a render blit through
  `util_blitter` (a draw of a quad that samples the source) [U].
- **TFU**: a scaled or 1:1 copy of colour between two images by the TFU unit.
  On V3D 4.2 the TFU writes UIF or LT, never a raster destination
  (`v3dx_tfu.c:58-62`, `docs/notes/v3d-tfu-raster-chain.md:72-84`); a
  raster *source* is supported at `NUMMM = 0`. yakogl's colour images are
  raster (renderbuffers, the window, texture storage), so the TFU could only
  write a sampled texture's UIF `RtImage`, which the next sample reads; the
  raster storage would still be untiled by the CPU for a read-back. It also
  does not convert between unlike formats, filter anything but its own
  bilinear, flip about the source's y reliably for the GL rectangles, nor
  touch depth/stencil, nor decode sRGB into float formats [U]. It is a
  speed path for a sampled-texture destination, not a way to fix a row.
- **TLB blit**: resolve and same-format copy through a job that loads
  the source and stores the destination. A single-sample, converting,
  scaling or flipping blit is not its case [U]. For yakogl it is the
  multisample resolve of `v3d-msaa-es3.md`, whose rows are `fbo.msaa.*`,
  not `fbo.blit.*`.
- **Render blit**: Mesa's general path, a shader draw with a sampler; it
  converts any pair, decodes sRGB by sampling and encodes by the TLB, writes
  depth through a fragment depth output and stencil bit by bit [U]. For
  yakogl it needs the source as a texture (tiled copy or `RtImage`), a blit
  program per format class in the online compiler, and the sRGB render target
  (G2, G5) to work first. It would replace the CPU codec with the GPU, and the
  CPU codec is correct today.

Which groups a V3D-side blit would fix: **none of G1-G5, G7**. Their cause
is not that the copy is made on the CPU. It would fix G6 only as the
CPU entries of 3.4 (a `ds_write` / `stencil_resolve` in the back end, the
Mesa-independent route the PowerVR already took); a GPU depth/stencil blit
(render blit with depth export, stencil per bit) is far more work for the
same 16 rows. The reasons to build a GPU blit later are speed and nothing in
this tsv: a 1920x1080 CPU copy per frame (a compositor-style use) is slow
(the per-pixel `load`/`store` of `copy_colour`), where the TFU into a sampled
texture's UIF image or a render-pass blit would not be.

## 5. Ranked issue drafts

Posted by the planner (agents never run `gh`). Rows in the order they pay.

1. **V3D: `ds_write` and `stencil_resolve`, so glBlitFramebuffer works between
   depth/stencil renderbuffers (16 `fbo.blit.depth_stencil` rows)**
   `src/v3d/backend.rs:6460-6461` has `ds_write: None, stencil_resolve: None`.
   A blit whose destination is a depth or stencil renderbuffer, or whose
   source is a stencil renderbuffer, is `GL_INVALID_OPERATION` ("logged once")
   (`src/gl/blit.rs`, `REFUSED_DEPTH_DST`, `REFUSED_STENCIL`). Pi 4 FB2 fails
   16 cases for it: `depth_component{16,24,32f}_{basic,scale}`,
   `depth{24_stencil8,32f_stencil8}_{basic,scale,depth_only,stencil_only}`,
   `stencil_index8_{basic,scale}`. Add the two entries to the V3D table, a CPU
   read-modify-write of the `ZsBuf` through `v3d_tex_load_box`/`v3d_tex_store_box`
   (design: `docs/notes/v3d-fbo-blit.md` 3.4), depth first (8 rows), stencil
   next (8). No `src/gl` change. Host tests on the V3D rig. Risk: D32F is a
   24-bit round trip. Only `src/v3d` and tests.

2. **V3D: the 29 Pi 4-only sRGB blit rows (`fbo.blit.conversion.*srgb8_alpha8*`
   23, `fbo.blit.default_framebuffer.srgb8_alpha8_*` 6)**
   The same cases pass on the AM62P with the shared CPU blit codec, so the V3D
   sRGB target (draw, clear, read-back) is wrong or refused. Advances the
   sRGB items (`docs/es3/srgb.md`, v3d-srgb-draw): add a case per FB2 sRGB row
   to the host tests (draw a gradient into an sRGB renderbuffer and read it
   back; clear an sRGB texture to yellow and blit into it) and rerun FB2 on
   the board for the first message of each row (#320). Fix the stale comment
   at `src/v3d/backend.rs:5978` (the table has an sRGB row).

3. **GL core: `GL_EXT_color_buffer_float` (RG32F, RGBA32F, R11F_G11F_B10F,
   R32F as extension formats) (134 `fbo.blit` NotSupported rows, more in
   `fbo.render`, `fragment_out`)**
   dEQP skips every `fbo.blit.conversion` pair and every
   `default_framebuffer` case with an `r32f`, `rg32f`, `rgba32f` or
   `r11f_g11f_b10f` end (112 + 22) because the extension is not listed. It is
   an all-or-nothing claim: the GL core needs the surfaces and read-back for
   the three new names, blit codec arms, the draw's output class; the V3D
   renders all four in hardware (`v3dx_format_table.c:91,96,101,146`; 32F
   RTs are 64/128 bpp jobs with smaller tiles and soft blending). Needs
   `src/gl` and `src/shaderc` (not this plan's) and a per-back-end render
   claim. Draft only; depends on the per-format matrix.

4. **GL core: `glReadPixels` of an R8 colour image returns the green the
   clear left (3 `default_framebuffer.r8_*_blit_from_default` rows, Pi 4 and
   AM62P)**
   A 1-channel (and 2-channel) fixed-point image is stored as RGBA8 bytes;
   the clear writes all four channels and `read_pixels`
   (`src/gl/framebuffer.rs:2969`) returns the stored bytes where ES gives
   (r, 0, 0, 1). The blit codec masks by `AttachmentBits`; the read-back does
   not. Mask in the read-back (or the clear). Host test: clear R8 with
   (1, 1, 0, 1), read RGBA, expect (255, 0, 0, 255). Touches `src/gl`.
   [inferred, unmeasured.]

5. **GL core: `GL_EXT_separate_depth_stencil` (12
   `fbo.blit.depth_stencil.*_stencil_index8_*` NotSupported rows)**
   A depth renderbuffer and a stencil renderbuffer attached separately are
   incomplete; dEQP then skips. Optional (NotSupported is a valid answer);
   do it after issue 1. Touches `src/gl` (`check_status`, the extension string).

6. **Docs: `docs/es3-cts-failures.md` RC8 says "`fbo.blit.conversion` passes"
   and "the 16 `fbo.blit.depth_stencil` failures ... are for a board run to
   re-measure"**
   Both are AM62P statements. On the Pi 4 (FB2, `925dbca9`) `conversion` has
   112 NotSupported + 23 Fail, and the depth_stencil Fails are explained by
   issue 1. Goes into the docs item (v3d-es3-docs), not a separate change.

Advances: #1261 (Pi 4 GLES3): rows of G1-G7 above, with the finding that
none of the 194 is a fault of the CPU blit's arithmetic, flips, filters or of
the raster/tiled hand-over, and that 16 are fixed by two V3D table entries.
