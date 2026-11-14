<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface and uniform limits: what each GPU can honestly report

Plan #359 (roadmap stage 4). The limits below were under their ES 3.0 minimum
on every GPU (`known_gaps()` of `tests/es3_caps.rs`); this note says what each
one can be per GPU, what carries it, what was raised here, and what is left to
its owner. HOST-ONLY means measured on the host, never on a board; [U] marks a
hardware reading no measurement of ours backs yet. Line numbers are at this
commit; Mesa lines are Mesa 26.2.1 (`$MESA_FULL_SRC_26_2_1`: the vendored
`third_party/mesa-26.2.1` has no v3d driver or compiler sources).

## 1. The float and 64-bit query rows are not a separate defect

`docs/notes/v3d-notpass-rest.md` G3 suspected a defect in the float and 64-bit
query paths. There is none. `glGetFloatv` (`src/gl/state.rs:960`) reads the same
arms `glGetIntegerv` (`:1101`) does and converts their values, and
`glGetInteger64v` (`:1398`) widens the same number; the only difference is the
widenings of UNSIGNED state, where a signed widening would be wrong (the four
stencil masks, `is_unsigned_state` `:1015-1024`, pinned by
`the_stencil_masks_widen_as_unsigned`, and `get_floatv_converts_integer_state`).
So the three integer forms of a pname are one number, and a limit under its
minimum fails in all three.

