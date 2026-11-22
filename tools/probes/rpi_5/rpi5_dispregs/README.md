<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_dispregs

A read-only look at the display state the firmware left on the Pi 5. It
answers, as far as registers and the firmware mailbox can,
`docs/notes/rpi5-display.md` section 4 Q3, Q5 and Q6. Part of ROADMAP stage 5
(issue #7), step P11 of `docs/notes/pre-study-rpi5.md`.

The image writes no display, HVS, pixel valve, HDMI or PHY register. Its only
writes are the PM watchdog's (the board layer) and the mailbox's (the request
buffer and the doorbell); every request tag is a GET tag.

## What it reads

| Step | Reads | Answers |
|---|---|---|
| `hvs` | VERSION, then CTRL0 (ENB, FWIDTH 28:16, LINES 12:0) and STATUS (MODE, FRCNT) of channels 0 and 1 | Q3: the size the HVS composes |
| `pv0`, `pv1` | PV_CONTROL, PV_V_CONTROL, PV_HORZA/B, PV_VERTA/B raw and decoded (`pv_hdmi.h`, `pv_timing.h`: EN, VIDEN, CLK_SELECT, PIXEL_REP, format, active width and height, the VIC if it is one of the three modes of `pv_timing.h`) | Q3: which valve runs, and its active width |
| clocks | per clock 4 (HVS core), 13 (HDMI HSM), 14 (HDMI bvb), 16 (HVS disp): GET_CLOCK_STATE `0x00030001`, GET_CLOCK_RATE `0x00030002`, GET_CLOCK_RATE_MEASURED `0x00030047`, GET_MAX_CLOCK_RATE `0x00030004`, each with its response code, tag answer word and value | Q5 |
| display | GET_NUM_DISPLAYS `0x00040013`, GET_DISPLAY_SETTINGS `0x00040014` (64-byte answer, printed as raw words) | Q6 |
| `hdmi` | HDMI_HOTPLUG (hdmi block + 0x1c8) and HD_VID_CTL (hd + 0x44 for HDMI0, + 0x48 for HDMI1), only when the HSM clock (13) answered state on and a measured rate other than 0; else the line `hdmi skipped (hsm clock off)` | Q6 |

Citations: the clock tags and ids are `rpi5_scanout/fw_clk.h:29-35`
(from `raspberrypi-firmware.h`); the display tags are
`driver/docs/rpi5-display.md:203`; the valve and HDMI0 registers are
`rpi5_scanout/pv_hdmi.h`; HDMI1's block is `driver/docs/rpi5-display.md:68,302`.

The mailbox is `rpi5_mbox`'s "clear" way (`CONFIG_CACHE_MANAGEMENT=y`, a clean
before the doorbell and an invalidate after the reply), a copy of
`rpi_mbox.c`'s transact that keeps the response code word.

Not read, and why: PV_MUX and PV_STAT (no offset of either is in the repo; the
probe says so in a line), the HDMI PHY, and any display-list word.
`SET_DISPLAY_NUM` is a setter and is never sent, so GET_DISPLAY_SETTINGS
answers for the display the firmware has selected; asking for the second
display's settings is left for a run that may select one. A read of an HDMI
block with its clock off may hang the bus; hence the HSM gate. The watchdog
would reset the board, and the run would lose its end line.

## Build

```sh
west build -p always -b rpi_5 -d build/rpi5_dispregs yakogl/tools/probes/rpi_5/rpi5_dispregs -- -DYAKOGL_WERROR=ON
```

`tools/check.sh`'s zephyr row picks the directory up by its glob.

## Run

```sh
BOARD_END_RE='^rpi5_dispregs: (done|FAIL)' \
  tools/board.sh pi5 run build/rpi5_dispregs/zephyr/zephyr.bin <logdir> rpi5_dispregs
sed -E 's/^[0-9]{2}:[0-9]{2}:[0-9]{2} //' <logdir>/<run>/console.txt > console.log
bash tools/probes/rpi_5/rpi5_dispregs/verdicts.sh console.log
```

The image ends with `rpi5_dispregs: done (...)` or `rpi5_dispregs: FAIL
(reason)`, then `feeding stopped, reset expected`; the PM watchdog resets the
board within 10 s and tryboot returns it to Linux. It is armed in the board
layer's first init hook, before `main`, and fed for 60 s at most
(`CONFIG_YAKOGL_RPI5_WDOG_HOLD_S`).

