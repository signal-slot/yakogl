<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: the TMU words, texture state, sampler state and layouts for the ES 3.0 look-ups

Source: Mesa 26.2.1 (tag `mesa-26.2.1`, commit `da14d65e4499`, MIT; the pin is
`third_party/mesa-26.2.1/UPSTREAM`). Every cite below is `path:line` in that
tree, paths relative to its root. The files under `src/broadcom/cle/`,
`src/broadcom/common/` and `src/gallium/drivers/v3d/v3dx_format_table.c` are
vendored; the others (`src/broadcom/compiler/*`, `src/gallium/drivers/v3d/v3dx_state.c`,
`v3d_resource.c`, `v3d_uniforms.c`, `v3d_program.c`, `src/compiler/*`) are read from
the full tarball the pin names. The facts are Mesa's V3D 4.2 choices (`V3D42_*`
structs); where Mesa only shows what it programs, the note says "Mesa
programs", not what the hardware would also accept.

Yakogl symbols: `src/v3d/texture.rs` (`refusal_for`, `tmu_format`, `state`,
`sampler`, `wrap`), `src/v3d/ffi.rs` (`V3dTexState`, `V3dSampler`),
`src/shaderc/online/v3d/prepare.rs` (`Op::Tex`, `SOp::Tex`, `SOp::TexSizeWord`),
`src/v3d/state.rs` (`rt_format`).

## 1. How a look-up reaches the TMU (all look-ups)

1. **Register writes.** Mesa turns each NIR texture source into one write of
   a TMU magic register, in the order of `instr->src[]`
   (`src/broadcom/compiler/v3d_tex.c:202-216`); only the coordinate's first
   component `s` is held back (`v3d_tex.c:100-107`, `:281-282`) and written
   last, as the retiring write (`v3d_tex.c:370-383`). GLSL-derived NIR orders
   the sources coordinate, projector, comparator, offset, bias, lod, ddx
   (`src/compiler/glsl/glsl_to_nir.cpp:2743-2805`); `lower_txp = ~0`
   (`src/broadcom/compiler/vir.c:807`) removes the projector.
2. **Registers by source** (`v3d_tex.c:100-198`):

   | source | write |
   |---|---|
   | coord `s` | held; retires the look-up (item 3) |
   | coord `t` (2 or more non-array components) | `TMUT` (`:108-113`) |
   | coord `r` (3 or more non-array components) | `TMUR` (`:115-120`) |
   | array layer (`is_array`, the last coord component) | `TMUI` (`:122-128`) |
   | bias | `TMUB` (`:131-135`) |
   | lod | `TMUB` (`:137-139`) |
   | shadow comparator (reference) | `TMUDREF` (`:155-158`) |
   | non-constant offset | `TMUOFF` (`:175-188`) |
   | constant offset | no write: P2 fields (section 2) |

   `non_array_components = coord_components - is_array` (`:208-210`), so a
   2D-array look-up writes `s`, `t` then `TMUI`; a 3D look-up `s`, `t`, `TMUR`.
3. **The retiring write** (`v3d_tex.c:370-383`): `txf` writes `TMUSF`
   (cube is not allowed, `:373`); a cube writes `TMUSCM`; `txl` writes
   `TMUSLOD`; everything else writes `TMUS`. The write count is
   `ldtmu_count = return_words_of_texture_data` (`:383`).
4. **Configuration words before the retiring write** are queued with the
   `wrtmuc` signal on a NOP (`v3d_tex.c:56-62`), in this order: P0 always
   (`:294-300`), then P1, then P2. P2 is written only when it differs from the
   default `{op = REGULAR}` or for `lod` queries (`:303-306`, `:367-368`);
   P1 is then written too, from the default `{per_pixel_mask_enable}` when no
   sampler is needed (`:352-365`, default `:64-66`).
5. **Input FIFO budget.** `tmu_writes > 16 / threads` halves the thread count
   (`v3d_tex.c:264-270`), so each added source (layer, reference, offset) can
   cost threads in the QPU program.
