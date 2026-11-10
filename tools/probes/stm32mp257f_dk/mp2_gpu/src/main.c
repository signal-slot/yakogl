/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_gpu: the STM32MP257F-DK's GPU powered from our image and its identity
 * read (README.md; docs/notes/pre-study-stm32mp257f-dk.md section 6 item 2).
 * Before main() the board layer has printed the early banner and the
 * hand-over record (stm32mp2.c) and armed OP-TEE's watchdog
 * (stm32mp2_wdog.c). This powers the GPU through OP-TEE's SCMI (performance
 * domain 1, power domain 0) and the RCC gate and reset, every SCMI message
 * and OP-TEE frame on the console; reads the identity; writes HI_INTR_ENBL
 * and reads it back; resets the GPU through the RCC to idle; prints one
 * verdict line and resets the board back into ST's Linux through PSCI. Any
 * failure prints "verdict FAIL" and resets at once. Nothing at or above the
 * GPU's offset 0x800 is touched (the crate refuses it).
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>
#include <stm32mp2_gpu.h>

#define GPU_NODE DT_NODELABEL(gpu)
#define MP2_GPU_PHYS DT_REG_ADDR(GPU_NODE)
#define MP2_GPU_SIZE DT_REG_SIZE(GPU_NODE)
/* The RCC (st,stm32mp2-rcc at 0x44200000; the pre-study note, section 4):
 * the board's tree has no node for it. One page, RCC_GPUCFGR at +0x83c. */
#define MP2_RCC_PHYS 0x44200000UL
#define MP2_RCC_SIZE 0x1000UL

/* What Linux read on the board (#1281). */
#define MP2_GPU_MODEL 0x8000u
#define MP2_GPU_REV   0x6205u

#define MP2_GPU_INTR_VALUE 0xffffffffu
#define MP2_GPU_IDLE_READS 1000u

BUILD_ASSERT(MP2_GPU_SIZE == 0x800, "the GPU's register window is 0x800 bytes");
BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "an image that talks to OP-TEE's SCMI runs under OP-TEE's watchdog");

static uint8_t shm[YD_MP2_GPU_SHM_SIZE] __aligned(YD_MP2_GPU_SHM_ALIGN);

static void log_line(void *ctx, const char *line, size_t len)
{
	ARG_UNUSED(ctx);
	printk("mp2_gpu: %.*s\n", (int)len, line);
}

/* The crate's panic handler (lib.rs) ends here: a broken invariant. */
FUNC_NORETURN void yd_panic(const uint8_t *file, size_t file_len, uint32_t line)
{
	printk("mp2_gpu: verdict FAIL rust panic at %.*s:%u\n", (int)file_len, (const char *)file,
	       (unsigned int)line);
	mp2_reset_to_linux("mp2_gpu FAIL");
}

static FUNC_NORETURN void fail(const char *what, int32_t rc, const struct yd_mp2_gpu_err *e)
{
	printk("mp2_gpu: verdict FAIL %s rc=%d kind=%u step=%u a=0x%x b=0x%x\n", what, (int)rc,
	       (unsigned int)e->kind, (unsigned int)e->step, (unsigned int)e->a,
	       (unsigned int)e->b);
	mp2_reset_to_linux("mp2_gpu FAIL");
}

