<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P console lease: owner brief

For the board owner. One fault, two fixes, one check.

## The fault

The farm's AM62P console lease (device `usb-serial-1`, reached as
`$BOARD_UART`) carries the Verdin AM62P's **mcu_uart0** (Linux **ttyS3**).

U-Boot, the Linux getty and Zephyr print on **main_uart0** (Linux **ttyS2**).

So a leased session sees no boot output, and every AM62P session that needs
the console is blocked.

## Evidence

Three of the session records were later folded or removed (no case ran in
them). Git history keeps their full text.

- `am62p-gles3-r2` (git history; one row in
  `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md`, "The
  attempts"): `echo probe | sudo tee /dev/ttyS2` over ssh did not reach the
  leased port.
- `am62p-gles3-percase` (git history; quoted in
  `tools/cts/log/investigations/am62p-console-empty.md`, section 8): probes
  on all four ttys; only the ttyS3 probe reached the leased port.
- `am62p-es2-r1` (git history; same investigation, section 1): zero bytes
  for the whole session.
- `tools/cts/log/investigations/am62p-console-empty.md`, Summary and
  hypothesis H1: the lease names the mcu_uart0 channel.
- `tools/cts/log/board-sessions/am62p-es2-r1c.md`, "Step 1": a session that
  used the main_uart0 channel captured fine. It also records that the farm
  resource registration was not changed. So the lease definition is still
  the fault; sessions only work around it when they can.

## Conflict with console-map.md

`tools/cts/log/investigations/console-map.md`, the AM62P row of the first
table, says main_uart0 on `usb-serial-1` and "Verified". The sessions above
show the leased default does not carry main_uart0. This brief does not edit
that file.

## Owner fixes (pick one)

a. Rewire the bridge channel the lease uses to the Verdin's **main_uart0**
   pins.
b. Point the `usb-serial-1` device definition in the farm config at the
   bridge port that carries main_uart0.

## One-line check

Over ssh to `$BOARD_HOST`:

```
echo probe | sudo tee /dev/ttyS2
```

`probe` must show up on `$BOARD_UART`. If it does, the lease is right.

## Blocked until then

- Plan #20: `am62p-es2-final-a` and `am62p-es2-final-b`.
- gl_test case 171.
- The AM62P ES3 (dEQP-GLES3) per-case runs.

No one touched the board, the bridge or the farm config for this brief.
