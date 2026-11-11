/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The STM32MP257F's GPU powered for the Vivante driver (gc_board_power_on,
 * drivers/gpu/gpu_vivante_gc_rust.c: the DK's vivante,gc node has no "gpc"
 * and "ccm" ranges), from the GPU device's init before it identifies and
 * resets the GPU. Plan #322, issues #7 and #1281.
 *
 * yd_mp2_gpu_power_on (stm32mp2_gpu.h): OP-TEE opened, SCMI PERFORMANCE
 * domain 1 and POWER domain 0, the RCC gate on and the reset pulsed in
 * RCC_GPUCFGR, HI_IDLE_STATE until idle, the session closed; every SMC,
 * OP-TEE frame and SCMI message on the console ("gc_power: ...") before it
 * is sent, so an OP-TEE panic leaves the request that caused it in the log.
 * Then yd_mp2_gpu_identify: unless the part is model 0x8000 revision 0x6205
 * (what Linux read on the board, #1281) the image refuses it with one line
 * and resets the board through PSCI. A failed power-on does the same: the
 * GPU's state is then unknown and nothing else may touch it.
 *
 * The GPU block is the driver's mapping (0x800 bytes); nothing at or above
 * offset 0x800 is read from the CPU (the crate refuses it before the bus).
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include "stm32mp2.h"
#include "stm32mp2_gpu.h"

/* The RCC (st,stm32mp2-rcc at 0x44200000; the pre-study note, section 4):
 * the board's tree has no node for it. One page, RCC_GPUCFGR at +0x83c. */
#define MP2_RCC_PHYS 0x44200000UL
#define MP2_RCC_SIZE 0x1000UL

/* What Linux read on the board (#1281). */
#define MP2_GPU_MODEL 0x8000u
#define MP2_GPU_REV   0x6205u

#define MP2_GPU_IDLE_READS 1000u

BUILD_ASSERT(DT_REG_SIZE(DT_NODELABEL(gpu)) == 0x800, "the GPU's register window is 0x800 bytes");
BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "an image that talks to OP-TEE's SCMI runs under OP-TEE's watchdog");

static uint8_t shm[YD_MP2_GPU_SHM_SIZE] __aligned(YD_MP2_GPU_SHM_ALIGN);

static void log_line(void *ctx, const char *line, size_t len)
{
	ARG_UNUSED(ctx);
	printk("gc_power: %.*s\n", (int)len, line);
}

static FUNC_NORETURN void refuse(const char *what, int32_t rc, const struct yd_mp2_gpu_err *e)
{
	printk("gc_power: REFUSED %s rc=%d kind=%u step=%u a=0x%x b=0x%x\n", what, (int)rc,
	       (unsigned int)e->kind, (unsigned int)e->step, (unsigned int)e->a,
	       (unsigned int)e->b);
	mp2_reset_to_linux("gc_power refused the GPU");
}

int gc_board_power_on(mm_reg_t regs);
int gc_board_power_on(mm_reg_t regs)
{
	struct yd_mp2_gpu_err none = {0};
	struct yd_mp2_gpu_up up;
	struct yd_mp2_gpu_id id;
	mm_reg_t rcc;
	int32_t rc;

	device_map(&rcc, MP2_RCC_PHYS, MP2_RCC_SIZE, K_MEM_CACHE_NONE);
	struct yd_mp2_gpu g = {
		.rcc_base = rcc,
		.gpu_base = regs,
		.shm = shm,
		.shm_pa = k_mem_phys_addr(shm),
		.log = log_line,
		.log_ctx = NULL,
		.idle_reads = MP2_GPU_IDLE_READS,
		.channel = 0,
	};
	printk("gc_power: gpu at 0x%lx, rcc 0x%lx at 0x%lx, shm pa 0x%llx\n", (unsigned long)regs,
	       (unsigned long)MP2_RCC_PHYS, (unsigned long)rcc, (unsigned long long)g.shm_pa);

	rc = yd_mp2_gpu_power_on(&g, &up);
	if (rc != 0) {
		refuse("power-on", rc, &up.err);
	}
	printk("gc_power: powered: level %u (found %u), rcc 0x%08x, idle 0x%08x after %u reads\n",
	       (unsigned int)up.level, (unsigned int)up.level_before, (unsigned int)up.rcc,
	       (unsigned int)up.idle, (unsigned int)up.idle_reads);

	rc = yd_mp2_gpu_identify(&g, &id);
	if (rc != 0) {
		refuse("identify", rc, &id.err);
	}
	printk("gc_power: model=0x%x rev=0x%x product=0x%x eco=0x%x customer=0x%x table=%u\n",
	       (unsigned int)id.model, (unsigned int)id.revision, (unsigned int)id.product_id,
	       (unsigned int)id.eco_id, (unsigned int)id.customer_id,
	       (unsigned int)id.from_table);
	if (id.model != MP2_GPU_MODEL || id.revision != MP2_GPU_REV) {
		printk("gc_power: the GPU is model 0x%x rev 0x%x, not Linux's model 0x%x rev 0x%x\n",
		       (unsigned int)id.model, (unsigned int)id.revision, MP2_GPU_MODEL,
		       MP2_GPU_REV);
		refuse("identity", -1, &none);
	}
	return 0;
}
