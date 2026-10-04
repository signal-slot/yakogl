/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * scenario.h -- what the PowerVR kernel-side checks run on both sides
 * (tools/pvr/km/rec.h). Each recorder fills a struct km_api with its own
 * functions (the former C, the Rust entry points) and calls the scenarios;
 * everything they do and return goes to the record.
 */
#ifndef PVR_KM_SCENARIO_H
#define PVR_KM_SCENARIO_H

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "rec.h"
#include "pvr_render.h"   /* struct pvr_render_kick (driver/src; no OS dependency) */

/* A render-pool block (struct pvr_gpu_bo's fields). */
struct km_bo {
	uint64_t gpu;
	uint8_t *cpu;
	uint32_t heap_off;
};

struct km_api {
	/* gpu::pvr::mem */
	uint32_t (*heap_alloc)(uint32_t size, uint32_t align);
	uint8_t *(*heap_cpu)(uint32_t off);
	uint32_t (*heap_fwaddr)(uint32_t off);
	uint64_t (*heap_gpu)(uint32_t off);
	uint64_t (*heap_phys)(uint32_t off);
	struct km_bo (*bo_alloc)(uint64_t heap_base, uint32_t *bump, uint32_t size, uint64_t flags);
	struct km_bo (*bo_alloc_pm)(uint64_t heap_base, uint32_t *bump, uint32_t size);
	bool (*bo_rollback)(uint64_t heap_base, uint32_t *bump, uint32_t heap_off, const uint8_t *cpu,
			    uint32_t size);
	struct km_bo (*bo_place)(uint64_t heap_base, uint32_t start_off, uint32_t size, uint64_t flags);
	void (*pool_align)(uint32_t align);
	uint64_t (*bo_phys)(const struct km_bo *bo);
	/* gpu::pvr::mmu */
	void (*mmu_init)(void);
	void (*mmu_map_fw_heap)(void);
	void (*mmu_map)(uint64_t va, uint64_t phys, uint64_t size, uint64_t flags);
	bool (*mmu_take_dirty)(void);
	uint64_t (*mmu_lookup)(uint64_t va);
	unsigned (*mmu_audit)(const uint64_t *lo, const uint64_t *hi, unsigned n);
	uint64_t (*pc_phys)(void);
	/* gpu::pvr::ccb, freelist */
	void (*heap_set_next)(uint32_t off);
	void (*ccb_setup)(uint32_t kctl, uint32_t kcmds, uint32_t krtn, uint32_t fctl, uint32_t fcmds);
	uint32_t (*kccb_post)(const uint8_t *cmd);
	void (*kccb_doorbell)(void);
	void (*kccb_probe_slot)(uint32_t slot, const char *why);
	int (*fwccb_service)(void);
	void (*fl_set_reserve_mult)(uint32_t mult);
	void (*fl_pool_init)(uint32_t *gen_b, uint32_t pages);
	uint32_t (*fl_build)(uint32_t *gen_b, uint32_t npages, uint32_t maxpages, uint32_t grow, uint32_t ready,
			     uint32_t id);
	unsigned (*fl_audit)(void);
	/* no_grow, grow_reqs, grows_done, pool_peak, pm bump */
	void (*fl_counters)(uint32_t out[5]);
	/* gpu::pvr::fwinit: legacy = the former allocation sizes */
	void (*fwinit_build)(uint32_t pm_latency_ms, uint32_t core_clock_hz, bool legacy);
	int (*fw_wait_started)(void);
	/* kccb ctl, kccb cmds, kccb rtn, fwccb ctl, fwccb cmds, sysinit, tracebuf,
	 * trace ring, hwrinfobuf, heap next */
	void (*fwinit_offsets)(uint32_t out[10]);
	/* gpu::pvr::render */
	void (*fw_offsets_setup)(uint32_t hwrinfobuf, uint32_t tracebuf, uint32_t ring);
	void (*render_init)(void);
	void (*render_cfg)(const uint32_t cfg[10]);
	int (*render_submit)(const struct pvr_render_kick *k, uint32_t *fence);
	int (*render_wait)(uint32_t fence, uint32_t timeout_ms);
	uint32_t (*render_completed)(void);
	/* rc, gcccb ctl, gcccb, fcccb ctl, fcccb, gufo, fufo, mc sync */
	void (*render_offsets)(uint32_t out[8]);
};

/* The heaps and leaf flags (rgxheapconfig.h, rgxmmudefs_km.h; the host check
 * pins the Rust copies, so plain numbers here). */
#define SC_GENERAL 0x8000000000ull
#define SC_PDS     0xDA00000000ull
#define SC_USC     0xE000000000ull
#define SC_PMMETA  0xFA00000000ull
#define SC_FWHEAP  0xE1C0000000ull
#define SC_DATA    0x8ull                   /* SLC bypass */
#define SC_PM      0x4000000000000008ull    /* PM_META_PROTECT | SLC bypass */
#define SC_PM_C    0x4000000000000000ull    /* PM_META_PROTECT */

static void sc_bo(const struct km_api *a, const char *what, struct km_bo b)
{
	if (b.cpu) {
		printf("= %s gpu 0x%llx cpu 0x%llx heap_off 0x%x phys 0x%llx\n", what,
		       (unsigned long long)b.gpu, rec_off(b.cpu), b.heap_off,
		       (unsigned long long)a->bo_phys(&b));
	} else {
		printf("= %s none\n", what);
	}
}

