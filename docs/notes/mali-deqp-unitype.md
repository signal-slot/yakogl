<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): dEQP-GLES2 uniform type and precision refusals

Plan #377 item `study-unitype` (issue #20, stage 5 of issue #7). This is a
study only. It changes no code and records no reference. It takes three
compile refusals from the whole-tree NanoPC-T6 run at `37305608`
(`docs/es2-cts-failures.md` section M; the four slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/refusals.tsv`):

| refusal (`MaliError`) | text (`src/shaderc/online/mali/mod.rs:414-426`) | Fail | NotSupported |
|---|---|---|---|
| `UniformType` | a uniform other than a float, vec2, vec3, vec4 or mat4 | 362 (vertex) | 94 (vertex) |
| `UniformPrecision` | a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member | 77 | 2 |
| `FsUniform` | a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp | 285 | 0 |

These are the counts of section M's tally block. Re-counted from the
`refusals.tsv` and `results.tsv` of the four slices, they are the same:
456, 79 and 285 cases, 820 in all. Each case has its first refusal only.
For a `_both` case that is the vertex stage's refusal. The fragment
stage's refusal is not known until the vertex stage passes (section 2,
"behind it").

The note answers three questions:

1. Where does our compiler refuse? Section 1.
2. Which dEQP shapes stand behind the 820 cases? Section 2.
3. What does Mesa 26.2.1 do with each type and precision (packed storage,
   fp16 uniforms, FAU push)? Section 3.

It then gives a lowering plan sized for one session (section 4) and the
reference cases `ref-unitype` makes (section 5). It builds on three earlier
notes and does not repeat them:

- `docs/notes/mali-uniform-types.md`: the packed layout per type, and
  cases 100, 116, 166 and 80.
- `docs/notes/mali-int-uniforms.md`: int, uint and bool uniforms in the FAU.
- `docs/notes/mali-ubo-rows.md`: the row tables.

**Scope.** This note covers each uniform's type, layout and precision: where
its words are, how it is loaded and how it is converted. The arithmetic on
int and bool values belongs to `study-intval`, and uniform arrays belong to
`study-uniarray`. Their rows are counted in section 2 so that the totals
add up, and they are marked as handed off.

## Citations

- Ours: `<path>:<line>` from the repository root at `38ad30eb` (the
  integration tip this item started from).
- Mesa: `M:<path>:<line>`, where `<path>` is under `src/` of the Mesa 26.2.1
  tree (`MESA_PAN_TREE`, `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`).
- dEQP: `D:<file>`, a file of VK-GL-CTS at `93bca01861b0` (the commit that
  `DEQP_WORK` pins, `tools/cts/deps.toml`), under
  `modules/gles2/functional/`, `modules/glshared/` or `data/gles2/shaders/`.
  The dEQP sources were read on GitHub at that commit. No checkout was made
  here (see "Assumptions" in the item report). The shader text quoted below
  comes from those files. It is not generated output: the `measure` item
  commits the generated pairs (`tools/mali/testdata/deqp_shapes/`).

## 1. The refusal sites

| refusal | site | what it checks |
|---|---|---|
| `UniformType`, the type | `src/shaderc/online/mali/nir.rs:1012-1027` (`uniform_of`, the `_ =>` arm at `:1027`) | It takes `float`, `vec2/3/4`, `mat4`, `int`, `uint`, `bool`, `ivec2/3/4` and `uvec2/3/4`. `mat2`, `mat3`, `bvec2/3/4` and the non-square matrices fall through. `uniform_of` serves the vertex stage and, through `uniform_extent` (`nir_fs.rs:5867-5889`, `:5888`), the highp fragment stage. |
| `UniformType`, an `int` read as a value | `nir.rs:2927-2929` (`IrOp::LoadUniform` in the vertex walk) | A `GL_INT` uniform whose every read is not a uniform array's index (`index_only`, `nir.rs:1052-1062`). It checks `GL_INT` alone, not `ivec`, `uint` or `bool`. |
| `UniformPrecision` | `mod.rs:1302-1309` (the vertex stage, after `UboLayout::vs`) | A used uniform that is not `Precision::High` (`uniform_precision`, `mod.rs:1197-1218`) and has no fp16 word (`ubo.half(..)` is `None`). Only `nir::fp16_uniform` (`nir.rs:1042-1045`: float, vec2, vec3, vec4 or mat4, no array) makes fp16 words. A mediump struct member, a mediump array, and a uniform whose precision `uniform_precision` cannot find (a member of a nested struct or of an array of structs, `gl_DepthRange.*`) fall here. |
| `FsUniform`, the type | `select.rs:448-460` (`UboLayout::fs16`, the `_ =>` arm at `:460`) | A mediump fragment shader takes `int` and `bool` (one word, no array), `bvec2/3/4`, `float`, `vec2/3` (no array) and `vec4` (an array too). `ivec2/3/4`, every matrix, and the arrays of `int`, `bool`, `bvec`, `ivec`, `vec2` and `vec3` fall through. |
| `FsUniform`, highp | `select.rs:462-464` | A float uniform declared `highp` in a shader that `mod.rs` selects as mediump (`mod.rs:1455-1475`). Mesa keeps such a uniform fp32. |
| `FsUniform`, the read | `nir_fs.rs:6854`, `:6903`, `:6913`, `:6917`, `:7148`, `:7170`, `:7427`, `:7584`, `:8042`, `:8438`, `:8441`, `:8486`; `vec16.rs:1086-1087` | A word of the fp16 block that is read other than as its own aligned lane pair or one lane in both halves. These sites are reached only after the type sites pass. |

The front end lays out its own block like std430 (`reflect.rs:136-175`,
`type_layout`). A `vec3` and each `mat3` column are aligned to 4 dwords, and
a `mat2` column is 2 dwords. Mesa's packed block aligns nothing (section 3).
`select::UboRow` (`select.rs:238-266`) is the bridge between the two: a
row is `comps` consecutive front-end dwords placed at Mesa's word `word`.
A `mat3` is therefore three rows, one per column (front-end dwords 0-2,
4-6 and 8-10; Mesa words 0-2, 3-5 and 6-8). A `mat2` (front-end dwords 0-3
with no hole, Mesa words 0-3) is one row. The frame keeps the same rows
(`src/mali/frame.rs:540-557`, `UboRow`) to fill the block it uploads.

## 2. The dEQP shapes

The counts below come from the four slices' `refusals.tsv`, joined to
`results.tsv` for the status. They are grouped by case name, and each row's
shader is described from its dEQP generator. "Behind it" means the refusal
or work that is expected next once this type or precision is taken. It is
inferred from the source and not measured: the `measure` item measures it.

### `UniformType` (vertex stage), 362 Fail + 94 NotSupported

| shape | cases (Fail / NS) | example case | the uniform(s) | Mesa layout | owner | behind it |
|---|---|---|---|---|---|---|
| T1. Matrix ops, `uniform highp mat2/mat3` | 38 / 0 | `shaders.matrix.mul.uniform_highp_mat3_vec3_vertex` | `uniform highp matN u_in1` (D:es2fShaderMatrixTests.cpp, `ShaderMatrixCase::init`). For a `dynamic_*` case the first operand is `attribute matN a_matN` and the second a uniform. | fp32: mat2 is 4 words, mat3 is 9 words, column by column | **unitype** | `div` (componentwise `fdiv`), `matrixCompMult`, `++/--`, and matrix attributes: Arithmetic and attribute refusals of their own |
| T2. Matrix ops, `uniform lowp/mediump mat2/mat3` | 76 / 0 | `shaders.matrix.add.uniform_mediump_mat2_mat2_vertex` | the same, at lowp and mediump | fp16 (lower_precision): mat2 is 2 words, mat3 is 6 words (3 halves and 1 pad half per column) | **unitype** | the same as T1, in VS fp16 arithmetic |
| T3. `uniform_api` `mat2`, `mat3`, `struct { mat4; mat2 }` | 30 / 0 | `uniform_api.value.initial.render.basic.mat3_vertex` | `uniform mediump mat2/mat3` and a struct `mat4_mat2` (D:es2fUniformApiTests.cpp: non-bool types are `PRECISION_MEDIUMP`). The shader does `v_vtxOut *= compare_matN(u, matN(...))`, with `abs(a - b) < 0.05 ? 1.0 : 0.0` per lane. | a plain uniform is fp16; a struct member is fp32 (section 3.2) | **unitype** (type); the compare's `?:` is `FCMP` + `CSEL`, already lowered for floats | the 0.05 compare in VS fp16 |
| T4. `state_query.shader.uniform_value_matrix` | 1 / 0 | (itself) | `mat2`, `mat3`, `mat4` in the vertex shader | as T3 | **unitype** | none expected (the case reads the values back with `glGetUniform`) |
| T5. `light_amount.*` | 18 / 0 | `light_amount.4dir` | `uniform mat3 u_normalMatrix` (no qualifier, so VS default highp) and an array of `struct Light` read in loops (D:es2fLightAmountTest.cpp) | mat3 fp32, 9 words | **unitype** (the mat3); then uniarray and loops | `u_lights[ndx]` at a loop index: `UniformArray`/`UniformIndexed`, `LoopVertex` |
| T6. `uniform_api` `bvec2/3/4` | 60 / 0 | `uniform_api.value.assigned.by_value.render.basic.bvec3_vertex` | `uniform bvecN` (bool types have no precision). Read with `compare_bvecN(a, b)` = `a == b ? 1.0 : 0.0`. | one 32-bit word per lane, true = `~0` (section 3.3) | **unitype** (load); the `==` of bvecs (`ball_iequal`) is **intval** | `IntValue` / `CompareForm` at the bvec `==` |
| T7. `linkage.uniform_struct_partial_bvec2_vec2` | 1 / 0 | (itself) | `struct { bvec2 a; mediump vec2 b; }`, `res = float(val.a.x)` (D:data/gles2/shaders/linkage.test) | `a` is 2 words (`~0`/0); `b` is fp32 (a struct member) | **unitype** | `float(bool)` (`b2f32`): intval |
| T8. `uniform_api` int / `int_ivec4`, no array | 28 / 0 | `uniform_api.value.initial.render.basic.int_vertex` | `uniform mediump int`, or `struct { mediump int; mediump ivec4; }`, compared with `a == b` | one 32-bit word (int16 is off: section 3.4) | load: **unitype**, trivial; the use: **intval** | an int `==` in VS |
| T9. `state_query.shader.uniform_value_int` | 1 / 0 | (itself) | `int`, `ivec2/3/4` in the vertex shader | as T8 | intval | |
| T10. `uniform_api` `int_ivec4` in arrays (array_in_struct, struct_in_array, nested_structs_arrays, unused_uniforms) | 50 / 0 | `uniform_api.value.assigned.by_value.render.struct_in_array.int_ivec4_vertex` | arrays of structs with int and ivec4 members | as T8, element by element | **uniarray** + **intval** | |
| T11. `shaders.struct.local/uniform.*_vertex` | 25 / 10 | `shaders.struct.local.basic_vertex` | `uniform int ui_one` (no qualifier), struct members `mediump int` and `sampler2D` (D:es2fShaderStructTests.cpp). `S s = S(..., ui_one)`; the `_dynamic_index` cases index by it. | as T8 | **intval** (int as a struct member value, an index); the 10 NS are indexed locals | `IntValue`, `IndexedLocal`, `LoopVertex` |
| T12. `shaders.indexing.{tmp_array,varying_array,vector_subscript,matrix_subscript}.*_vertex`, `shaders.loops.*.vector_counter_vertex` | 0 / 84 | `shaders.indexing.tmp_array.vec2_dynamic_write_dynamic_read_vertex` | `uniform mediump int ui_zero, ui_one, ...` as run-time indices of locals and varyings, or as loop bounds (D:es2fShaderIndexingTests.cpp, D:es2fShaderLoopTests.cpp, `createSpecialLoopCase`) | as T8 | **intval** / **loops** | `IndexedLocal`, `LoopVertex` |
| T13. Random programs: `shaders.random.*`, `fragment_ops.interaction.basic_shader.*`, `uniform_api.random.*` (vertex) | 34 / 0 | `uniform_api.random.0` | rsg and `UniformCollection::random` mixes. Arrays have a 30% chance and structs a 20% chance, and the basic type is uniform over the type list. | per type | **measure** splits them | |

T1 to T5 and T6/T7 (the load of the bvec words) are this family's: 224
cases. T8 to T12 (198 cases, of them all 94 NotSupported) are int values.
Their load has no instruction of its own: Mesa reads the FAU word raw
(`intuni_*`, `mali-int-uniforms.md`). Their refusal at `nir.rs:2927` is
"an int value in the vertex stage" under another name. Section 4, step 0
says what to do with that site.

### `UniformPrecision` (vertex stage), 77 Fail + 2 NotSupported

| shape | cases (Fail / NS) | example case | the uniform(s) | Mesa layout | owner | behind it |
|---|---|---|---|---|---|---|
| P1. `linkage.uniform_struct*` | 21 / 0 | `linkage.uniform_struct_partial` | `struct Struct { mediump float a; mediump float b; } val;` and its vec2, vec3, vec4, int and ivec2 variants. `res = val.a`, `dummy = val.a + val.b`, `val.a.x + val.b.y`. `use_case_rip` mixes highp and mediump members (D:data/gles2/shaders/linkage.test). | fp32: a struct is not lowered (section 3.2). Each mediump read becomes `f2fmp`. | **unitype** | the fp16 add and its widening to the 32-bit varying |
| P2. `uniform_api` `basic_struct.float_vec4` | 14 / 0 | `uniform_api.value.initial.render.basic_struct.float_vec4_vertex` | `struct { mediump float; mediump vec4; }`, `compare_float`/`compare_vec4` | fp32, as P1 | **unitype** | the 0.05 compares in fp16 |
| P3. `builtin_variable.depth_range_vertex` | 1 / 0 | (itself) | `gl_DepthRange.near/far/diff` (highp in GLSL ES 1.00). `uniform_precision` finds no declaration for the name. | three fp32 state words (`STATE_DEPTH_RANGE`, the parameter list) | **unitype** (the precision lookup), together with F7 | none expected |
| P4. `uniform_api` `basic_array.vec4`, `basic_array_first_elem_without_brackets.vec4`, `struct_in_array.float_vec4`, `unused_uniforms.float_vec4`; `shaders.indexing.uniform_array.{vec4_static_read,float_dynamic_read,vec4_dynamic_read}_vertex` | 41 / 2 | `uniform_api.value.initial.render.basic_array.vec4_vertex` | `uniform mediump vec4 u[N]`, arrays of structs with mediump members | a mediump float array is fp16 (`glsl_without_array` is float), 2 words per vec4 element. A struct array is fp32. | **uniarray** | |

P1 to P3 are this family's: 36 cases.

### `FsUniform` (fragment stage), 285 Fail

| shape | cases | example case | the uniform(s) | Mesa layout | owner | behind it |
|---|---|---|---|---|---|---|
| F1. Matrix ops, `uniform lowp/mediump mat2/3/4` | 74 | `shaders.matrix.add.uniform_mediump_mat3_float_fragment` | `uniform mediump matN u_in<k>` (and for `dynamic_*`, a matrix varying) | fp16: mat2 is 2 words, mat3 is 6, mat4 is 8 (one column per 1, 2 or 2 words) | **unitype** | `dynamic_*`: a matrix varying (`FsVaryingLoad`/colour forms); `div`, `matrixCompMult` |
| F2. Matrix ops, `uniform highp mat2/3/4` | 37 | `shaders.matrix.sub.uniform_highp_mat2_mat2_fragment` | highp matrices in a shader whose `gl_FragColor` is mediump. The type refusal comes first; the highp one (`select.rs:462`) is behind it. | fp32 | **mixprec** (highp arithmetic into the fp16 colour) | `MixedPrecision` |
| F3. `uniform_api` `ivec2/3/4`, `basic_struct.int_ivec4`; `negative_api.shader.uniform_matrixfv_*` | 32 | `uniform_api.value.initial.render.basic.ivec3_fragment` | `uniform mediump ivecN`, or the struct's `ivec4` member. The negative-API cases are `uniform mediump ivec4 fTest; ... gl_FragColor.xy = vec4(fTest).xy;` (D:es2fNegativeShaderApiTests.cpp, `uniformTestFragSource`). | one 32-bit word per lane | load: **unitype**; `vec4(ivec)` (`i2f`) and the ivec `==`: **intval** | `IntValue` |
| F4. `fragment_ops.scissor.*` | 17 | `fragment_ops.scissor.contained_tri` | `uniform highp vec4 u_color; gl_FragColor = u_color;` with no default precision (D:glsScissorTests.cpp, `genShaders`) | fp32, 4 words. `gl_FragColor` is a mediump output, which `nir_lower_mediump_io` narrows to fp16 (section 3.5). | **unitype** (one declared highp) | none expected: the colour is the uniform |
| F5. `builtin_variable.depth_range_fragment` | 1 | (itself) | `gl_DepthRange.*` (highp) into the mediump colour | fp32 state words | **unitype**, with P3 | |
| F6. `uniform_api` `multiple_basic.fragment` | 5 | `uniform_api.value.initial.render.multiple_basic.fragment` | `float`, `ivec3`, `mat3` and `bvec2` (D:es2fUniformApiTests.cpp) | per type | **unitype** (mat3 and ivec3) + **intval** | the ivec `==` |
| F7. Arrays: `uniform_api` int/bool/bvec4/ivec4 arrays and struct arrays, `shaders.indexing.uniform_array.vec2/vec3_static_read_fragment`, `shaders.struct.uniform.loop_nested_struct_array_fragment` | 100 | `uniform_api.value.initial.render.basic_array.int_fragment` | arrays | | **uniarray** | |
| F8. Random programs (`shaders.random.*`, `uniform_api.random.*`, fragment) | 19 | `uniform_api.random.36` | mixes | | **measure** | |

F1, F3 (the load), F4, F5 and F6 are this family's: 129 cases. F2 is
mixprec's 37.

### What the family holds

| | cases | Fail | NS |
|---|---|---|---|
| unitype proper (T1-T7, P1-P3, F1, F3-F6) | 389 | 389 | 0 |
| int values (T8, T9, T11, T12; the load trivial) to **intval** / **loops** | 148 | 54 | 94 |
| arrays (T10, P4, F7) to **uniarray** | 193 | 191 | 2 |
| highp matrices in an fp16-colour shader (F2) to **mixprec** | 37 | 37 | 0 |
| random programs (T13, F8) to **measure** | 53 | 53 | 0 |
| total | 820 | 724 | 96 |

None of the 389 is expected to pass on the type alone. Every shape has a
use of its own behind the load (section 2, "behind it"). The ones whose use
is plain float arithmetic of kinds already lowered are F4 (17: the colour
*is* the uniform), P1 (21: `res = val.a`, `val.a + val.b`) and the
`add`/`sub`/`mul` rows of T1, T2 and F1. Those are where a lowering shows
passes first.

## 3. What Mesa 26.2.1 does

### 3.1 Packed storage and UBO 0

panfrost asks for packed uniforms (`M:gallium/drivers/panfrost/pan_screen.c:747`
`caps->packed_uniforms = true`), so `PackedDriverUniformStorage` is set
(`M:mesa/state_tracker/st_context.c:524-525`). The uniforms are lowered with
`nir_lower_io` and a type size of `glsl_count_dword_slots`
(`M:mesa/state_tracker/st_glsl_to_nir.cpp:688-692`, `:701-716`), then turned
into UBO 0 by `nir_lower_uniforms_to_ubo(nir, dword_packed=true, ...)`
(`:713-716`). That pass multiplies the offset by 4, not 16
(`M:compiler/nir/nir_lower_uniforms_to_ubo.c:86-92`). A constant offset
gets its exact alignment (`:102-104`); a run-time one gets
`MAX2(4, bit_size / 8)` (`:106`).

`glsl_count_dword_slots` (`M:compiler/glsl_types.c:2961-3011`) aligns and
pads nothing:

| type | dwords | where |
|---|---|---|
| `float`, `int`, `uint`, `bool`, their vectors, fp32 matrices | `glsl_get_components`: vec3 is 3, mat2 is 4, mat3 is 9, mat4 is 16 | `:2964-2968` |
| 16-bit (`float16` after lower_precision) | `DIV_ROUND_UP(vector_elements, 2) * matrix_columns`: vec3 and vec4 are 2, mat2 is 2, mat3 is 6, mat4 is 8 | `:2969-2972` |
| sampler | 0 (not bindless) | `:2976-2980` |
| array | element times length, with no stride padding | `:2986-2988` |
| struct | the sum of the members | `:2991-2998` |

A 16-bit column therefore starts a dword. A mediump `mat3` column is three
halves and one pad half, and the next column starts the next word. The
uniforms follow one another in declaration order, and an unused one gets no
word (`sel_u_unused`). Inside a struct the members are consecutive, and an
fp32 member after an fp16 one cannot occur (section 3.2).

### 3.2 Which uniforms become fp16

`lower_precision` runs on ES shaders when the stage has fp16 or int16
(`M:compiler/glsl/glsl_parser_extras.cpp:2488-2491`). panfrost sets
`fp16`, `glsl_16bit_consts` and `fp16_const_buffers` for every stage from
v6 (`M:gallium/drivers/panfrost/pan_screen.c:585-588`), so for the vertex
stage too, and `int16 = false` (`:591`). A uniform variable is retyped to
16 bits only under the conditions of
`lower_variables_visitor::visit(ir_variable)`
(`M:compiler/glsl/lower_precision.cpp:1022-1035`):

- It is not in a buffer block, and `glsl_without_array(var->type)->base_type
  == GLSL_TYPE_FLOAT` (`:1029-1031`). So **a float scalar, vector or
  matrix, or an array of one, is lowered**: mediump `mat2/mat3/mat4` and
  mediump float arrays become fp16. **A struct is not lowered**, whatever
  its members' precision, because its base type is `GLSL_TYPE_STRUCT`. **An
  int is not lowered**, because `can_lower_type` asks for `int16`
  (`:176-177`).
- It is mediump or lowp (`:1033-1034`).

A read of a variable that was not lowered, inside an expression that is
lowered, is converted at the read. `find_lowerable_rvalues_visitor` takes
the precision of the dereference, including a struct member's
(`ir_dereference_record`, `:362-370`). `lower_precision_visitor::handle_rvalue`
wraps a dereference in `convert_precision(false, ...)`, an `f2fmp`
(`:724-734`, `:683-705`), and does not lower the struct itself (`:761-765`).
So a mediump struct member is **an fp32 load followed by `f2fmp`** where a
mediump operation reads it. An fp16 uniform (a plain mediump float) is a
16-bit `load_ubo` that a highp operation widens with `f2f32`. Our vertex
stage already does this (`nir.rs:2941-2951`, `vsfp16_19_hello`:
`F16_TO_F32 r4, u4.w1.h1`).

### 3.3 Bool uniforms

Mesa stores a bool uniform as a 32-bit word with true = `~0`:
`UniformBooleanTrue` is `~0U` when `NativeIntegers` is set
(`M:mesa/state_tracker/st_extensions.c:1356`), and `glUniform*` writes
it (`M:mesa/main/uniform_query.cpp:1398-1441`). The load is 32-bit and
becomes a 1-bit bool with `b2b1`, which is `ine x, 0`
(`M:compiler/nir/nir_lower_io.c:544-549`). The back end makes bools
32-bit again (`pan_nir_lower_bool_to_bitsize`,
`M:panfrost/compiler/bifrost/bifrost_nir.c:350`). Recorded in the
fragment stage: a bool is `MOV.i32 r0, u5.w0` and a `CSEL.u32.eq` against
0 (intbool_76_boolany), or its low half tested with `MUX.v2i16.int_zero`
(case 112, `mali-int-and-size.md`). A `bvecN` takes N words. Our
fragment stage takes them already (`select.rs:451-455`). The vertex
stage's `uniform_of` lacks `GL_BOOL_VEC2/3/4`.

Whether true is `1` or `~0` matters only where Mesa uses the raw word as a
mask (`LSHIFT_AND` of two bools, then used as a 0/~0 select). The
reference must set true values to show this (section 5, caveat 2).

### 3.4 Int uniforms

An int uniform is one 32-bit word per lane, read raw, at every precision:
`int16` is off (`pan_screen.c:591`), and `can_lower_type` refuses ints
(`lower_precision.cpp:176-177`). Recorded: `intuni_*`, `uniidx_d`,
`unitype_80`. The load adds nothing to what `uniform_of` gives. What
remains is the arithmetic (intval).

### 3.5 FAU push and 16-bit loads

`bi_opt_push_ubo` (`M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:150-210`)
pushes every `load_ubo` that has a constant address and a constant UBO and
whose offset is a multiple of 4 (`bi_is_pushable_ubo`, `:21-32`). Its range
is the opcode's `sr_count` words (`:62-63`, `:82-84`), and the ranges are
picked as `mali-uniform-types.md` section 3 describes (`:121-147`). A
16-bit load of one or two halves is a one-word range. Its halves are read
as `uN.wM.h0`/`.h1` or as `v2f16` (`bi_emit_load_ubo`,
`M:panfrost/compiler/bifrost/bifrost_compile.c:983-993`). A 16-bit load
whose byte offset is 2 mod 4 would not be pushable. It would come from the
odd half of a word read alone, if a pass shrank the load to it. No recorded
reference shows one (the vec2 and vec4 fp16 reads are whole words). The
mediump mat3 references (section 5) settle it for the pad half.

The fragment stage narrows its mediump outputs:
`nir_lower_mediump_io(nir, nir_var_shader_in | nir_var_shader_out, ...)`
runs only for the fragment shader
(`M:panfrost/compiler/bifrost/bifrost_nir.c:913-916`). So F4's
`gl_FragColor = u_color` (highp) is four fp32 FAU words, an `f2fmp` per
lane (two `V2F32_TO_V2F16` expected), and the fp16 colour. The vertex
stage keeps its outputs 32-bit (the same comment, `:913`), so P1's
`f2fmp` chain ends in an `f2f32` before the varying store. Whether
`nir_opt_algebraic` folds `f2f32(f2fmp(x))` to `x` for `res = val.a` is
for the reference to show.

## 4. Lowering plan (`lower-unitype`, one session)

Every step is a host test, `tests/mali_deqp_unitype.rs`, byte-compared to
section 5's references. Each step keeps `tools/mali/corpus-verdicts.txt`
and `tests/fixtures/mali_glmark2_verdicts.txt` moving forward only. Before
the session, re-read the sites above at the then current tip: plan #367's
lower-* items edit the same files.

0. **Re-attribute the vertex int read (no lowering).** `nir.rs:2927`
   raises `UniformType` for what is an int value in the vertex stage. The
   load needs nothing: the word is already in `uniform_of`'s table. That
   site should become intval's refusal (`IntValue`) when intval's vertex
   lowering lands (`lower-intval-1`), not here, so that 148 cases move with
   their family (T8, T9, T11, T12). This item only records the decision.
1. **Vertex fp32 `mat2`/`mat3` (T1, T4, T5; 57 cases).** In `uniform_of`
   (`nir.rs:1012-1027`), take `GL_FLOAT_MAT2` (4 components) and
   `GL_FLOAT_MAT3` (9). At `nir.rs:2930-2936`, Mesa's load is one
   column: base = the column's first front-end dword, size = the rows.
   For mat3 the column stride is 4 front-end dwords (`reflect.rs:159-161`).
   In `UboLayout::vs` (`select.rs:351-385`), a mat3 is three rows of 3
   (front-end dwords `c * 4`, Mesa words `3c`), each split. A mat2 is one
   row of 4, split as mat4 is. The frame needs no change: it already
   copies by row.
2. **Vertex fp16 `mat2`/`mat3` (T2, T3; 106 cases).** `fp16_uniform`
   (`nir.rs:1042-1045`) also takes mat2 and mat3. The fp16 rows are one per
   column (mat2: 2 halves, 1 word; mat3: 3 halves, 2 words, with the pad
   half unused). `UboRow` with `halves` already gives `div_ceil(comps, 2)`
   words per row. The mat4 path (`vsfp16_19_hello`, words 6-13) is the
   model. Per lane the read is `F16_TO_F32` of its half, as `:2941-2951`
   does.
3. **Vertex mediump struct members (P1, P2; 35 cases).** At `mod.rs:1302-1309`,
   accept a struct member (a name with `.`, `uniform_precision` returning
   Medium or Low) as **fp32** words: Mesa does not lower a struct (section
   3.2). The conversion `f2fmp` (`F32_TO_F16`, or `V2F32_TO_V2F16` for a
   pair) goes where the vertex walk reads such a member into fp16
   arithmetic. Section 5's P1 references decide whether `res = val.a` keeps
   it. T7's `bvec2` member and step 4 share the bvec load.
4. **Vertex `bvec2/3/4` load (T6, T7; 61 cases).** In `uniform_of`, take
   `GL_BOOL_VEC2/3/4` as 2, 3 and 4 raw words, as `fs16` does. Their `==`
   and `float(bool)` uses stay refused until intval. The test pins only the
   refusal moving from `UniformType` to the intval one (no bytes yet).
5. **Fragment highp uniform into the mediump colour (F4, F5; 18 cases).**
   In `fs16` (`select.rs:462-464`), a float or vec uniform declared highp is
   an fp32 row (`halves: false`) instead of a refusal. Its lanes reach the
   fp16 colour through `V2F32_TO_V2F16` pairs (or `F32_TO_F16`), as section
   5's scissor reference shows. `gl_DepthRange` (P3, F5) also needs its
   precision lookup (`uniform_precision`, `mod.rs:1197-1218`): the
   built-in's members are highp. This is the smallest shape with a
   complete program (`gl_FragColor = u_color`), so it is the best first
   step if the session starts at the fragment side.
6. **Fragment fp16 `mat2/3/4` (F1, F6's mat3; up to 79 cases).** `fs16`
   takes `GL_FLOAT_MAT2/3/4` (halves) as one row per column (split) for
   mat3, and as one row for mat2 and mat4. The lane-pair reads
   (`nir_fs.rs:6896-6917`) then see a column's aligned pairs. For mat3, lane
   2 is a half alone (`.h00`): covered by the "one component in both
   lanes" arm (`:6905-6910`).
7. **Fragment `ivec2/3/4` load (F3, F6's ivec3; 37 cases).** `fs16` takes
   `GL_INT_VEC2/3/4` as raw words (`(n, false)`), as the bvecs. The `i2f`
   and `==` that follow are intval's. The test pins only the refusal moving.

Order for one session, by unblocked cases and the confidence of the
reference: 5, 1, 3, 2, then 6 if time remains. Steps 4 and 7 are small and
can go with intval's first step instead. Step 0 is a decision only. Steps
1, 2 and 6 share the per-column row change, so do them in one commit
series with one test each.

## 5. The reference cases for `ref-unitype`

Method: the **shim driver** (`tools/mali/drm_shim_ref.sh run`, Mesa 26.2.1
gallium panfrost on drm-shim, the GLSL path), with the trace reduced by
`tools/mali/pan_ref_diff.py extract --draw`, as in
`mali-uniform-types.md` section 6. `pan_shc` is not the byte reference
for these, because its SPIR-V front end has no `lower_precision`
(`tools/mali/README-pan_shc.md`, "Known gaps": `mediump`) and every shape
here except T1 and T5 is about mediump. Use pan_shc only for metadata (the
FAU word names), and only where its bytes are equal. Names are
`tools/mali/testdata/deqp_unitype_<name>.{vert,frag,expect.json}`, and the
sources are the `measure` item's pairs from `tools/mali/testdata/deqp_shapes/`,
verbatim.

| name | from case | records | step |
|---|---|---|---|
| `deqp_unitype_vsmat2_highp` | `shaders.matrix.add.uniform_highp_mat2_mat2_vertex` | fp32 mat2, 4 words, column loads | 1 |
| `deqp_unitype_vsmat3_highp` | `shaders.matrix.mul.uniform_highp_mat3_vec3_vertex` | fp32 mat3, 9 packed words, `FMA` chain order | 1 |
| `deqp_unitype_vsmat3_light` | `light_amount.1dir` (the vertex stage) | the highp mat3 beside mat4s; it stays refused behind the struct array, so its bytes are for later | 1 |
| `deqp_unitype_vsmat2_mediump` | `shaders.matrix.add.uniform_mediump_mat2_float_vertex` | fp16 mat2, 2 words | 2 |
| `deqp_unitype_vsmat3_mediump` | `shaders.matrix.mul.uniform_mediump_mat3_vec3_vertex` | fp16 mat3, 6 words, the pad half, push or `LD_PKA` | 2 |
| `deqp_unitype_vsmat3_lowp` | `shaders.matrix.sub.uniform_lowp_mat3_mat3_vertex` | lowp = mediump (a check) | 2 |
| `deqp_unitype_vsapi_mat2` | `uniform_api.value.initial.render.basic.mat2_vertex` | the 0.05 compares in VS fp16 | 2 |
| `deqp_unitype_vsstruct_partial` | `linkage.uniform_struct_partial` | `res = val.a`: fp32 member, the `f2fmp` fold | 3 |
| `deqp_unitype_vsstruct_add` | `linkage.uniform_struct` | `val.a + val.b` in fp16, `f2f32` at the varying | 3 |
| `deqp_unitype_vsstruct_rip` | `linkage.uniform_struct_use_case_rip` | highp and mediump members in one struct | 3 |
| `deqp_unitype_vsapi_struct` | `uniform_api.value.initial.render.basic_struct.float_vec4_vertex` | the struct member compares | 3 |
| `deqp_unitype_vsbvec3` | `uniform_api.value.initial.render.basic.bvec3_vertex` | the bvec words, and the `==` for intval | 4 |
| `deqp_unitype_fshighp_color` | `fragment_ops.scissor.contained_tri` (`gl_FragColor = u_color`) | a highp vec4 into the fp16 colour | 5 |
| `deqp_unitype_depthrange` | `shaders.builtin_variable.depth_range_vertex` and `_fragment` | `gl_DepthRange` state words in both stages | 5 |
| `deqp_unitype_fsmat2_mediump` | `shaders.matrix.add.uniform_mediump_mat2_mat2_fragment` | fp16 mat2 in the FS | 6 |
| `deqp_unitype_fsmat3_mediump` | `shaders.matrix.add.uniform_mediump_mat3_float_fragment` | fp16 mat3, the pad half | 6 |
| `deqp_unitype_fsmat4_mediump` | `shaders.matrix.mul.uniform_mediump_mat4_vec4_fragment` | fp16 mat4 times a vector | 6 |
| `deqp_unitype_fsivec4` | `negative_api.shader.uniform_matrixfv_invalid_count` (`uniformTestVertSource` / `uniformTestFragSource`) | ivec4 words, `vec4(fTest).xy` (`I32_TO_F32`) | 7 |
| `deqp_unitype_fsmulti` | `uniform_api.value.initial.render.multiple_basic.fragment` | float, ivec3, mat3 and bvec2 in one block: the packed order | 6, 7 |

Caveats for the references:

1. `tools/mali/gl_ref.py --uniform-values` sets float, int, ivec, bool and
   bvec uniforms and matrices (`uniform_components()`). It exits on a
   struct uniform. Leave the flag out for that one (the values do not
   change the instructions, `mali-uniform-types.md` section 6).
2. `deqp_unitype_vsbvec3` is drawn with true values (a `bvecN` is true
   where its component is even). It shows whether a raw `~0` reaches an
   instruction as a mask (section 3.3).
3. The matrix cases put `attribute highp vec4 a_position` and a
   `v_color`/`v_coords` varying around the operation. The `dynamic_*`
   variants add matrix attributes and varyings. Take the `uniform_*`
   variants for this family, so that the vertex attributes are vec4 only.

## 6. Hand-offs found here

- **intval**: T8, T9, T11 and T12 (148 cases, 94 of them NS) are int
  values in the vertex stage. Their refusal text is `UniformType` today
  (`nir.rs:2927`). The bvec `==` (T6) and the ivec `i2f`/`==` (F3) come
  after this family's loads.
- **uniarray**: T10, P4 and F7 (193 cases). Mesa lowers a mediump *float*
  array to fp16 (2 words per vec4 element, 1 per float element in the low
  half: `glsl_count_dword_slots` of float16 is
  `DIV_ROUND_UP(1, 2) = 1`). An array of structs stays fp32 (section 3.2).
  `uniform_precision` cannot see a member of an array of structs
  (`mod.rs:1203-1205`).
- **mixprec**: F2 (37). Highp matrix arithmetic in a shader whose colour is
  the mediump `gl_FragColor`: fp32 values narrowed at the output (section
  3.5).
- **measure**: T13 and F8 (53 random programs) need the generated sources
  to be split by type.

## References

Plan #377 item `ref-unitype`. One reference per row of section 5, made
with the shim driver (Mesa 26.2.1 gallium panfrost on drm-shim, the GLSL
path). `gl_DepthRange` is two references, one per case
(`depthrange_vs`, `depthrange_fs`), because each dEQP case reads it in one
stage only and a reference is one real program. So there are 20 triples
`tools/mali/testdata/deqp_unitype_<name>.{vert,frag,expect.json}`.

**Source.** The integration tip has no `tools/mali/testdata/deqp_shapes/`
pair yet (the `measure` item is still running). Every `.vert`/`.frag` is
therefore the dEQP-GLES2 text **rebuilt** from VK-GL-CTS `93bca01861b0`
(`tools/cts/deps.toml`): the generator's code transcribed for the case's
parameters, or the `.test` case parsed the way `gluShaderLibrary.cpp`
parses it (`removeExtraIndentation`, which keeps dEQP's own odd indents)
and specialized the way `glsShaderLibraryCase.cpp` specializes it. A file
path below is under `modules/gles2/functional/` unless it starts with
`modules/` or `data/`. The `uniform_api` cases are the `value.initial`
ones, so every compared value is zero (`generateZeroVarValue`) and the text
has no random part.

**Command.** From the repository root, with `D=tools/mali/testdata`,
`<work>` the `drm_shim_ref.sh build` directory (Mesa 26.2.1 tarball, sha256
as in `third_party/mesa-26.2.1/UPSTREAM`) and `<out>` a scratch directory:

```sh
tools/mali/drm_shim_ref.sh run <work> $D/deqp_unitype_<name>.vert $D/deqp_unitype_<name>.frag <out>/deqp_unitype_<name> [--uniform-values]
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_unitype_<name>.trace --draw --attributes > $D/deqp_unitype_<name>.expect.json
```

`--uniform-values` (column "values") is left out where `gl_ref.py` cannot
set a uniform: bool, bvec and struct uniforms (section 5, caveat 1). Three
runs of all 20 gave byte-equal expect.json files.

| name | case | source (rebuilt) | values | what it pins for `lower-unitype` | step |
|---|---|---|---|---|---|
| `vsmat2_highp` | `shaders.matrix.add.uniform_highp_mat2_mat2_vertex` | `es2fShaderMatrixTests.cpp:888-1121`, `:1255-1391` | yes | fp32 mat2: 4 FAU words per matrix, column-major, each lane read as its own `uN.wM` (`FADD.f32`) | 1 |
| `vsmat3_highp` | `shaders.matrix.mul.uniform_highp_mat3_vec3_vertex` | as above | yes | fp32 mat3: 9 packed words (no column padding), the `FMA.f32` chain of `mat3 * vec3` | 1 |
| `vsmat3_light` | `light_amount.1dir` | `es2fLightAmountTest.cpp:62-120`, `:156-157`, `:194` | no | highp mat3 `u_normalMatrix` beside two mat4s and the `Light` struct array, then `normalize` (`FRSQ.f32`). Mesa takes it; ours stays refused behind the struct array (uniarray), so its bytes are for later | 1 |
| `vsmat2_mediump` | `shaders.matrix.add.uniform_mediump_mat2_float_vertex` | as `vsmat2_highp` | yes | fp16 mat2: 2 words, one per column, read as `v2f16`; the sum `F16_TO_F32` per lane before the 32-bit varying store | 2 |
| `vsmat3_mediump` | `shaders.matrix.mul.uniform_mediump_mat3_vec3_vertex` | as above | yes | fp16 mat3: 6 words, 2 per column, the third half of each column alone with a zero pad half. All 6 are pushed FAU words; lane 2 is read as `uN.wM.h00`. No unaligned load and no `LD_PKA` | 2 |
| `vsmat3_lowp` | `shaders.matrix.sub.uniform_lowp_mat3_mat3_vertex` | as above | yes | lowp is laid out and computed as mediump (fp16 words, `FADD.v2f16` with `.neg`) | 2 |
| `vsapi_mat2` | `uniform_api.value.initial.render.basic.mat2_vertex` | `es2fUniformApiTests.cpp:345-351`, `:1135-1286`, `:538-567` | yes | the `compare_mat2` chain against zeros in VS fp16: Mesa folds it to an fp16 max of `abs` lanes, one `FCMP_OR.v2f16.lt` against the 0.05 word (`2a662a66`), `S16_TO_S32`, `CSEL.u32.eq` | 2 |
| `vsstruct_partial` | `linkage.uniform_struct_partial` | `data/gles2/shaders/linkage.test:741`, expanded by `modules/glshared/glsShaderLibraryCase.cpp:193-226`, `:441-511` and `framework/opengl/gluShaderLibrary.cpp:473-558`, `:1912-1991` | no | `res = val.a`: the member is one fp32 word and no conversion is left (`f2f32(f2fmp(x))` folds). The varying program is `FMA.f32 r0, u5.w1, 0x0, u5.w1` and the store | 3 |
| `vsstruct_add` | `linkage.uniform_struct` | `linkage.test:646`, expanded as above | no | Mesa drops the `dummy` varying (`out0 + dummy - dummy` folds), so the VS has no varying program. The FS reads both members as fp32 words and narrows them with one `V2F32_TO_V2F16` (a struct stays fp32 in the FS too) | 3 |
| `vsstruct_rip` | `linkage.uniform_struct_use_case_rip` | `linkage.test:1619`, expanded as above | no | highp and mediump members in one struct: the whole struct is fp32 words in declaration order; `res = val.constantAttenuation` is the same one `FMA.f32` as `vsstruct_partial` | 3 |
| `vsapi_struct` | `uniform_api.value.initial.render.basic_struct.float_vec4_vertex` | as `vsapi_mat2` | no | mediump struct members `float` and `vec4` as 5 fp32 words, narrowed with `V2F32_TO_V2F16` pairs (and `FADD.f32 rN.h0` for the odd lane) into the fp16 compares | 3 |
| `vsbvec3` | `uniform_api.value.initial.render.basic.bvec3_vertex` | as `vsapi_mat2` | no | bvec3: 3 raw 32-bit words. `compare_bvec3(u, bvec3(false))` is `LSHIFT_OR` of the three words and `CSEL.u32.eq` against 0 | 4 |
| `fshighp_color` | `fragment_ops.scissor.contained_tri` | `modules/glshared/glsScissorTests.cpp:120-158` (`#version 100`) | yes | a highp vec4 into the fp16 colour: 4 fp32 FAU words, two `V2F32_TO_V2F16` | 5 |
| `depthrange_vs` | `shaders.builtin_variable.depth_range_vertex` | `es2fShaderBuiltinVarTests.cpp:203-232`, `:714-715` | yes (none declared) | `gl_DepthRange.near/far/diff` as fp32 FAU words in the VS (`u5.w1`, `u6.w0`, `u6.w1`), `diff*0.5 + 0.5` as one `FMA.f32` with `0x38000000.h1` | 5 |
| `depthrange_fs` | `shaders.builtin_variable.depth_range_fragment` | as above | yes (none declared) | the same words in the FS: near and far narrowed by one `V2F32_TO_V2F16`, `diff` through `FMA.f32` and `FADD.f32 r0.h0` | 5 |
| `fsmat2_mediump` | `shaders.matrix.add.uniform_mediump_mat2_mat2_fragment` | as `vsmat2_highp` | yes | fp16 mat2 in the FS: 2 words per matrix, `FADD.v2f16` of whole words | 6 |
| `fsmat3_mediump` | `shaders.matrix.add.uniform_mediump_mat3_float_fragment` | as above | yes | fp16 mat3 in the FS: 6 words, lane 2 of each column read as `uN.wM.h00` (the pad half unread) | 6 |
| `fsmat4_mediump` | `shaders.matrix.mul.uniform_mediump_mat4_vec4_fragment` | as above | yes | fp16 mat4 times a vector: 8 words, the `FMA.v2f16` chain with `.h00`/`.h11` swizzles of the vector | 6 |
| `fsivec4` | `negative_api.shader.uniform_matrixfv_invalid_count` | `es2fNegativeShaderApiTests.cpp:46-58`, `:985-987` | yes | mediump ivec4: one 32-bit word per lane; only `.xy` is read, `S32_TO_F32` of `u2.w0` and `u2.w1`, then `V2F32_TO_V2F16`. The VS is the mediump `mat4 * vec4` fp16 path | 7 |
| `fsmulti` | `uniform_api.value.initial.render.multiple_basic.fragment` | as `vsapi_mat2` | no | `float`, `ivec3`, `mat3` and `bvec2` in one mediump FS: the float is an fp16 word, the ivec3 3 raw words, the mat3 6 fp16 words, the bvec2 2 raw words (`LSHIFT_OR` and `.not`) | 6, 7 |

The three questions section 5 left to the references:

1. **The pad half of a mediump `mat3`** (`vsmat3_mediump`, `fsmat3_mediump`):
   no unaligned load. Every column is two whole pushed words, the pad half
   is the zero high half of the second word, and lane 2 is read as
   `uN.wM.h00`. So `UboRow` with `halves` and `div_ceil(3, 2) = 2` words per
   column is the layout.
2. **`f2f32(f2fmp(x))` for `res = val.a`** (`vsstruct_partial`,
   `vsstruct_rip`): it folds. No `F32_TO_F16` or `F16_TO_F32` is left. The
   one `FMA.f32 x, 0, x` before the store is also in programs with no struct
   (`vsmat2_highp`, `vsmat3_lowp`, `vsbvec3`), so it belongs to the varying
   store, not to the member read.
3. **The scissor fragment shader's narrowing** (`fshighp_color`): two
   `V2F32_TO_V2F16` straight from the FAU words, no move.

`vsbvec3` and `fsmulti` are drawn with true values (section 5, caveat 2):
gl_ref.py sets a `bvecN` true where its component is even, and Mesa
uploads a true as the word `ffffffff`. The words are the same as with
every value false: `vsbvec3` and `fsmulti` only OR the raw bvec
words and test the result against 0 (`LSHIFT_OR`, then `CSEL.u32.eq` or
`.not`), and that is the same for a true of `1` and of `~0`. The bytes do
not depend on the values; with `--uniform-values` only the printed FAU
words change.

## Lowering map (at 008306cb)

Plan #377 item `map-unitype`. A map from the steps of section 4 onto the
compiler as it is at `008306cb`, for `lower-unitype`. It changes no code. The
`file:line` citations below are of that commit; sections 1 to 6 cite
`38ad30eb`, and the lines have moved since (`uniform_of` is now
`nir.rs:996-1031`, its `_ =>` arm at `:1028`; the vertex `UniformPrecision`
raise is `mod.rs:1307`; `fs16` is `select.rs:434-486`). Re-read the sites at
the tip of the session: plan #367's lower-* items edit the same files. The
tests are those of `tests/mali_deqp_unitype.rs`, run with
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_unitype`.
Paths without a directory are under `src/shaderc/online/mali/`.

### M0. The raise sites at the tip

| refusal | file:function (line) | condition |
|---|---|---|
| `UniformType`, the type, vertex | `nir.rs:uniform_of` (`:1028`) | `gl_type` outside float, vec2/3/4, mat4, int, uint, bool, ivec2/3/4, uvec2/3/4. `GL_FLOAT_MAT2`, `GL_FLOAT_MAT3`, `GL_BOOL_VEC2/3/4` and the non-square matrices fall through. Reached from `nir.rs:build`, arm `IrOp::LoadUniform` (`:3047`), and from `select.rs:UboLayout::vs` through `nir_fs.rs:uniform_extent` (`:5872-5895`, which ends in `nir::uniform_of` at `:5894`). |
| `UniformType`, an int read | `nir.rs:build`, `IrOp::LoadUniform` (`:3050-3052`) | `gl_type == GL_INT` and the value is not only the index of a `LoadUniformIdx` (`nir.rs:index_only`, `:1053`). This is an int value (intval's), not a type refusal. |
| `UniformType`, highp fragment | `nir_fs.rs:uniform_extent` (`:5894`) and `nir_fs.rs:uniform_load` (`:5903`), through `select.rs:UboLayout::fs32` (`:393`) | a highp FS reads a type `uniform_of` lacks (the F2 shape; mixprec's after the type). |
| `UniformPrecision` | `mod.rs:compile_stage`, vertex arm (`:1302-1308`, raise `:1307`) | a uniform the program reads (`ubo.word(..)` is `Some`) whose `uniform_precision` (`mod.rs:1197-1220`) is not `High` and that has no half (`ubo.half(..)` is `None`). A half exists only for a plain float, vec2/3/4 or mat4, no array (`nir.rs:fp16_uniform`, `:1043`). A mediump struct member, and a uniform with no declaration for `uniform_precision` (`gl_DepthRange.near`: its base `gl_DepthRange` is no entry of `decls`, `:1203-1205`, so the lookup is `None`), fall here. |
| `FsUniform`, the type | `select.rs:UboLayout::fs16` (`:460`, the `_ =>` arm) | any type but int/bool (no array), bvec2/3/4 (no array), float, vec2/3 (no array), vec4. `GL_INT_VEC2/3/4` and every `GL_FLOAT_MAT*` fall through. `fs16` is called from `mod.rs:compile_stage` (`:1459`, `:1469`, `:1475`) and from `vec16.rs:program` (`:448`). |
| `FsUniform`, highp | `select.rs:UboLayout::fs16` (`:462-464`) | `halves && highp(u.name)`. `highp` is `mod.rs:1395`: `prec(name)` is `High`, and `prec` is a plain name lookup (`mod.rs:1239`), so a struct member or `gl_DepthRange.*` is never highp there. |
| `FsUniform`, the read | `nir_fs.rs` `:6908`, `:6918`, `:6922`, `:7153`, `:7175`, `:7432`, `:7589`, `:8047`, `:8447`, `:8450`, `:8495`; `vec16.rs:1086-1087` | a half of the fp16 block read other than as its own aligned pair or one lane (`ubo.half(d)` is `None`, or the swizzle has no form). |
| built-in | `lower.rs:3393-3396` ("this use of gl_DepthRange is not lowered yet") | only the struct as a whole. The members already are rows (`gl_DepthRange.near/far/diff`, float, after the application's uniforms: `lower.rs:14841-14870`), so the front end needs no change. |

Pins in `src/shaderc/online/mali/tests.rs` that a step moves, and are edited
in the same commit (a test that pins a refusal moves with it): `:1178` (mat2,
`UniformType`), `:1182` and `:1185` (`UniformPrecision`), `:1194-1196` and
`:7955` (`FsUniform`), `:4016` (`UniformType`), `:1087` (comment on the bool
uniform).

### M1. What every step shares: the row model and the frame

* `select.rs:UboRow` (`:241-266`) is `comps` consecutive front-end dwords at
  Mesa's word `word`, fp32 or fp16 `halves`. `UboLayout::push` (`:332`)
  appends a row; `row` (`:487`) finds the row of a dword by `dword ..
  dword + comps`; `half` (`:499`) gives `(word, half, first word)`; `fau`
  (`:559-570`) turns a word into `FauWord::Uniform { dword }` or
  `UniformHalves { lo, hi }`; `load_of` (`:532`) cuts the loads; `split_last`
  (`:345`) marks a row's words as loaded one by one.
* **A matrix cannot be one row when its columns do not touch.** The front
  end strides a `mat3` column to 4 dwords and leaves one unused
  (`reflect.rs:type_layout`, `:136-175`: `mat3` is 12 dwords, `mat2` is 4
  with stride 2, `mat4` is 16). So a `mat3` is **three rows**, `dword = off
  + 4c`, `comps = 3`, in consecutive words; a `mat2` and a `mat4` are one
  row (no hole). Today `vs`, `fs32` and `fs16` push one row per uniform
  (`select.rs:377`, `:409`, `:469`) and call `split_last` for an array or
  matrix (`:377-379`, `:409-411`, `:469-471`). The per-column loop is new in
  all three and is the change shared by M2, M4 and M7. A helper
  `matrix_cols(gl_type) -> Option<(cols, rows, stride)>` in `select.rs`,
  used by those three and by `nir.rs`, keeps one table.
* **The frame needs no change for any step.** It fills the block from the
  same rows (`src/mali/frame.rs:UboRow`, `:552-557`; `words`, `:559-566`;
  the fill at `:3008-3022`: `uniforms[r.dword + k]` for a 32-bit row, and
  for `halves` `f32_to_f16(uniforms[r.dword + c])` for `c < r.comps`, zero
  past it). The push words come from `UniformHalves { lo, hi }` and
  `Uniform { dword }` (`:3059-3063`; bounds in `check_draw`, `:2601-2602`).
  `src/mali/program.rs:77-80` and `:108-111` copy rows and words through.
  What a step needs is that the compiler's `dword` and `comps` name the
  front-end dwords the GL core fills. The pad half of a `mat3` column is
  `hi: None` (`select.rs:563`: `k + 1 < r.comps` is false), and
  `frame.rs:3014` writes zero for it. The fp16 conversion is the frame's
  `f32_to_f16` (`frame.rs:586`, round to nearest even, as `_mesa_float_to_half`,
  section 3.2): a mediump matrix needs nothing new there. A bool or an int
  is stored raw. GL's `~0`-for-true (section 3.3) is the GL core's
  conversion at `glUniform`, not the frame's; the `vsbvec3` reference does
  not depend on it (References).
* `gl_DepthRange` values come from `src/mali/backend.rs:1816-1818`
  (`near`, `far`, `diff`); nothing to add.

### M2. Step 1: vertex fp32 `mat2`/`mat3` (T1, T4, T5)

(a) Raise: `nir.rs:uniform_of` `:1028`, through `nir.rs:build` `:3047` and
`select.rs:UboLayout::vs` (`uniform_extent`, `nir_fs.rs:5894`).

(b) Change:

* `nir.rs:uniform_of`: add `GL_FLOAT_MAT2 => 4` and `GL_FLOAT_MAT3 => 12`
  (front-end dwords, as `u.dwords`; Mesa's block holds 9 words for the mat3).
* `nir.rs:build`, arm `IrOp::LoadUniform` (`:3047-3077`): the load is one
  column. Today `(base, size)` is `(u.dword_off + (dword_off - u.dword_off)
  / 4 * 4, 4)` when `total == 16`. Make it per type: mat3 base `off +
  4*col`, `size = 3`; mat2 base `off + 2*col`, `size = 2`, with `col =
  (dword_off - u.dword_off) / stride`. The guard `first + comps > size`
  (`:3057`) then holds per column. `vsmat2_highp` and `vsmat3_highp` read
  each lane as its own `uN.wM` (`FADD.f32`, and the `FMA.f32` chain of
  `mat3 * vec3`): that is the `else` arm `:3069-3074`, no new instruction.
* `select.rs:UboLayout::vs` (`:351-385`): rows per column as M1 (mat3: three
  rows of 3 at front-end `off + 4c`, Mesa words `3c..3c+2`, nine words;
  mat2: one row of 4), each `split_last` (Mesa loads a column; the offset is
  not the whole uniform's, `load_of`). The `halves` test (`:368-371`) is
  false here.
* `fs32` (`:393`) takes the same rows through the helper, but the highp
  matrix arithmetic of F2 is mixprec's.

(c) Frame: none (M1).

(d) Un-ignores, in order: `vsmat2_highp`, `vsmat3_highp`. In the same commit
the pin of `vsmat3_light` in `REFUSED` (`tests/mali_deqp_unitype.rs:27-48`)
moves from `OTHER_TYPE` to the uniform-array text (`mod.rs:417`, "a uniform
array"), because its mat3 passes and the `Light` struct array stands first;
it stays `#[ignore]`d for `lower-uniarray`.

(e) glmark2: none moves. The verdicts file's only uniform-type text is row 20
(`tests/fixtures/mali_glmark2_verdicts.txt:52`, `ideas:speed=duration vs
51b2`), an int read (M9), not a matrix. Dependencies: none for the mat2/mat3
loads. `div` (componentwise `fdiv`), `matrixCompMult` and matrix attributes
are other families' arithmetic. `vsmat3_light` needs lower-uniarray (the
`Light` array) and the loops.

