<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall fragment stage: a mediump ES 3.00 shader's arithmetic on gl_FragCoord (gl_test case 218)

Date 2026-11-08, integration tip `3895ebca`; refusals tightened 2026-11-09 on
main `c929e14c`. Plan #336 item `fs-fragcoord16`, issue #33. Host only, no
board run. The two references of section 2 were drawn on 2026-11-08 with
Mesa 26.2.1 on drm-shim (`tools/mali/drm_shim_ref.sh`), by the worker that
made the first change, whose shim was built into its
`$CONNECTED_DEPS_DIR/mesa-shim`. The worker of the 2026-11-09 change has no
drm-shim build (its `$CONNECTED_DEPS_DIR` holds `mesa-pan` with `pan_shc`
only), so the shapes section 4 refuses since then were not drawn. `D` = `tools/mali/testdata`,
`M` = Mesa's `src/compiler/nir/`, `B` = Mesa's `src/panfrost/compiler/bifrost/`.
Earlier designs: `mali-fragcoord-mediump.md` (the ES 1.00 mediump read, the
ES 3.00 highp shaders) and `mali-highp-fragcoord.md` (the fp32 position build).

## 1. The case and the refusal

gl_test case 218 shadowcmp (`driver/tools/pvrlab/gl_test.c`, `case_shadowcmp`)
draws with this fragment shader:

```glsl
#version 300 es
precision mediump float;
uniform mediump sampler2DShadow u_near;
uniform mediump sampler2DShadow u_lin;
out vec4 o_color;
void main() {
    float right = step(32.0, gl_FragCoord.x);
    vec2 b = vec2(floor((gl_FragCoord.x - 32.0 * right) / 8.0), floor(gl_FragCoord.y / 16.0));
    float n = texture(u_near, vec3((b + 0.5) / 4.0, 0.5));
    float l = texture(u_lin, vec3((b + 1.0) / 4.0, 0.5));
    float r = right > 0.5 ? l : n;
    o_color = vec4(r, r, r, 1.0);
}
```

The 2026-11-07 sweep (`tools/cts/log/2026-11-07-t6-step4a`) refused it with
`MaliError::FragCoordMediump`. The refusal text says "fp16 arithmetic", but the
shape is a different one: a **mediump ES 3.00** shader. There gl_FragCoord is
highp (`G/builtin_variables.cpp:1333-1335`), so every operation on it has a
highp operand and lower_precision leaves it at fp32 (`lower_precision.cpp:295-312`).
Only the stores to the mediump locals `right` and `b` narrow. The walk refused
the read itself (`nir_fs.rs`, the third `IoSlot::FragCoord` arm: "a mediump ES
3.00 shader's highp read has no reference").

## 2. References

| Files (`D/`) | What |
|---|---|
| `fcfp16_218_shadowcmp.{vert,frag,expect.json,stderr,stdout,trace}` | the case's own program pair (its `vs_src` and `fs_src`, byte copies) |
| `fcfp16_218_coord.{vert,frag,expect.json,stderr,stdout,trace}` | the same position arithmetic, the look-ups replaced by the coordinates they take: `o_color = vec4((b + 0.5) / 4.0, (b + 1.0) / 4.0)`. This one is pinned byte for byte |

Each was drawn once with gl_ref.py's defaults (64x64 RGBA8 target; ES 3 context
from the `#version 300 es` line; gl_ref.py binds no texture to a
`sampler2DShadow`, so the case's descriptors are the shim's defaults, not a
statement about a depth texture). To reproduce:

```
tools/mali/drm_shim_ref.sh build mesa-26.2.1.tar.xz $CONNECTED_DEPS_DIR/mesa-shim
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim D/fcfp16_218_coord.vert D/fcfp16_218_coord.frag D/fcfp16_218_coord
python3 -I tools/mali/pan_ref_diff.py extract D/fcfp16_218_coord.trace --draw > D/fcfp16_218_coord.expect.json
```

