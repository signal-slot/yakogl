<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_fence

The second step of a Vivante bring-up (`docs/notes/imx8mp-gc-bringup.md`,
step 4: gc_identify, gc_fence, gc_clear, gc_triangle): one command buffer
whose fence retires through the GPU's interrupt, nothing drawn.

The GPU device's init identifies the GPU and resets it to idle
(`driver/drivers/gpu/gpu_vivante_gc_rust.c`: `yd_gc_identify`,
`yd_gc_reset`); `gc_render_init()` does the hardware init, the MMU v2 tables,
the interrupt and the front end's idle loop (`yd_gc_hw_init`, `yd_gc_mmu_*`,
`yd_gc_ring_init`, all in `driver/rust/src/ffi/vivante.rs`). The sample then
submits 64 bytes of NOPs, which the render service wraps with its prologue
and the EVENT, and waits for the fence with a bound
(`CONFIG_GC_FENCE_WAIT_MS`, 1000 ms). A retired fence proves the front end
fetched through the MMU, the EVENT reached the pixel engine, the interrupt
arrived on the right line and the ring re-armed.

It uses only the board layer (`yakogl_board.h`) and the GPU driver's own
interface (`gc.h`, `gc_render.h`), so it builds for every board of the board
layer whose devicetree has a `vivante,gc` node; the board's files are the
board layer's `app.conf` and `app.overlay` (`examples/zephyr.cmake`).

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_fence \
    yakogl/driver/samples/zephyr/gc_fence -- -DYAKOGL_WERROR=ON
# the STM32MP257F-DK's A35 (plan #322)
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/gc_fence_mp2 \
    yakogl/driver/samples/zephyr/gc_fence -- -DYAKOGL_WERROR=ON
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
gc_fence: fence 1 irqs 1 elapsed <us> us irq-latency <us> us (completed 1, wait 0)
gc_fence: PASS
yk-record: 594b02.. 00000001 00000001 01......
```

The record is kind APP: w1 = stage << 8 | status (1 pass, 2 fail; the stage
is the step that failed, 0 on a pass: 1 the GPU device, 2 gc_render_init,
3 the allocation, 4 the submit, 5 the wait, 6 a fence retired with no
interrupt counted); w2 = the fence seqno (0 when none was handed out);
w3 = the interrupts that retired a fence << 24 (saturating at 255) | the
microseconds from the submit to the wait's return (24 bits, saturating). The
interrupt count is `gc_render_perf()`'s `kicks_done`, which only the
interrupt handler moves; the irq latency is its `gpu_busy_ns` (submit to
interrupt). The elapsed time has the wait's 1 ms polling step in it.

Built for the Verdin iMX8M Plus A53 and the STM32MP257F-DK A35 with `-Werror`; not run on a board yet.
