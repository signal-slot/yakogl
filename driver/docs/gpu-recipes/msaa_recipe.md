<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 4x multisampling on BXS-4-64 (BVNC 36.53.104.796)

> **Verified on silicon (2026-09-14):** `driver/tools/pvrlab/gl_test.c` case 16 draws a triangle
> whose hypotenuse is the window diagonal into an `EGL_SAMPLES 4` pbuffer: every edge pixel
> resolves to exactly half coverage (0x7f), interior/exterior pixels are exact, and the
> single-sampled config keeps a hard edge. `gl_panel=2` spins the cube anti-aliased.

This GPU's ISP has 4 samples per pixel natively (`isp_samples_per_pixel = 4`), so 4x MSAA
keeps the 16x16-pixel tile; only the per-sample storage grows. Everything below mirrors
Mesa (`pvr_arch_job_render.c`, `pvr_arch_srv_job_render.c`, `pvr_arch_job_common.c`).

## HWRT dataset (`src/pvr_render.c` `hwrt_get(w, h, samples)`)
- Samples spread 2x2 (`pvr_get_samples_in_xy`): the tail-pointer and region-header
  buffers are sized from `tiles_x * 2`, `tiles_y * 2`; `rgn_header_size` (SIPF v2, per 2x2
  group) is 4x the single-sampled count (1024x600: 640 -> 2560).
- `te_screen`, `te_mtile1/2`, `isp_mtile_size` unchanged below 8 samples.
- `teaa` = `TE_AA.x2` (bit 0, >= 2 samples) | `TE_AA.y2` (bit 3, >= 4 samples) = 0x9.
- `multi_sample_ctl` = `PPP_MULTISAMPLECTL` sample positions in 1/16 px, x/y nibbles per
  sample: 4x = (6,2) (14,6) (2,10) (10,14) -> 0xEAA26E26; `flipped_multi_sample_ctl` has
  y mirrored (16 - y) -> 0x2A62AEE6. 1x is (8,8) -> 0x88.
- One HWRT dataset per (width, height, samples).

## Fragment kick registers
- `ISP_AA.mode` = 2 (`AA_4X`).
- `ISP_CTL`: 4x quadruples the partition size. The pixel width is the one
  `USC_PIXEL_OUTPUT_CTRL` says (4 output regs * 4 samples * 256 px = 4096 dwords), so only
  2 of the 8192-dword reserved partition store fit -> `pipe_enable = 1` (0x80031000 instead
  of 0x80035000), as Mesa derives both from the same width. `USC_PIXEL_OUTPUT_CTRL` is
  unchanged (the partition mask covers `max_partitions`). `pipe_enable = 3` (a 2-register
  pixel against the 4-register output control: four tiles in flight, twice what the store
  holds) was in the kick when gl_test 198's 1024-triangle 4x picture over 64x64 locked the
  fragment DM up (no USC slot in use, no page fault; 2026-10-10); a board run of case 198
  with `pipe_enable = 1` is the confirmation that this was the cause.
- PBE `REG_WORD0.downscale` (bit 56): the emit resolves the samples into the
  single-sampled memory surface (the EGL surface stays 8888 with tightly packed rows).
- A kick that may not be the frame's last (no swap: a full arena, the draw cap, a
  glFinish / glReadPixels before more drawing) emits the attachment a second time with
  `downscale` off into the back end's sample store (`MsStore`, `src/pvr/backend.rs`):
  the same output registers, mrt_index 1, the two-emit end-of-tile program, the
  target's stride and extent, four images' worth of bytes. The next kick's load quad
  binds the store as the target's multisampled image (`TEXSTATE_IMAGE_WORD0.smpcnt` =
  log2 4) and runs at the full sample rate, so each sample reloads itself. Unmeasured on
  the board (`docs/handoff/20261015-pvr-msaa-split.md`): the unresolved layout and the
  per-sample fetch.
- ZLS: the depth/stencil buffer is `align16(w) * 2` by `align16(h) * 2` samples of packed
  D24S8; `zlsextent` (tiles) is unchanged. 1024x600 needs 9.5 MiB, hence the 64 MiB carveout.

## GL/EGL side
- EGL config 2 = `EGL_SAMPLES 4`, `EGL_SAMPLE_BUFFERS 1`; the surface carries the sample
  count, `pvr_draw.samples` follows the bound target (FBOs stay 1) and a sample-count change
  starts a new frame. `GL_SAMPLES`/`GL_SAMPLE_BUFFERS` report it for the default surface.
- The depth/stencil-clear quad works unchanged (the FS output is written to every covered
  sample); the colour-load quad of a split frame reads the sample store per sample (above).
  `glSampleCoverage` / alpha-to-coverage go through the fragment shader's sample-kill
  variant and the per-draw control word (`fs_meta_word`, `src/pvr/backend.rs`).
