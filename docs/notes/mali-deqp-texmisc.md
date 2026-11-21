<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): TexVertex, TexKind and FragCoordMediump refusals, the dEQP shapes against Mesa 26.2.1

Plan #387, item `texmisc-ref` (ROADMAP stage 5, issue #7). It covers three
refusal texts that had no owner and no reference set at the tip
(`docs/notes/mali-deqp-families.md` section 18.2, rows 20, 23 and 24):

| Row | Refusal text (`MaliError`, message line) | Stage |
|--:|---|---|
| 20 | "a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum" (`TexVertex`, `src/shaderc/online/mali/mod.rs:459`) | vs |
| 24 | "a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)" (`TexKind`, `mod.rs:462`) | fs |
| 23 | "fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y" (`FragCoordMediump`, `mod.rs:432`) | fs |

The item changes no compiler code. It commits 17 references,
`tools/mali/testdata/deqp_texmisc_<shape>.{vert,frag,expect.json}` with a
`.license` sidecar per shader naming the case, and
`tests/mali_deqp_texmisc.rs`. All 17 are refused at the tip: each test is
ignored, and `refusals_pinned` pins its refusal text.

**HOST-ONLY.** Every count below comes from `deqp-gles2` run on the host
against the Mali back end's C pair (`tools/mali/deqp-host-measure.sh`). Only
the compile and link outcomes of that run are real. A board status (Fail,
NotSupported) is the NanoPC-T6's own, taken from the slices.

- **Tip:** `ca7cdc529e8221b6ff93e317e8ef45f8428fda7c` (plan #387's
  integration tip).
- **CTS:** VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `tools/cts/cts-yakogl-target.patch` applied (uncommitted) and its external
  sources fetched, in `$CONNECTED_DEPS_DIR/VK-GL-CTS`.
