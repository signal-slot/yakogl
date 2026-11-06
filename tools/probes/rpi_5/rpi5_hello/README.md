<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_hello

`hello_world` for the Raspberry Pi 5 (`rpi_5`, BCM2712): item H9 of
`docs/notes/pre-study-rpi5.md`. It prints the build line, the board, the
console and the memory the image was linked into, then counts a few seconds on
the system timer. No GPU code.

Build-only: it has never run on a board. It carries no watchdog, so a hang
would hold the Pi 5 until a person power-cycles it; `tools/board.sh` refuses
it. `boards/rpi_5.overlay` resizes `sram0` to 32 MiB from 0x200000, as
`driver/samples/zephyr/common/rpi_4b_sram.dtsi` does for the Pi 4.

Build (from the west workspace; add `-- -DYAKOGL_WERROR=ON` for zero warnings
in our files):

```sh
west build -p always -b rpi_5 -d build/rpi5_hello yakogl/tools/probes/rpi_5/rpi5_hello
```
