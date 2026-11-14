<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 sized texture formats in the GL core

Built in `888af5d0` and `e5c4a180`. What `glTexImage2D/3D`, `glTexStorage2D/3D`,
`glTexSubImage2D/3D`, `glCopyTexImage2D` and friends take when the library
reports ES 3.0 (`report-es2` off), how the texels are stored, and what a back
end must do to sample them. Under `report-es2` nothing here exists
(`es3_formats::find` is `None`; the new type and format enums stay as unknown
to the upload calls as before).

Code: `src/gl/es3_formats.rs` (tables and conversions, no context),
`src/gl/texture.rs` (the wiring), `src/backend.rs` (the storage classes and
the back end's mask). Tests: `tests/es3_sized_formats.rs` (tables, per-GPU
accept/refuse matrix, conversions against hand-computed values,
completeness) and `src/gl/es3_format_tests.rs` (the GL entry points on the
mock back end).

## What is covered

The 35 colour formats of ES 3.0.6 table 3.2 that the original tables did not
hold: `R8/RG8/RGB8/RGBA8_SNORM`, `R11F_G11F_B10F`, `RGB9_E5`, `RGB16F`,
`RG32F`, `RGB32F`, `RGB10_A2`, `RGB10_A2UI`, and the 24 integer formats
`{R,RG,RGB,RGBA}{8,16,32}{I,UI}`. The formats the tables already held
(`R8`, `RG8`, `RGB8`, `RGBA8`, `SRGB8`, `SRGB8_ALPHA8`, `RGB565`, `RGBA4`,
`RGB5_A1`, `R16F`, `RG16F`, `RGBA16F`, `R32F`, `RGBA32F`) behave as before
(the sRGB pair in its own class, `docs/es3/srgb.md`);
`sized_format()` / `texfmt_of()` consult `es3_formats` only for a name they
do not know.

The pairs: `es3_formats::transfer_pair_valid` is table 3.2's colour rows (the
`*_INTEGER` bases, `BYTE`, `SHORT`, `UNSIGNED_SHORT`, `INT`, `UNSIGNED_INT`,
`UNSIGNED_INT_2_10_10_10_REV`, `UNSIGNED_INT_10F_11F_11F_REV`,
`UNSIGNED_INT_5_9_9_9_REV`, with their bases). An internal format of the new
table takes only its own row's pairs (GL_INVALID_OPERATION otherwise), and a
texture of an older format refuses the pairs only the new formats take.
`GlTexture::es3_fmt` (index + 1 into `FORMATS`, in what was padding) records
which format a texture holds; the texture's `format`/`type_` are still the
upload's.

## Storage and the back end's mask

Each format lists storage classes, best first. `backend::sampled_texfmts()`
is the mask of classes this build's back end samples
(`PVR_SAMPLED_TEXFMTS`, `GC_SAMPLED_TEXFMTS`, `V3D_SAMPLED_TEXFMTS`); the
texture is stored in the first class of its list that is in the mask. The new
classes are `GLES_TEXFMT_R11G11B10F`, `RGB9E5`, `RGB10A2`, `RGB10A2UI`,
`RGBA8_SNORM`, `RGBA8UI/I`, `RGBA16UI/I`, `RGBA32UI/I`, `RGBA32F` (ids 11..22;
every one four components wide, linear rows, 4/8/16 bytes a texel).

| format | classes, best first |
|---|---|
| `R/RG/RGB/RGBA8_SNORM` | `RGBA8_SNORM`, `RGBA16F` |
| `R11F_G11F_B10F`, `RGB9_E5` | its own packed word, `RGBA16F` (exact) |
| `RGB16F` | `RGBA16F` |
| `RG32F`, `RGB32F` | `RGBA32F`, `RGBA16F` (the same loss `RGBA32F` already has) |
| `RGB10_A2` | `RGB10A2`, `RGBA8` (two bits lost; the V3D samples the class) |
| `RGB10_A2UI`, every integer format | its own class only |

The PowerVR samples every new class but `RGB10A2` (`PVR_SAMPLED_TEXFMTS`; the
TPU formats are Mesa's, `docs/es3/pvr-formats.md` section 2, and the integer
classes are read by `isampler2D`/`usampler2D`), so there each format is
stored in its own class except `RGB10_A2`, which stays in `RGBA8` because a
texture of the `RGB10A2` class is no render target and has no
glGenerateMipmap here. The GC7000 and the V3D masks list only the classes
their samplers already read (the GC7000UL's also lists the 8/16-bit and R32/RG32 integer
classes and `RGB10_A2UI`, its descriptor decodes them): there the other integer
formats are **refused by name** (a log line `glTexImage2D: GL_RGBA16UI is not supported:
this back end samples none of its storage classes` and GL_INVALID_OPERATION,
in glTexImage2D/3D, glTexStorage2D/3D and glCopyTexImage2D); the float,
packed-float and snorm formats land in `RGBA16F` on the GC7000 and are
refused on the V3D (whose TMU binding reads no half float yet); `RGB10_A2`
lands in `RGBA8` except on the V3D, which samples and renders the `RGB10A2`
class (10/10/10/2 surface, field-wise CPU `glGenerateMipmap`;
`docs/notes/v3d-rgb10a2.md`).