6. **P0** (`src/broadcom/cle/v3d_packet.xml:1534-1537`): bits 0-3
   `Return words of texture data`, the rest the texture-state address.
   Mesa sets the words to `components_read & 0xf` for a 32-bit output and
   `& 0x3` for 16-bit (`v3d_tex.c:250-253`), packs the texture index into
   bits 24-31 for the driver to replace with the address
   (`v3d_tex.c:294-298`; the driver ORs the compiler's low bits onto the
   texture-state address of the bound view,
   `src/gallium/drivers/v3d/v3d_uniforms.c:154-157`).
7. **P1** (`v3d_packet.xml:1539-1544`): bit 0 `Output Type 32-bit`, bit 1
   `Unnormalized coordinates`, bit 2 `Per-pixel mask enable`, the rest the
   sampler-state address. Mesa sets bit 2 outside fragment shaders and for
   `skip_helpers` (`v3d_tex.c:315-321`), bit 1 for rect samplers only, and
   the sampler index in bits 24-31 (`:343`). A look-up that needs no sampler
   (`txf`) takes P1 from the compiler as a constant, no sampler address
   (`v3d_tex.c:346-351`); a `txf` of a 32-bit texture still carries
   `output_type_32_bit` (`:237-242`, `:309-312`), and the sampler used for the
   precision decision is the texture index (`:234-235`,
   `src/gallium/drivers/v3d/v3d_program.c:306-310`).
8. **Output size.** `sampler_is_32b` has a bit per texture unit whose format
   returns 32 bits (`v3d_program.c:563-575`; `v3d_get_tex_return_size` rows are
   the 4th and 5th columns of `v3dx_format_table.c:60-208`: 32 for R32F/RG32F/
   RGBA32F, R32I..RGBA32UI, 16-bit-channel UNORM/SNORM, depth; 16 for the
   others). The shader's value is packed 16-bit unless the bit is set
   (`src/broadcom/compiler/vir.c:632-641`).
   Maps to `refusal_for(b, ret32)` / `return_32` in `texture.rs` and
   `LodKey::ret32`.

## 2. texelFetch, texture offsets, gradients, bias

### texelFetch (`txf`)

- Registers: coordinate components as in section 1; the integer `lod` is
  written to `TMUB`, then `s` to `TMUSF` (`v3d_tex.c:137-139`, `:372-374`).
  `nir_lower_tex` adds `lod = 0` to a `txf` that has none
  (`src/compiler/nir/nir_lower_tex.c:1795-1805`), so every `txf` writes
  `TMUB`.
- For `txf` no `Disable AutoLOD` is set: automatic LOD is already off
  (`v3d_tex.c:141-150`). `Disable AutoLOD` (P2 bit 1,
  `v3d_packet.xml:1571`) is set for a cube `txl` only (`v3d_tex.c:147-150`)
  and for gather (`:261`).
- No sampler address is needed (section 1 item 7); wrap and filter state
  do not apply. The texture state is the only memory read.
- `txf` is not allowed on a cube (`v3d_tex.c:373`).
- Yakogl: `Op::Tex` has no fetch form yet (`prepare.rs:1327`); the
  texel-fetch path ends in `TMUSF`.

### Constant and variable offsets

- Constant offset: P2 `Offset S` bits 8-11, `Offset T` 12-15, `Offset R` 16-19,
  each a signed 4-bit `int` (`v3d_packet.xml:1564-1566`). Mesa fills
  `offset_s` always, `offset_t` for 2 or more non-array components,
  `offset_r` for 3 (`v3d_tex.c:164-173`). A constant offset makes P2
  non-default, so P2 and P1 are written (`:303-306`, `:352-368`).
- Non-constant offset: `x = off.x & 0xf`, `y = off.y & 0xf`,
  `TMUOFF = x | (y << 4)` (`v3d_tex.c:179-188`); no `r` component, and P2
  `Offset Format 8` (bit 0, `v3d_packet.xml:1572`) is not set by
  `v3d_vir_emit_tex` (`v3d_tex.c:255-262`).
- An offset in a `txf` and in a bias/lod look-up uses the same code
  (`handle_tex_src` takes the source by type, `v3d_tex.c:99-198`).
- Yakogl: `Op::Tex { offset, .. }` carries the offset today
  (`prepare.rs:1327`).

### textureGrad (`txd`)

