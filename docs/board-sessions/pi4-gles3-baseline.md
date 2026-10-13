<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gles3-baseline: Pi 4 ES 3.0 gl_test cases and the first dEQP-GLES3 baseline

Date: 2026-10-14. Integration tip `cd738b1a`
(`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`). HEAD8 = `cd738b1a`.

The images were built from the ccfarm workspace checkout (a detached
worktree is not what this session could make at first, and the checkout is
clean at the tip). Between the first and the second build the checkout gained
one commit of this record's own, which only adds files under `docs/`, so the
build stamps read `cd738b1` for the case 196 image and `3f0ab31` for the
others; the code is that of `cd738b1a` in all of them. The two gl_test images
carry `-dirty` because the default of `V3D_GL_TEST_CASE` / `V3D_GL_TEST_CASES`
in `driver/samples/zephyr/v3d_gl_test/Kconfig` was changed in the working tree
for the build and put back (nothing of that is committed). The dEQP images are
clean.

**Result.** gl_test on the Pi 4 (V3D 4.2): **196 `occl` FAIL, 202 `blitconv`
PASS, 203 `tfcap` PASS, 204 `srgb` FAIL (the render half refuses by name),
205 `mappbo` PASS.** 0 MMU faults, 0 recoveries, 0 resets in both runs.
dEQP-GLES3, 7 of the 91 images of `tools/cts/pi4-gles3-tree.txt` ran, each
with a complete record and 0 cases without a result.

## Environment (names only)

