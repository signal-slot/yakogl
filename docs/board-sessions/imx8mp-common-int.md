<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP common set from the integration image (v3d-hh-gate)

Status: **the rows below were run on the board at integration commit
`f013dbbb36c740023a6e43941835bbf20eda1c09`, by an earlier task (01a128d0). The
re-run on the current integration tip `39bb67dc527891b2a40b8e461a82a468405bfa9a`
(121 commits later) is not run.** At f013dbb: GM, SO, SV and CL pass; GT fails
at case 16 exactly as the origin/main baseline did. Nothing regressed and
nothing was fixed in the record. Board: Verdin i.MX8M Plus (GC7000UL),
`torizon@verdin-imx8mp`, kernel `6.6.142-7.7.0-Torizon`, boot `uboot-mmc`.
Date of those runs: 2026-10-11 (12:01-12:21 local). The board answered ssh
again on 2026-10-12 (kernel 6.6.142-7.7.0).

## Not run on 39bb67d: missing inputs

The session that wrote this version could not build or run, because its shell
refused these steps (no approval is possible in an unattended run):

- the build and run scripts: `tools/cts/cts-common.sh imx8mp build ...` (also
  with `--dry-run`), a wrapper script of the exports below, and `chmod +x` on it
  were each refused, so no image was built and the board was not booted.
  A worktree of 39bb67d was made at `$D/wt-int-39bb67d` (allowed after the
  first attempt); the workspace itself is main `40dd210`, which has none of
  `docs/board-sessions/`, `docs/benchmarks-baseline/` or the merged fixes;
- `AGL_SLINT_CLUSTER_DIR`: `git clone https://github.com/signal-slot/agl-slint-cluster`
  and a `curl` of its tarball (docs/release/building.md input 9) were refused,
  and the deps cache has no clone, so the CL row cannot build;
