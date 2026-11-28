<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2 `mixprec3`: FS mixed precision and fragment varying loads at 8ff01055

Date 2026-11-24. Plan #402 item `ref-mixprec3` (stage 5, issue #20, NanoPC-T6
Mali-G610) added the references and the test; section 5 is the lowering
that item `lower-mixprec3` made from them. The
references are `tools/mali/testdata/deqp_mixprec3_*` (Mesa 26.2.1 drm-shim,
GPU g610) and the test is `tests/mali_deqp_mixprec3.rs`. Read
`docs/notes/mali-deqp-mixcol.md` and `docs/notes/mali-deqp-mixprec.md` first:
they hold the shapes these two refusals had at 37305608 and bee6896d. All of
this is HOST-ONLY. Nothing here ran on the board.

## 1. The two refusals

Both refusals come from the board slices
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`. Each is the
first refusal of a case, and every case is a Fail.

| key | `MaliError` (`src/shaderc/online/mali/mod.rs`) | text | board Fail |
|---|---|---|---:|
| MIX | `MixedPrecision` (:440) | a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) | 168 |
| LOAD | `FsVaryingLoad` (:540) | a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) | 50 |

## 2. How the tip was measured

The plan asked for `tools/mali/deqp-host-measure.sh`, but this host could not
run it. That script builds dEQP from a full VK-GL-CTS checkout with its
external sources and the yakogl target, and there is none here. Only the
GLES2 generator sources were fetched: a sparse, blob-less VK-GL-CTS at 93bca01
(`modules/gles2/functional/`, `modules/glshared/`) in
`$CONNECTED_DEPS_DIR/VK-GL-CTS-src.part`. There is no `.qpa`. Instead:

- **Operator cases (77 of the 218).** Every `shaders.operator.*_fragment`
  case of both lists was rebuilt from dEQP's operator harness. The harness
  was first read off the verbatim `.qpa` sources already in
  `tools/mali/testdata/deqp_mixcol_*` and `deqp_colourpair_*`. It was then
  checked against `es2fShaderOperatorTests.cpp` (`ShaderOperatorCase`,
  lines 600-744):
  - the input swizzles `z, wy, zxy, yzwx` / `x, yx, yzx, wzyx` / `y, zy, wyz, xwzy`;
  - the output swizzles `x, yz, xyz, xyzw`;
  - `mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0)`;
  - the scale and bias line of each case's `BuiltinFuncInfo` or
    `BuiltinOperInfo`.

  Each rebuilt case went through the product route at the tip e65567ea
  (`tools/mali/route.sh measure`, features
  `host-test,gpu-mali,shaderc-online,no-shader-table`). **Every one of the
  72 rebuilt operator cases gives the same first refusal as on the board**
  (MIX or LOAD, case for case). The other 5 operator cases are the
  `sequence` ones; their verbatim mixcol sources were run instead.
- **Verbatim sources already in testdata.** These were run through the same
  route:
  - `deqp_mixcol_{hp_scale, hp_bool, hp_param, hp_seq, int_uniform, int_uniform_struct, f2i_rnd}`
    are still MIX.
  - `deqp_mixcol_{ck_scale_highp, lz_rnd_other}` and the shape
    `mixprec_fs_varying_other` (return.conditional_return_always_fragment)
    are still LOAD.
  - These compile at the tip: `deqp_mixcol_f2i_seq`
    (sequence.no_side_effects.mediump_vec4_ivec4_bvec4), `deqp_mixcol_f2i_dead`
    (random.scalar_conversion.fragment.27, on the board list) and the shape
    `mixprec_fs_mixed_other` (random.conditionals.fragment.27, on the board
    list).
- **Not re-measured.** The `shaders.random.*` cases (MIX 97, LOAD 16) other
  than the four verbatim ones above, and `shaders.matrix.*` (MIX 11). dEQP
  makes these with generators that need a dEQP build (the random shader
  generator, `es2fShaderMatrixTests.cpp`). The mixcol note
  puts most of the random MIX cases under `int_uniform` and `f2i_rnd`.

## 3. Shapes

"covered" names an existing reference that draws the shape. A reference
named `mixprec3 <shape>` is new here (`deqp_mixprec3_<shape>`).

| text | shape | board Fail | the shape | reference |
|---|---|---:|---|---|
| MIX | `hp_vec3_fn` | 5 | a highp vec3/vec4 built-in result (`sqrt`, `inversesqrt`, `normalize`) written into the mediump colour | mixprec3 `hp_vec3_fn` (normalize.highp_vec3) |
| MIX | `hp_dot` | 3 | `dot` of two highp vector varyings, into `color.x` | mixprec3 `hp_dot` (dot.highp_vec3) |
| MIX | `hp_bvec4` | 5 | `float_compare.*.highp_vec4`: four fp32 compares, every colour lane a bool | mixprec3 `hp_bvec4` (lessThan.highp_vec4) |
| MIX | hp_scale | 21 | one highp lane through a unary op or built-in into `color.x` | covered: `deqp_mixcol_hp_scale` |
| MIX | hp_bool | 12 | a highp compare into `color.x` or `color.yz` | covered: `deqp_mixcol_hp_bool` |
| MIX | hp_seq / f2i_seq | 5 | the sequence operator | covered: `deqp_mixcol_hp_seq`, `deqp_mixcol_f2i_seq` |
| MIX | hp_param | 4 | a highp function parameter | covered: `deqp_mixcol_hp_param` |
| MIX | int_uniform_struct | 4 | an int struct member read as a float | covered: `deqp_mixcol_int_uniform_struct` |
| MIX | int_uniform | 1 | `fragment_ops.interaction.basic_shader.73` | covered: `deqp_mixcol_int_uniform` |
| MIX | matrix | 11 | `matrix.{add,sub,mul}.const_highp_matN_matN`, `mul.*_highp_matN_vecN`, `unary_addition.dynamic_highp_mat4` | not rebuilt (§2) |
| MIX | random | 97 | `shaders.random.*` | not re-measured (§2) |
| LOAD | `hp2_sum` | 14 | a highp float op of two or three highp scalar varyings (`+ - *`, their assigns, `mix`, `distance`, `dot`, `reflect` of floats; `distance` of vec2) into `color.x` | mixprec3 `hp2_sum` (add.highp_float) |
| LOAD | `hp_vec3_sum` | 3 | a highp float with a highp vec3 (`+ - *`) into `color.xyz` | mixprec3 `hp_vec3_sum` (add.highp_float_vec3) |
| LOAD | `mp_vec3_scale` | 3 | a mediump vec3 varying copy (`+in0`, `in0++`, `in0--`) into `color.xyz`, then `* 0.5 + 0.5` | mixprec3 `mp_vec3_scale` (plus.mediump_vec3) |
| LOAD | `lp_vec3_scale` | 3 | the same at lowp: a lowp colour beside the mediump `color` | mixprec3 `lp_vec3_scale` (plus.lowp_vec3) |
| LOAD | `vary_array_static` | 4 | a mediump varying array read at constant indices (`{float,vec2}_static_write_static{,_loop}_read`) | mixprec3 `vary_array_static` (float_static_write_static_read) |
| LOAD | ck_scale_highp | 3 | `unary_operator.*.highp_vec2` | covered: `deqp_mixcol_ck_scale_highp` |
| LOAD | lz_rnd_other | 2 | `random.basic_expression.{combined,fragment}.45` | covered: `deqp_mixcol_lz_rnd_other` |
| LOAD | return | 2 | `return.conditional_return_{always,never}_fragment` | covered: `deqp_gaps_mixprec_fs_varying_other` |
| LOAD | random | 16 | `shaders.random.*` | not re-measured (§2) |

Totals: MIX 5+3+5+21+12+5+4+4+1+11+97 = 168 and LOAD 14+3+3+3+4+3+2+2+16 =
50. The eight new references cover 40 board cases (MIX 13, LOAD 27).

The `.license` of each source names the generator lines of VK-GL-CTS 93bca01
it was written from. Nothing in the sources is assumed:

- `hp_dot` scales by 0.1 (`BuiltinFuncInfo "dot"`, line 1824):
  `color = color * 0.10 + 0.50;`. `hp_vec3_fn` takes 0.5 and 0.5
  (`"normalize"`, line 1828).
- `vary_array_static` is `es2fShaderIndexingTests.cpp:286-372`. Its fragment
  shader starts with `precision mediump int;` and reads with four
  `res += var[k];`.

These are still not `.qpa` text. The generators are the source of the
`.qpa`, but a lowering that reaches the real dEQP case should still check
its words against a `.qpa` once one is available.

### The cases

- MIX `hp_vec3_fn`: exponential.inversesqrt.highp_vec3, .highp_vec4,
  exponential.sqrt.highp_vec3, .highp_vec4, geometric.normalize.highp_vec3
  (all `shaders.operator.*_fragment`).
- MIX `hp_dot`: geometric.dot.highp_vec2, .highp_vec3, .highp_vec4.
- MIX `hp_bvec4`: float_compare.{equal, greaterThan, greaterThanEqual,
  lessThan, lessThanEqual}.highp_vec4.
- MIX hp_scale: angle_and_trigonometry.{degrees,radians}.highp_float,
  common_functions.{abs,fract}.highp_float,
  exponential.{sqrt,inversesqrt}.highp_float,
  geometric.length.highp_{float,vec2,vec3,vec4},
  geometric.normalize.highp_float,
  unary_operator.{minus,plus}.highp_float, and
  unary_operator.{pre,post}_{increment,decrement}_{effect,result}.highp_float.
- MIX hp_bool: binary_operator.equal.highp_{float,vec2,vec4},
  binary_operator.{greater,greater_or_equal,less,less_or_equal}.highp_float,
  float_compare.{equal,greaterThan,greaterThanEqual,lessThan,lessThanEqual}.highp_vec2.
- MIX hp_seq / f2i_seq: sequence.no_side_effects.highp_float_int,
  sequence.no_side_effects.{highp,mediump,lowp}_vec4_ivec4_bvec4,
  sequence.side_effects.highp_float_int.
- MIX hp_param: functions.datatypes.struct_constructor_highp_in_fragment,
  functions.qualifiers.{in,inout,out}_highp_float_fragment.
- MIX int_uniform_struct: linkage.uniform_struct_partial_{int_float,ivec2_ivec2,vec2_int},
  struct.local.nested_struct_array_fragment.
- MIX int_uniform: fragment_ops.interaction.basic_shader.73.
- MIX matrix: matrix.add.const_highp_mat{2_mat2,3_mat3,4_mat4},
  matrix.sub.const_highp_mat{2_mat2,3_mat3,4_mat4},
  matrix.mul.const_highp_{mat2_mat2,mat4_vec4},
  matrix.mul.dynamic_highp_{mat2_vec2,mat4_vec4},
  matrix.unary_addition.dynamic_highp_mat4 (all `_fragment`).
- MIX random (97):
  - all_features.fragment.11;
  - comparison_ops.fragment.{18,19,36,37,47};
  - conditionals.combined.1;
  - conditionals.fragment.{1,27,41,48,60,84,86,97};
  - exponential.fragment.{28,31,33,91};
  - scalar_conversion.combined.{27,36,45,61,70,97,98};
  - scalar_conversion.fragment.{19,27,36,37,38,42,44,45,61,70,75,81,86,89,90,97,98};
  - swizzle.fragment.{0,4,9,10,27,32,43,46,47};
  - texture.fragment.{4,5,9,10,13,14,15,18,27,32,36,42,44,45,46,47,50,61,62,66,77,83,85,90,113,116,119,123,129,130,132,133,137,140,145};
  - trigonometric.fragment.{6,8,24,28,33,42,43,75,91,95}.
- LOAD `hp2_sum`: binary_operator.{add,sub,mul}{,_assign_effect,_assign_result}.highp_float,
  common_functions.mix.highp_float, geometric.distance.highp_{float,vec2},
  geometric.dot.highp_float, geometric.reflect.highp_float.
- LOAD `hp_vec3_sum`: binary_operator.{add,sub,mul}.highp_float_vec3.
- LOAD `mp_vec3_scale`: unary_operator.{plus,post_increment_result,post_decrement_result}.mediump_vec3.
- LOAD `lp_vec3_scale`: the same three, lowp_vec3.
- LOAD `vary_array_static`: indexing.varying_array.{float,vec2}_static_write_static_read,
  indexing.varying_array.{float,vec2}_static_write_static_loop_read.
- LOAD ck_scale_highp: unary_operator.{plus,post_increment_result,post_decrement_result}.highp_vec2.
- LOAD lz_rnd_other: random.basic_expression.{combined,fragment}.45.
- LOAD return: return.conditional_return_{always,never}_fragment.
- LOAD random (16):
  - basic_expression.combined.{31,57};
  - basic_expression.fragment.{8,31,33,57};
  - comparison_ops.fragment.15;
  - conditionals.fragment.{19,36,75};
  - scalar_conversion.{combined,fragment}.65;
  - swizzle.fragment.{15,36,41};
  - texture.fragment.110.

## 4. What Mesa does, per reference

The table gives the `fs.fragment` region of each `.expect.json` (its
`text`). Three things hold for all eight references:

- **No `F32_TO_F16` or `F16_TO_F32` instruction.** Mesa narrows an fp32
  value to fp16 in one of two ways. A lone lane takes `FADD.f32 rN.h0, x,
  0x0.neg`: the conversion is folded into an add of -0 with a 16-bit
  destination. Two lanes that sit next to each other in a load take
  `V2F32_TO_V2F16 r, a, b`. Nothing is ever widened back.
- **Every varying load is `LD_VAR_BUF_IMM.f32 ... src_f32`.** It is `.v4`,
  except in `hp2_sum`, which loads one scalar. Even mediump and lowp
  varyings travel as fp32 words (the vertex `STORE.i32` of f32 values), and
  the fragment shader converts them after the load.
- **The colour is two `v2f16` words**, joined by `MKVEC.v2i16` or
  `U16_TO_U32`. The constant 1.0 is a FAU half (`u2.w0.h0`) or an immediate.

| reference | words | LD_VAR | where Mesa converts | the rest |
|---|---:|---|---|---|
| `hp_vec3_fn` | 19 | one `.v4.f32` | after the fp32 `normalize` (`FMA.f32` dot, `FRSQ.f32`, three `FMA.f32`), each lane alone: three `FADD.f32 rN.h0`. Then `MKVEC.v2i16 (z,x)`, `(y, 1.0)` | `FMA.v2f16` by 0.5, 0.5 on both words |
| `hp_dot` | 14 | two `.v4.f32` (slots 0, 1) | after the fp32 three-term dot (`FMA.f32` chain), one `FADD.f32 r0.h0` | `U16_TO_U32` (lane, 0.0), `FMA.v2f16` by 0.1 (`0x2E66`) and 0.5; `ATEST` takes 1.0 from the FAU (`u2.w0`), and the second colour word is a FAU `MOV` (`u2.w1`) |
| `hp_bvec4` | 17 | two `.v4.f32` | none: each `FCMP_OR.f32.lt.m1` is followed by `MUX.v2i16 r, 0x0, 0x3C000000.h11, r.h00` (the 16-bit `b2f`) | two `MKVEC.v2i16` |
| `hp2_sum` | 8 | **one scalar** `.f32`: Mesa's link moves `in0 + in1` into the vertex shader (`FADD.f32` in `vs.varying`), so the fragment shader reads one sum | one `FADD.f32 r0.h0` of the loaded lane | `U16_TO_U32`, second word `MOV.i32 0x3C000000` |
| `hp_vec3_sum` | 15 | one `.v4.f32`: the link packs `v_in0.z` and `v_in1.yzx` into one slot | after the three `FADD.f32`, each lane alone (`FADD.f32 rN.h0` ×3) | `MKVEC.v2i16 (x,y)`, `(z, 1.0)`; no scale |
| `mp_vec3_scale` | 14 | one `.v4.f32` | at the load: `V2F32_TO_V2F16 r0, r0, r1` (an aligned pair) and `FADD.f32 r1.h0, r2` (the third lane alone) | `MKVEC.v2i16 r1, r1.h0, r0.h0` (lane z, then x of the pair), `MKVEC.v2i16 r0, r0.h1, u2.w0.h0`; `FMA.v2f16` by 0.5 on each word |
| `lp_vec3_scale` | 14 | as `mp_vec3_scale` | as `mp_vec3_scale`: its fragment words are the same | lowp and mediump are both fp16 in Mesa |
| `vary_array_static` | 12 | one `.v4.f32` (the four `var[k]` in one slot) | at the load: `V2F32_TO_V2F16` of `(var0, var1)` and of `(var2, var3)` | the sum at fp16 on halves (`FADD.v2f16 r0.h00, r0.h11`, ...), then `U16_TO_U32`, `MOV.i32 0x3C000000` |

The LOAD shapes do not match the refusal's three cases one for one.

- `mp_vec3_scale`, `lp_vec3_scale` and `vary_array_static` are the "aligned
  pair" cases. Mesa narrows the pair `(x, y)` of a load with one
  `V2F32_TO_V2F16`. It then splits the pair again by halves (`r0.h0`,
  `r0.h1`) into words of another order.
- `hp2_sum` and `hp_vec3_sum` are "lanes of unmeasured precision": highp
  arithmetic that the link moves or packs, then narrowed lane by lane.
- The MIX shapes are a highp (fp32) computation that ends in the mediump
  colour. That is Mesa's `FADD.f32 .h0` at the last fp32 value, the same as
  in `deqp_mixcol_hp_scale`. The bool form is `MUX.v2i16` with no conversion.

## 5. The lowering (plan #402 `lower-mixprec3`)

Landed on 2026-11-28 (redone on main 8c9abf51) from proto-mixprec3-tip's combined diff (proto-mixlink2's
diff from its report, with the varying-load half rebuilt from
proto-varyload's description; proto-varyload's own diff was lost). HOST-ONLY:
nothing here ran on the board. It takes these references:

| test | now byte-equal (`MATCHED` or `LOWERED`) |
|---|---|
| `mali_deqp_mixprec3` | `hp_bvec4`, `mp_vec3_scale`, `lp_vec3_scale`, `vary_array_static` |
| `mali_deqp_mixprec4` | all six |
| `mali_deqp_mixcol` | `hp_bool`, `lz_rnd_other` |
| `mali_deqp_varyload` | `rnd57`, `rnd31`, `basic33`, `cond19`, `tex110` |
| `mali_deqp_irindex` | `load_vec3_cmp` |
| `mali_deqp_constprod` | `constchain_fs` |
| `mali_deqp_texmisc` | `tk_rnd_bias_proj` |

What it does (`src/shaderc/online/mali/`):

1. **Highp compares stay fp32.** A compare of a highp varying's lanes
   (`Fs::hp_lane`) and constants is `FCMP_OR.f32` followed by the 16-bit
   `b2f` `MUX.v2i16` (`hp_bvec4`, the mixprec4 compares). A compare of
   arithmetic on those lanes keeps the 16-bit select that main's FS_REFRACT
   made for refract's `k < 0.0` (`tests/mali_fs_refract.rs`; at fp32 dEQP's
   scalar refract fell back to ARITH). `nir_fs.rs` `swapped_compares` orders
   the two varying loads of a vector compare as Mesa does (`gt_vec2`,
   `le_vec2`).
2. **Varying conversions in Mesa's order.** `conversion_order` relaxes the
   `lone` rule. A varying array's elements count as one variable (`Fs::arr_el`).
   A slot holding several variables, or an array, is converted lazily at each
   variable's first reader (`Conversions::lazy`, the `flush` closure in
   `select`). A pair whose odd lane is read first, or colour-only lanes Mesa
   pairs as (odd, even), converts with the odd lane low (`Conversions::swap`,
   `Leaves::conv_swap`, used by `conversion()`). The existing `same` check
   still guards the order. Main's VARYING_LOAD (a27396fa) came first for
   one case of several variables: packed mediump vec3s and vec4s in a
   mediump shader convert at their variable's first read
   (`Conversions::later`, the `CONV` event; gm2_vload_*), and the same
   vectors in a highp or mixed shader keep the FsVaryingLoad refusal
   (`gm2_cf_loop1_if_fs`). The lazy path takes the other slots: scalars
   and arrays. A `CONV` event flushes no lazy conversion.
3. **Overwritten stores are dead.** `mod.rs` `overwritten_stores` is used by
   `live_varying_loads` and `nir_fs.rs` `ir_live`. It drops an output store
   that a later store of the same output replaces in `main`'s top-level list
   with no control node between them (`tex110`, `cond19`), and an overwritten
   store to an output's own local (`nir.rs` `overwritten_stores`, now
   `pub(super)`; `basic33`). Values that only such a store read no longer
   refuse. So seven other pins move forward to a later refusal (`arith`,
   `fsint2`, `intrnd`, `intval3`, `mixprec2`), none of them backward.
4. **`-0.0 * b` folds to the constant -0.0** in `walk_nodes` (`rnd57`), and a
   colour `Pack` reads `Leaf::Part { h: 1 }` as `.h1` (`basic33`).

Still open:

- mixprec3 `hp2_sum` (FORM) and `hp_vec3_sum` (LOAD) need the link side:
  the vertex shader stores the sum the link moved into it, and the packed
  slot of `v_in0.z` and `v_in1`.
- varyload `swz41` (LOAD) is refused. `cond36` (fs word [1]: ours
  `V2F32_TO_V2F16.discard`, 11 words against 9, extra FAU words) and `basic8`
  (fs word [1]: ours `FADD.f32.discard r2.h0`, 18 words against 19) compile
  with other words, held by `differences_pinned`.
- mixcol `hp_param`, `hp_seq`, `int_uniform_struct` (MIX) and `ck_rnd_other`,
  `lz_rnd` (FORM). `f2i_dead` still differs in its FAU words.

No glmark2 verdict moves ("mali: 69 shaders, 52 compiled; 31 pairs tried, 31
linked; 23 of 33 scenes compile-clean", the same as before).

## 6. Reproducing

```sh
# a reference (MESA_SHIM_WORK defaults to $CONNECTED_DEPS_DIR/mesa-shim)
tools/mali/deqp_ref.sh <shape>.vert <shape>.frag tools/mali/testdata/deqp_mixprec3_<shape> --instrument shim
# the test
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec3
```

The shim used here was built from the tarball of
`third_party/mesa-26.2.1/UPSTREAM` (sha256 checked) with `drm_shim_ref.sh
build`. It redraws `gm2_cf_ifelse_fs` and `deqp_mixcol_hp_scale` byte for
byte. A second build in `$CONNECTED_DEPS_DIR/mesa-shim` redrew
`deqp_mixprec3_hp_vec3_fn` byte for byte. It then drew `hp_dot` again with
the generator's 0.1 scale, and `vary_array_static` again with its
`precision mediump int;` line, which leaves its bytes unchanged.
