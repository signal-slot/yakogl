<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): a uniform read at a run-time index, Mesa's design

Issue #1385, plan #152 item `notes-uniform-index`, for the follow-up item
`uniform-index-122`: the fragment shader of gl_test case 122 (`uniidxi`) reads
`fcols[fidx + int(t.w * 0.0)]` after its texture look-up
(`tools/shaders/es100/uniidx_i.frag:9`), and the back end refuses the read with
`Valhall: a uniform read at a run-time index is not lowered yet`
(`MaliError::UniformIndexed`, `src/shaderc/online/mali/mod.rs:281`). This note
is what Mesa 26.2.1's panfrost does for such a read on v10, how it sits with the
fragment uniform block pushed to the FAU (#1384 S3,
`src/shaderc/online/mali/fau.rs`), what the frame side needs, which corpus rows
are refused for it, and the steps. It is a reading of Mesa's source and ours,
checked against one Mesa board recording of case 122 itself (section 6);
nothing else was run. A fact only the shim or the
board can settle is marked **pin on the shim** (Mesa's bytes) or **pin on the
board**; a fact section 6 settled says so. The texture refusals around it are `docs/notes/mali-texture-refusals.md`
(section (b) names this refusal as 122's next one); the general lowering is
`docs/notes/mali-lowering-map.md` and `driver/docs/nanopc-t6-mali.md`, "The
general lowering (approved 2026-10-19)".

## Citations

- Mesa: `M:<path>:<line>`, `<path>` under `src/` of the Mesa 26.2.1 tree
  (`MESA_PAN_TREE`, `$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`, `VERSION`
  26.2.1). `M:panfrost/pan_shc/` is the repository's oracle copy of the GL
  driver's compile path, `M:gallium/drivers/panfrost/` the driver itself; where
  both hold the line, the driver's is cited.
- Ours: `<path>:<line>` from the repository root, at `main` c9a8dd77.
- Every citation carries its whole path, so each can be checked alone. The
  compiler read is Mesa's default Bifrost-BIR path (kraid runs only for stages
  named in `PAN_USE_KRAID`, default none:
  `M:panfrost/compiler/pan_compiler.c:49-68`).

## The answers

1. **Mesa makes one `LD_PKA.i<n>` per access.** The GL uniform block is UBO 0;
   `load_uniform` becomes `load_ubo(0, byte offset)`, and a run-time index only
   makes the byte offset an SSA value instead of a constant. The instruction's
   operands are a 32-bit byte offset (a register for a run-time index) and a
   mode descriptor (resource table 0, index 0: the constant `0x0`). There is no
   second form for the indirect case; the constant case is the same
   instruction (section 1).
2. **A block can be both pushed and read indirectly.** Mesa pushes only the
   *constant-offset, word-aligned* reads (`bi_is_pushable_ubo`), picks the words
   by the reads it finds, and every other read of the block keeps its `LD_PKA`.
   A run-time read does not unpush anything and is not served by the FAU: the
   FAU holds a copy of some words, `LD_PKA` reads the whole block from memory.
   What it forces is that the block is **uploaded** (`ubo_mask` bit 0) and has a
   descriptor in the UBO table (section 2).
3. **The frame side has the pieces but not the layout.** It has the `Buffer`
   descriptor, a UBO table, the resource table, and the draw's uniform storage.
   It lacks: the block built in Mesa's packed layout (today it only picks single
   words out of the GL storage), a table whose entry 0 is that block (today
   entry 0 is the sysval block), and the table count (section 3).
4. **Eight corpus rows are refused for it**, plus 122 once its coordinate
   passes (section 4); the shim already holds the constant-offset `LD_PKA`
   (`sel_u_fau130`), and no reference holds a run-time offset.
5. The steps are in section 5.
6. **The Mesa board recording of case 122 answers the open points it can**
   (section 6): the read is UBO 0 of resource table 0; the non-indexed word
   (`fidx`) is still pushed to FAU; the entry is exactly 36 bytes; Mesa made
   **one `i64` load** of a mediump `vec4` (not two `i32`) and folded
   `int(t.w * 0.0)` away, so the address is `fidx << 3`; the picture is the
   expected colour. Where it differs from sections 1-4 (load count, table
   count, the folded index) those sections now say so.

## 1. How Mesa lowers `load_uniform` with a non-constant offset on v10

### From GLSL to `load_ubo`

- The state tracker lowers the default-block uniforms to `load_uniform` with
  `nir_lower_io` in *dword* units (`PackedDriverUniformStorage`), then, because
  the Bifrost options set `lower_uniforms_to_ubo`
  (`M:panfrost/compiler/bifrost/bifrost_compile.h:135`), to UBO 0 with
  `nir_lower_uniforms_to_ubo` (`M:mesa/state_tracker/st_glsl_to_nir.cpp:701-716`;
  the oracle copy `M:panfrost/pan_shc/pan_shc.c:549-552`). Panfrost reports
  packed uniforms (`M:gallium/drivers/panfrost/pan_screen.c:747`).
