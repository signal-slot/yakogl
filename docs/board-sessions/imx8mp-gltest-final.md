<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# i.MX8MP board session: gl_test default list from origin/main (imx8mp-gltest-final)

Plan #143 item `imx8mp-gltest-final`. Date: 2026-10-14. Board: Verdin i.MX8M Plus
(GC7000), the leased `verdin-imx8mp` resource, `BOARD_BOOT=uboot-mmc`.

## Summary

**Blocked: the board was not reachable from the worker, so no board run happened and
there are no results.** Nothing below is a measurement.

| what | state |
|---|---|
| origin/main | `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328` (`cd738b1a`), the tip of the workspace on 2026-10-14 |
| Build | GT image built from `cd738b1a` by an earlier worker (see "Build"); not rebuilt here |
| Board run | not run: `ssh torizon@verdin-imx8mp` did not resolve (see "Blocker") |
| `board.sh ... verdict=` lines | none: `tools/board.sh imx8mp` was not started |
| Case #8 (gl_test case 16, msaa) | not run |
| Case #9 (gl_test case 10) | not run |
| Case #25 (gl_test case 2) | not run |
| Per-case pass/fail of the default list | not run; no failures observed, none can be claimed |

## Blocker

`ssh torizon@verdin-imx8mp -- uname -a` failed with
`ssh: Could not resolve hostname verdin-imx8mp: Name or service not known`.
`mcp__ccfarm__ask_farm` answered that ccfarm's access records list the board as
reachable from the nodes Lepton and nuc and as unreachable from this node (hx90); no
other route is recorded for hx90. A second, earlier worker of this item did reach the
board and built the image, but was refused `BOARD_HOST=... BOARD_BOOT=... tools/board.sh`
as a command prefix, so it did not boot it either. Rerun this item on a node that reaches
the board and may start `tools/board.sh`.

## Build

The earlier worker built the default-list image from origin/main `cd738b1a` with
`tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt <IMX8MP_OUT>/gtfinal-cd738b1a GT`:

```
common-images: build cd738b1a (cd738b1ae7cf99166a6bdd86cf24dac04c4c4328) board imx8mp
GT rc=0 sha256=c04fd86a118e3cf8bb815838d6ef47d106043e12286d8a501f059bc69b25f2de build=GC_BUILD_STAMP=2026-10-13T19:06:47Z
common-images: 0 row(s) not built
```

That image lives on the earlier worker's dependency cache, not on this node (this node
has only `gtfinal-dc514010`, which is not origin/main).

## Commands for the rerun (not run)

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $IMX8MP_OUT/gtfinal-cd738b1a GT   # default list
BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run $IMX8MP_OUT/gtfinal-cd738b1a/GT/zephyr/zephyr.bin <rec> gt-cd738b1a-1
# case 16 (#8) is skipped in the default list; alone, as in imx8mp-gltest-r2:
tools/cts/imx8mp-gltest-build.sh msaa-cd738b1a 0 -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc SEL=16 tools/board.sh imx8mp run $IMX8MP_OUT/msaa-cd738b1a/zephyr/zephyr.bin <rec> msaa-cd738b1a
```

Cases #9 (10) and #25 (2) are in the default list; their flake rate (about 4 of 32
boots for case 10 in the last measurement) means one boot says little. History:
`docs/investigations/gc-kick-diff.md`, `gc-msaa-record.md`. The reference for the
default list is `imx8mp-pr133-baseline.md`: 48 of 49 run, case 16 skipped, case 44 (#6)
failing, on `63efed8e`.

## Verdict lines

None. No `tools/board.sh imx8mp` command was started in this session.
