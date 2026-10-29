<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: fewer kicks for the PowerVR `glGenerateMipmap` (design)

Written 2026-10-25 as a design; path B is built since 2026-10-28 (see
"Built") and was measured on the board on 2026-10-29 ("Measured"): it cuts
the kicks as predicted but makes the frame slower, so it is off by default
(`Backend::mip_cpu_tail`). Every number below is from the records the note
cites, and the predictions are marked as such.

## The cost

glmark2 terrain takes 93.9 to 94.0 ms a frame with the GPU mipmap
(`d3b8d4b2`), 53 kicks a frame: 5 chains, each 9 kicks (`mip_chain_gpu`,
`src/pvr/backend.rs`, one `record_mip_level` per level above 0) plus the
scene's own. The GPU keeps up (the last kick completes 1 ms after submit), so
the frame is the per-kick chain. 94 ms / 53 is 1.77 ms a kick, the same
period as one kick retired per ~1.95 ms with fragment phases back to back in
`pvr-fence-wait.md` ("One kick in flight"). A kick costs about 1.8 ms no
matter how little it draws: a level of 3x2 texels costs what a level of
400x300 does. So the lever is the **number** of kicks, not their size.

## Why not several levels in one kick

- Level k needs level k-1 complete. Inside one render the PowerVR has no
  such barrier: a pixel reads what the sampler fetched from memory, and the
  frame's own stores land at the end of the tile. So levels cannot chain in
  one pass.
- Multiple render targets (PBE outputs) would let one pass write several
  levels if each output computed its texel straight from level 0 (a
  2^k x 2^k box, no dependence on k-1). Two problems: the 8-bit result is
  not the iterated box filter's (it rounds once, not k times), so the
  picture moves against `tools/cts/glmark2-pictures.txt`, column `am62p`;
  and the backend has one target per frame (`GlesDraw.target_*`), so it is a
  new draw ABI across `src/gl`, every backend table and the generated
  headers. Rejected.

## The paths

| Path | Kicks per 10-level chain | Cost in the render service | Risk |
|---|---|---|---|
| A. Transfer queue (TQ3D blit per level) | 9 jobs, unless one submit carries all the blits | A transfer context beside the render context (create, destroy, CCB, its own USC EOT and PDS code: `driver/src/pvr_usc_eot_blob.h` already names the transfer props), a fence from render to transfer and back, a new job type in `render.rs` `submit` | High. Not tried. Unknown whether a transfer job costs less than ~1.8 ms or runs in the fragment data master's queue like a kick (the TQ3D path does). 9 jobs would gain nothing if it does. |
| B. GPU for the big levels, CPU for the tail | 3 or 4 | None in the render service. The backend waits for the last GPU level (`wait_fence`), then the GL core filters the tail on the CPU | Low. The CPU tail reads GPU-written memory, but levels 4..9 of terrain are under 2,500 texels (about 10 KB). |
| C. MRT from level 0 | 4 or fewer | New draw ABI | Rejected above. |

## Recommendation: B first, A only if B falls short

B first: it costs no new render-service feature, and its worst case is a
wait. It applies to terrain (800x600, NPOT) and refract (1600x1200, NPOT),
not only to POT chains, because the CPU tail uses the GPU's own filter (design
point 3).

### Which steps are exact

The GPU level is one bilinear sample at each texel centre of the level above
(`mipgen.frag`, `src/pvr/backend.rs` "glGenerateMipmap on the GPU"). Where a
level is half the one above, that sample is the 2x2 box. Where the step is
odd (75 -> 37) it is the weighted mean of the nearest four. The CPU filter
(`src/gl/texture.rs` `glGenerateMipmap`, the loop over `chain_levels`) is
always a truncating 2x2 box that rounds half up in integers, so it matches
the GPU on even steps only, and then only to the TPU's rounding.

| Chain | Steps | First odd step |
|---|---|---|
| terrain 800x600 | 400x300, 200x150, 100x75 even; 50x37, 25x18, 12x9, 6x4, 3x2, 1x1 | level 4 (75 -> 37) |
| refract 1600x1200 | 800x600 .. 100x75 even (4 levels) | level 5 (75 -> 37) |

So the cut that keeps the large levels on the GPU (levels above 64x64
texels: terrain 1..3, refract 1..4) also keeps every GPU level on an even
step. The CPU tail starts with the first odd step. A plain CPU box there
would not be the all-GPU picture, so the tail must reproduce the GPU sample.

