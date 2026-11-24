/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the NanoPC-T6 (yakogl_board.h;
 * driver/docs/nanopc-t6-display.md, step D3):
 *   window   the last pages of the Mali OSAL's carve-out
 *            (osal_mali_board_window): the GPU's address space maps them
 *            with the rest and no allocation returns them. Uncached, so
 *            neither the GPU nor a check needs cache maintenance. The OSAL
 *            fixes the carve-out's layout when EGL starts it, so the window
 *            is taken before eglInitialize()
 *   screen   VP0 of the VOP on HDMI0 at 60 Hz, the window at the screen's
 *            origin and the rest of a larger screen black
 *            (yd_t6_screen_show). The first board_show() reads what was
 *            left. A picture that runs (the crash route: 1024x768 or
 *            1920x1080 as Linux left it) is adopted. No picture (a normal
 *            kexec or U-Boot: PD_VOP off, its clocks gated or VP0 in
 *            STANDBY) starts the display from cold, in the build's mode
 *            (CONFIG_YAKOGL_BOARD_NANOPC_T6_COLD_1920X1080), each line of
 *            the chain on the UART at once. Either way the first show's
 *            record is printed. A later show of another buffer is a flip,
 *            which never waits. Nothing is copied. A first show that
 *            stopped is final. After the run's hold the section 5
 *            read-backs (read_back below): the picture still runs and a
 *            display is connected
 *   record   the yk-record line on the UART (common/record.c). With
 *            CONFIG_YAKOGL_BOARD_RECORD_SCREEN the console's last lines and
 *            that line are also drawn over the window on the screen when the
 *            run is over (record_screen below): a run judged from the
 *            capture unit's pictures alone says its result there
 *   watchdog WDT0 (nanopc_t6.c), fed while board_wdog_alive() says so
 *   reset    t6_reset_to_linux(): PSCI SYSTEM_RESET, back into Armbian
 */
#include <zephyr/fatal.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "nanopc_t6.h"
#include "osal_internal.h"

/* The display path's register blocks (YD_T6_DISPLAY_*_PHYS, yakogl_drivers.h),
 * as the t6_cold probe maps them: the adopt's seven, the CRU with the PMU1
 * CRU at +0x30000 and the HDMI controller up to its interrupt words for the
 * chain from cold, and the chain's own three (vop_grf, vo1_grf, the PHY). */
#define VOP_MAP  0x2000
#define CRU_MAP  0x31000
#define HDMI_MAP 0x5000
#define PHY_MAP  0x2000

/* The cold start's mode (yd_t6_screen_show's `mode`), at 60 Hz. */
#ifdef CONFIG_YAKOGL_BOARD_NANOPC_T6_COLD_1920X1080
#define COLD_MODE      YD_T6_DISPLAY_COLD_MODE_1920X1080
#define COLD_MODE_NAME "1920x1080"
#else
#define COLD_MODE      0u
#define COLD_MODE_NAME "1024x768"
#endif

static struct yd_t6_display_cold_bases s_bases;
static struct yd_env s_env;
static struct yd_t6_screen s_screen;
static bool s_mapped;
/* The window board_window() handed out and the last one a show put on the
 * screen. A timer handler reads them (t6_record_screen): they change with
 * interrupts off. */
static struct board_window s_window, s_shown;

static void window_set(struct board_window *to, const struct board_window *win)
{
	unsigned int key = irq_lock();

	*to = *win;
	irq_unlock(key);
}

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	uint64_t phys;
	void *cpu;
	int rc;

	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || h > SIZE_MAX / 4u / w) {
		return -EINVAL;
	}
	rc = osal_mali_board_window((size_t)w * h * 4u, &phys, &cpu);
	if (rc != 0) {
		return rc;
	}
	*out = (struct board_window){
		.phys = phys,
		.cpu = cpu,
		.w = w,
		.h = h,
		.stride = w * 4u,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	window_set(&s_window, out);
	return 0;
}

/* The drivers crate's lines (the chain from cold), on the UART at once and
 * as they are: a line printed before a hang must not wait in a log buffer. */
static void cold_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	ARG_UNUSED(ctx);
	ARG_UNUSED(level);
	printk("%.*s\n", (int)len, (const char *)buf);
}