(f) Risks: (1) `u.dwords` of a `mat3` is 12 by `type_layout` (3 columns of
stride 4); check it in `reflect` before writing the row ranges. (2) Mesa's
block has 9 words with no hole; `UboLayout::push` advances by `row.words()`
(`:339`), so consecutive rows are consecutive words. (3) A matrix read in one
column still takes all its words in Mesa (a used uniform is laid whole), so
`vs` must loop all columns, not the read ones. (4) An array of matrices is
refused first by `uniform_of` (`:1008`), unchanged.

### M3. Step 5 (the smallest, do it first): highp uniform in a mediump fragment shader, and `gl_DepthRange` (F4, F5, P3)

(a) Raise: `select.rs:UboLayout::fs16` `:462-464` (fragment); `mod.rs:compile_stage`
`:1307` (vertex, `gl_DepthRange.*` has no precision); `vec16.rs:program`
`:448` (a one-uniform colour goes through `vec16` first, `mod.rs:1396-1398`).

(b) Change:

* `mod.rs:uniform_precision` (`:1197-1220`): a member of `gl_DepthRange` is
  highp (GLSL ES 1.00 section 7.5). Return `Some(Precision::High)` when
  the base is `gl_DepthRange`. This one line makes `depthrange_vs` pass `:1307`.
  The FS `prec` closure (`:1239`) is a plain `find`: make it `uniform_precision`
  too, as the vertex arm does at `:1290`, or `highp(name)` is `false` for a
  struct member and `gl_DepthRange.*` and `fs16` lays them out as halves.
