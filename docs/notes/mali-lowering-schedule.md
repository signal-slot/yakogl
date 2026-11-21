<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): one schedule of every lowering step, with case forecasts (plan #387, items lower-schedule, schedule-refresh)

**Status: planning note, no code.** Plan #387 has many lowering items that all edit
`src/shaderc/online/mali/**` and so run one at a time. This note puts the steps of every
merged lowering design into one table (section 2), proposes one serial order (section 3), and
lists the steps two designs claim (4a), the steps with no item (4b) and the per-text
forecast against the 100-case line of criterion c2 (4c). It changes nothing under `src/`,
`tests/` or `tools/`, and edits none of the design notes.

Item owners refreshed at the integration tip (`c0142485`): since `lower-schedule` merged at
`f0a8d2af` the plan added the lowering items `fclane-lower-3`, `arith-lower-2/3/4a/4b/5/6`,
`unif2-lower-2`, `constprod-lower-2/3`, `mixcol-lower-2`, `ctrlflow-lower-1`,
`fmaloc-lower-1` and the reference/design items `intval4-ref`, `mali-ref-struct`,
`unif2-struct-redraw`, `libm-fold-design`, `loops3-ref`, `audit-ref`, `texmisc-lower-design`
and the per-design `*-sites` items; the loop, fmaloc and intval-rest designs merged; and the
item `posvary-lower` was repurposed (it now draws the vs16 section-7 references). Case counts
are unchanged from what `lower-schedule` wrote (the designs' own numbers; section 4c's text
counts from `mali-deqp-families.md` 18.2 at `9ac31f30`); a remeasure is `remeasure-tip`'s
item, not this note's refresh.

## 0. Sources, and what the numbers are

Read: `mali-intval-vs16-lowering.md`, `mali-arith-lowering.md`, `mali-fclane-lowering.md`,
`mali-posvary-lowering.md`, `mali-unif2-lowering.md`, `mali-mixcol-lowering.md`,
`mali-constprod-lowering.md`, `mali-loops-lowering.md`, `mali-fmaloc-lowering.md`,
`mali-intval-rest-lowering.md` (all merged designs), `mali-deqp-families.md` section 18 (the
ranked table), and, where **no `*-lowering` design is merged yet**, the reference notes
`mali-deqp-cfold2.md` section 4 (its steps A, C, D, B), `mali-deqp-ctrlflow.md` section 5
(its steps 1-6), `mali-deqp-negzero.md` section 4, `mali-deqp-intval.md`,
`mali-deqp-texmisc.md` section 4, `mali-deqp-misc2.md` section 4,
`mali-deqp-defects.md` and `mali-deqp-compiled-audit.md` (fix candidates D1, V1, F1, S1).
No `mali-cfold-lowering.md` or `mali-ctrlflow-lowering.md` exists: CF1's and CT1's step split
are the reference notes' own, and a design that lands may move it. `mali-deqp-loops.md` and
`mali-deqp-fmaloc.md` stay read for the case tables; their step splits were superseded by the
merged designs. Rows built on a reference note are marked **(ref note)**: their step split is
that note's, and the design that lands may move it.

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

`IR1` irindex-fix; `V1..V4` intval vs16 sessions; `R1` intval-rest steps 1-4 (the design's step 0 is its
step 1 session's first hour, references by `intval4-ref`); `F1`, `F2B`, `F2D`, `F2C`, `F2E` fclane (the design's
two steps; the plan split its mechanism C and E out as `fclane-lower-3`); `A1a..A8` arith; `P1..P5` posvary (no
item today: the plan item `posvary-lower` was repurposed to draw the vs16 section-7 references);
`U1..U9` unif2 (the design's step numbers; it labels its step 1 `U0` and step 2 `D1`); `MX0..MX5` mixcol (its step
numbers); `K0..K8` constprod (its S0..S8); `N1..N3` negzero; `A0`, `LS1..LS11` loops (`LSn` = `mali-loops-lowering.md`'s S1..S11,
formerly the ref note's); `T1`, `T2` fmaloc (`T1` = its ranked steps 1, 4, 7; `T2` is not in the fmaloc design);
`CF1`'s parts `CF-A`, `CF-C`, `CF-D`, `CF-B` (`mali-deqp-cfold2.md` 4); `CT1`'s steps 1-6 (`mali-deqp-ctrlflow.md` 5);
`DF1` and `MISC` the texts whose fixes have no lowering item.

## 2. Every step of every design

Item column: the plan #387 item id that owns the step (done / in review / running / ready /
todo), or **no item**. "Needs" lists prerequisite steps (section 3 turns them into the order).
Each design's first action now has a `*-sites` item (running or ready: `vs16-sites`,
`arith-sites`, `constprod-sites`, `fclane-sites`, `loops-sites`, `mixcol-sites`, `unif2-sites`,
`posvary-sites`, `fmaloc-sites`, `misc2-sites`, `ctrlflow-sites`) that measures each pinned
reference's refusal site at the tip and maps it to the step; it is named once per family.

### 2.1 Integer and boolean (intval, irindex)

