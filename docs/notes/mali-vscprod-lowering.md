<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex constant-product refusal at the tip, and the design of its open step K3

**Status: design** (plan #387, item `vsconstprod-design`; issue #7). Nothing
under `src/` changed. Measured at the integration tip **`6e9e515c`**
(2026-11-28). The new references are `tools/mali/testdata/deqp_vscprod_*`
(14), pinned by `tests/mali_deqp_vscprod.rs`. Every count is HOST-ONLY: a
compile or link outcome on this host, never a board result. Rechecked at
main `56b5878a`: the 14 references are still refused with this text
(`route.sh measure`, the test's `refusals_pinned`). The sites are named by
their lines at `56b5878a` (the `nir.rs` guard moved from :2628 to :2634;
`glsl.rs` and `mod.rs` did not change). The case counts are those of
`6e9e515c` and were not measured again.

The text is `MaliError::ArithConstant` (`src/shaderc/online/mali/mod.rs:461`):
"a constant product in a shape Mesa's algebraic rules move or fold (a
constant times a product with a constant, a negation of a negation, a
constant in a fused multiply-add)". The Verdin iMX95 whole-tree run (images
`e63ab28e`, `tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/console/*.log`)
has **304** lines `refused this vertex shader: Mali(Valhall: a constant
product …`, one per case, all Fail. The plan called it the a3d8d2de run; the
committed consoles with exactly 304 such lines are the `e63ab28e` slices,
and this note uses them.

Read first: `mali-constprod-lowering.md` (steps S1-S8, here K1-K8 as in
`mali-lowering-schedule.md` 2.7), `mali-kfold-lowering.md` (K2, K4, K5, and
its section 2.1, which first measured the K3 site), `mali-nonfin-lowering.md`
(NF1 is K3, as kfold 2.1 corrected), `mali-vs-constant-products.md`.

## 1. What was already there, and the gap this note fills

- **K1** (a negated literal first, `glsl.rs` `neg_products`) landed after
  `e63ab28e` (`f9d2a3ea`). It takes **71** of the 304 cases off every
  refusal: they compile and link on the host.
- **K2** (two negated literals; a negated literal times a product) has a
  design and references: `mali-kfold-lowering.md` (`deqp_kfold_k2_*`) and
  `mali-constprod-lowering.md` S2 (`deqp_constprod_{neglit_pair,chainlit}_*`).
  Its owner is train 4, commit 9 (`mali-lowering-schedule.md` 5.5a).
- **K8** (`-2.0 * a` of a matrix, `hoist_operand` declines) and the `mixed`
  reassociation (`reassoc_mixed_vs`, part of K3 in constprod S3) have
  references in `deqp_constprod_*`.
- **K3**, the `is_const_product` guard of `Nir::alu`, had five references
  (`deqp_constprod_constchain_{degrees,attrib,loop}_vs`, `constchain_fs`,
  `reassoc_mixed_vs`), the two `deqp_cfold2_*_inot` of nonfin NF1 and the
  seven `deqp_nonfin_*` (dynamic-read forms, with the undef NaN), and no
  design step beyond constprod S3's "step 0 names which operand". kfold 2.1
  counted it at 176 cases and found no item for it. **This note is that
  step's design**, with 14 new references for the forms the earlier ones do
  not cover (static index chains, the mediump and invariance forms, random
  shaders).

## 2. How this was measured

- **Cases.** The 304 cases of the console lines (CRLF stripped), and then
  every case the iMX95 slices refused (5,677, the `refusals.tsv` of the six
  slices), so that cases that moved to this text from another one are
  counted too:

```sh
S=; for d in tools/cts/log/*imx95-gles2*; do S="$S --slice $d"; done
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" \
    $S --out "$TMPDIR/m-all" --chunk 100
```

  on the deqp-gles2 of `tools/cts/deps-env.sh --build HOST_DEQP_CTS`
  (VK-GL-CTS `93bca01`). All 5,677 began, none crashed. **The tip does not
  build as it is**: `capi/Cargo.toml` has `vivante-st` twice (lines 41 and
  85, from `75a7d901` and `110080eb`), so every cargo command on the
  workspace fails ("duplicate key"). The measure and the tests below ran
  with line 41 removed in the working tree, not committed.
