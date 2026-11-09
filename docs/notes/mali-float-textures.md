<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): sampling RGBA16F and R16F textures, gl_test cases 44 and 91

Plan #336, item ana-frame (issue #32); re-checked at `04ac35d3`, after
frame-3dlevels, by item texfmt-check (file:line references, the
`stride_texels` check, the reference command and the section 2 words). gl_test cases 44 `es3hdr` and 91
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
(`src/gl/es3_formats.rs:81`, `:128`), filled (r, g, b, 1); RG16F (r, g, 0, 1).
Only **two new storage formats** reach the back end: `GLES_TEXFMT_RGBA16F`
(5, 8 bytes a texel) and `GLES_TEXFMT_R16F` (6, 2 bytes), `src/backend.rs:93-94`,
`backend::gles_texfmt_bytes` (`src/backend.rs:315`).

## 2. The Mesa 26.2.1 reference

### Command

**Rerun at `04ac35d3` (2026-11-09, plan item texfmt-check): every word below
came out the same.** The first run (item ana-frame) was at `0406388a`.

The Mesa tree is `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` (`VERSION`
26.2.1, the tree `MESA_PAN_TREE` names, which also holds `build-pan` for
`pan_shc`). The drm-shim build `tools/mali/drm_shim_ref.sh build` makes needs
a tarball; with the tree already extracted, the same configuration is run on
it into a second build directory (the options are the script's, verbatim;
`LLVM_DIR` is the script's variable, default `/usr/lib/llvm/23`, which this
host does not have: its LLVM is 21.1.8 in `/usr/lib/llvm-21`, the one
`build-pan` was configured with):

    export PATH=/usr/lib/llvm-21/bin:$PATH PKG_CONFIG_PATH=/usr/lib/llvm-21/lib/pkgconfig
    meson setup $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-shim \
      $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1 -Dbuildtype=debug \
      -Dgallium-drivers=panfrost -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= \
      -Dglx=disabled -Degl=enabled -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled \
      -Dopengl=false -Dglvnd=disabled -Dprefix=$CONNECTED_DEPS_DIR/mesa-pan/inst-shim-26.2.1
    ninja -C $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-shim install

(1450 ninja steps from scratch; it exists in the deps directory now, built on
2026-11-09, and is reused. `libpanfrost_noop_drm_shim.so`, `libEGL.so.1`,
`libGLESv2.so.2` land in `inst-shim-26.2.1/lib/x86_64-linux-gnu`;
`drm_shim_ref.sh run <work>` finds them when `<work>/inst` is that prefix, e.g.
`ln -s ../inst-shim-26.2.1 <work>/inst`.)

**`PAN_MESA_DEBUG` has no pass-through today, and no other documented way
takes these flags on the shim.** `drm_shim_ref.sh:74` runs the python with an
inline `PAN_MESA_DEBUG=trace` in front of the command, and an inline
assignment beats the caller's exported variable. Tested on the unmodified
script with `PAN_MESA_DEBUG=trace,linear,nocrc,dump` exported: the trace is
the `trace`-only one (`Clump ordering: Tiled U-Interleaved`, `Row stride:
0x400`, `Size: 0x420`, 994 lines, no BO hex dumps). `mesa-board-ref.sh:12`
sets the same flags, but on the board (`BOARD_HOST`), not the shim. So
frame-texfmt makes **two changes under `tools/mali/`**:

1. `tools/mali/drm_shim_ref.sh:74`: `PAN_MESA_DEBUG=trace` becomes
   `PAN_MESA_DEBUG="${DRM_SHIM_REF_PAN_DEBUG:-trace}"` (tested form; the
   caller's value replaces the whole string, so it must contain `trace`), and
   the usage header (`:9-18`) names `DRM_SHIM_REF_PAN_DEBUG`. Nothing else in
   the script changes; an unset variable keeps every existing trace byte for
   byte (the committed `tools/mali/testdata` traces are taken with `trace`).
2. `tools/mali/gl_ref.py`: `--tex-format F --tex-size N` (below) in `main()`'s
   `option` list (`:263-275`), the doc comment (`:8-12`) and the
   plain-`sampler2D` branch (`:470-476`).

`tools/mali/gl_ref.py` gives every `sampler2D` a 2x2 RGBA8 texture and has no
option for another format, so the dumps were taken with a throwaway copy of
it (with `drm_shim_ref.sh` copied beside it with change 1; neither is
committed) whose plain-`sampler2D` branch (`tools/mali/gl_ref.py:470-476`)
reads the format and side from the environment:

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

(`--tex-format F --tex-size N` is that with `F` and `N` from the command line
instead of `GL_REF_TEXFMT` and `GL_REF_TEXSIDE`.) The draw is
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

The command of each run (`<work>/inst` the symlink above; with frame-texfmt's
two changes the throwaway env variables are the options):

    DRM_SHIM_REF_PAN_DEBUG=trace,linear,nocrc,dump \
      tools/mali/drm_shim_ref.sh run <work> hdr.vert hdr.frag <out> \
      --tex-format RGBA16F --tex-size 8       # then R16F 8, RGBA16F 1 (RGB16F, RG16F, RGBA8 8)

which runs, inside the script,

    LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
      PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
      BIFROST_MESA_DEBUG=shaders,shaderdb PAN_MESA_DEBUG=trace,linear,nocrc,dump \
      PANDECODE_DUMP_FILE=$OUT.trace python3 -u gl_ref.py $INST hdr.vert hdr.frag \
      --tex-format RGBA16F --tex-size 8

- `linear`: src/mali samples the GL core's linear rows (`ClumpOrdering::LINEAR`,
  `src/mali/frame.rs:1444`). Without it (rerun, `trace,nocrc,dump`) Mesa picks
  U-interleaved for the texture: `Clump ordering: Tiled U-Interleaved`,
  `Texel interleave: true`, `Row stride: 0x800`, `Size: 0x800` for the 8x8
  RGBA16F; Texture word 2 reads `00001688` and Plane `0300011A 00000800 ..
  00000800`.
- `nocrc`: the board dumps src/mali is compared with are taken with it
  (`tools/mali/mesa-board-ref.sh:12`). Without it (rerun, `trace,linear,dump`)
  a texture of 4 bytes a texel or less that may be a render target gets a
  transaction-elimination CRC appended to its level
  (`panfrost_should_checksum`,
  `M:gallium/drivers/panfrost/pan_resource.c:838-853`; added to `size_B` at
  `M:panfrost/lib/pan_layout.c:129-133`): the plane `Size` of the 8x8
  RGBA8, RG16F and R16F textures reads `0x220` instead of `0x200`. RGBA16F
  (8 bytes) never gets one: `0x200` with or without `nocrc`. RGB16F (RAW64)
  is also 8 bytes (`0x200` with `nocrc`; not rerun without it).
- `dump`: pandecode appends the memory of every BO (`Buffer: memory_<gpu>
  gpu <gpu>` and a hex dump), from which the raw words below were read (the
  Plane at the `Surfaces` address `7fffffffd080` in the BO at
  `7fffffffd000`; the Texture is the 32-byte entry whose words 4 and 5 are
  that address).

