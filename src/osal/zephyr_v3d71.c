/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_v3d71.c - Zephyr OSAL backend for YakoGL on the Broadcom V3D 7.1
 * (Raspberry Pi 5, BCM2712 D0, V3D 7.1.10). The sibling of zephyr_v3d.c (the
 * Pi 4's V3D 4.2): the same OSAL contract (gles_osal.h's ops, the kick
 * descriptor of src/v3d/v3d_kick.h, the carve-out, submit and wait), the
 * same instruments (zephyr_v3d.h), on driver/src/v3d/v3d_hw.c's BCM2712 path
 * (struct v3d_regs.soc = V3D_SOC_BCM2712; driver/docs/rpi5-v3d71-facts.md).
 * CONFIG_YAKOGL_GPU_V3D71 builds this file instead of zephyr_v3d.c; the Pi
 * 4's OSAL is not changed by it.
 *
 *   init        -> the PM watchdog checked armed (PM_WDOG's count, read
 *                  before any V3D register: the board has no remote power,
 *                  so nothing touches the GPU unless the reset that brings
 *                  Linux back is already coming; rpi4/rpi_wdog.c arms it in
 *                  the first PRE_KERNEL_1 hook, CONFIG_YAKOGL_RPI5_WDOG, as
 *                  in the rpi_5 probes), then the hub, core, SMS and pm
 *                  mapped, v3d_power_on (firmware clock 5, pm's 2712 reset
 *                  bit, the SMS), v3d_identify (7.1 or refused by name), the
 *                  carve-out mapped Normal non-cacheable, the page table over
 *                  it (36-bit PA: a 24-bit page number), v3d_mmu_init
 *   gpu_alloc   -> the 4.2 OSAL's granule map over the carve-out
 *                  (v3d_granules.h), small blocks from the top opt-in
 *   gpu_free    -> deferred until the newest job that may read it has
 *                  completed (v3d_frees.h, as zephyr_v3d.c)
 *   map_phys    -> a window's pages in the MMU and a Normal non-cacheable
 *                  CPU view of them
 *   submit      -> the job in flight finished first (one job on the GPU at
 *                  a time), the tile state zeroed, v3d_submit_bin and the
 *                  binning polled to FLDONE with the overflow blocks in hand
 *                  (v3d_wait_bin, as the probes do it on the board), then
 *                  v3d_submit_render: it returns with the render running
 *   wait        -> FRDONE polled (up to JOB_HANG_US from the render's
 *                  start; a timeout of 0 only looks), v3d_caches_clean, the
 *                  fence completed and the held frees drained
 *   completed   -> the same finish when FRDONE is already up, never blocking
 *
 * What differs from zephyr_v3d.c, said rather than hidden:
 *   - the binning is polled inside submit, not taken from the interrupt:
 *     the BCM2712's V3D interrupts (core GIC SPI 249, hub 250) have not run
 *     on the board yet, and the polled bin is what rpi5_triangle passed with
 *     (tools/cts/RESULTS.md, 'V3D 7.1: Raspberry Pi 5 on Zephyr'). Moving
 *     the bin to the interrupt is a board step of its own.
 *   - no job waits in a slot behind the one in flight: a second submit
 *     finishes the first.
 *   - the TFU is refused (-ENOTSUP, its queries unanswered): the 7.1 TFU is
 *     a block of its own (0x700, with IOC; v3d_tfu71.*) that this file does
 *     not drive yet.
 *   - the job builder's packet-skip counters are the 4.2 builder's
 *     (v3d_job.c), which a 7.1 image does not carry: osal_v3d_pack_counts
 *     says off and zero.
 * A stage that does not end is recorded, the GPU reset (v3d_reset: the SMS
 * powered off and on around pm's reset bit) and the job's fence completes
 * with the error; only a reset that fails takes the GPU as hung.
 *
 * Memory is Normal non-cacheable for the CPU (K_MEM_ARM_NORMAL_NC), as on
 * the Pi 4: the carve-out is a __noinit static whose cacheable alias is
 * written back and invalidated once at init.
 *
 * The host check is tools/v3d/check71.d/osal71_mock.sh: this file and
 * v3d_hw.c against a register model, the register sequence of init and of
 * one bin + render job pinned.
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

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_bounds.h"
#include "osal_log.h"
#include "v3d_kick.h"
#include "v3d.h"
#include "v3d_regs.h"
#include "v3d_frees.h"
#include "v3d_granules.h"
#include "zephyr_v3d.h"

/* The watchdog must be in the image: CONFIG_YAKOGL_GPU_V3D71 selects it. */
#ifndef CONFIG_YAKOGL_RPI5_WDOG
#error "zephyr_v3d71.c needs CONFIG_YAKOGL_RPI5_WDOG (the BCM2712 PM watchdog armed at boot)"
#endif

/* ---- the carve-out (as zephyr_v3d.c) ------------------------------------ */

#define V3D_OSAL_CARVEOUT_BYTES CONFIG_YAKOGL_V3D_CARVEOUT_BYTES
#define GRANULE_SHIFT 8u
#define GRANULE       (1u << GRANULE_SHIFT)
#define PAGE          4096u

/* VA 0 is never mapped, the carve-out starts at 1 MiB, windows follow it.
 * 512 MiB of VA is a 512 KiB page table at the front of the carve-out. */
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
/* one binner overflow block: 256 KiB, Linux's and Mesa's simulator's size
 * (docs/notes/v3d71-bin-submit.md section 4) */
#define BIN_OVERFLOW_BLOCK (256u * 1024u)
static uint32_t used_map[GRANULES / 32];

/* The PM watchdog's count register (rpi4/rpi_wdog.c, PM_WDOG; Linux's
 * bcm2835_wdt.c): the ticks left, 1/65536 s each, in the low 20 bits; 0 when
 * nothing armed it. pm is the block v3d_hw.c's power steps use too. */
#define PM_WDOG            0x24
#define PM_WDOG_TICKS_MASK 0x000fffffu

/* A stage's bounds (zephyr_v3d.c's): past JOB_SLOW_MS it is slow but stands,
 * past JOB_HANG_US it has not ended and the GPU is reset. */
#define JOB_SLOW_MS 1000u
#define JOB_HANG_US 4000000u

static struct {
	bool inited;
	struct v3d_regs regs;
	struct v3d_identity id;
	uint8_t *nc;            /* the carve-out's Normal non-cacheable CPU view */
	uintptr_t phys;         /* its physical address */
	uint32_t *pt;           /* the page table, at the front of the carve-out */
	uint32_t first_granule; /* past the table and the scratch page */
	uint32_t window_va;     /* the next VA for map_phys */
	uint32_t submitted, completed;
	int last_rc;            /* the last completed job's result */
	uint32_t prev_completed; /* the one before it and its result */
	int prev_rc;
	/* the job whose render runs: its fence, its stamps and its lists */
	bool inflight;
	uint32_t if_fence, if_c0, if_c_bin, if_c_render;
	uint32_t if_bcl_start, if_bcl_end, if_rcl_start, if_rcl_end;
	struct osal_v3d_bin_pool bin; /* after the last job that binned */
	uint32_t ov_blocks[MAX(CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS, 1)];
	struct v3d_bin_overflow ov;
	struct v3d_clock_ops clk;
	uint32_t wdog_ticks;    /* PM_WDOG at init */
} s;

static struct v3d_granules gm;

static uint32_t va_of(uint32_t off)
{
	return VA_BASE + off;
}

/* the GPU did not finish and the reset failed: nothing more is submitted
 * and nothing given back */
static bool hung;
static uint32_t hung_fence, hung_refused, hung_leaked_kib;
static uint32_t diag_mmu_faults, diag_slow, diag_slowest_ms, diag_failed, diag_recoveries, diag_bin_oom;

/* ---- lifecycle -------------------------------------------------------- */

/* The board's clock switch (driver/zephyr/lib/board/rpi5/rpi5_v3d.c, the
 * firmware's clock 5 through the mailbox) when init() gets no platform. */
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

	/* the watchdog first: armed at boot, before anything here touches
	 * the V3D (the file comment) */
	s.regs = (struct v3d_regs){ .soc = V3D_SOC_BCM2712 };
	device_map(&s.regs.pm, BCM2712_PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	s.wdog_ticks = sys_read32(s.regs.pm + PM_WDOG) & PM_WDOG_TICKS_MASK;
	if (s.wdog_ticks == 0) {
		printk("osal(v3d71): refused: the PM watchdog is not armed (PM_WDOG 0): no GPU access "
		       "without the reset that brings the board back (CONFIG_YAKOGL_RPI5_WDOG)\n");
		return -ENODEV;
	}
	printk("osal(v3d71): PM watchdog armed (%u ms left of its period)\n",
	       (unsigned int)(((uint64_t)s.wdog_ticks * 1000u) >> 16));

	if (!arg)
		arg = osal_zephyr_v3d_default_platform();
	if (!arg || !arg->clock_set) {
		printk("osal(v3d71): init needs struct osal_zephyr_v3d_init with the V3D clock switch\n");
		return -EINVAL;
	}
	s.clk.set = arg->clock_set;
	s.clk.ctx = arg->clock_ctx;

	device_map(&s.regs.hub, BCM2712_V3D_HUB_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&s.regs.core, BCM2712_V3D_CORE0_BASE, 0x6000, K_MEM_CACHE_NONE);
	device_map(&s.regs.sms, BCM2712_V3D_SMS_BASE, 0x700, K_MEM_CACHE_NONE);

	rc = v3d_power_on(&s.regs, &s.clk);
	if (rc) {
		printk("osal(v3d71): power-on failed (%d)\n", rc);
		return rc;
	}
	rc = v3d_identify(&s.regs, &s.id);
	printk("osal(v3d71): V3D %u.%u, %u QPUs, VPM %u bytes, PA %u / VA %u bits, MMU %s (identify %d)\n",
	       s.id.ver / 10, s.id.ver % 10, s.id.qpu_count, s.id.vpm_size, s.id.pa_width, s.id.va_width,
	       s.id.has_mmu ? "yes" : "no", rc);
	if (rc) {
		printk("osal(v3d71): refused: not a single-core V3D 7.1 with an MMU (HUB_IDENT1 0x%08x, "
		       "CTL_IDENT0 0x%08x)\n", s.id.hub_ident[1], s.id.ctl_ident[0]);
		return rc;
	}

	/* the carve-out: its cacheable alias written back and dropped once */
	s.phys = k_mem_phys_addr(carve);
	sys_cache_data_flush_and_invd_range(carve, sizeof(carve));
	k_mem_map_phys_bare(&s.nc, s.phys, sizeof(carve), K_MEM_PERM_RW | K_MEM_ARM_NORMAL_NC);
	if (!s.nc) {
		printk("osal(v3d71): cannot map the carve-out\n");
		return -ENOMEM;
	}

	/* the page table and the scratch page at the front, the rest mapped
	 * linearly from VA_BASE */
	s.pt = (uint32_t *)s.nc;
	memset(s.pt, 0, PT_BYTES);
	const uint32_t scratch_off = PT_BYTES;
	memset(s.nc + scratch_off, 0, PAGE);
	const uint32_t heap_off = scratch_off + PAGE;
	rc = v3d_pte_map(s.pt, PT_ENTRIES, va_of(heap_off), s.phys + heap_off,
			 (uint32_t)sizeof(carve) - heap_off, true);
	if (rc) {
		printk("osal(v3d71): the carve-out at phys 0x%llx does not fit the page table (%d)\n",
		       (unsigned long long)s.phys, rc);
		return rc;
	}
	s.first_granule = heap_off / GRANULE;
	s.window_va = ROUND_UP(va_of((uint32_t)sizeof(carve)), PAGE);
	v3d_granules_init(&gm, used_map, s.first_granule, GRANULES);
	v3d_granules_top_below(&gm, CONFIG_YAKOGL_V3D_CARVEOUT_TOP_BELOW / GRANULE);

	v3d_irq_quiesce(&s.regs); /* this OSAL polls (the file comment) */
	v3d_caches_init(&s.regs);
	rc = v3d_mmu_init(&s.regs, s.phys, s.phys + scratch_off);
	if (rc) {
		printk("osal(v3d71): MMU init failed (%d)\n", rc);
		return rc;
	}
	for (uint32_t i = 0; i < CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS; i++) {
		int64_t g = v3d_granules_find(&gm, BIN_OVERFLOW_BLOCK / GRANULE, PAGE / GRANULE);
		if (g < 0) {
			printk("osal(v3d71): the carve-out cannot hold %u binner overflow blocks of %u KiB\n",
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
	printk("osal(v3d71): ready: carve-out %u MiB at phys 0x%llx, GPU VA 0x%x..0x%x\n",
	       (unsigned int)(sizeof(carve) >> 20), (unsigned long long)s.phys, va_of(heap_off),
	       va_of((uint32_t)sizeof(carve)));
	return 0;
}

static void report_pending(const char *what);

static void z_shutdown(void)
{
	report_pending("");
	/* the V3D stays powered; the watchdog bounds the image's lifetime */
	s.inited = false;
}

/* ---- GPU memory + MMU --------------------------------------------------- */

static struct v3d_frees frees;

static void release_granules(const struct v3d_frees_run *r)
{
	v3d_granules_mark(&gm, r->granule, r->n, false);
}

static void report_pending(const char *what)
{
	char line[160];

	pf_format(&frees.q, line, sizeof(line));
	printk("osal(v3d71): %s%s\n", what, line);
}

#define V3D_PENDING_FREES_WAIT_US 1000000

static int z_wait(int64_t fence, int64_t timeout_us);
static int finish_inflight(bool poll);

static int wait_pending(uint32_t fence, uint32_t *completed)
{
	int rc = z_wait(fence, V3D_PENDING_FREES_WAIT_US);

	if (rc)
		return rc;
	*completed = MAX(s.completed, fence);
	return 0;
}

static void drain(void)
{
	if (hung)
		return;
	v3d_frees_drain(&frees, s.completed, release_granules);
}

static uint32_t poll_completed(void)
{
	if (s.inflight && !hung)
		(void)finish_inflight(true);
	return s.completed;
}

static uint32_t poll_and_drain(void)
{
	if (hung)
		return s.completed;
	return v3d_frees_poll(&frees, poll_completed, release_granules);
}

static uint32_t oom_refused_total;

static int z_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	(void)heap_base;
	(void)flags;
	if (!s.inited || !out || size == 0 || size > sizeof(carve))
		return -EINVAL;
	const uint32_t n = DIV_ROUND_UP(size, GRANULE);
	const uint32_t align = size >= PAGE ? PAGE / GRANULE : 1;

	if (frees.q.n)
		(void)poll_and_drain();
	int64_t g = v3d_granules_place(&gm, n, align);
	if (g < 0) {
		if (s.inflight)
			(void)finish_inflight(false);
		drain();
		g = v3d_granules_place(&gm, n, align);
	}
	if (g < 0) {
		if (oom_refused_total++ == 0)
			printk("osal(v3d71): out of GPU memory for %u bytes (%u frees held; later refusals "
			       "are counted)\n", (unsigned int)size, frees.q.n);
		return -ENOMEM;
	}
	v3d_granules_mark(&gm, (uint32_t)g, n, true);
	const uint32_t off = (uint32_t)g * GRANULE;
	out->cpu = s.nc + off;
	out->gpu = va_of(off);
	out->phys = s.phys + off;
	out->size = size;
	out->impl = (void *)(uintptr_t)out->gpu;
	return 0;
}

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
		hung_leaked_kib += (uint32_t)(mem->size / 1024);
		rc = -EIO;
	} else if (mem->impl && in_nc(mem->cpu)) {
		const uint32_t off = (uint32_t)((uint8_t *)mem->cpu - s.nc);
		const uint32_t completed = poll_and_drain();

		rc = v3d_frees_free(&frees, off / GRANULE, DIV_ROUND_UP(mem->size, GRANULE),
				    (uint32_t)DIV_ROUND_UP(mem->size, 1024), s.submitted, completed,
				    PF_WAIT(wait_pending), release_granules);
		if (rc && frees.q.refused == 1)
			report_pending("a free refused, memory kept (the first; the rest are counted): ");
	}
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return rc;
}

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
	const uint64_t base = ROUND_DOWN(phys, PAGE);
	const uint64_t span = ROUND_UP((phys - base) + size, PAGE);
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
		nwindows++;
		printk("osal(v3d71): more than %u map_phys windows: the later ones have no CPU view\n",
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

static void print_stall(const char *what, uint32_t fence, int rc, const struct v3d_stall *st)
{
	/* VIO_ADDR holds the top 32 bits of a va_width-bit address */
	const uint32_t w = s.id.va_width > 32 ? s.id.va_width - 32 : 0;

	printk("osal(v3d71): job %u %s did not end (%d): core_sts 0x%x hub_sts 0x%x ct0 ca 0x%x ra 0x%x "
	       "ct1 ca 0x%x ra 0x%x err 0x%x mmu_ctl 0x%x vio 0x%llx vio_id 0x%x; bcl 0x%x..0x%x "
	       "rcl 0x%x..0x%x\n",
	       fence, what, rc, st->core_int_sts, st->hub_int_sts, st->ct0ca, st->ct0ra, st->ct1ca, st->ct1ra,
	       st->err_stat, st->mmu_ctl, (unsigned long long)((uint64_t)st->mmu_vio_addr << w),
	       st->mmu_vio_id, s.if_bcl_start, s.if_bcl_end, s.if_rcl_start, s.if_rcl_end);
}

/* the last job's times, in cycles (osal_v3d_last_job_us) */
static struct {
	uint32_t prep, bin, render, clean, total, render_late;
} last_job_cyc;
static struct {
	uint32_t fence, prep, bin, render, clean, total, render_late;
} done_job_cyc;

#define RECENT_JOBS OSAL_V3D_RECENT_JOBS
static struct {
	uint32_t fence, prep, bin, render, clean, total, render_late, idle;
} recent_cyc[RECENT_JOBS];
static uint32_t recent_n, recent_prev_end;

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
	const uint32_t i = recent_n % RECENT_JOBS;
	recent_cyc[i].fence = fence;
	recent_cyc[i].prep = last_job_cyc.prep;
	recent_cyc[i].bin = last_job_cyc.bin;
	recent_cyc[i].render = last_job_cyc.render;
	recent_cyc[i].clean = last_job_cyc.clean;
	recent_cyc[i].total = last_job_cyc.total;
	recent_cyc[i].render_late = last_job_cyc.render_late;
	recent_cyc[i].idle = (recent_n == 0 || gap < 0) ? 0 : (uint32_t)gap;
	recent_n++;
	recent_prev_end = c_end;
}

static void complete(uint32_t fence, int rc)
{
	s.prev_completed = s.completed;
	s.prev_rc = s.last_rc;
	s.completed = fence;
	s.last_rc = rc;
	drain();
}

/* A stage that did not end: reset the GPU (v3d_reset, the BCM2712's power
 * cycle through the SMS and pm), or take it as hung when that fails. */
static void recover(uint32_t fence)
{
	int rc = v3d_reset(&s.regs, &s.clk, s.phys, s.phys + PT_BYTES);

	if (rc == 0) {
		diag_recoveries++;
		printk("osal(v3d71): job %u: GPU reset (recovery %u)\n", fence, diag_recoveries);
		return;
	}
	hung = true;
	hung_fence = fence;
	printk("osal(v3d71): job %u: the GPU reset failed (%d): the GPU is taken as hung; no later job is "
	       "submitted\n", fence, rc);
}

static void note_slow(uint32_t fence, const char *what, uint32_t cyc)
{
	const uint32_t ms = k_cyc_to_us_floor32(cyc) / 1000u;

	if (ms <= JOB_SLOW_MS)
		return;
	diag_slow++;
	if (ms > diag_slowest_ms)
		diag_slowest_ms = ms;
	printk("osal(v3d71): job %u %s took %u ms (slow; past %u ms)\n", fence, what, ms, JOB_SLOW_MS);
}

/* Is the render in flight over (FRDONE, a fault, or past its bound)? Reads
 * only: the done bit is left for the wait that consumes it. */
static bool inflight_ready(void)
{
	const uint32_t faults = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET |
				V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET | V3D71_HUB_CTL_INT_STS_INT_GMPV_SET;
	const uint32_t core = sys_read32(s.regs.core + V3D_CTL_0_INT_STS);
	const uint32_t hub = sys_read32(s.regs.hub + V3D_HUB_CTL_INT_STS);

	return (core & V3D_CTL_0_INT_STS_INT_FRDONE_SET) || (hub & faults) ||
	       k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_render) >= JOB_HANG_US;
}

