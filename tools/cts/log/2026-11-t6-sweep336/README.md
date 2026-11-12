<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 sweep336: plan #336's lowerings, cases x2, panthor-gltest --sweep x2

Plan #336, item board-sweep (#32, #33). Board: NanoPC-T6 (RK3588,
Mali-G610 MC4, panthor, kernel 6.18.55-current-rockchip64). boot_id
unchanged over every run below (`30f48a90-…`). Board clock 2026-11-12,
00:46:47 to 00:48:04 UTC (the four runs; the build before them took
2 min 27 s).

Code: origin/main `0decb5f7` (`0decb5f77fcac488c0b8e90e9a6a1b8e4c4d42e0`,
tree `7eadf62d`), with nothing on top: `predictions.txt` was committed
after it, before the runs, and is not part of the binary. `git describe
--always --dirty` on the board's clone: `v0.1.0-963-g0decb5f`. The binary
carried `yakogl-build=v0.1.0-963-g0decb5f`, sha256 `7d94179a…b054`
(`7d94179a97896754502f57a9a8bef0a873674252b15d32c50fa42a43b08fb054`). The
Mali changes since step4a are plan #336's lowerings: fs-transc
(`a8f6b066`, 77), fs-deriv (`59ed9d7c`, 138), fs-trunc (`7e17b2c1`, 199),
fs-fold (70), fs-fragcoord16 (`ed1440de`, 218), fs-indexlocal (`8eb4c6d4`,
80), vs-cmp-clamp (86, 182), vs-abs-if (`50a088da`, 181, 188), vs-lookups
(`b36b504f`, 101, 149), link-partial (19), frame-mrt55 and board-check
(`ce9cd45c`, 55), frame-3dlevels and frame-texfmt (206, 44, 91).

How it ran, as in step4a: this host has no aarch64 cross gcc, so
`tools/mali/panthor/run-on-board.sh` cannot build here. Its steps were done
by hand under connected's exclusive lease of the board: the commits from
`44f7b508` to `0decb5f7` were brought to a new clone on the board by a git
bundle (bundle sha256 checked at both ends, commit and tree ids checked),
`cargo build -p yakogl-mali-panthor --release --target
aarch64-unknown-linux-gnu --bin panthor-gltest` with `+crt-static` ran
natively on the board, then per run run-on-board.sh's run half: the
build-stamp check against `git describe --always --dirty`, a free
`/dev/dri/renderD128` (`fuser`), boot_id before and after, and a `dmesg -w`
capture between `yakogl-run-start` / `yakogl-run-end` markers, whose lines
were counted and passed through `tools/cts/mali-kmsg.py` (as on main). The
host-side board lock was not taken; the lease kept other tasks off the
board. Raw logs, dumps and dmesg captures stay out of the tree.

## 1. Case runs, two each

`panthor-gltest 19 44 55 70 77 80 86 91 101 138 149 181 182 188 199 206
218`, twice: `cases-1.txt` and `cases-2.txt`, identical, each `17 case(s)
run: FAIL 2, PASS 13, REFUSED 2`; 0 kernel messages between the markers, 0
after the end marker, `kmsg: resets=0 timeouts=0 mmu_faults=0 fw_faults=0
gpu_faults=0 cs_faults=0 other_panthor=0 first=-` for each.

| Case | Verdict line (both runs) |
|---|---|
| 19, 44, 55, 70, 80, 86, 91, 101, 138, 181, 182, 188, 199 | `PASS` |
| 77 | `panthor-gltest: verdict case 77 transc: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED draw: mali: a vertex shader that reads more than one texture is not built yet` |
| 206 | `panthor-gltest: verdict case 206 es3levels: FAIL rc -1, first bad check 2 got 0x04602020 want 0x04602090` |
| 218 | `panthor-gltest: verdict case 218 shadowcmp: FAIL rc -1, first bad check 3 got 0xff000000 want 0xffbfbfbf` |

## 2. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines
and the summary of each run, identical (`cmp` exits 0). 0 kernel messages
in either run, 0 after the end marker, `kmsg: resets=0 timeouts=0
mmu_faults=0 fw_faults=0 gpu_faults=0 cs_faults=0 other_panthor=0 first=-`
for each.

    panthor-gltest: 256 case(s) run: FAIL 2, NO-CASE 80, PASS 161, REFUSED 5, SKIP 8
    panthor-gltest: 256 case(s) run: FAIL 2, NO-CASE 80, PASS 161, REFUSED 5, SKIP 8