Rerun: six runs with `trace,linear,nocrc,dump`: RGBA8, RGBA16F, RGB16F,
RG16F, R16F at 8x8, RGBA16F at 1x1 (and five more with a flag dropped, above),
each rc 0 with `glGetError 0x0` and the renderer `Mali-G610 MC4
(Panfrost)`. The traces of RGBA8 and RGBA16F at 8x8 differ in **two lines** of
pandecode's text, `Format (v7): RGBA8 UNORM RGBA` / `RGBA16F RGBA` and `Clump
format: RAW32` / `RAW64` (plus the texel bytes and the host pointers in the
dump): **the three shader programs are byte-identical** (the instruction
lines of the two traces `diff` clean: the fragment program does not depend on
the sampled format), and so are the sampler, the resource tables and every
other descriptor.

Citations: `M:<path>:<line>` under `src/` of that Mesa 26.2.1 tree, ours
`<path>:<line>` at 04ac35d3.

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
| RGB16F | `37C04022 00070007 00000A88 00000000 FFFFD080 00007FFF 00000000 00000000` | `0300021A 00000200 FFFE6000 00007FFF 00000040 00000000 00000000 00070007` |
| RG16F | `33C04022 00070007 00000B08 00000000 FFFFD080 00007FFF 00000000 00000000` | `0200021A 00000200 FFFE6000 00007FFF 00000040 00000000 00000000 00070007` |

Sampler, all six `nocrc` runs (nearest, clamp to edge on S and T, GL's default
REPEAT on R): `06A99901 00000000 00000000 00000000 00000000 00000000 00000000 00000000`.

Decoded (pandecode of the RGBA16F 8x8 run): Texture `Type: Texture`,
`Dimension: 2D`, `Format (v7): RGBA16F RGBA`, `Width: 8`, `Height: 8`,
`Swizzle: 1672 (RGBA)`, `Texel interleave: false`, `Levels: 1`, `Minimum
level: 0`, `Minimum LOD: 0`, `Maximum LOD: 0`, `Sample count: 1`, `Array size:
1`, `Depth: 1`; Plane `Plane type: Generic`, `Clump ordering: Linear`, `Clump
format: RAW64`, `Size: 0x200`, `Row stride: 0x40`, `Slice stride: 0`, `Width:
8`, `Height: 8`. Row stride 64 = 8 texels x 8 bytes (Mesa's linear rows are on
64 bytes, `M:panfrost/lib/pan_mod.c:759-771`); the 1x1 texture's row is 8
bytes padded to 64, `Size` 0x40 (one row): **rerun, the 1x1 words hold**
(`Row stride: 0x40`, `Size: 0x40`, `Width: 1`, `Height: 1`, Plane words 1
and 4 `00000040`, Texture word 1 `00000000`). The plane `Size` is `row_stride *
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

(Line numbers of the tip `04ac35d3`, after frame-3dlevels; the note's first
numbers were from before it.)

1. `texel_format` (`src/mali/backend.rs:1738-1785`, the `TexelFormat` struct
   at `:1725`) knows RGBA8, sRGB8_A8, R32F (`:1757`) and the three ETC2;
   `GLES_TEXFMT_RGBA16F` and `GLES_TEXFMT_R16F` fall to `_ => return None`
   (`:1783`).
2. `texture_of` (`:2014`) then refuses with `NOT_BUILT_TEXTURE_FORMAT`
   (defined `:214-215`), from its `checks` table (`:2068-2073`, the
   `f.is_none()` row at `:2070`; the type row at `:2069`), and once more at
   `:2080` (`f.ok_or`). A 3D texture no longer reaches the table: since
   frame-3dlevels `volume` (`:2061`) takes it to `volume_of` (`:2183`) for
   any non-block format. This is the sweep's string.
3. Behind it, every texel-size computation assumes **4 bytes a texel** for a
   format that is not a block format (`block_bytes == 0`):
   - `level_at` (`:1812-1837`): `pitch = stride_texels * 4`, `row_bytes = w * 4`
     (`:1821-1822`), used for the in-place levels (`:2083-2088`),
     `needs_copy` (`:1922-1929`), `storage_end` (`:1931-1941`, the 2D branch
     `:1939-1940`) and the copy (`:2089-2113`);
   - **the 3D path, which frame-3dlevels added and which this note's first
     text did not know**: `grid_level_at` (`:1896`), `storage_end`'s volume
     branch (`:1937`) and `volume_of` (`:2197`, `:2202`, `:2213`) all have
     `* 4`, and `volume_of` is entered for **any** non-block format
     (`:2061`). The moment `texel_format` returns RGBA16F/R16F, a 3D RGBA16F
     texture would be built as 4-byte texels (wrong pitch and row, no
     refusal). frame-texfmt must either use `texel_bytes` there too or keep
     3D refused for a texel of other than 4 bytes (`volume` false for it, so
     the `checks` table's type row refuses it: `NOT_BUILT_TEXTURE_TYPE`; the
     3D cases 48 and 206 are RGBA8 and unaffected). Add a `backend_tests.rs`
     row either way (`volume(..)` beside `:1128`);
   - `check_draw`'s row check, `row = 4 * t.extent(l).0` for a non-block
     texture (`src/mali/frame.rs:2415`), and `SampledTexture`'s docs ("RAW32
     for 4-byte texels", `src/mali/frame.rs:1300-1301`; the struct is at
     `:1268`).
   With `stride_texels * 4` an 8x8 RGBA16F's pitch reads 32 bytes instead of
   64 (see the check in section 4 step 3): that is not on 64, so `needs_copy`
   would copy half of each row.
