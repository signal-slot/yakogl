/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Verdin iMX8M Plus (yakogl_board.h):
 *   window   the GPU carve-out (gc_render_alloc), Normal non-cacheable, as
 *            gc_gl_triangle's window
 *   screen   the panel through imx8mp_display_on() / _set_fb(); a window of
 *            the panel's size is scanned in place (a flip), a smaller one is
 *            a view into a panel-pitch buffer with YAKOGL_BOARD_SCANOUT_VIEW
 *            (scanned in place too), else copied into a panel-sized buffer
 *            of the carve-out (the fallback, counted: the 1024x600 Slint
 *            window on the 1280x800 panel without the view)
 *   record   the SNVS LPGPR (gc_result.c, board_sink_store) and the line on
 *            the panel console
 *   selector the SNVS selector (gc_result.c, board_selector)
 *   watchdog the WDOG1 self-arm (gc_wdog_arm.c)
 *   reset    sys_reboot (PSCI SYSTEM_RESET)
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/reboot.h>
#include <errno.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include "yakogl_board.h"
#include "board_internal.h"
#ifdef CONFIG_SAFEUI_GPU_VIVANTE
#include "gc_render.h"
#endif
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
#include "imx8mp_display.h"
#endif

/* SCANOUT_VIEW: the panel-pitch buffer board_window() made, 0 for none */
static uintptr_t s_view_phys;
static size_t s_view_bytes;
#if defined(CONFIG_SAFEUI_DISPLAY_IMX8MP) && defined(CONFIG_SAFEUI_GPU_VIVANTE)
/* The buffer the LCDIF scans since board_show() turned it on or latched it,
 * 0 before the first show */
static uintptr_t s_scan;
#endif

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || w > 4096u || h > 4096u) {
		return -EINVAL;
	}
#ifdef CONFIG_SAFEUI_GPU_VIVANTE
	static bool taken;
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo bo;
	int rc;

	if (taken) {
		return -ENOMEM;
	}
	if (!device_is_ready(gpu)) {
		return -ENODEV;
	}
	/* The window has to exist before EGL wraps it; gc_render_init is
	 * idempotent, eglInitialize calls it again through the OSAL. */
	rc = gc_render_init(gpu);
	if (rc != 0) {
		printk("board: gc_render_init: %d\n", rc);
		return -ENODEV;
	}
	/* SCANOUT_VIEW: a window narrower or shorter than the panel is a view
	 * into a buffer of the panel's pitch (pw * 4 bytes, the LCDIF's PITCH,
	 * which it wants 64-byte aligned: ip/lcdifv3.rs), so board_show() scans
	 * it in place. The buffer has h + ph rows: the LCDIF scans ph rows from
	 * whichever row a part of the window starts at (a second half at row
	 * H scans rows H .. H + ph - 1). */
	uint32_t pw = 0, ph = 0, stride = w * 4u;
	size_t rows = h;

#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
	if (IS_ENABLED(CONFIG_YAKOGL_BOARD_SCANOUT_VIEW)) {
		imx8mp_display_geometry(&pw, &ph);
		if (pw != 0u && w <= pw && (w < pw || h < ph) && (pw * 4u) % 64u == 0u) {
			stride = pw * 4u;
			rows = (size_t)h + ph;
		}
	}
#endif
	rc = gc_render_alloc((size_t)stride * rows, 64, &bo);
	if (rc != 0) {
		printk("board: gc_render_alloc(%ux%u, stride %u): %d\n", w, h, stride, rc);
		return -ENOMEM;
	}
	taken = true;
	*out = (struct board_window){
		.phys = (uintptr_t)bo.phys,
		.cpu = bo.cpu,
		.w = w,
		.h = h,
		.stride = stride,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	if (stride != w * 4u) {
		/* what the panel shows around the window: black, not old memory */
		memset(bo.cpu, 0, (size_t)stride * rows);
		s_view_phys = (uintptr_t)bo.phys;
		s_view_bytes = (size_t)stride * rows;
		printk("board: the %ux%u window is a view into a %ux%u-pitch buffer (no copy)\n", w, h, pw,
		       (unsigned int)rows);
	}
	return 0;
#else
	return -ENODEV;
#endif
}

