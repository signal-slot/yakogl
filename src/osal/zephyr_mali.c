/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_mali.c - Zephyr OSAL backend for YakoGL on the Arm Mali-G610
 * (NanoPC-T6, RK3588). The sibling of zephyr.c (PowerVR), zephyr_gc.c
 * (GC7000) and zephyr_v3d.c (V3D); the build compiles exactly one of them,
 * and all define `osal_zephyr_ops`, so nothing above the OSAL changes with
 * the GPU.
 *
 * The OSAL's logic is Rust (driver/rust, board::nanopc_t6_osal, through the
 * yd_t6_osal_* entries of yakogl_drivers.h); this file holds what is the
 * image's: the memory the GPU uses (statics, uncached), the register
 * mappings, the delay that feeds the board's watchdog, the log and the
 * table. Every op forwards to one call:
 *
 *   init        -> yd_t6_osal_up: Z1 to Z3 (power, AS0, the CSF firmware
 *                  booted from the init argument's image), then CSG 0 / CS 0
 *                  started over a group VM of four regions (the ring, the
 *                  heap's pool and chunks, the carve-out); Z4's record is
 *                  printed when a step stops
 *   gpu_alloc   -> yd_t6_osal_alloc: whole pages of the carve-out, first fit
 *   gpu_free    -> yd_t6_osal_free: held until the kick being recorded
 *                  completes
 *   map_phys    -> yd_t6_osal_map_phys: a range inside the carve-out, where
 *                  it is; -ENOTSUP for any other
 *   submit      -> yd_t6_osal_submit: the struct osal_mali_kick's stream as
 *                  one job on the ring (src/osal/mali.rs)
 *   wait        -> yd_t6_osal_wait: the sync object; panthor's 5 s job
 *                  timeout for a negative or longer timeout
 *   completed   -> yd_t6_osal_completed
 *                  (each of the three handles a tiler OOM and, on a GPU
 *                  error, restarts the group: one line per restart,
 *                  note_restart below)
 *   shutdown    -> yd_t6_osal_shutdown: the last kick waited for, CS 0
 *                  stopped and CSG 0 terminated; then the counts
 *   query       -> OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP; the
 *                  clock and draw stats of zephyr_timing.c
 *   cache_*     -> nothing: every CPU view is uncached, and each job's
 *                  FLUSH_CACHE2 cleans the GPU's caches
 *
 * Absent: mmu_map, gpu_read/write, reg_*, wait_vblank, present (the board
 * has no display on Zephyr yet). A tiler out-of-memory event grows the
 * heap, up to CONFIG_YAKOGL_MALI_HEAP_CHUNKS; a GPU fault, fatal or the
 * job timeout loses the kicks in flight and restarts the group; what needs
 * a GPU reset stops the OSAL (board::nanopc_t6_osal, "Recovery").
 *
 * Memory: __nocache statics (CONFIG_NOCACHE_MEMORY), physical address =
 * k_mem_phys_addr() of the CPU address, as the t6_* probes place theirs.
 */

#include <stdarg.h>
#include <stdbool.h>
#include <string.h>
#include <errno.h>

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/linker/section_tags.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_log.h"
#include <nanopc_t6.h>
#include <yakogl_drivers.h>

int yakogl_osal_zephyr_present(const struct osal_gpu_mem *mem, uint32_t width, uint32_t height,
			       uint32_t stride_bytes, uint32_t format); /* zephyr_timing.c */

#if !defined(CONFIG_NOCACHE_MEMORY)
#error "the Mali OSAL's memory is __nocache: CONFIG_NOCACHE_MEMORY=y"
#endif

/* The RK3588's blocks Z1 to Z3 touch (soc::rk3588, board::nanopc_t6_gpu:
 * the CRU, the PMU, SPI2 to the RK806 PMIC, the GPU's QoS, the GPU), and
 * the GPU mapping's size: the doorbells are at 0x80000 and up. */
#define T6_CRU_PHYS  0xfd7c0000UL
#define T6_PMU_PHYS  0xfd8d8000UL
#define T6_SPI2_PHYS 0xfeb20000UL
#define T6_QOS_PHYS  0xfdf35000UL
#define T6_GPU_PHYS  0xfb000000UL
#define T6_GPU_MAP   0x280000
#define T6_TIMER_HZ  24000000 /* CNTFRQ_EL0 */

#define PAGE 4096u

/* The firmware's sections, placed one after the other from page boundaries
 * (the board's mali_csffw.bin: seven, 896 KiB); a firmware that needs more
 * is refused at init (-EINVAL). */
#define MALI_FW_ARENA (1024u * 1024u)
/* AS0's tables: the firmware's nine pages and one for the queue's memory
 * (t6_submit's pool of sixteen). */
#define MALI_POOL_PAGES 16
/* The queue's AS0 memory: 8 KiB of user pages, then CSG 0's suspend
 * buffers; 1 MiB is far over what the firmware asks (t6_submit). */
#define MALI_QUEUE_MEM (1024u * 1024u)
#define MALI_CARVE CONFIG_YAKOGL_MALI_CARVEOUT_BYTES
#define MALI_CHUNKS CONFIG_YAKOGL_MALI_HEAP_CHUNKS
#define MALI_CHUNKS_INITIAL CONFIG_YAKOGL_MALI_HEAP_INITIAL_CHUNKS
/* The group VM's tables, 4 KiB pages only: one level-3 table per 2 MiB of
 * the carve-out and the chunks, and the few above them for the four
 * regions (17 for a 16 MiB carve-out and one chunk, the host test's). */
#define MALI_VM_POOL_PAGES (MALI_CARVE / (2u * 1024u * 1024u) + MALI_CHUNKS + 16)

BUILD_ASSERT(MALI_CARVE % PAGE == 0, "the carve-out is whole pages");
BUILD_ASSERT(MALI_CHUNKS_INITIAL >= 1 && MALI_CHUNKS_INITIAL <= MALI_CHUNKS,
	     "the heap starts with 1 to YAKOGL_MALI_HEAP_CHUNKS chunks");
BUILD_ASSERT(MALI_CARVE <= YD_T6_OSAL_CARVE_MAX, "the carve-out is at most YD_T6_OSAL_CARVE_MAX");

static __nocache uint8_t mali_fw[MALI_FW_ARENA] __aligned(PAGE);
static __nocache uint8_t mali_pool[MALI_POOL_PAGES * PAGE] __aligned(PAGE);
static __nocache uint8_t mali_queue[MALI_QUEUE_MEM] __aligned(PAGE);
static __nocache uint8_t mali_ring[YD_T6_OSAL_QUEUE_SIZE] __aligned(PAGE);
static __nocache uint8_t mali_heap_pool[YD_T6_OSAL_HEAP_POOL_SIZE] __aligned(PAGE);
static __nocache uint8_t mali_chunks[MALI_CHUNKS * (size_t)YD_T6_OSAL_HEAP_CHUNK] __aligned(PAGE);
static __nocache uint8_t mali_carve[MALI_CARVE] __aligned(PAGE);
static __nocache uint8_t mali_vm_pool[MALI_VM_POOL_PAGES * PAGE] __aligned(PAGE);

static struct {
	bool inited;
	uint32_t restarts; /* the last count note_restart printed */
	bool stopped;      /* note_restart printed the stop */
} s;

static struct yd_t6_fw_backing backing(uint8_t *cpu, size_t size)
{
	return (struct yd_t6_fw_backing){
		.pa = k_mem_phys_addr(cpu), .cpu = (uintptr_t)cpu, .size = size,
	};
}

/* The Rust waits poll through this (Z3's bring-up, the 5 s job timeout):
 * the board glue wants t6_watchdog_feed() at most
 * CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart, so every delay feeds it, in slices
 * of at most 100 ms, as t6_submit's does. */
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

/* ---- lifecycle ---------------------------------------------------------- */

__weak const struct osal_zephyr_mali_init *osal_zephyr_mali_default_platform(void)
{
	return NULL;
}

static int z_init(void *platform_arg)
{
	const struct osal_zephyr_mali_init *arg = platform_arg;
	static struct yd_t6_submit_up o;
	static struct yd_t6_gpu_up z1; /* 500 bytes: not on the caller's stack */
	struct yd_t6_osal_mem mem = { 0 };
	mm_reg_t cru, pmu, spi2, qos, gpu;
	int rc;

	if (s.inited)
		return 0;
	if (!arg)
		arg = osal_zephyr_mali_default_platform();
	if (!arg || !arg->fw || arg->fw_len == 0) {
		printk("osal(mali): init needs struct osal_zephyr_mali_init with the CSF firmware\n");
		return -EINVAL;
	}
	device_map(&cru, T6_CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, T6_PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, T6_SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, T6_QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&gpu, T6_GPU_PHYS, T6_GPU_MAP, K_MEM_CACHE_NONE);

	mem.fw = backing(mali_fw, sizeof(mali_fw));
	mem.pool = mali_pool;
	mem.pool_pages = MALI_POOL_PAGES;
	mem.tables_pa = k_mem_phys_addr(mali_pool);
	mem.queue = backing(mali_queue, sizeof(mali_queue));
	mem.ring = backing(mali_ring, sizeof(mali_ring));
	mem.heap_pool = backing(mali_heap_pool, sizeof(mali_heap_pool));
	mem.heap_chunks = backing(mali_chunks, sizeof(mali_chunks));
	mem.carve = backing(mali_carve, sizeof(mali_carve));
	mem.vm_pool = mali_vm_pool;
	mem.vm_pool_pages = MALI_VM_POOL_PAGES;
	mem.vm_tables_pa = k_mem_phys_addr(mali_vm_pool);
	mem.heap_initial_chunks = MALI_CHUNKS_INITIAL;

	t6_watchdog_feed();
	rc = yd_t6_osal_up(cru, pmu, spi2, qos, gpu, arg->fw, arg->fw_len, T6_TIMER_HZ, &mem, clean,
			   delay_us, on_handover, &z1, &o);
	t6_watchdog_feed();
	if (rc) {
		printk("osal(mali): up: rc %d step %u refusal %u detail 0x%llx; Z3 step %u stop %u "
		       "refusal %u\n",
		       rc, o.step, o.refusal, (unsigned long long)o.detail, o.fw.step, o.fw.stop,
		       o.fw.refusal);
		if (o.fw.step == YD_T6_FW_STEP_GPU) {
			printk("osal(mali): Z1 stopped: stop %u, refusal %u, detail 0x%08x%08x\n",
			       z1.stop, z1.refusal, z1.detail[1], z1.detail[0]);
		}
		return rc;
	}
	s.inited = true;
	printk("osal(mali): ready: gpu_id 0x%08x, heap ctx 0x%llx, %u of %u chunk(s) of 0x%x, "
	       "carve-out %u KiB at 0x%llx, csg ack %u us\n",
	       o.fw.gpu_id, (unsigned long long)o.heap_ctx, (unsigned int)MALI_CHUNKS_INITIAL,
	       (unsigned int)MALI_CHUNKS, o.heap_chunk_size, (unsigned int)(MALI_CARVE / 1024),
	       (unsigned long long)YD_T6_OSAL_CARVE_VA, o.csg_ack_us);
	return 0;
}

void osal_mali_diag_print(void)
{
	struct yd_t6_osal_counts c;

	if (yd_t6_osal_counts(&c) != 0)
		return;
	printk("osal(mali): kicks %llu, completed %llu; pages %u (high-water %u) of %u; "
	       "frees held %u, refused %u; stopped by sched refusal %u (0x%x)\n",
	       (unsigned long long)c.submitted, (unsigned long long)c.completed, c.live_pages,
	       c.live_pages_high, (unsigned int)(MALI_CARVE / PAGE), c.held, c.refused_frees,
	       c.dead_refusal, c.dead_value);
	printk("osal(mali): restarts %u (last by sched refusal %u, 0x%x); tiler OOMs %u grown, "
	       "%u with no chunk; heap %u of %u chunk(s); group %s\n",
	       c.restarts, c.last_refusal, c.last_value, c.oom_chunks, c.oom_no_chunk,
	       c.heap_chunks, (unsigned int)MALI_CHUNKS,
	       c.unended ? "may still run" : c.dead_refusal != 0 || c.down ? "ended" : "running");
}

/* One line per group restart a call just caused, with the sched refusal
 * that caused it (driver/rust/src/gpu/mali/sched.rs, SchedRefusal) and the
 * kicks it lost up to; one line when the OSAL stops, saying whether the
 * group ended or may still run (then it needs a GPU reset).
 * The OSAL's own numbers: 28 TilerOomCounts, 29 HeapGrow, 30 HeapRemake,
 * 31 RestartBudget (the value: the refusal that would have restarted the
 * group), 32 CacheFlush (the GPU cache clean after the group's end did not
 * complete); 27 is the Z4 record's DrainTimeout, which the OSAL never
 * meets. */
static void note_restart(void)
{
	struct yd_t6_osal_counts c;

	if (yd_t6_osal_counts(&c) != 0)
		return;
	if (c.restarts != s.restarts) {
		s.restarts = c.restarts;
		printk("osal(mali): restart %u after sched refusal %u (0x%x): kicks up to %llu lost\n",
		       c.restarts, c.last_refusal, c.last_value, (unsigned long long)c.submitted);
	}
	if (c.dead_refusal != 0 && !s.stopped) {
		s.stopped = true;
		printk("osal(mali): stopped by sched refusal %u (0x%x) after kick %llu: %s; every "
		       "later kick is -EIO\n",
		       c.dead_refusal, c.dead_value, (unsigned long long)c.submitted,
		       c.unended ? "the group may still run, memory kept (needs a GPU reset)" :
				   "the group ended");
	}
}

static void z_shutdown(void)
{
	/* The group ends; the memory stays the image's (the GL core owns its
	 * buffers until the image ends). */
	int rc = yd_t6_osal_shutdown();

	note_restart();
	printk("osal(mali): shutdown: %s (%d)\n",
	       rc == 0 ? "group stopped and terminated, GPU caches cleaned" :
	       rc == -ETIMEDOUT ? "group stopped and terminated, GPU cache clean refused: memory kept" :
	       rc == -EINVAL ? "no group to end" :
	       "stop or terminate refused: the group may still run, memory kept", rc);
	osal_mali_diag_print();
}

void osal_mem_line(char *buf, size_t n)
{
	struct yd_t6_osal_counts c;

	if (yd_t6_osal_counts(&c) != 0) {
		snprintk(buf, n, "osal(mali) mem: absent");
		return;
	}
	snprintk(buf, n, "osal(mali) mem: %u KiB in use of %u (high-water %u), %u frees held",
		 c.live_pages * 4u, (unsigned int)(MALI_CARVE / 1024), c.live_pages_high * 4u, c.held);
}

/* ---- GPU memory --------------------------------------------------------- */

static int z_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	/* one carve-out, mapped executable: neither the heap nor the flags
	 * change where a buffer goes (board::nanopc_t6_osal) */
	ARG_UNUSED(heap_base);
	ARG_UNUSED(flags);
	return yd_t6_osal_alloc(size, out);
}

static int z_gpu_free(struct osal_gpu_mem *mem)
{
	return yd_t6_osal_free(mem);
}

static int z_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
		      struct osal_gpu_mem *out)
{
	ARG_UNUSED(heap_base);
	ARG_UNUSED(flags);
	return yd_t6_osal_map_phys(phys, size, out);
}

