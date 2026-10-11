/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_fw_boot.c -- PowerVR BXS-4-64 (RISC-V firmware) boot for Zephyr: the C
 * shim of the kernel side. It maps the carveout, owns the kernel side's
 * context (struct yd_pvr_km) and sequences the boot: the firmware image
 * through the DDK's loader, the FWIF structures, the GPU page tables, the
 * DDK's RGXStart, the wait for the firmware. The pvr_drv.h API the render
 * service and the self-test use is wrappers here.
 *
 * What is not C: the carveout's allocators, the page tables, the command
 * rings, the freelists and the FWIF structures are Rust
 * (driver/rust/src/gpu/pvr: mem, mmu, ccb, freelist, fwinit; ours, written
 * against the vendored DDK types, docs/source-form.md step 7,
 * driver/docs/pvr-kernel-rust.md); the firmware start/stop and the image
 * loader are the TI/Img DDK 25.2's rgxstartstop.c and rgxfwimageutils.c
 * (third_party/ti-img-rogue-ddk-25.2, Dual MIT/GPLv2, used under MIT),
 * compiled unmodified on our rgxlayer (rgxlayer.rs, step 6). This file began
 * as a port of Linux drivers/gpu/drm/imagination; nothing of that is left
 * (step 7). The bring-up self-test render is pvr_selftest.c.
 */

#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/cache.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/logging/log.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>

#include "pvr_ddk.h"
#include "rgxstartstop.h"
#include "rgxfwimageutils.h"
#include <yakogl_drivers.h>
#include <safeui/yd_env.h>
#include "pvr_internal.h"
#include "pvr_drv.h"

LOG_MODULE_DECLARE(gpu_pvr, CONFIG_SAFEUI_GPU_PVR_LOG_LEVEL);

/* Registers, heap bases and firmware-interface layouts come from the vendored
 * DDK headers (driver/compat/pvr_ddk.h); driver/tools/offsetof/ddk_abi_pins.h
 * pins the ones the C uses, gpu::pvr::fwif (tools/pvr/km) the Rust ones. */

/* The firmware's active power management latency (RGXFWIF_RUNTIME_CFG
 * ui32ActivePMLatencyms), set at boot. With 100 (the value
 * written until 2026-09-29) every MMU cache invalidate after a process's
 * first was acknowledged ~98 ms after its post on the AM62P; the delay
 * follows this value (median 98.45 / 18.95 / 4.10 ms at 100 / 20 / 5,
 * records/am62p/pm-latency). 5 ran the Linux regression set with nothing
 * moved (records/am62p/pm5-regset). Power cost of the shorter latency not
 * measured. The build sets it through PVR_FW_PM_LATENCY_MS (Zephyr:
 * CONFIG_SAFEUI_GPU_PVR_PM_LATENCY_MS). */
#ifndef PVR_FW_PM_LATENCY_MS
#define PVR_FW_PM_LATENCY_MS 5
#endif
uint32_t pvr_fw_pm_latency_ms = PVR_FW_PM_LATENCY_MS;

/* ---- the DDK firmware --------------------------------------------------- *
 * Drive the TI/Img production DDK firmware rgx.fw.36.53.104.796 (25.2), which
 * is PROVEN to tile geometry on this AM62P (rgx_triangle_test, 58.9 FPS). The
 * firmware is loaded and started by the DDK's own code (rgxfwimageutils.c,
 * rgxstartstop.c RGXStart), compiled as vendored; see "the DDK's start/stop
 * and image loader" below.
 */

/* ---- FW address regions / heap layout --------------------------------- */
/* The config region at the top of the firmware heap
 * (RGX_FIRMWARE_CONFIG_HEAP_BASE): connection control, OS init and system
 * init at fixed slots the firmware is linked against (gpu::pvr::fwinit). */
#define FW_HEAP_CONFIG_OFFSET      ((uint32_t)PVR_FW_HEAP_CONFIG_OFFSET)

