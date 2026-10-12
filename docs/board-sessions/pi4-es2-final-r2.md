<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-final-r2: Pi 4 dEQP-GLES2 whole tree from the origin/main tip (BLOCKED, nothing run)

Date: 2026-10-12. `origin/main` tip after `git fetch origin`: `d56e79e5`
("Merge pull request #182 from signal-slot/work/issue-8"). The workspace
started at `db3545fa`, an ancestor of that tip. No image was built from
`d56e79e5` and no case ran on the Pi 4.

Result: there are no `board.sh` verdict lines, no per-row counts and no TALLY.
No host result is offered in place of a device result.

| Image (rows) | Pass | Fail | NotSupported | QualityWarning | crash | state |
|---|---|---|---|---|---|---|
| rows 0-28 (17165 mustpass names) | - | - | - | - | - | not run |
| whole tree | - | - | - | - | - | not run |

Failing cases by name: none known; nothing ran.

## Blocking errors (exact)

1. The console value is not available to this worker. The farm's
   `fetch_from_farm` for `BOARD_UART` answered:
   `no online node exports BOARD_UART for yakogl (only paths of node-local values, dependency caches and [worker] export_paths are lent; values that are not paths and secrets never are)`.
   It is a `host:port`, not a path, so it cannot be lent. `BOARD_HDMI_TIMINGS`
   is the same kind of value.
2. Reading the environment is refused in this session: `printenv BOARD_UART`,
   `printenv BOARD_HDMI_TIMINGS`, `printenv MESA_V3D_TREE`,
   `printenv DEQP_WORK` and a shell loop testing whether they are set each
   ended with "This command requires approval". `get_project_env` ended with
   "Claude requested permissions to use mcp__ccfarm__get_project_env, but you
   haven't granted it yet." So it could not be shown that any of the four is
   set. The task statement says `MESA_V3D_TREE` and `DEQP_WORK` have no value
   on this machine.
3. `docs/board-sessions/` does not exist at `db3545fa` or `d56e79e5`. The
   records the task names (`pi4-es2-r1.md`, `pi4-console-probe.md`,
   `zephyr-ws-setup.md`) are not in this tree, so the comparison below is
   limited to the one record found on the integration branch (see below).

## What was checked

- The leased Pi 4 answers over ssh, idle (`uptime`: load average
  0.03 0.04 0.04, up 35 min). Nothing was copied to it or booted on it.
- `tools/board.sh` at this tip accepts `BOARD_UART` as `host:port` of a serial
  bridge for `pi4` (`uart_is_bridge`, `bridge_capture`), so the earlier
  blocker (no TCP console form in `board.sh pi4`) is gone from the tooling.
  Only the value is missing.
- The shared deps cache holds `zephyr-ws`, `zephyr-sdk-1.0.1`,
  `deqp-stage1-rpi_4b` and `pi4-deqp-images`. They were not used, because the
  run cannot be done without the console, and an image set takes hours.

## Diff against pi4-es2-r1-tip.md

`pi4-es2-r1-tip.md` (the integration-branch version, commit `ce60c5e2`) is
itself a blocked note: no per-case results, no TALLY. There are no failing
cases to diff. The only baseline with numbers is `fe5e8e73` (25 Fail, all
cube cases; `docs/es2-cts-failures.md` section A). This session cannot say
whether the 25 cube cases now pass.

## Resume

With `BOARD_HOST` set to the leased ssh login, `BOARD_UART` (`host:port` of the
Pi 4 bridge), `BOARD_HDMI_TIMINGS`, `MESA_V3D_TREE` and `DEQP_WORK` (the
`rpi_4b` stage 1), at `d56e79e5` or later, in a worktree or clone:

```sh
tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-d56e79e5
TREE_IMAGE_DIR=build/deqp/pi4-d56e79e5 tools/board.sh pi4 tree \
  tools/cts/pi4-full-tree.txt records/pi4/deqp-d56e79e5
tools/board.sh pi4 tally records/pi4/deqp-d56e79e5
```

The tree is resumable (`TREE_ROWS=<rows left>`).
