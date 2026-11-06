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

### Added

- V3D: ES 3.0 texture and QPU work for the Raspberry Pi 4: 3D textures and
  2D-array textures (the layout and slice offsets in `v3d_tex.c`, uploaded
  at their `texture.rs` offsets; the draw refusal stays), `texelFetch` and
  the `*Offset` look-ups in the QPU compiler, swizzle and wrap R taken from
  `GlesTextureBind`, and an integer clear program for a scissored or masked
  clear of an integer colour target. Also the V3D 7.1 (Raspberry Pi 5)
  QPU model, encoder and decoder, host-side only (#210).
- V3D: a `GLES_SURF_RGB10_A2` render surface with its TLB words, and the GL
  core's side of `GL_RGB10_A2` (own surface, bits, CPU mipmap chain). Both
  are gated off: no back end reports the format yet
  (`docs/notes/v3d-rgb10a2.md`).
- GC7000: texture-format words for the remaining ES 3.0 classes, checked
  against Mesa; RGBA32F/UI/I as two G32R32 planes with a companion slot and
  the compiler's 128-bit look-up keys (kept inert,
  `docs/notes/gc-128bit-texture-sampling.md`); the four 64-bit integer
  render targets (RGBA16I/UI, RG32I/UI) drawn through
  a tiled shadow, 17 of the 19 integer surfaces (RGBA32I/UI stay refused).
- PowerVR: the groundwork of the firmware's partial render (#2, #3), all
  off by default: the render kick's zeroed `frag_pr` block
  (`PvrRenderFragPr`, 96 bytes), the SPM scratch size and PBE store words
  (`src/pvr/spm.rs`), the reserved PR end-of-tile PDS data slot, the
  partial-render stall decoders (`pr_diag.rs`) and the debug trace groups.
  `gl_test` case 217 `forcedpr` is the reproducer
  (`docs/notes/pvr-render-pool.md`, `docs/notes/pvr-pr-zs.md`).
- PowerVR: an integer render target variant packs several colour outputs
  (the two outputs of `modf`); more than four output registers are refused
  by name.
- Mali (`gpu-mali`, NanoPC-T6, #32, #33): the GLSL ES compiler reaches many
  more shader forms (integer and bool values, run-time loops and arrays,
  several samplers, 2D-array and 3D textures, mixed fp16/fp32 code,
  control flow kept as branches or flattened into selects, fragment outputs
  past location 0), and the frame builder draws into several colour targets
  and keeps a split 4x frame's samples. Each is checked byte for byte
  against Mesa's drm-shim output.
- NanoPC-T6 bring-up, not on Zephyr's GPU path yet: the MCU firmware's
  section list, CSF interface and boot over a host model (Z3); the
  `board::nanopc_t6_fw` probe with a `#[repr(C)]` record and its C entry
  `yd_t6_fw_up` (declared in `yakogl_drivers.h`); and Z4, the CSG/CS
  start, the scheduler's submit sequence, ring, doorbell and sync-object
  wait, and the tiler heap's creation, growth and chunk return, transcribed
  from panthor and run over host models only. `driver/tools/t6-verdict.sh`
  judges a board `uart.log`.

### Changed

- PowerVR: a depth texture a kick rendered into is sampled by the TPU from
  the ISP's depth buffer (X8U24, Mesa's design), not resolved into R32F
  texels on the CPU first (`docs/notes/pvr-depth-sample.md`, #40). glmark2
  shadow on the AM62P goes from 353 to 10.6 ms a frame and refract from
  376 to 37.0 ms (score 461 to 467); gl_test case 215 `shadowmap` is the
  reproducer (355 to 27.8 ms a frame for its 1600x1200 shadow map).
- PowerVR: a render of a size whose two HWRT datasets are both in flight
  takes a second pair, up to four, instead of waiting for the first kick
  (#42). A mipmap chain that draws its large levels on the GPU and filters
  the tail on the CPU exists as an opt-in (`mip_cpu_tail`, off): on the
  AM62P it was slower than drawing every level on the GPU
  (`docs/notes/am62p-pvr-mipmap-kicks.md`); `gl_test` case 216 `mipchains`
  prints its kicks a frame.
- Build: `capi/assemble.cmake` and the external linkage handle the
  `__rustc::rust_eh_personality` name that recent nightlies reference from
  `compiler_builtins`: the references are read off the archive and renamed
  to `yakogl_rust_eh_personality`, and the build fails if one is left.
  `libyakogl.a` links with `--whole-archive` again on such a nightly.

### Fixed

- V3D: occlusion counters sit at a 1024-byte group each (V3D's TLB counter
  groups), not a 64-byte stride; `gl_test` 196 `occl` read 0 for every
  query on the Raspberry Pi 4.
- GC7000: a user uniform block member is read with a `load` from the
  block's copy in memory, not from the 320-register constant file, when
  its register holds only members; large uniform blocks now link.
- GC7000: `ETC2_RGB` is sampled as Mesa's `EXT_NONE` format, so the T, H
  and planar modes of ETC2 decode (ETC1 blocks decode as before).

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
  panthor kernel driver, not on Zephyr yet (#26). Its newest `gl_test`
  sweep reads 98 of 256 cases PASS and 0 FAIL (2026-10-24).
- Slow glmark2 scenes: terrain on the AM62P (91 ms a frame; shadow and
  refract are 10.6 and 37.1 ms since #40), terrain, shadow and refract on
  the i.MX8MP (4, 2 and 2 FPS), terrain on the Pi 4 (162 ms).
- One thread issues every GL and EGL call; the library does not lock
  (`include/gles_osal.h`, "threads").
- Some display and GPU bring-up files were written after GPL-only Linux
  drivers and are still in that form; `docs/source-form.md` lists them.

[Unreleased]: https://github.com/signal-slot/yakogl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/signal-slot/yakogl/releases/tag/v0.1.0
