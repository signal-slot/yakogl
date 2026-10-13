<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-common-int: AM62P common set at the integration tip built, not run, 2026-10-14, integration cd738b1a

Details: `docs/board-sessions/am62p-common-int.md`.

## Open work

- AM62P benchmarks on the integration image (plan #18, item v3d-hh-gate):
  **partial**. All five rows (GM, SO, SV, CL, GT) build from `cd738b1a`
  (zephyr.bin hashes in the board-session doc); none ran, because the session
  was not allowed to use ssh to the board. HANDOFF should keep the last AM62P
  common-set result as the baseline and mark glmark2 terrain, refract and
  shadow as unconfirmed on the board after the npot and pvr-npot-mips merges.

## Next board session

- Run `tools/cts/cts-common.sh am62p run` on the images already built from this
  commit (command in the board-session doc) in a session that may use ssh. The
  console is the bridge's main_uart0 port, not the lease's mcu_uart0 port.
