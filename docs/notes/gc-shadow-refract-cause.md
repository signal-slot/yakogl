<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: where shadow's and refract's frames go at the tip, and why W1-W5 do not move them

A host note for plan #355 (item sr-depth-cause; issues #1204 and #15).
glmark2 `shadow` and `refract` on the i.MX8MP. Nothing here ran on a
board: every board figure is an estimate with its basis written beside it, to
be replaced by the Armadillo-X2 sessions of the plan (`x2-glmark2-slow-scenes.md`
has no ms a frame yet). Read at `7c2af19d`. The earlier notes stay as they are
(`gc-slow-shadow-refract.md` is owned by the board items).

## What was run

1. The existing harness, `tests/gc_scene_kicks.rs`:
   `cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_scene_kicks -- --nocapture --test-threads=1`.
   Frames 3 to 6 of each scene, all four identical.
2. The new timing test, `tests/gc_depth_resolve_cost.rs`:
   `cargo test --release --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_resolve_cost -- --ignored --nocapture --test-threads=1`.
   It resolves a 1600x1200 D24S8 surface (the pattern of `gc_depth_resolve.rs`,
   so every bit of a word is used) through the real `ds_cache::resolve_depth`,
   30 runs, best and mean. Two references run on the same bytes: a `memcpy`
   of the surface, and the byte-wise `u8` loop the board ran before
   resolve-wide. No assertion is about time; the last texel is checked
   against the rule.

## Counts, a frame at the tip

| scene | model | kicks | waits outside the swap | `finish()` | CPU tiled copies | CPU depth resolves |
|---|---|---:|---:|---:|---:|---:|
| shadow | `Lag` (the most the board pays) | 2 | 1 | 0 | 0 | 1 |
| shadow | `Done` | 2 | 0 | 0 | 0 | 1 |
| refract | `Lag` | 3 | 1 | 0 | 0 | 1 |
| refract | `Done` | 3 | 0 | 0 | 0 | 1 |
| terrain (for scale) | `Lag` | 11 | 11 | 0 | 0 | 0 |
| terrain | `Done` | 11 | 0 | 0 | 0 | 0 |

Shape of the frame, from the harness's own trace (frame 6, `Lag`):

