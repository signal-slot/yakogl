<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-common-int: AM62P common set at the integration tip, 2026-10-14, integration cd738b1a

Details: `docs/board-sessions/am62p-common-int.md`.

## State of main

AM62P common set at `cd738b1a` (build id `d5cb878`): GM `done`, score 465, 33
scenes ran, 0 failed; SO `done` (crc16 c592); SV `empty` (0 console bytes,
unresolved, same image as SO); CL `done` (crc16 110c, 75.3 fps, 46.3 Mpx/s at
1024x600); GT `done`, 50 of 50.

## Open work

- AM62P glmark2: the score passes but **terrain (1 fps) and shadow (3 fps) run
  out of GPU memory** (`render pool exhausted`, 5464 KiB and 21848 KiB
  requested, 3648 and 2744 KiB left) and **refract raises 1694
  `framebuffer 1 incomplete (0x8cd6)` errors**. Defects #1 and #2 are not closed
  on this board. The render pool (59200 KiB, ~38 MB live at jellyfish's end)
  does not fit terrain's 800x600 NPOT mip chain (5.6 MB) after jellyfish; the
  `pvr-pool-npot` replay item is the host side of this.
- SV on the AM62P returned no console bytes; repeat once to tell a boot or
  bridge flake from a real failure.

## Known defects

- Terrain and shadow memory failure on the AM62P (above), no issue number yet.

## Next board session

- Re-run SV alone. Fix the pool (or the order of allocations) and re-run GM;
  watch for `render pool exhausted` and `out of GPU memory` in the console, the
  tally's "GL errors raised" is `15 or more` and does not name the scene.
- The console is the bridge's main_uart0 port, not the lease's mcu_uart0 port.
