<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P partial render: cases 175 and 217, knob off and on (2026-11-27)

Plan #301 item board-regress, issue #2 of signal-slot/yakogl (acceptance c2).
Design: docs/notes/pvr-render-pool.md, "Design: the firmware's partial
render". Board: Verdin AM62P (PowerVR BXS-4-64), U-Boot one-shot from eMMC
(`BOARD_BOOT=uboot-mmc`), `tools/board.sh am62p run`.

Every image was built from main **`c8be59f6`** (the tip of origin/main on
2026-11-27): the build line in each log is `build tree=c8be59f6`. The knob is
`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`. Its Kconfig default is n and was
not changed.

## Runs

| log | image | Kconfig | result |
|---|---|---|---|
| `217-off.log` | `aa4a7b35` | `tools/cts/pr-configs/gl_test-217-off.conf` (`pr-session.sh 217 --leg 217-off`): knob off, cap 512 pages, 1220 layers | TIMEOUT, **stop cause 2** (out of memory at the list's maximum), partial renders 0, recoveries 0: the known negative |
| `217-on.log` | `8157554a` | `gl_test-217-on.conf` (`pr-session.sh 217on --leg 217-on`): knob on, the same cap and layers | the at-maximum wait line, then **1 partial render**, the kick completed, no stop, recoveries 0, **but FAIL pixels**: 1,001,186 of 1,048,576 pixels differ, first at (48,0): got `0xff000000`, want `0xff076511` |
| `175-on-noreserve.log` | `5d2c3827` | `gl_test-175-on.conf` + `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y` | **PASS**, grow request refused, **1 partial render**, no stop, GPU recoveries 0 |
| `175-off-noreserve.log` | `3b5850fe` | `gl_test-175-off.conf` + `..._FL_PROBE_NO_RESERVE=y` | TIMEOUT, **stop cause 4**, partial renders 0, recoveries 0: the known negative |
| `175-on.log` | `e0e8a37f` | `gl_test-175-on.conf` as committed | FAIL: the grow was **served** (256 pages), no refusal, partial renders 0 |
| `175-off.log` | `f24a98f0` | `gl_test-175-off.conf` as committed | FAIL: the grow was served, no refusal, no stop |

Each `*.index.txt` is board.sh's INDEX.txt for that run (image sha256, times,
the verdict line).

### Case 175: the committed fragments do not refuse the grow

The committed `gl_test-175-{off,on}.conf` fragments leave
`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE` unset (tools/cts/pr-configs/README.md
says so: they follow `pr-session.sh`, which has no option for it). With the
reserve present, case 175's one grow request is served
(`FREELIST_GROW id=1 -> +256 pages`), nothing is refused, and the case's own
check fails in both knob states: these images do not exercise the partial
render. The neg175 record (`tools/cts/log/2026-10-31-pr-neg175/`) was built
with `NO_RESERVE=y`, and the pr-configs README says a rebuild of neg175 adds
it on the command line. The `-noreserve` pair is that rebuild: the
committed fragment plus `-DCONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`.
Those two runs are the acceptance runs for case 175.

### Case 217 knob on: the partial render happens, the picture is wrong

With the knob on, the at-maximum stop of the knob-off image becomes the
bounded wait (`9b2e88bc`), and the firmware renders partially:

    W: render: at maximum (list 1, 512 pages), awaiting the firmware's partial render for up to 10 s
    I: FWCCB: UPDATE_STATS element=1 (partial renders) adjust=1 (seen 1)
    I: render: partial render after the at-maximum notice (list 1); its wait is over
    gl_test: forcedpr: FAIL pixels: 1220 layers (1 step): requests 0 (served 0, refused 0), notices 1, stop 0, partial renders 0, GPU recoveries 0
    gl_test: forcedpr: 1001186 of 1048576 pixels differ; first at (48,0) from the bottom left: got 0xff000000 want 0xff076511

