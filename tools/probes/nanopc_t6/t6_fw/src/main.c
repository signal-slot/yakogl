/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_fw: Z3 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "The firmware
 * (Z3)"). The image embeds the CSF firmware (driver/src/fw/mali_csffw_blob.h,
 * made by driver/tools/mkmalifw.sh, never committed) and backs each of the
 * seven sections it loads with static storage, in the order of
 * fw::Image::sections() (board::nanopc_t6_sections::Backing { pa, cpu, size }
 * per section). The shared-interface section's backing is __nocache: the
 * CPU reaches it through an uncached mapping, as the GPU does. Before main()
 * the board glue has armed WDT0 (nanopc_t6.c). This runs Z1's power-on
 * (yd_t6_gpu_up) and Z2's AS0 (yd_t6_mmu_up) as t6_mmu does; booting the MCU
 * is a later step.
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/cache.h>
#include <zephyr/linker/section_tags.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/kernel/mm.h>

#include <stddef.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#include "fw/mali_csffw_blob.h"

#define CRU_PHYS  0xfd7c0000UL
#define PMU_PHYS  0xfd8d8000UL
#define SPI2_PHYS 0xfeb20000UL
#define QOS_PHYS  0xfdf35000UL
#define GPU_PHYS  0xfb000000UL

/* AS0's tables: nine pages for the firmware's sections (the design page,
 * "Z2"), from a pool of sixteen. */
#define POOL_PAGES 16
static uint8_t t6_fw_pool[POOL_PAGES * 4096] __aligned(4096);

/*
 * What the seven sections map to (the board's mali_csffw.bin, 896 KiB; the
 * protected-mode section is skipped by the parser). MCU addresses and sizes:
 * 0x00000000, 0x00400000 and 0x00410000 64 KiB each, read-only data;
 * 0x00800000 128 KiB, the code; 0x01000000 and 0x02000000 256 KiB each, zeroed
 * data; 0x04000000 64 KiB, the shared interface. 4 KiB aligned, no overlap.
 */
static uint8_t t6_fw_sec0[64 * 1024] __aligned(4096);
static uint8_t t6_fw_sec1[64 * 1024] __aligned(4096);
static uint8_t t6_fw_sec2[64 * 1024] __aligned(4096);
static uint8_t t6_fw_sec3[128 * 1024] __aligned(4096);
static uint8_t t6_fw_sec4[256 * 1024] __aligned(4096);
static uint8_t t6_fw_sec5[256 * 1024] __aligned(4096);
/* The shared section: uncached for the CPU too (CONFIG_NOCACHE_MEMORY). */
static __nocache uint8_t t6_fw_shared[64 * 1024] __aligned(4096);

struct t6_fw_section {
	uint32_t va;
	uint8_t *cpu;
	size_t size;
};

static const struct t6_fw_section t6_fw_sections[] = {
	{ 0x00000000, t6_fw_sec0, sizeof(t6_fw_sec0) },
	{ 0x00400000, t6_fw_sec1, sizeof(t6_fw_sec1) },
	{ 0x00410000, t6_fw_sec2, sizeof(t6_fw_sec2) },
	{ 0x00800000, t6_fw_sec3, sizeof(t6_fw_sec3) },
	{ 0x01000000, t6_fw_sec4, sizeof(t6_fw_sec4) },
	{ 0x02000000, t6_fw_sec5, sizeof(t6_fw_sec5) },
	{ 0x04000000, t6_fw_shared, sizeof(t6_fw_shared) },
};
#define SECTIONS ARRAY_SIZE(t6_fw_sections)
BUILD_ASSERT(SECTIONS == 7, "the board's firmware has seven loaded sections");

/* driver/rust/src/ffi/rk3588.rs pins the same layouts. */
BUILD_ASSERT(sizeof(struct yd_t6_gpu_up) == 4 * 108, "struct yd_t6_gpu_up moved");
BUILD_ASSERT(offsetof(struct yd_t6_gpu_up, ident) == 4 * 61, "struct yd_t6_gpu_up moved");
BUILD_ASSERT(offsetof(struct yd_t6_gpu_up, stop) == 4 * 104, "struct yd_t6_gpu_up moved");

