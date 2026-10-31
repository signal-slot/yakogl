<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: the memory layout of 2D-array, 3D and ETC2/EAC textures

Study, no code change. The rules the ES 3.0 texture items of the Pi 4 back end
(2D-array and 3D sampling, ETC2/EAC sampling) rely on, each re-derived from
Mesa 26.2.1 and cited by file:line. Paths are relative to the Mesa root.

Where the sources are: `src/gallium/drivers/v3d/v3dx_format_table.c` and
`src/broadcom/cle/v3d_packet.xml` are in `third_party/mesa-26.2.1/`;
`src/gallium/drivers/v3d/v3d_resource.c` and `v3dx_state.c` (the resource and
the texture state) are not vendored, they are in the full tree (`VERSION`
26.2.1; here `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`). "Mesa" below means
that full tree. Every `v3d_resource.c` / `v3dx_state.c` line was read there.

## 1. A 2D array uses the cube-map path

Mesa's gallium v3d gives every non-3D target the same layer arithmetic; a
cube map, a cube array, a 2D array and a 1D array differ only in the layer
count (`prsc->array_size`).

- `v3d_setup_slices` lays out ONE layer's whole mip chain first
  (`v3d_resource.c:625-736`, levels from `last_level` down to 0, so level 0 is
  last and largest) and then, for every target but 3D, sets
  `cube_map_stride = align(slices[0].offset + slices[0].size, 64)`
  (`v3d_resource.c:761-763`) and adds `cube_map_stride * (array_size - 1)` to
  the resource size (`:764`). The stride is the end of level 0, so the end of
  the chain, rounded up to 64 bytes. The comment at `:757-760` says it: "Arrays
  and cube textures have a stride which is the distance from one full mipmap
  tree to the next (64b aligned)".
- A layer of a level is at `slices[level].offset + layer * cube_map_stride`
  (`v3d_layer_offset`, `:769-779`, the non-3D arm at `:778-779`). So a 2D array
  of N layers is N complete mip trees, each tree laid out as a 2D texture of
  one layer's size; the depth is never minified for it.