* `select.rs:UboLayout::fs16` (`:462-464`): when `halves && highp(name)`,
  push `(comps, false)` instead of returning `FsUniform`. Mesa keeps an
  explicitly highp uniform fp32 (section 3.2); the four words come out as
  `FauWord::Uniform` (`select.rs:567`). An array stays refused.
* `nir_fs.rs`, the mediump `LoadUniform` arm (`:4867-4892`): it reads every
  lane with `uniform_lane(.., 16, ..)` unless the type is int/bool (`int`,
  `:4872-4875`). A highp uniform in a mediump shader must read each lane as
  **32 bits** (`uniform_lane(d, 32, at)`, `:1334`) and narrow it with
  `Alu::F2fmp` (the narrow that `V2F32_TO_V2F16` is). The colour's pair read
  (`vectorise`, `:6836`) then sees a narrowed pair. `Leaf::Uniform`
  (`:6908-6918`) is for fp16 halves and must keep refusing an fp32 word.
  `fshighp_color` is two `V2F32_TO_V2F16` straight from four FAU words
  (References, question 3). `depthrange_fs`: near and far in one
  `V2F32_TO_V2F16`, `diff` through `FMA.f32` and `FADD.f32 r0.h0`.
* `vec16.rs:program` (`:424-448`) collects every `At::Uni` as a 16-bit
  one-component node for `fs16`. A highp uniform must not become one: the
  program returns `None` for it and `nir_fs::walk` decides (as for a mixed
  shader, `mod.rs:1396`).

