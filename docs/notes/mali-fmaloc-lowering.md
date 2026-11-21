<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): lowering the TexCoord and FusedMultiplyAdd refusals to Mesa's words (design)

**Status: design, no code.** Plan #387 item `fmaloc-design`; the implementation items are the
steps of section 4, to be filed by the planner. It reads the 15 Mesa 26.2.1 references
`tools/mali/testdata/deqp_fmaloc_{fma,tc}_*` that [`mali-deqp-fmaloc.md`](mali-deqp-fmaloc.md)
(item `fmaloc-ref`) pins as refused in `tests/mali_deqp_fmaloc.rs`, and the compiler
(`src/shaderc/online/mali/`, read only) at `5cb80624`. It changes neither. The seven
`deqp_fmaloc_tk_*` references are `MaliError::TexKind` (a bias or a projection, not
`TexCoord`) and are outside this note (section 6).

Counts. The plan's table (`mali-deqp-families.md` 18.2 rows 12 and 15, the board run of
`de2f2c103`) gives `TexCoord` 182 fs cases (texture.specification 86, uniform_api.value 52,
shaders.random.texture 25, uniform_api.info_query 11, shaders.discard 4) and
`FusedMultiplyAdd` 139 vs cases (shaders.operator 42, shaders.matrix 41,
shaders.constant_expressions 32, shaders.random 15; the host measurement found 37 of the
board's FMA cases compile at the tip through `mix`/`mod`/`smoothstep`/vector `reflect`). The host measurement of
`fmaloc-ref` (HOST-ONLY, `cc89a1cf`) classed 63 FMA and 104 TEX_COORD cases into the shapes
below; those are the "pinned" counts. The remaining cases of each text (TexCoord: about 78;
FMA: 139 minus 63 pinned minus the 37 that compile) are not in any shape: the first step
of each family starts with a host recount (`tools/mali/deqp-host-measure.sh --filter`), see
section 5.

Line numbers are those of `5cb80624`. Plan 377's `lower-constop` and the other items edit
`nir.rs` and `nir_fs.rs` too, so an implementer finds each site by the **arm** named here and
not by the number.

## 1. Mesa's words per shape

All sequences are the `fs.fragment` or `vs.varying` text of the shape's reference
(`<shape>.expect.json`), with loads, stores and the `ATEST`/`BLEND` tail left out.

### 1.1 TexCoord (fragment, mediump)

| Shape (`deqp_fmaloc_`) | Source | Mesa's sequence |
|---|---|---|
| `tc_cube_face` (86) | `textureCube(s, vec3(-1, -y, +x))`, `x = v.x*2.0-1.0`, `y = v.y*2.0-1.0`, `v` a mediump vec2 varying | the fp16 pair `FMA.v2f16 r0, r0^.h10, 2.0, -1.0` (x and y at once); `FADD.v2f16` for `-y`; `F16_TO_F32` per lane (the constant `-1` is an operand); then `build_cube_desc`: `CUBEFACE1`, `CUBEFACE2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP.f32`, `FMA.f32 k = 0.5 * rcp`, two `FMA.f32.clamp_0_1 .., 0.5`, `MUX.i32.bit` (face into s); the four staging `MOV`s and `TEX_SINGLE ... .cube.computed`. 22 instructions |
| `tc_cube_const` (10) | `textureCube(u, vec3(0.0))` in dEQP's `compare_vec4` harness | **No fold**: `CUBEFACE1 r0, 0x0, 0x0, 0x0`, `CUBEFACE2_V9 r1, 0x0, 0x0, 0x0`, `CUBE_SSEL r2, 0x0, 0x0, r1`, `CUBE_TSEL r3, 0x0, 0x0, r1`, `FRCP.f32 r0, r0^`, `FMA.f32 r0, 0x38000000.h1, r0^, 0x0.neg`, two `FMA.f32.clamp_0_1 .., 0x38000000.h1`, `MUX.i32.bit r1, r2^, r1^, u2.w0`, the `MOV`s, `TEX_SINGLE ... .cube.computed`; then the harness in fp16 (`FADD.v2f16`, `FADD_IMM.v2f16`, 3 `FMAX.v2f16 .abs`, `FCMP_OR.v2f16.lt.f1`, `IADD.v2u16`, `MKVEC.v2i16`). 28 instructions |
| `tc_cube_rnd_sub` (7) | `textureCube(a, c - b)`, `c`, `b` mediump vec3 varyings | the two varyings' loads (`LD_VAR_BUF_IMM.v4` and `.v2`) narrowed (`V2F32_TO_V2F16` x3, one `MKVEC.v2i16 r3, r1.h1, r2.h0` pairing `c.w` of the `.v4` load's second pair with `b.x`), `FADD.v2f16 r0, r0^, r3^.neg` (x,y) and `FADD.v2f16 r1, r1^.h00, r2^.neg.h11` (z), three `F16_TO_F32`, then the same cube sequence with the staging words at `r2/r0/r1`. 29 instructions |
| `tc_2d_scalar` (1) | `texture2D(a, vec2(c, b))`, `c` a `const bool` true, `b` a float varying | `MOV.i32 r2, 0x3F800000` (the constant 1.0, as a raw staging word), `MOV.i32 r3, r0^` (the scalar varying), `MOV.i32 r0, u2.w0`, `MOV.i32 r1, 0x0`, `TEX_SINGLE ... .2d.computed`. 10 instructions |