/* The ring sizes gpu::pvr::ccb builds (KCCB_NUM_CMDS / FWCCB_NUM_CMDS there). */
#define KCCB_NUM_CMDS   128u   /* NUMCMDS_LOG2_DEFAULT = 7 */
#define FWCCB_NUM_CMDS  32u    /* NUMCMDS_LOG2 = 5         */

/* ---- the kernel side's context (Rust: gpu::pvr::km) ------------------ *
 * The carveout's allocators and the GPU page tables are Rust
 * (driver/rust/src/gpu/pvr/mem.rs, mmu.rs; docs/source-form.md step 7,
 * driver/docs/pvr-kernel-rust.md): this file owns their context, fills its
 * layout and OS hooks at boot (pvr_fw_boot) and keeps the pvr_drv.h API as
 * thin wrappers over it. */
static struct yd_pvr_km g_km;
#define g_cpu (g_km.cpu)   /* mapped CPU base of the carveout */

static void km_flush(void *ctx, const void *addr, size_t len)
{
	ARG_UNUSED(ctx);
	sys_cache_data_flush_range((void *)addr, len);
}

static void km_invd(void *ctx, const void *addr, size_t len)
{
	ARG_UNUSED(ctx);
	sys_cache_data_invd_range((void *)addr, len);
}

static void km_fence(void *ctx)
{
	ARG_UNUSED(ctx);
	barrier_dmem_fence_full();
}

static void km_busy_wait_us(void *ctx, uint32_t us)
{
	ARG_UNUSED(ctx);
	k_busy_wait(us);
}

/* A line from the Rust side, into this driver's log module. Lines are at most
 * 400 bytes (gpu::pvr::km::KM_LINE_MAX); the logger may defer formatting, so
 * the text goes in as a "%s" of a terminated copy. */
static void km_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	char line[401];

	ARG_UNUSED(ctx);
	if (len >= sizeof(line)) {
		len = sizeof(line) - 1;
	}
	memcpy(line, buf, len);
	line[len] = '\0';
	switch (level) {
	case 1:
		LOG_ERR("%s", line);
		break;
	case 2:
		LOG_WRN("%s", line);
		break;
	case 3:
		LOG_INF("%s", line);
		break;
	default:
		LOG_DBG("%s", line);
		break;
	}
}

static const struct yd_pvr_km_ops g_km_ops = {
	.ctx = NULL,
	.flush = km_flush,
	.invd = km_invd,
	.fence = km_fence,
	.busy_wait_us = km_busy_wait_us,
	.log = km_log,
};

/* Grows of grow_pages each that every growable freelist can serve before it
 * answers "no grow" when there is no shared pool (4 x 256 pages = 4 MiB per
 * global list). */
#ifndef PVR_FL_GROW_RESERVE_DEFAULT
#define PVR_FL_GROW_RESERVE_DEFAULT 4
#endif

/* CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK (gpu::pvr::ccb): a FWCCB slot read
 * without the command magic is re-read for up to 2 ms before it is
 * consumed, and the write offset is watched for going back. Off by default
 * (an instrument, tools/cts/log/investigations/pvr-render-pool.md "FWCCB slot
 * read empty"). */
#ifndef PVR_FL_PROBE_SERVE_MAX
/* CONFIG_SAFEUI_GPU_PVR_FL_PROBE_SERVE_MAX: a probe, never a product
 * setting (gpu::pvr::freelist::serve). 0: off. */
#define PVR_FL_PROBE_SERVE_MAX 0
#endif
#ifndef PVR_FW_UNCACHED_DEFAULT
/* CONFIG_SAFEUI_GPU_PVR_FW_UNCACHED (gpu::pvr::mem heap_fwaddr): hand the
 * firmware the uncached window for the host's objects, as Linux
 * drm/imagination does. The Kconfig option is on by default and defines this
 * as 1; 0 here is what a build gets with the option set to n (the cached
 * window, kept for measurement). */
#define PVR_FW_UNCACHED_DEFAULT 0
#endif
#ifndef PVR_FWCCB_CHECK_DEFAULT
#define PVR_FWCCB_CHECK_DEFAULT 0
#endif

