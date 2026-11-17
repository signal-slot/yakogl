<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 uniform-block size and combined uniform components, per GPU

Date 2026-11-16, tip `cb2f0ffb`. Written 2026-11-13 as plan #359 item
`ubo-design-note` (from the read-only study `design-ubo-limits`); brought to
the tree's state by plan #375 item `ubo-note-update`: **the V3D raise is
implemented (`cb52fae8`)**, see sections 8 and 9. Sections 1-7 are the design
as written; 4-6 (PowerVR, GC7000UL, Mali) are still the plan. Docs only:
nothing was run. Mesa is the 26.2.1 tree
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, MIT; facts only quoted).
`[M]` = measured by reading, `[U]` = unknown, not measured.

## 1. What is reported and what ES 3.0 asks

`src/gl/state.rs:1266-1270` reports on every GPU:

| query | reported | ES 3.0 minimum |
|---|---|---|
| `MAX_UNIFORM_BLOCK_SIZE` | `MAX_UNIFORM_DWORDS * 4` = 6144 | 16384 |
| `MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS` | `MAX_UNIFORM_DWORDS` = 1536 | 50176 |
| `MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS` | `MAX_UNIFORM_DWORDS` = 1536 | 50048 |
| `MAX_{VERTEX,FRAGMENT}_UNIFORM_BLOCKS` | 12 (`:1262`) | 12 |
| `MAX_COMBINED_UNIFORM_BLOCKS` | `PROG_MAX_BLOCKS` 24 (`program.rs:69`) | 24 |
| `MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` | 1536 (`:1250-1251`) | 1024 / 896 |

`MAX_UNIFORM_DWORDS` = 1536 (`src/gl/context.rs:83`) is today three
different things in one constant: the default-block budget
(`MAX_VERTEX/FRAGMENT_UNIFORM_VECTORS` = 384, `context.rs:89-90`, which the
Appendix A packing `src/shaderc/packing.rs:150` and `gl_MaxVertexUniformVectors`
`src/shaderc/online/lower.rs:12650,12881` read), the capacity of the per-stage
block the GL core keeps, and the block-size answer.

**The formula** (ES 3.0 §6.2 / table 6.20, "implementation dependent values"):

```text
MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS   >= MAX_VERTEX_UNIFORM_BLOCKS   * MAX_UNIFORM_BLOCK_SIZE / 4
                                            + MAX_VERTEX_UNIFORM_COMPONENTS
MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS >= MAX_FRAGMENT_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4
                                            + MAX_FRAGMENT_UNIFORM_COMPONENTS
```

