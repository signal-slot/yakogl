<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_clear

The third step of a Vivante bring-up (`docs/notes/imx8mp-gc-bringup.md`,
step 4): fill a small linear XRGB8888 render target with a colour through the
**resolve engine** and read every pixel back on the CPU.

The bring-up is `gc_fence`'s (the GPU device's identify and reset, then
`gc_render_init()`). The clear is `gc_draw_clear()`, a resolve with a fill
value: this family has no BLT engine, and the resolve engine touches none of
the 3D state, so the picture depends on memory, the MMU, the submit path and
one fixed-function block only. If `gc_clear` passes and a triangle does not,
the fault is in the 3D pipe.

The target (`CONFIG_GC_CLEAR_WIDTH` x `CONFIG_GC_CLEAR_HEIGHT`, 256 x 128; the
width a multiple of 16, which the resolve engine needs) is poisoned with
`0x00a55a3c` and cleared **twice** with different colours, so neither "the
allocator zeroed it" nor "only the first clear landed" can pass. Every pixel
is compared after each clear, and two guard rows after the target must keep
the poison (a clear that ran past the window).

It uses only the board layer (`yakogl_board.h`) and the GPU driver's own
interface (`gc.h`, `gc_render.h`, `gc_draw.h`), so it builds for every board
of the board layer whose devicetree has a `vivante,gc` node.

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_clear \
    yakogl/driver/samples/zephyr/gc_clear -- -DYAKOGL_WERROR=ON
# the STM32MP257F-DK's A35 (plan #322)
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/gc_clear_mp2 \
    yakogl/driver/samples/zephyr/gc_clear -- -DYAKOGL_WERROR=ON
```

On the STM32MP257F-DK the board layer (`driver/zephyr/lib/board/stm32mp2`)
powers the GPU from the driver's init (`gc_power_mp2.c`: SCMI over OP-TEE,
every message on the console as `gc_power: ...`, the RCC gate and reset),
identifies it and refuses anything but model 0x8000 revision 0x6205 (#1281)
with a `gc_power: REFUSED ...` line and a PSCI reset. The GPU's memory is
the 64 MiB `gpu-reserved` area at 0xfa800000; the run ends with PSCI
SYSTEM_RESET back into ST's Linux.

The result line and the record:

```
gc_clear: target 256x128 stride 1024 at GPU 0x........
gc_clear: 256x128 wrong pixels 0 (clear 1 0, clear 2 0, guard 0)
gc_clear: PASS
yk-record: 594b02.. 00000001 00000002 01000080
```

On a pass the record is kind APP: w1 = status 1, w2 = the clears (2),
w3 = W << 16 | H. When pixels are wrong it is kind CHECK, tag CHECK_UNDRAWN,
w1 = the wrong pixels of both clears and the guard rows (the first wrong
pixel of each part is on the console). Any other failure is kind APP with
w1 = stage << 8 | status 2 (1 the GPU device, 2 gc_render_init, 3 the
allocation, 4 the clear's submit, 5 its wait, 6 the read-back), w2 = the
clears completed, w3 = W << 16 | H.

Built for the Verdin iMX8M Plus A53 and the STM32MP257F-DK A35 with `-Werror`; not run on a board yet.
The C sample of the same name ran on the Verdin iMX8M Plus on 2026-09-20
with the C kernel side (probes only); it was removed with the per-GPU
samples (5a496fd5).
