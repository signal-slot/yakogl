<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: the path to a 16384-byte uniform block

Date 2026-11-16, tip `b72adb59`. Plan #378 item `study-gc-ubo`. Docs only:
nothing was changed in code and nothing was run (no board, no host test).
Continues `docs/notes/ubo-limits.md` §5 (read it first) and
`docs/notes/gc-ubo-const-limit.md` §7. Mesa is the 26.2.1 tree
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, MIT; facts only quoted).
`[M]` = measured by reading the code at the tip, `[U]` = unknown, not
measured.

## 1. Verdict

**Most of the compiler path to 16384 bytes already exists; what is missing
is the raise itself and the one silicon fact.** Block members whose whole
constant register is block-only are already read from memory (`mov t.x,
byte_offset` + `load.u32` from the `SHC_VIV_CONST_UBO_ADDR` slot) and the
back end already hands a block over 1280 dwords on in place
(`gc-ubo-const-limit.md` §7). The GC7000UL is limited to 6144 bytes only by
the GL core's constants, which is why the raise is a `context.rs` arm plus
proof, not a new read path. Recommended shape: **keep one address per stage
(the flattened array, F below)**; a per-block address (P below) is the Mesa
shape and a later optimisation, not a prerequisite.

## 2. What is already in place [M]

| piece | where |
|---|---|
| block-only registers leave the constant file (`reg_of`, `NOT_IN_FILE`) | `src/shaderc/online/select.rs:1810-1853` (`uniform_block`) |
| `reads_block_memory(dword_off)` | `select.rs:1864-1866` |
| the read: `mov t.x, imm(dword_off*4)`, `load.u32 got, t.x, u[ubo_addr]`, then `mov` with the narrower-uniform swizzle | `select.rs:1897-1940` (`block_read`), dispatched at `:2842-2844`; the constant-file path is kept for mixed registers at `:2109` |
| run-time index: clamp, `imullo0`, `add`, `load.u32` | `select.rs:3014-3100` (`LoadUniformIdx`) |
| the address slot, one per stage, `data` 0 | `select.rs:2753-2767` (`ubo_addr_reg`) |
| `reads_ubo`, `ubo_copy` (whole stage array into the frame arena, +16 zero bytes) | `src/vivante/backend.rs:3007-3025`, called at `:3627-3638` |
| slot value | `src/vivante/shader.rs:514-516` (`const_value`: `vs_ubo_gpu` / `fs_ubo_gpu`) |
| a stage over 1280 dwords is handed over in place | `backend.rs:3100-3114` (`take_uniforms`), test `a_uniform_block_past_the_constant_registers_is_taken_whole` (`src/vivante/backend_tests.rs:5597-5620`) |
| arena room for both copies, reserved from `gl.*_uniform_dwords` (a `u16`) | `backend.rs:2995-2999` |
| host interpreter understands `load` with memory entries | `tests/common/mod.rs:274-300` |
| unit test of the member read | `select.rs:6043` `block_members_are_loads_and_leave_the_constant_file` |

Why one address is enough today: the GL core flattens every block into the
stage's array behind the default rows (`program.rs:678` `fill_uniform_blocks`,
`ubo-limits.md` §2), so block 0's "address" is the address of the whole
array and a member is `dword_off * 4` from it. That is also the V3D's
model (`context.rs:94-131`).

## 3. What must move from the constant file to `load` for a block over 1280 dwords

The constant file is 320 vec4 = 1280 dwords, shared by both stages
(`src/vivante/state.rs:24`, `shader.rs:666-667`).

1. **Block members.** Every read of a block-member register. Already done by
   `uniform_block` when *all* real dwords of the register are members. A
   16384-byte block is 4096 dwords = 1024 registers, so only a read of one
   of the (at most two) boundary registers can mix members with default
   uniforms. Those registers stay in the file and `const_value` fills them
   from the stage array (`shader.rs:517+`), which still works because the
   array is in place (`take_uniforms`). Nothing to move, but **the file must
   hold the default rows and the boundary registers**: unchanged by size.
2. **Run-time indices into a member array.** Already `LoadUniformIdx`.
3. **Default-block uniforms stay in the file.** They are bounded by the
   384-row Appendix A budget (`MAX_UNIFORM_DWORDS`, stays 1536) but the
   GC's file is 320 registers for two stages: the existing
   `constant_files_fit` refusal (`shader.rs:644`) keeps applying, and is
   unrelated to the block size. Do not change it.
