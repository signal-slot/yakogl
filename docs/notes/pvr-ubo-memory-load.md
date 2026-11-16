<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR uniform blocks read by `ld` from memory

Date 2026-11-16, tip `b72adb59`. Plan #378 item `design-note`. This note is
docs only: no code was changed and nothing was run (no `PCO_SHC`, no board).
Mesa is the 26.2.1 tree (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, MIT; the
note quotes facts from it, not code). Mesa paths are relative to
`src/imagination/`. `[M]` = measured by reading, `[U]` = unknown, not
measured.

The goal is that the PowerVR (AM62P, BXS-4-64) reports
`MAX_UNIFORM_BLOCK_SIZE` 16384 and combined uniform components 50688 under ES
3.0 (`docs/notes/ubo-limits.md` §1, §4). That is honest only if the USC reads
block members from memory, because a 4096-dword block does not fit in 2048
shared registers (`src/shaderc/online/powervr/chain.rs:153`). The V3D raise
on main is the model for the GL side (`src/gl/context.rs:109-131`,
`src/gl/limits.rs:40-46`). The GC7000 path is the model for the compiler
side (§5 below).

## 1. pco's `ld` encoding

**The op.** `O_LD = hw_op('ld', OM_ALU_RPT1 + [OM_MCU_CACHE_MODE_LD], 1, 3)`
and `O_LD_REGBL` likewise (`pco/pco_ops.py:409-410`). Both have one
destination and three sources: `src0` = data-return channel (DRC), `src1` =
burst length, `src2` = address. [M]

**Encode maps** (`pco/pco_map.py:1352-1374`) [M]:
- `ld` → `I_LD_IMMBL`: `drc` ← `SRC(0)`, `burstlen` ← the immediate in
  `SRC(1)`, `srcseladd` ← the group source slot holding `SRC(2)`,
  `cachemode_ld` ← the op modifier.
- `ld.regbl` → `I_LD_REGBL`: the same, except the burst length is read from a
  register. `srcselbl` ← the slot holding `SRC(1)`.

**The backend-op word.** It is part of the `I_BACKEND` bit set
(`pco/pco_isa.py:1854-1926`, field types `:2027-2044`). The bit structs are
`I_LD_IMMBL` (`:2380-2394`, 3 bytes) and `I_LD_REGBL` (`:2396-2409`, 2
bytes). The repo carries both: `src/shaderc/online/powervr/isa_tables.rs:427`
(`ld_immbl`, `bytes: 3`) and `:428` (`ld_regbl`, `bytes: 2`). [M]

| byte | bits | field | `ld_immbl` | `ld_regbl` |
|---|---|---|---|---|
| 0 | 7:5 | `backend_op` | `dma` = 0b111 | `dma` = 0b111 |
| 0 | 4 | `immbl` | 1 | 0 |
| 0 | 3 | `drc` | DRC 0/1 | DRC 0/1 |
| 0 | 2:0 | `dma_op` | `ld` = 0b001 | `ld` = 0b001 |
| 1 | 7:5 | `srcseladd_ldst` | s0..s5 = 0..5 | s0..s5 |
| 1 | 4:2 | `burstlen_2_0` / `srcselbl_ldst` | burst bits 2:0 | burst-length slot s0..s5 |
| 1 | 1:0 | `cachemode_ldst` | `cachemode_ld` | `cachemode_ld` |
| 2 | 7:1 | `tiled`, `srcseldata`, `dsize`, `rsvd2_ldst` | `rsvd2_ld` = 0 (7 bits) | -- |
| 2 | 0 | `burstlen_3` | burst bit 3 | -- |

The field definitions behind the table [M]:
- `backend_op` is `F_BACKEND_OP`; `dma` = 0b111 (`pco/pco_isa.py:1647-1658`).
- `dma_op` is `F_DMA_OP`; `ld` = 0b001 (`:1768-1776`).
- `srcsel` is s0..s5 = 0..5 (`:1679-1688`).
- `cachemode_ld` (`:1778-1784`): `normal` 0, `bypass` 1,
  `force_line_fill` 2. The repo has it at `isa_tables.rs:30`; the field is
  at `:327`.
- `burstlen` is `F_UINT4_POS_WRAP` (`:135`): values 1..16, encoded `% 16`,
  so 16 is 0. Its two pieces are byte 2 bit 0 (the high bit) and byte 1 bits
  4:2 (`isa_tables.rs:326`, `Xform::WrapMod16`).

