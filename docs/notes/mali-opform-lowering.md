<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the fragment operand-form refusal, a lowering design

Plan #387, item `opform-design` (HOST-ONLY, no board). A design for later
lowering items. It changes no compiler code. It adds the Mesa 26.2.1
references `tools/mali/testdata/deqp_opform_*` and their test
`tests/mali_deqp_opform.rs`.

The text is `MaliError::FsOperandForm`:

> Valhall: a fragment shader operand of a form or in a place no reference
> draws (a lane alone first in a sum, a uniform first) is not lowered yet

Citations are `<file>:<line>` under `src/shaderc/online/mali/` at the
integration tip **`8133663ca5da`** (`8133663ca5da6042860053ff2c68484fdcaf7c9a`).
Mesa's words are quoted from the `text` of the references' own `.expect.json`.

## 0. Summary

1. **The cases.** The Verdin iMX95 (Mali-G310) dEQP-GLES2 tree at
   `e63ab28e` (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*/console/`)
   prints the text **190 times, for 190 cases**. For **181** of them it is
   the case's first refusal. The other 9 are `combined` random cases whose
   vertex shader is refused first (section 6). The item named 188 lines at
   `a3d8d2de`. The console logs in the tree are the `e63ab28e` sessions,
   and those are what was counted here.
2. **Where they stop at the tip.** Each case's pair was rebuilt from its
   generator (section 2) and compiled at the tip in a throw-away
   instrumented copy of the library (not committed). 169 of the 181 still
   stop at this text. 10 now compile and 2 now stop at `ArithConstant`.
   Of the 169:
   - 141 stop at one site: the vectoriser's productions gate
     (`nir_fs.rs:12009`, `f.prods & !admitted != 0 || splat`).
   - 28 stop at four smaller sites: `:10649` (12), `:11494` (8),
     `:11124` (4) and `:10373` (4).
3. **The train does not reach them.** The same probe on the `intval-rest`
   train (`224cc3f7`, `src/` of its three commits applied on the tip) gives
   the same outcome, site and productions for all 181 cases.
4. **What the arithmetic design already plans.** Step 4 of
   `docs/notes/mali-arith-lowering.md` (the pair trees, A4a/A4b in
   `docs/notes/mali-lowering-schedule.md`) names most of the forms. Its
   references in `tests/mali_deqp_arith.rs` pin 10 of the 15 classes below
   (O1, plus `Op@AddB`, `HalfSwapped@Output` and the `b2f` mask). The
   **residue** is the forms no design names:
   - vec3 `min`/`max`/`clamp` with non-constant bounds (A4a lowered only
     the scalar forms);
   - matrix negation (`Neg@AddA`);
   - a flattened `if` whose compare reads a computed lane;
   - a swapped half as a product operand;
   - a uniform half splat as an output word;
   - a constant/lane pair under a negation;
   - a splat as an FMA's addend.

   The 12 new references cover the residue. They also pin one shape each of
   the `matrix`, `return` and `conversions.vector_combine` groups, which no
   arith reference does.
5. **Forecast.** Section 5 lists 8 steps. If every step's productions were
   admitted, 169 cases would leave the text. With the gate switched off in
   a throw-away copy, 97 of the gate's 141 then compile (words not
   compared). The other 44 stop at another text: `FsVaryingLoad` 23,
   `MixedPrecision` 6, the link's varying offsets 6 (`MALI_LAYOUTS_DIFFER`, `link.rs:36`), `ArithConstant` 4,
   `Defect(Operand)` 3, `ColourForm` 2.

## 1. The text and its sites at `8133663ca5da`

The text is at `mod.rs:524`, and `nir_fs.rs` raises it at 20 sites. The
probe printed `line!()` at each site. The cases reach five of them:

| Site | What it refuses | First-refusal cases |
|---|---|---|
| `nir_fs.rs:12009` (the `return` of the `if` at `:12008`) | the productions gate: a node `(Form, Place)` outside `ADMITTED` (`nir_fs.rs:1039`), plus the `admitted` extensions `cross`/`mid`; or a uniform half splat in both lanes of an output word (`splat`) | 141 |
| `nir_fs.rs:10649` | `vectorise`'s `Fmax \| Fmin` arm: the bound pair is not a constant | 12 |
| `nir_fs.rs:11494` | `lane_tree`'s `B2f` arm: the 16-bit mask is not an `Fcmp` (`Icmp`, `Ior`, `Iand`, `Ixor` of `Fs::mask16`) | 8 |
| `nir_fs.rs:11124` | `cmp_node`, a flattened `if`'s compare: an operand that is neither a constant nor a uniform's half | 4 |
| `nir_fs.rs:10373` | `vectorise`'s `Fexp2` pair: varying lanes that are not one aligned pair (`.zxy`) | 4 |

At the gate, the probe printed the bits of `f.prods & !admitted` by form and
place (`Form`: `nir_fs.rs:991`, `Place`: `:1012`).

## 2. How the pairs were made

- **`random.*` and `fragment_ops.interaction.basic_shader`** (64 cases).
  The pairs come from `rsg::ProgramGenerator` (`framework/randomshaders`,
  VK-GL-CTS 93bca01), built on the host with a small driver. The driver
  sets the parameters of `es2fRandomShaderTests.cpp`, and
  `es2fFragOpInteractionTests.cpp`'s seed `0x667eacfd ^ deInt32Hash(ndx)`.
  Two sources taken verbatim from a `.qpa` earlier came out byte-equal
  (`deqp_cfold2_random_mixed_icmp` = `random.conditionals.vertex.45`,
  `deqp_cfold2_random_mixed_f2i` = `random.texture.vertex.29`).
- **`matrix.*`, `operator.*`, `return.*`, `conversions.*`, `linkage`**.
  These are written from the generators (`es2fShaderMatrixTests.cpp`,
  `es2fShaderOperatorTests.cpp`, `es2fShaderReturnTests.cpp`,
  `glsShaderLibraryCase.cpp` with `conversions.test`/`linkage.test`). Three
  checks came out byte-equal to existing references:
  - `matrix.add.const_mediump_mat3_float` = `deqp_arith_matrix_storeout_fs_mediump`;
  - `conversions.matrix_combine.bool_float_int_vec2_bool_bvec2_float_to_mat3` = `deqp_arith_other_storeoutput_fs_mediump`;
  - seven `operator` shapes = their `deqp_arith_op_*_fs_mediump`.
- **Precision.** `lowp` and `mediump` lower the same way: both are fp16
  in Mesa and in our walk. One measured `mediump` pair stands for both.

The CTS sources are a sparse clone at 93bca01
(`$CONNECTED_DEPS_DIR/VK-GL-CTS-gles2src`: `modules/gles2/functional`,
`modules/glshared`, `data/gles2/shaders`, `framework/randomshaders`,
`framework/delibs`, `framework/common`, `framework/referencerenderer`).

## 3. The form classes

The 169 first-refusal cases still at the text. A case with two productions
counts under the later step (section 5). "Arith" names the reference in
`tests/mali_deqp_arith.rs` that already pins the form, if any. "Here" names
the reference this item adds.

| Class | Production(s) / site | What the shader does | Cases | dEQP groups (cases) | Arith ref | Here |
|---|---|---|---|---|---|---|
| MA | `Mk@AddA` | a pair made of halves of two words (`MKVEC`) first in a sum: a vec3 `.zxy` + 1.0, `++`/`--`, `+=` | 36 (+ 6 with NS, + 6 with OB, + 1 with MB) | operator.unary_operator 12, binary_operator.add* 6, random 16, interaction 2 | `op_binary_add_assign`, `op_unary_incdec` | - |
| MB | `Mk@MulB`, `Mk@FmaB` | the same pair second in a product, or as an FMA's multiplicand (`mix`, `reflect`, `*=`) | 19 + 8 | operator.binary_operator.mul* 8, common_functions.mix 4, geometric.reflect 2, random 13 | `op_binary_mul*`, `op_common_mix`, `op_geometric_reflect` | - |
| SA | `Splat@AddA` | a scalar lane in both halves first in a sum (`float + vec3`) | 6 (+ 2 with NS) | binary_operator.add 4, sub 2 | `op_binary_add` | - |
| NS | `Splat@NegSrc`, `Mk@NegSrc` | a splat or a pair under `fneg`: `vec - float`, unary `-` | 2 + 4 (+ 6 with MA, + 2 with SA, + 8 with OB) | binary_operator.sub* 6, unary_operator.minus 2, matrix.sub 2, random 2 | `op_binary_sub*`, `op_unary_unary_minus` | - |
| AB | `Mk@AbsSrc` + `Abs@FmaA` | `abs` of a pair under the colour's scale/bias FMA | 2 | common_functions.abs 2 | `op_common_abs` | - |
| NF | `Neg@FmaC` (+ `Mk@MulB`) | a negation as an FMA's addend (`cross`; random `a - b*c`) | 1 + 2 | geometric.cross 1, random 2 | `op_geometric_cross` | - |
| X2 | `:10373` | `exp2` of a vec3 `.zxy` varying | 4 | exponential.exp2 4 | `op_exponential_exp2_fs_*` | - |
| OB | `Op@AddB` | an operation's result second in a sum: the matrix reductions `res[0]+res[1]+res[2]`, random sums | 21 (+ 6 with NS, + 2 with CN, + 6 with MA) | matrix.add 6, matrix.sub 6, matrix.mul 3, random 17, interaction 1 | `matrix_storeout_fs_mediump` | `matadd_uniform`, `matsub_uniform` |
| HO | `HalfSwapped@Output` | a varying's swapped pair as an output word (`vec4(v.wzy, 1.0)`, `var.xyyx`) | 5 (+ 1 with SO) | return.*_never 3, linkage.varying_type_mismatch_2 1, interaction 1 | `other_storeoutput_fs_none` | `ret_never` |
| HS | `HalfSwapped@MulB` | a swapped pair second in a product (`float * vec4.wzyx`) | 3 | binary_operator.mul.*_float_vec4 2, random 1 | - | `mul_float_vec4` |
| MM | `:10649` | `min`/`max`/`clamp` of vec3 lanes with lane bounds | 12 | common_functions.min 4, max 4, clamp 4 | - (A4a lowered scalar forms only) | `min_vec3`, `clamp_vec3_float` |
| BM | `:11494` | `b2f` of a 16-bit mask that is not an fp16 compare: `bool(float)`/`int(..)` built into a vector or matrix | 8 | conversions.vector_combine 5, matrix_combine 3 | `other_storeoutput_fs_mediump` (matrix_combine) | `conv_vec3` |
| NM | `Neg@AddA` + `Mk@NegSrc` | the negation of a matrix's cross-slot column pairs, first in a sum | 4 | matrix.negation 4 | - | `matneg_mat3` |
| IF | `:11124` | a flattened `if` / `return` under `v.x + v.y >= 0.0` (a computed compare operand) | 4 | return.*_dynamic 3, interaction 1 | - | `ret_dynamic` |
| SO, CN, SC | `splat-output`, `Const@NegSrc`, `Splat@FmaC` | a uniform half in both lanes of an output word; a constant/lane pair under `fneg`; a splat as an FMA's addend | 2 (+ 1 with HO) + 2 + 2 | random 4, matrix.mul.const_*_mat3_float 2 | - | `rnd_splat18`, `rnd_constneg85`, `matmul_const` |

## 4. What Mesa emits

Mesa's words are quoted from the `fs.fragment` text of the references.
The varying loads, `ATEST`, `BLEND` and the blend branch are left out.

- **Lane grouping (MA, MB, SA, NS).** The common thread is
  `nir_opt_vectorize` regrouping lanes after `nir_lower_mediump_io`, not
  our pairs of source lanes. Mesa narrows the fp32 varying words as they
  are read: `V2F32_TO_V2F16` for an aligned pair, `FADD.f32 r.h0, r,
  0x0.neg` for a lone lane. It then computes each colour pair from those
  halves with `.h00`/`.h11`/`.h10` swizzles on the operands, and
  assembles the colour words last (`MKVEC.v2i16`).
  - `op_binary_add`: `FADD.v2f16 r1, r0.h00, r1^.h10` then
    `MKVEC.v2i16 r0, r1.h0, r0^.h0`.
  - `op_binary_sub`: `MKVEC.v2i16 r2, r1.h1, r2^.h0` *before* the
    operation. That form is admitted only beside a cross-slot lane today
    (`cross`, `nir_fs.rs:11999`).
- **OB (matrix sums).** `matadd_uniform`: each column word of the uniform
  plus the splat, then the column sums pairwise:
  ```
  FADD.v2f16 r2, u3.w0, r0.h00
  FADD.v2f16 r3, u2.w0, r0.h00
  FADD.v2f16 r2, r3^, r2^        ; an operation second in a sum
  ```
  The `.z` lane goes the same way with `.h00` uniform halves:
  `FADD.v2f16 r1, u3.w1.h00, r0.h00`.
- **NS with OB.** `matsub_uniform`: the same words, with the subtrahend as
  a source modifier, `FADD.v2f16 r2, u3.w0, r0.neg.h00`. No `fneg` node
  is left.
- **NM.** `matneg_mat3`: `-M` folds into the reduction's sums as modifiers
  on both operands:
  ```
  MKVEC.v2i16 r5, r1.h1, r2.h0
  FADD.v2f16 r0, r0^.neg, r5^.neg
  FADD.v2f16 r1, r1^.neg.h00, r2^.neg.h11
  FADD.v2f16 r0, r0^, r3^.neg
  ```
- **HO.** `ret_never`: the swizzle goes into the conversion's operand
  order, `V2F32_TO_V2F16.discard r0, r3^, r2^` (`.wz`), and `.y` is
  `FADD.f32 r1.h0, r1^, 0x0.neg`. There is no swap node.
- **HS.** `mul_float_vec4`: a swapped half swizzle on the FMA operand:
  ```
  FMA.v2f16 r2, r0.h00, r2^.h10, 0x0.neg
  FMA.v2f16 r0, r0^.h00, r1^.h10, 0x0.neg
  ```
- **MM.**
  - `min_vec3`: per lane pair, with the halves swizzled into the
    operation:
    ```
    FMIN.v2f16 r1, r1^.h00, r2.h11
    FMIN.v2f16 r3, r0.h00, r3^.h00
    FMIN.v2f16 r0, r0^.h11, r2^.h00
    MKVEC.v2i16 r1, r1^.h0, r3^.h0
    ```
  - `clamp_vec3_float`: `FMAX.v2f16 r1, r1^, r0.h00`,
    `FMAX.v2f16 r0, r2^.h00, r0^.h00`, `MKVEC`, then
    `FMIN.v2f16 r1, r1^.h11, r2.h00` and `FMIN.v2f16 r0, r0^, r2^.h00`.
- **BM.** `conv_vec3`, `vec3(bool, float, int)` with the library's
  `isOk`:
  - `bool(v)`: `FCMP_OR.v2f16.ne.f1 r1, r0.h00, 0x0, 0x0` (the `.f1`
    result type is the `b2f` folded into the compare).
  - `int(v * 1.0025)`: `F16_TO_F32`, `FROUND.f32.rtz`, `FADD.f32 r1.h0`.
  - The `isOk` masks: `FCMP_OR.v2f16.ge.m1` are combined by
    `LSHIFT_AND.v2i16 r1, r1^.h00, 0x0.b00, r2^`, then
    `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00`.

  The mask the arm refuses is the `iand` of the 16-bit compares that
  `all(lessThanEqual(..))` makes, read by `b2f`.
- **IF.** `ret_dynamic`: `v.x + v.y >= 0.0` becomes
  `FCMP_OR.v2f16.lt.m1 r4, r4^.h00, r4^.neg.h11, 0x0`. That is
  `x < -y`, with the arms swapped: the sum is an operand modifier, not a
  node. Then `S16_TO_S32 r4, r4^.h0` and three `CSEL.u32.eq r, r4, 0x0, a,
  b` on the **fp32** varying words, and the selected words narrowed after
  (`FADD.f32 r0.h0, r0^, 0x0.neg`).
- **SO.** `rnd_splat18`: `vec4(c, c, b, 0.0)` is
  `IADD.v2u16.discard r1, u2.w0.h00, 0x0` (the uniform half splat moved
  into a register) and `U16_TO_U32 r2, r0^.h0` for `(b, 0.0)`.
- **CN.** `rnd_constneg85`:
  `MKVEC.v2i16 r0, r0^.h0, u2.w0.h0` (lane `d` with the constant 7.5 from
  the FAU), then `FADD.v2f16 r0, u2.w1, r0^.neg`. The constant `vec4` is a
  uniform word first, and the pair under the negation is a modifier.
- **SC.** `matmul_const`: the constant matrix times the splat, reduced, is
  `FMA.v2f16 r1, r0.h00, u2.w0, 0x0.neg`. The constant column sums are
  folded into one FAU word, and the third lane is
  `MKVEC.v2i16 r2, r0^.h0, u2.w1.h0`.

## 5. Steps and host forecast

The forecast counts first-refusal cases of the iMX95 run (`e63ab28e`)
measured at the tip. A case leaves the text when every production or site
it makes is covered by the steps so far. "Next" is what those cases do
with the gate switched off (`if false && ..` at `:12009`, a throw-away copy).
"Compile" there means the pair compiles; its words are not compared with
Mesa's. Steps O1 to O3 are the arithmetic design's step 4 in this note's
terms.

| Step | Forms | Refs to un-ignore | Cases leaving the text | Cumulative | dEQP groups | Next, gate off |
|---|---|---|---|---|---|---|
| O1 | MA, MB, SA, NS, AB, NF, X2: Mesa's lane regrouping (narrow per read, operate on halves, `MKVEC` last; neg/abs as modifiers) | arith `op_binary_*`, `op_unary_*`, `op_common_abs/mix`, `op_geometric_*`, `op_exponential_exp2_fs_*` | 93 (+4 not first) | 93 | operator.binary_operator 28, operator.unary_operator 14, common_functions.mix 4 + abs 2, exp2 4, geometric 3, random.basic_expression 21, random.conditionals 6, random.scalar_conversion 3, random.comparison_ops 2, interaction 2, matrix.sub 2, other random 2 | 66 compile, `FsVaryingLoad` 12, link offsets 5, `ArithConstant` 3, `ColourForm` 2, `MixedPrecision` 1 |
| O2 | OB: an operation second in a sum (with NS/CN modifiers on its operands) | `matadd_uniform`, `matsub_uniform`, arith `matrix_storeout_fs_mediump` | 33 (+3) | 126 | matrix.add 6, matrix.sub 6, matrix.mul 3, random 17, interaction 1 | 21 compile, `MixedPrecision` 5 (highp matrices), `FsVaryingLoad` 5, `ArithConstant` 1, link 1 |
| O3 | HO, HS: swapped pairs into the conversion's operand order or as a `.h10` swizzle | `ret_never`, `mul_float_vec4`, arith `other_storeoutput_fs_none` | 8 (+1) | 134 | return.*_never 3, binary_operator.mul 2, linkage 1, interaction 1, random 1 | 2 compile, `FsVaryingLoad` 5, `Defect(Operand)` 1 (linkage) |
| O4 | MM: the `Fmax \| Fmin` arm with lane bounds (`:10649`) | `min_vec3`, `clamp_vec3_float` | 12 | 146 | common_functions.min 4, max 4, clamp 4 | not measured (a different site) |
| O5 | BM: `b2f` of an `iand`/`Icmp` mask in `lane_tree` (`:11494`) | `conv_vec3`, arith `other_storeoutput_fs_mediump` | 8 | 154 | conversions.vector_combine 5, matrix_combine 3 | not measured |
| O6 | NM: `Neg@AddA` (neg modifiers on both sum operands) | `matneg_mat3` | 4 | 158 | matrix.negation 4 | 4 compile |
| O7 | IF: `cmp_node` with a computed operand, the `CSEL` on fp32 words (`:11124`) | `ret_dynamic` | 4 | 162 | return.*_dynamic 3, interaction 1 | not measured |
| O8 | SO, CN, SC | `rnd_splat18`, `rnd_constneg85`, `matmul_const` | 7 (+1) | 169 | random 5, matrix.mul 2 | 4 compile, `Defect(Operand)` 2, `FsVaryingLoad` 1 |

Notes on the steps:

- **O1 and O2 are the bulk.** They share one mechanism: build the pair
  graph from Mesa's per-read narrowing, not from source-lane pairs. They
  should land together or in this order. Their risk is the one the
  arithmetic design names for step 4: `tests/mali_fs_arith.rs`,
  `tests/mali_fragment_random.rs`, the glmark2 corpus
  (`tests/mali_glmark2_corpus.rs`, `MKVEC.v2i16` order),
  `tests/mali_deqp_mixprec.rs` and `tests/mali_fs_operand.rs`.
  `ADMITTED`'s own test is `the_fragment_productions_are_the_references`
  (`tests.rs`): it must list every new production's reference.
- **O3, O6 and O8 are small.** They are admission plus a source modifier
  or an operand order, each with its reference.
- **O4, O5 and O7 change one arm each.** None of them reaches the gate.
- **After O1 to O3, `FsVaryingLoad` is next for 22 cases**
  (`docs/notes/mali-deqp-varyload.md`'s family). It is the follow-up to
  forecast once O1 lands.

## 6. Cases this design does not move

- **10 compile at the tip.** Since `e63ab28e`:
  - matrix.negation.dynamic_highp_mat4;
  - matrix.sub.dynamic_{lowp,mediump}_mat3_mat3;
  - matrix.sub_assign.dynamic_{lowp,mediump}_mat3;
  - random.conditionals.{fragment,combined}.{58,69};
  - random.texture.fragment.107.

  Their words were not compared with Mesa's. A board rerun confirms them.
- **2 stop at `ArithConstant` at the tip:**
  matrix.add/sub.const_highp_mat3_float (the constant-product family).
- **9 are not first refusals.** Their vertex shader is refused first on
  the board: random `combined` and interaction.6, at `ConstantFold` 2,
  `VaryingNotFromAttribute` 2, `UniformType` 3 and `ControlFlow` 2. They
  leave this text with the steps (4 with O1, 3 with O2, 1 each with O3/O8)
  but count only when their vertex refusal goes too.

## 7. The references and the test

`tools/mali/testdata/deqp_opform_<shape>.{vert,frag,expect.json}`, each
with `.license` files naming the dEQP case. They were drawn with
`tools/mali/deqp_ref.sh --instrument shim` and are identical on the G310v2
(`tools/mali/g310_check.sh 'deqp_opform_*'`: 12 identical, 0 differ).
`tests/mali_deqp_opform.rs` has the feature set
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`:

- one ignored byte-compare test per reference;
- `references_parse`: every file and the `fs.fragment` word count;
- `refusals_pinned`: the whole refusal text of each reference at the tip,
  in `REFUSED`.

A lowering step moves its references to `LOWERED` or `DIFFERS` in the same
commit.

| Reference | dEQP case (`dEQP-GLES2.functional.`) | Class | `fs.fragment` words |
|---|---|---|---|
| `matadd_uniform` | shaders.matrix.add.uniform_mediump_mat3_float_fragment | OB | 17 |
| `matsub_uniform` | shaders.matrix.sub.uniform_mediump_mat3_float_fragment | OB + NS | 17 |
| `matmul_const` | shaders.matrix.mul.const_mediump_mat3_float_fragment | SC | 10 |
| `matneg_mat3` | shaders.matrix.negation.dynamic_mediump_mat3_fragment | NM | 18 |
| `min_vec3` | shaders.operator.common_functions.min.mediump_vec3_fragment | MM | 20 |
| `clamp_vec3_float` | shaders.operator.common_functions.clamp.mediump_vec3_float_fragment | MM | 19 |
| `mul_float_vec4` | shaders.operator.binary_operator.mul.mediump_float_vec4_fragment | HS | 15 |
| `ret_dynamic` | shaders.return.output_write_dynamic_fragment | IF | 16 |
| `ret_never` | shaders.return.output_write_never_fragment | HO | 8 |
| `conv_vec3` | shaders.conversions.vector_combine.bool_float_int_to_vec3_fragment | BM | 29 |
| `rnd_splat18` | shaders.random.basic_expression.fragment.18 | SO | 10 |
| `rnd_constneg85` | shaders.random.scalar_conversion.fragment.85 | CN | 19 |

## 8. Re-measuring

The probe is not in the tree: it is an instrumented copy and a test that
compiles a directory of pairs. To redo it on a later tip:

1. Copy the tree. In `nir_fs.rs` of the copy, make every
   `Err(MaliError::FsOperandForm)` print `line!()`, and make the gate at
   `:12009` print the bits of `f.prods & !admitted` decoded as
   `Form@Place` (11 forms x 10 places, `bit = form * 10 + place`).
2. Compile each pair's fragment shader alone (`mali_ref`'s `compile`,
   made `pub` in the copy), then the pair (`compile_pair`).
