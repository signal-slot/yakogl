/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_gc.c - Zephyr OSAL backend for YakoGL on the Vivante GC7000UL
 * (i.MX8MP). The sibling of zephyr.c, which does the same job for the
 * PowerVR BXS-4-64; the build compiles exactly one of them, and both define
 * `osal_zephyr_ops`, so nothing above the OSAL changes with the GPU.
 *
 * Everything here forwards to the driver's render service (src/vivante/
 * gc_render.h), which owns the carveout, the MMU v2 page tables, the front
 * end's ring, the events and the stall dump:
 *
 *   init        -> gc_render_init(DEVICE_DT_GET_ONE(vivante_gc))
 *   gpu_alloc   -> gc_render_alloc()   (the driver's pool)
 *   gpu_free    -> gc_render_free(), once the kicks that may read it are done
 *   map_phys    -> gc_render_map_phys()
 *   submit      -> gc_render_submit()  (kick = struct gc_kick)
 *   wait        -> gc_render_wait()
 *   completed   -> gc_render_completed()
 *   cache_*     -> gc_render_cache_*()
 *
 *   query       -> one question only: OSAL_QUERY_GC_FRAME_OPEN, which the
 *                  backend uses to say whether it is recording a frame (it
 *                  decides how long a free is held; gc_frees.h)
 *
 * Two things are deliberately absent, because this GPU has no equivalent:
 *
 *   - mmu_map(). The service maps at a VA it chooses; there is no public call
 *     to map an arbitrary one. gl_egl.c needs map_phys OR mmu_map, and
 *     map_phys is the better of the two anyway (it consumes no pool memory).
 *   - reg_read()/reg_write(). The Vivante backend writes a command stream and
 *     never touches a register; the driver keeps the register block private.
 *
 * Heap bases are ignored on purpose: the backend separates its pools with the
 * device-VA bases in gc_backend.c, but the carveout is identity-mapped, so the
 * address an allocation actually gets is its physical address. Nothing in the
 * GL core or the backend does arithmetic against a heap base, so the bases stay
 * what they are -- labels for which pool a request came from.
 */

#include <stdarg.h>
#include <stdbool.h>
#include <string.h>
#include <errno.h>

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/cache.h>
#include <zephyr/sys/util.h>
#include <zephyr/logging/log.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_bounds.h"
#include "osal_log.h"
#include "zephyr_clock.h"
#include "gc_kick.h"
#include "gc_render.h"
#include "gc_frees.h"

LOG_MODULE_REGISTER(gles_osal_gc, CONFIG_YAKOGL_LOG_LEVEL);

/* Every allocation is command-buffer aligned. It is the strictest of the
 * three alignments the hardware asks for (GC_ALIGN_CMDBUF == GC_ALIGN_SHADER
 * == GC_ALIGN_DEFAULT == 64), so one value covers command streams, shader
 * code and data without the OSAL having to guess what a request is for. */
#define GC_OSAL_ALIGN  GC_ALIGN_CMDBUF
/* a GEM buffer's alignment, for CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN */
#define GC_PAGE_SIZE_OSAL 4096u

static struct {
	bool inited;
} s;

/* ---- lifecycle ---------------------------------------------------- */
static int z_init(void *platform_arg)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	int rc;

	/* struct osal_zephyr_init carries a PowerVR register base; this driver
	 * maps its own registers from the device tree and takes no argument. */
	(void)platform_arg;

	if (s.inited)
		return 0;
	if (!device_is_ready(gpu)) {
		LOG_ERR("osal(gc): the vivante,gc device did not initialise");
		return -ENODEV;
	}
	rc = gc_render_init(gpu);
	if (rc) {
		LOG_ERR("osal(gc): render service init failed (%d)", rc);
		return rc;
	}
	s.inited = true;
	LOG_INF("osal(gc): ready");
	return 0;
}

static void report_pending(const char *what);

static void z_shutdown(void)
{
	report_pending(""); /* the pending-free queue's high-water and refusals */
	s.inited = false;
	/* The GPU stays powered and running its idle loop (owned by the DT
	 * driver); the bump allocator has nothing to give back. */
}