### 1.2 FusedMultiplyAdd (vertex, highp unless stated)

| Shape (`deqp_fmaloc_`) | Source | Mesa's sequence |
|---|---|---|
| `fma_constmat_mat` (9) | `const matN * const matN` | folded by the GLSL front end: no varying program, the colour is a FAU constant in the fragment shader (`MOV.i32 r0, u2.w0`, `r1, u2.w1`; 7 instructions) |
| `fma_const_asin` (9), `fma_const_reflect` (1), `fma_const_matmat` (1) | a `const` built-in or operator on constants (`asin(vec2(0.0)).y`, `reflect(1.0, 2.0)`, `mat * mat`) | the same: constant folded to a vertex output the link moves into the fragment shader; the fragment words are the `isOk` harness in fp16 (10 to 11 instructions) |
| `fma_constmat_vec` (9) | `const mat3 * attribute vec3` | per lane a `FMA.f32` chain with FAU constants for the non-zero entries (`FMA.f32 r0, u5.w1, r2, r0^`); the zero entries are folded out; 13 instructions |
| `fma_attrmat_vec` (9) | `attribute mat3 * attribute vec3` | three `LD_ATTR_IMM.v3` (the matrix's columns), then 8 `FMA.f32` and 1 `FADD.f32`; 26 instructions |
| `fma_attrmat_unimat` (6) | `attribute mat4 * uniform mat4` (and `*=`) | 4 `LD_ATTR_IMM.v4`, 48 `FMA.f32` and 9 `FADD.f32` with the uniform matrix as FAU words; 73 instructions |
| `fma_asin` (8) | `asin(x)`, `acos(x)` of an attribute lane | three `FMA.f32 .., x.abs, c, ..` Horner terms, `FADD 1-|x|`, `FRSQ.f32` + `FRCP.f32` (the sqrt), two `FCMP_OR.f32.lt.f1` and `FADD` (the sign), a small-x branch by `CSEL.u32.eq`; 35 instructions |
| `fma_atan` (5) | `atan(x)` | `FRCP.f32`, `FCMP_OR.f32.ge.m1 1.0 >= |x|`, `CSEL`, six `FMA.f32`, `CSEL` of the -pi/2 offset, then `LSHIFT_AND.i32` + `LSHIFT_OR.i32` (copysign); the mediump `*0.5+0.5` envelope is one `FMA.v2f16`; 37 instructions |
| `fma_atan2` (4) | `atan(y, x)` | 11 `FMA.f32`, 2 `FRCP`, 8 `CSEL`, 5 `FCMP_OR`; 57 instructions |
| `fma_reflect_float` (2) | `reflect(float I, float N)` | `FADD.f32 r1, r0, r0` (2N), `FMA.f32 r0, r4, r0^, 0x0.neg` (N*I), `FMA.f32 r0, r1^.neg, r0^, r4^` |

Why Mesa has FMAs at all: `float_mul_add32` is `has_ffma | fuse`, so `nir_opt_algebraic`'s
late pass rewrites `fadd(fmul(a, b), c)` to `ffma` when the product has only `fadd` users
([`mali-vs-fma.md`](mali-vs-fma.md), [`mali-algebraic-folds.md`](mali-algebraic-folds.md)).

## 2. The refusing sites, per shape

### 2.1 FusedMultiplyAdd

There is one raising arm: the vertex walk's catch-all `AluOp::Ffma => return
Err(MaliError::FusedMultiplyAdd)` (`nir.rs:4781`, in the walk after the arms `is_mat_term`
4741, `is_fmod` 4747, `is_cross` 4754, `is_mix`/`is_smoothstep_inner` 4761, `is_reflect`
4770). The helper `vs_fmod` (`nir.rs:2649`) raises the same text for a malformed `mod` and
is not a dEQP shape. A front-end `ffma` reaches 4781 when none of the shape predicates
match:

| Shape | Why no earlier arm matches |
|---|---|
| `fma_const_*`, `fma_constmat_mat` | all three sources are constants. The arms test the IR shape, not the values, and run before `vs.alu`, whose constant fold (`nir.rs:2181`, `Alu::Ffma => f(0)*f(1)+f(2)`: **unfused**, two roundings, as Mesa's GLSL folder does) would take it. Nothing in the walk folds an `ffma` whose lanes `x(0..2)` are all `Val::Const` before the catch-all |
| `fma_constmat_vec` | `is_mat_term` (`nir.rs:2759`) requires `srcs[0]` to be `IrOp::LoadUniform` of a matrix column; a `const mat3` column is `IrOp::ConstF32` |
| `fma_attrmat_vec`, `fma_attrmat_unimat` | `srcs[0]` is `IrOp::LoadInput` (an attribute matrix's column), and in `attrmat_unimat` the splat factor is a uniform lane: `is_mat_term` wants the *uniform* as the column |
| `fma_asin`, `fma_atan`, `fma_atan2` | no predicate: the front end's polynomial `ffma`s (`builtin_call.rs` `asin` 1384, `atan`, `atan2`) over attribute lanes. They also use `bcsel`, `fsign`, `frsq` which are separate texts (`Arithmetic`, boolean values) once the `ffma` is accepted |
| `fma_reflect_float` | `is_reflect` (`nir.rs:2595`) requires `srcs[0]` to be `fmul(fdot, -2.0)`; for scalars the front end writes `fmul(fmul(n, i), -2.0)` with no `fdot` |

### 2.2 TexCoord

`MaliError::TexCoord` is raised from about 20 sites in `nir_fs.rs`; the dEQP shapes reach
four:

| Shape | Site | Why |
|---|---|---|
| `tc_cube_face` | the mediump implicit cube arm (`nir_fs.rs:6596`, `return Err(MaliError::TexCoord)` at 6608), predicate `cube_dir_measured` (`nir_fs.rs:6865`) | the direction `(-1, -y, x)` is a constant first lane and fp16 trees for lanes 1 and 2. The predicate's `pair` form needs lanes 0 and 1 to be trees (`f.alu_of(v).is_some()`) and lane 2 a constant or a uniform; its `raw` form needs raw varying lanes. A constant x, and a *tree* z, are neither |
| `tc_cube_const` | the same arm and predicate | `raw` wants `dir.iter().any(varying)` and `pair` wants trees: three constants are neither. (The explicit-level arm at 6512-6516 refuses an all-constant direction on purpose; the implicit arm has no all-constant case.) |
| `tc_cube_rnd_sub` | the same arm and predicate | lanes 0 and 1 are sums of two varying lanes (a tree: accepted), but lane 2 is a sum too (`c.z - b.z`), and `pair` allows only a constant or uniform lane 2 |
| `tc_2d_scalar` | the generic 2D arm (`nir_fs.rs:6808`, `_ => return Err(MaliError::TexCoord)`) | `s` is `Val::Const` and `t` a varying lane: not `(Varying, Varying)`, not `(Const, Const)`; the computed arm needs both `!= Val::Undef`. **Inferred, not measured:** `c` is a `bool` constant that reaches the walk as `Val::Undef` (a boolean value), so this is an `intval` shape that surfaces as `TexCoord`; step 7 confirms it |

The look-up's other sites that matter here: the pair builder `coord_lane` (`nir_fs.rs:9676`;
`_ => TexCoord`) and the lone-computed check (`nir_fs.rs:9913-9927`) refuse any computed
coordinate that is not one fp16 lane, and `index` (`nir_fs.rs:8118-8125`) any varying
coordinate that is not lanes (0,1) or (2,3) of one load. These are the sites a lowering
meets after the first.

## 3. The mechanism per shape

### 3.1 Cube directions of a mediump fragment shader (`tc_cube_face`, `tc_cube_const`, `tc_cube_rnd_sub`)

The back end already has Mesa's cube sequence (`cube_rows`, `nir_fs.rs:9753`, and
`Nir::cube_desc` for the vertex/highp side) and the look-up form
`TexLookup { cube: Some(Cube { lod: CubeLod::Computed }) }` with `bits: 16`
(`nir_fs.rs:6613-6616`). What is missing is the **direction shapes**: today `CubeDir`
(`nir_fs.rs:9696`) is `Pair(pair, z)` (x and y one fp16 pair, z a lone lane that is a
constant or uniform: case41) or `Raw([RawLane; 3])` (raw varying lanes, a link constant:
varyconst_101_cubebias). Three more are needed:

1. **A constant lane 0 or 1 beside trees** (`tc_cube_face`: x = const -1). In Mesa's words
   the pair is `FMA.v2f16 r0, r0^.h10, 2.0, -1.0` over `(y, x)`, so the constant lane is
   *not* in the pair: lane 0 (-1) is an `F16_TO_F32`-widened constant, a CUBEFACE1 *operand*
   (as case114's `CUBEFACE1.discard r2, 0x3F800000`), and lanes 1 and 2 are the two halves of
   one `FMA.v2f16` (`v.xy * 2.0 - 1.0` fuses: a product with 2.0 under an add is the `FMA`,
   measured equal, the comment of `cube_dir_measured`), with `-y` an `FADD.v2f16 ..neg`.
   Mechanism: generalise `pair` to "lanes 1 and 2 are the two halves of one pair, lane 0 is
   a constant", with the order `(y, x)` swapped by the half swizzle `.h10`. The six faces of
   `texture.specification` differ only in the constant's sign and which of x, y, -x, -y lands
   in which lane; one implementation covers the six sign forms (the compare is on
   `Val::Const`, the swizzles are `.h00/.h11/.h10`, `Pairs` already has them). The cube
   sequence itself is unchanged.
2. **An all-constant direction** (`tc_cube_const`). No fold: the cube rows take constant
   operands (`CUBEFACE1 r0, 0x0, 0x0, 0x0`: the lanes are the *fp32 constant words*,
   `F16_TO_F32` of an exact fp16 constant folded to the word). Mechanism: allow
   `dir.iter().all(Const)` in the implicit arm (`nir_fs.rs:6596`), keep `Cube::lod =
   Computed`, and make sure the constant passes (`opt.rs` / `fau.rs`) do not fold `FRCP` and
   `FMA` of constants: Mesa does not, so a fold here would be "compiled, not Mesa's". The
   compare harness that follows (`compare_vec4`: `FADD.v2f16`, `FADD_IMM.v2f16 #0xB5C3B3AE`,
   `FMAX.v2f16 .abs`, `FCMP_OR.v2f16.lt.f1`) is **separate and not pinned** by a
   `deqp_fmaloc_*` reference apart from this one; check by the byte compare that it is
   already expressible (it is the same `isOk` shape as `fma_const_asin`'s fragment, 10
   instructions) and move to the colour-form text if it is not.
