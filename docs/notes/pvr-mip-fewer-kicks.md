<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: GPU-render-only ways to make the PowerVR mipmap chain in fewer kicks (a study)

Written 2026-11-07 from the code at `d83e25b5` (line numbers are that
commit's) and the vendored headers, read only. No code changed and no board
was run. Two routes are out of scope and are not repeated here: path A, the
transfer queue (`pvr-tq-mipmap.md`), and path B, the CPU tail
(`am62p-pvr-mipmap-kicks.md`, off by default as `Backend::mip_cpu_tail`).
This note asks whether **render kicks alone** can build a chain in fewer
kicks than one a level.

**Verdict.** One rule forces a kick per level: a level can only be sampled
after the kick that wrote it has finished. No size, PBE, EOT or alignment
rule forces it. So a kick can write several levels if it computes them from
the last level already in memory. The tail route, below, is the best: one
extra kick writes the whole tail of a chain as **one linear surface over the
tail's bytes**. It brings each of terrain's chains (9 and 8 kicks today)
down to 4 kicks, and keeps today's single-target draw ABI. Levels 1 to 4 stay
bit-identical by construction. Levels 5 and below are bit-identical only if
the shader's ALU bilinear matches the TPU's. No vendored header says whether
it does, so the board experiment E1 below decides that before any code is
written.

## The cost being attacked (figures checked at `d83e25b5`)

- glmark2 terrain on the AM62P: **83.06 ms a frame** at `6a7021ec`
  (`tools/cts/RESULTS.md:83`, the AM62P row, and `:249`), with every level
  on the GPU.
- gl_test case 216 `mipchains`, terrain's chains with glFinish each frame:
  **46.15 ms a frame, 40 kicks a frame**, GPU TA|3D busy 7.75 ms
  (`tools/cts/RESULTS.md:300-305`, main `61ad38a1` plus the kick count). So
  a queued kick costs about 1.15 ms, and at most 0.19 ms of that is
  hardware time (`pvr-tq-mipmap.md`, "The cost being attacked"). The
  fixed per-kick cost is what this note tries to avoid paying.
- **How many chains.** `driver/rust/src/gpu/pvr/render.rs:80-82` and case
  216 (`RESULTS.md:300`) both say terrain has **four** chains a frame: three
  800x600 chains, each 9 level kicks, and one 256x256 chain, 8 level kicks.
  That is 35 level kicks. Case 216 adds 5 passes, which makes its 40.
  `am62p-pvr-mipmap-kicks.md:14-16` (at `d3b8d4b2`) and this plan's goal
  say "5 chains of 9 kicks, 53 kicks a frame" instead. The tables below
  give both counts. The open item terrain-mip-usage settles which is right.
- **Where a level is built today.** `mip_chain_gpu` (`src/pvr/backend.rs:
  6241-6295`) makes one `record_mip_level` per level, in a loop at
  `:6268-6269`. Each call is a frame of its own (`:6138-6202`): one
  full-screen quad, drawn by `mipgen.frag`
  (`tools/shaders/internal/es/mipgen.frag:21-23`,
  `texture(u_src, gl_FragCoord.xy * u_scale)`). It reads level k-1 through
  a one-level bilinear CLAMP_TO_EDGE sampler (`backend.rs:6170-6187`). The
  frame's grid is level 1's size for every level (`:6151-6152`, `:6269`).
  The PBE store is clipped to the level (`mip_clip`, `:6148`; slot 0's
  maxclip at `:8899-8913`), at the level's own pitch (`:6153`).

## What the hardware and our code allow

| Rule | What it says | Where | Forces one kick a level? |
|---|---|---|---|
| A sample reads memory | The TPU fetches texels from memory. The PBE writes a tile only at the end of that tile, through the end-of-tile program. Within one render, nothing orders tile A's store before tile B's fetch, and the texture cache is not invalidated in between. So level k+1 cannot be *sampled* in the kick that *writes* it. | `am62p-pvr-mipmap-kicks.md:26-29`; the EOT programs per render, `src/osal/render.rs:35-48`; one PBE state array per 3D command, Mesa `csbgen/rogue/kmd_stream.xml:156,165` (`CMD_FRAG.pbe_word`, 1536 bits) | **Yes, and it is the only rule that does.** Every route below works around it by computing the deeper levels from a level already in memory. |
| Surfaces per kick | The hardware has 8 PBE emits (`PVR_NUM_PBE_EMIT_REGS 8U`), which is also Mesa's colour-attachment limit. Our captured EOT programs cover 3. | `third_party/mesa-26.2.1/src/imagination/include/hwdef/rogue_hw_defs.h:154`; `common/pvr_limits.h:35`; `src/osal/render.rs:17-20`; `src/backend.rs:741-743`; backend `num_targets`, `src/pvr/backend.rs:5234` | No. Up to 3 surfaces today, 8 in hardware. |
| Output registers | Four 32-bit output registers per pixel. Attachments past them spill to tile buffers (2 of them, and only the last attachment may be reloaded from one). An RGBA8 output is one dword, so 4 RGBA8 levels fit without a spill. | `src/osal/render.rs:22-23, 35-38`; Mesa `common/device_info/bxs-4-64.h:104` (`usc_min_output_registers_per_pix = 2`) | No. It limits a kick to 4 levels with registers alone. |
| PBE state per surface | Each emit has its own address (`STATE_WORD0.address_low`, 4-byte units), its own `mrt_index`, its own linestride, and its own min and max clip in x and y. | Mesa `csbgen/rogue/pbestate.xml:226-227, 236-237, 258-260, 266, 277-281`; ours `pbe_reg_words` `src/pvr/backend.rs:896-930` and `pbe_emit_words` `:1014-1050` | No. Each surface of a kick can have its own pitch, its own clip and its own base. **But** our MRT path takes a secondary target's pitch from its width (`(tw + 1) & !1`, `backend.rs:8916`), and `GlesDrawTarget` has no pitch field (`src/backend.rs:798-810`). Using a level as the second or third target needs that field, or a backend-private override like `mip_clip`. |
| Render size per kick | Each HWRT dataset has one size, and a size has `HWRT_MEMBERS` = 4 members. Every level is drawn on level 1's grid with the store clipped, so a chain never changes size. | `driver/rust/src/gpu/pvr/render.rs:76-85, 749-772`; `backend.rs:6124-6131` | No. A route must stay inside level 1's grid, or it pays for a new HWRT set. |
| Alignment | The PBE linestride counts pixel pairs (`UNIT_SIZE 2`), so a target's pitch is even. A 1-wide level is pitch 1 and is allowed only as a single row. The store is clipped per pixel, not per pair. Today's chain already stores pitches 2 to 512 at offsets in 4-byte units, so nothing coarser applies. | `pbestate.xml:258-260`; `backend.rs:6262-6266`; `src/gl/context.rs:239-245` | No. A surface over several levels needs an even pitch. |
| Level addressing | A texture state word holds one `texaddr` (level 0) and a stride. The TPU finds level l by its own arithmetic: each level rounded up to a power of two, and the levels one after another. | Mesa `csbgen/rogue/texstate.xml:307, 312` (`IMAGE_WORD1`), `:329` (`STRIDE_IMAGE_WORD1`); `src/gl/texture.rs:1752-1784`; `pvr-npot-levels.md`, rules 1-3 | No, but a level cannot live anywhere else. A route that writes levels into an atlas needs a copy afterwards. |
| MSAA resolve | The PBE `downscale` averages a pixel's 4 samples when it stores them. | `pbestate.xml:255`; `backend.rs:885-888`; `bxs-4-64.h:90` | Not a route (see "Rejected"). |

The DDK 25.2 kernel headers vendored under `third_party/ti-img-rogue-ddk-25.2`
do not include the 3D command's register payload (the PBE words, the EOT
state). They name only the PBE's HWPerf blocks
(`include/rogue/rgx_hwperf.h:280-299`). Mesa's `kmd_stream.xml` is the
layout we cite, as `render.rs:19-24` and `pvr-tq-mipmap.md` do.