- The block's layout is what `nir_lower_io` with `st_packed_uniforms_type_size`
  makes it: `glsl_count_dword_slots`
  (`M:compiler/glsl_types.c:2961-2988`) gives a scalar or vector its component
  count (a `float[4]` is 4 dwords, element stride 4 bytes; a `vec4[4]` is 16
  dwords, element stride 16 bytes), and a 16-bit type
  `DIV_ROUND_UP(components, 2)` dwords per column (a mediump `vec4` is 2 dwords,
  8 bytes). Nothing is aligned. Our mapping of it is `UboLayout`
  (`src/shaderc/online/mali/select.rs:185-259`).
- `nir_lower_uniforms_to_ubo` with `dword_packed` multiplies the offset by 4
  (`M:compiler/nir/nir_lower_uniforms_to_ubo.c:84-88`). With a constant offset
  it records the exact alignment; **with a run-time offset it records the
  pessimistic `MAX2(4, bit_size / 8)` bytes at offset 0**
  (`M:compiler/nir/nir_lower_uniforms_to_ubo.c:99-106`). That alignment, not
  the array's real stride, is all the later passes know about an indexed load.
  It also records the range (`:108-109`).

### From `load_ubo` to the instruction

- `nir_opt_load_store_vectorize` may merge UBO loads only up to the common
  alignment (`mem_vectorize_cb`,
  `M:panfrost/compiler/bifrost/bifrost_nir.c:182-199`, options `:297-310`).
  `nir_lower_mem_access_bit_sizes` then splits every UBO load by
  `mem_access_size_align_cb` (`M:panfrost/compiler/bifrost/bifrost_nir.c:776-830`,
  applied to `nir_var_mem_ubo` at `:1009-1017`): a load is at most 16 bytes, and
  must be aligned to the next power of two of its size, otherwise it shrinks to
  the alignment (`:805-808`). So a run-time-indexed `vec4` fp32 load, aligned to
  4, is **four 32-bit loads**, not one `i128`; a constant-offset one keeps its
  alignment and stays one. **The board recording contradicts this for the
  mediump `vec4`:** Mesa 25.0.7 made one `LD_BUFFER.i64` (opcode byte `0x6a`,
  the same as `LD_PKA`; the older pandecode names it `LD_BUFFER`) of the
  run-time indexed `fcols[fidx]`, not two `i32`
  (`0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-uniidxi.pandecode:460`).
  So the 8-byte, element-stride-8 load survives an index whose alignment the
  UBO pass recorded as 4. Whether 26.2.1 and an fp32 `vec4` (one `i128` or four
  `i32`) agree is **pin on the shim** (`sel_ui_vec4`, `sel_ui_mediump_vec4`);
  the row stays generic in the width.
- `bi_emit_load_ubo` (`M:panfrost/compiler/bifrost/bifrost_compile.c:983-996`)
  takes the NIR offset source as the address (an immediate when constant, the
  SSA value otherwise) and the NIR buffer source as the second operand; the
  `case nir_intrinsic_load_ubo` is `:1723-1725`. `bi_load_ubo_to`
  (`:870-883`) emits, for arch >= 9, `LD_PKA` with `seg = BI_SEG_UBO` (before
  v9: a plain `LOAD`).
- The buffer source is a resource handle: `va_lower_load_ubo_intrin` rewrites
  the buffer index to `pan_nir_res_handle(PAN_TABLE_UBO, 0, index)`
  (`M:gallium/drivers/panfrost/pan_nir_lower_res_indices.c:189-198`, dispatched
  at `:230-231`), which for a constant index is the immediate
  `(table << 24) | index` (`M:panfrost/compiler/pan_nir.h:51-61`,
  `M:panfrost/compiler/pan_compiler.h:603-630`). UBO 0 is therefore the
  constant `0x0`, the sysval UBO `0x1` (`PAN_UBO_SYSVALS`,
  `M:gallium/drivers/panfrost/pan_context.h:320`).

### The instruction: address, offset form, bounds

- `LD_PKA.i8 ... .i128` (`M:panfrost/compiler/bifrost/valhall/ISA.xml:1256-1425`;
  opcode `0x6A` with `opcode2` selecting the width, `:1256-1262`, the 128-bit one
  at `:1403`) has two sources: "Address to load from after adding offset" and
  "Mode descriptor" (`:1272-1275`). The description (`:1262-1269`): the low byte
  of the mode descriptor indexes a table of buffer descriptors, and the load
  reads the contents of that *buffer*, with the bits 25-31 of the mode
  descriptor selecting between the descriptor table and the buffer.
- `va_pack_load(I, true)` (`M:panfrost/compiler/bifrost/valhall/va_pack.c:746-769`,
  called for the `LD_PKA` family at `:895-904`) packs source 0 at bit 0, source 1
  at bit 8 (`:767-768`), the load lane and `unsigned` at 36-39, the memory-access
  bits at 24;
  **it does not pack `byte_offset`** (that is the plain `LOAD`'s immediate,
  `:761-762`). So the address operand is the whole byte offset: a register
  holding `index * stride + base` for a run-time index, a FAU or immediate
  constant for a constant one.
