/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * km_check.c -- the PowerVR kernel side in Rust (driver/rust/src/gpu/pvr:
 * km, mem, mmu, ccb, freelist, fwinit, render; the yakogl-drivers crate built for the host with feature
 * pvr) on the recording platform of rec.h.
 *
 *   km_check pins    the Rust layout numbers as PIN(value, expression) lines
 *                    (driver/tools/offsetof/ddk_abi_pins.h's format), which
 *                    tools/check.sh compiles against the vendored DDK headers
 *   km_check mem     the allocators and page tables (scenario.h); the record
 *                    must equal mem-674ea1cf.rec (the C before step 7,
 *                    record_c.sh)
 *   km_check ccb     the KCCB, the FWCCB service and the freelists; must
 *                    equal ccb-674ea1cf.rec
 *   km_check fwinit-former
 *                    the FWIF structures and the boot wait at the former
 *                    allocation sizes and object base; must equal
 *                    fwinit-674ea1cf.rec
 *   km_check fwinit  the same as the driver builds them (the DDK sizes, the
 *                    objects from the DDK's host main heap); must equal
 *                    fwinit-step7.rec, whose difference from the former is
 *                    README.md's
 *   km_check render  the render service's firmware side against a model of
 *                    the firmware; must equal render-674ea1cf.rec
 *   km_check render-verbose
 *                    the same with PVR_RENDER_VERBOSE (a shorter run); must
 *                    equal render-verbose-674ea1cf.rec
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include <yakogl_drivers.h>
#include "rec.h"
#include "scenario.h"

/* ---- the platform hooks ---- */
static void op_flush(void *ctx, const void *p, size_t n) { (void)ctx; rec_flush(p, n); }
static void op_invd(void *ctx, const void *p, size_t n) { (void)ctx; rec_invd(p, n); }
static void op_fence(void *ctx) { (void)ctx; rec_fence(); }
static void op_busy(void *ctx, uint32_t us) { (void)ctx; rec_busy_wait_us(us); }
static void op_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	(void)ctx;
	printf("L%u %.*s\n", level, (int)len, (const char *)buf);
}
static uint32_t bus_read32(void *ctx, uintptr_t addr) { (void)ctx; return rec_read32(addr); }
static void bus_write32(void *ctx, uintptr_t addr, uint32_t val) { (void)ctx; rec_write32(addr, val); }
static void bus_barrier(void *ctx, uintptr_t addr) { (void)ctx; printf("b 0x%04lx\n", (unsigned long)addr); }

static const struct yd_pvr_km_ops ops = {
	.flush = op_flush, .invd = op_invd, .fence = op_fence, .busy_wait_us = op_busy, .log = op_log,
};
static const struct yd_pvr_bus bus = {
	.read32 = bus_read32, .write32 = bus_write32, .barrier = bus_barrier,
};

/* The layout of driver/src/pvr_internal.h for this carveout. */
static struct yd_pvr_km km;

static void km_setup(void)
{
	rec_init();
	memset(&km, 0, sizeof(km));
	km.cpu = rec_cpu;
	km.phys = REC_PHYS;
	km.size = REC_SIZE;
	km.ops = &ops;
	km.bus = &bus;
	km.regs = 0;
	km.fw_main_size = 4u * 1024 * 1024;
	km.fw_config_off = 0x2000000u - 0x30000u;
	km.pt_area_off = km.fw_main_size + 0x30000u;
	km.pt_area_end = km.pt_area_off + 2u * 1024 * 1024;
	km.pool_off = km.pt_area_end;
	km.pt_next = km.pt_area_off;
	km.pool_next = km.pool_off;
	km.fl_reserve_mult = 4;
}

/* ---- the API over the Rust entry points ---- */
static struct km_bo bo_of(struct yd_pvr_bo b)
{
	return (struct km_bo){ .gpu = b.gpu, .cpu = b.cpu, .heap_off = b.heap_off };
}
static uint32_t r_heap_alloc(uint32_t size, uint32_t align) { return yd_pvr_heap_alloc(&km, size, align); }
static uint8_t *r_heap_cpu(uint32_t off) { return yd_pvr_heap_cpu(&km, off); }
static uint32_t r_heap_fwaddr(uint32_t off) { return yd_pvr_heap_fwaddr(&km, off); }
static uint64_t r_heap_gpu(uint32_t off) { return yd_pvr_heap_gpu(&km, off); }
static uint64_t r_heap_phys(uint32_t off) { return yd_pvr_heap_phys(&km, off); }
static struct km_bo r_bo_alloc(uint64_t base, uint32_t *bump, uint32_t size, uint64_t flags)
{
	return bo_of(yd_pvr_bo_alloc(&km, base, bump, size, flags));
}
static struct km_bo r_bo_alloc_pm(uint64_t base, uint32_t *bump, uint32_t size)
{
	return bo_of(yd_pvr_bo_alloc_pm(&km, base, bump, size));
}
static bool r_bo_rollback(uint64_t base, uint32_t *bump, uint32_t heap_off, const uint8_t *cpu, uint32_t size)
{
	(void)base;
	return yd_pvr_bo_rollback(&km, bump, heap_off, cpu, size);
}
static struct km_bo r_bo_place(uint64_t base, uint32_t start, uint32_t size, uint64_t flags)
{
	return bo_of(yd_pvr_bo_place(&km, base, start, size, flags));
}
static void r_pool_align(uint32_t align) { yd_pvr_pool_align(&km, align); }
static uint64_t r_bo_phys(const struct km_bo *b)
{
	struct yd_pvr_bo x = { .gpu = b->gpu, .cpu = b->cpu, .heap_off = b->heap_off };

	return yd_pvr_bo_phys(&km, &x);
}
static void r_mmu_init(void) { yd_pvr_mmu_init(&km); }
static void r_mmu_map_fw_heap(void) { yd_pvr_mmu_map_fw_heap(&km); }
static void r_mmu_map(uint64_t va, uint64_t phys, uint64_t size, uint64_t flags)
{
	yd_pvr_mmu_map(&km, va, phys, size, flags);
}
static bool r_take_dirty(void) { return yd_pvr_mmu_take_dirty(&km); }
static uint64_t r_lookup(uint64_t va) { return yd_pvr_mmu_lookup(&km, va); }
static unsigned r_audit(const uint64_t *lo, const uint64_t *hi, unsigned n) { return yd_pvr_mmu_audit(&km, lo, hi, n); }
static uint64_t r_pc_phys(void) { return yd_pvr_mmu_pc_phys(&km); }
static void r_heap_set_next(uint32_t off) { km.heap_next = off; }
static void r_ccb_setup(uint32_t kctl, uint32_t kcmds, uint32_t krtn, uint32_t fctl, uint32_t fcmds)
{
	km.kccb_ctl_off = kctl;
	km.kccb_cmds_off = kcmds;
	km.kccb_rtn_off = krtn;
	km.fwccb_ctl_off = fctl;
	km.fwccb_cmds_off = fcmds;
}
static uint32_t r_kccb_post(const uint8_t *cmd) { return yd_pvr_kccb_post(&km, cmd); }
static void r_kccb_doorbell(void) { yd_pvr_kccb_doorbell(&km); }
static void r_kccb_probe_slot(uint32_t slot, const char *why) { yd_pvr_kccb_probe_slot(&km, slot, why); }
static int r_fwccb_service(void) { return yd_pvr_fwccb_service(&km); }
static void r_fl_set_reserve_mult(uint32_t m) { km.fl_reserve_mult = m; }
static void r_fl_pool_init(uint32_t *gen_b, uint32_t pages) { yd_pvr_fl_pool_init(&km, gen_b, pages); }
static uint32_t r_fl_build(uint32_t *gen_b, uint32_t np, uint32_t max, uint32_t grow, uint32_t ready, uint32_t id)
{
	return yd_pvr_fl_build(&km, gen_b, np, max, grow, ready, id);
}
static unsigned r_fl_audit(void) { return yd_pvr_fl_audit(&km); }
static void r_fwinit_build(uint32_t pm_latency_ms, uint32_t core_clock_hz, bool legacy)
{
	/* the former allocation sizes: the CCB controls at 24 bytes, the trace
	 * buffer at 864, the GPU utilisation at 10280 (PVR_LEGACY_SIZE_*) */
	const struct yd_pvr_fwinit cfg = {
		.pm_latency_ms = pm_latency_ms,
		.core_clock_hz = core_clock_hz,
		.flags = 0,
		.ccb_ctl_size = legacy ? 24u : 0u,
		.tracebuf_size = legacy ? 864u : 0u,
		.gpu_util_size = legacy ? 10280u : 0u,
	};

	yd_pvr_fwinit_build(&km, &cfg);
}
static int r_fw_wait_started(void) { return yd_pvr_fw_wait_started(&km); }
static void r_fwinit_offsets(uint32_t o[10])
{
	o[0] = km.kccb_ctl_off;
	o[1] = km.kccb_cmds_off;
	o[2] = km.kccb_rtn_off;
	o[3] = km.fwccb_ctl_off;
	o[4] = km.fwccb_cmds_off;
	o[5] = km.sysinit_off;
	o[6] = km.tracebuf_off;
	o[7] = km.trace_buf_off;
	o[8] = km.hwrinfobuf_off;
	o[9] = km.heap_next;
}
static void r_fl_counters(uint32_t out[5])
{
	out[0] = km.fl_no_grow;
	out[1] = km.fl_grow_reqs;
	out[2] = km.fl_grows_done;
	out[3] = km.fl_pool_peak;
	out[4] = km.pm_bump;
}

static uint32_t g_verbose;

static struct yd_pvr_render rr;

static void r_free_bo(void *ctx, const struct yd_pvr_bo *bo, uint32_t size)
{
	(void)ctx;
	printf("= free_bo gpu 0x%llx size %u\n", (unsigned long long)bo->gpu, size);
}

static const struct yd_pvr_render_ops rops = { .free_bo = r_free_bo };

static void r_fw_offsets_setup(uint32_t hwrinfobuf, uint32_t tracebuf, uint32_t ring)
{
	km.hwrinfobuf_off = hwrinfobuf;
	km.tracebuf_off = tracebuf;
	km.trace_buf_off = ring;
}
static void r_render_cfg(const uint32_t c[10])
{
	rr.cfg.combined_kick = c[0];
	rr.cfg.wait_rering = c[1];
	rr.cfg.wait_repost = c[2];
	rr.cfg.mmu_ack = c[3];
	rr.cfg.mmu_gate = c[4];
	rr.cfg.mmu_ack_probe = c[5];
	rr.cfg.mmu_ack_reping = c[6];
	rr.cfg.hwrt_refuse = c[7];
	rr.cfg.hwrt_cleanup = c[8];
	rr.cfg.hwrt_reinit = c[9];
	rr.cfg.verbose = g_verbose;
}
/* The render service's init as driver/src/pvr_render.c does it, less the
 * static PDS/USC areas: the parameter buffer, the border page, a grow pool,
 * the three freelists, the render context. */
static void r_render_init(void)
{
	struct yd_pvr_render_cfg cfg = rr.cfg;

	memset(&rr, 0, sizeof(rr));
	rr.cfg = cfg;
	rr.ops = &rops;
	rr.last_hs = -1;
	(void)yd_pvr_bo_alloc(&km, SC_GENERAL, &rr.gen_b, 0x200000u, SC_DATA);
	(void)yd_pvr_bo_alloc(&km, SC_GENERAL, &rr.gen_b, 4096, SC_DATA);
	km.fl_reserve_mult = 0;
	yd_pvr_fl_pool_init(&km, &rr.gen_b, 256);
	rr.gfl_off = yd_pvr_fl_build(&km, &rr.gen_b, 2048, 2048 + 256, 1024, 0, 1);
	rr.gfl2_off = yd_pvr_fl_build(&km, &rr.gen_b, 2048, 2048 + 256, 1024, 0, 2);
	rr.fl_off = yd_pvr_fl_build(&km, &rr.gen_b, 26, 32, 0, 0, 3);
	yd_pvr_render_ctx_init(&km, &rr);
	rr.inited = 1;
}
static int r_render_submit(const struct pvr_render_kick *k, uint32_t *fence)
{
	return yd_pvr_render_submit(&km, &rr, k, fence);
}
static int r_render_wait(uint32_t fence, uint32_t timeout_ms)
{
	return yd_pvr_render_wait(&km, &rr, fence, timeout_ms, "frame fence");
}
static uint32_t r_render_completed(void) { return yd_pvr_render_completed(&km, &rr); }
static void r_render_offsets(uint32_t o[8])
{
	o[0] = rr.rc_off;
	o[1] = rr.gcccbctl_off;
	o[2] = rr.gcccb_off;
	o[3] = rr.fcccbctl_off;
	o[4] = rr.fcccb_off;
	o[5] = rr.gufo_off;
	o[6] = rr.fufo_off;
	o[7] = rr.mc_sync_off;
}

static const struct km_api r_api = {
	.heap_alloc = r_heap_alloc,
	.heap_cpu = r_heap_cpu,
	.heap_fwaddr = r_heap_fwaddr,
	.heap_gpu = r_heap_gpu,
	.heap_phys = r_heap_phys,
	.bo_alloc = r_bo_alloc,
	.bo_alloc_pm = r_bo_alloc_pm,
	.bo_rollback = r_bo_rollback,
	.bo_place = r_bo_place,
	.pool_align = r_pool_align,
	.bo_phys = r_bo_phys,
	.mmu_init = r_mmu_init,
	.mmu_map_fw_heap = r_mmu_map_fw_heap,
	.mmu_map = r_mmu_map,
	.mmu_take_dirty = r_take_dirty,
	.mmu_lookup = r_lookup,
	.mmu_audit = r_audit,
	.pc_phys = r_pc_phys,
	.heap_set_next = r_heap_set_next,
	.ccb_setup = r_ccb_setup,
	.kccb_post = r_kccb_post,
	.kccb_doorbell = r_kccb_doorbell,
	.kccb_probe_slot = r_kccb_probe_slot,
	.fwccb_service = r_fwccb_service,
	.fl_set_reserve_mult = r_fl_set_reserve_mult,
	.fl_pool_init = r_fl_pool_init,
	.fl_build = r_fl_build,
	.fl_audit = r_fl_audit,
	.fl_counters = r_fl_counters,
	.fwinit_build = r_fwinit_build,
	.fw_wait_started = r_fw_wait_started,
	.fwinit_offsets = r_fwinit_offsets,
	.fw_offsets_setup = r_fw_offsets_setup,
	.render_init = r_render_init,
	.render_cfg = r_render_cfg,
	.render_submit = r_render_submit,
	.render_wait = r_render_wait,
	.render_completed = r_render_completed,
	.render_offsets = r_render_offsets,
};

static int pins(void)
{
	const uint8_t *e;
	size_t n;
	uint64_t v;

	printf("/* generated by km_check pins: gpu::pvr::fwif */\n");
	for (uint32_t i = 0; yd_pvr_fwif_pin(i, &e, &n, &v); i++) {
		printf("PIN(0x%llxull, %.*s)\n", (unsigned long long)v, (int)n, (const char *)e);
	}
	for (uint32_t i = 0; yd_pvr_mh_tag(i, &e, &n); i++) {
		printf("PIN(%u, RGX_MH_TAG_ENCODING_MH_TAG_%.*s)\n", i, (int)n, (const char *)e);
	}
	for (uint32_t i = 0, a; yd_pvr_fwif_align_check(i, &a); i++) {
		printf("ALIGN(%u, %u)\n", i, a);
	}
	return 0;
}

/* The Rust panic handler's hook (the board's is driver/drivers/common/yd_zephyr.c). */
void yd_panic(const uint8_t *file, size_t file_len, uint32_t line)
{
	printf("FAIL Rust panic at %.*s:%u\n", (int)file_len, (const char *)file, line);
	exit(4);
}

/* The host's precompiled `core` names the personality routine; the crate is
 * panic=abort and never unwinds (as tools/pvr/rgxlayer_check.c). */
void rust_eh_personality(void);
void rust_eh_personality(void)
{
	abort();
}

int main(int argc, char **argv)
{
	if (argc == 2 && !strcmp(argv[1], "pins")) {
		return pins();
	}
	if (argc == 2 && !strcmp(argv[1], "mem")) {
		km_setup();
		scenario_mem_mmu(&r_api);
		rec_dump_memory();
		return 0;
	}
	if (argc == 2 && !strcmp(argv[1], "ccb")) {
		km_setup();
		scenario_ccb(&r_api);
		rec_dump_memory();
		return 0;
	}
	if (argc == 2 && (!strcmp(argv[1], "fwinit-former") || !strcmp(argv[1], "fwinit"))) {
		km_setup();
		scenario_fwinit(&r_api, !strcmp(argv[1], "fwinit-former"));
		rec_dump_memory();
		return 0;
	}
	if (argc == 2 && (!strcmp(argv[1], "render") || !strcmp(argv[1], "render-verbose"))) {
		g_verbose = !strcmp(argv[1], "render-verbose");
		km_setup();
		scenario_render(&r_api, g_verbose);
		rec_dump_memory();
		return 0;
	}
	fprintf(stderr, "usage: km_check pins|mem|ccb|fwinit-former|fwinit|render|render-verbose\n");
	return 2;
}
