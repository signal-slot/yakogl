<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 ETC2/EAC: the six formats the GL core decodes at upload

Design, plan #354 (roadmap stage 4), decided by its design item at main
`6f9e52d4` against VK-GL-CTS `93bca01`. ES 3.0 requires ten ETC2/EAC formats.
The GL core stores four of them as blocks (`RGB8_ETC2`, `SRGB8_ETC2`,
`RGBA8_ETC2_EAC`, `SRGB8_ALPHA8_ETC2_EAC`; `ETC1_RGB8_OES` is the fifth name
and is not one of the ten). This note is about the other six and why they do
not get a class of their own yet. Function names are the address; line
numbers are of main at `6dc53970` and drift.

Status: the decoder (`src/gl/etc2.rs`), the six names in the GL core
(`texture.rs` `decoded_image`, `write_decoded`), the V3D mask bits and the
host tests (`tests/es3_etc2_eac.rs`, `tests/es3_etc2_v3d.rs`,
`tests/eac_half_precision.rs`) are on main; the host dEQP before/after rows
were not run on the machine that did the work (no VK-GL-CTS tree, see the
commit message). Nothing here was measured on a board.

## 1. The decision

The GL core decodes the six formats on the CPU at upload (decoder
`src/gl/etc2.rs`) into a class every back end already samples, on v3d, pvr
and vivante:

| GL name | stored as | texel | value |
|---|---|---|---|
| `R11_EAC` | `R16F` | (r, 0, 0, 1) | u16 / 65535 |
| `SIGNED_R11_EAC` | `R16F` | (r, 0, 0, 1) | max(i16 / 32767, -1) |
| `RG11_EAC` | `RGBA16F` | (r, g, 0, 1) | as R11_EAC per channel |
| `SIGNED_RG11_EAC` | `RGBA16F` | (r, g, 0, 1) | as SIGNED_R11_EAC per channel |
| `RGB8_PUNCHTHROUGH_ALPHA1_ETC2` | `RGBA8` | alpha 0 or 255 | rgb 0 where alpha is 0 |
| `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2` | `SRGB8_A8` | alpha 0 or 255 | rgb 0 where alpha is 0 |

`RG11` goes to RGBA16F because there is no RG16F class. The memory cost is 4x
for R11 (16 bits a texel against 4) and 8x for the others (RGBA16F 64 bits a
texel against 8; RGBA8 32 bits against 4). This is what Mesa's `util_format`
does when a driver has no hardware for a compressed format.

A format is stored only if the build is not `report-es2` and the back end
samples its decode class (`backend::sampled_texfmts()`, the same test
`stored_compressed_format` applies to the sRGB pair today). The four native
formats stay native. The Mali samples none of the decode targets, so the six
stay `GL_INVALID_ENUM` there.

Why not a native class now: each back end needs its own class constants, mask
bits and, on the GC, new `gc_texfmt` arms (section 6); the V3D and PowerVR
files are owned by other plans. The decode needs nothing from a back end.

## 2. Why R16F and not R32F

ES 3.0 does not filter 32-bit float (`es3_formats::filterable`), and
`texture.wrap` has LINEAR cases for the EAC formats. The GC does not sample
RGBA32F either. A half float holds all 2048 unsigned and all 2047 signed R11
codes as distinct values, so nothing is lost by the narrower class.

## 3. dEQP's tolerance for 11-bit EAC

Sources are the CTS files named, with every line number below read at
VK-GL-CTS 93bca01 (fetched with curl from raw.githubusercontent.com).

- The reference decoder widens an 11-bit value with
  `extend11To16(v) = (v << 5) | (v >> 6)` (`tcuCompressedTexture.cpp:804-816`;
  the signed form applies the sign) and the reference level is
  `UNORM_INT16` / `SNORM_INT16`. Our decode (`u16 / 65535`,
  `max(i16 / 32767, -1)` on the same 16-bit value) is the same number.
- `texture.wrap` (`es3fTextureWrapTests.cpp:388-399`) uses
  `computeColorBitsThreshold(bits, colorBits) / colorScale`, with `bits` the
  render target's 8 and `colorBits = max(bits - (isNearestOnly && !isSRGB ? 1 : 2), 0)`.
  So `colorBits` is 6 for every sRGB format in every filter mode,
  nearest-only included (`es3fTextureWrapTests.cpp:391-392`); for the other
  formats it is 7 for nearest-only and 6 otherwise. The per-channel scalar
  is `computeColorBitsError(bits, numAccurateBits)` at
  `tcuTexVerifierUtil.cpp:49-56`: `(2^(bits-numAccurateBits) + 1/256) / (2^bits - 1)`: 7 bits 0.0078585, 6 bits 0.015702. The
  signed formats (colour scale 0.5) double them: 0.015717 and 0.031403. So
  the R11/RG11 formats get 0.0078585 (unsigned) or 0.015717 (signed) with
  nearest-only filtering, and the sRGB formats get 0.015702 whatever the
  filter.
