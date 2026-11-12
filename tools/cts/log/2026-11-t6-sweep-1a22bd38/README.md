<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 sweep at main 1a22bd38e: panthor-gltest --sweep x2

Board: NanoPC-T6 (RK3588, Mali-G610 MC4, panthor, kernel
6.18.55-current-rockchip64), the board's own Linux, not Zephyr. boot_id
unchanged over both runs (`5e2a981d-…`). 2026-11-12, about 03:52 UTC.

Code: origin/main `1a22bd38e`, nothing on top. The binary carried
`yakogl-build=v0.1.0-1005-g1a22bd38e`. The Mali changes since sweep336
(`0decb5f7`) that the sweep reaches: the sampler2DShadow compare (case 218)
and the vertex fused multiply-add of a divisor mix (case 221), both
`68b9cdf26` and the three commits before it; gl_test's new cases 223 to
229.

How it ran: `tools/mali/panthor/run-on-board.sh panthor-gltest --sweep`
from a detached checkout of that commit on the developer PC, twice. The
script took the board lock, cross-built the static aarch64 binary, checked
the build stamp, read boot_id before and after, and captured `dmesg -w`
between its run markers. Raw logs and dmesg captures stay out of the tree.

## 1. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines
and the summary of each run, identical (`cmp` exits 0). 0 kernel messages
between the markers and 0 in the 1 s after the end marker, in both runs.

    panthor-gltest: 256 case(s) run: FAIL 3, NO-CASE 73, PASS 165, REFUSED 6, SKIP 9
    panthor-gltest: 256 case(s) run: FAIL 3, NO-CASE 73, PASS 165, REFUSED 6, SKIP 9

sweep336 (`2026-11-t6-sweep336`): `FAIL 2, NO-CASE 80, PASS 161, REFUSED 5, SKIP 8`.

Cases 218 and 221 also ran on their own before the sweeps, from the same
binary: `2 case(s) run: PASS 2`, 0 kernel messages.

## 2. The cases that are not PASS

| Case | Verdict | Refusal or fault named | Issue |
|---|---|---|---|
| 77 transc | REFUSED compile | fragment `arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | #33 |
| 106 mrt3 | REFUSED draw | `several colour targets in a frame that does not start from a whole colour clear are not built yet` | #33 |
| 149 vsunits | REFUSED draw | `a vertex shader that reads more than one texture is not built yet` | #32 |
| 163 drawbuf0 | REFUSED draw | `a draw whose first draw buffer is GL_NONE is not built yet`; the case's checks pass (it accepts the refusal) | #33 |
| 206 es3levels | FAIL | check 2 got `0x04602020` want `0x04602090`: levels 1 and 2 of the 3D texture read level 0 | #4350 |
| 226 negapi_uniformui, 227 negapi_uniformuiv | REFUSED compile | `a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet` | #4496 |
| 228 negapi_readpix_fmt | FAIL | `first bad check -1`: `GL_RGBA32I` and `GL_RGBA32UI` textures and `glRenderbufferStorageMultisample` give GL error 0x502 (the back end has no integer texture class and no multisample renderbuffer) | #4496 |
| 229 tf_deleted_output | FAIL | `first bad check -1`: `new X byte 0 is 0x00, want 0xab`, then `glBufferSubData` 0x502 on the new buffer | #4496, #3889 |

SKIP, 9 cases: 97, 177, 178, 179, 200, 201, 215, 222, 223.

## 3. Delta against tools/cts/log/2026-11-t6-sweep336 (both runs)

`python3 -I tools/mali/sweep_passsets.py --delta tools/cts/log`: exit 0,
each earlier PASS set is a subset of this record's 165. Nine cases differ:

- to `PASS` from `FAIL`: 218 shadowcmp
- to `PASS` from `REFUSED compile`: 221 instmix
- to `PASS` from `NO-CASE`: 224, 225
- `NO-CASE` -> `SKIP`: 223
- `NO-CASE` -> `REFUSED compile`: 226, 227
- `NO-CASE` -> `FAIL`: 228, 229

No case leaves the PASS set; 4 join it.
