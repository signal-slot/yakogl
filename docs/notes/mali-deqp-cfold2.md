<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the `ConstantFold` refusals left at the tip, their shapes and Mesa 26.2.1 references

**Status: study** (plan #387, item `cfold-ref`; issues #20, #7). Nothing
under `src/` changed. The references are `tools/mali/testdata/deqp_cfold2_*`,
pinned by `tests/mali_deqp_cfold2.rs`. Measured at the integration tip
**`5cb806246171`** (2026-11-21; the host measure's `commit` reads
`5cb80624`). Mesa paths are relative to `src/` of the 26.2.1 tree.

The refusal is `MaliError::ConstantFold` (`src/shaderc/online/mali/mod.rs:450`):
"a constant operation on or to a value that is not a finite normal float".
`docs/notes/mali-deqp-families.md` section 18.2 row 9 counts it as the first
refusal of 204 must-pass cases at `9ac31f30`. The constop step `0fd576ea`
(plan #377) lowered the shapes of `docs/notes/mali-deqp-constop.md` sections
2 and 8. The `deqp_constop_*` references predate it and do not cover what is
left; this note and its references do.

## 1. How this was measured

- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" --jobs 3
  --chunk 100` (the command of section 18 of the families note). VK-GL-CTS is
  at `93bca018` with `cts-yakogl-target.patch`, fetched and patched the way
  `tools/cts/deps.toml` documents. The whole list of the slices
  `2026-11-15-t6-gles2-S{1..4}-37305608` ran: 10,597 cases, all began, 106
  processes exited 1 and 2 exited 139 (the two `scalar_conversion.*.96`
  crashes of section 18.4). 6,132 cases carry a host refusal. HOST-ONLY: these
  are compile and link outcomes only.
- **The cases:** 204 cases have the `ConstantFold` text as their first
  refusal. This is the same total and the same split by the script's shapes
  as section 18.3 row 3.6 at `9ac31f30`. Board status (`37305608`): 167 Fail,
  28 NotSupported, 9 Pass (the 9 `shader_api.shader_source.split_source_*`,
  which check no draw).
- **Refusal sites:** a scratch probe outside the tree (a detached worktree
  whose `nir.rs` and `nir_fs.rs` print `file!():line!()`, the op and the
  operand bits at every `MaliError::ConstantFold` construction; line numbers
  unchanged) compiled each case's refusing program through `tests/mali_ref`'s
  `compile_pair`. Every one of the 204 refused at exactly one site, so each
  case is counted once under the site and op that refused it.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_cfold2_<name> --instrument shim`, on the drm-shim
  that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was byte-equal. The
  dEQP pairs are verbatim from the host measure's `.qpa` (the refusing
  program of the case). As with `deqp_loops2_*`, the provenance is in the
  `.license` file beside each shader.

## 2. Counts at the tip, by the script's shape

The script's shapes (`deqp-host-measure.sh`'s `constop` table) are regexes on
the source. `zero_operand`, for example, is "the source has a `0.0`". The
construct column is what the probe found the compiler refusing.

| Script shape | Cases | Fail / NS / Pass (board) | VS / FS | Constructs (cases) | dEQP groups |
|---|--:|---|---|---|---|
| `zero_operand` | **98** | 69 / 20 / 9 | 92 / 6 | int compare of constants 30, `inot` of a constant 52, `rcp(0.0)` 10, mediump `rcp` not exact in fp16 6 | shaders.indexing 66 (`tmp_array` 24, `vector_subscript` 27, `matrix_subscript` 15), light_amount 16, shader_api.shader_source 9, shaders.matrix.div 6, state_query.shader 1 |
| `random_mixed` | **60** | 60 / 0 / 0 | 58 / 2 | int compare 22, `inot` 9 (7 of 0, 2 of ~0), `f2i` 4, `rcp(0.0)` 11, `rsq(0.0)` 12, mediump `rcp` 2 | shaders.random 52 (`conditionals` 18, `trigonometric` 10, `exponential` 9, `all_features` 6, `texture` 5, `comparison_ops` 4), fragment_ops.interaction.basic_shader 8 |
| `const_other` | **23** | 15 / 8 / 0 | 20 / 3 | `inot` of a constant 17, int compare 1, `ior` 1, `iabs` 1, mediump `rcp` 3 | shaders.indexing.varying_array 16, shaders.constant_expressions.operators 3, shaders.matrix.div 3, shaders.struct.uniform 1 |
| `const_call_transcendental` | **16** | 16 / 0 / 0 | 0 / 16 | `log2` of a constant 12 (`log2`, `log`, `pow`, 4 each), `iand` of `atan`'s sign 4 | shaders.constant_expressions.builtin_functions.{exponential, trigonometry} |
| `const_call_other` | **7** | 7 / 0 / 0 | 7 / 0 | int compare 3, `inot` of ~0 3, `iabs` 1 | shaders.constant_expressions.builtin_functions.vector_relational 6, .operators.math_ivec 1 |
| **Total** | **204** | 167 / 28 / 9 | 177 / 27 | | |

