# Mali: a uniform first in `mix` (plan #367 lower-nonfinite-const, step fsoperand)

jellyfish fs `8ee5` (glmark2's gradient, `mix(color1, color2, uv.x *
uv.y)` of two vec3 uniforms in a mediump shader) was refused at
FS_OPERAND: "a fragment shader operand of a form or in a place no reference
draws". The vectoriser records each operand form in each place it makes
(`nir_fs.rs` `prod`). Anything outside `ADMITTED` is refused until a
reference draws it, and `the_fragment_productions_are_the_references`
(src/shaderc/online/mali/tests.rs) keeps `ADMITTED` equal to what the
listed references make.

## What Mesa does (gm2_fsop_4)

The front end's `mix(a, b, t)` is `ffma(t, b + fneg(a), a)`. Mesa's program
for `vec4(mix(color1, color2, uv.x), 1.0)`:

```
FADD.f32.discard r0.h0, r0^, 0x0.neg              uv.x to fp16
FADD.v2f16 r1, u2.w1, u2.w0.neg                   color2.xy - color1.xy
FADD.v2f16 r2, u3.w1.h00, u3.w0.neg.h00           color2.z - color1.z
FMA.v2f16 r1, r0.h00, r1^, u2.w0
FMA.v2f16 r0, r0^.h00, r2^.h00, u3.w0.h00
```

The difference has a uniform first in a sum (`Uniform` at `AddA`) and a
uniform under the negation, as its source modifier (`Uniform` at `NegSrc`).

## The lowering

Both productions are admitted, and gm2_fsop_4 joins the production test's
list. Nothing else changes. Admitting `Op` second in a sum as well (for
gm2_fsop_5's `color1 + vec3(uv.x * uv.y)`) was tried and left out. It lets
deqp_arith_matrix_storeout_fs_mediump, deqp_posvary_flat_sep_kau_bs26 and
gm2_nonfin_3 compile with differences from their references. Mesa fuses
the product into each lane pair's sum, which the walk's fusion does not.

## Results

`tests/mali_fs_operand.rs`:

- gm2_fsop_4 is byte-equal, and so are fsmix_uniforms; _2, _3 and _6.._9
  match as before.
- gm2_fsop_1, the mediump gradient itself, compiles in another order: Mesa
  has both differences before both multiply-adds, while the pair graph
  orders one key's nodes by lane pair (every node of the gradient has the
  key of the colour's store). It is an ignored match test.
- gm2_fsop_5 stays refused.

glmark2: `8ee5` compiles. Jellyfish stays refused at vs `08a2`
(ARITH_CONST, constprod-rest), so the clean scenes stay at 22 of 33.
`tools/mali/corpus-verdicts.txt` does not move.

Left: the order rule (nodes of one key ordered by their IR operation
before the lane pair), which makes gm2_fsop_1 and so 8ee5 byte-equal; the
fusion of a product two lane pairs read (gm2_fsop_5).
