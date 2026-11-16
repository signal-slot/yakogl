<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: dEQP-GLES2 constant operations (constop): constant built-in calls, constant colour lanes, -0.0 through a variable, fp16 constants

**Study, plan #377 item study-constop** (issue #20; nothing under `src/`
changed). It covers three refusals of `src/shaderc/online/mali/mod.rs` that
the Mali-G610 tree at `37305608` (docs/es2-cts-failures.md section M, slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`) reports:

| `MaliError` | Text (`mod.rs`) | Cases |
|---|---|---|
| `ConstantFold` | `:449` "a constant operation on or to a value that is not a finite normal float" | FS 489 Fail + 6 NotSupported, VS 114 Fail (and 9 VS Pass) |
| `NegZeroShared` | `:440` "a -0.0 that a comparison reads through a variable, or that another value reads too" | VS 70, FS 47 |
| `Fp16Inexact` | `:531` "a constant fp16 does not hold" | 3 (vertex cases, at the relink) |

Mesa is 26.2.1 (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`); `G/` is its
`src/compiler/glsl/`, `N/` `src/compiler/nir/`, `P/` `src/panfrost/compiler/`.
Line numbers of our code are those at `38ad30eb` (no file under
`src/shaderc/online/mali/` differs from origin/main `dab58772`).

Plan #367 holds `CONST_FOLD` and `ARITH_CONST` as glmark2 meets them (section
5). This note takes the dEQP shapes left beside them.

## 1. Method

The tree's cases were sorted from `refusals.tsv` by test group. For each group
the dEQP shader was rebuilt from its generator at VK-GL-CTS `93bca01` (the
revision `DEQP_WORK` pins; `modules/glshared/glsShaderConstExprTests.cpp`,
`glsShaderLibraryCase.cpp`, `modules/gles2/functional/es2fShaderOperatorTests.cpp`,
`es2fShaderMatrixTests.cpp`, `es2fShaderStructTests.cpp`, read on GitHub). Each
shader went through the Mali compiler on the host
(`--features host-test,gpu-mali,shaderc-online,no-shader-table`,
`SHC_ONLINE.compile`, and `compile_vs_linked` for a pair). A temporary marker
on every `MaliError::{ConstantFold, NegZeroShared, Fp16Inexact}` site recorded
which site refused. The marker and the probe test were removed afterwards and
nothing of them is committed.

Check of the reconstruction: the template predicts each case's per-stage
verdict, and it matches the tree. `ceil`, `sqrt`, `sign`, `mix`, `asin`,
`length(1.0)`, `distance(1.0, 2.0)` and `refract(1.0, 1.0, 0.5)` are refused
`ConstantFold` in the fragment variant only. Their vertex variants fail
earlier on `Arithmetic` or `FusedMultiplyAdd`, as the tree shows. `step` is
refused in the vertex variant only, and `step_*_fragment` Passes on the board.
`radians`, `degrees` and `dot(1.0, 1.0)` compile and are not in the list.

The `random.*` groups (195 `ConstantFold`, 113 `NegZeroShared`) are written by
dEQP's random shader generator from a seed. Their sources come from the
`measure` item's host run (`tools/mali/testdata/deqp_shapes/`), and they are
counted here only as groups.

## 2. The shapes and their counts

`ConstantFold`, 618 cases (Fail unless noted). The sites are those the probe
recorded:

| Shape | Groups | FS | VS | Site |
|---|---|---|---|---|
| **S1a** a built-in call on constants (`const T cval = floor(T(37.3));`) | `constant_expressions.builtin_functions` | 50 | 64 | `nir.rs:2051` |
| **S1b** the same through a dot product or `mix` (an `ffma` of constants) | idem (`dot`/`length`/`normalize`/`distance`/`reflect`/`refract` vec2-4, `mix` and `asin` FS) | 29 | 18 | `nir.rs:2004` |
| **S1c** the same with a bool constant operand (`bcsel` of a folded compare) | `atan_separate_*_fragment` | 4 | 0 | `nir.rs:1997` |
| **S2a** mediump FS colour: `vec4(0.0, 0.0, 0.0, 1.0)` lanes times the scale, plus the bias | `operator.{binary_operator (90), unary_operator (18), geometric (26), common_functions (9)}` | 143 | 0 | `nir_fs.rs:1065`, `:1097` |
| **S2b** the same where `x * 57.29578` (`degrees`) or `x * 0.01745` (`radians`) meets the scale: a constant product not exact in fp16 | `operator.angle_and_trigonometry` | 8 | 0 | `nir_fs.rs:1087` |
| **S2c** FS matrix `++`/`--`: `vec4(.., 1.0) + vec4(.., 0.0)` | `matrix.{pre,post}_{in,de}crement.dynamic_*` | 36 | 0 | `nir_fs.rs:1097` |
| **S2d** FS constant matrices `const mat in0, in1; res = in0 op in1` | `matrix.{add,sub,mul}.const_*_fragment` | 27 | 0 | `nir_fs.rs:4523` (add, sub, mat4 mul), `:1065` (mat2/mat3 mul) |
| **S3** an unrolled float counter `i++` (`fadd(0.0, 1.0)`, ...) | `loops.{for,while,do_while}_constant_iterations.basic_*_float_fragment` | 3 + 6 NS | 0 | `nir_fs.rs:1097` |
| **S4** random shaders | `random.{basic_expression 48, texture 30, conditionals 26+8, scalar_conversion 25, comparison_ops 13+3, swizzle 13, exponential 12+5, trigonometric 10+1, all_features 1}` (FS+VS) | 178 | 17 | not measured |
| **S5** constants through structs, functions and compares | `struct.local.parameter_{in,}out_* (6 VS)`, `fragment_ops.interaction.basic_shader (3+3)`, `constant_expressions.{operators.math_mat, operators.selection, complex_types.struct}`, `function.*_variable_aliasing`, `functions.datatypes.struct_constructor_highp_in_fragment`, `misc.compound_assignment.mul_swizzled_vec_by_matrix`, `preprocessor.definitions.define_value_and_function_fragment`, `conditionals.if.constant_conditional_assignment_to_matrix`, `state_query.shader.uniform_value_sampler` | 11 | 15 | `nir.rs:2051` (`struct.local`, a compare of constants), `nir.rs:1997` (a `?:` on a constant bool) |
| (Pass) `shader_api.shader_source.split_source_*_fragment` | the test checks no draw | 0 | 9 Pass | - |