(c) Frame: none (M1; `Uniform { dword }`, `frame.rs:3059`). Check only that
`backend.rs:1816-1818` fills the three rows the front end appends after the
application's uniforms.

(d) Un-ignores, in order: `depthrange_vs` (the one line, `:1307`),
`fshighp_color`, `depthrange_fs`.

(e) glmark2: none expected (no `ok` glmark2 shader declares a highp uniform
in a mediump fragment shader). If the corpus test shows a row moving, move it
forward in `tests/fixtures/mali_glmark2_verdicts.txt` in the same commit. No
dependency on intval or uniarray. mixprec owns F2 and shares `fs16`'s highp
arm and `mod.rs:1437-1449`: land one of the two first and rebase.

(f) Risks: (1) The change is for `!fs.highp` shaders only; it must not set
`f.mixed` (`mod.rs:1442-1447`, `:1479`) for a value narrowed at once. (2)
Check in the reference that `fs.highp` is false for the scissor shader (the
colour is mediump), else the path is `fs32` + `select_highp` and the work is
elsewhere. (3) `fs16` is also `vec16::program`'s layout: both call sites must
agree on a row's `halves`.

### M4. Step 2: vertex fp16 `mat2`/`mat3` (T2, T3)

(a) Raise: `nir.rs:uniform_of` `:1028` (type), then `mod.rs:1307`
(`UniformPrecision`: `fp16_uniform` takes no mat2 or mat3, so no half).