static void sc_lookup(const struct km_api *a, uint64_t va)
{
	printf("= lookup 0x%llx 0x%016llx\n", (unsigned long long)va, (unsigned long long)a->mmu_lookup(va));
}

/* The carveout's allocators and the page tables, in the order the driver
 * uses them at boot and in the render service's init, plus the edges: a
 * mapping across a directory and a catalogue boundary, a remap, a rollback
 * that may and two that may not, placed spans, an exhausted pool, an audit
 * with and without the allowed range. */
static void scenario_mem_mmu(const struct km_api *a)
{
	uint32_t gen = 0, pds = 0, usc = 0, pm = 0;
	static const struct { uint32_t size, align; } objs[] = {
		{ 24, 8 }, { 128 * 160, 8 }, { 128 * 4, 8 }, { 24, 8 }, { 32 * 160, 8 },
		{ 4, 8 }, { 2336, 8 }, { 3896, 8 }, { 1088, 8 }, { 160, 8 }, { 10280, 8 },
		{ 864, 8 }, { 48000, 8 }, { 4096, 0x1000 }, { 40, 8 }, { 512, 8 },
		{ 1272, 8 }, { 2048, 8 }, { 64, 8 }, { 100, 128 }, { 32768, 128 },
	};

	printf("= scenario mem+mmu\n");
	for (unsigned i = 0; i < sizeof(objs) / sizeof(objs[0]); i++) {
		uint32_t off = a->heap_alloc(objs[i].size, objs[i].align);

		printf("= heap_alloc %u %u -> 0x%x cpu 0x%llx fw 0x%08x gpu 0x%llx phys 0x%llx\n", objs[i].size,
		       objs[i].align, off, rec_off(a->heap_cpu(off)), a->heap_fwaddr(off),
		       (unsigned long long)a->heap_gpu(off), (unsigned long long)a->heap_phys(off));
	}
	/* the config region (connection control, OS init, system init) */
	for (uint32_t off = 0x1FD0000u; off <= 0x1FF0000u; off += 0x10000u) {
		printf("= config 0x%x cpu 0x%llx fw 0x%08x gpu 0x%llx phys 0x%llx\n", off,
		       rec_off(a->heap_cpu(off)), a->heap_fwaddr(off), (unsigned long long)a->heap_gpu(off),
		       (unsigned long long)a->heap_phys(off));
	}

	a->mmu_init();
	printf("= pc_phys 0x%llx\n", (unsigned long long)a->pc_phys());
	a->mmu_map_fw_heap();
	printf("= dirty %d\n", a->mmu_take_dirty());
	printf("= dirty %d\n", a->mmu_take_dirty());

	/* the render service's init */
	sc_bo(a, "pbuf", a->bo_alloc(SC_GENERAL, &gen, 0x200000u, SC_DATA));
	sc_bo(a, "border", a->bo_alloc(SC_GENERAL, &gen, 4096, SC_DATA));
	sc_bo(a, "usc0", a->bo_alloc(SC_USC, &usc, 4096, SC_DATA));
	sc_bo(a, "pds0", a->bo_alloc(SC_PDS, &pds, 4096, SC_DATA));
	for (int n = 1; n <= 4; n++) {
		sc_bo(a, "eot", a->bo_alloc(SC_USC, &usc, 40u * n + 4u, SC_DATA));
	}
	for (int n = 0; n < 2; n++) {
		sc_bo(a, "tilebuf", a->bo_alloc(SC_GENERAL, &gen, 16u * 16u * 4u * 4u, SC_DATA));
	}
	a->pool_align(0x4000);
	sc_bo(a, "fl pool", a->bo_alloc(SC_GENERAL, &gen, 1024u * 4096u, SC_DATA));
	for (int f = 0; f < 3; f++) {
		a->pool_align(0x4000);
		sc_bo(a, "fl backing", a->bo_alloc(SC_GENERAL, &gen, (f < 2 ? 1536u : 26u) * 4096u, SC_DATA));
		sc_bo(a, "fl array", a->bo_alloc_pm(SC_PMMETA, &pm, (f < 2 ? 4096u : 32u) * 4u));
	}
	sc_bo(a, "vdm_cs", a->bo_alloc(SC_GENERAL, &gen, 4096, SC_DATA));
	sc_bo(a, "mlist", a->bo_alloc_pm(SC_PMMETA, &pm, 0x1000));
	sc_bo(a, "fl cached", a->bo_alloc(SC_GENERAL, &gen, 3 * 4096, SC_PM_C));
	printf("= bumps gen 0x%x pds 0x%x usc 0x%x pm 0x%x\n", gen, pds, usc, pm);

	/* rollback: the newest block (yes), then a block under it (no), then the
	 * newest of one heap after another heap bumped the pool (no) */
	{
		struct km_bo x = a->bo_alloc(SC_GENERAL, &gen, 5000, SC_DATA);
		struct km_bo y = a->bo_alloc(SC_GENERAL, &gen, 4096, SC_DATA);
		struct km_bo z;

		sc_bo(a, "x", x);
		sc_bo(a, "y", y);
		bool r;

		r = a->bo_rollback(SC_GENERAL, &gen, y.heap_off, y.cpu, 4096);
		printf("= rollback y %d gen 0x%x\n", r, gen);
		r = a->bo_rollback(SC_GENERAL, &gen, x.heap_off + 0x1000, x.cpu, 4096);
		printf("= rollback x? %d gen 0x%x\n", r, gen);
		z = a->bo_alloc(SC_PDS, &pds, 4096, SC_DATA);
		sc_bo(a, "z", z);
		r = a->bo_rollback(SC_GENERAL, &gen, x.heap_off, x.cpu, 5000);
		printf("= rollback x %d gen 0x%x\n", r, gen);
		r = a->bo_rollback(SC_PDS, &pds, z.heap_off, z.cpu, 4096);
		printf("= rollback z %d pds 0x%x\n", r, pds);
		r = a->bo_rollback(SC_GENERAL, &gen, x.heap_off, x.cpu, 5000);
		printf("= rollback x %d gen 0x%x\n", r, gen);
		r = a->bo_rollback(SC_GENERAL, NULL, 0, x.cpu, 4096);
		printf("= rollback null %d\n", r);
		r = a->bo_rollback(SC_GENERAL, &gen, gen, rec_cpu, 0);
		printf("= rollback zero %d\n", r);
	}

	/* placed spans */
	sc_bo(a, "place vs", a->bo_place(SC_GENERAL, 0x92000u + 0x40u, 0x1200u, SC_DATA));
	sc_bo(a, "place pds", a->bo_place(SC_PDS, 0x7f0u, 0x20u, SC_DATA));
	sc_bo(a, "place pm", a->bo_place(SC_PMMETA, 0x100000u, 0x10u, SC_PM));

	/* across a directory (2 MiB) and a catalogue (1 GiB) boundary */
	gen = 0x3FFFE000u;
	sc_bo(a, "cross", a->bo_alloc(SC_GENERAL, &gen, 0x4000u, SC_DATA));
	gen = 0x401FF000u;
	sc_bo(a, "cross pd", a->bo_alloc(SC_GENERAL, &gen, 0x3000u, SC_DATA));
	/* a remap: the same VA onto other pages */
	a->mmu_map(SC_GENERAL + 0x3000u, REC_PHYS + 0x3000000u, 0x2000u, SC_PM_C);
	/* an external range (a scan-out buffer outside the carveout) */
	a->mmu_map(SC_GENERAL + 0x20000000u, 0x100000000ull, 0x3000u, SC_DATA);
	printf("= dirty %d\n", a->mmu_take_dirty());

	/* an exhausted pool */
	sc_bo(a, "huge", a->bo_alloc(SC_GENERAL, &gen, 0x7F000000u, SC_DATA));
	sc_bo(a, "rest", a->bo_alloc(SC_GENERAL, &gen, (uint32_t)REC_SIZE, SC_DATA));

	sc_lookup(a, SC_FWHEAP);
	sc_lookup(a, SC_FWHEAP + 0x3FF000u);
	sc_lookup(a, SC_FWHEAP + 0x400000u);
	sc_lookup(a, SC_FWHEAP + 0x1FD0000u);
	sc_lookup(a, SC_FWHEAP + 0x1FFF000u);
	sc_lookup(a, SC_GENERAL);
	sc_lookup(a, SC_GENERAL + 0x3000u);
	sc_lookup(a, SC_GENERAL + 0x3FFFF000u);
	sc_lookup(a, SC_GENERAL + 0x40000000u);
	sc_lookup(a, SC_GENERAL + 0x40200000u);
	sc_lookup(a, SC_GENERAL + 0x20002000u);
	sc_lookup(a, SC_PMMETA + 0xc000u);
	sc_lookup(a, SC_PMMETA + 0x100000u);
	sc_lookup(a, SC_USC + 0x5000u);
	sc_lookup(a, 0x1234567000ull);
	sc_lookup(a, SC_PDS + 0x40000000u);

	{
		uint64_t lo = 0x100000000ull, hi = 0x100003000ull;

		printf("= audit %u\n", a->mmu_audit(NULL, NULL, 0));
		printf("= audit allowed %u\n", a->mmu_audit(&lo, &hi, 1));
	}
}

