<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a look-up at a coordinate computed from another's texel (TEX_DEPENDENT)

**Status: gm2_nbshrf_8 and gm2_nbshrf_9 are byte-equal** (plan #367
lower-train-2, from proto-tex-dependent; issues #32/#33). This is the
TEX_DEPENDENT family of docs/notes/mali-glmark2-forecast.md. The Mali online
compiler (`src/shaderc/online/mali/`) refused a mediump fragment look-up
whose coordinate reads an earlier look-up's texel with `MaliError::TexCount`
("a fragment shader texture look-up beside another in a form only a lone
look-up takes"). The refusal came from `pair_coords`, which could not pair a
texel lane, and from `multi_forms`, which only admitted `fadd(<converted
pair>, <constant pair>)` as a coordinate. glmark2's refract fs 3fae chains
three such look-ups (distance map, normal map, image map). The references
are Mesa 26.2.1 on drm-shim (`tools/mali/testdata/gm2_nbshrf_8`, `_9`, plan
#367 study-next-shadow-refract, 931d5325). They are original reductions with
no glmark2 text. The host test is `tests/mali_tex_dependent.rs`. Background:
mali-tex-multi.md (the route this family extends), mali-texture-lookups.md
and mali-texture-refusals.md.

## 1. What Mesa draws

gm2_nbshrf_8 (`float d = texture2D(m0, vp.xy * 0.5 + 0.5).x; vec3 q =
vp.xyz + vn * d; gl_FragColor = texture2D(m1, q.st * 0.5 + 0.5);`):

```
LD_VAR_BUF_IMM.f32.slot0.v4 ... @r0:r1:r2:r3, index:0x0      vp, read whole
V2F32_TO_V2F16 r0, r0^, r1^                                   vp.xy converted
LD_VAR_BUF_IMM.f32.slot1.v3 ... @r2:r3:r4, index:0x10        vn, before the first look-up
V2F32_TO_V2F16 r1, r2^, r3^
FMA.v2f16 r2, r0, 0x38000000.h11, 0x38000000.h11              vp.xy * 0.5 + 0.5, one fused pair op
F16_TO_F32 r3, r2.h0 ; F16_TO_F32 r2, r2^.h1 ; MOV ... ; MOV r2, u2.w0 ; MOV r3, 0x0
TEX_SINGLE.slot2.reserved.16.r.2d.computed.wait2 @r2, @r4:r5, [r2^:r3^]   red only, no .skip
FMA.v2f16 r0, r1^, r2^.h00, r0^                               vn.xy * d + vp.xy, d the word's low half
FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11
F16_TO_F32 ... TEX_SINGLE.slot0.skip.reserved.16.2d.computed.wait0126 @r0:r1, ...
```

- **The texel is its look-up's word.** The coordinate reads the fp16 texel
  through a swizzle of the word (`r2^.h00` broadcasts `d`), so no move or
  `MKVEC` is needed. gm2_nbshrf_9's `b + n * 0.75` reads `n.xy` as the
  aligned pair (`FMA.v2f16 r0, r1^, u3.w1, r0^`).
- **Each coordinate is fp16 pair arithmetic.** `x * 0.5 + 0.5` and `vn * d
  + vp` are each one `FMA.v2f16` after the late fusion. A constant with no
  fp16 immediate is a pushed pair (`1.5` is `u2.w0 = 3e003e00`, `0.75` is
  `u3.w1 = 3a003a00`).
- **The look-up's texel mask is what is read.** `.r` when only `d` is read,
  `.rg` for gm2_nbshrf_9's `n.st`. Only the last look-up carries `.skip`,
  because the helper lanes need each earlier texel for the next
  coordinate's derivatives. The existing look-up emission already does both.
- **The grafted look-up sets the order.** GLSL's `opt_tree_grafting`
  grafts a temporary that is assigned once and read once into its reader.
  An `ir_swizzle` of an `ir_texture` (`float d = texture2D(..).x`) is one
  such rvalue. So gm2_nbshrf_8's whole colour is one tree:
  `texture2D(m1, (vp.xyz + vn * texture2D(m0, ..).x).st * 0.5 + 0.5)`.
  glsl_to_nir visits its children first, left to right: `vp`, then `vn`,
  then the first look-up's coordinate and the look-up. The two loads and
  their conversions therefore stand before the first `FMA.v2f16`.
  gm2_nbshrf_9's `e` and `b` are read twice and stay statements, and its
  `vp.xyz + vn * 1.5` visits `vp` first.
- The varyings are read whole (`.v4` of `vp`, `.v3` of `vn`) although only
  `.xy` is used. Our load already reads them that way for these shapes.

## 2. The lowering (`nir_fs.rs`)

The shader takes TEX_MULTI's route (`multi16`, mali-tex-multi.md section
2). TEX_DEPENDENT adds four things:

1. **`grafted_locals`** grafts a one-lane local of a 2D look-up (plain:
   no shadow, projection or offset) when the local's one read reaches a
   later look-up's coordinate (`feeds_coord`). Before, only 2D-array and 3D
   one-lane texels and whole 2D texels were grafted. Every other 2D
   texel lane keeps its statement as before (unmeasured).
2. **`tree_order`**: when a coordinate reads a texel (`tex_dependent`),
   `pair_coords` first puts the nodes in the order of a depth-first walk
   from the colour's stores, sources in order. That walk is glsl_to_nir's
   order over the grafted tree. TEX_MULTI's rule then moves only those
   conversions that the walk did not place among the inputs.
3. **`coord_pair`** pairs two lanes of one fp16 look-up's word (the same
   lane twice: `H00` or `H11`; an aligned pair: `H01`). It reads them
   through the word's `vec2` (`texel_word`), which is `vectorize16`'s node
   and is made once, right after the look-up.
4. **`multi_forms`** admits the coordinates of a dependent shader
   (`dep_coord`): fp16 sums and products of two lanes over constant
   pairs, converted varying pairs, texel words and each other. These
   include the non-dependent `vp.xy * 0.5 + 0.5` of the same shader.
   Without a dependent coordinate, the TEX_MULTI rules apply unchanged.

Byte-equal: gm2_nbshrf_8 and gm2_nbshrf_9, in all three regions.

## 3. What stays refused

| Form | Why |
|---|---|
| A uniform in a dependent coordinate (`vp.xy + u * d`) | no reference; `dep_coord` takes no uniform |
| Any operation other than a sum or a product on the way (`abs`, `normalize`, `refract`) | no reference. refract fs 3fae's coordinates are built from `refract()` and `normalize()` (FS_REFRACT, CONST_RCP16), so refract needs `dep_coord` to grow with those families |
| A dependent look-up beside a discard, an `if` or a loop | `multi16` takes straight-line code only |
| A highp dependent shader | another route (`select`), not measured here |

The texel words past the first (`t.z`, `t.w` read by a coordinate) pair
through the same `vec2` as the colour's words in `vectorize16`. No
reference draws such a coordinate yet (refract's `back_normal.z` will).

## 4. Measured (lower-train-2, on main 9a74631a and the steps before it)

- `tests/mali_tex_dependent.rs`: both references byte-equal, and the
  refusal pins above.
- All 1,810 `tools/mali/testdata` pairs compiled through `compile_pair`
  before and after the change (`tests/mali_corpus_diff.rs`): only
  gm2_nbshrf_8 and _9 move (refused before, byte-equal after).
- `tools/mali/corpus-verdicts.sh --check`: no regression, no move.
- `tests/mali_glmark2_corpus.rs` (glmark2 22c527cb): 23 of 33 scenes
  compile-clean, unchanged. refract fs 3fae still stops first at the
  constant-product refusal (lower-train-3's family).
