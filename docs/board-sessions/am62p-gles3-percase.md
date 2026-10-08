<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES3 per-case failure list (am62p-gles3-percase)

**Result: no dEQP case result was obtained.** The board session was started
but the leased serial bridge port is wired to the wrong UART (section "Why the
console was silent"): it never sees the Zephyr console, so there is no UART
log, no tally and no per-case list. `am62p-gles3-percase-fails.tsv` was not written
(an empty or header-only file would read as "no failures"). Commit under test:
the integration tip `28c22b8d` (this note is the only change on top of it). The single trial image below was built from a docs-only descendant of it (tree string `2584915e`, a commit that no longer exists after the squash), and its log is empty anyway. The 396d667a figures stay those of
`tools/cts/RESULTS.md` section 1r (31,278 pass / 8,835 fail / 2,636 not
supported / 5 QualityWarning / 56 lifetime without result). Per-image
pass/fail/n.s.: none measured.

## What was done (2026-10-08)

1. Board reachable by ssh (key login; the Torizon kernel 6.6.138). The
   firmware `/lib/firmware/rgx.fw.36.53.104.796` was copied with `scp` to a
   scratch file and turned into the git-ignored `driver/src/fw/pvr_fw_blob.h`
   with a short `python3 -I` script (`driver/tools/mkfwblob.sh` was refused at
   the time). A copy of the firmware file had been committed by a ccfarm turn commit; the history was squashed, so the branch holds only this note.
2. `tools/cts/am62p-deqp-images.sh` with `DEQP_MODULE=gles3` builds. Image 7
   (shaders.builtin_functions, builtin_variable, conditionals,
   constant_expressions) built clean: rc=0, zephyr.bin sha256 prefix
   `315083cf7467af6b`, `sample: build tree=2584915e`. The other 15 were not
   built to completion (a build of them from a clean commit was started, then
   stopped when the console proved silent).
3. `tools/board.sh am62p status`: the U-Boot hook is installed
   (`yako_src=mmc`), so `BOARD_BOOT=uboot-mmc` is the route that needs no TFTP
   server. A first `board.sh ... tree` attempt failed at once with the bridge
   busy (rc=3, before any reboot, staged file removed). A second attempt, later
   in the session, got the bridge and ran: the board rebooted at the start of
   the run, the runner waited 426 s, and the verdict was
   `board.sh am62p img7 rc=4 verdict=empty boot=uboot-mmc row boot_verdict=empty
   result=none resets=absent record=absent back_after=426s`. `console.raw`,
   `console.ts` and `console.txt` are all 0 bytes, and the leased console MCP
   client of this session, connected to the same adapter, also read 0 bytes
   (offset 0 for the whole session). `yako_once` was cleared (U-Boot consumed
   the one-shot) and Linux is up again with a boot time right after the run
   started, i.e. the image did not run for its 600 s. Without a console it
   cannot be said whether Zephyr started, died early, or U-Boot's load failed.
4. The run is therefore **not** a board result and counts as nothing.
   `board.sh` also printed `gawk: command not found` (line 911) on this host;
   that may only affect the post-run tally.

## Why the console was silent

The lease names the bridge port `usb-serial-1:35932`. Probed from Linux on the
board (ssh, `sudo -n`), each UART at 115200 in turn, with the lease's console
client reading the port:

| Linux tty | controller | Linux alias | what port 35932 received |
|---|---|---|---|
| ttyS2 (getty, U-Boot/Linux console) | `serial@2800000` (main_uart0) | serial2 | nothing |
| ttyS0 | `serial@2810000` (main_uart1) | serial0 | nothing |
| ttyS1 | `serial@0` in the `2b300050` target module (wkup_uart0) | serial1 | nothing |
| ttyS3 | `serial@4a00000` (mcu_uart0) | serial3 | `YAKOGL-PROBE-S3`, verbatim |

(The first probe, written to ttyS0/1/2/3 at once while ttyS0/1/3 were still
at 9600 baud, arrived as 74 NUL bytes: the 9600 baud ttyS3 text read at
115200.) A newline typed into port 35932 gave no login prompt either: the
getty is on ttyS2.

The Zephyr image prints on `main_uart0`
(`driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts`:
`zephyr,console = &main_uart0`), the controller Linux calls ttyS2. So port
35932 is wired to mcu_uart0 and can never carry U-Boot's or Zephyr's output.
This matches the repository's own records: `tools/cts/RESULTS.md` (around
line 1007) runs the AM62P console on `usb-serial-1:35933` and calls 35932
dead (for the console), and every AM62P dEQP run there, including section 1r,
used port 35933. This session's lease did not include port 35933, so it was
not used.

