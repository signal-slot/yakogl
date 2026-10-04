/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr.c - Zephyr OSAL backend for YakoGL on the PowerVR BXS-4-64.
 *
 * Thin glue over the driver: the bring-up driver (src/pvr_bringup.c +
 * src/pvr_fw_boot.c) has powered the GPU, mapped the registers and booted the
 * firmware; the render service (src/pvr_render.c) owns the render context,
 * freelists, HWRT datasets and the CCCB/KCCB submit path. This file only
 * adapts those to the gles_osal.h contract:
 *
 *   init        -> pvr_render_init()
 *   gpu_alloc   -> pvr_render_alloc() (the driver's shared bump allocator, so
 *                  GL objects and driver objects never collide)
 *   gpu_write/  -> memcpy + Zephyr cache maintenance (the carveout is CPU-mapped
 *   gpu_read       write-back; the GPU is not coherent with the CPU caches)
 *   submit      -> pvr_render_submit(kick)   (kick = struct pvr_render_kick)
 *   wait        -> pvr_render_wait(fence)
 *   query       -> pvr_render_static_get()
 *   reg_read/   -> sys_read32/sys_write32 on the register base recorded at boot
 *   reg_write
 *
 * (It also compiled in the Linux kernel-module harness, pvrgpu.ko, through
 * a Zephyr API shim; that harness is retired: driver/tools/pvrlab/README.md.)
 */

#include <stdarg.h>
#include <stdbool.h>
#include <string.h>
#include <errno.h>

#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/cache.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/logging/log.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_bounds.h"
#include "osal_log.h"
#include "zephyr_clock.h"
#include "pvr_drv.h"
#include "pvr_render.h"
#include "pvr/pvr_backend.h"  /* pvr_backend_ds_report */

LOG_MODULE_REGISTER(gles_osal, CONFIG_YAKOGL_LOG_LEVEL);

static struct {
	bool      inited;
	uintptr_t regs;
} s;

/* ---- lifecycle ---------------------------------------------------- */
static int z_init(void *platform_arg)
{
	const struct osal_zephyr_init *cfg = platform_arg;
	int rc;

	s.regs = (cfg && cfg->regs) ? cfg->regs : (uintptr_t)pvr_regs();
#if defined(CONFIG_YAKOGL_PVR_FRAME_SETS)
	/* the backend's frame sets (zephyr/Kconfig YAKOGL_PVR_FRAME_SETS): set
	 * before the backend starts; the defaults are what it always built */
	rc = pvr_backend_set_frame_arenas(CONFIG_YAKOGL_PVR_FRAME_SETS, CONFIG_YAKOGL_PVR_ARENA_GEN_KB,
					  CONFIG_YAKOGL_PVR_ARENA_PDS_KB);
	if (rc)
		printk("osal(zephyr): frame arenas %u x (general %u + pds %u KiB) refused (%d); the backend keeps "
		       "what it has\n", CONFIG_YAKOGL_PVR_FRAME_SETS, CONFIG_YAKOGL_PVR_ARENA_GEN_KB,
		       CONFIG_YAKOGL_PVR_ARENA_PDS_KB, rc);
	else if (CONFIG_YAKOGL_PVR_FRAME_SETS != 4 || CONFIG_YAKOGL_PVR_ARENA_GEN_KB != 1024 ||
		 CONFIG_YAKOGL_PVR_ARENA_PDS_KB != 768)
		printk("osal(zephyr): frame arenas %u x (general %u + pds %u KiB), not the default 4 x (1024 + 768)\n",
		       CONFIG_YAKOGL_PVR_FRAME_SETS, CONFIG_YAKOGL_PVR_ARENA_GEN_KB, CONFIG_YAKOGL_PVR_ARENA_PDS_KB);
#endif
	rc = pvr_render_init();
	if (rc) {
		LOG_ERR("osal(zephyr): render service init failed (%d)", rc);
		return rc;
	}
	s.inited = true;
	LOG_INF("osal(zephyr): ready (regs=%#lx)", (unsigned long)s.regs);
	return 0;
}

