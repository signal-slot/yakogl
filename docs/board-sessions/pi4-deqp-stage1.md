<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP stage 1 (host only): built, row D proven

Status: **stage 1 built and proven.** dEQP-GLES2 libraries for `rpi_4b`
are at
`/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/work`
(`DEQP_WORK`). Row D of `tools/cts/pi4-full-tree.txt` built with
`tools/cts/pi4-deqp-images.sh`: `D rc=0`, conformance verdict passed, the
test image was then deleted. No board was touched; nothing was flashed.

## Inputs, at their pinned revisions

| input | revision | pinned by |
|---|---|---|
| VK-GL-CTS | `93bca01861b0e3ef3c387027a9791e6d065f900c` | docs/release/building.md:277 |
| CTS externals (zlib 1.2.13, libpng v1.6.50, renderdoc_app.h v1.1, SPIRV-Tools `14521db9`, glslang `ebe60e33`, SPIRV-Headers `49654312`, Vulkan-Docs `f84d432d`, amber `fee69c1b`, jsoncpp `89e2973c`, VulkanSC-pcutil `f67d9e8e`, Vulkan-Video-Samples v0.5.0, video_generator v0.1.1) | as listed | CTS `external/fetch_sources.py` (default set, no VVL) |
| Zephyr | `af17c0c8aa2a82a0b714a423915ad3610063ee68` (4.4.99) | west.yml |
| fatfs / hal_nxp / hal_ti / picolibc | `f4ead3bf` / `1e91007b` / `d9d5b1c9` / `01254932` | Zephyr's west.yml (imported by name-allowlist) |
| Zephyr SDK | 1.0.1, modular: `zephyr-sdk-1.0.1_linux-x86_64_minimal` + `toolchain_gnu_linux-x86_64_aarch64-zephyr-elf` | Zephyr `SDK_VERSION` |
| Rust | nightly (`rustup` default), target `aarch64-unknown-none` installed | docs/HANDOFF.md (Pi 4 needs `RUSTUP_TOOLCHAIN=nightly`) |
| cmake / west | 3.31.10 / 1.5.0, in the workspace venv | see Notes (host cmake is 4.3.5) |

Everything lives under
`/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b`
(8.1 GB total): `dl/` archives, `cts-src/` (CTS at 93bca018, made a git
repo so `cts-build-zephyr.sh` can clone from it), `zephyr-ws/` (west
workspace: `yakogl` clone at `2718a1e9`, `zephyr`, `modules/…`), `sdk/`,
`venv/` (`WEST_VENV`), `work/` (stage 1), `logs/`, `tmp/` (scratch).

## Commands

Stage 1 (tools/cts/README.md section 2d, `BOARD=rpi_4b`):

```sh
ROOT=/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b
export PATH=$ROOT/venv/bin:/home/tasuku/.cargo/bin:$PATH
export ZEPHYR_BASE=$ROOT/zephyr-ws/zephyr
export ZEPHYR_SDK=$ROOT/sdk/zephyr-sdk-1.0.1
export ZEPHYR_SDK_INSTALL_DIR=$ZEPHYR_SDK
export RUSTUP_TOOLCHAIN=nightly
ZEPHYR_MODULES="$($ROOT/zephyr-ws/yakogl/tools/cts/zephyr-modules.sh $ROOT/zephyr-ws)" \
CTS=$ROOT/cts-src WORK=$ROOT/work BOARD=rpi_4b \
    $ROOT/zephyr-ws/yakogl/tools/cts/cts-build-zephyr.sh
```

Row D proof (stage 2, one image, into a scratch dir; `$TMPDIR` is unset on
this host and `/tmp` is outside the allowed directories, so `TMPDIR` was
set to `$ROOT/tmp`):

```sh
export ZEPHYR_WS=$ROOT/zephyr-ws DEQP_WORK=$ROOT/work WEST_VENV=$ROOT/venv
TMPDIR=$ROOT/tmp tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt $TMPDIR/deqp-D D
```

## Results

- Stage 1: rc 0 in 100.7 s (64 cores). 418 ninja targets; libraries in
  `work/build`, ending with `modules/gles2/libdeqp-gles2-package.a`.
  `work/` is 2.0 GB (`cts` 1.7 GB, `build` 87 MB, `flags` 167 MB).
- Row D (`# image D: 292`): the `.built` line

  ```
  D rc=0 sha256=74ae8f8b1812fe314e52600b0618344888663084f4526a3917b9f7281909c8f0 build=2718a1e9
  ```

  and the verdict from `check-deqp-images.py` in `D.log`:

  ```
  check-deqp-images: the dEQP image recipes build what conformance needs
  ```

  Image links at RAM 155444 KB of 256 MB (59.30 %). Cold build 59.8 s,
  warm (ccache) 16.1 s. The test directory was deleted after the check.

## Notes for the next session

- `pi4-deqp-images.sh` exits 1 on a selective build unless the last line
  of the table is among the requested labels. Cause: `selected()` pipes
  `images()` through a `while` whose body ends in a failed test for the
  unselected last line, and `set -o pipefail` propagates it. The build
  itself is fine — trust the per-image `<LABEL> rc=…` line. (Reproduced
  minimally with a two-label table; a full-tree run exits 0.)
- `capi/yakogl_capi.cmake` needs a cargo with `rustc` next to it. The
  host's kache cargo shim (`~/.local/lib/kache/shims/cargo`) has none;
  put `~/.cargo/bin` earlier on PATH (the same choice
  `tools/hostharness/glmark2/run.sh:169-174` makes).
- Host cmake is 4.3.5, which rejects CTS's zlib
  (`cmake_minimum_required(VERSION 2.4.4)`). The venv's cmake 3.31.10
  configures the whole tree.
- A Zephyr tree imported from a tarball (no git history) must still be a
  git repo with `refs/heads/manifest-rev` at HEAD, or `west list` and
  `zephyr_module.py` fail the manifest import from `west.yml`.
- The stage-1 CTS copy is a `--shared` clone of `cts-src` with
  `cts-yakogl-target.patch` and `cts-yakogl-zephyr.patch` applied; its
  externals are symlinks into `cts-src/external/*/src`. Keep `cts-src`
  while `work/cts` is in use.

## Environment values

- `DEQP_WORK` = `/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/work`
- `ZEPHYR_WS` = `/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/zephyr-ws`
- `WEST_VENV` = `/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/venv`
- `ZEPHYR_SDK_INSTALL_DIR` = `/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-rpi_4b/sdk/zephyr-sdk-1.0.1`