- The shim already pins the constant form: in `sel_u_fau130`'s position program
  the word is `026a80f83800c0be`, `LD_PKA.i128.unsigned.slot0 @r0:r1:r2:r3,
  u63.w0, 0x0` (`tools/mali/testdata/sel_u_fau130.expect.json:9`, `:58`), where
  `u63.w0` is the promoted immediate `0x1e0` = byte 480
  (`tools/mali/testdata/sel_u_fau130.expect.json:235`), the offset of word 120,
  and `0x0` is the mode descriptor, UBO 0. The test that pins it is
  `src/shaderc/online/mali/tests.rs:1837-1845`.
- **Alignment.** The offset must satisfy the width's alignment (16 bytes for
  `i128`, 4 for `i32`; the splitting above is Mesa's way to guarantee it). No
  pass inserts a rounding of a run-time offset; a block layout that is only
  dword-aligned (a `vec3`, a mediump `vec4`) is handled by the narrower loads.
- **Bounds.** The compiler adds no check: `robust_descriptors`, which the
  driver sets for arch >= 9 (`M:gallium/drivers/panfrost/pan_shader.c:209-212`),
  is declared in `M:panfrost/compiler/pan_compiler.h:40-42` and read by no
  Bifrost or Valhall pass (a search of `src/panfrost` finds it in no other
  file but the Vulkan driver's setup and `pan_shc.c:715`). The limit is the
  buffer descriptor's `Size` (`panfrost_emit_ubo`,
  `M:gallium/drivers/panfrost/pan_cmdstream.c:1438-1452`: `cfg.size = size`,
  the bytes of the constant buffer). The recording confirms `Size` is the exact
  packed bytes, 36, not rounded to 16 (`mesa-uniidxi.pandecode:261`, field
  `Size` of the `Buffer` at `:258`); its index (2) is in range, so it shows no
  out-of-range read. A read past it is the hardware's: **pin on
  the board** what an out-of-range `LD_PKA` returns (zero is the expected
  robust behaviour; nothing here shows it). Our IR clamps the index to
  `[0, len)` (`src/shaderc/online/ir.rs:367-374`, `src/shaderc/online/eval.rs:511-519`)
  where Mesa does not (GLSL ES leaves an out-of-range index undefined):
  the Mali lowering follows Mesa, so it emits no clamp, and in-range indices,
  all the corpus shapes below read the same bytes.
- Scheduling: UBO loads are read-only, so `va_insert_flow` puts no ordering
  barrier on them (`M:panfrost/compiler/bifrost/valhall/va_insert_flow.c:118-119`).
  The result is still a slot result the reader waits for. `flow.rs` transcribes
  `va_insert_flow` (`src/shaderc/online/mali/flow.rs:13-17`: "memory accesses are
  serialised"), so the UBO exemption above must be transcribed there too, and the
  `wait` bits of an `LD_PKA` next to a store or another load are **pin on the
  shim**.

### What the index arithmetic needs

`fidx + int(t.w * 0.0)` is integer work in front of the address: `F2I32`, `IADD`,
and the scaling `index * stride` (a shift or `IMUL` by a constant, plus the
block base). Today any integer value is refused (`MaliError::IntValue`,
`src/shaderc/online/mali/mod.rs:116`, `:276`), and `uniform int fidx` is itself
a block word that must be read as raw bits. The ALU rows are #1384's S4 (`f2i`,
`iadd` per `docs/notes/mali-lowering-map.md:191-193`, `:272`); this note adds
only the scaling and the `LD_PKA` row. The exact NIR the GL driver makes of the
index (an `imul`/`ishl`, an `iadd` of the base, a 16-bit form for a mediump
`int`) is **pin on the shim** with the references of section 4.

## 2. With the fragment uniform block pushed to FAU (S3)

### What Mesa's push analysis does

- `bi_opt_push_ubo` runs after code emission, before copy propagation, when the
  shader has pushable UBOs (`M:panfrost/compiler/bifrost/bifrost_compile.c:4447-4449`;
  `pushable_ubos` is UBO 0 and, with sysvals, the sysval UBO,
  `M:gallium/drivers/panfrost/pan_shader.c:196-205`). It looks only at
  `LD_PKA`/`LOAD` instructions with `seg == UBO` (`bi_is_ubo`,
  `M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:14-19`).
- A load is *pushable* only if **both** its address and its buffer are
  constants, the buffer is in `pushable_ubos`, and the offset is word-aligned
  (`bi_is_pushable_ubo`, `M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:21-33`).
  A run-time offset fails the first test. `bi_analyze_ranges` counts only
  pushable loads (`:50-89`), `bi_pick_ubo` picks words from them, last UBO first
  (`:120-147`), and the rewrite replaces only loads whose words were picked
  (`:189-206`).
- **Every load that stays is recorded in `ubo_mask`:** a non-pushable load of a
  constant buffer sets that buffer's bit, a non-constant buffer sets all of
  them, and a pushable load whose word was not picked also sets the bit
  (`M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:162-187`). The context starts
  with `~0` (nothing pushed, everything uploaded,
  `M:panfrost/compiler/bifrost/bifrost_compile.c:4401-4402`), and the mask goes to
  the shader info (`:4703`, then `&= (1 << num_ubos) - 1`,
  `M:panfrost/compiler/bifrost/bifrost_nir.c:1311`).
