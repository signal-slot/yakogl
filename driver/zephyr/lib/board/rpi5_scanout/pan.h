/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pan.h: window view panning of the Pi 5 scanout by rewriting the plane's
 * pointer word in the HVS6 display list (routes 1 and 3), instead of a
 * mailbox call as on the Pi 4 (yd_rpi4_pan_row). Header-only C11.
 *
 * The display-list pointer word is a 32-bit bus address; the caller gives
 * the bus address of row 0 and the index of the pointer word in the list
 * (no dependency on hvs6_dlist.h or bus_addr.h).
 *
 * Alignment: PAN_PTR_ALIGN = 16 bytes is UNVERIFIED. driver/docs/rpi5-v3d71-
 * facts.md section 10 does not state the HVS6 pointer alignment, and no
 * vc4_plane.c scaler6 fact was available to confirm it; 16 is the
 * conservative choice (the HVS fetches in whole 16-byte units). The board
 * check in the display session decides; change PAN_PTR_ALIGN only then.
 */
#ifndef YAKOGL_RPI5_PAN_H
#define YAKOGL_RPI5_PAN_H

#include <stdbool.h>
#include <stdint.h>

#define PAN_PTR_ALIGN 16u /* unverified, see above */

enum pan_status {
	PAN_OK = 0,
	PAN_ERR_ARG = -1,       /* zero pitch, bad bpp, view taller than surface */
	PAN_ERR_ROW = -2,       /* the view would run past the surface */
	PAN_ERR_ALIGN = -3,     /* pointer or pitch not aligned */
	PAN_ERR_RANGE = -4,     /* the pointer does not fit 32 bits */
	PAN_ERR_VBLANK = -5,    /* not in vblank, nothing written */
	PAN_ERR_WRITER = -6,    /* no writer or no predicate */
};

/*
 * New pointer value for the view whose first line is `row` and first column
 * `x` of a surface at base_bus with `pitch` bytes per line, surface_rows lines
 * tall, shown through a view_rows-line window. bpp is bytes per pixel (2 or 4).
 * The last valid row is surface_rows - view_rows.
 */
static inline int pan_pointer(uint64_t base_bus, uint32_t pitch, uint32_t row, uint32_t bpp,
			      uint32_t x, uint32_t surface_rows, uint32_t view_rows,
			      uint32_t *out)
{
	uint64_t ptr;

	if (!out || pitch == 0 || (bpp != 2 && bpp != 4) || view_rows == 0 ||
	    view_rows > surface_rows || (uint64_t)x * bpp >= pitch)
		return PAN_ERR_ARG;
	if (row > surface_rows - view_rows)
		return PAN_ERR_ROW;
	ptr = base_bus + (uint64_t)row * pitch + (uint64_t)x * bpp;
	if ((ptr & (PAN_PTR_ALIGN - 1)) || (pitch & (PAN_PTR_ALIGN - 1)))
		return PAN_ERR_ALIGN;
	if (ptr > 0xffffffffull)
		return PAN_ERR_RANGE;
	*out = (uint32_t)ptr;
	return PAN_OK;
}

typedef void (*pan_write_fn)(void *ctx, uint32_t index, uint32_t value);
typedef bool (*pan_vblank_fn)(void *ctx);

/*
 * Write one word at display-list index `index`, only while in_vblank()
 * is true (a write mid-frame tears). Returns PAN_ERR_VBLANK without writing
 * otherwise; the caller retries after the next vblank.
 */
static inline int pan_write(pan_write_fn write, pan_vblank_fn in_vblank, void *ctx,
			    uint32_t index, uint32_t value)
{
	if (!write || !in_vblank)
		return PAN_ERR_WRITER;
	if (!in_vblank(ctx))
		return PAN_ERR_VBLANK;
	write(ctx, index, value);
	return PAN_OK;
}

#endif
