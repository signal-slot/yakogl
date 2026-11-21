<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout16

The build skeleton of the Raspberry Pi 5 16 bpp scanout probe (plan #385,
item s16skel). It writes no register. It prints the build line, then a
read-only dump (`src/dump.h`) and ends:

- the HVS (mapped with `device_map()`, 0x1a000 bytes) VERSION word and its
  stepping (C0/D0); CTRL0, STATUS (FRCNT, MODE) and DL (LACT) of channels 0 to 2;
- PV_CONTROL.EN and PV_V_CONTROL.VIDEN of pixel valves 0 and 1.

Every step prints a line even when a block did not map (a read of an unmapped
block returns `0xffffffff`). Then `rpi5_scanout16: dump rc=N` (N is the number
of blocks that did not map) and `rpi5_scanout16: skeleton done`; the image then
stops feeding the watchdog (`rpi_wdog_alive()` in `src/main.c`).

`src/hold_check.h`, `verdicts.sh` and their selftests belong to later steps of
the probe.

```sh
west build -p always -b rpi_5 -d build/rpi5_scanout16 yakogl/tools/probes/rpi_5/rpi5_scanout16 -- -DYAKOGL_WERROR=ON
```

The `zephyr_rpi5_probe_images` row in `tools/check.sh` builds it
automatically; no new row is needed.

## On the board

Not run yet.
