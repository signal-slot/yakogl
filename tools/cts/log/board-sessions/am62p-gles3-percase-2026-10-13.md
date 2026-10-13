<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# AM62P board session: dEQP-GLES3 per case, and the depth-PBO trace (2026-10-13)

Item oq-gc of plan #19 (ccfarm `01a13578-d134-74b9-a507-0227e984ec7b`).

**Status: in progress, no result yet.** The board firmware was fetched and
`driver/src/fw/pvr_fw_blob.h` generated (not committed). No stage 1, image or
board run has happened yet: the build and run scripts were waiting for
harness approval when this was written. Results are filled in below as the
images run.

## Console

The lease named `usb-serial-1` port 35932, which carries mcu_uart0 (ttyS3).
Port 35933 of the same bridge carries main_uart0 (ttyS2). A write to ttyS2
over ssh arrived clean on 35933 (the probe table is in
`farm-console-blocked-2026-10.md`, row 2026-10-13). This session uses
`BOARD_UART=<bridge>:35933`.

## Inputs

- Firmware: the board's `/lib/firmware/rgx.fw.36.53.104.796`, 229,376 B,
  sha256 `32c8b3d7730a523b…` (the same on the board and in the copy), turned
  into `driver/src/fw/pvr_fw_blob.h` by `driver/tools/mkfwblob.sh`.
- Environment: the values of `pi4-deqp-stage1-gles3.md` ("Environment"),
  all under the farm's dependency cache.
- Stage 1: `MODULE=gles3 BOARD=verdin_am62p/am62p54/a53
  tools/cts/cts-build-zephyr.sh`, `WORK=<deps>/deqp-stage1-am62p-gles3/work`.
- Images: `DEQP_MODULE=gles3 tools/cts/am62p-deqp-images.sh
  tools/cts/am62p-gles3-tree.txt <out> <tag>` (28 images).
- Board: `BOARD_BOOT=uboot-mmc BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt
  tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt <rec>`.

## Results

## Depth-PBO trace

## Not run