step4a (`2026-11-07-t6-step4a`): `FAIL 2, NO-CASE 83, PASS 147, REFUSED 15, SKIP 9`.

## 3. The cases that are not PASS

| Case | Verdict | Refusal or fault named | Predicted |
|---|---|---|---|
| 77 transc | REFUSED compile | fragment `Arithmetic` (exp2's `F32_TO_S32` rounding mode and fp16 `inversesqrt`, as `a8f6b066` says) | the same line |
| 106 mrt3 | REFUSED draw | `several colour targets in a frame that does not start from a whole colour clear are not built yet` (NOT_BUILT_TARGETS_PRELOAD) | `SKIP` |
| 149 vsunits | REFUSED draw | `a vertex shader that reads more than one texture is not built yet` (NOT_BUILT_VERTEX_TEXTURES); the program compiles, each of its draws is refused (GL error 0x502) | `PASS` |
| 163 drawbuf0 | REFUSED draw | `a draw whose first draw buffer is GL_NONE is not built yet` (NOT_BUILT_OUTPUT_SHIFT); the case's checks 1-3 pass (it accepts the refusal) | `SKIP` |
| 206 es3levels | FAIL | check 2 got `0x04602020` want `0x04602090`: levels 1 and 2 read level 0 (gl-3dlevels, src/gl/draw.rs, is not on this commit) | the same line |
| 218 shadowcmp | FAIL | check 3 got `0xff000000` want `0xffbfbfbf`: the program compiles and draws, no refusal, no fault; every compare that should give a non-zero result reads 0 (23 of the checks; those wanting `0xff000000` pass) | `PASS` |
| 221 instmix | REFUSED compile | vertex `a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet` | `NO-CASE` |

## 4. Predicted vs observed

Predicted `FAIL 1, NO-CASE 83, PASS 162, REFUSED 1, SKIP 9`
(`predictions.txt`). 249 of the 256 lines are as predicted; all 12 of the
tool's PASS-CANDIDATE cases and the hand-moved 55 PASS. Seven lines differ:

- 149 predicted `PASS`: the vertex shader's two look-ups now compile
  (`b36b504f`); the draw is refused next, in the frame builder (`REFUSED
  draw: mali: a vertex shader that reads more than one texture is not
  built yet`). A new cause.
- 218 predicted `PASS`: `FAIL rc -1, first bad check 3 got 0xff000000 want
  0xffbfbfbf`. The sampler2DShadow compare gives 0 wherever the case wants
  a lit texel (nearest and linear alike). A new cause.
- 106 mrt3 and 163 drawbuf0 predicted `SKIP`: the GL now offers several
  draw buffers on the Mali (`ce9cd45c`), so the cases run instead of
  skipping, and are refused by the names in section 3. New causes.
- 220 lodtime, 221 instmix, 222 dtpu predicted `NO-CASE`: cases added to
  gl_test since step4a. 220 `PASS`, 222 `SKIP`, 221 `REFUSED compile`
  (vertex ffma; docs/notes/mali-vs-fma.md's family).

## 5. Delta against tools/cts/log/2026-11-07-t6-step4a (both runs)

`python3 -I tools/mali/sweep_passsets.py --delta tools/cts/log`: exit 0,
`7 record(s), each earlier PASS set is a subset of 2026-11-t6-sweep336 (161
PASS)`. 20 cases differ; the prefix `panthor-gltest: verdict case <n>
<name>: ` is left out:

- to `PASS` from a refusal: 19 (link, partial varying), 44 and 91 (draw,
  RGBA16F sampling), 70 (constant product), 80 (indexed local), 86, 181,
  182, 188 (vertex arithmetic), 101 (vertex constant-lane look-up), 138,
  199 (fragment arithmetic)
- to `PASS` from `FAIL`: 55 (`first bad check -1`, the 0x500 attach)
- to `PASS` from `NO-CASE`: 220
- 149: `REFUSED compile: … a vertex shader texture look-up other than one
  2D look-up at a level …` -> `REFUSED draw: mali: a vertex shader that
  reads more than one texture is not built yet`
- 218: `REFUSED compile: … fp16 arithmetic on gl_FragCoord …` -> `FAIL rc
  -1, first bad check 3 got 0xff000000 want 0xffbfbfbf`
- 106, 163: `SKIP` -> the `REFUSED draw` lines of section 3
- 221: `NO-CASE` -> `REFUSED compile` (vertex ffma); 222: `NO-CASE` -> `SKIP`

No case leaves the PASS set; 14 join it.
