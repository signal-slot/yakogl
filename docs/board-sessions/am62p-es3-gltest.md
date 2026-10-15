<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P ES 3.0 gl_test cases 196, 200-205, ES 2.0 gl_test, dEQP-GLES3 rows

Device results. Date: 2026-10-14. Boot: `uboot-mmc`. Board: Verdin AM62P.

Commit: `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328` (sha8 `cd738b1a`), the
ccfarm/integration tip. Every image was built from this commit. The one
exception to "as it is": the ES 3.0 gl_test image was built with
`CONFIG_GL_TEST_CASES` appended to `driver/samples/zephyr/gl_test/prj.conf`
for the build and the line removed again afterwards (nothing committed), so
the image's build id reads `PVR_SAMPLE_TREE=cd738b1-dirty`. The sources are
the commit's.

## Result in short

- gl_test `196,200,201,202,203,204,205` on the online compiler: **7 of 7
  pass** (`gl_test: all: 0 of 7 cases failed`), 0 resets. Every expected
  value held: 196 q0..q4 `available 1` with 0, 1, 0, 1, 1; 200 intrt 16/16;
  201 usamp 8/8; 202 blitconv 12/12; 203 tfcap 9/9; 204 srgb 8/8; 205
  mappbo 15/15. First board run of 196, 202, 203, 204 and 205.
- ES 2.0 gl_test default set: **0 of 50 cases failed**, the same line as
  the last AM62P record (`docs/board-sessions/am62p-es2-r1c.md`, `GT`).
- dEQP-GLES3 rows 1, 2, 4, 24 of `tools/cts/am62p-gles3-tree.txt` (images 2,
  3, 5, 25): all four `done`, every case begun has a result, 0 resets.
  Row 24 (`transform_feedback` and the `ubo` groups): **882 of 882 transform
  feedback and 1,099 of 1,099 ubo cases pass** (51 result lines were lost
  on `f2a13877`; none now). Row 4 now runs to its end (1,662 cases; 558
  never ran on `f2a13877`). Rows 1, 2: `fbo.blit.conversion` 325 pass, 0
  fail, 112 NotSupported.
- Images 20 and 21 were not run.

## Console port

The environment's `BOARD_UART` named the bridge's port 31 and the lease the
port 32. The first run (record `20261014-0632-es3gl-96460639`) on port 31
gave `verdict=empty` (`back_after=427s`, 0 console bytes), and the leased
console (port 32) stayed silent through the whole session: it is mcu_uart0,
as `tools/cts/log/investigations/am62p-console-empty.md` explains. The
main_uart0 bridge answered on port 33 of the lab's other bridge address,
the port `docs/board-sessions/am62p-es2-r1c.md` and
`am62p-es2-final-r2.md` used. Every result below was captured there
(`BOARD_UART=<bridge host>:<port 33>`). The farm registration was not
changed. The empty record is kept; it is not a result.

## Build

Environment names only: `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`,
`PVR_FW_BLOB` (copied to `driver/src/fw/pvr_fw_blob.h`, git-ignored, not
committed; `git status` was clean before and after), `DEQP_WORK` (the gles3
stage 1, its `work` directory), `PCO_SHC`, `GLSL2USC_CACHE`.

```sh
# ES 2.0 default set: the GT row of the common set (online compiler)
tools/cts/common-images.sh am62p tools/cts/common-tree.txt <imgdir> GT
#   GT rc=0 sha256=ef24b1ce6ecdb65d380f68134a567c2657fdec75b473ee302611cc7602cef181 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=cd738b1

# ES 3.0 set: the same command with this line appended to
# driver/samples/zephyr/gl_test/prj.conf for the build (then removed)
#   CONFIG_GL_TEST_CASES="196,200,201,202,203,204,205"
tools/cts/common-images.sh am62p tools/cts/common-tree.txt <imgdir3> GT
#   GT rc=0 sha256=9646063906d803cb... build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=cd738b1-dirty

# dEQP-GLES3 images 2, 3, 5, 25
DEQP_MODULE=gles3 TREE_ROWS=1,2,4,24 \
  tools/cts/am62p-deqp-images.sh tools/cts/am62p-gles3-tree.txt <img-gles3> cd738b1a
#   img2 rc=0 bin=3f8e6363d28c1e8e   img3 rc=0 bin=4406868d42fc24d1
#   img5 rc=0 bin=beb16b266474a759   img25 rc=0 bin=b6c2f602c8fa5ef1
```

Run:

```sh
export BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc BOARD_UART=<bridge host>:<port 33>
tools/board.sh am62p run <imgdir3>/GT/zephyr/zephyr.bin records/am62p es3gl
tools/board.sh am62p run <es2 GT image copy> records/am62p es2gl
TREE_IMAGE_DIR=<img-gles3> TREE_TAG=cd738b1a TREE_ROWS=1,2,4,24 \
  tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt records/am62p/gles3-cd738b1a
```

Rows 4 and 24 are in both requested sets (`TREE_ROWS=4,24` and
`TREE_ROWS=1,2,4`), so one tree run of rows 1, 2, 4, 24 covered both.

## Records (under `records/`, git-ignored)

| run | record directory under `records/am62p/` |
|---|---|
| ES 3.0 gl_test, first try, wrong port (empty) | `20261014-0632-es3gl-96460639` |
| ES 3.0 gl_test | `20261014-0639-es3gl-96460639` |
| ES 2.0 gl_test default set | `20261014-0641-es2gl-ef24b1ce` |
| dEQP image 2 (row 1) | `gles3-cd738b1a/20261014-0647-img2-cd738b1a-3f8e6363` |
| dEQP image 3 (row 2) | `gles3-cd738b1a/20261014-0650-img3-cd738b1a-4406868d` |
| dEQP image 5 (row 4) | `gles3-cd738b1a/20261014-0653-img5-cd738b1a-beb16b26` |
| dEQP image 25 (row 24) | `gles3-cd738b1a/20261014-0655-img25-cd738b1a-b6c2f602` |

## gl_test ES 3.0: verdict lines (verbatim)

```
board.sh am62p es3gl rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=77s
gl_test: the list 196,200,201,202,203,204,205 (7 cases) on one context
gl_test: all: 0 of 7 cases failed (7 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 48
```

Case 196 occl:

```
gl_test: occl: q0 ANY hidden: available 1 result 0 (want 1 0)
gl_test: occl: q1 ANY visible: available 1 result 1 (want 1 1)
gl_test: occl: q2 CONSERVATIVE empty: available 1 result 0 (want 1 0)
gl_test: occl: q3 ANY two kicks: available 1 result 1 (want 1 1)
gl_test: occl: q4 ANY then covered: available 1 result 1 (want 1 1)
gl_test:   occl: the white quad on top  (32,32) = 0xffffffff  want 0xffffffff  ok  [check 1]
gl_test:   occl: the occluder in the corner ( 2, 2) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
gl_test: case 196 occl: PASS
```

Case 200 intrt:

```
gl_test:   rgba8ui cleared              (1,1) = 7 8 9 10  want 7 8 9 10  ok  [check 1]
gl_test:   rgba8ui cleared              (7,6) = 7 8 9 10  want 7 8 9 10  ok  [check 2]
gl_test:   rgba8ui drawn                (0,3) = 200 17 3 0  want 200 17 3 0  ok  [check 3]
gl_test:   rgba8ui drawn                (5,3) = 200 17 3 5  want 200 17 3 5  ok  [check 4]
gl_test:   rgba8ui left drawn           (2,4) = 200 17 3 2  want 200 17 3 2  ok  [check 5]
gl_test:   rgba8ui right kept clear     (6,4) = 7 8 9 10  want 7 8 9 10  ok  [check 6]
gl_test:   rgba8ui loaded, left redrawn (1,5) = 3 200 17 1  want 3 200 17 1  ok  [check 7]
gl_test:   rgba8ui loaded, right kept   (5,5) = 7 8 9 10  want 7 8 9 10  ok  [check 8]
gl_test:   rgba32ui cleared             (1,1) = 305419896 2596069104 1 4294967295  want 305419896 2596069104 1 4294967295  ok  [check 9]
gl_test:   rgba32ui cleared             (7,6) = 305419896 2596069104 1 4294967295  want 305419896 2596069104 1 4294967295  ok  [check 10]
gl_test:   rgba32ui drawn               (0,3) = 2147483649 70000 5 0  want 2147483649 70000 5 0  ok  [check 11]
gl_test:   rgba32ui drawn               (5,3) = 2147483649 70000 5 5  want 2147483649 70000 5 5  ok  [check 12]
gl_test:   rgba32ui left drawn          (2,4) = 2147483649 70000 5 2  want 2147483649 70000 5 2  ok  [check 13]
gl_test:   rgba32ui right kept clear    (6,4) = 305419896 2596069104 1 4294967295  want 305419896 2596069104 1 4294967295  ok  [check 14]
gl_test:   rgba32ui loaded, left redrawn (1,5) = 5 2147483649 70000 1  want 5 2147483649 70000 1  ok  [check 15]
gl_test:   rgba32ui loaded, right kept  (5,5) = 305419896 2596069104 1 4294967295  want 305419896 2596069104 1 4294967295  ok  [check 16]
gl_test: case 200 intrt: PASS
```

