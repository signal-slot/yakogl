<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_wdog

**Question.** Does WDOG3 reset the Verdin iMX95, so that every later probe
can rely on it? And what did U-Boot leave in WDOG3 (pre-study section 1.6:
whether U-Boot's watchdog still runs after `go`)?

**What it does.** The board glue arms WDOG3 at `PRE_KERNEL_1` as in every
image (`ip::wdog32` behind `yd_wdog32_arm`: 60 s, Linux's CS with INT), but
this image is built with `CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED=n`: nothing
refreshes it. `main()` prints the banner, a line every 10 s, and waits. The
System Manager's `FAULT_WDOG3` reaction (`sys_reset`) resets the whole board,
which boots as from power-on: U-Boot, then Linux. Nothing else is touched.

**Expected lines** (values illustrative):

```
imx95_wdog: build 0123abcd
imx95_wdog: wdog found CS 0x00002980 TOVAL 0x00000400 (stopped); armed rc 0 CS 0x000029e0 TOVAL 0x00001d4c timeout 60000 ms
imx95_wdog: not fed: reset expected within 60000 ms of the arming (uptime now 12 ms)
imx95_wdog: alive at 10 s
...
imx95_wdog: alive at 50 s
```

then, at about 60 s after the arming, the console stops and U-Boot's banner
follows (the pass). The glue's own lines before them say the same:
`verdin_imx95: WDOG3 armed: CS ... (was running|was stopped) ...` and
`verdin_imx95: WDOG3 is not fed ...`.

**Reading it.**
- `wdog found ... (running)`: U-Boot left WDOG3 enabled (CS.EN set at
  hand-over); `CS` and `TOVAL` as found are what it wrote. `(stopped)`: it
  did not. Either way the arming takes over.
- `armed rc` not 0 never shows here: the glue resets at once with
  `verdin_imx95: WDOG3 arming FAILED (rc, refusal ...)`; `refusal` is
  `YD_WDOG32_REFUSED_*` (yakogl_drivers.h).
- `imx95_wdog: FAIL no reset: still running 30 s after WDOG3's 60000 ms
  timeout`: WDOG3 counts but its timeout does not reset the board (the
  System Manager's configuration is not the one read, pre-study U1), or the
  counter does not run (its clock). The probe then takes the PSCI reset; if
  that returns too, a person must power-cycle the board. Do not run the
  other probes before this one passes.
- The console stops before 60 s: something else reset the board (the
  lifetime's PSCI reset is at 150 s, so not that).

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_wdog yakogl/tools/probes/verdin_imx95/imx95_wdog`.
The board session waits about 60 s plus the reboot.