- The state packs `array_stride_64_byte_aligned = cube_map_stride / 64`
  (`v3dx_state.c:961`; the unit is 64 bytes, the field 26 bits, see section 3)
  and `image_depth = last_layer - first_layer + 1` (`v3dx_state.c:938`), the
  layer count of the view. The base pointer is the layer `first_layer` of
  level 0 (`:957-960`). Only a cube array divides the depth by 6
  (`:945-948`: "when we are sampling from cube arrays ... layers / 6, but not
  ... sampling from 2d image arrays"). A plain cube map is the same path with
  depth 6.
- The 2D array and the cube share the tiling decision of a 2D resource
  (section 2): per level, from that level's size alone, never from the layer
  count (`:649-698`).

What the GL core gives the back end: `GlesTextureBind::type_`
(`GLES_TEXTYPE_2D_ARRAY` = 2, `src/backend.rs:807`), `layers` (the layer
count, `:838`), and `width`/`height` of ONE layer (`:840-841`), with the
layers stacked plainly because the V3D table sets `native_layered_textures`
(`src/backend.rs:1385-1393`, `src/v3d/backend.rs:6356-6358`). A level holds its
layers one after the other (`src/v3d/texture.rs:56-65`, `gl_level_layered`).

Current state (yakogl today): the 2D-array arm is refused,
`"a 2D array or 3D texture is not sampled yet"` (`src/v3d/texture.rs:107`).
The state helper already does the cube arithmetic for any layer count:
`array_stride_64 = align(s0.offset + s0.size, 64) / 64`
(`driver/src/v3d/v3d_tex.c:522-524`), `depth = layers(b)`
(`src/v3d/texture.rs:307-310`), and `v3d_tex_layer_stride`,
`v3d_tex_layers_size`, `v3d_tex_layer_offset` (`v3d_tex.c:343-372`) are
Mesa's `:761-779`. What is missing is only the type in `layers()` and the
refusal; the layer layout has no 3D arm.

## 2. 3D: per-level depth, rounded to a power of two from level 1

`v3d_setup_slices` (`v3d_resource.c:583-767`):

- **Level depth.** `pot_depth = 2 * v3d_get_dimension_mpad(depth, 1, 1)`
  (`:609`); `v3d_get_dimension_mpad(d, level, block)` is
  `u_minify(block * next_pow2(ceil(u_minify(d,1) / block)), level - 1)`
  (`:573-581`). Level 0 has `level_depth = depth` (`:636-637`); a level `i >= 1`
  has `u_minify(pot_depth, i)` (`:639`). So the depth of level 1 is
  `next_pow2(depth >> 1)`: a depth of 6 allocates `next_pow2(3) = 4` slices at
  level 1, a depth of 5 allocates 2 (`next_pow2(2)`), a depth of 9 allocates 4.
  Then level `i` halves that again, at least 1. The same pot rule runs for the
  width and height of levels `i >= 2` (`:629-635`, `pot_width/pot_height` at
  `:603-608`); levels 0 and 1 use the true sizes.
- **Size.** A level's bytes are `slice->size * level_depth`
  (`:720-722`), `slice->size = padded_height * stride` being ONE slice;
  `offset += slice_total_size` (`:736`); level 1 additionally rounds its total
  to a 4096-byte page when level 1 could be UIF XOR (`:729-733`); the whole
  resource is moved to a 4 KiB base (`:749-755`).
- **Tiling per level.** Chosen per level from that level's width and height in
  blocks, identical for every slice of the level: LINEARTILE when
  `width <= utile_w || height <= utile_h`, UBLINEAR 1/2 columns when
  `width <= uif_block_w` / `2 * uif_block_w`, else UIF (XOR when the padded
  height is a multiple of the page-cache rows) (`:649-698`). Level 0 alone is
  forced to UIF only when `uif_top` (shared or MSAA, `:866`), not for a
  texture. The first-level `ub_pad` is stored per slice (`:681`) and the state
  gets level 0's (`v3dx_state.c:976-985`).
- **Array stride.** For 3D `cube_map_stride = slices[0].size` (`:765-766`):
  the distance between two slices of the SAME level is one slice's size; the
  state field is `array_stride_64_byte_aligned = slices[0].size / 64`
  (`v3dx_state.c:961`). Level 0's slice size is a multiple of 64 because a
  slice is whole utiles of 64 bytes. A slice `k` of level `l` is at
  `slices[l].offset + k * slices[l].size` (`v3d_layer_offset`, `:777-778`),
  which is why every level has its own `size` and why levels 1 and up need the
  padded slice count: the offset of the next (larger) level depends on it.
- **State.** `image_depth = prsc->depth0` (`v3dx_state.c:936`), the level 0
  depth; the hardware minifies it itself.

What the GL core gives the back end for a 3D texture: `type_ = GLES_TEXTYPE_3D`
(`src/backend.rs:808`), `layers` = the depth of level 0 (`:838`), `width` and
`height` of one slice. In GL storage the 3D texture is stacked with
`native_layered_textures` (`src/gl/texture.rs:1412-1431`): `depth` slots of the
level's size at EVERY level, level `l` holding its `depth >> l` slices (at
least 1, `depth_at`, `src/gl/texture.rs:1369-1371`) in its first slots; only
those are read. The padded slices Mesa allocates (depth 6 at level 1: 4 slices
where GL has 3) are tiling padding the back end owns; GL never writes them, so
the tiled copy leaves them zero or unread. The number of levels is
`chain_levels_3d` (log2 of the largest side, including the depth,
`src/gl/texture.rs:1363-1365`), which can exceed the 2D chain (4x4x16).

