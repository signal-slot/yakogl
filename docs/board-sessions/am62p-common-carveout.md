<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P common set at the 256 MiB GPU carve-out (origin/main `e3749a25`)

Status: **complete. All five rows ran from U-Boot and read `verdict=done`.**
GM scores 462 (33 scenes ran, 0 failed, 0 unsupported) with **no `render pool
exhausted`, no `out of GPU memory`, no `framebuffer N incomplete` line and GL
errors raised 0 (2573 at `cd738b1a`)**; SO, SV and CL probes pass; GT passes
50 of 50. SV, which returned 0 console bytes at `cd738b1a`, ran twice here and
printed its whole console both times. What stays: terrain (1 fps), shadow
(3 fps) and refract (1 fps, 85 at `cd738b1a` when its draws were skipped) spend
their time in GL calls on the CPU, not on the GPU (section "glmark2 per
scene").

Built from origin/main `e3749a25bde174b5e22c05478ff55d1059d85ed1` (the
integration tip of 2026-10-15), which contains `504d4fae` ("glmark2 on the
AM62P: the 256 MiB GPU carve-out the dEQP image uses";
`git merge-base --is-ancestor 504d4fae HEAD` holds). The GM image's devicetree
has `gpu-carveout@a0000000 { reg = <0xa0000000 0x10000000>; no-map; }` and its
`.config` has `CONFIG_KERNEL_VM_SIZE=0x20000000`, `CONFIG_MAX_XLAT_TABLES=128`
(read in `build/common/am62p-e3749a25/GM/zephyr/`). Board: Verdin AM62P
(PowerVR BXS-4-64), boot `uboot-mmc`, console on the bridge's main_uart0 port.
Date: 2026-10-15, GM started 17:07 local, GT ended 17:24, the SV repeat ended
17:27. Records: `records/am62p/common-e3749a25/` (the five rows) and
`records/am62p/common-e3749a25-sv2/` (the SV repeat), both gitignored.

## What was built

Zephyr `af17c0c8aa2a`, nightly Rust (`rustc 1.101.0-nightly (080cc2714
2026-10-11)`), `tools/cts/cts-common.sh am62p build build/common/am62p-e3749a25`.
All five rows rc=0, `PVR_SAMPLE_TABLE=c95547a1`, `PVR_SAMPLE_TREE=e3749a25`
(each console prints `sample: build tree=e3749a25 table=c95547a1`):

| Row | sha256 of zephyr.bin (first 16) | full sha256 |
|-----|------|------|
| GM | `0b851d28652f4741` | `0b851d28652f4741b4f82d693c278f07512f63ec2640967037acc00dc138c89e` |
| SO | `829b850368d4fd49` | `829b850368d4fd49d1c994c21855c07d42deff2036578984c22e0739c09c317d` |
| SV | `b058d31f3894e710` | `b058d31f3894e710dccac2734225a44159770682c7130c62073edf42ab5dd74c` |
| CL | `d6787fb4ca5d976b` | `d6787fb4ca5d976bfe8f6b65dfcf259d6c7e948b30d72cc34808cd21949c5068` |
| GT | `287872ecbf105519` | `287872ecbf1055190819f4617d83ec5b47ad9dd12ad9349c55960e935584a17c` |

SV is no longer byte-identical to SO (at `cd738b1a` the two rows shared one
image): the two `.config` files differ in one line,
`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y`, which the slint sample's `main.c` reads;
the AM62P board layer still has no view, and the two consoles are the same
apart from timing (section "SO and SV").

Two build details of this host, neither a change to the repository:

- The nightly toolchain had no `rust-src` component and `rustup component
  add` was not available to the session, so the three `-Z build-std` rows
  (SO, SV, CL) were built with cargo's standard-library source root pointed
  at the deps cache's copy of `rust-src` for the same nightly (commit
  `080cc2714`, the one the installed rustc reports). GM and GT do not build
  Rust with `build-std` and needed nothing.
- `driver/src/fw/pvr_fw_blob.h` was copied from the deps cache into the
  (gitignored) path; a fresh worktree has no `driver/src/fw/`.

## Commands

```sh
tools/cts/cts-common.sh am62p build build/common/am62p-e3749a25
BOARD_HOST=<user>@<host> BOARD_UART=<bridge host:port, main_uart0> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-e3749a25 records/am62p/common-e3749a25
BOARD_HOST=<user>@<host> BOARD_UART=<bridge host:port, main_uart0> BOARD_BOOT=uboot-mmc \
  tools/board.sh am62p run build/common/am62p-e3749a25/SV/zephyr/zephyr.bin records/am62p/common-e3749a25-sv2 SV
```

