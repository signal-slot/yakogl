<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): one schedule of every lowering step, with case forecasts (plan #387, item lower-schedule)

**Status: planning note, no code.** Plan #387 has many lowering items that all edit
`src/shaderc/online/mali/**` and so run one at a time. This note puts the steps of every
merged lowering design into one table (section 2), proposes one serial order (section 3), and
lists the steps two designs claim (4a), the steps no item owns yet (4b) and the per-text
forecast against the 100-case line of criterion c2 (4c). It changes nothing under `src/`,
`tests/` or `tools/`, and edits none of the design notes.

## 0. Sources, and what the numbers are

Read: `mali-intval-vs16-lowering.md`, `mali-arith-lowering.md`, `mali-fclane-lowering.md`,
`mali-posvary-lowering.md`, `mali-unif2-lowering.md`, `mali-mixcol-lowering.md`,
`mali-constprod-lowering.md` (the merged designs), `mali-deqp-families.md` section 18 (the ranked table),
and, where **no design is merged yet**, the reference notes `mali-deqp-loops.md` section 9.5
(eleven steps), `mali-deqp-fmaloc.md` section 3, `mali-deqp-negzero.md` section 4,
`mali-deqp-intval.md`. Not in the tree at `7b540fee`, so not read: `mali-loops-lowering.md`,
`mali-fmaloc-lowering.md`, `mali-intval-rest-lowering.md`, `mali-cfold-lowering.md`,
`mali-ctrlflow-lowering.md`, `mali-deqp-irindex.md`, `mali-deqp-defects.md` (their items are
running, in review or not started). Rows built on a reference note are marked **(ref note)**:
their step split is that note's, and the design that lands may move it.

- **Forecast cases** are the design's own number: cases whose **first refusal** is the text
  and that the step moves. They are an **upper bound of cases that leave the text**, not
  passes: a case then meets its next refusal, and a random program finishes only when
  every operation in it is lowered. Where a design says "cases whose shader then compiles"
  the row says so; none of the merged designs forecasts passes, except where noted.
- The text counts of section 4c start from `mali-deqp-families.md` 18.2 (host, `9ac31f30`;
  Fail + NS + Pass of the 10,597 refused-at-`37305608` cases). They are **not** the board's
  must-pass counts. The 100-case line is applied to these counts; the board rerun
  (`board-rerun-1`) re-ranks.
- Text abbreviations: A `Arithmetic`, C `ColourForm`, UT `UniformType`, UP `UniformPrecision`,
  AC `ArithConstant`, LM `LoopMediump`, LV `LoopVertex`, FU `FsUniform`, IV `IntValue`,
  CF `ConstantFold`, UA `UniformArray`, IR `Defect(IrIndex)`, TC `TexCoord`, MP `MixedPrecision`,
  FO `FsOperandForm`, FMA `FusedMultiplyAdd`, NZ `NegZeroShared`.

## 1. How the rows are named

`IR1` irindex-fix; `V1..V4` intval vs16 sessions; `R1` intval-rest; `F1`, `F2B`, `F2D`, `F2C`, `F2E` fclane;
`A1a..A8` arith; `P1..P5` posvary; `U1..U9` unif2 (the design's step numbers); `MX0..MX5` mixcol (its step
numbers); `K0..K8` constprod (its S0..S8); `N1..N3` negzero; `LS1..LS11` loops (the ref note's S1..S11);
`T1`, `T2` fmaloc (ref note); `CF1`, `CT1`, `DF1` the texts with no design at all.

## 2. Every step of every design

Owner column: the plan item that owns the step today, or **no item yet**. "Needs" lists
prerequisite steps (section 3 turns them into the order).

### 2.1 Integer and boolean (intval, irindex)

| Id | Design step | Text(s) | Forecast cases (kind) | Un-ignores / refs | Needs | Owner today |
|---|---|---|---|---|---|---|
| IR1 | `Defect(IrIndex)` (families 18.4; `FsPrecision::of` re-parse of a bvec/bool source, unconfirmed) | IR | 187 (first refusal; 185 direct + 2 link errors; groups swizzles 108, conversions 45) | none known (irindex-ref in review) | - | irindex-fix |
| V2 | vs16 M5 + M6: the two gates (`UniformPrecision` for a mediump int, `UniformType` for an int uniform) and the `vs_is_uniform_word` bool-by-type fix | UT, UP | UT 346 (`vs_int_uniform`) + UP 99 first refusals; **a gate, not a pass count** (the cases move to loops/indexing/random). `vs_struct_uniform` 116 is UT too; whether M6 takes it is open (4c assumes it does not) | none; moves pinned refusals in `mali_deqp_loops.rs`/`mali_deqp_uniarray.rs` | - | intval-vs16 (vs16-2) |
| V1 | vs16-1: M1 narrow mediump operands, M2 `fmin`/`fmax` rule and 16-bit mask, M3 arm for scalar and `bvec3` | A (board: IntValue) | 8 + a few (first refusal) | `opvs_logical_and` DIFFERS->LOWERED, `opvs_not_equal_bvec3` REFUSED->LOWERED | - (V2 independent) | intval-vs16 (vs16-1) |
| V2b | vs16 section 10: the width-two (four after R9) 32-bit bool reduction `LSHIFT_OR/AND.i32 ... .not` for bool uniform words | A (`vs_reduce`) | 179 (`uniform_api.bool_compare` vs; first refusal) | `deqp_arith_uniform_api_bool_compare_vs_none` and its siblings | V2 (bool word by type); unif2 D1 for the `.not` forms (U2) | **no item yet** (the design says "schedule with vs16-2"; not in the intval-vs16 item's title) |
| V3 | vs16-3: M3 widths 2 and 4, whole-vector `==`/`!=`, `ior`/`ixor`/`not`, the `float_compare` vector forms | A | about 70 of the 87 named mediump vertex operator cases (first refusal; 86 = arith step 6's overlap, 8 of them in V1 -> 78) | the section-7 references of the design (not generated yet) | V1; the section-7 references | intval-vs16-3 |
| V4 | vs16 M4: mediump compare results into a colour; fp16 uniform operands (`vsapi_mat2`, `vsapi_struct`, `ck_lane_mediump`, `rnd_vs_mediump_kept`) | A | not counted by the design (4 DIFFERS references) | those 4 references | V3 | intval-vs16-3 ("3b: the rest of M3 and M4") if it fits, else **no item yet** |
| R1 | intval-rest: fragment int/bool shapes (**design running**: counts from families 18.3) | IV | 271 first refusals (`int_uniform` 101, `ivec_operator` 72, `random_mixed` 40, `bool_uniform` 31, `step_sign` 21, other 6); +146 `isok_int_round` (A/FO, arith X2) | the fs `deqp_intval_*` references | - (fs step 1 `ab9cde96` landed) | intval-rest (todo, design running) |

### 2.2 `gl_FragColor` lane sources (fclane design)

| Id | Design step | Text | Forecast cases (kind) | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| F1 | step 1, mechanism A: computed fp32 lane narrowed alone, `MKVEC`/`U16_TO_U32`, mirrored `(0.0, lane)` word, `FMA.v2f16` post-step | C | 406 (first refusal) | `op_pair_{yz,xyz,xyzw,x}`, `mat_cols_sum`, `mat_elem_pairs`, `mat4_swz_sum`, `mat_vec_lanes` (8 refs) | - | fclane-lower-1 |
| F2D | step 2, mechanism D: fp16 vector trees over converted pairs (`.hN` swizzles, `Leaf::Straddle`) | C | 138 (`tex_coord_ramp` 20, `vec3_local` 52, `rnd_vec4` 66: the last is not promised whole) | those 3 refs | F1 | fclane-lower-2 |
| F2B | step 2, mechanism B: two halves of different sources (`varying_merged`, `struct_fields`, `tex_zw_lane`) | C | 36 | those 3 refs | F1 | fclane-lower-2 |
| F2C | step 2, mechanism C: broadcast of a computed bool-float lane (`bcast_compare_*`, `bcast_const`) | C | 233, of which **94 read a struct uniform: no reference, not drawable** (`gl_ref.py`); 139 checkable | `bcast_compare_{int,bool,float,array}`, `bcast_const` | R1 and A1a/A1b (the mask lowerings) | fclane-lower-2 |
| F2E | step 2, mechanism E: `op_pair_other` (bvec2 round trips) | C | 8 | `op_pair_other` | R1, F1 | fclane-lower-2 |

### 2.3 Arithmetic built-ins (arith design; counts are arith section 3, `Moves`)

| Id | Design step | Text | Forecast cases (first refusal) | Refs | Needs | Owner today |
|---|---|---|---|---|---|---|
| A1a | 1a: fs `a / b` (`Frcp` of a lane, `mod`, `smoothstep`, matrix `/`) | A | 179 (107 operator suite + 54 matrix) | 12 | - | arith-lower-1 |
| A1b | 1b: fs fp32 transcendental kernels (`sin cos tan log log2 pow`, `exp2`) | A | 187 | 17 | - | arith-lower-1 |
| A2 | 2: vs fp32 built-ins, `vs_clamp` of two computed operands | A | 189 | 21 | A1b (`transc32`) | **no item yet** |
| A3 | 3: fs `floor ceil sign`, float `!=`, `sqrt`/`inversesqrt` random | A | 141 | 14 | - | **no item yet** |
| A4a | 4(a): pair trees, `min max clamp` and the add/sub/mul family | FO + A | 216 for step 4 as a whole (`FsOperandForm` 152 assumed to be all in step 4, the other 64 in A) | 26 (a+b) | - (widest risk: `mali_fs_arith.rs`, glmark2 corpus) | **no item yet** |
| A4b | 4(b): `mix reflect cross`, matrix `+`, `storeoutput` | FO + A | rest of step 4 | (in the 26) | A4a | **no item yet** |
| A5 | 5: fs `bcsel` of computed arms; `indexing.dynamic_select` 60 is the local-array mechanism | A | 96 | 6 | - | **no item yet** |
| A6 | 6: vs fp16 forms of mediump ops (`exp log tan ceil sign min max clamp`, `vs_reduce`) | A | 159, **of which 86 are V1+V3's** (see 4a): 73 are this step's | 20 | V2, V3 | **no item yet** |
| A7 | 7: composites `acos asin atan atan2 faceforward refract` (fs) | A | 48 | 12 | A1a, A3, A4 | **no item yet** |
| A8 | 8: `do_rebalance_tree` chains (`Rebalance`, a separate text, 12) | `Rebalance` | 12 | 1 | - | **no item yet** |
| X1, X2 | constant folds 75; integer/bool 325 (179 = V2b, 146 = R1) | A | 400 | - | not this family's | CF1 / V2b, R1 |

Items 1a and 1b are `arith-lower-1`: two commits in one session or two sessions (design 4).

### 2.4 Posvary (posvary design; 42 cases, HOST-ONLY)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| P1 | step 1: M3, a varying the vertex shader never writes (filter outputs with no store) | `VaryingForm`/`PositionLane` | 4 | `unwritten_{not_declared,declared_unused,declared_dead_read,negative_vs}` | - | posvary-lower |
| P2 | step 2: M2, `fract` of a local copy (`Undef` lane of `Ffract`) | `PositionLane` | 6 (up to 22 with 16 `FMA` cases of the same shader) | the six `fract_*` | - | posvary-lower |
| P3 | step 3: M1a (dead lanes leave the reads) + M1b (`pack_lanes`, scalar flat after a smooth vec4) | `VaryingNotFromAttribute` | 16 (part of 31) | `flat_sep_rnd13`, `flat_sep_kau_bs26` | - | posvary-lower |
| P4 | step 4: M1c (the fill, one record per run, `.v3` shrink) + M1d (`tex_uniform_w`) | same | 15 (rest of 31) | `flat_sep_vec4_bs51`, `flat_lane_vec4_v86`, `flat_lane_vec4_c30`, `tex_uniform_w{,_array}` | P3 | posvary-lower |
| P5 | step 5: M4, a loop's phi in a varying (`loop_phi_array`) | same | 1 (NotSupported) | `loop_phi_array` | LS-array/int-uniform loop (no posvary step owns it) | posvary-lower |

The design says the 29 + 2 cases of M1 "can move to another refusal after this" (random shaders most).

### 2.5 Uniform types and arrays (unif2 design; counts of `mali-deqp-unif2.md`)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| U1 | step 1: `gl_ref.py --uniform-values` for arrays and struct members, redraw | none | 0 (tools) | none | - | **claimed twice**: `fclane2-struct` (running, `gl_ref.py`) and `unif2-lower-1` (title "step 1") |
| U2 | step 2: the bool word (`fau.rs`, `select.rs`, `program.rs`, `frame.rs`; GL stores 1, Mesa `~0`) | none | 0 | none | must land with U3, before U6 | **no item yet** (see 4a: unif2-lower-1 should be U2+U3) |
| U3 | step 3: M1, int/bool/ivec/bvec array element rule, top-level arrays, both stages | UA, FU | 226 (`arr_bool_array` 95, `arr_int_array` 46, `fs_int_uniform` 52, `fs_bool_uniform` 33); families' texts: UA 137, FU 76 | the `arr_*`, `fs_int_uniform`, `fs_bool_uniform` refs (first as the prerequisites land) | U2; V2 (vertex), R1 and F1 (fragment compare and lane) | **no item yet** (unif2-lower-1 is titled "step 1" = U1; see 4a) |
| U4 | step 4: host forecast of M1 | none | 0 (a measure) | none | U3 | not a lowering |
| U5 | step 5: M3, struct uniforms (leaf rows, member arrays, nested) | UT, FU, UA | 191 (`vs_struct_uniform` 134, `fs_struct_uniform` 55, `arr_struct_array` 2) | `vs_struct_uniform`, `fs_struct_uniform`, `arr_struct_array` | U1, U3, V2 (vertex int members) | **no item yet** |
| U6 | step 6: M4, vs bvec/bool compare chain (`.not` first lane) | A | 67 (`vs_bool_uniform`) | `vs_bool_uniform` | U2, V2b (shared `vs_reduce`) | **no item yet** |
| U7 | step 7: M5, two uniform words as one fp16 pair (`fs_other`) | FU | 7 | `fs_other` | F1 | **no item yet** |
| U8 | step 8: M6, a mat3 array element, fp16 | UA | 1 | `arr_mat_array` | - | **no item yet** |
| U9 | step 9: M2 (`fs_mat_uniform` 119, `vs_mediump_struct` 53, `vs_mediump_array` 25 = 197): **check only**, the lane is fclane's | C (FU) | 197 owned by fclane | those 3 refs | F1 | fclane-lower-1/2 (no unif2 session) |

### 2.6 Mixprec and colourpair (mixcol design; 243 cases)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| MX0 | step 0: make the site a measured fact | none | 0 | none | - | fold into MX1 |
| MX1 | step 1: M4, the pair as an operand, the operator harness | `ColourPair` etc. | 18 | `ck_scale`, `ck_scale_highp`, `lz_scale` | - (shares `vectorise` with F1: serialise) | mixcol-lower-1 |
| MX2 | step 2: M4, random sums and products | `ColourPair`, `ColourZeroHalf` | 59 | `ck_sum`, `ck_rnd_other`, `lz_rnd`, `lz_rnd_other`, 6 of colourpair | MX1 | **no item yet** |
| MX3 | step 3: M3, a declared highp value in a mediump shader | MP | 64 (only 9 compile alone: 55 also need F1..F2C) | `hp_param`, `hp_seq`, `hp_loop` | F1, F2C | **no item yet** |
| MX4 | step 4: M1, an fp32 expression reading a raw mediump lane or uniform | MP | 68 | `int_uniform`, `f2i_seq` | - | **no item yet** |
| MX5 | step 5: M2, width judged on live nodes only | MP | 34 | `f2i_rnd`, `f2i_dead` | - | **no item yet** |

### 2.7 Constant products (constprod design; 533 cases = AC 386 + FMA 139 + `AlgebraicForm` 8)

| Id | Design step | Text | Forecast cases (first refusal) | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| K0 | S0: probe that makes the sites measured | none | 0 | none | - | fold into K1 |
| K1 | S1: negated literal first in a scalar product (`neg_products`, `Alu::Fmul`) | AC | 174 | `litfirst_vs`, `litfirst_array_vs` | constop merged (it is) | constprod-lower-1 |
| K2 | S2: product of two negated literals; literal times a product | AC | 124 | `neglit_pair_{vs,fs}`, `chainlit_{vs,fs}` | K1 | **no item yet** |
| K3 | S3: constant times a product with a constant; mixed reassociation | AC | 64 | `constchain_{degrees,attrib,loop}_vs`, `constchain_fs`, `reassoc_mixed_vs` | K2 | **no item yet** |
| K4 | S4: constant calls (`acos asin atan atan2 reflect refract mod`) take the `alu_sized` route | AC + FMA | 48 (assumed 24 AC + 24 FMA); **the cases whose shader then compiles**: the 48 `constant_expressions` library cases | 7 refs (`atan` may stay ignored) | cfold (value) disjoint | **no item yet** |
| K5 | S5: `acos/asin/atan/atan2` of a vertex value | FMA | 47 | `fma_trig_{acos,atan2}_vs`, `fma_random_trig_vs` | after A7 (shared `builtin_call.rs`: A7 fixes the chain order) | **no item yet** |
| K6 | S6: matrix product of attributes and constants | FMA | 41 | `fma_matmul_{const,dynamic,mat}_vs`, `fma_mulassign_vs` | K4 | **no item yet** |
| K7 | S7: scalar `reflect/refract/distance/fract` of attributes | FMA | 19 | `fma_geom_{reflect,refract}_vs`, `fma_invariance_vs` | K5 | **no item yet** |
| K8 | S8: `-2.0 * a` for a vector/matrix, the factoring's sum | FMA, `AlgebraicForm` | 16 (assumed 8 + 8) | `litfirst_mat_{vs,fs}`, `alg_constmat_fs` | K2 | **no item yet** |

The design's own forecast: the cases that **compile** after their step are K4's 48 and K5-K7's 107 (15 random);
K1-K3's cases (330 of 533, mostly random) only move to the next refusal.

### 2.8 Negzero (`mali-deqp-negzero.md` section 4; 137 cases; one item, three steps)

| Id | Step | Text | Forecast cases | Un-ignores | Needs | Owner today |
|---|---|---|---|---|---|---|
| N1 | `constglobal`: constant global initialisers fold in `glsl.rs` | NZ | 22 | `deqp_negzero_constglobal_reduced` first | K1 (same `glsl.rs` region, constprod 5); libm built-in fold (CF1) | negzero-lower |
| N2 | `mainbuiltin`: built-ins on constants inside `main` | NZ | 84 | the `mainbuiltin` refs | N1; CF1 | negzero-lower |
| N3 | `mainplain`: int/bool constant folding, copy propagation of a constant store | NZ | 31 | the `mainplain` refs | R1 | negzero-lower |

### 2.9 Loops (**ref note** `mali-deqp-loops.md` 9.5; `loops-design` running; 537 cases in eleven steps)

| Id | Step | Text | Forecast cases | Needs | Owner today |
|---|---|---|---|---|---|
| LS3 | fs kept loop, plain body, uniform or dynamic bound | LM | 128 (NotSupported) | R1 (int counters, `int(v_one + 0.5) * ui_n`) | loops-lower-1 |
| LS4 | fs loops Mesa deletes or unrolls (decision only, no new code) | LM | 22 | LS3 (shipped together in the note) | loops-lower-1 |
| LS1 | vs counted loops Mesa unrolls (break/continue first/last, nested, `return`) | LV | 87 (27 Fail; 42 equal references that compile) | - | **no item yet** (loops-lower-2 is "vertex loops": the design decides) |
| LS2 | array subscripts Mesa unrolls and deletes (`indexing.*dynamic_loop*`) | LM 50 + LV 44 | 94 | LS3 (fs part) | **no item yet** |
| LS5 | fs kept loop with `break`/`continue` (12 are `GuardMediump`) | LM, `GuardMediump` | 28 | LS3 | loops-lower-1 |
| LS6 | fs nested or sequenced kept loops | LM | 24 | LS3 | loops-lower-1 |
| LS7 | fs vector-subscript loops, `discard`/`return`/`struct` kept loops | LM | 44 | LS3 | loops-lower-1 |
| LS8 | vs kept loop, plain body (float counters, `LOOP_CAP`) | LV | 32 + 97 int ones that sit behind UT today (the note: "on top of S8-S10 when unitype lands") | V2 | loops-lower-2 |
| LS9 | vs nested/sequenced kept loops | LV | 24 | LS8 | loops-lower-2 |
| LS10 | vs kept loop with `break`/`continue`/`if` | LV | 36 | LS8 | loops-lower-2 |
| LS11 | vs vector-subscript loops | LV | 18 | LS8 | loops-lower-2 |

### 2.10 FMA/LOCALS/texture (ref note `mali-deqp-fmaloc.md` section 3; `fmaloc-design` in review) and texts with no design

| Id | Step | Text | Forecast cases | Needs | Owner today |
|---|---|---|---|---|---|
| T1 | TEX_COORD cube sequence (`tc_cube_face` 86, `tc_cube_const`, `tc_cube_rnd_sub`) and the scalar-and-constant 2D coordinate | TC | 104 (first refusal; partner shader compiles) | - | **no item yet** (fmaloc-design in review) |
| T2 | TEX_VERTEX projection 11, 2D bias 15, cube bias 1 | `TexVertex` | 27 | - | **no item yet** |
| (FMA) | the ref note's FMA rows (const fold 20, matrix 24, trig 17, reflect 2 = 63) | FMA | 63 | - | **covered by K4-K7: fmaloc must drop them** (4a) |
| CF1 | the rest of `ConstantFold` (`zero_operand` 98, `random_mixed` 60, `const_other` 23, `const_call_*` 23; `Fsin Fcos Flog2 Fexp2 Fpow` of constants) | CF | 204 (+ 75 of A: X1) | - | **no item yet** (cfold-ref in review, cfold-design todo) |
| CT1 | `ControlFlow` 73, `DiscardForm` 16 | `ControlFlow`, `DiscardForm` | 89 | - | **no item yet** (ctrlflow-ref running, ctrlflow-design todo) |
| DF1 | `Defect(Encode)` 6 (`attribute_location.bind_aliasing`), the library SIGSEGV 2 | `Defect` | 8 | - | **no item yet** (defects-ref running) |
| MISC | texts under 100: `TexKind` 42, `FsVaryingLoad` 43, `FragCoordMediump` 42, `VaryingDuplicate` 49, `ColourPair` (MX), `Invariant` 9, `LinkedVertexNotLowered` 8, `AlgebraicForm` 8 (K8), `AttributeNumbering` 8, `Rebalance` 12 (A8), `Spill` 2, `TooManyValues` 2, `FragmentTooLarge` 4, `Fp16Inexact` 3, `TexLanes` 1, `UniformIndexed` 10 | various | about 270 | - | **no item yet** (misc2-ref, texmisc-ref running) |

## 3. Recommended serial order

Rules, in the order the goal gives them: (1) integer and boolean first; (2) then `gl_FragColor` lane
sources; (3) then the rest by cases per session; (4) never before a prerequisite; (5) two steps that edit
the same function go in the order the designs name. A slot is one agent session (design sizes: "one
session" each; `A1a`+`A1b` and `A4a`+`A4b` are the designs' own two-session steps). A step whose
design is not merged (`R1`, `LS*`, `T*`, `CF1`) takes the slot but its place moves when the design
lands; **when the slot's item is not ready, take the next ready one** (`arith-lower-1`, `constprod-lower-1`,
`posvary-lower`, `loops-lower-1`, `negzero-lower`, `mixcol-lower-1` are ready today).

| Q | Step | First-refusal cases | Why here |
|--:|---|--:|---|
| 1 | IR1 | 187 | a defect on bool/int shapes; the best cases per session; touches `mod.rs` only |
| 2 | V2 | 445 (UT 346, UP 99) | the gate: loops, unif2 vertex halves and A6 need it; one short session |
| 3 | V1 | 8+ | first byte-equal win, proves M1; same item as Q2 |
| 4 | V2b | 179 | `vs_reduce` width two; design 10 says "with vs16-2" |
| 5 | R1 (1-3 sessions) | 271 (+146) | the fragment integer/boolean text; int counters for loops |
| 6 | F1 | 406 | largest single lane mechanism, no prerequisite |
| 7 | F2D | 138 | after F1 |
| 8 | F2B | 36 | after F1 (may share Q7) |
| 9 | A1a | 179 | F2C needs the mask lowerings |
| 10 | A1b | 187 | A2 needs `transc32` |
| 11 | F2C | 139 checkable (94 more with no reference) | after R1, A1a, A1b |
| 12 | F2E | 8 | after R1, F1 (may share Q11) |
| 13 | K1 | 174 | cheap `glsl.rs` rule; must precede N1 |
| 14 | A4a | about 110 | widest risk, so early (before the corpus pins grow) |
| 15 | A4b | about 106 | |
| 16 | A2 | 189 | after A1b |
| 17 | U2 + U3 | 226 | needs V2, R1, F1 |
| 18 | U5 | 191 | needs U1, U3 |
| 19 | A3 | 141 | |
| 20 | K2 | 124 | `glsl.rs`, after K1 |
| 21 | LS3 + LS4 | 150 (128 NS) | needs R1 (counters); loops-lower-1 |
| 22 | LS1 | 87 (27 Fail) | independent; Fail cases |
| 23 | LS2 | 94 | after LS3 |
| 24 | V3 (+V4) | about 78 | needs the vs16 section-7 references |
| 25 | CF1 | 204 (+75 A) | N1/N2 need its libm fold; no design yet |
| 26-28 | N1, N2, N3 | 22, 84, 31 | after K1, CF1, R1 |
| 29 | MX4 | 68 | takes MP under 100 |
| 30 | MX3 | 64 | 55 need F1..F2C |
| 31 | MX2 | 59 | |
| 32 | MX1 | 18 | rebases onto F1 (shared `vectorise`) |
| 33 | MX5 | 34 | |
| 34 | T1 | 104 | |
| 35 | A5 | 96 | |
| 36 | A6 | 73 | after V2, V3 |
| 37 | K3 | 64 | |
| 38 | K4 | 48 | |
| 39 | A7 | 48 | after A1a, A3, A4; fixes the chain order K5 reuses |
| 40 | K5 | 47 | |
| 41 | K6 | 41 | after K4 |
| 42 | K7 | 19 | |
| 43 | K8 | 16 | |
| 44 | A8 | 12 | |
| 45-51 | LS8, LS10, LS5, LS7, LS6, LS9, LS11 | 129 (32 + 97), 36, 28, 44, 24, 24, 18 | after R1, V2 |
| 52-54 | U6, U7+U8 | 67, 8 | |
| 55 | T2 | 27 | |
| 56-60 | P1..P5 | 4, 6, 16, 15, 1 | ready today but 42 cases in five sessions: last of the families |
| 61+ | CT1, DF1, MISC | 89, 8, about 270 | no design |

Counting the sessions with 1 per row (R1 as 2, ranges as written), the order is about 66 sessions.
Deliberate departures from "by count": A4 goes before A2 and A3 because it has the widest risk
(the pinned glmark2 corpus), not the most cases, so the later steps rebase onto it; K1 goes at Q13 because
N1 needs it; MX1 (18) is late because it rebases onto F1. Posvary is ready now but its yield per
session is the lowest of the families; take it whenever no earlier slot is ready.

## 4. Double claims, missing items, forecast

### 4a. Steps claimed by two designs or two items, and the owner to keep

| Step | Claimed by | Keep | Reason |
|---|---|---|---|
| vs mediump `==`/`!=`, `any`/`all`, `bany` (86 cases; arith 6) | `mali-intval-vs16-lowering.md` M3 and `mali-arith-lowering.md` step 6 | **intval-vs16 / intval-vs16-3** | arith section 10 and vs16 section 10 say the same cases seen from two sides; arith step 6 shrinks from 159 to 73 |
| 179 `uniform_api.bool_compare` vs (width-two bool reduction) | vs16 section 10 ("schedule with vs16-2"), arith X2, intval-rest | **intval-vs16** (as V2b; widen the item title) | the vertex `vs_reduce` arm; it is not fragment |
| `fs_int_uniform` 52 / `fs_bool_uniform` 33 (families' FU: 76) | unif2 step 3 (M1) and intval-rest | **unif2** for the uniform read/layout (U3); intval-rest for the compare/convert | unif2 section 4 assumes intval-rest's compare exists |
| unif2 step 1 (`gl_ref.py` array/struct values; same files) | `fclane2-struct` (running) and `unif2-lower-1` (titled "step 1") | **fclane2-struct** | it is running and owns `gl_ref.py`. Retitle unif2-lower-1 to U2+U3 (the bool word and the arrays); else it has nothing of its own to change |
| unif2 M2 (197 `fs_mat_uniform`, `vs_mediump_struct`, `vs_mediump_array`) | unif2 step 9 and fclane | **fclane-lower-1/2** | unif2 step 9 is "check only" |
| mixcol M4 steps 1-2 (77) and fclane step 1 (same `vectorise`/`lane_tree` emission) | mixcol-lower-1 and fclane-lower-1 | both, **fclane-lower-1 first** | whichever lands second rebases; the first's words are a subset |
| mixcol `hp_loop` (3) | mixcol step 3 and loops | **loops** | mixcol says "count them once, under whichever lands the highp loop counter" |
| ref note `mali-deqp-fmaloc.md` FMA rows (63) | fmaloc (in review) and constprod S4-S7 | **constprod** (families' FMA text is 139, all in K4-K8) | fmaloc-design should keep only T1 and T2 |
| constprod S4 route (48) and `ConstantFold` | constprod S4 and the cfold items | **constprod S4** (route); cfold owns the value (`Fsin Fcos Flog2 Fexp2 Fpow`) | constprod section 3 splits by the text a case reaches first |
| built-in folds in `glsl.rs` (negzero N1/N2 `tan exp log sin cos ...`) and cfold (`alu_sized`) | negzero-lower and the cfold items | **one libm-fold helper, first user writes it**: put CF1 before N1, N2 | Mesa folds with the host libm; two implementations would differ in bits |
| `glsl.rs` `neg_products` / `plus_zero` | constprod S1 and negzero-lower | K1 first, then N1 | constprod section 5 |
| `builtin_call.rs` chains | arith step 7 (fs) and constprod S5/S7 (vs) | different walks, **A7 first, K5 and K7 after** | whoever lands first fixes the `ffma` order |
| `vs_reduce` arms | vs16 M3 and arith 6 and unif2 step 6 | V3 writes them as arms before `_ => Err(Arithmetic)`; A6 and U6 rebase | vs16 section 10 |
| `IrIndex` 187 | irindex-fix and intval-rest (families 1a "to triage with intval-rest") | **irindex-fix** | a defect, not a lowering |
| posvary M2's 16 cases (`367:FMA` in the same vertex shader) | posvary P2 and constprod K5-K7 | P2 for the lane, K for the FMA | the 6 cases are P2's; the 16 follow K |
| loops `S2` 94 (`indexing.*dynamic_loop*`) | loops-lower-1 (fragment, 50) and loops-lower-2 (vertex, 44) | the loops design decides; one session of the array rule serves both | |

### 4b. Steps with no item yet (item titles, one-line scope)

1. **arith-lower-2**: vs fp32 built-ins and `vs_clamp` of two computed operands (A2, 189).
2. **arith-lower-3**: fs `floor ceil sign`, float `!=`, `sqrt`/`inversesqrt` random (A3, 141).
3. **arith-lower-4**: fs pair trees and `min max clamp`; two sessions, A4a then A4b (216).
4. **arith-lower-5**: fs `bcsel` of computed arms (A5, 96; the 60 dynamic selects are the local-array path).
5. **arith-lower-6**: vs fp16 forms of mediump ops after the vs16 sessions (A6, 73).
6. **arith-lower-7**: fs composites `acos asin atan atan2 faceforward refract` (A7, 48), and **arith-lower-8** rebalance chains (12).
7. **intval-vs-bool-reduce** (or widen intval-vs16): the width-two/four 32-bit bool reduction in `vs_reduce` (V2b, 179).
8. **fclane-lower-3**: mechanism C and E after intval-rest and arith-lower-1 (F2C 139 + 94 without references, F2E 8); today inside fclane-lower-2 with B and D, four sessions in one item.
9. **unif2-lower-1**, retitled: U2 + U3, the bool word and int/bool arrays (226); **unif2-lower-2**: U5 struct uniforms (191); **unif2-lower-3**: U6, U7, U8 (75).
10. **mixcol-lower-2..5**: MX2 random pairs (59), MX4 raw mediump lane/uniform (68), MX3 declared highp (64), MX5 live-node width (34).
11. **constprod-lower-2..8**: K2 (124), K3 (64), K4 (48), K5 (47), K6 (41), K7 (19), K8 (16).
12. **negzero-lower** is one item for three sessions (N1, N2, N3); fine, but N3 needs intval-rest.
13. **loops-lower-0** (vertex unrolled loops, LS1, 87) and **loops-lower-array** (array subscripts, LS2, 94): loops-lower-1 is fragment and loops-lower-2 is "vertex loops (LoopVertex)"; these two shapes are neither (design `loops-design` decides).
14. **fmaloc-lower-1**: TEX_COORD cube sequences (T1, 104); **fmaloc-lower-2**: TEX_VERTEX (T2, 27).
15. **cfold-lower-1**: the remaining `ConstantFold` (CF1, 204) and the 75 constant-fold first refusals of A (X1), with the libm helper N1/N2 reuse.
16. **ctrlflow-lower-1**: `ControlFlow` 73 + `DiscardForm` 16; **defects-fix**: `Defect(Encode)` 6 and the library SIGSEGV 2; **misc-lower**: the small texts of `MISC`.
17. **intval-vs16-4** only if V4 (M4) does not fit intval-vs16-3.

### 4c. Per-text forecast against the 100-case line (criterion c2)

Method: baseline = families 18.2; each step removes its own cases from the text it is counted
under, **no inflow** from cases that arrive from other texts. Inflow is real (a gate lifts cases
into the next text): it is added only where a design quantifies it (LV +97 after V2). So these
are the **earliest** slots a text can fall under 100, and the board rerun decides. Steps with an
assumed split say so below the table.

Sixteen texts are at or above 100 now; UP (99), ControlFlow (73) and the rest are already under.

| Text | Now | Trajectory (Q = slot of section 3) | First under 100 |
|---|--:|---|---|
| IR | 187 | Q1 0 | **Q1** |
| IV | 271 | Q5 0 (R1; design pending) | **Q5** |
| UT | 462 | Q2 116; Q18 (U5) 0 | **Q18** (116 after Q2 is the vs struct shapes) |
| UP | 99 | Q2 0 | already under; 0 at Q2 |
| C | 821 | Q6 415; Q7 277; Q8 241; Q11 102; Q12 94 | **Q12** (94 left = the structType shapes with no reference: `fclane2-struct` must give references or C stays at 94, 6 under the line) |
| FU | 280 | Q6 201 (fs_mat 79 lifted by the lane, unif2's reading); Q17 125; Q18 50; Q52-54 43 | **Q18** (if `fs_mat_uniform` does not leave with the lane, FU stays at 129 and has no step: missing item) |
| UA | 193 | Q17 56 (137 of `bool_array`+`int_array`); Q18 22 (`array_in_struct` 34 assumed); U8 −1 | **Q17** |
| AC | 386 | Q13 212; Q20 88; Q37 24; Q38 0 | **Q20** |
| FO | 152 | Q14-15 0 (A4 assumed to take all) | **Q15**; Q14 alone only if A4a takes 53 or more |
| A | 1463 | Q3 1455; Q4 1276; Q5 1130; Q9 951; Q10 764; Q14-15 700; Q16 511; Q19 370; Q24 292; Q25 217; Q35 121; Q36 48; Q39 0 | **Q36** (the last of the large texts: A5 and A6 are needed) |
| FMA | 139 | Q38 115; Q40 68; Q41 27; Q42 8; Q43 0 | **Q40** |
| NZ | 137 | Q26 115; Q27 31; Q28 0 | **Q27** |
| LM | 281 | Q21 131; Q23 81; Q45-51 (LS5-7) 0 | **Q23** |
| LV | 241 | Q2 338 (+97 hidden int loops); Q22 251; Q23 207; Q45 (LS8, 32 + 97) 78; Q46 42; Q49-51 0 | **Q45** |
| CF | 204 | Q25 0 | **Q25** (design pending) |
| TC | 182 | Q34 78 (T1's 104; the 78 other cases are not in fmaloc's count: assumption) | **Q34** |
| MP | 166 | Q29 98; Q30 34; Q33 0 | **Q29** |

Assumptions behind the table: step 4's 216 = FO 152 + A 64; constprod S4 = 24 AC + 24 FMA and S8 = 8 FMA +
8 `AlgebraicForm`; V2 leaves `vs_struct_uniform` 116 in UT; V2b (179) and R1's isok 146 come out of A; the whole
arith step 6 is 86 (V1 8 + V3 78) + 73. The LS counts are one reading of a ref note: LM 281 + `GuardMediump`
12 + `LoopForm` 3 = 296 = LS3 + LS4 + LS2 (fs 50) + LS5 + LS6 + LS7.

**c2 (every text under 100) is reached at Q45** (LS8, the last text over the line), after about 50 sessions,
provided inflow does not push another text over (`ControlFlow` 73, `ColourPair` 61, `TexKind` 42 and `FsVaryingLoad`
43 are the ones to watch: the gates V2, R1 and F1 send cases into them). The first board rerun
(`board-rerun-1`) should come after Q12 (intval and all of fclane: C 94, IV 0, IR 0, UT 116): by
then IR, IV and C are under the line (UP and the UT gate too) and the table is re-ranked from
measured counts; a second rerun after Q36 (A) and one after Q45 close c2.
