<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: where glmark2 shadow's and refract's frames go (host breakdown)

The i.MX8MP record at main `a917d123` reads glmark2 `shadow` and `refract` at
2 FPS each, written down as "500 ms a frame" (`tools/cts/RESULTS.md`, i.MX8MP
row). On the host their GL calls take about 0.3 ms a frame. This note is the
host breakdown of one frame of each scene on the GC7000 build, for plan #328
and issue #1204: the GL-call CPU time, the CPU passes over GPU memory, the CPU
waits on the GPU, the bounded waits, and the GPU work from the kicks. Then it
ranks the causes, names a fix for each, and gives the PERF record values that
would confirm each cause on the Armadillo-X2.

Host work only. Nothing ran on a board. The instrumentation was temporary and
is not committed. Code is cited as `path:line` at `964f50f0`. The current
main (`48fc90e3`) differs from it by 9 commits, and none of them touches a
file cited here.

## Answer in short

1. **"500 ms" is a lower bound, not the frame.** The record's FPS is
   `ceil(1 / t)` (`driver/samples/zephyr/glmark2/src/main.cpp:1115`, the same
   formula as upstream's `main-loop.cpp:178`). So 2 FPS means a mean frame t
   with 500 ms <= t < 1000 ms. Shadow and refract can differ by up to a factor
   of 2 and still both read 2. "The same 500 ms whatever the work" is the
   rounding of the record, not a fixed wait. The same holds for the earlier
   readings: refract's 1 FPS before the GPU mipmap chain meant t >= 1000 ms,
   and terrain's 4 FPS means 250 to 333 ms. `RESULTS.md` and
   `imx8mp-glmark2-gap.md` read the step the wrong way ("up to one FPS step
   faster", "333 to 500 ms").
2. **No wait or timeout sets the frame.** Every GPU wait on these paths is
   bounded at 2000 ms (`src/osal/zephyr_gc.c:438`, `:511`). A wait that hits
   the bound dumps the GPU state and resets it
   (`driver/src/vivante/gc_render.c:971-992`). The record shows
   `resets=absent`, and a 2 s timeout every frame would read 1 FPS, not 2.
   The only time quantum on these paths is the 1 ms poll sleep
   (`gc_render.c:969`), about 1 ms a wait.
3. **The frame is set by the CPU depth resolve, which reads non-cacheable
   memory one byte at a time.** Both scenes render depth into a 1600x1200
   depth texture every frame (twice the canvas) and then sample it. The GC
   back end turns the D24S8 surface into R32F texels on the CPU
   (`src/vivante/ds_cache.rs:606-613`): 1.92 M texels, 7.68 MB read and
   7.68 MB written, both in the carve-out, which is mapped Normal
   non-cacheable (`driver/src/vivante/gc_render.c:79-89`). Compiled for the
   board's target (`aarch64-unknown-none`, whose features are
   `+v8a,+strict-align,+neon`), the loop reads each texel with **three `ldrb`
   byte loads**: 5.76 M uncached loads per frame. The host compiles the same
   loop to wide SIMD loads, so it takes 0.3 ms there. The AM62P calibrates
   the cost: the PowerVR's resolve of the same 1600x1200 texture, one aligned
   32-bit load per texel on the same A53 core class and non-cacheable memory,
   took 349 ms a frame. Three loads where that loop had one puts this one at
   roughly 350 to 1000 ms, which covers the 500 to 1000 ms the record allows.
4. **The rest is small next to it.** The GPU work is about 10 to 25 ms
   (shadow) and 15 to 40 ms (refract), from the kicks below. The CPU waits
   outside the swap total about 10 to 15 ms (shadow) and 15 to 40 ms
   (refract). Cache maintenance over the two 7.7 MB ranges adds an estimated
   2 to 25 ms, and the present about 13 to 17 ms. No glGenerateMipmap takes
   the CPU path.

## How it was measured

- **The GL-call profile.** `tools/hostharness/glmark2/run.sh`, GC7000 build,
  with `YAKOGL_GC_CARVEOUT_BYTES=245642240 GM2_PROFILE=1 GM2_WRITE=0
  GM2_FRAMES=20 GM2_ENTRIES='^(shadow|refract)$'`. glmark2 was at
  `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the deps cache's
  `glmark2-src` checkout. The tree was `964f50f0`, unmodified. x86-64 host,
  one run, 19 counted frames per scene. Every entry ended ok with 0 GL errors
  and 0 allocations refused. The pictures are black because the host renders
  nothing.
- **Kicks, waits and CPU passes.** The same command was run on the same tree
  with temporary `glog!` lines (`gmdiag:`), then reverted. The lines were at:
  - every kick (`frame_submit`: fence, target address and size, stream
    words);
  - every draw (target, clear flags, index count, depth and colour state,
    textures);
  - every point that waits on the board when the GPU is behind:
    `frame_open`'s slot fence, `finish`, `finish_previous` and
    `gc_return_mipmap_chain`;
  - every GL-core `backend::flush` and `finish`, with its caller
    (`#[track_caller]`);
  - the CPU passes in `src/vivante`: `ds_cache::resolve_depth`, the fill,
    `copy_plane`, `seed_depth` and the `tiled_copy` refresh;
  - every `cache_flush` or `cache_invd` of 64 KiB or more in
    `src/osal/linux.c`, a catch-all for any CPU pass over GPU memory.

  Each frame below repeats identically over frames 2 to 20. The host OSAL
  completes a kick when it is submitted, so the host waits for nothing. The
  waits listed are the points where the board waits.
- **The loop's code on the board's target.** `cargo rustc -p yakogl --release
  --no-default-features --features
  osal-zephyr,gpu-vivante,shaderc-online,no-shader-table --target
  aarch64-unknown-none --lib -- --emit=asm`, with the toolchain of this host.
  The image build uses the same target with no `-C target-cpu`
  (`capi/yakogl_capi.cmake:20`, set by no board file) and the release profile.
  The inner loop of `ds_cache::resolve_depth`, inlined into
  `gc_depth_resolve`, is:

  ```
  ldrb w17, [x15, #2]      ; byte 2 of the D24S8 word
  ldrb w18, [x15, #3]      ; byte 3
  ldrb w18, [x15, #1]      ; byte 1 (byte 0, the stencil, is shifted out)
  ... orr / lsr #8
  ucvtf d1, w17 ; fmul d1, d1, d0 ; fcvt s1, d1   ; unorm24_depth
  str s1, [x8], #4         ; one R32F texel
  ```

  The loop reads `[u8; 4]` chunks of a byte slice
  (`src.as_chunks::<4>()`, `u32::from_ne_bytes(*s)`, `ds_cache.rs:610-611`).
  Under `+strict-align` the compiler may not assume that a `[u8; 4]` is
  4-byte aligned, so it loads the three bytes it needs one by one. It does
  not vectorise the loop. The surface itself is 64-byte aligned.

## The frames

### shadow (`scene-shadow.cpp`, 3 kicks a frame)

| kick | target | what it draws | stream words |
|---|---|---|---|
| K1 | window 800x600 | glmark2's canvas clear, colour and depth, alone in its kick: the next draw changes the target (`backend.rs:2902-2903`) | 308 |
| K2 | depth only, 1600x1200 (no colour, `target 0x0`) | depth clear quad plus the horse, 21,516 indices (7,172 triangles), depth test and write | 558 |
| K3 | window 800x600 | ground quad (4 vertices, samples the depth texture) plus the horse (21,516 indices), lit | 588 |

Between K2 and K3, the ground's `glDrawArrays` samples the depth texture:
`build_common` (`src/gl/draw.rs:3374`) -> `resolve_sampled_depth`
(`draw.rs:3322`) -> `depth_for_draw` (`src/gl/texture.rs:837`). That call
submits K2 (`texture.rs:850`: logged as `gl flush from texture.rs:850`), finds
that the GC7000 does not sample its depth buffer (`src/backend.rs:1858-1872`:
no GC arm), and calls `resolve_depth` (`texture.rs:856`). That function waits
for K2 (`texture.rs:709`, `flush()` then `finish()`) and calls the back end's
`gc_depth_resolve` (`texture.rs:729`, `src/vivante/backend.rs:2994`).

### refract (`scene-refract.cpp`, 13 kicks a frame)

| kick | target | what it draws | stream words |
|---|---|---|---|
| K1 | window 800x600 | the canvas clear, colour and depth | 308 |
| K2 | colour 1600x1200 RGBA8 (linear) plus depth 1600x1200 | colour and depth clear plus the bunny, 208,998 indices (69,666 triangles), front faces culled | 558 |
| K3..K12 | levels 1..10 of K2's colour target: 800x600, 400x300, 200x150, 100x75, 50x37, 25x18, 12x9, 6x4, 3x2, 1x1 | `glGenerateMipmap` on the GPU, one quad, one kick per level (`src/vivante/mipmap.rs:270-281`) | 328 each |
| K13 | window 800x600 | the bunny (208,998 indices), three textures: the depth texture, the mipmapped colour target, the image | 446 |

The chain is the GPU's ("the GPU made the 11-level chain of a 1600x1200
texture", 0 refused). The resolve runs before K13 is recorded, as in shadow.
On the host K2 has already completed by then, so the host log has no
`texture.rs:850` flush line for refract. On the board K2 and the chain are
still running, so `texture.rs:709` calls `finish()`.

### Per frame, both scenes

| | shadow | refract |
|---|---|---|
| **(1) GL-call CPU, host** (ms a frame, wall) | 0.312 in GL and EGL calls: `glDrawArrays` 0.306 (3 calls; the resolve is inside it, the `GlTextures` step: 42.2 us of the 43.7 us a draw); `glClear` 0.002 (2); `eglSwapBuffers` 0.001; the rest under 0.001 each | 0.297: `glDrawArrays` 0.280 (2; `GlTextures` 18.1 of 19.3 us a draw over 15 draws); `glGenerateMipmap` 0.010 (1); `glClear` 0.003 (2); the rest under 0.001 each |
| **(2) CPU passes over GPU memory** (all in the carve-out) | resolve: 1600x1200, reads 7,680,000 B of D24S8, writes 7,680,000 B of R32F; `cache_invd` 7,782,400 B (`ds_cache.rs:598`, the whole surface allocation) and `cache_flush` 7,680,000 B (`texture.rs:732`). No tiled copy, plane copy, seed or readback | the same resolve, same bytes. The chain is GPU work. Nothing else of 64 KiB or more |
| **(3) CPU waits on the GPU outside the swap** (board) | 1: `texture.rs:709` -> `backend.rs:3196` `finish`, which waits for every slot (`:3200-3205`), here K1 and K2 | 1 + up to 7: `texture.rs:709` -> `finish` (K2 to K12), plus `frame_open` (`backend.rs:2229-2233`) for K6 to K12, each of which reuses the slot of the kick four before it (`GC_FRAMES = 4`, `:77`). K6's slot is K2's, the 1600x1200 pass |
| the swap (board) | `eglSwapBuffers` (`src/gl/egl.rs:2413`) -> `finish_previous` (`backend.rs:3220`). It waits for the previous frame's K3, finished long before (the resolve takes far longer). In practice no wait | the same, for the previous frame's K13 |
| **(4) GPU work** | K1 0.48 Mpx, K2 1.92 Mpx clear plus the horse's depth, K3 at most 0.96 Mpx shaded | K1 0.48 Mpx, K2 1.92 Mpx clear plus the bunny, chain 0.64 Mpx over 10 kicks, K13 at most 0.48 Mpx with 3 textures |

The host's swap calls `finish()`, not `finish_previous()`. The harness window
is rendered in place, so `complete` is true at `egl.rs:2405`. The board's
glmark2 window is EGL's double buffer (`imx8mp-frame-floor.md`, "As built"),
which takes `finish_previous`. That difference is the harness's, not a
difference in these scenes.

### The GPU estimate

The model is `imx8mp-frame-floor.md` candidate 2: 3.9 ns a pixel with a
1-instruction shader, about 22 ns a pixel with real shaders (gc_slint). Every
colour target is linear, so no target gets early depth. Coverage of the
horse and the bunny is not known on the host (nothing is rasterised), so the
figures are ranges.

| scene | clears and quads (exact pixels) | models (coverage assumed 15 to 40 %) | estimate |
|---|---|---|---|
| shadow | K1 0.48 Mpx + K2 1.92 Mpx = 2.40 Mpx, 9.4 ms | horse depth 0.3 to 0.8 Mpx, ground at most 0.48 Mpx, horse lit about 0.1 to 0.2 Mpx: 2 to 15 ms | **about 10 to 25 ms** |
| refract | K1 0.48 + K2 colour and depth 1.92 = 2.40 Mpx, 9.4 ms; the chain 0.64 Mpx, 2.5 ms | bunny into the 1600x1200 target 0.3 to 0.8 Mpx, bunny with 3 textures 0.1 to 0.2 Mpx, and 69,666 triangles of vertex work twice: 4 to 25 ms | **about 15 to 40 ms** |

The AM62P's GPU did the shadow-map pair in 6.4 ms a frame
(`docs/notes/pvr-depth-sample.md:104-109`). Either way the GPU is a few
percent of a frame of 500 ms or more.

### The waits and their bounds, with values

| wait | call site | bound | on timeout | in these scenes |
|---|---|---|---|---|
| the resolve's `finish()` | `texture.rs:709` -> `backend.rs:3196-3208` -> `wait_fence(f, -1)` (`:2414-2419`) | -1 becomes `GC_WAIT_FOREVER_MS` = 2000 ms (`zephyr_gc.c:438`, `:510-511`) | `gc_render_wait` logs, dumps the GPU state, recovers (a GPU reset) and returns -ECANCELED or -ETIMEDOUT (`gc_render.c:971-992`). The GL core keeps the old texels (`texture.rs:710`) | 1 a frame; it covers K1 and K2 (shadow), K2 to K12 (refract) |
| a stream slot reused | `frame_open`, `backend.rs:2229-2233` | 2000 ms, as above | the draw is refused (`-1`) | refract: K6 to K12 (K6 waits for K2) |
| the swap | `egl.rs:2413` -> `gc_finish_previous`, `backend.rs:3220-3233` | 2000 ms | `EGL_CONTEXT_LOST` | normally no wait (see above) |
| the mipmap chain read back | `gc_return_mipmap_chain`, `mipmap.rs:352-365` | 2000 ms | logged once | not called in these frames (no CPU read of the chain) |
| the full free queue | `gc_render.c:347`, `GC_DEFER_WAIT_MS` 2000 (`gc_render_defer.h:68`) | 2000 ms | | not reached (no allocation churn per frame) |
| a GPU that stops retiring | `gc_render.c:950`, `s_hang_ms` | | as above | not reached |
| the poll inside each wait | `gc_render.c:968-969`, `k_sleep(K_MSEC(1))` | | | 1.0 to 1.1 ms of granularity a wait (10,000 Hz tick, `imx8mp-frame-floor.md` candidate 3) |
| the latch on present | board layer (`board_imx8mp.c:479-510`, `latched`) | one panel period | | at most 17 ms |

**Can a wait set the frame?** No. A wait that times out takes 2000 ms and
resets the GPU. Even one such frame per 10 s scene would show as a reset in
the verdict line. The `a917d123` rows have `resets=absent`. One timeout every
frame would read 1 FPS, not 2. Without a timeout, a wait lasts as long as
the GPU work it covers (10 to 40 ms, above) rounded up to the 1 ms poll. No
fence that never signals can be in play: it would end in the 2000 ms bound
and a reset.

## Accounting for the board frame (500 to 1000 ms)

| part | shadow | refract | basis |
|---|---|---|---|
| CPU depth resolve, 5.76 M uncached byte loads and 1.92 M uncached 4-byte stores | **about 350 to 1000 ms** | **the same** | the AM62P loop: 1 aligned load per texel, 349 ms a frame for 1.92 M texels (`pvr-depth-sample.md:104-111`: 355.20 ms a frame with 6.35 ms of GPU). This loop has 3 loads per texel: the same 349 ms if the in-order A53 overlaps the three loads of a texel, close to 3 times that if it serialises them. Not measured on the i.MX8MP |
| cache maintenance of the two ranges | 2 to 25 ms | the same | about 241,000 64-byte lines a frame (`ds_cache.rs:598`, `texture.rs:732`; `sys_cache_data_invd_range` / `_flush_range`, `gc_render.c:461-469`), pure cost on a non-cacheable mapping. Estimate |
| CPU waits outside the swap | 10 to 15 ms | 15 to 40 ms | the GPU work they cover, plus the 1 ms poll |
| present (window copy plus latch) | 13 to 17 ms | the same | `imx8mp-frame-floor.md`, "Step 3 result": 12.3 to 12.9 ms of copy, plus the latch at one buffer |
| GL calls outside the resolve | under 1 ms | under 1 ms | host 0.01 to 0.02 ms a frame outside `GlTextures`, even at 10 to 50 times |
| GPU, overlapped except where waited for | (10 to 25 ms, inside the wait above) | (15 to 40 ms) | the kicks above |
| **sum** | **about 380 to 1060 ms** | **about 400 to 1100 ms** | |

The record's window is 500 to 1000 ms for each. The resolve is the only term
that can reach it. Everything else together is about 30 to 100 ms. Shadow's
reading did not move when the loop went from a per-texel closure to whole
rows (`gc-depth-resolve.md`, `0d21ab47`), and the compiled code shows why:
both forms make the same uncached byte loads, and the change removed only
cached-memory work. Shadow did not move when refract did (1 -> 2 FPS)
because refract's gain came from removing its CPU mipmap filter, which
shadow never had.

## Ranked causes, the fix for each, and how the X2 would confirm it

The X2 stores w0..w2 of a record, so v0..v3 of each PERF page reach the
board's verdict. Page 4 is the one item fb-counters is adding (selector
`0x1004`). Its values here are as the plan names them: v0 GPU mipmap chains,
v1 CPU mipmap fallbacks, v2 CPU waits outside the swap, v3 KiB of CPU passes.
They are given per frame; scale them to whatever unit fb-counters chooses
(per frame or per 100 frames). Pages 0, 1 and 3 are `gm_perf_page.h:17-29`.
Every value saturates at 0xffff = 65,535: page 0 v1 (frame) and v2 (gl), and
page 3 v0 and v1 (kcycles), read 65,535 at these frame times. The usable
frame time is **10 000 / page 0 v0 ms**: a scene is 10 s, and glmark2 ends it
at the first frame past that (`scene.cpp:116`).

