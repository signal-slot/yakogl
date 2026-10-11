<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Zephyr samples

Everything here builds from this repository alone: the GPU drivers, YakoGL and
the board support for the two Cortex-A53 targets (`driver/boards/toradex`,
`driver/soc/ti/k3`, `driver/dts`, `driver/drivers`) are all in it. The
Raspberry Pi 4 uses Zephyr's own `rpi_4b`.

| Board | Target | GPU |
|---|---|---|
| Toradex Verdin AM62P (Dahlia) | `verdin_am62p/am62p54/a53` | PowerVR BXS-4-64 |
| Toradex Verdin iMX8M Plus | `verdin_imx8mp/mimx8ml8/a53` | Vivante GC7000UL |
| Atmark Armadillo-X2 | `armadillo_x2/mimx8ml8/a53` | Vivante GC7000UL |
| Atmark Armadillo-X2, direct boot (TF-A BL33, `docs/direct-boot.md`) | `armadillo_x2/mimx8ml8/a53/direct` | Vivante GC7000UL |
| Raspberry Pi 4 | `rpi_4b` | Broadcom V3D 4.2 |

## The workspace

The repository root holds a west manifest (`west.yml`): Zephyr pinned to the
revision these samples were built against, and only the modules they need
(fatfs, hal_nxp, hal_ti, picolibc).

```sh
python3 -m venv .venv && . .venv/bin/activate && pip install west
west init -m https://github.com/signal-slot/yakogl --mr main yakogl-ws
cd yakogl-ws && west update
pip install -r zephyr/scripts/requirements-base.txt
# the Zephyr SDK (1.0.1 was used): https://docs.zephyrproject.org/latest/develop/toolchains/zephyr_sdk.html
```

The repository lands in `yakogl-ws/yakogl` and is a Zephyr module by itself.
Each sample adds `driver/` (a second module: the drivers and the board
support) to the module list on its own (`common/yakogl_dir.cmake`), so a
plain `west build` needs no module arguments.

A workspace whose manifest is something else works too, as long as no other
module defines the same boards: a SafeUI checkout (slint-safe-ui-zephyr, where
the Verdin boards used to live) must not be a module of the build, or Zephyr
stops with "Board(s): {'verdin_am62p'}, defined multiple times". The
`tools/cts` scripts pass `-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh
<workspace>)`, which leaves such a checkout out; do the same by hand in that
case.

## Three first builds

`triangle` is the one triangle sample for all three boards (a nightly cargo
builds its Rust part: `RUSTUP_TOOLCHAIN=nightly`). From the top of the
workspace:

```sh
# Verdin AM62P: needs the PowerVR firmware header, generated from the board's
# own firmware file and never committed (TI/Imagination property)
yakogl/driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796
west build -p always -b verdin_am62p/am62p54/a53 -d build/triangle-am62p \
    yakogl/driver/samples/zephyr/triangle

# Verdin iMX8M Plus
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/triangle-imx8mp \
    yakogl/driver/samples/zephyr/triangle

# Raspberry Pi 4
west build -p always -b rpi_4b -d build/triangle-pi4 \
    yakogl/driver/samples/zephyr/triangle
```

How an image reaches the board: the product form is a standalone boot from
the board's own eMMC or SD card (`docs/standalone-boot.md`); for a test run,
`tools/board.sh <board> run <zephyr.bin> <logdir>` boots it once (U-Boot on
the Verdins, the firmware's tryboot on the Pi 4) and reads the result.
`driver/tools/kexec-zephyr/` is a development route only.

## What else some samples need

- **Slint samples** (`slint`, the one Slint sample on all three boards since Step 1
  of `driver/docs/one-method.md`, and `cluster`, the one agl-slint-cluster sample since
  Step 5): a no_std Slint and patched femtovg/glow (`slint/nostd/README.md`,
  `-DSLINT_NOSTD_DIR=`, `-DNOSTD_PATCH_DIR=`) and a nightly `cargo`
  (`RUSTUP_TOOLCHAIN=nightly`). The `slint` sample's layout is in `safeui/` (below).
  `cluster` also needs a clone of signal-slot/agl-slint-cluster
  (`-DAGL_SLINT_CLUSTER_DIR=`) and Noto Sans Regular, Light and ExtraLight
  (`-DNOTO_SANS_DIR=`, default `/usr/share/fonts/noto`); its host reference and
  probe tables come from `cluster/host/ref/run.sh` (an unpatched Slint at the no_std
  tree's base, outside this repository).
- **dEQP** (`deqp`, `v3d_deqp`): a stage-1 build of the CTS libraries,
  `tools/cts/cts-build-zephyr.sh` (see `deqp/README.md`).
- **glmark2** (`glmark2`, the performance measure): a glmark2 checkout at the
  pinned commit, `-DGLMARK2_SRC=` (see `driver/docs/glmark2.md`).

## safeui/

The files the Slint samples share with the SafeUI safety core (moved here from
the SafeUI repository, contents unchanged):

- `include/safeui/ui_layout.h`: where every element of the meter cluster sits,
  per panel; `scripts/gen_layout_slint.py` turns it into
  `ui/<panel>/safeui-layout.slint`, which both the rich UI (these samples) and
  the safety layer import (`--check` verifies the checked-in files);
- `include/safeui/frame_ring.h` (+ `ipc.h`, `abi.h`): the frame-ring protocol
  of route 3, where the i.MX8M Plus's M7 composites over the A53's frames (no
  sample uses it since Step 1 removed gc_slint and gc_common's ring).

`safeui_dir.cmake` finds them; `-DSAFEUI_DIR=<tree>` names another tree with
the same layout.