/* ---- the command rings and the freelists ---- */

#define SC_KCCB_SLOTS 128u
#define SC_FWCCB_SLOTS 32u
#define SC_CMD 160u

static uint8_t *sc_kctl;   /* KCCB control block, for the firmware tick */

/* The firmware takes everything posted (its read offset catches up). */
static void sc_fw_consume_kccb(void)
{
	memcpy(sc_kctl + 4, sc_kctl + 0, 4);
}

static uint8_t *sc_fctl, *sc_fcmds;

static void sc_w32(uint8_t *p, uint32_t v) { memcpy(p, &v, 4); }
static uint32_t sc_r32(const uint8_t *p) { uint32_t v; memcpy(&v, p, 4); return v; }
static void sc_w64(uint8_t *p, uint64_t v) { memcpy(p, &v, 8); }

/* The firmware posts one FWCCB command: type, then 64-bit words at data
 * offset 8 (the freelist ID, or a count and IDs), or raw 32-bit words. */
static uint8_t *sc_fw_post(uint32_t type)
{
	uint32_t wo = sc_r32(sc_fctl + 0);
	uint8_t *c = sc_fcmds + (size_t)wo * SC_CMD;

	memset(c, 0, SC_CMD);
	sc_w32(c, type);
	sc_w32(sc_fctl + 0, (wo + 1) & (SC_FWCCB_SLOTS - 1));
	return c;
}

