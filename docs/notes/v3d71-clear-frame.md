<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: the clear-only frame, packet by packet, and the 7.1 tile sizes

Raspberry Pi 5 port (#7), item H5 of `pre-study-rpi5.md` (the 7.1 frame
builder) and the first clear on the board (M1 to M4). This note is the
specification `driver/src/v3d/v3d_frame.c` is changed to at 7.1, and the
probe that checks it reads. It changes no code.

Sources, each read for this note:

- `X` = `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`: packet
  codes, fields and `min_ver`/`max_ver`. Opcode and length are what Mesa's
  `gen_pack_header.py <xml> 71` writes as `V3D71_<NAME>_opcode` / `_length`
  (and `42` for the 4.2 column). For a packet with sub-ids the length is the
  packet's, 9 bytes for every `Tile Rendering Mode Cfg` part.
- `C` = `tools/v3d/shim71/clear.clif`: the drm-shim dump of Mesa's GL clear of
  a 64x64 target at V3D 7.1.7.0 (`gl_ref: clear, framebuffer 64x64`). The
  line numbers below are lines of this file.
- `M` = Mesa 26.2.1 `src/gallium/drivers/v3d/` (tag `mesa-26.2.1`, commit
  `da14d65e`, MIT). The gallium driver is not vendored; the tree is the
  checkout the gate's Mesa work obtains (the same tag as `third_party/`).
  `U` = `src/broadcom/common/v3d_util.c`, which is vendored (same lines in
  `third_party/mesa-26.2.1/`).
- `F` = our `driver/src/v3d/v3d_frame.c` at `f1a9c0e0`.

The three lists in `C` are 19 bytes (binning, `C:10-23`), 96 bytes (render,
`C:31-114`, `@add_render` end `0xd060`) and 27 bytes (the generic tile list,
`C:122-146`, `GENERIC_TILE_LIST` end `0xe01b`). The byte counts below add up
to exactly those; the totals are the check that no packet was missed.

## 1. The packets

CLIF names are the shortnames of `X` (`CLEAR_VCD_CACHE` is `X`'s
`Flush VCD cache`, code 19; `OCCLUSION_QUERY_COUNTER_ENABLE` is
`Occlusion Query Counter`, code 92; `TILE_COORDS`, `END_LOADS`, `STORE`,
`END_TILE`, `GENERIC_TILE_LIST`, `SUPERTILE_COORDS`, `END_RENDER`,
`IMPLICIT_TILE_COORDS`, `BRANCH_IMPLICIT_TILE` and `RETURN` are the
shortnames of the packets `F` emits as `TILE_COORDINATES`, `END_OF_LOADS`,
`STORE_TILE_BUFFER_GENERAL`, `END_OF_TILE_MARKER`,
`START_ADDRESS_OF_GENERIC_TILE_LIST`, `SUPERTILE_COORDINATES`,
`END_OF_RENDERING`, `TILE_COORDINATES_IMPLICIT`,
`BRANCH_TO_IMPLICIT_TILE_LIST` and `RETURN_FROM_SUB_LIST`). The "4.2 clear"
column is what `F` emits; "same" means the same packet with the same fields.

### 1.1 Binning list (`C:10-23`, 19 bytes)

Emitted by `v3dX(start_binning)` (`M:v3dx_draw.c:45-114`) and capped by
`v3dX(bcl_epilogue)` (`M:v3dx_job.c:34-71`). Ours: `emit_start_binning` and
the `FLUSH` in `v3d_frame_build_clear` (`F`).