**Group.** `group_map(O_LD)` (`pco/pco_map.py:3301-3316`) builds the group
like this [M]:
- Header `I_IGRP_HDR_MAIN` with `oporg = be`, `w0p = w1p = false` (no ALU
  write-back), `rpt = 1`, and `cc`/`end`/`atom`/`olchk` from the modifiers.
- The address `SRC(2)` goes in the group's source slot `s[0]`, so
  `srcseladd` = s0.
- The destination `DEST(0)` goes in slot `s[3]`.

`ld.regbl` (`:3318-3335`) also puts the burst-length register in `s[1]`
(`srcselbl` = s1). The address is a 64-bit register pair
(`pco_ref_new_ssa_addr`, two 32-bit components). The destination is `chans`
consecutive registers.

**Cache mode.** `get_ld_cache_mode` (`pco/pco_trans_nir.c:186-197`) picks
`bypass` for `coherent`/`volatile` access and `normal` otherwise. A GL
uniform block is `normal`. [M]

**Example backend-op words** (derived by hand from the tables above, DRC 0,
`normal`, address in s0) [M by reading; the `ld-encode` item checks them
against `PCO_SHC`]:

| instruction | bytes 0, 1, 2 |
|---|---|
| `ld drc0, 1, s0` (one dword) | `0xF1 0x04 0x00` |
| `ld drc0, 4, s0` (a vec4) | `0xF1 0x10 0x00` |
| `ld drc0, 8, s0` | `0xF1 0x00 0x01` |
| `ld drc0, 16, s0` | `0xF1 0x00 0x00` |
| `ld.regbl drc0, s1, s0` | `0xE1 0x04` |

**Burst-length forms.** The immediate form carries 1..16 dwords. The
register form takes the count from a register. pco uses the register form
only for the tile-buffer load (`trans_flush_tile_buffer`,
`pco/pco_trans_nir.c:726-773`, burst up to 1024 written
as 0). A uniform-block member is at most a vec4 per IR load (`ir.rs:468-472`,
`comps`), so **the immediate form is enough here**; `ld.regbl` is not needed.

## 2. `load_ubo` in pco, and our two sequences

**Correction to the plan text.** `nir_intrinsic_load_ubo` does not go to
`trans_load_global`. It is translated by **`trans_load_buffer`**
(`pco/pco_trans_nir.c:1190-1246`, dispatched at `:2191-2194` together with
`load_ssbo`). `trans_load_global` (`:1248-1263`) serves `load_global_2x32`
(`:2196-2197`). Both end in the same `pco_ld`. [M]

`trans_load_buffer` [M]:
1. Finds the descriptor's shared register `sh_index`
   (`fetch_resource_base_reg_packed`, `:822-838` → `:776-820`: the binding's
   range start plus `elem * stride`).
2. Adds the offset to the address:
   `add64_32 a.lo, a.hi = sh[sh_index], sh[sh_index+1], offset (.s)`.
   The base address is the descriptor's first two dwords.
3. Adds the dynamic offset:
   `add64_32 b.lo, b.hi = a.lo, a.hi, sh[sh_index+3] (.s)`. Dword 3 of the
   descriptor is the dynamic offset; `struct pvr_buffer_descriptor` is
   `{u64 addr; u32 size; u32 offset}`, `vulkan/pvr_common.h:190-196`.
4. `ld dest, drc0, chans, b` (burst = `chans`, the destination's component
   count; 32-bit only, `assert(bits == 32)`).

The constant offset and the run-time offset take the same path. A constant
is an immediate source: `trans_const` emits `movi32`
(`pco/pco_trans_nir.c:3731-3801`), and `pco_const_imms.c` turns small
values into hardware constant registers. `add64_32`
(`pco/pco_ops.py:385`, encode `pco/pco_map.py:1214-1232`, group
`:2282-2300`) is `int32_64` op `add6432`. It writes `ft0`/`fte` (lo/hi). The
carry-in is used when `SRC(3)` is not null. [M]

**Our shape.** We keep one address per stage, not one per block (§6
explains why). The offset is the member's byte offset in the stage's
flattened uniform array.

**Constant offset** (an `Op::LoadUniform { dword_off, comps }` whose dword
is a block member):

