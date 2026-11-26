<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 sweep at main d9efa418: panthor-gltest --sweep x2

Board: NanoPC-T6 (RK3588, Mali-G610 MC4, panthor, kernel
6.18.55-current-rockchip64), the board's own Armbian, not Zephyr. boot_id
unchanged over both runs (`ad1ec39d-…`): before `ad1ec39d-d5a1-4be3-a289-3cf5fcb59d6d`,
after `ad1ec39d-d5a1-4be3-a289-3cf5fcb59d6d` (each run's own before/after
and `tools/cts/mali-board-state.sh` around both). 2026-11-26, about 00:02
UTC (run 1) and 00:03 UTC (run 2); the board had been up since 2026-11-24.

Code: origin/main `d9efa418`, nothing on top, in a clean tree (`git describe
--always --dirty` gives `d9efa418`). The binary carried
`yakogl-build=d9efa418` and printed `panthor-gltest: build d9efa418`.
Reference record: `2026-11-t6-sweep-1a22bd38` (main `1a22bd38e`, 2026-11-12).

How it ran: `tools/mali/panthor/run-on-board.sh panthor-gltest --sweep`
twice, with the cross toolchain of `tools/cts/deps-env.sh --build --board
nanopc_t6 MALI_DEQP_TOOLCHAIN` first on PATH. The script took the board
lock, cross-built the static aarch64 binary, checked the build stamp, read
boot_id before and after, and captured `dmesg -w` between its run markers.
`tools/cts/mali-board-state.sh snapshot` before the first run and after the
second, then `compare`: /boot unchanged (419 files), modules unchanged (100),
U-Boot env not readable (not compared), boot_id unchanged. Raw logs and
dmesg captures stay out of the tree (records/nanopc-t6/).

Kernel lines: 0 kernel messages between the run markers and 0 in the 1 s
after the end marker, in both runs. The boot's `dmesg | grep -i panthor`
was the same before and after apart from the four run markers: the 8
probe lines of the boot (firmware, `Initialized panthor 1.5.0`), no fault,
reset or timeout line.

## 1. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines
and the summary of each run, identical (`cmp` exits 0; sha256
`bddbd1beb358620a8858fe7eac11899614186b6efaefd99dbfbfd41d0d2a3a35`).

    panthor-gltest: 256 case(s) run: FAIL 1, NO-CASE 53, PASS 185, REFUSED 9, SKIP 8
    panthor-gltest: 256 case(s) run: FAIL 1, NO-CASE 53, PASS 185, REFUSED 9, SKIP 8

Tally per verdict class, against 1a22bd38 (`FAIL 3, NO-CASE 73, PASS 165,
REFUSED 6, SKIP 9`):

| Class | 1a22bd38 | d9efa418 | Delta |
|---|---|---|---|
| PASS | 165 | 185 | +20 |
| REFUSED | 6 | 9 | +3 |
| FAIL | 3 | 1 | -2 |
| SKIP | 9 | 8 | -1 |
| NO-CASE | 73 | 53 | -20 |
| FAULT | 0 | 0 | 0 |

## 2. The cases that are not PASS, NO-CASE or SKIP

| Case | Verdict | Refusal or failure named | Since 1a22bd38 |
|---|---|---|---|
| 77 transc | REFUSED compile | fragment `arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | unchanged |
| 106 mrt3 | REFUSED draw | `several colour targets in a frame that does not start from a whole colour clear are not built yet` | unchanged |
| 128 uniidxj | REFUSED compile | fragment `a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet` | was PASS |
| 163 drawbuf0 | REFUSED draw | `a draw whose first draw buffer is GL_NONE is not built yet` | unchanged |
| 226 negapi_uniformui, 227 negapi_uniformuiv | REFUSED compile | fragment `a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet` | unchanged |
| 228 negapi_readpix_fmt | FAIL | `rc -1, first bad check -1 got 0x00000000 want 0x00000000` | unchanged |
| 241 es3_precision_int, 242 es3_precision_uint | REFUSED compile | fragment `an integer colour output other than one declared highp uvec4 (an ivec4, int or uint, fewer lanes, or a uvec4 mediump or highp only by precision highp int) is not lowered yet` | new case (was NO-CASE) |
| 245 mrt4 | REFUSED draw | `a colour clear of several targets other than one whole clear of every target before any draw is not built yet` | new case (was NO-CASE) |

SKIP, 8 cases: 97 varyings15, 200 intrt, 201 usamp, 222 dtpu, 223 msaarbo,
244 w128tex, 246 tsprobe, 247 dlayout. FAULT: none.

## 3. Per-case diff against 2026-11-t6-sweep-1a22bd38 (both runs)

By hand (`diff` of the two verdict files; `tools/cts/mali-sweep-tally.py`
is not in this tree yet). 28 cases differ; the other 228 verdict lines are
byte-identical.

| Case | 1a22bd38 | d9efa418 |
|---|---|---|
| 128 uniidxj | PASS | REFUSED compile |
| 149 vsunits | REFUSED draw | PASS |
| 177 zonly | SKIP | PASS |
| 178 heldfree | SKIP | PASS |
| 179 retile | SKIP | PASS |
| 206 es3levels | FAIL (check 2 got `0x04602020` want `0x04602090`) | PASS |
| 215 shadowmap | SKIP | PASS |
| 229 tf_deleted_output | FAIL | PASS |
| 230 flat_lines | NO-CASE | PASS |
| 231 flat_lines_wide | NO-CASE | PASS |
| 232 flat_line_strip | NO-CASE | PASS |
| 233 flat_line_strip_wide | NO-CASE | PASS |
| 234 flat_line_loop | NO-CASE | PASS |
| 235 flat_line_loop_wide | NO-CASE | PASS |
| 236 flat_tri_strip | NO-CASE | PASS |
| 237 flat_tri_fan | NO-CASE | PASS |
| 238 drawrandom_packed | NO-CASE | PASS |
| 239 drawrandom_intfloat | NO-CASE | PASS |
| 240 drawrandom_intw | NO-CASE | PASS |
| 241 es3_precision_int | NO-CASE | REFUSED compile |
| 242 es3_precision_uint | NO-CASE | REFUSED compile |
| 243 divmix | NO-CASE | PASS |
| 244 w128tex | NO-CASE | SKIP |
| 245 mrt4 | NO-CASE | REFUSED draw |
| 246 tsprobe | NO-CASE | SKIP |
| 247 dlayout | NO-CASE | SKIP |
| 248 tearbars | NO-CASE | PASS |
| 249 linewidth | NO-CASE | PASS |

(Cases 177 and 178 were named `?` in the 1a22bd38 lines; the names above
are this record's.) 21 cases join PASS, 1 leaves it (128).

## 4. Against the forecast

- 149, 206, 229, 177, 178, 179, 215 forecast to change: all seven did, each
  to PASS. As forecast.
- 230 to 249 forecast to be new cases: all twenty now run (NO-CASE 73 to 53,
  as forecast). Of them 15 PASS, 3 SKIP (244, 246, 247) and 3 are REFUSED
  (241, 242 compile, 245 draw); the earlier read-only session at this
  commit also saw 241, 242 and 245 as non-PASS.
- 128 forecast possibly to become REFUSED: it did (REFUSED compile, the
  fragment varying load or conversion refusal), leaving the PASS set, as
  the earlier session at this commit saw.
- 77, 106, 163, 226, 227 and 228: no change forecast, none seen (the same
  verdict lines byte for byte).
- No change outside the forecast.
