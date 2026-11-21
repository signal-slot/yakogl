<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): look-ups beside another at a mediump computed coordinate (TEX_MULTI)

**Status: gm2_texmulti_1..3 and gm2_constfold_1..3 byte-equal; gm2_texmulti_4..6
stay refused** (plan #367 lower-fma, issues #32/#33). The TEX_MULTI family of
docs/notes/mali-glmark2-forecast.md: the Mali online compiler
(`src/shaderc/online/mali/`) took several fragment look-ups only at a vec2
varying read whole or two constants (`MaliError::TexCount`, "a fragment
shader texture look-up beside another in a form only a lone look-up takes").
glmark2's desktop blur passes (fs 68af, 7f17), the effect2d kernels and
terrain's tilt-shift (fs 8c86, bbba, bdfd, e043) sample one texture at the
varying plus offsets. The references are Mesa 26.2.1 on drm-shim
(`tools/mali/testdata/gm2_texmulti_*`, drawn with `--uniform-values`, from
plan #367 results-row, 2e6b574c; `gm2_constfold_*`, without, from
lower-const-fold). The host test is `tests/mali_tex_multi.rs`; the pan_shc
verdicts are in `tools/mali/testdata/cases.txt`. Background:
mali-texture-lookups.md (several look-ups, their handles and staging),
mali-texture-refusals.md (the family list), mali-tex-swizzle.md (the
look-up's texel words).

## 1. What Mesa draws

gm2_texmulti_2 (`texture2D(s, v + vec2(-0.25, 0.0)) * 0.375 +
texture2D(s, v + vec2(0.25, 0.0)) * 0.625`), the final listing:

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
V2F32_TO_V2F16 r0, r0^, r1^                        the varying converted once
FADD_IMM.v2f16 r1, r0, #0xB400                     tap 1: v + (-0.25, 0.0), one vec2 op
F16_TO_F32 r2, r1.h0 ; F16_TO_F32 r1, r1^.h1      both halves widened
MOV.i32 r3, r1^ ; MOV.i32 r4, u2.w0 ; MOV.i32 r5, 0x0
TEX_SINGLE.slot1.skip.reserved.16.rgb.2d.computed @r2:r3, @r2:r3, [r4^:r5^]
FADD_IMM.v2f16 r0, r0^, #0x3400                    tap 2: v + (0.25, 0.0)
F16_TO_F32 ... TEX_SINGLE.slot2 ...
FMA.v2f16.discard r0, r0^, u2.w1, 0x0.neg          the colour over the texel words
FMA.v2f16 r0, r2^, u3.w0, r0^ ...
```

- **Each coordinate is one fp16 vector operation.** GLSL's `v + vec2(c,
  0.0)` stays a vec2 `fadd` in NIR: `nir_lower_alu_width` keeps 16-bit
  vectors of two (`bi_vectorize_filter`), and `fadd(a, 0.0)` is not folded
  per lane (the rule needs the constant in every lane it reads,
  nir_search.c). So `x + 0.0` survives (`#0x34000000` of gm2_texmulti_1) and
  two taps that share a lane (`y + 0.0` of both taps of gm2_texmulti_2) are
  two operations. Our walk is lane-wise and CSEs the shared lane into one
  node, which `vectorize16` would pair once only.
- **The varying is converted at its first fp16 reader.** lower_precision
  puts the `f2fmp` in the expression that reads the varying; a look-up at
  the varying itself before it reads the fp32 load as it is (gm2_texmulti_1:
  the first `TEX_SINGLE @r0:r1`, then `V2F32_TO_V2F16 r0, r0, r1`; in _3 the
  first tap is computed, so the conversion follows the load).
- **The look-ups stand in program order among the nodes**, each after its
  coordinate's operation and widening.
- **The colour is fp16 pairs over the look-ups' words**: `FMA.v2f16 r0,
  r0^, u2.w1, ..` reads the look-up's first word whole.
- A centre tap at `v + vec2(0.0, 0.0 * k)` is the varying itself (GLSL IR
  folds an add of a zero vector), read as loaded (gm2_texmulti_3's
  `TEX_SINGLE.slot0 ... @r0:r1, @r0:r1`).

## 2. The lowering (`nir_fs.rs`)

A mediump shader whose straight-line code makes several 2D look-ups, one at
least at a computed coordinate (`multi16`), takes the highp route
(`select_highp`, as `bool_colour` and the kept branches of `kept16` do;
`mod.rs` `compile_stage`):

1. `convert_inputs16` converts the varying lanes the fp16 operations read
   (`V2F32_TO_V2F16` of a pair).
2. `pair_coords` moves each such conversion to just before its first reader
   in the order, then makes each computed coordinate the look-up's own
   two-lane operation (`coord_pair`, recursive: two constants one word of
   fp16 halves, an aligned pair of one value its word, two alike one-lane
   operations a new two-lane one over their sources' pairs), inserted before
   the look-up. The walk's shared lanes are left to the dead code.
3. `multi_forms` admits only what the references draw: each coordinate
   `fadd(<converted pair>, <constant pair>)`, the colour sums and products
   of texels and constants. Anything else is `TexCount` as before; a
   product of a constant product with another constant (gm2_constfold_4's
   `t * K1 * K1`, which Mesa folds to one inexact constant) is the
   constant-product gap, `ArithConstant`.
4. `vectorize16` pairs the colour's lanes; two texels of one look-up word
   are read through a `vec2` of that word (`Fs::vec16`'s form, no
   instruction).
5. The colour is re-read from its stores after the pairing (a lane paired
   is the pair's half). The look-ups are node events in program order
   (`inline`), and `move_loads` counts a look-up at the varying as the
   load's reader, so the load stays before it.
6. `tex_highp` widens both halves of the coordinate (`F16_TO_F32`) into the
   staging words, as for cflow_148_texbranch.

Byte-equal: gm2_texmulti_1, _2, _3; gm2_constfold_1 (desktop blur's
separable pass, 7f17's shape), _2 (the effect2d Laplace kernel, 3c3d), _3
(its -0.0 weight).

## 3. What stays refused

| Reference | Shape | Mesa | Why not here |
|---|---|---|---|
| gm2_texmulti_4 | 7 taps stepped by a mediump uniform | `(3u, 2u)` as one `FMA.v2f16` of `(3.0, 2.0)` by `u.h00`, `vec2(3u, 0)` by `U16_TO_U32`, the tap `FMA.v2f16 (3u, 0), (-1, 1), v` | `vec2(fneg(a), 0.0)` folded into an `ffma` by `(-1, 1)`, and products of two taps paired: not in the walk's rules |
| gm2_texmulti_5 | terrain's tilt-shift, the step from `v_uv.y`, 5 vertical taps | `abs`, a product pair, `MKVEC.v2i16 0x0.h0, s.h1`, taps as `FMA.v2f16` | as _4, plus the step's arithmetic |
| gm2_texmulti_6 | the tilt-shift at tilt 0.0, 3 horizontal taps | the centre tap (at the varying) scheduled first, the `-s` tap last | as _5, plus Mesa's scheduler reordering the look-ups |
| gm2_constfold_4 | the non-separable blur, `t * K1 * K1` | the two constants folded into one (inexact in fp16) | the walk keeps both products when their product is inexact (`Fs::op`, deqp_constop_op_degrees): ARITH_CONST |

So desktop blur's separable passes and effect2d's Laplace kernel compile;
terrain's four tilt-shift shaders and the non-separable blur stay refused.
The next step for terrain is the `vec2(fneg(a), 0.0)` → `ffma(vec2(a, 0.0),
(-1, 1), v)` form of gm2_texmulti_4 and the step arithmetic of _5, then the
look-up order of _6.
