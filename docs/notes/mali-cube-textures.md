<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): cube-map textures, Mesa's design

Issue #1385, plan #152 item `notes-cube`, for the follow-up item
`tex-cube-113`: gl_test case 113 (`cubemap`) samples a cube map from the
vertex shader, `v_col = textureCubeLod(u_s, u_dir, u_lod)`
(`tools/shaders/es100/cubemap.vert:12`), 30 draws, one a (face, level) of a
16x16 map with 5 levels (`docs/notes/mali-texture-refusals.md`, section (a)).
Once the vertex-shader look-up lands (`tex-vs`), the case still stops at the
cube. This note is what Mesa 26.2.1's panfrost does for a cube on v10, what the
board's Mesa dump of case 113 holds, and where and how it lands in our tree.
It is a reading of Mesa's source, of the recorded dump and of ours; nothing
here was run on the board. A fact only the board can settle is marked **pin on
the board**. The vertex look-up itself is `tex-vs`
(`docs/notes/mali-texture-refusals.md`); the general lowering is
`driver/docs/nanopc-t6-mali.md`, "The general lowering (approved 2026-10-19)",
and `docs/notes/mali-lowering-map.md`.

## Citations

- Mesa: `M:<path>:<line>`, `<path>` under `src/` of the Mesa 26.2.1 tree
  (`MESA_PAN_TREE`, `$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`). The default
  Bifrost-BIR path is the one read (kraid runs only for a stage named in
  `PAN_USE_KRAID`, default none: `M:panfrost/compiler/pan_compiler.c:49-68`).
- Ours: `<path>:<line>` from the repository root, at `main` fc2f48f3.
- The board record: `D:<line>` is a line of the *decompressed*
  `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-cubemap.pandecode.xz`
  (`xz -dc`, 130,335 lines; `mesa/run-cubemap.txt`: renderer `Mali-G610
  (Panfrost)`, **Mesa 25.0.7-2+deb13u1**, not 26.2.1; 30 dump files, one a
  draw, PASS with 30 fetches right). Where 25.0.7 and 26.2.1 differ, the text
  says so.

Every citation carries its whole path.

## The answers

1. **Descriptor.** A cube is a `Texture` descriptor with `Dimension = Cube`
   (value 0), `Width`/`Height` of **one face**, `Levels` the chain's length,
   `Array size = 1` (the number of cubes: layers / 6) and `Surfaces` pointing
   at a table of **6 x levels** `Generic Plane`s (32 bytes each) in the order
   **face-major, level-minor** (`plane = face * levels + level`). Nothing else
   differs from the 2D descriptor we emit. The sampler is the 2D sampler with
   `Seamless Cube Map` set (we already set it,
   `src/mali/frame.rs:1143`).
2. **Compiler.** The coordinate `vec3 dir` becomes two staging words: `S` with
   the face index in its top 3 bits and `T`, by `CUBEFACE1`, `CUBEFACE2`,
   `CUBE_SSEL`, `CUBE_TSEL` and a short fp32 sequence (`FRCP`, two `FMA`,
   two clamps, one bitwise `MUX`). `TEX_SINGLE` then carries
   `dimension = cube` (value 3 of its 2-bit field) and, for
   `textureCubeLod`, the explicit-LOD word as a third staging register.
3. **Upload.** The GL core's storage is **level-major**, Mesa's is
   layer-major; the descriptor does not care, because every plane carries its
   own pointer and row stride. Face `f` of level `l` is at
   `level_off[l] + f * (h >> l) * pitch`, a multiple of 64 because the core's
   pitch is (5c0844cb): the cube is sampled **in place, with no copy**. Our
   `level_at` must count the faces in a level's size.
4. **Landing.** Four places refuse a cube today (section (d)); the
   descriptor builder and `TexSingle` gain a dimension; four ALU rows are new.
5. **Steps.** Section (f): the descriptor and the planes first (pinned to the
   board's words), the compiler rows next (pinned to a drm-shim reference of a
   vertex-shader cube look-up), the case on the board last.
6. **The fragment stage and a constant direction (cases 41, 114).** Section
   (g): Mesa's sequence is the same in either stage, a constant lane an
   immediate operand, the TEX in its register form; the references are
   `tools/mali/testdata/case41.*`, `case114_v.*`, `case114_f.*`.

## (a) The texture descriptor of a cube on v10

### The dump of case 113

The first draw's vertex-position resource table has the sampler as its entry 3
and the texture as its entry 4 (`D:114-160`; the varying table repeats both,
`D:1372-1406`; all 30 draws carry the same texture, 60 `Texture:` blocks in the
file). Decoded by pandecode (`D:160-178`):

```
Texture:    Type: Texture   Dimension: Cube   Sample corner position: false
  Normalize coordinates: false   Format (v7): RGBA8 UNORM RGBA
  Width: 16   Height: 16   Swizzle: 1672 (RGBA)   Texel interleave: false
  Levels: 5   Minimum level: 0   Minimum LOD: 0.000000   Sample count: 1
  Maximum LOD: 4.000000   Surfaces: 0x7fffffffd080   Array size: 1   Depth: 1
```

and as words, the 32 bytes at offset 0x1A0 of the memory block at
`0x7ffffffcb000` (`D:4171`, the line `0001A0`/`0001B0`, `D:4197-4198`):

| word | bytes (little endian) | value | fields |
|---|---|---|---|
| 0 | `02 00 C0 2E` | `0x2EC00002` | Type = 2 (Texture) bits 0-3; **Dimension = 0 (Cube)** bits 4-6; Sample corner position = 0 bit 8; Normalize coordinates = 0 bit 9; Format = `0xBB000` bits 10-31 (`RGBA8_UNORM` 0xBB << 12 \| order RGBA 0, `src/mali/genxml/v10.rs:381`, `:437`) |
| 1 | `0F 00 0F 00` | `0x000F000F` | Width - 1 = 15 bits 0-15; Height - 1 = 15 bits 16-31 |
| 2 | `88 06 04 00` | `0x00040688` | Swizzle = `0x688` (1672, RGBA) bits 0-11; Texel interleave = 0 bit 12; **Levels - 1 = 4** bits 16-20; Minimum level = 0 bits 24-28 |
| 3 | `00 00 00 04` | `0x04000000` | Minimum LOD = 0 bits 0-12; Sample count log2 = 0 bits 13-15; **Maximum LOD = 0x400 (4.0, 8.8 unsigned)** bits 16-28 |
| 4-5 | `80 D0 FF FF  FF 7F 00 00` | `0x00007FFF_FFFFD080` | **Surfaces** = the planes' address |
| 6 | `00 00 00 00` | `0` | **Array size - 1 = 0** bits 0-15 (one cube) |
| 7 | `00 00 00 00` | `0` | Depth - 1 = 0 bits 0-15 |

The field layout is `M:panfrost/genxml/v10.xml:1327-1345`; the words equal what
the decoded text says field by field (the bit positions are v10.xml's). They
are the 2D descriptor's words but word 0 (`Dimension` 0, not 2, so `0x2EC00002`
where a 2D RGBA8 texture has `0x2EC00022`) and the table behind word 4-5. `Array size = 1` for a cube is *not* six: Mesa divides
the layer count by 6 for a cube (`M:panfrost/lib/pan_texture.c:1198-1209`,
`M:panfrost/lib/pan_texture.c:1316`).