static void sc_fw_grow(uint64_t id)
{
	sc_w64(sc_fw_post(0x2ABC0067u) + 8, id);
}

static void sc_counters(const struct km_api *a)
{
	uint32_t c[5];

	a->fl_counters(c);
	printf("= fl no_grow %u grow_reqs %u grows_done %u pool_peak %u pm_bump 0x%x\n", c[0], c[1], c[2], c[3], c[4]);
}

static void scenario_ccb(const struct km_api *a)
{
	uint32_t gen = 0, kctl, kcmds, krtn, fctl, fcmds, fl[8];
	uint8_t cmd[SC_CMD];

	printf("= scenario ccb+freelist\n");
	/* where the firmware image ends (objs from 0xa1000) */
	a->heap_set_next(0xa1000u);
	kctl = a->heap_alloc(24, 8);
	kcmds = a->heap_alloc(SC_KCCB_SLOTS * SC_CMD, 8);
	krtn = a->heap_alloc(SC_KCCB_SLOTS * 4, 8);
	fctl = a->heap_alloc(24, 8);
	fcmds = a->heap_alloc(SC_FWCCB_SLOTS * SC_CMD, 8);
	sc_kctl = a->heap_cpu(kctl);
	sc_fctl = a->heap_cpu(fctl);
	sc_fcmds = a->heap_cpu(fcmds);
	sc_w32(sc_kctl + 8, SC_KCCB_SLOTS - 1);
	sc_w32(sc_fctl + 8, SC_FWCCB_SLOTS - 1);
	a->ccb_setup(kctl, kcmds, krtn, fctl, fcmds);
	a->mmu_init();
	rec_tick = sc_fw_consume_kccb;

	sc_bo(a, "pbuf", a->bo_alloc(SC_GENERAL, &gen, 0x200000u, SC_DATA));
	/* the render service's big-carveout lists: a shared pool, no reserve */
	a->fl_set_reserve_mult(0);
	a->fl_pool_init(&gen, 1024);
	fl[1] = a->fl_build(&gen, 2048, 2048 + 1024, 1024, 0, 1);
	fl[2] = a->fl_build(&gen, 2048, 2048 + 1024, 1024, 0, 2);
	fl[3] = a->fl_build(&gen, 26, 32, 0, 0, 3);
	/* the small-carveout sizing, with a reserve per list */
	a->fl_set_reserve_mult(4);
	fl[4] = a->fl_build(&gen, 448, 4096, 256, 64, 4);
	fl[5] = a->fl_build(&gen, 16, 64, 8, 0, 5);
	fl[6] = a->fl_build(&gen, 8, 16, 4, 0, 6);
	fl[7] = a->fl_build(&gen, 8, 16, 0, 0, 7);   /* past the tracked six */
	for (int i = 1; i <= 7; i++) {
		printf("= fl %d heap 0x%x fw 0x%08x\n", i, fl[i], a->heap_fwaddr(fl[i]));
	}
	sc_counters(a);

	/* 130 commands: the ring wraps */
	for (uint32_t i = 0; i < 130; i++) {
		for (uint32_t j = 0; j < SC_CMD; j++) {
			cmd[j] = (uint8_t)(i * 7u + j);
		}
		sc_w32(cmd, 0x2ABC0000u | i);
		printf("= post %u slot %u\n", i, a->kccb_post(cmd));
	}
	a->kccb_doorbell();
	a->kccb_probe_slot(5, "post");
	a->kccb_probe_slot(0, "miss");

	/* the firmware's requests */
	sc_fw_grow(1);   /* from the pool */
	sc_fw_grow(4);   /* from its own reserve */
	sc_fw_grow(3);   /* grow 0: no grow */
	sc_fw_grow(9);   /* unknown */
	for (int i = 0; i < 5; i++) {
		sc_fw_grow(5);   /* four from its reserve, then the pool */
	}
	{
		uint8_t *c = sc_fw_post(0x2ABC0068u);

		sc_w32(c + 8, 3);
		sc_w64(c + 16, 1);
		sc_w64(c + 24, 3);
		sc_w64(c + 32, 9);
		c = sc_fw_post(0x2ABC0068u);
		sc_w32(c + 8, 25);   /* clamped to 18 */
		for (int i = 0; i < 18; i++) {
			sc_w64(c + 16 + 8 * i, (uint64_t)(i % 7) + 1);
		}
		c = sc_fw_post(0x2ABC0069u);
		for (uint32_t i = 0; i < 10; i++) {
			sc_w32(c + 8 + 4 * i, 0x100u + i);
		}
		c = sc_fw_post(0x00000069u);   /* the low byte is what is compared */
		sc_w32(c + 8, 7);
		sc_fw_post(0x12345678u);
	}
	printf("= serviced %d\n", a->fwccb_service());
	sc_counters(a);
	/* round the FWCCB ring a few times: list 6 until the pool is spent */
	for (int round = 0; round < 6; round++) {
		for (int i = 0; i < 8; i++) {
			sc_fw_grow(round & 1 ? 2 : 6);
		}
		printf("= serviced %d\n", a->fwccb_service());
	}
	printf("= serviced %d\n", a->fwccb_service());
	sc_counters(a);

	printf("= fl audit %u\n", a->fl_audit());
	{
		/* list 3's PFN array (through its RGXFWIF_FREELIST and the page
		 * tables): an entry below the stack that is not zero, one in the
		 * stack that is not a page of its backing */
		uint64_t arr_gpu, pte;
		uint8_t *arr;

		memcpy(&arr_gpu, a->heap_cpu(fl[3]), 8);
		pte = a->mmu_lookup(arr_gpu);
		arr = rec_cpu + ((pte & 0x000000FFFFFFF000ull) - REC_PHYS);
		printf("= fl3 arr 0x%llx pte 0x%llx\n", (unsigned long long)arr_gpu, (unsigned long long)pte);
		sc_w32(arr + 0, 0xdeadu);
		sc_w32(arr + 40, 0xbeefu);
	}
	printf("= fl audit %u\n", a->fl_audit());
	rec_tick = NULL;
}