By mechanism (the lowering's unit, section 4) the same 204 cases are:

| Mechanism | Site (tip) | Op, operand bits seen | Cases |
|---|---|---|--:|
| **A. Integer and boolean operations on constants in the walk** | `nir.rs:2166` (an operand that is not `plain`: an int below 2^23 or `~0` reads as a denormal or a NaN), `nir.rs:2253` (the arm "the front end folds a constant expression") | `Icmp(Eq)` 56 (`[0, 2]`, `[0, 8]`, `[fffffffc, 1]`, ...), `Inot` 81 (`[0]` 76, `[ffffffff]` 5), `F2i` 4 (`[45800000]`), `Iand` 4 (`[40000000, 80000000]`), `Iabs` 2 (`[9]`), `Ior` 1 (`[ffffffff, 0]`) | **148** |
| **B. A transcendental built-in on a constant** | `nir.rs:2253` | `Flog2` 12 (`[42c80000]` = 100.0, `[3fd9999a]` = 1.7, ...) | **12** |
| **C. `rcp` or `rsq` of 0.0, an infinity the next operation absorbs** | `nir.rs:2274` (a folded result that is not `plain`), `nir.rs:2214` (`fold_float(Frsq)` declines) | `Frcp [0] -> 7f800000` 21, `Frsq [0]` 12 | **33** |
| **D. A mediump `rcp` of a constant not exact in fp16** | `nir_fs.rs:6104` | `1/0.7` (`3f333333 -> 3fb6db6e`), `1/1.2` (`3f99999a -> 3f555555`), `1/-3.0`, `1/0.0` | **11** |
| | | | **204** |

By site: `nir.rs:2166` 64, `nir.rs:2253` 96, `nir.rs:2274` 21,
`nir.rs:2214` 12, `nir_fs.rs:6104` 11. No case refuses at the other sites
(`nir.rs:2171`, `:2806`, `:2811`; `nir_fs.rs:1051`, `:1053`, `:1429`,
`:1481`, `:5964`, `:7160`), and `glsl.rs` raises none. The families note's
"Raised at" cell (`nir.rs:2166, :2171, :2274, :2806`) is corrected by this
table.

The partner shader of 182 of the 204 compiles at the tip (the measure's
empty `next_family`). The other 22 refuse in the partner too: `const_other`
14 (`varying_array`: `loops` 4, `367:ARITH` 4, `mixprec` 4,
`367:FRAGCOLOR_LANE` 2), `random_mixed` 7 (`367:ARITH` 5, `367:CONTROL_FLOW`
1, `constop` 1), and `state_query.shader.uniform_value_sampler` 1 (a highp
sampler).

## 3. The references, and what Mesa does with each

All references are `tools/mali/testdata/deqp_cfold2_<name>.{vert,frag,expect.json}`,
with `.license` files. One per construct and script shape, 19 in all.

| Reference | dEQP case | Mech. | What Mesa 26.2.1 does (from the reference) |
|---|---|---|---|
| `zero_operand_icmp` | `shaders.indexing.tmp_array.vec4_static_write_static_loop_read_vertex` | A | `nir_opt_loop_unroll` unrolls the static loop, and `nir_opt_constant_folding` folds the counter's `ieq`/`ilt`/`inot` (`compiler/nir/nir_opt_constant_folding.c`). No compare is left: the varying region is 30 words of products and sums. |
| `zero_operand_inot` | `shaders.indexing.vector_subscript.vec2_static_loop_subscript_write_direct_read_vertex` | A | The same; the varying region is 16 words. |
| `random_mixed_icmp` | `shaders.random.conditionals.vertex.45` | A | An `if` on an int vector compare with constant lanes (`ivec3(-5, h, -8.0) - ivec3(g, -6.0, f) == ivec3(e, -2, c)`): the constant lanes' `ieq` fold (varying 15 words). |
| `random_mixed_inot` | `shaders.random.conditionals.vertex.2` | A | An `inot` of a constant 0 (false) in the random conditionals; it folds (varying 21 words). |
| `random_mixed_f2i` | `shaders.random.texture.vertex.29` | A | `int(4096.0)` and the like fold (`f2i32`; varying 22 words). |
| `const_other_inot` | `shaders.indexing.varying_array.vec4_static_loop_write_static_read` | A | The unrolled write loop of a varying array (varying 83 words, FS 22 words). |
| `const_other_ior` | `shaders.constant_expressions.operators.logical_vertex` | A | `true \|\| false` is folded by the front end. The ShaderLibrary FS (10 words) compares against a uniform, and FAU `ffffffff` is the folded `true` of the VS's output. |
| `const_other_iabs` | `shaders.constant_expressions.operators.math_int_vertex` | A | `abs(9)` and the int arithmetic fold; FS FAU `00000010` (16), `40f00000` (7.5). |
| `const_call_other_bvec` | `shaders.constant_expressions.builtin_functions.vector_relational.equal_bvec2_vertex` | A | `equal(bvec2, bvec2)` of constants is folded by the front end (`compiler/glsl/ast_function.cpp:657-664`, `constant_expression_value`). |
| `const_call_other_ivec` | `...vector_relational.equal_ivec2_vertex` | A | The same for `ivec2`. |
| `const_call_transcendental_atan` | `...trigonometry.atan_combined_float_fragment` | A (`iand`) | `atan(2.0)` is folded by the front end (`ast_function.cpp:657`) to `0x3f8db70d`, the correctly rounded `atanf(2.0f)`. No `iand` survives. Folding the expansion op by op must give this word; the pin decides. |
| `const_call_transcendental_log2` | `...exponential.log2_float_fragment` | B | `log2(100.0)` is folded by the front end with `log2f` (`compiler/glsl/ir_expression_operation.py:458`) to `0x40d49a78`, in FAU beside the compare's `0.05` (`3d4ccccd`). |
| `const_call_transcendental_pow` | `...exponential.pow_float_fragment` | B | `pow(1.7, 3.5)` is folded with `powf` on the fp32 inputs (`ir_expression_operation.py:725`) to `0x40ccfc0f`. That is `pow` of `1.7f` and `3.5f`, correctly rounded; the double `pow(1.7, 3.5)` rounds to `0x40ccfc0e`. The `log` cases fold with `logf` the same way (`log(42.12)` gives `0x406f64b9`). |
| `zero_operand_rcp_inf` | `shader_api.shader_source.split_source_2_null_terminated_fragment` (VS `gl_Position = vec4(0.0);`) | C | `nir_lower_viewport_transform` (`compiler/nir/nir_lower_viewport_transform.c:83-88`) takes `frcp(w)`, which is +inf for `w = 0.0`. It clamps it to ±`MAX_W_RECIP_ABS` = 2^15 (`:47`), and the folds give the stored position: x, y = 32.0 (`42000000`, the viewport offset; `0 * 32768 * scale + offset`), z = 0.5, w = 32768.0 (`IADD_IMM ... #0x47000000`). The infinity never reaches an output. |
| `random_mixed_rcp_inf` | `shaders.random.trigonometric.vertex.10` | C | The same `frcp` of a folded 0.0 position lane; varying 69 words. |
| `random_mixed_rsq_inf` | `shaders.random.all_features.vertex.63` | C | `inversesqrt(0.0)` folds to +inf (`ir_expression_operation.py:453`, `1.0F / sqrtf`), and what reads it folds to finite constants: no varying region, and the FS (7 words) is a constant colour. |
| `zero_operand_rcp16` | `shaders.matrix.div.const_mediump_mat3_mat3_fragment` | D | A constant colour, FAU `b638bcab 3c003db6` (fp16 lanes). |
| `const_other_rcp16` | `shaders.matrix.div.const_mediump_mat2_mat2_fragment` | D | A constant colour, fp16 lanes `bccd 4100 37fe 3c00`. The third lane is `res[0][1] + res[1][1]` = `0.7/0.7 + 0.4/-0.8`, exactly 0.5 in fp32. Mesa's `0x37fe` comes out only from fp16 `fmul(a, frcp(b))`, each op rounded to nearest even: a fold in fp32, and an fp16 divide, both give `0x3800`. So Mesa lowers `fdiv` to `fmul(a, frcp(b))` and folds it at fp16 after `lower_precision`, keeping the inexact reciprocal. |
| `random_mixed_rcp16` | `shaders.random.texture.fragment.40` | D | `texture2DProj(k, vec3(c, m, b), l)` divides by the constant `b = -3` (`rcp(-3.0)`, not exact in fp16). Mesa drops that look-up, whose `d` is overwritten, and keeps the last one at the constant coordinate `vec2(c, b) * vec2(b, b)` = (-3.0, 9.0): FAU `c0400000`, `41100000`; FS 9 words. |

`light_amount.*` (16 cases: int compare 14, `inot` 2) has no reference.
`gl_ref.py --uniform-values` does not set its `uniform Light u_lights[N]`
struct array ("does not set a Light uniform"), so `deqp_ref.sh` stops. Its
construct is the unrolled counter of `zero_operand_icmp`/`_inot`.

### 3.1 `light_amount.*` (intval5-ref)

Drawn by item `intval5-ref` (the "has no reference" statement above is
superseded): `deqp_intval5_light_amount_2_2` and `deqp_intval5_light_amount_0_16`,
`tools/shaders/es100/light_amount_{2_2,0_16}.vert` with `light_amount.frag`,
Mesa 26.2.1 G610 shim, pinned by `tests/mali_deqp_intval5.rs`. `gl_ref.py`
sets the struct array now (`README-deqp_ref.md`, "Struct uniforms"), so the old
"does not set a Light uniform" stop is gone; both shapes (2 directional + 2 omni,
0 + 16) draw. The unrolled counter's compare and `inot` are folded away by Mesa:
the fragment program is the same 8 words for both (`LD_VAR_BUF_IMM.f32.slot0.v3`,
`V2F32_TO_V2F16 r0, r0^, r1^`, `FADD.f32 r1.h0, r2^, 0x0.neg`, `MKVEC.v2i16 r1,
r1^.h0, u2.w0.h0` for the constant alpha, ATEST/BLEND), and the vertex shader's
position region has 40 words in both (the varying region 98 words for 2 + 2 and
455 for 0 + 16). No integer compare (`ICMP`, `CSEL`) survives into either vertex region,
so the 16 cases need the constant fold of the unrolled counter (step A), not an
integer lowering. Both refuse at the tip with the "constant operation on or to a
value that is not a finite normal float" text. Caveat: a plain array of struct
members' uniform values are not what the dEQP case sets (all are the
`--uniform-values` sequence), so only the words, not the FAU values, are the
case's.

