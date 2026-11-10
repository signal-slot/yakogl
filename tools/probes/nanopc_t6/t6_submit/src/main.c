/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_submit: Z4 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Submit
 * (Z4)"): Z3's firmware boot, CSG 0 / CS 0, the tiler heap, one L1 frame
 * (driver/rust/src/board/nanopc_t6_frame.rs, tools/mali/panthor/src/l1.rs over
 * src/mali/frame.rs) submitted and its sync object waited on. The image embeds
 * the CSF firmware (driver/src/fw/mali_csffw_blob.h, made by
 * driver/tools/mkmalifw.sh, never committed). The Z4 entry brings Z3 up
 * itself, so this file does not call yd_t6_fw_up.
 *
 * This is the scaffold: it prints the build, the firmware digest, the Z3
 * section backings and the Z4 storage (the VA the frame uses, the size, the
 * PA), then stops with a FAIL line. The entry (yd_t6_submit) and its record
 * are not wired yet; the probe-verdict step replaces the FAIL line with the
 * call and the record, taking its storage from t6_z4_regions below.
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
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

/* AS0's tables: the firmware's nine pages (Z3) and the Z4 regions' (the
 * frame's arenas and buffers, the heap, the ring, the sync object), from a
 * pool of thirty-two. */
#define POOL_PAGES 32
static uint8_t t6_submit_pool[POOL_PAGES * 4096] __aligned(4096);

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
 * Z4's storage. The VAs and sizes are driver/rust/src/board/nanopc_t6_frame.rs's
 * pub consts (generated from l1.rs, d2d9a816): CTX_VA/CTX_SIZE, DESC_VA/DESC_SIZE,
 * CODE_VA/CODE_SIZE, SPDS_VA/SPDS_SIZE, DATA_VA/DATA_SIZE,
 * GEOMETRY_BUFFER_VA/GEOMETRY_BUFFER_SIZE, TARGET_VA (64x64 RGBA8 at row stride
 * 256: 16 KiB), HEAP_FIRST_CHUNK_VA/HEAP_CHUNK_SIZE. The ring buffer (4 KiB,
 * panthor's size), the sync object page (4 KiB, one 16-byte object) and the heap
 * context page (4 KiB) have no fixed VA in the frame: the Z4 entry places them
 * (va 0 here). Every region the CPU writes and the GPU or MCU reads is
 * __nocache, so no cache clean is owed; the heap chunk is only touched by the
 * GPU (plain bss).
 */
static __nocache uint8_t t6_z4_ctx[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_desc[0x4000] __aligned(4096);
static __nocache uint8_t t6_z4_code[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_spds[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_data[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_geometry[0x10000] __aligned(4096);
static __nocache uint8_t t6_z4_target[0x4000] __aligned(4096);
static __nocache uint8_t t6_z4_ring[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_sync[0x1000] __aligned(4096);
static __nocache uint8_t t6_z4_heap_ctx[0x1000] __aligned(4096);
static uint8_t t6_z4_heap_chunk[0x200000] __aligned(4096);

struct t6_z4_region {
	const char *name;
	uint64_t va; /* 0: the Z4 entry places it */
	uint8_t *cpu;
	size_t size;
};

static const struct t6_z4_region t6_z4_regions[] = {
	{ "context arena", 0x102000000ULL, t6_z4_ctx, sizeof(t6_z4_ctx) },
	{ "descriptor arena", 0x102010000ULL, t6_z4_desc, sizeof(t6_z4_desc) },
	{ "code arena", 0x102020000ULL, t6_z4_code, sizeof(t6_z4_code) },
	{ "spds arena", 0x102030000ULL, t6_z4_spds, sizeof(t6_z4_spds) },
	{ "data arena", 0x102040000ULL, t6_z4_data, sizeof(t6_z4_data) },
	{ "geometry buffer", 0x102100000ULL, t6_z4_geometry, sizeof(t6_z4_geometry) },
	{ "render target", 0x103000000ULL, t6_z4_target, sizeof(t6_z4_target) },
	{ "heap first chunk", 0x800000a00000ULL, t6_z4_heap_chunk, sizeof(t6_z4_heap_chunk) },
	{ "heap context", 0, t6_z4_heap_ctx, sizeof(t6_z4_heap_ctx) },
	{ "CS ring buffer", 0, t6_z4_ring, sizeof(t6_z4_ring) },
	{ "sync object", 0, t6_z4_sync, sizeof(t6_z4_sync) },
};
#define Z4_REGIONS ARRAY_SIZE(t6_z4_regions)

#define GPU_MAP_SIZE 0x280000 /* the doorbells are at 0x80000 and up */

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;

	printk("t6_submit: build %s\n", T6_SUBMIT_BUILD);
	printk("t6_submit: WDT0 period %u ms, fed until %u ms of uptime\n",
	       t6_watchdog_period_ms(), t6_watchdog_deadline_ms());
	/* The embedded image: its size, digest and magic (little endian). */
	printk("t6_submit: image %u bytes, %u loaded sections, sha256 %s, magic 0x%08x\n",
	       (unsigned int)MALI_CSFFW_SIZE, (unsigned int)SECTIONS, MALI_CSFFW_SHA256,
	       (unsigned int)(mali_csffw_blob[0] | mali_csffw_blob[1] << 8 |
			      mali_csffw_blob[2] << 16 | (uint32_t)mali_csffw_blob[3] << 24));
	for (size_t i = 0; i < SECTIONS; i++) {
		printk("t6_submit: section %u: VA 0x%08x, %u KiB, backing %p at PA 0x%llx\n",
		       (unsigned int)i, t6_submit_sections[i].va,
		       (unsigned int)(t6_submit_sections[i].size / 1024),
		       (void *)t6_submit_sections[i].cpu,
		       (unsigned long long)k_mem_phys_addr(t6_submit_sections[i].cpu));
	}
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&gpu, GPU_PHYS, GPU_MAP_SIZE, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	printk("t6_submit: pool %p at PA 0x%llx (%u pages)\n", (void *)t6_submit_pool,
	       (unsigned long long)k_mem_phys_addr(t6_submit_pool), POOL_PAGES);
	for (size_t i = 0; i < Z4_REGIONS; i++) {
		const struct t6_z4_region *r = &t6_z4_regions[i];

		if (r->va) {
			printk("t6_submit: Z4 %s: VA 0x%llx, ", r->name, (unsigned long long)r->va);
		} else {
			printk("t6_submit: Z4 %s: VA by the entry, ", r->name);
		}
		printk("%u bytes, backing %p at PA 0x%llx\n", (unsigned int)r->size,
		       (void *)r->cpu, (unsigned long long)k_mem_phys_addr(r->cpu));
	}
	t6_watchdog_feed();

	/* probe-verdict replaces this with yd_t6_submit over t6_z4_regions, and
	 * the Z4 record and PASS or FAIL. */
	printk("t6_submit: FAIL: Z4 entry not wired (ffi-entry)\n");

	t6_reset_to_linux("t6_submit done");
}
