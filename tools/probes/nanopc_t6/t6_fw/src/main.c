/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_fw: Z3 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "The firmware
 * (Z3)"). The image embeds the CSF firmware (driver/src/fw/mali_csffw_blob.h,
 * made by driver/tools/mkmalifw.sh, never committed) and backs each of the
 * seven sections it loads with static storage, in the order of
 * fw::Image::sections() (struct yd_t6_fw_backing { pa, cpu, size } per
 * section). The shared-interface section's backing is __nocache: the CPU
 * reaches it through an uncached mapping, as the GPU does. Before main() the
 * board glue has armed WDT0 (nanopc_t6.c). yd_t6_fw_up runs Z1's power-on,
 * Z2's AS0 over every section, the load and the MCU's boot (GLB version, the
 * global configuration, one PING); main() prints its record and PASS or FAIL.
 * When Z1's power-on stops, it prints Z1's record as t6_gpu prints it
 * (../common/t6_z1.c) before the FAIL line.
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

#include "t6_z1.h"

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

/* In the order fw::Image::sections() lists the board's file: yd_t6_fw_up
 * takes the backings in it, not in ascending VA. */
static const struct t6_fw_section t6_fw_sections[] = {
	{ 0x00400000, t6_fw_sec1, sizeof(t6_fw_sec1) },
	{ 0x00410000, t6_fw_sec2, sizeof(t6_fw_sec2) },
	{ 0x00000000, t6_fw_sec0, sizeof(t6_fw_sec0) },
	{ 0x00800000, t6_fw_sec3, sizeof(t6_fw_sec3) },
	{ 0x02000000, t6_fw_sec5, sizeof(t6_fw_sec5) },
	{ 0x01000000, t6_fw_sec4, sizeof(t6_fw_sec4) },
	{ 0x04000000, t6_fw_shared, sizeof(t6_fw_shared) },
};
#define SECTIONS ARRAY_SIZE(t6_fw_sections)
BUILD_ASSERT(SECTIONS == 7, "the board's firmware has seven loaded sections");

/* driver/rust/src/ffi/rk3588.rs pins the same layouts. */
BUILD_ASSERT(sizeof(struct yd_t6_fw_up) == 192, "struct yd_t6_fw_up moved");
BUILD_ASSERT(sizeof(struct yd_t6_fw_backing) == 24, "struct yd_t6_fw_backing moved");

#define GPU_MAP_SIZE 0x280000 /* the doorbells are at 0x80000 and up */
#define TIMER_HZ     24000000 /* CNTFRQ_EL0 on the RK3588 */

