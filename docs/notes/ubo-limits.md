<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 uniform-block size and combined uniform components, per GPU

Date 2026-11-13, tip `de328573`. Plan #359 item `ubo-design-note`
(re-derived from the finished read-only study `design-ubo-limits`). Docs
only: no code was changed and nothing was run. Mesa is the 26.2.1 tree
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

Not yet checked [U]: whether block-member rows count against the 384-row
Appendix A budget at link (`packing.rs:150` `MAX_ROWS`; `program.rs:1117`
excludes `blocks.members` rows from the default-uniform list, which suggests
they do not). `ubo-v3d` must test a 16384-byte block next to 384 default rows
before relying on it.

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
5. Block-member rows are in the same flat array as default uniforms, so a
   stage with 12 full blocks has `1536 + 49152 = 50688` dwords, under the
   `u16` of `vs_uniform_dwords`/`V3dUniformStorage` (65535). No wider field
   is needed for the V3D.

## 8. Ordered change list for the V3D (what `ubo-v3d` edits)

Order matters: each step keeps the tree green.

1. `src/gl/limits.rs` (from `gl-limits-formula`): `combined_components(default_components,
   blocks, block_bytes)` const fn; wire nothing yet.
2. Back-end capability: a field on `YakoglBackend` (`src/backend.rs`) for the
   stage block capacity in dwords, default `MAX_UNIFORM_DWORDS` for PowerVR,
   GC7000UL and Mali, `1536 + 12 * 4096` for the V3D (`src/v3d/backend.rs`
   table). Grep every `YakoglBackend {` literal in `tests/` and `src/` and
   add the field. Wait for #354's edits to `src/backend.rs`,
   `src/v3d/backend.rs` and `src/gl/context.rs`.
3. `src/gl/program.rs:1869` `block_dwords` and `draw.rs:3908-3913`: replace
   `MAX_UNIFORM_DWORDS` by the capability. Check §2's open question: block
   rows must not count against `packing::MAX_ROWS`. The `draw.rs:2314`/`:8404`
   staging buffers stay `MAX_UNIFORM_DWORDS`-sized (the V3D is host-only).
   Wait for #343 (program.rs).
4. `src/gl/state.rs:1266-1270` (`!report-es2`): `MAX_UNIFORM_BLOCK_SIZE` =
   capability-derived (V3D 16384), combined components from the §1 formula;
   other GPUs keep 6144 / 1536 until their raise. Keep
   `MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` at 1536.
5. Tests: `tests/es3_caps.rs` `known_gaps()` (V3D rows leave the list),
   `tests/es3_limits_v3d.rs` (16384 / 50688), `tests/vivante_imx8mp_limits.rs`
   and `tests/es3_limits_*` for the unchanged GPUs, `tests/es2_report_guard.rs`
   unchanged; `v3d-ubo-sim-probe`'s test grows into a 16384-byte block
   compile + sim read at the last dword; a host draw test of a full block.
6. Only if dEQP shows it needed: `ldunifa` for a run-time index over more than
   256 values (`prepare.rs:1396`), after the constant-offset path is green;
   its own item, with a copy of the block into the job's data arena for
   `QUNIFORM_UBO_ADDR`.
7. A Pi 4 board step (#320): a draw with a 16384-byte block read at the first
   and last dword, and `functional.ubo` and `implementation_limits` of the
   dEQP tree.

## 9. What was built on the V3D (plan #359, `ubo-v3d`)

Steps 1-5 of §8, with one change to §7.2: the capability is a per-GPU-build
constant in `src/gl/context.rs`, as `MAX_VARYING_VECTORS` is, not a field
of `YakoglBackend`. The value is per build anyway, and a field would have
to be added to every `YakoglBackend {` literal in `tests/` and `src/` (the
2026-10-14 main breakage, CLAUDE.md).

- `MAX_UNIFORM_BLOCK_DWORDS` (V3D 4096, elsewhere `MAX_UNIFORM_DWORDS`),
  `MAX_STAGE_UNIFORM_BLOCKS` (12) and `MAX_STAGE_UNIFORM_DWORDS` (V3D
  1536 + 12 * 4096 = 50688, elsewhere 1536), all under `!report-es2`;
  `MAX_COMBINED_UNIFORM_COMPONENTS` is the stage storage, asserted equal to
  §1's formula on the V3D. Every dword count stays a `u16`.
- `program.rs` `block_dwords` and `draw.rs`'s clamp use
  `MAX_STAGE_UNIFORM_DWORDS`; the staging copy stays two
  `MAX_UNIFORM_DWORDS` blocks, and a back end that reads it (not the V3D)
  never gets more (`draw.rs` refuses with GL_OUT_OF_MEMORY rather than
  write past it).
- `state.rs` answers `GL_MAX_UNIFORM_BLOCK_SIZE` 16384 and the combined
  components 50688 on the V3D, 6144 and 1536 elsewhere.
- §2's open question: block-member rows do not count against the Appendix
  A budget. A program with two 16384-byte blocks and a default uniform in
  one stage links, and the draw hands the back end both blocks whole with
  every uniform-stream word inside the storage
  (`tests/v3d_uniform_host.rs` (f)).
- Not built: §8 step 6 (`ldunifa` for a run-time index over more than 256
  values, `tests/v3d_ubo_large_sim.rs` keeps its two `#[ignore]`d cases) and
  step 7 (the Pi 4 board run).
