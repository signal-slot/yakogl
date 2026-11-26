<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: the ES 3.0 gl_test cases 199, 206, 223-245 at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Item board-es2 of plan #399 (issue #1261): the ES 3.0 `gl_test` cases of
`driver/tools/pvrlab/gl_test.c` that the Pi 4 record
`../2026-11-pi4-gltest/` (cases 196, 200-205 at `67b27e26`) did not run, on
the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot). Built from a detached
worktree of `87881c78` with `tools/cts/common-images.sh --cases LIST pi4
tools/cts/common-tree.txt <dir> GT`; both images printed `rc=0
build=V3D_BUILD=87881c78`.

- `GT` (`--cases 199,206,223,224,...,244`, one context, 24 cases): sha256
  `8799a90d89eb3fec7a779138438dffa9053462b65bd20e14b60bce32c0eb2578`, read
  back the same on the board.
- `GT245` (`--cases 245`, a fresh context): sha256
  `713d3c2a144a0833d318521fbb16de656fc429f4aed4c644182dbda1c06054d5`, read
  back the same. It links, but the plain `--cases 245` image has the `mrt4`
  knob off (`CONFIG_YAKOGL_TEST_KNOBS` and `CONFIG_V3D_GL_TEST_MRT4` are not
  set by `common-images.sh`, and `v3d_debug_set_draw_targets` is not defined
  in `src/v3d`), so the case prints SKIP: it did not draw four targets.

## Verdicts

`GT`: `v3d_gl_test: 20 of 21 cases passed (2396 ms)`; `gl_test: all: 1 of 21
cases failed (20 passed; 0 timed out; first failure case 228 at index 7; ...
3 skipped, first skip case 206)`. Per case: 199 PASS; 206 SKIP (the V3D does
not sample 3D or 2D-array textures); 223 SKIP (GL_MAX_SAMPLES 1 < 4); 224-227
PASS; 228 FAIL; 229-243 PASS; 244 SKIP (the two-plane RGBA32 path is the
GC7000's).

`GT245`: `gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS 3 < 4)`,
`v3d_gl_test: case 245 SKIP`.

Case 228 (`negapi_readpix_fmt`) fails on its multisample part, as in the
`GT` run's own first-failure line (index 7); its per-check lines are not kept
in the excerpt. The Pi 4's `GL_MAX_SAMPLES` is 1 (the reason of case 223's
SKIP). No OSAL fault in either image: 0 slow jobs, 0 MMU faults, 0
recoveries, nothing hung, no allocation refused.

## Files

`GT.console.txt` and `GT245.console.txt` are excerpts of the Zephyr side of
the console, copied from the leased console tool (the serial bridge was held
by it, so `tools/board.sh`'s own capture was not available): the firmware and
Linux lines, the per-check `ok [check n]` lines, the pixel dumps, the
negative-API GL error lines and cases 224-242's PASS lines are left out
(marked `# ...` in `GT.console.txt`; the verdict block lists every case).
Every line kept is verbatim.

Method, by hand in `tools/board.sh`'s order: the image installed to
`/boot/firmware` as `yakogl-<LABEL>.bin` with a `tryboot.txt` naming it, both
read back (the image by sha256), `sudo reboot "0 tryboot"`, the console read
to the `RECORD end` line, the wait for Linux (a new boot id each time), the
staged image and `tryboot.txt` removed and read back (none left; the one-byte
`tryboot.txt` an earlier session left was removed with them, `config.txt`
untouched). The PM watchdog returned the board to Linux after each image.