```text
add64_32  t0, t1 = sh[A], sh[A+1], imm(dword_off * 4)   (.s, no carry-in)
ld        t2..t2+comps-1, drc0, comps, t0:t1            (normal)
wdf       drc0
```

- `A` is the address pair (§3).
- `imm` is the existing immediate path: `SelSrc::Imm`, which prepare
  materialises from `CONST_REGS` (`src/shaderc/online/powervr/prepare.rs:62-70`)
  or by a move.
- A vecN member is **one** `ld` with burst N into N consecutive temps. The
  value's lanes are those temps, in place of today's
  `SelSrc::Fixed { bank: Shared, reg: dword_off + pick }`
  (`src/shaderc/online/powervr/select.rs:676-677`).
- Two loads of the same block dword are two `ld`s until a CSE exists. pco
  does not share them either. [M]

**Run-time offset** (`Op::LoadUniformIdx { base_dw, stride_dw, len, index,
comps }` over a block-member array):

```text
<clamp index to [0, len)>                    (as today: select.rs:1950-1970)
imul      o, i, imm(stride_dw * 4)           (byte stride; always emitted)
iadd      o, o, imm(base_dw * 4)             (when base_dw > 0)
add64_32  t0, t1 = sh[A], sh[A+1], o         (.s)
ld        t2.., drc0, comps, t0:t1
wdf       drc0
```

The clamp is today's (`select.rs:1950-1970`): `ilt`/`bcsel` twice. It keeps
the read inside the array, so `ld` never runs past the snapshot (§6). The
existing default-uniform path writes `idx0` and reads `shared[idx0]`. The
`ld` path does not touch `idx0`, so a block read can sit between two
`idx0` reads (`prepare.rs:1339`).

A 64-bit address with no offset could in principle go straight from the
shared pair into `s0`. pco never does that; it always adds into temps. We
do the same [U whether a backend `s0` may name a shared pair].

## 3. The address source: a shared-register pair

**What Mesa does** [M]:
- The Vulkan driver puts descriptors, not addresses, into shared registers.
  The PDS descriptor program DMAs each used descriptor set's range from the
  set's memory into the shared registers at `range.start`
  (`vulkan/pvr_arch_pipeline.c:474-530`: `pvr_pds_descriptor_set {
  .size_in_dwords = range.count, .destination = range.start }`).
- At record time the const map writes the set's device address into the
  PDS data segment
  (`PVR_PDS_CONST_MAP_ENTRY_TYPE_DESCRIPTOR_SET`,
  `vulkan/pvr_arch_cmd_buffer.c:5567-5585`).
- Dynamic buffers get their own `PVR_BUFFER_TYPE_DYNAMIC` DMA (`:5594` ff.).
- The shader then reads `sh[sh_index], sh[sh_index+1]` as the buffer's base
  (§2).
- `maxUniformBufferRange` is 16 KiB and there are 12 per-stage uniform
  buffers (`vulkan/pvr_physical_device.c:626,635`).

**Ours (decision).**

*Per stage.* There is one 64-bit pair `{lo, hi}` per stage that reads any
block member. It holds the device address of that draw's snapshot of the
stage's flattened uniform array (§6).

*Where it sits.* The pair is at an **absolute** shared register, past the
descriptors and the default-uniform prefix. This is the same convention as
`meta_off` (`src/shaderc/online/powervr/abi.rs:105-110`;
`src/pvr/backend.rs:7831-7855` uses it as `dest_offset` verbatim) and
`fs_front_face_off1` (`src/shaderc/mod.rs:1341`; `chain.rs:1731` writes
`f + 1`). We choose an even register [U whether the hardware requires 64-bit
alignment for an `add64_32` source pair; even costs at most one register].

*How the compiler names it.*
- A new record field `ubo_addr_off1: u16` on `ShcShader` (`src/shaderc/mod.rs`,
  next to `fs_front_face_off1`, 0 = the stage reads no block member). The
  `UscProgram` mirror `ubo_addr_off: Option<u16>` goes in `abi.rs`.
- The chain computes it like `meta_off` (`chain.rs:758`:
  `base + meta_dword`). It is the first even register past the
  default-uniform prefix and before `meta_off`. The front-face and meta
  registers move up by two when it is present.
- `src/shaderc/mod.rs` is held by plan #366. The field waits for it, or is
  added in coordination.