`verdicts.sh` prints `q3:`, `q5:` and `q6:` answer lines (with a detail line
per valve and per clock) and exits 0 for done, 1 for incomplete (no end line),
2 for a usage error and 3 for the image's FAIL. Its header names the log
format; `verdicts_selftest.sh` covers each exit, the two-pixels-per-clock
answers and the skipped HDMI path.

## On the board

2026-11-22, one tryboot session on the Pi 5 (BCM2712 D0). The image was
built with `-DYAKOGL_WERROR=ON` from `a9ca11e1` (it printed
`rpi5_dispregs: build a9ca11e1`; Zephyr `af17c0c8aa2a`; image sha256
`e579e282`, 61728 bytes). The board came back to Linux by itself, through
the PM watchdog and tryboot, 168 s after the reboot. `get_throttled` read
0x0 before and after the run, and no image file or `tryboot.txt` was left
under `/boot/firmware`.

End line:

```
rpi5_dispregs: done (pv 2 of 2, clock tags 16 of 16, hdmi read)
```

`verdicts.sh` on the console with its `HH:MM:SS ` prefix stripped: exit 0.

```
rpi5_dispregs verdict: q3 pv0: running clk_select 0 pixel_rep 0 active 960x1080 vic 0; hvs ch0 fwidth 1919 lines 1079
rpi5_dispregs verdict: q3 pv1: running clk_select 0 pixel_rep 0 active 320x480 vic 0; hvs ch1 fwidth 639 lines 479
rpi5_dispregs verdict: q3: pv0 runs (both valves run) (the repo maps pv0 to HDMI0, pv1 to HDMI1); two pixels per clock: unknown (valve active width 960, HVS FWIDTH 1919)
rpi5_dispregs verdict: q5 clock core (4): state on rate 500000000 measured 910010752 max 910000000
rpi5_dispregs verdict: q5 clock hsm (13): state on rate 0 measured 648006592 max 648000000
rpi5_dispregs verdict: q5 clock bvb (14): state off rate 0 measured 324003296 max 324000000
rpi5_dispregs verdict: q5 clock disp (16): state off rate 0 measured 648009856 max 648000000
rpi5_dispregs verdict: q5: hsm on measured 648006592, bvb off measured 324003296, disp off measured 648009856
rpi5_dispregs verdict: q6: displays: 1, settings: answered, hdmi: hdmi0 hotplug 1 hd_vid_ctl enable 1, hdmi1 hotplug 0 hd_vid_ctl enable 1; the firmware drives HDMI0
rpi5_dispregs verdict: end: done (pv 2 of 2, clock tags 16 of 16, hdmi read)
rpi5_dispregs verdict: done
```

The raw words behind them, as the image printed them:

```
rpi5_dispregs: pv0 regs control 0x07174001 vcontrol 0x00000003 horza 0x004a0016 horzb 0x002c03c0 verta 0x00240005 vertb 0x00040438
rpi5_dispregs: pv1 regs control 0x07174001 vcontrol 0x00000003 horza 0x00180030 horzb 0x00080140 verta 0x00210002 vertb 0x000a01e0
rpi5_dispregs: display_settings words 0x00000000 0x00000780 0x00000438 0x00000010 0x00000f00 0x00000780 0x00000438 0x00000000 0x00000000 0x00000000 0x00000000 0x00000000 0x00000000 0x00000000 0x00000000 0x00000000
```

GET_DISPLAY_SETTINGS answered 1920x1080 at 16 bpp, pitch 3840: the
firmware's FB0 that the boot loader logged (`1920x1080 stride 3840 base
0x3f800000`). pv0's active width of 960 is half the HVS channel 0's 1920
(FWIDTH 1919). This is consistent with two pixels per clock, but
`verdicts.sh` leaves it as unknown. Both valves and both HVS channels run.
HDMI1 has no hotplug, yet its HD_VID_CTL reads enabled, with 0x98010000
against HDMI0's 0x80010000. The HSM, bvb and disp clocks answer a
GET_CLOCK_RATE of 0, while their measured rates sit at the max. Of the four
clocks only core (4) and hsm (13) answer state on.

The run was captured as in `rpi5_scanout16`: the farm's console lease holds
the serial bridge, so `BOARD_UART` was a local `am62p_netboot_uart.py feed`
relay of the lease's copy of the same console. The copy left out the boot
loader's lines before `Starting OS` and Zephyr's three `xlat tables` log
lines; every `rpi5_dispregs:` line is as the board printed it. `board.sh`
saw the end line (`boot_verdict=complete`) but named no result
(`verdict=no_record`, rc 4), because its pi5 label table does not know
`rpi5_dispregs` yet. The run directory is outside the tree (gitignored
`build/`).
