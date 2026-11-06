/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_boot: what the Raspberry Pi 5 firmware handed to the kernel= image,
 * saved by boot_x0.S before Zephyr's __start overwrites x0 (see README.md).
 */
#ifndef RPI5_BOOT_H_
#define RPI5_BOOT_H_

#include <stdint.h>

/* "RPI5BOOT" as a little-endian u64, written last by the stub. */
#define RPI5_BOOT_MAGIC 0x544f4f4235495052ULL

/* Field order and size are the stub's store layout (boot_x0.S): 8 x u64. */
struct rpi5_boot_state {
	uint64_t magic;      /* RPI5_BOOT_MAGIC once the stub has run */
	uint64_t x[4];       /* x0..x3 at entry; x0 is the DTB address */
	uint64_t current_el; /* CurrentEL as read: EL number << 2 */
	uint64_t entry_pc;   /* address the stub ran at (MMU off: physical) */
	uint64_t mpidr;      /* MPIDR_EL1 */
};

/* The stub's record; valid only if rpi5_boot_valid(). Not cleared by BSS init. */
const volatile struct rpi5_boot_state *rpi5_boot_state(void);

static inline int rpi5_boot_valid(void)
{
	return rpi5_boot_state()->magic == RPI5_BOOT_MAGIC;
}

#endif /* RPI5_BOOT_H_ */
