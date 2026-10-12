/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The Vivante GC7000 render service: the implementation of gc_render.h.
 *
 * It owns the carveout (the CPU mapping and a bump allocator), drives the
 * milestone-2 and milestone-3 pieces in the right order at init, and turns the
 * hardware's 30 recycled events into monotonic fences.
 *
 * Nothing here has run on silicon.
 */

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include "gc_internal.h"
#include "hw/state_hi.xml.h"
#include "hw/state_3d.xml.h"

LOG_MODULE_DECLARE(gpu_gc, CONFIG_SAFEUI_GPU_VIVANTE_LOG_LEVEL);

/* See gc_render.h. Empty unless a sample overrides it; the calls below cost
 * nothing at all in that case. */
__weak void gc_render_mark(unsigned int step)
{
	ARG_UNUSED(step);
}

struct gc_dev g_gc;

static bool s_ready;

void *gc_cpu_of(struct gc_dev *g, uint32_t gpu_va)
{
	if (g->carveout == NULL || gpu_va < g->carveout_phys ||
	    gpu_va >= g->carveout_phys + g->carveout_size) {
		return NULL;
	}
	return g->carveout + (gpu_va - g->carveout_phys);
}

void gc_dev_publish(const struct device *dev, mm_reg_t regs, uint64_t carveout_phys,
		    size_t carveout_size, const struct gc_identity *id)
{
	g_gc.dev = dev;
	g_gc.id = id;
	g_gc.regs = regs;
	g_gc.carveout_phys = carveout_phys;
	g_gc.carveout_size = carveout_size;
}

bool gc_render_ready(void)
{
	return s_ready;
}

/* ------------------------------------------------------------------ *
 * The carveout
 * ------------------------------------------------------------------ */

/*
 * Map the carveout Normal non-cacheable, the attribute the Linux reference
 * gets from dma_alloc_wc for exactly the same objects. The CPU and the GPU
 * share cache lines here -- the ring's WAIT word is rewritten under a front end
 * that is reading it, and the page tables are read by the MMU while the CPU
 * adds entries -- and no amount of clean-and-invalidate makes that safe.
 * Device memory is not an option either: it alignment-faults the struct and
 * memcpy writes the command builders do.
 *
 * This is the same decision the PowerVR driver made on the AM62P and for the
 * same reason (src/pvr_fw_boot.c).
 */
