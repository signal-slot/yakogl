<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): 3D texture levels 1 and up, case 206 es3levels

Issue #32, plan #336 item `frame-3dlevels`. gl_test case 206 (`es3levels`)
FAILs on the NanoPC-T6, `first bad check 2 got 0x04602020 want 0x04602090`
(`tools/cts/log/2026-11-07-t6-step4a/README.md`): its 16x16x4 3D texture is
drawn right at level 0 and reads level 0's red (0x20) where levels 1 and 2
hold 0x90 and 0xf0. This note says where the levels are lost, what this item
changed, what Mesa 26.2.1 describes and how ours differs, and what is left.
Nothing here was run on the board.

## 1. Where levels 1 and 2 are lost

The texture goes through three layers; the first two dropped the chain.

| Layer | Before this item | After |
| --- | --- | --- |
| GL core (`src/gl`) | `native_layered_textures` is false for this backend, so `texture_alloc_storage_3d` (src/gl/texture.rs) lays the volume out as a grid of level 0's slices and keeps levels 1 and up after it, each a grid of its own slices (`level_off[l]`, `grid_level_held`). The draw hands the grid as level 0 alone: `GlTexture::levels` stays 1 for a grid, so `sampling_fields` (src/gl/draw.rs) gives `mip_levels` 1 and `mip_filter` 0 whatever the minification filter. | unchanged (outside this item) |
| Backend (`src/mali/backend.rs`) | `texture_of` took a 3D binding only with `levels == 1`; `volume_of` copied level 0's grid, stacked. | `volume_of` copies every level the binding names, each from its own grid (`grid_level_at`), and refuses a binding naming more levels than the chain has (`INVALID_TEXTURE_STORAGE`). |
| Frame (`src/mali/frame.rs`) | `SampledTexture::volume` already described a chain: one plane a level, `Depth` level 0's slices, `Levels` and LOD 0 to the last (93f9c958, pinned to `texmore_206_vol` by `a_volume_chain_is_the_shim_s`). | unchanged |

So on the board's tip the draw still gets one level: the case keeps failing
until the GL core hands this backend the grid's levels (section 4).

## 2. The GL core's grid, level by level

`texture_alloc_storage_3d` (native layers off, `mip_level_align` 64):

- `cols = ceil(sqrt(depth))` (`grid_cols_for`); level 0 is one 2D image of
  `cols * w` x `ceil(depth / cols) * h` texels at `chain_pitch`
  (`chain_layout_npot(.., one_level = true, ..)`), slice k at column `k %
  cols`, row `k / cols`.
- level l >= 1 starts on the next 64 bytes after level l-1 and is a grid of
  its own `max(depth >> l, 1)` slices of `max(w >> l, 1)` x `max(h >> l,
  1)`, the same `cols` columns, `ceil(depth_l / cols)` rows of slices, on
  the same `chain_pitch` (`pitch = chain_pitch` when it is non-zero, which
  it is for this backend).

`grid_level_at` (src/mali/backend.rs) is that rule. Case 206's 16x16x4:
`cols` 2, pitch 128 bytes, level 0 at 0 (0x1000 bytes), level 1 (8x8x2, one
row of slices) at 0x1000 (0x400), level 2 (4x4x1) at 0x1400 (0x200).
`a_gl_volume_chain_is_copied_level_by_level` (src/mali/backend_tests.rs)
uploads that volume with glTexImage3D through the GL core on the mock, checks
`grid_level_at` against the core's own `level_off`, and checks every row of
every copied slice against the bytes glTexImage3D was given.

## 3. The descriptors against Mesa 26.2.1

Mesa (`pan_sampled_texture_emit`, `emit_generic_plane`,
`M:panfrost/lib/pan_texture.c:410-416`; the shim's `texmore_206_vol`,
`tools/mali/testdata/texmore_206_vol.trace:353-404`) writes `Dimension 3D`,
`Depth 4`, `Levels 3`, LOD 0 to 2 and one plane a level. Its image is
U-interleaved 16x16 tiles; ours is the backend's linear copy, each level's
slices one under the other on 64-byte rows, the levels back to back on 64
bytes.

| Plane | Field | Mesa | Ours | Cause |
| --- | --- | --- | --- | --- |
| 1, 2 | Clump ordering | Tiled U-Interleaved | Linear | Mesa's tiles, our linear rows |
| 1, 2 | Row stride | 0x400 | 0x40 | a row of 16x16 tiles; one linear row |
| 1 | Slice stride | 0x400 | 0x200 | one tile row a slice; 8 rows of 64 bytes |
| 2 | Slice stride | 0x400 | 0x100 | the same; 4 rows of 64 bytes |
| 1 | Size | 0x800 | 0x400 | 2 slices of a tile row; 2 of 8 rows |
| 2 | Size | 0x400 | 0x100 | 1 slice of a tile row; 4 rows |
| 1 | Pointer | base + 0x1000 | base + 0x1000 | equal: level 0 is 0x1000 in both |
| 2 | Pointer | base + 0x1800 | base + 0x1400 | follows level 1's 0x800 tiled bytes; its 0x400 linear |
| 1, 2 | Width, Height, Clump format, Plane type | 8x8 / 4x4, RAW32, Generic | the same | |

Every other plane byte is equal. `a_volume_chain_s_levels_1_and_2_are_the_shim_s`
(src/mali/frame_tex_tests.rs) pins these per level (the texture descriptor's
`Levels`, `Depth` and maximum LOD too); `a_volume_chain_is_the_shim_s` keeps
pinning the whole table (sampler byte-equal, every plane). The 2D chains (cube,
array, mipmapped 2D) do not go through `volume_of` and keep their bytes:
their tests (`a_two_layer_array_chain_is_the_shim_s`,
`a_nearest_mipmapped_chain_is_the_board_s`, ...) are unchanged and pass.

## 4. Left: the GL core's hand-over (outside this item)

What case 206 still needs, in `src/gl/draw.rs` where a texture binding is
filled (after `sampling_fields`, beside the PowerVR's `native_vol` block): for
a `GLES_TEXTYPE_3D` binding of a grid (`t.grid_cols != 0`) on this backend,
`mip_levels` = the grid levels held and written (`grid_level_held`, the
`level_defined` bits, clipped by `GL_TEXTURE_MAX_LEVEL`) and `mip_filter`
from the minification filter as `sampling_fields` gives it for a chain
(`*_MIPMAP_NEAREST` 1, `*_MIPMAP_LINEAR` 2). `tb.bytes` can stay level 0's:
`volume_of` reads the levels after it, as a 2D chain's are read. The PowerVR
must keep getting one level for its grid program (case 206's `GL_TEST_PVR`
labels), so the hand-over has to be this backend's alone: a backend-table
answer or the Mali's feature, not a change of `sampling_fields` for every
backend. With it, levels 1 and 2 reach the frame as section 3 describes;
the board then shows whether the texture unit agrees (the case's checks 2
and 3).