## Bit-exactness: what "today's level" is

Today, level k+1 texel (x, y) is `Q(T(L_k, c))`:
- `T` is the TPU's bilinear sample of level k, from memory, at
  c = (x + 0.5, y + 0.5) * u_scale, where `u_scale` is 1 / the level's size
  computed in f32 on the CPU (`backend.rs:6143`).
- `Q` is our compiler's pack of the float output to the U8 target
  (`source_format` 8 per channel, `backend.rs:1037`).

A kick that writes several levels can reproduce `Q(T(L_k, c))` for the
**first** level below the one in memory, **bit for bit**. It uses the same
`texture()` call on the same sampler state with the same coordinate bits
(computed as `(float(x) + 0.5) * u_scale[l]` with today's `u_scale` passed
as a uniform), and the same output pack.

Every deeper level needs `Q(T'(...))`, where `T'` is a bilinear over values
that are **not in memory**. `T'` must therefore be ALU arithmetic in the
shader. It is bit-identical to today only if two things both hold:

- The ALU reproduces the TPU's filter exactly. On an even step, ties
  (a + b + c + d = 2 mod 4) make the result hang on the last bit of the
  TPU's output. On terrain's odd steps (75 -> 37 and below, `am62p-pvr-
  mipmap-kicks.md:64-67`), it also hangs on the TPU's sub-texel quantisation
  of the coordinate.
