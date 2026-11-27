<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test (Zephyr: Verdin AM62P, Verdin iMX8M Plus, Armadillo-X2, STM32MP257F-DK, Raspberry Pi 4, Raspberry Pi 5, NanoPC-T6, Verdin iMX95)

Runs `driver/tools/pvrlab/gl_test.c`, YakoGL's on-silicon test suite, on the
GPUs: the Verdin AM62P's PowerVR, the i.MX8M Plus's Vivante GC7000UL, the
STM32MP257F-DK's Vivante GCNanoUltra31, the Pi 4's V3D, the NanoPC-T6's
Mali-G610 and the Verdin iMX95's Mali-G310 (built only). It is one sample for
every board: the Zephyr board (`-b`) picks the GPU, `main_am62p.c`,
`main_imx8mp.c`, `main_stm32mp2.c`, `main_rpi4.c` or `main_mali.c` (the
NanoPC-T6 and the Verdin iMX95; the lines that start the back end and the
board's way to give a verdict) and the files in `boards/` (`imx8mp.conf` and
`armadillo_x2.conf` are added by `CMakeLists.txt` for the i.MX8M Plus boards,
the board layer's `stm32mp2/app.conf` for the DK, the board layer's
`verdin_imx95/app.conf` and `app.overlay` and then `verdin_imx95.conf` here
for the Verdin iMX95, `../common/verdin_imx95.cmake`). The options of each GPU are in
their own menu of `Kconfig`. Every case reads its pixels back and prints
`gl_test: case N <name>: PASS/FAIL`; a run ends with
`gl_test: all: N of M cases failed` (the STM32MP257F-DK's one-case image
prints it too, as `gl_test: all: <0|1> of 1 cases failed ...`; see its
section below). The header comment of `gl_test.c` lists every case. Current
results are in `tools/cts/STATUS.md`.

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

## STM32MP257F-DK, Cortex-A35 (Vivante GCNanoUltra31)

The DK's GPU (model 0x8000, revision 0x6205; #1281) runs the same suite
through `driver/samples/zephyr/gl_test/src/main_stm32mp2.c` and the board layer (`yakogl_board.h`,
`driver/zephyr/lib/board/stm32mp2`), as `gc_fence` and `gc_clear` do, not the
i.MX8MP's `yakogl_gc_board`. Before `main()` the layer powers the GPU for the
driver (`gc_power_mp2.c`: SCMI over OP-TEE, the RCC gate and reset; anything
but 0x8000/0x6205 is refused with a PSCI reset) and arms OP-TEE's watchdog;
the run ends with the record and PSCI SYSTEM_RESET back into ST's Linux, at
`CONFIG_YAKOGL_STM32MP2_LIFETIME_S` (100 s here) at the latest. The board
files are the board layer's `app.conf` (added by `CMakeLists.txt`) and
`app.overlay` (included by `boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay`),
and `boards/stm32mp257f_dk_stm32mp257fxx_a35.conf` for gl_test's own (the
board layer on, the lifetime).

The DK image's libyakogl is built with the crate feature `vivante-st`
(`CMakeLists.txt` adds it to `YAKOGL_EXTRA_FEATURES` for this board only), so the
active Vivante profile is the GCNanoUltra31_VIP2's; the i.MX8MP image keeps its own.

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
# the first board run: case 1 ("uniform") alone
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/gl_test_mp2 \
    yakogl/driver/samples/zephyr/gl_test -- -DYAKOGL_WERROR=ON \
    -DCONFIG_GL_TEST_MP2_SET_HEAD=y
