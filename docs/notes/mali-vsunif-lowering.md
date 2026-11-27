<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: what is left behind the vertex "uniform type" refusal (vsunif-lowering)

Plan #387 item `vsuniftype-design`. It designs the lowering for the cases the
Verdin iMX95 (Mali-G310) whole-tree dEQP-GLES2 run refused with the vertex
shader's

> Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet

(`MaliError::UniformType`). This item changes no compiler code. Its
references are the twelve `tools/mali/testdata/deqp_vsunif_*` of section 5,
and `tests/mali_deqp_vsunif.rs` pins them (`DESIGN`). The eight older
`deqp_vsunif_*` references and their analysis are in
`docs/notes/mali-deqp-vsunif.md` (plan #402). Every count in this note is
**HOST-ONLY**: a compile and link outcome measured on this host with
`tools/mali/deqp-host-measure.sh`. The board statuses quoted are the
iMX95's own.

## 1. The measure

- Board run: the Verdin iMX95 slices `tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/`
  (the plan item calls the run "main a3d8d2de, 2026-11-25"; the run's README
  and every console say `build e63ab28e`). The text appears on **389** lines
  of the consoles, once per case: 389 must-pass cases (all 389 are in
  `tools/cts/gles2-mustpass.txt`), 298 Fail and 91 NotSupported on the board.
  The NotSupported ones are the `indexing.*` and `struct.*dynamic*` cases,
  which dEQP reports as "Dynamic indexing not supported" when the compile
  fails.
- Only slice S4 has a `refusals.tsv`. The case list and a one-slice
  `results.tsv`/`refusals.tsv` were therefore rebuilt from the consoles (the
  `Test case '...'..` line, the first `refused this vertex shader:
  Mali(Valhall: a uniform other than` line under it, the `deqp: <status>`
  line), in `$TMPDIR`, not committed.
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, a blobless
  clone in `$CONNECTED_DEPS_DIR/VK-GL-CTS-host-gles2` without `doc/`, with
  `tools/cts/cts-yakogl-target.patch` and `external/fetch_sources.py`.
- Command (at the tip `8133663c`, and again in a worktree of the train's
  result commit `224cc3f7`, the `intval-rest` train):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host-gles2" \
    --slice "$TMPDIR/vsu/slice" --cases "$TMPDIR/vsu/cases.txt" --out "$TMPDIR/vsu/measure" --jobs 3
```

All 389 cases began on both trees. Four processes exited 1 (dEQP's "not
every case passed"); none crashed.

## 2. The site at `8133663c`

The text has one site left: `nir::uniform_of`
(`src/shaderc/online/mali/nir.rs:1128`, the `_ =>` arm of the `gl_type`
table, `nir.rs:1086-1131`). The table takes float, vec2..4, mat2..4, bool,
bvec2..4, int, ivec2..4, uint and uvec2..4. A struct reaches it only as its
flattened leaf rows, and a sampler has no block row, so neither reaches the
arm. The second site of the 2026-11-24 note (`IrOp::LoadUniform` of a
`GL_INT` read other than as an index, a loop operand or a conversion) is
gone: plan #387's `5e1feec9` (intval vs16-2, "vertex int uniforms read as
their words") removed it. That commit landed after `e63ab28e`, the iMX95
run's build, which is why the board still reports the text.

**At the tip none of the 389 cases meets this text.** The type the iMX95
refused in all of them is `int`/`ivec` (`mediump int ui_*`, the `int`
members of struct uniforms). The tip and the train give the same outcome
except for 13 cases. After the train they stand as follows (the
first refusal per case, the stage of the refusing shader):

| first refusal after the train (`224cc3f7`) | stage | cases | Fail | NS |
|---|---|--:|--:|--:|
| compiles and links (the host's render verdict is void: 107 Fail, 7 Pass on the host) | - | **114** | 90 | 24 |
| arithmetic other than +, -, * ... | vs | 99 | 99 | 0 |
| a loop in a vertex shader | vs | 77 | 63 | 14 |
| a varying lane left unwritten | vs | 50 | 1 | 49 |
| a gl_FragColor lane other than the varying's own lane or a constant | fs | 11 | 11 | 0 |
| a mediump fragment shader uniform other than ... | fs | 6 | 6 | 0 |
| a varying lane computed from no attribute | vs | 5 | 5 | 0 |
| a uniform array | vs | 4 | 4 | 0 |
| a gl_Position lane left unwritten | vs | 3 | 0 | 3 |
| a constant product in a shape Mesa's algebraic rules move or fold | vs | 3 | 3 | 0 |
| control flow (if, a loop, a return before the end) | vs 3, fs 2 | 5 | 5 | 0 |
| a fused multiply-add other than a matrix product's | vs | 2 | 2 | 0 |
| an integer or boolean value | fs | 2 | 2 | 0 |
| a fragment shader that mixes highp and mediump or lowp float values | fs | 2 | 2 | 0 |
| a fragment shader operand of a form or in a place no reference draws | fs | 2 | 2 | 0 |
| a vertex shader uniform that is not highp ... | vs | 1 | 0 | 1 |
| arithmetic (fs), constant product (fs), fragment varying load form (fs) | fs | 3 | 3 | 0 |
| **total** | | **389** | 298 | 91 |

The 13 cases the train moves are all in the "constant operation on a value
that is not a finite normal float" row at the tip. 11 go to "a varying lane
left unwritten", 1 to arithmetic and 1 now compiles. No other case changes.
`tests/mali_deqp_vsunif.rs` passes unchanged on both trees.

## 3. What belongs to another design (not designed here)

| cases (after the train) | dEQP groups (`dEQP-GLES2.functional.`) | text | owner |
|--:|---|---|---|
| 74 | `uniform_api.{value.assigned,value.initial,info_query}.*.{basic_struct,array_in_struct,struct_in_array,nested_structs_arrays}.int_ivec4_*`, `*.multiple_nested_structs_arrays.*` | arithmetic (vs) | **unif2** M3 (steps U3, U5: `deqp_unif2_vs_struct_uniform`) and **intval-vs16** M5/M6 (`deqp_vs16ref_uapi_struct_int_ivec4`). Their `compare_ivec4(u, ivec4(..))` is `ball_iequal4`, which reaches the same `vs_reduce` arm as V1 below. Whichever lands first writes that arm and the other reuses it |
| 4 | `uniform_api.{info_query,value.assigned}.unused_uniforms.int_ivec4_*` | a uniform array (vs) | **unif2** M1 (step U3, int and ivec arrays) |
| 77 | `shaders.loops.{for,while,do_while}_{dynamic,uniform}_iterations.*_vertex` (61), `shaders.struct.local.*loop*_vertex` (6), `shaders.indexing.*.*loop*_vertex` (10) | a loop in a vertex shader | **loops** design (LoopVertex). The bound is an int product `one * ui_n` (`deqp_loops2_vs_for_dyn_nested`), or an int uniform count. The train does not lift any of them |
| 16 + 3 | `shaders.random.{exponential,trigonometric,conditionals,comparison_ops}.vertex.*`, `fragment_ops.interaction.basic_shader.{31,34,53}` | arithmetic (vs) | **arith** step 2 (vertex fp32 built-ins, item `arith-lower-2`) |
| 2 | `shaders.random.trigonometric.vertex.*` | fused multiply-add | **fmaloc** |
| 5 + 3 | `shaders.random.*` (varying from no attribute), `indexing.vector_subscript.*` (gl_Position lane unwritten) | varylink / posvary | **posvary** (the residue after the train's posvary steps) |
| 3 + 1 | `shaders.random.*` | constant product (vs, fs) | **constprod** |
| 3 + 2 | `shaders.random.conditionals.*`, `basic_shader.*` | control flow | **ctrlflow** |
| 25 | fragment shaders of `shaders.random.*`, `basic_shader.*`, `uniform_api.*` | fclane 11, unifmp 6, intval 2, mixprec2 3, opform 2, arith 1 | **fclane**, **unifmp**, **intval-rest**, **mixprec2**, **opform**, **arith** |
| 1 | `shaders.struct.uniform.nested_struct_array_dynamic_index_vertex` | a vertex shader uniform that is not highp | **unif2** (`deqp_unif2_arr_struct_array`'s nested member precision), then V5 |

Overlap with the intval and unif2 designs:

- **intval-vs16** (`docs/notes/mali-intval-vs16-lowering.md`). M5, the int
  uniform as a value, has landed (vs16-2) and is what removed this text.
  M1 narrows a mediump operand where a mediump compare reads it. V3 below
  is the residue M1 does not reach (an fp16 *uniform* compared, the fp16
  select trees). The uniform_api struct compares are its
  `uapi_struct_int_ivec4`.
- **unif2** (`docs/notes/mali-unif2-lowering.md`) owns the int, ivec and
  struct uniform arrays (U3), the struct uniforms' leaf rows and member
  arrays (U5) and the vertex bool chains (U6). This note designs none of
  them. Its `vs_int_uniform` (the `tmp[ui_zero]` subscript read) is V3's
  shape and is already pinned there.
- **plan #402** (`docs/notes/mali-deqp-vsunif.md` section 8) left
  `vec_subscript`, `tmp_array` (V3), `struct_dynidx` (V5), `struct_cmp` (V1)
  and `varying_array` (V2). They are the same steps as here.

## 4. What is left for this design: 56 refused and 23 compiled cases, five steps

| step | mechanism (Mesa 26.2.1) | references | dEQP groups | cases now refused | compiled, words differ |
|---|---|---|---|--:|--:|
| V1 | vertex vector and struct `==` / `!=` | `local_equal`, `local_nested_equal`, `struct_cmp` | `shaders.struct.local.{basic,nested}_{equal,not_equal}_vertex`, `shaders.struct.uniform.{equal,not_equal}_vertex` | **6** (+ the arm the 74 unif2/vs16 cases share) | - |
| V2 | stores at a run-time index into a local vector, matrix or array, or a varying array | `vec2_dynwrite`, `tmp_dynwrite`, `mat2_dynwrite`, `varying_array` | `shaders.indexing.vector_subscript.*_dynamic_subscript_write_*_vertex` (12), `shaders.indexing.tmp_array.*_dynamic_write_*_vertex` (12), `shaders.indexing.matrix_subscript.*_dynamic_write_*_vertex` (9), `shaders.indexing.varying_array.*_dynamic_write_*` (16), `shaders.struct.local.loop_struct_array_vertex` (1) | **50** | - |
| V3 | mediump vertex values at fp16: `V2F32_TO_V2F16`, the `MUX.v2i16` select tree of a run-time read, `FCMP_OR.v2f16` of an fp16 uniform | `vec2_subscript`, `mat2_subscript`, `local_cond_assign`, `vec_subscript`, `tmp_array` | `shaders.indexing.{vector_subscript,tmp_array,matrix_subscript}.*_dynamic_read_vertex` / `*_dynamic_subscript_read_vertex` (18), `shaders.struct.local.{,nested_}conditional_assignment_vertex` (2) | 0 | **20** |
| V4 | a large local array in thread-local storage | `tmp_float_const` | `shaders.indexing.tmp_array.float_{const,static}_write_dynamic_read_vertex` | 0 | **2** |
| V5 | a struct uniform array at a run-time index: `LD_PKA` at `LSHIFT_OR`/`IADD_IMM` offsets | `struct_dynidx` | `shaders.struct.uniform.struct_array_dynamic_index_vertex` (+ `nested_struct_array_dynamic_index_vertex` after unif2) | 0 (+1 after unif2) | 1 |

"Compiled, words differ" are cases that compile at the tip but whose
reference differs: a hazard (they may draw wrong pixels on the board), not a
refusal. The forecast counts V3 to V5 as "compiled, not Mesa's" made
Mesa's, not as new compiles.

The other compiled struct cases are byte-equal at the tip: `random_dead_int`,
`struct_local`, `struct_int` (plan #402), and `local_array_member`,
`local_parameter`, `unif_struct_array` (this item). They stand for
`shaders.struct.local.{basic,nested,array_member,parameter,parameter_nested,return,return_nested,struct_array,nested_struct_array}_vertex`,
`shaders.struct.uniform.{nested,struct_array}_vertex` and the 58 compiled
`shaders.random.*` cases. The `random.*` ones are not all checked one by
one.

### V1. Vertex vector and struct `==` (6 cases, then the 74 shared)

Site: `nir::vs_reduce` (`nir.rs:4417`), its last arm `_ =>
Err(MaliError::Arithmetic)` (`nir.rs:4544`). The function takes
`(Ball, Feq, 2)` against a zero, `(Bany, Flt, 3)`, constant folds, and the
width-three 16-bit mask and bool chains. A probe at the tip (`if (p == q)`
of two computed mediump vec3s; `u_v == ivec4(..)` of a mediump ivec4
uniform) gets this refusal. A scalar float `==` and an `int == int` compile.
A struct `a == b` is the member-wise `ball_fequal3` / `ieq` AND, so it
reaches the same arm.

Mesa (`local_equal`, `vs.varying`): the mediump vec3 members are narrowed
(`FADD.f32 rX.h00, x, 0x0.neg`, `MKVEC.v2i16` with the constant lane) and
compared as one fp16 pair `FCMP_OR.v2f16.eq.m1` (the constant `.x` lanes fold
away), the pair's two masks are joined with `IADD.v2u16 r.h11, 0` /
`LSHIFT_AND.v2i16` and widened with `S16_TO_S32`, the scalar float member is
`FCMP_OR.f32.eq.m1`, the int member is `ICMP_OR.u32.eq.m1 u5.w1, <MOV.i32 of
u6.w0>`. The masks are `LSHIFT_AND.i32`, and the colour lane is `CSEL.u32.eq
mask, 0, 0, 0x3F800000`. A compare whose members are all equal constants
folds (`a == c` of identical locals is not compiled: one lane stores the
constant). Lowering: a `vs_reduce` arm for `Ball`/`Bany` of `Feq`/`Fne` at n
= 3 over 16-bit operands (the pair plus the odd lane, Mesa's order above),
and `Ieq`/`Ine` at n = 4 over 32-bit int words (the `ICMP_OR`/`ICMP_AND`
chain of `deqp_unif2_vs_struct_uniform`). Then the struct's member AND in
`ball` order.

### V2. Stores at a run-time index (50 cases)

Site: the vertex walk's varying-store check, `MaliError::VaryingForm`
("a varying lane left unwritten", `mod.rs:517`). A store `tmp[ui_k] = x` into a local
vector, matrix or array (or a varying array) at a run-time index leaves the
walk without a known lane, so the varying built from it counts as unwritten.

Mesa runs `nir_lower_indirect_derefs`: each element `e` gets `bcsel(index
== e, x, old)`. The int index is compared raw from its FAU word against byte
immediates (`ICMP_OR.u32.eq.m1 u5.w1, 0` for element 0, `ICMP_OR.s32.lt.m1
u5.w1, 0x1000000.b3/.b2`, a `<` chain for more elements, combined with
`LSHIFT_OR`/`LSHIFT_AND .neg`). The values are fp16 (V3's narrowing), so
the select is `MUX.v2i16.int_zero` over a 16-bit mask pair (`MKVEC.v2i16
mask0.h00, mask1.h00`) or one mask `.h00`. After the stores the reads at
constant indices are plain registers (`vec2_dynwrite`: one `MUX.v2i16`,
`FADD.v2f16 r.h00, r.h11`, `F16_TO_F32`; `tmp_dynwrite`: two `MUX.v2i16` per
element and store, 4 × 4 of them; `mat2_dynwrite`: one `ICMP_OR.s32.lt.m1`
and two `MUX.v2i16`). The `varying_array` writes are the same at fp32 with
`CSEL.u32.eq` per lane (`docs/notes/mali-deqp-vsunif.md` section 5).
Lowering: in the vertex walk, a run-time-index store into a local becomes
per-element selects on the index, in Mesa's element order, before the
varying-lane check. It needs V3 for the fp16 selects to be byte-equal (at
fp32 it would compile, but not to Mesa's words). Forecast: 50 cases leave
this text. Their next refusal is not measured; the `*_loop_read` ones (12)
also read in a loop and may then meet the loops text.

### V3. Mediump vertex values at fp16 (20 compiled cases)

At the tip these compile with fp32 values. Mesa keeps a mediump local at
fp16 in the vertex shader too (`nir_lower_mediump` /
`LowerPrecisionFloat16`):

- `vec2_subscript`, `mat2_subscript` (and plan #402's `vec_subscript`,
  `tmp_array`): the attribute is narrowed with `V2F32_TO_V2F16` (`FADD.f32
  r.h00, z, 0x0.neg` for an odd lane), scaled with `FMA.v2f16` by fp16
  constants (`0x38000000.h11` = 0.5), and a run-time read `tmp[ui_k]` is a
  `MUX.v2i16` tree on `ICMP_OR.s32.lt.m1` of the int word against byte
  immediates. It ends with `FADD.v2f16` and `F16_TO_F32` before the varying
  store. First difference: instruction 1 of `vs.varying` (ours `FMA.f32`).
- `local_cond_assign`: `if (uf_one > 0.0)` of a mediump float uniform is
  `FCMP_OR.v2f16.lt.m1 r0, 0x0, u6.w1.h00, 0x0` and `S16_TO_S32` (ours:
  `F16_TO_F32` of the uniform, then an f32 compare), then the same
  `CSEL.u32.eq` selects. Only the order differs after that.

This is intval-vs16's M1 extended from compares of attributes to locals
and fp16 uniforms. Forecast: 20 compiled cases become Mesa's words
(`indexing.*_dynamic_read_vertex` 18, `struct.local.*conditional_assignment_vertex`
2), no change in the refusal count. V2's 50 depend on it to be byte-equal.

### V4. Thread-local storage of a large local array (2 compiled cases)

`tmp_float_const` (`mediump float arr[40]`, read at four run-time indices):
Mesa does not make select trees. It moves the array to scratch
(`nir_lower_vars_to_scratch`): `vs.position` (104 words against our 24)
stores every element's fp16 constant with `IADD_IMM.i32 r0, 0x0, #imm` and
`STORE.i16.force.slotN @r0, thread_local_pointer.w0, offset:2k`, and loads
them at `index * 2`. The secondary offset is 896 against our 256. Lowering
needs a thread-local storage area in the driver's shader descriptor (the
TLS size, `thread_local_pointer`) and the size threshold Mesa uses to pick
scratch over `nir_lower_indirect_derefs`. Forecast: 2 compiled cases
(`tmp_array.float_{const,static}_write_dynamic_read_vertex`) become Mesa's.
This is a design of its own; do it last.

