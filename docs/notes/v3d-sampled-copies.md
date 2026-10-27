<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: sampled copies of the depth texture and the colour chain, and Mesa's way

Study, no code change. The question: why did glmark2 shadow and refract tile
a fresh copy of the depth texture and of the colour chain every frame, and
how does Mesa's v3d avoid it? The finding that decides the rest: **at main
the per-frame depth resolve, the depth copy and the colour copy are already
gone** (`262fa921`, `c6f17516`). What is left is on the GPU side and in the
CPU wait at `glGenerateMipmap`, not in a CPU retile. The proposals below are
ranked accordingly.

## What was measured

The glmark2 host harness, V3D build (`tools/hostharness/glmark2/README.md`,
"Profile"), glmark2 `22c527cb`, 800x600, 20 frames (counted frames 2..20),
on a tree whose sources equal main `a9bb71b5` (it differs from `b4e5f46e`
in a notes file only). Measured with `GM2_PROFILE=1`, `GM2_ENTRIES='^(shadow|refract)$'`,
run back to back on one machine, `YAKOGL_V3D_TFU=1` (the glmark2 image's
setting) and `=0` (the CPU path). `GM2_PERF` was not run: the counters below
already say where the time is. Host time: the ranking carries to the board,
the milliseconds do not.

| scene | TFU | cpu ms a frame | top entry points (wall ms a frame) | depth resolves / depth copy | colour chain | modelled TFU host ms a frame |
|---|---|---|---|---|---|---|
| shadow | 1 | 0.064 | `glDrawArrays` 0.030, `eglSwapBuffers` 0.007 | 0 / none (log: "sampled from its depth buffer (DEPTH24_X8): no CPU resolve, no tiled copy") | none | 0 |
| shadow | 0 | 0.155 | `glDrawArrays` 0.083, `eglSwapBuffers` 0.019 | 0 / none | none | - |
| refract | 1 | 0.178 | `glDrawArrays` 0.088, `glGenerateMipmap` 0.019, `eglSwapBuffers` 0.017 | 0 / none | render image, 11 levels, 11,001,856 bytes, one TFU job a frame, no tiled copy | 38.6 (1 job) |
| refract | 0 | 32.88 | `glGenerateMipmap` 23.77 (CPU box filter), `glDrawArrays` 12.37 (tiled copy of the chain) | 0 / none | CPU filter + tiled copy every frame | - |

Bytes: no depth copy in any run (it was 7,684,096 bytes); refract's tiled-copy
high-water is 12,058,624 bytes at TFU=1 (setup-time copies: the first sample
after a render registers the target, and a 1 MB texture), and the carve-out
high-water is 33,224,448 (shadow) and 61,409,280 (refract) of 100,134,912.
Neither scene raises a GL error or refuses a draw.

What this says: at the main tip **the resolve and the depth copy are zero in
both scenes, with and without the TFU**, so there is no host millisecond left
in them. Without the TFU, refract's colour chain is still a CPU filter plus a
tiled copy (36.7 ms of GL time), which is the TFU path's job and why the board
takes it. With the TFU, the host sees 0.18 ms; the 38.6 ms is the model's
host time for the TFU, not the CPU's.

The earlier numbers this study was commissioned on, for the record (all from
`docs/notes/pi4-glmark2-cpu.md`): main `c451ca46`, CPU path: shadow resolve
6.1 and depth copy 5.1 of 11.3 ms; refract copies 13.0 (colour chain ~7.7,
depth ~5.3), `glGenerateMipmap` 10.3, resolve 6.2 of 29.3 ms. Main
`0fbc81b0` against `262fa921` (TFU=1): shadow 7.09 -> 0.12 and refract
7.22 -> 0.23 cpu ms a frame.

Board, glmark2 from main `dcbfb30b` (same note, "On the board"), ms per
frame: shadow 175.28 -> 7.30, refract 196.03 -> 18.77; refract's GL-call time
11.59 ms is outside its timed steps (`glmark2-steps` 0.09 ms), shadow's
0.97. Both show "end waited" N of N: the frame is GPU-bound (swap 6.33 and
7.17 ms).

The bytes: the depth texture's copy was 7,684,096 bytes (1600x1200 x 4 B,
UIF-padded); refract's colour chain image is 11,001,856 bytes
(`tests/v3d_rt_sample.rs`); the tiled-copy high-water was 19,742,720 bytes
before and drops by 7,684,096 after (`pi4-glmark2-cpu.md`).

## Mesa's way, against this back end

Read from the vendored `third_party/mesa-26.2.1/src/gallium/drivers/v3d/
v3dx_format_table.c` (only the format tables are vendored: `v3d_resource.c`,
`v3d_blit.c`, `v3d_context.c` are not in the tree, and no `MESA_V3D_TREE`
was set here, so the statements about them rest on this repository's own
notes of them, `docs/online-shaderc/v3d-backend.md` and the source comments
that cite them, and on Mesa 26.2.1's code as those notes quote it; they were
not re-read for this study).

