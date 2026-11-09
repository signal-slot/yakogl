<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a local array of more than 8 elements, the `TooManyLocals` refusal (row 13 of `mali-corpus-families.md`)

**Status: design, nothing implemented** (plan #336, item ana-locals-loops;
issue #33). Row 13 of `mali-corpus-families.md` names two sources,
`tools/shaders/es100/tmparr_const.vert` and `.frag` (dEQP
`shaders.indexing.tmp_array.vec4_const_write_dynamic_read_vertex` and
`_fragment`, `es2fShaderIndexingTests.cpp`, as their file headers say). Neither
is a sweep case. The F4 section of `mali-remaining-families.md` has the
fragment reference and a proposal (`locals-cap`); this note re-measures
both stages, corrects its claim that the vertex side is not exercised,
and puts the lowering in steps against the tip (f5bc25f4).

"Measured" below is a run on this session of Mesa 26.2.1 on the drm-shim
(`tools/mali/drm_shim_ref.sh run`, built with its `build` line's meson options
from the extracted 26.2.1 tree, `GPU_ID` 0xa867, `BIFROST_MESA_DEBUG=shaders,shaderdb`).
The fragment run's stderr equals the committed
`tools/mali/testdata/other_tmparr.stderr` byte for byte but the one line that
names the trace file (`cmp`/`diff` of the two), so the fragment words below
are the committed reference's.
Everything marked **inferred** was not measured.

## 1. The refusal sites on the tip

| where | what |
|---|---|
| `nir.rs:65` | `MAX_LOCALS: usize = 32`, "the fragment walk's too" |
| `nir.rs:2488-2489` | `build` (vertex): `m.locals.len() > MAX_LOCALS` -> `TooManyLocals`, before anything is walked |
| `nir_fs.rs:2661-2662` | `walk` (fragment): the same test |
| `vec16.rs:387` | `program` (the mediump fragment fp16 route) returns `Ok(None)` on the same test, so the lane walk (`nir_fs.rs:2661`) is next |
| `mod.rs:269`, `:354`, `:493` | the variant, its list entry, the text "a shader with more than 32 local variables is not lowered yet" |
| `tests.rs:1204` | pins it with 33 scalar locals (`VS`), not with an array |
| `tools/mali/corpus-verdicts.txt:99-100` | both sources, this text |

Why 40 locals: the front end gives an array's elements one local each, a
vec4 per element (`glsl.rs:171-176`, `same_run`: "an array's elements ...
one local each, consecutive", lower.rs `alloc_var`). `vec4 arr[40]` is 40
locals, and the 33rd decides. `glsl::passes` runs first (`mod.rs:1101`) and
leaves the arrays' run-time reads alone: `ilt_trees` rewrites an index ladder of
at most `MAX_LADDER = 8` slots (`glsl.rs:201`, the returns at `:356` and
`:434` give up at 8), so the 40-slot `ieq` ladder is still in the module
when `walk` counts. The refusals this source meets in order: `TooManyLocals`
here; with the cap raised, an `ieq`/`bcsel` ladder over 40 slots would be
built that is not Mesa's program (section 2) -- **inferred** (not run).

## 2. What Mesa does

### Source lines (`src/panfrost/compiler/bifrost/`, Mesa 26.2.1)

- `bifrost_nir.c:209-210`: `nir_split_array_vars` and
  `nir_shrink_vec_array_vars` on `nir_var_function_temp`, in the optimisation
  loop: the array stays one variable (every element is read at a run-time
  index, so the split has nothing to cut).
- `bifrost_nir.c:975-984`: `nir_shader_has_local_variables` ->
  `nir_lower_indirect_derefs_to_if_else_trees(nir_var_function_temp, 8)`: an
  indirect access on an array of at most 8 elements becomes an if/else tree
  (case 80's binary search, `mali-indexed-locals`/`glsl.rs` `ilt_trees`).
  40 > 8, so the array is not touched.
- `:993-995` `nir_remove_dead_variables(function_temp)`: only arrays nothing
  reads are dropped (see "the position program" below).
- `:998-999` `nir_lower_vars_to_explicit_types(function_temp,
  glsl_get_natural_size_align_bytes)`, `:1001-1005`
  `nir_lower_explicit_io(function_temp, nir_address_format_32bit_offset_as_64bit)`:
  the variable gets a byte offset (natural layout: an f16 vec4 is 8 bytes,
  align 8) and every deref becomes `load_scratch` / `store_scratch`. That
  is `nir->scratch_size`; the shaderdb header prints `scratch: 320`.
- `bifrost_compile.c:1744-1750`: `load_scratch` / `store_scratch` ->
  `bi_emit_load` / `bi_emit_store` with `BI_SEG_TL`, `VA_MEMORY_ACCESS_FORCE`
  (the comment at `:1029-1030`: the 64-bit address happens on `store_scratch`
  produced by `nir_lower_vars_to_scratch`).

### Measured: the NIR (fragment, `tmparr_const.frag`)

    scratch: 320
    16x4 %0 = load_const (0x3000, 0x3400, 0x3800, 0x3c00)   ; arr[0] = (0.125, 0.25, 0.5, 1.0)
    32   %1 = load_const (0x00000000)
         @store_scratch (%0, %1 (0x0)) (align_mul=256, align_offset=0, wrmask=xyzw)
    16x4 %2 = load_const (0x2c00, 0x3000, 0x3400, 0x3800)
         @store_scratch (%2, %3 (0x8)) (align_mul=256, align_offset=8, wrmask=xyzw)
      ... 40 stores at offsets 0, 8, ..., 312 (arr[4..39] are splats 4.0 .. 39.0:
          0x4400 .. 0x4700 ..., one f16 per lane)
    32   %82 = ishl %80.x, %81 (0x3)
    16x4 %83 = @load_scratch (%82) (access=none, align_mul=8, align_offset=0)
      ... 4 loads, one per `ui_*` index; res = two fadd on the f16x2 halves per load

The constant vec4 values are f16 (the array is `mediump`), stored whole, no
compare or select ladder.

### Measured: the Valhall words (fragment, 153 instructions, 8 registers, 23 uniforms)

Stores, rotating the slots 0, 1, 2 (`STORE.i64.force.slotN`, offset 8k):

    IADD_IMM.i32.discard r0, 0x0, #0x3C003800     ; arr[0] words: 0x34003000 and
    MOV.i32 r2, u4.w0                              ; 0x30002C00 are u4.w0 / u4.w1
    MOV.i32 r3, r0^                                ; (FAU, shared with arr[1..3])
    STORE.i64.force.slot0 @r2:r3, thread_local_pointer.w0, offset:0
    IADD_IMM.i32 r0, 0x0, #0x38003400
    MOV.i32 r2, u4.w1
    MOV.i32.wait0 r3, r0^
    STORE.i64.force.slot1 @r2:r3, thread_local_pointer.w0, offset:8
    ...
    MOV.i32 r0, u5.w0                              ; arr[4] = splat 4.0: one FAU word twice
    MOV.i32.wait0 r1, u5.w0
    STORE.i64.force.slot1 @r0:r1, thread_local_pointer.w0, offset:32
    ...                                            ; u5..u22: 36 FAU words, offsets 32 .. 312

The four non-splat vectors give four `IADD_IMM` words
(`0x3C003800, 0x38003400, 0x2C002800, 0x28002400`) and two FAU words
(`0x34003000`, `0x30002C00`: `u4.w0`, `u4.w1`); the 36 splats are one FAU word
each (`u5.w0` .. `u22.w1`). Loads, four times:

    MOV.i32 r0, 0x3020100
    LSHIFT_OR.i32 r1, u2.w0, r0.b33, 0x0           ; index << 3  (byte 3 of 0x03020100 is 3)
    IADD.u32 r1, thread_local_pointer.w0, r1
    MOV.i32 r2, r1
    MOV.i32.wait0 r3, thread_local_pointer.w1
    LOAD.i64.force.unsigned.slot1 @r2:r3, [r2^:r3^], offset:0
    ...
    FADD.v2f16 r1, r2^, r4^   FADD.v2f16 r2, r3^, r5^   ; accumulate
    ATEST.wait @r60, r60^, r1.h1, atest_datum.w0
    BLEND.slot2.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0

Shaderdb: `153 instrs ... 0 loops, 0:0 spills:fills, 440 spill cost, 8
registers used, 23 uniforms used` (the "440 spill cost" is the load/store
latency model, no spill happened). The temporary pre-NIR-to-Valhall form
(before RA) uses `STORE.i64.tl %4^, tls_ptr, tls_ptr[1], byte_offset:16` and
`LOAD.i64.tl r2^, r3^, byte_offset:0` with `tls_ptr` the thread-local
pair; `tls_ptr` becomes the special register `thread_local_pointer`.

### Measured: the vertex side (new; `tmparr_const.vert` + `va.frag`)

`other_tmparr` pairs the vertex shader with `tmparr_const.frag`, which never
reads `v_color`, so that pair's varying program is empty (0 instructions) and
its position program the usual 24: its note says the vertex side is "not
exercised". With `va.frag` (reads `v_color`) both vertex programs carry the
array:

    MESA_SHADER_POSITION shader: 149 instrs ... 400 spill cost, 11 registers used, 22 uniforms used
    MESA_SHADER_VARYING  shader: 170 instrs ... 440 spill cost,  9 registers used, 24 uniforms used
    scratch: 320   (both NIR dumps)

- Both programs hold the **40 stores** (`STORE.i64.force`, same words as the
  fragment's, in the same slot rotation). Only the VARYING program holds the 4
  loads. The position program therefore stores 40 vec4 that nothing in it
  reads: Mesa does not remove the stores of a variable once it is scratch
  (the variable is live in NIR: `v_color` shares the NIR with the position
  variant before `nir_opt_varyings`/IDVS split removes the loads only).
  Why it keeps the stores is **inferred**; that it does is measured
  (40 `STORE.i64.force` before line 762 of the dump, no `load_scratch`).
- The descriptors: the draw's pandecode trace has `Position Local Storage`,
  `Varying Local Storage` and `Fragment Local Storage`, each `TLS Size: 5`
  and a `TLS Base Pointer` (one buffer, the same address in all three).
  `TLS Size: 5` for 320 bytes is `pan_get_stack_shift`
  (`src/panfrost/lib/pan_scratch.c:46-52`): `util_logbase2_ceil(
  DIV_ROUND_UP(size, 16))` = ceil(log2(20)) = 5, written by `pan_emit_tls`
  (`pan_desc.c:902-910`, `cfg.tls_size = shift`); the size itself is
  `info->tls_size = nir->scratch_size` (`bifrost_nir.c:1289`), and the
  register allocator's spill count shares the field (`bi_ra.c:1227,1322`).
- Without the use (tmparr_const.vert + a fragment not reading `v_color`) the
  position program is 24 instructions with no stores: the dead array is
  removed before scratch (`:993-995`).

## 3. Lowering steps

Prerequisite choices (**decisions, not measured**): scope the first change to
the shape both sources have, `mediump vec4 arr[N]`, `N > 8`, every element
stored once, all stores before the reads, each store a constant (so it needs
no value to survive a loop or branch), every read at a run-time int index
that is a uniform (an `index_only` uniform, `nir.rs:2563`) or an index ladder
of those. `tmparr_const` is exactly this.

1. **Recognise the array in the walk, before the count.** In `nir::build`
   (`nir.rs:2488`) and `nir_fs::walk` (`nir_fs.rs:2661`), the check counts
   locals of the module; replace it with a count that does not charge a run
   of locals that `same_run` (`glsl.rs:178-183`) says is one array of
   more than `MAX_LADDER` slots and that the shape test of step 2 accepts.
   Keep `TooManyLocals` for everything else (including a 33-local straight
   program, `tests.rs:1204`). `vec16.rs:387` must skip the array the same
   way or return `None` (it already does for >32).
2. **Take the 40-slot ladder as one scratch array.** `glsl.rs` `ilt_trees`
   gives up past 8 (`:356`, `:434`); add a sibling that finds the same
   ladders at n > 8 and returns an `ArrayRead { first_local, n, index }`
   instead of rewriting them (reads) and the `StoreLocal` slot run
   (`n` stores) as `ArrayStore`. Nothing is rewritten to selects: Mesa keeps
   a variable (section 2).
3. **A scratch node kind.** `nir.rs` `Kind`: `StoreScratch`/`LoadScratch`
   (64-bit address from the `tls` pointer + byte offset, 16-bit vec4 as 2
   words), `bir.rs` `STORE.i64.tl` / `LOAD.i64.tl` with the `force` and
   `unsigned` bits (the encoding is Valhall's `LOAD`/`STORE` with
   `BI_SEG_TL`; the `thread_local_pointer` special register, `.w0`/`.w1`, is
   a register file operand `bir.rs`/`pack.rs` do not have: **inferred**
   from the words above, check against `isa` before adding). The address
   is `IADD.u32 tls.w0, (index << 3)` with `LSHIFT_OR.i32 idx, 0, 0x3020100.b3`
   (a byte-select constant; the same `0x3020100` constant as the ILT
   words of `other_80_matdyn.expect.json`). The `slotN` rotation is the
   existing scoreboard slot allocator's (`flow.rs`; the stores here rotate
   0, 1, 2 and the loads 1, 2, 0, 1: the first load `slot1`).
4. **Constants.** Reuse the FAU assignment in first-use order (`fau.rs`);
   a vec4 of equal halves is one word (`u5.w0` used for both `MOV`s).
   The non-splat vectors: a word that is not in FAU is an `IADD_IMM`; Mesa
   puts the first two distinct f16 words in FAU `u4.w0`/`u4.w1` and the next
   four as `IADD_IMM` -- the rule that decides which is the same as
   `mali-fau-*`/`fau.rs` (**inferred**: compare the two sources' assignment
   with the generator before relying on the split).
5. **Report the scratch size.** `compile_*` returns `MaliShader`; add the
   byte size so `frame.rs` writes `Local Storage` `TLS Size` and a base
   pointer (`LocalStorage::default()` today, per the F4 note): for the
   vertex draw `Position` and `Varying` and for the fragment draw `Fragment`
   (three descriptors, one buffer). Without it the compiler's words are
   right and the draw faults (**inferred**; no board run). This can be
   a second item (`F4` note).
6. **The vertex position variant.** Emit the 40 stores in the position
   program when the array is read in the varying program (section 2); a
   straight-line program that never reads the array gets none (the dead
   array is removed first).

The `Spill` alternative (160 scalar slots in registers) is not Mesa's and
cannot be byte-equal; do not take it.

## 4. The host test to add

- `tests.rs`, beside `other_80_matdyn`: `other_tmparr_is_mesa_26_2_1` over
  `tools/mali/testdata/other_tmparr.{vert,frag,expect.json}`: the committed
  fragment program is byte-equal to `check_program`'s (the committed JSON has
  the fragment words; its vertex side is the unexercised one). The vertex
  half needs a second reference, **not in the tree**: `tmparr_const.vert` +
  `va.frag` through `drm_shim_ref.sh run` and `pan_ref_diff.py extract --draw`
  (testdata files are out of this item's contract; the item that adds
  the vertex step records it as `other_tmparr_vs.*`).
- A unit test that the 40-slot ladder is recognised and a 33-scalar program is
  still `TooManyLocals`, a 9-element array read at an index is the array path
  and an 8-element one still the binary search (`glsl_tests.rs`, beside
  `:73-116`).
- Move the two verdicts: `tools/mali/corpus-verdicts.sh --write`
  (`corpus-verdicts.txt:99-100`).

## 5. What it moves

- **Corpus sources:** `es100/tmparr_const.vert` and `.frag` only (the
  refusal row 13 has two sources). Other `tmp_array` sources of dEQP
  GLES2 (`indexing.tmp_array.*`, ints, bools, 4-element vectors etc.) are
  within 8 elements for most or need the same scratch for the larger
  ones: **inferred**, no tally was run for this note (`corpus-tally` is the
  plan's item for that).
- **gl_test / dEQP cases:** none of gl_test's 256. dEQP-GLES2
  `shaders.indexing.tmp_array.vec4_const_write_dynamic_read_vertex` and
  `_fragment` (and possibly the `_dynamic_write` variants of the same
  generator, **inferred**) once plan #327's sweep includes them.
- **Not moved by this:** `IndexedLocal` (case 80, `matdyn`) -- that is the
  <= 8 element binary search, a different lowering, already in `glsl.rs`.
- **Sequence:** after the vertex and fragment walks stop being edited by
  fs-fold/vs-lookups (`nir.rs`/`nir_fs.rs` are shared), and before any frame
  change for `Local Storage`.

## 6. Open points

- The `Local Storage` base pointer and the per-core total
  (`pan_get_total_stack_size`, `pan_scratch.c:56`; `pan_cmdstream.c:1670-1678`
  allocates `dev->thread_tls_alloc` x `core_id_range`) are not read for
  this note: `frame.rs` needs both, the shift is known (step 5).
- Whether a constant-only array is also a candidate for a `LD_PKA`/FAU
  table by Mesa when it is read-only (`nir_opt_large_constants`): not
  here (Mesa wrote 40 stores), but `bifrost_nir.c` has an option for it at a
  size threshold; **inferred**, the stores are measured.
