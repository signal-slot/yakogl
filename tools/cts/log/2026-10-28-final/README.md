<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 final: case 214 and the chains cases x2, panthor-gltest --sweep x2

Plan #154, item board-final (#32, #33). Board: NanoPC-T6 (RK3588, Mali-G610,
panthor). boot_id unchanged over all four runs; 0 kernel messages between
the run markers and 0 in the second after them, in every run. Board clock
2026-10-28, 13:34 to 13:35 UTC.

Code: commit `b081baf1` (the integration tip `b3f6bf1c`, after board-mid and
chains, plus the predictions commit). Since board-mid's `788d5ae7` only
chains (`b3f6bf1c`, src/shaderc/online/mali) and pre-study documents
merged; src/mali did not move. The binary carried
`yakogl-build=v0.1.0-74-gb081baf`, sha256 `493c4908…2f5c`.

Predictions: `predictions.txt`, `python3 -I tools/mali/sweep_predict.py
tools/cts/log/2026-10-28-mid` with each of its 7 PASS-CANDIDATE cases set by
hand, committed (`b081baf1`) **before the first run**.

How it ran: as board-mid did. `tools/mali/panthor/run-on-board.sh` was
first refused by the session's permission layer; once allowed, it took the
board lock and stopped in its build (`compile gl_test.c:
"aarch64-linux-gnu-gcc" ... could not run: No such file or directory`):
this host has no aarch64 cross gcc. Its steps were done by hand under
connected's exclusive lease of the board: the tree was cloned on the board from the public repository at
`b3f6bf1c` and the predictions commit was recreated there byte-equal (same
tree `69b5c9c3`, same commit `b081baf1`); `cargo build -p
yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu --bin
panthor-gltest` with `+crt-static` ran natively on the board; then, per run,
the build-stamp check against `git describe --always --dirty`, a free
`/dev/dri/renderD128` (`fuser`), boot_id before and after, and a `dmesg -w`
capture between `yakogl-run-start` / `yakogl-run-end` markers (board-mid's
run half, pointed at this clone). Raw logs and dmesg captures stay out of
the tree.

## Case runs, two each

`panthor-gltest 42 63 91 149 184 185 189 214` (the 7 candidates and case
214), twice: `cases-1.txt` and `cases-2.txt`, identical, each `8 case(s)
run: FAIL 2, PASS 3, REFUSED 3`.

| Case | Verdict line (both runs) |
|---|---|
| 214 | `panthor-gltest: verdict case 214 splitkick: FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080` |
| 63 | `panthor-gltest: verdict case 63 matattr: FAIL rc -1, first bad check 1 got 0xff800000 want 0xff804000` |
| 42 | `panthor-gltest: verdict case 42 es3array: REFUSED draw: mali: a texture other than one 2D image or a cube map of six faces (an array, a 3D texture) is not built yet` |
| 91 | `panthor-gltest: verdict case 91 hdrblend: REFUSED draw: mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet)` |
| 184, 185, 189 | `PASS` |

Case 63's six checks, both runs: mat2 `0xff800000`, mat3 `0xff000000`,
mat4 `0x00000000`, each at (32,32) and (2,2), want `0xff804000`. The colour
is the matrix's first row (`a_xf[i][0]`), so every column after the first
reads 0: mat2 loses green, mat3 green and blue, mat4 all four. Its vertex
programs are byte-equal to the drm-shim references (varyconst_63_matattr,
chain_matattr3, chain_matattr4), so the defect is expected on the frame
side (the attribute descriptors of a matrix attribute's columns 1..n); not
yet checked against a Mesa board reference.

## Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run. Their verdict lines are identical.

    panthor-gltest: 256 case(s) run: FAIL 4, NO-CASE 85, PASS 126, REFUSED 32, SKIP 9

Mid record (`2026-10-28-mid`): `FAIL 3, NO-CASE 85, PASS 123, REFUSED 36, SKIP 9`.

## Predicted vs observed

Predicted `FAIL 3, NO-CASE 85, PASS 127, REFUSED 32, SKIP 9`. One line
differs, every other of the 256 matches to the string:

- 63 predicted `PASS`, is `FAIL rc -1, first bad check 1 got 0xff800000
  want 0xff804000` (above).
- 184, 185, 189 PASS as predicted; 42, 91, 149 keep board-mid's line as
  predicted; 214 FAIL unchanged as predicted (no src/mali commit since
  board-mid); 2 and 141 FAIL unchanged; 80 still REFUSED by the chain line.

## Delta against tools/cts/log/2026-10-28-mid (both runs)

REFUSED to PASS (3): 184 `depchain`, 185 `kindcost`, 189 `mulone`. REFUSED
to FAIL: 63 `matattr`. No other line changed.

## PASS-set check

`python3 -I tools/mali/sweep_passsets.py --require
8,28,29,30,113,122,169,196,197,198,207,208,209 --delta <dir>` over a scratch
directory holding `2026-10-27-frameboard`, `2026-10-28-mid` and this record:
exit 0, `3 record(s), each earlier PASS set is a subset of 2026-10-28-final
(126 PASS)`, no REQUIRED line. The two 2026-10-28 records share a date, and
the tool orders such records by the commit that added them, which a scratch
copy has none of; the scratch directory was therefore made a git repository
whose commits add frameboard, mid and final in the tree's order. Over
`2026-10-27-frameboard` and this record alone: exit 0, 41 cases differ:
the 39 of board-mid's README, with 184 and 185 now REFUSED to PASS instead
of a moved refusal, plus 189 (REFUSED to PASS) and 63 (REFUSED to FAIL).
