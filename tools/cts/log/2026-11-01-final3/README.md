<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 final3: case 214 and the new candidates x2, panthor-gltest --sweep x2

Plan #154, item full-gate-final (#31, #32, #33). Board: NanoPC-T6 (RK3588,
Mali-G610, panthor). boot_id unchanged over all four runs
(`5c5c0878-…`). Board clock 2026-11-01, 20:12 to 20:14 UTC.

This record replaces the unmerged final2 run (ddd5743b, at `d1b24389`, the
same verdict lines): its pull request also changed `tools/check.sh` and was
closed when its gate failed. This one is the board record alone, at the
current main.

Code: commit `8222fff1` (main `0ff58329` plus the predictions commit). The
binary carried `yakogl-build=v0.1.0-131-g8222fff`, sha256
`e40dcb5b…2987`. The one Mali code change since `d1b24389` is `606ce713`
(a vertex look-up at a coordinate with one constant lane, case 149 step A);
the others touch gc, v3d, pvr, docs, Mali testdata and the AM62P/i.MX8MP CTS
tools.

Predictions: `predictions.txt`, committed (`8222fff1`) **before the first
run**: `python3 -I tools/mali/sweep_predict.py tools/cts/log/2026-10-28-final`
names the same 6 PASS-CANDIDATE cases as at `d1b24389` (42, 45, 91, 98,
104, 149), so each line is the one final2 observed.

How it ran: this host has no aarch64 cross gcc, so
`tools/mali/panthor/run-on-board.sh` cannot build here (its first step,
`cargo build ... --target aarch64-unknown-linux-gnu` with
`aarch64-linux-gnu-gcc` as the linker). As in final2 its steps were done by
hand under connected's exclusive lease of the board: the commits from
`ab60b8c5` to `8222fff1` were recreated on the board's clone with the same
ids (each tree and commit object checked); `cargo build -p
yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu --bin
panthor-gltest` with `+crt-static` ran natively on the board; then, per run,
run-on-board.sh's run half: the build-stamp check against `git describe
--always --dirty`, a free `/dev/dri/renderD128` (`fuser`), boot_id before
and after, and a `dmesg -w` capture between `yakogl-run-start` /
`yakogl-run-end` markers. Raw logs and dmesg captures stay out of the tree.

## Case runs, two each

`panthor-gltest 2 42 45 91 98 104 149 166 214 216`, twice: `cases-1.txt`
and `cases-2.txt`, each `10 case(s) run: FAIL 1, FAULT 1, PASS 6, REFUSED
2`. They differ in one line, case 104's count of lost work (1, then 2). One
kernel message per run, case 104's `panthor fb000000.gpu: [drm] job
timeout: ... comm=panthor-gltest, seqno=2`; 0 after the end marker.

| Case | Verdict line (both runs) |
|---|---|
| 214 | `panthor-gltest: verdict case 214 splitkick: PASS` |
| 2, 42, 45, 166, 216 | `PASS` |
| 98 | `panthor-gltest: verdict case 98 uniloop: FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000` |
| 104 | `panthor-gltest: verdict case 104 divloop: FAULT work lost 1, group state/fatal queues 0x1/0x0, vm usable true` (run 1), `work lost 2` (run 2) |
| 91 | `panthor-gltest: verdict case 91 hdrblend: REFUSED draw: mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet)` |

## Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run. Their verdict lines are identical. One kernel
message per run (case 104's job timeout), 0 after the end marker.

    panthor-gltest: 256 case(s) run: FAIL 3, FAULT 1, NO-CASE 84, PASS 132, REFUSED 27, SKIP 9

Final record (`2026-10-28-final`): `FAIL 4, NO-CASE 85, PASS 126, REFUSED 32, SKIP 9`.

## Predicted vs observed

Predicted `FAIL 3, FAULT 1, NO-CASE 84, PASS 132, REFUSED 27, SKIP 9`, as
observed. One of the 256 lines differs, the risk the predictions named:

- 104 predicted `FAULT work lost 1, ...`, is `FAULT work lost 2,
  group state/fatal queues 0x1/0x0, vm usable true` in both sweeps.

## Delta against tools/cts/log/2026-10-28-final (both runs)

`python3 -I tools/mali/sweep_passsets.py --require
8,28,29,30,113,122,169,196,197,198,207,208,209 --delta tools/cts/log`: exit
0, `5 record(s), each earlier PASS set is a subset of 2026-11-01-final3 (132
PASS)` (frameboard 102, tip 106, mid 123, final 126), no REQUIRED line.
The delta, in the tool's words:

- case 2: `FAIL rc -1, first bad check 26 got 0xff000080 want 0xff0040bf` -> `PASS`
- case 42: `REFUSED draw: mali: a texture other than one 2D image or a cube map of six faces (an array, a 3D texture) is not built yet` -> `PASS`
- case 45: `REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a loop other than one a constant bound ends within 32 passes (a break behind a run-time condition, which Mesa unrolls with guards) is not lowered yet)` -> `PASS`
- case 98: the same loop line -> `FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000`
- case 104: the same loop line -> `FAULT work lost 2, group state/fatal queues 0x1/0x0, vm usable true`
- case 166: `REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a uniform past the FAU's 128 words (Mesa loads it from the UBO) is not lowered yet)` -> `PASS`
- case 214: `FAIL rc -1, first bad check 5 got 0xffc0c0c0 want 0xff808080` -> `PASS`
- case 216: `NO-CASE` -> `mipchains: PASS`

## Host checks at the same tip

- `python3 -I tools/mali/corpus_forward.py 381b0f09 HEAD`: `corpus-verdicts:
  10 commits, 139 ok -> 217 ok, 0 regressions`, exit 0;
  `test_corpus_forward.py`: 7 tests OK. The qt* exemption plan #154 (a)
  expected is not needed: the removal (c2538996) predates the root
  `381b0f09` (the tool's docstring says so).
- `tools/mali/corpus-verdicts.sh --write`: 324 verdicts (217 ok),
  `tools/mali/corpus-verdicts.txt` unchanged; `fvg_refusals.py`: `36 ok, 0
  texture refusal(s), 0 other`.
- `tools/mali/check.sh` (MESA_PAN_TREE set): `14 ok, 0 failed, 0 gap(s)`,
  `mali scope green`.
- `tools/check.sh --changed 0ff58329` (scopes docs, tools): `30 ok, 0
  failed, 1 gap(s)`. The gap is the glmark2 corpus row: the host's
  GLMARK2_SRC named no tree. Its steps were then run by hand on
  deps-env.sh's GLMARK2_SRC: run.sh for powervr and vivante (no stale depth
  texture, no GL error, 0 of 33 GC entries above 3/4 of the carve-out) and
  `tests/glmark2_corpus.rs` for powervr, gc7000 and v3d, each `69 shaders,
  69 compiled, 0 failed; 33 entries, 93 pairs tried, 69 linked`.
- The full `tools/check.sh` was not run: this host's environment names
  stale ZEPHYR_WS, HAL_NXP_DIR and MESA_V3D_TREE values and the session
  could not override them.
