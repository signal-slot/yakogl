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

Status. On main: the decoder (`src/gl/etc2.rs`, against Mesa's outputs), the
six names in the GL core (`texture.rs` `decoded_format`, `decoded_image`,
`decoded_sub_image`; tests `tests/es3_etc2_eac.rs` and the unit tests beside
them), the V3D's four block classes in `V3D_SAMPLED_TEXFMTS` (host draw test
`tests/es3_etc2_v3d.rs`) and the V3D's shadow compare in `sampler_for`
(`tests/es3_shadow_v3d_draw.rs`). Nothing here was measured on a board; the
host dEQP before/after for the three GPUs is section 7. The rules of section 4
are implemented in part; what differs is listed there.

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

Sources are the CTS files named; they are not in this machine's deps cache,
so the lines below are the design item's reading, not re-read for this note
(the sRGB `colorBits` line was read in 93bca01).

- The reference decoder widens an 11-bit value with
  `extend11To16(v) = (v << 5) | (v >> 6)` (`tcuCompressedTexture.cpp:804-816`;
  the signed form applies the sign) and the reference level is
  `UNORM_INT16` / `SNORM_INT16`. Our decode (`u16 / 65535`,
  `max(i16 / 32767, -1)` on the same 16-bit value) is the same number.
- `texture.wrap` (`es3fTextureWrapTests.cpp:388-399`) uses
  `computeColorBitsThreshold(8, colorBits) / colorScale`. `colorBits` is 6
  for every sRGB format in every filter mode, nearest-only included
  (`es3fTextureWrapTests.cpp:391-392`); for the other formats it is 7 for
  nearest-only and 6 otherwise. `tcuTexVerifierUtil.cpp:49-56` gives
  `(2^(b-n) + 1/256) / (2^b - 1)`: 7 bits 0.0078585, 6 bits 0.015702. The
  signed formats (colour scale 0.5) double them: 0.015717 and 0.031403. So
  the R11/RG11 formats get 0.0078585 (unsigned) or 0.015717 (signed) with
  nearest-only filtering, and the sRGB formats get 0.015702 whatever the
  filter.
- `texture.format` (`es3fTextureFormatTests.cpp:192/413/590`) uses
  2/255 = 0.00784 and NEAREST.
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

Design:

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

Done (this change): the decode goes through the uncompressed entry points:
`glTexImage2D` / `glTexImage3D` with no texels make the level, then
`glTexSubImage2D` / `glTexSubImage3D` write it one stripe at a time (one block
row, at most 64 texels wide, `Decoded::stripe`, `upload_stripes`), the texels
as `GL_HALF_FLOAT` or `GL_UNSIGNED_BYTE`, halves rounded to nearest by
`es3_formats::f32_to_half`. The library is `no_std` without a heap, so a stripe
is a 1 KiB stack buffer, not an image-sized one; the cost is one sub-image call
per 64 x 4 texels (4,096 for a 1024 x 1024 level). The calls run under the
default unpack state and with no PIXEL_UNPACK_BUFFER bound,
`with_default_unpack`; the application's state is put back. The compressed call's own `data` was resolved against its
PIXEL_UNPACK_BUFFER first. A sub-image is checked as `compressed_tex_sub_image`
checks one (level defined, inside the level, whole blocks or the level's edge,
imageSize); the level is told to be of the format only by its storage class
(`t.texfmt`).

Not done, and issue drafts of them are in the commit report: `decoded_from` (so
a level made by `glTexImage2D(GL_R16F)` is not told from one made by
`glCompressedTexImage2D(GL_COMPRESSED_R11_EAC)` and takes a compressed
sub-image where ES 3.0 says `GL_INVALID_OPERATION`), the refusals of
`glCopyTexSubImage*`, `glGenerateMipmap` and a framebuffer attachment of a
decoded texture (it is a plain RGBA8 / R16F texture to the core, so those
work), `glTexStorage*` with a compressed format, and the level-0 respecify
rule.
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
- The V3D: 11, since `V3D_SAMPLED_TEXFMTS` (`src/backend.rs`) holds the linear
  and sRGB ETC2 classes (before it the sRGB pair was not stored and the TMU
  refused every ETC2 draw as "a texel format the V3D does not sample").
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
  `driver/src/v3d/v3d_tex.c`) has all ten TMU rows, pinned by
  `tools/v3d/tex_check.c`; Mesa `v3dx_format_table.c:159-168`. The four block
  classes are in the mask. The EAC and punch-through ones need the class
  constants (`GLES_TEXFMT_ETC2_RGB_PT` exists), `etc2_of` arms and mask bits;
  the TMU rows R11, SIGNED_R11, RG11, SIGNED_RG11 (types 34-37, X001 / XY01)
  and RGB8_PUNCHTHROUGH_ALPHA1 (33) are in `etc2_row` already, so the work is
  the GL core's native storage of the six, `compressed_format_info` and the
  `stored_compressed_format` rule, not the back end's rows.
