<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: a shadow look-up that compares per texel (PCF) — 2D, 2D arrays, mipmap filters, mirrored repeat and cubes

Design as built, plan #325 items `pcf-mip` (`919db72b`), `pcf-cube`
(`43af85d7`) and this note (`docs-rt-rows`), over the first arm
(`d7b2cf8b`). Written 2026-11-18 on the integration tip `56bed483`. It
describes only what is on the tree: the shadow arm of
`src/shaderc/online/powervr/select.rs` and the sampler words of
`src/pvr/backend.rs`. It gives section 7.2 of `docs/es3/pvr-2darray.md`
as built; that note keeps the Mesa-pco reading, the array layout and the
descriptor map this one presumes, and its section 7.1 is why the compare
is shader work. Nothing here was checked on a board.

## 1. The rule, and who does what

ES 3.0.6 3.8.15.1: with `TEXTURE_COMPARE_MODE = COMPARE_REF_TO_TEXTURE`
"the comparison is done for each texel, and the filter then weights the
comparison results". A filter that samples one texel — NEAREST both
ways without a mipmap filter, or `NEAREST_MIPMAP_NEAREST` — gets the
same answer from comparing that texel's depth after sampling, so the
plain filtered compare is exact there. Every filter that blends texels
or levels must compare before it weights.

The BXS-4-64 has no attested hardware depth compare that the operands
pco drives can reach (`pvr-2darray.md` section 7.1: `cmp_mode` exists
in the sampler state but Mesa 26.2.1 never writes it, and no `smp`
operand carries a reference). So the rule splits:

- **the TPU** samples. For the fallback arm, the plain look-up as any
  non-shadow sampler samples it — filter, level choice, wrap and, for a
  native array, the layer all the hardware's. For the PCF arm, a
  *gather*: the raw texels of the bilinear footprint at a level the
  program named, each tap index wrapped by the sampler's own address
  mode (section 4).
- **the program** compares the reference against each sampled depth and
  weights the 1.0/0.0 results exactly as the filter would have weighted
  the depths, then chooses between the two arms when it runs, from the
  sampler's descriptor (section 3). One program therefore serves every
  filter, every wrap mode and every compare function: a sampler object
  can change all of them without a recompile.

## 2. Program shape

`select.rs` takes an `ir::Op::Tex` with `shadow: true` on `D2`, `Cube`
and `D2Array`. The depth is first sampled by the plain look-up of the
same sampler — same coordinates, level, offset and gradients — into a
scratch block (`Sel::tex_dst`), then the footprints are gathered:

- **2D**: `pcf_taps` — two gathers, one per level the filter reads (the
  same level twice when the filter reads one).
- **2D array, native** (`pvr_native_array`): the same two gathers, each
  carrying the layer address — `array_layer_addr`, `tao` with the
  layer's 64-bit address as two data words after the level.
- **2D array, stacked**: the same two gathers at `array_uv`'s clamped
  coordinate, which keeps the footprint inside the layer; the level's
  size and the gradients scale the image height by 1 / layers.
- **cube, native or stacked**: `pcf_cube_taps` — eight gathers, four
  taps at each of the two levels, one texel per tap (section 4).

`shadow_compare` then compares against the reference — lane 2 for a 2D
sampler (on `textureProj` the reference is divided by `q` as the
coordinates are), lane 3 for a cube or a 2D array — and the arm's
result is chosen by sampler meta dword 13 bit 0. A look-up is one
sample plus two gathers (2D, array) or plus eight (cube), in every
configuration; when bit 0 says "no per-texel work", the gathers run and
their results are not used. A cube look-up's `dmn` is 3d on the native
cube (with the `SampleZ` direction word) and 2d on the stacked image.

## 3. Sampler meta words 12 and 13

A shadow program reads its own descriptor at run time
(`SelOp::TexWord` groups in `shadow_compare` and `pcf_levels`).

**Dword 12** — `PCO_SAMPLER_META_COMPARE_OP`, the field Mesa packs the
same way; `tex_desc_sampler_lod` writes the GL core's
`GlesTextureBind::compare_op` there and nothing else writes it (unit
test `the_compare_op_is_sampler_meta_word_0`):

| bits | meaning |
|---|---|
| 0 | depth **less than** the reference |
| 1 | depth **equal** to the reference |
| 2 | depth **greater than** the reference |
| 3 | the reference is clamped to [0, 1] (a fixed-point depth; Mesa reads that bit from the image meta `PCK_INFO`, here the GL core says it, since depth is stored as floats whatever its internal format) |
| 4 | `TEXTURE_COMPARE_MODE` is NONE: the result is the sampled depth |

