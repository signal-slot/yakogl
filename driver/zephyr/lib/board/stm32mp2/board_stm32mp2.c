/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the STM32MP257F-DK's A35 (yakogl_board.h;
 * CONFIG_YAKOGL_BOARD_STM32MP2; plans #322 and #353, issues #7 and #1281):
 *   window   with the display (CONFIG_YAKOGL_STM32MP2_DISPLAY: the LTDC,
 *            DSI, ADV7535 and I2C drivers and the path's power-on) and the
 *            Vivante driver: a block of the GPU carve-out (gc_render_alloc,
 *            Normal non-cacheable, identity-mapped below 4 GiB) of
 *            XRGB8888 rows at the LTDC's pitch, ROUND_UP(w * 4, 64)
 *            (docs/notes/stm32mp257f-dk-display-bringup.md, "GPU buffer
 *            format"), 4 KiB aligned as gc_triangle's scan-out buffer, and
 *            allowed for the LTDC master (stm32_ltdc_board_fb_check,
 *            "Framebuffer memory and bus-master access"). A stride wider
 *            than w * 4 makes the window GLES_NATIVE_WINDOW_SCANOUT, as on
 *            the i.MX8MP. Without the display, or with its LTDC device not
 *            ready, board_window() refuses (-ENOTSUP), as before.
 *   screen   LTDC -> DSI -> ADV7535 -> HDMI through the LTDC driver
 *            (yakogl_drv/stm32_ltdc.h). The first show brings the chain up
 *            (display_blanking_off). A window (or a part of it) of the
 *            mode's width at the chain's pitch, with the mode's height of
 *            rows of this layer's window from its first row on, is scanned
 *            in place: stm32_ltdc_set_fb() points the layer at it and waits
 *            for the vblank reload. Any other window is copied into the
 *            driver's own frame buffer (the fallback, counted:
 *            board_show_copied) and the layer pointed back at it. Every
 *            show then waits for one frame (stm32_ltdc_wait_frame). Without
 *            the display, -ENOTSUP, as before.
 *   record   the yk-record line on USART2 (common/record.c)
 *   watchdog OP-TEE's arm,smc-wdt, armed at boot by stm32mp2_wdog.c
 *            (its "gc_wdog_arm: armed" line), and the layer's lifetime
 *            (CONFIG_YAKOGL_STM32MP2_LIFETIME_S)
 *   reset    mp2_reset_to_linux(): PSCI SYSTEM_RESET, back into ST's Linux
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "stm32mp2.h"

#if defined(CONFIG_YAKOGL_STM32MP2_DISPLAY) && defined(CONFIG_YAKOGL_DRV_GPU_VIVANTE) &&      \
	DT_HAS_COMPAT_STATUS_OKAY(yakogl_stm32_ltdc)
#define MP2_SCREEN 1
#include <zephyr/drivers/display.h>
#include <EGL/eglplatform.h>
#include <yakogl_drv/stm32_ltdc.h>
#include "gc_render.h"
#else
#define MP2_SCREEN 0
#endif

BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "a board-layer image on the DK runs under OP-TEE's watchdog");

#if MP2_SCREEN
/* The LTDC's line alignment (the driver's ltdc_pitch, ip/stm32_ltdc.rs) and
 * the pixel engine's. */
#define MP2_PITCH_ALIGN 64u
/* gc_triangle's scan-out buffer alignment, kept for the window. */
#define MP2_WINDOW_ALIGN 4096u

static const struct device *ltdc_dev(void)
{
	return DEVICE_DT_GET_ONE(yakogl_stm32_ltdc);
}

