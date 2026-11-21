<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the small unowned dEQP-GLES2 refusal texts at `7b540fee` (references)

**Status: references, no compiler change** (plan #387, item `misc2-ref`;
issues #7, #20). This note covers the ten small refusal texts of
`mali-deqp-families.md` section 18.2 that no plan #387 item names as its
own. For each text it gives the cases and shapes measured at the tip, the
references (this item's `deqp_misc2_*` and other items'), what Mesa 26.2.1
does where a reference shows it, the refusing sites, and a lowering order.
HOST-ONLY. Earlier study: `mali-deqp-misc.md` (plan #377; `Invariant`,
`Spill`, `TooManyValues`, `TexLanes`, `AlgebraicForm`), `mali-rebalance.md`,
`mali-algebraic-folds.md`.

## 1. The tip and the measure

- Tip: the integration tip **`7b540fee`**
  (`7b540fee70f91c784120de3e1fca8cd8a9fa4421`). The measure ran on this
  item's branch at `e02af218`. That branch adds only this item's docs and
  testdata to `7b540fee` (`git diff --stat 7b540fee e02af218 -- src
  Cargo.toml capi` is empty), so the compiler measured is `7b540fee`'s.
- Command (section 18's, 4 cores):

  ```sh
  tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
      --out "$PWD/build/measure" --jobs 3 --chunk 100
  ```

  CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, fetched as `deps.toml`
  DEQP_WORK does it (clone, check out the commit, `external/fetch_sources.py`).
  The three files of `tools/cts/cts-yakogl-target.patch` were added
  (uncommitted, as `cts-build.sh` does it). Cases: the 10,597 of the slices
  `2026-11-15-t6-gles2-S{1..4}-37305608`. All began. 106 processes exited 1
  and 2 exited 139, as at `9ac31f30`. The script reported 6,132 cases with a
  host refusal.
- **Result: every count of section 18.2 holds at the tip.** The ten texts
  are the first refusal of 57 cases, the same 57 as section 18.2's per-text
  counts. Section 18.2 and this note's first version (`ced6caff`) named
  some cases only by group or by an older case of the same shape. The case
  lists in section 2 are measured and replace those names.
- The references were drawn with Mesa 26.2.1's drm-shim
  (`tools/mali/deqp_ref.sh <vert> <frag> <out> --instrument shim`). The shim
  was rebuilt by `deqp_ref.sh` into `$CONNECTED_DEPS_DIR/mesa-shim` from
  `$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz`. Self-check: a redraw of
  `gm2_cf_ifelse_vs` is byte-equal to the committed `.expect.json` except
  for its `trace` file name (`cmp` of the rest is silent).

## 2. Per text: cases, shapes, references, sites

Cases: first refusal per case at `7b540fee` (all board `Fail`, `NS` 0).
Stage: the shader the measure names as refused. Sites: `grep
MaliError::<variant>` at `7b540fee`, outside `tests.rs` and the message
table.

| # (18.2) | Text (`MaliError`) | Cases | Measured cases (dEQP-GLES2.functional.) | Shapes | References | Refusing sites |
|--:|---|--:|---|---|---|---|
| 28 | `Rebalance` | 12 (vs 8, fs 4) | `shaders.random.all_features.vertex.{14,35,54,84}`, `.fragment.{6,24,82}`; `shaders.invariance.{highp,mediump,lowp}.common_subexpression_1`; `shaders.functions.misc.multi_arg_float_{vertex,fragment}` | `funcsum` (2): `sum(v) = v.x + v.y + v.z + v.w` through a function. `invariant_chain` (3): the `a + b + c + d + e` sum under an invariant `gl_Position`. `random` (7): seed-generated chains | **new** `deqp_misc2_rebalance_funcsum_fs`, `_invariant`, `_random_fs`, `_random_vs`. The `funcsum` vertex case is arith-ref's `deqp_arith_other_x_vs_mediump`: same pair, same words (checked, `cmp` silent apart from the `trace` name), so no second copy | `glsl.rs:51`, `:1255`, `:1270`, `:1275`, `:1310`, `:1334`, `:1338`, `:1344`, `:1346`, `:1372`, `:1379`, `:1383`, `:2566`, `:2571` |
| 31 | `Invariant` | 9 (vs) | `shaders.invariance.{highp,mediump,lowp}.common_subexpression_{0,2,3}` | `invariant_arith`: an invariant `gl_Position` computed with arithmetic | `deqp_misc2_invariant_cse` (verbatim `highp.common_subexpression_0`), `deqp_misc2_invariant_posonly` (our text) | `nir.rs:5264` |
| 32 | `LinkedVertexNotLowered` | 8 (vs, at the link) | `attribute_location.bind_aliasing.cond_{float,vec2,vec3,vec4,mat2,mat3,mat4}`, `.max_cond_mat4` | `bind_aliasing`: two attributes bound to one location, selected by a `bool` uniform | none. **Skipped**: the words depend on the attribute locations, and `tools/mali/gl_ref.py` binds attributes in declaration order (`:433`). Two attributes at one location cannot be drawn without a tool change | `mod.rs:2021` (`compile_vs_mapped_blocks`) |
| 33 | `AlgebraicForm` | 8 (fs) | `shaders.matrix.mul.const_{highp,mediump,lowp}_{mat2,mat4}_float_fragment` (6); `shaders.random.comparison_ops.fragment.{13,46}` (2) | `constmat` (6); `random_cmp` (2): constant-folded globals and a negative constant times a varying lane in the colour | `constmat`: `deqp_misc_constmat_*`, `deqp_constprod_alg_constmat_fs` (other items). **new** `deqp_misc2_algebraic_random13`, `_random46` | `nir_fs.rs:1491`, `:3208`, `:6289`, `:9691`, `:12750` |
| 34 | `AttributeNumbering` | 8 (vs) | `shaders.random.basic_expression.vertex.{49,75}`, `.conditionals.vertex.{25,75,95}`, `.scalar_conversion.vertex.{69,97}`, `.swizzle.vertex.44` | All eight: an attribute lane of `v_color` multiplied by a constant zero, so its only read goes away. `zero_lanes` (5): the zero is `0`, `0.0` or `false`. `dead_attrs` (3: `basic_expression.vertex.{49,75}`, `conditionals.vertex.75`): the zero is `-0.0`, and other attributes are read only by dead code | **new** `deqp_misc2_attrnum_zero_lanes` (`scalar_conversion.vertex.97`), `deqp_misc2_attrnum_dead_attrs` (`basic_expression.vertex.49`) | `nir.rs:3972`, `nir.rs:3991` |
| 38 | `FragmentTooLarge` | 4 (fs) | `uniform_api.info_query.struct_in_array.mat4_mat2_fragment`, `uniform_api.value.assigned.by_pointer.{get_uniform,render}.struct_in_array.mat4_mat2_fragment`, `uniform_api.random.20` | `struct_array_compare`: `uniform structType u_var[3]` (a `mat4` and a `mat2` each), every lane compared with a constant | none. **Skipped**: `gl_ref.py --uniform-values` cannot set a struct uniform ("does not set a structType uniform (u_var[3])"). Item fclane2-struct adds that | `vec16.rs:247`, `:593`, `:980`, `nir_fs.rs:8311` |
| 40 | `Fp16Inexact` | 3 (fs, the relinked pair) | `shaders.matrix.sub.const_{highp,mediump,lowp}_mat3_mat3_vertex` | `mat3sub` | `deqp_constop_mat3sub_vs` (constop) | `vec16.rs:530`, `nir.rs:2131`, `:2986`, `:3190`, `nir_fs.rs` (18 sites) |
| 41 | `Spill` | 2 (vs) | `shaders.matrix.mul.uniform_{lowp,mediump}_mat4_mat4_vertex` | `spill` | `deqp_misc_spill_{lowp,mediump}_mat4_mat4` (plan #377) | `ra.rs:697` |
| 42 | `TooManyValues` | 2 (vs) | `uniform_api.random.3`, `uniform_api.random.79` | `uniform_api_random`: about 220 lines of struct and array uniform compares in the vertex shader | none. **Skipped**: both declare struct uniforms (`structType0 u_var1`), which `gl_ref.py` cannot set | `nir.rs:2024`, `:4294`, `:5792`, `:5986`, `select.rs:1114`, `nir_fs.rs:4160`, `:7199`, `:10938`, `:10958`, `:12117`, `:12236` |
| 43 | `TexLanes` | 1 (fs) | `shaders.struct.uniform.sampler_nested_fragment` | `struct_sampler`: `texture2D(s.b.a, v_coords.xy * s.b.b + s.a).rgb` with a sampler in a nested struct | none. **Skipped**: struct uniform `S s`, `gl_ref.py` cannot set it | `nir_fs.rs:8704`, `:9898`, `:9905`, `:10640` |
| | **Total** | **57** | | | | |

Owned elsewhere in practice. Section 18.2 lists these texts as unowned. But
`mali-arith-lowering.md` step 8 takes `Rebalance` (with
`other_x_vs_mediump`, the `funcsum` vertex case). `mali-constprod-lowering.md`
S8 takes the `constmat` factoring of `AlgebraicForm` (6 of 8).
`mali-deqp-constop.md` step 7 (H1) takes `Fp16Inexact` (all 3). This item
adds references for the `Rebalance` shapes that arith step 8 has none for,
and for `AlgebraicForm`'s two random cases.

## 3. The references of this item (`tools/mali/testdata/deqp_misc2_*`)

Every draw printed `glGetError 0x0`. The sources written by `deqp_ref.sh`
were checked equal (`cmp`) to the input pair. The dEQP pairs are verbatim
from the measure's `src/` (the program of the case's first refusal), with
`.license` sidecars (The Khronos Group Inc.). Words per region:

| Reference | Case | vs.position | vs.varying | fs.fragment | Refusal at `7b540fee` |
|---|---|--:|--:|--:|---|
| `invariant_cse` | `shaders.invariance.highp.common_subexpression_0` | 34 | 25 | 15 | `Invariant` |
| `invariant_posonly` | our text | 34 | 0 | 7 | `Invariant` |
| `rebalance_funcsum_fs` | `shaders.functions.misc.multi_arg_float_fragment` | 24 | 15 | 25 | `Rebalance` |
| `rebalance_invariant` | `shaders.invariance.highp.common_subexpression_1` | 68 | 75 | 15 | `Rebalance` |
| `rebalance_random_fs` | `shaders.random.all_features.fragment.24` | 24 | 7 | 44 | `Rebalance` |
| `rebalance_random_vs` | `shaders.random.all_features.vertex.35` | 24 | 19 | 7 | `Rebalance` |
| `algebraic_random13` | `shaders.random.comparison_ops.fragment.13` | 24 | 13 | 16 | `AlgebraicForm` |
| `algebraic_random46` | `shaders.random.comparison_ops.fragment.46` | 24 | 8 | 11 | `AlgebraicForm` |
| `attrnum_zero_lanes` | `shaders.random.scalar_conversion.vertex.97` | 24 | 18 | 7 | `AttributeNumbering` |
| `attrnum_dead_attrs` | `shaders.random.basic_expression.vertex.49` | 24 | 22 | 7 | `AttributeNumbering` |

**What Mesa does.** The instructions quoted are from the shim's
`BIFROST_MESA_DEBUG=shaders` output (`drm_shim_ref.sh run`) of the same
pair.

- **Invariant** (`invariant_cse`, `invariant_posonly`). Every operation of
  the invariant output is `exact`: products are `FMA.f32 a, b, 0x0.neg`
  (Mesa's `fmul`), sums are separate `FADD.f32`, nothing is contracted, and
  there is no reassociation. 1.0e20 and 1.0e-20 are FAU words (`u3.w0`,
  `u3.w1`). The viewport transform (`nir_lower_viewport_transform`, not
  exact) stays fused: `FMA.f32 rN, rN^, uK.w0, uK.w1`. In `cse`, `nir_opt_cse`
  merges the non-invariant `v_unrelated`'s equal term with the position's,
  and the merged term stays `exact`. Its four lanes are stored as `x+s, z+s,
  x+s, z+s` (four `STORE.i32.estream`, two values): Mesa's link does not
  merge these equal lanes. `posonly`'s varying program is empty and its
  fragment program is the constant colour. `cse` schedules the four
  position `FADD.f32 a.c, s` as x, z, y, w; `posonly` as x, y, z, w.
- **AttributeNumbering** (`attrnum_zero_lanes`). `v_color = vec4(bool(d),
  float(b), c, int(b)) * vec4(0, 0, 1, a)` with `a = 0`. Mesa folds `b *
  0.0` to 0 (GLES does not preserve NaN or signed zero here). The varying
  program has **no `LD_ATTR`**: it stores four constants (`IADD_IMM.i32
  ..., #0x0`, `#0x0`, `#0x3E800000` = 0.25, `#0x0`). Attribute `b` is still
  declared, but nothing reads it. Our walk numbers attributes before that
  fold, so it does not know `b` is gone, and refuses (`nir.rs:3972`/`:3991`).
- **AlgebraicForm** (`algebraic_random46`). Only the last `gl_FragColor =
  vec4(0, c * b, a, 0)` survives; the three writes before it are dead.
  `c = 0.25 * float(-1)` folds to -0.25, which enters as a negated inline
  constant: `FMA.v2f16 r1, 0x34000000.neg.h11, r0.h00, 0x0.neg`. Then
  `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` and `U16_TO_U32 r2, r0^.h1` pack the
  colour `(0, -0.25*b, a, 0)`.
- **Rebalance**. GLSL's `do_rebalance_tree` reassociates chains of three or
  more `+` or `*` before NIR (`mali-rebalance.md`). `rebalance_invariant`
  carries both problems: the chain and the invariant output.
  `opt_rebalance_tree.cpp` skips invariant assignments, so the
  `gl_Position` chain is not rebalanced and stays exact, while
  `v_unrelated`'s chain is rebalanced.

## 4. Lowering order (one-session steps)

Ranked by the cases each step can free, then by cost. Only the ten texts
are counted. A case freed from its first refusal passes only if its later
refusals are gone too. The invariance cases' next text is `unitype`
(measure, `next_family`), so they also wait for unif2's `u_color` step.

1. **`AttributeNumbering` (8): the zero-product fold.** Before the walk
   numbers attributes, apply Mesa's `fmul x, 0.0 → 0.0` rule (with `false`
   as 0.0) to the varying lanes, and drop the attributes that are then
   unread. Byte-compare `attrnum_zero_lanes` (5 cases). Then
   `attrnum_dead_attrs` (3 cases): there the zero is `-0.0`, so check in its
   disassembly what Mesa makes of `x * -0.0` before writing the rule, and
   drop the attributes only dead code reads. This text has no other owner.
   One session.
2. **`Invariant`, step A (`posonly`) (prepares 9 + 3).** Carry an `exact`
   flag from each invariant output back through its operands. For an exact
   value: no `fmul`+`fadd` contraction (`FMA.f32 a, b, -0` then
   `FADD.f32`), no inexact algebraic rule, and no rebalance. The viewport
   transform stays fused. Remove the refusal at `nir.rs:5264` for these
   trees only. Byte-compare `invariant_posonly`.
3. **`Invariant`, step B (`cse`) (9).** The exact flag goes through CSE
   into a non-invariant varying that shares the term. Also needed: the
   position adds' lane order, and four stores of two values. That
   conflicts with our `VaryingDuplicate` refusal, which assumes Mesa's link
   merges equal lanes. Byte-compare `invariant_cse`.
4. **`Rebalance` (12), with arith step 8.** `do_rebalance_tree` is a GLSL
   IR pass on `+`/`*` chains. Byte-compare `rebalance_funcsum_fs`, arith's
   `other_x_vs_mediump`, `rebalance_random_fs`, `rebalance_random_vs`.
   Then, after steps 2 and 3, `rebalance_invariant`, where the invariant
   chain is not rebalanced. The `random` cases (7) may hit further texts
   once the chain is lowered. Arith step 8 owns this step. These
   references give it the cases its own reference does not cover.
5. **`AlgebraicForm`'s random pair (2).** Dead colour writes and a
   negated constant in a product (`0x3400.neg`). Byte-compare
   `algebraic_random46` and `algebraic_random13`. Small; can go with
   constprod S8 (`constmat`, the other 6).
6. **`LinkedVertexNotLowered` (8, plus the 6 `Defect(Encode)` cases of
   the same group, section 18.4).** First `gl_ref.py` needs an explicit
   `glBindAttribLocation` list, so two attributes can share a location.
   Then draw `bind_aliasing.cond_float`. Then lower `compile_vs_mapped_blocks`
   (`mod.rs:2014-2022`): walk the vertex shader at the link's location map
   instead of declaration order. Probably two sessions.
7. **`Spill` (2).** Mesa's mediump vertex `mat4 * mat4` is `FMA.v2f16` on
   fp16 FAU halves and fits 32 registers (`mali-deqp-misc.md`). It needs the
   mediump vertex walk, so it goes after `intval-vs16` / `intval-vs16-3`.
8. **`FragmentTooLarge` (4), `TooManyValues` (2), `TexLanes` (1).** All
   seven read struct uniforms. First draw the references once fclane2-struct
   has struct members in `gl_ref.py`
   (`uniform_api.info_query.struct_in_array.mat4_mat2_fragment`,
   `uniform_api.random.3`, `shaders.struct.uniform.sampler_nested_fragment`).
   Then raise the no-std arena bounds (`vec16.rs`, `nir.rs`, `nir_fs.rs`):
   Mesa has no such bound. These shaders are also dense in unif2's struct
   and array shapes, so raising a table alone would move them to another
   refusal. Take them last.

Not in this order: `Fp16Inexact` (constop step 7) stays with that item.

## 5. Issue comment (for the person to post on #20, copy to #7)

```text
misc2-ref (plan #387): the ten small unowned Mali refusal texts, host dEQP measure at integration tip 7b540fee
(tools/mali/deqp-host-measure.sh, VK-GL-CTS 93bca01, the 10,597 cases of the 37305608 slices).

First-refusal counts at the tip equal mali-deqp-families.md 18.2's: Rebalance 12, Invariant 9,
LinkedVertexNotLowered 8, AlgebraicForm 8, AttributeNumbering 8, FragmentTooLarge 4, Fp16Inexact 3,
Spill 2, TooManyValues 2, TexLanes 1 (57 cases, all board Fail). Case lists, now measured:
docs/notes/mali-deqp-misc2.md section 2 (e.g. TexLanes is struct.uniform.sampler_nested_fragment,
AlgebraicForm's two random cases are random.comparison_ops.fragment.13/46, Rebalance is
functions.misc.multi_arg_float_*, invariance.*.common_subexpression_1 and random.all_features).

New Mesa 26.2.1 references (tools/mali/testdata/deqp_misc2_*, test tests/mali_deqp_misc2.rs, 10 ignored with
their refusal pinned; refusals_pinned passes): invariant_cse, invariant_posonly, rebalance_funcsum_fs,
rebalance_invariant, rebalance_random_fs, rebalance_random_vs, algebraic_random13, algebraic_random46,
attrnum_zero_lanes, attrnum_dead_attrs.

Findings: Mesa keeps an invariant output's operations exact (FMA.f32 x, y, -0 products, separate FADD.f32),
keeps the viewport transform fused, and CSE carries exact into a non-invariant varying. AttributeNumbering
is an attribute lane times a constant zero (all 8 cases): Mesa folds x * 0.0 to 0 and the varying program
reads no attribute.

Not drawn: bind_aliasing (gl_ref.py cannot bind two attributes to one location) and the
FragmentTooLarge / TooManyValues / TexLanes shaders (struct uniforms; waiting for fclane2-struct's gl_ref.py change).

Lowering order: note section 4 (AttributeNumbering fold first, then Invariant A/B, Rebalance with arith step 8,
AlgebraicForm random pair, bind_aliasing, Spill after the mediump vertex walk, struct-uniform texts last).
```

## 6. `gl_ref.py --attrib-bindings` and the bind_aliasing references (`deqp_misc3_*`)

Plan #387, item `board-baseline-rows`. This is the tool change step 6 of
section 4 asked for.

- **Option.** `tools/mali/gl_ref.py --attrib-bindings a_0=1,a_1=1,...` calls
  `glBindAttribLocation` with the listed locations before the link; two names
  may share one. A name not listed takes the lowest free run of locations after
  the listed ones are removed, in declaration order. A listed name the vertex
  shader lacks, a bad item, or the option with `--divisors` stops the run with
  exit 2. Without the option the numbering and every existing reference are
  unchanged (spot check: `deqp_misc2_invariant_cse` regenerates byte-identical).
  `tools/mali/deqp_ref.sh` forwards the option (shim instrument only; it
  makes shim the default). Help is in `gl_ref.py`'s header; tests in
  `tools/mali/test_gl_ref.py` (`python3 -I tools/mali/test_gl_ref.py`).
- **References.** `tools/mali/testdata/deqp_misc3_bind_{cond_float,cond_vec2,
  cond_vec3,cond_vec4,cond_mat2,cond_mat3,cond_mat4,max_cond_mat4}`: `.vert`,
  `.frag`, `.bind` (the bindings as `name location` lines, as
  `deqp_defects_encode_*.bind`) and `.expect.json`, made with
  `deqp_ref.sh ... --instrument shim --attrib-bindings <the .bind pairs>` on
  Mesa 26.2.1 (drm-shim built the documented way into
  `$CONNECTED_DEPS_DIR/mesa-shim`); every draw `glGetError 0x0`. `cond_T`
  is dEQP's `BindAliasingAttributeTest` with offset 0: `a_0` of type T read
  when `u_A != 0.0`, a vec4 `a_1` read when `u_A == 0.0`, both bound to
  location 1. `max_cond_mat4` is `BindMaxAliasingAttributeTest`: four pairs
  of mat4 at locations 12, 8, 4, 0. **Caveat:** the CTS clone and the `.qpa`
  were not available when these were made, so the shader texts are written
  from the generator's code (`generateVertexShaderTemplate`, the same form as
  the verbatim `max_cond_float` pair of `deqp_defects_encode_*`), not copied
  from a run; check them against the next host measure's `.qpa` and redraw if a
  byte differs.
- **What Mesa does.** Aliased names share one load: each location has one
  `LD_ATTR_IMM`, whichever name reads it (`cond_float`: one load,
  `index:0x0`). The record `index` counts the *read locations in ascending
  order*, not the location number: location 1 alone is `index:0`;
  `max_cond_mat4`'s locations 12, 8, 4, 0 are loaded as index 3, 2, 1, 0.
  `cond_mat2` loads two records (`a_0`'s two columns, `index:0x0` v4 and
  `index:0x1` v2, since `vec4(mat2)` is the four floats of both columns);
  `cond_mat3` loads a v4 and an f32 (`index` 0 and 1: three floats of column 0
  and the first of column 1); `vec4(mat4)` reads column 0 only, one load. This is what
  `compile_vs_mapped_blocks` must produce from the link's location map.
- **Test.** `tests/mali_deqp_misc3.rs`: eight `#[ignore]`d byte-compares and
  `refusals_pinned`, which pins the link's refusal (`the Valhall back end of
  a vertex shader at the link's attribute locations is not lowered yet`).
  They compile the way `glBindAttribLocation` + `glLinkProgram` do
  (`compile_bound`, as `tests/mali_deqp_defects.rs`).

## 7. Measured sites at `31a4a6c1`

Plan #387, item `misc2-sites`. Tip: `31a4a6c1184a5cc5a18c6be9eb2bb024b29842ac`.
Method: a scratch test (not committed) loaded each `deqp_misc2_*` pair through
`mali_ref::compile_pair`, as `tests/mali_deqp_misc2.rs` does (features
`host-test,gpu-mali,shaderc-online,no-shader-table`). Every
`MaliError::{Rebalance,Invariant,AlgebraicForm,AttributeNumbering}` in
`glsl.rs`, `nir.rs` and `nir_fs.rs` printed `file!():line!()` when built
(temporary, reverted; `git status` clean, nothing under `src/` committed). The
site is the last one printed before the refusal text returned. Steps are
section 4's. Line numbers are at `31a4a6c1` (the `nir.rs` ones moved from
section 2's `:5264` to `:5303`).

| Reference | Refusal text (start) | Site | Step of section 4 |
|---|---|---|---|
| `invariant_cse` | an invariant output (Mesa keeps its arithmetic exact, unfused) | `nir.rs:5303`: `m.outputs.iter().any(is_invariant) && !invariant_copies(m)`, before any lane is walked | 3 (after 2) |
| `invariant_posonly` | the same | `nir.rs:5303` | 2 |
| `rebalance_funcsum_fs` | three or more additions or multiplications in a chain | `glsl.rs:1275`, `visit`: `Reduction::Unknown if self.chain(v) >= 3` (the `sum(v)` chain of four lanes) | 4 |
| `rebalance_invariant` | the same | `glsl.rs:1275`. The `Rebalance` comes first, so the `Invariant` of `nir.rs:5303` is not reached | 4 (after 2 and 3) |
| `rebalance_random_fs` | the same | `glsl.rs:1379`, `dsw`: `nord != nids`, a span of the tree's ids whose node count differs from its instructions | 4 |
| `rebalance_random_vs` | the same | `glsl.rs:1379` | 4 |
| `algebraic_random13` | a shape Mesa's algebraic rules rewrite without a reference | `nir_fs.rs:1491`: the `fadd` factoring arm, `fmul(a, b) + fmul(a, c)` whose constants' sum is not 1.0 | 5 |
| `algebraic_random46` | the same | `nir_fs.rs:1491` | 5 |
| `attrnum_zero_lanes` | a vertex shader whose attribute numbering waits for a fold ... | `nir.rs:3991`: the late-fold check `depends & !vs.output_inputs != 0` (a lane's constant differs between the first and the second walk, and it depends on an attribute no output lane of the first reads) | 1 |
| `attrnum_dead_attrs` | the same | `nir.rs:3991` | 1 |

Notes on reading the table:

- `glsl.rs:2566` and `:2571` (`post_order`) also printed in
  `rebalance_random_*`, seven times each, but they are `ok_or(MaliError::
  Rebalance)`, whose argument is built eagerly. Nothing failed there. The
  refusal is `:1379` only.
- `nir.rs:3972` (the walk of the second pass failing) is not the site of
  any of the ten: both `attrnum_*` references get through the second walk
  and stop at the comparison of `:3991`.
- Both `algebraic_*` references stop at the same arm (`:1491`), not at
  `:3208`, `:6289`, `:9703` or `:12764`. Which two products the arm factors
  in each shader was not read (the shaders were not traced further). A
  lowering of step 5 starts at this arm.

### `x * -0.0` (the open check of step 1)

Result, read from the `vs.varying` text of
`deqp_misc2_attrnum_dead_attrs.expect.json` (its `v_color.x` is `j * e *
float(-0.0)` with `j = 2.375`, `e` an attribute): **Mesa folds `x * -0.0`
to a constant `+0.0`, like `x * 0.0`.** The first varying store (lane x) is
`IADD_IMM.i32 r4, 0x0, #0x0`, stored with no attribute load and no `FMA`. The
word is `+0.0` (all bits zero), not `-0.0` (`0x80000000`). The sign of the
zero is not kept, because GLSL ES lets the compiler ignore signed zeros and
NaN here. So the rule of step 1 is the same for the `-0.0` references:
`fmul(x, ±0.0) → +0.0`, with the result `+0.0` whatever the sign of the zero
and of `x`. Compare `attrnum_zero_lanes`, whose x, y, w lanes are `+0.0`
by the same rule (`IADD_IMM #0x0`) and whose z is the constant `#0x3E800000`.

Facts about the same disassembly that step 1 has to reproduce:

- The other lanes of `dead_attrs` are constants folded the same way: lane z
  (`-1.0 * 1.0 * h`, `h = float(0.75 * i)` = -0.1875) is `IADD_IMM #0x3E400000`
  (0.1875). Lane y (`4.0 * d * f`) is `LD_ATTR_IMM` `index:0x2`, `FMA.f32 r0,
  0x44000000.h1, r0^`, `LD_ATTR_IMM` `index:0x4`, `FMA.f32 r0, r0^, r1^`. Lane
  w (`-1.5 * i * g`) is `LD_ATTR_IMM` `index:0x1`, `FMA.f32 r1, u5.w1, r1^`
  (`u5.w1` = 0.375). Three attributes are loaded, with indices 1, 2 and 4.
- The attributes `b`, `c` (read only by dead code) and `e` (read only through
  the `* -0.0` product) have no load. The indices are not 1, 2, 3: the gap at
  3 shows Mesa still numbers some attribute not loaded. Which numbering rule
  gives 1, 2, 4 for `g`, `d`, `f` is not read from these two references and has to be
  settled by the lowering of step 1 (compare `max_cond_mat4`'s
  location-order indices in section 6).
- Only two references carry this text here (`attrnum_zero_lanes` and
  `attrnum_dead_attrs`), covering all eight cases (5 and 3, section 2). The `-0.0`
  question concerns `attrnum_dead_attrs` alone: `attrnum_zero_lanes` has
  only `0`, `0.0` and `false`.