4. **Literals** are placed after the remaining uniform registers
   (`first_literal`, `select.rs:1852`) and now cost nothing extra for
   blocks.
5. **Offsets.** `block_read` builds the offset with `imm_s20` (signed 20
   bits, +-524287 bytes): `dword_off` is a `u16`, so the largest offset is
   262140. The full formula array (50688 dwords, 202752 bytes) fits; no
   change. The `LoadUniformIdx` bound and stride immediates fit the same way.

So **no selector change is required for F.** The one thing the selector
cannot say is whether the hardware `load` takes a byte offset above 64 KiB
(a flat array of twelve blocks reaches 196 KiB) [U], see §8.

## 4. How each block's address reaches the shader

**F (recommended): one address per stage.** The slot is
`SHC_VIV_CONST_UBO_ADDR` with `data` 0, the address of the stage's flat
array copy; a member's offset is its row in that array. No new constant
kind, no new `GlesDraw` field, no `YakoglBackend` literal to chase. Costs:
`ubo_copy` copies the whole array (up to 50688 dwords = 198 KiB a stage,
396 KiB for both) into the 1 MiB frame arena every draw
(`GC_ARENA_BYTES`, `backend.rs:90`); `staging_need` reserves the same
(`:2995-2999`) and `full = ... arena_used + need > GC_ARENA_BYTES`
(`:3322-3325`) ends the frame early, and `big` goes to `ensure_extra`
(`:3322`). Programs that lay out little still copy little: the array is
sized by what the stages lay out, not by the cap (`program.rs:1889`,
`draw.rs:2336-2342`).

**P (the Mesa shape, later): one address per bound block.** Mesa gives every
user block its own `ETNA_UNIFORM_UBO_ADDR` whose value is the bound
buffer's BO address plus the binding offset (`etnaviv_uniforms.c:183-189`,
`etnaviv_compiler_nir.c:644-655`, `gc-ubo-const-limit.md` §2 items 3-4), and
reads the block's own offset (at most 16384), with the default block as
UBO 0. For us P means:

- `ConstSlot { contents: SHC_VIV_CONST_UBO_ADDR, data: block_index }`
  (`ubo_addr_reg` today hard-codes `data: 0`, `select.rs:2754`; take the
  block as a parameter and place one slot per distinct block).
- `select_blocks` gets the block of each member range, not only
  `(dword_off, dwords)` (`codegen::assemble_from_ir_blocks`,
  `reflection.block_members`), so `block_read` can subtract the block's base
  row from `dword_off` and pick its slot.
- `GlesDraw` carries per stage and block the buffer GPU address and the
  offset (12 per stage), the GL core stops flattening for this GPU
  (`fill_uniform_blocks`, `draw.rs:3908-3913`), and `const_value` reads
  `data` (`shader.rs:514`).
- Gain: no per-draw copy of 16 KiB x 12; offsets stay under 16384 as
  Mesa's. Cost: a `GlesDraw` ABI change reaching every back end
  (`YakoglBackend`-style literals in `tests/`), the GL core and the host
  array copy it must keep for the other GPUs; too much for the raise itself.

Decision: ship F, measure, take P only if a dEQP case or a frame shows the
copy cost (or the board shows [U] of §8 failing above a small offset, in
which case P is the fix, since its offsets are the ones Mesa ships).

## 5. What the back end must change (F)

Nothing for correctness; only checks and the cap:

1. `take_uniforms` already has no size bound other than `u16`
   (`backend.rs:3100`). Keep the null/unaligned refusal.
2. `ubo_copy` returns 0 when the arena cannot hold the copy, and the draw is
   refused (`backend.rs:3627-3638`). With 396 KiB per draw the 1 MiB arena
   holds two such draws a frame; `full` (`:3322-3325`) must flush before it,
   and `staging_need` already counts the copy. Check that a draw needing
   more than the arena lands in `ensure_extra` rather than `return -1`
   (the `big` path, `:3322-3335`): a 50688-dword stage plus attribute data
   can exceed `GC_ARENA_BYTES`. Test it (§7 item 4).
3. Only the slot's reader is per draw: `reads_ubo` false means no copy, so
   a program with a default block and no member reads copies nothing.
