<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: the algebraic folds of constant products (cases 189 mulone, 192 predlive)

What Mesa 26.2.1 (panfrost, G610, drm-shim) emits for the shapes that the
fragment compiler refuses as `MaliError::ArithConstant`, so that a compiler
item can lower them byte-equal. Nothing here changes the compiler.

## 1. The refusal

(Sections 1-7 are the study, with its line numbers; section 8 is the state
after plan #154's chains item lowered most of it.)

`MaliError::ArithConstant` (`src/shaderc/online/mali/mod.rs:152`, message at
`:361`): "Valhall: a constant product in a shape Mesa's algebraic rules move or
fold (a constant times a product with a constant, a negation of a negation, a
constant in a fused multiply-add) is not lowered yet". Raised at:

| site | shape |
|---|---|
| `nir.rs:1367` (`Alu::Fmul`, `self.user`) | `fmul(fneg a, fneg b)` |
| `nir.rs:1378` | `fneg(fneg a)` |
| `nir.rs:1397` | `fmul` of a constant and a product with a constant (`is_const_product`) |
| `nir.rs:1407` | `ffma` with a `1.0` or `-1.0` factor |
| `nir_fs.rs:548`, `:571` | the fragment stage's `fmul(fneg, fneg)` and `fneg(fneg)` |
| `nir_fs.rs:1101` | an `ffma` chain under a constant scale that is not the matrix shape |

`tools/mali/corpus-verdicts.txt` (current) for the two cases' sources
(`tools/shaders/gltest/`):

| source | verdict |
|---|---|
| `mulone_fs_one.frag` (189) | `ArithConstant` |
| `mulone_fs_half.frag` (189) | ok |
| `mulone_fs_mods.frag` (189) | `Rebalance` (`do_rebalance_tree`), a different refusal |
| `predlive_fs_uni.frag` (192) | `ArithConstant` |
| `predlive_fs_coord.frag` (192) | `FragCoordMediump` (`gl_FragCoord.x < 32.0` in a mediump read), a different refusal |

So after this refusal alone, `mulone_fs_one` and `predlive_fs_uni` may be the
only two that get through; `mods` and `coord` hit one more each (Rebalance;
FragCoordMediump, whose words are in `algfold_192_coord.expect.json` and in the
fragcoord notes). Whether `one` and `uni` hit a further refusal after this one
was not measured (the compiler was not run on them after a fix; this item
changes no code). Which of the sites above `one` reaches is an inference: its
NIR has the `±1.0` lane of `mat3 * vec3(0.5, 0.5, 1.0)`, i.e. the
`ffma(±1.0, ..)` site (`nir.rs:1407`); `uni` has `2.0 * (u_q * 3.0)`, the
constant-times-product site (`nir.rs:1397`).

Measured with `tools/mali/corpus-verdicts.sh` on the minimal fragments
(copied temporarily under `tools/shaders/`, then removed): `algfold_constmul`,
`algfold_negneg` and `algfold_constfma` each give `ArithConstant`;
`algfold_constfma_ctl` is `ok`. Each minimal pair therefore isolates one
refusal, and `constfma_ctl` is the passing control.

## 2. The references

`<work>` is `$CONNECTED_DEPS_DIR/mesa-shim` (a `drm_shim_ref.sh build` of Mesa
26.2.1), `D=tools/mali/testdata`; every pair is drawn with the ES 1.00
`position_only.vert` and `--uniform-values` (`u_x u_q u_a u_b` = 16 32 48 64,
which are `u2.w0 u2.w1 u3.w0 u3.w1` or the order of first use):

```
tools/mali/drm_shim_ref.sh run <work> $D/algfold_<n>.vert $D/algfold_<n>.frag <scratch>/algfold_<n> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <scratch>/algfold_<n>.trace --draw > $D/algfold_<n>.expect.json
```

| `<n>` | fragment shader | origin |
|---|---|---|
| `189_mulone` | `mat3(...) * vec3(0.5, 0.5, 1.0)` | `gltest/mulone_fs_one.frag` |
| `189_half` | the same times `vec3(0.5)` | `mulone_fs_half.frag` |
| `189_mods` | `abs(u_x - 2.0*u_a)*1.0`, `1.0 * -(u_q-u_a)`, `(u_b*1.0)*u_q*8.0` | `mulone_fs_mods.frag` |
| `192_predlive` | `if (u_x > 0.0)` with `a*2.0`, `b*0.5`, `b-a`, `a+b` | `predlive_fs_uni.frag` |
| `192_coord` | the same on `gl_FragCoord.x < 32.0` | `predlive_fs_coord.frag` |
| `constmul` | `2.0 * (u_q * 3.0)`, `2.0 * (u_a * 3.0)` | minimal, shape 1 |
| `negneg` | `-(-u_q)`, `-(-u_a)` | minimal, shape 2 |
| `constfma` | `dot(vec2(u_q,u_a), vec2(1.0,2.0))`, `dot(vec2(u_x,u_b), vec2(-1.0,3.0))` | minimal, shape 3 |
| `constfma_ctl` | `u_q*u_a + 0.5`, `u_x*u_b + 0.25` | control: a non-`±1` constant addend |

Only the fragment stage matters (the vertex stage is the same
`gl_Position = a_pos`). The expect.json `fs.fragment` regions hold the words,
text, FAU and preload. The NIR below is Mesa's final one (stderr of the draw).

## 3. Shape 1: a constant times a product with a constant

Rule, `nir_opt_algebraic.py:2406` ("Reassociate constants in add/mul chains"):
`(('~fmul', '#a', ('fmul', b, '#c')), ('fmul', ('fmul', a, c), b))`. The two
constants are folded (`nir_opt_constant_folding`) and the result multiplies the
variable once. No `is_used_once` condition: a product that has other uses stays
and a second one is made.

`algfold_constmul` (`2.0 * (u_q * 3.0)`): NIR `fmul %1.x, 6.0`, `fmul %1.y, 6.0`
(`6.0` = `0x40c00000` in the FAU). Words:

```
MOV.i32.discard r0, u3.w0            ; the shared moved operand (the 6.0)
FMA.f32 r1, u2.w0, r0, 0x0.neg       ; x * 6.0 + -0.0   (an fmul is an FMA with -0.0)
FMA.f32 r0, u2.w1, r0^, 0x0.neg
```

`algfold_192_predlive`: `r = a*2.0` with `a = u_q*3.0` is `fmul(6.0, u_q)`
(`FMA.f32 r0, u4.w0, r0^, 0x0.neg`, `u4.w0` = `0x40c00000`), computed apart
from `a = u_q*3.0` (`FMA.f32 r2, u2.w1, r2^, 0x0.neg`, `u2.w1` = 3.0), which
the colour's third lane still needs. Not folded: `b * 0.5` (`b` is an fadd, no
constant product inside).

