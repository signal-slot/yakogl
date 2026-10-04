/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Verdin AM62P (yakogl_board.h):
 *   window   a static buffer in the image, used through a Normal
 *            non-cacheable alias (as v3d_gl_slint does on the Pi 4), so
 *            neither the GPU nor a check needs cache maintenance
 *   screen   the SafeUI DSS application plane, through sample_glue.c's
 *            pvr_sample_show_window / _flip_window (vflip when the driver
 *            can; the window is then drawn bottom-up): every board_show()
 *            points the plane at the window's memory, never a copy
 *   record   the yk-record line on the UART (common/record.c); no register
 *            that survives the RTI reset is known (one-method.md 1.5)
 *   watchdog the MAIN RTI0 self-arm (rti_arm.c)
 *   reset    sys_reboot
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/cache.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/reboot.h>
#include <errno.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "sample_glue.h"

static uint8_t s_win[CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB * 1024] __noinit __aligned(4096);
static bool s_taken, s_shown;

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	uint8_t *nc = NULL;
	/* CONFIG_YAKOGL_BOARD_WINDOW_RGB565: 2 bytes a pixel */
	const uint32_t bpp = IS_ENABLED(CONFIG_YAKOGL_BOARD_WINDOW_RGB565) ? 2u : 4u;
	size_t bytes = (size_t)w * h * bpp;

	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || bytes > sizeof(s_win)) {
		return -EINVAL;
	}
	if (s_taken) {
		return -ENOMEM;
	}
	/* Nothing the CPU cached of the buffer may be written back over the
	 * GPU's pixels later. */
	sys_cache_data_flush_and_invd_range(s_win, sizeof(s_win));
	k_mem_map_phys_bare(&nc, k_mem_phys_addr(s_win), ROUND_UP(bytes, 4096),
			    K_MEM_PERM_RW | K_MEM_ARM_NORMAL_NC);
	if (nc == NULL) {
		return -ENOMEM;
	}
	s_taken = true;
	*out = (struct board_window){
		.phys = k_mem_phys_addr(s_win),
		.cpu = nc,
		.w = w,
		.h = h,
		.stride = w * bpp,
		.format = (IS_ENABLED(CONFIG_YAKOGL_BOARD_WINDOW_RGB565) ? GLES_NATIVE_FORMAT_RGB565
									  : GLES_NATIVE_FORMAT_XRGB8888) |
			  pvr_sample_window_flags(),
	};
	return 0;
}

int board_show(const struct board_window *win)
{
	int rc;

	if (!IS_ENABLED(CONFIG_PVR_SAMPLE_SHOW_ON_DSS)) {
		return -ENOTSUP;
	}
	/* the DSS path is set up for 32-bit pixels: a 565 window is rendered,
	 * not shown (CONFIG_YAKOGL_BOARD_WINDOW_RGB565) */
	if ((win->format & ~GLES_NATIVE_WINDOW_BOTTOM_UP) == GLES_NATIVE_FORMAT_RGB565) {
		return -ENOTSUP;
	}
	if (s_shown) {
		return pvr_sample_flip_window(win->phys, win->w, win->h, win->stride, win->format);
	}
	rc = pvr_sample_show_window(win->phys, win->w, win->h, win->stride, win->format);
	s_shown = rc == 0;
	return rc;
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
	printk("board: resetting the SoC\n");
	k_busy_wait(20000); /* the UART drains */
	sys_reboot(SYS_REBOOT_COLD);
	CODE_UNREACHABLE;
}
