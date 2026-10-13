<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the level layout of an NPOT 2D chain (`2d.npot_size`)

Item `am62p-es2-r1c`, 2026-10-13, host only. No board was used and nothing
under `src/` changed.

## Why

`docs/es2-cts-failures.md` (section H.5 and the "Round 1" table) names a risk
that no test covers: the PowerVR TPU's level arithmetic for
`dEQP-GLES2.functional.texture.completeness.2d.npot_size`, a 255x255 chain
whose level 2 is 63x63 at a 64-texel pitch. Only that case and glmark2's
terrain reach it. No board run has failed on it (AM62P main `32778b57` and
later: none of the 18 NPOT cases is among the failures), so this is a pin of
what the GL core hands the back end, not a fix.

## What is pinned

`tests/pvr_npot_levels.rs` (5 tests, no `#[ignore]`):

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_npot_levels
```

It replays `Incomplete2DSizeCase::createTexture` (RGBA8, REPEAT,
NEAREST_MIPMAP_NEAREST, one `glTexImage2D` for each of 255, 127, ... 1 in
that order, then a quad) on a stand-in back end with the PowerVR's flags
(`mip_level_align` 0, `native_layered_textures` false), and checks:

| Item | Expected | Derived from |
|------|----------|--------------|
| level offsets (bytes) | 0, 262144, 327680, 344064, 348160, 349184, 349440, 349504 | rule 2 below |
| level pitch (texels) | 256, 128, 64, 32, 16, 8, 4, 2 | rule 2 |
| level count | 8 (GL's), not 9 (the physical chain) | GL: 255 >> 7 = 1 |
| storage | at least 349524 bytes | 4 * (4^9 - 1) / 3 |
| level 2 | 63x63 at pitch 64, offset 327680; texel (62,62) is level 2's, column 63 is padding | rules 2, 3 |
| every texel | level l's byte (l + 1) inside its GL extent, 0 in the padding | rule 3 |
| bind | 255x255, `stride_texels` 256, 8 levels, `mip_filter` 1, base = level 0 | rules 1, 2 |
| IMAGE_WORD0 | textype 4, width - 1 = 254, height - 1 = 254 | texstate.xml |
| STRIDE_IMAGE_WORD1 | low 16 bits 0x80FF (stride - 1 = 255, mipmaps_present), bits 59..63 = 0x11 (alpha_msb, 8 levels) | texstate.xml |
| no mip filter | stride 255, no mipmaps_present, 1 level | `build_tex_desc` rule: one level without a mip filter |

The rules (Mesa 26.2.1 pvr, `pvr_image.c`, as recorded in
`tools/cts/log/investigations/pvr-render-pool.md`; `driver/docs/pvr-render.md`
itself holds no texture layout, only the pool and the fence wait):

1. `pvr_image_plane_init_physical_extent`: a multi-level image's extent is
   rounded up to a power of two per axis (255 -> 256).
2. `pvr_image_plane_setup_mip_levels`: level l is `256 >> l` texels wide and
   high; its pitch is that width; its offset is the sum of the sizes of the
   levels before it; the layer is sized as if the chain went down to 1x1
   (nine physical levels), though GL's chain of 255 has eight.
3. A level sits in the top-left of its physical extent; the rest is padding.

The numbers were worked out from these rules first (the table in the test's
header), then compared with the code. They agree.

## Result

The code and the rules agree at every level, so there is no `#[ignore =
"BUG: ..."]` test. The upload order matters and is covered: level 0 is
uploaded alone, level 1 grows the one-level texture into its chain, and level
0's texels survive that (`relayout_keeping_levels`).

## What is not pinned, and why

- **The texture words are not read from the code.** `build_tex_desc` in
  `src/pvr/backend.rs` is private and the real PowerVR back end cannot run on
  the host, so the test cannot call it. It encodes the words from the bind
  (the function's inputs) by the rules above, so the bind-to-words step is
  covered by the in-library test
  `an_800x600_npot_chain_is_a_stride_texture_of_the_power_of_two_pitch`
  (same word layout, 800x600), not by this file. A follow-up that exposes the
  builder to integration tests under `host-test` would close that gap; that is
  a `src/` change and outside this item.
- **Hardware.** Whether the TPU reads level 2 at offset 327680 with a 64-texel
  pitch is for the AM62P to say: dEQP `texture.completeness.2d.npot_size` in
  the final tree (`am62p-es2-final-a`), and glmark2 terrain.
- The sampler's LOD range (maxlod `(levels - 1) * 64`) is not part of this
  pin.
- Cube maps keep their base level on the PowerVR (H.1), so they have no
  level layout to pin.
