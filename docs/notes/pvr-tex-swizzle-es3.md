<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# PowerVR texture swizzle: the 80 AM62P `texture.swizzle` fails at origin/main

Cause T-SWIZZLE of `tools/cts/am62p-notpass-causes.py`: 80 Fail lines under
`dEQP-GLES3.functional.texture.swizzle.*` in
`docs/es3/am62p-gles3-notpass.tsv` (the ad2b5ed whole-tree run of
2026-10-14). This note checks them on the host at origin/main
**630fcdfac** (2026-11-28).

**Verdict: every group passes on the PowerVR host path at 630fcdfac.
No cause is left. All 80 are expected to pass on the next AM62P run.**
They failed at ad2b5ed because the swizzle never reached the texture
state. The GL core now hands it to the back end
(`GlesTextureBind::swizzle`, docs/es3-cts-failures.md RC18). Since 2df189df
(2026-11-11, "pvr: texture swizzle composed into the texture state") the
PowerVR puts it into the texture state.

## 1. What the cases do (dEQP source)

From `modules/gles3/functional/es3fTextureSwizzleTests.cpp` (VK-GL-CTS at
93bca01, as tools/cts/README.md fetches it):

- Every case builds a 2D texture of up to 128x128, level 0 only. The level
  is a component gradient. The texture uses NEAREST and CLAMP_TO_EDGE and
  sets `GL_TEXTURE_SWIZZLE_{R,G,B,A}`. One `texture()` quad is drawn in
  GLSL ES 3.00 and read back with `glReadPixels`.
- The reference is `sampleTexture` into RGBA8. That gives the base-format
  vector of ES 3.0.6 table 3.24, for example (0,0,0,A) for GL_ALPHA and
  (L,L,L,1) for GL_LUMINANCE. The reference then applies the swizzle, with
  GL_ZERO = 0 and GL_ONE = 255 (`swizzle()`, lines 49-81).
- `single_channel` (20 cases): GL_RGBA8. For each channel r/g/b/a, each of
  red, green, blue, alpha, zero and one except the channel's own default.
- `multi_channel` (63 cases): 7 formats x 9 swizzle sets. The formats are
  alpha, luminance, luminance_alpha (unsized, GL_UNSIGNED_BYTE), red (R8),
  rg (RG8), rgb (RGB8) and rgba (RGBA8). The sets are all_red, all_green,
  all_blue, all_alpha, all_zero, all_one, bgra, abgr and one_one_red_green.

## 2. Groups, counts and host verdict

The AM62P counts are the TSV's Fail lines. 83 cases are in the mustpass. At
ad2b5ed only alpha_bgra, luminance_bgra and luminance_alpha_bgra passed,
because in those cases the swizzle does not change the picture.

| group | mustpass | AM62P Fail (ad2b5ed) | host at 630fcdfac |
|---|---:|---:|---|
| single_channel.r_{green,blue,alpha,zero,one} | 5 | 5 | pass |
| single_channel.g_{red,blue,alpha,zero,one} | 5 | 5 | pass |
| single_channel.b_{red,green,alpha,zero,one} | 5 | 5 | pass |
| single_channel.a_{red,green,blue,zero,one} | 5 | 5 | pass |
| multi_channel.alpha_* | 9 | 8 (bgra passed) | pass |
| multi_channel.luminance_* | 9 | 8 (bgra passed) | pass |
| multi_channel.luminance_alpha_* | 9 | 8 (bgra passed) | pass |
| multi_channel.red_* | 9 | 9 | pass |
| multi_channel.rg_* | 9 | 9 | pass |
| multi_channel.rgb_* | 9 | 9 | pass |
| multi_channel.rgba_* | 9 | 9 | pass |
| **total** | **83** | **80** | **83 pass, 0 fail** |

The constants are covered by the swizzle sets: ZERO by `*_zero` and
`all_zero`, and ONE by `*_one`, `all_one` and `one_one_red_green`. Each
channel source (R, G, B, A) is tested for every output channel, in
`single_channel` and in the `all_*` sets.

## 3. Evidence

### 3.1 Existing host tests at 630fcdfac (all pass)

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
  --test es3_pvr_swizzle --test es3_tex_filtering
