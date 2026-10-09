<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP benchmark baseline (before plan #2)

Status: **all five common-set rows built and ran on the board.** GM, SO, SV and
CL reach a passing verdict; GT fails (case 16). Board: Verdin i.MX8M Plus
(GC7000UL), `$BOARD_HOST`, kernel `6.6.142-7.7.0-Torizon`.
Date: 2026-10-08 (runs 07:50-08:07 local).

## What was built

- GM, SO, SV, GT: **origin/main `8abeaabb`** (`8abeaabb0807617fa90c3d72b26ad4e70dd5ff94`),
  in a clean detached worktree, with `tools/cts/common-images.sh imx8mp
  tools/cts/common-tree.txt <img-dir>` (build ids `PVR_SAMPLE_TREE=8abeaabb`).
- CL: origin/main has **no CL row and no cluster case** in `common-images.sh`,
  so CL was built with the integration tip's script and sample (`98ab5fae` plus
  the first, doc-only commit of this branch, `0f608491`; `PVR_SAMPLE_TREE=0f608491`).
- Zephyr `v4.4.0-9848-gaf17c0c8aa2a`, board `verdin_imx8mp/mimx8ml8/a53`,
  nightly Rust (`-Z build-std`; `rust-src` had to be added to the nightly toolchain
  after SO/SV first failed with `.../rustlib/src/rust/library/Cargo.lock does not exist`).
- Inputs: glmark2 `22c527cb`, Slint `1007c032` + `slint-nostd.patch`, femtovg 0.26.0
  and glow 0.18.0 from the cargo registry + patches, signal-slot/agl-slint-cluster
  (default branch), Noto Sans Regular/Light/ExtraLight from notofonts.github.io.

| Row | sha256 of zephyr.bin (first 16) | build ids |
|-----|------|-----------|
| GM | `69c1af4854318974` | PVR_SAMPLE_TABLE=0db3922e, PVR_SAMPLE_TREE=8abeaabb |
| SO | `609cb24023c811b6` | same |
| SV | `95361f76689b9c27` | same |
| CL | `eb9424588367d3b3` | PVR_SAMPLE_TABLE=c95547a1, PVR_SAMPLE_TREE=0f608491 |
| GT | `1f4aac5b035f3dc7` | GC_BUILD_STAMP=2026-10-07T22:31:37Z |

## Run

`BOARD_BOOT=uboot-mmc tools/cts/cts-common.sh
imx8mp run <img-dir> <outdir>` (`board.sh tree`, then `common-tally.py`; the
mmc route, because no TFTP server was set up). The i.MX8MP has **no console
capture**: the result is the SNVS record, so there is no console text, no
per-scene glmark2 scores and no cluster fps; only the final glmark2 score.

## Verdict lines (board.sh)

```
board.sh imx8mp GM rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:GLMARK2 resets=absent record=594b120e,fc3f002a,2184003f,03200258 back_after=397s unsupported=0 uptime=31s
board.sh imx8mp SO rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=99s uptime=21s
board.sh imx8mp SV rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,08420862,00210842,c5920883 back_after=105s uptime=26s
board.sh imx8mp CL rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PROBES resets=absent record=594b0601,10c410c4,1905f7bf,b1076d5f back_after=152s uptime=22s
board.sh imx8mp GT rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=65s uptime=27s
```

## Tally (common-tally.py)

```
GM GM rc=0 verdict=done result=record:GLMARK2 ...; resets absent; image=69c1af4854318974
    glmark2 score 42; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none
SO SO rc=0 verdict=done result=record:PROBES ...; resets absent
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
SV SV rc=0 verdict=done result=record:PROBES ...; resets absent
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
CL CL rc=0 verdict=done result=record:PROBES ...; resets absent
    cluster probes pass (status pass p0 10c4 p1 10c4 p2 f7bf p3 1905 p4 6d5f crc16 b107 (RGB565)); throughput absent (no console records)
GT GT rc=4 verdict=failed result=record:CASE ...; resets absent
    gl_test passed 46 of 48 (CASE record); failing: 16, then unknown (the record carries the first failure only)
common-tally: rows 5, with a run 5, without 0; verdict done 4, other 1
```

Reading:

- **GM**: glmark2 score **42**, 33 scenes ran, 0 failed, 0 unsupported, 800x600
  on-screen. Per-scene scores: not available (no console on this board).
- **SO / SV**: identical probe record (`crc16 c592`); the scan-out view (SV)
  matches the copy path.
- **CL**: probes pass; fps/Mpx/s not reported (no console records).
- **GT**: fails. `CASE seq 14: case 16 check 255 status 6 (set failed) got
  0x0000002e want 0x00000030`; 46 of 48 passed, 16 and one more unknown case
  failed. No GPU reset. Not investigated here (HANDOFF notes the i.MX8MP's
  gl_test random failures, cases 2 and 10; case 16 is a different one).

Run directories (not committed) were under the deps cache
`scratch/out1/` (`20261008-0750-GM-69c1af48`, `-0758-SO-609cb240`,
`-0800-SV-95361f76`, `-0802-CL-eb942458`, `-0805-GT-1f4aac5b`).