4. `src/gl/draw.rs:2336` sizes the program's staging buffer from the layout;
   `uniforms_host_only: true` (`backend.rs:3887`) means the GC does not use
   it, so a raised cap must not allocate it larger than the layout needs
   (it does not: `min(MAX_STAGE_UNIFORM_DWORDS)` is an upper bound only).

## 6. Ordered change list

Each step keeps the tree green. `src/vivante/**` is held by plan #366 until
it releases it; steps 3 and 6 touch it (tests only in step 3).

1. `tests/gc_ubo_large_block.rs` (new, host, `gpu-vivante,shaderc-online`,
   §7 items 1-3). Compile and run with the lane-faithful interpreter of
   `tests/common/mod.rs:282` a 16384-byte block read at the first and last
   dword by constant and by run-time index. Green before any raise: it
   pins that §2 already holds at the real size.
2. `src/shaderc/online/select.rs` only if step 1 finds a failure (for
   example the `arena.alloc_filled(regs * 4)` or `table` allocations of
   `uniform_block`, `:1816-1835`, exceeding the selector's arena at 12672
   registers [U]); then a unit test beside `:6043`.
3. `src/vivante/backend_tests.rs` (near `:5597`): a draw whose stage block
   is 4096 dwords: `ubo_copy` holds all of it, the slot names it, and the
   draw falls to `ensure_extra`, not `-1`, when the copy does not fit the
   frame arena. Held by #366: wait.