static void report_pending(const char *what);
static void report_pending_now(const char *when);
static uint32_t pending_kib(void);
#if defined(CONFIG_YAKOGL_PVR_POOL_OWNERS)
static void report_owners_at_refusal(void);
#endif

/* CONFIG_YAKOGL_PVR_POOL_RECLAIM (zephyr/Kconfig): a free with no frame open
 * waits only for the last submitted kick, and a refused allocation waits for
 * the submitted kicks and is tried once more (driver/docs/pvr-render-pool.md,
 * mechanism 2). On by default (zephyr/Kconfig); n keeps the old hold. */
#if defined(CONFIG_YAKOGL_PVR_POOL_RECLAIM)
#define PVR_POOL_RECLAIM 1
#else
#define PVR_POOL_RECLAIM 0
#endif
static int reclaim_and_retry(enum pvr_render_heap heap, uint32_t size, struct pvr_gpu_bo *bo);

static void z_shutdown(void)
{
	report_pending(""); /* the pending-free queue's high-water and refusals */
	report_pending_now("at shutdown");
	pvr_render_pool_report();
	s.inited = false;
	/* The GPU stays powered/booted (owned by the DT driver); nothing to free
	 * in the bump allocator. */
}

/* ---- GPU memory + MMU --------------------------------------------- */
static enum pvr_render_heap heap_of(uint64_t heap_base)
{
	if (heap_base == PVR_USCCODE_HEAP_BASE)
		return PVR_RENDER_HEAP_USC;
	if (heap_base == PVR_PDSCODEDATA_HEAP_BASE)
		return PVR_RENDER_HEAP_PDS;
	return PVR_RENDER_HEAP_GENERAL;
}

static int z_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags,
		       struct osal_gpu_mem *out)
{
	struct pvr_gpu_bo bo;
	int rc;

	(void)flags;   /* all GL data is SLC-bypassed (pvr_fw_boot.c:1201) */
	if (!s.inited || !out || size == 0 || size > 0xFFFFFFFFu)
		return -EINVAL;
	if (heap_base != PVR_GENERAL_HEAP_BASE && heap_base != PVR_USCCODE_HEAP_BASE &&
	    heap_base != PVR_PDSCODEDATA_HEAP_BASE) {
		LOG_ERR("osal: unknown GPU heap %#llx", (unsigned long long)heap_base);
		return -EINVAL;
	}
	rc = pvr_render_alloc(heap_of(heap_base), (uint32_t)size, &bo);
	if (rc == -ENOMEM) {
		report_pending_now("an allocation refused"); /* after the pool's own summary */
#if defined(CONFIG_YAKOGL_PVR_POOL_OWNERS)
		report_owners_at_refusal();
#endif
		if (PVR_POOL_RECLAIM)
			rc = reclaim_and_retry(heap_of(heap_base), (uint32_t)size, &bo);
	}
	if (rc)
		return rc;
	out->cpu  = bo.cpu;
	out->gpu  = bo.gpu;
	out->phys = pvr_bo_phys(&bo);
	out->size = ROUND_UP(size, 0x1000);
	out->impl = bo.cpu;   /* non-NULL marks a live allocation for the GL core */
	return 0;
}

/* Frees are deferred: the GL core releases a texture or buffer as soon as
 * the application deletes it, but the kick being recorded (the next one to
 * be submitted) and every kick still in flight may read it. Each entry is
 * tagged with the sequence of the kick that was being recorded at the time;
 * it is released once a fence at or beyond that sequence has completed.
 * The queue and its rule on a full queue: pending_frees.h. */

/* The queue's capacity. What bounds it: static RAM (an entry is 32 bytes,
 * 4 KiB for the queue) against how often a burst of deletes between two
 * kicks has to wait for the oldest kick (pending_frees.h). */
#define PENDING_FREES_CAP 128
/* The full-queue wait's bound: the PowerVR backend's own submit wait
 * (src/pvr/backend.rs WAIT_MS). */
#define PENDING_FREES_WAIT_MS 20000u

struct z_pending {
	struct pvr_gpu_bo bo;
	uint32_t size;
};
#define PF_PAYLOAD  struct z_pending
#define PF_CAPACITY PENDING_FREES_CAP
#include "pending_frees.h"

