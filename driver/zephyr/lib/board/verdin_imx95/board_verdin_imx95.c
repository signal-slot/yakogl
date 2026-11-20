/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Verdin iMX95's A55 (yakogl_board.h;
 * CONFIG_YAKOGL_BOARD_VERDIN_IMX95), so that the examples build for the
 * board (examples/zephyr.cmake):
 *   window   the Mali carve-out's window memory (osal_zephyr_mali_window_mem:
 *            the carve-out's last CONFIG_YAKOGL_MALI_WINDOW_BYTES, never
 *            allocated, Normal non-cacheable, reached by the GPU through
 *            map_phys): the windows from its start, up to
 *            CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB, then one panel-sized screen
 *            buffer
 *   screen   with CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY the Toradex 10.1" DSI
 *            display on X4 (verdin_imx95_display.c): a window of the
 *            panel's size (1280x800, stride 5120) is scanned in place, the
 *            first show bringing the picture up and every later one a flip
 *            that returns once the DPU has taken the buffer (so no
 *            board_display_busy); any other window is copied into the
 *            screen buffer at the panel's (0,0) and that buffer is shown
 *            (the fallback, counted: board_show_copied). Without the option
 *            board_show() refuses (-ENOTSUP)
 *   record   the yk-record line on the console UART (common/record.c)
 *   watchdog WDOG3 (verdin_imx95_wdog.c), fed while board_wdog_alive()
 *   reset    verdin_imx95_reset(): PSCI SYSTEM_RESET, the picture taken
 *            down first
 * Built, not run on a board yet.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "verdin_imx95.h"
#ifdef CONFIG_YAKOGL_GPU_MALI
#include "zephyr_mali_board.h"
#endif
#ifdef CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY
#include "verdin_imx95_display.h"
#endif

#define WIN_BYTES    ((size_t)CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB * 1024u)
#define SCREEN_W     1280u
#define SCREEN_H     800u
#define SCREEN_PITCH (SCREEN_W * 4u)
#define SCREEN_BYTES ((size_t)SCREEN_PITCH * SCREEN_H)

BUILD_ASSERT(WIN_BYTES % 4096u == 0, "the windows end on a page: the screen buffer starts on one");
#ifdef CONFIG_YAKOGL_GPU_MALI
BUILD_ASSERT(WIN_BYTES + SCREEN_BYTES <= CONFIG_YAKOGL_MALI_WINDOW_BYTES,
	     "the Mali carve-out's window memory holds the windows and the screen buffer");
#endif
#ifdef CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY
BUILD_ASSERT(SCREEN_W == VERDIN_IMX95_SCREEN_W && SCREEN_H == VERDIN_IMX95_SCREEN_H,
	     "the screen buffer is the panel's size");
#endif

static uintptr_t s_mem_phys; /* the window memory, 0 before the first window */
static uint8_t *s_mem_cpu;
static size_t s_win_used; /* bytes the windows took from its start */

static int window_mem(void)
{
	size_t bytes = 0;
	void *cpu = NULL;
	int rc;

	if (s_mem_phys != 0u) {
		return 0;
	}
#ifdef CONFIG_YAKOGL_GPU_MALI
	rc = osal_zephyr_mali_window_mem(&s_mem_phys, &cpu, &bytes);
#else
	rc = -ENOTSUP; /* the window memory is the Mali OSAL's */
#endif
	if (rc != 0 || bytes < WIN_BYTES + SCREEN_BYTES) {
		printk("verdin_imx95: no window memory in the Mali carve-out: %d (%zu bytes)\n", rc,
		       bytes);
		s_mem_phys = 0u;
		return rc != 0 ? rc : -ENOMEM;
	}
	s_mem_cpu = cpu;
	return 0;
}

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	size_t bytes = (size_t)w * h * 4u;

	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || w > 4096u || h > 4096u) {
		return -EINVAL;
	}
	if (window_mem() != 0) {
		return -ENOMEM;
	}
	bytes = ROUND_UP(bytes, 4096u);
	if (bytes > WIN_BYTES - s_win_used) {
		return -ENOMEM;
	}
	*out = (struct board_window){
		.phys = s_mem_phys + s_win_used,
		.cpu = s_mem_cpu + s_win_used,
		.w = w,
		.h = h,
		.stride = w * 4u,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	s_win_used += bytes;
	return 0;
}

int board_show(const struct board_window *win)
{
	board_first_frame();
#ifdef CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY
	uint8_t *screen = s_mem_cpu + WIN_BYTES;
	uint32_t w, h;

	if (win->format != GLES_NATIVE_FORMAT_XRGB8888 || s_mem_phys == 0u) {
		return -ENOTSUP;
	}
	if (win->w == SCREEN_W && win->h == SCREEN_H && win->stride == SCREEN_PITCH) {
		return verdin_imx95_display_show(win->phys, win->stride);
	}
	/* the fallback: the window's rows at the panel's (0,0), the rest of
	 * the screen as it was (black at first: the window memory is zeroed) */
	w = MIN(win->w, SCREEN_W);
	h = MIN(win->h, SCREEN_H);
	board_show_copied(win->w, win->h, "not the panel's 1280x800");
	for (uint32_t y = 0; y < h; y++) {
		memcpy(screen + (size_t)y * SCREEN_PITCH,
		       (const uint8_t *)win->cpu + (size_t)y * win->stride, (size_t)w * 4u);
	}
	return verdin_imx95_display_show(s_mem_phys + WIN_BYTES, SCREEN_PITCH);
#else
	ARG_UNUSED(win);
	return -ENOTSUP;
#endif
}

int board_selector(void)
{
	return -1;
}

void board_sink_store(const uint32_t w[4])
{
	ARG_UNUSED(w); /* the UART line is the sink */
}

FUNC_NORETURN void board_reset(void)
{
#ifdef CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY
	/* the backlight off and the bridge's EN low before the SoC resets
	 * (board_finish's hold, on the main thread) */
	verdin_imx95_display_down();
#endif
	verdin_imx95_reset("board: run over");
}
