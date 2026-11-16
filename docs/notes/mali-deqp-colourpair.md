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