static struct pf_queue s_pending;
static uint32_t s_last_submitted;

static void kt_completed(uint32_t done);

/* The queue's counters, every run: at close and at the first refusal.
 * printk, not LOG_*, so no log level makes a refusal silent. */
static void report_pending(const char *what)
{
	char line[160];

	pf_format(&s_pending, line, sizeof(line));
	printk("osal(zephyr): %s%s\n", what, line);
}

#if defined(CONFIG_YAKOGL_PVR_POOL_OWNERS)
/* The owner lines and their balance (zephyr/Kconfig
 * YAKOGL_PVR_POOL_OWNERS): the owners' KiB plus what the pending-free queue
 * holds, against the pool's live bytes (handed out minus free). A negative
 * remainder means an owner was counted twice. */
static void report_owners(void)
{
	uint64_t drv = pvr_render_pool_owners();
	uint64_t be = pvr_backend_pool_owners();
	uint64_t gl = yakogl_pool_owners();
	uint64_t pend = pending_kib(), out, freeb, live, sum;

	pvr_render_pool_usage(&out, &freeb);
	live = (out - freeb) / 1024;
	sum = drv + be + gl + pend;
	printk("osal(zephyr): pool owners: driver %llu + backend %llu + gl %llu + pending %llu = %llu KiB; "
	       "pool live %llu KiB (handed out %llu - free %llu); unaccounted %lld KiB\n",
	       (unsigned long long)drv, (unsigned long long)be, (unsigned long long)gl,
	       (unsigned long long)pend, (unsigned long long)sum, (unsigned long long)live,
	       (unsigned long long)(out / 1024), (unsigned long long)(freeb / 1024),
	       (long long)live - (long long)sum);
}
#endif

#if defined(CONFIG_YAKOGL_PVR_POOL_OWNERS)
/* At the first refused allocation, the part of the owner report that is
 * safe to take from inside an allocation: the driver's owners and the
 * pending frees. The backend's and the GL core's tables are borrowed by the
 * very call that is allocating (a GL entry point, a backend draw), so they
 * are not walked here; what they hold is the rest of the live pool, printed
 * as one figure. The run-end report has the backend's own lines, and the
 * GL core's share at the refusal is that figure minus the backend's. */
static void report_owners_at_refusal(void)
{
	static bool done;
	uint64_t drv, pend, out, freeb, live;

	if (done)
		return;
	done = true;
	drv = pvr_render_pool_owners();
	pend = pending_kib();
	pvr_render_pool_usage(&out, &freeb);
	live = (out - freeb) / 1024;
	printk("osal(zephyr): pool owners at the first refusal: driver %llu + pending %llu KiB; "
	       "backend + gl (the rest of live) %lld KiB; pool live %llu KiB (handed out %llu - free %llu)\n",
	       (unsigned long long)drv, (unsigned long long)pend, (long long)live - (long long)(drv + pend),
	       (unsigned long long)live, (unsigned long long)(out / 1024), (unsigned long long)(freeb / 1024));
}
#endif

/* osal_internal.h: the close record, every run; the OSAL stays up */
void osal_zephyr_diag_print(void)
{
	char ds[160];

	report_pending("");
	report_pending_now("at close");
	pvr_render_pool_report();
#if defined(CONFIG_YAKOGL_PVR_POOL_OWNERS)
	report_owners();
#endif
	pvr_backend_ds_report(ds, sizeof(ds));
	printk("osal(zephyr): %s\n", ds);
}

/* What the queue holds now: memory no allocation can reuse until its fence
 * completes (driver/docs/pvr-render-pool.md, mechanism 2). An empty queue has
 * no oldest fence: "none", not 0. */
static uint32_t pending_kib(void)
{
	uint32_t kib = 0;

	for (unsigned int i = 0; i < s_pending.n; i++)
		kib += s_pending.e[i].kib;
	return kib;
}

/* The reclaim's counters (CONFIG_YAKOGL_PVR_POOL_RECLAIM): allocations the
 * pool refused, and of those: served after giving back without a wait,
 * served after a wait, and refused still. */