3. **Sums of two varying vec3 in all three lanes** (`tc_cube_rnd_sub`). Mesa narrows both
   varyings, pairs `(c.x-b.x, c.y-b.y)` as one `FADD.v2f16` and `c.z-b.z` as a second one
   with `.h00/.h11` swizzles, then widens each lane: a `tree` over (varying, varying) for
   lane 2 too, in a `Pair(pair, CubeLane::Node)` (`CubeLane::Node(x, swz)` exists:
   `nir_fs.rs:9698`). Mechanism: drop "lane 2 is a constant or uniform" from `pair` in favour
   of "lane 2 is a one-lane tree (`coord_lane`)", and have `coord_lane` accept a sum of two
   varying lanes (today it takes a product of a uniform lane and a constant, case121: `Fmul`
   only, `nir_fs.rs:9676`). Not a new Mesa shape: the same `FADD.v2f16 r0, r0^, r3^.neg`
   words the pair path emits for `x, y`.

Un-ignored: `fmaloc_tc_cube_face` (mechanism 1), `fmaloc_tc_cube_const` (2),
`fmaloc_tc_cube_rnd_sub` (3).

### 3.2 `texture2D(a, vec2(c, b))` with a boolean/constant first lane (`tc_2d_scalar`)

The reference shows the first lane as `MOV.i32 r2, 0x3F800000` and the second the varying
`MOV.i32 r3, r0^`: a `(Const, scalar varying)` computed coordinate at fp32. The generic 2D arm
(`nir_fs.rs:6796-6808`) accepts `Computed` for `s != Undef && t != Undef`; the refusal is
reached because the bool constant `c` is not a `Val::Const` lane of the fragment walk (it is
a boolean value: `bool(...)` of constants through the global initialisers of the dEQP
random program, which the integer/boolean item `intval` owns), so this shape is a
**`intval` shape that surfaces as a TexCoord** and has no coordinate-side mechanism of its
own. Mechanism: none here; once the boolean constant `c` is `Const(F32_ONE)` at the walk, the
existing computed-coordinate arm emits `MOV.i32 r2, 0x3F800000` and `MOV.i32 r3, r0^`.
Verify with the reference after `intval`; it is a one-reference check, in the step that
removes the last `intval` text of `shaders.random.texture.fragment.12`.