Inputs (`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `GLMARK2_SRC`,
`SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `NOTO_SANS_DIR`,
the firmware blob, `RUSTUP_TOOLCHAIN=nightly`) came from the deps cache. The
board login and the bridge port came from the farm (`ask_farm`); the console
is the bridge's main_uart0 port, one above the leased port (mcu_uart0,
silent), as `docs/board-sessions/am62p-common-int.md` and
`tools/cts/log/investigations/am62p-console-empty.md` say. The board accepted
ccfarm's node key, not the user's key of this host, so the runner was given an
ssh-agent holding that key. No address, port or key is written here.

Two GM attempts before the run were refused by `board.sh` before any reboot
and are kept outside the tree directory in
`records/am62p/common-e3749a25-refused/` (one with the user's key, "the
board's Linux does not answer"; one when the bridge was held by another
client). They are not results. Another session's dEQP tree (one-shot images
named `img<N>-e3749a25-...`) was using the board at the time; this run waited
until the board and the bridge had been free for ten minutes, and started at
17:07.

## Verdict lines (cts-common.sh run, verbatim)

```
board.sh am62p GM rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:GLMARK2 resets=0 record=594b1201,fc3f01ce,6184003f,00258320 back_after=417s unsupported=0
board.sh am62p SO rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=128s
board.sh am62p SV rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=128s
board.sh am62p CL rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,008510c4,19050085,110c6d5f back_after=178s
board.sh am62p GT rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=88s
```

The SV repeat (`tools/board.sh am62p run`, verbatim):

```
board.sh am62p SV rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=115s
```

## Tally (common-tally.py, verbatim)

```
common-tally: build e3749a25
common-tally: am62p, 5 rows from records/am62p/common-e3749a25
GM GM rc=0 verdict=done result=record:GLMARK2 record=594b1201,fc3f01ce,6184003f,00258320; resets (console count) 0; image=0b851d28652f4741 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=e3749a25
    glmark2 score 462; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none; GL errors raised 0, draws that sampled an incomplete texture 0
    glmark2 cell green; scenes in the console 33, clean 33, with known-bad lines 0 (no glmark2-mem lines: a line goes to the next scene's result)
    run records/am62p/common-e3749a25/20261015-1707-GM-0b851d28
SO SO rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets (console count) 0; image=829b850368d4fd49 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=e3749a25
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
    run records/am62p/common-e3749a25/20261015-1714-SO-829b8503
SV SV rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets (console count) 0; image=b058d31f3894e710 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=e3749a25
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
    run records/am62p/common-e3749a25/20261015-1717-SV-b058d31f
CL CL rc=0 verdict=done result=record:PROBES record=594b0601,008510c4,19050085,110c6d5f; resets (console count) 0; image=d6787fb4ca5d976b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=e3749a25
    cluster probes pass (status pass p0 10c4 p1 0085 p2 0085 p3 1905 p4 6d5f crc16 110c (RGB565)); throughput: 1024x600: 4390 frames in 60019 ms, 73.1 fps, 44.9 Mpx/s; draw 13.5 ms mean, 21.97 ms/Mpx
    run records/am62p/common-e3749a25/20261015-1719-CL-d6787fb4
GT GT rc=0 verdict=done result=legacy:gl_test_all record=absent; resets (console count) 0; image=287872ecbf105519 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=e3749a25
    gl_test passed 50 of 50 (0 failed, 0 timed out); failing: none
    run records/am62p/common-e3749a25/20261015-1723-GT-287872ec
common-tally: rows 5, with a run 5, without 0; verdict done 5, other 0
```

`cts-common.sh` exited 0. (The tally's "no glmark2-mem lines" remark is not
true of this console, which has 66 `glmark2-mem:` lines; the per-scene cell
is green either way because no known-bad line occurs. A tally reading, not a
board one.)

## glmark2 per scene (GM console, verbatim FPS lines)

