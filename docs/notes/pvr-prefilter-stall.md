<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the online prefilter program stops the first cube-face draw

2026-10-22, host only. Branch `work/pvr-fragment-tail` (`fbfbca3e`). The
investigation changed nothing under `src/` and used no board; the H1 guard
(last section) is host-verified only. Issue #1635.

## The defect

On the Verdin AM62P, the first draw that runs Qt Quick 3D's environment-map
prefilter (`tools/shaders/qt3d/environmentmapprefilter_300.frag`) with the
online program never completes. That draw renders 512x512 into a cube face,
mip 0, so `roughness` is 0 and `distribution` is 1 (GGX). The fence does not
move, there is no HWR, and the render service stops after 10 s. This happens
with `sampleCount` 1024 and with 8. The Mesa-reference (pco) program for the
same source renders the scene correctly.

## The two programs side by side

Both programs are decoded with `listing::group_text`: ours from
`chain::compile_fragment`, pco's from `SHC_BLOB_TABLE` (blend key 0).

| | ours | pco |
|---|---|---|
| bytes / groups | 9120 / 672 | 2856 / 270 |
| declared temps | **250** | 33 |
| `if`s in the loop body | all if-converted: both arms run, `movc` joins, no `cnd*` | branches under the execution mask (`cndst`/`cndef`, `br` past an arm no instance runs) |
| the two `textureLod` samples | **run on every pass for every instance** | each runs only inside its arm |
| `smp` fields | `fcnorm=1 drc=0 dmn=3d chan=4 lodm=replace pplod=1 proj=0 nncoords=0` | identical |
| `smp` operands | image state `sh0`, sampler state `sh8`, data `t3..t6` (block `t3..t18`, 7..18 zero), result `t19..t22` | `sh0`, `sh8`, data `t15..t18`, result `t8..` |
| `wdf` | right after each `smp`, before the result is read | the same |
| loop | test at the top sets the break flag, back edge `br if(p0)` on `Ior(Ige(c, 4096), flag)`. The body makes one more pass with its stores guarded (9 passes for `sampleCount` 8) | `cnd` loop |

The `smp` encoding, the descriptors, the `wdf` placement and the texture
state are the same in both programs. The back end builds one descriptor for
both programs, so the descriptor cannot be the difference.

## What the host shows

The byte model (`tests/usc_exec`, `take_levels()`) ran our program with the
first draw's uniforms (roughness 0, resolution 512, lodBias 0, distribution
1) and with `sampleCount` 8. The level word of each `smp`, in program order,
was:

```text
[NaN, 0.0, NaN, 0.0, ...]   18 samples: 9 passes x 2
```

The first sample of each pass belongs to the `distribution == 0` arm. Its
level comes from the GGX pdf. At roughness 0, `D_GGX(1.0, 0.0)` is 0/0, so
the pdf is NaN and `computeLod` returns NaN. The GLSL never samples with this
value. The `roughness == 0.0` guard and the `distribution` test keep it out,
and pco's branches skip the arm. Our if-conversion issues the sample anyway
and discards its result in a `movc`.

At roughness 0.25 every level is finite (6.6 to 9.6). With `sampleCount` 0
the extra pass samples at level +inf. pco's program cannot be run in the
model, which lacks `msk_lsl`.

## Hypotheses, ranked

1. **The TPU is handed a NaN level (H1).** Every pixel of the first draw
   issues one `smp` per pass with level NaN. The sampler state clamps the
   level to `maxlod` (`tex_desc_sampler`). A comparison with NaN fails, so the
   value can get past the clamp. If the level then reaches the address
   computation, the TPU reads far outside the chain. On Rogue an MMU fault
   stalls the requester and does not trigger HWR. That matches "no fence, no
   HWR" and "8 and 1024 alike". It also matches "the first draw": only mip 0
   has roughness 0. pco never produces this access.