- `NOTO_SANS_DIR`: `fc-list` lists Noto fonts under
  `/usr/share/fonts/noto` (the script's default); I did not check that the
  three exact file names (Regular, Light, ExtraLight) are there.

Present and usable in the deps cache: `zephyr-ws` (with `.venv/bin/west`),
`zephyr-sdk-1.0.1`, `glmark2-src`, `slint-nostd-src` (with
`internal/renderers/femtovg/Cargo.toml`), `yakogl-nostd/nostd` (femtovg, glow).
The commands in the next section, run from a checkout of 39bb67d with the
cluster clone and fonts added, produce the verdict lines for that commit;
nothing below is a result for 39bb67d. The tables and verdict lines below are
the f013dbb run, copied from the earlier task unchanged.

## What was built

- Integration tip: `f013dbbb36c740023a6e43941835bbf20eda1c09`. The worktree was
  that commit plus one commit of this item that only added this document
  (`PVR_SAMPLE_TREE=4a2a082` in the build ids is that commit; no code differs).
- Zephyr `v4.4.0-9848-gaf17c0c8aa2a`, board `verdin_imx8mp/mimx8ml8/a53`,
  nightly Rust. `common-images.sh` added `CONFIG_GC_BOARD_WDOG_SELF_ARM=y` to every
  row itself (all five rows rc=0, none `no_marker`).

| Row | sha256 of zephyr.bin (first 16) | build ids |
|-----|------|-----------|
| GM | `cc75446b5e783f6d` | PVR_SAMPLE_TABLE=0db3922e, PVR_SAMPLE_TREE=4a2a082 |
| SO | `cdff9e62cb9e0cc9` | same |
| SV | `f8df4bd2e274f319` | same |
| CL | `9af12dd1d6e64a60` | same |
| GT | `328fce8ae8f40c49` | GC_BUILD_STAMP=2026-10-11T03:00:16Z |

## Commands

```sh
D=/home/tasuku/.local/share/ccfarm/deps/yakogl
export ZEPHYR_WS=$D/zephyr-ws WEST_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export GLMARK2_SRC=$D/glmark2-src SLINT_NOSTD_DIR=$D/slint-nostd-src NOSTD_PATCH_DIR=$D/yakogl-nostd/nostd
export AGL_SLINT_CLUSTER_DIR=$D/agl-slint-cluster NOTO_SANS_DIR=$D/noto-sans
export RUSTUP_TOOLCHAIN=nightly
export BOARD_HOST=torizon@verdin-imx8mp BOARD_BOOT=uboot-mmc

IMG=$D/scratch/int-img OUT=$D/scratch/int-out
tools/cts/cts-common.sh imx8mp build $IMG
tools/cts/cts-common.sh imx8mp run   $IMG $OUT
tools/cts/cts-common.sh imx8mp tally $OUT
```

(`run` itself ends with the tally; `tally` was run again and printed the same.)
The i.MX8MP has no console capture: the result is the SNVS record, so there
are no per-scene glmark2 lines, no cluster fps, and GT names only the first
failing case.

## Verdict lines (board.sh, verbatim)

```
board.sh imx8mp GM rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:GLMARK2 resets=absent record=594b120e,fc3f002a,2184003f,03200258 back_after=397s unsupported=0 uptime=29s
board.sh imx8mp SO rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=113s uptime=21s
board.sh imx8mp SV rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=151s uptime=27s
board.sh imx8mp CL rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,10c410c4,1905f7bf,b1076d5f back_after=214s uptime=24s
board.sh imx8mp GT rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=112s uptime=26s
```

## Tally (common-tally.py, verbatim)

```
common-tally: build 4a2a082
common-tally: imx8mp, 5 rows from .../scratch/int-out
GM GM rc=0 verdict=done result=record:GLMARK2 record=594b120e,fc3f002a,2184003f,03200258; resets absent; image=cc75446b5e783f6d build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=4a2a082
    glmark2 score 42; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none
SO SO rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets absent; image=cdff9e62cb9e0cc9 build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=4a2a082
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
SV SV rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets absent; image=f8df4bd2e274f319 build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=4a2a082
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
CL CL rc=0 verdict=done result=record:PROBES record=594b0601,10c410c4,1905f7bf,b1076d5f; resets absent; image=9af12dd1d6e64a60 build=PVR_SAMPLE_TABLE=0db3922e,PVR_SAMPLE_TREE=4a2a082
    cluster probes pass (status pass p0 10c4 p1 10c4 p2 f7bf p3 1905 p4 6d5f crc16 b107 (RGB565)); throughput absent (no console records)
GT GT rc=4 verdict=failed result=record:CASE record=594b010e,0010ff06,0000002e,00000030; resets absent; image=328fce8ae8f40c49 build=GC_BUILD_STAMP=2026-10-11T03:00:16Z
    gl_test passed 46 of 48 (CASE record); failing: 16, then unknown (the record carries the first failure only)
common-tally: rows 5, with a run 5, without 0; verdict done 4, other 1
```

Row-by-row:

- **GM**: score 42, 33 scenes ran, 0 failed, 0 unsupported, 800x600 on-screen.
  The tally reports no per-scene lines on this board (no console).
- **SO / SV**: identical probe record, `crc16 c592`; the scan-out view matches
  the copy path.
- **CL**: probes pass, `crc16 b107`; throughput absent.
- **GT**: `CASE seq 14: case 16 check 255 status 6 (set failed) got 0x0000002e
  want 0x00000030`; 46 of 48 passed. Cases 10, 2 and 171: the record carries
  only the first failure, which is case 16, so the tally cannot say whether
  10, 2 or 171 passed in this run. 171 (viewwin-bgra) is not the reported
  failure; the second failing case is "unknown", as in the baseline. Cases 10
  and 2 fail at random (`docs/board-sessions/imx8mp-gc-case10.md`), which a
  single boot cannot settle.

## Comparison with docs/benchmarks-baseline/imx8mp.md (origin/main `8abeaabb`)

| Row | Baseline | Integration | Differs |
|-----|----------|-------------|---------|
| GM | score 42, 33 ran, 0 failed, 0 unsupported | same | no |
| SO | pass, `crc16 c592` | same | no |
| SV | pass, `crc16 c592` | same | no |
| CL | pass, `crc16 b107` | same | no |
| GT | failed 16, 46 of 48, got 0x2e want 0x30 | same | no |

Regressions: none. Fixes visible in the record: none. Every record word of
every row is identical to the baseline; only `back_after` and `uptime` differ.
The merged npot, pvr-npot-mips and viewwin-bgra changes are therefore not
visible on this board through the record: glmark2's 42 and 33 scenes are
unchanged, and GT's case-16 failure (got 0x2e, want 0x30, set failed) is
still open and is the next thing to investigate for the GT row.
