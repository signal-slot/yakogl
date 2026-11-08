<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): sampling RGBA16F and R16F textures, gl_test cases 44 and 91

Plan #336, item ana-frame (issue #32). gl_test cases 44 `es3hdr` and 91
`hdrblend` are REFUSED at draw time in `tools/cts/log/2026-11-07-t6-step4a/`
(`sweep-1.verdicts.txt:48`, `:95`):

    REFUSED draw: mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet

Both render into an 8x8 `GL_RGBA16F` texture (the colour target is built since
case 54, `docs/notes/mali-frame-formats.md` section 4 and 6) and then
**sample** it, which the back end refuses. This note gives Mesa 26.2.1's
texture, plane and sampler descriptors for a sampled 2D RGBA16F texture (and
RGB16F, RG16F, R16F), dumped on the drm-shim, where src/mali refuses today,
and the change that builds it. 3D textures and their levels are out of scope
(`docs/notes/mali-3d-levels.md`, item frame-3dlevels).

## 1. What the cases sample

- **44 `es3hdr`** (`driver/tools/pvrlab/gl_test.c:6823-6960`): `tex` is
  `glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F, 8, 8, 0, GL_RGBA, GL_HALF_FLOAT,
  NULL)`, NEAREST/NEAREST, CLAMP_TO_EDGE on S and T (`gl_test.c:6871-6875`),
  attached as COLOR_ATTACHMENT0 and drawn into (cleared above 1.0), then
  sampled by `gl_test_es300_hdrsample_fs_src`
  (`driver/tools/pvrlab/gl_test_shaders_es100.h:6948-6958`: `texture(hdr,
  v_xy * 0.5 + 0.5) * 0.25`, highp) into the RGBA8 window.
- **91 `hdrblend`** (`gl_test.c:21621-`): the same 8x8 RGBA16F texture as
  target of two blended draws, sampled by `gl_test_hdrscale_fs_src`
  (`gl_test_shaders_es100.h:614-621`, ES 1.00, mediump, `texture2D(hdr, v_uv)
  * 0.25`); then step 7 uploads a **1x1** RGBA16F texel (0.5, 0.25, 1.0, 1.0)
  (`glTexImage2D(.., GL_RGBA16F, 1, 1, .., GL_HALF_FLOAT, texel)`) and samples
  it the same way.

So the two cases need exactly: a 2D, one-level, linear RGBA16F texture of 8x8
(GPU-written, sampled in place) and of 1x1 (CPU-written; its 8-byte row is
not on 64 bytes, so the back end copies it, `needs_copy`), nearest, clamped.

How the GL core stores the float formats (`src/gl/texture.rs:5852-5856`):
`GL_RGBA16F`, `GL_RGBA32F` (as halves) and `GL_RG16F` go to
`GLES_TEXFMT_RGBA16F`; `GL_R16F` to `GLES_TEXFMT_R16F`; `GL_R32F` to
`GLES_TEXFMT_R32F` (already sampled). `GL_RGB16F` is `F16_ST` = RGBA16F
(`src/gl/es3_formats.rs:81`, `:120`), filled (r, g, b, 1); RG16F (r, g, 0, 1).
Only **two new storage formats** reach the back end: `GLES_TEXFMT_RGBA16F`
(5, 8 bytes a texel) and `GLES_TEXFMT_R16F` (6, 2 bytes), `src/backend.rs:93-94`,
`backend::gles_texfmt_bytes` (`src/backend.rs:315`).

## 2. The Mesa 26.2.1 reference

### Command

