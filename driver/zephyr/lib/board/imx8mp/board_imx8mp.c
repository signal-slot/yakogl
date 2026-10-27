/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Verdin iMX8M Plus (yakogl_board.h):
 *   window   the GPU carve-out (gc_render_alloc), Normal non-cacheable
 *   screen   the panel through imx8mp_display_on() / _set_fb(); a window of
 *            the panel's size is scanned in place (a flip), a smaller one is
 *            a view into a panel-pitch buffer with YAKOGL_BOARD_SCANOUT_VIEW
 *            (scanned in place too), else copied into one of
 *            YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS panel-sized buffers of the
 *            carve-out (the fallback, counted: the 1024x600 Slint window on
 *            the 1280x800 panel without the view, glmark2's 800x600). The
 *            LCDIF's latch of a new address is waited for on a thread of its
 *            own (below); with one screen buffer (the default) every show
 *            waits for it, with more a copied window's show returns once the
 *            copy is done
 *   record   the SNVS LPGPR (gc_result.c, board_sink_store) and the line on
 *            the panel console
 *   selector the SNVS selector (gc_result.c, board_selector)
 *   watchdog the WDOG1 self-arm (gc_wdog_arm.c)
 *   reset    sys_reboot (PSCI SYSTEM_RESET)
 *   finish   with YAKOGL_BOARD_IMX8MP_DISPLAY_RECORD, a passed run's last
 *            record is the display chain's state (board_finish_record)
 *
 * With YAKOGL_BOARD_IMX8MP_ROUTE3 (board_route3.c) the windows are
 * frame-ring buffers in the M7's carve-out, the first show brings the panel
 * up and releases the M7, and every later show is a publish to the ring: the
 * M7 flips the LCDIF, this layer does not (no latch thread). If the M7 does
 * not come up, board_route3_show() says -EAGAIN and this layer flips the
 * same windows itself from then on.
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
#ifdef CONFIG_YAKOGL_DRV_GPU_VIVANTE
#include "gc_render.h"
#endif
#ifdef CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP
#include "imx8mp_display.h"
#endif
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_DISPLAY_RECORD
#include <yakogl_drivers.h>
#include "yakogl_record.h"
#endif
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
#include "board_imx8mp_route3.h"
BUILD_ASSERT(CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS >= 2,
	     "route 3 renders into one window while the M7 shows the other");
#endif

/* SCANOUT_VIEW: the panel-pitch buffer board_window() made, 0 for none */
static uintptr_t s_view_phys;
static size_t s_view_bytes;

#if defined(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP) && defined(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
/*
 * The latch. imx8mp_display_set_fb() writes the address, sets SHADOW_LOAD_EN
 * and spins until the LCDIF clears it at its next VSYNC: up to one panel
 * period, 17.0 ms at 58.76 Hz. Done on the showing thread it is a floor of
 * one period under every frame, whatever the swap interval
 * (docs/notes/imx8mp-frame-floor.md). So it runs on the latch thread below.
 * Whether a show waits for it is decided here:
 *
 *  - A window scanned in place (the panel's size, a view) is the buffer the
 *    GPU draws into next, so its show always waits for its latch. #793 let
 *    such a show return at once and glmark2 drew wrong on the board (#923,
 *    reverted by faee8812). Why the picture broke is not confirmed: the GM
 *    row's 800x600 window is copied, not scanned in place, so "the GPU drew
 *    into the buffer on the panel" does not explain the glmark2 picture by
 *    itself. The in-place show keeps its wait until a board run says more.
 *  - A copied window's show waits too while YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS
 *    is 1 (the default): the copy goes into the one buffer the LCDIF scans,
 *    and the timing is the code's before this thread. With 2 or 3 buffers
 *    the copy goes into a buffer the LCDIF neither scans nor is latching,
 *    and the show returns once the copy is done. That the LCDIF then only
 *    ever scans a complete frame, and the picture stays whole without the
 *    wait, is the hypothesis the board run of the note's step 2 checks; it
 *    is not the default until it has.
 *
 * State, under s_lock:
 *   s_shown     the address the LCDIF scans (turned on, or latched); 0
 *               before the first show
 *   s_latching  the address whose latch is in flight on the thread; 0 none
 *   s_asked     the newest request (s_asked_pending), taken by the thread:
 *               an address to latch, or 0 for the shown address again, a
 *               plain VSYNC wait (yakogl_zephyr_wait_vblank)
 *   s_ask_seq   counts requests; s_done_seq is the request the thread
 *               completed last, s_done_rc its set_fb result
 *   s_show_seq  the request of the last show; s_show_vblank says the next
 *               yakogl_zephyr_wait_vblank() waits for that latch instead of
 *               a VSYNC of its own (see there)
 * A newer request replaces an older one the thread has not taken yet (the
 * newest frame wins, nothing queues), so the thread is at most one latch
 * behind and a screen buffer is free as soon as the thread no longer holds
 * its address. The thread spins in set_fb's busy wait at the lowest
 * priority: on another core with CONFIG_SMP (the Verdin's 4 A53s), and
 * preempted by the showing thread on one core.
 */
static K_MUTEX_DEFINE(s_lock);
static K_CONDVAR_DEFINE(s_done);
static K_SEM_DEFINE(s_work, 0, 1);
static K_THREAD_STACK_DEFINE(s_latch_stack, 8192);
static struct k_thread s_latch_thread;
static uintptr_t s_shown;
static uintptr_t s_latching;
static uintptr_t s_asked;
static bool s_asked_pending;
static uint32_t s_ask_seq;
static uint32_t s_done_seq;
static int s_done_rc;
static uint32_t s_show_seq;
static bool s_show_vblank;

/* The panel-sized buffers a copied window is shown through, as many as the
 * carve-out gave up to the option; made at the first copy */
static struct gc_bo s_screen[CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS];
static unsigned int s_screens;

static void latch_thread(void *p1, void *p2, void *p3)
{
	ARG_UNUSED(p1);
	ARG_UNUSED(p2);
	ARG_UNUSED(p3);
	for (;;) {
		uintptr_t phys;
		uint32_t seq;
		int rc;

		k_sem_take(&s_work, K_FOREVER);
		k_mutex_lock(&s_lock, K_FOREVER);
		if (!s_asked_pending) {
			k_mutex_unlock(&s_lock);
			continue;
		}
		phys = s_asked != 0u ? s_asked : s_shown;
		seq = s_ask_seq;
		s_asked_pending = false;
		s_latching = phys;
		k_mutex_unlock(&s_lock);

		rc = imx8mp_display_set_fb(phys);

		k_mutex_lock(&s_lock, K_FOREVER);
		if (rc == 0) {
			s_shown = phys;
		}
		/* a latch that failed leaves the LCDIF on s_shown: the buffer
		 * is free again */
		s_latching = 0u;
		s_done_seq = seq;
		s_done_rc = rc;
		k_condvar_broadcast(&s_done);
		k_mutex_unlock(&s_lock);
	}
}

/* The panel scans `phys` (turned on): start the latch thread. s_lock held. */
static void latch_start(uintptr_t phys)
{
	s_shown = phys;
	k_thread_create(&s_latch_thread, s_latch_stack, K_THREAD_STACK_SIZEOF(s_latch_stack),
			latch_thread, NULL, NULL, NULL, K_LOWEST_APPLICATION_THREAD_PRIO, 0, K_NO_WAIT);
}

/* Ask the thread to latch `phys`, or the shown address again for 0 (a request
 * waiting for an address is kept then). s_lock held. Returns the request's
 * number for latched(). */
static uint32_t ask(uintptr_t phys)
{
	if (phys != 0u || !s_asked_pending) {
		s_asked = phys;
	}
	s_asked_pending = true;
	k_sem_give(&s_work);
	return ++s_ask_seq;
}

/* Wait until request `seq`, or a later one, completed. s_lock not held.
 * Returns the completing set_fb's result (0, -ETIMEDOUT from the display
 * driver's own bound, or another errno), or -ETIMEDOUT when `timeout` ran
 * out first. */
static int latched(uint32_t seq, k_timeout_t timeout)
{
	int rc;

	k_mutex_lock(&s_lock, K_FOREVER);
	while ((int32_t)(s_done_seq - seq) < 0) {
		if (k_condvar_wait(&s_done, &s_lock, timeout) != 0) {
			k_mutex_unlock(&s_lock);
			return -ETIMEDOUT;
		}
	}
	rc = s_done_rc;
	k_mutex_unlock(&s_lock);
	return rc;
}

/* The screen buffers, once: as many as the option asks and the carve-out
 * gives, at least one (-ENOMEM for none). Zeroed by gc_render_alloc, so the
 * panel around a smaller window shows black. */
static int screens_alloc(uint32_t pw, uint32_t ph)
{
	size_t bytes = (size_t)pw * 4u * ph;

	if (s_screens != 0u) {
		return 0;
	}
	while (s_screens < ARRAY_SIZE(s_screen)) {
		int rc = gc_render_alloc(bytes, 64, &s_screen[s_screens]);

		if (rc != 0) {
			if (s_screens == 0u) {
				printk("board: no carve-out for the %ux%u screen: %d\n", pw, ph, rc);
				return -ENOMEM;
			}
			printk("board: the carve-out holds %u of %u %ux%u screen buffers: %d\n",
			       s_screens, (unsigned int)ARRAY_SIZE(s_screen), pw, ph, rc);
			break;
		}
		s_screens++;
	}
	printk("board: a copied window is shown through %u %ux%u screen buffer(s)\n", s_screens,
	       pw, ph);
	return 0;
}

/* The screen buffer to copy the next frame into, s_lock held: one the LCDIF
 * neither scans nor is latching, a free one before the one whose frame still
 * waits for the thread (that frame is then dropped: the newest wins). NULL
 * when every buffer is on the panel or on its way there. */
static struct gc_bo *screen_pick(void)
{
	struct gc_bo *waiting = NULL;

	for (unsigned int i = 0; i < s_screens; i++) {
		uintptr_t phys = (uintptr_t)s_screen[i].phys;

		if (phys == s_shown || phys == s_latching) {
			continue;
		}
		if (s_asked_pending && phys == s_asked) {
			waiting = &s_screen[i];
			continue;
		}
		return &s_screen[i];
	}
	return waiting;
}
#endif /* CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP && CONFIG_YAKOGL_DRV_GPU_VIVANTE */

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u || w > 4096u || h > 4096u) {
		return -EINVAL;
	}