| Id | Design step | Text(s) | Forecast cases (kind) | Un-ignores / refs | Needs | Item |
|---|---|---|---|---|---|---|
| IR1 | `Defect(IrIndex)` (families 18.4; `FsPrecision::of` re-parse of a bvec/bool source, unconfirmed) | IR | 187 (first refusal; 185 direct + 2 link errors; groups swizzles 108, conversions 45) | none known (`mali-deqp-irindex.md` landed: irindex-ref done) | - | irindex-fix (ready) |
| V2 | vs16 M5 + M6: the two gates (`UniformPrecision` for a mediump int, `UniformType` for an int uniform) and the `vs_is_uniform_word` bool-by-type fix | UT, UP | UT 346 (`vs_int_uniform`) + UP 99 first refusals; **a gate, not a pass count** (the cases move to loops/indexing/random). `vs_struct_uniform` 116 is UT too; whether M6 takes it is open (4c assumes it does not) | none; moves pinned refusals in `mali_deqp_loops.rs`/`mali_deqp_uniarray.rs` | - | intval-vs16 (vs16-2; ready) |
| V1 | vs16-1: M1 narrow mediump operands, M2 `fmin`/`fmax` rule and 16-bit mask, M3 arm for scalar and `bvec3` | A (board: IntValue) | 8 + a few (first refusal) | `opvs_logical_and` DIFFERS->LOWERED, `opvs_not_equal_bvec3` REFUSED->LOWERED | - (V2 independent) | intval-vs16 (vs16-1; ready) |
| V2b | vs16 section 10: the width-two (four after R9) 32-bit bool reduction `LSHIFT_OR/AND.i32 ... .not` for bool uniform words | A (`vs_reduce`) | 179 (`uniform_api.bool_compare` vs; first refusal) | `deqp_arith_uniform_api_bool_compare_vs_none` and its siblings (R9, the width-four bool uniform shape, is being drawn by `posvary-lower`'s new deliverable, in review) | V2 (bool word by type); unif2 D1 for the `.not` forms (U2) | intval-vs16 (design 10: the vs16-2 session adds this arm; its item title does not name it yet - 4a) |
| V3 | vs16-3: M3 widths 2 and 4, whole-vector `==`/`!=`, `ior`/`ixor`/`not`, the `float_compare` vector forms | A | about 70 of the 87 named mediump vertex operator cases (first refusal; 86 = arith step 6's overlap, 8 of them in V1 -> 78) | the section-7 references R1-R10: being drawn by `posvary-lower` (in review: `deqp_vs16ref_*`, `tests/mali_deqp_vs16ref.rs`) | V1; the section-7 references | intval-vs16-3 (todo) |
| V4 | vs16 M4: mediump compare results into a colour; fp16 uniform operands (`vsapi_mat2`, `vsapi_struct`, `ck_lane_mediump`, `rnd_vs_mediump_kept`) | A | not counted by the design (4 DIFFERS references) | those 4 references | V3 | intval-vs16-3 ("3b: the rest of M3 and M4"; vs16 6 keeps a vs16-4 only if it does not fit) |
| R1 | intval-rest steps 1-4: fragment int/bool shapes (design merged: `mali-intval-rest-lowering.md`; counts from families 18.3) | IV | 271 first refusals (`int_uniform` 101, `ivec_operator` 72, `random_mixed` 40, `bool_uniform` 31, `step_sign` 21, other 6); +146 `isok_int_round` (A/FO, arith X2) | the fs `deqp_intval_*` references; the 22 `deqp_intrest_*` pairs of the design's section 7 (`intval4-ref`, in review) | - (fs step 1 `ab9cde96` landed); step 3 needs the U1 redraw, U2 (unif2 D1) and fclane C, and shares the array layout work with U3 | intval-rest (step 1 only, todo); steps 2-4 **no item** (4b) |

### 2.2 `gl_FragColor` lane sources (fclane design)

| Id | Design step | Text | Forecast cases (kind) | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| F1 | step 1, mechanism A: computed fp32 lane narrowed alone, `MKVEC`/`U16_TO_U32`, mirrored `(0.0, lane)` word, `FMA.v2f16` post-step | C | 406 (first refusal) | `op_pair_{yz,xyz,xyzw,x}`, `mat_cols_sum`, `mat_elem_pairs`, `mat4_swz_sum`, `mat_vec_lanes` (8 refs) | - (`fclane-sites` measures each reference's site first) | fclane-lower-1 (todo) |
| F2D | step 2, mechanism D: fp16 vector trees over converted pairs (`.hN` swizzles, `Leaf::Straddle`) | C | 138 (`tex_coord_ramp` 20, `vec3_local` 52, `rnd_vec4` 66: the last is not promised whole) | those 3 refs | F1 | fclane-lower-2 (todo) |
| F2B | step 2, mechanism B: two halves of different sources (`varying_merged`, `struct_fields`, `tex_zw_lane`) | C | 36 | those 3 refs | F1 | fclane-lower-2 (todo) |
| F2C | step 2, mechanism C: broadcast of a computed bool-float lane (`bcast_compare_*`, `bcast_const`) | C | 233, of which 94 read a struct uniform (the references are now drawable: `gl_ref.py` struct members landed with `fclane2-struct`, and `mali-ref-struct` (running) computes struct-member values in `tests/mali_ref`); 139 checkable | `bcast_compare_{int,bool,float,array}`, `bcast_const` | R1 steps 1-2 and A1a/A1b (the mask lowerings) | fclane-lower-3 (todo; the plan split mechanism C out of fclane-lower-2) |
| F2E | step 2, mechanism E: `op_pair_other` (bvec2 round trips) | C | 8 | `op_pair_other` | R1 steps 1-2, F1 | fclane-lower-3 (todo) |

### 2.3 Arithmetic built-ins (arith design; counts are arith section 3, `Moves`)

| Id | Design step | Text | Forecast cases (first refusal) | Refs | Needs | Item |
|---|---|---|---|---|---|---|
| A1a | 1a: fs `a / b` (`Frcp` of a lane, `mod`, `smoothstep`, matrix `/`) | A | 179 (107 operator suite + 54 matrix) | 12 | - | arith-lower-1 (ready) |
| A1b | 1b: fs fp32 transcendental kernels (`sin cos tan log log2 pow`, `exp2`) | A | 187 | 17 | - | arith-lower-1 (ready) |
| A2 | 2: vs fp32 built-ins, `vs_clamp` of two computed operands | A | 189 | 21 | A1b (`transc32`) | arith-lower-2 (todo) |
| A3 | 3: fs `floor ceil sign`, float `!=`, `sqrt`/`inversesqrt` random | A | 141 | 14 | - (its float `!=` is also the audit cause F1: `ctrlflow-design` + `audit-ref` land the fp32 two-varying compare) | arith-lower-3 (todo) |
| A4a | 4(a): pair trees, `min max clamp` and the add/sub/mul family | FO + A | 216 for step 4 as a whole (`FsOperandForm` 152 assumed to be all in step 4, the other 64 in A) | 26 (a+b) | - (widest risk: `mali_fs_arith.rs`, glmark2 corpus) | arith-lower-4a (todo) |
| A4b | 4(b): `mix reflect cross`, matrix `+`, `storeoutput` | FO + A | rest of step 4 | (in the 26) | A4a | arith-lower-4b (todo) |
| A5 | 5: fs `bcsel` of computed arms; `indexing.dynamic_select` 60 is the local-array mechanism | A | 96 | 6 | - | arith-lower-5 (todo) |
| A6 | 6: vs fp16 forms of mediump ops (`exp log tan ceil sign min max clamp`, `vs_reduce`) | A | 159, **of which 86 are V1+V3's** (see 4a): 73 are this step's | 20 | V2, V3 | arith-lower-6 (todo) |
| A7 | 7: composites `acos asin atan atan2 faceforward refract` (fs) | A | 48 | 12 | A1a, A3, A4 | **no item** (4b) |
| A8 | 8: `do_rebalance_tree` chains (`Rebalance`, a separate text, 12) | `Rebalance` | 12 | 1 | - (`mali-deqp-misc2.md` 4 step 4 gives this step the references its own one does not cover) | **no item** (4b) |
| X1, X2 | constant folds 75; integer/bool 325 (179 = V2b, 146 = R1) | A | 400 | - | not this family's | X1 -> CF1 (`cfold-design` for CF-A; **no item** for CF-C/D/B); X2 -> V2b (intval-vs16), R1 |

Items 1a and 1b are `arith-lower-1`: two commits in one session or two sessions (design 4).
Design 6's first action - printing each pinned reference's refusal site at the tip - is the
item `arith-sites` (running), and the arith steps start from its mapping.

### 2.4 Posvary (posvary design; 42 cases, HOST-ONLY)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| P1 | step 1: M3, a varying the vertex shader never writes (filter outputs with no store) | `VaryingForm`/`PositionLane` | 4 | `unwritten_{not_declared,declared_unused,declared_dead_read,negative_vs}` | - | **no item** (4b; the plan id `posvary-lower` was repurposed to the vs16 section-7 references, in review) |
| P2 | step 2: M2, `fract` of a local copy (`Undef` lane of `Ffract`) | `PositionLane` | 6 (up to 22 with 16 `FMA` cases of the same shader) | the six `fract_*` | - | **no item** (4b) |
| P3 | step 3: M1a (dead lanes leave the reads) + M1b (`pack_lanes`, scalar flat after a smooth vec4) | `VaryingNotFromAttribute` | 16 (part of 31) | `flat_sep_rnd13`, `flat_sep_kau_bs26` | - | **no item** (4b) |
| P4 | step 4: M1c (the fill, one record per run, `.v3` shrink) + M1d (`tex_uniform_w`) | same | 15 (rest of 31) | `flat_sep_vec4_bs51`, `flat_lane_vec4_v86`, `flat_lane_vec4_c30`, `tex_uniform_w{,_array}` | P3 | **no item** (4b) |
| P5 | step 5: M4, a loop's phi in a varying (`loop_phi_array`) | same | 1 (NotSupported) | `loop_phi_array` | LS-array/int-uniform loop (no posvary step owns it) | **no item** (4b) |

(`posvary-sites`, running, measures each posvary/varylink2 reference's site at the tip first.)

The design says the 29 + 2 cases of M1 "can move to another refusal after this" (random shaders most).

### 2.5 Uniform types and arrays (unif2 design; counts of `mali-deqp-unif2.md`)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| U1 | step 1 (`U0`): `gl_ref.py --uniform-values` for arrays and struct members, redraw | none | 0 (tools) | none | - | done as `fclane2-struct` (`gl_ref.py`); the redraw is `unif2-struct-redraw` (running). `unif2-lower-1` is titled "step 1" = this step, which it can no longer change: retitle it to steps 2+3 (4a) |
| U2 | step 2 (`D1`): the bool word (`fau.rs`, `select.rs`, `program.rs`, `frame.rs`; GL stores 1, Mesa `~0`) | none | 0 | none | must land with U3, before U6 | unif2-lower-1 (ready, after the 4a retitle) |
| U3 | step 3: M1, int/bool/ivec/bvec array element rule, top-level arrays, both stages | UA, FU | 226 (`arr_bool_array` 95, `arr_int_array` 46, `fs_int_uniform` 52, `fs_bool_uniform` 33); families' texts: UA 137, FU 76 | the `arr_*`, `fs_int_uniform`, `fs_bool_uniform` refs (first as the prerequisites land) | U2; V2 (vertex), R1 and F1 (fragment compare and lane) | unif2-lower-1 (ready, after the 4a retitle) |
| U4 | step 4: host forecast of M1 | none | 0 (a measure) | none | U3 | **no item** (a measure, not a lowering; `remeasure-tip` and the board reruns are the other measures) |
| U5 | step 5: M3, struct uniforms (leaf rows, member arrays, nested) | UT, FU, UA | 191 (`vs_struct_uniform` 134, `fs_struct_uniform` 55, `arr_struct_array` 2) | `vs_struct_uniform`, `fs_struct_uniform`, `arr_struct_array` (redrawn by `unif2-struct-redraw`, running) | U1, U3, V2 (vertex int members) | unif2-lower-2 (todo) |
| U6 | step 6: M4, vs bvec/bool compare chain (`.not` first lane) | A | 67 (`vs_bool_uniform`) | `vs_bool_uniform` | U2, V2b (shared `vs_reduce`) | **no item** (4b) |
| U7 | step 7: M5, two uniform words as one fp16 pair (`fs_other`) | FU | 7 | `fs_other` | F1 | **no item** (4b) |
| U8 | step 8: M6, a mat3 array element, fp16 | UA | 1 | `arr_mat_array` | - | **no item** (4b) |
| U9 | step 9 (`M2` check): `fs_mat_uniform` 119, `vs_mediump_struct` 53, `vs_mediump_array` 25 = 197): **check only**, the lane is fclane's | C (FU) | 197 owned by fclane | those 3 refs | F1 | fclane-lower-1/2 (todo; no unif2 session) |

(`unif2-sites`, running, measures each unif2/uniarray reference's site at the tip first.)

### 2.6 Mixprec and colourpair (mixcol design; 243 cases)

| Id | Design step | Text | Forecast cases | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| MX0 | step 0: make the site a measured fact | none | 0 | none | - | `mixcol-sites` (running; it also maps each mixcol reference to its step, as design 5 orders steps 1, 2, 4, 3, 5) |
| MX1 | step 1: M4, the pair as an operand, the operator harness | `ColourPair` etc. | 18 | `ck_scale`, `ck_scale_highp`, `lz_scale` | - (shares `vectorise` with F1: serialise) | mixcol-lower-1 (ready) |
| MX2 | step 2: M4, random sums and products | `ColourPair`, `ColourZeroHalf` | 59 | `ck_sum`, `ck_rnd_other`, `lz_rnd`, `lz_rnd_other`, 6 of colourpair | MX1 | **no item** (4b) |
| MX3 | step 3: M3, a declared highp value in a mediump shader | MP | 64 (only 9 compile alone: 55 also need F1..F2C) | `hp_param`, `hp_seq`, `hp_loop` | F1, F2C | **no item** (4b) |
| MX4 | step 4: M1, an fp32 expression reading a raw mediump lane or uniform | MP | 68 | `int_uniform`, `f2i_seq` | - | mixcol-lower-2 (todo) |
| MX5 | step 5: M2, width judged on live nodes only | MP | 34 | `f2i_rnd`, `f2i_dead` | - | **no item** (4b) |

### 2.7 Constant products (constprod design; 533 cases = AC 386 + FMA 139 + `AlgebraicForm` 8)

| Id | Design step | Text | Forecast cases (first refusal) | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| K0 | S0: probe that makes the sites measured | none | 0 | none | - | constprod-sites (running; the design's 7 "what this design does not know" is exactly its open site-measure, over the constprod **and fmaloc** references) |
| K1 | S1: negated literal first in a scalar product (`neg_products`, `Alu::Fmul`) | AC | 174 | `litfirst_vs`, `litfirst_array_vs` | constop merged (it is) | constprod-lower-1 (ready) |
| K2 | S2: product of two negated literals; literal times a product | AC | 124 | `neglit_pair_{vs,fs}`, `chainlit_{vs,fs}` | K1 | constprod-lower-2 (todo) |
| K3 | S3: constant times a product with a constant; mixed reassociation | AC | 64 | `constchain_{degrees,attrib,loop}_vs`, `constchain_fs`, `reassoc_mixed_vs` | K2 | **no item** (4b) |
| K4 | S4: constant calls (`acos asin atan atan2 reflect refract mod`) take the `alu_sized` route | AC + FMA | 48 (assumed 24 AC + 24 FMA); **the cases whose shader then compiles**: the 48 `constant_expressions` library cases | 7 refs (`atan` may stay ignored) | the value fold is the one `libm-fold-design` (running) designs (shared with CF-B and N1/N2) | constprod-lower-3 (todo, K4+K5 in one item) |
| K5 | S5: `acos/asin/atan/atan2` of a vertex value | FMA | 47 | `fma_trig_{acos,atan2}_vs`, `fma_random_trig_vs` | after A7 (shared `builtin_call.rs`: A7 fixes the chain order; A7 has **no item**) | constprod-lower-3 (todo) |
| K6 | S6: matrix product of attributes and constants | FMA | 41 | `fma_matmul_{const,dynamic,mat}_vs`, `fma_mulassign_vs` | K4 | **no item** (4b; `mali-fmaloc-lowering.md` step 2 and `mali-deqp-texmisc.md`'s cube-bias steps read the same arms - 4a) |
| K7 | S7: scalar `reflect/refract/distance/fract` of attributes | FMA | 19 | `fma_geom_{reflect,refract}_vs`, `fma_invariance_vs` | K5 | **no item** (4b) |
| K8 | S8: `-2.0 * a` for a vector/matrix, the factoring's sum | FMA, `AlgebraicForm` | 16 (assumed 8 + 8) | `litfirst_mat_{vs,fs}`, `alg_constmat_fs` (`mali-deqp-misc2.md` 4 step 5 pairs the 2 random `AlgebraicForm` cases with it) | K2 | **no item** (4b) |

The design's own forecast: the cases that **compile** after their step are K4's 48 and K5-K7's 107 (15 random);
K1-K3's cases (330 of 533, mostly random) only move to the next refusal.

### 2.8 Negzero (`mali-deqp-negzero.md` section 4; 137 cases; the item covers N1 and N2)

| Id | Step | Text | Forecast cases | Un-ignores | Needs | Item |
|---|---|---|---|---|---|---|
| N1 | `constglobal`: constant global initialisers fold in `glsl.rs` | NZ | 22 | `deqp_negzero_constglobal_reduced` first (the item's done_when names it) | K1 (same `glsl.rs` region, constprod 5); libm built-in fold (`libm-fold-design`, running) | negzero-lower (ready) |
| N2 | `mainbuiltin`: built-ins on constants inside `main` | NZ | 84 | the `mainbuiltin` refs | N1; CF-B (the fold helper `libm-fold-design` designs) | negzero-lower (ready) |
| N3 | `mainplain`: int/bool constant folding, copy propagation of a constant store | NZ | 31 | the `mainplain` refs | R1 | **no item** (the `negzero-lower` title - "fold global initialisers and built-ins on constants" - covers N1/N2 only; 4b) |

### 2.9 Loops (merged design `mali-loops-lowering.md` 4-5; 537 cases in eleven steps plus the decision step `A0`)

| Id | Step | Text | Forecast cases | Needs | Item |
|---|---|---|---|---|---|
| A0 | `loop_plan.rs`: one `Delete/Unroll/Partial/Keep` decision for both walks (no case moves on its own; its highp 40-pass constant decision needs a reference `loops3-ref` draws, not a "compiles" pin) | - | 0 | - | loops-lower-1 (ready; design order A0, S4, S3, S5, S6, S2-fs, S7) |
| LS3 | fs kept loop, plain body, uniform or dynamic bound (= S3) | LM | 128 (NotSupported) | R1 (int counters, `int(v_one + 0.5) * ui_n`) | loops-lower-1 (ready) |
| LS4 | fs loops Mesa deletes or unrolls (decision only, no new code) (= S4) | LM | 22 | LS3 (the design ships A0+S4+S3 first) | loops-lower-1 (ready) |
| LS1 | vs counted loops Mesa unrolls (break/continue first/last, nested, `return`) (= S1) | LV | 87 (27 Fail; 42 equal references that compile) | A0, the phi pairing (design 5) | loops-lower-2 (todo; the merged design assigns S1 to it) |
| LS2 | array subscripts Mesa unrolls and deletes (`indexing.*dynamic_loop*`) (= S2) | LM 50 + LV 44 | 94 | LS3 (fs part); S2-vs reuses S2-fs's `Partial` rule | loops-lower-1 (S2-fs, ready) / loops-lower-2 (S2-vs, todo) |
| LS5 | fs kept loop with `break`/`continue` (12 are `GuardMediump`) (= S5) | LM, `GuardMediump` | 28 | LS3 | loops-lower-1 (ready) |
| LS6 | fs nested or sequenced kept loops (= S6) | LM | 24 | LS3 | loops-lower-1 (ready) |
| LS7 | fs vector-subscript loops, `discard`/`return`/`struct` kept loops (= S7) | LM | 44 | LS3 | loops-lower-1 (ready) |
| LS8 | vs kept loop, plain body (float counters, `LOOP_CAP`) (= S8) | LV | 32 + 97 int ones that sit behind UT today (the note: "on top of S8-S10 when unitype lands") | V2; A0 and the mask forms from lower-1; the `vs_fn_after_seq`/`vs_fn_mixed` references of `loops3-ref` | loops-lower-2 (todo) |
| LS9 | vs nested/sequenced kept loops (= S9) | LV | 24 | LS8 | loops-lower-2 (todo) |
| LS10 | vs kept loop with `break`/`continue`/`if` (= S10) | LV | 36 | LS8 | loops-lower-2 (todo) |
| LS11 | vs vector-subscript loops (= S11) | LV | 18 | LS8 | loops-lower-2 (todo) |

The merged design assigns the two items: **loops-lower-1** = A0, S4, S3, S5, S6, S2-fs, S7
(296 cases); **loops-lower-2** = S1, S8, S9, S10, S11, S2-vs (241), and it "needs lower-1's
A0, the phi pairing and S5's mask forms, so it follows lower-1's S3 and S5" (design 6). The
`loops-lower-1` item title says "top step": one item, several one-commit steps. Three shapes
are outside both items (design 8, **no item** - 4b): the `IndexedLocal` follow-up of S2/S7/S11
(local arrays at a run-time index, 156 cases whose next refusal is unmeasured), the
thread-local scratch-array shape `fs_idx_tmp_vec4_cd` (4), and S11's second dead-position
loop if it appears. `loops-sites` (running) maps each loops/loops2 reference to its step;
`loops3-ref` (running) draws the A0 and S8 references.

### 2.10 FMA/LOCALS/texture (merged design `mali-fmaloc-lowering.md` 4; TexKind/TexVertex/FragCoordMediump: the ref note `mali-deqp-texmisc.md`, whose design item `texmisc-lower-design` is running) and texts with no lowering item

| Id | Step | Text | Forecast cases | Needs | Item |
|---|---|---|---|---|---|
| T1 | TEX_COORD cube sequence (`tc_cube_face` 86, `tc_cube_const`, `tc_cube_rnd_sub`) and the scalar-and-constant 2D coordinate (design steps 1, 4, 7) | TC | 104 (first refusal; partner shader compiles) | - (`fmaloc-sites` measuring; the design's step 4 wants the `uniform_api` recount before ranking) | fmaloc-lower-1 (ready; its title: "the top step of mali-fmaloc-lowering.md not covered by plan 367's lower-fma") |
| T2 | TEX_VERTEX projection 11, 2D bias 15, cube bias 1 (the ref note's split; `mali-deqp-texmisc.md` recounts TexVertex at 60 at the tip and puts projection/bias/cube into its steps 3-5, not in the fmaloc design) | `TexVertex` | 27 | - | **no item** (4b; the design is `texmisc-lower-design`, running) |
| (FMA) | the ref note's FMA rows (const fold 20, matrix 24, trig 17, reflect 2 = 63) | FMA | 63 | - | plan 367's `lower-fma` first (`fmaloc-lower-1` defers to it); the merged fmaloc design kept its own steps 2/3/5/6 for exactly these shapes, and the same fold is constprod K4/K5 = `constprod-lower-3` (todo): one route, 4a |
| CF1 | the rest of `ConstantFold` (`zero_operand` 98, `random_mixed` 60, `const_other` 23, `const_call_*` 23; `Fsin Fcos Flog2 Fexp2 Fpow` of constants); `mali-deqp-cfold2.md` 4 orders it A, C, D, B | CF | 204 (+ 75 of A: X1) | CF-B with the `libm-fold-design` helper (running) | CF-A 148: cfold-design (ready); CF-C 33, CF-D 11, CF-B 12: **no item** (4b) |
| CT1 | `ControlFlow` 73, `DiscardForm` 16; `mali-deqp-ctrlflow.md` 5 orders six one-session steps | `ControlFlow`, `DiscardForm` | 89 | steps 3-5 wait for intval's compares; step 2's `disc_uniform` part after R1 | step 1 `cf-dead` 45: ctrlflow-lower-1 (todo); steps 2-5 (12, 11, 9, 8): **no item** (4b); step 6 (4) belongs to loops-lower-1 per the note |
| DF1 | `Defect(Encode)` 6 (`attribute_location.bind_aliasing`), the library SIGSEGV 2 (`mali-deqp-defects.md`, done: root causes `encode` and `lanecycle`, references drawn) | `Defect` | 8 | the encode references need `gl_ref.py` `--bind` (`board-baseline-rows`, in review); the lowering is misc2 4 step 6 with `LinkedVertexNotLowered` 8 | encode 6: **no item** (4b); SIGSEGV 2: texmisc-design (ready) as the compiler fix, measure-crash-resume (running) for the measure's chunk |
| MISC | texts under 100: `TexKind` 42, `FsVaryingLoad` 43, `FragCoordMediump` 42, `VaryingDuplicate` 49, `ColourPair` (MX), `Invariant` 9, `LinkedVertexNotLowered` 8, `AlgebraicForm` 8 (K8), `AttributeNumbering` 8, `Rebalance` 12 (A8), `Spill` 2, `TooManyValues` 2, `FragmentTooLarge` 4, `Fp16Inexact` 3, `TexLanes` 1, `UniformIndexed` 10 | various | about 270 | `Fp16Inexact` stays with the constop item; TexKind/FragCoordMediump follow `mali-deqp-texmisc.md` 4 after `texmisc-lower-design` | **no item** (references: misc2-ref, texmisc-ref, varylink2-ref done; `misc2-sites` ready) |

## 3. Recommended serial order

Rules, in the order the goal gives them: (1) integer and boolean first; (2) then `gl_FragColor` lane
sources; (3) then the rest by cases per session; (4) never before a prerequisite; (5) two steps that edit
the same function go in the order the designs name. A slot is one agent session (design sizes: "one
session" each; `A1a`+`A1b` and `A4a`+`A4b` are the designs' own two-session steps; the merged loop
design groups `A0+S4+S3` as one item's first commits). The `R1`, `LS*` and `T*` designs are now merged
(`mali-intval-rest-lowering.md`, `mali-loops-lowering.md`, `mali-fmaloc-lowering.md`), so their slots are
firm; the `CF1` and `CT1` step splits are the cfold2 and ctrlflow notes' own (cfold2 4, ctrlflow 5). Every
lowering slot edits `src/shaderc/online/mali/**`, and at the time of writing plan 367's `lower-fma` item
holds that directory, which is why plan #387 keeps most lowering items `todo`; the measure items
(`*-sites`, `remeasure-tip`, `measure-crash-resume`) and the reference items (`intval4-ref`,
`loops3-ref`, `audit-ref`, `mali-ref-struct`, `unif2-struct-redraw`) do not wait for it.
**When the slot's item is not ready, take the next ready one** (`irindex-fix`, `intval-vs16`,
`arith-lower-1`, `constprod-lower-1`, `loops-lower-1`, `unif2-lower-1`, `mixcol-lower-1`, `negzero-lower`,
`cfold-design`, `ctrlflow-design`, `texmisc-design` are ready today).

| Q | Step | Item | First-refusal cases | Why here |
|--:|---|---|--:|---|
| 1 | IR1 | irindex-fix | 187 | a defect on bool/int shapes; the best cases per session; touches `mod.rs` only |
| 2 | V2 | intval-vs16 (vs16-2) | 445 (UT 346, UP 99) | the gate: loops, unif2 vertex halves and A6 need it; one short session |
| 3 | V1 | intval-vs16 (vs16-1) | 8+ | first byte-equal win, proves M1; same item as Q2 |
| 4 | V2b | intval-vs16 (vs16-2 session) | 179 | `vs_reduce` width two; design 10 says "with vs16-2" (item title widening: 4a) |
| 5 | R1 (1-3 sessions) | intval-rest (step 1; **no item** for steps 2-4) | 271 (+146) | the fragment integer/boolean text; int counters for loops |
| 6 | F1 | fclane-lower-1 | 406 | largest single lane mechanism, no prerequisite |
| 7 | F2D | fclane-lower-2 | 138 | after F1 |
| 8 | F2B | fclane-lower-2 | 36 | after F1 (may share Q7) |
| 9 | A1a | arith-lower-1 | 179 | F2C needs the mask lowerings |
| 10 | A1b | arith-lower-1 | 187 | A2 needs `transc32` |
| 11 | F2C | fclane-lower-3 | 139 checkable (94 more; their struct references are being unblocked by `mali-ref-struct` and `unif2-struct-redraw`) | after R1, A1a, A1b |
| 12 | F2E | fclane-lower-3 | 8 | after R1, F1 (may share Q11) |
| 13 | K1 | constprod-lower-1 | 174 | cheap `glsl.rs` rule; must precede N1 |
| 14 | A4a | arith-lower-4a | about 110 | widest risk, so early (before the corpus pins grow) |
| 15 | A4b | arith-lower-4b | about 106 | |
| 16 | A2 | arith-lower-2 | 189 | after A1b |
| 17 | U2 + U3 | unif2-lower-1 (after the 4a retitle) | 226 | needs V2, R1, F1 |
| 18 | U5 | unif2-lower-2 | 191 | needs U1, U3 |
| 19 | A3 | arith-lower-3 | 141 | |
| 20 | K2 | constprod-lower-2 | 124 | `glsl.rs`, after K1 |
| 21 | LS3 + LS4 (+ A0) | loops-lower-1 | 150 (128 NS) | needs R1 (counters); the design ships A0+S4+S3 first |
| 22 | LS1 | loops-lower-2 | 87 (27 Fail) | Fail cases; the merged design puts S1 in lower-2, after lower-1's A0/S3/S5 |
| 23 | LS2 | loops-lower-1 (fs) / loops-lower-2 (vs) | 94 | after LS3; one `Partial` rule serves both halves |
| 24 | V3 (+V4) | intval-vs16-3 | about 78 | needs the vs16 section-7 references (being drawn by `posvary-lower`, in review) |
| 25 | CF1 | cfold-design (CF-A; **no item** for CF-C/D/B) | 204 (+75 A) | N1/N2 need the libm fold (`libm-fold-design`); cfold2 4 gives 192 of 204 out of the line |
| 26-28 | N1, N2, N3 | negzero-lower (N1, N2; **no item** for N3) | 22, 84, 31 | after K1, CF1, R1 |
| 29 | MX4 | mixcol-lower-2 | 68 | takes MP under 100 |
| 30 | MX3 | **no item** | 64 | 55 need F1..F2C |
| 31 | MX2 | **no item** | 59 | |
| 32 | MX1 | mixcol-lower-1 | 18 | rebases onto F1 (shared `vectorise`) |
| 33 | MX5 | **no item** | 34 | |
| 34 | T1 | fmaloc-lower-1 | 104 | the design's ranked step 1; the FMA half goes with plan 367's `lower-fma` and K4/K5 (4a) |
| 35 | A5 | arith-lower-5 | 96 | |
| 36 | A6 | arith-lower-6 | 73 | after V2, V3 |
| 37 | K3 | **no item** | 64 | |
| 38 | K4 | constprod-lower-3 | 48 | |
| 39 | A7 | **no item** | 48 | after A1a, A3, A4; fixes the chain order K5 reuses |
| 40 | K5 | constprod-lower-3 | 47 | |
| 41 | K6 | **no item** | 41 | after K4 |
| 42 | K7 | **no item** | 19 | |
| 43 | K8 | **no item** | 16 | |
| 44 | A8 | **no item** | 12 | |
| 45-51 | LS8, LS10, LS5, LS7, LS6, LS9, LS11 | loops-lower-2, loops-lower-2, loops-lower-1, loops-lower-1, loops-lower-1, loops-lower-2, loops-lower-2 | 129 (32 + 97), 36, 28, 44, 24, 24, 18 | after R1, V2 |
| 52-54 | U6, U7+U8 | **no item** | 67, 8 | |
| 55 | T2 | **no item** (design: `texmisc-lower-design`) | 27 | |
| 56-60 | P1..P5 | **no item** (the `posvary-lower` id was repurposed; 4b) | 4, 6, 16, 15, 1 | 42 cases in five sessions: last of the families |
| 61+ | CT1, DF1, MISC | ctrlflow-lower-1 (CT1 step 1 only); **no item** for CT1 2-5, DF1 encode, MISC | 89, 8, about 270 | ctrlflow 5 orders the CT1 steps; the SIGSEGV half of DF1 is texmisc-design |

Counting the sessions with 1 per row (R1 as 2, ranges as written), the order is about 66 sessions.
Deliberate departures from "by count": A4 goes before A2 and A3 because it has the widest risk
(the pinned glmark2 corpus), not the most cases, so the later steps rebase onto it; K1 goes at Q13 because
N1 needs it; MX1 (18) is late because it rebases onto F1. Posvary has no item today (its old item id
was repurposed, 2.4; re-raise it from 4b) and its yield per session is the lowest of the families:
take it whenever no earlier slot is ready.

## 4. Double claims, missing items, forecast

### 4a. Steps claimed by two designs or two items, and the owner to keep

| Step | Claimed by | Keep | Reason |
|---|---|---|---|
| vs mediump `==`/`!=`, `any`/`all`, `bany` (86 cases; arith 6) | `mali-intval-vs16-lowering.md` M3 and `mali-arith-lowering.md` step 6 | **intval-vs16 / intval-vs16-3** | arith section 10 and vs16 section 10 say the same cases seen from two sides; arith step 6 shrinks from 159 to 73 |
| 179 `uniform_api.bool_compare` vs (width-two bool reduction) | vs16 section 10 ("schedule with vs16-2"), arith X2, intval-rest | **intval-vs16** (as V2b: design 10 makes the vs16-2 session add the arm; widen the item title to name it) | the vertex `vs_reduce` arm; it is not fragment |
| `fs_int_uniform` 52 / `fs_bool_uniform` 33 (families' FU: 76) | unif2 step 3 (M1) and intval-rest | **unif2** for the uniform read/layout (U3); intval-rest for the compare/convert | unif2 section 4 assumes intval-rest's compare exists |
| unif2 step 1 (`gl_ref.py` array/struct values; same files) | `fclane2-struct` (done: the `gl_ref.py` member/array work), `unif2-struct-redraw` (running: the redraw) and `unif2-lower-1` (titled "step 1") | **fclane2-struct + unif2-struct-redraw** for the step; **retitle unif2-lower-1 to U2+U3** (the bool word and the arrays) | the tool work landed with `fclane2-struct` and the redraw is `unif2-struct-redraw`'s scope; else `unif2-lower-1` has nothing of its own to change |
| unif2 M2 (197 `fs_mat_uniform`, `vs_mediump_struct`, `vs_mediump_array`) | unif2 step 9 and fclane | **fclane-lower-1/2** | unif2 step 9 is "check only" |
| mixcol M4 steps 1-2 (77) and fclane step 1 (same `vectorise`/`lane_tree` emission) | mixcol-lower-1 and fclane-lower-1 | both, **fclane-lower-1 first** | whichever lands second rebases; the first's words are a subset |
| mixcol `hp_loop` (3) | mixcol step 3 and loops | **loops** | mixcol says "count them once, under whichever lands the highp loop counter" |
| ref note `mali-deqp-fmaloc.md` FMA rows (63) | `mali-fmaloc-lowering.md` (merged; its ranked steps 2, 3, 5, 6 are these 63) and constprod S4-S7 (K4/K5 = `constprod-lower-3`; K6/K7 no item) | **constprod** (families' FMA text is 139, all in K4-K8); plan 367's `lower-fma` runs first and `fmaloc-lower-1` takes only the top remaining step | fmaloc 5: "Do not write a second fold: route the `ffma` through `vs.alu`" |
| constprod S4 route (48) and `ConstantFold` | constprod S4 and the cfold items | **constprod S4** (route); cfold owns the value (`Fsin Fcos Flog2 Fexp2 Fpow`) | constprod section 3 splits by the text a case reaches first |
| built-in folds in `glsl.rs` (negzero N1/N2 `tan exp log sin cos ...`, CF-B `log2/log/pow`, constprod S4) | negzero-lower, the cfold items and constprod S4 | **one libm-fold helper, designed by `libm-fold-design` (running)**; it folds in `glsl.rs` before the expansion, and CF-B, N1, N2 and S4 all call it | Mesa folds with the host libm; two implementations would differ in bits |
| `glsl.rs` `neg_products` / `plus_zero` | constprod S1 and negzero-lower | K1 first, then N1 | constprod section 5 |
| `builtin_call.rs` chains | arith step 7 (fs) and constprod S5/S7 (vs) | different walks, **A7 first (it has no item - 4b), K5 (`constprod-lower-3`) and K7 (no item) after** | whoever lands first fixes the `ffma` order |
| `vs_reduce` arms | vs16 M3 and arith 6 and unif2 step 6 | V3 writes them as arms before `_ => Err(Arithmetic)`; A6 and U6 rebase | vs16 section 10 |
| `IrIndex` 187 | irindex-fix and intval-rest (families 1a "to triage with intval-rest") | **irindex-fix** | a defect, not a lowering |
| posvary M2's 16 cases (`367:FMA` in the same vertex shader) | posvary P2 and constprod K5-K7 | P2 (no item today) for the lane, K for the FMA | the 6 cases are P2's; the 16 follow K |
| loops `S2` 94 (`indexing.*dynamic_loop*`) | loops-lower-1 (fragment, 50) and loops-lower-2 (vertex, 44) | decided by `mali-loops-lowering.md` 4-5: S2-fs in lower-1, S2-vs in lower-2, reusing the `Partial { n }` rule | the design is merged; the claim is split, not open |
| audit causes D1 (`b2i32` of a 16-bit mask widens the wrong lane, est. 13) and F1 (a compare of two varying lanes at fp16 where Mesa uses fp32, est. 228) | `ctrlflow-design` (the fix item, ready), `audit-ref` (running: the `deqp_audit_*` references), and the mask/compare lowerings that pass the same arms (R1 steps 1-2, F2C, A3's float `!=`) | **ctrlflow-design + audit-ref**: one fix commit pinned by the audit references | `mali-deqp-compiled-audit.md` 4.1/4.3; fixing the lane selection before the mask lowerings land keeps one byte-equal story per shape |

### 4b. Steps with no item (item titles, one-line scope)

What `lower-schedule` listed here as missing has since gained items: `arith-lower-2/3/4a/4b/5/6`
(A2-A6), `fclane-lower-3` (F2C+F2E), `unif2-lower-1` (retitle to U2+U3) and `unif2-lower-2` (U5),
`mixcol-lower-1/2` (MX1, MX4), `constprod-lower-1/2/3` (K1, K2, K4+K5), `cfold-design` (CF-A),
`ctrlflow-lower-1` (CT1 step 1), `fmaloc-lower-1` (T1), `loops-lower-1/2` (the merged design split
all eleven loop steps between them), `irindex-fix`, `negzero-lower` (N1+N2), `intval-vs16` and
`intval-vs16-3` (V1-V4, V2b in the vs16-2 session); plus the reference/design items `intval4-ref`,
`mali-ref-struct`, `unif2-struct-redraw`, `libm-fold-design`, `loops3-ref`, `audit-ref`,
`texmisc-lower-design`, the `*-sites` family, `remeasure-tip` and `measure-crash-resume`. What is
left without an item:

1. **arith-lower-7**: fs composites `acos asin atan atan2 faceforward refract` (A7, 48), and **arith-lower-8**: rebalance chains (A8, 12; `mali-deqp-misc2.md` 4 step 4 gives it references).
2. **intval-rest-2..4**: the rest of intval-rest - M2 `iand`/`ior` of masks and the constant-first `fmin`/`fmax` rule (about 40 nominal; F2C and step 3 read its masks), M3 fragment int/bool uniform arrays and compare words (226 nominal; waits on the U1 redraw, U2 and F2C, and shares its array layout work with U3), M4 int selects (12-27; the design waits on arith's `Bcsel` arm - its step 5). The `intval-rest` item owns step 1 only.
3. **posvary-lower-1..5** (re-raise; the old `posvary-lower` id now draws the vs16 section-7 references): P1..P5 (42 cases HOST-only in five sessions).
4. **unif2-lower-3**: U6 vs bvec/bool compare chain (67), U7 `fs_other` uniform-word pair (7), U8 mat3 array element (1).
5. **mixcol-lower-3..5**: MX2 random pairs (59), MX3 declared highp (64), MX5 live-node width (34).
6. **constprod-lower-4..7**: K3 (64), K6 (41; its arms are shared with `mali-fmaloc-lowering.md` step 2 - 4a), K7 (19), K8 (16; misc2 pairs the 2 random `AlgebraicForm` cases with it).
7. **negzero N3** (a `negzero-lower` second commit or **negzero-lower-2**): `mainplain` int/bool constant folding and copy propagation (31; needs intval-rest steps 1-2).
8. **cfold-lower-2..4**: CF-C an infinity the next op absorbs (33), CF-D a mediump `rcp` of a constant (11), CF-B `log2`/`log`/`pow` of constants (12) through the `libm-fold-design` helper. (X1's 75 A-text constant folds: the CF-A/C/D routes move 192 of CF's 204 under the line on their own, cfold2 4.)
9. **ctrlflow-lower-2..5**: the ctrlflow note's steps 2-5 - `disc-forms` (12), `fs-select32` (11), `vs-nested-select` (9), `fs-mask-logic` (8); its step 6 belongs to `loops-lower-1`. Step 1 is `ctrlflow-lower-1`; the audit D1/F1 fix is `ctrlflow-design` + `audit-ref`.
10. **texmisc-lower-1..6** (= T2 plus the TexKind/FragCoordMediump half of MISC): `mali-deqp-texmisc.md` 4's six steps (TexVertex harness 60, fp16 int round trip 32, TexKind projection 14, bias 17, cube 11, FragCoordMediump rest 10 - its own tip counts). The design is `texmisc-lower-design` (running); step 1 waits on `intval-vs16` and `arith-lower-1`; the fmaloc cube sequence (T1) feeds steps 3 and 5.
11. **loops, the shapes outside loops-lower-1/2** (`mali-loops-lowering.md` 8): the `IndexedLocal` follow-up of S2/S7/S11 (156 cases: local arrays at a run-time index, `mali-indexed-locals.md` territory - the loop text leaves but those cases still do not compile, next refusal unmeasured), the thread-local scratch-array shape `fs_idx_tmp_vec4_cd` (4), and a second dead-position loop if S11 finds one. (`LS1` and `LS2` themselves now have items: the merged design assigned S1/S2-vs to `loops-lower-2` and S2-fs to `loops-lower-1`.)
12. **defects-fix + misc-lower** (= the DF1 encode half and the rest of MISC): `mali-deqp-misc2.md` 4's ownerless steps - `AttributeNumbering` (8), Invariant A `posonly` and B `cse` (9 + 3; they wait on unif2's `u_color`), `LinkedVertexNotLowered` 8 together with the 6 `Defect(Encode)` cases (the `gl_ref.py` `--bind` references are `board-baseline-rows`, in review), `Spill` 2, and `FragmentTooLarge` 4 + `TooManyValues` 2 + `TexLanes` 1 last; plus `FsVaryingLoad` 43 and `VaryingDuplicate` 49 (the varylink2 references). `Fp16Inexact` stays with the constop item; the SIGSEGV fix is `texmisc-design`.

U4 (the host forecast after U3) is not a lowering and needs no item of its own: the design's
step-3 session performs it, or a `remeasure-tip`/board rerun covers it.

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
| IV | 271 | Q5 0 (R1; design merged, item covers step 1 - 4b) | **Q5** |
| UT | 462 | Q2 116; Q18 (U5) 0 | **Q18** (116 after Q2 is the vs struct shapes) |
| UP | 99 | Q2 0 | already under; 0 at Q2 |
| C | 821 | Q6 415; Q7 277; Q8 241; Q11 102; Q12 94 | **Q12** (94 left = the structType shapes: `fclane2-struct` landed the `gl_ref.py` side; `mali-ref-struct` and `unif2-struct-redraw` must make the struct references byte-comparable, or C stays at 94, 6 under the line) |
| FU | 280 | Q6 201 (fs_mat 79 lifted by the lane, unif2's reading); Q17 125; Q18 50; Q52-54 43 | **Q18** (if `fs_mat_uniform` does not leave with the lane, FU stays at 129 and has no step: missing item) |
| UA | 193 | Q17 56 (137 of `bool_array`+`int_array`); Q18 22 (`array_in_struct` 34 assumed); U8 −1 | **Q17** |
| AC | 386 | Q13 212; Q20 88; Q37 24; Q38 0 | **Q20** |
| FO | 152 | Q14-15 0 (A4 assumed to take all) | **Q15**; Q14 alone only if A4a takes 53 or more |
| A | 1463 | Q3 1455; Q4 1276; Q5 1130; Q9 951; Q10 764; Q14-15 700; Q16 511; Q19 370; Q24 292; Q25 217; Q35 121; Q36 48; Q39 0 | **Q36** (the last of the large texts: A5 and A6 are needed) |
| FMA | 139 | Q38 115; Q40 68; Q41 27; Q42 8; Q43 0 | **Q40** |
| NZ | 137 | Q26 115; Q27 31; Q28 0 | **Q27** |
| LM | 281 | Q21 131; Q23 81; Q45-51 (LS5-7) 0 | **Q23** |
| LV | 241 | Q2 338 (+97 hidden int loops); Q22 251; Q23 207; Q45 (LS8, 32 + 97) 78; Q46 42; Q49-51 0 | **Q45** |
| CF | 204 | Q25 0 (needs CF-C/D/B too - 4b) | **Q25** (cfold2 4: CF-A 148 is `cfold-design`; A+C+D take 192 of 204 under the line without the 4b items) |
| TC | 182 | Q34 78 (T1's 104; the 78 other cases are not in fmaloc's count: assumption) | **Q34** |
| MP | 166 | Q29 98; Q30 34; Q33 0 | **Q29** |

Assumptions behind the table: step 4's 216 = FO 152 + A 64; constprod S4 = 24 AC + 24 FMA and S8 = 8 FMA +
8 `AlgebraicForm`; V2 leaves `vs_struct_uniform` 116 in UT; V2b (179) and R1's isok 146 come out of A; the whole
arith step 6 is 86 (V1 8 + V3 78) + 73. The LS counts are confirmed by `mali-loops-lowering.md` 6: LM 281 + `GuardMediump`
12 + `LoopForm` 3 = 296 = LS3 + LS4 + LS2 (fs 50) + LS5 + LS6 + LS7.

**c2 (every text under 100) is reached at Q45** (LS8, the last text over the line), after about 50 sessions,
provided inflow does not push another text over (`ControlFlow` 73, `ColourPair` 61, `TexKind` 42 and `FsVaryingLoad`
43 are the ones to watch: the gates V2, R1 and F1 send cases into them). The first board rerun
(`board-rerun-1`) should come after Q12 (intval and all of fclane: C 94, IV 0, IR 0, UT 116): by
then IR, IV and C are under the line (UP and the UT gate too) and the table is re-ranked from
measured counts; a second rerun after Q36 (A) and one after Q45 close c2.