static void z_cache_nop(void *cpu, size_t n)
{
	ARG_UNUSED(cpu);
	ARG_UNUSED(n);
}

/* ---- submit + completion ------------------------------------------------ */

static int64_t z_submit(const void *kick_desc, size_t kick_desc_len)
{
	const struct osal_mali_kick *k = kick_desc;

	int64_t fence;

	if (!k || kick_desc_len != sizeof(*k))
		return -EINVAL;
	fence = yd_t6_osal_submit(k->stream_gpu, k->stream_bytes);
	note_restart();
	return fence;
}

static int z_wait(int64_t fence, int64_t timeout_us)
{
	int rc = yd_t6_osal_wait(fence, timeout_us);

	note_restart();
	return rc;
}

static int64_t z_completed(void)
{
	int64_t done = yd_t6_osal_completed();

	note_restart();
	return done;
}

static int z_query(uint32_t what, void *out, size_t out_sz)
{
	if (!out)
		return -EINVAL;
	switch (what) {
	case OSAL_QUERY_MALI_GPU:
		if (out_sz != sizeof(struct osal_mali_gpu))
			return -EINVAL;
		return yd_t6_osal_gpu(out);
	case OSAL_QUERY_MALI_TILER_HEAP:
		if (out_sz != sizeof(struct osal_mali_heap))
			return -EINVAL;
		return yd_t6_osal_heap(out);
	case OSAL_QUERY_CLOCK_NS:
	case OSAL_QUERY_DRAW_STATS:
		return osal_zephyr_timing_query(what, out, out_sz);
	default:
		return -ENOTSUP;
	}
}

