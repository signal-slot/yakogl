<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: colour attachments of different sizes

Research, written before any code item. Today `extra_targets`
(`src/v3d/backend.rs`, called from `draw_call`) refuses a draw whose colour
attachments past the first differ in size from the first one, by name:
"colour attachments of different sizes". ES 3.0 4.4.4.2 allows such a
framebuffer and 4.4.1 renders "only within the largest area that can fit in
all of the attachments", that is the intersection. This note says what the GL
core hands over, how the job builder sizes the frame and stores each RT, what
Mesa's v3d does, which part of the V3D back end must change (it is
`src/v3d` only; the job builder needs no change), the host tests, and which
dEQP rows it can move (probably none on the Pi 4 today).

Function names are the address; line numbers drift (yakogl is main at
`a8e79b52`). Mesa is 26.2.1, paths relative to the Mesa tree; the vendored
`third_party/mesa-26.2.1` holds only the v3d format table, so the files below
were read in the full 26.2.1 tree of the deps cache (`mesa-pan`), the same
tag. Mesa is described in words here, not copied.

## (a) What the GL core hands over

- `GlesDraw::target_w/target_h` (`src/backend.rs`) are the size of the
  **frame's own target**: the first colour attachment the draw writes
  (`resolve_target` in `src/gl/draw.rs`, via `framebuffer::frame_attachment`),
  not the intersection. `GlesDraw::target_gpu` and `target_pitch` are that
  attachment's.
