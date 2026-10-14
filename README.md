<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL

YakoGL is an OpenGL ES 2.0 + EGL library for embedded GPUs that runs without an
operating-system GPU driver. It drives the GPU directly, from Zephyr or bare metal.
Applications are ordinary GL ES 2.0 clients built against the Khronos headers in
`include/`.

The library is a `no_std` Rust crate (`yakogl`) with a C ABI (`capi/`). The GPU drivers
it needs are in `driver/`, in the same repository.

The name comes from 夜光貝 (yakogai), the turban shell whose nacre is cut for 螺鈿
(raden) inlay. It also reads as "yet another kernel-optional GL".

## Boards

One GPU per build, chosen by exactly one Cargo feature (none or two is a compile error):

| Board | SoC | GPU | Cargo feature |
| --- | --- | --- | --- |
| Toradex Verdin AM62P | TI AM62P | Imagination PowerVR BXS-4-64 | `gpu-powervr` |
| Toradex Verdin iMX8M Plus | NXP i.MX 8M Plus | Vivante GC7000UL | `gpu-vivante` |
| Raspberry Pi 4 | Broadcom BCM2711 | Broadcom V3D 4.2 | `gpu-v3d` |

Tests boot a Zephyr image once from the bootloader: U-Boot on the two Verdin boards
(from the eMMC, where Linux stages it, or over TFTP), the firmware's tryboot on the
Raspberry Pi 4. Each board also boots standalone,
from its own eMMC or SD card with no host and no network, as `docs/standalone-boot.md`
describes. Software rendering is out of scope: every result here comes from the real GPU.

## Status

OpenGL ES 2.0 is close to complete on all three boards. Known defects are tracked as
GitHub Issues.

dEQP-GLES2 (the Khronos conformance tests, the 17,165-case mustpass list):

| Board | Failures | Notes |
| --- | --- | --- |
<<<<<<< HEAD
| AM62P | 54 | 17,070 pass, 34 not supported, 7 warnings (full tree on `32778b57`, 2026-10-10). The 54: 9 `shaders.invariance` cases, fixed in `e17fb6a7` and passing on the board, and 45 `texture.vertex.2d` cases, fixed on the host, board confirmation pending. The 4x multisample image's 2 failures of that tree (alpha to coverage) pass since `452d6c31`. The last tree before it (2026-10-02, `d60b9d76`) had 0 |
| i.MX8MP | 46 | 17,272 of 17,371 cases pass, 46 fail (all `GL_BGRA_EXT` renderbuffers in `fbo`, the report-es2 allowlist of `8bbf9ba2`), 34 not supported, 19 quality warnings, the same cases warn on NXP's own driver (full tree on `835cf9cc`, 2026-10-14; `32778b57` had 0 fail) |
| Raspberry Pi 4 | 0 | 17,126 pass, 34 not supported, 5 warnings: the full tree on `a27d69c0` (2026-10-10) with its 9 cube-map failures rerun and passing on `1cf946a9`, and one row rerun for 34 cases the serial capture lost (2026-10-11). Not yet one tree from one commit |
=======
| AM62P | 0 | Final tree on `63efed8e` (2026-10-12; `docs/board-sessions/am62p-es2-final-r2.md`): 17,141 pass, 18 not supported, 6 warnings (5 quality, 1 compatibility). The 17 images at 1x give 17,125 pass and 34 not supported; the 4x multisample image passes its 16 cases. `GL_BGRA_EXT` renderbuffers (see the Pi 4) are not covered: the commit predates the change that broke them |
| i.MX8MP | 0 | Final tree on `c345ab74` (2026-10-12; `docs/board-sessions/imx8mp-es2-final-r2.md`): 17,318 of 17,371 cases pass, 34 not supported, 19 quality warnings, the same cases warn on NXP's own driver; identical to the trees of 2026-10-01 and 2026-10-10. Same BGRA caveat as the AM62P |
| Raspberry Pi 4 | 46 | Rows 0-14 of the tree on `20a48791` (2026-10-14; `docs/board-sessions/pi4-es2-final.md`): 11,877 pass, 46 fail, 25 not supported; every failure is a `GL_BGRA_EXT` renderbuffer case, a regression of `8bbf9ba2`; rows 15-28 have not run at that commit. The last whole tree (`d56e79e5`, 2026-10-12) had 17,127 pass, 0 fail, 34 not supported, 4 warnings |
>>>>>>> 146a27e72a61bc8802d8c54ddf1d9c356b414a61

