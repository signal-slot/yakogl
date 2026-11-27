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
3. **The frame is set by the CPU depth resolve, which at `964f50f0` read
   non-cacheable memory one byte at a time.** Both scenes render depth into a 1600x1200
   depth texture every frame (twice the canvas) and then sample it. The GC
   back end turns the D24S8 surface into R32F texels on the CPU
   (`src/vivante/ds_cache.rs:606-613`): 1.92 M texels, 7.68 MB read and
   7.68 MB written, both in the carve-out, which is mapped Normal
   non-cacheable (`driver/src/vivante/gc_render.c:79-89`). At the tree of
   this measurement (`964f50f0`), compiled for the board's target
   (`aarch64-unknown-none`, whose features are `+v8a,+strict-align,+neon`),
   the loop read each texel with **three `ldrb` byte loads**: 5.76 M
   uncached loads per frame. Today's `resolve_row` (`ds_cache.rs:822`,
   `25fa1a96`) reads the aligned middle of a row with one `u128` load
   (`ldp`) per four words, checked at `5483d7cf`: the byte loads are what
   the 2 FPS baseline paid, and the X2 rows measure what is left. The host
   compiles the same loop to wide SIMD loads, so it takes 0.3 ms there. The AM62P calibrates
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
  not vectorise the loop. The surface itself is 64-byte aligned. This is
  `964f50f0`'s loop: `resolve_row` (landed in `25fa1a96`) issues the wide
  `ldp` load instead, checked at `5483d7cf`.

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
v1 CPU mipmap fallbacks, v2 CPU waits outside the swap, v3 KiB of the CPU
passes over the carve-out that are counted today: the CPU mipmap filter
(`mipmap.rs:341`) and the CPU tiled copy (`tiled_copy.rs:559`) only
(`gm_perf_page.h:30-34`). **v3 does not count the CPU depth resolve**:
`ds_cache.rs resolve_depth` adds no bytes, so v3 reads 0 in shadow and refract
in every mode (the six cells of gm-x2-expect). A resolve-bytes counter does
not exist yet; it would be one `fetch_add` in `resolve_depth` after
`wait_kick` succeeds (not on its early returns, `ds_cache.rs:767-777`),
owned by `src/vivante/**` (plans #400 and #410 serialize it), with its own
test. Until it lands the resolve shows in page 0 v2 (gl ms), page 0 v5 (frame
to frame) and page 1 v4 (kicks per 100 frames).
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
  **Done on the host** (item resolve-wide): `resolve_row` in `ds_cache.rs`
  reads the 16-byte aligned middle of each row with one volatile `u128`
  load (`ldp x, x`) per four words and the head and tail with one `ldr w`
  a word; only a row off the 4-byte boundary (no surface of the cache) keeps
  the `ldrb` loop. Checked with
  `cargo rustc -p yakogl --release --lib --no-default-features --features
  osal-zephyr,gpu-vivante,shaderc-online,no-shader-table --target
  aarch64-unknown-none -- --emit=asm` and the loads of `resolve_row` in the
  `.s` file. The X2's ms a frame is still to be measured.
- **Fix, real.** No CPU resolve at all: the texture unit samples the D24S8
  surface, as the PowerVR does since `86d0b482` (AM62P shadow 353 -> 10.6 ms,
  refract 375 -> 37 ms; `pvr-depth-sample.md`). The GC needs the board
  experiment of `gc-depth-sample.md` (item gt-depth-tpu) first, then
  `samples_rendered_depth` gets a GC arm (`src/backend.rs:1858-1872`). This
  removes the resolve, its two cache ranges and the resolve's `finish()`
  (cause 3).
- **Confirms the cause (before any fix).** Page 4 v3 would be 15,000 KiB a
  frame (7,500 read plus 7,500 written) only once the resolve counter exists;
  today it reads 0. Page 0 v2 (gl) = 65,535 while page 1 v0 (wait us) is about 10,000
  to 40,000. The CPU is busy inside GL calls, not waiting. Page 0 v0 = 10 to
  20 frames.
- **Confirms the cheap fix.** Same bytes, other loads: no counter moves.
  Page 0 v0 rises to about 30 to 200 frames (50 to 350 ms a frame). Page 0
  v2 may fall below 65,535.
- **Confirms the real fix.** Page 4 v2 drops by 1 in both
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
  If the board ever showed v1 = 1 for refract, its page 4 v3 would grow
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

Kept for the X2; the figures go in the before/after table of "glmark2
shadow, refract and terrain, before and after, measured on the Armadillo-X2 in
place of the Verdin iMX8MP [plan #417]" at the end of this note (the X2 runs
only once its container is stopped: `x2-glmark2-slow-scenes.md`, last
section).

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
  1 v3 under 40,000, page 4 v3 = 0 (the resolve is not counted);
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

## Plan #355: host state and what the X2 must measure

Written at `a54d9357`. Every figure is a host count or cited from a note;
**no X2 millisecond exists** (`x2-glmark2-slow-scenes.md`). Issues #1204 and
#15's scene rows are filled by X2 sessions only.

### What landed

- `9cdeff6d` (W1-W5: `src/vivante/tiled_rt.rs`, `early_z.rs`) behind the knob
  `gc_tiled_rt`, default off (`src/vivante/backend.rs:455`); `342c286a` and
  `ad0e070e` after it (see `gc-slow-terrain.md`, same section).
- `tests/gc_scene_kicks_tiled.rs` counts knob off and on per scene.
- Plan #328's wide depth-resolve loads are in `src/vivante/ds_cache.rs:742-777`
  (`resolve_row`, `as_chunks::<4>` with `from_ne_bytes`). The 350 to 1000 ms
  of cause 1 above was estimated for the byte loads before them; the host
  cannot say what remains. That is the first X2 number to read.

### The measured cause, as the host gives it

- **The knob takes neither scene.** `tiled_rt_spec`
  (`src/vivante/backend.rs:736-750`) and `tiled_rt.rs:140-147` accept a
  4-byte colour target with the depth test on; the harness's shadow and refract
  passes (1600x1200) render with a depth **texture**, which `ds_cache::pe_private`
  (`src/vivante/ds_cache.rs:263-270`) refuses (`tests/gc_scene_kicks_tiled.rs:44-46`).
- **Counts, a frame, knob off and on, identical** (`:26-42`, `Lag`): shadow
  2 kicks, 1 wait outside the swap, 1 CPU depth resolve, RS other 3, 3 draws;
  refract 3 kicks, 1 wait, 1 resolve, RS other 4, 12 draws; RS tile-in, copy
  back and early-depth draws 0 in both. The ceilings are 3 and 4 kicks
  (`:80-87`).
- **What dominates stays:** the CPU depth resolve of a 1600x1200 depth
  texture (cause 1, above; 1.92 M texels), and the GPU work of 10 to 25 ms
  (shadow) and 15 to 40 ms (refract), neither moved by W4/W5. The lever is
  depth-sample-gc (`gc-depth-sample-wire.md`; `gc-depth-sample.md`).

### Predicted gain

`gc-slow-host-counts.md:541-601`: **0 ms** for both. Even if early depth took
the depth pass, the cited range is 0 to 1 ms (shadow) and 0 to 2 ms
(refract). Reading rule: a difference within the spread of repeated runs is
the prediction; a slowdown of 0.2 to 1 ms (800x600) or 1 to 4 ms (1600x1200)
a job would be a tiled shadow the host does not show.

### Rows the X2 sessions fill

Kept for the X2; the figures go in the Armadillo-X2 before/after table at the
end of this note.

Empty until a session runs; one table per scene (shadow, refract).

| row | commit | gc_tiled_rt | ms a frame | board.sh verdict line | record words |
|---|---|---|---:|---|---|
| before | 72972ba4 | n/a (not built) | - | verdict= stop: not run (the 72972ba4 command stopped after terrain's not_back: `board.sh x2 terrain-nosel rc=5 verdict=not_back boot=kexec-dev DEV boot_verdict=not_back result=none resets=absent record=absent back_after=?s`) | - |
| tip | b947cc22 (stop 2026-11-27, not run) | off | - | verdict= stop: not run (same) | - |
| tip | | on | | | |
| D (on minus off) | | | | | |

Stop, 2026-11-24 (item x2-before-now): still empty. The board answered
from the leased worker, but no jump was made: an autostarting podman
container (`slint-launcher`) makes `board.sh x2 run` refuse
(`something is running (containers 1)`); the 72972ba4 command stopped at its first scene (terrain,
`no verdict line, rc=3`), so shadow and refract were not run. Details: `x2-glmark2-slow-scenes.md`, "Stop,
2026-11-24".

Stop, 2026-11-25 (item x2-session-a, commit `3c3fe33b`, knob off and on):
still empty, so no gain or loss is measured and no cause is confirmed on the
board. The board answered from the leased worker and `tools/board.sh x2
status` read `containers 1`: `slint-launcher` still runs, so `board.sh x2 run`
would refuse the jump and none was made. Details: `x2-glmark2-slow-scenes.md`,
"Session A: stop, 2026-11-25".

Stop, 2026-11-27 (item x2-before-rerun; measured on the Armadillo-X2
standing in for the Verdin iMX8MP, record
`records/x2/20261127-0954-before-72972ba4/`): still empty for shadow and
refract. The loader's X2 crash list held (`crash: i.MX 8M Plus
(fsl,imx8mp): gpio-mxc and irqsteer rows are let through`, `jump=ok`), and
the first scene of the 72972ba4 command, terrain, ran to `glmark2: PASS` on
the console. After the reset the board refused the task machine's ssh key,
which lives on the board's tmpfs only. The run therefore ended
`verdict=not_back` (rc 5), the command stopped without a retry, and shadow,
refract and the tip `b947cc22` were not run. Details:
`x2-glmark2-slow-scenes.md`, "Stop, 2026-11-27 (plan #355, item
x2-before-rerun)".

## glmark2 shadow, refract and terrain, before and after, measured on the Armadillo-X2 in place of the Verdin iMX8MP [plan #417]

The Verdin iMX8MP is away (off the farm since 2026-11-16, no return date). The
owner's rule of 2026-11-27 moves the i.MX8MP / GC7000UL board work to the
Armadillo-X2 (same i.MX 8M Plus SoC and GC7000UL GPU), recorded rows included,
so every figure below is **measured on the Armadillo-X2 in place of the Verdin
iMX8MP** and each record says so. The table's before rows hold the
2026-11-27 session (item gm-before-x2): the images were built and shadow
was started, but the kexec loader refused the jump and nothing ran, so no
ms a frame exists yet and every after cell is still a placeholder. The
container that stopped the 2026-11-24 and 2026-11-25 sessions can now be
stopped for a run (`X2_STOP_CONTAINERS=1`, `225c6956`;
`docs/notes/x2-glmark2-slow-scenes.md`, last section); what stops a run
today is the loader's interrupt-chip list, recorded in `gc-depth-sample.md`,
"The X2 session of 2026-11-27".

### What each session records

- the origin/main commit the image was built from (8 hex digits at least);
- the image sha256 of the glmark2 image, built for the X2 with
  `IMX8MP_BOARD=x2` (armadillo_x2/mimx8ml8/a53; the Verdin build is the
  default and is not used);
- the record directory, `records/x2/<YYYYMMDD-HHMM>-<name>-<sha8>/`
  (gitignored, redacted; no board address or UART path in it or here);
- PERF page 0 v0, the frames in the 10 s scene, and the ms a frame,
  10 000 / v0 (page 0 v0 saturates at 65,535, so a value there is a bound);
- PERF page 4 v3, the KiB a frame of the counted CPU passes (CPU mipmap
  filter and CPU tiled copy; not the depth resolve, so 0 for shadow and
  refract until a resolve counter lands); the figure that shows the resolve
  is page 0 v2, page 0 v5 and page 1 v4 (page 0 v5 and page 1 v4 are v5 and
  v4, in w3, which the X2 does not store: read them from a host or
  Verdin-style record, or leave the cell empty);
- the pages are read at selectors `SEL=0x1000`, `0x1001`, `0x1003` and
  `0x1004` (page 4 is `0x1004`); the X2 stores w0..w2, so v0..v3 come from
  these pages;
- for each scene, shadow, refract and terrain, one session per row; terrain is
  the control (expected unchanged by the depth-sample default).

### Expected effect

- **Before** (commit with `25fa1a96` and `e320630d`, CPU resolve with wide
  loads, cause 1 "Fix, cheap"): the byte loads are gone, so ms a frame falls
  from the 500 to 1000 ms of the 2 FPS baseline; how far is unknown until
  measured. The resolve still runs: page 0 v2 stays high.
- **After** (depth-sample default, outcome (a) or (b) of
  `gc-depth-sample-wire.md`): no CPU resolve, so page 0 v2 falls to a few ms for
  shadow and refract, and the frame falls to the GPU and wait costs (causes 3
  and 4, an estimated 10 to 40 ms), as the PowerVR got in `21099fcd`. Terrain
  is expected unchanged (`gc-slow-terrain.md`). If the verdict is (c), the CPU
  fixes stay and there is no "after" beyond them.

### Table (before rows: the 2026-11-27 attempt, no jump; measured on the Armadillo-X2 in place of the Verdin iMX8MP)

| scene | when | origin/main commit | image sha256 | records/x2/ directory | page 0 v0 | page 4 v3 KiB (CPU mip and tiled copy; not the resolve) | page 0 v2 (gl; shows the resolve) | page 0 v5 | page 1 v4 | ms a frame |
|---|---|---|---|---|---:|---:|---:|---:|---:|---:|
| shadow | before | `02c5c9af` | `2247c47eedb058d23f7a6fd3570af75134487835c6597d50d5d6238d64589c15` | `records/x2/20261127-0538-gm-before-shadow-02c5c9af/` | — | — | — | — | — | no figure: the jump was refused |
| shadow | after | — | — | records/x2/— | — | — | — | — | — | — |
| refract | before | `02c5c9af` | `8feb2585a44db698171221bdaedee6d1726433952eeae393bbb3d35e2ebfd1fb` | none (dry run only) | — | — | — | — | — | no figure: not reached |
| refract | after | — | — | records/x2/— | — | — | — | — | — | — |
| terrain (control) | before | `02c5c9af` | `b2057bb8cb2d09b65d5164a3e3b6731c23e6eadf35f261ce561154a2fdb24658` | none (dry run only) | — | — | — | — | — | no figure: not reached |
| terrain (control) | after | — | — | records/x2/— | — | — | — | — | — | — |

Stop, 2026-11-27 (item gm-before-x2, measured on the Armadillo-X2 in place
of the Verdin iMX8MP): shadow's one run (`shadow/20261127-0540-shadow-nosel-2247c47e/`
inside the record, no selector) never left Linux. `x2 status` read
`containers 0`, every image carried `gc_wdog_arm: armed`, and all 15 dry
runs (three scenes x no selector, `0x1000`, `0x1001`, `0x1003`, `0x1004`)
returned rc 0; then `kexec_zephyr --crash` refused the jump because
`crash_irq_chips` (`4c0626d1`) accepts no interrupt on the X2's `gpio-mxc`
or `irqsteer` chips — it printed "not triggering", brought the CPUs back
and unloaded the crash image, and the runner's `not_back` line describes a
jump that never happened. The runner stopped without retry, so refract and
terrain were not run; their before rows hold only their built images. The
record's `meta.txt` says `board_measured=Armadillo-X2 (i.MX 8M Plus,
GC7000UL), stands in for the Verdin iMX8MP`. A rerun can reuse the three
sha256 values above (images from `02c5c9af`, about 1.5 min a build on that
machine) once the loader accepts the X2 — the change to
`driver/tools/kexec-zephyr/**` named in `gc-depth-sample.md`'s session
section. Until it lands no cell of this table can fill.
