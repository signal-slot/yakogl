<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 step4a: the step-4 lowerings, Mesa 26.2.1 references, cases x2, panthor-gltest --sweep x2

Plan #309, item fix-matattr-63 (#32, #33). Board: NanoPC-T6 (RK3588,
Mali-G610 MC4, panthor, kernel 6.18.55-current-rockchip64). boot_id
unchanged over every run below (`5c5c0878-…`). Board clock 2026-11-07,
15:10 to 15:26 UTC.

Code: commit `e8199bf4` (the integration tip `44f7b508` plus the
predictions commit). The binary carried `yakogl-build=v0.1.0-479-ge8199bf`,
sha256 `89288bba…55cb`. The Mali changes since the final3 record are the
step-4 items of plan #309: `65ea7f9c` (one attribute record per matrix
column, 63), `87e9eded` (80's chain), `4b9da225` (power-of-two instance
divisors, 141), `3e7a589f` (the kept-loop pass counter, 98 and 104),
`f4ab97c3` (81's discard), `1b65b592` (mixed fp32/fp16, 44 and 49),
`df7320df` and `60c8b6e5` (branches and flattened ifs, 68, 71, 74, 142,
148), `9adc6cab` (vertex integer conversions), `f93fece8` (fragment outputs
past location 0, 55), `e75b0f4d` (RGBA16F blends, 91), `93f9c958` (3D
textures, one level, 48 and 206), `31ee39a6` (bool mask ops, 76),
`5eebce03` (89's flat u2f motion) and `27fed39b` (size, 3D and bias
look-ups, 46, 48, 206, 101).

How it ran, as in final3: this host has no aarch64 cross gcc, so
`tools/mali/panthor/run-on-board.sh` cannot build here. Its steps were done
by hand under connected's exclusive lease of the board: the commits from
`0ff58329` to `e8199bf4` were brought to the board's clone by a git bundle
(commit and tree ids checked), `cargo build -p yakogl-mali-panthor
--release --target aarch64-unknown-linux-gnu --bin panthor-gltest` with
`+crt-static` ran natively on the board, then per run run-on-board.sh's run
half: the build-stamp check against `git describe --always --dirty`, a free
`/dev/dri/renderD128` (`fuser`), boot_id before and after, and a `dmesg -w`
capture between `yakogl-run-start` / `yakogl-run-end` markers. The
host-side board lock was not taken; the lease kept other tasks off the
board. Raw logs, dumps and dmesg captures stay out of the tree.

## 1. Mesa 26.2.1 references on the board

`tools/mali/mesa-board-ref.sh`'s steps by hand, as in 2026-10-28-boardref
(the same reason): `gcc -O1 -Wall -Werror ... -lEGL -lGLESv2` on the board,
`PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync`, `--libdir
/root/mesa-26.2.1-build/inst/lib` (with the disk shader cache off), each
program twice. Every run: rc 0, `verdict <name>: PASS`, 0 kernel messages,
the two runs' stdout equal, and every descriptor word read below equal
between the two runs.

| program | case | sha256 | draws | stdout sha256 |
|---|---|---|---|---|
| `es3instdiv.c` (2026-10-28-boardref's, unchanged) | 141 | `8bd1ca34…0d10` | 2 frames, divisor 2, arrays then elements | `cbdc30f4…52c3` (as in boardref) |
| `matattr.c` | 63 | `03a24c14…b7fa` | 3 frames: mat2, mat3, mat4, checks 1-6 | `5cc9fd30…1886` |
| `es3mrt.c` | 55 | `3dfa3d6d…d20e` | 1 frame, two RGBA8 targets, checks 1-2 | `9cd701bc…9da7` |
| `hdrblend.c` | 91 | `4a0d1eda…5532` | 3 frames: two scissored blends into 8x8 RGBA16F, the x0.25 read-back, the uploaded texel; checks 1-7 | `1b13e4b8…7883` |

The programs are gl_test's cases on their own, with two departures the
first runs showed were needed (those first runs failed and are not the
record):

- `matattr.c` binds `a_pos` to 0 and `a_xf` to 1 before the link: Mesa's
  linker put `a_xf` at 0 and every check read `0x00000000`. The case
  relies on location 1 (gl_test checks `glGetAttribLocation(a_xf) == 1`).
- `es3mrt.c` calls `glDrawBuffers({COLOR_ATTACHMENT0, COLOR_ATTACHMENT1})`:
  `case_es3mrt` never does, so `GL_DRAW_BUFFER1` stays `GL_NONE` (ES 3.0
  4.2.1) and Mesa wrote attachment 0 only (check 2 read `0x00000000`). The
  case as written depends on a non-conformant default.

## 2. Our descriptors pinned to the dumps

Each dump is read at the memory block after the job. "Ours" is what the
pinned host tests build or assert (`src/mali/*_tests.rs`, the compiler's
`tests.rs` and the expect.json references); every differing byte is named.

**141 es3instdiv** (`frame_instance_tests::a_divisor_of_two_is_case_141_s`):
the three Attribute descriptors are the test's 24 words exactly
(`2fc00115 00000002 0 0 10 …`, `2bc04025 01000042 0 1 18 …`, `2fc00025
01000042 0 2 18 …`); the three Buffers `0x19` with sizes `0x60`, `0x30`,
`0x28`, the third at the second's address + 8, as the test's; `MOVE32 r34,
#0x4` (INSTANCE_COUNT). 0 differing bytes but the buffers' addresses.

**63 matattr**: `pan_ref_diff.py diff-ref <frame> <expect.json>` of each
draw against the in-tree references whose sources are byte copies of the
case's (mat2 `varyconst_63_matattr`, mat3 `chain_matattr3`, mat4
`other_63_matattr`): `vs.position`, `vs.varying` and `fs.fragment` are
byte-equal in all three (8/13/20 varying instructions, `secondary_offset`
256/256/384), with work registers and preload equal. What differs is
draw-time FAU state, not code: word 2 of both vertex programs' FAU
(`c2000000` -32.0 on the board's window, `42000000` +32.0 in the
reference, the viewport's y scale) and, in the mat3 draw, fragment FAU word
5 (`fffd0400` against 0, the 64-bit slot's unused half that
2026-10-28-boardref section 4 describes). The Attribute records are 3, 4
and 5 a draw, one per column location, `RGBA32F`/`RG32F RGB1`/`RGB32F RGB1`
as the compiler records them (`other_63_matattr_is_byte_equal_to_mesa_26_2_1`).
The named difference (docs/notes/mali-builtin-inputs.md, frame work for
#32): for column c >= 1 Mesa writes Attribute `Offset` (bytes 8-11) 0 and
folds the column offset into its Buffer (`Address` bytes 8-11 base +
offset, `Size` bytes 4-7 the store less the offset, e.g. mat2 `0x70 @
+0x10`, `0x68 @ +0x18`; mat4 `0x130 @ +0x10` … `0x100 @ +0x40`); ours keeps
the offset in the Attribute's bytes 8-11 (0x10, 0x18; 0x10, 0x1c, 0x28;
0x10, 0x20, 0x30, 0x40) and the whole store in the Buffer. Bytes 0-7 and
12-31 of every Attribute (format, `Offset enable`, table, buffer index,
stride 0x20/0x34/0x50) are equal. Both fetch the same data (case 63 PASSes
below).

**55 es3mrt** (`frame_mrt_tests` build of two RGBA8 targets): the two blend
descriptors (`00000200 f0122122 00000019 000ed000`, `00000200 f0122122
00010019 000ed000`), `BLEND_DESC` d50's count tag `…182`, DCD0 `0x21AB`,
DCD1 `0x3FFFF`, the FBD pointer's tag `…005` and the 64 bytes of framebuffer
parameters are equal but for the tiler context's address (bytes 56-63).
Each render target equals ours but for the base address (bytes 32-39) and
the `Surface stride` (bytes 44-47): Mesa `0x4000`, ours 0 (a single layer;
the field is not read for one). The drm-shim's AFBC and CRC differences
that `frame_mrt_tests` names are gone here (the board runs `linear,nocrc`).

**91 hdrblend** (`frame_blend_tests::case_91_s_frame_builds_with_mesa_s_float_blend_words`):
the left draw's blend descriptor `00000201 f0d32d32 0000001a 000df000` and
the right's `00000201 f0503503 0000001a 000df000` are ours byte for byte
(the right one was derived from the board's 25.0.7 `over`; 26.2.1 confirms
it); DCD0 `0x21AA` and DCD1 `0x1FFFF` for both draws; the FBD parameters
equal but for the tiler address. The render target (`48000000 86888299`,
RGBA16F internal, float writeback, clear halves `38004000 3c003400`) equals
ours but for the base address and the `Surface stride` (Mesa `0x200`, ours
0). The blends are therefore pinned on the board; case 91 still refuses at
its read-back draw (below).

## 3. Case runs, two each

`panthor-gltest 19 44 46 48 49 55 63 68 71 74 76 80 81 89 91 98 101 104 141
142 148 149 206`, twice: `cases-1.txt` and `cases-2.txt`, identical, each
`23 case(s) run: FAIL 2, PASS 15, REFUSED 6`; 0 kernel messages, 0 after
the end marker.

| Case | Verdict line (both runs) |
|---|---|
| 46, 48, 49, 63, 68, 71, 74, 76, 81, 89, 98, 104, 141, 142, 148 | `PASS` |
| 19 | `panthor-gltest: verdict case 19 es100: REFUSED link: online shaderc: a Mali link whose fragment shader reads part of a varying (the link keeps only those components) is not lowered yet ('Tint')` |
| 44 | `panthor-gltest: verdict case 44 es3hdr: REFUSED draw: mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet` |
| 55 | `panthor-gltest: verdict case 55 es3mrt: FAIL rc -1, first bad check -1 got 0x00000000 want 0x00000000` |
| 80 | `panthor-gltest: verdict case 80 matdyn: REFUSED compile: online shaderc: the code generator refused this fragment shader: Mali(Valhall: an indexed local variable is not lowered yet)` |
| 91 | `panthor-gltest: verdict case 91 hdrblend: REFUSED draw: mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet` |
| 101 | `panthor-gltest: verdict case 101 cubebias: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader look-up at a coordinate with a constant lane (a shape no reference draws) is not lowered yet)` |
| 149 | `panthor-gltest: verdict case 149 vsunits: REFUSED compile: online shaderc: the code generator refused this vertex shader: Mali(Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet)` |
| 206 | `panthor-gltest: verdict case 206 es3levels: FAIL rc -1, first bad check 2 got 0x04602020 want 0x04602090` |

## 4. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines
and the summary of each run, identical. 0 kernel messages in either run, 0
after the end marker.

    panthor-gltest: 256 case(s) run: FAIL 2, NO-CASE 83, PASS 147, REFUSED 15, SKIP 9

final3 (`2026-11-01-final3`): `FAIL 3, FAULT 1, NO-CASE 84, PASS 132, REFUSED 27, SKIP 9`.

## 5. Predicted vs observed

Predicted `NO-CASE 84, PASS 152, REFUSED 11, SKIP 9`. 250 of the 256
lines are as predicted; 70, 80 and 101's new refusal texts were predicted
exactly. Six lines differ:

- 19 predicted `PASS`: the vertex fp16 uniforms now compile (vsfp16-19),
  and the link refuses next: `REFUSED link: online shaderc: a Mali link
  whose fragment shader reads part of a varying (the link keeps only those
  components) is not lowered yet ('Tint')`. sweep_predict.py judges each
  shader alone, not the link.
- 44 predicted `PASS`: the program compiles; the draw that samples the
  RGBA16F FBO texture is refused: `REFUSED draw: mali: a texture stored
  other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not
  built yet` (a GL error 0x502 in its `glDrawArrays`).
- 91 predicted `PASS`: the same refusal, at the read-back draw that
  samples the RGBA16F target; the blended draws before it build (section 2).
- 55 predicted `PASS`: `FAIL rc -1, first bad check -1 got 0x00000000 want
  0x00000000`: before any draw, `glFramebufferTexture2D(...,
  GL_COLOR_ATTACHMENT1, ...)` raises GL error 0x500 (`gl_test: GL error 0x500
  after mrt attach`). The frame builder's two-target frame is pinned (above)
  but the GL core does not attach a second colour target on this backend.
- 206 predicted `PASS`: `FAIL rc -1, first bad check 2 got 0x04602020 want
  0x04602090`: the 3D texture's level 0 draw is right (check 1), levels 1
  and 2 read level 0's texel (checks 2-3; `93f9c958` builds a 3D texture of
  one level and says a 3D binding of more levels is refused, but this one
  is not: it draws, wrong).
- 218 predicted `NO-CASE` (the final3 sweep had no case 218): a case added
  to gl_test since, `verdict case 218 shadowcmp: REFUSED compile: online
  shaderc: the code generator refused this fragment shader: Mali(Valhall:
  fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a
  compare of one lane, or floor and fract of x and y is not lowered yet)`.

## 6. Delta against tools/cts/log/2026-11-01-final3 (both runs)

`python3 -I tools/mali/sweep_passsets.py --delta tools/cts/log`: exit 0,
`6 record(s), each earlier PASS set is a subset of 2026-11-07-t6-step4a (147
PASS)` (frameboard 102, tip 106, mid 123, final 126, final3 132). The
delta, in the tool's words (the prefix `panthor-gltest: verdict case <n>
<name>: ` left out; `…` stands for `online shaderc: the code generator
refused this <stage> shader: Mali(Valhall: `):

- case 46: `REFUSED compile: … a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet)` -> `PASS`
- case 48: the same look-up line -> `PASS`
- case 49: `REFUSED compile: … a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet)` -> `PASS`
- case 63: `FAIL rc -1, first bad check 1 got 0xff800000 want 0xff804000` -> `PASS`
- case 68, 71, 74, 142, 148: `REFUSED compile: … control flow (if, a loop, a return before the end) is not lowered yet)` -> `PASS`
- case 76: `REFUSED compile: … an integer or boolean value is not lowered yet)` (fragment) -> `PASS`
- case 81: `REFUSED compile: … a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet)` -> `PASS`
- case 89: `REFUSED compile: … an integer or boolean value is not lowered yet)` (vertex) -> `PASS`
- case 98: `FAIL rc -1, first bad check 4 got 0xfff00010 want 0xfff00000` -> `PASS`
- case 104: `FAULT work lost 2, group state/fatal queues 0x1/0x0, vm usable true` -> `PASS`
- case 141: `FAIL rc -1, first bad check 2 got 0xff00ff00 want 0x80008000` -> `PASS`
- case 19: `REFUSED compile: … a vertex shader uniform that is not highp (Mesa's fp16 uniform) is not lowered yet)` -> `REFUSED link: online shaderc: a Mali link whose fragment shader reads part of a varying (the link keeps only those components) is not lowered yet ('Tint')`
- case 44: `REFUSED compile: … a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet)` -> `REFUSED draw: mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet`
- case 55: `REFUSED compile: … a fragment output other than gl_FragColor is not lowered yet)` -> `FAIL rc -1, first bad check -1 got 0x00000000 want 0x00000000`
- case 70: `REFUSED compile: … arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet)` -> `REFUSED compile: … a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet)`
- case 80: `REFUSED compile: … three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet)` -> `REFUSED compile: … an indexed local variable is not lowered yet)`
- case 91: `REFUSED draw: mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet` -> `REFUSED draw: mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet`
- case 101: `REFUSED compile: … a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet)` (fragment) -> `REFUSED compile: … a vertex shader look-up at a coordinate with a constant lane (a shape no reference draws) is not lowered yet)` (vertex; its cubebias checks 2 and 3 now pass first)
- case 206: the look-up line -> `FAIL rc -1, first bad check 2 got 0x04602020 want 0x04602090`
- case 218: `NO-CASE` -> `shadowcmp: REFUSED compile: … fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet)`

No case leaves the PASS set; 15 join it.
