<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_triangle

Milestone 5 of `docs/notes/imx8mp-gc-bringup.md`: clear a surface, then draw one
smooth-shaded triangle into it through the 3D pipe and check the pixels.

The vertices are red (top), green (bottom left) and blue (bottom right), and
the shaders are a pass-through pair generated with `etna_shc` -- both compile
to a single `NOP`, because this hardware loads vertex attributes straight into
the shader's registers and reads the outputs back out of them. So if the
picture is wrong, the shader is not the reason: it is the state around it.

Three colours rather than one, because that separates failures that a
single-colour triangle cannot:

| what the probes show | what is wrong |
| --- | --- |
| every probe still the clear colour | nothing drew |
| the three vertex probes agree | the triangle drew but did not interpolate |
| the corner is no longer the clear colour | the geometry or the viewport |
| the dominant channel is the wrong one | the pixel format, not the render |
| the centroid is not a blend | flat shading, or one varying channel lost |

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_triangle \
    yakogl/driver/samples/zephyr/gc_triangle
# on the board, the development route (kexec from Torizon, driver/tools/kexec-zephyr/):
#   kexec_zephyr zephyr.bin 0x42000000
```

Run `gc_clear` first: this sample starts with the same clear, and if that
part fails there is nothing to say about the triangle.

Expected console lines:

```
target 256x128 stride 1024 at GPU 0x580600xx
  corner        (  1,  1) = 0x000040 (clear is 0x000040)
  near red (top)      (128, 26) = 0xdd1111    (approximately)
  near green (left)   ( 46,108) = 0x11dd11
  near blue (right)   (210,108) = 0x1111dd
  centroid              = 0x555555
gc_triangle: PASS
```

The exact channel values depend on where the rasteriser puts the sample
point; the checks are "the matching channel is at least 0x80 and above the
other two", and for the centroid "all three are between 0x20 and 0xe0".

On a failure the sample prints `gc_triangle: FAIL <reason>` and the stall
dump before resetting the SoC.

Ran on the Verdin iMX8M Plus on 2026-09-20 with the C kernel side: PASS
(`docs/notes/imx8mp-gc-driver-notes.md`; the milestones are
`docs/notes/imx8mp-gc-bringup.md`). Not run since the kernel
side became Rust (`driver/docs/vivante-kernel-rust.md`).