glmark2-es2 at 800x600 runs all 33 scenes on every board:

| Board | Score |
| --- | --- |
| AM62P | 465 (`f2a13877`, 2026-10-11) |
| Raspberry Pi 4 | 183 (2026-10-04) |
| i.MX8MP | 43 (`cd738b1a`, 2026-10-14) |

Two scenes drew wrong: terrain sampled a black texture on every board, and refract
missed its depth pass on the AM62P and Raspberry Pi 4. Both are fixed on the host;
on the AM62P the depth pass now resolves, but no picture of either scene has been
checked yet. On the Raspberry Pi 4 refract's draw was refused once a frame; the
host harness no longer refuses it with the image's 24 MiB tiled-copy budget, but
the board has not run it yet (see the Issues). The i.MX8MP's score is probably capped by a wait for the panel's
VSYNC on every frame (a prediction from the code,
`tools/cts/log/investigations/imx8mp-frame-floor.md`). Performance work has not
started yet.

Slint applications run on all three boards from one sample with the same picture
(`driver/samples/zephyr/slint/`).

`tools/cts/STATUS.md` has the detailed conformance state (in Japanese) and
`tools/cts/RESULTS.md` the measurement log. Commit hashes in the logs from before
2026-10-04 refer to the development history before this repository was published.

## Shader compilers

Where a shader's machine code comes from, chosen at build time:

- **online** (the default everywhere but the bare-metal make build): our own
  `no_std` GLSL ES compiler running on the board (`src/shaderc/online/`,
  feature `shaderc-online`; Kconfig `YAKOGL_SHADERC_ONLINE`, CMake
  `YAKOGL_SHADERC=online`).
- **precompiled shaders**: the source list (`tools/shaders/precompiled.list`)
  compiled by the same compiler while the image is built (`yakogl-precompile`),
  looked up by the compiler's own record key (feature `precompiled-shaders`;
  Kconfig `YAKOGL_PRECOMPILED_SHADERS` and `YAKOGL_PRECOMPILED_APPS`, CMake
  `YAKOGL_PRECOMPILED_SHADERS` and `YAKOGL_PRECOMPILED_APPS`).
- **offline**: a table of shaders compiled on the host, looked up by the SHA-256 of
  the GLSL text (`src/shaderc/offline.rs`). Its only table today is the Mesa
  reference (`tests/reference/`, what Mesa's pco_shc / etna_shc made of
  `tools/shaders/precompiled.list`), host-test material that a product build
  links only when it asks for it (feature `mesa-reference-table`, Kconfig
  `YAKOGL_SHADERC_OFFLINE`): a measurement-only choice, to be replaced by the
  table our compiler precompiles (`tools/precompile`).

Every image also binds the back ends' own programs (the PowerVR's frame-load
and clear quads' shaders, the GC7000's clear pair), which our compiler made
once on the host from `tools/shaders/internal/es/` (`yakogl-precompile
--internal`, committed as `src/pvr/internal.rs` and `src/vivante/internal.rs`),
not Mesa's `pco_shc` or `etna_shc`. Two sets of the PowerVR driver's programs are
still Mesa's pco output, captured into C headers (`driver/src/pvr_usc_*_blob.h`,
`tools/pco_shc/`): the end-of-tile programs every render ends with
(`pvr_render.c`) and the boot self-test render's vertex shader (`pvr_selftest.c`;
its clear program is our encoder's words).

