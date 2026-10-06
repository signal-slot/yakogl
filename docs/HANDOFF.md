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

Results are from main `81c53c61` (gl_test) and `7dada1ff` (Slint, cluster, table image), except the Pi 4. All three boards boot standalone from eMMC (the two Verdins, U-Boot) or SD (Pi 4).
`docs/standalone-boot.md` has the procedure.

| Board | gl_test | Slint | Cluster |
|---|---|---|---|
| AM62P | 48 / 48, 0 resets (the set now has 192 `predlive`) | pass on `7dada1ff` (705 frames in 10007 ms) | 91.8 fps, 56.4 Mpx/s, 16.28 ms/Mpx on `7dada1ff` (`39bd6983`: 91.2 fps, 56.0 Mpx/s, 16.11 ms/Mpx) |
| i.MX8MP | fails: first failure case 16 check 255 (4x MSAA), the same as on `39bd6983`, where it was 45 / 47 | not run on `81c53c61` | not run on `81c53c61` |
| Pi 4 | 12 / 12 on `afa12e9b` (not run since) | not run since | not run since |

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

Case 193 `offarm` (`7dada1ff`): 186 arm 1 with the never-run arm grown by N
groups, 3D busy per frame (record `20261005-2118-case193-7dada1ff-ad3d86cf`
online, `20261005-2119-case193-table-7dada1ff-ed2efe33` table; pixels equal
at every N):

| Added groups N | Run-time compiler | Table |
|---|---|---|
| 0 | 54564 us | 54851 us |
| 8 | 55415 us | 56008 us |
| 32 | 59323 us | 60633 us |
| 128 | 91142 us | 91745 us |

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
The GC7000 table still lags on `matattr`: issue #22, not fixed by `0fe778f6`.

A never-run arm of a predicated `if` still costs time (gl_test 193, 194 on the
AM62P): 0.11 ms a group for a small arm, rising to 0.38 ms a group beyond
about 128 groups (an executed group is about 0.79 ms). A branch has a fixed
cost of 5336 us a frame, about 6.8 executed groups (195). The default lowering
(`select::branch_pays`, `5f4d3677`, board-proved: gl_test 48 / 48, record
`20261006-0403-gt-5f4d3677-ecce9b75`) branches an `if` iff the masked cost of
each arm exceeds that fixed cost, which is crossed at 49 groups an arm; 0 of
168 corpus `if`s branch, and `fill_color` (31 / 23 groups) stays predicated.
`PVR_BACKEND_OPT_IF_LOWERING` PREDICATE and BRANCH force a form. If a real
shader ever shows a large arm under a uniform condition, the step is a per-draw
variant chosen by the GL core from the condition's value (predicated when the
long arm runs, branched when it is skipped; `fill_color` at N = 128 would go
from 91.1 to 55.3 ms with the arm off); it is not planned now.

## Open work, in order

1. **Compile shaders at build time with our own compiler.** Shaders known
   at build time are compiled then; the run-time compiler runs only on a
   miss. An image without the run-time compiler refuses a miss by name.
   Mesa's `pco_shc` / `etna_shc` then compile nothing.
   Done so far: the one source list every table generator reads
   (`tools/shaders/precompiled.list`) and `yakogl-precompile`, which
   compiles it with our compiler on the host into `build/precompiled/`
   (tools/README.md); next, an image carries that table, and the Mesa
   tables become host-test reference material only.
   Then rename the two paths to "precompiled shaders" and "shader compiler"
   (features `precompiled-shaders` and `shader-compiler`; the source moves
   from `src/shaderc/online` to `src/shaderc/compiler`). The words online,
   offline and remote go away.
2. **Design clean-up**:
   - `DESIGN.md` and `docs/` describe the current design, not its history.
   - About 50 debug and experiment globals in product code are removed or
     moved out.
   - The 26 samples are cut to the few that are needed.
   - The bare-metal claim is made true, or it is removed.
   - The `SAFEUI_*` and `powervr-bxs` names are removed.
   - Our remaining C (the OSAL, `gl_test.c`) moves to Rust.
3. **Known defects.** Each has a GitHub issue; what remains is the fixes:
   - i.MX8MP gl_test case 10 fails about 1 run in 3 (nondeterminism; the
     cause is not found) (#9);
   - i.MX8MP case 16 (4x MSAA) fails (#8);
   - the Pi 4 differs from the ES 2.0 reference in 25 cube cases (#12).

## How to work here

- The gate is `tools/check.sh`. Run it from a clean detached checkout of
  the exact commit, with `driver/src/fw/pvr_fw_blob.h` copied in, and with
  `ZEPHYR_WS=<west workspace>` and `HAL_NXP_DIR=<hal_nxp>` set. Without
  them the Zephyr and hal_nxp rows are gaps and the gate is red. The
  Mesa-table reproduction row also needs `PCO_SHC=<pco_shc from
  tools/pco_shc/build_pco_shc.sh, content id f537fc86b41f>`,
  `GLSLANG=<glslangValidator>` and, for speed, `GLSL2USC_CACHE=<dir>`;
  without them that row is a gap and the gate is red. It takes about 45
  minutes.
- Board runs use `tools/board.sh <board> run <zephyr.bin> <record dir>
  <label>`. Run them from a detached checkout of main, and only with images
  built from commits on origin/main. Use `BOARD_BOOT=uboot-mmc` on the
  Verdins.
- One GL client per board. Check that a board is idle before you use it.