| # | Packet | Code, bytes | 7.1 fields Mesa sets (`C` value) | `X` | Mesa | 4.2 clear (`F`) |
|---|---|---|---|---|---|---|
| 1 | `NUMBER_OF_LAYERS` | 119, 2 | Number of Layers 1 (`C:10-11`) | `X:939-941` | `v3dx_draw.c:61-65`, `job->num_layers > 0`: always true for a plain target (`util_framebuffer_get_num_layers` is at least 1, `v3dx_draw.c:129`) | **not emitted** (Mesa emits it at 4.2 too). Decision for `frame71`: the 7.1 byte reference is 19 bytes with it; emit it so the lists equal `C` |
| 2 | `TILE_BINNING_MODE_CFG` | 120, 9 | Width 64, Height 64 (pixels, minus-one encoded), **Log2 Tile Width 3, Log2 Tile Height 3** (64 px), tile allocation initial block size 1 (128 B), tile allocation block size 0 (64 B) (`C:12-18`) | `X:967-993` (4.2: `X:943-965`) | `v3dx_draw.c:71-83` | **changed fields**: 4.2 sets width, height, `number_of_render_targets` 1, `maximum_bpp_of_all_render_targets` 32, the two block sizes. At 7.1 the render-target count, max bpp, 4x and double-buffer fields are gone from this packet and the tile size is given explicitly as log2 of width and height (3 bits each at bits 8 and 11) |
| 3 | `CLEAR_VCD_CACHE` | 19, 1 | none | `X:457` | `v3dx_draw.c:105` | same (`FLUSH_VCD_CACHE`) |
| 4 | `OCCLUSION_QUERY_COUNTER_ENABLE` | 92, 5 | address `[null]` (`C:20-21`) | `X:822-824` | `v3dx_draw.c:108` | same (`OCCLUSION_QUERY_COUNTER`, no address) |
| 5 | `START_TILE_BINNING` | 6, 1 | none | `X:431` | `v3dx_draw.c:113` | same |
| 6 | `FLUSH` | 4, 1 | none | `X:429` | `v3dx_job.c:70` | same |

2 + 9 + 1 + 5 + 1 + 1 = 19 bytes, `@add_bin` end `0x8013` (`C:162-168`).
Without `NUMBER_OF_LAYERS` it would be 17.

### 1.2 Render list (`C:31-114`, 96 bytes)

`v3dX(emit_rcl)` (`M:v3dx_rcl.c:701-945`) and `emit_render_layer` (`:566`).
Ours: `emit_rcl`.