- PowerVR: Mesa `pvr_arch_formats.c:158-167`; `src/pvr/csbgen.rs` has the
  texstate constants 70-74 (`ROGUE_TEXSTATE_FORMAT_COMPRESSED_*`). The
  punch-through class needs only a mask bit.
- GC: Mesa `etnaviv_format.c:244-250`; `src/vivante/regs.rs`
  `GC_TEX_FORMAT_EXT_*` has the words. Punch-through needs a mask bit (the
  sRGB one through `SAMP_CTRL1.SRGB`); EAC needs new `gc_texfmt` arms.
- `GLES_TEXFMT_ETC2_RGB_PT` (class 3) is mapped by all three back ends
  (`etc2_of`, `src/vivante/texture.rs`, `src/pvr/backend.rs`) but is in no
  mask and has never run on a board.

## 7. What the host can and cannot show (dEQP before/after)

HOST-ONLY runs of the patched VK-GL-CTS 93bca01 (`MODULE=gles3
tools/cts/host-deqp-run.sh <filter>`, section 2b of `tools/cts/README.md`),
before at `f2748a1f` (main) and after this change, per `GPU=`. The Linux OSAL
runs no command stream, so every read-back is black: a case that reaches its
picture check fails as "Image verification failed" / "Image comparison failed",
before and after. What the host shows is whether the upload and the query
were accepted. Case counts are cases of the filter (the `etc2` and `eac`
filters overlap: wrap, format and the other groups name both).

| filter | GPU | before | after |
|---|---|---|---|
| `etc2` (240) | v3d | 160 NotSupported, 80 Fail (72 `GL_INVALID_OPERATION` at `es3fTextureWrapTests.cpp:371`, 4 at `es3fTextureFormatTests.cpp:1070`, 4 image) | 240 Fail, all at the picture check (216 verification, 24 comparison) |
| `etc2` (240) | powervr, vivante | 80 NotSupported, 160 Fail (picture check) | 240 Fail, all at the picture check |
| `eac` (240) | v3d | 200 NotSupported, 40 Fail | 240 Fail, all at the picture check |
| `eac` (240) | powervr, vivante | 160 NotSupported, 80 Fail | 240 Fail, all at the picture check |
| `texture.format.compressed` (44) | v3d | 32 NotSupported, 12 Fail | 44 Fail, all "Image comparison failed" |
| `texture.format.compressed` (44) | powervr, vivante | 24 NotSupported, 20 Fail | 44 Fail, all "Image comparison failed" |
| `implementation_limits` (49) | v3d | 38 Pass, 11 Fail | 40 Pass, 9 Fail |
| `implementation_limits` (49) | powervr, vivante | 41 Pass, 8 Fail | 43 Pass, 6 Fail |
| `compressed_texture_formats` (`state_query`, 6) | all three | 1 Pass, 5 Fail | 6 Pass |
| `texture.shadow` (432) | v3d, powervr | 432 Fail (picture check) | 432 Fail (picture check) |
| `texture.shadow` (432) | vivante | 432 Fail ("Failed to compile shader program") | the same |

Read: every one of the ten formats is now accepted by the upload in every wrap,
format and cube case the filters reach, on all three libraries (the NotSupported
rows are gone; the V3D's 76 upload errors, which were the sRGB pair it did not
store, are gone), and the two list rows of `implementation_limits` and the six
`state_query` rows pass. What the host cannot show: whether a case that now stops at the
picture check would pass it, so no `texture.wrap` or `texture.format.compressed`
pass count is claimed (the board's, #313 AM62P, #320 Pi 4, #316 i.MX8MP); the 11-bit tolerance of
section 3; and the 432 `texture.shadow` cases, which fail identically
before and after on the host because the compare is applied by the TMU to a
texture the host never renders (the record that carries the function is pinned
by `tests/es3_shadow_v3d_draw.rs` instead). The GC7000UL's 432 shadow cases fail
to compile (RC2 / `docs/notes/gc-shadow-compare.md`), which this item does not
touch.

## What is not measured

No board has sampled a decoded texture. The host shows the decoder against
Mesa's outputs, the GL core's rules and the draw path on the interpreter or
the back end's host tests; whether the PowerVR TPU, the V3D TMU and the GC
sample R16F, RGBA16F and SRGB8_A8 at the tolerances above on silicon is the
boards' to show (#313 AM62P, #320 Pi 4, #316 i.MX8MP).
