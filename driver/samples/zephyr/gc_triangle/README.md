<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_triangle

The fourth step of a Vivante bring-up (`docs/notes/imx8mp-gc-bringup.md`,
step 4: `gc_identify`, `gc_fence`, `gc_clear`, `gc_triangle`): clear a small
linear XRGB8888 render target to a known background, draw **one Gouraud
triangle** with a red, a green and a blue vertex, wait for the fence and read
the picture back on the CPU.

The bring-up is `gc_clear`'s (the GPU device's identify and reset, then
`gc_render_init()`, then the target). The background is `gc_draw_clear()` (the
resolve engine, which `gc_clear` has proved), the triangle is
`gc_draw_triangle()` (the 3D pipe with the pass-through shaders). If
`gc_clear` passes and `gc_triangle` does not, the fault is in the 3D pipe.
Only the CPU reads pixels; the GPU's 0x800+ state space is never read
(`driver/docs/vivante-kernel-rust.md`, "Hardware findings that still hold").

The target (`CONFIG_GC_TRIANGLE_WIDTH` x `CONFIG_GC_TRIANGLE_HEIGHT`, 256 x
128; the width a multiple of 16) is cleared to `0x00102030`; two guard rows
after it are poisoned with `0x00a55a3c`. The triangle is red at the top
(clip 0, 0.8), green bottom left (-0.8, -0.8), blue bottom right (0.8, -0.8).

The check is the i.MX8MP's three-apex read-back
(`docs/notes/imx8mp-gc-bringup.md`, the `gl_triangle_user` lines):

| check | passes when |
|---|---|
| red apex | the pixel 10% of the way from the red vertex to the centroid has red >= 0xc0 and green, blue <= 0x40 |
| green left | the same for the green vertex and green |
| blue right | the same for the blue vertex and blue |
| outside | every pixel outside the triangle's bounding box (3 pixels of margin) is still the background, and the guard rows still hold the poison |
| covered | the pixels that differ from the background are within 20% of the triangle's area |

It uses only the board layer (`yakogl_board.h`) and the GPU driver's own
interface (`gc.h`, `gc_render.h`, `gc_draw.h`), so it builds for every board
of the board layer whose devicetree has a `vivante,gc` node.

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_triangle \
    yakogl/driver/samples/zephyr/gc_triangle -- -DYAKOGL_WERROR=ON
# the STM32MP257F-DK's A35 (plan #322)
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/gc_triangle_mp2 \
    yakogl/driver/samples/zephyr/gc_triangle -- -DYAKOGL_WERROR=ON
```

On the STM32MP257F-DK the board layer (`driver/zephyr/lib/board/stm32mp2`)
powers the GPU from the driver's init (`gc_power_mp2.c`, as for `gc_fence`),
refuses anything but model 0x8000 revision 0x6205 with a `gc_power: REFUSED
...` line and a PSCI reset, and the run ends with PSCI SYSTEM_RESET back into
ST's Linux.

The result lines (one per check, the pixel as `rrggbbaa`, then the verdict):

```
gc_triangle: GPU model 0x... revision 0x... product 0x... customer 0x... eco N
gc_triangle: target 256x128 stride 1024 at GPU 0x........
gc_triangle: red apex   PASS (128,22) = f31010ff
gc_triangle: green left PASS (51,105) = 10f010ff
gc_triangle: blue right PASS (205,105) = 1010f0ff
gc_triangle: outside     PASS (background kept outside the box and in the guard rows)
gc_triangle: covered     PASS 10486 pixels (8389..12583 expected)
gc_triangle: PASS (0 failing checks)
yk-record: ...
```

(The pixel values above only show the shape; they are not a measurement.) A
failing check prints `FAIL` in its line, then `gc_triangle: FAIL (N failing
checks)`.

The record, in `gc_clear`'s convention. On a pass it is kind APP: w1 =
status 1, w2 = the covered pixels, w3 = W << 16 | H. When a check fails it is
kind CHECK, tag CHECK_UNDRAWN: w1 = the failing checks, w2 = the covered
pixels, w3 = a bit mask of the failing checks (bit 0 red apex, 1 green left,
2 blue right, 3 outside, 4 covered). Any other failure is kind APP with
w1 = stage << 8 | status 2 (1 the GPU device, 2 gc_render_init, 3 the
allocation, 4 the clear's submit, 5 the triangle's submit, 6 a fence's wait,
7 the read-back), w2 = the fences retired, w3 = W << 16 | H. A failed step's
errno is on the console.

Built for the Verdin iMX8M Plus A53 and the STM32MP257F-DK A35 with `-Werror`; not run on a board yet.
