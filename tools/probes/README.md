<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Probes

Zephyr images that measure one thing on one board and are not samples
(`driver/samples/zephyr` holds `gl_test`, `deqp`, `glmark2` and the Vivante
bring-up samples `gc_fence` and `gc_clear`; `triangle` and `slint` are under
`examples/`). `tools/check.sh` builds each with `-DYAKOGL_WERROR=ON` (the
`zephyr` rows, by path: `zephyr_werror_image`).

- `nanopc_t6/t6_hello`: milestone Z1 on the NanoPC-T6, the build line and the
  watchdog arming (`driver/docs/nanopc-t6-mali.md`).
- `nanopc_t6/t6_gpu`: the second half of Z1, the GPU's power at hand-over.
- `nanopc_t6/t6_mmu`: milestone Z2, AS0's page tables from the image and a
  GPU cache flush round trip, after Z1's power-on.
- `nanopc_t6/t6_fw`: milestone Z3, the Mali CSF firmware boot after Z2's page
  tables (board module `driver/rust/src/board/nanopc_t6_fw.rs`, C entry
  `yd_t6_fw_up`). It prints the build, the firmware digest and Z3's record on
  uart2. The image embeds `driver/src/fw/mali_csffw_blob.h`, which
  `driver/tools/mkmalifw.sh` makes from the board's firmware file and which is
  never committed.
- `nanopc_t6/t6_submit`: milestone Z4, after Z3's firmware boot: CSG 0 / CS 0,
  the tiler heap, the L1 frame from `driver/rust/src/board/nanopc_t6_frame.rs`
  submitted and its sync object waited on (board module
  `driver/rust/src/board/nanopc_t6_submit.rs`, C entry `yd_t6_submit`). It
  prints Z3's record and the Z4 record as the lines Z4a to Z4i on uart2, then
  PASS or FAIL. It embeds the same never-committed
  `driver/src/fw/mali_csffw_blob.h`. Host side only, not run on the board
  (`driver/docs/nanopc-t6-mali.md`, "Z4 board procedure").
- `stm32mp257f_dk/mp2_identify`: the STM32MP257F-DK's return-trial image, the
  hand-over record and OP-TEE's watchdog armed at boot, then PSCI reset.
- `rpi_5/*`: the Raspberry Pi 5 probes (`tools/probes/rpi_5/README.md`).

Gate rows (`tools/check.sh`, reached by a change under `tools/probes/` or
`driver/`):

- `zephyr  rpi_5 probe images`: every directory of `rpi_5/` with a
  `CMakeLists.txt` and a `prj.conf`, `west build -b rpi_5` with
  `-DYAKOGL_WERROR=ON`. Without `ZEPHYR_WS` it is the one gap naming it.
- `zephyr  nanopc_t6 t6_fw image (Z3, the firmware boot), -Werror on our C
  files`: builds `nanopc_t6/t6_fw`. It is a gap without `ZEPHYR_WS` or the
  CSF firmware header (made from the board's file first when one is given).
- `test    rpi_5 fdt_walk host test`: `rpi_5/common/fdt/test/run.sh` (host cc,
  ASan/UBSan, no Zephyr). Ok only with `dtc` on PATH and `fdt_walk: N checks,
  0 failed` without a SKIPPED line; without `dtc` it is a gap naming it. If it
  fails, `test/test_fdt.c` lines 211 to 231 hard-code dtc's layout.

Build (from the west workspace):

```sh
west build -p always -b nanopc_t6 -d build/t6_hello yakogl/tools/probes/nanopc_t6/t6_hello
```