/* Finish the job in flight: FRDONE waited for (bounded by JOB_HANG_US from
 * the render's start; with `poll`, only looked at), the TMU caches cleaned,
 * the fence completed. 0 when it completed (or none was in flight), 1 when
 * `poll` found it running, the job's error when it failed (GPU reset). */
static int finish_inflight(bool poll)
{
	struct v3d_stall st;

	if (!s.inflight)
		return 0;
	if (poll && !inflight_ready())
		return 1;
	const uint32_t fence = s.if_fence;
	last_job_cyc.render_late =
		(sys_read32(s.regs.core + V3D_CTL_0_INT_STS) & V3D_CTL_0_INT_STS_INT_FRDONE_SET) ? 1u : 0u;
	const uint32_t elapsed_us = k_cyc_to_us_floor32(k_cycle_get_32() - s.if_c_render);
	int rc = v3d_wait(&s.regs, V3D_CTL_0_INT_STS_INT_FRDONE_SET,
			  elapsed_us >= JOB_HANG_US ? 0 : JOB_HANG_US - elapsed_us, &st);
	const uint32_t c_frdone = k_cycle_get_32();

	s.inflight = false;
	last_job_cyc.render = c_frdone - s.if_c_render;
	if (rc) {
		diag_failed++;
		if (rc == -EFAULT)
			diag_mmu_faults++;
		print_stall("render", fence, rc, &st);
		recover(fence);
	} else {
		note_slow(fence, "render", last_job_cyc.render);
		rc = v3d_caches_clean(&s.regs);
		last_job_cyc.clean = k_cycle_get_32() - c_frdone;
		if (rc)
			printk("osal(v3d71): job %u: the TMU cache clean did not end (%d)\n", fence, rc);
	}
	const uint32_t c_end = k_cycle_get_32();
	complete(fence, rc);
	last_job_cyc.total = c_end - s.if_c0;
	note_job_done(fence, c_end);
	return rc;
}