/* ---- the FWIF structures and the boot wait ---- */

static uint8_t *sc_started;
static unsigned sc_ticks;

/* The firmware sets bFirmwareStarted on its third tick. */
static void sc_fw_start(void)
{
	if (++sc_ticks == 3) {
		sc_started[0] = 1;
	}
}

/* Where each pointer of the OS init and the system init lands (firmware
 * address -> heap offset), so a layout change reads as a short table. */
static void sc_fw_table(const struct km_api *a, const char *name, const uint8_t *base, unsigned off)
{
	uint32_t fa = sc_r32(base + off);

	printf("= %-28s fw 0x%08x heap 0x%x\n", name, fa, fa & 0x3FFFFFFFu);
	(void)a;
}

static void scenario_fwinit(const struct km_api *a, bool legacy)
{
	uint32_t o[10];
	const uint8_t *osi, *sys;

	printf("= scenario fwinit%s\n", legacy ? " (former sizes)" : "");
	/* where the firmware image ends: the former packing, or the DDK's host
	 * main heap */
	a->heap_set_next(legacy ? 0xa1000u : 0xc0000u);
	a->fwinit_build(5, 800000000u, legacy);
	a->fwinit_offsets(o);
	printf("= kccb ctl 0x%x cmds 0x%x rtn 0x%x fwccb ctl 0x%x cmds 0x%x sysinit 0x%x tracebuf 0x%x ring 0x%x hwrinfo 0x%x heap_next 0x%x\n",
	       o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7], o[8], o[9]);
	osi = a->heap_cpu(o[5] - 0x10000u);
	sys = a->heap_cpu(o[5]);
	sc_fw_table(a, "osinit.KernelCCBCtl", osi, 0);
	sc_fw_table(a, "osinit.KernelCCB", osi, 4);
	sc_fw_table(a, "osinit.KernelCCBRtnSlots", osi, 8);
	sc_fw_table(a, "osinit.FirmwareCCBCtl", osi, 12);
	sc_fw_table(a, "osinit.FirmwareCCB", osi, 16);
	sc_fw_table(a, "osinit.HWRInfoBufCtl", osi, 28);
	sc_fw_table(a, "osinit.FwOsData", osi, 36);
	sc_fw_table(a, "osinit.HwPerfData", osi, 144);
	sc_fw_table(a, "osinit.HwPerfRuntimeCfg", osi, 148);
	sc_fw_table(a, "osinit.GpuUtilFWCtl", osi, 152);
	sc_fw_table(a, "sysinit.RuntimeCfg", sys, 628);
	sc_fw_table(a, "sysinit.TraceBufCtl", sys, 632);
	sc_fw_table(a, "sysinit.FwSysData", sys, 636);
	sc_fw_table(a, "sysinit.HWPerfCtl", sys, 644);
	sc_fw_table(a, "sysinit.AlignChecks", sys, 648);
	printf("= sysinit.FaultPhysAddr 0x%llx\n", (unsigned long long)(sc_r32(sys + 560) | (uint64_t)sc_r32(sys + 564) << 32));

	/* the firmware boots on its third tick */
	sc_started = a->heap_cpu(o[5]) + 664;
	sc_ticks = 0;
	rec_tick = sc_fw_start;
	printf("= wait %d\n", a->fw_wait_started());
	rec_tick = NULL;
	/* and now it does not: the timeout dump of its trace and assert record */
	sc_started[0] = 0;
	{
		uint8_t *ring = a->heap_cpu(o[7]);
		uint8_t *tb = a->heap_cpu(o[6]);

		for (uint32_t i = 0; i < 64; i++) {
			sc_w32(ring + 4 * i, 0x1000u + i * 0x11u);
		}
		sc_w32(tb + 8, 42);
		memset(tb + 24, 'p', 200);
		memcpy(tb + 24, "rgxfw_main.c", 12);
		memset(tb + 24 + 200, 'i', 200);
		memcpy(tb + 24 + 200, "bad things", 10);
		sc_w32(tb + 24 + 400, 1234);
	}
	printf("= wait %d\n", a->fw_wait_started());
}

/* ---- the render service, against a model of the firmware ---- */

