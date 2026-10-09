<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP stage 1 (host only): built, row D proven

Status: **stage 1 built and proven.** The dEQP-GLES2 libraries for
`rpi_4b` (`DEQP_WORK`) are in each worker's deps cache, listed below. On
each worker, row D of `tools/cts/pi4-full-tree.txt` built with
`tools/cts/pi4-deqp-images.sh`: `D rc=0` and the conformance verdict
passed; the test image was then deleted. No board was touched and
nothing was flashed.

Stage 1 is built on two farm workers, each in its own deps cache:

| worker | `DEQP_WORK` | stage 1 | row D |
|---|---|---|---|
| host A (12 cores, loaded) | `$DEQP_WORK` | rc 0 | `D rc=0`, verdict passed |
| host B (64 cores) | `$DEQP_WORK` | rc 0 | `D rc=0`, verdict passed |

The host A build is in the next section. Every section after it
describes the host B build, and the paths there are on that worker.

## On host A

Inputs: the shared deps cache `<deps>`
already held them all at the pinned revisions: `VK-GL-CTS/` at
`93bca01861b0e3ef3c387027a9791e6d065f900c` with its externals fetched,
`zephyr-ws/` (west workspace, `zephyr` at
`af17c0c8aa2a82a0b714a423915ad3610063ee68`, fatfs, hal_nxp, hal_ti,
picolibc; `west` in `zephyr-ws/.venv`), and `zephyr-sdk-1.0.1/` with
`gnu/aarch64-zephyr-elf` (gcc 14.3.0). Rust nightly has
`aarch64-unknown-none`. Host cmake is 4.2.3; it configures CTS's zlib with
`CMAKE_POLICY_VERSION_MINIMUM=3.5` in the environment.

Commands (with `D=<deps>`):

```sh
export PATH=$D/zephyr-ws/.venv/bin:$HOME/.cargo/bin:$PATH RUSTUP_TOOLCHAIN=nightly
export ZEPHYR_BASE=$D/zephyr-ws/zephyr ZEPHYR_SDK=$D/zephyr-sdk-1.0.1 ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export CMAKE_POLICY_VERSION_MINIMUM=3.5
ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh $D/zephyr-ws)" CTS=$D/VK-GL-CTS \
    WORK=$D/deqp-stage1-rpi_4b/work BOARD=rpi_4b tools/cts/cts-build-zephyr.sh
ZEPHYR_WS=$D/zephyr-ws WEST_VENV=$D/zephyr-ws/.venv DEQP_WORK=$D/deqp-stage1-rpi_4b/work \
    tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt /tmp/yakogl-pi4-stage1/rowD-hostA D
```

Results:

- Stage 1: the first run was stopped by a 10-minute tool limit at ninja
  step 400 of 418 on a host at load 15 to 30. The rerun (same command)
  finished rc 0 in 313.8 s, 417 steps, ending
  `cts-build-zephyr: libraries in …/deqp-stage1-rpi_4b/work/build`.
  `work/` is 2.0 GB. The inputs it uses take 4.2 GB (`VK-GL-CTS`), 3.5 GB
  (`zephyr-ws`) and 1.5 GB (`zephyr-sdk-1.0.1`), shared with the other
  boards.
- Row D in 200.1 s, the `.built` line

  ```
  D rc=0 sha256=b58b874356375ea204ea9c12e38ae8a1cc57c58c3fceb4e9c5ed1c45db7802ce build=25610e54
  ```

  and from `D.log`:

  ```
               RAM:     155440 KB       256 MB     59.30%
  check-deqp-images: the dEQP image recipes build what conformance needs
  ```

  The script exited 1, which is the selective-build quirk described under
  "Notes for the next session"; the `D rc=0` line is the result. The test
  directory was deleted after the check.

### The host gate on host A

`tools/check.sh --quick` on this branch, which changes only this file (no
gate step reads `docs/`), run with two toolchains:

| toolchain | result | failures |
|---|---|---|
| nightly: `RUSTUP_TOOLCHAIN=nightly`, which the farm session exports for the Pi 4 sample builds (`.ccfarm/project.toml`); clippy 0.1.101 (2026-10-06) | 83 ok, 16 failed | 13 clippy rows (`needless_range_loop` and others in `src/`, that nightly's lint set; the plan's `clippy-clean` item) and the 3 host rows below |
| default: `RUSTUP_TOOLCHAIN` unset, so stable rustc 1.99.0 / clippy 0.1.99, the toolchain `tools/check.sh:1275` is written for; 4345 s at load 20 to 26 | 96 ok, 3 failed, 0 gaps | the 3 host rows below |

The three rows that fail on either toolchain are each a tool this worker
lacks: the same three `docs/release/building.md:32-34` lists from a clean
machine, and the kind `docs/HANDOFF.md:47-48` describes ("each for a tool
the host lacks").

- `no table in the artefact`: the control counts 0 symbols. The release
  profile has `lto = true` (`Cargo.toml:184`), so the archive's object is
  LLVM bitcode (magic `BC C0 DE`); the host's GNU `nm` has no LLVM plugin
  (`/usr/lib/bfd-plugins/` holds only GCC's `liblto_plugin.so`) and there
  is no `llvm-nm`, so `nm` says `rcgu.o: file format not recognized`, which
  the step's `2>/dev/null` hides.
- `tools   board.sh dry-run matrix and replays`: 207 checks, 1 failed,
  `console_from_ts` (`tools/board.sh:910`), which needs `gawk`; the host
  has only `mawk` (`tools/board_selftest.sh: 行 592: gawk: command not found`).
- `gen     src/pvr/csbgen.rs + csbgen_wrap.c == bindgen`: `libclang-21` is
  installed without clang's resource headers (`stdbool.h` not found) and
  there is no `clang`.

For the stable run the aarch64 targets were added to that toolchain
(`rustup target add aarch64-unknown-none aarch64-unknown-none-softfloat
--toolchain stable`); without them the four `check … (aarch64-unknown-none)`
rows are a gap. An earlier nightly run (79 ok, 20 failed) also saw three of
those rows and the artefact rows fail with cc-rs `failed to find tool
"clang"`; the two later runs did not reproduce it. Installing `gawk`, an
LLVM `nm` (or the LLVMgold plugin) and clang's resource headers on the
worker would make the default-toolchain gate green here; that is outside
this item.

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
`<deps>/deqp-stage1-rpi_4b`
(8.1 GB total): `dl/` archives, `cts-src/` (CTS at 93bca018, made a git
repo so `cts-build-zephyr.sh` can clone from it), `zephyr-ws/` (west
workspace: `yakogl` clone at `2718a1e9`, `zephyr`, `modules/…`), `sdk/`,
`venv/` (`WEST_VENV`), `work/` (stage 1), `logs/`, `tmp/` (scratch).

## Commands

Stage 1 (tools/cts/README.md section 2d, `BOARD=rpi_4b`):

```sh
ROOT=<deps>/deqp-stage1-rpi_4b
export PATH=$ROOT/venv/bin:$HOME/.cargo/bin:$PATH
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
  A second run rebuilt D with the identical sha256: the image build is
  reproducible.

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
- `pi4-deqp-images.sh` builds fine from either checkout against this
  deps workspace (verified both ways: the clone gave `build=2718a1e9`,
  a workspace copy gave `build=75277d5c-dirty`, both `D rc=0`). The
  `build=` stamp comes from the checkout the script runs from
  (`git -C "$REPO" describe`), so run it from the checkout whose stamp
  the image should carry.
- The stage-1 CTS copy is a `--shared` clone of `cts-src` with
  `cts-yakogl-target.patch` and `cts-yakogl-zephyr.patch` applied; its
  externals are symlinks into `cts-src/external/*/src`. Keep `cts-src`
  while `work/cts` is in use.

## Environment values

host A:

- `DEQP_WORK`, `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`: the host's own directories (not recorded)

host B:

- `DEQP_WORK`, `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`: the host's own directories (not recorded)
