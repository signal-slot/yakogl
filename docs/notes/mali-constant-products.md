<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: case 70 loopcarry, the constant products Mesa folds

**Design, implementation in fs-fold** (plan #336, issue #33). Nothing here
changes the compiler. Mesa 26.2.1 is `MESA_PAN_TREE`
(`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`); `nir_opt_algebraic.py` line
numbers are that file's. This continues mali-algebraic-folds.md (shapes 1-3
of cases 189 and 192, lowered in plan #154) for the one refusal left in a
**mediump** shader: case 70.

## 1. The case and the reference

`tools/shaders/es100/loopcarry.frag` (a byte copy of
`tools/mali/testdata/fsarith_70_loopcarry.frag` and `alu_loopcarry.frag`,
mediump):

```
float n = ceil(u_k.x * 3.0);   float k = u_k.y;   float acc = 0.0;
for (float i = 0.0; i < 8.0; i += 1.0) {
    if (i >= n) { break; }
    acc += k;
    float t = acc * 2.0;
    acc = t * 0.5;
}
gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
```

Today `tools/mali/corpus-verdicts.txt:72` gives `MaliError::ArithConstant`
for it ("a constant product in a shape Mesa's algebraic rules move or
fold"); the sweep's case 70 (`tools/cts/log/2026-11-07-t6-step4a`) is
REFUSED with the same text. **Case 71 deadloop is the same program without
`t`** (`acc += u_k.y` only, plus dead work) and PASSes. The references:

* `fsarith_70_loopcarry.expect.json` and `alu_loopcarry.expect.json`: the
  same draw twice; 29 fragment words, 32 registers, preload
  `0x3000000000000000`, FAU `[0, 0, 0, 0, 0, 40004200, 47004700, 46004600,
  45004400, 42004000]` (no `--uniform-values`: word 4, `u_k`, is 0). Vertex
  stage: the plain 24-word pass-through (`fsarith_70_loopcarry.vert`).
* `cflow_71_deadloop.expect.json`: **the same 29 fragment words and the same
  vertex words** (checked; only FAU word 4 differs, `4c404c00`, the
  `--uniform-values` of that draw). mali-alu-ops.md `:57-58` says so
  ("alu_deadloop equals alu_loopcarry").

So the target program is 71's, already byte-equal in
`tests.rs` `the_flattened_ifs_are_byte_equal_to_mesa_26_2_1`
(`CFLOW_FLATTEN`, `:6021`, the test `:6055`). The whole of case 70 is that Mesa **removes the
`* 2.0 * 0.5` pair**; nothing in the reference is new. Its words, for the
record (mediump, `v2f16`): `FMA.v2f16 r0, u2.w0, u2.w1, r0^.neg` is
`(u_k.x * 3.0, u_k.y * 2.0)` in `.h0`/`.h1` (the `* 2.0` is `acc` after two
trips, `fadd(k, k)` -> `fmul(k, 2.0)`, nir_opt_algebraic.py `:274`),
`FROUND.v2f16.rtp` (`ceil`), `FMIN.v2f16 .., 8.0` (the trip cap), then a
ladder of `FCMP_OR.v2f16` + `MUX.v2i16` + `FADD.v2f16 r2, u2.w0.h11, r2^`
(`acc + k`, six times for trips 3-8).

## 2. Which of Mesa's rules fire, and in what order

Derived by hand from the source and the reference, **not** read from a
NIR dump (no `fsarith_70_loopcarry.stderr` exists; fs-fold should draw one,
§5). The shader reaches `nir_opt_algebraic` after the GLSL IR passes, the
`lower_precision` pass (the arithmetic is fp16 from here on: a constant is
an fp16 value, `0.5` and `2.0` exact) and, in the optimisation loop, before
and after `nir_opt_loop_unroll`.

Per trip, the loop body is `acc' = fmul(fmul(fadd(acc, k), 2.0), 0.5)`.

1. `nir_opt_algebraic.py:2406`, "Reassociate constants in add/mul chains":
   `(('~fmul', '#a', ('fmul', b, '#c')), ('fmul', ('fmul', a, c), b))`.
   The outer operand `0.5` is `#a`, the inner product is `fmul(b, #c)` with
   `b = fadd(acc, k)` and `#c = 2.0` (commutative match; the first
   alternative of `:2394` needs *two* non-constant operands and does not
   match). No `is_used_once` condition. The result is
   `fmul(fmul(0.5, 2.0), b)`.
2. `nir_opt_constant_folding`: `fmul(0.5, 2.0)` is `1.0` (exact at fp16).
3. `nir_opt_algebraic.py:292`, `(('fmul', a, 1.0), ('fcanonicalize', a))`,
   then `fcanonicalize(a)` of a value only used as float is `a` (`:161-167`,
   `fcanonicalize(is_only_used_as_float)`): the product disappears and
   `acc' = fadd(acc, k)`.

All three fire in the loop body *before* unrolling (the loop's phi makes
`acc` opaque, and rule 1 does not care), so after them case 70's loop is
case 71's: `acc += k`. The unroller then makes the passes of 71:

* trip 1: `fadd(0.0, k)` is `k` (the identity, `fadd(a, 0.0)`);
* trip 2: `fadd(k, k)` is `fmul(k, 2.0)` (`:274`);
* trips 3-8: `fadd(acc, k)` stays an `fadd` (no rule matches a product plus
  its factor: the reference has the `FADD.v2f16`).

If the unroller ran first (our walk does: `walk_loop`, `nir_fs.rs:3271`
unrolls the trips and builds each as a node), the same rules fire on the
unrolled chain per trip and give the same values: trip 2's
`fmul(fmul(fmul(k, 2.0), 2.0), 0.5)` is rule 1 twice, `fmul(4.0, k)` then
`fmul(2.0, k)` -- the node trip 2's `fadd(k, k)` already made, which
`node()` CSEs (`nir_fs.rs:971`). Every intermediate constant (`1.0`, `4.0`,
`2.0`, `3.0 * 0.5 = 1.5`...) is exact in fp16; a product that is not is the
`ConstantFold` refusal that already exists (`nir_fs.rs:4041-4056`, the refusal at `:4053`).

No other rule fires. The `distributive` rules (`:246`, `:3614`) need two
products of one factor; `k` is not a product. `fceil` (`ceil(u_k.x * 3.0)`),
`fmin`, the compares and the flattened `if`s are 71's, byte-equal today.

## 3. Where our compiler stands, and where the fold goes

The refusal site is **`ladder_forms`** (`nir_fs.rs:7096-7120`; `guarded16` is `:6973`), called from
`select_highp`'s prelude (`:7502`, `if guarded16(f)`, after `nir::dce` and
`dead_cf`): it looks at the 16-bit `Fmul`/`Fadd` nodes of the unrolled
ladder and returns `MaliError::ArithConstant` for any whose one operand is a
constant and whose other is a node of the same op with a constant (its doc
comment: "`(acc * 2.0) * 0.5` is `acc`, case 70's loopcarry"). The walk
builds that chain because `Fs::op` (`nir_fs.rs:982-1034`, the 16-bit scalar
rules) has no rule for it:

| `Fs::op` rule (Fmul/Fadd, 16-bit) | NIR rule |
|---|---|
| `IdentitySecond`/`IdentityFirst`, `Zero`, `Negate` (`:989-997`, `:1001`) | `fmul(a, 1.0)` `:292`, `fmul(a, 0.0)` (inexact), `fmul(a, -1.0)` `:296`; `fadd(a, 0.0)` |
| two constants: `ConstantFold` (`:999`) | `nir_opt_constant_folding` |
| `fmul(fneg a, b)` -> `fneg(fmul(a, b))`; both negated: `ArithConstant` (`:1003-1010`) | `:2387`, `:230` |
| `fadd(fmul(a, b), fmul(a, c))` (`factor`, `:1018-1024`, helper `:1038`) | `:246` |
| `fadd(a, a)` -> `fmul(a, 2.0)` (`:1030`) | `:274` |

Similar folds elsewhere, none of which this case reaches:

* `nir_fs.rs:1960-2009` (`alu32`, the **32-bit** `Fmul` arm): rule `:2406`,
  `fmul(#a, fmul(b, #c))` -> `fmul(fmul(a, c), b)` with the folded constant
  first (measured: `s6_discloop`, `fbload_predlive_fs_coord`, prec_82). It
  is the model for the 16-bit rule, but it is gated on `self.loops > 0 ||
  y is Const || kept_local` and builds through `op32`; the mediump lanes go
  through `Fs::op`.
* `glsl.rs:1399` (`Gl::reassociate`, header `:23-28`; GLSL IR's
  `reassociate_constant`, `opt_algebraic.cpp:175-216`): `op(const, tree)` swaps the constants in the *front end's*
  trees. It does not see this program: `t` is a local, the product is
  built in the loop through the local, and a mixed-width or multi-use shape
  is refused (`MaliError::Rebalance`, `:1226`, `:1241`, `:1281`...).
* `nir.rs:2044-2056` (`Nir::alu`, `:2052` the test, `is_const_product` `:2114`): the **vertex** `Alu::Fmul` refusal of a constant times
  a product with a constant (`is_const_product`), unreached by a mediump
  fragment shader.
* `nir_fs.rs:4736-` (`MatSum`, doc comment): `:2398-2412` are "not transcribed"; the
  matrix-term sums only.
* `opt.rs` is the BIR pass list (`mod_prop`, `fuse_add_imm`, `cse`); it
  holds no NIR algebra and gets no change.

### What to add (fs-fold)

In `Fs::op` (`nir_fs.rs:982`), before the final `(AluOp::Fmul, _) =>
self.node(..)` arm and after the two-constant arm, per lane:

1. `Fmul` with one operand `Const(c2)` and the other a 16-bit node
   `Fmul` with a `Const(c1)` operand and a non-constant `b` (find it with
   `alu_of`, `:959`; it already ignores the node's width): fold
   `r = c1 * c2` in f32 and **require `f16_exact(r)`** (else keep the
   existing `Err(MaliError::ConstantFold)`: no reference rounds a
   non-exact fp16 constant product). Then the new product is
   `fmul(b, r)` run through the *same* rules as any `fmul` with a constant:
   `r == 1.0` -> `b` (rule `:292`/`:161-167`); `r == -1.0` -> `neg(b)`
   (`:296`); `r == 0.0` (or `-0.0`) -> `Const(0)` as `Rule::Zero` returns;
   otherwise `node(Fmul, [b, Const(r)])`. Put the **variable first**,
   the constant second: that is the shape `fadd(a, a)` already gives
   (`:1030`) and the one `vectorise`/the pair FMA of 71 expects (`FMA.v2f16
   r0, u2.w0, u2.w1`: uniforms first, the constant pair second). Hold the
   order if the byte test shows otherwise.
2. Both operand orders (the match is commutative: `x` const or `y` const).
3. Per lane, not per pair: `Rule` is computed per lane pair from the IR's
   constants (`:4057-4075`); the NIR rules run on scalars before
   `nir_opt_vectorize`, and 70's chain is scalar. The new rule lives inside
   `op`, not in the `Rule` enum.
4. The inner product is left behind as a dead node when it had no other
   use (rule `:2406` has no `is_used_once`); `nir::dce` (`:7499`) removes it
   before `ladder_forms`.
5. **`Fadd`:** the twin rules `:2410`/`:2411` (`fadd(#a, fadd(b, #c))`) are
   *not* needed by 70 and have no reference. Keep `ladder_forms`'s check for
   `Fadd` and leave the `Fmul` arm as a guard that should now never fire (or
   narrow it to `Fadd`); do not delete the refusal for chains with no
   reference.
6. The 32-bit twin (`alu32`'s `Fmul` arm) is unchanged.

Do not touch `glsl.rs`, `nir.rs` or `link.rs` for this case.

## 4. Tests to add (fs-fold)

In `src/shaderc/online/mali/tests.rs`:

1. `case_70_loopcarry_is_byte_equal_to_mesa_26_2_1`: sources
   `td!("fsarith_70_loopcarry.vert")` / `.frag` /
   `.expect.json`, asserted equal to `tools/shaders/es100/loopcarry.frag`
   and to `td!("alu_loopcarry.frag")` (the `deadloop` assertion at
   `:6081` is the pattern); `check_linked_for("70", vs, fs, json)`
   (`:3824`; `compile_linked` is `:471`: both IDVS programs and the 29-word fragment program with all
   facts, no uniform values). Or add it to `CFLOW_FLATTEN` (`:6047`) *if* the
   helper there can take a no-values json: it draws with
   `--uniform-values`, so use `check_linked_for`.
2. **71 stays byte-equal**: `the_flattened_ifs_are_byte_equal_to_mesa_26_2_1`
   is the guard and must pass unchanged. Add an assertion that the two
   references' `fs.fragment` `words` are equal (70 == 71), so the premise of
   §1 is pinned and a regenerated reference cannot drift.
3. Unit tests of the fold on tiny fragment shaders (compile, no reference):
   `(k * 2.0) * 0.5` is `k` (no `FMA`/`ConstantFold`), `(k * 3.0) * 0.5` is
   one `FMA` by `1.5` (exact), `(k * 2.0) * -0.5` is `fneg` of `k`
   (`FADD ..neg` / `.neg` source), `(k * 0.1) * 0.3` keeps
   `ConstantFold` (the product is not an fp16 value), and `(k + 2.0) + 0.5`
   keeps `ArithConstant` (the `Fadd` twin has no reference).
4. `tools/mali/corpus-verdicts.txt` regenerated in the same commit
   (`tools/mali/corpus-verdicts.sh`): `es100/loopcarry.frag` becomes `ok`
   (it is a generated file, `tools/check.sh` compares the bytes). Check the
   other lines of that file do not move (any other source that was
   `ArithConstant` through `ladder_forms` may become `ok` or move to its
   next refusal; list them in the commit).
5. Sentinels that must not move: the `algfold_*` references in `tests.rs`
   (`the_chain_population_is_byte_equal_to_mesa_26_2_1`,
   `the_corpus_matrix_products_by_a_constant_are_the_references`). The
   `tests/backend_differential.rs`, `alloc_clobber.rs` and `branch_splat.rs`
   lists name `es100/loopcarry.frag` too, but run the generic (sema) path of
   the other back ends, not the Mali walk; they are unaffected.

## 5. Open points

* (Section 6: the draw was redone at `04ac35d3` and its final NIR read;
  the choice between the two routes below is still open.) The NIR of the
  draw was not recorded: re-draw `fsarith_70_loopcarry` with
  the shim (`tools/mali/drm_shim_ref.sh run`, mali-alu-ops.md `:283-286`)
  and keep `fsarith_70_loopcarry.stderr`/`.nir` beside it, to confirm that
  rule `:2406` fires before the unroll (§2) and to rule out the GLSL-IR
  route (`t` grafted into `acc = t * 0.5`, `reassociate_constant` giving
  `acc * 1.0`, then `:292`). Both end in `fadd(acc, k)`; the choice only
  changes where in our walk the fold lives (`Fs::op` per node, or
  `glsl.rs` per tree). The `Fs::op` fold is the smaller one and is enough
  for the byte-equal test.
* The 16-bit fold's constant arithmetic: NIR folds an fp16 `fmul` in
  float and rounds to nearest even; `f16_exact` makes every accepted case
  exact, so the rounding mode never matters here.
* Other mediump unrolled loops (`ladder_forms`'s users: cases 71, 148, the
  corpus `loop_*` sources) may hold `(x * c1) * c2` or `(x + c1) + c2`
  chains; the new rule changes the former for them too, correctly (Mesa
  folds all of them) but unmeasured. Run `tools/mali/corpus-verdicts.sh`
  before and after and look at the diff.

## 6. Checked at 04ac35d3

Checked on the integration tip `04ac35d3` (plan #336, item fold-check; the
line numbers in sections 1-5 are that commit's, corrected in place above).
`origin/main` is 21 commits ahead (`032bae69`); against it only `nir.rs` and
`tests.rs` of `src/shaderc/online/mali` differ (+148 and +65 lines, the
vertex work), so the `nir_fs.rs`, `opt.rs` and `glsl.rs` lines hold there
and the `nir.rs` and `tests.rs` ones move: look those up by name.

**What moved.** Only line numbers, none of the steps: `CFLOW_FLATTEN` `:6047`
is `:6021` (test `:6055`), the `ConstantFold` check `:4038-4056` is
`:4041-4056` (refusal `:4053`), the `Fs::op` rule lines `:984-998` are
`:989-1001`, `alu32`'s `Fmul` arm is `:1960-2009`, `reassociate_constant` is
now `Gl::reassociate` (`glsl.rs:1399`, not `:642-760`), mali-alu-ops.md
`:259-263` is `:283-286`. Unchanged: `ladder_forms` (`:7096-7120`),
`guarded16` (`:6973`), the call in `select_highp` (`:7501-7502`),
`nir::dce` (`:7499`), `alu_of` (`:959`), `node` (`:971`), `walk_loop`
(`:3271`), `Nir::alu`'s vertex refusal (`nir.rs:2044-2056`),
`check_linked_for` (`tests.rs:3824`), `corpus-verdicts.txt:72` and
mali-alu-ops.md `:57-58`. `opt.rs` still holds no NIR algebra (`mod_prop_*`,
`fuse_add_imm`, `cse`): no change there. trunc-motion (`motion.rs`),
indexlocal-glsl (`glsl.rs`, `ilt_trees`) and the vertex notes touch none of
this item's code: the `Fs::op` rules, `ladder_forms` and the `algfold_*`
references are as written. Every line of `nir_opt_algebraic.py` cited here
(`:161`, `:167`, `:230`, `:246`, `:274`, `:275`, `:292`, `:296`, `:2387`,
`:2394`, `:2398`, `:2406`, `:2410`, `:2411`, `:2419`, `:3614`) was read in
the 26.2.1 tree and is the rule the text names.

**The reference, rerun.** Built the shim as `drm_shim_ref.sh build` does
(its meson options, the unpacked `mesa-pan/mesa-26.2.1` tree, a separate
build dir: `$CONNECTED_DEPS_DIR/mesa-shim/{build,inst}`), then drew
`fsarith_70_loopcarry` (`drm_shim_ref.sh run`, no `--uniform-values`) and
`cflow_71_deadloop` (`--uniform-values`) and extracted both with
`pan_ref_diff.py extract --draw`. Both regenerate the committed
`.expect.json` **byte for byte** (only the `trace` name differs). The
fragment words of 70 and 71 are equal, as section 1 says: 29 words, the
first `FMA.v2f16 r0, u2.w0, u2.w1, r0^.neg`. The final NIR
(`BIFROST_MESA_DEBUG=shaders`, not recorded before) shows the order of
section 2's result: `%3 = fmul %1, (3.0, 2.0)`, `%7 = fadd %3.y, %1.y`
(trip 3), then `fadd %1.y, ..` for trips 4-8, with no `2.0` or `0.5`
product left. **Which route removed the pair** (NIR `:2406` then `:292`, or
GLSL IR's `reassociate_constant`) is still not told apart: that needs
`NIR_DEBUG=print` before the algebraic pass. Either gives the same graph,
and the `Fs::op` fold is enough for the bytes.

**Next after the refusal, measured.** A throwaway (not committed): a test
of `compile_linked` + `check_stages_for` of the case 70 sources against
`fsarith_70_loopcarry.expect.json`, run three ways:

1. As committed: `ArithConstant` from `ladder_forms` (REFUSED, as the sweep).
2. `ladder_forms`'s `ArithConstant` return disabled, no fold: **no other
   refusal.** It compiles, to **45 fragment words against 29**. Words 0-3
   (`MOV`, `FMA`, `FROUND`, `FMIN`) are equal; word 4 is the first
   difference (`FMA.v2f16 r0, r0^.h11, 0x38000000.h11, 0x0.neg`, a `* 0.5`,
   where the reference has `FADD.v2f16 r2, r0.h11, u2.w0.h11`), and every
   later trip carries an `FMA .. 0x40000000` (`* 2.0`) and an
   `FMA .. 0x38000000` (`* 0.5`) around its `FADD`.
3. The refusal disabled **and** section 3's fold added (about 10 lines in
   `Fs::op`, before the last `Fmul` arm: one operand a constant `c2`, the
   other a 16-bit `Fmul` with a constant `c1` and a non-constant `b`;
   `c1 * c2` required `f16_exact`; `1.0` gives `b`, else
   `node(Fmul, [b, Const(r)])`, variable first): **case 70 is byte-equal
   to the reference**: both vertex programs, the 29 fragment words and
   every fact (registers, preload, FAU words) that `check_stages_for`
   compares. So the next refusal case 70 meets after the fold is **none**,
   and no byte difference either; fs-fold's host test (section 4, item 1)
   should pass as written. Not run with the prototype: the other Mali
   tests and `corpus-verdicts.sh` (section 4 items 2 and 4 and section 5's
   last bullet are still to do). The prototype also kept `ladder_forms`'s
   `Fmul` arm off, so that it stays silent for 70 after the fold is
   inferred (the chain it looks for, a constant `Fmul` over a constant
   `Fmul`, no longer exists); keep it, as section 3 item 5 says.

## 7. The boundary with the vertex constant-product fold

mali-vs-constant-products.md found that the vertex "constant product"
refusal is **not a fold of constants** and sits at a different site, so the
two items do not overlap in code:

| | fs-fold (this note) | vertex item (mali-vs-constant-products.md) |
|---|---|---|
| Stage | fragment, **mediump** (16-bit lanes), the unrolled-loop route (`guarded16`) | vertex, 32-bit |
| Site | `Fs::op` (`nir_fs.rs:982`) and `ladder_forms` (`:7096`) | the `AluOp::Ffma` arm of the vertex `IrOp::Alu` match (`nir.rs:2326-2336`) |
| Shape | `(x * c1) * c2` (rule `:2406`, then `:292`); the `fadd` twin (`:2410`) stays refused | `M * vec4(v, 1.0)`: `ffma(c3, 1.0, acc)` becomes `fadd(acc, c3)` (S1); `vec4(v2, 0.0, 1.0)` pushes `c3` into the first `ffma` (S2, rule `:4358`, inferred) |
| Reference | cases 70 and 71 (`fsarith_70_loopcarry`, `cflow_71_deadloop`) | `vsfold_*`, to be drawn (bump51, shadowb, sh37, shade) |
| Corpus | `es100/loopcarry.frag` (`corpus-verdicts.txt:72`) | `es100/mvstruct.vert` (`:80`), `es100/xlink.vert` (`:134`) |
| gl_test cases | 70 | none |
| glmark2 | none known | 13 vertex shaders: bump-51d0, bump-270a, pulsar-9d75, shadow-37f4, shadow-6964, shadow-b9d1, shading-83cd, shading-78e3, shading-e5f2, refract-7215, build-ace2, buffer-7234, terrain-18bc |

fs-fold covers a **fragment** `fmul` of a constant by an `fmul` (either
operand order) of a constant and a non-constant, at fp16, when the folded
product is an fp16-exact value (`1.0` gives the operand, `-1.0` its `fneg`,
`0.0` the `Zero` rule), including the products an unrolled loop builds per
trip (section 2). It does **not** change the 32-bit fragment arm (`alu32`,
`:1960-2009`), the `Fadd` twins (`:2410-2411`, no reference), `MatSum` (the
fragment matrix-term sums, `:2398-2412` not transcribed), the vertex
`Nir::alu` arms (`nir.rs:2018`, `:2054`, `:2326-2336`) or `glsl.rs`.

Left for the vertex item: everything in the right column, i.e. all 13 of
glmark2's vertex shaders. Its S1 arm needs no fold at all (acc first, then
the column), only S2's term-major issue order. The only vertex sites that
*are* constant products (`nir.rs:2018`, a negation of a negation, and
`:2044-2056`, a constant times a product with a constant) fire for no
glmark2 shader and no sweep case; neither item touches them.

glmark2 fragment shaders: `tools/mali/corpus-verdicts.txt` lists only the
repo's own sources, none of glmark2's. A grep of the glmark2 checkout's
`data/shaders/*.{vert,frag}` (`22c527cb`) for a product with two constants
on one line (`x * c1 * c2`, `c * (x * c)`) finds none, so no glmark2
fragment shape is known to need fs-fold; those shaders' next refusals are
`pow`, `normalize`, `reflect`, `length` and `fract` (ana-fsvec).
