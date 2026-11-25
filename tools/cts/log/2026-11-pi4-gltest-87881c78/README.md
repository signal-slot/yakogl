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
  `4df863d859568787f5c32451c1c6803ebc30efa3b2b9a2ed0aee0e1318215fca`, read
  back the same on the board.
- `GT245` (`--cases 245`, a fresh context): sha256
  `3d644cbfae914ab4e6b9c84bcdc6d5d2048a56bee1bd90a4db48fa2785d6ff8a`. It
  links, but the plain `--cases 245` image has the `mrt4` knob off
  (`CONFIG_YAKOGL_TEST_KNOBS` and `CONFIG_V3D_GL_TEST_MRT4` are not set by
  `common-images.sh`), so the case prints SKIP: it did not draw four targets.

## Verdicts

`GT`: `v3d_gl_test: 20 of 21 cases passed (2396 ms)`, `gl_test: all: 1 of 21
cases failed (20 passed; 0 timed out; first failure case 228 at index 7; ...
3 skipped, first skip case 206)`.

```
v3d_gl_test: case 199 PASS      es3trunc
v3d_gl_test: case 206 SKIP      es3levels (the V3D does not sample 3D or 2D-array textures)
v3d_gl_test: case 223 SKIP      msaarbo (GL_MAX_SAMPLES 1 < 4: 4x FBOs are off, cargo feature msaa-fbo)
v3d_gl_test: case 224 PASS      negapi_texsubimage2d
v3d_gl_test: case 225 PASS      negapi_texsubimage3d
v3d_gl_test: case 226 PASS      negapi_uniformui
v3d_gl_test: case 227 PASS      negapi_uniformuiv
v3d_gl_test: case 228 FAIL      negapi_readpix_fmt
v3d_gl_test: case 229 PASS      tf_deleted_output
v3d_gl_test: case 230 - 243 PASS  (flat_lines ... divmix: 14 cases)
v3d_gl_test: case 244 SKIP      w128tex (the two-plane RGBA32 path is the GC7000's)
```

`GT245`: `gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS 3 < 4)`,
`v3d_gl_test: case 245 SKIP`.

Case 228 fails on its multisample part, not on the read-back formats (all six
ReadPixels checks read the expected `0x502`): `RenderbufferStorageMultisample(4,
RGBA8)` gave GL error `0x502`, want `0x0`; the multisample FBO status was
`0x8cd6`, want COMPLETE; `SAMPLE_BUFFERS` read 0, want 1. The Pi 4's
`GL_MAX_SAMPLES` is 1 here (the same reason as case 223's SKIP), so the case's
multisample expectation does not hold on this build. No OSAL fault in either
image: 0 slow jobs, 0 MMU faults, 0 recoveries, nothing hung, no allocation
refused.

## Files

`GT.console.txt` and `GT245.console.txt` are the Zephyr side of the console
(firmware and Linux lines left out, the repeated "last 22 lines" block and
some OSAL statistics lines dropped). In `GT.console.txt` the passing cases'
per-check `ok [check n]` lines and the negative-API GL error lines of cases
224-227 are elided (marked in the file); every line kept is verbatim. The PM watchdog
returned the board to Linux after each image, the staged image and
`tryboot.txt` were removed and read back (none left), and `config.txt` was not
touched. Run by hand, as `../2026-11-pi4-gltest/` was, because the leased
console tool holds the serial bridge.
