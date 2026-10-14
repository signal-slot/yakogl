<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gles3: Pi 4 full dEQP-GLES3 tree session (item board-pi4)

Date: 2026-10-15. Integration tip `4a4fad60d8febb500286dbbfb63950988431aee7`.

**Result: not run. The board could not be reached.** Nothing in this record is
a board result; there are no image hashes, record directories or counts.

## Cause

The lease named `pi4-elegance` with the login `signal-slot@elegance-1` (on the
tailnet). From the worker that ran this item:

```
ssh signal-slot@elegance-1 -- uname -a
ssh: Could not resolve hostname elegance-1: Name or service not known
```

The name does not resolve on this worker (it is not on the tailnet, or its
resolver does not know the host). `BOARD_UART` and `BOARD_HDMI_TIMINGS` were also
not set in the shell (ccfarm reported them as missing environment), so
`tools/board.sh pi4 tree` could not have run even with a reachable host.
Per the item ("if the board is unreachable, record the cause and stop") no
retry was made, and no image was built (the images are only useful with a
board to run them on).

## Not run

- all 91 images of `tools/cts/pi4-gles3-tree.txt` (not built, not run);
- gl_test 196, 200 to 205 (no verdict line);
- the ES 2.0 default gl_test set and the ES 2.0 regression rows (D, FB1, T1, T3, TF).

The 7-image baseline of `cd738b1a` (`pi4-gles3-baseline.md`) stands unchanged.
`pi4-gles3-fails.tsv` was not written: there is no non-Pass case that ran.

## To rerun

Follow `docs/es3-board-recipe-pi4.md` sections 3, 4 and 8 on a worker that can
resolve and reach the lease's host and has `BOARD_UART` and `BOARD_HDMI_TIMINGS`.
The gles3 stage 1 exists in the deps cache
(`deqp-stage1-rpi_4b-gles3/work/build/modules/gles3`); the gles2 stage 1 for the
ES 2.0 rows (`DEQP_WORK`) was not found there. Run the tree in rows of at most
four, or as a background job with the longest timeout (HANDOFF "Known defects").

## Retry after permissions were granted

`tailscale status` on the worker lists `elegance-1` (100.70.54.122) as active,
but `ssh signal-slot@elegance-1` still fails with the same resolver error and
`getent hosts elegance-1` returns nothing (MagicDNS name not resolvable here).
The lease allows ssh to the host only as written, so the tailnet address was
not substituted. `BOARD_UART` is still unset. `BOARD_HDMI_TIMINGS`, `ZEPHYR_WS`,
`ZEPHYR_SDK_INSTALL_DIR` and `WEST_VENV` are now set. A rerun needs the name
resolved (or the address allowed in the lease) and `BOARD_UART`.