What a lowering does: fold in the NIR pass. For `fmul(c1, fmul(x, c2))` with
both constants literal, produce `fmul(c1*c2, x)` (the folded constant in fp32,
the product computed as Mesa's constant folder does; the refusal exists
because GLSL IR's `reassociate_constant` already folds some of these in the
front end, in an order the compiler does not transcribe). The reassociated
product is emitted as the constant-first `FMA x, const, -0.0` form every
other fmul is. `nir_fs.rs:885-915` already does this under `self.loops > 0`
(unrolled loop counters); the new case is the source constant.

## 4. Shape 2: a negation of a negation

Rule, `nir_opt_algebraic.py:230`: `(('fneg', ('fneg', a)), ('fcanonicalize', a))`.
`fcanonicalize` of a value used as float is `a` (`:167`, and `:161-163` when the
denormal mode is not flush), so the pair vanishes.

`algfold_negneg` (`-(-u_q)`, `-(-u_a)`): NIR is `f2f16 %1` with nothing between;
words are the whole fragment shader:

```
V2F32_TO_V2F16 r0, u2.w0, u2.w1     ; both lanes straight from the FAU
MOV.i32 r1, 0x3C000000              ; 1.0 f16 pair, then ATEST + BLEND
```

What a lowering does: fold in the NIR pass: `fneg(fneg a)` is `a` (the
same value, no instruction). `nir.rs:1378` and `nir_fs.rs:571` can return
`Ok(a)` instead of the error. Mesa's `fmul(fneg a, fneg b)` ->
`fneg(fmul(fneg a, b))` -> `fmul(a, b)` (`:2387` twice, then `:230`) is the
same fold (`nir.rs:1367`, `nir_fs.rs:548`); this item drew no such pair.
Neither case 189 nor 192 contains a literal `-(-x)`; the shape is in the
message but not reached by them.