/* ---- GPU memory + MMU --------------------------------------------- */

/* struct gc_bo and struct osal_gpu_mem hold the same four things; the GPU
 * address is 32-bit here because the GC7000's MMU is. */
static void bo_to_mem(const struct gc_bo *bo, struct osal_gpu_mem *out)
{
	out->cpu  = bo->cpu;
	out->gpu  = (uint64_t)bo->gpu;
	out->phys = bo->phys;
	out->size = bo->size;
	out->impl = (void *)(uintptr_t)bo->gpu;   /* non-NULL marks it live */
}

/* CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN (a measurement, off by default): an
 * allocation of a page or more starts on a page, as a GEM buffer does. The
 * color_clear A/B: dEQP's results change with the image, deterministically,
 * and a 64-byte-aligned render target shares its first and last 256-byte PE
 * cache lines with its neighbours. Smaller blocks keep 64, so command
 * streams and uniforms pack as before. */
static size_t alloc_align(size_t size)
{
#ifdef CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN
	if (size >= GC_PAGE_SIZE_OSAL)
		return GC_PAGE_SIZE_OSAL;
#else
	(void)size;
#endif
	return GC_OSAL_ALIGN;
}

static void drain_frees(void);

static int z_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags,
		       struct osal_gpu_mem *out)
{
	struct gc_bo bo;
	int rc;

	(void)heap_base;   /* see the file comment: the carveout is identity-mapped */
	(void)flags;       /* the carveout is Normal non-cacheable for every user */
	if (!s.inited || !out || size == 0)
		return -EINVAL;
	/* Frees whose kicks have completed go back before the pool is asked;
	 * before, they only returned on the next free or submit. */
	drain_frees();
	rc = gc_render_alloc(size, alloc_align(size), &bo);
#ifdef CONFIG_YAKOGL_GC_LOG_BIG_ALLOCS
	{
		/* A measurement: who asks for a megabyte or more, the first 16
		 * times (addr2line the two return addresses). */
		static uint32_t n_big;

		if (size >= (1u << 20) && n_big++ < 16)
			printk("osal(gc): alloc %zu bytes (rc %d) from %p <- %p\n", size, rc,
			       __builtin_return_address(0), __builtin_return_address(1));
	}
#endif
	if (rc)
		return rc;
	bo_to_mem(&bo, out);
	return 0;
}

/*
 * Frees are deferred (src/osal/gc_frees.h has the rule and its host check).
 *
 * The GL core releases a texture or a buffer as soon as the application
 * stops needing it, but the frame being recorded and every kick still in
 * flight may read it. So a freed block goes back to the driver's pool only
 * once the last kick that may read it has completed: the frame being
 * recorded while the backend records one (it says so with
 * OSAL_QUERY_GC_FRAME_OPEN), else the last kick submitted. A block whose
 * kick has completed goes straight back (gc_render_free_retired): it has
 * waited for its own kick already, and gc_render_free() would hold it one
 * kick more.
 *
 * This used to be unnecessary here and is not any more: gc_render_free()
 * reclaimed nothing but the most recent allocation, so a premature free was
 * a leak, not a corruption. Now that the pool has a free list, memory handed
 * back early is handed straight out again -- to a caller that writes it
 * while the GPU is still reading it.
 */
static struct gc_frees s_frees = { .frame_open = true };
static uint32_t s_last_submitted;

static void release_retired(const struct gc_frees_block *b)
{
	const struct gc_bo bo = { .cpu = b->cpu, .gpu = b->gpu, .phys = b->phys, .size = b->size };

	gc_render_free_retired(&bo);
}

static void drain_frees(void)
{
	gc_frees_drain(&s_frees, gc_render_completed(), release_retired);
}

/* The free counters (gc_frees.h), for a measurement to read: frees given back
 * at once, held frees given back by a drain, the held bytes' high-water and
 * what refused frees kept (a full queue, pending_frees.h). */
void gc_osal_frees_stats(uint32_t *at_once, uint32_t *drained, uint32_t *held_peak_kib, uint32_t *leaked_kib)
{
	*at_once = s_frees.freed_at_once;
	*drained = s_frees.drained;
	*held_peak_kib = (uint32_t)(s_frees.held_peak_bytes >> 10);
	*leaked_kib = s_frees.q.refused_kib;
}

