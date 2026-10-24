/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer's common part (yakogl_board.h): "still making progress",
 * the GPU reset count and the end of a run, the same steps on every board.
 * What differs (the sink, the reset) is board_internal.h's per-board part.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include "yakogl_board.h"
#include "yakogl_record.h"
#include "board_internal.h"
#include "sample_glue.h"

#ifdef CONFIG_YAKOGL_SHADERC_ONLINE
int shc_online_keep_stats(size_t *in_use, size_t *peak); /* gles_shaderc.h */
#endif
#ifdef CONFIG_YAKOGL
uint32_t yakogl_draws_refused(void); /* src/backend.rs */
#endif

/*
 * The OSAL's "show this buffer" (gles_osal.h, present; src/osal/zephyr_timing.c
 * declares the weak default): a window surface's finished colour buffer,
 * shown the way board_show() shows a sample's window. With it EGL
 * double-buffers its window surfaces (EGL_BACK_BUFFER) and no sample has to
 * show anything itself.
 */
int yakogl_zephyr_present(uintptr_t phys, void *cpu, uint32_t width, uint32_t height,
			  uint32_t stride_bytes, uint32_t format)
{
	const struct board_window win = {
		.phys = phys, .cpu = cpu, .w = width, .h = height, .stride = stride_bytes,
		.format = format,
	};

	return board_show(&win);
}

#ifdef CONFIG_YAKOGL_BOARD_DISPLAY_BUSY
/*
 * The OSAL's "which of these buffers may the display still scan"
 * (gles_osal.h, display_busy; src/osal/zephyr.c takes it when it is linked):
 * board_display_busy(). Only on a board whose show does not wait; elsewhere
 * the symbol is absent, the op is NULL and EGL keeps its ring of three.
 */
uint32_t yakogl_zephyr_display_busy(const uint64_t *phys, size_t n)
{
	return board_display_busy(phys, n);
}
#endif

/* Uptime in ms of the last board_watchdog_feed(); -1 until the first call.
 * One 64-bit store, read by the feeders' timer handlers. */
static volatile int64_t s_alive_ms = -1;
static volatile bool s_released;

void board_watchdog_feed(void)
{
	s_alive_ms = k_uptime_get();
}

bool board_wdog_alive(void)
{
	int64_t at = s_alive_ms;

	if (s_released) {
		return false;
	}
	return at < 0 || k_uptime_get() - at <= (int64_t)CONFIG_YAKOGL_BOARD_ALIVE_S * MSEC_PER_SEC;
}

void board_wdog_release(void)
{
	s_released = true;
}

#ifdef CONFIG_YAKOGL_BOARD_BOOT_TIME
void board_boot_time(const char *where)
{
	/* The architected counter, read directly: k_uptime starts at this
	 * image's own boot, the counter at the boot loader's start of it. */
	uint64_t cnt, frq;

	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(cnt));
	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(frq));
	printk("boot time: system counter at %s: %llu us (%llu ticks at %llu Hz)\n", where,
	       (unsigned long long)(frq >= 1000000u ? cnt / (frq / 1000000u) : 0u),
	       (unsigned long long)cnt, (unsigned long long)frq);
}

static int board_boot_time_init(void)
{
	board_boot_time("application init");
	return 0;
}

SYS_INIT(board_boot_time_init, APPLICATION, 0);

void board_first_frame(void)
{
	static bool marked;

	if (!marked) {
		marked = true;
		board_boot_time("the first frame");
	}
}
#endif

static uint32_t s_show_copies;

void board_show_copied(uint32_t w, uint32_t h, const char *why)
{
	if (s_show_copies++ == 0u) {
		printk("board: board_show copies the %ux%u window to the screen (%s); "
		       "every copy is counted\n", w, h, why);
	}
}

uint32_t board_show_copies(void)
{
	return s_show_copies;
}

#ifdef CONFIG_YAKOGL_DRV_GPU_VIVANTE
/* The Vivante driver's recovery count (src/vivante/gc_render.h). */
uint32_t gc_render_resets(void);
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
/* The PowerVR firmware's recoveries since the render service started (its
 * HWR counter), and the render service's stop (driver/src/pvr_render.h). */
uint32_t pvr_render_hwr_recoveries(void);
typedef void (*pvr_render_stop_fn)(uint32_t cause, uint32_t fence, uint32_t recoveries,
				   uint32_t fl_id, uint32_t kib);
void pvr_render_set_stop_handler(pvr_render_stop_fn fn);
#endif

uint32_t board_gpu_resets(void)
{
#ifdef CONFIG_YAKOGL_DRV_GPU_VIVANTE
	return gc_render_resets();
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
	/* The firmware recovers a hung data master itself (HWR) and counts
	 * it; a run that recovered is never a clean pass. */
	return pvr_render_hwr_recoveries();
#else
	/* The V3D driver does not recover a wedged GPU: a hang ends the run
	 * at the watchdog instead. */
	return 0;
#endif
}

