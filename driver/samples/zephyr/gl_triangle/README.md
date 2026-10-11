<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_triangle (Zephyr)

The smallest YakoGL application: one EGL window surface over a pixel buffer
the program owns, one Gouraud triangle, pixels read back through `glReadPixels`.
On the Verdin AM62P it also puts the window on the DSI panel. The one
triangle sample for all three boards is `../triangle`; this one stays for the
AM62P until Step 3 of `driver/docs/one-method.md` removes it.

```sh
# once: embed the board's DDK firmware (TI/Img property, never committed)
driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796   # -> driver/src/fw/pvr_fw_blob.h
# build: see the workspace in ../README.md
```

## Boards

- `sk_am62/am6254/a53` (upstream Zephyr): `boards/sk_am62_am6254_a53.overlay`
  adds the GPU node and a 64 MiB carveout. Compile-tested only.
- `verdin_am62p/am62p54/a53`: the board lives in this repository's driver
  module (`driver/boards/toradex/verdin_am62p`, with `driver/soc/ti/k3` and
  `driver/dts`; moved from the SafeUI application repository). Its board DTS carries the GPU node with
  the carveout at 0x97800000 and the display path (DSS1 -> DSI -> SN65DSI83
  -> LVDS panel) wired but disabled; `boards/verdin_am62p_am62p54_a53.overlay`
  enables the display, the I2C/GPIO banks, the TISCI power-on list and the
  RTI watchdog node, and `boards/verdin_am62p_am62p54_a53.conf` sets the
  init order (mailbox 40, TISCI 45, power-on 46, GPIO 47, I2C 50, DSI 70,
  DSS 80, GPU 90, panel bridge 95). Build it from a workspace made from
  yakogl's `west.yml` (`../README.md`):

  ```sh
  # from the top of a west workspace made from yakogl's west.yml (../README.md)
  west build -p always -b verdin_am62p/am62p54/a53 -d build/gl \
      yakogl/driver/samples/zephyr/gl_triangle
  ```

  Verified on the board on 2026-09-14: the triangle renders on the panel
  (camera) and the read-back column matches (black, gradient, black).

## What the sample does

1. Fills the window (a 1024x600 XRGB8888 buffer in the image, `__noinit`,
   cache-cleaned) with a CPU pattern so the display path can be judged even
   when the GPU draw fails.
2. Draws the triangle through EGL/GL ES 2.0 and reads a column of pixels back.
3. Shows the window as the DSS application plane (`CONFIG_PVR_SAMPLE_SHOW_ON_DSS`,
   the SafeUI `am62p_dss` driver) and keeps it for `CONFIG_PVR_SAMPLE_HOLD_SECONDS`.
4. Resets the SoC (`CONFIG_PVR_SAMPLE_REBOOT`) so the boot loader boots its
   default OS again; a fatal error resets too instead of halting.

Steps 3 and 4 and the watchdog are the board layer's
(`driver/zephyr/lib/board/`), shared with `gl_test`. Run the image with
`tools/board.sh am62p run <zephyr.bin> <logdir>` (U-Boot starts it;
`docs/standalone-boot.md`).

`CONFIG_GL_TRIANGLE_WINDOW_PHYS` can point the window at an existing scan-out
buffer instead (XRGB8888 rows); 0 uses the buffer in the image.

## Entering Zephyr from Linux (kexec, development only)

`driver/tools/kexec-zephyr/` boots `zephyr.bin` from Torizon without a serial
console or U-Boot access. It is a development route; a result counts only
when the boot loader started the image. Three things a previous Linux leaves
behind are handled:

- the GICv3 CPU interface is in split EOI/deactivate mode (`ICC_CTLR_EL1.EOImode`),
  which Zephyr's driver does not reset; the early hook in
`driver/soc/ti/k3/am62px/soc_a53.c` puts it
  back (without it every interrupt fires once and the kernel hangs in its first
  timed wait);
- the K3 RTI watchdog keeps running with a 60 s period and cannot be stopped;
  the SoC layer (`driver/soc/ti/k3/am62px/soc_a53_wdt.c`) services it in its
  open window
  (`CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT`, here for 150 s so a hung
  board still resets);
- peripherals may be powered down (Linux runtime PM): the `safeui,tisci-power-on`
  node switches I2C1 and the GPIO banks on before their drivers run.

The console is `main_uart0` (Verdin UART_3 on the X18 debug USB).