3. Rebuild the random pairs from `framework/randomshaders` with the
   group's parameters, as in section 2.

A board rerun's console logs give the case list again.

## 9. Landed (plan #387, item `mali-train-3a`)

Re-probed at `5fd5bf43` with the recipe of section 8. The sites are now:
the gate `nir_fs.rs:12417` (27 references of `tools/mali/testdata`),
`:10671` (X2, the `:10373` site), `:10947` (MM, `:10649`), `:11844` (BM,
`:11494`) and `:11471` (IF, `:11124`). The references' productions are
section 3's.

With the gate switched off (a throw-away copy), the gate's references do
this: byte-equal are `deqp_opform_{matadd_uniform,matsub_uniform}`,
`deqp_unitype_fsmat3_mediump` and `deqp_intrnd_dead_cond_fs`; every arith
`op_binary_*`, `op_unary_incdec`, `op_common_{abs,mix}`,
`op_geometric_{cross,reflect}_fs_mediump`, `matrix_storeout_fs_mediump`,
`deqp_opform_{matmul_const,matneg_mat3,rnd_constneg85}` and
`deqp_posvary_flat_sep_kau_bs26` compiles with other words than Mesa's;
`op_unary_unary_minus_fs_mediump` and `random_storeoutput_fs_mediump` stop
at "a fragment varying load or conversion"; `rnd_splat18` at
`Defect(Operand)`.

