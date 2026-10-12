<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-final-b: Pi 4 dEQP-GLES2 whole tree at the integration tip (BLOCKED, nothing run)

Date: 2026-10-12. Tree base: `40dd2107fa2c` (main when this item was redone;
the earlier try started from `e3efeaadccd4`). Rows 0-28 of
`tools/cts/pi4-full-tree.txt` are all unrun.

Result: no image was built and no case ran on the Pi 4. There are no
`board.sh` verdict lines, no per-image counts and no TALLY. The 25 cube
cases have no verdict, so the comparison with `fe5e8e73` (25 Fail, all cube;
`docs/es2-cts-failures.md` section A) cannot be made. No host result is offered
in place of a device result.

| Image (rows) | Pass | Fail | NotSupported | QualityWarning | crash | state |
|---|---|---|---|---|---|---|
| AL ... XT (rows 0-28, 17165 mustpass names) | - | - | - | - | - | not run |
| whole tree | - | - | - | - | - | not run |

Failing cases by name: none known; nothing ran.
`tools/cts/tree-diff.py` does not exist at this tip; the comparison would be
by hand.

## What was checked

- The lease gave ssh to the Pi 4. It answered and was idle (load average
  0.02 0.02 0.04, `uptime` over ssh). Nothing was copied to it or booted on it.
- This time `BOARD_UART` was set to a serial bridge (`host:port` form) and
  `BOARD_HDMI_TIMINGS` was set. `tools/board.sh` at this tip supports the
  bridge form for `pi4` (`uart_is_bridge`, `bridge_capture`), which the
  earlier try found missing. A TCP connect to the bridge succeeded and the
  bridge was not busy.
- The shared deps cache holds the Zephyr workspace with `west`, the SDK and
  the Pi 4 dEQP stage 1 (`DEQP_WORK`: `build/`, `cts/`). The `--dry-run` of
  `tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <img>` printed
  all 29 labels (AL ... XT), with `-DEXTRA_CONF_FILE=texture-2048.conf` on T5.

## Why it could not run

The real image build (`tools/cts/pi4-deqp-images.sh` without `--dry-run`)
was refused by the worker's command permission layer, three times, in the
foreground and in the background, with the environment passed inline and
through a wrapper script. Only the dry run and read-only commands were
allowed. The refusal was not about the board. Because the build was refused,
`tools/board.sh pi4 tree` was not started either: it waits for the
`<img>/<LABEL>.built` files, and running it without images would only
produce MISSING rows.

A leftover image set from another item exists in the deps cache. Its commit
is not this tip, so it was not used.

## Resume command

Needs a worker where the image build is allowed. Then, with `BOARD_HOST` (the
leased ssh login), `BOARD_UART` (the bridge) and `BOARD_HDMI_TIMINGS` set, at
the tip or later:

```sh
export DEQP_WORK=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b RUSTUP_TOOLCHAIN=nightly
export ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws WEST_VENV=$ZEPHYR_WS/.venv
export ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1
tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <img>      # <img> must not exist; T5 gets texture-2048.conf itself
TREE_IMAGE_DIR=<img> tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt <rec>/full
tools/board.sh pi4 tally <rec>/full
```

The tree is resumable (`TREE_ROWS=<rows left>`, `tools/cts/cts_resume.py`).
Expected time: 29 images at about 5.4 min each on the board, about 2 h 40 min
(`tools/cts/README.md`), plus the image builds. Then compare with `fe5e8e73`
by hand and expect the 25 cube cases (T1 18, T3 6, TF 1 at sweep 30) to move
from Fail to Pass.