### 3.3 Constant `ffma` (`fma_const_*`, `fma_constmat_mat`: 20 cases)

Mesa folds the expression before NIR. The mechanism is to fold at the walk: in the vertex
walk, **before** the `AluOp::Ffma` arms (`nir.rs:4703`...), take a front-end `ffma` whose
lanes `x(0)`, `x(1)`, `x(2)` are all `Val::Const` through `vs.alu`, which folds it
(`nir.rs:2181`, unfused `a*b+c`: the same arithmetic as GLSL's folder, pinned by
`deqp_constop_ce_mod`). Then the constant output is a link constant, and the existing
constant-varying move puts it in the fragment shader (as `gm2_arithconst_*`). Two traps:

- The GLSL front end computes `asin`, `acos`, `reflect` of constants by libm/closed form; the
  *compiler's own* front end (`builtin_call.rs`) expands them to the polynomial and folds that
  with f32 arithmetic. The results can differ in the last bit; the colour is rounded to fp16
  by the fragment harness, so most cases agree, but a byte compare of the FAU word is the
  check. Where they differ, the fold must take the built-in whole (`asin` of a constant by a
  correctly rounded host formula) in `builtin_call.rs` and not through the polynomial. That is
  `negzero-lower`/`lower-constop`'s territory (section 5).
- A folded `fsign`, `frsq` etc. in the same polynomial is refused as `ConstantFold`
  (`nir.rs:2163-2165`, a non-plain value) or by the constant text of plan 377; the 20 cases
  move to that text, not to a pass, unless the fold is complete.