### 1. The CPU depth resolve's uncached byte loads (both scenes, about 350 to 1000 ms)

- **Fix, cheap (host first).** Read the surface with aligned wide loads in
  `src/vivante/ds_cache.rs:606-613`. View the row as `&[u32]` (the surface
  is 64-byte aligned and the row stride is a multiple of 4), or copy each
  row into a cached stack buffer with 16-byte or wider loads and convert
  from there. Fewer uncached loads: 5.76 M -> 1.92 M (aligned `u32`) or
  0.48 M (16-byte loads). That is 3 to 12 times, an estimated 50 to 350 ms a
  frame left. Host proof: `tests/gc_depth_resolve.rs` stays bit-exact, plus a
  check of the `aarch64-unknown-none` code (no `ldrb` in the loop), the way
  this note read it. The same `as_chunks::<4>` and `from_ne_bytes` form is in
  `copy_plane` (`ds_cache.rs:474-475`) and `seed_depth` (`:533-534`); they
  are not per frame here.
- **Fix, real.** No CPU resolve at all: the texture unit samples the D24S8
  surface, as the PowerVR does since `86d0b482` (AM62P shadow 353 -> 10.6 ms,
  refract 375 -> 37 ms; `pvr-depth-sample.md`). The GC needs the board
  experiment of `gc-depth-sample.md` (item gt-depth-tpu) first, then
  `samples_rendered_depth` gets a GC arm (`src/backend.rs:1858-1872`). This
  removes the resolve, its two cache ranges and the resolve's `finish()`
  (cause 3).
