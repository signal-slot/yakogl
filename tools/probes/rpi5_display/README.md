<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_display

U3 on the Raspberry Pi 5 (`driver/docs/rpi5-display.md`): does a scanout run
when a `kernel=` image starts, and does the property mailbox hand out a
framebuffer. Not run on a board yet. It lives here, not under `rpi_5/`, until
#321 releases that tree.

```sh
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_display yakogl/tools/probes/rpi5_display
```

| File | What |
|---|---|
| `src/main.c` | the image: D1 (framebuffer GET tags), D2/D4 (HVS, pixel valve 0 and 1 words before and after), D3 (`rpi_fb_alloc`), D5 (pattern read back, then the split screen: console on top, pattern below) |
| `linux_ref.sh`, `linux_ref.md` | the read-only Linux dump to run first, over ssh (C0) |
| `linux_ref_selftest.sh` | host self-test of `linux_ref.sh` against a fake root |
| `screen_mock.sh` | host test of `driver/zephyr/lib/board/rpi5/rpi5_screen.c` (`test/rpi5_screen_mock.c`) |

The image ends with `rpi5_display: PASS (...)` or `rpi5_display: FAIL (reason)`;
every register read group is announced first, so a hang names its block. The
PM watchdog (`CONFIG_YAKOGL_RPI5_WDOG`, armed before `main`) returns the board
to Linux. The Pi 5 rules of `tools/probes/rpi_5/README.md` apply: run through
`tools/board.sh pi5`, record `get_throttled`, and if the board does not come
back stop and report that it needs a person. Records go to
`records/rpi5/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored, redacted).
