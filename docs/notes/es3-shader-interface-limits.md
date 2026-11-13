<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface and uniform limits, per GPU

Plan #359. What each limit can honestly be on each GPU, what changed, and
what is left with the work it needs. The V3D varyings have their own design
(`v3d-varyings.md`); the transform-feedback, sample and format limits are
other plans'. Mesa lines are Mesa 26.2.1 as `docs/es3/backend-caps.md` cites
them. HOST-ONLY means the host dEQP build (`tools/cts/README.md` 2b), which
keeps the command stream instead of running it: its verdicts on the limit
queries are real, a render verdict from it is never quoted.

## What is reported (ES 3.0 build)

| limit (ES 3.0 minimum) | V3D | PowerVR | GC7000UL | Mali (stage 5, untouched) |
|---|---|---|---|---|
| MAX_VARYING_VECTORS (15), MAX_VARYING_COMPONENTS (60), MAX_FRAGMENT_INPUT_COMPONENTS (60) | 15 / 60 / 60, was 8 / 32 / 32 | 15 / 60 / 60 | 15 / 60 / 60 | 8 / 32 / 32 |
| MAX_VERTEX_OUTPUT_COMPONENTS (64) | **64**, was 32 | **64**, was 60 | 60 (open, below) | 32 |
| MAX_UNIFORM_BLOCK_SIZE (16384) | 6144 | 6144 | 6144 | 6144 |
| MAX_COMBINED_VERTEX / FRAGMENT_UNIFORM_COMPONENTS (50176 / 50048) | 1536 | 1536 | 1536 | 1536 |

`report-es2` answers 8 for MAX_VARYING_VECTORS everywhere and refuses the
other pnames, as it did (`tests/es2_report_guard.rs`).

## Varyings and the vertex output

The spec counts `gl_Position` among the vertex outputs and not among the
user varyings: 16 vectors out (64 components), 15 in (60). So 64 is
honest exactly where the stage carries the position and 15 varyings
together, with or without a `gl_PointSize` beside them.

- **V3D**: `v3d-varyings.md`, "Status". 60 scalar fragment inputs against
  Mesa's `V3D_MAX_FS_INPUTS` 64, flag words past 24 inputs, the vertex
  shader's 65 VPM rows (9 sectors) against the record's 4-bit sector size.
  Before this the job builder refused any draw of more than 24 inputs, so a
  program of seven vec4 varyings was refused at the draw although 8 was
  reported. `tests/es3_v3d_varyings.rs`, `tools/v3d/job_check.c` part 13b.
- **PowerVR**: 15 / 60 / 60 were already carried (`tests/es3_pvr_varyings.rs`
  builds fifteen vec4 with `gl_FragCoord.z`, `gl_PointCoord` and a flat
  one, the last iterator at coefficient 240 of the 252 the `ditrp` offset
  encodes). Its vertex output is the 4 dwords of the position, the 60 of the
  varyings and the point size, 65 dwords, in the VDM's 8-bit size in units
  of 4 dwords: the 64 components were reached all along and only the query
  said 60. This change reports 64.
- **GC7000UL, [U]**: the vertex stage has 16 io rows (`SHC_VIV_MAX_IO`),
  the position in one (`vivante/backend_tests.rs`
  `fifteen_vec4_varyings_are_programmed_not_a_panic`), and Mesa's etnaviv
  takes `max_varyings` 16 with one slot for the position
  (`etnaviv_screen.c:965-968`). What is not known is whether a
  `gl_PointSize` write takes an io row of its own or a register beside them
  (`select.rs` `vs_pointsize_reg` puts it in a register above the others,
  and the io tables do not list it, which suggests the latter). If it is a
  register, 64 is reached as on the PowerVR and the raise is the one line
  in `context.rs`; the board test is a vertex shader of position, 15
  varyings and `gl_PointSize` that draws (gl_test case 97's vertex shader
  plus a point size). Left at 60, issue text 4 below.

## Uniform blocks: 16384 bytes and the combined components

**The spec's number.** ES 3.0 table 6.32: MAX_UNIFORM_BLOCK_SIZE >= 16384
and MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS >= MAX_VERTEX_UNIFORM_BLOCKS x
MAX_UNIFORM_BLOCK_SIZE / 4 + MAX_VERTEX_UNIFORM_COMPONENTS, 12 x 4096 +
1024 = 50176 for the vertex stage and 12 x 4096 + 896 = 50048 for the
fragment stage (the minimums of the default blocks). The formula is the
upper bound a link may be asked to accept: a program may declare twelve
blocks of 16384 bytes in one stage and must link. So reporting the formula
is honest only when twelve full blocks can be read, which is not the same
as one block of 16384.

