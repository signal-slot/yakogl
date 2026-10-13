# AM62P session for PR #133, case 206 and the gl_test set on 066a2db8 (main_uart0 console)

Date: 2026-10-12. Board: Verdin AM62P (PowerVR), reached by ssh; console read
on the main_uart0 bridge port (the leased default port is mcu_uart0 / ttyS3,
see investigations/am62p-console-map.md).

Outcome: **case 206: not run. gl_test set: not run.** The probe passed and
both images were in hand with the expected sha256s; `tools/board.sh` itself
was refused by the tool permission layer on every attempt, so no board
session started and nothing was booted.

## Commits

- 066a2db8 (`066a2db81d40e519efc05dcbdcdd4e359468fe9a`) = PR #133 head
  af9ecc53 merged with main 2fe4b879. The images' source; this session did not
  rebuild them.
- am62p-pr133-baseline.md has no main-side run to compare with.

## Probe (no reboot)

am62p-main (earlier session): `tools/am62p_netboot_uart.py capture` on the
main_uart0 bridge port was running while `echo YK-PROBE-S2 > /dev/ttyS2` ran
once over ssh (rc 0). The capture's RAW held 13 bytes with YK-PROBE-S2 once,
so the port is main_uart0. (Its first attempt, capture exit 124, ran its time
bound; a second capture started while the first was alive got exit 4, "busy",
held by that session's own first capture.)

am62p-run2 (this session): the same capture (bound 25 s, exit 124 = connected
and ran to the bound, not busy), and `echo YK-PROBE-S2 > /dev/ttyS2` once over
ssh (rc 0). RAW = 13 bytes, `grep -a -c YK-PROBE-S2` = 1. The probe passed
again: main_uart0 is the port to give board.sh as BOARD_UART.

## Images

Obtained **without a fetch and without a rebuild**: this machine's dependency
cache holds the prebuilt directories under pr133-images-066a2db8 (img-am62p,
img-am62p-206, img-imx8mp, img-imx8mp-206, img-pi4, img-pi4-206). Each
GT/zephyr/zephyr.bin, `sha256sum` here:

| Image | sha256 | Expected (pr133-board-images.md) |
|---|---|---|
| img-am62p-206 (case 206 only) | `1d0cc4edb7f4c850e7be32b2697a7256ecc1d8b4947b0677e9104ba105e1c202` | same |
| img-am62p (whole set) | `df7109ac6dbb42023d3312811b274180e554376c9b1d6d6bb559fc7f52ae54ac` | same |

(am62p-main recorded that the cache of its own machine held only the Pi 4
images and its fetches timed out; this session ran on a machine that has all
six.)

## board.sh

Not run. Reason: the exact command

```
BOARD_HOST=<lease login> BOARD_UART=<bridge host>:<main_uart0 port> BOARD_BOOT=uboot-mmc \
  tools/board.sh am62p run <cache>/pr133-images-066a2db8/img-am62p-206/GT/zephyr/zephyr.bin \
  records/am62p/pr133-066a2db8 c206-066a2db8
```

(record directory `records/am62p/pr133-066a2db8` created first as its own
command) was refused with "This command requires approval" on the first
attempt (a `| tee` form: "multiple operations ... require approval") and
again on **3 retries of the same plain command**. Per the brief it is recorded
as refused after 3 retries. The whole-set image (set-066a2db8) was not
attempted: it runs only after case 206 and the same command shape would be
refused the same way. The empty record directory was removed.

- board.sh verdict lines: none (never started).
- `gl_test:` verdict line of case 206: not run (reason above).
- `gl_test: all:` tally of the set: not run (reason above).
- Recovery count: 0 (nothing was booted; the board was not rebooted; no
  other client held the console).

## Next

The images and the probe are settled. What remains is only that the two
board.sh commands above run in a session where the permission layer grants
them (`tools/board.sh am62p run ...`, case image first, then img-am62p as
set-066a2db8).
