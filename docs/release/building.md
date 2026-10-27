<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Building every benchmark image from a clean machine

This guide starts from a machine that has only this repository. It lists every
input the images need, where each one comes from, how to build it, which
variable points to it, and which images need it. Then it gives the build
command for each image on each board.

Boards: TI AM62P (`am62p`, Zephyr board `verdin_am62p/am62p54/a53`), NXP
i.MX8MP (`imx8mp`, `verdin_imx8mp/mimx8ml8/a53`), Raspberry Pi 4 (`pi4`,
`rpi_4b`).

Images:

| Image | Sample (under `driver/samples/zephyr/`) | Needs |
|---|---|---|
| glmark2 | `glmark2` | 1, 2, 3, 4, 5 (AM62P also 6) |
| Slint demo (not a benchmark) | `slint` | 1, 2, 3, 4, 7, 8 (AM62P also 6) |
| gl_test | `gl_test` | 1, 2, 3, 4 (AM62P also 6) |
| dEQP | `deqp` | 1, 2, 3, 4, 9 (AM62P also 6) |

## How this file was checked

Every command here is copied from the tree, with the file and line it comes
from; written 2026-10-08. No step has been run on a clean machine, and no image
was built for this file (network commands and scripts outside `tools/check.sh`
were refused in the session that wrote it). The gate run of that day and what
it showed is in `git show 5ad2fa0e:docs/release/building.md`. A first run on a
clean machine may still find a gap. If it does, fix the step here.

Re-checked on 2026-10-14 at `a63fcc5b` without running a build (the session
could not run build tools): every repository path named here exists, and the
`file:line` references that had moved were corrected to the lines of this commit
(`tools/check.sh` grew by about 1,200 lines since). Line numbers drift; when
one is wrong, search the file for the name beside it. The external inputs
(URLs, hashes, versions) were not fetched.

## The inputs

"Pin" is what the repository fixes. "Not pinned" means the tree names no
version; the text says where the tree uses the input. Lines are in this
repository at the commit that added this file.

### 1. The Zephyr west workspace

- **What:** Zephyr plus the modules the A53 and Pi 4 images need (fatfs,
  hal_nxp, hal_ti, picolibc, zephyr-lang-rust). This repository is a Zephyr
  module of it (`zephyr/module.yml`).
- **Source:** `https://github.com/zephyrproject-rtos/zephyr`, pinned to commit
  `af17c0c8aa2a82a0b714a423915ad3610063ee68` by `west.yml:27`. The module
  allow-list is `west.yml:28-34`.
- **Fetch:**
  ```sh
  python3 -m venv $WORK/venv && . $WORK/venv/bin/activate && pip install west
  west init -m https://github.com/signal-slot/yakogl --mr main $WORK/ws
  cd $WORK/ws && west update
  pip install -r zephyr/scripts/requirements-base.txt
  ```
  (`driver/samples/zephyr/README.md:20-27`). Your own checkout of this
  repository can replace the `west init` clone: use `west init -l` on it, or
  keep the checkout apart and set `YAKOGL_DIR` (below).
- **Variable:** `ZEPHYR_WS`, the directory that holds `zephyr/` and `.west/`.
  `ZEPHYR_BASE` is `$ZEPHYR_WS/zephyr` (`tools/cts/common-images.sh:155`).
  `tools/cts/imx8mp-env.sh:33` calls the same directory `ZEPHYR_WORKSPACE`;
  give both the same value.
- **Needed by:** every image.
- **Module list:** the scripts pass `-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)`
  (`tools/cts/zephyr-modules.sh:1-30`). It needs `west` on `PATH`. In a
  workspace made from `west.yml` the list is what west finds anyway.
- **Also on the host:** `cmake` >= 3.20 (`driver/samples/zephyr/glmark2/CMakeLists.txt:17`),
  `ninja`, `dtc`, `patch` (glmark2 patches a file with it,
  `.../glmark2/CMakeLists.txt:67`), `python3`, `git`.