(b) Change:

* `nir.rs:fp16_uniform` (`:1043-1046`): add `GL_FLOAT_MAT2 | GL_FLOAT_MAT3`
  to the `float` match, and its comment (`:1030-1042`).
* `nir.rs:build`, fp16 arm (`:3059-3068`): `load16(Kind::Uniform { dword:
  base, comps: size })` with the column's `base` and `size` of M2, each lane
  `F16_TO_F32` of its half (`vsfp16_19_hello`'s form, unchanged).
* `select.rs:UboLayout::vs` (`:368-379`): the `halves` test finds a 16-bit
  `Kind::Uniform` node inside `dword_off .. dword_off + dwords`: true for a
  column. Rows as M1: mat3 three rows `comps = 3, halves = true`
  (`words() = div_ceil(3, 2) = 2`, six words, as `vsmat3_mediump`); mat2
  one row `comps = 4, halves = true` (two words, one a column). The pad half
  is `UniformHalves { hi: None }`, so lane 2 is `uN.wM.h00`. The mat4's split
  rows (`vsfp16_19_hello`, words 6-13) are the model.
* `vsapi_mat2` adds the 0.05 compare in fp16 (`FCMP_OR.v2f16.lt` on a max of
  `abs` lanes, then `S16_TO_S32` and `CSEL`). If it stops at another refusal,
  pin that one and leave the test ignored; the form is not this family's.

(c) Frame: none (M1): `frame.rs:3012-3016` writes `h(2k) | h(2k+1) << 16`
with zero past `comps`, the mat3's pad half.

(d) Un-ignores, in order: `vsmat2_mediump`, `vsmat3_mediump`, `vsmat3_lowp`,
`vsapi_mat2` (the last only if its compare passes).

(e) glmark2: none moves (the rows are new and no `ok` row reads a mat2 or
mat3). No dependency on intval or uniarray.

(f) Risks: (1) lowp is laid out as mediump (`vsmat3_lowp` is the check):
`fp16` (`mod.rs:1291-1293`) takes both already. (2) A one-lane read of a
column must not become a 16-bit load at byte offset 2 mod 4 (not pushable,
section 3.5); the reference pushes all six words whole, so keep the
one-word loads. (3) A mat3 times vec3 in fp16 may use `FMA.v2f16`
pairs: an arithmetic form outside this family.

### M5. Step 3: vertex mediump struct members (P1, P2)

(a) Raise: `mod.rs:compile_stage` `:1307`. The member `val.a` is mediump
(`uniform_precision` reads it from the struct's spelling, `:1205-1219`) and
has no half, because `fp16` (`mod.rs:1291-1293`) names a plain uniform only
and a struct is never lowered (section 3.2).

(b) Change:

* `mod.rs:1302-1308`: accept a struct member (`name` contains `.`) of
  precision Medium or Low as an **fp32** row. The test becomes "highp, or a
  struct member": Mesa keeps the struct fp32 and reads each mediump member
  through `f2fmp`.
* `nir.rs:build`, `LoadUniform` (`:3047`): the member is read fp32 (arm
  `:3069`); where fp16 arithmetic uses it the walk inserts `Alu::F2fmp`, as it
  does for varyings. `vsstruct_partial` and `vsstruct_rip` fold
  `f2f32(f2fmp(x))` to `x` (References, question 2): no conversion for
  `res = val.a`; the one `FMA.f32 x, 0, x` before the store belongs to the
  varying store, not to the member. `vsapi_struct` narrows with
  `V2F32_TO_V2F16` pairs and `FADD.f32 rN.h0` for the odd lane.
* **The fragment side** (`vsstruct_add` and `vsapi_struct` are compiled as
  pairs; the FS reads both members as fp32 words and narrows them). `fs16`
  pushes a member by its `gl_type`, `halves = true` for a float, because
  `prec` (`mod.rs:1239`) is a name lookup that returns `None` for `val.a`
  and `highp(name)` is false. That is a **silent wrong layout**, not a
  refusal, if a struct uniform reaches `fs16`: find out first whether a
  struct uniform in an FS is refused earlier, and write the test that pins
  it. Then make `fs16` push a member as `(comps, false)` and the mediump
  `LoadUniform` arm (`nir_fs.rs:4867`) read it at 32 bits with `F2fmp`:
  the same mechanism as M3, so do M3's `nir_fs.rs` change first.
* `UboLayout::vs` (`:351`): a member is one row of its own (`m.uniforms`
  holds `val.a` and `val.b` as entries with their own `dword_off`), fp32, in
  declaration order: nothing new, since the node's `bits` is 32.

(c) Frame: none; the row is fp32 (`frame.rs:3017-3018`).

(d) Un-ignores, in order: `vsstruct_partial`, `vsstruct_rip`,
`vsstruct_add`, `vsapi_struct`. (`depthrange_vs` is taken by M3.)
Status: with the struct members as fp32 rows (205547fa) and lower-intval-1's
`bool_colour` (the `isOk` result in the colour) together, `vsstruct_partial`
and `vsstruct_rip` are byte-equal and un-ignored; `vsstruct_add` compiles
and is pinned as `differs` for risk (2) below (the reference VS has no
varying program, ours has one).

(e) glmark2: the one struct uniform of the corpus, `texfill.vert`'s
`u_fill` (`mod.rs:1286-1289`), is highp-qualified and takes the `High` arm
first: the new arm must not change it. No verdict is expected to move. No
dependency on intval. Arrays of structs (T10, P4) are lower-uniarray's, with
`uniform_precision`'s `None` for a nested member (`:1203-1205`).

(f) Risks: (1) The struct is fp32 in both stages while a plain mediump
uniform beside it (`ref_out0` in `vsstruct_add.frag`) is fp16: one block mixes
fp32 and fp16 rows. `halves` is a row attribute (`select.rs:249-253`), so
the layout supports it, but test the push and `load_of` on that mix. (2) The
reference's VS has no varying program (`out0 + dummy - dummy` folds); ours
has one unless the fold matches, which is constop's or colourpair's rule: a
mismatch there is pinned, not fixed here. (3) No check at `:1302` guards the
FS path (see the fragment side above).

### M6. Step 4: vertex `bvec2/3/4` load (T6, T7)

(a) Raise: `nir.rs:uniform_of` `:1028`.

(b) Change: add `GL_BOOL_VEC2 => 2`, `GL_BOOL_VEC3 => 3`, `GL_BOOL_VEC4 => 4`
next to the `GL_BOOL` arm (`:1019-1027`). The int filter at `:3050` is
`GL_INT` only; leave bools alone there (a bool is read through the same
`IrOp::LoadUniform` and is intval's after the load). `UboLayout::vs` takes
`comps` from `uniform_extent` and pushes `halves = false`. `reflect.rs:type_layout`
has `BVec` already (`:151-153`, `bvec3` aligned to 4), so the row is `dword =
off, comps = 3`.

(c) Frame: none. The word is `uniforms[dword]` raw. The references only OR the
words and test against 0 (References), so a true of 1 or `~0` gives the same
bytes.

(d) Un-ignores: **`vsbvec3` stays ignored** while the `==` of bvecs
(`ball_iequal`: `LSHIFT_OR` + `CSEL.u32.eq`) is refused by intval's text
(`IntValue` or `CompareForm`): move its pin in `REFUSED` from `OTHER_TYPE`
to that text in the same commit. If `lower-intval-1` has landed and the
compare passes, `vsbvec3` is un-ignored here.

(e) glmark2: none. Dependency: **lower-intval** for the bvec `==` and
`float(bool)` (`b2f32`).

(f) Risks: `index_only` (`:1053`) and the `GL_INT` filter do not cover
`GL_BOOL`; a bool used as an index is not a dEQP shape here.

### M7. Step 6: fragment fp16 `mat2/3/4` (F1, F6's mat3)

(a) Raise: `select.rs:UboLayout::fs16` `:460`, and `vec16.rs:448` (the same
call).

(b) Change:

* `select.rs:UboLayout::fs16` (`:446-471`): add `GL_FLOAT_MAT2`,
  `GL_FLOAT_MAT3`, `GL_FLOAT_MAT4` as `halves = true`. A mat2 is one row
  `(4, true)` (two words), a mat4 one row `(16, true)` (eight words), a mat3
  three rows `(3, true)` at `off + 4c` (two words each, six). Each row
  `split_last` (the loop at `:466-470` already does it for `is_matrix`; the
  comment at `:463-465` that a matrix does not reach here goes).
* `nir_fs.rs`, mediump `LoadUniform` arm (`:4867-4892`): lane `c` of
  front-end dword `dword_off + c` is `uniform_lane(.., 16, at)` (`:4881`);
  `dword_off` already carries the column stride, so the lane read is
  unchanged. `f.nir.uniform_load[n] = u.dword_off` (`:4882-4884`) records the
  load's origin: for a matrix set it to the **column's** first dword, or
  `load_of` cuts the wrong words.
* Pair reads (`vectorise`, `:6896-6922`): lanes `(2k, 2k+1)` of a column are
  one word (`Swz::H01`); lane 2 of a mat3 column is a half alone (`H00`, the
  `d == e` arm, `:6909-6914`): no code. Test that the pad half is never
  asked for. The other `Leaf::Uniform` readers (`:7153`, `:7175`, `:7432`,
  `:7589`, `:8047`, `:8447`) all go through `ubo.half(d)` and follow the
  rows. `fsmat4_mediump` is `FMA.v2f16` with `.h00`/`.h11` swizzles of the
  vector.

(c) Frame: none (M1; `frame.rs:3012-3016`).

(d) Un-ignores, in order: `fsmat2_mediump`, `fsmat3_mediump`,
`fsmat4_mediump`. `fsmulti` (float, ivec3, mat3, bvec2 in one block) needs M8
too.

(e) glmark2: none expected (no `ok` glmark2 fragment shader reads a matrix
uniform). No dependency on intval. The `dynamic_*` matrix cases need matrix
varyings (colour forms) and are outside this step.

(f) Risks: (1) The VS of each pair is compiled first: the `fs*` references
use a plain `mat4 * vec4` VS (`fsivec4` records the mediump fp16 path), so
they reach the FS today; confirm with `refusals_pinned`. (2) A mat3 adds
three rows, bounded by `MAX_LAYOUT_ROWS` (`select.rs:296`, 256) and, for a
block the program loads, `MAX_UBO_ROWS` (`:301`, 32). (3) A matrix in an
`fs.highp` shader goes to `fs32` and `uniform_of` (M2), not here.

### M8. Step 7: fragment `ivec2/3/4` load (F3, F6's ivec3)

(a) Raise: `select.rs:UboLayout::fs16` `:460`.

(b) Change: add `GL_INT_VEC2/3/4` (and the `GL_UNSIGNED_INT_VEC*` types if a
case needs them) with `array_len <= 1` as `(n, false)` next to the bvec arms
(`:451-455`). In `nir_fs.rs`'s mediump `LoadUniform` arm add them to the
`int` list (`:4872-4875`) so each lane is read as a raw 32-bit word. The use
(`vec4(fTest).xy` is `S32_TO_F32` of `u2.w0` and `u2.w1`, then
`V2F32_TO_V2F16`) is intval's `i2f`, as is the `==`.

(c) Frame: none.

(d) Un-ignores: `fsivec4` and `fsmulti` only once intval's `i2f` and the
ivec/bvec `==` pass; otherwise both stay `#[ignore]`d and `REFUSED` moves
their pin from `FS_MEDIUMP` to the new text. `fsmulti` also needs M7's mat3.

(e) glmark2: none. Dependency: **lower-intval-1**.

(f) Risks: `mod.rs:is_float_uniform` (`:1224-1226`) lists the int vector types
`0x8B53..=0x8B58` and `0x8DC6..=0x8DC8`; `GL_BOOL_VEC4` (`0x8B59`) is not in
it, so a `bvec4` uniform counts as float for the `mixed` test
(`mod.rs:1442-1447`). Add it when bvec4 is taken.

### M9. Step 0 (a decision) and the int read

`nir.rs:3050-3052` raises `UniformType` for an int uniform read as a value.
It should be intval's refusal (`IntValue`), so that the 148 cases of T8, T9,
T11 and T12 move with their family, not with this one. The only glmark2
verdict of the family that this can touch is row 20
(`tests/fixtures/mali_glmark2_verdicts.txt:52`, `ideas:speed=duration vs
51b2`): its text is "a uniform other than a float, ...". Whoever changes
`nir.rs:3050` moves that row in the same commit. (The scene's
`tools/hostharness/glmark2/FINDINGS-mali-allscenes.md:365` records the older
"an integer or boolean value" text for it; that file is a record and is not
edited.)

### M10. Order, tests per commit, and dependencies

1. M3: the `uniform_precision` line (`depthrange_vs`), then `fs16` and
   `nir_fs.rs` (`fshighp_color`, `depthrange_fs`): the smallest complete
   programs.
2. M2: `vsmat2_highp`, `vsmat3_highp` (the `vsmat3_light` pin moved), with the
   per-column rows helper.
3. M4: `vsmat2_mediump`, `vsmat3_mediump`, `vsmat3_lowp`, `vsapi_mat2`.
4. M5: `vsstruct_partial`, `vsstruct_rip`, `vsstruct_add`, `vsapi_struct`,
   after M3's fp32-in-mediump read.
5. M7: `fsmat2_mediump`, `fsmat3_mediump`, `fsmat4_mediump`.
6. M6 and M8: `vsbvec3`, `fsivec4`, `fsmulti`: their pins move, or they
   un-ignore, with `lower-intval-1`.

Each commit un-ignores its tests, moves the other pins (`REFUSED` in
`tests/mali_deqp_unitype.rs:27-48`, and the unit tests of M0), regenerates
`tools/mali/corpus-verdicts.txt` and `tests/fixtures/mali_glmark2_verdicts.txt`
only if a row moves (forward only), and runs `cargo test ... --test
mali_deqp_unitype` plus the unit tests of `src/shaderc/online/mali/tests.rs`
it touches.

Dependencies: `lower-intval-1` for `vsbvec3`, `fsivec4`, `fsmulti` and the
int read (M9); `lower-uniarray` for `vsmat3_light`, every array shape (T10,
P4, F7) and the nested member of `uniform_precision`; `lower-mixprec` for F2
(a highp matrix in the fp16 colour), which shares `fs16`'s highp arm and
`mod.rs:1437-1449`. No frame change is needed by any step (M1);
`frame-uniarray` (uniform arrays at fp16 strides) is a separate item.

## Lowering result (lower-unitype)

Plan #377 item `lower-unitype`. 11 of the 20 references are byte-equal to
Mesa 26.2.1 (`tests/mali_deqp_unitype.rs`, `MATCHED`; 13 tests pass, 9 are
ignored with their pin). Since the references are drawn with
`--uniform-values` it is 10 of 20 (12 pass, 10 ignored): `vsbvec3` has
Mesa's program words but not its FAU order, below. The doc-m spike's patch (205547fa) was already on
the tip when the item started: it took M3 (`gl_DepthRange` highp, a highp or
struct-member uniform read at 32 bits and narrowed in a mediump fragment
shader, `FsPrecision::fp32_uniforms`), M2 (fp32 `mat2`/`mat3` rows per
column, `UboLayout::push_uniform`) and M5's fp32 struct members
(`vsmat2_highp`, `vsmat3_highp`, `fshighp_color`, `depthrange_vs`,
`vsstruct_partial`, `vsstruct_rip`; `fsmat4_mediump` came with constop).
This item adds four, all in `src/shaderc/online/mali/`:

- `fsmat2_mediump` (M7): `vec16.rs` takes a program with no `if` that reads
  a mat2 or mat3 uniform (`flat`): Mesa's column operations are vectors, a
  mat2 sum two `FADD.v2f16 r0, u2.w0, u2.w1` of whole columns, the colour's
  lanes `MKVEC`s of their halves, where the lane walk pairs the colour's
  lanes across columns (the `FsUniform` refusal). The arithmetic is allowed
  at the top of `main` only there; compares stay the `if`'s. A mat4 stays the
  lane walk's (`fsmat4_mediump`, gm2_fsarith_1).
- `vsbvec3` (M6): `nir.rs` `vs_reduce` takes `all(b == bvec3(false))` of
  booleans (the bvec uniform's words, `i2b` leaves them as they are) as
  `inot(ior(ior(b.y, b.x), b.z))`: `LSHIFT_OR.i32` twice, then the
  `bcsel(inot(a), ..)` arm's `CSEL.u32.eq r0, r0^, 0x0, 0x3F800000, 0x0`.
  The y-before-x order of the first OR is measured; other widths have no
  reference and stay refused. The program words are
  Mesa's. The FAU is not: the reference, drawn with `--uniform-values`
  (the bvec3 true, false, true), holds the three uniform words as 0,
  `ffffffff`, `ffffffff`, that is y first, the order Mesa's program reads
  them; ours holds x, y, z, so the ORs take the words in another order
  (the same result for this shape). The reference drawn with the
  uniform unset could not show it. The test is ignored as `differs`
  (`DIFFERS` of `tests/mali_deqp_unitype.rs`) until the FAU follows
  Mesa's order.
- `vsmat2_mediump`, `vsmat3_mediump` (M4): the vertex walk computes a
  mediump `fadd` / `fmul` / `ffma` / `fneg` over fp16 uniforms at 16 bits,
  as lower_precision does with `fp16_const_buffers` (`vs_uniform16`,
  `vs_alu16u`): an fp16 uniform's lane is read at 16 bits under the
  `f2f32` its load made (left to `dce`); a mediump attribute's lanes are
  narrowed where the expression reads them, the whole vector at once in
  aligned pairs and an odd lane (`vs_narrow_input`: `V2F32_TO_V2F16 r0, r0^,
  r1^`, then `FADD.f32 r1.h0, r2^, 0x0.neg`); the vector is cut into its
  aligned pairs and an odd lane alone (`FMA.v2f16 r3, u7.w0.h00, r0.h11,
  0x0.neg`); a pair's operand is the uniform's first word whole or one of
  its first two lanes twice (`Val::Pair(n, H01 / H00 / H11)`, which
  `select.rs` `vs_variant` reads as the node's first word through the
  swizzle; `shrink` counts a `Pair`'s lanes), a 16-bit lane twice
  (`r0.h00`), or the `vec2` of the lanes. The front end's matrix product is
  already Mesa's `fmul` of column 1 then the `ffma`s of columns 0 and 2.
  An operation whose lanes are 16-bit alone at an odd width now takes this
  path too (`vs_pairs16` takes even widths). `vs_variant` gives an `f2f16`
  of two lanes both its 32-bit sources.
  An fp16 uniform's later word read as a pair (a vec4's or a mat4 column's
  z and w) has no form here and keeps the fp32 walk: deqp_misc_spill_*'s
  `u_in0 * u_in1` of two mat4 stays at its 32-register refusal.

What moved with it: no `tools/shaders` verdict (`tools/mali/corpus-verdicts.txt`
unchanged, `the_corpus_verdicts_are_pinned` passes); no other family's pin
(`tests/mali_deqp_*.rs` all pass, none of their ignored references newly
equal); `tools/mali/testdata/cases.txt` records no unitype shape (its rows
are pan_shc's control cases). `tests/fixtures/mali_glmark2_verdicts.txt`
moves forward on three lines that were already stale at the item's base
(the same result with this change stashed): scene 20's `vs bf4b` and `vs
c265` become `ok` and `vs 51b2` moves from the integer refusal to control
flow, lower-intval-2's vertex `imul`/`iadd` (that note's "What moved with
it" expected them); the corpus was generated with
`tools/hostharness/glmark2/run.sh` (PowerVR, 69 shaders).

Left, by owner:

| reference | where it stands | owner |
|---|---|---|
| `vsmat3_lowp` | its fp16 sums are Mesa's words; Mesa also stores `v_color.w` (1.0) in the flat `.v4`, the sums' `f2f32` hiding it from the link, where the walk propagates it. Keeping it (the `kept` rule for a vector of 16-bit lanes) meets the link's "a varying lane computed from no attribute" refusal | lower-varylink |
| `vsapi_mat2`, `vsapi_struct` | Mesa folds the products of the 0.05 compares' `b2f` into `b2f(iand(..))` and the `iand` of `flt(abs(x), 0.05)` into one `FCMP_OR.v2f16.lt.m1` of the lanes' `FMAX.v2f16` (`.abs` sources), `S16_TO_S32` into the `CSEL.u32.eq`; the walk compares each lane at fp32 | lower-intval step 3 / lower-mixprec (the mediump compare in the vertex stage) |
| `fsmat3_mediump` | refused (`FsOperandForm`); Mesa's program is `vec16.rs`'s column chunks plus the `v_coords` varying narrowed once, which that route does not read (it has no varying layout) | a `vec16.rs` varying, or the lane walk's operand form |
| `fsivec4`, `fsmulti` | refused (`FragColorLane`): `vec4(ivec4).xy` beside a texel; fsmulti's `ivec3`/`bvec2`/`mat3` compares into one colour | lower-intval |
| `depthrange_fs` | differs: Mesa's `diff * 0.5 + 0.5` is an `FMA.f32` narrowed after | lower-mixprec |
| `vsstruct_add` | differs: Mesa's link drops the `dummy` varying | the link (mali_ref cannot reach it) |
| `vsmat3_light` | refused at its loop, then the `Light` array | lower-loops, lower-uniarray |

## The int uniform rules and what is left (spike-uniarray-rest2, at 87388100)

Two vertex rules for `int` uniforms moved with
`docs/notes/mali-deqp-uniarray.md` section 10. Neither changes a byte of
an existing reference:

- **A mediump int read only as an fp16 array's run-time index** passes
  the "vertex shader uniform that is not highp" rule (mod.rs, nir.rs
  `index16_only`). Mesa narrows no integer (`caps->int16` is false,
  pan_screen.c:591), so the int is its 32-bit word whatever its precision
  (deqp_uniarray_dyn_vec{2,3,4}_vs: `LSHIFT_OR.i32 r1, u5.w1, r0.b22, 0x0`).
  Any other read of a mediump int still refuses there. The refusal table's
  source for that gap (src/shaderc/online/mali/tests.rs) is now a mediump
  int index of a highp float array.
- **An int uniform read only through `float(i)`** passes "a uniform other
  than a float, vec2, vec3, vec4 or mat4" (nir.rs `int_float_only`). Its
  word is converted in place, `S32_TO_F32 r2, u7.w0`
  (deqp_uniarray_struct_member_vs). Any other arithmetic on it keeps
  `UniformType`: the gap's source is now `float(i + 1)`, and so are
  `an_int_uniform_outside_an_array_index_is_a_raw_word` and
  tests/mali_ref_selftest.rs `a_refused_shape_stays_refused`. Because of
  this, `deqp_unif2_arr_struct_array` (tests/mali_deqp_unif2.rs) now stops
  later: its `float(s[0].c)` passes, and the precision rule stops it at the
  members of the nested struct array (`s[0].b[1].b[0]`). Its pin moved.

The three references `tests/mali_deqp_unitype.rs` still ignores, at this
tip:

| reference | where it stands | what it needs |
|---|---|---|
| `vsapi_struct` | differs: 13 words against Mesa's 17 | Mesa narrows the five fp32 struct members two at a time (`V2F32_TO_V2F16 r0, u6.w1, r0^` of `m1.y, m1.z`; `r1` of `m0, m1.x`; `FADD.f32 r2.h0, u7.w1, 0x0.neg` of `m1.w` alone). `nir_opt_vectorize` then re-pairs the lanes with `MKVEC.v2i16` for a tree of `FMAX.v2f16` with `.abs` sources. One `FCMP_OR.v2f16.lt.m1` follows against the fp16 pair `0x2a662a66` of 0.05, then `S16_TO_S32` and `CSEL.u32.eq`. `vs_cmp_and` builds that tree for fp16 uniforms (`vsapi_mat2`). For fp32 members, the walk keeps an fp32 `FCMP_OR` per lane and their product. The lowering needs (a) the narrowed members as `vs_cmp_side`s and (b) Mesa's vectorizer's pairing of a five-leaf `fmax` tree, the `MKVEC` pairs and their `.h00`/`.h11` order. |
| `fsmat3_mediump` | refused, `FsOperandForm` (a lane alone first in a sum, a uniform first) | `vec16.rs`'s column chunks with the `v_coords` varying narrowed once. That route has no varying layout. |
| `fsmulti` | refused, `FragColorLane` | the float, ivec3, mat3 and bvec2 compares in one product of the colour (intval's fragment compare forms) |
