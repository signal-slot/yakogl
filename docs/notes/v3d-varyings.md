<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D varyings: 15 vectors and 64 vertex output components

Plan #359, item `v3d-varyings-note`, from the read-only design
`design-v3d-varyings`. Line numbers are at the integration tip `2aee4c92`
(the `src/` and `driver/` files may have moved by a few lines since);
Mesa lines are Mesa 26.2.1 (`$MESA_FULL_SRC_26_2_1`: the vendored
`third_party/mesa-26.2.1` has no v3d driver or compiler sources). Code
that relies on this design cites this note.

## What is reported now, and what the ES 3.0 spec wants

`src/gl/context.rs:149-159` gives the V3D `MAX_VARYING_VECTORS` 8 (a first
cut, "raised with the board run"). `context.rs:188-194` derives
`MAX_VERTEX_OUTPUT_COMPONENTS` and `MAX_FRAGMENT_INPUT_COMPONENTS` as
`MAX_VARYING_VECTORS * 4`, and `state.rs:1254-1256` answers
`MAX_VARYING_COMPONENTS` the same way, so all four move together. The
shader's `gl_MaxVaryingVectors`, `gl_MaxVertexOutputVectors` and
`gl_MaxFragmentInputVectors` are these divided by four.

ES 3.0 minimums: `MAX_VARYING_VECTORS` 15, `MAX_VARYING_COMPONENTS` 60,
`MAX_FRAGMENT_INPUT_COMPONENTS` 60, `MAX_VERTEX_OUTPUT_COMPONENTS` 64.

Why vertex output is 64 and varyings 60: the spec counts `gl_Position` (one
vec4) among the vertex outputs, so 16 vectors leave 15 for user varyings
(60 components). The fragment side has 15 vectors; the built-ins it reads
are not counted as inputs the same way. On the V3D the fixed rows are not
varyings: a vertex's VPM output is four fixed-function rows (x, y, z, 1/w:
`select.rs:2578-2587`), one more row if `gl_PointSize` is written
(`select.rs:2549-2553`, Mesa `v3d_nir_lower_io.c:410-431`), then the user
varyings, one dword each (`select.rs:2565`, `4 + skip + varyings`). So
`gl_Position` and the point size cost VPM rows beside the varyings; they
are not taken from the 64 fragment inputs the hardware interpolates.

## The hardware and Mesa's limit

Mesa's v3d takes 64 inputs for a fragment shader and 64 per vertex
(`src/broadcom/common/v3d_limits.h:32-35`, `V3D_MAX_FS_INPUTS` 64) and
reports `max_varyings = V3D_MAX_FS_INPUTS / 4` = 16
(`src/gallium/drivers/v3d/v3d_screen.c:323`); the state tracker copies it
(`src/mesa/state_tracker/st_extensions.c:397-398`). So Mesa answers
16 vectors and 64 components for all of them.

## What the code carries today, stage by stage

- **FS input list and flag words** (`src/shaderc/online/v3d/chain.rs:419-445`):
  `flat` and `centroid` are one `u32` each, bit *i* for input *i*; input
  32 or later is `NotYet`. `select_fragment` takes the same `u32`s
  (`select.rs:2210-2216`). `FsProg` carries them at `chain.rs:138-140`
  (`flat_shade_flags`, `centroid_flags`; `noperspective_flags` beside
  them). The comparison JSON prints one word (`chain.rs:1322-1327`).
- **VS output rows** (`select.rs:2560-2562`): `rows > 64` is
  `VpmRowOutOfRange`. With 4 fixed rows, the point size and 64 varyings
  that is 69. `vpm_fields` (`chain.rs:320-337`, Mesa
  `v3d_vs_set_prog_data`, `vir.c:896-937`) sizes the VPM output from
  `vpm_output_rows` in 8-row units and refuses a VPM that cannot hold two
  batches (`chain.rs:69-70`, `vir.c:936`); with 69 rows a batch is
  9 units, which the 64 KiB VPM still holds twice. The 4.2 and 7.1 VPM
  sizes must both be re-run through `vpm_fields` in the compiler item.
- **Draw side** (`driver/src/v3d/v3d_job.c:501-513`, `759-764`, `800-802`;
  `v3d_frame.c:595-598`, `694-697`; `v3d_frame.h:174-176`): `struct
  v3d_fs_prog` holds word 0 only (24 varyings), the job builder refuses
  `num_inputs > 24`, and `EMIT_VARYING_FLAGS` emits one packet per kind
  with the higher words zeroed (`ZERO_ALL_*` when word 0 is 0).
  `number_of_varyings_in_fragment_shader` is `fs->num_inputs`
  (`v3d_frame.c:697`). The Rust mirror is `V3dFsProg`
  (`src/v3d/ffi.rs:76-95`, pin at `ffi.rs:835`, C side
  `ffi_layout_check.c:29`), filled at `src/v3d/backend.rs:3680-3682`.
- **gl_PointCoord** takes two implicit varyings ahead of the declared ones
  and only for points (`backend.rs:3692-3697`, `chain.rs:534-535`).
- **ES 3.00 link** has no check of the varying component count
  (`src/shaderc/online/reflect.rs:1068-1070`; the ES 2.00 path checks at
  `src/shaderc/link.rs:351`).

## Mesa's design

