<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): one constant-fold helper for built-ins that gives Mesa's bits (CF1, N1/N2, K4)

**Status: design** (plan #387, item `libm-fold-design`; issue #7). Nothing
under `src/`, `tests/` or `tools/` changed, no reference was redrawn. The
measurements below come from scratch programs outside the tree (a Python
script that calls the host's libm through `ctypes` and checks it against an
80-digit `decimal` evaluation, and a Rust program that includes
`src/shaderc/online/fmath.rs` by path and calls `std`); how to repeat them is
in section 7. Host: glibc 2.43, rustc 1.99.1, x86-64. Mesa paths are relative
to `src/` of the 26.2.1 tree (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`; the
vendored `third_party/mesa-26.2.1` has no `compiler/`).

`mali-lowering-schedule.md` section 4a decides that the built-in folds of
`cfold` (CF1, `mali-deqp-cfold2.md` step B), `negzero-lower` (N1, N2,
`mali-deqp-negzero.md` section 4) and `constprod` S4
(`mali-constprod-lowering.md`) share **one** helper, because Mesa folds with
the host's C library and two implementations would differ in bits. This note
says what Mesa computes, what the references hold, and what the helper is.

## 1. How Mesa 26.2.1 folds a built-in on constants

Two folders see a constant, in this order. A GLSL built-in is folded by the
first one that gets all its operands as constants; only what is left reaches
the second.

### 1.1 The GLSL IR folder (front end): `compiler/glsl/ir_constant_expression.cpp`

`ir_expression::constant_expression_value` evaluates the expression with the C
expression of `compiler/glsl/ir_expression_operation.py`, in the type of the
operands (f32 for `float`). It runs when `const` initialisers are evaluated
(`ast_to_hir`), when a call is inlined and by `do_constant_folding` in the
linker's optimisation loop, so everything dEQP writes as `const float cval =
sin(float(3.0))` is folded here, before NIR exists. A 16-bit operand is
widened to f32 first (`:714-724`); there is no flush of denormals here.

| GLSL built-in | IR op and its C expression (`ir_expression_operation.py`) | note |
|---|---|---|
| `sin`, `cos` | `sinf(x)`, `cosf(x)` (`:568-569`) | glibc's binary32 functions, **not** correctly rounded |
| `tan` | the built-in's body is `sin(x) / cos(x)` (`builtin_functions.cpp:6512-6518`): `sinf(x) / cosf(x)`, a true f32 division | two roundings; **not** `tanf` |
| `asin`, `acos` | `asinf(x)`, `acosf(x)` (`:571-572`) | |
| `atan(x)` | `atan(x)` (**double**, `:570`), stored to the f32 | f64 value rounded once |
| `atan(y, x)` | `atan2(y, x)` (**double**, `:752`) | f64 value rounded once |
| `exp`, `log` | `expf(x)`, `logf(x)` (`:455-456`) | |
| `exp2`, `log2` | `exp2f(x)`, `log2f(x)` (`:457-458`) | |
| `pow` | `powf(x, y)` (`:725`) | |
| `sqrt` | `sqrtf(x)` (`:454`) | IEEE exact |
| `inversesqrt` | `1.0F / sqrtf(x)` (`:453`) | two roundings; inf for 0 |
| `floor`, `ceil`, `trunc`, `fract`, `sign`, `abs` | `floorf`, `ceilf`, `truncf`, `x - floorf(x)`, `float((x>0)-(x<0))`, `fabsf` (`:450-451`, `:561-564`) | exact |
| `mod` | `x - y * floorf(x / y)`, unfused, a true division (`:689`) | `deqp_constop_ce_mod`: `0x40233340`; the fused value is `0x40233335` (`mali-deqp-constop.md` section 4) |
| `reflect(I,N)` | the body `I - 2 * (dot(N,I) * N)` (`builtin_functions.cpp:7224`): f32 `mul`, `sub` (`add`/`mul` are `a + b`, `a * b`, unfused) | |
| `a / b` | `a / b` (f32); `rcp` is `1.0F / x` (`:452`) | |

The GLSL IR folder does not know `ffma`; a constant `ffma` that our IR emits
for `mod`, `smoothstep` or `dot` must still fold as `a * b + c` with two
roundings (the existing rule of `nir.rs` `alu_sized`).

### 1.2 The NIR folder: `compiler/nir/nir_opt_constant_folding.c`, `nir_constant_expressions.py`

What survives the GLSL IR folder (a built-in on an operand that became
constant only after uniform or varying propagation, after `lower_precision`
or inside NIR's own lowerings) is folded by `nir_eval_const_opcode`, with
`execution_mode = shader->info.float_controls_execution_mode` (0 for GL: no
denormal flush, round to nearest even).

| NIR op | const expression (`nir_opcodes.py`) | note |
|---|---|---|
| `fsin`, `fcos` | `sinf`, `cosf` (`:448-449`, f32; f64 only for a 64-bit source) | the same glibc routines as 1.1 |
| `fexp2`, `flog2` | `exp2f`, `log2f` (`:269-270`) | |
| `fpow` | `powf` (`:1049`) | |
| `fsqrt`, `frsq`, `frcp` | `sqrtf`, `1.0f / sqrtf`, `1.0f / x` (`:266-268`) | exact / two roundings |
| `ffma`, `ffma_weak` | **`fmaf`** (`:1150-1190`), fused | the NIR atan/asin/acos polynomials (1.4) are `ffma_weak` |
| `fmul`, `fadd`, `fsat`, ... | plain f32 C arithmetic | |
| `fsign` | `isnan ? 0 : ...` (`:242`) | |

`exp` and `log` do not exist in NIR. `glsl_to_nir.cpp:2177-2178` expands a
GLSL IR `exp`/`log` that was not folded to `exp2f(x * float(M_LOG2E))` and
`log2f(x) * float(1/M_LOG2E)`; **the two paths give different bits**
(section 3.3): `exp(5.5)` is `0x4374b122` through `expf` and `0x4374b121`
through `exp2f(x * log2e)`. A constant that the GLSL IR folder saw gets the
`expf` bits.

### 1.3 Sized and 16-bit operands

For a `float16` NIR op the source is widened to float, the **f32** libm
function is evaluated, and the result is rounded **once to f16 by
`_mesa_float_to_float16_rtne`** (`nir_constant_expressions.py:551-556`; RTZ
only when the execution mode asks for it, not in GL). It is not the correctly
rounded f16 of the exact result, and it is a second rounding after the f32
one. An f16 chain such as `a / b` on mediump (`fmul(a, frcp(b))`) rounds after
every op, f32 arithmetic in between: `deqp_cfold2_const_other_rcp16` is the
proof (section 3.2).

### 1.4 Built-ins NIR lowers to polynomials: no libm, but a fold all the same

`asin`, `acos`, `atan`, `atan2` that reach NIR with a constant operand but a
non-constant sibling lane (a `vec4` with one constant lane) are expanded by
`nir_builtin_builder.c` (`build_asin`, `nir_atan`, `nir_atan2`) into `ffma_weak`
polynomials, and **NIR folds the polynomial** lane by lane: no libm, `fmaf`
steps. The coefficients become FAU words of the runtime sequence (for example
`bc46c6a5`, `3d5be101`, `bdf0555d`, `3e468bc1`, `beaa5476`, `3f7ffea5`,
`bfc90fdb` in `deqp_negzero_mainbuiltin_vs`, `deqp_constprod_fma_trig_*`:
`-0.0121323213173444f` ... `0.9999793128310355f`, `-M_PI_2`). A lane at
`atan(1.0)` gives `0x3f490fa4` there, not `0x3f490fdb` (section 3.4). This is
the same helper's business only in that the fold must be `fmaf`, once the
expansion exists; the expansion itself is the arith/fclane design's.

### 1.5 NaN, infinity, denormals

Neither folder flushes (GL: float controls 0) and neither checks for NaN or
infinity; the C result is stored. `rcp(0.0)` and `inversesqrt(0.0)` are +inf
(`mali-deqp-cfold2.md` step C). A NaN keeps the host's bit pattern (x86-64's
default NaN is `0xffc00000`, aarch64's `0x7fc00000`). No reference of the four
families has a NaN or a subnormal constant, so none of this is measured
(section 6).

### 1.6 What the host libm is

glibc 2.28 and later use Szabolcs Nagy's routines for `sinf`, `cosf`, `expf`,
`exp2f`, `log2f`, `logf`, `powf` (ARM optimized-routines: table lookups,
polynomials in double, about 0.5 to 0.56 ULP, **not** correctly rounded) and
the x86-64 build has FMA ifunc variants of them. The double functions
(`atan`, `atan2`) are below 1 ULP and in practice correctly rounded.
Mesa's folds call `asinf` and `acosf` (binary32) but the double `atan`/`atan2`.
Which glibc drew the references is not recorded in the tree (section 6).

## 2. The constants of the four reference families

Families (all under `tools/mali/testdata/`): `deqp_cfold2_*` (19 references),
`deqp_negzero_*` (9), `deqp_constprod_*` (31) and `deqp_arith_constexpr_*` (13). I read every `expect.json` (words, FAU, `IADD_IMM`
and `FADD_IMM` immediates) against the sources. 23 sources call a built-in
that could fold; the results that **reach a word** are the rows of section 3.
A constant varying of a vertex shader is linked into the fragment shader
(`nir_link_opt_varyings`): there is no `vs.varying` region, and the folded
value is an fp32 FAU word of the fragment shader for a highp varying, or an
fp16 pair (`IADD_IMM #0xHHHHHHHH`, the same half twice) when the varying is
mediump in the fragment shader.

The folded values that do **not** reach a word, per reference (every call is
in dead code, in a compare whose result folds to a bool, or behind a lane
Mesa replaces; I could not follow them without Mesa's NIR dump, which needs
the shim build that this machine does not have, section 6): `cfold2_random_mixed_rcp_inf`
(2 calls: a runtime `atan`), `cfold2_random_mixed_rsq_inf` (7; one reaches
`3ad6`, below), `negzero_constglobal_vs` (5), `negzero_constglobal_fs` (22),
`negzero_mainbuiltin_fs` (21), `negzero_mainbuiltin_vs` (15; one reaches
`3f490fa4`, below), `constprod_reassoc_mixed_vs` (12; one reaches `bec4cfa7`),
`constprod_fma_random_trig_vs`, `fma_trig_acos_vs`, `fma_trig_atan2_vs`
(runtime polynomials), `fma_foldcall_acos_vs` / `foldcall_acos_fs` (`acos(1.0)`
is exactly 0.0 on every libm). The refs for `negzero_constglobal_reduced`,
`negzero_mainplain_*`, `negzero_array_inout_*`, `cfold2_const_other_*` and
`cfold2_random_mixed_{f2i,icmp,inot,rcp16}` call no libm function at all.

## 3. Measured table: Mesa's bits against `fmath.rs`, `std` and the exact value

Columns: **Mesa** the word in the reference's `expect.json`; **glibc** the host
libm function Mesa calls, through `std` (`f32::sin()` is `sinf`) and `ctypes`
(both give the same bits for every row); **f64 once** the f64 libm value
rounded once to f32; **exact** the correctly rounded value (80-digit
evaluation, exactly rounded to binary32 with ties-to-even); **fmath** this
tree's `fmath.rs`. A cell is one word; "=" means equal to Mesa's.

### 3.1 The libm results that reach a word

| Reference | Source constant | Mesa's fold | Mesa word | glibc | f64 once | exact | fmath |
|---|---|---|---|---|---|---|---|
| `arith_constexpr_fsin_fs_highp` | `sin(3.0)` | `sinf` | `3e1081c3` (FAU) | = | = | = | = |
| `arith_constexpr_fsin_vs_highp` | `sin(3.0)` into a mediump varying | `sinf`, then f16 RTNE | `3084` (`#0x30843084`) | `3084` | `3084` | `3084` | `3084` (via f32) |
| `arith_constexpr_fcos_fs_highp` | `cos(3.2)` (`3.2f`) | `cosf` | `bf7f903f` | = | = | = | = |
| `arith_constexpr_fcos_vs_highp` | `cos(3.2)` | `cosf`, f16 | `bbfd` | `bbfd` | `bbfd` | `bbfd` | `bbfd` |
| `arith_constexpr_fexp2_fs_highp` | `exp2(6.7)` (`6.7f`) | `exp2f` | `42cfefc4` | = | = | = | = |
| `arith_constexpr_fexp2_vs_highp` | `exp2(6.7)` | `exp2f`, f16 | `567f` | `567f` | `567f` | `567f` | `567f` |
| `cfold2_const_call_transcendental_log2` | `log2(100.0)` | `log2f` | `40d49a78` | = | = | = | = |
| `arith_constexpr_flog2_vs_highp` | `log2(100.0)` | `log2f`, f16 | `46a5` | `46a5` | `46a5` | `46a5` | `46a5` |
| `cfold2_const_call_transcendental_atan` | `atan(2.0)` | `atan` (double) | `3f8db70d` | `atanf` = | = | = | none |
| `cfold2_const_call_transcendental_pow` | `pow(1.7, 3.5)` (`1.7f`, `3.5f`) | `powf` | `40ccfc0f` | = | = | = | none |
| `constprod_fma_foldcall_atan_vs` | `atan(-1.0, -1.0)` | `atan2` (double), f16 | `c0b6` (`c0b6c0b6`); f32 `c016cbe4` | `c0b6` | `c0b6` | `c0b6` | none |
| `cfold2_random_mixed_rsq_inf` | `tan(sqrt(sqrt(0.25)))` | `sqrtf`, `sinf / cosf`, f16 | `3ad6` (FAU `3ad60000`); f32 `3f5ac132` | f32 `3f5ac132`, f16 `3ad6` (`tanf` gives the same) | n/a | f32 `3f5ac132` | none |
| `constprod_reassoc_mixed_vs` | `asin(0.375)` (negated in the FMA) | `asinf` | `bec4cfa7` = `-3ec4cfa7` | `3ec4cfa7` | = | = | none |
| `negzero_mainbuiltin_vs` | lane `atan(g)`, `g` = true = 1.0 | NIR `atan` polynomial, `fmaf` | `3f490fa4` (`IADD_IMM #0x3F490FA4`) | polynomial: fused `3f490fa4`, unfused `3f490fa4`; `atanf(1.0)` is `3f490fdb` | n/a | n/a | none |

Rows 12 to 14 are identified by the value alone (a 32-bit or 16-bit match of
the computed value with a word the reference holds, no other source of that
word): the lane-to-source mapping of those random shaders was not traced.
Every row agrees in all four columns: **on the constants the references hold,
glibc, the f64 value rounded once, the exact value and `fmath.rs` give Mesa's
bits.** `fmath.rs` has no `atan`, `pow`, `exp`, `log`, `asin`, `acos`, `atan2`,
`tan`.

### 3.2 The non-libm folds that fix the rules

| Reference | Fold | Mesa word | Rule shown |
|---|---|---|---|
| `constop` `ce_mod_vec2_float_fs` (existing, `mali-deqp-constop.md`) | `mod(a, b)` | `40233340` | unfused `x - y*floorf(x/y)`; fused is `40233335` |
| `constprod_fma_foldcall_mod_vs` | `mod(87.65, 3.7)` into a mediump varying | `411a` | f32 `40233340` (unfused) or `40233335` (fused) both give `411a`: the reference cannot tell |
| `cfold2_const_other_rcp16` | mediump `mat2 / mat2` | `bccd 4100 37fe 3c00` | f16 `fmul(a, frcp(b))`, each op f32 then RTNE: lanes `0.6/-0.5` = `bccd`, `-1/-0.4` = `4100`, `0.7/0.7 + 0.4/-0.8` = `3bff + b800` = `37fe` (an f16 divide gives `3c00`, so `3800`, not `37fe`) |
| `constprod_fma_foldcall_reflect_*` | `reflect(1.0, 1.0)` | `-1.0` | exact in f32 |

### 3.3 Where the other path differs (the reason for two entry points)

Over 2,000,000 random f32 arguments each (xorshift, host glibc 2.43):

| Comparison | Arguments, range | Differs from the glibc value | Largest difference |
|---|---|---|---|
| `fmath::sin` vs `f32::sin` | 2e6 in [-8, 8] | 25,493 (1.27 %) | 1 ULP |
| `fmath::cos` vs `f32::cos` | 2e6 in [-8, 8] | 25,810 (1.29 %) | 1 ULP |
| `fmath::exp2` vs `f32::exp2` | 2e6 in [-10, 10] | 1,302 (0.065 %) | 1 ULP |
| `fmath::log2` vs `f32::log2` | 2e6 in [0.001, 1000] | 437 (0.022 %) | 1 ULP |
| `fmath::{sin,cos,exp2,log2}` vs the f64 libm value rounded once | the same | **0** of each | 0 |
| `f32::powf` vs f64 `pow` once | x in [0.01, 20], y in [-8, 8] | 1,282 (0.064 %) | |
| `f32::exp` vs f64 `exp` once | [-8, 8] | 1,296 (0.065 %) | |
| `f32::ln` vs f64 `ln` once | [0.01, 100] | 3,480 (0.17 %) | |
| `f32::atan` vs f64 `atan` once | [-8, 8] | 0 | |
| `f32::asin`, `f32::acos` vs f64 once | [-1, 1] | 0, 0 | |
| `f32::atan2` vs f64 `atan2` once | y in [-8, 8], x in [-1, 1] | 0 | |
| `sinf / cosf` vs f64 `tan` once | [-8, 8] | 638,493 (32 %) | |
| `exp2f(x * float(M_LOG2E))` vs `expf(x)` | [-8, 8] | 1,291,707 (65 %) | |
| `log2f(x) * float(1/M_LOG2E)` vs `logf(x)` | [0.01, 100] | 500,697 (25 %) | |

Reading: `fmath.rs` is **exactly** "evaluate in f64, round once" (0 differences
from the f64 libm on 8e6 arguments), which is not what glibc does: glibc's
`sinf` and `cosf` miss the correctly rounded result for about 1.3 % of the
arguments, `exp2f`, `expf`, `powf` for about 0.065 %, `logf` 0.17 %, `log2f`
0.022 %. The `atan` family is a double call (by Mesa) or agrees with the exact
value on this glibc (`asinf`, `acosf`: see section 6, the glibc version). Where
glibc's own routine is below correct rounding, no f64-then-round helper can be
bit-exact on every argument: only a port of the same algorithm can. `tan` is
`sinf / cosf` and compounds it (32 % off a correct `tan`, about 2.6 % off if
`sin` and `cos` were glibc-exact and the division is done in f32).

### 3.4 Constants of the polynomial lowering (for the arith/fclane designs)

`build_asin` coefficients (`-0.0187293`, `0.0742610`, `-0.2121144`,
`1.5707288`, `M_PI_2f` as f32) are the FAU words `bc996e30`, `3d981627`,
`be593484`, `3fc90da4`, `3fc90fdb` of `deqp_constprod_fma_trig_acos_vs`; the
`nir_atan` ones are listed in 1.4; they are the double constants of
`nir_builtin_builder.c` converted to f32 (round to nearest even), which is
what a Rust `as f32` of the double literal does.

## 4. The helper

### 4.1 Location

A new shared module **`src/shaderc/online/mesa_fold.rs`**, next to `fold.rs`
and `fmath.rs`, `no_std`, `pub(crate)`, built on `fmath.rs` for the four
functions it already has. Not `fold.rs`: `fold_float` is also the V3D's and the
PowerVR USC's fold and returns `None` for `sin`/`cos`/`exp2`/`log2` except where
exact, because those QPUs and the USC do not compute these as glibc does; a
transcendental arm there would change their words. Not `mali/`: negzero N1/N2
fold in `glsl.rs` (the front end) and CF1/K4 in `mali/nir.rs` `alu_sized`, and
`glsl.rs` is not a Mali file. `fmath.rs` is not changed: the transform-feedback
capture uses it in the product and its pins must not move; the new module
calls it.

### 4.2 Signature

```rust
/// A built-in Mesa folds with the host libm (section 1).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Libm { Sin, Cos, Tan, Asin, Acos, Atan, Atan2, Exp, Exp2, Log, Log2, Pow, Sqrt, Rsq }

/// The binary32 result of the GLSL IR folder (1.1) on binary32 operands:
/// `ir_expression_operation.py`'s C expression, bit for bit. `y` is the second
/// operand of `Atan2` and `Pow`, ignored otherwise. Never `None`: the result
/// of a domain error is the host's (NaN, inf), as Mesa stores it.
pub(crate) fn fold_glsl(f: Libm, x: f32, y: f32) -> f32;

/// The same for a built-in NIR sees (1.2): `Exp` is `exp2f(x * LOG2E_F32)`,
/// `Log` is `log2f(x) * LN2_F32`; `Tan` does not exist (use `Sin`, `Cos`).
pub(crate) fn fold_nir(f: Libm, x: f32, y: f32) -> f32;

/// A 16-bit result (1.3): the f32 function of the widened operands, then
/// `_mesa_float_to_float16_rtne`. `x`, `y` are f16 bit patterns.
pub(crate) fn fold_nir_f16(f: Libm, x: u16, y: u16) -> u16;

/// `fmaf`: the single rounding of `a * b + c` (NIR `ffma`, `ffma_weak`).
pub(crate) fn fma32(a: f32, b: f32, c: f32) -> f32;
```

Callers keep their own rule for *when* to fold (a call folds as the **whole
GLSL built-in**, in GLSL IR order: `mod`, `smoothstep`, `reflect` fold as their
GLSL body does, section 1.1, never op by op after our IR's own expansion):

* `glsl.rs` (N1, N2): `fold_glsl` for a call whose operands are all literals or
  `const`; exactly where Mesa's GLSL IR folder folds, i.e. before our IR's
  `Flog2 * k` expansion of `exp`/`log` (`mali-deqp-cfold2.md` step B says the
  same).
* `mali/nir.rs` `alu_sized` (CF1, K4): `fold_nir` / `fold_nir_f16` for an `Fsin`
  `Fcos` `Fexp2` `Flog2` `Fpow` of constants (today `ConstantFold`), `fma32` for
  `Ffma` **only** where the op came from a NIR polynomial (1.4); an `Ffma` that
  our IR emitted for a GLSL built-in keeps the unfused rule.

### 4.3 How it reaches bit equality

Three levels, one function boundary, so a level can change without touching a
caller:

1. **Exact functions** (no libm): `Sqrt` is `fmath::sqrt` (correctly rounded,
   equals `sqrtf`); `Rsq` is `1.0 / sqrt` and `rcp` is `1.0 / x`, two roundings
   as Mesa; `floor`/`ceil`/`trunc` stay `fold.rs`'s. `Atan`, `Atan2` are
   Mesa's **double** calls: an f64 evaluation of the function (argument
   reduction by `x/(1+sqrt(1+x^2))` and the odd series, error below 2^-52
   relative), rounded once to f32, equals Mesa's value except where the f64
   result lies within 2^-29 relative of a binary32 rounding tie (about 1 in
   5e8 arguments, and the same family of arguments for glibc's double
   `atan`): accepted, because **it is the very computation Mesa does** (f64
   value, then the store to f32).
2. **Evaluate in f64, round once** (`fmath.rs`'s method, measured above as the
   correctly rounded result on 8e6 arguments): `Exp2`, `Log2` (existing
   `fmath::exp2`, `fmath::log2`), new `exp`, `log`, `pow` (as `exp2(y * log2(x))`
   in f64, with the double-double or the error analysis it needs: `pow` must
   stay below 2^-30 relative before the rounding), `Asin`, `Acos` (f64 `atan2`
   of `x` and `sqrt(1 - x^2)`). These differ from glibc's f32 function on the
   fractions of section 3.3 (0.02 to 0.17 % of the arguments), because glibc's
   is not correctly rounded; on every constant of section 3.1 they agree.
3. **Port glibc's algorithm** for the functions where level 2 is not enough:
   `sinf`, `cosf` (about 1.3 % off correct rounding, `tan` doubles that) and,
   only if a reference needs it, `expf`, `exp2f`, `logf`, `log2f`, `powf`. The
   routines are Szabolcs Nagy's (MIT OR Apache-2.0, compatible with this
   tree), arithmetic in double plus small tables (`sincosf` coefficients and
   the `1/(2pi)` bits, 32-entry `exp2f`, 16-entry `log2f`, 128-entry `powf`
   data), so a port is a few hundred lines and bit equal to glibc except for
   inputs where the double evaluation's last bit flips the f32 rounding
   (about 2^-29 per argument, FMA or not). Needs the rules of section 5 to
   decide when.

**Decision of this design: level 1 and 2 first, in CF1, and level 3 only for
`sin`, `cos` (and hence `tan`) if the check of section 5 finds an argument
that differs.** Reason: all 14 rows of 3.1 agree at level 2; levels 3 for
`exp2`, `log2`, `pow` change fewer than 1 argument in 1,000.

`fold_nir_f16` is level 2's f32 result rounded with a `to_f16_rtne` (already
in the tree for the existing f16 folds: `select::f16_exact` is the exact-only
twin; the RTNE rounding of a f32 to f16 is a plain bit routine).

