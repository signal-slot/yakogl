/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stm32mp2.h -- what a sample on the STM32MP257F-DK's A35 calls (stm32mp2.c).
 *
 * Before main() the layer has printed the early banner and the hand-over
 * record, and started the lifetime: CONFIG_YAKOGL_STM32MP2_LIFETIME_S after
 * boot the board resets through PSCI whatever the sample does. A sample
 * ends with mp2_reset_to_linux(), which resets the board back into ST's
 * Linux (the next boot is the card's default one).
 */
#ifndef YAKOGL_STM32MP2_H
#define YAKOGL_STM32MP2_H

#include <stdint.h>
#include <zephyr/toolchain.h>

/* "MP2SEEN\0": the entry code ran and filled the record. */
#define MP2_HANDOVER_SEEN  0x004e45455332504dULL
/* The record's value in the image: the entry code never ran. */
#define MP2_HANDOVER_EMPTY 0x59544d4532504dULL

/* What the image's first code found (stm32mp2_entry.S, __wrap___start),
 * before any of Zephyr's code ran. */
struct mp2_handover {
	uint64_t seen;   /* MP2_HANDOVER_SEEN once the entry code ran */
	uint64_t x[4];   /* x0-x3 at entry (booti: x0 the DTB, x1-x3 0) */
	uint64_t el;     /* CurrentEL as read (EL in bits [3:2]) */
	uint64_t sctlr;  /* SCTLR_EL2 at EL2, else SCTLR_EL1 */
	uint64_t mpidr;  /* MPIDR_EL1 */
	uint64_t cntfrq; /* CNTFRQ_EL0 */
	uint64_t pc;     /* the address the entry code ran at */
};

/* The record (valid when seen == MP2_HANDOVER_SEEN). */
const struct mp2_handover *mp2_handover_get(void);

/* Uptime in ms at which the layer resets the board through PSCI. */
uint32_t mp2_lifetime_ms(void);

/* PSCI_VERSION as BL31 answered it at boot (0x00010001 is PSCI 1.1), or the
 * negative PSCI error. */
int32_t mp2_psci_version(void);

/* Called by the watchdog (stm32mp2_wdog.c, when it is in the image) once it
 * armed OP-TEE's watchdog with timeout_s, before the report at PRE_KERNEL_1
 * 99; the report then says so. Without the call the report says no watchdog
 * is armed by the image. */
void mp2_note_wdog_armed(uint32_t timeout_s);

/* The end of a run: `why` is printed, then PSCI SYSTEM_RESET. If the firmware
 * returns, the CPU halts with interrupts off (a watchdog, when one runs, is
 * then the way out). Any context. */
FUNC_NORETURN void mp2_reset_to_linux(const char *why);

#endif /* YAKOGL_STM32MP2_H */
