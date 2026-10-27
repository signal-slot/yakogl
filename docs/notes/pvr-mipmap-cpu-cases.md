<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the glGenerateMipmap cases that stay on the CPU filter, and why

State at main `51f10a7b`. `d3b8d4b2` draws a 2D RGBA8 chain on the GPU, one
quad per level (`src/pvr/backend.rs` `pvr_generate_mipmap`, offered from
`src/gl/texture.rs` `glGenerateMipmap`). glmark2 calls glGenerateMipmap only
with that case (`gc-mipmap-cpu-cases.md` lists its three scenes). No
benchmark row and no dEQP-GLES3 case measured here shows the cost of any
other case, so none of them moves to the GPU now. Each one below stays on
the CPU box filter (`glGenerateMipmap`, the loop after the offer), with the
reason it is not offered.

| Case | Where it is refused | Why it stays on the CPU |
|---|---|---|
| Cube maps | `t.target == GL_TEXTURE_2D` in the GL core's offer; `t.type_ != GLES_TEXTYPE_2D` in `mip_chain_gpu` | The storage is six faces stacked, each face its own chain. `mip_chain_layout` and `record_mip_level` walk one image. A face loop needs the face stride and a per-face check of attachments, and the PowerVR NPOT cube keeps one level (`pvr-npot-levels.md`). The CPU filter does it face by face already. |
| Narrow formats (L8, A8, LA8, RGB565, RGBA4, RGB5A1) and float (RGBA16F) | `t.texfmt == GLES_TEXFMT_RGBA8` in the offer; `t.format != GLES_TEXFMT_RGBA8` in `mip_chain_gpu` | The level quad is drawn by the fixed program `PVR_INTERNAL_MIPGEN` into an RGBA8 PBE target, and the draw's `RGBA8_BYTES` and the level offsets assume four bytes a texel. Each other format needs its own PBE target format and a TPU read that returns the channels the CPU filter averages (L and A as one byte each). Float averages need the RGBA16F target and its blend-free store. Nothing in the tree draws into those formats from this path, so each would be new, untested-on-board code. |
| sRGB (SRGB8_A8) | the same `texfmt` test | The CPU filter averages in linear space (decode, average, encode). A linear-filtered TPU read of an sRGB texture decodes, and a PBE store to an sRGB target encodes, but this is only the same result when both sides are set as sRGB; nobody has run it on a board. A host test cannot show it: the stand-in GPU draws no texels (`tests/mipmap_offer_guard.rs`). |
| A 1-wide level above a taller tail (32x64, 1x64, ...) | `lv[1..n].iter().any(l.pitch % 2 != 0 && l.rows > 1)` in `mip_chain_gpu` | The PBE writes rows in pixel pairs (`pbe_reg_words` linestride), so a level one texel wide with rows under it cannot be a render target at its own pitch. It would need the level drawn at a two-texel pitch into scratch and copied, for chains glmark2 never makes (power-of-two squares and 800x600). |
| ES 3.0 sized formats (`es3_fmt != 0`) | `t.es3_fmt == 0` in the offer | These are the formats of `src/gl/es3_formats.rs` (RGB10_A2, R11F_G11F_B10F, integer, R8/RG8, ...). Integer formats have no filtered chain (an unfilterable format is not mipmapped by glGenerateMipmap, `generate_mipmap_follows_renderable_and_filterable`), and the others are the formats above under other names. The same reasons apply. |

## What would move one

The cases go in this order if an application or a dEQP-GLES3 case shows the
cost, one format at a time and each with a board run, because the stand-in
GPU of the host tests draws nothing and cannot check the texels:

1. Cube maps (RGBA8): a loop of `mip_chain_gpu` over the faces. The host
   test is the call pattern of `tests/mipmap_offer_guard.rs` (kicks per
   face); the contents need the board, as for 2D.
2. Other 4-byte-or-fewer UNORM formats the PBE can render (RGB565, RGBA4,
   RGB5A1, R8, RG8), each with its target format in `record_mip_level`.
3. sRGB and float, last: they need the conversion shown on a board before
   the CPU filter's output can be the host reference.

The transfer queue was not tried as an alternative; it would copy and not
filter, so it only helps if the firmware's blit filters.

Done-when of the issue: every case above either goes to the GPU with a test
or is listed here with its reason. None goes to the GPU yet, so the issue
stays open for the first one that an application shows.