#ifndef PVR_POOL_TOP_ABOVE
/* GENERAL render-pool requests of more than this many bytes come from the
 * carveout's end downward (gpu::pvr::pool, the top region); -DPVR_POOL_TOP_ABOVE=0
 * builds the arm without it, every block from the bump as before. */
#define PVR_POOL_TOP_ABOVE YD_PVR_POOL_TOP_ABOVE
#endif

/* The context for a mapped carveout and register block (pvr_internal.h's
 * layout). */
static void km_setup(uint8_t *cpu, mm_reg_t regs)
{
	g_km = (struct yd_pvr_km){
		.cpu = cpu,
		.phys = PVR_GPU_MEM_PHYS,
		.size = PVR_GPU_MEM_SIZE,
		.ops = &g_km_ops,
		.bus = NULL,
		.regs = regs,
		.fw_main_size = (uint32_t)PVR_FW_HEAP_MAIN_PHYS,
		.fw_config_off = FW_HEAP_CONFIG_OFFSET,
		.pt_area_off = (uint32_t)PVR_PT_AREA_OFFSET,
		.pt_area_end = (uint32_t)PVR_PT_AREA_END,
		.pool_off = (uint32_t)PVR_RENDER_POOL_OFFSET,
		.pt_next = (uint32_t)PVR_PT_AREA_OFFSET,
		.pool_next = (uint32_t)PVR_RENDER_POOL_OFFSET,
		.fl_reserve_mult = PVR_FL_GROW_RESERVE_DEFAULT,
		.fwccb_check = PVR_FWCCB_CHECK_DEFAULT,
		.fw_uncached = PVR_FW_UNCACHED_DEFAULT,
		.fl_probe_serve_max = PVR_FL_PROBE_SERVE_MAX,
		.pool_top_above = PVR_POOL_TOP_ABOVE,
	};
}

static inline uint8_t *heap_cpu(uint32_t off)   { return yd_pvr_heap_cpu(&g_km, off); }
static inline uint64_t heap_gpu(uint32_t off)   { return yd_pvr_heap_gpu(&g_km, off); }
static inline uint32_t heap_fwaddr(uint32_t off) { return yd_pvr_heap_fwaddr(&g_km, off); }
static inline uint64_t heap_phys(uint32_t off)  { return yd_pvr_heap_phys(&g_km, off); }

uint32_t pvr_heap_alloc(uint32_t size, uint32_t align)
{
	return yd_pvr_heap_alloc(&g_km, size, align);
}

/* ---- GPU MMU (Rust: gpu::pvr::mmu) ------------------------------------- */
bool pvr_mmu_take_dirty(void)
{
	return yd_pvr_mmu_take_dirty(&g_km);
}

void pvr_mmu_map(uint64_t va, uint64_t phys, size_t size, uint64_t l0flags)
{
	yd_pvr_mmu_map(&g_km, va, phys, size, l0flags);
}

unsigned pvr_mmu_audit(const uint64_t *allow_lo, const uint64_t *allow_hi, unsigned nallow)
{
	return yd_pvr_mmu_audit(&g_km, allow_lo, allow_hi, nallow);
}

uint64_t pvr_mmu_lookup(uint64_t va)
{
	return yd_pvr_mmu_lookup(&g_km, va);
}

/* The page catalogue, then the firmware heap's window onto the carveout. */
static int pvr_mmu_build(void)
{
	yd_pvr_mmu_init(&g_km);
	yd_pvr_mmu_map_fw_heap(&g_km);
	return 0;
}

/* ---- the DDK's start/stop and image loader ---------------------------- *
 * RGXStart, RGXStop (rgxstartstop.c) and the firmware image loader
 * (rgxfwimageutils.c) are the vendored DDK's, compiled unmodified by
 * driver/compat/pvr_rgxstartstop.c and pvr_rgxfwimageutils.c. The rgxlayer.h
 * functions they call are ours, driver/rust/src/gpu/pvr/rgxlayer.rs
 * (libyakogl_drivers.a), and this is the context (`hPrivate`) they run on.
 * tools/pvr/rgxstart/README.md has what the switch to them changed on the
 * register bus. */