**To make a back end sample a class**: make its texture state decode the class
(`GlesTextureBind::format` carries the `GLES_TEXFMT_*`), then add the class to
its mask in `src/backend.rs`. The GL core then stores natively, with no other
change; the tests state the expectation per mask.

## Rules implemented

* Upload conversion on the CPU (`es3_formats::convert_row`): the source texel
  of any table 3.2 pair is decoded to four components (floats, or raw
  integers sign-extended) and encoded into the class; components the format
  does not have read as (0, 0, 0, 1). Packed floats clamp negatives to 0 and
  large values to the largest finite one; `RGB9_E5` follows
  EXT_texture_shared_exponent; halves round to nearest even.
* ES 3.0 3.8.13: an integer texture, and one of 32-bit float texels
  (`RG32F`, `RGB32F`), is complete only with NEAREST magnification and
  NEAREST or NEAREST_MIPMAP_NEAREST minification (`incomplete_reason`,
  `draw.rs`). A sampler object's filters count.
* `glGenerateMipmap` is GL_INVALID_OPERATION for these formats except
  `RGB10_A2` (3.8.14: renderable and filterable only; in its class on the
  V3D, held as `RGBA8` elsewhere).
* `glCopyTexImage2D`/`glCopyTexSubImage2D/3D` into an integer texture is
  GL_INVALID_OPERATION (no integer colour buffer to copy from, ES 3.0 table
  3.16); into the others it converts from the framebuffer's RGBA8.
* A texture of these formats is no render target except `RGB10_A2` (its own
  surface on the V3D, `RGBA8` elsewhere) (`framebuffer.rs` attachment guard).

## Float colour targets

`R16F`, `RG16F`, `RGBA16F`, `R32F`, `RG32F`, `RGBA32F` and `R11F_G11F_B10F`.
Plan #360's decision and evidence: `docs/notes/es3-float-targets.md`.

The rule:

* ES 3.0.6 table 3.13 marks none of them colour-renderable (`RGB16F` is a
  texture-only format there as well).
* `GL_EXT_color_buffer_half_float` adds the 16F ones.
* `GL_EXT_color_buffer_float` adds all seven.
* Without the extension a renderbuffer or texture of such a format is
  framebuffer-incomplete (ES 3.0.6 4.4.4, `FRAMEBUFFER_INCOMPLETE_ATTACHMENT`),
  so `R32F` in particular must be incomplete when `GL_EXT_color_buffer_float`
  is not claimed. The extension texts are cited by name only here.