The minimums are this formula at the minimum inputs: 12 * 16384 / 4 + 1024 =
50176 and 12 * 4096 + 896 = 50048. Mesa computes exactly this
(`st_extensions.c:271-273`: `MaxUniformComponents + MaxUniformBlockSize / 4 *
MaxUniformBlocks`, with `MaxUniformBlocks = max_const_buffers - 1`, "the first
one is for ordinary uniforms", `:262-266`; and `MaxUniformBlockSize =
max_constant_buffer_size`, below 16384 turns UBOs off, `:190-200`). With our
own inputs (1536 default components, 12 blocks) a 16384-byte block gives
**1536 + 12 * 4096 = 50688** for both stages, above both minimums. The
reported value must come from the formula, not be set beside the other two
(`gl-limits-formula` adds the const fns in `src/gl/limits.rs`).

The number is only honest if the GL core can hold it: a program that uses all
twelve blocks of a stage at full size has 12 * 4096 dwords of block members
plus the default uniforms.

## 2. Where a uniform block is read from today (all GPUs: the GL core)

Blocks are not hardware UBOs here. They are flattened into the stage's
default-block rows and read as ordinary uniforms:

1. `fill_uniform_blocks` (`src/gl/program.rs:678`) copies each bound buffer
   range, member by member (`copy_block_member`, `:720`), into the program's
   per-stage host array (`p.vs_uniforms` / `p.fs_uniforms`) at the member's
   row. The state.rs comment (`:1256-1261`) says the same.
2. The array is allocated at link at the layout's size, not at the cap:
   `block_dwords` (`program.rs:1869`) =
   `max(sh.uniform_dwords, end of the last block-member row).min(MAX_UNIFORM_DWORDS)`,
   `calloc` at `:2000-2019`, the caps in `vs_uniform_cap`/`fs_uniform_cap`.
   (`:4858-4862` is a unit-test fixture, not the link.)
3. At draw, `draw.rs:3908-3913` clamps `vs_dw`/`fs_dw` to `MAX_UNIFORM_DWORDS`
   and hands the backend the host array (`vs_uniform_cpu`/`fs_uniform_cpu`,
   `:3934-3946,3970`) plus the dword count (`u16`, so at most 65535 dwords).
4. The per-program GPU staging buffer is `MAX_UNIFORM_DWORDS * 4 * 2 + 16`
   bytes (`draw.rs:2312-2314`, test twin `:8404`), used only by backends that
   do not read the host arrays (`uniforms_host_only()`, `src/backend.rs:2003`).

So **every stage block above 1536 dwords is cut at `block_dwords` (a member
past it is simply not stored, `copy_block_member` refuses writes past `cap`)
and again at `draw.rs:3908`**. The GL core is the first limit on all three
GPUs; the back ends are the second, below.

Block rows and the packing budget [M], settled: block-member rows do not
count against the 384-row Appendix A budget at link (`packing.rs:150`
`MAX_ROWS`; `program.rs:1117` leaves `blocks.members` rows out of the
default-uniform list). `tests/v3d_ubo_full_stage.rs` (`cb2f0ffb`) links 384
default vec4 next to a 16384-byte block in the fragment and the vertex stage,
and a 385th default vec4 does not link. The same test draws a stage with the
full default block and 12 blocks of 16384 bytes: 50688 dwords reach the draw,
each block at its 4096-dword slot, first and last dword read. Blocks sit
behind the default block's actually used rows, not always at dword 1536:
`tests/v3d_ubo_default_sweep.rs` (`bdbb8b12`) pins the placement for default
blocks of 0, 1, 255 and 384 vec4; the slots above hold because that draw's
default block is full.

## 3. V3D (Raspberry Pi 4)

**Read path today.** The V3D back end reads the host arrays only
(`uniforms_host_only`; `src/v3d/backend.rs:3878-3879`: `storage(d.vs_uniform_cpu,
d.vs_uniform_dwords)` -> `V3dUniformStorage { words, count }`, `ffi.rs:134`).
The compiler turns every uniform read into a `ldunif` of a stream word:
`Op::LoadUniform` -> `SOp::Uniform(dw)` (`src/shaderc/online/v3d/prepare.rs:1375-1385`),
selected as `Uniform { contents: QUNIFORM_UNIFORM, data: dw }`
(`select.rs:2100`). At draw the stream is built from the array by
`data` = dword index (`chain.rs:1211-1215`, `shaderc/mod.rs:1134`). So a block
dword costs **one stream word per read in the shader**, not a register, and the
only bound is `dw < m.uniform_dwords` (`prepare.rs:1380`), i.e. the block the GL
core fills. There is no 1536 in the V3D compiler or back end; the cap is the GL
core's.

**Run-time index.** `LoadUniformIdx` (`prepare.rs:1391-1445`) loads every
element and selects with the index, up to `MAX_SELECTED` = 256 values, and
refuses a longer array with `NotYet("... (no ldunifa yet)")` (`:1396-1398`).
Mesa's path for that is `ldunifa` with `QUNIFORM_UBO_ADDR`
(`nir_to_vir.c:3188-3197,3241-3360`; `v3d_uniforms.c:320-340` writes the
buffer's address as a relocation) and for the general UBO `load_ubo` through the
TMU (`nir_to_vir.c:216-225,525-575`). A 16384-byte block read at constant
offsets needs none of that; an array of 4096 dwords indexed at run time (dEQP
`ubo.random`/`*_array` cases) does.

**What the V3D can honestly report.** `MAX_UNIFORM_BLOCK_SIZE` 16384 is
reachable with the existing constant-offset path, limited by (a) the GL core
cap, (b) the draw's uniform stream size in the job's data arena [U, not read:
`ubo-v3d` checks `src/v3d/job` against one stream word per read], (c) shader
size. Mesa's v3d reports `max_const_buffer0_size = 16 * 1024 * sizeof(float)`
= 64 KiB and 16 constant buffers (`v3d_screen.c:164-165`), so the silicon is
not the limit. Combined components: 50688 by the formula, with the block
storage `1536 + 12 * 4096 = 50688` dwords per stage, which fits the `u16`
dword counts (`ffi.rs`, `backend.rs:3712`). The probe `v3d-ubo-sim-probe`
measures the compiler/sim side.

## 4. PowerVR (AM62P, BXS-4-64)

**Read path today.** The USC reads uniforms from **shared registers**: a
uniform's dword `k` is shared register `dword_off + k`
(`src/shaderc/online/powervr/abi.rs:100-102`). At draw the back end DMAs the
block through the PDS into the common store: `src/pvr/backend.rs:7775-7809`
(`PvrPdsDmaKick { address, dwords: uni_dw, dest_offset: shared_dw }`), after
the sampler descriptors; `PVR_BACKEND_UNIFORM_PREFIX` is off (`:285-291`: the
compiler's `uniform_sh_dwords` is not a bound on what a shader reads), so the
whole `fs_uniform_dwords` goes. The per-draw snapshot is arena-put in the
GENERAL arena, sized `GEN_PER_DRAW_MAX` = 16 KiB for "two 1536-dword uniform
snapshots + staged attributes" (`backend.rs:449-450`).

**Limits in the path.**
- The register-file bound: `shared: 2048` (`chain.rs:153`), the widest
  shared-register operand field is 11 bits; a source can name directly only
  registers below sh256 and the rest go through an index register
  (`prepare.rs:1311-1390`). pco's own Qt material program uses 1460 shared
  registers (comment at `chain.rs:140-152`). So **the shared registers cannot
  hold a 4096-dword block plus the descriptors**: the block-in-registers design
  tops out below 2048 dwords for both samplers and block together.
- The DMA program holds `PVR_PDS_DMA_MAX_KICKS` = 8 kicks (`ffi.rs:116`) and
  the common-store size is the PPP field `shared_reg_count` (`:7975`), whose
  hardware width is not read [U].
- The GENERAL arena per draw (16 KiB) is smaller than one 16384-byte block
  plus the second stage.

**What Mesa does.** pco has a memory path: `nir_intrinsic_load_ubo` is
translated (`pco_trans_nir.c:2191`, `trans_load_global` at `:1248`: a global
load) and the Vulkan driver advertises `maxUniformBufferRange = 16 KiB` and
12 per-stage uniform buffers (`pvr_physical_device.c:626,635`). So the
hardware reads UBOs from memory at 16384 bytes and Mesa's ES driver stack
therefore meets ES 3.0 here.

**What the PowerVR can honestly report.** 16384 is reachable only by a
memory-load path in the USC selector (read a user block with a global load of
the buffer address, the Mesa shape) with the 6144-byte shared-register path
kept for default uniforms. As built (block in shared registers), the honest
value stays near 6144; the register bound above caps it below 8192 bytes
once descriptors are counted. **Not for this plan to edit**: `src/pvr/**` is
#350/#325's. See the issue text in the item report.

## 5. GC7000UL (i.MX8MP, `src/vivante`)

**Read path today.** The constant file is 320 vec4 registers = 1280 dwords
shared by both stages (`GC_MAX_UNIFORM_DWORDS`, `src/vivante/state.rs:24`;
`NumberOfConstants` 0x140 in the NXP hwdb, `gc-ubo-const-limit.md` §2 item
6). The back end writes the constant registers as command-stream words, so it
copies each stage block to the CPU first (`take_uniforms`,
`src/vivante/backend.rs:3084-3125`): a stage over 1280 dwords is *not*
copied but handed over in place, and block members are read **from memory**
with `load` from the block's copy `ubo_copy` (`backend.rs:3010,3623-3629`),
the slot `SHC_VIV_CONST_UBO_ADDR`. This is `gc-ubo-const-limit.md` §7
(option A, done on the host 2026-10-28): the compiler leaves block-only
registers out of the constant file (`select.rs` `block_read`) and the back end
no longer refuses a block over 1280 dwords. Room in the constant file for the
two stages is `vs_room = GC_MAX_UNIFORM_DWORDS`, `fs_room = that - vs regs`
(`src/vivante/shader.rs:666-667`).

**What is left.** The memory path has no size bound of its own other than the
arena reserved for `ubo_copy` (`backend.rs:2193` reserve from
`gl.*_uniform_dwords`, `u16`), so the limit is again the GL core's
(`block_dwords`, `draw.rs:3908`). Mesa/etnaviv: `max_const_buffer0_size`
16384 "so state tracker enables UBOs", `max_const_buffers = ETNA_MAX_CONST_BUF`
(16) (`gc-ubo-const-limit.md` §2 item 5, `etnaviv_screen.c:164,183,196`), user
blocks never go back to the constant file. So 16384 bytes and the formula's
50688 are reachable by the GPU, provided `load.u32` at a constant offset works
on silicon: it is **unproven on the board** (`gc-ubo-const-limit.md` §6-7:
"not done: the i.MX8MP run of a constant-offset `load` from a user block").
The GL-core raise on this GPU waits for that board run and for #335/#355.

## 6. Mali (NanoPC-T6) and the others

The Mali back end keeps its own limits in `context.rs` (stage-5 plans own
them). Valhall reads default uniforms from the FAU and user UBOs with
`LD_PKA` from UBO 0 beside the FAU push (`notes/mali-uniform-index.md`); the
block size is not raised here.

## 7. Decisions the code relies on

1. **Split the constant.** `MAX_UNIFORM_DWORDS` stays 1536 as the *default
   uniform* budget (vectors, packing, `gl_Max*Vectors`, `GL_MAX_*_UNIFORM_COMPONENTS`).
   A separate per-stage **block storage capacity**, a back-end capability that
   defaults to 1536, bounds `block_dwords` and `draw.rs:3908-3913`. Raising the
   shared constant would also raise `gl_MaxVertexUniformVectors` and the
   packing budget, which no GPU has shown it can honour.
2. **The capability is per back end**, and the report follows it:
   `MAX_UNIFORM_BLOCK_SIZE = min(cap_dwords, 4096) * 4`-style (V3D 16384,
   others 6144 until their raise), and the combined components come from the
   formula of §1, so the three queries cannot disagree.
3. **Under `!report-es2` only** (ES 2.0 report is unchanged;
   `tests/es2_report_guard.rs`).
4. A field added to `YakoglBackend` means every literal of it in `tests/` and
   `src/` gets it (`grep -rl 'YakoglBackend {' tests src`; CLAUDE.md, the
   2026-10-14 main breakage).
5. Block-member rows are in the same flat array as default uniforms and
   follow the default block's actual size (section 2,
   `tests/v3d_ubo_default_sweep.rs`), so a stage with 12 full blocks and a
   full default block fills at most `1536 + 49152 = 50688` dwords, under the
   `u16` of `vs_uniform_dwords`/`V3dUniformStorage` (65535). No wider field
   is needed for the V3D.

## 8. Ordered change list for the V3D (the plan; status as built)

Order matters: each step keeps the tree green.

1. **Done** (`31127ec8`). `src/gl/limits.rs`: `combined_uniform_components(default_components,
   blocks, block_bytes)` const fn and the ES 3.0 minimums.
2. **Done differently** (`cb52fae8`). Planned: a `YakoglBackend` field. Built:
   a cfg-gated const pair, see section 9.
3. **Done** (`cb52fae8`). `block_dwords` (`program.rs:1889`) and the draw.rs
   upload caps (`staging_stage_dwords`, `draw.rs:2336-2342`) are bounded by
   `MAX_STAGE_UNIFORM_DWORDS`. Section 2's question is answered: block rows
   are outside `packing::MAX_ROWS`, pinned by `tests/v3d_ubo_full_stage.rs`
   (`cb2f0ffb`). The default-uniform budget and the packing stay
   `MAX_UNIFORM_DWORDS`.
4. **Done** (`cb52fae8`). `state.rs:1268-1271` answers
   `MAX_UNIFORM_BLOCK_SIZE` and the combined components from the consts (V3D
   16384 / 50688 under `!report-es2`, the others 6144 / 1536);
   `MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` stay 1536.
5. **Done.** `cb52fae8`: `tests/es3_limits_v3d.rs` (16384 / 50688),
   `tests/es3_caps.rs` `known_gaps()` lists the family for the other GPUs
   only, `v3d/backend_tests.rs` `a_uniform_block_of_16384_bytes_reaches_the_stream_whole`
   (host compile and sim read), the per-GPU pin in the `state.rs` unit test.
   Since: `tests/v3d_ubo_large_vs.rs` (`06e4b56e`, a vertex stage reads the
   first and last dword) and `tests/v3d_ubo_full_stage.rs` (`cb2f0ffb`, the
   full-stage host draw).
6. **Decided: not shown to be needed on the host** (`ldunifa`,
   `prepare.rs:1396`): section 9's verdict — no host `functional.ubo` case
   after the raise hit the refusal, and the board run that could show a need
   (step 7, `#320`) is pending.
7. **Pending**: the Pi 4 board step (#320): a draw with a 16384-byte block
   read at the first and last dword, and `functional.ubo` and
   `implementation_limits` of the dEQP tree. The boards are held by plan #347.

## 9. As implemented (`cb52fae8`, plan #359)

**The capability is two consts, not a `YakoglBackend` field** (the deviation
from section 7 item 2 and section 8 step 2). `src/gl/context.rs:94-131`:

- `MAX_UNIFORM_BLOCK_SIZE` (bytes): `ES3_MIN_MAX_UNIFORM_BLOCK_SIZE` = 16384
  under `gpu-v3d` without `report-es2`, `MAX_UNIFORM_DWORDS * 4` = 6144
  otherwise;
- `MAX_STAGE_UNIFORM_DWORDS`, the per-stage block storage capacity:
  `limits::combined_uniform_components(MAX_UNIFORM_DWORDS,
  MAX_STAGE_UNIFORM_BLOCKS, MAX_UNIFORM_BLOCK_SIZE)` = 1536 + 12 * 4096 =
  50688 on the same cfg arm, `MAX_UNIFORM_DWORDS` otherwise; asserted to fit
  the `u16` dword counts (`context.rs:134`).

Why: a struct field adds a member to every `YakoglBackend` literal in `src/`
and `tests/` (CLAUDE.md, the 2026-10-14 breakage) for a value that is a
compile-time property of the GPU feature, and the consts give `state.rs`,
`program.rs` and `draw.rs` one source, so the queries and the storage cannot
disagree. The cost: a build carries one GPU's limits, as the rest of
`context.rs` already does. **Raising the PowerVR or GC7000UL later means
editing the cfg arms of these two consts** (and the back end's own bounds);
sections 4 and 5 stay the plan for them. The Mali is unchanged.

**Matrix layout in a block: implemented, no compiler defect** [M]. A study
traced the earlier claim that the V3D compiler or the host sim reads
uniform-block matrices with the wrong layout (a `mat2` column stride of 2
dwords, `row_major` read as `column_major`) to the old test harness: it
mapped buffer dwords to default-block dwords at the compiler level and so
bypassed the GL core's std140 copy (`fill_uniform_blocks` /
`copy_block_member`, `program.rs:736,778`), the copy that makes the read
right. `tests/v3d_ubo_matrix_layouts.rs` (`4863d8ae`) puts every std140
matrix shape, `column_major` and `row_major`, in a 16384-byte block read
through the GL draw path (that copy, then the sim on the stream the draw
builds) and un-ignores `sim_reads_each_matrix_as_std140_says`: it passes for
every matrix shape in both majors. Nothing in the compiler or the sim needs
a matrix-layout fix.

**`ldunifa` (step 6).** `LoadUniformIdx` still refuses an array over
`MAX_SELECTED` = 256 values. Host dEQP-GLES3 at `26d1c4a2`
(`docs/es3/host-gles3-part-v3d.tsv`, HOST-ONLY) [M], which predates
`cb52fae8`: `functional.ubo` 0 pass / 2355 fail, `functional.implementation_limits`
38 pass / 11 fail, `functional.state_query` 914 pass / 23 fail / 9 not
supported. That is the baseline, not a verdict on the raise.

**Verdict after the raise (host dEQP `functional.ubo`, HOST-ONLY).** The
raise is in `cb52fae8`; the host run that follows it is the one of
`docs/notes/es3-interface-limits.md` section 6, library of `7c39ee92`
(VK-GL-CTS `93bca01`, `MODULE=gles3 tools/cts/host-deqp-run.sh`). Its
`functional.ubo.` result is 2355 cases, 2355 Fail, 0 Pass, unchanged from the
run before the raise. Every Fail is "Image compare failed" (the shaders
compiled and linked and the draw ran; the linux OSAL reads back black), and
**no case was refused at compile or link, so none hit the `no ldunifa yet`
refusal**: there are no refusal shapes to list. Verdict: `ldunifa` is **not
shown to be needed** by `functional.ubo`; the run cannot show the opposite
either, because the black read-back hides a wrong run-time index. So the
`ldunifa-study` design stays unstarted and becomes its own item only when a
board `functional.ubo` run (plan #347's boards, `#320`) names run-time-index
cases failing on the 256-value refusal or on a wrong value. The separate
post-raise rerun report (`ubo-deqp-rerun`, per-group pass, fail and
not-supported counts) was not available to this note, so no per-group counts
beyond the above are written here. The full-stage claim itself is pinned by
`tests/v3d_ubo_full_stage.rs` and the 256-value threshold by
`tests/v3d_ubo_dynamic_index_limit.rs`.