*What the back end writes per draw.* It writes two dwords,
`snapshot_gpu as u32` and `(snapshot_gpu >> 32) as u32`, through the same
mechanism as the meta word: `arena_put_words(&mut b.gen, &[lo, hi], ...)`
and one `PvrPdsDmaKick { address, dwords: 2, dest_offset: ubo_addr_off1 -
1 }` (`backend.rs:7837-7855`). The back end then raises `shared_dw` to cover
it.
- This needs **no PDS-generator change** (`driver/src/pvr_pds_render_gen.*`):
  it is one more DMA kick of a few dwords.
- The kick count per program is descriptors + default prefix + address +
  meta (+ front face). That is at most 5 of `PVR_PDS_DMA_MAX_KICKS` = 8
  (`src/pvr/ffi.rs:116`) [U: count the vertex program's kicks at
  `backend.rs:8385` ff. as well].
- A PDS constant (DOUTW from the data segment, the way Vulkan writes literal
  constants) would save the 8 bytes of arena. It is not needed for
  correctness, and it would touch the generator that #350 holds.

*Shared registers at or above 256.* A pair at sh256 or above is read through
`idx0` by `index_high_shared` (`prepare.rs:1311-1330`,
`SHARED_DIRECT_LIMIT`), as for any shared source [U: that a 64-bit
`add64_32` source pair read through `idx0` reads both halves. The
`ubo-select` item tests a pair above 256]. A default prefix of up to 1536
dwords puts the pair there.

## 4. The wait on the result

pco's `pco_schedule` (`pco/pco_schedule.c:24-55`) puts `wdf <drc>` directly
after every instruction with a DRC source (`ld`, `smp`, ...). For `st` with
`idf` it puts an `idf` first. There is no latency hiding: the next
instruction waits for the data. `wdf` is a control group: `I_IGRP_HDR_CONTROL`,
`ctrlop = wdf`, `miscctl = DRC` (`pco/pco_map.py:3747-3757`; op
`pco/pco_ops.py:467`). pco uses DRC 0 for every load (`pco_ref_drc(PCO_DRC_0)`,
`pco_trans_nir.c:1241,1259`). [M]