### 2. The west venv

- **What:** a Python venv holding `west` and Zephyr's Python requirements.
- **Source:** `pip install west`; the requirement file is in the workspace.
  The `west` version is not pinned.
- **Variable:** `WEST_VENV` (its `bin/` goes first on `PATH`,
  `tools/cts/common-images.sh:138`). The i.MX8MP scripts call it
  `ZEPHYR_VENV` and source `$ZEPHYR_VENV/bin/activate`
  (`tools/cts/imx8mp-env.sh:10,35-37`); give both the same value.
- **Needed by:** every image.
- **Caution:** the Zephyr rows of `tools/check.sh` (`:2517-2519`) ignore
  `WEST_VENV` and look for `$ZEPHYR_WS/.venv`. If you run the full gate, make the venv there, or link
  it (`ln -s $WORK/venv $ZEPHYR_WS/.venv`).

### 3. The Zephyr SDK

- **What:** the `aarch64-zephyr-elf` GCC toolchain (GCC 14.3 and its
  libstdc++, `driver/docs/glmark2.md:21`).
- **Source:** the Zephyr SDK, version **1.0.1**
  (`driver/samples/zephyr/README.md:27`, `tools/cts/imx8mp-env.sh:11`).
  Download page:
  `https://docs.zephyrproject.org/latest/develop/toolchains/zephyr_sdk.html`.
  Install the AArch64 toolchain (`aarch64-zephyr-elf`) and the host tools; run
  the SDK's `setup.sh` (`./setup.sh -t aarch64-zephyr-elf`). Its `-c` option
  registers the SDK in `~/.cmake/packages`; it is not needed, because
  `ZEPHYR_SDK_INSTALL_DIR` is enough. I did not check the archive name.
- **Variable:** `ZEPHYR_SDK_INSTALL_DIR` (west and the dEQP stage 1 script
  both read it).
- **Needed by:** every image.

### 4. A nightly Rust toolchain

- **What:** the Rust part of YakoGL (`libyakogl.a`) and the Slint
  staticlib are built with `-Z build-std` for `aarch64-unknown-none`.
- **Source:** `rustup`. Use `nightly` with the `rust-src` component. The
  tree does not pin a nightly date (`README.md`, the `YAKOGL_RUSTUP_TOOLCHAIN`
  and `YAKOGL_CARGO_BUILD_STD` rows of the options table,
  `capi/yakogl_capi.cmake:13-14`).
  Minimum stable for the host crates: `rust-version` 1.88 (`Cargo.toml`).
  ```sh
  rustup toolchain install nightly --component rust-src
  rustup target add aarch64-unknown-none aarch64-unknown-none-softfloat
  ```
- **Variable:** `RUSTUP_TOOLCHAIN=nightly`.
- **Needed by:** every image (glmark2, gl_test and dEQP through the YakoGL
  library; Slint also through its own cargo step).
- **`CC_aarch64_unknown_none`:** not needed for an image. The Zephyr build
  hands the Zephyr toolchain's compiler to the C wrappers
  (`build.rs:24-33`). Only the host gate (`tools/check.sh`) builds them
  outside Zephyr, and there `build.rs` uses `clang` unless this variable names
  another cross compiler (`build.rs:108-112`). On a machine with only a
  versioned clang (for example `clang-23`), set it to that name.

### 5. glmark2

- **What:** the benchmark, compiled by path and unchanged except one patched
  file. GPL-3.0-or-later: never vendored here.
- **Source:** `https://github.com/glmark2/glmark2` at commit
  `22c527cb0556f3a1ac4445aaa52cc532760928d5` (version 2023.01), pinned in
  `driver/samples/zephyr/glmark2/CMakeLists.txt:28-29`. The build refuses any
  other commit, and a tree with local changes (`:45-47`).
- **Fetch:**
  ```sh
  git clone https://github.com/glmark2/glmark2 $WORK/glmark2
  git -C $WORK/glmark2 checkout 22c527cb0556f3a1ac4445aaa52cc532760928d5
  ```
