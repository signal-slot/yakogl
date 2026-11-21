<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# deqp (Zephyr)

dEQP-GLES2, the Khronos conformance suite, as a Zephyr image: EGL and GLES2
are YakoGL linked in directly, shaders come from the in-process compiler
(`CONFIG_YAKOGL_SHADERC_ONLINE`), the test data is embedded, and the test log
never exists as a file. Current results per board are in
`tools/cts/STATUS.md`.

```sh
# stage 1, once per CTS revision: dEQP's libraries with the Zephyr SDK compiler
ZEPHYR_BASE=... ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh <workspace>)" \
    tools/cts/cts-build-zephyr.sh               # -> build/deqp-zephyr/{cts,build}
# stage 2: the image
west build -b verdin_am62p/am62p54/a53 driver/samples/zephyr/deqp -- \
    -DZEPHYR_MODULES="$ZEPHYR_MODULES" \
    -DDEQP_SRC=$PWD/build/deqp-zephyr/cts -DDEQP_BUILD=$PWD/build/deqp-zephyr/build \
    -DCONFIG_DEQP_CASE='"dEQP-GLES2.functional.prerequisite.*"'
```

What is here, and why:

- `driver/samples/zephyr/deqp/src/main.cpp`: the `tcu::App` loop of dEQP's own `tcuMain.cpp`, with the
  arguments built in (`CONFIG_DEQP_CASE`, a `CONFIG_DEQP_SURFACE_SIZE` pbuffer,
  images and shader sources not logged).
- `driver/samples/zephyr/deqp/src/deqp_archive.cpp`: a `tcu::Archive` over the test data, which
  `driver/samples/zephyr/deqp/tools/embed_data.py` turns into C arrays at build time (77 files, 1.1 MB for
  GLES2). There is no file system.
- `driver/samples/zephyr/deqp/src/deqp_log.c`: `--wrap=fopen` hands dEQP a picolibc stream (`fdevopen`)
  for its QPA log. The stream is parsed line by line and never stored. Each case
  becomes one console line, `deqp: <StatusCode> <case>`, and the summary counts
  the cases that began but never reached a result: absence is its own outcome.
  How the console leaves the board (UART, or the panel for a camera) is the
  board's business, not the sample's.
- `driver/samples/zephyr/deqp/src/deqp_stubs.c`: what the framework references and Zephyr lacks (child
  processes for the subprocess modes, `getcwd`/`lstat`/`readlink`). Each one
  fails the way the real call fails when the thing does not exist.
- The package entry (`modules/gles2/tes2TestPackageEntry.cpp`) is compiled into
  the image, because dEQP builds it into the executable and not into any
  library. Without it the image carries the tests but no package to run.

Configuration that matters:

- `CONFIG_ENFORCE_ZEPHYR_STDINT=n`. Zephyr's convention makes `int64_t`
  `long long`, while the SDK's `libstdc++.a` was built with `long`, so every
  libstdc++ call with a 64-bit type mislinks (`istream::seekg`,
  `this_thread::sleep_for` came up unresolved). Stage 1 leaves the same
  `-imacros zephyr_stdint.h` out.
- `CONFIG_POSIX_API`, `POSIX_THREADS`, and full libstdc++ with exceptions and
  RTTI: delibs runs over Zephyr's POSIX layer, and stage 1 compiles against
  those exact headers (`cts-build-zephyr.sh` takes the flags from this sample).
- A 1 MiB main stack (dEQP itself peaks near 10 KB, the compiler near 74 KB)
  and a 64 MiB C heap.
- A 600 s bounded watchdog feed on the AM62P. The online compiler takes about
  0.2 s per shader on the A53, so size a run's case list to that budget.

## Four draw targets

`CONFIG_V3D_DEQP_MRT4=y` (Pi 4, V3D) and `CONFIG_PVR_DEQP_MRT4=y` (AM62P,
PowerVR) make `src/main.cpp` call `v3d_debug_set_draw_targets(4)` /
`pvr_debug_set_draw_targets(4)` before the first context, so
`GL_MAX_DRAW_BUFFERS` and `GL_MAX_COLOR_ATTACHMENTS` answer 4 instead of 3;
the start-up line `deqp: <setter>(4) -> <previous count>` names the knob and
its return value. Both default to n and need `DEQP_TEST_KNOBS`, which the
image always selects; the setters are the library's test knobs, defined with
`CONFIG_YAKOGL_TEST_KNOBS`, so the image links against a library built with
that config (the deqp sample always selects it). A product never sets it.