- Mesa does not use the hardware coefficient mode for GL: `lower_txd = true`
  (`vir.c:802`) rewrites `txd` into `txl`. `lower_gradient`
  (`nir_lower_tex.c:827-882`): `size = i2f(textureSize(level 0))`
  (3 components for 3D, 1 for 1D, else 2, `:838-854`); `dPdx = ddx * size`,
  `dPdy = ddy * size` (`:865-866`); `rho = max(|dPdx|, |dPdy|)` for one
  component, else `max(sqrt(dot(dPdx,dPdx)), sqrt(dot(dPdy,dPdy)))`
  (`:868-875`); `lod = log2(rho)` (`:878`), "ignoring GL state biases".
  `replace_gradient_with_lod` removes `ddx`/`ddy`, clamps with
  `min_lod` when present, adds the `lod` source and sets the op to `txl`
  (`:660-678`).
- A cube takes `lower_gradient_cube_map` (`:681`, `:830-833`).
- The explicit-gradient hardware path exists (P2 bit 4 `Coefficient Mode`,
  `v3d_packet.xml:1569`; set for `txd`, `v3d_tex.c:260`) but the `txd`
  never reaches it in Mesa's GL path.
- The resulting `txl` writes the `lod` to `TMUB` and `s` to `TMUSLOD`
  (`v3d_tex.c:137-139`, `:377-378`).
- `textureSize` at level 0 for the formula is the uniform path of section 6.

### Bias and lod

- `bias` goes to `TMUB` and the look-up retires with `TMUS` (`v3d_tex.c:131-135`,
  `:379-381`); `txl` writes `lod` to `TMUB` and retires with `TMUSLOD`
  (`:137-139`, `:377-378`).
- The sampler state's `Fixed Bias` (`s8.8`, `v3d_packet.xml:1722`) is set from
  `cso->lod_bias` (`v3dx_state.c:597`); `Max/Min Level-of-Detail` are `u4.8`
  (`v3d_packet.xml:1723-1724`).

## 3. Shadow look-ups

- **Reference register.** The reference (`nir_tex_src_comparator`) is written
  to `TMUDREF`, the texture sources keep their `src[]` order, `s` is last
  (`v3d_tex.c:155-158`, `:202-216`). GLSL sources: comparator after the
  coordinate and projector, before the offset (`glsl_to_nir.cpp:2749-2775`).
  A shadow look-up on a 2D array writes `s`, `t`, `TMUI`, `TMUDREF` and
  retires on `TMUS` (or `TMUSLOD` for `txl`).
- **The compare is in the sampler state**, not in the QPU program:
  `Depth Compare Function` is 3 bits at bit 4 of the `Sampler State`
  (`v3d_packet.xml:1728`), enum `V3D_COMPARE_FUNC` NEVER=0 LESS=1 EQUAL=2
  LEQUAL=3 GREATER=4 NOTEQUAL=5 GEQUAL=6 ALWAYS=7
  (`v3d_packet.xml:3-12`, same order as GL and `pipe_compare_func`). Mesa
  programs `compare_mode ? compare_func : V3D_COMPARE_FUNC_NEVER`
  (`v3dx_state.c:598-600`).
- The texture must be a depth type: `DEPTH COMP16` = 21, `DEPTH COMP24` = 22,
  `DEPTH COMP32F` = 23, `DEPTH24_X8` = 24 (`v3d_packet.xml:354-357`). The
  format table gives depth the return size 32 and one channel, the swizzle
  `XXXX`: `Z16_UNORM` -> `DEPTH_COMP16`, `Z32_FLOAT` -> `DEPTH_COMP32F`,
  `X8Z24_UNORM`/`S8_UINT_Z24_UNORM` -> `DEPTH24_X8`
  (`v3dx_format_table.c:149-156`). A depth look-up therefore sets
  `Output Type 32-bit` (section 1 item 8).
- Sampling depth from a depth/stencil format demotes
  `S8_UINT_Z24_UNORM` to `X8Z24_UNORM` (`v3dx_state.c:1083-1089`).
- Yakogl: `Op::Tex { shadow, .. }` (`prepare.rs:1327`); the compare function
  lands in `V3dSampler`, which has no compare field yet (`ffi.rs:562-574`).

## 4. Texture shader state (`TEXTURE_SHADER_STATE`, 4.2)

