<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D tiled copies: which path of Mesa's tiling they take, and what the utile fast path would save

Read at main `a0f75ca3`. A study: no driver change. The host profile in
`docs/notes/pi4-glmark2-cpu.md` puts about 13 ms a frame (refract), 5 ms
(shadow) and 5 ms (terrain) in `v3d_move_pixels_general_percpp`, Mesa's
tiling loop, and the question was why the "generic" path is taken instead of
the utile fast path (`v3d_load_utile`/`v3d_store_utile`, NEON on aarch64).

**Measurements.** `tools/v3d/tiling_bench.sh` ran on the host at
`a0f75ca3`; the timings are in "Host timings", the bytes of both walks are
identical in every case. Nothing was run on a board.

## The finding: there is no separate generic path

`v3d_move_pixels_general_percpp` (`third_party/mesa-26.2.1/src/broadcom/common/v3d_tiling.c`)
*is* the utile path. For a box it

1. calls `v3d_store_utile` (or `v3d_load_utile`) for every whole utile
   inside the box, addressed by `get_pixel_offset(cpp, image_h, x, y)`;
2. walks the partial edges with `v3d_move_pixels_unaligned`, one
   `get_pixel_offset` and one `cpp`-byte `memcpy` per texel;
3. does the whole box that way only when it holds no whole utile.

`v3d_store_utile` is `static inline` in `v3d_cpu_tiling.h`, so a profiler
attributes its time to its caller, `v3d_move_pixels_general_percpp`. The
profile line is therefore the utile fast path plus its address arithmetic,
not evidence that a slower per-pixel path was taken. Nothing in the copies
below is misaligned enough to reach step 3 except the last two levels of a
chain.

## The path each copy takes

The chain of calls: `copy_image_for` (`src/v3d/backend.rs`) →
`tiled_copy` fill → (`tfu_tile`, else) `texture::tile` (`src/v3d/texture.rs`)
→ `v3d_tex_store_level` per level (`src/v3d/ffi.rs`) → `v3d_tex_store_box`
(`driver/src/v3d/v3d_tex.c`) with the box `0,0,level width,level height` →
`v3d_store_tiled_image` (`driver/src/v3d/v3d_tiling_mesa.c` includes Mesa's
file unmodified).

cpp is 4 for RGBA8 and for a 32-bit depth texture, so a utile is 4x4 texels,
64 B in rows of 16 B (`gpu_stride` 16), a UIF block 8x8. `v3d_tex_layout`
(`driver/src/v3d/v3d_tex.c:151`) picks the layout by level size:

| level width or height | layout |
|---|---|
| width <= 4 or height <= 4 | LT (LINEARTILE) |
| width <= 8 | UBLINEAR 1 column |
| width <= 16 | UBLINEAR 2 column |
| larger | UIF, XOR when the padded height is a page-cache multiple, else NO_XOR |

(level 0 and 1 use the real size, levels 2 and up the padded power of two
size for the layout; the box is always the real level size.) RASTER is not a
layout of these copies: the GL core's linear storage is the *source*.

Every level whose width and height are multiples of 4 takes only step 1:
`v3d_store_utile` per utile, all whole. For the glmark2 shapes:

| copy | levels | box alignment | path |
|---|---|---|---|
| refract colour chain, 1600x1200 RGBA8, 11 levels | 1600x1200, 800x600, 400x300, 200x150, 100x75, 50x37, 25x18, 12x9, 6x4, 3x2, 1x1 | levels 0-2 are multiples of 4 | levels 0-2 whole utiles, step 1 only; levels 3-8 add edges (step 2: a 2-row or 3-row bottom edge and, from level 5, a right edge); levels 9-10 (3x2, 1x1) hold no whole utile: step 3, per texel |
| refract / shadow depth texture, 32-bit, single level | 1600x1200 or the shadow map's size | multiple of 4 | step 1 only, UIF |
| terrain 800x600 RGBA8 chains | 800x600 down | NPOT chains (see `CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES` in `rpi_4b.conf`) | step 1 for the big levels, edges on the odd ones |

