<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-r1a2: Pi 4 dEQP-GLES2 rows 15-28 at the origin/main tip (BLOCKED)

Date: 2026-10-15 (two attempts, same result). Item `pi4-es2-r1a2` (plan #20,
roadmap stage 1).

**Result: no run. The Pi 4 could not be reached from the worker, so rows 15-28
produced no result at all. No Pass/Fail number in this file comes from a
device; there are none.** The 0-Fail baseline for these rows stays
`pi4-es2-final-r2.md` (`d56e79e5`).

- Intended commit: `origin/main` after `git fetch origin`,
  `548860dfe42031d81ffd65f93881bca8992539dc` (second attempt; the first
  attempt saw `5a1cf4a2`). No image was built from it, because without the
  board there was nothing to run; a host-only build is deliberately not
  presented as a result.
- Rows 15-28 of `tools/cts/pi4-full-tree.txt` (S4 S5f S5v SMALL T1 T2 T3 T4
  T5 TF U1 VA DR XT): not run.

## What was tried, verbatim

1. Board login, with the leased ssh destination (both attempts):

```
$ ssh <board-login> -- uname -a
ssh: connect to host <board-host> port 22: Connection timed out
```

2. Serial/power bridge host (`usb1`, leased):

```
$ ssh <bridge-host> -- true
ssh: connect to host <bridge-host> port 22: Connection refused
```

3. Bridge TCP port: `ask_farm` gave the bridge host:port (not recorded
   here). In the first attempt a TCP connect to it was accepted, so the
   bridge was up. In the second attempt `ask_farm` reported that no farm node
   can ssh to the board and gave no power-cycle method. The board is off or
   off the network, so `board.sh` cannot make its ssh/sudo pre-check or stage
   an image.

`tools/board.sh pi4 tree` needs both `BOARD_HOST` (ssh + sudo) and
`BOARD_UART`; `BOARD_UART` is not set in the worker environment. It dies in
its pre-checks without them, so it was not started.

## Per row, against pi4-es2-final-r2.md

| Row | Name | This run | r2 (d56e79e5) |
|---|---|---|---|
| 15 | S4 | not run | 2027 Pass, 0 Fail |
| 16 | S5f | not run | 100 Pass |
| 17 | S5v | not run | 100 Pass |
| 18 | SMALL | not run | 227 Pass |
| 19 | T1 | not run | 224 Pass |
| 20 | T2 | not run | 248 Pass |
| 21 | T3 | not run | 304 Pass |
| 22 | T4 | not run | 69 Pass |
| 23 | T5 | not run | 7 Pass |
| 24 | TF | not run | 20 Pass |
| 25 | U1 | not run | 1267 Pass |
| 26 | VA | not run | 571 Pass |
| 27 | DR | not run | 101 Pass |
| 28 | XT | not run | 3 Pass, 9 NotSupported |

TALLY / verdict lines printed by the device: none (nothing ran).

## Failing cases

Unknown: not run. This file does not state 0 Fail.

## What the retry needs

- The board powered on and reachable by ssh (power cycle or network fix by
  the owner; no farm node can reach it now).
- `BOARD_UART` from the farm; set it with `ccfarm env set`.
- Then: build `tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <out>`
  from a detached worktree of the newest `origin/main`, and run
  `TREE_ROWS=15-28 TREE_IMAGE_DIR=<out> BOARD_BOOT=tryboot tools/board.sh pi4
  tree tools/cts/pi4-full-tree.txt <recdir>` (about 5 minutes a row).
