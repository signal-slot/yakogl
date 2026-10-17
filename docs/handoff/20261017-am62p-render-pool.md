<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-render-pool, 2026-10-17, plan item baseline3-am62p (from main `c854e0ae`)

Host investigation, no code change:
`docs/notes/am62p-render-pool-glmark2.md`.

## State of main

No board number changes. Add one sentence after the ES 2.0 table: why the
AM62P glmark2 image needs the 256 MiB carve-out of `504d4fae`. On the
board's 64 MiB, 25224 KiB of the 59200 KiB render pool is the driver's
persistent set (freelists 12440, frame sets 7488, statics, the window's
depth buffer). The NPOT mip chains laid out at their POT extent (800x600
chain 5592404 bytes, 1600x1200 chain 22369620 bytes) then do not fit
terrain or refract in total. It was not fragmentation and not a leak
(`docs/notes/am62p-render-pool-glmark2.md`, sections 1 and 2).

## Open work

Item 1 (stage 2), add under it:

- **AM62P glmark2 render pool: pin the 256 MiB fix on the host**
  (pvr-pool-npot). The fix is on main (`504d4fae`, board `e3749a25`: 0
  `render pool exhausted`, 0 incomplete framebuffers, 0 GL errors). The
  test is designed in the investigation's last section: the
  `pool/tests/replay.rs` sequence with POT-extent chains and freelists sized
  per carve-out (`driver/src/pvr_render.c:632-675`). It must show
  refusals for want of memory at 64 MiB and no refusal at the overlay's
  carve-out with every entry under 3/4 of the pool (model now: refract
  139356 of 255808 KiB).
- Measured on the host harness (PowerVR build, per-entry heap
  high-water): terrain's scene needs 38732 KiB and refract's 45160 KiB
  above the back end's fixed part, so with the driver's 25224 KiB
  persistent set they peak at 63956 and 70384 KiB, both over the 64 MiB
  image's 59200 KiB pool. At 256 MiB refract's predicted 140144 KiB
  matches the board's 139212. Left: why the board had about 3.6 MiB more
  room at terrain than host and replay say (it served one of the two
  chains); it does not change the answer.
- A decision, low: from 128 MiB up the render service takes a quarter of
  the carve-out as the freelist grow pool (`driver/src/pvr_render.c:632-650`), 64 MiB
  at 256 MiB, and glmark2 asked for 0 grows at `e3749a25`. A 120 MiB
  carve-out leaves more room for a scene than a 128 MiB one.
- `docs/investigations/` was retired on 2026-10-16 (`docs/README.md`). This
  plan item asked for the file there. Move it to `docs/notes/` when the
  pvr-pool-npot test cites it.

## Known defects

- Remove "AM62P glmark2 terrain and shadow out of GPU memory" if a fold
  still finds it: closed by `504d4fae` at `e3749a25`. Its 21848 KiB request
  was **refract's** 1600x1200 colour chain (entry 24 in f8741799's record),
  not shadow's; shadow has only a 1600x1200 depth texture without mipmaps.
- **A refused NPOT chain grow leaves the texture with no storage** (no
  issue yet, low): `texture_alloc_laid_out` releases the level-0 storage
  (`src/gl/texture.rs:1598`) before it asks for the chain (`:1602`). When
  that is refused, the texture loses level 0 too, and every later frame
  reports `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` (refract's 1694 lines at
  64 MiB) instead of drawing level 0. GL allows undefined contents after
  `GL_OUT_OF_MEMORY`, and the 256 MiB image never reaches this path.
  Allocating the chain before the release would raise the peak by the
  level-0 size (7500 KiB for refract).

## Next board session

AM62P, no extra run: in the next common set's GM console, check that the
`render pool summary` line keeps bump plus top high-water under 3/4 of
255808 KiB (`e3749a25`: 91356 + 56716 KiB). If a 64 MiB GM record from
cd738b1a or f8741799 is at hand, read the ERR `render pool general: free
... largest ...` line after each `render pool exhausted` and the `frees
held` field of `glmark2-mem: entry 21 ... end`. They confirm sections 1 and
2 of the investigation on the board.
