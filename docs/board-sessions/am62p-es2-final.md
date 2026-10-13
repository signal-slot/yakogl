<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-final-b: AM62P dEQP-GLES2 images 10-17, ms4 tree, gl_test (dEQP covered by final-r2; gl_test BLOCKED, not run)

Date: 2026-10-13. Newest `origin/main` seen: `f8741799`.

## Result in short

- **dEQP (images 1-17, ms4 tree, whole-tree TOTAL, comparison with
  `32778b57`) is covered by `docs/board-sessions/am62p-es2-final-r2.md`** on
  `origin/main`: a whole-tree run at `63efed8e`, 17 images 17,125 pass / 0 fail
  / 34 NotSupported / 5 QualityWarning / 1 CompatibilityWarning; ms4 16 / 16;
  combined 17,141 pass / 0 fail. Cite the numbers from that record. This item
  ran no dEQP and produced no `board.sh` verdict or TOTAL line of its own.
- **gl_test 198 / 197 / 171 were not run**: no CASE record words exist. The
  console was silent and ssh was not granted (below).
- `docs/board-sessions/am62p-es2-r1c.md` and `docs/boards/am62p-console-lease.md`,
  named as the console procedure, were not readable in this tree.

## Blocker (exact)

1. **The leased console is silent.** The lease is `usb-serial-1` (the bridge
   port the earlier records identify as the mcu_uart0 one). The MCP console
   client reported `connected`, offset 0, 0 bytes buffered. A CR/LF written to
   it and a 6 s `serial_expect` on `.+` gave 0 bytes (timeout), re-checked in
   this round with the same result. The proven form (bridge port-33,
   main_uart0) is not in this lease.
2. **ssh to the board is refused.** `ssh -o BatchMode=yes torizon@<am62p>
   uname -r` returned "This command requires approval" and was not granted, so
   neither a console probe nor `BOARD_HOST` for `tools/board.sh` is available.
   The lease carries no `RESOURCE_*` ssh value for the AM62P.

With a console that carries no output the procedure stops here. Nothing was
flashed, rebooted or written to the board other than one CR/LF.

## To unblock (gl_test only)

Add the main_uart0 bridge port and the ssh login of the AM62P to the lease, or
allow ssh for the task. Then, in a worktree of `origin/main` under `$TMPDIR`,
use commit `63efed8e` so gl_test shares the commit of final-r2 (or the newest
tip, `f8741799`, if the owner prefers; record which). Build the gl_test image
as the GT row of `tools/cts/common-tree.txt` / `tools/cts/cts-common.sh am62p
build/run` do, and run cases 198, 197 and 171 **one at a time** with the knob
the gl_test sample documents (as `cases 196,198` was run on `452d6c31`). Record
each run's CASE record words and any FWCCB / CONTEXT_RESET lines.

## gl_test

| case | verdict | CASE record words | CONTEXT_RESET / FWCCB |
|------|---------|-------------------|-----------------------|
| 198 a2cconst | not run | - | - |
| 197 vtxgrid | not run | - | - |
| 171 | not run | - | - |