4. `sampled` (`src/mali/backend.rs:2129-2181`) takes `f.swizzle` only for a
   block format (the `match` at `:2141`); any other gets
   RGBA/RGB1/BGRA/BGR1/RRR1 from the binding's flags. R16F needs Mesa's R001
   (`0xB20`) to match byte for byte.
5. Not a refusal: `SAMPLED_TEXFMTS` is 0 (`src/mali/backend.rs:96`, doc
   `:93-95`; consumed by `backend::sampled_texfmts`, `src/backend.rs:307`);
   the GL core still stores RGBA16F and R16F (the "float trio" is sampled
   where the back end says nothing, `src/backend.rs:295-299`), so no GL-side
   change is needed for cases 44 and 91.

The fragment shader needs nothing: the dump's programs are byte-identical for
the RGBA8 and RGBA16F textures, and the compiler's verdict for both cases'
programs is already `ok` (they are refused at draw time only).

## 4. The fix (item frame-texfmt)

All in `src/mali/backend.rs`, plus one line of `src/mali/frame.rs`
(frame-3dlevels has merged; nothing holds the file for this note).

1. **`TexelFormat` gains the bytes of a texel** (say `texel_bytes: u32`, 4 for
   RGBA8/sRGB8_A8/R32F, 0 or unused for a block format), or derive it with
   `backend::gles_texfmt_bytes(t.format)` (`src/backend.rs:315`; 8 for
   RGBA16F `:318`, 2 for R16F `:325`).
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
   `texel_bytes`** instead of 4 (`src/mali/backend.rs:1821-1822`, and the 3D
   path's `:1896`, `:1937`, `:2197`, `:2202`, `:2213` or a refusal, section 3
   step 3). The copy then pads an 8-byte (1x1) or 16-byte row to 64, which
   gives the 1x1 case's `Row stride 0x40`, `Size 0x40` exactly (rerun, section
   2).

   **Check, settled: yes.** The GL core's `stride_texels` is in texels of the
   storage format, so `stride_texels * 8` is the byte pitch of RGBA16F and
   `* 2` of R16F. Evidence, all at the tip: (a) the 2D bind sets
   `stride_texels = t.level_pitch(tw)` (`src/gl/draw.rs:4338`; the other
   binds `:751`, `:4379`, `:4406`, `:4441`) and the same draw computes the
   storage bytes as `level_pitch(tw) * rows * gles_texfmt_bytes(t.texfmt)`
   (`:4323-4326`, and `:752` for the 1-level bind): the pitch is multiplied by
   the format's bytes, so it is in texels of that format; (b) `level_pitch`
   (`src/gl/context.rs:264-276`, `level_pitch_at` `:282-288`) is documented
   "in texels" and returns `chain_pitch` or `tex_pitch(lw)` (width rounded up
   to even, `:206-208`); the back end's 64-byte row rule is applied in texels
   of the format through `linear_pitch(w, bpp)` / `row_aligned(pitch, unit,
   align)` (`src/gl/context.rs:214-224`, `:230-232`; the chain's use
   `src/gl/texture.rs:1697`, with `bpp = gles_texfmt_bytes(texfmt)` from
   `storage_layout`, `:1822`): `row_aligned(8, 8, 64)` is 8 texels = 64 bytes,
   `row_aligned(tex_pitch(1) = 2, 8, 64)` is 8 texels, `row_aligned(8, 2, 64)` is 32 texels;
   (c) `convert_row_float` (`src/gl/texture.rs:220-238`) writes the texel
   `x` at byte `x * 8` (RGBA16F) or `x * 2` (R16F) of the row, and its caller
   puts row `y` at `((y + j) * pitch + x) * bpp` (`:2241`, call `:2244`); (d) the other readers do the
   same, e.g. the framebuffer attach `pitch * gles_texfmt_bytes` (`src/gl/
   framebuffer.rs:566`) and `egl.rs:2685`. One consequence for the test the
   section 5 asks for: for an 8-byte texel `row_aligned` makes the pitch of a
   1x1 level 8 texels, not 1, so `stride_texels` is 8 for the 1x1 case's GL
   core and the copy is taken only when an imported image's pitch is not on 64
   bytes (`needs_copy`, `:1922`). A `stride_texels` of 1 with 8-byte texels
   is a hand-made bind (a test), as the existing `needs_copy` rows are.
