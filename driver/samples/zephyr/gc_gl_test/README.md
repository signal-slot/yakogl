<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_gl_test: the GL suite on the i.MX8MP's Vivante GC7000UL

`driver/tools/pvrlab/gl_test.c` is the on-silicon suite every GPU is held to:
each case reads its pixels back. The same file compiles here. Only the lines
that start the back end know which GPU this is, and they are behind
`CONFIG_YAKOGL_GPU_VIVANTE`.

Current results are in `tools/cts/STATUS.md`. The bring-up diary of
September 2026 (hold-time readings, results per case, the host build) is
`ab43a20c5:tools/cts/log/board-sessions/imx8mp-gc-gl-test-2026-09.md`.

## Build and run

The common-set image (the default case list, with the board's watchdog
self-arm) comes from `tools/cts/common-images.sh`. A sweep image (the online
compiler, the ES 1.00 shader set, 48 case numbers from a first case) comes
from `tools/cts/imx8mp-gltest-build.sh <name> <first case>`; it needs the
variables of `tools/cts/imx8mp-env.sh`.

Run an image with `tools/board.sh imx8mp run <zephyr.bin> <logdir>`: U-Boot
starts it (`docs/standalone-boot.md`), and the runner reads the result record
afterwards. With a sweep image, `SEL=<case - first>` runs one case alone;
`SEL=256` runs the whole range.

Kconfig (`Kconfig` in this directory): `GC_GL_TEST_CASE`, the sweep options
`GC_GL_TEST_SWEEP*`, `GC_GL_TEST_SKIP`, the diagnostic reports
(`GC_GL_TEST_REPORT_*`, `GC_GL_TEST_DEPTH_*`), and the window size.

If something else owns the panel, build without the display driver and the
panel console: `-DCONFIG_SAFEUI_DISPLAY_IMX8MP=n
-DCONFIG_GC_COMMON_PANEL_CONSOLE=n`. To check an image is free of the display
driver, count its functions: `nm zephyr.elf | grep -c 'imx8mp_display_'` must
be 0. A loose grep for `imx8mp_display` also matches the absolute Kconfig
symbol `CONFIG_DT_HAS_SAFEUI_IMX8MP_DISPLAY_ENABLED`, which is set whenever the
devicetree has the node.

## The test knobs

This image selects `CONFIG_YAKOGL_TEST_KNOBS` (`GC_GL_TEST_KNOBS`), the
library's `test-knobs` feature: the GC7000 switches and probes the suite sets
to run an experiment's arms (`gc_debug_set_msaa` and the 4x scratch probes
for case 16, `gc_debug_set_narrow_la` for case 132's RGBA8 arm,
`gc_debug_set_flip_experiment` for case 88). gl_test.c calls them only under
that define, and a product image has none of them (`tools/check.sh`, "no test
knob in the artefact").

## Reading a run

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

## A probe can change the path it is measuring

Case 53 was written to isolate a depth clear so the stored value could be
read back. Isolating it moved the clear onto another code path: a clear with
no draw behind it builds a draw of its own, and that draw left `depth_range`
at `{0, 0}`. Every such clear stored 0, while the path case 13 uses was
correct. Case 53 now checks both paths (checks 1-2: the clear alone, then
drawn over; checks 3-4: a draw takes the clear along). When a probe isolates
the thing under test, check that it has not moved it onto different code.
