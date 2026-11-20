<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES2 regression rows (D FB1 T1 T3 TF) at fad0558a

commit: fad0558a7def0e5677761608eec56b12e9855d18

The ES 2.0 regression rows of `docs/es3/board-recipe-pi4.md` section 8, run on
the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot) at the commit of the Pi 4
dEQP-GLES3 rerun (`../2026-11-pi4-gles3-fad0558a-rerun/README.md`), plan #380,
item pi4-es2 (issue #1261). Stage 4's criterion is that the stage-1 ES 2.0
result stays the same; this is that check, against
`../2026-11-pi4-es2-b1b5b845/README.md`. Rows 1, 4, 19, 21 and 24 (counted from
0) of `tools/cts/pi4-full-tree.txt` (unchanged since `b1b5b845`; section 8 of
the recipe lists the same five rows), the gles2 stage 1 as `DEQP_WORK`, no
`DEQP_MODULE`, `BOARD_MUSTPASS` unset (the GLES2 list). Per-image files,
relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts
are the mustpass view (`gles2-mustpass.txt`) of the tally's TOTAL line.

| row | image | mustpass | Pass | Fail | NotSupported | other | begun | run | GPU recoveries | hangs | verdict | `b1b5b845` (mustpass / Pass / begun) | vs `b1b5b845` |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | D | 292 | 292 | 0 | 0 | 0 | 305 | 12.1 s | 0 | none | DONE, all passed | 292 / 292 / 305 | equal |
| 4 | FB1 | 390 | 390 | 0 | 0 | 0 | 392 | 11.7 s | 0 | none | DONE, all passed | 390 / 390 / 392 | equal |
| 19 | T1 | 181 | 181 | 0 | 0 | 0 | 224 | 27.1 s | 0 | none | DONE, all passed | 181 / 181 / 224 | equal |
| 21 | T3 | 304 | 304 | 0 | 0 | 0 | 304 | 43.2 s | 0 | none | DONE, all passed | 304 / 304 / 304 | equal |
| 24 | TF | 17 | 17 | 0 | 0 | 0 | 20 | 2.0 s | 0 | none | DONE, all passed | 17 / 17 / 20 | equal |
| | total | 1,184 | 1,184 | 0 | 0 | 0 | 1,245 | | 0 | | | 1,184 / 1,184 / 1,245 | equal |

("begun" is the image's own counter: its filter also matches cases outside
`gles2-mustpass.txt`; every one of the 1,245 begun cases passed, the images' own
`done:` lines read `0 failed, 0 not supported, 0 warnings, complete=1`.)

## Result

**No row regressed: all five rows are equal to `b1b5b845`.** Every image ran to
the end (`complete=1`, `N cases begun, N with a result, 0 without`) and every
mustpass case in their filters passed: 1,184 of 1,184, 0 Fail, 0 NotSupported,
0 without a result, nothing moved from Pass. The longest run is T3's 43.2 s,
far inside the 210 s record deadline. `pi4-console-scan.py` over the five
consoles: `5 image run(s): 5 finished, 0 without a finish line; recoveries 0,
job timeouts 0, hung 0, MMU faults 0, resets 0, faults 0`; every image printed
`slow jobs: 0 ...; MMU faults: 0; recoveries: 0; hung: none`. Every console
prints `*** Booting Zephyr OS build af17c0c8aa2a ***`, `I: shader compiler
scratch: 8388608 bytes` (the 8 MiB compiler scratch, the one difference of the
images to `b1b5b845`'s besides the code since) and `v3d_deqp: build fad0558a`.

## The cycles

Run 2026-11-21 about 01:28-02:00 JST (2026-11-20 about 16:28-17:00 UTC), one
image at a time, each staged only after Linux was back and the previous one's
files were gone. Per image:

| image | zephyr.bin sha256 (read back on the board) | Linux back (boot start per the board's clock, JST) | clean-up | `cases in` line |
|---|---|---|---|---|
| D | `bd9563b1d315fdf1f1dde9ca05e1a414e422de092081bd59f824f3950907bd18` | 01:33:48 | staged image removed, tryboot.txt put back (read back) | 305 cases in 12118 ms, record at 12 s after boot |
| FB1 | `ef592148165cc1f2914bc5db2cd72d9114050af2035c61e767741d0c44c6d70b` | 01:39:09 | the same | 392 cases in 11715 ms, record at 11 s after boot |
| T1 | `cdc30ecd7dbea7808d786bc6da80570c4bc727576acd1983915daf500b0d6bf5` | 01:44:25 | the same | 224 cases in 27136 ms, record at 27 s after boot |
| T3 | `264169a8e116bd929f6d7561cbe21f7d2d11671854088c5378f51e3230bed584` | 01:49:45 | the same | 304 cases in 43175 ms, record at 43 s after boot |
| TF | `c3760c5b1a44b60352513ede253de37708ea222a30fc25b7de30fa05cedaea5a` | 01:55:15 | the same | 20 cases in 2015 ms, record at 2 s after boot |

Every image printed `board: watchdog armed` before Zephyr started; the PM
watchdog reset the board at about 250 s and Linux came back each time with a
new boot_id on its normal configuration (the first ssh after a cycle was
refused or timed out once or twice before it answered; nothing was staged
then). The time stamps are the board's own clock as read right after Linux
came up (before it was synchronised, so only the cadence of about 5 min per
cycle is meant by them); the exact reboot-to-Linux seconds were not recorded.

**The board after the last cycle:** Linux up (boot_id changed after each
cycle), `/boot/firmware` held no `yakogl-*.bin`, no `config-linux.txt` and no
staged leftover in `/tmp`; `tryboot.txt` read back as a single newline, as
before the session (`od -c`: `\n`); `config.txt` was never written (its sha256
read `6ea8eaef9ebacfe30f679b778bbffe3a041803bf1c8f1ce8c1952a75a362de46`).
No partition other than the files under `/boot/firmware` was written, no
module loaded, Tailscale and sshd untouched, no U-Boot variable (the Pi 4 has
none; the boot is `reboot "0 tryboot"`).

## Method

Built from a detached worktree of exactly `fad0558a` (`git describe`:
`fad0558a`): the gles2 stage 1 through `tools/cts/deps-env.sh --build --board
rpi_4b DEQP_WORK` (VK-GL-CTS 93bca01, Zephyr SDK 1.0.1, 418 ninja steps, rc 0;
see "Caveats" for the checkout it was started from), then the recipe's first
command, `DEQP_WORK=<gles2 stage 1> tools/cts/pi4-deqp-images.sh
tools/cts/pi4-full-tree.txt <dir> D FB1 T1 T3 TF` run from that worktree: five
`<LABEL> rc=0 sha256=... build=fad0558a` lines (the sha256 above; each equals
the file sent to the board and the file read back from `/boot/firmware`).
The stage 'board-common-pi4' is `tools/cts/board-stage.sh pi4 common`'s
preparation as the recipe describes it: Zephyr SDK 1.0.1 and the west
workspace from the dependency cache, `RUSTUP_TOOLCHAIN=nightly`,
`CMAKE_POLICY_VERSION_MINIMUM=3.5`.

`tools/board.sh pi4 tree` did not run: the Pi 4's UART bridge takes one client
at a time and the leased serial console (the MCP server) held it, so, as in the
`b1b5b845` ES 2.0 session and the GLES3 rerun's sessions 1 and 2, the steps of
`tools/board.sh pi4` were done by hand, in its order, one image at a time:
`sudo`, free-space (293 MiB free) and leftover prechecks (no `yakogl-*.bin`, no
`config-linux.txt`, `tryboot.txt` the one-newline stock); the undo printed
before each staging (`sudo rm -f /boot/firmware/yakogl-<LABEL>.bin`, and
`tryboot.txt` put back to one newline); the image installed to
`/boot/firmware/yakogl-<LABEL>.bin`; `tryboot.txt` written with `pi4_stage`'s
lines (its comment line, `[all]`, `kernel=`, `arm_64bit=1`, `enable_uart=1`,
`uart_2ndstage=1`; no `hdmi_timings`: the panel's value is not in the
environment); both read back (sha256, text) before the reboot; `sudo reboot "0
tryboot"`; the end line awaited on the leased serial console (`serial_expect`);
the console read from it (`serial_read`, saved to disk by the tool, every
chunk contiguous: T3's took two) and cut by a script from the first line after
`reboot: Restarting system` to the `v3d_deqp: build fad0558a DONE...` end line,
CRs and NULs dropped, never copied by hand; Linux back (a new boot_id), the
staged image removed and `tryboot.txt` put back, read back. Since
`board.sh` did not run there are no `board.sh pi4 <LABEL> rc=... verdict=...`
lines; the verdicts are the images' own end lines (`v3d_deqp: build fad0558a
DONE, all passed: N cases, P N F 0 R 0 N 0 other 0`) and the scan's. The tally
is the stand-alone `tools/cts/deqp-uart-tally.py`, not `board.sh pi4 tally`.
The raw serial reads did not survive the turn; the committed consoles are the
only record. The GLES3 rerun's session 4 at the same commit is
`../2026-11-pi4-gles3-fad0558a-rerun-s4/README.md`.

## Caveats

- The gles2 stage 1 build was started from the integration checkout
  (`d48ba327`) before the worktree existed in the shell's directory, so the
  `cts-build-zephyr.sh` step used that checkout's `driver/samples/zephyr/deqp`
  for the CTS library flags and `capi`; the five images themselves (the
  `.built` lines, `build=fad0558a`, and the images' own `v3d_deqp: build
  fad0558a`) were built from the `fad0558a` worktree. An image build from
  `d48ba327` stamped `build=d48ba327` was stopped and its output discarded;
  none of it was run on the board. If the stage 1 libraries differ between
  the two commits, the rows above may not be a pure `fad0558a` build; the
  planner may want a repeat of the stage 1 from the worktree.
- Console timestamps are absent (the leased console's reads carry none).

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the
tally reads past them.

- `<LABEL>.console.txt`: the image's console from the first line after Linux's
  `reboot: Restarting system with command '0 tryboot'` to the `v3d_deqp: build
  fad0558a ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `deqp-uart-tally.py --label <LABEL> --mustpass
  ../../gles2-mustpass.txt <LABEL>.console.txt`, run from inside this
  directory on relative names.
