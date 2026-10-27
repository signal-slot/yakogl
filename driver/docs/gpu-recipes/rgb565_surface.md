<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# RGB565 colour surfaces (PBE pack + TPU read-back)

A 16-bit R5G6B5 window (DRM `RGB565`, red in bits 15..11; the SafeUI
application surface) needs two encodings on top of the RGBA8888 path.
Verified by `gl_test` case 21 (on the Linux harness, since removed, and on Zephyr, 2026-09-14).

## PBE (writing the surface)

- `PBESTATE_STATE_WORD1.packmode` (bits 7..12, `pbestate.xml`) = `R5G6B5` = 0x14;
  the source stays `8_PER_CHANNEL` + `norm` (`EOT_EMIT_WORD1_FLAGS`): the fragment
  shader still emits unorm8 channels, the PBE packs them. The pack mode puts channel
  0 in the **low** 5 bits (a red-only frame read back 0x001F with the identity
  swizzle), so an RGB565 target (red high, as the DSS `RGB565` plane and
  `UI_RGB565()` expect) uses the swapped channel order, the same `PBE_SWIZZLE_BGRA`
  as XRGB8888.
- `REG_WORD0.linestride` stays in pixel pairs (`(w - 1) / 2`) whatever the pixel size
  (an 8-byte-unit reading of it halved the RGB565 pitch: rows 32..63 of a 64x64
  window were never written). `REG_WORD1` clips are pixels.
- The clear colour of the background object is still packed RGBA8 (no byte swap:
  the XRGB8888 swap only applies to the 32-bit scan-out format).

## TPU (reading the surface: the load quad and EGLImages)

- `IMAGE_WORD0.texformat` = `R5G6B5` = 5 (`texstate.xml`), `textype` STRIDE, channels
  0 and 2 swapped (the TPU's channel 0 is the low 5 bits too) and alpha `SRC_ONE`
  (`swiz3 = 4`, as for ETC RGB): `TEX_IMAGE_WORD0_RGB565 = (5 << 27) | 0x8884`.
- `STRIDE_IMAGE_WORD1.stride` is in texels, so the RGBA8 value is reused; the layer
  size is `stride * height * 2`.
- Image meta `PCK_INFO` = `PVR_PCK_FORMAT_INVALID | UNORM` = 0x11f (pco has no pack
  format for 565; the TPU already returns normalised floats), the same value the
  ETC formats use.

## YakoGL

`enum pvr_surf_format` (`pvr_backend.h`) carries the surface format from EGL
(`GLES_NATIVE_FORMAT_RGB565`, config 3: `EGL_BUFFER_SIZE` 16, 5/6/5/0) through the
context's default surface into `pvr_draw.target_format`; `glReadPixels` expands the
16-bit pixels (`r5 << 3 | r5 >> 2`, ...). A 32-bit config refuses an RGB565 window
(`EGL_BAD_MATCH`) and vice versa. FBO textures stay RGBA8888.