/* The layer was pointed at a window or the driver's buffer at least once. */
static bool s_shown;
/* The window board_window() made: a show scans in place only rows inside it. */
static uintptr_t s_win_phys;
static size_t s_win_bytes;
#endif

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	memset(out, 0, sizeof(*out));
#if MP2_SCREEN
	/* One window per run (yakogl_board.h); a sample splits it itself. */
	static bool taken;
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo bo;
	uint32_t stride;
	size_t bytes;
	int rc;

	if (!device_is_ready(ltdc_dev())) {
		return -ENOTSUP;
	}
	if (w == 0u || h == 0u || w > 4096u || h > 4096u) {
		return -EINVAL;
	}
	if (taken) {
		return -ENOMEM;
	}
	if (!device_is_ready(gpu)) {
		return -ENODEV;
	}
	/* The window exists before EGL wraps it; gc_render_init is
	 * idempotent, eglInitialize calls it again through the OSAL. */
	rc = gc_render_init(gpu);
	if (rc != 0) {
		printk("board: gc_render_init: %d\n", rc);
		return -ENODEV;
	}
	stride = ROUND_UP(w * 4u, MP2_PITCH_ALIGN);
	bytes = (size_t)stride * h;
	rc = gc_render_alloc(bytes, MP2_WINDOW_ALIGN, &bo);
	if (rc == 0 && bo.cpu == NULL) {
		rc = -EFAULT;
	}
	if (rc != 0) {
		printk("board: gc_render_alloc(%ux%u, stride %u): %d\n", w, h, stride, rc);
		return -ENOMEM;
	}
	/* bo.phys is identity-mapped and below 4 GiB (gc_render.h): what the
	 * LTDC's L1CFBAR takes. The range must pass the LTDC master's RIF and
	 * RISAF4 checks, or the screen stays black with no error. */
	rc = stm32_ltdc_board_fb_check(bo.phys, bytes);
	if (rc != 0) {
		printk("board: the window at 0x%llx (%zu bytes) is not allowed for the LTDC: %d\n",
		       (unsigned long long)bo.phys, bytes, rc);
		gc_render_free(&bo);
		return -ENODEV;
	}
	taken = true;
	s_win_phys = (uintptr_t)bo.phys;
	s_win_bytes = bytes;
	*out = (struct board_window){
		.phys = (uintptr_t)bo.phys,
		.cpu = bo.cpu,
		.w = w,
		.h = h,
		.stride = stride,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	if (stride != w * 4u) {
		/* the LTDC scans this buffer at its own pitch: EGL must not
		 * double-buffer it */
		out->format |= GLES_NATIVE_WINDOW_SCANOUT;
	}
	printk("board: window %ux%u stride %u at 0x%llx\n", w, h, stride,
	       (unsigned long long)bo.phys);
	return 0;
#else
	ARG_UNUSED(w);
	ARG_UNUSED(h);
	return -ENOTSUP;
#endif
}

int board_show(const struct board_window *win)
{
	board_first_frame();
#if MP2_SCREEN
	const struct device *dev = ltdc_dev();
	const struct stm32_ltdc_status *st;
	struct stm32_ltdc_fb fb;
	uint64_t scan;
	uint32_t isr = 0;
	int rc;

	if (!device_is_ready(dev)) {
		return -ENOTSUP;
	}
	/* The mode (chosen at the first call) and the driver's own buffer. */
	rc = stm32_ltdc_framebuffer(dev, &fb);
	if (rc != 0) {
		printk("board: stm32_ltdc_framebuffer: %d\n", rc);
		return -ENODEV;
	}
	if (win->w > fb.width || win->h > fb.height ||
	    (win->format & GLES_NATIVE_WINDOW_BOTTOM_UP) != 0u || fb.bpp != 4u) {
		return -ENOTSUP;
	}
	st = stm32_ltdc_status(dev);
	if (st == NULL) {
		return -ENODEV;
	}
	if (!st->on) {
		rc = display_blanking_off(dev);
		if (rc != 0) {
			printk("board: display chain: step %u err %d\n", st->step, st->err);
			return -ENODEV;
		}
		printk("board: display %ux%u pitch %u, pixel clock %u Hz\n", fb.width, fb.height,
		       fb.pitch, st->pixel_clock_hz);
	}
	if (win->w == fb.width && win->stride == fb.pitch && s_win_phys != 0u &&
	    win->phys >= s_win_phys &&
	    win->phys + (size_t)fb.height * fb.pitch <= s_win_phys + s_win_bytes) {
		/* the mode's width at the chain's pitch, with the mode's rows of
		 * this layer's window from the part's first row on: scanned in
		 * place (the flip keeps the chain's pitch and height) */
		scan = win->phys;
	} else {
		/* the fallback: the window copied into the driver's buffer
		 * (Normal non-cacheable, so the barrier in set_fb is all the
		 * maintenance it needs), the rest of the screen kept */
		board_show_copied(win->w, win->h, "not the mode's width at the LTDC's pitch");
		for (uint32_t y = 0; y < win->h; y++) {
			memcpy((uint8_t *)fb.virt + (size_t)y * fb.pitch,
			       (const uint8_t *)win->cpu + (size_t)y * win->stride,
			       (size_t)win->w * 4u);
		}
		scan = 0u; /* the driver's own buffer */
	}
	rc = stm32_ltdc_set_fb(dev, scan);
	if (rc != 0) {
		printk("board: stm32_ltdc_set_fb(0x%llx): %d\n", (unsigned long long)scan, rc);
		return rc == -ENODEV ? -ENODEV : -EIO;
	}
	if (!s_shown) {
		s_shown = true;
		printk("board: the LTDC scans 0x%llx\n", (unsigned long long)st->fb_phys);
	}
	rc = stm32_ltdc_wait_frame(dev, &isr);
	if (rc != 0) {
		printk("board: stm32_ltdc_wait_frame: %d (isr 0x%x)\n", rc, isr);
		return rc;
	}
	return 0;
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
	mp2_reset_to_linux("board: run over");
}