int board_show(const struct board_window *win)
{
#if defined(CONFIG_SAFEUI_DISPLAY_IMX8MP) && defined(CONFIG_SAFEUI_GPU_VIVANTE)
	static bool on;
	static struct gc_bo screen; /* the panel-sized copy, when the window is smaller */
	uint32_t pw = 0, ph = 0;
	uintptr_t scan;
	uint32_t pitch;
	int rc;

	imx8mp_display_geometry(&pw, &ph);
	if (pw == 0u || ph == 0u) {
		return -ENODEV;
	}
	if (win->w > pw || win->h > ph || (win->format & GLES_NATIVE_WINDOW_BOTTOM_UP)) {
		return -ENOTSUP;
	}
	if ((win->w == pw && win->h == ph) ||
	    (s_view_phys != 0u && win->stride == pw * 4u && win->phys >= s_view_phys &&
	     win->phys + (size_t)ph * win->stride <= s_view_phys + s_view_bytes)) {
		/* the panel's size, or a view at the panel's pitch with ph rows
		 * of its buffer from the window's first row on: scanned in place */
		scan = win->phys;
		pitch = win->stride;
	} else {
		if (screen.cpu == NULL) {
			rc = gc_render_alloc((size_t)pw * 4u * ph, 64, &screen);
			if (rc != 0) {
				printk("board: no carve-out for the %ux%u screen: %d\n", pw, ph, rc);
				return -ENOMEM;
			}
			memset(screen.cpu, 0, (size_t)pw * 4u * ph);
		}
		board_show_copied(win->w, win->h, "neither the panel's size nor a view");
		for (uint32_t y = 0; y < win->h; y++) {
			memcpy((uint8_t *)screen.cpu + (size_t)y * pw * 4u,
			       (const uint8_t *)win->cpu + (size_t)y * win->stride, (size_t)win->w * 4u);
		}
		scan = (uintptr_t)screen.phys;
		pitch = pw * 4u;
	}
	if (on) {
		/* The LCDIF already scans this buffer (the one-buffer window
		 * after its first show): nothing to flip. set_fb would only
		 * wait for the next VSYNC (the shadow load), a panel period a
		 * frame whatever the swap interval; pacing is
		 * yakogl_zephyr_wait_vblank()'s below. */
		if (scan == s_scan && !IS_ENABLED(CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC)) {
			return 0;
		}
		rc = imx8mp_display_set_fb(scan);
		if (rc == 0) {
			s_scan = scan;
		}
		return rc;
	}
	rc = imx8mp_display_on(scan, pitch);
	on = rc == 0;
	if (on) {
		s_scan = scan;
	}
	return rc;
#else
	ARG_UNUSED(win);
	return -ENOTSUP;
#endif
}

/* The vertical-sync source for eglSwapInterval (src/osal/zephyr.c has the
 * weak default, -ENOTSUP). It latches the address the LCDIF already scans
 * again: set_fb sets the shadow-load bit, which the LCDIF clears at its next
 * VSYNC, and returns then. The picture does not change; it is the wait
 * board_show() did every frame before, now only for a swap interval of 1 or
 * more. It works on both LCDIF builds (imx8mp_lcdif.c and the Rust
 * yakogl,imx8mp-lcdif, whose interrupt only a cold bring-up enables).
 *
 * The wait cannot be cut short: it ends at the VSYNC or at the LCDIF
 * driver's own bound (36 ms in imx8mp_lcdif.c, 83 ms in lcdifv3.rs), and a
 * VSYNC later than timeout_us is reported as -ETIMEDOUT. Returns 0,
 * -ETIMEDOUT, or -ENOTSUP before the first show, with
 * YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC (board_show() waits instead) and
 * without the display. */
int yakogl_zephyr_wait_vblank(int64_t timeout_us)
{
#if defined(CONFIG_SAFEUI_DISPLAY_IMX8MP) && defined(CONFIG_SAFEUI_GPU_VIVANTE)
	uint64_t t0;
	int64_t us;
	int rc;

	if (IS_ENABLED(CONFIG_YAKOGL_BOARD_IMX8MP_SHOW_WAITS_VSYNC) || s_scan == 0u) {
		return -ENOTSUP;
	}
	if (timeout_us == 0) {
		return -ETIMEDOUT;
	}
	t0 = k_cycle_get_64();
	rc = imx8mp_display_set_fb(s_scan);
	if (rc == -ETIMEDOUT) {
		return rc;
	}
	if (rc != 0) {
		return -ENOTSUP;
	}
	us = (int64_t)k_cyc_to_us_floor64(k_cycle_get_64() - t0);
	return timeout_us > 0 && us > timeout_us ? -ETIMEDOUT : 0;
#else
	ARG_UNUSED(timeout_us);
	return -ENOTSUP;
#endif
}

FUNC_NORETURN void board_reset(void)
{
	printk("board: resetting the SoC\n");
	sys_reboot(SYS_REBOOT_COLD);
	CODE_UNREACHABLE;
}
