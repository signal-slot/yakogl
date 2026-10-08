<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-r1: AM62P dEQP-GLES2 images 8-17, ms4, gl_test (BLOCKED, nothing run on the board)

Integration image, not a landing proof. Nothing here is a device result.

Date: 2026-10-09. Integration tip 47ad0712.

## Blocker

The device console could not be read. The serial bridge for the board
reports "connected" but delivered 0 bytes for the whole session. No image was
built and no dEQP case was run; the tally and verdict lines of images 8-17,
ms4 and gl_test do not exist. Reason for every one: `console silent`.

## What was tried (step A, before any build)

1. `serial_status`: connected, offset 0.
2. Wrote a CR/LF to the console, waited 5 s for any output: nothing.
3. Over ssh, wrote a marker string to the Linux console device
   (`ttyS2`, the one `dmesg` reports as `console [ttyS2] enabled`) and a
   marker to `/dev/kmsg`; waited on the bridge: nothing.
4. Rebooted the board over ssh and waited 120 s on the bridge for U-Boot,
   kernel or login output: nothing. The board did come back (ssh up, uptime
   about 1 min), so the board is healthy and only the UART path is dead.
5. The bridge cannot send a break and has no reconnect or power-cycle
   control from this session. The baud rate is not documented in the repo
   (HANDOFF only says `BOARD_UART` comes from the environment), so no baud
   change was possible on the bridge side.

## Needed from a person

Check the UART wiring (Verdin debug UART to the adapter: TX/RX swapped or
loose, ground) and the serial bridge/adapter for the board, then re-run this
item. Both the previous attempt and this one saw the same symptom, which
points to the wiring or bridge, not to the software.

## Known and unrelated

`tools/check.sh --quick` fails on a pre-existing clippy error
(`overly_complex_bool_expr`, src/gl/texture.rs:9199, present since cc78859).
Not addressed here.
