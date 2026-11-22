<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1 references taken on the Raspberry Pi 5 (D3)

Item D3 of `docs/notes/pre-study-rpi5.md` (section 6): what the Pi 5's own
Linux says about its GPU, and the control lists Mesa submits on it. They
replace the drm-shim approximations of `tools/v3d/shim71/` (H3) where they
differ; the differences are listed below. Nothing here comes from our
driver. The `gl_*` files are not from the board: they are Mesa on the
drm-shim at the board's revision (the last section).

Taken on 2026-11-19 on the bench board (Raspberry Pi 5 Model B Rev 1.1,
8 GB, BCM2712 D0), Raspberry Pi OS Lite trixie, kernel `6.18.50+rpt-rpi-2712`
(16 KiB pages), Mesa `26.2.2-1~bpo13+0~rpt1` (the image's packages; nothing
was installed), boot loader of 2025-06-13. `vcgencmd get_throttled` read
`throttled=0x0` before, between and after the records. Linux was not
rebooted for them.

| File | What | How |
|---|---|---|
| `v3d_ident.txt` | debugfs `v3d_ident` | `sudo cat /sys/kernel/debug/dri/1002000000.v3d/v3d_ident` |
| `v3d_regs.txt` | debugfs `v3d_regs` (raw hub, MMU, GMP and core 0 registers) | the same directory, `v3d_regs` |
| `clocks.txt` | the V3D clock at idle (`clk_summary`), debugfs `measure_clock`, `vcgencmd measure_clock`, the `*_freq` keys of `vcgencmd get_config int`, core volts, `get_throttled` | as its `#` lines say |
| `clear.clif`, `clear.stdout` | Mesa's `V3D_DEBUG=clif` dump of a GL clear (64x64 RGBA8 FBO, `glClearColor(0, 0, 1, 1)`) | `V3D_DEBUG=clif MESA_SHADER_CACHE_DISABLE=true python3 tools/v3d/gl_ref.py /usr/lib/aarch64-linux-gnu clear` on the board (stderr is the `.clif`, stdout the `.stdout`) |
| `triangle.clif`, `triangle.stdout` | the same clear and one Gouraud triangle | the same with `triangle` |

`gl_ref.py` is the program `tools/v3d/drm_shim_ref.sh run` used for
`shim71/`, unchanged, on the board's real render node (no shim, no loader
override); it was copied to a tmpfs and removed afterwards.

## The GPU, decoded

`python3 tools/v3d/ident71.py tools/v3d/pi5/v3d_regs.txt`:

- HUB_IDENT0..3 `0x42554856 0x00081117 0x00001900 0x00020a10`; CTL_IDENT0..2
  `0x07443356 0x81001431 0xc0078101`; MMU_DEBUG_INFO `0x20804664`.
