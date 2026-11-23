<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP colour pairs: a constant then a varying lane, a varying lane and 0.0 (Mali)

Study (plan #377, item study-colourpair). No code, no references made. Source
of the counts: `refusals.tsv` of the four slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` (whole tree at
37305608). The shader texts were not regenerated for this note (that is the
`measure` item); the shapes below come from the case names, the refusal texts
and the unit tests that pin each refusal. Items marked *to confirm* need the
host measure's shader pair.

## The two refusals

| MaliError | Text (mod.rs) | Cases |
|---|---|---|
| `ColourPair` | "a colour pair of a constant then a varying lane" (`mod.rs:266`, text `:523`) | 142 |
| `ColourZeroHalf` | "a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32)" (`mod.rs:275`, text `:527-529`) | 29 |

All 171 are refusals of the **fragment** shader (the text says "fragment
shader" or "link error: ... fragment shader"); none is in a vertex stage. most
are `*_vertex` cases of dEQP's operator tests, where the vertex shader
computes the value and the fragment shader only forwards the `v_color` varying
(`gl_FragColor = v_color`-shaped), the rest are `random.*` (23) fragment/combined
shaders and `default_vertex_attrib.float`.

## Refusal sites

`ColourPair` is raised at two places, both in `nir_fs.rs`:

- `nir_fs.rs:8374`, `word_of`: the word `(Const, lane)` of a **highp**
  fragment shader (`!f.prec.highp` is the mediump arm just above, `:8372`,
  which builds `Word::Lanes` and is drawn by `intbig_case72`).
- `nir_fs.rs:6918`, the `Leaf` classification of a pair (`vectorise`):
  `(None, None, Val::Const(_), _)` with the second lane a varying load
  (`f.input_lane(b)`), mediump or not: the constant is the low half.

`ColourZeroHalf` is raised at:

- `nir_fs.rs:6890`: `(Some((v, i)), None, _, Val::Const(c))` with `f16(c)` =
  0: a load lane then 0.0.
- `nir_fs.rs:7227`, `lane_word`: a lone computed tree `a` then 0.0, unless
  `vec_tree` (the only drawn form is `fsvec_length_mediump`'s `U16_TO_U32 r0,
  r0^.h0` of a length; `Leaf::Zext` exists for it).

Both refusals sit **before** the back end: `bir.rs:492` has `U16ToU32` and
`opt.rs:345-375` `lower_mkvec_swz` already rewrites `MKVEC.v2i16 x, #0` into
`U16_TO_U32 x` (and `x, x` into `IADD.v2u16 .h00`), so the refusal is
"no reference was drawn", not a missing op. `Leaf::Zext` and `Leaf::Lane {x,
half}` exist in the pair classifier.

## What Mesa 26.2.1 emits (fp16 colour packing)

From the merged notes and references (the Mesa tree is not in this repository;
the file:line cites are those the sources and notes already carry):

- The colour is stored at fp16 for a mediump `gl_FragColor`:
  `nir_lower_mediump_io` converts the stored colour right before its store
  (`bifrost_nir.c:914`, `nir_lower_mediump.c:102-104`, as cited at
  `nir_fs.rs` doc of `Conversions`). Lanes of the link-stage colour lose the
  `f2f32(f2fmp(a))` pair (`nir_opt_algebraic.py:1991`).
- The fp16 narrowing of an fp32 lane is folded into its producer:
  `FADD.f32 rN.h0, x, 0x0.neg` (mali-varying-constants.md:119-120; also
  mali-fragment-outputs.md:385).
- Two lanes become one 32-bit blend word with `MKVEC.v2i16 rN, a.h0, b.h0`;
  `blend` takes the words in `r0:r1` unchanged (`BLEND.v4.f16`,
  mali-fragment-outputs.md:302; mali-varying-constants.md:120-121, 161, 412-413).
- A constant half is a FAU/inline operand: `MKVEC.v2i16 r2, r0^.h0,
  u2.w0.h0` (x, 1.0) with `u2.w0 = 0x3c003c00` (mali-varying-constants.md:120,
  226, 240, 413). The lane is **first**, the constant second: that is what
  every reference draws.
- `bi_lower_mkvec_swz` (`bi_lower_mkvec_swz.c:242-260`): `MKVEC.v2i16 x, #0`
  becomes `U16_TO_U32 x` (`:256-257`), so (lane, 0.0) is one conversion and
  needs no MKVEC; `(x, x)` is `IADD.v2u16 x.h00, 0` (`:92-102`, Valhall
  `build_swz_v2i16_to`). Repo copy: `opt.rs:341-375`, `emit.rs:300, 368, 399`.
- The (constant, lane) order has **no** drawn reference for a highp fragment
  shader or a straight load lane. The mediump masked-colour case
  (`intbig_case72.expect.json`) draws `MKVEC.v2i16 u2.w1.h0, r0^.h0`: the
  constant is src0 as an FAU half. A (const, load) pair is expected to be the
  same op with the load's half in src1 (the load's `.h0` after its own
  conversion, or the second half of a converted aligned pair, `.h1`) *to
  confirm with the reference*.

