# gc_gl_triangle

The i.MX8MP sibling of `gl_triangle`: the same Gouraud triangle, drawn through
the public OpenGL ES 2.0 API on the Vivante GC7000UL.

`gc_triangle` proves the driver can drive the 3D pipe with its own command
builders. This proves the stack above it:

```
  EGL + GL ES 2.0  (yakogl/src/gl_*.c)
  Vivante backend  (yakogl/src/vivante/)     -> a GC7000 command stream
  Zephyr OSAL      (yakogl/src/osal/zephyr_gc.c)
  render service   (src/vivante/gc_render.c) -> the ring, the event, the fence
```

So a failure here while `gc_triangle` passes is a failure in that stack, not in
the hardware bring-up. Run `gc_triangle` first.

There is no display driver on this target yet, so the window is a linear
XRGB8888 buffer in the GPU carveout and the check is `glReadPixels`. The
vertices are red (top), green (bottom left) and blue (bottom right), which
separates the failures a single-colour triangle cannot -- `gc_triangle`'s README
has the table.

## Build and run

YakoGL is the `yakogl/` submodule of this repository. When it is not checked
out -- in a git worktree, for instance -- name the tree with `YAKOGL_DIR`:

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_gl_triangle \
    yakogl/driver/samples/zephyr/gc_gl_triangle
# on the board (Torizon): kexec_zephyr zephyr.bin 0x42000000
```

Expected console lines:

```
gc_gl_triangle: 256x128 window, OpenGL ES 2.0 through YakoGL
window 256x128 stride 1024 at GPU 0x580600xx
  corner        (  1,  1) = 000040 (clear is 000040)
  near red (top)      (128,101) = dd1111      (approximately)
  near green (left)   ( 46, 19) = 11dd11
  near blue (right)   (210, 19) = 1111dd
  centroid              = 4a4a4a
gc_gl_triangle: PASS
```

The probe rows differ from `gc_triangle`'s because `glReadPixels` takes GL
window coordinates, whose y grows upwards; the GL core flips them for the
top-down window surface.

Like every sample here, it services the watchdog Linux left armed for
`CONFIG_GC_SAMPLE_HOLD_SECONDS` and then resets the SoC through PSCI, on the
failure paths as well as on success.

## Not yet run on silicon

Compile-tested only, like the rest of the i.MX8MP work. Two things this sample
is the natural place to settle, both from `VIVANTE_NOTES.md`:

- **Question 1, the channel order of a colour target read back by the CPU.**
  The pixel engine writes B,G,R,A, which is exactly XRGB8888, and the GL core
  swaps for that format in `glReadPixels`. If the three vertex probes each
  report the wrong dominant channel, that is the pixel format and not the
  render.
- **Question 7, how much flushing a draw really needs.** A picture that is
  right but stale shows up here as probes holding the 0x7f7f7f pattern the CPU
  wrote before the frame.