The edge texels are under 1 % of a chain's bytes; level 0 is about three quarters of a
chain's tiled bytes and every level of it is whole utiles. So for the
time that matters the answer to "why is the generic path taken" is: it is
not; the utile path is, and it is address-computation and store bound.

**Which copies the CPU does on the board.** `tfu_tile`
(`src/v3d/backend.rs:3373`) hands a *single-level* image of
`tfu_min_bytes` (64 KiB) or more to the TFU when `CONFIG_YAKOGL_V3D_TFU` is
set, as `rpi_4b.conf` does; it refuses a multi-level layout
(`layout.levels != 1`). So on the Pi 4:

- the depth texture copies (shadow, refract) are TFU jobs, not CPU tiling;
- only the multi-level colour chains (refract's, terrain's) and copies below
  64 KiB are tiled by the CPU.

The host profile runs without `YAKOGL_V3D_TFU=1` by default, so its 13 / 5 /
5 ms include the depth copies that the board does not tile on the CPU. A
before/after must set `YAKOGL_V3D_TFU=1` to match the board.

## The build: is the NEON code compiled in for aarch64

`v3d_store_utile` has three bodies (`v3d_cpu_tiling.h`):

| build | selected by | body |
|---|---|---|
| 32-bit ARM | `V3D_BUILD_NEON && DETECT_ARCH_ARM` | `vld1/vstm` |
| aarch64 | `DETECT_ARCH_AARCH64` | `ld1 {v.D}[n]` ×8, `st1 {v0.2d-v3.2d}` |
| anything else | | `memcpy` per row |

`DETECT_ARCH_AARCH64` is set by `util/detect_arch.h` from the compiler's
`__aarch64__`, so the aarch64 board build gets the `st1` body without any
flag: `V3D_BUILD_NEON` is only for 32-bit ARM and none of YakoGL's builds
define it (nor need to). Both `CMakeLists.txt:183` and `zephyr/CMakeLists.txt:101`
compile `v3d_tiling_mesa.c` with the target's compiler, which defines
`__aarch64__`. The sample sets no optimisation level (`grep OPTIMIZATIONS
driver/samples/zephyr/glmark2` finds nothing), so the image gets Zephyr's
default for it; the cost of the loop at `-Os` against `-O2` is a bench
output (`tools/v3d/tiling_bench.sh` builds both).

`gpu_stride` is `utile_w * cpp`: 8 for cpp 1, 16 for cpp 2 and 4, 32 for cpp 8
and 16. The asm handles 8 and 16 only, so RGBA8 and 32-bit depth (16) get
it; 8- and 16-byte texels fall back to the `memcpy` loop even on aarch64.

Checked: `tiling_bench.sh` finds the Zephyr SDK's `aarch64-zephyr-elf-gcc`,
compiles the bench for aarch64 (not run) and counts 18 `st1` instructions in
the object, so the NEON utile store is compiled in for the aarch64 build with
no flag. (The object is the bench's, built at `-Os`; the Zephyr image's own
object was not inspected.)

## Host timings

Best of 5 runs, host (cached memory, not the A72), whole chain or level 0.
"utile loop" is `v3d_store_utile` as the host builds it (the `memcpy` body).
The study's run, before the walker:

| texture | opt | per-pixel walk | utile loop | ratio |
|---|---|---|---|---|
| 1600x1200 RGBA8, 11 levels | -O2 | 33.8 ms | 7.6 ms | 4.4x |
| 1600x1200 RGBA8, 11 levels | -Os | 48.8 ms | 14.3 ms | 3.4x |
| 1600x1200 32-bit depth | -O2 | 26.2 ms | 6.9 ms | 3.8x |
| 1600x1200 32-bit depth | -Os | 35.2 ms | 9.7 ms | 3.6x |
| 800x600 RGBA8, 10 levels | -O2 | 7.7 ms | 1.4 ms | 5.5x |
| 800x600 RGBA8, 10 levels | -Os | 11.6 ms | 2.9 ms | 4.0x |

