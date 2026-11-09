<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): fragment pow, normalize, reflect, length and fract

**Status: design** (plan #336, issue #33). This note changes no code. It says,
for each of the five GLSL built-ins in a fragment shader, which NIR Mesa
26.2.1 lowers it to, which Valhall instructions Mesa emits for a small
representative shader (quoted), which of them `src/shaderc/online/mali/`
already makes and which are missing, which glmark2 shaders each unblocks, and
in what order to implement them.

## How the words were made

The reference is Mesa 26.2.1's own GL driver on the drm-shim, the arm of
mali-alu-ops.md: `mesa-26.2.1.tar.xz` (sha256 `c47e81bd...a4442`, checked
against `third_party/mesa-26.2.1/UPSTREAM`), built with
`tools/mali/drm_shim_ref.sh build` (LLVM 21, `LLVM_DIR=/usr/lib/llvm-21`) in
`$CONNECTED_DEPS_DIR/mesa-shim`. One pass-through vertex shader
(`attribute vec4 a_pos, a_v; varying vec4 v;`) and seven fragment shaders,
each a `precision mediump float` and a `precision highp float` form (14
draws), `varying vec4 v` and `uniform vec4 u`:

| name | `main` body |
|---|---|
| pow | `gl_FragColor = vec4(pow(v.x, u.y), 0.0, 0.0, 1.0);` |
| normalize | `gl_FragColor = vec4(normalize(v.xyz), 1.0);` |
| reflect | `gl_FragColor = vec4(reflect(v.xyz, u.xyz), 1.0);` |
| length | `gl_FragColor = vec4(length(v.xyz), 0.0, 0.0, 1.0);` |
| fract | `gl_FragColor = vec4(fract(v.x), 0.0, 0.0, 1.0);` |
| nconst | `gl_FragColor = vec4(normalize(vec3(1.0, 2.0, 2.0)) * v.x, 1.0);` |
| lconst | `gl_FragColor = vec4(length(vec3(1.0, 2.0, 2.0)) * v.x, 0.0, 0.0, 1.0);` |

Per draw (64x64, `glGetError` 0 for all 14):

    tools/mali/drm_shim_ref.sh run <shim> fsv.vert fsv_<name>_<mediump|highp>.frag target/fsv/<same>
    python3 -I tools/mali/pan_ref_diff.py extract target/fsv/<same>.trace --draw

The listings below are the fragment program's disassembly from the run's
stderr (`BIFROST_MESA_DEBUG=shaders,shaderdb`, the last program, which is
the one `extract --draw` pairs with `fs.fragment`), without the instruction
bytes and without the tail (`ATEST`, `BLEND`, `IADD_IMM r48`, `BRANCHZI`), which
is the same in all of them. The first `pan_shc` version of this note (a SPIR-V
path that stays at fp32) matches the highp listings of `fract`, `length`,
`normalize` and `pow` word for word, and `reflect` highp except for operand
order (see reflect). Every mediump listing below is measured here: Mesa's GLSL
front end lowers a mediump operand to fp16 (`V2F32_TO_V2F16`, `.v2f16`
instructions, `FRSQ.f16`), so the fp16 forms differ from the highp ones
beyond the narrowing. The fixtures `tools/mali/testdata/fsarith_*` are made in
the first implementation commit (item 1 below), not by this note.

## What the front end already gives the back end

`src/shaderc/online/builtin_call.rs` (:860-990) expands the built-ins to the IR
the Mali walk reads:

| GLSL | IR |
|---|---|
| `normalize(v)` | `Fdot(v, v)`, `Frsq`, `Fmul(v, splat)` |
| `length(v)` | `Fdot(v, v)`, `Fsqrt` |
| `reflect(I, N)` | `Fdot(N, I)`, `Fmul(d, -2.0)`, `Ffma(splat, N, I)` |
| `pow(x, y)` | `Flog2(x)`, `Fmul(y, l)`, `Fexp2(m)` |
| `fract(x)` | `AluOp::Ffract` |

so there is nothing for the front end to do; the work is in
`nir_fs.rs` / `nir.rs` / `emit.rs` / `bir.rs` / `pack.rs`.

## The walk today

* `Alu` (nir.rs:96-) has `Frcp`, `Frsq`, `Ffloor`, `Fceil`, `Ftrunc`, `Fadd`,
  `Fmul`, `Ffma`, `Fneg`, `Fabs`, `Fclamp`; **no** `Flog2`, `Fexp2`.
* The highp lane arm (`Fs::lane32`, nir_fs.rs ~2043-2056) makes `Fsqrt` as
  `Frcp(Frsq(x))`, `Frsq`, `Ffloor` and `Ffract` as
  `Fadd(x, Fneg(Ffloor(x)))`. The two-lane mediump arm (~1792) does `Ffract`
  on a pair of halves (`FROUND.v2f16`).
* The mediump walk's catch-all `ir::Op::Alu(..) => Err(Arithmetic)`
  (nir_fs.rs:4216) refuses everything else, among them `Ffract` of a single
  mediump lane or of three or four lanes, `Fsqrt`, `Frsq`, `Flog2`, `Fexp2`.
  The dot products are handled (`Fdot2/3/4`, ~:2284).
* `bir.rs` has `FrcpF32`, `FrsqF32`, `FroundF32`, `FmaF32`, `FaddF32`; no
  `FlogdF32`, `FexpF32`, `FrexpeF32`, `FaddLscaleF32`, `FmaRscaleF32`, and
  `F32ToS32` always writes `.rtz`. mali-transcendentals.md (case 77) designs
  exactly these for `sin`/`cos`/`log2`/`exp2`; **pow is that note's log2 and
  exp2 plus one multiply**, so it adds no new row beyond the note.

## Per built-in

### fract

NIR: `lower_ffract` (`nir_opt_algebraic.py`, for arch < 11): `ffract(a)`
becomes `fadd(a, fneg(ffloor(a)))`.

highp (`fsv_fract_highp`):

```
FROUND.f32.rtn.discard r1, r0
FADD.f32 r0, r0^, r1^.neg
FADD.f32.wait0126 r0.h0, r0^, 0x0.neg       # the colour's narrowing
```

mediump (`fsv_fract_mediump`; the varying arrives as fp32, one lane is
narrowed into the low half, `FROUND`/`FADD` run as `v2f16` on that half):

```
FADD.f32.discard r0.h0, r0^, 0x0.neg
FROUND.v2f16.rtn r1, r0.h00
FADD.v2f16.wait0126 r0, r0^.h00, r1^.neg.h00
```

Yakogl: the instruction pair exists (`Alu::Ffloor` -> `FROUND.rtn`, `Fadd`
with `Fneg` folded as a `.neg` source). The highp lane arm builds it. Missing:
an arm for a single mediump lane and for three/four mediump lanes (a
`Word::Lanes` colour, nir_fs.rs ~6750); for a pair of fp16 lanes the existing
`FROUND.v2f16` path is used, and the three-lane case is a pair plus a lone
lane. Reference (tools/mali/testdata): `s4_ffract.*` is the highp case.

### length

NIR: `fdot3(v, v)` -> `fsqrt` -> `lower_fsqrt` (`frcp(frsq(a))`).

highp (`fsv_length_highp`):

```
FMA.f32.discard r2, r2^, r2^, 0x0.neg
FMA.f32 r1, r1^, r1^, r2^
FMA.f32 r0, r0^, r0^, r1^
FRSQ.f32 r0, r0^
FRCP.f32 r0, r0^
```

mediump (`fsv_length_mediump`): the varying's x,y are narrowed as a pair, z
alone, the dot is `v2f16` FMAs on single halves and the root is `.f16`:

```
V2F32_TO_V2F16.discard r0, r0^, r1^
FADD.f32 r1.h0, r2^, 0x0.neg
FMA.v2f16 r1, r1^.h00, r1^.h00, 0x0.neg
FMA.v2f16 r1, r0.h11, r0.h11, r1^.h00
FMA.v2f16 r0, r0^.h00, r0^.h00, r1^.h00
FRSQ.f16 r0, r0^.h00
FRCP.f16.wait0126 r0, r0^.h00
```

The dot is a chain of `FMA` (`v.z*v.z + -0.0`, then `v.y*v.y +`, `v.x*v.x +`),
Mesa's order z, y, x, in both precisions. Yakogl: all five exist (`Fdot3`
expansion, `Frsq`, `Frcp`); the highp `Fsqrt` arm is `Frcp(Frsq(x))`. Missing:
the same `Fsqrt` arm in the mediump walk (a scalar result from three lanes),
as `FRSQ.f16` / `FRCP.f16` (new `.f16` forms of the two bir ops if they do
not exist yet) after `v2f16` FMAs. Smallest of the five.

