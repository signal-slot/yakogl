# ES 3.0 sized texture formats in the GL core

Item `sized-formats-gl` of plan #3. What `glTexImage2D/3D`, `glTexStorage2D/3D`,
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
`RGB5_A1`, `R16F`, `RG16F`, `RGBA16F`, `R32F`, `RGBA32F`) behave as before;
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
| `RGB10_A2` | `RGB10A2`, `RGBA8` (two bits lost) |
| `RGB10_A2UI`, every integer format | its own class only |

The masks today list only the classes the samplers already read, so on every
GPU: the integer formats and `RGB10_A2UI` are **refused by name** (a log line
`glTexImage2D: GL_RGBA16UI is not supported: this back end samples none of its
storage classes` and GL_INVALID_OPERATION, in glTexImage2D/3D, glTexStorage2D/3D
and glCopyTexImage2D); the float, packed-float and snorm formats land in
`RGBA16F` on the PowerVR and the GC7000 and are refused on the V3D (whose TMU
binding reads no half float yet); `RGB10_A2` lands in `RGBA8` everywhere.

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
  `RGB10_A2` held as `RGBA8` (3.8.14: renderable and filterable only).
* `glCopyTexImage2D`/`glCopyTexSubImage2D/3D` into an integer texture is
  GL_INVALID_OPERATION (no integer colour buffer to copy from, ES 3.0 table
  3.16); into the others it converts from the framebuffer's RGBA8.
* A texture of these formats is no render target except `RGB10_A2` as
  `RGBA8` (`framebuffer.rs` attachment guard).

## Not done (left to other items)

* Sampling the new classes (`formats-pvr`, `formats-v3d`, `formats-gc`) and
  isampler/usampler in the shader back ends.
* Integer and `RGB10_A2` render targets, `glGetInternalformativ`'s texture
  answers (ES 3.0 knows renderbuffers only; the query stays as `int-rt-gl`
  leaves it): `es3_formats::Es3Format::storage_here()` is the question a
  caller asks.
* `SRGB8`/`SRGB8_ALPHA8` are stored as plain `RGBA8` as before (no decode in
  any sampler); `RGB5_A1` from `UNSIGNED_INT_2_10_10_10_REV` and the
  32-bit-float filterability of `R32F`/`RGBA32F` are not modelled (they
  predate the table and are not identifiable from the storage class).
* Mip levels >= 1 of NPOT 3D/2D-array textures (`tex3d-levels`).