/* ---- diagnostics, threads ------------------------------------------------ */

static void z_log(const char *fmt, ...)
{
	struct osal_log_line line;   /* OSAL_LOG_LINE_MAX: a core line is never cut */
	const char *text;
	va_list ap;

	va_start(ap, fmt);
	text = osal_log_format(&line, osal_vsnprintk, fmt, ap);
	va_end(ap);
	printk("%s\n", text);
}

/* thread_id (gles_osal.h, "threads"): the running thread's k_thread address
 * folded to 32 bits, as zephyr_v3d.c's. */
static uint32_t z_thread_id(void)
{
	uintptr_t t = (uintptr_t)k_current_get();

	return (uint32_t)(t ^ (t >> 16 >> 16));
}

/* ------------------------------------------------------------------------ *
 * ops table + registrar.
 * ------------------------------------------------------------------------ */
const struct osal_ops osal_zephyr_ops = {
	.init        = z_init,
	.shutdown    = z_shutdown,
	.gpu_alloc   = z_gpu_alloc,
	.gpu_free    = z_gpu_free,
	.mmu_map     = NULL,
	.map_phys    = z_map_phys,
	.gpu_write   = NULL,
	.gpu_read    = NULL,
	.cache_flush = z_cache_nop,
	.cache_invd  = z_cache_nop,
	.submit      = z_submit,
	.wait        = z_wait,
	.completed   = z_completed,
	.reg_read    = NULL,
	.reg_write   = NULL,
	.query       = z_query,
	.log         = z_log,
	.thread_id   = z_thread_id,
	.present     = yakogl_osal_zephyr_present,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