The Mesa tree is `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` (`VERSION`
26.2.1, the tree `MESA_PAN_TREE` names, which also holds `build-pan` for
`pan_shc`). The drm-shim build `tools/mali/drm_shim_ref.sh build` makes needs
a tarball; with the tree already extracted, the same configuration was run on
it into a second build directory (the options are the script's, verbatim):

    meson setup $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-shim \
      $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1 -Dbuildtype=debug \
      -Dgallium-drivers=panfrost -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= \
      -Dglx=disabled -Degl=enabled -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled \
      -Dopengl=false -Dglvnd=disabled -Dprefix=$CONNECTED_DEPS_DIR/mesa-pan/inst-shim-26.2.1
    ninja -C $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-shim
    meson install -C $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-shim

(`libpanfrost_noop_drm_shim.so`, `libEGL.so.1`, `libGLESv2.so.2` land in
`inst-shim-26.2.1/lib/x86_64-linux-gnu`; `drm_shim_ref.sh run <work>` finds
them when `<work>/inst` is that prefix.)

`tools/mali/gl_ref.py` gives every `sampler2D` a 2x2 RGBA8 texture and has no
option for another format, so the dumps were taken with a copy of it whose
plain-`sampler2D` branch (`tools/mali/gl_ref.py:470-476`) reads the format
and side from the environment:

```python
F16 = {"RGBA16F": (0x881A, GL_RGBA, 4), "RGB16F": (0x881B, 0x1907, 3),
       "RG16F": (0x822F, 0x8227, 2), "R16F": (0x822D, 0x1903, 1)}
fname = os.environ.get("GL_REF_TEXFMT", "RGBA8")
side = int(os.environ.get("GL_REF_TEXSIDE", "2"))
if fname in F16:
    ifmt, fmt, nc = F16[fname]
    # fp16 0.5, 0.25, 1.0, 1.0 (case 91's uploaded texel)
    halves = [0x3800, 0x3400, 0x3C00, 0x3C00][:nc] * (side * side)
    texels = (C.c_ushort * len(halves))(*halves)
    gl.glTexImage2D(target, 0, ifmt, side, side, 0, fmt, 0x140B, texels)  # GL_HALF_FLOAT
else:
    ...  # the RGBA8 texture as before, side x side
```

(item frame-texfmt turns this into a `--tex-format F --tex-size N` option of
`gl_ref.py`; nothing under `tools/mali/` is changed by this item). The draw is
an ES 3.0 program sampling `uniform sampler2D hdr` (`#version 300 es`: the
half-float sized formats need an ES 3 context), case 91's arithmetic:

```glsl
#version 300 es                       // hdr.vert
in vec4 a_pos; in vec2 a_uv; out vec2 v_uv;
void main() { gl_Position = a_pos; v_uv = a_uv; }

#version 300 es                       // hdr.frag
precision mediump float;
uniform sampler2D hdr; in vec2 v_uv;
layout(location = 0) out vec4 fragColor;
void main() { fragColor = texture(hdr, v_uv) * 0.25; }
```

and the environment is `drm_shim_ref.sh run`'s with two `PAN_MESA_DEBUG`
flags added:

    LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
      PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
      BIFROST_MESA_DEBUG=shaders,shaderdb PAN_MESA_DEBUG=trace,linear,nocrc,dump \
      PANDECODE_DUMP_FILE=$OUT.trace GL_REF_TEXFMT=RGBA16F GL_REF_TEXSIDE=8 \
      python3 -I -u gl_ref_f16.py $INST hdr.vert hdr.frag

- `linear`: src/mali samples the GL core's linear rows (`ClumpOrdering::LINEAR`,
  `src/mali/frame.rs:1444`). Without it Mesa picks U-interleaved for the
  texture: `Clump ordering: Tiled U-Interleaved`, `Texel interleave: true`,
  `Row stride: 0x800`, `Size: 0x800` for the 8x8 RGBA16F (first run, not used).
- `nocrc`: the board dumps src/mali is compared with are taken with it
  (`tools/mali/mesa-board-ref.sh:12`). Without it a texture of 4 bytes a texel
  or less that may be a render target gets a transaction-elimination CRC
  appended to its level (`panfrost_should_checksum`,
  `M:gallium/drivers/panfrost/pan_resource.c:838-853`; added to `size_B` at
  `M:panfrost/lib/pan_layout.c:129-133`): the plane `Size` of the 8x8
  RGBA8, RG16F and R16F textures reads `0x220` instead of `0x200`. RGBA16F
  (8 bytes) never gets one: `0x200` with or without `nocrc`.