* shadow: `kick@ground, wait@ground, kick@swap`. The canvas clear and the
  1600x1200 depth pass share one stream; the ground draw's depth resolve
  submits it and waits for it (`wait_kick`, the surface's `written_by`); the
  ground and the lit horse are the second kick, at the swap.
* refract: `kick@its chain, kick@its chain, wait@bunny, kick@swap`. The
  colour+depth pass and its 11-level GPU chain are kicks of their own; the
  bunny's draw resolves the depth, waits, and the window pass goes at the swap.

These are lower than the ceilings in `gc_scene_kicks.rs` (shadow 3 kicks,
refract 4 kicks and 2 waits, terrain 14): `gc-frames-ring` has landed and the
ceilings and their comments were not lowered with it. That is a stale
ceiling, not a failure (the test passes); a later commit that owns the file
should tighten them to the counts above. Not done here (outside the item).

One CPU depth resolve a frame in each scene, of a 1600x1200 texture: 1.92 M
texels, 7.68 MB read from the carve-out and 7.68 MB written to the R32F
texels (`gc-slow-shadow-refract.md`, "(2) CPU passes over GPU memory"). The
cache maintenance that used to go with it is gone (no cache maintenance on the
carve-out landed), and the wait is for one kick, not all four slots.

## Host timings of the resolve

`resolve_depth` of a 1600x1200 surface (6400 B a row, 1216 rows), release
build, host memory, no-op cache calls, on the task machine (x86_64 KVM guest, 12th Gen Intel Core i7-1255U):

| scene | `resolve_depth` best | mean | ns a texel (best) | `memcpy` of the surface | byte-wise `u8` loop |
|---|---:|---:|---:|---:|---:|
| shadow | 2.144 ms | 2.397 ms | 1.116 | 1.450 ms (0.755 ns) | 1.721 ms (0.897 ns) |
| refract | 1.906 ms | 2.150 ms | 0.993 | 1.243 ms (0.647 ns) | 1.494 ms (0.778 ns) |

The two scenes run the same code on the same size; the 10 % between them is
the run-to-run spread. Use about 1.0 to 1.1 ns a texel, 1.9 to 2.1 ms a
resolve. (`gc-slow-shadow-refract.md` has 0.3 ms for the byte loop on the host;
that is the GL-call time without the resolve. The 1024x1024 print of
`gc_depth_resolve.rs` is another size.)

What the host number says: the *compute* of the resolve is cheap, and the host
compiles the byte-wise loop to the same speed as the wide one (4 `u8` loads
vectorised, 0.78 to 0.90 ns), so the host cannot show the effect of
resolve-wide at all. The host timing is therefore only the floor; the cost on
the board is the memory type, below.

## Scaling to the A53 (aarch64, the i.MX8MP)

Basis, in the order of weight:

1. **The AM62P calibration.** The PowerVR's resolve of the same 1600x1200
   texture, one aligned 32-bit load a texel from non-cacheable memory on an A53
   (the same core class), took 349 ms a frame with 6.35 ms of GPU
   (`pvr-depth-sample.md`, board measurement 2026-10-27, 355.20 ms a frame).
   That is **182 ns a texel**, one uncached load and one store included,
   about 180 times the host's 1.0 ns. It is a latency-bound figure: the
   texel rate hangs on the bus, not on the A53's ALU.
2. **The loop now.** `resolve_row` reads the carve-out with one volatile
   `u128` load per four texels: 0.48 M loads a frame, where the AM62P loop had
   1.92 M and the byte-wise loop 5.76 M. At the AM62P's cost per load
   (about 180 ns), 0.48 M loads are **about 87 ms**; if the `u128` load is
   issued as two 8-byte transactions, **about 175 ms**. The stores go to
   non-cacheable memory through the write buffer, 7.68 MB: a few ms to some
   tens of ms (unmeasured), on top.
3. **The compute floor.** The host's 1.0 ns a texel scaled by 5 to 10 for an
   in-order 1.8 GHz A53 against the host: **7 to 20 ms** even if the loads were
   free (the convert is one `ucvtf`, a multiply and a narrow, a store).

So the CPU resolve is **about 90 to 200 ms a frame on the board now**, inside the
50 to 350 ms the earlier note wrote (`gc-slow-shadow-refract.md`, "Fix,
cheap") and the 500 ms frame floor of the FPS record. Unmeasured on the
i.MX8MP: the memory path differs from the AM62P's (the LPDDR4 controller, the
carve-out's attribute is Normal non-cacheable on both, `gc_render.c:79-89`).
The X2 session's page 0 v0 is the check: a frame of about 100 to 250 ms for
each scene at the tip is this estimate right.

## The cause, per scene, at the tip

**shadow.** One CPU depth resolve of 1.92 M texels, 90 to 200 ms (7.68 MB read
uncached). Behind it the one `wait_kick` for the depth pass: the GPU work it
covers is 10 to 25 ms (`gc-slow-shadow-refract.md`, the GPU table), serial
with the CPU because the resolve cannot start before it. The remaining
terms are the present (13 to 17 ms) and under 1 ms of GL calls. Shadow is
**CPU-resolve bound**: of a frame of about 120 to 250 ms, 75 to 80 % is the
resolve, 10 to 15 % the wait and present.

**refract.** The same resolve (same size, same code, same bytes). Its extra
is the 11-level GPU chain in its own kick and the wait that goes with it
(the bunny's draw waits for K2, the colour+depth pass, whose stream is
behind the chain's), 15 to 40 ms of GPU. Refract is **CPU-resolve bound** as
well, with a larger GPU and wait share: about 130 to 270 ms a frame.

Neither scene is bound by kicks (2 and 3), cache maintenance (gone), the
fence poll (IRQ since fence-irq) or GL-core CPU (0.3 ms).

## Why W1 to W5 leave them as they are

`gc-tiled-wire.md` W4 qualifies a draw for a tiled shadow only when it has
one 4-byte colour target **and** a depth surface that is `pe_private` (W3: no
plane comes from a texture, no window id, scale 1). None of the two scenes'
draws does:

| draw | colour target | depth surface | W4 |
|---|---|---|---|
| shadow K2, the horse into the 1600x1200 map | none (depth only) | the depth *texture* (bit 30 id), sampled later | no: no colour target, depth not private |
| shadow ground and lit horse | the window | the window's | no: the window (bit 31) is not a shadow |
| refract K2, the bunny into the 1600x1200 target | RGBA8 texture, 4 bytes | the depth *texture*, sampled by the bunny | no: depth not private |
| refract window pass | the window | the window's | no |

W3 also fixes the depth *texture* of both scenes at `Linear` ("A non-private
surface keeps `Linear`, so the depth textures of shadow and refract are not
touched"), and W5's early depth is decided only for draws into a tiled shadow.
The resolve of W4 is the RS copy of a *shadow* of a colour target; the
depth resolve is the CPU's `resolve_depth` and W3 only adds an early return for
a non-linear surface, which these surfaces are not. Result: with
`gc_tiled_rt` on, the host harness counts of shadow and refract are the same
as the table above, and session A should show **no change beyond noise**
(a frame to a frame spread of a few percent) for both. Terrain is the scene
W4 and W5 can move (its 800x600 pass has a depth16 *renderbuffer*, which is
private). A change in shadow or refract in session A would therefore be a
defect to look for (a draw that took a shadow it should not), not a gain.

(W10, the direct sampling of single-level shadow targets, and W11, the tiled
mipmap chain, are also colour-only and do not touch the depth texture; W11
changes refract's GPU chain, not its resolve.)

## What removes the cost, and the expected gain

The numbers use the estimate above (resolve 90 to 200 ms, bounds 50 to 350) and
`gc-slow-shadow-refract.md`'s other terms. They are model figures, not
measurements.

| step | removes | adds | expected frame, shadow | refract | status |
|---|---|---|---|---|---|
| **depth sample, (a): the TPU samples the D24S8 surface** (`gc-depth-sample-wire.md` (a)) | the CPU resolve (1 a frame), its `wait_kick`; one kick fewer with `gc-depth-sample-wire.md` C9 | none | **about 25 to 45 ms** (GPU 10 to 25, present 13 to 17); 4 to 8 times | **about 40 to 70 ms** | blocked on the `dtpu` board record (checks 4, 5, 7); no record yet |
| **depth sample, (b).1: an RS copy into a supertiled depth view, then D24X8 SUPER_TILED** (`gc-depth-sample-wire.md` (b).1) | the same | one RS pass, 7.68 MB read and 7.68 MB write: about 7.5 ms at the GPU's 3.9 ns a pixel (`imx8mp-frame-floor.md` candidate 2), unmeasured | about 32 to 52 ms | about 48 to 78 ms | needs the extra `dtpu` arm (knob 3); the RS rate is unmeasured |
| **depth sample, (b).2: a GPU quad pass into the R32F texels** | the same | one full-size pass, about 7.5 ms plus the decode's ALU | about 33 to 55 ms | about 50 to 80 ms | needs `dtpu` check 6 |
| **W12a, layout only: depth textures super-tiled, the CPU resolve through the map** | nothing | the CPU reads the same bytes through a gather map; no fewer uncached loads | **0 or worse** (same uncached loads, read in supertile order; an estimate) | the same | not a step by itself; needs W12b |
| **W12b, the TS-aware resolve** (W9's depth TS, fast depth clear) | the reads of the tiles still at the clear value (gc-tiled-wire's own figure: shadow and refract 30 to 150 ms) | the TS bits' read | the saving is the fraction of tiles the horse never touches: the horse is a small part of 1600x1200, so most of the map is clear, and a big fraction goes (50 to 90 % of the 90 to 200 ms: **45 to 180 ms**) | the bunny is larger on the 1600x1200 colour pass: **30 to 130 ms** | needs the probe (TS with the MMU off) and W7 to W9; the resolve stays |
| **W10, W11** | none of the depth resolve | | 0 | 0 (W11: the GPU chain only) | |

How the expected frames were counted: after the CPU resolve is gone, a frame
is GPU work overlapped with the present (`imx8mp-frame-floor.md`): shadow's
GPU 10 to 25 ms and the present 13 to 17 ms in series per the sum in
`gc-slow-shadow-refract.md`, refract's 15 to 40 ms. The AM62P's measured
results are the check: shadow 353 to 10.6 ms and refract 375 to 37 ms there
(`pvr-depth-sample.md`), and the GC's GPU is slower than the PowerVR's 6.4 ms
for the same pair, hence the wider range here.

Order, by gain per risk:

1. **The depth sample, whichever of (a), (b).1, (b).2 the `dtpu` record
   allows**: the only step that removes the cost; 4 to 8 times on both scenes.
   Nothing in W1 to W12 replaces it. It needs the `dtpu` board record first.
2. **W12b** only if the board will not sample depth (check 6 fails, outcome
   (c) of `gc-depth-sample-wire.md`): it keeps the CPU resolve and cuts the
   tiles at the clear. Gain 30 to 180 ms, so the frame stays 60 to 120 ms. It
   costs the whole W7 to W9 chain, which needs the TS probe.
3. **Not worth a step**: W10, W11, W12a alone.

## Caveats

* The host timings do not reproduce the board's problem. The host runs
  the byte-wise loop and the wide loop at the same speed, and `memcpy` is not
  far under either, so a host test can only prove that the answer is
  bit-exact and that no extra CPU pass was added; the load count is what the
  board pays, and `src/vivante/ds_cache.rs`'s comment records it
  (`resolve_row`).
* The A53 figures are one calibration from another SoC. The X2's page 0 v0
  rate and PERF page 4 v3 (about 15,000 KiB a frame before the wide
  loop, `gc-slow-shadow-refract.md` cause 1; the value after resolve-wide is
  not recorded) are the confirmation to look for.
* The per-load cost may not be a constant. The 87 ms figure assumes a
  16-byte load costs what the AM62P's 4-byte load did; if the cost grows with
  the bytes, the resolve is above the range given, up to the 175 ms of two
  transactions and beyond.
