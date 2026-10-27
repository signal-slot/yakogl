/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_v3d.c - Zephyr OSAL backend for YakoGL on the Broadcom V3D 4.2
 * (Raspberry Pi 4). The sibling of zephyr.c (PowerVR) and zephyr_gc.c
 * (GC7000); the build compiles exactly one of them, and all define
 * `osal_zephyr_ops`, so nothing above the OSAL changes with the GPU.
 *
 * The V3D has no firmware and no ring, and there is no Zephyr driver for it:
 * this file is the whole render service, on the driver's OS-free steps
 * (driver/src/v3d/v3d_hw.c, milestones M1..M3, each run on the board):
 *
 *   init        -> v3d_power_on (the firmware clock switch comes from the
 *                  application, struct osal_zephyr_v3d_init), v3d_identify,
 *                  the carve-out mapped Normal non-cacheable, the MMU page
 *                  table over it, v3d_mmu_init
 *   gpu_alloc   -> a first-fit granule map over the carve-out, read a word
 *                  at a time from a first-free hint; opt-in, small blocks
 *                  by last fit from the top (v3d_granules.h)
 *   gpu_free    -> deferred until the newest job that may read it (the one
 *                  being recorded, else the one in flight) has completed
 *   map_phys    -> a window's pages in the MMU and a Normal non-cacheable
 *                  CPU view of them (glReadPixels reads through it)
 *   submit      -> struct v3d_kick: tile state zeroed, v3d_submit_bin, the
 *                  render started by v3d_isr at FLDONE, and it returns with
 *                  the job running: one job runs on the GPU and one more
 *                  waits in a slot (s.pending) for it. The waiting job
 *                  starts when the running one is finished
 *                  (finish_inflight); a third submit finishes the running
 *                  job first
 *   wait        -> for the job in flight, and for the waiting one when
 *                  that is the fence: FRDONE waited for (up to
 *                  JOB_HANG_US; a timeout of 0 only polls), v3d_caches_clean,
 *                  the fence completed and the held frees drained; for an
 *                  older fence, its result
 *   completed   -> the same finish when FRDONE is already up, never blocking
 *   query       -> OSAL_QUERY_V3D_VPM_BYTES (the identified VPM), and the
 *                  others osal_internal.h names (OSAL_QUERY_CLOCK_NS: the
 *                  64-bit cycle counter in ns, zephyr_timing.c)
 *
 * The board's display (the firmware framebuffer, through the mailbox) and its
 * bounded-lifetime watchdog are board support the application brings, as the
 * GC7000's panel is (driver/zephyr/lib/board/imx8mp): the OSAL takes the one thing
 * of the board's it needs, the V3D clock switch, as its init argument.
 *
 * What M6 does not do, said rather than hidden: the binner gets no overflow
 * pool (an out-of-memory binner is the job's error, -ENOMEM), and a hang is
 * not recovered (the job fails with the registers printed; the board's
 * watchdog resets it). Both are the next steps once a workload needs them.
 *
 * Memory is Normal non-cacheable for the CPU (K_MEM_ARM_NORMAL_NC, not
 * K_MEM_CACHE_NONE, which is Device-nGnRnE on arm64 and faults on the
 * unaligned accesses memcpy makes), so the cache operations have nothing to
 * do for GPU memory. The carve-out itself is a __noinit static of the image,
 * so the linker keeps everything else away from it; its cacheable alias is
 * written back and invalidated once at init and never touched again.
 */

#include <stdarg.h>
#include <stdbool.h>
#include <string.h>
#include <errno.h>

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/irq.h>
#include <zephyr/dt-bindings/interrupt-controller/arm-gic.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_bounds.h"
#include "osal_log.h"
#include "v3d_kick.h"
#include "v3d.h"
#include "v3d_job.h" /* v3d_job_pack_skip, v3d_job_pack_totals */
#include "v3d_regs.h"
#include "v3d_frees.h"
#include "v3d_granules.h"
#include "zephyr_v3d.h"
int yakogl_osal_zephyr_present(const struct osal_gpu_mem *mem, uint32_t width, uint32_t height,
			       uint32_t stride_bytes, uint32_t format); /* zephyr_timing.c */

/* ---- the carve-out ---------------------------------------------------- */

#define V3D_OSAL_CARVEOUT_BYTES CONFIG_YAKOGL_V3D_CARVEOUT_BYTES
#define GRANULE_SHIFT 8u                      /* 256-byte allocation granules */
#define GRANULE       (1u << GRANULE_SHIFT)
#define PAGE          4096u

/* The GPU address space: VA 0 is never mapped (a null address faults), the carve-out
 * starts at 1 MiB, and windows (map_phys) follow it. 512 MiB of VA is a
 * 512 KiB page table, carved from the front of the carve-out. */
#define VA_BASE     0x00100000u
#define VA_SPACE    0x20000000u
#define PT_ENTRIES  (VA_SPACE / PAGE)
#define PT_BYTES    (PT_ENTRIES * 4u)

BUILD_ASSERT(V3D_OSAL_CARVEOUT_BYTES % PAGE == 0, "the carve-out is whole pages");
BUILD_ASSERT(V3D_OSAL_CARVEOUT_BYTES > PT_BYTES + 2 * PAGE, "the carve-out holds the page table");
BUILD_ASSERT(VA_BASE + (uint64_t)V3D_OSAL_CARVEOUT_BYTES < VA_SPACE,
	     "the carve-out fits the GPU VA space");

static uint8_t carve[V3D_OSAL_CARVEOUT_BYTES] __aligned(PAGE) __noinit;

#define GRANULES (V3D_OSAL_CARVEOUT_BYTES / GRANULE)
BUILD_ASSERT(GRANULES % 32 == 0, "the granule map is whole 32-bit words (v3d_granules.h)");
/* one binner overflow block: 256 KiB, the size Mesa's simulator glue hands
 * the binner (v3d_isr_core) */
#define BIN_OVERFLOW_BLOCK (256u * 1024u)
static uint32_t used_map[GRANULES / 32];

static struct {
	bool inited;
	struct v3d_regs regs;
	struct v3d_identity id;
	uint8_t *nc;            /* the carve-out's Normal non-cacheable CPU view */
	uintptr_t phys;         /* its physical address */
	uint32_t *pt;           /* the page table, at the front of the carve-out */
	uint32_t first_granule; /* the allocator's first: past the table and the scratch page */
	uint32_t window_va;     /* the next VA for map_phys */
	uint32_t submitted, completed;
	int last_rc;            /* the last completed job's result, for wait() */
	/* the completed job before it and its result: with two jobs
	 * outstanding, the older one may complete, and the newer one too,
	 * before the backend waits for the older one's fence (z_wait) */
	uint32_t prev_completed;
	int prev_rc;
	/* the job whose render runs (submitted == completed + 1, or + 2 with
	 * a job waiting, while true): its fence, when its render started
	 * (cycles), when its submit began (for last_job_cyc.total) and its
	 * lists (for the failure line) */
	bool inflight;
	uint32_t if_fence, if_c0, if_c_render;
	/* the binner's part of the job in flight: bin_pending from the bin's
	 * submit until v3d_isr saw FLDONE and started the render; bin_rc the
	 * ISR's verdict when the binner ran the overflow pool dry (-ENOMEM);
	 * the tile pool for the pool record the ISR takes */
	volatile bool bin_pending;
	volatile int bin_rc;
	uint32_t if_c_bin, if_c_fldone, if_tile_alloc, if_tile_alloc_size;
	uint32_t if_bcl_start, if_bcl_end, if_rcl_start, if_rcl_end;
	/* the job submitted behind the one in flight (the newest fence): its
	 * kick, kept until the running job is finished and the binner and the
	 * tile memory are free for it (every job shares them) */
	bool pending;
	uint32_t pend_fence, pend_c0;
	struct v3d_kick pend_kick;
	struct osal_v3d_bin_pool bin; /* after the last job that binned */
	/* the binner's overflow pool (CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS
	 * blocks, taken at init; ov.used is per job) */
	uint32_t ov_blocks[MAX(CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS, 1)];
	struct v3d_bin_overflow ov;
	struct v3d_clock_ops clk;
} s;

/* the allocator over the carve-out's granules (v3d_granules.h); a held
 * free stays marked used until given back */
static struct v3d_granules gm;

/* The V3D's interrupt: GIC SPI 74, level high (BCM2711_V3D_GIC_SPI, the
 * BCM2711 device tree's interrupt for the GPU at 0x7ec00000). Zephyr numbers
 * a GIC SPI n as IRQ n + 32 (map_arm_gic_irq_type in Zephyr's
 * scripts/dts/gen_defines.py). The core's interrupt only: the hub's (MMU faults)
 * shares the line on the 2711 (one interrupt in its tree) but stays masked;
 * a fault is seen by the wait's status read, as before. */
#define V3D_IRQ (74 + 32)
#define V3D_BIN_IRQS (V3D_CTL_0_INT_STS_INT_FLDONE_SET | V3D_CTL_0_INT_STS_INT_OUTOMEM_SET)
static void v3d_isr(const void *arg); /* below, beside recover */
/* the ISR's calls and the overflow blocks it gave the binner (the diag print) */
static uint32_t isr_calls, isr_overflow_fed;

static uint32_t va_of(uint32_t off)
{
	return VA_BASE + off;
}

/* ---- lifecycle -------------------------------------------------------- */

/* UIFCFG read at init, repeated in the job record */
static uint32_t diag_uifcfg;

__weak const struct osal_zephyr_v3d_init *osal_zephyr_v3d_default_platform(void)
{
	return NULL;
}

static int z_init(void *platform_arg)
{
	const struct osal_zephyr_v3d_init *arg = platform_arg;
	int rc;

	if (s.inited)
		return 0;
	if (!arg)
		arg = osal_zephyr_v3d_default_platform();
	if (!arg || !arg->clock_set) {
		printk("osal(v3d): init needs struct osal_zephyr_v3d_init with the V3D clock switch\n");
		return -EINVAL;
	}
	s.clk.set = arg->clock_set;
	s.clk.ctx = arg->clock_ctx;
	/* the job builder reads it at each v3d_job_begin */
	v3d_job_pack_skip = IS_ENABLED(CONFIG_YAKOGL_V3D_PACK_SKIP);

	device_map(&s.regs.hub, BCM2711_V3D_HUB_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&s.regs.core, BCM2711_V3D_CORE0_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&s.regs.pm, BCM2711_PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	device_map(&s.regs.asb, BCM2711_V3D_ASB_BASE, 0x1000, K_MEM_CACHE_NONE);

	/* M1 */
	rc = v3d_power_on(&s.regs, &s.clk);
	if (rc) {
		printk("osal(v3d): power-on failed (%d)\n", rc);
		return rc;
	}
	rc = v3d_identify(&s.regs, &s.id);
	printk("osal(v3d): V3D %u.%u, %u QPUs, VPM %u bytes, MMU %s (identify %d)\n",
	       s.id.ver / 10, s.id.ver % 10, s.id.qpu_count, s.id.vpm_size,
	       s.id.has_mmu ? "yes" : "no", rc);
	/* M6 step C: the depth/stencil buffers are laid out as Mesa's zsbuf,
	 * whose layout code assumes the hub's UIF configuration (4 KiB pages, 8
	 * banks); nothing here writes UIFCFG, so the board states its value */
	diag_uifcfg = sys_read32(s.regs.hub + V3D_HUB_CTL_UIFCFG);
	printk("osal(v3d): UIFCFG 0x%08x\n", diag_uifcfg);
	if (rc)
		return rc;

	/* the carve-out: its cacheable alias written back and dropped once,
	 * then only the Normal non-cacheable view is used */
	s.phys = k_mem_phys_addr(carve);
	sys_cache_data_flush_and_invd_range(carve, sizeof(carve));
	k_mem_map_phys_bare(&s.nc, s.phys, sizeof(carve), K_MEM_PERM_RW | K_MEM_ARM_NORMAL_NC);
	if (!s.nc) {
		printk("osal(v3d): cannot map the carve-out\n");
		return -ENOMEM;
	}

	/* M2: the page table and the scratch page at the front, the rest of the
	 * carve-out mapped linearly from VA_BASE */
	s.pt = (uint32_t *)s.nc;
	memset(s.pt, 0, PT_BYTES);
	uint32_t scratch_off = PT_BYTES;
	memset(s.nc + scratch_off, 0, PAGE);
	uint32_t heap_off = scratch_off + PAGE;
	rc = v3d_pte_map(s.pt, PT_ENTRIES, va_of(heap_off), s.phys + heap_off,
			 (uint32_t)sizeof(carve) - heap_off, true);
	if (rc) {
		printk("osal(v3d): the carve-out does not fit the page table (%d)\n", rc);
		return rc;
	}
	s.first_granule = heap_off / GRANULE;
	s.window_va = ROUND_UP(va_of((uint32_t)sizeof(carve)), PAGE);
	v3d_granules_init(&gm, used_map, s.first_granule, GRANULES); /* the map is empty again */
	/* small blocks from the top (v3d_granules.h), opt-in; 0 = first fit */
	v3d_granules_top_below(&gm, CONFIG_YAKOGL_V3D_CARVEOUT_TOP_BELOW / GRANULE);

	v3d_irq_quiesce(&s.regs);
	IRQ_CONNECT(V3D_IRQ, IRQ_DEFAULT_PRIORITY, v3d_isr, NULL, IRQ_TYPE_LEVEL);
	irq_enable(V3D_IRQ);
	v3d_caches_init(&s.regs);
	rc = v3d_mmu_init(&s.regs, s.phys, s.phys + scratch_off);
	if (rc) {
		printk("osal(v3d): MMU init failed (%d)\n", rc);
		return rc;
	}
	/* the binner's overflow blocks: taken once, never freed, so a job that
	 * runs out needs no allocation (and no page-table change) while the
	 * binner waits */
	for (uint32_t i = 0; i < CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS; i++) {
		int64_t g = v3d_granules_find(&gm, BIN_OVERFLOW_BLOCK / GRANULE, PAGE / GRANULE);
		if (g < 0) {
			printk("osal(v3d): the carve-out cannot hold %u binner overflow blocks of %u KiB\n",
			       (unsigned int)CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS, BIN_OVERFLOW_BLOCK / 1024);
			return -ENOMEM;
		}
		v3d_granules_mark(&gm, (uint32_t)g, BIN_OVERFLOW_BLOCK / GRANULE, true);
		s.ov_blocks[i] = va_of((uint32_t)g * GRANULE);
	}
	s.ov = (struct v3d_bin_overflow){
		.blocks = s.ov_blocks,
		.nblocks = CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS,
		.block_size = BIN_OVERFLOW_BLOCK,
	};
	s.inited = true;
	printk("osal(v3d): ready: carve-out %u MiB at phys 0x%lx, GPU VA 0x%x..0x%x\n",
	       (unsigned int)(sizeof(carve) >> 20), (unsigned long)s.phys, va_of(heap_off),
	       va_of((uint32_t)sizeof(carve)));
	return 0;
}

static void report_pending(const char *what);

static void z_shutdown(void)
{
	report_pending(""); /* the pending-free queue's high-water and refusals */
	/* The V3D stays powered; the image's lifetime is bounded anyway. */
	s.inited = false;
}

/* ---- GPU memory + MMU --------------------------------------------------- */

/* Frees wait only for the job the backend is recording (v3d_frees.h). */
static struct v3d_frees frees;

static void release_granules(const struct v3d_frees_run *r)
{
	v3d_granules_mark(&gm, r->granule, r->n, false);
}

/* The queue's counters, every run: in the close record (osal_v3d_diag_print),
 * at shutdown and at the first refusal. */
static void report_pending(const char *what)
{
	char line[160];

	pf_format(&frees.q, line, sizeof(line));
	printk("osal(v3d): %s%s\n", what, line);
}

/* The full-queue wait's bound: the backend's own (src/v3d/backend.rs
 * JOB_TIMEOUT_US). z_wait bounds any non-zero timeout by JOB_HANG_US (4 s
 * from the job's render start), as it does for the backend's waits. */
#define V3D_PENDING_FREES_WAIT_US 1000000

static int z_wait(int64_t fence, int64_t timeout_us);

/* pending_frees.h's wait: z_wait, which finishes the one job in flight */
static int wait_pending(uint32_t fence, uint32_t *completed)
{
	int rc = z_wait(fence, V3D_PENDING_FREES_WAIT_US);

	if (rc)
		return rc;
	*completed = MAX(s.completed, fence);
	return 0;
}

/* the GPU did not finish a job within JOB_HANG_US: from then on nothing is
 * submitted and nothing given back (the job may still read or write it) */
static bool hung;
static uint32_t hung_fence, hung_refused, hung_leaked_kib;

static void drain(void)
{
	if (hung)
		return;
	v3d_frees_drain(&frees, s.completed, release_granules);
}

/* the job in flight, finished (zephyr_v3d.c below z_submit), and every job
 * submitted, finished */
static int finish_inflight(bool poll);
static void finish_all(void);
static void start_pending(void);
static void start_job(const struct v3d_kick *k, uint32_t fence, uint32_t c0);

/* The allocations refused for lack of memory, by size: the line is printed
 * the first time a size is refused and at its 10th, 100th, ... refusal, not
 * at every attempt (DEMO12 printed 423 lines for one D24S8 size the backend
 * retried every job: 87 us a byte of UART inside the timed loop). Sizes past
 * the table are counted together and printed the same way. */
#define OOM_SIZES 16
static struct {
	uint32_t size, refused;
} oom_sizes[OOM_SIZES + 1]; /* the last slot: every other size */
static uint32_t oom_refused_total;

static void oom_line(size_t size)
{
	unsigned i = 0;
	while (i < OOM_SIZES && oom_sizes[i].refused && oom_sizes[i].size != (uint32_t)size)
		i++;
	oom_sizes[i].size = i < OOM_SIZES ? (uint32_t)size : 0;
	const uint32_t n = ++oom_sizes[i].refused;
	oom_refused_total++;
	bool power_of_ten = true;
	for (uint32_t m = n; m > 1 && power_of_ten; m /= 10)
		power_of_ten = m % 10 == 0;
	if (!power_of_ten)
		return;
	if (i < OOM_SIZES)
		printk("osal(v3d): out of GPU memory for %u bytes (%u frees held; this size refused %u time%s)\n",
		       (unsigned int)size, frees.q.n, n, n == 1 ? "" : "s");
	else
		printk("osal(v3d): out of GPU memory for %u bytes (%u frees held; sizes past the first %u "
		       "refused %u times)\n", (unsigned int)size, frees.q.n, OOM_SIZES, n);
}

/* v3d_frees.h's poll: the job in flight finished if FRDONE is up (never
 * blocking), and the newest completed fence. While the GPU is taken as hung
 * nothing is finished or given back. */
static uint32_t poll_completed(void)
{
	if (s.inflight && !hung)
		(void)finish_inflight(true);
	return s.completed;
}

/* Before a free is fenced or an allocation placed: a completion the GPU has
 * reached but the OSAL has not looked at yet, and the frees it releases. */
static uint32_t poll_and_drain(void)
{
	if (hung)
		return s.completed;
	return v3d_frees_poll(&frees, poll_completed, release_granules);
}

static int z_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	(void)heap_base; /* one pool; the heap labels only name the request */
	(void)flags;     /* all of it is Normal non-cacheable */
	if (!s.inited || !out || size == 0 || size > sizeof(carve))
		return -EINVAL;
	uint32_t n = DIV_ROUND_UP(size, GRANULE);
	/* page-aligned for anything a page or larger (the tile memory, the
	 * lists), granule-aligned below that */
	uint32_t align = size >= PAGE ? PAGE / GRANULE : 1;
	/* held frees whose job has completed come back before the place, so the
	 * allocation is not put around blocks that are already free */
	if (frees.q.n)
		(void)poll_and_drain();
	int64_t g = v3d_granules_place(&gm, n, align);
	if (g < 0) {
		/* whatever completed jobs no longer hold, then once more; a job
		 * still in flight is finished first, since it may hold frees */
		if (s.inflight)
			finish_all();
		drain();
		g = v3d_granules_place(&gm, n, align);
	}
	if (g < 0) {
		oom_line(size);
		return -ENOMEM;
	}
	v3d_granules_mark(&gm, (uint32_t)g, n, true);
	uint32_t off = (uint32_t)g * GRANULE;
	out->cpu = s.nc + off;
	out->gpu = va_of(off);
	out->phys = s.phys + off;
	out->size = size;
	out->impl = (void *)(uintptr_t)out->gpu; /* non-NULL marks it live */
	return 0;
}

/* Frees wait for the job being recorded, as in zephyr.c and zephyr_gc.c: the
 * GL core lets go of a buffer while the job it was drawn in is still open. */
static bool in_nc(const void *p)
{
	return p >= (const void *)s.nc && (const uint8_t *)p < s.nc + sizeof(carve);
}

static int z_gpu_free(struct osal_gpu_mem *mem)
{
	int rc = 0;

	if (!mem)
		return 0;
	if (mem->impl && in_nc(mem->cpu) && hung) {
		hung_leaked_kib += (uint32_t)(mem->size / 1024); /* the hung job may use it */
		rc = -EIO; /* refused: counted in hung_leaked_kib, printed by osal_v3d_diag_print */
	} else if (mem->impl && in_nc(mem->cpu)) {
		uint32_t off = (uint32_t)((uint8_t *)mem->cpu - s.nc);
		/* fenced against a completion looked at now, not at the last wait */
		const uint32_t completed = poll_and_drain();
		rc = v3d_frees_free(&frees, off / GRANULE, DIV_ROUND_UP(mem->size, GRANULE),
				    (uint32_t)DIV_ROUND_UP(mem->size, 1024), s.submitted, completed,
				    PF_WAIT(wait_pending), release_granules);
		if (rc && frees.q.refused == 1)
			report_pending("a free refused, memory kept (the first; the rest are counted): ");
	}
	/* a window (map_phys) keeps its VA and its mapping: an image's windows
	 * live as long as it does */
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return rc;
}

/* The windows map_phys opened (a native buffer, an EGLImage's memory): each
 * one's GPU range and the CPU view mapped for it, so cpu_of can answer inside
 * them too. map_phys never unmaps, so the table only grows; a window past its
 * end still works for the GPU, but the CPU cannot read it (said once). */
#define WINDOWS 32
static struct {
	uint32_t va, span;
	uint8_t *cpu;
} windows[WINDOWS];
static uint32_t nwindows;

static int z_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
		      struct osal_gpu_mem *out)
{
	(void)heap_base;
	(void)flags;
	if (!s.inited || !out || size == 0)
		return -EINVAL;
	uint64_t base = ROUND_DOWN(phys, PAGE);
	uint64_t span = ROUND_UP((phys - base) + size, PAGE);
	if ((uint64_t)s.window_va + span > VA_SPACE)
		return -ENOMEM;
	int rc = v3d_pte_map(s.pt, PT_ENTRIES, s.window_va, base, (uint32_t)span, true);
	if (rc)
		return rc;
	rc = v3d_mmu_flush(&s.regs);
	if (rc)
		return rc;
	uint8_t *cpu = NULL;
	k_mem_map_phys_bare(&cpu, (uintptr_t)base, (size_t)span, K_MEM_PERM_RW | K_MEM_ARM_NORMAL_NC);
	out->cpu = cpu ? cpu + (phys - base) : NULL;
	out->gpu = s.window_va + (uint32_t)(phys - base);
	if (nwindows < WINDOWS) {
		windows[nwindows].va = s.window_va;
		windows[nwindows].span = (uint32_t)span;
		windows[nwindows].cpu = cpu;
		nwindows++;
	} else if (nwindows == WINDOWS) {
		nwindows++; /* once */
		printk("osal(v3d): more than %u map_phys windows: the later ones have no CPU view "
		       "(a texture there is refused by name)\n",
		       WINDOWS);
	}
	out->phys = phys;
	out->size = size;
	out->impl = (void *)(uintptr_t)out->gpu;
	s.window_va += (uint32_t)span;
	return 0;
}

