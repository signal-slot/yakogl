<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout16

Frame 1 of the Raspberry Pi 5 display (docs/notes/rpi5-display.md section 3;
it answers Q2 of section 4: does the firmware redraw into its plane, or change
its display list, while we hold the plane). The image adopts the RGB565 plane
the firmware left running on HDMI0, draws 16 bpp fbcon text on its top rows and
a pattern below, reads the plane back, holds it without writing, reads it back
again, then stops feeding the watchdog.

It writes only the plane's bytes. It never writes a pixel valve, HDMI, PHY or
HVS register, nor a word of any display list.

## What it does

1. The read-only register dump of `src/dump.h`: HVS VERSION and stepping,
   CTRL0/STATUS/DL of channels 0 to 2, PV_CONTROL.EN and PV_V_CONTROL.VIDEN of
   both pixel valves. It maps the HVS and the two valves; the rest of the image
   reads through those mappings. iommu4 is read too (plane_match.h of
   `tools/probes/rpi5_display`): an address the IOMMU may translate is never
   written.
2. Z1 as `rpi5_display` does it: channels 0 to 2 sampled twice 100 ms apart,
   each judged by `scanout_alive.h` once per mapped valve. Line [1]
   `u3 scanout:` names the first running channel. Its list (DL.LACT, or
   LPTRS.HEADE when LACT is 0) is read, walked to its END and printed word by
   word, and the first plane decoded: line [2] `u3 plane:`.
3. `adopt.h` decides on the plane. The probe draws RGB565 only. The buffer is
   mapped Normal non-cacheable. The plane is hashed twice 100 ms apart before
   the write (is the firmware drawing before we do: hold_check.h's redraw A),
   its visible bytes are cleared and read back: line [3] `u3 adopt-readback:`.
4. `src/frame16.h`: up to 16 text rows (line [4]), the pattern band (line
   [5]), the same frame rendered into a RAM image of the expected bytes, the
   written rows cleaned to memory (`fb_flush.h`), and readback 1 byte-equal
   against the expected image (line [6]).
5. The hold (`src/hold_check.h`): a reference sample right after the write,
   then one sample every 10 s (STATUS, LPTRS, the list's words hashed, 64
   plane rows hashed, the valve's words). A sample waits up to 100 ms for
   FRCNT to move, because two samples seconds apart can hold the same 6-bit
   count. `hold_clamp_end` ends the hold 10 s before the watchdog stops
   being fed. `hold_judge` decides.
6. Readback 2 (line [7]), STATUS.FRCNT twice 50 ms apart (line [8]), the
   valve's PV_CONTROL and PV_V_CONTROL (line [9]), the hold line (line [10],
   bytes = readback 2's differing bytes, words = list words that differ from
   the adopted snapshot) and the result (line [11]). Then
   `rpi5_scanout16: feeding stopped, reset expected`, and the PM watchdog
   resets the board within one period.

Every one of the eleven lines is printed even when an earlier step failed: a
step that could not run prints FAIL, stopped or not drawn with its reason, so
the log stays complete and `verdicts.sh` judges it FAIL, not incomplete.

## Build

From `$ZEPHYR_WS` (where `yakogl` is this repository):

```sh
west build -p always -b rpi_5 -d build/rpi5_scanout16 yakogl/tools/probes/rpi_5/rpi5_scanout16 -- -DYAKOGL_WERROR=ON
```

The `zephyr_rpi5_probe_images` row in `tools/check.sh` builds it
automatically.

The hold length is a CMake knob: `-DRPI5_SCANOUT16_HOLD_MS=<ms>`, 240000 by
default and at most 360000. The watchdog is fed for
`CONFIG_YAKOGL_RPI5_WDOG_HOLD_S` = 280 s (`prj.conf`). The image clamps the
hold to that time minus 10 s, counted from boot, and says so on its hold
record line. A longer hold needs both raised. The image's end line has to come
before `tools/board.sh` pi5's RUN_BOUND of 420 s, counted from the reboot. With
the defaults the image ends about 245 s after boot. No `tools/check.sh` list
names probe knobs, so this one is described here only.

## Run

```sh
tools/board.sh pi5 run build/rpi5_scanout16/zephyr/zephyr.bin <logdir> rpi5_scanout16
sed -E 's/^[0-9]{2}:[0-9]{2}:[0-9]{2} //' <logdir>/<run>/console.txt > console.log
bash tools/probes/rpi_5/rpi5_scanout16/verdicts.sh console.log
```

`board.sh` writes the run into `<logdir>/<YYYYMMDD-HHMM>-rpi5_scanout16-<sha8>/`.
Its `console.txt` begins each line with `HH:MM:SS `. `verdicts.sh` anchors
each line at its start, so the prefix is stripped first. The run ends at
`rpi5_scanout16: PASS|FAIL` (board.sh pi5's end pattern). The next line,
`feeding stopped, reset expected`, comes before the watchdog's reset.

`verdicts.sh` exits with 0 for PASS, 1 for incomplete, 2 for a usage error, and
3 for a complete log that fails (the board ran, a condition failed). Its `q2:`
line answers Q2 and does not change the exit status. The host tests are
`verdicts_selftest.sh`, `frame16_selftest.sh` and `hold_check_selftest.sh`.

## On the board

Not run yet.