- `dump`: pandecode appends the memory of every BO, from which the raw words
  below were read (the descriptor at its address in its BO's hex dump).

Six runs, each rc 0 with `glGetError 0x0` and the renderer `Mali-G610 MC4
(Panfrost)`: RGBA8, RGBA16F, RGB16F, RG16F, R16F at 8x8, and RGBA16F at 1x1.
The two traces of RGBA8 and RGBA16F at 8x8 differ only in the texture's
format word, its plane's clump format and size, the texel bytes and the host
pointers pandecode prints: **the three shader programs are byte-identical**
(the fragment program does not depend on the sampled format), and so are the
sampler, the resource tables and every other descriptor.

Citations: `M:<path>:<line>` under `src/` of that Mesa 26.2.1 tree, ours
`<path>:<line>` at 0406388a.

### Mesa's format rows

| GL internal format | pipe format st/mesa picks | `Format (v7)` (pan_format.c) | swizzle | clump (`pan_clump_format`, by block size) |
|---|---|---|---|---|
| `GL_RGBA16F` | `R16G16B16A16_FLOAT` | `RGBA16F RGBA` (`M:panfrost/lib/pan_format.c:548`) | RGBA 0x688 | RAW64 (`M:panfrost/lib/pan_texture.c:303-316`) |
| `GL_RGB16F` | `R16G16B16X16_FLOAT` (RGB16F has no render support, `:488` `V_____`) | `RGBA16F RGB1` (`:551`) | RGB1 0xA88 | RAW64 |
| `GL_RG16F` | `R16G16_FLOAT` | `RG16F RGB1` (`:542`, RG01) | RG01 0xB08 | RAW32 |
| `GL_R16F` | `R16_FLOAT` | `R16F RGB1` (`:538`, R001) | R001 0xB20 | RAW16 |

(`Format (v7)` is `MALI_PACK_FMT`: `Format << 12 | order`, the order `RGB1` =
0x10, packed at bits 10..31 of word 0. genxml: `Format::RGBA16F` 0xdf,
`RG16F` 0xcf, `R16F` 0xc7, `RGBA8_UNORM` 0xbb, `src/mali/genxml/v10.rs:383-405`;
`ClumpFormat::RAW16/32/64` 1/2/3, `:4681-4683`.)

### The descriptor words (`nocrc`, 8x8 unless stated)

32-bit words, little-endian, word 0 first. The addresses are the shim's
(`Surfaces` 0x7fffffffd080, plane `Pointer` 0x7ffffffe6000); a test builds its
own and compares them by cause, as `src/mali/frame_tex_tests.rs` does.

| texture | Texture (32 bytes) | Generic Plane (32 bytes) |
|---|---|---|
| RGBA8 (baseline) | `2EC00022 00070007 00000688 00000000 FFFFD080 00007FFF 00000000 00000000` | `0200021A 00000200 FFFE6000 00007FFF 00000040 00000000 00000000 00070007` |
| **RGBA16F** | `37C00022 00070007 00000688 00000000 FFFFD080 00007FFF 00000000 00000000` | `0300021A 00000200 FFFE6000 00007FFF 00000040 00000000 00000000 00070007` |
| **RGBA16F 1x1** | `37C00022 00000000 00000688 00000000 FFFFD080 00007FFF 00000000 00000000` | `0300021A 00000040 FFFE6000 00007FFF 00000040 00000000 00000000 00000000` |
| **R16F** | `31C04022 00070007 00000B20 00000000 FFFFD080 00007FFF 00000000 00000000` | `0100021A 00000200 FFFE6000 00007FFF 00000040 00000000 00000000 00070007` |
| RGB16F (with CRC run; RAW64, no CRC either way) | `37C04022 00070007 00000A88 00000000 ...` | `0300021A 00000200 ...` |
| RG16F (with CRC run) | `33C04022 00070007 00000B08 00000000 ...` | `0200021A 00000220 ...` (0x200 with `nocrc`, as R16F) |

Sampler, all six runs (nearest, clamp to edge on S and T, GL's default
REPEAT on R): `06A99901 00000000 00000000 00000000 00000000 00000000 00000000 00000000`.

Decoded (pandecode of the RGBA16F 8x8 run): Texture `Type: Texture`,
`Dimension: 2D`, `Format (v7): RGBA16F RGBA`, `Width: 8`, `Height: 8`,
`Swizzle: 1672 (RGBA)`, `Texel interleave: false`, `Levels: 1`, `Minimum
level: 0`, `Minimum LOD: 0`, `Maximum LOD: 0`, `Sample count: 1`, `Array size:
1`, `Depth: 1`; Plane `Plane type: Generic`, `Clump ordering: Linear`, `Clump
format: RAW64`, `Size: 0x200`, `Row stride: 0x40`, `Slice stride: 0`, `Width:
8`, `Height: 8`. Row stride 64 = 8 texels x 8 bytes (Mesa's linear rows are on
64 bytes, `M:panfrost/lib/pan_mod.c:759-771`); the 1x1 texture's row is 8
bytes padded to 64, `Size` 0x40 (one row). The plane `Size` is `row_stride *
height` (`surface_stride_B`, `M:panfrost/lib/pan_mod.c:768-774`, emitted by
`get_linear_or_u_tiled_plane_props`, `M:panfrost/lib/pan_texture.c:389-422`),
which is already src/mali's rule (`SampledTexture::plane`,
`src/mali/frame.rs:1439-1454`).