static int z_gpu_write(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, src, n))
		return -EINVAL;
	memcpy((uint8_t *)mem->cpu + off, src, n);
	return 0;
}

static int z_gpu_read(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, dst, n))
		return -EINVAL;
	memcpy(dst, (uint8_t *)mem->cpu + off, n);
	return 0;
}

/* GPU memory is Normal non-cacheable (see the file comment): nothing to do
 * there. A cached buffer handed in from elsewhere is maintained. */
static void z_cache_flush(void *cpu, size_t n)
{
	if (cpu && n && !in_nc(cpu))
		sys_cache_data_flush_range(cpu, n);
}

static void z_cache_invd(void *cpu, size_t n)
{
	if (cpu && n && !in_nc(cpu))
		sys_cache_data_invd_range(cpu, n);
}

/* ---- submit + completion ------------------------------------------------ */

/* The CPU view of `n` bytes at a GPU address inside the carve-out, or NULL. */
static uint8_t *cpu_of(uint32_t va, uint32_t n)
{
	if (va >= VA_BASE && (uint64_t)va - VA_BASE + n <= sizeof(carve))
		return s.nc + (va - VA_BASE);
	for (uint32_t i = 0; i < MIN(nwindows, WINDOWS); i++) {
		if (windows[i].cpu && va >= windows[i].va &&
		    (uint64_t)va - windows[i].va + n <= windows[i].span)
			return windows[i].cpu + (va - windows[i].va);
	}
	return NULL;
}

