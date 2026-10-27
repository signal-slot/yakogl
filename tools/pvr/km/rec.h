/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * rec.h -- the recording platform of the PowerVR kernel-side checks
 * (tools/pvr/km): a carveout in host memory and a register file, and the
 * record both recorders write. record_c.c (the C the driver had before
 * docs/source-form.md step 7, cut out of origin/main 674ea1cf) and
 * km_check.c (the Rust modules of driver/rust/src/gpu/pvr) print the same
 * lines for the same events, so their records can be compared with cmp.
 *
 * Record lines (offsets are carveout offsets, registers bank offsets):
 *   F <off> <len>     clean (sys_cache_data_flush_range)
 *   I <off> <len>     invalidate (sys_cache_data_invd_range)
 *   B                 full barrier (barrier_dmem_fence_full)
 *   D <us>            busy wait
 *   W <reg> <val>     32-bit register write;  R <reg> <val>  read
 *   L<level> <text>   a log line (1 ERR .. 4 DBG)
 *   = ...             a result the scenario prints
 *   M <off> <val>     at the end: every non-zero 32-bit word of the carveout
 *
 * Everything here is static inline: each recorder is one translation unit.
 */
#ifndef PVR_KM_REC_H
#define PVR_KM_REC_H

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The carveout both sides run on: the host build's PVR_GPU_MEM_PHYS and
 * PVR_GPU_MEM_MIN_SIZE (driver/src/pvr_internal.h). */
#define REC_PHYS 0xC0000000ull
#define REC_SIZE (64ull * 1024 * 1024)
#define REC_REGS_SIZE 0x10000u

static uint8_t *rec_cpu;
static uint32_t rec_regs[REC_REGS_SIZE / 4];

static inline void rec_init(void)
{
	void *p = NULL;

	if (posix_memalign(&p, 4096, REC_SIZE) != 0) {
		printf("FAIL no memory for the carveout\n");
		exit(2);
	}
	memset(p, 0, REC_SIZE);
	rec_cpu = p;
}

static inline unsigned long long rec_off(const void *p)
{
	const uint8_t *b = p;

	if (b < rec_cpu || b >= rec_cpu + REC_SIZE) {
		printf("FAIL address outside the carveout\n");
		exit(2);
	}
	return (unsigned long long)(b - rec_cpu);
}

static inline void rec_flush(const void *p, size_t len)
{
	printf("F 0x%llx %zu\n", rec_off(p), len);
}

static inline void rec_invd(const void *p, size_t len)
{
	printf("I 0x%llx %zu\n", rec_off(p), len);
}

static inline void rec_fence(void)
{
	printf("B\n");
}

/* The firmware's side of a busy wait: the scenario may set this to act as
 * the firmware would while the host waits (consume the KCCB, say). */
static void (*rec_tick)(void);

static inline void rec_busy_wait_us(uint32_t us)
{
	printf("D %u\n", us);
	if (rec_tick) {
		rec_tick();
	}
}

static inline uint32_t rec_read32(uintptr_t reg)
{
	if (reg >= REC_REGS_SIZE || reg % 4) {
		printf("FAIL register read outside the bank: 0x%lx\n", (unsigned long)reg);
		exit(2);
	}
	printf("R 0x%04lx 0x%08x\n", (unsigned long)reg, rec_regs[reg / 4]);
	return rec_regs[reg / 4];
}

/* The firmware's side of a register write (a doorbell, say). */
static void (*rec_wtick)(uintptr_t reg, uint32_t val);

static inline void rec_write32(uintptr_t reg, uint32_t val)
{
	if (reg >= REC_REGS_SIZE || reg % 4) {
		printf("FAIL register write outside the bank: 0x%lx\n", (unsigned long)reg);
		exit(2);
	}
	printf("W 0x%04lx 0x%08x\n", (unsigned long)reg, val);
	rec_regs[reg / 4] = val;
	if (rec_wtick) {
		rec_wtick(reg, val);
	}
}

/* The final memory record. */
static inline void rec_dump_memory(void)
{
	const uint32_t *w = (const uint32_t *)rec_cpu;

	for (size_t i = 0; i < REC_SIZE / 4; i++) {
		if (w[i]) {
			printf("M 0x%zx 0x%08x\n", i * 4, w[i]);
		}
	}
}

#endif /* PVR_KM_REC_H */
