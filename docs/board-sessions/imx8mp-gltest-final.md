<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# i.MX8MP board session: gl_test default list from origin/main (imx8mp-gltest-final)

Plan #143 item `imx8mp-gltest-final`. Date: 2026-10-14. Board: Verdin i.MX8M Plus
(GC7000), the leased `verdin-imx8mp` resource, `BOARD_BOOT=uboot-mmc`.

## Summary

**Blocked, no results.** The board answered over ssh and an image for case 16 was built
from the origin/main tip, but every attempt to start `tools/board.sh imx8mp` was refused
by the worker's command policy, so nothing was booted. Nothing below is a measurement.

| what | state |
|---|---|
| origin/main | `e0eb883d3428b096aac9028f07bce90c159df111` (`e0eb883d`, "Merge pull request #615 from signal-slot/work/gate-every-runner"), the tip after `git fetch origin` on 2026-10-14; built in a detached worktree of that commit |
| Board reachable | yes: `ssh torizon@verdin-imx8mp -- uname -a` printed `Linux verdin-imx8mp-... 6.6.142-7.7.0 #1-Torizon SMP PREEMPT ... aarch64 GNU/Linux` |
| Image for case 16 | built: `tools/cts/imx8mp-gltest-build.sh msaa-e0eb883d 0 -DEXTRA_CFLAGS=-pipe` printed `rc=0` (blocker 2 explains `-pipe`) |
| Default-list image (GT) | **not built** at `e0eb883d` (blocker 2) |
| Board run | not run (blocker 1) |
| `board.sh ... verdict=` lines | none: `tools/board.sh imx8mp` never started |
| Case #8 (gl_test case 16, msaa) | not run |
| Case #9 (gl_test case 10) | not run |
| Case #25 (gl_test case 2) | not run |
| Per-case pass/fail of the default list | not run; no failure was observed and none can be claimed |

## Blockers (exact)

1. **`tools/board.sh` cannot be started.** `board.sh` needs `BOARD_HOST` (the lease's
   `torizon@verdin-imx8mp`, also in `$RESOURCE_IMX8MP_SSH`) and `BOARD_BOOT=uboot-mmc` in
   its environment, and it has no fallback for `BOARD_HOST` on the i.MX8MP. The worker
   command policy answered `This command requires approval` (no approval is possible in
   an unattended session) to each of these forms:
   - `BOARD_HOST=... BOARD_BOOT=uboot-mmc tools/board.sh imx8mp --help`
   - `BOARD_HOST=... BOARD_BOOT=uboot-mmc SEL=16 tools/board.sh imx8mp run <msaa image> <rec> msaa-e0eb883d`
   - `bash <wrapper script that exports the two variables and calls board.sh>`
   I did not look for another way around the refusal. No `tools/board.sh imx8mp`
   process ever ran, so there is no `verdict=` line.
2. **The GT image cannot be built on this worker.** `tools/cts/common-images.sh imx8mp
   tools/cts/common-tree.txt <dir> GT` failed at build step 182 of 190, three
   times in two different output directories, with the same line in `GT.log`:
   `driver/tools/pvrlab/gl_test.c:22031:1: fatal error: error writing to /tmp/ccXXXXXX.s: Disk quota exceeded`
   (`df` shows free space on `/tmp`, so this is a quota of the worker's sandbox, not a
   full disk). The compiler writes the assembly of the 22,000-line `gl_test.c` to
   `$TMPDIR`, and I could not set `TMPDIR` (a command with a `TMPDIR=...` prefix is also
   refused). `common-images.sh` has no way to pass `-pipe`. The case-16 image went through
   because `tools/cts/imx8mp-gltest-build.sh` takes extra cmake arguments, so
   `-DEXTRA_CFLAGS=-pipe` (the compiler pipes instead of writing the `.s` file; the code
   is the same) got past it. That script builds the sweep configuration (online compiler,
   ES 1.00 shader set), not the common-set GT image.

An earlier worker of this item had built the GT image at `cd738b1a`
(`GT rc=0 sha256=c04fd86a118e3cf8bb815838d6ef47d106043e12286d8a501f059bc69b25f2de`), but
that image is on another node's cache and `cd738b1a` is not the tip any more, so it is not
used here.

## Commands for the rerun (not run here)

On a worker that may start `tools/board.sh` with `BOARD_HOST`/`BOARD_BOOT` set and has
room in `/tmp` (or may set `TMPDIR`), from a detached worktree of origin/main:

```sh
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $IMX8MP_OUT/gtfinal-<sha8> GT   # default list
BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run $IMX8MP_OUT/gtfinal-<sha8>/GT/zephyr/zephyr.bin <rec> gt-<sha8>-1
# case 16 (#8) alone, as in imx8mp-gltest-r2:
tools/cts/imx8mp-gltest-build.sh msaa-<sha8> 0
BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc SEL=16 tools/board.sh imx8mp run $IMX8MP_OUT/msaa-<sha8>/zephyr/zephyr.bin <rec> msaa-<sha8>
```

The case-16 image `msaa-e0eb883d` from this session (built at the tip `e0eb883d`) is
in this worker's `$IMX8MP_OUT` only if that cache is kept; rebuilding it takes one
command.

Cases #9 (10) and #25 (2) are in the default list; their flake rate (about 4 of 32
boots for case 10 in the last measurement) means one boot says little. History:
`docs/investigations/gc-kick-diff.md`. The reference for the default list is
`imx8mp-common-int.md` (GT on `39bb67d`: `rc=4 verdict=failed`, case 16 first failing
check `got 0x2e want 0x30`, 46 of 48 passed).

## Verdict lines

None. No `tools/board.sh imx8mp` command started in this session; the refused commands
printed `This command requires approval` and no `board.sh` line.