| # | Packet | Code, bytes | 7.1 fields Mesa sets (`C` value) | `X` | Mesa | 4.2 clear (`F`) |
|---|---|---|---|---|---|---|
| 1 | `TILE_RENDERING_MODE_CFG_COMMON` (sub-id 0) | 121, 9 | Number of Render Targets 1, Image Width 64, Image Height 64, 4x 0, double-buffer 0, **Depth-buffer disable 0**, Early-Z disable 1, Early-Z direction LT/LE, Internal Depth Type depth_32f, Early Depth/Stencil Clear 0, **Log2 Tile Width 3, Log2 Tile Height 3** (`C:31-43`) | `X:1025-1060` (4.2: `X:996-1023`) | `v3dx_rcl.c:719-775`; log2 at `:770-773` | **changed fields**: the same RT count, size and `early_z_disable`; `maximum_bpp_of_all_render_targets` is gone (the sub-id is 3 bits wide, not 4, and the RT bpp moves to packet 2); new `log2_tile_width`/`log2_tile_height`; new `Depth-buffer disable`, which Mesa leaves 0 in `C` even with no depth buffer (a clear-only frame needs no setting of it; `pre-study-rpi5.md` §3.2 keeps its use open) |
| 2 | `TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1` (sub-id 2), once per colour target | 121, 9 | Render Target number 0, **Base Address 0**, **Stride 32** (the field holds 31: minus-one, 7 bits), **Internal BPP 0** (32), **Internal Type and Clamping 8**, Clear Color low bits `0xffff0000` (`C:44-50`) | `X:1112-1123` | `v3dx_rcl.c:864-875`; stride `U:316-326` = `tile_width * bpp_words / 2` (64 * 1 / 2); base `:874` | **new packet, replaces two**: 4.2's `CLEAR_COLORS_PART1` (sub-id 3: low 32 + next 24 bits of clear colour, RT number) and `COLOR` (one packet for all four RTs: bpp, type, clamp per RT) are dropped. The clear colour moves into this packet (32 bits here; Parts 2 and 3, below, add the rest) |
| 2a | `..._RENDER_TARGET_PART2` (sub-id 3) | 121, 9 | Clear Color mid bits (40 bits), RT number | `X:1134-1139` | `v3dx_rcl.c:877-882`, only if RT bpp is 64 or 128 | not in this frame (32 bpp). Absent for the clear we do |
| 2b | `..._RENDER_TARGET_PART3` (sub-id 4) | 121, 9 | Clear Color top bits (56 bits), RT number | `X:1152-1157` | `v3dx_rcl.c:886-892`, only if 128 bpp | not in this frame |
| 3 | `TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES` | 121, 9 | Z Clear Value 0.0, Stencil Clear Value 0 (`C:51-53`) | `X:1094-1101` (4.2: `X:1085-1092`) | `v3dx_rcl.c:919-923` | **changed sub-id**: 7.1 sub-id 1 (4.2: 2); fields same |
| 4 | `TILE_LIST_INITIAL_BLOCK_SIZE` | 126, 2 | size of first block 1 (128 B), use auto-chained tile lists 1 (`C:54-56`) | `X:1187-1194` | `v3dx_rcl.c:928-931` | same |
| 5 | `MULTICORE_RENDERING_TILE_LIST_BASE` (`X`: `Multicore Rendering Tile List Set Base`) | 123, 5 | Tile List Set Number 0, address `tile_alloc + 0` (`C:57-59`) | `X:1179-1183` | `v3dx_rcl.c:576-578` | same |
| 6 | `MULTICORE_RENDERING_SUPERTILE_CFG` | 122, 9 | supertile 1x1 tiles, frame 1x1 supertiles, 1x1 tiles, multicore 0, raster order 0, bin tile lists 1 (`C:60-69`) | `X:1164-1177` | `v3dx_rcl.c:580-614` | same (`F` follows the same loop; the sizes come from the tile size: see section 2) |
| 7 | `TILE_COORDS` | 124, 4 | column 0, row 0 (`C:70-72`) | `X:1159-1162` | `v3dx_rcl.c:620-623` | same |
| 8 | `END_LOADS` | 26, 1 | none | `X:487` | `v3dx_rcl.c:641` | same |
| 9 | `STORE` (buffer **None**) | 29, 13 | Buffer to Store 8 (None), Memory Format Raster, Flip Y 0, Dither None, Decimate sample 0, Output Image Format 0, Clear buffer being stored 0, R/B swap 0, channel reverse 0, Height in UB or Stride 0, Height 0, Address null (`C:74-86`) | `X:491-525` | `v3dx_rcl.c:642-644` (the dummy store of GFXH-1742) | same |
| 10 | `CLEAR_RT` | 25, **1** | none (first tile only) | `X:485` (4.2: `CLEAR_TILE_BUFFERS`, `X:480-483`, 2 bytes, flags Z/stencil and all RTs) | `v3dx_rcl.c:646-655` (`#else` branch `CLEAR_RENDER_TARGETS`); the initial clear is only in the first tile because `do_double_initial_tile_clear` (`:551`) is false without double buffering | **changed packet**: same opcode 25, but 7.1 has `Clear Render Targets` with no fields (1 byte); the Z/stencil flag is gone and depth is not cleared by it. The count of RCL bytes differs by 1 per use |
| 11 | `END_TILE` | 27, 1 | none | `X:489` | `v3dx_rcl.c:656` | same |
| 12-15 | the second dummy tile: `TILE_COORDS`, `END_LOADS`, `STORE` None, **no clear**, `END_TILE` | 4 + 1 + 13 + 1 | as 7 to 9, 11 (`C:89-106`) | | `v3dx_rcl.c:638-657` (`for i = 0..1`, coords again at `i > 0`) | same (`F` loops twice and clears at `i == 0` only) |
| 16 | `CLEAR_VCD_CACHE` | 19, 1 | none | `X:457` | `v3dx_rcl.c:658` | same |
| 17 | `GENERIC_TILE_LIST` (`Start Address of Generic Tile List`) | 20, 9 | start `CL_0xe000+0`, end `CL_0xe000+0x1b` (`C:108-110`) | `X:459-462` | `v3dx_rcl.c:381-384` | same (start and end in the tile list) |
| 18 | `SUPERTILE_COORDS` | 23, 3 | column 0, row 0 (`C:111-113`), once per supertile that overlaps the draw area | `X:475-478` | `v3dx_rcl.c:688-696`; the draw area of a clear is the whole target (`v3dx_draw.c:1787-1790`) | same |
| 19 | `END_RENDER` | 13, 1 | none | `X:438` | `v3dx_rcl.c:944` | same |

Byte count of the frame in `C`: 9 + 9 + 9 + 2 + 5 + 9 = 43 for packets 1 to 6;
tile one 4 + 1 + 13 + 1 + 1 = 20; tile two 4 + 1 + 13 + 1 = 19; 1 + 9 + 3 + 1
= 14; total 96. At 4.2 the same frame is 106 bytes: `CLEAR_COLORS_PART1` and
`COLOR` (18) replace `RENDER_TARGET_PART1` (9), and `CLEAR_TILE_BUFFERS` is
one byte longer (computed from `X`; not a measured 4.2 dump).

