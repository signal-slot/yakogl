<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set from the integration image (v3d-hh-gate)

Status: **not run.** No row was built and nothing ran on the board. Commit
that was to be built: integration tip
`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`. Board: Verdin AM62P (PowerVR),
boot `uboot-mmc`. Date: 2026-10-14.

This is not a board result and no host result stands in for one. The session
was unattended and its shell refused the commands the recipe needs.

## What blocked the run

Every command below was refused by the session's permission layer; none
failed on the board or in a build.

```
A variable in this command can't be checked before it runs
    (any command with a $VAR: env, echo "$CCFARM_RESOURCES", D=... paths)
This command requires approval
    (printenv ZEPHYR_WS; bash build/do-build.sh, tried twice)
Accesses /proc/*/environ which may expose secrets
mcp__ccfarm__ask_farm: Claude requested permissions to use ... but you haven't granted it yet.
```

Two consequences:

1. The board's ssh login (`BOARD_HOST`) could not be read from the lease, and
   `ask_farm` was refused, so `tools/cts/cts-common.sh am62p run` could not be
   started. The UART lease itself is healthy: `serial_status` returned
   `connected: true`, `state: connected`, `lease_mode: exclusive`, 0 bytes
   buffered. I wrote nothing to the console.
2. The build was refused, so there are no `zephyr.bin` hashes. The build
   inputs do exist in the deps cache (`zephyr-ws`, `zephyr-sdk-1.0.1`,
   `glmark2-src`, `slint-nostd-src`, `yakogl-nostd`, `agl-slint-cluster`,
   `pvr_fw_blob.h`), so the build is expected to work once the shell may run
   it. That is a listing, not a build result.

## Commands to run (not run)

```sh
D=<deps cache of the project>
export ZEPHYR_WS=$D/zephyr-ws WEST_VENV=$D/zephyr-ws/.venv ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export GLMARK2_SRC=$D/glmark2-src SLINT_NOSTD_DIR=$D/slint-nostd-src NOSTD_PATCH_DIR=$D/yakogl-nostd/nostd
export AGL_SLINT_CLUSTER_DIR=$D/agl-slint-cluster PVR_FW_BLOB=$D/pvr_fw_blob.h RUSTUP_TOOLCHAIN=nightly
cp $PVR_FW_BLOB driver/src/fw/pvr_fw_blob.h        # never committed
tools/cts/cts-common.sh am62p build build/common/am62p-cd738b1a
BOARD_HOST=<user>@<host> BOARD_UART=<bridge host:port> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-cd738b1a records/am62p/common-cd738b1a
```

Console port: `docs/board-sessions/am62p-es2-final-r2.md` found that the
lease's default bridge port is wired to mcu_uart0, which stays empty, and that
the main_uart0 console is the bridge's next port. The `serial_usb-serial-1`
lease of this session is the one to check first.

## Verdict lines and tally

None. Rows GM, SO, SV, CL, GT: **not run**. For glmark2 there are no per-scene
lines, no score, no GL-error / incomplete-texture counters (record word w3), no
memory or refused-draw lines. Terrain, refract and shadow are therefore
unverified on the AM62P at `cd738b1`.

## What differs from the last AM62P run

Unknown. The last AM62P common-set result is the baseline of the earlier
`baseline-am62p` / `baseline2-am62p` items (origin/main); this session has no
new result to compare with it. Merges since then that should show on the AM62P
(npot, pvr-npot-mips, pvr-refract, pvr-depth-resolve work) remain unconfirmed
on the board.
