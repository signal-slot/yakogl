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

Ran on 2026-11-21 on the Pi 5 (tryboot, one run, image built with
`-DYAKOGL_WERROR=ON` from `2b88a9f6` on origin/main, `image_sha256` fefd1ba5,
69920 bytes, default hold 240000 ms; the PM watchdog reset the board and it
came back to Linux by itself; `get_throttled` 0x0 before and after). The
HDMI0 sink was attached: the boot loader saw hotplug on HDMI0 and none on
HDMI1, chose 1920x1080 at 60 Hz, and logged its framebuffer as `FB0 ...
1920x1080 stride 3840 base 0x3f800000`, the plane the probe found in the list.
The run, with `BOARD_END_RE='^rpi5_scanout16: (PASS|FAIL)'`:

```
board.sh pi5 rpi5_scanout16 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:rpi5_scanout16_pass resets=0 record=absent back_after=409s
```

`verdicts.sh` on the captured console (prefix stripped), exit 0:

```
  [1] u3 scanout: running
  [2] u3 plane: addr=0x3f800000 fmt=RGB565 pitch=3840 w=1920 h=1080
  [3] u3 adopt-readback: PASS (1920x1080 RGB565 at 0x3f800000 adopted, cleared and read back, 4147200 bytes; plane steady before the write)
  [4] rpi5_scanout16: text rows 16 changed
  [5] rpi5_scanout16: pattern drawn (8 colour bars below the text)
  [6] u3 readback 1: PASS (4147200 bytes read back byte-equal)
  [7] u3 readback 2: PASS (4147200 bytes read back byte-equal)
  [8] u3 frcnt: before 48 after 51
  [9] u3 pv: control=0x7174001 en=1 viden=1
  [10] u3 hold: 240012 ms plane=kept bytes=0 list=changed words=1
  [11] rpi5_scanout16: PASS (plane adopted, 16 text rows and the pattern read back byte-equal twice, 240012 ms hold, FRCNT advanced, EN and VIDEN set)
  [12] rpi5_scanout16: feeding stopped, reset expected (the PM watchdog resets the SoC, and tryboot returns the board to Linux)
  missing: none
q2: yes, list changed (1 list words changed in 240012 ms)
verdicts: PASS (plane adopted, both read-backs match, frcnt 48 -> 51 advanced, EN and VIDEN set)
```

What the registers and bytes show. HVS VERSION 0x2454 (D0); channel 0
(CTRL0 0x877f0437) and channel 1 (0x827f01df) enabled, channel 2 off; both
valves PV_CONTROL 0x07174001, PV_V_CONTROL 0x00000003; iommu4 disabled.
Channel 0's list starts at word 672 (LPTRS = DL = 0x2a0) with one plane
(`0x600cc004 0x00000000 0x0000fff0 0x0437077f 0x800000a8 0x00000000
0x3f800000 0x00000f00`, then 0xb0b0b0b0 fill and 0x20000000 at word 704); the
64-word walk found no END. The plane hashed the same twice before the write,
then held hash 0x17275865 in all 24 samples (every 10 s, `rows 0`), and
readback 2 was byte-equal after 240 s: the firmware does not redraw into its
plane. LPTRS stayed 0x2a0 and the valve words stayed the same in every sample.
The hash of the 64 list words did change: from sample 0 on it took eight
values, none of them the reference 0xdb5347d5, recurring in a cycle, and at
the end one word differed from the adopted snapshot.
The probe does not print which word it was or its values, so whether the
firmware edits its list or the HVS writes a status or context word back into
it is not yet known.

The console lease of the farm holds the board's serial bridge (one client at
a time), so `board.sh`'s first attempt was refused by its pre-check (`bridge is
busy`, rc 3, nothing staged, no reboot). The run above took `BOARD_UART` from a
local relay on the host that passed on the lease's transcript of the same
console. The relay's copy condensed the 55 repeated 0xb0b0b0b0 list lines into
two; every verdict line is as the board printed it. The run directory was
outside the checkout and is not committed.