- **Variable:** `GLMARK2_SRC` (or `-DGLMARK2_SRC=`; it must contain
  `src/default-benchmarks.h`, `tools/cts/common-images.sh:146`).
- **Needed by:** the glmark2 image.

### 6. The PowerVR firmware header `driver/src/fw/pvr_fw_blob.h`  — cannot be public

- **What:** the AM62P's GPU firmware, `rgx.fw`, embedded as a C array
  (`driver/src/pvr_fw_boot.c:303`). It is Imagination and TI property.
  **It is not in the repository, never committed, and not redistributable.**
  `driver/.gitignore` ignores `src/fw/`. The header is also not
  reproducible without the firmware file.
- **How you get it:** take `rgx.fw` for BVNC 36.53.104.796 (the file name
  used in the docs is `rgx.fw.36.53.104.796`) under its licence from one of:
  TI's Processor SDK (`README.md:112`), the board's own
  `/lib/firmware`, or the matching upstream linux-firmware or Mesa
  open-firmware package (`driver/tools/mkfwblob.sh:6-8`). I did not check
  that the upstream package carries this exact version. You must accept the
  licence of whichever you take.
- **Make the header:**
  ```sh
  driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796   # writes driver/src/fw/pvr_fw_blob.h
  ```
  A header from another checkout works too: it depends on the firmware only.
- **Variable:** none. The file must be at that path in the checkout you build.
  A git worktree or a fresh clone has no copy; copy it in.
- **Needed by:** every **AM62P** image (the scripts stop without it:
  `tools/cts/common-images.sh:141`, `tools/cts/am62p-deqp-images.sh:119`).
  The i.MX8MP and the Pi 4 do not need it. The full gate also needs it
  (ccfarm copies it into a gate checkout: `copy_from_origin`).

### 7. The no_std Slint workspace

- **What:** upstream Slint plus a patch that makes the core, the compiler and
  the FemtoVG renderer `no_std`.
- **Source:** `https://github.com/slint-ui/slint` at commit
  `1007c032ca1573e60dce2113e802e0ccb91c2641`, then
  `examples/nostd/slint-nostd.patch`
  (`examples/nostd/README.md:11,43-45`).
- **Fetch:**
  ```sh
  N=$REPO/examples/nostd        # $REPO: this repository
  git clone https://github.com/slint-ui/slint.git $WORK/slint-nostd/slint
  cd $WORK/slint-nostd/slint
  git checkout 1007c032ca1573e60dce2113e802e0ccb91c2641
  patch -p1 < $N/slint-nostd.patch
  ```
  The README says the patch reproduces 960 text files of the original tree;
  six binary assets are not in it and no sample needs them (`README.md:68-79`).
- **Variable:** `SLINT_NOSTD_DIR=$WORK/slint-nostd/slint` (it must hold
  `internal/renderers/femtovg/Cargo.toml`, `examples/slint/CMakeLists.txt:35`).
- **Needed by:** the Slint image.
- **Network:** the cargo build also fetches the crates the patched workspace
  depends on from crates.io. Run it once with network, or vendor them.

### 8. The no_std femtovg and glow forks

- **What:** two crates.io crates with a `no_std` patch.
- **Source:** `femtovg` **0.26.0** and `glow` **0.18.0** from crates.io,
  patched with `femtovg-0.26.0-nostd.patch` and `glow-0.18.0-nostd.patch`
  (same README, table at `:9-13`, steps at `:47-53`).
- **Fetch** (the README copies from `~/.cargo/registry`; a plain download does
  the same; the `static.crates.io` URL form is standard, I did not run it):
  ```sh
  mkdir -p $WORK/nostd && cd $WORK/nostd
  curl -L https://static.crates.io/crates/femtovg/femtovg-0.26.0.crate | tar xz && mv femtovg-0.26.0 femtovg
  curl -L https://static.crates.io/crates/glow/glow-0.18.0.crate       | tar xz && mv glow-0.18.0 glow
  (cd femtovg && patch -p1 < $N/femtovg-0.26.0-nostd.patch)
  (cd glow    && patch -p1 < $N/glow-0.18.0-nostd.patch)
  cp $N/nostd-workspace.Cargo.toml Cargo.toml
  ```