`NegZeroShared`, 117 cases. The random groups are `all_features` (19 FS, 30
VS), `trigonometric` (19, 19) and `conditionals` (8, 18). The others are
`fragment_ops.interaction.basic_shader.{42,91}` (VS) and
`functions.array_arguments.copy_local_inout_on_call_{fragment,vertex}`. Each
probe form refused at `glsl.rs:1912` (shape **N1**, section 3).

`Fp16Inexact`, 3 cases: `matrix.sub.const_{highp,mediump,lowp}_mat3_mat3_vertex`,
refused by `compile_vs_linked` (the log says "fragment shader", the stage the
relink builds; shape **H1**).

### What each shape needs after its fold (the next refusal)

- **S1, vertex variants (82)**: the program's fragment shader is ShaderLibrary's
  mediump compare (`bool RES = isOk(out0, ref_out0, 0.05)`). Today it refuses
  `IntValue`: 37 `builtin_functions.*_vertex` cases already stop there, and so
  do `trivial.float_vertex` and `operators.math_float_vertex`, while their
  `_fragment` twins Pass. Folding S1 moves the 82 vertex cases to `IntValue`
  (the study-intval / lower-intval family). They gain nothing until that lands.
- **S1, fragment variants (83)**: the highp FS-only template. `degrees`,
  `radians`, `step` and `dot(1.0, 1.0)` show the program compiles once its
  constant is a literal, so these are expected to Pass.
- **S2a-S3, highp variants**: the inputs and `res` are highp while `color`
  is `mediump`. 56 of the 143 S2a cases are highp, and of the 196 S2a, S2b,
  S2c and S3 cases 71 are highp, 71 mediump and 54 lowp. Once the colour
  folds, the highp ones can stop at `MixedPrecision` (the study-mixprec
  family). The 125 lowp and mediump ones are the expected gain.
- **S2d**: its first site is #367's (section 5). It is re-measured after
  #367's lowering.

## 3. The refusal sites

`ConstantFold`:

| Site | What refuses | Shapes |
|---|---|---|
| `nir.rs:1996-1998` | `alu_sized`, all operands constant: an operand that is not `plain` (`nir.rs:800-803`: a zero or a normal float). A boolean `true` (`0xffffffff`, a NaN pattern after `pan_nir_lower_bool_to_bitsize`), an infinity, a NaN or a denormal | S1c, S5 (`true ? 1.0 : 2.0` refuses here) |
| `nir.rs:2004` | a constant `Ffma` ("no_std has no fma") | S1b; #367's `gm2_arithconst_3/4` VS and glmark2's light-basic `normalize(LightSourcePosition.xyz)` stop here too |
| `nir.rs:2010-2051` | every other op on constants, on the premise "the front end folds a constant expression": `Fclamp`, `Frsq`, `Ffloor`, `Fceil`, `Ftrunc`, `Fcmp`, `Bcsel`, `B2f`, the conversions, the int ops, `Fsin`/`Fcos`/`Flog2` | S1a, S5 |
| `nir.rs:2062-2064` | a folded `Fadd`/`Fmul`/`Frcp`/`Fmin`/`Fmax` whose result is not `plain` (an overflow to infinity, a denormal): probe `3.0e38 * 10.0`, `1.0e-30 * 1.0e-15` | none outside S4 seen |
| `nir_fs.rs:1065` | a mediump lane `fmul` of two constants ("the front end folds them") | S2a, S2d (mul) |
| `nir_fs.rs:1087` | `fmul(#a, fmul(b, #c))`: the folded `a * c` not exact in fp16 | S2b |
| `nir_fs.rs:1097` | a mediump lane `fadd` of two constants | S2a (bias only), S2c, S3 |
| `nir_fs.rs:4520-4524` | a mediump IR `Fmul`/`Fadd` of constants: an operand or result not exact in fp16 | S2d; #367's `gm2_constfold_2..6` |
| `nir_fs.rs:4596` | a mediump compare of two constants that `nir::fold_int` does not fold | none seen |
| `nir_fs.rs:5587-5592` | `Terms::fold` (the reassociated sum tree): an operand or result not `plain` | none seen |