- V3D 7.1, IPREV 10, IPIDX 16, compat_rev 2 (Mesa's renderer string: `V3D
  7.1.10.2`); 1 core, 3 slices, 12 QPUs, 1 TMU a slice (debugfs prints
  3 TMUs, slices times IDENT1's NTMU), no semaphores, MMU yes, no TFU, MSO
  yes, no L3C; VPM 65536 bytes (IDENT1 VPM_SIZE 8); MMU VA and PA 36 bits,
  MMU version 4.
- Mesa's variant consequences (U2): no accumulators, draw index (rev >= 10:
  the BCM2712 D0 shader record, below), unpack sat and max0, 8 render
  targets, framebuffer 7680, clipper XY granularity 64, CLE readahead 1024.
- debugfs prints the revision as `7.1.10.16`: its fourth number is IPIDX
  (HUB_IDENT3 bits 7:0), not the compat_rev Mesa prints fourth (2).
- Clock: `fw-clk-v3d` reads 500 MHz with enable count 0 while idle; under
  load (debugfs `measure_clock`) the V3D ran at about 965 to 978 MHz and
  `vcgencmd measure_clock v3d` read 960 MHz (`v3d_freq=960`,
  `v3d_freq_min=500`). `core_freq` 910 (min 500); the core clock read 910
  or 500 MHz depending on load.

## Differences from `tools/v3d/shim71/`

Compared with the buffer names' addresses taken out (the offsets kept):

1. **The GPU revision.** The shim reports `V3D 7.1.7.0`; the board is
   `V3D 7.1.10.2` (BCM2712 D0; the firmware loads the `bcm2712d0` overlay
   on this board). The shim's revision is older than the board's.
2. **Buffer addresses and sizes.** The board's Mesa allocates every buffer
   in 16 KiB (the kernel's page size; Mesa 26.2.1's `v3d_bufmgr.c` aligns to
   4 KiB, the shim's dumps have 4 KiB buffers), so every buffer base differs
   (`resource_0x9000`, `CL_0x11000`, ... against `resource_0x3000`,
   `CL_0x8000`, ...) and the `@format blank` sizes with them. Field by
   field, the only value that moves is the **tile allocation size**, the
   submit's `ADD_BIN` fourth argument (qms): 16384 on the board, 12288 on
   the shim and in our builder. `tools/v3d/check71.d/frame_check.sh` (d)
   compares it apart (the board's must be ours rounded up to 16 KiB).
3. **The clear's control lists are otherwise equal**: the same 36 packets
   with the same fields (binning list 19 bytes, render list 96, generic tile
   list 27). frame_check (d): `clif_cmp: 36 packets, 0 differences`.
4. **The triangle's shader record.** Mesa packs the BCM2712 D0 layout
   (`GL Shader State Record Draw Index`, `v3d_packet.xml`: "exclusive to
   2712D0 (V3D 7.1.10)", picked by `v3d_device_has_draw_index`, rev >= 10),
   which inserts `cs_basevertex`, `cs_drawindex`, `vs_basevertex` and
   `vs_drawindex` and moves every flag from bit 5 up by 2 to 4 bits. Mesa's
   CLIF dump decodes it with the plain 7.1 layout, so the dump shows three
   flags that look changed: "uses real pixel centre W" 1 -> 0, "Insert
   Primitive ID as first varying" 0 -> 1, "Disable implicit point/line
   varyings" 1 -> 0. Read at the D0 layout the values are the shim's (real
   pixel centre W is bit 15, set; Insert Primitive ID bit 18, clear; Disable
   implicit point/line varyings bit 21, not decoded by the plain layout).
   A 7.1 shader record we write for this board must use the D0 layout.
5. **Shader code and the rest of the triangle job** (the FS/VS/CS words, the
   uniforms, both attribute records, the vertex data and every control
   list packet) are equal to the shim's.

No other line differs.

## The gl_test shapes, Mesa on the drm-shim at 7.1.10 (refs71)

Not from the board: Mesa's own GL driver (gallium v3d) on Mesa's drm-shim,
the method of `tools/v3d/shim71/` (`tools/v3d/README.md`, H3), with the shim
reporting the Pi 5's revision. They are the references for the 7.1 job
builder and GL table on the shapes of gl_test cases 5, 58, 59, 66, 67 and
102, the six cases P9 runs (`docs/notes/pi4-v3d-bringup.md`, "gl_test on the
V3D"), and for the 4- and 8-target MRT clears.

**The Mesa build.** Mesa 26.2.1 from the tarball
`third_party/mesa-26.2.1/UPSTREAM` names (sha256 `c47e81bddc4760360a41ac3c5acec38acb81f9d750ecef47e7f3adc7021a4442`,
checked),
built on 2026-11-20 on an x86_64 Ubuntu host (gcc 15.2.0, meson 1.10.1) by

```sh
bash tools/v3d/drm_shim_ref.sh build <mesa-26.2.1.tar.xz> <work dir>
bash tools/v3d/drm_shim_ref.sh run <work dir> <shape> tools/v3d/pi5/gl_<shape> 7.1.10
```

The `build` form adds a third shim GPU, `7.1.10`, to the drm-shim
(`src/broadcom/drm-shim/v3d_noop.c` of the extracted tree): the identity
registers of `v3d_regs.txt` above (HUB_IDENT1..3 `0x00081117 0x00001900
0x00020a10`, CTL_IDENT0..2 `0x07443356 0x81001431 0xc0078101`, UIFCFG
`0x45`). The shim's `71` and `42` GPUs are unchanged: the same build at `71`
gives `shim71/clear.clif`, `triangle.clif` and `triangle.stdout` byte for
byte. **The device revision**: every `gl_*.stdout` reads `gl_shapes: renderer
V3D 7.1.10.2` (version 7.1, rev 10, compat_rev 2; the board's Mesa prints the
same). Check of the shim's identity against the board: `gl_ref.py triangle`
on this build at `7.1.10` equals this directory's `triangle.clif` (the board's
dump) through `tools/v3d/clif_cmp.py --ignore-addresses`: 59 packets, 0
differences (the D0 draw-index shader record included; at `71` the three
record flags of "Differences" item 4 differ).

**The input** is `src/gl_shapes.py` (`drm_shim_ref.sh run` takes it for any
case other than `clear` and `triangle`): gl_test's GL calls for the case, in
an OpenGL ES 3.0 context on an EGL surfaceless display, 64x64 (gl_test's TW x
TH), gl_test's ES 1.00 shader pair (`gl_test_shaders_es100.h`,
`gl_test_vs_src`/`gl_test_fs_src`) and its 40-byte vertex at attributes 0, 1,
2. Cases 5, 58, 59, 66 and 102 draw to gl_test's window surface, which has
depth and stencil; here that is a framebuffer object of a GL_RGBA texture and
a GL_DEPTH24_STENCIL8 renderbuffer. Case 67 makes its own framebuffer objects
as gl_test does. glReadPixels is left out (no job completes on the shim);
every glFinish of the case stays, and each ends one job. The docstring of
`gl_shapes.py` has every call of every shape.

| File (`.clif`, `.stdout`) | gl_test case | Jobs | What the jobs hold |
|---|---|---|---|
| `gl_scissor` | 5 `case_scissor` | 2 | a full black clear; then, scissored to (0,0,32,32), a clear (a blitter quad) and the yellow full-screen quad |
| `gl_scisbox` | 58 `case_clearscis` | 3 | blue, then a red clear scissored to (16,16,32,32) (a blitter quad); the same with a green quad before the scissored clear; the same with an empty (0-wide) scissor box |
| `gl_writemask` | 59 `case_clearmask` | 4 | a clear under `glColorMask` all off; a depth-tested (GL_LESS) red quad, a depth clear under `glDepthMask(GL_FALSE)`, a green quad behind; colour and stencil cleared to 0, a stencil clear to 0xff through `glStencilMask(0x0f)` (a blitter quad with `STENCIL_CFG`) and a draw testing EQUAL 0xff; then a draw testing EQUAL 0x0f in a new job, whose render list `LOAD`s render target 0 and the stencil (`d24s8`) the job before stored |
| `gl_viewport` | 66 `case_clearvp` | 3 | a white clear at the full viewport, a blue one at a quarter viewport (the whole target), a green clear scissored to the bottom-left quarter |
| `gl_fbods` | 67 `case_fbods` | 9 | frames 1 to 8: colour-only FBO with the depth test (no depth buffer: `Internal Depth Type` 0, `depth_32f`, the field's zero; `depth_16` in frames 4 and 5, `depth_24` in the stencil-only frames 6 to 8) and then the stencil test on; a GL_RGB colour target cleared with alpha 0.25; two FBOs sharing one colour texture, each with a GL_DEPTH_COMPONENT16 renderbuffer (depth-tested draws, the second with no clear); a GL_STENCIL_INDEX8 FBO with a scissored stencil-only clear and a stencil-tested draw, and the same with a draw ahead in one job and across a glFinish (the last job loads the stencil) |
| `gl_colormask` | 102 `case_maskclear` | 6 | black, then an R+A masked clear to (1,1,1,0.5); black, then a G-only clear scissored to the bottom-left 32x32; black, then a B-only clear and a yellow quad on the bottom-left quadrant with every channel on |
| `gl_mrt4` | - | 1 | one clear of four GL_RGBA8 colour targets (`glDrawBuffers` 4): `Number of Render Targets` 4, four `TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1` |
| `gl_mrt8` | - | 1 | the same with eight targets, V3D 7.1's maximum |

Reading them:

- `MESA: error: Export failed` follows each job (one per glFinish): the shim
  has no sync-file export for the wait. It comes after the job's dump, which
  is complete; it is not a `MESA: info:` line, so the decoders skip it.
- A clear that Mesa cannot do in the tile buffer (a scissor box, a partial
  colour mask, a stencil write mask) is a draw in the dump (`GL_SHADER`,
  `COLOR_WMASKS`, Mesa's blitter shaders): that is Mesa's way, not
  necessarily ours. A full clear is `CLEAR_RT` (7.1's `Clear Render Targets`)
  in the render list.
- The `GL_SHADER` records are the D0 layout (`GL Shader State Record Draw
  Index`), decoded by the dump with the plain 7.1 layout, as "Differences"
  item 4 above says for the triangle.
- Buffers are 4 KiB-aligned (the shim) where the board's are 16 KiB: the
  addresses are the shim's, and so is the tile allocation size `ADD_BIN`
  arg3 of a job with no draw: 12288 (10 jobs), where the board's 16 KiB
  buffers give 16384 ("Differences" item 2); the 19 jobs with a draw have
  16384, as the board's triangle has.
- No file name contains `clear` or `ident`: `check71.d/frame_check.sh` (d)
  takes every `pi5/*clear*.clif` as a board clear, and `ident71.sh` every
  `pi5/*ident*`.

**Decoding.** `bash tools/v3d/pi5/src/decode71.sh` builds the checker of
`tools/v3d/check71.d/shim71.sh` against the vendored reference build
(`tools/v3d/mesa_ref.sh`; or `MESA_V3D_TREE`) and reads every `gl_*.clif`
with the V3D 7.1 packet spec of Mesa's CLE decoder: every packet each control
list names is a 7.1 command and no list is empty. It also checks that each
`.stdout` reports `V3D 7.1.10.2` and `glGetError 0x0`, and the number of jobs
in each file (the table). On 2026-11-20: 8 files, 1514 packets, 0 failed.
Two runs of all eight shapes gave byte-identical files.

## The P10 texture shapes, Mesa on the drm-shim at 7.1.10 (texrefs71)

The same method, build and command as the section above (Mesa 26.2.1,
commit `da14d65e4499e66468094be52bff9ea0915a695e`, built on 2026-11-20 on
an x86_64 Ubuntu host, gcc 15.2.0, meson 1.10.1; the same build reproduced
the eight refs71 files byte for byte first). They are the references for
the 7.1 texture state (tex71), glGenerateMipmap through the TFU (mip71) and
the first cube-map case of the board (board71-tex, U8):

```sh
bash tools/v3d/drm_shim_ref.sh run <work dir> <shape> tools/v3d/pi5/gl_<shape> 7.1.10
```

| File (`.clif`, `.stdout`) | gl_test case | Jobs | What the jobs hold |
|---|---|---|---|
| `gl_tex2d` | - | 2 | a 64x64 `GL_RGBA8` texture sampled over the 64x64 window, NEAREST, then LINEAR |
| `gl_texfmt` | - | 2 | the same draw from a `GL_SRGB8_ALPHA8` texture, then from a `GL_RGB565` texture |
| `gl_mipgen` | - | 1 (and 1 TFU) | `glGenerateMipmap` of a 7-level 64x64 `GL_RGBA8` texture (a TFU job, before the draw), then a LINEAR_MIPMAP_LINEAR draw |
| `gl_cube` | 115 `case_texcube` | 12 | per face, NEAREST then LINEAR: a clear to (0,0,0,0) and the face's quad into a 32x32 viewport |

**The GL calls.** Each shape starts with the window of the section above
(a 64x64 `GL_RGBA` texture and a `GL_DEPTH24_STENCIL8` renderbuffer,
`glViewport(0, 0, 64, 64)`). `gl_tex2d`, `gl_texfmt` and `gl_mipgen` draw
with gl_test's ES 1.00 UV pair (`gl_test_uv_vs_src`,
`gl_test_texture_fs_src`: `gl_FragColor = texture2D(u_tex, v_uv)`, `u_tex`
on unit 0) and its 24-byte `puv` vertex (pos4 and uv2, `puv_quad()`: two
triangles over the viewport, uv (0,0) at NDC (-1,-1)) at attributes 0 and
1. Their texels are fixed bytes (`gl_shapes.py` `tex_bytes()`), uploaded
with `GL_UNPACK_ALIGNMENT` 4; each draw is `glClearColor(0,0,0,1)`,
`glClear(GL_COLOR_BUFFER_BIT)`, `glDrawArrays(GL_TRIANGLES, 0, 6)`,
`glFinish()`.

- `gl_tex2d`: `glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 64, 64, 0,
  GL_RGBA, GL_UNSIGNED_BYTE, data)`, min and mag filter `GL_NEAREST`, the
  wrap modes left at `GL_REPEAT`; the draw; min and mag `GL_LINEAR`; the
  draw.
- `gl_texfmt`: the same with `GL_SRGB8_ALPHA8` (`GL_RGBA`,
  `GL_UNSIGNED_BYTE`), NEAREST, the draw; then a new texture
  `GL_RGB565` (`GL_RGB`, `GL_UNSIGNED_SHORT_5_6_5`), NEAREST, the draw.
- `gl_mipgen`: `glTexStorage2D(GL_TEXTURE_2D, 7, GL_RGBA8, 64, 64)`,
  `glTexSubImage2D` of level 0, `glGenerateMipmap(GL_TEXTURE_2D)`, min
  filter `GL_LINEAR_MIPMAP_LINEAR`, mag `GL_LINEAR`, the draw.
  (`glTexStorage2D` so that Mesa does not reallocate the texture, and copy
  level 0, inside `glGenerateMipmap`.)
- `gl_cube`: gl_test case 115 (`driver/tools/pvrlab/gl_test.c:11163`
  `case_texcube`) with its scene from `tools/v3d/tex_cube_case.h`: the
  texcube pair (`gl_test_texcube_vs_src`, `gl_test_texcube_fs_src`:
  `textureCube(u_s, v_dir)`, `v_dir = u_major + a_pos.x * u_sx + a_pos.y *
  u_ty`), the six 4x4 `GL_RGBA` faces of `tcc_make_faces()` (R a ramp along
  s, G along t, B `16 + 40 * face`) to `GL_TEXTURE_CUBE_MAP_POSITIVE_X + f`,
  wrap S and T `GL_CLAMP_TO_EDGE`, `glViewport(0, 0, 32, 32)`; then for
  draw i in 0..11 (face i / 2, LINEAR when i is odd): `glUniform3f` of
  `tcc_faces[face]`'s `u_major`, `u_sx`, `u_ty`, min and mag filter,
  `glClearColor(0,0,0,0)`, `glClear`, the quad, `glFinish`. The context is
  ES 3.0, so Mesa's core asks for seamless cube filtering
  (`src/mesa/main/version.c:690`, `CubeMapSeamless = _mesa_is_gles3(ctx)`), the case board71-tex runs
  on the ES 3.0-reporting image (`docs/notes/v3d71-cube-seamless.md`
  section 8). glReadPixels and the verdict are left out, as in every shape.

**Reading the texture records.** The CLIF dump decodes the control lists
and shader records but prints the uniform streams and Mesa's state buffer
(`resource_0x1000`: sampler records, texture records and shader code) as
raw words. `python3 -I tools/v3d/pi5/src/texstate71.py <file.clif> [job]`
takes, for every job, the fragment shader's uniform stream (from the
record's `Fragment Shader Uniforms Address` to the next stream) and each
pair of words that point into dumped words, TMU Config Parameter 0
(texture state address | return words) and Parameter 1 (sampler state
address | flags), and prints both records field by field with the 7.1
layouts of `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`
(`Texture Shader State` min_ver 71, `Sampler State`). In every job of the
four shapes the fragment shader has exactly those two uniforms: P0 return
words 3, P1 per-pixel mask 1, unnormalized 0, output 32-bit 0. The records
(words in memory order; fields as texstate71.py decodes them):

| Shape, job | Texture Shader State (7.1), 6 words | Fields | Sampler State, 6 words | Fields |
|---|---|---|---|---|
| `gl_tex2d` 1 | `0x0000f000 0x00000200 0x00404001 0x00b1a840 0x000f0340 0x00003c00` | type 4 RGBA8, 64x64, depth 1, base/max level 0/0, swizzle R G B A, array stride 256 (x 64 bytes), level 0 strictly UIF, XOR off, UB_PAD 0, R/B swap 0, reverse 0, transfer func 0 (none), Cb = Cr = base >> 6 (960), chroma offsets 1, 1 | `0x00100007 0 0 0 0 0` | mag, min, mip filter nearest; min LOD 0, max LOD 1/256; wrap S, T, R 0 (REPEAT); bias 0; compare NEVER; border 0 |
| `gl_tex2d` 2 | the same record | | `0x00100004 0 0 0 0 0` | mag and min LINEAR, mip nearest; the rest as job 1 |
| `gl_texfmt` 1 | `0x0000f004 0x00000200 0x00404001 0x00b1a840 0x000f0340 0x00003c00` | as `gl_tex2d`, with **transfer func 1 (sRGB)** in bits 2..4 of word 0 | `0x00100007 0 0 0 0 0` | as `gl_tex2d` 1 |
| `gl_texfmt` 2 | `0x0001b000 0x00000100 0x00404001 0x0031a860 0x001b0340 0x00006c00` | **type 6 RGB565**, swizzle R G B **1**, array stride 128, R/B swap 0, transfer func 0 | `0x00100007 0 0 0 0 0` | as `gl_tex2d` 1 |
| `gl_mipgen` 1 | `0x00011000 0x00000300 0x00404001 0x06b1a840 0x00110340 0x00004400` | type 4 RGBA8, 64x64, **max level 6**, array stride 384 (the 7-level chain, 24576 bytes), level 0 strictly UIF, transfer func 0 | `0xf0000000 0 0 0 0 0` | mag, min and mip filter LINEAR; min LOD 0, **max LOD 15**; wrap REPEAT |
| `gl_cube` odd jobs (NEAREST) | `0x0000d000 0x10000002 0x01800400 0x00b1a040 0x000d0300 0x00003400` | type 4 RGBA8, **4x4, depth 6** (the six faces as layers), array stride 1 (64 bytes, one 4x4 face), base/max level 0/0, level 0 not strictly UIF (a 4x4 image is not UIF-tiled), R/B swap 0, transfer func 0, no cube or seamless field | `0x00100007 0x00490000 0 0 0 0` | mag, min, mip filter nearest; **wrap S, T and R 1 (CLAMP)**; max LOD 1/256 |
| `gl_cube` even jobs (LINEAR) | the same record | | `0x00100004 0x00490000 0 0 0 0` | mag and min LINEAR, mip nearest; wrap S, T, R CLAMP |

What Mesa sets, and where (Mesa 26.2.1; `third_party/` vendors the format
table and the packet XML; the other files are the tarball's, as extracted by
the build):

- **Transfer function, per revision.** `src/gallium/drivers/v3d/v3dx_state.c:1023-1029`:
  `is_srgb = util_format_is_srgb(cso->format)`, written as `tex.srgb` at
  `V3D_VERSION == 42` and as `tex.transfer_func = is_srgb ?
  TRANSFER_FUNC_SRGB : TRANSFER_FUNC_NONE` at `V3D_VERSION >= 71` (line
  1028): `gl_texfmt` job 1's word 0 `0x...004`. The texture type stays
  RGBA8 (`v3dx_format_table.c:67`, `R8G8B8A8_SRGB` -> `SRGB8_ALPHA8` render
  target, `RGBA8` texture).
- **R/B swap.** The Gallium (GL) path never writes `r_b_swap` or `reverse`
  (no assignment in `src/gallium/drivers/v3d/`); only Vulkan does
  (`src/broadcom/vulkan/v3dvx_image.c:144-145`, `tex.reverse` and
  `tex.r_b_swap` from the view's `channel_reverse` and `swap_rb`). A
  swapped channel order reaches the GL record through the format table's
  type and swizzle: `GL_RGB565` with `GL_UNSIGNED_SHORT_5_6_5` is Gallium's
  `B5G6R5_UNORM`, which `third_party/mesa-26.2.1/src/gallium/drivers/v3d/v3dx_format_table.c:81`
  maps to texture type `RGB565` (the XML: "B in low bits", `v3d_packet.xml:339`)
  with swizzle `SWIZ_XYZ1`; `gl_texfmt` job 2 has R/B swap 0, swizzle R G B 1.
  So no shape here can show the bit set: in GL it is 0 at every revision.
- **The TFU job.** `glGenerateMipmap` reaches `v3d_generate_mipmap`
  (`src/gallium/drivers/v3d/v3d_blit.c:240`; the cap is
  `v3d_screen.c:296`, `DRM_V3D_PARAM_SUPPORTS_TFU`, which the shim answers 1,
  `drm-shim/v3d_noop.c:147`), which calls `v3dX(tfu)` with `for_mipmap`
  (`src/gallium/drivers/v3d/v3dx_tfu.c`; the 7.1 block is lines 165-202,
  `IOC` at 176-201, the ioctl at 204).
- **Seamless cube.** v3d has no field for it (`docs/notes/v3d71-cube-seamless.md`
  sections 1 and 3); the one trace in the records is the state tracker's:
  `src/mesa/state_tracker/st_atom_sampler.c:88-95` forces wrap S, T and R
  to `CLAMP_TO_EDGE` for a cube map when `seamless_cube_map` is set (it is,
  `CubeMapSeamless` in an ES 3 context): `gl_cube`'s sampler word 1
  `0x00490000` is CLAMP in all three, where the GL state has wrap R at its
  default REPEAT.

**The TFU registers (`gl_mipgen`).** Mesa's `V3D_DEBUG=clif` dumps the
control lists of render jobs only; a TFU job is its own ioctl,
`DRM_IOCTL_V3D_SUBMIT_TFU`, and the shim's handler is a no-op, so the CLIF
holds no trace of it. `gl_shapes.py` runs each of the four P10 shapes with
`src/tfu_spy.c` ahead of the shim in `LD_PRELOAD` (it builds it with `cc`
into `<libdir>/gl_shapes_tfu_spy.so` and runs itself again;
`drm_shim_ref.sh` is unchanged): an `ioctl()` that prints the submission's
`struct drm_v3d_submit_tfu`
(`third_party/mesa-26.2.1/include/drm-uapi/v3d_drm.h:317-347`) on stdout
and passes the call on. Its two lines in `gl_mipgen.stdout`:

```
gl_shapes: SUBMIT_TFU icfg 0x03040000 iia 0x00011000 iis 0x00000008 ica 0x00000000 iua 0x00000000 ioa 0x00011000 ios 0x00400040 coef 0x00000000 0x00000000 0x00000000 0x00000000 ioc 0x00086061 bo_handles 6 0 0 0 in_sync 1 out_sync 1 flags 0x0 extensions 0
gl_shapes: SUBMIT_TFU 7.1: ICFG IFORMAT 6 OTYPE 4; IOS 64x64; IOC DIMTW 1 NUMMM 6 FORMAT 6 STRIDE 8
```

With the shifts of Mesa's `src/broadcom/common/v3d_tfu.h` (`V3D71_*`):
`ICFG` = input format 6 (`UIF_NO_XOR`, bits 23 up) and output type 4
(`RGBA8`, bits 16 up), nothing else (at 7.1 the level count moved to IOC);
`IIA` = `IOA` = `0x00011000`, the texture's own buffer (source and
destination are the same resource; `bo_handles` names it once); `IIS` 8 =
the source's padded height in UIF blocks (64 / (2 x 4-row utiles));
`IOS` = `0x00400040`, height 64 << 16 | width 64; **`IOC` = `0x00086061`**:
`DIMTW` 1 (bit 0: level 0 is not written, only the chain below it), `NUMMM`
6 (bits 4 up: levels 1 to 6), `FORMAT` 6 (bits 12 up: `UIF_NO_XOR`),
`STRIDE` 8 (bits 16 up: the destination's padded height in UIF blocks);
`ICA`, `IUA`, the coefficients and `flags` 0; in and out sync the context's
one syncobj. The draw that follows samples the chain: `Max Level` 6 in the
texture record above.

**Divergences from the board.** Those of the section above hold here: the
GL shader records are the D0 draw-index layout decoded by the dump with the
plain 7.1 layout (every new record reads "Insert Primitive ID as first
varying" 1; "Differences" item 4), and buffers are 4 KiB-aligned on the
shim where the board's are 16 KiB, so every address differs (the texture
base pointers, `Cb`/`Cr` (base >> 6), the TFU `IIA`/`IOA`, the P0/P1
uniforms). All 17 jobs have a draw and a tile allocation size of 16384, the
board's figure. Two more, from the TFU: the shim answers
`DRM_V3D_PARAM_SUPPORTS_TFU` 1 unconditionally (`v3d_noop.c:147`), where
the board's answer comes from its kernel (this directory's `v3d_ident.txt`
decodes as "no TFU" in the identity registers, so board71-tex should read
what the board's own Mesa does for `glGenerateMipmap` before taking
`gl_mipgen`'s TFU job as the board's); and the shim never runs the TFU
job, so nothing here says what the TFU writes.

**Decoding.** `bash tools/v3d/pi5/src/decode71.sh` covers the four files
too: their job counts (the table), `V3D 7.1.10.2` and `glGetError 0x0` in
each `.stdout`, the number of `SUBMIT_TFU` lines (1 in `gl_mipgen`, 0 in
the others), and a texture and sampler record found by `texstate71.py` for
the fragment shader of every job. On 2026-11-20: 12 files, 2432 packets,
0 failed; 2 + 2 + 1 + 12 texture records. Two runs of the four shapes gave
byte-identical files.

## The P13 draw shapes, Mesa on the drm-shim at 7.1.10 (refs71b)

The same method, build and command as the two sections above (Mesa 26.2.1,
the same tarball and sha256, built on an x86_64 host with the `build` form of
`drm_shim_ref.sh`; the same build reproduced `gl_scissor` and `gl_stencilops`
byte for byte on a second run). Shapes of `src/gl_shapes.py` (`DRAW_SHAPES`;
the docstring has every GL call) for the draws dEQP-GLES2 and glmark2 need
beyond the six gl_test cases. Each is one job, so the packets between the draws
of a job show what a state change emits. The vertex is gl_test's 40-byte cuv
vertex and the window the one above, except where the row says otherwise.

| File (`.clif`, `.stdout`) | Jobs | What the job holds |
|---|---|---|
| `gl_blend` | 1 | after a black clear, two quads with alpha 0.5: `glBlendFuncSeparate(SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE, ZERO)` with `GL_FUNC_ADD`, then `(CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR, ONE, ONE)` with `GL_FUNC_REVERSE_SUBTRACT` (colour) and `GL_MAX` (alpha) and `glBlendColor(0.25, 0.5, 0.75, 0.5)`: two `BLEND_CFG` (colour and alpha factors and modes) and the `BLEND_CCOLOR` of the constant |
| `gl_depthfunc` | 1 | depth cleared to 0.5 with depth writes off: a quad at z -0.5 under `GL_LESS`, then a quad at z 0.75 under `GL_GEQUAL`: the `CFG_BITS` depth-test function (LESS, then GEQUAL) changing between two draws with Z updates off |
| `gl_stencilops` | 1 | `GL_DEPTH24_STENCIL8`, stencil cleared to 0x10: a quad with `glStencilFunc(LEQUAL, 0x20, 0x3f)`, `glStencilOp(INCR, DECR, INVERT)`, write mask 0x0f and the depth test; a quad with different front (`REPLACE, KEEP, INCR_WRAP`) and back (`ZERO, DECR_WRAP, INVERT`) ops, `NOTEQUAL 0x05` mask 0xf0, write mask 0xff: three `STENCIL_CFG` (one for the first draw, front and back for the second) |
| `gl_indexed` | 1 | `glDrawElements` of the quad's six vertices as `GL_TRIANGLES` with `GL_UNSIGNED_BYTE`, `GL_UNSIGNED_SHORT` (at byte offset 4) and `GL_UNSIGNED_INT` indices, and a `GL_TRIANGLE_STRIP` of u16 indices with an index restart (`GL_PRIMITIVE_RESTART_FIXED_INDEX`, 0xffff): four `INDEX_BUFFER_SETUP` and `INDEXED_PRIM_LIST`, the index type, and `Enable Primitive Restarts` 1 in the strip draw only |
| `gl_instanced` | 1 | `glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3)` with `a_pos` per vertex and `a_color` and `a_off` at `glVertexAttribDivisor` 1 from a second buffer: one `VERTEX_ARRAY_INSTANCED_PRIMS` and the attribute records' instance divisor |
| `gl_prims` | 1 | `GL_POINTS` (4 vertices, `gl_PointSize` 8.0 in the vertex shader), `GL_LINES` (4 vertices, `glLineWidth(1.0)`) and `GL_TRIANGLE_STRIP` (4 vertices), as `VERTEX_ARRAY_PRIMS` of three primitive types with `POINT_SIZE` and `LINE_WIDTH` |

`bash tools/v3d/pi5/src/decode71.sh` covers them with the earlier files (one
job each, `V3D 7.1.10.2`, `glGetError 0x0`): 18 files, 2799 packets in 88
commands of the 7.1 spec, 0 failed. The divergences from the board are those
above (D0 draw-index shader records read by the dump with the plain 7.1
layout; 4 KiB buffer alignment on the shim). The run writes `.stderr` files
next to the outputs; they are not kept.
