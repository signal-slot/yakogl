/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_dlist_write: a writer for a BCM2712 HVS (scaler6) display list of one
 * unscaled, unrotated, linear plane followed by the end-of-list word. The
 * inverse of hvs6_dlist_first_plane: the field positions are the decoder
 * header's (hvs6_dlist.h, which holds the rpi:<file>:<line> citations of
 * rpi-6.12.y commit 5bd564aa under drivers/gpu/drm/vc4/); none is redefined
 * here.
 *
 * Header only, no Zephyr, no MMIO: it fills a caller-supplied uint32_t buffer
 * and the caller copies it to the display-list memory (HVS base +
 * HVS6_DLIST_OFFSET, from word HVS6_BOOTLOADER_DLIST_END on).
 *
 * Layout: Linux's for one unscaled plane (rpi:vc4_plane.c:1899-2300), a
 * 9-word entry and then the list's own END word, 10 words in all:
 *   0 CTL0  VALID | NEXT=9 | UNITY | ORDERRGBA | PIXEL_FORMAT, linear; the
 *           alpha-mask field (bits 19:18) is 3 FIXED for a format without
 *           alpha (rpi:vc4_plane.c:1333-1346, rpi:vc4_regs.h:1370-1372)
 *   1 POS0  START_Y, START_X
 *   2 CTL2  alpha in bits 15:4 = alpha >> 4, 0xfff0 opaque
 *           (rpi:vc4_plane.c:2161-2166, rpi:vc4_regs.h:1383-1389)
 *   3 POS2  SRC_LINES-1, SRC_WIDTH-1
 *   4 POS3  context word 0xc0c0c0c0 (rpi:vc4_plane.c:2184-2185)
 *   5 PTR0  address bits 39:32
 *   6 PTR1  address bits 31:0
 *   7 PTR2  PITCH in bytes
 *   8 END   inside the entry, counted by NEXT (rpi:vc4_plane.c:2296-2299)
 *   9 END   the list's end word, written by the CRTC flush
 *           (rpi:vc4_hvs.c:1316, counted at :1138)
 * No POS1 word (unity), so no scaling; no flip, no tiling.
 *
 * Open board question: Linux also sets UPM bits in PTR0 (base, handle, size;
 * rpi:vc4_plane.c:1124-1197). They are not written here; the board will show
 * whether a raw RGB plane needs them. The older 8-word form (NEXT=8, CTL2=0,
 * one END) is not kept.
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_WRITE_H
#define YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_WRITE_H

#include <stddef.h>
#include <stdint.h>

#include "hvs6_dlist.h"

enum hvs6w_format {
	HVS6W_XRGB8888 = 1, /* RGBA8888 code, order ARGB */
	HVS6W_ARGB8888,     /* same words as XRGB8888 */
	HVS6W_RGB565,       /* RGB565 code, order RGBA */
};

enum hvs6w_err {
	HVS6W_ERR_BUFFER = -1, /* out is NULL or too short for the list */
	HVS6W_ERR_FORMAT = -2, /* not one of enum hvs6w_format */
	HVS6W_ERR_SIZE = -3,   /* width or height 0 or over 8192 */
	HVS6W_ERR_POS = -4,    /* x or y over 13 bits */
	HVS6W_ERR_PITCH = -5,  /* pitch under a row, over 17 bits, or unaligned */
	HVS6W_ERR_ADDR = -6,   /* address over 40 bits */
};

/* Words of the plane entry (NEXT, its inner END included) and of a written
 * list: the entry and the list END word. */
#define HVS6W_ENTRY_WORDS 9u
#define HVS6W_LIST_WORDS (HVS6W_ENTRY_WORDS + 1u)
/* CTL0 alpha-mask field, bits 19:18; FIXED is 3 (rpi:vc4_regs.h:1370-1372). */
#define HVS6W_CTL0_ALPHA_MASK_SHIFT 18
#define HVS6W_CTL0_ALPHA_MASK_FIXED 3u
/* CTL2 of an opaque plane: alpha 0xfff in bits 15:4. */
#define HVS6W_CTL2_OPAQUE 0xfff0u
/* The POS3 context word Linux writes. */
#define HVS6W_POS3_CONTEXT 0xc0c0c0c0u

struct hvs6w_plane {
	enum hvs6w_format format;
	uint32_t x, y;          /* position on the screen */
	uint32_t width, height; /* source size = screen size (unscaled) */
	uint32_t pitch;         /* bytes per row; a multiple of 4 */
	uint64_t addr;          /* 40-bit fetch address */
};

/* Bytes per pixel of a format, 0 if it is none. */
static inline uint32_t hvs6w_bpp(enum hvs6w_format f)
{
	switch (f) {
	case HVS6W_XRGB8888:
	case HVS6W_ARGB8888:
		return 4;
	case HVS6W_RGB565:
		return 2;
	}
	return 0;
}

/*
 * Write the entry and the END word at out[0..HVS6W_LIST_WORDS) (out_words is
 * the room). Returns the word count, or an hvs6w_err; on an error out is not
 * touched. The decoder's ADDR_MODE 0 (linear) comes from the zeroed field.
 */
static inline int hvs6w_write_plane(uint32_t *out, size_t out_words, const struct hvs6w_plane *p)
{
	uint32_t code, order, bpp;

	if (out == NULL || p == NULL || out_words < HVS6W_LIST_WORDS)
		return HVS6W_ERR_BUFFER;
	bpp = hvs6w_bpp(p->format);
	if (bpp == 0)
		return HVS6W_ERR_FORMAT;
	if (p->width == 0 || p->height == 0 || p->width > HVS6_POS2_WIDTH_MASK + 1u ||
	    p->height > HVS6_POS2_LINES_MASK + 1u)
		return HVS6W_ERR_SIZE;
	if (p->x > HVS6_POS0_X_MASK || p->y > HVS6_POS0_Y_MASK)
		return HVS6W_ERR_POS;
	if (p->pitch < p->width * bpp || p->pitch > HVS6_PTR2_PITCH_MASK || (p->pitch & 3u))
		return HVS6W_ERR_PITCH;
	if (p->addr >> 40)
		return HVS6W_ERR_ADDR;

	if (p->format == HVS6W_RGB565) {
		code = HVS6_FMT_RGB565;
		order = HVS6_ORDER_RGBA;
	} else {
		code = HVS6_FMT_RGBA8888;
		order = HVS6_ORDER_ARGB;
	}
	out[0] = HVS6_CTL0_VALID | (HVS6W_ENTRY_WORDS << HVS6_CTL0_NEXT_SHIFT) | HVS6_CTL0_UNITY |
		 (HVS6W_CTL0_ALPHA_MASK_FIXED << HVS6W_CTL0_ALPHA_MASK_SHIFT) |
		 (order << HVS6_CTL0_ORDER_SHIFT) | code;
	out[1] = (p->y << HVS6_POS0_Y_SHIFT) | p->x;
	out[2] = HVS6W_CTL2_OPAQUE;
	out[3] = ((p->height - 1u) << HVS6_POS2_LINES_SHIFT) | (p->width - 1u);
	out[4] = HVS6W_POS3_CONTEXT;
	out[5] = (uint32_t)(p->addr >> 32) & HVS6_PTR0_UPPER_MASK;
	out[6] = (uint32_t)p->addr;
	out[7] = p->pitch;
	out[8] = HVS6_CTL0_END;
	out[9] = HVS6_CTL0_END;
	return (int)HVS6W_LIST_WORDS;
}

#endif /* YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_WRITE_H */
