<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the lazy raster store of a sampled render target, and every CPU reader it must serve

Design and test map for "Ranked fix proposals" 1 of `v3d-sampled-copies.md`
(item v3d-lazy-raster-guard), read at main `b9130d64`. The change is made
(item v3d-lazy-raster); "As built" at the end says how it differs from the
plan below.

## What changes

Today a job rendering into a sampled texture (`rt_texture`: stamped 2D RGBA8
at the GL core's pitch, not an EGLImage's, not bgr565/MSAA/RT 1..3) stores
RT 0 twice: into the GL core's raster storage (`V3dJobTarget::address`,
`stride`) and into the render image (`RtImage`, `uif_address`; built in
`v3d_job_target`/`emit_tile_list`). The GPU samples the image
(`rt_sample`, TFU chain); the raster storage is there for the CPU only.

Lazy raster: while `RtImage::stored` and no CPU path has asked, the job
stores only the image. The image is the live copy; the storage is stale.
Whoever needs the storage detiles the image into it first, as
`rt_chain_back` (`v3d_return_mipmap_chain`) already does for levels 1..
(`v3d_tex_load_box` per level into `gl_level_layered` rows). The new case is
level 0, and one hazard the chain case does not have: **a later job that
loads the target** (a render that does not clear and does not invalidate)
loads the previous colour from the raster `address` through the TLB, so a
job on a target whose storage is stale must load from the image or the
storage must be made current before such a job is recorded (the first is
free of CPU cost; the job builder, `driver/src/v3d/v3d_job.c`, loads and
stores RT 0 with one address pair today). `reattached_texture_rendered_on_half`
is the host model of that case.

## Where the storage must be made current

One entry point on the back end, `backend::make_storage_current(b)` (name
proposed): if the binding's `RtImage` is `stored` and the storage is stale,
`v3d_tex_load_box` level 0 into the storage rows (`texture::gl_level_layered`
level 0), `cache_flush`, mark the storage current; level 0 only, the chain
stays `rt_chain_back`'s. It waits for the job in flight that stored the image
(`wait_in_flight`) as `copy_image_for` does for `target_kick`. Called:

1. from the GL core's `settle_texture` (`src/gl/texture.rs`), next to the
   existing `chain_back(c, ti)`: that function is already "every CPU write of
   the texels comes through here first". It covers readers 4, 5, 6 (as writer
   of level 0) and 7 below;
2. from `framebuffer::read_pixels_sync`/`gpu_idle_for_cpu`
   (`src/gl/framebuffer.rs`) when the read framebuffer's colour attachment is
   a texture: covers 1, 2, 3 (the source side of glCopyTex*) and 8 (blit
   source, `blit.rs` `attachment_image`/`read_image`), which do not call
   `settle_texture` on the source texture;
3. from `v3d_generate_mipmap`'s refusal branches and from the GL core's CPU
   filter in `glGenerateMipmap` (`texture.rs`, level 0 is the filter's
   source): reader 9;
4. from `copy_image_for` (`backend.rs`) for a target `rt_eligible` refuses
   (its tiled copy is made from storage): reader 10;
5. wherever the image is dropped with its storage still wanted:
   `rt_free_image` (`keep_chain`) must settle level 0 before
   `gpu_free`, as it does for the chain; and `rt_for_render` for "another
   image whose storage the render overlaps" (a second view, a level above 0
   attached) must make that storage current before the new job writes it.

The storage-current bit is per `RtImage`, cleared by every job that stores
the image without the raster store, set by `make_storage_current`, and
treated as "current" for a texture with no `RtImage`.

## The CPU readers of level 0

"Reader" is any code that reads level-0 texels of a texture that can be a
render target through `mem.cpu`, or hands that memory to the GPU as a source.
Found by `grep -n 'mem\.cpu' src/gl/*.rs` and the `cpu_of`/`texture::gl_bytes`
users in `src/v3d/backend.rs`, then each followed to its caller.

