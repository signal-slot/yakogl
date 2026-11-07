<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: shadow's, refract's and terrain's streams against Mesa etnaviv's, kick by kick

Plan #328, item stream-diff, issue #1204. The breakdowns
(`gc-slow-shadow-refract.md` "The frames", `gc-slow-terrain.md` "The 52
kicks") count our kicks. This note puts each frame's command stream beside
the stream Mesa etnaviv emits for the same glmark2 scene on the same part
(the GC7000UL row of the drm-shim). It ranks what differs, in ms a frame on
the board.

Host only. No board was run and no code was changed. Code is cited as
`path:line` at `98251549`.

## Answer in short

- **Mesa records each frame as one submit.** Shadow is 1 against our 3,
  refract 1 against our 13, terrain 2 against our 52. The words are about
  the same: shadow 1,136 against 1,454, refract 4,718 against 4,592,
  terrain 20,446 against 18,964. So the volume of re-emitted state costs
  nothing. What costs is where the kicks end and how the clears are done.
- **Inside a draw the two drivers sync the same way.** Mesa writes the
  same sequence before every draw that we write: FLUSH 0x27 and 0x10, then
  a semaphore and stall pair, the FE to PE stall, and a pair before the
  draw. The only extra flush in our streams is the colour and depth flush
  before a draw whose target did not change (Mesa writes 0x20 alone there).
  It costs about 0 ms.
- **Mesa never clears with a quad in these scenes.** Every glClear is an RS
  fill of the tile-status buffer: 16x68 words for an 800x600 surface, 16x240
  for a 1600x1200 one. Our full-target quads take about 9.4 ms a frame in
  shadow, 9.4 ms in refract and 8.5 ms in terrain (model).
- **The cheapest fix the breakdowns do not rank:** clear a full,
  unscissored, unmasked target with an **RS fill of the surface itself**,
  without tile status. A fill writes one word to every pixel, so it does not
  depend on the surface's layout and needs none of `gc-tiled-targets.md`'s
  layout steps. This board has already done an RS fill of a linear surface
  (`driver/src/vivante/gc_draw.c:201-262`, sample `gc_clear` PASS,
  `imx8mp-gc-bringup.md:214`). The estimate is 3.6 to 8 ms a frame for
  shadow, 0 to 7 ms for refract and 1.7 to 6.8 ms for terrain. The range is
  the RS fill rate, which nobody has measured (finding 1).
- **Mesa samples the rendered depth surface directly** in shadow and
  refract. The texture descriptor's LOD_ADDR[0] is the depth pass's
  PE_PIPE_DEPTH_ADDR, read through the sampler TS. This is the golden for
  item depth-sample-gc, the fix of the 350 to 1000 ms CPU resolve. The
  words are under "The golden of the depth sample".

## How the streams were made

Both sides ran the same glmark2 scenes at 800x600, `-b :nframes=3`.
glmark2 was at `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the deps cache's
`glmark2-src`, which `tools/hostharness/glmark2/README.md` pins.

**Ours.** The GC7000 build of the glmark2 host harness, unmodified. It was
built once by `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out> vivante`
(`GM2_WRITE=0 GM2_FRAMES=1 GM2_ENTRIES='^shadow$'`). Each scene was then run
directly with the environment `run_entry` sets, plus the Linux OSAL's
stream dump (`src/osal/linux.c:409-431`):

```sh
YAKOGL_GC_CARVEOUT_BYTES=245642240 YAKOGL_ERROR_LOG=1000000 GM2_PROFILE=0 GM2_FRAMES=3 \
  YAKOGL_CS_DUMP=<dir>/<scene>.cs \
  <out>/glmark2-host-vivante --size 800x600 -b :nframes=3 -b <scene>
```

Every entry ended `ok`, with 0 GL errors, 0 refused draws and 0
allocations refused.

**Mesa's.** Mesa 26.2.1 (the deps cache's source tree), patched with
`tools/vivante/mesa-cs-golden.patch` (sha256 `a84cc2a4126eb9a1...`) and built
with the README's recipe ("Building it"; 1022 of 1022 steps). The build is
in `$CONNECTED_DEPS_DIR/mesa-etnaviv-golden`. glmark2 itself, from the same
checkout, was built as `glmark2-es2-null`. It needed the bundled
png/jpeg/zlib: `meson.build`'s `if host_machine.system() == 'windows'` was
changed to `if true` in a copy, with `-Dc_args='-DZ_HAVE_UNISTD_H -include
unistd.h'`. It ran off-screen against the shim:

```sh
LD_PRELOAD=$B/src/etnaviv/drm-shim/libetnaviv_noop_drm_shim.so \
LD_LIBRARY_PATH=$B/src/egl:$B/src/mesa/glapi/es2api:$B/src/gallium/targets/dri \
MESA_LOADER_DRIVER_OVERRIDE=etnaviv ETNA_SHIM_GPU=GC7000UL MESA_SHADER_CACHE_DISABLE=true \
ETNA_SHIM_CS_DUMP=<dir>/mesa-<scene>.cs [ETNA_SHIM_MEM_DUMP=<dir>/mesa-<scene>.mem] \
  glmark2-es2-null --off-screen --size 800x600 -b :nframes=3 -b <scene>
```

`GL_RENDERER: Vivante GC7000 rev 6204`. Without
`MESA_LOADER_DRIVER_OVERRIDE` the loader takes this host's virtio GPU, and
`eglInitialize` fails.

Both files were decoded with `tools/vivante/cs_decode.py`, using Mesa's
`state_3d.xml.h` and `state.xml.h` from that tree. A throwaway script (not
committed) counted each submit's flushes, semaphores, stalls, RS kicks and
draws.

**Which submits.** Every frame after the first was identical in words on
each side.

| scene | ours: submits in the file, frame used | Mesa: submits, frame used |
|---|---|---|
| shadow | 10: setup 0, frames 1-3 / 4-6 / 7-9; used 4-6 | 3, one a frame; used 1 |
| refract | 40: setup 0, frames of 13; used 27-39 | 4; used 3 |
| terrain | 238: setup 0-81, frames of 52; used 186-237 | 8, two a frame after setup; used 6-7 |

**One difference in the shape of the run.** glmark2's canvas is the EGL
window on the board and in our harness. Under `--off-screen` it is an FBO,
which Mesa makes tiled with tile status. So Mesa's canvas clears are TS
fills too, and Mesa's window draws get early depth. The scene's own passes
are the same GL calls on both sides.

## The frames side by side

Notation: `F27` is GL_FLUSH_CACHE 0x27 (colour, depth, texture, shader L1),
`F10` is the vertex-texture flush, `S` a semaphore and stall-token pair,
`St` the FE to PE STALL command, `RS` an RS_KICKER, `Dn` a draw of n
vertices.

### shadow

| | ours (3 kicks, 1,454 words) | Mesa (1 submit, 1,136 words) |
|---|---|---|
| canvas clear | K1 alone (308 words): `F27 F10 S St S D4`, a 800x600 colour and depth quad | `F03 S RS F03 RS S`: two TS fills, 16x68 words each |
| depth clear | K2: `D4`, a 1600x1200 depth quad, late depth (RA_EARLY_DEPTH 0x15000030, PE_DEPTH_CONFIG 0x00041711) | `F03 S RS S`: one TS fill, 16x240 words |
| horse, depth pass | K2: `F27 F10 S St S D21516`, late depth | `F27 F10 S St S D21516`, **early depth** (0x05000031, 0x05051311: EARLY_Z, DISABLE_ZS, super-tiled) |
| kick end | K2 submitted by `texture.rs:850`, then the CPU resolve | none |
| ground | K3: `F27 F10 S St S D4`, samples the CPU-resolved R32F copy | `F27 F10 S St S D4`, samples the depth surface itself through the sampler TS (TS_SAMPLER_CONFIG[0] 0x63, TX_CTRL 0x23) |
| horse, lit | K3: `F27 F10 S St S D21516` | `F20 S St S D21516` (same target: no colour or depth flush) |

### refract

| | ours (13 kicks, 4,592 words) | Mesa (1 submit, 4,718 words) |
|---|---|---|
| clears | K1: canvas quad alone (308). K2: 1600x1200 colour and depth quad (`D4`) | four TS fills (2 x 16x68, 2 x 16x240), then **one in-place RS resolve of the 1600x1200 colour target** (RS_CONFIG 0x4686, source = destination, window 0x04b00640) |
| bunny into the FBO | K2: `D208998`, late depth | `D208998`, early depth, colour TS (TS_MEM_CONFIG 0x3c3) |
| mipmap chain | K3..K12: one kick a level, `F27 F10 S St S D4` each (328 words) | ten u_blitter draws in the same stream, `F07 F10 S St S D3` each, late depth |
| kick ends | 12: K1 (target), K2 (target), each level (`mipmap.rs:270-281`), the chain's flush (`:278`) | none |
| final bunny | K13: three textures, depth from the CPU resolve | three textures, depth through the sampler TS |

Mesa's in-place resolve runs before the first draw. The sampler views still
bound from the previous frame's final pass include the mipmapped colour
texture. A mipmapped view cannot use the sampler TS
(`gc-tiled-targets.md` "Sampler TS"), so Mesa resolves it. So Mesa also
writes the 1600x1200 colour surface once a frame through the RS, which
bounds what a TS clear can gain for refract below the 7.5 ms model.

### terrain

| | ours | Mesa |
|---|---|---|
| submits | 52 (`gc-slow-terrain.md` "The 52 kicks") | 2 (16,040 + 4,406 words) |
| clears | 8 quads | 11 TS fills (RS_CLEAR_CONTROL 0x0001ffff 0x11111111, windows 16x68, 16x36, 16x8) |
| before a chain is sampled | nothing (linear, no TS) | 6 in-place RS resolves: 4 at 256x256, 2 at 800x600 |
| mipmap chains | 43 one-level kicks | 43 draws in the stream |
| draws | 60 (clears included) | 51 |

## Findings, ranked

The ms are board estimates from the model of `imx8mp-frame-floor.md`
(candidate 2), with the calibrations that `gc-slow-terrain.md` and
`gc-tiled-targets.md` ("Per-scene passes and the pixel model") use:

- 3.9 ns a pixel for a quad;
- the glmark2 `clear` scene, one 800x600 colour and depth quad: 1,981 us of
  GPU busy a frame (`imx8mp-frame-floor.md` "Step 3 result"). That is 4.1 ns
  a pixel and 1.9 GB/s written;
- 0.03 to 0.08 ms of GPU a kick for the kick's own overhead
  (`gc-slow-terrain.md` "The board timeline model").

PERF values are those the X2 keeps: v0..v3 of a page, so page 1 v4 (kicks)
does not reach the verdict. Page 0 v0 (frames) moves less than 1 % for
shadow and refract while their CPU depth resolve sets the frame (350 to
1000 ms). Until depth-sample-gc or resolve-wide lands, confirm the GPU
findings by page 1 v3 (GPU busy us a frame) and page 3 v1 (HI idle kcycles),
not by the frame time.

| rank | finding | code | ms a frame (shadow / refract / terrain) | proposed fix | X2 PERF value that confirms it |
|---|---|---|---|---|---|
| 1 | **Full-target clears drawn as quads, where Mesa uses an RS fill.** Shadow K1 0.48 Mpx and K2 1.92 Mpx (depth only); refract K1 0.48 and K2 1.92 Mpx (colour and depth); terrain 8 quads, 2.18 Mpx. With the quads at 4.1 ns a pixel, that is 9.4 / 9.4 / 8.5 ms of GPU | `src/vivante/backend.rs:2346-2349` (`emit_clear`, "the tile-status fast clear is an optimisation this backend leaves out"), called from `record` `:3097`; the RS block exists (`emit_rs_copy` `:938-960`) | **3.6-8 / 0-7 / 1.7-6.8** (at an RS fill rate of 2 to 8 GB/s; the bytes are 11.5 / 19.2 / 13.5 MB) | `emit_clear` takes a full, unscissored, unmasked clear of a target whose padded width is a multiple of 16 and height a multiple of 4. It records the RS fill as `gc_draw_clear` does (`driver/src/vivante/gc_draw.c:217-259`: F03, RA to PE stall, RS_CONFIG A8R8G8B8, linear strides, RS_CLEAR_CONTROL bits and fill value, RS_SINGLE_BUFFER around the kick, stall). That is one fill for colour and one for depth over the padded D24S8 allocation. A depth-only clear keeps stencil with bits 0xeeee, Mesa's S8Z24 case (`tools/vivante/README.md`, mode 8). A clear with no stencil plane writes the whole word, as Mesa does. Scissored, masked and odd-sized clears keep the quad. No layout or TS change: a uniform fill is the same bytes in any layout. Host test: the mock stream of a glClear shows the RS fill words of golden mode 15 (or 8) and no clear quad; the `ds_cache` plane bookkeeping (`draw_clears`) is unchanged. `cs_golden` modes 8, 12 and 15 already are such a fill (no TS, RS_CLEAR_CONTROL 0x0001eeee with the depth word), so after the change `cs_ours` modes 8, 12, 14 and 15 diff against them RS block for RS block. | `-b clear` alone (the cleanest A/B): page 1 v3 **1,981 -> 500 to 1,000 us** at 4 to 8 GB/s (no change at 2 GB/s rejects the gain); page 3 v3 (KiB written) unchanged, since the same bytes are written. shadow: page 1 v3 down by 3,600 to 8,000; refract down by 0 to 7,000; terrain down by 1,700 to 6,800; page 3 v1 up by the same time. Step 0 (iv) of `gc-tiled-targets.md` (the quad, RS fill and TS fill timed on one 1600x1200 surface) measures the rate directly. |
| 2 | **Kick boundaries Mesa does not have**: 2 / 12 / 50 a frame more. Each one adds the OSAL's epilogue and prologue (flush, TS flush, FE to PE semaphore and stall, EVENT, then F03, a stall and the pipe select; `driver/rust/src/gpu/vivante/cmdbuf.rs:157-190`), our context init (`backend.rs:2186-2219`, about 80 words), the interrupt, and, when the FE has gone idle first, its WAIT of 24,414 cycles (30 us, `cmdbuf.rs:54-56`). On the CPU side come `frame_open`'s waits for a slot reused 4 kicks later (`backend.rs:2229-2235`, `GC_FRAMES` 4 `:77`) and their 1 ms poll | a target change ends the kick (`backend.rs:2902-2904`); each mipmap level is a target (`src/vivante/mipmap.rs:266-281`) | **0.1-0.2 / 1-3 / 9-14** (terrain's 7 to 10 of starvation is `gc-slow-terrain.md` rank 2; GPU overhead 0.06-0.16 / 0.4-1.0 / 1.5-4) | (a) mip-one-kick (ready): a chain in the open stream. Mesa's u_blitter chain needs only the per-level `F07 F10 S St S` that our `emit_flushes` already emits (`backend.rs:1790-1795`), and no RS or extra semaphore. Refract goes from 13 kicks to 3, terrain from 52 to 9. (b) After that, a render-target change does not end the kick: keep recording and flush the PE caches with the stall that every draw already has. `frame_submit` would then run only at a CPU wait, a full stream, a swap or an RS resolve of an MSAA target (the `ms_dest_gpu` resolve `:2300-2303` has to be emitted at the target change instead). That takes terrain to about 2, the same as Mesa. Gain after (a): 0.2-0.6 ms terrain, about 0.1 shadow and refract. | page 1 v1 (waits per 100 frames): refract 200 to 900 -> 100 to 200 after (a); terrain 600 to 800 drops by the 5 to 7 `frame_open` waits a frame. page 3 v1 (HI idle) falls by about 5,000 kcycles (terrain, after (a)). Page 1 v4 (kicks per 100 frames: 300 / 1,300 / 5,200 now) is in w3, which the X2 does not keep; page 4 (fb-counters) would need a kicks count to show it. |
| 3 | **The canvas clear alone in its kick** (shadow and refract K1, terrain's first kick): the clear is recorded, then the FBO pass's first draw changes the target and submits it | `backend.rs:2902-2903` | **0.03-0.08 each** for the boundary. The pixels (2.0 ms) are finding 1's | Finding 1 makes the clear cheap. Finding 2 (b) removes its kick. Deferring the pending canvas clear until the next draw into the window (the GL core's pending clear, `src/gl/draw.rs`) would fuse it into K3 or K13 and save the same boundary, at the cost of GL-core state that plan #310 is also changing. Not worth doing alone. | none on the X2 (under 0.1 ms). Covered by finding 2's values. |
| 4 | **Early depth off on shadow's depth-only pass.** Mesa allows it with no colour target (`etnaviv_state.c:1010-1023`; golden: RA_EARLY_DEPTH 0x05000031, PE_DEPTH_CONFIG 0x05051311); ours denies it for every draw | `backend.rs:1770-1784`; the dummy colour surface (`ensure_no_colour_surface` `:792`) is not a reason Mesa's rule knows | **0-1 / 0 / 0** (`gc-tiled-targets.md` step 5) | Step 5's rule, for draws with no colour target. It needs the depth layout of step 2 (SINGLE_BUFFER(2), tiled depth) but **not** step 4's tiled colour, so shadow's pass can take it right after step 2. That is a change of sequence for the study's list, not new work. | shadow page 1 v3 down by 0 to 1,000 us. |
| 5 | **A colour and depth flush before a draw into the same target** (shadow K3's horse; terrain's two-draw passes): ours writes `F27 F10`, Mesa `F20` | `backend.rs:1786-1795` (unconditional by design) | **about 0** (gc_slint: per-draw flushes and stalls measured at 47.4 against 47.9 ms, `imx8mp-frame-floor.md` candidate 2) | none here; `gc-kick-diff.md`'s K1 knob is the place | none |
| 6 | **State re-emitted in each kick**: the context init and every state group at the first draw of each kick (`dedup::invalidate`, `backend.rs:2253`, `mipmap.rs:269`) | as cited | **about 0**: the frames have about as many words as Mesa's (1,454 / 4,592 / 18,964 against 1,136 / 4,718 / 20,446); the FE fetches 6 to 76 KB a frame | none, beyond finding 2 | none |

Not ranked, because it is already the plan's largest item: the CPU depth
resolve (350 to 1000 ms, `gc-slow-shadow-refract.md` cause 1). Mesa has no
counterpart to it. Its stream samples the surface the depth pass wrote,
which is the evidence below.

What Mesa does and this note does not propose: TS fast clears, tiled colour
targets, sampler TS and in-place resolves. These are steps 2 to 6 of
`gc-tiled-targets.md`, and the goldens above agree with that study's reading
of Mesa's source: the TS fill sizes, early depth with no colour target,
resolves before sampling a mipmapped chain, and sampler TS for single-level
views. Terrain's 6 resolves (2 at 800x600, 4 at 256x256) are the cost step
4's gain estimate subtracts.

## The golden of the depth sample

Shadow, Mesa, one frame (`ETNA_SHIM_MEM_DUMP`, `cs_decode.py --mem
--tex-desc`), for item depth-sample-gc:

- The depth pass: PE_PIPE_DEPTH_ADDR[0] `0xfe86f000`, PE_DEPTH_STRIDE
  `0x1900`, PE_DEPTH_CONFIG `0x05051311`, TS_MEM_CONFIG `0x41`, depth TS
  status at `0xfe86b000`, clear value `0xffffff00`.
- The ground's sampler, slot 0: TS_SAMPLER_CONFIG[0] `0x63`,
  TS_SAMPLER_STATUS_BASE[0] `0xfe86b000`, TS_SAMPLER_CLEAR_VALUE[0]
  `0xffffff00`, TS_SAMPLER_SURFACE_BASE[0] `0xfe86f000`,
  NTE_DESCRIPTOR_TX_CTRL[0] `0x23`, SAMP_CTRL0 `0x00202212`, SAMP_CTRL1
  `0x2`, LOD_MINMAX `0xfff`.
- Its descriptor: LOD_ADDR[0] `0xfe86f000` (the depth pass's surface
  itself), CONFIG0 `0x00022002`, SIZE `0x04b00640` (1600x1200),
  LINEAR_STRIDE `0x1900`, CONFIG1 `0x08500000`, CONFIG2 `0x00030000`,
  LOG_SIZE_EXT `0x0a3a0aa4`, SLICE `0x0076c000`, 3D_CONFIG `0x1`.

So Mesa samples the surface the PE wrote, at the PE's own address, through
the TS, with no copy and no CPU pass. Refract's final pass does the same
for its depth texture (TX_CTRL[0] `0x23`, TS status `0xfe1ea000`). Mesa's
depth surface is super-tiled. Ours is row-linear under SINGLE_BUFFER(1)
(`backend.rs:1497-1512`). Whether the texture unit reads our layout is
gt-depth-tpu's board question. These words only say what a working driver
programs for its own layout.

## What this note does not show

- No board numbers. The RS fill rate behind finding 1 is unmeasured. The
  range of 2 to 8 GB/s is an assumption: at the low end the RS is no faster
  per byte than the quad's 1.9 GB/s, at the high end it is half the LPDDR4
  peak. If the X2's `clear` A/B shows page 1 v3 unchanged, finding 1 is
  worth only the depth-only clears' half-rate penalty (shadow 3.6 ms) or
  nothing.
- That the RS fills our padded D24S8 allocation and linear colour targets
  at the stride the PE uses was not run. The bring-up `gc_clear` filled a
  linear window surface on this board (2026-09-20), not a depth surface.
- Mesa's run is off-screen (see "How the streams were made"), so its canvas
  has TS and early depth that a window would not.
- The per-kick overhead (0.03 to 0.08 ms) is `gc-slow-terrain.md`'s
  assumption, not a measurement.
- The streams were read for state, flushes, stalls, RS and draws. Shader
  code and uniforms were not compared (item noise-shader-count compares
  terrain's shader).
