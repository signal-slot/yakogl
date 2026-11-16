/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_fmt: the RGB pixel formats of a BCM2712 HVS (scaler6) plane that the
 * Pi 5 screen cares about, mapped to the board layer's window format
 * (GLES_NATIVE_FORMAT_*, src/gl/egl.rs:47-51; yd_rpi4_window_format in
 * driver/rust/include/yakogl_drivers.h:178 returns the same values).
 * Header-only C11, no Zephyr, no other header: the numbers are mirrored here
 * and the mock (driver/zephyr/lib/board/test/hvs6_fmt_mock.c) pins them.
 * Built by tools/v3d/check71.d/hvs6_fmt.sh.
 *
 * Linux is GPL: only facts and file:line; "rpi:" is raspberrypi/linux
 * rpi-6.12.y at 5bd564aa (driver/docs/rpi5-v3d71-facts.md, header table).
 *
 * Plane control word 0: PIXEL_FORMAT is bits 4:0 (rpi:vc4_regs.h:1377) and
 * ORDERRGBA bits 14:13 (rpi:vc4_regs.h:1374).
 *
 * Format codes, enum hvs_pixel_format (rpi:vc4_regs.h:1058-1091):
 *   RGB565 4 (:1065)   RGB888 5 (:1067)   RGBA8888 7 (:1070)
 * Orders of the 32-bit word, HVS5 numbering (rpi:vc4_regs.h:1097-1100):
 *   RGBA 0, BGRA 1, ARGB 2, ABGR 3.
 * Linux uses code RGBA8888 with order ARGB for XRGB8888 and ARGB8888 and
 * with order ABGR for XBGR8888 (the table at rpi:vc4_plane.c:43-65, as
 * quoted at hvs6_dlist.h:196-198). The order names read from the most
 * significant byte of the little-endian 32-bit word, so the other two
 * orders (RGBA, BGRA) are the DRM RGBA8888 and BGRA8888 layouts: derived
 * from those two cited points, not read from the table.
 *
 * Open (the Linux tree was not reachable from this session, so nothing
 * here was re-read): the order Linux writes for RGB565 and RGB888. RGB565
 * is paired with order RGBA as hvs6_dlist_write.h does (that writer's
 * choice, not a citation); RGB888 is listed in every order, all
 * "unsupported" (the board layer has no 24 bpp window format).
 *
 * X versus A: the plane words of XRGB8888 and ARGB8888 are the same (alpha
 * is used or ignored by the blend mode elsewhere in the list), so
 * hvs6_fmt_lookup returns the X entry (no alpha) and the A entry is only
 * reachable through hvs6_fmt_at.
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_HVS6_FMT_H
#define YAKOGL_BOARD_RPI5_SCANOUT_HVS6_FMT_H

#include <stddef.h>
#include <stdint.h>

/* Codes and orders as above; named HVS6F_ so the header stands alone. */
#define HVS6F_CODE_RGB565 4u
#define HVS6F_CODE_RGB888 5u
#define HVS6F_CODE_RGBA8888 7u
#define HVS6F_ORDER_RGBA 0u
#define HVS6F_ORDER_BGRA 1u
#define HVS6F_ORDER_ARGB 2u
#define HVS6F_ORDER_ABGR 3u

/*
 * Window formats, GLES_NATIVE_FORMAT_* (src/gl/egl.rs:47-51):
 * RGBA8888 1 = R,G,B,A in memory; XRGB8888 2 = B,G,R,X in memory;
 * RGB565 3. 0: the board layer has no window format for it.
 */
#define HVS6F_WIN_UNSUPPORTED 0u
#define HVS6F_WIN_RGBA8888 1u
#define HVS6F_WIN_XRGB8888 2u
#define HVS6F_WIN_RGB565 3u

struct hvs6_fmt {
	const char *name; /* DRM-style name of the layout */
	uint8_t code;     /* PIXEL_FORMAT */
	uint8_t order;    /* ORDERRGBA */
	uint8_t bpp;      /* bytes per pixel */
	uint8_t alpha;    /* 1: the layout has an alpha channel */
	uint8_t window;   /* HVS6F_WIN_* */
};

/* X variants first, so a lookup by (code, order) finds the no-alpha one. */
static const struct hvs6_fmt hvs6_fmt_table[] = {
	{"XRGB8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_ARGB, 4, 0, HVS6F_WIN_XRGB8888},
	{"XBGR8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_ABGR, 4, 0, HVS6F_WIN_RGBA8888},
	{"RGBX8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_RGBA, 4, 0, HVS6F_WIN_UNSUPPORTED},
	{"BGRX8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_BGRA, 4, 0, HVS6F_WIN_UNSUPPORTED},
	{"ARGB8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_ARGB, 4, 1, HVS6F_WIN_XRGB8888},
	{"ABGR8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_ABGR, 4, 1, HVS6F_WIN_RGBA8888},
	{"RGBA8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_RGBA, 4, 1, HVS6F_WIN_UNSUPPORTED},
	{"BGRA8888", HVS6F_CODE_RGBA8888, HVS6F_ORDER_BGRA, 4, 1, HVS6F_WIN_UNSUPPORTED},
	{"RGB565", HVS6F_CODE_RGB565, HVS6F_ORDER_RGBA, 2, 0, HVS6F_WIN_RGB565},
	{"RGB888/RGBA", HVS6F_CODE_RGB888, HVS6F_ORDER_RGBA, 3, 0, HVS6F_WIN_UNSUPPORTED},
	{"RGB888/BGRA", HVS6F_CODE_RGB888, HVS6F_ORDER_BGRA, 3, 0, HVS6F_WIN_UNSUPPORTED},
	{"RGB888/ARGB", HVS6F_CODE_RGB888, HVS6F_ORDER_ARGB, 3, 0, HVS6F_WIN_UNSUPPORTED},
	{"RGB888/ABGR", HVS6F_CODE_RGB888, HVS6F_ORDER_ABGR, 3, 0, HVS6F_WIN_UNSUPPORTED},
};

#define HVS6F_COUNT (sizeof(hvs6_fmt_table) / sizeof(hvs6_fmt_table[0]))

/* Entry i, NULL past the table. */
static inline const struct hvs6_fmt *hvs6_fmt_at(size_t i)
{
	return i < HVS6F_COUNT ? &hvs6_fmt_table[i] : NULL;
}

/* The entry of a PIXEL_FORMAT code and ORDERRGBA pair, NULL if unlisted. */
static inline const struct hvs6_fmt *hvs6_fmt_lookup(uint32_t code, uint32_t order)
{
	for (size_t i = 0; i < HVS6F_COUNT; i++)
		if (hvs6_fmt_table[i].code == code && hvs6_fmt_table[i].order == order)
			return &hvs6_fmt_table[i];
	return NULL;
}

/*
 * The inverse: the scanout entry for a window format, NULL if none. XRGB8888
 * gives XRGB8888 (code 7, order ARGB), RGBA8888 (R,G,B,A in memory) gives
 * XBGR8888 (code 7, order ABGR), RGB565 gives RGB565. The first entry with
 * the window value wins, the no-alpha one: a scanout plane need not carry
 * alpha.
 */
static inline const struct hvs6_fmt *hvs6_fmt_for_window(uint32_t fmt)
{
	if (fmt == HVS6F_WIN_UNSUPPORTED)
		return NULL;
	for (size_t i = 0; i < HVS6F_COUNT; i++)
		if (hvs6_fmt_table[i].window == fmt)
			return &hvs6_fmt_table[i];
	return NULL;
}

#endif
