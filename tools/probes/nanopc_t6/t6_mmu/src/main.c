/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_mmu: Z2 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Z2: AS0 and
 * a cache flush"). Before main() the board glue has armed WDT0
 * (nanopc_t6.c). This runs Z1's power-on (yd_t6_gpu_up, as t6_gpu) and
 * prints its outcome in one line; then yd_t6_mmu_up (board::nanopc_t6_mmu
 * in the drivers crate): AS0 and the interrupt state as found, one cache
 * flush round trip, AS0's tables built in this image's .bss with the MCU's
 * shared-interface section (VA 0x04000000, 64 KiB) mapped to a buffer of
 * this image, a CPU walk of VA 0x04001234, the tables cleaned to the point
 * of coherency, AS0 enabled as panthor enables the MCU's VM, AS0 read back,
 * and the flush panthor does after a map. The last line is PASS or FAIL
 * with the step that stopped; the board then resets back into Armbian.
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/cache.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/kernel/mm.h>

#include <stddef.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#define CRU_PHYS  0xfd7c0000UL
#define PMU_PHYS  0xfd8d8000UL
#define SPI2_PHYS 0xfeb20000UL
#define QOS_PHYS  0xfdf35000UL
#define GPU_PHYS  0xfb000000UL

/* AS0's tables: Z2 uses four pages (one table a level); Z3's firmware
 * sections need nine (the design page, "Z2"), so the pool has sixteen. */
#define POOL_PAGES 16
static uint8_t t6_mmu_pool[POOL_PAGES * 4096] __aligned(4096);
/* What the MCU's shared-interface section maps to (not read or written by
 * the GPU at Z2: nothing walks AS0 before the MCU runs, Z3). */
static uint8_t t6_mmu_backing[64 * 1024] __aligned(4096);

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

static void print_as(const char *when, const uint32_t a[12], const uint32_t irq[4])
{
	printk("t6_mmu: AS0 %s: TRANSTAB 0x%08x%08x MEMATTR 0x%08x%08x TRANSCFG 0x%08x%08x "
	       "LOCKADDR 0x%08x%08x STATUS 0x%08x FAULTSTATUS 0x%08x FAULTADDRESS 0x%08x%08x\n",
	       when, a[1], a[0], a[3], a[2], a[5], a[4], a[7], a[6], a[8], a[9], a[11], a[10]);
	printk("t6_mmu: IRQ %s: MMU_INT_RAWSTAT 0x%08x MMU_INT_MASK 0x%08x GPU_INT_RAWSTAT 0x%08x "
	       "GPU_INT_MASK 0x%08x\n",
	       when, irq[0], irq[1], irq[2], irq[3]);
}

static void print_op(const char *what, const uint32_t o[10])
{
	printk("t6_mmu: %s: LOCKADDR 0x%08x%08x, LOCK polls %u/%u; GPU_CMD 0x%08x, "
	       "GPU_INT_RAWSTAT 0x%08x before the clear, 0x%08x after, CLEAN_CACHES_COMPLETED "
	       "after %u polls of 1 us; UNLOCK polls %u/%u\n",
	       what, o[1], o[0], o[2], o[3], o[4], o[5], o[6], o[7], o[8], o[9]);
}