static struct yd_env g_rgx_env;
static struct yd_pvr_layer g_rgx;

/* The firmware heap's sub-heaps (rgx_heap_firmware.h, sized by the
 * configuration's RGX_FW_CODE/DATA/RODATA_SECTION_SIZE, the build the
 * firmware matches): the code at the bottom (the DDK requires the code
 * allocation at RGX_FIRMWARE_RAW_HEAP_BASE, rgxinit.c), the RW data and the RO
 * data in their own sub-heaps, where the firmware's boot code maps its data
 * window (it reprograms the boot data remap to RW data + 132 KiB itself, the
 * value a running DDK showed), and the host's objects in the main heap after
 * them. Until docs/source-form.md step 7 the RW data offset was the value
 * captured from a running DDK (0x80000, the same number) and the objects
 * followed the RO data (0xa1000). */
#define FW_CODE_OFF   ((uint32_t)(RGX_FIRMWARE_CODE_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE))
#define FW_DATA_OFF   ((uint32_t)(RGX_FIRMWARE_PRIV_DATA_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE))
#define FW_RODATA_OFF ((uint32_t)(RGX_FIRMWARE_PRIV_RODATA_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE))
#define FW_OBJS_OFF   ((uint32_t)(RGX_FIRMWARE_HOST_MAIN_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE))
static uint32_t g_code_off, g_data_off, g_rodata_off;

/* The core clock the firmware is told (RGXFWIF_RUNTIME_CFG, below); here for
 * RGXWaitCycles. */
#define PVR_CORE_CLOCK_HZ 800000000u

static void rgx_layer_init(mm_reg_t regs)
{
	yd_env_init(&g_rgx_env, NULL);
	g_rgx = (struct yd_pvr_layer){
		.regs = regs,
		.env = &g_rgx_env,
		.bus = NULL,
		.kernel_pc = 0, /* set once the MMU is built (pvr_fw_start) */
		.fw_code_devva = PVR_FW_HEAP_GPU_BASE + FW_CODE_OFF,
		.fw_data_devva = PVR_FW_HEAP_GPU_BASE + FW_DATA_OFF,
		.core_clock_hz = PVR_CORE_CLOCK_HZ,
		.poll_timeout_us = 0, /* the DDK's bound on this SoC, 0.5 s */
		.poll_step_us = 0,
		.powered = 0,
	};
}

/* The DDK firmware is embedded at build time from a LOCAL copy the integrator
 * makes with tools/mkfwblob.sh (src/fw/pvr_fw_blob.h, gitignored: the blob is
 * TI/Img property and is never committed). */
#include "fw/pvr_fw_blob.h"

static int ddk_fw_get_blob(const uint8_t **fw_out, uint32_t *size_out)
{
	*fw_out = pvr_fw_blob;
	*size_out = PVR_FW_BLOB_SIZE;
	return 0;
}

/* The DDK's loader: RGXGetFWImageAllocSize reads the info header and layout
 * table at the end of the image (the last FW_BLOCK_SIZE bytes), checks the
 * DDK version the image was built for and sums the allocation sizes;
 * RGXProcessFWImage copies every PT_LOAD segment to the allocation of its
 * layout entry's type (at the entry's allocation offset), zeroes the bss tails
 * and fills the RISC-V bootloader's data block (no coremem on this core).
 * Placement checked against the former loader of this file, byte for byte on
 * the real image: tools/pvr/rgxlayer_check.c. */