Layout (`v3d_packet.xml:1592-1631`): `Image Width/Height/Depth` 14 bits each at
bits 58, 72, 86 (`:1617-1619`); `Array Stride (64-byte aligned)` 26 bits at bit 32
(`:1621`); `Texture base pointer` at bit 0 (`:1623`); `Texture type` 7 bits at
bit 100 (`:1616`); `Swizzle R/G/B/A` 3 bits each at 108/111/114/117
(`:1602-1613`) with values Zero=0 One=1 Red=2 Green=3 Blue=4 Alpha=5
(`:1603-1608`); `Base Level` bit 124, `Max Level` bit 120 (`:1599-1600`);
`sRGB` bit 3 (`:1627`); the UIF fields at `:1594-1597`.

How Mesa fills it (`src/gallium/drivers/v3d/v3dx_state.c:904-988`):

- **Image depth.** 3D: `depth0`; otherwise the number of layers in the view
  (`last_layer - first_layer + 1`) (`:935-939`). A cube array divides by 6
  (`:945-948`).
- **Width/height** are `width0/height0` masked to 14 bits (`:921-933`).
- **Base pointer** is the BO offset plus `v3d_layer_offset(prsc, 0, first_layer)`
  (`:957-960`; `v3d_resource.c:771-780`).
- **Array stride** is `rsc->cube_map_stride / 64` for every target, array,
  cube and 3D (`:961`). For 3D `cube_map_stride` is the level-0 slice size
  (section 5).
- **Base/Max Level** are `base_level` and `last_level` of the view
  (`:950-952`).
- **Swizzle.** The four fields are `v3d_translate_pipe_swizzle` of the view's
  swizzle composed with the format's swizzle (`:1031-1034`, composition
  `:1059-1071`). The format swizzles use the constants 0 and 1 for the
  missing channels: `R8` -> `X001`, `RG8` -> `XY01`, `RGB565` and RGB-only
  formats -> `XYZ1`, depth -> `XXXX` (`v3dx_format_table.c:47-58`, `:81-85`).
  The hardware swizzle values `Swizzle Zero` (0) and `Swizzle One` (1) are
  the constants (`v3d_packet.xml:1603-1604`). Mesa's comment says the composed
  view swizzle goes to the shader key for a 32-bit return and into the state for
  16-bit (`v3dx_state.c:1059-1062`); the code also writes it to the state for
  both (`:1031-1034`).
- **Texture type** is `v3d_get_tex_format` (`:1036-1037`), the third column
  of `v3dx_format_table.c:60-208`.
- Yakogl: `V3dTexState { depth, array_stride_64, swizzle, type_, ... }`
  (`src/v3d/ffi.rs:542-557`); `state()` in `texture.rs` sets
  `depth = layers(b)` and takes `array_stride_64` from
  `v3d_tex_state_from_layout`; `tmu_format()` returns the type and swizzle,
  and `refusal_for` refuses `GLES_TEXTYPE_2D` with layers above 1 and 3D.

### Data formats for the formats the plan adds

`Texture Data Formats` (`v3d_packet.xml:328-385`):

| class | data format (value) | Mesa row | swizzle | return |
|---|---|---|---|---|
| RGB565 | `RGB565` 6 | `B5G6R5_UNORM` | `XYZ1` | 16 (`v3dx_format_table.c:81`) |
| RGB10_A2 | `RGB10_A2` 9 | `R10G10B10A2_UNORM` | `XYZW` | 16 (`:71`) |
| RGB10_A2UI | `RGB10_A2UI` 114 | `R10G10B10A2_UINT` | `XYZW` | 16 (`:75`) |
| R16F | `R16F` 16 | `R16_FLOAT` | `X001` | 16 (`:90`) |
| RGBA16F | `RGBA16F` 18 | `R16G16B16A16_FLOAT` | `XYZW` | 16 (`:100`) |
| ETC2 RGB8, sRGB8 | `RGB8_ETC2` 32 | `ETC2_RGB8`, `ETC2_SRGB8` | `XYZ1` | 16 (`:159-160`) |
| ETC2 RGB8 A1 | `RGB8_PUNCHTHROUGH_ALPHA1` 33 | `ETC2_RGB8A1`, `ETC2_SRGB8A1` | `XYZW` | 16 (`:161-162`) |
| ETC2 RGBA8 (EAC) | `RGBA8_ETC2_EAC` 38 | `ETC2_RGBA8`, `ETC2_SRGBA8` | `XYZW` | 16 (`:163-164`) |
| EAC R11 | `R11_EAC` 34 / `SIGNED_R11_EAC` 35 | `ETC2_R11_UNORM/SNORM` | `X001` | 16 (`:165-166`) |
| EAC RG11 | `RG11_EAC` 36 / `SIGNED_RG11_EAC` 37 | `ETC2_RG11_UNORM/SNORM` | `XY01` | 16 (`:167-168`) |

