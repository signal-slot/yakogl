/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * record_c.c -- runs the PowerVR kernel-side scenarios (scenario.h) on the C
 * the driver had before docs/source-form.md step 7: the functions of
 * driver/src/pvr_fw_boot.c at origin/main 674ea1cf that record_c.sh cuts
 * out into old_*.inc, compiled here unchanged against the recording
 * platform (rec.h). Its record is committed (*-674ea1cf.rec); km_check.c
 * runs the same scenarios on the Rust modules and tools/check.sh compares.
 */
#include <assert.h>
#include <errno.h>
/* Zephyr's errno value where the host's differs (the Rust side returns
 * Zephyr's numbers) */
#undef ETIMEDOUT
#define ETIMEDOUT 116
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "pvr_drv.h"
#include "rec.h"
#include "scenario.h"

/* What the cut-out code uses from Zephyr and from the rest of the file. */
#define ROUND_UP(x, align) \
	((((unsigned long)(x) + ((unsigned long)(align) - 1)) / (unsigned long)(align)) * (unsigned long)(align))
#ifndef ARRAY_SIZE
#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#endif
#define __ASSERT(cond, ...) do { if (!(cond)) { printf("FAIL assert: " __VA_ARGS__); printf("\n"); exit(3); } } while (0)
#define LOG_ERR(fmt, ...) printf("L1 " fmt "\n", ##__VA_ARGS__)
#define LOG_WRN(fmt, ...) printf("L2 " fmt "\n", ##__VA_ARGS__)
#define LOG_INF(fmt, ...) printf("L3 " fmt "\n", ##__VA_ARGS__)
#define LOG_DBG(fmt, ...) printf("L4 " fmt "\n", ##__VA_ARGS__)

static void sys_cache_data_flush_range(void *p, size_t n)
{
	rec_flush(p, n);
}

static void sys_cache_data_invd_range(void *p, size_t n)
{
	rec_invd(p, n);
}

static void barrier_dmem_fence_full(void)
{
	rec_fence();
}

/* hostshim/: register access and time are the recorder's */
uint32_t sys_read32(mm_reg_t addr)
{
	return rec_read32(addr);
}

void sys_write32(uint32_t data, mm_reg_t addr)
{
	rec_write32(addr, data);
}

void rec_busy_wait(uint32_t us)
{
	rec_busy_wait_us(us);
}

#pragma GCC diagnostic ignored "-Wformat"
#pragma GCC diagnostic ignored "-Wunused-variable"
#include "old_layout.inc"
#include "old_mem_mmu.inc"
#include "old_fwif_vars.inc"
#include "old_bo.inc"
#include "old_bo_phys.inc"
#include "old_ccb.inc"
#include "old_fl.inc"
uint32_t pvr_fw_pm_latency_ms = 5;
#include "old_fwinit.inc"
#include "old_wait.inc"
/* The render service of 674ea1cf (pvr_render.c, whole) and the accessors it
 * calls in pvr_fw_boot.c. Its static PDS/USC areas need the command-stream
 * generators, which the scenarios do not reach: stubs that stop the run. */
#include "old_accessors.inc"
#include "pvr_render_regs.h"
#include "pvr_pds_render_gen.h"
#include "pvr_pds_pixel_event_gen.h"
#define LOG_MODULE_DECLARE(...)
#ifndef DIV_ROUND_UP
#define DIV_ROUND_UP(n, d) (((n) + (d) - 1) / (d))
#endif
#ifndef MAX
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#endif
uint64_t pvr_pds_pe_build_doutu_src0(uint64_t a, uint32_t b, uint32_t c, int d)
{
	(void)a; (void)b; (void)c; (void)d;
	printf("FAIL a generator was called\n");
	exit(2);
}
int pvr_pds_generate_pixel_event_tq(uint64_t a, const uint32_t *b, uint32_t c, int d, uint32_t *e,
				    uint32_t *f, uint32_t *g, uint32_t *h)
{
	(void)a; (void)b; (void)c; (void)d; (void)e; (void)f; (void)g; (void)h;
	printf("FAIL a generator was called\n");
	exit(2);
}
int pvr_pds_render_kick_usc_program(uint64_t a, uint32_t b, uint32_t c, uint32_t *d, uint32_t *e,
				    struct pvr_pds_render_kick_usc_info *f)
{
	(void)a; (void)b; (void)c; (void)d; (void)e; (void)f;
	printf("FAIL a generator was called\n");
	exit(2);
}
uint32_t pvr_cr_event_pixel_pds_info(uint32_t n)
{
	(void)n;
	printf("FAIL a generator was called\n");
	exit(2);
}
#include "old_render.inc"

