/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Raspberry Pi 4 (yakogl_board.h):
 *   window   with YAKOGL_BOARD_SCANOUT_VIEW, a view into the firmware
 *            framebuffer at its (0,0) with its pitch when the window fits
 *            its width and its rows in memory (YAKOGL_BOARD_RPI4_FB_VIRTUAL_
 *            HEIGHT: two screens for a double buffer); board_show() of it
 *            copies nothing, and of the half at row y pans the screen to y
 *            (rpi_fb_pan, SET_VIRTUAL_OFFSET: v3d_gl_cluster's direct
 *            present). Any other window is a static buffer in the image
 *            through a Normal non-cacheable alias (v3d_gl_slint's window),
 *            copied by board_show() and counted (board_show_copied)
 *   screen   the firmware framebuffer from the mailbox (rpi_mbox.c),
 *            YAKOGL_BOARD_RPI4_FB_WIDTH x _HEIGHT; the console (fbcon.c)
 *            moves beside or under the window, or (no room) to the UART only
 *   record   the yk-record line on the mini UART, and the record screen:
 *            every console line is kept in a ring, and board_finish()
 *            freezes the last ones, the last yk-record line at the bottom,
 *            on the framebuffer until the watchdog resets the board
 *            (v3d_gl_test's frozen record, made common)
 *   watchdog the PM watchdog (rpi_wdog.c), fed until YAKOGL_BOARD_LIFETIME_S
 *   reset    the feeding stops (board_wdog_release); the PM watchdog resets
 *            the SoC within its 10 s period and tryboot returns to Linux
 *   format   the firmware's answer to SET_PIXEL_ORDER (RGB: RGBA8888, BGR:
 *            XRGB8888; src/board/rpi4.rs), printed as "board: rpi4
 *            framebuffer pixel order ...", for every window
 *   GPU      the V3D OSAL's default platform (the firmware clock switch), so
 *            eglGetDisplay(EGL_DEFAULT_DISPLAY) works as on the Verdins
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/cache.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <errno.h>
#include <stddef.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "osal_internal.h" /* struct osal_zephyr_v3d_init */
#include "rpi_mbox.h"
#include "fbcon.h"
#include <yakogl_drivers.h> /* yd_rpi4_*: the framebuffer decisions, src/board/rpi4.rs */

BUILD_ASSERT(sizeof(struct yd_rpi4_fb) == 24 && offsetof(struct yd_rpi4_fb, rows) == 20,
	     "src/ffi/board.rs pins rpi4::Fb");

/* ---- the GPU's platform -------------------------------------------------- */

static int v3d_clock_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	return rpi_clock_state(RPI_FW_CLK_V3D, on);
}

static const struct osal_zephyr_v3d_init s_v3d_platform = { .clock_set = v3d_clock_set };

const struct osal_zephyr_v3d_init *osal_zephyr_v3d_default_platform(void)
{
	return &s_v3d_platform;
}

/* ---- the console: UART + framebuffer + the ring of the record screen ---- */

#define RING_LINES 30
#define RING_COLS  158

static printk_hook_fn_t s_uart_out, s_fb_out;
static struct rpi_fb s_fb;
static bool s_have_fb;
static uint32_t s_fb_rows;  /* the framebuffer's rows in memory (its virtual height) */
/* The windows' format: the firmware's answer to SET_PIXEL_ORDER, never a
 * constant (0: not answered, and board_window() refuses). */
static uint32_t s_format;
static bool s_view;         /* board_window() handed out a view (SCANOUT_VIEW) */
static uint32_t s_shown_y;  /* the row the screen starts at (the last pan) */
static volatile bool s_ring_off; /* the record screen is being drawn */
static volatile bool s_frozen;   /* the record screen is up: nothing draws */
static char s_ring[RING_LINES][RING_COLS + 1];
static unsigned int s_ring_next, s_ring_count, s_ring_col;
static char s_last_record[RING_COLS + 1];

static void ring_put(int c)
{
	if (c == '\n') {
		char *l = s_ring[s_ring_next];

		l[s_ring_col] = '\0';
		if (strncmp(l, "yk-record: ", 11) == 0) {
			memcpy(s_last_record, l, sizeof(s_last_record));
		}
		s_ring_next = (s_ring_next + 1u) % RING_LINES;
		s_ring_count += s_ring_count < RING_LINES;
		s_ring_col = 0;
	} else if (c != '\r' && s_ring_col < RING_COLS) {
		s_ring[s_ring_next][s_ring_col++] = (char)c;
	}
}

static int console_out(int c)
{
	if (s_uart_out) {
		s_uart_out(c);
	}
	if (s_frozen) {
		return c;
	}
	if (IS_ENABLED(CONFIG_YAKOGL_BOARD_RECORD_SCREEN) && !s_ring_off) {
		ring_put(c);
	}
	return s_fb_out ? s_fb_out(c) : c;
}