static int64_t z_submit(const void *kick_desc, size_t len)
{
	const struct v3d_kick *k = kick_desc;
	struct v3d_stall st;

	if (!s.inited || !k || len < sizeof(*k))
		return -EINVAL;
	if (k->magic != V3D_KICK_MAGIC) {
		printk("osal(v3d71): kick magic 0x%08x is not a V3D kick\n", k->magic);
		return -EINVAL;
	}
	uint8_t *ts = cpu_of(k->tile_state, k->tile_state_size);
	if (!ts || k->bcl_end <= k->bcl_start || k->rcl_end <= k->rcl_start) {
		printk("osal(v3d71): a kick with bad lists or tile state\n");
		return -EINVAL;
	}
	/* one job on the GPU: the binner and the tile memory are the running
	 * job's until its render is over */
	if (s.inflight)
		(void)finish_inflight(false);

	const uint32_t c0 = k_cycle_get_32();
	const uint32_t fence = ++s.submitted;
	if (hung) {
		hung_refused++;
		complete(fence, -EIO);
		return (int64_t)fence;
	}
	last_job_cyc.prep = last_job_cyc.bin = last_job_cyc.render = last_job_cyc.clean = 0;
	last_job_cyc.total = last_job_cyc.render_late = 0;
	/* the tile state array starts zeroed (Mesa allocates it fresh) */
	memset(ts, 0, k->tile_state_size);
	s.if_fence = fence;
	s.if_c0 = c0;
	s.if_bcl_start = k->bcl_start;
	s.if_bcl_end = k->bcl_end;
	s.if_rcl_start = k->rcl_start;
	s.if_rcl_end = k->rcl_end;
	s.ov.used = 0;
	barrier_dmem_fence_full();

	s.if_c_bin = k_cycle_get_32();
	last_job_cyc.prep = s.if_c_bin - c0;
	v3d_submit_bin(&s.regs, k->bcl_start, k->bcl_end, k->tile_alloc, k->tile_alloc_size, k->tile_state);
	int rc = v3d_wait_bin(&s.regs, JOB_HANG_US, &st, &s.ov);
	const uint32_t c_fldone = k_cycle_get_32();

	last_job_cyc.bin = c_fldone - s.if_c_bin;
	if (rc) {
		diag_failed++;
		if (rc == -ENOMEM)
			diag_bin_oom++;
		else if (rc == -EFAULT)
			diag_mmu_faults++;
		print_stall("bin", fence, rc, &st);
		recover(fence);
		complete(fence, rc);
		last_job_cyc.total = k_cycle_get_32() - c0;
		note_job_done(fence, k_cycle_get_32());
		return (int64_t)fence;
	}
	note_slow(fence, "bin", last_job_cyc.bin);
	s.bin = (struct osal_v3d_bin_pool){
		.jobs = s.bin.jobs + 1,
		.pool = k->tile_alloc,
		.pool_size = k->tile_alloc_size,
		.bpca = sys_read32(s.regs.core + V3D_PTB_0_BPCA),
		.bpcs = sys_read32(s.regs.core + V3D_PTB_0_BPCS),
		.qma = sys_read32(s.regs.core + V3D_CLE_0_CT0QMA),
		.qms = sys_read32(s.regs.core + V3D_CLE_0_CT0QMS),
		.overflow_used = s.ov.used,
		.overflow_blocks = s.ov.nblocks,
		.overflow_block_size = s.ov.block_size,
	};
	s.if_c_render = k_cycle_get_32();
	v3d_submit_render(&s.regs, k->rcl_start, k->rcl_end);
	s.inflight = true;
	return (int64_t)fence;
}