```
[build] use-vbo=false: FPS: 101 FrameTime: 9.966 ms
[build] use-vbo=true: FPS: 879 FrameTime: 1.139 ms
[texture] texture-filter=nearest: FPS: 939 FrameTime: 1.066 ms
[texture] texture-filter=linear: FPS: 935 FrameTime: 1.070 ms
[texture] texture-filter=mipmap: FPS: 920 FrameTime: 1.087 ms
[shading] shading=gouraud: FPS: 706 FrameTime: 1.418 ms
[shading] shading=blinn-phong-inf: FPS: 606 FrameTime: 1.652 ms
[shading] shading=phong: FPS: 485 FrameTime: 2.065 ms
[shading] shading=cel: FPS: 414 FrameTime: 2.420 ms
[bump] bump-render=high-poly: FPS: 331 FrameTime: 3.023 ms
[bump] bump-render=normals: FPS: 845 FrameTime: 1.185 ms
[bump] bump-render=height: FPS: 705 FrameTime: 1.420 ms
[effect2d] kernel=0,1,0;1,-4,1;0,1,0;: FPS: 173 FrameTime: 5.799 ms
[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;: FPS: 112 FrameTime: 8.972 ms
[pulsar] light=false:quads=5:texture=false: FPS: 890 FrameTime: 1.125 ms
[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: FPS: 47 FrameTime: 21.298 ms
[desktop] effect=shadow:windows=4: FPS: 145 FrameTime: 6.911 ms
[buffer] ...update-method=map (interleave=false): FPS: 62 FrameTime: 16.392 ms
[buffer] ...update-method=subdata: FPS: 62 FrameTime: 16.364 ms
[buffer] ...interleave=true:...update-method=map: FPS: 69 FrameTime: 14.665 ms
[ideas] speed=duration: FPS: 179 FrameTime: 5.597 ms
[jellyfish] <default>: FPS: 159 FrameTime: 6.327 ms
[terrain] <default>: FPS: 1 FrameTime: 1467.257 ms
[shadow] <default>: FPS: 3 FrameTime: 352.807 ms
[refract] <default>: FPS: 1 FrameTime: 2046.240 ms
[conditionals] fragment-steps=0:vertex-steps=0: FPS: 951 FrameTime: 1.052 ms
[conditionals] fragment-steps=5:vertex-steps=0: FPS: 547 FrameTime: 1.831 ms
[conditionals] fragment-steps=0:vertex-steps=5: FPS: 952 FrameTime: 1.051 ms
[function] fragment-complexity=low:fragment-steps=5: FPS: 787 FrameTime: 1.271 ms
[function] fragment-complexity=medium:fragment-steps=5: FPS: 559 FrameTime: 1.791 ms
[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5: FPS: 741 FrameTime: 1.351 ms
[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5: FPS: 741 FrameTime: 1.351 ms
[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5: FPS: 258 FrameTime: 3.882 ms
                                  glmark2 Score: 462
glmark2: score 462: 33 scenes ran, 0 failed, 0 unsupported (33 list entries); window 800x600, shown yes, GPU resets 0
glmark2: GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)
yakogl: draws refused 0
glmark2: PASS
```

(As at `cd738b1a`, the console prints the first FPS line of `use-vbo=false`,
gouraud, blinn-phong-inf, terrain, shadow and refract without the `[scene]`
prefix; the names above come from the matching `glmark2-frame:` lines.)

Pool and firmware counters at close (verbatim):

```
I: render pool summary: 255808 KiB, bump 91356 KiB now, 91356 KiB high-water; top 4096 KiB now, 56716 KiB high-water
I: render pool general: free 1316 KiB in 9 ranges, largest 384 KiB; 2937 reused, 2913 bumped, 0 refused; 5823 frees (2326320 KiB)
I: render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)
I: render freelists: reserve refills refused 0 (0 KiB); firmware UPDATE_STATS: partial renders 0, out of memory 0, TA stores 0, 3D stores 0, CDM stores 0, TDM stores 0, other 0
```

(`cd738b1a`: pool 59200 KiB, 2 refused allocations, 4 grow requests, 5
firmware `out of memory` stats.) The live set is largest at refract's end:
`glmark2-mem: entry 24 end: ... pool handed out 151625728, ... live 142553088
bytes` (142.6 MB), more than twice the old 64 MiB carve-out.

### The three checks against `cd738b1a`

Counted in `records/am62p/common-e3749a25/20261015-1707-GM-0b851d28/console.txt`
(549 lines):

| Check | `cd738b1a` (`docs/board-sessions/am62p-common-int.md`) | `e3749a25` |
|-------|------|------|
| (a) terrain, shadow: `render pool exhausted` | 2 (terrain 5464 KiB requested, 3648 KiB left; shadow 21848 KiB requested, 2744 KiB left), with `out of GPU memory` for both textures | **0 lines** `render pool exhausted`, **0 lines** `out of GPU memory`, **0 lines** `GL error 0x` |
| (b) refract: `framebuffer 1 incomplete` | 1694 lines (`gl_fb_target: framebuffer 1 incomplete (status 0x8cd6)`); terrain also had `framebuffer 7 incomplete` each frame | **0 lines** matching `framebuffer [0-9]+ incomplete` |
| (c) GL errors raised (GLMARK2 record / console) | 2573 (console); the record's 4-bit field reads "15 or more" (word w3 `0f258320`) | **0** (console: `GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)`; the record's field reads 0, word w3 `00258320`) |

The record's counters are 4 bits each (the top of word w3: `0f258320` at
`cd738b1a`, `00258320` here), so the record can only say "15 or more" against
"0"; the console line gives the count. The incomplete-texture counter is 0 in
both runs.

### Scenes that are still slow

Three scenes draw a few frames in their 10 s, and all of their time is in the
GL calls (the `glmark2-frame:` lines, `us per frame: gl ..., swap ...`):