`length` of a constant (`lconst`, `length(vec3(1.0, 2.0, 2.0)) * v.x`) is
folded by Mesa to 3.0; no `FRSQ`/`FRCP` is emitted. In the highp run the
product `3.0 * v.x` is not in the fragment program at all: `nir_opt_varyings`
moved the `fmul` to the vertex shader (the VS's NIR holds `fmul 3.0, v.x`), so
the fragment program only reads the varying and narrows it. In the mediump run
the multiply stays in the fragment shader with 3.0 as a pushed fp16 word.
Fragment listings (`fsv_lconst_*`):

```
highp:   FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
mediump: FADD.f32.discard r0.h0, r0^, 0x0.neg
         FMA.v2f16.wait0126 r0, u2.w0, r0^.h00, 0x0.neg
```

### normalize

NIR: `fdot3(v, v)`, `frsq`, three `fmul`.

highp (`fsv_normalize_highp`):

```
FMA.f32.discard r3, r2, r2, 0x0.neg
FMA.f32 r3, r1, r1, r3^
FMA.f32 r3, r0, r0, r3^
FRSQ.f32 r3, r3^
FMA.f32 r0, r0^, r3, 0x0.neg
FMA.f32 r1, r1^, r3, 0x0.neg
FMA.f32 r2, r2^, r3^, 0x0.neg
FADD.f32 r0.h0, r0^, 0x0.neg  (x3, then MKVEC.v2i16 pairs for the colour)
```

mediump (`fsv_normalize_mediump`; x,y as one fp16 pair, z in its own
register, the two products as one `v2f16` FMA and one single-half FMA):

```
V2F32_TO_V2F16.discard r0, r0^, r1^
FADD.f32 r1.h0, r2^, 0x0.neg
FMA.v2f16 r2, r1.h00, r1.h00, 0x0.neg
FMA.v2f16 r2, r0.h11, r0.h11, r2^.h00
FMA.v2f16 r2, r0.h00, r0.h00, r2^.h00
FRSQ.f16 r2, r2^.h00
FMA.v2f16 r0, r0^, r2.h00, 0x0.neg
FMA.v2f16.wait0126 r1, r1^.h00, r2^.h00, 0x0.neg
MKVEC.v2i16.wait r1, r1^.h0, u2.w0.h0
```

The mediump colour needs no per-lane narrowing: the pair `r0` is already the
x,y halves, `MKVEC.v2i16` packs z with the constant 1.0 (`u2.w0.h0`).

`normalize` of a constant (`nconst`, `normalize(vec3(1.0, 2.0, 2.0)) * v.x`)
is folded to (1/3, 2/3, 2/3): no dot, `FRSQ` or `FRCP`. The folded
components are pushed words (`u2.w0`, `u2.w1`; one fp32 constant each for
highp, where y and z share one product; for mediump `u2.w0` is read as a pair
of fp16 lanes, so one `v2f16` FMA makes the x,y halves and the second one
makes z) (`fsv_nconst_*`):

```
highp:   FMA.f32.discard r1, u2.w0, r0, 0x0.neg
         FMA.f32 r0, u2.w1, r0^, 0x0.neg
         FADD.f32 r1.h0, r1^, 0x0.neg
         FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
         MKVEC.v2i16 r1, r1^.h0, r0.h0
         MKVEC.v2i16 r2, r0^.h0, u3.w0.h0
         MOV.i32 r0, r1^
         MOV.i32.wait r1, r2^
mediump: FADD.f32.discard r0.h0, r0^, 0x0.neg
         FMA.v2f16 r1, u2.w0, r0.h00, 0x0.neg
         FMA.v2f16.wait0126 r0, u2.w1, r0^.h00, 0x0.neg
         MKVEC.v2i16 r2, r0^.h0, u3.w0.h0
         MOV.i32 r0, r1^
         MOV.i32.wait r1, r2^
```

Yakogl: every row exists (`Frsq`, `Fmul` as `FMA ... -0.0`, the `Fdot3`
chain). Missing: the mediump arm for `Frsq` (the catch-all) and a splat of the
scalar `frsq` over three lanes. No new bir op. The `Fmul` by a splat scalar is
three independent `FMA` rows; only the dot's operand order (z, y, x) and the
register reuse are to be matched by the byte test.

### reflect

NIR: `fdot3(N, I)`, `fmul(d, -2.0)`, `ffma(k, N, I)`; algebraic rules turn it
into `fadd(d, d)` and `ffma(fneg(N), d2, I)`: **one** `FADD` (`d + d`), then
three `FMA`s, in both precisions.

highp (`fsv_reflect_highp`):

```
FMA.f32.discard r3, u3.w0, r2, 0x0.neg
FMA.f32 r3, u2.w1, r1, r3^
FMA.f32 r3, u2.w0, r0, r3^
FADD.f32 r3, r3^, r3^                         # 2*dot
FMA.f32 r0, r3.neg, u2.w0, r0^
FMA.f32 r1, r3.neg, u2.w1, r1^
FMA.f32 r2, r3^.neg, u3.w0, r2^
```

mediump (`fsv_reflect_mediump`; the dot and the 2*dot are `v2f16`, the
uniform halves are read as `u2.w1.h00`, `u2.w0.h11` and so on):

```
V2F32_TO_V2F16.discard r0, r0^, r1^
FADD.f32 r1.h0, r2^, 0x0.neg
FMA.v2f16 r2, u2.w1.h00, r1.h00, 0x0.neg
FMA.v2f16 r2, u2.w0.h11, r0.h11, r2^.h00
FMA.v2f16 r2, u2.w0.h00, r0.h00, r2^.h00
FADD.v2f16 r2, r2^.h00, r2^.h00
FMA.v2f16 r0, r2.neg.h00, u2.w0, r0^
FMA.v2f16.wait0126 r1, r2^.neg.h00, u2.w1.h00, r1^.h00
MKVEC.v2i16.wait r1, r1^.h0, u3.w0.h0
```

Difference from the `pan_shc` (SPIR-V, fp32) listing the first version of
this note used, highp: the dot's FMAs name the uniform first (`u3.w0, r2`
where `pan_shc` has `r2, u3.w0`; likewise `u2.w1, r1` and `u2.w0, r0`), and
the three final FMAs carry the negation on the register (`r3.neg, u2.w0, r0^`
where `pan_shc` has `u2.w0.neg, r3, r0^`; likewise r1 and r2). The
instruction count and the order of the rows are the same. The shim's operand
order is the one the byte test must produce. Answer to the open question:
Mesa emits one `FADD`, not three.

