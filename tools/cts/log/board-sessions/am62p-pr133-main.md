# AM62P session for PR #133, case 206 and the gl_test set on 066a2db8 (main_uart0 console)

Date: 2026-10-12. Board: Verdin AM62P (PowerVR), reached by ssh; console read
on the main_uart0 bridge port (the leased default port is mcu_uart0 / ttyS3,
see investigations/am62p-console-map.md).

Outcome: **case 206: not run. gl_test set: not run.** The probe passed; the
images could not be obtained, so board.sh was never started.

## Commits

- 066a2db8 = PR #133 head af9ecc53 merged with main 2fe4b879 (the images'
  source; this session did not rebuild them).
- am62p-pr133-baseline.md has no main-side run to compare with.

## Probe (no reboot)

`tools/am62p_netboot_uart.py capture <bridge>:<main_uart0 port> RAW STAMPED`
was started, then `echo YK-PROBE-S2 > /dev/ttyS2` ran once over ssh (rc 0).

- First attempt: capture exit 124 (connected, ran its 30 s bound).
- Second attempt, started while the first was still alive: exit 4,
  `capture: the bridge <host:port> is busy (another client holds it)`. The
  holder was this session's own first capture (it was still inside its
  timeout), not a foreign client.
- Third attempt, after the first had ended: connected, exit 124 (bound
  reached), RAW = 13 bytes, `grep -a -c YK-PROBE-S2` = 1. YK-PROBE-S2 arrived,
  so the port is main_uart0.

## Images

Wanted: pr133-images-066a2db8/img-am62p-206 (sha256
1d0cc4edb7f4c850e7be32b2697a7256ecc1d8b4947b0677e9104ba105e1c202) and
img-am62p (sha256
df7109ac6dbb42023d3312811b274180e554376c9b1d6d6bb559fc7f52ae54ac), each
GT/zephyr/zephyr.bin.

- The machine's dependency cache holds only img-pi4 and img-pi4-206 under
  pr133-images-066a2db8; the other directory there (PR133_IMAGES_066A2DB8) is
  an rpi_4b build (zephyr.bin sha256
  1569b52c7b12673b4055e96311f66688d9e330636a55582a127cf06f3fafe015), not an
  AM62P image.
- mcp__ccfarm__fetch_from_farm for the directory, img-am62p-206 and img-am62p:
  each call returned "The operation timed out" (4 calls in all).
- Not rebuilt: the brief says to rebuild only when the images cannot be
  obtained, but the build recipe (pr133-board-images.md on the prebuild
  branch) could not be read in this session (git show on the branch was
  refused twice), and a rebuild would not be checked against the sha256s.

## board.sh

Not run (no image). No board.sh verdict lines, no `gl_test:` line for case
206, no `gl_test: all:` tally. Recovery count: 0 (nothing was booted; the
board was not rebooted).

## Next

Retry with the two AM62P images in the dependency cache; the probe above
holds for the main_uart0 port.