`NegZeroShared` (`glsl.rs`, `Tree::neg_zero_compares` and `plus_zero`,
`:1793-1917`, which carry out `N/nir_opt_algebraic.py:771-776` on the IR):

| Site | What refuses |
|---|---|
| `glsl.rs:1838-1843` | a compare with a -0.0 side whose other side may fold to a constant |
| `glsl.rs:1894-1896` | `fneg` of a value that may fold to a zero the rule cannot name (`-abs(0.0)`, a local stored twice) |
| `glsl.rs:1899-1903` | a `ConstF32` -0.0 that other readers share (`zero_for_compares` is not `Some(true)`) |
| `glsl.rs:1909-1914` | anything else that folds or may fold to -0.0: a local (`const float z = -0.0;`), a product (`-1.0 * 0.0`), a vector lane (`vec2(-0.0, 1.0).x`), a -0.0 read by a compare and by a product |

Every N1 probe refused at `:1912`: `const float z = -0.0; a.x < z` (VS and FS),
`float z = -1.0 * 0.0`, `vec2 z = vec2(-0.0, 1.0); a.x >= z.x`, and
`float z = -0.0; (a.x < z) ? a * z : ..`. The literal form `a.x < -0.0`
compiles (the VS; the FS fails `ColourForm` first), and `a + vec4(-0.0)`
compiles.

`Fp16Inexact`: `nir.rs:1979` (a constant `F2f16` not exact), `vec16.rs:476`,
and the mediump tree's `f16_rtne` sites in `nir_fs.rs` (`:1941`, `:6748`,
`:7142`, `:7168`, `:7203`, `:7222`, `:7423`, `:7531`, `:7580`, `:7682`,
`:7786`, `:8354`, `:8642`, `:8800`, `:9021`, `:9778`, `:9798`).
`select.rs:78` `f16_rtne` returns `None` outside the normal fp16 exponents.
In a mat4 variant of H1 the relink refused at `nir_fs.rs:6748` (`vectorise`,
the constant of a moved varying lane). The dEQP mat3 values (section 6)
compiled and relinked in the probe, so the H1 site is **not confirmed**: the
measure item's committed pair decides it.

## 4. What Mesa 26.2.1 does

**Built-in calls on constants are folded by the GLSL front end, at fp32.**
`G/ast_function.cpp:657-664`: in GLSL ES 1.00 (`is_version(120, 100)`) a call
whose arguments are constant expressions is replaced by
`sig->constant_expression_value(...)`, an `ir_constant`, and no instruction
is emitted. The call's body is evaluated by
`ir_function_signature::constant_expression_value`
(`G/ir_constant_expression.cpp:1170`) over the built-in's GLSL IR, and each
`ir_expression` by `:689` with the C expressions of
`G/ir_expression_operation.py`:
`rsq` `1.0F / sqrtf` (`:453`), `sqrt` `sqrtf` (`:454`), `floor` `floorf`
(`:563`), `ceil` (`:562`), `fract` `x - floorf(x)` (`:564`), `sign` (`:451`),
`rcp` (`:452`), `mul`/`add` (`:670`, `:646`), `min`/`max` (`:722-723`),
`saturate` (`:601`), `atan2` (`:752`), **`fma` `a * b + c` unfused**
(`:755`), `lrp` `a * (1 - t) + b * t` (`:757`). A `const` initialiser is
folded the same way (`G/ast_to_hir.cpp:4841-4842`, `:4904`). Apart from this
the GLSL IR has no constant-folding pass: `do_common_optimization`
(`G/glsl_parser_extras.cpp:2577-2617`) runs `do_tree_grafting`,
`do_minmax_prune`, `do_rebalance_tree`, `do_algebraic` and jumps. An operator
on constants that are not `const` variables (`color * 0.5` after
`color.x = res`) is left to NIR.

**mediump constants are rounded once to fp16.** `G/lower_precision.cpp:739-746`
(and `:1000-1010` for initialisers) converts each constant of a lowered
expression with `_mesa_float_to_half` (`src/util/half_float.h:62`), which
rounds to nearest even. Its slow path makes a subnormal, a zero or an
infinity, as docs/notes/mali-mixed-precision.md section 2 records.