## Shapes

The vertex shaders of dEQP's operator tests write `v_color = vec4(res, ...)` or
`vec4(res.xy, 0.0, 1.0)`-like colours by size of the result; the fragment
shader forwards the varying and the linked fragment shader ends up with a
constant z/w pair next to the varying x/y pair (the vertex's constants are
propagated by the link, mali-varying-constants.md). Counts are from the
refusal text of each case; the grouping is by case name:

| Shape | Refusal | Cases | Where |
|---|---|---|---|
| (const, lane): result vec2 forwarded, a constant lane before the varying lane | ColourPair | 142 less the 17 random rows | binary_operator add/sub/mul/div (+ `_assign_result`/`_assign_effect`) `*_vec2_*_vertex`, `*_float_vertex` mixed, `unary_operator.plus/minus`, `selection.*_vec2/ivec2_vertex`, `geometric.normalize.*vec2_vertex` |
| (lane, 0.0): a float result in x, a constant 0.0 in y | ColourZeroHalf | 29 | binary_operator add/sub/mul/div `*_float_vertex` (12), `default_vertex_attrib.float` (8), `inversesqrt` (2), `tmp_array` (1), `random.basic_expression` vertex 4, combined 1, fragment 1 (6) |
| (const, lane) in `random.*` fragment shaders | ColourPair | 17 (vertex 6, fragment 3, combined 1, conditionals 4, texture 2, trigonometric 1; to confirm: the lane may be a computed tree, not a load) | `random.basic_expression.*`, `conditionals.*`, `texture.fragment`, `trigonometric.fragment` |

Splits to confirm from the measure's shader pairs: how many of the 142 are
the load-lane form (`:6918`, `:8374`) and how many `Leaf::Lane`'s tree form,
and whether any is mediump (the mediump `Word::Lanes` arm is already taken at
`:8372`, so the highp rows are the ones that reach `:8374`).

### Relation to #367 (FRAGCOLOR_LANE)

`ColourForm` ("a gl_FragColor lane other than the varying's own lane or a
constant", 793 cases, `mod.rs:263`, raised at `nir_fs.rs:6919`, `:6915-6916`
and the `index`/`Lanes` arms above it) is #367's. The two here are the two
narrow refusals `word_of`/`vectorise` raise **before** reaching that
catch-all, for pairs that *are* the varying's own lane plus a constant. They
share the classifier (`vectorise`, `Leaf`), so a `Leaf::Lane`/`Leaf::Mk`
generalisation done for #367 may take these too; the plan item
`lower-colourpair` should be rebased on #367's merged `nir_fs.rs` and verify
the two sites still exist. The shapes with a non-own lane (swizzles) stay
#367's.

## Lowering plan (one session)

1. **References (item ref-colourpair)**, `tools/mali/testdata/deqp_colourpair_*`,
   by the pan_shc method of `gm2_*`, from one pair per shape of the measure's
   `tools/mali/testdata/deqp_shapes/`:
   - `deqp_colourpair_ck_lane`: highp, `vec4(1.0? ...)`: (const, load lane)
     pair — the operator `*_vec2_vertex` shape with the fragment `vec4(0.0/1.0,
     v.x, ...)`. (Drawn forms to compare: `MKVEC.v2i16 c.h0, lane.h0`.)
   - `deqp_colourpair_ck_lane_mediump`: the same at mediump (guards the
     `:8372` arm).
   - `deqp_colourpair_ck_pair_second`: constant then the second half of an
     aligned converted pair (`.h1`).
   - `deqp_colourpair_lane_zero_load`: (load lane, 0.0): `U16_TO_U32 x.h0`
     (`:6890`); `sel_vc_zero` exists but is mediump with a float varying;
     add the highp/vec4-first-lane variant.
   - `deqp_colourpair_lane_zero_tree`: (computed tree, 0.0) of a form
     `vec_tree` does not take (`:7227`): `random.basic_expression.fragment.68`.
   - `deqp_colourpair_default_attrib`: the `default_vertex_attrib.float` shape
     (a constant attribute value forwarded, vertex stage default 0.0 lanes).