Un-ignored: `fmaloc_fma_const_asin`, `_const_reflect`, `_const_matmat`, `_constmat_mat`.

### 3.4 Matrix products over a constant or attribute matrix (`fma_constmat_vec`, `fma_attrmat_vec`, `fma_attrmat_unimat`: 24 cases)

The vertex walk already emits Mesa's `FMA.f32` chain for a *uniform* matrix column
(`is_mat_term`, the `Prod::MatProduct` arms at `nir.rs:4703-4745`: the column's words are FAU
words). The three shapes change which operand is the source of the column words:

1. **A `const matN`** (`fma_constmat_vec`): the column is `IrOp::ConstF32`. Mesa folds the
   zero entries (`fmul` by 0, `fadd` of 0) and keeps the others as FAU constants. Mechanism:
   extend `is_mat_term`'s column test to `ConstF32` and let `vs.alu` fold the zero lanes (the
   existing `Alu::Ffma if uniform && is_zero(a) || is_zero(b)` rule at `nir.rs:2307`).
2. **An attribute matrix** (`fma_attrmat_vec`): the column is `IrOp::LoadInput`; the three
   `LD_ATTR_IMM.v3` are what the vertex stage already emits for a `vec3` attribute, so the
   lanes are `Val`s of the input loads and need no FAU word. Mechanism: let `is_mat_term`
   accept `LoadInput` of a `matN` (the column index from the input slot's location offset),
   emitting the same `Alu::Ffma` over the load's lanes.
3. **`attribute mat4 * uniform mat4`** (`fma_attrmat_unimat`, `*=`): the splat factor
   (`srcs[1]`) is a uniform lane and the column an input. The existing arm takes the splat as
   an input lane and the column as the uniform word; the roles swap here. Mechanism: let the
   arm take either operand as the uniform one (a `mat_term_sides` helper), keeping the
   summation order: Mesa's chain is `FMA(col0, s0, 0)`... the reference's 48 `FMA.f32` and 9
   `FADD.f32` pin the order (the `FADD`s are the last-term additions of the 4x4 product: a
   term by `1.0` of the identity is the `ArithConstant` arm at `nir.rs:4728`).

All three are *uniform-lowp/mediump* precision variants too (the 9 of each shape are lowp,
mediump, highp x3 sizes): the references pin highp; lowp and mediump differ only in the
narrowing envelope of section 1, so draw one reference of each in the step (see 5).

Un-ignored: `fmaloc_fma_constmat_vec`, `_attrmat_vec`, `_attrmat_unimat`.

### 3.5 Trigonometric polynomials (`fma_asin`, `fma_atan`, `fma_atan2`: 17 cases)

The `ffma` of an `asin` term has no shape predicate; a plain `Alu::Ffma` of non-constant
lanes (`vs.alu(Alu::Ffma, x(0), x(1), x(2))`) is already what the `mix`/`smoothstep` arms
emit. The words also need `FRSQ`+`FRCP` (the sqrt), `FCMP_OR.f32.lt.f1` (sign), `CSEL`
(select) and for `atan` the `LSHIFT_AND`/`LSHIFT_OR` copysign. **This is the vertex form of
arith-design's step 7 (composites `acos asin atan atan2`, fragment, 32 cases):** the same
Mesa polynomial and selects at fp32, in the other stage's walk
([`mali-arith-lowering.md`](mali-arith-lowering.md) row `acos`, `asin` and `atan`, `atan2`, and
its step 7). Mechanism: accept the polynomial's `ffma` as `FMA.f32` (one arm, `AluOp::Ffma`
of non-constants, **after** the shape arms) and take `fsign`, `bcsel`, `flt` through the
vertex-side select the `arith-lower-1` step builds. The `ffma` arm alone moves these shapes
from `FusedMultiplyAdd` to the compare/select text (`Arithmetic`/`IntegerValue`), no pass
until the selects are lowered.

Un-ignored when the selects exist: `fmaloc_fma_asin`, `_atan`, `_atan2`.

### 3.6 `reflect(float, float)` (`fma_reflect_float`: 2 cases)

`is_reflect` wants `fmul(fdot, -2.0)`; for scalars the front end writes `fmul(fmul(n, i),
-2.0)` (a `dot` of floats is a product, `a * b`). Mesa's words: `FADD.f32 r1, r0, r0` (the
`fmul(a, 2.0)` late rule), `FMA.f32 r0, r4, r0^, 0x0.neg` (N*I), `FMA.f32 r0, r1^.neg, r0^,
r4^`. Mechanism: let `is_reflect` accept `Fmul` of two scalars where it wants the `fdot`
(`nir.rs:2601`), and in the arm at 4770 take `d` as that product (`vs.alu(Alu::Fmul, ..)`)
instead of reading `m.def(s[0].val)`'s `fdot`. Un-ignored: `fmaloc_fma_reflect_float`.

## 4. Ranked one-session steps