4. `src/gl/context.rs:112` and `:131`: add `gpu-vivante` beside `gpu-v3d` in
   the two `cfg` arms under `!report-es2` (the `MAX_UNIFORM_BLOCK_SIZE` arm
   at `:109-112`, the `MAX_STAGE_UNIFORM_DWORDS` arm at `:124-131`), so
   `MAX_UNIFORM_BLOCK_SIZE` is 16384 and the combined components
   `limits.rs:40` `combined_uniform_components(1536, 12, 16384)` = 50688.
   `MAX_UNIFORM_DWORDS` and `MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` stay
   1536. The `u16` assert at `:135` already passes (50688 + blend consts <
   65535). Its stale comment (`:104-107`, "the GC7000 holds the blocks in its
   constant file") is corrected in the same commit. This waits for the board
   run of step 7 (see §8) and for plan #375's `tests/es3_limits_ubo_unchanged.rs`.
5. `tests/es3_limits_ubo_unchanged.rs` (GC rows of its `gpu-vivante`
   feature set) and a new `tests/es3_limits_gc_ubo.rs` (model:
   `tests/es3_limits_v3d.rs`): 16384 / 50688 through `glGetIntegerv`,
   `glGetFloatv`, `glGetInteger64v`. The same commit as step 4.
6. `tests/vivante_imx8mp_limits.rs`: pin the new values if it asserts a
   uniform limit (it mentions `uniforms_host_only` at `:416`; check
   `grep -n UNIFORM_BLOCK tests/vivante_imx8mp_limits.rs`).
7. Board step (§8), then `tests/es3_caps.rs` `known_gaps()` drops the GC
   rows (after #366 releases it), `ubo-limits.md` §5 and the RC10 clause of
   `docs/es3-cts-failures.md`, `tools/cts/RESULTS.md` (current row only).

## 7. Tests

1. **Constant, first and last dword** of a 16384-byte `std140` block
   (`uniform B { vec4 v[1024]; };` read at `v[0]` and `v[1023]`): compile
   with `gpu-vivante`, check the register is not in the file
   (`reads_block_memory`), that the `load` offset is `0` and `16368`, and run
   the encoded program in `tests/common/mod.rs:282` with a memory entry of
   the block (+16 bytes of tail as `ubo_copy` makes) for the value.
2. **Run-time index** over the same array (`v[i]`): `LoadUniformIdx`'s clamp
   and `imullo0`, for `i` = 0, 1023 and an out-of-range one.
3. **Twelve blocks**: block 11's last dword is at dword 49151 + default
   rows; `dword_off` and the `imm_s20` offset are in range (§3 item 5).
4. **Host draw** (the backend host rig of `src/vivante/backend_tests.rs`,
   `Rig`): a full block, the slot's value is the copy's address, the copy
   holds dwords 0 and 4095, no fault; and a draw that needs more than the
   arena.
5. **Report**: `tests/es3_limits_gc_ubo.rs` (§6 item 5); `es2_report_guard`
   (`cargo test --features report-es2 --test es2_report_guard`) unchanged:
   no ES 2.0 shader has block members.
6. **dEQP (host-only, `tools/cts/host-deqp-run.sh run`, not edited)**:
   `functional.ubo` and the `implementation_limits` uniform-block cases on
   the `gpu-vivante` build, before and after step 4. The four
   `single_nested_struct_array` cases already pass on the host
   (`tests/es3_ubo.rs`, `KNOWN_LIMITS` empty).

Feature sets in each test header:
`--no-default-features --features host-test,gpu-vivante,shaderc-online`.

## 8. The board check it needs (i.MX8MP)

`gc-ubo-const-limit.md` §6 and §7 still say it: **the i.MX8MP run of a
constant-offset `load` from a user block has never been done.** The
silicon-proven reads are the run-time-indexed ones (gl_test 108, 128,
`VIVANTE_NOTES.md` "A uniform array at a run-time index is a load from
memory"). The raise must not reach the report before this passes.

New gl_test case (name `ubo16k`): a vertex or fragment shader reading from
one 16384-byte std140 block with **constant** indices `v[0]`, `v[1]`,
`v[1022]`, `v[1023]` and a **run-time** index at the same elements, in both
stages, plus a second block (so the flat offset passes 16384 and, with 12
blocks bound, 64 KiB: the [U] of §3). The buffer is filled with a
fingerprint (the dword number); the case checks the colour. Run it as
`VIVANTE_NOTES.md` ("Board recipe") does:
`tools/cts/imx8mp-gltest-build.sh ubo16k 0 "<case list>"` and
`SEL=<case> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run <zephyr.bin> ...`,
then dEQP `functional.ubo` and `implementation_limits` on the board.
The `tools/vivante` `cs_golden` mode 25 (Mesa under `drm-shim`,
`ETNA_SHIM_GPU=GC7000UL`) is the reference for the stream shape. Boards are
held by plan #347: the step stays pending until it releases them. If the
offsets above 64 KiB fail, the fallback is P (§4), whose offsets Mesa ships
(at most 16384).

## 9. Risks

- The silicon fact of §8 [U]; the 32-bit address (`d.vs_ubo_gpu as u32`,
  `shader.rs:515`) is the GC's address space.
- Per-draw copy cost of F (up to 396 KiB) and the early frame flush it
  forces; measure on a frame that uses a full block, not on dEQP.
- The selector's arena for a 12672-register table [U] (step 2 of §6).
- `docs/notes/ubo-limits.md` §5 says the raise "waits for #335/#355";
  both are named there, not re-checked here.

## 10. Issue text (for the planner; agents do not run `gh`)

Title: `GC7000UL: raise MAX_UNIFORM_BLOCK_SIZE to 16384 and combined uniform components to 50688 (ES 3.0, RC10)`

Body:

> The i.MX8MP (GC7000UL) reports `MAX_UNIFORM_BLOCK_SIZE` 6144 and combined
> uniform components 1536; ES 3.0 asks 16384 and 50176/50048
> (`docs/es3-cts-failures.md` RC10, `docs/notes/ubo-limits.md` §5). The read
> path is already memory `load` for block-only registers
> (`select.rs` `block_read`, `backend.rs` `ubo_copy`), so the raise is
> design `docs/notes/gc-ubo-block-raise.md`:
> 1. `tests/gc_ubo_large_block.rs` host tests (16384-byte block, first/last
>    dword, constant and run-time index, twelve blocks);
> 2. backend test that a 4096-dword stage copy is whole and an oversized
>    draw takes `ensure_extra` (needs `src/vivante/**`, plan #366);
> 3. i.MX8MP gl_test `ubo16k`: a constant-offset `load` from a user block
>    (never run on silicon) at the first/last dword, past 16 KiB and past
>    64 KiB; dEQP `functional.ubo` and `implementation_limits`;
> 4. only then `context.rs` arm for `gpu-vivante` (16384 / 50688),
>    `tests/es3_limits_gc_ubo.rs`, the GC rows of `es3_caps.rs`
>    `known_gaps()`, `ubo-limits.md` §5, RC10, `tools/cts/RESULTS.md`.
> Optional later: per-block addresses (Mesa shape) if the flat copy costs.
> Board step waits for the boards (plan #347).
