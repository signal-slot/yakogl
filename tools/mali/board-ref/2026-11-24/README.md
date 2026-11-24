# Board Mesa descriptor dumps: rebind, vertex cube edge, vertex two textures

These are the NanoPC-T6's own Mesa (panfrost on panthor, Mali-G610) pandecode
dumps of three GLES2 draws. The open dEQP-GLES2 pixel causes of
docs/es2-cts-failures.md M.1 rows 14, 5/11 and 17 are pinned to them.

- Board Mesa: `renderer: Mali-G610 (Panfrost)`,
  `version: OpenGL ES 3.1 Mesa 25.0.7-2+deb13u1` (Armbian, kernel 6.18).
- Command shape (run from the repository root, `BOARD_HOST` from the
  environment):

      BOARD_HOST=<user@board> tools/mali/mesa-board-ref.sh \
          tools/mali/board-ref/2026-11-24/<prog>.c tools/mali/board-ref/2026-11-24 <name> [arg]

  | name | prog | arg |
  |---|---|---|
  | rebind-rgb565-s8 | rebind-rgb565-s8.c | |
  | rebind-rgb565-s8-tex | rebind-rgb565-s8.c | tex |
  | rebind-rgb565-s8-texs8 | rebind-rgb565-s8.c | texs8 |
  | vs-cube-edge | vs-cube-edge.c | |
  | vs-cube-edge-lod | vs-cube-edge.c | lod |
  | vs-tex2d-cube | vs-tex2d-cube.c | |

  The script compiles the program on the board and runs it with
  `PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync`. Each run leaves
  `<name>.pandecode` (all of the frames, in order), `<name>.stdout` and
  `<name>.dmesg` (empty every time: the runs added no kernel messages). The
  `pandecode: dump command stream to file ...` lines in the .stdout name the
  script's fixed work directory on the board.
- Every run had the same boot_id before and after (no reboot) and ended
  with `rc=0`. The sha256 lines the script printed:

      43e40e4a0d45ced5c29dcd1d2f24b8b9af4e86e92cce6e19ae051ec2b1aa30c4  rebind-rgb565-s8.pandecode
      5a3d46f0c9da7437234c1e6d9e492ca2c85b5350586421f1bb41c674d2543de6  rebind-rgb565-s8.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  rebind-rgb565-s8.dmesg
      e2bedbf5b4bb5264bbe48fadb1e3df52d833a079ad339979e61f10fe070e82b5  rebind-rgb565-s8-tex.pandecode
      28c98d5822b3cf78c5f21943b01b089594ef93105258e4642d8511b4063fd3f6  rebind-rgb565-s8-tex.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  rebind-rgb565-s8-tex.dmesg
      c493b2feb2d412fe717ab5cec6681faf51366c5ebe0b5a40d9f43315ecf06b4e  rebind-rgb565-s8-texs8.pandecode
      6ae774210c74bd7ccbb9a37f1363a8817755f82fdc81565c848a3cd0500cb1c5  rebind-rgb565-s8-texs8.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  rebind-rgb565-s8-texs8.dmesg
      bdea9e7cd25658f9e16490275152e63ed594d121857ebc1d92792df8cbb3677f  vs-cube-edge.pandecode
      302d58a707c53d8f7c903584e0fb64c2ee09285bd7a5f44d78087467fb3d9781  vs-cube-edge.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  vs-cube-edge.dmesg
      f594b0711e139ed849a43070ebb63096fb49f8c78654ad1fc9133159f93a86bd  vs-cube-edge-lod.pandecode
      c881cc16173992fe1df3a43bfc1b6eabf20e9c801edbae152ba42171b3283373  vs-cube-edge-lod.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  vs-cube-edge-lod.dmesg
      70155b2d669a5fb2091915e3de1d73d54548e43dd406c5f95025790b2406907c  vs-tex2d-cube.pandecode
      bab5710054e4047787863474773175322f8e9022b110a26ae892c5aa6d14704e  vs-tex2d-cube.stdout
      e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  vs-tex2d-cube.dmesg

  The pandecode bytes carry GPU addresses, so a rerun can give a different
  .pandecode sha256 while the same words.

## (a) rebind-rgb565-s8: #11329, M.1 row 14

The program issues the GL calls of
`fbo.render.recreate_colorbuffer.rebind_rbo_rgb565_stencil_index8`, taken
from VK-GL-CTS `es2fFboRenderTest.cpp` (`RecreateBuffersTest::render`, with
rebind set), as its header describes. The textures are single-colour where
dEQP's have patterns. The steps:

1. Clear colour (1,0,0,1), depth and stencil. The depth test is on, and a
   textured quad is drawn at z 0.
2. A scissored (32,32,64,64) stencil clear to 1.
3. With the FBO still bound, the colour buffer is deleted, the same name is
   bound again with the same storage, and it is attached again.
