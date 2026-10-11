<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
> Record landed from `ccfarm/integration` (merge `39bb67dc5`, ccfarm `01a12aff-138a-71f2-8410-f8160906a73f`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# Pi 4 console bridge probe (2026-10-11)

Integration commit: 76d349c462b8

## Result: the bridge accepts a connection but did not carry the marker

1. Inputs. BOARD_UART was not in the session environment or the lease; the
   bridge host:port came from the farm's notes (`ask_farm`, a USB serial
   node on the tailnet, one client at a time). The board's ssh login came
   from the lease.
2. Connect (python3 -I, socket, 5 s connect timeout): **accepted**. It was
   not refused, did not time out, was not closed by the peer and printed no
   'busy' text. Reading for about 40 s: **0 bytes** while the board sat idle
   in Linux.
3. Marker over ssh. The board's kernel command line has
   `console=ttyAMA0,115200` and `/dev/serial0` points to ttyAMA0, so ttyAMA0
   is the console. With the socket open, 12 s in, `echo PROBE_MARKER_42 >
   /dev/ttyAMA0` ran over ssh and succeeded (exit 0, so the login can write
   the device). Result: **2 bytes** arrived through the bridge afterwards and
   the marker text was **not** among them (no printable content in the two
   bytes, likely line-ending or noise).
4. The socket was closed cleanly. The board was not rebooted, left in Linux,
   idle (load near 0).

## Interpretation

The bridge is up and free, but a write to the board's console UART did not
arrive as text. Possible causes (not distinguished here): the bridge's
adapter is not wired to the Pi 4's ttyAMA0 header pins, a baud-rate
mismatch (the 2 stray bytes would fit that), or the Pi's console UART is
disabled/muxed to another function while Linux runs. A non-printing
2 bytes is not evidence of a working console.

## What pi4-es2-tip-r2 should expect

- A TCP connection to the bridge works; refusal/busy handling should only
  trigger if another client holds it.
- Linux userspace output is not a proof of a working console: the marker
  test failed. The dEQP firmware's own UART output is a different path
  (firmware drives the UART itself), so the board run may still see output,
  but if the run's console.txt is empty or garbled, record that as a
  blocker (wiring/baud on the bridge side) and do not guess results.
- Before the long run, a quick check that the dEQP image prints its banner
  over the bridge is worth doing first.