/* fbcon_attach() installs fbcon's own hook; keep it and put ours back. */
static void console_attach(uint8_t *fb, uint32_t width, uint32_t rows_px)
{
	fbcon_attach(fb, width, s_fb.pitch, rows_px, 1);
	s_fb_out = __printk_get_hook();
	__printk_hook_install(console_out);
}

static int board_rpi4_init(void)
{
	uint32_t hz = 0, max_hz = 0;
	int rc;

	s_uart_out = __printk_get_hook();
	__printk_hook_install(console_out);
	const int mbox = rpi_mbox_init();

	rc = mbox;
	if (mbox == 0 && CONFIG_YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT > CONFIG_YAKOGL_BOARD_RPI4_FB_HEIGHT) {
		uint32_t got[4] = { 0 };

		rc = rpi_fb_alloc_virtual(CONFIG_YAKOGL_BOARD_RPI4_FB_WIDTH, CONFIG_YAKOGL_BOARD_RPI4_FB_HEIGHT,
					  CONFIG_YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT, &s_fb, got);
		if (rc == 0) {
			s_fb_rows = CONFIG_YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT;
		} else {
			printk("board: rpi4 virtual framebuffer %ux%u refused (%d; granted %ux%u virtual "
			       "%ux%u): one screen, windows are copied\n",
			       CONFIG_YAKOGL_BOARD_RPI4_FB_WIDTH, CONFIG_YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT, rc,
			       got[0], got[1], got[2], got[3]);
		}
	}
	if (mbox == 0 && s_fb_rows == 0u) {
		rc = rpi_fb_alloc(CONFIG_YAKOGL_BOARD_RPI4_FB_WIDTH, CONFIG_YAKOGL_BOARD_RPI4_FB_HEIGHT, &s_fb);
		s_fb_rows = rc == 0 ? s_fb.height : 0u;
	}
	if (rc != 0) {
		printk("board: no framebuffer (%d): the UART only\n", rc);
		return 0;
	}
	s_have_fb = true;
	console_attach(s_fb.cpu, s_fb.width, s_fb.height);
	printk("board: rpi4 framebuffer %ux%u (%u rows in memory) pitch %u phys 0x%08x\n", s_fb.width,
	       s_fb.height, s_fb_rows, s_fb.pitch, s_fb.phys);
	uint32_t order = UINT32_MAX;

	s_format = yd_rpi4_window_format(s_fb.order_code, s_fb.order, &order);
	if (s_format != 0u) {
		printk("board: rpi4 framebuffer pixel order %s depth 32: windows are %s\n",
		       order == 1u ? "RGB" : "BGR", s_format == GLES_NATIVE_FORMAT_RGBA8888 ? "RGBA8888" : "XRGB8888");
	} else {
		printk("board: rpi4 framebuffer pixel order not answered (code 0x%08x value %u): no window\n",
		       s_fb.order_code, s_fb.order);
	}
	/* The V3D at its highest clock, as every Pi 4 sample before. */
	if (rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz) == 0 && max_hz != 0u &&
	    rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz != max_hz) {
		rc = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		printk("board: V3D clock %u -> %u Hz (%d)\n", hz, max_hz, rc);
	}
	return 0;
}
/* Before main(), after the kernel and the drivers. */
SYS_INIT(board_rpi4_init, APPLICATION, 0);

/* ---- the window and the screen -------------------------------------------- */

static uint8_t s_win[CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB * 1024] __noinit __aligned(4096);
static bool s_taken, s_console_moved;

static struct yd_rpi4_fb fb_view(void)
{
	return (struct yd_rpi4_fb){ .phys = s_fb.phys, .width = s_fb.width, .height = s_fb.height,
				    .pitch = s_fb.pitch, .rows = s_fb_rows };
}

/* The console beside the w x h window at the screen's (0,0), or under it,
 * or (no room: a window as wide as the screen and at least as tall) the
 * UART only. Once. */
