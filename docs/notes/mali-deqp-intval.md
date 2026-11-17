<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 "integer or boolean value" refusals, Mesa 26.2.1, and a lowering plan

Plan #377, item study-intval (issue #20; stage 5, issue #7). A study: nothing
under `src/` or `tools/` changed and no reference was made. It covers the
refusal `Valhall: an integer or boolean value is not lowered yet`
(`MaliError::IntValue`) in section M of `docs/es2-cts-failures.md`, the tally
of the 37305608 tree (2026-11-15): **FS 1,861 Fail, VS 1,317 Fail, plus 41
NotSupported** (37 FS, 4 VS). The `refusals.tsv` of the four slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` hold 3,223 cases with
this refusal: the 3,178 Fail and 41 NotSupported above, and 4 Pass.

`docs/notes/mali-deqp-families.md` (item measure) did not exist when this was
written. The shapes below are grouped from the case names and the dEQP
generators' source. They do not record the first refusal of each shader pair.
Item ref-intval reconciles them with the measure's table (see "Handoff" in
the item report).

## What was read first (plan #367 and earlier, the int notes)

- `docs/notes/mali-int-loops.md`: the `intloop_*` references (loop counters,
  `int_compare`, `tri_int`, `idiv`, `dynidx`) and its six-step order.
- `docs/notes/mali-int-alu.md`: the integer ALU group (`/ % >> ^ ~ min max`,
  `int()`/`uint()`, `bool(float)`), mostly ES 3.00.
- `docs/notes/mali-int-uniforms.md`: int, uint and bool uniform layout in the
  FAU (`intuni_*`).
- `docs/notes/mali-int-and-size.md`: typed int/bool values in the walks
  (`intbig_*`, `intbool_76/89/199`, `intbool_retnested`). It lists five
  proposed lowering items. Item 3, **fs-int-arith** (`Ineg`, `Imul`, `Iadd`,
  `Ine`, `b2f` of fp16 sources in fp32 integer arithmetic, `tri_int`), is
  still open and overlaps step 1 below.
- The glmark2 references of #367: `gm2_intloop_{cond,const,func,lamp,uniform}_{fs,vs}`,
  `gm2_cf_*` (call, calls, constloop_struct, discard, earlyret, ifand,
  ifelse, ifuni, loop1_if, loop_const, loop_uni) and the `gm2_varylink_*`
  shapes. They contain only int **loop counters**: `for (int i = 0; i < N or
  u_loops; i++)`, with `float(i)` and `i == k` inside.

**Left to #367 and the other families.**

- Int loop counters, constant-bound or uniform-bound loops, their `ilt` and
  `iadd` of the counter, and `float(i)`: #367's INT_VALUE and its loop items.
- Run-time loops: the NotSupported cases `shaders.loops.*_dynamic_iterations`
  (11), `shaders.return.return_in_{dynamic,infinite}_loop_*_vertex` (4) and
  `shaders.functions.control_flow` (1). These go to study-loops and #367.
- Run-time indices: `shaders.indexing.tmp_array` (16 NS),
  `indexing.matrix_subscript` (4 NS), `fragdata.valid_dynamic_index` (1 NS),
  and `struct.local`/`struct.uniform` (4 NS) when they index. These are the
  `dynidx` compare ladder (mali-int-loops.md, "dynidx"), and go to
  study-uniarray and #367.

This note keeps the int and bool **values** that are not loop counters or
indices. These are the Fail rows: 1,861 FS and 1,317 VS.

## 1. Where the compiler refuses (every site of `MaliError::IntValue`)

The variant is declared at `src/shaderc/online/mali/mod.rs:126`, listed in
`GAPS` at `src/shaderc/online/mali/mod.rs:337`, and given its text at
`src/shaderc/online/mali/mod.rs:413`. It is returned at six sites:

| site | walk | what it refuses |
|---|---|---|
| `src/shaderc/online/mali/nir.rs:2910` | vertex (`build`) | `IrOp::ConstBool`: any boolean constant that reaches the vertex walk. `ConstI32` is taken (its bits) but a bool literal is not, so `true`/`false` never becomes NIR's 0 / ~0. |
| `src/shaderc/online/mali/nir.rs:3087` | vertex, `AluOp::Imod` | `imod` whose divisor is not a positive power-of-two constant. Only the `iand(a, 2^k-1)` rewrite is taken. ES 1.00 has no `%`, so dEQP-GLES2 should not reach it, except through the front end's own lowering. |
| `src/shaderc/online/mali/nir.rs:3211` | vertex, the integer-op arm | an `iadd` that is not `isub` in disguise (`iadd(a, ineg(b))`). A general `a + b`, `i++`, `+=` of ints is refused: "no reference draws another sum in this stage". |
| `src/shaderc/online/mali/nir.rs:3236` | vertex, the integer-op arm | every other integer or boolean op: `imul`, `idiv`, `iand`, `ixor`, `ilt`, `ile`, `ine`, `b2i`, `i2b`, `udiv`, `umod`, `ult`, `ule`, `imin`, `imax`, `umin`, `umax`, `inot`. Only `ieq`, `ior`, `bcsel`, `bany`/`ball` (`vs_reduce`) and the conversions and shifts (`f2i`, `f2u`, `i2f`, `u2f`, `ftrunc`, `ishl`, `ishr`, `ushr`, `ineg`, `isub`) are taken. |
| `src/shaderc/online/mali/nir_fs.rs:1533` | fragment, `Fs::mask_op` | `ior`/`iand` of two booleans that are neither both 32-bit (`bool32`: bool uniform words, int compares) nor both fp16 compare masks (`mask16`). This covers a mix of the two widths, and a bool that is not a mask at all (a `bcsel` result, or a bool read through a local). |
| `src/shaderc/online/mali/nir_fs.rs:1624` | fragment, `Fs::int_lane` (fn at `nir_fs.rs:1559`), last arm | every integer/bool op of a mediump program that is not one of the forms already drawn: `f2i` of an fp16, uniform or fp32 lane; `i2b` of a bool uniform; `bnot`/`inot` of a mask; `ieq`/`ine` against a constant true/false; `ilt`, `ile`, `ieq`; `iadd`, `i2f`, `u2f` and `idiv` of non-mask values. It refuses `ine` of two run-time ints, `imul`, `ineg`, `isub`, `b2i`, `imin`/`imax`, `ixor`, `inot` of a non-mask, and any arm whose guard fails (for example `iadd` of a mask, as in `int(b) + 1`). |

The highp fragment walk reaches `int_lane` through the same dispatch
(`nir_fs.rs:4718`). There is no separate highp refusal site.

## 2. The dEQP shapes (after #367)

Counts are from `refusals.tsv` (Fail and NotSupported together, all with this
refusal), grouped by case name. The shader text comes from the generators of
VK-GL-CTS main, read from GitHub:

- `modules/glshared/glsShaderLibraryCase.cpp` (`genCompareOp`,
  `genFragmentShader`, `specializeFragmentShader`);
- `modules/gles2/functional/es2fShaderOperatorTests.cpp` (`ShaderOperatorCase`).

The case names are those of the 37305608 tree's mustpass.

| shape | cases (FS / VS) | what the shader does with ints/bools | likely first site |
|---|---|---|---|
| **R. shader-library result check** (`shaders.{swizzles,preprocessor,conversions,functions,scoping,struct,indexing,constant_expressions,conditionals,...}`, the `*_fragment` cases) | 1,080 / 103 | `bool RES = isOk(out0, ref_out0, 0.05) && ...;` then `gl_FragColor = vec4(RES, RES, RES, 1.0)`. A float `isOk` is `abs(a-b) <= eps*abs(b)+eps` (an fp16 `fge` mask, or `all(lessThanEqual(..))` of vectors). An int `isOk` is `a == b`, and an int input is `int(v_x * 1.0025)` of a float varying. `ref_*` are uniforms (`int`/`bool`/`ivec` for those outputs). | FS `mask_op` (`nir_fs.rs:1533`) when a 32-bit `ieq` meets an fp16 mask; `int_lane` (`nir_fs.rs:1624`) for `ine` of two run-time ints or `inot` of a non-mask. Then the `vec4(RES)` `b2f`. The VS cases (103) are `shaders.functions.qualifiers`, `conversions.*`, `scoping.valid` and `constant_expressions`: `nir.rs:3236` (`iand`, `inot`) or `nir.rs:2910`. |
| **A. operator int arithmetic** (`shaders.operator.{binary,unary}_operator.{add,sub,mul,div,*_assign_*,pre/post_increment/decrement,plus,minus}`, int/ivec2-4, lowp/mediump/highp, and `ivecN_int` mixes) | 516 (all int `operator.*`) / 408 | `int in0 = int(a_in0.z)` (VS) or `int(v_in0.z)` (FS), `res = in0 op in1`, `color = vec4(res) * scale + bias`. lowp int inputs are mediump interpolators. | VS: `nir.rs:3211` (`iadd`), `nir.rs:3236` (`imul`, `idiv`). FS: `int_lane` (`nir_fs.rs:1624`) for `imul`, `ineg`, `isub`, and `iadd` once an operand is a mask. |
| **C. compares to a float colour** (`operator.{int,bool,float}_compare`, `binary_operator.{less,less_or_equal,greater,greater_or_equal,equal,not_equal}`) | 114+28+16+24 / 123+28+28+24 (approx.; see note 1) | `bvecN res = lessThan(in0, in1)` / `res = in0 < in1`, then `color = vec4(res)` / `float(res)`. Bool inputs are `in0 > 0.0` or `greaterThan(v_in0, vec4(0.0))`. | VS: `nir.rs:3236` (`ilt`, `ile`, `ine`, `b2i` via `vec4(bvec)`). FS: `nir_fs.rs:1624` (`ine` of run-time ints, `inot` of `equal`'s result), `nir_fs.rs:1533` (`equal(bvec,bvec)` is `ieq` of two masks of different widths). |
| **L. bool logic** (`binary_operator.logical_{and,or,xor}`, `unary_operator.not`, `operator.selection`, `operator.sequence`) | 15+12+6 / 9+4+4 (approx.) | `res = in0 && in1`, `^^`, `!in0`, `b ? x : y`, `(a, b)` | VS: `nir.rs:3236` (`iand`, `ixor`, `inot`). FS: `nir_fs.rs:1533`, `1624` (`ixor`). |
| **G. random shader generator** (`shaders.random.{scalar_conversion,conditionals,exponential,trigonometric,basic_expression,swizzle,texture,comparison_ops,all_features}`, `fragment_ops.interaction.basic_shader`) | 180 / 590 | rsg programs with `bool` and `int` locals, literals `true`/`false`, `int(f)`/`float(i)`/`bool(f)`, `if` on bools. | VS: mostly `nir.rs:2910` (`ConstBool`), then `3236`. FS: `1624`. Needs the measure. |
| **U. int and bool uniforms in the VS** (`uniform_api.value.{assigned,initial}`, `uniform_api.info_query.*`, `uniform_api.random.*`, `state_query.shader.uniform_value_boolean`) | 0 / 88 | each uniform is compared with its expected value and the comparison is folded into the colour. For `bool`/`bvec`, the uniform word is `i2b` (`ine 0`). | `nir.rs:3236` (`i2b`, `ine`, `iand` of the compares). Likely after study-unitype's VS uniform-type gate: the walk reaches the op first. |
| (left) run-time loops and indices | 37 NS / 4 NS | see above | #367, study-loops, study-uniarray |

Note 1: `float_compare` cases are counted with C because the refused value is
the bool vector of the compare. The overlap between the C and L name groups
is not separated here. The exact split is the measure's.

## 3. What Mesa 26.2.1 does

Source: `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` (the tarball tree of
`tools/mali/README-pan_shc.md`). Lines are of that tree.

**Native integers, no `lower_int_to_float`.** panfrost sets
`caps->integers = true` (`src/gallium/drivers/panfrost/pan_screen.c:581`) and
`glsl_feature_level` 140 or 330 (`pan_screen.c:767`). The state tracker
therefore sets `NativeIntegers` for every GLSL version, ES 1.00 included
(`src/mesa/state_tracker/st_extensions.c:1285`, the `GLSLVersion >= 130`
branch). The GLSL compiler keeps `int` as a real 32-bit integer and never
runs the int-to-float lowering that a GPU without native ints gets. A
`bool` uniform's true is `~0` (`UniformBooleanTrue`,
`st_extensions.c:1356`), the word our FAU already pushes
(`mali-int-uniforms.md`). `caps->int16 = false` (`pan_screen.c:591`), so
lower_precision does not narrow ints (`st_extensions.c:311` gates it on
`int16`). **A mediump or lowp `int` is a 32-bit value in both stages.** Only
floats go to fp16. This is what `Fs::int32` already models.

**GLSL IR to NIR** (`src/compiler/glsl/glsl_to_nir.cpp`):

- `ir_unop_logic_not` becomes `inot` (`:2149`).
- `i2f`, `b2f`, `f2i`, `f2b`, `i2b`, `b2i` (`:2181`-`:2188`) become
  `i2f32`, `b2f32`, `f2i32`, `fneu(a, 0)`, `ine(a, 0)` and `b2i32`.
- Integer `ir_binop_div` becomes `idiv` (`:2423`).
- `logic_and` and `logic_xor` (`:2459`, `:2465`) become `iand` and `ixor` of
  1-bit bools. `logic_or` becomes `ior`.
- `ir_binop_less` (`:2479`) and `ir_binop_equal` (`:2497`) become
  `ilt`/`flt` and `ieq`/`feq` by type. `>=` and `>` swap their sources, as
  the int notes say.
- `all_equal`/`any_nequal` (`:2513`) become `ball_iequal`/`bany_inequal`,
  which `nir_lower_alu_width` reduces as `ior`/`iand` chains.

**NIR passes that move these shapes** (`src/panfrost/compiler/bifrost/`):

- `bifrost_compile.h:89` `lower_isign` and `bifrost_compile.h:126` `has_isub`:
  `a - b` stays `isub`.
- `bifrost_nir.c:1070` `nir_opt_idiv_const` (a constant divisor becomes a
  multiply-high sequence).
- `bifrost_nir.c:1078`-`1081` `nir_lower_idiv` with `allow_fp16 = true`. Its
  float type is `sz * 2` bits (`src/compiler/nir/nir_lower_idiv.c:103`), so a
  32-bit `idiv` goes through the fp32 reciprocal sequence (`emit_udiv`,
  `nir_lower_idiv.c:32`). This is the 55-instruction sequence of
  `intloop_idiv` / `intbig_case79`.
- `bifrost_nir.c:350` `pan_nir_lower_bool_to_bitsize`
  (`src/panfrost/compiler/pan_nir_lower_bool_to_bitsize.c:313`). Each 1-bit
  bool takes its compare's operand width: an fp16 `flt` gives a 16-bit
  0/~0, and an `ieq` of ints gives 32 bits. An `iand`/`ior`/`bcsel` that mixes
  them gets an `i2i`/`u2u` conversion to one width (`lower_bool_to_bitsize_instr`,
  `pan_nir_lower_bool_to_bitsize.c:282`). This is the case `Fs::mask_op`
  refuses today.
- `bifrost_nir_algebraic.py:126`-`129`: `b2f{16,32}(a)` becomes
  `bcsel_pan(a, 1.0, 0.0)`, and `b2f(inot(a))` becomes
  `bcsel_pan(a, 0.0, 1.0)`. Each `vec4(RES)` / `vec4(bvec)` colour is
  therefore a select of constants: `MUX.v2i16` at a 16-bit store, or
  `CSEL.u32.eq` at 32 bits.

**Valhall instructions** (`src/panfrost/compiler/bifrost/bifrost_compile.c`,
`bi_emit_alu`):

| NIR | line | Valhall |
|---|---|---|
| `ieq_pan`, `ine_pan`, `ilt_pan`, `ige_pan` | `:3253`-`3258` | `ICMP_OR.s32.{eq,ne,lt,ge}.m1` (`.u32` for eq/ne in the disassembly: the type of equality does not matter), result type M1 (0 / ~0) |
| `ult_pan`, `uge_pan` | `:3261`-`3264` | `ICMP_OR.u32.{lt,ge}.m1` |
| `iadd`, `ineg`, `isub` | `:3306`, `:3351`, `:3355` | `IADD.s32` (`IADD_IMM.i32` once a constant is folded), `ISUB.s32 0, a`, `ISUB.s32` |
| `imul` | `:3367` | `IMUL.i32` |
| `iabs` | `:3371` | `IABS.s32` (inside the idiv sequence) |
| `iand`, `ior`, `ixor`, `inot` | `:3375`, `:3379`, `:3383`, `:3387` | `LSHIFT_AND`, `LSHIFT_OR`, `LSHIFT_XOR` with shift 0, and `LSHIFT_OR 0, ~a` (`.i32`, or `.v2i16` for 16-bit masks) |
| `imin`/`umin`, `imax`/`umax` | `:3061`-`3070` | `CSEL.{s32,u32}.lt` / `.gt` (no MIN/MAX op for ints) |
| `b2i32` | `:3246`-`3251` | `MUX.i32 0, 1, b, int_zero` |
| `f2i32` | `:3100` | `F32_TO_S32.rtz`, or `F16_TO_S32.rtz` of an fp16 source |
| `i2f32` | `:3193` | `S32_TO_F32` |
| `bcsel_pan` | `:2967` | `MUX.{i32,v2i16}` with `int_zero` (a `b2f` after the algebraic rule; `CSEL` once `va_lower_isel` fuses its compare) |

Measured instances of most of these rows already exist in `intloop_tri_int`,
`intloop_int_compare`, `intloop_idiv`, `intbool_76_boolany` and
`intbool_199_vsc` (`tools/mali/testdata`). None of them draws a dEQP shape:
the `isOk` chain, an int `ref_*` uniform compared with `ieq`, or a VS
`imul`/`iadd`/`ilt`.

Per shape:

- **R.** Each float `isOk` is an fp16 `fge` (16-bit mask), and each int
  `isOk` an `ieq` of `F16_TO_S32(v * 1.0025)` with a 32-bit uniform word
  (32-bit mask). The `&&` chain is `iand` after
  `pan_nir_lower_bool_to_bitsize`, which narrows the 32-bit mask to 16 bits
  (or widens the other) at the mix. The colour is
  `bcsel_pan(RES, 1.0, 0.0)` in each of three lanes, which CSE makes one
  value. A float-only check is all 16-bit: `FCMP_OR.v2f16`,
  `LSHIFT_AND.v2i16`, `MUX.v2i16`.
- **A.** Integer arithmetic is fully 32-bit. `vec4(res)` is `S32_TO_F32`,
  then the fp16 multiply-add of scale and bias after `F32_TO_F16`. lowp and
  mediump give the same program as highp, apart from the colour's precision.
  `/` is the `nir_lower_idiv` sequence per lane, unless `nir_opt_idiv_const`
  folds a constant divisor.
- **C.** `ICMP_OR`/`FCMP_OR` into a mask, then `bcsel_pan` to the colour
  (`MUX.v2i16`, or `CSEL.u32.eq` in the VS's fp32). `equal(bvec, bvec)` of
  bools that are `greaterThan(v, 0.0)` is `ieq` of two masks of the same
  width (both fp16 in a mediump FS, both fp32 in the VS).
- **L.** `iand`/`ior`/`ixor`/`inot` (`LSHIFT_*`). `?:` on a bool is a
  `bcsel`. `,` drops its left side.
- **G.** Constant folding first (`nir_opt_constant_folding`): most `true` /
  `false` literals and `bool(c)` fold away before the back end, so Mesa's
  program has a `bcsel` or nothing. Our VS walk refuses the literal itself
  (`nir.rs:2910`) before any fold could apply.
- **U.** A bool uniform is a 32-bit FAU word (`~0`/`0`), and `i2b` is that
  word. An `int` uniform's compare is `ICMP_OR.u32.eq` against an `.b`/FAU
  constant.

## 4. Lowering plan (two steps, one session each)

Each step is byte-compared to the `deqp_intval_*` references that phase 1b
(item ref-intval) makes with `tools/mali/drm_shim_ref.sh run` +
`pan_ref_diff.py extract --draw`, using the pan_shc method of
`tools/mali/testdata/gm2_*`. The shader pairs come from the measure's
`tools/mali/testdata/deqp_shapes/`. Tests go in
`tests/mali_deqp_intval.rs`. Any glmark2 verdict that moves updates
`tests/fixtures/mali_glmark2_verdicts.txt` in the same commit (CLAUDE.md;
`corpus-verdicts.txt`/`cases.txt` as #367 does).

### Step 1 (lower-intval-1): the fragment walk (shapes R, A-FS, C-FS, L-FS), about 1,700 FS cases

Files: `src/shaderc/online/mali/nir_fs.rs`, and `emit.rs`/`bir.rs`/`pack.rs`
only where a new row is needed.

1. `Fs::mask_op` (`nir_fs.rs:1527`-`1534`): replace the refusal at `:1533`
   with Mesa's `pan_nir_lower_bool_to_bitsize` rule for a mix. Convert the
   32-bit operand to 16 bits, or the reverse, as the reference shows (read
   it from `deqp_intval_reslib_int_fs`), then the `.v2i16` or `.i32` op.
2. `Fs::int_lane` (`nir_fs.rs:1559`-`1625`): add `ine` and `ieq` of two
   run-time ints, and `ieq` of an int and a uniform word (`ICMP_OR.u32.ne`/`.eq`).
   Add `imul` (`IMUL.i32`), `ineg`/`isub` (`ISUB.s32`), and `iadd` with a
   constant (`IADD_IMM.i32`). Add `inot` of a 32-bit bool, `ixor` of masks
   (`LSHIFT_XOR`), `b2i32` (`MUX.i32`) and `imin`/`imax` (`CSEL.s32`).
   The mediump-int note's fs-int-arith item (`tri_int`) is the same work. Do
   it here and take `intloop_tri_int` as an extra byte-equal check.
3. The colour of `vec4(RES, RES, RES, 1.0)`: `b2f` of a 32-bit or 16-bit
   mask, which becomes `bcsel_pan(RES, 1.0, 0.0)` and is folded into the
   store's `MUX.v2i16` (the `mask_colour`/`select32` paths of
   `intbool_76`). Check that the three equal lanes are one CSE'd value.

References step 1 needs (item ref-intval), all with the shader library's
pass-through VS and the operator suite's VS:

- `deqp_intval_reslib_float_fs`: `swizzles.vector_swizzles.mediump_vec3_xyz_fragment`, the float-only `isOk` chain.
- `deqp_intval_reslib_int_fs`: `conversions.scalar_to_scalar.float_to_int_fragment`, an int `isOk` with the `* 1.0025` input and an int `ref_` uniform.
- `deqp_intval_reslib_mixed_fs`: a case with a float and an int output, `RES && isOk(int)` of mixed widths (from `functions.datatypes` or `conversions.vector_combine`).
- `deqp_intval_opfs_add_int_mediump`, `deqp_intval_opfs_mul_ivec4_highp`, `deqp_intval_opfs_sub_assign_ivec2_lowp`, `deqp_intval_opfs_div_int_mediump`: shape A, the FS `*_fragment` cases.
- `deqp_intval_opfs_cmp_ivec3`, `deqp_intval_opfs_equal_bvec2`: shape C.
- `deqp_intval_opfs_logical_xor`, `deqp_intval_opfs_not`: shape L.

### Step 2 (lower-intval-2): the vertex walk (shapes A-VS, C-VS, L-VS, G, U), about 1,300 VS cases

Files: `src/shaderc/online/mali/nir.rs` (`build`, the integer-op arm), with
`opt.rs`/`motion.rs` only if the link needs it.

1. `IrOp::ConstBool` (`nir.rs:2910`): NIR's 0 / ~0 as `Val::Const`, the way
   `ConstI32` is. This lets the rsg (G) and `true`/`false` literals fold
   through `fold_int`.
2. The general `iadd` (`nir.rs:3211`): `IADD.s32`, or `IADD_IMM.i32` with a
   constant.
3. The arm at `nir.rs:3236`: `imul` (`IMUL.i32`), `ilt`/`ile`/`ine`/`ult`/`ule`
   (`ICMP_OR.*.m1`, `ile` swapped to `ige`), `iand`/`ixor`/`inot`
   (`LSHIFT_*.i32`), `i2b` of a uniform word (the word itself),
   `b2i32` (`MUX.i32`), `imin`/`imax`/`umin`/`umax` (`CSEL`) and
   `idiv`/`udiv` (the `nir_lower_idiv` sequence that `Fs::idiv` already
   emits, shared). `umod` and `imod` with a run-time divisor stay refused:
   ES 1.00 has no `%`, and nothing in this family needs them.
4. `vec4(res)` of an int vector into `v_color` (`S32_TO_F32` per lane) and
   `vec4(bvec)` (`CSEL.u32.eq c, 0, 0, 1.0`), checked against the
   equal-lanes merge (study-varylink): a constant result lane may make two
   varying lanes equal.

References step 2 needs:

- `deqp_intval_opvs_add_int_highp`, `deqp_intval_opvs_mul_ivec3_mediump`,
  `deqp_intval_opvs_div_ivec2_highp`, `deqp_intval_opvs_post_increment_int`:
  shape A, the `*_vertex` cases.
- `deqp_intval_opvs_less_ivec4`, `deqp_intval_opvs_not_equal_bvec3`,
  `deqp_intval_opvs_logical_and`: shapes C and L.
- `deqp_intval_rsg_constbool_vs`: one `shaders.random.conditionals.*` VS
  that refuses at `ConstBool` in the measure.
- `deqp_intval_uniapi_bool_vs`, `deqp_intval_uniapi_ivec_vs`:
  `uniform_api.value.assigned.*` with `bool`/`bvec` and `int`/`ivec` uniforms
  (drawn with `--uniform-values`, so the compare is not folded).

What the two steps leave: the run-time loops and indices (NotSupported 41,
#367 / loops / uniarray), and any case whose next refusal is another family's
(equal varying lanes, uniform type). The measure's "next refusal" column
says how many Fail cases each step actually turns into Pass.

## 5. References (item ref-intval)

25 Mesa 26.2.1 references, `tools/mali/testdata/deqp_intval_<name>.{vert,frag,expect.json}`,
made 2026-11-16. Each is one real dEQP-GLES2 case of the 37305608 tree whose
`refusals.tsv` row is this refusal. Each pair, compiled through
`tests/mali_ref` (`compile_pair`) at 0728f44d, refuses with
`Valhall: an integer or boolean value is not lowered yet`, so the rebuilt
text reaches the same refusal as the board.

**Shader text.** The integration tip has no `tools/mali/testdata/deqp_shapes/`,
so every pair was **rebuilt** from VK-GL-CTS 93bca01861b0 (the revision
`tools/cts/README.md` and RESULTS.md use). The source was a blobless sparse
clone in `$CONNECTED_DEPS_DIR/VK-GL-CTS-sparse`. Each generator was
transcribed as a scratch script (not committed), following the source:

- **R** (shader library): the case's `values` and `both` source in
  `data/gles2/shaders/<file>.test`, parsed as `gluShaderLibrary.cpp`
  `parseShaderSource` (:518) and `removeExtraIndentation` (:473) do. They are
  then specialized by `modules/glshared/glsShaderLibraryCase.cpp`. A
  `_fragment` case uses `genVertexShader` (:141) and
  `specializeFragmentShader` (:359). A `_vertex` case uses
  `specializeVertexShader` (:285) and `genFragmentShader` (:233). Both use
  `genCompareOp` (:193) and `gluShaderLibrary.cpp` `genCompareFunctions`
  (:1912).
- **A, C, L** (`shaders.operator.*`): `ShaderOperatorCase::ShaderOperatorCase`
  (`modules/gles2/functional/es2fShaderOperatorTests.cpp:590`-`745`), with
  the operator string, input types, scale and bias of the case's table row
  (cited per reference below). The case name is built at `:1915`-`2090`.
  `de::floatToString(x, 2)` is `std::fixed`, two digits
  (`framework/delibs/decpp/deStringUtil.cpp:119`).
- **G** (`shaders.random.*`): the generator itself. VK-GL-CTS's
  `framework/randomshaders/rsg*.cpp` was built on the host (without
  `rsgProgramExecutor.cpp` and `rsgTest.cpp`) with a small main that sets
  the group's `rsg::ProgramParameters` as
  `es2fRandomShaderTests.cpp` (`createRandomShaderCase` :38, the groups
  :84 and :175) does. The main prints `rsg::Shader::getSource()` of
  both stages, as `glsRandomShaderCase.cpp:142`/`:514` uses them.
- **U** (`uniform_api.*`): `UniformCase::writeUniformDefinitions`,
  `generateVertexSource` and `generateFragmentSource`
  (`es2fUniformApiTests.cpp:1135`, `:1237`, `:1263`) for
  `UniformCollection::basic(TYPE_BOOL)` (`:345`). The `value.assigned`
  value is the case's first `de::Random::getBool()`, seeded
  `deStringHash("bool_vertex") ^ 0` (`:1899`, the default
  `--deqp-base-seed` of 0), which gives `true`. `value.initial` compares
  with `false` (`FEATURE_UNIFORMVALUE_ZERO`, `:2103`).

**Command** (the same for all 25; a Mesa 26.2.1 build from
`tools/mali/drm_shim_ref.sh build` of the tarball checked against
`third_party/mesa-26.2.1/UPSTREAM`, in `$CONNECTED_DEPS_DIR/mesa-shim`):

```sh
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/deqp_intval_<name>.vert tools/mali/testdata/deqp_intval_<name>.frag out/deqp_intval_<name>
tools/mali/pan_ref_diff.py extract out/deqp_intval_<name>.trace --draw > tools/mali/testdata/deqp_intval_<name>.expect.json
```

gl_ref.py's default draw: 64x64, triangles, no `--uniform-values`. gl_ref.py
sets no `bool` uniform, and a uniform's value does not change the program.
The 25 were drawn twice and the two sets of `expect.json` are byte-identical.

| reference | dEQP-GLES2 case (`functional.`) | source | what it pins |
|---|---|---|---|
| `reslib_float_fs` | `shaders.swizzles.vector_swizzles.mediump_vec3_xyyx_fragment` | `swizzles.test:678`, rebuilt | step 1 (R): the float-only `isOk` chain, all 16-bit: `FCMP_OR.v2f16.ge.m1` per check, `LSHIFT_AND.v2i16` for `all()` and `&&`, `MUX.v2i16` for `vec4(RES, RES, RES, 1.0)`. The study's `mediump_vec3_xyz_fragment` does not refuse with this text in the tree. |
| `reslib_int_fs` | `shaders.conversions.scalar_to_scalar.float_to_int_fragment` | `conversions.test:28`, rebuilt | step 1 (R): an int `isOk`, `F32_TO_S32.rtz` of the fp32 load (it stays f32), `ICMP_OR.u32.eq.m1` against the `ref_out0` FAU word, the 32-bit mask straight into `MUX.v2i16 ... r0.h00` |
| `reslib_bvec_fs` | `shaders.swizzles.vector_swizzles.mediump_bvec3_xyz_fragment` | `swizzles.test:5067`, rebuilt | step 1 (R): `bool(v)` is `FCMP_OR.v2f16.ne.m1`, `bvec3 == bvec3` is `ICMP_OR.v2u16.eq.m1` of 16-bit masks, then `LSHIFT_AND.v2i16`. 108 bvec swizzle FS cases. |
| `reslib_mixed_fs` | `shaders.conversions.vector_combine.bool_float_int_to_ivec3_fragment` | `conversions.test:4642`, rebuilt | step 1 (`mask_op`): a 16-bit mask (`FCMP_OR.v2f16.ne.m1`) widened (`IADD.v2u16 .h00`) into `CSEL.u32.eq` for `ivec3(bool)`, `F16_TO_S32` of `v_in2 * 1.0025`, `ICMP_AND.u32.eq.m1` and `LSHIFT_AND.i32` of 32-bit masks, then `MUX.v2i16` |
| `reslib_int_vs` | `shaders.conversions.scalar_to_scalar.float_to_int_vertex` | `conversions.test:28`, rebuilt | step 2 (R, VS): `float(int(x))` folds to `FROUND.f32.rtz` (no int op is left in the VS); the FS is the float `isOk(float, int)` of `genFragmentShader` |
| `opfs_add_int_mediump` | `shaders.operator.binary_operator.add.mediump_int_fragment` | `es2fShaderOperatorTests.cpp:1574`, rebuilt | step 1 (A): `F32_TO_S32.rtz` of each f32 lane, `IADD.s32`, `S32_TO_F32`, then the fp16 scale and bias (`FMA.v2f16`) |
| `opfs_mul_ivec4_highp` | `shaders.operator.binary_operator.mul.highp_ivec4_fragment` | `:1598`, rebuilt | step 1 (A): `IMUL.i32` per lane, 2 varying slots |
| `opfs_sub_assign_ivec2_lowp` | `shaders.operator.binary_operator.sub_assign_effect.lowp_ivec2_fragment` | `:1632`, rebuilt | step 1 (A): `ISUB.s32`, lowp ints with mediump interpolators |
| `opfs_div_int_mediump` | `shaders.operator.binary_operator.div.mediump_int_fragment` | `:1610`, rebuilt | step 1 (A): the 32-bit `nir_lower_idiv` sequence (`IABS`, `F32_TO_U32`, `FRCP.f32`, `IMUL`, `ICMP_OR.u32.ge`, `LSHIFT_XOR`, 67 words) |
| `opfs_cmp_ivec3` | `shaders.operator.int_compare.lessThan.mediump_ivec3_fragment` | `:1862`, rebuilt | step 1 (C): `ICMP_OR.s32.lt.m1` per lane, `MUX.v2i16` for `vec3(bvec3)` |
| `opfs_equal_bvec2` | `shaders.operator.binary_operator.equal.bvec2_fragment` | `:1712`, rebuilt | step 1 (C): `greaterThan` is `FCMP_OR.v2f16.lt.m1`, `bvec2 == bvec2` is `ICMP_OR.v2u16.eq.m1` and `LSHIFT_AND.v2i16` |
| `opfs_logical_xor` | `shaders.operator.binary_operator.logical_xor.bool_fragment` | `:1726`, rebuilt | step 1 (L): `LSHIFT_XOR.v2i16` of two fp16 masks |
| `opfs_not` | `shaders.operator.unary_operator.not.bool_fragment` | `:1529`, rebuilt | step 1 (L): no integer op is left. `float(!(x > 0.0))` is one `FCMP_OR.v2f16.ge.f1` (a compare with a 1.0/0.0 result). |
| `opvs_add_int_highp` | `shaders.operator.binary_operator.add.highp_int_vertex` | `:1574`, rebuilt | step 2 (A): `IADD.s32` in the varying program; the colour's fp16 `FMA.v2f16` then `F16_TO_F32` |
| `opvs_mul_ivec3_mediump` | `shaders.operator.binary_operator.mul.mediump_ivec3_vertex` | `:1598`, rebuilt | step 2 (A): `IMUL.i32` per lane |
| `opvs_div_ivec2_highp` | `shaders.operator.binary_operator.div.highp_ivec2_vertex` | `:1610`, rebuilt | step 2 (A): the idiv sequence twice (144 words of varying program) |
| `opvs_post_increment_int` | `shaders.operator.unary_operator.post_increment_effect.highp_int_vertex` | `:1543`, rebuilt | step 2 (A): `res++` is `IADD_IMM.i32` |
| `opvs_less_ivec4` | `shaders.operator.int_compare.lessThan.highp_ivec4_vertex` | `:1862`, rebuilt | step 2 (C): `ICMP_OR.s32.lt.m1` and `CSEL.u32.eq` to 1.0/0.0 per lane. `binary_operator.less` has only scalar cases; this is the study's `less_ivec4`. |
| `opvs_not_equal_bvec3` | `shaders.operator.binary_operator.not_equal.bvec3_vertex` | `:1718`, rebuilt | step 2 (C): the VS's mediump compare is fp16 too (`FCMP_OR.v2f16.lt.m1`), `ICMP_OR.v2u16.ne.m1`, `LSHIFT_OR.v2i16` for `any`, `S16_TO_S32` then `CSEL.u32.eq` |
| `opvs_logical_and` | `shaders.operator.binary_operator.logical_and.bool_vertex` | `:1722`, rebuilt | step 2 (L): `(a > 0.0) && (b > 0.0)` is rewritten to `min(a, b) > 0.0`: `FMIN.v2f16` and one `FCMP_OR.v2f16.lt.m1`, with no `iand` |
| `rsg_constbool_vs` | `shaders.random.conditionals.vertex.11` | rsg, seed 11, rebuilt | step 2 item 1 (`ConstBool`): `vec4(true, 1, 1.0, true)` folds. `v_color` is a constant, so Mesa's link drops the varying program and the FS stores the constant from the FAU. The study-varylink constant-lane rule applies too. |
| `rsg_conv_vs` | `shaders.random.scalar_conversion.vertex.2` | rsg, seed 2, rebuilt | step 2 item 1: bool and int literals, `int(bool(1))`, `float(true)` and the int globals all fold. Only `FMA.f32`/`FADD.f32` of the attributes are left. |
| `rsg_cond_fs` | `shaders.random.conditionals.fragment.18` | rsg, seed 18, rebuilt | step 1 (G, FS): mediump `int` uniforms, `vec4(int)` is `S32_TO_F32` of the FAU word, the bool constants fold. The VS's varyings are dead and dropped. |
| `uniapi_bool_vs` | `uniform_api.value.assigned.by_value.render.basic.bool_vertex` | `es2fUniformApiTests.cpp:1135`/`:1237`, rebuilt (value `true`) | step 2 (U): `compare_bool(u_var, true)` is one `CSEL.u32.eq` on the bool FAU word. The FS reads `v_vtxOut` as `src_flat32`. |
| `uniapi_bool_initial_vs` | `uniform_api.value.initial.render.basic.bool_vertex` | as above, value `false` | step 2 (U): the same program with the `CSEL` sources swapped |

What the references do not cover, and why:

- **`uniapi_ivec_vs`** (study section 4) was not made. No `int`/`ivec`
  `uniform_api` case has this refusal in the tree. All 72 `uniform_api.value.*`
  rows with it are `bool` or `bvec4` (the `basic`, `basic_struct`,
  `array_in_struct`, `struct_in_array` and `nested_structs_arrays`
  collections). The struct ones (`bool_bvec4_*`) are for after
  study-uniarray's struct uniforms.
- **`rsg_constbool_vs` and `rsg_cond_fs`** get their result from Mesa's link:
  the constant varying is moved into the FS, or the dead varyings are
  removed. `tests/mali_ref` does not reach our constant-lane path
  (`vs_constants`, see its module comment). lower-intval-2 may need the unit
  tests of `src/shaderc/online/mali/tests.rs` for the first.
- Only a sample of the rsg groups (G) and of `fragment_ops.interaction` was
  taken. The measure's per-shape counts say whether more are needed.

## 6. Lowering map (at 008306cb)

Plan #377, item map-intval. Docs only: nothing was built or run. Every
`file:line` is of commit `008306cb31fe` (the integration tip) and **replaces
the line numbers of section 1**, which were read at an older tree (its
`nir.rs:2910/3087/3211/3236` are now `3033/3234/3358/3383`; the `nir_fs.rs`
ones did not move). `emit.rs` already holds most integer rows the references
use: 32-bit `Iadd`, `Imul`, `Isub`, `Ineg`, `Ixor`, `Iand`, `Ior`, `Inot`,
`Imin`, `Imax`, `Iabs`, `B2i`, `Icmp(Lt/Ge/Eq/Ne)` (`emit.rs:65`-`:152`,
`:280`-`:327`) and 16-bit `Icmp(Eq/Ne)`, `Ior`, `Iand` (`:163`-`:190`); `bir.rs`
has `IaddS32`, `IaddImmI32`, `IsubS32`, `ImulI32`, `IaddV2u16`. The walks, not
the emitter, refuse.

### (a) Every place `IntValue` is raised

The variant: `mod.rs:126`, in `GAPS` at `mod.rs:337`, text at `mod.rs:413`.
Raised at six sites (the `tests.rs` hits are expectations):

| # | file:function (line) | condition |
|---|---|---|
| 1 | `nir.rs`, vertex walk `build` (`nir.rs:3033`) | `IrOp::ConstBool(_)`: any boolean literal in the vertex walk. `ConstI32` (`:3031`) is taken as its bits. The FS walk folds the same op to 0 / `u32::MAX` at `nir_fs.rs:4238`. |
| 2 | `nir.rs`, `build`, `AluOp::Imod` arm (`nir.rs:3234`) | `imod` whose divisor is not a positive power-of-two constant (`:3230`-`:3235`). ES 1.00 has no `%`. |
| 3 | `nir.rs`, `build`, the `F2i..Iadd` arm, `iadd` match (`nir.rs:3358`) | `iadd(p, q)` with no `ineg` source (the `isub` rewrite, `:3350`-`:3357`) that `fold_int` (`nir.rs:816`) did not fold: every `a + b`, `i++`, `+=` of ints. |
| 4 | `nir.rs`, `build`, the arm ending `AluOp::Inot => return Err(IntValue)` (`nir.rs:3383`) | `Imul, Idiv, Iand, Ixor, Ilt, Ile, Ine, B2i, I2b, Udiv, Umod, Ult, Ule, Imin, Imax, Umin, Umax, Inot` (`:3361`-`:3383`). Taken before it: `Ieq`, `Ior`, `Bcsel`, `Bany/Ball` (`vs_reduce`, `nir.rs:2657`), `Ftrunc F2i F2u I2f U2f Ishl Ishr Ushr Ineg`, `isub`. |
| 5 | `nir_fs.rs`, `Fs::mask_op` (`nir_fs.rs:1532`, raise `:1538`) | `ior`/`iand` of two booleans that are not two constants, not both `bool32`, not both `mask16`. Called from the any/all accumulation (`nir_fs.rs:4699`) only. |
| 6 | `nir_fs.rs`, `Fs::int_lane` (`nir_fs.rs:1564`, last arm `:1629`) | any op not taken by the arms above it: `F2i` of a lane that is not fp16, uniform or fp32; `I2b` of a non-bool-uniform; `Bnot/Inot` of a non-`mask_lane`; `Ieq/Ine` of a mask and a constant other than 0 / ~0; **`Ine` of two run-time ints**, `Imul`, `Ineg`, `Isub`, `B2i`, `Imin`, `Imax`, `Ixor`, `Iand`, `Ior`; `Iadd/I2f/U2f/Idiv` with a `mask_lane` source. |

Sites 5 and 6 are the only FS sites. `int_lane` is called from the `int_op(a)`
arm of the FS instruction walk (`nir_fs.rs:4709`-`:4724`; `int_op` at
`nir_fs.rs:10016`), twice recursively (`:1602`, `:1606`) and at `nir_fs.rs:3451`.
The highp walk uses the same arm.

### (b) What to change and emit

**lower-intval-1, the fragment walk** (`nir_fs.rs`; a row in `emit.rs`/`bir.rs`
only if missing). Integers stay 32-bit in a mediump shader; a boolean takes its
compare's width.

1. `Fs::mask_op` (`:1532`): replace the `Err` at `:1538` with
   `pan_nir_lower_bool_to_bitsize`'s mix rule. `reslib_mixed_fs` shows it: the
   16-bit mask (`FCMP_OR.v2f16.ne.m1`) widened by `IADD.v2u16 .h00` into the
   32-bit side, then `LSHIFT_AND.i32`. `reslib_int_fs` (32-bit
   `ICMP_OR.u32.eq.m1` straight into `MUX.v2i16`) and `reslib_bvec_fs`
   (`ICMP_OR.v2u16.eq.m1` of 16-bit masks) pin the pure cases. `mask16`
   (`:1512`) matches only `Fcmp|Ior|Iand` nodes at 16 bits; it must also take
   the 16-bit `Icmp(Eq/Ne)` node a `bvec == bvec` makes (`emit.rs:163`,
   `:167`), or `opfs_equal_bvec2` and `reslib_bvec_fs` stop earlier. The
   widening needs a node/row of its own (none in `emit.rs`).
2. `Fs::int_lane` (`:1564`), new arms before `:1629`: `Ine` of two run-time
   ints as `Icmp(Ne)` (`ICMP_OR.u32.ne.m1`, `emit.rs:280`); `Imul` (`IMUL.i32`),
   `Isub` (`ISUB.s32`), `Ineg` (`ISUB.s32 0, a`), each guarded by
   `!mask_lane` like the `Iadd` arm (`:1618`); `Iadd` with a constant must
   fuse to `IADD_IMM.i32` (`opt.rs:409`-`:421` does); `Ixor` of masks as
   `LSHIFT_XOR` at 32 bits and at 16 bits (`opfs_logical_xor`:
   `LSHIFT_XOR.v2i16`; `emit.rs:189`-`:190` have no 16-bit `Ixor`, so add the
   row and a `bir` op); `B2i` (`MUX.i32`, `emit.rs:157`); `Imin/Imax`
   (`CSEL.s32`, `emit.rs:114`-`:115`); `Inot` of a non-mask (`emit.rs:324`).
   `Idiv` exists (`Fs::idiv`, `:1700`); check `opfs_div_int_mediump` first
   before adding anything.
3. The colour `vec4(RES, RES, RES, 1.0)` / `vec4(bvec)` is `b2f` of a mask,
   folded into the store's `MUX.v2i16` by the `mask_colour`/`select32` path
   (`nir_fs.rs:1502`); only the byte compare proves it. `opfs_not` has no
   integer op in Mesa (`FCMP_OR.v2f16.ge.f1`): the `inot` must fold into the
   compare (the `Bnot` arm, `:1594`), not be emitted.
4. `intloop_tri_int` and `tools/shaders/es100/tri_int.frag` are the same work
   (mali-int-and-size.md item 3): an extra byte-equal check.

**lower-intval-2, the vertex walk** (`nir.rs` `build`; `opt.rs`/`motion.rs` only
if the link needs it). A mediump VS compare is still fp16
(`opvs_not_equal_bvec3`: `FCMP_OR.v2f16.lt.m1`, then `ICMP_OR.v2u16.ne.m1`), so
the VS meets the same width mix.

1. Site 1 (`nir.rs:3033`): `Val::Const(if b { u32::MAX } else { 0 })` per lane,
   as `nir_fs.rs:4238`. `nir.rs:897` already counts `ConstBool` as constant.
   This alone is `rsg_conv_vs`.
2. Site 3 (`nir.rs:3358`): a general `iadd` as `(Alu::Iadd, p, q)`; `opt.rs:409`
   gives `IADD_IMM.i32` (`opvs_add_int_highp`, `opvs_post_increment_int`).
3. Site 4 (`nir.rs:3361`-`:3383`): split the arm. `Imul` -> `Alu::Imul`;
   `Ilt` -> `Icmp(Lt)`, `Ile` -> `Icmp(Ge)` with swapped sources (as
   `nir_fs.rs:1614`-`:1617`), `Ine` -> `Icmp(Ne)`; `Iand`, `Ixor`, `Inot`,
   `B2i`, `Imin`, `Imax` -> the 32-bit rows; `I2b` of a uniform word -> the
   word (`uniapi_bool_vs`: one `CSEL.u32.eq` on the FAU word; `_initial_` swaps
   its sources; the uniform-type gate of study-unitype is met first for
   `bvec4`); `Idiv` -> `Fs::idiv` is a method of `Fs` over `self.nir`, so lift
   it to `Nir` (or a free function) first (`opvs_div_ivec2_highp`, the
   sequence twice, 144 words). `Udiv/Umod/Umin/Umax/Ult/Ule` stay refused (no
   reference).
4. Site 2 (`nir.rs:3234`): leave refused.
5. `opvs_logical_and` has no integer op in Mesa (`FMIN.v2f16` and one
   `FCMP_OR`): it needs an algebraic rewrite of the `iand` of two compares,
   not an op. It may land last.
6. `vec4(bvec)` (`Bcsel(c, 1.0, 0.0)` -> `B2f`) and `vec4(int)` (`I2f`) are
   already taken in `build`.

### (c) Tests each step un-ignores, in order

Remove each test's `#[ignore]` and its `PINNED` entry (`refusals_pinned`,
`tests/mali_deqp_intval.rs:211`) in the same commit.

