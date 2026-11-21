<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): lowering the TexVertex, TexKind and FragCoordMediump refusals to Mesa's words: design

**HOST-ONLY design.** Plan #387 item `texmisc-lower-design` (issue #7, ROADMAP
stage 5; issue #32 for the texture look-up kinds). No change under `src/`,
`tests/` or `tools/` is part of it. It builds on
[`mali-deqp-texmisc.md`](mali-deqp-texmisc.md) (the 17 references
`tools/mali/testdata/deqp_texmisc_*`, the 144 cases of its section 1, the
counts of its section 4) and on the 7 `deqp_fmaloc_tk_*` references of
[`mali-deqp-fmaloc.md`](mali-deqp-fmaloc.md). Counts are that note's
first-refusal cases at `ca7cdc52` (HOST-ONLY), never a render verdict. The
G310 needs no separate work ([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)).

What this note adds:

1. each reference's Mesa 26.2.1 words, read from the `text` of the
   `vs.varying` / `fs.fragment` regions of its `expect.json` (section 2);
2. the refusing sites **at the current tip** (`c0142485`, read, not
   instrumented: section 3; the line numbers of the reference note's section 3
   have drifted by 15-30 lines);
3. the mechanisms, the one-session steps ranked by cases, the tests at risk,
   the overlaps and a forecast per step (sections 4-8).

## 0. Summary

1. **The schedule's T2 is not a TexVertex row and nobody owns it.**
   `mali-lowering-schedule.md` 2.10 row T2 ("TEX_VERTEX projection 11, 2D bias
   15, cube bias 1 = 27", owner "no item yet") names the *board's* family
   label `TEX_VERTEX`; the text behind it is `MaliError::TexKind`, raised by
   the **fragment** walk (it is the 27 `deqp_fmaloc_tk_*` cases, 42 at the tip
   with the 15 new random ones). `mali-fmaloc-lowering.md` designs TexCoord and
   FusedMultiplyAdd only and says so in its section 6 ("the `tk_*` references
   ... are `TexKind`"). So steps TK1-TK3 below are **not** a duplicate of
   anything in fmaloc, and `fmaloc-lower-1` / plan 367's `lower-fma` do not
   cover them. The text `TexVertex` (vertex shaders, 60 cases) is a different
   shape and is covered by no earlier design.
2. **TexVertex is `intval-vs16`'s mechanism M4 over texels, not a new
   shape.** The 60 cases are `uniform_api`'s `compare_vec4` harness; Mesa folds
   it to one `FMAX.v2f16` chain and one `FCMP_OR.v2f16.lt` exactly as for the
   uniform words of `vsapi_mat2` (`mali-intval-vs16-lowering.md` 4.4 and M4,
   "a vertex `cmp_product` ported from the fragment walk", ordered last and
   "a large port"). Build the port once, with texel operands in the same
   session as M4 or right after it (step TV1).
3. **FragCoordMediump is an int round trip in fp16 raised under the wrong
   name.** None of the 42 reads `gl_FragCoord`. Its refusing site per shape is
   still **unconfirmed** (section 3.3); step FC0 is the one-time measurement
   that fixes it, as `fclane-design` did for `ColourForm`.
4. **Ranked steps** (section 5): TK1 (21, no dependency, fragment only), FC1
   (32, after FC0), TV1 (60, after `intval-vs16` M4), TK2 (10), TK3 (11), FC2
   (10). By count TV1 is first, by readiness TK1 is first; the table gives both.

## 1. Cases per shape (restated from the reference note, with the step each goes to)

| Shape (reference `deqp_texmisc_*` / `deqp_fmaloc_*`) | Cases | Text | Step |
|---|--:|---|---|
| `tv_2d_compare` 14, `tv_rnd_2d` 7, `tv_2d_array_compare` 22 | 43 | TexVertex | TV1 (2D part) |
| `tv_cube_compare` 14, `tv_rnd_cube_array` 3 | 17 | TexVertex | TV1 (cube part, TV1c) |
| `fmaloc_tk_proj_vec3/4`, `fmaloc_tk_rnd_proj` (`.19`), `tk_rnd_proj_swizzle` (`.131`), plus `.23`, `.56` | 6 | TexKind | TK1 |
| `fmaloc_tk_bias_uniform/_varying`, `fmaloc_tk_rnd_bias` (`.48`), `tk_rnd_bias_fragcolor` (`.124`), `tk_rnd_bias_two` (`.41`, dead look-ups), plus 8 more bias-only cases | 14 | TexKind | TK1 |
| `tk_rnd_bias_proj` (`.108`, dead look-ups) | 1 | TexKind | TK1 |
| `tk_rnd_proj_sum` (`.142`), `.69`, `.94`, `.98`, `.80`, `.91`, `.97`, `.136` (projection of a built or computed coordinate); `.78`, `.99` (bias and projection) | 10 | TexKind | TK2 |
| `fmaloc_tk_cube_bias`, `tk_rnd_cube_bias` (`.121`), and 9 more cube cases | 11 | TexKind | TK3 |
| `fcm_int_to_float` 4, `fcm_ivec2_to_vec2` 13, `fcm_int_bool_to_vec2` 3, `fcm_int_to_mat2` 12 | 32 | FragCoordMediump | FC1 |
| `fcm_rnd_int_lanes` 9, `fcm_eval_order` 1 | 10 | FragCoordMediump | FC2 |

Total 144 (TexVertex 60, TexKind 42 = 21 + 10 + 11, FragCoordMediump 42). The
TexKind split is the reference note's step table (its section 4, steps 3-5)
regrouped by what each step needs.

## 2. Mesa's words, per shape (Mesa 26.2.1, G610 drm-shim)

Read from the `expect.json` disassembly; instruction numbers are the index in
the region. `r?` register numbers are Mesa's and are fixed by the existing
register allocator; the lowering matches the *forms*, the byte compare checks
the rest.

### 2.1 TexVertex (`vs.varying`; the fragment program is the 10-instruction pass-through)

`tv_2d_compare` (16 instructions), the harness of
`v *= compare_vec4(texture2D(u_var, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36))`:

```
 0-3  MOV.i32 r0, 0x0 ; MOV.i32 r1, 0x0 ; MOV.i32 r2, u5.w1 ; MOV.i32 r3, 0x0     staging (coordinate (0,0), descriptor)
 4    TEX_SINGLE.slot0.reserved.16.2d.zero.wait0 @r0:r1, @r0:r1, [r2^:r3^]         fp16 texels, explicit level 0
 5    FADD.v2f16 r2, r0.h11, u6.w0                 texel.w - w' and texel.y - y' as one pair (the constant pair is an FAU word)
 6    FADD_IMM.v2f16 r1, r1^, #0xB5C3B3AE          texels (z, w) minus the folded constants (an immediate pair)
 7-9  FMAX.v2f16 r0, r0^.abs.h00, r2^.abs.h00 ; FMAX.v2f16 r1, r1^.abs.h11, r1^.abs.h00 ; FMAX.v2f16 r0, r1^.h00, r0^.h00
                                                   the max of the four |distances|, one lane
 10   FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u6.w1, 0x0  max < 0.05 (FAU word 0x2A66)
 11   S16_TO_S32 r0, r0^.h0
 12   CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000    1.0 or 0.0
 13   FMA.f32 r0, r0^, 0x0, r0^                    the exact ffma(x, 0.0, x) of `v_vtxOut *= x` (vsapi_mat2 has the same word)
 14-15 LEA_BUF_IMM + STORE.i32.estream.end         the varying store
```

- `tv_2d_array_compare` (38): three look-ups in order, `slot0..slot2` (each its
  own `MOV` pair for the descriptor word; the coordinate registers re-zeroed),
  each followed by its own 5 words (`FADD.v2f16` / `FADD_IMM.v2f16`, three
  `FMAX`), and the three per-look-up maxima joined by two more
  `FMAX.v2f16 .h00`; **one** `FCMP_OR`, `S16_TO_S32`, `CSEL`, `FMA` for the
  whole array. The harness's `compare_vec4` constants repeat (`#0xB5C3B3AE`,
  `u6.w0`).
- `tv_rnd_2d` (16): as `tv_2d_compare` with both constant pairs immediate
  (`#0xB0CDB571`, `#0xB971B8E1`): no FAU word, the compare constant `u6.w0`.
- `tv_cube_compare` (25): the cube direction `vec3(0.0)` is **not folded**:
  `CUBEFACE1 r0, 0x0, 0x0, 0x0` / `CUBEFACE2_V9` / `CUBE_SSEL` / `CUBE_TSEL`,
  `FRCP.f32`, `FMA.f32 0x38000000.h1 * r0` (0.5/major), two
  `FMA.f32.clamp_0_1`, `MUX.i32.bit r1, r2^, r1^, u5.w1`, two `MOV.i32`,
  `TEX_SINGLE.slot0.reserved.16.cube.zero`, then the same 9 harness words
  (14-22).
- `tv_rnd_cube_array` (64): one cube sequence shared by four look-ups
  (`MOV.i32` pairs from r1/r0 for each `TEX_SINGLE ...cube.zero` slot0/1/2/0),
  each look-up's two `FADD_IMM.v2f16` plus three `FMAX`, and the **other
  uniforms'** compares (the sampler array's neighbours in the random program)
  joined into the same chain (`FADD_IMM.v2f16 r1, u5.w1, #0xC03D487B`, a
  `FADD.v2f16` of a `MOV.i32`-ed word); one `FCMP_OR` at the end (58). Its
  fragment shader (68) is the same harness on `cube.computed`, with a flat
  varying `LD_VAR_BUF_IMM ... src_flat32` for `v_vtxOut` and
  `MUX.v2i16 r0, 0x0, r1^.h00, r0^.h00` for `v *= result`.

