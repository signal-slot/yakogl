<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Pi 4 dEQP-GLES2 whole tree from the origin/main tip (pi4-es2-final-a): blocked, nothing run

Date: 2026-10-12. Item `pi4-es2-final-a` (plan #20, roadmap stage 1).

**Result: no image was built and no case ran on the Pi 4.** There is no
`board.sh` verdict line and no TALLY to quote, and this record gives no
numbers in place of device results. All 29 rows (0-28) of
`tools/cts/pi4-full-tree.txt` are still missing, so all 17,165 mustpass
names are still without a result at this tip.

- origin/main tip after `git fetch origin`: `2a3a36fca96f` ("Merge pull
  request #138 from signal-slot/work/int-pr92"). The images would have been
  built from this commit. They were not built (see 3).
- Workspace: main `db3545fa25e2` (an ancestor of that tip, 177 commits
  behind). On origin/main, `tools/board.sh` has the TCP console form:
  `uart_is_bridge` (BOARD_UART = `host:port`), `bridge_capture`, and
  `pi4_uart_free` skipping the local-device scan for a bridge.

## 0. Step 0: is there a pi4-es2-tip-r2 record to cite?

No. The only `pi4-es2-tip-r2.md` is in commit `6ac3b755` ("pi4 es2 whole
tree at a25712f2: blocked, console value unreadable"). It holds no tree and
no TALLY: it was blocked for the same reason as this session (section 1).
It is not in this tree or on origin/main. So the source-diff check has
nothing to compare against, and the tree had to run here.

## 1. Blocker 1: the console bridge value could not be read

`tools/board.sh pi4 tree` needs `BOARD_UART` (the `host:port` of the Pi 4's
raw TCP serial bridge) and `BOARD_HDMI_TIMINGS`. Neither could be obtained:

1. ccfarm reports that this node has no `BOARD_UART` value for the project.
   The lease names only the board's ssh login; no console channel is part
   of it.
2. Every command that reads the environment was refused. `printenv TMPDIR
   CCFARM_DEPS_DIR` got "This command requires approval", and any command
   with a `$VAR` got "A variable in this command can't be checked before it
   runs".
3. Both farm tools that could supply the value were refused:
   `mcp__ccfarm__ask_farm` (twice) and `mcp__ccfarm__get_project_env` each
   returned "Claude requested permissions to use ... but you haven't granted
   it yet."
4. An earlier session on this node left a pty-to-TCP bridge link in the
   project's dependency cache (`pi4-uart`, made by a `uartbridge.py` helper
   there). The link still points at a live pseudo-terminal, but no
   `uartbridge` process runs (`pgrep -af uartbridge` matches only itself).
   So the pty number may now belong to an unrelated terminal. It was **not**
   used: `stty` on it and a read of it could disturb someone else's
   session, and it is not proven to be the Pi 4 console.

The same cause, as recorded in
`tools/cts/log/board-sessions/farm-console-blocked-2026-10.md` and in
`6ac3b755`: the console value lives in the node environment or the farm's
notes, and this session's harness could not read either.

## 2. Blocker 2: the lease is shared

This turn holds the Pi 4 with lock mode **shared**. `board.sh pi4
tree` stages each of the 29 images on the Pi over ssh and reboots it into
Zephyr. Other tasks holding the same shared lease would lose the board
during that time. Even with the console value, the tree must not run under a
shared lease. It needs an exclusive lease (or a `capability:power-cycle` /
`serial-console` requirement that serializes the Pi 4).

The board itself was reachable: one read-only `uptime` over ssh showed
Linux up 1 day 8:52, load 0.00, 2 users. Nothing else was done on it. It
was not rebooted and nothing was staged.

## 3. Images: not built

- `tools/cts/pi4-deqp-images.sh` needs `DEQP_WORK`, a dEQP-GLES2 stage 1 for
  `rpi_4b` (`$DEQP_WORK/cts`, and
  `$DEQP_WORK/build/modules/gles2/libdeqp-gles2-package.a`). This node had
  none: no `libdeqp-gles2-package.a` under the dependency cache or the
  owner's checkouts (searched to depth 7, the Zephyr workspace excluded).
  `fetch_from_farm DEQP_WORK` copied one from another node into this node's
  dependency cache (`DEQP_WORK`, 2.0 GB). The tool call itself timed out,
  but the copy went on and completed. It passes the images script's check:
  `cts/` exists and `build/modules/gles2/libdeqp-gles2-package.a` is
  present. It has not been built against yet. The CMake build directory
  came from another machine, so absolute paths in it may need a stage-1
  rebuild (`tools/cts/cts-build-zephyr.sh`) if `pi4-deqp-images.sh` fails
  on them.
- A worktree of the origin/main tip could not be made. `git worktree add
  --detach <dir> 2a3a36fca96f` (under /tmp and under the dependency cache)
  and `git archive 2a3a36fca96f | tar -x` were refused ("requires
  approval").
- With blockers 1 and 2 in force, the images could not have been run, and
  origin/main will move before the next session. So no build was forced.

## 4. Results

| Row(s) | Image | Pass | Fail | NotSupported | QualityWarning | crash |
|---|---|---:|---:|---:|---:|---:|
| 0-28 | all 29 | not run | not run | not run | not run | not run |

Whole-tree total: **0 of 17,165** mustpass names have a result. Missing:
rows 0-28 (all), for the reasons in sections 1-3.

`board.sh` verdict lines: none (the runner was not invoked).
`tools/board.sh pi4 tally`: none (no record directory).

Failing cases: none measured. The 25 `fe5e8e73` cube Fail cases
(`tools/cts/log/board-sessions/pi4-es2-r2-baseline.md` section 2a,
`docs/es2-cts-failures.md` section A) each have the verdict **not run**.
The expected outcome is Pass: the non-seamless cube fix (`LodKey::cube_noseam`)
is on origin/main. That is unproven on the device.

| # | Case (`dEQP-GLES2.functional.` dropped) | Row (image) | fe5e8e73 | This session |
|---:|---|---|---|---|
| 1 | `shaders.texture_functions.vertex.texturecubelod` | 24 (TF) | Fail | not run |
| 2 | `texture.mipmap.cube.basic.linear_linear` | 21 (T3) | Fail | not run |
| 3 | `texture.mipmap.cube.basic.linear_nearest` | 21 (T3) | Fail | not run |
| 4 | `texture.mipmap.cube.bias.linear_linear` | 21 (T3) | Fail | not run |
| 5 | `texture.mipmap.cube.bias.linear_nearest` | 21 (T3) | Fail | not run |
| 6 | `texture.mipmap.cube.projected.linear_linear` | 21 (T3) | Fail | not run |
| 7 | `texture.mipmap.cube.projected.linear_nearest` | 21 (T3) | Fail | not run |
| 8 | `texture.vertex.cube.filtering.linear_mipmap_linear_linear_clamp` | 19 (T1) | Fail | not run |
| 9 | `texture.vertex.cube.filtering.linear_mipmap_linear_linear_mirror` | 19 (T1) | Fail | not run |
| 10 | `texture.vertex.cube.filtering.linear_mipmap_linear_nearest_clamp` | 19 (T1) | Fail | not run |
| 11 | `texture.vertex.cube.filtering.linear_mipmap_linear_nearest_mirror` | 19 (T1) | Fail | not run |
| 12 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_clamp` | 19 (T1) | Fail | not run |
| 13 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_mirror` | 19 (T1) | Fail | not run |
| 14 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_repeat` | 19 (T1) | Fail | not run |
| 15 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_clamp` | 19 (T1) | Fail | not run |
| 16 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_mirror` | 19 (T1) | Fail | not run |
| 17 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_repeat` | 19 (T1) | Fail | not run |
| 18 | `texture.vertex.cube.wrap.clamp_clamp` | 19 (T1) | Fail | not run |
| 19 | `texture.vertex.cube.wrap.clamp_mirror` | 19 (T1) | Fail | not run |
| 20 | `texture.vertex.cube.wrap.clamp_repeat` | 19 (T1) | Fail | not run |
| 21 | `texture.vertex.cube.wrap.mirror_clamp` | 19 (T1) | Fail | not run |
| 22 | `texture.vertex.cube.wrap.mirror_mirror` | 19 (T1) | Fail | not run |
| 23 | `texture.vertex.cube.wrap.mirror_repeat` | 19 (T1) | Fail | not run |
| 24 | `texture.vertex.cube.wrap.repeat_clamp` | 19 (T1) | Fail | not run |
| 25 | `texture.vertex.cube.wrap.repeat_mirror` | 19 (T1) | Fail | not run |

## 5. Resume

Prerequisites: an **exclusive** Pi 4 lease, `BOARD_UART` (bridge
`host:port`) and `BOARD_HDMI_TIMINGS` exported in the task shell from the
node environment (never written to a file), and `DEQP_WORK` (now in this
node's dependency cache, section 3). Then, from a worktree of the origin/main tip
(`<tip>`):

    git worktree add --detach <wt> origin/main
    cd <wt>   # with DEQP_WORK, ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR, RUSTUP_TOOLCHAIN=nightly
    tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <imgdir>
    # prove the console first (farm-console-blocked-2026-10.md, last line):
    # the first image's dEQP banner must arrive over the bridge
    TREE_IMAGE_DIR=<imgdir> BOARD_HOST=<from the lease> \
      tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt records/pi4/deqp-<tip>
    tools/board.sh pi4 tally records/pi4/deqp-<tip>

If the run stops, resume the rows that are left with `TREE_ROWS=<rows>`
(`tools/cts/cts_resume.py`). Compare per row with
`tools/cts/log/board-sessions/pi4-es2-r2-baseline.md` section 1. The
expected TALLY is there in section 3: 17,127 pass / 0 fail / 35
NotSupported / 3 QualityWarning, with +1 pass / -1 NotSupported if
`shaders.fragdata.invalid_assign_to_1` passes.