static void c_fw_offsets_setup(uint32_t hwrinfobuf, uint32_t tracebuf, uint32_t ring)
{
	g_hwrinfobuf_off = hwrinfobuf;
	g_tracebuf_off = tracebuf;
	g_ddk_trace_buf_off = ring;
}
static void c_render_cfg(const uint32_t c[10])
{
	pvr_render_combined_kick = c[0];
	pvr_render_wait_rering = c[1];
	pvr_render_wait_repost = c[2];
	pvr_render_mmu_ack = c[3];
	pvr_render_mmu_gate = c[4];
	pvr_render_mmu_ack_probe = c[5];
	pvr_render_mmu_ack_reping = c[6];
	pvr_render_hwrt_refuse = c[7];
	pvr_render_hwrt_cleanup = c[8];
	pvr_render_hwrt_reinit = c[9];
}
/* pvr_render_init() less the static PDS/USC areas, with the scenario's
 * small grow pool (km_check.c r_render_init does the same on the Rust). */
static void c_render_init(void)
{
	memset(&R, 0, sizeof(R));
	R.pbuf = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &R.gen_b, PB_RESERVE_BYTES);
	R.border = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &R.gen_b, 4096);
	pvr_fl_set_reserve_mult(0);
	pvr_fl_pool_init(&R.gen_b, 256);
	R.gfl_off = pvr_build_freelist(&R.gen_b, 2048, 2048 + 256, 1024, 0, 1);
	R.gfl2_off = pvr_build_freelist(&R.gen_b, 2048, 2048 + 256, 1024, 0, 2);
	R.fl_off = pvr_build_freelist(&R.gen_b, 26, 32, 0, 0, 3);
	render_context_init();
	{
		RGXFWIF_HWRINFOBUF *hib = (void *)pvr_heap_cpu(pvr_hwrinfobuf_off());

		sys_cache_data_invd_range(hib, sizeof(*hib));
		R.hwr_seen = hib->ui32HwrCounter;
	}
	R.inited = true;
}
static int c_render_submit(const struct pvr_render_kick *k, uint32_t *fence)
{
	return pvr_render_submit(k, fence);
}
static int c_render_wait(uint32_t fence, uint32_t timeout_ms) { return pvr_render_wait(fence, timeout_ms); }
static uint32_t c_render_completed(void) { return pvr_render_completed(); }
static void c_render_offsets(uint32_t o[8])
{
	o[0] = R.rc_off;
	o[1] = R.gcccbctl_off;
	o[2] = R.gcccb_off;
	o[3] = R.fcccbctl_off;
	o[4] = R.fcccb_off;
	o[5] = R.gufo_off;
	o[6] = R.fufo_off;
	o[7] = R.mc_sync_off;
}

