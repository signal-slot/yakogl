<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-common-int: AM62P common set at the integration tip not run, 2026-10-14, integration cd738b1a

Details: `docs/board-sessions/am62p-common-int.md`.

## Open work

- AM62P benchmarks on the integration image (plan #18, item v3d-hh-gate):
  **not run**. The session's shell refused the build and the board login could
  not be read, so there is no GM / SO / SV / CL / GT verdict at `cd738b1a`.
  HANDOFF should keep the last AM62P common-set result as the baseline and
  mark glmark2 terrain, refract and shadow as unconfirmed on the board after
  the npot and pvr-npot-mips merges.

## Next board session

- Re-dispatch with a shell that may run `bash`/build scripts and read the
  lease's login. The recipe is in the board-session doc. The build inputs are
  in the deps cache already. Use the bridge's main_uart0 port as the console
  (see `docs/board-sessions/am62p-es2-final-r2.md`).
