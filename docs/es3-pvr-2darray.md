<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: 2D-array and shadow sampling, as Mesa's pvr / pco does it

Design for plan #3 item `pvr-2darray` (and the array half of `pvr-shadow`).
Written 2026-10-08 on the integration tip `14d6c177`. Docs only: nothing in
`src/` changed, nothing was run on a board.

## 0. Sources, and what was and was not checked

Mesa 26.2.1 (tag `mesa-26.2.1`, tarball sha256 `c47e81bd...a4442`, the pin in
`third_party/mesa-26.2.1/UPSTREAM`). `third_party/mesa-26.2.1` carries only the
csbgen XML and the device-info headers, not `pvr_image.c` or `pco/`. The
`mesa-26.2.1-src` directory in the deps cache is empty, and `mesa-26.2.1-caps`
has only `src/imagination/vulkan`. For this document `src/imagination/` was
extracted from the same tarball (checked against the sha256 above) into
`<mesa-26.2.1-pvr>`. Every `Mesa path:line` below was read there;
paths are relative to `src/imagination/`. `csbgen/rogue/texstate.xml` there is
byte-identical to `third_party/mesa-26.2.1/src/imagination/csbgen/rogue/texstate.xml`
(checked with `cmp`), so those line numbers hold in both.

Parked work: `refs/ccfarm/origin/` has no branch for 2D arrays or shadows.
`am62p/array2d` (step 1, `80daed91`) and `am62p/array2d-s3-parked`
(step 2, `0fb9a36f`) are named in `tools/cts/RESULTS.md:995` but are not in this
clone (neither object exists). What is known of them is that one row. Its
design (address override, layer-major storage) is the same as below. Its
measured step 1 (stacked storage, `tao`) read level 0 only, which is why 12
mipmapped arrays went Pass -> Fail; section 6 says how this design avoids that.

## 1. Where it fails today

- `Sel` samples a 2D array as one tall 2D image (`array_uv`,
  `src/shaderc/online/powervr/select.rs:2606`; the arm is `select.rs:1061-1081`,
  `texelFetch` is `select.rs:908-947`, `textureSize` is `select.rs:1188-1215`).
  The layer becomes `v = (clamp(c.y, 0.5/h, 1 - 0.5/h) + l) / layers`. The only
  protection against crossing layers is that `v` clamp. LINEAR taps at a layer
  edge are clamped by the shader, not wrapped by the layer, and a mip level
  of a stacked image is a stack of smaller layers whose heights the TPU rounds
  (`gl/texture.rs` `chain_layout`, `pot_stack`). That is the failure of
  texture.filtering.2d_array (210) and the 18 of 24 shadow 2d_array cases that fail
  even with NEAREST.
- The descriptor is the plain 2D one: `build_tex_desc`
  (`src/pvr/backend.rs:3603`) writes `TEXTYPE_STRIDE`, `tex_stride_image_word1`
  (`backend.rs:932`) and, for the array, `width = w`, `height = h * layers`
  (`gl/draw.rs:3375-3393`, `tb.height = rows`). The array's `type_` is
  `GLES_TEXTYPE_2D_ARRAY` (`draw.rs:3400`) but the PowerVR back end never reads it
  (`backend.rs:3677` tests only the cube probe type).
- The layer count reaches the shader as a hidden float uniform
  `pvr_layers_<sampler>` (`select.rs:2575`, filled by
  `gl/draw.rs` `fill_stage_layer_uniforms:2418`).
- Offsets, bias on a 3D texture, gradients and every shadow form beyond plain
  `texture` / `textureLod` on a stacked texture hit the catch-all
  `select.rs:1164-1166`.

## 2. The model: Mesa has no hardware array path either

Mesa's `pco_nir_lower_tex` sets `hw_array_support = false`
(`pco/pco_nir_tex.c:468`); the hardware-array branch is
`UNREACHABLE("Hardware support for array indexing not implemented")`
(`pco_nir_tex.c:657-659`). The `array` bit of `smp` exists in the ISA
(`pco/pco_isa.py:1960`, only in `smp_extabc` `:2598-2633`; ours
`isa_tables.rs:437`) and Mesa never sets it. A texture array in Mesa is:

1. an ordinary 2D image state (textype 2D) whose `IMAGE_WORD1.depth` holds
   `layers - 1`;
2. a software layer address: `smp` with `tao` (texture address override) and a
   64-bit address in two data words.

The sequence (`pco_nir_tex.c:656-692`):

```
array_max = depth field of word1                  (usclib tex.cl:52-61)
index     = iclamp(index, 0, array_max)           (:664-665)
offset    = index * meta[LAYER_SIZE]              (:675-678)
addr      = texaddr(word1) + offset  (64 bit)     (:680-688; tex.cl:39-50)
smp ... tao, data block = coords, lod, addr_lo, addr_hi, lookup
```