/* The model: on every busy-wait tick (and on a doorbell write) it takes the
 * KCCB commands posted since the last time and acts on them the way the
 * firmware's protocol says: an MMUCACHE writes its sync value, a kick sets
 * its context's client CCB write offset and runs that CCB (an UPDATE writes
 * its UFO, a FENCE that is not met stops the CCB there), a CLEANUP answers in
 * its return slot. Knobs make it misbehave on purpose. */
static struct {
	uint32_t kctl, kcmds, krtn, rc_fa;
	uint32_t ctl[2], ring[2];       /* geometry, fragment */
	uint32_t hwrinfo;
	bool stopped;                   /* takes nothing */
	unsigned drop_kicks;            /* kicks consumed and not applied */
	unsigned drop_mmu;              /* MMUCACHE consumed without its sync */
	unsigned defer_mmu;             /* MMUCACHE sync held until the next doorbell */
	uint32_t deferred_sync_off, deferred_sync_val;
	bool doorbell;
	bool cleanup_busy;
	unsigned hwr_on_3d;             /* the next fragment run raises a HWR */
	const struct km_api *a;
} fw;

static uint8_t *fw_heap(uint32_t fa)
{
	return fw.a->heap_cpu(fa & 0x3FFFFFFFu);
}

static void fw_run_ccb(int which)
{
	uint8_t *ctl = fw.a->heap_cpu(fw.ctl[which]);
	uint8_t *ring = fw.a->heap_cpu(fw.ring[which]);

	while (sc_r32(ctl + 4) != sc_r32(ctl + 0)) {
		uint32_t rd = sc_r32(ctl + 4);
		const uint8_t *h = ring + rd;
		uint32_t type = sc_r32(h), size = sc_r32(h + 4);

		if (type == 0x2ABC00D4u) { /* FENCE */
			if (sc_r32(fw_heap(sc_r32(h + 16))) != sc_r32(h + 20)) {
				return;
			}
		} else if (type == 0x2ABC00D5u) { /* UPDATE */
			if (which == 1 && fw.hwr_on_3d) {
				uint8_t *hib = fw.a->heap_cpu(fw.hwrinfo);

				fw.hwr_on_3d--;
				sc_w32(hib + 2176, sc_r32(hib + 2176) + 1);
				sc_w32(hib + 64, 0x00000040u);           /* event status */
				sc_w32(hib + 72, 2u);                    /* HWR type */
				sc_w32(hib + 76, 3u);                    /* DM 3D */
				sc_w32(hib + 60, 7u);                    /* HWR number */
				sc_w32(hib + 68, 0x10u);                 /* recovery flags */
				sc_w64(hib + 0, 0x001032800001c0f0ull);  /* BIF req: read, tag 0x32 */
				sc_w64(hib + 8, 0x0000000000010001ull);  /* BIF MMU status */
				sc_w64(hib + 16, 0x00000000c0430000ull); /* PC */
			}
			sc_w32(fw_heap(sc_r32(h + 16)), sc_r32(h + 20));
		}
		sc_w32(ctl + 4, (rd + 16u + size) & (32u * 1024u - 1u));
	}
}

static void fw_kick(const uint8_t *data)
{
	int which = sc_r32(data) == fw.rc_fa ? 0 : 1;

	sc_w32(fw.a->heap_cpu(fw.ctl[which]) + 0, sc_r32(data + 4));
	fw_run_ccb(which);
}

static void fw_step(void)
{
	uint8_t *kctl;

	if (!fw.a || fw.stopped) {
		return;
	}
	kctl = fw.a->heap_cpu(fw.kctl);
	if (fw.doorbell && fw.deferred_sync_off) {
		sc_w32(fw.a->heap_cpu(fw.deferred_sync_off), fw.deferred_sync_val);
		fw.deferred_sync_off = 0;
	}
	fw.doorbell = false;
	while (sc_r32(kctl + 4) != sc_r32(kctl + 0)) {
		uint32_t rd = sc_r32(kctl + 4);
		const uint8_t *c = fw.a->heap_cpu(fw.kcmds) + (size_t)rd * SC_CMD;
		uint32_t type = sc_r32(c);

		if (type == 0x2ABC0066u) { /* MMUCACHE */
			if (fw.drop_mmu) {
				fw.drop_mmu--;
			} else if (fw.defer_mmu) {
				fw.defer_mmu--;
				fw.deferred_sync_off = sc_r32(c + 12) & 0x3FFFFFFFu;
				fw.deferred_sync_val = sc_r32(c + 16);
			} else {
				sc_w32(fw_heap(sc_r32(c + 12)), sc_r32(c + 16));
			}
		} else if (type == 0x2ABC0065u || type == 0x2ABC0071u) { /* KICK, COMBINED */
			if (fw.drop_kicks) {
				fw.drop_kicks--;
			} else if (type == 0x2ABC0065u) {
				fw_kick(c + 8);
			} else {
				fw_kick(c + 8);
				fw_kick(c + 36);
			}
		} else if (type == 0x2ABC0069u) { /* CLEANUP */
			sc_w32(fw.a->heap_cpu(fw.krtn) + 4 * rd, fw.cleanup_busy ? 3u : 1u);
		}
		sc_w32(kctl + 4, (rd + 1) & (SC_KCCB_SLOTS - 1));
	}
}

static void fw_doorbell(uintptr_t reg, uint32_t val)
{
	(void)val;
	if (reg == 0x0b00u) {
		fw.doorbell = true;
	}
}