```

- `tests/es3_pvr_swizzle.rs`: 5 passed. It checks the descriptor's
  swiz0..3 for the identity, a permutation, ZERO/ONE, and the A8/L8
  composition with the format's own swizzle.
- `tests/es3_tex_filtering.rs`: 7 passed. It checks that the GL core's bind
  carries `GL_TEXTURE_SWIZZLE_*` for every 8-bit format, from the texture
  and not from a sampler object, and in the upload hook too.

Neither test joins the two halves for dEQP's exact cases. The scratch test
below does.

### 3.2 Scratch test (not committed): dEQP's 83 cases, GL core to TPU state

`tests/scratch_pvr_swizzle.rs` (a throwaway) took the recording back end of
`tests/es3_tex_filtering.rs` and ran it with the same feature set. For each
of the 83 cases it does the following:

1. It runs dEQP's GL calls. A 4x4 level 0 of the case's
   internalformat/format/GL_UNSIGNED_BYTE is uploaded with distinct
   component bytes (0x11, 0x22, 0x33, 0x44 for as many components as the
   format has). Then it sets NEAREST, CLAMP_TO_EDGE and the case's four
   `GL_TEXTURE_SWIZZLE_*`, and draws one quad with a `#version 300 es`
   `texture()` shader. The shader compiles through the in-process
   compiler.
2. It passes the recorded `GlesTextureBind` to
   `yakogl::pvr::backend::texture_descriptor`, which is the
   `build_tex_desc` the PowerVR draw calls (src/pvr/backend.rs, the
   per-binding loop of the draw). It decodes TEXSTATE_IMAGE_WORD0
   `texformat` [27:33] and swiz0..3 [14:16], [11:13], [8:10], [5:7]
   (third_party Mesa csbgen/rogue/texstate.xml). SRCCHAN_n reads channel n
   of the texel in the store; SRC_ONE is 4 and SRC_ZERO is 5.
3. It reads the texel the GL core stored at `base_gpu` (the host OSAL maps
   the GPU address to the CPU pointer). It evaluates the TPU's look-up from
   that texel and the decoded swizzle, and fails a case whose swizzle reads
   a channel the TPU format does not have.
4. It compares the result with dEQP's reference: the table 3.24 base vector
   of the uploaded bytes, swizzled, with ONE = 255.

Result: **83 cases, 0 fail**. The paths the run showed:

| dEQP format | GL core store (`GLES_TEXFMT`) | TPU texformat | format swizzle (then the bind's on top) |
|---|---|---|---|
| alpha | A8 (9) | U8 (0) | ZERO, ZERO, ZERO, R |
| luminance | L8 (8) | U8 (0) | R, R, R, ONE |
| luminance_alpha | LA8 (10) | U8U8 (7) | R, R, R, G |
| red, rg, rgb, rgba | RGBA8 (0); absent channels stored as 0 and alpha as 0xff | U8U8U8U8 (12) | identity |

Some example lines: `alpha` abgr gives (00, 00, 00, 11), so A goes into
red and every other output is ZERO. `luminance` all_alpha gives
(ff, ff, ff, ff), the format's ONE. `red` bgra gives (00, 00, 11, ff).
`rgb` abgr gives (ff, 33, 22, 11), the stored alpha 0xff read as a channel.

### 3.3 What the host run does not show

- It does not execute the TPU. It relies on Mesa's swiz field meaning
  (pvr_get_hw_swizzle composes a view's swizzle the same way) and on the
  first texel only. Filtering is NEAREST over a gradient, so the position
  in the texel grid does not depend on the swizzle.
- Host dEQP renders nothing on the PowerVR (RC18). Only a board run shows
  the pixels.
- Two conditions could hide a pass behind another cause. The first is a
  default framebuffer readback that is not RGBA8: dEQP converts the
  reference to the target format, so it would be the same for every
  texture case. The second is a tree image that is not built from a commit
  at or after 2df189df. The 472630df images of board-a..d contain
  2df189df.

## 4. Remaining cause and proposed fix

None. No defect remains, no fix is proposed, and no new issue is needed.
If a board run at or after 2df189df still shows a swizzle Fail, the first
places to look are these:

- `compose_tex_swizzle` and its call in `build_tex_desc_lod`
  (src/pvr/backend.rs), and `TEXF_A8` / `TEXF_L8` / `TEXF_LA8`.
- Whether the native-cube or native-3D branches that follow the call
  overwrite `fmt.swiz`. This does not matter for these 2D cases.
- The GL core's `GlesTextureBind::swizzle` fill (src/gl, RC18).

## 5. Rule row for `tools/cts/am62p-notpass-causes.py` (for causes-index)

```python
    ("T-SWIZZLE", {F}, r"^texture\.swizzle\.", None, "known",
     "docs/notes/pvr-tex-swizzle-es3.md", "swizzle not in the texture state at ad2b5ed; fixed by 2df189df (host: 83/83 at 630fcdfac), expected to pass on rerun"),
```