- **Variable:** `NOSTD_PATCH_DIR=$WORK/nostd`. If unset, CMake uses
  `$SLINT_NOSTD_DIR/../nostd` (`examples/slint/CMakeLists.txt:40-42`),
  which is `$WORK/slint-nostd/nostd`, not the directory above. Set the
  variable, or make the directory name match. The build needs `femtovg/Cargo.toml`
  and `glow/Cargo.toml` in it (`:43`).
  `tools/cts/common-images.sh` passes it only when it is set (`:113,119`).
- **Needed by:** the Slint and femtovg images.

### 9. dEQP stage 1 (the CTS libraries)

- **What:** the dEQP-GLES2 libraries, built once with the Zephyr SDK compiler.
  The dEQP images link them.
- **Source:** `https://github.com/KhronosGroup/VK-GL-CTS`. The scripts do
  **not** pin a revision: they take the HEAD of the checkout you give them
  (`tools/cts/cts-build-zephyr.sh:56-57`). The repository cites commit
  `93bca01` (`NOTICE:173`; full hash
  `93bca01861b0e3ef3c387027a9791e6d065f900c` in the link at
  `docs/online-shaderc/v3d-backend.md:2477`) for the reference code and the
  patches `tools/cts/cts-yakogl-target.patch` and `cts-yakogl-zephyr.patch`.
  Use that commit. I did not check that the patches still apply to any other.
  Apache-2.0.
- **Fetch and build:**
  ```sh
  git clone https://github.com/KhronosGroup/VK-GL-CTS $WORK/VK-GL-CTS
  git -C $WORK/VK-GL-CTS checkout 93bca01861b0e3ef3c387027a9791e6d065f900c
  (cd $WORK/VK-GL-CTS && python3 external/fetch_sources.py)   # as tools/cts/cts-build.sh:73
  export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
  export ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)"
  CTS=$WORK/VK-GL-CTS WORK=$DEQP_WORK BOARD=<zephyr board> tools/cts/cts-build-zephyr.sh
  ```
  The user's CTS checkout is not written to: the script makes a `--shared`
  clone in `$DEQP_WORK/cts`, symlinks the fetched `external/*/src` into it,
  and applies the two patches there (`:56-64`). `fetch_sources.py` is needed
  because of that symlink step; the script does not run it. Needs `cmake` and
  `ninja`. About 1.9 GB (`tools/cts/imx8mp-deqp-stage1.sh:10`).
