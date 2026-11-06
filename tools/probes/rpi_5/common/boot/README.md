<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi_5 boot state (x0 and the entry state)

The Pi 5 firmware starts a `kernel=` arm64 image with the DTB address in x0
(Linux boot protocol). Zephyr overwrites x0 before any init hook runs:
`zephyr/arch/arm64/core/reset.S` `__reset_prep_c` (line 45, `switch_el x0, ...`)
and the lines after use x0 as a scratch, and `__start` (reset.S line 114) is
entered from the image header `b __start` (`arch/arm64/core/header.S`, line 26).

`boot_x0.S` defines `__wrap___start`. Linked with `-Wl,--wrap=__start`, every
undefined reference to `__start` (the header's branch, and `smp.c`'s
`&__start`) resolves to it. It saves x0..x3, CurrentEL, the address it runs at
and MPIDR_EL1 into a `.noinit` record (not cleared by BSS init), then branches
to `__real___start` with x0..x30, sp, DAIF and NZCV unchanged. The one thing it
does not keep is TPIDR_EL0, its scratch holder (left 0; UNKNOWN at entry).
Only the CPU with MPIDR affinity 0 writes the record.

Note: `--wrap` does not rewrite the linker script's `ENTRY(__start)`, so the ELF
entry field still names the real `__start`. The firmware loads the raw image
and jumps to its first instruction, the header branch, so the stub runs. A
tool that jumps to the ELF entry would bypass it.

## Use in a probe

In the probe's `CMakeLists.txt`, after `project(...)`:

```cmake
include(${CMAKE_CURRENT_SOURCE_DIR}/../common/boot/rpi5_boot.cmake)
```

In C:

```c
#include "rpi5_boot.h"

if (rpi5_boot_valid()) {
	const volatile struct rpi5_boot_state *s = rpi5_boot_state();
	printk("dtb=%llx el=%llx entry=%llx\n", s->x[0], s->current_el >> 2, s->entry_pc);
}
```

`x[0]` is the DTB address, `current_el` is the raw CurrentEL (EL number << 2),
`entry_pc` is where the image actually runs (answers the load address, U6).
The record is valid only if `magic` is `RPI5_BOOT_MAGIC`; RAM is not cleared
between boots, so a stale record from an earlier image is possible if the stub
did not run. The DTB must be read while the memory it sits in is still mapped
(Zephyr's rpi_5 map is 40-bit; see the pre-study note).

Do not add a probe directory for this unit: the generic rpi_5 gate row builds
every `tools/probes/rpi_5/<name>/`, and `common/` is not a probe.