static const char *const stop_name[] = {
	[YD_T6_MMU_STOP_NONE] = "none",
	[YD_T6_MMU_STOP_FEATURES] = "MMU_FEATURES is not 48 VA / 40 PA bits",
	[YD_T6_MMU_STOP_FLUSH] = "the cache flush round trip did not complete",
	[YD_T6_MMU_STOP_TABLES] = "the page tables could not be built",
	[YD_T6_MMU_STOP_WALK] = "the CPU walk of VA 0x04001234 misses the backing",
	[YD_T6_MMU_STOP_CLEAN] = "the cache clean of the tables failed",
	[YD_T6_MMU_STOP_ENABLE] = "the AS0 enable sequence stopped",
	[YD_T6_MMU_STOP_READ_BACK] = "AS0 reads back other than written",
	[YD_T6_MMU_STOP_FAULT] = "AS0 is active or faulted after the update",
	[YD_T6_MMU_STOP_FLUSH_PT] = "the flush after the map stopped",
};

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static int clean(uintptr_t addr, size_t len)
{
	/* Zephyr's "flush" is a clean to the point of coherency (dc cvac). */
	return sys_cache_data_flush_range((void *)addr, len);
}

static void on_handover(const struct yd_t6_gpu_up *h)
{
	ARG_UNUSED(h);
	t6_watchdog_feed();
}

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;
	struct yd_t6_gpu_up z1;
	struct yd_t6_mmu_up o;
	uint64_t tables_pa, backing_pa;
	int64_t t0;
	int rc;

	printk("t6_fw: build %s\n", T6_FW_BUILD);
	printk("t6_fw: WDT0 period %u ms, fed until %u ms of uptime\n", t6_watchdog_period_ms(),
	       t6_watchdog_deadline_ms());
	/* The embedded image: its size, digest and magic (little endian). */
	printk("t6_fw: image %u bytes, %u loaded sections, sha256 %s, magic 0x%08x\n",
	       (unsigned int)MALI_CSFFW_SIZE, (unsigned int)SECTIONS, MALI_CSFFW_SHA256,
	       (unsigned int)(mali_csffw_blob[0] | mali_csffw_blob[1] << 8 |
			      mali_csffw_blob[2] << 16 | (uint32_t)mali_csffw_blob[3] << 24));
	for (size_t i = 0; i < SECTIONS; i++) {
		printk("t6_fw: section %u: VA 0x%08x, %u KiB, backing %p at PA 0x%llx\n",
		       (unsigned int)i, t6_fw_sections[i].va,
		       (unsigned int)(t6_fw_sections[i].size / 1024), (void *)t6_fw_sections[i].cpu,
		       (unsigned long long)k_mem_phys_addr(t6_fw_sections[i].cpu));
	}
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	/* GPU control at 0x0, the MMU block at 0x2000-0x25ff */
	device_map(&gpu, GPU_PHYS, 0x4000, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	rc = yd_t6_gpu_up(cru, pmu, spi2, qos, gpu, delay_us, on_handover, &z1);
	t6_watchdog_feed();
	printk("t6_fw: Z1: rc %d stop %u refusal %u: GPU_ID 0x%08x MMU_FEATURES 0x%08x "
	       "AS_PRESENT 0x%08x L2_READY 0x%x\n",
	       rc, z1.stop, z1.refusal, z1.ident[0], z1.ident[4], z1.ident[5], z1.l2_ready[0]);
	if (rc != 0) {
		printk("t6_fw: FAIL: Z1's power-on stopped (stop %u, refusal %u, detail "
		       "0x%08x%08x); run t6_gpu for its lines\n",
		       z1.stop, z1.refusal, z1.detail[1], z1.detail[0]);
		t6_reset_to_linux("t6_fw done");
	}

	tables_pa = k_mem_phys_addr(t6_fw_pool);
	/* Z2's single-section map: the shared section's backing. */
	backing_pa = k_mem_phys_addr(t6_fw_shared);
	printk("t6_fw: pool %p at PA 0x%llx (%u pages)\n", (void *)t6_fw_pool,
	       (unsigned long long)tables_pa, POOL_PAGES);

	t0 = k_uptime_get();
	rc = yd_t6_mmu_up(gpu, z1.ident[4], t6_fw_pool, POOL_PAGES, tables_pa, backing_pa, clean,
			  delay_us, &o);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_fw: FAIL: yd_t6_mmu_up refused its arguments\n");
		t6_reset_to_linux("t6_fw done");
	}
	if (o.stop == YD_T6_MMU_STOP_NONE) {
		printk("t6_fw: Z2: AS0 on TRANSTAB 0x%08x%08x, %u of %u table pages, %lld ms\n",
		       o.after[1], o.after[0], o.tables_used, POOL_PAGES,
		       (long long)(k_uptime_get() - t0));
	} else {
		printk("t6_fw: FAIL: Z2 stop %u (%s), refusal %u, detail 0x%08x%08x\n", o.stop,
		       o.stop < ARRAY_SIZE(stop_name) ? stop_name[o.stop] : "unknown", o.refusal,
		       o.detail[1], o.detail[0]);
		t6_reset_to_linux("t6_fw done");
	}

	/* yd_t6_fw_up (the MCU boot over the sections above) is called here. */

	t6_reset_to_linux("t6_fw done");
}
