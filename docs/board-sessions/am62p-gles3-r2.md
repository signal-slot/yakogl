<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES3 per-case run r2 — BLOCKER (console)

Date: 2026-10-14. Plan #19, item oq-gc. Integration tip dc51401.

**Result: no board results. Nothing was built or run.** The leased serial
bridge port does not carry main_uart0 (Linux `ttyS2`, where Zephyr prints).
Per the item's step 0, the build was skipped. No results are claimed.

## Console check (step 0)

Board shell over ssh (`$RESOURCE_VERDIN_AM62P_SSH`), bridge port
`$RESOURCE_USB_SERIAL_1_HOST:$RESOURCE_USB_SERIAL_1_PORT`, read with the
serial MCP tools. Each tty got an `echo ... | sudo tee /dev/ttySn`.

| tty | baud on board | text seen on the leased port |
|---|---|---|
| ttyS2 (main_uart0) | 115200 | **nothing** (`echo probe`; 0 bytes) |
| ttyS0 | 9600 | nothing |
| ttyS1 | 9600 | nothing |
| ttyS3 | 9600 | only NUL bytes (one NUL per byte sent: baud mismatch) |
| ttyS3 | set to 115200 (temporarily) | `DDDD-ttyS3-at-115200` arrived intact |

Conclusion: **the leased bridge port is wired to `ttyS3`**, not `ttyS2`.
ttyS3 was restored to 9600 / `sane` afterwards. No other change was made on
the board.

Zephyr's console is main_uart0 (`ttyS2`). With this wiring the Zephyr images
would print where the bridge cannot see them, so `tools/board.sh am62p tree`
could not collect a per-case list. Same class of fault as the earlier
`am62p-gles3-percase` attempt (wrong UART), but a different tty: that one was
mcu_uart0, this one is `ttyS3`.

## Not done (all of it)

- Firmware blob (`driver/tools/mkfwblob.sh`), gles3 image build, board runs
- Depth-PBO trace (`teximage2d_depth_pbo`, `teximage3d_depth_pbo`)
- Tally, `am62p-gles3-r2-fails.tsv` (not written: no results)

## What is needed to unblock

Move the bridge's UART cable to main_uart0 (the pins that appear as `ttyS2`
at 115200), then re-run this item. Check again with
`echo probe | sudo tee /dev/ttyS2` and look for the text on the bridge port.

Note: the earlier session record is tools/cts/log/board-sessions/am62p-plan1.md. It gives no tty wiring, so it does not change this finding.