To finish the item, the AM62P lease has to hand out the bridge port wired to
main_uart0 (35933 in the repository's records), or the adapter on 35932 has
to be moved to the Verdin's debug UART. Check it before building anything:
from Linux, `echo probe | sudo tee /dev/ttyS2` must show up on the leased port.

## Commands that finish the run

From a clean checkout, with shell access to the board, a working UART bridge
and the deps cache. The shell environment must hold the variables (the
sandbox of this session refused `VAR=x cmd` prefixes). `BOARD_HOST` is the
lease's ssh destination (`$RESOURCE_VERDIN_AM62P_SSH`), `BOARD_UART` the
bridge address of `usb-serial-1`
(`$RESOURCE_USB_SERIAL_1_HOST:$RESOURCE_USB_SERIAL_1_PORT`); neither is written
down here. Every log's first line `sample: build tree=` must show the checked
out commit; commit the docs first, then build, so the tree string is clean.

```sh
D=$CCFARM_DEPS_DIR
export ZEPHYR_WS=$D/zephyr-ws WEST_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export DEQP_WORK=$D/deqp-stage1-am62p-gles3 DEQP_MODULE=gles3
export BOARD_HOST=$RESOURCE_VERDIN_AM62P_SSH
export BOARD_UART=$RESOURCE_USB_SERIAL_1_HOST:$RESOURCE_USB_SERIAL_1_PORT BOARD_BOOT=uboot-mmc

# firmware blob (never inside the repository tree except the ignored header)
scp "$BOARD_HOST":/lib/firmware/rgx.fw.36.53.104.796 "$TMPDIR/rgx.fw"
driver/tools/mkfwblob.sh "$TMPDIR/rgx.fw"     # writes the git-ignored driver/src/fw/pvr_fw_blob.h

# images img<N>-<tag>: filter-file line N = image N
T=$(git rev-parse --short=8 HEAD)
tools/cts/am62p-deqp-images.sh tools/cts/am62p-gles3-tree.txt records/am62p/gles3-percase-img $T > records/am62p/gles3-percase-img/build.out

# board, the task's order (zero-based rows of the tree file: images 7,8,9,15,16,18 = 6,7,8,14,15,17)
export TREE_IMAGE_DIR=$PWD/records/am62p/gles3-percase-img TREE_TAG=$T
TREE_ROWS=6,7,8,14,15,17 tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt records/am62p/gles3-percase-a
# then TREE_ROWS=21,22,23,24 ; 1,2,3,4 ; 0,5
tools/board.sh am62p tally records/am62p/gles3-percase-a
tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt <console logs>
tools/cts/qpa-from-uart.py ...   # per-case results, then the tsv:
# case<TAB>verdict<TAB>first message (160 chars, tabs removed)<TAB>image
```

`TREE_IMAGE_DIR` + `TREE_TAG` make `board.sh tree` take the unlabelled
filter file and find the builder's `img<N>-<tag>` directories. An image
that outlives the 600 s watchdog is split by mustpass names as RESULTS 1r
did; stop starting images after about 2.5 hours of board time.

## Images not run

All of them: 7, 8, 9, 15, 16, 18, 22, 23, 24, 25, 2, 3, 4, 5, 1, 6 (and the
deliberately skipped 19*/21 and lifetime). Image 7 was built and tried once
(no console output, above); no image produced a result.

## Per-subgroup first-message table

Not available. The existing group-level message buckets are in RESULTS.md
section 1r.
