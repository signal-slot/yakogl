<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface and uniform limits: what each GPU can honestly report

Plan #359, items 1-4. Rows: dEQP-GLES3 `implementation_limits`
(`max_varying_vectors`, `max_varying_components`,
`max_fragment_input_components`, `max_vertex_output_components`,
`max_uniform_block_size`, `max_combined_{vertex,fragment}_uniform_components`)
and `state_query.integers*` (the `*_getfloat` / `*_getinteger64` forms of the
same). ES 3.0 minimums: 15 / 60 / 60 / 64, 16384, 50176 / 50048.

Status of each limit after this change (`tests/es3_caps.rs` `known_gaps()`
is the machine-checked list):

| Limit | PowerVR | GC7000UL | V3D | Mali |
|---|---|---|---|---|
| MAX_VARYING_VECTORS / _COMPONENTS / FRAGMENT_INPUT | 15 / 60 / 60 | 15 / 60 / 60 | **15 / 60 / 60** (was 8 / 32 / 32) | 8 / 32 / 32 (stage 5 owns it) |
| MAX_VERTEX_OUTPUT_COMPONENTS | 60 | 60 | **64** (was 32) | 32 |
| MAX_UNIFORM_BLOCK_SIZE | 6144 | 6144 | 6144 | 6144 |
| MAX_COMBINED_*_UNIFORM_COMPONENTS | 1536 | 1536 | 1536 | 1536 |

## 1. The float and 64-bit query forms: no defect of their own

`docs/notes/v3d-notpass-rest.md` G3 suspected a separate defect in
`get_floatv` / `get_integer64v`. There is none. `glGetFloatv` and
`glGetInteger64v` answer an integer pname by calling `get_integerv` and
widening (`src/gl/state.rs` `glGetFloatv`, `glGetInteger64v`, with the
unsigned widening of the four stencil masks), so a `*_getfloat` row fails
exactly when the integer value is below the minimum. Nothing in the host build
answers one form differently from another (read to the line, and pinned
below). What the host cannot explain: the Pi 4 tsv lists `*_getfloat` and
`*_getinteger64` rows of these limits as Fail and no `*_getinteger` row of
them (only `compressed_texture_formats_getinteger`), although the integer
values are below the minimum too. The dEQP source was not read here, so
whether those rows are not in the group, or passed on the board, is open; it
is a question for the board rerun (issue draft 6), not for the GL core.
Pinned: `tests/es3_limits_v3d.rs` `every_g3_pname_answers_the_same_in_all_three_integer_forms` (all GPUs'
pnames) and `tests/es3_v3d_varyings.rs`
`the_four_limits_read_15_60_60_64_in_every_query_form` (the raised ones,
integer, float and 64-bit). The Pi 4 tsv is from 2026-10-31 and predates the
conversion path; the board rerun confirms.

## 2. The V3D varyings (done)

What a fragment input is on the V3D: one scalar per `ldvary`
(`num_inputs` = 4 per vec4, plus 2 for the implicit point varyings, plus 1
for the primitive id). 15 vec4 are 60 inputs, Mesa's `V3D_MAX_FS_INPUTS` is
64 (`v3d_limits.h`).

What was below 60, bottom up:

1. **The shader record's flag words.** `FLAT_SHADE_FLAGS`,
   `NON_PERSPECTIVE_FLAGS` and `CENTROID_FLAGS` carry 24 inputs a packet
   (`*_flags_for_varyings_v024`, 24 bits) at `varying_offset_v0` words;
   Mesa keeps `flat_shade_flags[((V3D_MAX_FS_INPUTS - 1) / 24) + 1]`, three
   words, and `emit_varying_flags` writes one packet per non-zero word (the
   first with the lower words unchanged for word 0 and zeroed otherwise, the
   higher ones zeroed; each later with both unchanged; `ZERO_ALL_*` when all
   are zero). This repo kept one `u32` of 32 bits: bits 24..31 were silently
   dropped by the packet, and `draw_is_supported` refused more than 24
   inputs, so a program of eight vec4 read 32 inputs and was refused by the
   job builder. Now `ShcV3dFs`, `V3dFsProg` and `struct v3d_fs_prog` carry
   word 0 in the old fields and words 1-2 in `*_flags_hi[2]` (appended, so no
   offset moved; `abi_layout.rs` regenerated), the compiler splits at 24
   (`online/v3d/chain.rs` `FLAG_WORDS`, `select_fragment` indexes the words),
   and `v3d_job.c` `EMIT_VARYING_FLAGS` follows Mesa's loop. The job builder
   accepts 64 inputs.
