<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set at the 256 MiB GPU carve-out, 2026-10-15, origin/main `e3749a25`

Session record `docs/board-sessions/am62p-common-carveout.md`; RESULTS.md
section "AM62P: common set on origin/main `e3749a25`, the 256 MiB GPU
carve-out (2026-10-15)"; records `records/am62p/common-e3749a25/` and
`records/am62p/common-e3749a25-sv2/` (gitignored).

## State of main

AM62P row, the gl_test, glmark2 and slint/cluster cells (add in front of the
`cd738b1a` readings):

- gl_test: 50 / 50, 0 resets, on `e3749a25` (2026-10-15, common set; `docs/board-sessions/am62p-common-carveout.md`).
- glmark2: 462, 33 scenes ran, 0 failed, 0 unsupported, 0 resets, **GL errors
  raised 0, no `render pool exhausted`, no `out of GPU memory`, no
  `framebuffer N incomplete`** on `e3749a25` (2026-10-15, common set, the
  first AM62P glmark2 run on the 256 MiB carve-out of `504d4fae`). terrain
  1 fps, shadow 3 fps and refract 1 fps remain, now CPU-side ("Known defects").
- slint / cluster: SO pass (crc16 c592, 56 fps), **SV pass (crc16 c592, the
  console captured in both of two boots; the `cd738b1a` empty capture is not
  reproduced)**, CL pass (crc16 110c, 73.1 fps, 44.9 Mpx/s at 1024x600) on
  `e3749a25` (2026-10-15, common set).

## Open work

- Item 9, "AM62P common-set follow-ups": the first two sub-items are done on
  the board. `504d4fae` removes the out-of-memory of terrain and shadow and
  refract's incomplete framebuffers (RESULTS.md, 2026-10-15); #1 and #2 are no
  longer out-of-memory on the AM62P, but no capture shows the pictures, so
  they are still not closed by a picture. SV was repeated twice with the whole
  console each time: the 2026-10-14 empty record was a capture flake of that
  attempt; nothing to fix in the image or the row.
- New: **PowerVR `glGenerateMipmap` on the CPU** (host item). terrain (800x600,
  10 levels, 5.6 MB) and refract (1600x1200, 11 levels, 22 MB) box-filter their
  chains on the CPU every frame (`src/pvr/backend.rs` `generate_mipmap: None`,
  `src/gl/texture.rs`'s CPU path): 1467 ms and 2046 ms of GL-call time per
  frame on the board, 1 fps each. The V3D's TFU path (2026-10-14) is the model;
  the PowerVR needs a GPU chain (TQ or a per-level draw). shadow's 353 ms per
  frame is not explained by this reading and needs kick times
  (`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y`) in a next GM image.
- `tools/cts/common-tally.py` prints "no glmark2-mem lines: a line goes to the
  next scene's result" for a console that has 66 `glmark2-mem:` lines (the GM
  console of this run); the verdict is right (no known-bad line), the remark is
  not. Small tool fix, host only.

## Known defects

- Close "AM62P glmark2 terrain and shadow out of GPU memory" (no issue number;
  `cd738b1a`, 2026-10-14): on `e3749a25` the GM console has 0 `render pool
  exhausted`, 0 `out of GPU memory`, 0 `framebuffer N incomplete` and 0 `GL
  error` lines, `GL errors raised 0` (was 2573); the pool is 255808 KiB, 0
  grow requests, firmware `out of memory 0` (was 59200 KiB, 4 grow requests, 5).
- Open "AM62P glmark2 terrain and refract at 1 fps, shadow at 3 fps: CPU time
  in GL calls" (no issue yet; `e3749a25`, 2026-10-15,
  `docs/board-sessions/am62p-common-carveout.md`): `glmark2-frame:` reads
  `gl 1467010 us` (terrain), `gl 352737 us` (shadow), `gl 2045892 us` (refract)
  per frame with `swap` in the tens of microseconds; one `fence ... not moved
  at 20 ms, re-posted` per terrain frame (the known re-post defect, 8 in the
  run, 0 GPU recoveries). refract's 85 fps on `cd738b1a` was the scene with
  its draws skipped; 1 fps is the drawn scene. Cause read in the sources, not
  measured: the PowerVR backend's `glGenerateMipmap` is the GL core's CPU box
  filter.
- The SV empty-console reading of 2026-10-14 is closed: not reproduced in two
  boots on `e3749a25` (17294 and 17296 console bytes, both `done`).

## Next board session

Nothing is needed from this item. When a GPU mipmap path for the PowerVR
lands, re-run GM (`tools/cts/cts-common.sh am62p build|run`) with
`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y` added to the GM image and read terrain,
shadow and refract's `glmark2-frame:` lines against 1467 / 353 / 2046 ms. Two
operational notes for whoever runs the AM62P next (no address in any file):
the board accepts ccfarm's node key, not this host's user key (give
`tools/board.sh` an ssh-agent holding the node key, or install the user key);
and another session's dEQP tree may hold the board and the bridge for hours
with long idle gaps between rows (its builds), so wait for a quiet board (no
`/var/yakogl/once-*.bin`, nobody in `who`, the bridge probe free) before a
run, because on the AM62P `TREE_RETRIES` is 0 and a refused row stops the
other session's chain.