A product image is one of three: the compiler only (the default); the precompiled shaders in
front of the compiler (a hit runs no compile, a miss compiles); and the
precompiled shaders only (Kconfig `YAKOGL_SHADERC_PRECOMPILED_ONLY`, CMake
`YAKOGL_SHADERC=precompiled`: no compiler, no scratch region; a shader, a
variant or a program the table lacks is refused by name, `-ENOENT`, with the
line in the info log). For gl_test's ES 1.00 set on the AM62P they are
2,431,248, 5,970,192 and 5,101,512 bytes (zephyr.bin).

## The PowerVR firmware

The PowerVR GPU needs Imagination's firmware, which is not in this repository and is
never distributed with it. Take `rgx.fw` from TI's Processor SDK under its licence and
turn it into a header with `driver/tools/mkfwblob.sh`.

## Layout

- `include/` Khronos `GLES2/`, `EGL/`, `KHR/` headers plus the small YakoGL extras.
- `Cargo.toml` the Cargo workspace; `src/lib.rs` the Rust crate `yakogl` (an rlib
  with no panic handler; features `gpu-powervr` | `gpu-vivante` | `gpu-v3d` (exactly
  one), `osal-linux` | `osal-zephyr` | `osal-baremetal`, `host-test` for host tests only). `capi/` the crate `yakogl-capi` that turns it into
  `libyakogl.a` (panic handler, `assemble.cmake`, the hosted EH personality
  `eh_personality.c`, the CMake glue `yakogl_capi.cmake`). `src/util.rs` holds the shared helpers (global-state cell, C strings,
  the C heap). `src/abi_layout.rs` + `src/abi_layout_check.c`, both generated by
  `yakogl-tools abi-layout`, pin the size and every field offset of the `#[repr(C)]`
  structs shared between the C headers and the crate (35 structs, 563 fields) on both
  sides, so header/Rust drift fails the build.
- `src/gl/*.rs` the GL and EGL entry points, the context and object model, and the
  draw resolver (`draw.rs`) that hands one `struct gles_draw` (`GlesDraw`) to the
  backend. `consts.rs` and `egl_consts.rs` are generated from the Khronos headers by
  `yakogl-tools gl-consts`.
- `src/backend.rs` the backend contract, a function table.
- `src/pvr/` the PowerVR backend (`backend.rs`): per-frame arenas, VDM/PPP/PDS
  streams, one GEOM+FRAG kick per frame. It calls the driver's OS-free command
  generators (C, in `driver/src/`) through the FFI declared in `ffi.rs`;
  `ffi_layout_check.c` holds only `_Static_assert`s that pin the driver structs,
  constants and prototypes `ffi.rs` mirrors, so driver header drift fails the build.
- `src/shaderc/` the shader backend: `online/` the compiler; `offline.rs` looks
  shaders up by the hash of their GLSL in a table compiled on the host, the Mesa
  reference (`shaderc::mesa_reference`, from `tests/reference/blobs.rs` and
  `vivante_blobs.rs`, generated; host tests and `mesa-reference-table` only).
