<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Probes

Zephyr images that measure one thing on one board and are not samples
(`driver/samples/zephyr` holds `gl_test`, `deqp`, `glmark2` and the Vivante
bring-up samples `gc_fence` and `gc_clear`; `triangle` and `slint` are under
`examples/`). `tools/check.sh` builds each with `-DYAKOGL_WERROR=ON` (the
`zephyr` rows, by path: `zephyr_werror_image`).

- `nanopc_t6/t6_hello`: milestone Z1 on the NanoPC-T6, the build line and the
  watchdog arming (`driver/docs/nanopc-t6-mali.md`).
- `nanopc_t6/t6_gpu`: the second half of Z1, the GPU's power at hand-over.
- `nanopc_t6/t6_mmu`: milestone Z2, AS0's page tables from the image and a
  GPU cache flush round trip, after Z1's power-on.
- `stm32mp257f_dk/mp2_identify`: the STM32MP257F-DK's return-trial image, the
  hand-over record and OP-TEE's watchdog armed at boot, then PSCI reset.

Build (from the west workspace):

```sh
west build -p always -b nanopc_t6 -d build/t6_hello yakogl/tools/probes/nanopc_t6/t6_hello
```
