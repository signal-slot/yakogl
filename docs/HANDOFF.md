<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Handoff

The state of the work and what comes next. Update this file in the same
commit as any result that changes it. Last update: 2026-10-05.

Board addresses, UART paths and credentials never go in this file; they come
from the environment (`BOARD_HOST`, `BOARD_UART`, ...).

## Goal, in order

1. ES 2.0 finished on the three boards (AM62P PowerVR, i.MX8MP GC7000,
   Raspberry Pi 4 V3D), each booted from its own storage by its bootloader.
2. Minimum performance.
3. A public website (`website/`). It goes live when the repository is public.
4. ES 3.1 / 3.2.

## State of main

Results are from main `39bd6983`. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 47 / 47, 0 resets | pass | 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx |
| i.MX8MP | 45 / 47 (first failure case 16; the second is not named in the record) | not run on `39bd6983` | not run on `39bd6983` |
| Pi 4 | 12 / 12 on `afa12e9b` (not run on `39bd6983`) | not run on `39bd6983` | not run on `39bd6983` |

AM62P, the fill shader from femtovg (`fill_color`):

| Measurement | Run-time compiler (`39bd6983`) | Mesa table (`afa12e9b`) |
|---|---|---|
| gl_test case 39, perf 2 (16 full-screen quads), 3D busy | 81.1 ms | 83.4 ms |
| case 186 arm 0 | 80.9 ms | 83.4 ms |
| case 186 arm 1 | 55.3 ms | 54.9 ms |
| case 186 arm 2 | 69.8 ms | 71.6 ms |

The table column comes from an earlier image. No table image of `39bd6983` was run.

## Open work, in order

1. **Land the join removal**: branch `work/pvr-blend-factor-once`, commit
   `c685c549`. When both arms of a predicated `if` compute a value, each arm
   writes one register, as Mesa's pco does, so no select follows the `if`.
   It adds gl_test 192 `predlive`. The full gate is clean (96 ok, 0 failed,
   0 gaps). It has not run on a board.
   Host count for `fill_color.nodiscard`: 105 -> 104 groups, with 69 groups
   on the executed path (the table also has 69).
   Board run after landing:
   - AM62P: GT, case 39, cases 186 + 187, Slint, cluster;
   - i.MX8MP: GT.
2. **Does a predicated-off instruction group cost time?** Measure it on the
   AM62P. Grow the off arm of the same program by N groups, once compiled by
   our compiler and once from the table, and read the 3D busy time.
   An old gap of about 5.5 ms in case 186 arm 1 (on `afa12e9b`) is not
   explained by the group counts. Re-measure it on main first: the
   `39bd6983` numbers above may have closed it.
3. **Compile shaders at build time with our own compiler.** Shaders known
   at build time are compiled then; the run-time compiler runs only on a
   miss. An image without the run-time compiler refuses a miss by name.
   Mesa's `pco_shc` / `etna_shc` then compile nothing.
   Then rename the two paths to "precompiled shaders" and "shader compiler"
   (features `precompiled-shaders` and `shader-compiler`; the source moves
   from `src/shaderc/online` to `src/shaderc/compiler`). The words online,
   offline and remote go away.
4. **Design clean-up**:
   - `DESIGN.md` and `docs/` describe the current design, not its history.
   - About 50 debug and experiment globals in product code are removed or
     moved out.
   - The 26 samples are cut to the few that are needed.
   - The bare-metal claim is made true, or it is removed.
   - The `SAFEUI_*` and `powervr-bxs` names are removed.
   - Our remaining C (the OSAL, `gl_test.c`) moves to Rust.
5. **Known defects**, each with a GitHub issue:
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (nondeterminism; the
     cause is not found);
   - i.MX8MP case 16 (4x MSAA) fails;
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases.
6. `tools/cts/STATUS.md` was last updated on 2026-10-04 and needs to catch
   up with the results above.

## How to work here

- The gate is `tools/check.sh`. Run it from a clean detached checkout of
  the exact commit, with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>` and `HAL_NXP_DIR=<hal_nxp>` set. Without
  them the Zephyr and hal_nxp rows are gaps and the gate is red. It takes
  about 45 minutes.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- One GL client per board. Check that a board is idle before you use it.
