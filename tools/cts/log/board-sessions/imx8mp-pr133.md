<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP session for PR #133: case 206 and the gc_gl_test set

Date: 2026-10-12. Plan item board-imx8mp. Board: Verdin iMX8M Plus (GC7000), `BOARD_BOOT=uboot-mmc`,
`BOARD_HOST` from the leased resource's ssh login. The board has no UART, so each result is the SNVS
record that `tools/board.sh` decodes.

Result: **case 206 passed. The set gave passed 48 of 49 run, skipped 1 (case 16), failing 1: case 44.**
That is the same as the main baseline, down to the record words. Two boots, one per image, and 0
recoveries. No case was re-run.

## Commits

| ref | commit |
|---|---|
| built from | `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`gl_test: case 206 reads back 3D and 2D-array mip levels 1 and 2 (PR #133)`) |
| `origin/work/int-pr49-board` | not on origin (`git fetch origin`, 2026-10-12). `066a2db8` is the commit that board-branch was going to push as that branch. It is on `origin/ccfarm/01a12f31-af26-7505-a35a-153b04454dae` |
| PR #133 head (`origin/work/int-pr49`) | `af9ecc535ac00d84fb3f0fa9af6db1918b8c9a2d` |
| main merged into the branch | `2fe4b879` (merge `81f4c089`) |
| baseline main (`imx8mp-pr133-baseline.md`) | `63efed8e18501adbeec5677ac5576d51a48d3daf` |
| `origin/main` at the session | `502032bf90d03891d34c8a997372940a7348fa24` (not merged into the branch) |

The branch's main (`2fe4b879`) and the baseline's main (`63efed8e`) are different commits.

## Build

Environment: `ZEPHYR_WS`, `WEST_VENV` and `ZEPHYR_SDK_INSTALL_DIR` are this machine's project values.
`IMX8MP_OUT=$CCFARM_DEPS_DIR/imx8mp-out`. Each image was built from a detached worktree at `066a2db8`.

The default set uses the GT row unchanged, from a clean worktree:

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $IMX8MP_OUT/pr133-066a2db8 GT
```
```
common-images: build 066a2db8 (066a2db81d40e519efc05dcbdcdd4e359468fe9a) board imx8mp
GT rc=0 sha256=c30d8bd64de663c44b6d9bfdc543296767420fa48578e651c57e657275f9731a build=GC_BUILD_STAMP=2026-10-12T10:01:53Z
common-images: 0 row(s) not built
```

The case image is the same GT row, from a second worktree at `066a2db8`. The case is selected with
`CONFIG_GC_GL_TEST_CASE` (`driver/samples/zephyr/gc_gl_test/Kconfig`, default 7, the regression set).
Case 206 is not in the regression set, so I added one line to that worktree's
`driver/samples/zephyr/gc_gl_test/prj.conf` and did not commit it: `CONFIG_GC_GL_TEST_CASE=206`. Hence
`-dirty`. The built `.config` has `CONFIG_GC_GL_TEST_CASE=206`; the set image's has `=7`.

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $IMX8MP_OUT/pr133-066a2db8-c206 GT
```
```
common-images: build 066a2db8-dirty (066a2db81d40e519efc05dcbdcdd4e359468fe9a) board imx8mp
GT rc=0 sha256=0f082a546bd2698a3e04be107878a28701c0b9628ad5d275b883133a0fb750b5 build=GC_BUILD_STAMP=2026-10-12T10:05:11Z
common-images: 0 row(s) not built
```

## Runs

`tools/board.sh --dry-run imx8mp run` on the case image first. It printed the expected sequence: lock,
pre-checks, SNVS clear, staging to `/var/yakogl` on the eMMC, the `yako_once` one-shot, reboot, read-back.
Then the real runs, one after the other (one GL client at a time), each run's output going to a file:

```sh
BOARD_HOST=$BOARD_HOST BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
  $IMX8MP_OUT/pr133-066a2db8-c206/GT/zephyr/zephyr.bin records/imx8mp/pr133-066a2db8 c206-066a2db8-1 > c206.out 2>&1
BOARD_HOST=$BOARD_HOST BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
  $IMX8MP_OUT/pr133-066a2db8/GT/zephyr/zephyr.bin records/imx8mp/pr133-066a2db8 gt-066a2db8-1 > gt.out 2>&1
```

Case 206 (the runner's own lines):

```
board.sh imx8mp: c206-066a2db8-1: build 066a2db8 boot uboot-mmc (row) image sha256 0f082a546bd2698a footprint 50855936
board.sh imx8mp: c206-066a2db8-1: image build: GC_BUILD_STAMP=2026-10-12T10:05:11Z; zephyr: v4.4.0-9848-gaf17c0c8aa2a
records:
  CASE (1) seq 14: case 206 check 255 status 1 (pass) got 0x00000006 want 0x00000000
board.sh imx8mp c206-066a2db8-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00ceff01,00000006,00000000 back_after=33s uptime=26s
```

The default set:

```
board.sh imx8mp: gt-066a2db8-1: build 066a2db8 boot uboot-mmc (row) image sha256 c30d8bd64de663c4 footprint 50855936
board.sh imx8mp: gt-066a2db8-1: image build: GC_BUILD_STAMP=2026-10-12T10:01:53Z; zephyr: v4.4.0-9848-gaf17c0c8aa2a
records:
  CASE (1) seq 14: set status 7 (set failed) passed 48 of 49 run; skipped 1, first skip case 16; failing 1: first case 44, second case none
board.sh imx8mp gt-066a2db8-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,002c0107,ffff0030,00100031 back_after=87s uptime=21s
```

Record directories (`records/` is gitignored, so they stay on the machine that ran them, under the
`066a2db8` worktree):

- `records/imx8mp/pr133-066a2db8/20261012-1905-c206-066a2db8-1-0f082a54/` (started 19:05:59, ended 19:06:47)
- `records/imx8mp/pr133-066a2db8/20261012-1907-gt-066a2db8-1-c30d8bd6/` (started 19:07:32, ended 19:09:14)

## Outcome

| item | value |
|---|---|
| case 206 | pass (`status 1`, `verdict=done`), record `594b010e,00ceff01,00000006,00000000` |
| set | passed 48 of 49 run; skipped 1 (case 16); failing 1: case 44, record `594b010e,002c0107,ffff0030,00100031` |
| re-runs | none (44 is the known #6; neither 2 nor 10 failed) |
| boots | 2 (`boot_verdict=cycled`, `resets=absent` both) |
| recoveries | 0 |

## Comparison with the baseline

The baseline (`imx8mp-pr133-baseline.md`, main `63efed8e`, without PR #133) gave passed 48 of 49 run,
skipped 1 (case 16), failing 1: case 44, record `594b010e,002c0107,ffff0030,00100031`. On `066a2db8` (PR
#133 merged with main `2fe4b879`) the set's verdict line and record words are the same. The PR changes
nothing in the GC7000 regression set. Cases 2 and 10 (#25, #9) passed in this boot, which says nothing
about their rates.

The record does not hold case 206's per-check colours, only pass. Its commit message says the GC7000
wants each level's own colour for both the 3D texture and the 2D array (L0..L2). A pass therefore means
the GC7000 sampled levels 1 and 2 of both. There is no on-board control on main, because case 206 does not
exist there. The control is the host-mock one in the case commit.
