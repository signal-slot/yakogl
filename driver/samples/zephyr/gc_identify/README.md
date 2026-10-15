<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_identify

The first Zephyr program for the i.MX8MP GPU: it runs the Vivante driver's
milestone 1 (power the GPU domains through the GPC, clock the core through the
CCM, read the identity registers, reset to idle), prints the result, services
the watchdog Linux left running for a bounded time and resets the SoC.

```sh
# in a west workspace made from yakogl's west.yml (see ../README.md)
west build -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_identify \
    yakogl/driver/samples/zephyr/gc_identify
# on the board, the development route (kexec from Torizon, driver/tools/kexec-zephyr/):
#   kexec_zephyr zephyr.bin 0x42000000
```

Expected console lines: `GC7000 rev 0x6204`, `reset to idle`, `gc_identify: PASS`.
Ran on the Verdin iMX8M Plus on 2026-09-20 with the C kernel side: PASS
(`docs/notes/imx8mp-gc-driver-notes.md`; the milestones are
`docs/notes/imx8mp-gc-bringup.md`). Not run since the kernel
side became Rust (`driver/docs/vivante-kernel-rust.md`).