static void print_stall(const char *what, int rc, const struct v3d_stall *st)
{
	printk("osal(v3d): %s: %d core_sts 0x%x hub_sts 0x%x ct0 ca 0x%x ra 0x%x ct1 ca 0x%x "
	       "ra 0x%x err 0x%x mmu_ctl 0x%x vio 0x%x vio_id 0x%x\n",
	       what, rc, st->core_int_sts, st->hub_int_sts, st->ct0ca, st->ct0ra, st->ct1ca,
	       st->ct1ra, st->err_stat, st->mmu_ctl, st->mmu_vio_addr, st->mmu_vio_id);
}

/* A stage's time. A gl_test job takes milliseconds. Past JOB_SLOW_MS a job
 * is slow: counted, and its time printed, but its result stands. Past
 * JOB_HANG_US it has not ended (this OSAL has no progress check, so the
 * bound is generous): the GPU is reset
 * (recover), and only if that fails is it taken as hung, with nothing more
 * submitted and nothing given back, since the GPU may still use it.
 * The E runs on 8c8df5d1 had a job pass its 1 s timeout and end later,
 * when the next job's wait took its done bit: a timed-out job must not
 * overlap the next one. */
#define JOB_SLOW_MS 1000u
#define JOB_HANG_US 4000000u

/* The job record (osal_internal.h osal_v3d_diag_print; first used on the
 * textured v3d_gl_test hang, 2026-09-24): the command-list registers once
 * after the first job that completes (the baseline the stall lines are read
 * against) and at the first job that does not (v3d_regs.h). */
static bool diag_baseline_done, diag_first_failure_done;
static uint32_t diag_mmu_faults; /* stages ended by an MMU fault */
static uint32_t diag_slow, diag_slowest_ms; /* stages past JOB_SLOW_MS that ended */
static uint32_t diag_failed; /* stages that did not end (osal_v3d_job_counts) */
static uint32_t diag_recoveries; /* GPU resets after such a stage (osal_v3d_recoveries) */
static uint32_t diag_bin_oom; /* bins that ran the overflow pool dry */

/* the registers of one snapshot, kept for osal_v3d_diag_print() */
struct diag_snap {
	bool taken;
	uint32_t fence, core_sts, hub_sts, err, ct0cs, ct0ea, ct0ca, ct0ra, ct0qba, ct0qea;
	uint32_t ct1ca, ct1ra, ct1qba, ct1qea;
	uint32_t mmu_ctl, vio_id, vio_addr;
	char what[112];
};

/* The MMU's faulting client, from its AXI ID (V3D_MMU0_VIO_ID): the top
 * three bits of the low byte name the unit, in the order Mesa's simulator
 * glue prints them (handle_mmu_interruptions: L2T, PTB, PSE, TLB, CLE, TFU,
 * MMU, GMP); within a unit only the IDs seen on V3D 4.x are named, the rest
 * is "unknown". Printed text only; no check covers it on the host. */
