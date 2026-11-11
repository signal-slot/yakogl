/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_submit: Z4 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Submit
 * (Z4)"): Z3's firmware boot, CSG 0 / CS 0, the tiler heap, one L1 frame
 * (driver/rust/src/board/nanopc_t6_frame.rs, tools/mali/panthor/src/l1.rs over
 * src/mali/frame.rs) submitted and its sync object waited on. The image embeds
 * the CSF firmware (driver/src/fw/mali_csffw_blob.h, made by
 * driver/tools/mkmalifw.sh, never committed). yd_t6_submit brings Z3 up itself
 * (same arguments as yd_t6_fw_up), so this file does not call yd_t6_fw_up.
 *
 * Output on uart2: the build, WDT0, the image, its sections and the pool (as
 * t6_fw prints them), the "Z3: rc" line and Z3's record (t6_fw's formats), then
 * the Z4 record in the nine lines Z4a..Z4i that t6-verdict.sh judges (a 64-bit
 * word is 0x%016llx, a register word 0x%08x, a count or time decimal), then PASS or
 * FAIL and the reset. Every Z4 region is __nocache storage with PA = CPU
 * address; the watchdog is fed around the call, by on_handover and by every delay_us.
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

/* AS0's tables: the firmware's nine pages (Z3) and one for the queue memory,
 * from a pool of sixteen; the group VM's tables (ten pages) come from their
 * own pool of sixteen. */
#define POOL_PAGES 16
static uint8_t t6_submit_pool[POOL_PAGES * 4096] __aligned(4096);
#define VM_POOL_PAGES 16
static __nocache uint8_t t6_submit_vm_pool[VM_POOL_PAGES * 4096] __aligned(4096);

/*
 * Z3's seven firmware sections, backed as t6_fw backs them (the board's
 * mali_csffw.bin: 0x00000000, 0x00400000 and 0x00410000 64 KiB each, 0x00800000
 * 128 KiB, 0x01000000 and 0x02000000 256 KiB each, 0x04000000 the 64 KiB shared
 * interface, __nocache), listed in the order of fw::Image::sections().
 */
static uint8_t t6_submit_sec0[64 * 1024] __aligned(4096);
static uint8_t t6_submit_sec1[64 * 1024] __aligned(4096);
static uint8_t t6_submit_sec2[64 * 1024] __aligned(4096);
static uint8_t t6_submit_sec3[128 * 1024] __aligned(4096);
static uint8_t t6_submit_sec4[256 * 1024] __aligned(4096);
static uint8_t t6_submit_sec5[256 * 1024] __aligned(4096);
static __nocache uint8_t t6_submit_shared[64 * 1024] __aligned(4096);

struct t6_submit_section {
	uint32_t va;
	uint8_t *cpu;
	size_t size;
};

static const struct t6_submit_section t6_submit_sections[] = {
	{ 0x00400000, t6_submit_sec1, sizeof(t6_submit_sec1) },
	{ 0x00410000, t6_submit_sec2, sizeof(t6_submit_sec2) },
	{ 0x00000000, t6_submit_sec0, sizeof(t6_submit_sec0) },
	{ 0x00800000, t6_submit_sec3, sizeof(t6_submit_sec3) },
	{ 0x02000000, t6_submit_sec5, sizeof(t6_submit_sec5) },
	{ 0x01000000, t6_submit_sec4, sizeof(t6_submit_sec4) },
	{ 0x04000000, t6_submit_shared, sizeof(t6_submit_shared) },
};
#define SECTIONS ARRAY_SIZE(t6_submit_sections)
BUILD_ASSERT(SECTIONS == 7, "the board's firmware has seven loaded sections");

/*
 * Z4's storage, one buffer per YD_T6_SUBMIT_REGION_* (sizes from the header's
 * *_SIZE; the entry places each at its *_VA). The queue's AS0 backing holds
 * the 8 KiB of user pages and CSG 0's two suspend buffers (4 KiB rounded
 * each); 1 MiB is far over what the firmware asks, and a short one stops with
 * STEP_QUEUE_MEM and the bytes needed in detail. The regions are __nocache
 * (the CPU writes, the GPU or MCU reads; no clean is owed).
 */