The render list is a function of the target's size only through packets 1,
5, 6 and the supertile loop: the tile size (section 2) decides the tile count,
the supertile configuration and the number of `SUPERTILE_COORDS`.

**What the frame cannot see in `C`, and `frame71` must decide:**

- Packet 2's `Base Address` is the RT's offset in the TLB in units of 512
  bits (`X:1119` comment; `M:v3dx_rcl.c:871,874`: `base_addr += tile_height *
  stride / 8` per RT). With one RT it is 0. For the clear we do it is 0 and
  the stride is `tile_w * words / 2` = 32 for a 64-pixel tile at 32 bpp.
- Packet 2's `Internal Type and Clamping` is **not** the 4.2 `Internal Type`
  enum. `X:154-175` (`Render Target Type Clamp`, 7.1 only): `8` = 8, `8i` = 0,
  `8ui` = 4, `16f` = 9, `32f` = 10 and so on, with clamped variants at 16 and
  up. An RGBA8 target is 8 (`C:49`: "8 /* 8 */"). `v3d_frame.c` writes
  `V3D_INTERNAL_TYPE_8` (value 2 in `X:126`) plus a separate clamp; that does
  not carry over. `M:v3dx_rcl.c:413-449` (`clamp_for_format_and_type`) maps
  the internal type to this field.
- Stride is 7 bits stored minus one, so at most 128; the largest legal
  value is a 64-pixel tile at 128 bpp (64 * 4 / 2 = 128, stored 127). Base
  address is 11 bits. Neither limit is reached by a tile the TLB can hold.

### 1.3 Generic tile list (`C:122-146`, 27 bytes)

`v3d_rcl_emit_generic_per_tile_list` (`M:v3dx_rcl.c:345-385`), the list the
render list branches to once per tile. Ours: `emit_tile_list`.

| # | Packet | Code, bytes | 7.1 fields Mesa sets (`C` value) | `X` | Mesa | 4.2 clear (`F`) |
|---|---|---|---|---|---|---|
| 1 | `IMPLICIT_TILE_COORDS` | 125, 1 | none | `X:1185` | `v3dx_rcl.c:357` | same |
| 2 | `END_LOADS` | 26, 1 | none (no loads in a clear) | `X:487` | `v3dx_rcl.c:359` (`v3d_rcl_emit_loads`, whose last packet is `END_OF_LOADS`, `:238`) | same |
| 3 | `PRIM_LIST_FORMAT` | 56, 2 | primitive type 2 (List Triangles), tri strip or fan 0 (`C:124-126`) | `X:657` | `v3dx_rcl.c:364-366` | same |
| 4 | `SET_INSTANCEID` | 54, 5 | Instance ID 0 | `X:649-651` | `v3dx_rcl.c:369-371` | same |
| 5 | `BRANCH_IMPLICIT_TILE` | 21, 2 | tile list set number 0 | `X:464-467` | `v3dx_rcl.c:373` | same |
| 6 | `STORE` (render target 0) | 29, 13 | Buffer to Store 0 (RT0), Memory Format 4 (UIF no XOR) in `C`, Flip Y 0, Dither None, Decimate sample 0, Output Image Format 27 (rgba8), Clear buffer being stored 0, R/B swap 0, channel reverse 0, Height in UB or Stride 8 (UIF blocks), Height 0, Address `resource+0` (`C:131-143`) | `X:491-525` | `store_general` (`v3dx_rcl.c:115-178`), called from `v3d_rcl_emit_stores` (`:242`) | same packet; **our field values differ on purpose**: Memory Format Raster (0), Output Image Format RGBA8 (27, the same), Height in UB or Stride the pitch in bytes. The probe's target is a raster buffer, Mesa's clear is UIF (a GL texture); `Raster` and the byte stride are `X:493-496`'s own words ("height in ub for UIF, byte stride for raster") |
| 7 | `CLEAR_RT` | 25, 1 | none | `X:485` | `v3dx_rcl.c:331-340`, taken when `job->clear_tlb` | **changed packet** as in 1.2 #10 (4.2 `CLEAR_TILE_BUFFERS`, both flags set) |
| 8 | `END_TILE` | 27, 1 | none | `X:489` | `v3dx_rcl.c:377` | same |
| 9 | `RETURN` | 18, 1 | none | `X:456` | `v3dx_rcl.c:379` | same (`RETURN_FROM_SUB_LIST`) |

