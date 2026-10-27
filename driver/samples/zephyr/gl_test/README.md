<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test (Zephyr: Verdin AM62P, Verdin iMX8M Plus, Armadillo-X2, Raspberry Pi 4)

Runs `driver/tools/pvrlab/gl_test.c`, YakoGL's on-silicon test suite, on the
three GPUs: the Verdin AM62P's PowerVR, the i.MX8M Plus's Vivante GC7000UL
and the Pi 4's V3D. It is one sample for every board: the Zephyr board (`-b`)
picks the GPU, `src/main_am62p.c`, `main_imx8mp.c` or `main_rpi4.c` (the lines
that start the back end and the board's way to give a verdict) and the files
in `boards/` (`imx8mp.conf` and `armadillo_x2.conf` are added by
`CMakeLists.txt` for the i.MX8M Plus boards). The options of each GPU are in
their own menu of `Kconfig`. Every case reads its pixels back and prints
`gl_test: case N <name>: PASS/FAIL`; a run ends with
`gl_test: all: N of M cases failed`. The header comment of `gl_test.c` lists
every case. Current results are in `tools/cts/STATUS.md`.

```sh
# once per checkout: embed the board's DDK firmware (TI/Img property, never
# committed, and .gitignored -- so a FRESH clone or git worktree has nothing
# here and the build stops at `fatal error: fw/pvr_fw_blob.h`)
driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796   # -> driver/src/fw/pvr_fw_blob.h
# ...or, when the firmware itself is not on the host any more, copy the
# generated header from another checkout: it is a build artefact of the same
# firmware and does not depend on the branch.
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_am62p/am62p54/a53 -d build/gl_test \
    yakogl/driver/samples/zephyr/gl_test
yakogl/tools/board.sh am62p run build/gl_test/zephyr/zephyr.bin <logdir>
```

`tools/board.sh` needs an image with the RTI self-arm
(`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`); `tools/cts/common-images.sh` builds
the common-set image that way.

Kconfig: `GL_TEST_CASE` (7 = all), `GL_TEST_CASES`, `GL_TEST_PANEL` (1 = demo
scene, 2 = spinning MSAA cube on the DSI panel, shown through the DSS
application plane), `GL_TEST_DEBUG` (1 = command-stream dumps, 2 =
depth/stencil dumps), `GL_TEST_PRETRANSFORM`. The board glue (watchdog, hold
and reset, the DSS plane) is the board layer, `driver/zephyr/lib/board/`.

## Verdin iMX8M Plus and Armadillo-X2 (Vivante GC7000UL)

Only the lines that start the back end know which GPU this is, and they are
behind `CONFIG_YAKOGL_GPU_VIVANTE`.

Current results are in `tools/cts/STATUS.md`. The bring-up diary of
September 2026 (hold-time readings, results per case, the host build) is
`ab43a20c5:tools/cts/log/board-sessions/imx8mp-gc-gl-test-2026-09.md`.

### Build and run

The common-set image (the default case list, with the board's watchdog
self-arm) comes from `tools/cts/common-images.sh`. A sweep image (the online
compiler, the ES 1.00 shader set, 48 case numbers from a first case) comes
from `tools/cts/imx8mp-gltest-build.sh <name> <first case>` (`prj_sweep.conf`); it needs the
variables of `tools/cts/imx8mp-env.sh`.

Run an image with `tools/board.sh imx8mp run <zephyr.bin> <logdir>`: U-Boot
starts it (`docs/standalone-boot.md`), and the runner reads the result record
afterwards. With a sweep image, `SEL=<case - first>` runs one case alone;
`SEL=256` runs the whole range.

Kconfig (`Kconfig`, the i.MX8M Plus menu): `GC_GL_TEST_CASE`, the sweep options
`GC_GL_TEST_SWEEP*`, `GC_GL_TEST_SKIP`, the diagnostic reports
(`GC_GL_TEST_REPORT_*`, `GC_GL_TEST_DEPTH_*`), and the window size.

If something else owns the panel, build without the display driver and the
panel console: `-DCONFIG_YAKOGL_DRV_DISPLAY_IMX8MP=n
-DCONFIG_GC_COMMON_PANEL_CONSOLE=n`. To check an image is free of the display
driver, count its functions: `nm zephyr.elf | grep -c 'imx8mp_display_'` must
be 0. A loose grep for `imx8mp_display` also matches the absolute Kconfig
symbol `CONFIG_DT_HAS_YAKOGL_IMX8MP_DISPLAY_ENABLED`, which is set whenever the
devicetree has the node.

### The test knobs

The AM62P and i.MX8MP images select `CONFIG_YAKOGL_TEST_KNOBS` (`GL_TEST_KNOBS`), the
library's `test-knobs` feature: the GC7000 switches and probes the suite sets
to run an experiment's arms (`gc_debug_set_msaa` and the 4x scratch probes
for case 16, `gc_debug_set_narrow_la` for case 132's RGBA8 arm,
`gc_debug_set_flip_experiment` for case 88). gl_test.c calls them only under
that define, and a product image has none of them (`tools/check.sh`, "no test
knob in the artefact").

### The runner

Every gl_test image selects `CONFIG_YAKOGL_GL_TEST_RUNNER` (`GL_TEST_RUNNER`),
the library's `gl-test-runner` feature: the suite's walk is the gl_test runner
(`driver/tools/pvrlab/runner`, Rust), which `gl_test_run()` calls after its
bring-up. It owns which cases run (the regression set, the list
`gl_test_set_list()` stored, one case), the skips by name, each case's
`skipped` / `SKIP` / `PASS` / `FAIL` / `TIMEOUT` line, the tally the record
reads (`gl_test_passed` and the other globals) and the `gl_test: all:` line;
the case bodies are the C ones of `gl_test.c`, called one at a time. The
console text is the one the C walker printed, so `tools/board.sh` reads it
unchanged. A product image has none of its `gl_test_*` symbols (the same
`tools/check.sh` row as the knobs).

### Reading a run

The reading is the result record: four words in the SNVS that survive the SoC
reset, decoded by `yakogl-tools record`. It is the case and its first failing
check with what it read and wanted, or the set's summary (passed of run, the
first two failing cases, how many cases were skipped by name), or UNFINISHED
with the case a stalled suite was in. A skipped case is not run and is not a
failure. The image resets the SoC right after writing the record
(`gc_sample_finish_recorded`), with `GC_GL_TEST_SETTLE_SECONDS` at its default
0.