static int64_t z_completed(void)
{
	if (!s.inited)
		return 0;
	if (s.inflight && inflight_ready())
		(void)finish_inflight(false);
	return (int64_t)s.completed;
}

/* 0 when the fence completed and its job succeeded, 1 when a poll
 * (timeout_us == 0) found it in flight, <0 the job's error. As zephyr_v3d.c,
 * a timeout_us other than 0 is bounded by JOB_HANG_US from the render's
 * start, whatever its value. */
static int z_wait(int64_t fence, int64_t timeout_us)
{
	if (!s.inited || fence <= 0 || fence > s.submitted)
		return -EINVAL;
	if (s.inflight && (uint32_t)fence == s.if_fence) {
		const int rc = finish_inflight(timeout_us == 0);

		if (rc == 1)
			return 1;
	}
	if ((uint32_t)fence == s.completed)
		return s.last_rc;
	return (uint32_t)fence == s.prev_completed ? s.prev_rc : 0;
}

/* ---- queries -------------------------------------------------------------- */

BUILD_ASSERT(sizeof(struct osal_v3d_cpu_of) == 24, "struct osal_v3d_cpu_of");
BUILD_ASSERT(sizeof(struct osal_v3d_bin_pool) == 40, "struct osal_v3d_bin_pool");

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
			drain();
		return 0;
	}
	if (what == OSAL_QUERY_V3D_SPLIT_ZS && out && out_sz == sizeof(uint32_t)) {
#ifdef CONFIG_YAKOGL_V3D_SPLIT_ZS
		*(uint32_t *)out = 1u;
#else
		*(uint32_t *)out = 0u;
#endif
		return 0;
	}
	if (what == OSAL_QUERY_V3D_S8_STENCIL && out && out_sz == sizeof(uint32_t)) {
#ifdef CONFIG_YAKOGL_V3D_S8_STENCIL
		*(uint32_t *)out = 1u;
#else
		*(uint32_t *)out = 0u;
#endif
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
	/* OSAL_QUERY_V3D_TFU_*: not answered, the 7.1 TFU is not driven here
	 * (the file comment) */
	if (what == OSAL_QUERY_CLOCK_NS || what == OSAL_QUERY_DRAW_STATS)
		return osal_zephyr_timing_query(what, out, out_sz);
	return -ENOTSUP;
}