#define QUEUE_MEM_SIZE 0x100000
static __nocache uint8_t t6_z4_queue_mem[QUEUE_MEM_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_queue[YD_T6_SUBMIT_QUEUE_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_arenas[YD_T6_SUBMIT_ARENAS_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_geometry[YD_T6_SUBMIT_GEOMETRY_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_target[YD_T6_SUBMIT_TARGET_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_heap_pool[YD_T6_SUBMIT_HEAP_POOL_SIZE] __aligned(4096);
static __nocache uint8_t t6_z4_heap_chunk[YD_T6_SUBMIT_HEAP_CHUNK_SIZE] __aligned(4096);

struct t6_z4_region {
	uint8_t *cpu;
	size_t size;
};

static const struct t6_z4_region t6_z4_regions[YD_T6_SUBMIT_REGIONS] = {
	[YD_T6_SUBMIT_REGION_QUEUE] = { t6_z4_queue, sizeof(t6_z4_queue) },
	[YD_T6_SUBMIT_REGION_ARENAS] = { t6_z4_arenas, sizeof(t6_z4_arenas) },
	[YD_T6_SUBMIT_REGION_GEOMETRY] = { t6_z4_geometry, sizeof(t6_z4_geometry) },
	[YD_T6_SUBMIT_REGION_TARGET] = { t6_z4_target, sizeof(t6_z4_target) },
	[YD_T6_SUBMIT_REGION_HEAP_POOL] = { t6_z4_heap_pool, sizeof(t6_z4_heap_pool) },
	[YD_T6_SUBMIT_REGION_HEAP_CHUNK] = { t6_z4_heap_chunk, sizeof(t6_z4_heap_chunk) },
};

BUILD_ASSERT(YD_T6_SUBMIT_QUEUE_MCU_MIN <= QUEUE_MEM_SIZE, "queue memory");
BUILD_ASSERT(sizeof(struct yd_t6_fw_up) == 192, "struct yd_t6_fw_up moved");
BUILD_ASSERT(sizeof(struct yd_t6_fw_backing) == 24, "struct yd_t6_fw_backing moved");
BUILD_ASSERT(sizeof(struct yd_t6_submit_up) == 392, "struct yd_t6_submit_up moved");
BUILD_ASSERT(sizeof(struct yd_t6_submit_mem) == 192, "struct yd_t6_submit_mem moved");

#define GPU_MAP_SIZE 0x280000 /* the doorbells are at 0x80000 and up */
#define TIMER_HZ     24000000 /* CNTFRQ_EL0 on the RK3588 */

static const char *const step_name[] = {
	[YD_T6_SUBMIT_STEP_NONE] = "none",
	[YD_T6_SUBMIT_STEP_FRAME] = "the frame or its memory refused",
	[YD_T6_SUBMIT_STEP_FW] = "Z3 stopped",
	[YD_T6_SUBMIT_STEP_IFACE] = "reading the interface again refused",
	[YD_T6_SUBMIT_STEP_IDENT] = "no address space 1",
	[YD_T6_SUBMIT_STEP_QUEUE_MEM] = "the queue memory is short or misaligned",
	[YD_T6_SUBMIT_STEP_AS0_MAP] = "adding the queue memory to AS0 refused",
	[YD_T6_SUBMIT_STEP_CLEAN] = "a clean of table pages failed",
	[YD_T6_SUBMIT_STEP_AS0_FLUSH] = "AS0's FLUSH_PT stopped",
	[YD_T6_SUBMIT_STEP_VM_MAP] = "the group VM's tables refused",
	[YD_T6_SUBMIT_STEP_VM_ENABLE] = "address space 1's enable stopped",
	[YD_T6_SUBMIT_STEP_HEAP] = "the tiler heap refused",
	[YD_T6_SUBMIT_STEP_START] = "sched::start refused",
	[YD_T6_SUBMIT_STEP_SUBMIT] = "sched::submit refused",
	[YD_T6_SUBMIT_STEP_WAIT] = "sched::wait refused",
	[YD_T6_SUBMIT_STEP_STOP] = "CS 0's stop refused",
	[YD_T6_SUBMIT_STEP_TERMINATE] = "CSG 0's terminate refused",
};

/* The Rust waits (Z3's bring-up, sched::wait's 5 s job timeout) poll through
 * this callback and run between two of the feeds below, which the board glue
 * wants at most CONFIG_YAKOGL_NANOPC_T6_ALIVE_S (5 s) apart: a run that ends
 * in the 5 s timeout would be reset before it prints its Z4 record. So every
 * delay feeds the watchdog, in slices of at most 100 ms. t6_watchdog_feed() is
 * one store of the uptime (any context), and the Rust calls this from thread
 * context only. A loop that stops calling delay_us() is still reset after
 * ALIVE_S. */
static void delay_us(uint32_t us)
{
	do {
		uint32_t slice = us > 100000U ? 100000U : us;

		t6_watchdog_feed();
		k_busy_wait(slice);
		us -= slice;
	} while (us > 0U);
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

/* Z3's record, as t6_fw prints it (the same formats, this probe's prefix). */
static void print_z3(const struct yd_t6_fw_up *o)
{
	printk("t6_submit: Z3: image version 0x%08x, %u sections loaded, %u protected skipped, "
	       "%u table pages, %u flushes\n",
	       o->image_version, o->sections, o->protected_skipped, o->tables_used, o->flushed);
	printk("t6_submit: Z3: GPU_ID 0x%08x SHADER_PRESENT 0x%08x%08x MMU_FEATURES 0x%08x "
	       "shared PA 0x%08x%08x clean rc %d\n",
	       o->gpu_id, o->shader_present[1], o->shader_present[0], o->mmu_features,
	       o->shared_pa[1], o->shared_pa[0], o->clean_rc);
	printk("t6_submit: Z3: MCU_STATUS %u before, %u after, booted in %u us, JOB_INT_RAWSTAT "
	       "0x%08x\n",
	       o->mcu_status_before, o->mcu_status, o->boot_waited_us, o->job_raw);
	printk("t6_submit: Z3: GLB version %u.%u.%u (0x%08x), features 0x%08x, instr 0x%08x, "
	       "%u groups stride 0x%x\n",
	       (o->version >> 24) & 0xff, (o->version >> 16) & 0xff, o->version & 0xffff,
	       o->version, o->features, o->instr_features, o->group_num, o->group_stride);
	printk("t6_submit: Z3: GLB input 0x%08x output 0x%08x, CSG0 in 0x%08x out 0x%08x "
	       "%u streams stride 0x%x, CS0 in 0x%08x out 0x%08x features 0x%08x\n",
	       o->glb_input, o->glb_output, o->csg0[0], o->csg0[1], o->csg0[2], o->csg0[3],
	       o->cs0[0], o->cs0[1], o->cs0[2]);
	printk("t6_submit: Z3: CORE_EN 0x%08x%08x, poweroff 0x%08x progress 0x%08x idle 0x%08x, "
	       "ack_irq_mask 0x%08x\n",
	       o->core_en_mask[1], o->core_en_mask[0], o->poweroff_timer, o->progress_timer,
	       o->idle_timer, o->ack_irq_mask);
	printk("t6_submit: Z3: init req 0x%08x acked 0x%08x in %u us, PING %s in %u us, "
	       "GLB req 0x%08x ack 0x%08x\n",
	       o->init_req, o->acked, o->init_ack_us, o->ping_acked ? "acked" : "NOT acked",
	       o->ping_ack_us, o->glb_req, o->glb_ack);
}

/* The Z4 record in nine lines, Z4a..Z4i, as t6-verdict.sh's table_t6_submit
 * judges them: a 64-bit word is 0x%016llx, a register word 0x%08x, a count or
 * time decimal. The fields are those of struct yd_t6_submit_up after fw. */
static void print_z4(const struct yd_t6_submit_up *o)
{
	printk("t6_submit: Z4a: step %u refusal %u detail 0x%016llx\n", o->step, o->refusal,
	       (unsigned long long)o->detail);
	printk("t6_submit: Z4b: AS_PRESENT 0x%08x L2_FEATURES 0x%08x as0_tables_used %u "
	       "vm_tables_used %u suspend_size %u protm_suspend_size %u\n",
	       o->as_present, o->l2_features, o->as0_tables_used, o->vm_tables_used,
	       o->suspend_size, o->protm_suspend_size);
	printk("t6_submit: Z4c: heap ctx 0x%016llx chunk 0x%016llx chunk_size %u stride %u\n",
	       (unsigned long long)o->heap_ctx, (unsigned long long)o->heap_chunk,
	       o->heap_chunk_size, o->heap_stride);
	printk("t6_submit: Z4d: CSG0 req 0x%08x ack 0x%08x CS0 req 0x%08x ack 0x%08x, "
	       "acked in %u/%u us\n",
	       o->csg_req, o->csg_ack, o->cs_req, o->cs_ack, o->csg_ack_us, o->cs_ack_us);
	printk("t6_submit: Z4e: insert 0x%016llx extract 0x%016llx seqno 0x%016llx "
	       "sync_status 0x%08x, wait %u us\n",
	       (unsigned long long)o->insert, (unsigned long long)o->extract,
	       (unsigned long long)o->seqno, o->sync_status, o->wait_us);
	printk("t6_submit: Z4f: latest_flush 0x%08x sync_updates %u status_cmd_ptr 0x%016llx "
	       "info 0x%016llx\n",
	       o->latest_flush, o->sync_updates, (unsigned long long)o->status_cmd_ptr,
	       (unsigned long long)o->info);
	printk("t6_submit: Z4g: stop %u us refusal %u, terminate %u us refusal %u, "
	       "cleared %u %u %u\n",
	       o->stop_us, o->stop_refusal, o->terminate_us, o->terminate_refusal,
	       o->cleared[0], o->cleared[1], o->cleared[2]);
	printk("t6_submit: Z4h: target red %u blue %u fill %u other %u of %u pixels\n", o->red,
	       o->blue, o->fill, o->other, o->pixels);
	printk("t6_submit: Z4i: frame digest 0x%016llx\n", (unsigned long long)o->digest);
}

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;
	struct yd_t6_fw_backing backings[SECTIONS];
	struct yd_t6_submit_mem mem = { 0 };
	static struct yd_t6_submit_up o;
	int64_t t0;
	int rc;

	printk("t6_submit: build %s\n", T6_SUBMIT_BUILD);
	printk("t6_submit: WDT0 period %u ms, fed until %u ms of uptime\n",
	       t6_watchdog_period_ms(), t6_watchdog_deadline_ms());
	/* The embedded image: its size, digest and magic (little endian). */
	printk("t6_submit: image %u bytes, %u loaded sections, sha256 %s, magic 0x%08x\n",
	       (unsigned int)MALI_CSFFW_SIZE, (unsigned int)SECTIONS, MALI_CSFFW_SHA256,
	       (unsigned int)(mali_csffw_blob[0] | mali_csffw_blob[1] << 8 |
			      mali_csffw_blob[2] << 16 | (uint32_t)mali_csffw_blob[3] << 24));
	for (size_t i = 0; i < SECTIONS; i++) {
		backings[i].pa = k_mem_phys_addr(t6_submit_sections[i].cpu);
		backings[i].cpu = (uintptr_t)t6_submit_sections[i].cpu;
		backings[i].size = t6_submit_sections[i].size;
		printk("t6_submit: section %u: VA 0x%08x, %u KiB, backing %p at PA 0x%llx\n",
		       (unsigned int)i, t6_submit_sections[i].va,
		       (unsigned int)(t6_submit_sections[i].size / 1024),
		       (void *)t6_submit_sections[i].cpu, (unsigned long long)backings[i].pa);
	}
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&gpu, GPU_PHYS, GPU_MAP_SIZE, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	printk("t6_submit: pool %p at PA 0x%llx (%u pages)\n", (void *)t6_submit_pool,
	       (unsigned long long)k_mem_phys_addr(t6_submit_pool), POOL_PAGES);

	mem.queue.pa = k_mem_phys_addr(t6_z4_queue_mem);
	mem.queue.cpu = (uintptr_t)t6_z4_queue_mem;
	mem.queue.size = sizeof(t6_z4_queue_mem);
	for (size_t i = 0; i < YD_T6_SUBMIT_REGIONS; i++) {
		mem.regions[i].pa = k_mem_phys_addr(t6_z4_regions[i].cpu);
		mem.regions[i].cpu = (uintptr_t)t6_z4_regions[i].cpu;
		mem.regions[i].size = t6_z4_regions[i].size;
	}
	mem.vm_pool = t6_submit_vm_pool;
	mem.vm_pool_pages = VM_POOL_PAGES;
	mem.vm_tables_pa = k_mem_phys_addr(t6_submit_vm_pool);

	/* Z1 to Z3 as t6_fw, then Z4, in one run; the probe feeds around it. */
	t0 = k_uptime_get();
	rc = yd_t6_submit(cru, pmu, spi2, qos, gpu, mali_csffw_blob, MALI_CSFFW_SIZE, backings,
			  SECTIONS, t6_submit_pool, POOL_PAGES, k_mem_phys_addr(t6_submit_pool),
			  TIMER_HZ, &mem, clean, delay_us, on_handover, &o);
	t6_watchdog_feed();
	printk("t6_submit: Z3: rc %d step %u, %lld ms\n", rc, o.fw.step,
	       (long long)(k_uptime_get() - t0));

	if (rc == -EINVAL) {
		printk("t6_submit: FAIL: yd_t6_submit refused its arguments\n");
	} else {
		if (o.fw.step != YD_T6_FW_STEP_NONE) {
			printk("t6_submit: Z3: stop %u refusal %u detail 0x%08x%08x acked 0x%08x\n",
			       o.fw.stop, o.fw.refusal, o.fw.detail[1], o.fw.detail[0],
			       o.fw.acked);
		}
		if (o.fw.step == YD_T6_FW_STEP_NONE || o.fw.step >= YD_T6_FW_STEP_BOOT) {
			print_z3(&o.fw);
		}
		print_z4(&o);
		if (rc == 0 && o.step == YD_T6_SUBMIT_STEP_NONE &&
		    o.digest == YD_T6_SUBMIT_DIGEST) {
			printk("t6_submit: PASS\n");
		} else {
			printk("t6_submit: FAIL: Z4 step %u (%s): rc %d, refusal %u\n", o.step,
			       o.step < ARRAY_SIZE(step_name) ? step_name[o.step] : "unknown", rc,
			       o.refusal);
		}
	}

	t6_reset_to_linux("t6_submit done");
}
