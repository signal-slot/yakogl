<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 multisample renderbuffers on all four back ends

Design, no code. `docs/notes/v3d-msaa-es3.md` (the V3D note) says what the GL
core makes of a multisample renderbuffer (1), how Mesa's v3d keeps four
samples (2), the V3D part (3), the GL-core part (4) and the dEQP rows (5). It
ends with a switch: the GL-core part must be gated per back end, because a 4x
FBO handed to a back end that cannot draw it is harmful. This note extends
that to the PowerVR (AM62P), the V3D (Pi 4), the GC7000UL (i.MX8MP) and the
Mali (NanoPC-T6), and fixes the decisions the plan's other items implement:
the capability and its feature (a), the contract between the GL core and a
back end (b), the four back ends (c), the dEQP rows (d), the open points and
the RC8 row draft (e). Reference: roadmap stage 4, plan #339.

Address is by function name; lines drift. Mesa is 26.2.1
(`third_party/mesa-26.2.1` holds only the format table; the full tree is the
deps cache's `mesa-pan/mesa-26.2.1`), described in words, not copied. What
is measured on a board is said so; everything else is a reading of code.

## 0. Where the code is today

- The GL core: `GL_MAX_SAMPLES` is 1 (`state.rs`); `glRenderbufferStorageMultisample`
  answers `samples > 1` with `GL_INVALID_OPERATION` and stores 1 as plain
  storage with `rb.samples = 1`; `GlesDraw.samples` is the window's
  `default_samples` or the literal 1 for an FBO (`draw.rs`);
  `glBlitFramebuffer` (`blit.rs`) is a CPU copy that "resolves" one-sample
  data; `glReadPixels` from a framebuffer whose `framebuffer_samples > 0`
  is `GL_INVALID_OPERATION` (ES 3.0 4.3.2, already right).
- The back ends draw 4x only for the window (`EGL_SAMPLES 4` configs), each
  its own way, and each keeps the samples of **one** surface between jobs:
  see (c). Nothing hands any of them a 4x FBO.
- `YakoglBackend::attachment_gone(id)` already exists: the GL core calls it
  with `ds_object_id(serial, texture)` (`context.rs`) when an object is
  deleted or its storage specified again (`respecified`), so a back end that
  keeps depth/stencil per object frees that buffer. The colour samples of a
  renderbuffer need the same event and the same identity (b).
- The AM62P's ms4 dEQP image passes the 16 ES 2.0 `multisample.*` cases on
  the 4x config (`tools/cts/RESULTS.md`, row "AM62P, ms4 image", from
  `e9901869a`, measured): the PowerVR's 4x draw, coverage, alpha-to-coverage,
  depth and stencil work on the window. What an FBO adds is the
  identity of the samples across jobs, the renderbuffer's pitch and
  format, and the resolve into `rb.mem`.

## (a) The capability and its feature

- `YakoglBackend` gets a last field `max_samples: u32`: the most samples a
  renderbuffer can have on this back end. **0 = single-sampled only, the
  default for every back end.** 4 is the only other value anyone sets (a
  request for 2 is rounded up, (b)). The value 1 is not used.
- A cargo feature **`msaa-fbo`**, off by default, is the only thing that
  makes a back end's table say 4: each back end's `backend()` writes
  `max_samples: if cfg!(feature = "msaa-fbo") { 4 } else { 0 }`, and only
  after that back end's 4x FBO host test passes (items `v3d-msaa-fbo`,
  `pvr-msaa-fbo`, `gc-msaa-fbo`, `mali-msaa-fbo`). Until its item lands a
  back end writes 0 whatever the feature says. `Cargo.toml` and
  `tools/check.sh`'s feature lists get the name in the GL-core item
  (`gl-msaa-cap`).
- **The back ends' 4x code is not behind the feature.** The feature only moves
  the table's number. A back end's host test hands its `draw` a
  `GlesDraw { samples: 4, .. }` directly (as `backend_tests.rs` modules do
  for the window), and needs no feature; the GL core's host tests
  (`tests/es3_msaa_gl.rs`) use a stand-in table with `max_samples: 4`. So
  every host test runs in an existing check.sh row, and a build without the
  feature is byte-identical in behaviour to today's.
- Board images keep the feature off until a board session has run the
  regression set with it (CLAUDE.md: hardware-facing change ships opt-in).
  With it off `GL_MAX_SAMPLES` stays 1 and the dEQP rows do not move on a
  board. The GC7000UL is the one back end where the feature also has to
  stand for the knob (c3).
- Adding the field touches every `YakoglBackend {` literal (the four back
  ends, `src/abi_layout.rs`, `src/testutil.rs`, about 55 tests): one item
  (`gl-msaa-cap`) does it, and it recounts the size assert in `abi_layout.rs`
  (232 today).

## (b) The contract between the GL core and a back end

1. **Counts.** `glRenderbufferStorageMultisample(samples)` with
   `0 <= samples <= max_samples` stores `GL_RENDERBUFFER_SAMPLES = 0` for 0
   and **4 for any request 1..=max_samples** (ES 3.0 4.4.2.1: "at least"; Mesa
   rounds 2 to 4 the same way). `samples > max_samples` stays
   `GL_INVALID_OPERATION`, which is why the 8-sample dEQP rows stay
   `NotSupported` (they read `GL_MAX_SAMPLES` first). With `max_samples == 0`
   a request of 1 keeps today's behaviour (`rb.samples = 1`: "sample buffers"
   for completeness, one sample in fact), and every other count is refused as
   today. `GL_MAX_SAMPLES` is `max(max_samples, 1)`.
   `glGetInternalformativ(GL_SAMPLES / GL_NUM_SAMPLE_COUNTS)` answers 4 / 1
   for a multisample-renderable format and 0 counts for the others.
2. **Formats.** The ES 3.0 core list (RGBA8, SRGB8_ALPHA8, RGB10_A2, RGB8,
   RGB565, RGBA4, RGB5_A1, R8, RG8, and the depth/stencil formats) plus the
   float ones the back end draws single-sampled
   (`backend::rt_format_drawable`; the 32F and R11F ones stay out, as in the
   V3D note (2)). Integer formats with `samples > 0` stay
   `GL_INVALID_OPERATION`: ES 3.0 has no multisample integer format, so no
   back end ever keeps or resolves integer samples. A format a back end then
   cannot draw at 4x is refused at the draw (`-ENOTSUP`, `GL_INVALID_OPERATION`,
   counted as a back-end gap), not at storage time.
3. **Storage.** The GL core allocates **only the single-sample storage**
   (`rb.mem`, resolved size, as today). It never allocates samples. Size cap:
   `glRenderbufferStorageMultisample` with `samples > 1` and
   `width * height > 2048 * 2048` is `GL_OUT_OF_MEMORY` (the samples are four
   times the surface per attachment, 64 MiB at RGBA8); the cap is the GL
   core's, one constant, see (e).
4. **Draw.** `GlesDraw.samples` is `framebuffer_samples` for an FBO (4 or 1),
   in the draw and the clear builders alike; sample coverage, alpha-to-coverage
   and the sample mask then apply to an FBO through the same field. Every
   attachment of one FBO has the same count (completeness, below).