Current state: refused (`texture.rs:107`). `v3d_tex_layout` is the 2D,
one-layer, uncompressed case only, "never-shared, no winsys stride"
(`driver/src/v3d/v3d_tex.c:127-129`), its pot rule is `dimension_mpad` with
block size 1 and no depth (`:120-125`), and it has no `level_depth`; there is
no 3D arm. `v3d_tex_state_from_layout` always writes the cube stride,
`align(s0.offset + s0.size, 64) / 64` (`v3d_tex.c:522-524`), which is the
wrong value for a 3D texture (it must be `s0.size / 64`). `v3d_tex_pack_state`
accepts a depth up to 14 bits (`:542`).

## 3. The texture-shader-state fields

`TEXTURE_SHADER_STATE` for V3D 4.2 (`src/broadcom/cle/v3d_packet.xml:1592-1631`,
`max_ver="42"`; the generated `V3D42_TEXTURE_SHADER_STATE_pack`, 24 bytes,
`driver/src/v3d/generated/v3d_packet_v42_pack.h:4973-5043`). Bit positions
within the 192-bit record, from the XML `start`/`size`:

| field | bits | size | XML line |
| --- | --- | --- | --- |
| `Array Stride (64-byte aligned)` | 32..57 | 26 | `:1621` |
| `Image Width` | 58..71 | 14 | `:1619` |
| `Image Height` | 72..85 | 14 | `:1618` |
| `Image Depth` | 86..99 | 14 | `:1617` |
| `Texture type` | 100..106 | 7 | `:1616` |
| `Extended` | 107 | 1 | `:1614` |
| `Swizzle R/G/B/A` | 108/111/114/117 | 3 each | `:1602-1613` |
| `Max Level`, `Base Level` | 120..123, 124..127 | 4 each | `:1599-1600` |
| `Level 0 UB_PAD`, `Level 0 XOR enable`, `Level 0 is strictly UIF`, `UIF XOR disable` | 128..131, 132, 134, 135 | | `:1593-1597` |

The base pointer is bits 0..31 (the low 6 bits are flags: flip X/Y,
`flip S and T`, `sRGB` bit 3, AHDR, reverse border), so the image is 64-byte
aligned. In the generated packer: `array_stride_64_byte_aligned` is bytes 4-7
(`cl[4..7]`, low 2 bits of byte 7), `image_width` is byte 7 bits 2.. to byte 8,
`image_height` bytes 9-10, `image_depth` bytes 10-12 (bit 6 of byte 10 to bit 3
of byte 12), `texture_type` byte 12 bit 4 to byte 13 bit 2
(`v3d_packet_v42_pack.h:4991-5015`).

Which carries what: a 3D texture: `image_depth` = level 0 depth,
`array_stride_64` = slice size / 64 (section 2). A 2D array: `image_depth` =
layer count, `array_stride_64` = tree stride / 64 (section 1). A cube: depth 6.
The texture type is the format's (section 4), the same for a 2D, an array and
a 3D texture of one format; 3D versus array is told to the TMU by the look-up
(the dimensionality of the coordinates in the TMU config, not a field of
this record), so the record has no dimension field.

Image width and height are 14 bits (a size of 16384 is not representable,
`v3dx_state.c:932-933`, mask 0x3fff); the depth is 14 bits too and Mesa
asserts nothing on it, so 16384 layers or slices cannot be described.

Current state: `v3d_tex_pack_state` writes exactly these fields
(`driver/src/v3d/v3d_tex.c:559-567`), with `width`/`height`/`depth` checked
against 14 bits (`:540-544`) and `array_stride_64` against 26 bits (`:544`).
`state()` (`src/v3d/texture.rs:287-312`) sets `depth = layers(b)` for the cube.

## 4. ETC2 and EAC

**TMU type and return size.** The ten ES 3.0 formats are rows of
`v3dx_format_table.c:159-168`. The columns are `FORMAT(pipe, rt, tex, swiz,
return_size, return_channels)` (`:37-46`): no render-target format (`NO`) and
a return size of 16 for all ten. The `Texture Data Format` values are
`v3d_packet.xml:371-379`.