## 4. Lowering order (one-session steps, reusing constop's folds)

Each step lists its cases (an upper bound: a case can need two steps, and
section 2 names the partner refusals) and the reference to un-ignore first.

1. **A, integer and boolean constants in the vertex walk: 148 cases**
   (`zero_operand` 82, `random_mixed` 35, `const_other` 20,
   `const_call_other` 7, `atan` 4). In `Nir::alu`'s all-constant branch
   (`nir.rs:2140-2280`), send the integer and boolean ops to the existing
   `nir::fold_int` (`nir.rs:865`, already used by the fragment walk at
   `nir_fs.rs:5964`). This must happen **before** the `plain` test at
   `nir.rs:2165`, which is a float test and rejects int bit patterns. Map
   `Alu::Icmp(c)` to `AluOp::Ilt/Ile/Ieq/Ine` (swapping operands for `>`),
   `Inot`, `Iand`, `Ior`, `Ixor`, `Iadd`, `F2i`, `I2f`, `B2i` to their
   `AluOp`. `Iabs` (2 cases) is the one op `fold_int` lacks: add it there,
   not a second copy. This step reuses fold code and adds none. First
   un-ignore: `const_call_other_ivec` and `const_other_ior` (FS 10 words,
   VS position only), then `zero_operand_icmp`. Of the 148, the 16
   `varying_array` and the 7 random cases with a partner refusal (section 2)
   then wait on other texts. The 4 `atan` cases also need their folded
   value to equal `0x3f8db70d`.
