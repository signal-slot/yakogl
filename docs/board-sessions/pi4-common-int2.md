<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session 2, common set from the integration tip

Status: **the five images built (rc=0 each, twice), but nothing ran on the board.**
The Pi 4's console is reachable only through `BOARD_UART`, and the machine
that held the lease (`system76`) had no value for it. `tools/board.sh pi4
run|tree` needs `BOARD_UART` for every row, because the Pi 4's results come
back only on its serial console. No verdict line, glmark2 score, per-scene
fps, carve-out boot line, Slint or cluster crc16, or gl_test tally comes from
this session. Every figure in "Expected" below is a reference from earlier
records, not a reading.

Second turn (same day): the task was resumed with the message that it had
moved to `nuc`, which has `BOARD_UART`. The shell still ran on `system76`.
`board.sh --dry-run` printed the placeholder `/dev/PI4_UART`, so `BOARD_UART`
was still unset. `BOARD_HOST` could not be set either: the command prefix,
`env`, `export` and a wrapper script were all refused by the worker's
permission policy. `git worktree add` was refused too, so the second build
ran from the workspace itself (HEAD `1e0c7566`). That commit adds only this
document and the handoff note on top of `3cc4b60a`, so the sources are
identical. Its five images are listed below the first build's.

HEAD8: `3cc4b60a` (the integration tip, `3cc4b60acf33ef757a754c99827962fbbced0446`;
board.sh printed `build 3cc4b60a`). Board: Raspberry Pi 4 (the farm's
lease, `$BOARD_HOST`), boot tryboot, no `BOARD_BOOT`. Date 2026-10-15.

## Built images (`tools/cts/cts-common.sh pi4 build build/common/pi4-3cc4b60a`)

Zephyr `v4.4.0-9848-gaf17c0c8aa2a`, board `rpi_4b`, SDK 1.0.1. The `.built`
lines, verbatim:

```
GM rc=0 sha256=5e09d0ee852bc4123fc3512ee766817c63b0a80ae1c17329caac3501e6a2de0b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3cc4b60a
SO rc=0 sha256=b13cdc5ae0698b6a3e320379322f83fb399cc5a3c55b373b1c69526dccd5b292 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3cc4b60a
SV rc=0 sha256=495826d8a117712f5457d6391e885dbaca2267adb097882a43a2011245151b25 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3cc4b60a
CL rc=0 sha256=83abd1fae10dc78061efaf9a594ec3ac69a82fcc4fa8bc6240427db1c71ab413 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3cc4b60a
GT rc=0 sha256=c0dfb81cbde883d9804f1346be28972dfdb9b3ab03de4dfca6b034df779230d0 build=V3D_BUILD=3cc4b60a
```

Second build, `tools/cts/cts-common.sh pi4 build build/common/pi4-1e0c7566`
(workspace HEAD `1e0c7566`, same sources as `3cc4b60a`; the sha256 values
differ only because the tree id compiled into the image differs):

```
GM rc=0 sha256=d330355f306ce35eb540522cb534ee6fd8ba667e67326a0b35573d41657f70c8 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=1e0c7566
SO rc=0 sha256=ca3eea9359c2b6be2cbad8ecd23ac6b1fb7fafbd1d1f9853ed0baff7c99e8a31 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=1e0c7566
SV rc=0 sha256=17d8424d2d6b943aa58b93113b3e35ff111cd3070120acd3b1640da56b2fb76c build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=1e0c7566
CL rc=0 sha256=3045a80ce94d83db03e01038d9eacb5a11267508d6ae777c36afd365154f2c26 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=1e0c7566
GT rc=0 sha256=4155a2925ea2306bcd0a9186a8c7d080fcc8f04dac229808f51a1b32a9cf9bb3 build=V3D_BUILD=1e0c7566
```

`tools/board.sh --dry-run pi4 run .../GM/zephyr/zephyr.bin` read GM's footprint
again as 250,490,880 bytes (`build 1e0c7566 boot tryboot (row)`).

GM's `.config` has `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES=100663296` (96 MiB) and
`CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES=25165824` (24 MiB), as
`driver/samples/zephyr/glmark2/boards/rpi_4b.conf` sets. The carve-out does
not overflow RAM at link time: board.sh's image check (`--dry-run`) read GM's
footprint as 250,490,880 bytes, inside the Pi 4's load window
(0x200000..0x10200000), and SO's as 139,952,128.

## What was checked on the board (ssh only)

The board was idle: up 5 h 10 min, load 0.00, no GL client running, no
`yakogl-*.bin` left in `/boot/firmware`. `tryboot.txt` was the empty file a
clean board.sh run leaves. The board's Linux has no `/dev/serial/by-id`.
Nothing was staged and the board was not rebooted. On the second turn it was
again idle: up 7 h 08 min, load 0.00, no GL client, no `yakogl-*.bin`, an
empty `tryboot.txt`; the only sessions were the board's own seat0/tty1 login
and the check's own ssh.

