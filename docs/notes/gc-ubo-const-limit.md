<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: four dEQP-GLES3 ubo cases need more than 320 constant registers

Date 2026-10-14, tip `cd738b1ae7cf`. Plan #144 item `gc-ubo-const`. Investigation
only: no file under `src/` or `tests/` changed (the temporary prints used for
the measurements were removed again; `git status` was clean before this note).

## Verdict

**The 320 is a real limit of the silicon. These four cases are still fixable,
because the limit is hit by our design, not by what the cases need.**

Our code generator copies every uniform-block member into the constant file.
Mesa etnaviv never does that for a user block: it reads user blocks from
memory with `LOAD`. With the same path here, each of the four cases needs
about 77 registers per stage (154 of 320) instead of 184 per stage (368).
The path exists already (`SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`, proven on the
i.MX8MP for dynamic indexing); it is used only for run-time indexes today.

## 1. Measured demand (host, `gpu-vivante`)

Command (the three-line print was added in `link_checked`, `select.rs`
`uniform_src` and the test's known-limit arm, then removed):

```text
cargo test --release --no-default-features \
  --features host-test,gpu-vivante,shaderc-online \
  --test es3_ubo structs_single_nested_struct_array -- --nocapture
```

All four cases give the same numbers, in both stages (a case declares the
block in the vertex and the fragment shader, "both"):

| case (`single_nested_struct_array.`) | vertex regs | fragment regs | total | limit |
|---|---|---|---|---|
| `per_block_buffer.shared_instance_array_both` | 184 | 184 | 368 | 320 |
| `per_block_buffer.std140_instance_array_both` | 184 | 184 | 368 | 320 |
| `single_buffer.shared_instance_array_both`    | 184 | 184 | 368 | 320 |
| `single_buffer.std140_instance_array_both`    | 184 | 184 | 368 | 320 |

Link log of each: `the constant registers the two stages need (vertex 184,
fragment 184: uniforms and literals) exceed the 320 this GPU has`.

What the 184 registers (736 dwords, `const_count`) are, per stage:

| part | dwords | registers |
|---|---|---|
| uniform rows: the block `Block block[3]` copied in (`uniform_dwords`) | 429 (+3 pad) | 108 |
| literal area: dEQP's expected values baked into the shader | 304 (167 literal + 137 unused lanes) | 76 |
| `SHC_VIV_CONST_UBO_ADDR` slots | 0 | 0 |

Other numbers of the same compile:

- 126 uniform reads per stage (`uniform_src` calls), every one a
  constant-offset read, so each costs no instruction today.
- 1232 (vertex) and 1237 (fragment) instructions.
- The literal area holds only **20 distinct values** in 76 registers: 59
  registers hold 2 values, 16 hold 3, 1 holds 1, none 4. A literal tuple
  (e.g. an `ivec3` compare) must sit in one register, so tuples that differ
  take separate registers.

Why `block[3]` and not `block`: `tests/es3_ubo.rs` `struct_case` builds
`S s; vec2 v; T t[2]; uint u` with `T { mat3 a; S b[3]; }`,
`S { ivec3 a; ivec2 b[4]; vec4 c; }` and an instance array of 3. The members
`S.c` and `v` are unused. 429 active dwords is the sum of the three copies.

## 2. What Mesa etnaviv does (release 26.2.1, the tag the repo already uses)

Source: `$CCFARM_DEPS_DIR`'s `mesa-src/mesa-26.2.1` (tag `mesa-26.2.1`; the
`deps/yakogl/mesa` checkout is a 26.3.0-devel tree and was used only to
cross-check). Paths are under `src/gallium/drivers/etnaviv/` unless noted.

1. `etnaviv_compiler.c:66`: `.lower_uniforms_to_ubo = info->halti >= 2`. The
   GC7000UL is HALTI5, so every default-block uniform becomes `load_ubo` of
   **UBO 0** (`src/compiler/nir/nir_lower_uniforms_to_ubo.c:145` makes the
   room by `num_ubos++`: real UBOs move up by one, to 1 and above).
   `src/mesa/state_tracker/st_glsl_to_nir.cpp:714` calls the pass.
