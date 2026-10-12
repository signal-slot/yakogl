<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, PR #133 baseline (v3d_gl_test on main): not run

Date: 2026-10-12. Plan #141, item base-pi4: the whole default `v3d_gl_test`
set on origin/main, without PR #133, as the Pi 4 baseline the later
board-pi4 item compares work/int-pr49-board against.

Status: **the image was built; nothing ran on the board.** The board was not
rebooted. There is no `gl_test: all: ...` line, no failing case line, no records/...
directory and no recovery count from this session. No host result is offered
in place of a device result.

## Commit

The baseline commit is origin/main `2fe4b8798b5f` ("Merge pull request #141
from signal-slot/work/issue-127"), fetched in this session; the plan named
`87ce4a4c`, and main has since moved by three commits (`a52f9155`,
`86239c24`, `2fe4b879`), none of them under
`driver/samples/zephyr/v3d_gl_test/`. The sample's default list is
`CONFIG_V3D_GL_TEST_CASES` = `5,58,59,66,67,102,3,9,11,12,13,4` (12 cases).

## Build

From a detached worktree of `2fe4b879` (clean), with `ZEPHYR_WS`,
`WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and `RUSTUP_TOOLCHAIN=nightly` from the
environment:

```sh
$SW/tools/cts/common-images.sh pi4 $SW/tools/cts/common-tree.txt <img> GT
```

```
common-images: build 2fe4b879 (2fe4b8798b5faf2807f31dd486296f21045f4abf) board pi4
GT rc=0 sha256=a429543dc4c803224e186229586152a96209089c403fd6d804727a7eb42fd1b5 build=V3D_BUILD=2fe4b879
common-images: 0 row(s) not built
```

The image is 2,128,664 bytes, on Zephyr `af17c0c8` (the workspace's pin).

## Why it did not run on the board

`v3d_gl_test` reports only on its UART and the HDMI screen (the frozen
record, `v3d_gl_test RECORD` ... `RECORD end`). So `board.sh pi4` needs
`BOARD_UART`: here that is the `host:port` of the serial bridge in front of the
Pi 4's Debug Probe, which the farm's notes name. This worker's environment has
no `BOARD_UART`, and the unattended session refused every `board.sh pi4 run`
invocation that passed `BOARD_HOST` and `BOARD_UART` into its environment,
the `--dry-run` included. So there is no verdict line. A tryboot without the
capture would have been a reboot that measures nothing, so the board was not
rebooted.

Earlier in the session the build was refused too (`git worktree add`, `git
clone`, `git checkout`, `cp` of the workspace, and `ask_farm` not granted).
After the permission decisions the worktree and `ask_farm` worked; `git clone`
and `cp` stayed denied and were not needed.

## What was checked on the board (ssh, read-only)

The board answers ssh with the leased key; Linux 6.12 aarch64, up 1 h 59 min,
load average 0.08 0.04 0.00; no `yakogl-*.bin` staged in `/boot/firmware`,
no tryboot.txt pending, no permanent default (`config-linux.txt` absent).
Nothing was copied to it or run on it.

## Rows

| row | image | verdict line | gl_test line | records | recoveries |
|---|---|---|---|---|---|
| GT `v3d_gl_test`, default set | `a429543dc4c80322...`, `V3D_BUILD=2fe4b879` | not run | not run | none | not counted (no run) |

## Comparison with the last Pi 4 set

There is nothing of this session's to compare. For the next session, the last
Pi 4 sets of this list:

- docs/HANDOFF.md "State of main": 12 / 12 on `afa12e9b` (not run since).
- tools/cts/RESULTS.md, the common set round on `36bdc26f`: `gl_test: all: 0 of 12 cases
  failed (12 passed; 0 timed out ...)`, `v3d_gl_test: 12 of 12 cases passed`,
  resets 0.
- An untracked record in the main checkout, not in the repository:
  `records/pi4/gt-1cf946a9/20261011-0152-gt-1cf946a9-4555048a` (2026-10-11,
  `V3D_BUILD=1cf946a9`): `board.sh pi4 gt-1cf946a9 rc=0 verdict=done
  boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end
  resets=0 record=absent back_after=262s` and `gl_test: all: 0 of 12 cases
  failed (12 passed; 0 timed out; first failure case -1 at index -1; second
  failure case -1; 0 skipped, first skip case -1); precompiled hits 0,
  compiler runs 18`. `1cf946a9` is an ancestor of `2fe4b879`; it is the
  nearest Pi 4 set to this baseline, but it is not this baseline.

## To run

On a worker that has the Pi 4's console (`BOARD_UART`) and the Zephyr inputs
(`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `RUSTUP_TOOLCHAIN=nightly`),
from a detached checkout of origin/main (`$SW`), board idle, one GL client:

```sh
$SW/tools/cts/common-images.sh pi4 $SW/tools/cts/common-tree.txt <img> GT
$SW/tools/board.sh --dry-run pi4 run <img>/GT/zephyr/zephyr.bin <rec> gt-<sha8>
$SW/tools/board.sh pi4 run <img>/GT/zephyr/zephyr.bin <rec> gt-<sha8>
```

`BOARD_HOST`, `BOARD_UART` and `BOARD_HDMI_TIMINGS` from the environment.
Expected from the sets above: `gl_test: all: 0 of 12 cases failed`, resets 0.
If a case fails, run it alone once more (`-DCONFIG_V3D_GL_TEST_CASES="<n>"`).
The default list holds no 3D or 2D-array mip-level case, so PR #133's own
case is not in it: the board-pi4 item runs that case by itself.
