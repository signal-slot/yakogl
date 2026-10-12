<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (plan #2, `baseline3-am62p`, third attempt): built, not run

Status: **all five common-set images built from origin/main; none was run on
the board, so there are no verdict lines.** Reason: the console bridge
(`BOARD_UART=host:port`, resource `usb-serial-1`) was not available to this
task, and `tools/board.sh am62p run` stops without it (`board.sh`:
`: "${BOARD_UART:?set BOARD_UART (or UART_ADDR)=host:port of the serial bridge}"`).

- origin/main: `f87417993258d88e5cd88cdffbce67547c1bd3f0` (after `git fetch origin`).
- Board: Toradex Verdin AM62P, resource `verdin-am62p`; `ssh torizon@verdin-am62p -- true`
  answered (login works). The task's lease listed only `verdin-am62p`; the
  serial resource `usb-serial-1` was not leased, `BOARD_UART` was unset in the
  environment, and `ask_farm` for its host:port was denied. The repository
  records the port only as `<tailnet address of usb-serial-1>:35933`
  (tools/cts/log/investigations/console-map.md) with no host, and a guessed
  address was not used.

## Commands

```
git worktree add --detach $CCFARM_DEPS_DIR/b3-wt f87417993258d88e5cd88cdffbce67547c1bd3f0
cp $CCFARM_DEPS_DIR/pvr_fw_blob.h b3-wt/driver/src/fw/pvr_fw_blob.h   # not in git
bash b3-wt/tools/cts/common-images.sh --dry-run am62p b3-wt/tools/cts/common-tree.txt $CCFARM_DEPS_DIR/b3-img
bash b3-wt/tools/cts/common-images.sh am62p b3-wt/tools/cts/common-tree.txt $CCFARM_DEPS_DIR/b3-img
# env: ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR from the farm; RUSTUP_TOOLCHAIN=nightly;
#      GLMARK2_SRC (22c527cb), SLINT_NOSTD_DIR=<deps>/slint-nostd/slint, NOSTD_PATCH_DIR=<deps>/nostd,
#      AGL_SLINT_CLUSTER_DIR=<deps>/agl-slint-cluster, NOTO_SANS_DIR=<deps>/noto-sans
```

common-tree.txt on origin/main already has the CL (cluster) row, so
`common-images.sh` built it; no hand build was needed.

## Build result (verbatim)

```
common-images: build f8741799 (f87417993258d88e5cd88cdffbce67547c1bd3f0) board am62p
GM rc=0 sha256=4c32b1de5410531e6a60e95497aba0165b10d15a332565d5310d26d344f28d49 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
SO rc=0 sha256=f8d79627fda1015d76d38d48a8fcb9843680eedce5968975256d56264a4b0cd1 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
SV rc=0 sha256=f8d79627fda1015d76d38d48a8fcb9843680eedce5968975256d56264a4b0cd1 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
CL rc=0 sha256=5fbe624aa2aa45ac28a0aaed2432ae5e1614f9ad75cca74b66962fd1e3b31b02 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
GT rc=0 sha256=a60b089a53522177eb17ef6a1141ee1b256e1b8d711d66c7214615ebae176a8c build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=f8741799
common-images: 0 row(s) not built
```

SO and SV are byte-identical on the AM62P (the scan-out view option is
ignored there), as common-tree.txt says.

## Per benchmark

| Row | built | ran | scenes passed | GL errors | memory/pool messages |
|-----|-------|-----|---------------|-----------|----------------------|
| GM glmark2 | yes | no: no `BOARD_UART` | n/a | n/a | n/a |
| SO slint | yes | no: no `BOARD_UART` | n/a | n/a | n/a |
| SV slint-view | yes | no: no `BOARD_UART` | n/a | n/a | n/a |
| CL cluster | yes | no: no `BOARD_UART` | n/a | n/a | n/a |
| GT gl_test | yes | no: no `BOARD_UART` | n/a | n/a | n/a |

## To finish

With the images above (kept under `$CCFARM_DEPS_DIR/b3-img/<LABEL>/zephyr/zephyr.bin`),
on a task that leases `usb-serial-1`:

```
BOARD_HOST=torizon@verdin-am62p BOARD_UART=<bridge>:<port> BOARD_BOOT=uboot-mmc \
  tools/board.sh am62p run <img>/<L>/zephyr/zephyr.bin <rec>/<L> <L>-f8741799   # L in GM SO SV CL GT
python3 tools/cts/common-tally.py am62p --row GM=<rundir> ... --row GT=<rundir>
```

The earlier attempt (origin/main `3bcaa3e1`, branch
`ccfarm/01a12c50-e6f3-7300-9b79-16cd32374b7d`) ran all five rows with
`verdict=done`; that is not a measurement of `f8741799`.
