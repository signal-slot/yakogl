<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the mediump fragment-shader uniform refusal at 8ff01055, its shapes and Mesa 26.2.1 references

**Status: study, one shape lowered** (plan #402, items `ref-fsunif` and
`lower-fsunif`; issue #20). `lower-fsunif` made `struct_basic` byte-equal
(section 6); the other references stay refused. The references are `tools/mali/testdata/deqp_fsunif_*`, and
`tests/mali_deqp_fsunif.rs` pins them. Measured HOST-ONLY at the integration
tip **`7462e3d5c5a5`** (2026-11-25). Nothing in this note was checked on the
NanoPC-T6. Mesa paths are relative to `src/` of the 26.2.1 tree.

The refusal is `MaliError::FsUniform` (`src/shaderc/online/mali/mod.rs:437`):
"a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4
array or int read in its own fp16 lane pairs or as one lane, or one declared
highp". On the board (`docs/es2-cts-failures.md` section M) it is 271 Fail
and 1 NotSupported at `8ff01055`. `docs/notes/mali-deqp-unitype.md` and
`docs/notes/mali-deqp-unif2.md` drew the uniform shapes of their time; this
note groups the cases left, draws references for the shapes that had none,
and names the earlier reference for each of the others.

## 1. How this was measured

- **Cases:** the 272 cases whose `refusals.tsv` text in the slices
  `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds this refusal
  (271 Fail, 1 NotSupported in `results.tsv`).
- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --jobs 3 --slice <each of the four>
  --cases <the 272 names>`. `--filter` matches case names, not refusal text,
  so the list went in through `--cases`. VK-GL-CTS is at `93bca018` with
  `cts-yakogl-target.patch`, fetched shallow and sparse (without the Vulkan
  and GL 4 data). All 272 cases began, and 3 processes exited 1. **All 272
  have this text as their first host refusal**, each in the fragment shader:
  0 not reproduced, 159 distinct programs.
- **Refusal sites:** a scratch probe outside the commit (a detached worktree
  whose `mod.rs`, `select.rs`, `nir_fs.rs` and `vec16.rs` record
  `file!():line!()` at every `MaliError::FsUniform`, with line numbers
  unchanged) compiled every program in the measure's `programs.tsv` through
  `tests/mali_ref`'s `compile_pair`. Each program refuses at one of six
  sites (section 2). The same probe over every existing
  `tools/mali/testdata/deqp_*` pair names the references that already stand
  on this refusal (section 3).
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_fsunif_<shape> --instrument shim`, run on the
  drm-shim that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was byte-equal. The
  pairs are verbatim from the measure's `.qpa` (the refusing program of the
  case). The `.license` beside each shader names the case.

## 2. The refusal sites at the tip

| Site | What it refuses | Programs | Cases |
|---|---|--:|--:|
| `select.rs:628` (`UboLayout::fs16`, the `_ =>` arm) | the type: an array of `int`, `bool`, `bvecN`, `ivecN` or a matrix; a struct member of those is the same row of the flattened uniform list | 45 | 141 |
| `nir_fs.rs:10089` (`lane_tree`, two uniform halves) | two halves of the fp16 block in one word that are neither one half twice nor a uniform's own aligned pair | 39 | 55 |
| `nir_fs.rs:10102` (`lane_tree`, one uniform half) | a half of the fp16 block beside a lane that is no uniform half (a computed lane, a varying's) | 55 | 56 |
| `nir_fs.rs:9678` (`lane_tree`, two fp32 words) | two words of an fp32 uniform (declared highp, a struct's member) that are not consecutive words of one load | 15 | 15 |
| `nir_fs.rs:9683` (`lane_tree`, one fp32 word) | a word of an fp32 uniform beside a lane that is no fp32 uniform word | 4 | 4 |
| `mod.rs:1601` (`fp32_uniforms`, a 64-bit mask) | a float fp32 uniform past the 64th row of the uniform list | 1 | 1 |

## 3. The shapes at the tip (HOST-ONLY)

| Shape | Cases | Board Fail / NS | Site | Reference |
|---|--:|---|---|---|
| **`arr_bvec`**: `uniform bvec4 u_var[3]`, `uniform_api.{info_query,value.*}.basic_array*.bvec4*_fragment` | **16** | 16 / 0 | `select.rs:628` | **new**: `arr_bvec4` |
| **`arr_ivec`**: `uniform mediump ivec4 u_var[3]` | **11** | 11 / 0 | `select.rs:628` | **new**: `arr_ivec4` |
| **`arr_mat`**: `uniform mediump mat4 u_var[3]` | **8** | 8 / 0 | `select.rs:628` | **new**: `arr_mat4` |
| **`struct_int`**: `struct { mediump int m0; mediump ivec4 m1; mediump int m2[3]; mediump ivec4 m3[3]; }` (`array_in_struct.int_ivec4_fragment` 11), and its nested form (`nested_structs_arrays.int_ivec4_fragment` 7) | **18** | 18 / 0 | `select.rs:628` | **new**: `struct_int_ivec4` |
| **`mat16`**: `shaders.matrix.{add,sub,mul,mul_assign,matrixcompmult}.{uniform,dynamic}_{lowp,mediump}_*_fragment`: a mat2 (14 cases), mat3 (2) or mat4 (16) whose halves the expression reads across the fp16 pairs | **32** | 32 / 0 | `nir_fs.rs:10089` | **new**: `mat4_add_float`, `mat2_mul_mat2` |
| **`mat32`**: the same group at highp (`uniform highp matN` into the mediump `gl_FragColor`): mat2 2, mat3 4, mat4 5 | **11** | 11 / 0 | `nir_fs.rs:9678` | **new**: `highp_mat4_add_float` |
| **`struct32`**: `shaders.struct.uniform.*_fragment`, mediump float members (Mesa keeps a struct's members fp32) written to the colour lane by lane | **8** | 7 / 1 | `nir_fs.rs:9683` 4 (`basic`, `array_member`, `sampler`, `sampler_array`), `nir_fs.rs:9678` 4 (`nested`, `struct_array`, `loop_struct_array`, `dynamic_loop_struct_array`: the NotSupported one) | **new**: `struct_basic`; covered: `deqp_uniarray_struct_member_fs` (`array_member`) |
| `arr_bool`: `uniform bool u_var[3]` | 16 | 16 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_bool_uniform` |
| `arr_int`: `uniform mediump int u_var[3]` | 11 | 11 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_int_uniform` |
| `struct_bool`: the struct of `bool`, `bvec4` and arrays of them (`array_in_struct.bool_bvec4_fragment` 11, nested 7) | 18 | 18 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_struct_uniform` |
| `arr_float`: `uniform mediump float u_var[3]`, the three elements compared (`basic_array.float_fragment` 9, `float_both` 9) | 18 | 18 / 0 | `nir_fs.rs:10089` | covered: `deqp_uniarray_api_float` |
| `rnd_two`: two float uniforms in one word of an expression, `shaders.random.{basic_expression,comparison_ops,conditionals,scalar_conversion}` | 5 | 5 / 0 | `nir_fs.rs:10089` | covered: `deqp_unif2_fs_other` (the fragment shader of `basic_expression.{combined,fragment}.78`) |
| `rnd_lane`: a uniform's half beside a computed lane: `shaders.random.*` 44, `fragment_ops.interaction.basic_shader.{30,75,86}` 3 | 47 | 47 / 0 | `nir_fs.rs:10102` | covered: `deqp_intval3_rsg_swizzle12_fs` (by its name `shaders.random.swizzle.fragment.12`, not one of the 272: the board at `8ff01055` refused that case with "an integer or boolean value"; at the tip the reference refuses at this site) |
| `invariance`: `shaders.invariance.{highp,mediump,lowp}.common_subexpression_{0,2,3}` (`uniform mediump vec4 u_color`) | 9 | 9 / 0 | `nir_fs.rs:10102` | covered: `deqp_misc2_invariant_cse` (the fragment shader of `mediump.common_subexpression_0`) |
| `struct_mat`: the struct of `mat4`, `mat2` and arrays of them (`array_in_struct.mat4_mat2_fragment` 9, nested 5) | 14 | 14 / 0 | `select.rs:628` | not drawn (the limit of 8): `arr_mat4`'s words in `deqp_unif2_fs_struct_uniform`'s member order |
| `multi_arr`: `float u_var_0[3]`, `ivec3 u_var_1[3]`, `bvec2 u_var_2[3]` together (`multiple_basic_array.fragment`) | 9 | 9 / 0 | `select.rs:628` | not drawn: `arr_ivec4` and `arr_bvec4` beside `deqp_uniarray_api_float` |
| `multi_nested`: three nested structs of float and vec4 arrays (`multiple_nested_structs_arrays.fragment`) | 7 | 7 / 0 | `select.rs:628` | not drawn |
| `api_random`: `uniform_api.random.{4,9,13,23,31,34,39,42,47,62,80,81,89,93}` | 14 | 14 / 0 | `select.rs:628` 13, `mod.rs:1601` 1 (`random.23`) | not drawn |
| **all** | **272** | 271 / 1 | | 104 cases in the 7 shapes with the 8 new references (1 of them on an earlier one); 124 cases on 7 earlier references; 44 cases not drawn |

"Covered" means an earlier reference refuses at the same site with the same
kind of uniform. It does not mean that one lowering takes every case of the
row: the 47 `rnd_lane` programs are 47 different expressions, and what each
refuses with once its uniform is read is `unmask-forecast`'s question.

## 4. What Mesa emits, per new reference

The fragment program's FAU: `uN.wM` is 32-bit word `2N + M` of the table;
words 0..3 (u0, u1) are 0 in every reference here and no program reads
them, the pushed uniform words start at u2, the program's own constants follow them. `--uniform-values`
gives component `c` of the `k`-th uniform leaf the value `16 (k + 1) + c`
(gl_ref.py). **A plain array uniform (`uniform T u[3]`) stays unset**: the
three `arr_*` references have every uniform word 0, so their expect.json
checks the program's words and the FAU's length and constants, but not
which element stands in which word. Where the words stand is read below from
the operations and their constants.

Mesa pushes only the words the program reads, in the order of the uniform
storage, and then `bi_opt_reorder_push` (`panfrost/compiler/bifrost/
bi_opt_push_ubo.c`) moves two words that one instruction reads into one
64-bit FAU slot (an instruction reads one slot).

| Reference (case) | FAU words of the uniforms | fp16 conversions and the ops of note |
|---|---|---|
| `arr_bvec4` (`uniform_api.value.assigned.by_pointer.render.basic_array.bvec4_fragment`) | 12 words, u2..u7, one 32-bit word a lane (`0` or `~0`), two slots an element: element 1 in u4, u5 and element 2 in u6, u7 in lane order (x = `.w0` of the first slot). Element 0, compared with `(true, true, false, false)`, has its two false lanes in **u2** (`LSHIFT_OR.i32 r1, u2.w1, 0, u2.w0`, then `.not`) and its two true lanes in u3: the reordered pair. The constant `0x3c003c00` is u8.w0. | none: no value is a float before the result. `a == b` is `LSHIFT_AND.i32` over the lanes, a lane compared with false inverted first (`LSHIFT_OR.i32 r, 0, 0, u.not`). The product of the three compares is the AND of the masks, `MUX.v2i16 r0, 0, 0x3C000000.h11, r0.h00` makes the fp16 1.0 or 0.0, and the colour is `(r, r, r, 1.0)`: `IADD.v2u16 r0, r0.h00, 0` and `MKVEC.v2i16 r1, r0.h0, u8.w0.h0`. `v_vtxOut` is the constant 1.0 the link propagates: no varying is loaded. |
| `arr_ivec4` (`...render.basic_array.ivec4_fragment`) | 12 words, u2..u7, one 32-bit word a lane, in element and lane order (element `e` lane `l` is word `4 + 4e + l`: `u2.w0 == -7`, `u2.w1 == -2`, `u3.w0 == -5`, `u3.w1 == -5` for element 0). u8.w0 is the constant -5. | none: `caps->int16` is false (`gallium/drivers/panfrost/pan_screen.c`), so `nir_lower_mediump` narrows no integer. A lane is `ICMP_OR.u32.eq.m1 r, u, <constant>, 0` (the first of a pair) or `ICMP_AND.u32.eq.m1 r, u, <constant>, r` (chained); the constant is an immediate (`0x1000000.b3` = 1), an `IADD_IMM.i32` or a FAU word. The colour as `arr_bvec4`'s, the 1.0 from `IADD_IMM.i32 r2, 0, #0x3C003C00`. |
| `arr_mat4` (`...render.basic_array.mat4_fragment`) | 24 words, u2..u13: 48 fp16 halves, 8 words an element, 2 words a column, in order (`u2.w0` = `u_var[0][0].xy`). u14.w0 is `0x2a662a66` (0.05), u14.w1 `0x3c003c00`. | the uniforms are fp16 in the table (`lower_precision` narrows the `mediump mat4[3]`, the driver uploads halves): no conversion in the program. `abs(a - b) < 0.05` of a word is `FADD_IMM.v2f16 r, u, #<-b.y, -b.x>`, then `FMAX.v2f16 r, r.abs.h00, r.abs.h11`; the 48 compares fold into a tree of `FMAX.v2f16` and one `FCMP_OR.v2f16.lt.f1 r0, r0.h00, u14.w0, 0`. |
| `struct_int_ivec4` (`...render.array_in_struct.int_ivec4_fragment`) | 20 words, u2..u11, one 32-bit word a lane, the struct flattened in member order: `m0` u2.w0; `m1` u2.w1, u3.w0, u3.w1, u4.w0; `m2[0..2]` u4.w1, u5.w0, u5.w1; `m3[0]` u6.w0..u7.w1; `m3[1]` u8.w0..u9.w1; `m3[2]` u10.w0..u11.w1 (the values `0x10`, `0x20..0x23`, `0x30`, `0x40`, `0x50`, `0x60..0x63`, `0x70..`, `0x80..`: an array member's element is a leaf of its own). u12.w0, u12.w1, u13.w0 are the constants -6, -9, -10. | none, as `arr_ivec4`. The words are odd-aligned after `m0`: an `ivec4` straddles three slots, and no reorder happens (each instruction reads one uniform word and a constant). |
| `mat4_add_float` (`shaders.matrix.add.uniform_mediump_mat4_float_fragment`) | 8 words, u2..u5: the 16 fp16 halves, 2 words a column, in order (`0x4c404c00` = 16.0, 17.0 in u2.w0). u6.w0 is `0x3c003c00`. | the varying is loaded at 32 bits and narrowed alone (`LD_VAR_BUF_IMM.f32`, `FADD.f32 r0.h0, r0, -0` = `F32_TO_F16`); the matrix needs none. `res[0].xyz + res[1].yzw + res[2].zwx + res[3].wxy`: each `FADD.v2f16 r, <uniform>, r0.h00` reads a word whole when the two lanes are its own pair (`u2.w0`, `u3.w1`, `u4.w1`, `u5.w0`) and one half twice when a lane stands alone (`u3.w0.h11`, `u2.w1.h00`, `u4.w0.h00`, `u5.w1.h11`); the sums then take `.h00`/`.h11` of those results and `MKVEC.v2i16` pairs the lanes. No instruction reads two different FAU halves into one word. |
| `mat2_mul_mat2` (`shaders.matrix.mul.uniform_mediump_mat2_mat2_fragment`) | 4 words, u2..u3, a word a column, **reordered**: `u_in0`'s columns are u2.w0 (`0x4c404c00`) and u3.w0 (`0x4cc04c80`), `u_in1`'s u2.w1 (`0x50205000`) and u3.w1 (`0x50605040`), so that each `FMA` reads one slot. u4.w0 is `0x3c003c00`. | none. A column of the product is `FMA.v2f16 r, u3.w0, <u_in1 col>.h11, -0` then `FMA.v2f16 r, u2.w0, <u_in1 col>.h00, r`: a column word of `u_in0` times one half of `u_in1`'s column, read as `u2.w1.h00` in place or through `MOV.i32 r, u2.w1` (`r.h11`) when the instruction's slot is the other one. |
| `highp_mat4_add_float` (`shaders.matrix.add.uniform_highp_mat4_float_fragment`) | 12 words, u2..u7, fp32, only the components read (`[0].xyz`, `[1].yzw`, `[2].xzw`, `[3].xyw`: 16, 17, 18, 21, 22, 23, 24, 26, 27, 28, 29, 31 as floats), packed in storage order. u8.w0 is `0x3c003c00`. | everything at 32 bits (`FADD.f32 r, u, r0` twelve times and six sums); the three colour lanes are narrowed **each alone** (`FADD.f32 r.h0, r, -0`) and paired by `MKVEC.v2i16` (`r2.h0, r1.h0`; `r0.h0, u8.w0.h0` with the constant 1.0). No `V2F32_TO_V2F16`: the lanes are three different sums. The varying is `highp`: loaded at 32 bits, not narrowed. |
| `struct_basic` (`shaders.struct.uniform.basic_fragment`) | 4 words, u2..u3, fp32 (a struct's member is not narrowed, `compiler/glsl/lower_precision.cpp`): `s.a` u2.w0 (32.0), `s.b.x` u2.w1 (48.0), `s.b.y` u3.w0 (49.0), the int `s.c` u3.w1 (`0x40`). `s.b.z` and `ui_one` are not read and not pushed. | `gl_FragColor = vec4(s.a, s.b.x, s.b.y, s.c)`: the first word of the colour is `V2F32_TO_V2F16 r0, u2.w0, u2.w1` (two fp32 words of one slot), the second is `s.b.y` narrowed alone (`FADD.f32 r1.h0, u3.w0, -0`) beside `S32_TO_F32 r0, u3.w1` narrowed alone, then `MKVEC.v2i16 r1, r1.h0, r0.h0`. |

## 5. For the lowering (`lower-fsunif`)

- The three type shapes (`arr_bvec`, `arr_ivec`, `struct_int`) need rows in
  `UboLayout::fs16` only for the layout; their reads are integer compares
  (`fsint`'s and `intval`'s ops), with no fp16 word. `arr_mat` is the one
  that adds fp16 words: `mat4`'s row at 8 words an element.
- `mat16` and `mat32` are reads, not layouts: a half or a word alone
  (`.h00`/`.h11` of a FAU word, `F32_TO_F16` of one fp32 word) where the tip
  takes only an aligned pair. `mat2_mul_mat2` also needs the reordered
  slots: which words share a slot follows the instructions.
- A reference whose struct members gl_ref.py set compares its FAU with
  `mali_ref::differences_members(.., true)` (`assert_matches_members` in
  the test): `struct_int_ivec4` and `struct_basic` are not in
  `mali_ref::STRUCT_VALUES`.
- The three `arr_*` references cannot check the upload order of the array's
  words (all 0): gl_ref.py would have to set a plain array through
  `u_var[0]`.

## 6. Lowered (`lower-fsunif`, HOST-ONLY)

Measured HOST-ONLY on the tip `0491f842` and nothing more; not checked on
the NanoPC-T6.

- **`struct_basic` is byte-equal** and in `MATCHED` (compared through
  `assert_matches_members`). The colour's second word, an fp32 uniform word
  beside an int uniform's float (`s.b.y`, `s.c`), is the new pair leaf
  `Leaf::WideInt` of `nir_fs.rs` (`vectorise`'s fp32-word arm, which refused
  it): the int is converted and narrowed alone (`S32_TO_F32`, `FADD.f32
  .h0`), the word narrowed alone (`FADD.f32 r1.h0, u3.w0, -0`), the two
  halves paired by `MKVEC.v2i16` before ATEST, which reads the int's half.
  The scheduler (`sched.rs`, `bi_pressure_schedule`) sinks the first word's
  `V2F32_TO_V2F16` past ATEST as Mesa does. The first word was already
  `Leaf::Wide`.
- **`deqp_uniarray_struct_member_fs` is byte-equal too** (the same shape,
  `shaders.struct.uniform.array_member_fragment`): `tests/mali_deqp_uniarray.rs`
  un-ignores `struct_member_fs` and drops its `FsUniform` pin.
- The site that raised it in this tree was the `_ =>` of that arm (the
  section 2 row `nir_fs.rs:9683`; the lines moved since `7462e3d5`).
- **Still refused**, with their sites in this tree: `arr_bvec4`,
  `arr_ivec4`, `arr_mat4` and `struct_int_ivec4` at `select.rs:628` (the
  type); `mat4_add_float` and `mat2_mul_mat2` at the two-uniform-halves
  arm of `lane_tree` (the section 2 row `nir_fs.rs:10089`). The matrix
  shapes need Mesa's pairing predicate over fp16 uniform lanes (one half
  alone as `.h00`/`.h11`, sums of single lanes, `MKVEC.v2i16` of computed
  halves) and, for `mat2_mul_mat2`, `bi_opt_reorder_push`'s slots; the type
  shapes need `UboLayout::fs16` rows for arrays and int-member structs.
- The `FsUniform` pins of `tests/mali_deqp_boolpix.rs` and
  `tests/mali_deqp_audit.rs` (two struct-member words narrowed as a pair on
  the `bool_colour`/`int_colour16` routes) did not move: `Leaf::WideInt`
  covers a word beside an int's float, not that pair.
- No glmark2 verdict moved (`mali_glmark2_corpus` passes unchanged).
