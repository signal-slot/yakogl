<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: glmark2's ideas scene (plan #367 lower-ideas)

glmark2 22c527cb's `ideas:speed=duration` (scene 20) links nine programs
(`tests/mali_glmark2_corpus.rs` `PROGRAMS`). At the integration tip
`31cdaefc`, four of its shaders were refused (`tests/fixtures/
mali_glmark2_verdicts.txt`):

| shader | what it is | refusal at 31cdaefc | after this item |
| --- | --- | --- | --- |
| fs df64 | the alpha-tested glyph: `texture2D(tex, gl_FragCoord.xy / 32.0)`, `if (c.w < 0.5) discard;` | TEX_COORD | `ok`, byte-equal (gm2_ideas_1) |
| vs 51b2 | the logo and lamp vertex shader: `unitvec(v1, v2)` with three early returns, called with `vec4(0.0, 0.0, 0.0, 1.0)` | CONTROL_FLOW | `ok`, vertex programs byte-equal (gm2_cf_earlyret_vs) |
| fs c565 | the logo: the same function of a varying and a uniform, `pow(.., 30.0)` | INT_VALUE | INT_VALUE (open) |
| fs 1aca | the lamp: three `Lamp` structs in a global array, a counted loop calling the function | INT_VALUE | INT_VALUE (open) |