So the run did not end in cause 4, and the run-C fallback
(docs/notes/pvr-pr-atmax.md) was not needed to force the partial render.
The run-end firmware counter says 1 partial render (`firmware UPDATE_STATS:
partial renders 1`). gl_test's own summary line still says `partial renders
0`. It seems to read its counter before the firmware's statistics arrive, but
this session did not check that. 95.5% of the 1024x1024 pixels read back as
opaque black, `0xff000000`. That is the case's clear colour
(`glClearColor(0, 0, 0, 1)` in `case_forcedpr`), so most of the drawn layers
are missing from the colour target the final render resolves.
The likely causes are the SPM background reload of the colour (the load
shader and `pds_pr_bgnd`, which the code says is still not written into the
3D_PR: render.rs `PvrRenderFragPr`) or the PR PBE/EOT store into the SPM
scratch. pr-verdict files the run as "class 2 F8 corrupt frame". The depth
read-back has no mismatch line. The 2026-11-02 run C (no cap, no reserve,
before the frame side) also ended with "colour mostly the clear" and its
depth equal.

## pr-verdict lines (`python3 -I tools/cts/pr-verdict.py [--expect ...] <log> <case>`)

    217-off.log --expect stop 217
    pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
    pr-verdict: forcedpr FAIL stopped layers=1220 steps=1 partial_renders=0 recoveries=0
    pr-verdict: grow requests 0 served 0 unserved 0
    pr-verdict: OK

    217-on.log --expect pr 217
    pr-verdict: case 217 FAIL partial_renders=1 oom=1 hwr=0 stopped=no
    pr-verdict: forcedpr FAIL pixels layers=1220 steps=1 partial_renders=0 recoveries=0
    pr-verdict: grow requests 0 served 0 unserved 0
    pr-verdict: at-maximum wait resolved by partial render
    pr-verdict: class 2 F8 corrupt frame
    pr-verdict: FAIL case 217 is FAIL; forcedpr FAIL pixels

    175-on-noreserve.log --expect pr 175
    pr-verdict: case 175 PASS partial_renders=1 oom=1 hwr=0 stopped=no
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: class 1 success
    pr-verdict: OK

    175-off-noreserve.log --expect stop 175
    pr-verdict: case 175 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause4
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: OK

    175-on.log --expect pr 175
    pr-verdict: case 175 FAIL partial_renders=0 oom=1 hwr=0 stopped=no
    pr-verdict: grow requests 1 served 1 unserved 0
    pr-verdict: class 0 unclassified
    pr-verdict: FAIL case 175 is FAIL; partial renders 0 (want >= 1)

    175-off.log --expect stop 175
    pr-verdict: case 175 FAIL partial_renders=0 oom=1 hwr=0 stopped=no
    pr-verdict: grow requests 1 served 1 unserved 0
    pr-verdict: class 0 unclassified
    pr-verdict: FAIL no 'render: stopped' line (want cause 2 or 4)

## board.sh verdict lines

    board.sh am62p pr-217-gltest rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=124s
    board.sh am62p pr-217on-gltest rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=122s
    board.sh am62p pr-regress-175R-on rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=99s
    board.sh am62p pr-regress-175R-off rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=120s
    board.sh am62p pr-regress-175-on rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=104s
    board.sh am62p pr-regress-175-off rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=105s

`verdict=failed` is board.sh's reading of gl_test's FAIL/TIMEOUT. For the
negatives that is the expected result. Every boot was `boot_verdict=done`
with `resets=0`.

## How the consoles were taken

The lease's serial MCP server held the board's serial bridge, so
board.sh's own capture got `busy`. The runs used the feed bridge that
tools/cts/README.md documents (`python3 tools/am62p_netboot_uart.py feed
<file> <port>`, `BOARD_UART=127.0.0.1:<port>`). The agent appended the MCP
console's output to the feed file. Each log is relayed unchanged from the
line holding `reboot: Restarting system` through `Starting kernel ...`, so
the last Linux shutdown lines before that line are not in the logs. The
`HH:MM:SS` stamps in front of the lines are the times at which the relay
delivered them, which can be seconds to minutes late. They support no timing
claim, and neither does `back_after`. The 217 legs ran through
`tools/cts/pr-session.sh --leg`. That script found no console
(`pr-session.sh: no console log for gltest`): it looks for
`<build>/run-gltest/console.txt`, but board.sh writes the run into a
timestamped subdirectory. Its README therefore had no pr-verdict lines.
The logs here were copied from board.sh's run directory and redacted as
pr-session.sh does, and pr-verdict was run on them by hand with the leg's
arguments.

The board runs before the agent's worker restarted (same day, the same
image hashes `aa4a7b35`, `8157554a`, `5d2c3827`, `3b5850fe`, `e0e8a37f`,
`f24a98f0`) gave the same results line for line. Their logs were lost with
the worker. These are the second runs.