So word by word, an RGBA16F texture differs from the RGBA8 one src/mali
already builds in **two fields only**: Texture word 0's format (`0x37C00022`
vs `0x2EC00022`, `Format::RGBA16F` for `RGBA8_UNORM`), and Plane word 0's clump
format (`RAW64`, `0x03` in the top byte, vs `RAW32`). R16F adds its swizzle
(`0xB20`, R001) and order (RGB1, `0x31C04022`) and clump RAW16.

## 3. Where src/mali refuses today

1. `texel_format` (`src/mali/backend.rs:1733-1780`) knows RGBA8, sRGB8_A8,
   R32F and the three ETC2; `GLES_TEXFMT_RGBA16F` and `GLES_TEXFMT_R16F` fall
   to `_ => return None`.
2. `texture_of` then refuses with `NOT_BUILT_TEXTURE_FORMAT`
   (`src/mali/backend.rs:209-210`), from its `checks` table
   (`src/mali/backend.rs:2022-2030`, the `f.is_none()` row at `:2024`; also
   `:2017` for a 3D texture and `:2034`). This is the sweep's string.
3. Behind it, every texel-size computation assumes **4 bytes a texel** for a
   format that is not a block format (`block_bytes == 0`):
   - `level_at`: `pitch = stride_texels * 4`, `row_bytes = w * 4`
     (`src/mali/backend.rs:1815-1817`), used for the in-place levels
     (`:2037-2041`), `needs_copy` (`:1878-1884`), `storage_end`
     (`:1886-1895`) and the copy (`:2042-2066`);
   - `volume_of` / `storage_end` for a 3D grid (`:1890`, `:2143-2146`; out of
     scope here, but the same `* 4`);
   - `check_draw`'s row check, `row = 4 * t.extent(l).0` for a non-block
     texture (`src/mali/frame.rs:2415`), and `SampledTexture`'s docs ("RAW32
     for 4-byte texels", `src/mali/frame.rs:1300-1306`).
   With `stride_texels * 4` an 8x8 RGBA16F's pitch reads 32 bytes instead of
   64: that is not on 64, so `needs_copy` would copy half of each row.
4. `sampled` (`src/mali/backend.rs:2083-2127`) takes `f.swizzle` only for a
   block format; any other gets RGBA/RGB1/BGRA/BGR1/RRR1 from the binding's
   flags. R16F needs Mesa's R001 (`0xB20`) to match byte for byte.
5. Not a refusal: `SAMPLED_TEXFMTS` is 0 (`src/mali/backend.rs:93-95`); the
   GL core still stores RGBA16F and R16F (the "float trio" is sampled where
   the back end says nothing, `src/backend.rs:295-299`), so no GL-side change
   is needed for cases 44 and 91.

The fragment shader needs nothing: the dump's programs are byte-identical for
the RGBA8 and RGBA16F textures, and the compiler's verdict for both cases'
programs is already `ok` (they are refused at draw time only).

## 4. The fix (item frame-texfmt)

All in `src/mali/backend.rs`, plus one line of `src/mali/frame.rs` (owned by
frame-3dlevels while it runs: take it after that item merges).

1. **`TexelFormat` gains the bytes of a texel** (say `texel_bytes: u32`, 4 for
   RGBA8/sRGB8_A8/R32F, 0 or unused for a block format), or derive it with
   `backend::gles_texfmt_bytes(t.format)` (`src/backend.rs:315`).