static const char *axi_client(uint32_t id)
{
	id &= 0xffu;
	if (id < 0x20)
		return "L2T (the texture unit's cache)";
	if (id == 0x20)
		return "PTB (the binner)";
	if (id == 0x40)
		return "PSE";
	if (id >= 0x60 && id < 0x80)
		return "TLB (the tile buffer)";
	if (id >= 0x80 && id < 0x88)
		return "CLE (the control lists)";
	if (id == 0xa0)
		return "TFU";
	if (id >= 0xc0 && id < 0xe0)
		return "MMU";
	if (id == 0xe0)
		return "GMP";
	return "unknown";
}

/* V3D_MMU0_VIO_ADDR holds the top 32 bits of the faulting address: shifted
 * back by the VA width past 32 bits (as Mesa's handle_mmu_interruptions) */
static uint64_t vio_address(uint32_t raw)
{
	uint32_t w = s.id.va_width > 32 ? s.id.va_width - 32 : 0;
	return (uint64_t)raw << w;
}
static struct diag_snap diag_baseline, diag_failure;

static void diag_line(const char *prefix, const struct diag_snap *d)
{
	printk("%s job %u: core_sts 0x%x hub_sts 0x%x err 0x%x ct0 cs 0x%x ea 0x%x ca 0x%x ra 0x%x "
	       "qba 0x%x qea 0x%x ct1 ca 0x%x ra 0x%x qba 0x%x qea 0x%x\n",
	       prefix, d->fence, d->core_sts, d->hub_sts, d->err, d->ct0cs, d->ct0ea, d->ct0ca,
	       d->ct0ra, d->ct0qba, d->ct0qea, d->ct1ca, d->ct1ra, d->ct1qba, d->ct1qea);
	if (d->hub_sts & (V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET))
		printk("%s job %u MMU fault: mmu_ctl 0x%x vio_id 0x%x (%s) vio_addr 0x%llx%s\n", prefix, d->fence,
		       d->mmu_ctl, d->vio_id, axi_client(d->vio_id),
		       (unsigned long long)vio_address(d->vio_addr),
		       (d->hub_sts & V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET) ? ", a write" : "");
}

static void diag_take(struct diag_snap *d, const char *what, uint32_t fence)
{
	*d = (struct diag_snap){
		.taken = true,
		.fence = fence,
		.core_sts = sys_read32(s.regs.core + V3D_CTL_0_INT_STS),
		.hub_sts = sys_read32(s.regs.hub + V3D_HUB_CTL_INT_STS),
		.err = sys_read32(s.regs.core + V3D_ERR_0_STAT),
		.ct0cs = sys_read32(s.regs.core + V3D_CLE_0_CT0CS),
		.ct0ea = sys_read32(s.regs.core + V3D_CLE_0_CT0EA),
		.ct0ca = sys_read32(s.regs.core + V3D_CLE_0_CT0CA),
		.ct0ra = sys_read32(s.regs.core + V3D_CLE_0_CT0RA),
		.ct0qba = sys_read32(s.regs.core + V3D_CLE_0_CT0QBA),
		.ct0qea = sys_read32(s.regs.core + V3D_CLE_0_CT0QEA),
		.ct1ca = sys_read32(s.regs.core + V3D_CLE_0_CT1CA),
		.ct1ra = sys_read32(s.regs.core + V3D_CLE_0_CT1RA),
		.ct1qba = sys_read32(s.regs.core + V3D_CLE_0_CT1QBA),
		.ct1qea = sys_read32(s.regs.core + V3D_CLE_0_CT1QEA),
		.mmu_ctl = sys_read32(s.regs.hub + V3D_MMU0_CTL),
		.vio_id = sys_read32(s.regs.hub + V3D_MMU0_VIO_ID),
		.vio_addr = sys_read32(s.regs.hub + V3D_MMU0_VIO_ADDR),
	};
	snprintk(d->what, sizeof(d->what), "%s", what);
}

/* `st`: the failed wait's snapshot, or NULL. v3d_wait clears the hub's
 * interrupt status when it sees an MMU fault, so a read here would say no
 * fault: the hub status and the MMU's registers come from the snapshot, taken
 * before that clear (the struct.uniform record of 00:19 had no fault line). */
static void diag_regs(const char *what, uint32_t fence, const struct v3d_stall *st)
{
	struct diag_snap *d = strncmp(what, "BASELINE", 8) == 0 ? &diag_baseline : &diag_failure;
	diag_take(d, what, fence);
	if (st) {
		d->hub_sts = st->hub_int_sts;
		d->mmu_ctl = st->mmu_ctl;
		d->vio_id = st->mmu_vio_id;
		d->vio_addr = st->mmu_vio_addr;
	}
	char prefix[128];
	snprintk(prefix, sizeof(prefix), "osal(v3d): %s", what);
	diag_line(prefix, d);
}

/* For the sample's record, printed after the run: the two snapshots (or
 * that there is none) and how many jobs timed out. There is no GPU reset
 * after a timeout. A job that timed out is not necessarily hung: the E runs
 * on 8c8df5d1 had later cases pass after timed-out ones, so such a job can
 * end late (and its done bit can reach the next job's wait). */
uint32_t osal_v3d_bin_oom_jobs(void)
{
	return diag_bin_oom;
}

uint32_t osal_v3d_recoveries(void)
{
	return diag_recoveries;
}

/* the last job's times (osal_v3d_last_job_us), in cycles */
static struct {
	uint32_t prep, bin, render, clean, total;
	uint32_t render_late; /* osal_v3d_job_us.render_late */
} last_job_cyc;

/* the last completed job's (osal_v3d_last_completed_job_us): a copy of
 * last_job_cyc taken when the job completes, before a submit zeroes it */
static struct {
	uint32_t fence;
	uint32_t prep, bin, render, clean, total;
	uint32_t render_late;
} done_job_cyc;

/* The ring of the most recent completed jobs (osal_v3d_recent_jobs_us), in
 * cycles; a few stores per job, no printing. idle is the cycles from the end
 * of the previous job's GPU work (its render, cache clean and every TFU job
 * run since, see recent_prev_end) to this job's bin submit; 0 for the first
 * job, and when the stamps cross. tfu_n / tfu_cyc: the TFU jobs run since the
 * previous job completed (tfu_now) and their time. */
#define RECENT_JOBS OSAL_V3D_RECENT_JOBS
struct recent_job_cyc {
	uint32_t fence;
	uint32_t prep, bin, render, clean, total;
	uint32_t render_late, idle, tfu_n, tfu_cyc;
};
static struct recent_job_cyc recent_cyc[RECENT_JOBS];
static uint32_t recent_n;        /* jobs noted since init */
static uint32_t recent_prev_end; /* when the GPU last stopped working */
static uint32_t pend_tfu_n, pend_tfu_cyc;

/* c_end: the GPU's last work for this job ended (after the clean and the
 * queued TFU jobs) */
static void note_job_done(uint32_t fence, uint32_t c_end)
{
	done_job_cyc.fence = fence;
	done_job_cyc.prep = last_job_cyc.prep;
	done_job_cyc.bin = last_job_cyc.bin;
	done_job_cyc.render = last_job_cyc.render;
	done_job_cyc.clean = last_job_cyc.clean;
	done_job_cyc.total = last_job_cyc.total;
	done_job_cyc.render_late = last_job_cyc.render_late;

	const int32_t gap = (int32_t)(s.if_c_bin - recent_prev_end);
	struct recent_job_cyc *r = &recent_cyc[recent_n % RECENT_JOBS];
	r->fence = fence;
	r->prep = last_job_cyc.prep;
	r->bin = last_job_cyc.bin;
	r->render = last_job_cyc.render;
	r->clean = last_job_cyc.clean;
	r->total = last_job_cyc.total;
	r->render_late = last_job_cyc.render_late;
	r->idle = (recent_n == 0 || gap < 0) ? 0 : (uint32_t)gap;
	r->tfu_n = pend_tfu_n;
	r->tfu_cyc = pend_tfu_cyc;
	pend_tfu_n = pend_tfu_cyc = 0;
	recent_n++;
	recent_prev_end = c_end;
}

unsigned osal_v3d_recent_jobs_us(struct osal_v3d_recent_job_us *out, unsigned max)
{
	if (!out)
		return 0;
	unsigned n = recent_n < RECENT_JOBS ? recent_n : RECENT_JOBS;
	if (n > max)
		n = max;
	for (unsigned i = 0; i < n; i++) {
		const struct recent_job_cyc *r =&recent_cyc[(recent_n - n + i) % RECENT_JOBS];
		out[i].fence = r->fence;
		out[i].prep = k_cyc_to_us_floor32(r->prep);
		out[i].bin = k_cyc_to_us_floor32(r->bin);
		out[i].render = k_cyc_to_us_floor32(r->render);
		out[i].clean = k_cyc_to_us_floor32(r->clean);
		out[i].total = k_cyc_to_us_floor32(r->total);
		out[i].render_late = r->render_late;
		out[i].idle = k_cyc_to_us_floor32(r->idle);
		out[i].tfu_jobs = r->tfu_n;
		out[i].tfu = k_cyc_to_us_floor32(r->tfu_cyc);
	}
	return n;
}

