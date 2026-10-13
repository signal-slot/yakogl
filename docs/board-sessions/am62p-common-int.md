<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set from the integration image (v3d-hh-gate)

Status: **partial. All five images built; nothing ran on the board.** Built from
the integration tip `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328` plus one docs-only
commit (this note, `d5cb878`; the build ids below say `d5cb878`). No file
under `src/` or `driver/` differs from `cd738b1`. Board: Verdin AM62P
(PowerVR), boot `uboot-mmc`. Date: 2026-10-14.

There is no board verdict, no tally and no glmark2 score. No host result stands
in for one.

## What was built

Zephyr from the shared workspace in the deps cache, board `verdin_am62p`, nightly
Rust, `tools/cts/cts-common.sh am62p build build/common/am62p-cd738b1a`
(`PVR_FW_BLOB` copied to `driver/src/fw/pvr_fw_blob.h`, ignored by git, not
committed). Printed by `common-images.sh`, verbatim:

```
GM rc=0 sha256=6fd665d87d69fda5bc1658f6bbfbe830a59e8803f74b53f4624d74385ce34cc3 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
SO rc=0 sha256=464a55f18093fd5d33c53a10880d7554f10777141f16a3636a323dcf1a9beda6 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
SV rc=0 sha256=464a55f18093fd5d33c53a10880d7554f10777141f16a3636a323dcf1a9beda6 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
CL rc=0 sha256=c62be4976254c501732b6d0f0556dca66cb1044fcbaddade255c52f8874deb2f build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
GT rc=0 sha256=e10577d16a56c6f46bd70d13775e8517440539e937ddfe1714d9517e1bf4031b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
common-images: 0 row(s) not built
```

| Row | sha256 of zephyr.bin (first 16) |
|-----|------|
| GM | `6fd665d87d69fda5` |
| SO | `464a55f18093fd5d` |
| SV | `464a55f18093fd5d` (same as SO: the AM62P board layer has no scan-out view, so SV is the copy path again, as common-tree.txt says) |
| CL | `c62be4976254c501` |
| GT | `e10577d16a56c6f4` |

The images were built, not booted. A build only shows that the sources
compile and link.

## What blocked the run

The run needs ssh to the board (`board.sh` checks `ssh ... sudo -n true`, stages
the image on the eMMC over ssh) and a UART bridge. The permission layer of this
unattended session did not grant ssh, and `bash build/do-run.sh` (a wrapper of
`cts-common.sh am62p run`) was refused twice. Exact lines:

```
This command requires approval
    ssh -o BatchMode=yes -o ConnectTimeout=20 torizon@verdin-am62p true
    bash build/do-run.sh          (twice)
```

I did not work around the refusal. The UART lease itself was healthy
(`serial_status`: `connected: true`, `lease_mode: exclusive`, 0 bytes buffered)
and I wrote nothing to the console.

Login and port were not in the lease (it only lists the serial bridge). They came
from `ask_farm`; three nodes answered, two sources agree on the login
`torizon@verdin-am62p`, and one says the console is bridge port +1 (the
main_uart0 port; the lease's own port is mcu_uart0 and stays silent, see
`docs/board-sessions/am62p-es2-final-r2.md`). Those values are not repeated in
a file other than as the user name above. Unverified: the ssh login answered
nothing, since it was never tried.

## Commands to run

```sh
BOARD_HOST=<user>@<host> BOARD_UART=<bridge host:port of main_uart0> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-cd738b1a records/am62p/common-cd738b1a
```

The build used `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`,
`GLMARK2_SRC`, `SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`,
`PVR_FW_BLOB` and `RUSTUP_TOOLCHAIN=nightly` from the deps cache. One trap: a
fresh worktree has no `driver/src/fw/` directory; create it before copying the
blob.

## Verdict lines and tally

None. Rows GM, SO, SV, CL, GT: **built, not run.** No glmark2 per-scene lines,
score, GL-error / incomplete-texture counters (record word w3) or memory /
refused-draw lines. Terrain, refract and shadow are unverified on the AM62P at
this commit.

## What differs from the last AM62P run

Unknown, for the same reason. Merges since the origin/main baseline that should
show on the AM62P (npot, pvr-npot-mips, pvr-refract) are unconfirmed on the
board.
