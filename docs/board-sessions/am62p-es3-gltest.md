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