static int pvr_fw_load(void)
{
	const uint8_t *fw;
	uint32_t size;
	IMG_DEVMEM_SIZE_T code_sz, data_sz, rodata_sz, cm_code_sz, cm_data_sz;
	RGX_FW_INFO_HEADER info;
	PVRSRV_FW_BOOT_PARAMS params;
	PVRSRV_ERROR e;

	if (ddk_fw_get_blob(&fw, &size) != 0) {
		return -ENOENT;
	}
	e = RGXGetFWImageAllocSize(&g_rgx, fw, size, &code_sz, &data_sz, &rodata_sz, &cm_code_sz,
				   &cm_data_sz, &info);
	if (e != PVRSRV_OK) {
		LOG_ERR("DDK FW: image rejected (RGXGetFWImageAllocSize: %d)", (int)e);
		return -EINVAL;
	}
	g_code_off = FW_CODE_OFF;
	g_data_off = FW_DATA_OFF;
	g_rodata_off = FW_RODATA_OFF;
	/* The image's own layout has to agree with the sub-heaps: its RO data
	 * follows its RW data at the distance of the two sub-heaps (in the layout
	 * table, RISCV_PRIVATE_RODATA - RISCV_PRIVATE_DATA), and every part fits
	 * its sub-heap; no coremem on this core. */
	if (cm_code_sz != 0 || cm_data_sz != 0 || code_sz > RGX_FIRMWARE_CODE_HEAP_SIZE ||
	    data_sz > RGX_FIRMWARE_PRIV_DATA_HEAP_SIZE || rodata_sz > RGX_FIRMWARE_PRIV_RODATA_HEAP_SIZE ||
	    RGXGetFWImageSectionAddress(&g_rgx, RISCV_PRIVATE_RODATA) -
			    RGXGetFWImageSectionAddress(&g_rgx, RISCV_PRIVATE_DATA) !=
		    FW_RODATA_OFF - FW_DATA_OFF) {
		LOG_ERR("DDK FW: layout does not fit the sub-heaps (code %u, data %u, rodata %u, coremem %u/%u B)",
			(uint32_t)code_sz, (uint32_t)data_sz, (uint32_t)rodata_sz, (uint32_t)cm_code_sz,
			(uint32_t)cm_data_sz);
		return -EINVAL;
	}
	g_km.heap_next = FW_OBJS_OFF;
	LOG_INF("DDK FW: code@0x%x (%u B), data@0x%x (%u B), objs from 0x%x", g_code_off,
		(uint32_t)code_sz, g_data_off, g_rodata_off + (uint32_t)rodata_sz - g_data_off, g_km.heap_next);

	memset(&params, 0, sizeof(params)); /* sRISCV: no coremem */
	e = RGXProcessFWImage(&g_rgx, fw, heap_cpu(g_code_off), heap_cpu(g_data_off),
			      heap_cpu(g_rodata_off), NULL, NULL, &params);
	if (e != PVRSRV_OK) {
		LOG_ERR("DDK FW: RGXProcessFWImage failed (%d)", (int)e);
		return -EINVAL;
	}
	LOG_INF("DDK FW: %u-byte image for DDK %u.%u loaded (code 0x%x, data 0x%x, rodata 0x%x)",
		size, info.ui16PVRVersionMajor, info.ui16PVRVersionMinor, g_code_off, g_data_off,
		g_rodata_off);

	return 0;
}

/* ---- FWIF control structures ----------------------------------------- *
 * Rust (gpu::pvr::fwinit); their heap offsets are in g_km. */

/* Listed pages for each global freelist; 0 = the carveout-derived default. */
uint32_t pvr_fl_init_pages;

/* The freelists, their grow pool and the CCBs are Rust (gpu::pvr::freelist,
 * ccb); their state is in g_km. */
void pvr_fl_set_reserve_mult(uint32_t mult)
{
	g_km.fl_reserve_mult = mult;
}

/* ---- exported accessors (pvr_drv.h) ---------------------------------- */
static mm_reg_t g_regs_saved;
mm_reg_t pvr_regs(void)                    { return g_regs_saved; }
struct yd_pvr_km *pvr_km(void)             { return &g_km; }
uint8_t *pvr_drv_carveout_cpu(void)        { return g_cpu; }
uint64_t pvr_drv_carveout_phys(void)       { return PVR_GPU_MEM_PHYS; }
uint8_t *pvr_heap_cpu(uint32_t off)        { return heap_cpu(off); }
uint32_t pvr_heap_fwaddr(uint32_t off)     { return heap_fwaddr(off); }
uint64_t pvr_heap_gpu(uint32_t off)        { return heap_gpu(off); }
uint64_t pvr_pc_phys(void)                 { return yd_pvr_mmu_pc_phys(&g_km); }
uint32_t *pvr_pm_heap_bump(void)           { return &g_km.pm_bump; }
uint32_t pvr_kccb_ctl_off(void)            { return g_km.kccb_ctl_off; }
uint32_t pvr_hwrinfobuf_off(void)          { return g_km.hwrinfobuf_off; }
uint32_t pvr_kccb_rtn_off(void)            { return g_km.kccb_rtn_off; }
uint32_t pvr_fwccb_ctl_off(void)           { return g_km.fwccb_ctl_off; }
uint32_t pvr_fwccb_cmds_off(void)          { return g_km.fwccb_cmds_off; }
uint32_t pvr_tracebuf_off(void)            { return g_km.tracebuf_off; }
uint32_t pvr_ddk_trace_buf_off(void)       { return g_km.trace_buf_off; }

