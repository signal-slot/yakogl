<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_gl_triangle

The i.MX8MP sibling of `gl_triangle`: the same Gouraud triangle, drawn through
the public OpenGL ES 2.0 API on the Vivante GC7000UL. The one triangle sample
for all three boards is `../triangle`; this one stays until Step 3 of
`driver/docs/one-method.md` removes it.

`gc_triangle` proves the driver can drive the 3D pipe with its own command
builders. This proves the stack above it:

```
  EGL + GL ES 2.0  (src/gl/)
  Vivante backend  (src/vivante/)                  -> a GC7000 command stream
  Zephyr OSAL      (src/osal/zephyr_gc.c)
  render service   (driver/src/vivante/gc_render.c) -> the ring, the event, the fence
```

So a failure here while `gc_triangle` passes is a failure in that stack, not in
the hardware bring-up.

The window is a linear XRGB8888 buffer in the GPU carveout, panel-sized by
default (1280x800, `prj.conf`), and it is shown on the panel through the
display driver (`driver/src/display/imx8mp_display.c`). The check is
`glReadPixels`. The vertices are red (top), green (bottom left) and blue
(bottom right), which separates the failures a single-colour triangle cannot;
`gc_triangle`'s README has the table.

## Build and run

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_gl_triangle \
    yakogl/driver/samples/zephyr/gc_gl_triangle -- \
    -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
yakogl/tools/board.sh imx8mp run build/gc_gl_triangle/zephyr/zephyr.bin <logdir>
```

The board has no UART: the runner reads the result from the SNVS record
(`yakogl-tools record`). The panel console shows the sample's lines.

- `prj_parity.conf` (`-DEXTRA_CONF_FILE=prj_parity.conf`) builds the 1024x600
  window of the AM62P's `gl_triangle`, so the PICTURE record compares across
  boards. Its baseline is `594b040e ff403f80 35706c4e 04000258`, the same from
  a U-Boot entry and from kexec
  (`tools/cts/log/board-sessions/imx8mp-netboot.md`, step C).
- `CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD` writes a DISPLAY record (pixel clock,
  lane rate, the bridge's IRQ_STAT) about a second into the picture.

The probe rows differ from `gc_triangle`'s because `glReadPixels` takes GL
window coordinates, whose y grows upwards; the GL core flips them for the
top-down window surface.

The image services WDOG1 for `CONFIG_GC_SAMPLE_HOLD_SECONDS` and then resets
the SoC through PSCI, on the failure paths as well as on success.