- **terrain**: 7 frames; `gl 1467010, swap 19` us per frame. One
  `W: render: fence ... (frame fence) not moved at 20 ms, re-posted 1 kick(s)`
  per frame (7 of the run's 8; the known re-post defect in `docs/HANDOFF.md`,
  not a hang: the kick completes after the re-post). At `cd738b1a` terrain
  also read 1 fps, but with its mip chain refused and every frame erroring.
- **shadow**: 29 frames; `gl 352737, swap 11` us per frame. 3 fps, as at
  `cd738b1a` (381 ms then, with its 1600x1200 texture refused). The HWRT
  1600x1200 is set up once (two `I: render: HWRT 1600x1200 x1` lines).
- **refract**: 5 frames; `gl 2045892, swap 28` us per frame (one re-post, the
  8th). **85 fps at `cd738b1a` was the scene with its target allocations
  refused and 1694 incomplete framebuffers, so its draws were skipped**; 1 fps
  is the price of drawing it.

Where the CPU time goes is not in the console (the image is built without
`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES`, so `pvr kick times: absent`). Read in the
sources (not measured): the PowerVR backend sets `generate_mipmap: None`
(`src/pvr/backend.rs`), so `glGenerateMipmap` is the GL core's CPU path,
"Box-filter the chain down from level 0 (CPU; the chain is CPU-visible)"
(`src/gl/texture.rs`). terrain filters its 800x600 10-level chain (5.6 MB)
and refract its 1600x1200 11-level chain (22 MB) every frame; the V3D got a
TFU path for this on 2026-10-14 (`docs/HANDOFF.md`, "Render targets sampled
from their render image"), the PowerVR has none. shadow has no mip chain; its
353 ms per frame is not explained by this reading. This item does not change
driver code; the host side is open work.

The score moved 465 -> 462: refract's 85 fps became 1 fps and every other
scene is within 2 fps of `cd738b1a` (the per-scene table above against
`am62p-common-int.md`).

## SO and SV

Both probes pass with crc16 `c592`, the same window crc32 `0x3b10b032`, the
same five probe points (diff <= 1 per channel) and the same 56 fps
(`slint: animation: 564 frames in 10016 ms` SO, `563 frames in 10004 ms`
SV). With timestamps stripped the two consoles differ only in the one-shot
token, frame counts and pool bookkeeping (64 lines of a 200-line console):
on the AM62P the view option changes nothing visible, as
`docs/investigations/am62p-scanout-view.md` says.

### SV, run twice

| Attempt | Record | console.raw bytes | Verdict | back_after |
|---------|--------|------|------|------|
| 1 (the tree) | `records/am62p/common-e3749a25/20261015-1717-SV-b058d31f` | 17294 | `done`, PROBES pass, crc16 c592 | 128 s |
| 2 (`board.sh am62p run ... SV`) | `records/am62p/common-e3749a25-sv2/20261015-1725-SV-b058d31f` | 17296 | `done`, PROBES pass, crc16 c592 | 115 s |

(SO's console: 17295 bytes.) The `cd738b1a` SV row (0 console bytes,
`verdict=empty`, `back_after=427s`) is not reproduced: two boots of the SV
image, each with the whole console captured and the same record as SO. The
empty capture of 2026-10-14 was a capture or bridge flake of that attempt,
not a property of the image or the row.

## CL and GT

- CL: PROBES pass, crc16 `110c`; `1024x600: 4390 frames in 60019 ms, 73.1
  fps, 44.9 Mpx/s; draw 13.5 ms mean, 21.97 ms/Mpx` (`cd738b1a`: 4519 frames,
  75.3 fps, 46.3 Mpx/s, 13.1 ms). The first frame: `glCompileShader 72/439
  ms`, `glLinkProgram 36/315 ms` (the in-image compiler; `precompiled hits 0`).
- GT: `gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; ...; 0
  skipped); precompiled hits 0, compiler runs 208`. `resets=0` on every row.

## Comparison with `cd738b1a` (the row before this one)

| Row | `cd738b1a` (2026-10-14) | `e3749a25` (2026-10-15) |
|-----|------|------|
| GM | done, 465, 33/0/0; terrain and shadow out of GPU memory, refract 1694 incomplete framebuffers, 2573 GL errors | done, 462, 33/0/0; **0 pool exhaustions, 0 incomplete framebuffers, 0 GL errors**; terrain 1 fps, shadow 3 fps, refract 1 fps (CPU-side time) |
| SO | done, crc16 c592 | done, crc16 c592, 56 fps |
| SV | empty (0 console bytes) | done, crc16 c592, twice |
| CL | done, crc16 110c, 75.3 fps | done, crc16 110c, 73.1 fps |
| GT | done, 50 / 50 | done, 50 / 50 |
