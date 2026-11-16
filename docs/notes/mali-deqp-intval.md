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