- `GlesDraw::targets[i]` (`GlesDrawTarget`: `gpu`, `w`, `h`, `format`) carries
  each target's **own** size, built by `resolve_targets` from
  `framebuffer::fb_target_n` (the attached level's `st.w`, `st.h`). Slot 0
  repeats the frame target. A `GL_NONE` slot past the first has `gpu` 0 and
  size 0x0. `num_targets` is the highest slot with memory plus one.
- `GlesDraw::ds_w/ds_h` (`resolve_target`'s depth/stencil half, from
  `ds_identity`) are the depth (else stencil) attachment's own size, set for
  an FBO only; the V3D back end does not read them today.
- The GL core already decides completeness the ES 3.0 way: with the default
  build `report-es2` is off and unequal attachments are
  `GL_FRAMEBUFFER_COMPLETE` (`src/GL_CORE_NOTES.md`, "Colour attachments of
  different sizes"; test `colour_attachments_of_different_sizes_by_version`,
  `src/gl/draw.rs`). The comment there says each backend "clips to it
  (PowerVR) or refuses by name (V3D)". So nothing is missing in `src/gl` for
  the V3D to do this: every size it needs is already in the `GlesDraw`.
- The viewport and scissor are in GL window coordinates and are not clamped
  to the intersection by the core; a back end clips (below).
- The other places that see the sizes are not draws on the V3D:
  `glBlitFramebuffer` (`src/gl/blit.rs`, `blit_colour`) and `glReadPixels` run
  on the CPU and cut each image to its own bounds (`Rect::of_image`), so a
  mixed-size FBO does not reach the V3D there. (GL says the blit region is
  limited by the draw framebuffer; the core cuts to each attachment. That is
  a separate, GL-core question, not this refusal.)

## (b) How the job builder sizes the frame and stores each RT

`driver/src/v3d/v3d_job.c`, `driver/src/v3d/v3d_job.h`:

- One frame size per job: `struct v3d_job_target.width/height`. They are the
  binning configuration (`v3d_job_begin`: `bin.width_in_pixels`,
  `height_in_pixels`), the tile geometry (`v3d_frame_tiling_bpp(j->t.width,
  j->t.height, job_rts, msaa, max_bpp)` in `v3d_job_end`), the rendering
  mode (`TILE_RENDERING_MODE_CFG_COMMON.image_width_pixels/
  image_height_pixels`), the supertile loop and the default clip window
  (`emit_clip_window`: lo 0,0, hi `t.width`, `t.height`, then the viewport and
  scissor). Nothing in the builder looks at an attachment's own size.
- Each RT's store/load is independent of the frame size: RT 0 uses
  `t.address` and `t.stride`; RT 1..3 use `t.rt[n - 1].address` and
  `.stride` (`v3d_job_end`, `emit_tile_list` loops over `job_rts`: `LOAD/
  STORE_TILE_BUFFER_GENERAL` with `memory_format` raster and
  `height_in_ub_or_stride` = `rt->stride`). The TLB writes a tile's rows
  clipped to the frame (the image width/height of the rendering mode), so an
  RT bigger than the frame is written only inside the frame, and the rest of
  its memory is untouched. An RT smaller than the frame would be written past
  its rows: that is why the frame must not be larger than any attachment.
- The FBO's rows are stored GL row 0 first: the back end draws an FBO without
  `flip_y` (`src/v3d/backend.rs`, the `v3d_depth_resolve` comment, and
  `v3d_job.h` `wpos` (1, 0) for an FBO). The intersection is therefore the
  rectangle (0, 0)..(min w, min h) in memory too: each RT's base address and
  row stride stay what they are, only fewer rows and fewer pixels per row are
  stored. No address offset is needed (a window, stored top row first, never
  has a second attachment).
- The builder's checks: `v3d_job_begin` refuses width or height 0 and
  `rts > 4`; it has no per-RT size. So a smaller frame is a plain change of
  `width/height` in `V3dJobTarget`.

What the back end passes today (`begin_job`, `src/v3d/backend.rs`): the job's
`width/height` are `Target.w/h` = `d.target_w/h`; `address`/`stride` are RT
0's (`stride = pitch * bpp`); and RT 1..3 get `stride: target_pitch(w) *
bytes` with **`w` the frame's, i.e. RT 0's, width**. That last line is the
one place that bakes in "all RTs have RT 0's size" beyond the refusal.

## (c) What Mesa's v3d does

- The frame size is the minimum over the bound surfaces. Mesa's state tracker
  derives the gallium framebuffer's width and height from the bound
  surfaces: `st_update_framebuffer_state` in
  `src/mesa/state_tracker/st_atom_framebuffer.c` calls
  `update_framebuffer_size`, a `MIN2` of the framebuffer width/height with
  each colour surface's and the depth/stencil surface's size. (The gl
  framebuffer's `Width/Height` are the minimum too, `fbobject.c`
  `_mesa_test_framebuffer_completeness`: `minWidth`/`minHeight`, assigned to
  `fb->Width/Height` at the end.) `util_framebuffer_min_size`
  (`src/gallium/auxiliary/util/u_framebuffer.c`) is the same rule for drivers
  that ask.
- v3d takes that size as is. `v3d_set_framebuffer_state`
  (`src/gallium/drivers/v3d/v3dx_state.c`) copies the state;
  `v3d_start_draw` (`v3dx_draw.c`) sets `job->draw_width/draw_height` from
  `v3d->framebuffer.width/height`. Those feed the binning config
  (`v3dX(start_binning)`: `width_in_pixels`, `height_in_pixels`), the tile
  count (`v3d_job.c`: `tile_desc.draw_x/draw_y`, rounded up from the draw size
  over the tile size) and the rendering mode (`v3dx_rcl.c`:
  `image_width_pixels`, `image_height_pixels`). No v3d code compares the
  attachments with each other: there is no "different sizes" refusal.
- The per-RT store is per resource: `store_general` (`v3dx_rcl.c`) takes the
  surface's own address (`v3d_layer_offset`) and
  `v3d_surface_get_height_in_ub_or_stride` (same file): the stride of the
  surface's own raster slice, or the padded height in UIF blocks of its own
  slice. Both are the resource's, never the frame's. A UIF surface bigger than
  the frame is simply stored in part. The tile-buffer size takes the largest
  bpp of all RTs (`v3d_get_tile_buffer_size`), which `v3d_frame_tiling_bpp`
  already mirrors (the `typed` path).
- Viewport and scissor are clipped to the frame by `CLIP_WINDOW`
  (`v3dx_emit.c`, "Clip to the scissor if it's enabled, but still clip to the
  viewport"; the window never exceeds the drawable the binning config
  names). `emit_clip_window` in `v3d_job.c` is the same.
- A clear: `st_Clear` (`st_cb_clear.c`) limits a quad to the framebuffer's
  `_Xmin.._Ymax` box (itself inside `Width/Height`, the minimum); `v3d_clear`
  takes the TLB clear (`v3d_tlb_clear`, `v3dx_draw.c`) for what remains, and
  a TLB clear covers the whole frame (`job->draw_*`), so it clears the
  intersection and nothing past it. Mesa has no special case for unequal
  sizes: a smaller frame is all it needs.

## (d) Does a draw, a clear and a blit stay inside the intersection?

With the frame set to the intersection:

- **Draw.** Rasterisation is limited to the binning config's frame, the clip
  window (`emit_clip_window`, clamped to `t.width/t.height`) and the tiles
  that exist. Every store stays inside the frame in every RT, each at its own
  address and stride. Yes.
- **Clear (TLB).** A fused `glClear` is the tile buffer's clear colours
  stored with the tile list: it covers the frame, so it clears the
  intersection only, which is what GL asks. The GL core's `plan_clear` decides
  "the scissor holds the whole target" against RT 0's size
  (`plan_clear(..., (d.target_w, d.target_h, ...))`): a scissor that holds the
  intersection but not RT 0 becomes a clear quad, a scissor that holds RT 0
  stays a TLB clear; both give the intersection on the V3D. Yes.
- **Clear quad** (scissored or masked): drawn through the same job, so the
  quad is clipped by the same clip window. Yes (and the several-attachment
  quad is the separate item `v3d-mrt-clear`).
- **Blit and read-back.** CPU (see (a)); the V3D has nothing to do, and the
  jobs have been flushed before (`gpu_idle_for_cpu`).

## (e) The change, in `src/v3d` only

The job builder, the driver and `src/gl` need **no** change. The edits, all in
`src/v3d/backend.rs` and its tests:

1. **The frame size is the intersection.** In `draw_call`, after
   `extra_targets`, compute `(fw, fh)` = the minimum of `d.target_w/h` and of
   every written target past the first (`gpu != 0`), and of the depth/stencil
   attachment's `d.ds_w/ds_h` when `d.ds_persist` and the draw has an FBO
   (`ds_w != 0`). `Target.w/h`, the tile-count refusal (`MAX_TARGET`), the
   `v3d_frame_tiling_bpp` check in `begin_job`, the job's `width/height` and
   `Target` identity use `(fw, fh)`.
   `extra_targets` stops returning the "different sizes" error. A draw whose
   sizes are all equal is byte for byte what it is today.
2. **Each RT's own pitch.** Replace `target_pitch(w) * bytes` (RT 1..3 in
   `begin_job`) by the RT's own width. `Target` gets `rt_w: [u32; 3]` (or the
   pitches) from `t.w`, part of the job's identity (`g.j.target != target`
   flushes). RT 0 keeps `draw_pitch(d)` (its `target_pitch`, which may be a
   view's wider pitch). `extra_targets` returns them.
3. **RT 0's render image and the depth buffer.** Two things were sized by the
   frame and assumed it was the surface:
   - `RtImage` (the render image, `v3d-lazy-raster.md`) is keyed and sized by
     RT 0's `w` x `h`, and `rt_level0_back` writes the whole image back to
     the raster storage. A job whose frame is smaller than RT 0 stores only the
     intersection into it, so the rest of the image is not what the storage
     holds. Safest choice: a draw whose frame is smaller than RT 0 does not use
     a render image (`rt = None`, the raster store of RT 0 alone, as a target
     with no sampling history). The alternative, a seeded image per size, is
     not worth it for a case dEQP barely has. The test must show the raster
     store and that a later sample of the texture still sees the pixels (the
     existing CPU-copy path for a target with no image).
   - `ZsBuf` (`b.w`, `b.h`, one per `ds_id`) is re-allocated "when its id
     comes back at another size" (the comment at the `ZsBuf` table). A frame
     that shrinks between two draws of the same depth buffer therefore drops
     what the buffer held, as a resized renderbuffer does; that is not
     right for an attachment that did not change, but it is the existing
     behaviour of any re-sized `ds_id` and the intersection changes only when
     an attachment changes. Check with a test (below); a `ZsBuf` keyed by the
     depth attachment's own size would be the alternative.
4. **Silent gap beside it: a depth/stencil attachment of a size other than
   RT 0's.** Today nothing reads `ds_w/ds_h` in `src/v3d`: a 64x64 colour
   with a 32x32 depth renderbuffer draws 64x64 against a private 64x64 `ZsBuf`
   (GL renders only 32x32). It is the same rule, one edit (1. above includes
   it), but it moves behaviour that is not behind the refusal; a test pins it.
5. The two stale comments (`extra_targets` doc, "at RT 0's size";
   `draw_call` "each at RT 0's size in its own format" in `begin_job`) and
   `docs/online-shaderc/v3d-backend.md` ("Multiple render targets") say the
   RTs share a size; they are updated in the same commit. `GL_CORE_NOTES.md`
   and `docs/es3-cts-failures.md` ("the V3D refuses unequal sizes by name")
   are the docs item's, not this one's.

Not needed: any `V3dJobTarget` field, `v3d_job.c`, `tools/v3d/job_check.c`
(it checks the builder against Mesa's words; sizes it already takes apart:
`width/height` and `rt[n].stride` are separate fields), a GL-core change.
Pinned fixtures: none moves (a draw of equal sizes is unchanged); the
`attachments_the_job_builder_cannot_take_are_refused_by_name` test changes
its first case (see (f)).

## (f) Host tests (`src/v3d/backend_tests.rs`, `Rig`, `mrt_draw`)

`mrt_draw` already builds an N-attachment draw; `rig.jobs()` shows the
`V3dJobTarget` (`width`, `height`, `rts`, `rt[i].address/stride`).

1. `colour_attachments_of_different_sizes_draw_in_their_intersection`:
   RT 0 64x64, RT 1 32x48 (both orders): the draw is accepted (0), one job,
   `width == 32`, `height == 48`, `rt[0].stride` = RT 1's pitch for the
   RT 1 slot (32 px x 4) and RT 0's (64 x 4) for RT 0; the same with RT 0
   the smaller. Revert check: with the `extra_targets` refusal back, `draw`
   returns -EINVAL and the log names it.
2. `an_attachment_past_the_first_keeps_its_own_pitch`: RT 1 wider than RT 0
   (96x32 against 64x64): `rt[0]` (RT 1) `stride == 96*4`, frame 64x32.
   Revert check: the old `target_pitch(w)` gives 64*4, a store that walks
   into the next row.
3. `a_gl_none_slot_is_not_a_size`: the existing
   `a_none_draw_buffer_is_configured_and_never_given_an_address` stays green
   (the `gpu == 0` skip comes first, so 0x0 is never in the minimum).
4. `the_depth_attachment_limits_the_frame`: colour 64x64, `ds_w/ds_h` 32x32,
   `ds_persist`: frame 32x32. And `ds_w` 0 (default framebuffer) does not
   limit. Revert check: width stays 64.
5. `a_fused_clear_of_unequal_attachments_clears_the_intersection`: `do_clear`
   with RT sizes 64x64 and 32x32: `jobs[0].clear` as for an equal draw, the
   job 32x32, RT 0 stored 32x32 (no `load` of the part outside). With
   `clear_quad`/scissor: refused by the other item's name only while
   `v3d-mrt-clear` is open, drawn after.
6. `a_smaller_frame_does_not_use_the_render_image`: RT 0 sampled (an
   `RtImage` live) and a draw with a smaller RT 1: the job has
   `uif_address == 0`, raster store only; a following draw with equal sizes
   is the old path again.
7. `a_job_is_not_continued_across_an_intersection_change`: two draws with the
   same RT 0/RT 1 addresses and RT 1 resized between them: two jobs
   (`g.j.target != target`).
8. Edit `attachments_the_job_builder_cannot_take_are_refused_by_name`: the
   `d.targets[1].w = 32` case moves out (test 1); the format and scissored-clear
   cases stay.

A device test is not needed for the size logic (the TLB's frame clip is
Mesa's documented behaviour, packet fields the builder already writes); one
Pi 4 run of the dEQP groups below checks the whole.

## (g) Which dEQP rows it moves

`docs/es3/pi4-gles3-fails.tsv` (read only) has 1,383 not-pass rows. The
board logs of the run (`tools/cts/log/2026-10-31-pi4-gles3-a/`) list the
V3D refusals by name per image (`v3d_refused_draws`, "causes:"), and none
names "colour attachments of different sizes"; the only cause printed in
FO6 and M1 is the integer clear. **So no row is known to hit this refusal
today.** By case name, the groups that can draw into unequal attachments:

- `dEQP-GLES3.functional.fbo.completeness.size.*` (45 rows are in
  `fbo.completeness` as a whole): those cases build unequal attachments but
  only read `glCheckFramebufferStatus`; the GL core already answers
  COMPLETE (ES 3.0), so they do not reach the V3D back end. Not moved by
  this change.
- `fbo.render.*` (305 rows), `fbo.color.*` (206 rows): single-size
  attachments in dEQP's fixtures (`fbo.render.resize` and `recreate_*`
  re-create attachments at new sizes, one size at a time). The
  `fbo.render.shared_*` and `fbo.render.depth*` cases share a depth
  attachment between framebuffers; a depth renderbuffer smaller than the
  colour would be the only unequal case (item 4). Unknown without a
  board run; the case sources are not in the tree.
- `draw_buffers_indexed.*` (76 rows), `fragment_out.*` (158 rows):
  all attachments are created at one size by the test.

My estimate: 0 to a few `fbo.render`/`fbo.color` rows, decided by a Pi 4
run after the change, not by this reading. The value is that the V3D draws
the ES 3.0 framebuffers the GL core already calls complete, and that
attachment sizes stop being a hidden precondition of the other items (the
several-attachment clear, `v3d-mrt-clear`, takes the frame size from here).
Plan #320's rerun of FO6, M1 and the `fbo.*` images after the plan lands
is where the count appears; if it is 0, say so on #1261.

## Risks the tests do not close

- The `RtImage`/lazy raster interaction (item 3) is the one with state across
  jobs; a Pi 4 run of `fbo.render.texsubimage` and the shared-colourbuffer
  cases is the check.
- `ZsBuf` re-allocation on a frame-size change drops stored depth (item 3);
  harmless for a framebuffer that does not change between draws.
- The tile counts: `MAX_JOB_TILES` and the 2048 limit are checked on the
  frame, which only gets smaller.