- The intermediate `Q` is reproduced. That is the same pack as the output,
  which is in our hands.

No vendored header documents the TPU's filter precision. `texstate.xml`
gives the state words only. So nothing beyond the first level of a kick is
"unchanged by construction". The experiment E1 below turns that into a
measurement.

**ES 2.0 answers.** `tests/es2_report_guard.rs` pins API answers on a
stand-in back end that registers `generate_mipmap: None`
(`tests/es2_report_guard.rs:132`). It never reaches the PowerVR chain, so
**no route changes it**. The board's texel answers are the ones at stake:
the dEQP-GLES2 generate-mipmap cases and the glmark2 pictures (column
`am62p`, tolerance 8). ES 2.0.25 3.7.11 requires no particular filter.
The board answers stay bit-identical where the table says "identical", and
for the other levels only if E1 reads zero.

## The routes

The kick counts are level kicks per chain, for the three 800x600 chains
(9 levels above 0) and the one 256x256 chain (8 levels above 0).

### Route 1, tail: the tail in one kick, as one linear surface over its bytes (rank 1)

Levels l >= s+1 of a chain are one contiguous byte range of the storage.
Each level follows the one before (`src/gl/texture.rs:1752-1771`;
`mip_chain_layout`, `backend.rs:6095-6111`). So keep levels 1..s as today's
kicks, then draw **one more frame** whose single target is that byte range,
viewed as a P-texel-pitch surface of R rows (P even, P x R <= level 1's
grid). Its fragment program, `mipgen_tail.frag` (new), does this for each
pixel (X, Y):

1. i = Y * P + X. It finds the level l from the tail's level offsets
   (uniforms, at most 6 compares) and then (x, y) inside that level (the
   pitch is a power of two, so this is a shift and a mask).
2. Padding, x >= w_l or y >= h_l, writes 0. The GL core clears new storage
   (`texture.rs:1791-1796`), and no GL path reads padding.
3. Otherwise it computes level l's texel from level s, which is in memory.
   The leaves are level s+1 values, each `Q(T(L_s, c))` exactly as today.
   Each step above them is the ALU bilinear plus `Q`, iterated as a 4-ary
   reduction with a stack of at most 5 partial values (a loop, no
   recursion).