static struct { uint32_t refusals, served_now, served_waited, refused; } s_reclaim;

static void report_pending_now(const char *when)
{
	char oldest[16] = "none";
	char reclaim[96] = "reclaim off";

	if (s_pending.n) {
		uint32_t o = s_pending.e[0].fence;

		for (unsigned int i = 1; i < s_pending.n; i++)
			o = MIN(o, s_pending.e[i].fence);
		snprintk(oldest, sizeof(oldest), "%u", o);
	}
	if (PVR_POOL_RECLAIM)
		snprintk(reclaim, sizeof(reclaim),
			 "reclaim on: %u refusals, %u served without a wait, %u after a wait, %u refused",
			 s_reclaim.refusals, s_reclaim.served_now, s_reclaim.served_waited, s_reclaim.refused);
	printk("osal(zephyr): %s: pending frees now %u (%u KiB), oldest fence %s, last submitted %u; %s\n",
	       when, s_pending.n, pending_kib(), oldest, s_last_submitted, reclaim);
}

/* The PowerVR pool's line: its per-heap largest free range and its bump
 * high-water are in pvr_render_pool_report (the run's end), not exposed as
 * numbers, so they are "absent" here. */
void osal_mem_line(char *buf, size_t n)
{
	uint64_t out = 0, freeb = 0;

	pvr_render_pool_usage(&out, &freeb);
	snprintk(buf, n,
		 "osal(zephyr) mem: pool handed out %llu, free in the lists %llu, live %llu bytes; "
		 "frees held %u (%u KiB); high-water absent, largest free range absent (pvr_render_pool_report)",
		 (unsigned long long)out, (unsigned long long)freeb, (unsigned long long)(out - freeb), s_pending.n,
		 pending_kib());
}

static void release_pending(const struct z_pending *p)
{
	pvr_render_free(&p->bo, p->size);
}

static void drain_pending_frees(uint32_t completed)
{
	pf_drain(&s_pending, completed, release_pending);
}

/* pending_frees.h's wait: the render service's, bounded as the backend's */
static int wait_pending(uint32_t fence, uint32_t *completed)
{
	uint32_t done;
	int rc = pvr_render_wait(fence, PENDING_FREES_WAIT_MS);

	if (rc == -ETIMEDOUT)
		return 1;
	if (rc)
		return rc;
	done = MAX(pvr_render_completed(), fence);
	kt_completed(done);
	*completed = done;
	return 0;
}

/* Whether a held block waits for a kick already submitted: only such a
 * block can come back by waiting. */
static bool held_for_submitted(void)
{
	for (unsigned int i = 0; i < s_pending.n; i++)
		if (s_pending.e[i].fence <= s_last_submitted)
			return true;
	return false;
}

/* CONFIG_YAKOGL_PVR_POOL_RECLAIM: the pool refused an allocation.
 * 1. Without waiting: give back every held block whose kick has already
 *    completed (one read of the completion counter); when that gave
 *    anything back, try again.
 * 2. Still refused, and a held block waits for a kick already submitted:
 *    wait for the submitted kicks (bounded by PENDING_FREES_WAIT_MS, the
 *    backend's own submit wait), give back what they held, and try once
 *    more. Blocks held for the kick still being recorded stay held: no wait
 *    completes a kick that is not submitted.
 * The GL call that allocated (glBufferData, glTexImage2D, a draw's staging)
 * blocks for the wait in 2, inside the frame being recorded. Each step
 * prints its own line. */