The diagnostic knobs whose answer has no record field
(`GC_GL_TEST_REPORT_COUNTERS`, `GC_GL_TEST_DEPTH_VALUE`,
`GC_GL_TEST_DEPTH_PROBE`) are read by the time the board takes to come back.
The table of hold times is in the diary named above.

### A probe can change the path it is measuring

Case 53 was written to isolate a depth clear so the stored value could be
read back. Isolating it moved the clear onto another code path: a clear with
no draw behind it builds a draw of its own, and that draw left `depth_range`
at `{0, 0}`. Every such clear stored 0, while the path case 13 uses was
correct. Case 53 now checks both paths (checks 1-2: the clear alone, then
drawn over; checks 3-4: a draw takes the clear along). When a probe isolates
the thing under test, check that it has not moved it onto different code.

## Raspberry Pi 4 (V3D 4.2)

The suite runs through YakoGL's GL core, its in-process compiler and its V3D
backend on the OSAL `src/osal/zephyr_v3d.c` (`docs/online-shaderc/v3d-backend.md`,
"M6 step B"), with the GL ES 1.00 shader set. The suite renders into its own
64x64 surface; the HDMI framebuffer is only the console. The verdicts are
repeated at the end, and the last lines of the run are frozen into one record
on the screen (`V3D_GL_TEST_RECORD*`) before the bounded-lifetime watchdog
(`V3D_GL_TEST_HOLD_S`) resets the board. Kconfig: `V3D_GL_TEST_CASES` (the
list), `V3D_GL_TEST_CASE` (one case alone), `V3D_GL_TEST_MSAA`.