(`v3d_packet.xml` lines: RGB565 `:339`, RGB10_A2 `:342`, R16F `:349`,
RGBA16F `:351`, ETC2/EAC `:371-379`, RGB10_A2UI `:422`.)

- The sRGB ETC2 formats use the same data format as the linear ones; the
  difference is the `sRGB` bit of the texture state (`v3d_packet.xml:1627`),
  set from `util_format_is_srgb` (`v3dx_state.c:1023-1026`).
- ETC2 and RGB10_A2 are not render targets: the RT column of
  `v3dx_format_table.c:159-168` is `NO`.
- RGB10_A2 data: "R low, A high" (`v3d_packet.xml:342`); the 16-bit return
  is 4 half-float words (`v3dx_format_table.c:71`, the return columns `16, 0`).
- The sampler-state variants Mesa keeps for integer formats
  (`1010102U` for a 10-bit unsigned class) are chosen by channel size
  (`v3dx_state.c:1094-1124`).
- Yakogl: `tmu_format()` rows and `V3D_SAMPLED_TEXFMTS`
  (`src/backend.rs`) must carry these data formats; `refusal_for` names
  "ETC2, or the RGB10A2 class".

## 5. Sampler state (`SAMPLER_STATE`)

`v3d_packet.xml:1709-1734`: `Wrap S/T/R` 3 bits at bit 48/51/54 (`:1718-1720`);
`Wrap I Border` bit 57 (`:1717`); `Border Color Mode` bit 58 (`:1716`);
`Fixed Bias` bit 32 (`:1722`); `Max/Min Level-of-Detail` bits 20/8
(`:1723-1724`); `Depth Compare Function` bit 4 (`:1728`); `Anisotropy
Enable` bit 3, `Mip/Min/Mag filter Nearest` bits 2/1/0 (`:1730-1733`).

- `Wrap Mode`: REPEAT 0, CLAMP 1, MIRROR 2, BORDER 3, MIRROR_ONCE 4
  (`v3d_packet.xml:79-85`).
- Mesa programs `wrap_r = translate_wrap(cso->wrap_r)` beside `s` and `t`
  (`v3dx_state.c:593-595`, `:781-787`), and `wrap_i_border = false`
  (`:591`). It treats BORDER on any of the three as using the border colour
  (`:785-787`).
- Yakogl: `sampler()` in `texture.rs` fixes `wrap_r: V3D_WRAP_MODE_REPEAT`;
  `V3dSampler.wrap_r` exists (`ffi.rs:565`); the GL ES 3.0 value is
  `GlesTextureBind::wrap_r`, mapped by `wrap()`.
- Swizzle is in the texture state (section 4), not the sampler.

## 6. Resource layout: 3D, 2D-array, cube, compressed

`v3d_setup_slices` (`src/gallium/drivers/v3d/v3d_resource.c:582-768`):

- **cpp and blocks.** `rsc->cpp = util_format_get_blocksize(format)` (`:797`),
  i.e. the bytes of one block for a compressed format (ETC2 RGB8: 8,
  RGBA8 EAC: 16). `block_width/height` are the format's block (`:595-596`);
  each level's width and height are divided by the block, rounded up
  (`:646-647`). The slice stride is `level_width * cpp` (`:704`), the slice
  size `level_height * stride` (`:719-720`).
- **Power-of-two shrink.** Level 0 and level 1 take their own size; level 2
  and below take the minified level-1 power-of-two padded value
  (`:603-611`, `:629-635`). The padding is from the block-aligned level-1
  size (`v3d_get_dimension_mpad`, `:572-580`).
- **3D slice shrink.** `pot_depth` is padded the same way with block size 1
  (`:609-611`); the depth of level `i` is `u_minify(depth, 0)` for level 0
  and `u_minify(pot_depth, i)` for the others (`:636-639`). The level's size in
  memory is `slice->size * level_depth` (`:722`); levels are laid from the
  smallest to the largest (`:625`), so level 0 is last.
