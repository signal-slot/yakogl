<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): FMA, LOCALS and texture coordinate refusals, the dEQP shapes against Mesa 26.2.1

Plan #387, item `fmaloc-ref` (ROADMAP stage 5, issue #7). It covers the four
refusal texts that plan #367 held through glmark2 shapes only. Before this
item, none of their dEQP-GLES2 shapes had a Mesa reference
(`docs/notes/mali-deqp-families.md` section 2):

| Family | Refusal text (`MaliError`) |
|---|---|
| FMA | "a fused multiply-add other than a matrix product's (a built-in function's)" (`FusedMultiplyAdd`) |
| LOCALS | "a shader with more than 32 local variables" (`TooManyLocals`; since `74308a71` the text says 256) |
| TEX_COORD | "a texture coordinate other than a vec2 varying read whole or two constants" (`TexCoord`) |
| TEX_VERTEX | "a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)" (`TexKind`) |

The item changes no compiler code. It commits 22 references,
`tools/mali/testdata/deqp_fmaloc_<shape>.{vert,frag,expect.json}`, and
`tests/mali_deqp_fmaloc.rs`. All 22 are refused at the tip: each test is
ignored, and `refusals_pinned` pins its refusal text.

**HOST-ONLY.** Every count at the tip below comes from `deqp-gles2` run on the
host against the Mali back end's C pair (`tools/mali/deqp-host-measure.sh`).
Only the compile and link outcomes of that run are real. A board status
(Fail, NotSupported) is the NanoPC-T6's own, taken from the slices.

- **Tip:** `cc89a1cf3085f1ba50ee28986c809867396b079d` (plan #387's integration
  tip; it contains `74308a71` locals and `5244a7f5` TEX_COORD).
- **CTS:** VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `tools/cts/cts-yakogl-target.patch` applied (uncommitted) and its external
  sources fetched.
- **Board counts:** the four slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`. For each case the
  first row of `refusals.tsv` is its first refusal.
- **Cases measured:** the 396 cases whose first board refusal is one of the
  four texts (100 FMA, 160 LOCALS, 109 TEX_COORD, 27 TEX_VERTEX; 380 Fail,
  16 NotSupported). They were put into a one-slice directory (`results.tsv` and
  `refusals.tsv` of these cases only), which the script takes through
  `--slice`:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --slice <dir with the 396 cases' results.tsv and refusals.tsv> \
    --out <out> --jobs 3 --chunk 50
```