1 + 1 + 2 + 5 + 2 + 13 + 1 + 1 + 1 = 27 bytes, `C:146` ends at `0xe01b`.
The 4.2 list is 28 bytes (`CLEAR_TILE_BUFFERS` is 2).

### 1.4 What the submit carries

`@add_bin` and `@add_render` (`C:162-186`) are the register-side facts the
frame does not hold: the binning list `0x8000..0x8013`, the tile allocation
`0x9000` of 12288 bytes (3 pages), the tile state `0xc000` (one page) and the
render list `0xd000..0xd060`. `v3d_frame_alloc_sizes` (`F`) gives the same
12288 for one tile: `align(1 * 128, 4096) + 8192`, and no pool for no draws
(`U:v3d_tile_alloc_sizes`, shared by both generations). The tile state is
`tiles * 256` bytes (256 here, a page in the allocation). Nothing in this
section changes at 7.1.

### 1.5 Summary for `frame71`

New in 7.1, against `F`: `NUMBER_OF_LAYERS` (optional, see 1.1); log2 tile
fields in the binning and common packets; `RENDER_TARGET_PART1` in place of
`CLEAR_COLORS_PART1` + `COLOR`; `CLEAR_RT` in place of `CLEAR_TILE_BUFFERS`
(twice); `ZS_CLEAR_VALUES` sub-id 1. Dropped: `CLEAR_COLORS_PART1`, `COLOR`,
the RT count and max bpp of both configuration packets. Everything else is
the 4.2 sequence unchanged, in the same order.

## 2. The 7.1 tile sizes

A 7.1 frame must give the tile size itself (the log2 fields of 1.1 and 1.2),
and it must be a size the TLB can hold, so the builder chooses it the way
Mesa does: `v3d_choose_tile_size` for `devinfo->ver >= 71`
(`U:122-206`, the 7.x branch `:152-181`).

The rule, in Mesa's words:

- Tile sizes in order of preference (`U:133-141`): 64x64, 64x32, 32x32,
  32x16, 16x16, 16x8, 8x8.
- `color_bpp` = the **sum** over the colour targets of `4 * words(bpp)` bytes
  (32 bpp = 4, 64 = 8, 128 = 16: `v3d_context.c:278-296` computes
  `total_bpp += 4 * v3d_internal_bpp_words(...)`), times 4 with 4x
  multisample (`U:169`). `depth_bpp` = 4 bytes times 4 with multisample
  (`U:170`), taken whether or not there is a depth buffer.
- The first size in the list for which `tile_size_valid(w * h, color_bpp,
  depth_bpp)` holds is used (`U:171-177`). `tile_size_valid`
  (`U:106-119`) with the TLB sizes `U:101-103` (colour 16 KiB, depth 16 KiB,
  auxiliary depth 8 KiB): valid if `pixels * depth_bpp <= 8 KiB` and
  `pixels * color_bpp <= 32 KiB` (the depth buffer fits the auxiliary buffer
  and its 16 KiB joins the colour), or `pixels * depth_bpp <= 16 KiB` and
  `pixels * color_bpp <= 16 KiB`.
- Double buffering adds one step (`U:179-180`); the table below is single
  buffered, as the clear is.

`log2` of the tile edge in the packets: 8 -> 0, 16 -> 1, 32 -> 2, 64 -> 3
(`X:975-986`).

### 2.1 The table

Tile width x height, from the rule above for N targets of the same bpp
(colour bytes per pixel = N * bytes, times 4 with 4 samples). The 4.2 value
in the right half of each cell is what `v3d_choose_tile_size`'s 4.x branch
(`U:183-198`, which `v3d_frame_tiling_bpp` mirrors) gives for the same input;
"-" is a configuration the generation does not have (4.2 has at most 4 targets
and its table ends at 8x8: the index would be out of range).

1 sample:

| Render targets | 32 bpp 7.1 / 4.2 | 64 bpp 7.1 / 4.2 | 128 bpp 7.1 / 4.2 |
|---|---|---|---|
| 1 | 64x64 / 64x64 | 64x32 / 64x32 | 64x32 / 32x32 |
| 2 | 64x32 / 64x32 | 64x32 / 32x32 | 32x32 / 32x16 |
| 4 | 64x32 / 32x32 | 32x32 / 32x16 | 32x16 / 16x16 |
| 8 | 32x32 / - | 32x16 / - | 16x16 / - |

4 samples:

| Render targets | 32 bpp 7.1 / 4.2 | 64 bpp 7.1 / 4.2 | 128 bpp 7.1 / 4.2 |
|---|---|---|---|
| 1 | 32x32 / 32x32 | 32x16 / 32x16 | 32x16 / 16x16 |
| 2 | 32x16 / 32x16 | 32x16 / 16x16 | 16x16 / 16x8 |
| 4 | 32x16 / 16x16 | 16x16 / 16x8 | 16x8 / 8x8 |
| 8 | 16x16 / - | 16x8 / - | 8x8 / - |

(4.2 has at most four render targets, `v3d_limits.h:44`; "-" marks the
eight-target rows, which exist only at 7.1.)

Worked cell, for the check: 1 target, 128 bpp (16 B), 1 sample. 64x64:
4096 * 16 = 64 KiB > 32 KiB, no. 64x32: 2048 * 16 = 32 KiB <= 32 KiB and
2048 * 4 = 8 KiB <= 8 KiB (auxiliary depth), valid: **64x32**. 4.2 gives
index 2 (128 bpp) = 32x32. The 8-target, 4-sample, 128 bpp cell: colour
8 * 16 * 4 = 512 B per pixel, depth 16 B; 8x8 = 64 pixels: 32 KiB and 1 KiB,
valid, and 16x8 (128 pixels) is 64 KiB: the rule ends at 8x8, which is the
case the comment at `U:149-151` says the auxiliary buffer exists for.
Mixed bpp is not in the table: the 7.1 rule uses the sum, so two 32 bpp
targets and one 128 bpp target are 24 bytes per pixel (a 64x32 tile needs 48
KiB, over the 32 KiB limit; 32x32 needs 24 KiB, valid: 32x32), where 4.2's
rule takes three targets and 128 bpp as one step of the count and two of the
bpp (16x16).

### 2.2 `v3d_frame_tiling_bpp` must not be reused at 7.1

`F`'s `v3d_frame_tiling_bpp` is Mesa's **4.x** branch (`U:183-198`):
`idx = rts_bucket (1 -> 0, 2 -> 1, 3-4 -> 2) + 2 * msaa + max_bpp_index`,
with the table cut at 7 entries. Where it differs from 7.1 (the table above,
every difference is that 4.2 steps down and 7.1 does not):

- The 4.2 key is the **largest** internal bpp among the targets plus a
  bucket of the count; the 7.1 key is the **sum** of the bytes of all of
  them. One 128 bpp target is 32x32 on 4.2 and 64x32 on 7.1; two 64 bpp
  targets 32x32 against 64x32 (cells above).
- 4.2 steps down once for 2 targets and twice for 3 or 4, whatever their bpp;
  7.1 has no count bucket, only the byte sum: four 32 bpp targets are 64x32 on
  7.1 and 32x32 on 4.2.
- For the clear-only frame (one 32 bpp target, one sample) the two coincide at
  64x64. Reusing `v3d_frame_tiling_bpp` would be wrong for any frame with
  more than one target, 64 bpp or wider, or 4 samples at 128 bpp, and it has
  no eight-target case.
