<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: gl_test 196, 200, 201-205 (2026-11-04)

- origin/main commit the image was built from:
  `67b27e2614e91845602d508425d234f712543a2c` (the newest main ref in the
  clone; `git fetch origin` failed, no access). It contains e883be8f (integer
  clear program) and 9b1ce2ff (occlusion counters).
- Image: `tools/cts/common-images.sh --cases 196,200,201,202,203,204,205 pi4
  tools/cts/common-tree.txt <dir> GT`, board `rpi_4b`, one context, build
  stamp `V3D_BUILD=67b27e26`.
  sha256 `0fc581955c17e0b941e74a563e1889e338abfa05de11e724f0ef0a551e82ac83`;
  the board read the staged file back with the same sha256.
- Run by hand (the serial console is held by the session's console bridge, so
  `tools/board.sh pi4 run` could not capture it): image copied to
  `/boot/firmware`, `tryboot.txt` naming it as the kernel, `reboot "0
  tryboot"`, console read through the bridge. The board's PM watchdog
  (`board: PM watchdog resets the SoC ~210 s after boot`) returned it to
  Linux; the staged image was removed and `tryboot.txt` emptied again.
- `console.txt` is the Zephyr side of the console (firmware and Linux lines
  left out, the repeated "last 22 lines" block dropped).
- An earlier run at main 91583704 (image sha256 `5bb9b0b2...`) gave the same
  verdicts.

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
compiles their bodies only with `GL_TEST_PVR`, so the run measures nothing
about the V3D's integer render targets. The final verdict is `SKIP`; the
`FAIL` line after each `SKIP` is the case wrapper's, not a counted failure
(`0 of 5 cases failed`). OSAL: no GPU reset, no MMU fault, no timeout.
`GL error 0x502 in glDrawArrays` is printed once before case 205's checks (all
15 pass).

Not done in this slice: the dEQP-GLES3 fbo.render probe (no gles3 stage 1 in
the dependency cache, and VK-GL-CTS with its externals is not fetched; the
build does not fit the 60-minute bound).