- `texture.format` (`es3fTextureFormatTests.cpp:192/413/590`) uses
  `getPixelFormat().getColorThreshold() + RGBA(1,1,1,1)` and NEAREST
  minification and magnification. Lines 192, 413 and 590 are the 2D, cube
  and 2D-array cases; the 3D, compressed 2D and compressed cube cases
  (758, 898, 1041) and the file cases (1167, 1287) use the same expression.
  2/255 = 0.00784 assumes `getColorThreshold()` is 1/255 for an 8-bit
  target (`tcuPixelFormat.hpp`, not read).
- Our error is the half-float rounding of a 16-bit value in [0, 1] or
  [-1, 1]: 2.37e-4 unsigned, 2.29e-4 signed with round-to-nearest-even
  (`es3_formats::f32_to_half`), about 3% of the tightest tolerance (nearest,
  0.0078585). The truncating `texture::float_to_half` gives 4.88e-4. Use
  `f32_to_half`.

The punch-through pair has no such margin question: RGBA8 holds the decoded
8-bit result exactly, and SRGB8_A8 is sampled through the same sRGB decode as
the native pair. Both are exact, so they are inside any threshold above,
including the sRGB 6-bit one.

## 4. The GL core's rules for a decoded texture

- `GlTexture.decoded_from` (a new field; grep `tests/` and `src/` for struct
  literals of `GlTexture` when it is added) records the compressed name.
- `glCompressedTexSubImage*` must use the same name; any other is
  `GL_INVALID_OPERATION`. A sub-image must be block aligned as for any
  compressed format; the blocks are decoded into the stored texels.
- `glCopyTexSubImage*`, `glGenerateMipmap`, colour-renderability
  (`src/gl/framebuffer.rs`) and EGLImage export (`src/gl/egl.rs`) are refused:
  they would read or write texels that are not the compressed data.
- A level-0 respecify with another format, or `glTexStorage*`, clears
  `decoded_from`. (`glTexStorage` with a compressed format is not supported
  by the GL core today; that is a separate gap.)
- `TEXTURE_3D` stays `GL_INVALID_OPERATION` (ES 3.0 allows it only with
  extensions). 2D arrays decode each layer. Cube faces and levels decode per
  call. A PBO source needs nothing extra: the upload reads the bytes from the
  buffer as for the native formats and decodes them.
- Partial edge blocks (width or height not a multiple of 4) decode a whole
  block and keep the texels inside the level.

## 5. Format counts

`GL_NUM_COMPRESSED_TEXTURE_FORMATS` and `GL_COMPRESSED_TEXTURE_FORMATS` list
what the build stores (`stored_compressed_formats`):

- PowerVR and GC under ES 3.0: 11 (ETC1 plus the ten).
- The V3D: 11, since `V3D_SAMPLED_TEXFMTS` (`src/backend.rs`) holds the
  linear and sRGB ETC2 classes.
- `report-es2`: 3 on every build (ETC1, `RGB8_ETC2`, `RGBA8_ETC2_EAC`),
  unchanged.
- Mali: the six are refused, so its count is what its mask allows.

The pins that move are in `src/gl/state.rs`, `tests/es3_caps.rs`,
`tests/gltest_case20_formats.rs`, `tests/vivante_imx8mp_limits.rs` and
`tests/es3_limits_v3d.rs`; they change in the commit that adds the names.

## 6. Native later (not done)

All three GPUs decode all ten in hardware. Going native replaces the decode
per back end and gives back the 4x/8x memory. These are follow-up issues:

- V3D: `etc2_row` (`src/v3d/texture.rs`, over the rows of
  `driver/src/v3d/v3d_tex.c`) already has all ten TMU rows, pinned by
  `tools/v3d/tex_check.c`; Mesa `v3dx_format_table.c:159-168`. It needs five
  class constants, `etc2_of` arms and mask bits.
- PowerVR: Mesa `pvr_arch_formats.c:158-167`; `src/pvr/csbgen.rs` has the
  texstate constants 70-74 (`ROGUE_TEXSTATE_FORMAT_COMPRESSED_*`). The
  punch-through class needs only a mask bit.
- GC: Mesa `etnaviv_format.c:244-250`; `src/vivante/regs.rs`
  `GC_TEX_FORMAT_EXT_*` has the words. Punch-through needs a mask bit (the
  sRGB one through `SAMP_CTRL1.SRGB`); EAC needs new `gc_texfmt` arms.
- `GLES_TEXFMT_ETC2_RGB_PT` (class 3) is mapped by all three back ends
  (`etc2_of`, `src/vivante/texture.rs`, `src/pvr/backend.rs`) but is in no
  mask and has never run on a board.

## What is not measured

No board has sampled a decoded texture. The host shows the decoder against
Mesa's outputs, the GL core's rules and the draw path on the interpreter or
the back end's host tests; whether the PowerVR TPU, the V3D TMU and the GC
sample R16F, RGBA16F and SRGB8_A8 at the tolerances above on silicon is the
boards' to show (#313 AM62P, #320 Pi 4, #316 i.MX8MP).