**NIR folds everything else** (`N/nir_opt_constant_folding.c:41-56`,
`nir_eval_const_opcode` with `info.float_controls_execution_mode`), in each
`nir_opt_constant_folding` of Bifrost's loop (`P/bifrost/bifrost_nir.c:242`,
`:340`, `:367`, `:985`, `:1283`):

- A float result is flushed only if the mode asks for it
  (`N/nir_constant_expressions.py:97-98`, `:563-568`). A 16-bit result is
  `_mesa_float_to_float16_rtne` of the op computed in `float` on the halves'
  values unless the mode says RTZ (`:120-123`, `:551-556`).
- The GL path sets no float controls: nothing under `G/`, `src/mesa/` or
  `src/gallium/drivers/panfrost/` writes `float_controls_execution_mode`, so
  it is 0. There is no flush, no RTZ and no signed-zero preserve.
  `P/pan_compiler.c:308-320` derives the hardware's `ftz_fp16`/`ftz_fp32` from
  the same 0 (Valhall forces fp32 flush only when fp16 flushes), and
  `P/bifrost/bifrost_compile.c:4369-4372` likewise. Constant folding therefore keeps
  denormals, infinities and NaNs as IEEE gives them.
- NIR's `ffma` folds **fused** (`fmaf`, `N/nir_opcodes.py:1150-1163`), unlike
  the GLSL IR's `fma`. `frsq` is `1.0f / sqrtf` (`:267`), `ffloor`/`ffract`
  are `floorf`-based (`:439-440`) and `flrp` is unfused (`:1228`).
- Signed zero: with no preserve flag the `nsz` rules fire.
  `fadd(a, 0.0) -> a` (`N/nir_opt_algebraic.py:236`), `fadd(a, -0.0) -> a`
  (`:237`), `fmul(a, ±0.0) -> 0.0` under `nsz,nnan` (`:277-278`) and
  `ffma(±0.0, a, b) -> b` (`:305-306`). The compare rules
  `flt/fge(a, -0.0) -> (a, 0.0)`, `fneu(a, -0.0) -> fneu(0.0, a)` and
  `feq(-0.0, a) -> feq(a, 0.0)` (`:771-776`) build a **new** `0.0` immediate
  for the compare. Another reader of the original `-0.0` keeps it. Copy
  propagation has already handed a `const float z = -0.0` or a folded
  `-1.0 * 0.0` to the compare as a `load_const`, so the rule sees it.

## 5. What plan #367 holds, and what this plan takes

#367's references `tools/mali/testdata/gm2_constfold_1..6` (516cc7b1) and
`gm2_arithconst_1..4` (6beeba85) were run through the same probe:

- `gm2_constfold_2..6` (FS) refuse at `nir_fs.rs:4523` (the blur and Laplace
  weights, `0.0 * TextureStepX`). `gm2_constfold_1` refuses `ArithConstant`.
- `gm2_arithconst_3/4` (VS) and glmark2's `light-basic.vert` (with its added
  `const vec4 LightSourcePosition`) refuse at `nir.rs:2004`.

So #367's lowering owns `nir_fs.rs:4523` and `nir.rs:2004`. This plan takes:

- `nir.rs:1997` and `nir.rs:2010-2051` (S1a, S1c, S5) and `:2062`;
- `nir_fs.rs:1065`, `:1087` and `:1097` (S2a, S2b, S2c, S3), which no glmark2
  reference reaches;
- `NegZeroShared` (N1). glmark2's terrain `1637` is the only #367 shader with
  it, and no #367 reference makes it;
- `Fp16Inexact` (H1).

S1b and S2d wait for #367. After its `nir.rs:2004` and `nir_fs.rs:4523`
lowering they are re-probed, and whatever remains joins this family. One rule
is to hand over to #367's lower item: **an `ffma` of constants that comes
from a built-in call on constants (`normalize(const)`, `dot(const, const)`)
is Mesa's front-end fold, and it is unfused** (`a * b` rounded, then `+ c`
rounded; `G/ir_expression_operation.py:755`). It is not NIR's `fmaf`.

## 6. The dEQP shaders (as rebuilt; the measure item's pairs supersede them)

ShaderLibrary FS-only case (`constant_expressions.builtin_functions.*_fragment`;
`glsShaderConstExprTests.cpp` and `glsShaderLibraryCase.cpp`). The vertex
shader is `attribute highp vec4 dEQP_Position; void main() { gl_Position =
dEQP_Position; }`:

```
#version 100
precision highp float;
precision highp int;
uniform float ref_out0;
float out0;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
void main()
{
    const float cval = floor(float (37.3));
    out0 = cval;
    bool RES = isOk(out0, ref_out0, 0.05);
    gl_FragColor = vec4(RES, RES, RES, 1.0);
}
```