/* ---- instruments (zephyr_v3d.h, osal_internal.h) ------------------------- */

int osal_v3d_tfu(const struct v3d_tfu_job *j, struct v3d_tfu_status *st)
{
	ARG_UNUSED(j);
	if (st)
		*st = (struct v3d_tfu_status){ 0 };
	return s.inited ? -ENOTSUP : -ENODEV;
}

uint32_t osal_v3d_bin_oom_jobs(void)
{
	return diag_bin_oom;
}

uint32_t osal_v3d_recoveries(void)
{
	return diag_recoveries;
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

unsigned osal_v3d_recent_jobs_us(struct osal_v3d_recent_job_us *out, unsigned max)
{
	if (!out)
		return 0;
	unsigned n = recent_n < RECENT_JOBS ? recent_n : RECENT_JOBS;
	if (n > max)
		n = max;
	for (unsigned i = 0; i < n; i++) {
		const unsigned r = (recent_n - n + i) % RECENT_JOBS;

		out[i] = (struct osal_v3d_recent_job_us){
			.fence = recent_cyc[r].fence,
			.prep = k_cyc_to_us_floor32(recent_cyc[r].prep),
			.bin = k_cyc_to_us_floor32(recent_cyc[r].bin),
			.render = k_cyc_to_us_floor32(recent_cyc[r].render),
			.clean = k_cyc_to_us_floor32(recent_cyc[r].clean),
			.total = k_cyc_to_us_floor32(recent_cyc[r].total),
			.render_late = recent_cyc[r].render_late,
			.idle = k_cyc_to_us_floor32(recent_cyc[r].idle),
			.tfu_jobs = 0, /* no TFU on this OSAL yet */
			.tfu = 0,
		};
	}
	return n;
}

void osal_v3d_mem_used(uint32_t *used, uint32_t *high, uint32_t *total)
{
	const uint32_t heap = s.inited ? GRANULES - s.first_granule : 0;

	if (used)
		*used = gm.used * GRANULE;
	if (high)
		*high = gm.high * GRANULE;
	if (total)
		*total = heap * GRANULE;
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

void osal_v3d_find_counts(uint32_t *calls, uint64_t *reads, uint32_t *reads_max)
{
	if (calls)
		*calls = gm.finds;
	if (reads)
		*reads = gm.reads;
	if (reads_max)
		*reads_max = gm.reads_max;
}

/* The 4.2 job builder's packet skip is not in a 7.1 image: off, nothing. */
void osal_v3d_pack_counts(bool *on, uint32_t *jobs, uint32_t *emitted, uint32_t *skipped,
			  uint32_t *compared, uint32_t *mismatched)
{
	if (on)
		*on = false;
	if (jobs)
		*jobs = 0;
	if (emitted)
		*emitted = 0;
	if (skipped)
		*skipped = 0;
	if (compared)
		*compared = 0;
	if (mismatched)
		*mismatched = 0;
}

void osal_mem_line(char *buf, size_t n)
{
	uint32_t used, high, total, held, held_bytes, longest, runs;

	osal_v3d_mem_used(&used, &high, &total);
	osal_v3d_frees_held(&held, &held_bytes);
	osal_v3d_mem_free_runs(&longest, &runs, NULL);
	snprintk(buf, n,
		 "osal(v3d71) mem: carve-out used %u, high-water %u, of %u bytes; frees held %u (%u bytes); "
		 "longest free run %u bytes (%u runs)",
		 used, high, total, held, held_bytes, longest, runs);
}

void osal_v3d_diag_print(void)
{
	uint32_t used, high, total;

	printk("osal(v3d71): jobs %u submitted, %u completed (last rc %d); slow %u (the slowest %u ms); "
	       "failed %u; MMU faults %u; recoveries %u; binner out of memory %u (%u overflow blocks of "
	       "%u KiB); hung: ",
	       s.submitted, s.completed, s.last_rc, diag_slow, diag_slowest_ms, diag_failed, diag_mmu_faults,
	       diag_recoveries, diag_bin_oom, s.ov.nblocks, BIN_OVERFLOW_BLOCK / 1024);
	if (hung)
		printk("job %u (the GPU reset failed), %u later jobs not submitted, %u KiB not given back\n",
		       hung_fence, hung_refused, hung_leaked_kib);
	else
		printk("none\n");
	osal_v3d_mem_used(&used, &high, &total);
	printk("osal(v3d71): carve-out %u bytes used now, %u at most, of %u; %u frees held; %u allocations "
	       "refused for lack of GPU memory\n", used, high, total, frees.q.n, oom_refused_total);
	report_pending("");
	{
		char line[96];

		yakogl_log_repeats_report(line, sizeof(line));
		printk("%s\n", line);
	}
}

/* ---- diagnostics ---------------------------------------------------------- */

static void z_log(const char *fmt, ...)
{
	struct osal_log_line line;
	const char *text;
	va_list ap;

	va_start(ap, fmt);
	text = osal_log_format(&line, osal_vsnprintk, fmt, ap);
	va_end(ap);
	printk("%s\n", text);
}

static uint32_t z_thread_id(void)
{
	const uintptr_t t = (uintptr_t)k_current_get();

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
	.present     = yakogl_zephyr_present,
};

void osal_zephyr_register(void)
{
	osal_set_ops(&osal_zephyr_ops);
}