## 5. Shape 3: a constant in a fused multiply-add

Rules, `nir_opt_algebraic.py:311-314` (in 26.2.1 `fmul(a, -1.0)` is `fneg` at
`:296`, `fmul(a, 1.0)` is `:292`):
`ffma(1.0, a, b)` is `fadd(a, b)`, `ffma(-1.0, a, b)` is `fadd(fneg a, b)`,
commutative. `fmul(a, 1.0)` is `fcanonicalize(a)` (`:292`), so a `*1.0` term of
a matrix product or dot product disappears.

`algfold_189_mulone` (the `±1.0` case; `u_x u_q u_a u_b` = x y z w of the
uniform block) NIR:

```
%2 = fneg %1.y
%4 = ffma %1.y, 0.5, %1.w          ; q*0.5 + (b*1.0 -> b)
%5 = ffma %1.z, 0.5, %4            ; + a*0.5
%6 = fadd %2, %1.z                 ; a*0.5 + a*0.5 + (-q)*1.0 -> a - q
%7 = ffma %1.y, 0.5, %1.x          ; (0*0.5 dropped) q*0.5 + x
```

Words (FAU: `u2.w0`=x=16, `u2.w1`=q=32, `u3.w0`=a=48, `u3.w1`=b=64; `0x38000000.h1`
= 0.5):

```
MOV.i32.discard r0, u3.w1
FMA.f32 r0, u2.w1, 0x38000000.h1, r0^     ; red:   q*0.5 + b
MOV.i32 r1, u3.w0
FADD.f32 r1, u2.w1.neg, r1^               ; green: -q + a   (the fadd, not an ffma)
FMA.f32 r0, u3.w0, 0x38000000.h1, r0^     ; red:   + a*0.5
...
MOV.i32 r1, u2.w0
FMA.f32 r1, u2.w1, 0x38000000.h1, r1^     ; blue:  q*0.5 + x
```

Besides the `1.0` fold two more folds appear: a term times `0.0`
(`vec3(u_q, u_a, 0.0)` row) is dropped, and `a*0.5 + a*0.5` in green becomes
`a` (no multiply remains). The 0.5 is an f16 immediate half (`.h1`).

`algfold_constfma` (`dot(v, vec2(1.0, 2.0))`): the `*1.0` term is the addend
itself, NIR `ffma %z, 2.0, %y`; `dot(v, vec2(-1.0, 3.0))` is
`ffma %w, 3.0, fneg %x`:

```
MOV.i32.discard r0, u3.w0
FMA.f32 r0, u3.w1, 0x40000000, r0^        ; 2.0 is a full-word FAU constant here
MOV.i32 r1, u4.w0
FMA.f32 r1, u2.w1, r1^, u2.w0.neg         ; the -1.0 term folds into a source negation
```

