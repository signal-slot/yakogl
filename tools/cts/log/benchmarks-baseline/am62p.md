<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (plan #2, `baseline3-am62p`, third attempt): 5 of 5 rows done

Status: **all five common-set rows (GM, SO, SV, CL, GT) built from origin/main
and ran on the board; every row ended `verdict=done`.** One qualification: the
first CL run (`20261013-0041`) captured no console (`capture: cannot reach
usb-serial-1:35933: timed out`, `verdict=scan_failed`, `rc=4`, board back after
424 s); CL was re-run alone and is the row tallied below.

- origin/main: `f87417993258d88e5cd88cdffbce67547c1bd3f0` (after `git fetch origin`).
- Board: Verdin AM62P, ssh `torizon@verdin-am62p`, boot `uboot-mmc`
  (one-shot `yako_once`), console bridge `BOARD_UART=usb-serial-1:35933`
  (main_uart0, per tools/cts/log/investigations/console-map.md; the ccfarm
  resource record lists 35932, which the notes say is the silent mcu_uart0).
- Date 2026-10-13. `<deps>` = the worker's dependency cache, `$CCFARM_DEPS_DIR`.
- Records: `<deps>/b3-rec/<LABEL>/<run>/`.

## Commands

```
git worktree add --detach <deps>/b3-wt f87417993258d88e5cd88cdffbce67547c1bd3f0
cp <deps>/pvr_fw_blob.h <deps>/b3-wt/driver/src/fw/pvr_fw_blob.h        # not in git
tools/cts/common-images.sh --dry-run am62p tools/cts/common-tree.txt <deps>/b3-img
tools/cts/common-images.sh am62p tools/cts/common-tree.txt <deps>/b3-img
  # env: ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR, RUSTUP_TOOLCHAIN=nightly,
  #      GLMARK2_SRC (22c527cb), SLINT_NOSTD_DIR=<deps>/slint-nostd/slint,
  #      NOSTD_PATCH_DIR=<deps>/nostd, AGL_SLINT_CLUSTER_DIR=<deps>/agl-slint-cluster,
  #      NOTO_SANS_DIR=<deps>/noto-sans

export BOARD_HOST=torizon@verdin-am62p BOARD_UART=usb-serial-1:35933 BOARD_BOOT=uboot-mmc
tools/board.sh --dry-run am62p run <deps>/b3-img/<L>/zephyr/zephyr.bin <deps>/b3-rec/<L> <L>-f8741799
tools/board.sh am62p run <deps>/b3-img/<L>/zephyr/zephyr.bin <deps>/b3-rec/<L> <L>-f8741799   # L = GM SO SV CL GT
python3 tools/cts/common-tally.py am62p --row GM=<rundir> --row SO=<rundir> --row SV=<rundir> --row CL=<rundir> --row GT=<rundir>
```

`common-tree.txt` on origin/main already has the CL row, so `common-images.sh`
built the cluster sample; no hand build was needed.

Build result (verbatim):

```
common-images: build f8741799 (f87417993258d88e5cd88cdffbce67547c1bd3f0) board am62p
GM rc=0 sha256=4c32b1de5410531e6a60e95497aba0165b10d15a332565d5310d26d344f28d49 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
SO rc=0 sha256=f8d79627fda1015d76d38d48a8fcb9843680eedce5968975256d56264a4b0cd1 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
SV rc=0 sha256=f8d79627fda1015d76d38d48a8fcb9843680eedce5968975256d56264a4b0cd1 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
CL rc=0 sha256=5fbe624aa2aa45ac28a0aaed2432ae5e1614f9ad75cca74b66962fd1e3b31b02 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
GT rc=0 sha256=a60b089a53522177eb17ef6a1141ee1b256e1b8d711d66c7214615ebae176a8c build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
common-images: 0 row(s) not built
```

## Verdict lines (verbatim)

```
board.sh am62p GM-f8741799 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0 record=594b1201,fc3f01d1,6184003f,0f258320 back_after=414s unsupported=0
board.sh am62p SO-f8741799 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210862,312b0883 back_after=137s
board.sh am62p SV-f8741799 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210862,312b0883 back_after=124s
board.sh am62p CL-f8741799 rc=4 verdict=scan_failed boot=uboot-mmc row boot_verdict=scan_failed result=none resets=absent record=absent back_after=424s     (first CL run, console bridge timed out)
board.sh am62p CL-f8741799 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,008510c4,19050085,110c6d5f back_after=179s     (CL re-run)
board.sh am62p GT-f8741799 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=102s
```

Tally (verbatim):

```
common-tally: build f8741799
common-tally: am62p, 5 rows from --row
GM GM-f8741799 rc=0 verdict=done result=record:GLMARK2 record=594b1201,fc3f01d1,6184003f,0f258320; resets (console count) 0; image=4c32b1de5410531e build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
    glmark2 score 465; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none; GL errors raised 15 or more, draws that sampled an incomplete texture 0
    run <deps>/b3-rec/GM/20261013-0028-GM-f8741799-4c32b1de
SO SO-f8741799 rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210862,312b0883; resets (console count) 0; image=f8d79627fda1015d build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
    slint probes pass (status pass p0 0862 p1 0842 p2 0862 p3 0021 p4 0883 crc16 312b (RGB565))
    run <deps>/b3-rec/SO/20261013-0036-SO-f8741799-f8d79627
SV SV-f8741799 rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210862,312b0883; resets (console count) 0; image=f8d79627fda1015d build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
    slint probes pass (status pass p0 0862 p1 0842 p2 0862 p3 0021 p4 0883 crc16 312b (RGB565))
    run <deps>/b3-rec/SV/20261013-0039-SV-f8741799-f8d79627
CL CL-f8741799 rc=0 verdict=done result=record:PROBES record=594b0601,008510c4,19050085,110c6d5f; resets (console count) 0; image=5fbe624aa2aa45ac build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
    cluster probes pass (status pass p0 10c4 p1 0085 p2 0085 p3 1905 p4 6d5f crc16 110c (RGB565)); throughput: 1024x600: 4390 frames in 60015 ms, 73.1 fps, 44.9 Mpx/s; draw 13.5 ms mean, 21.97 ms/Mpx
    run <deps>/b3-rec/CL/20261013-0051-CL-f8741799-5fbe624a
GT GT-f8741799 rc=0 verdict=done result=legacy:gl_test_all record=absent; resets (console count) 0; image=a60b089a53522177 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
    gl_test passed 50 of 50 (0 failed, 0 timed out); failing: none
    run <deps>/b3-rec/GT/20261013-0049-GT-f8741799-a60b089a
common-tally: rows 5, with a run 5, without 0; verdict done 5, other 0
```

## Per benchmark

| Row | ran | scenes / result | GL errors | memory / pool messages |
|-----|-----|-----------------|-----------|------------------------|
| GM glmark2 | yes | score 465; 33 scenes ran, 0 failed, 0 unsupported; 800x600 on-screen; 0 draws that sampled an incomplete texture | `glmark2: GL errors raised 2582 ... (GL errors recorded 2)`; 15 `GL error` lines after the first pool exhaustion: `0x505 glGenerateMipmap` ×1, then ×5 each of `0x506 current_drawable_program`, `0x502 glGenerateMipmap`, `0x506 resolve_target`; plus 7 `yakogl: program N has no uniform ...` notes (program 2: `ModelViewMatrix`, `NormalMap` ×2, `HeightMap` ×2; program 8: `fogDensity`, `fogNear`, `fogFar`, `fogColor`) | **two `render pool exhausted` errors**: `5464 KiB requested, 3648 KiB left` (00:32:57, entry 22) and `21848 KiB requested, 2744 KiB left` (00:33:20, entry 24); pool 59200 KiB, high-water bump 21588 KiB, top 36760 KiB; `FWCCB: FREELIST_GROW` ×3; `glmark2-mem` per entry, last: `pool handed out 28246016, free in the lists 1335296, live 26910720 bytes; frees held 4 (96 KiB)` |
| SO slint | yes | `slint: check PASS (tolerance 1 per channel)`, 544 frames in 10009 ms, `slint: PASS`; GL_RENDERER PowerVR BXS-4-64 (YakoGL), OpenGL ES 2.0 YakoGL | none | none printed |
| SV slint-view | yes | same records as SO (byte-identical image: the AM62P ignores the scan-out view option) | none | none printed |
| CL cluster | yes (2nd run) | `cluster: check: PASS`, `cluster: PASS`; 4390 frames in 60015 ms, 73.1 fps, 44.9 Mpx/s; draw 13.5 ms mean (worst 27.1) | none | `cluster: mem stage 7: ui heap 456672/5731216/50331648 malloc 9216/18288/1048576 keep 562432/562432/1048576 gpu carve-out not measured; window 4915200 bytes`; no pool errors |
| GT gl_test | yes | `gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 208`; `gl_test: OK (rc=0)` | `GL error 0x501 in glCompressedTexImage2D`, `GL error 0x500 in glCompressedTexImage2D` (the compressed-texture cases; the cases passed) | none printed |

## Notes

- Compared with the earlier attempt at `3bcaa3e1` (branch
  `ccfarm/01a12c50-e6f3-7300-9b79-16cd32374b7d`): glmark2 score 465 and
  33/33 scenes are unchanged, but this run shows two `render pool exhausted`
  errors in the glmark2 run (entries 22 and 24) that the earlier note did not
  record, and the glmark2 record's last word differs (`0f258320`, earlier
  `03200258`). The cluster throughput is 73.1 fps (earlier 75.1).
- The first CL attempt's failure was the console capture, not the board: the
  image booted and Linux returned (`back_after=424s`); only the bridge
  connection timed out. The re-run went through on the same port.
- The GM run ends with `board: holding 20 s, then the reset`; the lines after
  are the default Linux boot, not sample output.
- Inputs: Zephyr workspace `af17c0c8aa2a` + SDK 1.0.1, slint (`slint-nostd/slint`)
  with the `nostd` femtovg/glow forks, `agl-slint-cluster` (branch `main`),
  glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`, Noto Sans Regular/Light/ExtraLight,
  nightly Rust with `rust-src`.
