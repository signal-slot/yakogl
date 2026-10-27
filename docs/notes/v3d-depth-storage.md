<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: deferring the R32F storage of a sampled depth texture

Design, pin, and (section "Done") the change. `docs/notes/v3d-sampled-copies.md` ("Ranked
fix proposals", 2) found that a depth texture whose depth is only ever
sampled from its D24S8 buffer still allocates a full R32F backing store that
no one reads (1600x1200 x 4 B = 7,680,000 bytes; refract's carve-out
high-water on the board is 76.7 MB of about 100 MB). This note names every
allocation site, every reader and writer of that storage, the exact deferral
plan, and the test that covers each. The tests are `tests/v3d_depth_storage.rs`
(built on `tests/depth_resolve_v3d.rs`'s rig: the real GL core, the real V3D
back end, the real layout code, a faked job builder and a platform whose
"GPU" stores a known depth word).

Run: `cargo test --no-default-features --features
host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_depth_storage`.

Line numbers below are of main at `459a570e` (`src/gl` is the same at
`3b3f7ad8`) and drift; the function names are the address.

## Why the storage exists

The GL core keeps every depth texture as R32F texels (`texfmt_of`,
`src/gl/texture.rs`: `GL_DEPTH_COMPONENT*` and `GL_DEPTH*_STENCIL8` map to
`GLES_TEXFMT_R32F`). A draw with the texture as its depth attachment renders
into the back end's own D24S8 buffer, keyed by the texture's depth object id,
not into the texels. The texels are filled only when the CPU needs them
(`texture::resolve_depth`). On the V3D a draw that samples the texture reads
the D24S8 buffer (`depth_for_draw` -> `backend::samples_rendered_depth`,
`GlesTextureBind::depth_id`), so for shadow and refract the texels are never
written and never read; only their allocation remains.

## Allocation sites of the storage (all `osal::gpu_alloc(heap_data(), ..)`)

| site (file: function) | when |
|---|---|
| `src/gl/texture.rs: texture_alloc_laid_out` | the one place 2D storage is allocated; reached from the next row |
| `src/gl/texture.rs: texture_alloc_storage_ex` (via `texture_alloc_storage`) | glTexImage2D level 0 (`tex_image_2d`, both its lay-out branches), glTexStorage2D (`glTexStorage2D`), `relayout_keeping_levels`'s caller, `grow_npot_chain` |
| `src/gl/texture.rs: texture_alloc_storage_3d` | glTexImage3D / glTexStorage3D arrays and 3D textures (`tex_image_3d_level`, `glTexStorage3D`), `relayout_3d_keeping_levels`; not a depth target |
| `src/gl/texture.rs: compressed_tex_image` | compressed formats only; never a depth texture |

glmark2's depth texture is the first row: `glTexImage2D(GL_TEXTURE_2D, 0,
GL_DEPTH_COMPONENT, w, h, 0, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, NULL)` ->
`tex_image_2d` -> `texture_alloc_storage` -> `texture_alloc_storage_ex` ->
`texture_alloc_laid_out`, which also clears the new storage
(`context::clear_storage`, a pass over 7.68 MB).

The back end allocates nothing for the texels. Its own D24S8 buffer
(`src/v3d/backend.rs: zs_buf`, 7,684,096 bytes at 1600x1200) is separate and
stays.

## Every reader and writer of the storage

The table was derived from `grep -rn '\.mem\b' src/gl` (551 hits, most of
them buffers, renderbuffers, EGL surfaces and tests), plus `mem\.impl_` and
`mem\.cpu\.is_null` over the same tree, every non-test hit of a texture's
`mem` given a row or a reason. There are two classes, and the second is the
dangerous one:

* **bytes**: the code reads or writes the texels through `mem.cpu` / `mem.gpu`
  / `mem.size`. After the deferral `mem` is empty, so these must call
  `ensure_storage` first (or must never see a deferred texture);
* **"has an image" gates**: `mem.impl_.is_null()` / `mem.cpu.is_null()` used
  to ask "was a level ever specified?". After the deferral a deferred texture
  answers "no" to it, and the code takes its *no image* branch: a draw binds
  a black 1x1, a copy is skipped, a re-specification re-lays out. No crash,
  a wrong picture. Every one of these becomes `GlTexture::has_image()`
  (`!mem.impl_.is_null() || storage_deferred`).

"covered by" names the test of `tests/v3d_depth_storage.rs` that runs the
path today, with the property that must hold after the change; "reason" says
why none can.

### Allocation and release

| # | site (file: function) | what | covered by / reason |
|---|---|---|---|
| 1 | `texture.rs: texture_alloc_laid_out` | allocate + `clear_storage` (writer) | `shadow_like_carve_out_high_water`, `refract_like_carve_out_high_water` (the 7,680,000 bytes) |
| 2 | `texture.rs: texture_alloc_storage_3d`, `compressed_tex_image` | other allocators | reason: arrays, 3D and block formats; no depth texture is rendered into as one (`attachment_storage` keeps no layer for a depth texture, `blit.rs: depth_texels` wants `GL_TEXTURE_2D`); they keep allocating at once |
| 3 | `texture.rs: free_texture`, `glDeleteTextures`; `context.rs: release_storage_with` | free | every test's `teardown`; the carve-out tests start from `mark_peak` after a warm-up. `release_storage_with` skips a null `impl_`, so a deferred texture frees nothing |
| 4 | `texture.rs: texture_attach_external` (EGLImage); `egl.rs: image_of_texture` (2644) | replace `mem` with an image's / make an image of the texture's `mem` | reason: `image_of_texture` refuses any `texfmt` but RGBA8/RGB565, so a depth texture is never made an image; `texture_attach_external` onto a deferred texture releases an empty `mem` (row 3) and attaches colour |
| 5 | `context.rs: yakogl_pool_owners` tally (949, `texs.add(&t.mem)`) | counts bytes | reason: report only; its texture bytes fall by 7,680,000 for a deferred texture, which is the expected change, no test |

### Bytes: readers and writers

| # | site | what | covered by / reason |
|---|---|---|---|
| 6 | `texture.rs: resolve_depth` (715-718) | `mem.size` check, then the resolve writes `mem.cpu + level_off` | `rendered_then_read_on_the_cpu_resolves_into_the_storage`; `ensure_storage` goes after the "was it rendered" tests and before the size check |
| 7 | `texture.rs: depth_texture_rendered` (740) | gate of 6 and of `depth_for_draw`: `!mem.cpu.is_null()` | every test; becomes `has_image()` |
| 8 | `texture.rs: depth_for_draw` | sampled-from-buffer or resolve; reads size and `levels` only | `rendered_then_sampled_compares_like_the_stored_depth`; every `return 0` ends in `ensure_storage` (plan 2) |
| 9 | `texture.rs: settle_texture` (callers: `tex_image_2d`, `glTexSubImage2D`, `glCopyTexImage2D`, `glCopyTexSubImage2D`, `compressed_tex_sub_image`, `glGenerateMipmap`, `generate_mipmap_3d`, `glTexImage3D`, `tex_sub_image_3d`, `glCopyTexSubImage3D`) | `resolve_depth`, then `stamp_texels` | `rendered_then_read_on_the_cpu_resolves_into_the_storage` (glTexSubImage2D), `generate_mipmap_is_refused_and_copy_tex_sub_image_settles_a_depth_texture` (glCopyTexSubImage2D); `a_level_above_zero_grows_the_chain_and_resolves_the_rendered_depth` (tex_image_2d); the 3D and compressed callers: reason, row 2 |
| 10 | `texture.rs: texture_write_rect` (1916-2070), `texture_write_blocks`, `texture_write_etc1` | CPU rows into `mem.cpu`, or `gpu_write` | `uploaded_then_sampled_reads_the_uploaded_texels` (glmark2-style NULL then data of the same size) and `rendered_then_read_on_the_cpu_resolves_into_the_storage` (glTexSubImage2D); blocks and ETC1: reason, row 2 |
| 11 | `texture.rs: texels_writable` (938) | `!mem.impl_.is_null()` plus the format list | same two tests; becomes `has_image()` (it must say yes for a deferred depth texture, so that `ensure_storage` can run) |
| 12 | `texture.rs: texture_copy_from_fb` (2174; glCopyTexSubImage2D) | CPU copy of the read buffer into the texels | `generate_mipmap_is_refused_and_copy_tex_sub_image_settles_a_depth_texture`: accepted for a depth texture, settles (one resolve), the rest of the texels keeps the rendered depth; the copied values are the canvas's colour and are not pinned |
| 13 | `texture.rs: relayout_keeping_levels` (1100-1169), `grow_npot_chain` (1066) | copy existing level 0 into a new, bigger lay-out | `a_level_above_zero_grows_the_chain_and_resolves_the_rendered_depth`: on the V3D (`mip_level_align` 0) a level above 0 given to a non-power-of-two depth texture grows the chain, keeps level 0's rendered depth, and the next frame resolves instead of sampling the buffer. `ensure_storage` must run before the copy (`grow_npot_chain` returns false when `mem.cpu` is null and would silently skip the growth) |
| 14 | `texture.rs: take_pe_order` (876-914) | byte swap of RGBA8 texels | reason: `texfmt != GLES_TEXFMT_RGBA8` and the explicit depth-format test at 877 return first |
| 15 | `texture.rs: glGenerateMipmap` (3766 gate, 3806 `filtered`) | needs an image | `generate_mipmap_is_refused_...`: a depth texture is `GL_INVALID_OPERATION` at 3793, before it looks at the storage; the gate at 3766 gives the same error for "no image" and "deferred", so nothing moves |
| 16 | `texture.rs: relayout_3d_keeping_levels` (1316-1396), `tex_image_3d_level` (4567-4628), `tex_sub_image_3d` (4822), `generate_mipmap_3d` (3903-4080), `compressed_tex_image` (3315-3346), `compressed_tex_sub_image` (3633), `texture_write_etc1`/`etc1_image_decoded` (3395-3471) | arrays, 3D, block formats | reason: row 2; they do not defer, and a depth texture of these targets is never rendered into |
| 17 | `blit.rs: depth_texels` (334-344), `copy_depth` (851, 871) | glBlitFramebuffer depth: `resolve_depth` on both sides, then a float copy between the two storages | `rendered_then_read_on_the_cpu_resolves_into_the_storage` (texture as read side, as draw side, partial blit keeps the rendered rest). `depth_texels` tests `mem.cpu.is_null()` and `mem.size` before it resolves: `ensure_storage` first, in `depth_texels` |
| 18 | `tf_capture.rs: settle_for_read` (1321) and its caller (1187, 1196, 1219, 1249) | a vertex-shader sampler: `resolve_depth`, then the texels are handed to the capture | reason: the rig has no transform-feedback shader, and glmark2 has none. The gate at 1187 (`mem.impl_.is_null()` -> `Bound::NONE`, "no image") runs *before* `settle_for_read`, so a deferred rendered texture would read as (0, 0, 0, 1): it becomes `has_image()`, and `ensure_storage` goes between `resolve_depth` and the `cache_invd`/`bound_of` at 1196-1219 |

### "Has an image" gates

| # | site | what | covered by / reason |
|---|---|---|---|
| 19 | `draw.rs: build_common` (3788) `if t.mem.impl_.is_null() { continue; }` | no texture: the back end binds a black 1x1. Runs *before* `tb.depth_id` is set (3841) | `rendered_then_sampled_compares_like_the_stored_depth`: `draw.len() == 1` and the DEPTH24_X8 record would fail at once. This is the gate that matters for glmark2 shadow: it becomes `has_image()` |
| 20 | `draw.rs: build_common` (3835, 3842: `tb.base_gpu`, `tb.cpu`) | the texels' address for the back end | `uploaded_then_sampled_reads_the_uploaded_texels` (R32F record, tiled copy equal to the texels). A texture sampled from the buffer carries `depth_id` and the back end does not read these; one that falls back to texels has had `ensure_storage` in `depth_for_draw` |
| 21 | `draw.rs: texture_bind_2d` (673-682) | `base_gpu`/`cpu` of the bind for `texture_uploaded`, `generate_mipmap`, `chain_back` | reason: its callers are `note_uploaded` (RGBA/UNSIGNED_BYTE only), `glGenerateMipmap` (refused for depth), `chain_back` (needs `chain_in_backend`, only after a back end mipmap); a depth texture reaches none. A draw does *not* bind through it (the earlier draft of this note said so: wrong) |
| 22 | `draw.rs: dump_cube_atlas` (2923-2994), `dump_bound_texture` (3056-3114), `debug_dump_cubes` | debug logging reads `mem.cpu` / `mem.size` | reason: `dump_cube_atlas` is RGBA16F cubes; `dump_bound_texture` returns on a null `mem.cpu` (3057), so a deferred texture is not dumped |
| 23 | `framebuffer.rs: attachment_storage` (408-419, depth-texture branch) | `!mem.impl_.is_null()` and `storage_holds(&t.mem, offset, lw, lh, ..)`; its callers (224, 663-714, 814, 914, 1512, `blit.rs` 157/292) ask for size, format and completeness | every test (`glCheckFramebufferStatus` in `setup`); `respecifying_a_depth_texture_keeps_it_a_working_target_or_makes_it_colour` (complete after a same-size re-specification). The gate becomes `has_image()`, the size check `t.storage_bytes` against the level. The colour-attachment branch (430, 482) is untouched: not a depth format |
| 24 | `framebuffer.rs: texture_bits` (1878) | `mem.impl_.is_null() && width == 0` | reason: needs both; a deferred texture has a width |
| 25 | `native_cube.rs` (145-288, 513) | PowerVR native cube copies of RGBA16F cubes | reason: PowerVR only, cubes only |
| 26 | `texture.rs: tex_image_2d` (2455 `differs`, 2488 relayout/alloc choice, 2539 and 2561 level > 0 before level 0, 2655 "storage allocated, nothing to write") | decide keep, re-lay-out or write | `respecifying_a_depth_texture_keeps_it_a_working_target_or_makes_it_colour` (same-size NULL again: still a target, still sampled from the buffer), `uploaded_then_sampled_reads_the_uploaded_texels` (data after NULL: the keep branch, then 10/11), `a_level_above_zero_...` (the grow branch). All become `has_image()`; 2655 must call `ensure_storage` when `pixels` is not NULL |
| 27 | `texture.rs: glCopyTexImage2D` (2936) | returns when there is no allocation | `respecifying_a_depth_texture_keeps_it_a_working_target_or_makes_it_colour`: a depth internal format is refused before `tex_image_2d` (2869-2882), and a colour one re-lays the texture out as colour with storage of its own, so the gate never sees a deferred texture; `has_image()` keeps it that way |
| 28 | `texture.rs: compressed_tex_image` (3315, 3331), `glCompressedTexSubImage2D` path (3395, 3427, 3471, 3633), `tex_image_3d_level`, `tex_sub_image_3d` (4822) | the same gate for block formats and 3D | reason: row 2/16 |

Not a reader: `glClear`, `glInvalidateFramebuffer` (they act on the back end's
D24S8 buffer), `glReadPixels` (colour only; no depth read in the build), the
shader compiler, `note_level_written`, `stamp_texels`, `levels_written`
(bookkeeping, no storage access; `stamp_texels` moves with the resolve in
`rendered_then_read_on_the_cpu_resolves_into_the_storage`).

The v3d back end: `src/v3d/backend.rs: v3d_depth_resolve` reads its D24S8
buffer and writes the caller's `dst` floats, allocating nothing
(`rendered_then_read_...`, plus `tests/depth_resolve_v3d.rs`);
`v3d_depth_sampled` / `depth_sample` answer whether a draw may sample the
buffer and touch no texel (`rendered_then_sampled_...`); `copy_image_for` /
`rt_sample` / `v3d_texture_state` read `base_gpu`/`cpu` only when `depth_id`
is 0 (`uploaded_then_sampled_...`).

## The deferral plan

Rule: a depth texture that `tex_image_2d` or `glTexStorage2D` lays out with no
pixels (the glmark2 call) keeps its layout but not its allocation, until
something needs texels. Everything else (3D/array, block formats, any
non-depth texture) keeps allocating at once. The first level 0 of a depth
texture is NULL in glmark2; a NULL spec followed by data (the
`uploaded_then_sampled_...` sequence) and a level above 0 are *later* events
on a deferred texture and are handled by `ensure_storage`, not by the rule.

1. **Record the layout, skip the allocation.** In `texture_alloc_laid_out`
   (the single allocation site), when a new flag `defer_storage` is set by
   the two callers above (depth `format`, `texfmt == R32F`, 2D, one level,
   `pixels == NULL`), compute and store `level_off`, `chain_pitch` and the
   byte size (`t.storage_bytes`) as now, release any old storage, leave
   `t.mem` empty, set `t.storage_deferred = true`, and skip
   `clear_storage`. Nothing else in `texture_alloc_laid_out` moves.
2. **One function allocates it**: `ensure_storage(t)` does what the skipped
   half of `texture_alloc_laid_out` did (`gpu_alloc`, `clear_storage`, clear
   `storage_deferred`, a failed `gpu_alloc` -> `GL_OUT_OF_MEMORY` at the
   caller). It runs
   * in `resolve_depth`, after the "was it rendered" tests and before the
     `mem.size` check (rows 6, 9, 17, 18 reach it through `settle_texture`,
     `copy_depth`, `settle_for_read`), and in `blit.rs: depth_texels` /
     `copy_depth` for a texture that was never rendered (row 17);
   * in `settle_texture`, so every CPU write (rows 10, 12) finds storage, and
     at the top of `tex_image_2d` and `glTexSubImage2D` when `pixels` is not
     NULL (row 26);
   * before `grow_npot_chain` and `relayout_keeping_levels` copy level 0
     (row 13): without it `grow_npot_chain` returns false on a null
     `mem.cpu` and the level above 0 is written into storage that is too
     small;
   * in `depth_for_draw` on every path that returns 0 (the texture was never
     rendered, the back end refused, a chain or a level above 0): the draw
     then falls back to the texels (rows 19, 20), which a deferred texture
     must have. A deferred texture that was never rendered samples cleared
     texels (0.0), exactly what it sampled before.
3. **Replace every "has an image" test by `GlTexture::has_image()`** (rows 7,
   11, 18, 19, 23, 26, 27): `!mem.impl_.is_null() || storage_deferred`. The
   two that matter most are `draw.rs: build_common` (19: a deferred, rendered
   texture would otherwise sample black, which only the
   `rendered_then_sampled_...` test sees) and `framebuffer.rs:
   attachment_storage` (23: the framebuffer would be incomplete);
   `attachment_storage` checks `t.storage_bytes` against the level instead
   of `storage_holds(&t.mem, ..)`. `Storage.mem` for a deferred depth
   texture must not be dereferenced by its callers (`framebuffer.rs` lines
   using `st.mem?` are colour paths).
4. **Keep the 2D/one-level/no-pixels rule narrow.** A cube face or a layer
   never defers (`depth_for_draw` already refuses them), so they keep
   today's behaviour with no new path. A level above 0 is not refused: the
   texture grows its chain (row 13) and from then on resolves.
5. **A deferred texture that is rendered then CPU-read** allocates inside
   `resolve_depth` (a `clear_storage` of 7.68 MB once, then the resolve's own
   write): one more 7.68 MB pass than today for the programs that do read,
   none for those that do not. glmark2 shadow and refract never do.

Expected after the change (numbers from the tests, in bytes, host rig):
shadow-like high-water 15,390,016 -> 7,710,016 and refract-like 23,518,364 ->
15,838,364 (both minus exactly 7,680,000; the rest is the D24S8 buffer of
7,684,096 plus arenas and, for refract, the colour texture and its chain). The
test constants `SHADOW_PEAK` and `REFRACT_PEAK` change by that and by no
more; if they drop by anything else a reader has been missed or moved.

## Done

The plan above is in the commit "v3d: a sampled depth texture's R32F storage
is allocated only when something reads it" (on main at 552ac8e4 plus it).
`GlTexture::storage_deferred` (in what was padding: `gl_texture` stays 248
bytes, `src/abi_layout*` pin its offset) marks a 2D depth texture laid out by
glTexImage2D(level 0, NULL) or glTexStorage2D(1 level) without storage, only
where `backend::defers_depth_storage()` says so (the V3D while initialised;
the PowerVR, GC7000 and Mali allocate as before). `texture::ensure_storage`
allocates and clears it at plan step 2's points (`resolve_depth`,
`settle_texture`, `tex_image_2d` with pixels, `depth_for_draw` on every path
that returns 0, `blit.rs` before `depth_texels`, `tf_capture.rs`
`settle_for_read`), with two backstops the plan did not name:
`texture_write_rect` (every CPU texel write) and `draw.rs: build_common` (a
deferred texture bound without a `depth_id`). The gates of step 3 ask
`GlTexture::has_image()`; `attachment_storage` skips the size check for a
deferred store. The byte count is not stored: `ensure_storage` lays the
one-level layout out again.

The tests changed where the plan said they would, and no more:
`SHADOW_PEAK` and `REFRACT_PEAK` fell by exactly `STORAGE`; the shadow-like
test's "glTexImage2D allocated it" became "did not"; and
`a_level_above_zero_...` finds no storage after the sampled frame and the
grown chain's after it. New: `rendered_then_sampled_...` and both carve-out
tests check the storage is still deferred after their frames, and
`never_rendered_then_sampled_allocates_cleared_texels` covers
`depth_for_draw`'s never-rendered path (allocated for the draw, texels 0.0).

Host before/after (the test rig, deterministic, bytes at the high-water;
`--test-threads=1` and default threads agree):

| sequence | before | after | difference |
|---|---|---|---|
| shadow-like (1600x1200 depth, 3 frames) | 15,390,016 | 7,710,016 | -7,680,000 |
| refract-like (+ 800x600 RGBA8 chain) | 23,518,364 | 15,838,364 | -7,680,000 |
| allocated by glTexImage2D(NULL) | 7,680,000 | 0 | -7,680,000 |

The ms a frame of glmark2 shadow and refract on the V3D host build
(`tools/hostharness/glmark2/run.sh`, `GM2_PROFILE=1`) was not measured with
this change: the harness could not be run where it was made. What the
change removes from the host's time is one 7.68 MB clear at the
glTexImage2D, in the scene's setup, not in a counted frame; the per-frame
path is untouched (the draw already sampled the D24S8 buffer), so no
per-frame difference is expected, and a harness run should show the two
scenes within the noise floor of `FINDINGS-variance.md`. The board's
carve-out line (refract 76.7 -> about 69 MB) still needs a board session.

## What the tests pin

| test | pins |
|---|---|
| `rendered_then_sampled_compares_like_the_stored_depth` | a rendered depth texture is sampled from the buffer (DEPTH24_X8, one level, no resolve, no tiled copy, exactly one texture bound in the ground's draw), and the shadow compare against a reference below, at and above a stored depth gives the stored depth's result, for every texel of an 80x60 target. Must pass unchanged after the fix: it is the test of rows 8, 19, 20 |
| `uploaded_then_sampled_reads_the_uploaded_texels` | a NULL spec then glTexImage2D with data of the same size (GL_UNSIGNED_INT) -> one tiled copy with an R32F record whose texels equal the upload's floats; no buffer, no resolve. After the fix the data call allocates (rows 10, 11, 26) |
| `rendered_then_read_on_the_cpu_resolves_into_the_storage` | glTexSubImage2D after a render resolves once and keeps the rendered rest; glBlitFramebuffer depth with the texture as source and as destination resolves and copies the rendered depth. After the fix these are the paths that allocate on first use (rows 6, 9, 10, 17) |
| `generate_mipmap_is_refused_and_copy_tex_sub_image_settles_a_depth_texture` | glGenerateMipmap on a depth texture is GL_INVALID_OPERATION and resolves nothing; glCopyTexSubImage2D into one settles it (one resolve) and keeps the rest (rows 12, 15) |
| `respecifying_a_depth_texture_keeps_it_a_working_target_or_makes_it_colour` | glTexImage2D(NULL) of the same size again leaves the framebuffer complete and the next frame sampled from the buffer; glCopyTexImage2D with a depth internal format is an error and resolves nothing; with GL_RGBA it makes the texture a 4x4 RGBA8 one with storage (rows 23, 26, 27) |
| `a_level_above_zero_grows_the_chain_and_resolves_the_rendered_depth` | glTexImage2D(level 1, NULL) on a rendered 1600x1200 depth texture grows the storage, keeps level 0's rendered depth, gives two levels, and the next draw resolves instead of sampling the buffer (rows 9, 13, 26) |
| `shadow_like_carve_out_high_water`, `refract_like_carve_out_high_water` | the bytes the platform allocated at their high-water for glmark2's shadow sequence (a 1600x1200 depth texture, three frames) and a refract-like one (the same plus an 800x600 RGBA8 texture rendered, mip-mapped and sampled each frame): 15,390,016 and 23,518,364, with the R32F storage 7,680,000 of it (rows 1, 3) |

The carve-out tests count `gpu_alloc` bytes in the test platform, after a
warm-up that makes the permanent arenas, so the numbers do not depend on test
order (checked with `--test-threads=1` and the default threads). They count
this rig's allocations, not the board's: the board's refract 76.7 MB also has
the colour chain, the per-job arenas and the tiled copies; what carries over
is the 7.68 MB difference.

Rows with a reason and no test: 2, 4, 5, 14, 16, 21, 22, 24, 25, 28 (a
format or target a depth texture never has, or a report) and 18 (a
transform-feedback shader the rig cannot compile; the code path is row 6).

## Risks the tests do not close

* The window system's own depth buffers are not textures and are not touched.
* A path that tests a texture's `mem` and is not in the table is the failure
  to fear, and its failure is *quiet*: a "has an image" gate (rows 19, 23,
  26, 27 are the ones found) answers "no" for a deferred texture and the
  code takes its no-image branch (black 1x1, skipped copy, incomplete
  framebuffer), and a bytes reader reads a null pointer. Before the change
  re-run `grep -rn 'mem\.impl_\|mem\.cpu\.is_null\|\.mem\.size\|\.mem\.gpu'
  src/gl`, and check each non-test hit against the table above, line by
  line. The tests catch rows 19, 23, 26 and 27 and the bytes readers 6, 10,
  12, 13 and 17; row 18 they cannot.
* The board's memory line (refract 76.7 -> ~69 MB) needs a board session; the
  host shows the same difference through these tests.
* A gate row for `tests/v3d_depth_storage.rs` is the item gate-v3d-new-rows;
  until it lands only `tools/check.sh`'s scope rule (`tests/v3d_*` reaches the
  `v3d` rows) runs it through `--changed`.