2. **Lowering**, all in `nir_fs.rs`, no new ops:
   - `:6890` and `:7227`: drop the `ColourZeroHalf` error and return
     `Leaf::Zext` from the load (`Leaf::Mk` with half 0 -> `Zext(Half)`), the
     same leaf `vec_tree` already uses; relax `vec_tree`'s guard to any lone
     tree once the references fix the register/ordering.
   - `:6918` and `:8374`: build `Leaf::Mk {k, c, half}` with the constant as
     src0 (`MKVEC.v2i16 c.h0 FAU, lane.h0`); `word_of` returns
     `Word::Lanes(a, b)` for highp too (the mediump arm already does).
   - Check `opt.rs:lower_mkvec_swz` does not rewrite the (const, lane) form
     (src1 const 0 only), and the FAU packing (`fau.rs`) of a constant half in
     src0 for both 0.5/1.0 words and for `-0.0`.
3. **Test** `tests/mali_deqp_colourpair.rs` byte-compares `fs.fragment` of
   each reference (and the vertex words where the shim program is byte-equal);
   move `tests.rs:1231` and `:1239` to "compiles"; keep `tests.rs:8090` (a
   `ColourPair` of a different cause: the attribute-numbering fold) and fix it
   if it now compiles; `tools/mali/corpus-verdicts.txt` and `cases.txt` rows
   follow if a glmark2 shader's verdict changes (none is expected: glmark2 does
   not draw these pairs).
4. **Size**: two classifier edits and one leaf form; the work is the
   references. The dEQP unblocks 171 cases at most; each may stay red for a
   neighbouring refusal (`measure` gives the next refusal per case).

## References to make (list)

`deqp_colourpair_ck_lane`, `_ck_lane_mediump`, `_ck_pair_second`,
`_lane_zero_load`, `_lane_zero_tree`, `_default_attrib`; each `.vert`,
`.frag`, `.expect.json` by `tools/mali` with `MESA_PAN_TREE`, `PAN_SHC`.

## Open points

- Counts per shape are by case name, not by generated shader; the measure
  item gives the exact split.
- Whether Mesa orders (const, lane) as `MKVEC c, lane` for highp is *not*
  measured; the mediump case 72 is the only reference of the form.
  Settled by `deqp_colourpair_ck_lane_highpcolour` (References below):
  `MKVEC.v2i16 r1, 0x0.h0, r0.h0`, the constant is src0.

## References (item ref-colourpair, 2026-11-16)