static void map_bases(void)
{
	mm_reg_t vop, mmu, pmu, cru, sys_grf, phy_grf, hdmi, vop_grf, vo1_grf, phy;

	device_map(&vop, YD_T6_DISPLAY_VOP_PHYS, VOP_MAP, K_MEM_CACHE_NONE);
	device_map(&mmu, YD_T6_DISPLAY_VOP_MMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, YD_T6_DISPLAY_PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&cru, YD_T6_DISPLAY_CRU_PHYS, CRU_MAP, K_MEM_CACHE_NONE);
	device_map(&sys_grf, YD_T6_DISPLAY_SYS_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy_grf, YD_T6_DISPLAY_HDPTX0_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&hdmi, YD_T6_DISPLAY_HDMI0_PHYS, HDMI_MAP, K_MEM_CACHE_NONE);
	device_map(&vop_grf, YD_T6_DISPLAY_VOP_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&vo1_grf, YD_T6_DISPLAY_VO1_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy, YD_T6_DISPLAY_HDPTX0_PHYS, PHY_MAP, K_MEM_CACHE_NONE);
	s_bases = (struct yd_t6_display_cold_bases){
		.display = {
			.vop = vop,
			.vop_mmu = {mmu + 0xe00, mmu + 0xf00},
			.pmu = pmu,
			.cru = cru,
			.sys_grf = sys_grf,
			.phy_grf = phy_grf,
			.hdmi = hdmi,
		},
		.vop_grf = vop_grf,
		.vo1_grf = vo1_grf,
		.phy = phy,
	};
	yd_env_init(&s_env, NULL);
	s_env.log = cold_log;
	s_mapped = true;
}

/* The record of the show that left YD_T6_SCREEN_NEW: what the adopt found
 * and did, which of the two routes ran, and for the cold start where the
 * chain ended. `rc` is the show's and `ms` its time; on the cold route the
 * adopt had stopped (-ENODEV) and the time is the chain's but for the
 * adopt's few reads before its stop. */
static void print_first_show(int rc, long long ms, const struct yd_t6_display_adopt *adopt,
			     const struct yd_t6_screen_start *start)
{
	bool cold = start->route == YD_T6_SCREEN_ROUTE_COLD;

	t6_display_print_adopt("board", cold ? -ENODEV : rc, adopt);
	if (!cold) {
		printk("board: first show: a picture was left running: adopt, rc %d\n", rc);
		return;
	}
	printk("board: first show: no picture was left running: cold start at %s, rc %d\n",
	       COLD_MODE_NAME, rc);
	t6_display_print_cold("board", rc, ms, &start->cold);
}

static int screen_show(const struct board_window *win)
{
	struct yd_t6_display_adopt adopt;
	struct yd_t6_screen_start start = {0};
	int64_t t0;
	int rc;

	/* the VOP's window is set up for XRGB8888, top row first */
	if (win->format != GLES_NATIVE_FORMAT_XRGB8888) {
		return -ENOTSUP;
	}
	if (!s_mapped) {
		map_bases();
	}
	t0 = k_uptime_get();
	rc = yd_t6_screen_show(&s_bases, &s_env, COLD_MODE, &s_screen, win->phys, win->w, win->h,
			       win->stride, &adopt, &start);
	if (start.route != YD_T6_SCREEN_ROUTE_NONE) {
		print_first_show(rc, (long long)(k_uptime_get() - t0), &adopt, &start);
	}
	if (rc == 0) {
		window_set(&s_shown, win);
	}
	return rc;
}

int board_show(const struct board_window *win)
{
	board_first_frame();
	return screen_show(win);
}

#ifdef CONFIG_YAKOGL_BOARD_DISPLAY_BUSY
/* A flip above does not wait for the VOP to take the address, so the display
 * may still scan an earlier buffer: the screen keeps which. Before the first
 * show nothing is mapped and no buffer is busy. */
uint32_t board_display_busy(const uint64_t *phys, size_t n)
{
	if (!s_mapped) {
		return 0;
	}
	return yd_t6_screen_busy(&s_bases.display, &s_screen, phys, n);
}
#endif

int board_selector(void)
{
	return -1;
}

void board_sink_store(const uint32_t w[4])
{
	ARG_UNUSED(w); /* the UART line is the sink */
}

#ifdef CONFIG_YAKOGL_BOARD_RECORD_SCREEN
BUILD_ASSERT(!IS_ENABLED(CONFIG_YAKOGL_SAMPLE_REBOOT),
	     "the fatal error handler below is this board's: it halts for WDT0");

