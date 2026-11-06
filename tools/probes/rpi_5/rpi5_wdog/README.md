<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_wdog

The Raspberry Pi 5's first board fact (`docs/notes/pre-study-rpi5.md`, section
6): does the BCM2712 PM watchdog reset the board? The board has no remote
power, so every Pi 5 image arms it first; this is the smallest image that shows
it works.

What it does: the board layer (`driver/zephyr/lib/board/rpi4/rpi_wdog.c`, the
SoC a parameter, `CONFIG_YAKOGL_RPI5_WDOG`) arms the `pm` watchdog
(`0x10_7d200000`, 10 s period) in the first `PRE_KERNEL_1` hook and feeds it
every 3 s for 30 s. The image prints the build, a line per feed interval, then
`rpi5_wdog: feeding stopped, reset expected within 10 s`. The board resets;
with `tryboot` it comes back into Linux. If it is still alive 15 s past the
period it prints `rpi5_wdog: FAIL no reset` and spins: **a person must then
power-cycle the board**.

Shown by: the console (uart10) goes quiet after the stopped line and the
boot loader's log follows; `board: PM watchdog resets` appears once.

Build (from the west workspace; `tools/check.sh` builds it with
`-DYAKOGL_WERROR=ON`):

```sh
west build -p always -b rpi_5 -d build/rpi5_wdog yakogl/tools/probes/rpi_5/rpi5_wdog
```

Run: `tools/board.sh pi5` (item runner-pi5), tryboot with `kernel=` naming the
staged `zephyr.bin`; the lifetime to wait for is about 30 s + 10 s + the
reboot.