#ifdef CONFIG_YAKOGL_DRV_GPU_VIVANTE
	/* Up to YAKOGL_BOARD_IMX8MP_WINDOWS windows, each its own carve-out
	 * buffer (a view takes the one and only). A sample that renders into
	 * two of the panel's size in turn and shows each in place never draws
	 * into the buffer the LCDIF scans (slint, SLINT_WINDOW_BUFFERS). */
	static unsigned int taken;
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo bo;
	int rc;

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
	/* route 3: the windows are the ring's buffers (unless the M7 image did
	 * not load: then windows of our own, and no M7) */
	if (board_route3_state() != BOARD_ROUTE3_ABSENT) {
		int r3 = board_route3_window(taken, w, h, out);

		if (r3 == 0) {
			taken++;
			return 0;
		}
		if (board_route3_state() != BOARD_ROUTE3_ABSENT) {
			return r3;
		}
	}
#endif
	if (taken >= CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS || s_view_phys != 0u) {
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

#ifdef CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP
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
	taken++;
	*out = (struct board_window){
		.phys = (uintptr_t)bo.phys,
		.cpu = bo.cpu,
		.w = w,
		.h = h,
		.stride = stride,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	if (stride != w * 4u) {
		/* the LCDIF scans this buffer: EGL must not double-buffer it */
		out->format |= GLES_NATIVE_WINDOW_SCANOUT;
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
	board_first_frame();
#if defined(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP) && defined(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
	uint32_t pw = 0, ph = 0;
	uintptr_t scan;
	uint32_t pitch;
	bool wait; /* the show waits for its latch: see "The latch" above */
	uint32_t seq;
	int rc;

	imx8mp_display_geometry(&pw, &ph);
	if (pw == 0u || ph == 0u) {
		return -ENODEV;
	}
	if (win->w > pw || win->h > ph || (win->format & GLES_NATIVE_WINDOW_BOTTOM_UP)) {
		return -ENOTSUP;
	}
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
	{
		static uintptr_t r3_first; /* what the panel was turned on with */
		int st = board_route3_state();

		if (st != BOARD_ROUTE3_OFF && st != BOARD_ROUTE3_ABSENT) {
			if (st == BOARD_ROUTE3_LOADED) {
				r3_first = win->phys;
			}
			rc = board_route3_show(win);
			if (rc != -EAGAIN) {
				return rc;
			}
			/* no M7: the panel still scans the first frame (nobody
			 * else flipped it); from here this layer flips */
			k_mutex_lock(&s_lock, K_FOREVER);
			latch_start(r3_first);
			k_mutex_unlock(&s_lock);
		}
	}
#endif
	k_mutex_lock(&s_lock, K_FOREVER);
	if (!IS_ENABLED(CONFIG_YAKOGL_BOARD_IMX8MP_ALWAYS_COPY) &&
	    ((win->w == pw && win->h == ph) ||
	     (s_view_phys != 0u && win->stride == pw * 4u && win->phys >= s_view_phys &&
	      win->phys + (size_t)ph * win->stride <= s_view_phys + s_view_bytes))) {
		/* the panel's size, or a view at the panel's pitch with ph rows
		 * of its buffer from the window's first row on: scanned in place */
		scan = win->phys;
		pitch = win->stride;
		wait = true;
	} else {
		struct gc_bo *bo;

		rc = screens_alloc(pw, ph);
		if (rc != 0) {
			k_mutex_unlock(&s_lock);
			return rc;
		}
		bo = screen_pick();
		if (bo == NULL && s_latching != 0u) {
			/* one or two buffers and frames faster than the panel:
			 * the latch in flight ends at the next VSYNC and frees
			 * the buffer it replaces (paced at the panel's rate) */
			seq = s_ask_seq;
			k_mutex_unlock(&s_lock);
			(void)latched(seq, K_FOREVER);
			k_mutex_lock(&s_lock, K_FOREVER);
			bo = screen_pick();
		}
		if (bo == NULL) {
			/* one buffer: the one on the panel, and the show waits for
			 * its latch below (the default; the timing of the code
			 * before the latch thread) */
			bo = &s_screen[0];
		}
		board_show_copied(win->w, win->h, "neither the panel's size nor a view");
		for (uint32_t y = 0; y < win->h; y++) {
			memcpy((uint8_t *)bo->cpu + (size_t)y * pw * 4u,
			       (const uint8_t *)win->cpu + (size_t)y * win->stride, (size_t)win->w * 4u);
		}
		scan = (uintptr_t)bo->phys;
		pitch = pw * 4u;
		wait = scan == s_shown;
	}
	if (s_shown == 0u) {
		/* the first show: the panel on, then the latch thread */
		rc = imx8mp_display_on(scan, pitch);
		if (rc == 0) {
			/* the window is scanned (display_on waited for its latch):
			 * now the viewer may see the panel */
			imx8mp_display_backlight_on();
			latch_start(scan);
		}
		k_mutex_unlock(&s_lock);
		return rc;
	}
	seq = ask(scan);
	s_show_seq = seq;
	s_show_vblank = true;
	k_mutex_unlock(&s_lock);
	return wait ? latched(seq, K_FOREVER) : 0;
#else
	ARG_UNUSED(win);
	return -ENOTSUP;
#endif
}

/* The vertical-sync source for eglSwapInterval, called by the GC7000's OSAL
 * (src/osal/zephyr_gc.c, .wait_vblank; its weak default and zephyr.c's say
 * -ENOTSUP): one panel period per call, counted from the frame on the panel.
 *
 *  - The first call after a show waits for that show's latch: the VSYNC that
 *    puts the shown frame on the panel. The next show's latch can only come
 *    at a later VSYNC, so the frame stays on the panel for a period, which is
 *    what swap interval 1 asks. A show that already waited for its latch (a
 *    window scanned in place, and a copied one with one screen buffer) makes
 *    this return at once: the frame waits once, in the show, not a second
 *    time here. A copied show that returned after the copy (2 or 3 screen
 *    buffers) is paced here, at the panel's rate.
 *  - A further call (interval 2 and up, or a swap with no show since the last
 *    call) waits for the next VSYNC: the thread latches the address a frame
 *    asked for, or the shown address again (the picture does not change).
 *
 * The wait ends at that VSYNC or at `timeout_us` (-ETIMEDOUT; 0 does not
 * wait, a negative value waits for ever), and a latch the display driver gave
 * up on (its own bound) is -ETIMEDOUT too. -ENOTSUP before the first show
 * (nothing is on the panel yet) and for any other display error. The waits a
 * frame of each application makes: docs/notes/imx8mp-frame-floor.md, "Waits
 * per swap" (host/latch_model.c plays them). */
int yakogl_zephyr_wait_vblank(int64_t timeout_us)
{
#if defined(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP) && defined(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
	k_timeout_t timeout;
	uint32_t seq;
	bool show;
	int rc;

	k_mutex_lock(&s_lock, K_FOREVER);
	if (s_shown == 0u) {
		k_mutex_unlock(&s_lock);
		return -ENOTSUP;
	}
	show = s_show_vblank;
	seq = show ? s_show_seq : ask(0u);
	k_mutex_unlock(&s_lock);
	if (timeout_us < 0) {
		timeout = K_FOREVER;
	} else {
		timeout = K_TIMEOUT_ABS_TICKS(k_uptime_ticks() +
					      (int64_t)k_us_to_ticks_ceil64((uint64_t)timeout_us));
	}
	rc = latched(seq, timeout);
	if (show) {
		/* that show's period is counted once its latch is done (not when
		 * the deadline ran out first), unless a newer show came */
		k_mutex_lock(&s_lock, K_FOREVER);
		if (s_show_seq == seq && (int32_t)(s_done_seq - seq) >= 0) {
			s_show_vblank = false;
		}
		k_mutex_unlock(&s_lock);
	}
	return rc == 0 || rc == -ETIMEDOUT ? rc : -ENOTSUP;
#else
	ARG_UNUSED(timeout_us);
	return -ENOTSUP;
#endif
}

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_DISPLAY_RECORD
/* The DISPLAY record (record.rs, kind DISPLAY; issue #23): the chain as it
 * scans the window, after a second of it, so the bridge's IRQ_STAT has had
 * about 60 frames to latch a link error. Only after a pass, and only once a
 * show put the window on the panel: a failed run keeps its verdict. */
void board_finish_record(bool pass)
{
	const struct imx8mp_display_info *info = imx8mp_display_info();
	struct yd_record_display d;
	uint32_t w[3];
	int irq;

	if (!pass || s_shown == 0u) {
		return;
	}
	for (int i = 0; i < 10; i++) {
		k_msleep(100);
		board_watchdog_feed();
	}
	irq = imx8mp_display_bridge_irq_stat();
	d = (struct yd_record_display){
		.adopted = info->adopted ? 1u : 0u,
		.pixel_clock_hz = info->pixel_clock_hz,
		.lane_rate_hz = info->dsi_hs_clock_hz,
		.irq_now = irq >= 0 ? (uint32_t)irq : YK_RECORD_DISPLAY_IRQ_NOT_READ,
		.irq_at_start = info->bridge_irq_stat,
		.health = imx8mp_display_health(),
		.bpp_code = (imx8mp_display_layer_control() >> 24) & 0xfu,
		.pitch = info->pitch,
	};
	yd_record_display_words(&d, w);
	printk("board: display %s, pixel clock %u Hz, DSI %u Hz, IRQ_STAT 0x%02x (start 0x%02x), "
	       "health 0x%02x, CTRLDESCL0_5 0x%08x, pitch %u\n",
	       info->adopted ? "adopted" : "cold", info->pixel_clock_hz, info->dsi_hs_clock_hz,
	       d.irq_now, d.irq_at_start, d.health, imx8mp_display_layer_control(), d.pitch);
	board_result(YK_RECORD_KIND_DISPLAY, w[0], w[1], w[2]);
}
#endif

FUNC_NORETURN void board_reset(void)
{
	printk("board: resetting the SoC\n");
	sys_reboot(SYS_REBOOT_COLD);
	CODE_UNREACHABLE;
}