The expressions are `floor(T(37.3))`, `ceil(T(82.2))`, `fract(T(17.75))`,
`mod(T(87.65), MT(3.7))`, `sign(T(-18.0))`,
`smoothstep(MT(3.0), MT(5.0), T(4.0))`, `step(MT(3.2), T(4.2))`,
`mix(T(10.0), T(20.0), MT(0.75))`, `sqrt(T(10.0))`, `inversesqrt(T(10.0))`,
`length`/`normalize`/`distance`/`dot` of `1.0` or `vecN(1.0)`,
`faceforward(T(1.0), T(1.0), T(1.0))`, `reflect(vecN(1.0), vecN(1.0, 0.0..)).x`,
`refract(.., 0.5).x`, `asin(T(0.0))`, `atan(T(-1.0), T(-1.0))`, `atan(T(2.0))`
and `equal(ivecN(1), ivecN(2))`. In the vertex variant `out0` is a `varying`
written by the vertex shader, and the program's fragment shader is the
mediump compare (it refuses `IntValue`, section 2).

Operator FS case (`es2fShaderOperatorTests.cpp`), `add_assign_effect`,
mediump vec3 (the in1 swizzle is not quoted from the source):

```
varying mediump vec4 v_in0;
varying mediump vec4 v_in1;
void main()
{
	mediump vec3 in0 = v_in0.zxy;
	mediump vec3 in1 = v_in1.<swizzle>;
	mediump vec3 res = vec3(0.0);
	res = in0;
	res += in1;
	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.xyz = res;
	color = color * <scale> + <bias>;
    gl_FragColor = color;
}
```

The scale and bias are the entry's (`de::floatToString(x, 2)`: `0.25`/`0.5`
for add, `0.04`/`0.5` for degrees, `25.0`/`0.5` for radians, `0.8`/`0.1` for
fract). The unused colour lanes (`0.0 * scale + bias`, `1.0 * scale + bias`)
are the constants that refuse. Matrix constant inputs
(`es2fShaderMatrixTests.cpp`) are `mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2,
-0.3, 1.0)` (a translation, printed to one decimal) and `mat3(1.2, 0.1, -0.1,
0.1, 0.9, 0.2, 0.2, -0.1, 0.7)`, reduced as `vec4(res[0]+res[1]+res[2],
1.0)`.

## 7. Lowering plan (one session: lower-constop)

All of it is in `src/shaderc/online/mali/**`, with a host test
`tests/mali_deqp_constop.rs` byte-compared to the `deqp_constop_*` references
(section 8). Order: after #367's `CONST_FOLD`/`ARITH_CONST` lowering. Re-probe
S1b and S2d first.

1. **Fold the highp ops on constants in `nir.rs` `alu_sized`**
   (`:1974-2066`, S1a, S1c, S5). Replace the refusing arms `:2010-2051` with
   Mesa's GLSL IR fold, mapping `Alu` to the shared
   `crate::shaderc::online::fold::fold_float` (`src/shaderc/online/fold.rs:24`,
   used by V3D and PowerVR; it is not changed). That function already has the
   exact `floor`, `ceil`, `trunc`, `fract`, correctly rounded `sqrt`,
   `rsq = 1 / sqrt` with two roundings, `sign`, compares, `b2f`, `i2f` and
   `f2i`. Add in `nir.rs`: `Fclamp` (min/max of the bounds), `Bcsel` on a
   constant condition, and the bool/int operands. `:1997` must test
   `plain` only for float operands, so that a bool `true` (`0xffffffff`) is
   read as a bool. Keep refusing what Mesa folds through the host's libm
   (`Fsin`, `Fcos`, `Flog2` other than exact cases) and any non-finite or
   denormal operand or result. No dEQP shape outside S4 needs those. They
   need a reference showing Mesa's bits first (`fmath.rs` is within 1 ULP,
   not glibc-exact).
   Caveat: S1 folds the **whole built-in** in GLSL IR order, and our IR lowers
   some built-ins differently. `mod` is `x - y * floor(x / y)` with a true
   division in GLSL IR (`ir_binop_div`), and `smoothstep` divides too.
   Ours may be `rcp`-then-`mul` or an `ffma`. Fold only where the IR's op
   sequence gives Mesa's bits, which each reference's FAU word shows. Leave
   `asin` and `atan` until their references match (their GLSL bodies are
   polynomials in `G/builtin_functions.cpp`).
2. **Fold two fp16 constants in the mediump lane walk** (`nir_fs.rs:1065`,
   `:1097`: S2a, S2c, S3). Do as `nir_eval_const_opcode` at 16 bits: the halves
   to `f32`, `*` or `+` in `f32`, then `f16_rtne`
   (`N/nir_constant_expressions.py:551-556`), and return the constant.
   The operands are the already rounded halves (`lower_precision`). An
   fp32 product of two halves is exact, so it is one rounding. Keep refusing a
   result that rounds to a subnormal, zero (from a non-zero) or infinity until
   a reference shows one. The block at `nir_fs.rs:4507-4527` (`f16_exact` on
   both operands and the result) is #367's. Make the new fold compute
   the same bits where both apply, so the two never disagree.
