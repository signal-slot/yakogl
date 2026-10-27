<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: a depth texture sampled from its depth buffer (glmark2 shadow, #40)

What the code relies on when a draw samples a depth texture a kick rendered
into: the TPU reads the ISP's depth/stencil buffer itself, and the CPU
resolve of that buffer into the texture's R32F texels (`src/gl/texture.rs`
`resolve_depth`, `pvr_backend_depth_resolve`) runs only for a CPU read or
write of the texels. Cited from `src/pvr/backend.rs` (`pvr_depth_sampled`,
`depth_buffer_binding`, `TEXF_X8U24`) and `src/backend.rs`
(`samples_rendered_depth`).

## The cost it removes

glmark2 `shadow` at 800x600 on the AM62P ran at 353 ms a frame (score 461,
`tools/cts/RESULTS.md` before this change) while 30 of 33 scenes ran at
real-time rates. The scene renders the horse's depth into a 1600x1200
GL_DEPTH_COMPONENT texture (twice its canvas), then draws the ground
sampling that texture with NEAREST and compares in the shader. On the host
harness `docs/notes/pvr-refract-draw.md` had already attributed 95 to 97%
of the scene's `glDrawArrays` time to one step, `pvr_backend_depth_resolve`:
before the ground draw the GL core waited for the depth pass's kick and
converted the 1600x1200 packed D24S8 words the ISP stored (1.92 Mpx) into
R32F texels, one closure call and one f64 divide a texel, on the CPU, every
frame. On the board that conversion reads GPU memory the CPU had to
invalidate first and writes 7.68 MB of texels it then flushes; the A53 took
the whole frame for it. The GPU's own work for the frame is a few
milliseconds (gl_test case 215 prints it beside the wall time).

The reproducer is gl_test case 215 `shadowmap` (`driver/tools/pvrlab/gl_test.c`):
the scene's two passes on a 1600x1200 depth texture, 20 frames, ms a frame
and the depth map's Mpx/s, the last frame's picture checked at four points.

## The design (Mesa's)

Mesa's PowerVR Vulkan driver has no depth-resolve step: a depth/stencil
image is an ordinary image the ISP stores to and the TPU samples, and the
format work is one table row, `FORMAT_DEPTH_STENCIL(Z24_UNORM_S8_UINT,
ST8U24, X8U24, U8X24)` (`src/imagination/vulkan/pvr_arch_formats.c:156`,
Mesa 26.2.1): the TPU format of the whole image is ST8U24, of a view on the
depth aspect X8U24 (the 24-bit unorm in bits 23..0 as red, the top byte
ignored), of a view on the stencil aspect U8X24. `pvr_arch_pack_tex_state`
(`pvr_arch_tex_state.c`) takes a linear 2D view as TEXTYPE_STRIDE with
`stride` the row pitch in texels less one, `alpha_msb` from
`vk_format_is_alpha_on_msb` (false for Z24X8), and the pack info from
`setup_pck_info`: `pco_pipe_to_pck_format` has no pack format for Z24X8
("no pack format", 0x1f) and the format is unorm (0x100), so 0x11f.

YakoGL's depth/stencil buffer is the ISP's ZLS store: packed D24S8
(`ZLS_FORMAT_TYPE` 24BITINT), laid out linearly at the ZLS-tile-aligned
width (32 pixels), rows top-down, which is exactly the layout
`pvr_backend_depth_resolve` reads (confirmed on the AM62P on 2026-10-01 by
gl_test 169, the rendered ramp read back exactly). So the buffer is a
STRIDE image of X8U24 texels with stride the aligned width, and the TPU
returns the same 24-bit depth the resolve converts: a draw reads the same
depth either way.

## What the code does

1. Before a draw that samples a depth texture whose rendered depth is newer
   than its texels, the GL core asks `backend::samples_rendered_depth(id, w,
   h)` after submitting the kick that rendered it (`gl::texture::depth_for_draw`,
   as the V3D already did). The PowerVR answers (`pvr_depth_sampled`) yes
   when the single-sampled buffer whose depth plane `id` owns holds depth a
   submitted kick stored (`valid`, the plane `LIVE`) at the texture's size
   and is not the frame still being recorded. A plane that is a whole clear
   alone keeps the CPU path (the resolve writes the clear value; the ISP may
   not have stored it).
2. The GL core resolves nothing and gives the draw's binding `depth_id`.
   `depth_buffer_binding` turns that binding into a `PVR_TEXFMT_DEPTH_BUFFER`
   one: the buffer's address, stride the aligned width, one level, the API
   extent, the texture's own sampler state; `build_tex_desc` packs it as
   X8U24, (r, 0, 0, 1), or (r, r, r, 1) under GL_OES_depth_texture, pack
   info 0x11f. A binding whose buffer is not there to sample (a check, not a
   path: `ds_make_room` never gives up a buffer holding contents) samples the
   black texel and says so once.
3. No wait: the GPU runs kicks in order, so the kick that samples the
   buffer reads it after the one that stored it (`flush`'s comment; the
   same rule every render-to-texture here relies on). The texels stay stale
   until a CPU read or write resolves them (`settle_texture`), as before.
4. The texture's R32F storage is deferred (`pvr_defers_depth_storage`, as
   the V3D's `docs/notes/v3d-depth-storage.md`): glmark2's 1600x1200 map
   never allocates its 7.68 MB of texels unless something reads them.

The exit line counts the draws that sampled a buffer and the refusals
(`ds_report`: "depth sampled from the buffer N (refused M)").

## Tests

- `src/pvr/backend.rs` `a_depth_buffer_binding_is_mesas_x8u24_stride_image`:
  the descriptor words against Mesa's values.
- `tests/depth_resolve_pvr_backend.rs`: glmark2's shadow frame on the real
  backend over a stand-in GPU: every frame's ground samples the buffer that
  holds its own depth (with the canvas kick in flight too), no resolve, the
  storage never allocated; a CPU write resolves first; a GL_LESS pass after a
  glTexSubImage2D still loads the uploaded depth.
- gl_test 215 on the board: the picture at four points, and the figure.

## Board measurement (AM62P, 2026-10-27)

gl_test case 215, 20 frames of the two passes on a 1600x1200 depth map, the
same picture at its four points in both runs:

| image | ms a frame | depth map Mpx/s | GPU busy a frame (TA\|3D) |
|---|---:|---:|---:|
| main `c5dcf5e8` plus the case alone | 355.20 | 5.4 | 6.35 ms |
| with this design (`c999f2f7`) | 27.75 | 69.1 | 6.37 ms |

The GPU's work is the same 6.4 ms; the 349 ms that went were the CPU
resolve. What remains above the GPU time (about 21 ms) is the two kicks'
submit and fence round trips with a glFinish each frame, not the texture.

glmark2 at 800x600 (`tools/cts/RESULTS.md`, the AM62P row): shadow 353.23
to 10.64 ms a frame (33x), refract 375.53 to 37.03 ms (10x: its depth
texture takes the same path), score 461 to 467, the other 31 scenes
unchanged within the run-to-run spread, every picture probe of shadow equal
to the recorded column, 0 GL errors, 0 resets, 2026 draws a run sampled a
buffer and none was refused.