static int map_carveout(struct gc_dev *g)
{
	uint8_t *cpu = NULL;

#ifdef K_MEM_ARM_NORMAL_NC
	k_mem_map_phys_bare(&cpu, g->carveout_phys, g->carveout_size,
			    K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
	k_mem_map_phys_bare(&cpu, g->carveout_phys, g->carveout_size,
			    K_MEM_CACHE_WB | K_MEM_PERM_RW);
#endif
	if (cpu == NULL) {
		LOG_ERR("carveout: mapping 0x%llx+%zu failed; is KERNEL_VM_SIZE big enough?",
			(unsigned long long)g->carveout_phys, g->carveout_size);
		return -ENOMEM;
	}
	g->carveout = cpu;
	LOG_INF("carveout: 0x%llx (%zu MiB) at %p", (unsigned long long)g->carveout_phys,
		g->carveout_size / (1024 * 1024), (void *)cpu);
	return 0;
}

/*
 * Free list over the pool.
 *
 * The pool is a bump allocator over the carveout: nothing it hands out ever
 * comes back, which is all the bring-up samples need, and exactly what a
 * running GL application cannot live with. FemtoVG re-uploads its vertex
 * buffer every frame, and YakoGL's glBufferData has to take fresh storage
 * whenever the old one may still be read by a kick that has not completed --
 * so every frame costs the pool another buffer, and 64 MiB is gone in
 * seconds (measured on 2026-09-21: the GL reported an error a few seconds
 * into gc_slint's animation, and it was GL_OUT_OF_MEMORY from here).
 *
 * So freed blocks go on this list and are reused before the bump moves. The
 * carveout is identity-mapped and permanently mapped in the GPU MMU, so a
 * reused block needs no MMU work, only zeroing: one offset describes it in
 * the CPU, the physical and the GPU address spaces at once. Adjacent blocks
 * merge; there is no further coalescing, because a UI's working set is a
 * stable set of sizes and exact reuse dominates. This mirrors
 * pvr_render_free() in src/pvr_render.c, which the PowerVR side needed for
 * the same reason.
 */
#include "gc_render_pool.h"
#include "gc_render_defer.h"

/* The pool itself. It was inline here; it lives in the header so that
 * tools/gclab/pooltest can exercise this code rather than a transcription of
 * it, which is what makes a pool test worth having. */
static struct gc_pool g_pool;

/* Freed blocks waiting for the kick that referenced them to retire. GL lets an
 * application delete a buffer a draw is still reading, so the lifetime is ours
 * to keep; see gc_render_defer.h for what that cost to find. */
static struct gc_defer g_defer;
static bool g_defer_off;

/* Turn the deferral off, to reproduce the defect deliberately. There is no
 * knob to turn it on, because it is the path rather than an option. */
/* Log the shader-state registers once, after the first submit that COMPLETES.
 *
 * A module parameter rather than a sysfs write because the sysfs node needs
 * root and the harness that runs the board does not have it -- but it does
 * have a reload wrapper that forwards insmod arguments, so a load-time knob is
 * inside its scope and a runtime one is not. The measurement was otherwise
 * unreachable on the arm that matters. */

/* Re-enable the read-back on the fence-timeout path -- that is, on a GPU that
 * has stopped answering. Off by default and named for the hazard: a board
 * rebooted once with this behaviour unconditional, causation unproven. For a
 * deliberate experiment with the board otherwise idle, never for a sweep. */

/* How long a deadline-free wait tolerates a GPU that retires nothing before it
 * treats the silence as a hang. 0 restores the old behaviour: wait until the
 * fence retires or the caller is interrupted, however long that is.
 *
 * Without it, a GL client wedged the GPU and stayed blocked until something
 * killed it (gl_test 72 on the online pair, 2026-09-23: 300 s each time). The
 * recovery then ran only from the close path's bounded wait, after the client
 * was already gone, so the error it exists to report reached nobody. The bound
 * is on RETIREMENT, not on the waited fence: a fence behind a long queue of
 * healthy kicks keeps seeing fence_done move and never trips it. */
static uint32_t s_hang_ms = GC_HANG_MS_DEFAULT;

void gc_render_set_hang_ms(uint32_t ms)
{
	s_hang_ms = ms;
}

void gc_render_defer_disable(bool off)
{
	g_defer_off = off;
}

/* Give the pool back every block whose kick has retired. Batched so the stack
 * frame stays small -- kernel stacks are 16 KiB and this runs under the
 * harness as well as on Zephyr. */
static void defer_drain(struct gc_dev *g)
{
	struct gc_defer_block batch[16];
	unsigned n;

	do {
		n = gc_defer_drain(&g_defer, g->fence_done, batch, (unsigned)(sizeof(batch) / sizeof(batch[0])));
		for (unsigned i = 0; i < n; i++) {
			gc_pool_free(&g_pool, batch[i].off, batch[i].size);
		}
	} while (n == (unsigned)(sizeof(batch) / sizeof(batch[0])));
	g->pool_double_frees = g_pool.double_frees;
}

void gc_render_defer_stats(unsigned *live, unsigned *peak, unsigned *refused)
{
	if (live) {
		*live = g_defer.n;
	}
	if (peak) {
		*peak = g_defer.peak;
	}
	if (refused) {
		*refused = g_defer.refused;
	}
}

int gc_render_alloc(size_t size, size_t align, struct gc_bo *out)
{
	struct gc_dev *g = gc_dev_get();
	size_t start;

	if (align == 0) {
		align = GC_ALIGN_DEFAULT;
	}
	if (g->carveout == NULL) {
		return -ENODEV;
	}
	/* Retired blocks come back before the bump is asked to grow: without
	 * this a deferred block would only return on the next free, and a
	 * workload that allocates steadily would grow the pool past what it
	 * actually needs. */
	defer_drain(g);
	{
		/* The free list is GC_POOL_FREE_SLOTS entries and, once it
		 * is full, the smallest block is evicted to make room. It used to say so per free; the pool cannot
		 * log, so the warning lives here and fires once, on the way
		 * past a tenth of the pool. Silence was not an option: losing
		 * memory invisibly is how a pool ends up looking like a
		 * hardware fault. */
		static bool warned;

		if (!warned && g_pool.dropped > (g_pool.limit - g_pool.base) / 10) {
			warned = true;
			LOG_WRN("pool: %zu KiB dropped (the %u-slot free list cannot hold this "
				"allocation pattern); reuse will degrade",
				g_pool.dropped / 1024, GC_POOL_FREE_SLOTS);
		}
	}
	start = gc_pool_alloc(&g_pool, size, align);
	if (start == SIZE_MAX) {
		LOG_ERR("pool: %zu bytes at align %zu does not fit (%zu of %zu used)",
			size, align, g_pool.bump - g_pool.base,
			g_pool.limit - g_pool.base);
		return -ENOMEM;
	}
	size = ROUND_UP(size, align);
	memset(g->carveout + start, 0, size);

	out->cpu = g->carveout + start;
	out->phys = g->carveout_phys + start;
	out->gpu = (uint32_t)out->phys;   /* the carveout is identity-mapped */
	out->size = size;
	return 0;
}

void gc_render_pool_usage(size_t *live, size_t *bumped, size_t *total, size_t *dropped, size_t *largest)
{
	size_t big = g_pool.limit - g_pool.bump;

	for (unsigned int i = 0; i < GC_POOL_FREE_SLOTS; i++)
		if (g_pool.free[i].size > big)
			big = g_pool.free[i].size;
	if (live)
		*live = g_pool.live;
	if (bumped)
		*bumped = g_pool.bump - g_pool.base;
	if (total)
		*total = g_pool.limit - g_pool.base;
	if (dropped)
		*dropped = g_pool.dropped;
	if (largest)
		*largest = big;
}

/*
 * Put the pool back as a freshly loaded module has it: the bump at the base
 * and no free blocks at all.
 *
 * The service is one render context, so exactly one GL client at a time (the
 * chardev refuses a second process). When that client is gone, nothing it
 * allocated can still be referenced, and starting the next client from the
 * base is both correct and free. It also makes every client the *first*
 * client, which on the i.MX8MP is the difference between 32 and 13 of gl_test's
 * 38 cases: the pool's block reuse hands out memory that a bump allocation
 * does not, and this side-steps the whole question between clients.
 *
 * It does not excuse the reuse defect itself, which still bites inside a
 * single client's run and is the open half of that investigation.
 */
void gc_render_pool_reset(void)
{
	/* The deferred blocks name memory in the pool being reset, so they go
	 * with it; keeping them would free into a pool that no longer has
	 * those blocks outstanding. */
	gc_defer_init(&g_defer);
	gc_pool_reset(&g_pool);
}

/* How often the defer queue's full path runs, and how it ends. Counted
 * unconditionally: the path is silent when its wait succeeds, and a wait inside
 * a free is exactly the kind of stall a user sees as stutter. */
static struct gc_defer_full_stats s_defer_stats;

void gc_render_defer_full_stats(struct gc_defer_full_stats *out)
{
	*out = s_defer_stats;
}

/* True when `cpu` is the carveout's own view: only those blocks belong to the
 * pool. An import (gc_render_map_phys) has a CPU view of its own since
 * 2026-10-01 and must never be handed to gc_pool_free. */
static bool in_carveout(const struct gc_dev *g, const void *cpu, size_t size)
{
	const uint8_t *p = cpu;

	return g->carveout != NULL && p >= g->carveout && size <= g->carveout_size &&
	       (size_t)(p - g->carveout) <= g->carveout_size - size;
}

void gc_render_free(const struct gc_bo *bo)
{
	struct gc_dev *g = gc_dev_get();
	size_t off;
	uint32_t oldest;

	if (bo == NULL || bo->cpu == NULL || bo->size == 0 || !in_carveout(g, bo->cpu, bo->size)) {
		return;
	}
	off = (size_t)((uint8_t *)bo->cpu - g->carveout);

	if (g_defer_off) {
		gc_pool_free(&g_pool, off, bo->size);
		g->pool_double_frees = g_pool.double_frees;
		return;
	}
	/*
	 * Stamp with the kick being recorded now (fence_next): a draw already
	 * recorded but not yet submitted can still reference this block, and
	 * nothing later than that kick can. Waiting for fence_next rather than
	 * "the next kick to retire" is what keeps a block from being held
	 * longer than its own work needs.
	 */
	defer_drain(g);
	if (gc_defer_push(&g_defer, off, bo->size, g->fence_next)) {
		return;
	}
	/* Full. Wait for the oldest kick to retire and try once more. */
	s_defer_stats.full++;
	if (gc_defer_oldest(&g_defer, &oldest)) {
		int64_t t0 = k_uptime_get();
		int wrc = gc_render_wait(oldest, GC_DEFER_WAIT_MS);
		uint32_t waited = (uint32_t)(k_uptime_get() - t0);

		if (waited > s_defer_stats.max_wait_ms) {
			s_defer_stats.max_wait_ms = waited;
		}
		if (wrc == -EAGAIN) {
			/* The oldest stamp is the kick still being recorded:
			 * nothing can retire it until the caller submits. */
			s_defer_stats.full_unsubmitted++;
		}
		if (wrc == 0) {
			defer_drain(g);
			if (gc_defer_push(&g_defer, off, bo->size, g->fence_next)) {
				return;
			}
		}
	}
	s_defer_stats.leaked++;
	/*
	 * Still no room. The block is leaked -- deliberately, and loudly. The
	 * alternative is to free it now, which is the use-after-free this
	 * queue exists to prevent, and it would happen under pressure where it
	 * is least likely to be noticed. Leaking costs memory; freeing early
	 * costs a wrong picture and a day finding out why.
	 */
	LOG_ERR("defer queue full (%u slots, %u refused so far): leaking %zu bytes at +0x%zx "
		"rather than returning memory a kick may still be reading",
		(unsigned)GC_DEFER_SLOTS, g_defer.refused, bo->size, off);
}

void gc_render_free_retired(const struct gc_bo *bo)
{
	struct gc_dev *g = gc_dev_get();

	if (bo == NULL || bo->cpu == NULL || bo->size == 0 || !in_carveout(g, bo->cpu, bo->size)) {
		return;
	}
	gc_pool_free(&g_pool, (size_t)((uint8_t *)bo->cpu - g->carveout), bo->size);
	g->pool_double_frees = g_pool.double_frees;
}

int gc_render_map_phys(uint64_t phys, size_t size, struct gc_bo *out)
{
	struct gc_dev *g = gc_dev_get();
	uint64_t base = ROUND_DOWN(phys, GC_PAGE_SIZE);
	size_t len = ROUND_UP(phys + size, GC_PAGE_SIZE) - base;
	uint32_t va;
	int rc;

	
/*
	 * Memory that is already in the carveout needs no import: it is
	 * identity-mapped and it has a CPU view. Hand those back.
	 *
	 * This is not only to save import VA. The carveout is mapped Normal
	 * non-cacheable, so a second, cached CPU mapping of the same physical
	 * page would be a mismatched-attribute alias -- something the ARM
	 * architecture leaves unpredictable, and a silent source of stale
	 * pixels. A caller that renders into a buffer it allocated with
	 * gc_render_alloc() (YakoGL's EGL window surface does exactly that)
	 * must get the carveout's own mapping back.
	 */
	if (g->carveout != NULL && phys >= g->carveout_phys &&
	    size <= g->carveout_size &&
	    phys - g->carveout_phys <= g->carveout_size - size) {
		out->cpu = g->carveout + (phys - g->carveout_phys);
		out->phys = phys;
		out->gpu = (uint32_t)phys;   /* the carveout is identity-mapped */
		out->size = size;
		return 0;
	}

	if (g->import_va + len > GC_IMPORT_VA_BASE + GC_IMPORT_VA_SIZE) {
		LOG_ERR("import: no VA left for 0x%llx+%zu", (unsigned long long)phys, size);
		return -ENOMEM;
	}
	va = g->import_va;
	rc = gc_mmu_map(g, va, base, len);
	if (rc) {
		return rc;
	}
	g->import_va += (uint32_t)len;

	/*
	 * And a CPU view, Normal non-cacheable like the carveout's, because
	 * glReadPixels on a window surface reads through it (the GL core's
	 * fallback, the OSAL's gpu_read, needs one too). Without it every
	 * read-back of an imported surface came back as zeros and
	 * GL_OUT_OF_MEMORY -- route 3's window buffers in the M7 carveout,
	 * whose first-frame probes then left that error for the render loop to
	 * fail on (2026-10-01, record 594b0216 000f0d02 15050000). The
	 * attribute matches the mapping the route-3 ring made of the same pages
	 * (gc_common's safeui_ring_a53.c, removed with gc_slint in Step 1), so
	 * an importer's two views are not a mismatched-attribute alias.
	 */
	{
		uint8_t *cpu = NULL;

#ifdef K_MEM_ARM_NORMAL_NC
		k_mem_map_phys_bare(&cpu, base, len, K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
		k_mem_map_phys_bare(&cpu, base, len, K_MEM_CACHE_WB | K_MEM_PERM_RW);
#endif
		out->cpu = cpu != NULL ? cpu + (phys - base) : NULL;
	}
	out->phys = phys;
	out->gpu = va + (uint32_t)(phys - base);
	out->size = size;
	LOG_INF("import: 0x%llx+%zu at GPU VA 0x%08x", (unsigned long long)phys, size,
		out->gpu);
	return 0;
}

void gc_render_cache_flush(void *cpu, size_t n)
{
	sys_cache_data_flush_range(cpu, n);
}

void gc_render_cache_invd(void *cpu, size_t n)
{
	sys_cache_data_invd_range(cpu, n);
}

/* ------------------------------------------------------------------ *
 * Init
 * ------------------------------------------------------------------ */

#ifndef GC_LINUX_MODULE
/* The Zephyr entry point: check the device the devicetree built, then run the
 * shared initialisation. A Linux module has no such device and calls
 * gc_render_init_dev() directly (gc_render.h). */
int gc_render_init(const struct device *dev)
{
	gc_render_mark(GC_RENDER_STEP_ENTER);
	if (!device_is_ready(dev) || !gc_alive(dev)) {
		LOG_ERR("render: the GPU did not come up (milestone 1 failed)");
		return -ENODEV;
	}
	return gc_render_init_dev(gc_dev_get());
}
#endif /* !GC_LINUX_MODULE */

int gc_render_init_dev(struct gc_dev *g)
{
	int rc;

	gc_render_mark(GC_RENDER_STEP_ENTER);
	if (s_ready) {
		return 0;
	}
	if (g->regs == 0 || g->carveout_size == 0) {
		LOG_ERR("render: the device did not publish its registers or carveout");
		return -ENODEV;
	}

	gc_render_mark(GC_RENDER_STEP_DEV_READY);
	g->event_free = GC_EVENT_ALL;
	g->fence_next = 1;
	g->fence_done = 0;
	gc_pool_init(&g_pool, GC_OFF_POOL, g->carveout_size);
	k_sem_init(&g->event_room, 0, GC_EVENT_COUNT);
	k_mutex_init(&g->submit_lock);

	rc = map_carveout(g);
	if (rc) {
		return rc;
	}
	gc_render_mark(GC_RENDER_STEP_CARVEOUT);
#ifdef CONFIG_YAKOGL_GC_CARVEOUT_FILL
	/* A measurement (zephyr/Kconfig): the pool as a fixed pattern rather
	 * than the previous boot's leftovers, before the GPU can read any of
	 * it. The MMU tables below GC_OFF_POOL are built by gc_mmu_init and
	 * must not see a pattern that could read as a valid entry. */
	{
		uint32_t t0 = k_uptime_get_32();

		memset(g->carveout + GC_OFF_POOL, CONFIG_YAKOGL_GC_CARVEOUT_FILL_BYTE,
		       g->carveout_size - GC_OFF_POOL);
		LOG_INF("carveout: pool filled with 0x%02x (%zu MiB, %u ms)",
			CONFIG_YAKOGL_GC_CARVEOUT_FILL_BYTE,
			(g->carveout_size - GC_OFF_POOL) / (1024 * 1024),
			k_uptime_get_32() - t0);
	}
#endif
	/* The order is fixed by the hardware: the register init has to grant
	 * non-secure AHB access before anything else is reachable, the page
	 * tables have to exist before the MMU is switched on, the MMU has to be
	 * on before the front end is started on a virtual address, and the
	 * interrupt has to be connected before the first event can fire. */
	rc = gc_hw_init(g);
	if (rc) {
		return rc;
	}
	gc_render_mark(GC_RENDER_STEP_HW);
	rc = gc_mmu_init(g);
	if (rc) {
		return rc;
	}
	gc_render_mark(GC_RENDER_STEP_MMU);
	gc_irq_connect(g);
	gc_render_mark(GC_RENDER_STEP_IRQ);
	rc = gc_mmu_enable(g);
	if (rc) {
		return rc;
	}
	gc_render_mark(GC_RENDER_STEP_MMU_ON);
	rc = gc_ring_init(g);
	if (rc) {
		return rc;
	}
	gc_render_mark(GC_RENDER_STEP_RING);
	s_ready = true;
	LOG_INF("render service up: %u events, pool %zu KiB free",
		GC_EVENT_COUNT, (g->carveout_size - GC_OFF_POOL) / 1024);
	return 0;
}

/* ------------------------------------------------------------------ *
 * Submit and wait
 * ------------------------------------------------------------------ */

int gc_render_submit(const struct gc_submit *s, uint32_t *fence_out)
{
	struct gc_dev *g = gc_dev_get();
	uint32_t fence;
	int event, rc;

	if (!s_ready) {
		return -ENODEV;
	}
	/* One writer of the ring at a time: a submit appends a segment and
	 * then patches the previous idle loop, and two of those interleaved
	 * would lose a segment. Blocking here is correct -- waiting for an
	 * event to come free inside the lock is exactly the back pressure a
	 * second caller should feel. */
	k_mutex_lock(&g->submit_lock, K_FOREVER);

	K_SPINLOCK(&g->lock) {
		fence = g->fence_next++;
		if (g->fence_next == 0) {
			g->fence_next = 1;   /* 0 means "nothing submitted" */
		}
	}
	event = gc_event_alloc(g, fence, 1000);
	if (event < 0) {
		k_mutex_unlock(&g->submit_lock);
		return event;
	}
	/* gc_render_perf: stamped before the ring is committed, so the
	 * interrupt can never see the event before its start time. */
	g->event_t0_ns[event] = gc_now_ns();
	rc = gc_ring_submit(g, s, (uint32_t)event, fence);
	if (rc) {
		K_SPINLOCK(&g->lock) {
			g->event_fence[event] = 0;
			g->event_free |= BIT(event);
		}
		k_sem_give(&g->event_room);
		k_mutex_unlock(&g->submit_lock);
		return rc;
	}
	k_mutex_unlock(&g->submit_lock);
	*fence_out = fence;
	return 0;
}

void gc_render_perf(struct gc_render_perf *out)
{
	struct gc_dev *g = gc_dev_get();

	K_SPINLOCK(&g->lock) {
		out->gpu_busy_ns = g->perf_busy_ns;
		out->kicks_done = g->perf_kicks_done;
	}
	out->wait_ns = g->perf_wait_ns;
	out->waits = g->perf_waits;
	out->wait_sleeps = g->perf_wait_sleeps;
	/* The front end's own clock and idle counters (etnaviv perfmon's
	 * HI TOTAL_CYCLES / IDLE_CYCLES) and the memory traffic: 32-bit
	 * free-running, so a reader takes differences over less than a wrap
	 * (5 s at 800 MHz). */
	out->hi_total_cycles = gc_rd(g, VIVS_HI_PROFILE_TOTAL_CYCLES);
	out->hi_idle_cycles = gc_rd(g, VIVS_HI_PROFILE_IDLE_CYCLES);
	out->hi_read_bytes8 = gc_rd(g, VIVS_HI_PROFILE_READ_BYTES8);
	out->hi_write_bytes8 = gc_rd(g, VIVS_HI_PROFILE_WRITE_BYTES8);
	/* the module counters gc_render_perf_select() chose */
	out->mc_pe = gc_rd(g, VIVS_MC_PROFILE_PE_READ);
	out->mc_sh = gc_rd(g, VIVS_MC_PROFILE_SH_READ);
	out->mc_ra = gc_rd(g, VIVS_MC_PROFILE_RA_READ);
}

void gc_render_perf_select(uint32_t config0, uint32_t config1)
{
	struct gc_dev *g = gc_dev_get();

	gc_wr(g, VIVS_MC_PROFILE_CONFIG0, config0);
	gc_wr(g, VIVS_MC_PROFILE_CONFIG1, config1);
}

uint32_t gc_render_completed(void)
{
	return gc_dev_get()->fence_done;
}

void gc_render_counters(uint32_t *fence_timeouts, uint32_t *event_busy,
			uint32_t *wait_ms)
{
	struct gc_dev *g = gc_dev_get();

	if (fence_timeouts) {
		*fence_timeouts = g->fence_timeouts;
	}
	if (event_busy) {
		*event_busy = g->event_busy;
	}
	if (wait_ms) {
		*wait_ms = g->wait_ms;
	}
}

static bool fence_done(struct gc_dev *g, uint32_t fence)
{
	return (int32_t)(g->fence_done - fence) >= 0;
}

/* Was this fence one a reset abandoned? Signed difference, like fence_done,
 * so it keeps working when the 32-bit counter wraps. fence_lost is 0 until
 * the first recovery and fence 0 is never handed out, so before any reset
 * this is false for every real fence. */
static bool fence_lost(struct gc_dev *g, uint32_t fence)
{
	return g->fence_lost != 0 && (int32_t)(g->fence_lost - fence) >= 0;
}

/* Reset the core and bring registers, MMU hardware and ring back, in the order
 * gc_render_init_dev() established them. The caller holds submit_lock. Shared by
 * the fence-timeout recovery and the idle reset between clients, so the two can
 * never drift apart. */
static int reset_reinit(struct gc_dev *g, const char *who)
{
	int rc;

	/* The reset invalidates both of these before it runs, so that a failure
	 * partway through cannot leave the rest of the driver believing the
	 * front end is running or the MMU is on. */
	g->fe_running = false;
	g->mmu_on = false;

	rc = gc_bringup_reset(g->regs, g->id);
	if (rc) {
		/* Nothing further is safe: the GPU did not reach idle, so it
		 * may still be reading memory we would otherwise release. Stay
		 * broken and loud rather than guess. */
		LOG_ERR("%s: the reset did not take (%d); the GPU is left stopped", who, rc);
		return rc;
	}

	/* Everything the reset undid, in the order gc_render_init_dev() fixed:
	 * registers, MMU, ring. Two things there are deliberately NOT repeated.
	 *
	 * gc_irq_connect() is not called again: it registers a handler with the
	 * host, which a GPU reset does not touch, and connecting it twice is a
	 * host-level error.
	 *
	 * gc_mmu_init() is not called again either, and that one matters. The
	 * page tables live in the carveout; a core reset clears registers, not
	 * memory, so the MTLB, the slave tables and every mapping in them are
	 * still exactly as they were. Rebuilding them would be pointless work
	 * and, worse, actively destructive: gc_mmu_init() wipes the MTLB and
	 * rewinds import_va, so every dma-buf a client had imported would lose
	 * its GPU address while the client still held it, and the next import
	 * would hand out the same address again. Only the MMU HARDWARE has to
	 * be re-established, which is what gc_mmu_enable() does -- and it is
	 * what etnaviv does too: etnaviv_iommu_restore() reprograms the MMU
	 * registers from a context whose page tables it never rebuilds.
	 */
	rc = gc_hw_init(g);
	if (rc == 0) {
		rc = gc_mmu_enable(g);
	}
	if (rc == 0) {
		rc = gc_ring_init(g);
	}
	if (rc) {
		LOG_ERR("%s: re-init failed (%d) after a reset that took; "
			"the GPU is idle but unusable",
			who, rc);
		return rc;
	}
	return 0;
}

/* ------------------------------------------------------------------ *
 * Recovery
 *
 * A fence that does not complete used to be the end of the road: the driver
 * logged the stall dump and returned -ETIMEDOUT, and every later submit went
 * into a ring the front end was no longer reading. One hung kick took the GPU
 * out until somebody with root reloaded the module -- which, on a board the
 * team cannot get sudo on, means out for the day. That happened: a dEQP sweep
 * wedged the i.MX8MP after 1,431 cases with twelve events in flight and
 * nothing retiring.
 *
 * The reason there was no recovery was written down honestly in gc_render.h:
 * reclaiming a timed-out kick's event slot is only safe if the GPU has
 * certainly stopped running it, and "certainly stopped" means a reset. The
 * driver HAS a reset -- gc_bringup_reset(), which every boot runs and which is
 * therefore the best-tested sequence in this file. It was simply never wired
 * to the timeout. That is what this function is.
 *
 * After the reset the old command stream does not exist, so the late-interrupt
 * hazard that made recycling unsafe is gone. Every step below rests on that
 * one fact.
 *
 * What it must NOT do is let the abandoned work look successful. Two questions
 * share one number today and they are not the same question:
 *
 *   "is the GPU still reading this memory?"   -- what the OSAL's deferred free
 *                                                asks. After a reset: no, for
 *                                                everything. gc_render_completed()
 *                                                keeps answering that.
 *   "did my kick produce its picture?"        -- what a waiter asks. For work
 *                                                the reset threw away: NO, and
 *                                                gc_render_wait() must say so
 *                                                rather than return 0.
 *
 * So the recovery advances fence_done (memory is free) and records fence_lost
 * (results are not). A waiter on an abandoned fence gets -ECANCELED, which the
 * GL layer reports as a lost context. Reporting failure for a picture that was
 * never drawn is the whole point: a recovery that quietly returned success
 * would turn a visible hang into invisible wrong output, which is worse than
 * the hang it replaced.
 */
int gc_render_recover(const char *why)
{
	struct gc_dev *g = gc_dev_get();
	uint32_t abandoned;
	int rc;

	if (!s_ready) {
		return -ENODEV;
	}
	/* Hold the ring against new writers for the whole sequence: between
	 * the reset and gc_ring_init() there is no ring to write into. */
	k_mutex_lock(&g->submit_lock, K_FOREVER);
	abandoned = g->fence_next - 1;
	LOG_ERR("recovering the GPU (%s): fences %u..%u abandoned, %u already done", why,
		g->fence_done + 1, abandoned, g->fence_done);

	rc = reset_reinit(g, "recover");
	if (rc) {
		k_mutex_unlock(&g->submit_lock);
		return rc;
	}

	/* Retire the abandoned work. fence_done moves so that memory is
	 * released and no later wait blocks on a fence the GPU will never
	 * signal; fence_lost records that those results do not exist.
	 *
	 * Every event slot comes back, and it is the reset that makes that
	 * safe: the stream that would have raised the stale interrupts is
	 * gone. event_room is deliberately NOT re-initialised -- a thread may
	 * be blocked on it inside gc_event_alloc, and re-initialising a
	 * semaphore under a waiter is undefined. Such a waiter times out
	 * within its bound, re-reads event_free, and finds all thirty free.
	 */
	g->fence_lost = abandoned;
	g->fence_done = abandoned;
	g->event_free = GC_EVENT_ALL;
	g->resets++;
	LOG_ERR("recover: GPU reset and re-initialised (reset %u); "
		"fences through %u report a lost context",
		g->resets, abandoned);
	k_mutex_unlock(&g->submit_lock);
	return 0;
}

/*
 * Return an idle GPU to its just-initialised state, for the next client.
 *
 * Measured on the i.MX8MP, 2026-09-23: one run of gl_test 63 (mat2/3/4 vertex
 * attributes) through an etna_shc-compiled program left register state behind
 * that every LATER client inherited -- fresh processes on any pair drew frames
 * of 0x00000000, the clear included, with no fence timeout and no kernel line.
 * Zeroing every allocation (scrub_allocs) did not prevent it; a core reset
 * cleared it. So the service resets the core between clients: a client can no
 * longer be graded on state another one left.
 *
 * This is containment, not the fix -- the backend must program what its draws
 * depend on -- and it is QUIET: it is not a recovery, nothing is abandoned, no
 * fence is marked lost, and it never logs the wording the recovery counters
 * look for. It refuses with -EBUSY while any fence is outstanding.
 */
int gc_render_reset_idle(void)
{
	struct gc_dev *g = gc_dev_get();
	int rc;

	if (!s_ready) {
		return -ENODEV;
	}
	k_mutex_lock(&g->submit_lock, K_FOREVER);
	if (g->fence_done != (uint32_t)(g->fence_next - 1)) {
		k_mutex_unlock(&g->submit_lock);
		LOG_WRN("idle reset refused: fences through %u submitted, %u done",
			g->fence_next - 1, g->fence_done);
		return -EBUSY;
	}
	rc = reset_reinit(g, "idle reset");
	if (rc == 0) {
		g->event_free = GC_EVENT_ALL;
		g->idle_resets++;
		LOG_INF("idle reset %u: GPU returned to its initial state for the next client",
			g->idle_resets);
	}
	k_mutex_unlock(&g->submit_lock);
	return rc;
}

uint32_t gc_render_resets(void)
{
	return gc_dev_get()->resets;
}

uint32_t gc_render_max_quiet_ms(void)
{
	return gc_dev_get()->max_quiet_ms;
}

int gc_render_wait(uint32_t fence, uint32_t timeout_ms)
{
	struct gc_dev *g = gc_dev_get();
	uint32_t errors_before = g->irq_errors;
	/* Time is the clock's, never a count of sleeps. The loop sleeps "1 ms",
	 * which on Zephyr is a millisecond and on the Linux harness is
	 * msleep(1): at least two jiffies, 20..32 ms measured at CONFIG_HZ=100.
	 * Counting iterations made every bound ~30x its stated value there --
	 * "did not complete in 1000 ms" was logged 32 s after the wait began
	 * (dmesg, 2026-09-23 05:51:50 -> 05:52:22 JST). */
	const int64_t t0 = k_uptime_get();
	int64_t progress_at = t0;
	uint32_t done_seen = g->fence_done;
	const char *why = "fence timeout";
	uint32_t ms;

	/* A fence that has not been handed out yet cannot be retired by
	 * waiting, and a timeout on it would reset a GPU that is healthy. The
	 * driver itself asks this: gc_render_free() stamps deferred blocks with
	 * fence_next (the kick being recorded), and a full defer queue waits on
	 * the oldest stamp, which is fence_next when every entry is younger than
	 * the last submit. Say so without waiting, and never recover for it.
	 * Every fence at or below fence_next - 1 was submitted before its id
	 * could reach a caller, so from here on some work is outstanding for as
	 * long as the fence is not done -- which is what the hang bound needs. */
	if ((int32_t)(fence - (uint32_t)(g->fence_next - 1)) > 0) {
		return -EAGAIN;
	}
	if (fence_done(g, fence)) {
		return fence_lost(g, fence) ? -ECANCELED : 0;
	}
	const uint64_t perf_t0 = gc_now_ns();

	g->perf_waits++;

	for (;;) {
		int64_t now;

		if (fence_done(g, fence)) {
			g->wait_ms += (uint32_t)(k_uptime_get() - t0);
			g->perf_wait_ns += gc_now_ns() - perf_t0;
			/* fence_done also advances over work a reset threw
			 * away, so "done" alone does not mean "drawn". */
			if (fence_lost(g, fence)) {
				return -ECANCELED;
			}
			return 0;
		}
		if (g->irq_errors != errors_before) {
			gc_render_stall_dump("AXI bus error or MMU exception");
			return -EIO;
		}
		now = k_uptime_get();
		ms = (uint32_t)(now - t0);
		if (timeout_ms != GC_WAIT_FOREVER) {
			if (ms >= timeout_ms) {
				break;
			}
		} else {
			/* A deadline-free wait still ends on a hang: nothing
			 * retired for s_hang_ms. Any retirement restarts the
			 * clock, so a long but moving queue never trips it. */
			if (g->fence_done != done_seen) {
				done_seen = g->fence_done;
				progress_at = now;
			}
			/* The instrument for the bound: the longest stretch any
			 * waiter saw with work outstanding and nothing retiring.
			 * A legitimate kick that ran close to hang_ms shows here
			 * before the bound is ever made the default. */
			if ((uint32_t)(now - progress_at) > g->max_quiet_ms) {
				g->max_quiet_ms = (uint32_t)(now - progress_at);
			}
			if (s_hang_ms != 0 && now - progress_at >= s_hang_ms) {
				why = "no fence retired within hang_ms";
				break;
			}
			/* A wait with no deadline must still end when the
			 * waiting thread is told to stop -- otherwise a GPU
			 * that retires nothing leaves an unkillable process
			 * behind. A bounded wait ends on its own, and its
			 * caller may be the cleanup of a process that is
			 * already dying, so it is never cut short. On Zephyr
			 * this is constant false and the test folds away. */
			if (gc_wait_aborted()) {
				LOG_WRN("fence %u: wait abandoned after %u ms at the "
					"caller's request (completed %u)",
					fence, ms, g->fence_done);
				return -EINTR;
			}
		}
		g->perf_wait_sleeps++;
		k_sleep(K_MSEC(1));
	}
	g->perf_wait_ns += gc_now_ns() - perf_t0;
	if (fence_done(g, fence)) {
		return fence_lost(g, fence) ? -ECANCELED : 0;
	}
	g->fence_timeouts++;
	g->wait_ms += ms;
	if (timeout_ms == GC_WAIT_FOREVER) {
		LOG_ERR("fence %u: nothing retired for %u ms, waited %u ms (completed %u)",
			fence, (uint32_t)(k_uptime_get() - progress_at), ms, g->fence_done);
	} else {
		LOG_ERR("fence %u did not complete in %u ms (completed %u)", fence, ms,
			g->fence_done);
	}
	gc_render_stall_dump(why);
	/* Put the GPU back in a usable state before returning. A failed
	 * recovery leaves exactly the situation this code replaced -- a
	 * stopped GPU -- so it can only improve on doing nothing, and the
	 * caller is told the truth either way. */
	if (gc_render_recover(why) == 0) {
		return -ECANCELED;
	}
	return -ETIMEDOUT;
}


void gc_render_stall_dump(const char *why)
{
	struct gc_dev *g = gc_dev_get();
	uint32_t inflight = 0;
	uint32_t idle;

	LOG_ERR("=== GC7000 stall dump: %s ===", why);
	LOG_ERR("fences: next %u, completed %u; irq errors %u", g->fence_next, g->fence_done,
		g->irq_errors);
	idle = gc_rd(g, VIVS_HI_IDLE_STATE);
	LOG_ERR("hi: CLOCK_CONTROL 0x%08x IDLE_STATE 0x%08x AXI_STATUS 0x%08x INTR_ENBL 0x%08x",
		gc_rd(g, VIVS_HI_CLOCK_CONTROL), idle, gc_rd(g, VIVS_HI_AXI_STATUS),
		gc_rd(g, VIVS_HI_INTR_ENBL));
	gc_idle_state_decode(idle);
	/* Did anything move since the last dump? A GPU that is grinding through
	 * work slowly and one that is frozen print the same registers; only the
	 * difference between two dumps separates them. This costs two words of
	 * state and saves reading `sort -u` over an hour of logs, which is how
	 * it was established the first time. */
	if (g->dump_seen && g->dump_idle == idle && g->dump_fence == g->fence_done) {
		LOG_ERR("state: UNCHANGED since the previous dump (idle state and "
			"completed fence both identical) -- frozen, not progressing");
	}
	g->dump_idle = idle;
	g->dump_fence = g->fence_done;
	g->dump_seen = true;
	LOG_ERR("pm: POWER_CONTROLS 0x%08x MODULE_STATUS 0x%08x PULSE_EATER 0x%08x",
		gc_rd(g, VIVS_PM_POWER_CONTROLS), gc_rd(g, VIVS_PM_MODULE_STATUS),
		gc_rd(g, VIVS_PM_PULSE_EATER));
	gc_ring_dump(g);
	gc_mmu_dump(g);
	/* No shader-state read-back here, and none anywhere: reading the
	 * 0x0800+ state registers from the CPU resets this board. Two builds
	 * that did it took the board down, one on a wedged GPU and one on a
	 * GPU that had just completed a submit, against zero reboots in dozens
	 * of runs without them. The HI/PM/MC registers above are a different
	 * space and have been read all day without incident. See
	 * driver/docs/vivante-kernel-rust.md, "Hardware findings that still
	 * hold". */

	for (uint32_t i = 0; i < GC_EVENT_COUNT; i++) {
		if (!(g->event_free & BIT(i))) {
			LOG_ERR("  event %u in flight for fence %u", i, g->event_fence[i]);
			inflight++;
		}
	}
	LOG_ERR("=== %u events in flight ===", inflight);
}
