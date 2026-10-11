<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_fence

Milestones 2 and 3 of `tools/cts/log/board-sessions/imx8mp-gc-bringup.md` on the real GPU, without
drawing anything: hardware init, MMU v2, the interrupt, the front end on its
idle loop, and then a handful of empty submits with a fence wait after each.

It is the smallest program that proves the whole submit path:

- the front end fetches from a **GPU virtual** address, so the MMU translates
  (the page-table array, the secure-AHB indirection and the identity map);
- the `EVENT` command reaches the pixel engine, so the ring's prologue and
  epilogue are well formed;
- the interrupt arrives with the right bit, so the event mechanism and the
  GIC line in the device tree are right;
- the ring re-arms round after round, so overwriting the idle loop's `WAIT`
  word with a `LINK` is safe under a running front end.

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_fence \
    yakogl/driver/samples/zephyr/gc_fence
# on the board, the development route (kexec from Torizon, driver/tools/kexec-zephyr/):
#   kexec_zephyr zephyr.bin 0x42000000
```

Expected console lines:

```
mmu: on (PTA ... entry 0 -> MTLB ..., safe page ...)
fe: running the idle loop at 0x580440xx (WAIT 24414 cycles)
render service up: 30 events, pool ... KiB free
round 0: fence 1 done (completed 1)
...
gc_fence: PASS
```

On a failure the sample prints `gc_fence: FAIL <reason>` and then the stall
dump (front-end DMA address and the ring words around it, the idle state, the
MMU status, the events still in flight) before resetting the SoC.

`CONFIG_GC_FENCE_ROUNDS` sets how many submits; more than one is the point.

Ran on the Verdin iMX8M Plus on 2026-09-20 with the C kernel side: PASS
(`tools/cts/log/board-sessions/imx8mp-gc-driver-notes.md`; the milestones are
`tools/cts/log/board-sessions/imx8mp-gc-bringup.md`). Not run since the kernel
side became Rust (`driver/docs/vivante-kernel-rust.md`).