2. **C, an infinity that the next operation absorbs: 33 cases** (`rcp(0.0)`
   21, `rsq(0.0)` 12; 9 of them board Pass already). At `nir.rs:2274` and
   `nir.rs:2214`, give `Frcp`/`Frsq` of ±0.0 the IEEE ±inf, as
   `nir_eval_const_opcode` does with the GL path's float controls of 0
   (`mali-deqp-constop.md` section 4). Then refuse only a non-finite value
   that reaches an output or an op with no fold. The clamp of the viewport
   transform (±2^15) is the fold that absorbs it in `zero_operand_rcp_inf`.
   Check that the walk's clamp folds `Fclamp` of ±2^15 (`nir.rs:2186`
   handles `ZERO_INF`, `M1_1` and `ZERO_ONE`; `_ => f(0)` passes the value
   through unclamped, which would give inf instead of 32768.0). First
   un-ignore: `zero_operand_rcp_inf` (VS position only).
3. **D, a mediump `rcp` of a constant: 11 cases.** At `nir_fs.rs:6104`,
   round `1/c` of the fp16 operand to nearest even (`f16_rtne`, as the
   constant fold at `nir_fs.rs:1051-1053` does) instead of refusing an
   inexact one, and let the product that uses it round again. The bits of
   `const_other_rcp16` (section 3) are the check. First un-ignore:
   `const_other_rcp16`.
