<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 mid: case 214 and the first lowerings x2, panthor-gltest --sweep x2

Plan #154, item board-mid (#32, #33). Board: NanoPC-T6 (RK3588, Mali-G610,
panthor). boot_id unchanged over all four runs; 0 kernel messages between
the run markers and 0 in the second after them, in every run. Board clock
2026-10-28, 10:19 to 10:21 UTC.

Code: commit `b9d91861` (the integration tip `965ac247`, after split-fix,
texture-2, varyconst, builtin, intbool, intbool-2 and sizelimit, plus the
predictions commit). The binary carried `yakogl-build=v0.1.0-65-gb9d9186`
(the board's clone holds the public tag `v0.1.0`; the commit is the same),
sha256 `cb63ea3c…4525`.

Predictions: `predictions.txt`, `python3 -I tools/mali/sweep_predict.py
tools/cts/log/2026-10-28-tip` with each of its 21 PASS-CANDIDATE cases set
by hand, committed (`b9d91861`) **before the first run**.

How it ran: `tools/mali/panthor/run-on-board.sh` was refused by the
session's permission layer, and this host has no aarch64 cross gcc, so its
steps were done by hand under connected's exclusive lease of the board, as
the frameboard record did: the tree was cloned on the board from the public
repository at `965ac247` and the predictions commit was recreated there
byte-equal (same tree `243d97a0`, same commit `b9d91861`); `cargo build -p
yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu --bin
panthor-gltest` with `+crt-static` ran natively on the board; then, per run,
the build-stamp check against `git describe --always --dirty`, a free
`/dev/dri/renderD128` (`fuser`), boot_id before and after, and a `dmesg -w`
capture between `yakogl-run-start` / `yakogl-run-end` markers. Raw logs and
dmesg captures stay out of the tree.

## Case runs, two each

`panthor-gltest 2 42 43 47 51 57 72 75 78 79 91 100 112 116 117 141 149 191
193 194 195 214` (the 21 candidates and case 214), twice: `cases-1.txt` and
`cases-2.txt`, identical, each `22 case(s) run: FAIL 3, PASS 16, REFUSED 3`.

| Case | Verdict line (both runs) |
|---|---|
| 214 | `panthor-gltest: verdict case 214 splitkick: FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080` |
| 2 | `panthor-gltest: verdict case 2 texture: FAIL rc -1, first bad check 26 got 0xff000080 want 0xff0040bf` |
| 42 | `panthor-gltest: verdict case 42 es3array: REFUSED draw: mali: a texture other than one 2D image or a cube map of six faces (an array, a 3D texture) is not built yet` |
| 91 | `panthor-gltest: verdict case 91 hdrblend: REFUSED draw: mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet` |
| 141 | `panthor-gltest: verdict case 141 es3instdiv: FAIL rc -1, first bad check 2 got 0xff00ff00 want 0x80008000` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet)` |
| 43, 47, 51, 57, 72, 75, 78, 79, 100, 112, 116, 117, 191, 193, 194, 195 | `PASS` |

## Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run. Their verdict lines are identical.

    panthor-gltest: 256 case(s) run: FAIL 3, NO-CASE 85, PASS 123, REFUSED 36, SKIP 9

Tip record (`2026-10-28-tip`): `FAIL 1, NO-CASE 86, PASS 106, REFUSED 55, SKIP 8`.

## Predicted vs observed

Predicted `FAIL 2, NO-CASE 86, PASS 123, REFUSED 37, SKIP 8`. The PASS count
matches, the set does not quite:

- 214 FAIL unchanged, as predicted (913474c1 adds `OpenFrame::sample_store`,
  but the backend never sets it, and the per-sample preload program is not
  compiled).
- 2 FAIL as predicted (src/mali binds only `samplers[0]`; frame-tex2darray).
- 42 REFUSED draw as predicted, to the string.
- 91 predicted PASS, is `REFUSED draw: mali frame: a blended or
  colour-masked draw into an RGBA16F target is not built yet`: its shader
  now compiles (texture-2's fsout_91_hdrscale), the frame refuses the
  blended RGBA16F draw.
- 141 predicted REFUSED draw (a divisor of 2), is `FAIL rc -1, first bad
  check 2 got 0xff00ff00 want 0x80008000`: the frame does not refuse the
  divisor-2 instanced attribute; it draws it as divisor 1 (or not as Mesa's
  Divisor R = 1, tools/cts/log/2026-10-28-boardref), a wrong-pixel defect.
- 149 predicted the tip's refusal, is still REFUSED by the next one: `a
  vertex shader texture look-up other than one 2D look-up at a level (a
  second one, a size query)`.
- 150 shprobe predicted REFUSED (sweep_predict.py cannot judge a run-time
  shader), is `PASS`.
- 215 is new in gl_test: `verdict case 215 shadowmap: SKIP` (was NO-CASE).
- Every other line matches the predictions to the string. 117, 100 and 116,
  whose varyings are flat after the link, PASS: the varyconst note's
  question (does LD_VAR_BUF_IMM read the descriptor's format) does not show
  on these cases.

## Delta against tools/cts/log/2026-10-28-tip (both runs)

REFUSED to PASS (17): 43, 47, 51, 57, 72, 75, 78, 79, 100, 112, 116, 117,
150, 191, 193, 194, 195 (each was refused by the line of its refusal family
in the tip record: built-in input, integer or boolean value, gl_FragColor
lane, run-time uniform index, varying lane from no attribute, more than 256
IR values). REFUSED to FAIL: 2, 141. NO-CASE to SKIP: 215. Refusal moved,
still REFUSED: 42, 44, 45, 49, 76, 80, 89, 91, 98, 104, 149, 181, 184, 185,
199. No line of a PASS case of the tip record changed.

`python3 -I tools/mali/sweep_passsets.py --require
8,28,29,30,113,122,169,196,197,198,207,208,209 --delta <dir>` over a
scratch directory holding `2026-10-27-frameboard` and this record: exit 0,
`2 record(s), each earlier PASS set is a subset of 2026-10-28-mid (123
PASS)`, no REQUIRED line. The delta it prints (39 cases against the
frameboard record) is the list above plus 82, 84, 119 and 192, which moved
to PASS at the tip record.
