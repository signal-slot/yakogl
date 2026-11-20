<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Examples

Small programs that draw with YakoGL. Each one is plain OpenGL ES and EGL;
the operating system is a build detail (today Zephyr, through
`zephyr.cmake`), and no example has a file of its own for a board.

| Example | What it draws |
|---|---|
| `triangle/` | one Gouraud triangle: the GL calls in C, the scene and its check in a Rust crate (`check/`) the host tests run too |
| `triangle-rust/` | the same triangle with every call (EGL, GL, the board layer) made from Rust; the scene and the check are `triangle/check`'s, so both leave the same PICTURE record |
| `slint/` | a Slint UI (`ui/cluster.slint`, a meter cluster) rendered through YakoGL and animated; its check reads the first frame's probes |
| `femtovg/` | one frame drawn with femtovg (the 2D renderer Slint uses): a rounded rect, a gradient and a stroked line; its check reads every pixel back |

The three boards are the Verdin AM62P (PowerVR), the Verdin iMX8M Plus
(GC7000) and the Raspberry Pi 4 (V3D). Build one for a board with west, from
a west workspace that has this repository as its manifest
(`docs/es3/zephyr-env.md`):

```sh
west build -p always -b verdin_am62p/am62p54/a53 yakogl/examples/triangle
west build -p always -b verdin_imx8mp/mimx8ml8/a53 yakogl/examples/triangle
west build -p always -b rpi_4b yakogl/examples/triangle
```

The other examples build the same way, with their own directory in place of
`triangle`.

The NanoPC-T6 (Mali; work in progress, the top `README.md`) builds
`triangle` with `-b nanopc_t6`. Its image embeds the board's GPU firmware:
make `driver/src/fw/mali_csffw_blob.h` from the board's `mali_csffw.bin`
with `driver/tools/mkmalifw.sh` first. The image is not yet run on the
board (`driver/docs/nanopc-t6-display.md`, step D3).

`triangle` also builds for the Toradex Verdin iMX95 (Mali-G310),
`-b verdin_imx95/mimx9596/a55`, with the Mali CSF firmware header
(`driver/src/fw/mali_csffw_blob.h`, the NanoPC-T6's) in place, and nothing
more: the GPU's bring-up behind `eglInitialize` (the power-on over SCMI,
then the firmware) and the Toradex 10.1" DSI display on the Mallow's X4
behind `board_show()` are in the image; the board has no watchdog yet, and
nothing has run on one (`driver/zephyr/lib/board/verdin_imx95`,
`docs/standalone-boot.md`).

`slint` also needs the no_std Slint tree and the patched femtovg and glow,
which live outside this repository: `nostd/README.md` says how to make them,
and the build takes them as `-DSLINT_NOSTD_DIR=<slint tree>
-DNOSTD_PATCH_DIR=<femtovg and glow>`. `femtovg` needs the patched femtovg
and glow only: `-DNOSTD_PATCH_DIR=<femtovg and glow>`.

The Verdin AM62P image needs the PowerVR firmware header first, made from
the board's own firmware file and never committed:
`yakogl/driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796`.

What a board needs (its devicetree overlay and Kconfig fragment: the display
path, the GPU memory, the watchdog) is in the board layer,
`driver/zephyr/lib/board/<board>/app.overlay` and `app.conf`, the same for
every example; `zephyr.cmake` applies them for the board `-b` names.

The image boots from the board's own boot loader: U-Boot on the Verdins,
the firmware on the Pi 4 (`docs/standalone-boot.md`). An example draws its
picture and leaves the board layer's PICTURE record, which
`tools/board.sh <board> run <zephyr.bin> <logdir>` reads.