static int reclaim_and_retry(enum pvr_render_heap heap, uint32_t size, struct pvr_gpu_bo *bo)
{
	uint32_t before = pending_kib(), done;
	int64_t t0;
	int wrc, rc;

	s_reclaim.refusals++;
	done = pvr_render_completed();
	kt_completed(done);
	drain_pending_frees(done);
	if (pending_kib() < before) {
		rc = pvr_render_alloc(heap, size, bo);
		printk("osal(zephyr): reclaim without waiting: gave back %u KiB, the retry %s\n",
		       before - pending_kib(), rc == 0 ? "was served" : "was refused");
		if (rc == 0) {
			s_reclaim.served_now++;
			return 0;
		}
	}
	if (!held_for_submitted()) {
		s_reclaim.refused++;
		printk("osal(zephyr): reclaim found nothing held for a submitted kick: no wait, refused "
		       "(%u KiB held for the kick being recorded)\n", pending_kib());
		return -ENOMEM;
	}
	before = pending_kib();
	t0 = k_uptime_get();
	wrc = pvr_render_wait(s_last_submitted, PENDING_FREES_WAIT_MS);
	if (wrc == 0)
		kt_completed(s_last_submitted);
	drain_pending_frees(pvr_render_completed());
	rc = pvr_render_alloc(heap, size, bo);
	if (rc == 0)
		s_reclaim.served_waited++;
	else
		s_reclaim.refused++;
	printk("osal(zephyr): reclaim after a wait: waited %lld ms for kick %u (%d), gave back %u KiB, the retry %s\n",
	       (long long)(k_uptime_get() - t0), s_last_submitted, wrc, before - pending_kib(),
	       rc == 0 ? "was served" : "was refused");
	return rc;
}

static int z_gpu_free(struct osal_gpu_mem *mem)
{
	int rc = 0;

	if (!mem)
		return 0;
	if (mem->impl && mem->cpu && mem->size) {
		const struct z_pending p = {
			.bo = { .gpu = mem->gpu, .cpu = mem->cpu,
				.heap_off = (uint32_t)(mem->gpu - PVR_GENERAL_HEAP_BASE) },
			.size = (uint32_t)mem->size,
		};

		/* Without the reclaim: the kick being recorded, whether or not
		 * anything is recorded. With it: that kick only while a frame is
		 * open, else the last one submitted (pvr_backend_free_fence). */
		const uint32_t fence = PVR_POOL_RECLAIM ? pvr_backend_free_fence(s_last_submitted)
							: s_last_submitted + 1;

		rc = pf_push(&s_pending, &p, (uint32_t)DIV_ROUND_UP(mem->size, 1024),
			     fence, s_last_submitted, PF_WAIT(wait_pending), release_pending);
		if (rc && s_pending.refused == 1)
			report_pending("a free refused, memory kept (the first; the rest are counted): ");
		/* a block no kick in flight can read goes back now */
		if (PVR_POOL_RECLAIM && rc == 0 && fence <= s_last_submitted)
			drain_pending_frees(pvr_render_completed());
	}
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return rc;
}

static int z_mmu_map(uint64_t gpu_va, uint64_t phys, size_t size, uint32_t flags)
{
	(void)flags;
	if (!s.inited)
		return -EINVAL;
	pvr_mmu_map(gpu_va, phys, ROUND_UP(size, 0x1000), PVR_L0_FLAGS_DATA);
	return 0;
}

static int z_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
		      struct osal_gpu_mem *out)
{
	struct pvr_gpu_bo bo;
	int rc;

	(void)flags;
	if (!s.inited || !out || size == 0 || size > 0xFFFFFFFFu)
		return -EINVAL;
	rc = pvr_render_map_phys(heap_of(heap_base), phys, (uint32_t)size, &bo);
	if (rc)
		return rc;
	memset(out, 0, sizeof(*out));
	out->gpu = bo.gpu;
	out->phys = phys;
	out->size = ROUND_UP(size, 0x1000);
	out->impl = (void *)1;   /* a valid mapping */
	/* A cached CPU view for glReadPixels on a window surface (the caller
	 * owns the pages; the GPU and a display read them directly). Without
	 * it the surface is write-only from the CPU's point of view. */
	k_mem_map_phys_bare((uint8_t **)&out->cpu, phys & ~0xFFFULL, out->size + (phys & 0xFFF),
			    K_MEM_CACHE_WB | K_MEM_PERM_RW);
	if (out->cpu)
		out->cpu = (uint8_t *)out->cpu + (phys & 0xFFF);
	return 0;
}

static int z_gpu_write(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, src, n))
		return -EINVAL;
	memcpy((uint8_t *)mem->cpu + off, src, n);
	sys_cache_data_flush_range((uint8_t *)mem->cpu + off, n);
	return 0;
}

