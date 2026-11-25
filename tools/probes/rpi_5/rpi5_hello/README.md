<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_hello

`hello_world` for the Raspberry Pi 5 (`rpi_5`, BCM2712): item H9 of
`docs/notes/pre-study-rpi5.md`. It prints the build line, the board, the
console and the memory the image was linked into, then counts a few seconds on
the system timer. No GPU code.

It carries the board layer's PM watchdog, the way every other
`tools/probes/rpi_5` probe does: `CONFIG_YAKOGL_RPI5_WDOG=y` links
`driver/zephyr/lib/board/rpi4/rpi_wdog.c`, which arms the BCM2712 PM watchdog
in the first PRE_KERNEL_1 hook, feeds it every 3 s until 30 s after boot
(`CONFIG_YAKOGL_RPI5_WDOG_HOLD_S=30`), and then stops — the SoC does a full
reset within one watchdog period, and from a tryboot boot that reset brings
Linux back with nobody at the power switch. A hang cannot hold the board. The
console carries `board: watchdog armed` and the announce line
`board: PM watchdog resets the SoC ~40 s after boot (tryboot -> Linux)` (the
40 is the 30 s of feeding plus one 10 s watchdog period: the latest the reset
can come). The announce's fixed prefix `board: PM watchdog resets` and the
armed line are what `tools/board.sh pi5` greps the image for before booting,
so it no longer refuses this image (its `NETBOOT_NO_WATCHDOG=1` override is
not needed). `boards/rpi_5.overlay` resizes `sram0` to 32 MiB from 0x200000, as
`driver/samples/zephyr/common/rpi_4b_sram.dtsi` does for the Pi 4.

## Verdicts

`verdicts.sh <console.log>` checks the count line `src/main.c` ends with,
`rpi5_hello: counted 3 s in <ms> ms`: it passes when the count is 3 s and
`<ms>` lies in 3000..4000. It prints `verdicts: PASS|FAIL ...` and exits 0
(done), 1 (no end line), 2 (usage) or 3 (FAIL). `verdicts_selftest.sh` checks
it against inline logs (good, short count, slow, truncated).

Build (from the west workspace; add `-- -DYAKOGL_WERROR=ON` for zero warnings
in our files):

```sh
west build -p always -b rpi_5 -d build/rpi5_hello yakogl/tools/probes/rpi_5/rpi5_hello
```