How Mesa gets there (`nir_opt_algebraic`, the rules `mali-intval-vs16-lowering.md`
4.4 and `nir_fs.rs`'s `cmp_product` comment cite): `f2fmp(b2f32(a))` is
`b2f16(a)` (:1980); `fmul(b2f(a), b2f(b))` is `b2f(iand(a, b))` (:1679);
`iand(flt(a, c), flt(b, c))` is `flt(fmax(a, b), c)` (:1159); `|a - b|` of a
constant `b` becomes `|a - b|` with the constant folded into one
`FADD_IMM.v2f16` of a pair. What differs from the uniform-word case: the
operands are the two halves of a **texel word pair** (`r0` = (x, y), `r1` =
(z, w), the `TEX_SINGLE` result packed two a register) and the sub is of a
constant pair, so there is no `.abs.h00` of one word's two halves; the pair
builder is the texel pairing of `vs_tex_read16`.

### 2.2 TexKind (fragment, mediump; `fs.fragment`)

References and their words (the fmaloc ones are in `mali-deqp-fmaloc.md`
section 2: projection, uniform/varying bias, constant bias, cube bias):

| Reference | Instrs | Words that matter |
|---|--:|---|
| `tk_rnd_proj_swizzle` (`.131`) | 14 | `LD_VAR_BUF_IMM.v3`; `FRCP.f32 r2, r2^`; `FMA.f32 r0, r0^, r2, 0x0.neg` and `r1` (the two products with the divisor, no fused add); `TEX_SINGLE.slot1.skip.reserved.16.2d.computed @r0:r1, @r0:r1, [r2^:r3^]` with `MOV.i32 r2, u2.w0` / `r3, 0x0`; the `.ywzx` result swizzle is `MKVEC.v2i16.discard r2, r0.h1, r1.h1` + `MKVEC.v2i16 r1, r1^.h0, r0^.h0` |
| `tk_rnd_proj_sum` (`.142`) | 20 | `V2F32_TO_V2F16`; `FADD.v2f16 r1, r0.h11, u2.w0.h00`; `MKVEC.v2i16 r0, r0^.h0, u2.w1.h0`; `FADD_IMM.v2f16 r0, r0^, #0xC980C980`; `FADD.v2f16 r1, 0x3C000000.h11, r1^.h00`; three `F16_TO_F32`; `FRCP.f32`; two `FMA.f32 .. 0x0.neg`; `2d.computed` |
| `tk_rnd_bias_fragcolor` (`.124`) | 15 | `V2F32_TO_V2F16`, `FMA.v2f16 r0, r0^, r0^, 0x0.neg` (b*b), two `F16_TO_F32`, three `MOV.i32` of the staging words, **`MOV.i32 r4, u2.w0`** (the folded 8.8 bias, an FAU word), `TEX_SINGLE.slot1.skip.reserved.16.2d.computed_bias @r0:r1, @r2:r3:r4, ..` |
| `tk_rnd_bias_two` (`.41`) | 18 | no `TEX_SINGLE`: both look-ups' results are overwritten; the program is the colour arithmetic alone |
| `tk_rnd_bias_proj` (`.108`) | 7 | no `TEX_SINGLE`: `gl_FragColor = a`, two `V2F32_TO_V2F16` of the varying and the blend |
| `tk_rnd_cube_bias` (`.121`) | 24 | `FADD.f32 r0, r1^, r0^` (bias = c + b); the cube sequence with the constant `1.0` lane: `CUBEFACE1 r1, 0x3F800000, 0x3F800000, r2`, `CUBEFACE2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, `FMA.f32 0x38000000.h1`, two `.clamp_0_1`, `MUX.i32.bit r2, r4^, r3^, u2.w0`; the bias pack `FMA.f32.clamp_m1_1 r0, r0^, u2.w1, 0x0.neg` (1/16), `FMA.f32 r0, r0^, u3.w0, 0x0.neg` (4096), `F32_TO_S32.rtz`, `U16_TO_U32 r0, r0^.h0`; `TEX_SINGLE...16.cube.computed_bias @r0:r1, @r2:r3:r4` |

The pattern across them: the 2D `computed` and `computed_bias` look-ups differ
from the implicit-coordinate look-up the walk already builds only in the
**staging words** (coordinate lanes widened by `F16_TO_F32`, the bias word as
the third word) and in the `LodMode`; `emit.rs` already builds
`ComputedBias` for a cube (`nir_fs.rs:13029`, `bir.rs:262`).

### 2.3 FragCoordMediump (fragment, mediump, no `gl_FragCoord`)

| Reference | Instrs | Words that matter (the int round trip) |
|---|--:|---|
| `fcm_int_to_float` | 18 | `FADD.f32.discard r0.h0, r0^, 0x0.neg` (narrow the varying), `IADD_IMM.i32 r1, 0x0, #0x3C033C03` + `FMA.v2f16 r0, r0^.h00, r1^, 0x0.neg` (`* 1.0025`, fp16), `F16_TO_S32.rtz r0, r0^.h0`, `S32_TO_F32 r0, r0^`, `FADD.f32 r0.h0, r0^, 0x0.neg` (narrow the float back), then the `isOk` harness in fp16: `FADD.v2f16 r0, r0^.h00, u2.w0.neg.h00`, `FADD.v2f16 r1, u2.w0.abs.h00, 0x0.neg`, `FMA.v2f16 r1, u2.w1, r1^.h00, u2.w1`, `FCMP_OR.v2f16.ge.f1`, `IADD_IMM.i32 r1, 0x0, #0x3C003C00`, `MKVEC.v2i16`, `IADD.v2u16 r0, r0^.h00, 0x0` |
| `fcm_ivec2_to_vec2` | 24 | one `FMA.v2f16` for both lanes, two `F16_TO_S32.rtz` (`r0.h1`, `r0^.h0`), two `S32_TO_F32`, two `FADD.f32 .h0` narrowings, `MKVEC.v2i16 r0, r0^.h0, r1^.h0`, the harness at width two (`FADD.v2f16`, `FMA.v2f16`, `FCMP_OR.v2f16.ge.m1`), `all()` as `LSHIFT_AND.v2i16 r0, r0^.h11, 0x0.b00, r0^` + `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00` |
| `fcm_int_bool_to_vec2` | 22 | here `float(int(x))` is **`ftrunc`**: `F16_TO_F32 r1, r1^.h0`, `FROUND.f32.rtz r1, r1^`, `FADD.f32 r1.h0, r1^, 0x0.neg`; the bool lane `FCMP_OR.v2f16.ne.f1 r0, r0^.h11, 0x0, 0x0`; `MKVEC.v2i16 r0, r1^.h0, r0^.h0` |
| `fcm_int_to_mat2` | 22 | the `ftrunc` form, a zero lane `MKVEC.v2i16 r0, 0x0.h0, r0^.h0`, `U16_TO_U32 r1, r0.h0` to re-widen a half, two `FADD.v2f16`, `FMAX.v2f16 r0, r1^.abs, r0^.abs` |
| `fcm_rnd_int_lanes` (`.88`) | 11 | `F32_TO_S32.rtz`, `S32_TO_F32`, `FADD.f32 r0.h0`, `MKVEC.v2i16 r0, r0^.h0, u2.w0.h0`, `FADD_IMM.v2f16 r0, r0^, #0x42004000`; alpha `ATEST .. 0x0`, `MOV.i32 r1, u2.w1` |
| `fcm_eval_order` | 22 | the int words of the call: `F16_TO_S32.rtz` twice, `IADD.s32`, `IADD_IMM.i32 #0x1`, `FCMP_OR.v2f16.lt.m1` + `S16_TO_S32` + `CSEL.u32.eq r0, r1^, 0x0, 0xFFFFFFFF, r0^`, `ICMP_OR.u32.eq.m1 r0, r0^, u2.w0, 0x0`, `MUX.v2i16`, `MKVEC.v2i16 r1, r0.h0, u3.w1.h0` |

The two forms of `float(int(x))` (section 2.3 of the reference note): where
the int value has **no other reader**, `nir_opt_algebraic.py:1250`
(`i2f32(f2i32(a))` is `ftrunc(a)`) fires, and the words are
`F16_TO_F32` + `FROUND.f32.rtz` (+ a narrowing `FADD.f32 .h0`); where the int
is also read by an integer operation (`fcm_eval_order`) or by the harness as
an int, the words are `F16_TO_S32.rtz` + `S32_TO_F32`. The back end already has
both halves for the **fp32 walk**: `Fs::int_lane` takes `F2i` of an fp16 value
as `F2i32` (`F16_TO_S32.rtz`, `nir_fs.rs:2304`), the highp arm folds
`I2f(F2i(a))` to `Ftrunc` (`nir_fs.rs:3477`), and `narrowed()` accepts
`F2f16` of an `I2f` lane (`nir_fs.rs:2592`). What the references demand beyond
them is that the narrowed lane then enters fp16 arithmetic (`vec16_of`,
`alu16`) and the all-fp16 `isOk` harness.

## 3. The refusing sites at `c0142485`

### 3.1 TexVertex

| Site | What it refuses |
|---|---|
| `src/shaderc/online/mali/nir.rs:4650` (in the vertex walk's `IrOp::Alu(a, s)` arm, which starts at 4638; the guard is the `any` closure above 4650) | a `Prec::Medium` operation (other than the fp16 product/sum arms `vs_alu16` call at 4629 and `vs_alu16u` call at 4636) whose operand is a lane of a `Kind::VsTex { medium: true }` look-up. All 60 cases raise it here (**likeliest**, not instrumented): the harness's `fadd`, `fabs`, `flt`, `bcsel`/`b2f`, `fmax` chain. The look-up itself is taken by the arm ending at 5207 (`vs_tex_read16` used at 5184) |
| `nir.rs:5201` | a second look-up of one sampler, coordinate and level wanted at fp16 after one built at fp32 (`Some(k) if half && bits != 16`); not reached by these shapes (the array's three look-ups use three different sampler indices) |
| `nir.rs:5209` | `IrOp::Tex` with no level / `TexSize` in a vertex shader: ES 3 forms, no case here |

`vs_tex_read16` (the predicate behind `half`, `nir.rs:3471`) decides
whether a look-up's texels stay fp16: "every reader converts up or is a
mediump product or sum". The harness's readers are `fadd`/`fabs`/`fmax`/`flt`,
so `half` must also be true for **the compare-product reader** or the look-up
is built at fp32 (`FMA.f32`-width, 32-bit `TEX_SINGLE`), which is not Mesa's
(`TEX_SINGLE...16...`).

### 3.2 TexKind

| Site | What it refuses | Cases |
|---|---|--:|
| `nir_fs.rs:6785-6787` (the fallback `ir::Op::Tex { sampler, coord, .. }` arm, guard `matches!(op.es100_tex(), Some((Es100Tex::Tex2D, None)))`) | `Tex2DBias` and `Tex2DProj` of every precision; and any cube look-up that is neither the implicit mediump arm nor the bias arm below | TK1 21, TK2 10, the 2D ones of TK3 |
| `nir_fs.rs:6577` (the mediump `textureCube` bias arm, 6556-6577; `f.uniform_dword(bias).is_none()`) | a cube bias that is not a uniform lane: `texturecube_bias`'s varying bias (`fmaloc_tk_cube_bias`) and `.121`'s `c + b` sum | TK3 11 |
| `nir_fs.rs:6533` | an explicit mediump cube level (`textureCubeLod`-like): ES 3, no case here | 0 |
| `nir_fs.rs:3990`, `:6639`, `:6675`, `:6718`, `:6746`, `:6775`, `:11725`, `:12582` | ES 3 forms (cube size, levels, shadow, a size branch, look-up halves): no dEQP-GLES2 case reaches them | 0 |
| any `Tex` with a **dead** result | the walk raises at the Tex op *before* any liveness check, while Mesa's NIR drops the look-up (`.41`, `.108`, and the dead statements of `.131`) | 2 pinned, more unknown (TK1) |

### 3.3 FragCoordMediump

All sites are in `nir_fs.rs`:

| Site | Condition |
|---|---|
| `:2844` (`Fs::vec16_of`) | two halves made one fp16 vector where neither is a texel pair, a lane-and-constant half nor both `is_f16` |
| `:2878` | `alu16`'s `fc[k]` arm: a `gl_FragCoord` lane other than x or y |
| `:2965`, `:2970`, `:2974` (`alu16`'s pair arm, 2955-2975) | a pair with no texel or narrowed operand (`own(p) \|\| own(q)`), a mixed constant / non-constant second operand, or a non-`Fadd` second pair |
| `:3016-3020` (`alu16`'s tail) | one-lane arms: `Fmul` of a lone value other than a texel and a constant; the catch-all `_` |
| `:8695` (`vectorise`) | the narrowed-position pair other than (x, y) |

`:5672` maps this text to `MixedPrecision` in a highp shader without a
`gl_FragCoord` read, so the 42 (mediump shaders) meet one of the first four.
**Which one per shape is not confirmed:** no run with `line!()` exists. The
likely map, from reading the arms:

- `fcm_int_to_float`, `fcm_ivec2_to_vec2`, `fcm_rnd_int_lanes`:
  `F2f16(I2f(..))` is `narrowed()`, so the vector builder takes it; the lane
  that is **not** narrowed is `1.0025 * v` (an `F2i32`'s source) or the `isOk`
  `b`/`ref_out0` uniform beside it: the second-pair arm `:2970-2974`
  (`(Val::Const(_), _)` / non-`Fadd`) or `:2965`.
- `fcm_int_bool_to_vec2`, `fcm_int_to_mat2`: the bool lane
  (`FCMP_OR.v2f16.ne`) and the `ftrunc` lane (`F2f16(Ftrunc(..))`, whose source
  is not an `int_float`, so `narrowed()` is false) pair as `own(p) \|\| own(q)`
  false: `:2965`.
- `fcm_eval_order`: integer arithmetic; the first refusal is an int one the
  walk reaches before this text on a later pass (intval-rest's `IntValue`
  family, once its arms exist).

**Step FC0** (the first action of FC1, ten minutes): instrument each
`Err(MaliError::FragCoordMediump)` above with `eprintln!("{}", line!())` plus
the operands, run the six `fcm_*` references through
`cargo test --test mali_deqp_texmisc -- --ignored` (or the drm-shim
`deqp_ref.sh` path), record the six sites in this note's section 3.3, revert.
The recipe is `mali-fclane-lowering.md` section 3.

## 4. The mechanisms

### TV1. A vertex compare-product over mediump texels

*What.* The `compare_vec4` harness of 2.1 in the vertex walk: for each of the
four lanes of one look-up (`texture2D`/`textureCube`, or one element of a
sampler array) `abs(t - c) < 0.05 ? 1.0 : 0.0`, the four (and the array's
three, and the random program's other uniform compares) multiplied together,
`v_vtxOut *= product`.

*Mechanism.* It is `intval-vs16`'s M4 with texels as the operand. M4 grows the
vertex walk with: `abs` source modifiers on a 16-bit `Fmax`, the
product-of-selects fold (`b2f(iand)`, `flt(fmax)`), the narrowing of a uniform
word, and a vertex `cmp_product` ported from the fragment walk
(`nir_fs.rs:5748-5775`'s arm, `Fs::cmp_product`/`cmp_select`). TV1 adds, on
top of M4:

1. `vs_tex_read16`: a look-up read by the compare-product's `fadd` (or an
   `fsub` of a constant) stays fp16 (`half` true), so the look-up is the
   16-bit `TEX_SINGLE` of the references (3.1);
2. the texel pairing: `r0` = (x, y), `r1` = (z, w) as two fp16 words, the
   `FADD.v2f16 r2, r0.h11, <const pair word>` / `FADD_IMM.v2f16 r1, r1^,
   #pair` subtract of a constant **pair**, the pair built from the dEQP
   constants rounded to fp16 (`#0xB5C3B3AE` = (-0.12, -0.24) halves in some
   order, reproduced literally); which constants are immediates and which an
   FAU word (`tv_2d_compare` has one of each, `tv_rnd_2d` two immediates) is
   decided by whether the pair is two non-zero constants: the existing fp16
   constant rules of `vs_alu16`; the `0.00` lane makes the FAU-word form;
3. the `FMAX` chain order (`.abs.h00` of one operand against `.abs.h11` of
   another, then `.h00` of the partial results), joined across look-ups of an
   array **before** the single `FCMP_OR`;
4. the cube part (TV1c, 17 cases): the constant-direction cube sequence
   `CUBEFACE1 r0, 0x0, 0x0, 0x0` ... with **no constant fold of `FRCP`/`FMA`**
   (Mesa does not fold them; a fold here is "compiled, not Mesa's"). The
   vertex side builds it through `Nir::cube_desc` (`nir.rs`'s level-0 look-up
   arm, ending at 5207, already takes a cube at level 0), which must keep an
   all-constant direction unfolded. The fragment form of the same
   constant-direction cube is fmaloc's `tc_cube_const` (T1), whose
   fragment-side implementation is separate; the vertex words are in
   `tv_cube_compare` (they carry `CUBEFACE2_V9`, `MUX.i32.bit ..u5.w1`).

*Files.* `nir.rs` (the Alu arm at 4638-4651 and a new arm before it,
`vs_tex_read16`, the look-up arm ending at 5207); `emit.rs` only if the `FMAX`
chain's `.abs.hNN` modifiers are not yet expressible for a two-register pair.

*Dependency.* `intval-vs16` M1-M4 (`intval-vs16-3`, "the rest of M3 and M4",
todo). If M4 slips, TV1 cannot start: **do not write the port twice.**

### TK1. Fragment 2D bias and projection of a varying read whole; dead look-ups

*What.* Three small mechanisms in `Fs`'s fallback look-up arm
(`nir_fs.rs:6785`):

1. **Projection of a whole varying** (`texture2DProj(s, v)`, `v` a `vec3`/`vec4`
   read whole): `FRCP.f32` of `.z` or `.w`, two `FMA.f32 x*r, 0.0.neg`, a
   plain `2d.computed` look-up (`deqp_fmaloc_tk_proj_vec3/4`, `tk_rnd_proj_swizzle`
   2.2). The back end already builds `2d.computed` (the computed-coordinate arm
   at 6807) and `Frcp` / `Fmul`; the new part is accepting `Es100Tex::Tex2DProj`
   and mapping its coordinate lanes `(x, y)` and divisor `(z or w)` to the
   existing `TexCoord::Computed` source with the two products. The result
   swizzle (`.ywzx`) is the existing texel-lane `MKVEC.v2i16` machinery.
2. **2D bias** (`texture2D(s, c, bias)`): the third staging word of
   `2d.computed_bias` is the 8.8 pack the cube bias arm builds
   (`FMA.f32.clamp_m1_1 * 1/16`, `FMA.f32 * 4096`, `F32_TO_S32.rtz`,
   `U16_TO_U32`; `nir_fs.rs:6556-6577` for the uniform bias, a constant
   bias is its FAU word `MOV.i32 r4, u2.w0`, `deqp_texmisc_tk_rnd_bias_fragcolor`).
   The new part is `dim: D2` with a bias: `TexLookup` has the cube-only
   `ComputedBias` mode; `emit.rs:13029` selects it on `Some(_)` cube only, so
   a 2D look-up needs the `None if t.bias != Val::Undef` arm
   (`(LodMode::ComputedBias, plain)`) and `pack.rs:143`'s mode word for D2.
   The bias value: a uniform lane (the existing `uniform_dword`), a varying
   (`fmaloc_tk_bias_varying`, pack words in 2.2) or a folded constant.
3. **Skip dead look-ups.** The walk visits every `Tex` op in program order and
   raises on a kind it cannot lower, even if no later op reads it. Mesa's NIR
   removes dead look-ups (`.41`: two `texture2D(.., bias)`, `.108`: three
   look-ups and the int arithmetic). One liveness pass over the IR (a `Tex`
   whose result has no reader that reaches an output or a kept side effect) before the
   refusing arms makes `.41` and `.108` compile as Mesa's colour arithmetic
   alone, with **no** look-up. This is also correct for the look-ups the
   walk *can* lower: Mesa drops them too, so a lowered-but-dead look-up in a
   case today is "compiled, not Mesa's" and the pass fixes it (check against
   the references of `mali-deqp-compiled-audit.md` and the glmark2 corpus;
   section 6).

*Files.* `nir_fs.rs` (the fallback arm 6785-6820, a liveness helper near
`push_look_up`), `bir.rs` / `emit.rs` (`LodMode::ComputedBias` for D2),
`pack.rs:143`.

*Dependency.* None for the 21 (the coordinates are a varying read whole, or
a folded constant bias). The 14 random bias/projection cases carry other
operations (fp16 `b*b`, int arithmetic) that other steps own; each of them
finishes only when every operation in it is lowered.

### TK2. Projection of a built or computed coordinate; bias and projection together

*What.* `texture2DProj(a, vec3(c, f, e) + vec3(d, c, b))` (`.142`) and the
three built ones (`vec3(x, y, int(c))`, constants and int lanes); `.78`/`.99`
mix a bias and a projection. Mesa: the coordinate computed in fp16 (the sum,
`FADD.v2f16` + `FADD_IMM.v2f16`), three `F16_TO_F32`, then TK1's projection.

*Mechanism.* TK1's projection with a *computed* coordinate: it is fmaloc's
**computed-coordinate** form (T1/`tex-cube-face`'s `coord_lane` sums,
`mali-fmaloc-lowering.md` 3.1 mechanism 3, "a one-lane tree: `coord_lane`
accepts a sum of two varying lanes") applied to three lanes. Do not rebuild
`coord_lane`: use fmaloc's. The int lane (`.98`'s `int(c)`) is `intval-rest`'s
`F2i32` of an fp16 lane.

*Dependency.* fmaloc T1 step 4 (the sum lanes), and `intval-rest` step 1 for
the int-built coordinate.

### TK3. Cube bias that is not a uniform lane

*What.* `fmaloc_tk_cube_bias` (a varying bias) and `.121` (`c + b`, with a
constant `1.0` direction lane): `FADD.f32 r0, r1^, r0^` for the sum, then the
8.8 pack of TK1(2).

*Mechanism.* Drop `f.uniform_dword(bias).is_none()` at `nir_fs.rs:6576` for a
bias that is an fp32 value Mesa packs in registers: reuse TK1's pack builder
with the cube dimension (the pack is dimension-independent; only the LodMode
word differs). The direction `vec3(e, int(true), d)` (a constant lane beside
varyings) is fmaloc's `tc_cube_face` shape (`mali-fmaloc-lowering.md` 3.1
mechanism 1: a constant lane 0 or 1 beside trees) and `int(true)` the
`intval-rest` constant fold.

*Dependency.* TK1's bias pack; fmaloc's `tex-cube-face` (the `CubeDir`
variants) for the 10 random cube shapes beside it. The one plain case
(`texturecube_bias`) needs only TK1.

### FC1. fp16 int round trip (`shaders.conversions`)

*What.* `float(int(v * 1.0025))`, `vec2(ivec2)` and `mat2(int)` forms and their
`isOk` fp16 harness (2.3).

*Mechanism.* Follow each reference's form:
`F2i32` of an fp16 lane (`F16_TO_S32.rtz`, exists), `I2f` of the int
(`S32_TO_F32` + the `FADD.f32 .h0` narrowing, `narrowed()` exists), or,
where the int has no other reader, `Ftrunc` over the **widened** fp16 value
(`F16_TO_F32` + `FROUND.f32.rtz` + the narrowing), which the fp16 walk lacks:
`nir_fs.rs:3477`'s `I2f(F2i(a))` fold is in the fp32 `alu32` path only. The
narrowed lane must then be accepted by the vector builder and the pair arms
as a first-class fp16 lane (an `is_f16` that holds for a narrowed int lane and
a narrowed `Ftrunc` lane) so that `vec16_of` (`:2844`), `alu16`'s `own(..)`
(`:2965`) and the second-pair arm (`:2970-2974`) take it; the constant halves
(`0x0.h0` in `fcm_int_to_mat2`, `0x3C003C00` for alpha) are the existing
lane-and-constant halves. The `isOk` harness words (`FADD.v2f16 r0, r0^, ..neg`,
`FMA.v2f16 .. u2.w1`, `FCMP_OR.v2f16.ge`, `LSHIFT_AND.v2i16` + `MUX.v2i16` for
`all`) are those of `deqp_mixprec_cmpv4` and `fcm_*` shares them across all
six references: build the harness first on `fcm_int_to_float`
(the smallest), then widths 2, 3 and 4.

*Files.* `nir_fs.rs` (`Fs::alu16` and its helpers, `vec16_of`, `narrowed`,
`is_f16`; the fp16 `Ftrunc` node in `bir`/`emit` if absent).

*Dependency.* FC0 (the site map). `intval-rest`'s `bool(int)` arms for the
bool lanes (3 cases); nothing for the other 29.

### FC2. Random int lanes and the function-call shape

*What.* `fcm_rnd_int_lanes` (`float(int(int(b)))` of a highp lane, a mixed
constant half) and `fcm_eval_order` (integer arithmetic behind the fp16 one).

*Mechanism.* FC1's lane plus the random shaders' other operations
(`367:ARITH` family) and `intval-rest`'s int `iadd`/`icmp`/`csel` arms.

*Dependency.* FC1 and `intval-rest` steps 1-2.

## 5. Ranked one-session steps

Ranked by first-refusal cases removed, then by what each needs. "Start" gives
the order a planner can actually run today.

| Rank | Step | Shapes (cases) | Needs | Forecast (HOST-ONLY, first refusals removed; expected to pass) | Start |
|--:|---|---|---|---|---|
| 1 | **TV1** vertex compare-product over mediump texels (2D 43 first, cube 17 as TV1c) | `tv_*` (60) | `intval-vs16` M4 (the port) and `arith-lower-1`'s vertex `fabs` | 60 removed; 39 pass once nothing else is behind, 21 more after their fragment partners (12 `FRAGCOLOR_LANE`, 9 `TEX_COORD` `tc_cube_const`) | after M4 |
| 2 | **FC1** fp16 int round trip | `fcm_int_to_float`, `_ivec2_to_vec2`, `_int_bool_to_vec2`, `_int_to_mat2` (32, `shaders.conversions`) | FC0; `intval-rest` for the 3 bool cases | 32 removed; 29 pass, 3 after `intval-rest` | after FC0 (can run now) |
| 3 | **TK1** 2D bias, 2D projection of a varying read whole, dead look-ups | `fmaloc_tk_proj_vec3/4`, `tk_rnd_proj_swizzle`, `fmaloc_tk_bias_uniform/_varying`, `tk_rnd_bias_fragcolor/_two/_proj`, `fmaloc_tk_rnd_*` (21) | nothing | 21 removed; 7 plain cases pass (`texture2dproj_vec{3,4}`, `texture2d_bias`, `texture.mipmap.2d.bias.*` 4) and 4 of the random ones the references pin (`.131`, `.41`, `.108`, `.48`); the other random ones when their other operations lower | **now** |
| 4 | **TK3** cube bias from a varying or a sum | `fmaloc_tk_cube_bias`, `tk_rnd_cube_bias`, 9 random (11) | TK1's bias pack; fmaloc `tex-cube-face` for 10 | 11 removed; 1 pass plain, 10 after fmaloc step 1 | after TK1 |
| 5 | **FC2** random and function shapes | `fcm_rnd_int_lanes`, `fcm_eval_order` (10) | FC1, `intval-rest` | 10 removed; 2 pinned references pass, 8 random ones by their other operations | after FC1 |
| 6 | **TK2** projection of a built/computed coordinate; bias + projection | `tk_rnd_proj_sum`, 7 projection shapes, 2 bias+projection (10) | fmaloc T1 step 4, `intval-rest` step 1 | 10 removed; 1 pinned reference passes (`.142`) | after fmaloc |

Subtotal 144. By cases alone TV1 is first, but it waits for the largest
`intval-vs16` piece. Recommended **start order**: TK1 and FC0+FC1 first (no
dependencies, disjoint code in `nir_fs.rs`'s look-up arm vs its `alu16`), TV1
with or right after `intval-vs16-3`, TK3, FC2 and TK2 after their
dependencies. TK1 and FC1 can run in parallel only if plan 367's `lower-fma`
has released `src/shaderc/online/mali/**`: both edit `nir_fs.rs`, in different
functions (a rebase, not a rewrite).

Expected whole-dEQP effect: these 144 cases are 0.8% of the 17,165 must-pass
cases; the board's rerun should show them moving by their first-refusal count
only where the shader has no later refusal. This is the least valuable family
of plan #387's table by cases per session and the most Mesa-exact by shape;
rank it behind `intval-rest` and `fclane-lower-1/2` for the plan's 100-case
rule: only TV1 (60) and TK1+TK3 (32) are near it, and none reaches 100 alone.

## 6. Tests and pins at risk

- `src/shaderc/online/mali/tests.rs:1259` (mediump `texture2D(s, v, 1.0)`,
  `TexKind`) and `:1260` (mediump `texture2DProj`, `TexKind`) flip with TK1
  (re-pin to the next refusal or `ok`; the highp shaders' look-ups of those
  kinds have no reference and must stay `TexKind`: add them as new pins).
  `:1263`, `:3530`, `:3563`, `:8917`, `:8919` are highp/ES 3 cube and shadow
  pins: they must **not** move; TK1/TK3 touch mediump 2D and the mediump cube
  bias arm only.
- `tests.rs:1258` (`max(texture2DLod(s, p.xy, 0.0).x, 0.5)` in a vertex
  shader, `TexVertex`): TV1's new arm sits before the generic mediump-texel
  refusal; a mediump `fmax` of a texel and a constant is exactly what
  M4's `FMAX` arm takes. Expect the pin to move; check it against a new Mesa
  reference before deciding it is compiled (`vs_clamp`'s rules, `nir.rs:3574`).
- `tests.rs:1298` (`vec4(gl_FragCoord.yx, ..)` mediump, `FragCoordMediump`) and
  `:1299` (highp `gl_FragCoord.x * 0.5`): FC1 relaxes the pair arms; the
  `fc[k]` arm (`nir_fs.rs:2878`) and `vectorise`'s (x, y)-only pairing
  (`:8695`) must stay. Do not weaken `:2878`.
- `tests/mali_deqp_texmisc.rs` `refusals_pinned` (17 references): each step
  moves its references from `#[ignore]` + refusal pin to a byte-equal test,
  the way `tests/mali_deqp_fmaloc.rs` did; `fmaloc_tk_*` tests in
  `tests/mali_deqp_fmaloc.rs` (lines 148-178, ignored and pinned `TexKind`)
  move with TK1/TK3.
- **Compiled-not-Mesa's risk (TK1's dead-look-up pass):** it changes which
  look-ups an already-compiling shader emits. A shader whose result Mesa drops
  but the walk keeps today is "compiled, not Mesa's"; the pass moves it toward
  Mesa but also moves bytes of any pinned reference with a dead look-up. Run
  the whole `tests/mali_*` set that byte-compares references, and
  `tests/mali_glmark2_corpus.rs` (`tests/fixtures/mali_glmark2_verdicts.txt`:
  verdicts only move forward). The texture scenes 2-4 (`texture-filter`) are
  `TexCoord` and are not changed by these steps.
- `tools/check.sh`'s Mali dEQP step runs `mali_deqp_{intval,varylink,constop,
  unitype,uniarray,mixprec,colourpair,loops,misc}.rs` and **not**
  `mali_deqp_texmisc.rs` (the `deqp-gate` item adds every `mali_deqp_*.rs`);
  until then a step that edits these tests must name them in its own commit
  and run `cargo test --test mali_deqp_texmisc` locally (the contract of
  `CLAUDE.md`: the tests you changed).

## 7. Overlaps

- **fmaloc T1 (`fmaloc-lower-1`, plan 367's `lower-fma`)**: TK2 and TK3 use its
  cube direction variants (`CubeDir`, `coord_lane` sums); TV1c uses the same
  cube words at a constant direction (`tc_cube_const`'s) in the vertex walk;
  none of its steps covers `TexKind`. T2 of the schedule ("TEX_VERTEX 27") is
  TK1's and TK3's shapes, not a vertex step: **fix the schedule row** (item
  `schedule-refresh`): T2 becomes `TexKind` 42 here, TexVertex 60 separate.
- **intval-vs16 (M1-M4)**: TV1 *is* M4's port plus texel operands; whoever
  does M4 should read 2.1 first and design the operand abstraction (a
  16-bit lane source: uniform word half or texel half) once.
- **intval-rest**: FC1's bool lanes (3), FC2, TK2's `int(c)`, TK3's `int(true)`
  need its `i2b`/`F2i32`/`iadd`/`icmp` arms. FC1's other 29 do not.
- **fclane (lower-1/2/3)**: the colour of `tv_*`'s `_both` fragment shaders
  (12 `FRAGCOLOR_LANE`) and the `(RES, RES, RES, 1.0)` colour of the `fcm_*`
  harness are fclane's lane sources; `fcm_*`'s `MKVEC.v2i16 r1, r0.h0,
  r1^.h0` + `IADD.v2u16 r0, r0^.h00, 0x0` is the broadcast shape F2C/F2E. If
  fclane lands first the harness colour needs no work here; if not, FC1 owns
  the lanes it needs.
- **arith (A0..A8)**: `abs` of a vertex mediump operand (TV1) and the fp16
  `abs`/`fmax` forms of the arith design's `FMAX.v2f16 .abs` rows; build the
  source-modifier support once.
- **constprod / libm-fold**: none; the constants here are exact fp16 pairs.
- **misc2 / mixcol**: `deqp_mixprec_cmpv4` is the fragment form of the
  compare product FC1's `isOk` harness resembles; reuse, do not duplicate.

## 8. What this design does not know

- The refusing site per shape of FragCoordMediump (section 3.3; step FC0).
- The TexVertex site is read from the guard at `nir.rs:4638-4651`, not
  measured; the four non-array `tv_*` shapes should be instrumented at the
  start of TV1 like FC0.
- Whether the vertex `cmp_product` can share `Fs::cmp_product`'s state
  (`Fs::cmp_chain`) or needs a `Nir` copy: `mali-intval-vs16-lowering.md`'s M4
  says "moves into `Nir`".
- Precision and size variants of the TexKind shapes (`vec3` vs `vec4`
  projection, lowp samplers, `mat3`/`mat4`, `ivec3`/`ivec4`): one reference per
  shape exists; a step draws its own with `tools/mali/deqp_ref.sh` before
  editing where the shape differs (the reference note's section 6).
- Why Mesa emits `FMA.f32 r0, r0^, 0x0, r0^` for `v *= x` (`ffma(x, 0.0, x)`):
  it matches `vsapi_mat2`, so the existing word is reused; the compare-product
  port should reproduce it, not derive it.
- Counts are first refusals at `ca7cdc52`; `remeasure-tip` and
  `measure-crash-resume` (plan items) may change them.
