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
   Q2's list half is named while the hold runs: the adopted snapshot keeps
   up to 64 list words, and at every sample each word that differs from it is
   printed (at most 8, the rest counted):
   `rpi5_scanout16: q2 word <list word index> off <index - HEADE> entry-off
   <offset in the first plane entry, or none> was 0x.. now 0x..`. Right after
   the reference sample and again at the end of the hold a burst reads the
   list once per FRCNT change for 16 frames and prints, with FRCNT, every word
   that differs from the snapshot or between two frames
   (`q2 burst <reference|end> frame <k> FRCNT <n>: w<index>=0x..`), which
   shows whether a word follows the frame (the HVS writing status back) or
   changes rarely (a firmware edit). The closing burst runs inside the
   hold, before `hold_clamp_end`'s end, so the end line stays 10 s ahead of
   the watchdog. `rpi5_scanout16: q2 entry words <n> ctx-off <k>` gives the
   entry's length and the POS3 context word's offset (4 unity, else 5), and
   `u3 q2: word <n> entry-off <k> was 0x.. now 0x.. per-frame yes|no` (or
   `u3 q2: no list word changed`) closes the question. These lines are not
   part of the eleven and are optional to `verdicts.sh`. The probe writes no
   register and no list word.
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
line answers Q2 and does not change the exit status; when the log has the
`u3 q2:` line it also names the list word, its entry offset and values and
classifies it as `hvs-writeback`, `firmware-edit` or `unknown`. The host tests are
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

### Q2's list word (2026-11-22)

Ran on 2026-11-22 on the Pi 5 (tryboot, one run, image built with
`-DYAKOGL_WERROR=ON` from `ed71b85b`, `image_sha256` e4852862, 74016 bytes,
default hold 240000 ms; the PM watchdog reset the board and it came back to
Linux by itself; `get_throttled` 0x0 before and after; the staged image was
removed and the card left without a tryboot.txt, as before the run). The boot
loader again chose 1920x1080 at 60 Hz on HDMI0 with `FB0 ... base
0x3f800000`. As on 2026-11-21, `BOARD_UART` was a local feed
(`tools/am62p_netboot_uart.py feed`) that the agent filled from the lease's
console transcript; the 0xb0b0b0b0 list lines were condensed there, every
verdict and q2 line is as the board printed it. The run, with
`BOARD_END_RE='^rpi5_scanout16: (PASS|FAIL)'`:

```
board.sh pi5 rpi5_scanout16 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:rpi5_scanout16_pass resets=0 record=absent back_after=406s
```

The end line, and `verdicts.sh` on the captured console (prefix stripped),
exit 0:

```
rpi5_scanout16: PASS (plane adopted, 16 text rows and the pattern read back byte-equal twice, 238370 ms hold, FRCNT advanced, EN and VIDEN set)
```

```
  [10] u3 hold: 238370 ms plane=kept bytes=0 list=changed words=1
  missing: none
q2: yes, list changed (1 list words changed in 238370 ms); word 676 entry-off 4 was 0x8000005d now 0x800000d2 per-frame no: hvs-writeback
verdicts: PASS (plane adopted, both read-backs match, frcnt 35 -> 38 advanced, EN and VIDEN set)
```

Lines [1] to [7], [9] and [12] read as on 2026-11-21; [8] was `u3 frcnt:
before 35 after 38`, and [11] is the end line above. The plane entry at word 672 was `0x600cc004 0x00000000
0x0000fff0 0x0437077f 0x8000005d 0x00000000 0x3f800000 0x00000f00` when the
list was snapshotted. The q2 lines, verbatim (the `q2 sample:` lines between
them all read `1 list words differ from the snapshot`):

```
rpi5_scanout16: q2 burst reference frame 0 FRCNT 44: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 1 FRCNT 45: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 2 FRCNT 46: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 3 FRCNT 47: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 4 FRCNT 48: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 5 FRCNT 49: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 6 FRCNT 50: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 7 FRCNT 51: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 8 FRCNT 52: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 9 FRCNT 53: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 10 FRCNT 54: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 11 FRCNT 55: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 12 FRCNT 56: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 13 FRCNT 57: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 14 FRCNT 58: w676=0x80008000
rpi5_scanout16: q2 burst reference frame 15 FRCNT 59: w676=0x80008000
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000103
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x8000002a
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800003b2
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800002d9
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000244
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000192
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800000a1
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000007
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800003a1
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800002ac
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x8000020a
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000123
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000075
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800003f4
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000351
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000265
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800001bf
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800000d4
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x8000002e
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x8000039f
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800002f6
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x8000020c
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x80000163
rpi5_scanout16: q2 burst end frame 0 FRCNT 10: w676=0x80008000
rpi5_scanout16: q2 burst end frame 1 FRCNT 11: w676=0x80008000
rpi5_scanout16: q2 burst end frame 2 FRCNT 12: w676=0x80008000
rpi5_scanout16: q2 burst end frame 3 FRCNT 13: w676=0x80008000
rpi5_scanout16: q2 burst end frame 4 FRCNT 14: w676=0x80008000
rpi5_scanout16: q2 burst end frame 5 FRCNT 15: w676=0x80008000
rpi5_scanout16: q2 burst end frame 6 FRCNT 16: w676=0x80008000
rpi5_scanout16: q2 burst end frame 7 FRCNT 17: w676=0x80008000
rpi5_scanout16: q2 burst end frame 8 FRCNT 18: w676=0x80008000
rpi5_scanout16: q2 burst end frame 9 FRCNT 19: w676=0x80008000
rpi5_scanout16: q2 burst end frame 10 FRCNT 20: w676=0x80008000
rpi5_scanout16: q2 burst end frame 11 FRCNT 21: w676=0x80008000
rpi5_scanout16: q2 burst end frame 12 FRCNT 22: w676=0x80008000
rpi5_scanout16: q2 burst end frame 13 FRCNT 23: w676=0x80008000
rpi5_scanout16: q2 burst end frame 14 FRCNT 24: w676=0x80008000
rpi5_scanout16: q2 burst end frame 15 FRCNT 25: w676=0x80008000
rpi5_scanout16: q2 word 676 off 4 entry-off 4 was 0x8000005d now 0x800000d2
rpi5_scanout16: q2 entry words 32 ctx-off 4
u3 q2: word 676 entry-off 4 was 0x8000005d now 0x800000d2 per-frame no
```

The changing word is list word 676, offset 4 in the 32-word plane entry,
which is the entry's POS3 context word (`ctx-off 4`). It was 0x8000005d in
the snapshot. Read right after each of the 32 FRCNT steps it was 0x80008000.
At the 23 hold samples, taken every 10 s and not aligned to the frame, it was a
different 0x800000xx..0x800003xx value every time (low bits 0x007 to 0x3f4).
So the HVS writes it back while it scans the plane, and the firmware does not
edit the list: `verdicts.sh` classifies it as `hvs-writeback` because the
word sits at the context offset. The probe's `per-frame no` only says that
the value read just after each FRCNT step did not change from frame to frame.
No other list word changed, and LPTRS stayed 0x2a0.