2. `etnaviv_nir_lower_ubo_to_uniform.c:30-48`: "lower the load_ubo intrinsics
   **for block 0** back to load_uniform"; `if (block > 0) return false` (line
   47). Only a constant block index of 0 with a constant offset returns to the
   constant file. A user block (index >= 1) is never lowered.
   `etnaviv_compiler_nir.c:1315` runs the pass.
3. `etnaviv_compiler_nir.c:644-655` (`nir_intrinsic_load_ubo`): emits
   `ISA_OPC_LOAD`, type U32, `src[0]` = the byte offset, `src[1]` = a constant
   of kind `ETNA_UNIFORM_UBO_ADDR` with the block index. There is no
   constant-file path in this case.
4. `etnaviv_uniforms.c:183-189`: `ETNA_UNIFORM_UBO_ADDR` is filled with a
   relocation to the bound constant buffer's BO plus its offset. So the GPU
   reads the app's buffer in place; nothing is copied.
5. `etnaviv_screen.c:164,183,196`: `ubo_enable = halti >= 2`;
   `max_const_buffers = ETNA_MAX_CONST_BUF` (16,
   `etnaviv_internal.h:45`); `max_const_buffer0_size` 16384 "so state tracker
   enables UBOs".
6. Constant-file size: `etnaviv_screen.c:793-796` (HALTI1+, unified):
   `max_vs_uniforms = max_ps_uniforms = MIN2(512, num_constants - 64)`.
   The hwdb entry for the i.MX8MP part, `gc7000ULN_v123` (ChipID 0x7000,
   revision 0x6204), has `NumberOfConstants` 0x140 = **320**
   (`src/etnaviv/hwdb/nxp/gc_feature_database.h:18853`). The two stages share
   the file: `etnaviv_emit.c:790-792` sets `VS_UNIFORM_BASE` 0 and
   `PS_UNIFORM_BASE` to the vertex stage's `uniforms.count / 4`. So 320 is the
   sum, as `constant_files_fit` assumes (`src/vivante/shader.rs:531`).
7. Shader size: `ETNA_MAX_TOKENS` 2048 (`etnaviv_compiler.h:44`) is the
   instruction cap Mesa advertises.

Conclusion for the GC7000 (HALTI5): Mesa **puts default-block uniforms in the
constant file and every user UBO in memory**, read with `LOAD`. A dEQP ubo
case never competes with the constant file for its block data on Mesa; only
its literals do.

Not checked: I did not run Mesa itself on these four cases (no board or
drm-shim run in this item). That Mesa's path passes them on silicon is
inferred from the source, not measured.

## 3. How this repo handles a block today

- `src/gl/program.rs:663` `fill_uniform_blocks`: before a draw, each bound
  block's bytes are copied member by member into the stage's default-block
  uniform rows (`copy_block_member`). From there a member is an ordinary
  uniform row.
- `src/shaderc/online/select.rs:1388` `uniform_block`: pushes
  `ConstSlot::uniform(d)` for **every** dword of `uniform_dwords`, rounded up
  to a register; `const_reg` (line 1449) starts literals after that
  (`first_literal`).
- `select.rs:1409` `uniform_src`: a `LoadUniform` is a constant-file operand,
  register `dword_off / 4`.
- `select.rs:2306` `LoadUniformIdx` already emits the Mesa shape
  (`imullo0 / add / load.u32 v, t.x, u[ubo_addr]`); `ubo_addr_reg` (line 2084)
  places the `SHC_VIV_CONST_UBO_ADDR` slot.
- `src/vivante/backend.rs:2203` `reads_ubo` sees that slot; `ubo_copy`
  (line 2209) copies the stage's whole block into the frame arena with 16 zero
  bytes after it; `const_value` (`src/vivante/shader.rs:401`) writes that
  address into the slot. Arena room for both blocks is reserved at
  `backend.rs:2193`.
- `src/shaderc/online/backend.rs:1358` `link_checked` is where the 368 > 320
  is refused.