Yakogl: all instructions exist; what is missing is the rewrite
`fmul(d, -2.0)` -> `fadd(d, d)` (the `d*2.0 = d+d` rule,
`nir_opt_algebraic.py`) and `ffma(fmul(a, -2.0), n, i)` -> `ffma(fneg(n), 2a, i)`
in `nir.rs` (check mali-algebraic-folds.md for what is already transcribed),
and a source `neg` on the first operand of `FmaF32` with a uniform source
(exists: `Src.neg`). No new bir op.

### pow

NIR: `lower_fpow` (panfrost sets it): `fpow(a, b)` ->
`fexp2(fmul(flog2(a), b))`, then Bifrost's own expansion of `flog2` and
`fexp2` (`bi_flog2_32`, `bi_fexp_32`).

highp (`fsv_pow_highp`):

```
FREXPE.f32.log.discard r1, r0
S32_TO_F32 r1, r1^
FADD_LSCALE.f32 r2, 0x3F800000.neg, r0
FLOGD.f32 r0, r0^
FMA.f32 r0, r0^, r2^, r1^                      # log2(x)
MOV.i32 r1, u3.w0                              # the constant 24
FMA_RSCALE.f32 r0, u2.w1, r0^, 0x0.neg, r1^    # y * log2(x) * 2^24
F32_TO_S32 r1, r0                              # round none, no .rtz
FEXP.f32 r0, r1^, r0^
FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
```