void osal_v3d_last_job_us(struct osal_v3d_job_us *out)
{
	if (!out)
		return;
	out->jobs = s.submitted;
	out->prep = k_cyc_to_us_floor32(last_job_cyc.prep);
	out->bin = k_cyc_to_us_floor32(last_job_cyc.bin);
	out->render = k_cyc_to_us_floor32(last_job_cyc.render);
	out->clean = k_cyc_to_us_floor32(last_job_cyc.clean);
	out->total = k_cyc_to_us_floor32(last_job_cyc.total);
	out->render_late = last_job_cyc.render_late;
}

void osal_v3d_last_completed_job_us(struct osal_v3d_job_us *out)
{
	if (!out)
		return;
	out->jobs = done_job_cyc.fence;
	out->prep = k_cyc_to_us_floor32(done_job_cyc.prep);
	out->bin = k_cyc_to_us_floor32(done_job_cyc.bin);
	out->render = k_cyc_to_us_floor32(done_job_cyc.render);
	out->clean = k_cyc_to_us_floor32(done_job_cyc.clean);
	out->total = k_cyc_to_us_floor32(done_job_cyc.total);
	out->render_late = done_job_cyc.render_late;
}

uint32_t osal_v3d_mmu_faults(void)
{
	return diag_mmu_faults;
}

bool osal_v3d_hung(void)
{
	return hung;
}

void osal_v3d_job_counts(uint32_t *slow, uint32_t *failed, uint32_t *refused)
{
	if (slow)
		*slow = diag_slow;
	if (failed)
		*failed = diag_failed;
	if (refused)
		*refused = hung_refused;
}

void osal_v3d_mem_free_runs(uint32_t *longest, uint32_t *runs, uint32_t *free_total)
{
	uint32_t l = 0, r = 0;
	if (s.inited)
		v3d_free_runs(used_map, s.first_granule, GRANULES, &l, &r);
	if (longest)
		*longest = l * GRANULE;
	if (runs)
		*runs = r;
	if (free_total)
		*free_total = s.inited ? (GRANULES - s.first_granule - gm.used) * GRANULE : 0;
}

void osal_v3d_frees_held(uint32_t *n, uint32_t *bytes)
{
	uint32_t b = 0;
	for (unsigned int i = 0; i < frees.q.n; i++)
		b += frees.q.e[i].p.n * GRANULE;
	if (n)
		*n = frees.q.n;
	if (bytes)
		*bytes = b;
}

void osal_mem_line(char *buf, size_t n)
{
	uint32_t used, high, total, held, held_bytes, longest, runs;

	osal_v3d_mem_used(&used, &high, &total);
	osal_v3d_frees_held(&held, &held_bytes);
	osal_v3d_mem_free_runs(&longest, &runs, NULL);
	snprintk(buf, n,
		 "osal(v3d) mem: carve-out used %u, high-water %u, of %u bytes; frees held %u (%u bytes); "
		 "longest free run %u bytes (%u runs)",
		 used, high, total, held, held_bytes, longest, runs);
}

void osal_v3d_find_counts(uint32_t *calls, uint64_t *reads, uint32_t *reads_max)
{
	if (calls)
		*calls = gm.finds;
	if (reads)
		*reads = gm.reads;
	if (reads_max)
		*reads_max = gm.reads_max;
}

void osal_v3d_pack_counts(bool *on, uint32_t *jobs, uint32_t *emitted, uint32_t *skipped,
			  uint32_t *compared, uint32_t *mismatched)
{
	struct v3d_job_pack_counts n;
	uint32_t ended;
	v3d_job_pack_totals(&n, &ended);
	if (on)
		*on = v3d_job_pack_skip;
	if (jobs)
		*jobs = ended;
	if (emitted)
		*emitted = n.emitted;
	if (skipped)
		*skipped = n.skipped;
	if (compared)
		*compared = n.compared;
	if (mismatched)
		*mismatched = n.mismatched;
}

void osal_v3d_mem_used(uint32_t *used, uint32_t *high, uint32_t *total)
{
	/* the allocator's granules: the carve-out past the page table and the
	 * scratch page (the binner's overflow blocks, taken at init, count) */
	uint32_t heap = s.inited ? GRANULES - s.first_granule : 0;
	if (used)
		*used = gm.used * GRANULE;
	if (high)
		*high = gm.high * GRANULE;
	if (total)
		*total = heap * GRANULE;
}

void osal_v3d_diag_print(void)
{
	if (diag_baseline.taken)
		diag_line("BASELINE (first completed)", &diag_baseline);
	else
		printk("BASELINE: none (no job completed)\n");
	if (diag_failure.taken)
		diag_line(diag_failure.what, &diag_failure);
	else
		printk("FIRST FAILURE: none (no timeout)\n");
	printk("slow jobs: %u (past %u ms, ended; the slowest %u ms); MMU faults: %u; recoveries: %u; hung: %s",
	       diag_slow, JOB_SLOW_MS, diag_slowest_ms, diag_mmu_faults, diag_recoveries, hung ? "" : "none\n");
	if (hung)
		printk("job %u (the GPU reset failed), %u later jobs not submitted, %u KiB not given back\n",
		       hung_fence, hung_refused, hung_leaked_kib);
	printk("binner out of memory: %u jobs (%u overflow blocks of %u KiB, "
	       "CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS; each such job ends in a GPU reset)\n",
	       diag_bin_oom, s.ov.nblocks, BIN_OVERFLOW_BLOCK / 1024);
	printk("binning by interrupt (IRQ %u): %u interrupts, %u overflow blocks fed by the ISR\n", V3D_IRQ,
	       isr_calls, isr_overflow_fed);
	{
		uint32_t used, high, total;
		osal_v3d_mem_used(&used, &high, &total);
		printk("carve-out: %u bytes used now, %u at most, of %u; %u frees held\n", used, high, total,
		       frees.q.n);
		report_pending("");
		/* the allocator's search cost (v3d_granules.h): a map read tests
		 * one 32-granule word; then the fragmentation it leaves (the
		 * longest free run is the largest block that fits) and whether
		 * small blocks come from the top */
		uint32_t longest, runs;
		osal_v3d_mem_free_runs(&longest, &runs, NULL);
		printk("carve-out find: %u calls, %llu map reads (%llu a call, at most %u in one); free runs %u, "
		       "longest %u bytes; small blocks from the top: ", gm.finds, (unsigned long long)gm.reads,
		       (unsigned long long)(gm.finds ? gm.reads / gm.finds : 0), gm.reads_max, runs, longest);
		if (gm.top_below)
			printk("below %u bytes (CONFIG_YAKOGL_V3D_CARVEOUT_TOP_BELOW)\n", gm.top_below * GRANULE);
		else
			printk("off (CONFIG_YAKOGL_V3D_CARVEOUT_TOP_BELOW 0)\n");
		/* the job builder's packet skip (v3d_job.h), over the jobs ended */
		struct v3d_job_pack_counts pk;
		uint32_t pk_jobs;
		v3d_job_pack_totals(&pk, &pk_jobs);
		printk("packets (packet skip %s, CONFIG_YAKOGL_V3D_PACK_SKIP): %u jobs ended; emitted %u, "
		       "skipped %u; groups compared %u, mismatches %u\n", v3d_job_pack_skip ? "on" : "off",
		       pk_jobs, pk.emitted, pk.skipped, pk.compared, pk.mismatched);
		unsigned sizes = 0;
		for (unsigned i = 0; i <= OOM_SIZES; i++)
			sizes += oom_sizes[i].refused != 0;
		printk("allocations refused for lack of GPU memory: %u (%u size slots; the line is printed at "
		       "each size's 1st, 10th, 100th, ... refusal)\n", oom_refused_total, sizes);
	}
	printk("UIFCFG 0x%08x (read at init)\n", diag_uifcfg);
	{
		char line[96];

		yakogl_log_repeats_report(line, sizeof(line));
		printk("%s\n", line);
	}
}

/* A stage did not end (its registers already recorded): reset the GPU, which
 * on the BCM2711 is a power cycle (v3d_reset: power off, power on, the MMU
 * on the same table). The job's frame is lost either way; after a reset the GPU is
 * idle, so what the job held can come back and later jobs are submitted.
 * Only when the reset itself fails is the GPU taken as hung. */

/* The binner's end, from its interrupt (OUTOMEM is edge-signalled and, on
 * V3D 4.x, FLDONE does not come while the binner is out of memory). OUTOMEM: the next overflow
 * block, as wait_bits did in the spin; none left: the job's verdict is
 * -ENOMEM and the binner stays stalled until the waiter resets the GPU (as
 * the spin's -ENOMEM did). FLDONE: the pool as binning left it, then the
 * render starts (register writes only: v3d_submit_render). A fence is never
 * completed here: FRDONE, the TMU clean and the fence stay in
 * finish_inflight (the frees rule, v3d_frees.h). */