- **Confirms the cause (before any fix).** Page 4 v3 = 15,000 KiB a frame
  (7,500 read plus 7,500 written; 7,500 if only reads are counted) in both
  scenes. Page 0 v2 (gl) = 65,535 while page 1 v0 (wait us) is about 10,000
  to 40,000. The CPU is busy inside GL calls, not waiting. Page 0 v0 = 10 to
  20 frames.
- **Confirms the cheap fix.** Page 4 v3 unchanged (same bytes, other loads).
  Page 0 v0 rises to about 30 to 200 frames (50 to 350 ms a frame). Page 0
  v2 may fall below 65,535.
- **Confirms the real fix.** Page 4 v3 = 0 and page 4 v2 drops by 1 in both
  scenes. Page 0 v2 (gl) drops to a few ms (well under 65,535). Page 0 v0
  rises to about 250 to 500 frames (20 to 40 ms a frame, GPU and present).

### 2. Cache maintenance on the non-cacheable carve-out (both, an estimated 2 to 25 ms)

- **Fix.** `z_cache_flush` and `z_cache_invd` (`src/osal/zephyr_gc.c:359-369`)
  and `gc_render_cache_flush` / `_invd` (`gc_render.c:461-469`) skip the DC
  range operations when the range is in the carve-out, which is Normal
  non-cacheable for every user (`zephyr_gc.c:154`; `z_gpu_write` already
  calls it a no-op, `:343-345`). Host: a counting stand-in. `gc_render.c` is
  fence-irq's file now, so this goes after that item.