The kick is one target, as `record_mip_level` draws today: `target_pitch` =
P, viewport and `mip_clip` = P x R, on level 1's grid. It needs no MRT, no
new `GlesDraw` or `GlesDrawTarget` field and no new HWRT size.

The two sizes, worked out from the layout rules (`src/gl/context.rs:239-
245, 293-300`; `texture.rs:1773-1789`):

| Chain | s | Tail | Texels in the range | Surface P x R | Fits level 1's grid | Worst pixel |
|---|---|---|---|---|---|---|
| 800x600 (phys 1024, grid 400x300) | 3 | levels 4..9 (50x37 .. 1x1) | 4096+1024+256+64+16+4 = 5460 | 84 x 65 | yes | level 9: 1024 leaf samples |
| 800x600 | 2 | levels 3..9 | 21844 | 172 x 127 | yes | 4096 |
| 800x600 | 1 | levels 2..9 | 87380 | 340 x 257 | yes | 16384 |
| 256x256 (grid 128x128) | 3 | levels 4..8 (16x16 .. 1x1) | 341 | 20 x 17, plus 1 texel | yes | 256 |

The 800x600 range ends at level 9. Level 10 (1 texel) is in the storage but
not in GL's chain, so the range is even.

The 256x256 chain's level 8 is 1x1 at pitch 1 (`context.rs:239-245`). That
leaves a remainder of one texel. It goes out through a second PBE emit (one
row, 1x1 clip at level 8's address; 2 targets exist,
`render.rs`/`src/osal/render.rs:17-20`). The other way, an R x P range that
runs one texel past the storage, relies on allocator slack and is not taken.

The total shading work is small: about 9,500 TPU samples for the 800x600
tail at s = 3 (1850 + 450x4 + 108x16 + 24x64 + 6x256 + 1x1024). The ISP
walks the same 475 tiles of level 1's grid as any level kick. s = 3 keeps
the slowest thread at about 1024 dependent samples. s = 2 saves one more
kick and costs 4x that thread. That trade is measured, not assumed.

- **Kicks per chain:** 800x600: 9 -> **4**; 256x256: 8 -> **4**.
- **A frame:** 35 level kicks -> 16 (case 216: 40 -> 21). With the "5 x 9"
  count: 45 -> 20 (53 -> 28).
- **Bit-identical:** levels 1..3 (unchanged kicks) and level 4 (the first
  level of the tail kick, `Q(T(L_3))`) by construction. Levels 5..9 only if
  E1 reads zero. Those are the smallest levels: 450 texels and fewer.
