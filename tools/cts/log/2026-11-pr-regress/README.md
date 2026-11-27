<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P regression set, partial-render knob off and on (2026-11-27)

Plan #301 item board-regress, issue #2 of signal-slot/yakogl (acceptance c3:
the regression set unchanged with `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`
on and off). Board: Verdin AM62P (PowerVR BXS-4-64), `tools/board.sh am62p
run`, U-Boot one-shot from eMMC.

The regression set is the AM62P common set (`tools/cts/common-tree.txt`):
gl_test with its default case list (`GL_TEST_CASE=7`, 50 cases) and glmark2
with its default benchmark set (33 scenes). Every image was built from main
**`c8be59f6`** with `tools/cts/common-images.sh`'s AM62P arguments (the
watchdog self-arm, `-DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`) plus only
`-DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n|y`. The build line in each log
is `build tree=c8be59f6`.

| log | image | knob | result |
|---|---|---|---|
| `gltest-off.log` | `191d6a99` | off | 50 of 50 PASS, GPU recoveries 0, no `render: stopped` |
| `gltest-on.log` | `1e5c8c99` | on | 50 of 50 PASS, GPU recoveries 0, no `render: stopped` |
| `glmark2-off.log` | `5590e5ba` | off | score **489**, 33 scenes, 0 failed, 0 unsupported, resets 0 |
| `glmark2-on.log` | `2d9099f9` | on | score **485**, 33 scenes, 0 failed, 0 unsupported, resets 0 |

In all four runs the firmware's run-end statistics read `partial renders 0,
out of memory 0`, and there were 0 grow requests. The regression set never
fills a free list, so the knob's partial render is never exercised here. What
the knob still changes on every run: the firmware boots with
`FWINIT_PARTIAL_RENDER` (`0328f6be`), its partial-render trace groups are
unmasked (fwinit-log-unmask), and eligible frames carry a filled `frag_pr`.

## pr-regress-compare (`python3 -I tools/cts/pr-regress-compare.py <off> <on>`)

The full outputs are `compare-gltest.txt` and `compare-glmark2.txt`.

gl_test (`gltest-off.log` against `gltest-on.log`):

    cases differing: 0 of 50
    == resets
    run1: GPU recoveries 0 (HWR counter 0), reset lines 0, render: stopped 0
    run2: GPU recoveries 0 (HWR counter 0), reset lines 0, render: stopped 0
    pr-regress: SAME

glmark2 (`glmark2-off.log` against `glmark2-on.log`):

    == resets
    run1: GPU recoveries 0 (HWR counter 0), reset lines 0, render: stopped 0
    run2: GPU recoveries 0 (HWR counter 0), reset lines 0, render: stopped 0
    glmark2 score: run1=489 run2=485  DIFFERS
    glmark2 differences: 25
    pr-regress: DIFFERENT (0 cases, 0 resets)

The glmark2 DIFFERENT comes only from per-scene FPS, which the tool compares
for exact equality. Of the 25 differences, 22 are scene FPS changes of 0.3
to 1.8%: build vbo 903 against 887 is the largest, conditionals 1097 against
1092 and texture nearest 1050 against 1043 are typical. In 21 of those 22 the
knob-on run is lower; bump high-poly is the one that is higher (335 against
336). 10 scenes are equal. Two more "differences" are one scene, terrain, at
13 FPS in both runs: in each run a driver warning line came between
terrain's scene header and its FPS line, so the tool keyed the scene
differently in the two logs. The last difference is the total score.
Reading the logs directly:

- no scene failed, became unsupported or changed its set-up result;
- there is no reset, recovery or `render: stopped` in either run;
- the picture probes (`glmark2-picture:` lines, 33 in each run) are equal in
  22 scenes (the five probe colours, not the CRCs). In five scenes (texture
  linear and mipmap, shading gouraud and blinn-phong-inf, bump high-poly)
  they differ by one or two counts per channel. The other six are the
  animated scenes: desktop (2), buffer map (2), terrain and refract.
  RESULTS.md already lists these as mismatches between single runs. The
  CRCs differ in nearly every scene, because the scenes animate and the
  frame read depends on timing.

Reading: the regression set's pass set is unchanged (gl_test SAME, 50 of 50
in both). glmark2 runs the same 33 scenes with the same outcomes and no
resets. The knob-on score is 0.8% lower, 485 against 489. The knob-off 489
equals the AM62P row of main `b884868c` (RESULTS.md). One knob-on run is not
enough to tell this offset from run-to-run noise. The likely source is the
firmware's unmasked trace groups, but this session did not check that.

## board.sh verdict lines

    board.sh am62p pr-regress-GT-off rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=144s
    board.sh am62p pr-regress-GT-on rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=149s
    board.sh am62p pr-regress-GM-off rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0 record=594b1201,fc3f01e9,6184003f,00258320 back_after=425s unsupported=0
    board.sh am62p pr-regress-GM-on rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0 record=594b1201,fc3f01e5,6184003f,00258320 back_after=417s unsupported=0

Each `*.index.txt` is board.sh's INDEX.txt for the run.

## How the consoles were taken

The consoles went through the feed bridge of tools/cts/README.md
(`am62p_netboot_uart.py feed`), because the lease's serial MCP server held
the board's bridge. The agent relayed each run's console, unchanged, from
the `reboot: Restarting system` line through `Starting kernel ...`. The line
stamps and `back_after` are relay times (the glmark2 runs were relayed in
pieces, which accounts for the 417 to 425 s). They support no timing claim.
The glmark2 FPS and scores are the image's own numbers, and the relay does
not affect them.

Not run in this session: d320c8a5's glmark2 configuration (one kick in
flight, `tools/cts/pr-configs/glmark2-d320c8a5-{off,on}.conf`).