/* The queue's counters, every run: at close and at the first refusal.
 * printk, not LOG_*, so no log level makes a refusal silent. */
static void report_pending(const char *what)
{
	char line[160];

	pf_format(&s_frees.q, line, sizeof(line));
	printk("osal(gc): %s%s, held peak %u KiB\n", what, line,
	       (unsigned int)(s_frees.held_peak_bytes >> 10));
}

void gc_backend_ds_report(char *buf, size_t cap); /* src/vivante/ds_cache.rs */

/* osal_internal.h: the close record, every run; the OSAL stays up */
void osal_zephyr_diag_print(void)
{
	char ds[160];

	report_pending("");
	gc_backend_ds_report(ds, sizeof(ds));
	printk("osal(gc): %s\n", ds);
}

void osal_mem_line(char *buf, size_t n)
{
	size_t live, bumped, total, dropped, largest;

	gc_render_pool_usage(&live, &bumped, &total, &dropped, &largest);
	gc_frees_count(&s_frees); /* held_bytes as the queue stands now */
	snprintk(buf, n,
		 "osal(gc) mem: pool live %zu, handed out once %zu (the high-water), of %zu bytes, dropped %zu; "
		 "frees held %u (%zu bytes); largest free block %zu bytes",
		 live, bumped, total, dropped, s_frees.q.n, s_frees.held_bytes, largest);
}

/* pending_frees.h's wait, bounded as z_wait bounds the backend's (below) */
static int wait_pending(uint32_t fence, uint32_t *completed);

/* CONFIG_YAKOGL_GC_POOL_NO_REUSE (a measurement, off by default): a freed
 * block never goes back to the pool, so every allocation comes from the bump
 * and gets clean, ascending addresses -- the Zephyr twin of gcgpu.ko's
 * pool_no_reuse. The pool's block reuse has an open defect that "still bites
 * inside a single client's run" (gc_render.c, above gc_render_pool_reset),
 * and dEQP's color_clear on the i.MX8MP changes result with the image,
 * deterministically. It leaks by design; what it leaked is counted so a run
 * that ran the carveout dry is told apart from one that did not. */
static uint32_t s_no_reuse_leaked_kib;

uint32_t gc_osal_no_reuse_leaked_kib(void)
{
	return s_no_reuse_leaked_kib;
}

static int z_gpu_free(struct osal_gpu_mem *mem)
{
	int rc = 0;

	if (!mem)
		return 0;
#ifdef CONFIG_YAKOGL_GC_POOL_NO_REUSE
	if (mem->impl && mem->cpu && mem->size) {
		if (s_no_reuse_leaked_kib == 0)
			LOG_WRN("osal(gc): CONFIG_YAKOGL_GC_POOL_NO_REUSE: freed memory is never "
				"reused (a measurement; it leaks)");
		s_no_reuse_leaked_kib += (uint32_t)((mem->size + 1023) / 1024);
	}
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return 0;
#endif
	if (mem->impl && mem->cpu && mem->size) {
		const struct gc_frees_block b = { .cpu = mem->cpu, .gpu = (uint32_t)mem->gpu,
						  .phys = mem->phys, .size = mem->size };

		rc = gc_frees_free(&s_frees, &b, s_last_submitted, gc_render_completed(),
				   PF_WAIT(wait_pending), release_retired);
		if (rc && s_frees.q.refused == 1)
			report_pending("a free refused, memory kept (the first; the rest are counted): ");
	}
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return rc;
}

static int z_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
		      struct osal_gpu_mem *out)
{
	struct gc_bo bo;
	int rc;