- **If libpng's NEON file fails** with `arm_neon.h: lane index must be a
  constant immediate`, the stage 1 was compiled at -O0 (the toolchain file
  carries no `-O`). Export `CFLAGS=-O2 CXXFLAGS=-O2` and start again with an
  empty `build/` in the stage 1 directory. Seen once, on 2026-10-08 with the
  i.MX8MP stage 1; not checked since.
- **One stage 1 per board family:** `BOARD` selects the compile flags, so use
  `BOARD=verdin_am62p/am62p54/a53` (the script's default) for the AM62P,
  `BOARD=rpi_4b` for the Pi 4 (`tools/cts/pi4-deqp-images.sh:26-27`), and the
  i.MX8MP wrapper `tools/cts/imx8mp-deqp-stage1.sh` (`BOARD` defaults to
  `verdin_imx8mp/mimx8ml8/a53`, `tools/cts/imx8mp-env.sh:30`). Give each its
  own `WORK`.
- **Variables:** `DEQP_WORK` for the AM62P and the Pi 4 (the stage 1 `WORK`;
  it holds `cts/` and `build/`, checked at `tools/cts/am62p-deqp-images.sh:115-118`
  and `tools/cts/pi4-deqp-images.sh:96-98`). `DEQP_STAGE1` for the i.MX8MP (the
  same directory layout, `tools/cts/imx8mp-deqp-stage1.sh:9-10`).
  `IMX8MP_OUT`: any empty directory; the i.MX8MP scripts write images and logs
  there (`tools/cts/imx8mp-env.sh:12,40`).
- **Needed by:** the dEQP images only.

### 10. Inputs of the host gate only

These are not needed to build an image. They are needed for the full
`tools/check.sh`. Without them the rows are reported as gaps and the gate is
red (`tools/check.sh`'s header). Two tools set a gate host up:
`tools/setup-gate-host.sh --check` prints one ok / missing line per tool the
gate needs (`--install` adds what rustup can add), and `tools/cts/deps-env.sh`
names the dependency layout (one directory per host,
`~/.local/share/connected/deps/yakogl` unless `CONNECTED_DEPS_DIR` is set;
`tools/cts/deps-env.sh --layout` prints it) and prints
the variables below from it; `.github/workflows/gate.yml` reads it the same
way.

| Variable | What | Source and pin | Used at |
|---|---|---|---|
| `HAL_NXP_DIR` | Zephyr's `modules/hal/nxp` | made by `west update` in 1; the generator pins hal_nxp `1e91007b` and refuses another | `tools/check.sh:2717-2731` |
| `PCO_SHC` | Mesa's PowerVR compiler `pco_shc`, built from Mesa 26.2.1 | tarball `https://archive.mesa3d.org/mesa-26.2.1.tar.xz`, sha256 `c47e81bddc4760360a41ac3c5acec38acb81f9d750ecef47e7f3adc7021a4442` (`third_party/mesa-26.2.1/UPSTREAM`); build with `tools/pco_shc/build_pco_shc.sh <tarball> <work dir>`; it must print id `f537fc86b41f` (same id only on a similar toolchain: LLVM/clang/libclc 23.1.2 and SPIRV-LLVM-Translator 23.1.1 were used, `tools/pco_shc/build_pco_shc.sh:24-26`) | `tools/check.sh:2579-2673` |
| `GLSLANG` | `glslangValidator` >= 16.4 | distro package `glslang` | `tools/glsl2usc.sh:17` |
| `GLSL2USC_CACHE` | a cache directory for the shader compile (3.5 min with a warm one) | any writable directory | `tools/glsl2usc.sh:18`, `tools/check.sh:2579` |
| `MESA_V3D_TREE` | optional: a built Mesa tree to compare V3D checks against | not needed: the Mesa sources are vendored in `third_party/mesa-26.2.1` | `tools/check.sh:1274`, `:1679` |
| `CC_aarch64_unknown_none` | cross C compiler, see 4 | | `build.rs:108-112` |
| `BINDGEN_EXTRA_CLANG_ARGS` | only when the bindgen row (`csbgen-bindings --check`) fails with `'stdbool.h' file not found`: the host has a libclang but not clang's resource headers | point it at gcc's builtin headers, for example `-isystem $ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/lib/gcc/aarch64-zephyr-elf/14.3.0/include` (measured 2026-10-08 on a host without clang) | `tools/check.sh:2432` |

### 11. Board run inputs (not for building)