static void console_beside(uint32_t w, uint32_t h)
{
	if (s_console_moved) {
		return;
	}
	s_console_moved = true;
	if (s_fb.width - w >= 160u) {
		console_attach(s_fb.cpu + (size_t)w * 4u, s_fb.width - w, s_fb.height);
	} else if (h < s_fb.height && s_fb.height - h >= 64u) {
		console_attach(s_fb.cpu + (size_t)h * s_fb.pitch, s_fb.width, s_fb.height - h);
	} else {
		s_fb_out = NULL;
	}
}

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	uint8_t *nc = NULL;
	size_t bytes = (size_t)w * h * 4u;
	const struct yd_rpi4_fb fb = fb_view();

	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u) {
		return -EINVAL;
	}
	if (s_taken) {
		return -ENOMEM;
	}
	if (s_have_fb && s_format == 0u) {
		return -ENODEV; /* the pixel order was not answered: no honest format */
	}
	/* SCANOUT_VIEW: the framebuffer itself, at (0,0) with its pitch, when the
	 * window fits its width and its rows in memory and the pitch is an even
	 * number of whole pixels (the GPUs write pixel pairs;
	 * board::rpi4::window_is_view); the console moves out of its way now,
	 * before the sample writes the window. The framebuffer is Normal
	 * non-cacheable (rpi_fb_alloc, rpi_fb_alloc_virtual). */
	if (IS_ENABLED(CONFIG_YAKOGL_BOARD_SCANOUT_VIEW) && s_have_fb &&
	    yd_rpi4_window_is_view(&fb, w, h) == 1) {
		console_beside(w, h);
		s_taken = true;
		s_view = true;
		*out = (struct board_window){
			.phys = s_fb.phys,
			.cpu = s_fb.cpu,
			.w = w,
			.h = h,
			.stride = s_fb.pitch,
			/* the firmware's pixel order; SCANOUT: the screen scans this
			 * buffer, so EGL must not double-buffer it (eglplatform.h) */
			.format = s_format | GLES_NATIVE_WINDOW_SCANOUT,
		};
		printk("board: the %ux%u window is a view into the framebuffer, pitch %u (no copy)\n", w, h,
		       s_fb.pitch);
		return 0;
	}
	if (bytes > sizeof(s_win)) {
		return -EINVAL;
	}
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
		.stride = w * 4u,
		/* copied byte for byte into the framebuffer: its pixel order too;
		 * XRGB8888 when there is no framebuffer to show it on */
		.format = s_have_fb ? s_format : GLES_NATIVE_FORMAT_XRGB8888,
	};
	return 0;
}

/* A view, or a part of it at a row boundary: the screen is pointed at its
 * first row (board::rpi4::pan_row). 1 when `win` is not such a part (the
 * caller copies), 0 when it is on the screen (no mailbox call when the
 * screen already starts there), -EIO when the firmware did not take the
 * pan (the screen stays where it was). */
static int show_view(const struct board_window *win)
{
	const struct yd_rpi4_fb fb = fb_view();
	uint32_t y = 0, echo = UINT32_MAX;

	if (!s_view || yd_rpi4_pan_row(&fb, win->phys, win->w, win->h, win->stride, &y) != 1) {
		return 1;
	}
	if (y == s_shown_y) {
		return 0;
	}
	if (rpi_fb_pan(y, &echo, NULL) != 0 || echo != y) {
		return -EIO;
	}
	s_shown_y = y;
	return 0;
}

int board_show(const struct board_window *win)
{
	board_first_frame();
	int rc;

	if (!s_have_fb) {
		return -ENODEV;
	}
	if (win->w > s_fb.width || win->h > s_fb.height ||
	    (win->format & GLES_NATIVE_WINDOW_BOTTOM_UP)) {
		return -ENOTSUP;
	}
	rc = show_view(win);
	if (rc <= 0) {
		return rc;
	}
	board_show_copied(win->w, win->h, s_view ? "not a part of the view" : "not a view");
	console_beside(win->w, win->h);
	for (uint32_t y = 0; y < win->h; y++) {
		memcpy(s_fb.cpu + (size_t)y * s_fb.pitch,
		       (const uint8_t *)win->cpu + (size_t)y * win->stride, (size_t)win->w * 4u);
	}
	return 0;
}

int board_selector(void)
{
	return -1;
}

void board_sink_store(const uint32_t w[4])
{
	ARG_UNUSED(w); /* the UART line and the ring are the sink */
}

/* The record screen: the whole framebuffer cleared, the last lines of the
 * run, the last yk-record line at the bottom; nothing draws after it. */
static void freeze_record_screen(void)
{
	uint32_t rows = s_fb.height / 16u; /* fbcon's 8x16 font at scale 1 */
	unsigned int n = s_ring_count;

	if (rows < 4u) {
		return;
	}
	if (n > rows - 3u) {
		n = rows - 3u;
	}
	s_ring_off = true; /* the replay must not overwrite the ring it reads */
	/* on the rows on screen (the last pan's) */
	console_attach(s_fb.cpu + (size_t)s_shown_y * s_fb.pitch, s_fb.width, s_fb.height);
	printk("board: record, %lld s after boot\n", (long long)(k_uptime_get() / 1000));
	for (unsigned int i = 0; i < n; i++) {
		printk("%s\n", s_ring[(s_ring_next + RING_LINES - n + i) % RING_LINES]);
	}
	printk("%s\n", s_last_record[0] ? s_last_record : "yk-record: none");
	s_frozen = true;
}

FUNC_NORETURN void board_reset(void)
{
	if (IS_ENABLED(CONFIG_YAKOGL_BOARD_RECORD_SCREEN) && s_have_fb) {
		freeze_record_screen();
	}
	printk("board: feeding stopped; the PM watchdog resets the SoC\n");
	board_wdog_release();
	for (;;) {
		k_sleep(K_FOREVER);
	}
}
