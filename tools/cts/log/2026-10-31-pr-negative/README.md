<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Forced-PR case (gl_test 217), partial render off: it did NOT reproduce the stop

Date 2026-10-31, AM62P (Verdin), item board-negative of plan #148 (issue #2/#3).

**Result: the 10 s stop did not reproduce.** Both runs ended with
`forcedpr: NOT EXERCISED` in 2.4 s: the kick completed without a single grow
request, stop, HWR or firmware partial render. The 256 overlapping 512x512
rectangles did not exhaust the 512-page list, so the case's premise (parameter
data exceeds the list) does not hold on this board. The expected stop (cause 2
or 4) was not seen; there is no HWR count or UPDATE_STATS stop to record other
than the zeros below.

## Image

- Source: origin/main `6db486e3` (has `forcedpr-images.sh` and case 217).
- Built with `driver/tools/pvrlab/forcedpr-images.sh <out>` (`ZEPHYR_WS` = the
  west workspace, `driver/src/fw/pvr_fw_blob.h` from the deps cache), `off` image.
- Kconfig: `CONFIG_GL_TEST_CASES="217"`,
  `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512`,
  `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`,
  `CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`,
  `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n`.
- zephyr.bin sha256 `276664f5845f54dc1848cd5e94b6ca3ea566ded66edf2777b0021625cedde0ba`.

## Commands

`tools/cts/pr-session.sh` and `tools/cts/pr-verdict.py` do not exist on main, so
`pr-verdict.py` was not run. `tools/board.sh am62p run` could not be used: its
serial capture needs a TCP bridge reachable by name and the TFTP route needs
host settings this task machine lacks. The board's own `uboot-mmc` one-shot was
done by hand instead, as `board.sh` does it (image written to `/var/yakogl`,
sha256 read back, `fw_setenv yako_src mmc`, `yako_file`, `yako_once`, reboot),
with the console read through the leased serial MCP. Staged files and the
`yako_file` value were restored afterwards; `yako_once` is empty.

## Lines (both runs, same)

```
gl_test: forcedpr: NOT EXERCISED: 256 layers: requests 0 (served 0, refused 0), notices 0, stop 0, partial renders 0, GPU recoveries 0
I: render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)
I: render freelists: ... firmware UPDATE_STATS: partial renders 0, out of memory 0, TA stores 0, 3D stores 0, CDM stores 0, TDM stores 0, other 0
```

`run1.log`, `run2.log` are excerpts of the serial console (from the
start of the capture, with the addresses/pointer dumps dropped; nothing
else redacted was present). Case 175's knob-off negative is recorded by
neg175-board under `tools/cts/log/*-pr-neg175/`.
