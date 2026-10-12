<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP gc_gl_test baseline on main, for PR #133

Date: 2026-10-12. Plan item base-imx8mp. Board: Verdin iMX8M Plus (GC7000), `BOARD_BOOT=uboot-mmc`,
`BOARD_HOST` from the environment (the leased resource's ssh login). The board has no UART, so the
result is the SNVS record that `tools/board.sh` decodes.

Result: **48 of 49 run, 1 skipped (case 16), failing 1: case 44.** That is the known failure (#6). No
other case failed, so no case was re-run on its own. One boot, 0 recoveries.

## Commit

`origin/main` after `git fetch origin` on 2026-10-12:
`63efed8e18501adbeec5677ac5576d51a48d3daf` (`Merge pull request #144 from signal-slot/work/issue-128`).
The plan named `87ce4a4c`, but main had moved on. PR #133's branch (`origin/work/int-pr49`, `af9ecc53`)
is not an ancestor of `63efed8e`, so this is main without the PR. The image was built and run from a clean
detached worktree at that commit (`dirty=0`). The runner prints `build 63efed8e`.

## Build

```sh
export ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws WEST_VENV=$ZEPHYR_WS/.venv \
       ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1 IMX8MP_OUT=$CCFARM_DEPS_DIR/imx8mp-out
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $IMX8MP_OUT/pr133-base-63efed8e GT
```

The GT row is `gc_gl_test -b verdin_imx8mp/mimx8ml8/a53 -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, with the
sample's default case list. Output:

```
common-images: build 63efed8e (63efed8e18501adbeec5677ac5576d51a48d3daf) board imx8mp
GT rc=0 sha256=4c0aa8c1c5c32d948963ba22cc34c60a1d12af6eb980334d19f198f9809c0881 build=GC_BUILD_STAMP=2026-10-12T09:04:32Z
common-images: 0 row(s) not built
```

## Run

```sh
BOARD_HOST=$BOARD_HOST BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
  $IMX8MP_OUT/pr133-base-63efed8e/GT/zephyr/zephyr.bin records/imx8mp/pr133-base-63efed8e gt-63efed8e-1
```

There was no `SEL`, so the whole default set ran. Output (the runner's own lines):

```
board.sh imx8mp: gt-63efed8e-1: build 63efed8e boot uboot-mmc (row) image sha256 4c0aa8c1c5c32d94 footprint 50843648
board.sh imx8mp: gt-63efed8e-1: image build: GC_BUILD_STAMP=2026-10-12T09:04:32Z; zephyr: v4.4.0-9848-gaf17c0c8aa2a
records:
  CASE (1) seq 14: set status 7 (set failed) passed 48 of 49 run; skipped 1, first skip case 16; failing 1: first case 44, second case none
board.sh imx8mp gt-63efed8e-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,002c0107,ffff0030,00100031 back_after=65s uptime=25s
```

Records dir: `records/imx8mp/pr133-base-63efed8e/20261012-1808-gt-63efed8e-1-4c0aa8c1/` (INDEX.txt,
meta.txt, snvs.txt; started 18:08:01, ended 18:09:26). `records/` is gitignored, so the directory stays on
the machine that ran it.

## Outcome

| item | value |
|---|---|
| image | sha256 `4c0aa8c1c5c32d948963ba22cc34c60a1d12af6eb980334d19f198f9809c0881` |
| set | passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44 |
| re-runs | none (44 is a known failure; neither 2 nor 10 failed) |
| boots | 1 (`boot_verdict=cycled`, `resets=absent`) |
| recoveries | 0 |

## Comparison with the last i.MX8MP set in docs/HANDOFF.md

docs/HANDOFF.md on main lists the last set as 47 / 48 run, 1 skipped (case 16, #8), first failure case 44
(#6), on `a0f439d6` (2026-10-10, `records/imx8mp/20261010-1623-gt-a0f439d6-44175f39`, record
`594b010e 002c0107 ffff002f 00100030`). On `63efed8e` the default list has one more case: 49 run (w3 low
half `0x31`) instead of 48 (`0x30`), and 48 passed (`0x30`) instead of 47 (`0x2f`). The first failure
(case 44, w1 `002c0107`), the skip (case 16) and the absence of a second failure are the same. Cases 2 and
10 (#25, #9) passed in this boot, which says nothing about their rates.

This is the reference for board-imx8mp. On `work/int-pr49-board`, a set that fails only case 44 (plus the
new PR #133 case, if it fails) matches main. Any other failure, apart from the 2 and 10 flakes, is a change.