/* The structures the firmware reads at boot, at the DDK's sizes, the host's
 * objects from the DDK's host main heap (gpu::pvr::fwinit). The two bring-up
 * switches the C had stay compile-time: PVR_ASSERT_ON_HWR (halt, do not
 * reset, on a recovery, so the fault registers persist) and
 * PVR_CTXSWITCH_OFF. */
static int pvr_fw_build_structs(void)
{
	const struct yd_pvr_fwinit cfg = {
		.pm_latency_ms = pvr_fw_pm_latency_ms,
		.core_clock_hz = PVR_CORE_CLOCK_HZ,
		.flags = 0
#ifdef PVR_ASSERT_ON_HWR
			 | YD_PVR_FWINIT_ASSERT_ON_HWR
#endif
#ifdef PVR_CTXSWITCH_OFF
			 | YD_PVR_FWINIT_CTXSWITCH_OFF
#endif
		,
	};

	yd_pvr_fwinit_build(&g_km, &cfg);
	return 0;
}

/* ---- firmware start / stop (the DDK's rgxstartstop.c) ------------------ */
/* Start the firmware: the DDK's RGXStart (reset, SLC, ECC safety events, the
 * RISC-V wrapper's boot windows, AXI-ACE, the firmware MMU context's catalogue
 * base, the core's release). Needs the firmware loaded and the MMU built. */
static int pvr_fw_start(void)
{
	PVRSRV_ERROR e;

	g_rgx.kernel_pc = yd_pvr_mmu_pc_phys(&g_km);
	e = RGXStart(&g_rgx);
	if (e != PVRSRV_OK) {
		LOG_ERR("RGXStart failed (%d)", (int)e);
		return -EIO;
	}
	return 0;
}

/* Stop the firmware processor for power-down: the DDK's RGXStop (waits for the
 * GPU to idle, bounded). The firmware must have been asked to power off first;
 * nothing in the driver calls this yet. */
int pvr_fw_stop(void)
{
	PVRSRV_ERROR e = RGXStop(&g_rgx);

	if (e != PVRSRV_OK) {
		LOG_ERR("RGXStop failed (%d)", (int)e);
		return e == PVRSRV_ERROR_TIMEOUT ? -ETIMEDOUT : -EIO;
	}
	return 0;
}

static int pvr_wait_for_fw_boot(void)
{
	return yd_pvr_fw_wait_started(&g_km);
}

/* ---- the render pool (Rust: gpu::pvr::mem) ------------------------------ */
/* Render-object backing: the render pool, the carveout's tail, mapped into
 * the GPU heaps (Rust: gpu::pvr::mem, which has what each one does). */
static inline struct pvr_gpu_bo bo_from(struct yd_pvr_bo b)
{
	return (struct pvr_gpu_bo){ .gpu = b.gpu, .cpu = b.cpu, .heap_off = b.heap_off };
}

struct pvr_gpu_bo pvr_gpu_bo_alloc_f(uint64_t heap_base, uint32_t *heap_bump,
				     uint32_t size, uint64_t l0flags)
{
	return bo_from(yd_pvr_bo_alloc(&g_km, heap_base, heap_bump, size, l0flags));
}

