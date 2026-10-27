/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * record_main.c -- records the register sequence of the firmware start the
 * driver used before docs/source-form.md step 6: pvr_fw_start() and the
 * functions it called (rogue_slc_init, riscv_wrapper_init, rogue_axi_ace_init,
 * rogue_bif_init) of driver/src/pvr_fw_boot.c at origin/main 38438f4d, which
 * record_main.sh cuts out of that commit into old_start.inc. They run here
 * unchanged against a recording sys_read32/sys_write32/k_busy_wait
 * (hostshim/), with the inputs tools/pvr/rgxlayer_check.c gives RGXStart:
 * the page catalogue at 0xe0300000, the firmware heap at
 * RGX_FIRMWARE_RAW_HEAP_BASE, SLC_CTRL_MISC found as PSG-hazard-check | 0xf00000.
 * Output: main-38438f4d.trace on stdout, in rgxlayer_check.c's trace format.
 */
#include <stdio.h>
#include <stdint.h>

#include "pvr_internal.h"

#define REGS_SIZE 0x10000u
static uint32_t g_regs[REGS_SIZE / 4];

uint32_t sys_read32(mm_reg_t addr)
{
	printf("R 0x%04lx 0x%08x\n", (unsigned long)addr, g_regs[addr / 4]);
	return g_regs[addr / 4];
}

void sys_write32(uint32_t data, mm_reg_t addr)
{
	printf("W 0x%04lx 0x%08x\n", (unsigned long)addr, data);
	g_regs[addr / 4] = data;
}

void rec_busy_wait(uint32_t us)
{
	printf("D %u\n", us);
}

/* What the cut-out functions use from the rest of pvr_fw_boot.c. */
#define LOG_INF(...) ((void)0)
#define PT_ADDR_MASK (~RGX_MMUCTRL_PT_DATA_PAGE_CLRMSK)
static uint64_t g_pc_phys = 0xe0300000ull;
static inline uint64_t heap_gpu(uint32_t off)
{
	return PVR_FW_HEAP_GPU_BASE + off;
}

#include "old_start.inc"

int main(void)
{
	g_regs[RGX_CR_SLC_CTRL_MISC / 4] = RGX_CR_SLC_CTRL_MISC_ENABLE_PSG_HAZARD_CHECK_EN | 0x00f00000u;
	return pvr_fw_start(0);
}
