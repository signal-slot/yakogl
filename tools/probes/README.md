<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Probes

Zephyr images that measure one thing on one board and are not samples
(`driver/samples/zephyr` holds only `triangle`, `gl_test`, `slint`, `deqp` and
`glmark2`). `tools/check.sh` builds each with `-DYAKOGL_WERROR=ON` (the
`zephyr` rows, by path: `zephyr_werror_image`).

- `nanopc_t6/t6_hello`: milestone Z1 on the NanoPC-T6, the build line and the
  watchdog arming (`driver/docs/nanopc-t6-mali.md`).
- `nanopc_t6/t6_gpu`: the second half of Z1, the GPU's power at hand-over.
- `nanopc_t6/t6_mmu`: milestone Z2, AS0's page tables from the image and a
  GPU cache flush round trip, after Z1's power-on.
- `rpi_5/rpi5_hello`: item H9 of `docs/notes/pre-study-rpi5.md`, the Raspberry
  Pi 5 image with no GPU code — built, not run (the board is not in the lab).

Build (from the west workspace):

```sh
west build -p always -b nanopc_t6 -d build/t6_hello yakogl/tools/probes/nanopc_t6/t6_hello
```
