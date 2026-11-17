<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): glmark2's fragment arithmetic (ARITH)

**Status: gm2_fsmisc_1..8 and gm2_fsarith_1..2 byte-equal; gm2_fsarith_3..7
open** (plan #367 lower-fs-arith, issues #32/#33). The fragment ARITH family
of docs/notes/mali-glmark2-forecast.md: the arithmetic the Mali online
compiler (`src/shaderc/online/mali/`) refused in glmark2's fragment shaders.
The references are Mesa 26.2.1 on drm-shim (`tools/mali/testdata/
gm2_fsmisc_*`, `gm2_fsarith_*`, drawn without `--uniform-values`); the host
test is `tests/mali_fs_arith.rs`, the pan_shc verdicts are in
`tools/mali/testdata/cases.txt`.

## 1. What each reference needed

| Case | Shape | What the compiler had not | Where |
|---|---|---|---|
| gm2_fsmisc_1 | `sqrt(x)` of a vec4 varying | `frsq` of a varying's lane (only a computed one); a pair of one-lane ops packed; `FRSQ.f16`/`FRCP.f16` of a high half (`.h11`, bits 28-29: the table's 1-bit `lane0` and bit 29 set after the encoding); every lane's `frsq` before every lane's `frcp` (`lower_fsqrt` runs on the vector) | nir_fs.rs walk `Frsq`/`Fsqrt`, `vectorise`, `lane_tree`; pack.rs |
| gm2_fsmisc_2 | `fract(3.0 * x.x)` squared, `vec4(d, x.yzw)` | a word of a computed lane and a varying's lane (`MKVEC.v2i16 r0, r1^.h0, r0^.h1`, the lane converted with its pair at the load); the late `ffma` fusion takes a product used once only (`fmul(is_used_once)`) | `mixed_word`, `pair_read`, `Fs::used_once` |
| gm2_fsmisc_3 | `clamp(x, 0.25, 0.75)` | mediump `fmax`/`fmin` (`FMAX.v2f16`, `FMIN.v2f16` of pairs, rows `(Fmax,16,2)`, `(Fmin,16,2)`) | walk `Fmax`/`Fmin`, emit.rs |
| gm2_fsmisc_4 | `vec4(smoothstep(0.25, 0.75, x.x))` | `frcp` of a constant folded; `fmin(fmax(a, 0.0), 1.0)` as `fsat` fused into its producer; `fmul(a, 2.0)` as the late `fadd(a, a)` after fusion; Mesa's `t * (t * (3 - 2t))` (`_smoothstep`, builtin_functions.cpp) where the front end writes `(t * t) * ..`; one scalar in both lanes of a word (`MKVEC x.h0, x.h0` as `IADD.v2u16`, the other word its `MOV.i32`) | walk `Frcp`, `smoothstep_tail`, `late_rules`, `select` roots |
| gm2_fsmisc_5 | `exp2(x)` of a vec4 varying | `exp2` at all: `bi_fexp_32` (`FMA_RSCALE.f32 x, 1.0, -0.0, 24`, `F32_TO_S32` with round 0, new `Op::F32ToS32Rte`, `FEXP.f32`); of a varying's lane on the load's fp32 words (its `f2fmp` gone), the results narrowed in pairs (`Leaf::Raw`) | emit.rs `fexp2`, `Leaf::Raw`, bir.rs |
| gm2_fsmisc_6 | `exp2(-2.0 * d * d)`, `mix` of two `const vec4` | `min` of two computed lanes; `nir_opt_vectorize` of separate scalar ops with shared sources (`mates`: `dist.x * dist.w` and `dist.y * dist.w` one `FMA.v2f16`, `dist.z * dist.w` alone); a negated literal first in a one-lane product (glsl.rs leaves it, the walk makes `fneg(fmul(2.0, d))`); `exp2` of a computed fp16 lane (`transc_rows`); constants folded at fp16 (`1.0 - 0.8` is `0x3268`: 0.8 rounds to `0x3A66` first) | `mates`, `neg_literal`, walk's constant fold |
| gm2_fsmisc_7 | `mix(a, b, t)` of varyings | production `(Splat, FmaA)` | `ADMITTED` |
| gm2_fsmisc_8 | `step(0.5, x) * y` | `inot(flt)` as the inverse 16-bit compare; `fmul(b2f(m), y)` as `bcsel(m, y, 0.0)` (nir_opt_algebraic.py:1695); two lanes' compares one `FCMP_OR.v2f16.ge.m1` (row `(Fcmp(Ge),16,2)`) read whole by `MUX.v2i16`, its 0 side `.h00` so `va_lower_isel` keeps the `MUX` | `int_lane`, `Fs::op`, `vectorise`, `op` |
| gm2_fsarith_1 | mediump directional light: `normalize(vec3(nm * vec4(n, 1.0)))`, `normalize(light)`, `max(dot(), 0.0)`, `pow(.., 100.0)` | a `mat4` uniform in the fp16 block (`UboLayout::fs16`, columns in order as halves, a load each); `M * vec4(v, 1.0)` reassociated (`reassoc_fma_optimizations`: `c2 z + (c0 x + (c1 y + c3))`); the lanes of a vector operation kept in pairs although only a reduction reads them (`Fs::pair_lane`, `Leaf::Part`), its dead lanes dropped (`Fs::mark_live`), the colour's own lane index preferred for CSE'd lanes (`Pairs::hint`); a vec3 operation's undefined fourth lane; GLSL tree grafting of a vec3/float local the front end widened to 4 (`grafted_locals`); `fmax(a, 0.0)` as `.clamp_0_inf`; `pow` as `bi_fpow_32` (`fexp2(fmul(flog2(a), b))` is `fpow`, nir_opt_algebraic.py:1872; `log2` of a computed lane, then `FMA_RSCALE` of the exponent by the log); a scalar product fused into a pair's sum reads its constant as a pair; productions `(Const, FmaA)` | select.rs, nir_fs.rs |
| gm2_fsarith_2 | the same at highp | `pow` at fp32 (row `(Fpow,32,1)`); the three-term matrix chain, each lane's chain whole | `Fs::alu32_lanes`, `lane32` |