- **Layer stride.** For a non-3D target `cube_map_stride =
  align(slices[0].offset + slices[0].size, 64)` is the distance from one
  full mip chain to the next, and `size += stride * (array_size - 1)`
  (`:757-764`); the base-level layer address is `slice->offset + layer *
  cube_map_stride` (`:779`). For a 3D target `cube_map_stride =
  slices[0].size`, the stride between the slices of level 0
  (`:765-767`), and a slice is addressed `slice->offset + layer * slice->size`
  (`:776-777`).
- **Tiling.** A level is raster, linear-tile, UIF-blocks 1 or 2 columns, or
  UIF by its width and height against the u-tile and UIF-block size
  (`:649-698`); the u-tile size comes from `cpp` (`:591-594`). 1D targets
  align their width to 64 bytes in raster (`:651-653`). `slice->ub_pad` and
  XOR are set for UIF levels (`:681-696`); level 1 is page-aligned when it
  could be UIF-XOR (`:724-734`); the level-0 start is 4096-aligned
  (`:749-755`).
- **View of a layer range.** The base pointer shifts by `v3d_layer_offset`
  of the first layer (`v3dx_state.c:957-958`), and the image depth is the
  layer count (`:938`).
- Yakogl: `driver/src/v3d/v3d_tex.c` (`layout`) and `texture.rs`
  `V3dTexLayout` follow this routine for 2D and cube; for a 3D and
  array binding the stride choice above applies.

### textureSize

No hardware size query is used for a texture: Mesa passes the size as
uniforms (`src/broadcom/compiler/nir_to_vir.c:967-971`). `txs` reads
`QUNIFORM_TEXTURE_WIDTH + i` for component `i`; the array count of an array
sampler is `QUNIFORM_TEXTURE_ARRAY_SIZE` in the last component
(`nir_to_vir.c:914-934`); the order of the uniform kinds is WIDTH, HEIGHT,
DEPTH, ARRAY_SIZE (`src/broadcom/compiler/v3d_compiler.h:270-273`). Every
dimension except the array count is minified by the lod in the shader:
`max(size >> level, 1)` (`nir_to_vir.c:896-912`, `:944-947`). The driver's
values are the base resource sizes minified by `first_level`, the array
count `array_size` (divided by 6 for cube arrays), and `last_level -
first_level + 1` for levels (`v3d_uniforms.c:51-88`). `nir_lower_tex` supplies
`lod = 0` when `textureSize` has none (`nir_lower_tex.c:1795-1805`).
Yakogl: `SOp::TexSizeWord`, `Op::TexSize` (`prepare.rs:1451`).

## 7. The RGB10A2 render-target row

- Format row: `R10G10B10A2_UNORM` is output format `RGB10_A2`, texture
  format `RGB10_A2`, swizzle `XYZW`, 16-bit return
  (`v3dx_format_table.c:71`); `B10G10R10A2_UNORM` swaps with `ZYXW` (`:72`),
  the X2 forms use `XYZ1`/`ZYX1` (`:73-74`); `R10G10B10A2_UINT` is
  `RGB10_A2UI` for both (`:75`).
- Internal type and bpp (`v3dx_format_table.c:219-341`): `RGB10_A2`
  renders as `V3D_INTERNAL_TYPE_16F`, `V3D_INTERNAL_BPP_64`
  (`:252-261`, with sRGB, R11F_G11F_B10F and RGBA16F: "sRGB RTs are stored in
  the tile buffer at 16F", `:255-258`). `RGB10_A2UI` renders as
  `V3D_INTERNAL_TYPE_16UI`, `V3D_INTERNAL_BPP_64` (`:282-286`).
- The float RTs the plan reads beside it: `RGBA16F` is 16F/64 (`:254`),
  `R16F` and `RG16F` 16F/64 ("64bpp to make sure the TLB doesn't throw away
  the alpha channel before alpha test happens", `:263-270`), `RGBA32F` 32F/128,
  `RG32F` 32F/64, `R32F` 32F/32 (`:319-330`).
- Yakogl: `rt_format(surf)` in `src/v3d/state.rs:439` holds the same
  `internal_type`/`internal_bpp` columns; the RGB10A2 class the GL core keeps
  in RGBA8 is the gap `refusal_for` names.
