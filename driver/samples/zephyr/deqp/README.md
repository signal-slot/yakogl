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

- `src/main.cpp`: the `tcu::App` loop of dEQP's own `tcuMain.cpp`, with the
  arguments built in (`CONFIG_DEQP_CASE`, a `CONFIG_DEQP_SURFACE_SIZE` pbuffer,
  images and shader sources not logged).
- `src/deqp_archive.cpp`: a `tcu::Archive` over the test data, which
  `tools/embed_data.py` turns into C arrays at build time (77 files, 1.1 MB for
  GLES2). There is no file system.
- `src/deqp_log.c`: `--wrap=fopen` hands dEQP a picolibc stream (`fdevopen`)
  for its QPA log. The stream is parsed line by line and never stored. Each case
  becomes one console line, `deqp: <StatusCode> <case>`, and the summary counts
  the cases that began but never reached a result: absence is its own outcome.
  How the console leaves the board (UART, or the panel for a camera) is the
  board's business, not the sample's.
- `src/deqp_stubs.c`: what the framework references and Zephyr lacks (child
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