4. A colour-only clear to (0,0,1,0).
5. With stencil EQUAL 0, a quad is drawn from z 1 to z -1.

Expected: the second clear colour inside the box and the second texture
outside. Mesa submits no frame of its own before the read-back: one dump
file with two fragment jobs, one `Framebuffer @` per job.

- `rebind-rgb565-s8` (RGB565 rbo + STENCIL_INDEX8 rbo): `status before:
  0x8cd5`, `status after: 0x8cd5`, `inside (64,64): 0 0 255 255`, `outside
  (8,8): 0 255 255 255`, `fnv1a: abf5bdc5`, `pixels differing: 0`, `verdict:
  PASS`.
- `rebind-rgb565-s8-tex` (RGB/UNSIGNED_BYTE tex2d + DEPTH_COMPONENT16 rbo +
  STENCIL_INDEX8 rbo, i.e. `rebind_tex2d_rgb_depth_component16_stencil_index8`):
  `status before: 0x8cdd` (FRAMEBUFFER_UNSUPPORTED), `verdict:
  UNSUPPORTED`. The board's Mesa does not take separate depth and stencil
  renderbuffers, so the dEQP case with that config is NotSupported on Mesa.
  The dump holds only the context set-up.
- `rebind-rgb565-s8-texs8` (RGB tex2d + STENCIL_INDEX8 rbo,
  `rebind_tex2d_rgb_stencil_index8`): the same results and the same words as
  the rbo run except the colour target (`Internal Format: R8G8B8A8`,
  `Writeback Format: R8G8B8A8`, clear words `4278190335` / `4294901760`),
  `fnv1a: abf5bdc5`, `verdict: PASS`.

The first job: the draw and the scissored stencil clear. Mesa does the
stencil clear as a draw: a `Triangle fan` over the box, colour target
format `INVALID` (no colour write), stencil `Always` with all ops `Replace`,
reference `0x1`, write mask `0xff`, `Force Early`/`Force Early`. The quad
draw has `Stencil test enable: false` and `Depth function: Always` (the
depth test is on with no depth buffer), and `Weak Early`/`Weak Early`. The
first job's framebuffer has `Pre Frame 1: Never`, `Frame Shader DCDs: 0x0`
and `ZS Clean Pixel Write Enable: true`. The clear words are `Color 0..3:
992`, the RGB565 internal packing of (1, 0, 0, 1).

