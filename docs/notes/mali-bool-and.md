<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: `&&` of two compares in a mediump fragment shader (plan #367 lower-train-2, step BOOL_AND)

glmark2 shadow fs `cf4b` tests `ShadowCoord.w > 0.0 && light_distance <
sc_perspective.z` and picks the shadow factor 0.5 or 1.0 from that test.
In a mediump shader, an `iand` of two 16-bit compare masks fell through
`Fs::int_lane` and was refused at INT_VALUE ("an integer or boolean value
is not lowered yet"). The Mesa 26.2.1 drm-shim references are
`tools/mali/testdata/gm2_nbshrf_1` (two varyings' lanes on one side) and
`gm2_nbshrf_2` (a texel on one side), both original reductions made by
study-next-shadow-refract.

## What Mesa does

gm2_nbshrf_1, `if (sc.w > 0.0 && sc.x < sc.z) s = 0.5;` and then
`vec4(s, s, s, 1.0)`:

```
LD_VAR_BUF_IMM.f32 ... @r0:r1:r2:r3                 sc, fp32
FADD.f32.discard r1.h0, r3^, 0x0.neg                sc.w to fp16
FCMP_OR.f32.lt.m1 r0, r0^, r2^, 0x0                 sc.x < sc.z, fp32
FCMP_OR.v2f16.lt.m1 r1, 0x0, r1^.h00, 0x0           0.0 < sc.w, fp16
LSHIFT_AND.v2i16 r0, r1^.h00, 0x0.b00, r0^          the &&, 16 bits
MUX.v2i16 r0, 0x3C000000.h11, 0x38000000.h11, r0^.h00   s
IADD.v2u16 r1, r0.h00, 0x0 / MKVEC.v2i16 r2, r0^.h0, u2.w0.h0   (s, s), (s, 1.0)
```

1. A compare of two narrowed varyings' lanes goes back to fp32. The late
   rule `flt(f2fmp(a), f2fmp(b))` -> `flt(a, b)` (nir_opt_algebraic.py:
   4185-4186) leaves a 32-bit mask. The rule does not apply to a compare
   against a constant (`0.0 < sc.w`), which stays fp16.
2. `pan_nir_lower_bool_to_bitsize` gives the `iand` the width of its
   first source, here the 16-bit mask. The 32-bit mask is read through
   `i2i16`, which costs no instruction: `LSHIFT_AND.v2i16` reads the
   word.
3. The order: the fp32 compare comes before the fp16 one. Mesa runs
   `nir_move_comparisons` twice (at the link and at the compile). Two
   compares that share a first user swap on each run, so they normally
   end in their original order (`nir::move_compares`). By the second run
   the bool lowering has put `i2i16(b)` right before the `iand`. `b`'s
   first user is then the conversion, so `b` stays where the first run put
   it, before `a`.
4. `nir_opt_peephole_select` flattens the `if` into `bcsel_pan(m, 0.5,
   1.0)`, one `MUX.v2i16`. The colour reads it replicated, the
   `IADD.v2u16 .h00`, and beside the constant 1.0.

gm2_nbshrf_2 (`d < sc.z` with `d` a texel, the select scaling `col.rgb`)
has two fp16 compares. `sc.z` and `sc.w` are narrowed as one pair
(`V2F32_TO_V2F16 r1, r4^, r5^`), the compares and the `LSHIFT_AND.v2i16`
are in source order, and the select feeds `FMA.v2f16 r1, r0.h00, r1^,
0x0.neg` for `(r, g)` and an `FMA.v2f16` of the narrowed `b`.

## The lowering (src/shaderc/online/mali/nir_fs.rs)

- `Fs::and16` (the `Iand` arm of `Fs::int_lane` for two 16-bit masks):
  - A second compare of two varyings' lanes (`Fs::varying_cmp16`) is made
    a 32-bit compare where it stands, since nothing reads it yet. Neither
    lane is then narrowed for it.
  - The `iand` is a 16-bit node.
  - Refused, as forms no reference draws:
    - such a compare first, which would give a 32-bit `iand`;
    - a compare already read;
    - an `iand` of anything other than two compares (a chain `a && b &&
      c`);
    - two compares sharing an operand in the same place
      (nir_opt_algebraic.py:1158-1165's `fmin`/`fmax`). The one rule
      `Fs::and_masks` already transcribes is kept.
- `walk_if16` flattens an `if` whose condition is such a 16-bit `iand`, as
  it does one on a 16-bit compare.
- `bool_colour` also admits a colour lane that is a 16-bit select of two
  constants on such an `iand`, at most one distinct select. That is the
  route of gm2_nbshrf_1 (`select_highp`). Two different selects on one
  mask (`vec4(o0, o1, ..)`) failed an invariant in the selection
  (`src32`) and are refused before it.
- `resized_and_order` runs after the two `move_compares` runs. It applies
  step 3 above: `a, b, iand` becomes `b, a, iand` when `a` is a 16-bit
  compare and `b` a 32-bit one.
- `lane_tree` (the pair graph, gm2_nbshrf_2's route since the shader has a
  look-up) gets an arm for a select of two constants on a 16-bit `iand` of
  two 16-bit compares: each compare one lone node, the `iand`, then the
  `MUX`. A 32-bit compare in the `&&` is refused there.

## Results

- `tests/mali_bool_and.rs`: gm2_nbshrf_1 and gm2_nbshrf_2 are byte-equal
  (words, facts, FAU), drawn with and without `--uniform-values` (neither
  has a uniform). Two guards check refusals: the fp32 compare first, and
  two selects on one `&&`.
- Every reference under tools/mali/testdata (1,737 pairs) was compiled
  before and after: only gm2_nbshrf_1 and _2 change, both from refused to
  byte-equal.
- `tools/mali/corpus-verdicts.txt`: 262 ok before and after. Three
  refusals stop later, with another text:
  - es100/cond_complex.frag stops at FS_OPERAND, two selects on one `&&`;
  - es100/loop_full.frag stops at the mediump loop ladder;
  - es100/tri_discard.frag stops at the mediump discard.
- glmark2 (`tests/fixtures/mali_glmark2_verdicts.txt`, glmark2 22c527cb):
  shadow fs cf4b, refused at INT_VALUE (this `&&`) on main at 692e2a63,
  now stops later, at FRAGCOLOR_LANE ("a gl_FragColor lane other than the
  varying's own lane or a constant"): its colour `vec4(shadow *
  Color.rgb, 1.0)` on the select. Scenes stay at 23 of 33 compile-clean. The same
  write also records two lines main had already moved without the
  fixture (terrain fs 4d26 and c549, ARITH to the texture look-up
  refusal).

Left: a 32-bit `iand` (the fp32 compare first); `&&` chains; two selects
on one mask; the shared-operand rules; the same fp32 rule for a compare of
two varyings' lanes outside an `&&` (the walk still makes `if (sc.x <
sc.z)` alone an fp16 compare; no reference yet).