- So for one block with constant reads *and* an indexed read: the constant reads
  are still pushed (their words occupy FAU slots, in the usual order), the
  indexed read keeps its `LD_PKA`, and bit 0 of `ubo_mask` is set. The pushed
  words and the indexed reads of the same words are two copies of the same
  bytes. For a block that is **only** read indirectly (case 122: `fcols[...]`
  and `fidx`) nothing of UBO 0 is pushed except what a constant read names
  (`fidx` is a uniform read at a constant word: pushed). **Confirmed by the
  recording:** the fragment FAU is `00000002 00000000`
  (`mesa-uniidxi.pandecode:383-384`): `fidx` = 2 is the one pushed word `u0`
  (the second word pads to an even count), and `fcols`, the indexed array, is
  not pushed; the shader reads `u0` at `:459` and loads the array at `:460`.
- Pushing never interferes with the addressing of the indirect read. The
  indexed load addresses the block in memory by byte offset from word 0, in the
  packed layout, wherever the pushed copies are in the FAU; `reorder_push`
  (`M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:305-378`) permutes only the
  FAU copies.

### What the driver does with the mask

- `panfrost_emit_const_buf` (`M:gallium/drivers/panfrost/pan_cmdstream.c:1497-1651`)
  allocates `ubo_count + 1` Buffer descriptors, zeroed
  (`:1523-1540`); the sysval block goes to entry 1 (`:1553`); then, for each
  bit of `ubo_mask` that has a bound constant buffer, it uploads the buffer
  (`panfrost_map_constant_buffer_gpu`, a copy to GPU memory when the size is
  non-zero) and writes its descriptor (`:1557-1577`). A UBO whose bit is clear
  (everything of it pushed) has a **zero descriptor**.
- The pushed words are copied from the CPU-mapped user buffer by relocation
  (`pan_fau_foreach_reloc`, `:1598-1646`; the comment `:1624-1646` says why
  real UBOs are not pushed this way).

### Consequence for the S3 design

`fau.rs`'s `push_ubo` is Mesa's pass for the *pushable* reads only: it counts
`Idx::Ubo` sources, emits the words and rewrites to `Idx::Uniform`
(`src/shaderc/online/mali/fau.rs:104-190`). The words it does not push today are
`UboFallback` (`:151`, `:180`; `MaliError::UboFallback`, `mod.rs:130`, `:288`),
the case Mesa serves with an `LD_PKA`. The run-time read is a **second source of
that same instruction**, and the two should land in one row: a `Ld Ubo`
(`LD_PKA`) BIR op taking (byte offset source, buffer handle), which

- `push_ubo` ignores, as `bi_is_pushable_ubo` does (a non-constant offset is not
  an `Idx::Ubo`); it must also *not* mark the word range of an indexed read as
  read (Mesa's range analysis never saw it);
- the compile result reports as `ubo_mask` bit 0, so the frame side uploads the
  block (section 3);
- later also serves the `UboFallback` case (a constant offset past the 128 FAU
  words), which `sel_u_fau130` pins, so the row's shape is shared and S3's
  FAU-overflow refusal is lifted by the same code.

Two fragments of S3 need a decision from this note: S3 lays the fragment block
out in fp32 words (`FauWord::Uniform`), while the fragment mediump uniforms are
fp16 halves today (`UboLayout::fs`, `src/shaderc/online/mali/select.rs:236-241`,
`nir_fs.rs:415` `uniform_lane`; the old plan's `frag.rs:744` is at da18ff68). An indexed read of a *mediump* array (`uniidx_a` to `_d`, `_g`
to `_j`, case 122 are all `precision mediump float`) reads the halves layout from
memory, so the block's layout in memory and the push words' layout must come
from one description (`UboLayout`), not two. For highp fp32 arrays (femtovg's
`frag[14]`, `idxtex.frag`) the two coincide.

## 3. The descriptors the frame side needs

What exists (`src/mali/frame.rs`):

| piece | where |
|---|---|
| the resource tables and their order (`TABLE_UBO = 0`) | `src/mali/frame.rs:218-224` |
| the `Buffer` descriptor (size, address), written for the sysval block | `src/mali/frame.rs:2204-2211`; `src/mali/genxml/v10.rs:3984-4000` |
| a stage's UBO table and push block | `emit_stage_uniforms`, `src/mali/frame.rs:2175-2243` |
| the table entries per stage (`resource(.., TABLE_UBO, ubos.gpu, count)`) | `src/mali/frame.rs:2463-2477`, `:2246` |
| the draw's uniform storage (the front end's dwords) | `DrawCall::fs_uniforms`, `src/mali/frame.rs:1233-1236`; filled `src/mali/backend.rs:1353-1354` |
| push words (`PushWord::Uniform`, `UniformHalves`) built from that storage | `src/mali/frame.rs:441-460`, `:2224-2237`; from the record `src/mali/program.rs:55-100` |
| the checks of the push words against the storage | `src/mali/frame.rs:1830-1845` |