int main(void)
{
	const struct mp2_handover *h = mp2_handover_get();
	struct yd_mp2_gpu_err none = {0};
	struct yd_mp2_gpu_up up;
	struct yd_mp2_gpu_id id;
	struct yd_mp2_gpu_reset rs;
	uint32_t intr = 0;
	mm_reg_t gpu, rcc;
	int32_t rc;

	printk("mp2_gpu: build %s\n", MP2_GPU_BUILD);
	if (h->seen == MP2_HANDOVER_SEEN) {
		printk("mp2_gpu: handed over at EL%u, x0 0x%llx, sctlr 0x%llx, PSCI 0x%x\n",
		       (unsigned int)((h->el >> 2) & 3), (unsigned long long)h->x[0],
		       (unsigned long long)h->sctlr, (unsigned int)mp2_psci_version());
	} else {
		printk("mp2_gpu: no hand-over record (the entry code did not run)\n");
	}

	device_map(&gpu, MP2_GPU_PHYS, MP2_GPU_SIZE, K_MEM_CACHE_NONE);
	device_map(&rcc, MP2_RCC_PHYS, MP2_RCC_SIZE, K_MEM_CACHE_NONE);
	struct yd_mp2_gpu g = {
		.rcc_base = rcc,
		.gpu_base = gpu,
		.shm = shm,
		.shm_pa = k_mem_phys_addr(shm),
		.log = log_line,
		.log_ctx = NULL,
		.idle_reads = MP2_GPU_IDLE_READS,
		.channel = 0,
	};
	printk("mp2_gpu: gpu 0x%lx (0x%x bytes) at 0x%lx, rcc 0x%lx at 0x%lx, shm pa 0x%llx\n",
	       (unsigned long)MP2_GPU_PHYS, (unsigned int)MP2_GPU_SIZE, (unsigned long)gpu,
	       (unsigned long)MP2_RCC_PHYS, (unsigned long)rcc, (unsigned long long)g.shm_pa);

	rc = yd_mp2_gpu_power_on(&g, &up);
	if (rc != 0) {
		fail("power-on", rc, &up.err);
	}
	printk("mp2_gpu: powered: level %u (found %u), rcc 0x%08x, idle 0x%08x after %u reads\n",
	       (unsigned int)up.level, (unsigned int)up.level_before, (unsigned int)up.rcc,
	       (unsigned int)up.idle, (unsigned int)up.idle_reads);

	rc = yd_mp2_gpu_identify(&g, &id);
	if (rc != 0) {
		fail("identify", rc, &id.err);
	}
	printk("mp2_gpu: model=0x%x rev=0x%x product=0x%x eco=0x%x customer=0x%x "
	       "date=0x%08x time=0x%08x features=0x%08x table=%u streams=%u\n",
	       (unsigned int)id.model, (unsigned int)id.revision, (unsigned int)id.product_id,
	       (unsigned int)id.eco_id, (unsigned int)id.customer_id, (unsigned int)id.date,
	       (unsigned int)id.time, (unsigned int)id.features, (unsigned int)id.from_table,
	       (unsigned int)id.stream_count);
	if (id.model != MP2_GPU_MODEL || id.revision != MP2_GPU_REV) {
		printk("mp2_gpu: identity is not Linux's (model 0x%x rev 0x%x)\n", MP2_GPU_MODEL,
		       MP2_GPU_REV);
		fail("identity", -1, &none);
	}

	rc = yd_mp2_gpu_intr_check(&g, MP2_GPU_INTR_VALUE, &intr);
	if (rc != 0) {
		struct yd_mp2_gpu_err e = {YD_MP2_GPU_ERR_READBACK, 0, MP2_GPU_INTR_VALUE, intr};

		fail("intr_enbl", rc, &e);
	}

	rc = yd_mp2_gpu_reset_idle(&g, &rs);
	if (rc != 0) {
		fail("reset", rc, &rs.err);
	}
	printk("mp2_gpu: reset: rcc 0x%08x held, 0x%08x released; idle 0x%08x after %u reads\n",
	       (unsigned int)rs.rcc_held, (unsigned int)rs.rcc, (unsigned int)rs.idle,
	       (unsigned int)rs.idle_reads);

	printk("mp2_gpu: verdict PASS model=0x%x rev=0x%x product=0x%x eco=0x%x customer=0x%x "
	       "intr=ok idle=ok\n",
	       (unsigned int)id.model, (unsigned int)id.revision, (unsigned int)id.product_id,
	       (unsigned int)id.eco_id, (unsigned int)id.customer_id);
	mp2_reset_to_linux("mp2_gpu done");
}