The sampler (entry 3, `D:130-155`, the 32 bytes at `0x1C0`, `D:4199`:
`01 99 A9 1E  00 00 FF 1F`): words 0x1EA99901, 0x1FFF0000; clamp-to-edge on
R, T and S, round-to-nearest-even, **`Seamless Cube Map: true`** (word 0 bit
23, `M:panfrost/genxml/v10.xml:1136`), normalized coordinates, minify and
magnify nearest, mipmap mode nearest, maximum LOD 31.996094. Mesa sets the bit
from the GL state, `cfg.seamless_cube_map = cso->seamless_cube_map`
(`M:gallium/drivers/panfrost/pan_cmdstream.c:231`), true for ES 2 and 3
(`M:gallium/drivers/panfrost/pan_screen.c:824-825`).

### How Mesa fills it

1. **The view.** `panfrost_create_sampler_view_bo` translates the target with
   `panfrost_translate_texture_dimension` (`PIPE_TEXTURE_CUBE` and
   `_CUBE_ARRAY` to `MALI_TEXTURE_DIMENSION_CUBE`,
   `M:gallium/drivers/panfrost/pan_resource.h:187-209`,
   `M:gallium/drivers/panfrost/pan_cmdstream.c:1756-1757`) and builds a
   `pan_image_view` with `first_layer`/`last_layer` of the sampler view: 0 and
   5 for a whole cube, since a cube's resource has six array layers
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1815-1840`).
2. **The descriptor.** `pan_sampled_texture_emit` packs `dimension =
   iview->dim`, the width and height of `first_level`, `levels = last_level -
   first_level + 1`, `array_size` and `surfaces`; `maximum_lod = levels - 1`
   (`M:panfrost/lib/pan_texture.c:1272-1325`; the lines are 1300, 1315, 1316,
   1319, 1322). `pan_texture_get_array_size` is `layer_count / 6` for a cube
   and asserts a multiple of 6 (`M:panfrost/lib/pan_texture.c:1198-1217`).
   `texel_interleave` is false for a linear image.
3. **The surfaces: faces are layers.** On v7 and later "faces as extra
   layers": one entry per (layer, level) with the layer outermost, the level
   innermost (`M:panfrost/lib/pan_texture.c:1089-1100`; the comment at
   `:1074-1077`). A cube's layer index is its face (0..5, +X -X +Y -Y +Z -Z),
   so the order is `[face 0: level 0..4][face 1: level 0..4] ... [face 5]`.
   v6 and older interleave level outermost and face inside
   (`M:panfrost/lib/pan_texture.c:1101-1126`); that is *not* the Valhall
   order. The dump agrees: the 30 planes are at `0x7fffffffd080 + 32 * (5 *
   face + level)` (`D:3286-3345`, below).
4. **One plane.** `emit_generic_plane` packs a `Generic Plane`
   (`M:panfrost/genxml/v10.xml:1358-1369`): clump ordering (linear), clump
   format (RAW32 for 4-byte texels), size, pointer, row stride, slice stride,
   the level's width and height (`M:panfrost/lib/pan_texture.c:425-459`).
   The pointer is `plane base + level offset + layer * array_stride`
   (`M:panfrost/lib/pan_texture.c:388-422`, the layer term at `:418`);
   `array_stride` is the whole mip chain of one layer rounded to 64 bytes: "Arrays
   and cubemaps have the entire miptree duplicated"
   (`M:panfrost/lib/pan_layout.c:141-143`; the image is `array_stride *
   array_size`, `:157`).

### The planes in the dump

`D:3276` starts the memory block at `0x7fffffffd000`; the planes start at
offset `0x80` (`D:3286`), 30 of them, 32 bytes each, ending at `0x440`
(`D:3345`). Plane 0 (face 0, level 0) is

```
000080  1A 02 00 02  00 04 00 00  00 B0 FD FF  FF 7F 00 00
000090  40 00 00 00  00 00 00 00  C0 07 00 00  0F 00 0F 00
```

| word | value | fields |
|---|---|---|
| 0 | `0x0200021A` | Type = 0xA (Plane) bits 0-3; Plane type = 1 (Generic) bits 4-7; Clump ordering = 2 (Linear) bits 8-11; Clump format = 2 (RAW32) bits 24-31 |
| 1 | `0x400` | Size = 1024 (16 rows of 64 bytes) |
| 2-3 | `0x7FFFFFDB000` | Pointer |
| 4 | `0x40` | Row stride = 64 |
| 5 | `0` | (row stride high) |
| 6 | `0x7C0` | Slice stride = 1984 |
| 7 | `0x000F000F` | Width - 1 = 15, Height - 1 = 15 |

The 30 planes, by pointer (`D:3286-3345`, decoded `D:179-`): face 0 at
`...db000` (size 1024, 16x16), `...db400` (512, 8x8), `...db600` (256, 4x4),
`...db700` (128, 2x2), `...db780` (64, 1x1); face 1 at `...db7c0`, `...dbbc0`,
`...dbdc0`, `...dbec0`, `...dbf40`; face 5 level 0 at `...dd6c0` (`D:1162`)... In short,
face `f` starts at `0x7ffffffdb000 + f * 0x7C0`: the chain of one face is
`1024 + 512 + 256 + 128 + 64 = 1984` bytes (a multiple of 64, so no rounding),
and **every level, down to 1x1, has row stride 64** (Mesa's
`ALIGN_POT(width * blocksize, 64)`, `M:panfrost/lib/pan_mod.c:761-766`, 64 is
the minimum). `Size` is `row stride * rows` of the level.

The slice stride of 1984 is the **25.0.7** driver's. Mesa 26.2.1's
`get_linear_or_u_tiled_plane_props` writes 0 for a non-3D, single-sample image
(`M:panfrost/lib/pan_texture.c:419-421`), so the words in 26.2.1 would differ in
word 6 (we already write 0 for a 2D plane, `src/mali/frame.rs:1153-1170`).
Whether the texture unit reads the field for a cube stays **open**, see
"The open questions, from the recording" below (the recording has 1984 in
every plane and no run with another value; the drm-shim run of step 2 shows
which value 26.2.1 writes).

## (b) The compiler: the cube coordinate and the TEX dimension

Mesa lowers a cube look-up before the instruction is built, in
`pan_nir_lower_tex` (run from the Bifrost NIR pipeline,
`M:panfrost/compiler/bifrost/bifrost_nir.c:963`; for arch >= 9 the pass is
`va_lower_tex_instr`, `M:panfrost/compiler/pan_nir_lower_tex.c:1078-1121`).

1. **The coordinate becomes two words.** In `va_lower_tex` a cube (`coord_comps
   == 3`) takes `build_cube_desc(srcs.coord)` and puts its two channels in
   staging registers S and T; a cube has no R (`M:panfrost/compiler/pan_nir_lower_tex.c:825-831`).
   A cube `txf` is rewritten as a 2D array first (`:819-823`; not our case).
   `textureCubeLod` is `txl`, so the LOD goes in the explicit slot (zero:
   `ZERO_LOD`) like a 2D `txl` (`:928-934`, the `lod_mode` and `VA_TEX_SR_LOD`
   lines); the staging words in order are S, T, (LOD)
   (`enum valhall_tex_sreg`, `:787-798`, compacted at `:959-962`).
2. **`build_cube_coord2_face`** (`M:panfrost/compiler/pan_nir_lower_tex.c:68-100`),
   for `dir = (x, y, z)`:
   ```
   (max_xyz, face) = cubeface_pan(x, y, z)         // :75-77
   s = cube_ssel_pan(z, x, face)                    // :79
   t = cube_tsel_pan(y, z, face)                    // :80
   k = 0.5 / max_xyz                                // :95  (fdiv)
   s = fsat(ffma(s, k, 0.5)); t = fsat(ffma(t, k, 0.5))   // :97-98
   return (s, t, face)
   ```
   The comment (`:82-92`) gives the GLES formula and the reason for the FMA +
   saturate form (NaN and infinity).
3. **`build_cube_desc`** (`:103-128`, the select at `:125`) packs the face into S: `s =
   bitfield_select(0x1FFFFFFF, s, face)`, the low 29 bits from `s`, the top 3
   from `face` (the face index "is preshifted", i.e. already in bits 29-31,
   by `CUBEFACE2`). The struct in the comment is `{ float s : 29; unsigned
   face : 3; float t : 32; }`.
4. **The hardware ops** (`M:panfrost/compiler/bifrost/bifrost_compile.c:2109-2138`):
   `cubeface_pan` is **two instructions** on Valhall, `CUBEFACE1` (the
   maximum of the absolute values) and `CUBEFACE2_V9` (the face index), the
   results collected as a pair (`:2119-2124`); `cube_ssel_pan` and
   `cube_tsel_pan` are `CUBE_SSEL` and `CUBE_TSEL`. Operands:
   `CUBEFACE1`/`CUBEFACE2_V9` (x, y, z), `CUBE_SSEL` (z, x, face),
   `CUBE_TSEL` (y, z, face), each with absolute and negate on the
   coordinates (`M:panfrost/compiler/bifrost/valhall/ISA.xml:3198-3216`,
   `:3158-3180`; the intrinsics `M:compiler/nir/nir_intrinsics.py:1767-1776`).
5. **The ALU rows a cube needs**, beyond the look-up:

   | NIR | Valhall | in our ISA table | in our BIR (`src/shaderc/online/mali/bir.rs`) |
   |---|---|---|---|
   | `cubeface_pan` | `CUBEFACE1`, `CUBEFACE2` | `Op::Cubeface1/2` (`src/shaderc/online/mali/isa/v10.rs:287-289`, opcodes 0xC0, 0xC1) | missing |
   | `cube_ssel_pan`, `cube_tsel_pan` | `CUBE_SSEL`, `CUBE_TSEL` | `Op::CubeSsel/Tsel` (`isa/v10.rs:291-293`, opcode 0xBC, bit 24 = 0 / 1) | missing |
   | `fdiv(0.5, m)` | `FRCP.f32` then `FMA` (`lower_fdiv`, `M:panfrost/compiler/bifrost/bifrost_compile.c:3398-3403`) | `FrcpF32` | present (`bir.rs:183`) |
   | `ffma` | `FMA.f32` (`M:panfrost/compiler/bifrost/bifrost_compile.c:2869-2871`) | `FmaF32` | present (S2's) |
   | `fsat` | a clamp, `bi_fclamp_to` with `CLAMP_0_1` (`:2881-2885`) | `FaddF32` with the clamp | S4's `fsat` row |
   | `bitfield_select` | `MUX.i32` in bit mode, `MUX(s1, s2, mask)` (`:3419-3426`) | `MuxI32` (`isa/v10.rs:719`) | missing |

   Our ISA table has the four cube opcodes already, so the work is the BIR
   `Op`s, their sources and their packer arms; each is one shim reference
   (section (f)), as S4 does for its rows. The mask `0x1FFFFFFF` is a 32-bit
   constant source: it comes from the FAU constant words or `MOV`
   (the same question as S4's other immediates).
6. **The TEX dimension.** `bi_emit_tex_valhall` passes `valhall_tex_dimension
   (tex->sampler_dim)` to `bi_tex_single_to`: `GLSL_SAMPLER_DIM_CUBE` gives
   `BI_DIMENSION_CUBE` (`M:panfrost/compiler/bifrost/bifrost_compile.c:3461-3485`,
   `:3489-3531`). The Valhall ISA's `Dimension` enum is `1d, 2d, 3d, cube` in
   order, a 2-bit field at bit 28 of `TEX_SINGLE`
   (`M:panfrost/compiler/bifrost/valhall/ISA.xml:477-482`,
   `M:panfrost/compiler/bifrost/valhall/valhall.py:408`), so **cube is 3**; our
   table says the same (`E_TEX_DIMENSIONALITY_M`, `isa/v10.rs:149`,
   `M_DIMENSION`, `:12060`). The "0 for a cubemap" comment in
   `M:panfrost/compiler/bifrost/bifrost.h:513` is Bifrost's encoding, not
   Valhall's. The descriptor's `Dimension` (0) and the instruction's
   (3) are two different numbers for the same thing.
7. **What a cube look-up does not have:** no array index, no offset, no
   compare in 113 (`array_enable`, `texel_offset`, `compare_enable` stay 0 as
   in `src/shaderc/online/mali/pack.rs:341-344`); `lower_txd_cube_map` removes
   explicit-gradient cubes (`M:panfrost/compiler/pan_compiler.c:125`).

## (c) The upload: six faces on yakogl's 64-byte rows

### What the GL core stores

`chain_layout_npot` lays a cube out **level-major**: "level i is `layers`
faces of (w >> i) x (h >> i)" (`src/gl/texture.rs:1509-1514`, the loop
`:1531-1553`): the six faces of level `l` are stacked one under the other on
**one row pitch** (`chain_pitch`, level 0's, rounded to the backend's
`linear_row_align`, 64 for the Mali, `src/gl/texture.rs:1500-1503`, 5c0844cb),
each level starting on a 64-byte boundary (`mip_level_align`
`src/mali/backend.rs:2465`). So

```
level_off[l+1] = align64(level_off[l] + pitch * (h >> l) * 6)
face f, level l  at  base + level_off[l] + f * (h >> l) * pitch
```

(`cube_faces_are_stacked_rows_of_one_storage`, `src/gl/texture.rs:10055`:
the face's rows start at `face * lh`; `src/gl/texture.rs:2672`, `:3493`). For the
case's 16x16 RGBA8 cube the pitch is 64 bytes (16 * 4, already a multiple), a
level is `6 * (16 >> l) * 64` bytes. Mesa's layout is layer-major
(face contiguous with its chain, `array_stride` 1984 in the dump). **The two
need not agree**: the descriptor names each surface by pointer.

### The descriptor over this storage

For face `f`, level `l` the plane is: pointer `base_gpu + level_off[l] + f *
(h >> l) * pitch`, row stride `pitch`, size `pitch * (h >> l)`, width and
height `max(w >> l, 1)`, `max(h >> l, 1)`, clump RAW32, linear, slice stride 0
(see the 25.0.7/26.2.1 note above). Each pointer is a multiple of 64 because
`level_off[l]` and `pitch` are. The texture unit reads a 64-byte-multiple row
stride and nothing else (the 2026-10-19 measurement in 5c0844cb); a face start
that is 64-aligned and a `pitch` that is 64-aligned are what it needs. The GL
core therefore needs **no change** for a power-of-two cube; the planes' table
is `6 * levels * 32` bytes (960 for 113), written in `face * levels + level`
order, and the texture descriptor's `Surfaces` is its address.

### What the back end gets (`GlesDraw` / `GlesTextureBind`)

With `native_layered_textures: false` (`src/mali/backend.rs:2470`) the core
hands the whole stack as one 2D image: `type_ = GLES_TEXTYPE_CUBE`,
`layers = 6`, `width` a face's, **`height = 6 * face height`**, `stride_texels`
the level pitch, `bytes` the stack
(`src/gl/draw.rs:3795-3830`, `src/backend.rs:764-792`). So the face height is
`height / layers`. The alternative, `native_layered_textures: true` (the
GC7000's: `height` is a face's), also changes `npot_mip_chains`
(`src/gl/texture.rs:1036-1038`) and the 3D layout (`src/gl/texture.rs:1240`),
so non-power-of-two chains would start to exist for the Mali and meet
`NOT_BUILT_TEXTURE_NPOT_CHAIN`. **Recommendation: keep `false`** and divide by
`layers` in `level_at`; flipping it is a separate decision. A
non-power-of-two cube keeps one level on the Mali today (the core's rule for
`layers > 1` with `mip_level_align != 0`), which the descriptor's `levels` must
follow (`levels_of`).

The vertex stage's textures are in `GlesDraw::vs_textures`
(`src/gl/draw.rs:3810-3814`); `tex-vs` makes the vertex resource table's
TEXTURE entry, the cube's table is built by the same code.

## (d) Where it lands in our code

The cube is refused in these places today, in the order case 113 meets them:

| refusal | where | removed by |
|---|---|---|
| `MaliError::TexVertex` "a texture look-up in a vertex shader is not lowered yet" | `src/shaderc/online/mali/nir.rs:837` (`IrOp::Tex { .. } \| IrOp::TexSize { .. }`), message `src/shaderc/online/mali/mod.rs:307` | `tex-vs` |
| the IR: `textureCubeLod` is `{Cube, Lod}`, which `es100_tex()` maps to `None` (only `{Cube, Implicit}` is `Es100Tex::TexCube`) | `src/shaderc/online/ir.rs:501`, `:519-520` | `tex-vs` / this item (the IR form) |
| `MaliError::TexKind` "...(a bias, a level, a projection, a cube map, a size query)..." | `src/shaderc/online/mali/nir_fs.rs:592` (the fragment look-up accepts `{Tex2D, None}` only), message `mod.rs:308-311`; test row `src/shaderc/online/mali/tests.rs:876` | not needed for 113 (its FS is a pass-through); a `textureCube` in a fragment shader later |
| `NOT_BUILT_TEXTURE_TYPE` "a texture other than one 2D image (a cube map, an array, a 3D texture)" | raised in `sampled_texture`, `src/mali/backend.rs:1602` (`t.type_ != GLES_TEXTYPE_2D \|\| t.layers > 1`), text `:204-205`; test `src/mali/backend_tests.rs:984-986` | **this item** |
| the descriptor builders know only 2D: `SampledTexture` (one `level` array, no face), `plane(l)`, `texture(surfaces)` with `TextureDimension::_2D` and `array_size: 1` | `src/mali/frame.rs:1049-1190`; the table writer `src/mali/frame.rs:2455-2465` (`tex.levels` planes) | **this item** |
| `Op::TexSingle` has no dimension, and the packer writes `TEX_DIM_2D` (1) | `src/shaderc/online/mali/bir.rs:263-274`, `src/shaderc/online/mali/pack.rs:87`, `:337-346` | **this item** (a `dim` field, cube = 3) |

The functions to change, by name: in `src/mali/backend.rs`, `sampled_texture`
(`:1565-1660`) and its `checks` array (`:1601-1607`), `level_at`
(`:1485-1507`, the level's size becomes `pitch * rows * layers` and the plane's
offset gets the `face * rows * pitch` term), `levels_of`, `needs_copy` and
`storage_end` (`:1510-1530`, which assume one image); in `src/mali/frame.rs`,
`SampledTexture` and its `plane`/`texture` methods (a `face_count`, a
`dimension`) and the table writer; in `src/shaderc/online/mali/`, `bir.rs`
(`Op::TexSingle { dim }`, the four cube `Op`s), `pack.rs` (the arms), `nir.rs`
(the vertex walk: the cube look-up as the `build_cube_desc` sequence) and
`ir.rs` (the IR form). `src/mali/backend.rs:2470` stays.

## (e) The recorded reference

Section (a) quotes the descriptor, the sampler and the planes of the board's
Mesa 25.0.7 dump of case 113. Further facts of that dump:

- 30 dump files, one a draw, the same descriptor in each (the case aims each
  draw at one texel of one face and level with `u_dir` and `u_lod`, which are
  uniforms, so the program and the descriptors do not change);
  `run-cubemap.txt`: each draw `ok` with the colour `0xff40c000` for (face 0,
  level 0, texel (12, 4)), `0xff206001` for (0, 1), ... `0xff40c010` for (1, 0):
  the texel carries its face and level.
- The vertex job is an IDVS job whose position and varying tables both name
  the cube as entry 4 and the sampler as entry 3 (`D:81-160`, `D:1327-1406`);
  there is no fragment texture (the fragment shader passes `v_col`).
- The dump holds the command stream and the memory, **not** the vertex
  shader's code or disassembly (pandecode does not print it, `D:1-80` is the
  command stream): the instruction words of the cube sequence need the drm-shim
  run of section (f), not this file.
- Mesa 25.0.7 is not 26.2.1: the slice stride (above) is the known difference;
  everything else in the descriptor is what the 26.2.1 code in (a) writes.

### The open questions, from the recording

Read on 2026-10-21 from `xz -dc mesa-cubemap.pandecode.xz` (130,336 lines by
`split('\n')`, the last empty), `mesa-cubemap.stdout`, `run-cubemap.txt` and
`cases.c` (`case_cubemap`, `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/cases.c:702-785`).
`D:` lines are of the decompressed file; the first draw's block is `D:114-1314`,
the other 29 repeat it.

| question | answer | where |
|---|---|---|
| Slice stride of a plane, 0 or 1984 | **1984 in all 30 planes of every draw** (1,800 `Slice stride: 1984` lines in the file, no other value), the same for a 16x16 level and for the 1x1 one, so it is the layer stride (one face's whole chain: `1024+512+256+128+64`), not a per-level size. Raw word 6 of plane 0 is `C0 07 00 00` | `D:214`, `D:252`, ... `D:1314`; raw `D:3287`, last plane `D:3345` |
| Does the texture unit need it | **Not answered.** Every recorded sampling used 1984 and all were right; a run with 0 is not in the record. The pointer of each plane already includes the face offset (below), so the field is redundant for the address, but nothing shows the unit ignores it. 26.2.1 writes 0 (`M:panfrost/lib/pan_texture.c:419-421`). Our level-major storage has no single face stride (a face is `rows * pitch`, which differs per level), so 1984 cannot be written for it anyway | settled by: case 113 on the board with the planes' slice stride 0 (step 5 below; if wrong, retry with the level's `rows * pitch * 6`) |
| Face pointer | Each face-and-level plane carries its **own full 64-bit pointer** in word 2-3 (`Pointer:`): `0x7ffffffdb000 + 1984 * face + level_off`; no base-plus-face is computed by the unit. Face 0 `D:212`, face 1 `D:402` (`0x7ffffffdb7c0`), face 2 `D:592` (`...dbf80`), face 3 `D:782` (`...dc740`), face 4 `D:972` (`...dcf00`), face 5 `D:1162` (`...dd6c0`) | `D:212` ... `D:1314` |
| Pointer alignment | The recorded face pointers are 64-byte aligned and **not** 4 KiB aligned (`0x7c0` steps); the draws sampling faces 1-5 are all right. So a 64-aligned face start is enough on the Mali G610 for a Mesa-written descriptor. This closes the "face pointer not on a 4 KiB boundary" item of step 5 | `D:402`...`D:1314`, `run-cubemap.txt` |
| Face order of the surfaces | face-major, level-minor, as in answer 1: planes 0-4 are face 0 levels 0-4 (`D:212`-`D:364`, `...db000, db400, db600, db700, db780`), plane 5 is face 1 level 0 (`D:402`), plane 29 is face 5 level 4 (`D:1314`, `...dde40`, size 64). The raw table is `D:3286-3345`, 32 bytes a plane | as listed |
| Dimension field | `Dimension: Cube` (word 0 bits 4-6 = 0, raw `02 00 C0 2E`), in 60 of 60 texture blocks; `Width: 16`, `Height: 16` are one face's | `D:163`, `D:167-168`, raw `D:4197` |
| Array size and depth | `Array size: 1` (raw word 6 = 0, so the field is `size - 1`), `Depth: 1`, in 60 of 60 blocks: one cube, not six layers | `D:177-178`, raw `D:4198` words 6-7 |
| Sampler's cube / seamless bits | The sampler has **one** cube bit, `Seamless Cube Map: true` (word 0 bit 23; raw `01 99 A9 1E`); there is no separate "cube" flag in the sampler, the cube-ness is the texture's `Dimension`. all 90 `Seamless Cube Map` lines of the file read true | `D:138`, `D:1384`, `D:2598`, raw `D:4199` |
| Face colours | Yes, every one of the 30 readbacks shows the expected texel: the texel is `0xff` `y*16`.. `x*16` .. `face*16 + level` (blue byte high nibble = face, low = level), e.g. `0xff40c010` for face 1 level 0, `0xff000054` for face 5 level 4; `cubemap wrong fetches = 0  want 0  ok`, `verdict case 113 cubemap: PASS rc 0, read-backs 30` | `mesa-cubemap.stdout` lines `gl_test:   cubemap: face F level L ...`; readbacks `mesa-cubemap-rb01..30.pam` (fnv1a in the stdout) |
| The vertex shader's cube instructions and explicit LOD | **Not in the record**: pandecode here prints the command stream and memory, not the shader's code (see above). The instruction words, and how the LOD word is staged, are in step 2's drm-shim `.trace`; whether the hardware honours the explicit LOD in a vertex look-up is the board run of step 5 | step 2, step 5 |

## (f) The steps for `tex-cube-113`

Order: after `tex-vs` (the vertex-stage tables and `TEX_SINGLE` for 2D). The
frame side can be written first, since it needs no compiler. Each step is
checked against the record named, and `tools/mali/**` and `src/mali/**` are
touched by `frame-msaa-zs` (running): the steps wait for it.

1. **Pin the planes and the descriptor, host only.** Cut the dump's first
   draw: `tools/mali/board_dump_cut.py mesa-cubemap.pandecode
   7fffffffd000 7ffffffcb000` (the decompressed dump; the script needs the
   plain file) into `tools/mali/board-dump/cube-2026-10-20.txt`, and add a test
   to `src/mali/frame_tex_tests.rs` (beside `TEX`, `MIP`, `FMT`, `:24`, `:413`,
   `:529`) that builds a 16x16, 5-level cube `SampledTexture` and compares the
   30 planes (`0x80..0x440` of the first block) and the texture words
   (`0x1A0..0x1C0` of the second) — except for the slice-stride word, which the
   test takes from step 2 (the recording's word 6 is 1984 in every plane,
   `D:3287`; our planes write 0, so that word is the one allowed difference).
2. **Pin Mesa 26.2.1's words on the shim.** `tools/mali/gl_ref.py` binds only
   `sampler2D` (`SAMPLERS = ("sampler2D",)`, `:72`, the binding loop
   `:217-229`: a 2x2 `GL_TEXTURE_2D`). Add `samplerCube`: a
   `GL_TEXTURE_CUBE_MAP` of 6 faces, `glGenerateMipmap` or 5 levels of 16x16,
   nearest, clamp. Then a case in `tools/mali/testdata/cases.txt`
   (`cube_vs  cube_vs.vert  cube_vs.frag  triangles  vs.position=...
   vs.varying=...`, the VS of case 113 with `u_dir`, `u_lod` set by
   `--uniform-values`, the FS `tri.frag`-like) and its `.expect.json` by
   `tools/mali/ref_cases.py` / `drm_shim_ref.sh run` (the two new files and
   `cases.txt`; the pattern of `sel_t_tex`, `fvg_texture_copy_unclipped`).
   The `.trace` it keeps is a pandecode with the memory blocks: read the
   plane words (slice stride) and the texture descriptor from it, and compare
   with this note.
3. **The frame side.** `SampledTexture` takes a face count and a dimension;
   `texture()` writes `Dimension = Cube`, `array_size 1`; the planes are
   `6 * levels`, `face * levels + level`; `sampled_texture` drops the cube from
   its `checks` (the `layers > 1` test stays for arrays and 3D) and passes the
   levels' face-major pointers; `level_at` counts the faces. A unit test per
   the step 1 words; the backend test row `src/mali/backend_tests.rs:984` turns
   from refused to built, with a 3D texture and a 2D array still refused.
4. **The compiler.** The IR form of `textureCubeLod` (`{Cube, Lod}` ->
   `Es100Tex::TexCubeLod`, or the existing `Tex` with a cube dim and an LOD
   source), the vertex walk's cube arm emitting the sequence of section (b)
   (six BIR ops and the saturates, then `TexSingle { dim: cube,
   explicit LOD }`), `bir.rs` and `pack.rs` for the four cube ops, the packer's
   `dimensionality` = 3, one shim reference each for `CUBEFACE1`, `CUBEFACE2`,
   `CUBE_SSEL`, `CUBE_TSEL` and the whole sequence: **byte-equal** to
   step 2's `vs.position` (and `vs.varying`).
5. **The board, last.** Case 113 on the NanoPC-T6 through `panthor-gltest`
   (the board session of #1385): 30 fetches right, 0 kernel messages, against
   the Mesa pictures `mesa/mesa-cubemap-rb01..30.pam`. **Pin on the board**
   what the recording left open (section (e), "The open questions, from the
   recording"): the plane's slice stride, written 0 (the recording only has
   1984), and the explicit LOD of a vertex look-up (`tex-vs`). The face
   pointer question is settled: Mesa's are 64-aligned and not 4 KiB aligned,
   and sample right (`D:402`), so ours need no more.
6. **The issue.** A comment on #1385 with the commit and the shim words;
   `tools/cts/RESULTS.md` / `STATUS.md` row for 113 only after the board
   run.

What stays refused after these steps: a cube map array and a cube in the
fragment shader (`TexKind`: `textureCube` with an implicit LOD needs
`TEX_SINGLE`'s computed LOD with the `CUBE_*` sequence in the fragment stage,
the same ops, different staging); a 3D texture and a 2D array
(`NOT_BUILT_TEXTURE_TYPE`); a non-power-of-two cube with a chain; a block
format cube (the plane's size and row stride differ).

## (g) A cube at a constant direction (114) and in the fragment stage (41)

The study for plan #153's `fs-cube` item (#1385): gl_test case 114
`cubelod` and case 41 `es3cube`. Nothing in `src/` was changed for it.

**Citations in this section.** Mesa as `M:` above (the 26.2.1 tarball, whose
lines do not move). Our code is named by file and symbol (a function, a
match arm, an enum variant), **never by line**: the lines of `nir_fs.rs`
move with every lowering item, a symbol does not. The state read is main
71c586bc. Every Mesa instruction quoted is copied from a committed
`.expect.json` (its `regions.<program>.text`), named with it.

### The references

| file | gl_test | shaders | drawn with |
|---|---|---|---|
| `tools/mali/testdata/case114_v.{vert,frag,expect.json}` | 114, `prog[0]` (`case_cubelod`, `driver/tools/pvrlab/gl_test.c`) | `tools/shaders/es300/cubelodv.{vert,frag}`, byte-identical | `--uniform-values` |
| `tools/mali/testdata/case114_f.{vert,frag,expect.json}` | 114, `prog[1]` | `tools/shaders/es300/cubelodf.{vert,frag}`, byte-identical | `--uniform-values` |
| `tools/mali/testdata/case41.{vert,frag,expect.json}` | 41 (`case_es3cube`) | `tools/shaders/es300/cube.{vert,frag}`, byte-identical | `--uniform-values` |

Each was drawn on Mesa 26.2.1's drm-shim with the committed tools, as
`docs/notes/mali-builtin-inputs.md` draws its `#version 300 es` pairs:

```
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/case41.vert tools/mali/testdata/case41.frag /tmp/case41 --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract /tmp/case41.trace --draw \
    > tools/mali/testdata/case41.expect.json
```

(`case114_v`, `case114_f` the same.) `gl_ref.py` asks for an ES 2 EGL context
(`EGL_CONTEXT_CLIENT_VERSION` 2) and Mesa makes it at the highest ES version
the driver has: the requested version is only a minimum
(`M:mesa/state_tracker/st_manager.c:1062-1065`, the context's version from
`_mesa_compute_version`, `M:mesa/state_tracker/st_context.c:685`), so
`#version 300 es` compiles with no option. A `samplerCube` is bound to
`gl_ref.py`'s cube (case 113's 16x16, five levels); the programs do not depend
on the texture, the descriptors in the trace are 113's, and `extract` keeps
only the programs.

The corpus verdicts at main before `fs-cube` (`tools/mali/corpus-verdicts.txt`;
after it, the end of this section):

| shader | verdict |
|---|---|
| `tools/shaders/es300/cube.vert` | `ok` |
| `tools/shaders/es300/cube.frag` | `MaliError::TexKind`, "a texture look-up other than texture2D(sampler2D, coordinate) ..." |
| `tools/shaders/es300/cubelodv.vert` | `MaliError::TexVertexConstant`, "a vertex shader look-up at a coordinate with a constant lane ..." |
| `tools/shaders/es300/cubelodv.frag` | `ok` |
| `tools/shaders/es300/cubelodf.vert` | `MaliError::VaryingNotFromAttribute`, "a varying lane computed from no attribute ..." |
| `tools/shaders/es300/cubelodf.frag` | `MaliError::TexKind` |

### The coordinate lowering, as Mesa emits it

`va_lower_tex` replaces a cube's `vec3` direction by `build_cube_desc`'s two
words (`M:panfrost/compiler/pan_nir_lower_tex.c:825-829`) in either stage: the
pass does not look at the stage. `build_cube_coord2_face`
(`M:panfrost/compiler/pan_nir_lower_tex.c:68-101`) and `build_cube_desc`
(`M:panfrost/compiler/pan_nir_lower_tex.c:117-128`) are section (b)'s sequence. `case114_f.expect.json`,
`fs.fragment`, `textureLod(u_s, v_dir, u_lod)` with `v_dir = (1.0, y, z)`
(highp):

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
CUBEFACE1.discard r2, 0x3F800000, r0, r1        ; max(|x|, |y|, |z|), x = 1.0
FRCP.f32 r2, r2^
FMA.f32 r2, 0x38000000.h1, r2^, 0x0.neg         ; k = 0.5 * (1 / max)
CUBEFACE2_V9 r3, 0x3F800000, r0, r1             ; the face, in bits 29-31
CUBE_TSEL r0, r0^, r1, r3                       ; (y, z, face)
CUBE_SSEL r1, r1^, 0x3F800000, r3               ; (z, x, face)
FMA.f32.clamp_0_1 r1, r1^, r2, 0x38000000.h1    ; s = sat(ssel * k + 0.5)
FMA.f32.clamp_0_1 r0, r0^, r2^, 0x38000000.h1   ; t = sat(tsel * k + 0.5)
MUX.i32.bit r1, r1^, r3^, u2.w1                 ; S = (s & 0x1fffffff) | face
MOV.i32 r2, u3.w0
FMA.f32.clamp_m1_1 r2, u2.w0, r2^, 0x0.neg      ; the level: fsat_signed(lod / 16)
FMA.f32 r2, r2^, u3.w1, 0x0.neg                 ; * 4096
F32_TO_S32.rtz r2, r2^
U16_TO_U32 r2, r2^.h0                           ; u2u32(f2i16(...))
MOV.i32 r4, r1^
MOV.i32 r5, r0^
MOV.i32 r6, r2^                                 ; staging S, T, level
MOV.i32 r0, u4.w0
MOV.i32 r1, 0x0                                 ; the handle pair
TEX_SINGLE.slot1.skip.reserved.32.cube.explicit.wait0126 @r0:r1:r2:r3, @r4:r5:r6, [r0^:r1^]
```

(the FAU words: `u2.w1` = `0x1fffffff`, `u3.w0` = `0x3d800000` = 1/16,
`u3.w1` = `0x45800000` = 4096, `u2.w0` the uniform `u_lod`.) Three facts the
fragment-stage look-up adds to section (b):

1. **The LOD mode follows the GLSL call, as in the vertex stage**
   (`M:panfrost/compiler/pan_nir_lower_tex.c:927-946`): a level that is not
   the constant 0 is `EXPLICIT`, three staging words, the level as
   `u2u32(build_sfixed_8_8(lod))`
   (`M:panfrost/compiler/pan_nir_lower_tex.c:932-933`,
   `M:panfrost/compiler/pan_nir_lower_tex.c:136-141`; 114's `u_lod`);
   a constant 0 (`nir_scalar_is_zero`,
   `M:panfrost/compiler/pan_nir_lower_tex.c:929-930`) is `ZERO_LOD`, two
   words (41's `textureLod(cube, d, 0.0)`: `cube.zero`); a fragment
   `textureCube` without a level would be `COMPUTED_LOD`, two words
   (`M:panfrost/compiler/pan_nir_lower_tex.c:944-945`).
2. **A constant lane stays an operand.** `cubeface_pan`, `cube_ssel_pan`
   and `cube_tsel_pan` are intrinsics, which NIR's constant folding does not
   evaluate, so the constant 1.0 reaches the instructions as a source.
   `va_lower_constants` encodes it from the ISA's immediate table
   (`va_lut_index_32`, `M:panfrost/compiler/bifrost/valhall/va_lower_constants.c:24-28`):
   `0x3F800000`, no FAU word. It is the first source of `CUBEFACE1` and
   `CUBEFACE2_V9` and the middle one of `CUBE_SSEL`.
3. **The register form of TEX, no varying-texture fusion.** The direction is
   loaded by `LD_VAR_BUF_IMM` (`src_f32`) into registers and the TEX reads
   staging registers. The fused `VAR_TEX_*` instructions are
   `unused="true"` in the Valhall ISA
   (the eight `<ins>` entries at
   `M:panfrost/compiler/bifrost/valhall/ISA.xml:3780-3959`), and the pass that fuses (`bi_opt_message_preload`) runs only for
   `ctx->arch == 7` (`M:panfrost/compiler/bifrost/bifrost_compile.c:4460-4467`).
   `sel_t_tex.expect.json` (a 2D look-up at a varying) is the same: `@r0:r1`
   loaded, then `TEX_SINGLE ... @r0:r1, @r0:r1`.

### Case 114, the vertex stage (`case114_v`)

`textureLod(u_s, vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95), u_lod)` in the
vertex shader. `case114_v.expect.json`, `vs.varying`:

```
LD_ATTR_IMM.v2.f32.slot0.wait0 @r0:r1, r60^, r61^, index:0x0, table:0x1
FMA.f32 r1, r1^, u4.w1, 0x0.neg                 ; a_pos.y * 0.95 (u4.w1 = 0x3f733333)
FMA.f32 r0, r0^, u4.w1, 0x0.neg                 ; a_pos.x * 0.95
CUBEFACE1 r2, 0x3F800000, r1, r0
CUBEFACE2_V9 r3, 0x3F800000, r1, r0
CUBE_SSEL r4, r0, 0x3F800000, r3
CUBE_TSEL r0, r1^, r0^, r3
...                                             ; FRCP, FMA, two clamps, MUX.i32.bit, the level, as above
TEX_SINGLE.slot1.reserved.32.cube.explicit @r0:r1:r2:r3, @r4:r5:r6, [r0^:r1^]
```

The sequence is `cube_vs.expect.json`'s (113's, all lanes uniforms) with the
constant lane as an immediate. Ours refuses it in the vertex walk
(`src/shaderc/online/mali/nir.rs`, the `IrOp::Tex` arm of the walk, its
`cube` branch: any `Val::Const` lane of the direction returns
`MaliError::TexVertexConstant`). The 2D branch's check beside it stays:
there a constant lane would be a staging word, which no vertex reference
draws.

### Case 114, the fragment stage (`case114_f`)

The vertex shader writes `v_dir = vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95)`.
Mesa's link propagates the constant lane into the fragment shader
(`nir_opt_varyings`, "Constant, uniform, UBO load, and uniform expression
propagation", `M:compiler/nir/nir_opt_varyings.c:118-124`) and drops it
from the varying: `case114_f.expect.json`'s `vs.varying` stores two words
(`STORE.i32.estream.slot2 @r1`, `STORE.i32.estream.slot0.end @r0`), its
`fs.fragment` loads `.v2` and reads `0x3F800000` for x (the listing above).
So 114's fragment pair needs two things: the cube in the fragment stage
(below) and that link propagation, which ours refuses at the vertex side
(`src/shaderc/online/mali/nir.rs`, the varying-store loop of the vertex
walk: its `Val::Const(_)` arm returns `MaliError::VaryingNotFromAttribute`).
The link is the one `docs/notes/mali-varying-constants.md` reads for cases
63, 78 and 101 ("A constant lane is not a varying"); case 101 `cubebias`
(`v_dir = vec3(1.0, -a_pos.y, -a_pos.x)`, a cube with a bias) has the same
`CUBEFACE1 r2, 0x3F800000, r0, r1` in its fragment shader, so one lowering
of the link serves 101 and 114.

### Case 41, the fragment stage (`case41`)

`vec3 d = axis + vec3(v_xy.x, v_xy.y, 0.0) * 0.8; textureLod(cube, d, 0.0)`,
mediump, `axis` a uniform: no lane is constant. `case41.expect.json`,
`fs.fragment`:

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
V2F32_TO_V2F16.discard r0, r0^, r1^             ; mediump: v_xy to fp16
MOV.i32 r1, u2.w0                               ; axis.xy, packed fp16
FMA.v2f16 r0, r0^, u3.w0, r1^                   ; v_xy * 0.8 + axis.xy (u3.w0 = 0x3a663a66)
F16_TO_F32 r1, r0.h0                            ; x
F16_TO_F32 r0, r0^.h1                           ; y
F16_TO_F32 r2, u2.w1.h0                         ; z = axis.z
CUBEFACE1 r3, r1, r0, r2
CUBEFACE2_V9 r4, r1, r0, r2
CUBE_SSEL r1, r2, r1^, r4
CUBE_TSEL r0, r0^, r2^, r4
FRCP.f32 r2, r3^
FMA.f32 r2, 0x38000000.h1, r2^, 0x0.neg
FMA.f32.clamp_0_1 r1, r1^, r2, 0x38000000.h1
FMA.f32.clamp_0_1 r0, r0^, r2^, 0x38000000.h1
MUX.i32.bit r1, r1^, r4^, u3.w1
MOV.i32 r2, r1^
MOV.i32 r3, r0^                                 ; staging S, T
MOV.i32 r0, u4.w0
MOV.i32 r1, 0x0
TEX_SINGLE.slot1.skip.reserved.16.cube.zero.wait1 @r2:r3, @r2:r3, [r0^:r1^]
```

A mediump direction is computed in fp16 and each lane converted to fp32
(`F16_TO_F32`) before the cube ops, which are fp32 only; the texel is fp16
(`.16`, two registers) because the shader is mediump, as in 2D look-ups
(`sel_t_tex`).

### The descriptors

Nothing new. A cube read by the fragment shader is the same `Texture`
descriptor (`Dimension = Cube`, one face's size, `Array size` 1) and the
same 6 x levels planes as section (a), and the sampler the same
(`Seamless Cube Map`, clamp-to-edge on S, T and R): Mesa emits each
stage's tables with the same `panfrost_emit_resources(batch, stage)`
(`M:gallium/drivers/panfrost/pan_cmdstream.h:261`, called per stage at
`M:gallium/drivers/panfrost/pan_csf.c:1219`). Ours already does too: `sampled_texture` in
`src/mali/backend.rs` builds a `SampledTexture` of six faces for either stage
(its `vertex` argument picks only the binds), and `fs_texture` lands in the
fragment resource table's `TABLE_TEXTURE` entry in `src/mali/frame.rs` as
`vs_texture` does in the vertex one; `SampledTexture::sampler` forces the
cube's wrap modes. The drm-shim traces hold 113's cube (above), so the board
run of `fs-cube` checks 41's 8x8 single-level cube and 114's 16x16 chain.

### What `fs-cube` needs in our code (not done here)

1. **The IR form.** `textureLod` on a cube is `{Cube, Lod}`, which
   `Op::es100_tex` (`src/shaderc/online/ir.rs`) maps to `None`;
   `Es100Tex::TexCube` is only `{Cube, Implicit}`. The fragment walk
   gates on `es100_tex` (below), so it must match `ir::Op::Tex` on its
   fields (`dim: TexDim::Cube`, `lod: TexLod::Implicit | TexLod::Lod(_)`,
   no shadow, projection or offset) as the vertex walk in `nir.rs` already
   does, or `Es100Tex` gains a `TexCubeLod`.
2. **`src/shaderc/online/mali/nir_fs.rs`, `walk_nodes`, the
   `ir::Op::Tex` arm.** It accepts only `Some((Es100Tex::Tex2D, None))`
   (else `MaliError::TexKind`) and a coordinate that is a whole vec2 varying
   or two constants (else `MaliError::TexCoord`). For a cube it would take
   the three lanes (`lane(vals, coord, 0..3)`), any value, and build
   section (b)'s sequence as fragment nodes: `Cubeface1`, `Cubeface2`,
   `CubeSsel`, `CubeTsel`, `Frcp`, `Fmul` by 0.5, two `Ffma` + 0.5 with the
   0-1 clamp, `BitfieldSelect` with `0x1fffffff`, and for an explicit level
   the `Fmul`/clamp/`Fmul`/`F2i` 16/`U2u32` chain. That sequence exists once,
   in the vertex walk's `cube` branch (`nir.rs`, the `IrOp::Tex` arm,
   `Alu::Cubeface1` ... `CUBE_S_MASK`): move it into one function both walks
   call rather than writing it twice. `TexLookup` (and `TexCoord`) then carry
   the two or three staging values, a `dim` and a LOD mode.
3. **`nir_fs.rs`, `tex`.** It builds `Op::TexSingle` with `sr_count: 2`,
   `lod: LodMode::Computed`, `dim: TexDim::D2` fixed; it would take the
   staging values, `sr_count` 2 or 3, `LodMode::Zero` or `LodMode::Explicit`
   and `TexDim::Cube` from the look-up. `Op::TexSingle` (`bir.rs`) and its
   packer arm (`pack.rs`) already take these fields (the vertex look-up in
   `select.rs`, the `nir::Kind::VsTex` arm, uses them). `flow.rs` needs
   nothing: `uses_helpers` counts only a computed level, as Mesa's
   `bi_instr_uses_helpers`, so 41 and 114 end the helper invocations at
   once (the references' `.discard` on their second instruction), and
   `analyze_helper_requirements` sets `skip` on any look-up whose result
   no helper needs, whatever its level (both references carry `.skip`).
4. **The two fragment selectors.** The staging values must be selected
   before the TEX event, in both `select` (the fp16 pairs, case 41: the
   direction in fp16, then `F16_TO_F32` per lane, as `case41` shows) and
   `select_highp` (fp32, 114). The cube ops are 32-bit rows `emit.rs` has
   (`Alu::Cubeface1`, `Cubeface2`, `CubeSsel`, `CubeTsel`; `bir.rs` and
   `pack.rs` have their `Op`s); `select.rs` itself (the vertex selector)
   needs nothing. Case 41's `F16_TO_F32` is not a row yet: the ISA table
   has it (`isa::v10::Op::F16ToF32`, `src/shaderc/online/mali/isa/v10.rs`),
   `bir.rs` has no `Op` for it (the same gap as `mali-texture-refusals.md`
   section (d), case 121).
5. **The layout.** `layout` (`nir_fs.rs`) forces `read |= 0b11` for a
   `TexCoord::Varying` and, at its end, requires the coordinate to be the
   first two lanes of one load; for a computed direction the lanes it reads
   are found by the ordinary `reaches` walk if the staging values are among
   the roots `layout` walks.
6. **The constant lane.** In the vertex walk, drop the `cube` branch's
   `TexVertexConstant` check: the constant becomes an operand, which
   `fau.rs` encodes from the immediate table (`lut32`, Mesa's
   `va_lut_index_32`) as `0x3F800000`; byte-compare with `case114_v`.
   For `case114_f` the link must propagate the vertex shader's constant
   store into the fragment shader (above): the fragment shader then depends
   on the vertex shader, as it does today only for the varying order (the
   program's fragment shader compiled again with a `VaryingOrder` variant
   key, `src/shaderc/online/mali/link.rs`). That key is full (a tag and
   seven nibbles), so carrying a 32-bit constant needs another carrier: a
   design decision of `fs-cube`, not settled here.
7. **The checks.** Byte-equal to `case41.expect.json` (`fs.fragment`),
   `case114_v.expect.json` (`vs.varying`) and `case114_f.expect.json`
   (both programs) through the testdata tests in
   `src/shaderc/online/mali/tests.rs`, then `tools/mali/corpus-verdicts.sh`
   (the six rows above move to `ok`), then the board (41 and 114, x2).
   `src/mali` needs no change by this reading; the board run confirms it.

### What `fs-cube` did (plan #153)

Items 1-5 as read above (`Nir::cube_desc`, `Nir::sfixed_8_8`, the
`ir::Op::Tex` cube arm of `walk_nodes`, `tex` with the staging values, the
mediump direction's `F16_TO_F32` rows), item 6 and two more:

- **The link step (item 6's carrier).** The program's stages are built in
  Mesa's order by three calls of `src/shaderc/online/mali/mod.rs`:
  `vs_constants` (the vertex shader's constant lanes, `link::VaryingConsts`),
  `compile_fs_for_vs` (the fragment shader with each read of such a lane the
  constant, its varyings laid out without them), then `compile_vs_for_reads`
  with that shader's reads, which no longer hold the constant lanes. The
  constants go in through `FsPrecision::consts`, not a variant key. Each
  shader alone keeps its verdict (`cubelodf.vert` stores the constant, which
  no program of Mesa's does: `VaryingNotFromAttribute`). The GL core does not
  call these steps yet: `src/gl/program.rs` builds the program's vertex
  shader for the fragment shader (`link_vs_for_fs`) before the link entry
  that may replace the fragment shader, so wiring them in is a change
  outside the Mali paths, a later item, before 114 can run on the board.
- **The fp32 colour.** Mesa stores a highp ES 3.00 output at fp32
  (`BLEND.v4.f32`, ATEST on the alpha's fp32 word; the blend descriptor is
  the same, its `Register Format` F16 in the board dump
  `0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs4/mesa/mesa-cubelod.pandecode.xz`).
  `FsPrecision::colour_f32` takes it from the output's declaration;
  `select_highp` stores the four words.

Byte-equal: `case41.expect.json`, `case114_v.expect.json` and
`case114_f.expect.json` (every program, the last through the link step),
`sel_v_const.expect.json` (the link step on a mediump pair). Corpus:
`es300/cube.frag`, `cubelodf.frag` and `cubelodv.vert` move to `ok`.
