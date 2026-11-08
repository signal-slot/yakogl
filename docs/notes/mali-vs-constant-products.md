<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: the vertex "constant product", a matrix product with a constant `w` (glmark2's largest refusal family)

**Design, implementation in a later vs-fold item** (plan #336, issue #33).
Nothing here changes the compiler. This is the vertex twin of
mali-constant-products.md (case 70, fragment). Mesa is 26.2.1
(`MESA_PAN_TREE`, `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`; line numbers are
that tree's); the references below were drawn on its drm-shim (`GPU_ID
0xa867`, `tools/mali/drm_shim_ref.sh`), 64x64, `--uniform-values`. Words and
NIR are quoted as measured; what I could only read off the sources is marked
**inferred**.

**Finding that changes the item's name.** The refusal the corpus tally calls
"vertex constant product" is not a fold of constants. In all 13 glmark2
shaders that print it, it is raised at one site, `nir.rs` (the `AluOp::Ffma`
arm of the vertex `IrOp::Alu` match, `:2330-2336`): **a matrix-times-vector
term whose scale is the literal `1.0`**, which GLSL writes as `M * vec4(p,
1.0)` (and `vec4(p, 0.0, 1.0)`). The two sites that really are about
products of constants (`:2018` a negation of a negation, `:2054` a constant
times a product with a constant) fire for **no** glmark2 shader and for no
gl_test sweep case.

## 1. The glmark2 shaders and the refusal text

Corpus (not committed, GPL-3.0): `tools/hostharness/glmark2/run.sh
"$GLMARK2_SRC" <out> powervr` with `GLMARK2_SRC` the checkout at
`22c527cb0556f3a1ac4445aaa52cc532760928d5` (`$CONNECTED_DEPS_DIR/glmark2-src`),
which writes `<out>/shaders/` (69 shaders: 29 vertex, 40 fragment; `index.txt`
maps each to its scene entries). Run it with `GM2_WRITE=0` (the default
rewrites `tools/hostharness/glmark2/FINDINGS.md`). `tests/glmark2_corpus.rs`
does not run the Mali back end (its cfg is `gpu-powervr`, `gpu-vivante`,
`gpu-v3d`), so I ran a throwaway copy of it with the cfg `feature =
"gpu-mali"`, `--features host-test,gpu-mali,shaderc-online,no-shader-table`,
`forget_compiled()` between shaders, and nothing kept. Result at `e9e72f05`:
69 shaders, 11 compile, **51 are refused by the compiler** and 7 more fail
only to link with a refused partner. Every shader of the family prints, with
return code `-95`:

```
online shaderc: the code generator refused this vertex shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet)
```

(`MaliError::ArithConstant`, `mod.rs:403`). The 13 shaders, with their
scenes (`index.txt`), all raised at `nir.rs:2334` (checked with a temporary
`eprintln!` at the three `ArithConstant` returns, reverted):

| Shader | Scenes | The product |
|---|---|---|
| `bump-51d0a6207c34d50c.vert` | bump:normals | `MVP * vec4(position, 1.0)` |
| `bump-270aae3cfcae71a2.vert` | bump:height | that, and `NormalMatrix * vec4(normal \| tangent \| cross(..), 1.0)` |
| `pulsar-9d75bb1847b5cc25.vert` | pulsar | `MVP * vec4(position, 1.0)` |
| `shadow-37f46ae8961221f2.vert` | shadow, refract | `MVP * vec4(position, 1.0)`, `Normal = normal` |
| `shadow-6964fcf6aaac9739.vert` | shadow | `NormalMatrix * vec4(normal, 1.0)`, `normalize`, `max(dot, 0)`, `MVP * ..` |
| `shadow-b9d18073e5979540.vert` | shadow | `LightMatrix * pos4`, `MVP * pos4`, `pos4 = vec4(position, 0.0, 1.0)` (a vec2 attribute) |
| `shading-83cd81fbb702ee85.vert` | shading:gouraud | as `shadow-6964` |
| `shading-78e3ba59e507a5cc.vert` | shading:blinn-phong-inf, bump:high-poly | `NormalMatrix * vec4(normal, 1.0)`, `MVP * ..` |
| `shading-e5f21674435c0404.vert` | shading:phong, cel | those and `ModelViewMatrix * vec4(position, 1.0)` |
| `refract-7215f4f5052a383c.vert` | refract | those, and `LightMatrix * vec4(position, 1.0)` |
| `build-ace204f6171a58b1.vert` | build, texture:* | as `shadow-6964` |
| `buffer-723443fdbaf22e8b.vert` | buffer | four `MVP * vec4(v, 1.0)` (position, tvertex0..2) |
| `terrain-18bca803a331552c.vert` | terrain | `modelViewMatrix * vec4(position, 1.0)`, `normalMatrix * vec4(.., 1.0)` |

Two repo sources are the same family (`tools/mali/corpus-verdicts.txt:80,134`):
`tools/shaders/es100/mvstruct.vert` (`ubuf.mvp * vec4(attr_pos, 1.0)`, twice;
a struct uniform) and `es100/xlink.vert` (`u_fade.matrix * vec4(a_pos, 0.0,
1.0)`).

The count the tally gave, **18 of 55**, I get as 13 here plus 5 vertex shaders
that print the generic `arithmetic` text (`conditionals-3107..`, `-a21c..`,
`function-0162..`, `loop-3cb0..`, `ideas-51b2..`) and are 13 + 5 = 18. Four of
the five (`ModelViewProjectionMatrix * pos`, with `pos` a local I did not
read) are **inferred** to hold the same shape behind an earlier refusal of
another op; `ideas-51b2` is `normalMatrix * normal` (a mat3 times a vec3, no
constant `w`) and is not this family. I could not reproduce 55 (51
compile-refused, 58 counting the link failures).

## 2. The NIR shape, and how it differs from case 70

`mat * vec` is lowered by the front end as a sum of column terms, in Mesa's
GLSL order (`MAT_TIMES_VEC_ORDER = MesaGlsl`, `lower.rs` `mat_times_column`:
columns 1, 0, 2, 3): `acc = fmul(c1, y)`, then `ffma(c0, x, acc)`,
`ffma(c2, z, acc)`, `ffma(c3, w, acc)`, each over the four rows (the columns
are `load_uniform` vec4s). With `w = 1.0` the last term is
`ffma(c3, 1.0, acc)`; with `z = 0.0` the third is `ffma(c2, 0.0, acc)`, which
the walk already drops. Our `Nir::alu` (`nir.rs:2066`) would turn the former
into `fadd(c3, acc)` (nir_opt_algebraic.py `:311`, `ffma(1.0, a, b) ->
fadd(a, b)`), but the `Ffma` arm of the IR match refuses it first (`:2330`:
"NIR's rules meet it inside the vector sums of the product, which no
reference draws in this stage" -- this note draws it).

Case 70 (fragment) is a *chain* `fmul(fmul(acc, 2.0), 0.5)` whose two
constants Mesa folds (`:2406`, then `:292`); it needs a new rule in `Fs::op`.
The vertex shape has **one** constant, the `1.0` of the `w` splat; nothing
is folded between constants, and what the reference does is two other
things, both of which the sums of the product already contain:

* **S1, `M * vec4(v3, 1.0)`**: Mesa's NIR after the algebraic passes is the
  `ffma` chain of columns 1, 0, 2 and then a plain **`fadd(acc, c3)`** with
  the accumulator *first* (final NIR of `bump51` below). `fmul(c3, 1.0)` is
  removed by `:292` (`fmul(a, 1.0) -> fcanonicalize(a)`, which is `a` for
  fp32 without denormal flushing, `:161-167`; GLSL IR has no `x * 1.0` rule,
  mali-rebalance.md step 3), so the term keeps GLSL's `add(acc, term)`
  operand order. **Inferred** from the rules; the order is measured.
* **S2, `M * vec4(v2, 0.0, 1.0)`**: the `* 0.0` term vanishes and the `c3`
  addend is pushed down into the **first** multiply: `ffma(c0, x, c3)`, then
  `ffma(c1, y, that)`. Measured (final NIR of `shadowb`). The rule is
  **inferred** to be `reassoc_fma_optimizations`, third rule
  (`nir_opt_algebraic.py:4358-4360`): `fadd(fadd(a:fmul, b:fmul), c:not_fmul)
  -> fadd(a, fadd(b, c))`, with `is_used_once` on the inner `fadd`s. It
  explains why S1 is *not* rewritten: there the tree is `fadd(ffma(.., ffma(..,
  fmul)), c3)`, left-nested over three products, which no rule of the list
  matches. It also fixes the **issue order**: the references issue all four
  rows of the `x` term, then all four of the `y` term (term-major), as
  the vec4 expressions are rewritten before they are scalarised
  (**inferred**).

`-1.0` (`fadd(c3, fneg ..)`, `:312`) and `+-0.0` in other lanes are **not
drawn**: keep their refusal until a reference exists.

## 3. What Mesa 26.2.1 emits

Sources (fresh, so they can be committed; the glmark2 text is GPL and must not
be): `bump51.vert` is `attribute vec3 position; attribute vec2 texcoord;
uniform mat4 ModelViewProjectionMatrix; varying vec2 TextureCoord; void
main() { TextureCoord = texcoord; gl_Position = ModelViewProjectionMatrix *
vec4(position, 1.0); }` (the body of `bump-51d0`), fragment `gl_FragColor =
vec4(TextureCoord, 0.0, 1.0)` at `mediump`. `shadowb.vert` is
`shadow-b9d1`'s body (`Color = MaterialDiffuse; vec4 pos4 = vec4(position,
0.0, 1.0); ShadowCoord = LightMatrix * pos4; gl_Position =
ModelViewProjectionMatrix * pos4`, `position` a vec2), fragment
`Color + ShadowCoord * 0.01`. A third, `sh37`, is `shadow-37f4`
(`Normal = normal`, `MVP * vec4(position, 1.0)`, `Normal` read as
`vec4(Normal * 0.5 + 0.5, 1.0)`). Drawn with

    tools/mali/drm_shim_ref.sh run <shim> V F target/vsfold/<name> --uniform-values
    python3 -I tools/mali/pan_ref_diff.py extract target/vsfold/<name>.trace --draw > ...expect.json

(`<shim>`: `meson setup` of the unpacked `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`
with the options of `drm_shim_ref.sh build`, prefix `$CONNECTED_DEPS_DIR/mesa-pan/inst`,
then `meson install`; a separate build directory, the tree's `build-pan` is
`pan_shc`'s.) All three: preload `0x3800000000000000`, 32 registers,
`secondary_offset` 384.

**bump51, `vs.position`, 40 words** (`pan_ref_diff` text; the control
`ctl_mat4.vert`, `m * a_pos`, is also 40 words with the `w` term an FMA):

```
LD_ATTR_IMM.v3.f32.slot0.wait0 @r0:r1:r2, r60^, r61^, index:0x0, table:0x1   0866800622017d7c
FMA.f32 r3, u5.w0, r1, 0x0.neg      (y x4: rows x,y,z,w)  00b2c30400c0018a
FMA.f32 r3, u3.w0, r0, r3^          (x x4)                00b2c30000430086
FMA.f32 r1, u7.w0, r2, r3^          (z x4)                00b2c1000043028e
FADD.f32 r1, r1^, u9.w0             00a4c10000009241      <- the w term
FADD.f32 r2, r3^, u9.w1             00a4c20000009343
FADD.f32 r3, r4^, u10.w0            00a4c30000009444
FADD.f32 r0, r0^, u10.w1            00a4c00000009540
FRCP.f32 r0, r0^  ...               009cc00000000040
```

(`u3..u10` is the matrix in dword order, `u5` column 1, `u9` column 3.) Final
NIR (`BIFROST_MESA_DEBUG=shaders`):

```
%4 = fmul %2.x, %3.y    ... (c1 * y)      %9 = ffma %8.x, %3.x, %4   ... (c0 * x)
%15 = ffma %14.x, %3.z, %9  ... (c2 * z)  %21 = fadd %15, %20.x      ... (+ c3, w = 1.0)
```

`shade.vert` (`shading-e5f2`'s body) gives the same four `FADD.f32 r, r^,
u.w` in its varying program for `vec3(NormalMatrix * vec4(normal, 1.0))`
(three rows only: `FADD.f32 r1, r1^, u18.w0`, `r2, r3^, u18.w1`, `r0, r0^,
u19.w0`), then `FMA` squares, `FRSQ.f32` and three `FMA`s for `normalize`.

**shadowb, `vs.position`, 32 words** (no `FADD` at all; 4 words fewer than a
`vec4` position would need):

```
LD_ATTR_IMM.v2.f32.slot0.wait0 @r0:r1, r60^, r61^, index:0x0, table:0x1   0866800412017d7c
FMA.f32 r2, u3.w0, r0, u3.w1   00b2c20000870086   c0.x * x + c3.x
FMA.f32 r3, u4.w0, r0, u4.w1   00b2c30000890088
FMA.f32 r4, u5.w0, r0, u5.w1   00b2c400008b008a
FMA.f32 r0, u6.w0, r0^, u6.w1  00b2c000008d408c
FMA.f32 r2, u7.w0, r1, r2^     00b2c2000042018e   c1.x * y + (..)
FMA.f32 r3, u7.w1, r1, r3^     00b2c3000043018f
FMA.f32 r4, u8.w0, r1, r4^     00b2c40000440190
FMA.f32 r0, u8.w1, r1^, r0^    00b2c00000404191
```

The FAU pairs `c0.x, c3.x` in `u3.w0/u3.w1` (values `0x42000000`, `0x42300000`
for `--uniform-values`): the push order is the first use in the final NIR
(`%7 = ffma %6.x, %1.x, %4.x`, `%13 = ffma %12.x, %1.y, %7`).

**What ours emits today.** Nothing: both refuse. With the guard lifted
(experiment below), S1 differs from the reference in **four operand orders**
only (`FADD.f32 r1, u9.w0, r1^` against `FADD.f32 r1, r1^, u9.w0`, words
`00a4c10000004192` against `00a4c10000009241`), and S2 has 36 words
(`FMA .. 0x0.neg` for the `y` term and four `FADD`s of `c3`, the unmerged
form) against 32.

**Experiment (temporary edits to `nir.rs`/`tests.rs`, reverted; nothing
under `src/` is in this commit).** Replacing the refusal arm by

```rust
AluOp::Ffma if is_mat_term(m, &s) && x(1) == Val::Const(F32_ONE) => {
    vs.note(Prod::MatProduct);
    vs.alu(Alu::Fadd, x(2), x(0), Val::Undef)?      // acc first, column second
}
```

and comparing with `check_vs_in(vs, fs, json, VIEWPORT_64X64)`: **`bump51`
and `sh37` are byte-equal in `vs.position` and `vs.varying`**. A
reassociation of S2 added to that arm (`Fadd(Ffma(p, q, Fmul(r, s)), c3)` ->
`Ffma(r, s, Ffma(p, q, c3))`, built per lane) gives the right 32 words and the
right instruction *set*, but issues row by row (`FMA r2 c0*x+c3; FMA r2
c1*y+r2; FMA r3 ..`) where Mesa issues term by term, so it is **not** byte
equal: the S2 rewrite must create the four `ffma(c0, x, c3)` before the four
`ffma(c1, y, ..)`. Lifting the guard alone, the 13 shaders move to their next
refusal: 4 compile (`bump-51d0`, `pulsar-9d75`, `shadow-37f4`,
`shadow-b9d1`), 8 stop at `Frsq` (every `normalize`: `build-ace2`,
`bump-270a`, `refract-7215`, `shading-78e3`, `shading-83cd`, `shading-e5f2`,
`shadow-6964`, `terrain-18bc`) and 1 at a non-matrix `Ffma`
(`buffer-7234`, `length`/`cross`, `FusedMultiplyAdd`).

## 4. Implementation plan

**Arms.**

1. `nir.rs:2326-2336`, the `AluOp::Ffma` arm of the vertex `IrOp::Alu`
   match: replace the comment and split the refusal. A scale of `Const(F32_ONE)`
   on an `is_mat_term` is S1: `vs.alu(Alu::Fadd, x(2), x(0), ..)` (acc, then the
   column), as the experiment. A `F32_MINUS_ONE` scale, or a constant *column*
   (`x(0)`), keeps `ArithConstant` (no reference). Add a `Prod` production if
   `the_uniform_population_is_referenced` needs one for the new `FADD`
   (`Prod::MatProduct` already covers the arm; a new case that draws it is
   enough).
2. S2, same arm: when the accumulator `x(2)` is `Ffma(p, q, Fmul(r, s))` and
   both nodes have one use (`is_used_once` of the rule), emit
   `Ffma(p, q, c3)` for every lane, **then** `Ffma(r, s, ..)` for every lane
   (term-major; the lane loop of the arm is per lane, so this needs a
   second pass over the lanes of the same IR op, or a rewrite after the
   walk and before `dce`). `Nir::alu`'s `:2066` rule (`ffma` by `1.0` to
   `fadd(x, c)`) stays: it is the dot-product rule and keeps its order.
3. `opt.rs`: no change expected. `fuse_add_imm` does not touch a FAU operand
   and the experiment passes `bump51` with `opt.rs` as it is. Confirm that
   S2's `FMA .., u3.w1` (a uniform as the addend) survives `mod_prop`/`cse`.
4. Not touched: `nir.rs:2018` and `:2054` (the real constant products; no
   corpus shader reaches them), `nir_fs.rs` (case 70 is fs-fold's), `lower.rs`
   (the term order is already Mesa's).

**Host test** (`src/shaderc/online/mali/tests.rs`): a table like
`CFLOW_FLATTEN`, `VSFOLD: [(name, vs, fs, json); N]`, and
`the_vertex_constant_w_is_byte_equal_to_mesa_26_2_1` that runs
`check_program(vs, fs, json)` (vertex and fragment words, facts, FAU values;
`--uniform-values` as `UNIFORM_POPULATION`). References to commit in
`tools/mali/testdata/` (by the item that implements, not this one):
`vsfold_one` (S1, vec3 attribute, as bump51), `vsfold_normal` (S1 in the
varying program, a `vec3(M * vec4(n, 1.0))` varying, the pattern `sh37`
and `shade` already show), `vsfold_xy` (S2, as shadowb). The corpus
sources `es100/mvstruct.vert` (S1, and a second matrix in the varying
program) and `es100/xlink.vert` (S2) have struct uniforms, which
`--uniform-values` cannot set (`gl_ref: --uniform-values does not set a Fade
uniform`): draw them without it, as case 70 was, and compare with
`check_linked_for`. Sentinels that must not move: the 25/52
`the_uniform_population_is_byte_equal_to_mesa_26_2_1` (its counts are
pinned), `sel_u_mat4`, `sel_u_vecmat`, `the_chain_population_...`. Compile-only
tests with no reference: `vec4(p, -1.0)` and `vec4(p, 1.0)` times a
column through a *dynamic* matrix still refuse. Regenerate
`tools/mali/corpus-verdicts.txt` (`mvstruct.vert` and `xlink.vert` move) in
the same commit.

**What it unblocks.**

* gl_test sweep (256 cases): **none directly.** None of the 15 REFUSED cases
  has a `M * vec4(v, 1.0)` vertex shader (their vertex halves are
  pass-throughs); the family is glmark2's.
* glmark2 (33 scenes): by itself, **one scene**, `pulsar:quads=5:texture=false:
  light=false` -- with the S1 experiment `pulsar-9d75.vert` links with its
  fragment shader `build-4748.frag` on the host (compile and link only; not
  drawn). `shadow-37f4.vert` also links with `shadow-928e.frag`, but the
  scenes that use them (shadow, refract) need other programs. The
  remaining 8 of the 13 wait for **vertex `normalize` (`Frsq` plus
  `Fdot3`-style FMAs, vs-alu-ops.md item 6)**, which is the next family and
  should be planned together with this one: `build`, `texture` (3 modes),
  `shading` (4), `bump` height and high-poly, `shadow`, `refract`, `terrain`
  need it; their fragment shaders then meet `pow`/`normalize`/`reflect`/
  `length`/`fract` (ana-fsvec). `bump:normals` needs the fragment `bump-487d`.

**Open points.**

* The S2 rule is inferred from the source, not read from a pass trace; the
  items' first step is to run `NIR_DEBUG=print` on `shadowb` (or print the NIR
  before and after `nir_opt_algebraic` with the shim) and confirm `:4358`
  fires on the vec4 expression, before the order is built into the walk.
* `-1.0` and `0.0` as `w`, and a constant *row* instead of a constant
  column, are undrawn.
* Whether the 4 `conditionals`/`function`/`loop` vertex shaders are S1 behind
  another refusal (section 1) is checked only by their next compile after
  the vertex compare/clamp item (vs-cmp-clamp) and the `Frsq` item.