```

Run it with `tools/board-stm32mp2.sh` (the one-shot from the DK's own U-Boot,
back into ST's Linux; its header comment has the steps).

Kconfig (`Kconfig`, the STM32MP257F-DK menu):

- `GL_TEST_MP2_CASE`: the case to run, 7 the regression set (the default).
- `GL_TEST_MP2_SKIP` (default ""): cases the set leaves out; the set is the
  i.MX8MP's (case 7, `SET_ORDER`) minus this list. The default leaves
  nothing out: case 10 (prims, a 3 px wide LINE_LOOP), the one case held
  out while the Q11 line-width decision was open, passes on the DK.
- `GL_TEST_MP2_SET_HEAD` (off by default): run only the first
  `GL_TEST_MP2_SET_HEAD_CASES` cases of the regression set, in its order
  (`SET_ORDER`, `driver/tools/pvrlab/runner/src/lib.rs`), instead of
  `GL_TEST_MP2_CASE`. With the default of 1 that is case 1, "uniform" (one
  fragment-uniform draw, read back), alone: the DK's first GL run
  (`gc_gl_triangle`, `docs/notes/pre-study-stm32mp257f-dk.md` row P7). 2 adds
  case 2 (texture), and so on; more than one case runs on one context as a
  list.
- `GL_TEST_MP2_CASES` (default ""): an explicit comma list of case numbers
  (`"1,2,3"`, at most 64), one shard of the regression set: it overrides
  `GL_TEST_MP2_CASE` and `GL_TEST_MP2_SET_HEAD`, one number runs that case
  alone and more are walked on one context.
  `tools/cts/mp2-gltest-shards.py` writes one `shard-NNNN.conf` fragment
  with it per shard (build with `-DEXTRA_CONF_FILE=<out>/shard-NNNN.conf`).
  Anything but a comma-separated list of decimal case numbers makes the
  image print `gl_test: FAIL: CONFIG_GL_TEST_MP2_CASES is not a list of
  cases: <value>` and run nothing. Empty: as before.

A one-case run (`GL_TEST_MP2_CASE` other than 7, a `SET_HEAD` of one case,
or a one-number `GL_TEST_MP2_CASES`) has no set tally of its own, so the
image prints the runner's line for it, the same fields as its `all_line`:
`gl_test: all: <0|1> of 1 cases failed (<0|1> passed; 0 timed out; first
failure case <N|-1> at index <0|-1>; second failure case -1; <0|1> skipped,
first skip case <N|-1>); precompiled hits H, compiler runs R`. 0 failed is
a pass, 1 a failure; a case the image skips reads as 0 of 1 failed with 1
skipped. `tools/probes/stm32mp257f_dk/verdict/dk_verdict.py` reads the
verdict from this line.

The console has every case's line and `gl_test: all:`; the record is the
`yk-record:` line on USART2, kind CASE as on the i.MX8MP: one case gives
w1 = case << 16 | first bad check << 8 | status (1 pass, 2 a check failed,
3 failed with no check, 5 no GPU; check 255 none) with w2, w3 the checks made
and 0, or what the bad check read and wanted; a set or a head of more than
one case gives the set summary (status 7). Raise the lifetime (and the
runner's `RUN_BOUND`, 120 s) for anything longer than a few cases: past it
the layer resets the board in the middle of the suite.

Built for the DK with `-Werror`. The one-case tally line is confirmed on the
board: the one-case image built with `GL_TEST_MP2_SET_HEAD=y` from origin/main
`dd6cfa02` printed `gl_test: all: 0 of 1 cases failed (1 passed; ...)` and
passed, record
`records/stm32mp257f-dk/20261126-0118-gl_test_case1_tally-d3a5c59e`, console
excerpt committed in `08f03625`
(`docs/notes/stm32mp257f-dk-bringup.md`, "Board verdicts", step 8). The
`CONFIG_GL_TEST_MP2_CASES` single-number form builds (host build, `269e1839`)
but has not been run on the board.

## Raspberry Pi 4 (V3D 4.2)

The suite runs through YakoGL's GL core, its in-process compiler and its V3D
backend on the OSAL `src/osal/zephyr_v3d.c` (`docs/online-shaderc/v3d-backend.md`,
"M6 step B"), with the GL ES 1.00 shader set. The suite renders into its own
64x64 surface; the HDMI framebuffer is only the console. The verdicts are
repeated at the end, and the last lines of the run are frozen into one record
on the screen (`V3D_GL_TEST_RECORD*`) before the bounded-lifetime watchdog
(`V3D_GL_TEST_HOLD_S`) resets the board. Kconfig: `V3D_GL_TEST_CASES` (the
list), `V3D_GL_TEST_CASE` (one case alone), `V3D_GL_TEST_MSAA`, `V3D_GL_TEST_MRT4`.

Four-target knobs (measurements, off by default, gated by
`YAKOGL_TEST_KNOBS`): `V3D_GL_TEST_MRT4` (Pi 4) and `PVR_GL_TEST_MRT4` (AM62P)
make the image call `v3d_debug_set_draw_targets(4)` /
`pvr_debug_set_draw_targets(4)` at startup, before the first context, so
`GL_MAX_DRAW_BUFFERS` and `GL_MAX_COLOR_ATTACHMENTS` answer 4 instead of 3.
Products leave them off. Build with `-DCONFIG_V3D_GL_TEST_MRT4=y` (Pi 4) or
`-DCONFIG_PVR_GL_TEST_MRT4=y` (AM62P) and also `-DCONFIG_YAKOGL_TEST_KNOBS=y`,
which they depend on. The image prints one line naming the setter and its
return value; case 245 (`mrt4`) then runs instead of printing SKIP (it
prints SKIP unless `GL_MAX_DRAW_BUFFERS` is 4). The setters are the Rust
`knob` item's (`src/v3d/knob.rs`, `src/pvr/knob.rs`): an image with a knob on
links once they have merged; default images never reference them
(`docs/notes/mrt4-v3d-pvr.md`, "The image knobs").

## Raspberry Pi 5 (V3D 7.1)

The suite runs through YakoGL's GL core, its in-process compiler (the GL ES
1.00 shader set) and the V3D backend's 7.1 table (cargo feature `gpu-v3d71`)
on the OSAL `src/osal/zephyr_v3d71.c`. The table is chosen when the image is
built, not at run time (`docs/notes/v3d71-gl-table.md`, section 4).

Board files: `boards/rpi_5.conf` sets `CONFIG_YAKOGL_GPU_V3D` and
`CONFIG_YAKOGL_GPU_V3D71` and the 48 MiB carve-out, as on the Pi 4.
`CONFIG_YAKOGL_GPU_V3D71` (`zephyr/Kconfig`; it depends on `YAKOGL_GPU_V3D`
and `SOC_BCM2712`) selects the board layer's PM watchdog (`YAKOGL_RPI5_WDOG`)
and firmware mailbox (`YAKOGL_RPI5_MBOX`, the V3D clock), both defined in
`driver/zephyr/lib/board/Kconfig`. `boards/rpi_5.overlay` makes `sram0`
96 MiB from 0x200000, below 4 GiB and below the DTB the firmware places
above the image.

The image has no screen. The console is uart10, as in the
`tools/probes/rpi_5` probes, because the Pi 5's mailbox framebuffer
(`YAKOGL_BOARD_RPI5`) is not proven on the board yet. The options are the
Pi 4's menu, with these differences:

- `V3D_GL_TEST_CASES` defaults to P9's six cases, `5,58,59,66,67,102`, the
  list the Pi 4 started with (`docs/notes/pi4-v3d-bringup.md`, "gl_test on
  the V3D").
- `V3D_GL_TEST_HOLD_S` (200 s) is also the board layer watchdog's hold
  (`YAKOGL_RPI5_WDOG_HOLD_S`, 30 s in the probes).

```sh
# from the top of the west workspace, as tools/check.sh's zephyr_werror_image
# builds it (the module list comes from tools/cts/zephyr-modules.sh)
west build -p always -b rpi_5 -d build/rpi5_gl_test yakogl/driver/samples/zephyr/gl_test -- \
    "-DZEPHYR_MODULES=$(yakogl/tools/cts/zephyr-modules.sh "$PWD")" \
    "-DYAKOGL_DIR=$PWD/yakogl" -DYAKOGL_WERROR=ON
