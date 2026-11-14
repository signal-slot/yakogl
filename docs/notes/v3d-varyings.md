<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D varyings: 15 vectors and 64 vertex output components

Plan #359, item `v3d-varyings-note`, from the read-only design
`design-v3d-varyings`; the raise itself is the section "What the raise
carries" (it replaces the two change lists the design had). Line numbers of
the first sections are at the integration tip `2aee4c92` (the `src/` and
`driver/` files may have moved by a few lines since);
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

The V3D build answers `MAX_VARYING_VECTORS` 15, `MAX_VARYING_COMPONENTS`
and `MAX_FRAGMENT_INPUT_COMPONENTS` 60 and `MAX_VERTEX_OUTPUT_COMPONENTS`
64: ES 3.0's minimums, not Mesa's 16 vectors. Three reasons, each measured
on the host or read in the code:

- A fragment shader that reads 64 input components does not compile here:
  `select_fragment` emits every `ldvary` before the body ("The varyings
  first, in input order"), so 64 values are live at once, and the
  allocator refuses with "more live values than 2 threads have registers
  (no spilling yet)" (`varyings_tests.rs`
  `a_vs_with_16_vec4_outputs_feeds_15`). 60 fit.
- A program reading `gl_PointCoord` takes two implicit varyings ahead of
  the declared ones; 60 + 2 stays within the 64 inputs the hardware
  iterates, and whether 64 + 2 does is a board question (below).
- No board has drawn more than a few varyings; 15 is what the conformance
  run needs.

`MAX_VERTEX_OUTPUT_COMPONENTS` is 64 rather than 60 because `gl_Position`
is not one of the V3D's varyings: its four fixed-function rows come before
them in the VPM (above), so a vertex shader with 16 vec4 outputs links with
a fragment shader reading 15 of them. `src/gl/context.rs` splits the
constant for `gpu-v3d` without `report-es2`; the PowerVR, GC7000 and Mali
keep `MAX_VARYING_VECTORS * 4`. Under `report-es2` the V3D stays at 8,
which `tests/es2_report_guard.rs` pins (16 passed on the V3D's ES 2.0
build).

## What the raise carries

The design above planned three flag words per kind through every struct
the fragment program crosses. That moves `struct shc_v3d_fs`
(`include/gles_shaderc.h`), hence `struct shc_v3d` and `struct shc_shader`
on a V3D build and every struct that embeds them (the generated
`src/abi_layout.rs` and `src/abi_layout_check.c`), `struct v3d_fs_prog`
(`driver/src/v3d/v3d_frame.h`, `src/v3d/ffi.rs` and its pins), and the
tools and tests that mirror them (`tools/v3d/pack_seq.h`,
`v3d_prog_header.py`, `sim_fixture.py`, `tests/v3d_sim_mesa.rs`,
`tests/es3_flatshade_guard.rs`). The raise does not need it: of ES 3.0's
60 inputs only the flat and centroid ones need a flags bit, and only a
program with more than 24 of them needs a second word (what that costs is
below). So every ABI byte stays as it is, and the raise is:

1. **One flags word, the qualified inputs first.** `chain.rs`
   `compile_program`: the flags are word 0 of Mesa's arrays, inputs 0-23
   (`FLAGS_WORD_INPUTS`). When a flat or centroid input would land past
   23, the input list is reordered with the flat and centroid inputs first,
   each group in its own order; `ldvary` takes the inputs in the list's
   order and the vertex shader writes them in the same order (`wanted`),
   so both stages follow. A program whose qualified inputs all lie below 24
   keeps Mesa's order, which is every program the comparison fixtures hold.
   More than 24 flat and centroid components are refused by name ("more
   than 24 flat and centroid varying components"), never drawn smooth.
2. **64 fragment inputs.** `chain.rs` refuses more than `FS_MAX_INPUTS` (64,
   Mesa's `V3D_MAX_FS_INPUTS`) by name, before selection.
3. **69 VPM output rows.** `select.rs` `VPM_MAX_OUTPUT_ROWS`: x, y, z, 1/w,
   the point size and 64 varyings, Mesa's largest layout (the cap was 64,
   which refused 15 vec4 with `gl_PointSize`). `vpm_fields` holds it on the
   64 KiB VPM (9 sectors a batch, seven batches).
4. **The job builder.** `v3d_job.c` `draw_is_supported` takes up to
   `V3D_MAX_FS_INPUTS` (`v3d_frame.h`; it took 24) and refuses a flag bit
   past 23 instead of dropping it (the packet field is 24 bits);
   `EMIT_VARYING_FLAGS` is unchanged, since word 0 with the higher words
   zeroed is Mesa's emission for this case (`v3dx_emit.c:177-208`).
   `v3d_frame.c` (the M5 frame) takes the same input count.
5. **The GL core.** `context.rs` `MAX_VARYING_VECTORS` and
   `MAX_VERTEX_OUTPUT_COMPONENTS` (above); `state.rs` and the shader's
   `gl_Max*` constants read them. The ES 1.00 link's Appendix A packing
   packs against 15.

Tests: `src/shaderc/online/v3d/varyings_tests.rs` (the compiler and the
interpreter: 15 vectors, 16 vec4 outputs feeding 15, the point size row,
the reorder with the flat values reaching the colour, the two refusals),
`tests/es3_v3d_varyings.rs` (the GL core, the real back end and the real
job builder: 60-input draws of GLSL ES 3.00 and 1.00, the decoded
`FLAT_SHADE_FLAGS` of a late flat varying, the link refusals),
`tools/v3d/job_check.c` part 2 (the job builder's refusals and a 60-input
draw), and the pins in `tests/es3_caps.rs`, `tests/es3_limits_v3d.rs` and
`src/gl/state.rs`.

What the one word costs, measured on the host (`tests/es3_shaderc_infolog.rs`
on `gpu-v3d`, VK-GL-CTS 93bca01's transform-feedback programs): 85 of them
have more than 24 flat or centroid components and are refused at link by
name (the `array.*` and `array_element.*` `ivec4`/`uvec4` cases, 54; 30
`random*`; `extra.mixed.centroid_mat_flat_int`). 26 of them were refused
before too ("a flat varying past input 31"); the other 59 linked, and the
job builder refused every draw of them (more than 24 inputs): on the Pi 4
that was a ResourceError, which ended the TF1 image's run at
`array.interleaved.lines.highp_ivec4`. A link refusal is a Fail and the run
goes on (the test's totals: 1190 linked and 134 refused before, 1131 and
193 now). The test's V3D floor of the 600 random programs that link
through the GL API moves from 504 to 459: 45 with a flat or centroid input
past 23, refused by the flags word or, once their flat inputs move first,
by the allocator (no spilling yet).

Host dEQP-GLES3 `transform_feedback.*` on `GPU=v3d` (HOST-ONLY, 882
cases, main `40539acd` against this change): Pass 740 -> 849, Fail
130 -> 2, NotSupported 12 -> 31. 106 Fail -> Pass and 3 NotSupported ->
Pass are programs of 9 to 15 vectors that the old 8-vector limit and the
24-input job builder refused; 22 Fail -> NotSupported are array captures
the flags word now refuses at link, which dEQP reports as "Capturing
arrays is not supported" (es3fTransformFeedbackTests.cpp:1033-1036). The
capture runs on the CPU (`gl/tf_capture.rs`), so these are the host's own
verdicts on the captured buffers, not a board result.

The three-word design stays the follow-up for a program with more than 24
flat or centroid components (an issue draft of plan #359): the flags
become `[u32; 3]` in `FsProg`, `shc_v3d_fs` and `v3d_fs_prog`, the pins are
regenerated (`yakogl-tools abi-layout`), and `EMIT_VARYING_FLAGS` becomes
the port of `emit_varying_flags` described above.

## The query forms

Section G3 of `docs/notes/v3d-notpass-rest.md` suspected a separate defect
in the float and 64-bit query paths, because the board's
`state_query.integers.*_getfloat` / `*_getinteger64` rows failed while no
`*_getinteger` row did. There is no such defect. For an implementation
limit dEQP makes only the boolean, int64 and float forms in
`state_query.integers` and the boolean, integer and float forms in
`state_query.integers64` (es3fIntegerStateQueryTests.cpp, "implementation
defined limits have their own tests so just check the conversions", at
VK-GL-CTS 93bca01): the integer form is the `implementation_limits` row.
Each form checks the value against the minimum, so all of them fail with a
value below it and pass with one that meets it. `glGetFloatv` and
`glGetInteger64v` answer the `glGetIntegerv` value converted
(`src/gl/state.rs`); `tests/es3_limits_v3d.rs`
`every_g3_pname_answers_the_same_in_all_three_integer_forms` and
`tests/es3_caps.rs` pin that. The AM62P's `max_varying_vectors_getfloat`
Fail is the same: that run (ad2b5ed, 2026-10-13) predates the PowerVR's
15 (b9a5a673, 2026-11-09).

## Open, no owner yet

- The ES 3.00 link does not check varying component counts
  (`reflect.rs:1068-1070`): a GLSL ES 3.00 pair past 60 is refused by the
  code generator (above 64 inputs, or by the register allocator), not by
  the link's own rule. dEQP's negative varying cases would see a link
  failure either way.
- `tools/shaders/known-uncompilable-v3d.txt` lost its `varyings15` entry:
  the pair links under ES 3.0. `tests/v3d_online_corpus.rs` is not run in a
  `report-es2` build (it refuses every GLSL ES 3.00 source there), so the
  ES 2.0 build's refusal of 15 vectors has no entry.
- A Pi 4 board check, a step for #320: gl_test case 97 (`varyings15`)
  now runs on the V3D instead of skipping, and dEQP-GLES3
  `shaders.linkage.varying.*` with 15 vec4, with `gl_PointCoord` beside
  them, and with flat inputs past input 23. Nothing here has run on the
  board.