- **Confirms.** No PERF value isolates it. Page 0 v0 rises by an estimated 1
  to 5 % at today's frame. Measure it after cause 1's fix, when it is a
  larger share.

### 3. CPU waits on the GPU outside the swap (shadow 10 to 15 ms, refract 15 to 40 ms)

- **Fixes.**
  - The fence wait woken by the interrupt instead of the 1 ms poll (item
    fence-irq): it removes the up to 1 ms of rounding per wait.
  - The resolve waits for its own kick, not for all four slots: the table's
    `wait_kick: None` (`backend.rs:3380`), so `finish()` at `texture.rs:709`
    waits for refract's chain as well. Done (item resolve-own-kick): the
    table takes `gc_wait_kick` (`kick::wait_kick`, one kick's fence, the
    open frame submitted first), the GL core's resolve waits with it, and
    the back end's resolve waits for the surface's `written_by`, the kick
    the last depth-writing draw was recorded in (ds_cache.rs `drawn`).
    Host test `tests/gc_resolve_wait_kick.rs`; not measured on a board.
  - The chain in one kick, not ten (`mipmap.rs:270-281`), which also removes
    refract's `frame_open` waits.

  These matter only after cause 1's fix.
- **Confirms.** Page 4 v2 = 1 a frame for shadow and 1 to 8 for refract.
  Page 1 v1 (waits per 100 frames) = about 100 to 200 (shadow) and 200 to
  900 (refract). Page 1 v2 (sleeps per 100 frames) is about 100 times the
  waited ms: 1,000 to 1,500 (shadow), 1,500 to 4,000 (refract). With
  fence-irq, page 1 v2 goes to 0. With one kick a chain, refract's page 4 v2
  goes to 1.