With the walker (`v3d_tiling_uif.c`, commit `eaf94822` "v3d: a UIF store
walker for cpp-4 boxes that steps addresses"), `tools/v3d/tiling_bench.sh`,
another host (GCC 15.2), the lower of two runs of the script. "walker" is
the driver's `v3d_tex_store_level`: the walker on the levels of whole UIF
blocks (the share in the last column), Mesa's function on the others;
"ratio" is utile loop / walker. Every row's bytes are identical to Mesa's
(`tiling_bench: 87 checks, 0 failed` at both levels, both runs).

| texture | opt | per-pixel walk | utile loop | walker | ratio | walker's share of the bytes |
|---|---|---|---|---|---|---|
| 1600x1200 RGBA8, 11 levels | -O2 | 68.0 ms | 23.3 ms | 4.0 ms | 5.9x | 93.8 % |
| 1600x1200 RGBA8, 11 levels | -Os | 84.8 ms | 9.8 ms | 3.4 ms | 2.8x | 93.8 % |
| 1600x1200 32-bit depth | -O2 | 51.9 ms | 16.7 ms | 2.2 ms | 7.4x | 100 % |
| 1600x1200 32-bit depth | -Os | 72.8 ms | 8.1 ms | 2.3 ms | 3.5x | 100 % |
| 800x600 RGBA8, 10 levels | -O2 | 17.1 ms | 5.4 ms | 1.8 ms | 3.0x | 75.0 % |
| 800x600 RGBA8, 10 levels | -Os | 22.6 ms | 2.9 ms | 1.3 ms | 2.3x | 75.0 % |
| 256x256 RGBA8, 9 levels | -O2 | 2.5 ms | 0.76 ms | 0.033 ms | 23x | 99.6 % |
| 256x256 RGBA8, 9 levels | -Os | 2.7 ms | 0.29 ms | 0.050 ms | 5.7x | 99.6 % |

The odd sizes (1000x750 10 levels, 257x255 9 levels, 1366x766 1 level) take
no walker level, so both columns run Mesa's code; their walker / utile loop
ratios spread from 0.76x to 1.83x over the two runs and both levels
(1000x750 at -O2: 0.99x, then 0.82x). That spread is this host's run-to-run
noise, not a cost of the walker. So a whole-chain walker run can come out
slower than Mesa's utile loop (an earlier run of the bench showed 0.91x at
`-O2`); in the two runs above every row below 1x is a fallback row, and on
every size the walker takes it was faster in every run.

On this host the utile loop is 2x to 2.4x *slower* at `-O2` than at `-Os`,
the opposite of the study's table; the walker's times are within the noise
at both levels.

Per frame: refract's 13 ms profile entry matches the host utile loop for a
1600x1200 chain plus a depth texture at `-O2` (7.6 + 6.9 ms), which confirms
the profile runs the utile path. The per-pixel walk is not taken, so it is not
a saving to claim: it would be 60 ms. What is on offer is (a) `-Os` against
`-O2`: 6.7 ms on the refract chain and 2.8 ms on a depth texture on the host,
if the board image builds `v3d_tiling_mesa.c` at `-Os` (the sample sets no
level; not confirmed), and (b) the address-stepping specialisation below,
now built: on the host the chain's store drops to 4.0 ms at `-O2` and 3.4 ms
at `-Os` (the walker table above). With `YAKOGL_V3D_TFU=1` the depth copy is a TFU job, so the
refract saving left for the CPU is the chain's: up to about 6.7 ms (a) on the
host, shadow and terrain (800x600 chains, 1.5 ms) about 1.5 ms.

What the numbers mean for a frame: the per-pixel walk is *not* what the
board runs, so the ratio is the most the utile path could be worth if it
were not already in use. The host's utile body is the `memcpy` loop, the
board's is `st1`; the board's destination is Normal non-cacheable memory,
the host's is cached. The host table bounds the address arithmetic, not the
board's store cost.

## What a fix could be (proposal, for the issue)

The time left is in the utile loop itself, so the lever is not "enable the
fast path" but making it cheaper:

1. `get_pixel_offset` is recomputed per utile with `ffs`, a divide by 4 and
   `align` (`v3d_get_uif_pixel_offset`). A walk over a UIF block (2x2
   utiles, 256 B contiguous) and over the 4-block groups with the
   addresses stepped, not recomputed, removes that and writes 256 B runs.
   Mesa's loop is a generic one for every layout; a UIF-only specialisation
   in the driver (new file next to `v3d_tiling_mesa.c`, not an edit of
   Mesa's) would be the change. Functions: a `v3d_tex_store_box` branch for
   `s->tiling` UIF with cpp 4 and an aligned box, falling back to Mesa's
   function otherwise.
2. Build flags: `-O2` for `v3d_tiling_mesa.c` and `v3d_tex.c` only
   (`set_source_files_properties(... COMPILE_OPTIONS -O2)`), if the bench
   shows `-Os` is slower.
3. Read side: the source is the GL core's linear storage, itself in
   non-cacheable memory (`CONFIG_YAKOGL_V3D_*` carve-out); the source rows
   of a 4-row utile are `pitch` apart, so each utile reads 4 lines. Walking
   utile rows of 4 source rows in the outer loop keeps those lines hot.
   (The same, plus the write-combining of 64 B stores, is what only a board
   run can show.)

Host repro: `tools/v3d/tiling_bench.sh` before and after for the loop;
the frame cost with `tools/hostharness/glmark2` refract, `YAKOGL_V3D_TFU=1`,
ms per frame, as the other V3D items. Risk: the layout code is the one
`tex_check` compares against Mesa, byte for byte, so a new walker is
covered by that check (add a row for it); a wrong UIF XOR step is the
failure to expect; the board win is unknown until measured, because the
destination is non-cacheable and the A72's write buffer may already merge
the 16 B stores (then 1 and 2 save only the arithmetic).

Another route to the same ms is not tiling the colour chain on the CPU:
`v3d-sampled-copies-study` covers it.

## The UIF walker (proposal 1, built)

`driver/src/v3d/v3d_tiling_uif.c`, `v3d_tex_store_uif4`: for a UIF level
(XOR or not) with cpp 4 and a box whose x, y, width and height are multiples
of 8, `v3d_tex_store_box` stores whole UIF blocks with stepped addresses: a
block row's offset once for the even and once for the odd columns (the XOR
flips bit 4 of the block row in odd columns), 256 B to the next block, one
column's bytes (`padded_height / 8 * 1 KiB`) at every fourth. The four utiles
of a block are stored by Mesa's `v3d_store_utile` (the `st1` code on
aarch64). Every other box goes to Mesa's `v3d_store_tiled_image` as before.
`v3d_tex.c` includes the file, so every build of `v3d_tex.c` compiles it.

Which levels take it: a level of whole UIF blocks. In the 1600x1200 chain
levels 0 and 1 (about 94 % of the chain's bytes); in the 800x600 chain level
0 (about 75 %; 400x300 is not a multiple of 8); the 1600x1200 depth texture;
in the 256x256 chain levels 0 to 3. Edge levels and odd sizes (1000x750,
257x255, 1366x766) fall back to Mesa whole.

Proposal 2 (`-O2` for the tiling sources in the Zephyr build) is not built:
the bench run with the walker shows `-Os` no slower than `-O2` (Mesa's utile
loop is 2x to 2.4x faster at `-Os` with GCC 15.2, the walker the same within
the noise), so the condition the proposal set does not hold. Which level the
board's aarch64 compiler should use is a board measurement.

`tools/v3d/tiling_bench.c` now times three walks (per-pixel, Mesa's utile
loop, the driver's `v3d_tex_store_level` with the walker), compares every
level's bytes with Mesa's for 1600x1200 11 levels, the 1600x1200 depth
texture, 800x600 10 levels, 256x256 9 levels (UIF_XOR) and the odd sizes,
checks per level that the walker is taken exactly for UIF, cpp 4, whole
blocks, and checks 18 boxes: sub-boxes of whole blocks inside a column,
across columns, in the odd columns of a UIF_XOR level (rows 15 and 16, the
XOR bit) take the walker with Mesa's bytes; unaligned boxes, UBLINEAR and LT
levels and cpp 1, 2, 8 and 16 fall back. Last line: `tiling_bench: N checks,
0 failed`; at `eaf94822`, 87 checks, 0 failed at `-O2` and at `-Os`. The
timings are the walker table under "Host timings"; the bench also prints a
"level 0 only" line per size (1600x1200 level 0 at `-O2`: utile loop
16.9 ms, walker 2.3 ms).