bool pvr_gpu_bo_bump_rollback(uint64_t heap_base, uint32_t *heap_bump, uint32_t heap_off,
			      const uint8_t *cpu, uint32_t size)
{
	ARG_UNUSED(heap_base);
	return yd_pvr_bo_rollback(&g_km, heap_bump, heap_off, cpu, size);
}

struct pvr_gpu_bo pvr_gpu_bo_alloc(uint64_t heap_base, uint32_t *heap_bump, uint32_t size)
{
	return pvr_gpu_bo_alloc_f(heap_base, heap_bump, size, PVR_L0_FLAGS_DATA);
}

struct pvr_gpu_bo pvr_gpu_bo_alloc_pm(uint64_t heap_base, uint32_t *heap_bump, uint32_t size)
{
	return bo_from(yd_pvr_bo_alloc_pm(&g_km, heap_base, heap_bump, size));
}

struct pvr_gpu_bo pvr_gpu_bo_place(uint64_t heap_base, uint32_t start_off, uint32_t size,
				   uint64_t l0flags)
{
	return bo_from(yd_pvr_bo_place(&g_km, heap_base, start_off, size, l0flags));
}

/* ---- the command rings (Rust: gpu::pvr::ccb) --------------------------- *
 * Posting keeps the release barrier between the write offset reaching
 * memory and the MTS doorbell (the "lost doorbell" stall), and reads the
 * doorbell back as the DDK's __MTSScheduleWrite does; ccb.rs has the order. */
void pvr_kccb_doorbell(mm_reg_t regs)
{
	ARG_UNUSED(regs);
	yd_pvr_kccb_doorbell(&g_km);
}

void pvr_kccb_probe_slot(uint32_t slot, const char *why)
{
	yd_pvr_kccb_probe_slot(&g_km, slot, why);
}

uint32_t pvr_kccb_post(mm_reg_t regs, const RGXFWIF_KCCB_CMD *cmd)
{
	BUILD_ASSERT(sizeof(*cmd) == 160, "gpu::pvr::fwif::KCCB_CMD_SIZE");
	ARG_UNUSED(regs);
	return yd_pvr_kccb_post(&g_km, (const uint8_t *)cmd);
}

/* The freelist handshake: the FW asks (RGXFWIF_FWCCB_CMD_FREELIST_GROW /
 * _FREELISTS_RECONSTRUCTION), the host answers on the KCCB (Rust:
 * gpu::pvr::freelist). */
unsigned pvr_freelist_audit(void)
{
	return yd_pvr_fl_audit(&g_km);
}

int pvr_fwccb_service(mm_reg_t regs)
{
	ARG_UNUSED(regs);
	return yd_pvr_fwccb_service(&g_km);
}

uint64_t pvr_bo_phys(const struct pvr_gpu_bo *bo)
{
	struct yd_pvr_bo b = { .gpu = bo->gpu, .cpu = bo->cpu, .heap_off = bo->heap_off };

	return yd_pvr_bo_phys(&g_km, &b);
}

void pvr_fl_pool_init(uint32_t *gen_b, uint32_t pages)
{
	yd_pvr_fl_pool_init(&g_km, gen_b, pages);
}

/* Grow requests the host refused, seen and served, and the pool's peak: live
 * counters that answer "did the firmware ask, and did we answer?" without
 * depending on the log still holding the lines. */
uint32_t pvr_fl_refusals(void)
{
	return g_km.fl_no_grow;
}

uint32_t pvr_fl_grow_reqs(void)
{
	return g_km.fl_grow_reqs;
}

uint32_t pvr_fl_grows_done(void)
{
	return g_km.fl_grows_done;
}

uint32_t pvr_fl_pool_peak_pages(void)
{
	return g_km.fl_pool_peak;
}

uint32_t pvr_build_freelist(uint32_t *gen_b, uint32_t npages, uint32_t maxpages,
			    uint32_t grow, uint32_t ready, uint32_t fl_id)
{
	return yd_pvr_fl_build(&g_km, gen_b, npages, maxpages, grow, ready, fl_id);
}

