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

`pan_shc` (tools/mali/README-pan_shc.md), the binary built by
`tools/mali/build_pan_shc.sh` in a Mesa 26.2.1 tree
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-pan/src/panfrost/pan_shc/pan_shc`),
run through `tools/mali/glsl2pan.py` on one pass-through vertex shader and
five one-line fragment shaders, all `precision mediump float`, a
`varying vec4 v` and a `uniform vec4 u`:

| name | `main` body |
|---|---|
| pow | `gl_FragColor = vec4(pow(v.x, u.y), 0.0, 0.0, 1.0);` |
| normalize | `gl_FragColor = vec4(normalize(v.xyz), 1.0);` |
| reflect | `gl_FragColor = vec4(reflect(v.xyz, u.xyz), 1.0);` |
| length | `gl_FragColor = vec4(length(v.xyz), 0.0, 0.0, 1.0);` |
| fract | `gl_FragColor = vec4(fract(v.x), 0.0, 0.0, 1.0);` |

The disassembly below is `disassemble_valhall`'s, from stderr, without the
instruction bytes. **Caveat.** `pan_shc` reads SPIR-V, not Mesa's GLSL path,
and in this run it left the arithmetic at fp32 (the varying is read
`LD_VAR_BUF_IMM.f32 ... src_f32` and only the colour is narrowed with
`FADD.f32 r.h0`); in a shader whose operands the GLSL front end lowers to
fp16 (a mediump uniform, `lower_precision`) the same operations appear around
`F16_TO_F32` / `FADD.f32 r.h0`, as in mali-transcendentals.md. The sequences
of the five operations themselves do not depend on that. The drm-shim arm
(`drm_shim_ref.sh`) is not built on this machine, so **no byte-for-byte
`tools/mali/testdata` reference exists for these five shaders yet**; the first
implementation commit makes them (item 1 below). The NIR pass names are from memory of Mesa 26.2.1's `nir_opt_algebraic.py`
and `bifrost_compile.c`; the instruction listings are measured.

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

```
FROUND.f32.rtn.discard r1, r0
FADD.f32 r0, r0^, r1^.neg
FADD.f32.wait0126 r0.h0, r0^, 0x0.neg       # the colour's narrowing
```

Yakogl: the instruction pair exists (`Alu::Ffloor` -> `FROUND.rtn`, `Fadd`
with `Fneg` folded as a `.neg` source). The highp lane arm builds it. Missing:
an arm for a single mediump lane and for three/four mediump lanes (a
`Word::Lanes` colour, nir_fs.rs ~6750); for a pair of fp16 lanes the existing
`FROUND.v2f16` path is used, and the three-lane case is a pair plus a lone
lane. Reference (tools/mali/testdata): `s4_ffract.*` is the highp case.

### length

NIR: `fdot3(v, v)` -> `fsqrt` -> `lower_fsqrt` (`frcp(frsq(a))`).

```
FMA.f32.discard r2, r2^, r2^, 0x0.neg
FMA.f32 r1, r1^, r1^, r2^
FMA.f32 r0, r0^, r0^, r1^
FRSQ.f32 r0, r0^
FRCP.f32 r0, r0^
```

The dot is a chain of `FMA` (`v.z*v.z + -0.0`, then `v.y*v.y +`, `v.x*v.x +`),
Mesa's order z, y, x. Yakogl: all five exist (`Fdot3` expansion, `Frsq`,
`Frcp`); the highp `Fsqrt` arm is `Frcp(Frsq(x))`. Missing: the same `Fsqrt`
arm in the mediump walk (a scalar result from three lanes). Smallest of the
five; no new bir op.

### normalize

NIR: `fdot3(v, v)`, `frsq`, three `fmul`.

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

Yakogl: every row exists (`Frsq`, `Fmul` as `FMA ... -0.0`, the `Fdot3`
chain). Missing: the mediump arm for `Frsq` (the catch-all) and a splat of the
scalar `frsq` over three lanes. No new bir op. The `Fmul` by a splat scalar is
three independent `FMA` rows; only the dot's operand order (z, y, x) and the
register reuse are to be matched by the byte test.

### reflect

NIR: `fdot3(N, I)`, `fmul(d, -2.0)`, `ffma(k, N, I)`; algebraic rules turn it
into `fadd(d, d)` and `ffma(fneg(N), d2, I)`.

```
FMA.f32.discard r3, r2, u3.w0, 0x0.neg
FMA.f32 r3, r1, u2.w1, r3^
FMA.f32 r3, r0, u2.w0, r3^
FADD.f32 r3, r3^, r3^                         # 2*dot
FMA.f32 r0, u2.w0.neg, r3, r0^
FMA.f32 r1, u2.w1.neg, r3, r1^
FMA.f32 r2, u3.w0.neg, r3^, r2^
```

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
`src0`/`src1`. This is read from one listing; confirm with the shim reference
before coding.

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

1. Make the references: run the five shaders (plus a mediump-uniform variant of
   each) on the drm-shim build, add them as `fsarith_*` testdata, and compare
   with the sequences above. Cheapest item, settles the `pow` fusion question.
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

* No byte reference yet for these five (see the caveat): the listings are
  `pan_shc`'s SPIR-V path at fp32. The register choice and `^` flags must be
  checked against the shim before an expected-words test is written.
* The `pow` fusion (item 5) is inferred from one listing.
* docs/README.md has no index line for this note: that file is outside this
  item's files (plan #327 is editing it); the planner adds it.
