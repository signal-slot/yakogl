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
| Atmark Armadillo-X2, direct boot (TF-A BL33, `docs/notes/x2-direct-boot.md`) | `armadillo_x2/mimx8ml8/a53/direct` | Vivante GC7000UL |
| Raspberry Pi 4 | `rpi_4b` | Broadcom V3D 4.2 |

## The samples

Four samples here, each one source tree for every board it runs on: the Zephyr
board (`-b`) picks the GPU, and what differs per board is in the sample's
`boards/` (`<board>.conf`, `<board>.overlay`) and the board layer
(`driver/zephyr/lib/board`). The fifth, the triangle, is the first example
(`examples/triangle`, `examples/README.md`).

| Sample | What it is |
|---|---|
| `gl_test` | YakoGL's on-silicon suite (`driver/tools/pvrlab/gl_test.c`); `src/main_<board>.c` starts the board's back end |
| `deqp` | dEQP-GLES2/3 as an image |
| `glmark2` | the benchmark |
| `examples/slint` | the Slint demo (outside this directory): kept working on every board, not a test or a benchmark |

The earlier per-GPU samples (`gc_*`, `v3d_*`, `gl_triangle`, `gl_femtovg`, ...)
are gone: their boards became board configurations of these, the probes that
the gate still builds are under `tools/probes/` and the host replay is
`tools/host-replay/`.

## The samples

Five (two of them under `examples/`), each one for every board: the Zephyr board (`-b`) picks the GPU, and
the per-board files sit under the sample's `boards/` (per-GPU variants are
board configurations, not samples of their own).

| Sample | Purpose |
|---|---|
| `examples/triangle` | the triangle (outside this directory) |
| `gl_test` | the on-silicon GL test suite (`tools/pvrlab/gl_test.c`) |
| `examples/slint` | the Slint demo (outside this directory) |
| `deqp` | the dEQP conformance run |
| `glmark2` | the performance measure |

Probes are not samples: the NanoPC-T6 bring-up images are in
`tools/probes/`, and the host replay of recorded runs is `tools/host-replay/`.

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
    yakogl/examples/triangle

# Verdin iMX8M Plus
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/triangle-imx8mp \
    yakogl/examples/triangle

# Raspberry Pi 4
west build -p always -b rpi_4b -d build/triangle-pi4 \
    yakogl/examples/triangle
```

How an image reaches the board: the product form is a standalone boot from
the board's own eMMC or SD card (`docs/standalone-boot.md`); for a test run,
`tools/board.sh <board> run <zephyr.bin> <logdir>` boots it once (U-Boot on
the Verdins, the firmware's tryboot on the Pi 4) and reads the result.
`driver/tools/kexec-zephyr/` is a development route only.

## What else some samples need

- **Slint** (`slint`, the one Slint sample on all three boards since Step 1
  of `driver/docs/one-method.md`): a demo, kept working on every board but
  never a test or a benchmark. It needs a no_std Slint and patched
  femtovg/glow (`examples/nostd/README.md`, `-DSLINT_NOSTD_DIR=`,
  `-DNOSTD_PATCH_DIR=`) and a nightly `cargo` (`RUSTUP_TOOLCHAIN=nightly`).
  Its layout is in `cluster/` (below). The AGL cluster sample (`cluster`, the
  UI of the external agl-slint-cluster repository) was removed on 2026-10-19:
  it drew the same kind of cluster as `slint`.
- **dEQP** (`deqp`, on all three boards): a stage-1 build of the CTS libraries,
  `tools/cts/cts-build-zephyr.sh` (see `deqp/README.md`).
- **glmark2** (`glmark2`, the performance measure): a glmark2 checkout at the
  pinned commit, `-DGLMARK2_SRC=` (see `driver/docs/glmark2.md`).

## cluster/

The files the Slint samples share with the SafeUI safety core (moved here from
the SafeUI repository, contents unchanged):

- `include/cluster/ui_layout.h`: where every element of the meter cluster sits,
  per panel; `scripts/gen_layout_slint.py` turns it into
  `ui/<panel>/cluster-layout.slint`, which both the rich UI (these samples) and
  the safety layer import (`--check` verifies the checked-in files);
- `include/cluster/frame_ring.h` (+ `ipc.h`, `abi.h`): the frame-ring protocol
  of route 3, where the i.MX8M Plus's M7 composites over the A53's frames (no
  sample uses it).

`cluster_dir.cmake` finds them; `-DCLUSTER_DIR=<tree>` names another tree with
the same layout.