- The 4.2 tile size is not written into any packet: the hardware derives it
  from the count, the bpp and the msaa bit of the binning and common
  packets (`F`'s own comment on `v3d_frame_tiling_bpp`). At 7.1 the packet
  carries it (`Log2 Tile Width/Height`), so the number the builder computes
  is what the hardware uses, and a wrong tile size is a TLB overflow, not a
  disagreement the hardware corrects.

The 7.1 chooser is a new function (`v3d_frame_tiling_v71`, `frame71`'s to
name), with the sum-of-bytes key and the Mesa loop; the supertile loop that
follows (`F`, `emit_render_layer`, `M:v3dx_rcl.c:580-614`) is the same
for both and unchanged.

## 3. What of the Pi 4 `v3d_clear` carries to a Pi 5 clear probe

Read from `git show 5a496fd5^:driver/samples/zephyr/v3d_clear/src/main.c`
(269 lines; the sample was removed in `5a496fd5`). Line numbers are that
file's. The Pi 5 probe is `probe-clear` (`rpi5_clear`), run by
`tools/board.sh pi5`.

### 3.1 Carries over unchanged

- **Verdict lines** (`:86-89`, `:259-260`): `v3d_clear: PASS (...)` or
  `v3d_clear: FAIL (...)` with the reason, on a single line; one line per
  step with its raw result before it (`M1 power_on`, `M2 mmu_init`, `M3 bin`,
  `M3 render`, `M4`). The board runner's pass test is the string
  `v3d_clear: PASS`. Keep the exact text and the reasons (`M1 power-on`,
  `M1 identity`, `M2 page table`, `M2 MMU`, `frame sizes`, `frame build`,
  `M3 bin: out of memory`, `M3 bin: MMU fault`, `M3 bin: timeout`,
  `M3 render: MMU fault`, `M3 render: timeout`, `M4 pixels`, `M4 guard row`).
- **`print_stall`** (`:91-97`): `core_sts`, `hub_sts`, the two control-list
  current/end addresses, `err_stat`, `mmu_ctl`, `vio_addr`, from
  `struct v3d_stall`.
- **The M4 check** (`:239-260`): count every target pixel that is not the
  clear colour (`bad`), remember the first (x, y, value), count the guard
  row's changed pixels (`guard_bad`), print `M4: %u of %u target pixels wrong
  [(first at x,y: value)], guard row %u of %u changed`, and pass only when
  both are 0. The Pi 5 criterion in the plan is the same ("0 wrong pixels and
  an unchanged guard row").
- **Pre-fill** (`:205-214`): the target starts as a sentinel (magenta
  `0xffff00ff`) written by the CPU and the row above it as a guard (`0xff0000ff`
  red), so a job that writes nothing, or writes outside, fails. The clear
  colour `0xff00ff00` (R in the low byte; "green") is the expected value.
- **The order and the cache steps** (`:165-168`, `:214-216`, `:229-237`):
  fill the page table, `sys_cache_data_flush_range` of the page table; then
  `v3d_irq_quiesce`, `v3d_caches_init`, `v3d_mmu_init`; clear `tile_state`
  and flush the lists (`cls`) and the tile state to memory; submit the bin
  list, poll `FLDONE` with `v3d_wait`; submit the render list, poll `FRDONE`;
  `v3d_caches_clean` and only then read the target. These are the same
  calls (`v3d.h`), now with the 7.x register back end of `hw71`.
- **Mapping buffers** (`:45-68`): one page table, `va_next` from
  `0x00100000` (VA 0 never mapped), `gpu_map` calling `v3d_pte_map` and
  refusing out-of-range or unaligned (it prints `gpu_map: refused`), and the
  buffers: scratch page, three control-list pages (binning, render, tile
  list), `tile_alloc`, `tile_state`. The sizes `tile_alloc` 64 KiB and
  `tile_state` 64 KiB hold for any target that `v3d_frame_alloc_sizes` sizes
  below them (checked at `:189-193`, "frame sizes").
- **Run the frame builder through the tile tables**: `v3d_frame_tiling`,
  `v3d_frame_alloc_sizes`, the three `v3d_cl_init`, `v3d_frame_build_clear`
  (`:185-201`) and the `frame %ux%u: ...` summary line with the byte counts
  (`:202-203`). At 7.1 the builder's byte counts for a target within one tile
  are the numbers of section 1 (19 / 96 / 27 for 64x64; the probe prints them,
  and equality with `C` is a check of its own).
- **The end**: nothing runs afterwards; the image idles until the watchdog
  resets the SoC and tryboot brings Linux back (`:16-17`, `:263-268`). On the
  Pi 5 this is `wdog-lib`'s `rpi5_wdog`, armed before anything else.

### 3.2 Does not carry over

- **The framebuffer as target** (`:101-116`, `:152-160`, `:205-212`,
  `:241-250`): the Pi 4 sample asks the firmware mailbox for a 1280x720
  framebuffer and clears its bottom half; the console text is the top half.
  The plan needs no display on the Pi 5 and the mailbox for it is another
  item (`mbox-pi5`). The probe's target is a **static page-aligned buffer in
  the image**, a few whole tiles (for example 128x64 pixels, 4 bytes each),
  with one guard row of padding in the same buffer immediately above the first
  target row (as the Pi 4 kept the row above the target), pre-filled with the
  sentinel and guard values; the page table maps this one buffer. The console
  is `uart10`, so nothing shares the buffer with text.
- **`fbcon_attach`**, `rpi_fb_alloc`, `fb.pitch`, the 22-text-row scale
  comment (`:109-112`): gone. `printk` goes to `uart10` as the other Pi 5
  probes do.
- **The clock steps** (`:70-84`, `:118-121`, `:140-144`): the Pi 4's
  `clk_set` calls firmware clock 5 through `rpi_clock_state` and the V3D clock
  rate is read and set to the maximum through the mailbox. At 7.1 the same
  clock is firmware clock 5 but the call, and whether it is needed, is the D2
  result (`pre-study-rpi5.md` §6, §3.4; `mbox-pi5`/`probe-d2`). Do not copy
  `clk_set`: pass the `v3d_clock_ops` the Pi 5 power sequence needs
  (`v3d.h`: `v3d_power_on` for the BCM2712 is "SMS `CLEAR_POWER_OFF`, state
  polled, then the reset request" with the clock callback).
- **The register blocks** (`:123-130`): the Pi 4 maps `BCM2711_V3D_HUB_BASE`,
  `..._CORE0_BASE`, `BCM2711_PM_BASE` and `BCM2711_V3D_ASB_BASE` and prints
  `PM_GRAFX`, `ASB M/S/ID`. The Pi 5 maps hub, core and the SMS
  (`BCM2712_V3D_HUB_BASE`, `..._CORE0_BASE`, `..._SMS_BASE`,
  `v3d_regs.h:44-46`), has no ASB, and its pre-power print is the SMS state.
  Addresses above 4 GiB: `device_map` takes a 64-bit address and 40-bit
  mappings are the `zephyr-boot-state` item's result.
- **The address the GPU sees** (`:56-68`, `:157-160`): the Pi 4 sample hands
  the GPU `k_mem_phys_addr(...)` of each buffer. Whether that is what the
  Pi 5 V3D MMU wants (bus address against ARM physical address, U4/U10) is not
  settled by this note; `v3d_pte_map` refuses a page number over 24 bits (a
  36-bit address). A low `sram0` carve-out keeps both questions off the path
  until D2 reports `MMU_DEBUG_INFO.PA_WIDTH`.
- **Frame and tile size**: `v3d_frame_tiling(f.width, f.height, &t)` selects
  the 4.2 tile size (`:187`). At 7.1 the probe must use the 7.1 chooser of
  section 2.2 and pass the tile size into the packets; the 1-target 32 bpp
  clear is 64x64 either way.
- **`CLEAR_RGBA8` in the frame** (`:177-184`, `F`): the clear colour goes
  into `RENDER_TARGET_PART1`'s `Clear Color low bits` (32 bits), the internal
  type field is the `Render Target Type Clamp` value 8, not the 4.2 enum
  (section 1.2).
- **Wait times** (`:221`, `:230`: 1,000,000 us): the same polling; whether the
  7.1 core's interrupt status bits (`FLDONE`, `FRDONE`, `OUTOMEM`, the MMU
  violation) and the wait are the 4.2 ones is `hw71`'s landed back end
  (`v3d_hw.c`); the probe uses `v3d_wait` unchanged and keeps the reasons
  above.

### 3.3 What the Pi 5 probe adds

- The identity line equal to Linux's (D3: `v3d_ident` and the core IDENT
  words), `M1 identity`, printed in hex so `ident71` can decode it.
- `get_throttled` is a Linux-side record, not part of the probe; the board
  runner records it with the result (plan, "The supply is weak").
- The empty-list check of M3 (an empty binning list and an empty render list
  raise `FLDONE` and `FRDONE`) before the clear: `v3d_clear`'s single job is
  the clear itself, so the probe runs the clear and an empty frame as two
  stages if the plan wants "empty lists" reported on its own line.

## Open points handed on

- NUMBER_OF_LAYERS in the 7.1 frame: emit it or not (1.1).
- `Depth-buffer disable` for a clear-only frame: Mesa leaves it 0 (1.2, #1).
- The 4.2 byte count of the same frame (106 / 28) is computed from `X`, not
  measured; `frame_check` at 4.2 pins what the gate holds.
