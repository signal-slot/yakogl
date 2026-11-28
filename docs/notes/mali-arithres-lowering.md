<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the `Arithmetic` residue after trains 2 and 3a: owners, Mesa references and the four unowned forms (plan #387, item arithres-refs)

**Status: design and references, no compiler change.** The Verdin iMX95 (G310)
rerun gave the text `MaliError::Arithmetic` ("arithmetic other than +, -, * and
a dot or matrix product ...") 886 fragment and 623 vertex refusal lines. The text
is still the largest. This note measures, at main's tip, which shapes
still stop on it. It groups them by sub-form and names the existing design that
owns each one (`mali-arith-lowering.md`, `mali-highpfs-lowering.md`,
`mali-intval-vs16-lowering.md`, `mali-vsunif-lowering.md`,
`mali-vsuarr-lowering.md`, `mali-libm-fold.md`, `mali-loops2-lowering.md`, the
negzero notes). For the forms no design draws, it adds the Mesa 26.2.1
references `tools/mali/testdata/deqp_arithres_*` and the lowering steps
AR1 to AR4. The test is `tests/mali_deqp_arithres.rs`.

## 1. Method and totals

- **Tip.** main `56b5878aaeeb` (2026-11-28), after trains 2 and 3a and
  after lower-train-2 (`a27396fa`) and `4378e983`. The table was first drawn
  at the integration tip `6e9e515c3f2a`, then measured again here.
- **Measure.** `tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS-host> --out
  build/measure --jobs 3 --chunk 100`, the method of `mali-deqp-families.md`
  section 19. It used the same 10,597 cases of the NanoPC-T6 slices
  `2026-11-15-t6-gles2-S{1..4}-37305608` and CTS `93bca018` with the yakogl
  target (`HOST_DEQP_CTS` of `tools/cts/deps.toml`). HOST-ONLY: compile and
  link outcomes. 4,693 cases meet a host refusal. **1,406 have `Arithmetic`
  as their first refusal: 743 vertex, 663 fragment** (1,317 board Fail; the
  89 board NotSupported are all fragment).
- **What moved since `6e9e515c`.** At `6e9e515c` the count was 1,410 (743
  vertex, 667 fragment). That is the split of section 19 at `e9033663`, so
  train 3a (unifmp, opform O1-O3) moved no `Arithmetic` case. The same
  measure was run again at `6e9e515c` on every case that refuses with
  `Arithmetic` at either tip. (`6e9e515c`'s `capi/Cargo.toml` has the
  `vivante-st` feature twice, and cargo refuses that manifest, so line 41 was
  deleted in that worktree.) Case by case, the two tips differ in 12 cases:
  - The 8 `operator.geometric.refract.*_fragment` cases left the text.
    lower-train-2's FS_REFRACT lowers their selects (they were F4).
  - 4 cases refused with another text first at `6e9e515c` and now meet
    `Arithmetic` first. `fragment_ops.interaction.basic_shader.{23,51}`
    went past `varylink` and now stop in the `Fne` arm on
    `vec3(e, f, e) != d` (F11). `shaders.random.trigonometric.fragment.{47,49}`
    went past `constop` and now stop in the highp `Fsign` arm on
    `asin`/`acos` (F10).
  - The other 1,402 cases have `Arithmetic` first, in the same stage, at
    both tips. The vertex side did not change.
- **Site.** A throw-away worktree (not committed) printed `file:line` at
  every `MaliError::Arithmetic` construction in `nir.rs`, `nir_fs.rs` and
  `vec16.rs`. The line numbers are `56b5878a`'s. A scratch test called
  `tests/mali_ref`'s `compile_pair` on the first refusing program of every
  one of the 1,406 cases (from `programs/` of the measure). The site of a
  case is the last site that program printed. All 1,406 cases refuse with
  the same text on the scratch path. At `6e9e515c`, `nir::vs_reduce` also
  printed the reduction's `(AluOp, compare, width)`. Every arm's count is
  the same at both tips (apart from F4, F10 and F11 above), so that split
  of V1-V5 still holds.
- **Owner.** A sub-form is **owned** when a merged design names it as a step
  and a committed reference pins it. The reference must stop at the same
  site: at `6e9e515c`, the same instrumentation was run over all 1,384 committed
  `deqp_*`/`gm2_*` pairs, and section 2's "Pinned by" column lists the ones
  that stop there. A sub-form that no design draws is **new**.

The G310 refusal-line counts are not case counts. A line is one refused
compile on the board, so a case can log several. The host counts below are
first-refusal cases, the unit of `mali-deqp-families.md`.

## 2. The residue, every case

`Cases` are first-refusal cases at `56b5878a`. The column sums are 743 (vertex)
and 663 (fragment). Sites are `file:line` at the tip and drift with every
compiler commit. The arm name is the stable handle.

### 2.1 Vertex (743)

| # | Sub-form | Site (arm) | Cases | dEQP groups | Owner | Pinned by (stops at the site) |
|--:|---|---|--:|---|---|---|
| V1 | `all(equal(..))` of a 4-wide int/bool vector (`Ball Ieq 4`) | `nir.rs:4782` (`vs_reduce` last arm) | 259 | uniform_api.value 200 (bvec4/ivec4, structs, arrays), uniform_api.info_query 30, uniform_api.random 9, struct.{local,uniform} 6, fragment_ops.interaction, operator.equal | vs16 M3/M4 (R5, R9); vsunif V1; vsuarr VA1; unif2 U6 (train 3b) | `deqp_vsuarr_init_{bvec4,ivec4}`, `deqp_vsuniarr_{bvec4,ivec4,unused_bvec4}`, `deqp_vs16ref_{bcmp_eq_bvec4,uapi_bvec4_mix}`, `deqp_unif2_vs_struct_uniform`, `deqp_arith_other_ball_vs_none`, `deqp_cfold2_random_mixed_inot` |
| V2 | `all(equal(..))` of a 3-wide bool vector, a lane not a plain bool word (`Ball Ieq 3`) | `nir.rs:4729` | 82 | uniform_api.value 44, uniform_api.random 14, shaders.random.conditionals 10, info_query 8 | vs16 M3; vsuarr VA1 | `deqp_vsuarr_init_multi`, `deqp_vsuniarr_multi`, `deqp_vsuniarr2_{bvec3,ivec3}_part`, `deqp_vs16ref_uapi_bvec3_mix`, `deqp_cfold2_random_mixed_icmp` |
| V3 | `Ball Ieq 2` (ivec2/bvec2: a lane not a bool word against 0 or ~0, or both constants false) | `nir.rs:4752`, `:4756` | 29 + 13 | uniform_api.value 22, operator.equal 4, uniform_api.random 6, info_query 3, fragment_ops.interaction 2 | vs16 M3 (width 2 held, 4.6) | `deqp_arith_op_binary_equal_vs_{highp,mediump,none}` |
| V4 | `any(notEqual(..))` of int/bool vectors (`Bany Ine 2/3/4`) | `nir.rs:4782` | 12 + 16 + 14 | operator.not_equal, uniform_api, fragment_ops.interaction | vs16 M3 (R5); arith step 6 | `deqp_arith_op_binary_not_equal_vs_*`, `deqp_arith_other_bany_vs_none`, `deqp_arith_random_bany_vs_none`, `deqp_vs16ref_bcmp_ne_bvec4` |
| V5 | float-vector `==`/`!=` (`Ball Feq 2/3/4`, `Bany Fne 2/3/4`) | `nir.rs:4766`, `:4782` | 4 + 15 + 8 + 10 + 6 + 7 | operator.{equal,not_equal} vecN, struct compares, fragment_ops.interaction | vsunif V1/VT1; vs16 M3 | `deqp_vsunif_{local_equal,local_nested_equal,struct_cmp}`, `deqp_vs16ref_eq_mediump_vec4`, `deqp_negzero_mainplain_vs` |
| V6 | `any`/`all` of a value that is not an ALU result (a constant or a bool input) | `nir.rs:4633` | 12 | constant_expressions.vector_relational 6, operator.bool_compare.{all,any} 6 | arith step 6 / X1 (constant `ball`/`bany`); vs16 R5 | `deqp_arith_constexpr_{ball,bany}_vs_highp`, `deqp_arith_op_bool_{all,any}_vs_none`, `deqp_vs16ref_{ball,bany}_bvec{3,4}` |
| V7 | a built-in kernel of constants that `fold::fold_float` cannot make exactly (`sin cos tan exp exp2 log log2 pow`) | `nir.rs:7238` | 150 | shaders.random 117 (all_features 50, exponential 38, trigonometric 29), constant_expressions 33 | arith X1 → `mali-libm-fold.md` CF1 (`mesa_fold.rs`, no caller yet) | `deqp_arith_constexpr_{fcos,fexp2,flog2,fsin}_vs_highp`, `deqp_arith_random_{flog2,fsin}_vs_none`, `deqp_cfold2_random_mixed_rsq_inf`, `deqp_constrnd_sqrt_zero_vs`, `deqp_negzero_mainbuiltin_vs` |
| V8 | mediump `min`/`max`/`clamp` (fp16 in Mesa) | `nir.rs:4407` (`vs_clamp`, no constant operand) | 43 | operator.common_functions.{min,max,clamp} 42, algorithm.rgb_to_hsl_vertex 1 | arith step 6 | `deqp_arith_op_common_{min,max,clamp}_vs_mediump`, `deqp_arith_other_fmax_vs_mediump` |
| V9 | `min`/`max` of an `inout` array argument copy | `nir.rs:4407` | 4 | functions.array_arguments.copy_* | negzero (array `inout`) | `deqp_negzero_array_inout_vs` |
| V10 | **vec3/vec4 `sin cos tan`, vec4 `exp exp2 log log2` of a mediump attribute** | `nir.rs:6598` (the `wide` check) | 27 | operator.angle_and_trigonometry.{sin,cos,tan}.{mediump,lowp}_vec{3,4} 12, operator.exponential.*.mediump_vec4 4, shaders.random 11 | **new: AR2** | `deqp_arithres_vswide_*` (6) |
| V11 | mediump `ceil`, `sign` (fp16 in Mesa) | `nir.rs:7242` (vertex catch-all) | 16 | operator.common_functions.{ceil,sign}.{mediump,lowp}_* | arith step 6 | `deqp_arith_op_common_{ceil,sign}_vs_mediump` |
| V12 | mediump `tan`, `log` (a product of a kernel's result) | `nir.rs:6580` | 7 | operator.angle_and_trigonometry.tan 4, operator.exponential.log 3 | arith step 6 | `deqp_arith_op_angle_tan_vs_mediump`, `deqp_arith_op_exponential_log_vs_mediump` |
| V13 | a kernel of a computed or mediump-uniform operand (mediump `exp`'s `x * log2(e)`, `exp2(vec4(.., u_mediump, ..))`) | `nir.rs:6604` | 9 | operator.exponential.exp.mediump 3, shaders.random 6 | arith step 6 | `deqp_arith_op_exponential_exp_vs_mediump` |
| | **Vertex total** | | **743** | | new: 27 | |

### 2.2 Fragment (663)

| # | Sub-form | Site (arm) | Cases | dEQP groups | Owner | Pinned by (stops at the site) |
|--:|---|---|--:|---|---|---|
| F1 | `ivecN(floor(a + 0.5)) == b`: the shader library's `isOk` int harness (floor of a computed lane, then `f2i`) | `nir_fs.rs:7736` (`Fceil\|Ffloor` arm) | 154 | swizzles.vector_swizzles 93, conversions 53, struct 6, functions.array_arguments 2 | arith X2 → intval-rest R1 (and the plan's `fsintres-design`) | `deqp_arith_isok_int_round_fs_mediump`, `gm2_fsarith2_6` |
| F2 | `int(floor(v * 4.0) + 0.05)` of the `gl_Max*` built-in tests | `nir_fs.rs:7736` | 8 | builtin_variable.max_* | arith step 3 (`other.ffloor`) | `deqp_arith_other_ffloor_fs_none` |
| F3 | `bcsel` of computed arms: `?:` and the dynamic-index select | `nir_fs.rs:8060` (`Bcsel` arm) | 125 | indexing 83 (vector/matrix subscript, tmp_array, varying_array), operator.selection 30, functions.array_arguments 4, loops.*_dynamic_iterations 3, struct 3, algorithm.hsl_to_rgb 1, random.trigonometric 1 | arith step 5; loops2 / idxloc for the loop and index shapes | `deqp_arith_{indexing_dynamic_select_fs_none,op_selection_select_fs_*,other_bcsel_fs_*}`, `deqp_idxloc_fs_*`, `deqp_loops2_fs_{do,for}_dyn_select_iteration_count`, `deqp_loops2_fs_idx_*`, `deqp_intval4_sel_*`, `deqp_negzero_array_inout_fs`, `gm2_nbshrf_6` |
| F4 | `atan`, `atan2`, `faceforward` (their selects; `refract` left with FS_REFRACT, `a27396fa`) | `nir_fs.rs:8060` | 24 | operator.angle_and_trigonometry.{atan,atan2} 16, operator.geometric.faceforward 8 | arith step 7 | `deqp_arith_op_angle_{atan,atan2}_fs_*`, `deqp_arith_op_geometric_faceforward_fs_*` |
| F5 | `sin`/`cos`/`log2` of a computed lane, a constant or a uniform (not a varying's lane) | `nir_fs.rs:8094` (`Fsin\|Fcos\|Flog2` arm) | 113 | shaders.random 108 (trigonometric 43, all_features 37, exponential 28), flush_finish 5 | arith step 1b (its random half) | `deqp_arith_random_{fsin,fcos,frsq}_fs_mediump`, `deqp_arith_other_fsin_fs_none` |
| F6 | highp `vec2`-`vec4` kernels (`sin cos tan log log2 pow`) | `nir_fs.rs:10884` (`vectorise`, `has_raw_transc`) | 18 | operator.*.highp_vec{2,3,4}_fragment | highpfs HF1 | `deqp_highpfs_hp_{sin_vec2,sin_vec3,sin_vec4,cos_vec3,tan_vec3,log_vec3,log2_vec3,pow_vec3}` |
| F7 | **mediump/lowp `vec2`-`vec4` kernels on a varying's lanes** (a raw kernel beside another lane in a colour word) | `nir_fs.rs:10884` | 31 | operator.angle_and_trigonometry.{sin,cos,tan}.{mediump,lowp}_vec{2,3,4} 18, operator.exponential.{log,log2,pow}.mediump_vec{2,3,4} 9, shaders.random 4 | **new: AR1** | `deqp_arithres_fsvec_*` (7) |
| F8 | `exp2` of a constant | `nir_fs.rs:8135` | 41 | shaders.random.exponential 33, all_features 8 | arith step 1b (the constant `exp2`) | `deqp_arith_random_fexp2_fs_mediump`, `deqp_constrnd_rnd_fragment43` |
| F9 | `sqrt`/`inversesqrt` of a uniform or a constant | `nir_fs.rs:8206` (`Frsq\|Fsqrt` arm) | 35 | shaders.random 35 | arith step 3 | `deqp_arith_random_fsqrt_fs_mediump` |
| F10 | fp32 `sign`: highp `sign`, and `acos`/`asin` (built at 32 bits) | `nir_fs.rs:8250` (`Fsign` highp arm) | 31 | operator.angle_and_trigonometry.{acos,asin} 16, operator.common_functions.sign.highp 4, shaders.random 11 (trigonometric 8, all_features 3) | arith step 7 (`acos asin`); highpfs HF1 / arith 3 (`sign`) | `deqp_arith_op_angle_{acos,asin}_fs_*`, `deqp_arith_op_common_sign_fs_highp`, `deqp_highpfs_hp_sign_vec3`, `deqp_arith_random_flog2_fs_mediump` |
| F11 | float `!=` of operands that are not two varyings' lanes | `nir_fs.rs:8292` (`Fne` arm) | 29 | shaders.random.conditionals 21, fragment_ops.interaction 5, random.all_features 2, functions 1 | arith step 3 | `deqp_arith_random_fne_fs_mediump` |
| F12 | highp constant built-ins in the fp32 domain (`sin cos tan`, `all`/`any` of constant compares, `isOk`'s `all(lessThanEqual)` of a highp constant) | `nir_fs.rs:4487` (`Fs::alu32` catch-all) | 20 | constant_expressions 19, misc.compound_assignment 1 | arith X1 | `deqp_arith_constexpr_{ball,bany,fcos,fsin}_fs_highp` |
| F13 | `1.0 / gl_FragCoord.w` | `nir_fs.rs:7882` (`Frcp` arm) | 1 | builtin_variable.fragcoord_w | arith step 1a | `deqp_arith_other_frcp_fs_none` |
| F14 | **`FRCP` of a computed mediump lane** (a product, a `max - min`, `atan`'s `1 / x` of a product) | `nir_fs.rs:7882` | 9 | shaders.random.trigonometric 8, algorithm.rgb_to_hsl 1 | **new: AR3** | `deqp_arithres_fsrcp_prod` |
| F15 | **highp `mat3 / float`, `mat3 / mat3` in a fragment shader** | `nir_fs.rs:7882` | 4 | matrix.div.{uniform,dynamic}_highp_mat3_{float,mat3}, matrix.div_assign.dynamic_highp_mat3 | **new: AR4** | `deqp_arithres_matdiv_hp_mat3_{float,mat3}` |
| F16 | highp constant `exp`/`exp2` (`pow32_of` none) | `nir_fs.rs:4485` | 8 | constant_expressions.exponential.{exp,exp2} | arith X1 | `deqp_arith_constexpr_fexp2_fs_highp` |
| F17 | a constant tree whose fp16 halves repeat across pairs | `nir_fs.rs:1515` (`builtin16`) | 6 | shaders.random.all_features 6 | negzero N1/N2 (constant globals, built-ins in `main`) | `deqp_negzero_{constglobal,mainbuiltin}_fs` |
| F18 | highp `exp` (Mesa moves `x * log2(e)` into the vertex shader) and `exp` of a non-varying operand | `nir_fs.rs:8137` | 6 | operator.exponential.exp.highp 4, shaders.random.exponential 2 | highpfs HF1 (`exp`); arith 1b | `deqp_highpfs_hp_exp_{float,vec3}`, `deqp_fclane_op_pair_x` |
| | **Fragment total** | | **663** | | new: 44 | |

**Owned: 1,335 cases. New: 71** (AR1 31, AR2 27, AR3 9, AR4 4). Most of
the residue is owned by four groups. The vertex compare reductions
`vs_reduce` (V1-V6, 487 cases) belong to vs16/vsunif/vsuarr/unif2: 3b's VA1
or VT1 writes the arm first, and each design names it. The rest are arith
steps 5 and 7 (F3, F4: 149), arith X1/libm-fold (V7, F12, F16: 178) and arith
X2/intval R1 (F1: 154). None of these lacks a reference.

## 3. The new forms: Mesa's words

All 17 references are drawn on Mesa 26.2.1's G610 drm-shim
(`tools/mali/deqp_ref.sh --instrument shim`, `--uniform-values`).
`tools/mali/g310_check.sh 'deqp_arithres_*'` gives 17 identical, 0 differ: the
G310's words are the same. `.nir.sha` is `tools/mali/nir_refs.sh`. The words
quoted below leave out the loads, the stores and the `ATEST`/`BLEND` tail.

| Reference (`deqp_arithres_`) | Case (dEQP-GLES2.functional.) | Form | `fs.fragment` words | Outcome at the tip |
|---|---|---|--:|---|
| `fsvec_sin_vec2` | shaders.operator.angle_and_trigonometry.sin.mediump_vec2_fragment | AR1 | 29 | refused, `nir_fs.rs:10884` |
| `fsvec_sin_lowp_vec3` | ...sin.lowp_vec3_fragment | AR1 | 41 | refused, `:10884` |
| `fsvec_cos_vec3` | ...cos.mediump_vec3_fragment | AR1 | 41 | refused, `:10884` |
| `fsvec_tan_vec4` | ...tan.mediump_vec4_fragment | AR1 | 70 | refused, `:10884` |
| `fsvec_log2_vec3` | shaders.operator.exponential.log2.mediump_vec3_fragment | AR1 | 26 | refused, `:10884` |
| `fsvec_log_vec2` | ...log.mediump_vec2_fragment | AR1 | 26 | refused, `:10884` |
| `fsvec_pow_vec4` | ...pow.mediump_vec4_fragment | AR1 | 46 | refused, `:10884` |
| `vswide_sin_vec3` | ...sin.mediump_vec3_vertex | AR2 | 7 | refused, `nir.rs:6598` |
| `vswide_sin_vec4` | ...sin.mediump_vec4_vertex | AR2 | 7 | refused, `:6598` |
| `vswide_cos_vec4` | ...cos.mediump_vec4_vertex | AR2 | 7 | refused, `:6598` |
| `vswide_tan_vec3` | ...tan.mediump_vec3_vertex | AR2 | 7 | refused, `:6598` |
| `vswide_exp2_vec4` | ...exp2.mediump_vec4_vertex | AR2 | 7 | refused, `:6598` |
| `vswide_log2_vec4` | ...log2.mediump_vec4_vertex | AR2 | 7 | refused, `:6598` |
| `fsrcp_prod` | (written after the operator template: `1.0 / (in0.x * in0.y)`) | AR3 | 11 | refused, `nir_fs.rs:7882` |
| `fsrcp_diff` | (written: `1.0 / (in0.x - in0.y)`, AR3's boundary) | - | 11 | **byte-equal** (`LOWERED`) |
| `matdiv_hp_mat3_float` | shaders.matrix.div.uniform_highp_mat3_float_fragment | AR4 | 21 | refused, `nir_fs.rs:7882` |
| `matdiv_hp_mat3_mat3` | shaders.matrix.div.uniform_highp_mat3_mat3_fragment | AR4 | 27 | refused, `:7882` |

Every refusal is the whole `Arithmetic` message (`refusals_pinned`).
`fsvec_sin_lowp_vec3` (lowp, `v_in0.zxy`) has the same shape as the mediump
vec3 references: lowp is mediump in Mesa.

**AR1, fragment vectors of a raw kernel** (`fsvec_*`). One fp32 load of the
varying (`LD_VAR_BUF_IMM.f32...v4`). The fp32 kernel of arith step 1b runs on
each lane's own word (sin and cos: `FMA.f32 x, u2.w0, u2.w1`, `FADD_IMM.f32
#0xC9400000`, `FMA.f32`, `FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32`,
`FMA.f32 .neg`, `FMA.f32.clamp_m1_1`, `FADD.f32`; `log2`: `FREXPE.f32.log`,
`S32_TO_F32`, `FADD_LSCALE.f32`, `FLOGD.f32`, `FMA.f32`). **The narrowing
does not follow the colour words.** Mesa narrows the result vector in its
own groups, and each kernel runs in the order of those groups. `MKVEC.v2i16`
then re-pairs the halves into the colour words. Three widths are drawn:

| Width (dEQP's `in0`) | Narrowing groups, in kernel order | Example |
|---|---|---|
| vec2 (`v_in0.wy` into `color.yz`) | in0.x alone, in0.y alone (`FADD.f32 rN.h0, rN^, 0x0.neg` each) | `fsvec_sin_vec2`, `fsvec_log_vec2` |
| vec3 (`v_in0.zxy` into `color.xyz`) | in0.x alone, then (in0.y, in0.z) in one `V2F32_TO_V2F16` | `fsvec_cos_vec3`, `fsvec_log2_vec3`, `fsvec_sin_lowp_vec3` |
| vec4 (`v_in0.yzwx` into `color`) | (in0.x, in0.w), then (in0.y, in0.z), each one `V2F32_TO_V2F16` | `vswide_cos_vec4` (the same in the vertex stage) |

In vec3, for example, the colour words are `MKVEC.v2i16 (x.h0, yz.h0)` and
`MKVEC.v2i16 (yz.h1, alpha)`. One `FMA.v2f16` per word applies the operator
template's `* 0.5 + 0.5`. `tan_vec4` runs all four reductions (`FMA`,
`FADD_IMM`, `FMA`) first. Then come the four table pairs, two
`FMA.f32.clamp_m1_1` per lane, and the `FADD`s. Then four `V2F32_TO_V2F16`
(the sines and the cosines, pairwise) and an `FRCP.f16` of each cosine half.
The quotient is reassociated with the scale: `FMA.v2f16 s, sin, 0.5, -0`,
then `FMA.v2f16 c, s, rcp, 0.5`. `log_vec2` is two `log2` kernels, each
narrowed alone, with the ln 2 product inside the `FMA.v2f16`. `pow_vec4` does
all four `log2`s, then the second varying's load, then four `FMA_RSCALE.f32`,
then four `F32_TO_S32`/`FEXP.f32` pairs. Each result is narrowed alone and
paired with `MKVEC.v2i16`. The first load of `pow_vec4` does not end with
`r61^`: the second load reuses `r61`.

**AR2, vertex vec3/vec4 kernels** (`vswide_*`, `vs.varying`). One
`LD_ATTR_IMM.v3/v4.f32` of the attribute. The fp32 kernels of arith step 2
run in the narrowing groups of the AR1 table: vec3 `a_in0.zxy` is in0.x
alone, then (in0.y, in0.z); vec4 `a_in0.yzwx` is (in0.x, in0.w), then
(in0.y, in0.z). The sin/cos reduction's two constants sit in different FAU
slots here (`u5.w1` = 2/pi, `u6.w0` = bias), unlike the scalar reference
(`u2.w0`, `u2.w1`), so the bias is moved once to a register
(`MOV.i32.wait0 r3, u6.w0`) and every lane's first
`FMA.f32 x, u5.w1, r3` reads it. Then come `MKVEC.v2i16` into the colour
words, the `FMA.v2f16` scale and bias, four `F16_TO_F32` widenings and the
varying `STORE`s. `tan_vec3` interleaves its three reductions before the
table reads, as `tan_vec4` does, and `FRCP.f16` each narrowed cosine.
`exp2_vec4` is four `FMA_RSCALE.f32 x, 1.0, -0, r4` (r4 = `u5.w1`, moved
once), `F32_TO_S32` and `FEXP.f32`. Its template writes the result without
`* 0.5 + 0.5` and it is stored at fp32 without narrowing. `log2_vec4` is
four `log2` kernels, also stored at fp32. The store offsets come from
`IADD_IMM.i32 #0x4/#0x8/#0xC` (`exp2_vec4`) or FAU words (`u4.w0`, `u4.w1`,
`u5.w0`).

**AR3, `FRCP` of a computed mediump lane** (`fsrcp_prod`).
`V2F32_TO_V2F16` of the two loaded words, `FMA.v2f16 r0, r0.h00, r0.h11,
0x0.neg` (the product as a pair op), `FRCP.f16 r0, r0^.h00`, `U16_TO_U32`, then
the template's `FMA.v2f16`. `fsrcp_diff` (`FADD.v2f16 r0.h00, r0.neg.h11`, then
the same) is byte-equal today through `smoothstep_den`. AR3 generalises that
case to any computed fp16 lane.

**AR4, highp `mat3` division in a fragment shader** (`matdiv_*`). Both are
fp32 on FAU words and narrowed at the colour. `mat3 / float`: one
`FRCP.f32.discard r0, u6.w1` of the scalar. The column sum
`res[0] + res[1] + res[2]` is **factored over the reciprocal**: three
`FADD.f32` chains add the matrix words first (`u2.w0 + u2.w1`, then
`+ u5.w0`, ...), then come three `FMA.f32 r, rcp, sum, 0x0.neg`. Three
`FADD.f32 .h0` narrow the lanes, and two `MKVEC.v2i16` pair them with the
constant alpha (`u7.w0.h0`). This is `nir_opt_algebraic`'s
`a*b + a*c -> a*(b + c)`, not a per-element division. `mat3 / mat3`: nine
`FRCP.f32` of the divisor's words, interleaved with the `FMA.f32` chains
of the column sums (`FMA.f32 r0, u4.w0, rcp, 0x0.neg`, then
`FMA.f32 r0, u2.w1, rcp', r0`, ...), so each sum is a fused chain. The order
of the words in the chains follows the reassociated sum, not the columns.

## 4. Steps for the new forms

Each step lifts its references in `tests/mali_deqp_arithres.rs` (moves them to
`LOWERED`, drops the `#[ignore]`) in the commit that lowers them. All of them
edit `src/shaderc/online/mali/**`, which plan items mali-train-3b, -3c and -4
hold. These steps go after those trains, or into train 4 beside its arith
rows. The forecast is the cases of section 2 that leave `Arithmetic`. It is an
upper bound: a random program finishes only when every op in it is
lowered.

1. **AR1: fragment vector kernels** (31 cases, 7 references). Site:
   `nir_fs.rs` `vectorise` (`has_raw_transc` beside a non-constant tree),
   and the `Fsin|Fcos|Flog2` arm, which already accepts a varying's lane.
   1. Let `vectorise` accept two `Leaf::RawLane` kernels as a pair's two
      lanes when both are the same op on lanes of one varying load. Also
      accept a kernel beside another lane. Keep the scalar form of step 1b.
   2. Emit the kernels group by group in Mesa's narrowing groups (section 3's
      table: vec2 lane by lane, vec3 x then (y, z), vec4 (x, w) then (y, z)).
      Narrow a lone lane with `FADD.f32 .h0` and a group with
      `V2F32_TO_V2F16`. Then re-pair the halves into the colour words with
      `MKVEC.v2i16`. The groups are not the colour words, so the existing
      pair builder's "one colour word, one pair" assumption does not hold
      here.
   3. `tan`: all lanes' reductions, then the table pairs, then
      `V2F32_TO_V2F16` of the sines and the cosines, then `FRCP.f16` per
      cosine half, with the quotient reassociated into the scale's
      `FMA.v2f16` (`fsvec_tan_vec4`). `log`: the ln 2 product after the narrowing.
      `pow`: all `log2`s, the second load, then the `exp2` halves
      (`fsvec_pow_vec4`).
   4. Byte-compare: `fsvec_*` (7). Re-run `mali_deqp_arith.rs` (step 1b's
      scalar pins) and `mali_deqp_highpfs.rs` (HF1 takes the highp vectors
      F6 through the same `vectorise`; land HF1 first or together, because
      both edit the same check).
2. **AR2: vertex vec3/vec4 kernels** (27 cases, 6 references). Site:
   `nir.rs` walk, the `wide` refusal (`n > 2` for sin/cos, `n > 3` for
   exp2/log2).
   1. Remove the width limit. Order the per-lane kernels by AR1's
      narrowing groups (vec3 x then (y, z), vec4 (x, w) then (y, z)).
   2. When the reduction's two FAU words are in different slots, move the
      bias to a register once (`MOV.i32 rN, u6.w0`) and read it in every
      lane. `exp2`'s scale word is moved the same way (`vswide_exp2_vec4`).
      Check whether `opt.rs`/`emit.rs` already does this for any
      two-slot `FMA`. It is the FAU one-slot rule, not a kernel rule.
   3. Narrow in the same groups (`V2F32_TO_V2F16`, a lone lane
      `FADD.f32 .h0`) and re-pair with `MKVEC.v2i16` (`vswide_sin_vec3`,
      `vswide_cos_vec4`). `tan`: interleaved reductions and `FRCP.f16` per
      narrowed cosine (`vswide_tan_vec3`).
   4. Byte-compare: `vswide_*` (6). The mediump `exp`/`log` vec4 cases also
      need arith step 6's fp16 product (V12/V13) before they are
      byte-equal.
3. **AR3: `FRCP.f16` of a computed mediump lane** (9 cases, 1 reference
   plus the boundary pin). Site: `nir_fs.rs` `Frcp` arm (the
   `input_lane`/`uniform_dword`/`smoothstep_den`/`tan` test). Accept any
   lane `f.is_f16` computes and emit `FRCP.f16 r, x.h00` on the pair op's
   result (`fsrcp_prod`). `fsrcp_diff` must stay byte-equal. The random
   cases (`atan(vec4(..) * a)`) then reach step 7's `Bcsel` (F4) next. Do
   not count them as passes.
4. **AR4: highp `mat3` division** (4 cases, 2 references). Site:
   `nir_fs.rs` `Frcp` arm, reached through the highp island (`Fs::hp32`)
   for a `mat3`. `mat2`/`mat4` (`deqp_arith_matrix_div_fs_highp`) are
   byte-equal today.
   1. Allow `FRCP.f32` of a highp uniform word in `hp32`.
   2. Factor a shared reciprocal out of a sum of products
      (`a*r + b*r -> (a + b)*r`) before the walk, as Mesa's algebraic pass
      does (`matdiv_hp_mat3_float`).
   3. For `mat3 / mat3`, fuse each column sum into `FMA.f32` chains in
      the reassociated order (`matdiv_hp_mat3_mat3`).
   4. The `dynamic_*` cases read `attribute highp mat3`. `gl_ref.py` cannot
      bind matrix columns (`README-deqp_ref.md`), so they have no
      reference. Their fragment words differ from the uniform ones only in
      the varying load.

## 5. What this note does not decide

- The owned rows are not re-designed. Section 2 only records that a design
  and a pinning reference exist at the same site. Where a design describes
  the shape loosely, section 2 names the step that comes closest: F5 and F8
  (1b's random half), F9 and F11 (step 3's operand rules), and V13 (step 6).
- V7's 117 random cases need the `mesa_fold.rs` helper to give Mesa's bits
  for every kernel of a constant. `mali-libm-fold.md` owns that. No
  reference was added here.
- The host counts are first refusals on the T6 slice cases (10,597). They
  are not the G310's 886/623 lines, which count refused compiles. The next
  iMX95 or NanoPC-T6 rerun re-ranks them.
