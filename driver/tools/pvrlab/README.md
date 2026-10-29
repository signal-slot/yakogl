<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test: the on-silicon GL suite

`gl_test.c` is the GL suite every board runs: each case draws through the public
GL ES API and checks pixels read back with `glReadPixels`. The Zephyr samples
`gl_test` (PowerVR), `gc_gl_test` (GC7000) and `v3d_gl_test` (V3D) call
`gl_test_run()`. The header comment lists every case.

- `gl_test_shaders.h`, `gl_test_shaders_es100.h`: the GLSL sources the cases
  use, generated from `tools/shaders/` by `gen_gl_test_shaders.py`. The offline
  shader tables match sources by SHA-256, so never edit them by hand: change the
  file under `tools/shaders/`, then regenerate.
- `runner/`: the gl_test runner (Rust, the crate `yakogl-gltest-runner`), the
  walk `gl_test_run()` calls after its bring-up: which cases run (the
  regression set `SET_ORDER`, the list `gl_test_set_list()` stored, one case),
  the skips by name, each case's `skipped` / `SKIP` / `PASS` / `FAIL` /
  `TIMEOUT` line, the tally a board's record reads and the `gl_test: all:`
  line. The case bodies stay C and are called through FFI; a case moves to
  Rust later by replacing its function and its arm of the C switch. Every
  build that links `gl_test.c` links the runner: the Zephyr images and the
  host harness through `libyakogl.a`'s `gl-test-runner` feature, panthor-gltest
  as a crate dependency. `cargo test -p yakogl-gltest-runner` pins the lines
  and the count on the host.

`tools/check.sh` compiles `gl_test.c` on the host for the PowerVR and GC7000
arms, and runs `gen_gl_test_shaders.py --check`, which regenerates both headers
and refuses a drift against the committed ones. The directory name is historical:
it once held a Linux kernel-module harness for the PowerVR, now removed.

## Case 217 (`forcedpr`): a forced partial render, read back whole

The minimal repro of the firmware's partial render (issue #3; design:
`docs/notes/pvr-render-pool.md`, "Design: the firmware's partial render", "The
first board measurement", step 1). PowerVR only. Case 175 (`refusehang`) shows
a refused grow; this case checks the picture. In one kick, with the global free
lists capped at 512 pages (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512`:
no grow is possible), it draws `CONFIG_GL_TEST_FORCEDPR_LAYERS` (default 256)
overlapping 512x512 rectangles into a 1024x1024 target with a 16-bit depth
buffer and the depth test on (LESS). Each rectangle has its own colour and a
distinct, scrambled depth, so the winner at a pixel is not the last one drawn
and a partial render in the middle of the draw must keep the depth it has
already tiled. The parameter data of the frame exceeds the list. The case
reads the whole buffer back and compares it, pixel for pixel (a channel may
differ by 1), with the image it computes from the layers' rectangles, colours
and depths; a pixel no rectangle covers is the opaque black clear.

It prints one verdict line:

```
gl_test: forcedpr: PASS|FAIL stopped|FAIL pixels|NOT EXERCISED: N layers: requests R (served S, refused F), notices O, stop C, partial renders P, GPU recoveries H
gl_test: forcedpr: D of 1048576 pixels differ; first at (x,y) from the bottom left: got 0x... want 0x...
```

`partial renders` is the UPDATE_STATS count the driver keeps
(`pvr_render_fl_counters`, word 8) and `GPU recoveries` the firmware's HWR
count (`pvr_render_hwr_recoveries`), both as deltas over the case. PASS needs
the image equal, a partial render (P >= 1), stop 0 and 0 recoveries. A frame
that completed without a partial render did not reach the mechanism: NOT
EXERCISED, a failure; raise the layers.

- **Knob off (the known negative).** The geometry phase runs out of parameter
  memory with nothing to render, the kick never completes, and the render
  service ends the run at its 10 s stop (cause 4; cause 2 with the grow knob).
  The case then sees stop != 0, prints `FAIL stopped` without reading the
  buffer and returns a failure; the stop's record and `GPU recoveries` are the
  board record.
- **Knob on** (`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y`): the kick
  completes with P >= 1 and 0 recoveries, and the image is equal.

**It is not in the default list and must not join it until the knob lands on:**
with the knob off it hangs by design (the 10 s stop). Run it by number
(`CONFIG_GL_TEST_CASES="217"`, or `gl_test_set_list`).

**The two images** come from `driver/tools/pvrlab/forcedpr-images.sh <out-dir>`
(`--dry-run` prints the two `west build` argument lists and needs nothing;
`--layers N` sets the layer count). Both are the `gl_test` sample for
`verdin_am62p/am62p54/a53` with `CONFIG_GL_TEST_CASES="217"`,
`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512` and the board's watchdog
self-arm (`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`,
`CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`, what `tools/board.sh` wants), and
differ in one line:

| image | knob | expected |
|---|---|---|
| `<out-dir>/off` | `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n` | `FAIL stopped`, stop cause 4 (or 2) |
| `<out-dir>/on` | `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y` | `PASS` (once the frame side lands) |

It needs `ZEPHYR_WS` (and `WEST_VENV`) as `tools/cts/common-images.sh` does and
`driver/src/fw/pvr_fw_blob.h` (`driver/tools/mkfwblob.sh`). Run an image with
`tools/board.sh am62p run <out-dir>/<off|on>/zephyr/zephyr.bin <logdir>`.