4. **B, `log2`/`log`/`pow` on constants: 12 cases**, plus the value check
   of `atan`. Mesa folds these in the GLSL front end with libm at fp32
   (`log2f`, `logf`, `powf`, `atanf`; correctly rounded for these inputs).
   They belong with `negzero-lower`'s front-end folding of built-ins on
   constants (`mali-deqp-negzero.md` section 4, steps 1 and 2), not in
   `nir.rs`: fold the call in `glsl.rs` before the expansion to
   `Flog2 * k`. A no_std fold needs a correctly rounded `log2f`/`powf`;
   pin each value against these references (`0x40d49a78`, `0x40ccfc0f`,
   `0x406f64b9`, `0x3f8db70d`). First un-ignore:
   `const_call_transcendental_log2`.

Steps 1 to 3 together take 192 of the 204 below the 100-case line on their
own. Step 1 alone does it: 204 - 148 = 56.

## 5. The test

`tests/mali_deqp_cfold2.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_cfold2`.

- 19 reference tests, all `#[ignore]` with the `ConstantFold` refusal quoted.
- `refusals_pinned` runs. It asserts each one's exact current refusal
  (`mali_ref::refusal_change`). It also checks that every
  `deqp_cfold2_*.expect.json` on disk is in `REFUSALS` or `MATCHED`;
  `MATCHED` is empty at this tip.
- At `5cb80624`: 1 passed, 19 ignored. With `--ignored`, each of the 19
  fails only because the pair is refused.

## 6. Issue comment text (for #20; agents do not post it)

> ConstantFold (`MaliError::ConstantFold`, "a constant operation on or to a
> value that is not a finite normal float"): at `5cb806246171`, the host
> measure (`deqp-host-measure.sh`, all 10,597 board-refused cases) finds
> 204 cases with this first refusal (167 Fail, 28 NS, 9 Pass on the board),
> the same as the `9ac31f30` rank. A probe of the refusal sites puts every
> case at one site: `nir.rs:2166` 64, `nir.rs:2253` 96, `nir.rs:2274` 21,
> `nir.rs:2214` 12, `nir_fs.rs:6104` 11. Four mechanisms:
> (A) integer/boolean ops on constants in the vertex walk, 148 (unrolled
> loop counters in shaders.indexing and light_amount, random conditionals,
> constant_expressions int/bool); (B) `log2`/`log`/`pow` of a constant, 12;
> (C) `rcp`/`rsq` of 0.0, whose infinity Mesa's viewport clamp (±2^15) or
> fold absorbs, 33; (D) a mediump `rcp` of a constant not exact in fp16,
> 11 (Mesa folds `fmul(a, frcp(b))` in fp16 with each op rounded).
> 19 Mesa 26.2.1 references are pinned in
> `tools/mali/testdata/deqp_cfold2_*` and `tests/mali_deqp_cfold2.rs`.
> Lowering order: A by routing the walk's constant int ops to the existing
> `nir::fold_int` (148 cases, this alone takes the text under 100), then C,
> D, and B with negzero's front-end built-in folding. Details:
> `docs/notes/mali-deqp-cfold2.md`.