The probe image is a dEQP-GLES3 stage 2 build (`DEQP_MODULE=gles3`,
`tools/cts/README.md`) with the symbol added to the `west build` command:

```sh
# Pi 4 (V3D): board rpi_4b; AM62P (PowerVR): verdin_am62p/am62p54/a53
west build -p always -b rpi_4b -d build/pi4-deqp-mrt4 driver/samples/zephyr/deqp -- \
    -DZEPHYR_MODULES="$ZEPHYR_MODULES" -DDEQP_MODULE=gles3 \
    -DDEQP_SRC=$PWD/build/deqp-zephyr/cts -DDEQP_BUILD=$PWD/build/deqp-zephyr/build \
    -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n \
    -DCONFIG_V3D_DEQP_MRT4=y \
    -DCONFIG_DEQP_CASE='"dEQP-GLES3.functional.implementation_limits.*"'
```

(the other arguments as in the stage 2 example at the top; the AM62P arm is
`am62p-deqp-images.sh`'s, with `-DCONFIG_PVR_DEQP_MRT4=y`; tree images build
through `pi4-deqp-images.sh` / `am62p-deqp-images.sh` and their tables,
where a board session can add the symbol the same way).

The probe is meant for the three case groups of
`docs/notes/mrt4-deqp-cases.md`, whose verdict lines are in that note's
"Check lines for a board session's tally":
`functional.implementation_limits.*` (the two count cases; Pi 4 image A1 of
`tools/cts/pi4-gles3-tree.txt`), `functional.draw_buffers_indexed.*` (image
D1; the count alone changes none of the 76 rows), and
`functional.fragment_out.*` (image FR2; the `random.*` cases that today fail
with `GL_INVALID_ENUM` at `es3fFragmentOutputTests.cpp:286`). Re-take the
same groups on the baseline image (the symbol off) first: the note's
expected-flip list is measured against the same build.

## STM32MP257F-DK (A35)

```sh
# stage 1 with the DK's flags (-mcpu=cortex-a35), in a directory of its own
BOARD=stm32mp257f_dk/stm32mp257fxx/a35 WORK=$DEPS/deqp-stage1-stm32mp2 \
CFLAGS=-O2 CXXFLAGS=-O2 ZEPHYR_BASE=... ZEPHYR_MODULES=... tools/cts/cts-build-zephyr.sh
# stage 2
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 driver/samples/zephyr/deqp -- \
    -DZEPHYR_MODULES="$ZEPHYR_MODULES" -DYAKOGL_WERROR=ON \
    -DDEQP_SRC=$DEPS/deqp-stage1-stm32mp2/cts -DDEQP_BUILD=$DEPS/deqp-stage1-stm32mp2/build \
    -DCONFIG_DEQP_CASE='"dEQP-GLES2.functional.prerequisite.*"'
```

The DK runs on the board layer (`driver/zephyr/lib/board/stm32mp2`, its
`app.conf` and `app.overlay`), as gc_fence, gc_clear and gl_test: the layer
powers the GPU, arms OP-TEE's watchdog and feeds it for
`CONFIG_YAKOGL_STM32MP2_LIFETIME_S` (600 s here, one group), and
`board_finish()` ends the run with `deqp: PASS`/`FAIL`, the yk-record line
and PSCI SYSTEM_RESET into ST's Linux. The case lines go to USART2. The
image links at 0xa0000000 in the 256 MiB `zephyr,sram` with the i.MX8MP's
160 MiB heap; the GPU carve-out is the board's 64 MiB at 0xfa800000.
At 5b3c0a47 the gles2 image is a 10.2 MiB `zephyr.bin` (the test data
embedded; bootfs has about 34 MiB free) with a 221.8 MiB footprint
(text 7.5, rodata 1.8, bss 50.7, noinit 161.0 with the heap), inside the
runner's 0xa0000000..0xb0000000.
`tools/board-stm32mp2.sh` stages `zephyr.bin` on bootfs (`/boot/yakogl`) and
waits `RUN_BOUND` (120 s by default) for the end line: give a dEQP run
`RUN_BOUND=600` or more.

## NanoPC-T6 (Mali-G610)

