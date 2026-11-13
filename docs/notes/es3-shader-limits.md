<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface and uniform limits: what each GPU can honestly report

Plan #359 (roadmap stage 4). The limits below were under their ES 3.0 minimum
on every GPU (`tests/es3_caps.rs` `known_gaps()`); this note says what each
one can be per GPU, what carries it, what was raised and what is left.
HOST-ONLY means measured on the host, never on a board.

## 1. The float and 64-bit query rows are not a separate defect

`docs/notes/v3d-notpass-rest.md` G3 suspected a defect in `get_floatv` and
`get_integer64v`. There is none. `glGetFloatv` and `glGetInteger64v` call
`get_integerv` and widen its answer (`gl/state.rs`; the stencil masks widen as
unsigned), so the three forms of a pname are one number. dEQP's
`state_query.integers` group registers the minimum-value cases (`max_varying_vectors`,
`max_samples`, ...) only for the boolean, 64-bit and float verifiers
(`es3fIntegerStateQueryTests.cpp` at 93bca01: `implementationLimitVerifiers`
is `{Boolean, Integer64, Float}`); the integer form of the same limit is the
`implementation_limits` row. That is why a limit showed `*_getfloat` and
`*_getinteger64` Fail and no `*_getinteger` row: all three failed on the value
below the minimum, and only two forms are in that group. The `integers64` group
asks `getinteger` and `getfloat` of 64-bit limits, which is why its Fail rows
are the other two forms.

Pinned for every limit this plan moves in `tests/es3_v3d_varyings.rs`
(`the_four_limits_read_the_es3_minimums_in_every_query_form`) and
`tests/es3_v3d_ubo_size.rs`, and for the whole G3 list in
`tests/es3_limits_v3d.rs` (`every_g3_pname_answers_the_same_in_all_three_integer_forms`).

## 2. Per limit and GPU

| Limit | PowerVR | GC7000UL | V3D 4.2 | Mali |
|---|---|---|---|---|
| `MAX_VARYING_VECTORS` / `_COMPONENTS`, `MAX_FRAGMENT_INPUT_COMPONENTS` | 15 / 60 / 60 (already) | 15 / 60 / 60 (already) | **15 / 60 / 60** (was 8 / 32 / 32) | 8 / 32 / 32, unchanged (stage 5 plans) |
| `MAX_VERTEX_OUTPUT_COMPONENTS` | 60: 64 is [U], issue draft 3 | 60: 64 is [U], issue draft 4 | **64** (was 32) | 32, unchanged |
| `MAX_UNIFORM_BLOCK_SIZE` | 6144: issue draft 1 | 6144: issue draft 2 | **16384** (was 6144) | 6144, unchanged |
| `MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS` | 1536: issue draft 1 | 1536: issue draft 2 | **50688** (was 1536) | 1536, unchanged |

`report-es2` keeps every answer it had (`tests/es2_report_guard.rs`): the V3D's
`MAX_VARYING_VECTORS` is 8 there.

## 3. V3D design

### 3.1 Varyings: 15 vec4, 60 inputs

What carries sixty fragment inputs, against Mesa's v3d (26.2.1):

