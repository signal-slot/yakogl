/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Raspberry Pi 5's screen through the BCM2712 property mailbox (see
 * board_rpi5.h; driver/docs/rpi5-display.md, route 2).
 *
 * One property message, the framebuffer group (raspberrypi-firmware.h): the
 * physical size 0x48003, the virtual size 0x48004, the depth 0x48005 = 32,
 * the pixel order 0x48006, the virtual offset 0x48009, ALLOCATE 0x40001 and
 * GET_PITCH 0x40008, through the Pi 4's transport (rpi4/rpi_mbox.c, built for
 * CONFIG_SOC_BCM2712). What differs from board_rpi4.c:
 *   - the size is the granted one, not the asked one: the firmware's answer
 *     (the sink's preferred mode may be 3840x2160, 32 MiB at 32 bpp) decides
 *     the screen, the pitch and the rows in memory;
 *   - the ALLOCATE address is kept as it is (yd_rpi_fb_from_answer, board 5:
 *     there is no 0xc0000000 alias to clear);
 *   - every answer is checked and a refusal is a named line: a depth other
 *     than 32, an unanswered tag, a pitch or size that does not hold the rows,
 *     a window the screen cannot hold;
 *   - one greppable line ("board: rpi5 fb answer") records what the firmware
 *     answered, the raw ALLOCATE address and size included.
 * The decisions (the pixel-order format, which windows are views, the pan row)
 * are the Pi 4's, from yakogl_drivers.h (src/board/rpi4.rs, rpi_fb.rs).
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <stddef.h>
#include <string.h>
#include <EGL/eglplatform.h>

#include "board_rpi5.h"
#include "rpi_mbox.h"
#include "fbcon.h"
#include <yakogl_drivers.h> /* yd_rpi_*, yd_rpi4_*: the framebuffer decisions */

#define RPI_BOARD 5 /* yd_rpi_*'s board number: the Pi 5 */

BUILD_ASSERT(sizeof(struct yd_rpi4_fb) == 24 && offsetof(struct yd_rpi4_fb, rows) == 20,
	     "src/ffi/board.rs pins rpi4::Fb");

static printk_hook_fn_t s_uart_out, s_fb_out;
static uint8_t *s_cpu;         /* the framebuffer, Normal non-cacheable */
static struct yd_rpi4_fb s_fb; /* phys 0: no screen */
static uint32_t s_format;      /* the windows' format (0: pixel order not answered) */
static bool s_taken, s_view, s_console_moved;
static uint32_t s_shown_y;     /* the row the screen starts at (the last pan) */

static int console_out(int c)
{
	if (s_uart_out) {
		s_uart_out(c);
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

/* Append a tag (id, value bytes, request code 0, values) to `w` at `*n`; the
 * index of its first value word (the layout of rpi_mbox_request_timed). */
static uint32_t fb_tag_add(uint32_t *w, uint32_t *n, uint32_t tag, uint32_t nval, const uint32_t *vals)
{
	w[(*n)++] = tag;
	w[(*n)++] = nval * 4u;
	w[(*n)++] = 0;
	uint32_t at = *n;

	for (uint32_t i = 0; i < nval; i++) {
		w[(*n)++] = vals ? vals[i] : 0u;
	}
	return at;
}

static bool answered(const uint32_t *w, uint32_t at)
{
	return (w[at - 1u] & 0x80000000u) != 0u;
}

/*
 * Ask for a width x height screen with `rows` rows in memory and build the
 * screen from the answer. 0 and s_fb, s_cpu, s_format set, or a negative errno
 * with the reason printed (nothing set).
 */
static int screen_alloc(uint32_t width, uint32_t height, uint32_t rows)
{
	uint32_t t[48], n = 0;
	const uint32_t phys[2] = { width, height }, virt[2] = { width, rows }, depth = 32u, order = 1u,
		       off[2] = { 0, 0 }, align[2] = { 4096u, 0 };
	const uint32_t a_phys = fb_tag_add(t, &n, 0x00048003, 2, phys);    /* SET_PHYSICAL_WIDTH_HEIGHT */
	const uint32_t a_virt = fb_tag_add(t, &n, 0x00048004, 2, virt);    /* SET_VIRTUAL_WIDTH_HEIGHT */
	const uint32_t a_depth = fb_tag_add(t, &n, 0x00048005, 1, &depth); /* SET_DEPTH */
	const uint32_t a_order = fb_tag_add(t, &n, 0x00048006, 1, &order); /* SET_PIXEL_ORDER: RGB */
	const uint32_t a_off = fb_tag_add(t, &n, 0x00048009, 2, off);      /* SET_VIRTUAL_OFFSET */
	const uint32_t a_alloc = fb_tag_add(t, &n, 0x00040001, 2, align);  /* ALLOCATE */
	const uint32_t a_pitch = fb_tag_add(t, &n, 0x00040008, 1, NULL);   /* GET_PITCH */
	const int rc = rpi_mbox_request_timed(t, n, NULL);

	if (rc != 0) {
		printk("board: rpi5 framebuffer request failed (%d): no screen\n", rc);
		return rc;
	}
	const uint32_t bus = t[a_alloc], size = t[a_alloc + 1u], pitch = t[a_pitch], dep = t[a_depth];
	const uint32_t pw = t[a_phys], ph = t[a_phys + 1u], vw = t[a_virt], vh = t[a_virt + 1u];

	/* What a run records: grep "board: rpi5 fb answer". */
	printk("board: rpi5 fb answer allocate 0x%08x size %u pitch %u depth %u order code 0x%08x value %u "
	       "physical %ux%u virtual %ux%u asked %ux%u rows %u\n",
	       bus, size, pitch, dep, t[a_order - 1u], t[a_order], pw, ph, vw, vh, width, height, rows);

	const struct {
		uint32_t at;
		const char *name;
	} must[] = {
		{ a_phys, "SET_PHYSICAL_WIDTH_HEIGHT" }, { a_virt, "SET_VIRTUAL_WIDTH_HEIGHT" },
		{ a_depth, "SET_DEPTH" },                { a_off, "SET_VIRTUAL_OFFSET" },
		{ a_alloc, "ALLOCATE" },                 { a_pitch, "GET_PITCH" },
	};

	for (size_t i = 0; i < ARRAY_SIZE(must); i++) {
		if (!answered(t, must[i].at)) {
			printk("board: rpi5 framebuffer refused: %s was not answered (code 0x%08x): no screen\n",
			       must[i].name, t[must[i].at - 1u]);
			return -EIO;
		}
	}
	if (bus == 0u || size == 0u) {
		printk("board: rpi5 framebuffer refused: ALLOCATE gave address 0x%08x size %u: no screen\n", bus,
		       size);
		return -EIO;
	}
	if (dep != 32u) {
		printk("board: rpi5 framebuffer refused: depth %u is not 32 bpp: no screen\n", dep);
		return -ERANGE;
	}
	if (vw != pw) {
		printk("board: rpi5 framebuffer refused: virtual width %u is not the physical width %u: no "
		       "screen\n",
		       vw, pw);
		return -ERANGE;
	}
	if (yd_rpi_allocate_ok(pw, vh, pitch, size) != 1) {
		printk("board: rpi5 framebuffer refused: pitch %u size %u do not hold %u rows of %u 32 bpp "
		       "pixels: no screen\n",
		       pitch, size, vh, pw);
		return -EIO;
	}
	struct yd_rpi4_fb fb;

	if (yd_rpi_fb_from_answer(RPI_BOARD, bus, pw, ph, vh, pitch, size, &fb) != 1) {
		printk("board: rpi5 framebuffer refused: %u rows in memory under the %u of the screen: no "
		       "screen\n",
		       vh, ph);
		return -EIO;
	}
	mm_reg_t va = 0;

	device_map(&va, (uintptr_t)fb.phys, size, K_MEM_ARM_NORMAL_NC);
	if (va == 0) {
		printk("board: rpi5 framebuffer refused: mapping 0x%08x + %u failed: no screen\n", bus, size);
		return -ENOMEM;
	}
	s_fb = fb;
	s_cpu = (uint8_t *)va;
	uint32_t ord = UINT32_MAX;

	s_format = yd_rpi4_window_format(t[a_order - 1u], t[a_order], &ord);
	return 0;
}

static int board_rpi5_init(void)
{
	s_uart_out = __printk_get_hook();
	__printk_hook_install(console_out);
	int rc = rpi_mbox_init();

	if (rc == 0) {
		const uint32_t rows = CONFIG_YAKOGL_BOARD_RPI5_FB_VIRTUAL_HEIGHT > CONFIG_YAKOGL_BOARD_RPI5_FB_HEIGHT ?
					      CONFIG_YAKOGL_BOARD_RPI5_FB_VIRTUAL_HEIGHT :
					      CONFIG_YAKOGL_BOARD_RPI5_FB_HEIGHT;

		rc = screen_alloc(CONFIG_YAKOGL_BOARD_RPI5_FB_WIDTH, CONFIG_YAKOGL_BOARD_RPI5_FB_HEIGHT, rows);
	}
	if (rc != 0) {
		printk("board: rpi5 no framebuffer (%d): the UART only\n", rc);
		return 0;
	}
	console_attach(s_cpu, s_fb.width, s_fb.height);
	printk("board: rpi5 framebuffer %ux%u (%u rows in memory) pitch %u phys 0x%08x\n", s_fb.width,
	       s_fb.height, s_fb.rows, s_fb.pitch, (uint32_t)s_fb.phys);
	if (s_format != 0u) {
		printk("board: rpi5 framebuffer pixel order %s depth 32: windows are %s\n",
		       s_format == GLES_NATIVE_FORMAT_RGBA8888 ? "RGB" : "BGR",
		       s_format == GLES_NATIVE_FORMAT_RGBA8888 ? "RGBA8888" : "XRGB8888");
	} else {
		printk("board: rpi5 framebuffer pixel order not answered: no window\n");
	}
	return 0;
}
/* Before main(), after the kernel and the drivers, as the Pi 4's. */
SYS_INIT(board_rpi5_init, APPLICATION, 0);

bool rpi5_screen_ready(void)
{
	return s_fb.phys != 0u;
}

void rpi5_screen_info(uint32_t *width, uint32_t *height, uint32_t *rows, uint32_t *pitch)
{
	if (width) {
		*width = s_fb.width;
	}
	if (height) {
		*height = s_fb.height;
	}
	if (rows) {
		*rows = s_fb.rows;
	}
	if (pitch) {
		*pitch = s_fb.pitch;
	}
}

/* The console beside the w x h window at the screen's (0,0), or under it, or
 * (no room: a window as wide as the screen and at least as tall) the UART
 * only. As the Pi 4's. Once. */
static void console_beside(uint32_t w, uint32_t h)
{
	if (s_console_moved) {
		return;
	}
	s_console_moved = true;
	if (s_fb.width - w >= 160u) {
		console_attach(s_cpu + (size_t)w * 4u, s_fb.width - w, s_fb.height);
	} else if (h < s_fb.height && s_fb.height - h >= 64u) {
		console_attach(s_cpu + (size_t)h * s_fb.pitch, s_fb.width, s_fb.height - h);
	} else {
		s_fb_out = NULL;
	}
}

int rpi5_window(uint32_t w, uint32_t h, struct board_window *out)
{
	memset(out, 0, sizeof(*out));
	if (w == 0u || h == 0u) {
		return -EINVAL;
	}
	if (!rpi5_screen_ready()) {
		return -ENODEV;
	}
	if (s_taken) {
		return -ENOMEM;
	}
	if (s_format == 0u) {
		printk("board: rpi5 window %ux%u refused: the pixel order was not answered\n", w, h);
		return -ENODEV;
	}
	if (w > s_fb.width || h > s_fb.rows) {
		printk("board: rpi5 window %ux%u refused: larger than the granted screen %ux%u (%u rows in "
		       "memory)\n",
		       w, h, s_fb.width, s_fb.height, s_fb.rows);
		return -EFBIG;
	}
	if (yd_rpi4_window_is_view(&s_fb, w, h) != 1) {
		printk("board: rpi5 window %ux%u refused: not a view into the framebuffer (pitch %u)\n", w, h,
		       s_fb.pitch);
		return -EINVAL;
	}
	console_beside(w, h);
	s_taken = true;
	s_view = true;
	*out = (struct board_window){
		.phys = (uintptr_t)s_fb.phys,
		.cpu = s_cpu,
		.w = w,
		.h = h,
		.stride = s_fb.pitch,
		/* the firmware's pixel order; SCANOUT: the screen scans this buffer,
		 * so EGL must not double-buffer it (eglplatform.h) */
		.format = s_format | GLES_NATIVE_WINDOW_SCANOUT,
	};
	printk("board: rpi5 the %ux%u window is a view into the framebuffer, pitch %u (no copy)\n", w, h,
	       s_fb.pitch);
	return 0;
}

int rpi5_show(const struct board_window *win)
{
	uint32_t y = 0, echo = UINT32_MAX;

	if (!rpi5_screen_ready()) {
		return -ENODEV;
	}
	if (!s_view || yd_rpi4_pan_row(&s_fb, win->phys, win->w, win->h, win->stride, &y) != 1) {
		printk("board: rpi5 show of %ux%u refused: not a part of the view\n", win->w, win->h);
		return -EINVAL;
	}
	if (y == s_shown_y) {
		return 0;
	}
	if (rpi_fb_pan(y, &echo, NULL) != 0 || echo != y) {
		printk("board: rpi5 pan to row %u not taken (echo %u)\n", y, echo);
		return -EIO;
	}
	s_shown_y = y;
	return 0;
}