What yakogl does today (the extension string and the checks are the GL
core's, the same for all three GPUs):

* `GL_EXT_color_buffer_half_float` is listed in the ES 3.0 build
  (`src/gl/state.rs:112`); `GL_EXT_color_buffer_float` is listed nowhere.
* `is_color_format` accepts `R16F`, `RG16F`, `RGB16F`, `RGBA16F` and `R32F`
  (`src/gl/framebuffer.rs:646-650`), so `R32F` is complete today without the
  extension: this is the gap to the rule. `RG32F`, `RGBA32F` and
  `R11F_G11F_B10F` are not in it and are incomplete (correct without the
  extension; with it they would need a surface class first).
* The renderbuffer arm maps `RGBA16F`, `RGB16F` and `RG16F` to the
  `GLES_SURF_RGBA16F` class, `R16F` to `GLES_SURF_R16F` and `R32F` to
  `GLES_SURF_R32F` (`src/gl/framebuffer.rs:743-747`); the texture arm does the
  same by texture format (`src/gl/framebuffer.rs:965-967`), and the
  attachment lookup by internal format (`src/gl/framebuffer.rs:1279-1281`).
  Only these three float classes exist (`src/backend.rs:364-366`).
* `rt_format_drawable` is true for every class that is not sRGB, `RGB10_A2`
  or integer (`src/backend.rs:569-590`), so the three float classes pass it on
  every GPU.

The set each back end draws today, from its class mapping:

| GPU | Drawn float classes | Read from |
| --- | --- | --- |
| V3D 4.2 | `RGBA16F` (16F, 64 bpp), `R16F` (16F, also 64 bpp), `R32F` (32F, 32 bpp) | `src/v3d/state.rs:468-470` |
| PowerVR BXS-4-64 | `RGBA16F` (PBE `F16F16F16F16`), `R16F` (`F16`), `R32F` (`F32`) | `src/pvr/backend.rs:1024-1026` |
| GC7000UL | `RGBA16F` (`A16B16G16R16F`), `R16F`, `R32F` (PE formats) | `src/vivante/backend.rs:337-339` |

`RG16F` and `RGB16F` use the `RGBA16F` class everywhere. None of the three
draws `RG32F`, `RGBA32F` or `R11F_G11F_B10F`.

The plan #360 decision:

* V3D: claims `GL_EXT_color_buffer_float` only under the **planned** Cargo
  feature `color-buffer-float`, off by default; not landed yet.
* PowerVR BXS-4-64: does not claim it.
* GC7000UL: does not claim it.
* report-es2 builds claim neither extension (`tests/es2_report_guard.rs`).

## Compressed formats decoded at upload

The six ES 3.0 ETC2/EAC formats with no block class are held as ordinary
textures of a class the back end samples (plan #354,
`docs/notes/etc2-eac.md`): R11_EAC and SIGNED_R11_EAC as R16F, RG11_EAC and
SIGNED_RG11_EAC as RG16F (RGBA16F class), RGB8_PUNCHTHROUGH_ALPHA1_ETC2 as
RGBA8 and SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 as SRGB8_ALPHA8; `GlTexture::decoded_from`
names the origin. An ES 3.0 sized format of the same class is a different
texture to them (the sub-image rule).

## Not done (left to other items)

* Sampling the new classes and isampler/usampler on the V3D (#1232, #1233).
  The GC7000UL (`src/vivante/**`) takes the 8/16-bit and the R32/RG32 integer
  classes and `RGB10_A2UI` (`GC_SAMPLED_TEXFMTS`); its texture descriptor composes the
  format swizzle with `GlesTextureBind::swizzle` and SAMP_CTRL0's W wrap comes
  from `wrap_r` (`840ea7ec`, `tests/es3_tex_swizzle_gc.rs`,
  `docs/notes/gc-texture-swizzle-wrap.md`). Still out of the mask, so refused:
  `RGBA32F/UI/I`, whose two-plane back end (a second sampler slot, a per-draw
  shader key) and compiler join are in the tree but not offered
  (`docs/notes/gc-128bit-texture-sampling.md`, `gc-128bit-swizzle.md`,
  `tests/es3_wide128_gc.rs`; the flip is pending, item gc-wide128-flip).
  `R32I/UI` and `RG32I/UI` are in the mask (descriptor rows `e40e9418`, flip
  `b5e04b14`): sampled natively, `EXT_R32I` / `EXT_G32R32I`, so the GL core
  stores them natively and no longer refuses them on this back end. The PowerVR's integer cube, 3D and 2D-array samplers
  select since the stacked arms take the sampler's return type
  (`docs/es3/pvr-formats.md` section 10.1); their AM62P dEQP-GLES3
  `texture.format` run is still to do.
* Integer render targets on the V3D, and `RGB10_A2` as its own render target
  on the PowerVR and the GC7000 (the V3D has it; the PowerVR writes the nineteen integer formats since
  `fc658e64`, merged in `a27d69c0`; `docs/es3/pvr-formats.md` section 10).
  On the GC7000UL the 8/16-bit integer targets work (`tests/es3_int_rt_gc.rs`)
  and `R32I/UI`, `RG32I/UI` draw as colour targets on the host back end, as
  renderbuffers and, since `6c5b004f` (`integer_texture_surface` rows), as
  texture attachments (`tests/es3_int32_rt_gc.rs`, `dafc866c`); `RGBA32I/UI` stays refused by
  name: the tiled shadow's 16-byte plane split and join, the plane masks and
  clear words are inert, tested functions (`src/vivante/wide_target.rs`,
  `docs/notes/gc-128bit-render-targets.md`, `gc-128bit-rt-shader.md`; a
  64-bit target beside another: `gc-int64-mixed-targets.md`). The fourth
  render target is host-tested (`tests/es3_mrt4_gc.rs`, `53e6189c`), but
  `DRAW_TARGETS` stays 3 for `gpu-vivante` until a board draws four
  (`docs/notes/gc-mrt4-board-check.md`).
  `glGetInternalformativ`'s texture answers (ES 3.0 knows renderbuffers only;
  the query stays as `b9b2ac00` leaves it):
  `es3_formats::Es3Format::storage_here()` is the question a caller asks.
* sRGB is no longer stored as plain `RGBA8`: `SRGB8`/`SRGB8_ALPHA8` are their
  own class (`GLES_TEXFMT_SRGB8_A8`, `src/backend.rs`) since `3c433ed2` (merged
  in `c4dce9125`), decoded by every sampler (`docs/es3/srgb.md`). The 32-bit-float filterability of `R32F`/`RGBA32F` is not
  modelled (it predates the table and is not identifiable from the storage
  class). `RGB5_A1` from `UNSIGNED_INT_2_10_10_10_REV` takes the 10-bit word
  since `e5c4a180`, and every level of a NPOT 3D/2D-array texture is valid
  since `c40414b9`.