5. **Identity.** The back end keeps the samples itself, **keyed by the
   attachment, not by the address**: a freed `rb.mem` can come back at the
   same GPU address and size, so (address, w, h) (what the V3D's `MsBuf` uses
   today) would resurrect stale samples after a same-size respecification.
   Colour has no id in `GlesDraw` today (depth has `depth_id`/`depth_gen`).
   Name: `GlesDraw::target_id` (and `GlesDrawTarget::id` for `targets[1..]`)
   is `ds_object_id(rb.serial, false)` for a renderbuffer attachment, 0 for
   a window, a texture or an EGLImage; `target_gen` / `GlesDrawTarget::gen` is
   its `storage_gen`. Both are appended to the C-visible structs with their
   `abi_layout.rs` asserts, in `gl-msaa-cap`. A zero id means "not a
   multisample renderbuffer, the back end's own surface rule applies" (the
   window). Depth/stencil already carry `depth_id`/`depth_gen`/`stencil_*`.
6. **Dropping.** There is **no new back-end entry**: `attachment_gone(id)`
   is extended in meaning, "the contents of GL object `id` are gone, free
   what you keep for it, depth/stencil and colour samples". The GL core
   already calls it on deletion and in `respecified`; it also calls it from
   `glInvalidateFramebuffer` / `glInvalidateSubFramebuffer` (whole-attachment
   or not: a sub-rectangle invalidation is allowed to keep everything, so
   the GL core calls it only for a whole-attachment invalidate of a 4x
   attachment) and `glDiscard`-like paths. The back end may free after the
   kicks that still read the buffer have completed (`gpu_free`'s contract);
   an id it does not know is a no-op. `storage_gen` stays a guard in the draw
   (a gen that differs from the slot's drops the slot).
7. **End of a job: the resolved image is in `rb.mem`.** Whatever a back end
   keeps, when a job that drew into a 4x renderbuffer is complete (its fence
   passed), `rb.mem` holds the **resolved** colour of everything drawn so far.
   A back end resolves at the end of **every** job that touches the
   renderbuffer, not only at the last: one resolve store a tile per job is the
   price of a contract with no "is this the last job" question. The GL core
   flushes the pending job of the read renderbuffer the way `glReadPixels`
   does (`flush` + the existing `wait_kick`/`finish` path, then the read).
8. **Depth/stencil.** The back end keeps the depth and stencil samples per
   object (`ds_id`-style keys it has for the window), keyed by `depth_id` /
   `depth_gen`. The resolved depth or stencil is read through the existing
   entries (`depth_resolve`, `stencil_resolve`, `ds_write`), which at 4x
   return **one sample per pixel** (sample 0 of the pixel; ES 3.0 4.3.3 leaves
   which sample of a depth or stencil resolve to the implementation). Each
   back end item pins what its hardware stores and says so.
9. **Resolve and the other readers.** Blit, read-back and sampling then
   read resolved pixels on the CPU:
   - `glBlitFramebuffer`, a multisample read to a single-sample draw: after the
     flush above, the existing CPU copy of `rb.mem`, plus the ES 3.0 4.3.3
     errors the item `gl-msaa-resolve` adds (a scaled, format-mismatched or
     region-mismatched resolve, and a draw framebuffer with sample buffers,
     are `GL_INVALID_OPERATION`).
   - **Implicit resolve.** ES 3.0 has none for FBOs: no multisample
     textures, `glReadPixels` / `glCopyTexImage2D` / `glCopyTexSubImage*`
     from a multisample read framebuffer are `GL_INVALID_OPERATION` (the
     existing `framebuffer_samples(c) > 0` check, kept), a renderbuffer is not
     sampled. The blit is the only way out, so "implicit resolve" is the
     job-end resolve of item 7 (and, for the window, the swap's, which each
     back end already has). EXT_multisampled_render_to_texture is not part
     of ES 3.0 and not covered, though the Mali's frame is that form (c4).
   - A blit *into* a 4x framebuffer, and a 4x to 4x blit, are
     `GL_INVALID_OPERATION` (exists).
10. **Completeness.** `check_status` compares the samples of all attachments:
    all renderbuffers with the same count, and a texture (count 0) mixed with a
    4x renderbuffer is `GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE` (the rule
    exists; with real counts the depth/stencil attachments take part).

What the contract costs a back end, in one sentence each: it must keep a
sample store per live 4x attachment (not per window surface), resolve at
every job end, and fall back to "start from the resolved image in every
sample" when it has no slot, which is lossy only for edge pixels drawn in an
earlier job (a back end with fewer slots than live attachments loses those
edges; it never loses correctness of the resolved colour).

## (c) The four back ends

Common numbers: a 4x target is 4 times the single-sample bytes per colour
attachment, at the format's own bytes a pixel; depth/stencil 4 times as well.
dEQP's renderbuffers (`fbo.msaa` 128x128, `multisample.*` 64..256 a side, the
`rasterization.fbo` ones 256x256) are small: 128x128 RGBA8 is 256 KiB of
samples; 1024x1024 is 16 MiB; the cap of (b)3 is 64 MiB.

### (c1) PowerVR (BXS-4-64, AM62P, `src/pvr`)

**Sample storage.** A 4x frame draws with `CR_ISP_AA.mode = AA_4X`
(`isp_aa`) and `CR_ISP_CTL` as `isp_ctl(samples, pixel_regs, ..)`, whose
`pipe_enable` is `isp_tiles_in_flight - 1`. The colour samples across kicks
live in `MsStore` (one slot, `ms_store_ready`, `ms_bytes`: samples times the
target's pitch, height and bytes a pixel): a kick that may not be the frame's
last emits each tile twice, the resolved colour into the target
(`downscale` on the PBE store, the end-of-tile resolve) and the unresolved
samples into the store as the TPU's `smpcnt` multisampled image; the next
kick's load quad binds the store and runs at the full sample rate
(`fs_sample_rate`). It is keyed by `(target_gpu, w, h, pitch, format,
samples)` with a `valid` flag. Depth is per `DsKey` at 4x (`ds_get(.., samples)`,
`ds_bytes`: twice the width and height, so 4x the plane); depth and stencil
are separate planes. **For an FBO:** the key becomes `(target_id,
target_gen)`, the slot count at least 2 (a pool, LRU), and `valid` drops
on `attachment_gone`; a 4x FBO's kick is "may not be the last" unless it ends
a swap, which an FBO never does, so **every FBO kick takes the two-store
path** (the cost: an unresolved write beside the resolved one every tile of
every FBO job; the sample store traffic is the 4x bytes).
The unresolved layout of the PBE against the TPU's `smpcnt` image and a
per-sample fetch of it are **unmeasured on the board** (#1289); a 4x FBO
rendered across kicks stands on that, a 4x FBO drawn and blitted in one kick
does not.
**Tile and memory.** 16x16 tiles. A USC partition is `pixel_regs x samples
x 256` dwords, the ISP has at most 6 tiles in flight: 6 at 1x with four
registers, **2 at 4x** (the BXS-4-64's reserved partition store), and `max(1)`
guards the rest. gl_test case 198's lock-up (1024 a2c triangles over 64x64)
was the over-commit of this number (`isp_tiles_in_flight`'s history); an FBO
at 4x with many objects a tile is the same shape and the same fix. A wider
pixel (`pixel_regs` 8, several attachments) at 4x is 8192 dwords a tile:
**multi-attachment 4x stays refused by name on the PowerVR** (one colour
attachment, plus depth/stencil, as `fbo.msaa` needs).
**Colour formats.** The GL core stores every format of 8 bits or less as RGBA8888
on the PowerVR (so the whole 8-bit group of (b)2 is the one existing 4x RGBA8
case), SRGB8_ALPHA8 draws (`SRGB_RT`, the PBE's gamma on store: whether the
PBE's downscale averages before or after the gamma is unmeasured; dEQP
compares with a tolerance), RGB10_A2 is stored RGBA8 on this back end (two bits
lost, `backend-caps.md`): the `rgb10_a2` row is a risk, not a promise.
R16F, RG16F, RGBA16F follow the single-sample float RT rows.
**Resolve and implicit resolve.** The job-end resolve is the PBE's `downscale`
store (item 7 of (b)); no TPU resolve pass is needed. (Mesa's pvr is a Vulkan driver; this note did
not read how it resolves a multisample attachment.)
Depth/stencil blit reads `ds_write` / `stencil_resolve` / `depth_resolve`,
which exist; at 4x they must read the 4x plane (`ds_bytes` layout) and take
sample 0 (the 2x2 spread's first element).
**dEQP rows** (am62p-gles3-notpass.tsv, same counts as the Pi 4): see (d). The
ms4 image's 16 passes say the window's 4x draw is conformant; fbo_4_samples
is the same draw into an FBO.

### (c2) V3D 4.2 (Pi 4, `src/v3d`, `driver/src/v3d`)

Everything of the V3D note applies (sections 2 and 3, which this note does not
repeat); the decisions here are the interfaces.
**Sample storage.** `MsBuf` (`MS_BUFS = 2`, keyed by target address) becomes
keyed by `(target_id, target_gen, RT index)` with the window keeping its
address key (id 0), `MS_BUFS` at least 6, and the per-job guarantee that the
RTs of the running job are never the victim (V3D note (3)). The samples are
a single-level UIF at twice the width and height in the format's own cpp
(`v3d_tex_layout_msaa(w, h, cpp, ..)`, not the literal 4). Depth/stencil:
`ZsBuf` already has an `msaa` flag; an FBO's depth buffer is identified by
`depth_id` while the window's follows the surface: the host test checks
`ds_persist` and eviction with an FBO that outlives a job.
**Resolve.** The tile store does both: `ALL_SAMPLES` into the sample buffer and
`4X` decimate into the raster target, per RT, in every job (item 7 is the
V3D's native shape). A load of RT n from a buffer with no valid samples starts
from the clear colour: `valid` false after `attachment_gone` and after a swap
of a window.
**Tile and memory.** One bit for the whole job: every RT, depth and stencil
are 4x. `v3d_frame_tiling_bpp` already gives the Mesa table (one RGBA8 4x:
32x32; a 64-bpp RT at 4x: 32x16; 4 RTs at 4x with a 128-bpp one: 8x8). Memory is
4x the pixels times cpp per RT: 16 MiB for a 1024x1024 RGBA8 RT, four 64-bpp
RTs at 1024x1024 are 128 MiB.
**Formats.** The 8-bit group is internal type `8`; SRGB8_ALPHA8, RGB10_A2 and
the 16F rows are `16F` at 64 bpp and TLB-resolvable (`v3d_format_supports_tlb_resolve_and_blend`);
R11F and 32F stay refused. The two refusals of `draw_call` ("4x multisampling
of several colour attachments", "of an ES 3.0 colour format") and the guards
in `v3d_job_begin` go, per V3D note (3).
**Implicit resolve.** The resolve store above; `glBlitFramebuffer` then
reads `rb.mem`. Depth/stencil resolves at 4x use the Z/S store's decimate or
sample 0 (the item pins it against `ZsBuf`'s `zs_decimate`).
**Owner and gate.** `src/v3d/**`, `driver/src/v3d/v3d_job.[ch]` (+ the
Rust mirror `src/v3d/ffi.rs`) and `tests/es3_msaa_v3d.rs` are plan #330's;
items `v3d-msaa-job` and `v3d-msaa-fbo` wait for it. Un-ignore: the
GL-core tests (`max_samples_is_four`, `internalformat_counts_four`,
`rgba8_4x_draw_and_resolve`, `other_es3_formats_4x_storage`) are the GL
core's and need the stand-in or the feature; the V3D tests
(`srgb8_alpha8_*`, `two_rgba8_*`, `rgba8_and_srgb_*`) need `v3d-msaa-job`.

### (c3) GC7000UL (i.MX8MP, `src/vivante`)

**Sample storage.** The 4x draw is a **scratch surface twice the target on
each axis** (`ms_scale` 2, `ms_pitch`: `align_up(2w, 64) * bpp`), SUPER_TILED, in
the one `g().msaa` allocation (`ensure_msaa`), that the RS averages into the
target at frame end (`emit_resolve`: DOWNSAMPLE_X/Y, tiled source, window in
source samples), `g.ms_dest_*` naming the target. There is **one** scratch for
every target (it grows, `msaa_bytes`) and nothing reloads it: a second frame
on the same target starts from garbage unless cleared, a different target
overwrites it. So today the GC7000UL keeps no samples across jobs; that is
the window's rule, valid only because the swap makes the content undefined.
**For an FBO:** a pool of scratch surfaces keyed by `(target_id, target_gen)`
(LRU, at least 2, size `align_up(2w,64) * 2h * bpp` each), so that a job that
does not clear continues in the target's scratch, and the RS resolve at
every job end (it already is per frame) leaves `rb.mem` current. A target with
no scratch yet (first draw, or evicted) and no whole clear starts from an
undefined 4x surface: the back end **clears it from the resolved image with a
draw** (the existing clear/copy quad drawing `rb.mem` as a texture into the
scratch at 4x; the RS cannot upsample), or, when that is not built, refuses
the draw (`-ENOTSUP`) rather than draw on garbage: this choice is the item's
(open point 4). Depth: `ds_cache` keyed per object with `ms` already
scales the buffer (`ds_w * ms`), the PE writes it row-linear under
SINGLE_BUFFER 1 (measured, `backend.rs`) or supertiled per Mesa; the item
decides with (E) of `gc-msaa-addresses.md`.
**State, measured.** 4x on this core works only with the whole of Mesa's
state: bit 1 (PS_INPUT_COUNT and temporaries one more, PE_LOGIC_OP UNK24 = 5)
and bit 4 (PE_LOGIC_OP SINGLE_BUFFER 2 for the supertiled target); neither
alone (round 3, arm 19, `gc-msaa-addresses.md`: case 16 passes on the board,
2026-10-15/16, `aed962e2`). The remaining differences, not yet tried: no tile
status (D), depth not supertiled (E), `emit_resolve`'s 16-bit format
(RGBA4/RGB5_A1 pick A4R4G4B4 whatever, RGB565 is right). No 4x
`multisample.*` dEQP run exists on the GC7000UL.
**Gate.** 4x is today behind the `gc_debug_msaa` knob (`GC_MSAA_ENABLE` false,
`msaa_on()`), and the knob is also what offers the 4x EGL config. With
`msaa-fbo`: `msaa_on()` is true and bits 1 and 4 are on (arm 19 is the only
measured arm), the knob stays for the other arms. This also turns the 4x
window config on for a `msaa-fbo` build (open point 3).
**Tile and memory.** No tiles (the PE writes by pixel): the cost is the
scratch, 4x the pixels (twice each side, padded to 64 wide) plus the
depth at `ds_w * 2`. Constraint: the RS resolves widths in multiples of 16 and
heights (in source samples) multiples of 8, i.e. `target_w & 15 == 0 &&
target_h & 3 == 0` (`ms_resolvable`); otherwise the back end **draws
single-sampled** (it logs once). For an FBO the GL core has promised 4 samples,
so such a draw is a coverage loss (edges are full-coverage), not a wrong
resolved colour; dEQP's `fbo.msaa` and `multisample.*` renderbuffers are
128, 256 or a power of two a side, all fit. Pad `rb.mem` or refuse 4x on odd
sizes (open point 4).
**Formats.** RGBA8 group is the one 4x case (stored RGBA8888 for the 8-bit
formats; RGB565 and 16-bit formats are the format-specific RS rows
`ms_rs_format`: R5G6B5 for RGB565, A4R4G4B4 for the others at 2 bytes);
R16F/RGBA16F follow the single-sample float rows; one colour attachment at
4x (as PowerVR).
**Resolve and implicit resolve.** RS downsample into the target at frame end,
plus the contract's item 9 (CPU copy of `rb.mem`). Mesa's etnaviv always pairs a
4x render target with a compressed tile status and resolves with the RS
(`etna_try_rs_blit`, window in source samples) or, for a blit it cannot do, a
shader; yakogl has no tile status and no shader resolve, and does not need one
for item 7.
**Owner.** `src/vivante/**` is #328's (most of it) and `backend.rs` /
`backend_tests.rs` #333's: item `gc-msaa-fbo` waits. #335 holds
`src/vivante/texture.rs`, `wide_target.rs`, `src/backend.rs` and
`tests/es3_*gc*.rs`.

### (c4) Mali (Valhall G610, NanoPC-T6, `src/mali`)

**Sample storage.** The Mali's native 4x is the **render-to-texture form**:
the tile buffer holds 4 samples and the writeback averages them into
single-sampled memory (`MALI_MSAA_AVERAGE` for an image of 1 sample viewed as
4: `mali_sampling_mode` in Mesa's `pan_desc.c`; the
`EXT_multisampled_render_to_texture` path of panfrost, `backend.h:221-224`).
Mesa keeps a real multisample image with `MALI_MSAA_LAYERED` (all samples,
one layer a sample, `nr_samples > 1` on the image). Our frame builder has both:
`SampleBuffer` (`sample_store`, `resolve_frame`) writes the samples
(`Writeback MSAA Layered`) into four planes of the largest 4x target and, **in the
same kick**, averages them into the target; the next frame of that target that
starts from its image draws the planes back with the per-sample preload
program (`preload::colour_samples`), as Mesa reloads a multisampled
surface (`tools/cts/log/2026-10-28-splitref`, gl_test case 214, measured).
Today **one** store serves the last 4x target, and `SampleKey` is (gpu, w, h,
row_stride, format). **If a frame cannot keep its samples** (no store memory,
a row stride that is not a multiple of 64, the per-sample preload not
built: `sample_store` returns `None`), the frame averages into the target
only and the next frame reloads the resolved colour into all four
samples: the conformance cost is the edge pixels of earlier jobs, as in (b);
it is what a pure render-to-texture 4x would always cost, which is why the
store exists.
**For an FBO:** the store becomes a pool keyed by `(target_id, target_gen)`
(at least 2, each 4 planes of its target), `holds` per slot, dropped on
`attachment_gone`. `GlesDraw::samples` is today 1 or 4 only
(`check_target`: `NOT_BUILT_SAMPLES` for other counts), and the comment of
the store says "an FBO is single-sampled": that line is the one to rewrite.
**Depth/stencil.** A 4x frame's depth/stencil is kept with 4 samples **on chip
only** (the ZS target's `MSAA` average, case 207 `msaazs`, host test
`a_4x_depth_frame_is_the_board_s`): **the depth samples do not survive a
frame**; the next frame preloads the single-sample linear D24S8 plane into
all four. For `fbo.msaa`'s depth/stencil rows (one draw, one blit) this is
enough; for a 4x FBO drawn over several jobs with depth it is the same edge
loss and no worse than a store-time resolve; a depth sample store is an open
point, not part of the first item.
**Tile and memory.** From Mesa's `pan_select_tile_size`: the colour
allocation at 4x RGBA8 is 16384 bytes (effective tile 1024 pixels), and with a
4x depth attachment the effective tile drops to 512 and the allocation to
8192 (`docs/notes/mali-frame-refusals.md` section 4; read from Mesa, the board's
Mesa 25.0.7 gives 1024/16384). RGBA16F: 1024 / 8192 at 4x. The memory is four
planes a store (`4 * row_stride * height`) for the colour samples.
**Colour formats and targets.** RGBA8, RGB8 and what the frame builder draws
single-sampled (`frame_formats_tests.rs`); the same one colour attachment limit
as the others (`frame_mrt_tests.rs` is the single-sample several-target form;
a 4x several-target frame is not built).
**Resolve and implicit resolve.** The same-kick averaging (`resolve_frame`) is
item 7 as it stands: `rb.mem` is resolved after every kick. A blit reads it
after the fence. There is no sample-by-sample resolve engine: Mesa's blit of a
multisample image goes through the blitter, not through this path, and this
note did not read it.
**Owner.** `src/mali/**` is the Mali plans' (#336, #332, #327): item
`mali-msaa-fbo` waits for them.
**Host evidence.** `src/mali/frame_msaa_tests.rs` compares our 4x frame with
Mesa's, byte for byte, for the colour, depth, stencil, alpha-to-coverage and the
4x blend shader (cases 16, 207, 198, 211) on the board's dumps: the draw is
settled on the window's shape; an FBO changes the target (a texture-style
renderbuffer's stride) and the store's identity, which the item's host test
pins with the same builder (`Target { samples: 4 }` from a renderbuffer).

## (d) Which dEQP rows each part moves

Counts from `docs/es3/am62p-gles3-notpass.tsv` (PowerVR, the 1x images) and
`docs/es3/pi4-gles3-fails.tsv` (V3D); the two files give the same count for
each group below (`Fail` / `NotSupported`). `docs/es3/host-gles3-tip.tsv` has
only group totals per back end, **HOST-ONLY (render verdicts meaningless)**:
`functional.multisample` is 9 pass / 7 fail / 48 NotSupported on all three of
powervr, vivante and v3d, and `functional.fbo` 534/1239/255, 479/1294/255 and
456/1317/255 (pass/fail/NotSupported). The 48 NotSupported are `multisample.default_framebuffer`
16 + `fbo_4_samples` 16 + `fbo_8_samples` 16; the 16 `fbo_max_samples` run at
1 sample today. There is no dEQP-GLES3 inventory for the Mali.

| group | rows | now (boards) | GL core (gl-msaa-cap, gl-msaa-resolve) | back end |
|---|---:|---|---|---|
| `fbo.msaa.{2,4}_samples.*`, colour | 16 each | 9 Fail (r8 rg8 rgb8 rgb565 rgba4 rgb5_a1 rgba8 srgb8_alpha8 rgb10_a2) + r16f rg16f Fail, rgba16f r11f r32f rg32f rgba32f NotSupported | storage, blit-resolve | each back end's 4x draw: 9 plausible on all four; `rgb10_a2` is a risk on the PowerVR and GC7000UL (stored RGBA8); r16f/rg16f follow the float RT rows; the 5 NotSupported stay |
| `fbo.msaa.{2,4}_samples.*`, depth/stencil | 6 each (depth16/24/32f, depth24_stencil8, depth32f_stencil8, stencil8) | Fail | storage, depth/stencil blit via `depth_resolve` / `stencil_resolve` | 4x Z/S per object: exists for the window on all four; the Mali keeps no depth samples across frames (c4) |
| `fbo.msaa.8_samples.*` | 22 | NotSupported | none | none: `max_samples` 4 keeps them |
| `fbo.completeness.samples.*` | 7 | NotSupported | **GL core only** (counts above 1, mixed counts, tex+rbo) | none |
| `fbo.invalidate.{whole,sub}.unbind_blit_msaa_{color,depth,depth_stencil,stencil}` | 8 | Fail | `attachment_gone` on invalidate, blit | 4x draw and resolve |
| `multisample.fbo_4_samples.*` | 16 | NotSupported | `GL_MAX_SAMPLES` 4 | the coverage, a2c and mask of an FBO draw: PowerVR measured on the window (ms4 image), V3D/GC/Mali by their host references |
| `multisample.fbo_max_samples.*` | 4 Fail (12 pass at 1) | Fail | max 4 | same as above |
| `multisample.fbo_8_samples.*` | 16 | NotSupported | none | none |
| `multisample.default_framebuffer.*` | 16 | NotSupported | none | an EGL 4x config of the dEQP image (plan #320's switch); the AM62P has its ms4 image |
| `rasterization.fbo.rbo_multisample_4.*` | 12 | NotSupported | max 4 | 4x draw (lines, wide lines, points, fill rules) |
| `rasterization.fbo.rbo_multisample_max.*` | 2 (`lines_wide`) | Fail | max 4 | as above; wide-line risk (also Fail at 1 sample for a reason this note did not measure) |
| `state_query.integers.max_samples_{getfloat,getinteger64}` | 2 | Fail | **GL core only**, only with a table that says 4: **a board with the feature off stays Fail** | none |
| `negative_api.buffer.blit_framebuffer_multisample` | 1 | Fail | resolve errors | none |

Rows moved with both parts, as the V3D note counts them: 34 `fbo.msaa`
(17 each of 2 and 4 samples), 7 completeness, 8 invalidate, 20 `multisample.fbo_*`
(4 and max), 14 `rasterization.fbo.rbo_multisample_*`: about 83; 2 to 3 with the GL
core alone (`max_samples_*`, maybe `blit_framebuffer_multisample`). This is
**per back end where the back end's 4x draw conforms**: the rows are the
same on the PowerVR and the V3D files; the GC7000UL has no 4x dEQP at all,
so its 83 are unmeasured until a board session runs it. The 8-sample and
default-framebuffer rows (about 60) and the 10 float NotSupported rows do not move.
Why the 2- and 4-sample colour cases are `Fail` and the 8-sample ones
`NotSupported` is read from the status, not measured: a request above the
`GL_MAX_SAMPLES` 1 gets `GL_INVALID_OPERATION`; the 8-sample cases check the
limit first.
Every number is for a build with `msaa-fbo`: the host dEQP rerun (item
`deqp-rerun`, `tools/cts/host-deqp-run.sh`, README section 2b) is run with
it per back end and records HOST-ONLY counts of statuses (NotSupported to
Fail/Pass moves), not render verdicts; board counts wait for the board
sessions (#313, #316, #320).

### (d1) HOST-ONLY baseline of the three groups, `msaa-fbo` absent (2026-11-10)

**HOST-ONLY: the linux OSAL keeps the command stream and every read-back is
black, so no Pass or Fail from a case that compares pixels is a render
verdict; the statuses that do mean something are NotSupported and the API
errors (`Got GL_INVALID_OPERATION`).** Not a board result: nothing here belongs
in `tools/cts/RESULTS.md`.

- yakogl commit `6770479e` (the library as built, `GL_MAX_SAMPLES` 1; the
  script printed `library and harness from 6770479e`).
- VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, `fetch_sources.py`, the
  `targets/yakogl` files of `tools/cts/cts-yakogl-target.patch`.
- Commands (README section 2b, `MODULE=gles3`, one `OUT` per run), for
  `GPU` in `vivante powervr v3d` and `G` in the three filters below:

```sh
tools/cts/host-deqp-run.sh --module=gles3 --gpu=$GPU --cts=<VK-GL-CTS> \
  --out=build/host-deqp/msaa-$GPU-off-<name> dEQP-GLES3.functional.<G>
# G: fbo.msaa.   multisample.fbo_max_samples.   rasterization.fbo.rbo_multisample_max.
```

The three back ends gave the same counts and the same messages in every
group (a per-back-end table each, as the plan asks):

| vivante (`GPU=vivante`) | cases | Pass | Fail | NotSupported | QualityWarning |
|---|---:|---:|---:|---:|---:|
| `fbo.msaa.` | 70 | 0 | 38 | 32 | 0 |
| `multisample.fbo_max_samples.` | 16 | 9 | 7 | 0 | 0 |
| `rasterization.fbo.rbo_multisample_max.` | 12 | 5 | 7 | 0 | 0 |

| powervr (`GPU=powervr`) | cases | Pass | Fail | NotSupported | QualityWarning |
|---|---:|---:|---:|---:|---:|
| `fbo.msaa.` | 70 | 0 | 38 | 32 | 0 |
| `multisample.fbo_max_samples.` | 16 | 9 | 7 | 0 | 0 |
| `rasterization.fbo.rbo_multisample_max.` | 12 | 5 | 7 | 0 | 0 |

| v3d (`GPU=v3d`) | cases | Pass | Fail | NotSupported | QualityWarning |
|---|---:|---:|---:|---:|---:|
| `fbo.msaa.` | 70 | 0 | 38 | 32 | 0 |
| `multisample.fbo_max_samples.` | 16 | 9 | 7 | 0 | 0 |
| `rasterization.fbo.rbo_multisample_max.` | 12 | 5 | 7 | 0 | 0 |

Failures by message, the same on all three back ends:

| group | count | message |
|---|---:|---|
| `fbo.msaa.` | 34 | `Got GL_INVALID_OPERATION at es3fFboTestCase.cpp:154` (the 2- and 4-sample `glRenderbufferStorageMultisample`, samples > 1) |
| `fbo.msaa.` | 4 | `compareThreshold(color, green)` at `es3fFboMultisampleTests.cpp:346` (`renderbuffer_resize.*`; a black read-back, meaningless) |
| `multisample.fbo_max_samples.` | 7 | `Failed` (pixel comparison against black, meaningless) |
| `rasterization.fbo.rbo_multisample_max.` | 4 / 2 / 1 | `Incorrect rasterization` / `Found invalid pixel values` / `Found invalid pixels` (black read-back, meaningless) |

**Why the NotSupported ones end so.** The 32 `fbo.msaa.` NotSupported are 20
`Sample count not supported` and 12 `Format not supported` (the float colour
formats). `FboTestCase::checkSampleCount` (`es3fFboTestCase.cpp:363`) only
asks `glGetInternalformativ(GL_SAMPLES)` for a count above the spec minimum
of 4, so the 8-sample cases end NotSupported at once, while the 2- and
4-sample cases skip the check, call `glRenderbufferStorageMultisample` and
get `GL_INVALID_OPERATION` from the core (`GL_MAX_SAMPLES` 1): those are the
34 Fails. Both move with the capability: 4 keeps the 8-sample ones
NotSupported and turns the 34 into allocations. The `fbo_max_samples` and
`rbo_multisample_max` cases show no NotSupported and no API error; what they
request from `GL_MAX_SAMPLES` was not read for this baseline, and their
Pass/Fail split is black-readback noise that moves only on a board. The
groups of (d) that are NotSupported on the boards for `GL_MAX_SAMPLES` 1
(`multisample.fbo_4_samples`, `rbo_multisample_4`, `completeness.samples`)
are not among these three filters and are not re-measured here.

(d) counts rows of the board inventories; this counts the cases each filter
selects as the script ran them (`fbo.msaa.` 70, `fbo_max_samples.` 16,
`rbo_multisample_max.` 12), so the totals differ from (d)'s Fail counts for
the same group names.

### (d2) HOST-ONLY rerun with `msaa-fbo` (to be filled by item `deqp-rerun`)

Same three tables per back end and the same filters, one `OUT` per run, with
the option of README section 2b on; the Pass/Fail/NotSupported moves against
(d1) are the point, not the render verdicts. Placeholder: no rerun yet.

## (f) Resolve semantics per GPU

What a 4x-to-1x resolve does on each back end, as Mesa 26.2.1 does it (a
reading of code, MIT parts only; nothing here is measured on a board unless a
cell says so) and where yakogl stands. Columns: the resolve operation, the
space the colours are averaged in, Mesa `file:line` (under `src/` of the
Mesa tree), and yakogl `file:line` or **board only**. **board only** means
Mesa does not say and the hardware has to be read back on a board; it is
not a pass. The `glBlitFramebuffer` resolve averages the samples of a
colour (integer formats take one sample, as Mesa's blitter does) and
leaves depth and stencil to one sample; sample 0 is the plan's choice, as
Mesa's.

Mesa lines found moved against the earlier read, corrected here:
`v3dx_rcl.c` store decimate is 158 / 165 / 167 (ALL_SAMPLES / 4X /
SAMPLE_0), not 166-168; the depth blit job is `v3d_tlb_blit`
(`v3d_blit.c:512-620`, the Z/S store at 613), its `FIXME: support
depth/stencil` at 405 belongs to the *fast* path only.

### (f1) V3D (`gallium/drivers/v3d`, yakogl `driver/src/v3d/v3d_job.c`)

The tile buffer resolves in the TLB when its internal type is 8-bit or 16F
(`v3d_format_supports_tlb_resolve_and_blend`, `v3d_formats.c:148-166`; the
caller falls back to a shader blit otherwise, `v3d_blit.c:382-388`).
sRGB and RGB10_A2 are both 16F at 64 bpp, with the conversion at tile
buffer load and store (`v3dx_format_table.c:250-259`), so the average is
taken on the halves: linear for sRGB, the unorm value for RGB10_A2.

| row | resolve operation | averaging space | Mesa | yakogl |
|---|---|---|---|---|
| RGBA8 | TLB store, `DECIMATE_MODE_4X` from a 4x blit buffer | tile buffer, 8-bit normalised | `v3dx_rcl.c:159-166` (4X at 165), `v3d_blit.c:382-388`, `v3dx_format_table.c:65` | `v3d_job.c:1136` (4X store of RT0), kept samples `:1118-1121` and load `:1056-1059`, both `RGBA8` literals |
| SRGB8_ALPHA8 | same store; the tile buffer holds 16F linear | linear (decoded at load, encoded at store) | `v3dx_format_table.c:67`, `:250-259`; `v3d_formats.c:160-166` | typed target: `rt_internal` `v3d_job.c:155-161` (16F, 64 bpp, clamp NORM); the 4x path refuses typed targets (`v3d_job.c:311-316`) and its samples are the `RGBA8` literal at `:1118` |
| RGB10_A2 | same store, 16F | unorm value (no gamma) | `v3dx_format_table.c:71`, `:252`, `:255-259` | `rt_internal` `v3d_job.c:162-170`; same refusal and literal as the sRGB row |
| depth (D24S8) | `v3d_tlb_blit`: Z store with `decimate SAMPLE_0` into a 1x D24S8 buffer (`resolve_4x` is false for Z/S) | none: sample 0 | `v3d_blit.c:512-620` (store at 613), `v3dx_rcl.c:165-167`, `:286-309`, `:105-107` (load of the 4x source `ALL_SAMPLES`) | today `zs_decimate` = `ALL_SAMPLES` at 4x (`v3d_job.c:1080-1081`), stored at `:1174-1179`; `v3d_depth_resolve` (`src/v3d/backend.rs:4798`) reads a 1x layout, so no kept-sample copy for it exists |
| stencil | same store, stencil half of D24S8 (`S8` format when split) | none: sample 0 | `v3dx_rcl.c:288-304` | `v3d_job.c:1186-1192` (`zs_decimate`), `:1080-1081` |

For the V3D item: (1) keep the samples per RT in that RT's own output
format (`rt_fmt` + `rt_internal`, `v3d_job.c:259`, `:151`), not the `RGBA8`
literal at `v3d_job.c:1118` (and the load at `:1056`), or an sRGB/RGB10_A2
4x target would be stored as 8-bit bytes the tile buffer cannot hold; (2)
lift the typed-target refusal at 4x (`v3d_job.c:311-316`); (3) keep a
sample-0 store of depth and stencil into a 1x D24S8 beside the 4x one, for
`v3d_depth_resolve` (`backend.rs:4798`) to read.

### (f2) PowerVR (`imagination/vulkan`, yakogl `src/pvr`)

Colour: the PBE's `downscale` bit on the end-of-tile store
(`pvr_arch_job_common.c:282`, `reg.downscale = surface_params->down_scale`),
averaging in the PBE. An sRGB format uses an F16 source and `gamma` on
(`pvr_arch_pbe_get_src_format_and_gamma`, `pvr_arch_job_common.c:56-60`,
`gamma` set at `:166-171`, format row `pvr_arch_formats.c:213`); RGB10_A2
is `A2R10B10G10` with an F16 source and no gamma (`pvr_arch_formats.c:223`),
so an sRGB average is in the F16 linear values, encoded on store.
Depth and stencil do not go through the PBE at 4x unless the format is
depth-only and the mode is average (`pvr_can_pbe_resolve_ds_attachment`,
`pvr_arch_cmd_buffer.c:4041-4057`: stencil, any other mode, or a part
without `gs_rta_support` is refused): they take `PVR_RESOLVE_TYPE_TRANSFER`
(`pvr_arch_cmd_buffer.c:4374`, `:4412`), a transfer-engine pass.
`PVR_RESOLVE_SAMPLE0` (a `VK_RESOLVE_MODE_SAMPLE_ZERO_BIT`,
`pvr_arch_cmd_buffer.c:3737-3738`) goes to the `default:` of the transfer
setup (`pvr_arch_job_transfer.c:2098-2105`): instance-rate shader, the TPU
told the sample count and the shader's `sno` naming the sample.
`PVR_RESOLVE_BLEND` on a D24S8 pixel format is an instance-rate USC filter
(`pvr_int_pbe_pixel_requires_usc_filter`, `:1974-1992`), not PBE downscale.

| row | resolve operation | averaging space | Mesa | yakogl |
|---|---|---|---|---|
| RGBA8 | PBE `downscale`, source 8 per channel, U8U8U8U8 | stored value (normalised, no gamma) | `pvr_arch_job_common.c:282`, `:39-79`, `pvr_arch_formats.c:105` | `pbe_reg_words` `src/pvr/backend.rs:896-915`, called with `downscale = b.samples > 1` at `:8958` (window only) |
| SRGB8_ALPHA8 | PBE `downscale`, F16 source, `gamma` on | linear (F16), encoded on store | `pvr_arch_job_common.c:56-60`, `:166-171`, `pvr_arch_formats.c:213` | `gamma: gles_surf_is_srgb` `backend.rs:914`; F16 source `:1017-1021`; order of average and encode: **board only** |
| RGB10_A2 | PBE `downscale`, F16 source, no gamma | unorm value | `pvr_arch_formats.c:223` | stored as RGBA8 today (note (d)): **board only** |
| depth | transfer pass, `PVR_RESOLVE_SAMPLE0` (average is the PBE only for a depth-only format with `gs_rta_support`) | none | `pvr_arch_cmd_buffer.c:4041-4057`, `:3737-3738`, `pvr_arch_job_transfer.c:2098-2105` | window/CPU path: `ds_resolve` `backend.rs:4259`, `pvr_backend_depth_resolve` `:4909`; the hardware sample-0 position: **board only** |
| stencil | same transfer pass (the PBE refuses stencil) | none | `pvr_arch_cmd_buffer.c:4053` | `pvr_backend_stencil_resolve` `backend.rs:4982`; sample position **board only** |

### (f3) GC7000UL (`gallium/drivers/etnaviv`, yakogl `src/vivante`)

Colour: the resolve engine (RS) copies the 2x2 (4x) tiled source into the
single-sampled target with `RS_CONFIG_DOWNSAMPLE_X`/`_Y` set when the source
has more samples (`etnaviv_rs.c:697-707`, config bits `:96-97`). Mesa does
not say how the RS averages or in which space; it takes no scaling
(`:712-719`), no mask (`:721-726`) and the same format on both sides
(`:728-738`), and only the formats of `translate_rs_format`
(`etnaviv_translate.h:279-310`). `etna_compatible_rs_format` (any
same-size format) is allowed only when *not* downsampling
(`etnaviv_rs.c:740-747`), so SRGB8_ALPHA8 (`R8G8B8A8_SRGB`: only the `B8G8R8A8`
variant is in the RS table, `etnaviv_translate.h:306-309`) and RGB10_A2 have
no RS resolve in Mesa and fall to the blitter's shader resolve
(`etnaviv_clear_blit.c`, `u_blitter.c`), which averages the sampled
values. Depth/stencil: a D24S8 resolve is a whole-pixel RS copy as
`RS_FORMAT_D32` (`etnaviv_translate.h:288-289`); Mesa promotes a depth
or stencil mask to the whole pixel for it (`etnaviv_clear_blit.c:171-180`),
so depth and stencil resolve together, and which sample the RS keeps is
not said.

| row | resolve operation | averaging space | Mesa | yakogl |
|---|---|---|---|---|
| RGBA8 | RS copy, `A8R8G8B8`, DOWNSAMPLE_X/Y | the RS's (raw 8-bit channels): **board only** | `etnaviv_rs.c:697-707`, `etnaviv_translate.h:306-309` | `emit_resolve` `src/vivante/backend.rs:1032-1049` (config at `:1041`), window only, `GC_MSAA_ENABLE` off; RS format `ms_rs_format` `:1022-1028` |
| SRGB8_ALPHA8 | not an RS format in Mesa (`R8G8B8A8_SRGB`); shader resolve in Mesa | blitter: sampled values, averaged (`util_make_fs_msaa_resolve` sums and multiplies by 1/n, `u_simple_shaders.c:790`) | `etnaviv_translate.h:306-309`, `u_blitter.c:1002-1030` | `ms_rs_format` gives `A8R8G8B8` for every 32-bit target: the RS averages the encoded bytes (**board only** whether that matches the dEQP tolerance) |
| RGB10_A2 | no RS format (`R10G10B10A2` absent from `translate_rs_format`); shader resolve | blitter: sampled unorm values | `etnaviv_translate.h:279-310`, `u_blitter.c:1002-1030` | stored RGBA8 (note (d)); an `A8R8G8B8` RS would average the four 8-bit fields of the word, wrong: the item must refuse the RS for it or draw single-sampled. **board only** for any other route |
| depth | whole-pixel RS copy, `D32`, mask promoted | none | `etnaviv_translate.h:288-289`, `etnaviv_clear_blit.c:171-180` | CPU resolve `resolve_depth` `src/vivante/ds_cache.rs:595`; the sample the RS keeps: **board only** |
| stencil | same copy (a stencil-only mask is promoted) | none | `etnaviv_clear_blit.c:171-180` | no stencil resolve of a 4x surface; **board only** |

### (f4) Mali (`gallium/drivers/panfrost`, `panfrost/lib`, yakogl `src/mali`)

Two routes in Mesa. (1) Gallium `panfrost_blit` runs `util_blitter` on the
*linear* view of both formats (`pan_blit.c:91`, `:95`: `util_format_linear`),
so an sRGB resolve averages the encoded values; the blitter's colour
resolve shader sums the samples and divides by their number
(`u_simple_shaders.c:790`), integer formats copy one sample
(`u_blitter.c:1005-1009`), and depth/stencil to a 1x destination draws
with sample index 0 (`u_blitter.c:1975-1986`; the MSAA-to-MSAA path loops the
samples, `:1955`). (2) The tile writeback `MALI_MSAA_AVERAGE` (a view with
more samples than its image, `pan_desc.c:26-41`, used for the render target
at `:1033` and the Z/S at `:518-521`), which yakogl uses. Mesa's newer
framebuffer-preload code folds a resolve into the store only for
`SAMPLE_0` on Z/S ("Z/S only supports SAMPLE_0", `pan_fb.c:177-179`), and
for colour only for formats that blend in hardware
(`pan_format_supports_msaa_average`, `pan_format.h:372-383`: not the
formats needing a blend shader, nor F16 before v11); sRGB is an R8G8B8A8
tile-buffer format with the sRGB flag (`BFMT_SRGB`, `pan_format.c:62-65`),
RGB10_A2 is blendable (`pan_format.c:108`). Which space the hardware
AVERAGE works in is not stated: **board only**.

| row | resolve operation | averaging space | Mesa | yakogl |
|---|---|---|---|---|
| RGBA8 | tile writeback `MALI_MSAA_AVERAGE` (or the blitter's summed fetches) | the tile buffer's 8-bit unorm: **board only** | `pan_desc.c:26-41`, `:1033`; `pan_format.c:62-65` | `Msaa::AVERAGE` `src/mali/frame.rs:2198-2208`, `resolve_frame` `:2337` |
| SRGB8_ALPHA8 | same writeback; blitter route on the linear view | blitter: encoded values (`pan_blit.c:91`, `:95`); writeback: **board only** | `pan_blit.c:91`, `:95`; `pan_format.c:62-65` | colour target format `TargetFormat` in `frame.rs`; sRGB 4x: **board only** |
| RGB10_A2 | same writeback (blendable); blitter route otherwise | unorm value | `pan_format.c:108`, `pan_format.h:372-383` | no 4x target of this format: **board only** |
| depth | blitter: `sample 0` draw; folded store: `SAMPLE_0` only | none | `u_blitter.c:1975-1986`, `pan_fb.c:177-179` | `frame.rs:2149-2154` writes the 4x D24S8 with **AVERAGE**, not SAMPLE_0 as Mesa's newer path would: which sample the hardware keeps is **board only** |
| stencil | same | none | same | same: **board only** |

### (f5) What each `*-msaa-fbo` item must implement

- **All four:** the resolve is colour average, depth/stencil one sample.
  Choose sample 0 where the back end can; where it cannot say (GC7000UL
  RS, Mali AVERAGE on Z/S) write the sample down in the host test as
  "unmeasured" and leave the cell of this note **board only** until a board
  run fills it. Integer formats copy one sample (Mesa: `u_blitter.c:1005`).
  `max_samples` stays 0 until the item's host test passes (note (a)).
- **`v3d-msaa-fbo`:** the three points under (f1): per-RT sample format,
  typed 4x targets, a sample-0 depth/stencil store for `v3d_depth_resolve`.
  sRGB and RGB10_A2 average on the 16F halves, as the TLB does.
- **`pvr-msaa-fbo`:** colour through `downscale` (`pbe_reg_words`,
  `gamma` for sRGB, F16 source); depth/stencil need their own sample-0
  path, not the PBE (Mesa: transfer pass, (f2)); RGB10_A2 stays refused or
  single-sampled while it is stored RGBA8.
- **`gc-msaa-fbo`:** RS `D32` for D24S8 (depth and stencil together);
  `ms_rs_format` must not hand RGB10_A2 or sRGB to the `A8R8G8B8` RS
  without a host test that pins the choice; sizes the RS cannot resolve are
  open point 4 of (e).
- **`mali-msaa-fbo`:** the writeback AVERAGE for colour; decide Z/S
  (`SAMPLE_0` as `pan_fb.c:177-179`, against `frame.rs:2149-2154`'s
  AVERAGE) in the item's host test and mark the hardware choice **board
  only**.

## (e) Open points and the RC8 row draft

1. **Size cap.** (b)3 sets `width * height > 2048 * 2048` as
   `GL_OUT_OF_MEMORY` at `glRenderbufferStorageMultisample`. The back ends
   allocate samples lazily at the first draw: when that fails the back end
   falls back to "samples not kept" (V3D `ms_buf` returns `None`, Mali
   `sample_store` returns `None`, PowerVR `ms_store_ready` false): correct as long
   as no job loads from the missing samples; the first job after a loss is
   the lossy start of the contract. A per-format or per-attachment-count cap
   (four 64-bpp RTs at 2048x2048 are 512 MiB) is the GL core's later decision.
2. **Slots.** How many 4x attachments a back end keeps live: the V3D 6 or more,
   the PowerVR, GC7000UL and Mali at least 2. dEQP draws one 4x FBO at a time;
   glmark2 and Qt use none. Revisit when a client uses several.
3. **GC7000UL `msaa-fbo` turns on the 4x window config** (the knob's other
   job): acceptable for an opt-in board build; or split a second feature.
4. **GC7000UL start of a job without a whole clear** (upscaling `rb.mem` into
   the scratch: draw or refuse) and **sizes the RS cannot resolve** (draw
   single-sampled vs. refuse vs. pad): decided by the item with a host test
   of the command stream; neither is a board fact. The mocks do not execute a
   kick: the GC7000UL host test pins register words, not pixels.
5. **Several colour attachments at 4x** are the V3D's only. The other three
   refuse by name. ES 3.0 allows them; dEQP's `fbo.msaa` does not need them.
6. **Depth/stencil resolve sample**: sample 0 for the CPU copy; the V3D, PowerVR
   and GC7000UL item pins which sample the hardware plane holds, the
   Mali's depth samples are not kept across frames.
7. **sRGB and RGB10_A2 resolve averaging**: the order of averaging and gamma is the PowerVR's PBE and the V3D's TLB
   (linear halves in the tile buffer); dEQP tolerance decides, unmeasured.
8. **Unmeasured on a board:** the PowerVR's unresolved-store `smpcnt` layout
   across kicks (#1289), the Mali's `AVERAGE`/`LAYERED` for an FBO target, the
   GC7000UL's every 4x dEQP row, the V3D's several-RT 4x. All stay behind the feature.
9. **Ordering across plans.** `src/gl/framebuffer.rs` and `src/gl/draw.rs` have
   other plans' items (#325, #335); `src/backend.rs` is #335's and #328's; the
   four `msaa-fbo` items wait for the back-end owners named in (c).

### RC8 row draft for `docs/es3-cts-failures.md`

(That file is held by plan #330 until its `inventory-rc8` item; this is the text
that replaces the last sentence of the RC8 section, "Multisample
renderbuffers: `fbo.msaa` 34, ... no item yet.")

> Multisample renderbuffers (`docs/notes/es3-msaa-renderbuffers.md`): `fbo.msaa`
> 34 (2 and 4 samples), `fbo.completeness.samples` 7, `fbo.invalidate.*_msaa_*`
> 8, `multisample.fbo_max_samples` 4 (and `fbo_4_samples` 16, NotSupported),
> `rasterization.fbo.rbo_multisample_max` 2 (and `_4` 12, NotSupported),
> `state_query.integers.max_samples_*` 2, `negative_api.buffer.blit_framebuffer_multisample`
> 1: about 83 rows move, the 8-sample, default-framebuffer and float
> (R11F/32F) rows stay. The cause is the GL core: `GL_MAX_SAMPLES` is 1 and a
> multisample renderbuffer is single-sampled storage. Fix, behind
> `YakoglBackend::max_samples` (0 by default) and the cargo feature
> `msaa-fbo` (off, board images unchanged): the GL core keeps 4 samples per
> renderbuffer and resolves in `glBlitFramebuffer` (`tests/es3_msaa_gl.rs`),
> each back end draws and resolves a 4x FBO and leaves the resolved image in
> the renderbuffer's storage (PowerVR, V3D, GC7000UL, Mali: host-tested,
> `<status per back end>`); board runs follow the three board sessions. Item
> plan #339.

### Status comment for issue #2 (text to post; agents do not run `gh`)

> msaa-note (plan #339): `docs/notes/es3-msaa-renderbuffers.md` at `<commit>`.
> It extends `v3d-msaa-es3.md` to the PowerVR, V3D, GC7000UL and Mali and fixes
> what the other items implement: (a) `YakoglBackend::max_samples: u32` (0 =
> single-sampled, the default) set to 4 only through the cargo feature
> `msaa-fbo` (off) on a back end whose 4x host test passes; board images stay off
> until a board session. (b) the GL core reports 4 for any request 1..=max,
> allocates only `rb.mem`, `GlesDraw.samples` = the framebuffer's count, the
> back end keeps the samples keyed by `GlesDraw::target_id`/`target_gen` (new,
> `ds_object_id` of the renderbuffer and its `storage_gen`), drops them in
> `attachment_gone` (extended, no new entry; also called from invalidate) and
> leaves the resolved image in `rb.mem` at the end of every job; blit,
> read-back and sampling read resolved pixels; ES 3.0 has no implicit
> resolve of an FBO. (c) per back end, with the PowerVR's, GC7000UL's and
> Mali's one-slot stores becoming pools. (d) about 83 dEQP rows per back end,
> 2-3 with the GL core alone. Open: the size cap, GC7000UL 4x window config under
> the feature, the start of an unclear GC job, one colour attachment on three
> back ends. Not run: any board.