- **Code size, an estimate:** `tools/shaders/internal/es/mipgen_tail.frag`
  (about 70 lines) and its line in `tools/shaders/precompiled.list`;
  regenerated `src/pvr/internal.rs` and `tests/reference/pvr_internal.rs`;
  `src/pvr/backend.rs` about 120 lines (tail range and P x R choice, one
  `record_mip_tail`, the remainder emit); `src/pvr/backend_tests.rs` about
  120 lines (P x R for both chains, the kick count, the PBE words: pitch,
  clip, the second emit's address). About 300 lines in total. Plus one
  `gl_test` read-back case for the board.
- **Risk:** the shader uses an integer loop with dynamic bounds and a small
  local array. The online compiler builds the internal programs
  (`src/pvr/internal.rs:6-9`) and passes dEQP-GLES2's loop and array
  groups, but an internal program of this shape has never been built.
  Ownership: `src/pvr/backend.rs`, `backend_tests.rs`, `internal.rs` and
  `tests/reference/pvr_internal.rs` are plan #325's (and `src/pvr/**` is
  also #301's) while those plans run.

### Route 2, mrt+tail: the MRT route for the top, the tail route for the rest (rank 2)

One MRT kick writes levels 1..3 from level 0 (3 targets). Then the tail route's
tail kick writes levels 4..9 from level 3.

- **Kicks per chain:** 800x600: 9 -> **2**; 256x256: 8 -> **2**.
- **A frame:** 35 -> 8 (case 216: 40 -> 13); "5 x 9": 45 -> 10 (53 -> 18).
- **Bit-identical:** levels 1 and 4 by construction. Levels 2, 3 and 5..9
  only if E1 reads zero. Levels 2 and 3 (200x150, 100x75) are large and
  sampled by terrain, so a 1-LSB deviation there shows more than one in
  the tail.
- **Code size:** the tail route plus the MRT route, about 550 lines. It also has the MRT
  route's ABI cost (the per-target pitch) inside the MRT path, which ES 3.0 work
  (#325) is changing.

### Route 3, mrt: several levels per kick as MRT outputs from one source (rank 3)

One kick writes levels k+1..k+m (m <= 3 today, 4 with a fourth EOT program,
8 in hardware) as separate PBE emits. Each emit has the level's address,
its own linestride and its own maxclip (`pbestate.xml:226-281`). All emits
cover the same grid, level 1's. Pixel (x, y) computes output j = level
k+j's texel (x, y) from level k. Output 1 is the TPU sample, as today.
Outputs 2..m are ALU cascades of depth up to 2 (4 and 16 leaf samples), with
a branch on gl_FragCoord so a pixel past level k+j's extent skips output j.

- **Kicks per chain with 3 targets:** 800x600: {1-3}, {4-6}, {7-9} = **3**;
  256x256: {1-3}, {4-6}, {7-8} = **3**. With 4 targets: 3 and 2.
- **A frame (3 targets):** 35 -> 12 (case 216: 40 -> 17); "5 x 9": 45 -> 15
  (53 -> 23).
- **Bit-identical:** levels 1, 4 and 7 by construction. The other six only
  if E1 reads zero.
- **Needs:** a pitch per secondary target. Today it is `(tw + 1) & !1`
  (`backend.rs:8916`), and `GlesDrawTarget` has no pitch
  (`src/backend.rs:798-810`). Either a new field, in which case every
  literal in `tests/` and `src/` must be checked (`grep -rl 'GlesDrawTarget
  {' tests src`, CLAUDE.md's first failure), or a backend-private
  `mip_pitch[3]` set by the mip draw only, as `mip_clip` is (`backend.rs:
  5228`). The MRT background clear for 2-3 normalised targets exists
  (`mrt_clear_dwords`, `backend.rs:2036-2046`). A new
  `mipgen3.frag` with 3 outputs (3 dwords, within the 4 output registers).
- **Code size:** about 300 lines (shader, pitch override, the grouping, host
  tests of the three emits' words).

An older note rejected this route as "the backend has one target per frame"
(`am62p-pvr-mipmap-kicks.md:30-37`). That is no longer true: `num_targets`
goes up to 3 (`backend.rs:5234`). The rounding objection in that note stays
true. It is the E1 question here.

### Rejected

- **Separate draws in one kick, one per level surface.** A render's PBE
  emits are fixed for the whole 3D command (`CMD_FRAG.pbe_word`,
  `kmd_stream.xml:165`). A draw cannot choose its surface, so this is route
  B with per-pixel branching, and saves nothing more.
- **Small levels packed as viewports into a scratch atlas, with the level
  descriptors pointed at them.** The TPU has one `texaddr` per texture and
  derives level offsets itself (`texstate.xml:307, 329`), so the descriptors
  cannot point anywhere else. The atlas would need a copy kick, or a CPU
  copy from the non-cacheable carve-out (path B's 7.9 ms lesson). The tail route
  is this idea with the atlas made the storage itself.
- **One emit per small level, packed by viewport and `minclip`/`maxclip`
  with a biased base address.** It is possible
  (`pbestate.xml:266, 277-281`), but it gives at most 3 levels per kick
  (`src/osal/render.rs:17-20`), so it is no better than the MRT route. The tail route
  does it with one emit.
- **The whole chain in one kick from level 0.** The worst pixel (level 9 of
  800x600) would be a cascade of 4^8 leaf samples. Levels 1..9 are 349,524
  texels, more than level 1's 400x300 grid, so this needs a new HWRT size.
  And only level 1 would be bit-identical.
- **Sampling a level written earlier in the same kick.** Nothing orders the
  stores between tiles (the first rule in the table). Even if the board
  happened to show it working, it would be a race.
- **The PBE MSAA resolve as the 2x2 box.** `downscale` averages a pixel's 4
  samples (`pbestate.xml:255`, `bxs-4-64.h:90`). Using it would need
  per-sample shading to put four level-k texels in one pixel. The sample
  store's layout is not a level's layout (`backend.rs:8880-8892`). And the
  resolve rounds like the PBE, not like `Q(T())`, so the level is not
  bit-identical. It also gives one level a kick.

## Ranking

| Rank | Route | Level kicks a frame (35 now; "5x9": 45) | Case 216 kicks (40 now) | Bit-identical by construction | New ABI | Code |
|---|---|---|---|---|---|---|
| 1 | tail: tail in one linear kick, s = 3 | 16 (20) | 21 | levels 1-4 | none | ~300 lines |
| 2 | mrt+tail: MRT 1-3, then the tail | 8 (10) | 13 | levels 1, 4 | per-target pitch | ~550 |
| 3 | mrt: MRT 3 a kick | 12 (15) | 17 | levels 1, 4, 7 | per-target pitch | ~300 |
| - | today | 35 (45) | 40 | all | - | - |

The tail route comes first. It keeps the single-target mip frame and changes no shared
struct. It leaves every large level byte for byte as today, and puts any
rounding question into levels of 450 texels and fewer. Mrt+tail is the follow-up
if A's board run shows the per-kick cost holds. It reuses A's tail kick.

## Board experiments that decide

**E1, before any route is built: does the ALU bilinear equal the TPU's?**
Write a `gl_test` case on an experiment branch. It fills an RGBA8 800x600
texture with random bytes and draws one kick with a shader that writes, for
each texel of a 400x300 target, `Q(T(L_0, c))` to output 0 and the ALU
`Q(bilinear(texelFetch x4, c))` to output 1. Run it at an even step
(800 -> 400) and an odd one (75 -> 37, 37 -> 18). Read both back and print
`differing bytes N, max |d| M` for each step.
- **N = 0** on both steps: all three routes are bit-identical at every
  level, and the board's ES 2.0 texel answers do not move.
- **N > 0 and M <= 1:** the deep levels move by at most 1 LSB. dEQP-GLES2 and
  the glmark2 pictures (tolerance 8) decide. Prefer the tail route, which confines the
  change to levels of 450 texels and fewer.
- **M > 1:** the ALU is wrong (the coordinate quantisation). Fix the
  emulation before building a route.

**E2, after the tail route is built, from a main commit:** case 216 and glmark2.
- Record values: `gl_test: mipchains: ... ms a frame` with its kicks a frame
  (**40 -> 21**), and the four quadrant probes PASS. glmark2 terrain ms a
  frame from `tools/cts/board-stage.sh am62p common`, against 83.06.
- Prediction, not a measurement: if the frame stays kick-bound at today's
  cost per kick, case 216 falls to about 46.15 x 21/40 = **24 ms**. Terrain
  falls to about 83.06 x 28/53 = **44 ms** on the "53 kicks" count, or by
  19 x 1.15 = 22 ms on the four-chain count (about 61 ms).
- The tail route is confirmed when case 216's kicks read 21 and its ms a frame falls by
  at least 15 ms (13 fewer kicks at the 1.15 ms queued cost, with
  slack for the tail kick's shading). Terrain's picture must be within
  tolerance 8 of column `am62p`, and the read-back of levels 5..9 must agree
  with E1's verdict.
- If the ms do not fall with the kicks, the cost of a frame is not the kick
  count. Like path B, it is then the waits: `HWRT_MEMBERS` = 4 per size
  (`render.rs:85`), and terrain's three 800x600 chains share one size. That
  is `pvr-waited-kick`'s question, not this one's.
