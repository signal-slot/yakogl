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
 *   screen   VP0 of the VOP on HDMI0, 1024x768 or 1920x1080 at 60 Hz as
 *            Linux left it: the first board_show() adopts it with the window
 *            at the screen's origin, the rest of a larger screen black
 *            (yd_t6_screen_show), and prints the adopt's record; a
 *            later show of another buffer is a flip, which never waits.
 *            Nothing is copied. An adopt that stopped is final
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

#include "yakogl_board.h"
#include "board_internal.h"
#include "nanopc_t6.h"
#include "osal_internal.h"

/* The display path's register blocks (rk3588-base.dtsi: vop, vop_mmu, pmu,
 * cru, sys_grf, hdptxphy0_grf, hdmi0), as the t6_display probe maps them. */
#define VOP_PHYS     0xfdd90000UL
#define VOP_MAP      0x2000
#define VOP_MMU_PHYS 0xfdd97000UL /* the two sets at +0xe00, +0xf00 */
#define PMU_PHYS     0xfd8d8000UL
#define CRU_PHYS     0xfd7c0000UL
#define SYS_GRF_PHYS 0xfd58c000UL
#define PHY_GRF_PHYS 0xfd5e0000UL
#define HDMI_PHYS    0xfde80000UL

static struct yd_t6_display_bases s_bases;
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

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static void map_bases(void)
{
	mm_reg_t vop, mmu, pmu, cru, sys_grf, phy_grf, hdmi;

	device_map(&vop, VOP_PHYS, VOP_MAP, K_MEM_CACHE_NONE);
	device_map(&mmu, VOP_MMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&sys_grf, SYS_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy_grf, PHY_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&hdmi, HDMI_PHYS, 0x1000, K_MEM_CACHE_NONE);
	s_bases = (struct yd_t6_display_bases){
		.vop = vop,
		.vop_mmu = {mmu + 0xe00, mmu + 0xf00},
		.pmu = pmu,
		.cru = cru,
		.sys_grf = sys_grf,
		.phy_grf = phy_grf,
		.hdmi = hdmi,
	};
	s_mapped = true;
}

static int screen_show(const struct board_window *win)
{
	struct yd_t6_display_adopt adopt;
	bool first = s_screen.state == YD_T6_SCREEN_NEW;
	int rc;

	/* the VOP's window is set up for XRGB8888, top row first */
	if (win->format != GLES_NATIVE_FORMAT_XRGB8888) {
		return -ENOTSUP;
	}
	if (!s_mapped) {
		map_bases();
	}
	rc = yd_t6_screen_show(&s_bases, &s_screen, win->phys, win->w, win->h, win->stride,
			       delay_us, &adopt);
	/* the show that left YD_T6_SCREEN_NEW was the adopt: its record */
	if (first && s_screen.state != YD_T6_SCREEN_NEW) {
		t6_display_print_adopt("board", rc, &adopt);
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
	return yd_t6_screen_busy(&s_bases, &s_screen, phys, n);
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
 * which is the adopt: thread context. Returns whether the record is on the
 * screen, and prints why when it is not. */
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
 * lifetime is over, and the record stays until WDT0's reset. Timer context. */
void t6_record_screen(void)
{
	(void)record_screen(false);
}

/* A fatal error. Zephyr printed its registers, which are in the ring: put
 * the record on the screen, then halt with interrupts off as the default
 * handler does. The feeder stops with them and WDT0 resets the SoC within
 * one period: the time the record stays. */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	ARG_UNUSED(esf);
	printk("board: fatal error %u: halted, WDT0 resets the SoC within %u ms\n", reason,
	       t6_watchdog_period_ms());
	(void)record_screen(false);
	k_fatal_halt(reason);
}
#endif

FUNC_NORETURN void board_reset(void)
{
	/* A single-buffered sample shows one buffer and never flips; the count
	 * says whether a double-buffered run reached the flip and the busy rule. */
	printk("board: screen state %u, %u flips\n", s_screen.state, s_screen.flips);
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