3. **`nir_fs.rs:1087`** (S2b, 8 cases). The rule at
   `N/nir_opt_algebraic.py:2406` folds `a * c` at fp16. Replace the
   `f16_exact` test by the step-2 fold (RTNE of the product of the two
   halves). Check against `deqp_constop_op_degrees_*`.
4. **N1, `glsl.rs` `plus_zero`** (`:1909-1914`, `:1899-1903`). Where the
   compare reads a value that folds to -0.0 (a local, a product, a vector
   lane) or a shared -0.0 constant, append a fresh `IrOp::ConstF32` with
   those lanes `+0.0` and point the compare's source at it. Leave the
   original for its other readers, as the rule's replacement does
   (`:771-776`). The order of the two constants in the FAU/immediates is
   what `deqp_constop_nz_*` pins. If the session runs short, this step is
   the one to move to a second session (117 cases, all but 4 in `random.*`:
   check the measure item's pairs first).
5. Not in this session: **H1** (`Fp16Inexact`, 3 cases). Mesa's
   `_mesa_float_to_half` never refuses: it gives a subnormal, a zero or an
   infinity. So `f16_rtne` at the moved-varying sites would get a full
   RTNE with subnormals, once `deqp_constop_mat3sub_vs` shows Mesa's words. Also
   not in it: `nir.rs:2062` (inf/denormal results) and the S4 shapes the
   measure item does not attribute to S1-S3.

Expected yield of steps 1-3, before the next refusal: S1 fragment 54 of the
83 (S1a 50 + S1c 4; S1b's 29 follow #367). The 82 vertex variants move to
`IntValue`. S2a-S3 give their 125 lowp and mediump cases; the 71 highp
ones may move to `MixedPrecision`. Step 4 adds up to 117. The board
rerun counts it.

## 8. Reference cases for ref-constop

All `tools/mali/testdata/deqp_constop_<name>.{vert,frag,expect.json}`, made
with pan_shc and the drm-shim as `gm2_*` are, each from the dEQP source above
or the measure item's pair:

| Name | Shape | What it pins |
|---|---|---|
| `ce_floor_fs` | S1a | the highp constant program (FAU word of `37.0`), the base for the rest |
| `ce_inversesqrt_vec4_fs` | S1a | `1 / sqrtf` with two roundings, vec4 |
| `ce_mod_vec2_float_fs` | S1a | GLSL IR's `x - y * floor(x / y)`, true division and unfused |
| `ce_smoothstep_fs` | S1a | `clamp` and division in the front-end fold |
| `ce_sign_vec3_fs`, `ce_fract_vec2_fs`, `ce_ceil_fs` | S1a | the simple ops |
| `ce_atan_separate_fs` | S1c | a bool constant operand, the inlined `atan` body's bits |
| `ce_asin_fs` | S1b | `asin`'s polynomial (after #367's `:2004`) |
| `ce_normalize_vec3_fs`, `ce_dot_vec3_fs` | S1b | the unfused front-end `dot` (the rule handed to #367) |
| `ce_floor_vs` | S1a VS | the vertex words of a constant varying (its FS refuses `IntValue` today) |
| `struct_param_out_vs` | S5 | a compare of constant struct members |
| `op_add_assign_effect_mediump_vec3_fs` | S2a | the fp16 colour lanes `0.0 * 0.25 + 0.5`, `1.0 * 0.25 + 0.5` |
| `op_minus_lowp_float_fs`, `op_abs_mediump_float_fs` | S2a | a unary and a function case |
| `op_abs_highp_float_fs` | S2a highp | whether Mesa's program is the mixed-precision one (mixprec handoff) |
| `op_degrees_mediump_float_fs` | S2b | the fp16 RTNE of `57.29578 * 0.04` |
| `matrix_preinc_mediump_mat3_fs` | S2c | `1.0 + 0.0` lanes |
| `loops_for_basic_mediump_float_fs`, `loops_while_basic_highp_float_fs` | S3 | the unrolled float counter |
| `nz_local_cmp_vs`, `nz_local_cmp_fs` | N1 | `const float z = -0.0; a.x < z` |
| `nz_prod_vs` | N1 | `float z = -1.0 * 0.0` |
| `nz_shared_vs` | N1 | `(a.x < z) ? a * z : ..`: both a `0.0` and a `-0.0` constant |
| `mat3sub_vs` | H1 | `matrix.sub.const_mediump_mat3_mat3_vertex` as a pair (the relink) |
| `cf_inf_fs`, `cf_denorm_fs` | `nir.rs:2062` | optional: Mesa's bits for an infinite and a denormal fold (float controls 0) |

### References made (item ref-constop, 2026-11-16)

All 28 of the table (the optional two included) are made. The integration tip
has no `tools/mali/testdata/deqp_shapes/` pair for any of them, so each
dEQP text was rebuilt from VK-GL-CTS `93bca01861b0` (`G2` below is
`modules/gles2/functional/`, `GS` `modules/glshared/`). The `nz_*` and `cf_*`
pairs are reductions: N1's dEQP cases are random or long
(`functions.test:2006-2047`), so these take section 3's probe forms in the
style of `negzero_local` and `negzero_vs_select`. Every reference is the pair as
committed, drawn once by Mesa 26.2.1's panfrost on the drm-shim and extracted:

```sh
tools/mali/drm_shim_ref.sh run $SHIM tools/mali/testdata/deqp_constop_<name>.vert \
    tools/mali/testdata/deqp_constop_<name>.frag <out>/deqp_constop_<name> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_constop_<name>.trace --draw \
    > tools/mali/testdata/deqp_constop_<name>.expect.json
```

`matrix_preinc_mediump_mat3_fs` adds `--mat-columns`, because dEQP enables every column
of `a_mat3`. `$SHIM` is the `drm_shim_ref.sh build` layout (`inst/`). It is
configured with that script's meson options in a second build directory of the
`build_pan_shc.sh` tree (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, extracted
from the tarball that `third_party/mesa-26.2.1/UPSTREAM` checks). This build
reproduces `gm2_constfold_1.expect.json` byte for byte.

| Reference | dEQP case (`dEQP-GLES2.functional.shaders.`) | Source (rebuilt) |
|---|---|---|
| `ce_floor_fs` | `constant_expressions.builtin_functions.common.floor_float_fragment` | `G2/es2fShaderConstExprTests.cpp:112`; template `GS/glsShaderConstExprTests.cpp:102-111,132-187`; `GS/glsShaderLibraryCase.cpp:141-191` (VS), `:359-427` (FS), `:1061-1072`; `framework/opengl/gluShaderLibrary.cpp:1981-1982` |
| `ce_inversesqrt_vec4_fs` | `...exponential.inversesqrt_vec4_fragment` | `es2fShaderConstExprTests.cpp:102`, the same template (`.w`, `:138`) |
| `ce_mod_vec2_float_fs` | `...common.mod_vec2_float_fragment` | `:115`, the `${MT}` scalar variant (`glsShaderConstExprTests.cpp:191-198`) |
| `ce_smoothstep_fs` | `...common.smoothstep_float_fragment` | `:121` |
| `ce_sign_vec3_fs` | `...common.sign_vec3_fragment` | `:111` |
| `ce_fract_vec2_fs` | `...common.fract_vec2_fragment` | `:114` |
| `ce_ceil_fs` | `...common.ceil_float_fragment` | `:113` |
| `ce_atan_separate_fs` | `...trigonometry.atan_separate_float_fragment` | `:86` |
| `ce_asin_fs` | `...trigonometry.asin_float_fragment` | `:84` |
| `ce_normalize_vec3_fs` | `...geometric.normalize_vec3_fragment` | `:150` |
| `ce_dot_vec3_fs` | `...geometric.dot_vec3_fragment` | `:145` |
| `ce_floor_vs` | `...common.floor_float_vertex` | `:112`; `glsShaderLibraryCase.cpp:285-356` (VS), `:233-282` (FS) |
| `struct_param_out_vs` | `struct.local.parameter_out_highp_vertex` | `G2/es2fShaderStructTests.cpp:163-209,607-644`; `GS/glsShaderRenderCase.hpp:71-79` |
| `op_add_assign_effect_mediump_vec3_fs` | `operator.binary_operator.add_assign_effect.mediump_vec3_fragment` | `G2/es2fShaderOperatorTests.cpp:501-505,590-742,1622,1965-2089` |
| `op_minus_lowp_float_fs` | `operator.unary_operator.minus.lowp_float_fragment` | idem, `:1525` |
| `op_abs_mediump_float_fs`, `op_abs_highp_float_fs` | `operator.common_functions.abs.{mediump,highp}_float_fragment` | idem, `:1778` |
| `op_degrees_mediump_float_fs` | `operator.angle_and_trigonometry.degrees.mediump_float_fragment` | idem, `:1733` |
| `matrix_preinc_mediump_mat3_fs` | `matrix.pre_increment.dynamic_mediump_mat3_fragment` | `G2/es2fShaderMatrixTests.cpp:888-1118,1165-1198,1271,1296-1376` |
| `mat3sub_vs` | `matrix.sub.const_mediump_mat3_mat3_vertex` | idem, `:80-85,831-850,1265,1356-1362` |
| `loops_for_basic_mediump_float_fs` | `loops.for_constant_iterations.basic_mediump_float_fragment` | `G2/es2fShaderLoopTests.cpp:284-472,1301,1323` |
| `loops_while_basic_highp_float_fs` | `loops.while_constant_iterations.basic_highp_float_fragment` | idem |
| `nz_local_cmp_vs`, `nz_local_cmp_fs`, `nz_prod_vs`, `nz_shared_vs` | N1 (section 3) | reductions of section 3's probes; `nz_shared_vs` writes `z` to `gl_Position.x` so that a `-0.0` reader stays besides the compare |
| `cf_inf_fs`, `cf_denorm_fs` | `nir.rs:2062` | reductions: `3.0e38 * 10.0`, `1.0e-30 * 1.0e-15` through a local |

Corrections to section 6 found while rebuilding:

- The operator, matrix, loop and struct shaders have no `#version` and no
  `precision` statement; every variable carries its own qualifier.
- The bias is printed `0.50` (`floatToString(x, 2)`), not `0.5`. The in1
  swizzle of a vec3 is `yzx` (`:502`).
- The second constant mat3 is `mat3(1.2, 0.1, 0.2, 0.1, 0.9, -0.1, -0.1, 0.2, 0.7)`.
  `s_constInMat31` is read row-major (`tcuMatrix.hpp:336-341`) and written
  column-major (`:831-850`), so the text is the transpose of the array.
- A float loop counter of the constant count declares an unused
  `uniform <prec> float uf_one;` (`es2fShaderLoopTests.cpp:329-330`). The increment is
  `ndx += 0.333333`.

What the references show (Mesa's words; nothing of ours was run):

- **S1** folds at fp32 into one FAU word, as section 4 says. Examples: `floor` `0x42140000`
  (37.0), `ceil` `0x42a60000`, `fract` `0x3f400000`, `dot` `0x40400000`,
  `atan` `0xc016cbe4`, and `inversesqrt` `0x3ea1e89b` (`1 / sqrtf`, two roundings).
  `mod` gives `0x40233340` (2.5500031). That is `x - round(y * floor(x / y))`, the true division
  and unfused; a fused one would give `0x40233335`. `normalize` gives
  `0x3f13cd3a`. `sign`, `smoothstep` and `asin` become inline constants
  (`-1.0`, fp16 `0.5`, `0.0`). `ce_floor_vs` has no varying program: the link
  moves the constant out0 into the FS as fp16 `0x50a0`.
- **S2a**: Mesa folds a constant colour lane only where its v2f16 pair is all
  constant (`op_degrees`: lanes z, w are the FAU word `0x38523800`, 0.5 and
  RTNE(1.0 * 0.04 + 0.5) = `0x3852`; ATEST reads `0x3f0a4000`). A constant
  lane paired with a computed one stays an ALU lane. `add_assign_effect`'s
  w is `MKVEC(x, 1.0)` then `FMA.v2f16 * 0.25 + 0.5`, and `minus`'s y is the
  zero lane of the same FMA. `matrix_preinc`'s w (`1.0 + 0.0`) is a run-time
  `FADD.v2f16` as well. The lower item's step 2 has to follow this pairing,
  not fold every lane.
- **S2b is not folded by Mesa.** `op_degrees` keeps `FMA.v2f16 x * 57.28125`
  (`0x5329`) and then `FMA.v2f16 * 0.04 + 0.5` (`0x291f`). There is no `a * c` constant,
  so step 3 (section 7) should emit both multiplies rather than fold.
- **S3**: both loops unroll and vanish. `res` is `coords.wxyz`, with no counter
  constant left.
- **S5** (`struct_param_out_vs`) folds to a constant colour that the link
  moves into the FS (`0x3c003c00`). The VS is only the position.
- **N1**: in `nz_local_cmp_vs/fs` and `nz_prod_vs` the compare reads the inline `0x0`
  (`FCMP_OR.f32.lt ..., 0x0`), and no `-0.0` remains. `nz_shared_vs` has both:
  the compare's inline `0x0` and the FAU word `0x80000000` for the output
  lane, as `:771-776` predicts.
- **cf**: float controls 0 keep both. `cf_inf_fs` pushes `0x7f800000` and
  `cf_denorm_fs` pushes `0x00000001` (the denormal 1e-45, not flushed).

Open points of section 9 that these settle:

- **H1** (Mesa's side): `mat3sub_vs` folds the whole mediump colour at fp16
  and the link moves it into the FS as the FAU words `0xb8009000`,
  `0x3c003265` (x = -2^-11 `0x9000`, y = -0.5, z = `0x3265`, w = 1.0). The
  exact result 0 is not what fp16 gives here. So the relink's constant is the moved varying
  lane's (the `nir_fs.rs:6748` kind of site), computed at fp16. Which of our
  `f16_rtne` sites refuses is still to be checked on our side (lower-constop).
- **op_abs_highp mixed precision**: yes. The highp FS loads fp32, takes `abs` at
  fp32 inside the conversion (`FADD.f32 r0.h0, r0.abs`), and does the colour
  at fp16 (`FMA.v2f16`). The mediump FS converts first and takes `abs` at fp16.
  The highp program is the mixed one (mixprec handoff).

## 9. Left open

- S4 (195 `ConstantFold`, 113 `NegZeroShared` in `random.*`): attribute to
  S1-S3/N1 from the measure item's `deqp_shapes` pairs before the board
  session. The rest (transcendentals on constants, inf/denormal folds) gets a
  later item.
- The H1 site is unconfirmed on our side (section 3). Mesa's side is in
  section 8, "References made".
- The VS S1 cases need `IntValue` in the compare fragment shader (intval).