/* ---- the API over the former functions ---- */
static struct km_bo bo_of(struct pvr_gpu_bo b)
{
	return (struct km_bo){ .gpu = b.gpu, .cpu = b.cpu, .heap_off = b.heap_off };
}
static uint32_t c_heap_alloc(uint32_t size, uint32_t align) { return pvr_heap_alloc(size, align); }
static uint8_t *c_heap_cpu(uint32_t off) { return heap_cpu(off); }
static uint32_t c_heap_fwaddr(uint32_t off) { return heap_fwaddr(off); }
static uint64_t c_heap_gpu(uint32_t off) { return heap_gpu(off); }
static uint64_t c_heap_phys(uint32_t off) { return heap_phys(off); }
static struct km_bo c_bo_alloc(uint64_t base, uint32_t *bump, uint32_t size, uint64_t flags)
{
	return bo_of(pvr_gpu_bo_alloc_f(base, bump, size, flags));
}
static struct km_bo c_bo_alloc_pm(uint64_t base, uint32_t *bump, uint32_t size)
{
	return bo_of(pvr_gpu_bo_alloc_pm(base, bump, size));
}
static bool c_bo_rollback(uint64_t base, uint32_t *bump, uint32_t heap_off, const uint8_t *cpu, uint32_t size)
{
	return pvr_gpu_bo_bump_rollback(base, bump, heap_off, cpu, size);
}
static struct km_bo c_bo_place(uint64_t base, uint32_t start, uint32_t size, uint64_t flags)
{
	return bo_of(gpu_bo_place(base, start, size, flags));
}
static void c_pool_align(uint32_t align) { g_render_next = ROUND_UP(g_render_next, align); }
static uint64_t c_bo_phys(const struct km_bo *b)
{
	struct pvr_gpu_bo x = { .gpu = b->gpu, .cpu = b->cpu, .heap_off = b->heap_off };

	return pvr_bo_phys(&x);
}
static void c_mmu_init(void) { pvr_mmu_init(); }
/* pvr_mmu_build() less its pvr_mmu_init() call: the map and the log line. */
static void c_mmu_map_fw_heap(void)
{
	const uint64_t fw_flags = RGX_MMUCTRL_PT_DATA_SLC_BYPASS_CTRL_EN;

	pvr_mmu_map(PVR_FW_HEAP_GPU_BASE, PVR_GPU_MEM_PHYS, PVR_FW_HEAP_MAIN_PHYS, fw_flags);
	pvr_mmu_map(heap_gpu(FW_HEAP_CONFIG_OFFSET), heap_phys(FW_HEAP_CONFIG_OFFSET),
		    PVR_FW_HEAP_SIZE - FW_HEAP_CONFIG_OFFSET, fw_flags);
	LOG_INF("MMU: FW heap mapped (%u KiB main + %u KiB config of the 32 MiB window), PC root phys=0x%llx",
		(unsigned)(PVR_FW_HEAP_MAIN_PHYS / 1024),
		(unsigned)((PVR_FW_HEAP_SIZE - FW_HEAP_CONFIG_OFFSET) / 1024), (unsigned long long)g_pc_phys);
}
static void c_mmu_map(uint64_t va, uint64_t phys, uint64_t size, uint64_t flags)
{
	pvr_mmu_map(va, phys, size, flags);
}
static bool c_take_dirty(void) { return pvr_mmu_take_dirty(); }
static uint64_t c_lookup(uint64_t va) { return pvr_mmu_lookup(va); }
static unsigned c_audit(const uint64_t *lo, const uint64_t *hi, unsigned n) { return pvr_mmu_audit(lo, hi, n); }
static uint64_t c_pc_phys(void) { return g_pc_phys; }
static void c_heap_set_next(uint32_t off) { g_heap_next = off; }
static void c_ccb_setup(uint32_t kctl, uint32_t kcmds, uint32_t krtn, uint32_t fctl, uint32_t fcmds)
{
	g_kccb_ctl_off = kctl;
	g_kccb_cmds_off = kcmds;
	g_kccb_rtn_off = krtn;
	g_fwccb_ctl_off = fctl;
	g_fwccb_cmds_off = fcmds;
}
static uint32_t c_kccb_post(const uint8_t *cmd) { return pvr_kccb_post(0, (const RGXFWIF_KCCB_CMD *)cmd); }
static void c_kccb_doorbell(void) { pvr_kccb_doorbell(0); }
static void c_kccb_probe_slot(uint32_t slot, const char *why) { pvr_kccb_probe_slot(slot, why); }
static int c_fwccb_service(void) { return pvr_fwccb_service(0); }
static void c_fl_set_reserve_mult(uint32_t m) { pvr_fl_set_reserve_mult(m); }
static void c_fl_pool_init(uint32_t *gen_b, uint32_t pages) { pvr_fl_pool_init(gen_b, pages); }
static uint32_t c_fl_build(uint32_t *gen_b, uint32_t np, uint32_t max, uint32_t grow, uint32_t ready, uint32_t id)
{
	return pvr_build_freelist(gen_b, np, max, grow, ready, id);
}
static unsigned c_fl_audit(void) { return pvr_freelist_audit(); }
static void c_fwinit_build(uint32_t pm_latency_ms, uint32_t core_clock_hz, bool legacy)
{
	if (!legacy || core_clock_hz != 800000000u) {
		printf("FAIL the former C has the former sizes and an 800 MHz clock only\n");
		exit(2);
	}
	pvr_fw_pm_latency_ms = pm_latency_ms;
	(void)pvr_fw_build_structs();
}
static int c_fw_wait_started(void) { return pvr_wait_for_fw_boot(); }
static void c_fwinit_offsets(uint32_t o[10])
{
	o[0] = g_kccb_ctl_off;
	o[1] = g_kccb_cmds_off;
	o[2] = g_kccb_rtn_off;
	o[3] = g_fwccb_ctl_off;
	o[4] = g_fwccb_cmds_off;
	o[5] = g_sysinit_off;
	o[6] = g_tracebuf_off;
	o[7] = g_ddk_trace_buf_off;
	o[8] = g_hwrinfobuf_off;
	o[9] = g_heap_next;
}
static void c_fl_counters(uint32_t out[5])
{
	out[0] = pvr_fl_refusals();
	out[1] = pvr_fl_grow_reqs();
	out[2] = pvr_fl_grows_done();
	out[3] = pvr_fl_pool_peak_pages();
	out[4] = g_pm_b;
}