All 396 cases began, and no process crashed or timed out (8 processes, all
exit 1, which is dEQP's "not every case passed").

## 1. The counts per text: 37305608 (board) and the tip (host)

| Text | Board first refusal at `37305608` | Host first refusal at `cc89a1cf` | Moved to another text | Compiles on the host |
|---|---:|---:|---:|---:|
| FMA | 100 (Fail 100) | **63** (all Fail, all VS) | 0 | 37 |
| LOCALS | 160 (Fail 144, NS 16) | **0** | 145 | 15 |
| TEX_COORD | 109 (Fail 109) | **104** (all Fail, all FS): 101 of its own and 3 from LOCALS | 8 | 0 |
| TEX_VERTEX | 27 (Fail 27) | **27** (all Fail, all FS) | 0 | 0 |

- **FMA, 37 compile now:** all 37 cases are `mix` (16), `mod` (6),
  `smoothstep` (9) and `reflect` of a vec2, vec3 or vec4 (6) in a vertex
  shader. The `is_mix`, `is_smoothstep_inner`, `vs_fmod` and `is_reflect`
  paths of plan #367 take them. Whether the words are byte-equal to Mesa's for
  these dEQP shapes is not measured here. The `gm2_fma_*` references pin only
  glmark2's forms.
- **LOCALS is gone:** no case has it as its first refusal. With `74308a71`
  (256 locals) the 160 cases go on to other refusals:
  - 50 `uniarray` (a uniform array);
  - 43 `unitype` (26 fragment mediump, 13 vertex not highp, 4 other type);
  - 24 `367:ARITH`;
  - 6 `constop`;
  - 4 `loops`;
  - 4 "a fragment colour of more than 255 vectorised operations and operands";
  - 3 `FRAGCOLOR_LANE`;
  - 3 TEX_COORD (the `uniform_api.random` cube cases below);
  - 2 `CONTROL_FLOW`;
  - one each `varylink`, `mixprec`, `CONST_FOLD` and `intval`;
  - 2 back end **defects** (below);
  - 15 compile on the host.

  The cases are mostly `uniform_api.*` (struct and array uniforms),
  `shaders.indexing.tmp_array` and `shaders.random`. The LOCALS refusal itself
  needs no shape and no reference. Mesa has no count like ours to spill or
  keep: `nir_lower_vars_to_ssa` turns every local into SSA values before the
  back end, and only registers can spill (`nir.rs:66-68` says the same of
  `MAX_LOCALS`). A shape that needs more than 32 work registers is a
  different text (`misc`'s `work_registers`).
- **TEX_COORD, 8 moved:** the 8 `uniform_api.*.basic.samplerCube_both` cases
  now stop first in their *vertex* shader, with "a vertex shader texture
  look-up other than 2D or cube look-ups at a level (a size query), or its
  fp16 texels read by a mediump operation other than a product or sum".
  That text belongs to no family here.
- **Defect (new work, not this item's):**
  `attribute_location.bind_aliasing.max_cond_vec2` and `max_cond_vec3` (board:
  LOCALS) are refused at the tip with "Valhall back end defect: an invariant
  of the pipeline did not hold (Encode)". That is a `Defect`, not a lowering
  gap.

## 2. The shapes and their references (at the tip)

The shape is the construct of the refusing shader. The representative is one
case of the shape; its pair is taken verbatim from the `.qpa`, and each
`.license` sidecar names the case. Every reference was drawn with
`tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_fmaloc_<shape>
--instrument shim`, that is Mesa 26.2.1's gallium panfrost on drm-shim, one
64x64 draw with `--uniform-values`, `pan_ref_diff.py extract --draw`. The shim
was built by `drm_shim_ref.sh build` from the 26.2.1 tarball, whose sha256
matches `third_party/mesa-26.2.1/UPSTREAM`. Checks:
- the same build reproduces `gm2_cf_ifelse_vs` and `gm2_cf_ifelse_fs` byte for
  byte (`README-deqp_ref.md`'s self-check);
- a second draw of `tc_cube_face`, `fma_atan2` and `tk_bias_uniform` is
  identical.

### FMA: 63 cases, all vertex shaders, all Fail; the partner fragment shader compiles in all 63

| Shape `deqp_fmaloc_*` | Cases | dEQP groups | Representative case (`dEQP-GLES2.functional.`) | What Mesa emits |
|---|---:|---|---|---|
| `fma_constmat_mat` | 9 | `shaders.matrix.mul.const_{lowp,mediump,highp}_mat{2,3,4}_mat{2,3,4}` | `shaders.matrix.mul.const_highp_mat3_mat3_vertex` | `const matN * const matN`: GLSL folds the product and the colour. The link moves the constant colour into the fragment shader (no varying program; FS `MOV.i32 r0, u2.w0` / `r1, u2.w1` of the FAU fp16 colour, 7 instructions) |
| `fma_constmat_vec` | 9 | `..._mat{2,3,4}_vec{2,3,4}` (const) | `shaders.matrix.mul.const_highp_mat3_vec3_vertex` | `const mat3 * attribute vec3`: the zero entries fold out and the rest are per-lane `FMA.f32` with FAU constants (`FMA.f32 r0, u5.w1, r2, r0^`). Varying program of 13 instructions |
| `fma_attrmat_vec` | 9 | `shaders.matrix.mul.dynamic_*_mat{2,3,4}_vec*` | `shaders.matrix.mul.dynamic_highp_mat3_vec3_vertex` | `attribute mat3 * attribute vec3`: three `LD_ATTR_IMM.v3`, then `FMA.f32` chains (8 `FMA.f32`, 1 `FADD.f32`). 26 instructions |
| `fma_attrmat_unimat` | 6 | `shaders.matrix.mul.dynamic_*_mat4_mat4`, `shaders.matrix.mul_assign.dynamic_*_mat4` | `shaders.matrix.mul.dynamic_highp_mat4_mat4_vertex` | `attribute mat4 * uniform mat4`: 48 `FMA.f32` and 9 `FADD.f32` over FAU uniform words. 73 instructions |
| `fma_asin` | 8 | `shaders.operator.angle_and_trigonometry.{asin,acos}.*` | `shaders.operator.angle_and_trigonometry.asin.highp_float_vertex` | the GLSL built-in's polynomial: `FMA.f32 r0, r2.abs, u5.w1, r0^` three times, then `FADD 1-|x|`, `FRSQ` + `FRCP` (sqrt), sign by two `FCMP_OR.f32.lt.f1` and `FADD`, a small-x branch selected by `CSEL.u32.eq`. 35 instructions |
| `fma_atan` | 5 | `atan.*` (4), `shaders.random.trigonometric.vertex.22` | `shaders.operator.angle_and_trigonometry.atan.highp_float_vertex` | `FRCP` of x, `FCMP_OR.f32.ge` against 1.0 and `CSEL` (x or 1/x), a six-term `FMA.f32` polynomial, a `CSEL` of the -pi/2 offset, then the sign copied by `LSHIFT_AND`/`LSHIFT_OR`. Then the mediump `* 0.5 + 0.5` colour is one `FMA.v2f16` with `F16_TO_F32` back out. 37 instructions |
| `fma_atan2` | 4 | `atan2.*` | `shaders.operator.angle_and_trigonometry.atan2.highp_float_vertex` | `nir_atan2`: 11 `FMA.f32`, 2 `FRCP`, 8 `CSEL`, 5 `FCMP_OR`. 57 instructions |
| `fma_reflect_float` | 2 | `shaders.operator.geometric.reflect.{highp,mediump}_float_vertex` | `shaders.operator.geometric.reflect.highp_float_vertex` | `reflect(I, N)` for scalars: `FADD.f32 r1, r0, r0` (2N), `FMA.f32 r0, r4, r0^, 0x0.neg` (N*I), `FMA.f32 r0, r1^.neg, r0^, r4^`. `is_reflect` wants an `fdot`, and the scalar form has a plain product |
| `fma_const_asin` | 9 | `shaders.constant_expressions.builtin_functions.trigonometry.{asin,acos}_*` (8), `shaders.random.all_features.vertex.60` (global `acos` of constants) | `shaders.constant_expressions.builtin_functions.trigonometry.asin_vec2_vertex` | GLSL folds the constant; `out0` is constant, the link moves it into the FS (no varying program). The FS is the `isOk` harness in fp16 (`FADD.v2f16`, `FMA.v2f16`, `FCMP_OR.v2f16.ge`, 10 instructions) |
| `fma_const_reflect` | 1 | `constant_expressions.builtin_functions.geometric.reflect_float_vertex` | the same | as `fma_const_asin` (11 FS instructions) |
| `fma_const_matmat` | 1 | `constant_expressions.operators.math_mat_vertex` | the same | as `fma_const_asin` (11 FS instructions) |

What Mesa does with an FMA. Mesa's GLSL built-ins (`asin`, `acos`, `atan`,
`reflect`, a matrix product) reach NIR as `fmul` and `fadd` trees, or as
`ffma` where the built-in writes one. Bifrost and Valhall set
`float_mul_add16` and `float_mul_add32` to `has_ffma | fuse`
(`M:panfrost/compiler/bifrost/bifrost_compile.h:128-131`). With that,
`nir_opt_algebraic`'s late pass rewrites `fadd(contract)(fmul(a, b), c)` into
`ffma(a, b, c)` when the product is used only by `fadd`s
(`M:compiler/nir/nir_opt_algebraic.py:3796-3818`, `is_only_used_by_fadd`).
A constant built-in (`const float cval = asin(...)`, `const mat3 a =
mat3(3.0) * mat3(4.0)`) is a GLSL constant expression, so it is folded before
NIR and no FMA reaches the back end at all (**inferred** from the words: no
varying program, the colour a FAU constant). Where NIR itself folds an
`ffma`, it uses `fmaf` (`M:compiler/nir/nir_opcodes.py:1159-1160`).

Our vertex walk (`nir.rs`, the `AluOp::Ffma` arm at `nir.rs:4781`) takes a
front-end `ffma` only as `mix`, `smoothstep`, `reflect` of a vector (an
`fdot`), `cross` or `mod` (`vs_fmod`). Every other `ffma` is refused, and
that includes one of constants: the walk does not fold a constant `ffma`
before it refuses.

Proposed lowering (for the lowering item, in this order):
1. **The constant fold (`fma_const_*`, `fma_constmat_mat`: 20 cases).** Fold
   an `ffma` whose three sources are all constants (`a*b+c` in f32, rounded
   once, as `fmaf` does). Then the existing constant-varying link moves the
   colour into the fragment shader. The references pin the fragment words
   that result: the `isOk` harness, or the FAU colour.
2. **A matrix product (`fma_constmat_vec`, `fma_attrmat_vec`,
   `fma_attrmat_unimat`: 24 cases).** The front end writes `matN * v` as one
   `fmul` and N-1 `ffma`s (`select.rs`'s
   `a_matrix_times_a_vector_is_a_mul_and_three_mads`). The walk already takes
   the position's `mat4 * vec4` (`gm2_*`). It has to be generalised to
   N = 2 and 3, to a constant matrix whose zero entries Mesa folds out, to a
   matrix attribute's columns, and to a uniform matrix's FAU words.
3. **The trigonometric polynomials (`fma_asin`, `fma_atan`, `fma_atan2`: 17
   cases).** Take a front-end `ffma` of any non-constant sources as one
   `FMA.f32`, plus the built-ins' `FCMP_OR`/`CSEL` selects and, for `atan`,
   the sign copy (`LSHIFT_AND`/`LSHIFT_OR` with `0x80000000`). The selects
   are boolean values, so this probably waits for `intval`'s compares and
   selects (**inferred**).
4. **`reflect(float, float)` (2 cases):** accept a plain `fmul` in place of
   `fdot` in `is_reflect`.

### TEX_COORD: 104 cases, all fragment shaders, all Fail; the partner vertex shader compiles in all 104

| Shape `deqp_fmaloc_*` | Cases | dEQP groups | Representative case | What Mesa emits |
|---|---:|---|---|---|
| `tc_cube_face` | 86 | `texture.specification.{basic_teximage2d,basic_texsubimage2d,texsubimage2d_empty_tex}.*_cube`, `{teximage2d,texsubimage2d}_align.cube_*`, `random_teximage2d.cube_*` | `texture.specification.basic_teximage2d.rgba8888_cube` | `textureCube(s, vec3(-1, -y, +x))` with x and y computed in fp16 (`FMA.v2f16 r0, r0^.h10, 2.0, -1.0`, `FADD.v2f16` negate, `F16_TO_F32`). Then Mesa's cube sequence: `CUBEFACE1`, `FRCP.f32`, `FMA.f32 *0.5`, `CUBEFACE2_V9`, `CUBE_TSEL`, `CUBE_SSEL`, `FMA.f32.clamp_0_1` twice, `MUX.i32.bit` of the face into s. Then `TEX_SINGLE...cube.computed`. 22 instructions |
| `tc_cube_const` | 10 | `uniform_api.{value.*,info_query}.basic.samplerCube_fragment` (7), `uniform_api.random.{27,48,54}` (3, board LOCALS) | `uniform_api.value.initial.render.basic.samplerCube_fragment` | `textureCube(u, vec3(0.0))`: Mesa does **not** fold the constant direction. The same `CUBEFACE1`... sequence runs on zero operands (`FMA.f32`, `CUBE_SSEL`/`TSEL`, `MUX`), then the `compare_vec4` harness in fp16. 28 instructions |
| `tc_cube_rnd_sub` | 7 | `shaders.random.texture.fragment.{8,16,20,22,92,111,126}` | `shaders.random.texture.fragment.16` | `textureCube(a, c - b)` of two mediump vec3 varyings: `FADD.v2f16`, `F16_TO_F32` back to f32, then the cube sequence. 29 instructions |
| `tc_2d_scalar` | 1 | `shaders.random.texture.fragment.12` | the same | `texture2D(a, vec2(c, b))`, c a constant `true` and b a float varying: `MOV.i32 r2, 0x3F800000`, `MOV.i32 r3, r0^` (the scalar load), `TEX_SINGLE...2d.computed`. 10 instructions |

The 86 `texture.specification` cases draw six faces each, one program per
face (`vec3(-1, -y, +x)`, `vec3(+1, -y, -x)`, ...). The host's first refusal
is the first face. A lowering must cover all six sign and swizzle forms;
only the first is pinned here.

Proposed lowering: the cube coordinate sequence above (`TEX_SINGLE` with
`cube.computed` after the `CUBEFACE1`/`CUBEFACE2_V9`/`CUBE_SSEL`/`CUBE_TSEL`
words), from a computed or constant `vec3`, as `mali-texture-refusals.md` and
`mali-tex-coords.md` (issue #33) design for the computed 2D coordinate.
`tc_cube_face` alone is 86 cases.

### TEX_VERTEX: 27 cases, all fragment shaders, all Fail; the partner vertex shader compiles in all 27

The text names the look-up *kind* (`MaliError::TexKind`), not the vertex
stage. The family's name is plan #367's.

| Shape `deqp_fmaloc_*` | Cases | dEQP groups | Representative case | What Mesa emits |
|---|---:|---|---|---|
| `tk_bias_uniform` | 4 | `texture.mipmap.2d.bias.*` | `texture.mipmap.2d.bias.linear_linear` | `texture2D(s, v, u_bias)`, the bias a mediump uniform: `F16_TO_F32 r0, u4.w0.h0`, clamped (`FMA.f32.clamp_m1_1` by 1/16, FAU `3d800000`), scaled (`FMA.f32` by 4096, `45800000`), so 8.8 fixed point. Then `F32_TO_S32.rtz` and `U16_TO_U32` into the third staging register of `TEX_SINGLE...2d.computed_bias`, then the case's colour scale and bias (`FMA.v2f16` twice). 16 instructions |
| `tk_bias_varying` | 1 | `shaders.texture_functions.fragment.texture2d_bias` | the same | the same pack of a varying bias (`LD_VAR_BUF_IMM.v3` loads coordinate and bias together). 12 instructions |
| `tk_proj_vec3` | 1 | `shaders.texture_functions.fragment.texture2dproj_vec3` | the same | `FRCP.f32` of `.z`, two `FMA.f32 x*r, y*r`, then a plain `2d.computed` look-up. 11 instructions |
| `tk_proj_vec4` | 1 | `...texture2dproj_vec4` | the same | the same fragment words as `tk_proj_vec3`, dividing by `.w`. The varying program loads the vec4 attribute but stores three lanes, and the FS loads three (`LD_VAR_BUF_IMM.v3`): the link drops the unread `.z`. 11 instructions |
| `tk_cube_bias` | 1 | `...texturecube_bias` | the same | the cube sequence of `tc_cube_face` plus the bias pack. 24 instructions |
| `tk_rnd_proj` | 9 | `shaders.random.texture.fragment.{19,56,69,80,91,94,97,99,115}` | `shaders.random.texture.fragment.19` | `texture2DProj(a, b)`: as `tk_proj_vec3` |
| `tk_rnd_bias` | 10 | `shaders.random.texture.fragment.{7,17,31,48,52,59,63,84,127,147}` | `shaders.random.texture.fragment.48` | `texture2D(a, c, float(b))` with b a constant int 4: the packed bias is a FAU word (`0x00000400`, 4.0 in 8.8) moved into the staging register (`MOV.i32 r2, u2.w0`), with no conversion words |

Proposed lowering: the projection (an `FRCP` and two products in front of
the existing 2D look-up: 11 cases) and the bias (clamp, scale, `F32_TO_S32`,
`U16_TO_U32`, or a FAU word for a constant, and `computed_bias`: 16 cases).
Four of the random cases (`fragment.52`, `.115`, `.127`, `.147`) also have a
cube look-up, which waits for the TEX_COORD cube sequence.

## 3. Forecast (HOST-ONLY, first refusals)

| Lowering | First refusals removed at the tip | Partner shader |
|---|---:|---|
| FMA constant fold | 20 | compiles (all 20) |
| FMA matrix products | 24 | compiles |
| FMA trigonometric polynomials (after the `intval` select) | 17 | compiles |
| FMA scalar `reflect` | 2 | compiles |
| TEX_COORD cube sequence (`tc_cube_face`, `tc_cube_const`, `tc_cube_rnd_sub`) | 103 | compiles |
| TEX_COORD scalar-and-constant 2D coordinate | 1 | compiles |
| TEX_VERTEX projection | 11 | compiles |
| TEX_VERTEX bias (2D) | 15 | compiles |
| TEX_VERTEX cube bias | 1 | compiles |
| **Total** | **194** | |

"Removed" means that the case's first refusal goes away. The link and the
draw were not measured, so a case can still fail at a later refusal in the
same shader, at the link, or on the board. The 7 random cube cases and the
`shaders.random.texture` cases with several look-ups need every one of their
look-ups lowered.

Only FMA (63) and TEX_COORD (104) of these texts are still at or above the
plan's 100-case bar together; taken alone, FMA is below it at the tip.
TEX_COORD's `tc_cube_face` (86) is the largest single shape. LOCALS (0) no
longer counts.

## 4. Issue comment

For issue #33 (FMA and TEX_COORD) and issue #32 (TEX_VERTEX, the texture
look-up kinds). connected posts it with the merge; this item posts nothing.

> Plan #387 `fmaloc-ref` (commit: "mali: fma, locals and texture coordinate
> refusals, the dEQP shapes pinned against Mesa 26.2.1 references"). The
> dEQP-GLES2 cases whose first board refusal at `37305608` was FMA (100),
> LOCALS (160), TEX_COORD (109) or TEX_VERTEX (27) were measured on the host
> at `cc89a1cf` (`tools/mali/deqp-host-measure.sh`, HOST-ONLY). FMA: 63 left,
> 37 compile now (`mix`, `mod`, `smoothstep` and vector `reflect`). LOCALS:
> 0 left, all 160 moved to other texts (uniarray 50, unitype 43, ARITH 24,
> ...) or compile (15). TEX_COORD: 104 (86 of them `textureCube` in
> `texture.specification.*_cube`). TEX_VERTEX: 27 (bias 16, projection 11).
> 22 shapes have a Mesa 26.2.1 drm-shim reference
> (`tools/mali/testdata/deqp_fmaloc_*`), pinned as refused in
> `tests/mali_deqp_fmaloc.rs`. Shapes, Mesa's words and the proposed lowering
> are in `docs/notes/mali-deqp-fmaloc.md`. New: two `attribute_location.
> bind_aliasing.max_cond_vec{2,3}` vertex shaders hit "Valhall back end
> defect: an invariant of the pipeline did not hold (Encode)".

## 5. Caveats

- The shape rules are this note's own: dEQP group and name, then the
  built-in or look-up in the source. They are not part of
  `deqp-host-measure.sh`'s `RULES` table, which classifies no `367:*` family.
- One reference per shape. Precision variants (lowp, mediump) of the matrix
  and trigonometric cases are not drawn separately; a lowering checks them
  against new references of its own.
- `tests/mali_deqp_fmaloc.rs` is not in `tools/check.sh`'s list of
  `mali_deqp_*` files (the gate row of plan #377's families). Adding it there
  is left to the plan (the file runs as-is: one test passes and 22 are
  ignored).