- Flags are bitsets of 64 (`V3D_MAX_FS_INPUTS`) bits packed to 24-bit
  words, word *i/24* bit *i%24*: `v3d_set_fs_prog_data_inputs`,
  `src/broadcom/compiler/vir.c:985-1003` (the `STATIC_ASSERT` says the
  array has at least `(64-1)/24+1` = 3 words).
- The draw emits each kind as one to three packets:
  `emit_varying_flags`, `src/gallium/drivers/v3d/v3dx_emit.c:177-208`.
  The first non-zero word *i* emitted, if *i* is 0, leaves the lower
  words unchanged and zeroes the higher ones; if *i* > 0 it zeroes both
  directions; every later word leaves both unchanged. The callers are at
  `v3dx_emit.c:621-643`; the per-kind callbacks are `127-176`.
- The packet is `Flat Shade Flags`, `Non-perspective Flags`, `Centroid
  Flags`: a 24-bit flag field at bits 8-31, 2-bit actions for the higher
  and lower numbered varyings at bits 6 and 4, and a 4-bit "varying offset
  V0" (the word index) at bit 0 (`src/broadcom/cle/v3d_packet.xml:810-815`
  and `870-884`). Offsets 0-2 cover 72 inputs; the field itself reaches
  word 15.

## Decision

Report 16 vectors and 64 for `MAX_VARYING_COMPONENTS`,
`MAX_FRAGMENT_INPUT_COMPONENTS` and `MAX_VERTEX_OUTPUT_COMPONENTS`, Mesa's
answer, on the V3D only (above ES 3.0's 15/60/60/64, so conformant). The
fallback if a board rejects 16 vec4s (see the open board check below) is
15/60/60/64, with the three component constants split from
`MAX_VARYING_VECTORS * 4` into 64/60/60 per GPU build, as the triage note
asked. In either case the constants in `context.rs:193-194` stop being
`MAX_VARYING_VECTORS * 4` on the V3D, and `state.rs:1254` follows. Under
`report-es2` the V3D stays 8 (`tests/es2_report_guard.rs`).

## Changes of `v3d-varyings-compiler`

1. Flag sets become three 24-bit words as Mesa does (`vir.c:992-1003`):
   `chain.rs:138-140` and `419-445` (drop the "input past 31" refusals),
   `FsProg`, `ShcV3dFs`/`shc_v3d_fs`, the `abi_layout` pins and
   `v3d_abi.rs`; the comparison JSON (`chain.rs:1322-1327`) prints all
   words.
2. `select_fragment` (`select.rs:2210`) takes the flags as 64-bit tests
   (a `u64` pair or `[u32; 3]`).
3. `select_vertex` raises its row cap (`select.rs:2560`) from 64 to
   4 + 1 + 64 = 69, and `VpmRowOutOfRange`'s doc (`select.rs:402`) moves
   with it.
4. `chain.rs` refuses more than 64 fragment inputs by name rather than by
   `NotYet`, and `vpm_fields` stays correct at 69 rows for both targets.
5. `sim.rs` needs no change: it interprets the code, not the flag words
   or the VPM layout. The host tests (`tests/es3_v3d_varyings.rs`, from
   `v3d-varyings-probe`) pin the 60- and 64-component shaders.

## Changes of `v3d-varyings-raise`

1. `V3dFsProg` and C `struct v3d_fs_prog` (`v3d_frame.h:174-176`) hold
   three flag words each; the pins in `ffi.rs:835` and
   `ffi_layout_check.c:29` move (the struct becomes 80 bytes), and
   `backend.rs:3680-3682` copies all three. The comparison fixtures keep
   only word 0; the new tests and `job_check.c` check words 1 and 2.
2. `EMIT_VARYING_FLAGS` (`v3d_job.c:501-513`) becomes a port of Mesa's
   `emit_varying_flags` (`v3dx_emit.c:177-208`) with the packet layout
   above (`varying_offset_v0` = the word index, the actions as in the
   rule above); `v3d_frame.c:595-598` (the all-zero case) widens the same
   way.
3. The draw cap rises from 24 to 64 inputs (`v3d_job.c:800-802`,
   `v3d_frame.c:597`, `v3d_frame.h:175`).
4. `context.rs:149-159` and `188-194` get the `cfg` split: the V3D answers
   16 and 64/64/64; PowerVR, GC and Mali are unchanged (Mali stays as it
   is for the stage-5 plans). `state.rs:1254-1256` reads the new
   constants.
5. Tests: `tests/es3_caps.rs` `known_gaps`, `tests/es3_limits_v3d.rs` and
   `tests/es2_report_guard.rs`, listed in the design's section 6.
6. Rebase onto #354 and #356 for `ffi.rs` and `backend.rs`.

## Open, no owner yet

- The ES 3.00 link does not check varying component counts
  (`reflect.rs:1068-1070`); with 64 inputs a shader past the limit should
  fail at link, as dEQP expects.
- `tools/shaders/known-uncompilable-v3d.txt:38-40` needs a separate entry
  per build once the limits differ.
- A Pi 4 board check, a step for #320: 16 vec4 varyings plus
  `gl_PointCoord` (does `num_inputs` of the shader record, two implicit
  point varyings included, stay within what the hardware takes?), and
  flat and centroid bits in words 1 and 2. Nothing here has run on the
  board; the 16 is Mesa's number, not a measurement of ours.
