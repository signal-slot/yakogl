<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 benchmark baseline (plan #2, item baseline-pi4)

**Not run, no measurement.** The session worker had no value for the variables
below, so nothing was built, nothing was run on the board, and no verdict line
exists. This file records why; it must be replaced by a real baseline when the
item is rerun.

Commit it would have measured: origin/main at 16a53f8d (the ref as seen from
the session's clone; re-record HEAD8 when the item is rerun).

Recipe when rerun: detached worktree at origin/main, then
`tools/cts/cts-common.sh pi4 build`, `run`, `tally` (tools/cts/README.md; set
in tools/cts/common-tree.txt).

| Row | Image | Record | board.sh verdict | Why it could not run |
|-----|-------|--------|------------------|----------------------|
| GM | glmark2 | none | none | not built: GLMARK2_SRC unset (also BOARD_HDMI_TIMINGS, BOARD_UART) |
| SO | slint, view off | none | none | not built: SLINT_NOSTD_DIR, NOSTD_PATCH_DIR, NOTO_SANS_DIR, RUSTUP_TOOLCHAIN unset (also BOARD_HDMI_TIMINGS, BOARD_UART) |
| SV | slint, view on | none | none | same as SO |
| CL | cluster, 1920x720 | none | none | not built: AGL_SLINT_CLUSTER_DIR, SLINT_NOSTD_DIR, NOSTD_PATCH_DIR, NOTO_SANS_DIR, RUSTUP_TOOLCHAIN unset (also BOARD_HDMI_TIMINGS, BOARD_UART) |
| GT | v3d_gl_test | none | none | not run: BOARD_HDMI_TIMINGS, BOARD_UART unset |

Variables that were set: ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR.

Figures the real baseline must still record, all unmeasured here: glmark2
score and ran/failed/unsupported; refract and terrain "out of GPU memory" and
"no GPU memory for a tiled copy" counts and fps; terrain GL error 0x502 lines;
Slint SO/SV verdicts and fingerprint; cluster probes and fps; gl_test pass
count; GPU resets.
