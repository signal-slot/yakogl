<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, common set from the integration image: not run

Status: **nothing was built and nothing ran on the board.** No verdict line
and no tally line exists; none is quoted here.

HEAD8: `198e8a5d` (ccfarm/integration tip, the workspace's snapshot).

## What was established

- The board answers: `ssh signal-slot@elegance-1 -- true` returned 0.
- Inputs present in the project's deps cache (`$CCFARM_DEPS_DIR`, here
  `/home/tasuku/.local/share/ccfarm/deps/yakogl`, seen by listing only, not
  built against): `zephyr-ws` (build-proof, modules, yakogl, zephyr),
  `zephyr-sdk-1.0.1`, `glmark2-src`, `slint-nostd`, `nostd`,
  `agl-slint-cluster`, `noto-sans` (Regular, Light, ExtraLight).
- `tools/board.sh pi4` reads the record from a character device on the
  machine that runs it (`BOARD_UART`, board.sh lines 751 and 822). This
  worker has no `/dev/ttyACM*` and no `/dev/serial` (`test -c /dev/ttyACM0..2`
  and `test -d /dev/serial` all failed). The Debug Probe is not attached to
  it, and no value for `BOARD_UART` is known to me.

## What could not run

| step | outcome |
|---|---|
| `mcp__ccfarm__ask_farm` (UART device and host, HDMI timings) | permission refused |
| `printenv`/`env` (to read ZEPHYR_WS etc. from the lease) | refused |
| `tools/cts/common-images.sh --dry-run pi4 ...` | refused |
| `tools/cts/common-images.sh pi4 tools/cts/common-tree.txt <img>` (GM SO SV CL GT) | not run: with no UART, no image could be judged, and a build alone gives no verdict |
| `tools/board.sh pi4 run ...`, `common-tally.py` | not run: `BOARD_UART` has no device on this worker |

`BOARD_HDMI_TIMINGS` would be the 1920x720 value of tools/cts/README.md
section 6 (`1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1`); I did
not export it because the run did not start.

## Expected, to check when it runs

Boot line `carve-out 96 MiB`; refract and terrain print no memory refusal
("out of GPU memory", "no GPU memory for a tiled copy"); terrain not black
(the record cannot show that by itself: it needs the glmark2-tally-scenes
item's per-scene tally or a look at the panel); Slint and cluster without GL
errors; gl_test as at baseline2-pi4.

## Before / after

| benchmark | before (baseline2-pi4) | after (this run) |
|---|---|---|
| GM glmark2 | not available in this workspace (branch ccfarm/01a115b0-... is not in the ref copy) | not run |
| SO / SV slint | same | not run |
| CL cluster | same | not run |
| GT v3d_gl_test | same | not run |

## To rerun

Needs a worker that has the Debug Probe's by-id device (the board's host, or
one the lease says carries it) and: `BOARD_HOST=signal-slot@elegance-1`,
`BOARD_UART`, `BOARD_HDMI_TIMINGS`, `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC`, `SLINT_NOSTD_DIR`,
`NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `NOTO_SANS_DIR`,
`RUSTUP_TOOLCHAIN=nightly` (build only). Then:

```sh
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt $TMPDIR/img
tools/board.sh --dry-run pi4 run $TMPDIR/img/GM/zephyr/zephyr.bin $TMPDIR/rec/GM GM-198e8a5d
tools/board.sh pi4 run $TMPDIR/img/<LABEL>/zephyr/zephyr.bin $TMPDIR/rec/<LABEL> <LABEL>-198e8a5d   # GM SO SV CL GT
tools/cts/common-tally.py <rec>
```
