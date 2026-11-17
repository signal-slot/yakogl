<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a fragment texture result read in a swizzle or a lane alone (TEX_SWIZZLE)

**Status: gm2_texswz_1..6 byte-equal; terrain fs d697 compiles and links**
(plan #367 lower-tex-swizzle, issues #32/#33). The TEX_SWIZZLE family of
docs/notes/mali-glmark2-forecast.md: the Mali online compiler
(`src/shaderc/online/mali/`) took an fp16 look-up's result only in its own
pairs, in order (`MaliError::TexLanes`, "a texture result's lanes other than
its own pairs in order"). The references are Mesa 26.2.1 on drm-shim
(`tools/mali/testdata/gm2_texswz_*`, drawn without `--uniform-values`):
cases 1 to 5 from ref-tex-swizzle, case 6 an original reduction of terrain
fs d697's shape added here (`tools/mali/drm_shim_ref.sh run`, then
`pan_ref_diff.py extract --draw`; the same build reproduces gm2_texswz_5's
expect.json byte for byte, `diff-ref`). The host test is
`tests/mali_tex_swizzle.rs`; the pan_shc verdicts are in
`tools/mali/testdata/cases.txt`. Background: mali-texture-lookups.md (the
look-up and its write mask), mali-texture-refusals.md (the family list),
mali-vs-texture-lanes.md (the vertex stage's texel lanes).

## 1. What each reference needed

The look-up writes the texels the colour reads, packed from the first
(`tex_mask`, `nir_def_components_read`): `.r` alone is `TEX_SINGLE ... .r
@r0`, `.a` alone `.a @r0` with the alpha in `r0.h0`. Its words are the
leaves `Leaf::Tex(j, h)` (word `h`, texels `2h`, `2h + 1`) and
`Leaf::TexLane(j, c)` (texel `c`'s half in both lanes) of the mediump pair
graph, and `Word::Tex` of a highp shader's colour.

| Case | Shape | What the compiler had not | Mesa's words | Where |
|---|---|---|---|---|
| gm2_texswz_1 | `vec4(t.r)`, mediump | one texel in both lanes of a word: the a == b root of a computed lane (gm2_fsmisc_4's) now takes an fp16 texel too, `Leaf::Pack { x, x }` of its `TexLane` | `ATEST .. r0.h0`, `IADD.v2u16 r0, r0^.h00, 0x0`, `MOV.i32 r1, r0` | `select` roots; ATEST of a `Pack` reads the texel's own half |
| gm2_texswz_2 | `vec4(t.a * 0.5)`, highp | a lowp texel's product in a highp shader is fp16 (lower_precision's `visit_enter(ir_texture)`); `alu16` had products of pairs only. A one-lane product of a texel by a constant (`Fs::alu16`), and its store to the mediump colour no longer marks the shader mixed (`Fs::lone_texel_product`) | `FMA.v2f16 r0, r0^.h00, 0x38000000.h11, 0x0.neg`, then case 1's words | `alu16` `Fmul if w == 1`, the colour store |
| gm2_texswz_3 | `vec4(t.yx, 0.0, 1.0)`, mediump | a word of a look-up with its halves swapped: `Leaf::TexSwapped(j, h)`, an output word only, made `MKVEC.v2i16 w.h1, w.h0` (a `MOV.i32` of `w.h10` is copy-propagated and loses the swizzle) | `IADD.v2u16 r0, r0^.h10, 0x0` (opt.rs `lower_mkvec_swz`), `MOV.i32 r1, 0x3C000000` | `vectorise` texel arm, `select` roots |
| gm2_texswz_4 | `t.wzyx`, highp | the same in the highp colour: `Word::TexSwapped`; ATEST on the high lane of the second word, the look-up word's low half | `ATEST .. r0.h0`, `IADD.v2u16 r1, r1^.h10`, `IADD.v2u16 r2, r0^.h10`, two `MOV.i32` | `word_of`, the highp store |
| gm2_texswz_5 | `mix(a, b, 1.0 - c)` of three look-ups, mediump (terrain's blend shape) | productions `(Tex, NegSrc)` and `(Tex, FmaC)` | `FADD.v2f16 r0, 0x3C000000.h11, r0.neg`, `FADD.v2f16 r4, r4^, r2.neg`, `FMA.v2f16 r0, r0^, r4^, r2^` | `ADMITTED` (tests.rs population) |
| gm2_texswz_6 | `g = dot(c.rgb, k)`, `vec4(g, g, g, c.a)`, mediump (terrain fs d697) | a word of a computed lane then a texel alone: `Leaf::Pack { x: g, y: TexLane(j, 3) }`; ATEST on the texel's half | `FMA.v2f16` chain of `r1.h00`, `r0.h11`, `r0.h00`; `ATEST .. r1.h1`; `IADD.v2u16 r2, r0.h00`; `MKVEC.v2i16 r1, r0^.h0, r1^.h1` | `vectorise` arm after `mixed_word` |

## 2. Rules kept narrow

Each new form stands where a reference draws it; elsewhere the old refusal
stays (`tests/mali_tex_swizzle.rs`
`the_shapes_no_reference_draws_stay_refused`, tests.rs's gap row):

- a swapped word read by an operation (`t.yx * 2.0`): Mesa would put the
  swizzle on the operation's source; no reference, `TexLanes` (checked in
  `select` after the pair graph is built);
- a pair of texels neither in order nor swapped (`t.xz`, `t.xzyw`):
  `TexLanes`;
- a texel before a computed lane in a word (`vec4(t.r, t.r * 0.5, ..)`):
  only the computed lane first is drawn (case 6), `TexLanes`;
- fp16 arithmetic on a highp shader's texels other than one texel by a
  constant (`t.a + 0.5`, `t * 0.5` stored to the mediump colour): still
  `MixedPrecision`.

## 3. glmark2

With the change, `tests/mali_glmark2_corpus.rs` (corpus of
`tools/hostharness/glmark2/run.sh`, glmark2 22c527cb) moves terrain fs d697
from `TexLanes` to `ok` (29 of 69 shaders compiled, 11 of 12 pairs linked)
and nothing else; the clean-scene count stays 5 of 33 ([0, 1, 5, 14, 16]):
terrain's other shaders are refused by other families (vs 18bc FAU_PAGE;
fs 8c86, bbba, bdfd, e043 CONST_FOLD; fs c549 ARITH; fs 4d26 a uniform read
at a run-time index; fs 1637 a -0.0 a comparison reads). `tools/mali/corpus-verdicts.txt` does not move (no
`tools/shaders` source has these shapes).