int pvr_fw_boot(mm_reg_t regs)
{
	int rc;

	g_regs_saved = regs;
	rgx_layer_init(regs);

	/* Map the GPU carveout Normal non-cacheable (the attribute Linux's
	 * dma_alloc_coherent gives on arm64). The firmware and the CPU
	 * share cache lines in the CCB control blocks and the HWRT data (the
	 * CPU writes write_offset, the firmware read_offset), so a cached
	 * mapping with clean/invalidate by hand loses the firmware's updates on
	 * the second kick (seen on silicon: a 3D BIF fault at VA 0). Device
	 * memory (K_MEM_CACHE_NONE) is not an option: it alignment-faults the
	 * memcpy/struct writes. The explicit cache calls stay for targets whose
	 * mapping is cached. */
	uint8_t *cpu = NULL;
#ifdef K_MEM_ARM_NORMAL_NC
	k_mem_map_phys_bare(&cpu, PVR_GPU_MEM_PHYS, PVR_GPU_MEM_SIZE,
			    K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
	k_mem_map_phys_bare(&cpu, PVR_GPU_MEM_PHYS, PVR_GPU_MEM_SIZE,
			    K_MEM_CACHE_WB | K_MEM_PERM_RW);
#endif
	km_setup(cpu, regs);
	if (g_cpu == NULL) {
		LOG_ERR("GPU carveout map failed (VM space?); aborting FW boot");
		return -ENOMEM;
	}
	LOG_INF("GPU carveout phys 0x%lx (%lu MiB) mapped at %p",
		(unsigned long)PVR_GPU_MEM_PHYS,
		(unsigned long)(PVR_GPU_MEM_SIZE / (1024 * 1024)), (void *)g_cpu);

	rc = pvr_fw_load();
	if (rc) {
		return rc;
	}
	rc = pvr_fw_build_structs();
	if (rc) {
		return rc;
	}
	rc = pvr_mmu_build();
	if (rc) {
		return rc;
	}

	/* Push everything (FW image, page tables, FWIF structs) out to DDR so
	 * the non-coherent GPU sees it. */
	sys_cache_data_flush_range(g_cpu, PVR_GPU_MEM_SIZE);

	LOG_INF("M3 complete (MMU + FW load + FWIF structs). Booting FW core...");
	rc = pvr_fw_start();
	if (rc) {
		return rc;
	}

	rc = pvr_wait_for_fw_boot();
	if (rc == 0) {
		LOG_INF("GPU bring-up milestone 4 complete (firmware booted)");
		/* Ack any pending OS0 IRQ the FW raised. */
		if (pvr_rd32(regs, RGX_CR_IRQ_OS0_EVENT_STATUS) & RGX_CR_IRQ_OS0_EVENT_STATUS_SOURCE_EN) {
			pvr_wr32(regs, RGX_CR_IRQ_OS0_EVENT_CLEAR, RGX_CR_IRQ_OS0_EVENT_CLEAR_SOURCE_EN);
		}
		/* No KCCB round-trip test at boot (the former M5 HEALTH_CHECK, gone
		 * with the C ring code in step 7): the render's KICK is the host->FW
		 * path that matters and is exercised every frame. */
		LOG_INF("DDK FW: skipping open-FW healthcheck; render KICK drives the FW");
#if !defined(PVR_RENDER_REGS_STUB) && !defined(PVR_NO_SELFTEST_RENDER)
		/* M-render: first GPU draw (TQ_3D fill). Enabled once the final
		 * cmd_transfer register values are filled in pvr_render_regs.h. */
		(void)pvr_render_ta3d(regs);
#ifdef PVR_RENDER_TWICE
		/* Warm-context test: the DDK trace that tiles (Alloc>0) is frame 984
		 * (warm). Run a 2nd render after the 1st (which HWRs) to see if a
		 * post-recovery / warmed geom context tiles differently. */
		LOG_INF("=== SECOND RENDER (warm-context test) ===");
		(void)pvr_render_ta3d(regs);
#endif
#else
		LOG_INF("render: scaffolding built; awaiting final cmd_transfer "
			"regs (pvr_render_regs.h) before first GPU draw");
#endif
	}
	return rc;
}