static int z_gpu_read(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, dst, n))
		return -EINVAL;
	sys_cache_data_invd_range((uint8_t *)mem->cpu + off, n);
	memcpy(dst, (uint8_t *)mem->cpu + off, n);
	return 0;
}

/* ---- cache maintenance -------------------------------------------- */
static void z_cache_flush(void *cpu, size_t n)
{
	if (cpu && n)
		sys_cache_data_flush_range(cpu, n);
}

static void z_cache_invd(void *cpu, size_t n)
{
	if (cpu && n)
		sys_cache_data_invd_range(cpu, n);
}

/* ---- submit + completion ------------------------------------------ */

/* GPU timing for the diagnostics beat: when each kick was submitted, when
 * its completion was first observed (the QPA polls every 2 ms, a wait
 * returns at once), and how long at least one kick was outstanding. */
#define KT_RING 64u
static struct { uint32_t fence; int64_t at; } s_kt[KT_RING];
static uint32_t s_kt_done;          /* fences accounted up to here */
static int64_t s_busy_since = -1;   /* uptime a kick became outstanding, -1: idle */
static struct { uint32_t n; uint64_t lat_sum, busy; uint32_t lat_max; } s_gpu_t;

static void kt_submitted(uint32_t fence)
{
	int64_t now = k_uptime_get();

	s_kt[fence % KT_RING].fence = fence;
	s_kt[fence % KT_RING].at = now;
	if (s_busy_since < 0)
		s_busy_since = now;
}

#ifndef YAKOGL_OSAL_KICK_LOG
#define YAKOGL_OSAL_KICK_LOG 0
#endif

static void kt_completed(uint32_t done)
{
	int64_t now;

	if (done <= s_kt_done)
		return;
	now = k_uptime_get();
	while (s_kt_done < done) {
		uint32_t f = ++s_kt_done;

		if (s_kt[f % KT_RING].fence == f) {
			uint32_t lat = (uint32_t)(now - s_kt[f % KT_RING].at);
			/* Off by default: this line is written to the serial
			 * console from the completion path, and the console is
			 * the frame time while the budget lasts (see the
			 * backend's own bring-up budgets). */
			static uint32_t log_left = YAKOGL_OSAL_KICK_LOG;

			s_gpu_t.n++;
			s_gpu_t.lat_sum += lat;
			if (lat > s_gpu_t.lat_max)
				s_gpu_t.lat_max = lat;
			/* Bring-up: the first kicks' times, to pair with the
			 * backend's per-kick line (what that pass rendered). */
			if (log_left) {
				log_left--;
				LOG_INF("osal: kick %u took %u ms", f, lat);
			}
		}
	}
	if (done >= s_last_submitted && s_busy_since >= 0) {
		s_gpu_t.busy += (uint64_t)(now - s_busy_since);
		s_busy_since = -1;
	}
}

/* Totals since boot: kicks completed, the sum and the maximum of their
 * submit-to-completion times, and the time the GPU had work (ms). */
void yakogl_zephyr_gpu_times(uint32_t *kicks, uint64_t *lat_sum_ms, uint32_t *lat_max_ms,
			     uint64_t *busy_ms)
{
	uint64_t busy = s_gpu_t.busy;

	if (s_busy_since >= 0)
		busy += (uint64_t)(k_uptime_get() - s_busy_since);
	if (kicks) *kicks = s_gpu_t.n;
	if (lat_sum_ms) *lat_sum_ms = s_gpu_t.lat_sum;
	if (lat_max_ms) { *lat_max_ms = s_gpu_t.lat_max; s_gpu_t.lat_max = 0; }
	if (busy_ms) *busy_ms = busy;
}

static int64_t z_submit(const void *kick_desc, size_t kick_desc_len)
{
	uint32_t fence = 0;
	int rc;

	if (!s.inited || !kick_desc || kick_desc_len < sizeof(struct pvr_render_kick))
		return -EINVAL;
	rc = pvr_render_submit((const struct pvr_render_kick *)kick_desc, &fence);
	if (rc)
		return rc;
	s_last_submitted = fence;
	kt_submitted(fence);
	return (int64_t)fence;
}

