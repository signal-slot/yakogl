<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `gl_FragColor` lane refusal in dEQP-GLES2: shapes, Mesa references, lowering map

**HOST-ONLY.** Plan #387 item `fclane-ref` (issue #7, ROADMAP stage 5). A
study plus references: no compiler change. Every case count in this note
comes from `deqp-gles2` run on a host against the Mali back end's C pair on
the Linux OSAL (`tools/mali/deqp-host-measure.sh`; method and limits in
`mali-deqp-families.md` sections 1 to 3): what is real is each shader's
compile and link outcome and our info log, never a render verdict. A board
status quoted here is the NanoPC-T6's own, from the committed slices.

The text: `Valhall: a gl_FragColor lane other than the varying's own lane or a
constant is not lowered yet` (`MaliError::ColourForm`,
`src/shaderc/online/mali/mod.rs`). It blocked 793 must-pass cases at
`abcc9013` (`mali-deqp-families.md` section 2, plan #367's text) and 1,768
refused compiles in the `de2f2c10` board run.

## 1. Method

- Tip `9ac31f30`; CTS VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
  (`tools/cts/deps.toml`) with `cts-yakogl-target.patch`, in
  `$CONNECTED_DEPS_DIR/VK-GL-CTS`.
- Measure (4 cores, about 5 minutes for the run itself, the host
  `deqp-gles2` build reused under `build/host-deqp-mali/`):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$CONNECTED_DEPS_DIR/fclane-scratch/measure" --jobs 3
```

  It runs the 10,597 refused cases of the four committed slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608`; 4,401 compile at the
  tip, 6,196 have a host refusal. **821 cases have this text as their first
  refusal** (`measure.tsv` column `host_family` = `367:FRAGCOLOR_LANE`; the
  mapping text to family is the script's `FAMILIES` table). All 821 are
  fragment-stage refusals; 820 are board Fail and 1 NotSupported
  (`shaders.matrix.*` and `uniform_api.*` as below). The partner
  shaders (the case's vertex shader) compile in every one of them: the
  refusal is the fragment shader's, on its own or joined with its vertex
  shader's constant lanes (`varying_merged`).
- The 821 are grouped into shapes by the colour source of the fragment
  shader's `gl_FragColor` (a script of the `.frag` sources in the measure's
  `src/`, not committed; the rules are the shape names' descriptions below).
  The counts sum to 821.
- Mesa 26.2.1's drm-shim, built the documented way into
  `$CONNECTED_DEPS_DIR/mesa-shim`
  (`tools/mali/drm_shim_ref.sh build "$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz" "$CONNECTED_DEPS_DIR/mesa-shim"`).
- Each reference (the pair verbatim from the case's `.qpa`, the vertex
  shader being the case's own) was made with:

```sh
MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
    <case>.vert <case>.frag tools/mali/testdata/deqp_fclane_<shape> --instrument shim
```

  which runs `drm_shim_ref.sh run <work> vs fs <prefix> --uniform-values`
  (`tools/mali/gl_ref.py` draws once through Mesa's panfrost GLES 2 driver on
  the G610 shim) and `pan_ref_diff.py extract <trace> --draw`. The G310 needs
  no separate reference (`mali-g310-vs-g610.md`).
- `tests/mali_deqp_fclane.rs` pins them. At the tip **all twenty are refused
  with exactly the colour-lane text** (`refusals_pinned`), so each byte-compare
  is `#[ignore]`d.

## 2. The shapes

A shape is named by what feeds the lanes of `gl_FragColor`. `Ref` is the
reference file `tools/mali/testdata/deqp_fclane_<ref>.*`; its pair is the
representative case. Counts are first-refusal cases at `9ac31f30`.

| # | Shape | Cases | Ref | dEQP groups (cases) | Representative case |
|---|---|---:|---|---|---|
| 1 | `op_pair_yz`: the operator harness `mediump vec4 color = vec4(0,0,0,1); color.yz = res; [color = color * s + b;]` of a `vec2` result; `res` computed from varyings' swizzles (`v_in0.wy`) | 112 | `op_pair_yz` | shaders.operator: binary 63, unary 21, common_functions 12, exponential 8, angle 4, geometric 4 | `shaders.operator.angle_and_trigonometry.degrees.highp_vec2_fragment` |
| 2 | `op_pair_xyzw`: the same with `color.xyzw = res` of a `vec4` result | 115 | `op_pair_xyzw` | shaders.operator: binary 57, unary 30, common_functions 12, sequence 6, angle 4, geometric 4 | `...degrees.highp_vec4_fragment` |
| 3 | `op_pair_xyz`: `color.xyz = res` of a `vec3` result | 49 | `op_pair_xyz` | shaders.operator: binary 24, unary 10, common_functions 6, angle 4, geometric 3, exponential 2 | `...degrees.highp_vec3_fragment` |
| 4 | `op_pair_x`: `color.x = res` of a `float` result | 8 | `op_pair_x` | shaders.operator: exponential 4, geometric 4 | `shaders.operator.exponential.exp.highp_float_fragment` |
| 5 | `op_pair_other`: the operator harness with another colour assignment (`sequence.side_effects.*`'s bool/vec2 lanes through a conversion) | 8 | `op_pair_other` | shaders.operator.sequence 8 | `shaders.operator.sequence.side_effects.highp_bool_vec2_fragment` |
| 6 | `bcast_compare`: `vec4(result, result, result, 1.0)` where `result = v_vtxOut * compare_<T>(u_var, <literal>)...`: one lane (a uniform compare product) in three words | 229 | `bcast_compare_{int,bool,float,array}` | uniform_api.value 188 (assigned 124, initial 64), uniform_api.info_query 27, uniform_api.random 14. By uniform kind: struct 94, bool/bvecN 61, int/ivecN 28, float array 27, sampler array 11, float 6, int array 2 | `uniform_api.info_query.basic.int_fragment` (int), `...basic.bool_fragment`, `...multiple_basic.fragment`, `...basic_array.vec4_fragment` |
| 7 | `mat_cols_sum`: `vec4(res[0]+res[1]+res[2], 1.0)` of a `mat3` of varying lanes (a sum of three columns) | 51 | `mat_cols_sum` | shaders.matrix: mul 10, add 6, sub 6, add_assign 3, matrixcompmult 3, negation 3 and the other operators | `shaders.matrix.add.dynamic_highp_mat3_float_fragment` |
| 8 | `mat_elem_pairs`: `vec4(res[0][0], res[0][1], res[1][0]+res[1][1], 1.0)` of a `mat2` (post_decrement: plus `tmpValue`) | 42 | `mat_elem_pairs` | shaders.matrix (add 6, sub 6, add_assign 3, matrixcompmult 3, mul 3, negation 3, ...) | `shaders.matrix.add.dynamic_highp_mat2_float_fragment` |
| 9 | `mat4_swz_sum`: `vec4(res[0].xyz + res[1].yzw + res[2].zwx + res[3].wxy, 1.0)` of a `mat4` | 23 | `mat4_swz_sum` | shaders.matrix (add 4, sub 4, mul 3, pre_decrement 3, pre_increment 3, mul_assign 2, ...) | `shaders.matrix.add.const_highp_mat4_float_fragment` |
| 10 | `mat_vec_lanes`: a vector-by-matrix product's lanes `vec4(res.x, res.y, res.x, 1.0)` / `res.z+res.w` | 6 | `mat_vec_lanes` | shaders.matrix.mul 6 | `shaders.matrix.mul.dynamic_highp_vec2_mat2_fragment` |
| 11 | `vec3_local`: a `vec3`/`vec4` local built by component or subscript writes, `vec4(vec3(res), 1.0)`, `vec4(res, 1.0)`, `vec4(res, 0.0, 1.0)`, `vec4(res)` | 52 | `vec3_local` | shaders.indexing: vector_subscript 24, matrix_subscript 12, tmp_array 2, varying_array 2 and other; shaders.matrix.mul 12 | `shaders.indexing.matrix_subscript.mat3_static_loop_write_static_loop_read_fragment` |
| 12 | `rnd_vec4`: the random-shader generator's vec4 expression of lanes (`vec4(b, float(a), ..)`, `a * gl_FragColor`, swizzles `.bgar`, int/bool conversions) | 66 | `rnd_vec4` | shaders.random: basic_expression 20, scalar_conversion 14, swizzle 9, conditionals 7, texture 6, comparison_ops 5 and others; fragment_ops.interaction 1 | `shaders.random.basic_expression.fragment.39` |
| 13 | `tex_coord_ramp`: `vec4(f0, f1, 1.0-f0, 1.0-f1)` of `x = v_coord.x; y = v_coord.y; f0 = (x+y)*0.5; f1 = 0.5+(x-y)*0.5` | 20 | `tex_coord_ramp` | texture.specification: basic_copyteximage2d 10, basic_copytexsubimage2d 10 | `texture.specification.basic_copyteximage2d.2d_alpha` |
| 14 | `varying_merged`: `gl_FragColor = v_color` where the vertex shader's `v_color` has two lanes of equal value (`color += vec4(a_0.xyz, a_0.x)`) and Mesa's link merges them | 18 | `varying_merged` | shaders.matrix.mul 12, attribute_location.bind 1, shaders.random 5 | `attribute_location.bind.vec3` |
| 15 | `struct_fields`: `vec4(s.a, s.b.x, s.b.y, s.c)` of a local struct (fields of a struct built from varyings and constants) | 14 | `struct_fields` | shaders.struct.local 12, shaders.struct.uniform 2 | `shaders.struct.local.basic_fragment` |
| 16 | `tex_zw_lane`: `gl_FragColor.xy = vec4(fTest).xy; gl_FragColor.zw = texture2D(...).zw` (two assignments, a convert of an `ivec4` uniform then a texel's lanes) | 4 | `tex_zw_lane` | negative_api.shader (`uniform_matrixfv_*`) 4 | `negative_api.shader.uniform_matrixfv_incompatible_type` |
| 17 | `bcast_const`: a broadcast of an integer constant-expression result or a parameter (`vec4(RES, RES, RES, 1.0)`) | 4 | `bcast_const` | shaders.constant_expressions.operators 2, shaders.functions.overloading 1, shaders.qualification_order 1 | `shaders.constant_expressions.operators.math_int_fragment` |
| | **Total** | **821** | 20 references | | |

Sums: 112+115+49+8+8 = 292 (the operator harness), 229, 51+42+23+6 = 122
(matrix), 52, 66, 20, 18, 14, 4, 4: 292+229+122+52+66+20+18+14+4+4 = 821.

Sub-shapes the representative does not separate (counts of the operator
harness by its colour post-step, `color = color * s + b` / `color * s` /
none): `op_pair_yz` 68 / 13 / 31, `op_pair_xyzw` 77 / 17 / 21,
`op_pair_xyz` 36 / 5 / 8, `op_pair_x` 4 / 2 / 2. The references chosen are
the scale-and-bias form (the largest) for the three vectors and the
scale-only `exp` for the float; the no-scale form is a smaller step of the
same lowering (`FMA.v2f16` absent).

No reference exists for 94 `bcast_compare` cases that read a `structType`
uniform (`gl_ref.py`'s `--uniform-values` "does not set a structType
uniform"; without it every uniform word is 0 and `mali_ref` compares the FAU
with the values), nor for the 11 sampler-array and 2 int-array ones; the
float-array reference stands for the array form. A step that lowers them
needs `gl_ref.py` to set struct members first (a separate change, outside
this item).

Not first-refusal here: cases of other texts that meet this one second
(the "Next refusal" column of the measure's `summary.md`: for example
`367:ARITH` 72, `unitype` `vs_int_uniform` 14, `vs_struct_uniform` 14,
`vs_mediump_struct` 16, `367:CONST_FOLD` 14, `varylink` `lane_no_attribute`
7, `367:CONTROL_FLOW` 2). Lifting those owners' texts raises this text's
count above 821; re-measure the steps of section 5 after them.

## 3. What Mesa emits for each shape

All 20 are the Mali-G610 fragment programs of Mesa 26.2.1; `colour` is
mediump, so the output is an fp16 `vec4` through `BLEND.f16`. Common frame:
`LD_VAR_BUF_IMM` (the varyings' lanes, f32, a `v4`/`v3`/`v2` load of the lanes
used), `ATEST @r60`, `BLEND.slot?.v4.f16.end @r0:r1`, `IADD_IMM.i32 r48, 0`,
`BRANCHZI`. The colour is two 32-bit words (x,y) and (z,w) of two fp16 each.

| Shape | What the two colour words are |
|---|---|
| `op_pair_yz` (13 instrs) | each lane op in fp32 (`FMA.f32 r0, r3^, u2.w0, 0x0.neg`: `degrees` = x * 57.29578 on `v_in0.w` and `.y`), the lane narrowed alone (`FADD.f32 r0.h0, r0^, 0x0.neg`), then the **pairs**: `MKVEC.v2i16 r0, 0x0.h0, r0^.h0` (constant 0 first, then the lane) and `MKVEC.v2i16 r1, r1^.h0, u3.w0.h0` (the lane, then the constant alpha from the FAU); the scale and bias per word `FMA.v2f16 r0, r0^, u2.w1, 0x38000000.h11` (0.04 scale in the FAU, 0.5 inline); `ATEST ... r1.h1` takes the alpha half of the last word |
| `op_pair_xyzw` (20) | the four lane ops, four narrows, `MKVEC.v2i16 r1, r1^.h0, r2^.h0` and `MKVEC.v2i16 r0, r3^.h0, r0^.h0` (the pairs ordered as the loads were: Mesa's `.xyzw` result came out as words (y,z) and (w,x) of the load), two `FMA.v2f16` scale-and-bias, then `MOV.i32` register shuffles into `r0:r1` for the BLEND source |
| `op_pair_xyz` (15) | three lane ops and narrows, `MKVEC.v2i16 r0, r2^.h0, r0^.h0` and `MKVEC.v2i16 r1, r1^.h0, u3.w0.h0`, two `FMA.v2f16` |
| `op_pair_x` (13) | `exp`: `F32_TO_S32` / `FEXP.f32` (the transcendental, the arith family's own), narrow, **`U16_TO_U32 r0, r0^.h0`** (lane x with y = 0, `lower_mkvec_swz`), `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x0.neg` (scale 0.5 only), the second word a bare `MOV.i32 r1, 0x38000000` constant (z,w = 0, 0.5 after the bias form) |
| `op_pair_other` (27) | `sequence.side_effects`: a compare to a mask (`FCMP_OR.v2f16.lt.m1`, `S16_TO_S32`, `CSEL.u32.eq`), the bool added back to two float lanes (`FADD.f32 r1, r1^, r0`), `F32_TO_S32.rtz` / `S32_TO_F32` int round trips and `FMA.v2f16` scale-bias. Needs the intval family's conversions first |
| `bcast_compare_int` (11) | no varying load at all: the vertex shader writes `v_vtxOut = 1.0` and Mesa's link replaces it by the constant, so `result` is the compare alone: `MOV.i32 r0, 0x0`, the compare as a mask (`ICMP_OR.u32.eq.m1 r0, u2.w0, u2.w1, r0^`, uniform and literal both in the FAU), the 0.0/1.0 select `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00`, then the three-word broadcast: `IADD.v2u16 r1, r0.h00, 0x0` (the lane in both halves), `MKVEC.v2i16 r2, r0^.h0, u3.w0.h0` (lane, alpha 1.0 from the FAU), `MOV.i32 r0, r1^`, `MOV.i32 r1, r2^` |
| `bcast_compare_bool` (10) | `MOV.i32 r0, u2.w0` (the bool uniform word), `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00`, then the same `IADD.v2u16 .h00` / `MKVEC` / `MOV` broadcast and alpha `u2.w1.h0` |
| `bcast_compare_float` (39) | `multiple_basic`: the fp16 compares of the float uniforms (`FADD_IMM.v2f16 r0, u6.w0, #...`, `FMAX.v2f16 ... .abs`, `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32`), int/bool uniform masks (`ICMP_OR`/`ICMP_AND.u32.eq.m1`, `CSEL.u32.gt`) combined with `LSHIFT_AND.i32 r1, r2^, 0x0.b00, r1^` (the `*` of 1.0/0.0 floats as AND of masks), `MUX.v2i16` and the same broadcast |
| `bcast_compare_array` (26) | the array's compares as `FADD_IMM.v2f16` + `FMAX.v2f16 .abs.h00/.h11` reductions (`abs(a-b) < 0.05` of four lanes, folded to one `FMAX` chain), `FCMP_OR.v2f16.lt.f1` against the 0.05 FAU word, then the broadcast: `IADD.v2u16 r1, r0.h00, 0x0`, `MKVEC.v2i16 r2, r0^.h0, u5.w1.h0` |
| `bcast_const` (11) | the compare folded to a mask of constants (`ICMP_OR.u32.eq.m1 r0, 0x7060504.b3, u2.w0, r0^`) then the same `MUX` and broadcast |
| `mat_cols_sum` (29) | fp32 `FADD` chains of the varying columns' lanes, e.g. `FADD.f32 r1, r2^, r5^` / `r1^, r8^` per colour lane; each sum narrowed alone (`FADD.f32 r1.h0, r1^, 0x0.neg`), `MKVEC.v2i16 r1, r1^.h0, r2^.h0` for (x,y) and `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0` for (z, alpha 1.0) |
| `mat_elem_pairs` (18) | `FADD.f32` of the lane loads with the scalar, narrowed alone, `MKVEC.v2i16 r1, r1^.h0, r2^.h0` and `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0` |
| `mat4_swz_sum` (19) | the constant matrix plus the scalar varying (`FADD_IMM.f32 r1, r0, #0x3F800000`, ...), the swizzle sum of four columns done in fp32, narrowed alone, the same two `MKVEC.v2i16` |
| `mat_vec_lanes` (14) | `FMA.f32` chains (`vec * mat`), the lanes narrowed alone, `MKVEC.v2i16 r0, r1.h0, r0^.h0` (a lane repeated in a word: the `.x` of the first word's source) and `MKVEC.v2i16 r1, r1^.h0, u2.w0.h0` |
| `vec3_local` (17) | the load converted by pairs (`V2F32_TO_V2F16 r0, r0^, r1^` and `r1, r2^, r3^`), `FMA.v2f16` scales with `.h10` swizzles, `FADD.v2f16` sums, `FMA.v2f16 r1, 0x34000000.h11, r1^, r2^`, and the alpha `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0`: a whole fp16 vector computation (the mixprec family's "fp16 pairs") ending in a colour |
| `rnd_vec4` (12) | `LD_VAR_BUF_IMM.v3`, `V2F32_TO_V2F16 r0, r1^, r0^` (converted in swapped order), `FADD.f32 r1.h0, r2^, 0x0.neg`, constants joined by `MKVEC.v2i16 r2, r0.h0, u2.w0.h0` and `FADD.v2f16 r0, r2^, r0^.h10` (a lane swizzle `.h10` as a source modifier): each random expression is its own arrangement of these |
| `tex_coord_ramp` (15) | `V2F32_TO_V2F16 r0, r0^, r1^` (both coordinates in one word), `FADD.v2f16 r1, r0.h00, r0.h11`, `FADD.v2f16 r0, r0^.h00, r0^.neg.h11`, `FMA.v2f16 ... 0x38000000.h11 ...` (the 0.5s), `MKVEC.v2i16 r2, r1.h0, r2^.h0`, `FADD.v2f16 r1, 0x3C000000.h11, r1^.neg.h00` (1.0 - f0) and `MKVEC.v2i16 r1, r1^.h0, r0.h0`. A lane (x, y) used as a pair half and alone: `conv_form` in `select` |
| `varying_merged` (8) | `LD_VAR_BUF_IMM.v3`, `V2F32_TO_V2F16 r0, r0^, r1^` and `FADD.f32 r1.h0, r2^, 0x0.neg` (z alone), `MKVEC.v2i16 r1, r1^.h0, r0.h0` (the **w word is (z, x)**: the merged lane x reused as the last half): the link removed varying lane w |
| `struct_fields` (10) | `LD_VAR_BUF_IMM.v3`, `V2F32_TO_V2F16 r0, r0^, r1^`, `FADD.f32 r1.h0, r2^, 0x0.neg`, the int uniform member through `S32_TO_F32 r2, u2.w0` + narrow, `MKVEC.v2i16 r1, r1^.h0, r2^.h0` |
| `tex_zw_lane` (15) | a `TEX_SINGLE` of the sampler at (0,0) (`r0` = the texel, only zw read), `S32_TO_F32 r1, u2.w0` / `r2, u2.w1` of the `ivec4` uniform and `V2F32_TO_V2F16 r1, r1, r2^`, the colour words (xy from the uniform conversion, zw from the texel) moved by `MOV.i32` into `r0:r1` |

## 4. Where the compiler raises it

All in `src/shaderc/online/mali/nir_fs.rs` (read for this note; the exact
site per shape was not instrumented, because no change under `src/` is part
of the item, so the site names are the likely ones):

- `layout` (about 7765, `Err(ColourForm)` at 7781): a colour lane that is
  `Val::Undef`.
- `vectorise` (8329 to 8718): the colour pair's two lanes `(a, b)` as one
  fp16 word, a `P::Leaf(...)` per accepted form. **The fall-through
  `_ => return Err(ColourForm)` at 8713 and the branch at 8649 are where
  shapes 1 to 5, 7 to 10, 13 to 15 end**: a pair of lanes of different
  loads or of a lane and a computed value whose form no earlier reference
  drew. The forms accepted today are `Half` (an aligned pair of one load's
  lanes), `Splat`, `Lanes`, `Mk` (constant then lane), `Const`, `Uniform`.
- `lane_tree` (9252, `Err(ColourForm)` at 9439): the lane-wise colour trees
  (the operator results); an operand that is not a load's lane, a constant
  or a uniform component.
- `scalar` (8950; 8961 and 8963): a sum of uniform-buffer reads
  (`sum_of_reads`), anything but `fadd` of `UboHalf`/`UboWord`: the
  `bcast_compare` family's compare products are not sums.
- `select` (9744; `Err(ColourForm)` at 9920): a varying lane converted both
  alone and as half of a pair (`conv_form`), shape 13's `x`, `y`.
- `word_of` (10539; 10584), `moved_key` (11825; 12246) and the several
  `walk_nodes` sites (5243; 5642 to 6281) for a compare, a select or an
  `Undef` colour at the NIR walk: `bcast_compare` and `op_pair_other`.
- `op` (1392, 1394), `neg` (1743, 1745), `abs` (1762, 1764), `op32` (2014,
  2034): the fp32 lane operations (`FMA.f32`, `FADD.f32`) whose operand form
  has no reference; the operator harness's `res` computations (shapes 1 to
  5) arrive here after the pair is accepted.

## 5. Lowering map, ranked by cases (one session each)

Every step is checked against its references (`tests/mali_deqp_fclane.rs`,
`LOWERED`) and the host measure (`tools/mali/deqp-host-measure.sh --filter`)
for its dEQP groups. A reference whose refusal moves to another text when
its lane is lowered (the lane's *operation* is another family's) stays in
`REFUSED` with the new text; it counts for this item's forecast only when the
other text's owner has landed.

1. **Operator colour pairs (292 cases: shapes 1 to 5), refs `op_pair_*`.**
   `vectorise`/`lane_tree`: accept **a computed fp32 lane narrowed alone
   (`FADD.f32 r.h0, r^, 0x0.neg`) as a pair half**, the `MKVEC.v2i16` of
   (constant 0, lane) / (lane, FAU alpha) / (lane, lane) in Mesa's pair
   order (the xyzw case orders (y,z) then (w,x): the first load lane
   goes last), and the word's scale-and-bias `FMA.v2f16 word, word^,
   u.w1, 0x38000000.h11` (or the bare scale). Emit: after `alu32_lanes`
   computes the lane, `select` emits `FADD.f32 .h0`, then `MKVEC`, then the
   word's `FMA.v2f16` with the FAU's scale/bias words. The lane operations
   themselves (`degrees`, `exp`, `abs`, `min`, ...: the ARITH family) are
   `arith-lower-1`'s: the cases whose lane ops are already lowered (`+`, `-`, `*`, `abs`) pass
   this text; the others turn into ARITH refusals. The forecast is not
   measurable before the step (it needs the lowering); measure it after with
   `--filter shaders.operator`.
2. **Broadcast of one computed lane (229: shape 6), refs `bcast_compare_*`.**
   Needs the intval/compare lowering of the lane (a mask from `ICMP_OR`,
   `FCMP_OR.v2f16`, `MUX.v2i16 0, 0x3C00.h11, mask.h00`, or the `FMAX`
   reduction of `abs(a - b) < 0.05`): that is `intval-rest` and the
   ARITH items. This item's part: `select` emits the **three-word
   broadcast** `IADD.v2u16 r1, r0.h00, 0x0` (lane in both halves),
   `MKVEC.v2i16 r2, r0^.h0, u.w1.h0` (lane, alpha) and the `MOV.i32` pair
   to the BLEND source, `vectorise` accepting one `P::Op` result in three
   colour positions (a `Splat` of a computed node). 94 of the 229 read a
   struct uniform and need `gl_ref.py` to set it (no reference).
3. **Matrix lane sums (122: shapes 7 to 10), refs `mat_*`.** `scalar`
   /`vectorise`: accept a sum of fp32 `FADD` over a load's lanes narrowed
   alone, the `MKVEC.v2i16 (x,y)` and `(z, alpha)` word forms (as step 1's
   pair half, so it follows step 1 almost for free), the FAU alpha
   `u.w0.h0`. Shape 9's constant matrix adds `FADD_IMM.f32` of the scalar
   varying with literal columns (`const_mat_chain`, 6927). The `post_decrement` cases (12
   of `mat_cols_sum`, 12 of `mat_elem_pairs`) add a second `tmpValue` term.
4. **Random fragment vec4 expressions (66: shape 12), ref `rnd_vec4`.** The
   unique per-case arrangements of step 1's and 3's forms plus conversion
   order (`conversion_order`, 8743: the load converted in swapped order),
   the `.h10` operand swizzle, `V2F32_TO_V2F16` of a `v3`. No separate
   lowering: re-measure after steps 1 and 3 and pin the residue.
5. **`vec3_local` (52), ref `vec3_local`.** Whole fp16 vector arithmetic
   ending in a colour (`FMA.v2f16`, `.h10` swizzles, two `V2F32_TO_V2F16`):
   the mixprec family's `alu16`/`vec16` (2806, 2759) lanes given as colour
   halves; 12 of them are `shaders.matrix.mul` of vec3 results.
6. **`tex_coord_ramp` (20), ref `tex_coord_ramp`.** `select`'s `conv_form`
   rule (9919): allow a lane converted as half of a pair and used alone in
   the same program when Mesa does (`FADD.v2f16 r1, r0.h00, r0.h11`).
7. **`varying_merged` (18), ref `varying_merged`.** The colour reads a
   varying whose lanes the link merged: the fragment's `w` is the load's
   lane `x`: `MKVEC.v2i16 r1, r1^.h0, r0.h0`. `index` (8056) must map the
   merged lane and `vectorise` accept (z, x) with z converted alone: the
   varylink family's `VaryingDuplicate` path from the vertex side exists
   (`mali-deqp-varylink.md`); this is its fragment side.
8. **`struct_fields` (14), `tex_zw_lane` (4), `bcast_const` (4).** Single
   forms: a struct member read (`ir` front end), an int uniform member
   converted (`S32_TO_F32` + narrow, `MKVEC` pair) and the second assignment
   `gl_FragColor.zw` after `.xy`, a texel's lanes as colour halves
   (`tex_zw_lane`; the texture refusals `TEX_COORD` may come first).

Dependencies on other items of the plan: steps 1 and 3 share the pair
machinery and should be one commit sequence; step 2 waits for the compare
lowerings of `intval-rest`/`arith-lower-1`; the host forecast of each is
re-measured with `tools/mali/deqp-host-measure.sh --filter <group>` after
the step and compared with section 2's counts.

## 6. Files

- `tools/mali/testdata/deqp_fclane_*.{vert,frag,expect.json}`: the twenty
  references (shape names in section 2's `Ref` column).
- `tests/mali_deqp_fclane.rs`: byte-compare per reference (`#[ignore]`d),
  `refusals_pinned`.
- Run: `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclane`.