/* The record screen (common/record_screen.c) over the picture on the screen.
 * It is drawn into the window shown last, which the VOP scans (a flip the
 * VOP has not taken yet takes effect at its next frame by itself): no
 * register is written, so a timer handler and the fatal error handler call
 * it too. With `show` and nothing shown yet (a run that failed before its
 * first frame) the window board_window() handed out is drawn and shown,
 * which is the first show (the adopt, or the start from cold): thread
 * context. Returns whether the record is on the screen, and prints why when
 * it is not. */
static bool record_screen(bool show)
{
	unsigned int key = irq_lock();
	bool shown = s_screen.state == YD_T6_SCREEN_SHOWN;
	bool fresh = s_screen.state == YD_T6_SCREEN_NEW;
	struct board_window win = shown ? s_shown : s_window;
	const char *why = NULL;
	int rc = 0;

	irq_unlock(key);
	if (!shown && !(show && fresh)) {
		why = fresh ? "nothing is shown" : "the screen stopped";
	} else if (win.cpu == NULL) {
		why = shown ? "the window shown has no CPU address" : "no window was taken";
	} else {
		rc = board_record_screen_draw(win.cpu, win.w, win.h, win.stride);
		if (rc <= 0) {
			why = "the window holds no three rows of text";
		} else if (!shown) {
			rc = screen_show(&win);
			why = rc != 0 ? "the show failed" : NULL;
		}
	}
	if (why != NULL) {
		printk("board: no record screen: %s (rc %d)\n", why, rc);
	}
	return why == NULL;
}

/* The watchdog's feeder stopped (nanopc_t6.c): the image hangs or its
 * lifetime is over, and the record stays until WDT0's reset. The sample's
 * last lines (board_last_lines) go under the feeder's line. Timer context. */
void t6_record_screen(void)
{
	unsigned int key = irq_lock();

	board_last_lines();
	irq_unlock(key);
	(void)record_screen(false);
}

/* A fatal error. Zephyr printed its registers, which are in the ring; the
 * sample's last lines (board_last_lines: the glmark2 list so far) go under
 * them, so a run read from the screen alone keeps what it did before the
 * error. Then the record on the screen, and the halt with interrupts off as
 * the default handler does. The feeder stops with them and WDT0 resets the
 * SoC within one period: the time the record stays. */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	ARG_UNUSED(esf);
	printk("board: fatal error %u: halted, WDT0 resets the SoC within %u ms\n", reason,
	       t6_watchdog_period_ms());
	board_last_lines();
	(void)record_screen(false);
	k_fatal_halt(reason);
}
#endif

/* The end of a run, after board_finish()'s hold: the section 5 read-backs
 * over the picture that is still up (t6_display_read_back, the lines of the
 * display probes under the tag "board"). Its HPD line is read the hold's
 * length after the first show, so it proves a connected display also where
 * the chain's own line could not: a display powered from the board's USB
 * ports, which the start switched on only just before the chain ran (the
 * U-Boot route, driver/docs/nanopc-t6-display.md, section 3). The frame check
 * clears VP0's frame start flag: no show follows. */
static void read_back(void)
{
	struct yd_t6_display_status s;

	if (s_screen.state != YD_T6_SCREEN_SHOWN) {
		printk("board: no read-backs after the hold: nothing is shown\n");
		return;
	}
	(void)t6_display_read_back("board", &s_bases.display, &s);
}

FUNC_NORETURN void board_reset(void)
{
	/* A single-buffered sample shows one buffer and never flips; the count
	 * says whether a double-buffered run reached the flip and the busy rule. */
	printk("board: screen state %u, %u flips\n", s_screen.state, s_screen.flips);
	read_back();
#ifdef CONFIG_YAKOGL_BOARD_RECORD_SCREEN
	/* The reset is a call, not a watchdog period: the record gets a hold of
	 * its own, long enough for the capture unit (Kconfig). */
	if (record_screen(true)) {
		printk("board: the record screen stays %d s\n", CONFIG_YAKOGL_BOARD_RECORD_SCREEN_S);
		for (int i = 0; i < CONFIG_YAKOGL_BOARD_RECORD_SCREEN_S; i++) {
			board_watchdog_feed();
			k_sleep(K_SECONDS(1));
		}
	}
#endif
	t6_reset_to_linux("board: run over");
}