	(void)heap_base;
	(void)flags;
	if (!s.inited || !out || size == 0)
		return -EINVAL;
	rc = gc_render_map_phys(phys, size, &bo);
	if (rc)
		return rc;
	bo_to_mem(&bo, out);
	/* A window surface inside the carveout comes back with the carveout's
	 * own CPU view, which is what glReadPixels reads through. A buffer
	 * outside it has none, and the GL core falls back to gpu_read -- which
	 * cannot serve it either, so say so once rather than return black
	 * pixels quietly. */
	if (!out->cpu)
		LOG_WRN("osal(gc): 0x%llx+%zu is outside the carveout; it has no CPU view, "
			"so glReadPixels on it will fail", (unsigned long long)phys, size);
	return 0;
}

static int z_gpu_write(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, src, n))
		return -EINVAL;
	memcpy((uint8_t *)mem->cpu + off, src, n);
	/* A no-op on the carveout's non-cacheable mapping, kept so the call is
	 * correct if a cached buffer is ever handed in. */
	gc_render_cache_flush((uint8_t *)mem->cpu + off, n);
	return 0;
}

static int z_gpu_read(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, dst, n))
		return -EINVAL;
	gc_render_cache_invd((uint8_t *)mem->cpu + off, n);
	memcpy(dst, (uint8_t *)mem->cpu + off, n);
	return 0;
}

/* ---- cache maintenance -------------------------------------------- */
static void z_cache_flush(void *cpu, size_t n)
{
	if (cpu && n)
		gc_render_cache_flush(cpu, n);
}

static void z_cache_invd(void *cpu, size_t n)
{
	if (cpu && n)
		gc_render_cache_invd(cpu, n);
}

/* ---- submit + completion ------------------------------------------ */
/* Time spent inside gc_render_submit (z_submit), in cycles. */
static uint64_t s_submit_cycles;

static int64_t z_submit(const void *kick_desc, size_t kick_desc_len)
{
	const struct gc_kick *k = kick_desc;
	struct gc_submit sub;
	uint32_t fence = 0;
	int rc;

	if (!s.inited || !k || kick_desc_len < sizeof(*k))
		return -EINVAL;
	if (k->magic != GC_KICK_MAGIC) {
		LOG_ERR("osal(gc): kick magic 0x%08x is not a GC7000 kick", k->magic);
		return -EINVAL;
	}
	/* The front end fetches 64-bit words and the service appends its
	 * epilogue at the end of the stream, so a stream that is not a whole
	 * number of them would desynchronise the fetch. */
	if (k->stream_bytes == 0 || (k->stream_bytes & 7u) != 0) {
		LOG_ERR("osal(gc): stream of %u bytes is not a multiple of 8", k->stream_bytes);
		return -EINVAL;
	}
	if (k->stream_gpu > UINT32_MAX) {
		LOG_ERR("osal(gc): stream address 0x%llx is outside the 32-bit GPU VA space",
			(unsigned long long)k->stream_gpu);
		return -EINVAL;
	}

	sub.cmd_gpu   = (uint32_t)k->stream_gpu;
	sub.cmd_bytes = k->stream_bytes;
	/* Flush everything: the backend does not tell the OSAL which caches its
	 * frame dirtied, and a missing flush shows a stale picture rather than
	 * an error. gc_render.h names this "no flush bit set". */
	sub.flags     = GC_SUBMIT_FLUSH_ALL;

	{
		/* A submit blocks while every event is in flight (gc_event_alloc,
		 * up to 1 s): time the GPU holds the CPU that no fence wait shows. */
		const uint64_t c0 = k_cycle_get_64();

		rc = gc_render_submit(&sub, &fence);
		s_submit_cycles += k_cycle_get_64() - c0;
	}
	if (rc)
		return rc;
	s_last_submitted = fence;
	/* Whatever the kicks that have already finished were holding can go
	 * back to the pool now; doing it here keeps the list short without a
	 * timer or a thread of its own. */
	drain_frees();
	return (int64_t)fence;
}

static int64_t z_completed(void)
{
	if (!s.inited)
		return 0;
	return (int64_t)gc_render_completed();
}

/* Longest a single frame may take before the driver dumps the hardware state
 * and gives up. The lab rule is that nothing waits forever: a board booted by
 * kexec has no way back except its own reset, so a stalled GPU has to become a
 * failed sample rather than a hung one. Two seconds is far beyond any frame
 * this backend draws and well inside the inherited watchdog's window. */