static void v3d_isr(const void *arg)
{
	ARG_UNUSED(arg);
	isr_calls++;
	const uint32_t sts = sys_read32(s.regs.core + V3D_CTL_0_INT_STS) & V3D_BIN_IRQS;
	if (!sts)
		return;
	sys_write32(sts, s.regs.core + V3D_CTL_0_INT_CLR);
	if (!s.bin_pending || s.bin_rc)
		return; /* not a binning this OSAL started, or one already failed */
	if (sts & V3D_CTL_0_INT_STS_INT_OUTOMEM_SET) {
		if (s.ov.used < s.ov.nblocks) {
			sys_write32(s.ov.blocks[s.ov.used], s.regs.core + V3D_PTB_0_BPOA);
			sys_write32(s.ov.block_size, s.regs.core + V3D_PTB_0_BPOS);
			s.ov.used++;
			isr_overflow_fed++;
		} else {
			sys_write32(V3D_BIN_IRQS, s.regs.core + V3D_CTL_0_INT_MSK_SET);
			s.bin_rc = -ENOMEM;
			return;
		}
	}
	if (sts & V3D_CTL_0_INT_STS_INT_FLDONE_SET) {
		sys_write32(V3D_BIN_IRQS, s.regs.core + V3D_CTL_0_INT_MSK_SET);
		s.if_c_fldone = k_cycle_get_32();
		s.bin = (struct osal_v3d_bin_pool){
			.jobs = s.bin.jobs + 1,
			.pool = s.if_tile_alloc,
			.pool_size = s.if_tile_alloc_size,
			.bpca = sys_read32(s.regs.core + V3D_PTB_0_BPCA),
			.bpcs = sys_read32(s.regs.core + V3D_PTB_0_BPCS),
			.qma = sys_read32(s.regs.core + V3D_CLE_0_CT0QMA),
			.qms = sys_read32(s.regs.core + V3D_CLE_0_CT0QMS),
			.overflow_used = s.ov.used,
			.overflow_blocks = s.ov.nblocks,
			.overflow_block_size = s.ov.block_size,
		};
		s.if_c_render = k_cycle_get_32();
		v3d_submit_render(&s.regs, s.if_rcl_start, s.if_rcl_end);
		barrier_dmem_fence_full();
		s.bin_pending = false;
	}
}

static void recover(uint32_t fence)
{
	/* the reset masks every interrupt (v3d_irq_quiesce); nothing of the
	 * reset job is left for the ISR */
	s.bin_pending = false;
	int rc = v3d_reset(&s.regs, &s.clk, s.phys, s.phys + PT_BYTES);
	if (rc == 0) {
		diag_recoveries++;
		printk("osal(v3d): job %u: GPU reset (recovery %u)\n", fence, diag_recoveries);
		return;
	}
	hung = true;
	hung_fence = fence;
	printk("osal(v3d): job %u: the GPU reset failed (%d): the GPU is taken as hung; no later job is "
	       "submitted\n", fence, rc);
}

/* A failed stage's record, once (the first failure), for both stages. */
static void first_failure(const char *stage, int rc, uint32_t fence, uint32_t bcl_start, uint32_t bcl_end,
			  uint32_t rcl_start, uint32_t rcl_end, const struct v3d_stall *st)
{
	if (diag_first_failure_done)
		return;
	diag_first_failure_done = true;
	char what[112];
	snprintk(what, sizeof(what), "FIRST FAILURE: %s, rc %d, bcl 0x%x..0x%x rcl 0x%x..0x%x", stage, rc,
		 bcl_start, bcl_end, rcl_start, rcl_end);
	diag_regs(what, fence, st);
}

/* TFU jobs queued behind the newest job submitted (OSAL_QUERY_V3D_TFU_QUEUE;
 * the job in flight, or the one waiting behind it, tagged in tfu_q.fence):
 * run by finish_inflight once that job's render is over and the caches are
 * clean, in the order queued, before its fence completes, so a wait() on it
 * or the next submit comes after them. The TFU and a render never run side by
 * side here (osal_v3d_tfu), and the job waiting behind the one in flight
 * starts only after finish_inflight, so the queue orders the render that
 * stores a level 0, the TFU that makes the chain from it, and the job that
 * samples the chain, as Mesa's in-sync on the render's out-sync orders the
 * kernel's TFU and render queues. The CPU does not wait for the render to
 * queue a job (glGenerateMipmap returns at once); it runs the TFU job itself
 * when it finishes the render (wait(), completed() or the next submit), a
 * TFU job's time and not the render's. */
#define TFU_QUEUE_MAX 4
static struct {
	struct v3d_tfu_job j[TFU_QUEUE_MAX];
	uint32_t fence[TFU_QUEUE_MAX]; /* the job each one follows */
	uint32_t n;
	uint32_t ran, dropped, failed; /* since init */
} tfu_q;

static int tfu_now(const struct v3d_tfu_job *j, struct v3d_tfu_status *st);

/* The queued TFU jobs that follow job `fence`, after its render ended with
 * `render_rc`: run when it succeeded, dropped (counted) when it failed or a
 * TFU job before them failed (the GPU was reset under them). The ones that
 * follow the job waiting behind it stay queued. */
static void tfu_queue_run(uint32_t fence, int render_rc)
{
	int rc = render_rc;
	uint32_t dropped = 0, kept = 0;

	for (uint32_t i = 0; i < tfu_q.n; i++) {
		if (tfu_q.fence[i] != fence) {
			tfu_q.j[kept] = tfu_q.j[i];
			tfu_q.fence[kept++] = tfu_q.fence[i];
			continue;
		}
		if (rc || hung) {
			dropped++;
			continue;
		}
		struct v3d_tfu_status st;

		rc = tfu_now(&tfu_q.j[i], &st);
		if (rc)
			tfu_q.failed++;
		else
			tfu_q.ran++;
	}
	tfu_q.n = kept;
	if (dropped) {
		tfu_q.dropped += dropped;
		printk("osal(v3d): %u queued TFU job(s) dropped after a failed job (%u so far; %u failed)\n", dropped,
		       tfu_q.dropped, tfu_q.failed);
	}
}

/* A fence is over (done, or failed and recovered from): it completes, wait()
 * reports how, and what it held goes back. */
static void complete(uint32_t fence, int rc)
{
	s.prev_completed = s.completed;
	s.prev_rc = s.last_rc;
	s.completed = fence;
	s.last_rc = rc;
	drain();
}

/* The binner's part of the job in flight, ended: 0 when the ISR saw FLDONE
 * and started the render (the render is in flight now), 1 when `poll` found
 * the binner still running, and the job's error when binning failed: the ISR
 * found the overflow pool dry (-ENOMEM), an MMU fault (-EFAULT), or no
 * FLDONE within JOB_HANG_US of the bin's start (-ETIMEDOUT). A failure ends
 * as the spin's did: the record, the counters, a GPU reset, and the fence
 * completes with the error. */
static int bin_finish(bool poll)
{
	const uint32_t fence = s.if_fence;
	const uint32_t faults = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;
	for (;;) {
		/* the verdict is read and, for a failure, the ISR shut out (the two
		 * interrupts masked, bin_pending cleared) under one irq_lock: an
		 * FLDONE that arrived between the bound's read and the mask would
		 * otherwise start a render the reset below then tears down (glue's
		 * review of 679be18d) */
		unsigned int key = irq_lock();
		barrier_dmem_fence_full();
		if (!s.bin_pending && !s.bin_rc) {
			irq_unlock(key);
			last_job_cyc.bin = s.if_c_fldone - s.if_c_bin;
			const uint32_t ms = k_cyc_to_us_floor32(last_job_cyc.bin) / 1000u;
			if (ms > JOB_SLOW_MS) {
				diag_slow++;
				if (ms > diag_slowest_ms)
					diag_slowest_ms = ms;
				printk("osal(v3d): job %u bin took %u ms (slow; past %u ms)\n", fence, ms, JOB_SLOW_MS);
			}
			return 0;
		}
		const uint32_t us = k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_bin);
		const int rc = s.bin_rc						? s.bin_rc :
			       (sys_read32(s.regs.hub + V3D_HUB_CTL_INT_STS) & faults) ? -EFAULT :
			       us >= JOB_HANG_US				? -ETIMEDOUT :
										  0;
		if (rc) {
			sys_write32(V3D_BIN_IRQS, s.regs.core + V3D_CTL_0_INT_MSK_SET);
			s.bin_pending = false; /* the ISR ignores what is left */
		}
		irq_unlock(key);
		if (rc) {
			struct v3d_stall st;
			v3d_stall_snapshot(&s.regs, &st);
			diag_failed++;
			printk("osal(v3d): job %u bin did not end (%d, after %u ms)\n", fence, rc, us / 1000u);
			first_failure("bin", rc, fence, s.if_bcl_start, s.if_bcl_end, s.if_rcl_start, s.if_rcl_end,
				      &st);
			if (rc == -ENOMEM)
				diag_bin_oom++;
			else if (rc == -EFAULT)
				diag_mmu_faults++;
			print_stall("bin", rc, &st);
			recover(fence);
			s.bin_rc = 0;
			s.inflight = false;
			tfu_queue_run(fence, rc); /* dropped: the job they follow failed */
			/* the job is over (failed, recovered): its fence completes */
			complete(fence, rc);
			last_job_cyc.bin = k_cycle_get_32() - s.if_c_bin;
			last_job_cyc.total = k_cycle_get_32() - s.if_c0;
			note_job_done(fence, k_cycle_get_32());
			start_pending();
			return rc;
		}
		if (poll)
			return 1;
		k_busy_wait(10);
	}
}

/* Is the in-flight job's render over, or is it past its hang bound? Reads
 * the status registers only: the done bit is left for the wait that
 * consumes it (wait_bits clears only what it waits for). While its binner
 * runs, the job is ready only when binning has failed (bin_finish ends it). */
