<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP common set from the integration image (v3d-hh-gate)

Status: **GM, SO, SV and GT ran on the board from integration tip
`39bb67dc527891b2a40b8e461a82a468405bfa9a`; CL did not run** (its inputs could
not be fetched, below). GM, SO and SV pass; GT fails at case 16, as the
origin/main baseline did. Board: Verdin i.MX8M Plus (GC7000UL),
`torizon@verdin-imx8mp`, boot `uboot-mmc`. Date: 2026-10-12 (23:27-23:42 local).
CL's last result is from an earlier task's run at integration `f013dbbb36c7`
(2026-10-11), kept at the end; it is not a result for `39bb67d`.

## What was built

Zephyr `v4.4.0-9848-gaf17c0c8aa2a`, board `verdin_imx8mp/mimx8ml8/a53`, nightly
Rust, from a worktree of `39bb67d`. `common-images.sh` added
`CONFIG_GC_BOARD_WDOG_SELF_ARM=y` itself (all four rows rc=0).

| Row | sha256 of zephyr.bin (first 16) | build ids |
|-----|------|-----------|
| GM | `f8a83289233c0864` | PVR_SAMPLE_TABLE=0db3922e, PVR_SAMPLE_TREE=39bb67d |
| SO | `2f68bcb947a409cc` | same |
| SV | `f12909fc33cd2fed` | same |
| CL | not built | needs `AGL_SLINT_CLUSTER_DIR` |
| GT | `d4aaf0e962a29e06` | GC_BUILD_STAMP=2026-10-12T14:27:08Z |

## Commands

```sh
D=/home/tasuku/.local/share/ccfarm/deps/yakogl
git worktree add --detach $D/wt-int-39bb67d refs/ccfarm/origin/heads/ccfarm/integration   # 39bb67d
export ZEPHYR_WS=$D/zephyr-ws WEST_VENV=$D/zephyr-ws/.venv ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export GLMARK2_SRC=$D/glmark2-src SLINT_NOSTD_DIR=$D/slint-nostd-src NOSTD_PATCH_DIR=$D/yakogl-nostd/nostd
export RUSTUP_TOOLCHAIN=nightly
cd $D/wt-int-39bb67d

# what ran (build with the CL row left out):
tools/cts/cts-common.sh imx8mp build $D/scratch/int2-img GM SO SV GT
# the run, from a wrapper (the shell would not run cts-common.sh run), the same
# as cts-common.sh run minus CL: BOARD_HOST=torizon@verdin-imx8mp BOARD_BOOT=uboot-mmc
env -u TREE_PREV -u TREE_IMAGE TREE_IMAGE_DIR=$D/scratch/int2-img \
  tools/board.sh imx8mp tree $D/scratch/int2-table.txt $D/scratch/int2-out   # common-tree.txt without CL
python3 tools/cts/common-tally.py imx8mp $D/scratch/int2-out               # what `cts-common.sh tally` runs

# all five rows, once AGL_SLINT_CLUSTER_DIR (a clone of
# https://github.com/signal-slot/agl-slint-cluster) and NOTO_SANS_DIR are set:
tools/cts/cts-common.sh imx8mp build  $D/scratch/int-img
tools/cts/cts-common.sh imx8mp run    $D/scratch/int-img $D/scratch/int-out
tools/cts/cts-common.sh imx8mp tally  $D/scratch/int-out
```

The i.MX8MP has no console capture: the result is the SNVS record, so there are
no per-scene glmark2 lines, no cluster fps, and GT names only the first failing
case.

## Verdict lines (board.sh, verbatim)

```
board.sh imx8mp GM rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:GLMARK2 resets=absent record=594b120e,fc3f002b,2184003f,03200258 back_after=403s unsupported=0 uptime=28s
board.sh imx8mp SO rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=156s uptime=22s
board.sh imx8mp SV rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=155s uptime=29s
board.sh imx8mp GT rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=121s uptime=22s
```

## Tally (common-tally.py, verbatim)

```
common-tally: build 39bb67d
common-tally: imx8mp, 4 rows from .../scratch/int2-out
GM GM rc=0 verdict=done result=record:GLMARK2 record=594b120e,fc3f002b,2184003f,03200258; resets absent; image=f8a83289233c0864 build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=39bb67d
    glmark2 score 43; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none
SO SO rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets absent; image=2f68bcb947a409cc build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=39bb67d
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
SV SV rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets absent; image=f12909fc33cd2fed build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=39bb67d
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
GT GT rc=4 verdict=failed result=record:CASE record=594b010e,0010ff06,0000002e,00000030; resets absent; image=d4aaf0e962a29e06 build=GC_BUILD_STAMP=2026-10-12T14:27:08Z
    gl_test passed 46 of 48 (CASE record); failing: 16, then unknown (the record carries the first failure only)
common-tally: rows 4, with a run 4, without 0; verdict done 3, other 1
```

CL: no line. Not run on `39bb67d`: `git clone` and `curl` of
`signal-slot/agl-slint-cluster` were refused in this unattended session, and the
deps cache has no clone.

GT detail: `CASE seq 14: case 16 check 255 status 6 (set failed) got 0x0000002e
want 0x00000030`; 46 of 48 passed. Cases 10, 2 and 171: the record carries only
the first failure, case 16, so the tally cannot say whether 10, 2 or 171 passed.
171 (viewwin-bgra) is not the reported failure; the second failing case is
"unknown", as in the baseline. Cases 10 and 2 fail at random
(`docs/board-sessions/imx8mp-gc-case10.md`), which a single boot cannot settle.

## Comparison with docs/benchmarks-baseline/imx8mp.md (origin/main `8abeaabb`)

| Row | Baseline | Integration `39bb67d` | Differs |
|-----|----------|-------------|---------|
| GM | score 42, 33 ran, 0 failed, 0 unsupported, record `fc3f002a` | score 43, 33 ran, 0 failed, 0 unsupported, record `fc3f002b` | **yes: score 42 to 43** |
| SO | pass, `crc16 c592` | same | no |
| SV | pass, `crc16 c592` | same | no |
| CL | pass, `crc16 b107` | not run | unknown |
| GT | failed 16, 46 of 48, got 0x2e want 0x30 | same | no |

The baseline values are as the earlier task recorded them from
`docs/benchmarks-baseline/imx8mp.md`; that file is not in main, and this
session did not re-read it.

Regressions: none. Fixes: none in a pass/fail verdict. GM's score moved from 42
to 43 (one point; the earlier `f013dbb` run scored 42, so a point of run-to-run
noise is possible and I did not repeat it). GT's case-16 failure is unchanged
and is the open item for that row. The npot, pvr-npot-mips and viewwin-bgra
merges are not visible in this board's records beyond that.

## CL at integration f013dbb (earlier task 01a128d0, 2026-10-11), not re-run

```
board.sh imx8mp CL rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,10c410c4,1905f7bf,b1076d5f back_after=214s uptime=24s
    cluster probes pass (status pass p0 10c4 p1 10c4 p2 f7bf p3 1905 p4 6d5f crc16 b107 (RGB565)); throughput absent (no console records)
```

Same record words as the baseline's CL.
