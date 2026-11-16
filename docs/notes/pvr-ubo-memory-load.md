<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR uniform blocks read by `ld` from memory

Date 2026-11-16, tip `b72adb59`. Plan #378 item `design-note`. This note is
docs only: no code was changed and nothing was run (no `PCO_SHC`, no board).
Mesa is the 26.2.1 tree (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, MIT; the
note quotes facts from it, not code). Mesa paths are relative to
`src/imagination/`. `[M]` = measured by reading, `[U]` = unknown, not
measured.

Reconciled on 2026-11-16 at tip `372e5184` (plan #378 item `note-reconcile`).
§8 settles the address layout, the block-byte source, the `ld` alignment and
the compiler/back-end interface against the three later studies. Where §3,
§6 or §7 say otherwise, §8 wins.

Kick budget and §8.3 alignment re-counted against the code on 2026-11-16 at
tip `8f019dae` (plan #378 item `note-kicks`): 6 fragment kicks today, 7 with
the pair, 1 spare of 8; 2 vertex kicks today, 3 with the pair.
`src/pvr/backend.rs` and `src/pvr/ffi.rs` are unchanged from §8's tip, so
§8's line numbers hold there too.

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

*How the compiler names it* (§8.4 gives the field's exact place).
- A new record field `ubo_addr_off1: u16` on `ShcShader` (`src/shaderc/mod.rs`,
  0 = the stage reads no block member). The
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
- The kick count per program stays within `PVR_PDS_DMA_MAX_KICKS` = 8
  (`src/pvr/ffi.rs:116`): 6 fragment kicks today, 7 with the pair, 1 spare
  of 8; 2 vertex kicks today, 3 with the pair, 5 spare. Counted in §8.1.
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
   need copy-on-write of in-flight buffers (`docs/notes/gl-buffer-update.md`).
   The alignment is not an obstacle: the 4 we report is what `ld` takes
   (§8.3).
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
4. *(Superseded by §8.2.)* Size the GENERAL arena per draw by the draw.
   The reconcile kept `GEN_PER_DRAW_MAX` as it is and relies on the
   existing full-frame retry instead (§8.2).
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
   - The §8.5 per-draw steps: the snapshot address pair kick, and the
     shared DMA of the prefix only. The prefix is taken when
     `ubo_addr_off1 != 0`; `PVR_BACKEND_UNIFORM_PREFIX` stays off for every
     other record.
   - A snapshot that does not fit returns `-EFRAME_FULL`, not `-ENOMEM`
     (§8.2).
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

## 8. Reconciled decisions (address layout, block bytes, alignment, interface)

Three read-only studies came after §1-§7: A (PDS address writes), B (back-end
arenas) and C (GL bindings). This section checks what they claim against the
tree at `372e5184` and fixes one contract for `ubo-select`, `ubo-chain` and
`ubo-backend`. Line numbers are at that tip.

### 8.1 Address layout: one pair per stage

**Decision.** Each stage gets **one** 64-bit pair. It holds the device
address of the stage's flattened uniform array as this draw reads it. A
member is read at `pair + 4 * dword_off` (§2). There are no per-block pairs.

The claims checked:
- *Study A (a staged table and one DOUTD kick, like the tile buffers).*
  This is true of the tree. The tile-buffer addresses are built as words,
  `arena_put_words(&mut b.gen, .., 8, ..)`, and then sent by one
  `PvrPdsDmaKick { dwords: n * 2, dest_offset: fs.tile_buffer_sh }`
  (`src/pvr/backend.rs:7909-7927`). That kick goes into the same
  `pvr_pds_render_dma_program` call as every other common-store kick
  (`:7929-7937`), so the generator does not change. With one pair per stage,
  "the table" is two words and the kick is the one §3 already describes.
  Study A's mechanism and §3 are the same thing.
- *Kick budget.* The fragment common-store program pushes at most these
  6 kicks today, each behind its own condition: descriptors
  (`:7771-7777`), the uniforms (`:7803-7809`), meta (`:7844-7850`), front
  face (`:7877`), the `gl_FragCoord` pair (`:7901`) and the tile buffers
  (`:7923`). The address pair makes 7 of `PVR_PDS_DMA_MAX_KICKS` = 8
  (`src/pvr/ffi.rs:116`), so **1 spare kick remains** in the fragment
  program, and every future per-draw shared-register write must be budgeted
  against that spare. `dma_push` already refuses a ninth kick
  (`:2417-2426`, `dma_kicks_full` `:2377`). The vertex program pushes 2
  kicks today, descriptors and uniforms (`:8377`, `:8402`), so the pair
  makes 3 of 8, 5 spare. Those two are direct `vk[nvk] = ..` writes — no
  `dma_push` guards the vertex array — so `ubo-backend` keeps the vertex
  kick count in range itself.
- *Why not one pair per block.* With per-block pairs the kick count stays
  the same (one table), but the shared-register cost is 2 per block, up to
  24 for 12 blocks. The addresses would also all point into the same
  snapshot (§8.2), so they would carry no information the member's
  `dword_off` does not already carry. Per-block pairs pay off only when
  blocks are read in place (§8.2, option C). Mesa's 4 dwords per block
  (`{u64 addr; u32 size; u32 offset}`, `vulkan/pvr_common.h:190-196`) serve
  descriptor sets and dynamic offsets, which GL does not have. We need no
  size word either: constant offsets lie inside the snapshot and run-time
  indices are clamped (§2).

### 8.2 Block bytes: the per-draw GENERAL-arena snapshot, as today

**Decision.** The bytes `ld` reads are the per-draw snapshot of the stage
array that the back end already writes into the frame set's GENERAL arena
(`backend.rs:7793-7801` FS, `:8391-8400` VS). The array is copied once. The
default prefix is DMAed out of that copy, and members are read in place in
it by `ld`. There is no new arena and no read from the GL buffer object.

The claims checked:
- *Study B: `GEN_PER_DRAW_MAX` (16 KiB, `backend.rs:450-451`) and the 1 MiB
  arena force a third arena.* The tree says otherwise. `GEN_PER_DRAW_MAX`
  is only the headroom `draw_part` checks before it starts a draw on a frame
  that already has draws (`:6883-6894`). A draw that then runs out of arena
  gets `-EFRAME_FULL` from `arena_put` (`:2450-2459`). `draw_part` flushes
  and retries it on a fresh frame (`:6896-6904`), and refuses it with a log
  line only when it does not fit an empty frame (`:6905-6921`). The default
  GENERAL arena is 1 MiB per frame set (`ARENA_GEN_BYTES`, `:300`; up to
  4 MiB, `:305`). Two full 50688-dword stages are 2 x 202752 = 405504
  bytes, which fits one empty frame. glmark2's largest frame used 516768
  bytes (`:293-298`), so such a draw splits that frame. It is not refused.
- *One real defect in the way.* The FS snapshot turns any `arena_put`
  failure into `-ENOMEM` (`backend.rs:7793-7801`), so the retry never runs
  for it. The VS path passes `rc` on (`:8394-8399`). `ubo-backend` returns
  `rc` at `:7800` too. Without that change a 200 KiB FS snapshot on a busy
  frame is refused instead of split.
- *A third arena* would cost render pool for every context
  (`docs/notes/pvr-render-pool.md`; `FrameSet`, `backend.rs:1527-1546`). It
  would also need a new init and reset path and a new size in
  `pvr_backend_set_frame_arenas` (`:9370-9388`), all to hold bytes the
  GENERAL arena holds already. It is rejected. Study B's other point, "copy
  only the block part", buys nothing here. The DMA source for the default
  prefix must be in memory anyway, and it is the start of the same array,
  so the one copy already serves both reads.
- *Study C: read the bound buffer in place* (`mem.gpu` + the clamped
  offset). This is rejected for now. The flat array is not the buffer's
  bytes:
  - `copy_block_member` reorders row-major matrices
    (`src/gl/program.rs:801-806`).
  - It packs a matrix column into 2 dwords for 2 rows and 4 otherwise
    (`:783-791`), which is not the std140 `matrix_stride`.
  - It turns a bool into 0/1 (`:794-796`, `:811`).
  - It skips a buffer with no CPU mapping (`:749-751`) and reads short
    ranges only as far as they go (`:753-754`, `:810`).

  Reading in place would move every one of those rules into the selector.
  It would also need what Study C lists: per-stage address arrays in
  `GlesDraw` (`src/backend.rs:1091` ff., held by plan #366), a `mem.gpu ==
  0` fallback, a `draw_stamp(StampAt::Buffer)` per block buffer, and a
  larger `DRAW_STAMPS_CAP` (`src/gl/draw.rs:473-476`, held by plan #361).
  The snapshot needs none of that. `fill_uniform_blocks`
  (`program.rs:736-773`) runs on the CPU before the draw is recorded
  (`draw.rs:3908`), so no block buffer is in flight on the snapshot's
  behalf. It also stays shared with the V3D and the GC7000. In-place reads
  stay a later optimisation (§6 option B).
- *The address handed to the pair* is the address the uniform DMA reads:
  `fs_uni_gpu` / `vs_uni_gpu` after the snapshot (`backend.rs:7787-7801`,
  `:8385-8400`). In the one case the GL core gives no host array (FS, the
  variant is larger than the program's array, `draw.rs:3985-3993`), that
  address is the program's own `pg.uniforms` buffer (`draw.rs:3978`). `ld`
  then reads that buffer, exactly as the DMA does today. This settles the
  §6 [U] about the `*_uniform_gpu` path: it needs nothing extra, because
  `pg.uniforms` is sized from the same `room` (`draw.rs:3990-3991`).

### 8.3 Alignment: dword, so `UNIFORM_BUFFER_OFFSET_ALIGNMENT` stays 4

**Rule.** An `ld` address must be 4-byte aligned. No larger alignment is
needed for bursts of 1 to 16 dwords.

The evidence [M by reading Mesa; silicon unconfirmed until §7 step 9]:
- Mesa's PowerVR Vulkan reports `minUniformBufferOffsetAlignment` =
  `PVR_UNIFORM_BUFFER_OFFSET_ALIGNMENT` (`vulkan/pvr_physical_device.c:715`),
  and that constant is 4 (`common/pvr_limits.h:71`; storage is 4 too,
  `:72`).
- pco tells NIR the same (`.min_ubo_alignment`, `pco/pco_nir.c:35`).
- pco merges UBO loads into vectors with `nir_opt_load_store_vectorize`
  (`pco/pco_nir.c:500-504`). Its callback `should_vectorize_mem_cb`
  (`:427-441`) ignores `align_mul` and `align_offset` and refuses only
  `bit_size > 32` and holes. So pco itself emits a 4-dword burst at an
  address that is only dword-aligned.
- The ISA has no alignment field. `ld_immbl`/`ld_regbl` carry only `drc`,
  `burstlen`, `srcseladd` and `cachemode_ld` (§1;
  `src/shaderc/online/powervr/isa_tables.rs:427-428`).

**Effect on GL.** None. We report `UNIFORM_BUFFER_OFFSET_ALIGNMENT` 4
(`src/gl/state.rs:1269`, pinned at `:3220`; checked in
`glBindBufferRange`, `src/gl/es3.rs:1287`), which equals Mesa's. Under §8.2
the reported 4 never reaches the hardware at all: the binding offset is
checked on the CPU (`es3.rs:1287-1291`) and no offset travels to the GPU;
the snapshot is placed at 16 bytes (`arena_put(.., 16, ..)`,
`backend.rs:7797`, `:8395`), and a member's byte offset is
`4 * dword_off`. So every `ld` address is a multiple of 4 — a consequence
of the staging, not a use of the reported value. The value would still be
correct for a later in-place read (Study C). `ubo-backend` asserts
`gpu & 3 == 0` on the pair it writes.

**The pair's own register** stays even (§3). This is a conservative choice,
not a known rule. The two 32-bit reads of the `gl_FragCoord` pair need no
evenness: it sits at `base + uniform_dwords`, whatever its parity
(`src/shaderc/online/powervr/select.rs:220-222`, `chain.rs:759`). Whether a
64-bit `add64_32` source pair needs it is [U], and even costs at most one
register.

### 8.4 The compiler-to-back-end interface

**The record field.** There is one new field on `ShcShader`:
`ubo_addr_off1: u16`. It is the shared register of the stage's address pair
`{lo, hi}` plus one, absolute, the same convention as `fs_front_face_off1`
(`src/shaderc/mod.rs:1338-1341`) and `fragcoord_ytf_off1` (`:1379-1383`).
0 means the stage reads no block member through `ld`.
- *Where it goes.* It goes **after `writes_depth`**, in what is padding
  today. `writes_depth` is at byte 1257 (`src/abi_layout.rs:219`) and the
  next field is at 1264 on every GPU build: `priv_` on the PowerVR and
  Mali, `viv`/`v3d` on the others (`abi_layout.rs:220-225`). A `u16` at
  1258 moves no offset and no size. `writes_depth` was added the same way:
  "In what was padding: no field moved" (`include/gles_shaderc.h:403`).
  `abi_layout.rs` gains `offset_of!(.., ubo_addr_off1) == 1258`, and
  `include/gles_shaderc.h` gains `uint16_t ubo_addr_off1;` after
  `writes_depth` (`:404`).
- *Not a per-block count or table.* One pair per stage (§8.1) makes
  Study A's "base plus one pair per block" and Study B's per-block
  `{offset, size}` list unnecessary. The back end needs the place of the
  pair and nothing else, because member offsets are compiled in.
- *Why not in an existing field.* `priv_` is taken (the table row,
  `src/shaderc/precompiled.rs:287`; the blob, `offline.rs:79`). A
  convention such as "the pair sits after the prefix" cannot tell a stage
  that reads members from one whose trailing default rows are unread, since
  both report `uniform_sh_dwords < uniform_dwords`.
- *Holds.* `src/shaderc/mod.rs`, `src/vivante/**` and `src/backend.rs` are
  held by plan #366, so the field lands after #366 releases `mod.rs` and
  `src/vivante/**`, or in a commit coordinated with it. `src/backend.rs`
  (`GlesDraw`) is **not** changed: §8.2 needs no new draw field.
- *The literal ripple* (CLAUDE.md, the 2026-10-14 breakage). Every full
  `ShcShader { .. }` literal must name the field.
  `grep -rl 'ShcShader {' src tests tools include` names 37 files at
  `372e5184`:
  - *Hand-written:* `src/gl/draw.rs`, `src/pvr/backend.rs`,
    `src/pvr/backend_tests.rs`, `src/shaderc/{link,mod,offline,precompiled}.rs`,
    `src/shaderc/online/{backend,codegen}.rs`,
    `src/shaderc/online/mali/{link,tests}.rs`, `src/v3d/backend_tests.rs`,
    `src/vivante/{backend,mipmap}.rs`, `tests/es2_pvr_ms4_constancy.rs`,
    `tests/es3_fragdepth_v3d_state.rs`, `tests/es3_pvr_modf.rs`,
    `tests/es3_wide128_{flow,join_targets,swizzle,texfetch,vertex}_gc.rs`,
    `tests/es3_wide128_gc.rs`, `tests/gc_es2ctx_random3.rs`,
    `tests/internal_programs.rs`, `tests/online_oracle.rs`,
    `tests/pvr_mipgen_program.rs`.
  - *Generators:* `tools/precompile/src/{drive,table}.rs`,
    `tools/vivante/gen_shader_table.py`, `tools/yakogl-tools/src/emit.rs`.
  - *Generated output,* regenerated in the same commit: `src/pvr/internal.rs`,
    `src/vivante/internal.rs`, `tests/reference/{blobs,gc_clear_shader,vivante_blobs,vivante_blobs_st}.rs`.

  Some files may hold only `..ZERO` updates; `ubo-chain` checks each one.
  The value is 0 everywhere except the online PowerVR record.
  `ShcPrecompiledReflection` (`mod.rs:1813-1839`) does **not** get the
  field: a table shader never reads by `ld`, and `materialize` sets 0.
- *The compiler-side mirror.* `UscProgram::ubo_addr_off: Option<u16>`
  (`src/shaderc/online/powervr/abi.rs:76-111`, beside `meta_off`) is
  written to the record next to `meta_off` (`abi.rs:466-469`).

**What `uniform_sh_dwords` means** (`mod.rs:1360-1363`, offset 1218,
`abi_layout.rs:206`). For an online PowerVR record with
`ubo_addr_off1 != 0`, it is the number of dwords, from dword 0 of the
stage's flat array, that the back end DMAs to `sh[desc_dw ..]`. It is never
0 in that case. It covers:
- every default-block dword the program reads, whether by a constant or by
  `idx0`;
- the blend constants, when the variant reads them (`blend_consts_off + 4`,
  `chain.rs:767-783`).

Block members at or past it are read only by `ld`. A member row below it (§5
[U] on the row order) is DMAed but never read from the shared registers. In
every other record the field keeps today's meaning, "0 = the whole block",
and the back end keeps ignoring it: `PVR_BACKEND_UNIFORM_PREFIX` stays
`false` (`backend.rs:286-292`). It was switched off for a pco table's count
that did not bound its reads. The online compiler zeroes the field today
(`abi.rs:550`); the new count is computed from the reads.

**The shared layout the compiler builds.** Today the `gl_FragCoord` pair,
meta and front face are placed after the **whole** flat array:
`ytf_dword = m.uniform_dwords`, `meta_dword = m.uniform_dwords + ..`,
`ff_dword = meta_dword + ..` (`select.rs:220-242`), and then `base +` each
(`chain.rs:757-760`). With a 4096-dword block that is past sh2048. So
`ubo-select`/`ubo-chain` rebase all three on the shared prefix:
- `base = TEX_DESC_DWORDS * samplers`;
- the prefix occupies `[base, base + uniform_sh_dwords)`;
- `ubo_addr_off1 - 1 = align2(base + uniform_sh_dwords)`;
- the `gl_FragCoord` pair, meta and front face follow, in that order.

The blend-constant guard (`chain.rs:776-780`) then tests the prefix, not
`m.uniform_dwords`.

### 8.5 What the back end does per draw (each stage, `ubo_addr_off1 != 0`)

1. Snapshot the whole `*_uniform_dwords` array into GENERAL as today. Pass
   `arena_put`'s `rc` on (§8.2).
2. DMA `[0, uniform_sh_dwords)` of it to `sh[desc_dw ..]` instead of the
   whole array (`backend.rs:7788-7810`, `:8386-8405`).
3. Put `[gpu as u32, (gpu >> 32) as u32]` with `arena_put_words(.., 8, ..)`
   and push `PvrPdsDmaKick { dwords: 2, dest_offset: ubo_addr_off1 - 1 }`.
   Raise `shared_dw` to `ubo_addr_off1 + 1`, as the tile-buffer and
   `gl_FragCoord` kicks do (`:7904`, `:7926`). For the VS, raise
   `vs_shared_dw`; `usc_common_size_bytes` is taken from it (`:8430`).
4. Leave `GEN_PER_DRAW_MAX`, `ARENA_*`, `PVR_PDS_DMA_MAX_KICKS` and the PDS
   generator unchanged.

### 8.6 Files the design reaches

- **Compiler:**
  - `src/shaderc/online/powervr/select.rs`: member ranges, the §2
    sequences, rebasing `ytf_dword`/`meta_dword`/`ff_dword`.
  - `src/shaderc/online/powervr/prepare.rs`: immediates, and `idx0` for a
    pair at sh256 or above.
  - `src/shaderc/online/powervr/{sel,encode,legalise,isa,assemble}.rs`:
    `ld`, `add64_32` and `wdf` (`ld-encode`).
  - `src/shaderc/online/powervr/alloc.rs` and `verify.rs`: the `ld`
    destination at the `wdf`.
  - `src/shaderc/online/powervr/chain.rs`: member rows, the layout of
    §8.4, `uniform_sh_dwords`; held by #361.
  - `src/shaderc/online/powervr/abi.rs`: `UscProgram::ubo_addr_off`, the
    record write.
  - `src/shaderc/mod.rs`, `src/abi_layout.rs` and
    `include/gles_shaderc.h`: the field; `mod.rs` is held by #366.
  - The literal files of §8.4.
- **`src/pvr/**`:** only `src/pvr/backend.rs` (§8.5) and
  `src/pvr/backend_tests.rs` (kick-list tests), both held by #361 and #350,
  plus the regenerated `src/pvr/internal.rs`. `src/pvr/ffi.rs` is **not**
  reached: no kick or arena constant moves.
- **GL core:** `src/gl/context.rs` only, for the `ctx-limits` cfg arms
  (§7 step 5). Its `MAX_STAGE_UNIFORM_DWORDS` arm sizes the arrays in
  `src/gl/program.rs` (`block_dwords`, `:1881`) and `src/gl/draw.rs`
  without a code change there. `src/gl/draw.rs` only gains the
  `ShcShader` literal field. `src/backend.rs` (`GlesDraw`) and
  `DRAW_STAMPS_CAP` are not reached.
- **PDS generator:** `driver/src/pvr_pds_render_gen.{c,h}` is **not**
  reached. The pair is one more DOUTD kick in the existing program
  (`backend.rs:7929-7937`).

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
- `pco/pco_nir.c`: `.min_ubo_alignment` `:35`, `should_vectorize_mem_cb`
  `:427-441`, `nir_opt_load_store_vectorize` `:500-504` (§8.3);
  `common/pvr_limits.h:70-72`; `vulkan/pvr_physical_device.c:715` (§8.3).
- `vulkan/pvr_physical_device.c:626,635,642-643`;
  `vulkan/pvr_common.h:190-196`; `vulkan/pvr_arch_pipeline.c:474-530`;
  `vulkan/pvr_arch_cmd_buffer.c:5567-5600`.
