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

Before the drivers initialise, the board layer powers the path
(`driver/zephyr/lib/board/stm32mp2/display_power_mp2.c`, over the crate's
`stm32mp2_display.h`): RIF check, OP-TEE's SCMI (HDMI 5 V on LDO5, the
clocks, FLEXGEN_27 at 148.5 MHz), the RCC gates and resets, the pins, the
ADV7535's reset pulse; every request on the console as `display_power: ...`.

`src/main.c` then uses the LTDC device (`ltdc` in the overlay), the chain
owner (`driver/drivers/display/display_yakogl_stm32_ltdc.c`):
`display_get_capabilities()` makes it read HPD and the EDID through the
ADV7535 and choose the mode (1920x1080@60 when the EDID prefers it, else
1280x720@60); the probe writes the bars (`src/pattern.c`) line by line with
`display_write()` into the driver's frame buffer, calls
`display_blanking_off()` (LTDC, DSI host, ADV7535 up in that order), prints
the chain's record (`mp2_display: chain ...`, `mode ...`, `ltdc ...`,
`dsi ...`, `adv7535 ...`) and checks 60 frames with `stm32_ltdc_wait_frame()`
(the LTDC's line interrupt, no transfer or underrun error). One line:

- `mp2_display: PASS: <w>x<h> <fmt> <n> frames`
- `mp2_display: FAIL: <step> <err>` (steps: `ltdc-ready`, the chain's
  `board`, `fb`, `adv-init`, `mode`, `clock`, `ltdc-probe`,
  `ltdc-configure`, `ltdc-enable`, `dsi-mode`, `dsi`, `adv-start`, `frame`,
  `i2c`, and the probe's `format`, `size`, `pattern`, `write`, `chain`)

It also compares its bar line with the crate's `yd_mp25_fill_color_bars`
(`mp2_display: bars: ...`, a note only). The LTDC's frame CRC is not in the
C ABI, so PASS rests on the frame flags, the picture on the KVM.

It then keeps scanning out for `CONFIG_MP2_DISPLAY_HOLD_S` seconds (default
60), one frame checked a second (`mp2_display: hold: <n> frames ok, <m>
failed` at the end), before `main()` returns, so the runner's `MP2_HOLD_S`
(tools/board-stm32mp2.sh) and the KVM can see the picture. `prj.conf` raises
the board layer's `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` to 150 so the layer
does not reset the board during the hold; a `BUILD_ASSERT` keeps the two
consistent.

## The overlay

`boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay` adds, under `/soc`, the
nodes of ST's tree as yakogl devices (bindings in driver/dts/bindings):
`i2c2` (0x40130000, kernel clock FLEXGEN_12 100 MHz) with the `adv7535`
child (reg 0x3d, edid 0x3f, cec 0x3c, packet 0x38, four lanes), `dsi`
(0x48000000, `bridge = <&adv7535>`, PHY reference FLEXGEN_28 27 MHz) and
`ltdc` (0x48010000, at most 1920x1080 `argb8888`, scanned as XRGB8888,
`dsi-host = <&dsi>`), and the frame buffer: `/chosen yakogl,ltdc-fb` names a
32 MiB `no-map` reserved area at 0xb0000000 (RISAF4 region 9, past the
image), of which `fb-size` 8 MiB is mapped normal non-cacheable. The board
and SoC files are untouched.

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