The session exported `BOARD_HOST` (the lease's ssh login, `RESOURCE_PI4_ELEGANCE_SSH`)
in the command that runs `tools/board.sh`; `BOARD_UART` (the serial bridge),
`BOARD_HDMI_TIMINGS`, `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and
`RUSTUP_TOOLCHAIN=nightly` were in the shell already. `DEQP_WORK` for the
gles3 images was the gles3 stage 1 in the deps cache,
`deqp-stage1-rpi_4b-gles3/work` (present already, 203 s build by `stage1-pi4`'s
recipe; nothing was rebuilt). `CMAKE_POLICY_VERSION_MINIMUM=3.5`.
`BOARD_BOOT` was not set (the default for the Pi 4 is `tryboot`).

## gl_test

Pi 4 symbols: `CONFIG_V3D_GL_TEST_CASE` (one case alone) and
`CONFIG_V3D_GL_TEST_CASES` (a list); `CONFIG_GL_TEST_CASES` is the AM62P
sample's. `tools/cts/common-images.sh` has no way to set them, so the Kconfig
default was edited for each build and restored.

```
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/g196 GT     # case 196
GT rc=0 sha256=caa566e61d867d239105a0b504ab94ac79f9da040952decf78aed06a16cfdaad build=V3D_BUILD=cd738b1-dirty
tools/board.sh pi4 run build/g196/GT/zephyr/zephyr.bin build/rec196 c196
board.sh pi4 c196 rc=4 verdict=failed boot=tryboot row boot_verdict=complete result=legacy:gl_test_case resets=0 record=absent back_after=257s

tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/g202 GT     # list 202,203,204,205
GT rc=0 sha256=642a188bc02390062251d3059e8f8e0f293464dba6b94360918f4cbad9af371f build=V3D_BUILD=3f0ab31-dirty
tools/board.sh pi4 run build/g202/GT/zephyr/zephyr.bin build/rec202 c202-205
board.sh pi4 c202-205 rc=4 verdict=failed boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end resets=0 record=absent back_after=257s
```

### 196 `occl` alone: FAIL (expected PASS)

```
gl_test: start (case=196 pretransform=0 ppp_ctrl=0x0 debug=0 no_coalesce=0)
gl_test: occl: q0 ANY hidden: available 1 result 0 (want 1 0)
gl_test: occl: q1 ANY visible: available 1 result 0 (want 1 1)
gl_test: occl: q2 CONSERVATIVE empty: available 1 result 0 (want 1 0)
gl_test: occl: q3 ANY two kicks: available 1 result 0 (want 1 1)
gl_test: occl: q4 ANY then covered: available 1 result 0 (want 1 1)
gl_test:   occl: the white quad on top  (32,32) = 0xffffffff  want 0xffffffff  ok  [check 1]
gl_test:   occl: the occluder in the corner ( 2, 2) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
gl_test: case 196 occl: FAIL
v3d_gl_test: 0 of 1 cases passed (184 ms)
slow jobs: 0 ...; MMU faults: 0; recoveries: 0; hung: none
```

Finding: `available 1` for all five, but the results are 0 for all five, q1 too.
HANDOFF's reading for "q0 or q1 wrong" is "the counter does not count (V3D:
`OCCLUSION_QUERY_COUNTER`)". So on the board the V3D back end's counter is
not read, or not enabled, although it is counted on the host. q3 is the same
0, so the "TLB does not add across jobs" question is not answered by this run.
Both pixel checks pass: the draws themselves are right. The expectation is
unchanged.

### 202 to 205 (one image, the list `202,203,204,205`)

| Case | Name | Result |
|---|---|---|
| 202 | blitconv | PASS, 12 of 12 checks (HANDOFF: 12/12) |
| 203 | tfcap | PASS, 9 of 9 checks (HANDOFF: 9/9) |
| 204 | srgb | FAIL |
| 205 | mappbo | PASS, 15 of 15 checks |

```
gl_test: all: 1 of 4 cases failed (3 passed; 0 timed out; first failure case 204 at index 2; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 29
slow jobs: 0 (past 1000 ms, ended; the slowest 0 ms); MMU faults: 0; recoveries: 0; hung: none
v3d_gl_test: 3 of 4 cases passed (477 ms)
gl_test: case 202 blitconv: PASS
gl_test: case 203 tfcap: PASS
gl_test: case 204 srgb: the render half is the PowerVR's (this GPU refuses an sRGB target by name)
gl_test: case 204 srgb: FAIL
gl_test: case 205 mappbo: PASS
```

Finding for 204: on the V3D the case stops at its render half ("this GPU
refuses an sRGB target by name") and prints FAIL, with no `check` lines; the
sample half (texel decode and the LINEAR filter after the decode) did not run.
HANDOFF says the sample half runs on all three boards. Either the case's
guard for the V3D skips the render half but still counts the refusal as a
failure, or the sRGB target refusal is reached before the sample half. Read
`driver/tools/pvrlab/gl_test.c` case 204 before the next session; the
expectation is not edited.

## dEQP-GLES3 on the Pi 4

```
export DEQP_WORK=<deps cache>/deqp-stage1-rpi_4b-gles3/work   DEQP_MODULE=gles3
tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt build/deqp-gles3 A1
tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt build/deqp-gles3b B1 C1 D1 FB1 FB2 FB3 FB4
export BOARD_MUSTPASS=<absolute path of tools/cts/gles3-mustpass.txt>
TREE_IMAGE_DIR=build/deqp-gles3  TREE_ROWS=0   tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt build/rec-gles3
TREE_IMAGE_DIR=build/deqp-gles3b TREE_ROWS=1-7 tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt build/rec-gles3b
```

Eight images built, `rc=0` each (A1 `db31d6d1`, B1 `8c328b78`, C1 `4d30961a`,
D1 `9043128d`, FB1 `b64102dd`, FB2 `4acbf938`, FB3 `3e51f1f9`, FB4 `83aa0f28`;
the sha256 prefixes). **Images that ran: A1, B1, C1, D1, FB1, FB2, FB3.**
FB4 was built and not run, and rows 8 to 90 were not built or run.

The runner (`tools/board.sh pi4 tree` with rows 1-7) was stopped by the
session's 10 minute background limit while FB3 was on the board. FB3's console
is complete (its `deqp: summary` line is there, below) but the runner did not
print FB3's verdict line or its own tally, and left `yakogl-FB3.bin` and
`tryboot.txt` staged on the board. They were removed over ssh afterwards
(`/boot/firmware` held no `yakogl` file after that) and the board was in Linux,
idle. A longer session should start the tree in the background with the
longest timeout, or run it a few rows at a time.

Verdict lines (A1 to FB2; FB3 has none, see above):

```
board.sh pi4 A1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=296s
board.sh pi4 B1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 C1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=306s
board.sh pi4 D1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 FB1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=298s
board.sh pi4 FB2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=306s
```

The consoles' own summary lines:

| Image | passed | failed | not supported | begun / with a result | time |
|---|---|---|---|---|---|
| A1 | 438 | 59 | 0 | 497 / 497 | 28.8 s |
| B1 | 695 | 0 | 0 | 695 / 695 | 23.1 s |
| C1 | 602 | 0 | 0 | 602 / 602 | 24.9 s |
| D1 | 354 | 15 | 76 | 445 / 445 | 18.7 s |
| FB1 | 454 | 137 | 7 | 598 / 598 | 18.8 s |
| FB2 | 187 | 308 | 146 | 641 / 641 | 24.8 s |
| FB3 | 68 | 225 | 36 | 329 / 329 | 12.9 s |
| **7 images** | **2,798** | **744** | **265** | **3,807 / 3,807** | |

Every image: `MMU faults: 0; recoveries: 0`; D1 had one slow job (3,152 ms).
Each image runs in under 30 s of its 240 s window, so the tree table (sized
for the AM62P's slower rates) can take far larger images on the Pi 4; the
91-image split is conservative.

`tools/board.sh pi4 tally` was not run over a directory (the runner's own
tally did not print, see above); the per-case failures come from
`tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt
--statuses <console>`, which counts only cases in the mustpass list. Its
failure lines are 740 (A1 59, D1 14, FB1 134, FB2 308, FB3 225; B1 and C1 0):
FB1 is 3 short of the console's 137 and D1 is 1 short of 15, so those cases
are outside `gles3-mustpass.txt`. They are in
`docs/board-sessions/pi4-gles3-baseline-fails.tsv`.

What the failures are (case-name prefixes, from the tsv):

- A1 (59): `attribute_location.mixed*` 48 and `implementation_limits.*` 11
  (`max_draw_buffers`, `max_color_attachments`, the uniform and varying
  component limits, `compressed_texture_formats`).
- D1 (14): `draw.random.*`.
- FB1 to FB3: `fbo.blit.conversion` 261, `fbo.completeness.renderable` 105,
  `fbo.color.repeated_clear` 72, `fbo.color.tex3d` 32, `tex2darray` 32,
  `fbo.blit.default_framebuffer` 31, `completeness.attachment_combinations`
  28, `fbo.color.texcube` 24, `tex2d` 24, `clear` 24, `fbo.blit.depth_stencil`
  16, `color.blend` 7, `fbo.depth` clamp 10, `completeness.size` 1.
  These are the integer-render-target, MRT, 2D-array and blit items of the
  plan on the V3D; the numbers are the Pi 4's first baseline and are not yet
  classified case by case.

B1 (`buffer`) and C1 (`clipping`) pass in full: 1,297 cases, 0 failures.

## Verdict

gl_test: 3 of 5 cases pass (202, 203, 205); 196 and 204 fail on the V3D.
dEQP-GLES3: 7 images, 3,807 cases, 2,798 pass / 744 fail / 265 not supported,
all with a result. Not run: 84 of 91 images (FB4 is built, the rest are
not). ES 2.0 regression rows were not run in this session.

## Records

The board's records stay under the gitignored `build/` of the workspace
(`build/rec196`, `build/rec202`, `build/rec-gles3`, `build/rec-gles3b`); they
are not committed.