mediump (`fsv_pow_mediump`): the whole pow stays fp32 (Mesa has no fp16
`FLOGD`/`FEXP`; the varying is read as fp32), only the uniform `u.y` is an
fp16 word, converted first, and the 24 is not a `MOV` but the pushed word
`u2.w1` read in `src3`:

```
FREXPE.f32.log.discard r1, r0
S32_TO_F32 r1, r1^
FADD_LSCALE.f32 r2, 0x3F800000.neg, r0
FLOGD.f32 r0, r0^
FMA.f32 r0, r0^, r2^, r1^                      # log2(x)
F16_TO_F32 r1, u2.w0.h1                        # y
FMA_RSCALE.f32 r0, r1^, r0^, 0x0.neg, u2.w1    # y * log2(x) * 2^24
F32_TO_S32 r1, r0
FEXP.f32 r0, r1^, r0^
FADD.f32 r0.h0, r0^, 0x0.neg
```

This is mali-transcendentals.md's rows 23-27 (log2) and 30-34 (exp2) with the
`* y` folded into `FMA_RSCALE`'s second source (the exp2 note's constant
`1.0` log2 base) instead of a separate multiply. Two differences to
carry over. The multiply by `y` is not a separate `FMUL`: `FMA_RSCALE.f32 r0,
u2.w1, r0^, 0x0.neg, r1^` has `y` as `src0`, `log2(x)` as `src1` and the scale
24 in `src3`, so `fexp2(y * log2(x))` costs one instruction, not a multiply
plus an `FMA_RSCALE` by 1.0 (case 77's form). And the 24 is here a pushed
constant (`MOV.i32 r1, u3.w0`) where case 77 has `IADD_IMM.i32 r4, 0x0, #0x18`:
`fau.rs` chooses, as it already does.

Yakogl missing: `FlogdF32`, `FrexpeF32`, `FaddLscaleF32`, `FmaRscaleF32`,
`FexpF32`, `F32ToS32` without `.rtz` and `Alu::Flog2` / `Fexp2` with their
rows (all designed in mali-transcendentals.md), plus the fusion: `emit.rs`
expands `Fexp2` of a product as one `FMA_RSCALE` with the factors as
`src0`/`src1`. The shim confirms the fusion, in both precisions. In the
mediump form the pow is still fp32, so the mediump arm needs `F16_TO_F32` for
a mediump `y` and the `FADD.f32 r0.h0` narrowing, not fp16 log/exp.

## glmark2 shaders and sweep cases

Source: `$CONNECTED_DEPS_DIR/glmark2-src/data/shaders/*.frag` and `*.all`,
searched for each call (a name match; whether the shader then passes also
depends on its other refusals, which the corpus tally of
tests/glmark2_corpus.rs decides and this note did not run):

| built-in | fragment shaders using it |
|---|---|
| pow | bump-normals, bump-height, bump-normals-tangent, bump-poly, ideas-lamp-lit, ideas-logo, jellyfish, light-advanced, light-phong, light-cel, light-refract, terrain |
| normalize | bump-height, bump-normals-tangent, bump-normals, bump-poly, ideas-lamp-lit, ideas-logo, light-refract, light-advanced, light-cel, terrain-normalmap, light-phong, terrain |
| reflect / refract | ideas-lamp-lit, ideas-logo, light-phong, light-cel, light-refract |
| length | light-cel, terrain |
| fract | conditionals, function, loop, light-basic-tex-bilinear and the `-step-*.all` variants of conditionals, function and loop |

gl_test (tools/shaders/es100): `equirect.frag` (`normalize`) and
`fragcentre.frag` (`fract`) are `ok` in `tools/mali/corpus-verdicts.txt`
already, so no sweep case is unblocked by the five alone. `pow` shares its
log2/exp2 with case 77 (`transc`), which the transcendentals note covers.

`reflect` appears in the list but `refract()` is another built-in (not
covered here): light-refract needs it after `reflect`.

## Implementation order

1. Make the references: the shim runs of this note are done (the `pow` fusion
   is confirmed); add the 14 draws as `fsarith_*` testdata
   (`extract --draw` JSON) and compare with the sequences above.
2. `length` and `normalize`: the mediump arms for `Fsqrt` and `Frsq` and the
   scalar-splat multiply. No new op. Unblocks the `normalize`-only shaders
   (terrain-normalmap) once their other refusals are gone.
3. `fract` of a single mediump lane and of three or four lanes.
4. `reflect`: the `d+d` and negated-operand rewrites in `nir.rs`, then the
   mediump `Ffma` arm. Unblocks the light and ideas shaders together with 2.
5. `pow`: after fs-transc (mali-transcendentals.md) lands `FlogdF32`,
   `FexpF32`, `FmaRscaleF32` and the others; then the fused multiply. Largest
   item and the one most glmark2 shaders wait for (12 of them).
6. Host tests per item in `src/shaderc/online/mali/tests.rs` in the style of
   `check_fs_stage`; `tools/mali/corpus-verdicts.txt` and
   `sweep_passsets.py --delta tools/cts/log` move forward only.

## Risks

* The listings are measured on the shim but not yet committed as
  `tools/mali/testdata` fixtures (item 1); the register choice and `^` flags
  are those of the runs above.
* The folded `lconst` product moves to the vertex shader in highp
  (`nir_opt_varyings`); a fragment lowering sees only the narrowing there.
* docs/README.md has no index line for this note: that file is outside this
  item's files (plan #327 is editing it); the planner adds it.