So the missing piece is only a choice in `select.rs`: read block-member rows
with `load` instead of from the constant file, and do not reserve constant
registers for them.

## 4. Options, ranked

### A (recommended): block-member rows are read from memory, as Mesa does

Every `LoadUniform` of a dword that belongs to a block member becomes
`mov t.x, imm(byte offset)` then `load.u32 v, t.x, u[ubo_addr]`; those dwords
get no constant-file registers.

- Demand after: 76 literal + 1 UBO-address register = **77 per stage, 154
  total** (not measured after a change; computed from the table above).
- Instructions: +2 per read; 126 reads per stage is +252, about 1490
  instructions, under the 2048 Mesa advertises. Not measured.
- Same path as the dynamic index, which passes gl_test 45 and dEQP indexing
  on the i.MX8MP (`src/vivante/VIVANTE_NOTES.md` "uniform_array", line 530
  on). Constant-offset `load` from a user block is a new use of it:
  **needs one board run** (recipe below).
- Benefit beyond these four: any block-heavy program stops competing for the
  320 registers (`ubo.random`, big instance arrays).

### B: spill only what does not fit

Keep constant-file reads while the program fits, and put the rest in memory.
Fewer instructions for small blocks, but two read shapes, a decision at link
that needs both stages' sizes (`link_checked` runs after both compiled), and a
second compile. More code for a smaller win. Pick it only if A is too slow in
a measured case.

### C: pack the literals better

The literal area is 76 registers for 20 distinct values. With perfect packing
(4 per register) it would be 42. The fit needs
(320 - 216) / 2 = 52 registers per stage for the literals alone, with all 108
uniform registers kept. That is 80 % filling of registers whose tuples are
fixed by the shader; the present first-fit gets 55 % (167 of 304). It is a
gamble: tuples of 2 and 3 distinct values force whole registers. It also helps
nothing when a case has more block data. Do it as a separate, smaller item.

### D (not recommended): raise or ignore the limit

320 is the hardware's `NumberOfConstants`. A draw past it reads registers
that do not exist. `constant_files_fit` is right to refuse.

## 5. Fix design (option A)