static bool inflight_ready(void)
{
	if (s.bin_pending || s.bin_rc) {
		const uint32_t faults = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;
		return s.bin_rc || (sys_read32(s.regs.hub + V3D_HUB_CTL_INT_STS) & faults) ||
		       k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_bin) >= JOB_HANG_US;
	}
	const uint32_t core = sys_read32(s.regs.core + V3D_CTL_0_INT_STS);
	const uint32_t hub = sys_read32(s.regs.hub + V3D_HUB_CTL_INT_STS);
	const uint32_t faults = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;
	const uint32_t elapsed_us = k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_render);
	return (core & V3D_CTL_0_INT_STS_INT_FRDONE_SET) || (hub & faults) || elapsed_us >= JOB_HANG_US;
}

/* Finish the job in flight: wait for FRDONE, bounded by JOB_HANG_US counted
 * from the render's start, and, when poll is set, not at all (return at once
 * if the render is not over); then the TMU clean, and the fence completes.
 * Returns 0 when the job is over and it completed (or none was in flight), 1
 * when poll found it still running (it stays in flight, for a later wait,
 * the next submit or completed()), and the job's error when it failed: the
 * render did not end within JOB_HANG_US, or an MMU fault; the GPU is then
 * reset (recover) and the fence completes with that error.
 *
 * last_job_cyc.render is the render's start to the moment this function saw
 * FRDONE, so it includes any time the CPU spent elsewhere before asking; with
 * the backend waiting right after every submit, that is the render. */
static int finish_inflight(bool poll)
{
	if (!s.inflight)
		return 0;
	if (s.bin_pending || s.bin_rc) {
		const int brc = bin_finish(poll);
		if (brc)
			return brc; /* 1: still binning (poll); <0: failed, completed */
	}
	struct v3d_stall st;
	const uint32_t fence = s.if_fence;
	const uint32_t t0_ms = k_uptime_get_32();
	/* the bin's time from its two stamps (the submit and the FLDONE
	 * interrupt): bin_finish writes it only when it had to wait for the
	 * binner, which a job finished at the next submit never does */
	last_job_cyc.bin = s.if_c_fldone - s.if_c_bin;
	if (poll && !inflight_ready())
		return 1; /* still in flight; checked quietly (v3d_wait logs its
			   * own timeout as an error, and a poll is not a failure) */
	/* FRDONE has no interrupt: when it is already up, the render ended at an
	 * unknown time before now, and the end stamp below is this look's */
	last_job_cyc.render_late =
		(sys_read32(s.regs.core + V3D_CTL_0_INT_STS) & V3D_CTL_0_INT_STS_INT_FRDONE_SET) ? 1u : 0u;
	/* the wait that consumes FRDONE, bounded by JOB_HANG_US from the start */
	const uint32_t elapsed_us = k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_render);
	int rc = v3d_wait(&s.regs, V3D_CTL_0_INT_STS_INT_FRDONE_SET, elapsed_us >= JOB_HANG_US ? 0 : JOB_HANG_US - elapsed_us,
			  &st);
	const uint32_t c_frdone = k_cycle_get_32();
	s.inflight = false;
	last_job_cyc.render = c_frdone - s.if_c_render;
	const uint32_t ms = k_cyc_to_us_floor32(last_job_cyc.render) / 1000u;
	if (rc == 0 && ms > JOB_SLOW_MS) {
		diag_slow++;
		if (ms > diag_slowest_ms)
			diag_slowest_ms = ms;
		printk("osal(v3d): job %u render took %u ms (slow; past %u ms)\n", fence, ms, JOB_SLOW_MS);
	}
	if (rc) {
		diag_failed++;
		printk("osal(v3d): job %u render did not end (%d, after %u ms, %u ms of it in this wait)\n", fence,
		       rc, ms, k_uptime_get_32() - t0_ms);
		first_failure("render", rc, fence, s.if_bcl_start, s.if_bcl_end, s.if_rcl_start, s.if_rcl_end,
			      &st);
		if (rc == -EFAULT)
			diag_mmu_faults++;
		print_stall("render", rc, &st);
		recover(fence);
	} else {
		rc = v3d_caches_clean(&s.regs);
		last_job_cyc.clean = k_cycle_get_32() - c_frdone;
		if (!rc && !diag_baseline_done) {
			diag_baseline_done = true;
			diag_regs("BASELINE after a completed", fence, NULL);
		}
	}
	/* the TFU jobs queued behind this job: after its render and clean,
	 * before its fence completes (tfu_q) */
	tfu_queue_run(fence, rc);
	const uint32_t c_end = k_cycle_get_32(); /* the GPU's last work for this job */
	complete(fence, rc);
	last_job_cyc.total = c_end - s.if_c0;
	note_job_done(fence, c_end);
	start_pending();
	return rc;
}

/* Every job submitted, finished: the running one, and the one waiting behind
 * it, which starts when the first is over. */
static void finish_all(void)
{
	while (s.inflight)
		(void)finish_inflight(false);
}

int osal_v3d_tfu(const struct v3d_tfu_job *j, struct v3d_tfu_status *st)
{
	struct v3d_tfu_status none;

	if (!st)
		st = &none;
	*st = (struct v3d_tfu_status){ 0 };
	if (!s.inited || !j)
		return -ENODEV;
	if (hung)
		return -EIO;
	if (!(s.id.hub_ident[1] & V3D_HUB_CTL_IDENT1_WITH_TFU_SET))
		return -ENOTSUP;
	/* the TFU must not run beside a render (the render may read what it
	 * writes, and both use the TMU/L2T caches): a job still in flight is
	 * finished first, so the TFU is alone (tex, 2026-09-26) */
	finish_all();
	if (hung)
		return -EIO; /* that job's recovery failed */
	return tfu_now(j, st);
}

/* One TFU job on the idle GPU (no render in flight): its result; a fault or
 * a timeout is logged and the GPU reset. Every TFU job passes here: it is
 * counted and timed for the ring (pend_tfu_*), and the GPU's last-work stamp
 * (recent_prev_end) moves to its end, so its time is not taken for idle. */
static int tfu_now(const struct v3d_tfu_job *j, struct v3d_tfu_status *st)
{
	const uint32_t c_tfu = k_cycle_get_32();
	int rc = v3d_tfu_run(&s.regs, j, JOB_HANG_US, st);

	pend_tfu_n++;
	recent_prev_end = k_cycle_get_32();
	pend_tfu_cyc += recent_prev_end - c_tfu;
	if (rc == -EFAULT || rc == -ETIMEDOUT) {
		diag_failed++;
		if (rc == -EFAULT)
			diag_mmu_faults++;
		printk("osal(v3d): TFU job did not end (%d after %u us): hub int 0x%08x, TFU_CS 0x%08x -> 0x%08x, "
		       "MMU vio addr 0x%08x id 0x%08x; iia 0x%08x ioa 0x%08x ios 0x%08x icfg 0x%08x\n",
		       rc, st->us, st->hub_int_sts, st->cs_before, st->cs_after,
		       sys_read32(s.regs.hub + V3D_MMU0_VIO_ADDR), sys_read32(s.regs.hub + V3D_MMU0_VIO_ID), j->iia,
		       j->ioa, j->ios, j->icfg);
		recover(0);
	}
	return rc;
}

static int64_t z_submit(const void *kick_desc, size_t len)
{
	const struct v3d_kick *k = kick_desc;

	if (!s.inited || !k || len < sizeof(*k))
		return -EINVAL;
	if (k->magic != V3D_KICK_MAGIC) {
		printk("osal(v3d): kick magic 0x%08x is not a V3D kick\n", k->magic);
		return -EINVAL;
	}
	uint8_t *ts = cpu_of(k->tile_state, k->tile_state_size);
	if (!ts || k->bcl_end <= k->bcl_start || k->rcl_end <= k->rcl_start) {
		printk("osal(v3d): a kick with bad lists or tile state\n");
		return -EINVAL;
	}
	/* two jobs at most: one running, one waiting behind it. With one
	 * waiting already, the running one is finished first (which starts the
	 * waiting one) and this one takes the slot. (A job's result is its own
	 * fence's, reported by wait.) */
	if (s.pending)
		(void)finish_inflight(false);

	const uint32_t c0 = k_cycle_get_32();
	uint32_t fence = ++s.submitted;
	if (hung) {
		/* the hung job may still run: this one is not submitted */
		hung_refused++;
		complete(fence, -EIO);
		return (int64_t)fence;
	}
	if (s.inflight) {
		/* the binner and the tile memory belong to the job running: this
		 * one waits for it (start_pending) */
		s.pend_kick = *k;
		s.pend_fence = fence;
		s.pend_c0 = c0;
		s.pending = true;
		return (int64_t)fence;
	}
	start_job(k, fence, c0);
	return (int64_t)fence;
}

/* Start job `fence` on the idle binner: the render follows from v3d_isr at
 * FLDONE. `c0` is when the job was submitted (the job's total time counts
 * from there). */