The references are `tools/mali/testdata/gm2_ideas_1..3` (study-ideas,
`7f730a6d`: df64 under mediump, c565 and 1aca whole, each paired with vs
51b2's text) and `gm2_cf_earlyret_vs` (vs 51b2 itself, ref-control-flow).
All are Mesa 26.2.1 panfrost on drm-shim with `--uniform-values`.
`tests/mali_ideas.rs` checks them. `tools/mali/testdata/cases.txt` lists
gm2_ideas_1 and gm2_cf_earlyret_vs with pan_shc's verdicts.

## 1. fs df64: a two-lane mediump coordinate of gl_FragCoord

gl_FragCoord is mediump in GLSL ES 1.00, so `gl_FragCoord.xy / 32.0` is fp16
arithmetic in a mediump shader (docs/notes/mali-fragcoord-mediump.md). Mesa
narrows x and y to one word and multiplies the pair by (1/32, 1/32). The
look-up's coordinate is fp32, so each half is widened into the staging words:

```text
U16_TO_F32 r0, r59.h0 / U16_TO_F32 r1, r59^.h1 / FADD_IMM.f32 +0.5 (x, y)
FMA.f32 r1, r1^, u2.w0, u2.w1               y through the flip
FADD.f32 r0.h0, r0^, 0x0.neg / FADD.f32 r1.h0, ...   / MKVEC.v2i16
FMA.v2f16 r0, r0^, u3.w0, 0x0.neg           u3.w0 = 0x28002800, 1/32 twice
F16_TO_F32 r1, r0.h0 / F16_TO_F32 r0, r0^.h1 / MOV.i32 r2, r1^ / MOV.i32 r3, r0^
TEX_SINGLE.slot0.skip.reserved.16.2d.computed @r0:r1, @r2:r3
DISCARD.f32.lt r1.h1, 0x38000000.h1         c.w < 0.5 on the texel's half
```

The lowering is in `nir_fs.rs` `select`, the mediump pair route:

- A lone look-up's computed coordinate with two different lanes is
  vectorised into one pair, as a cube direction's x and y already were
  (`vectorise`). At the look-up, each half is `F16_TO_F32` into its staging
  word. Before this item, only one lane in both (case 121's `vec2(c.w *
  0.5)`) was taken.
- `frag_coord_tree` keeps this to the measured shape: operations on
  gl_FragCoord's word (`Leaf::Frag`) and constants. A varying's lanes plus a
  constant (desktop blur's lone taps, gm2_constfold_5 and 6), a uniform, or
  anything else stays TEX_COORD.
- `cmp_leaf`, the operand of a mediump discard's compare, now takes a texel of
  the look-up's fp16 result: its half of the word (`Leaf::TexLane`).

## 2. vs 51b2: early returns with a constant argument

The front end inlines `unitvec` as three nested `if`s (the run form of
`lower.rs`: the code after a returning `if` goes into its `else`):

```text
if (v1.w == 0.0 && v2.w == 0.0) r = v2 - v1
else if (v1.w == 0.0)            r = -v1
else if (v2.w == 0.0)            r = v2
else                             r = v2.xyz / v2.w - v1.xyz / v1.w
```

With `v2` the constant `(0, 0, 0, 1)`, the first and third conditions fold
to false (`iand(x, false)`, `1.0 == 0.0`), and so do `v2.xyz / v2.w` (0.0)
and `0.0 - q` (`-q`). Mesa removes the dead `if`s and flattens the one left
(`nir_opt_peephole_select`) into a select of `-v1.xyz` and `-(v1.xyz *
rcp(v1.w))`. The reference shows that select made of the operands, with the
negation once, after `normalize`'s product, at each store:

```text
FRCP.f32 r1, r0                     1 / v_eye.w
FCMP_OR.f32.eq.m1 r0, r0^, 0x0, 0x0
FMA.f32 r2, r7, r1, 0x0.neg / CSEL.u32.eq r2, r0, 0x0, r2^, r7^    (z, then y, x)
FMA.f32 r1, r2, r2, 0x0.neg / FMA ... / FRSQ.f32 r1, r1             the dot of the select
FMA.f32 r0, r0^, r1, 0x0.neg / FADD.f32 r0, r0^.neg, 0x0.neg / STORE
```

The vertex walk (`nir.rs`) now does three things:

- `vs_steps` takes an `if` whose sides hold `if`s (`vs_side`, at most
  `VS_IF_DEPTH` = 4 deep).
- The walk keeps a stack of `VsIf` frames. An `if` whose condition folds is
  the side it picks. One that does not is joined by selects (`vs_join`) only
  when no enclosing `if` was kept too, and only when each side walked at most
  `PEEPHOLE_LIMIT_GL` operations, counted on the sides the folds left. Two
  kept `if`s nested, or a side over the limit, stay CONTROL_FLOW: no
  reference draws them.
- `vs_join` makes two negations one negation of the select. The dot product
  of two all-negated vectors is that of the vectors (`fdot(fneg a, fneg b)`,
  nir_opt_algebraic.py:4237-4239), so `normalize` reads the select and the
  existing `fmul(fneg(a), b)` rule moves the negation to the product.

gm2_cf_earlyret_vs's own fragment shader (`vec4(v_nrm + v_dir, v_eye.w)`,
mediump) is refused at FS_VARYING_LOAD, a family with no item. The test
compares the vertex programs with a fragment shader that reads every lane,
and the link then keeps the reference's layout (v_eye whole at 0x0, v_nrm at
0x10, v_dir at 0x1c). `deqp_ctrlflow_vs_nested` (an `if (false)` under a kept
`if`) became byte-equal by the same rule.

## 3. Open: fs c565 and 1aca

Both call the same function with two run-time vec4s (`v_eye` and a uniform,
or the lamp's position from a struct array). No condition folds, and Mesa
flattens the whole function in the fragment shader (gm2_ideas_2's
fs.fragment, 87 instructions):

- The function's return value is fp32 (a function's return is not lowered to
  mediump), so the three inner `if`s become fp32 selects of widened fp16
  values (`F16_TO_F32` of each lane, `S16_TO_S32` of the 16-bit mask,
  `CSEL.u32.eq`), narrowed again (`FADD.f32 r.h0`).
- The outer `a.w == 0.0 && b.w == 0.0` is the `iand` of two fp16 masks
  (`LSHIFT_AND.v2i16 r4, r7^.h00, 0x0.b00, r4^`). Its select of `b - a` is
  fp16 (`MUX.v2i16 .., r4.h00`).
- `pow(max(0.0, dot), 30.0)` is the fp32 `FLOGD`/`FEXP` route with
  `FMA_RSCALE`.

The fragment walk refuses the `iand` (INT_VALUE, the first refusal) and
would then need the mixed fp32/fp16 selects of a flattened `if` chain whose
values cross precisions. fs 1aca adds the counted loop of three passes over a
global struct array (gm2_cf_constloop_struct_fs is its smaller form, refused
at INT_VALUE too). Neither fits this item; they are named in its report.