static const uint32_t sc_cfg_default[10] = {
	/* combined_kick, wait_rering, wait_repost, mmu_ack, mmu_gate,
	 * mmu_ack_probe, mmu_ack_reping, hwrt_refuse, hwrt_cleanup, hwrt_reinit */
	1, 0, 1, 1, 1, 0, 0, 0, 0, 0,
};

static uint32_t sc_cfg[10];

static void sc_set(const struct km_api *a, int i, uint32_t v)
{
	sc_cfg[i] = v;
	a->render_cfg(sc_cfg);
}

static struct pvr_render_kick sc_kick;

static void sc_kick_init(uint64_t vdm)
{
	memset(&sc_kick, 0, sizeof(sc_kick));
	sc_kick.width = 1024;
	sc_kick.height = 600;
	sc_kick.samples = 1;
	sc_kick.geom.vdm_ctrl_stream_base = vdm;
	sc_kick.geom.ppp_ctrl = 0x00000123u;
	sc_kick.geom.te_psg = 0x00000456u;
	sc_kick.frag.usc_pixel_output_ctrl = 0x11u;
	for (int i = 0; i < 4; i++) {
		sc_kick.frag.usc_clear_register[i] = 0x2000u + i;
	}
	sc_kick.frag.isp_bgobjdepth = 0x3f800000u;
	sc_kick.frag.isp_bgobjvals = 0x22u;
	sc_kick.frag.isp_aa = 0x33u;
	sc_kick.frag.isp_ctl = 0x44u;
	sc_kick.frag.event_pixel_pds_info = 0x55u;
	sc_kick.frag.event_pixel_pds_data = 0x66u;
	for (int i = 0; i < 3; i++) {
		sc_kick.frag.pbe_word[i][0] = 0x1111000000000000ull + i;
		sc_kick.frag.pbe_word[i][1] = 0x2222000000000000ull + i;
	}
	sc_kick.frag.num_rt = 1;
	for (int i = 0; i < 3; i++) {
		sc_kick.frag.pds_bgnd[i] = 0x7000000000ull + i;
	}
	sc_kick.frag.isp_scissor_base = 0x8000300000ull;
	sc_kick.frag.isp_dbias_base = 0x8000301000ull;
	sc_kick.frag.isp_zlsctl = 0x9u;
	sc_kick.frag.isp_zload_store_base = 0x8000400000ull;
	sc_kick.frag.isp_stencil_load_store_base = 0x8000500000ull;
	sc_kick.frag.cmd_flags = 0x180u;
}

static uint32_t sc_submit(const struct km_api *a, uint32_t w, uint32_t h, uint32_t samples)
{
	uint32_t fence = 0;
	int rc;

	sc_kick.width = w;
	sc_kick.height = h;
	sc_kick.samples = samples;
	rc = a->render_submit(&sc_kick, &fence);
	printf("= submit %ux%u x%u rc %d fence %u\n", w, h, samples, rc, fence);
	return fence;
}

static void sc_wait(const struct km_api *a, uint32_t fence, uint32_t timeout)
{
	int rc = a->render_wait(fence, timeout);

	printf("= wait %u rc %d completed %u\n", fence, rc, a->render_completed());
}