### 4. GPU work (shadow about 10 to 25 ms, refract 15 to 40 ms)

- **Fix.** Only after causes 1 to 3. Tiled colour targets with tile status,
  fast clear and early depth (item tiled-target-study), since every pass
  here is a linear target with a full-target clear quad
  (`backend.rs:2345-2395`). Also no 1600x1200 depth clear by quad.
- **Confirms.** Page 1 v3 (GPU busy us a frame) = about 10,000 to 25,000
  (shadow) and 15,000 to 40,000 (refract). It does not saturate, and against
  a frame of 500 ms or more it shows that the GPU is idle about 95 % of the
  time. Page 3 v3 (GPU KiB written per interval): at least about 11,300 for
  shadow (two clears: 7,500 depth plus 1,875 colour plus 1,875 depth) and at
  least about 21,000 for refract (7,500 colour plus 7,500 depth plus 3,750
  canvas plus 2,500 chain). Page 3 v2 is the reads, a few MiB more. Page 3
  v0 and v1 saturate.

### 5. The CPU mipmap fallback (refract only): not taken

- **Evidence.** The host made refract's chain on the GPU, 0 refused
  (`gc-mipmap-cpu-cases.md`; this run). Shadow has no mipmap.
- **Confirms.** Page 4 v0 = 1 a frame (refract), 0 (shadow); page 4 v1 = 0.
  If the board ever showed v1 = 1 for refract, its page 4 v3 would also grow
  by about 10,000 KiB (the box filter's 7,500 read plus 2,500 written).

### Ruled out

- **A wait or timeout that ends at a fixed time:** see "Can a wait set the
  frame?". Page 1 v0 (wait us) stays far below the frame (about 10,000 to
  40,000 against 500,000 or more), and the verdict shows no reset.