- **The compiler.** Each used component is one `ldvary` input in `ldvary`
  order (`online/v3d/chain.rs` `fragment_inputs`); the vertex shader writes the
  same components into the VPM after its header, in the same order
  (`wanted`). The corpus pair `es100/varyings15` (60 components) had built
  its fragment shader since before this plan (`chain.rs` pins it at 2
  threads); what refused it was the GL limit (`MAX_VARYING_VECTORS = 8`, the
  link's Appendix A packing) and, behind it, the vertex shader's VPM row cap.
- **The VPM row cap.** `select.rs` refused a vertex shader with more than 64
  output rows (`VpmRowOutOfRange`): position 4 + point size 1 + 60 varyings is
  65. The shader record's output segment size is 4 bits of 8-row sectors
  (`v3d_packet.xml` GL Shader State Record), so the cap is 15 sectors, 120
  rows (`VPM_OUTPUT_ROWS`); nine sectors for 65 rows, and with the 64 KB VPM the
  linker computes (`vpm_fields`) there are 7 batches. `vpm_fields` refuses a
  segment past 15 sectors.
- **The record.** `number_of_varyings_in_fragment_shader` is 8 bits;
  `V3D_MAX_FS_INPUTS` is 64 (`v3d_limits.h`). The job builder refused more than
  24 inputs (`draw_is_supported`, "word 0 of the varying flags only"); it takes
  64 now.
- **The flag words.** `FLAT_SHADE_FLAGS`, `NON_PERSPECTIVE_FLAGS` and
  `CENTROID_FLAGS` carry 24 flags a packet, at a 4-bit word offset. Mesa's
  `emit_varying_flags` writes one packet per nonzero word (word 0: lower
  unchanged, higher zeroed; the first nonzero word past 0: both zeroed; the
  later ones: both unchanged), `ZERO_ALL_*` when a kind has none. The record
  carried word 0 only. `struct shc_v3d_fs` and `struct v3d_fs_prog` gain
  `flags_more[6]` (flat 1, 2; non-perspective 1, 2; centroid 1, 2); the
  compiler fills them from the inputs' positions; the builder writes the packets
  (`v3d_job.c` `EMIT_VARYING_FLAGS`; the M5 triangle builder still takes word 0
  and 24 inputs only). The old compiler set bit `i` for `i` up to 31 into a
  24-bit field and refused past 31 ("a flat varying past input 31"): a flat
  varying at input 24..31 was silently truncated by the packer.
  `tools/v3d/job_check.c` part 11b decodes a 60-input job with Mesa's decoder
  (flat words 0 and 1, centroid word 2); it runs in the gate.
- **The interpreter.** `chain.rs`
  `fifteen_vec4_with_flat_and_centroid_last_flag_words_2_and_sum_right` runs the
  60-input fragment shader in `sim.rs` with each input's value taken from its
  `(location, component)` and flat inputs carried in C with noise in the ldvary
  result, and compares the sum of the fifteen vec4: an input at the wrong index
  or a flat read from the wrong register shows in a component.

`MAX_VERTEX_OUTPUT_COMPONENTS` is 64: 15 varyings and the position vector,
which a program of fifteen vec4 plus `gl_Position` and `gl_PointSize` builds
(nine sectors) and which the ABI's 16-name table carries.
`gl_MaxVertexOutputVectors` is 16, `gl_MaxFragmentInputVectors` 15
(`context.rs`, read by `lower.rs`).

Not measured on a board: a draw with sixty inputs and flags in three words. The
host shows the compiler, the records, the packets (job_check against Mesa's
decoder) and the interpreter, not what the QPU and the varying interpolator do
with a flag in word 2. The first Pi 4 run of gl_test case 97 (`varyings15`) and
a flat/centroid program of 25+ inputs are the board checks.

A program of up to 60 inputs can need more registers than a 2-thread fragment
shader has (`more live values than 2 threads have registers`; the dEQP
`random.interleaved.triangles.173` program of the infolog test): that is the
compiler's register allocator, not the limit (issue draft 5).

### 3.2 Uniform blocks: 16384 bytes

A block is flattened into the stage's uniform storage and filled from the bound
range at the draw (`gl/program.rs`). The V3D compiler reads a block member as a
word of that storage (`QUNIFORM_UNIFORM`, `prepare.rs` `SOp::Uniform(u16)`), and
the job builder writes the current value of each word the shader reads into the
uniform stream (`v3d_job.c` `write_uniforms`, bounded by the storage's count):
the cost is the number of reads, not the block's size. Measured: a block of
sixteen `vec4[64]` members (4096 dwords) with static reads and one dynamic index
compiles and links; sixteen arrays all indexed dynamically run out of scratch
(`Sched(OutOfScratch)`: the dynamic index of an array is a scratch select), a
shader limit of dynamic indexing, not of the block.

The GL core part (`gl/context.rs`):

- `MAX_UNIFORM_BLOCK_SIZE` is 16384 on the V3D and `MAX_UNIFORM_DWORDS * 4`
  elsewhere; `MAX_STAGE_UNIFORM_DWORDS` is a stage's storage (default block plus
  12 blocks) on the V3D, 50688 dwords, and `MAX_UNIFORM_DWORDS` elsewhere.
  `block_dwords` (the program's array, sized to its layout) and the draw's cap
  use it; a program's arrays are never sized to the cap.
- `MAX_COMBINED_*_UNIFORM_COMPONENTS` is ES 3.0 section 6.2's formula,
  `blocks * block_size / 4 + default-block components` = 12 x 4096 + 1536 =
  50688, above the minimums 50176 and 50048. It is true because the stage's
  storage is that large.
- The GPU staging copy (`draw.rs`, `put_uniforms`) holds two default blocks and is
  never used by the V3D (`uniforms_host_only`); a block that does not fit it is an
  error now, not a write past the end (found by `tests/es3_v3d_ubo_size.rs` on a
  stand-in back end that staged: `malloc(): corrupted top size`).

`vec4 a[1024]` in a block is still refused (`too many of this kind of
declaration`: the reflection table's element count is a `u8`); a block of that
size is written as several arrays. dEQP's blocks are not that large.

### 3.3 Not changed on the V3D

`MAX_VERTEX/FRAGMENT_UNIFORM_COMPONENTS` stay 1536: the default block's budget is
the link's Appendix A packing of the default-block uniforms (384 vec4).

## 4. PowerVR and GC7000UL

Their owners hold `src/pvr/**` (#350, #325) and `src/vivante/**` (#335, #355);
nothing there changed. The GL core's constants are shared: with
`MAX_UNIFORM_BLOCK_SIZE` not above the default block, `MAX_STAGE_UNIFORM_DWORDS`
equals `MAX_UNIFORM_DWORDS` and every cap, query and allocation of those builds is
the byte it was (`tests/es3_caps.rs`, `tests/vivante_imx8mp_limits.rs`,
`state.rs the_es3_limits_are_what_this_gpu_carries_and_programs` run on both).

- **PowerVR uniform blocks.** The USC reads the uniform block out of the shared
  register file (`powervr/select.rs`: `LoadUniform` is `Bank::Shared`,
  `reg = dword_off`), which a PDS DMA fills before the shader starts. 4096 dwords
  do not fit the shared file beside the descriptors. Reading a block of that size
  needs the memory-load path: the block's GPU address in two shared registers and
  a global load (`ld`) per read, as Mesa's pco lowers `load_ubo`. Issue draft 1.
- **GC7000UL uniform blocks.** The constant file is 320 registers
  (`docs/notes/gc-ubo-const-limit.md`); etnaviv reads a user block from memory
  with `LOAD` and only block 0 (the default block) from the constants. Our code
  generator already has the load path (`SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`) for
  run-time indexes, proven on the i.MX8MP; the constant-offset load from a user
  block is not proven. Issue draft 2.
- **`MAX_VERTEX_OUTPUT_COMPONENTS` 64.** The PowerVR's VDM `vs_output_size` is 8
  bits of 4 dwords (Mesa `rogue/vdm.xml` VDM_STATE4) and its record holds position 4
  + 60 varyings + point size: 65 dwords, so 64 components of output look
  reachable. The GC7000UL has 16 output rows, one the position: 15 varyings + the
  position is 64 components if the position row counts as the sixteenth vector, as
  on the V3D. Neither is measured. Issue drafts 3 and 4.

## 5. Host dEQP, before and after: not run

`tools/cts/host-deqp-run.sh` (README 2b) was set up on a 4-core, 8 GB host: the
patched VK-GL-CTS 93bca01 was cloned and its sources fetched, and the build of
`deqp-gles3` (748 steps) ran at about 25 steps an hour of wall clock beside the
library builds, so the runs were stopped before the first case. No dEQP verdict
is claimed. What stands in for them, HOST-ONLY: the three query forms of every
moved limit (`tests/es3_v3d_varyings.rs`, `tests/es3_v3d_ubo_size.rs`), the
60-input and 4096-dword programs through the compiler, the GL core and the stand-in
back end, and `tests/es3_ubo.rs` (2,032 `ubo` cases, 15 tests) on the V3D build.
The run to make: `GPU=v3d|powervr|vivante MODULE=gles3 CTS=<93bca01 checkout>`
with the filters `dEQP-GLES3.functional.implementation_limits.`,
`.state_query.integers`, `.ubo.` and `.shaders.linkage.`, on this commit and on its
parent (the library is `dlopen`ed, so one `deqp-gles3` serves both).

## 6. Issue drafts

Agents never run `gh`; these are the texts to file in signal-slot/yakogl.

### Issue 1 (PowerVR): MAX_UNIFORM_BLOCK_SIZE 16384 and the combined uniform components need the USC memory-load path

Labels: pvr, es3, limits. Owner plans: #350, #325 (`src/pvr/**`).

`GL_MAX_UNIFORM_BLOCK_SIZE` is 6144 and `GL_MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS`
1536 on the PowerVR; ES 3.0 needs 16384 and 50176 / 50048. dEQP Fail rows:
`implementation_limits.max_uniform_block_size`,
`max_combined_{vertex,fragment}_uniform_components`,
`state_query.integers64.max_uniform_block_size_{getfloat,getinteger}` and the two
combined ones (AM62P `docs/es3/am62p-gles3-notpass.tsv`).

Cause: a uniform block is flattened into the stage's default block and read as
`Bank::Shared` registers (`online/powervr/select.rs` `LoadUniform`); a PDS DMA
fills the shared file with `uniform_dwords` before the shader runs
(`pvr/backend.rs`). 4096 dwords do not fit the shared register allocation beside
the sampler descriptors, so the GL core's cap is the default block,
`MAX_UNIFORM_DWORDS` = 1536 dwords.

Work (the `load_ubo` / global-load path in the USC selector):
1. In `src/shaderc/online/powervr/select.rs`, for a block member past the shared
   budget (or for every user block above a size), select a memory load: the block's
   GPU address in two shared registers (filled by the PDS DMA, one pair per block)
   and an `ld` (global load) of the member's dwords with the constant offset, as
   Mesa's pco lowers `load_ubo` (the instruction and its encoding from
   `third_party/ti-img-rogue-ddk-25.2` and Mesa's pco; [U] which `ld` form takes a
   32-bit offset and what the 128-bit loads need).
2. In `src/pvr/backend.rs`, bind each bound uniform range's GPU address in the
   PDS data segment; the GL core already hands `vs/fs_uniform_cpu`; it must hand
   the buffer's GPU address and offset (`GlUniformBuffer`), and the cache flush the
   load needs.
3. In `src/gl/context.rs`, set `MAX_UNIFORM_BLOCK_SIZE` to 16384 and
   `MAX_STAGE_UNIFORM_DWORDS` to `MAX_UNIFORM_DWORDS + 12 * 4096` for `gpu-powervr`
   (the V3D arms already read so), after a board run draws a 16384-byte block.
4. Tests: `tests/es3_v3d_ubo_size.rs` is the model (a block of sixteen `vec4[64]`
   arrays bound to 16384 bytes, the draw's block checked); `usc_exec` runs the
   selector's load on the host.
Board check (AM62P): a 16384-byte block read at its first, middle and last dword,
and dEQP `ubo.*` plus the six rows above.

### Issue 2 (GC7000UL): the constant-offset `load` from a user uniform block is unproven on the board

Labels: vivante, es3, limits. Owner plans: #335, #355 (`src/vivante/**`).

`GL_MAX_UNIFORM_BLOCK_SIZE` is 6144 and the combined components 1536 on the
GC7000UL (i.MX8MP); ES 3.0 needs 16384 and 50176 / 50048. The 320-register
constant file is the limit (`docs/notes/gc-ubo-const-limit.md`): our code
generator copies every uniform-block member into the constant file, which Mesa
etnaviv never does for a user block (it reads blocks above 0 from memory with
`LOAD`; `etnaviv_nir_lower_ubo_to_uniform.c`).

Our path exists for run-time indexes (`SHC_VIV_CONST_UBO_ADDR` slots, `ubo_copy`),
proven on the i.MX8MP for dynamic indexing. What is not proven is the
**constant-offset** `load` from a user block: a read at a compile-time offset
emitted as a `LOAD` with the block's address from `SHC_VIV_CONST_UBO_ADDR` and an
immediate offset, instead of a constant-file read.

Work:
1. In `src/shaderc/online/select.rs` `uniform_src` (GC7000), take the `load`
   path for user-block members past a threshold (the four dEQP cases need 154 of
   320 registers with it, 368 without: the note's table), constant offsets
   included.
2. `src/vivante/shader.rs`: reserve the address slots for the blocks that use it;
   `src/vivante/backend.rs`: bind each block's GPU address and keep the buffer
   coherent for the shader.
3. Board test on the i.MX8MP (a gl_test case, `docs/notes/gc-ubo-const-limit.md`
   section 5): a block of 4096 dwords read at constant offsets 0, 2048 and 4095
   with the values the application wrote, then `ubo.single_nested_struct_array.*`
   (the four cases that need more than 320 registers).
4. Only then, `src/gl/context.rs` `MAX_UNIFORM_BLOCK_SIZE` 16384 and
   `MAX_STAGE_UNIFORM_DWORDS` for `gpu-vivante`.

### Issue 3 (PowerVR): is `GL_MAX_VERTEX_OUTPUT_COMPONENTS` 64 reachable?

`implementation_limits.max_vertex_output_components` and
`state_query.integers.max_vertex_output_components_{getfloat,getinteger64}` Fail on
the AM62P with 60 (15 x 4). The record's `vs_output_size` is 8 bits of 4 dwords
(`rogue/vdm.xml` VDM_STATE4), position 4 + 60 varyings + point size is 65 dwords
(`tests/es3_pvr_varyings.rs`): the hardware field is not the limit. Question for a
board run: a program of fifteen vec4 varyings plus `gl_Position` and `gl_PointSize`
draws correctly (it builds and links on the host). If it does, 64 is the V3D's
reading (15 varyings + the position vector): set `VERTEX_OUTPUT_EXTRA` to 4 for
`gpu-powervr` in `src/gl/context.rs`. Owner plans #350 and #325.

### Issue 4 (GC7000UL): is `GL_MAX_VERTEX_OUTPUT_COMPONENTS` 64 reachable?

Same row, 60 on the i.MX8MP. 16 vertex output rows, one the position
(`SHC_VIV_MAX_IO`): 15 varyings reach the fragment stage (`MAX_VARYING_VECTORS`).
64 components counts the position vector as the sixteenth. The code generator
refuses a vertex shader with 16 outputs (`Abi(LoadBalancingTempsUnmeasured)`,
`select.rs`), which is the 15 varyings + position + something else, not 15 + the
position. Board run: fifteen vec4 varyings + `gl_Position` (+ `gl_PointSize`) on the
i.MX8MP; if it draws, `VERTEX_OUTPUT_EXTRA` 4 for `gpu-vivante`. Owner plans #335,
#355.

### Issue 5 (V3D): a 60-input fragment shader can run out of registers

After the limit rose to 15 vec4 varyings, programs of 60 fragment inputs reach the
code generator and some (dEQP `random.interleaved.triangles.173`, listed in
`tests/es3_shaderc_infolog.rs`) end `Program(NotYet("more live values than 2
threads have registers (no spilling yet)"))`. They fail at link where they failed
at link before (the limit), so no dEQP row moves from Pass; the work is spilling,
or fewer live inputs (read each `ldvary` result at its use) in `online/v3d/select.rs`
and `alloc.rs`. Host check: the infolog test's ceiling lowers.

### Issue 6 (V3D board): measure the 15-varying and 16384-byte-block draws

On a Pi 4: gl_test 97 (`varyings15`), a program of 60 inputs with a flat vec4 at
inputs 24..27 and a centroid one at 48..51 (flag words 1 and 2, `flags_more`), a
vertex shader of 65 output rows (nine VPM sectors), and a 16384-byte block read at
its first and last dword; then dEQP `implementation_limits.*`, `state_query.integers*`,
`ubo.*` and `shaders.linkage.varying.*`. `docs/es3-cts-failures.md` RC10 moves on the
result; `tools/cts/RESULTS.md` takes the rows.

### Issue 7 (Mali): the limits stay

`MAX_VARYING_VECTORS` 8, `MAX_UNIFORM_BLOCK_SIZE` 6144 on `gpu-mali` are the stage 5
plans' (#346, #341, #336, #327 NanoPC-T6): Mesa's panfrost takes 32 inputs a stage
on arch >= 9 (`pan_screen.c:573`), and Valhall reads a block with `LD_PKA` from UBO 0
(`docs/notes/mali-uniform-index.md`). Nothing changed here.
