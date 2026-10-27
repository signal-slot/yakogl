<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Changelog

All notable changes to YakoGL are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The first version is 0.1.0; its section says what the project is and what it has
at that version.

Commit hashes quoted here name commits of the non-public working repository
the project was developed in.

## [Unreleased]

## [0.1.0] - 2026-10-27

YakoGL is an OpenGL ES 2.0 and EGL 1.5 library for embedded GPUs that runs
without an operating-system GPU driver. It drives the GPU directly, from
Zephyr or bare metal. It is a `no_std` Rust crate with a C ABI.

### Results

The newest board results are in `tools/cts/RESULTS.md`; this list is the
state on 2026-10-26.

- dEQP-GLES2, the 17,165-case mustpass list, from one main commit,
  `e9901869a` (2026-10-15/16), reads 0 failures on the three boards:
  AM62P 17,140 pass, 18 not supported, 7 warnings (the 1x images with the
  16 `multisample.*` cases of the 4x image); i.MX8MP 17,318 pass,
  34 not supported, 19 quality warnings; Raspberry Pi 4 17,126 pass,
  34 not supported, 5 warnings. Later runs read the same counts: the AM62P
  1x images at `25cce714`, the i.MX8MP at `d0c5dc48`, the Pi 4 at `46f93767`.
  The accepted not-supported cases are listed in `docs/es2-cts-failures.md`,
  section C.
- glmark2-es2 at 800x600 runs all 33 scenes on every board, with 0 failed
  scenes and 0 GL errors. Scores from main `445c5653` (2026-10-20): AM62P
  461, Raspberry Pi 4 193, i.MX8MP 42. The Pi 4 read 195 at `c4af4917`
  (2026-10-22). A new table from one main commit is issue #1196.
- dEQP-GLES3 (OpenGL ES 3.0) is partial: the AM62P's whole tree read
  36,552 of 42,810 pass (2026-10-14), the Pi 4 ran 9 of its 91 images, the
  i.MX8MP has not run it (`ROADMAP.md`, stage 4).

### Added

#### Library
- OpenGL ES 2.0 and EGL 1.5 entry points, built against the Khronos headers
  in `include/`. Applications are ordinary GL ES 2.0 clients.
- The C, CMake and Zephyr builds report ES 2.0 by default
  (`YAKOGL_REPORT_ES2`); the cargo feature `report-es2` does the same for a
  Rust build.
- One GPU back end per build, chosen by one Cargo feature: `gpu-powervr`
  (Imagination PowerVR BXS-4-64, TI AM62P), `gpu-vivante` (Vivante GC7000UL,
  NXP i.MX 8M Plus), `gpu-v3d` (Broadcom V3D 4.2, Raspberry Pi 4) and
  `gpu-mali` (Arm Mali-G610, RK3588; in progress, see below).
- The OS layer behind one interface (`include/gles_osal.h`): a Zephyr OSAL
  per GPU, a bare-metal OSAL whose board hooks are still to be written
  (#1223), and a Linux OSAL for host tests with no GPU.
- EGL window surfaces are double-buffered by default (`EGL_BACK_BUFFER`) on
  a platform that can show a buffer; `EGL_SINGLE_BUFFER` renders in place.
  Pbuffer surfaces are supported too.
- `libyakogl.a` can sit next to other Rust static libraries in one image:
  the Rust runtime and the compiler_builtins libc and libm symbols are made
  local (`capi/assemble.cmake`).
- The ES 3.0 entry points (`include/GLES3/`, `src/gl/es3.rs`), reported when
  `YAKOGL_REPORT_ES2` is off. Transform feedback, uniform blocks, sized and
  integer formats, occlusion queries, `glBlitFramebuffer`, sRGB, buffer
  mapping and pixel buffers are tested on the host for the three GPUs; the
  board state is above.

#### Shader compilers
- Our own `no_std` GLSL ES compiler runs on the board (`src/shaderc/online/`,
  the default), so an application can compile shaders at run time.
- A precompiled tier: the shaders of known applications compiled by the same
  compiler while the image is built (`yakogl-precompile`), in front of the
  compiler or, in an image without the compiler, alone.
- A table of Mesa-compiled shaders for measurement images only
  (`mesa-reference-table`); no product build links it.

#### Boards, examples and samples
- Zephyr board support for the Toradex Verdin AM62P, the Toradex Verdin
  iMX8M Plus and the Raspberry Pi 4 (`driver/`, `zephyr/`). Each board boots
  from its own eMMC or SD card with no host (`docs/standalone-boot.md`).
- Examples: `examples/triangle`, `examples/triangle-rust` and
  `examples/slint` (a Slint UI drawn through YakoGL).
- The test and benchmark images: `gl_test`, dEQP-GLES2 and dEQP-GLES3, and
  glmark2-es2 (`driver/samples/zephyr/`, `tools/cts/`).
- The PowerVR firmware is not in the repository. `driver/tools/mkfwblob.sh`
  turns the firmware file from TI's SDK into a header that is never
  committed.

#### Tools and tests
- `tools/check.sh`, the gate the CI runs (`.github/workflows/gate.yml`):
  the formatter, clippy and the host tests in every feature configuration,
  and the Zephyr image builds.
- Host unit tests on strict mocks of the OS layer and the GPU
  (`src/testutil.rs`), and a host differential harness over a recording OS
  layer (`tools/hostharness/`).
- `tools/board.sh` runs an image on a board and reads its result.
- A licence and SPDX audit (`tools/license_audit.py`).

#### Documentation
- `README.md`, `DESIGN.md`, `ROADMAP.md`, `CONTRIBUTING.md`, `SECURITY.md`
  and a guide to building the benchmark images (`docs/release/building.md`).

### Known limits

- OpenGL ES 3.0 is not complete on any board (stage 4 of `ROADMAP.md`).
- The `gpu-mali` back end runs only on the NanoPC-T6's own Linux, through the
  panthor kernel driver, not on Zephyr yet (#1281). Its newest `gl_test`
  sweep reads 98 of 256 cases PASS and 0 FAIL (2026-10-24).
- Slow glmark2 scenes: terrain, shadow and refract on the AM62P (94, 353 and
  376 ms a frame) and on the i.MX8MP (4, 2 and 2 FPS), terrain on the Pi 4
  (162 ms).
- One thread issues every GL and EGL call; the library does not lock
  (`include/gles_osal.h`, "threads").
- Some display and GPU bring-up files were written after GPL-only Linux
  drivers and are still in that form; `docs/source-form.md` lists them.

[Unreleased]: https://github.com/signal-slot/yakogl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/signal-slot/yakogl/releases/tag/v0.1.0