- **The flat 500 ms:** the record's `ceil` rounding (above). Page 0 v0 gives
  the two scenes' own times.
- **GL-call CPU outside the resolve, tiled copies, plane copies, seeds and
  readbacks:** none of these appear in the frame. The host shows under
  0.02 ms a frame outside the resolve.

## What the X2 baseline should record (item x2-gm-before)

For each of shadow and refract, page 0, page 1, page 3 and, once on main,
page 4, with v0..v3 decoded:

- page 0 v0, the frames: the frame time, 10 000 / v0 ms;
- page 0 v2, 65,535 or not: whether the CPU work is in GL calls;
- page 1 v0..v3, the waits: whether time goes to waiting, and the GPU busy
  time;
- page 3 v2 and v3: the GPU traffic;
- page 4 v0..v3: chains, fallbacks, waits and bytes, against the values
  above.

Prediction for the baseline:

- shadow and refract: page 0 v0 between 10 and 20, page 0 v2 = 65,535, page
  1 v3 under 40,000, page 4 v3 about 15,000;
- refract: page 4 v0 = 1 and v1 = 0.

A baseline outside these values rejects this note's ranking.

## What this note does not show

- The per-load cost of the i.MX8MP's A53 on its carve-out was not measured.
  The 350 to 1000 ms range rests on the AM62P's resolve (one load per texel,
  349 ms) and on the loop's instruction count, not on an i.MX8MP reading.
  Whether the A53 overlaps three independent uncached loads is the open
  factor. Page 0 v0 of the X2 baseline measures it.
- The coverage of the models and the vertex cost of 69,666 triangles on the
  GC7000UL are assumptions. The GPU ranges are estimates (page 1 v3 measures
  them).
- The cost of the DC range operations on a non-cacheable range is an
  estimate.
- The `.s` was produced by this host's toolchain. The board image's
  toolchain may differ in version, but `+strict-align` is the target's, so
  the byte loads do not depend on the version.