static const struct km_api c_api = {
	.heap_alloc = c_heap_alloc,
	.heap_cpu = c_heap_cpu,
	.heap_fwaddr = c_heap_fwaddr,
	.heap_gpu = c_heap_gpu,
	.heap_phys = c_heap_phys,
	.bo_alloc = c_bo_alloc,
	.bo_alloc_pm = c_bo_alloc_pm,
	.bo_rollback = c_bo_rollback,
	.bo_place = c_bo_place,
	.pool_align = c_pool_align,
	.bo_phys = c_bo_phys,
	.mmu_init = c_mmu_init,
	.mmu_map_fw_heap = c_mmu_map_fw_heap,
	.mmu_map = c_mmu_map,
	.mmu_take_dirty = c_take_dirty,
	.mmu_lookup = c_lookup,
	.mmu_audit = c_audit,
	.pc_phys = c_pc_phys,
	.heap_set_next = c_heap_set_next,
	.ccb_setup = c_ccb_setup,
	.kccb_post = c_kccb_post,
	.kccb_doorbell = c_kccb_doorbell,
	.kccb_probe_slot = c_kccb_probe_slot,
	.fwccb_service = c_fwccb_service,
	.fl_set_reserve_mult = c_fl_set_reserve_mult,
	.fl_pool_init = c_fl_pool_init,
	.fl_build = c_fl_build,
	.fl_audit = c_fl_audit,
	.fl_counters = c_fl_counters,
	.fwinit_build = c_fwinit_build,
	.fw_wait_started = c_fw_wait_started,
	.fwinit_offsets = c_fwinit_offsets,
	.fw_offsets_setup = c_fw_offsets_setup,
	.render_init = c_render_init,
	.render_cfg = c_render_cfg,
	.render_submit = c_render_submit,
	.render_wait = c_render_wait,
	.render_completed = c_render_completed,
	.render_offsets = c_render_offsets,
};

int main(int argc, char **argv)
{
	rec_init();
	g_cpu = rec_cpu;
	if (argc == 2 && !strcmp(argv[1], "mem")) {
		scenario_mem_mmu(&c_api);
	} else if (argc == 2 && !strcmp(argv[1], "ccb")) {
		scenario_ccb(&c_api);
	} else if (argc == 2 && !strcmp(argv[1], "fwinit")) {
		scenario_fwinit(&c_api, true);
	} else if (argc == 2 && !strcmp(argv[1], "render")) {
#ifdef PVR_RENDER_VERBOSE
		scenario_render(&c_api, true);
#else
		scenario_render(&c_api, false);
#endif
	} else {
		fprintf(stderr, "usage: record_c mem|ccb|fwinit|render\n");
		return 2;
	}
	rec_dump_memory();
	return 0;
}