## 5. Which item writes it, and what pins it

**CF1 writes it first** (the schedule's rule), as the part of the `cfold`
lowering step B that folds `Fsin Fcos Flog2 Fexp2 Fpow` and `log2`/`pow`/`atan`
constants (`mali-deqp-cfold2.md` section 4, step B; first un-ignore
`const_call_transcendental_log2`). It adds `mesa_fold.rs` and its unit tests
and the `mali/nir.rs` call site. N1 and N2 (`negzero-lower`) then add only the
`glsl.rs` call site and K4 (`constprod-lower-3`) the `Ffma`/`mod` routing.

Tests that pin it:

* `mesa_fold.rs` unit tests (host): every row of 3.1 and 3.2 as `assert_eq!`
  of bits, `fold_glsl` and `fold_nir` apart for `Exp` at `5.5` (`4374b122`
  against `4374b121`), `Log` at `42.12` (`406f64b9`, the number in
  `mali-deqp-cfold2.md` section 4), `fma32` against `f32::mul_add` on a sweep,
  `atan` at `1.0` through the polynomial and `fma32` (`3f490fa4`).
* A host test (`std`, glibc) comparing `fold_glsl` and `fold_nir` with `std`
  on a few hundred thousand arguments that **counts** the differences per
  function and asserts the counts of section 3.3 as upper bounds (it is the
  guard that level 2 does not drift, and the trigger for level 3 if a count is
  raised for a function).
* The byte-compare references: `mali_deqp_cfold2.rs`
  (`const_call_transcendental_{log2,pow,atan}`), `mali_deqp_arith.rs`
  (`constexpr_f{sin,cos,exp2,log2}_{fs,vs}_highp`),
  `mali_deqp_constprod.rs` (`fma_foldcall_atan_vs`, `reassoc_mixed_vs`),
  `mali_deqp_negzero.rs` (`mainbuiltin_vs`, and the other shapes as the
  lowering reaches them).
* The check that decides level 3: when the first dEQP-GLES2 host forecast
  (`tools/mali/deqp-host-measure.sh`, the corpus of `constant_expressions`)
  of the lowered shapes runs, fold each `sin`/`cos`/`tan` constant of the cases
  and flag those where `fold_glsl` and `std` differ; each flagged argument is a
  case whose bits the helper would get wrong, while the dEQP check (the
  harness's `isOk` tolerance) passes with either. The count is the cost of
  skipping level 3.

## 6. What could not be decided

1. **The glibc version that drew the references.** Not recorded in the tree
   (`tools/mali/README-pan_shc.md` and the notes say nothing). Recent glibc
   versions import correctly rounded routines for some binary32 functions; I
   believe (not checked) that this is why `asinf`, `acosf` match the exact
   value on 2e6 arguments here (glibc 2.43) and may not on an older glibc. The
   `asin`/`acos` fold of `reassoc_mixed_vs` (`bec4cfa7`) agrees at every level
   on this host. Redrawing one reference on a second host would settle it; this
   item changes no reference.
2. **`sin`/`cos` level 3 yes or no** (section 4.3): decided only as "no until
   a count says so"; the number of distinct `sin`/`cos`/`tan` arguments in all
   dEQP-GLES2 `constant_expressions` and random shaders was not measured (the
   dEQP sources of the cases are not in this checkout: `data/gles3/shaders`
   has only the `.test` files, not `glsShaderConstExprTests.cpp`).
3. **Which GLSL IR path applies to a given constant** when the constant
   appears only after our front end's own folding (`exp(c)` where `c` is a
   folded `const`): Mesa folds a `const` initialiser first, so `fold_glsl`;
   whether it ever reaches `fold_nir` with an `exp` or `log` in a dEQP
   reference is unknown (no reference shows `exp`/`log` on a constant at all;
   the two paths differ on 65 % and 25 % of random arguments, so the first
   reference that does will show which). `fold_nir`'s `Exp`/`Log` forms are
   written from `glsl_to_nir.cpp:2177-2178` and unpinned.
4. **The fp16 folds** are pinned by five rows (3.1) and `rcp16` only; no
   reference holds an f16 `exp`, `log`, `pow`, `tan` result.
5. **Fused or unfused `mod` in f32**: decided unfused by `ce_mod_vec2_float_fs`
   (a fragment reference; `constprod_fma_foldcall_mod_vs` cannot tell, section
   3.2). x86-64 Mesa builds without FMA contraction (a generic `-march`); a
   distribution that builds Mesa with `-mfma` could contract `src0 - src1 *
   floorf(src0/src1)` into an `fma`, which this host's references did not.
6. **NaN, infinity and subnormal results**: no reference holds one; the helper
   returns the f32 the host libm would (inf for `exp2(200)`, NaN with the sign
   of x86-64 for `log2(-1)`), and the caller decides (today `fold_float`
   refuses non-normal results). Whether Mesa's Valhall back end keeps a
   subnormal immediate is not measured.
7. **Large arguments of `sin`/`cos`** (|x| above 2^19): `fmath.rs` loses
   accuracy there (its own doc says so). The references and dEQP's folded
   constants are below 2π in magnitude; a `fold_glsl(Sin, 1e9, _)` is
   unpinned and would need the exact reduction of glibc's (Payne-Hanek) port.
8. **The lane-to-source mapping** of rows 12 to 14 of 3.1 (random shapes):
   identified by value only (section 3.1).
9. **Folds I found no live instance of**: `Exp`, `Log` on a constant that
   reaches a word, `Tan` on a constant other than the `3ad6` lane, `Asin` and
   `Acos` of a non-trivial constant outside `reassoc_mixed_vs`, `Pow` other
   than `pow(1.7, 3.5)`: the helper covers them by the same rules (sections 1
   and 4) but no reference pins them.

## 7. How the measurements were made (scratch, not committed)

* Mesa side: read in `compiler/glsl/ir_expression_operation.py`,
  `ir_constant_expression.cpp`, `glsl_to_nir.cpp`, `builtin_functions.cpp`,
  `compiler/nir/nir_opcodes.py`, `nir_constant_expressions.py`,
  `nir_opt_constant_folding.c`, `nir_builtin_builder.c` of the tree in
  `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` (the Mesa tarball of
  `tools/mali/drm_shim_ref.sh build`; it has a `build-pan/` of the pan_shc
  build, no installed shim).
* Constants: for each reference a script prints each region's FAU words and
  `#0x` immediates; the sources give the call and its arguments.
* Libm: Python `ctypes` on `libm.so.6` (`sinf`, `cosf`, `exp2f`, `log2f`,
  `powf`, `logf`, `expf`, `atanf`, `tanf`, `fmaf`; double `atan`, `atan2`,
  `pow`, ...), the exact value by a Taylor/atan series and `decimal` at 80
  digits rounded to binary32 with ties-to-even from a `Fraction`; f16 by
  `struct` format `e` (round to nearest even).
* `fmath.rs` and `std`: a scratch crate with `#[path]` to
  `src/shaderc/online/fmath.rs`, `black_box` around the arguments so that no
  constant is folded at compile time by LLVM; the sweeps use an xorshift64
  generator seeded `0x123456789abcdef0` and 2,000,000 arguments per row.