The repo already does exactly this for samples:
- `SelOp::WaitData` ("Wait for data-return channel 0: `wdf`, after a
  sample", `src/shaderc/online/powervr/sel.rs:396-397`).
- `control("wdf", &[], "e1_zx", "wdf")` (`fragment.rs:415-417`).
- The `wdf` variant is `isa_tables.rs:530`, the op `:646`.

**Decision.** Every `ld` is followed by `SelOp::WaitData` on DRC 0, as pco
does it.

*Scheduler interaction.* The allocator must treat the `ld` destination
temps as written at the `wdf`, not at the `ld`. The simplest form is the one
the sample path uses: the `ld` and its `wdf` are one unit that nothing is
scheduled between (`chain.rs:1322` describes the sample's
"`wdf`, and a move of each result"). Two DRCs exist (`drc` is one bit). Batching
several `ld`s before one `wdf`, or splitting `smp`/`ld` across DRC 0/1, is a
later optimisation pco does not make either.

## 5. Default uniforms in shared registers, members through `ld`

**Where the member list comes from.**
- The front end's reflection lists every block member
  (`Reflection::block_members`, `src/shaderc/online/reflect.rs:133`; built at
  `:399-411`). Each entry's `row` indexes `reflection.uniforms`, whose
  `dword_off`/`dwords` are the member's place in the stage's flat array.
- The GC7000 compile turns that into `(dword_off, dwords)` ranges
  (`src/shaderc/online/codegen.rs:815-824`) and hands them to
  `select::select_blocks` (`codegen.rs:375-386`). `select.rs:1810-1857`
  `uniform_block` marks those dwords, and `reads_block_memory` (`:1864-1866`)
  sends a read there to `block_read` (`:1897` ff., a `load` from
  `ubo_addr_reg`, `:2753-2768`).
- The PowerVR does the same. The chain builds the member ranges from
  `lowered.reflection.block_members` in the same way, and passes them to a
  `powervr::select::select_blocks(m, arena, members)` beside
  `select` (`src/shaderc/online/powervr/select.rs:5325`).
- Note the name clash: `chain.rs:1854` `block_uniform_rows` returns every
  row of the *default* block (all rows with a `dword_off`). It is not the
  member list and must not be used as one.

**What selection does.**
- `Op::LoadUniform { dword_off, comps }`:
  - When `dword_off` is in a member range, it uses the §2 constant-offset
    sequence.
  - Otherwise it reads `SelSrc::Fixed { Shared, dword_off + pick }` as
    today (`select.rs:676-677`).
- `Op::LoadUniformIdx { base_dw, .. }`:
  - When `base_dw` is in a member range, it uses the §2 run-time sequence.
  - Otherwise it uses the existing `idx0` read of the shared registers
    (`select.rs:1950` ff.).
- A load that straddles a member and a default row is refused by name
  (`NotYet`). Reflection gives each member its own row, so this should not
  occur [U].

**The shared prefix.**
- The shared registers then hold only the dwords read through shared
  registers. The compiler reports that prefix as the end of the last
  non-member dword that is read, in `uniform_sh_dwords`
  (`src/shaderc/mod.rs:1363`; `abi.rs:550` zeroes it today).
- The back end DMAs `[0, uniform_sh_dwords)` of the snapshot into
  `sh[desc_dw..]` instead of the whole `fs_uniform_dwords`.
- This is the honest form of `PVR_BACKEND_UNIFORM_PREFIX`
  (`backend.rs:285-291`). It was turned off because the old count was the
  declared size, not what the shader reads (the Qt Quick 3D transform at
  dword 64 of 100). The new count is computed from the reads, so a read past
  it is impossible by construction.

[U] Is every member row placed after every default row in the flat array?
If a member row came first, it would sit inside the shared prefix. That
wastes registers but is still correct. The `ubo-chain` item asserts the
order on a mixed shader. If the order is wrong, the prefix is still bounded
by the default budget of 1536 dwords (packing, `MAX_UNIFORM_DWORDS`) plus
those members.

## 6. The per-draw back-end contract

**Decision: upload the GL core's flattened host array per draw. Do not
address the bound buffer object.**

Option A (chosen) is the per-stage snapshot. The GL core already flattens
each bound range into the program's per-stage host array:
`fill_uniform_blocks` / `copy_block_member` (`src/gl/program.rs`, see
`ubo-limits.md` §2). It hands the back end `vs/fs_uniform_cpu` and the dword
count. The back end already `arena_put`s that array into the GENERAL arena
per draw (`backend.rs:7795-7801`, VS `:8385-8396`). Its address is the
snapshot address. Reasons:
1. **No GL-core change in the read path.** std140 offsets, `row_major`,
   ranges shorter than the block (zeros), arrays of blocks and the
   member-to-row map all stay in `program.rs`. The shader reads dword
   `dword_off`, as the shared path does.
2. **Snapshot semantics.** Several draws in one kick that share a program
   each read their own block (the reason for the per-draw snapshot,
   `backend.rs:7781-7785`). A `glBufferSubData` after a draw cannot change
   what an in-flight draw reads. Addressing the buffer object directly would
   need copy-on-write of in-flight buffers (`docs/notes/gl-buffer-update.md`)
   and an `UNIFORM_BUFFER_OFFSET_ALIGNMENT` that matches the hardware.
3. **Bounds.** The snapshot is always the whole layout (`block_dwords`,
   `program.rs:1881-1890`). Constant offsets lie inside it by construction,
   and run-time ones are clamped (§2). So `ld` cannot fault on a short
   binding.
4. **One address, not twelve.** One pair per stage costs 2 shared
   registers and 1 DMA kick. Per-block addresses would cost 24 registers and
   12 kicks, beyond the 8-kick cap.
5. **It is the GC7000's shape.** There `ubo_copy` is the block's copy and
   `SHC_VIV_CONST_UBO_ADDR` the address (`ubo-limits.md` §5). One model
   serves both GPUs.

Option B (direct buffer address) saves the copy of up to 2 x 198 KiB per draw
at the 50688-dword ceiling. It is a later optimisation, if a profile ever
shows the copy.

**What the back end must do per draw (each stage):**
1. Put the whole snapshot (`*_uniform_dwords` dwords) in the GENERAL arena,
   as today. This is the `ld` target.
2. DMA only `[0, uniform_sh_dwords)` of it into the shared registers after
   the descriptors (§5).
3. When `ubo_addr_off1 != 0`, write `{lo, hi}` of the snapshot address to
   `sh[ubo_addr_off1 - 1]` (§3).
4. Size the GENERAL arena per draw by the draw. `GEN_PER_DRAW_MAX` = 16 KiB
   ("Two 1536-dword uniform snapshots + staged attributes",
   `backend.rs:450-451`) is the reserve `draw_part` checks before a draw
   (`:6886`) and bounds at setup (`:9382`). A 50688-dword stage is 202752
   bytes. So the reserve becomes `align16(vs_dw * 4) + align16(fs_dw * 4) +`
   the staged attributes, computed from `d`. A draw that does not fit an
   empty arena is refused (`-ENOMEM`), not split. `ARENA_GEN_MAX` is 4 MiB
   (`backend.rs:305`), room for two full stages [U: the default GENERAL
   arena size a context actually gets, against 2 x 202752 bytes].
5. **The snapshot must be readable by the USC's `ld`.** It must be mapped in
   the USC's heap and cached coherently [U: the GENERAL arena is already
   read by the PDS DMA; that the same BO is visible to a USC global load is
   checked by `tests/pvr_ubo_draw_16k.rs` on the host and on the board].