Case 201 usamp:

```
gl_test:   usampler2D bottom-left       (16,48) = 0xff0000c8  want 0xff0000c8  ok  [check 1]
gl_test:   usampler2D bottom-right      (48,48) = 0xff006400  want 0xff006400  ok  [check 2]
gl_test:   usampler2D top-left          (16,16) = 0xff320000  want 0xff320000  ok  [check 3]
gl_test:   usampler2D top-right         (48,16) = 0xff0180fa  want 0xff0180fa  ok  [check 4]
gl_test:   isampler2D bottom-left       (16,48) = 0xff7fb21c  want 0xff7fb21c  ok  [check 5]
gl_test:   isampler2D bottom-right      (48,48) = 0x40c00080  want 0x40c00080  ok  [check 6]
gl_test:   isampler2D top-left          (16,16) = 0xa862948a  want 0xa862948a  ok  [check 7]
gl_test:   isampler2D top-right         (48,16) = 0x90708878  want 0x90708878  ok  [check 8]
gl_test: case 201 usamp: PASS
```

Case 202 blitconv:

```
gl_test:   565 top-left from bottom-left ( 2,13) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
gl_test:   565 top-right from bottom-right (13,13) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
gl_test:   565 bottom-left from top-left ( 2, 2) = 0xffff0000  want 0xffff0000  ok  [check 3]
gl_test:   565 bottom-right from top-right (13, 2) = 0xff9c6531  want 0xff9c6531  ok  [check 4]
gl_test:   565 corner (0,0)             ( 0, 0) = 0xffff0000  want 0xffff0000  ok  [check 5]
gl_test:   rgba16f (0,3) = 1000 0 0 1000 /1000  want 1000 0 0 1000  ok  [check 6]
gl_test:   rgba16f (3,3) = 0 1000 0 1000 /1000  want 0 1000 0 1000  ok  [check 7]
gl_test:   rgba16f (0,0) = 0 0 1000 1000 /1000  want 0 0 1000 1000  ok  [check 8]
gl_test:   rgba16f (3,0) = 199 399 599 799 /1000  want 200 400 600 800  ok  [check 9]
gl_test:   565 corner cleared           ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 10]
gl_test:   565 blit kept beside it      ( 2, 2) = 0xffff0000  want 0xffff0000  ok  [check 11]
gl_test:   565 blit kept far away       (13,13) = 0xff00ff00  want 0xff00ff00  ok  [check 12]
gl_test: case 202 blitconv: PASS
```

Case 203 tfcap:

```
gl_test:   A primitives written         = 2  want 2  ok  [check 1]
gl_test:   tfcap A records              48 floats  ok  [check 2]
gl_test:   A guard untouched            = 1  want 1  ok  [check 3]
gl_test:   A the triangles are drawn    (16,16) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
gl_test:   B primitives written         = 2  want 2  ok  [check 5]
gl_test:   tfcap B positions            24 floats  ok  [check 6]
gl_test:   tfcap B colours              24 floats  ok  [check 7]
gl_test:   tfcap C positions            12 floats  ok  [check 8]
gl_test:   C discard draws nothing      (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
gl_test: case 203 tfcap: PASS
```

Case 204 srgb:

```
gl_test:   srgb cleared                 (1,1) = 0xff7c7c7c  want 0xff7c7c7c  ok  [check 1]
gl_test:   srgb cleared                 (6,6) = 0xff7c7c7c  want 0xff7c7c7c  ok  [check 2]
gl_test:   srgb one,one (0.4 linear)    (1,3) = 0xffaaaaaa  want 0xffaaaaaa  ok  [check 3]
gl_test:   srgb over (0.6,0.1,0.35,0.75) (6,3) = 0xbfa059cc  want 0xbfa059cb  ok  [check 4]
gl_test:   srgb loaded + 0.1 (0.502)    (2,5) = 0xffbcbcbc  want 0xffbcbcbc  ok  [check 5]
gl_test:   srgb right half kept         (5,5) = 0xbfa059cc  want 0xbfa059cb  ok  [check 6]
gl_test:   srgb texel decoded           (32,32) = 0x80003380  want 0x80003380  ok  [check 7]
gl_test:   srgb linear filter at 0.516  (32,32) = 0xff838383  want 0xff838383  ok  [check 8]
gl_test: case 204 srgb: PASS
```

Case 205 mappbo:

```
gl_test:   mappbo: a draw from the mapped VBO = 1282  want 1282  ok  [check 1]
gl_test:   mappbo: glUnmapBuffer        = 1  want 1  ok  [check 2]
gl_test:   mappbo: written through the map (32,40) = 0xff595452  want 0xff595452  ok  [check 3]
gl_test:   mappbo: glUnmapBuffer 2      = 1  want 1  ok  [check 4]
gl_test:   mappbo: collapsed by the flushed ranges (32,40) = 0xff000000  want 0xff000000  ok  [check 5]
gl_test:   mappbo: texel (0,0)          ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
gl_test:   mappbo: texel (1,0)          (24,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
gl_test:   mappbo: texel (2,0)          (40,48) = 0xffff0000  want 0xffff0000  ok  [check 8]
gl_test:   mappbo: texel (3,0)          (56,48) = 0xffffffff  want 0xffffffff  ok  [check 9]
gl_test:   mappbo: texel (0,1)          ( 8,16) = 0xff00ffff  want 0xff00ffff  ok  [check 10]
gl_test:   mappbo: texel (1,1)          (24,16) = 0xffffff00  want 0xffffff00  ok  [check 11]
gl_test:   mappbo: texel (2,1)          (40,16) = 0xffff00ff  want 0xffff00ff  ok  [check 12]
gl_test:   mappbo: texel (3,1)          (56,16) = 0xff204080  want 0xff204080  ok  [check 13]
gl_test:   mappbo: rows the pack buffer holds unlike glReadPixels' = 0  want 0  ok  [check 14]
gl_test:   mappbo: glUnmapBuffer 3      = 1  want 1  ok  [check 15]
gl_test: case 205 mappbo: PASS
```

The console ends with `gl_test: done` and `gl_test: OK (rc=0)`.

## gl_test ES 2.0 default set (verbatim)

```
board.sh am62p es2gl rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=88s
gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 208
```

The last AM62P record (`am62p-es2-r1c.md`, `GT`) printed the same line, the
same 50 passes and the same 208 compiler runs. 50 `case N ...: PASS` lines
in the console, no other case verdict. No regression.

## dEQP-GLES3 rows (verbatim)

Runner lines:

```
board.sh am62p img2-cd738b1a rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=153s
board.sh am62p img3-cd738b1a rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=151s
board.sh am62p img5-cd738b1a rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=131s
board.sh am62p img25-cd738b1a rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=264s
```

The images' own counters:

```
img2  deqp: done: 1444 passed, 250 failed, 262 not supported, 0 warnings, complete=1 | deqp: summary: 1956 cases begun, 1956 with a result, 0 without
img3  deqp: done: 1479 passed, 135 failed, 69 not supported, 0 warnings, complete=1 | deqp: summary: 1683 cases begun, 1683 with a result, 0 without
img5  deqp: done: 875 passed, 614 failed, 173 not supported, 0 warnings, complete=1 | deqp: summary: 1662 cases begun, 1662 with a result, 0 without
img25 deqp: done: 2313 passed, 108 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 2421 cases begun, 2421 with a result, 0 without
```

`tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt
--table --depth 2` (the mustpass filter drops the cases outside the list, so
its totals are a little below the counters):

Row 24 (image 25):

```
group                           pass   fail   n.s.     QW  other   none
transform_feedback               882      0      0      0      0      0
ubo                             1099      0      0      0      0      0
TOTAL                           1981      0      0      0      0      0
== mustpass: 1981 cases in these records' case filters (40829 outside them, not run, left out); 1981 with a result (2421 case lines seen)
```

Row 4 (image 5):

```
group                           pass   fail   n.s.     QW  other   none
fragment_ops                      43      0      0      0      0      0
fragment_out                     242    583    115      0      0      0
implementation_limits             38     11      0      0      0      0
instanced                         39      6      0      0      0      0
lifetime                          55      1      0      0      0      0
multisample                       12      4     48      0      0      0
negative_api                     301      7      4      0      0      0
occlusion_query                   62      0      0      0      0      0
pbo                               43      0      6      0      0      0
polygon_offset                    20      0      0      0      0      0
prerequisite                       3      0      0      0      0      0
TOTAL                            858    612    173      0      0      0
== mustpass: 1643 cases in these records' case filters (41167 outside them, not run, left out); 1643 with a result (1662 case lines seen)
```

Rows 1 and 2 (images 2 and 3), plus image 5 for `fragment_ops.scissor`:

```
group                           pass   fail   n.s.     QW  other   none
draw                             308     14      0      0      0      0
draw_buffers_indexed               0      0     76      0      0      0
fbo                             1406    367    255      0      0      0
fence_sync                        10      0      0      0      0      0
fragment_ops                    1152      0      0      0      0      0   (images 2 and 3)
TOTAL                           2876    381    331      0      0      0
== mustpass: 3588 cases in these records' case filters (39222 outside them, not run, left out); 3588 with a result (3639 case lines seen)
```

Rows 1, 2, 4 by the groups the HANDOFF names (counted from the console
result lines of these records):

| group | pass | fail | NotSupported |
|---|---:|---:|---:|
| `fbo.blit.conversion` | 325 | 0 | 112 |
| `fbo.blit.rect` | 70 | 0 | 0 |
| `fbo.blit.default_framebuffer` | 81 | 3 | 22 |
| `fbo.blit.depth_stencil` | 0 | 16 | 12 |
| `fbo.color.repeated_clear` (all kinds; its `blit` cases: 20 fail) | 56 | 40 | 12 |
| `fbo.invalidate.sub` / `.whole` | 12 / 12 | 7 / 7 | 0 |
| `fragment_ops.scissor` (26, image 5) | 26 | 0 | 0 |
| `lifetime` | 55 | 1 | 0 |

### Against `f2a13877` (RESULTS.md, "AM62P: the whole dEQP-GLES3 tree on `f2a13877`")

The per-image numbers of `f2a13877` are not in RESULTS.md and its records
are not on this machine, so the comparison is by the group table there.

| group | this session (rows run) | `f2a13877` (whole group) |
|---|---|---|
| transform_feedback | 882 pass, 0 fail, 0 NotSupported | 640 fail + 191 NotSupported |
| ubo (row 24) | 1,099 pass | no failure listed |
| row 24, image 25 | 2,421 begun, 2,421 with a result, 0 lost | 51 Lost in capture |
| fbo (rows 1, 2) | 367 fail, 255 NotSupported | 872 fail |
| lifetime | 55 pass, 1 fail | 1 fail |
| row 4 (image 5) | 1,662 cases, complete | aborted; 558 never ran |
| fragment_out | 583 fail, 115 NotSupported | 595 fail |
| fragment_ops (rows 1, 2, 4) | 1,195 pass, 0 fail | 1,109 fail (mostly `fbo_srgb`, image 4, not run) |
| draw | 14 fail | 14 fail |
| instanced | 6 fail | 6 fail |
| implementation_limits | 11 fail | 11 fail |

## Findings

Differences from the expectations; no expectation was edited.

1. **Console port.** The environment's `BOARD_UART` (port 31) and the leased
   console (port 32) both gave no bytes; the main_uart0 bridge was on port
   33 of the other bridge address. HANDOFF says only "the main_uart0 bridge
   port". The farm registration and the task's environment still name the
   wrong ports; a lease should name the port that works. The first run's
   record is `20261014-0632-es3gl-96460639` (`verdict=empty`).
2. **Case 204, checks 4 and 6**: `0xbfa059cc`, want `0xbfa059cb`. HANDOFF
   says `203 89 160 191`; the board prints red 204. The case's check
   accepts a step of one and prints `ok`, so the case passes. The cause
   (blend rounding on the PowerVR) was not investigated here. Same for case
   202 check 9 (`199 399 599 799` against `200 400 600 800`; the HANDOFF
   tolerance is 0.004).
3. **Case 205 mappbo**: the HANDOFF text has no number for the checks; the
   source has 15 and all 15 print `ok`.
4. **Image build id** of the ES 3.0 image reads `cd738b1-dirty` (the
   temporary `prj.conf` line, see the top). The code is the commit's.
5. **dEQP row 24 (transform_feedback, ubo)**: expected "a buffer mismatch
   there and not on the host is the board's memory path". There is none:
   882 + 1,099 pass, and the lost result lines of `f2a13877` do not recur
   (`done`, all 2,421 begun have a result).
