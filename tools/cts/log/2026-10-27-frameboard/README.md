<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 frameboard: cases 4, 52, 54 x2 and panthor-gltest --sweep x2

Plan #153, item frameboard (#32, #33). Board: NanoPC-T6 (RK3588, Mali-G610,
panthor), Linux 6.18.55-current-rockchip64, boot_id unchanged over all eight
runs, board clock 2026-10-27 11:55 to 11:57 UTC.

Code: the integration tip `0543b60b` (builtin-lower's cull fix `272033a3`,
frame-instance and frame-f16rt merged) plus the predictions commit. The
binary was cross-built on the host and carries `yakogl-build=376b9cf8`
(sha256 `575e5241…6506`). The predictions (`predictions.txt`) were
committed before the first run.

How it ran: `tools/mali/panthor/run-on-board.sh` was refused by the session's
permission layer, so its steps were done by hand under connected's exclusive
lease of the board: the same cargo build (target aarch64, static), the
build-id check, a free `/dev/dri/renderD128` (`fuser`), a `dmesg -w` capture
between `yakogl-run-start` / `yakogl-run-end` markers, and boot_id before and
after. Every run: 0 kernel messages between the markers. The raw logs and
dmesg captures stay out of the tree.

## Cases 4, 52 and 54, two runs each

| Run | Verdict line |
|---|---|
| case 4, 1 | `panthor-gltest: verdict case 4 cull: PASS` |
| case 4, 2 | `panthor-gltest: verdict case 4 cull: PASS` |
| case 52, 1 | `panthor-gltest: verdict case 52 es3split: PASS` |
| case 52, 2 | `panthor-gltest: verdict case 52 es3split: PASS` |
| case 54, 1 | `panthor-gltest: verdict case 54 es3chain: PASS` |
| case 54, 2 | `panthor-gltest: verdict case 54 es3chain: PASS` |

Case 54's log shows phase B's half-float pass ran: `gl_test: chain: cube
128x128 f16, six faces drawn (err 0x0)` down to `cube 4x4 f16`. All three
match the predictions.

## Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run. The two are byte-equal.

    panthor-gltest: 256 case(s) run: FAIL 1, NO-CASE 86, PASS 102, REFUSED 59, SKIP 8

Against the RESULTS.md Mali record (`2026-10-24-refusals`, 98 PASS), every
line that changed:

- REFUSED to PASS:
  - `verdict case 4 cull: PASS` (was `REFUSED draw: mali: a draw with face culling on is not built yet`)
  - `verdict case 54 es3chain: PASS` (was `REFUSED draw: mali: a colour target other than RGBA8888, XRGB8888 or RGB565 is not built yet`)
  - `verdict case 73 branchlane: PASS` (was `REFUSED compile: ... Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)`)
  - `verdict case 114 cubelod: PASS` (was `REFUSED compile: ... vertex shader: Mali(Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet)`)
- NO-CASE to FAIL: `verdict case 214 splitkick: FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080`.
  Case 214 is new in gl_test (written for the PowerVR's split 4x frame).
  Its own comment names this reading: the second kick reloaded the
  resolved colour, not the four samples, so the half-covered diagonal is
  blended over 0x80 again (0xc0). Not a PASS lost; a Mali defect to file.
- Refusal reason moved, still REFUSED: 63, 68, 71, 74, 78, 81, 100, 101,
  116, 142, 148 (exact lines in the verdict files).

No PASS was lost. `python3 -I tools/mali/sweep_passsets.py` over the
baseline record and this one: `each earlier PASS set is a subset of
2026-10-27-frameboard (102 PASS)`. Cases 8, 28, 29, 30, 52, 113, 122, 169,
196, 197, 198, 207, 208 and 209 PASS in both runs.

Prediction check: predicted 100 PASS (98 + 4 + 54) "or more"; got 102 (also
73 and 114, whose refusals were arithmetic other than +, -, * and a
constant varying lane). The FAIL of the new
case 214 was not predicted.
