<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): depth/stencil objects shared between framebuffers

Date 2026-11-15, base `fc195a8d`. Plan #373, item `zs-share`. This is cause 5
of `docs/es2-cts-failures.md` section M.1: 35 Fail in
`dEQP-GLES2.functional.fbo.render.{shared_depthbuffer, recreate_depthbuffer,
recreate_stencilbuffer}.*_depth_component16_stencil_index8`, seven colour
configurations each (`rbo_{rgb565,rgb5_a1,rgba4,bgra}`,
`tex2d_{rgb,rgba,bgra}`), in the 2026-11-12 tree
(`tools/cts/log/2026-11-12-t6-gles2-S1-a50d87b5/results.tsv`) and again on
2026-11-15 (`2026-11-15-t6-gles2-S1-37305608`). They were two defects, not
one:

| Cases | dEQP | Cause | Fix |
| --- | --- | --- | --- |
| `shared_depthbuffer.*` (7), `recreate_*.no_rebind_*` (14) | `Got unexpected error` | the draw is refused, `NOT_BUILT_ZS_ELSEWHERE` (row `T6S1R06`, `refused=42`: 3 a shared case, 1 a depth and 2 a stencil `no_rebind` case) | the plane is taken into the pair that draws it (section 2) |
| `recreate_*.rebind_*` (14) | `Image comparison failed` | no refusal: a scissored stencil clear that opens a frame also wrote the clear colour into its box (section 3) | the clear quad writes the colour only when the clear names it |

The `*_depth_component16` and `*_stencil_index8` cases (one renderbuffer)
passed before and are drawn as they were.