The bit-4 select is the last operation of `shadow_compare`.

**Dword 13** — `PCO_SAMPLER_META_RSVD0` (`pco_data.h`), the word Mesa
leaves reserved and no pco program reads. `shadow_filter_word(t,
levels)` in `src/pvr/backend.rs` packs the sampler's filter into it,
and `pcf_levels` / `shadow_compare` unpack it:

| bits | meaning |
|---|---|
| 0 | compare per texel — some filter blends texels or levels. 0 for NEAREST both ways and `NEAREST_MIPMAP_NEAREST`, where the one texel sampled is the whole filter |
| 1 | the magnification filter is NEAREST |
| 2 | the minification filter's filter *within the level* is NEAREST |
| 3 | the level is the nearest one (`*_MIPMAP_NEAREST`) |
| 4 | two levels are blended (`*_MIPMAP_LINEAR`) |
| 8..11 | the last level, `levels - 1` (4 bits, capped at 15) |

Bit 0 alone is LINEAR/LINEAR over one level; NEAREST both ways over a
chain that is not walked is 0 too. The word is packed for compare-less
samplers as well — the bit-4 select of word 12 wins then. The unit test
`the_shadow_filter_is_sampler_meta_word_1` pins: LINEAR/LINEAR over one
level `1`; LINEAR_MIPMAP_NEAREST over six levels `1|8|5<<8`; trilinear
`1|16|5<<8`; NEAREST mag `1|2`; NEAREST min `1|4`; NEAREST both ways
and `NEAREST_MIPMAP_NEAREST` `0`; NEAREST_MIPMAP_LINEAR
`1|2|4|16|5<<8`; and that the wrap modes are **not** in the word —
the TPU's gather wraps. The descriptor's LOD clamp (words 8, 9) and the
gather's replaced level are two different things: the gathers walk the
chain inside the levels range the descriptor states (the sampler's
`maxlod`), and the program's own level pick is clamped to bits 8..11.

## 4. What the program computes, what the TPU does