**Depth.** Mesa never copies a depth texture. A Z24S8 resource is one tiled
(UIF) image. The render writes it through the TLB's depth store, in the
`D24S8` render format, and the texture state points the TMU at the same
bytes with the `DEPTH24_X8` type (`v3dx_format_table.c:149`,
`FORMAT(S8_UINT_Z24_UNORM, D24S8, DEPTH24_X8, ...)`; `X8Z24_UNORM` the same
at `:150`). Option one of the brief's two readings is the one that applies:
the TMU samples the rendered tiled depth directly in a depth format, with no
TFU or TLB store into another layout. Z32F is `DEPTH_COMP32F` the same way
(`:152`), and stencil is a separate `S8` resource sampled as `RGBA8UI`
(`:151`, `:155-157`) because the TMU cannot read the stencil half. Nothing
retiles on the CPU because the CPU never owns a second raster form of the
texture: the resource's only storage is the tiled one, and a CPU read goes
through the transfer map (a detile into a staging buffer, only on a
`glReadPixels`-style access).

**Colour render targets.** Mesa keeps every render target UIF and samples
it as it is. glGenerateMipmap is the TFU job (`v3d_generate_mipmap` through
the TFU, `for_mipmap`), queued behind the render job; the kernel/scheduler
orders the two with a job dependency, not a CPU wait.

**This back end.** The GL core keeps every texture as raster rows (R32F for
depth), so this back end needed an adapter, and now has two:

- depth: `depth_sample` (`src/v3d/backend.rs`, `v3d_depth_sampled`,
  `texture::depth_buffer_state`) points the texture state at the D24S8
  buffer the jobs stored, as `DEPTH24_X8`: Mesa's way exactly. The R32F
  texels are never written for a rendered depth; `v3d_depth_resolve` is
  the CPU's own read path now (`depth_resolves` is 0 a frame for both
  scenes, `tests/depth_resolve_v3d.rs`).
