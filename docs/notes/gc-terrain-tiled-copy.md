<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: terrain's per-frame tiled copy

Item 3 of `docs/notes/imx8mp-glmark2-cpu.md`, "Fix items, in order".

## Finding: which textures, and why every frame

Found with the host harness (vivante build, `GM2_PROFILE=1` run of all 33
entries, 3 frames) and a temporary log line in `tiled_copy::refresh`
(removed). Terrain made six copies in three frames, two per frame, always
the same two textures:

| texture | size, format | vertex shader sampler | per frame |
|---|---|---|---|
| height map | 256x256 RGBA8, 1 level, 2D | `tDisplacement` (`terrain.vert:40`) | one copy |
| normal map | 256x256 RGBA8, 1 level, 2D | `tNormal` (`terrain.vert:45`) | one copy |

The log of each copy: `content_gen` constant (25 and 26), `target_kick`
rising by 9 a frame (5, 14, 23 and 6, 15, 24).

Who asks: `tiled_copy::prepare` -> `wanted_vs`, which tiles every plain 2D
texture the vertex stage samples (the GC7000 vertex stage cannot read linear
storage). Why again each frame: glmark2 (`SceneTerrain::update`) moves
`uvOffset` of the height map's program every frame and renders the height
map and then the normal map into their FBO textures again. A render
advances `target_kick`, which is part of the copy's `Key`. So the content
**changed** since the last copy: the key is right and the copy is not
redundant. "Keep the copy while the texture is unchanged" has nothing to
keep for terrain, and `tiled_copy.rs` already does it for textures that
really are unchanged.

`tests/gc_tiled_copy_reuse.rs` pins that behaviour: an unchanged
vertex-sampled texture gets one copy over four draws (the descriptor's
level 0 address stays); a `glTexSubImage2D` and a render into the texture
each give a new copy; the written texel is in the new copy. (On the host the
OSAL runs no stream, so the render case checks the new copy only.)

## What was saved: the cost of `tile`

The number of copies cannot go down; the cost of each can. `tile` moved one
texel at a time with a run-time `bpp` and computed `tiled_offset` for each.
A tile row (four texels) is contiguous in both layouts, so `tile_layer` now
moves it as one 16-byte run (32 for 8-byte texels; a tile cut by the right
edge moves its `lw % 4` texels). Unit test
`a_tile_row_run_moves_the_same_bytes_as_the_texel_copy` compares it with the
texel loop for 256x256, 17x13, 5x3, 1x1, 4x4, 2x7 at 1, 2, 4, 8 bytes a texel
and a wider row pitch.

Host, one 256x256 RGBA8 layer, release build, 2000 repeats
(`cargo test --release --no-default-features --features
host-test,gpu-vivante,shaderc-online --lib timing_of_a -- --ignored
--nocapture`):

| | ms a layer | terrain, two maps a frame |
|---|---|---|
| before, texel by texel | 0.049 | 0.10 |
| after, tile row runs | 0.0047 | 0.009 |

About 10 times faster in `tile`. This is the loop only: the rest of
`refresh` (clearing the destination, the cache calls, the allocation) is
unchanged, so the whole 0.3 to 0.45 ms of the earlier profile does not go to
zero. The OSAL on the board maps GPU memory non-cacheable, where the
per-texel loop costs more than on the host, so the board gain should be
larger; not measured.

## Not done

- The remaining copy cost is in `refresh` (zero fill of the destination,
  `cache_invd` of the source, `cache_flush`), which a render-target-aware
  path could avoid by rendering to a tiled texture directly (what Mesa
  does). That is a back-end change outside this item.
- Board: terrain's picture and ms a frame (#1206).
