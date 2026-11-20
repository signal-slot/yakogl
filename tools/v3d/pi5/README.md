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