2. **250 temporaries (H2).** Mesa allocates at most `rogue_get_temps()` =
   `MIN2(unified_store_temps_per_instance / 2, 248)` per instance
   (`include/hwdef/rogue_hw_utils.h`, used by `pco_ra.c`), and spills above
   that. Our cap is the DOUTU field's 252 (`abi::USC_MAX_TEMPS`). Our program
   declares 250 and uses `t248`/`t249`. No pco program ever does, so this is
   untested ground on the hardware. Until the tail fix (`930e24a9`) this
   shader was refused at 255. It is the only program known to run past 248.
3. **Throughput (H3, unlikely).** Ours is about 3x pco's groups per pass and
   makes one extra pass. At 250 temps about 3 of the 64 slots fit. That
   explains a slow draw, not an endless one at `sampleCount` 8.

## Board experiments that separate H1 and H2

- **H2:** run the working (table) program with
  `PVR_BACKEND_OPT_FS_TEMPS_MIN` = 250 (`backend.rs`, capped at 252). Only
  the kick's temps allocation changes. If the draw stops, the allocation is
  the cause, and the fix is `USC_MAX_TEMPS` = 248 plus a smaller program.
  This test does not check the use of register indices 248 and 249.
- **H1:** a one-draw case, `textureLod(samplerCube, dir, u)` with `u` = NaN
  (a uniform), on a cube with a 10-level chain. If the draw stops, the
  hardware hangs on NaN levels.

## The fix each hypothesis calls for

- H1: a sample must not run outside its arm. Either an `if` whose arm samples
  takes pco's branched form under the execution mask, as
  `lower_if_cond_exec` builds it (today only `predicable` arms can), or the
  if-converted sample's level and coordinates are selected to safe values
  when the arm is off. The first fix also removes most of the `movc` joins
  that drive the 250 temps.
- H2: `USC_MAX_TEMPS` 248, as Mesa uses. Then this shader is refused again
  (it needs 250) until its register pressure drops. That pressure comes from
  about 100 locals carried round the loop and joined at every flattened `if`,
  plus the 20 temporaries the sample block pins for the whole program
  (`expand_samples`: 16 data words and 4 result words, where this program
  reads 4 data words).

## The H1 guard (host-verified, not yet on the board)

`select::Sel::guard_sample` takes the second H1 fix. A look-up in a
flattened arm gets its level word or bias selected through the
arms' condition: level 0 where the arm is not taken. Its coordinates are
selected too where the arm also computes the level, and a cube's direction
always ((1, 0, 0) off the arm). The join still discards the result, so the
picture is the same. In the byte model, with the first draw's uniforms, the
prefilter's level words go from `[NaN, 0.0, NaN, 0.0, ...]` to all `0.0`
(`sampleCount` 8 and 1024). With `sampleCount` 0 the extra pass's +inf is
gone too. The pixels are unchanged. The program stays at 250 temps (9120 ->
9376 bytes, 672 -> 685 groups). So the guard tests H1 alone and leaves H2
open.

Operands that are the same whether the arm runs are not guarded: immediates,
uniforms, varyings read as they are, and exact plane gradients (made of a
varying and its planes). Neither are the coordinates of a constant level
(`textureLod(s, p, 0.0)`). Guarding those too put 96bc0c44f5554256.frag past
the 4 MiB scratch. That program's peak was already 4,156,180 bytes, and it
is 4,164,500 with the guard. A program with no guarded operand keeps its
bytes: femtovg's and gl_test's precompiled counts do not move.

The board check for H1, one draw of
`textureLod(samplerCube, dir, u_lod)` with the uniform `u_lod` = NaN on a
cube with a 10-level chain, outside any `if`, so the guard does not apply:

- the draw stops (no fence, no HWR): the TPU hangs on a NaN level, H1 holds,
  and the guarded prefilter should render;
- the draw completes: H1 is not the cause, and H2 comes next.

