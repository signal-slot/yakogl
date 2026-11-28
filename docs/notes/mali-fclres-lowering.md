<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `gl_FragColor` lane refusal residue after fclane-lower-1: shapes, Mesa references, lowering steps

**HOST-ONLY design.** Plan #387 item `fclres-refs` (issue #7, ROADMAP stage 5).
No change under `src/` is part of it. It continues
[`mali-fclane-lowering.md`](mali-fclane-lowering.md) (mechanisms A to E, the
first design) and [`mali-deqp-fclane.md`](mali-deqp-fclane.md) (the first 20
references), after `fclane-lower-1` landed mechanism A, B and part of D
(`tests/mali_deqp_fclane.rs` `LOWERED`: `mat_cols_sum`, `mat_elem_pairs`,
`mat_vec_lanes`, `op_pair_{xyz,xyzw,yz}`, `struct_fields`, `tex_coord_ramp`,
`tex_zw_lane`, `varying_merged`). Every count here is a host first-refusal
count (compile and link outcomes of `deqp-gles2` against the Mali C pair),
never a render verdict. The G310 needs no separate work
([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)): the references are the G610
drm-shim's.

The refusal text is `MaliError::ColourForm` (`src/shaderc/online/mali/mod.rs:538-540`):
"Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet".

## 1. What was measured, and how

- Cases: the 5,677 dEQP-GLES2 cases with a refusal in the Verdin iMX95
  (Mali-G310) whole-tree run's slices `tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e`
  (`refusals.tsv`: 818 cases with this text at `e63ab28e`; the 946 lines the
  plan quotes are the later `a3d8d2de` run, whose slices are not in the tree).
