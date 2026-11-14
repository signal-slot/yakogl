<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D varyings: 15 vectors and 64 vertex output components

Plan #359, item `v3d-varyings-note`, from the read-only design
`design-v3d-varyings`; the raise landed with the uniform-block raise of
`docs/notes/es3-shader-limits.md`. `src/` and `driver/` line numbers are at
this commit; Mesa lines are Mesa 26.2.1 (`$MESA_FULL_SRC_26_2_1`: the vendored
`third_party/mesa-26.2.1` has no v3d driver or compiler sources). Code that
relies on this design cites this note.

## What is reported, and what the ES 3.0 spec wants

`src/gl/context.rs:207-210` gives the V3D `MAX_VARYING_VECTORS` 15 (8 under
`report-es2`, the ES 2.0 answer as it was, `tests/es2_report_guard.rs`).
`context.rs:244-262` derives `MAX_VERTEX_OUTPUT_COMPONENTS` as
`MAX_VARYING_VECTORS * 4 + VERTEX_OUTPUT_EXTRA` (`VERTEX_OUTPUT_EXTRA` is 4 for
the V3D: `gl_Position` is written in the VPM beside the varyings) and
`MAX_FRAGMENT_INPUT_COMPONENTS` as `MAX_VARYING_VECTORS * 4`;
`state.rs` answers `MAX_VARYING_COMPONENTS` the same product. The shader's
`gl_MaxVaryingVectors`, `gl_MaxVertexOutputVectors` and
`gl_MaxFragmentInputVectors` are these divided by four
(`src/shaderc/online/lower.rs:12883-12884`), so all four move together.

ES 3.0 minimums: `MAX_VARYING_VECTORS` 15, `MAX_VARYING_COMPONENTS` 60,
`MAX_FRAGMENT_INPUT_COMPONENTS` 60, `MAX_VERTEX_OUTPUT_COMPONENTS` 64. All four
are met (Host: `tests/es3_v3d_varyings.rs`, `tests/es3_caps.rs`,
`tests/es3_limits_v3d.rs`).

Why vertex output is 64 and varyings 60: the spec counts `gl_Position` (one
vec4) among the vertex outputs, so 16 vectors leave 15 for user varyings
(60 components). The fragment side has 15 vectors; the built-ins it reads
are not counted as inputs the same way. On the V3D the fixed rows are not
varyings: a vertex's VPM output is four fixed-function rows (x, y, z, 1/w:
`online/v3d/select.rs` `select_vertex`), one more row if `gl_PointSize` is
written (Mesa `v3d_nir_lower_io.c:410-431`), then the user varyings, one dword
each. So `gl_Position` and the point size cost VPM rows beside the varyings;
they are not taken from the 64 fragment inputs the hardware interpolates.

## The hardware and Mesa's limit

Mesa's v3d takes 64 inputs for a fragment shader and 64 per vertex
(`src/broadcom/common/v3d_limits.h:32-35`, `V3D_MAX_FS_INPUTS` 64) and
reports `max_varyings = V3D_MAX_FS_INPUTS / 4` = 16
(`src/gallium/drivers/v3d/v3d_screen.c:323`); the state tracker copies it
(`src/mesa/state_tracker/st_extensions.c:397-398`). So Mesa answers
16 vectors and 64 components for all of them.

## The code, stage by stage

- **FS input list** (`src/shaderc/online/v3d/chain.rs`): `fragment_inputs`
  lays each used component out as one `ldvary` input in `ldvary` order, at most
  `MAX_FS_INPUTS` = 64 of them (`chain.rs:221`); a program past them is
  `V3dError::TooManyFragmentInputs` (`chain.rs:444-446`), not a `NotYet`.
- **Flag words.** Bit *i* means input *i* is flat (Mesa `emit_fragment_varying`),
  held as a 64-bit set in `select_fragment` and packed to 24-bit words by
  `flag_words`; `FsProg` (`chain.rs`) carries `flat_shade_flags`,
  `noperspective_flags` and `centroid_flags` (word 0) with their `*_hi[2]`
  arrays (words 1 and 2, inputs 24..63). The compiler ABI mirrors them:
  `shc_v3d_fs` in `include/gles_shaderc.h:253,259-260`, `ShcV3dFs` in
  `src/shaderc/mod.rs`, its `src/abi_layout.rs` pins at 24 and 48, and
  `src/shaderc/online/v3d/v3d_abi.rs`. `varyings_tests.rs`
  `flat_and_centroid_inputs_past_24_set_the_high_words` is the pin that the
  words are filled by position (before this change the compiler refused a flat
  varying past input 31 and truncated inputs 24..31 into the 24-bit field).
