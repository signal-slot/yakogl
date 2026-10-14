<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set from the integration image (v3d-hh-gate)

Status: **partial. GM, SO, CL and GT ran on the board; SV ran and returned no
console bytes (`verdict=empty`).** GM completes with score 465 but **terrain and
shadow run out of GPU memory (1 and 3 fps) and refract raises 1694
incomplete-framebuffer errors**; SO, CL pass; GT passes 50 of 50.

Built from the integration tip `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328` plus
one docs-only commit (`d5cb878`, the "build" id below; no file under `src/` or
`driver/` differs). Board: Verdin AM62P (PowerVR), boot `uboot-mmc`, console on
the bridge's main_uart0 port. Date: 2026-10-14, GM started 05:59 local, the
last row ended 06:22. Records: `records/am62p/common-cd738b1a/` (gitignored).

## What was built

Zephyr `af17c0c8aa2a`, nightly Rust, `cts-common.sh am62p build` (the firmware
blob copied from the deps cache into the ignored `driver/src/fw/pvr_fw_blob.h`,
never committed). All five rows rc=0, `PVR_SAMPLE_TABLE=c95547a1`,
`PVR_SAMPLE_TREE=d5cb878`:

| Row | sha256 of zephyr.bin (first 16) | full sha256 |
|-----|------|------|
| GM | `6fd665d87d69fda5` | `6fd665d87d69fda5bc1658f6bbfbe830a59e8803f74b53f4624d74385ce34cc3` |
| SO | `464a55f18093fd5d` | `464a55f18093fd5d33c53a10880d7554f10777141f16a3636a323dcf1a9beda6` |
| SV | `464a55f18093fd5d` | same image as SO (the AM62P board layer has no scan-out view, so SV is the copy path again) |
| CL | `c62be4976254c501` | `c62be4976254c501732b6d0f0556dca66cb1044fcbaddade255c52f8874deb2f` |
| GT | `e10577d16a56c6f4` | `e10577d16a56c6f46bd70d13775e8517440539e937ddfe1714d9517e1bf4031b` |

## Commands

```sh
tools/cts/cts-common.sh am62p build build/common/am62p-cd738b1a
BOARD_HOST=<user>@<host> BOARD_UART=<bridge host:port, main_uart0> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-cd738b1a records/am62p/common-cd738b1a
```