- Tip: the integration tip `1e1380b1`. `capi/Cargo.toml` there has the
  feature key `vivante-st` twice (lines 41 and 85, from `75a7d901` and
  `110080eb`), so no cargo command loads the workspace; the second line was
  removed in the working tree for the measure and the test run, not committed
  (see the item's report).
- Command (4 cores, CTS `93bca018` with `external/fetch_sources.py` and
  `tools/cts/cts-yakogl-target.patch`, as `mali-deqp-families.md` section 19):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" \
    --jobs 4 --chunk 100 --cases <the 5,677 cases in mustpass order> \
    --slice tools/cts/log/2026-11-24-imx95-gles2-S1a-e63ab28e ...  # all six slices
```

- Result: 4,685 of the 5,677 still meet a refusal on the host; **943 meet this
  text first** (938 Fail and 5 NotSupported by the G310 board's status). 922
  are a fragment compile refusal, 21 a link refusal (the relinked fragment
  shader, section 3.10).
- Refusal sites: every `Err(MaliError::ColourForm)` in `nir_fs.rs` was routed
  through an `eprintln!("CF@{}", line!())` and, at the three sites that fire
  most, the two operands (or the lane) were printed as a small tree
  (`alu_of`, `input_lane`, `uniform_dword`, three levels). The 624 distinct
  refused programs (922 cases) and the 19 relinked ones (21 cases) were
  compiled through `tests/mali_ref::compile_pair` in a scratch test. Exactly
  one site fires per program. The instrument and the scratch test were
  removed; `git diff src` is empty. Line numbers are those of `1e1380b1`.

| Site (`nir_fs.rs` at `1e1380b1`) | What it is | Cases |
|---|---|---:|
| **11057** | `vectorise`, the `else` of two load lanes that are no aligned pair, swapped pair, lane twice, whole-load pair or merged `w` (the old 8735) | 182 + 13 link |
| **11179** | `vectorise`, the final `_` arm of `match (input_lane(a), input_lane(b), a, b)`: a pair whose halves are not two load lanes (the old 8799) | 427 |
| **12053** | `lane_tree`'s last arm: a one-lane value the tree has no arm for (the old 9525) | 301 |
| **12641** | `select`, the conversion-form check: a varying lane converted both alone and as half of a pair (`conv_form`) | 10 + 8 link |
| 1841 | `Fs::op`: an operand is `Val::Undef` | 1 |
| 2354 | `Fs::abs`: `fabs` of `Val::Undef` | 1 |

## 2. The shapes, with their cases, groups and references

`L` is a varying lane, `U` a uniform word, `K` a constant. The reference
column names the pinned Mesa 26.2.1 references of the shape: **new** ones
are `tools/mali/testdata/deqp_fclres_*` (this item, `tests/mali_deqp_fclres.rs`),
the others were drawn by earlier items and are still refused with this text
at the tip.

| Shape | Site | Cases | dEQP groups (cases) | References |
|---|---|---:|---|---|
| **C** S4: a float-uniform compare chain `Bcsel(Fcmp(Lt)(Fmax/Fabs(..), 0.05), .., 0)` paired with `1.0` (the uniform_api check colour) | 11179 | 141 | uniform_api.value 120, info_query 18, random 3 | `fclane_bcast_compare_array`, `fclane2_struct_{array,arrmember,basic,nested}_float`, `fsunif_arr_mat4`, `fsunif3_random_80`, `uniarray_api_float`, `uniarray_struct`, `unif2_vs_mediump_{array,struct}` |
| **C** S9: `B2f` of a bool expression of uniforms the walk keeps as nodes (`B2f(Lane)`, `Iand`/`Inot` chains, `Icmp(Eq)(Lane, K)`) | 12053 | 108 | uniform_api.value 82, random 13, info_query 12, constant_expressions 1 | `fclane_bcast_compare_{bool,int,float}`, `fclane2_struct_basic_int`, `intval4_uapi_{bool,bvec2,bvec3,bvec4,int,ivec2,ivec3,ivec4}_fs`, `intval5_c_{bvec2,bvec3,bvec4,ivec4}`, `fsunif2_multi_nested`, `fsunif3_random_{4,31}`, `unif2_vs_bool_uniform`, `unitype_fsmulti`, `arith_uniform_api_bool_compare_vs_none` |
| S6a: `(0.0, computed lane)` of a mediump/lowp fp16 tree, the operator harness's `vec2` results | 11179 | 116 | shaders.operator 116 (binary 56, common_functions 35, unary 14, ...; 16 of them have an `exp`/`mod`/`sqrt`/`inversesqrt` lane: ARITH next) | **new** `k0_sub_vec2_mp`, `k0_mul_vec2_lp`, `k0_min_vec2_mp`; highp: `highpfs_pt_add_vec2`, `highpfs_pt_add_float_vec2`, `highpfs_pt_clamp_vec2_float_float`, `highpfs_hp_exp2_vec2` |
| S1: two lanes of one load that are no pair (`(y, z)` of a `.yzwx` operand, a `vec4` read back through a subscript) | 11057 | 145 | shaders.operator 105 (binary 48, common_functions 27, unary 20, ...), shaders.indexing 26, random 5, struct 4, matrix 4, fragment_ops 1 | **new** `swz_add_vec4_lp`, `swz_inc_vec4_mp`, `swz_max_vec4_mp`, `matsub_loop`, `vecsub_loop`; `fclane_vec3_local`, `highpfs_pt_{add,mul,max}_vec4`, `highpfs_pt_add_vec4_float`, `idxloc_fs_mat3_dl_sl` |
| **C** S5: `B2f` of int/bool uniform word compares (`Icmp(Eq)(U, K)`, `Iand` of them) | 12053 | 97 | uniform_api.value 83, info_query 7, random 6, constant_expressions 1 | `fclane_bcast_const`, `fsunif_arr_{bvec4,ivec4}`, `fsunif2_multi_basic_array`, `fsunif3_random_{34,62,93}`, `unif2_arr_{bool,int}_array`, `unif2_fs_{bool,int}_uniform` |
| **C** S3: a lane that is `Bcsel(Lane, 0.0, 1.0)` (a bool member read as a select) | 12053 | 82 | uniform_api.value 68, info_query 11, random 3 | `fclane2_struct_basic_bool`, `fsunif3_random_9`, `fsunif_struct_int_ivec4`, `unif2_fs_struct_uniform` |
| S6b/S6c: `(K, computed lane)` outside the operator harness; `K` is 0.0 (13) or another constant (28, e.g. `-1.5`, `0.75`) with `Fneg`/`Fadd`/`Fmul`/`I2f` lanes, some of uniforms | 11179 | 41 | shaders.random 41 | **new** `kneg_random`; `intrnd_dead_ivec_uniform_fs`, `misc2_algebraic_random46`, `intval_rsg_cond_fs` |
| S8: both halves the same one-lane op the pair graph has no pair arm for: `Frcp` 12, `Ffloor` 11, `F2f32` 6, `Fexp2` 5, `Fceil` 4, `B2f` of a float compare 4, `Flog2` 1 | 11179 | 43 | shaders.operator 29, random 10, matrix 4 | **new** `div_vec3_lp`, `mod_vec3_mp`, `f2f32_swizzle`, `exp_vec4_mp`, `ceil_vec4_mp`, `sign_vec4_mp`; `highpfs_hp_mod_vec3`, `gm2_fsarith2_1`, `gm2_fsarith_7`, `gm2_nbshrf_{3,5}`; `intval5_c_int_{k,lt}` (int compares) |
| S2: two lanes of *two* loads: the columns of a `mat2` (and a `vec2` with a `mat2`) the link keeps as separate varyings | 11057 | 37 | shaders.matrix 34, random 2, fragment_ops 1 | **new** `mat2_add_mp`, `vec2_mat2_mul_mp` |
| S7: the two lanes of one two-lane node | 11179 | 30 | shaders.operator 28 (sequence 11, unary 6, common_functions 5, ...), struct 2 | `fclane_op_pair_other`, `arith_op_common_smoothstep_fs_highp`, `highpfs_hp_smoothstep_vec3`, `intval3_opfs_sequence_bool_vec2_mediump` |
| S11a: an unfolded constant matrix product (`Ffma(K, K, Ffma(K, K, K))`) paired with `Undef` | 11179 | 17 | shaders.matrix.mul 15, mul_assign 2 | **new** `mat3_mul_const_mp` |
| S11b: two lanes of one matrix column of a `highp mat3` the link keeps whole (`(in1.0, in1.1)`) and a `vec4 * mat4` (`(in1.0, in2.0)`) | 11179 | 7 | shaders.matrix 7 | **new** `mat3_add_hp` |
| S11c: `(varying lane, I2f(int expression))`: `I2f(Iadd(U, U))`, `I2f(Isub(..))`, `I2f(F2i(..))` | 11179 | 7 | shaders.struct 4, random 2, fragment_ops 1 | **new** `struct_local_int` |
| S11d: other mixed pairs (`Fmul`/`Fadd`, `(L, Fadd)`, `(L, Fmul)`, `Fexp2` with `0.0`, `F2f32` with an op, `I2f` with `B2f`) | 11179 | 25 | random 15, matrix 4, operator 3, struct 2, fragment_ops 1 | `fclane_rnd_vec4`, `highpfs_hp_exp2_float`, `fsvec_pow_mediump` (no new reference: one-off arrangements of the random generator) |
| S14a (link): the relinked fragment shader of a vertex shader that writes one value to two lanes (`v_color = vec4(res.x, res.y, res.x, 1.0)`) | 11057 | 13 | shaders.matrix.mul `*_vertex` 12, random 1 | **new** `link_dup_mat2` |
| S13 + S14b: a lane converted alone and as half of a pair (`conv_form`), in the fragment shader (10) and in the relinked one (8: `v_color = vec4(b, a, float(a), 1.0)`) | 12641 | 18 | shaders.random 18 | **new** `link_conv_random`; `mixcol_ck_rnd_other`, `mixcol_lz_rnd` |
| S10: `I2f` of an int node in `lane_tree` | 12053 | 9 | random 6, linkage 3 | none (`intval` family) |
| S12: `F2f16` of a varying lane or of an `I2f` in `lane_tree` | 12053 | 5 | operator 2, random 3 | `mixprec3_hp2_sum` |
| `Undef` operands | 1841, 2354 | 2 | functions.overloading 1, random.all_features 1 | none |
| | | **943** | | 22 new, 66 earlier |

141 + 108 + 116 + 145 + 97 + 82 + 41 + 43 + 37 + 30 + 17 + 7 + 7 + 25 + 13 + 18 + 9 + 5 + 2 = 943.

The mechanism-C shapes (S3, S4, S5, S9: **428 cases**, all `uniform_api`) are
mechanism C of the first design (`F2C`, owner `fclane-lower-3`, todo); their
references exist (66 of them in all, most drawn by the unif2, fsunif, intval4,
intval5 and fclane2 items). This item drew references only for the shapes
that had none, or none at their precision.

## 3. Mesa's words per new reference

`tests/mali_deqp_fclres.rs` pins all 22 as refused with the text above
(`REFUSED`; `references_parse` checks each `fs.fragment` word count in
`FS_WORDS`). The words are the `text` of each `expect.json`'s `fs.fragment`
region. Common frame (as in the first design's section 1): `ATEST` of the
alpha half (or `0x3F800000`), `BLEND ... @r0:r1`, the `IADD_IMM`/`BRANCHZI`
tail, `work_reg_count` 32. "Narrow" is `FADD.f32 rX.h0, rY^, 0x0.neg`, "pair
convert" `V2F32_TO_V2F16`.

### 3.1 S6a: `(0.0, computed lane)` of a fp16 tree (`k0_*`)

`k0_sub_vec2_mp` (`in0 -= in1`, mediump vec2, `color.xy`-style harness with
the colour `(0, res.x', res.y', 1)` after the shuffle), 18 words:

```
LD_VAR .v4 @r0:r3 (in0); narrow r0.h0 <- r1 (y); narrow r1.h0 <- r3 (w)
LD_VAR .v4 @r2:r5 (in1); V2F32_TO_V2F16 r2, r2^, r3^
FADD.v2f16 r1, r1^.h00, r2.neg.h11        (lane w - in1.y, as a broadcast half)
FADD.v2f16 r0, r0^.h00, r2^.neg.h00       (lane y - in1.x)
MKVEC.v2i16 r1, 0x0.h0, r1^.h0            <- the (0.0, lane) word
FMA.v2f16 r1, r1^, 0x34000000.h11, 0x38000000.h11
MKVEC.v2i16 r0, r0^.h0, u2.w0.h0          (lane, alpha 1.0 from the FAU)
FMA.v2f16 r0, r0^, 0x34000000.h11, 0x38000000.h11
ATEST r0.h1; MOV.i32 x3 (swap the words into r0:r1)
```

`k0_mul_vec2_lp` is the same with `FMA.v2f16 .. r2.h11, 0x0.neg` (the product)
and the `0.5` scale; `k0_min_vec2_mp` (15 words) has `FMIN.v2f16 r1, r1^.h00, r0.h00`
over three narrowed lanes and no `MOV` swap (its words are already in
`r0:r1`). Points: (1) the single lanes are narrowed alone and the operation
is a `.v2f16` op on `.h00`/`.h11` broadcast halves, (2) the constant 0.0 is
the **low** half (`0x0.h0` first), (3) the post-step `FMA.v2f16` runs on the
packed word.

### 3.2 S6c: `(K, -uniform)` (`kneg_random`, 12 words)

```
MOV.i32 r0, u2.w0; MKVEC.v2i16 r0, u3.w0.h0, r0^.h0   (word = (K from the FAU, uniform))
MOV.i32 r1, u2.w1; MKVEC.v2i16 r1, u3.w1.h0, r1^.h0
FADD.v2f16 r0, u4.w0, r0^.neg; FADD.v2f16 r1, u4.w1, r1^.neg
FMA.v2f16 r0, u5.w0, r0^, 0x0.neg; FMA.v2f16 r1, 0x34000000.h11, r1^, 0x0.neg
```

The constants are fp16 pairs in the FAU (`3e003e00`, `c7803800`, ...), the
uniform's 16-bit value the high half; the negation folds into the `FADD`'s
`.neg` operand (the uniform is `Fneg(U)` in our graph).

### 3.3 S1: `(y, z)` of one load (`swz_*`, `matsub_loop`, `vecsub_loop`)

`swz_inc_vec4_mp` (`++in0` of `in0 = v_in0.yzwx`), 17 words:

```
LD_VAR .v4 @r0:r3; V2F32_TO_V2F16 r0, r0^, r1^ (x,y); V2F32_TO_V2F16 r1, r2^, r3^ (z,w)
FADD.v2f16 r0, r0^.h10, 0x3C000000.h11     (the swizzled word (y,x) + 1)
FADD_IMM.v2f16 r1, r1^, #0x3C003C00        ((z,w) + 1)
MKVEC.v2i16 r2, r0.h0, r1.h0               (y', z')
FMA.v2f16 r2, r2^, 0x38000000.h11, 0x0.neg
MKVEC.v2i16 r0, r1^.h1, r0^.h1             (w', x')
FMA.v2f16 r0, r0^, 0x38000000.h11, 0x0.neg
ATEST r0.h1; MOV.i32 x4
```

The operation runs on the **converted pairs, swizzled** (`.h10`), and the
colour's words are re-paired afterwards by two `MKVEC.v2i16` (low halves,
high halves). `swz_add_vec4_lp`/`swz_max_vec4_mp` (15 words each) are the same
with a narrowed scalar `in1` read as `.h00`. `matsub_loop` (22 words) is the
`fclane_vec3_local` arrangement twice (`FMA.v2f16 r2, r0.h10, ..` then
`MKVEC r4, r3.h1, r2.h1`, `MKVEC r2, r2^.h0, r3^.h0`). `vecsub_loop` (14 words)
reduces to **one** lane, broadcast as mechanism C's `IADD.v2u16 r1, r0.h00, 0x0`
and `MKVEC.v2i16 r2, r0^.h0, u2.w1.h0`. This is the first design's mechanism
D (`Leaf::Straddle`, step F2D), now with seven more references.

### 3.4 S8: a pair of the same one-lane op

- `div_vec3_lp` (16): `FRCP.f16 r0, r0^.h00` **once** for the scalar divisor,
  then `FMA.v2f16 r1, r1^, r0.h00, 0x0.neg` (pair) and `FMA.v2f16 r0, r2^.h00,
  r0^.h00, 0x0.neg` (lone lane): `x / s` is `x * rcp(s)` with one `FRCP` shared.
- `mod_vec3_mp` (32): three `FRCP.f16` (one per lane), `FMA.v2f16` products
  re-paired by `MKVEC`, `FROUND.v2f16.rtn` (floor) on the pair and on the lone
  lane, then `x - y * floor(..)` as `FMA.v2f16 .. neg ..` / `FADD.v2f16 .. neg`.
- `ceil_vec4_mp` (17): `FROUND.v2f16.rtp r0, r0^.h10` / `FROUND.v2f16.rtp r1, r1^`
  on the two converted pairs (the `.yzwx` swizzle as `.h10`), then the S1
  re-pairing.
- `exp_vec4_mp` (30): the `* log2(e)` product at fp16 (`FMA.v2f16 .., u2.w0`
  with `3dc53dc5`), then **widened** per lane (`F16_TO_F32` x4), the fp32
  `exp2` sequence per lane (`FMA_RSCALE.f32`, `F32_TO_S32`, `FEXP.f32`), then
  two pair converts: the arith family's `exp2` lowering (`mali-arith-lowering.md`),
  per lane, inside a fp16 tree.
- `sign_vec4_mp` (26): `FCMP_OR.v2f16.lt.m1` per pair, both directions
  (`0 < x`, `x < 0`), `MKVEC` re-pairing of the masks, `MUX.v2i16 r, 0x0,
  0x3C000000.h11, mask` (mask to 0.0/1.0), then `FADD.v2f16 a, b.neg`
  (`sign = (0<x) - (x<0)`).
- `f2f32_swizzle` (17): the `F2f32` lanes fold away in Mesa (mediump
  arithmetic stays fp16: `MKVEC.v2i16 r0, u3.w0.h0, r0^.h0`, `FMA.v2f16`,
  `FADD.v2f16 .. neg`); one lane is an `int` uniform (`S32_TO_F32 r1, u2.w1`,
  `FADD_IMM.f32`, narrow, `MKVEC` with an FAU constant).

### 3.5 S2: lanes of two loads (`mat2_add_mp` 12, `vec2_mat2_mul_mp` 17)

Mesa's link packs the two `vec2` columns of the `mat2` varying into **one**
slot: one `LD_VAR_BUF_IMM.f32.slot0.v4 @r0:r3, index 0x0`, two pair converts,
the operation per word (`FADD.v2f16 r0, r0^, u2.w0`), then
`FADD.v2f16 r2, r0.h11, r1.h11` and `MKVEC.v2i16 r0, r0^.h0, r1^.h0` /
`MKVEC.v2i16 r1, r2^.h0, u3.w0.h0`. Our walk sees two loads (`in0.0`, `in1.0`),
so the refusal is a varying-layout difference before it is a lane one: the
vertex side's `vs.varying` (16 / 25 words) stores both columns into one slot.

### 3.6 S11a: a constant matrix product (`mat3_mul_const_mp`, 7 words)

```
NOP.wait0126; ATEST 0x3F800000; MOV.i32 r0, u2.w0; MOV.i32 r1, u2.w1; BLEND ...
```

Mesa folds the whole `const mat3 * const mat3` and the column sum to two fp16
pair constants in the FAU (`3bad3d71`, `3c003a67`). Our walk keeps
`Ffma(1, 0.2, Ffma(0, 1.2, 0))` and pairs it with `Undef`: this is a constant
fold gap (`nir::fold`/`mesa_fold.rs`), not a lane form.

### 3.7 S11b: a `highp mat3` added in fp32 (`mat3_add_hp`, 27 words)

Mesa packs the nine floats into `v4`, `v4`, `.x` (indexes 0x0, 0x10, 0x20),
adds the uniform matrix per lane at fp32 (`FADD.f32 rN, rN^, uK.wJ`, 9), sums
the columns at fp32 (6 `FADD.f32`), narrows three lanes alone and packs
`MKVEC.v2i16 r0, r0^.h0, r1^.h0` / `MKVEC.v2i16 r1, r2^.h0, u6.w1.h0`:
mechanism A (step F1, landed) once the three-column load layout is Mesa's.

### 3.8 S11c: `(lane, float(int + int))` (`struct_local_int`, 12 words)

```
LD_VAR .v3; V2F32_TO_V2F16 r0, r0^, r1^
IADD.s32 r1, u2.w0, u2.w1; S32_TO_F32 r1, r1^; narrow r1.h0
narrow r2.h0 <- r2 (z); MKVEC.v2i16 r2, r2^.h0, r1.h0
ATEST r1^.h0; MOV.i32 r1, r2^
```

Mechanism B's `struct_fields` word (`(z, float(c))`, lowered), with the
integer operand an `Iadd` of two uniforms instead of one uniform.

### 3.9 S14a: the relinked fragment shader of a duplicated vertex output (`link_dup_mat2`, 7 words)

Mesa keeps all four lanes of `v_color` (no deduplication of `res.x` into
`x` and `z`): `LD_VAR .v4`, two pair converts, `ATEST r1.h1`, `BLEND`. The
vertex side (`vs.varying`, 21 words) stores `res.x` twice (`STORE.i32 ...
slot2 @r1` at two offsets). Our link merges `z` with `x` and the plain
`gl_FragColor = v_color` then meets two load lanes that are no pair: the
merge itself is the difference (`mali-deqp-varylink.md`'s `VaryingDuplicate`
path decides when a duplicate is merged; Mesa does not merge this one).

### 3.10 S14b: a lane read alone and as a pair half (`link_conv_random`, 7 words)

`v_color = vec4(b, a, float(a), 1.0)`: Mesa compacts the varying to two
lanes (`LD_VAR .v2`), one pair convert, and builds word 1 as
`MKVEC.v2i16 r1, r0.h1, u2.w0.h0` (z is the converted pair's **high half**,
the alpha 1.0 from the FAU). Our `select`'s `conv_form` check refuses a lane
converted both as a pair half and alone (12641).

## 4. Forecast per step (HOST-ONLY first-refusal cases)

The forecast is the number of cases whose **first** refusal is this text and
whose shape the step removes; a lifted case may stop at its next refusal
(ARITH for `exp`/`mod`/`sqrt`/`inversesqrt` lanes, intval for int lanes), so
it is an upper bound of new compiles, not of passes. Re-measure with
section 1's command after each step (`--cases` with the 943 cases is enough:
a few minutes on 4 cores; the 5,677 took about 15).

| Step | Shapes | Cases | Next refusal expected |
|---|---|---:|---|
| R1 | S6a, S6b, S6c | 157 | possibly ARITH for the 17 whose lane is `Fexp2`/`Ffma` (mod)/`Frcp`/`Frsq` (they reached the colour, so the walk accepted the op; their pair emission is not measured) |
| R2 | S1 | 145 | possibly ARITH for the `mix`/`smoothstep`/`reflect`/`distance` cases (15, by case name; not measured) |
| (F2C, `fclane-lower-3`) | S3, S4, S5, S9 | 428 | the intval masks (R1 steps 1-2 of the schedule) |
| R3 | S8 | 43 | ARITH for `Fexp2`/`Flog2` (6) until the arith lane forms are in pairs |
| R4 | S2, S11b, S14a | 57 | VaryingForm/varylink if the vertex packing is not Mesa's |
| R5 | S11a | 17 | none known |
| R6 | S11c, S13, S14b | 25 | intval for `I2f` of int expressions |
| (left) | S7 30 (owned by fclane E / arith), S11d 25, S10 9, S12 5, `Undef` 2 | 71 | |
| | | **943** | |

## 5. Proposed lowering steps, in order

Order: by cases, then by what each needs. Re-run section 1's instrument at
the start of each step (the lines move as other items land in `nir_fs.rs`).
Each step moves its references from `REFUSED` to `LOWERED` in
`tests/mali_deqp_fclres.rs` (and the earlier references of the shape in
their own files) in the same commit; a shape Mesa compiles differently stays
refused, it does not become `DIFFERS`.

1. **R1, `(K, computed lane)`** (`vectorise`, final arm at 11179, and its
   twin `(Some(op), None)` arm for `(lane, K)`): add the mirrored arm
   `(Const c, computed lane)`: the lane goes through `lane_tree` (its fp16
   operation on `.h00`/`.h11` broadcast halves of the converted operands,
   section 3.1) and the word is `MKVEC.v2i16 w, 0x0.h0, x.h0` for 0.0 or
   `MKVEC.v2i16 w, uK.wJ.h0, x.h0` with the constant's fp16 pair in the FAU
   (section 3.2: Mesa puts the constant pair in the FAU, not inline, when it
   is not 0.0). `Leaf::Lane { first: true }` / `Leaf::Mk { first: true }`
   already print the 0.0 and uniform forms for load lanes
   (`deqp_colourpair_ck_lane_highpcolour`); extend them to a computed node
   `x` (the node `lone_node` returns). `Fneg(U)` must fold into the consumer's
   `.neg` (section 3.2's `FADD.v2f16 r0, u4.w0, r0^.neg`), not become a word
   of its own. References: `k0_sub_vec2_mp`, `k0_mul_vec2_lp`, `k0_min_vec2_mp`,
   `kneg_random`; also check `highpfs_pt_add_vec2` and its siblings (highp:
   the lane is fp32, narrowed: mechanism A's form) and `intrnd_dead_ivec_uniform_fs`,
   `misc2_algebraic_random46`.
2. **R2, two lanes of one load that are no pair** (11057's `else`): operate
   on the converted pairs with the swizzle (`.h10`) and re-pair the results
   with `MKVEC.v2i16 lo.h0, hi.h0` / `lo.h1, hi.h1` (section 3.3). This is
   F2D's `Leaf::Straddle` of `mali-fclane-lowering.md` 5.2 item 2; the seven
   new references (`swz_*`, `matsub_loop`, `vecsub_loop`) plus `fclane_vec3_local`
   and `highpfs_pt_*_vec4` pin it. `vecsub_loop` additionally needs C's
   broadcast `IADD.v2u16 r1, r0.h00, 0x0` (do it after F2C or leave it
   refused). Order inside: the unary `swz_inc_vec4_mp` (no second operand)
   first, then the scalar-operand `swz_add_vec4_lp`/`swz_max_vec4_mp`, then
   `matsub_loop`.
3. **F2C** (`fclane-lower-3`, not this note's): S3/S4/S5/S9, 428 cases, with
   the existing references. It is the largest residue; it waits for the intval
   mask lowerings (schedule R1 steps 1-2), as the first design says.
4. **R3, pairs of one-lane ops** (11179): give the pair graph a pair arm for
   `Fceil`/`Ffloor` (`FROUND.v2f16.rtp`/`.rtn`), `Frcp` (`FRCP.f16` per lane,
   **shared** when both lanes read the same scalar: `div_vec3_lp`), `B2f` of a
   float compare (`FCMP_OR.v2f16.lt.m1` + `MUX.v2i16 0x0, 0x3C000000.h11`:
   `sign_vec4_mp`), and the `F2f32` that Mesa folds away (`f2f32_swizzle`).
   `Fexp2`/`Flog2` (`exp_vec4_mp`) need the arith fp32 kernels per lane with
   `F16_TO_F32` widening: after the arith item that owns `exp2`.
5. **R4, varying layout**: pack the columns of a `mat2`/`mat3` varying into
   consecutive components as Mesa does (`mat2_add_mp`, `vec2_mat2_mul_mp`:
   one `.v4`; `mat3_add_hp`: `.v4`, `.v4`, `.x`) on **both** sides (the
   vertex `vs.varying` words are in the references), and do not merge a
   duplicated vertex output that Mesa keeps (`link_dup_mat2`). This touches
   the varylink code, not `vectorise`: coordinate with the varylink owners;
   after it, the colour is mechanism A/B, already lowered.
6. **R5, constant matrix products**: fold `const mat * const mat` (and the
   column sum) to constants before the colour (`mat3_mul_const_mp`: two FAU
   pair words); the `nir::fold` / `mesa_fold.rs` path, with Mesa's bits.
7. **R6, mixed halves**: `(lane, I2f(int expression))` as `struct_fields`'s
   word with the integer expression (`IADD.s32 r1, u2.w0, u2.w1`,
   `S32_TO_F32`, narrow: `struct_local_int`), and a lane read as a pair's high
   half (`MKVEC.v2i16 r1, r0.h1, ..`) instead of the `conv_form` refusal at
   12641 (`link_conv_random`, `mixcol_lz_rnd`, `mixcol_ck_rnd_other`).

Risks: steps 1 and 2 change `vectorise`, the function every colour passes;
run `tests/mali_deqp_fclane.rs`, `mali_deqp_colourpair.rs`,
`mali_deqp_highpfs.rs`, `mali_deqp_mixprec*.rs` and the glmark2 corpus tests
after each (the first design's section 6 lists the rest). Step 5 changes
vertex words: the vertex references of the varylink and matrix families must
stay byte-equal.

## 6. Files

- This note; references `tools/mali/testdata/deqp_fclres_*.{vert,frag,expect.json}`
  (22, each `.vert`/`.frag` with a `.license` naming its dEQP case);
  `tests/mali_deqp_fclres.rs`.
- Run: `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclres`.
- Not yet in a `tools/check.sh` row (the Mali family rows list each
  `tests/mali_deqp_*.rs` by name).