### Design

1. `mip_chain_gpu` submits levels 1..=n, where n is the last level with more
   than `MIP_CPU_TAIL_TEXELS` texels (start: 64x64, so terrain n = 3 and
   refract n = 4), skipping the rest. It then waits for the last kick's fence
   (`wait_fence`) before it returns. A chain with n = chain_levels - 1 (small
   chains) is submitted whole as today and not waited for.
2. ABI, one choice. `pvr_generate_mipmap` keeps 0 = "the GL core filters
   all" and 1 = "the whole chain is the backend's, `return_mipmap_chain`
   applies", and adds `2 + n` for n >= 1 = "levels 1..=n are in the storage,
   complete, no kick of the chain is in flight, and `return_mipmap_chain` has
   nothing to do; the GL core filters from level n + 1". The call returns
   `2 + n` only after the wait. `YakoglBackend` gains no field, so no struct
   literal in `tests/` changes (`grep -rl 'YakoglBackend {' tests src`).
3. GL core, `src/gl/texture.rs`:
   - `backend::generate_mipmap` (`src/backend.rs:2041`) returns an enum
     (`None`, `Whole`, `Prefix(n)`) instead of a bool: the `== 1` becomes a
     match on 0, 1 and `2 + n`.
   - `Whole` is the branch at texture.rs:4019-4028 as today, including
     `chain_in_backend = true`. `Prefix(n)` must **not** set
     `chain_in_backend` (nothing is in the backend any more) and falls
     through to the CPU path. `settle_texture` finds no kick to wait for.
   - The CPU loop (texture.rs:4068, `for l in 1..`) starts at `n + 1`, with
     `(w, fh)` set to level n's size (halve the base size n times, each
     clamped at 1, as the loop's own update does) and the source offset of
     level n from `level_off`. The `cache_invd` before the loop already
     covers the GPU-written levels.
   - The CPU step for a `Prefix(n)` chain is the **GPU sample**: for each
     destination texel x of dw, the source coordinate is
     sx = (x + 0.5) * w / dw - 0.5, clamped to the edge, and the texel is the
     bilinear mean of floor(sx) and floor(sx) + 1 with the fractional
     weights, rows the same way, then rounded to nearest. On an even step
     this is the 2x2 box exactly. It is selected by a flag set only when the
     backend returned `Prefix(n)`, so a chain that no backend started keeps
     today's box, and the pictures of the other boards do not move. The
     flag is needed on RGBA8 only: the backend refuses every other format
     (`pvr_generate_mipmap` conditions at texture.rs:4005-4015).
4. Exactness, stated honestly. The TPU's weights are fixed point, so the CPU
   float step is expected within 1 LSB of the GPU level per channel, not
   bit-identical; the same holds for the GPU's even steps against the old
   all-CPU box. The picture check is the arbiter: each probe and the mean
   within `tolerance 8` levels per channel of column `am62p`
   (`tools/cts/glmark2-pictures.txt`). Nothing here is "unchanged by
   construction".
5. Tests. Host (`src/gl`): the bilinear CPU step equals the 2x2 box
   byte-for-byte on every even step of a 64x48 chain, and on 75 -> 37,
   37 -> 18 and 9 -> 4 equals an independent f64 reference within 1 LSB, with
   the clamped edge texels pinned. `src/pvr/backend_tests.rs`: the rig
   records n kicks (not chain_levels - 1) for 800x600, and the call returns
   `2 + 3`. Board: one run of the all-GPU chain (threshold 0) and one hybrid
   chain of the same texture, each read back, with every texel of levels
   4.. within 1 LSB.
6. The wait is a real cost: the CPU idles until the chain's n GPU kicks
   complete (about n x 1.8 ms if the GPU is otherwise idle), where today it
   does not wait. **Prediction, not measured:** 5 chains x 6 fewer kicks =
   30 fewer of 53 (23 left), and the frame falls from 94 ms toward
   94 x 23/53 + the waits, about 40 to 55 ms. If the waits eat it, the
   threshold moves (more GPU levels) and the same code applies.

If the board shows B short on time, A is the next design, starting with the
experiment in "What decides it".

## What decides it

One glmark2 terrain run on the AM62P from main with the kick counter
(`pvr_backend_mip_chains`, plus the per-frame kick count the record prints),
before and after:

- kicks a frame: 53 now; B predicts 23;
- ms a frame: 94 now;
- the picture against column `am62p` of `tools/cts/glmark2-pictures.txt`
  unchanged.

If B's picture or time falls short, A is the next design, starting with a
board experiment that does not need the whole transfer context: one TQ blit
job's cost alone, against the 1.8 ms of a kick.

## Built (2026-10-28)

Path B as designed above, points 1 to 3 and 5:

- `src/pvr/backend.rs`: `MIP_CPU_TAIL_TEXELS` (64 x 64) and `mip_gpu_last`
  (level 1 always, then each level over the threshold); `mip_chain_gpu`
  draws levels 1 ..= n and waits for the last kick when a tail is left, and
  `pvr_generate_mipmap` answers `2 + n`. A chain whose every level is large
  is drawn whole and not waited for, as before. `Backend::mip_gpu_all`
  keeps the all-GPU arm for the host tests of the per-level kicks.
- `src/backend.rs`: `generate_mipmap` returns `MipChain` (`None`, `Whole`,
  `Prefix(n)`); an answer past the chain is `None`.
- `src/gl/texture.rs`: `Prefix(n)` leaves `chain_in_backend` false and the
  CPU loop starts at level n + 1 with `mip_step_gpu_sample`, the GPU's
  bilinear sample at each texel centre (exact positions over 2 * dw, f32
  weights, rounded to nearest): the 2x2 box byte for byte on even steps.
- The kicks this predicts: terrain's 800x600 chains 3 each (were 9), its
  256x256 chain 1 (was 8); gl_test 216's four chains 3 + 1 + 3 + 3 = 10
  of 35 levels.

## Measured (2026-10-29)

gl_test case 216 `mipchains` (terrain's four chains a frame, 20 frames,
glFinish each; the case now prints its kicks a frame), one boot each on the
AM62P from the U-Boot one-shot, standard images:

| Image | ms a frame | Mpx/s | kicks a frame | GPU busy (TA\|3D) a frame |
|---|---:|---:|---:|---:|
| main `316b4ad7` plus the kick count | 46.15 | 43.5 | 40 | 7.75 ms |
| path B (`f006dc90`) | 83.10 | 24.2 | 15 | 6.99 ms |
| path B, cache maintenance of the tail alone | 81.00 | 24.8 | 15 | 7.00 ms |
| path B, the tail filtered in a heap copy | 105.60 | 19.0 | 15 | 7.00 ms |

The four quadrant probes are the same in every run (PASS). The kicks fell
as predicted (35 levels to 10, plus the 5 passes), but the frame rose. A
measurement build timed each prefix `glGenerateMipmap` (the mean of 80
calls): 15.39 ms in the backend (the chain's kicks and the wait for the
last), 7.87 ms in the CPU tail, a few microseconds in the settle and the
cache maintenance (90,112 bytes). So:

- **The wait is the cost.** On main a kick queued costs about 1.15 ms (40
  kicks in 46 ms); a prefix call waits for its pass and its three levels,
  four kicks, and that takes 15 ms. This is the fence
  wait of ce3354ac (a fence that does not move until the render service
  re-posts the kick), now met four times a frame instead of once.
- **The CPU tail reads uncached memory.** The carve-out is mapped Normal
  non-cacheable (`driver/src/pvr_fw_boot.c`), so each byte the filter reads
  is a DRAM access: 7.9 ms for a tail of a few thousand texels. Copying the
  bytes out first is slower still (the copy reads them uncached too).

Path B is therefore off by default: every level on the GPU, nothing waited
for, as on main. The code stays (the GL core's `MipChain::Prefix`, the GPU
sample on the CPU, the backend's prefix behind `mip_cpu_tail`). It can come
back only when a wait for a kick costs what a queued kick does, and with a
tail that does not read the carve-out byte by byte. Until then the lever
for the kicks is A (one transfer submit for the chain) or the cost of a
kick itself.

## Open

- Why four kicks waited for take about 15 ms on the AM62P when a queued
  kick costs about 1.15 ms (the render service's wait and re-post,
  ce3354ac).
- The read-back of the all-GPU and the hybrid chain (point 5, board) was
  not run: the prefix is off.
- The "Done when" of the issue (kicks and ms a frame down on the AM62P, one
  glmark2 run from main, picture within tolerance) is not met; the issue
  stays open.