| # | Reader (file: function) | Reads | Must be current at | Test (`tests/v3d_lazy_raster_readers.rs`) |
|---|---|---|---|---|
| 1 | `src/gl/framebuffer.rs`: `glReadPixels` -> `read_pixels` (`target_loc`, texture attachment) | the FBO's level | `read_pixels_sync` (path 2) | `read_pixels_from_the_fbo` |
| 2 | same, after another render of the target | stale-image case | same | `read_pixels_after_a_second_render` |
| 3 | `src/gl/texture.rs`: `glCopyTexImage2D`, `glCopyTexSubImage2D` (source: `framebuffer::read_pixels`; the destination goes through `settle_texture`) | the FBO's level | path 2 | `copy_tex_image_2d_from_the_target`, `copy_tex_sub_image_2d_from_the_target` |
| 4 | `texture.rs`: `glTexSubImage2D` (partial write keeps the rest of level 0) -> `settle_texture`; `backend::v3d_texture_uploaded` (writes/tiles the rows it is given) | the texels around the box | `settle_texture` (path 1) | `partial_tex_sub_image_2d_then_sample_then_read` |
| 5 | `texture.rs`: `tex_image_2d` (re-specifying over a rendered texture), `compressed_tex_sub_image`, `etc1_image_decoded`, `glTexImage3D`/`tex_sub_image_3d`/`glCopyTexSubImage3D` (3D/array/compressed: never a target, listed for the settle call they share) | storage the call replaces or keeps | `settle_texture` | none: no render target is a compressed, 3D or array texture (`rt_texture`: `layers == 1`, RGBA8); `tex_image_2d` re-specification replaces the storage and `context.rs` frees the old one through `backend::texture_released` (`v3d_texture_released`), which drops its image; see `src/v3d/backend_tests.rs`, "render images" |
| 6 | `texture.rs`: `glGenerateMipmap`'s own CPU filter (YAKOGL_V3D_TFU unset, or `v3d_generate_mipmap` answers 0); reads level 0, writes 1.. | level 0 whole | path 3 | `cpu_filtered_mipmap_then_read_level_1` (TFU off); with the TFU on, `v3d_generate_mipmap` reads the image, not the storage, and `tests/mipmap_offer_guard.rs` pins the chain |
| 7 | the texture attached to a second FBO and rendered into only in part (`rt_for_render` overlap rule, the job's load of the previous colour) | level 0 under the render | path 5 and the job load | `reattached_texture_rendered_on_half` |
| 8 | `src/gl/blit.rs`: `glBlitFramebuffer` (`read_image`, `attachment_image`, `load`); `copy_depth` reads the depth side (`resolve_depth`, not level 0 colour) | read FBO's level | path 2 | `blit_framebuffer_from_the_target` |
| 9 | `src/v3d/backend.rs`: `v3d_generate_mipmap` refusal branches (no image, stamp moved, fewer levels, TFU failed) -> the GL core's CPU filter of reader 6 | storage | path 3 | `cpu_filtered_mipmap_then_read_level_1`, and `tests/mipmap_offer_guard.rs` (`a_failing_tfu_leaves_the_chain_to_the_cpu`) |
| 10 | `backend.rs`: `copy_image_for` -> `tiled_copy` (CPU or TFU tiling from `cpu_of(b.base_gpu, gl_bytes)`): the first sample after a render, a moved stamp, a full `RT_IMAGES` table, and every `rt_eligible`-refused target | storage whole | path 4 | none for the refused targets: the sample draws here (`sample_it`) take the render image, not the tiled copy. **Gap:** a refused target (a full `RT_IMAGES` table of 8, an unsupported format) needs nine live targets or a non-RGBA8 one; the existing "render images" tests in `src/v3d/backend_tests.rs` cover `copy_image_for` after a render, and the lazy change must add its storage-current call there with a test of its own |
| 11 | `src/gl/tf_capture.rs` (`TexSource`, `cache_invd(t.mem.cpu)`, line 1196-1249): the CPU vertex shader of transform feedback fetching a texture | any level | `settle_texture`-equivalent before the capture | none: needs a transform-feedback program through the stand-in back end; `tf_capture_tests.rs` covers the fetch on plain textures. A rendered-into texture sampled by a TF vertex shader is a gap; the lazy change should call `make_storage_current` at `tf_capture.rs:1333` (`texture_target_use`), next to the existing kick wait |
| 12 | `src/gl/native_cube.rs`, `draw.rs` `dump_cube_atlas` and the attribute dumps (`attrib_dump_enabled`) | cube faces, debug logs | not a target (`layers == 6`, `native_cube`: `t.layers == 6`) | none: cube maps are never `rt_texture` (`layers == 1`) so no lazy raster applies; the debug dumps are log-only and may read stale texels |
| 13 | EGLImage sharing (`egl.rs` `eglCreateImage`, `t.external`) | the shared storage | never lazy | none: `rt_texture` requires `content_gen != 0` and `bgra == 0`; an EGLImage's texture has no `RtImage` (`rt_eligible` false) and keeps the raster store |

Paths 1..5 are the "Where the storage must be made current" list above.

### What does not read level 0 of a target, to be left alone

- The GPU's own samples: `rt_sample`, the TFU chain job and the job's
  store/load use the image (and, until the job load is moved, the storage).
- `depth_sample`/`resolve_depth`/`v3d_depth_resolve`: depth textures, item
  v3d-depth-storage-guard.
- Levels 1..: `rt_chain_back` on `chain_back(c, ti)` (existing).

## How the window system's own buffers stay out

Lazy raster applies to an `RtImage` slot, and a slot is created only
by `rt_for_render` for a texture the GL core registered (`rt_texture`:
`content_gen != 0`, `type_ == 2D`, RGBA8, `layers == 1`, `bgra == 0`).
The window system's surfaces are the context's `MemLoc::DefaultSurface`
(`framebuffer::target_loc` returns that arm when framebuffer 0 is bound):
no texture, no stamp, no `RtImage`, so their jobs keep the one store
and `eglSwapBuffers`/`v3d_surface_swapped` read the raster surface as before.
A pbuffer or pixmap EGLImage bound as a texture has `bgra`/no stamp and
`rt_eligible` is false. The change must keep that test the only way into the
lazy store: the job builder drops the raster store only when `uif_address`
is non-zero, which only `rt_for_render` sets.

## Test map

Every test runs with the TFU off and on except the CPU-filter one. They use
glReadPixels of an FBO, never `GlTexture::mem`, to read the result, so they
stay valid when the storage is stale on purpose. All pass at `b9130d64`
(the storage is always current today).

| Test | Reader |
|---|---|
| `read_pixels_from_the_fbo` | 1 |
| `read_pixels_after_a_second_render` | 2 |
| `copy_tex_image_2d_from_the_target` | 3 |
| `copy_tex_sub_image_2d_from_the_target` | 3 |
| `partial_tex_sub_image_2d_then_sample_then_read` | 4 |
| `cpu_filtered_mipmap_then_read_level_1` | 6, 9 |
| `reattached_texture_rendered_on_half` | 7 |
| `blit_framebuffer_from_the_target` | 8 |

Rows 5, 10, 11, 12, 13 have no test in this file for the reasons in the table.

The stand-in GPU of the test stores the current picture into the raster
storage and the image, or into the left half of both (`HALF`). When the
lazy change lands, the stand-in's raster store must follow the job's: it
writes the raster storage only when `t.address` is stored (the job target
drops it), and a test that fails then names the reader that read stale
texels. A one-line check for the change itself:
`tools/v3d/job_check.c` part 20 counts the stores in Mesa's CLE decoder; the
second RT 0 store must be absent for a target with no CPU reader.

## Order of the change

1. `RtImage` storage-current bit and `make_storage_current`; every caller of
   the list above, still with the raster store on (tests pass: nothing stale).
2. Job loads from the image for a target with a stale storage (or settle
   before a loading job); `reattached_texture_rendered_on_half` must pass.
3. Drop the raster store (the stand-in follows); every test above passes.
4. Gate row for the file in `tools/check.sh` (the item gate-v3d-new-rows) and
   a board check (GPU time of refract's swap column, `end waited`).

## As built

Steps 1 to 3 are made; step 4 is the items gate-v3d-new-rows and the Pi 4
board session.

- **Job builder** (`driver/src/v3d/v3d_job.h`, `v3d_job.c`): two flags in
  the padding after `typed` (no field moves, the struct stays 100 bytes):
  `uif_only` drops the raster store of RT 0, `uif_load` loads RT 0 from the
  image (its memory format, RGBA8) instead of the raster target.
  `v3d_job_begin` refuses either without `uif_address`. The window's jobs
  never have an image, so they keep their one raster store.
- **Back end** (`src/v3d/backend.rs`): `RtImage::raster_stale` is the
  storage-current bit (set by a submitted `uif_only` job, cleared by
  `rt_level0_back`, the `make_storage_current` above). A job sets `uif_load`
  while the bit is set: the image is then the only live copy, and every CPU
  access has gone through `rt_level0_back` first, so nothing newer is in the
  storage. Callers: `rt_free_image` with `keep_chain` (relayout, LRU
  eviction), `rt_for_render`'s overlap loop, `copy_image_for` (reader 10),
  and the GL core's entry `v3d_storage_current` (submits an open job on the
  target first). An image freed while the open job stores into it is taken
  out of that job (`uif_*` cleared in `g.j.job.t`; `v3d_job.c` emits the
  tile list from `j->t` at `v3d_job_end`).
- **One write-back per target** (`RtImage::raster_wanted`): once a CPU
  reader has needed a target's storage, its later jobs store the raster as
  well, as before. Without it a reader every frame (glGenerateMipmap's CPU
  filter with `YAKOGL_V3D_TFU` unset, reader 6) would trade the TLB's raster
  store for a CPU untile every frame. With the TFU on, glmark2 reads no
  target on the CPU, so the rule never fires there; a refused TFU chain
  (the arena line's mipmap refusal names) would fire it once.
- **GL core hooks** (paths 1 to 3 and reader 11): `texture::storage_back`
  calls `backend::storage_current` (not a table entry: the V3D is named in
  `src/backend.rs`, as `samples_rendered_depth` does, and does nothing unless
  initialised), from `settle_texture` (after `chain_back`), from
  `framebuffer::gpu_idle_for_cpu` for every colour attachment of the read
  and draw framebuffers (glReadPixels, glCopyTex*, both sides of
  glBlitFramebuffer), and from `tf_capture::settle_for_read`. The
  CPU filter of glGenerateMipmap (path 3) goes through `settle_texture`.
- **Counter**: `ColourStores`, the RT 0 stores of submitted jobs (raster and
  image, with bytes), kept for the last whole frame and logged at shutdown
  ("colour stores in the last frame"), with the write-backs since init.
- **Tests**: the stand-in GPU of `tests/v3d_lazy_raster_readers.rs` follows
  the job: it starts the tile buffer from what the job loads (raster, image
  with `uif_load`, or the clear colour), draws, and stores the raster only
  without `uif_only`; it reads the target from `v3d_job_end`. The scene's
  texture was sampled with a mipmap filter and no chain, so it was
  incomplete and never sampled: no render image was made and the tests
  passed without exercising the lazy store. It is now sampled with
  GL_LINEAR (level 0 alone is complete), `the_render_stores_the_image_alone`
  pins the store itself, and the partial-upload test makes the storage
  stale before its glTexSubImage2D. With `v3d_storage_current` disabled, 7
  of the 9 tests fail.