/* A step at or after `step` was reached. */
static bool reached(uint32_t stop, uint32_t step)
{
	return stop == YD_T6_MMU_STOP_NONE || stop > step;
}

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;
	struct yd_t6_gpu_up z1;
	struct yd_t6_mmu_up o;
	uint64_t tables_pa, backing_pa;
	int64_t t0;
	int rc;

	printk("t6_mmu: build %s\n", T6_MMU_BUILD);
	printk("t6_mmu: WDT0 period %u ms, fed until %u ms of uptime\n", t6_watchdog_period_ms(),
	       t6_watchdog_deadline_ms());
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	/* GPU control at 0x0, the MMU block at 0x2000-0x25ff */
	device_map(&gpu, GPU_PHYS, 0x4000, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	rc = yd_t6_gpu_up(cru, pmu, spi2, qos, gpu, delay_us, on_handover, &z1);
	t6_watchdog_feed();
	printk("t6_mmu: Z1: rc %d stop %u refusal %u: GPU_ID 0x%08x MMU_FEATURES 0x%08x "
	       "AS_PRESENT 0x%08x L2_READY 0x%x\n",
	       rc, z1.stop, z1.refusal, z1.ident[0], z1.ident[4], z1.ident[5], z1.l2_ready[0]);
	if (rc != 0) {
		printk("t6_mmu: FAIL: Z1's power-on stopped (stop %u, refusal %u, detail "
		       "0x%08x%08x); run t6_gpu for its lines\n",
		       z1.stop, z1.refusal, z1.detail[1], z1.detail[0]);
		t6_reset_to_linux("t6_mmu done");
	}

	tables_pa = k_mem_phys_addr(t6_mmu_pool);
	backing_pa = k_mem_phys_addr(t6_mmu_backing);
	printk("t6_mmu: pool %p at PA 0x%llx (%u pages), backing %p at PA 0x%llx\n",
	       (void *)t6_mmu_pool, (unsigned long long)tables_pa, POOL_PAGES,
	       (void *)t6_mmu_backing, (unsigned long long)backing_pa);

	t0 = k_uptime_get();
	rc = yd_t6_mmu_up(gpu, z1.ident[4], t6_mmu_pool, POOL_PAGES, tables_pa, backing_pa, clean,
			  delay_us, &o);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_mmu: FAIL: yd_t6_mmu_up refused its arguments\n");
		t6_reset_to_linux("t6_mmu done");
	}

	print_as("as found", o.found, o.found_irq);
	if (reached(o.stop, YD_T6_MMU_STOP_FLUSH)) {
		printk("t6_mmu: flush round trip: GPU_CMD 0x%08x, GPU_INT_RAWSTAT 0x%08x before the "
		       "clear, 0x%08x after; CLEAN_CACHES_COMPLETED after %u polls of 1 us\n",
		       o.flush[0], o.flush[1], o.flush[2], o.flush[3]);
	}
	if (reached(o.stop, YD_T6_MMU_STOP_FLUSH)) {
		printk("t6_mmu: tables at PA 0x%08x%08x: %u of %u pages used; backing PA 0x%08x%08x\n",
		       o.tables_pa[1], o.tables_pa[0], o.tables_used, POOL_PAGES, o.backing_pa[1],
		       o.backing_pa[0]);
	}
	if (reached(o.stop, YD_T6_MMU_STOP_TABLES)) {
		printk("t6_mmu: walk VA 0x%08x%08x: L0 0x%08x%08x L1 0x%08x%08x L2 0x%08x%08x "
		       "L3 0x%08x%08x (%u levels): PA 0x%08x%08x\n",
		       o.walk_va[1], o.walk_va[0], o.walk_desc[0][1], o.walk_desc[0][0],
		       o.walk_desc[1][1], o.walk_desc[1][0], o.walk_desc[2][1], o.walk_desc[2][0],
		       o.walk_desc[3][1], o.walk_desc[3][0], o.walk_levels, o.walk_pa[1],
		       o.walk_pa[0]);
	}
	if (reached(o.stop, YD_T6_MMU_STOP_WALK)) {
		printk("t6_mmu: tables cleaned to the point of coherency: rc %d\n", o.clean_rc);
	}
	if (reached(o.stop, YD_T6_MMU_STOP_ENABLE)) {
		print_op("AS0 enable, the whole-VA flush", o.enable_op);
		printk("t6_mmu: AS0 enable: TRANSTAB 0x%08x%08x MEMATTR 0x%08x%08x TRANSCFG "
		       "0x%08x%08x written; UPDATE polls %u/%u\n",
		       o.enable_regs[0][1], o.enable_regs[0][0], o.enable_regs[1][1],
		       o.enable_regs[1][0], o.enable_regs[2][1], o.enable_regs[2][0],
		       o.update_polls[0], o.update_polls[1]);
	}
	if (o.stop == YD_T6_MMU_STOP_NONE || o.stop >= YD_T6_MMU_STOP_ENABLE) {
		print_as("after", o.after, o.after_irq);
	}
	if (o.stop == YD_T6_MMU_STOP_NONE) {
		print_op("the flush after the map", o.flush_pt);
		printk("t6_mmu: PASS: AS0 on TRANSTAB 0x%08x%08x, three cache flush round trips, "
		       "%lld ms\n",
		       o.after[1], o.after[0], (long long)(k_uptime_get() - t0));
	} else {
		printk("t6_mmu: FAIL: stop %u (%s), refusal %u, detail 0x%08x%08x\n", o.stop,
		       o.stop < ARRAY_SIZE(stop_name) ? stop_name[o.stop] : "unknown", o.refusal,
		       o.detail[1], o.detail[0]);
	}
	t6_reset_to_linux("t6_mmu done");
}