Five Mesa 26.2.1 drm-shim references,
`tools/mali/testdata/deqp_colourpair_<shape>.{vert,frag,expect.json}`, each
`.vert`/`.frag` with a `.license` sidecar (dEQP's text, The Khronos Group
Inc.). They follow the `gm2_*` method (`README-pan_shc.md`, "The shim
driver"). For each `<shape>`:

```sh
tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/deqp_colourpair_<shape>.vert \
    tools/mali/testdata/deqp_colourpair_<shape>.frag <out>/deqp_colourpair_<shape>
tools/mali/pan_ref_diff.py extract <out>/deqp_colourpair_<shape>.trace --draw --attributes \
    > tools/mali/testdata/deqp_colourpair_<shape>.expect.json
```

The draw uses the defaults: 64x64, the whole viewport, and no
`--uniform-values` (none of the programs has a uniform). `<work>` is a
drm-shim build with `drm_shim_ref.sh build`'s meson options. It was configured
on the 26.2.1 source of the pan_shc tree (`build_pan_shc.sh`) with the
`subdir('pan_shc')` line taken out again, because the tarball could not be
downloaded on this host. The 159 vendored source files listed in
`third_party/mesa-26.2.1/UPSTREAM` match its sha256. Three committed
references from the same build are byte-equal (`pan_ref_diff.py diff-ref`, 0
differ): `tri` (2 regions), `sel_vc_zero` and `gm2_arithconst_1` (3 regions
each). Two draws of `ck_lane`, `default_attrib` and `ck_lane_highpcolour` are
identical.

| Reference | Case | Source | Fragment program (Mesa) | Pins for lower-colourpair |
|---|---|---|---|---|
| `ck_lane` | `shaders.operator.binary_operator.add.highp_float_vec2_vertex` (board: ColourPair) | `deqp_shapes/colourpair_constant_then_varying.{vert,frag}` on main (3b926d42), verbatim | 7 instrs: `LD_VAR_BUF_IMM.f32...v4`, `V2F32_TO_V2F16 r0, r0, r1`, `V2F32_TO_V2F16 r1, r2, r3`, `ATEST r1.h1`, `BLEND.v4.f16` | **no colour pair**: Mesa keeps all four lanes of `v_color` as varyings, the constants included (varying shader: 4 `STORE.i32.estream`, `0.0` from `IADD_IMM #0x0`, `1.0` from `#0x3F800000`) |
| `ck_lane_mediump` | `...add.mediump_float_vec2_vertex` (ColourPair) | rebuilt: `ck_lane` with the precision the `ShaderOperatorCase` constructor of `modules/gles2/functional/es2fShaderOperatorTests.cpp` puts on `a_in0`, `a_in1`, `in0`, `in1`, `res` (`mediump`); same fragment text | byte-equal to `ck_lane`'s | the same, with an fp16 vertex shader (varying shader of 23 instrs: `FADD.v2f16`, `F16_TO_F32`) |
| `lane_zero_load` | `...add.highp_float_vertex` (board: ColourZeroHalf) | `deqp_shapes/colourpair_varying_and_zero.{vert,frag}` on main, verbatim | byte-equal to `ck_lane`'s | **no (lane, 0.0) pair** either: the four lanes are varyings (`0.0` stored twice) |
| `default_attrib` | `default_vertex_attrib.float.vertex_attrib_1f` (ColourZeroHalf) | rebuilt: `es2fDefaultVertexAttributeTests.cpp` `genVertexSource` (TYPE_FLOAT, not normalizing, negative values, `s_valueRange` 10) and `s_passThroughFragmentShaderSource` | 8 instrs: `LD_VAR_BUF_IMM.f32` (one lane), `FADD.f32 r0.h0, r0, 0x0.neg`, `ATEST 0x3F800000`, `U16_TO_U32 r0, r0.h0`, `MOV.i32 r1, 0x3C000000`, `BLEND` | the (lane, 0.0) word is `U16_TO_U32`, and the (0.0, 1.0) word is an immediate `MOV`, not a FAU word. The link drops lanes y, z, w (varying shader: 5 instrs, one store) |
| `ck_lane_highpcolour` | not a dEQP case: `ck_lane` with `mediump vec4 color` changed to `highp vec4 color` | `ck_lane` + that one edit (sidecar says so) | 10 instrs: `LD_VAR...v2`, `V2F32_TO_V2F16 r0`, `ATEST 0x3F800000`, `MKVEC.v2i16 r1, 0x0.h0, r0.h0`, `MKVEC.v2i16 r2, r0^.h1, u2.w0.h0` (FAU word 4 `0x3c003c00`), `MOV r0, r1`, `MOV r1, r2`, `BLEND` | the (constant, lane) order of a pair Mesa *does* fold: constant as src0 (inline `0x0.h0`, not FAU), then the second half of the converted pair (`.h1`) with the FAU `1.0`. The varying is compacted to `v2` |

The rebuilt texts follow VK-GL-CTS 5bb4c7e8 (a sparse checkout of main,
2026-11-13), rebuilt by hand from these lines:
- `es2fShaderOperatorTests.cpp:605-613`: input precision; a float input keeps
  the case's precision.
- `:616-618`: `a_position` highp, `a_in<i>` at that precision.
- `:656`, `:692`: `in<i>` and `res` at the case's precision.
- `:703`: the colour is always `mediump`.
- `es2fDefaultVertexAttributeTests.cpp:337-345`: the header, with the blank
  line after `gl_Position`.
- `:352-354`: not normalizing, `/ float(10)`, `s_valueRange` at `:51`.
- `:357-358`: negative values, because `is_signed` at `:222` holds for a
  float loader and `NORMALIZING = 0` at `:93`.
- `:363`: one component.
- `:53-57`: the fragment shader.

The `deqp_shapes` pairs come from VK-GL-CTS 93bca01. A fresh measure `.qpa`
of the two rebuilt cases would confirm them byte for byte.

### What the references change in the lowering plan

- **The operator harness has no colour pair in Mesa.** For
  `ck_lane`, `ck_lane_mediump` and `lane_zero_load` the fragment program is
  the plain 4-lane load: our refusal comes from our link, which removes
  the vertex shader's constant lanes (rule 1 of `mali-varying-constants.md`).
  Mesa's link does not remove them here. A scratch probe (not committed) found
  the cause in the vertex shader's local `mediump vec4 color`:
  - with `highp vec4 color` (`ck_lane_highpcolour`), Mesa drops the
    constant lanes;
  - with only `v_color` made highp, Mesa keeps them.

  The likely mechanism is that the lane reaches the store as a precision
  conversion of a constant (`f2fmp`), which `nir_opt_varyings` does not treat
  as a constant. This is *not verified* in Mesa's source. So for these cases
  (all of the 141 operator `constant_then_varying` cases, and the operator
  `*_float_vertex` / `inversesqrt` cases of the 29) the lowering is a **link
  rule**, not the `vectorise`/`word_of` edit of step 2: a vertex output lane
  that is a constant written through a mediump value stays a varying. The
  fragment shader is then the load/convert form we already draw. This touches
  the varying link, which `lower-varylink` and #367's link rules also edit.
- **The (lane, 0.0) form is real** for `default_vertex_attrib.float` (8
  cases), whose constants are written straight into a highp `v_color`. Step 2's
  `Leaf::Zext` at `nir_fs.rs:6890` matches `U16_TO_U32 r0, r0.h0`. The
  (0.0, 1.0) word is `MOV.i32 r1, 0x3C000000`, an immediate.
- **(constant, lane), where Mesa folds it** (`ck_lane_highpcolour`):
  `MKVEC.v2i16 c, lane.h0` with the 0.0 as an inline `0x0.h0` src0, as
  step 2 planned for `:6918`/`:8374`. No dEQP case the measure classified
  needs it, so it guards the classifier edit if that edit is made, for
  example for `shaders.random.*`.

### References the study listed that were not made

- `ck_pair_second` ("constant then the `.h1` half"): no case draws it. The
  `.h1` half appears in `ck_lane_highpcolour`'s second word, as
  (lane `.h1`, constant).
- `lane_zero_tree` (`random.basic_expression.fragment.68`): the
  `deqp_shapes` pairs hold one example per shape, and the measure's rules put
  the `shaders.random.*` cases under the two operator shapes. dEQP's random
  shader text is generated by its random shader generator and cannot be
  rebuilt from the source by hand. The pair is in the `.qpa` of a
  `tools/mali/deqp-host-measure.sh` run. The 23 `shaders.random.*` cases of
  the two refusals stay unreferenced until a reference is made from that
  `.qpa`.

## Lowering (item lower-varylink, step B, 2026-11-19)

Done at 80403eba, after the varying link rules of
`mali-deqp-varylink.md` ("Lowering"). Nothing was run on a board.
`tests/mali_deqp_colourpair.rs` holds the result: 12 references are
byte-equal in every region it compares, 2 differ in the vertex program
only (fp16, as before), 5 are refused.

### What was lowered

1. **The link rule** (`nir.rs`, the varylink note's origins): a vertex
   lane that is a component of a vector operation is stored, a constant
   lane included, and is flat only when no lane of the operation reads an
   attribute. `rnd_vs_uniform_flat` and `rnd_vs_vecop_fs_uniform` are
   byte-equal from this alone: their fragment shaders have no colour pair
   once the link keeps those lanes. This is the same rule as
   lower-intval-2's `kept` lanes for a mediump `color`, without the
   precision: the "one predicate" the varylink map asked for is `kept`.
2. **A constant then a load's lane, as a word of the colour**
   (`nir_fs.rs` `vectorise`, `Leaf::Mk { first: true }`):
   `MKVEC.v2i16 <constant>.h0, lane.h0`, the constant the low half
   (`ck_lane_highpcolour`'s `MKVEC.v2i16 r1, 0x0.h0, r0.h0`). Byte-equal:
   `ck_lane_highpcolour`, `rnd_fs_ck_dup`, `rnd_fs_ck_lz`,
   `rnd_fs_kword_ck`, `rnd_fs_zword_zk`, `rnd_vs_ck_lz`, `rnd_vs_zk_lz`.
3. **A load's lane and 0.0, as a word of the colour** (`vectorise`):
   `MKVEC.v2i16 x, #0`, which `opt.rs` `lower_mkvec_swz` already makes
   `U16_TO_U32 x`. Byte-equal: `default_attrib`, and the older
   `sel_vc_zero` and `uniarray_vs` fragment programs, whose unit tests
   pinned the refusal and now compare the words.
4. **ATEST's alpha** when the second word is made after it from the
   alpha's own conversion (a lane in both halves, or a constant then the
   lane): the conversion's own half, `ATEST .. r0.h0` (`rnd_fs_ck_dup`),
   where the code read the high half of a word that did not exist yet.

### What stays refused, and why

| reference | refusal | why it was left |
|---|---|---|
| `rnd_fs_ck_madd`, `rnd_fs_ck_product`, `rnd_fs_zk_tree` | `a colour pair of a constant then a varying lane` | the pair is an **operand** of an operation. Mesa computes the lane alone and joins the halves afterwards (`rnd_fs_zk_tree`: `FADD.v2f16 r1, r2^.neg.h00`, then `MKVEC.v2i16 r1, 0x0.h0, r1^.h0`), and folds a lane's `0 * x`. Building the pair as `MKVEC` and running the two-lane operation gives a right program with other words, so the operand form is refused at the same site (`vectorise` checks that the pair is one of the colour's two words) |
| `rnd_fs_uniform_tz_ck` | `a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32)` | a **computed** lane and 0.0 (`lane_word`, the other site), which only `vec_tree` draws |
| `ck_lane_mediump`, `rnd_vs_mediump_kept` | compile; the vertex program differs | mixprec's vertex side (fp16 sums) |

`rnd_vs_vecop_kept` was refused at the partial-varying link (#367's link
rule, not the code generator's text). It is byte-equal since the program's
vertex shader is built for what its fragment shader reads
(`mali-product-route.md`, issue #6509): 13 byte-equal, 4 refused.

A lane and 0.0, or a constant then a lane, as an operand of an operation
keeps its refusal text, so section M's rows for those shapes do not move
to another family.

### For the item that takes the rest

- The operand form needs Mesa's split: the constant lane's operation
  folded (or computed alone), the varying lane's computed at one lane, and
  the `MKVEC` of the two results. `rnd_fs_ck_madd`, `rnd_fs_ck_product`
  and `rnd_fs_zk_tree` are its references.
- `lane_word`'s refusal (`rnd_fs_uniform_tz_ck`) is a classifier edit of
  the same kind as item 3, for a tree instead of a load.

## Lowering (item spike-colourpair-rest, 2026-11-23)

Done after lower-varylink step B, host only (nothing run on a board).
`tests/mali_deqp_colourpair.rs` and `tests/mali_deqp_mixcol.rs` hold the
result: no `deqp_colourpair_*` reference is refused any more.

- **A split pair** (`nir_fs.rs`, `Fs::split`): the lanes the walk makes
  one at a time from a pair `nir_lower_alu_width` splits (a mixed
  constructor's constants across two pairs, the `Fmul` and `Fadd` arm)
  are not paired by `vectorise`; each lane is its own tree and the word is
  packed afterwards, as Mesa does. Byte-equal: `rnd_fs_zk_tree`,
  `rnd_fs_ck_product`.
- **A constant then a uniform's component, and a uniform tree and 0.0**
  (`lane_srcs`, `uniform_tree`): `MKVEC.v2i16 <half>.h0, x` with the
  constant the low half, and `U16_TO_U32` of a sum of uniform components
  and constants. Byte-equal: `rnd_fs_uniform_tz_ck`.
- `rnd_fs_ck_madd` compiles to Mesa's operations and values with other
  words (`FS_DIFFERS`): Mesa makes the product's two constant pairs with
  `IADD_IMM`, we push them to the FAU.
- `deqp_mixcol_ck_scale`, `_ck_sum` and `_lz_scale` are byte-equal; the
  other colourpair rows of `mali_deqp_mixcol.rs` move to their next
  refusal (a colour lane form, or a varying load or conversion form).

`MaliError::ColourPair` and `ColourZeroHalf` stay for the shapes no
reference draws (the unit tests in `tests.rs` pin one shape each).