#define GC_WAIT_FOREVER_MS 2000u

/* Every wait for a kick, counted: how many, how many timed out, and the
 * time spent in them (osal_gc_wait_stats). A board without a console reads
 * these around a frame to tell a frame that waited on the GPU from one that
 * spent its time on the CPU. */
static struct {
	uint32_t waits, timeouts;
	uint64_t cycles, max_cycles;
} s_waits;

static int timed_wait(uint32_t fence, uint32_t ms)
{
	const uint64_t c0 = k_cycle_get_64();
	int rc = gc_render_wait(fence, ms);
	const uint64_t dc = k_cycle_get_64() - c0;

	s_waits.waits++;
	s_waits.cycles += dc;
	if (dc > s_waits.max_cycles)
		s_waits.max_cycles = dc;
	if (rc == -ETIMEDOUT)
		s_waits.timeouts++;
	return rc;
}

void osal_gc_gpu_stats(uint64_t *submit_us, uint64_t *gpu_busy_us, uint32_t *kicks)
{
	struct gc_render_perf p;

	gc_render_perf(&p);
	if (submit_us)
		*submit_us = k_cyc_to_us_floor64(s_submit_cycles);
	if (gpu_busy_us)
		*gpu_busy_us = p.gpu_busy_ns / 1000u;
	if (kicks)
		*kicks = p.kicks_done;
}

void osal_gc_wait_stats(uint32_t *waits, uint32_t *timeouts, uint64_t *wait_us, uint64_t *max_us)
{
	if (waits)
		*waits = s_waits.waits;
	if (timeouts)
		*timeouts = s_waits.timeouts;
	if (wait_us)
		*wait_us = k_cyc_to_us_floor64(s_waits.cycles);
	if (max_us)
		*max_us = k_cyc_to_us_floor64(s_waits.max_cycles);
}

/* The full-queue wait of pending_frees.h: the backend waits with no deadline
 * (timeout -1), which z_wait bounds by GC_WAIT_FOREVER_MS; so does this. */
static int wait_pending(uint32_t fence, uint32_t *completed)
{
	int rc = timed_wait(fence, GC_WAIT_FOREVER_MS);

	if (rc == -ETIMEDOUT)
		return 1;
	if (rc)
		return rc;
	*completed = MAX(gc_render_completed(), fence);
	return 0;
}

static int z_wait(int64_t fence, int64_t timeout_us)
{
	uint32_t ms;
	int rc;

	if (!s.inited || fence <= 0 || fence > UINT32_MAX)
		return -EINVAL;
	if (timeout_us < 0)
		ms = GC_WAIT_FOREVER_MS;
	else
		ms = (uint32_t)MIN((timeout_us + 999) / 1000, (int64_t)UINT32_MAX);

	rc = timed_wait((uint32_t)fence, ms);
	if (rc == -ETIMEDOUT)
		return 1;   /* the contract's "timed out"; the stall is already dumped */
	return rc;
}

/* ---- queries -------------------------------------------------------- */
static int z_query(uint32_t what, void *out, size_t out_sz)
{
	if (what == OSAL_QUERY_GC_FRAME_OPEN && out && out_sz == sizeof(uint32_t)) {
		s_frees.frame_open = *(const uint32_t *)out != 0;
		return 0;
	}
	if (what == OSAL_QUERY_CLOCK_NS)
		return osal_zephyr_clock_ns(out, out_sz);
	/* OSAL_QUERY_RENDER_STATIC is a PowerVR firmware object. This GPU has
	 * no firmware. */
	LOG_DBG("osal(gc): query %u is not available on this GPU", what);
	return -ENOTSUP;
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
	.mmu_map     = NULL,
	.map_phys    = z_map_phys,
	.gpu_write   = z_gpu_write,
	.gpu_read    = z_gpu_read,
	.cache_flush = z_cache_flush,
	.cache_invd  = z_cache_invd,
	.submit      = z_submit,
	.wait        = z_wait,
	.completed   = z_completed,
	.reg_read    = NULL,
	.reg_write   = NULL,
	.query       = z_query,
	.log         = z_log,
	.thread_id   = z_thread_id,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
