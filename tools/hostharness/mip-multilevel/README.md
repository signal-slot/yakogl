<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Three mip levels a render: the picture error against the iterated box chain

Plan #329, item `mip-multilevel-error`; candidate 2 of
`docs/notes/am62p-slow-terrain.md` (levels l+1..l+3 from level l in one render with three
colour targets, 4 renders an 800x600 chain instead of 9). Host model only: no
board, no `src/` change.

```sh
python3 -I tools/hostharness/mip-multilevel/compare.py [--glmark2-src DIR]   # or GLMARK2_SRC
```

Pure python (no numpy, no network; about a minute). It prints, per size and image,
a level table (max abs error / mean abs error / share of bytes that differ) and then
the worst max over the images per level with the verdict. Images: `random`,
`gradient`, `edges` (hard edges at odd places), `noise` (smooth value noise), and
the 12 PNGs of glmark2's `data/textures` (tiled to the size; `jellyfish-*` skipped).
The three `terrain-*.jpg` textures are not read (no JPEG decoder; they are 512x512,
POT, uploaded once and not part of terrain's per-frame chains,
`docs/notes/terrain-mip-usage.md`). Sizes: 800x600 and 256x256 RGBA8.
Seed 1; `--seed`, `--sizes`, `--images`, `--quick` (no PNGs).

## What is modelled

- **ref**: the GL core's chain (`glGenerateMipmap`, `src/gl/texture.rs`): each level the
  2x2 box of the one above, `(sum + 2) / 4`; sides halve by floor (min 1), a trailing odd
  row/column is dropped.
- **iter**: what the PowerVR makes today, one render a level
  (`mip_step_gpu_sample`): one bilinear tap at the scaled centre of each destination
  texel, 8-bit fraction, round to nearest.
- **one**: three levels a render with one such tap a target (same program, bigger
  ratio). Included to show why more taps are needed.
- **box**: three levels a render, target 2^j = the aligned box of level l as (2^j/2)^2
  bilinear taps, each at a corner shared by four texels (weight exactly 1/2), summed and
  rounded once. The box of an axis starts at `x * 2^j`; an axis already at side 1
  contributes extent 1. This is the tap variant that keeps NPOT levels exact: floor
  halving twice is `floor(n / 4)` and drops the same trailing texels, so the footprints
  are the iterated chain's, and only the rounding differs. Groups: 1; 2-4; 5-7; 8-10
  (800x600 gets 4 renders: 1, 2-4, 5-7, 8-9).

Model assumptions: round half up (`+0.5`, as the existing GPU path); exact corner
weights (a bilinear at a texel corner is 0.5/0.5 whatever the fraction bits); the shader
sums the taps at full precision before the one rounding (a fp32 sum of at most 16
values of 0..255 is exact, so only a half-even tie rule would differ).
The board's real rounding of ties is not measured.

## Tolerance

`tolerance 8` of `tools/cts/glmark2-pictures.txt` (read by the script), levels per
channel: `judge_picture` in `tools/cts/common-tally.py` compares five probes and the
mean of the 40x30 grid of the **final picture** against the am62p column, each within 8.
A mip level is only sampled with weight <= 1, so a level whose max error is <= 8 cannot
move a probe or the mean by more than 8. The verdict below applies that bound per level
(conservative: a level's error is far from every probe). Terrain also samples only
levels 1 and 2 of its first chain and level 0..1 of the rest.

## Result (worst max abs error over the 16 images a size, levels per channel)

| size | level | level size | iter (today) | one tap | **box taps** |
|---|---|---|---|---|---|
| 800x600 | 1 | 400x300 | 0 within | 0 within | **0 within** |
| 800x600 | 2 | 200x150 | 0 within | 0 within | **0 within** |
| 800x600 | 3 | 100x75 | 0 within | 149 OUT | **1 within** |
| 800x600 | 4 | 50x37 | 121 OUT | 218 OUT | **1 within** |
| 800x600 | 5 | 25x18 | 175 OUT | 196 OUT | **1 within** |
| 800x600 | 6 | 12x9 | 113 OUT | 178 OUT | **2 within** |
| 800x600 | 7 | 6x4 | 111 OUT | 223 OUT | **2 within** |
| 800x600 | 8 | 3x2 | 48 OUT | 93 OUT | **2 within** |
| 800x600 | 9 | 1x1 | 46 OUT | 72 OUT | **2 within** |
| 256x256 | 1 | 128x128 | 0 within | 0 within | **0 within** |
| 256x256 | 2 | 64x64 | 0 within | 0 within | **0 within** |
| 256x256 | 3 | 32x32 | 0 within | 137 OUT | **1 within** |
| 256x256 | 4 | 16x16 | 0 within | 181 OUT | **1 within** |
| 256x256 | 5 | 8x8 | 0 within | 128 OUT | **1 within** |
| 256x256 | 6 | 4x4 | 0 within | 85 OUT | **1 within** |
| 256x256 | 7 | 2x2 | 0 within | 104 OUT | **1 within** |
| 256x256 | 8 | 1x1 | 0 within | 49 OUT | **1 within** |

Mean error of the box variant: 0 at levels 1-2, 0.16-0.3 at level 3-5 (random: 0.37),
up to 0.75 at the smallest levels, with 17-75 % of bytes off by one (the single
rounding against the iterated chain's rounding at each step; no byte is off by more
than 2 over 16 images). The full per-image tables come from the script.

## Verdict

- **Box taps (3 levels a render): within the tolerance at every level**, both sizes:
  max error 0 at levels 1-2, 1 at levels 3-5 and 256x256's 3-8, 2 at 800x600's levels
  6-9, against a tolerance of 8. NPOT stays exact in footprint (75 -> 37 and after):
  the difference is rounding only, on random data too. Candidate 2 is not blocked by
  pictures if it uses box taps. Cost: 4 taps for j=2 and 16 for j=3 a texel of the
  top targets, at the sizes of levels 4 and up (50x37 and below for 800x600): small.
- **One tap a target: out of tolerance** from the first level made 2 or 3 steps up
  (800x600 level 3 on, 256x256 level 3 on); a single bilinear tap is a 2x2 box and sees
  1/4 or 1/16 of the footprint. Do not build the candidate that way.
- **Finding, outside the candidate: today's chain (`iter`) is already not the CPU box
  on NPOT levels 4 and up of 800x600** (50x37 and below: 121-175 on random, 23-42 on
  noise, because `mip_step_gpu_sample` samples at the scaled centre 75/37, which drifts
  by a texel across the image). POT chains (256x256, and 800x600 down to 100x75) are
  byte-identical to the CPU. The box taps would fix that too (their ref error is
  <= 2). Terrain reads only levels <= 2 so it does not show in its pictures, but
  dEQP-GLES2 `texture.mipmap.2d.generate.*` NPOT cases would be affected if they read
  those levels on the GPU path; a question for the board and #325, not measured here.

Not modelled: the sampler's exact tie rounding, the PowerVR's texture-fraction precision
for non-corner taps (only `iter` and `one` use them: 8 bits assumed), and the
program's float accumulation order. Only the board can confirm the bytes; the host says
the footprint and rounding bound.
