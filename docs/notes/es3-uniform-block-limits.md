<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 uniform-block limits: where a 16 KiB block is read from, per GPU

Plan #359, items 1 and 4 (read-only design; no limit moves with it).
Lines are at main `40539acd`. Mesa lines are Mesa 26.2.1.

## What ES 3.0 asks, and what is reported

ES 3.0 Table 6.32: `MAX_UNIFORM_BLOCK_SIZE` at least 16384 bytes, and
`MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` at least
`MAX_{stage}_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4 +
MAX_{stage}_UNIFORM_COMPONENTS`: 12 * 4096 + 1024 = 50176 for the vertex
stage and 12 * 4096 + 896 = 50048 for the fragment stage. dEQP checks the
fixed minimums in `implementation_limits` and, in `state_query.integers64`,
the formula from the values the implementation reports
(es3fInteger64StateQueryTests.cpp:240-249 at VK-GL-CTS 93bca01).

Every GPU reports `MAX_UNIFORM_BLOCK_SIZE` 6144 and both combined limits
1536 (`src/gl/state.rs` `get_integerv`, `MAX_UNIFORM_DWORDS` 1536 from
`src/gl/context.rs`). Those numbers are honest for the design the GL core
has: a uniform block is not read from its buffer by the GPU. The core
flattens each block into the stage's default block and copies the bound
range into it at every draw (`src/gl/program.rs` `fill_uniform_blocks`,
`copy_block_member`), and the default block is `MAX_UNIFORM_DWORDS` long
(`block_dwords` caps it, `program.rs:1861-1869`). So one block holds at
most 1536 dwords, and the blocks and the default uniforms of a stage share
the same 1536: the combined limit is that, not a sum.

Raising the reported values alone would be wrong: a program with a 16 KiB
block would link and then be truncated at `block_dwords`, silently.

## What each GPU must carry

A 16 KiB block, twelve of them a stage, cannot be a copy into a per-draw
block (twelve blocks are 192 KiB a stage a draw). Each GPU has to read a
user block from the buffer's memory, as Mesa does on all three.

- **V3D 4.2.** Today every uniform is a word of the uniform stream
  (`QUNIFORM_UNIFORM`, a dword index into the stage's storage, written per
  draw by the job builder from `fs_storage`/`vs_storage`), and a uniform
  array read at a run-time index loads every element and selects (at most
  256 values, `prepare.rs` `LoadUniformIdx`, "no ldunifa yet"). Mesa reads
  a user block (UBO index > 0) from memory: `QUNIFORM_UBO_ADDR` gives the
  block's address, written to `UNIFA`, and `ldunifa`/`ldunifarf` stream the
  dwords from there (nir_to_vir.c `ntq_emit_load_unifa`, `emit_ldunifa`),
  or a TMU general load for a run-time offset. What it needs here:
  1. the compiler: `UNIFA` writes and `ldunifa` reads (`isa.rs` already
     decodes the `ldunifa` signal bits; selection, scheduling and the
     interpreter do not), a `QUNIFORM_UBO_ADDR` uniform per block;
  2. the back end: the per-draw table of the bound blocks' GPU addresses
     (a field beside `fs_storage` in `struct v3d_job_draw`, its pins) and
     the job builder's `QUNIFORM_UBO_ADDR` word;
  3. the GL core: the blocks of a V3D program not flattened
     (`program.rs`), the binding's buffer and offset handed to the back
     end per draw, then `MAX_UNIFORM_BLOCK_SIZE` 16384 and the combined
     limits by the formula (`state.rs`).
- **GC7000UL.** The constant file is 320 vec4 registers for both stages,
  a silicon limit (`docs/notes/gc-ubo-const-limit.md`). The compiler copies
  every block member into it; the path that reads a block from memory with
  `LOAD` exists (`SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`) and is used for
  run-time indexes only. Mesa etnaviv reads every user block with `LOAD`.
  The same three steps: every user block through `LOAD`, the addresses per
  draw, the core's flattening off for the GC7000.
- **PowerVR BXS-4-64.** The default block goes to the USC's shared
  registers, filled per draw by a PDS `DOUTD` program
  (`src/pvr/backend.rs:7780-7795`, `8353-8361`). How many shared registers
  a stage may take, and so whether 1536 dwords are already near the
  ceiling, is [U] (not read to the line). Mesa's pco reads a UBO from
  memory with a load from its descriptor's address. The same three steps.

## The order, and what it waits for

1. The V3D first (this plan's instruction): steps 1 and 2 are
   `src/shaderc/online/v3d/**`, `src/v3d/**` and `driver/src/v3d/**`.
2. Step 3 is `src/gl/program.rs`, which plan #343 holds; the raise waits
   for #343 to release it, then lands with steps 1-2 of the V3D. Until all
   three are in, the reported values stay at 6144 / 1536 on every GPU, and
   `tests/es3_caps.rs` `known_gaps`, `tests/es3_limits_v3d.rs`
   (`the_uniform_block_family_meets_es3_0`, ignored) and
   `src/gl/state.rs` `the_es3_limits_are_what_this_gpu_carries_and_programs`
   keep them listed.
3. The GC7000UL (`src/vivante/**`, plans #335 and #355) and the PowerVR
   (`src/pvr/**`, plans #350 and #325) follow their owners: issue drafts of
   plan #359.

The rows this closes: `implementation_limits.max_uniform_block_size`,
`.max_combined_vertex_uniform_components`,
`.max_combined_fragment_uniform_components` and their
`state_query.integers64.*_getfloat` / `*_getinteger` rows (6 Fail on the
Pi 4 and on the AM62P), and the `ubo.*` cases a block over 6144 bytes or
over the shared 1536 refuses today.