### V5. A struct uniform array at a run-time index (1 compiled case, 1 more after unif2)

Plan #402's `struct_dynidx`: `s[ui_k].a` is `LD_PKA.i32` at `LSHIFT_OR.i32
index, 0x3020100.b33` (× the struct stride) plus `IADD_IMM` of the member
offset (`docs/notes/mali-deqp-vsunif.md` section 5). Ours compiles to a
select tree. It is the vertex counterpart of `UniformIndexed`'s `LD_PKA`
(the vsunifarr design's family). Forecast: 1 compiled
(`struct.uniform.struct_array_dynamic_index_vertex`) plus the 1 nested case
once unif2 admits its member precision.

## 5. The references (plan #387, this item)

Each is the case's first program, verbatim from the tip's `.qpa` (each source
ends in a newline). Each was drawn with

```sh
LLVM_DIR=/usr/lib/llvm-21 tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_vsunif_<shape> --instrument shim
```

on Mesa 26.2.1's G610 drm-shim (`$CONNECTED_DEPS_DIR/mesa-shim`, built with
`tools/mali/drm_shim_ref.sh build`). As a self-check, this shim redrew
plan #402's `deqp_vsunif_struct_local.expect.json` byte for byte.
`tools/mali/g310_check.sh --since HEAD` gives **12 identical, 0 differ** on
the G310v2. States at `8133663c` (identical at `224cc3f7`):

| reference | case (`dEQP-GLES2.functional.shaders.`) | state | step |
|---|---|---|---|
| `local_equal` | `struct.local.basic_equal_vertex` | refused, arithmetic | V1 |
| `local_nested_equal` | `struct.local.nested_equal_vertex` | refused, arithmetic | V1 |
| `vec2_dynwrite` | `indexing.vector_subscript.vec2_dynamic_subscript_write_component_read_vertex` | refused, varying lane unwritten | V2 (+V3) |
| `tmp_dynwrite` | `indexing.tmp_array.vec4_dynamic_write_static_read_vertex` | refused, varying lane unwritten | V2 (+V3) |
| `mat2_dynwrite` | `indexing.matrix_subscript.mat2_dynamic_write_static_read_vertex` | refused, varying lane unwritten | V2 (+V3) |
| `vec2_subscript` | `indexing.vector_subscript.vec2_component_write_dynamic_subscript_read_vertex` | differs (vs.varying 9 / 20 words, fs.fragment 10 / 8) | V3 |
| `mat2_subscript` | `indexing.matrix_subscript.mat2_static_write_dynamic_read_vertex` | differs (vs.varying 27 / 30 words) | V3 |
| `local_cond_assign` | `struct.local.conditional_assignment_vertex` | differs (vs.varying, instruction 0) | V3 |
| `tmp_float_const` | `indexing.tmp_array.float_const_write_dynamic_read_vertex` | differs (vs.position 24 / 104 words, TLS) | V4 |
| `local_array_member` | `struct.local.array_member_vertex` | byte-equal | - |
| `local_parameter` | `struct.local.parameter_vertex` | byte-equal | - |
| `unif_struct_array` | `struct.uniform.struct_array_vertex` | byte-equal (members' values, `STRUCT`) | - |

## 6. Order and forecast

Ordered by the cases each step unblocks (HOST-ONLY, of the 389 iMX95
cases, after the train):

1. **V2** (50 refused, then needs V3 for Mesa's words). It is the one step
   with a large count that nobody else owns.
2. **V3** (20 compiled made Mesa's, and V2's 50 byte-equal). Do it with V2 or
   just before it. It extends intval-vs16 M1.
3. **V1** (6 refused). It writes the `vs_reduce` arm that the 74
   unif2/vs16 `uniform_api` cases need too. If unif2 U5 lands first, V1 is
   the struct member AND only.
4. **V5** (1 compiled, 1 after unif2), with the vsunifarr design's `LD_PKA`
   vertex path.
5. **V4** (2 compiled), last: it needs thread-local storage in the driver.

Total: 56 must-pass cases leave a refusal (V1 + V2), and 23 compiled
ones become Mesa's words (V3 to V5: 20 + 2 + 1, plus V2's 50 once they
compile). The 389 board lines of this text go to 0 at the next board rerun from any tree that holds `5e1feec9`, whatever
this design lands. The cases then show under the texts of section 2.

## 7. Open

- The 114 cases that compile are not all checked against Mesa. The six
  byte-equal references stand for the struct groups. The 58 compiled
  `shaders.random.*` cases have only `random_dead_int` (plan #402).
- V2's next refusal is not measured. A prototype should rerun the measure of
  section 1 over the 50 cases.
- The local struct arrays at a run-time index
  (`shaders.struct.local.{struct_array,nested_struct_array,array_member}_dynamic_index_vertex`,
  3 NS, compiled) have no reference. They are probably V3's select tree over
  fp16 members. The next item should draw one before it counts them under V3.
- V4's threshold (when Mesa picks scratch over select trees) is not read
  from Mesa's source here. `tmp_array.vec4_*` with 4 elements uses select
  trees, and the 40-element float array uses scratch.