| GL format | TMU type (value) | swizzle (Mesa) | return size | block bytes |
| --- | --- | --- | --- | --- |
| COMPRESSED_RGB8_ETC2, COMPRESSED_SRGB8_ETC2 | `RGB8_ETC2` (32) | XYZ1 | 16 | 8 |
| COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 | `RGB8_PUNCHTHROUGH_ALPHA1` (33) | XYZW | 16 | 8 |
| COMPRESSED_RGBA8_ETC2_EAC, COMPRESSED_SRGB8_ALPHA8_ETC2_EAC | `RGBA8_ETC2_EAC` (38) | XYZW | 16 | 16 |
| COMPRESSED_R11_EAC | `R11_EAC` (34) | X001 | 16 | 8 |
| COMPRESSED_SIGNED_R11_EAC | `SIGNED_R11_EAC` (35) | X001 | 16 | 8 |
| COMPRESSED_RG11_EAC | `RG11_EAC` (36) | XY01 | 16 | 16 |
| COMPRESSED_SIGNED_RG11_EAC | `SIGNED_RG11_EAC` (37) | XY01 | 16 | 16 |

The sRGB forms use the same type as the linear ones: the sRGB decode is the
state's `sRGB` bit (`v3dx_state.c:1023-1025`, from `util_format_is_srgb`). The
block bytes are `util_format_get_blocksize` (`v3d_resource.c:797`:
`rsc->cpp = util_format_get_blocksize(format)`), 8 for the RGB8 and R11 forms
and 16 for the others. The 16-bit return size means the 16-bit TMU output
path (`v3d_get_tex_return_size`, `v3d_formats.c:102-116`). ETC1 has no row in
the table (Mesa decodes it at upload; `etc1_decoded_at_upload` in the V3D
table, `src/v3d/backend.rs:6360-6362`).