Ranked by the cases of the shapes the step finishes (pinned counts; the bound of each step is
the cases it *moves*, as in the arith note: a random case finishes only when every op in it is
lowered). Each step names its references, its host forecast and its checks; all steps run
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
--test mali_deqp_fmaloc` with the un-ignored tests, and re-pin
`tests/mali_deqp_fmaloc.rs`'s `REFUSALS` entries.

| Rank | Step | Shapes (cases) | Host forecast | dEQP groups unblocked |
|---|---|---|---:|---|
| **1** | `tex-cube-face`: constant lane + fp16 pair tree for a mediump cube direction (section 3.1, mechanism 1) | `tc_cube_face` (86) | 86 | `texture.specification.{basic_teximage2d,basic_texsubimage2d,texsubimage2d_empty_tex}.*_cube`, `{teximage2d,texsubimage2d}_align.cube_*`, `random_teximage2d.cube_*` |
| **2** | `fma-matrix`: `const`/attribute matrix columns in the vertex matrix-product arms (3.4, 1-3) | `fma_constmat_vec`, `fma_attrmat_vec`, `fma_attrmat_unimat` (24) | 24 | `shaders.matrix.mul.*`, `mul_assign.*` (and `shaders.operator` with matrix products) |
| **3** | `fma-const`: fold a constant `ffma` before the catch-all (3.3) | `fma_const_asin`, `_reflect`, `_matmat`, `fma_constmat_mat` (20) | 20 | `shaders.constant_expressions.builtin_functions.trigonometry.*`, `shaders.matrix.mul.const_*` |
| **4** | `tex-cube-misc`: all-constant direction and sums of varyings in all lanes (3.1, mechanisms 2 and 3) | `tc_cube_const`, `tc_cube_rnd_sub` (17 pinned; up to 52+11 more: see below) | 17 | `uniform_api.{value,info_query}.basic.samplerCube_fragment`, `shaders.random.texture.fragment.*` |
| **5** | `fma-trig`: the polynomial `ffma` plus the vertex selects (3.5) | `fma_asin`, `fma_atan`, `fma_atan2` (17) | 17, after the selects | `shaders.operator.angle_and_trigonometry.{asin,acos,atan,atan2}.*`, `shaders.random.trigonometric.vertex` |
| **6** | `fma-reflect-float`: scalar `reflect` (3.6) | `fma_reflect_float` (2) | 2 | `shaders.operator.geometric.reflect.*_float_vertex` |
| (7) | `tex-2d-scalar` (3.2): no coordinate work, a check of `tc_2d_scalar` after the `intval` boolean constants | `tc_2d_scalar` (1) | 1 | `shaders.random.texture.fragment.12` |

Notes on the ranks:

- **Step 4 is under-counted.** The fmaloc-ref host measurement found 10 `tc_cube_const`
  cases in the 396-case slice (those whose first board refusal at `37305608` was TEX_COORD),
  but the 20261117 board run (the plan's table) counts 52 `uniform_api.value` and 11
  `uniform_api.info_query` cases whose first refusal is TexCoord, most of them presumably
  `samplerCube` with a constant direction. The planner should rank step 4 by the recount:
  if those 63 are the `vec3(0.0)` shape it is the second step and the first of the
  fragment family to start, because it needs no new Mesa shape beyond what step 1 builds.
  Its first action is `deqp-host-measure.sh --filter uniform_api` at the tip.
- **Step 3 (constant fold) ranks below 2 by count but starts later than the table says:** it
  touches `nir.rs`'s constant fold, the same function as `lower-constop` (plan 377, in review)
  and `negzero-lower`. Do it after `negzero-lower` has landed; the ranking cannot move it
  above them.
- **Steps 1 and 4 share one `CubeDir` rewrite** (3.1). They are separate sessions only for the
  size of the references: the one that goes first adds the constant-lane case, the other adds
  the sum lanes; neither depends on the other's code if each adds a variant to `CubeDir`
  rather than reshaping `Pair`.
- **Highest-risk is step 2:** the three shapes cover nine precision variants each (lowp,
  mediump, highp x three sizes). The step draws four more references (`mat2 * vec2` highp,
  `mat3 * mat3` mediump, the lowp attribute case, a `*=` case) with `tools/mali/deqp_ref.sh`
  before editing, as `arith-ref` did.

Subtotal of the ranked steps: 86 + 24 + 20 + 17 + 17 + 2 + 1 = **167 cases, all of the
pinned ones** (104 TEX_COORD + 63 FMA). The upper bound, if the recounts bear out, is the
table's 182 + 139 minus the 37 FMA cases that compile; the rest has no shape yet.

## 5. Overlaps, and what is not duplicated

- **`arith-lower-1`** ([`mali-arith-lowering.md`](mali-arith-lowering.md)) builds `mod`,
  `smoothstep`, `a / b` in the **fragment** walk and the mediump pair builder. None of its
  steps 1a-6 touches the vertex `Ffma` arms or the cube direction, so steps 1-3 and 6 above
  are disjoint. Overlap: (a) step 5 (`fma-trig`) is the vertex form of its step 7; build the
  shared pieces (copysign, `FCMP`+`CSEL` sign, `FRSQ`+`FRCP` sqrt) once, in whichever lands
  first, and do not re-derive Mesa's `acos`/`asin` sequence (its table row has it); (b) the
  rows `mod`, `smoothstep`, `mix`, `reflect` of the arith table are the *fragment* ones;
  the vertex `mod`/`smoothstep`/`mix`/vector `reflect` already compile (the 37 cases) and are
  not touched here; `fma_reflect_float` is the vertex scalar `reflect` and is **not** in arith.
- **`fclane`** ([`mali-fclane-lowering.md`](mali-fclane-lowering.md)) changes how
  `gl_FragColor`'s lanes are taken. The cube shapes' colour is the texel as it is
  (`tc_cube_face`, `_rnd_sub`: `TEX_SINGLE` result straight to the blend) and the compare
  harness of `tc_cube_const` writes `vec4(result, result, result, 1.0)` of a product
  (a `FRAGCOLOR_LANE` text candidate: the same `isOk` fragment as `fma_const_asin`). If
  `fclane-lower-1` lands first, step 4's `tc_cube_const` reference may move through it with
  no step-4 work; if not, step 4 owns the harness lanes it needs.
- **`constop` / `negzero-lower` / `cfold-ref`** (plan 377, plan 387): step 3 is the
  constant `ffma` fold they own for other operators. Do not write a second fold: route the
  `ffma` through `vs.alu`.
- **`intval`** (`tc_2d_scalar`, and the compare/select parts of step 5): the boolean
  constants of `shaders.random` programs are its shapes.
- **`unif2-design` / `mixcol-design`**: no overlap in these 15 shapes (checked: none has a
  uniform array, a `mixprec`/`colourpair` shape or an int uniform).

### Verdicts at risk (`tests/mali_glmark2_corpus.rs`)

`tests/fixtures/mali_glmark2_verdicts.txt` pins the first refusal per glmark2 scene. The rules:
a verdict may only move forward (to `ok`), a change in place is a regression.

- **Texture coordinate**: scenes 2-4 (`texture:texture-filter=*`, fs `425b`) and scene 20
  (`ideas`, fs `df64`) are now refused with this text. Steps 1 and 4 change only
  `cube_dir_measured` and the cube arm, not the 2D arm `425b` goes through; if a step reaches
  them (step 7, `tc_2d_scalar`), the verdict moves to `ok` or to the next text: the fixture is
  rewritten with `MALI_GLMARK2_WRITE=1` in the same commit. A change in `index`
  (`nir_fs.rs:8118`) would move them.
- **FMA**: the vertex walk's matrix, `mix`, `mod`, `smoothstep`, `cross` and `reflect` arms
  compile 30+ glmark2 vertex shaders (`ok`: shading, bump, effect2d, buffer, jellyfish and
  the rest), pinned byte-equal by `gm2_fma_*`, `gm2_arithconst_*` and `vsfma_*`. Steps 2, 3
  and 6 edit `is_mat_term`, the arm order before 4781 and `is_reflect`: an `ok` vertex shader
  can move to a *different* refusal if the new predicate steals a shape from an old arm.
  Mitigation: add each new arm **after** the old ones and test it against
  `mali_glmark2_corpus` and `mali_vs_fma.rs`; a scene that was `ok` and is now something else
  is a regression the corpus test fails on.
- Step 3 interacts with the verdicts `...is not a finite normal float` (scenes 6, 8, 9, 10,
  11) and `a constant in a fused multiply-add` (scenes 7, 21): those are the *constant*
  texts of plan 377 and the fold must not claim them. Keep to the three-const-sources case.
- The unit tests that pin these texts (`tests.rs:1198` `reflect(p, vec4(0.0))`; `tests.rs:1281-1284`,
  `3636-3639`, `3761`, `3787`, `3842` for `TexCoord`) must be re-pinned by the step that moves
  them; steps 1 and 4 touch the `fstex_cubeface*` pins at `tests.rs:3636-3639` (their direction
  shapes are the six faces: `fstex_cubeface0..3` are refused as `TexCoord` and are the host-side
  twin of `tc_cube_face`; they are the first test to flip).

## 7. Measured sites at a7260420

Measured, not read: a scratch test (not committed) compiled every `tools/mali/testdata/deqp_fmaloc_*`
reference through the Mali online compiler as `tests/mali_deqp_fmaloc.rs` does (features
`host-test,gpu-mali,shaderc-online,no-shader-table`), with every `Err(MaliError::{FusedMultiplyAdd,
TexCoord,TexKind})` temporarily made to print its own `file:line` and, for `tc_2d_scalar`, the
coordinate lanes' `Val`. The instrumentation was reverted; nothing under `src/` or `tests/` changed.
All 22 references are refused, each with the text `REFUSALS` pins (none moved). Line numbers are
those of a7260420 and drift, as section 2's did.

| Reference | Text | Measured site | Step of section 4 |
|---|---|---|---|
| `fma_constmat_mat`, `fma_const_asin`, `fma_const_reflect`, `fma_const_matmat` | FMA | `nir.rs:4816`, the vertex walk's catch-all `AluOp::Ffma` (section 2.1 quoted 4781) | 3 `fma-const` (constprod K4, below) |
| `fma_constmat_vec`, `fma_attrmat_vec`, `fma_attrmat_unimat` | FMA | `nir.rs:4816` | 2 `fma-matrix` (constprod K6) |
| `fma_asin`, `fma_atan`, `fma_atan2` | FMA | `nir.rs:4816` | 5 `fma-trig` (constprod K5) |
| `fma_reflect_float` | FMA | `nir.rs:4816` | 6 `fma-reflect-float` (constprod K7) |
| `tc_cube_face` | TexCoord | `nir_fs.rs:6608`, the mediump implicit cube arm, as section 2.2 says | 1 `tex-cube-face` |
| `tc_cube_const`, `tc_cube_rnd_sub` | TexCoord | `nir_fs.rs:6608` | 4 `tex-cube-misc` |
| `tc_2d_scalar` | TexCoord | **`nir_fs.rs:9924`**, the pair builder's lone-computed check (`nd.src[0] != nd.src[1]`), not 6808 | none: see below; step 7 is void |
| `tk_bias_uniform`, `tk_bias_varying`, `tk_proj_vec3`, `tk_proj_vec4`, `tk_rnd_bias`, `tk_rnd_proj` | TexKind | `nir_fs.rs:6786`, the generic 2D arm's non-plain look-up | none in section 4: T2 (section 6) |
| `tk_cube_bias` | TexKind | `nir_fs.rs:6577`, the cube arm's kind check | none in section 4: T2, after step 1 |

The eleven FMA references share one site. The walk reaches 4816 after `is_mat_term`, `is_fmod`,
`is_cross`, `is_mix`/`is_smoothstep_inner` and `is_reflect` all fail; which predicate each shape
misses is still section 2.1's reading (the scratch test did not print the source shapes). What is
measured is that none of them is folded or accepted earlier.

### `tc_2d_scalar`: the inference is refuted

Section 2.2 said the bool constant `c` reaches the generic 2D arm as `Val::Undef`, so the case
would be an `intval` shape. It does not. At the generic arm the lanes are
`s = Val::Const(0x3F800000)` (1.0: `const bool c = bvec2(1.0, 0).x` is a folded true lane) and
`t = Val::Lane(4, 0)` (the varying `b`). Neither is `Undef`, so the arm takes
`TexCoord::Computed` and returns normally. The refusal comes later, at `nir_fs.rs:9924`: the
pair builder's rule "a computed coordinate: one fp16 lane in both" requires `nd.src[0] ==
nd.src[1]` (case 121's `vec2(c.w * 0.5)`), and here the two lanes differ (constant, varying).
So `tc_2d_scalar` is a **coordinate-side shape with a mechanism of its own**: a computed 2D
coordinate of two different lanes in the pair builder (`coord_lane`, `nir_fs.rs:9676`), whose
reference has `MOV.i32 r2, 0x3F800000` and `MOV.i32 r3, r0^` at fp32 in a mediump shader.
Section 3.2's "no coordinate work, a check after `intval`" and step 7 are wrong: nothing in
`intval` moves this refusal. It belongs to the TexCoord family (T1), as a step beside step 1:
one reference, one case (`shaders.random.texture.fragment.12`). Its session starts by reading
which words Mesa gives the pair, since `coord_lane` builds none for two different lanes.

### Which FMA rows belong to constprod

All eleven. `mali-lowering-schedule.md` 4a gives this note's 63 FMA cases to constprod (K4 to K7)
and leaves fmaloc T1 and T2. By reference: K4 (constant calls through the `alu_sized` route):
`fma_const_asin`, `fma_const_reflect`, `fma_const_matmat`, and `fma_constmat_mat` (a constant
`mat3 * mat3`; assumed, the constprod note does not name it); K5 (`acos/asin/atan/atan2` of a
vertex value): `fma_asin`, `fma_atan`, `fma_atan2`; K6 (matrix products of attributes and
constants): `fma_constmat_vec`, `fma_attrmat_vec`, `fma_attrmat_unimat`; K7 (scalar `reflect`):
`fma_reflect_float`. Section 4's steps 2, 3, 5 and 6 are therefore constprod's. What stays here is
steps 1 and 4 (cube directions), the `tc_2d_scalar` step above, and T2 (`tk_*`).

## 6. Not in this note

- The `tk_*` references (bias, projection, cube bias: 27 cases) are `MaliError::TexKind`;
  `tk_cube_bias` needs step 1's cube sequence first.
- `TooManyLocals` has no case left; the 160 cases moved to other texts
  (`mali-deqp-fmaloc.md` section 1).
- The two `attribute_location.bind_aliasing.max_cond_vec{2,3}` `Defect(Encode)` cases are not
  a lowering gap.
- `tests/mali_deqp_fmaloc.rs` is not yet in the `tools/check.sh` list of `mali_deqp_*` files
  (`tools/check.sh:2456-2460`); the first step to land adds it.