The second job's words (rebind-rgb565-s8.pandecode, second `Framebuffer
@` block):

- Framebuffer parameters: `Pre Frame 0: Never`, `Pre Frame 1: Early ZS
  always`, `Post Frame: Never`, `Frame Shader DCDs` non-zero, `Width: 128`,
  `Height: 128`, `Render Target Count: 1`, `Color Buffer Allocation: 4096`,
  `S Clear: 0`, `S Write Enable: false`, `S Preload Enable: false`, `S Unload
  Enable: false`, `Z Internal Format: D24`, `Z Write Enable: true`, `Z
  Preload Enable: false`, `Z Unload Enable: false`, `Has ZS CRC Extension:
  true`, `Z Clear: 0.000000`. The first job has the same words except `Pre
  Frame 1: Never` and `Frame Shader DCDs: 0x0`.
- ZS/CRC extension: `ZS Write Format: D24S8`, `ZS Block Format: Linear`,
  `S Write Format: XXX: INVALID`, `S Block Format: No Write`, `ZS Clean
  Pixel Write Enable: false` (first job: `true`), `ZS Writeback Row Stride:
  512`, `S Writeback Base` = `ZS Writeback Base` (the same address in both
  jobs), `S Writeback Row Stride: 0`. Mesa stores the STENCIL_INDEX8 renderbuffer as Z24S8: the stencil is
  written through the combined ZS surface (Z Write Enable), and there is no
  separate S surface.
- The ZS preload (Pre frame 1 DCD): `Stencil from shader: true`, `Depth
  write enable: true`, `Depth function: Always`, front `compare Always`,
  stencil fail/depth fail/depth pass `Replace`, write mask `0xff`, `Pixel kill operation: Force
  Late`, `ZS update operation: Force Late`. Its resource table holds a
  Clamp-to-Edge sampler and two textures over the ZS surface: `2D`, `Z24X8
  UNORM RGBA`, `128x128`, `Swizzle: 0 (RRRR)`, `Levels: 1`, and `2D`, `S8
  GRBA`, `128x128`, `Swizzle: 0 (RRRR)`.
- Colour render target 0: `Internal Format: R5G6B5A0`, `Writeback Format:
  R5G6B5`, `Writeback Block Format: Linear`, `Dithering Enable: true`,
  `Swizzle: 2058 (BGR0)`, `Clean Pixel Write Enable: true`, `Row Stride:
  256`, clear words `Color 0..3: 1040187392` (0x3E000000, (0, 0, 1, 0)). Its
  base address differs from the first job's, because the recreated
  renderbuffer is a new BO.
- The second job's draw: `Stencil test enable: true`, front and back
  `compare Equal`, every op `Keep`, write/value masks `0xff`, reference
  `0x0`, `Depth write enable: false`, `Depth function: Always`, `Pixel kill
  operation: Force Early`, `ZS update operation: Force Early`.

## (b) vs-cube-edge: #7489, M.1 rows 5 and 11

An 8x8 RGBA8 cube with 4 levels, GL_REPEAT on S and T, MIN
LINEAR_MIPMAP_LINEAR, MAG LINEAR, sampled in the vertex stage at
(1, 0.9 + 0.2u, 2v - 1), across the +X/+Y edge. The .stdout holds the 16x16
results (row v, column u, RRGGBBAA). `vs-cube-edge`: `fnv1a: 204b3ac5`;
`vs-cube-edge-lod` (textureCubeLod 1.5): `fnv1a: e63f36c5`.

- The sampler, the same in both runs, in the position and varying resource
  tables, entry 3: `Wrap Mode R/T/S: Clamp to Edge` (GL_REPEAT on a cube is
  not written as Repeat), `Seamless Cube Map: true`, `Round to nearest even:
  true`, `Normalized Coordinates: true`, `Minify nearest: false`, `Magnify
  nearest: false`, `Magnify cutoff: false`, `Mipmap Mode: Trilinear`,
  `Minimum LOD: 0.000000`, `Maximum LOD: 31.996094`, `LOD bias: 0.000000`,
  `Compare Function: Never`, `Maximum anisotropy: 1`, `LOD algorithm:
  Isotropic`, border 0.
- The texture, entry 4: `Dimension: Cube`, `Format (v7): RGBA8 UNORM RGBA`,
  `Width: 8`, `Height: 8`, `Swizzle: 1672 (RGBA)`, `Levels: 4`, `Minimum
  level: 0`, texture `Minimum LOD: 0.000000`, `Maximum LOD: 3.000000`, `Array
  size: 1`, `Depth: 1`. It has 24 planes, plane i = face i/4, level i%4,
  each `Clump format: RAW32`, `Slice stride: 960`. The level sizes are 512,
  256, 128, 64 at offsets 0, 0x200, 0x300, 0x380 within a face, and the faces
  are 0x3c0 apart.
- The vertex shader (pandecode disassembly): `CUBEFACE1`, `CUBEFACE2_V9`,
  `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, s/t `FMA.f32.clamp_0_1` by 0.5. The face
  is put in the top bits of s with `MUX.i32.bit ... 0x1FFFFFFF`. Then
  `TEX_SINGLE....16.cube.zero` (textureCube: LOD zero) or
  `TEX_SINGLE....16.cube.explicit` with the LOD register `0x180` (1.5 in 8.8
  fixed point) (textureCubeLod). The descriptor handle is `0x20001800` and
  the result is fp16.
- The fragment resource table has its own sampler only (entry 3); the cube
  is in the position/varying tables.

## (c) vs-tex2d-cube: #11308, M.1 row 17

A vertex shader reads `sampler2D u_tex2d` (unit 1) and `samplerCube u_cube`
(unit 0): `pixel (32,32): 64 128 128 255`, `pixels differing: 0`, `fnv1a:
02c31dc5`, `verdict: PASS`. One `RUN_IDVS` with
`varying_srt0.varying_fau0.varying_tsd0.frag_srt1.frag_tsd0`.

- The position and varying resource tables (one table at the same address
  for both) have 7 entries. 0: buffer (FAU/uniforms, 32 bytes). 1: attribute
  `RG32F`, stride 8. 2: buffer (1 MiB). 3: samplers, `Size: 0x40` = **2
  sampler descriptors**. 4: textures, `Size: 0x40` = **2 texture
  descriptors**. 5 and 6: empty.
- The slot order follows the shader's sampler declaration order, not the
  units. Sampler/texture 0 is `u_tex2d`: `2D`, `RGBA8 UNORM RGBA`, 4x4,
  `Levels: 1`; its sampler is `Wrap Mode R: Repeat`, `T/S: Clamp to Edge`,
  `Seamless Cube Map: true`, `Minify/Magnify nearest: true`, `Mipmap Mode:
  None`. Sampler/texture 1 is `u_cube`: `Cube`, `RGBA8 UNORM RGBA`, 4x4,
  `Levels: 1`; its sampler is `Wrap Mode R/T/S: Clamp to Edge`, the rest the
  same.
- The vertex shader: `TEX_SINGLE....16.2d.zero` with handle `0x20001800`
  (texture 0, sampler 0) and `TEX_SINGLE....16.cube.zero` with handle
  `0x20011801` (texture 1, sampler 1). Both results are fp16 and are added
  in `FADD.v2f16`.
- The fragment resource table has entry 3 only (`Size: 0x20`, one sampler)
  and no texture entry.