**Block tiling: a block is a texel.** `v3d_setup_slices` divides the level
size by the block size (`level_width = DIV_ROUND_UP(level_width, block_width)`
`:646`, height `:647`; the block is 4x4 for all ten) and from then on works in
blocks, with `cpp = blocksize`. So an 8-byte ETC2 block is one 8-byte texel
and a 16-byte block is one 16-byte texel, and the utile sizes of those cpp
apply (`src/broadcom/common/v3d_tiling.c:37-70`): a utile is 64 bytes, 4x2
blocks for cpp 8 (`utile_w` 4, `utile_h` 2) and 2x2 blocks for cpp 16
(`utile_w` 2, `utile_h` 2). The tiling choice (section 2) runs on the block
counts; a UIF block is 2x2 utiles. The store/load routines also move whole
blocks (`v3d_resource.c:321-322`, "Our load/store routines work on entire
compressed blocks"). The TMU hence reads a compressed level exactly as it reads
a texture of 8- or 16-byte texels of size `ceil(w/4) x ceil(h/4)`.

**Level sizing and the padding rule.** The power-of-two padding is "based on
the block size" (`v3d_resource.c:596-602`): `pot_width = 2 *
v3d_get_dimension_mpad(width, 1, block_width)` (`:603`), and the mpad function
rounds `ceil(u_minify(w,1) / block)` blocks up to a power of two, times the
block (`:573-581`). Levels 0 and 1 use the true size, and a level `i >= 2` uses
`u_minify(pot_width, i)` (`:629-635`), which is then divided by the block
(`:646`). So the number of blocks in a level `i >= 2` is
`ceil((pot_width >> i) / 4)`, not `ceil((w >> i) / 4)`:

- width 17: level 1 is 8 texels (2 blocks); `pot_width = 2 * 8 = 16`; level 2
  is 4 texels (1 block), the same as `17 >> 2 = 4`.
- width 36: level 0 is 9 blocks, level 1 is 18 texels, 5 blocks; `next_pow2(5)
  = 8` blocks, `pot_width = 64`; level 2 is `64 >> 2 = 16` texels, 4 blocks,
  where `36 >> 2 = 9` texels is 3 blocks. The two differ at the third level.

The padded blocks are allocated (they are in `slice->size` through the
padded `level_height`/`stride`) and are padding the back end owns. The level
is then aligned to its tiling (`align(level_width, utile_w)` etc., `:652-700`)
and the stride is `level_width * cpp` (`:704`); a block row of a level is a
whole number of utiles. A compressed resource is tiled like any other
(`should_tile` has no compressed arm, `:814-852`).

**What `compressed_blocks_in_rows` means for the V3D.** The flag
(`src/backend.rs:1383-1385`, doc `:1376-1385`) says how the GL core writes a
compressed level: `true` is raster rows of `ceil(w/4)` blocks, one row after
the next, at a row pitch of `row_aligned(blocks, block_bytes,
linear_row_align)` (`src/gl/texture.rs:3445-3452`, `level_block_grid`
`:3463-3475`); with `false` (the default) the level is the PowerVR's
power-of-two block grid in twiddled order, level 0's grid halved per level
(`:3453-3475`, `twiddle_index` `:3505`). In both cases the GL core's
`mip_level_align` (0 for the V3D) packs the levels one after the other
(`compressed_chain_layout`, `:3478-3496`). Neither is the V3D layout:
the V3D wants a tiled copy (utile/LT/UBLINEAR/UIF of 8- or 16-byte texels)
with Mesa's padded level sizes, which the back end builds from the GL core's
storage, as for RGBA8 (`docs/notes/v3d-sampled-copies.md`). The choice that
makes this a pure re-tile is `true`: raster rows with the level's own block
count in a row (`level_block_grid`'s first arm, `(w0 >> level).max(1)` texels
rounded to blocks) and `linear_row_align` 0 (`row_aligned` returns the pitch
unchanged, `src/gl/context.rs:214-217`), so the back end reads row `by` of a
level at `by * blocks_w * block_bytes` and stores the blocks with
`v3d_store_tiled_image` at `cpp = block_bytes`. With `false` the back end
would have to undo `twiddle_index` first. Neither choice changes where the
level is in GL storage relative to the other levels in a way the tiled copy
cares about: the tiled copy has its own offsets (Mesa's `slices[l].offset`),
so the GL core's offsets only say where to read.

Current state: the V3D table has `compressed_blocks_in_rows: false`
(`src/v3d/backend.rs:6355`); ETC2 and EAC are refused by name
(`src/v3d/texture.rs:109-112`, the ETC2 format is not in
`V3D_SAMPLED_TEXFMTS`); the format table of `driver/src/v3d/v3d_tex.c`
(`:413-486`) has no ETC row and `layout` takes cpp 1, 2, 4, 8 and 16
(`:156`) with `width` and `height` in texels (the 2D uncompressed case,
`:127-129`), so a block count must be passed as the size with the block bytes
as cpp, and the pot rule of `dimension_mpad` (`:120-125`) takes no block
size, so it must learn the block (Mesa `:573-581`).

## 5. What the next items take from this

- `v3d-tex-3d-array`: a layer-aware `layout` with `level_depth` per level
  (`depth` at 0, `next_pow2` pot rule from 1, section 2), `slice_total_size
  = size * level_depth`, the stride `s0.size / 64` for 3D and the tree
  stride for arrays, and the tiled copy that reads `depth >> l` slices from
  the GL core's first slots at each level.
- `v3d-tex-etc2`: the ten rows of the table in section 4, the block as cpp,
  the block-aware pot padding, `compressed_blocks_in_rows` set to `true`, and
  a tiled copy from raster block rows.
- Not measured: none of this is board-measured on the Pi 4; the rules are
  Mesa's. `tools/v3d/tex_check.c` compares our layout with Mesa's built
  library for the cases it covers; 3D and compressed cases have to be added
  to it with their arms.