The image runs through YakoGL's GL core, its in-process compiler and its Mali
backend on the OSAL `src/osal/zephyr_mali.c`, over our own kernel side. It is
kexec'd out of Armbian (`driver/tools/kexec-zephyr/nanopc-t6.sh`, one image
per jump, a development result only); the record is uart2.
`boards/nanopc_t6.conf` sets the Mali and WDT0's bounds (fed until 600 s, and
only while a case begins or ends at least every 60 s); `src/mali_board.c`
prints the build first and ends the run with the OSAL's close line,
`deqp: OK|FAILED (rc=N)` and the PSCI reset back into Armbian. The CSF
firmware is the board glue's (`driver/zephyr/lib/board/nanopc_t6/
nanopc_t6_mali.c`): make `driver/src/fw/mali_csffw_blob.h` with
`driver/tools/mkmalifw.sh <mali_csffw.bin>` first.

```sh
# stage 1 for this board (its own compile flags: Cortex-A55)
BOARD=nanopc_t6 WORK=$PWD/build/deqp-zephyr-t6 ZEPHYR_BASE=... \
    ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh <workspace>)" tools/cts/cts-build-zephyr.sh
# stage 2: one image per case filter
west build -p always -b nanopc_t6 -d build/t6-deqp driver/samples/zephyr/deqp -- \
    -DZEPHYR_MODULES="$ZEPHYR_MODULES" -DYAKOGL_WERROR=ON \
    -DDEQP_SRC=$PWD/build/deqp-zephyr-t6/cts -DDEQP_BUILD=$PWD/build/deqp-zephyr-t6/build \
    -DCONFIG_DEQP_CASE='"dEQP-GLES2.functional.prerequisite.*"'
```

The whole tree, one image per row of `tools/cts/mali-gles2-tree.txt`, is
built by `tools/cts/t6-deqp-images.sh` and run by `tools/board.sh nanopc-t6
tree` (`tools/cts/README.md`, section 2d).

Memory: about 183 MiB of the board's 256 MiB window (`nanopc_t6.dts`): the
gl_test image's GL core and OSAL memory, with a 64 MiB carve-out
(`boards/nanopc_t6.conf`: a 2048x2048 texture and its mip chain take
33.5 MiB), dEQP's code and the 64 MiB C heap. `zephyr.bin` is about 12 MiB
of the 64 MiB `kexec_zephyr` reads: the OSAL's memory is `.noinit`
(`src/osal/zephyr_mali.c`), so the file does not hold it. Every case
filter gives the same sizes (the whole package is linked; only the filter
string differs). `driver/tools/kexec-zephyr/nanopc-t6.sh` refuses an image
over either limit before it copies anything to the board.

## Verdin iMX95 (Mali-G310, built only)

The NanoPC-T6's image on the Verdin iMX95: the same `src/mali_board.c`
(the build line `vi95_deqp: build ...`, the OSAL's close line,
`deqp: OK|FAILED (rc=N)`; the end is `mali_deqp_finish()` here, called by
`src/main.cpp`, since the board layer's `sample_glue.c` has its own
`pvr_sample_finish`) over the board glue and the board layer
(`driver/zephyr/lib/board/verdin_imx95`: WDOG3, the GPU's SCMI power-on, the
CSF firmware, which is the G610's file), ending in `board_finish()` (the
`yk-record` line and PSCI SYSTEM_RESET). The board files are the board
layer's `app.conf` and `app.overlay`, then `verdin_imx95.conf` here
(`../common/verdin_imx95.cmake` merges it after `app.conf`; a
`boards/<board>.conf` would be merged before it and lose): the carve-out at
64 MiB for the GL side plus `app.conf`'s 8800 KiB of window memory
(76,120,064 bytes), the board layer's lifetime 600 s and alive window 60 s,
no hold, and the X4 panel off (`CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=n`;
`=y` on the command line asks for it). Stage 1 is the board's own
(`tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`), the tree's
images `tools/cts/imx95-deqp-images.sh` (`tools/cts/README.md`, section 5).

Memory (2026-11-21, rows T6S1R08, T6S4R01 and T6S1R06 of
`mali-gles2-tree.txt`, built with `-DYAKOGL_WERROR=ON`): the RAM footprint
(the AArch64 header's `image_size`) is 200,294,400 bytes (191.0 MiB) of the
256 MiB at 0xd0000000 (`verdin_imx95_mimx9596_a55.dts`, `zephyr,sram`), so
the image ends at 0xdbf04000 and leaves 65 MiB: the T6's image (about
183 MiB) plus the 8800 KiB of window memory. `zephyr.bin` is 12,645,672
bytes, the same for every row (the whole package is linked; only the filter
string differs). The layout as configured holds it; nothing was moved.