- **Board counts:** the four slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` (10,597 refused
  cases), as in section 18 of the families note.
- **Command** (section 18's):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

All 10,597 cases began; 106 processes exited 1 (dEQP's "not every case
passed") and 2 exited 139 (the two `scalar_conversion.*.96` crashes of
section 18.4, unchanged). 6,132 cases have a host refusal. The three texts
are the first refusal of **144** cases, the same counts as section 18.2
(at `9ac31f30`): TexVertex 60, TexKind 42, FragCoordMediump 42. All 144 are
board Fail; none is NotSupported.

## 1. The shapes and their cases at `ca7cdc52`

The shape is the construct of the refusing shader (this note's own rule: the
dEQP group and name, then the look-up or conversion in the source; the
script's `FAMILIES` table names the three texts `new:TEX_VS_FORM`,
`367:TEX_VERTEX` and `new:FRAGCOORD_FP16` with one shape each). The
representative's pair is verbatim from the run's `.qpa`.

### 1.1 TexVertex: 60 cases, all vertex shaders

Every case is `uniform_api`'s sampler check in the vertex shader:
`v_vtxOut *= compare_vec4(texture2D(u_var, vec2(0.0)), vec4(...))` (or
`textureCube(u_var, vec3(0.0))`), with `compare_vec4` the product of four
`abs(a - b) < 0.05 ? 1.0 : 0.0` of the mediump texels. The look-up itself
(2D or cube at level 0) is taken since `80403eba`; the refusal is the
harness: an `fsub`, `fabs` and compare of the fp16 texels, which the vertex
walk takes only as a product or sum.

| Shape `deqp_texmisc_*` | Cases | dEQP groups | Representative (`dEQP-GLES2.functional.`) |
|---|--:|---|---|
| `tv_2d_compare` | 14 | `uniform_api.value.{initial,assigned.*}.*.basic.sampler2D_{vertex,both}` 12, `uniform_api.info_query.basic.sampler2D_{vertex,both}` 2 | `uniform_api.value.initial.render.basic.sampler2D_vertex` |
| `tv_cube_compare` | 14 | the same with `samplerCube` (12 + 2) | `uniform_api.value.initial.render.basic.samplerCube_vertex` |
| `tv_2d_array_compare` | 22 | `uniform_api.value.*.basic_array*.sampler2D_{vertex,both}` 20, `uniform_api.info_query.basic_array.sampler2D_{vertex,both}` 2 | `uniform_api.value.initial.render.basic_array.sampler2D_vertex` (`u_var[3]`, three look-ups) |
| `tv_rnd_2d` | 7 | `uniform_api.random.{0,14,22,30,32,49,95}` (2D samplers, plain or arrays) | `uniform_api.random.30` |
| `tv_rnd_cube_array` | 3 | `uniform_api.random.{11,33,78}` (cube sampler arrays) | `uniform_api.random.33` (`u_var0[4]`; its fragment shader reads the four too) |

By dEQP group: `uniform_api.value` 44, `uniform_api.random` 10,
`uniform_api.info_query` 6. The partner fragment shader is refused too in
21 of the 60 (the `_both` cases, whose fragment shader runs the same
harness): `basic_array.sampler2D_both` and `random.0` by
`367:FRAGCOLOR_LANE` (12), `samplerCube_both` and `random.{33,78}` by
`367:TEX_COORD` (9, the `tc_cube_const` shape of `mali-deqp-fmaloc.md`).
The 2D `basic.sampler2D_both` fragment shaders compile.

### 1.2 TexKind: 42 cases, all fragment shaders

27 of the 42 are the cases `mali-deqp-fmaloc.md` measured at `cc89a1cf`
(board TEX_VERTEX) with the references `deqp_fmaloc_tk_*`, still pinned to
this text at the tip (`tests/mali_deqp_fmaloc.rs` passes). 15
`shaders.random.texture.fragment.*` cases are new: their board refusal was
`CONST_FOLD` (4), `constop` (7) or `intval` (4), and those lowerings moved
them on to this text. Shapes by the look-ups of the refusing shader:

| Shape | Cases | dEQP groups | References |
|---|--:|---|---|
| 2D projection only (`texture2DProj`, no bias, no cube) | 14 | `shaders.texture_functions.fragment.texture2dproj_vec{3,4}` 2, `shaders.random.texture.fragment.{19,23,56,69,80,91,94,97,98,131,136,142}` 12 | `deqp_fmaloc_tk_proj_vec3`, `_tk_proj_vec4`, `_tk_rnd_proj` (`.19`); new `deqp_texmisc_tk_rnd_proj_swizzle` (`.131`), `deqp_texmisc_tk_rnd_proj_sum` (`.142`) |
| 2D bias only (`texture2D(s, c, bias)`) | 14 | `texture.mipmap.2d.bias.*` 4, `shaders.texture_functions.fragment.texture2d_bias` 1, `shaders.random.texture.fragment.{7,17,31,41,48,59,63,84,124}` 9 | `deqp_fmaloc_tk_bias_uniform`, `_tk_bias_varying`, `_tk_rnd_bias` (`.48`); new `deqp_texmisc_tk_rnd_bias_fragcolor` (`.124`), `deqp_texmisc_tk_rnd_bias_two` (`.41`) |
| 2D bias and projection | 3 | `shaders.random.texture.fragment.{78,99,108}` | new `deqp_texmisc_tk_rnd_bias_proj` (`.108`) |
| a cube look-up (with or without a 2D bias or projection beside it) | 11 | `shaders.texture_functions.fragment.texturecube_bias` 1, `shaders.random.texture.fragment.{26,34,52,55,115,121,125,127,144,147}` 10 | `deqp_fmaloc_tk_cube_bias`; new `deqp_texmisc_tk_rnd_cube_bias` (`.121`) |

By dEQP group: `shaders.random` 34, `shaders.texture_functions` 4,
`texture.mipmap` 4 (section 18.2's row). The partner vertex shader compiles
in all 42.

### 1.3 FragCoordMediump: 42 cases, all fragment shaders, none reads `gl_FragCoord`

The text names `gl_FragCoord`, but no case of the 42 reads it. All are
`precision mediump` fragment shaders that build an fp16 vector (or compare)
from lanes that went through an integer conversion: dEQP's
`int in0 = int(v_in0 * 1.0025);` then `float(in0)`, `vec2(in0)`,
`mat2(in0, ...)`, or the random shaders' `vec4(float(i), e, int(h), ...)`.
The fp16 helpers that raise the text (section 3) refuse any fp16 pair
whose lanes are not both fp16 values of a known kind; the int-converted lane
is not one. The board refused them as `intval` (21),
`367:FRAGCOLOR_LANE` (15), `constop` (3), `367:ARITH` (2) and
`367:CONTROL_FLOW` (1); the work landed since moved them here.

| Shape `deqp_texmisc_*` | Cases | dEQP groups | Representative |
|---|--:|---|---|
| `fcm_int_to_float` (scalar result) | 4 | `shaders.conversions.scalar_to_scalar.int_to_float` 1, `.vector_to_scalar.ivec{2,3,4}_to_float` 3 (all `_fragment`) | `shaders.conversions.scalar_to_scalar.int_to_float_fragment` |
| `fcm_ivec2_to_vec2` (vector of int lanes) | 13 | `.scalar_to_vector.int_to_vec{2,3,4}` 3, `.vector_to_vector.ivec*_to_vec*` 6, `.vector_combine.{int_int_to_vec2,int_int_int_to_vec3,int_int_int_int_to_vec4,int_ivec2_int_to_vec4}` 4 | `shaders.conversions.vector_to_vector.ivec2_to_vec2_fragment` |
| `fcm_int_bool_to_vec2` (vector of int and bool lanes) | 3 | `.vector_combine.{int_bool_to_vec2,bool_ivec2_to_vec3,bvec2_int_to_vec3}` | `shaders.conversions.vector_combine.int_bool_to_vec2_fragment` |
| `fcm_int_to_mat2` (matrix of int, bool and float lanes) | 12 | `.scalar_to_matrix.int_to_mat{2,3,4}` 3, `.matrix_combine.*` 9 | `shaders.conversions.scalar_to_matrix.int_to_mat2_fragment` |
| `fcm_rnd_int_lanes` | 9 | `shaders.random.{comparison_ops.fragment.26, exponential.fragment.87, scalar_conversion.combined.68, scalar_conversion.fragment.{26,40,68}, texture.fragment.{37,87,88}}` | `shaders.random.texture.fragment.88` |
| `fcm_eval_order` | 1 | `shaders.functions.misc.argument_eval_order_1_fragment` | the same |

By dEQP group: `shaders.conversions` 32, `shaders.random` 9,
`shaders.functions` 1. The partner vertex shader compiles in all 42.

## 2. What Mesa emits (Mesa 26.2.1, G610 drm-shim)

Every reference was drawn with `tools/mali/deqp_ref.sh <vert> <frag>
tools/mali/testdata/deqp_texmisc_<shape> --instrument shim` (Mesa 26.2.1's
gallium panfrost on drm-shim, one 64x64 draw with `--uniform-values`,
`pan_ref_diff.py extract --draw`). The shim was built by
`drm_shim_ref.sh build` from the 26.2.1 tarball, whose sha256 matches
`third_party/mesa-26.2.1/UPSTREAM`. Checks: the same build reproduces
`deqp_fmaloc_tk_bias_uniform.expect.json` byte for byte, and a second draw
of each of the 17 is identical. The G310 needs no references of its own
(`mali-g310-vs-g610.md`).

### 2.1 TexVertex: the harness folds into one fp16 compare

| Reference | Varying program (instructions) | Mesa's words |
|---|--:|---|
| `tv_2d_compare` | 16 | `TEX_SINGLE.slot0.reserved.16.2d.zero` at the coordinate `(0, 0)` (`MOV.i32 r0, 0x0` twice): an explicit level 0, fp16 texels. Then the harness of the four compares is **one** compare: `FADD.v2f16` / `FADD_IMM.v2f16` of the texels and the negated constants (`#0xB5C3B3AE`), three `FMAX.v2f16` of the `.abs` lanes (the max of the four distances), `FCMP_OR.v2f16.lt.m1` against `0.05`, `S16_TO_S32`, `CSEL.u32.eq r0, r0, 0, 0, 0x3F800000` (1.0 or 0.0), then `FMA.f32 r0, r0, 0x0, r0` before the varying store |
| `tv_cube_compare` | 25 | the cube sequence at the constant direction `vec3(0.0)`, not folded (as `deqp_fmaloc_tc_cube_const` in a fragment shader): `CUBEFACE1`, `CUBEFACE2_V9`, `CUBE_SSEL`, `CUBE_TSEL` on `0x0`, `FRCP.f32`, `FMA.f32 *0.5`, two `FMA.f32.clamp_0_1`, `MUX.i32.bit`, then `TEX_SINGLE...16.cube.zero` and the same nine-word harness |
| `tv_2d_array_compare` | 38 | three look-ups (`slot0..2`, one per element, each its own descriptor word) and their harnesses joined by `FMAX.v2f16`: one `FCMP_OR`, one `CSEL` for the whole product |
| `tv_rnd_2d` | 16 | as `tv_2d_compare`, both constants folded into `FADD_IMM.v2f16` |
| `tv_rnd_cube_array` | 64 | four cube look-ups sharing one face selection, the other uniforms' compares joined into the same `FMAX` chain; its fragment shader (68) runs the same on `cube.computed` |