`algfold_constfma_ctl` (`u_q*u_a + 0.5`): a non-`±1` constant addend keeps the
FMA, the constant in a `MOV.i32 r0, 0x38000000` and `FMA r0, u3.w0, u3.w1,
r0^.h1`. So "a constant in a fused multiply-add" is not itself refused: only a
`±1.0` factor (`nir.rs:1407`) and the ffma chains of `nir_fs.rs:1101`.

What a lowering does: fold in the NIR pass. A product by `1.0` in the
front end's `ffma(x, 1.0, acc)` is `acc + x` (an `FADD`), by `-1.0` it is
`acc - x` (`FADD x.neg, acc`, as green above, `FADD.f32 r1, u2.w1.neg, r1^`),
and a `0.0` factor drops the term. Keep the operand order of the references
above: `FADD` takes the negated operand first, the accumulator second.

## 6. The other lines of 189 and 192

`algfold_192_predlive`, besides shape 1: the `if` is predicated, not branched.
NIR `flt_pan(0.0, u_x)`, two `bcsel_pan`. `s = b - a` / `s = a + b` becomes
`b + bcsel(c, -a, a)` (one fadd after the select):

```
FCMP_OR.f32.lt.m1 r3, 0x0, u2.w0, 0x0     ; c = 0.0 < u_x
CSEL.u32.eq r0, r3, 0x0, r2^, r0^         ; r = c ? 6.0*q : b*0.5   (arms: r2 = b*0.5, r0 = 6.0*q)
FADD.f32 r4, r2.neg, 0x0.neg              ; -a materialised for the select
CSEL.u32.eq r3, r3^, 0x0, r2, r4^         ; c ? -a : a
FADD.f32 r3, r1, r3^                      ; b + that
```

`b = u_a + 0.125` is `FADD_IMM.f32 r1, u3.w0, #0x3E000000`. The store is the
usual f16 pack/`ATEST`/`BLEND`; `r1` of the colour is moved before `BLEND`
(`MOV.i32.wait r1, r2^`). The select shape (`flt_pan`/`bcsel_pan`) is the
predicated-if note (`mali-lowering-map.md`), not a new refusal here.

`algfold_189_mods`: `abs(u_x - 2.0*u_a)` is `ffma(-2.0, a, x)` then `fabs`
(`FMA.f32 r1, 0x40000000.neg, u2.w1, r1^` and the `.abs` source modifier of
the f16 conversion); `1.0 * -(q - a)` is `fneg(fadd(q, fneg a))`, kept as
`FADD r0, u3.w0, r0^.neg` (a - q after the later `.neg` of the conversion);
`(u_b*1.0)*u_q*8.0` is `fmul(fmul(8.0, q), b)` (`FMA r1, 8.0, q, -r1`..: the
constant first). That chain is the `Rebalance` refusal, not shape 1: the
reassociation by `do_rebalance_tree` is the item to study for it.

## 7. Summary for a lowering item

| shape | rule (26.2.1) | emitted | lowering |
|---|---|---|---|
| `c1 * (x * c2)` | `nir_opt_algebraic.py:2406` | one `FMA x, c1*c2, -0.0` | fold in the NIR pass |
| `-(-x)` | `:230` -> `fcanonicalize` -> `a` (`:161-167`) | no instruction | fold in the NIR pass |
| `ffma(±1.0, a, b)` / `a * 1.0` | `:311-314` / `:292` | `FADD a(.neg), b` / the operand | fold in the NIR pass |
| `0.0` factor | `:305-306` (`ffma(nsz,nnan)`), `fmul` by 0 | the term dropped | exists for `fmul` (`nir_fs.rs:540`); extend to the dot chain |

All four are NIR folds; none selects a different instruction.

## 8. Lowered (plan #154 chains)

State after the chains item (`glsl.rs`, `nir.rs` `Nir::alu`, `nir_fs.rs`),
each pinned in `tests.rs` (`the_chain_population_is_byte_equal_to_mesa_26_2_1`,
and `the_corpus_matrix_products_by_a_constant_are_the_references`):