#if defined(CONFIG_YAKOGL) && defined(CONFIG_YAKOGL_GPU_POWERVR)
/* The PowerVR OSAL's run-end record (src/osal/zephyr.c): the pending-free
 * queue, the render pool's summary and the depth/stencil line. */
void osal_zephyr_diag_print(void);
#endif

/* The hold and the reset after a run's verdict. */
static FUNC_NORETURN void board_hold_and_reset(int hold)
{
	/* The display's counters around the hold, and the diagnosis knobs of
	 * the AM62P (the known-colour fill, the register dump): sample_glue.c,
	 * the same lines pvr_sample_finish prints; nothing on the other boards. */
	pvr_sample_before_hold();
	printk("board: holding %d s, then the reset\n", hold);
	/* The watchdog feeders are timers: they feed through the hold, up to
	 * CONFIG_YAKOGL_BOARD_LIFETIME_S, while this says it is alive. */
	for (int s = 0; s < hold; s++) {
		board_watchdog_feed();
		k_sleep(K_SECONDS(1));
	}
	pvr_sample_after_hold();
	board_reset();
}

#ifdef CONFIG_YAKOGL_DRV_GPU_PVR
/* The PowerVR render service stopped taking kicks (pvr_render.h): the run
 * ends here, with the GPU_STOP record as its verdict. Called inside the
 * sample's GL call, so the OSAL's run-end lines (the GL side's owners, the
 * backend's) are not printed; the driver printed its own before this. */
static void board_pvr_stopped(uint32_t cause, uint32_t fence, uint32_t recoveries, uint32_t fl_id,
			      uint32_t kib)
{
	printk("board: FAIL the GPU driver stopped the run (cause %u, fence %u, list %u, %u KiB, GPU recoveries %u); the GL side's run-end lines are not printed\n",
	       cause, fence, fl_id, kib, recoveries);
	board_result_tag(YK_RECORD_KIND_GPU_STOP, (uint8_t)cause, fence, recoveries,
			 (fl_id << 24) | (kib > 0xffffffu ? 0xffffffu : kib));
	board_hold_and_reset(CONFIG_YAKOGL_BOARD_HOLD_FAIL_S);
}

static int board_pvr_stop_init(void)
{
	pvr_render_set_stop_handler(board_pvr_stopped);
	return 0;
}

SYS_INIT(board_pvr_stop_init, APPLICATION, 0);
#endif

__weak void board_finish_record(bool pass)
{
	ARG_UNUSED(pass);
}

FUNC_NORETURN void board_finish(const char *name, int rc)
{
	uint32_t resets = board_gpu_resets();

#if defined(CONFIG_YAKOGL) && defined(CONFIG_YAKOGL_GPU_POWERVR)
	/* every run that ends here prints the pool's state (glmark2's terrain,
	 * docs/notes/pvr-render-pool.md); gl_test ends
	 * elsewhere and prints its own */
	osal_zephyr_diag_print();
#endif
#ifdef CONFIG_YAKOGL_SHADERC_ONLINE
	/* every arena prints its high-water every run (CLAUDE.md): the online
	 * compiler's keep region, which refuses a compile when it is full */
	{
		size_t keep_now = 0, keep_peak = 0;

		(void)shc_online_keep_stats(&keep_now, &keep_peak);
		printk("shaderc: keep high-water %zu of %u bytes (%zu held now)\n", keep_peak,
		       (unsigned int)CONFIG_YAKOGL_SHADERC_KEEP_BYTES, keep_now);
	}
#endif
#ifdef CONFIG_YAKOGL
	printk("yakogl: draws refused %u\n", yakogl_draws_refused());
#endif
	bool pass = rc == 0 && resets == 0u;
	int hold = pass ? CONFIG_YAKOGL_BOARD_HOLD_S : CONFIG_YAKOGL_BOARD_HOLD_FAIL_S;

	if (name == NULL) {
		name = "sample";
	}
	if (pass) {
		printk("%s: PASS\n", name);
	} else if (rc == 0) {
		printk("%s: FAIL the GPU was reset %u time(s): a recovered hang, not a clean run\n",
		       name, resets);
	} else {
		printk("%s: FAIL rc=%d\n", name, rc);
	}
	/* The sample's own result record is its verdict, with the detail; the
	 * APP record is written when there is none, and over it when the GPU
	 * was reset (a reset is never a clean pass). */
	if (!board_record_have_result() || resets != 0u) {
		board_result(YK_RECORD_KIND_APP,
			     (pass ? YK_RECORD_STATUS_PASS : YK_RECORD_STATUS_FAIL) |
				     (s_show_copies != 0u ? YK_RECORD_APP_SHOWN_BY_COPY : 0u),
			     0u, resets);
	}
	board_finish_record(pass);
	board_hold_and_reset(hold);
}
