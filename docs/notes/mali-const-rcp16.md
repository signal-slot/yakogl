<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: the fp16 reciprocal of a constant (CONST_RCP16, plan #367)

Refract's fragment shader `3fae` calls `refract(.., 1.0 / RefractiveIndex)`
with `const float RefractiveIndex = 1.2`. The mediump walk
(`src/shaderc/online/mali/nir_fs.rs`, the `AluOp::Frcp` arm) folded the
reciprocal of a constant only when both the constant and its reciprocal
were exact in fp16. Every other constant was refused as `ConstantFold`, "a
constant operation on or to a value that is not a finite normal float".
1.2 is not exact in fp16. Lowered in plan #367 lower-train-2 (from
proto-const-rcp16).

## What Mesa does

The reference is `tools/mali/testdata/gm2_nbshrf_7` (Mesa 26.2.1, drm-shim,
G610): `vec4(sc.xyz * (1.0 / ratio), 1.0)` with `const float ratio = 1.2`.
Its fragment FAU word 4 is `3aaa3aaa`, read by `FMA.v2f16 r0, r0^, u2.w0,
0x0.neg`.

- The GLSL front end does not fold the quotient. `1.0 / c` reaches NIR as
  `fdiv(1.0, c)`, which nir_opt_algebraic turns into `frcp(c)`.
- lower_precision makes the operation 16-bit and rounds `c` to fp16
  (`_mesa_float_to_half`): 1.2 becomes `0x3ccd`, which is 1.2001953.
- `nir_opt_constant_folding` widens the half, computes `1.0f / c` in
  `float` and rounds the result to nearest even
  (nir_constant_expressions.py:551-556). 0.8331976 rounds to `0x3aaa`.

The fp32 quotient rounded once would give `0x3aab` (0.8333333 is nearer
0.8334961 than 0.8330078). So the `3aaa` word shows that Mesa rounds the
operand first, as the 16-bit fold does. The same rule makes
`deqp_cfold2_zero_operand_rcp16` (`mat3 / mat3`) and
`deqp_cfold2_const_other_rcp16` (`mat2 / mat2`, its `fmul(a, frcp(b))`
lanes `bccd 4100 37fe 3c00`; docs/notes/mali-deqp-cfold2.md) byte-equal.
pan_shc's SPIR-V path has no lower_precision. Its word is the fp32
`3f555555`, so `cases.txt` records the case's `fs.fragment` as
`differs:mediump`.

## The fold

`fold_rcp16(c)` in nir_fs.rs (next to `fold_const16`) works in three steps:

1. Round `c` to fp16 with `f16_rtne`.
2. Take `1.0 / half` in f32.
3. Round the result with `round16`.

The walk's `Frcp` arm calls it in place of the exact-only test; its run-time
branch (`FRCP.f16` of a mediump lane, main's arith step 1a) is unchanged.
When `c` and `1/c` are both exact (gm2_fsmisc_4's `smoothstep`), the result
is the same value as before. A `fmul` of another constant by the result is
the existing 16-bit product fold (`fold_const16`). That is the
`fmul(a, frcp(b))` that `const_other_rcp16` shows.

These cases have no reference and are still refused:

- an operand of 0.0 (the reciprocal is an infinity);
- an operand that is an fp16 subnormal (`1.0e-5`);
- a result that is an fp16 subnormal (`1/30000`);
- an operand past fp16's range (`1.0e6`).

A highp constant in a mediump shader has a reciprocal that Mesa computes in
fp32. The fold is not the cause of its refusal: the shader that mixes
precisions is refused (`deqp_vsconst_fsdiv_highp_mat2`/`_mat3` now give that
refusal). If that refusal is ever lowered, this arm must not fold highp
constants at fp16. Tests: `tests/mali_const_rcp16.rs`.

## Results

A sweep of the 1,810 `tools/mali/testdata` pairs (`tests/mali_corpus_diff.rs`)
moves five: gm2_nbshrf_7, deqp_cfold2_zero_operand_rcp16 and
_const_other_rcp16 to byte-equal, deqp_vsconst_fsdiv_highp_mat2/_mat3 from
ConstantFold to the mixed-precision refusal. The glmark2 corpus does not move
(refract fs 3fae stops first at the constant product, lower-train-3's).

## Not covered

`x / 1.2` and `1.0 / 1.2` with a literal divisor never reach this arm. The
shared front end (`src/shaderc/online/lower.rs` `reciprocal`) folds the
quotient at fp32 and marks the constant `Prec::Unknown` "a refusal for the
back end that lowers precision". The Mali back end does not refuse it: it
compiles `sc.xyz / 1.2` with `3aab3aab`. That is the word of an fp32 fold
rounded once, and the NIR rule above predicts `3aaa3aaa`. No reference
draws a literal divisor yet, so this is an open cause, not part of
CONST_RCP16's fold.
