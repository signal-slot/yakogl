<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: gl_test 196, 200, 201-205 (2026-11-02)

- origin/main commit the image was built from: `9158370457bff8faa72ddc4520bfeb4e8b6fed62`
  (the newest main ref in the clone; `git fetch origin` failed, no access). It
  contains e883be8f (integer clear program) and 9b1ce2ff (occlusion counters).
- Image: `tools/cts/common-images.sh --cases 196,200,201,202,203,204,205 pi4
  tools/cts/common-tree.txt <dir> GT`, board `rpi_4b`, one context.
  sha256 `5bb9b0b223eb2675a16f5184e3662cacc8f94e9babacf745124f04ce543380a1`
  (the build was repeated and gave the same hash). Built before the run; the
  board read the staged file back with the same sha256.
- Run by hand (the serial console was held by the session's console bridge, so
  `tools/board.sh pi4 run` could not capture it): image staged as the tryboot
  kernel, `reboot "0 tryboot"`, console read through the bridge. The PM
  watchdog (`board: PM watchdog resets the SoC ~210 s after boot`) returned
  the board to Linux; the staged image and tryboot.txt were removed after.
- `console.txt` is the Zephyr side of the console (Linux boot noise and the
  firmware lines left out).

Verdict lines:

```
gl_test: case 196 occl: PASS
gl_test: case 200 intrt: SKIP (integer render targets are the PowerVR's only)
gl_test: case 200 intrt: FAIL
gl_test: case 201 usamp: SKIP (integer textures are the PowerVR's only)
gl_test: case 201 usamp: FAIL
gl_test: case 202 blitconv: PASS
gl_test: case 203 tfcap: PASS
gl_test: case 204 srgb: PASS
gl_test: case 205 mappbo: PASS
gl_test: all: 0 of 5 cases failed (5 passed; 0 timed out; ...; 2 skipped, first skip case 200)
v3d_gl_test: 5 of 5 cases passed (542 ms)
```

Case 200 and 201 do not run on the V3D: `driver/tools/pvrlab/gl_test.c`
compiles their bodies only with `GL_TEST_PVR`, so the run measures nothing about
the V3D's integer render targets. The final verdict is `SKIP`; the `FAIL` line
that follows each `SKIP` is the case wrapper's, not a counted failure
(`0 of 5 cases failed`). OSAL: no GPU reset, no MMU fault, no timeout.
`GL error 0x502 in glDrawArrays` is printed once before case 205's checks (all
15 pass).