(and the same for `fcfp16_218_shadowcmp`). A third shape was drawn and dropped:
`vec2 r = right > 0.5 ? l : n` on the coordinates. NIR moves that select into
the added constants (`bcsel(c, 1.0, 0.5)` then one `FADD.v2f16`), which the
case, whose select is of two texels, never meets.

## 3. Mesa's program (fcfp16_218_coord.stderr)

```
U16_TO_F32.discard r0, r59.h0          ; x = u2f32(pixel.x) + 0.5
U16_TO_F32 r1, r59^.h1
FADD_IMM.f32 r0, r0^, #0x3F000000
FADD_IMM.f32 r1, r1^, #0x3F000000
FMA.f32 r1, r1^, u2.w0, u2.w1          ; y, gl_FbWposYTransform
FCMP_OR.f32.ge.m1 r2, r0, u3.w0, 0x0   ; step's compare: x >= 32.0
MOV.i32 r3, u4.w0
CSEL.u32.eq r2, r2^, 0x0, u3.w1, r3^   ; c ? -32.0 : -0.0
FADD.f32 r0, r0^, r2^                  ; x - 32.0 * right
FMA.f32 r0, r0^, u4.w1, 0x0.neg        ; / 8.0 as * 0.125
FROUND.f32.rtn r0, r0^                 ; floor
FMA.f32 r1, r1^, u5.w0, 0x0.neg        ; y * 0.0625
FROUND.f32.rtn r1, r1^
FADD.f32 r0.h0, r0^, 0x0.neg           ; f2f16 at `vec2 b = ...`, both after both floors
FADD.f32 r1.h0, r1^, 0x0.neg
MKVEC.v2i16 r0, r0^.h0, r1^.h0         ; b, one fp16 pair
FADD_IMM.v2f16 r1, r0, #0x38003800     ; b + 0.5
FMA.v2f16 r1, r1^, 0x34000000.h11, 0x0.neg   ; * 0.25
FADD_IMM.v2f16 r0, r0^, #0x3C003C00    ; b + 1.0
FMA.v2f16.wait0126 r0, r0^, 0x34000000.h11, 0x0.neg
ATEST, the moves, BLEND.v4.f16
```

The FAU words (expect.json `fau`, from `u2`): `u2` = (1.0, 0.0), the
y transform; `u3` = (32.0, -0.0); `u4` = (-32.0, 0.125); `u5.w0` = 0.0625. The
`CSEL.u32.eq` takes -0.0 where the mask is 0 (x < 32) and -32.0 otherwise.

How NIR gets there (each rule is cited in `nir_fs.rs`, `Fs::alu_pos32`):

* `step(32.0, x)` is the front end's `b2f(inot(flt(x, 32.0)))`.
  `inot(flt(a, b))` is `fge(a, b)` (`M/nir_opt_algebraic.py:738-739`), so
  `FCMP_OR.f32.ge`.
* The narrowing of `right` (`f2fmp(b2f32(c))`) is exact. Bifrost lowers `b2f`
  to `bcsel_pan(c, 1.0, 0.0)` (`B/bifrost_nir_algebraic.py:128`). The product
  by the constant folds into the select (`M/nir_opt_algebraic.py:3430`), and
  so does its negation (`:1126`). What is left is `fadd(x, bcsel(c, -32.0, -0.0))`,
  all fp32.
* The divides are products by the reciprocal; `floor` is `FROUND.rtn`.
* The constructor `vec2(floor(..), floor(..))` stores into a highp temporary.
  The assignment to the mediump `b` narrows both lanes (`f2f16` x2) after
  both floors, then `vec2` of the halves. `b + 0.5` and `* 0.25` stay
  `vec2` operations at 16 bits (`bi_vectorize_filter`, width 2).