- **O2, the uniform matrix only** (class OB, with NS). The gate admits
  `Op@AddB` and `Splat@NegSrc` when every sum whose second operand is an
  operation is a sum of column sums: a word of the fp16 uniform block plus
  a lane alone (or that lane negated), or a sum of two of those
  (`uniform_lane_sum`). Byte-equal: `deqp_opform_matadd_uniform`,
  `deqp_opform_matsub_uniform`, `deqp_unitype_fsmat3_mediump`.
  A constant matrix's sums (arith `matrix_storeout_fs_mediump`, the same
  production) are drawn otherwise by Mesa and stay refused.
- **O3, class HS only.** `HalfSwapped@MulB` is admitted, and
  `grafted_locals` grafts a vector local assigned a varying's swizzle
  (`in1 = v_in1.wzyx`) as GLSL's `opt_tree_grafting` does any rvalue: the
  product's tree then loads `v_in0` first and converts its lane before
  `v_in1`'s load, Mesa's order (without it our order had `v_in1`'s load
  first and the pre-RA scheduler chose another order than Mesa's).
  Byte-equal: `deqp_opform_mul_float_vec4`.
- **Not landed: O3's class HO** (`ret_never`, arith
  `other_storeoutput_fs_none`). With `HalfSwapped@Output` admitted,
  `ret_never` stops at "a fragment varying load or conversion of a form
  no reference draws" (`conversion_order`: Mesa's pair with the odd lane
  low, `V2F32_TO_V2F16 r0, r3^, r2^`, is not built) and
  `other_storeoutput_fs_none` at `Defect(Operand)`, so the production
  stays out of `ADMITTED`.