**lower-intval-1** (12 tests): `opfs_not`; `reslib_float_fs`; `reslib_int_fs`;
`opfs_add_int_mediump`; `opfs_mul_ivec4_highp`; `opfs_sub_assign_ivec2_lowp`;
`opfs_cmp_ivec3`; `reslib_bvec_fs` and `opfs_equal_bvec2`; `opfs_logical_xor`;
`reslib_mixed_fs`; `opfs_div_int_mediump`. (The file marks `rsg_cond_fs`
step 1 too; it stays ignored, see (e).)

**lower-intval-2** (11 tests): `rsg_conv_vs`; `reslib_int_vs`;
`opvs_add_int_highp` and `opvs_post_increment_int`; `opvs_mul_ivec3_mediump`;
`opvs_less_ivec4`; `opvs_not_equal_bvec3`; `uniapi_bool_vs` and
`uniapi_bool_initial_vs`; `opvs_div_ivec2_highp`; `opvs_logical_and`. (And
`rsg_constbool_vs`, which stays ignored.)

That is 12 + 11 = 23 of the 25; the other 2 stay ignored.
The order only sequences within a step. A test can fail on a different
refusal (study-varylink's equal-lane merge on a constant lane, for the `opvs_*`
varyings): that is not a gap of the step; keep the test ignored and update
its pin text, do not delete it.

