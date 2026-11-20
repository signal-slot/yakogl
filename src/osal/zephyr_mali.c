/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_mali.c - Zephyr OSAL backend for YakoGL on the Arm Mali (the
 * NanoPC-T6's Mali-G610, RK3588; the Verdin iMX95's Mali-G310, i.MX 95, built
 * and host-tested, not run on a board yet: zephyr_mali_board.h). The sibling of zephyr.c
 * (PowerVR), zephyr_gc.c (GC7000) and zephyr_v3d.c (V3D); the build compiles
 * exactly one of them, and all define `osal_zephyr_ops`, so nothing above
 * the OSAL changes with the GPU.
 *
 * The OSAL's logic is Rust (driver/rust, board::nanopc_t6_osal, through the
 * yd_t6_osal_* entries of yakogl_drivers.h); this file holds what is the
 * image's: the memory the GPU uses (one static, made uncached), the delay
 * that feeds the board's watchdog, the log and the table. What depends on
 * the SoC (its register mappings, the GPU's power-on, the call that brings
 * the group up, the watchdog) is the board's (zephyr_mali_board.h). Every op
 * forwards to one call:
 *
 *   init        -> osal_zephyr_mali_board_up; on the NanoPC-T6 that is
 *                  yd_t6_osal_up (on the Verdin iMX95 yd_imx95_osal_up,
 *                  whose Z1 is the SCMI power-on): Z1 to Z3 (power, AS0, the CSF firmware
 *                  booted from the init argument's image), then CSG 0 / CS 0
 *                  started over a group VM of four regions (the ring, the
 *                  heap's pool and chunks, the carve-out); Z4's record is
 *                  printed when a step stops
 *   gpu_alloc   -> yd_t6_osal_alloc: whole pages of the carve-out, first fit;
 *                  a refused one takes back what the submitted kicks hold
 *   gpu_free    -> yd_t6_osal_free: held until the last kick that may read
 *                  it completes: the one being recorded while the backend
 *                  records a frame, else the last submitted
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
 *   query       -> OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP,
 *                  OSAL_QUERY_MALI_FRAME_OPEN (yd_t6_osal_frame_open); the
 *                  clock and draw stats of zephyr_timing.c
 *   cache_*     -> nothing: the CPU's mapping of the GPU's memory is
 *                  uncached, and each job's FLUSH_CACHE2 cleans the GPU's
 *                  caches
 *   present, display_busy
 *               -> the board layer's screen (osal_internal.h,
 *                  yakogl_zephyr_present; osal_mali_board_window below is
 *                  its window)
 *
 * Absent: mmu_map, gpu_read/write, reg_*, wait_vblank (the board's show
 * never waits). A tiler out-of-memory event grows the heap, up to
 * CONFIG_YAKOGL_MALI_HEAP_CHUNKS; a GPU fault, fatal or the job timeout
 * loses the kicks in flight and restarts the group; what needs a GPU reset
 * stops the OSAL (board::nanopc_t6_osal, "Recovery").
 *
 * Memory: one .noinit block of the image, whose mapping init makes uncached
 * ("The GPU's memory" below). The carve-out's last
 * CONFIG_YAKOGL_MALI_WINDOW_BYTES are the board's window memory
 * (osal_zephyr_mali_window_mem), never allocated.
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
#include "zephyr_mali_board.h"
#include <yakogl_drivers.h>

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
#define MALI_WINDOW CONFIG_YAKOGL_MALI_WINDOW_BYTES
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
BUILD_ASSERT(MALI_WINDOW % PAGE == 0 && MALI_WINDOW < MALI_CARVE,
	     "the board's window memory is whole pages at the carve-out's end, not all of it");

/* The GPU's memory: one block of the image's .noinit, which lies after
 * .bss and so is not in zephyr.bin. (A __nocache static is in the file:
 * Zephyr's nocache section lies before the loaded data, and the file holds
 * it as zeros. The carve-out then counts against the 64 MiB that
 * kexec_zephyr reads.) The CPU must not cache what the GPU reads and writes,
 * so init zeroes the block, takes it out of the CPU's caches and changes its
 * mapping to Normal non-cacheable in place (mem_uncache): what Zephyr's own
 * tables do for the nocache section. The block keeps its one mapping; the
 * same RAM is never mapped a second time with other attributes (the Arm
 * ARM's "mismatched memory attributes").
 *
 * The block is aligned and sized to 2 MiB: the change then replaces whole
 * level-2 block entries that map nothing else, so no entry that maps other
 * data of the image is split, and no translation table is taken
 * (CONFIG_MAX_XLAT_TABLES). Each area starts on a page, in this order. */
#define MALI_BLOCK	(2u * 1024u * 1024u)
#define OFF_FW		((size_t)0)
#define OFF_POOL	(OFF_FW + MALI_FW_ARENA)
#define OFF_QUEUE	(OFF_POOL + MALI_POOL_PAGES * PAGE)
#define OFF_RING	(OFF_QUEUE + MALI_QUEUE_MEM)
#define OFF_HEAP_POOL	(OFF_RING + YD_T6_OSAL_QUEUE_SIZE)
#define OFF_CHUNKS	(OFF_HEAP_POOL + YD_T6_OSAL_HEAP_POOL_SIZE)
#define OFF_CARVE	(OFF_CHUNKS + MALI_CHUNKS * (size_t)YD_T6_OSAL_HEAP_CHUNK)
#define OFF_VM_POOL	(OFF_CARVE + (size_t)MALI_CARVE)
#define MALI_MEM_USED	(OFF_VM_POOL + MALI_VM_POOL_PAGES * PAGE)
#define MALI_MEM_BYTES	ROUND_UP(MALI_MEM_USED, MALI_BLOCK)

BUILD_ASSERT(MALI_FW_ARENA % PAGE == 0 && MALI_QUEUE_MEM % PAGE == 0 &&
		     YD_T6_OSAL_QUEUE_SIZE % PAGE == 0 && YD_T6_OSAL_HEAP_POOL_SIZE % PAGE == 0 &&
		     YD_T6_OSAL_HEAP_CHUNK % PAGE == 0,
	     "every area of the GPU's memory is whole pages");

static uint8_t mali_mem[MALI_MEM_BYTES] __noinit __aligned(MALI_BLOCK);

static struct {
	bool inited;
	bool uncached;     /* mem_uncache changed mali_mem's mapping */
	uint32_t window;   /* the carve-out's last bytes, osal_mali_board_window's
			    * or MALI_WINDOW (osal_zephyr_mali_window_mem) */
	uint32_t restarts; /* the last count note_restart printed */
	bool stopped;      /* note_restart printed the stop */
} s;

/* Zero mali_mem and make its mapping uncached, once. The zeros are written
 * through the cached mapping and cleaned to memory, and the lines
 * invalidated, before the mapping changes: no cached line of the block is
 * left to be written back over what the GPU writes later. -ENOTSUP: the
 * image has no cache maintenance (CONFIG_CACHE_MANAGEMENT), so the block
 * cannot be taken out of the caches. k_mem_update_flags() unmaps the range
 * and maps it again with the new flags, under the kernel's memory lock;
 * the block has no user yet, so nothing reads it in between. */
static int mem_uncache(void)
{
	int rc;

	if (s.uncached)
		return 0;
	memset(mali_mem, 0, sizeof(mali_mem));
	rc = sys_cache_data_flush_and_invd_range(mali_mem, sizeof(mali_mem));
	if (rc)
		return rc;
	rc = k_mem_update_flags(mali_mem, sizeof(mali_mem), K_MEM_PERM_RW | K_MEM_ARM_NORMAL_NC);
	if (rc)
		return rc;
	s.uncached = true;
	return 0;
}

static struct yd_t6_fw_backing backing(size_t off, size_t size)
{
	return (struct yd_t6_fw_backing){
		.pa = k_mem_phys_addr(mali_mem + off),
		.cpu = (uintptr_t)(mali_mem + off),
		.size = size,
	};
}

/* The board's window (osal_internal.h): the carve-out's last pages, which
 * init then keeps from every gpu_alloc (struct yd_t6_osal_mem,
 * carve_reserved), so the GPU reaches the window through map_phys as it
 * does on the other boards. The block is made uncached here when init has
 * not run yet: it is zeroed only once, so what the caller writes into the
 * window before init stays. The size is kept for a later init (shutdown
 * leaves the memory the image's). */
int osal_mali_board_window(size_t bytes, uint64_t *phys, void **cpu)
{
	size_t reserved = ROUND_UP(bytes, PAGE);
	uint8_t *at;
	int rc;

	if (!phys || !cpu || bytes == 0 || reserved < bytes || reserved >= MALI_CARVE)
		return -EINVAL;
	if (s.inited)
		return -EBUSY;
	/* one window: a board with CONFIG_YAKOGL_MALI_WINDOW_BYTES has its own
	 * (osal_zephyr_mali_window_mem) at the same place */
	if (s.window != 0 || MALI_WINDOW != 0)
		return -ENOMEM;
	rc = mem_uncache();
	if (rc)
		return rc;
	s.window = (uint32_t)reserved;
	at = mali_mem + OFF_CARVE + MALI_CARVE - reserved;
	*phys = k_mem_phys_addr(at);
	*cpu = at;
	return 0;
}

int osal_zephyr_mali_window_mem(uintptr_t *phys, void **cpu, size_t *bytes)
{
	uint8_t *at = mali_mem + OFF_CARVE + MALI_CARVE - MALI_WINDOW;
	int rc;

	*phys = 0;
	*cpu = NULL;
	*bytes = 0;
	if (MALI_WINDOW == 0)
		return -ENOTSUP;
	/* zeroed and uncached once; z_init's call is then a no-op, so what
	 * the board wrote into its windows before eglInitialize stays */
	rc = mem_uncache();
	if (rc)
		return rc;
	s.window = MALI_WINDOW;
	*phys = k_mem_phys_addr(at);
	*cpu = at;
	*bytes = MALI_WINDOW;
	return 0;
}

/* The Rust waits poll through this (Z3's bring-up, the 5 s job timeout):
 * the board glue wants its feed (on the NanoPC-T6 t6_watchdog_feed(), at
 * most CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart), so every delay feeds it, in
 * slices of at most 100 ms, as t6_submit's does. */
static void delay_us(uint32_t us)
{
	do {
		uint32_t slice = us > 100000U ? 100000U : us;

		osal_zephyr_mali_board_feed();
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
	osal_zephyr_mali_board_feed();
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
	static struct yd_t6_gpu_up z1; /* hundreds of bytes: not on the caller's stack */
	struct yd_t6_osal_mem mem = { 0 };
	int rc;

	if (s.inited)
		return 0;
	if (!arg)
		arg = osal_zephyr_mali_default_platform();
	if (!arg || !arg->fw || arg->fw_len == 0) {
		printk("osal(mali): init needs struct osal_zephyr_mali_init with the CSF firmware\n");
		return -EINVAL;
	}
	osal_zephyr_mali_board_feed();
	rc = mem_uncache();
	if (rc) {
		printk("osal(mali): the GPU's memory was not made uncached: rc %d\n", rc);
		return rc;
	}
	mem.fw = backing(OFF_FW, MALI_FW_ARENA);
	mem.pool = mali_mem + OFF_POOL;
	mem.pool_pages = MALI_POOL_PAGES;
	mem.tables_pa = k_mem_phys_addr(mali_mem + OFF_POOL);
	mem.queue = backing(OFF_QUEUE, MALI_QUEUE_MEM);
	mem.ring = backing(OFF_RING, YD_T6_OSAL_QUEUE_SIZE);
	mem.heap_pool = backing(OFF_HEAP_POOL, YD_T6_OSAL_HEAP_POOL_SIZE);
	mem.heap_chunks = backing(OFF_CHUNKS, MALI_CHUNKS * (size_t)YD_T6_OSAL_HEAP_CHUNK);
	mem.carve = backing(OFF_CARVE, MALI_CARVE);
	mem.vm_pool = mali_mem + OFF_VM_POOL;
	mem.vm_pool_pages = MALI_VM_POOL_PAGES;
	mem.vm_tables_pa = k_mem_phys_addr(mali_mem + OFF_VM_POOL);
	mem.heap_initial_chunks = MALI_CHUNKS_INITIAL;
	/* the board's window (osal_mali_board_window), or else the
	 * configured window memory (osal_zephyr_mali_window_mem) */
	if (s.window == 0)
		s.window = MALI_WINDOW;
	mem.carve_reserved = s.window;

	osal_zephyr_mali_board_feed();
	rc = osal_zephyr_mali_board_up(arg, &mem, clean, delay_us, on_handover, &z1, &o);
	osal_zephyr_mali_board_feed();
	if (rc == -ENOTSUP) {
		/* no bring-up on this board yet: its line said what is missing */
		return rc;
	}
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
	       "carve-out %u KiB at 0x%llx (%u KiB of it the board window), csg ack %u us\n",
	       o.fw.gpu_id, (unsigned long long)o.heap_ctx, (unsigned int)MALI_CHUNKS_INITIAL,
	       (unsigned int)MALI_CHUNKS, o.heap_chunk_size, (unsigned int)(MALI_CARVE / 1024),
	       (unsigned long long)YD_T6_OSAL_CARVE_VA, s.window / 1024u, o.csg_ack_us);
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
	       "%u with no chunk; heap %u of %u chunk(s); group %s, memory %s; CS stops refused %u\n",
	       c.restarts, c.last_refusal, c.last_value, c.oom_chunks, c.oom_no_chunk,
	       c.heap_chunks, (unsigned int)MALI_CHUNKS,
	       c.unended ? "may still run" : (c.dead_refusal != 0 || c.down) ? "ended" : "running",
	       c.memory_kept ? "kept" : "not kept", c.stops_refused);
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
		       c.unended     ? "the group may still run, memory kept (needs a GPU reset)" :
		       c.memory_kept ? "the group ended, memory kept" :
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
	       rc == 0 ? "group terminated, GPU caches cleaned" :
	       rc == -ETIMEDOUT ? "group terminated, GPU cache clean refused: memory kept" :
	       rc == -EINVAL ? "no group to end" :
	       "terminate refused: the group may still run, memory kept", rc);
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
	case OSAL_QUERY_MALI_FRAME_OPEN:
		if (out_sz != sizeof(uint32_t))
			return -EINVAL;
		yd_t6_osal_frame_open(*(const uint32_t *)out != 0);
		return 0;
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

/* Which window buffers the display may still scan (gles_osal.h,
 * display_busy): the board layer's, as in zephyr.c
 * (driver/zephyr/lib/board/common/board.c, CONFIG_YAKOGL_BOARD_DISPLAY_BUSY).
 * Declared weak and defined nowhere here, so on an image without the board
 * layer the symbol is 0, the op is NULL, and EGL keeps its ring of three
 * buffers. */
extern uint32_t yakogl_zephyr_display_busy(const uint64_t *phys, size_t n) __attribute__((weak));

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
	.present     = yakogl_zephyr_present,
	.display_busy = yakogl_zephyr_display_busy,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
