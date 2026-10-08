<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-r1: AM62P dEQP-GLES2 images 8-17, ms4, gl_test (BLOCKED, nothing run on the board)

Integration image, not a landing proof. Nothing here is a device result.

Date: 2026-10-09. Integration tip 47ad0712.

## Blocker

The device console could not be read. The serial bridge named by the lease
(`usb-serial-1:35932`) reports "connected" but delivered 0 bytes for the whole
session. No image was built and no dEQP case was run; the tally and verdict
lines of images 8-17, ms4 and gl_test do not exist. Reason for every one:
`console silent`.

## Probable cause: the leased bridge port, not the wiring

`am62p-gles3-percase.md` ("Why the console was silent") already found that
port 35932 is wired to mcu_uart0 (Linux ttyS3) and can never carry U-Boot's,
the getty's or Zephyr's output, which are on main_uart0 (ttyS2).
`tools/cts/RESULTS.md` (section 1a, around line 1007) runs the AM62P console
on `usb-serial-1:35933` and calls 35932 dead. This session's lease again names
35932 only, so the same mismatch is the likely reason. Port 35933 is not
leased, so it was not used. The farm resource (lease or bridge port mapping)
needs fixing; the wiring is not necessarily at fault.

## What was tried (before any build)

All reads on the bridge were at 115200 8N1 (the documented rate, see
`docs/direct-boot.md`); the bridge is a plain byte forwarder and has no baud
control from this session.

1. `serial_status`: connected, offset 0.
2. Wrote a CR/LF to the console, waited 5 s: nothing.
3. Over ssh, wrote a marker to ttyS2 (the getty and U-Boot/Linux console)
   and to `/dev/kmsg`: nothing on the bridge. Expected, if 35932 is mcu_uart0.
4. Rebooted the board over ssh and waited 120 s for U-Boot, kernel or login
   output: nothing. The board came back (ssh up), so it is healthy.
5. Over ssh, `stty -F /dev/ttyS3 115200`, then wrote a marker to ttyS3 and
   watched the bridge for 5 s: nothing. This differs from the earlier record,
   where a ttyS3 probe arrived verbatim, so the bridge's mcu_uart0 path was
   also silent this time; wiring or the bridge may have changed since.
6. The bridge cannot send a break and has no reconnect or power-cycle control
   from this session.

## Needed from a person

Fix the farm resource so the AM62P console lease names the main_uart0 bridge
port (35933 per RESULTS.md 1a), or confirm the bridge mapping and wiring, then
re-run this item. If 35933 is leased and still silent, check the UART wiring.