The case's own program (`fcfp16_218_shadowcmp.stderr`) is the same up to the
`MKVEC`. Then each coordinate pair is widened (`F16_TO_F32` of each half),
collected with the reference 0.5 (`u5.w1`) and looked up by
`TEX_SINGLE.slotN.skip.reserved.16.r.2d.shadow.computed`. The select
`right > 0.5 ? l : n` is the `fge` itself: `flt(0.5, b2f(c))` is
`bcsel(c, flt(0.5, 1.0), flt(0.5, 0.0))` (`M/nir_opt_algebraic.py:1050`),
which is `c`, read through `i2i16` by `MUX.v2i16.int_zero r0, r1^.h00, r0^.h00, r2^.h00`.
The colour is `IADD.v2u16 r1, r0.h00, 0x0` (r, r) and `MKVEC.v2i16 r2, r0^.h0, u7.w0.h0`
(r, 1.0), `BLEND.v4.f16`.

## 4. The lowering (nir_fs.rs, mod.rs)

* **The read.** In a mediump ES 3.00 shader, gl_FragCoord is `Fs::frag_coord`
  (the fp32 build of a highp shader). Its x and y are marked in `Fs::pos32`,
  one bit per node.
* **Arithmetic** whose operands are such values or constants is
  `Fs::alu_pos32`, one fp32 node per lane. The forms are the ones listed
  above: `Bnot` of `flt` as `fge`, a `b2f` of that `fge` of the position
  and a constant (`step(k, x)`), a product of the `b2f` by a constant as a
  select of two constants, `fneg` of such a select as the select of the
  negated constants, a compare of a constant with a `b2f` as the condition.
  Compares, sums, products and `floor` go through the highp walk's
  `lane32`. The results are marked too. An operand of another kind (fp16, a
  uniform, a varying), another operation (`fract`, `abs`, `fneg` of the
  position, a `b2f` of an `flt` or of a constant first, a select on `step`),
  or gl_FragCoord's z or w is `FragCoordMediump`.
* **A named (declared) mediump local** narrows a marked value (`Fs::narrow_pos32`:
  `f2f16`, `FADD.f32 .h0`). `step`'s `b2f` is kept at fp32, since its
  narrowing folds away. A constructor's temporary (no name) keeps the fp32
  value, as GLSL's rvalue is highp. This is what puts both `f2f16`s after both
  floors. Two narrowed lanes of an aligned pair become one `vec2`
  (`MKVEC.v2i16`), the value Mesa's 16-bit `vec2` operations read. The pair
  is x then y, each lane computed from its own coordinate only
  (`Fs::pos_axes`).
* **Values made from the pair** (`Fs::made_of_pos`: the narrowed lanes, the
  `vec2` and every node with such an operand) go to no declared local: `vec2
  c = b`, `vec2 c = b * 0.5`, `c.x = b.y`, `vec3 c = vec3(b, 1.0)` and `b =
  b * 0.5` are `FragCoordMediump` at the store. Before this check those
  shapes reached `select_highp` and failed there as `Defect::IrIndex`.
  Constructor temporaries (no name) are the expression's own value and are
  checked where they are stored or read.
* **The colour** of such values is the reference's form only: stored once
  to the first target, each half one vector operation `(b + k) * c` (two
  constants, the divide a product) over the pair's lanes in order
  (`Fs::pos_lane`).
* **The route's form** (`pos16_form`, mod.rs): no `if`, no discard, no
  look-up, and every fp16 node one of the pair's (`f2f16`, `vec2`, the sums
  and products).
