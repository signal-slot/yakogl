<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): a framebuffer with no colour attachment

Date 2026-11-14, base `d09629d3`. Plan #367, item `depthonly`. This is cause
D-DEPTHONLY of `docs/notes/mali-glmark2-forecast.md` section 2: glmark2's
shadow scene (`DepthRenderTarget::setup`) renders its first pass into a
framebuffer whose only attachment is a `GL_DEPTH_COMPONENT` texture. With
`mali::backend::DRAWS_WITHOUT_COLOUR` false, `backend::DEPTH_ONLY_TARGETS` was
false for `gpu-mali`, `src/gl/framebuffer.rs` `check_status` answered
`GL_FRAMEBUFFER_UNSUPPORTED` (0x8cdd), and glmark2 gave the scene up. It is
now true. The GL core needed no change: it already hands such a draw over
with `target_gpu` 0 at the depth attachment's size, colour mask 0 and no
colour clear (the V3D, PowerVR and GC7000 back ends draw them).

Mesa citations are `file:line` in Mesa 26.2.1
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`); `G/` is
`src/gallium/drivers/panfrost/`, `L/` is `src/panfrost/lib/`.

## 1. What Mesa emits for `nr_cbufs` 0

Panfrost does not leave the colour part out. It emits the frame of a batch
whose `key.nr_cbufs` is 0 with one render target slot and treats that slot as
"no surface":

| Word | Mesa | Here |
| --- | --- | --- |
| FBD aggregate | `alloc_fbd`: FRAMEBUFFER, ZS_CRC_EXTENSION, `MAX2(nr_cbufs, 1)` RENDER_TARGETs (G/pan_csf.c:430-435) | `begin_frame_depth_only`: the same room for one target |
| Parameters `Render Target Count` | `MAX2(fb->rt_count, 1)` (L/pan_desc.c:1410) | 1 |
| `Effective Tile Size`, `Color Buffer Allocation` | `pan_cbuf_bytes_per_pixel`: "dummy/non-existent render-targets use RGBA8 UNORM", 4 bytes a sample (L/pan_desc.c:569-592), through `pan_select_tile_size` (:607-648) | one RGBA8 target's: 1024 pixels, 4096 bytes at 1x |
| The render target | `pan_emit_rt` for `!rt` (L/pan_desc.c:984-1001): clean tile write from `pan_get_clean_tile_info`, which loops over `fb->rt_count` (0, :1270-1279), so off; internal buffer offset 0; `rt_clear` of a slot not cleared, 0 (:964-976); dithering on; internal format R8G8B8A8; writeback block format Tiled U-Interleaved (v7+); every other field 0, so `Write Enable` off | `frame::dummy_render_target`: words `0x04000000 0x00008100 0 ... 0` |
| FBD pointer tag | `zs_crc_extension_present`, `render_target_count = MAX2(fb->rt_count, 1)` (L/pan_desc.c:1509-1512) | extension bit and 1 target, as a colour frame with Z/S (`DESC | 3`) |
| ZS/CRC extension, Z clear, Z write | as any frame with a depth/stencil view (L/pan_desc.c:1417-1446) | unchanged: the linear D24S8 of the attachment pair, written back |
| Each draw's BLEND array | `MAX2(rt_count, 1)` descriptors; "Disable blending for unbacked render targets": `rt_count == 0` gives enable off, mode Off (G/pan_cmdstream.c:286-307) | one descriptor, the disabled one the colour-masked draw already wrote |
| Fragment shader | `panfrost_fs_required` runs it only for side effects (discard), alpha-to-coverage or a colour buffer it writes; the loop over `nr_cbufs` finds none (G/pan_cmdstream.h:200-235) | `writes_colour` is false in a frame of no colour: no fragment shader unless it discards or alpha-to-coverage is on |
| DCD 0 | the no-shader branch: force early, forward pixel kill (G/pan_csf.c:1621-1637) | unchanged (the colour-masked draw's) |
| DCD 1 `Render Target Mask` | `outputs_written & ctx->fb_rt_mask`, `fb_rt_mask` 0 with no colour buffer (G/pan_context.c:457-462, G/pan_csf.c:1641-1645) | 0, even when the shader runs |

Mesa 26.2.1 also bounds the tile size by the Z tile buffer
(`z_tile_buf_budget`, L/pan_desc.c:617-625). For a 1x D24S8 frame that bound
is above the 1024-pixel hardware limit, so it changes nothing here; the
builder ignores it for every frame, as before.

## 2. The design

- `frame::OpenFrame::no_colour` marks a frame with no colour surface.
  `frame::begin_frame_depth_only` opens one. Its target `t` gives only the
  size and the sample count; its address, row stride and format are not read.
- `end_frame` sizes the tile for one RGBA8 target and writes the dummy render
  target in place of the colour targets. A colour preload, a sample store or
  colour targets 1.. on such a frame is the caller's defect
  (`INVALID_DEPTH_ONLY`).
- `emit_draw` treats the target as unbacked: no colour is written whatever
  the mask, so the blend is off and no fragment shader runs without side
  effects, and the render target mask is 0.
- `backend.rs`: a draw with `target_gpu` 0 and a depth or stencil object is
  depth-only (`depth_only`). `check` skips the colour checks for it. Its
  `TargetKey` has `gpu` 0, so a later draw with a colour target ends its
  frame. The colour format is a placeholder, RGBA8, the dummy's.
  `open_frame` opens it with `begin_frame_depth_only`, never with a colour
  preload or a sample store. That holds for its second frame too, when a
  frame runs out of room. A draw with neither a colour target nor a depth or
  stencil object is still `INVALID_NO_TARGET` (`-EINVAL`).
- The depth write-back, the planes (`ds_retention`), the Z/S preload of a
  later frame and `mali_depth_resolve` are the colour frame's, unchanged. So
  the pass that samples the shadow map resolves the depth the depth-only
  frame wrote, as for any depth texture (`docs/notes/mali-texture-refusals.md`
  (c)).

## 3. What is pinned, and against what

`src/mali/frame_depthonly_tests.rs`:

- The framebuffer parameters block is byte-equal to the board's frame of
  `tools/mali/board-dump/zs-depth-2026-10-19.txt` source "nocolour"
  (Mesa 25.0.7, an RGBA8 FBO with D24S8 and a draw under glColorMask(none)),
  but for the tiler pointer, the difference `frame_zs_tests.rs` already
  names. The board frame has one RGBA8 target, so its tile size, allocation
  and target count are those Mesa gives the dummy.
- The draw's ZSD, tiler flags and DCD words are byte-equal to that board
  draw, and so is its Blend descriptor but for the equation word
  `frame_zs_tests.rs` names. Mesa's colour-masked draw takes the same
  branches as its draw with no colour buffer (`fs_required` false, blend off,
  render target mask 0).
- The dummy render target has no board or drm-shim record: no trace in
  `tools/mali/board-dump` or `tools/mali/testdata` holds a render target
  without a surface. Its 16 words are pinned to the Mesa lines of section 1.
- On the backend mock: a depth-only clear is parked and the draw opens one
  frame (dummy target, the D24S8 extension, no fragment shader, mask 0). The
  pass that samples it gets the depth through `mali_depth_resolve` (-EBUSY
  while the frame is open). A colour pass then has its own render target and
  no extension. A draw into a colour target with depth keeps its target.

## 4. Open

- **Pin on the board**: glmark2 `shadow` on the NanoPC-T6, and a dump
  (`tools/mali/mesa-board-ref.sh`) of Mesa's depth-only FBO frame to replace
  the source-read dummy words with a byte comparison. That belongs to the
  plan's board item.
- glmark2's shadow pass then samples the depth texture. That is cause
  D-DEPTHTEX (R32F resolve), which is not part of this item.