The case lists say the same. `state_query.integers`
(`modules/gles3/functional/es3fIntegerStateQueryTests.cpp:2874`) registers its
minimum-value cases (`max_vertex_output_components` `:2941` with 64,
`max_varying_vectors` `:2963` with 15) through
`implementationLimitVerifiers = {Boolean, Integer64, Float}` (`:2990`, whose own
comment reads "implementation defined limits have their own tests so just check
the conversions to boolean, int64 and float") -- the integer form of the same
limit is the `implementation_limits` row. `state_query.integers64`
(`es3fInteger64StateQueryTests.cpp:277`) holds `max_uniform_block_size` 16384
(`:310`) through `{Boolean, Integer, Float}` (`:313-315`, "do not check the
values with integer64 verifier as that has already been checked in
implementation_limits"). That is why a limit showed `*_getfloat` and
`*_getinteger64` Fail rows and no `*_getinteger` row in one group and the other
pair in the next. Nothing was fixed in the query paths; the values moved.

`es3fInteger64StateQueryTests.cpp:246-252` computes the combined-component
minimum it checks from the limits *as reported*:
`maxUniformBlocks * uniformBlockSize / 4 + maxUniformComponents`, the ES 3.0
section 6.2 formula. A build that reported 12 blocks of 16384 bytes and 1536
default-block components passes the case whatever its storage really holds, so
the honest way to raise `MAX_COMBINED_UNIFORM_COMPONENTS` is to raise it with
the storage, which is what §3.2 does.

Pinned for every limit this plan moves: `tests/es3_v3d_varyings.rs`
(`the_shader_interface_limits_are_es3s_in_every_query_form`) and
`tests/es3_v3d_ubo_size.rs`
(`the_block_size_and_the_combined_components_meet_es3_in_every_query_form`)
ask each limit through `glGetIntegerv`, `glGetInteger64v` and `glGetFloatv`;
`tests/es3_limits_v3d.rs`
(`every_g3_pname_answers_the_same_in_all_three_integer_forms`) pins the whole
G3 list.

## 2. Per limit and GPU

| Limit | PowerVR BXS-4-64 | GC7000UL | V3D 4.2 | Mali-G610 |
|---|---|---|---|---|
| `MAX_VARYING_VECTORS` / `_COMPONENTS`, `MAX_FRAGMENT_INPUT_COMPONENTS` | 15 / 60 / 60 (already) | 15 / 60 / 60 (already) | **15 / 60 / 60** (was 8 / 32 / 32) | 8 / 32 / 32, unchanged (stage 5 plans) |
| `MAX_VERTEX_OUTPUT_COMPONENTS` | 60: 64 is [U], issue draft 3 | 60: 64 is [U], issue draft 4 | **64** (was 32) | 32, unchanged |
| `MAX_UNIFORM_BLOCK_SIZE` | 6144: issue draft 1 | 6144: issue draft 2 | **16384** (was 6144) | 6144, unchanged |
| `MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS` | 1536: issue draft 1 | 1536: issue draft 2 | **50688** (was 1536) | 1536, unchanged |

`report-es2` keeps every answer it had (`tests/es2_report_guard.rs`,
`es2_report_answers_are_unchanged`): the V3D's `MAX_VARYING_VECTORS` is 8 there
(`src/gl/context.rs:209-210`), and the ES 3.0-only pnames are
`GL_INVALID_ENUM`.

The four component limits move with `MAX_VARYING_VECTORS`:
`MAX_VERTEX_OUTPUT_COMPONENTS = MAX_VARYING_VECTORS * 4 + VERTEX_OUTPUT_EXTRA`
and `MAX_FRAGMENT_INPUT_COMPONENTS = MAX_VARYING_VECTORS * 4`
(`context.rs:261-262`), and `GL_MAX_VARYING_COMPONENTS` reads the same product
(`state.rs:1254`). The shader sees them divided by four
(`gl_MaxVertexOutputVectors`, `gl_MaxFragmentInputVectors`,
`src/shaderc/online/lower.rs:12883-12884`), so raising one limit moves the
shader's with it.

## 3. The V3D

### 3.1 Varyings: 15 vectors, 60 inputs, 64 vertex outputs

What carries sixty fragment inputs, beside Mesa's v3d (which takes 64 inputs
and reports 16 vectors, `v3d_limits.h:32-35` `V3D_MAX_FS_INPUTS`,
`v3d_screen.c:323`):

- **The compiler.** Each used component is one `ldvary` input, in `ldvary`
  order (`src/shaderc/online/v3d/chain.rs` `fragment_inputs`), and a program
  reading more than `MAX_FS_INPUTS` (64, `chain.rs:221`) of them fails with
  `TooManyFragmentInputs` (`chain.rs:444-446`) rather than with the old
  `NotYet`. The vertex shader writes the same components into the VPM after its
  header, in the same order (`wanted`). The corpus pair `es100/varyings15` (60
  components, gl_test 97) already built its fragment shader at 2 threads
  (`chain.rs:1736`); what refused it was the GL limit and, behind it, the
  vertex shader's VPM row cap.
- **The VPM row cap.** `select.rs:438` `MAX_VPM_OUTPUT_ROWS` is
  `4 + 1 + 60`: the four fixed-function rows (x, y, z, 1/w), the point size
  row, and the sixty varying components; `select_vertex` refuses more with
  `VpmRowOutOfRange` (`select.rs:2572-2573`). The shader record's output
  segment size is 4 bits of 8-row sectors, so `vpm_fields` (`chain.rs:342-366`)
  refuses an output segment past 15 sectors (120 rows) beside its two-batch
  rule (Mesa `v3d_vs_set_prog_data`, `vir.c:896-937`): 65 rows are nine
  sectors, and with the 64 KiB VPM `Target { vpm_size: 65536 }` there are
  enough batches. 15 is not the hardware's ceiling; it is ES 3.0's.
- **The record and the job builder.** `number_of_varyings_in_fragment_shader`
  is 8 bits (`driver/src/v3d/v3d_frame.c:701`, `v3d_job.c:897`); the job
  builder takes up to 64 inputs (`v3d_job.c:827`) and refuses a flag bit at or
  past 24 *within a word* of any kind (`v3d_job.c:829-834`), which is what the
  24-bit packet field holds.
- **The flag words.** `FLAT_SHADE_FLAGS`, `NON_PERSPECTIVE_FLAGS` and
  `CENTROID_FLAGS` carry 24 flags a packet, at a 4-bit word offset
  (`varying_offset_v0`; `v3d_packet.xml`, Mesa `emit_varying_flags`
  `v3dx_emit.c:177-208`). The record and the program carried word 0 only. Word
  0 stays `flat_shade_flags` / `noperspective_flags` / `centroid_flags` and
  words 1 and 2 are their `*_hi[2]` arrays (`include/gles_shaderc.h:253,259-260`
  `shc_v3d_fs`, `src/shaderc/mod.rs` `ShcV3dFs`, `src/abi_layout.rs` pins at 24
  and 48; C `struct v3d_fs_prog` `v3d_frame.h:174-186`, its Rust mirror
  `V3dFsProg` `src/v3d/ffi.rs` -- 80 bytes, the `_hi` arrays at 52, 60 and 68,
  pinned in `src/v3d/ffi_layout_check.c`), filled at `src/v3d/backend.rs`.
  `v3d_job.c` `EMIT_VARYING_FLAGS` (`:504-525`) is the port of Mesa's rule --
  one packet per nonzero word, the first one zeroing the words beside it that
  are not sent (the lower ones too unless it is word 0, the higher ones
  always), the later ones changing nothing, `ZERO_ALL_*` when a kind has none
  (`varying_flag_packets` `:529-533` counts them for the group packing). The
  M5 triangle builder (`v3d_frame.c`) still takes word 0 and 24 inputs only and
  refuses a program whose `_hi` word is nonzero (`v3d_frame.c:597-601`).
  The compiler used to set bit `i` for `i` up to 31 into the 24-bit field and
  refuse past 31: a flat varying at input 24..31 was silently truncated by the
  packer. `flag_words` now packs the 64-bit sets by position
  (`src/shaderc/online/v3d/select.rs`), pinned by
  `flat_and_centroid_inputs_past_24_set_the_high_words` in
  `src/shaderc/online/v3d/varyings_tests.rs`.
- **The interpreter.** `varyings_tests.rs` runs these programs through
  `sim.rs`: `a_vs_writes_15_vec4_varyings` and `a_fs_reads_15_vec4_varyings`
  (the rows and the inputs of fifteen vec4), `an_es300_pair_carries_15_vec4_varyings`
  (a VS and an FS over the pair), `flat_centroid_and_smooth_varyings_set_their_flags`,
  `scalars_and_vec2s_pack_into_60_components`, and the flag words above. An
  input at the wrong index or a flat read from the wrong register shows up as a
  wrong component there.
- **The host test.** `tests/es3_v3d_varyings.rs` builds the real C job builder
  as a shared object, dlopens it, records `v3d_job_draw`, and decodes the
  `FLATSHADE_FLAGS` / `NON_PERSPECTIVE_FLAGS` / `CENTROID_FLAGS` packets it
  wrote: `fifteen_smooth_vec4_draw_with_sixty_inputs`,
  `a_flat_vec4_past_input_48_goes_in_the_third_word`,
  `a_centroid_vec4_past_input_48_goes_in_the_third_word`,
  `flat_inputs_straddling_the_first_word_boundary`,
  `flags_in_all_three_words_are_three_packets`. It links what must not link:
  `sixteen_varying_vectors_do_not_link`, and
  `sixteen_vertex_outputs_link_when_the_fragment_shader_reads_fifteen` (15 user
  vectors plus `gl_Position` is ES 3.0's 64 output components).
- **The job builder against Mesa.** `tools/v3d/job_check.c` part 11b
  (`part_varying_words`, `:1661-1668`) draws a 60-input program and reads the
  packets back with Mesa 26.2.1's own decoder: flat flags in words 0 and 1,
  centroid flags in word 2. It runs in the gate (`job_check`, `cl_golden`,
  `tex_check` rows).

**15, not Mesa's 16.** ES 3.0 counts `gl_Position` among the vertex outputs:
`MAX_VERTEX_OUTPUT_COMPONENTS` 64 = 15 user vectors + the position vector, and
its 60 varying and 60 fragment-input components are the same fifteen. Mesa
answers 16 vectors for all four limits (`v3d_screen.c:323`), which on this
pipeline means 4 + 1 + 64 = 69 VPM output rows, 64 *user* components past the
64 the GL names, and `gl_MaxFragmentInputVectors` 16 beside
`MAX_VARYING_VECTORS` 16 -- numbers no measurement of ours covers. 15 is the
minimum, is what the nine sectors and the 64 components the record's count
covers, and keeps `VERTEX_OUTPUT_EXTRA` honest as "the V3D writes gl_Position
beside the varyings" (`context.rs:244-254`, `docs/notes/v3d-varyings.md`).
Going to 16 is a board question (issue draft 6), not a constant edit.

`gl_PointCoord` takes two implicit varyings ahead of the declared ones, for
points only (`src/v3d/backend.rs`, `chain.rs` `uses_implicit_point_line_varyings`):
with 60 declared inputs a points program is at 62 of the 64. Measured on the
host in `job_check` part 11 (`:1612-1635`), not on a board.

Not measured on a board: a draw with sixty inputs and flags in words 1 and 2.
The host shows the compiler, the records, the packets (against Mesa's decoder)
and the interpreter, not what the QPU and the varying interpolator do with a
flag in word 2. `docs/notes/v3d-varyings.md` keeps the board checks.

A program of up to 60 inputs can need more registers than a 2-thread fragment
shader has: `more live values than 2 threads have registers (no spilling yet)`
(`chain.rs:245` `DOES_NOT_FIT`), which the dEQP
`random.interleaved.triangles.173` program hits (`tests/es3_shaderc_infolog.rs`).
That is the register allocator, not the limit (issue draft 5): those programs
failed at link before the raise too, where they now fail a little later, so no
dEQP row moves from Pass.

### 3.2 Uniform blocks: 16384 bytes, 50688 combined components

A block is flattened into the stage's uniform storage and filled from the bound
range at the draw (`src/gl/program.rs`, `src/gl/draw.rs`). The V3D compiler
addresses a block member as a word of that storage -- a
`QUNIFORM_UNIFORM` stream entry with the dword index (`online/v3d/prepare.rs`
`SOp::Uniform(u16)`) -- and the job builder writes the current value of each
word the shader reads into the uniform stream (`driver/src/v3d/v3d_job.c`
`write_uniforms`): the cost is the number of reads, not the block's size. So
the block size and the count of blocks are a GL-core budget here, and the
budget is what the note's issue drafts are about on the other GPUs.

- `MAX_STAGE_UNIFORM_BLOCKS` 12 and `MAX_UNIFORM_BLOCK_SIZE` 16384 on the V3D,
  `MAX_UNIFORM_DWORDS * 4` = 6144 elsewhere (`context.rs:93-113`).
- `MAX_STAGE_UNIFORM_DWORDS` (`context.rs:119-124`) is a stage's storage: the
  default block's `MAX_UNIFORM_DWORDS` (1536) plus 12 blocks of 4096 dwords =
  50688 on the V3D, and `MAX_UNIFORM_DWORDS` unchanged on a GPU where a block
  is no larger than the default block. A `const assert!` holds it inside
  `u16::MAX`, because `GlProgram` sizes blocks in u16 dwords.
- `MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS` (`context.rs:130`) is ES
  3.0 section 6.2's formula, `blocks * block_size / 4 + default-block
  components` = 12 x 4096 + 1536 = 50688, above the minimums 50176 and 50048.
  It is true because the stage's storage is that large.
- A program's arrays are sized to its own layout: `block_dwords` caps at
  `MAX_STAGE_UNIFORM_DWORDS` (`program.rs:1861-1869`) and the link allocates
  the vertex and fragment arrays of that size (heap, `calloc`, beside the blend
  constants). The draw clamps a stage's dwords to the same constant
  (`draw.rs:3919-3923`).
- The GPU staging copy of the uniforms (`pg.uniforms`) holds two default blocks
  (`MAX_UNIFORM_DWORDS * 4` bytes) and the V3D never uses it
  (`uniforms_host_only: true`, `src/v3d/backend.rs`;
  `tests/vivante_imx8mp_limits.rs` pins it for the GC7000UL). A block of a GPU
  whose storage is larger does not fit it: `put_uniforms` (`draw.rs:947-954`)
  is `put` with a bounds check, so such a copy is refused (`GL_OUT_OF_MEMORY`)
  instead of writing past the end. Found by `tests/es3_v3d_ubo_size.rs` on a
  stand-in back end that staged: `malloc(): corrupted top size`. On the PowerVR
  and the GC7000UL the arithmetic keeps every copy inside the block -- two
  stages of 1536 dwords with the four blend constants -- so their behaviour is
  byte-identical (`tests/es3_caps.rs`, `tests/vivante_imx8mp_limits.rs`, and
  `the_es3_limits_are_what_this_gpu_carries_and_programs` in `state.rs` run on
  both).

Measured HOST-ONLY: `tests/es3_v3d_ubo_size.rs` -- the three query forms
(plus ES 3.0's formula as an arithmetic check on the reported pair), a block of
sixteen `vec4 a{j}[64]` members (4096 dwords, one dynamic index) linked and
drawn with the whole 16384 bytes present in the storage the back end is handed,
and a 6048-byte block drawn as before. `tests/v3d_ubo_large_sim.rs` is the
compiler-side half: a fragment shader reading `vec4` members up to dword 4095
of a block by constant index, and by an index held in a uniform, run in the QPU
interpreter with a stream built from a block whose dword `k` holds `k + 0.5`.

What the raise does **not** buy is a long dynamically indexed array:
`Op::LoadUniformIdx` selects among at most `MAX_SELECTED = 256` components
(`prepare.rs:1394-1396` -- no `ldunifa` yet, issue draft 8), and
`UniformRow::array_len` is a `u8` (`online/reflect.rs`, `fit_u8` `:1482`), so a
`vec4 a[1024]` is refused as "too many of this kind of declaration" and a block
of that size is written as several arrays. The three `#[ignore]`d cases of
`tests/v3d_ubo_large_sim.rs` name exactly those two limits. dEQP's blocks are
not that large.

### 3.3 Not changed on the V3D

`MAX_VERTEX/FRAGMENT_UNIFORM_COMPONENTS` stay 1536: that is the default block's
budget, the link's Appendix A packing of the default-block uniforms (384 vec4,
`context.rs:83`). `MAX_UNIFORM_BUFFER_BINDINGS` 24 (`context.rs:271`) and the block counts
(`context.rs:93`, `program.rs:69`) were already at their minimums.

## 4. PowerVR and GC7000UL: nothing changed, and what each raise needs

Their owners hold `src/pvr/**` (#350, #325) and `src/vivante/**` (#335, #355);
nothing there was touched. The GL core's constants are shared, and on these two
the raise is inert: with `MAX_UNIFORM_BLOCK_SIZE` at `MAX_UNIFORM_DWORDS * 4`,
`MAX_STAGE_UNIFORM_DWORDS` equals `MAX_UNIFORM_DWORDS`, so every cap, query and
allocation of those builds is the byte it was. Their varyings were already 15 /
60 / 60 (`context.rs:163-188`, `tests/es3_pvr_varyings.rs` draws with fifteen),
and their `MAX_VERTEX_OUTPUT_COMPONENTS` is 60 because `VERTEX_OUTPUT_EXTRA` is
0 there: whether the 64th component is reachable is their owners' measurement,
not this change (issue drafts 3 and 4).

- **PowerVR uniform blocks.** The USC reads a uniform block out of the shared
  register file (`online/powervr/select.rs` `LoadUniform` is `Bank::Shared`,
  `reg = dword_off`), which a PDS DMA fills before the shader runs
  (`src/pvr/backend.rs`). 4096 dwords do not fit the shared file beside the
  sampler descriptors, so the GL-core cap is the default block. Reading a block
  of that size needs the memory-load path: the block's GPU address in shared
  registers and a global load per read, as Mesa's pco lowers `load_ubo`.
  Issue draft 1.
- **GC7000UL uniform blocks.** The constant file is 320 registers
  (`docs/notes/gc-ubo-const-limit.md`); Mesa etnaviv reads a user block from
  memory with `LOAD` (`etnaviv_nir_lower_ubo_to_uniform.c`) and keeps only block
  0 in the constants. Our code generator already has a load path
  (`SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`) for run-time indexes, proven on the
  i.MX8MP; the constant-offset load from a user block is not proven. Issue
  draft 2.

## 5. Host dEQP, before and after

`tools/cts/host-deqp-run.sh` (README 2b) with `MODULE=gles3` on the V3D build and
-- §5.4 -- on the PowerVR and the GC7000UL builds, against VK-GL-CTS `93bca01` with
the yakogl target (`tools/cts/cts-yakogl-target.patch`), on the rows RC10 and G3
name:
`functional.implementation_limits.`, `functional.state_query.integers*`,
`functional.shaders.linkage.` and `functional.ubo.` -- this commit and
`94b733a6`, the commit this change was cut from (the library is `dlopen`ed, so one
`deqp-gles3` serves both).

HOST-ONLY, and never a render verdict: the LINUX OSAL keeps the command stream,
so every image comparison here is meaningless (`tools/cts/README.md`). What is
real in these groups is the case outcome of a query, which is what RC10 is about.

### 5.1 The V3D arms

The case list is the mustpass union of the CTS's `android/cts/main/gles3-main-*.txt`
filtered to the rows this change reaches: 863 cases -- `implementation_limits.` 49,
`state_query.integers` 517 and `state_query.integers64` 15, the four struct subgroups
of `ubo.` (136), and every case of the list whose name contains `varying`
(`shaders.linkage.varying.` 75 and `shaders.indexing.varying_array.` 64, beside the
limit and query rows already counted). Each arm is its own library built with the
harness's own configure line (`-DYAKOGL_OSAL=linux -DYAKOGL_GPU=v3d
-DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=OFF -DYAKOGL_SHARED=ON`), one for this
commit and one for `94b733a6` checked out beside the run; the same `deqp-gles3`
serves both, because the yakogl target dlopens `libGLESv2.so.2` and `libEGL.so.1`
through `LD_LIBRARY_PATH`. The harness's chunk loop ran them (chunk 100, two
processes, 256x256 pbuffer, `YAKOGL_V3D_TILED_COPIES_BYTES=16777216`), and
`--compare-only` compared the two run directories. Every one of the 863 cases began
in each arm.

### 5.2 21 rows Fail to Pass, none the other way

| Group | Before | After | Moved by this change |
|---|---|---|---|
| `implementation_limits.` | 11 Fail | 4 Fail | 7: `max_varying_vectors`, `max_varying_components`, `max_fragment_input_components`, `max_vertex_output_components`, `max_uniform_block_size`, `max_combined_vertex_uniform_components`, `max_combined_fragment_uniform_components` |
| `state_query.integers.` | 17 Fail | 9 Fail | 8: the `*_getfloat` and `*_getinteger64` forms of the four varying limits |
| `state_query.integers64.` | 6 Fail | 0 | 6: the `*_getfloat` and `*_getinteger` forms of `max_uniform_block_size` and of the two combined-component rows |

The before arm counts 11, 17 and 6 in these three groups, which is G3's Pi 4 record
reproduced case for case: the 11 are `compressed_texture_formats`,
`num_compressed_texture_formats`, `max_color_attachments`, `max_draw_buffers`,
`max_uniform_block_size`, the two combined-component rows,
`max_vertex_output_components` and the three varying rows. RC10's 11 and 25 are the
AM62P's (`ad2b5ed`, 2026-10-14): its 11 are the same set -- the build it ran reported
8 varyings -- while the PowerVR build of this commit reports 15 and its host arm
fails 8, 11 and 6 of these groups (§5.4); its `state_query` 25 is `integers` 17,
`integers64` 6, and one `rbo` and one `shader` row each
(`docs/es3/am62p-gles3-notpass.tsv`; the 9 `state_query.indexed` rows belong to
another cluster). The same seven limit rows are what a build at 8 varyings fails,
board or host.

The failure messages are the value checks: `Got invalid float value` and `Got invalid
integer value` in the query groups, `Requirement not satisfied` in
`implementation_limits`. The remaining 13 rows of these 581 are `max_draw_buffers` and
`max_color_attachments` (RC14), `max_samples_getfloat` and `max_samples_getinteger64`
(RC8, `max_samples_getinteger` passes), and the compressed-format list (plan #354) --
§1's point, seen on the host: dEQP registers the minimum-value check of those rows
only for the boolean, 64-bit and float verifiers.

### 5.3 What these groups cannot show

The compile and link outcomes are the same in both arms: 539 stage compiles and 259
programs link OK in each, and the same 21 negative cases are refused in each
(`shaders.linkage.varying.rules.*`, `shaders.linkage.varying.usage.writeback_1`,
`shaders.keywords.reserved_keywords.varying_*`,
`shaders.constants.const_float_assign_varying`,
`shaders.uniform_block.invalid.member_varying_interface_qualifier_*`). Nothing new
was refused, and nothing new could pass, for a plain reason: these groups never build
a program at the raised limits. Across all 863 cases the largest number of `in`
declarations in any generated fragment shader is 4, and
`modules/gles3/functional/es3fUniformBlockTests.cpp` never reads
`GL_MAX_UNIFORM_BLOCK_SIZE` -- the ubo group sizes its blocks itself, so its 136
cases are identical in the two arms, and they fail on the image comparison, which
means nothing on this OSAL. So these runs pin the query rows of §2 only; the
15-varying programs and a 16384-byte block are pinned by the Rust tests of §3, and
they meet real hardware in the board run of issue draft 6.

### 5.4 The PowerVR and the GC7000UL arms: nothing moved, which is the point

The same 863 cases against the `gpu-powervr` build and against the `gpu-vivante`
one, each at this commit and at `94b733a6` (the harness's configure line with
`-DYAKOGL_GPU=powervr` and `=vivante`; `YAKOGL_SHADERC=online`,
`YAKOGL_REPORT_ES2=OFF`, `YAKOGL_SHARED=ON`), the same chunk loop, all 863 cases
began in each of the four runs. `--compare-only` reports **0 cases changed** in
both pairs, 284 Fail and 579 Pass in every one of them. In these arms the query
groups fail the same rows before and after: 8 of `implementation_limits.`
(`max_uniform_block_size`, the two combined-component rows,
`max_vertex_output_components`, `max_draw_buffers`, `max_color_attachments`,
`compressed_texture_formats`, `num_compressed_texture_formats`), 11 of
`state_query.integers.` and 6 of `state_query.integers64.`. The varyings are not
among them -- those builds report 15 / 60 / 60 at `94b733a6` already -- and
what they are below (6144-byte blocks, 1536 combined components, 60 vertex output
components) is the compiler and board work of §4, which this change left where its
owners have it. So the raised V3D rows moved on the V3D build alone, and no row of
another GPU moved by it.

## 6. Issue drafts

Agents never run `gh`; these are the texts to file in signal-slot/yakogl, each
with the commit of this change as its record.

### Draft 1 (PowerVR, pvr + es3 + limits; owner plans #350, #325): 16384-byte uniform blocks need the USC memory-load path

`GL_MAX_UNIFORM_BLOCK_SIZE` is 6144 and `GL_MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS`
1536 on the PowerVR; ES 3.0 needs 16384 and 50176 / 50048 (dEQP Fail rows:
`implementation_limits.max_uniform_block_size`,
`max_combined_{vertex,fragment}_uniform_components`, and their
`state_query.integers{,64}.*_{getfloat,getinteger64,getinteger}` forms;
`docs/es3/am62p-gles3-notpass.tsv`).

Cause: a block is flattened into the stage's default block and read as
`Bank::Shared` registers (`src/shaderc/online/powervr/select.rs` `LoadUniform`,
`reg = dword_off`); the PDS DMA fills the shared file with `uniform_dwords`
before the shader runs (`src/pvr/backend.rs`). 4096 dwords do not fit that
allocation beside the sampler descriptors, so `src/gl/context.rs` caps a block
at the default block, `MAX_UNIFORM_DWORDS * 4`.

Work:
1. `src/shaderc/online/powervr/select.rs`: for a user block above a size, select
   a memory load -- the block's GPU address in two shared registers (filled by
   the PDS DMA, one pair per block) and a global `ld` of the member's dwords
   with the constant offset, as Mesa's pco lowers `load_ubo`. [U] which `ld`
   form takes a 32-bit dword offset and what a 128-bit load needs
   (`third_party/ti-img-rogue-ddk-25.2`, Mesa 26.2.1's pco).
2. `src/pvr/backend.rs`: bind each bound uniform range's GPU address in the PDS
   data segment. The GL core hands `vs_uniform_cpu` / `fs_uniform_cpu` today; it
   must hand the buffer's GPU address and offset (`GlUniformBuffer`) and flush
   the cache the load reads.
3. `src/gl/context.rs`: 16384 for `MAX_UNIFORM_BLOCK_SIZE` and
   `MAX_UNIFORM_DWORDS + 12 * 4096` for `MAX_STAGE_UNIFORM_DWORDS` on
   `gpu-powervr` (the V3D arms already read so), after a board run draws a
   16384-byte block.
4. Tests: `tests/es3_v3d_ubo_size.rs` is the model (sixteen `vec4[64]` members,
   16384 bytes bound, the storage the back end is handed checked); `usc_exec`
   runs the selector's output on the host.
Board check (AM62P): a 16384-byte block read at its first, middle and last
dword; then dEQP `ubo.*` and the six limit rows above.

### Draft 2 (GC7000UL, vivante + es3 + limits; owner plans #335, #355): the constant-offset `load` from a user uniform block is unproven on the board

`GL_MAX_UNIFORM_BLOCK_SIZE` 6144 and the combined components 1536 on the
i.MX8MP; ES 3.0 needs 16384 and 50176 / 50048. The 320-register constant file
is the limit (`docs/notes/gc-ubo-const-limit.md`): our code generator copies
every block member into it, which Mesa's etnaviv never does for a user block
(`etnaviv_nir_lower_ubo_to_uniform.c` reads blocks above 0 from memory with
`LOAD`).

The path exists for run-time indexes (`SHC_VIV_CONST_UBO_ADDR` address slots,
`ubo_copy`), proven on the i.MX8MP for dynamic indexing. Unproven is the
constant-offset `LOAD`: a read at a compile-time offset emitted as a load of the
block's memory, not a constant-file read.

Work:
1. `src/shaderc/online/select.rs` `uniform_src` (GC7000): take the load path for
   user-block members past a threshold, constant offsets included (the four dEQP
   cases need 154 of the 320 registers with it, 368 without -- the note's table).
2. `src/vivante/shader.rs`: reserve an address slot per block that uses it;
   `src/vivante/backend.rs`: bind each block's GPU address and keep the buffer
   coherent for the shader.
3. Board test (a gl_test case, `docs/notes/gc-ubo-const-limit.md` section 5): a
   block of 4096 dwords read at constant offsets 0, 2048 and 4095, the values the
   application wrote; then `ubo.single_nested_struct_array.*`.
4. Only then `src/gl/context.rs` `MAX_UNIFORM_BLOCK_SIZE` 16384 and the
   `gpu-vivante` arm of `MAX_STAGE_UNIFORM_DWORDS`.

### Draft 3 (PowerVR, pvr + es3 + limits; owner plans #350, #325): is `GL_MAX_VERTEX_OUTPUT_COMPONENTS` 64 reachable?

`implementation_limits.max_vertex_output_components` and its
`state_query.integers.*` forms Fail on the AM62P with 60 (15 x 4). The VDM's
`vs_output_size` is 8 bits in units of 4 dwords (Mesa csbgen `rogue/vdm.xml`
VDM_STATE4) and the record holds position 4 + 60 varyings + point size = 65
dwords, so the hardware field is not the limit. A program of fifteen vec4
varyings plus `gl_Position` and `gl_PointSize` builds and links on the host
(`tests/es3_pvr_varyings.rs`). Board question: it draws correctly; if it does,
`VERTEX_OUTPUT_EXTRA` becomes 4 for `gpu-powervr` in `src/gl/context.rs` and the
row closes with `tests/es3_caps.rs` and `docs/es3/backend-caps.md` §5.

### Draft 4 (GC7000UL, vivante + es3 + limits; owner plans #335, #355): is `GL_MAX_VERTEX_OUTPUT_COMPONENTS` 64 reachable?

Same row, 60 on the i.MX8MP. The vertex stage has 16 io rows
(`SHC_VIV_MAX_IO`), one of them the position, and 15 varyings reach the
fragment stage: 64 components counts the position as the sixteenth vector, as
on the V3D. The code generator refuses a vertex shader with 16 outputs
(`Abi(LoadBalancingTempsUnmeasured)`, `src/shaderc/online/select.rs`), which is
15 varyings + the position + something else. Board run: fifteen vec4 varyings
plus `gl_Position` (and `gl_PointSize`); if it draws, `VERTEX_OUTPUT_EXTRA` 4
for `gpu-vivante`.

### Draft 5 (V3D, v3d + shaderc): a 60-input fragment shader can run out of registers

With `MAX_VARYING_VECTORS` 15, programs of 60 fragment inputs reach the code
generator and some end `Program(NotYet("more live values than 2 threads have
registers (no spilling yet)"))` (`src/shaderc/online/v3d/chain.rs:245`), the
dEQP `random.interleaved.triangles.173` program among them
(`tests/es3_shaderc_infolog.rs`). They failed at link before the raise as well,
so no dEQP row moves from Pass. The work is spilling, or fewer live inputs (read
each `ldvary` result at its use) in `online/v3d/select.rs` and `alloc.rs`. Host
check: the infolog test's ceiling lowers.

### Draft 6 (V3D, v3d + board): measure the 15-varying and 16384-byte-block draws

On a Pi 4: gl_test 97 (`varyings15`); a program of 60 inputs with a flat vec4 at
inputs 24..27 and a centroid one at 48..51 (flag words 1 and 2, the `*_hi`
arrays); a vertex shader of 65 output rows (nine VPM sectors); the same with
`gl_PointCoord` (two implicit varyings ahead of 60 declared ones: 62 of the
hardware's 64); and a 16384-byte block read at its first and last dword. Then
dEQP `implementation_limits.*`, `state_query.integers*`, `ubo.*` and
`shaders.linkage.varying.*`. `docs/es3-cts-failures.md` RC10 moves on the result
and `tools/cts/RESULTS.md` takes the rows. Whether Mesa's 16 vectors (69 VPM
rows) is what the hardware takes is the same run: `a_vs_writes_16_vec4_outputs`
in `src/shaderc/online/v3d/varyings_tests.rs` stays `#[ignore]`d until then.

### Draft 7 (GL core, gl + es3): the uniform-block limits are queried and not enforced

Nothing checks a block against `GL_MAX_UNIFORM_BLOCK_SIZE`: the constant is read
by the query arms only (`src/gl/state.rs:1263-1270`, `:1069`), `block_dwords`
caps a program's storage at `MAX_STAGE_UNIFORM_DWORDS`
(`src/gl/program.rs:1861-1869`) and the draw clamps a stage's dwords
(`src/gl/draw.rs:3919-3923`), so an over-large block is silently truncated
instead of a link error. `MAX_STAGE_UNIFORM_BLOCKS` 12 is likewise a number the
query answers: `PROG_MAX_BLOCKS` is 24 (`src/gl/program.rs:69`) and no per-stage
count is checked. ES 3.0 wants both enforced (a block over
`MAX_UNIFORM_BLOCK_SIZE` fails to link; a 13th block of a stage is an error).
Fix in `src/gl/program.rs` at link, with a `tests/es3_ubo.rs` case each; note
that #343 owns `src/gl/program.rs` (this plan kept its edit to
`block_dwords` and the import).

### Draft 8 (V3D, v3d + shaderc + es3): `ldunifa` -- a uniform block read at a run-time index

`Op::LoadUniformIdx` selects among at most `MAX_SELECTED = 256` components
(`src/shaderc/online/v3d/prepare.rs:1394-1396`), so a dynamically indexed `vec4`
array is at most 64 entries, and `UniformRow::array_len` is a `u8`
(`src/shaderc/online/reflect.rs` `fit_u8` `:1482`), so an array of 256 elements
is refused by the front end. Mesa lowers these to `ldunifa`/`quniform_ubo_addr`
reads of the block in memory (`v3d_limits.h` `V3D_MAX_UBOS`, `nir_to_vir.c`).
The three `#[ignore]`d cases of `tests/v3d_ubo_large_sim.rs` are the tests this
closes; the ABI already has `QUNIFORM_UBO_ADDR` unused.

### Draft 9 (shaderc, es3): the ES 3.00 link does not check the varying component count

`src/shaderc/link.rs` checks the ES 2.00 varying count (Appendix A packing); the
ES 3.00 path (`src/shaderc/online/reflect.rs`, the varying tables) has no check
of the component totals against `MAX_VARYING_COMPONENTS` /
`MAX_VERTEX_OUTPUT_COMPONENTS` / `MAX_FRAGMENT_INPUT_COMPONENTS`, so a program
past them is refused by the back end (or, on a back end that does not count,
drawn wrong) instead of failing at link with a length the app can read. With the
limits now at 15 / 60 / 60 / 64 this is what makes `sixteen_varying_vectors_do_not_link`
(`tests/es3_v3d_varyings.rs`) hold for every back end, not only the V3D.

### Draft 10 (V3D, gate): the `report-es2` V3D library tests fail, and the gate only lints that config

`cargo test --lib` under `gpu-v3d,shaderc-online,report-es2,no-shader-table`
(`V3D_ES2_CONFIG`) fails 12 cases of `v3d::backend::backend_tests` (the
several-attachment, integer-clear and float-attachment cases) -- already at
`94b733a6`, so not a regression of this change. The gate runs clippy
`--all-targets` for that config and only `--lib shaderc::online::v3d` of its
tests (`tools/check.sh`), so the failures are invisible there while
`tests/es2_report_guard.rs` keeps the ES 2.0 answers pinned. Either fix the
cases or make the row run them; naming it here keeps the gap visible.

### Draft 11 (docs): line citations in five notes moved with this change

`src/gl/context.rs` grew 68 lines above them and `src/gl/draw.rs` 15 (the uniform
storage and the varying notes), so citations in notes this change did not touch
point 11 to 68 lines early now. The values they name are unchanged, only the
numbers: `docs/notes/es3-float-targets.md:189` `context.rs:532` -> `:600`
(`pub rgba32f`), `docs/notes/instanceid-per-draw.md:29` `draw.rs:5224` -> `:5239`
(`draw_instanced`), `docs/notes/pvr-int-mixed-targets.md:160,387,389,390`
`draw.rs:1346` -> `:1357` (`map_blend_factor`), `:1409` -> `:1420`
(`current_blend_key`), `:1398` -> `:1409` (`several_targets`), `:2499` -> `:2510`
(`resolve_targets`), `docs/online-shaderc/v3d-backend.md:1399` `draw.rs:996` ->
`:1007` (the upload's `GL_UNSIGNED_SHORT` arm), and
`docs/notes/gc-mrt4-board-check.md:52` `context.rs:154-156,163-165`, which names
a `MAX_VARYING_VECTORS` of 8: that is a session record, so re-pin it to
`context.rs:209-210` only if the note is meant to describe this build. Each owner
re-pins their own note.

The same kind of drift sits in `docs/es3/backend-caps.md` section 2 and section 6,
older than this change (`state.rs` gained ~36 lines below those cites before it,
`context.rs` 68 more with it). The rows this change's limits are about are re-pinned
here: 161, 167, 169, 170, 186-187, 189-190, 198. The rest of §2's GL-core cites are
listed with their current lines, for whoever takes the doc: 151 `state.rs:1250` ->
`:1286`, `context.rs:139` -> `:234-242`; 152 `state.rs:1249` -> `:1285`,
`context.rs:152` -> `:226-232`; 153 `state.rs:1251` -> `:1287`; 154
`state.rs:1253-1254` -> `:1289-1290`; 155 `state.rs:1190-1192` -> `:1226-1228`; 156
`state.rs:279,1180-1183` -> `:293,1216-1219`; 157 `state.rs:1184` -> `:1220`
(`context.rs:78` is right); 159 `state.rs:1198-1199,1214-1215` ->
`:1234-1235,1250-1251` (`context.rs:83,89-90` right); 324 `state.rs:1251` ->
`:1287`; 325 `state.rs:1255` -> `:1291`. Row 316 is not only a number: it cites
`context.rs:141-152` for a claim ("the GC7000 and V3D draw four targets") that the
comment there no longer makes -- `max_draw_buffers` and its comment are now at
`context.rs:226-232` and say 3 on every GPU until each back end's half of ES 3.0's 4
lands, and `CAP_COLOR_ATTACHMENTS` is at `:218-225` -- so that row needs the doc
owner's reading, not a re-pin.

## 7. What this change raised, and what each row is now

| Row | Before | Now | Test |
|---|---|---|---|
| `MAX_VARYING_VECTORS`, `_COMPONENTS`, `MAX_FRAGMENT_INPUT_COMPONENTS` (V3D) | 8 / 32 / 32 | 15 / 60 / 60 | `tests/es3_v3d_varyings.rs`, `tests/es3_caps.rs`, `tests/es3_limits_v3d.rs` |
| `MAX_VERTEX_OUTPUT_COMPONENTS` (V3D) | 32 | 64 | same |
| `MAX_UNIFORM_BLOCK_SIZE` (V3D) | 6144 | 16384 | `tests/es3_v3d_ubo_size.rs` |
| `MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` (V3D) | 1536 | 50688 | same |
| PowerVR, GC7000UL, Mali | unchanged | unchanged | `tests/es3_caps.rs` gap rows, `tests/vivante_imx8mp_limits.rs` |
| `MAX_DRAW_BUFFERS`, `MAX_COLOR_ATTACHMENTS` (3), `MAX_SAMPLES` (1), the compressed-format count | gaps | gaps, untouched | RC14, RC8, plan #354 |