- **Not landed: O1, and O2's constant matrix.** Admission is not enough
  for them (the gate-off list above): they need the pair graph built from
  Mesa's per-read narrowing (section 5's note). `deqp_intrnd_dead_cond_fs`
  (`Mk@MulB` under a uniform's word, byte-equal with the gate off) is left
  to its own test's owner (`tests/mali_deqp_intrnd.rs`).

## 10. Landed (plan #387, item `mali-train-3c`)

HOST-ONLY: byte-compares against the Mesa references, no board run.

### O4 (class MM)

The rule the two references and three of `deqp_highpfs_pt_*` draw, in
`nir_fs.rs`:

- **Lanes of different conversions stay one lane each** (`split_minmax`,
  the `min`/`max` form of `split_sum`): `min(v_in0.zxy, v_in1.yzx)` is
  three `FMIN.v2f16` of one lane, and the colour's word is their `vec2`
  (`Leaf::Pack`). `mates` no longer pairs two such lanes.
- **Two lanes that share an aligned pair are one operation** (`mates`'
  `shared` for an `fmin`/`fmax`: two lanes of one varying only in one
  aligned pair of its components): `max(v_in0.zxy, v_in1.x)` pairs y and
  z, the varying's x and y (`FMAX.v2f16 r1, r1^, r0.h00`), and leaves x
  alone.
- **A lane beside one that is paired with a third is packed**
  (`mated_apart`): the `min` of x and y after that `max` reads
  `MKVEC.v2i16 r0, r0^.h0, r1.h0`. The gate admits `Mk@AbsSrc` for this
  form alone (a `vec2` of lanes computed apart as the first source of a
  `min` or `max`); `abs` of a pair stays out.
- **The bound may be one varying lane in both halves** (`Form::Splat`),
  not only a constant pair.
- A half of a two-lane operation in a `MKVEC.v2i16` is read at its own
  half (`lane_srcs` and the `Leaf::Pack` operand: `MKVEC.v2i16 r1,
  r1^.h1, u2.w0.h0`); it was read at `.h0`, which made
  `deqp_highpfs_pt_min_vec3_float` compile with one wrong word before
  this was fixed in the same step.

Byte-equal now: `deqp_opform_min_vec3`, `deqp_opform_clamp_vec3_float`,
`deqp_highpfs_pt_clamp_vec3`, `deqp_highpfs_pt_min_vec3`,
`deqp_highpfs_pt_min_vec3_float`. `--lib mali::` and every
`tests/mali_*.rs`: 1,410 passed, 0 failed.

### O8 (classes SC and SO; CN not landed)

- **SC, `matmul_const`.** The class's name (a splat as an FMA's addend)
  described our words, not Mesa's. `res[0] + res[1] + res[2]` of `mat3(1.0,
  0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 1.0) * v` is, at the link,
  `fadd(fadd(fmul(v, 1.0), fmul(v, 0.0)), fmul(v, 0.2))`: the factoring
  (`nir_opt_algebraic.py:246`) runs on the inner sum first and then on the
  outer one, so the lane is `fmul(v, 1.2)` (`FMA.v2f16 r1, r0.h00, u2.w0,
  0x0.neg` by `0x399a3ccd`, 1.2 and 0.7). The walk had folded the inner
  sum to `v` and so made `v * 0.2 + v`. `unit_product` (the sibling of
  `zero_product`) finds a value folded from its product with 1.0 in the
  IR, and the sum arm gives the factoring that product back. No gate
  change.
- **SO, `rnd_splat18`.** A uniform's half in both lanes of an output word
  is `MKVEC.v2i16 u.hN, u.hN`, which `opt.rs` `lower_mkvec_swz` already
  makes `IADD.v2u16 r1, u2.w0.h00, 0x0`. The gate's `splat` refusal is
  gone.
- **CN, `rnd_constneg85`: not landed**, still refused (`Const@NegSrc`,
  `Op@AddB`). With the gate off it compiles to 21 words against Mesa's
  19: Mesa has `i - vec4(-8.0, 4.0, d, 7.5)` as `FADD.v2f16 r0, u2.w1,
  r0^.neg` (the constant word first, the lane pair a negated source),
  ours a product and two `FADD_IMM`; the loads' order differs too.

Byte-equal now: `deqp_opform_matmul_const`, `deqp_opform_rnd_splat18`.
`--lib mali::` and every `tests/mali_*.rs`: 1,412 passed, 0 failed.

### O6 (class NM)

`matneg_mat3` needed the two productions (`Mk@NegSrc`, `Neg@AddA`) and
one place:

- The gate admits them when every `vec2` under an fneg is a matrix
  column's pair across two slots (`Leaf::Lanes` of two loads). A `vec2`
  of two lanes of one slot under an fneg (`unary_minus`, class NS) is
  still out.
- A matrix varying read whole (the front end's column loads one after
  the other) is loaded and converted where its first column is: GLSL's
  lower_precision converts the variable at the dereference. Our third
  slot's load stood at the third column's negation, after the first
  column pair's `MKVEC.v2i16`; Mesa has the three `LD_VAR_BUF_IMM` and
  their conversions first.

Byte-equal now: `deqp_opform_matneg_mat3`. `--lib mali::` and every
`tests/mali_*.rs`: 1,413 passed, 0 failed.

### Not landed in `mali-train-3c`

Their references stay in `REFUSED` with the operand-form text.

- **O5 (class BM, `conv_vec3`)**, site `lane_tree`'s `B2f` arm. Mesa's
  29 words hold three things the pair graph does not build: `bool(v)` as
  `FCMP_OR.v2f16.ne.f1` paired with a raw lane by `MKVEC`; `int(v *
  1.0025)` as `F16_TO_F32`, `FROUND.f32.rtz`, `FADD.f32 r1.h0` (an fp32
  island inside an fp16 tree); and the `all()` of the two compare words as
  two `LSHIFT_AND.v2i16` before one `MUX.v2i16`. Each is a new node kind
  of the graph, not an admission.
- **O7 (class IF, `ret_dynamic`)**, site `cmp_node`. Mesa selects the
  varying's fp32 words (`CSEL.u32.eq` three times on `S16_TO_S32` of the
  16-bit compare) and narrows after; our flattened `if` selects fp16
  lanes (`MUX.v2i16`). Letting `cmp_node` take a computed operand would
  compile other words than Mesa's. The compare itself is `x < -y` with
  the arms swapped (`FCMP_OR.v2f16.lt.m1 r4, r4^.h00, r4^.neg.h11`).
- **O8's class CN (`rnd_constneg85`)**: above.
- **O3's class HO (`ret_never`)** is train 3a's and was not touched.

`FsVaryingLoad` (`varyload-lower-1`): `deqp_varyload_tex110` became
byte-equal through mixprec2 M2-1 (its refused conversions were in
overwritten stores). The other seven `deqp_varyload_*` stay refused, at
three sites of `nir_fs.rs` (probed at this item's tip):

| site | references | what it refuses |
|---|---|---|
| `conversion_order`, `bases.count_ones() > 1` | `rnd57`, `cond36`, `basic33`, `basic8` | two or three float varyings packed in one slot, each converted by a tree: every varying has its own `f2fmp` at its own place |
| `conversion_order`, `!same` | `rnd31`, `cond19` | a pair Mesa converts with its odd lane low (`V2F32_TO_V2F16 r0, r1^, r0^`) |
| `whole_load` (`unsure && !all_read`) | `swz41` | a varying's lanes beside values of no measured kind |

No step was written for them: the first two need `conversion_order` to
place each packed varying's conversions, which no note designs yet.
