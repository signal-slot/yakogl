<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: a fragment `refract()` (plan #367 lower-train-2, FS_REFRACT)

Refract (scene 24) calls `refract(eye, normal, 1.2)` in its mediump
fragment shader. Our front end expands it with the specification's own
formula (builtin_call.rs `refract`): `d = dot(N, I)`, `k = 1.0 - eta * eta
* (1.0 - d * d)`, and `k < 0.0 ? 0.0 : eta * I - (eta * d + sqrt(k)) * N`.
The select is an IR `bcsel` of a constant 0.0 and a computed vector on a
mediump compare. The walk took a select on a 16-bit mask only when both
sides were constants. Anything else was refused at nir_fs.rs's `Bcsel` arm
as "arithmetic other than +, -, * ..." (`MaliError::Arithmetic`). That is
FS_REFRACT. The reference is gm2_nbshrf_6 (study-next-shadow-refract):
`vec4(refract(normalize(vp.xyz), normalize(vn), 1.5) * 0.5 + 0.5, 1.0)`.
The lowering is proto-fs-refract's, applied on main in lower-train-2.

## What Mesa does (gm2_nbshrf_6)

Mesa's GLSL builtin (builtin_functions.cpp `_refract`) computes `n_dot_i`
and `k` as `float`s and returns from an `if`. NIR's peephole makes that a
`bcsel`. The fragment program after the two `normalize`s:

```
FMA.v2f16 r4, r3.h00, r1.h00, 0x0.neg           d = dot(N, I), from z
FMA.v2f16 r4, r2.h11, r0.h11, r4^.h00
FMA.v2f16 r4, r2.h00, r0.h00, r4^.h00
FMA.v2f16 r5, r4.neg.h00, r4.h00, 0x3C000000.h11  1 - d*d          one lane
FMA.v2f16 r5, r5^.neg.h00, u2.w0, 0x3C000000.h11  k = 1 - om*2.25  one lane
FRSQ.f16 r6, r5.h00                               sqrt(k) = rcp(rsq(k))
FRCP.f16 r6, r6^.h00
FMA.v2f16 r4, u2.w1, r4^.h00, r6^.h00           t = 1.5*d + sqrt(k)
FMA.v2f16 r2, r4.h00, r2^, 0x0.neg              t * N.xy
FMA.v2f16 r3, r4^.h00, r3^.h00, 0x0.neg         t * N.z
FMA.v2f16 r0, u2.w1, r0^, r2^.neg               1.5 * I.xy - t*N.xy
FMA.v2f16 r1, u2.w1, r1^.h00, r3^.neg.h00       1.5 * I.z - t*N.z
FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11  * 0.5 + 0.5, before the select
FMA.v2f16 r1, r1^.h00, 0x38000000.h11, 0x38000000.h11
FCMP_OR.v2f16.lt.m1 r2, r5^.h00, 0x0, 0x0       k < 0
MUX.v2i16 r0, r0^, 0x38000000.h11, r2.h00       k < 0 ? 0.5 : ..
MUX.v2i16.wait0126 r1, r1^.h00, 0x38000000.h11, r2^.h00
```

Four rules give this shape:

1. `op(bcsel(a, #b, c), #d)` is `bcsel(a, op(b, d), op(c, d))` for an
   `fadd` or `fmul` whose select is read once (nir_opt_algebraic.py
   :3429-3433). The constant must be the select's *first* side. So
   `bcsel(k < 0, 0.0, r) * 0.5 + 0.5` is `bcsel(k < 0, 0.5, r * 0.5 +
   0.5)`, and the late fusion makes `r * 0.5 + 0.5` an `ffma`.
2. `k` and `t` are scalars, one lane each. The front end fills every lane
   of those steps with the same value and records 0 in its lane table
   (ir.rs `Module::lanes`). The walk read them as 4-wide operations
   (`Fs::wide`), and the pair graph went round `d * d`'s pair
   (`Defect(LaneCycle)`).
3. `eta * (eta * om)` is `om * (eta * eta)` (GLSL's `reassociate_constant`,
   opt_algebraic.cpp:175-202), the value first: `FMA.v2f16 r5, r5^.neg.h00,
   u2.w0, ..`. Our front end folds `eta * eta` first, so the walk had the
   constant first.
4. `eta * I - t * N` is `fadd(fmul(eta, I), fneg(fmul(t, N)))`. The late
   fusion (nir_opt_algebraic.py:3817) takes the first product. The front
   end's `ffma(fneg(t), N, eta * I)` is the other fusion. `t * N` is its
   own vector operation, so both of its pairs come before both
   multiply-adds.

## The lowering (src/shaderc/online/mali/nir_fs.rs)

- The walk's mask16 `Bcsel` arm takes a constant on one side and an fp16
  lane on the other as a 16-bit `Bcsel` node.
- `bcsel_const_ops`, after `mark_live`, applies rule 1 to the 16-bit
  graph, using `Fs::used_once` on the live nodes. The select's node becomes
  the operation on its computed side and the operation's node becomes the
  select, so node order holds. The constant is folded at fp16
  (`fold_const16`).
- `scalar_step`: a step the lane table records as 0, all of whose sources
  are splats, one-valued constants or such steps, is one lane wide for
  `Fs::wide` (rule 2). A matrix product's steps also record 0 but read
  their columns lane by lane, so they are not scalar steps.
- The walk's `Fmul` arm swaps `const_square_step(s[0]) * b` to `b * k`
  (rule 3). The `Ffma` arm rewrites `refract_ffma` (rule 4), the products
  standing at `fneg(t)`'s instruction so that their events sort first.
- In the pair graph, `computed_select` is a select with a constant on one
  side and an fp16 sum or product on the other, on a compare with a
  computed operand. For two selects on one mask, the mask is the lane's
  compare and both sides are paired (`MUX.v2i16 r0, r0^, 0x38000000.h11,
  r2.h00`). A third select alone beside a constant is a lane packed with
  it (`MKVEC`). A select whose computed side is a `b2f` or another select
  (the compare products of dEQP's `compare_float`) is not one: those stay
  refused as before.

## Results (measured at lower-train-2, on main 9a74631a plus the steps before)

- gm2_nbshrf_6 is byte-equal in every region (`tests/mali_fs_refract.rs`).
  In `cases.txt`, pan_shc's SPIR-V path differs: `mediump` in the fragment
  program, and `unanalysed` in the varying program, because it stores `vn`
  before `vp`.
- The sweep of every `tools/mali/testdata` reference before and after
  (`tests/mali_corpus_diff.rs`, 1,810 programs) moves only three.
  gm2_nbshrf_6 goes REFUSED → EQUAL. deqp_arith_op_geometric_refract_fs_
  {highp,mediump} go ARITH → "a gl_FragColor lane ..." (`ColourForm`): dEQP's
  scalar `refract` of three varyings, colour lane x alone. Their pins in
  `tests/mali_deqp_arith.rs` move with them.
- `tests/mali_glmark2_corpus.rs`: 23 of 33 scenes clean, unchanged.
  Refract fs `3fae` still stops first at the constant product
  (ARITH_CONST: `normalize(-v)`, constprod-rest's, lower-train-3).

## What stops refract next

Measured by proto-fs-refract on stand-ins of fs `3fae` with glmark2's vertex
shader `7215`, `normalize(-v)` written `normalize(v)`:

1. The front `refract` with eta 1.2 compiles.
2. `MapCoord.xyz / MapCoord.w`: the fp16 `frcp` of a varying lane. Main's
   fragment divide of mediump lanes (dbdfb25b) may cover it; not re-measured
   here, since the constant product stops 3fae first.
3. Behind it: TEX_COORD2's wider forms (a lone look-up at refract's computed
   coordinate), TEX_DEPENDENT (DistanceMap's texel in NormalMap's
   coordinate) and CONST_RCP16 (`1.0 / RefractiveIndex`).

Building `refract` in Mesa's own order in builtin_call.rs would make rules 3
and 4 unnecessary, but would change the code the other back ends make.