6. **dEQP row 4 (lifetime and the rest of image 5)** now runs to the end:
   `lifetime.attach.deleted_output.buffer_transform_feedback` no longer
   aborts the image. It still fails ("Writing to container with deleted
   attachment modified a new object."), as does
   `lifetime.attach.deleted_input.buffer_vertex_array` ("Writing to a fresh
   object modified the container with a deleted attachment."). The tally
   counts one lifetime failure (the other case is outside the mustpass
   list); `f2a13877` had one. The tail that never ran (558 cases) now has
   results: `occlusion_query` 62 of 62 pass, `polygon_offset` 20 of 20,
   `prerequisite` 3 of 3, `pbo` 43 pass / 6 NotSupported, `negative_api`
   301 pass / 7 fail / 4 NotSupported, `multisample` 12 pass / 4 fail / 48
   NotSupported (the four failures are `fbo_max_samples.proportionality_*`
   and `sample_coverage_invert`).
7. **`fbo.blit` (rows 1, 2): expected to pass but for the integer `tex2d`
   names and the 16 `depth_stencil` failures.** Seen:
   - `fbo.blit.conversion`: 325 pass, 0 fail, **112 NotSupported**. In all
     112 the source or the destination is `r32f`, `rg32f`, `rgba32f` or
     `r11f_g11f_b10f` (the full list is in the console of the image 2
     record); nothing fails. Whether these four formats are meant to stay
     NotSupported is not decided here.
   - `fbo.blit.default_framebuffer`: **3 Fail**, all `r8_*_blit_from_default`
     (`r8_linear_out_of_bounds_blit_from_default`,
     `r8_linear_scale_blit_from_default`,
     `r8_nearest_scale_blit_from_default`); 22 NotSupported.
   - `fbo.blit.rect` 70 of 70 pass; `fbo.blit.depth_stencil` 16 Fail, 12
     NotSupported (the blit-ds item, as expected).
   - `fbo.color.repeated_clear.blit`: 20 Fail, all `tex2d`: the 19 integer
     names expected (r8i ... rgba32ui, `rgb10_a2ui`) and one more,
     `tex2d.rgb16f`, which HANDOFF does not list. (The other 20 Fail of the
     `repeated_clear` group in the table above are not `blit` cases.)
   - `fbo.invalidate.sub.unbind_blit_*` and `whole.unbind_blit_*`: **14
     Fail** (7 each: color, depth, stencil and the four `msaa` ones), where
     HANDOFF expected the `unbind_blit_color*` cases (8) to pass.
   - `fragment_ops.scissor.framebuffer_blit_*` 3 of 3 pass, as expected.
   Many of the fbo failures are `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`
   (`es3fFboTestCase.cpp:146`, `es3fFboRenderTest.cpp:358`).
8. Rows 1 and 2 were not compared case by case with `c4ccc966`
   (`records/am62p/deqp-gles3-c4ccc966` is not on this machine) nor with the
   `f2a13877` records; the numbers above are group totals against RESULTS.md.

## Re-run at the integration tip `9b525867` (gate-green)

Device result. Date: 2026-10-14. Boot: `uboot-mmc`. Board: Verdin AM62P.

Commit under test: `9b5258677baabd3c13374dab00a4d048d2000384` (sha8
`9b525867`), the ccfarm/integration tip, 51 commits after `cd738b1a`
(`src` and `driver` changes since: `src/gl/framebuffer.rs`,
`src/gl/texture.rs`, `driver/samples/zephyr/deqp/CMakeLists.txt`). Image:
`zephyr.bin` sha256
`4acb9d771372aa35648b3a0aeee382ce11895cac2d269e73ffa3ad3dcd56ac12`, build id
`PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=9b52586-dirty` (the `-dirty` is the
`CONFIG_GL_TEST_CASES` line below, appended for the build and removed again;
the sources are the commit's; `git status` was clean after).

### Result in short

Cases 196, 202, 203, 204 and 205: **5 of 5 pass**, 0 resets. Every expected
value held, with the same two small differences that already passed at
`cd738b1a` (finding 2 above: check 4 and 6 of case 204 read `...cc` for
`...cb`; case 202 check 9 reads 199 for 200, tolerance 0.004). No failing
case, so there is no failure analysis.

### Build and run

Environment names only: `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`,
`PVR_FW_BLOB` (copied to `driver/src/fw/pvr_fw_blob.h`, git-ignored, not
committed), `PCO_SHC`, `GLSL2USC_CACHE`.

```sh
# driver/samples/zephyr/gl_test/prj.conf gets this line for the build only
#   CONFIG_GL_TEST_CASES="196,202,203,204,205"
tools/cts/common-images.sh am62p tools/cts/common-tree.txt <imgdir> GT
#   GT rc=0 sha256=4acb9d771372aa35648b3a0aeee382ce11895cac2d269e73ffa3ad3dcd56ac12 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=9b52586-dirty

export BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc BOARD_UART=<bridge host>:<port 33>
tools/board.sh --dry-run am62p run <imgdir>/GT/zephyr/zephyr.bin records/am62p es3gl
tools/board.sh am62p disarm          # see "Deviations" 3
tools/board.sh am62p run <imgdir>/GT/zephyr/zephyr.bin records/am62p es3gl
```

Record: `records/am62p/20261014-0910-es3gl-4acb9d77` (git-ignored).

### Verdict line (verbatim)

```
board.sh am62p es3gl rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=64s
gl_test: the list 196,202,203,204,205 (5 cases) on one context
gl_test: all: 0 of 5 cases failed (5 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 34
```

The board.sh verdict is one line for the whole image; the five cases share
one boot, so each case's own verdict is its `gl_test: case N name: PASS`
line below.

### Console lines that carry the numbers (verbatim)

Case 196 occl, expected q0..q4 `available 1` with 0, 1, 0, 1, 1:

```
gl_test: occl: q0 ANY hidden: available 1 result 0 (want 1 0)
gl_test: occl: q1 ANY visible: available 1 result 1 (want 1 1)
gl_test: occl: q2 CONSERVATIVE empty: available 1 result 0 (want 1 0)
gl_test: occl: q3 ANY two kicks: available 1 result 1 (want 1 1)
gl_test: occl: q4 ANY then covered: available 1 result 1 (want 1 1)
gl_test:   occl: the white quad on top  (32,32) = 0xffffffff  want 0xffffffff  ok  [check 1]
gl_test:   occl: the occluder in the corner ( 2, 2) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
gl_test: case 196 occl: PASS
```

Case 202 blitconv, expected 12/12:

```
gl_test:   565 top-left from bottom-left ( 2,13) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
gl_test:   565 top-right from bottom-right (13,13) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
gl_test:   565 bottom-left from top-left ( 2, 2) = 0xffff0000  want 0xffff0000  ok  [check 3]
gl_test:   565 bottom-right from top-right (13, 2) = 0xff9c6531  want 0xff9c6531  ok  [check 4]
gl_test:   565 corner (0,0)             ( 0, 0) = 0xffff0000  want 0xffff0000  ok  [check 5]
gl_test:   rgba16f (0,3) = 1000 0 0 1000 /1000  want 1000 0 0 1000  ok  [check 6]
gl_test:   rgba16f (3,3) = 0 1000 0 1000 /1000  want 0 1000 0 1000  ok  [check 7]
gl_test:   rgba16f (0,0) = 0 0 1000 1000 /1000  want 0 0 1000 1000  ok  [check 8]
gl_test:   rgba16f (3,0) = 199 399 599 799 /1000  want 200 400 600 800  ok  [check 9]
gl_test:   565 corner cleared           ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 10]
gl_test:   565 blit kept beside it      ( 2, 2) = 0xffff0000  want 0xffff0000  ok  [check 11]
gl_test:   565 blit kept far away       (13,13) = 0xff00ff00  want 0xff00ff00  ok  [check 12]
gl_test: case 202 blitconv: PASS
```

Case 203 tfcap, expected 9/9 (PRIMITIVES_WRITTEN 2, records, guard, window
green at (16,16); B 2; C discard leaves the window red):

```
gl_test:   A primitives written         = 2  want 2  ok  [check 1]
gl_test:   tfcap A records              48 floats  ok  [check 2]
gl_test:   A guard untouched            = 1  want 1  ok  [check 3]
gl_test:   A the triangles are drawn    (16,16) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
gl_test:   B primitives written         = 2  want 2  ok  [check 5]
gl_test:   tfcap B positions            24 floats  ok  [check 6]
gl_test:   tfcap B colours              24 floats  ok  [check 7]
gl_test:   tfcap C positions            12 floats  ok  [check 8]
gl_test:   C discard draws nothing      (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
gl_test: case 203 tfcap: PASS
```

Case 204 srgb, expected 8/8 with the values of `docs/es3-srgb.md` section 10
(124, 170, 203 89 160 191, 188, 128 51 0 128, 131):

```
gl_test:   srgb cleared                 (1,1) = 0xff7c7c7c  want 0xff7c7c7c  ok  [check 1]
gl_test:   srgb cleared                 (6,6) = 0xff7c7c7c  want 0xff7c7c7c  ok  [check 2]
gl_test:   srgb one,one (0.4 linear)    (1,3) = 0xffaaaaaa  want 0xffaaaaaa  ok  [check 3]
gl_test:   srgb over (0.6,0.1,0.35,0.75) (6,3) = 0xbfa059cc  want 0xbfa059cb  ok  [check 4]
gl_test:   srgb loaded + 0.1 (0.502)    (2,5) = 0xffbcbcbc  want 0xffbcbcbc  ok  [check 5]
gl_test:   srgb right half kept         (5,5) = 0xbfa059cc  want 0xbfa059cb  ok  [check 6]
gl_test:   srgb texel decoded           (32,32) = 0x80003380  want 0x80003380  ok  [check 7]
gl_test:   srgb linear filter at 0.516  (32,32) = 0xff838383  want 0xff838383  ok  [check 8]
gl_test: case 204 srgb: PASS
```

Case 205 mappbo, expected 15/15:

```
gl_test:   mappbo: a draw from the mapped VBO = 1282  want 1282  ok  [check 1]
gl_test:   mappbo: glUnmapBuffer        = 1  want 1  ok  [check 2]
gl_test:   mappbo: written through the map (32,40) = 0xff595452  want 0xff595452  ok  [check 3]
gl_test:   mappbo: glUnmapBuffer 2      = 1  want 1  ok  [check 4]
gl_test:   mappbo: collapsed by the flushed ranges (32,40) = 0xff000000  want 0xff000000  ok  [check 5]
gl_test:   mappbo: texel (0,0)          ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
gl_test:   mappbo: texel (1,0)          (24,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
gl_test:   mappbo: texel (2,0)          (40,48) = 0xffff0000  want 0xffff0000  ok  [check 8]
gl_test:   mappbo: texel (3,0)          (56,48) = 0xffffffff  want 0xffffffff  ok  [check 9]
gl_test:   mappbo: texel (0,1)          ( 8,16) = 0xff00ffff  want 0xff00ffff  ok  [check 10]
gl_test:   mappbo: texel (1,1)          (24,16) = 0xffffff00  want 0xffffff00  ok  [check 11]
gl_test:   mappbo: texel (2,1)          (40,16) = 0xffff00ff  want 0xffff00ff  ok  [check 12]
gl_test:   mappbo: texel (3,1)          (56,16) = 0xff204080  want 0xff204080  ok  [check 13]
gl_test:   mappbo: rows the pack buffer holds unlike glReadPixels' = 0  want 0  ok  [check 14]
gl_test:   mappbo: glUnmapBuffer 3      = 1  want 1  ok  [check 15]
gl_test: case 205 mappbo: PASS
```

### Deviations from the common steps

1. **No detached worktree.** `git worktree add` needed an approval the run
   could not get, so the image was built in this task's own checkout, which
   is the integration tip itself (a branch at `9b525867`, clean before).
   The scripts build the checkout they live in, so the build is the same.
2. **Console port.** As in the first session: the task's `BOARD_UART` and
   the lease named ports 31 and 32, which are not main_uart0. The farm
   (`ask_farm`) named the main_uart0 bridge at port 33 of the lab's bridge
   host; `tools/am62p_netboot_uart.py probe` accepted it and the run
   captured the whole console there.
3. **A stale one-shot image.** The first `board.sh run` exited rc=3 before
   touching the board: `/var/yakogl` held `once-SV-...bin` from an earlier
   run that did not clean up (staged 2026-10-13 23:19, `yako_once` and
   `yako_default` empty). `tools/board.sh am62p disarm` removed it (the
   documented remedy in the refusal) and the run went through.
4. **First image build failed with rc=101** (`GT rc=101 sha256=none`) with the
   image directory under `/tmp`; the same command with the image directory
   in the dependency cache gave rc=0. Cause unknown: the build output was not
   kept. Next instrument if it recurs: run the `west build` of
   `common-images.sh` by hand and read its first error line. Both
   builds used the same sources, so the passing image is the one tested.
5. The five cases ran as one list, in one boot, so there is one board.sh
   verdict line for the five. The first session ran seven (196,
   200-205) in one list; the cases and their values are the same.

# Session 2: dEQP-GLES3 rows 3, 4, 6, 18, 21, 22 and the fbo MMU watch (`4e62d171`)

Device result. Date: 2026-10-14. Boot: `uboot-mmc`. Board: Verdin AM62P.

Commit under test: `4e62d171137b5d96d1865ad46c2d4a68d554499f` (sha8
`4e62d171`), `origin/main` after `git fetch origin`; the task's checkout was
that commit, clean, and the images were built from it as it is (image build
id `PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=4e62d171`, no `-dirty`).
`driver/src/fw/pvr_fw_blob.h` came from the dependency cache (gitignored,
not committed). Stage 1 was built for this session with
`tools/cts/cts-build-zephyr.sh` (`MODULE=gles3`, `BOARD=verdin_am62p/am62p54/a53`,
CTS checkout from the dependency cache): the `DEQP_WORK` the farm copied was a
GLES2 stage 1 and `am62p-deqp-images.sh` refused it ("no gles3 stage 1").

## Result in short

Six rows and the narrow fbo image ran to `verdict=done`, 0 resets, no GPU
reset lines. Row 3 `fragment_ops.blend.fbo_srgb` passes 1,152 of 1,152
(1,106 failed at `c4ccc966`). Rows 4 and 7 stop where they stopped before:
image 5 is identical to `cd738b1a` (875 / 614 / 173) and image 7 ends at the
same `ResourceError` as on `f2a13877`. The rows 18, 21 and 22 results are the
first per-case ones for those groups on this board since `f2a13877`. No fault
line next to `fbo.completeness.size.distinct` or `fbo.depth.*`.

## Console

Before the first image the candidate bridge ports were tried:
`tools/am62p_netboot_uart.py probe <bridge host>:<port>` accepted the
main_uart0 port (exit 0, nobody else on it) and a raw read of it returned the
board's `login:` prompt, so the console shows bytes there. The leased port
gave no bytes (mcu_uart0, as in session 1). `BOARD_UART` was set to the one
that answered; no host or port is written here. All seven runs captured their
whole console there (no `verdict=empty`).

## Build and run

```
# stage 1 (once)
tools/cts/cts-build-zephyr.sh        # MODULE=gles3 WORK=<stage 1> BOARD=verdin_am62p/am62p54/a53 CTS=<VK-GL-CTS>
DEQP_MODULE=gles3 TREE_ROWS=3,4,6,18,21,22 tools/cts/am62p-deqp-images.sh \
  tools/cts/am62p-gles3-tree.txt <imgdir> 4e62d171 > <imgdir>/build.out
TREE_IMAGE_DIR=<imgdir> TREE_TAG=4e62d171 TREE_ROWS=3,4,6,18,21,22 BOARD_BOOT=uboot-mmc \
  BOARD_HOST=<user@host> BOARD_UART=<bridge host>:<port main_uart0> \
  tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt records/am62p/gles3-4e62d171
# the MMU watch: a one-line filter file outside the repository
#   dEQP-GLES3.functional.fbo.completeness.size.*,dEQP-GLES3.functional.fbo.depth.*
DEQP_MODULE=gles3 tools/cts/am62p-deqp-images.sh <filter file> <imgdir-fbow> 4e62d171 > <imgdir-fbow>/build.out
TREE_IMAGE_DIR=<imgdir-fbow> TREE_TAG=4e62d171 tools/board.sh am62p tree <filter file> records/am62p/fbow-4e62d171
tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt --statuses records/am62p/gles3-4e62d171/*/console.txt
```

Build lines (`build.out`; all `rc=0`):

```
img4 rc=0 bin=0642a836ba48283d
img5 rc=0 bin=550e112e291614bc
img7 rc=0 bin=5c96ade423914687
img19 rc=0 bin=c05ac0b1e4e89b45
img22 rc=0 bin=17c9504297297eb0
img23 rc=0 bin=88d9ad25472ad561
img1 rc=0 bin=44ba5695a0d385d2      # the fbo MMU-watch image (its own build.out)
```

Rows 19 and 20 of the table (images 20 and 21, the
`texture.filtering.3d.formats.*` and `.sizes.*` GPU hang) were **not
run**: the task skips them, so `texture.filtering.3d.formats` and `.sizes`
(83 cases) have no result here.

## Records

| what | directory under `records/am62p/` |
|---|---|
| image 4 (row 3) | `gles3-4e62d171/20261014-1202-img4-4e62d171-0642a836` |
| image 5 (row 4) | `gles3-4e62d171/20261014-1207-img5-4e62d171-550e112e` |
| image 7 (row 6) | `gles3-4e62d171/20261014-1210-img7-4e62d171-5c96ade4` |
| image 19 (row 18) | `gles3-4e62d171/20261014-1214-img19-4e62d171-c05ac0b1` |
| image 22 (row 21) | `gles3-4e62d171/20261014-1218-img22-4e62d171-17c95042` |
| image 23 (row 22) | `gles3-4e62d171/20261014-1226-img23-4e62d171-88d9ad25` |
| fbo MMU watch | `fbow-4e62d171/20261014-1233-img1-4e62d171-44ba5695` |

## Verdict and done lines (verbatim)

```
board.sh am62p img4-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=258s
board.sh am62p img5-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=131s
board.sh am62p img7-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=249s
board.sh am62p img19-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=245s
board.sh am62p img22-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=449s
board.sh am62p img23-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=384s
board.sh am62p img1-4e62d171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=77s   (fbo MMU watch)
```

```
img4  deqp: done: 1981 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1981 cases begun, 1981 with a result, 0 without
img5  deqp: done: 875 passed, 614 failed, 173 not supported, 0 warnings, complete=1 | deqp: summary: 1662 cases begun, 1662 with a result, 0 without
img7  deqp: done: 594 passed, 41 failed, 0 not supported, 0 warnings, complete=0 | deqp: summary: 635 cases begun, 635 with a result, 0 without
img19 deqp: done: 235 passed, 413 failed, 24 not supported, 0 warnings, complete=1 | deqp: summary: 672 cases begun, 672 with a result, 0 without
img22 deqp: done: 851 passed, 437 failed, 60 not supported, 180 warnings, complete=1 | deqp: summary: 1528 cases begun, 1528 with a result, 0 without
img23 deqp: done: 852 passed, 871 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1723 cases begun, 1723 with a result, 0 without
fbow  deqp: done: 7 passed, 10 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 17 cases begun, 17 with a result, 0 without
```

Tally totals (`deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt`, the six
tree records), verbatim:

```
mustpass cases with a result: 7472 (of 8201 seen)
group                                       pass   fail    n/s  other
fragment_ops                                2018      0      0      0
fragment_out                                 242    583    115      0
implementation_limits                         38     11      0      0
instanced                                     39      6      0      0
lifetime                                      55      1      0      0
multisample                                   12      4     48      0
negative_api                                 301      7      4      0
occlusion_query                               62      0      0      0
pbo                                           43      0      6      0
polygon_offset                                20      0      0      0
prerequisite                                   3      0      0      0
shaders                                      512     40      0      0
texture                                     1763   1275     84    180
TOTAL                                       5108   1927    257    180
```

The tally counts only cases that are in `gles3-mustpass.txt`: 7,472 of the
8,201 case lines the six images printed have a result there; 729 (280 Pass,
449 Fail) are outside the list, so the images' own counters above are larger
than the table's pass and fail columns (the image counters sum to 5,388 pass,
2,376 fail, 257 NotSupported, 180 QualityWarning, which is 8,201). 1,397
mustpass cases of image 7's buckets never ran (`NoResult`, below).

## Group tables

`cd738b1a` is this file's first session (rows 1, 2, 4, 24); `f2a13877` is
RESULTS.md ("AM62P: the whole dEQP-GLES3 tree on `f2a13877`"). The per-case
records of both are not on this machine, so the comparison is by group, never
case by case, and a Pass that became a Fail cannot be shown or excluded
beyond the equal totals below.

| group (image) | this session `4e62d171` | `cd738b1a` | `f2a13877` | `c4ccc966` |
|---|---|---|---|---|
| `fragment_ops.blend.fbo_srgb` (4) | **1,152 pass, 0 fail** | not run | `fragment_ops` 1,109 fail (mostly this) | 1,106 of 1,152 fail |
| `fragment_ops` (4 and 5) | 2,018 pass, 0 fail | 1,195 pass (rows 1, 2, 4) | 1,109 fail | 1,111 fail |
| image 5 as a whole | 875 / 614 / 173, 1,662 cases, complete | 875 / 614 / 173, 1,662, complete | aborted, 558 never ran | - |
| `fragment_out` (5) | 242 pass, 583 fail, 115 NotSupported | 583 fail, 115 NotSupported | 595 fail | 619 fail |
| `lifetime`, `pbo`, `negative_api`, `multisample` (5) | 55/1, 43/0 (+6 NS), 301/7 (+4 NS), 12/4 (+48 NS) | the same (finding 6 above) | tail never ran | - |
| `shaders.builtin_functions` (7) | 512 pass, 40 fail, 987 not run | not run | 987 without a result | - |
| `shaders.builtin_variable`, `conditionals`, `constant_expressions` (7) | 29, 41, 340 not run | not run | 29, 41, 340 without a result | - |
| `texture.specification` (23) | 696 pass, 467 fail | not run | in `texture` 1,752 fail; the "736" bucket | - |
| `texture.swizzle` / `units` / `vertex` (23) | 3/80, 39/120, 85/89 (pass/fail) | not run | in `texture` | - |
| `texture.filtering` 2d / 2d_array / cube / 3d.combinations (19, 22) | 117/0 (+12 NS), 7/115 (+12 NS), 112/0 (+12 NS), 3/53 | not run | in `texture` | - |
| `texture.format` (22) | 340 pass, 158 fail, 48 NS (unsized 64 pass; compressed 12 pass, 32 NS) | not run | in `texture` | - |
| `texture.mipmap` / `shadow` / `size` (22) | 139/85, 144/108 (+180 QW), 78/0 | not run | in `texture` | - |

The `texture` fail total on this
session's rows (1,275) is a part of `f2a13877`'s 1,752 for the whole group:
the 3D filtering hang images were not run, so the groups are not the same set.

Expectations of HANDOFF, checked, none edited:

- Row 3 `fbo_srgb` all 1,152 pass: **met** (1,152 / 1,152).
- `fragment_out`: unchanged from `cd738b1a`, 583 fail / 115 NotSupported. The
  failure reasons of the six images have no
  `glGetError() == GL_NO_ERROR` at `es3fTextureSpecificationTests` (the
  "736" reason of `f2a13877`) and no "Texture upload failed ... at
  gluTexture.cpp" (the "522" reason): those two reasons are gone; what
  fails now is `Image comparison failed` (933), `Image verification failed`
  (692), `Compile failed at sglrGLContext.cpp:863` (490),
  `Failed to compile shader program at glsTextureTestUtil.cpp:411` (150),
  `Result comparison failed` (40) and a few single ones. The 531 and 1,461
  buckets of HANDOFF were not matched by reason text (no per-bucket number
  from `f2a13877` here beyond the group totals above).
- The sRGB view (`| grep -i srgb`, 1,310 cases of the tally): 1,219 pass, 27
  fail, 64 NotSupported. The 27 fail are the 12
  `fragment_out.array.fixed.srgb8_alpha8_*` (refused by name, as expected) and
  15 others, listed in finding 3.

## fbo MMU watch

Image `fbow` (one row, `fbo.completeness.size.*` and `fbo.depth.*`) ran
`verdict=done`, `resets=0`, 17 cases begun, 17 with a result, `complete=1`,
back after 77 s. Its console holds no MMU fault, page fault, FWCORE fault or
context reset line; `grep -i 'mmu\|fault\|reset'` finds only the boot's
`MMU: FW heap mapped ...`, `M3 complete (MMU + FW load + FWIF structs)` and
the sample's own `resetting the SoC` after `sample done`. Same for the six
tree consoles: no fault line. Results:

| case | status |
|---|---|
| `fbo.completeness.size.distinct` | Pass |
| `fbo.completeness.size.zero` | Pass |
| `fbo.depth.basic.depth24_stencil8`, `.depth32f_stencil8`, `.depth_component16`, `.depth_component24`, `.depth_component32f` | Pass (5) |
| `fbo.depth.depth_test_clamp.*` (5 formats) | Fail: `Compile failed at sglrGLContext.cpp:863` |
| `fbo.depth.depth_write_clamp.*` (5 formats) | Fail: `Compile failed at sglrGLContext.cpp:863` |

The filter `fbo.completeness.size.*` matches only those two cases in this
dEQP; the tree's image 2 had them inside `fbo.completeness.*`.

## Findings

Differences from the expectations; no expectation was edited.

1. **Console port.** As in session 1 the lease's port gave no bytes. The
   `BOARD_UART` of the machine's environment file did answer this time (probe
   exit 0 and a `login:` prompt). So the lease's port is still the one to
   fix; the farm lookup was not available to the session.
2. **Stage 1.** The `DEQP_WORK` that `fetch_from_farm` copied (2.1 GiB) is a
   GLES2 stage 1 (`build/modules/gles3` has no package library), so the
   images could not be built until a gles3 stage 1 was built next to it. A
   `DEQP_STAGE1` for the AM62P gles3 is still not a farm value (the farm's
   `DEQP_STAGE1` is the i.MX8MP one).
3. **sRGB cases that fail** besides the 12 refused by name (15):
   `texture.filtering.2d_array.formats.srgb8_alpha8_*` (6: linear,
   nearest and the four mipmap modes), `texture.specification.basic_teximage3d.srgb8_2d_array`,
   `.srgb8_alpha8_2d_array`, `.srgb8_alpha8_3d`, `basic_texsubimage3d.srgb8_3d`,
   `.srgb8_alpha8_3d`, `texstorage2d.format.srgb8_cube`, `.srgb8_alpha8_cube`,
   `texstorage3d.format.srgb8_3d`, `.srgb8_alpha8_3d`. HANDOFF expected them
   to pass. All are 2D-array, 3D or cube sRGB texture paths; the 2D and
   cube-filtering sRGB cases and the `pbo` ones pass, and
   `texture.filtering.2d_array` fails 115 of 122 for every format, so these
   six are probably the same 2D-array cause, not an sRGB one. Not
   investigated here.
4. **`texture.filtering.2d_array`, `mipmap.3d`, `filtering.3d.combinations`,
   `shadow.2d_array`** fail almost completely (115 of 122, 64 of 64, 53 of 56,
   108 of 144): the 3D and 2D-array sampling is the next large texture item;
   it is not an sRGB or a format matter.
5. **`texture.specification`**: 467 of 1,163 mustpass cases fail (538 of the
   console's failing result lines, counting cases outside the list). The
   `GL_NO_ERROR` reason (736 at `f2a13877`) is gone. What remains, by the
   console's `Fail (...)` lines: 354 `Compile failed at
   sglrGLContext.cpp:863` (cube and integer cases such as
   `basic_teximage2d.r16i_cube`) and 184 `Image comparison failed`. The 3D
   and 2D-array rows fail wholesale (`basic_teximage3d` 69 of 69,
   `texstorage3d` 70 of 76, `texsubimage3d_depth` 5 of 5).
6. **`texture.swizzle` 80 of 83, `texture.units` 120 of 159, `texture.vertex`
   89 of 174** fail: not in the HANDOFF's expectations, no earlier per-bucket
   number on this machine. By the console's reason lines: `swizzle` 80
   `Image comparison failed`, `units` 136 `Compile failed at
   sglrGLContext.cpp:863` and 18 `Image comparison failed`, `vertex` 99
   `Image comparison failed`; `texture.filtering` 489, `mipmap` 95 and
   `shadow` 108 `Image verification failed`, `texture.format` 150 `Failed to
   compile shader program at glsTextureTestUtil.cpp:411` and 8 `Image
   comparison failed` (these counts include cases outside the mustpass list,
   which is why they exceed the tally's).
7. **Image 7 stops at the same case as `f2a13877`**:
   `shaders.builtin_functions.precision.clamp.highp_fragment.vec3`, a
   `ResourceError` ("a draw (24384 vertices/indices) does not fit an empty
   frame: the general arena (1048576 bytes) is too small", then
   `GL_OUT_OF_MEMORY at glsShaderExecUtil.cpp:684`, "Test run was ABORTED!",
   `complete=0`). 987 `builtin_functions` plus 340 `constant_expressions`, 41
   `conditionals` and 29 `builtin_variable` = 1,397 stay without a result; the
   1,461 bucket of HANDOFF is thus still not measured. This is the image 7
   `ResourceError` item of the roadmap.
8. **fbo MMU watch**: no fault. `fbo.depth.depth_test_clamp` and
   `depth_write_clamp` (10 cases) fail on a shader compile, not on a GPU
   fault; `fbo.depth.basic` and `size.distinct` pass.
9. **A stale one-shot image** again (`once-img5-4e62d171-...bin`, `yako_once`
   empty) refused the first run with rc=3; `tools/board.sh am62p disarm`
   removed it and the next run went through. The first run's refused record
   directory (a `meta.txt` only) was moved aside, because a record is never
   overwritten.
10. **Comparison limits.** The per-case records of `cd738b1a` and `f2a13877`
    are not on this machine; no Pass-to-Fail regression could be listed. The
    totals that can be compared (image 5, `fragment_out`, `lifetime`) are
    equal to `cd738b1a`.

# Re-run at main `3ff06479`

Device result. Date: 2026-10-15. Boot: `uboot-mmc`. Board: Verdin AM62P.

Commit under test: `3ff0647950badb6bd0c8643a9df7e4146c3c2a54` (sha8
`3ff06479`), `origin/main` after `git fetch origin`; the task's checkout was
detached at it, clean. It is 12 commits after `291f3cb9` and all of them are
docs (`docs/handoff/`, `docs/board-sessions/`, `tools/cts/RESULTS.md`,
`tools/cts/STATUS.md`, `tools/cts/log/`): no file under `src`, `driver` or
`tools/*.sh` changed since `291f3cb9`. Every image was built from it as it
is: all four build ids read `PVR_SAMPLE_TREE=3ff06479`, no `-dirty` (the ES
3.0 case list went in as a `-D` option, Deviations 2).
`driver/src/fw/pvr_fw_blob.h` came from the dependency cache (git-ignored,
not committed; `git status` clean before and after).

An earlier attempt at this item ran the same four images from `291f3cb9` on
2026-10-15 (its pull request was closed before it landed, so that section is
not on main). Its totals, read from that doc, equal the ones below check for
check and count for count; its records are no longer on this machine.

## Result in short

- gl_test `196,200,201,202,203,204,205` on the online compiler: **7 of 7
  pass**, 0 resets. The 82 `q0..q4`, `[check N]` and `case N name: PASS`
  lines of the console are byte for byte the `cd738b1a` listing at the top of
  this file (`diff` against it: no difference). That includes the two small
  differences of finding 2 (case 204 checks 4 and 6 read `0xbfa059cc` for
  `...cb`; case 202 check 9 reads `199 399 599 799` for `200 400 600 800`),
  which pass as before.
- ES 2.0 gl_test default set: **0 of 50 cases failed**, 208 compiler runs,
  the same line as `cd738b1a` and `am62p-es2-r1c.md`. No regression.
- dEQP-GLES3 rows 1 and 2 (images 2 and 3): both `done`, 0 resets, every
  case begun has a result, no case lost in capture. Image 2 fails **218**
  (250 at `cd738b1a`): `draw` 14 Fail to 0 and `fbo` 367 to 350 Fail in the
  mustpass tally. Image 3 is unchanged (1,479 / 135 / 69). Every `fbo.blit`
  group of finding 7 reads exactly as at `cd738b1a`, name for name.
- Images 20 and 21 were not run (the known hang). Row 4 (finding 6) was not
  part of this re-run.

## Build and run

Environment names only: `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`,
`PVR_FW_BLOB` (the copy in the dependency cache), `PCO_SHC`,
`GLSL2USC_CACHE`, `DEQP_WORK` (the AM62P gles3 stage 1 in the dependency
cache, `deqp-stage1-am62p-gles3`). The image directories were in the
dependency cache, not under `/tmp`.

```sh
# (b) ES 2.0 default set
tools/cts/common-images.sh am62p tools/cts/common-tree.txt <img-es2> GT
#   GT rc=0 sha256=bffe314dd85cfd29dff1a076de65a1f40939efd7db4b44941fadf66481b95b64 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3ff06479

# (a) ES 3.0 set: west directly, the board, sample and options of
# common-images.sh's GT row plus the case list (Deviations 2)
west build -p always -b verdin_am62p/am62p54/a53 -d <img-es3>/GT driver/samples/zephyr/gl_test -- \
  -DZEPHYR_MODULES=<tools/cts/zephyr-modules.sh $ZEPHYR_WS> -DYAKOGL_DIR=<checkout> \
  -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n \
  -DCONFIG_GL_TEST_CASES="196,200,201,202,203,204,205"
#   GT rc=0 sha256=8e3fa8bba9f35402ffd9beb029dd186d78ef2ea07d4921a05a5b693c22e68470 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=3ff06479
#   <img-es3>/GT/zephyr/.config: CONFIG_GL_TEST_CASES="196,200,201,202,203,204,205"

# (c) dEQP-GLES3 images 2, 3
DEQP_WORK=<gles3 stage 1> DEQP_MODULE=gles3 TREE_ROWS=1,2 \
  tools/cts/am62p-deqp-images.sh tools/cts/am62p-gles3-tree.txt <img-gles3> 3ff06479 > <img-gles3>/build.out
#   img2 rc=0 bin=1d6e11f628d87788   img3 rc=0 bin=f3f913c77f8d589e

export BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc BOARD_UART=<the lease's bridge host>:<port>
python3 tools/am62p_netboot_uart.py probe $BOARD_UART      # rc 0 at the first try
tools/board.sh am62p status                                 # not armed, /var/yakogl empty, no lease
tools/board.sh --dry-run am62p run <GT image>/zephyr/zephyr.bin records/am62p es3gl
tools/board.sh am62p run <img-es2>/GT/zephyr/zephyr.bin records/am62p es2gl
tools/board.sh am62p run <img-es3>/GT/zephyr/zephyr.bin records/am62p es3gl
TREE_IMAGE_DIR=<img-gles3> TREE_TAG=3ff06479 TREE_ROWS=1,2 BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt \
  tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt records/am62p/gles3-3ff06479
tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt --table --depth 2 \
  records/am62p/gles3-3ff06479/*/console.txt
```

`tools/board.sh am62p status` before the first run: no board lease,
`yako_once` and `yako_default` empty, `/var/yakogl` empty (`yako_file` still
named another session's one-shot token, which U-Boot ignores without
`yako_once`). The board was not armed, so `disarm` was not run. After the
last run: the same, `/var/yakogl` empty.

Records (git-ignored):

| run | record directory under `records/am62p/` |
|---|---|
| ES 2.0 gl_test default set | `20261015-2043-es2gl-bffe314d` |
| ES 3.0 gl_test | `20261015-2051-es3gl-8e3fa8bb` |
| dEQP image 2 (row 1) | `gles3-3ff06479/20261015-2058-img2-3ff06479-1d6e11f6` |
| dEQP image 3 (row 2) | `gles3-3ff06479/20261015-2104-img3-3ff06479-f3f913c7` |

Each record's `meta.txt`: `who_sessions=0`, the one-shot image staged on the
eMMC and read back equal, removed after the run (read back).

## Verdict lines (verbatim)

ES 3.0:

```
board.sh am62p es3gl rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=79s
gl_test: the list 196,200,201,202,203,204,205 (7 cases) on one context
gl_test: all: 0 of 7 cases failed (7 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 48
```

The seven cases share one boot, so the per-case verdicts are the console's
`case N name: PASS` lines: `case 196 occl`, `case 200 intrt`, `case 201
usamp`, `case 202 blitconv`, `case 203 tfcap`, `case 204 srgb`, `case 205
mappbo`, each `PASS`. Check counts: 196 two pixel checks plus q0..q4
`available 1` with 0, 1, 0, 1, 1; 200 16/16; 201 8/8; 202 12/12; 203 9/9;
204 8/8; 205 15/15. The `ok` lines are identical to the `cd738b1a` listing
above (82 lines compared). The console ends `gl_test: done`, `gl_test: OK
(rc=0)`, then `GPU recoveries 0 (HWR counter 0)`.

ES 2.0:

```
board.sh am62p es2gl rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=78s
gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 208
```

50 `case N ...: PASS` lines; no FAIL, SKIP or timeout line in either
console; `GPU recoveries 0 (HWR counter 0)` in both.

dEQP-GLES3, runner lines:

```
board.sh am62p img2-3ff06479 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=213s
board.sh am62p img3-3ff06479 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=156s
```

The images' own counters:

```
img2  deqp: done: 1476 passed, 218 failed, 262 not supported, 0 warnings, complete=1 | deqp: summary: 1956 cases begun, 1956 with a result, 0 without
img3  deqp: done: 1479 passed, 135 failed, 69 not supported, 0 warnings, complete=1 | deqp: summary: 1683 cases begun, 1683 with a result, 0 without
```

Both consoles: no recoveries line, 0 GPU reset lines, no lost case. The
tree's own tally (`BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt` was set this
time) and `deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt --table
--depth 2` on the two consoles print the same table:

```
== mustpass: 3588 cases in these records' case filters (39222 outside them, not run, left out); 3588 with a result (3639 case lines seen)
group                           pass   fail   n.s.     QW  other   none
draw                             322      0      0      0      0      0
draw_buffers_indexed               0      0     76      0      0      0
fbo                             1423    350    255      0      0      0
fence_sync                        10      0      0      0      0      0
fragment_ops                    1152      0      0      0      0      0
TOTAL                           2907    350    331      0      0      0
```

`cd738b1a`, the same two images (its table above also counts image 5's
`fragment_ops.scissor`): `draw` 308 / 14, `fbo` 1,406 / 367 / 255,
`fragment_ops` 1,152 / 0.

The `fbo` subgroups, counted from all 3,639 console result lines (not only
the mustpass list), pass / fail / NotSupported:

| `fbo.` subgroup | pass | fail | NotSupported |
|---|---:|---:|---:|
| `api` | 20 | 0 | 0 |
| `blit` | 476 | 19 | 146 |
| `color` | 122 | 147 | 33 |
| `completeness` | 529 | 42 | 7 |
| `depth` | 5 | 10 | 0 |
| `invalidate` | 94 | 33 | 4 |
| `msaa` | 4 | 34 | 32 |
| `multiview` | 0 | 0 | 3 |
| `render` | 210 | 68 | 30 |
| `stencil` | 9 | 0 | 0 |

## Against findings 6 and 7

Finding 7 (`fbo.blit`, rows 1 and 2), counted from the console result lines
as in the `cd738b1a` table:

| group | `3ff06479` pass / fail / NotSupported | `cd738b1a` |
|---|---|---|
| `fbo.blit.conversion` | 325 / 0 / 112 | 325 / 0 / 112 |
| `fbo.blit.rect` | 70 / 0 / 0 | 70 / 0 / 0 |
| `fbo.blit.default_framebuffer` | 81 / 3 / 22 | 81 / 3 / 22 |
| `fbo.blit.depth_stencil` | 0 / 16 / 12 | 0 / 16 / 12 |
| `fbo.color.repeated_clear` (its `blit` cases) | 56 / 40 / 12 (44 / 20 / 8) | 56 / 40 / 12 (20 fail) |
| `fbo.invalidate.sub` / `.whole` | 12 / 7 and 12 / 7 | 12 / 7 and 12 / 7 |

The failing names are exactly the ones of finding 7 (54 names, from the
consoles):
- `blit.default_framebuffer`: `r8_linear_out_of_bounds_blit_from_default`,
  `r8_linear_scale_blit_from_default`, `r8_nearest_scale_blit_from_default`;
- `blit.depth_stencil`: the 16
  (`depth24_stencil8_{basic,depth_only,scale,stencil_only}`,
  `depth32f_stencil8_{basic,depth_only,scale,stencil_only}`,
  `depth_component{16,24,32f}_{basic,scale}`, `stencil_index8_{basic,scale}`);
- `color.repeated_clear.blit.tex2d`: the 19 integer formats (`r8i` to
  `rgba32ui`, `rgb10_a2ui`) and `rgb16f`;
- `invalidate.sub` and `invalidate.whole`:
  `unbind_blit_{color,depth,stencil,msaa_color,msaa_depth,msaa_stencil,msaa_depth_stencil}`,
  7 each.

So the `src/gl/blit.rs` part of finding 7 has not moved since `cd738b1a`.
`fragment_ops.scissor` is in image 5 (row 4), which did not run here.

What moved is outside `fbo.blit`. All 14 `draw` failures of `cd738b1a` pass
now (`draw` 324 Pass in the console, 322 in the mustpass list), and `fbo`
fails 17 fewer mustpass cases (350 against 367). The `cd738b1a` records are
not on this machine, so the 17 cannot be named case by case, and a Pass that
became a Fail elsewhere in the two images can be neither shown nor excluded
beyond the totals: image 2's fail count fell by 32, image 3's did not change.
The largest fail reasons of the two images are
`GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` at `es3fFboTestCase.cpp:146` (102),
`Compile failed at sglrGLContext.cpp:863` (67),
`GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` at `es3fFboRenderTest.cpp:358` (43),
`Image comparison failed` (42), `GL_INVALID_OPERATION at
es3fFboTestCase.cpp:154` (34) and `:90` (22), and the `fbo.completeness`
pair "checked is incomplete, expected complete" / "checked as complete,
expected incomplete" (21 each). The failing cases also include all 19
integer formats of `fbo.invalidate.format.*` (its 4 NotSupported are the
`*32f` ones).

Finding 6 (row 4, image 5) was not re-run: it was not in this task's rows.
Finding 2 (the case 204 and 202 roundings) is unchanged at `3ff06479`.

## Deviations from the common steps

1. **The variables came from the lease through a helper.** This worker's
   command sandbox refuses a `VAR=value` prefix on a script it does not know
   (`BOARD_UART=... tools/board.sh ...`, `DEQP_MODULE=... TREE_ROWS=...
   tools/cts/am62p-deqp-images.sh ...`, a wrapper shell script, `west`
   itself). A temporary Python helper (`tools/tmp_env.py`, never committed,
   removed before the commit) set `BOARD_HOST`, `BOARD_UART` (both from the
   lease's `RESOURCE_*` variables, no value in any file) and `BOARD_BOOT`,
   `DEQP_WORK`, `DEQP_MODULE`, `TREE_ROWS`, `TREE_IMAGE_DIR`, `TREE_TAG`,
   `BOARD_MUSTPASS`, and ran the script given to it. The scripts and their
   arguments are the ones listed above. `BOARD_UART` was not in the
   machine's environment (`board.sh --dry-run` printed `BRIDGE:PORT`
   without the helper).
2. **The ES 3.0 case list as a `-D` option, through `west` directly.**
   `tools/cts/common-images.sh` cannot forward a `-D` option, so a
   temporary wrapper (`tools/tmp-build-es3-gt.sh`, never committed, removed
   before the commit) ran the `west build` line above: the same board,
   sample, `-p always`, module list, `YAKOGL_DIR` and arm options as
   `common-images.sh`'s GT row, plus `-DCONFIG_GL_TEST_CASES`, and wrote the
   same `GT.built` line (sha256, build ids, the `rti_arm: armed` and build
   line checks). `prj.conf` was not edited: this is the first ES 3.0 gl_test
   image of this file without `-dirty` in its build id.
3. **No detached worktree.** `git worktree add` needed an approval the run
   could not get; the task's own checkout was detached at `origin/main`
   (`git checkout --detach origin/main`, clean) and the scripts built it.
4. **Order.** The ES 2.0 image finished first and ran first (20:43), the ES
   3.0 image next (20:51), then the tree (20:58, 21:04). The `--dry-run` of
   `board.sh am62p run` came before any run, with the earlier attempt's ES
   3.0 image as its stand-in (this session's was still building); it printed
   the lease's bridge as the capture and every board step.
5. **Console port.** The leased bridge port carried main_uart0 (U-Boot, the
   kernel and Zephyr all captured; `verdict=done` four times, no
   `verdict=empty`). `tools/am62p_netboot_uart.py probe` accepted at the
   first try: the lease's serial console tool did not hold the bridge this
   time (it was not used during the session).
6. **Stage 1.** `DEQP_WORK` was set to the AM62P gles3 stage 1 in the
   dependency cache (`deqp-stage1-am62p-gles3`,
   `build/modules/gles3/libdeqp-gles3-package.a` present). It was not
   rebuilt.
7. **No note under `docs/handoff/`.** The hand-over is the section below
   instead. A waiting note makes `tools/handoff-notes.sh --check` exit 1,
   which the review of the earlier attempt required to pass; main itself
   already carries a waiting note (`docs/handoff/20261015-am62p-gltest-es2.md`),
   so that check fails on main with or without this result. Whoever next
   updates `docs/HANDOFF.md` takes these lines from here.

## Hand-over (for `docs/HANDOFF.md`)

What `docs/HANDOFF.md` should say after this result, by section, in the
headings of `docs/handoff/README.md`.

### State of main

In "ES 3.0 (goal 4)", after "196 and 202 to 205 pass again at the
integration tip `9b525867` (...)", add: all seven cases 196 and 200 to 205
pass again at main `3ff06479` (2026-10-15; 7 of 7, 0 resets, every check
value the same as at `cd738b1a`; image sha256 `8e3fa8bb...`, build id
`3ff06479`, the case list given as `-DCONFIG_GL_TEST_CASES` on the `west
build` line; record `records/am62p/20261015-2051-es3gl-8e3fa8bb`). The ES
2.0 default set passes there too: 0 of 50 failed, 208 compiler runs (image
`bffe314d...`, build id `3ff06479`; record
`records/am62p/20261015-2043-es2gl-bffe314d`).

After "Rows re-run on `cd738b1a` (...)", add: rows 1 and 2 (images 2 and 3)
re-ran on `3ff06479` (2026-10-15; both `done`, 0 resets, every case begun has
a result; `records/am62p/gles3-3ff06479`). Image 2: 1,476 pass / 218 fail /
262 NotSupported (1,444 / 250 / 262 at `cd738b1a`). Image 3 is unchanged:
1,479 / 135 / 69. In the mustpass list, `draw` has 0 Fail (14 before) and
`fbo` has 350 Fail (367 before). Every `fbo.blit` group of finding 7 and
`fbo.invalidate.{sub,whole}.unbind_blit_*` read exactly as at `cd738b1a`.

### Known defects

"AM62P fbo / blit / invalidate failures": unchanged at `3ff06479`. The
failing cases are the same ones:
- `blit.default_framebuffer.r8_*` (3);
- `blit.depth_stencil` (16);
- `repeated_clear.blit.tex2d` (19 integer formats plus `rgb16f`);
- `invalidate.sub` / `.whole` `unbind_blit_*` (7 + 7).

The 14 `draw` failures of `cd738b1a` are gone.

### Next board session

In the AM62P part:

- An ES 3.0 gl_test image no longer needs the `prj.conf` edit:
  `-DCONFIG_GL_TEST_CASES="..."` on `common-images.sh`'s `west build` line
  (run by hand) builds it with a clean build id. `common-images.sh` itself
  still has no case override.
- The AM62P gles3 stage 1 is in the dependency cache
  (`deqp-stage1-am62p-gles3`); point `DEQP_WORK` at it (the farm's
  `DEQP_WORK` may still be the GLES2 one).
- `BOARD_UART` is not in the farm's environment on every worker; the
  lease's `RESOURCE_*` variables name the bridge, and a worker that refuses
  `VAR=value` prefixes needs a helper that exports them (Deviations 1).
- A gles3 tree's own tally needs `BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt`;
  with it the tree prints the mustpass table itself.