What the indexed read adds:

1. **Entry 0 of the UBO table is the uniform block, entry 1 the sysvals.** Today
   `emit_stage_uniforms` puts the sysval block at entry 0 with count 1 and
   leaves user UBOs "(none)" (`src/mali/frame.rs:2196-2211`). Mesa's table has
   the sysval block at `PAN_UBO_SYSVALS = 1`, UBO 0 at entry 0 (zero when its
   bit is clear) and `ubo_count + 1` entries
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1523-1553`). Nothing reads entry
   0 today (the sysval words are pushed by pointer), so the difference is
   invisible; an `LD_PKA` with mode `0x0` reads entry 0 and makes it matter.
   The table, its count and the sysval's slot should follow Mesa, **pin on the
   shim** with a trace: the shim's resource table words for a fragment shader
   with an indexed read give the count and the entry order. **The recording
   settles the fragment stage's case:** the UBO table (entry 0 of the fragment
   resource table, `mesa-uniidxi.pandecode:254-265`) has `Size: 0x20`, one
   32-byte descriptor, the `Buffer` of UBO 0 (`Size: 36`,
   `Address: 0x7ffffffcf2a0`); there is no second descriptor for a sysval block
   because the shader uses none. So the count is one entry for a shader with
   only UBO 0, not the `ubo_count + 1` read above. Mesa 25.0.7 is the driver
   that ran; whether 26.2.1 sizes it the same is **pin on the shim** (the
   `sel_ui_*` trace's table size), as is the entry order for a shader with
   both a UBO 0 read and sysvals.
2. **The block in Mesa's packed layout.** Pushing picks single words out of the
   front end's storage (`PushWord::Uniform { dword }`). The block the load reads
   is the whole of it, in the layout `UboLayout` describes: the stage's *used*
   uniforms in declaration order, one dword per component, mediump floats as fp16
   pairs (`select.rs:185-259`). The compile result must therefore carry that
   description (rows of front-end dword, Mesa word, components, and the halves
   flag; `ShaderMeta` has `push` and `sysvals` only today,
   `src/mali/frame.rs:507-516`), and `emit_stage_uniforms` build the block
   from `d.fs_uniforms` with the same conversions the push words use
   (`f32_to_f16`, `src/mali/frame.rs:462-495`) and upload it once per draw next to
   the sysvals. Mesa's upload copies the application's constant buffer; for the
   default block the bytes are the GL driver's packed storage, which is this
   layout.
3. **The descriptor:** `Buffer { size: bytes of the block, address }`, as the
   sysval one (`src/mali/frame.rs:2209-2210`), and `size` exact (the bound of
   section 1). A block of size 0 gets address 0, as Mesa
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1569-1577`).
4. **The compile result's `ubo_mask`** (a bit per buffer; only bit 0 can be set
   here), so the frame builds the block only for a shader that has a load left
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1557-1577`) and leaves a
   zero descriptor otherwise; that keeps every existing draw's descriptors
   byte-identical.
5. **`check_draw`:** a block read by `LD_PKA` needs its rows inside the draw's
   storage (the analogue of `NOT_BUILT_PUSH_UNIFORM`,
   `src/mali/frame.rs:156-159`, `:1836-1840`): a refusal string, not a read past
   `fs_uniforms`.

`src/mali/frame.rs` and `src/mali/backend.rs` belong to the running item
`frame-query` while this note is written; items 1-5 are for after it merges, and
the compiler side (sections 1-2, 4) does not touch them.

## 4. The other corpus rows, and the references that pin the words

### Rows refused for the run-time uniform index

`tools/mali/corpus-verdicts.txt` (generated; `tools/mali/corpus-verdicts.sh`)
holds eight rows with the refusal string:

| row | line | what it reads | gl_test |
|---|---|---|---|
| `tools/shaders/es100/uniidx_a.frag` | `tools/mali/corpus-verdicts.txt:102` | `mediump float u_arr[4]` at four uniform ints | 108 (`driver/tools/pvrlab/gl_test.c:13086`) |
| `tools/shaders/es100/uniidx_b.frag` | `tools/mali/corpus-verdicts.txt:103` | one `u_arr[ui_two]` | 108 |
| `tools/shaders/es100/uniidx_c.frag` | `tools/mali/corpus-verdicts.txt:104` | the same, with ALU between (and a varying) | 108 |
| `tools/shaders/es100/uniidx_d.frag` | `tools/mali/corpus-verdicts.txt:105` | `vec4 fcols[fidx]` | 108 |
| `tools/shaders/es100/uniidx_g.frag` | `tools/mali/corpus-verdicts.txt:108` | `fcols[fidx]` and a texture look-up after it | 120 (`driver/tools/pvrlab/gl_test.c:21388`) |
| `tools/shaders/es100/uniidx_h.frag` | `tools/mali/corpus-verdicts.txt:109` | `fcols[fidx]`, the look-up after the read, at a computed coordinate | 121 |
| `tools/shaders/es100/uniidx_j.frag` | `tools/mali/corpus-verdicts.txt:111` | `fcols[fidx].ba` with a varying | 128 |
| `tools/shaders/es300/idxtex.frag` | `tools/mali/corpus-verdicts.txt:137` | highp `vec4 fcols[fidx]` and `texture()` | 100 (`driver/tools/pvrlab/gl_test.c:20604`) |

(`driver/tools/pvrlab/gl_test.c:21317`, `:21324`, `:21388-21392`: the case table.)
`uniidx_i.frag` (case 122) is the ninth: its row says `a texture coordinate
other than a vec2 varying read whole` (`tools/mali/corpus-verdicts.txt:110`)
because the look-up comes first; once #1385's coordinate item takes the look-up,
its next refusal is this one (`docs/notes/mali-texture-refusals.md:343-358`).
Two more fragment shaders contain a run-time uniform read behind a different
first refusal: `tools/shaders/es100/uniform_idx.frag` (arithmetic,
`tools/mali/corpus-verdicts.txt` row `uniform_idx.frag`; its index is
`int(v_color.r * 8.0) - 2`, out of range on both sides, which Mesa does not
clamp: it cannot be a byte reference of the IR's clamp) and
`tools/shaders/es300/dynidx.frag` (an integer value; `cols[idx + i]` in a loop
over a uniform bound: needs S6 as well). The vertex shaders `uniidx_e.vert` and
`idxtex.vert` read `vcols[vidx]` too (cases 116, 100) behind
`a uniform other than a float, vec2, vec3, vec4 or mat4`
(`tools/mali/corpus-verdicts.txt:106`, `:138`); they take the vertex path
(`src/shaderc/online/mali/nir.rs:765`) and are not 122's item.

The refusal is raised in two places, one per walk: the fragment walk
(`src/shaderc/online/mali/nir_fs.rs:589`) and the vertex walk
(`src/shaderc/online/mali/nir.rs:765`). The IR op is `LoadUniformIdx { base_dw,
stride_dw, len, index, comps }` (`src/shaderc/online/ir.rs:374-380`); `base_dw`
and `stride_dw` are in the front end's dwords (fp32), not Mesa's packed words.
S1c deleted the fragment plan (b75d518c), so 122's landing is in the NIR-graph
path and the new `Ld Ubo` row.

### Shim references that pin the words

None of the corpus rows has a `.expect.json` with a run-time offset, so the
programs are references to produce first (`tools/mali/drm_shim_ref.sh run`,
`tools/mali/README-pan_shc.md:122-123`, committed as `tools/mali/testdata/*.expect.json`
with the source beside it, like the `sel_u_*` family). Smallest set, one per
open question:

| reference | source | pins |
|---|---|---|
| `sel_ui_vec4` | `tools/shaders/es100/uniidx_d.frag` (highp copy) | the offset arithmetic (`F2I`-free: `fidx` raw), the scaling of a `vec4` array (16 bytes), whether the load is four `LD_PKA.i32` or one `i128`, the base word of the array in the block, and the mode descriptor |
| `sel_ui_float` | `uniidx_b.frag` | the scaling of a `float` array (4 bytes), a single `i32` load, the read of an `int` uniform (a pushed word, bit-exact) |
| `sel_ui_mediump_vec4` | `uniidx_d.frag` as is (mediump) | the halves layout: element stride 8, the load widths for a mediump `vec4`, whether the result needs `V2F16_TO_V2F32`-style conversions |
| `sel_ui_pushed_and_loaded` | one constant read and one run-time read of the same array | both coexist: the constant word pushed (`FauWord`), the other as `LD_PKA`; the table of immediates |
| `sel_ui_add_f2i` | case 122's shape with a term Mesa cannot fold (`int(t.w * 0.0)` is folded, section 6) | the index chain `F2I32` + `IADD`, the texture result feeding an address |
| `sel_ui_fau130` | `tools/mali/testdata/sel_u_fau130.vert` (exists) | the constant-offset `LD_PKA` (`tools/mali/testdata/sel_u_fau130.expect.json:58`) |

Each reference also gives the **resource table**: the shim trace lists the
tables' count (the UBO table's, section 3, point 1). The frame side needs that
from one of them (a fragment shader with an indexed read), not a new run.

## 5. The steps for `uniform-index-122`

Prerequisites, not part of the item: `tex-coord-122` (the coordinate form,
`docs/notes/mali-texture-refusals.md`), S3 (the fragment uniform block pushed to
FAU, #1384) and the integer rows of S4 (`F2I32`, `IADD`; `S2`'s highp for any
fp32 array). `link-move` has landed. The item changes the Mali compiler in
`src/shaderc/online/mali/` and the frame side after `frame-query` has merged.

1. **References.** Produce the six references of section 4 (the `sel_ui_*` set;
   `sel_ui_fau130` exists), each with its `.expect.json` and source. *Acceptance:*
   `tools/mali/testdata/sel_ui_*.expect.json` exist; the shim's resource-table
   count and the UBO table's entry 0 are written into the commit message; the
   row of each pins its `LD_PKA` width count and the scaling. `tools/check.sh
   --changed origin/main` reaches the new cases.
2. **The `LD_PKA` row.** A BIR op `LdPka` (width, `seg = UBO`, source 0 the byte
   offset, source 1 the mode descriptor), its packing (`va_pack_load`,
   `M:panfrost/compiler/bifrost/valhall/va_pack.c:746-769`; the opcodes
   `isa/v10.rs` already has, `src/shaderc/online/mali/isa/v10.rs:568-583`), and the
   flow/wait derivation. *Acceptance:* the constant-offset form is byte-equal to
   `sel_u_fau130`'s `LD_PKA.i128` word; `a_uniform_past_128_words_is_mesa_s_ubo_load`
   (`src/shaderc/online/mali/tests.rs:1837-1845`) now compares the compiled bytes
   instead of the refusal, and `UboFallback` is raised only for what is still not
   lowered.
3. **The indexed read in the NIR-graph path.** Replace `LoadUniformIdx`'s refusal
   in `nir_fs.rs:379` by: the index lane (an fp32-graph integer value, S4's
   rows), the scaling by Mesa's element stride (`glsl_count_dword_slots`, a
   `stride_dw` of the IR converted to Mesa's, i.e. through `UboLayout::word`
   and the halves flag), the base, and the loads per the alignment rule of
   section 1; no clamp. `push_ubo` skips it and records `ubo_mask` bit 0.
   *Acceptance:* `sel_ui_vec4`, `sel_ui_float`, `sel_ui_mediump_vec4` and
   `sel_ui_pushed_and_loaded` byte-equal; `tools/mali/corpus-verdicts.txt` rows
   `uniidx_a`, `_b`, `_c`, `_d` and `uniidx_j.frag` become `ok` or move to their next refusal (the verdict file says which) (regenerated in
   the same commit, as `CLAUDE.md` requires), and `idxtex.frag` moves to its next
   refusal or `ok`.
4. **The texture before the read.** `uniidx_g`, `_h`, `_i` and `idxtex.frag`
   read after a look-up (`TEX_SINGLE`'s result feeds the address in `_i` through
   `int(t.w * 0.0)`); the `sel_ui_add_f2i` reference. *Acceptance:*
   `sel_ui_add_f2i` byte-equal; rows `uniidx_g`, `_h`, `_i` `ok`. For `_i`
   itself Mesa folded `t.w * 0.0` to 0 and emitted no `F2I32`/`IADD`
   (`mesa-uniidxi.pandecode:453-460`, section 6), so the compiler must fold the
   same way (or `_i` is not byte-equal) and the address is `fidx << 3` alone;
   the `F2I32` + `IADD` chain needs a reference whose term does not fold.
5. **The compile result and the frame side** (after `frame-query`): `ubo_mask`
   and the block description in the record and in `ShaderMeta`, the table
   (entry 0 the block, entry 1 the sysvals, Mesa's count), the block's bytes
   in Mesa's layout and the `Buffer` descriptor, the `check_draw` refusal
   (section 3, points 1-5). *Acceptance:* a host test of the descriptors
   against the shim trace's resource table and UBO entry; every existing draw's
   descriptors unchanged (a shader with `ubo_mask == 0` gets the zero entry 0
   Mesa gives it), `tools/check.sh --changed origin/main` green.
6. **The board.** gl_test cases 108, 120, 121, 122 and 128 (and 100's fragment
   stage, with the vertex stage's own refusals aside) PASS on the NanoPC-T6 and
   their rows in `tools/cts/RESULTS.md`/`tools/mali/RESULTS.md` replace the
   REFUSED ones; the issue comment names the commit. *Acceptance:* case 122's
   picture is `(0.1, 0.2, 0.3, 1.0)` as its comment states
   (`driver/tools/pvrlab/gl_test.c:13254-13255`); Mesa's own read-back at
   (32,32) is `0xff4d3319` against the case's `0xff4d331a` (+-2, section 6), so
   ours must be within that tolerance. **Pin on the board** still: an
   out-of-range index reading, recorded in the note (bounds, section 1) if a
   case reaches it.

## Open points and what can go wrong

- **The load count.** *Partly answered.* The recording shows one `i64` load
  for a run-time mediump `vec4` (section 6), so the pessimistic-alignment
  splitting of section 1 did not apply there. For an fp32 `vec4` (one `i128` or
  four `i32`) and for Mesa 26.2.1 the `sel_ui_vec4` reference says; the row is
  generic in the width.
- **Out of range.** *Open.* Case 122 reads index 2 of 4: no read past `Size` is
  recorded. The run that would settle it: a Mesa board recording (the
  `mref-local.sh` harness of `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/`) of a
  variant of `uniidx_i.frag` with `fidx` = 4 or more, reading the picture. It
  needs the board.
- **A pushed and an indexed copy of one word.** If Mesa's `bi_opt_cse` or copy
  propagation merges a pushed read with the load's result, the reference
  `sel_ui_pushed_and_loaded` shows it; none is expected, the load is not a
  `MOV`.
- **The clamp.** The IR's clamp is v3d's and PowerVR's contract
  (`src/shaderc/online/ir.rs:367-374`); the Mali path does not apply it. A
  reviewer of the item should expect the gl_test rows to read the same, and the
  one corpus row that is built to be out of range (`uniform_idx.frag`) to stay
  out of the Mali PASS list.
- **Fragment fp32 arrays.** Femtovg's `frag[14]` is read at constant indices and
  needs none of this; the item is for the ES 1.00/3.00 test shapes, not the
  product's shaders.

## 6. What the Mesa board recording of case 122 shows

Records: `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-uniidxi.pandecode`
(P below), `mesa-uniidxi.stdout`, `mesa-uniidxi-rb01.pam`, the case source
`0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/cases.c:790-847` and its
`README.md:62`, `:84-85`. The driver is **Mesa 25.0.7** on the Mali-G610
(`mesa-uniidxi.stdout:3`), not the 26.2.1 of the rest of this note; where that
could matter it is said. The shader is `tools/shaders/es100/uniidx_i.frag`
(mediump; `fcols[4]` of `vec4`, `fidx` = 2, `fcols[2] = (0.2, 0.4, 0.6, 1)`,
`cases.c:797-800`, `:830-835`). Its fragment program, P:453-464:

```
457  TEX_SINGLE ... @r0:r1, @r0:r1, ^r2         ; t = texture2D(ftex, vec2(0.5))
458  MOV.i32 r2, 0x3020100
459  LSHIFT_OR.i32 r2, u0, ^r2.b33, 0x0         ; r2 = fidx << 3
460  LD_BUFFER.i64.unsigned.slot1.wait01 @r2:r3, ^r2, 0x0
461-464 FADD.v2f16 / FMA.v2f16                   ; 0.5 * (fcols[..] + t), halves
```

| open point | answer | cited |
|---|---|---|
| Which UBO | **UBO 0**, entry 0 of resource table 0: the load's second operand, the mode descriptor, is the constant `0x0`, and entry 0 of the fragment resource table is a `Buffer`. | P:460 (last operand); P:254-265 |
| Is the non-indexed part still pushed | **Yes.** `fidx` is the pushed word: the fragment FAU is `00000002 00000000`, `u0` = 2 (the second word pads to an even count). `fcols`, the indexed array, is not pushed. | P:383-384 (`Fragment FAU`), P:459 (`u0`) |
| Entry size | `Size: 36` = 4 x 8 (a mediump `vec4` is 8 bytes) for `fcols` + 4 for `fidx`: the **exact packed bytes** in declaration order (`fcols` at byte 0, `fidx` at byte 32), not rounded. `Address: 0x7ffffffcf2a0`. The table entry is `Size: 0x20`, one 32-byte descriptor. | P:258-263 (`Buffer`: `Size`, `Address`); P:254-257 (entry 0 `Size`) |
| Alignment / offset form | The address is a register: `fidx << 3` (stride 8, the halves layout), made by `LSHIFT_OR` with the shift amount taken as byte 3 of the constant `0x03020100` (`.b33`); **no base add** (the array starts at byte 0) and no clamp. One `i64` at an 8-byte-aligned address. | P:458-460 |
| Load count | **One `i64`** for a mediump `vec4`. | P:460 |
| The term `int(t.w * 0.0)` | **Folded away**: no `F2I32`, no `IADD`; `t` is still used by the `FADD`. | P:453-464 |
| Bounds | **Not shown**: index 2 of 4 is in range. Open; the run is in "Open points". | |
| Readback | **The expected colours.** `uniidxi: fcols[fidx], texld before the movar (32,32) = 0xff4d3319  want 0xff4d331a  ok` (R 25, G 51, B 77, A 255 against 26, 51, 77, 255, tolerance 2); `verdict case 122 uniidxi: PASS rc 0`. The `.pam`'s FNV-1a is `e79c9dc5`. | `mesa-uniidxi.stdout:5-7`, `cases.c:844` |

What it changes:

- Section 1's splitting by the UBO pass's alignment is not what happened for
  this shape; `sel_ui_mediump_vec4` should expect one 8-byte load
  (`LD_PKA.i64` in 26.2.1's name; same opcode byte `0x6a`, P:460).
- Step 4: the `F2I32` + `IADD` chain is not in case 122's program. The front
  end must fold `t.w * 0.0` (or `uniidx_i` is not byte-equal); the chain needs a
  reference whose term does not fold.
- Section 3 point 1: a fragment shader with only UBO 0 has a one-entry UBO
  table in 25.0.7; "entry 0 the block, entry 1 the sysvals" is for a shader that
  has sysvals too.
- The mode descriptor `0x0` of `sel_u_fau130` (section 1) matches P:460.

Not answered by the recording, and the run that would settle each: the same
programs on **Mesa 26.2.1** (the shim, `tools/mali/drm_shim_ref.sh run`, the
`sel_ui_*` references of section 4): the load width of an fp32 `vec4`, the table
size and order with sysvals, the `wait` bits next to other memory accesses; an
out-of-range index (a board recording of a larger `fidx`).