- `src/osal/` the OS abstraction: `mod.rs` (`g_osal`, `osal_set_ops`) and the three
  OSALs, which are C for now (thin glue over C APIs; their move to Rust is open
  work in `docs/HANDOFF.md`): Zephyr, one translation unit per GPU (`zephyr.c`
  over the PowerVR render service, `zephyr_gc.c` for the GC7000, `zephyr_v3d.c`
  for the V3D) with `zephyr_shaderc_scratch.c` (the compiler's regions) and
  `zephyr_timing.c`; `baremetal.c`; `linux.c` (a host build with no GPU, for
  AddressSanitizer runs and the host harness).
- `src/gl_internal.h`, `src/gl_context.h`, `src/backend.h`, `src/pvr/pvr_backend.h`,
  `src/osal/osal_internal.h` the C view of the crate's `#[repr(C)]` structs and
  `#[no_mangle]` helpers, for the on-silicon test suite and the samples; keep them in
  step with the Rust structs (the layout pins above enforce it; re-run
  `yakogl-tools abi-layout driver/src` when a shared struct changes).
- `tools/` the host tools: `yakogl-tools/` (Rust, a workspace member; `cargo run -p
  yakogl-tools -- <command>`) with the offline shader table generator `glsl2usc` for
  the PowerVR (pco_shc in the Mesa Imagination tree; `glsl2usc.sh` runs it over the
  starter set) and the generators `usc-blob`, `femtovg-shaders`, `gl-consts`
  (`src/gl/consts.rs`, `egl_consts.rs`) and `abi-layout`; `vivante/` for the GC7000
  (Mesa's etnaviv compiler); `hostharness/` (`run.sh`, `compare.sh`, `report.py`, the
  recording OSAL `rec.c` and three harness programs), the host differential harness.

`DESIGN.md` explains the layering; the four `*_NOTES.md` files under `src/`
(`GL_CORE_NOTES.md`, `osal/OSAL_NOTES.md`, `shaderc/SHADERC_NOTES.md`,
`vivante/VIVANTE_NOTES.md`) record what is implemented and what is stubbed.

## Building

The Rust code is a Cargo workspace: the library crate `yakogl` (repository root) and
`yakogl-capi` (`capi/`), the static library for C. Host tools join as further
`members` in `Cargo.toml`; `default-members` is the library alone, and the C builds
always build `-p yakogl-capi`, so a host tool is never built into (or its features
unified with) the `no_std` archive. Rust 1.88 or newer (`rust-version`).

Every C build runs cargo for `yakogl-capi`, post-processes its static library with
`capi/assemble.cmake` (see "Linking") and compiles the C parts: the selected OSAL,
`abi_layout_check.c`, `ffi_layout_check.c`, the driver's command generators, and on
hosted targets `capi/eh_personality.c`. CMake and `baremetal.mk` append the C objects,
so `libyakogl.a` is the single archive; the Zephyr module links the Rust archive next to
its `zephyr_library` of the C OSAL (the generators come from the driver module).
Cargo runs through its own dep-info, so a build with nothing changed does nothing and
the archive (and the application) is only re-linked when an input changed. Changes
to `RUSTFLAGS`, cargo configuration or the toolchain are not tracked: touch
`Cargo.toml` after them.

```sh
# host library (no GPU)
cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux   # the driver sources: driver/src
cmake --build build
# cross build: a CMake toolchain file; the Rust target is derived from
# CMAKE_C_COMPILER_TARGET / CMAKE_SYSTEM_PROCESSOR + CMAKE_SYSTEM_NAME
# (aarch64 + Generic -> aarch64-unknown-none) or given with -DYAKOGL_RUST_TARGET;
# a triple whose architecture differs from the C compiler's is an error
cmake -S . -B build-a64 --toolchain aarch64.cmake -DYAKOGL_OSAL=baremetal ...
# bare metal (RUST_TARGET defaults to aarch64-unknown-none)
make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf- DRVSRC=driver/src
make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm \
     OBJCOPY=llvm-objcopy DRVSRC=...
# Zephyr: a workspace from west.yml (driver/samples/zephyr/README.md), or add this
# directory and driver/ to ZEPHYR_EXTRA_MODULES,
# CONFIG_YAKOGL=y (zephyr/Kconfig lists the FPU and libc requirements)
```

Options (CMake cache variables for `CMakeLists.txt` and `zephyr/`; `baremetal.mk`
has the make variables in parentheses):

| Option | Default | |
|---|---|---|
| `YAKOGL_OSAL` | `linux` (Zephyr and make: fixed, `zephyr` / `baremetal`) | the OSAL |
| `YAKOGL_SHADERC` (Zephyr `CONFIG_YAKOGL_SHADERC_ONLINE` / `_OFFLINE` / `_PRECOMPILED_ONLY`) | `online` (make: the Mesa reference, see `baremetal.mk`) | the shader back end: our compiler, or `offline`, the Mesa reference as the table (measurement images only), or `precompiled`, the table alone with no compiler |
| `YAKOGL_PRECOMPILED_SHADERS` (Zephyr `CONFIG_YAKOGL_PRECOMPILED_SHADERS`) | `OFF` (`n`) | the precompiled tier in front of the compiler: the build runs `yakogl-precompile` on the named apps first and links its table (`zephyr/Kconfig` records what the table costs) |
| `YAKOGL_PRECOMPILED_APPS` (Zephyr `CONFIG_YAKOGL_PRECOMPILED_APPS`) | empty, and empty with the tier on is a build error | the list's apps to precompile, e.g. `gl_test_es100`, `femtovg` (Zephyr also `CONFIG_YAKOGL_PRECOMPILED_LIST`, another list) |
| `YAKOGL_PVR_DRIVER_DIR` (`DRVSRC`) | `driver/src` (make: `../src`, the old two-repository layout, so pass `DRVSRC=driver/src`) | the PowerVR driver sources |
| `YAKOGL_RUST_LINKAGE` (`RUST_LINKAGE`) | `staticlib` | `external`: C parts only (`libyakogl_c.a`), see "Linking" |
| `YAKOGL_RUST_TARGET` (`RUST_TARGET`) | host / derived; Zephyr and make: `aarch64-unknown-none` | Rust target triple (64-bit only) |
| `YAKOGL_RUST_TARGET_CPU` (`RUST_TARGET_CPU`) | none; Zephyr: `cortex-a53` | `-C target-cpu` |
| `YAKOGL_CARGO_PROFILE` (`CARGO_PROFILE`) | `release` | `dev`: overflow checks and debug assertions (cargo profile `dev-lto`) |
| `YAKOGL_CARGO` (`CARGO`) | `cargo` on `PATH` | the cargo executable |
| `YAKOGL_RUSTUP_TOOLCHAIN` (`RUSTUP_TOOLCHAIN`) | inherited | e.g. `nightly`, `1.88` |
| `YAKOGL_CARGO_BUILD_STD` (`BUILD_STD=1`) | `AUTO` (make: off) | `-Z build-std=core`: AUTO uses it when the toolchain has no precompiled `core` for the target (a nightly with `rust-src`, as the samples in `driver/samples/` use); otherwise configuration stops with instructions |
| `YAKOGL_REPORT_ES2` (`REPORT_ES2`; Zephyr `CONFIG_YAKOGL_REPORT_ES2`) | `ON` (make: `1`; Kconfig: `y`) | the GL version the library reports: ES 2.0 by default. `OFF` (`REPORT_ES2=0`, `CONFIG_YAKOGL_REPORT_ES2=n`) reports ES 3.0, which Qt Quick 3D, a dEQP-GLES3 image and `#version 300 es` clients need. The cargo feature `report-es2` (for a build that links the crate directly) is unchanged: off unless asked for |
| `YAKOGL_EH_PERSONALITY` (CMake only) | `ON` | external linkage on hosted targets: `libyakogl_c.a` defines `rust_eh_personality` (turn off when the Rust application links `std`) |

The archive post-processing needs an `nm` and an `objcopy` that understand the target's
objects (`CMAKE_NM` / `CMAKE_OBJCOPY`, set by every CMake and Zephyr toolchain; for
clang cross builds `llvm-nm` / `llvm-objcopy`). A host GNU objcopy without the
target's back end leaves the objects unchanged; `assemble.cmake` checks the result and
stops.

## Linking

**C and C++ applications** link `libyakogl.a` (CMake: target `GLES2`) and compile
against `include/`. The archive is built so that it can sit next to other Rust static
libraries (the Slint and FemtoVG samples in `driver/samples/`, a Rust `std` library)
from the same or another toolchain, in any order:

- the panic handler lives in `yakogl-capi` and is internal after LTO; any other Rust
  runtime symbol (`_R...`, `_ZN...`, `rust_*`) left global by the compiler is made
  local by `capi/assemble.cmake`, which therefore requires the single LTO object of the
  `release` / `dev-lto` profiles and rejects anything else;
- compiler_builtins' `memcpy`, `memset`, `memmove`, `memcmp`, `bcmp`, `strlen` and its
  libm functions (about 70 on x86-64, 125 on aarch64: `sin`, `pow`, ...) are made local, so the platform's libc/libm
  stay in charge of the whole image (they would otherwise replace them as weak
  definitions pulled from an archive listed before libc); the compiler runtime helpers
  (`__udivti3`, ...) stay global;
- on hosted targets the unwind tables' `rust_eh_personality` is renamed to
  `yakogl_rust_eh_personality`, which `capi/eh_personality.c` defines as a forwarder to
  the GCC C personality `__gcc_personality_v0` (libgcc / compiler-rt): a foreign unwind
  through a GL frame runs Rust's abort landing pads instead of an empty function, and
  it cannot collide with the personality of a Rust `std` library.

The application provides (libc): `abort`, `malloc`, `free`, `memcpy`, `memmove`,
`memset`, `memcmp` (`bcmp` on hosted builds), `strlen`, `snprintf`; the bare-metal
OSAL also `vprintf`, the Linux OSAL `vfprintf`/`fputc`/`aligned_alloc`. Float
`printf` is needed only for the backend's debug attribute dump. No libm. A Rust panic
(bounds or overflow check) logs its message through the OSAL and calls `abort()`.

**Threads.** One thread issues every GL and EGL call of the process. It may create
several contexts and make them current in turn, but the library keeps no per-thread
state and does not lock for other threads: a call from a second thread, or two calls
at once, is not supported. Where the platform can tell threads apart (the OSAL's
`thread_id`; the Linux and Zephyr OSALs have it, bare metal does not), a call from
another thread while a context is current is refused and changes nothing: EGL returns
its failure value and that thread's `eglGetError` gives `EGL_BAD_ACCESS`, GL does
nothing and that thread's `glGetError` gives `GL_INVALID_OPERATION`, and the first
refusal is logged once. `include/gles_osal.h` ("threads") has the full contract.

**Rust applications** (for example a `no_std` staticlib built with
`cargo -Z build-std`, like the samples in `driver/samples/`) depend on the crate and
provide the image's one panic handler themselves:

```toml
[dependencies]
yakogl = { path = "yakogl", default-features = false, features = ["osal-zephyr", "gpu-powervr"] }
```

Exactly one `gpu-*` feature must be enabled. At most one `osal-*` feature may be enabled (several are a compile error; none leaves
`g_osal` null until `osal_set_ops()`); `host-test` is not for products (a compile
error on bare-metal targets and in `yakogl-capi`). The crate has no build script:
the C parts come from the build system with `YAKOGL_RUST_LINKAGE=external`, which
builds no Rust at all. Zephyr: the module's `zephyr_library` then holds the OSAL and
the layout checks and the application links its own Rust archive as before. CMake and
`baremetal.mk`: link `libyakogl_c.a` (CMake: target `GLES2`, which then carries only
the C parts) after the application's Rust archive. On hosted targets `libyakogl_c.a`
also defines `rust_eh_personality` for a `no_std` Rust library
(`-DYAKOGL_EH_PERSONALITY=OFF` if it links `std`). The Rust application's own
archive is not post-processed by YakoGL: whether its compiler_builtins `memcpy`/libm
copies replace the platform's is up to that build.

**API coverage** is checked, not assumed: `tools/api_coverage.py` (wired into `tools/check.sh`)
parses the OpenGL ES 2.0 core entry points declared in `include/GLES2/gl2.h` and the EGL 1.x
core declared in `include/EGL/egl.h` (186 names), `nm`s the built `yakogl-capi` archive for what
the library actually exports (298 names, which also cover the ES 3.0 subset and the EGLImage/KHR
extensions the crate implements), and parses the `PROCS` table in `src/gl/egl.rs` for the names
`eglGetProcAddress` resolves (298 names, identical to the exported set today). It prints one row
per name (declared / exported / procaddress) and the three difference sets, and fails only on an
inconsistency of ours -- a name exported but not resolvable through `eglGetProcAddress`, or the
reverse; a declared-but-not-exported name is reported and its count pinned in `check.sh` (0 for
both the ES 2.0 core and the EGL 1.x core today) so a lost or silently added entry point is
noticed either way.

## Testing on the host

**Unit tests** live next to the code (`#[cfg(test)] mod tests` at the end of each
module) and run on the host with

```sh
tools/check.sh --changed origin/main   # <- before a pull request: the rows your change reaches
tools/check.sh --quick                 #    the PowerVR build only, for a tight edit loop
tools/check.sh                         #    every row: what the CI runs

cargo test --no-default-features --features host-test,gpu-powervr   # the PowerVR build
cargo test --no-default-features --features host-test,gpu-powervr,report-es2   # the ES 2.0 report
cargo test --no-default-features --features host-test,gpu-vivante  # the GC7000 build
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online   # the compiler
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online
cargo test -p yakogl-tools                              # the host tools
```

**A red `cargo fmt --check` or a single warning is a defect, not a matter of
taste.** `tools/check.sh` is the gate: the formatter, clippy with
`--all-targets`, and the suite in every configuration of its CONFIGS list,
exiting non-zero on any of them. The CI runs it
(`.github/workflows/gate.yml`, on every pull request, every push to main and
nightly); locally, `tools/check.sh --changed <base>` runs the rows a change
reaches (`CONTRIBUTING.md`). A build that already prints one warning is a build
where the next real warning is invisible -- which is how three
`unused_unsafe` and a function with no callers survived here -- and `main`
has arrived unformatted twice.

The configurations include feature pairs, because a pair can be broken while
every single feature is green. A feature the gate never combines is a feature
the gate does not cover.

YakoGL reports ES 2.0 by default in the CMake, `baremetal.mk` and Zephyr builds (they
turn the `report-es2` feature on); a build that needs ES 3.0 says so explicitly (see
Options). The cargo feature itself is opt-in.

A few rules differ between the two versions this library can report (see
`src/GL_CORE_NOTES.md`, "Conformance"), so a test that pins one of them reads
`crate::REPORT_ES2` or is `#[cfg]`-ed to the build it belongs to. Run both.

`src/testutil.rs` gives a test a fresh GL context on mocks: `TestGl::new(w, h)`
installs a mock OSAL and a recording mock backend (every `GlesDraw` the core resolves,
with its uniform, attribute and index staging copied out), resets every process-global
of the library, and serialises the tests that use it (the library keeps global state).
The mock OSAL models a non-coherent platform: a CPU view and the device memory of
every allocation, synchronised only by `cache_flush` / `cache_invd` (both recorded as
allocation + offset + length); fresh memory is filled with 0xA5; allocations can come
without a CPU mapping; gpu_alloc / gpu_write / gpu_read failures can be injected. It
reports double frees and frees of foreign descriptors, cache calls outside live
allocations, invalidates that drop unflushed writes or race a running kick, and draws
that read unflushed, never-written, freed or out-of-range memory or whose inputs change
before their kick completes. The mock backend's kicks follow the PowerVR model
(`kick_id()` is the frame being recorded; kicks complete on finish, by test control or
after a configurable lag). Details: the module documentation of `src/testutil.rs`.
With the `host-test` feature `g_backend` starts null, so test binaries do not need the
driver's C generators. The convention for an open defect is a test marked
`#[ignore = "BUG: ..."]` asserting the *correct* behaviour. Five are open today
(`git grep -n 'ignore = "BUG' -- src tests`): four ES 2.0 report pins in
`tests/es2_report_guard.rs` and the PowerVR's split 4x frame in
`src/pvr/backend_tests.rs`; each is a known defect in `docs/HANDOFF.md`. Use the
convention rather than leaving a defect undocumented.

**Command-stream check.** `tools/hostharness/compare.sh <out> <libA> <libB> <driver checkout>`
builds two libraries (e.g. `rust:.` and `rust:/path/to/the/previous/checkout`, or a C tree
as `csrc:<dir>`) with AddressSanitizer, runs `driver/tools/pvrlab/gl_test.c`'s cases, the
harness's API sweeps and hostile sequences through both over a recording OSAL in every
platform mode, dev and release, and exits non-zero when a log differs in a way
`tools/hostharness/allow.txt` does not explain. `run.sh all <out> <lib> <driver>` runs one
library (golden logs). Nothing is rendered. `tools/hostharness/README.md` lists what is
and is not compared. Against a C tree the run is no longer expected to be quiet: it
reports the fix round's divergences from upstream (see "Status"), and each of those has
to be recognised before a real regression can be seen. Against the previous Rust build
it is the regression gate, and every remaining difference has to be explained.

## Licensing

Copyright 2026 Signal Slot, Inc. YakoGL's own code is Apache-2.0 (`LICENSE`),
whatever it uses or links: the Slint and FemtoVG samples included. Every source
file carries an SPDX line.

The tree is not all original. `NOTICE` lists every outside origin with its
licence and copyright holders, and `LICENSES/` holds one text per licence:

- **Mesa 26.2.1 (MIT; Imagination, Broadcom, Rob Clark, Intel, the Etnaviv
  Project, Igalia and others).** The PowerVR PDS/VDM/PPP/ISP generators, the
  USC blobs compiled by Mesa's pco, the V3D control-list encoder and frame/job
  builders, `gc_draw.c`, the ISA tables generated from Mesa's ISA descriptions
  and much of `src/shaderc/online/` are ports of, or derived from, Mesa code.
  Each such file names the upstream file, version and copyright holder in its
  header, with `SPDX-License-Identifier: Apache-2.0 AND MIT`.
- **The TI/Imagination Rogue DDK 25.2 kernel source (Dual MIT/GPLv2, used
  under MIT).** The firmware-interface headers and register definitions,
  vendored unmodified in `third_party/ti-img-rogue-ddk-25.2` and used by
  `pvr_bringup.c`, `pvr_fw_boot.c` and `pvr_render.c` (which began as ports of
  Linux drm/imagination, whose definitions they no longer carry). The DDK
  firmware binary is proprietary and is not in the tree; `driver/tools/mkfwblob.sh`
  turns a firmware the user supplies into a header that is never committed.
- **The etna_viv rnndb register headers (MIT)** in `driver/src/vivante/hw/`,
  **the Khronos API headers** in `include/` (MIT and Apache-2.0), **VK-GL-CTS**
  (Apache-2.0, the ETC1 decoder in `src/gl/etc1.rs`), and the public-domain X11
  misc-fixed font.

Some display and GPU bring-up files were written after GPL-only Linux drivers:
the i.MX8MP DSIM and LCDIF code and the V3D register and power code are still
in that form. The others have been rewritten from documentation in
`driver/rust`, and their C deleted: the SN65DSI8x bridge and the i.MX8MP
backlight from their data sheets, the i.MX8MP media power domains and the
AM62P DSS and DSI drivers from the reference manuals, and the Vivante kernel
side from the rnndb headers, NXP's galcore driver (dual MIT /
GPL-2.0-or-later, used under MIT) and the reference manual.
They are not licensed differently by this; `docs/source-form.md` lists each
one and how it is to be replaced by a permissive driver or rewritten from
documentation, and `third_party/README.md` describes how upstream code is to be
kept in its upstream form.