* **The program** goes through `select_highp`, as a ladder does (`pos16`,
  mod.rs). There `vectorize16` pairs the fp16 lanes (`b + 0.5` over the
  `vec2`'s two lanes). The colour's lanes are read again from their `Store`
  nodes after the pairing: a paired lane is now the pair's lane 1.

Pinned: `tests.rs`, `the_case_218_position_arithmetic_is_byte_equal_to_mesa_26_2_1`
(every word, the register count, preload and FAU words of `fcfp16_218_coord`).

Taken without a reference of their own, because they are the reference's
operations with other constants or in the other order of a sum: the form
without `step` (`vec2(floor(x / 8.0), floor(y / 16.0))` and the same
colour), other constants in the divides, the `floor` operands and the
colour's `(b + k) * c`, and `x + 32.0 * right` (a `CSEL` of 32.0 and 0.0).

Refused with `FragCoordMediump`, because no reference pins them. The first
four were drawn on the shim on 2026-11-08 (Mesa's order or vector shapes
differ from what the walk would build); the rest are refused because their
operations are not the reference's, and were not drawn:

* a narrowed pair whose two lanes come from one IR operation (`vec2 p =
  gl_FragCoord.xy * 0.5`): Mesa builds y before x;
* a narrowed lane alone, or beside a lane of another kind;
* a look-up whose coordinate is the fp32 position or the narrowed `vec2`
  itself (`texture(s, gl_FragCoord.xy / 64.0)`);
* the fp32 position stored to the colour without a mediump local first;
* the pair y then x, or a lane computed from both coordinates;
* the pair or fp16 arithmetic on it stored to a declared local (above);
* a colour of the pair beside constants (`vec4(b * 0.5, 0.0, 1.0)`), of the
  pair itself, of `b * k` alone, of a chain of constants (`b * 0.5 + 1.0`,
  the colour `* 2.0`), of a swizzle, of two operations' lanes, of a
  negation, or a second store to the colour (`o_color.w = 1.0`);
* an `if`, a discard, a select on the pair, a look-up anywhere in the
  program, or a look-up at fp16 arithmetic on the pair with an ordinary
  sampler (a shadow sampler's is the next gap, `TexKind`, section 5).

Each is a `gap_cases` row in `tests.rs`; so is case 218's own program, at
`TexKind`.

## 5. What case 218 meets next

With this change, case 218's program stops at its shadow look-ups:
`MaliError::TexKind` ("a texture look-up other than texture2D(sampler2D,
coordinate) ..."). The front end hands `ir::Op::Tex { shadow: true, .. }`, and
no fragment arm takes `shadow: true`. What the next item needs, from
`fcfp16_218_shadowcmp.expect.json`:

1. The coordinates: `F16_TO_F32` of each half of the fp16 pair (the
   `f2f32` that `nir_lower_tex` leaves for a highp coordinate of a mediump
   look-up), and the reference 0.5 as a third staging word (`MOV.i32 r6, u5.w1`).
2. `TEX_SINGLE ... .16.r.2d.shadow.computed`: a one-lane (`.r`) fp16 result
   with the shadow flag, the write mask of one texel.
3. The select of two texels on the 32-bit `fge` mask: `MUX.v2i16.int_zero`
   with the mask's low half (`i2i16`, `pan_nir_lower_bool_to_bitsize`).
4. The colour `vec4(r, r, r, 1.0)`: `IADD.v2u16 x.h00, 0x0` (the `MKVEC` of a
   lane twice, `lower_mkvec_swz`) and `MKVEC.v2i16 r, <1.0>`.
5. The frame side: a `GL_DEPTH_COMPONENT32F` texture and the sampler objects'
   compare mode and function in the descriptor (`docs/notes/mali-texture-refusals.md`,
   the depth texture). gl_ref.py binds no depth texture today, so the
   reference's descriptor words do not cover it.

## 6. Caveats

* The case's own program is not pinned. Its first half is
  `fcfp16_218_coord`'s, but nothing compiles the whole program until item 5's
  look-up exists.
* `inot(flt)` to `fge` needs both operands to be numbers (`is_a_number`).
  The position and a finite constant are. A compare of a computed NaN is not
  reachable from this shape.
* case 218's shader is not in `tools/shaders`, so `corpus-verdicts.txt` and
  `sweep_predict.py` do not see it (its GLSL is inline in gl_test.c, no
  `gl_test_*_src` symbol). No corpus verdict moved with this change.
* Nothing here was checked on the board.