How: Mesa's algebraic rules turn the product of `b2f(x < e)` terms into
`b2f(all(x < e))` and that into one compare of the maximum
(**inferred** from the words: one `FCMP_OR` per program, `FMAX` of `.abs`
lanes). The fragment program is the `v_vtxOut` colour pass-through (10
instructions).

### 2.2 TexKind: projection, bias pack, and look-ups Mesa removes

The fmaloc references (`mali-deqp-fmaloc.md` section 2, TEX_VERTEX) give the
projection (`FRCP.f32` of `.z` or `.w`, two `FMA.f32`, a plain
`2d.computed`), the varying or uniform bias (`F16_TO_F32`,
`FMA.f32.clamp_m1_1` by 1/16, `FMA.f32` by 4096, `F32_TO_S32.rtz`,
`U16_TO_U32` into the third staging word of `2d.computed_bias`) and the
constant bias (a FAU word). The new references add:

| Reference | FS instructions | Mesa's words |
|---|--:|---|
| `tk_rnd_proj_swizzle` (`.131`) | 14 | `texture2DProj(a, b).ywzx`: `LD_VAR_BUF_IMM.v3`, `FRCP.f32 r2`, two `FMA.f32 x*r`, `TEX_SINGLE...16.2d.computed`, then the `.ywzx` swizzle as two `MKVEC.v2i16` of the texel halves. Every other statement of the shader is dead (the last write wins) |
| `tk_rnd_proj_sum` (`.142`) | 20 | `texture2DProj(a, vec3(c, f, e) + vec3(d, c, b))`: the sum in fp16 (`V2F32_TO_V2F16`, `FADD.v2f16`, `FADD_IMM.v2f16 #0xC980C980`), three `F16_TO_F32`, then the projection and `2d.computed`. A computed coordinate: TexCoord's step too |
| `tk_rnd_bias_fragcolor` (`.124`) | 15 | `texture2D(a, b * b, gl_FragColor.y)`: `gl_FragColor.y` is a sum of constant globals, so the bias folds to its 8.8 word, a FAU constant (`MOV.i32 r4, u2.w0`, `0x000001cc`); the coordinate `FMA.v2f16 b*b`, two `F16_TO_F32`, `2d.computed_bias` |
| `tk_rnd_bias_two` (`.41`) | 18 | **no look-up**: both `texture2D(.., bias)` results are overwritten before `gl_FragColor`'s last write, and NIR's dead-code removal drops them. The words are the colour arithmetic alone (`S32_TO_F32` of int uniforms, `FMA.f32`, fp16 packs) |
| `tk_rnd_bias_proj` (`.108`) | 7 | **no look-up**: `gl_FragColor = a` is the last write; the three look-ups and the int arithmetic are dead. Two `V2F32_TO_V2F16` of the varying, `BLEND` |
| `tk_rnd_cube_bias` (`.121`) | 24 | `textureCube(a, vec3(e, int(true), d), c + b)`: `FADD.f32` of the two bias varyings, the cube sequence with the constant `1.0` lane (`CUBEFACE1 r1, 0x3F800000, 0x3F800000, r2`), the 8.8 bias pack (`FMA.f32.clamp_m1_1`, `FMA.f32`, `F32_TO_S32.rtz`, `U16_TO_U32`), `TEX_SINGLE...16.cube.computed_bias` |