`BOARD_HOST`, `BOARD_UART`, `BOARD_BOOT`, `BOARD_HDMI_TIMINGS` (the Pi 4
panel's `hdmi_timings`) are read by `tools/board.sh` at run time
(`tools/board.sh:133-149`). They never go into a file. They are not needed to
build.

## Set up the environment

Example. Use your own paths.

```sh
export WORK=/path/to/work                 # outside the repository
export REPO=/path/to/yakogl               # this repository
export ZEPHYR_WS=$WORK/ws                 # input 1
export WEST_VENV=$WORK/venv               # input 2
export ZEPHYR_SDK_INSTALL_DIR=$WORK/zephyr-sdk-1.0.1   # input 3
export RUSTUP_TOOLCHAIN=nightly           # input 4
export GLMARK2_SRC=$WORK/glmark2          # input 5
export SLINT_NOSTD_DIR=$WORK/slint-nostd/slint   # input 7 (the Slint demo only)
export NOSTD_PATCH_DIR=$WORK/nostd        # input 8 (the Slint demo only)
export DEQP_WORK=$WORK/deqp-am62p         # input 9 (AM62P; one per board)
cd $REPO
```

For the AM62P also run `driver/tools/mkfwblob.sh <rgx.fw>` (input 6).

## Build the common set: glmark2, gl_test

One script builds the rows of `tools/cts/common-tree.txt` for a board
(`GM` glmark2, `GT` gl_test). The Slint demo is no row: build it with the
plain `west build` below. Each row gets its own `west build` directory and a
`<LABEL>.built` file with `rc`, `sha256` and the build ids
(`tools/cts/common-images.sh:5-20`). A directory that exists is refused: use a
new `<img-dir>`.

```sh
tools/cts/cts-common.sh am62p  build build/common/am62p-$(git rev-parse --short=8 HEAD)
tools/cts/cts-common.sh imx8mp build build/common/imx8mp-$(git rev-parse --short=8 HEAD)
tools/cts/cts-common.sh pi4    build build/common/pi4-$(git rev-parse --short=8 HEAD)
```

One row only: add its label, for example `... build <img-dir> GM`. To print
the `west` arguments without building: `tools/cts/cts-common.sh --dry-run
<board> build <img-dir>`; it needs none of the inputs
(`tools/cts/common-images.sh:20`).

What each row runs, as a plain `west build` (from `$ZEPHYR_WS`, with
`ZEPHYR_BASE=$ZEPHYR_WS/zephyr`, `MODS=$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)`;
`tools/cts/common-images.sh:173-174`):

```sh
west build -p always -b <ZBOARD> -d <dir> $REPO/driver/samples/zephyr/<sample> -- \
    -DZEPHYR_MODULES="$MODS" -DYAKOGL_DIR=$REPO <arm flags> <row flags>
```

| Board | `<ZBOARD>` | `<arm flags>` (watchdog self-arm, `common-images.sh:70-83`) |
|---|---|---|
| AM62P | `verdin_am62p/am62p54/a53` | `-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n` |
| i.MX8MP | `verdin_imx8mp/mimx8ml8/a53` | `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` |
| Pi 4 | `rpi_4b` | none |

| Image | `<sample>` | `<row flags>` (`common-images.sh:108-124`) |
|---|---|---|
| glmark2 | `glmark2` | `-DGLMARK2_SRC=$GLMARK2_SRC` |
| Slint demo | `slint` | `-DSLINT_NOSTD_DIR=$SLINT_NOSTD_DIR -DNOSTD_PATCH_DIR=$NOSTD_PATCH_DIR`; for the scan-out view add `-DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y` (the AM62P has no view and makes no copy: `board_show()` flips the DSS plane to the PowerVR window, `driver/zephyr/lib/board/include/yakogl_board.h:114-118`, `driver/zephyr/lib/board/am62p/board_am62p.c:69-87`; there the image is the same copy-free one either way, `docs/notes/am62p-scanout-view.md`) |
| gl_test | `gl_test` | none (the sample's default case list) |

The `<arm flags>` are required: `tools/board.sh` refuses an image without its
board's watchdog line.

### Check the result

- `<img-dir>/<LABEL>.built` says `rc=0`, a `sha256` and the build ids. A
  non-zero `rc` is named: `no_marker` (no watchdog line in the binary:
  `board.sh` would refuse it), `no_build_line` (AM62P, Pi 4: the console would
  not say which build ran), `no_image`, `exists`
  (`tools/cts/common-images.sh:40-45,170,182-190`). The script exits non-zero when
  any row failed; its last line is `common-images: <n> row(s) not built`.
- The image is `<img-dir>/<LABEL>/zephyr/zephyr.bin`; the log is
  `<img-dir>/<LABEL>.log`.
- The run on a board is `tools/cts/cts-common.sh <board> run <img-dir> <outdir>`
  and then the tally; that needs a board (`tools/cts/cts-common.sh:10-30`).
  `tools/board.sh --dry-run <board> ...` prints what it would do, for a check
  without a board (`tools/board.sh:20-26`).

## Build the dEQP images

Do input 9 first, once per board family. Then:

**AM62P** (17 images of the whole GLES2 tree, one per line of
`tools/cts/am62p-full-tree.txt`; `tools/cts/am62p-deqp-images.sh:1-60`):

```sh
export DEQP_WORK=$WORK/deqp-am62p
tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt \
    build/deqp/am62p-$(git rev-parse --short=8 HEAD) $(git rev-parse --short=8 HEAD) \
    > build/deqp/am62p-$(git rev-parse --short=8 HEAD)/build.out
# the 4x multisample image: tools/cts/am62p-ms4-tree.txt, tag ms4
```

Make the output directory first (`mkdir -p`).
`--dry-run` lists the images with none of the environment. The script prints
`img<N> rc=<rc> bin=<sha256 prefix>` per image.

**Pi 4** (one `deqp` image per block of `tools/cts/pi4-full-tree.txt`;
`tools/cts/pi4-deqp-images.sh:1-30`):

```sh
export DEQP_WORK=$WORK/deqp-pi4          # stage 1 built with BOARD=rpi_4b
tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-$(git rev-parse --short=8 HEAD)
```

It runs `cmake -GNinja -DBOARD=rpi_4b ...` and `ninja` itself, then
`tools/cts/check-deqp-images.py` on the resolved `.config` (an image that
is not a conformance image gets `rc=not-conformance`).

**i.MX8MP** (one image carrying a 29-row table, selected at run time;
`tools/cts/imx8mp-deqp-build.sh:1-30`):

```sh
export ZEPHYR_WORKSPACE=$ZEPHYR_WS ZEPHYR_VENV=$WEST_VENV IMX8MP_OUT=$WORK/imx8mp-out
export DEQP_STAGE1=$WORK/deqp-imx8mp
tools/cts/imx8mp-deqp-stage1.sh                       # stage 1, once
tools/cts/imx8mp-deqp-build.sh deqp-$(git rev-parse --short=8 HEAD) \
    'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
```

The image is `$IMX8MP_OUT/<name>/zephyr/zephyr.bin`; the log is
`$IMX8MP_OUT/<name>.log`. The script ends by printing whether
`CONFIG_DEQP_REPORT_ES2=y` is in the image.

### Check the result

Each script writes a `rc=` line per image and a `sha256`. `rc=0` and a
non-empty `zephyr.bin` is a build. That a dEQP image is a conformance image
(compiler, ES 2.0 reporting, no table) is checked by
`tools/cts/check-deqp-images.py` (the Pi 4 script runs it; run it by hand on
the others). Running them on a board is `tools/board.sh <board> tree <table>
<outdir>` (`tools/board.sh:75-110`), and needs a board.

## Check the host side

`tools/check.sh --quick` is the short form of the gate, for an edit loop (about
120 rows; 14 minutes in the first walk and 4 minutes on a warm tree in the
second, see `docs/release/readme-walkthrough.md`), and
`tools/check.sh --changed <base>` runs the rows a change reaches. Neither is the
landing gate: the CI runs the full `tools/check.sh` on every pull request
(`.github/workflows/gate.yml`). The full run needs the inputs of section 10, and
the AM62P firmware header (input 6). Without them it reports gaps and is red.

## What did not work or is not settled

- No clean-machine run of any step above. See "How this file was checked".
- The Zephyr SDK archive name, and the `static.crates.io` download URLs, are
  from general knowledge, not from the tree.
- `tools/check.sh` looks for `$ZEPHYR_WS/.venv` and ignores `WEST_VENV`.
  This guide works around it; a script change would remove the need.