4. **`sampled` uses `f.swizzle` for R16F** (or for every format whose
   `texel_format` swizzle is not the identity), composed with the binding's
   flags the way the block formats' are; RGBA16F keeps the identity (the
   flags never apply to it: no BGRA float storage, `no_alpha` is for RGBA8
   storage of alpha-less formats).
5. **`check_draw`'s row check** (`src/mali/frame.rs:2415`) takes the texel
   bytes from the texture (a `texel_bytes` on `SampledTexture`, or from its
   clump: RAW16 2, RAW32 4, RAW64 8) instead of 4.
6. Remove nothing else: `NOT_BUILT_TEXTURE_FORMAT` (`:214-215`) stays for the
   formats still refused (L8/A8/LA8 narrow storage, RGB565, the integers,
   RGBA32F storage, R11G11B10F/RGB9E5 storage, ...), and its text grows
   "RGBA16F, R16F". `backend_tests.rs:1136` (`"L8 storage"`) keeps its
   refusal; add RGBA16F and R16F build rows beside it (`:1139` `"R32F
   storage"` is the model).

A texture that was a render target (case 44's and 91's 8x8) is sampled in
place: its rows were written by the RAW64 writeback of an opaque frame (or the
RGBA16F tile buffer of a blended one, `mali-frame-formats.md` section 6), both
of which store four fp16 halves a pixel on the target's row pitch; the board's
case 54 read back (1, 0, 0, 1) from such a face. The order of the render and
the sampling is the existing `target_kick` path (`texture_ready`,
`src/mali/backend.rs:1947-1953`, `texture_ready_one` `:1954`), unchanged;
`texture_ready_one` copies `storage_end(t).unwrap_or(0).max(t.bytes)` (`:1965`).

## 5. Tests the fix brings

- `tools/mali/testdata/frame_rgba16f_tex.trace` (and `_r16f`, `_1x1`): the
  `nocrc` traces of section 2, from `DRM_SHIM_REF_PAN_DEBUG=trace,linear,nocrc,dump
  tools/mali/drm_shim_ref.sh run <work> hdr.vert hdr.frag <out> --tex-format
  RGBA16F --tex-size 8` (and `R16F 8`, `RGBA16F 1`), with the two changes of
  section 2 (`drm_shim_ref.sh:74` and `gl_ref.py`'s options) made by
  frame-texfmt in the same commit. Or, smaller,
  `frame_rgba16f_tex.expect.txt` with the words of the table above and the
  command. The `dump` traces are about 1350 lines each (the BO hex dumps);
  the words are what a test needs.
- A host test beside `frame_tex_tests.rs` (or in it, after frame-3dlevels):
  build the draw with an RGBA16F (R16F) `GlesTextureBind` at the trace's
  addresses and row stride, and compare the texture, plane and sampler words
  with the table, the addresses as the only listed cause; plus the 1x1 copy
  (`Row stride` 0x40, `Size` 0x40, the texel's 8 bytes at the copy's start).
- `backend_tests.rs`: RGBA16F and R16F move from refused to built (beside
  `:1136`); a hand-made `stride_texels` of 1 with 8-byte texels takes the
  copy path (the GL core itself gives a Mali texture 8, section 4 step 3); a
  3D RGBA16F texture is refused or built with 8-byte texels, never with 4
  (section 3 step 3).
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
  (`src/mali/backend.rs:1757-1762`), where Mesa's table row for R32_FLOAT is
  R001 like R16F's (`M:panfrost/lib/pan_format.c:451`, `FMT(R32_FLOAT, R32F, R001, ..)`
  family). Both read (r, 0, 0, 1); whether to align R32F's swizzle is for the
  item that next touches it, not this one.
- `SAMPLED_TEXFMTS` (`src/mali/backend.rs:96`) could then name RGBA16F and
  R16F; that changes which storage class R11G11B10F, RGB9E5 and the snorm
  formats fall back to (`src/gl/es3_formats.rs:80-135`) and is a separate
  decision with its own tests (`src/gl/es3_format_tests.rs:230`).