| reference | state | how |
|---|---|---|
| `algfold_constmul` | byte-equal | GLSL's `reassociate_constant` (opt_algebraic.cpp:175-216), not NIR `:2406`: `2.0 * (u_q * 3.0)` swaps to `u_q * (2.0 * 3.0)` in the front end, the constants fold, `u_q` first as the words show (`FMA.f32 r1, u2.w0, r0, 0x0.neg`) |
| `algfold_189_mods` | byte-equal | the same swap on `((u_b * 1.0) * u_q) * 8.0` (docs/notes/mali-rebalance.md), then NIR's `x * 1.0` |
| `algfold_constfma` | byte-equal | `ffma(±1.0, a, b)` is `fadd(a or fneg(a), b)` (`Nir::alu`, :311-314), which the late fusion fuses: `ffma(u_a, 2.0, u_q)`, `ffma(u_b, 3.0, fneg(u_x))` |
| `algfold_constfma_ctl` | byte-equal | (was already) |
| `algfold_192_predlive`, `algfold_192_coord`, `fsout_192_uni`, `fsout_192_coord` | byte-equal | (were already, fragment shader; now both stages through the link) |
| `algfold_negneg` | byte-equal | `fneg(fneg(a))` is `a` (`glsl.rs` `negneg`, after the GLSL passes; `Nir::alu` and `Fs::neg` too), so the colour is `vec4(u_q, u_a, 0.0, 1.0)`. `u_x` is unused, so `u_q` and `u_a` are words 0 and 1 of the fragment shader's packed block, which `nir_opt_load_store_vectorize` makes one vec2 load (`mem_vectorize_cb`: 8 bytes within offset 0's alignment), converted by one `V2F32_TO_V2F16 r0, u2.w0, u2.w1` (`nir_fs.rs` `uniform_vector`, the pairing of `load_lane`) |
| `algfold_189_mulone` | byte-equal | `nir_fs.rs` `MatSum`: the front end's `m * v` by a constant `v` is GLSL's `((m0 * 0.5 + m1 * 0.5) + m2)` (`lower_mat_op_to_vec`, the term by 1.0 the column), split into scalar lanes (`lower_to_scalar`), and NIR's rules run in nir_search's order, the last instruction first: red's `fadd(fadd(fmul, fmul), b)` is reassociated (:2419) to `fadd(fadd(q*0.5, b), a*0.5)`, which the late fusion makes `ffma(a, 0.5, ffma(q, 0.5, b))`; green's the same, then `fadd(a, fadd(a, b))` (:275) and the constant reassociation (:2406, `2.0 * (a * 0.5)` is `a`) leave `fadd(fneg(q), a)`; blue's `0.0 * 0.5` folds and `ffma(q, 0.5, x)` remains |
| `algfold_189_half` | byte-equal | the same: with every term a product, red and blue meet the distributive law (:246) twice, `fmul(0.5, fadd(b, fadd(q, a)))` and `fmul(0.5, fadd(q, x))` (nir_search's commutative order puts the constant first and `b` before the sum); green's `a*0.5 + a*0.5` is `a` by the same law, and `fadd(a, fneg(fmul(q, 0.5)))` (:2387) fuses to `ffma(fneg(q), 0.5, a)` |

The `ArithConstant` sites now: `fmul(fneg a, fneg b)` (`nir.rs`, `nir_fs.rs`
`Fs::op`), a constant times a product with a constant that the IR pass did
not reach (`is_const_product`, e.g. through a local), a constant moved
across a change of width (`glsl.rs`), a matrix term by `±1.0` outside a
product by a constant vector (a scale of `-1.0`, or `1.0` beside scales
that are not constants in a shape other than femtovg's), a sum that keeps a
constant term in `MatSum` (the constant reassociations :2398-2412 are not
transcribed), and `nir_fs.rs`'s `ffma` chain under a constant scale.