Each layer is therefore a self-contained 2D image with its own mip chain, at a
fixed stride. The TPU filters, wraps and picks levels inside the layer; layers
never mix. That is the property the stacked image lacks.

## 3. Texture-state words

All of this is `TEXSTATE_*` in `csbgen/rogue/texstate.xml` and the packer
`pvr_arch_pack_tex_state` (`vulkan/pvr_arch_tex_state.c:114-324`).

### 3.1 Image word 0 (`texstate.xml:272-290`)

| Field | Bits | Today (stacked 2D) | Array |
|---|---|---|---|
| `textype` | 0..2 | `STRIDE` (4) (`backend.rs:859-861,3743`) | `2D` (1), twiddled: `pvr_arch_tex_state.c:157-161` (the 2D and 2D_ARRAY cases are the same line) |
| `width` | 34..47 | `w - 1` of the stack | `w - 1` of one layer (`:230`) |
| `height` | 48..61 | `h * layers - 1` | `h - 1` of one layer (`:231-233`) |
| `texformat`, `swiz*`, `gamma`, `smpcnt` | as today | as today | as today (`:190-205`) |

A linear layout is `TEXTYPE_STRIDE` for 2D and 2D_ARRAY alike (`:140-146`). So
the stride type is also available for an array, but its word 1
(`STRIDE_IMAGE_WORD1`, `texstate.xml:315-333`) has no `depth` field (bits 0..14
are the stride), so the layer count cannot live in the state. Section 6 chooses
the twiddled form for that reason.

### 3.2 Image word 1 (`texstate.xml:292-314`, set at `pvr_arch_tex_state.c:271-316`)

`IMAGE_WORD1` (twiddled, not the stride word): `num_mip_levels` [0:3],
`depth` [4:14], `mipmaps_present` [15], `texaddr` [16:53] (shift 2),
`alpha_msb` [59], `baselevel` [60:63]. For an array,
`depth = array_size - 1` (`:284-290`; a cube array divides by 6; a 3D texture
uses the extent depth, `:277-282`). `depth` max is 2047 (`texstate.xml:310`), so
2048 layers; ES 3.0 requires 256. Yakogl already has the packer
`tex_twiddled_image_word1(etc, levels, depth, mipmaps_present, base)`
(`backend.rs:952`), used for the native cube with `depth = 5` and pinned by
`the_cube_probe_descriptor_is_a_twiddled_textype_cube` (`backend.rs:6980`).

### 3.3 Image meta and the layout (software-defined, 4 dwords)

`pvr_image_descriptor` is the 4-dword state followed by `meta[4]`
(`vulkan/pvr_common.h:209-215`); `pco/pco_data.h:142-147` orders it:
`LAYER_SIZE`, `BUFFER_ELEMS`, `Z_SLICE`, `PCK_INFO`. `pvr_arch_pack_tex_state`
stores `meta[LAYER_SIZE] = info->layer_size` (`:318`). Yakogl's descriptor is the
same 20 dwords (`TEX_DESC_DWORDS`, `backend.rs:1001`): image 0..3, image meta
4..7, sampler 8..11, sampler meta 12..15, gather words 16..19 (Mesa:
`pvr_common.h:198-226`, gather words at `:201`). `w[4]` is already the layer size
(`backend.rs:3627`), `w[7]` the pack info.

The layer size is `pvr_image_plane_setup_mip_levels` (`vulkan/pvr_image.c:141-199`):

- every level `pitch * height_pitch` (`:159-166`), levels consecutive (`:164`);
- with more than one level the size includes the rest of the chain down to
  1x1 (`:173-188`), "the hw calculates layer strides as if a full mip chain ...
  were present";
- for more than one layer, `layer_size` is aligned to the image alignment
  (`:195-196`, 4 KiB for twiddled);
- extents are the next power of two, each axis alone, when twiddled or
  mipmapped (`:83-92`).

The layout shape is the one `gl/native_cube.rs` implements for six layers
(`face_bytes`, `native_cube.rs:137`; `FACE_ALIGN`, `:51`). It generalises to
`w x h` with `twiddle_index(x, y, pw, ph)` (`gl/texture.rs:2625`, which already
handles non-square).

### 3.4 Sampler words

Unchanged for the array: `SAMPLER_WORD0` (`texstate.xml:342-362`)
address modes `u`/`v` apply inside the layer; `addrmode_w` stays
`CLAMP_TO_EDGE` (`backend.rs:3796`). The sampler's `w` axis is not used.

### 3.5 The word the shader reads for size