No change in `src/gl/**` (fill_uniform_blocks already fills the rows the
memory copy needs). No change in `lower.rs`, `sema.rs`, `reflect.rs` (held by
plans #18/#19); the design avoids them. Size estimates are lines of Rust.

1. **Which dwords are block members** (`src/shaderc/online/codegen.rs`, ~25
   lines). `compile_*_blocks` already return `Blocks` (the block members with
   their uniform-table `row`). Build the list of `(dword_off, dwords)` ranges
   of those rows from `reflection.uniforms[row]` and hand it to
   `assemble_from_ir` (which already takes the uniform rows) as a new
   parameter. A new field on `ir::Module` is the alternative but
   `lower.rs:931` builds the module and is held; pass it beside the module.
2. **select.rs** (~80 lines, `src/shaderc/online/select.rs`; not under
   `vivante/**`, so it may collide with item `gc-const-array`, which edits
   `vivante/**` and may touch the same functions: do this after it merges).
   - `uniform_block` (1388): push `ConstSlot::uniform(d)` only for registers
     that hold at least one non-member dword. Keep a `reg_of[old_reg]` map
     (u16 per register, arena-allocated) so `uniform_src` and
     `first_literal` use the packed numbers. `ConstSlot::uniform(d)` already
     carries the block dword, so the backend writes the right value into the
     moved register with no change.
   - `uniform_src` (1409): if the dword is a member dword, emit
     `mov t.x, imm(dword_off*4)` + `load.u32 got, t.x, u[ubo_addr]`, then the
     usual swizzle (copy the `LoadUniformIdx` tail, lines 2362-2410, into one
     helper both use). `RGROUP_IMMED` is not allowed for `load`
     (`vivante/build.rs:364`), so the offset needs the `mov`. A read of a
     mixed register (default uniform and member dwords) is not split: the
     register stays in the file and is read as today; only an all-member
     register leaves it.
   - `uniform_src` returns a `SelSrc` today and emits nothing; it must become
     `&mut self` with a `Virtual` result. Callers: `select.rs:1607` and the
     matrix/column paths; check by compiling.
3. **Driver state** (`src/vivante/backend.rs`, `src/vivante/shader.rs`; ~30
   lines):
   - `take_uniforms` (src/vivante/backend.rs) refuses by name a block over
     `GC_MAX_UNIFORM_DWORDS` = 1280 dwords (5 KiB per stage, `state.rs:24`);
     the draw returns -1, the reason is logged once, and the refusal is
     counted. Until then the dwords past the cap have nowhere to go; the
     refusal is lifted when this item lands. The memory copy must hold the
     whole block: copy `uniform_dwords` straight from the GL block into
     `ubo_copy` (no cap; the GL side bounds it and `gl.*_uniform_dwords` is
     `u16`, up to 256 KiB). The static `UNIFORM_BLOCKS` stays for the dwords
     the constant file still needs.
   - `ubo_copy` size: 429 dwords = 1.7 KiB + 16 bytes per stage per draw
     (arena room already reserved from `gl.*_uniform_dwords`, line 2193).
   - `emit_stage_uniforms` / `constant_files_fit`: no change; both count
     `const_count`, which gets smaller.
4. **Tests** (the fixing item, not this one): take the four cases off
   `KNOWN_LIMITS` (`tests/es3_ubo.rs:1490`; the test fails if one passes
   while listed). Add to `src/vivante/backend_tests.rs` (near line 1493,
   "copy of its block in memory") a program that reads a constant-offset
   block member and check the `load` words and the slot; add
   `tests/gc_const_map.rs` coverage that a block-only register leaves the
   file. Run `tests/es2_report_guard.rs`: no ES 2.0 shader has block members,
   so `uniform_block` must produce the identical table for them.

Risks: (a) `load` of a constant offset on silicon is unproven (see below);
(b) the +2 instructions per read; (c) `select.rs` collisions with
`gc-const-array`.

## 6. Verification recipe for the fix

Host, gpu-vivante (no board needed for the register count):

```text
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
  --test es3_ubo
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
  --test es3_shaderc_corpus
cargo test --features report-es2 --test es2_report_guard
```

Expected: `es3_ubo` passes with the `KNOWN_LIMITS` list empty (the four cases
link, draw, and compare rows). Before the change those four log
`vertex 184, fragment 184`; after, the link must not fail, and the stage
numbers should be about 77 and 77. If a stage is above 100, the block is
still in the file.

Silicon (the new part is a constant-offset `load` from a user block): add
one i.MX8MP gl_test case that draws with a std140 block read at constant
offsets (`tools/vivante` cs_golden mode 25 shows Mesa's stream for the dynamic
case; compare the new case's words with Mesa 26.2.1 under drm-shim the same
way, `ETNA_SHIM_GPU=GC7000UL`), then run it as `VIVANTE_NOTES.md` ("Board
recipe") does: `tools/cts/imx8mp-gltest-build.sh <name> 0 "<case list>"` and
`SEL=<case> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run <zephyr.bin> ...`.
Expected: the case reports PASS and the record shows the expected colour for
every block member. I did not run a board session in this item, so no board
result is claimed here.

## 7. Status (2026-10-27): option A implemented on the host

- `select::select_blocks` takes the `(dword_off, dwords)` ranges of the
  uniform rows that are block members (`codegen::assemble_from_ir_blocks`,
  from `reflection.block_members`). A constant register all of whose real
  dwords are members is left out of the file (`Sel::reg_of`); its
  `LoadUniform` is `mov t.x, byte_offset` + `load.u32` from
  `SHC_VIV_CONST_UBO_ADDR` (`Sel::block_read`). A mixed register stays.
- `take_uniforms` no longer refuses a block over 1280 dwords: it hands the GL
  core's block over in place (null or unaligned still refused).
- `KNOWN_LIMITS` in `tests/es3_ubo.rs` is empty: the four cases link and
  compare on the host. Unit test: `block_members_are_loads_and_leave_the_
  constant_file` (select.rs).
- Not done: the i.MX8MP run of a constant-offset `load` from a user block.
