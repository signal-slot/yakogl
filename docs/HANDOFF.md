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

Results are from main `81c53c61`, except the Pi 4. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 48 / 48, 0 resets (the set now has 192 `predlive`) | not run on `81c53c61`; pass on `39bd6983` | not run on `81c53c61`; 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx on `39bd6983` |
| i.MX8MP | fails: first failure case 16 check 255 (4x MSAA), the same as on `39bd6983`, where it was 45 / 47 | not run on `81c53c61` | not run on `81c53c61` |
| Pi 4 | 12 / 12 on `afa12e9b` (not run since) | not run since | not run since |

Slint and cluster are not run on `81c53c61`: their images need a Rust
nightly with rust-src and the Slint no_std workspace, which the build host
does not have yet.

AM62P, the fill shader from femtovg (`fill_color`), 3D busy per frame:

| Measurement | Run-time compiler `81c53c61` | `39bd6983` | Mesa table (`afa12e9b`) |
|---|---|---|---|
| gl_test case 39, perf 2 (16 full-screen quads) | 77.19 ms | 81.1 ms | 83.4 ms |
| case 186 arm 0 (blend) | 76.98 ms | 80.9 ms | 83.4 ms |
| case 186 arm 1 (nodiscard, blend) | 54.56 ms | 55.3 ms | 54.9 ms |
| case 186 arm 2 (blend off) | 68.93 ms | 69.8 ms | 71.6 ms |
| case 186 arm 3 (nodiscard, blend off) | 44.00 ms | | |
| case 187 arms 0 / 1 / 2 / 3 | 81.02 / 59.05 / 72.52 / 47.94 ms | | |

The table column comes from an earlier image. No table image of `81c53c61`
was run.

The offline PowerVR table (`src/shaderc/blobs.rs`) records, since `0fe778f6`,
the toolchain that reproduces it byte for byte: a `pco_shc` built by
`tools/pco_shc/build_pco_shc.sh` with the patched usclib
(`// toolchain: pco_shc=f537fc86b41f glslang=9e007f4bf514`). That commit also
adds `matattr`'s 20 rows and re-keys `texsize.frag` to its current text.
On the AM62P (`60ffeec8`, table image) the regression set passes 39 / 40,
0 resets; the one failure is case 48 `es3vol`, which was already failing in
table images. Case 46 `es3size` (it uses `texsize.frag`) passes now; it was
refused before the re-key. Case 63 `matattr` passes, served from the table
for the first time.

## Open work, in order

1. **Slint and cluster on the AM62P for the join removal.** The join
   removal landed as `81c53c61`: when both arms of a predicated `if`
   compute a value, each arm writes one register, as Mesa's pco does, so no
   select follows the `if`. It adds gl_test 192 `predlive`. The full gate
   is clean (96 ok, 0 failed, 0 gaps). Board results on `81c53c61`: AM62P
   gl_test 48 / 48, case 39 and cases 186 + 187 pass, i.MX8MP gl_test
   unchanged (first failure case 16). What is left of its board run is
   Slint and cluster on the AM62P; they wait for a Rust nightly with
   rust-src and the Slint no_std workspace on the build host.
2. **Does a predicated-off instruction group cost time?** Measure it on the
   AM62P. Grow the off arm of the same program by N groups, once compiled by
   our compiler and once from the table, and read the 3D busy time.
   The old gap of about 5.5 ms in case 186 arm 1 is closed: on `81c53c61`
   it is 54.56 ms against the table's 54.9 ms. What is left is only whether
   a predicated-off group costs time. gl_test 193 `offarm` is that
   experiment: 186 arm 1 with the arm radius 0 never runs grown by N = 0,
   8, 32, 128 dependent steps (`tools/shaders/offarm/`). Both programs
   predicate the arm at every N and run the same 73 groups at radius 0:
   online 104 to 234 groups, the table (rows in `blobs.rs`) 105 to 235.
   Board run needed, on the AM62P: case 193 in an online image and in a
   table image.
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