2. **`texel_format` gains two rows**, from the table of section 2:
   ```text
   GLES_TEXFMT_RGBA16F => format Format::RGBA16F << 12 | RGBA, clump RAW64,
                          swizzle SWIZZLE_RGBA, block_bytes 0, texel_bytes 8
   GLES_TEXFMT_R16F    => format Format::R16F << 12 | RGB1,   clump RAW16,
                          swizzle swizzle4(0, 4, 4, 5) (R001 = 0xB20),
                          block_bytes 0, texel_bytes 2
   ```
   RGB16F and RG16F need no row: the GL core stores them as RGBA16F with the
   missing channels filled, so they are sampled through the RGBA16F row and
   identity swizzle. That differs from Mesa's own words for them (`RGBA16F
   RGB1` / swizzle 0xA88, `RG16F RGB1` / 0xB08 RAW32) but reads the same
   values; a test pins them as RGBA16F, not against Mesa's RGB16F/RG16F rows.
3. **`level_at`, `needs_copy`, `storage_end` and the copy use
   `texel_bytes`** instead of 4 (`src/mali/backend.rs:1816-1817`). The copy
   then pads an 8-byte (1x1) or 16-byte row to 64, which gives the 1x1 case's
   `Row stride 0x40`, `Size 0x40` exactly. **Check** that the GL core's
   `stride_texels` (`level_pitch`, `src/gl/context.rs:264-289`) is in texels
   of the storage format for RGBA16F (`convert_row_float` writes 8 bytes a
   texel, `src/gl/texture.rs:220-238`), so `stride_texels * 8` is the byte
   pitch.
4. **`sampled` uses `f.swizzle` for R16F** (or for every format whose
   `texel_format` swizzle is not the identity), composed with the binding's
   flags the way the block formats' are; RGBA16F keeps the identity (the
   flags never apply to it: no BGRA float storage, `no_alpha` is for RGBA8
   storage of alpha-less formats).
5. **`check_draw`'s row check** (`src/mali/frame.rs:2415`) takes the texel
   bytes from the texture (a `texel_bytes` on `SampledTexture`, or from its
   clump: RAW16 2, RAW32 4, RAW64 8) instead of 4.
6. Remove nothing else: `NOT_BUILT_TEXTURE_FORMAT` stays for the formats
   still refused (L8/A8/LA8 narrow storage, RGB565, the integers, RGBA32F
   storage, R11G11B10F/RGB9E5 storage, ...), and its text grows "RGBA16F,
   R16F". `backend_tests.rs:1129` (`"L8 storage"`) keeps its refusal; add
   RGBA16F and R16F build rows beside it.

A texture that was a render target (case 44's and 91's 8x8) is sampled in
place: its rows were written by the RAW64 writeback of an opaque frame (or the
RGBA16F tile buffer of a blended one, `mali-frame-formats.md` section 6), both
of which store four fp16 halves a pixel on the target's row pitch; the board's
case 54 read back (1, 0, 0, 1) from such a face. The order of the render and
the sampling is the existing `target_kick` path (`texture_ready`,
`src/mali/backend.rs:1897-1915`), unchanged.

## 5. Tests the fix brings

- `tools/mali/testdata/frame_rgba16f_tex.trace` (and `_r16f`, `_1x1`): the
  `nocrc` traces of section 2, from `gl_ref.py --tex-format RGBA16F
  --tex-size 8` (and `R16F 8`, `RGBA16F 1`) via `drm_shim_ref.sh run`, with
  `PAN_MESA_DEBUG=trace,linear,nocrc,dump` (the `run` form sets only `trace`
  today: the option or an environment pass-through is frame-texfmt's change).
  Or, smaller, `frame_rgba16f_tex.expect.txt` with the words of the table
  above and the command.
- A host test beside `frame_tex_tests.rs` (or in it, after frame-3dlevels):
  build the draw with an RGBA16F (R16F) `GlesTextureBind` at the trace's
  addresses and row stride, and compare the texture, plane and sampler words
  with the table, the addresses as the only listed cause; plus the 1x1 copy
  (`Row stride` 0x40, `Size` 0x40, the texel's 8 bytes at the copy's start).
- `backend_tests.rs`: RGBA16F and R16F move from refused to built; a
  `stride_texels` of 1 with 8-byte texels takes the copy path.
- After the board sweep, cases 44 and 91 should move from REFUSED to PASS or
  FAIL; neither has another refusal known after this one (fc-next's
  forecast), and case 91's blends are already built.

## 6. Open points

- The words are Mesa **26.2.1**'s on the shim; no board (Mesa 25.0.7) dump of
  a sampled RGBA16F texture exists. The render-target formats differed
  between the two (RAW64, `mali-frame-formats.md` section 6); the texture
  format word is the table row of both versions, so no difference is
  expected, but only the board sweep shows it.
- R32F is sampled today with the identity swizzle over `R32F RGB1`
  (`src/mali/backend.rs:1752-1757`), where Mesa's table row for R32_FLOAT is
  R001 like R16F's (`M:panfrost/lib/pan_format.c:451`, `FMT(R32_FLOAT, R32F, R001, ..)`
  family). Both read (r, 0, 0, 1); whether to align R32F's swizzle is for the
  item that next touches it, not this one.
- `SAMPLED_TEXFMTS` (`src/mali/backend.rs:95`) could then name RGBA16F and
  R16F; that changes which storage class R11G11B10F, RGB9E5 and the snorm
  formats fall back to (`src/gl/es3_formats.rs:80-126`) and is a separate
  decision with its own tests (`src/gl/es3_format_tests.rs:230`).