- **Sites.** A scratch, uncommitted probe: every
  `Err(MaliError::{ArithConstant,FusedMultiplyAdd})` of `glsl.rs`, `nir.rs`
  and `nir_fs.rs` printed `file!():line!()` (one-line edits, the `ok_or`s
  made lazy, so the line numbers are the tip's). Each of the 232 distinct
  programs of the measure's `programs.tsv` that belong to a vertex
  first-refusal case went through `tests/mali_ref`'s `compile_pair`; a case
  is counted at the site of its first program refused with this text.
  Reverted; `git status` shows no change under `src/`.
- **References.** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_vscprod_<form> --instrument shim`, the pair
  verbatim from the `.qpa`, on the drm-shim that `tools/mali/drm_shim_ref.sh
  build $CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz
  $CONNECTED_DEPS_DIR/mesa-shim` made. The redraw of
  `deqp_constprod_constchain_degrees_vs` was byte-identical to the
  committed one. `tools/mali/g310_check.sh 'deqp_vscprod_*'`: **14
  identical, 0 differ** on the G310v2. Each shader's `.license` names its
  case.

## 3. The sites at the tip

249 cases have this text first **in a vertex shader** (Fail 177, NotSupported
72 on the board). Their board texts were this one (137), the non-finite
constant text (107: the `indexing` cases of `mali-nonfin-lowering.md`,
moved here by ConstantFold step A), and 5 others.

| Site (`56b5878a`) | Step | What refuses | Cases | Of the 304 | Design, references |
|---|---|---|--:|--:|---|
| `nir.rs:2634` (`Nir::alu`, `Alu::Fmul if self.user && a constant && is_const_product(a or b)`) | **K3** | a constant times a product with a constant | **173** | 62 | **this note**; `deqp_vscprod_*`, `deqp_constprod_constchain_*`, `deqp_cfold2_*_inot`, `deqp_nonfin_*` |
| `glsl.rs:1751` (`neg_products`, both factors `fneg(ConstF32)`) | K2 | two negated literals | 56 | 56 | kfold K2 |
| `glsl.rs:1823` (`neg_products`, `clamped \|\| chain`) | K2 | a negated literal times a product | 15 | 14 | kfold K2 |
| `glsl.rs:1830` (`hoist_operand` declines) | K8 | `-2.0 * a` of a matrix | 3 | 3 | constprod S8, `litfirst_mat_vs` |
| `glsl.rs:1633` (`reassociate_constant`, `mixed`) | K3 (mixed) | a swap across a change of width | 2 | 2 | constprod S3, `reassoc_mixed_vs` |

The other 167 of the 304 at the tip: 71 compile (K1), 38 stop at the vertex
`Arithmetic` text, 8 at the fragment twin of this text, and the rest at 12
other texts with 11 or fewer cases each.

### 3.1 The 173 cases of K3, by form

| Form | dEQP group | Cases (board Fail / NS) | Mesa | References |
|---|---|--:|---|---|
| **F1** index chain: `coords = coords * 0.5` in a loop of 2-4 iterations, unrolled | `indexing.tmp_array` 32, `varying_array` 32, `matrix_subscript` 24, `vector_subscript` 18 | 106 (34 / 72) | **folds** | `idx_tmp_float_static`, `idx_tmp_vec4_staticloop`, `idx_tmp_vec3_dynwrite`, `idx_varying_vec2_static`, `idx_matrix_mat2_static`, `idx_vector_vec4_static`; `cfold2_{zero_operand,const_other}_inot`, `nonfin_*` |
| **F2** `degrees`/`radians`, then `color * 0.04 + 0.50` (`* 25.0`) | `operator.angle_and_trigonometry.{degrees,radians}` | 16 (16 / 0) | **keeps both** | `degrees_mediump_vec3`, `radians_highp_float`; `constprod_constchain_degrees_vs` |
| **F3** `(v / 10.0 + 1.0) / 2.0`, its lanes summed and halved | `default_vertex_attrib.{vec4,mat4}` | 16 (16 / 0) | **factors, then folds** | `attrib_mat4`; `constprod_constchain_attrib_vs` |
| **F4** products under `invariant gl_Position`: `value *= 1.9` unrolled (`loop_0/3/4`), `c / 1e2 / 1e2` (`common_subexpression_2`) | `invariance.{highp,mediump,lowp}` | 12 (12 / 0) | **keeps every product** | `inv_loop0_mediump`, `inv_loop3_lowp`, `inv_cse2_mediump`; `constprod_constchain_loop_vs` |
| **F5** random expressions with products of constant vectors | `random.{scalar_conversion 7, basic_expression 8, exponential 3, swizzle 2, comparison_ops 2, texture 1}` | 23 (23 / 0) | folds (per lane) | `rnd_expr24`, `rnd_sconv4` |

All 14 new references are refused at `nir.rs:2634` with this text (the
probe), so `refusals_pinned` holds them all in `REFUSED`. The test is green
at the tip and at the result commits of `lower-vsloop` (`cbb6b45b`,
`8180095c`) and `lower-train-2` (`13259474`), none of which touches the
guard.

## 4. Mesa's words per form

Quoted from each `expect.json` (`vs.varying`, or `vs.position` where the
value feeds `gl_Position`). `.h11` of an immediate is its high half
duplicated: `0x38000000.h11` is the fp16 pair (0.5, 0.5), `0x34000000.h11`
(0.25, 0.25).

- **F1, folds.** `idx_tmp_float_static` (`arr = [c, c*0.5, c*0.25,
  c*0.125]`, summed): `FMA.v2f16 r1, r0.h00, 0x38000000.h11, r0.h00` (`c *
  0.5 + c`, the source's order), `FMA.v2f16 r1, 0x34000000.h11, r0.h00,
  r1^.h00` (`0.25 * c + ..`: **the folded constant first**), `FMA.v2f16 r0,
  u5.w1, r0^.h00, r1^.h00` (`0.125 * c`, FAU `30003000`: 0.125 has no
  immediate). The rule is `nir_opt_algebraic.py:2406`, `('~fmul', '#a',
  ('fmul', b, '#c')) -> ('fmul', ('fmul', a, c), b)`: it fires after the
  loop is unrolled, whatever else reads the inner product (`c * 0.5` stays
  for `arr[1]`), and its folded product stands first; the late fusion makes
  the sum's `ffma`s. The vec3/vec4 forms are the same per lane pair
  (`FMA.v2f16 r2, 0x34000000.h11, r0, r2^`). `idx_varying_vec2_static`
  stores each element: `FMA.v2f16 r1, 0x34000000.h11, r0, 0x0.neg`.
  `idx_vector_vec4_static` (`tmp[i] = coords.x; coords = coords.yzwx *
  0.5`): the folded pair (0.25, 0.125) is one FAU word `30003400`,
  `FMA.v2f16 r1, u6.w0, r1^, 0x0.neg`, then the `MUX.v2i16` of the vector
  insert. `idx_matrix_mat2_static` has one product left, `FMA.v2f16 r2,
  r0.h10, 0x38000000.h11, 0x0.neg`: the second iteration's `* 0.5` is dead
  (its result is never read), and our guard refuses it before dead code is
  dropped. `idx_tmp_vec3_dynwrite` (a loop bounded by `ui_four`, a static
  read) has the same words as the static form and no compare: Mesa's words
  hold no guard for the uniform bound.
- **F2, keeps both.** `degrees_mediump_vec3`: `FMA.v2f16 r0, r0^, u5.w1,
  0x0.neg` (`x * 57.3`, `5329`), a `MKVEC.v2i16` that builds `color`, then
  `FMA.v2f16 r1, r1^, u6.w0, 0x38000000.h11` (`* 0.04 + 0.5`, `291f`). The
  vector built by `color.xyz = res` stands between the two products, so the
  rule's `fmul(b, #c)` does not match. `radians_highp_float`: `FMA.f32 r0,
  r2^, u5.w1, 0x0.neg` (`3c8efa35`), the narrowing `FADD.f32 r0.h0, r0^,
  0x0.neg`, then `FMA.v2f16 r0, r0^, u6.w0, 0x38000000.h11` (`* 25.0 +
  0.5`): a conversion between them as in `constchain_degrees_vs`.
- **F3, factors then folds.** `attrib_mat4`: `FMA.f32 r, v, 0x3DCCCCCD,
  0x3F800000` per lane (`v * 0.1 + 1.0`), `FMA.f32 r3, r3^, 0x38000000.h1,
  0x0.neg` (lane w: `* 0.5`), `FADD.f32 r0, r0^, r2` and `FMA.f32 r0,
  0x34000000.h1, r0^, 0x0.neg`: `(p.x*0.5 + p.z*0.5) * 0.5` is
  `nir_opt_algebraic.py:246` (`fadd(fmul(a, b), fmul(a, c))` ->
  `fmul(a, fadd(b, c))`) and then `:2406`, giving `0.25 * (x + z)`.
- **F4, keeps every product.** `inv_loop0_mediump` (`vs.position`):
  `FMA.v2f16 r2, r2^, u3.w0, 0x0.neg` twelve times (and its `r3` twin), one
  per iteration, the single constant `3f9a` (1.9). `inv_loop3_lowp`:
  `FMA.v2f16 r3, r3^, u3.w0, 0x0.neg` per iteration (`3c66`, 1.1).
  `inv_cse2_mediump`: `c / 1e2 / 1e2` is two products by the same `211f`
  (0.01), `FMA.v2f16 r2, r2^, u3.w1, 0x0.neg` twice, not one by 0.0001. The
  values feed `invariant gl_Position`, which `propagate_invariance`
  (`glsl_parser_extras.cpp:2612`) marks precise, and an exact `fmul` is not
  matched by a `~` rule. (`v_unrelated` reads the same values; its own sums
  are not precise, but the products are.)
- **F5, folds per lane.** `rnd_expr24`, `rnd_sconv4`: the constant vectors
  and their products are FAU words of folded lanes (`4400bd00`,
  `b800bd00`, ...) and a product kept with the folded constant first where
  the rule took it (`rnd_sconv4`: `FMA.v2f16 r1, u7.w1, r2^, r1^`;
  `rnd_expr24`: `FMA.f32 r0, r0^, 0x38000000.h1, r2^`). Which lane holds
  which fold was not traced lane by lane; the byte-compare decides.

What is **inferred, not measured**: that the folded constant of an fp16
`fmul` is the fp16 product of the two fp16 constants rounded once (NIR's
constant folding at 16 bits). Every F1 constant is a power of two, where any
order gives the same bits; F5's words are where it would show.

## 5. The steps

The guard is one predicate in the shared `Nir::alu` (`nir.rs:2623-2634`,
`self.user` only). Each step below replaces part of the refusal with the
rule Mesa applies, and leaves the rest refused.

1. **VC1, keep where Mesa keeps (F2 + F4, 28 cases).** The guard must not
   fire where Mesa's rule cannot match: (a) the inner product reaches the
   outer one through a vector build or a conversion (F2: in `Nir::alu` the
   operand is then a lane of a `MKVEC`/`f2f16` node, not an `Fmul` node, so
   the test needs the IR's view: the GLSL value that is multiplied is a
   constructor or a precision change); (b) the value is exact because it
   reaches `invariant gl_Position` (F4: `propagate_invariance`'s precise
   set; `glsl.rs` knows the `invariant` declaration and the def-use chain to
   the position store). Both forms then go on with the products as written
   (`ffma(x, c, 0)` per product, as the references show). References:
   `degrees_mediump_vec3`, `radians_highp_float`, `inv_loop0_mediump`,
   `inv_loop3_lowp`, `inv_cse2_mediump`, `constprod_constchain_{degrees,loop}_vs`.
   Groups: `operator.angle_and_trigonometry.{degrees,radians}.*_vertex` (16),
   `invariance.*.{loop_0,loop_3,loop_4,common_subexpression_2}` (12). The
   lowest risk: it only removes a refusal where no fold is due; what these
   cases meet next is not measured (the probe stops at the first error).
2. **VC2, the constant reassociation (F1 + F5, 129 cases).** In `Nir::alu`,
   `fmul(#a, fmul(b, #c))` (either side, a commutative match) becomes
   `fmul(fmul(a, c), b)`: the two constants folded at the product's bit size
   to one `Val::Const`, written **first**, the inner product kept for its
   other readers. Not for an exact value (VC1 b). A product that no output
   reads must be dropped before the guard sees it
   (`idx_matrix_mat2_static`; `edf06e44` already builds nothing for an
   unread value, so the guard must run on the built values only).
   References: the six `idx_*`, `rnd_expr24`, `rnd_sconv4`,
   `cfold2_{zero_operand,const_other}_inot`. Groups: `indexing.{tmp_array,
   varying_array, matrix_subscript, vector_subscript}.*_vertex` /
   `varying_array.*` (106: 34 board Fail, 72 board NotSupported, 44 of
   these with a loop bounded by a uniform; where a case reads an element no
   iteration wrote it then needs NF2's partial unroll and NF3's NaN word,
   `mali-nonfin-lowering.md`), and `random.*` (23). Forecast: the 34
   board-Fail `indexing` cases are the ones that may compile; the rest move
   to their next text.
3. **VC3, factoring before the fold (F3, 16 cases).** `fadd(fmul(a, b),
   fmul(a, c))` -> `fmul(a, fadd(b, c))` (`:246`) where `a` is the same
   constant, then VC2. References: `attrib_mat4`,
   `constprod_constchain_attrib_vs`. Groups:
   `default_vertex_attrib.{vec4,mat4}.vertex_attrib_*` (16, all Fail). After
   VC2, because it produces VC2's input.
4. **The mixed swap (`glsl.rs:1633`, 2 cases,
   `random.all_features.vertex.17/.94`)** stays with constprod S3's
   `reassoc_mixed_vs`; it is GLSL IR's `reassociate_constant` across a
   width change, a different function. Below 100 cases; last or left out.

Counts are first refusals; a random shader that passes this guard nearly
always meets another refusal, so the cases that compile are fewer.

## 6. Order relative to the schedule

- K3 had **no item** (`mali-lowering-schedule.md` 2.7, 4b item 6). VC1-VC3
  are K3. They edit `nir.rs` `Nir::alu` only (VC1 b reads `glsl.rs`'s
  invariance), not `vectorise`, not `glsl.rs` `neg_products`: they can go
  into **train 4** right after K2 (commit 9), as commits 9a (VC1), 9b (VC2)
  and 9c (VC3), before K4 (commit 10). K2 and K3 are independent sites; K2
  first only because train 4 already lists it.
- **Nonfin NF1 is VC2** (kfold 2.1): train 3b's NF3 waits on VC2 and on the
  loops' LS2-vs, not on K2. If train 3b goes before train 4, NF3 stays out
  of 3b, as schedule 5.4 already allows.
- The `IndexedLocal` follow-up (156 cases, `mali-deqp-idxloc.md`) waits on
  the K folds; VC2 is the one for the `indexing` chains.
- G310 and G610 words are identical for all 14 references, so the steps
  serve both boards.

## 7. Open, for the planner

- The tip's workspace does not build (`capi/Cargo.toml`'s duplicate
  `vivante-st`): one line to remove, outside this item.
- Which text each K3 case meets after VC1-VC3 is not measured; a prototype
  of VC1 with the host measure of the 173 cases would give it.
