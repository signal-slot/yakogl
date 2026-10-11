<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 1024x600 render-buffer sizing vs 64x64 (BXS-4-64, BVNC 36.53.104.796)

Status: finished record, moved here from `driver/docs/gpu-recipes/fullpanel_sizes.md` on 2026-10-11. Concluded that the full-panel buffers were not undersized; the failure was in the emit/ISP path. The render service sizes buffers per target (driver/src/pvr_render.c).

Conclusion of a read-only bring-up analysis, cut down. The original compared
our buffer sizes with words captured from the DDK's own 1024x600 render; those
captured register values are not reproduced.

## Conclusion

The hypothesis "the full-panel render buffers are undersized" is not
supported. A 1024x600 render needs only modest tiling buffers, and every one of
our buffers was already at least as large as the DDK needed for that size.
The full-panel failure was an emit/ISP-path problem, not a sizing one.

## The 1024x600 macrotile grid (arithmetic)

| Register | Meaning at 1024x600 |
|----------|---------------------|
| SCREENPIXELMAX | X max 1023, Y max 599 |
| TESCREEN | 64 tiles in X, 38 in Y: 2432 tiles of 16x16 |
| TEMTILE1 / TEMTILE2 | padded tile counts: 64 in X, 40 in Y |
| MTILESTRIDE | 160 B: one region-header row = 10 macrotiles x 16 B per header |
| RGNHEADERSIZE | 640 B: 40 macrotiles x 16 B |
| ISPMTILESIZE | macrotile of about 16 x 10 tiles |

rgn_header_size / mtile_stride gives 4 macrotile rows of 10; 10 x 4 = 40
macrotiles; padded Y of 40 tiles / 4 rows = 10 tiles per macrotile row. The
per-macrotile header is 16 B at any size: 64x64 has one macrotile
(rgn_header_size 16), 1024x600 has 40 times as many.

## Per-buffer need (one 1024x600 render per frame)

| Buffer | Need | Verdict |
|--------|------|---------|
| macrotile array | none: the HWRTData macrotile address is NULL (`PVR_MACROTILE_NULL`) | size irrelevant, do not grow it |
| region headers | field 0x280; the TA writes headers for all covered macrotiles, about 0xf00 per render | 64 KiB is ample |
| TPC | 0x2000 per render | 16 KiB is ample |
| VHeap | 0x600 per render, VA slot 0x2000 | tight, zero margin; 0x2000 matches the slot |
| macrotile list | 0x1000 | exact |
| local freelist | 26 initial, 32 max, 17 pages per kick | exact |
| global freelists | 512 pages | ample for one triangle |
| parameter buffer | far below 2 MiB for one triangle | ample |
| destination / framebuffer | 1024x600x4 | fine |

Only the VHeap had no margin, and it does not scale with resolution (the same
three vertices at 64x64 and 1024x600), so it cannot be what separates the
working small render from the failing full-panel one. Giving it 0x2000 is cheap
insurance. The geometry command's `te_psg` is resolution-independent
(`completeonterminate`, region stride 0: the region stride is carried by the
macrotile stride in the HWRTData). The TA does write region headers at full
panel, so tiling metadata lands; an undersize would show truncated or aliased
header writes.

## Where the full-panel failure was

Not the tiling buffers. The paths that differ between a 64-pixel render and the
full panel are:

- PBE line stride at pitch 4096: `(pitch_dwords - 1) / 2` (= 511 for a
  1024-dword row), validated only against a 64-pixel row. An off-by-one writes
  to the wrong stride and the panel shows nothing.
- The framebuffer remap and its 1024x600 mapping, taken only at full panel; a
  wrong VA or extent gives an all-empty panel with no hardware reset.
- `ppp_ctrl` against Mesa's value (affects the VS coordinate format); it would
  break the small render too, so it ranked lower.