### 2.3 FragCoordMediump: the int round trip in fp16

| Reference | FS instructions | Mesa's words |
|---|--:|---|
| `fcm_int_to_float` | 18 | `int(v_in0 * 1.0025)` is `FMA.v2f16 x, 0x3C03` (1.0025 in fp16) after the varying's narrowing (`FADD.f32 r0.h0`), `F16_TO_S32.rtz`, then `float(in0)` `S32_TO_F32` and `FADD.f32 r0.h0` (narrowed back to fp16). The `isOk` harness in fp16 (`FADD.v2f16`, `FMA.v2f16`, `FCMP_OR.v2f16.ge.f1`) |
| `fcm_ivec2_to_vec2` | 24 | the same per lane (`V2F32_TO_V2F16`, one `FMA.v2f16` for both lanes, two `F16_TO_S32.rtz`, two `S32_TO_F32`, two narrowings, `MKVEC.v2i16` of the pair), the harness, `LSHIFT_AND.v2i16` and `MUX.v2i16` for `all()` |
| `fcm_int_bool_to_vec2` | 22 | here `float(int(x))` is **`ftrunc`**: `F16_TO_F32`, `FROUND.f32.rtz`, `FADD.f32 .h0` (no integer word); the bool lane `FCMP_OR.v2f16.ne.f1` against 0 |
| `fcm_int_to_mat2` | 22 | the `ftrunc` form too (`F16_TO_F32`, `FROUND.f32.rtz`), the matrix's zero lanes folded (`MKVEC.v2i16 r0, 0x0.h0, r0.h0`), two `FADD.v2f16` and one `FMAX.v2f16` of the columns' distances |
| `fcm_rnd_int_lanes` (`.88`) | 11 | `float(int(int(b)))` of a highp lane: `F32_TO_S32.rtz`, `S32_TO_F32`, the narrowing, `MKVEC.v2i16` with a constant half, `FADD_IMM.v2f16 #0x42004000` |
| `fcm_eval_order` | 22 | integer arithmetic of the call (`IADD.s32`, `IADD_IMM.i32 #0x1`), `CSEL.u32.eq` of the `if`, `ICMP_OR.u32.eq` against the int uniform: an intval shape behind the fp16 one |