**What replaces the GENERAL-arena staging of blocks.** Nothing else is
staged. The same snapshot stops being *copied into the shared registers*
beyond the default prefix. The shared file then holds descriptors, the
default uniforms (at most 1536 dwords), the address pair, meta and front
face, which fits under 2048 in every case.

**`src/gl/draw.rs` and `src/gl/program.rs`.** No change in the read path.
They follow `MAX_STAGE_UNIFORM_DWORDS` already: `block_dwords`
(`program.rs:1889`) and draw's upload (`context.rs:113-116` says so). Once
`context.rs` gives the PowerVR the V3D's arm, they size the array at 50688.
The `u16` dword counts hold it (`context.rs:134`). The
`draw.rs` staging buffer (`fs_uniform_gpu`, `draw.rs:3978`) is only for
back ends that do not read the host arrays. The PowerVR reads
`*_uniform_cpu` when it is set (`backend.rs:7795`) [U: whether the PowerVR
ever takes the `*_uniform_gpu` path with a null cpu pointer. If it does, that
buffer must grow too].

## 7. Ordered change list and tests

Each step keeps the tree green. A step whose files another plan holds waits
for that plan.

1. **`ld-encode`** (`src/shaderc/online/powervr/{sel,encode,legalise,isa,assemble}.rs`;
   no `chain.rs`):
   - Add `SelOp::Load { burst }` (immediate form) and `SelOp::Add64_32`.
   - Encode `ld_immbl` and the `int32_64` `add6432` group.
   - Test: **`tests/pvr_ld_encode.rs`**. It checks the §1 bytes (burst 1, 4,
     8, 16; DRC 0/1; `normal`/`bypass`) and `add64_32` against Mesa pco
     through `PCO_SHC` where it is set (a `gap`, not a pass, without it), and
     always against the hand-derived bytes.
2. **`ubo-select`** (`select.rs`, `prepare.rs`, `alloc.rs`, `verify.rs`):
   - Add `select_blocks(m, arena, members)`, the §2 sequences and the §4
     `wait`.
   - Route an `ld`'s destination temps through the allocator like a
     sample's.
   - `prepare`: immediates for the offsets, and the high-shared `idx0`
     rewrite of the address pair.
   - `verify`: an `ld` is followed by `wdf` on its DRC before any read of
     its destination.
   - Unit tests in `select.rs`: a member read becomes `add64_32` + `ld` +
     `wdf`; a default read stays a shared operand; a run-time member index
     keeps the clamp; the pair is read through `idx0` when it sits at 256 or
     above.
