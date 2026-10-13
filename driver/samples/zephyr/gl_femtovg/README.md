<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_femtovg: one FemtoVG frame through YakoGL

The Rust staticlib (`rust/`, crate `fvg_zephyr`) wraps the no_std `fvg_demo`
crate and its `fvg_draw_frame()`: a clear, a rounded rect, a gradient rect
and a stroked line, drawn by femtovg over glow through YakoGL's GL ES 2.0.

## Boards

| Board (`-b`) | GPU | C file | Board support |
|---|---|---|---|
| `verdin_am62p/am62p54/a53` | PowerVR BXS-4-64 | `src/main.c` | the `pvr_sample_*` glue (DSS plane, hold, reset) |
| `verdin_imx8mp/mimx8ml8/a53` | Vivante GC7000UL | `src/main_board.c` | the board layer, `yakogl_board.h` |

The board file picks the path: `boards/verdin_imx8mp_mimx8ml8_a53.conf` turns
`CONFIG_YAKOGL_BOARD` on, and `CMakeLists.txt` then builds `main_board.c`
instead of `main.c`. The i.MX8MP board files follow the slint sample's
(`../slint/boards/verdin_imx8mp_mimx8ml8_a53.{conf,overlay}`).

`main_board.c` draws into a 1024x600 window from `board_window()`, shows it,
reads four points back with `glReadPixels` and compares them with
`fvg_demo`'s scene. The verdict is the board layer's: the line
`gl_femtovg: PASS` or `gl_femtovg: FAIL rc=...` and the APP record
(`board_finish`).

## FVG_NOSTD_DIR

The build needs the no_std FemtoVG workspace, which is not in this
repository. Pass it with `-DFVG_NOSTD_DIR=<path>` or the environment variable
of the same name. It holds:

- `femtovg/` and `glow/`: the patched forks (`../slint/nostd/README.md`,
  "Recreating the workspaces", step 2);
- `fvg_demo/`: a copy of `../slint/nostd/fvg_demo`;
- `Cargo.toml`: a copy of `../slint/nostd/nostd-workspace.Cargo.toml`.

## Build

```sh
west build -b verdin_imx8mp/mimx8ml8/a53 -d build/gl_femtovg \
    driver/samples/zephyr/gl_femtovg -- -DFVG_NOSTD_DIR=$WORK/nostd
west build -b verdin_am62p/am62p54/a53 -d build/gl_femtovg \
    driver/samples/zephyr/gl_femtovg -- -DFVG_NOSTD_DIR=$WORK/nostd
```

On the i.MX8MP the board layer selects the WDOG1 self-arm
(`CONFIG_GC_BOARD_WDOG_SELF_ARM`), so an image started by U-Boot is bounded
without extra options. Run the image with
`tools/board.sh imx8mp run <zephyr.bin> <logdir>`.