2. **The VPM output rows.** A vertex shader stores one row per output
   component after four fixed-function rows, plus one for a point size.
   `select.rs` refused more than 64 rows; 15 vec4 and a point size are 65.
   Now 72 (nine 8-row sectors, `VPM_OUTPUT_ROWS_MAX`; the record's output size
   is in sectors). The VPM holds 128 sectors, `vpm_fields` still refuses a
   program that leaves fewer than two batches.
3. **64 vertex output components.** The chain writes only the vertex
   outputs the fragment shader reads (`chain.rs` `wanted`), so a vertex shader
   declaring 16 vec4 beside a fragment shader reading 15 takes no row for the
   16th. That is all `MAX_VERTEX_OUTPUT_COMPONENTS` 64 asks of the compiler;
   the limit is 64 and the fragment side 60, as in Mesa's GL (a program may
   write more than the fragment stage reads, but not link with a fragment
   shader reading 16). The only consumer of an unread 16th output would be
   transform feedback, which runs on the CPU here (`gl/tf_capture.rs`) and is
   independent of the VPM layout.
4. **GL core.** `context.rs` `MAX_VARYING_VECTORS` 15 and
   `MAX_VERTEX_OUTPUT_COMPONENTS` 64 under `gpu-v3d`, `not(report-es2)`. An
   ES 1.00 program of 16 vec4 is refused by the Appendix A budget by the limit's
   name (the budget reads the constant); an ES 3.00 pair is not budget-checked
   on any GPU (`reflect::varying_budget`), so a 16-vec4 ES 3.00 fragment
   shader reaches the compiler and is refused by register pressure
   ("more live values than 2 threads have registers"): a refusal, with a
   misleading text. Left, see the issue drafts.

Tests: `src/shaderc/online/v3d/varyings_tests.rs` (compiler and lane
interpreter: 15 vec4 carried value for value, flags across three words, a
point size, 16 vertex outputs beside 15 inputs), `tests/es3_v3d_varyings.rs`
(the GL core and the real job builder: limits in the three query forms, a
15-vec4 draw with the flag packets decoded, a point size, 16 vertex outputs,
the ES 1.00 refusal). What the host cannot show: the hardware's
interpolation of 60 inputs. That is the Pi 4's.

## 3. Uniform blocks (design; not raised)

Today a uniform block is flattened into its stage's default block: the
shader reads block members as ordinary uniforms at dword offsets, and the
draw copies each bound range into the stage's array of `MAX_UNIFORM_DWORDS`
(1536) dwords (`gl/program.rs` `block_dwords`, `gl/draw.rs`). The *sum* of
the default block and every block of a stage is capped at 1536 dwords. So
6144 bytes for `MAX_UNIFORM_BLOCK_SIZE` and 1536 for the combined limits are
both honest for this design; the ES 3.0 formula
(`blocks * block size / 4 + default components` = 12 * 4096 + 1024 = 50176 and
12 * 4096 + 896 = 50048) holds only when block storage is separate from the
default block. Raising either number alone would report capacity a program
cannot use. What each GPU needs for 16384 bytes a block:

- **GC7000UL**: the shader already reads user blocks from memory with `load`
  from `SHC_VIV_CONST_UBO_ADDR` (`docs/notes/gc-ubo-const-limit.md`, section
  7), so the hardware and compiler carry a block of any size that the GL core
  keeps in GPU memory. What is missing is in the GL core: a block region per
  stage separate from the 1536-dword default block, bound per draw (draw.rs
  allocates `MAX_UNIFORM_DWORDS * 4 * 2 + 16` bytes a draw today), and the
  link rule that a block larger than the cap is an error. Then 16384 and the
  combined 50176 / 50048 are reportable. Owner: `src/vivante/**` (#335,
  #355) and `src/gl/{program,draw}.rs` (#343).
- **V3D**: the uniform stream carries one word per *read* of a uniform
  (`QUNIFORM_UNIFORM`, `data` the dword offset), so a block's size costs
  nothing in the stream; the cap is the CPU-side block array again. Two
  more things: a run-time index into an array of more than 256 elements is
  refused ("no ldunifa yet", `online/v3d/prepare.rs` `LoadUniformIdx`), and a
  block read at an index needs Mesa's `UNIFA` / `QUNIFORM_UBO_ADDR` path
  (a TMU general load of the block's memory, address from the stream). A
  16384-byte block with a dynamically indexed array, which is what the dEQP
  `ubo` cases do, needs that path. Order: the separate block region in the
  GL core (shared with the GC), then `ldunifa` in the V3D compiler, then the
  limit.
- **PowerVR**: the constant store is the USC's shared registers loaded by the
  PDS, bounded by the program's shared-register allocation; whether a 16384
  byte block can be read from memory in the USC (Mesa's pvr does so with
  loads from the UBO address in the descriptor) is [U]: not read here, and
  `src/pvr/**` belongs to #350 and #325.
- **Mali**: stage 5's.

The combined limits are computed once, in the GL core, from the three
numbers the spec names (`state.rs` should read
`MAX_x_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4 + MAX_x_UNIFORM_COMPONENTS`
instead of the bare dwords), and `tests/es3_caps.rs` already asserts that
relation on the reported values.

## 4. MAX_VERTEX_OUTPUT_COMPONENTS 64 on the PowerVR and GC7000UL (not changed)

Both report 60 = 15 * 4. The vertex stage's output rows are the limit:
GC7000UL has 16 io rows with one the position (`SHC_VIV_MAX_IO`, so 15
reach the fragment stage); the PowerVR's vertex output size is 8 bits in
units of 4 dwords (64 dwords at most carries 60 components plus the
position's 4). Whether a 16th unread output can be dropped as the V3D chain
does depends on whether each compiler writes only the outputs the fragment
stage reads; not read here for either, [U]. `src/pvr/**` and
`src/vivante/**` are other plans'.

## 5. Issue drafts

(Agents never run `gh`; these are the texts to file.)

1. **ES 3.0 uniform blocks: separate block storage from the default block**
   (GL core, `src/gl/{program,draw,state}.rs`; after #343). Body: section 3 of
   `docs/notes/es3-limits-design.md`. Done when a program with a block of
   16384 bytes links and draws on the GC7000UL on the host, `state.rs` derives
   the combined limits by the spec formula, `tests/es3_caps.rs` and
   `tests/vivante_imx8mp_limits.rs` move `MAX_UNIFORM_BLOCK_SIZE` and the two
   combined components out of `known_gaps`.
2. **V3D: `ldunifa` (QUNIFORM_UBO_ADDR) for uniform arrays read at a
   run-time index over 256 elements** (`src/shaderc/online/v3d`,
   `src/v3d`). Needed for the dEQP `ubo` cases with large arrays and for
   issue 1 on the V3D.
3. **PowerVR: can a 16384-byte uniform block be read from memory?** Read
   Mesa's pvr UBO path and the USC load instructions
   (`src/pvr/**`, #350 / #325 own it). Output: yes/no and the item list.
4. **PowerVR and GC7000UL: MAX_VERTEX_OUTPUT_COMPONENTS 64** (60 now). Decide
   from the compilers whether an unread 16th vertex output takes no row; if
   so raise `context.rs` `MAX_VERTEX_OUTPUT_COMPONENTS` for those GPUs and
   move the entry out of `known_gaps`.
5. **ES 3.00 link: refuse a program over MAX_VARYING_VECTORS by the limit's
   name** (`reflect::varying_budget` skips ES 3.00 pairs). On the V3D a
   16-vec4 ES 3.00 fragment shader is refused by register pressure with a
   text that does not name the limit. ES 3.0 §11.1.2.1 packing applies.
6. **Pi 4 board run of the V3D varying draws** (`tests/es3_v3d_varyings.rs`
   host-draws them; the hardware's interpolation of 60 inputs, flat and
   centroid across the three flag words, is the board's). The rows:
   `implementation_limits.max_varying_*`, `max_fragment_input_components`,
   `max_vertex_output_components`, `state_query.integers.*` of them and the
   `shaders.linkage` varying groups.