Inputs (`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC`,
`SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `PVR_FW_BLOB`,
`RUSTUP_TOOLCHAIN=nightly`) came from the deps cache. A fresh worktree has no
`driver/src/fw/`: create it before copying the blob. The console port is the
bridge's main_uart0 port, one above the lease's (mcu_uart0, silent), as
`docs/board-sessions/am62p-es2-final-r2.md` found. The first run attempt in this
item was blocked for several hours by the session's permission layer (ssh and
the run wrapper needed approval); only then did the run start.

## Verdict lines (cts-common.sh run, verbatim)

```
board.sh am62p GM rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0 record=594b1201,fc3f01d1,6184003f,0f258320 back_after=413s unsupported=0
board.sh am62p SO rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=128s
board.sh am62p SV rc=4 verdict=empty boot=uboot-mmc row boot_verdict=empty result=none resets=absent record=absent back_after=427s
board.sh am62p CL rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,008510c4,19050085,110c6d5f back_after=178s
board.sh am62p GT rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=87s
```

## Tally (common-tally.py, verbatim)

```
common-tally: build bba168d
common-tally: am62p, 5 rows from records/am62p/common-cd738b1a
GM GM rc=0 verdict=done result=record:GLMARK2 record=594b1201,fc3f01d1,6184003f,0f258320; resets (console count) 0; image=6fd665d87d69fda5 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
    glmark2 score 465; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none; GL errors raised 15 or more, draws that sampled an incomplete texture 0
SO SO rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets (console count) 0; image=464a55f18093fd5d build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
SV SV rc=4 verdict=empty result=none record=absent; resets absent; image=464a55f18093fd5d build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
    no result (result=none)
CL CL rc=0 verdict=done result=record:PROBES record=594b0601,008510c4,19050085,110c6d5f; resets (console count) 0; image=c62be4976254c501 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
    cluster probes pass (status pass p0 10c4 p1 0085 p2 0085 p3 1905 p4 6d5f crc16 110c (RGB565)); throughput: 1024x600: 4519 frames in 60021 ms, 75.3 fps, 46.3 Mpx/s; draw 13.1 ms mean, 21.32 ms/Mpx
GT GT rc=0 verdict=done result=legacy:gl_test_all record=absent; resets (console count) 0; image=e10577d16a56c6f4 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d5cb878
    gl_test passed 50 of 50 (0 failed, 0 timed out); failing: none
common-tally: rows 5, with a run 5, without 0; verdict done 4, other 1
```

`cts-common.sh` itself exited 1 (the SV row is `empty`).

## glmark2 per scene (GM console, verbatim FPS lines)

```
[build] use-vbo=false: FPS: 102 FrameTime: 9.863 ms
[build] use-vbo=true: FPS: 879 FrameTime: 1.139 ms
[texture] texture-filter=nearest: FPS: 939 FrameTime: 1.066 ms
[texture] texture-filter=linear: FPS: 935 FrameTime: 1.070 ms
[texture] texture-filter=mipmap: FPS: 920 FrameTime: 1.087 ms
[shading] shading=gouraud: FPS: 705 FrameTime: 1.419 ms
[shading] shading=blinn-phong-inf: FPS: 606 FrameTime: 1.651 ms
[shading] shading=phong: FPS: 485 FrameTime: 2.064 ms
[shading] shading=cel: FPS: 414 FrameTime: 2.419 ms
[bump] bump-render=high-poly: FPS: 329 FrameTime: 3.047 ms
[bump] bump-render=normals: FPS: 845 FrameTime: 1.184 ms
[bump] bump-render=height: FPS: 710 FrameTime: 1.409 ms
[effect2d] kernel=0,1,0;1,-4,1;0,1,0;: FPS: 173 FrameTime: 5.801 ms
[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;: FPS: 112 FrameTime: 8.974 ms
[pulsar] light=false:quads=5:texture=false: FPS: 889 FrameTime: 1.125 ms
[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: FPS: 47 FrameTime: 21.290 ms
[desktop] effect=shadow:windows=4: FPS: 145 FrameTime: 6.910 ms
[buffer] ...update-method=map (interleave=false): FPS: 62 FrameTime: 16.388 ms
[buffer] ...update-method=subdata: FPS: 62 FrameTime: 16.361 ms
[buffer] ...interleave=true:...update-method=map: FPS: 69 FrameTime: 14.644 ms
[ideas] speed=duration: FPS: 180 FrameTime: 5.582 ms
[jellyfish] <default>: FPS: 158 FrameTime: 6.330 ms
[terrain] <default>: FPS: 1 FrameTime: 1055.910 ms
[shadow] <default>: FPS: 3 FrameTime: 381.244 ms
[refract] <default>: FPS: 85 FrameTime: 11.804 ms
[conditionals] fragment-steps=0:vertex-steps=0: FPS: 951 FrameTime: 1.052 ms
[conditionals] fragment-steps=5:vertex-steps=0: FPS: 546 FrameTime: 1.832 ms
[conditionals] fragment-steps=0:vertex-steps=5: FPS: 952 FrameTime: 1.051 ms
[function] fragment-complexity=low:fragment-steps=5: FPS: 788 FrameTime: 1.271 ms
[function] fragment-complexity=medium:fragment-steps=5: FPS: 559 FrameTime: 1.791 ms
[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5: FPS: 741 FrameTime: 1.350 ms
[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5: FPS: 741 FrameTime: 1.351 ms
[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5: FPS: 258 FrameTime: 3.881 ms
                                  glmark2 Score: 465
glmark2: score 465: 33 scenes ran, 0 failed, 0 unsupported (33 list entries); window 800x600, shown yes, GPU resets 0
glmark2: GL errors raised 2573, draws that sampled an incomplete texture 0 (GL errors recorded 2)
yakogl: draws refused 0
glmark2: PASS
```

(The first FPS line per scene of the console is printed without the `[scene]`
prefix for `use-vbo=false`, gouraud, blinn-phong-inf and high-poly; the names
come from the matching `glmark2-frame:` lines.)

### Scenes with a GL error, incomplete-texture or memory line

The incomplete-texture counter (record word w3) is **0** for the whole run, so
no draw sampled an incomplete texture. `GL errors raised` is **2573**, and the
`render pool` / `out of GPU memory` lines name three scenes:

- **terrain** (1 fps, 10 frames). At 06:03:48:
  `E: render pool exhausted: 5464 KiB requested, 3648 KiB left`,
  `I: texture 800x600 x1 fmt 0 (10 levels, 5592404 bytes): out of GPU memory`,
  `I: GL error 0x505 in glGenerateMipmap`. Then every frame:
  `gl_fb_target: framebuffer 7 incomplete (status 0x8cd6)`,
  `GL error 0x506 in current_drawable_program`, `GL error 0x502 in glGenerateMipmap`,
  `GL error 0x506 in resolve_target`. Terrain is not working on the board:
  the 5.6 MB NPOT mip chain does not fit what is left of the render pool.
- **shadow** (3 fps, 27 frames). At 06:04:12:
  `E: render pool exhausted: 21848 KiB requested, 2744 KiB left`,
  `I: texture 1600x1200 x1 fmt 0 (11 levels, 22369620 bytes): out of GPU memory`.
  The scene ran, with a missing depth/mip texture; 1600x1200 is far above the
  glmark2 window.
- **refract** (85 fps, 848 frames). 1694 lines
  `gl_fb_target: framebuffer 1 incomplete (status 0x8cd6)`. I did not check
  whether the scene's image is right; 85 fps with incomplete framebuffers may
  mean draws were skipped. The pool counters at close: `render pool general ... 2 refused`, `osal(zephyr):
  reclaim ... 2 refused`. No `draws refused`: `yakogl: draws refused 0`.
- Also in the run: `FWCCB: UPDATE_STATS element=2 (out of memory)` five times
  (firmware parameter-buffer growth; 4 grow requests, all served, 0 GPU recoveries)
  at `bump-render=high-poly` and later scenes; they did not fail a scene.

Pool at close: `render pool summary: 59200 KiB`, bump 21588 KiB, top high-water
36760 KiB.

## Comparison with the earlier results

The plan item names terrain, refract and shadow as the earlier defects (#1, #2:
fixed on the host). On the AM62P at this commit:

| Scene | Earlier | Now |
|-------|---------|-----|
| terrain | defect #1 | **still failing on the board**: out of GPU memory, 1 fps, framebuffer incomplete each frame |
| refract | defect #2 | runs at 85 fps but with 1694 incomplete-framebuffer errors and 2 refused pool allocations |
| shadow | stale depth | runs at 3 fps; a 1600x1200 texture is out of GPU memory |

I did not have the earlier AM62P records to hand, so this table compares the
plan's defect list, not numbers. The `pvr-pool-npot` item (the pool replay with
NPOT mip chains) is in review; this run is evidence that the pool does not fit
terrain's 800x600 mip chain plus the live set (~38 MB live at jellyfish's end).

## SV

`SV` ran the same image as SO (sha256 `464a55f18093fd5d`, which passed three
minutes earlier). Its record has 0 console bytes (`console.raw`, `console.ts`,
`console.txt` all empty) and `back_after=427s`: the board did not print, or the
capture got nothing. I did not repeat it (time box). It is unresolved whether
this is a bridge or boot flake; the image itself is proven by SO.