Which form Mesa uses for `float(int(x))` (`F16_TO_S32` + `S32_TO_F32`, or
`F16_TO_F32` + `FROUND.f32.rtz`) differs between the references: the
`ftrunc` form appears where the int value is used only by the conversion
back (`nir_opt_algebraic`'s `i2f(f2i(a))` rule), the integer words where the
NIR keeps the int (**inferred**; the lowering must follow each reference).

## 3. The refusing sites at `ca7cdc52`

| Text | Site (file:line) | What it refuses here |
|---|---|---|
| TexVertex | `src/shaderc/online/mali/nir.rs:4631` (the vertex walk's `IrOp::Alu` arm: a mediump operation of a mediump sampler's texels other than the fp16 product or sum arms `vs_alu16` / `vs_alu16u` at `nir.rs:4605-4616`) | the harness's `fsub`/`fabs`/compare/`bcsel` of the texels (all 60, **likeliest**: the look-up arm at `nir.rs:5150-5172` takes the 2D and cube look-ups at level 0 and the other two `TexVertex` sites, `nir.rs:5166` and `:5174`, refuse a reuse in another width and an `IrOp::Tex` without a level, neither of which these shaders have) |
| TexKind | `src/shaderc/online/mali/nir_fs.rs:6784-6786` (the fallback `ir::Op::Tex` arm: anything but `texture2D(sampler2D, coordinate)`) | projection and 2D bias (31 cases, and the cube cases whose 2D bias or projection comes first); `nir_fs.rs:6576-6577` (a mediump `textureCube` bias that is not a uniform lane) for `texturecube_bias`'s varying bias and `.121`'s sum. Other `TexKind` sites (`:3990` cube size, `:6533` mediump cube level, `:6639`, `:6675`, `:6718`, `:6746` levels, `:6775` shadow, `:11725` size branch, `:12582` look-up halves) are ES 3 forms no case here reaches |
| FragCoordMediump | `src/shaderc/online/mali/nir_fs.rs:2844` (`Fs::vec16_of`: two halves made one fp16 vector, one not fp16), `:2878`, `:2965-2974`, `:3016-3020` (`Fs::alu16`), `:8695` (`vectorise`'s narrowed-position pair) | the fp16 vector or operation whose lane is an int-converted value. Which of these sites each shape meets is **not confirmed**: no debugger on the host and no `src/` change in this item. `nir_fs.rs:5672` maps `alu16`'s FragCoordMediump to MixedPrecision in a highp shader without a `gl_FragCoord` read, so the mediump shaders here meet it outside that path (`vec16_of` is the likeliest) |

## 4. Lowering order (one session each)

Ranked by cases whose first refusal goes away, after what each depends on.
"Removed" is the first refusal; a case can still meet a later refusal, at
the link, or on the board.

| Step | What | Cases | Depends on | References |
|--:|---|--:|---|---|
| 1 | **TexVertex harness:** fp16 `fsub`/`fadd` of texels and a constant, `fabs`, `flt`, `bcsel 1.0/0.0`, and the product of such terms folded to one `FMAX` chain and one `FCMP_OR.v2f16.lt` + `S16_TO_S32` + `CSEL.u32.eq` (+ Mesa's `FMA.f32 r, r, 0, r`). One sequence for 2D, cube and array look-ups | 60 (2D 21, cube 17, array 22) | `intval-vs16` (mediump compares and selects in the vertex walk) and `arith-lower-1`'s `fabs`; 21 also need their fragment partner (FRAGCOLOR_LANE 12, the TEX_COORD cube sequence 9) | `tv_2d_compare`, `tv_cube_compare`, `tv_2d_array_compare`, `tv_rnd_2d`, `tv_rnd_cube_array` |
| 2 | **fp16 int round trip:** `f2i` of an fp16 lane (`F16_TO_S32.rtz`), `i2f` back (`S32_TO_F32` + narrowing), Mesa's `ftrunc` form (`F16_TO_F32` + `FROUND.f32.rtz`) where it folds, and the resulting lane accepted in `vec16_of` / `alu16`'s pairs and compares | 32 (`shaders.conversions`) | the int conversions of `intval-rest`; nothing else | `fcm_int_to_float`, `fcm_ivec2_to_vec2`, `fcm_int_bool_to_vec2`, `fcm_int_to_mat2` |
| 3 | **TexKind 2D projection:** `FRCP` of `.z`/`.w` and two products before the existing 2D look-up; the result's swizzle | 14: 6 of a varying read whole (`texture2dproj_vec{3,4}`, `.19`, `.23`, `.56`, `.131`), 3 built of lanes and constants (`.69`, `.94`, `.98`), 5 computed (`.80`, `.91`, `.97`, `.136`, `.142`) | none for the 6; the 3 built ones need the constant and int lanes (`.98`'s `int(c)`, intval); the 5 computed ones wait for fmaloc-design's computed coordinate | `deqp_fmaloc_tk_proj_vec{3,4}`, `_tk_rnd_proj`, `tk_rnd_proj_swizzle`, `tk_rnd_proj_sum` |
| 4 | **TexKind 2D bias:** the 8.8 bias pack (`clamp_m1_1`, `*4096`, `F32_TO_S32.rtz`, `U16_TO_U32`) of a uniform or varying, a constant bias as its FAU word, `2d.computed_bias`; plus dropping a look-up whose result is dead **before** the refusal (`.41`, `.108`: Mesa draws none) | 17 (bias 14, bias + projection 3, with step 3) | step 3 for the 3 mixed cases | `deqp_fmaloc_tk_bias_uniform`, `_tk_bias_varying`, `_tk_rnd_bias`, `tk_rnd_bias_fragcolor`, `tk_rnd_bias_two`, `tk_rnd_bias_proj` |
| 5 | **TexKind cube with bias / beside 2D kinds:** the bias pack on `cube.computed_bias` for a non-uniform bias | 11 | fmaloc-design's cube sequence (TEX_COORD `tc_cube_*`) for computed directions, steps 3 and 4 for the 2D look-ups beside | `deqp_fmaloc_tk_cube_bias`, `tk_rnd_cube_bias` |
| 6 | **FragCoordMediump random and function shapes** | 10 | step 2 and `intval-rest` (int arithmetic, `ICMP_OR`, the call's selects) | `fcm_rnd_int_lanes`, `fcm_eval_order` |

Overlaps:
- **fmaloc-design (TexCoord):** steps 3 (`.142`'s computed projected
  coordinate) and 5 (the cube directions) use its computed-coordinate and
  cube sequences; TexVertex's cube shape uses the same cube words in the
  vertex walk at a constant direction (`tc_cube_const`'s, in the vertex
  shader). The fmaloc note's proposed TEX_VERTEX lowering (projection 11,
  bias 16) is steps 3 to 5 here; this note's counts replace its 27 with 42 at
  the tip. One owner should take steps 3 to 5 with fmaloc-design's TexCoord
  steps.
- **arith items:** step 1 needs the vertex walk's `fabs` and the
  product-of-`b2f` fold (`arith-design`'s built-ins); step 6's random
  shaders also carry `367:ARITH`-family operations.
- **intval:** steps 1 (vertex compares and selects, `intval-vs16`), 2 and 6
  (`intval-rest`'s int conversions and arithmetic) wait for intval. The board
  counted 21 of FragCoordMediump's 42 as `intval`.
- **The text's name:** `FragCoordMediump` is raised by the generic fp16
  helpers, and here by shaders without `gl_FragCoord`. Renaming or splitting
  the text (a `MaliError` change) is left to the lowering item.

Forecast (HOST-ONLY, first refusals removed): step 1, 60; step 2, 32;
steps 3 to 5, 42; step 6, 10. Together 144, of which 21 (TexVertex) still
need their fragment partner lowered.

## 5. Issue comment

For issue #32 (texture look-up kinds, TEX_VERTEX) and issue #7 (plan #387).
connected posts it with the merge; this item posts nothing.

> Plan #387 `texmisc-ref` (commit: "mali: TexVertex, TexKind and
> FragCoordMediump refusals at the tip: dEQP shapes pinned against Mesa
> 26.2.1 references"). Measured on the host at `ca7cdc52`
> (`tools/mali/deqp-host-measure.sh`, HOST-ONLY, the 10,597 refused cases
> of the slices `37305608`): TexVertex 60 (all `uniform_api`'s
> `compare_vec4` harness on mediump texels in the vertex shader: 2D 21, cube
> 17, sampler array 22), TexKind 42 (2D projection 14, 2D bias 14, both 3,
> a cube look-up 11; 15 are new since fmaloc-ref), FragCoordMediump 42
> (none reads `gl_FragCoord`: `shaders.conversions`' `int(v * 1.0025)`
> round trip in fp16, 32; random int lanes 9; one function case). 17 new
> Mesa 26.2.1 drm-shim references (`tools/mali/testdata/deqp_texmisc_*`),
> pinned as refused in `tests/mali_deqp_texmisc.rs`; with fmaloc's 7
> `tk_*` references every shape has one. Mesa folds the vertex harness into
> one `FCMP_OR.v2f16` of an `FMAX` chain, removes dead look-ups entirely
> (`random.texture.fragment.41`, `.108`), and writes `float(int(x))` either
> as `F16_TO_S32` + `S32_TO_F32` or as `FROUND.f32.rtz`. Lowering order in
> `docs/notes/mali-deqp-texmisc.md` section 4: vertex harness (60, after
> intval-vs16), fp16 int round trip (32), projection (14), bias (17), cube
> (11), the rest (10).

## 6. Caveats

- The shape rules are this note's own; `deqp-host-measure.sh`'s `RULES`
  classifies none of the three texts beyond one shape.
- One reference per shape (two for the larger TexKind shapes with fmaloc's).
  Precision and size variants (`vec3`/`vec4`, `mat3`/`mat4`, `_both` pairs)
  are not drawn separately; a lowering checks them against references of its
  own.
- The refusing site of FragCoordMediump per shape is not confirmed (section
  3).
- `tests/mali_deqp_texmisc.rs` is not in `tools/check.sh`'s list of
  `mali_deqp_*` files; adding it is left to the plan.