`textureSize` of an array reads word 1's `depth + 1` as the layer count, unshifted
by `lod` (`pco/usclib/tex.cl:63-94`, lines 76-91). Yakogl's `tex_size_2d`
(`select.rs:2440`) already reads words 0, 1 and 3; the layer count is
`(dword 2 >> 4) & 0x7ff` + 1. `baselevel` is word 1 bits 60..63 = dword 3 >> 28,
which `tex_size_2d` reads for a non-STRIDE image already. The hidden
`pvr_layers_<sampler>` uniform is then not needed by a native-array program (the
front end still declares it, and the table's stacked rows still read it).

## 4. The `smp` instruction and its operands

`smp` is `pco_isa.py:1931-1960` (fields), forms `smp_brief` (`:2500`), `smp_exta`
(`:2517-2540`), `smp_extab`, `smp_extabc` (`:2598-2633`). Ours:
`isa_tables.rs:433-437`. Yakogl builds `brief` or `exta` (`fragment.rs:267-308`);
`tao` is bit 74 of `exta` and is already a field of the form
(`isa_tables.rs:434`). The data block is built exactly as
`pco_emit_nir_smp` (`pco_nir_tex.c:230-416`):

```
words 0..n-1   coordinates (layer excluded)                 :242-244
               [projector]                                  :246-250
               [hardware array index, never used]           :252-256
               lod: bias | replace | grad pairs             :258-283
               if tao: a zero bias when no lod word, then   :285-300
                       addr_lo, addr_hi
               lookup word (soo offsets, sno)               :302-337
               [write data]
```

Flags (`:234-240`, `lower_smp` `pco_trans_nir.c:1767-1851`): `dmn` from
`to_pco_dim` (`pco_nir_tex.c:106-130`: 2D array is `2d`), `fcnorm` for a float
result, `nncoords` for fetch (`:727-730`), `lodm`, `pplod`, `proj`, `soo`,
`tao` (`:299`, set at `pco_trans_nir.c:1836`), `array` = 0.

If `tao` is set and the shader has no lod word, Mesa inserts a per-pixel lod
bias of 0 (`pco_nir_tex.c:288-294`): `lodm = bias`, `pplod = 1`. So an implicit
`texture()` on an array is `bias` mode with a zero word, not `normal`. The
derivative still comes from the coordinates; the layer is not a coordinate, so it
does not enter the level.

### 4.1 Per function

Yakogl's order is the same as Mesa's, so every form is one new word pair after
the level word. `N` is the number of coordinates (2 for an array: u, v).

| GLSL form | `lodm` | `pplod` | `nncoords` | `soo` | `tao` | data block after the coordinates |
|---|---|---|---|---|---|---|
| `texture(s, p)` (fragment) | `bias` | 1 | 0 | 0 | 1 | `0.0f, addr_lo, addr_hi` |
| `texture(s, p, bias)` | `bias` | 1 | 0 | 0 | 1 | `bias, lo, hi` |
| `textureLod(s, p, l)` | `replace` | 1 | 0 | 0 | 1 | `l, lo, hi` |
| `textureOffset(s, p, o[, bias])` | as above | 1 | 0 | 1 | 1 | `lvl, lo, hi, lookup` (`u` bits 0..5, `v` bits 6..11: `rogue_hw_defs.h:193-203`, `pco_nir_tex.c:306-322`) |
| `textureLodOffset` | `replace` | 1 | 0 | 1 | 1 | `l, lo, hi, lookup` |
| `texelFetch(s, p, l)` | `replace` | 1 | 1 | 0 | 1 | `l, lo, hi`; coordinate `p.xy + 0.5` (as `select.rs:875-905` does for 2D; Mesa passes `p`, the corner, `pco_nir_tex.c:727-730`) |
| `textureGrad(s, p, dx, dy)` | `gradients` | 0 | 0 | 0 | 1 | `dudx, dudy, dvdx, dvdy, lo, hi` (`:274-283`) |
| `textureSize(s, l)` | no `smp` | | | | | `(w >> l, h >> l, depth + 1)`, section 3.5 |
| `texture(sampler2DArrayShadow, p4)` | as `texture` | | | | | the depth like the first row, then `shadow_compare` against `p.w`; section 7 |

Vertex shaders have no derivative: only `textureLod` / `texelFetch` / sized-grad
forms, all with an explicit level; the form is the same `smp` (check that the
vertex `Stage` builds through the same `SelOp::Sample` path before relying on
it: the vertex `sampler2darrayshadow_vertex` case compiles today).

A level word is always present, so the `proj` + level conflict of
`fragment.rs:300` does not arise (arrays have no projective form in ES 3.0).

Offsets on an array stay inside the layer because the TPU's wrap works on the
layer image: this is what the stacked form refused (`select.rs:1164-1166`).

### 4.2 The address words

`addr = texaddr + clamp(layer) * layer_size`. Mesa decodes `texaddr` from word 1
each time (`usclib_tex_state_address`, `tex.cl:39-50`) and adds with a 64-bit
carry (`nir_uadd64_32`, `pco_nir_tex.c:684-685`). In Yakogl the heap base is
`0x80_0000_0000` (`backend.rs:230`), so the high word is non-zero: the carry
cannot be skipped. Two ways, one of them recommended:

- Mesa's: decode `texaddr` from dwords 2 and 3
  (`units = (d2 >> 16) | ((d3 & 0x3fffff) << 16)`; byte address = `units << 2`,
  so `lo = (d2 >> 14 & 0x3fffc) | (d3 << 18)`, `hi = (d3 & 0x3fffff) >> 14`)
  then `lo' = lo + off; hi' = hi + (lo' < lo)`.
- **Recommended**: the driver publishes the base in the two meta dwords the
  array does not use (`w[5]`, `w[6]`: `BUFFER_ELEMS` and `Z_SLICE`, used only by
  buffer textures and 2D-views-of-3D, `pco_nir_tex.c:75,546-560`;
  Yakogl never fills them). The shader does
  `lo' = lo + l * w[4]; hi' = hi + (lo' < lo)`, three ops fewer than the decode
  and no bitfield mistakes to pin. The decode is the fallback if the meta words
  turn out to be read by a table program. The base is already 4 KiB aligned
  (`gl/native_cube.rs:51`), so the address low bits are clear.

The carry uses ops the selector has: `Iadd`, a compare (`MachineOp::Uge` is used
by `tex_size_2d`, `select.rs:2455`) and `Bcsel`.

## 5. Layer rounding

ES 3.0.6 (2D array lookup, texture coordinate `r`):
`layer = clamp(floor(r + 0.5), 0, d - 1)` (section number in the spec not
re-checked here). The existing code already has this
(`select.rs:2606-2640`: `Fadd 0.5`, floor as a source modifier, `Fmax 0`,
`Fmin layers - 1`). Keep it, with these two differences from Mesa and from today:

1. Mesa rounds with `nir_f2i32_rtne` (round to nearest even, `pco_nir_tex.c:212-213`),
   which differs from `floor(r + 0.5)` exactly on halves (0.5 -> 0 vs 1,
   2.5 -> 2 vs 3). ES says `floor(r + 0.5)`. Do not copy Mesa here.
2. The upper bound comes from the state's `depth` field
   (`depth_f = I2f((dword 2 >> 4) & 0x7ff)`), not the uniform `layers - 1`
   (Mesa: `usclib_tex_state_array_max`, `tex.cl:52-61`, clamp at
   `pco_nir_tex.c:664-665`). Then `F2i`, then the integer multiply by `w[4]`.

`texelFetch` takes an integer layer; ES leaves out-of-range fetches undefined,
Mesa clamps the same way (`:664-665`). Keep the clamp (a bad address is a TCU
fault: `backend.rs:3610`).

NaN: `Fmax`/`Fmin` order as today's `array_uv`; unchanged.

## 6. Storage: derived twiddled copy (recommended) or layer-major linear storage

The GL core stores textures linear, level-major (`gl/native_cube.rs:7-24`), and
the array's mip chain is a stack per level. The `tao` address gives layer
`l`'s base, so layer `l` has to be a whole image (its own chain) at
`base + l * layer_size`. Two ways:

**C. Derived copy (recommended).** Exactly `gl::native_cube` generalised: a
power-of-two twiddled copy, layer-major, made at draw time, re-made when
`content_gen` or the render-into `target_kick` moves
(`gl/native_cube.rs:181-283`; staleness at `gl/draw.rs:3445-3470`). The
descriptor is `TEXTYPE_2D` + `tex_twiddled_image_word1(.., depth = layers - 1,
mipmaps_present = chain > 1)`, `w[4] = layer bytes`. Why:

- no change to the stored layout, uploads, `glTexSubImage3D`,
  `glFramebufferTextureLayer`, `glGenerateMipmap` or the V3D / GC7000 layouts
  (they walk the layers themselves, `native_layered_textures`);
- levels >= 1 work because the TPU walks a twiddled chain from the address
  override: the 12 Pass -> Fail of the parked step 1 (stacked storage, level 0
  only) do not appear;
- the cube copy is silicon-proven (0 of 3072 texels wrong,
  `native_cube.rs:29-31`), and the code to share (staleness, cache, `released`)
  exists;
- plan #2's files (`gl/texture.rs`, `gl/fbo*.rs`) are not touched.

Cost: a copy of the array on the first draw and after every change, and 2x
memory while both exist (cache of 16 entries, `native_cube.rs:55`). A 256-layer
RGBA8 64x64 array is 4 MiB per copy.

**L. Layer-major linear storage (the parked step 2).** Change `chain_layout`
for the PowerVR array so layer `l`'s whole chain is at `l * layer_size`, use
`TEXTYPE_STRIDE`, and keep storage and sampling the same bytes. No copy, but
it changes `gl/texture.rs` `chain_layout`/`level_off` and every user of them
(upload, sub-image, `glCopyTexSubImage3D`, the FBO layer attach, `generateMipmap`)
for one back end, and a stride descriptor cannot carry `depth` (section 3.1), so
the shader would need the count from a uniform. Fallback only if C's memory or
copy cost shows up on the board.

Choose C. Everything below assumes it; the L differences are the descriptor
(`STRIDE` word) and where the layer count comes from.

Copy rules (the cube's, with `p` -> `(pw, ph)`):

- `pw = next_pow2(w)`, `ph = next_pow2(h)` (`pvr_image.c:83-92`);
- `single` (no mipmap filter or one level): level 0 alone, the descriptor says
  one level; else the whole chain down to 1x1 whatever the sampler needs
  (`native_cube.rs:158-177`, `backend.rs:3699-3716`; Mesa `pvr_image.c:173-188`);
- layer bytes = chain texels * bpp, aligned to 4 KiB when `layers > 1`
  (`pvr_image.c:195-196`);
- copy texel `(x, y)` of layer `k`, level `l` from
  `level_off[l] + ((k * hl + y) * pitch + x) * bpp` (`hl = max(1, h >> l)`) to
  `layer_base + level_texel_off(pw, ph, l) * bpp + twiddle_index(x, y, pwl, phl) * bpp`;
- compressed arrays (ETC) are not copyable at first: keep the stacked image
  (`copyable()` false, as for a cube without a CPU view,
  `native_cube.rs:144`), and the program must then not be a native one: see
  section 8.

## 7. Shadow samplers (`sampler2DArrayShadow`, `sampler2DShadow`, `samplerCubeShadow`)

### 7.1 What Mesa does: no hardware compare

- The depth is sampled as a plain float look-up; the compare is shader code on
  the filtered result: `lower_tex_shadow` -> `nir_alphatst_pco` per channel
  (`pco_nir_tex.c:430-443`, `:754-779`; `pco_trans_nir.c:1853-1871`: `alphatst`
  then `psel`).
- The compare op is the sampler meta word 0 (`PCO_SAMPLER_META_COMPARE_OP`,
  `pco_data.h:151-157`; `vulkan/pvr_arch_sampler.c:112-118`: VkCompareOp or
  NEVER). The reference is clamped to [0, 1] for a unorm image
  (`pco_nir_tex.c:772-776`, `PCK_INFO` bit 8). Yakogl has all of this:
  `shadow_compare` (`select.rs:2368`), `w[12]` (`backend.rs:3763`).
- `SAMPLER_WORD0` does have a `cmp_mode` field (bits 59..61, `CMP_MODE`:
  `texstate.xml:52-61`, `:344`), overlapping the YCbCr plane address
  (`pvr_arch_sampler.c:35-39`). Mesa 26.2.1 never writes it (no use anywhere
  under `src/imagination/` outside the XML), and no `smp` operand carries a
  reference value (`pco_nir_tex.c:242-345`; `ROGUE_SMP_MAX_DATA_WORDS` is 18,
  `rogue_hw_defs.h:191`). So **there is no attested hardware depth compare, and
  none that can be driven from the operands pco emits.** Whether the TPU
  compares per texel if `cmp_mode` is set and a reference is somehow supplied is
  unknown. It is a one-run board probe at most (set `cmp_mode`, put the
  reference in an unused data word), not a design basis.

So Mesa's own Vulkan on this GPU compares after filtering, the same as Yakogl
today. The 180 QualityWarnings are the expected result of that, not a bug in the
selector.

### 7.2 What dEQP wants, and the implementable PCF

ES 3.0: with `TEXTURE_COMPARE_MODE = COMPARE_REF_TO_TEXTURE` the comparison is
done for each texel, and the filter then weights the comparison results.
`texture.shadow` expects that for LINEAR (and a mip chain). Software PCF, using
what Mesa has for gather:

- Gather sampler words: `pvr_sampler_descriptor.gather_words`
  (`pvr_common.h:198-202`), built at `pvr_arch_sampler.c:220-232`
  (`mipfilter = false`, `min/mag = LINEAR`, word 1 copied) and selected by
  `nir_load_smp_state_pco(.flags = tg4)` (`pco_nir_tex.c:515-521`,
  `pco_trans_nir.c:1719-1723`: dwords 16..19 of ours). **Yakogl leaves dwords
  16..19 zero today**: `tex_desc_sampler` (`backend.rs:3762`) fills 8, 9, 12 only.
- The sample: `tg4` is `smp` with `sb_mode = rawdata`
  (`pco_trans_nir.c:1793-1798`: `chans = enabled_channels`, destination
  `chans * 4`), `lodm = replace` with a level of 0 (`pco_nir_tex.c:645-649`) and
  the result swizzle `{ sample 2, 3, 1, 0 }` per channel
  (`pco_nir_tex.c:423-427`): raw sample 0 = (i0, j0), 1 = (i1, j0), 2 = (i0, j1),
  3 = (i1, j1) of the 2x2 footprint `i0 = floor(u * w - 0.5)`. In `isa_tables.rs:434`
  `sbmode` is a field of `smp_exta` (`rawdata` = 1).
- The PCF for LINEAR: gather the four depths (one `smp`; the array adds `tao`
  and the address words unchanged), compare each with the clamped reference
  (`shadow_compare`'s code per texel), then
  `mix(mix(c00, c10, fx), mix(c01, c11, fx), fy)` with
  `f = fract(coord * size - 0.5)`. `size` is `tex_size_2d`. Wrap and edge handling is the TPU's
  (the gather footprint), per layer for arrays.
- The program has to know the filter at run time (a sampler object can change
  it). Publish it in the sampler meta word the PowerVR leaves reserved:
  dword 13 = `PCO_SAMPLER_META_RSVD0` (`pco_data.h:153`), bit 0 = "PCF". The
  selector emits the existing one-sample compare and the gather arm and chooses
  with `bcsel` / a predicated `if` (the machinery used for `shadow_compare`'s
  bits, `select.rs:2368-2420`).
- Level: with `LINEAR` or `NEAREST` over one level the level is 0 (or
  `textureLod`'s). For the mipmap filters the footprint is at the level the TPU
  chose; the gather uses `lodm = replace`, so the level is the program's to
  compute (from the plane gradients as `plane_gradients` does for a gradient
  sample, or from `smp` `sbmode = coeffs`, `pco_nir_tex.c:132-190,720-725`), and
  `MIPMAP_LINEAR` is two PCFs lerped by the LOD fraction. This is the larger half
  of the 180; do it second.
- The five non-NEAREST filters times 18 cases times (2D, cube) is 180
  (`RESULTS.md:994`), and 108 for the array. `NEAREST_MIPMAP_NEAREST` needs no
  per-texel weighting (one texel), so its warnings may have another cause (level
  selection); the row itself says it was "a reading from the pattern, not
  measured". Reproduce one on the board before assuming PCF is the whole fix.

Scope split: `pvr-2darray` (this design, sections 3-6, 8-10) does the array
addressing and makes the array shadow forms correct for NEAREST. `pvr-shadow`
does 7.2 (gather, PCF, the remaining forms of `RESULTS.md:998`, `textureSize` on
shadow samplers) on top of it. The gather arm takes the same `tao` words, so
the order is array first.

## 8. What changes in `src/`

Selection (`src/shaderc/online/powervr/`):

- `sel.rs:304` `SelOp::Sample`: new field `layer: bool` (the `tao` form), and a
  group `SelOp::SampleAddr` (`srcs[0] = lo`, `srcs[1] = hi`) consumed by the chain
  like `SampleZ` (`sel.rs:309`; `prepare.rs:253,1382` list the markers).
- `select.rs`: new switch-guarded arms for `D2Array` (plain/lod/bias/offset/fetch,
  and `shadow: true` through the existing shadow arm `:1129`), a helper
  `array_addr(sampler, coord) -> (u, v, lo, hi)` replacing `array_uv`'s
  stacked rows (`:2606`), `layers_dword` use for the native path dropped
  (`:2575`), `TexSize` D2Array from the depth field (`:1192`), `smp_level`
  (`:2424`) reused, offsets allowed for arrays (`:1164`).
- `chain.rs:664` `expand_samples`: place `addr_lo, addr_hi` after the level word
  and before the lookup word (zero bias word when there is none, as `pco_nir_tex.c:288-294`);
  fix the loop zero-reset at `:777-783` (`data_words` + 2); consume
  `SampleAddr`; `:801`, `:872` carry the new field; `:435`, `:975` set the
  record flag (section 9).
- `fragment.rs:267` `build_smp`: new `tao` parameter, put in `exta`
  (`:296-308`) and in the "needs exta" test (`:309`).
- `assemble.rs:251-272` passes `layer`; `verify.rs:198` `check_smp` compares
  `tao`.
- Front end: nothing; the hidden `pvr_layers_` uniform stays for the stacked
  table rows (`reflect.rs:568-587`).

GL core and back end:

- New `src/gl/native_array.rs` (the copy, cache and `released` of
  `native_cube.rs`, `w x h` and N layers; share the cache code instead of
  duplicating it if the cube file is touched at all).
- `src/gl/draw.rs:3395-3470`: a `GLES_TEXTYPE_2D_ARRAY` + `sh.native_array`
  branch like the cube's, setting a new type (`GLES_TEXTYPE_2D_ARRAY_TWIDDLED`,
  `src/backend.rs:320`, `src/backend.h:139`, the C-ABI mirror
  `pvr_backend.h` and `ffi_layout_check.c`: an enum value only, no struct
  field) and `tb.width = t.width`, `tb.height = t.height`, `tb.layers`.
- `src/gl/context.rs:1648` release hook.
- `src/pvr/backend.rs:3677`: an arm for the new type next to the cube's:
  `tex_image_word0(fmt, ROGUE_TEXSTATE_TEXTYPE_2D, w, h)`,
  `tex_twiddled_image_word1(etc, chain, layers - 1, chain > 1, base)`,
  `w[4] = layer bytes`, `w[5], w[6] = base lo, hi`. `tex_desc_sampler` as is
  (`addrmode_w` clamp). The non-native arms are untouched.
- `include/gles_shaderc.h:375-380` / `src/shaderc/mod.rs:1101`: the record flag:
  bit 1 of the existing `native_cube` byte (`2` = native array), so no field
  moves and `src/abi_layout.rs` does not change. `draw.rs:3445` already guards
  with `type_ == CUBE`.
- `src/shaderc/mod.rs:116-141`: a switch `pvr_native_array` (`pvr_set_native_array`,
  `native_array_enabled`) beside the cube's, **default OFF first**, flipped
  after the board run; and `src/shaderc/precompiled.rs:149-160`
  `switches_at_boot` must include it, or a table made under another value would
  answer.

## 9. ES 2.0 and table boundary: what stays byte-identical

With the switch OFF (the first landing), nothing outside the new files and the
guarded arms moves. With it ON, only ES 3.0 programs that sample a 2D array move.

| Artifact | Why unchanged | How the gate shows it |
|---|---|---|
| ES 2.0 programs and every GPU's `report-es2` build | ES 2.0 has no `sampler2DArray`; the new arms match `TexDim::D2Array` only; `draw.rs` branch needs `type_ == 2D_ARRAY` and the record flag | `check.sh` `report-es2` configs; `tests/fixtures/online_refused_powervr_es2.txt` and `precompile_counts.txt` lines `es2=1` identical |
| `src/pvr/internal.rs`, `src/vivante/internal.rs` | the internal programs (`quad`, `fbload*`, `clear*`) sample no array (`internal.rs:12-40`) | `check.sh:1213` `--internal --verify src/pvr/internal.rs` |
| the precompiled table rows (pco / glsl2usc) | they keep sampling the stacked image: their record's `native_cube` is 0 (never 2), so `draw.rs` keeps the stacked bind; `pvr_layers_` is still filled | `precompiled.rs:615` compares the flag byte; the reproduction rows of `check.sh` |
| `precompile_counts.txt` `powervr es2=0` | OFF: identical. ON: rows of our compiler that sample an array change code size: re-pin with a reason (`PRECOMPILE_PIN_WRITE=1`) | `tools/precompile --pin` |
| GC7000 / V3D | no change in `src/vivante`, `src/v3d`; the `layered` branch of `draw.rs:3372-3388` is untouched | their unit tests |
| `tests/fixtures/online_refused_powervr.txt` (ES 3.0) | ON: sources refused over array offsets / shadow forms now compile: the diff is expected and reviewed | fixture diff |

2D, cube (native or stacked), 3D (stacked grid, `vol_uv`), `texelFetch` on 2D,
`textureSize` on 2D, `shadow_compare` for 2D/cube are not edited. The new
arms sit before the catch-all and match only `D2Array`.

## 10. Host tests (no hardware)

State words (in `src/pvr/backend.rs` tests, next to
`the_cube_probe_descriptor_is_a_twiddled_textype_cube`, and the sweep in
`backend_csbgen_tests.rs:422`):

1. `an_array_descriptor_is_a_twiddled_textype_2d_with_the_layer_count`:
   `iw0 & 7 == 1`, width/height are one layer's, `iw1 & 0xf` = chain,
   `(iw1 >> 4) & 0x7ff == layers - 1`, `(iw1 >> 15) & 1`, texaddr, `iw1 >> 60 == 0`,
   `w[4]` = the layer bytes (4 KiB multiple), `w[5..7]` = base lo/hi (hi
   `0x80`), `desc_maxlod`; a one-level copy says one level; ETC.
2. Same sweep as `csbgen_texture_descriptors_equal_the_hand_packed_ones`
   against a hand-packed reference for 1, 2, 7, 256 and 2048 layers (the 11-bit
   limit).
3. The non-array descriptors are unchanged: the existing pins (`TEX_IMAGE_WORD*`)
   run as they are.

Layout (`src/gl/native_array.rs` tests, as `native_cube.rs` tests):
`layer_bytes` for 1, 2, 5 layers, non-square (e.g. 8x2, 5x3), single vs chain,
`twiddle_index` with `pw != ph`; a copy of a known array checked texel by texel
at every (layer, level, x, y); stale after `glTexSubImage3D` and after a
render-into (`content_gen`, `target_kick`); `released`.

Instruction encoding (`tests/usc_limits.rs`, beside
`a_2d_array_samples_its_layer_in_the_stacked_image:5218` and
`a_texel_fetch_from_a_2d_array_reads_its_layers_texel:5957`; the native-cube
pair `a_native_cube_samples_as_pcos_programs_do:8046` is the model):

4. `a_native_array_samples_as_pcos_program_does`: compile `texture`,
   `textureLod`, `texture` with bias, `textureOffset`, `texelFetch` for
   `sampler2DArray` with the switch on; decode the group: `smp` form `exta`,
   `tao = 1`, `array = 0`, `dmn = 2d`, `pplod = 1`, `lodm` per the table in
   4.1, `soo`/`nncoords` as listed, and the data block's order (copies into
   `data+0..`: u, v, level, lo, hi, lookup), with the zero bias word when there
   is no level. Attest the field set against `pco_shc` (`tools/pco_shc`,
   `build-new`) on the same sources, as `fetch_fields` does for fetch
   (`usc_limits.rs:5870-5890`).
5. `the_record_says_native_array`: `rec.native_cube == 2` for a program with an
   array sampler; 0 with the switch off, 0 for stacked table rows, 1 for a cube.
   A dead array sample takes its words with it (`a_dead_cube_sample_takes_its_z_with_it:7979`).
6. Layer rounding: IR agreement through `tests/usc_exec` (needs `tao` modelled:
   today `mod.rs:849-852` refuses `tao`): the model applies `base + clamp(floor(z + 0.5), 0, depth) * w4`
   per layer, with layers -1, 0, 0.49, 0.5, 1.5, 2.5, `depth`, `depth + 1`
   and 1e9; 2.5 -> 3 (not Mesa's 2).
7. `textureSize` of a native array: `(w >> l, h >> l, depth + 1)` at levels 0 and
   1 with 3 and 5 layers (the stacked pair is
   `texture_size_of_a_stacked_texture_divides_the_stacked_height`).
8. The 64-bit carry: base `0x80_FFFF_F000`, layer size 0x1000, layer 3.
9. The loop zero-reset: an array sample inside a loop with a lookup word.
10. ES 2.0 identity: `report-es2` builds run the identical-output assertions
    (`internal.rs --verify`, `online_refused_powervr_es2.txt`); a test that
    the stacked path is unchanged with the switch off (the existing two array
    tests, run with `pvr_set_native_array(0)`).
11. `draw.rs`: a draw with a native-array program binds the copy type and a
    stacked program does not (as `a_native_cube_is_copied_again_after_a_render_...:10788`).

Board (after the host tests): the dEQP rows named in `RESULTS.md:995` (texture.filtering.2d_array
210, texture.shadow 2d_array NEAREST, texture_functions array cases) and the
cube/2D ES 2.0 rows (the ES 2.0 whole tree) with the switch on and off.

## 11. Open points

- Section 7.1: whether `SAMPLER_WORD0.cmp_mode` does anything on the BXS-4-64.
  Not needed for the plan; one probe if wanted.
- The meta words 5 and 6 for the base (section 4.2): confirm that no table
  program reads them (pco's programs read `LAYER_SIZE`, `PCK_INFO` and, for a 2D
  view of 3D, `Z_SLICE` only: `pco_nir_tex.c:552`; the stacked table rows are
  not given this descriptor type anyway).
- A table row whose program samples a `sampler2DArray` by pco's stacked form
  while the online compiler's switch is ON: stays on the stacked bind because
  the flag is per record. A program with both a native array and a stacked
  3D sampler (the 3D grid) is fine, the guards are per type.
- `textureGrad` on an array (`select.rs:1164` lists gradients): a follow-up,
  the data block is in 4.1.
- ES 3.0 `sampler2DArrayShadow` offers only `texture` and `textureSize` in the
  specification as far as this author recalls; the corpus
  (`texture_functions.sampler2darrayshadow_{fragment,vertex}`, 2 cases) is the
  authority.
