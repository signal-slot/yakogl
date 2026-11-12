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

## The linear scan-out copy

`CONFIG_GC_TRIANGLE_LINEAR_OUT=y` (default n) adds one step after the
checks: `gc_draw_copy()` copies the drawn target with the resolve engine
into the top left of a second linear XRGB8888 buffer, the kind a display
controller scans out (the DK's LTDC, the i.MX8MP's LCDIF), and the checks
run again on the copy. Neither core has a BLT engine
(`gcFEATURE_BIT_REG_BltEngine` 0 for both rows,
`tools/vivante/testdata/hwdb-compare-st-6205.txt`), so the copy is Mesa's
`etna_try_rs_blit` for a linear source and destination without tile
status. The render target itself is linear already (`PE_LOGIC_OP`
SINGLE_BUFFER, `gc_draw.c`), so no detiling is involved.

| option | default | meaning |
|---|---|---|
| `CONFIG_GC_TRIANGLE_LINEAR_OUT_WIDTH` | 0 = the target's | buffer width, at least the target's |
| `CONFIG_GC_TRIANGLE_LINEAR_OUT_HEIGHT` | 0 = the target's | buffer height, at least the target's |
| `CONFIG_GC_TRIANGLE_LINEAR_OUT_PITCH` | 0 = width x 4 rounded up to 64 | bytes, a multiple of 64 |
| `CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR` | 0 = allocated from the GPU carveout | physical address, 64-byte aligned, imported with `gc_render_map_phys()` |

The target's height must be a multiple of 4 (Mesa's `ETNA_RS_HEIGHT_MASK`);
the build asserts it and the other constraints. The buffer is poisoned with
`0x00a55a3c` first (with two guard rows after it when it is allocated).
On the copy: the three apexes, the outside (the background outside the
triangle's box, the poison everywhere past the drawn window), the covered
count, and a pixel-by-pixel comparison of the window with the target. An
address inside the carveout is used as it stands, so it has to lie past
what the pool hands out (its top end, for example).

```sh
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/gc_triangle_mp2_linear \
    yakogl/driver/samples/zephyr/gc_triangle -- -DYAKOGL_WERROR=ON \
    -DCONFIG_GC_TRIANGLE_LINEAR_OUT=y \
    -DCONFIG_GC_TRIANGLE_LINEAR_OUT_WIDTH=1280 -DCONFIG_GC_TRIANGLE_LINEAR_OUT_HEIGHT=720
```

The stream is checked on the host by `driver/src/vivante/host/copy.sh`
(gc_draw.c compiled unchanged against a model of the resolve engine).

## Result lines

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
```

With `CONFIG_GC_TRIANGLE_LINEAR_OUT=y` the copy's lines come before the
verdict:

```
gc_triangle: linear: red apex   PASS (128,22) = f31010ff
...
gc_triangle: linear: covered     PASS 10486 pixels (8389..12583 expected)
gc_triangle: linear: copy       PASS (0 of 256x128 pixels differ from the target)
gc_triangle: linear: PASS 1280x720 pitch 5120 at GPU 0x........ phys 0x........ (0 failing checks)
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
2 blue right, 3 outside, 4 covered, 5 a check of the linear copy, 6 a pixel
of the copy that differs from the target). Any other failure is kind APP with
w1 = stage << 8 | status 2 (1 the GPU device, 2 gc_render_init, 3 the
allocation, 4 the clear's submit, 5 the triangle's submit, 6 a fence's wait,
7 the read-back, 8 the linear buffer or the copy's submit), w2 = the fences
retired, w3 = W << 16 | H. A failed step's errno is on the console.

Built for the Verdin iMX8M Plus A53 and the STM32MP257F-DK A35 with `-Werror`,
with and without `CONFIG_GC_TRIANGLE_LINEAR_OUT`; not run on a board yet.