/* The vertical-sync source is the platform's, not the OSAL's: the display is
 * owned by the board glue (the AM62P's DSS driver, say), and YakoGL only
 * needs to be told when a scan-out period starts. A platform that has one
 * defines this; the default says it has none, and presentation is immediate.
 *
 * Returns 0 when a period passed, -ETIMEDOUT, or -ENOTSUP. */
__weak int yakogl_zephyr_wait_vblank(int64_t timeout_us)
{
	ARG_UNUSED(timeout_us);
	return -ENOTSUP;
}

static int z_wait_vblank(int64_t timeout_us)
{
	return yakogl_zephyr_wait_vblank(timeout_us);
}

static int64_t z_completed(void)
{
	uint32_t done;

	if (!s.inited)
		return 0;
	done = pvr_render_completed();
	drain_pending_frees(done);
	kt_completed(done);
	return (int64_t)done;
}

static int z_wait(int64_t fence, int64_t timeout_us)
{
	uint32_t ms;
	int rc;

	if (!s.inited || fence <= 0)
		return -EINVAL;
	ms = timeout_us < 0 ? 0xFFFFFFFFu : (uint32_t)((timeout_us + 999) / 1000);
	rc = pvr_render_wait((uint32_t)fence, ms);
	if (rc == -ETIMEDOUT)
		return 1;
	if (rc == 0) {
		drain_pending_frees((uint32_t)fence);   /* every kick up to it has completed */
		kt_completed((uint32_t)fence);
	}
	return rc;
}

/* ---- queries -------------------------------------------------------- */
static int z_query(uint32_t what, void *out, size_t out_sz)
{
	if (!s.inited || !out)
		return -EINVAL;
	switch (what) {
	case OSAL_QUERY_RENDER_STATIC:
		if (out_sz < sizeof(struct pvr_render_static))
			return -EINVAL;
		return pvr_render_static_get((struct pvr_render_static *)out);
	case OSAL_QUERY_CLOCK_NS:
		return osal_zephyr_clock_ns(out, out_sz);
	default:
		return -ENOTSUP;
	}
}

/* ---- registers ---------------------------------------------------- */
static uint32_t z_reg_read(uint32_t off)
{
	return sys_read32(s.regs + off);
}

static void z_reg_write(uint32_t off, uint32_t val)
{
	sys_write32(val, s.regs + off);
}

/* ---- diagnostics -------------------------------------------------- */
static void z_log(const char *fmt, ...)
{
	struct osal_log_line line;   /* OSAL_LOG_LINE_MAX: a core line is never cut */
	const char *text;
	va_list ap;

	va_start(ap, fmt);
	text = osal_log_format(&line, osal_vsnprintk, fmt, ap);
	va_end(ap);
	LOG_INF("%s", text);
}

/* thread_id (gles_osal.h, "threads"): the running thread's k_thread
 * address, folded to 32 bits. On these boards RAM lies below 4 GiB, so the
 * fold keeps the address as it is and two live threads never share it. */
static uint32_t z_thread_id(void)
{
	uintptr_t t = (uintptr_t)k_current_get();

	return (uint32_t)(t ^ (t >> 16 >> 16));
}

/* ------------------------------------------------------------------ *
 * ops table + registrar.
 * ------------------------------------------------------------------ */
const struct osal_ops osal_zephyr_ops = {
	.init        = z_init,
	.shutdown    = z_shutdown,
	.gpu_alloc   = z_gpu_alloc,
	.gpu_free    = z_gpu_free,
	.mmu_map     = z_mmu_map,
	.map_phys = z_map_phys,
	.gpu_write   = z_gpu_write,
	.gpu_read    = z_gpu_read,
	.cache_flush = z_cache_flush,
	.cache_invd  = z_cache_invd,
	.submit      = z_submit,
	.wait        = z_wait,
	.completed   = z_completed,
	.reg_read    = z_reg_read,
	.reg_write   = z_reg_write,
	.query       = z_query,
	.log         = z_log,
	.thread_id   = z_thread_id,
	.wait_vblank = z_wait_vblank,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