The level of detail and the levels the filter reads
(`pcf_lambda`, `pcf_levels`; the table is `pcf_taps`' comment):

```text
lambda   = lod                         textureLod
         = log2(rho) (+ bias)         otherwise; 0 in a vertex shader
rho^2    = max(|duv/dx * size|^2, |duv/dy * size|^2), level 0's size,
           textureGrad's gradients or the quad's (dFdx, dFdy)
minified = lambda > 0                  (ES 3.0.6 3.8.11, c = 0)
nearest  = minified ? bit 2 : bit 1   the filter within the level
bit 3 (MIPMAP_NEAREST): d1 = d2 = clamp(ceil(lambda + 0.5) - 1, 0, top)
bit 4 (MIPMAP_LINEAR):  l = clamp(lambda, 0, top), d1 = floor(l),
                        d2 = min(d1 + 1, top), t = l - d1
magnified, or neither:  d1 = d2 = 0, t = 0
```

`top` is bits 8..11. dEQP's reference picks the same levels
(`tcuTexture.cpp sampleLevelArray2DCompare`, `texref.rs` `levels`). A
NaN lambda — a NaN derivative — counts as not minified, so no gather is
handed a NaN level (one once stalled the TPU).

**The gather is a `smp`.** `SelOp::Sample { gather: true }` encodes it
as pco encodes `tg4` (`fragment.rs` `build_smp`, `pco_trans_nir.c`
`lower_smp`): the extended form with `sbmode` rawdata and one channel,
`lodm` replace, the footprint's four texels arriving in four
consecutive registers as (i0, j0), (i1, j0), (i0, j1), (i1, j1). A
constant offset rides in the lookup word. The gather reads the
texture's own sampler words (8..11): the descriptor's gather words 16..19
stay unused, as Mesa's gather sampler copies the main sampler's address
modes anyway (`pvr_arch_sampler.c:220-232`). So a tap index outside the
image is wrapped by the TPU, and MIRRORED_REPEAT needs no weights of
its own: the taps are i0 and i0 + 1 each wrapped, and the program
weights with the *unwrapped* coordinate — which is what dEQP's
reference weights (`tcuTexture.cpp sampleLinear2DCompare`,
`pvr-2darray.md` section 7.2's corrected comment).

**Within a level** the four compares are mixed as the TPU would mix
depths: `mix(mix(c00, c10, fx), mix(c01, c11, fx), fy)` with
`f = fract(coord * size - 0.5)` at that level's size (`tex_size_2d`
per level: the layer's for a native array, the stacked image's for a
stacked one, the face's width both ways for a cube), or, when bit 1 / 2
says NEAREST within the level, with the weights rounded to 0 or 1 — the
one tap the coordinate falls in. **Between the levels**, the two
results are mixed by `t`.

**The cube footprint is seamless**, as ES 3.0 filters a cube and as
dEQP's reference compares it (`texref.rs` `sample_cube_compare`):
`cube_face` takes the face and (s, t) of the direction (the major-axis
rule with its ties and signs, `cube_uv`'s, without the stacked clamp)
and carries the direction's gradients to (s, t) by the quotient rule;
the level of detail is log2 of the longer one in texels of level 0's
face. At each level, of face width n, i0 = floor(s·n − 0.5), j0
likewise; each tap's centre (2i+1)/n − 1 is mapped onto the face's
plane (`face_direction`), selected and projected as any direction is:
the tap inside the face reads itself, a tap past one edge the
neighbouring face's edge texel along the same row or column
(`seamless_texel`, dEQP's `getCubeEdgeFaceCoords`), and a tap past a
corner (outside in both axes) is flagged — `shadow_compare` then makes
its compare the mean of the other three results, `(sum − c) / 3`
(dEQP's `getCubeLinearSamples`; at most one tap per footprint). Each
texel is read by a gather a quarter texel past that texel's centre,
((i + 0.75)/n, (j + 0.75)/n) on its face, whose first tap is the texel:
on the stacked image at (u, (face + v) / 6), on the native cube at that
point's direction.

## 5. Host tests

HOST-ONLY throughout — programs, state words and the byte model; no
picture verdict exists on a host (its read-back is black). Feature sets
per each test's header:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_pcf
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_pcf_cube
```

- `tests/es3_pvr_pcf.rs` (`919db72b`): the program's shape (the plain
  sample and two gathers at a replaced level, `tao` only on a native
  array, quad derivatives only for an implicit level or a bias in a
  fragment shader); the arm's arithmetic on the CPU, operation for
  operation (`pcf_model`), against the shared texture reference
  (`src/shaderc/online/texref.rs` `sample_layer_compare`, dEQP's
  percentage-closer filter) over the six minification filters, both
  magnification filters and the three wrap modes, on one and three
  layers; and the compiled programs in the byte model (`tests/usc_exec`,
  which models a `sbmode` rawdata gather, and a cube gather through
  `cube_uv`) against that arithmetic for every filter word under
  `textureLod`, `textureGrad`, a bias and the implicit level, on the 2D
  sampler, the native array and the stacked one, checking the gathers'
  levels and the result.
- `tests/es3_pvr_pcf_cube.rs` (`43af85d7`): the same on the cube,
  against `sample_cube_compare`, over the same filters, levels of
  detail across a four-level chain, four compare functions and
  directions on all six faces with footprints inside, across an edge
  and at a corner (over 50,000 look-ups texref predicts); the
  edge-texel rule pinned by hand; the shape (the plain sample and eight
  one-channel gathers, `dmn` 3d native, 2d stacked); and the compiled
  programs in the byte model for every filter word under `textureGrad`,
  a bias and the implicit level, on both layouts.
- `tests/es3_pvr_shadow.rs` (`d7b2cf8b`): the first arm — LINEAR over
  one level comparing per texel; since `919db72b` it expects three
  samples per look-up and gathers on the stacked array, and since
  `43af85d7` nine for a cube shadow look-up.
- `src/pvr/backend.rs` unit tests: `the_compare_op_is_sampler_meta_word_0`
  and `the_shadow_filter_is_sampler_meta_word_1`, section 3.

HOST-ONLY dEQP records from the commits (`tools/cts/host-deqp-run.sh`,
`MODULE=gles3 GPU=powervr`, VK-GL-CTS `93bca01`): `texture.shadow.cube.`
144 of 144 cases began with 0 compile or link refusals before and after
`43af85d7` — every case in the group is a render verdict, so the change
the PCF arms make there is the AM62P's to measure.

## 6. Still open

- The board verdicts: the AM62P reruns of `texture.shadow.{2d,2d_array,cube}`
  (plan #325 item `board-rt-pcf`, then plan #313's tree runs), and
  whether the TPU's cube gather returns the texel of the direction's
  own face as its first tap (`43af85d7` leaves this open).
- Whether the AM62P run's `NEAREST_MIPMAP_NEAREST` QualityWarnings had
  another cause than the missing per-texel compare — level selection,
  `pvr-2darray.md` section 7.2; the row says it was "a reading from the
  pattern, not measured".
- The `cmp_mode` probe of `pvr-2darray.md` section 7.1: a one-run board
  probe at most; nothing in this design uses it.