### (d) glmark2 verdicts the steps may move

Rows of `tools/mali/corpus-verdicts.txt` refused with this text (6):
`tools/shaders/es100/tri_int.frag`, `cond_complex.frag`, `loop_full.frag`,
`tri_discard.frag`, `varyings15.frag` (all FS: step 1; `loop_full` may refuse
next on a loop, #367) and `ret_nested.vert` (VS: step 2). Rows of
`tests/fixtures/mali_glmark2_verdicts.txt` (3, all `20 ideas:speed=duration`):
`fs c565` (step 1), `vs bf4b` and `vs c265` (step 2). A row that moves, to `ok`
or to the next refusal, is regenerated in the lowering commit
(`MALI_GLMARK2_WRITE=1`, the test's header; `corpus-verdicts.sh --write`).
Every other row (`ok` and other refusals) can also move through a wrong width
rule, since `intbool_*`, `intbig_*`, `intloop_*` and `facing.frag` reach the
same FS arms: their byte compares must stay unchanged. `tests.rs` expects
`IntValue` at `:1235` (VS `gl_Position = p * float(int(p.x) * 3)`, the `imul`:
step 2), `:1251` and `:7096`; they are not byte-compares and move in the same
commit.

### (e) Risks, and what cannot be compared

- **`rsg_cond_fs` and `rsg_constbool_vs` cannot be byte-compared.** Mesa's link
  removes the dead varyings (`rsg_cond_fs`) or moves the constant varying into
  the FS (`rsg_constbool_vs`). `tests/mali_ref` compiles through
  `compile_pair` and does not reach our link-time constant-lane path
  (`vs_constants`; its module comment). They stay `#[ignore]`d with that reason
  after both steps; the unit tests of `src/shaderc/online/mali/tests.rs` hold
  them. Their shader bodies (mediump int uniform `S32_TO_F32` of the FAU word;
  folded `vec4(true, 1, 1.0, true)`) are exercised by `reslib_int_fs` and
  `rsg_conv_vs`.
- **#367 overlap.** `int_lane` and `int32` are where #367's INT_VALUE and loop
  items add arms (`mali-int-loops.md`); lower-loops-1 and lower-intval-1 both
  edit `nir_fs.rs`. Re-read `int_lane` on the latest main before editing and
  put each arm next to its neighbour so a conflict is mechanical.
- **The width rule.** A wrong widening in `mask_op` changes the 16-bit
  any/all of existing mediump shaders. Keep the `bool32` and `mask16` cases
  first and the mix last.
- **Folding hides Mesa's program.** `rsg_*` and `reslib_*` mostly fold in Mesa.
  `fold_int` (`nir.rs:816`) covers the compares and some arithmetic
  (`:835`-`:839`); add a case with each new arm (`Imul`, `Iand`, `Ixor`,
  `Imin`, `Imax`, `B2i`), or a literal folds in Mesa and not here.
- **`opfs_not` and `opvs_logical_and`** are algebraic rewrites, not integer
  ops: a faithful `inot` or `iand` is a byte mismatch.
- **Division.** `Fs::idiv` numbers its nodes "node for node in the order of
  case 79's NIR"; the VS's CSE and scheduling may not give the same order, so
  `opvs_div_ivec2_highp` may need its own session.
- **Next refusal.** How many of the 3,178 Fail become Pass is not known from
  this map; the measure (`mali-deqp-families.md`) gives the refusal after this
  one. The 88 `uniform_api` VS cases also need lower-unitype, and the `random.*`
  VS cases lower-varylink.
- Not measured: no build or test was run; sites and lines were read.

## 7. Step 1 (lower-intval-1): the fragment walk, done

Plan #377, item lower-intval-1. The twelve step-1 references of section 5
compile and are byte-equal to Mesa 26.2.1 (`tests/mali_deqp_intval.rs`, the
words, facts, FAU words and padding of every region through `tests/mali_ref`):
`reslib_float_fs`, `reslib_int_fs`, `reslib_bvec_fs`, `reslib_mixed_fs`,
`opfs_add_int_mediump`, `opfs_mul_ivec4_highp`, `opfs_sub_assign_ivec2_lowp`,
`opfs_div_int_mediump` (the 67-word `nir_lower_idiv` sequence through
`Fs::idiv` as it was), `opfs_cmp_ivec3`, `opfs_equal_bvec2`, `opfs_logical_xor`
and `opfs_not`. The thirteen step-2 references stay refused with the same
text and ignored; `rsg_cond_fs` is refused by its vertex shader's literals
(`ConstBool`, step 2 item 1), not by its fragment shader.

The references were drawn without `--uniform-values` (gl_ref.py cannot set a
`bool` uniform), so their FAU holds 0 in every uniform word; `tests/mali_ref`
got `Reference::load_unset` / `assert_matches_unset` for that (the uniform
words compared as 0), beside the existing compare for references drawn with
the option. `tools/mali/testdata/cases.txt` lists ten of the twelve as
pan_shc-against-shim cases (every mediump fragment program and the passthrough
vertex stage's mediump varyings `differs:mediump`, as the gm2_* rows);
`reslib_mixed_fs` and `opfs_logical_xor` are left out: pan_shc's vertex
position program has the reference's words but other facts, a result the
list has no form for (as gm2_fsmisc_2).

### What was lowered, and where (all in `src/shaderc/online/mali/`)

The fragment walk (`nir_fs.rs`, `Fs::int_lane` and its callers) takes, as
32-bit values of a mediump shader:

- `f2i32` of a varying's lane as loaded (`F32_TO_S32.rtz` of the fp32 word:
  `f2i32(f2fmp(a))` is `f2i32(a)`, nir_opt_algebraic.py:2002); `iadd`,
  `imul`, `isub` (the front end's `iadd(a, ineg(b))`) and `ineg` of
  non-masks; `b2f` of a 32-bit boolean (the colour's `MUX.v2i16` at the
  store, as the `bcsel(b, 1.0, 0.0)` arm); `b2i32` of a 16-bit mask (the
  `MUX.i32 0, 1, b` reading the mask replicated into a word, `IADD.v2u16
  r0, r0^.h00, 0x0`: `bi_lower_swizzle`); `f2b` of a mediump lane or an
  fp16 value as `FCMP_OR.v2f16.ne.m1` against 0; `ieq`/`ine` of two 16-bit
  masks, or of a mask and a bool uniform's word read through its low half
  (`ICMP_OR.v2u16.eq.m1 a.h00, u.w0.h00`), as a 16-bit node; `ixor` of
  masks at 16 bits (`LSHIFT_XOR.v2i16`, a new row of `emit.rs`) and at 32.
  `Fs::mask16` counts the 16-bit `ieq`/`ine`/`ixor` as masks.
- An `ivec2`/`ivec3`/`ivec4` uniform's lanes as 32-bit words, in the walk's
  load and in the fp16 block's layout (`select.rs` `UboLayout::fs16`,
  beside `GL_BOOL_VEC*`): `caps->int16` is false, no integer narrows.
- `kept_whole` (the link's whole-varying rule): a scalar `int(v.z)` leaves
  the lanes read (opfs_add's `.v2` load of two varyings' lanes), a vector
  `ivec3(v.zxy)` keeps the varying whole (opfs_cmp_ivec3's two `.v4`
  loads). Measured; which link pass keeps the vector's `f2fmp` alive is not
  read from Mesa.
- An integer's float (`i2f32`, `u2f32`) stored to a declared mediump
  variable is narrowed there (`Fs::narrow_stored`), the `f2f16` right after
  the conversion in the order (GLSL's `f2fmp(i2f32(a))` is one `i2fmp`,
  nir_opt_algebraic.py:1322, lowered in place: opfs_mul's `S32_TO_F32 r1`
  then `FADD.f32 r1.h0` lane by lane). `Fs::narrowed` counts such a lane,
  so `color * 0.10 + 0.50` on it goes through `Fs::alu16` as a highp
  shader's fp16 island does: `Fs::vec16_of` now takes a lane and a constant
  half as a pair (`vec2(x, 0.0)` is `U16_TO_U32`, `vec2(0.0, x)` and
  `vec2(x, 1.0)` `MKVEC.v2i16`), and `alu16`'s `fmul`/`fadd` of pairs folds
  a pair of constants at fp16 (`fold16`: opfs_add's lanes z and w, 0x3800
  and 0x38cd) and lets a vector's merely coinciding lanes stay the vector.

Two new routes to `select_highp` (`mod.rs`, after `mask_colour`):

- `bool_colour`: every colour lane a constant or the `b2f` of a boolean,
  the graph integer and fp16 operations over inputs, uniforms and
  constants. `convert_inputs16` first makes `nir_lower_mediump_io`'s
  conversions explicit, right after each load in the order: an aligned pair
  both read is one two-lane `f2f16` (`V2F32_TO_V2F16`), a lane read alone
  its own (`FADD.f32 .h0`); the fp16 compares then pair by `vectorize16`'s
  lane keys (equal_bvec2: `v_in1.xy` paired and read `.h10`, `v_in0`'s y
  and w alone).
- `int_colour16`: every colour lane a constant or a lane of a two-lane fp16
  operation over narrowed integers' floats and constants, no fp16 operation
  on a varying's lane (the pair graph's shape stays `select`'s).

`select_highp` itself: on a colour of booleans a 16-bit compare reading a
32-bit word reads its low half (`make_sources_canonical`'s `i2i16`); a
32-bit `b2i32` of a 16-bit mask gets the replicate first; `b2i32` and, on
`int_colour16`, a `vec2` are copies that `nir_opt_move` moves to their
first user (`key_of` follows a moved user to its new place); the pressure
scheduler then reorders nothing in these programs.

`opt.rs`: `va_fuse_cmp` for `LSHIFT_AND.i32` of two compares (the OR form
first, the swapped try is the OR's) into the new `ICMP_AND.s32`/`.u32`
(`bir.rs`, `pack.rs`; reslib_mixed's `ICMP_AND.u32.eq.m1 r1, r2^, u3.w0,
r1^`), and `bi_optimizer_result_type` for a `MUX.v2i16` between 0 and the
fp16 1.0 on an `FCMP.v2f16` (`.f1`, the mask's swizzle composed into the
compare's sources: opfs_not's `FCMP_OR.v2f16.ge.f1 r0, 0x0, r0^.h00, 0x0`).

### The order of compares: measured on Mesa's per-pass NIR

`tools/mali/drm_shim_ref.sh run` with `NIR_DEBUG=print_fs` in the
environment prints the NIR after each pass to the `.stdout`. For
`opfs_logical_xor`, `opfs_equal_bvec2` and `reslib_bvec_fs` (2026-11-17,
Mesa 26.2.1 built by `drm_shim_ref.sh build` into `$CONNECTED_DEPS_DIR/mesa-shim`):

- `pan_nir_lower_bool_to_bitsize` (bifrost_nir.c:350) runs before
  `nir_opt_sink`/`nir_opt_move` (:380-381), and `nir_alu_instr_is_comparison`
  takes the `_pan` compares (nir.c:3286-3289, `CASE_ALL_SIZES` lists
  `op##_pan`): the compares are moved to their first user, as
  `nir_fs.rs`'s `key_of` had it.
- `nir_opt_move` runs twice on a fragment shader: at the link
  (bifrost_nir.c:889-890) and at the compile (:380-381). Two compares that
  share a first user swap at each run (nir_opt_move.c:91-92: the one already
  in place keeps its index and the earlier lands between it and the user),
  so the final order is the program order (xor: the compare of in0, then
  in1, then `LSHIFT_XOR`). `nir.rs` `move_compares` keeps the pass's rule
  and `select_highp` calls it twice.
- `all(a == b)` / `any(a != b)` is one GLSL IR operation (`ball_iequal`),
  which `lower_reduction` (nir_lower_alu_width.c:96-126) scalarises from the
  last lane with each lane's compare right before the merge that reads it
  (`ieq.z`, `ieq.y`, `iand`, `ieq.x`, `iand`); the walk's Bany/Ball arm puts
  the front end's lane-order compares in that order (`reduction_order`).
  `all(lessThanEqual(..))` is a separate vector compare in lane order
  (reslib_float_fs, unchanged).
- `b2i32` and `vec` are copies to the pass (nir_opt_sink.c:72,
  `nir_move_copies`): reslib_mixed's `CSEL.u32.eq` and its `FCMP` follow
  the `ieq` that reads them, past the `ICMP_AND`.

### What moved with it

- `tests/mali_deqp_unitype.rs`: `fsivec4` is refused by the walk's integer
  ops now (`an integer or boolean value`), no longer by the fp16 block's
  layout; its pin moved. Its `vec4(fTest).xy` beside a texel is no step-1
  shape (lower-unitype).
- `src/shaderc/online/mali/tests.rs`: the IntValue probe of
  `every_reachable_gap_is_reached_by_an_es100_source` is `i != j` of two
  run-time ints (`vec4(float(int(v.x)))` compiles now); `ALL_OPS` and
  `mesa_opcode` list the two `ICMP_AND` ops; `fs_reference_sources` draws
  the new rows from `deqp_intval_opfs_logical_xor` and `reslib_mixed_fs`.
- `tools/mali/corpus-verdicts.txt`: no verdict moved (`corpus-verdicts.sh
  --check`: 254 ok to 254 ok). Section (d)'s `tri_int.frag`, `cond_complex.frag`,
  `loop_full.frag`, `tri_discard.frag` and `varyings15.frag` still refuse
  with this text: each meets a shape step 1 did not take first (`i != 2`
  of a run-time int in tri_int, `&&` of a constant `false` in cond_complex,
  the discard's `&&` of two masks, a loop, a run-time index).
- `tests/fixtures/mali_glmark2_verdicts.txt`: no glmark2 verdict moves with
  this change (the tree at ea40c2b7 and this one give the same 69 verdicts).
  The file was regenerated (`MALI_GLMARK2_WRITE=1`) all the same: at
  ea40c2b7 it was stale, 24 rows behind the merged lowerings of plan #367
  (buffer fs 9fe6 and terrain fs d697 to `ok`, the other 20 to a later
  refusal), and the test fails against a corpus until it is.

### Left to step 2 and later

- The vertex walk (section 4, step 2) untouched: all step-2 references still
  refuse at `nir.rs`'s four sites.
- `Fs::mask_op`'s mix of a 16-bit and a 32-bit boolean still refuses: no
  reference of this family draws it (reslib_mixed's `iand`s are all 32-bit).
  `imin`/`imax`, `inot` of a non-mask and `ine` of two run-time ints are not
  added either (no reference); `ine` of two ints is the gap probe.
- `Fs::alu16` refuses the shapes it does not take with `FragCoordMediump`'s
  text, which names gl_FragCoord; a narrowed integer's float that meets it
  (a product by a varying's lane, no pair) gets that text too. Split the
  text when such a shape is lowered.
- `Fs::narrowed` takes only `i2f32`/`u2f32` as the integer's float, not
  fp32 arithmetic on one: an early try with every fp32 lane caught the
  mediump gl_FragCoord's narrowed lanes (`fadd(u2f32(px), 0.5)`) and sent
  prec_84, deriv and tri_fragcoord through `alu16`.