Mesa citations are `file:line` in Mesa 26.2.1
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`); `G/` is
`src/gallium/drivers/panfrost/`. The Mesa 26.2.1 sources the design reads:
`src/mesa/main/fbobject.c` (attachment and completeness rules),
`G/pan_job.c` (`panfrost_batch_add_surface`, `panfrost_batch_update_access`),
`G/pan_fb_preload.c` (`pan_preload_needed`) and
`src/mesa/state_tracker/st_cb_clear.c` (the clear quad's colour mask). The
descriptor references are drm-shim traces of Mesa 26.2.1 in
`tools/mali/testdata/pan_zs_ref/`.

## 1. Why only the separate DEPTH_COMPONENT16 + STENCIL_INDEX8 cases

The backend keeps one linear D24S8 buffer per attachment pair
(`ZsBuf`, `src/mali/backend.rs`), keyed by `GlesDraw::ds_id`, which the GL
core makes from the names (slots) of the depth and the stencil attachment
(`src/gl/draw.rs` `ds_pair_id`). Each GL object's plane (`PlaneEntry`) says
what it holds and, when a past frame drew it, which pair's buffer holds it.
A frame of a pair starts a plane its own pair drew from that buffer (the Z/S
preload), and a plane another pair drew was refused: the frame writes both
planes of its buffer back, so it may not start one from memory that does not
hold it.

With one renderbuffer on one attachment point the pair id follows the
object, so the cases share the pair. With two renderbuffers it does not:

- `shared_depthbuffer`: framebuffer A holds depth 2 and stencil 3 (pair
  `0x20003`), framebuffer B holds A's depth 2 and its own stencil 5 (pair
  `0x20005`). A draws the depth; B's depth-tested quad, its scissored
  stencil clear and its stencil-tested quad found the depth in A's buffer.
- `recreate_*.no_rebind_*`: the framebuffer is unbound, the depth (or
  stencil) renderbuffer deleted, and its name given new storage. The old
  object stays attached (ES 2.0 4.4.3; Mesa fbobject.c:2262-2284) as an
  orphan in another slot (`src/gl/framebuffer.rs` `glDeleteRenderbuffers`),
  so the same objects come back under pair `0x40003` (or `0x20004`), and the
  depth the first quad drew is in the old pair's buffer.

The GL core reports such a framebuffer complete in the ES 2.0 build that the
board's dEQP-GLES2 runs (`report-es2`; ES 2.0 has no same-image rule). Mesa
reports it `GL_FRAMEBUFFER_UNSUPPORTED` ("Separate Depth/Stencil buffers",
`src/mesa/main/fbobject.c:1248-1253`), so with Mesa these 35 cases are
NotSupported, and the zs-refs traces of them
(`tools/mali/testdata/pan_zs_ref/*_depth_component16_stencil_index8.txt`)
hold no frame. The frames this backend now builds for them are compared
with Mesa's frames of the same draws in the `*_depth_component16` cases
where the fields are the same thing (section 4).

## 2. A plane drawn in another pair is taken into this pair's buffer

Mesa keeps the depth and the stencil in the GL object's own resource. Every
batch adds its Z/S surface as a written resource (`panfrost_batch_init` ->
`panfrost_batch_add_surface`, G/pan_job.c:41-52, 91), and a batch that
reads or writes a resource another batch wrote submits that writer first
(`panfrost_batch_update_access`, G/pan_job.c:243-265). The second
framebuffer's frame then names the same memory and preloads what it does
not clear (`pan_preload_needed`, G/pan_fb_preload.c:746-760): in the
DEPTH_COMPONENT16 trace, B's frame is `Pre Frame 1: Early ZS Always`, `Z
Clear: 0`, its ZS target the D16 resource A's frame wrote
(`shared_depthbuffer__rbo_rgb565_depth_component16.txt`, frame3).

Here the memory is the pair's, so the plane moves to the pair that draws it
(`take_plane`, `src/mali/backend.rs`): at the frame's first use of the
attachment (`add_to_open`), each plane that starts `Elsewhere`

1. gets this pair's buffer (`zs_buffer`, made on first use; the other
   pair's buffer is still held by the live plane, so it cannot be the one
   given);
2. waits for every submitted frame (`wait_all`): the frame that wrote the
   plane, and any frame still writing this pair's buffer back, as
   `seed_depth` does for depth texels the CPU wrote;
3. copies the plane's bits of each word, the depth's low 24 or the
   stencil's high 8, over the rows and columns both buffers have (the same
   linear D24S8 at `4 * w` bytes a row), keeping each word's other plane;
4. records the plane live in this pair (`set_plane`) and its start
   `Unloaded`, so the frame preloads it with the Z/S preload program it
   already uses for its own pair's planes.

The other pair's buffer keeps its old copy, but its entry no longer names
the plane: a later frame of that pair finds the plane `Elsewhere` and takes
it back the same way (the shared case's A after B). A plane cleared whole
before the frame uses the attachment is the frame's clear value, and nothing
is copied (`record_in` replaces the start). A frame that never uses the
attachment leaves the plane where it is (`end_open` skips `Elsewhere`).

The cost is a CPU wait and a copy of one plane, once each time the object
changes pair: rare (two framebuffers sharing an object, an orphaned
renderbuffer). A buffer per GL object, as Mesa has, would need the frame to
write the depth and the stencil to two surfaces (the ZS and the S writeback
of the ZS/CRC extension) and preload from two, which the frame builder and
the three Z/S preload programs do not build; the copy keeps every frame
descriptor as it was.

What stays refused, `NOT_BUILT_ZS_ELSEWHERE` (now "... whose buffer no
longer holds it is not built yet"): a plane recorded live in a pair whose
buffer entry was given up, so there is nothing to copy. `zs_buffer` gives
up a pair's entry only when no live plane names that pair, or when the pair
itself changes size, so no dEQP-GLES2 sequence known reaches it.

## 3. A scissored stencil clear that opens a frame wrote the colour

In the `rebind` cases the framebuffer stays bound. The GL core holds the
scissored `glClear(GL_STENCIL_BUFFER_BIT)` until the next draw;
`glDeleteRenderbuffers` ends the frame that holds the first quad
(`mali_attachment_gone`); the held clear then comes as a clear quad with the
recreated object, and is the draw that opens the next frame. `record_in`
set `quad.colour` from `do_clear || clear_quad` in that branch, so the quad
also wrote the GL state's clear colour (red) into the scissor box; the last
quad, which fails its stencil (depth recreated: the box holds stencil 1) or
depth test (stencil recreated: half the box is behind the first quad's
depth) there, left the red showing. The DEPTH_COMPONENT16-only and
STENCIL_INDEX8-only cases have no stencil clear, or a last quad that covers
the box, so they passed.

`do_clear` says whether the colour is cleared, `clear_quad` only how
(`backend::GlesDraw`); the quad writes the colour when `do_clear` says so,
as st/mesa's clear quad leaves every colormask 0 when it clears no colour
buffer (`src/mesa/state_tracker/st_cb_clear.c:224-240`). The other branches
of `record_in` already did.

## 4. Tests

`src/mali/backend_tests.rs`, feature set
`host-test,gpu-mali,shaderc-online,no-shader-table`:

- `a_depth_object_shared_by_two_framebuffers_is_drawn_in_both`: the shared
  case's draws as the GL core hands them (pairs `0x20003` and `0x20005`, the
  objects' ids and generations), 128x128 RGB565. No refusal; B's buffer
  holds A's depth bits with B's stencil byte; B's frame preloads the depth
  (Z only, the stencil from B's clear); its Framebuffer Parameters are
  Mesa's frame3 byte for byte except the addresses, the depth format (D24S8
  for Mesa's D16 resource) and B's stencil (Mesa's case has none); its
  depth-tested draw's state is Mesa's frame3-draw1 (`Less`, writes on,
  stencil test off, fixed-function depth). A's frame after it takes the
  depth B left, with A's stencil byte kept.
- `a_framebuffer_whose_pair_is_named_anew_keeps_its_depth`: the two
  `no_rebind` sequences (pairs `0x40003`, `0x20004`): the held scissored
  stencil clear and the last quad are drawn, the new pair's buffer holds
  the old pair's depth, the frame preloads it, and the clear writes no
  colour.
- `a_stencil_clear_that_opens_a_frame_writes_no_colour`: the two `rebind`
  sequences, `mali_attachment_gone` between the quads: the frame the held
  clear opens preloads the old image, the clear's Blend colour mask is 0
  and it writes the stencil, the last quad's is 0xf.
- `a_depth_preload_without_its_program_or_its_buffer_is_refused` (changed):
  `NOT_BUILT_ZS_ELSEWHERE` only with the pair's buffer entry given up;
  otherwise the depth is preloaded in the other pair and back.
- `gl::zs_cases_through_the_gl_core`: with `report-es2` as well, the five
  sequences of `tools/mali/testdata/pan_zs_ref/zs_seq.py` through the GL
  core on the mock, with no GL error, the pairs the tests above replay, and
  the held stencil clear's colour mask 0. In the gate's Mali build (ES 3.0
  rule) it checks that the framebuffer is `GL_FRAMEBUFFER_UNSUPPORTED`. The
  test has no `#[cfg]`: the Mali row's check (`tests_as_declared`) wants
  every `#[test]` under `src/mali` to run in its build. No gate row builds
  the Mali with `report-es2`, so the sequences are checked by hand
  (`cargo test --no-default-features --features
  host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --lib
  zs_cases_through`).

`src/mali/frame_zs_tests.rs` is unchanged: the frame builder writes the same
Z/S descriptors, and its pins (`the_depth_tested_frame_is_the_board_s`,
`the_frame_with_the_depth_test_off_is_the_board_s`,
`a_masked_draw_s_blend_and_dcd_are_the_board_s_masked_clear`,
`a_draw_with_no_colour_is_the_board_s_depth_only_draw`) keep holding for the
one-pair case.

Revert checks: without `take_plane`, the first two tests and the GL test
fail on the refused draws (`-ENOTSUP`, the board's `GL_INVALID_OPERATION`);
with `do_clear || clear_quad`, the third, the second and the GL test fail on
the clear's colour mask (0xf). The pixels are the board run's to show
(the mock's GPU writes nothing).