- **VS output rows**: `MAX_VPM_OUTPUT_ROWS` = `4 + 1 + 60` (`select.rs:438`),
  refused as `VpmRowOutOfRange` past it (`select.rs:2572-2573`); 65 rows are
  nine 8-row sectors. `vpm_fields` (`chain.rs:342-366`, Mesa
  `v3d_vs_set_prog_data`, `vir.c:896-937`) sizes the VPM output in sectors,
  keeps two batches (`chain.rs:362`, `vir.c:936`) and refuses a segment past 15
  sectors, which the record's 4-bit field cannot hold. With the 64 KiB VPM
  (`Target { vpm_size: 65536 }`) 65 rows leave enough batches.
- **Draw side** (`driver/src/v3d/v3d_job.c`, `v3d_frame.c`, `v3d_frame.h`):
  `struct v3d_fs_prog` (`v3d_frame.h:174-186`) carries the three flag sets as
  word 0 plus `*_hi[2]`, 80 bytes; `num_inputs` reaches 64 (`v3d_job.c:827`)
  and a flag bit at or past 24 *within a word* of any kind is refused
  (`v3d_job.c:829-834`). `EMIT_VARYING_FLAGS` (`v3d_job.c:504-525`) is the port
  of Mesa's `emit_varying_flags`, `varying_flag_packets` (`:529-533`) counts
  the packets for the group packing, and `number_of_varyings_in_fragment_shader`
  is `fs->num_inputs` (`v3d_frame.c:701`, `v3d_job.c:897`). The Rust mirror is
  `V3dFsProg` (`src/v3d/ffi.rs`, pinned against `src/v3d/ffi_layout_check.c`),
  filled at `src/v3d/backend.rs`. The M5 triangle builder (`v3d_frame.c`) keeps
  its own cap -- `num_inputs <= 24` and word 0 only -- and refuses a program
  whose `_hi` word is nonzero (`v3d_frame.c:597-601`).
- **gl_PointCoord** takes two implicit varyings ahead of the declared ones
  and only for points (`src/v3d/backend.rs`, `uses_implicit_point_line_varyings`
  in the record; `job_check` part 11 `:1612-1635`): sixty declared inputs with
  them are 62 of the 64.
- **ES 3.00 link** has no check of the varying component count
  (`src/shaderc/online/reflect.rs`; the ES 2.00 path checks at
  `src/shaderc/link.rs`); issue draft 9 of `docs/notes/es3-shader-limits.md`.

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

## The number reported: 15, not Mesa's 16

The design note asked for Mesa's 16 / 64 / 64 / 64, with 15 / 60 / 60 / 64 as
its fallback. 15 is what was reported, for three reasons:

- It is ES 3.0's own reading, where 64 vertex outputs = 15 user vectors + the
  position vector and 60 varying components = the same fifteen, so
  `MAX_VARYING_VECTORS`, `MAX_VARYING_COMPONENTS` and
  `MAX_FRAGMENT_INPUT_COMPONENTS` stay one product (`context.rs:261-262`) and
  the shader's `gl_MaxFragmentInputVectors` is 15 beside
  `MAX_VARYING_VECTORS` 15 rather than 16 beside 15.
- 15 vectors are 65 VPM output rows (nine sectors). 16 need 4 + 1 + 64 = 69
  rows, which `vpm_fields` accepts (the record's field reaches 15 sectors) but
  no measurement of ours covers, and 64 *user* components beside the 64 the GL
  names.
- Everything that carries the 15 was testable on the host: `chain.rs`
  `varyings_tests.rs` (the compiler and the interpreter),
  `tests/es3_v3d_varyings.rs` (link, the C job builder's packets, and the three
  query forms), and `tools/v3d/job_check.c` part 11b, which reads a 60-input
  job back through Mesa 26.2.1's own decoder in the gate.

16 remains a board question (`docs/notes/es3-shader-limits.md` issue draft 6),
not a constant edit: `a_vs_writes_16_vec4_outputs` in `varyings_tests.rs` stays
`#[ignore]`d naming `VpmRowOutOfRange`, and `sixteen_varying_vectors_do_not_link`
in `tests/es3_v3d_varyings.rs` pins the refusal the GL gives today.

## Open

- A Pi 4 board check, a step for #320: gl_test 97 (`varyings15`); a program of
  60 inputs with a flat vec4 at inputs 24..27 and a centroid one at 48..51 (the
  flag words 1 and 2); a vertex shader of 65 output rows; the same with
  `gl_PointCoord`, where `num_inputs` of the shader record carries two implicit
  point varyings ahead of the declared ones. Nothing here has run on a board.
- Whether the hardware takes Mesa's 16 vectors -- 69 VPM output rows, and a
  `num_inputs` of 64 declared plus two implicit -- is the same run.
- The ES 3.00 link does not check varying component counts
  (`reflect.rs`); with 64 inputs a shader past the limit should fail at link,
  as dEQP expects (issue draft 9).
- `tools/shaders/known-uncompilable-v3d.txt` carries the corpus pairs whose
  limits differ between the `report-es2` and the ES 3.0 build.