static void start_job(const struct v3d_kick *k, uint32_t fence, uint32_t c0)
{
	uint8_t *ts = cpu_of(k->tile_state, k->tile_state_size);

	last_job_cyc.prep = last_job_cyc.bin = last_job_cyc.render = last_job_cyc.clean = 0;
	last_job_cyc.total = 0;
	last_job_cyc.render_late = 0;
	/* the tile state array starts zeroed (Mesa allocates it fresh for
	 * every job); the previous job has finished with it */
	memset(ts, 0, k->tile_state_size);

	const uint32_t c_bin = k_cycle_get_32();
	last_job_cyc.prep = c_bin - c0;
	/* The job is in flight from its bin's submit: v3d_isr takes FLDONE (and
	 * OUTOMEM) and starts the render; the CPU returns now instead of
	 * spinning through the binning (lever 2 of the per-unit cost table:
	 * 158-227 us of every swap, DEMO17 / DRAWPROFCLS). A stale done or
	 * out-of-memory bit is cleared before the two are unmasked, so it
	 * cannot be taken for this job's. */
	s.if_fence = fence;
	s.if_c0 = c0;
	s.if_c_bin = c_bin;
	s.if_bcl_start = k->bcl_start;
	s.if_bcl_end = k->bcl_end;
	s.if_rcl_start = k->rcl_start;
	s.if_rcl_end = k->rcl_end;
	s.if_tile_alloc = k->tile_alloc;
	s.if_tile_alloc_size = k->tile_alloc_size;
	s.ov.used = 0;
	s.bin_rc = 0;
	s.bin_pending = true;
	s.inflight = true;
	sys_write32(V3D_BIN_IRQS, s.regs.core + V3D_CTL_0_INT_CLR);
	barrier_dmem_fence_full();
	sys_write32(V3D_BIN_IRQS, s.regs.core + V3D_CTL_0_INT_MSK_CLR);
	v3d_submit_bin(&s.regs, k->bcl_start, k->bcl_end, k->tile_alloc, k->tile_alloc_size,
		       k->tile_state);
}

/* The job in flight is over: the one waiting behind it starts now (the GPU
 * is idle: the binner, the tile memory and the TMU are free), or, when the
 * GPU is taken as hung, is refused as a submit then is. FRDONE has no
 * interrupt, so this runs where the OSAL looks: wait(), completed(), the
 * next submit, an allocation. */
static void start_pending(void)
{
	if (!s.pending)
		return;
	s.pending = false;
	if (hung) {
		hung_refused++;
		tfu_queue_run(s.pend_fence, -EIO);
		complete(s.pend_fence, -EIO);
		return;
	}
	start_job(&s.pend_kick, s.pend_fence, s.pend_c0);
}

/* The highest completed fence, never blocking: a job in flight whose render
 * is already over (or past its hang bound) is finished here first. */
static int64_t z_completed(void)
{
	if (!s.inited)
		return 0;
	if (s.inflight && inflight_ready())
		(void)finish_inflight(false); /* over, faulted or past the bound: no wait */
	return (int64_t)s.completed;
}

/* 0 when the fence has completed and its job succeeded, 1 when a poll
 * (timeout_us == 0) found the job still in flight, <0 the job's error.
 *
 * The job in flight is waited for until it ends or passes JOB_HANG_US (4 s
 * from its render's start), whatever timeout_us > 0 or < 0 the caller
 * passes: the backend passes JOB_TIMEOUT_US (1 s), and before the
 * asynchronous submit the OSAL waited inside submit() up to JOB_HANG_US
 * regardless of it, so a render of 1-4 s completed and its frame stood.
 * That stays so until the backend asks otherwise (tex's asynchronous
 * backend: it polls with timeout_us 0 or completed(), which the synchronous
 * backend never does). Honouring a caller's bound between 0 and JOB_HANG_US
 * is left for that step, measured with it.
 *
 * An older fence answers at once: the last two completed jobs' results (the
 * backend keeps two jobs outstanding, so both may have completed, in either
 * order of its waits, before it waits for the older), 0 for earlier ones
 * (an error of those was the result of their own wait). */
static int z_wait(int64_t fence, int64_t timeout_us)
{
	if (!s.inited || fence <= 0 || fence > s.submitted)
		return -EINVAL;
	/* the job waiting behind the one in flight: that one is finished first,
	 * which starts this one */
	if (s.pending && (uint32_t)fence == s.pend_fence) {
		int rc = finish_inflight(timeout_us == 0);

		if (rc == 1)
			return 1;
	}
	if (s.inflight && (uint32_t)fence == s.if_fence) {
		int rc = finish_inflight(timeout_us == 0);
		if (rc == 1)
			return 1;
	}
	if ((uint32_t)fence == s.completed)
		return s.last_rc;
	return (uint32_t)fence == s.prev_completed ? s.prev_rc : 0;
}

/* ---- queries -------------------------------------------------------------- */

/* src/osal/mod.rs OsalV3dCpuOf / OsalV3dBinPool pin the same sizes */
BUILD_ASSERT(sizeof(struct osal_v3d_cpu_of) == 24, "struct osal_v3d_cpu_of");
BUILD_ASSERT(sizeof(struct osal_v3d_bin_pool) == 40, "struct osal_v3d_bin_pool");
BUILD_ASSERT(sizeof(struct osal_v3d_tfu) == 40, "struct osal_v3d_tfu");
BUILD_ASSERT(sizeof(struct osal_v3d_tfu_queue) == 56, "struct osal_v3d_tfu_queue");

static int z_query(uint32_t what, void *out, size_t out_sz)
{
	if (what == OSAL_QUERY_V3D_VPM_BYTES && out && out_sz == sizeof(uint32_t) && s.inited) {
		*(uint32_t *)out = s.id.vpm_size;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_CPU_OF_GPU && out && out_sz == sizeof(struct osal_v3d_cpu_of) &&
	    s.inited) {
		struct osal_v3d_cpu_of *q = out;
		if (q->gpu > UINT32_MAX || q->bytes > UINT32_MAX)
			return -EINVAL;
		q->cpu = cpu_of((uint32_t)q->gpu, (uint32_t)q->bytes);
		return q->cpu ? 0 : -EINVAL;
	}
	if (what == OSAL_QUERY_V3D_JOB_OPEN && out && out_sz == sizeof(uint32_t) && s.inited) {
		frees.job_open = *(const uint32_t *)out != 0;
		if (!frees.job_open)
			drain(); /* the job has run: what it held goes back */
		return 0;
	}
	if (what == OSAL_QUERY_V3D_SPLIT_ZS && out && out_sz == sizeof(uint32_t)) {
		*(uint32_t *)out = IS_ENABLED(CONFIG_YAKOGL_V3D_SPLIT_ZS) ? 1u : 0u;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_S8_STENCIL && out && out_sz == sizeof(uint32_t)) {
		*(uint32_t *)out = IS_ENABLED(CONFIG_YAKOGL_V3D_S8_STENCIL) ? 1u : 0u;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_TILED_BUDGET && out && out_sz == sizeof(uint32_t)) {
		*(uint32_t *)out = CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_DRAWS_PER_JOB && out && out_sz == sizeof(uint32_t)) {
		*(uint32_t *)out = CONFIG_YAKOGL_V3D_DRAWS_PER_JOB;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_BIN_POOL && out && out_sz == sizeof(struct osal_v3d_bin_pool) &&
	    s.inited) {
		if (s.bin.jobs == 0)
			return -ENODATA;
		*(struct osal_v3d_bin_pool *)out = s.bin;
		return 0;
	}
#ifdef CONFIG_YAKOGL_V3D_TFU
	if (what == OSAL_QUERY_V3D_TFU_MIN_BYTES && out && out_sz == sizeof(uint32_t)) {
		*(uint32_t *)out = CONFIG_YAKOGL_V3D_TFU_MIN_BYTES;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_TFU_RUN && out && out_sz == sizeof(struct osal_v3d_tfu)) {
		struct osal_v3d_tfu *q = out;
		const struct v3d_tfu_job j = {
			.icfg = q->icfg, .iia = q->iia, .iis = q->iis, .ica = q->ica,
			.iua = q->iua, .ioa = q->ioa, .ios = q->ios, .coef0 = q->coef0,
		};
		struct v3d_tfu_status st;

		q->rc = osal_v3d_tfu(&j, &st);
		q->us = st.us;
		return 0;
	}
	if (what == OSAL_QUERY_V3D_TFU_QUEUE && out && out_sz == sizeof(struct osal_v3d_tfu_queue) && s.inited) {
		struct osal_v3d_tfu_queue *q = out;
		const struct v3d_tfu_job j = {
			.icfg = q->tfu.icfg, .iia = q->tfu.iia, .iis = q->tfu.iis, .ica = q->tfu.ica,
			.iua = q->tfu.iua, .ioa = q->tfu.ioa, .ios = q->tfu.ios, .coef0 = q->tfu.coef0,
		};

		if (q->after < 0 || q->after > (int64_t)s.submitted)
			return -EINVAL;
		/* a render in flight (the one `after` names, or one submitted
		 * after it): the job waits in tfu_q for finish_inflight; none: the
		 * job it follows is over, and it runs now */
		if (s.inflight && !hung) {
			if (tfu_q.n == TFU_QUEUE_MAX)
				return -EBUSY;
			/* behind the newest job: the one waiting, else the one running */
			q->fence = s.pending ? s.pend_fence : s.if_fence;
			tfu_q.fence[tfu_q.n] = q->fence;
			tfu_q.j[tfu_q.n++] = j;
			q->tfu.rc = 0;
			q->tfu.us = 0;
			return 0;
		}
		struct v3d_tfu_status st;

		q->fence = 0;
		q->tfu.rc = osal_v3d_tfu(&j, &st);
		q->tfu.us = st.us;
		return 0;
	}
#endif
	if (what == OSAL_QUERY_CLOCK_NS || what == OSAL_QUERY_DRAW_STATS)
		return osal_zephyr_timing_query(what, out, out_sz);
	return -ENOTSUP;
}

/* ---- diagnostics ---------------------------------------------------------- */

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

/* thread_id (gles_osal.h, "threads"): the running thread's k_thread
 * address, folded to 32 bits. On these boards RAM lies below 4 GiB, so the
 * fold keeps the address as it is and two live threads never share it. */
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
	.present     = yakogl_osal_zephyr_present,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