**What the GL core does today.** A uniform block is flattened into its
stage's default block (`program.rs`) and filled from the bound buffer range
at each draw: the block's dwords are copied into the program's per-stage
array (`GlProgram::vs_uniforms`, `fs_uniforms`, `MAX_UNIFORM_DWORDS` = 1536
dwords each, `program.rs` allocation; the staging block of
`MAX_UNIFORM_DWORDS * 4 * 2 + 16` bytes in `draw.rs`; `u16` dword counts),
sharing that budget with the default uniforms. One block can be at most
the stage's budget, 6144 bytes, and the combined components are that same
1536. `MAX_VERTEX/FRAGMENT_UNIFORM_VECTORS` (384) and the Appendix A
packing are derived from the same constant, so raising the budget for
blocks must not move the ES 2.0 vector limits: the budget for blocks and the
default block's become two constants.

**Where a 16384-byte block is read from, per GPU.**

- **V3D.** The QPU reads a uniform as the next word of the stage's uniform
  stream, built at draw time (`QUNIFORM_UNIFORM`: the datum is a dword index
  of the stage's block). `tests/v3d_ubo_large_sim.rs` already runs the
  compiled code on the interpreter against a 4096-dword block: constant
  indexes 0, 383, 384 and 1023 of a `vec4[1024]` block (8 members of 128),
  and dynamic indexes at 0, 383, 384, 1023 (16 members of 64), all pass.
  What does not: a dynamically indexed `vec4` array of more than 64 entries
  (`prepare.rs` `MAX_SELECTED` = 256 components, selects among them, no
  `ldunifa` yet) and an array of 256 (the reflection's `array_len` is a
  `u8`); both are `#[ignore]`d probes there. So the compiler is not the
  bound for the block size; the GL core's budget and the per-draw stream
  are (every read costs one stream word, a draw's block is snapshotted into
  the frame's arena). For the combined components, flattening twelve blocks
  per stage is 196 KB of copy per stage per draw: the real design is Mesa's
  (`v3d_nir_lower_ubo_loads`-style reads through `ldunifa` from an address
  held in the stream, `QUNIFORM_UBO_ADDR`; `sim.rs` models only
  `contents == 1` today and reports the rest): the draw puts the bound
  buffer's GPU address and offset in the stream and the shader loads from
  memory.
- **GC7000UL.** Mesa's etnaviv reads a user block from memory with `LOAD`
  and never copies it to the constant file (`gc-ubo-const-limit.md`); the
  path exists here (`SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`, proven on the
  i.MX8MP for run-time indexes) and is used only for those. The constant
  file is 320 registers, `GC_MAX_UNIFORM_DWORDS` = 1280 dwords a stage
  (`vivante/shader.rs`), so 4096 dwords cannot be constants at all: a block
  past what fits is a memory read, which is also what makes twelve blocks
  possible.
- **PowerVR, [U].** The block is DMA'd by the PDS from the program's uniform
  block into shared registers (`GlesDraw::vs/fs_uniform_gpu`, `*_dwords`):
  the cap is the shared-register file, not read in this plan
  (`src/pvr/**` is #350's). Whether the USC has a load from memory for a
  uniform (Mesa's `pvr` lowers `load_ubo` to a memory read through the
  constant-buffer address in the PDS, `rogue_nir_lower_ubo`-style) is the
  first question for the PowerVR issue.

**The change.** In the GL core, a block bound by `glBindBufferRange` stops
being copied into the default block when the GPU can read memory: the draw
carries (GPU address, size) per block and per stage, the link reports each
block's layout (it does already), and the budget of the default block
returns to 1536. Then MAX_UNIFORM_BLOCK_SIZE is the buffer's own limit
(16384 or more), the combined components follow the formula with the
GPU's default-block value, and `program.rs` (#343), `draw.rs` (#339) and
the three back ends (#335, #350, V3D) each change. Reporting 16384 without
that, from a larger flattened budget, would answer the block-size row and
leave the combined rows below the minimum (they are fixed numbers, 50176 and
50048): half the family, and a reported limit the link could not meet for
twelve blocks. This plan does not do it; the issue texts split it per
owner. `tests/es3_limits_v3d.rs`
`the_uniform_block_family_meets_es3_0` is the `#[ignore]`d expectation.

## The float and 64-bit query forms

dEQP's `state_query.integers` and `integers64` ask each limit through
`glGetIntegerv`, `glGetFloatv` and `glGetInteger64v` and expect the same
number. They fail on limits below their minimum in the form that checks the
value and on no other account: `get_integerv` is the single source
(`get_floatv` answers only the true float state and 0 values for any
other pname, `glGetFloatv` and `glGetInteger64v` then convert
`get_integerv`'s answer; unsigned state widens unsigned), so no limit answers one number
through one form and another through the next. The board rows of
`docs/es3/pi4-gles3-fails.tsv` (2026-10-31) and the AM62P's are the
values. Pinned: `tests/es3_limits_v3d.rs`
`every_g3_pname_answers_the_same_in_all_three_integer_forms` (15 pnames) and
the four varying limits in all three forms in `tests/es3_v3d_varyings.rs`.
The 17 + 6 rows therefore close with the values; the family that stays open
is the uniform one above, MAX_SAMPLES (#339), the compressed-format count
(#354) and the colour attachments (RC14).

## Host dEQP, before and after (HOST-ONLY)

**Not run.** The host deqp-gles3 build (VK-GL-CTS at 93bca01 with the
yakogl target patch, fetched into the dependency directory) takes hours on
a 4-core worker: 226 of 748 build steps in about two hours, with the
library builds and the 2 x 3 runs still to come, so it was stopped. What
stands in its place is the API verdict the host unit tests give for the same
questions (`tests/es3_v3d_varyings.rs`, `tests/es3_limits_v3d.rs`,
`tests/es3_pvr_varyings.rs`, `tests/es3_caps.rs` per GPU). To get the dEQP
numbers on a host with the deqp build: for GPU in v3d powervr vivante,
`MODULE=gles3 GPU=$GPU tools/cts/host-deqp-run.sh dEQP-GLES3.functional.<f>`
with `<f>` = `implementation_limits`, `state_query.integers` (covers
`integers64`), `shaders.linkage` and `ubo`, once at `f7a525e6` and once at
this change, and `--compare=` the two run directories. Expected: the four
varying rows of `implementation_limits` and their `state_query` forms pass
on the V3D, `max_vertex_output_components` also on the PowerVR, nothing
else moves.

## Issue texts for what is left

1. **V3D: uniform blocks read from memory, MAX_UNIFORM_BLOCK_SIZE 16384 and
   the combined components.** The V3D flattens blocks into the 1536-dword
   default block (`program.rs`, `draw.rs`, `MAX_UNIFORM_DWORDS`), so
   MAX_UNIFORM_BLOCK_SIZE is 6144 and MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_
   COMPONENTS 1536 (ES 3.0: 16384, 50176, 50048). Do: split the block budget
   from the default block's (the ES 2.0 vector limits stay), let the draw
   carry a bound range's GPU address and size per block, the V3D compiler
   read it with `ldunifa` from a `QUNIFORM_UBO_ADDR` stream word
   (`sim.rs` models `contents == 1` only; extend it and
   `tests/v3d_ubo_large_sim.rs`, whose probes of arrays past 64 `vec4`
   dynamically indexed are `#[ignore]`d), then report the block size from
   the buffer and the combined components from the formula, under
   `!report-es2`. Waits for #343 (`program.rs`) and #339 (`draw.rs`).
   dEQP: `implementation_limits.max_uniform_block_size`,
   `max_combined_{vertex,fragment}_uniform_components`, the
   `state_query.integers*` forms, `ubo.*`. Host-checkable.
2. **PowerVR: uniform blocks past the shared-register budget.** Same
   family, `src/pvr/**` (#350): does the USC read a uniform block from
   memory (PDS constant-buffer address, Mesa `rogue_nir_lower_ubo`), what is
   the largest block the shared registers take, and the PDS program that
   points at the bound range. Board or host (USC code shape) measurement
   first.
3. **GC7000UL: user blocks always from memory.** `SHC_VIV_CONST_UBO_ADDR`
   exists and is used for run-time indexes; use it for any block over the
   constant budget and report 16384 / the formula (`src/vivante/**`, #335,
   #355; `gc-ubo-const-limit.md` has the register arithmetic). Host golden
   against etnaviv's `LOAD`, board check on the i.MX8MP.
4. **GC7000UL: MAX_VERTEX_OUTPUT_COMPONENTS 60 -> 64.** Does a vertex shader
   with the position, 15 varyings and `gl_PointSize` fit the 16 io rows?
   Board: gl_test 97's vertex shader plus a point size on the i.MX8MP, one
   `glDrawArrays`. If it draws, `context.rs` `MAX_VERTEX_OUTPUT_COMPONENTS`
   gets `gpu-vivante` in its `any(...)`, `tests/vivante_imx8mp_limits.rs`
   and `tests/es3_caps.rs` follow.
5. **V3D: board check of 15 varyings.** Pi 4: a vertex shader with 15 vec4
   varyings and a point size (65 VPM rows), a fragment shader reading all 60
   with a flat one in the last word and a centroid one in word 1, and one
   with `gl_PointCoord` beside them; dEQP `shaders.varying.*` and
   `implementation_limits.max_varying_*`. Nothing here ran on the board.
   Then re-run the board lists whose `*_getfloat`/`*_getinteger64` rows are
   the same values.
6. **ES 3.00 link does not check varying components** (`reflect.rs`
   `varying_budget` returns early for ES 3.00; the ES 1.00 path checks): a
   program past MAX_VARYING_COMPONENTS should fail at link on every GPU.