3. **`ubo-chain`** (`chain.rs`, `abi.rs`, `src/shaderc/mod.rs`; waits for
   #361 on `chain.rs` and #366 on `mod.rs`):
   - Build the member ranges and place `ubo_addr_off1` (§3).
   - Report the read-derived `uniform_sh_dwords` (§5).
   - Test: **`tests/pvr_ubo_memory_load.rs`** (host compile, `host-test,
     gpu-powervr, shaderc-online`). A 16384-byte block (`vec4 v[1024]`) read
     at dword 0 and dword 4095, by constant and by uniform index, in each
     stage. A default uniform beside it stays in shared registers. The
     record's `ubo_addr_off1` and `uniform_sh_dwords` are checked. Both the
     member-row order (§5 [U]) and the shared total (under 2048) are
     asserted.
4. **`ubo-backend`** (`src/pvr/backend.rs`; `src/pvr/ffi.rs` only if a
   kick or arena constant moves; no `driver/src/pvr_pds_render_gen.*`; waits
   for #361 and #350):
   - The §6 per-draw steps: snapshot address pair kick, shared DMA of the
     prefix only, GENERAL reserve per draw.
   - Make `PVR_BACKEND_UNIFORM_PREFIX`'s branch the rule, or remove the
     switch.
   - `src/pvr/backend_tests.rs` gets kick-list tests.
   - Test: **`tests/pvr_ubo_draw_16k.rs`**. A host draw of a full
     16384-byte block, reading its first and last dword. On the host it
     runs through the pvr host path; the AM62P run waits for #347's boards.
5. **`ctx-limits`** (`src/gl/context.rs`, after step 4 and #375's
   `tests/es3_limits_ubo_unchanged.rs`):
   - Widen the V3D cfg arms of `MAX_UNIFORM_BLOCK_SIZE` and
     `MAX_STAGE_UNIFORM_DWORDS` (`context.rs:109-131`) to
     `any(gpu-v3d, gpu-powervr)` under `not(report-es2)`. The value is
     `ES3_MIN_MAX_UNIFORM_BLOCK_SIZE` and
     `combined_uniform_components(1536, 12, 16384)` = 50688.
   - `MAX_UNIFORM_DWORDS` and `MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS`
     stay 1536.
   - Tests: **`tests/es3_limits_pvr_ubo.rs`** (16384 / 50688 / 50688), the
     PowerVR pins of `tests/es3_limits_ubo_unchanged.rs` in the same commit,
     and `tests/es2_report_guard.rs` unchanged.
   - No `YakoglBackend` field is needed: the limit is a cfg arm, as on the
     V3D.
6. **`caps-gaps`**: the PowerVR rows of `tests/es3_caps.rs` `known_gaps()`
   (after #366).
7. **`deqp-after`**: host `dEQP-GLES3.functional.ubo.*` and the
   `implementation_limits` uniform-block cases on the PowerVR build,
   compared with the baseline.
8. **Docs**: index this note in `docs/README.md`; `ubo-limits.md` §4 and
   RC10 in `docs/es3-cts-failures.md` (after #375, #361, #366).
9. **Board** (AM62P, after #347): `functional.ubo`, `implementation_limits`,
   and the 16384-byte draw at its first and last dword. This is the first
   silicon read of a USC `ld` from the GENERAL arena (§6 item 5 [U]).

## Mesa files read

- `pco/pco_isa.py`: `F_UINT4_POS_WRAP` `:135`, `F_BACKEND_OP` `:1647`,
  `F_SRCSEL` `:1679`, `F_DMA_OP`/`F_CACHEMODE_LD` `:1768-1784`, `I_BACKEND`
  `:1854-1926`, field types `:2027-2044`, `I_LD_IMMBL`/`I_LD_REGBL`
  `:2380-2409`, `I_WDF` `:3103`.
- `pco/pco_ops.py`: `OM_MCU_CACHE_MODE_LD` `:247`, `O_ADD64_32` `:385`,
  `O_LD`/`O_LD_REGBL` `:409-410`, `O_WDF` `:467`.
- `pco/pco_map.py`: `add64_32` `:1214`/`:2282`, `ld` `:1352-1374`/`:3301-3335`,
  `wdf` `:1749`/`:3747`.
- `pco/pco_trans_nir.c`: `get_ld_cache_mode` `:186`, resource registers
  `:776-838`, tile load `:726-773`, `trans_load_buffer` `:1190`,
  `trans_load_global` `:1248`, dispatch `:2191-2197`, `trans_const` `:3731`.
- `pco/pco_schedule.c` `:24-55`; `pco/pco_const_imms.c` (constant registers).
- `vulkan/pvr_physical_device.c:626,635,642-643`;
  `vulkan/pvr_common.h:190-196`; `vulkan/pvr_arch_pipeline.c:474-530`;
  `vulkan/pvr_arch_cmd_buffer.c:5567-5600`.
