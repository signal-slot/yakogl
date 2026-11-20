<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# VC4 reference data (Mesa 26.2.1, gallium vc4, drm-shim)

What Mesa's own GL driver for the VideoCore IV submits for three draws, and
what Mesa's vc4 compiler makes of the two shader programs, for golden tests
of our packet and QPU encoders. Nothing here ran on a GPU: the drm-shim
answers the driver's ioctls and stubs the submit. `tools/vc4/ref_selftest.py`
checks these files (no Mesa needed).

## How each file was made

With a work directory built by `tools/vc4/drm_shim_ref.sh build
mesa-26.2.1.tar.xz <work dir>` (the tarball `third_party/mesa-26.2.1/UPSTREAM`
names), from the repository root:

```sh
tools/vc4/drm_shim_ref.sh run <work dir> clear    tools/vc4/testdata/clear
tools/vc4/drm_shim_ref.sh run <work dir> triangle tools/vc4/testdata/triangle
tools/vc4/drm_shim_ref.sh run <work dir> flat     tools/vc4/testdata/flat
tools/vc4/drm_shim_ref.sh shc <work dir> tools/shaders/passthrough.vert \
    tools/shaders/interp_color.frag tools/vc4/testdata/triangle
tools/vc4/drm_shim_ref.sh shc <work dir> tools/vc4/testdata/flat.vert \
    tools/vc4/testdata/flat.frag tools/vc4/testdata/flat
```

The cases are `tools/v3d/gl_ref.py`'s, drawn into a 64x64 RGBA8 framebuffer
object: `clear` is `glClearColor(0, 0, 1, 1)` and `glClear`; `triangle` adds
one Gouraud triangle (two `vec4` float attributes, a colour varying); `flat`
adds the same triangle in one colour, from the uniform `u_color` = (1, 0.5,
0, 1). The `shc` sources are the same programs in GLSL 450
(`passthrough.vert` and `interp_color.frag` are gl_ref.py's triangle pair;
`flat.vert` and `flat.frag` here are its flat pair).

## Files

Per case (`clear`, `triangle`, `flat`), one submit each (`submit1`):

| File | What |
|---|---|
| `<case>.submit1.struct.bin` | `struct drm_vc4_submit_cl` (176 bytes) as Mesa passed it, the four user pointers set to 0 |
| `<case>.submit1.bcl.bin` | the binner control list (`bin_cl_size` bytes) |
| `<case>.submit1.shader_rec.bin` | the shader records, with the relocation handle indices in front of each (the kernel's input format, `kernel/vc4_validate.c`) |
| `<case>.submit1.uniforms.bin` | the uniform streams, values filled in by Mesa |
| `<case>.submit1.bo_handles.bin` | the BO handle table |
| `<case>.submit1.bo<H>.bin` | BO handle H: a shader BO's QPU code in full; a plain BO (the vertex upload buffer) up to its last non-zero byte |
| `<case>.submit1.txt` | every field of the struct by name, and the BO table (index, handle, kind, size, file) |
| `<case>.log` | Mesa's `VC4_DEBUG=cl,qpu` text: the binner list as `vc4_cl_dump.c` decodes it and the QPU disassembly; also the shim's line for the stubbed submit |
| `<case>.stdout` | gl_ref.py's lines (renderer `VC4 V3D 2.1`, `glGetError 0x0`) |

Per program (`triangle`, `flat`):

| File | What |
|---|---|
| `<case>.shc.json` | vc4_shc: the fs, vs and cs QPU words, each uniform stream (`contents`, its name in `enum quniform_contents`, `data`), the FS input slots, the VS attribute offsets |
| `<case>.shc.disasm` | the same words through Mesa's `vc4_qpu_disasm` |

`flat.vert` and `flat.frag` are the flat program's sources for `shc`.

## What is in them

- `clear`: binner list of 21 bytes, 5 packets (`TILE_BINNING_MODE_CONFIG`,
  `START_TILE_BINNING`, `PRIMITIVE_LIST_FORMAT`, `INCREMENT_SEMAPHORE`,
  `FLUSH`), no shader record, one BO (the colour buffer, never mapped).
  `flags` is `USE_CLEAR_COLOR`, `clear_color` `0xffff0000` twice.
- `triangle` and `flat`: 92 bytes, 16 packets each; after the three above:
  `CLIP_WINDOW`, `CONFIGURATION_BITS`, `DEPTH_OFFSET`, `POINT_SIZE`,
  `LINE_WIDTH`, `CLIPPER_XY_SCALING`, `CLIPPER_Z_SCALING`, `VIEWPORT_OFFSET`,
  `FLAT_SHADE_FLAGS`, `GL_SHADER_STATE`, `GL_ARRAY_PRIMITIVE`, then
  `INCREMENT_SEMAPHORE`, `FLUSH`. One shader record (72 bytes for two
  attributes, 60 for one), three shader BOs, the vertex buffer and the colour
  buffer. QPU instructions (fs, vs, cs): triangle 13, 25, 23; flat 8, 21, 23.
- vc4_shc's fs, vs and cs words are byte for byte the shader BOs the GL driver
  submitted, for both programs (`ref_selftest.py` checks it).
- The render control list is not here: the kernel builds it from the submit's
  surface fields, and the shim has no kernel.
