<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glBlitFramebuffer depth: what is open (the window, cube and array sources)

`src/gl/blit.rs` copies depth and stencil on the CPU. It reads and writes a
renderbuffer's plane through `backend::depth_resolve`, `stencil_resolve` and
`ds_write` (the PowerVR implements all three: `pvr_backend_ds_write`,
`pvr_backend_stencil_resolve`), and it reads and writes a 2D depth texture's
R32F texels. Three source cases still answer `GL_INVALID_OPERATION`
(`REFUSED_DEPTH_SRC`):

1. depth read from the window;
2. depth read from a cube depth texture;
3. depth read from an array depth texture.

Writing into the window is refused for the same reason as (1) (`REFUSED_DEPTH_DST`).

## What the code says (not measured)

- The PowerVR stores the window top-down: `GlesDraw::flip_y` is set for the
  window surface and not for an FBO (`src/gl/draw.rs`, the tests that assert
  "the window surface is stored top-down" and "FBO textures are in texel
  order"). The depth buffer of a frame follows its target, so the packed D24S8
  words of a window frame are most likely row 0 = the top of the window, while
  an FBO's are row 0 = GL row 0. `depth_resolve` returns the words in memory
  order, so a window read needs a vertical mirror on the PowerVR. Nothing has
  read a window's depth back yet, so this is the design reading of the code,
  not a result.
- The V3D, the GC7000 and the Mali back ends: their window orientation was not
  read here. The V3D note `v3d-depth-storage.md` and the GC note
  `gc-depth-resolve.md` describe the layout of a depth *texture* only.
- A cube or array depth texture's face or layer: `depth_texels` accepts
  `GL_TEXTURE_2D` only, and `GlesDraw::depth_id` names the texture, not a face
  or layer. Whether a back end keeps one buffer per face/layer under one id or
  one buffer per texture is not known; `texture::resolve_depth` has the same
  2D-only limit.

## Why it was not finished here

The measurement needs a board: render a depth ramp into the window and into an
FBO with the same GL calls, read each plane with `depth_resolve`, and compare
the row order. The worker that did this change had no board (no `BOARD_UART`
in its environment), so (c) is deferred, as the open part of the GitHub issue "glBlitFramebuffer
depth/stencil". The steps: per back end, add a `YakoglBackend` flag for "the
window's depth is stored top-down", measure it with gl_test on the AM62P, Pi 4,
i.MX8MP and NanoPC-T6, then let `depth_source` read the window with a mirror
and let `copy_depth` write it back through `ds_write`.