`tools/cts/cts-common.sh --dry-run pi4 run build/common/pi4-3cc4b60a
records/pi4/common-3cc4b60a` printed the full sequence for all five rows
(`=== GM rc=0` ... `=== GT rc=0` in the dry run) with the placeholder
`/dev/PI4_UART`. Without the placeholder, the real run stops at the first row
on `: "${BOARD_UART:?set BOARD_UART (or PI4_UART)=...}"`.

## Verdict lines

None. `tools/cts/cts-common.sh pi4 run` was not started, so there is no
`board.sh pi4 <LABEL>-3cc4b60a ...` line for GM, SO, SV, CL or GT. Record
directory, once it runs: `records/pi4/common-3cc4b60a/`, in the checkout that
runs it (gitignored).

| row | verdict | reading |
|---|---|---|
| GM | not run (no `BOARD_UART`) | none |
| SO | not run | none |
| SV | not run | none |
| CL | not run | none |
| GT | not run | none |

## glmark2 per scene

Not run. The table to fill in. The reference column comes from the last Pi 4
common run, `86be8aeb` (`docs/board-sessions/pi4-common-int.md`, on the same
96 MiB carve-out and 24 MiB tiled-copy budget):

| scene | this session | reference at `86be8aeb` |
|---|---|---|
| boot line | not run | `osal(v3d): ready: carve-out 96 MiB at phys 0x2fdd000, GPU VA 0x181000..0x6100000` |
| terrain | not run | about 2 fps (447.6 ms/frame, 23 frames); no `out of GPU memory`, no `no GPU memory for a tiled copy`, no `GL error 0x502` |
| refract | not run | about 1.4 fps (720.8 ms/frame, 14 frames); `tiled copies high-water 19742720 of 25165824 bytes`; no memory refusal |
| the other 31 scenes | not run | not recorded per scene in that document |
| score | not run | 183; 33 ran, 0 failed, 0 unsupported; GL errors 0, incomplete-texture draws 0 |

## Expected (references; a different result is a finding)

- GM: the boot line says `carve-out 96 MiB`. No `out of GPU memory`, `no GPU
  memory for a tiled copy`, refused-draw or `GL error` line in any scene,
  terrain and refract included. The record reads `scenes ran 33 failed 0
  unsupported 0`, GL errors 0, incomplete-texture draws 0, and a score near
  183. "Terrain not black" can only be argued from 0 GL errors and 0
  incomplete-texture draws. The console carries no pixels.
- SO and SV: `slint: check PASS`. Record `594b0601,08420862,00210842,c5920883`,
  crc16 `c592`, crc32 `0xfaa46995`. SV adds `the 1024x608 window is a view
  into the framebuffer, pitch 5120 (no copy)`. An earlier unmerged run is
  said to have seen the fingerprint `0xe7113010`. This session cannot compare
  it with the reference.
- CL: `probes: 34 of 34 within 12`. Record `594b0601,00a710c4,10c40086,7b932967`,
  crc16 `7b93`, about 139.5 fps at 1920x720.
- GT: `v3d_gl_test: 12 of 12 cases passed`, 0 recoveries. If cases 2, 10 or
  16 fail, rerun GT once.

## Commands to rerun

From a checkout of `3cc4b60a` (here the ccfarm workspace itself, which was
clean at that commit), on a machine that has the Pi 4's console:

```sh
HEAD8=$(git rev-parse --short=8 HEAD)                 # 3cc4b60a
export ZEPHYR_WS=<west workspace> WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=<zephyr sdk>
export RUSTUP_TOOLCHAIN=nightly GLMARK2_SRC=<...> SLINT_NOSTD_DIR=<...> NOSTD_PATCH_DIR=<...> AGL_SLINT_CLUSTER_DIR=<...>
export BOARD_HOST=<lease user@host> BOARD_UART=<the Pi 4 console: device or host:port of its bridge>
export BOARD_HDMI_TIMINGS=<the lab's 17 fields, tools/cts/README.md>
tools/board.sh --dry-run pi4 status
tools/cts/cts-common.sh pi4 build build/common/pi4-$HEAD8          # all five rc=0 at 3cc4b60a
tools/cts/cts-common.sh --dry-run pi4 run build/common/pi4-$HEAD8 records/pi4/common-$HEAD8
tools/cts/cts-common.sh pi4 run build/common/pi4-$HEAD8 records/pi4/common-$HEAD8
grep -E 'carve-out|out of GPU memory|tiled copy|GL error|refused|was not recorded|glmark2: ' records/pi4/common-$HEAD8/*-GM-*/console.txt
```
