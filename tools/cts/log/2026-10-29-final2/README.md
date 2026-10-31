<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 final2: case 214 and the new candidates x2, panthor-gltest --sweep x2

Plan #154, item full-gate-final (#31, #32, #33). Board: NanoPC-T6 (RK3588,
Mali-G610, panthor). boot_id unchanged over all four runs
(`5c5c0878-…`). Board clock 2026-10-31, 09:26 to 09:27 UTC (the record keeps
the plan's directory name).

Code: commit `35ffb8a9` (the integration tip `d1b24389`, after cubebias101,
loops-rt, frame-tex2darray and ubo166, plus the predictions commit). The
binary carried `yakogl-build=v0.1.0-109-g35ffb8a`, sha256
`5c4c9534…2664`.

Predictions: `predictions.txt`, `python3 -I tools/mali/sweep_predict.py
tools/cts/log/2026-10-28-final` with its 6 PASS-CANDIDATE cases and 2, 166
and 214 set by hand, committed (`35ffb8a9`) **before the first run**.

How it ran: as board-final did. `tools/mali/panthor/run-on-board.sh
panthor-gltest 214` took the board lock and stopped in its build
(`compile gl_test.c: "aarch64-linux-gnu-gcc" ... could not run: No such file
or directory`): this host has no aarch64 cross gcc. Its steps were done by
hand under connected's exclusive lease of the board: the public repository
was cloned on the board at `d1b24389` and the predictions commit recreated
there byte-equal (same commit `35ffb8a9`); `cargo build -p
yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu --bin
panthor-gltest` with `+crt-static` ran natively on the board; then, per run,
the build-stamp check against `git describe --always --dirty`, a free
`/dev/dri/renderD128` (`fuser`), boot_id before and after, and a `dmesg -w`
capture between `yakogl-run-start` / `yakogl-run-end` markers (board-final's
run half, pointed at this clone). Raw logs and dmesg captures stay out of
the tree.

## Case runs, two each

`panthor-gltest 2 42 45 91 98 104 149 166 214` (the candidates, 2, 166 and
214), twice: `cases-1.txt` and `cases-2.txt`, identical, each `9 case(s)
run: FAIL 1, FAULT 1, PASS 5, REFUSED 2`. One kernel message per run,
case 104's `panthor fb000000.gpu: [drm] job timeout: ... comm=panthor-gltest,
seqno=2`; 0 in the second after the end marker.

| Case | Verdict line (both runs) |
|---|---|
| 214 | `panthor-gltest: verdict case 214 splitkick: PASS` |
| 2, 42, 45, 166 | `PASS` |
| 98 | `panthor-gltest: verdict case 98 uniloop: FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000` |
| 104 | `panthor-gltest: verdict case 104 divloop: FAULT work lost 2, group state/fatal queues 0x1/0x0, vm usable true` |
| 91 | `panthor-gltest: verdict case 91 hdrblend: REFUSED draw: mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet)` |

## Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run. Their verdict lines are identical. One kernel
message per run (case 104's job timeout), 0 after the end marker.

    panthor-gltest: 256 case(s) run: FAIL 3, FAULT 1, NO-CASE 84, PASS 132, REFUSED 27, SKIP 9

Final record (`2026-10-28-final`): `FAIL 4, NO-CASE 85, PASS 126, REFUSED 32, SKIP 9`.

In the sweep, case 104 reads `FAULT work lost 1` (2 in the 9-case runs);
the cases after it run as before (`vm usable true`).

## Predicted vs observed

Predicted `FAIL 2, NO-CASE 85, PASS 133, REFUSED 27, SKIP 9`. Three lines
differ, every other of the 256 matches to the string:

- 98 predicted `PASS`, is `FAIL rc -1, first bad check 4 got 0xfff00010
  want 0xfff00000`.
- 104 predicted `PASS`, is `FAULT work lost 1, group state/fatal queues
  0x1/0x0, vm usable true`.
- 216 predicted `NO-CASE`, is `mipchains: PASS` (a gl_test case added
  after the final record; the predictor keeps the last line).

## Delta against tools/cts/log/2026-10-28-final (both runs)

`python3 -I tools/mali/sweep_passsets.py --require
8,28,29,30,113,122,169,196,197,198,207,208,209 --delta tools/cts/log`: exit
0, `5 record(s), each earlier PASS set is a subset of 2026-10-29-final2 (132
PASS)` (frameboard 102, tip 106, mid 123, final 126), no REQUIRED line.
The delta, in the tool's words:

- case 2: `FAIL rc -1, first bad check 26 got 0xff000080 want 0xff0040bf` -> `PASS`
- case 42: `REFUSED draw: mali: a texture other than one 2D image or a cube map of six faces (an array, a 3D texture) is not built yet` -> `PASS`
- case 45: `REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a loop other than one a constant bound ends within 32 passes (a break behind a run-time condition, which Mesa unrolls with guards) is not lowered yet)` -> `PASS`
- case 98: the same loop line -> `FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000`
- case 104: the same loop line -> `FAULT work lost 1, group state/fatal queues 0x1/0x0, vm usable true`
- case 166: `REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a uniform past the FAU's 128 words (Mesa loads it from the UBO) is not lowered yet)` -> `PASS`
- case 214: `FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080` -> `PASS`
- case 216: `NO-CASE` -> `mipchains: PASS`

## Host checks at the same tip

- `python3 -I tools/mali/corpus_forward.py 381b0f09… HEAD`: `corpus-verdicts:
  10 commits, 139 ok -> 217 ok, 0 regressions`, exit 0;
  `test_corpus_forward.py`: 7 tests OK.
- `tools/mali/corpus-verdicts.sh --write`: 324 verdicts, `git diff
  --exit-code tools/mali/corpus-verdicts.txt` clean;
  `fvg_refusals.py`: `36 ok, 0 texture refusal(s), 0 other`.
- `MESA_PAN_TREE=… tools/mali/check.sh`: `14 ok, 0 failed, 0 gap(s)`,
  `mali scope green`; the Mali test row passed its 335 (`MALI_TESTS=335`).
- The full `tools/check.sh` (every row, `--jobs 2`, a fresh
  `CARGO_TARGET_DIR`; ZEPHYR_WS, HAL_NXP_DIR (the workspace's
  modules/hal/nxp), PCO_SHC, GLMARK2_SRC, MALI_CSFFW (fetched with
  `tools/cts/deps-env.sh --build MALI_CSFFW`, sha256 `a27847ea…405b`) and
  MESA_PAN_TREE under the deps directory; `driver/src/fw/pvr_fw_blob.h`
  copied in; MESA_V3D_TREE and CC_aarch64_unknown_none unset), 2026-10-31,
  ran to its end in 5195 s:

      check.sh: 207 ok, 2 failed, 0 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 not run (passed on main) in 5195s; test binaries: 1185 run, 0 skipped (passed on main)

  So the gate is **not** green. The two FAIL rows are `run pvr
  rgxlayer_check` and `run pvr km_check`: linking `libyakogl_drivers.a`
  (host, `panic = "abort"`) with `cc` fails on `undefined reference to
  '__rustc::rust_eh_personality'`, which this host's nightly (`rustc
  1.101.0-nightly (5d8fec066 2026-10-16)`) references from `core` and
  `compiler_builtins` and which nothing defines (`nm`: only `U`/`V`
  entries). Neither row is a Mali row; the fix is a new item outside this
  record. An earlier attempt on this host that did not finish also failed
  three aarch64-unknown-none rows because a preset `CC_aarch64_unknown_none`
  named a directory that does not exist; with it unset they pass.