- colour: the render image (`RtImage`, `rt_sample`, "Render targets
  sampled from their render image"): a second UIF image next to the raster
  storage, stored by a second `STORE_TILE_BUFFER_GENERAL` of RT 0, with
  the chain made by one TFU job (`v3d_generate_mipmap`). This is Mesa's
  layout with the GL core's raster storage kept alongside.

So where Mesa pays nothing for the raster form, this back end pays for it
in three places that are left, none of them a CPU retile:

1. every job rendering into a sampled target stores it twice (raster, then
   UIF): for refract 7.68 MB of extra TLB write a frame, terrain's four
   mipmapped targets (800x600, 256x256) likewise;
2. the depth texture's R32F storage (7.68 MB at 1600x1200) is allocated by
   the GL core and never written for a rendered depth: carve-out use with
   no reader (refract's 76.7 MB high-water of ~100 MB on the board);
3. `v3d_generate_mipmap` submits the open job and waits for it
   (`flush_job`, `wait_in_flight`) before queueing the TFU job, which
   Mesa does not (the TFU job waits for the render in the kernel). The
   running item v3d-tfu-mipmap-nowait owns this; it is the 11.6 ms of
   refract's GL time and terrain's five waits a frame on the board.

## Ranked fix proposals

The host's CPU cost for both scenes is already 0.12 and 0.23 ms a frame,
so no proposal saves host cpu ms of any size; the ranking is by the
board-side cost each removes, and by host-visible bytes and counters where
there is no host time. "Needs a board check" means the host harness cannot
show the effect.

1. **Don't write the raster storage of a sampled render target on every
   render (lazy raster).** Keep the UIF image as the live copy for a
   target that only the GPU reads; store the raster storage only when the
   GL core needs it (`glReadPixels`, `glCopyTex*`, `glTexSubImage2D`,
   window-system use), by detiling the image into the storage at
   `settle_texture` time the way `rt_chain_back` already does for levels
   1.. (`v3d_return_mipmap_chain`). Removes 7.68 MB of TLB write per
   refract frame and the same per sampled target in terrain and the
   desktop scenes. Functions: `emit_tile_list` / `v3d_job_target`
   (`uif_*`, drop the raster store while `RtImage::stored` and no CPU
   reader), `rt_chain_back` (extend to level 0), `rt_sample`, the GL core's
   `settle_texture` and the framebuffer's attach/`texture_target_use`
   paths in `src/gl/texture.rs`/`src/gl/framebuffer.rs`. Host repro:
   `tools/v3d/job_check.c` part 20 counts the stores through Mesa's CLE
   decoder (the second RT 0 store goes from "every job" to "none while no
   CPU reader"); `tests/v3d_rt_sample.rs` / the "render images" tests in
   `src/v3d/backend_tests.rs` for the CPU-read-after-render cases
   (glReadPixels, a level-0 `glTexSubImage2D`, a feedback target). The
   host harness shows no ms change (nothing is rendered). Risk: high; every
   CPU path that reads level 0 must find the storage current, and a missed
   one is a wrong picture. The window system's own buffers must stay out.
   **Needs a board check** (GPU time; refract's swap column 7.17 ms and
   `end waited`).
2. **Do not allocate the R32F storage of a depth texture whose depth is
   only ever sampled from its buffer.** Function area: the GL core's
   depth texture allocation (`src/gl/texture.rs` `glTexImage2D` for
   `GL_DEPTH_COMPONENT`, `resolve_depth`'s allocation) and
   `v3d_depth_sampled`: defer the 7,684,096-byte backing store to the first
   CPU read (`resolve_depth`) or write. Saves 7.68 MB of carve-out
   per depth texture (refract 76.7 -> ~69 MB of ~100 MB), no ms. Host
   repro: the carve-out high-water in `FINDINGS-v3d.md`'s GPU memory table
   and `tests/depth_resolve_v3d.rs`. Risk: medium (a texture that is
   written and read by the CPU later must still find its texels). Needs a
   board check only for the memory line; the host harness models the
   carve-out and shows it.
3. **The `glGenerateMipmap` wait** (`v3d_generate_mipmap`: `flush_job`,
   `wait_in_flight`): already planned as v3d-tfu-mipmap-nowait. Listed so
   the order is complete: it is the largest single board cost left of the
   three and is not duplicated here. Needs a board check.
4. **Count the remaining copy paths** (diagnostics, no change in the
   frame): `copy_image_for` is still the path for the first sample after a
   render, for a sample whose stamp moved, for a table full of render
   images (`RT_IMAGES` 8) and for the targets `rt_eligible` refuses. A
   "tiled copy made, by reason" counter at the frame-1 arena line would
   show, for any scene, which of them a scene still takes; today the
   reasons are only in log lines. Host repro: any harness scene with
   `GM2_PROFILE=1`. Risk: none. No board check.

## Host repro

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^(shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/vs-tfu1 v3d
YAKOGL_V3D_TFU=0 GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 \
  GM2_ENTRIES='^(shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/vs-tfu0 v3d
```

The first is the table's "after" row, the second the CPU path.
`GLMARK2_SRC` is glmark2 at `22c527cb0556f3a1ac4445aaa52cc532760928d5`
(README, "Command"). Expected on a healthy main: both scenes log "a
1600x1200 depth texture is sampled from its depth buffer (DEPTH24_X8)", no
"a stale tiled copy ... given back" line, `depth_resolves` 0, and cpu ms a
frame near the "after" rows.

## The copy counters

Proposal 4 above, built (`CopyReason` in `src/v3d/backend.rs`,
`v3d_copy_counters`; `tests/v3d_copy_reasons.rs`). Counts since init, printed
on two lines beside the "TFU mipmap at frame 1" line of the frame-1 arena
line and read by the host harness's log. Measurement only: no copy is removed.

"tiled copies for draws at frame N, by reason": the draw's binding was tiled
(a copy made, kept or in the job's arena) because the render image could not
serve it, or the draw was refused.

- `plain`: a texture that is no render target and no R32F (an upload).
- `r32f`: an R32F binding (the GL core's depth texels, or a float texture).
- `first-sample`: a render target sampled for the first time (registered).
- `cpu-write`: the CPU wrote the target's texels since the image was stored.
- `relayout`: the image was laid out again for more levels.
- `not-stored`: no stored image yet (or no image for want of memory).
- `no-chain`: a mipmap-filtered sample of an image whose levels 1.. are not
  GL's chain (no glGenerateMipmap since the render).
- `layout`: the TMU layout refused the image's size.
- `depth-refused`: a depth texture's buffer held no stored depth of its size.

"other copies at frame N":

- `upload`: a texture tiled when it was uploaded.
- `arena`: a draw's copy tiled into the job's arena (no kept copy); also
  counted under its draw reason.
- `stamp-replaced`: a kept copy of another stamp of the same storage freed
  for the new one.
- `stale-given-back`: a stale kept copy given back before a job opened.
- `image-store`: a job stored RT 0 into a render image besides the raster
  storage (the raster-to-UIF store).
- `level0-back`, `chain-back`: level 0, or the chain, untiled from the image
  into the storage for a CPU reader.

Pinned on the host (pairs are what moved in one frame): render then sample
`first-sample` once, then `image-store` a frame; render, glGenerateMipmap,
sample the same, with the chain by the TFU; a mipmap filter with no chain
`no-chain` and `image-store` a frame; a depth texture sampled after a depth
render nothing at all.
