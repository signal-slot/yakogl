<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 tip: panthor-gltest --sweep x2 at the integration tip

Plan #154, item board-tip (#32). Board: NanoPC-T6 (RK3588, Mali-G610,
panthor). boot_id unchanged over both runs; 0 kernel messages between the
run markers and 0 in the second after them, in both runs.

Code: commit `8c40d514` (the integration tip `c5b55ee2` plus the predictions
commit). The binary carried `yakogl-build=8c40d514` (the runner refuses a
binary whose build differs from the tree).

Predictions: `predictions.txt` was produced with
`python3 -I tools/mali/sweep_predict.py` from the frameboard record and the
compiler's verdicts at the tip, and committed (`8c40d514`) **before the first
run**. It holds 6 PASS-CANDIDATE cases (82, 84, 119, 149, 192, 194) on top of
the 102 PASS of the frameboard record.

Build and run, once per sweep, only through the board runner (the lock was
free; it was never waited on):

    BOARD_HOST=<user@board> CARGO_TARGET_DIR=<scratch> \
      tools/mali/panthor/run-on-board.sh panthor-gltest --sweep

(cross build for aarch64, static). Raw logs and dmesg captures stay out of
the tree. The runner exits 1 (a FAIL and REFUSED cases are in the sweep).

## Counts

Both runs: `256 case(s) run: FAIL 1, NO-CASE 86, PASS 106, REFUSED 55, SKIP 8`.
`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt` are identical.
Frameboard record: `FAIL 1, NO-CASE 86, PASS 102, REFUSED 59, SKIP 8`.

## Predicted vs observed

- 4 of the 6 candidates PASS in both runs: 82 fragcentre, 84 fragcoord,
  119 pbufup, 192 predlive.
- 149 vsunits and 194 offarmn stayed REFUSED: 149 `a vertex shader look-up
  at a coordinate with a constant lane`, 194 `a shader of more than 256 IR
  values`. They were candidates only; the board's online compile refuses
  them. Whether the host's per-source verdicts differ is not examined here.
- Everything else matches the predictions. Case 214 splitkick FAILs as
  expected (`first bad check 5 got 0xffc0c0c0 want 0xff808080`).

## Delta against tools/cts/log/2026-10-27-frameboard (both runs)

Only these four verdicts changed (old line, then new line):

    panthor-gltest: verdict case 82 fragcentre: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: mediump arithmetic on gl_FragCoord (a read not first copied to a highp variable) is not lowered yet)
    panthor-gltest: verdict case 82 fragcentre: PASS 
    panthor-gltest: verdict case 84 fragcoord: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: mediump arithmetic on gl_FragCoord (a read not first copied to a highp variable) is not lowered yet)
    panthor-gltest: verdict case 84 fragcoord: PASS 
    panthor-gltest: verdict case 119 pbufup: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: mediump arithmetic on gl_FragCoord (a read not first copied to a highp variable) is not lowered yet)
    panthor-gltest: verdict case 119 pbufup: PASS 
    panthor-gltest: verdict case 192 predlive: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet)
    panthor-gltest: verdict case 192 predlive: PASS 

(Case 0 differs only by a trailing space the runner prints after `PASS`.)

All 102 PASS cases of the frameboard record PASS in both runs; the PASS set
is the frameboard's 102 plus 82, 84, 119 and 192. Cases 8, 28, 29, 30, 113,
122, 169, 196, 197, 198, 207, 208 and 209 are PASS in both runs.
