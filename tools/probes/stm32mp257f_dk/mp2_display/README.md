<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_display

The STM32MP257F-DK's display probe (issues #7 and #1281): LTDC -> DSI ->
ADV7535 -> HDMI through the Zephyr display API, then eight colour bars.

```
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_display \
  tools/probes/stm32mp257f_dk/mp2_display
```

## What it does

`src/main.c` gets the LTDC device (`ltdc` in the overlay), writes the bars
(`src/pattern.c`) with `display_write()`, calls `display_blanking_off()` and
prints one line:

- `mp2_display: PASS: <w>x<h> <fmt>`
- `mp2_display: FAIL: <step> <err>` (steps: `ltdc-ready`, `format`, `size`,
  `pattern`, `write`, `blanking-off`)

It then keeps scanning out for `CONFIG_MP2_DISPLAY_HOLD_S` seconds (default
60) before `main()` returns, so the runner's `MP2_HOLD_S`
(tools/board-stm32mp2.sh) and the KVM can see the picture. `prj.conf` raises
the board layer's `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` to 150 so the layer
does not reset the board during the hold; a `BUILD_ASSERT` keeps the two
consistent.

**While the four drivers are compiling shells** (driver/drivers/display,
driver/drivers/mipi_dsi, driver/drivers/i2c answer `-ENOSYS`), the probe
prints `FAIL: write -38`. That is expected.

## The overlay

`boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay` adds, under `/soc`, the
nodes of ST's tree as yakogl devices (bindings in driver/dts/bindings):
`i2c2` (0x40130000) with the `adv7535` child (reg 0x3d, edid 0x3f, cec 0x3c,
packet 0x38, four lanes), `dsi` (0x48000000, `bridge = <&adv7535>`) and
`ltdc` (0x48010000, 1280x720 `argb8888`, `dsi-host = <&dsi>`). The board
and SoC files are untouched. Left out: the bridge's INT and reset GPIOs (no
GPIO controller in the tree yet), the LTDC interrupts, clocks/resets/RIF.
Placeholders until the `mp2_display_state` probe reads them: the I2C kernel
clock (64 MHz) and the DSI PLL reference (40 MHz).

## The pattern and its host test

Eight equal vertical bars: white, yellow, cyan, green, magenta, red, blue,
black (display_check's `colorbars.py` `BARS`). Formats: ARGB8888 (alpha
0xff), RGB888 (bytes B, G, R), RGB565. The stride is a parameter; the bytes
after each line's pixels are zeroed.

```
bash tools/probes/stm32mp257f_dk/mp2_display/test/pattern_test.sh
```

compiles `src/pattern.c` with the host `cc`, fills 1280x720, 1920x1080 and
800x480 buffers in each format (also with a padded stride), writes them as
PPM, and requires `colorbars.py` to answer `PASS: 8 bars white yellow cyan
green magenta red blue black`; it also checks the C colour table equals
`BARS`. Needs `cc` and `python3`.

The picture on the board is judged by
`tools/probes/stm32mp257f_dk/display_check/colorbars.py` from a KVM
screenshot.