static const char *const step_name[] = {
	[YD_T6_FW_STEP_NONE] = "none",
	[YD_T6_FW_STEP_PARSE] = "the image was refused",
	[YD_T6_FW_STEP_PLACE] = "the backings do not fit the sections",
	[YD_T6_FW_STEP_GPU] = "Z1's power-on stopped",
	[YD_T6_FW_STEP_MMU] = "Z2's AS0 stopped",
	[YD_T6_FW_STEP_BOOT] = "csf::boot refused",
	[YD_T6_FW_STEP_CLEAN] = "a cache clean of the load failed",
	[YD_T6_FW_STEP_VERSION] = "the GLB version is not the image's",
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

/* The Z3 record: every field of the run that reached the MCU. */
static void print_record(const struct yd_t6_fw_up *o)
{
	printk("t6_fw: Z3: image version 0x%08x, %u sections loaded, %u protected skipped, "
	       "%u table pages, %u flushes\n",
	       o->image_version, o->sections, o->protected_skipped, o->tables_used, o->flushed);
	printk("t6_fw: Z3: GPU_ID 0x%08x SHADER_PRESENT 0x%08x%08x MMU_FEATURES 0x%08x "
	       "shared PA 0x%08x%08x clean rc %d\n",
	       o->gpu_id, o->shader_present[1], o->shader_present[0], o->mmu_features,
	       o->shared_pa[1], o->shared_pa[0], o->clean_rc);
	printk("t6_fw: Z3: MCU_STATUS %u before, %u after, booted in %u us, JOB_INT_RAWSTAT "
	       "0x%08x\n",
	       o->mcu_status_before, o->mcu_status, o->boot_waited_us, o->job_raw);
	printk("t6_fw: Z3: GLB version %u.%u.%u (0x%08x), features 0x%08x, instr 0x%08x, "
	       "%u groups stride 0x%x\n",
	       (o->version >> 24) & 0xff, (o->version >> 16) & 0xff, o->version & 0xffff,
	       o->version, o->features, o->instr_features, o->group_num, o->group_stride);
	printk("t6_fw: Z3: GLB input 0x%08x output 0x%08x, CSG0 in 0x%08x out 0x%08x "
	       "%u streams stride 0x%x, CS0 in 0x%08x out 0x%08x features 0x%08x\n",
	       o->glb_input, o->glb_output, o->csg0[0], o->csg0[1], o->csg0[2], o->csg0[3],
	       o->cs0[0], o->cs0[1], o->cs0[2]);
	printk("t6_fw: Z3: CORE_EN 0x%08x%08x, poweroff 0x%08x progress 0x%08x idle 0x%08x, "
	       "ack_irq_mask 0x%08x\n",
	       o->core_en_mask[1], o->core_en_mask[0], o->poweroff_timer, o->progress_timer,
	       o->idle_timer, o->ack_irq_mask);
	printk("t6_fw: Z3: init req 0x%08x acked 0x%08x in %u us, PING %s in %u us, "
	       "GLB req 0x%08x ack 0x%08x\n",
	       o->init_req, o->acked, o->init_ack_us, o->ping_acked ? "acked" : "NOT acked",
	       o->ping_ack_us, o->glb_req, o->glb_ack);
}

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;
	struct yd_t6_fw_backing backings[SECTIONS];
	static struct yd_t6_gpu_up z1; /* 500 bytes: not on main()'s stack */
	struct yd_t6_fw_up o = { 0 };
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
		backings[i].pa = k_mem_phys_addr(t6_fw_sections[i].cpu);
		backings[i].cpu = (uintptr_t)t6_fw_sections[i].cpu;
		backings[i].size = t6_fw_sections[i].size;
		printk("t6_fw: section %u: VA 0x%08x, %u KiB, backing %p at PA 0x%llx\n",
		       (unsigned int)i, t6_fw_sections[i].va,
		       (unsigned int)(t6_fw_sections[i].size / 1024), (void *)t6_fw_sections[i].cpu,
		       (unsigned long long)backings[i].pa);
	}
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	/* GPU control at 0x0, the MMU block at 0x2000-0x25ff, the doorbells at 0x80000 */
	device_map(&gpu, GPU_PHYS, GPU_MAP_SIZE, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	printk("t6_fw: pool %p at PA 0x%llx (%u pages)\n", (void *)t6_fw_pool,
	       (unsigned long long)k_mem_phys_addr(t6_fw_pool), POOL_PAGES);

	/* Z1's power-on, Z2's AS0 over every section, the load and the MCU's
	 * boot, in one run. Every register access carries its own dmb (mmio.rs,
	 * Hw), the doorbell's included; clean() cleans each loaded section. */
	t0 = k_uptime_get();
	rc = yd_t6_fw_up(cru, pmu, spi2, qos, gpu, mali_csffw_blob, MALI_CSFFW_SIZE, backings,
			 SECTIONS, t6_fw_pool, POOL_PAGES, k_mem_phys_addr(t6_fw_pool), TIMER_HZ,
			 clean, delay_us, on_handover, &z1, &o);
	t6_watchdog_feed();
	printk("t6_fw: Z3: rc %d step %u, %lld ms\n", rc, o.step, (long long)(k_uptime_get() - t0));

	if (rc == -EINVAL) {
		printk("t6_fw: FAIL: yd_t6_fw_up refused its arguments\n");
	} else if (o.step == YD_T6_FW_STEP_NONE) {
		print_record(&o);
		if (rc == 0 && o.ping_acked && o.version == 0x01050000) {
			printk("t6_fw: PASS\n");
		} else {
			printk("t6_fw: FAIL: Z3 step %u (%s): rc %d, PING %s, GLB version 0x%08x\n",
			       o.step, step_name[o.step], rc, o.ping_acked ? "acked" : "not acked",
			       o.version);
		}
	} else {
		printk("t6_fw: Z3: stop %u refusal %u detail 0x%08x%08x acked 0x%08x\n", o.stop,
		       o.refusal, o.detail[1], o.detail[0], o.acked);
		if (o.step == YD_T6_FW_STEP_GPU) {
			t6_z1_print_stop("t6_fw", &z1);
		}
		if (o.step >= YD_T6_FW_STEP_BOOT) {
			print_record(&o);
		}
		printk("t6_fw: FAIL: Z3 step %u (%s)\n", o.step,
		       o.step < ARRAY_SIZE(step_name) ? step_name[o.step] : "unknown");
	}

	t6_reset_to_linux("t6_fw done");
}