static void scenario_render(const struct km_api *a, bool verbose_subset)
{
	uint32_t kctl, kcmds, krtn, fctl, fcmds, hwrinfo, tbuf, ring, o[8];
	uint32_t vbump = 0x30000000u, pbump = 0x10000000u, f = 0;
	struct km_bo vdm, pds;

	printf("= scenario render%s\n", verbose_subset ? " (verbose)" : "");
	a->heap_set_next(0xa1000u);
	kctl = a->heap_alloc(24, 8);
	kcmds = a->heap_alloc(SC_KCCB_SLOTS * SC_CMD, 8);
	krtn = a->heap_alloc(SC_KCCB_SLOTS * 4, 8);
	fctl = a->heap_alloc(24, 8);
	fcmds = a->heap_alloc(SC_FWCCB_SLOTS * SC_CMD, 8);
	sc_w32(a->heap_cpu(kctl) + 8, SC_KCCB_SLOTS - 1);
	sc_w32(a->heap_cpu(fctl) + 8, SC_FWCCB_SLOTS - 1);
	a->ccb_setup(kctl, kcmds, krtn, fctl, fcmds);
	sc_fctl = a->heap_cpu(fctl);
	sc_fcmds = a->heap_cpu(fcmds);
	hwrinfo = a->heap_alloc(2336, 8);
	tbuf = a->heap_alloc(864, 8);
	ring = a->heap_alloc(48000, 8);
	a->fw_offsets_setup(hwrinfo, tbuf, ring);
	sc_w32(a->heap_cpu(tbuf) + 8, 36);
	sc_w32(a->heap_cpu(tbuf) + 12, 2);
	for (uint32_t i = 0; i < 40; i++) {
		sc_w32(a->heap_cpu(ring) + 4 * i, 0xf0000000u | i);
	}
	a->mmu_init();
	memcpy(sc_cfg, sc_cfg_default, sizeof(sc_cfg));
	a->render_cfg(sc_cfg);
	a->render_init();
	a->render_offsets(o);
	printf("= render rc 0x%x gcccb ctl 0x%x ring 0x%x fcccb ctl 0x%x ring 0x%x gufo 0x%x fufo 0x%x mc 0x%x\n",
	       o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7]);

	/* a VDM stream (13 words) and a PDS data segment for the mapping dump */
	vdm = a->bo_alloc(SC_GENERAL, &vbump, 4096, SC_DATA);
	pds = a->bo_alloc(SC_PDS, &pbump, 4096, SC_DATA);
	{
		uint32_t w[13] = { 0x80u, 0x00301000u, 0, (uint32_t)pds.heap_off + 0x40u, (uint32_t)pds.heap_off + 0x80u,
				   0, 0, 0x80u, 0x00302000u, 9, 10, 11, 12 };
		uint32_t d[8] = { 0x1000u, 1, 2, 3, 0x00303000u, 0x80u, 6, 7 };

		memcpy(vdm.cpu, w, sizeof(w));
		memcpy(pds.cpu + 0x40, d, sizeof(d));
	}
	sc_kick_init(vdm.gpu);

	memset(&fw, 0, sizeof(fw));
	fw.a = a;
	fw.kctl = kctl;
	fw.kcmds = kcmds;
	fw.krtn = krtn;
	fw.rc_fa = 0xC0000000u | o[0];
	fw.ctl[0] = o[1];
	fw.ring[0] = o[2];
	fw.ctl[1] = o[3];
	fw.ring[1] = o[4];
	fw.hwrinfo = hwrinfo;
	rec_tick = fw_step;
	rec_wtick = fw_doorbell;

	/* A: the first renders, a pair built and reused */
	for (int i = 0; i < 3; i++) {
		sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	}
	if (verbose_subset) {
		/* H: a stall (the timeout dump) and recovery */
		fw.stopped = true;
		f = sc_submit(a, 1024, 600, 1);
		sc_wait(a, f, 25);
		fw.stopped = false;
		sc_wait(a, f, 100);
		goto out;
	}
	/* B: other sizes, multisampling, three targets, separate kicks */
	sc_wait(a, sc_submit(a, 64, 64, 4), 100);
	sc_kick.frag.num_rt = 3;
	sc_wait(a, sc_submit(a, 300, 200, 2), 100);
	sc_kick.frag.num_rt = 5; /* clamped to three */
	sc_wait(a, sc_submit(a, 300, 200, 2), 100);
	sc_kick.frag.num_rt = 1;
	sc_set(a, 0, 0);
	sc_wait(a, sc_submit(a, 1024, 600, 0), 100);
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 0, 1);

	/* C: eighteen back to back: the member-in-flight and UFO-ring waits */
	for (int i = 0; i < 18; i++) {
		f = sc_submit(a, 1024, 600, 1);
	}
	sc_wait(a, f, 100);

	/* D: a kick the firmware takes and does not apply: the re-post; then
	 * with the doorbell re-ring too */
	fw.drop_kicks = 1;
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 1, 1);
	fw.drop_kicks = 1;
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 1, 0);

	/* E: the MMUCACHE acknowledgement: seen, missed (with the probes),
	 * released by a re-ring */
	sc_set(a, 4, 0); /* mmu_gate off: every submit invalidates */
	sc_set(a, 5, 1); /* mmu_ack_probe */
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	fw.drop_mmu = 1;
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 6, 1); /* mmu_ack_reping 1 ms */
	fw.defer_mmu = 1;
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 6, 0);
	/* F: no acknowledgement wait: the in-order probe */
	sc_set(a, 3, 0);
	sc_wait(a, sc_submit(a, 1024, 600, 1), 100);
	sc_set(a, 3, 1);
	sc_set(a, 5, 0);
	sc_set(a, 4, 1);

	/* G: a hardware recovery during the wait */
	fw.hwr_on_3d = 1;
	f = sc_submit(a, 1024, 600, 1);
	sc_wait(a, f, 100);
	sc_wait(a, f, 100);

	/* H: a stall: the timeout dump, then (after a refused grow) the second */
	fw.stopped = true;
	f = sc_submit(a, 1024, 600, 1);
	sc_wait(a, f, 25);
	sc_wait(a, f, 25);
	sc_fw_grow(3); /* the local list cannot grow: a refusal */
	sc_wait(a, f, 25);
	sc_wait(a, f, 25);
	fw.stopped = false;
	sc_wait(a, f, 100);

	/* I: every pair taken: recycles (plain; with the cleanup and the
	 * rebuild; a cleanup the firmware refuses) */
	for (uint32_t i = 0; i < 18; i++) {
		sc_wait(a, sc_submit(a, 100 + 16 * i, 80 + 8 * i, 1), 100);
	}
	sc_set(a, 8, 1);
	sc_set(a, 9, 1);
	sc_wait(a, sc_submit(a, 512, 512, 1), 100);
	fw.cleanup_busy = true;
	sc_wait(a, sc_submit(a, 520, 520, 1), 100);
	fw.cleanup_busy = false;
	sc_set(a, 8, 0);
	sc_set(a, 9, 0);

	/* J: round the client CCBs (padding at the end) */
	for (int i = 0; i < 90; i++) {
		f = sc_submit(a, 1024, 600, 1);
		sc_wait(a, f, 100);
	}
out:
	rec_tick = NULL;
	rec_wtick = NULL;
	fw.a = NULL;
}

#endif /* PVR_KM_SCENARIO_H */