The case-77 program (`fsarith_77_transc`, `sin`, `cos`, `log2`, `exp2` of
uniforms) is byte-equal whole with `exp2` (fs_transc_tests.rs), and
`tools/shaders/es100/transc.frag` moved to `ok` in corpus-verdicts.txt.

## 2. Rules kept narrow

Each new form is admitted where a reference draws it and refused elsewhere
(`tests/mali_fs_arith.rs` `the_shapes_no_reference_draws_stay_refused`):
`pow` by 1.0 to 4.0 (rewritten by nir_opt_algebraic.py:1882-1886) refuses;
`exp2` of a varying's lane alone beside a constant refuses; a lone-lane word
takes a tree with a `pow` (gm2_fsarith_1's blue beside 1.0), not one with
`sin`; a `mat2`/`mat3` fragment uniform refuses; the negated-literal product
is left to the walk only for one lane in a mediump fragment shader; the
mediump reassociation of `M * vec4(v, 1.0)` takes three terms only (two
terms are `ffma(m1, y, ffma(m0, x, m2))` at fp32, unmeasured at fp16).

## 3. The FAU of the lit programs

The gm2_fsarith references hold 0 for every uniform word, so the test pins
which uniform each pushed word is from the instructions that read it:
gm2_fsarith_1's `FMA.v2f16 r2, u2.w0, r0.h11, u2.w1` is `nm[1].xy * n.y +
nm[3].xy`, so `u2.w0` is dwords 12-13 and `u2.w1` 20-21
(`gm2_fsarith_push_the_columns_mesa_reads`).

## 4. Open

- gm2_fsarith_1/2's program link: their vertex shader writes `uv`, which the
  fragment shader does not read, and the link refuses ("a Mali link whose
  stages place a varying at different offsets", `'n'`); even `gl_FragColor
  = vec4(n, 1.0)` is refused with that vertex shader. A varying-link item,
  not this one; the test compares the fragment program compiled alone.
- gm2_fsarith_3/4 (point light, Phong): `normalize(-v)`, `reflect(-ld, nn)`
  and `lpos.xyz / lpos.w - vp.xyz / vp.w` need NIR's fneg propagation
  (`fmul(fneg(a), fneg(b))` is refused as `ArithConstant`; Mesa folds the
  negations into the readers' `.neg`).
- gm2_fsarith_5 (cel): refused as `ConstantFold` (a constant operation on
  a value that is not a finite normal float; its constant `lsp` vector is
  read by `length` and a difference); not analysed further.
- gm2_fsarith_6 (bump normal map): a texel read `.xyz` (TEX_SWIZZLE,
  lower-tex-swizzle).
- gm2_fsarith_7 (shadow map): refused as `Arithmetic` at `sc / sc.w`
  (`frcp` of a varying's lane, which no reference draws alone); behind it
  a computed look-up coordinate (TEX_COORD).