# in short (module list and YAKOGL_DIR as the workspace sets them):
#   west build -p always -b rpi_5 driver/samples/zephyr/gl_test -- -DYAKOGL_WERROR=ON
```

Expected results of the six cases (the first board run never reached them,
see "On the board" below): 5 of 6 pass (5, 58, 66, 67, 102); case 59
is refused by name, with the 4.2 string ("the draw reads stencil an earlier
job wrote, and the target has no depth/stencil buffer to keep it in",
`tests/v3d71_refuse.rs`). Case 67 is drawn with no refusal: the 7.1 job loads
alpha as 1 and clears mask bit 3 for a target with no alpha
(`docs/notes/v3d71-gl-table.md`, section 5; `tests/v3d71_refuse67.rs`). Any
further refusal must carry a name too.

The PM watchdog is armed for the run: `YAKOGL_RPI5_WDOG_HOLD_S` takes
`V3D_GL_TEST_HOLD_S`, so the run and its record end before it fires. It
then brings Linux back, about 210 to 230 s after the image started.

The record is UART only (no file, no screen). A run ends with the line
`v3d_gl_test RECORD end`, which `tools/board.sh pi5` waits for
(`END_RE_DEFAULT`).

Run (one shot through the Pi firmware's tryboot):

```sh
tools/board.sh pi5 run <image> <logdir>
```

The board's ssh host and console come from the environment only:
`BOARD_HOST` and `BOARD_UART` (or `PI5_HOST` and `PI5_UART`). Never write a
value into a file or a commit.

On the board (2026-11-27, `d2dbaa7e`): the first run, one
`tools/board.sh pi5 run` of the image built from `85afd0cf` (record
`records/rpi5/20261127-1119-gltest71-85afd0cf`, gitignored, redacted): all
six cases FAIL for **one** named cause, logged once before case 5 —
`yakogl/v3d71: draw refused: V3D 7.1: the table records no draw or clear in
this build (the 7.1 modules are host-test only)` (`table71::REF_NO_RECORDING`):
the product build of the 7.1 table has no recording path (the 7.1 modules
compile only under the crate's `host-test` feature, `src/v3d/mod.rs`), so
no GPU job was submitted (`jobs 0 submitted`), every clear and draw raised
GL error 0x502, and cases 59 and 67 never reached their own refusals. The
run printed `v3d_gl_test: 0 of 6 cases passed (205 ms)` and ended
`v3d_gl_test RECORD end, build 85afd0cf`; the watchdog brought Linux back
374 s after the reboot. The run replays in `tools/board_selftest.sh` (its
console excerpt is in `tools/testdata/`). The 5-of-6 expectation above
awaits an image whose 7.1 back end is built for the product.

## NanoPC-T6 (Mali-G610)

The suite runs through YakoGL's GL core, its in-process compiler and its Mali
backend on the OSAL `src/osal/zephyr_mali.c`, over our own kernel side
(`board::nanopc_t6_osal`: the GPU powered, the CSF firmware booted, CSG 0 /
CS 0 started, each kick one job on its ring), with the GL ES 1.00 shader set.
The board has no display on Zephyr: the console is uart2. The image is
kexec'd out of Armbian (`driver/tools/kexec-zephyr/nanopc-t6.sh`, a
development result only) and ends with the board glue's PSCI reset back into
Armbian; WDT0 bounds it (`boards/nanopc_t6.conf`: fed until 300 s of uptime,
and only while the run feeds it at least every 30 s).

The CSF firmware is embedded from `driver/src/fw/mali_csffw_blob.h`
(gitignored, never committed): make it with
`driver/tools/mkmalifw.sh <mali_csffw.bin>` first. The board glue
(`driver/zephyr/lib/board/nanopc_t6/nanopc_t6_mali.c`, in every Mali image)
hands it to the OSAL as `osal_zephyr_mali_default_platform()` and prints,
before `main()`, its size and the sha256 that `mkmalifw.sh` took of the file
it made the header from (`t6: CSF firmware N bytes embedded, built from a file
with sha256 X ...`; not computed over the bytes at run time).

```sh
# from the top of the west workspace (driver/docs/nanopc-t6-mali.md, "Z4 board
# procedure", step 1, with this sample in place of t6_submit)
west build -p always -b nanopc_t6 -d build/t6_gl_test yakogl/driver/samples/zephyr/gl_test
```

Memory: about 81 MiB of the board's 256 MiB window (`nanopc_t6.dts`,
`zephyr,sram`), of it the OSAL's memory with its 32 MiB carve-out. That
memory is `.noinit` (`src/osal/zephyr_mali.c`), so `zephyr.bin` does not
hold it: the file is about 4 MiB, and `kexec_zephyr` reads at most 64 MiB.
`tools/check.sh` (`zephyr  nanopc_t6 gl_test image`) checks both limits, and
that the file is smaller than the carve-out.

Kconfig (the NanoPC-T6 menu): `T6_GL_TEST_CASE` (7, the regression set, by
default) and `T6_GL_TEST_CASES` (a list on one context). A full tiler heap
grows the heap; a GPU fault or the 5 s job timeout fails its case and
restarts the OSAL's group (one `osal(mali): restart N ...` line each), at
most 16 times in an image. An error that needs a GPU reset, or one past the
16, stops the OSAL (`osal(mali): stopped by sched refusal N ...`), and every
later case fails. The OSAL's two close lines (`osal(mali): kicks ...` and
`osal(mali): restarts ...`) give the counts. The run ends with
`gl_test: OK|FAILED (rc=N)` and `t6: gl_test done: PSCI SYSTEM_RESET`.

## Verdin iMX95 (Mali-G310, built only)

The same suite, the same `src/main_mali.c` and the same Kconfig menu
(`T6_GL_TEST_CASE`, `T6_GL_TEST_CASES`) as the NanoPC-T6: the GPU code is
the T6's (`docs/notes/mali-g310-vs-g610.md`). What differs is the board's
part, which `main_mali.c` takes from the board glue and the board layer
(`driver/zephyr/lib/board/verdin_imx95`): WDOG3 armed at `PRE_KERNEL_1`, the
GPU powered over SCMI and the CSF firmware booted (`verdin_imx95_mali.c`; the
G310's firmware is the G610's file, so the same
`driver/src/fw/mali_csffw_blob.h`), and the end through `board_finish()`:
`gl_test: OK|FAILED (rc=N)`, then `gl_test: PASS|FAIL`, the `yk-record`
line and `verdin_imx95: board: run over: PSCI SYSTEM_RESET`.

The board files are the board layer's `app.conf` and `app.overlay` (as every
example's), then `verdin_imx95.conf` here
(`../common/verdin_imx95.cmake`). That file turns the X12 LVDS panel off
(`CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=n`): the suite reads its pixels back and
the image never brings the panel up unless asked
(`-DCONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=y`): then, after the suite (same
cases, same lines), `src/main_mali.c` draws three colour bars into a
1280x800 window, `board_show()`s it and holds it for a camera
(`CONFIG_GL_TEST_VI95_PANEL_HOLD_S`, 30 s by default, 0 to 120; the console
prints `gl_test: panel: holding N s`, then `gl_test: panel: OK`). A panel
step that fails prints `gl_test: panel: FAIL (...)` and makes the run's
`gl_test: PASS|FAIL` line FAIL even when the suite passed. The
runner's `panel` argument is not used (it replaces the suite by the AM62P's
demo scene). It sets the
board layer's lifetime to 300 s and its alive window to 30 s, as the T6's
image, and no hold.

```sh
driver/tools/mkmalifw.sh <mali_csffw.bin>      # the header, never committed
west build -p always -b verdin_imx95/mimx9596/a55 -d build/vi95_gl_test \
    yakogl/driver/samples/zephyr/gl_test -- -DYAKOGL_WERROR=ON
```

Memory (2026-11-21): the image's RAM footprint (the AArch64 header's
`image_size`) is 93,069,312 bytes (88.8 MiB) of the 256 MiB at 0xd0000000
(`verdin_imx95_mimx9596_a55.dts`, `zephyr,sram`), of it the carve-out of
`app.conf` (40.6 MiB: the T6's 32 MiB for the GL side plus 8800 KiB of window
memory) and the tiler heap's 10 MiB. `zephyr.bin` is 4,612,216 bytes: the
OSAL's memory is `.noinit`. `tools/check.sh` (`zephyr  verdin_imx95 gl_test
image`) checks the footprint against `zephyr,sram`, the file against the
carve-out, the display off, and the build, WDOG3 and GPU bring-up lines once
each. Not run on a board yet (`docs/notes/pre-study-imx95.md`, B5).
