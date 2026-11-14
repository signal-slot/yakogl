/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_dlist: a decoder for the BCM2712 HVS (the 6.x "scaler6", the
 * brcm,bcm2712-hvs block at ARM 0x10_7c580000, size 0x1a000; see
 * driver/docs/rpi5-v3d71-facts.md section 10) as a firmware scanout leaves
 * it: the channel's enable and status, where its display list starts, and
 * the first plane of that list (buffer address, pitch, format, size and
 * position).
 *
 * Header only, no MMIO: the caller maps the block, reads the words and
 * passes them in (the channel registers as plain values, the display-list
 * memory as a uint32_t array). Nothing here writes a register.
 *
 * Facts only, from the Raspberry Pi kernel, rpi-6.12.y at commit
 * 5bd564aaa8dc4b1b8ed11733c19ec8ece63a72cc (the commit the facts file pins).
 * Citations are rpi:<file>:<line> under drivers/gpu/drm/vc4/. The code is
 * written from those facts, not taken from the (GPL) driver.
 *
 * Two generations share the dlist word layout but not the channel register
 * offsets: C (BCM2712 C0/C1 steppings) and D (D0). The VERSION register
 * tells them apart (hvs6_gen_from_version).
 *
 * The plane layout decoded is the one Linux writes for a single-plane
 * linear (raster) buffer; whether the firmware writes the same words is the
 * board question this decoder helps to answer. The address is what the HVS
 * fetches from (a DMA address; the HVS sits behind iommu4 in the device
 * tree, rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:612-623, so with that
 * IOMMU enabled it is an IOVA, not an ARM physical address).
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_H
#define YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* ---- block-level registers (byte offsets from the HVS base) ------------- */

/* VERSION, the same offset on C and D: rpi:vc4_regs.h:543 and :676. */
#define HVS6_VERSION 0x0000u
/* VERSION's field, bits 7:0: rpi:vc4_regs.h:544. */
#define HVS6_VERSION_MASK 0xffu
/* VERSION values: C0 0x53, D0 0x54 (rpi:vc4_regs.h:545-546); Linux starts
 * from 6_C for brcm,bcm2712-hvs and switches to 6_D on 0x54
 * (rpi:vc4_hvs.c:2083-2090). */
#define HVS6_VERSION_C0 0x53u
#define HVS6_VERSION_D0 0x54u
/* CXM_SIZE, the same offset on C and D (rpi:vc4_regs.h:547 and :677): Linux
 * uses its value as the display-list memory size in words
 * (rpi:vc4_hvs.c:1588-1590, then rpi:vc4_hvs.c:1605-1607). */
#define HVS6_CXM_SIZE 0x0004u

/* The display-list memory starts at SCALER5_DLIST_START from the HVS base
 * for every generation from 5 on (rpi:vc4_regs.h:541,
 * rpi:vc4_hvs.c:2150-2151). Indexes into it are 32-bit words. */
#define HVS6_DLIST_OFFSET 0x4000u
/* Linux allocates its lists from word 32 on and never overwrites words
 * 0..31, "the setup from the bootloader" (rpi:vc4_regs.h:1056,
 * rpi:vc4_hvs.c:1579-1583 for 6_C and 6_D). */
#define HVS6_BOOTLOADER_DLIST_END 32u

/* ---- per-channel registers ---------------------------------------------- */

/* Channels 0..2 (SCALER_CHANNELS_COUNT, rpi:vc4_regs.h:238). */
#define HVS6_CHANNELS 3u

/* Generation C: DISP0_CTRL0 0x30, LPTRS 0x3c, STATUS 0x44, DL 0x48
 * (rpi:vc4_regs.h:564, :587, :600, :614); channel stride
 * DISP1_CTRL0 - DISP0_CTRL0 = 0x50 - 0x30 (rpi:vc4_regs.h:622,
 * the stride rule at :565-567). */
#define HVS6C_DISP0_CTRL0 0x0030u
#define HVS6C_DISP0_LPTRS 0x003cu
#define HVS6C_DISP0_STATUS 0x0044u
#define HVS6C_DISP0_DL 0x0048u
#define HVS6C_DISP_STRIDE 0x0020u
/* Generation D: DISP0_CTRL0 0x100, LPTRS 0x110, STATUS 0x118, DL 0x11c
 * (rpi:vc4_regs.h:707, :711, :713, :714); stride 0x140 - 0x100
 * (rpi:vc4_regs.h:718, the stride rule at :565-567). */
#define HVS6D_DISP0_CTRL0 0x0100u
#define HVS6D_DISP0_LPTRS 0x0110u
#define HVS6D_DISP0_STATUS 0x0118u
#define HVS6D_DISP0_DL 0x011cu
#define HVS6D_DISP_STRIDE 0x0040u

/* DISPx_CTRL0: ENB bit 31 (rpi:vc4_regs.h:568; Linux's "is this FIFO
 * enabled" test, rpi:vc4_hvs.c:739), RESET bit 30 (:569), FWIDTH 28:16
 * (:570), ONESHOT bit 15 (:571), LINES 12:0 (:573). */
#define HVS6_CTRL0_ENB (1u << 31)
#define HVS6_CTRL0_RESET (1u << 30)
#define HVS6_CTRL0_FWIDTH_SHIFT 16
#define HVS6_CTRL0_FWIDTH_MASK 0x1fffu
#define HVS6_CTRL0_ONESHOT (1u << 15)
#define HVS6_CTRL0_LINES_MASK 0x1fffu
/* DISPx_LPTRS: HEADE 11:0, the list start Linux writes
 * (rpi:vc4_regs.h:591, written at rpi:vc4_hvs.c:1166-1169). */
#define HVS6_LPTRS_HEADE_MASK 0xfffu
/* DISPx_STATUS: EMPTY bit 22 (rpi:vc4_regs.h:604), FRCNT 21:16 (:605),
 * MODE 14:13 (:607) with DISABLED 0, INIT 1, RUN 2, EOF 3 (:608-611),
 * YLINE 12:0 (:612). Linux's debugfs treats DISABLED and EOF as "disabled"
 * (rpi:vc4_hvs.c:297-303). */
#define HVS6_STATUS_EMPTY (1u << 22)
#define HVS6_STATUS_FRCNT_SHIFT 16
#define HVS6_STATUS_FRCNT_MASK 0x3fu
#define HVS6_STATUS_MODE_SHIFT 13
#define HVS6_STATUS_MODE_MASK 0x3u
#define HVS6_STATUS_MODE_DISABLED 0u
#define HVS6_STATUS_MODE_INIT 1u
#define HVS6_STATUS_MODE_RUN 2u
#define HVS6_STATUS_MODE_EOF 3u
#define HVS6_STATUS_YLINE_MASK 0x1fffu
/* DISPx_DL: LACT 11:0, the list the channel is running; Linux's debugfs
 * walks the list from there (rpi:vc4_regs.h:619, rpi:vc4_hvs.c:307-311). */
#define HVS6_DL_LACT_MASK 0xfffu

/* ---- display-list words of a plane --------------------------------------- */

/* Control word 0: END bit 31 (rpi:vc4_regs.h:1359), VALID bit 30 (:1360),
 * NEXT 29:24, the entry's length in words (:1361; set to the entry's word
 * count at rpi:vc4_plane.c:2298-2299), ADDR_MODE 22:20 with LINEAR 0
 * (:1363-1364), UNITY bit 15 (:1373), ORDERRGBA 14:13 (:1374),
 * PIXEL_FORMAT 4:0 (:1377). Written at rpi:vc4_plane.c:2143-2152. */
#define HVS6_CTL0_END (1u << 31)
#define HVS6_CTL0_VALID (1u << 30)
#define HVS6_CTL0_NEXT_SHIFT 24
#define HVS6_CTL0_NEXT_MASK 0x3fu
#define HVS6_CTL0_ADDR_MODE_SHIFT 20
#define HVS6_CTL0_ADDR_MODE_MASK 0x7u
#define HVS6_CTL0_ADDR_MODE_LINEAR 0u
#define HVS6_CTL0_UNITY (1u << 15)
#define HVS6_CTL0_ORDER_SHIFT 13
#define HVS6_CTL0_ORDER_MASK 0x3u
#define HVS6_CTL0_FORMAT_MASK 0x1fu
/* Position word 0: START_Y 28:16, HFLIP bit 15, START_X 12:0
 * (rpi:vc4_regs.h:1379-1381; written at rpi:vc4_plane.c:2154-2159). */
#define HVS6_POS0_Y_SHIFT 16
#define HVS6_POS0_Y_MASK 0x1fffu
#define HVS6_POS0_X_MASK 0x1fffu
/* Position word 2: SRC_LINES 28:16 and SRC_WIDTH 12:0, each minus one
 * (rpi:vc4_regs.h:1394-1395; written at rpi:vc4_plane.c:2176-2182). */
#define HVS6_POS2_LINES_SHIFT 16
#define HVS6_POS2_LINES_MASK 0x1fffu
#define HVS6_POS2_WIDTH_MASK 0x1fffu
/* Pointer word 0: VFLIP bit 31, UPPER_ADDR 7:0, address bits 39:32
 * (rpi:vc4_regs.h:1397, :1405; written at rpi:vc4_plane.c:2196-2205);
 * pointer word 1 is address bits 31:0 (rpi:vc4_plane.c:2207-2208). */
#define HVS6_PTR0_VFLIP (1u << 31)
#define HVS6_PTR0_UPPER_MASK 0xffu
/* Pointer word 2, linear buffers: PITCH 16:0 in bytes (rpi:vc4_regs.h:1415;
 * written from fb->pitches at rpi:vc4_plane.c:2210-2215). */
#define HVS6_PTR2_PITCH_MASK 0x1ffffu

/* Word indexes within an entry, in the order Linux writes them
 * (rpi:vc4_plane.c:2143-2219): CTL0, POS0, CTL2, POS1 only when not unity
 * (:2168-2174), POS2, POS3 context (:2184-2185), then PTR0, PTR1, PTR2 per
 * plane. Each entry Linux writes ends with an END word inside its NEXT
 * count (rpi:vc4_plane.c:2296-2299); the list ends with an END word at an
 * entry start (rpi:vc4_hvs.c:1316, the walk at rpi:vc4_hvs.c:311-324). */
#define HVS6_W_CTL0 0u
#define HVS6_W_POS0 1u
#define HVS6_W_CTL2 2u
/* Words of an entry before its pointer words: unity 5, else 6. */
#define HVS6_W_PTR_UNITY 5u
/* Three pointer words per plane (PTR0..PTR2). */
#define HVS6_PTR_WORDS 3u

/* HVS pixel format codes, enum hvs_pixel_format (rpi:vc4_regs.h:1058-1091);
 * 18..23 are not defined. */
#define HVS6_FMT_RGB332 0u                     /* rpi:vc4_regs.h:1060 */
#define HVS6_FMT_RGBA4444 1u                   /* rpi:vc4_regs.h:1062 */
#define HVS6_FMT_RGB555 2u                     /* rpi:vc4_regs.h:1063 */
#define HVS6_FMT_RGBA5551 3u                   /* rpi:vc4_regs.h:1064 */
#define HVS6_FMT_RGB565 4u                     /* rpi:vc4_regs.h:1065 */
#define HVS6_FMT_RGB888 5u                     /* rpi:vc4_regs.h:1067 */
#define HVS6_FMT_RGBA6666 6u                   /* rpi:vc4_regs.h:1068 */
#define HVS6_FMT_RGBA8888 7u                   /* rpi:vc4_regs.h:1070 */
#define HVS6_FMT_YUV420_3PLANE 8u              /* rpi:vc4_regs.h:1072 */
#define HVS6_FMT_YUV420_2PLANE 9u              /* rpi:vc4_regs.h:1073 */
#define HVS6_FMT_YUV422_3PLANE 10u             /* rpi:vc4_regs.h:1074 */
#define HVS6_FMT_YUV422_2PLANE 11u             /* rpi:vc4_regs.h:1075 */
#define HVS6_FMT_H264 12u                      /* rpi:vc4_regs.h:1076 */
#define HVS6_FMT_PALETTE 13u                   /* rpi:vc4_regs.h:1077 */
#define HVS6_FMT_YUV444_RGB 14u                /* rpi:vc4_regs.h:1078 */
#define HVS6_FMT_AYUV444_RGB 15u               /* rpi:vc4_regs.h:1079 */
#define HVS6_FMT_RGBA1010102 16u               /* rpi:vc4_regs.h:1080 */
#define HVS6_FMT_YCBCR_10BIT 17u               /* rpi:vc4_regs.h:1081 */
#define HVS6_FMT_YUV420_15_6_2PLANE 24u        /* rpi:vc4_regs.h:1083 */
#define HVS6_FMT_YUV420_15_6_3PLANE 25u        /* rpi:vc4_regs.h:1084 */
#define HVS6_FMT_YUV420_13_4_2PLANE 26u        /* rpi:vc4_regs.h:1085 */
#define HVS6_FMT_YUV420_13_4_3PLANE 27u        /* rpi:vc4_regs.h:1086 */
#define HVS6_FMT_YUV420_11_2_2PLANE 28u        /* rpi:vc4_regs.h:1087 */
#define HVS6_FMT_YUV420_11_2_3PLANE 29u        /* rpi:vc4_regs.h:1088 */
#define HVS6_FMT_YUV420_9_0_2PLANE 30u         /* rpi:vc4_regs.h:1089 */
#define HVS6_FMT_YUV420_9_0_3PLANE 31u         /* rpi:vc4_regs.h:1090 */

/* Pixel orders of the 8888 family as Linux uses them on 5.x and 6.x
 * (pixel_order_hvs5): XRGB8888 and ARGB8888 are ARGB 2, XBGR8888 is ABGR 3
 * (rpi:vc4_regs.h:1097-1100; the table at rpi:vc4_plane.c:43-65). */
#define HVS6_ORDER_RGBA 0u
#define HVS6_ORDER_BGRA 1u
#define HVS6_ORDER_ARGB 2u
#define HVS6_ORDER_ABGR 3u

/* ---- decoder ------------------------------------------------------------- */

enum hvs6_gen { HVS6_GEN_UNKNOWN = 0, HVS6_GEN_C, HVS6_GEN_D };

/* Errors of hvs6_dlist_first_plane (0 is a decoded plane). */
enum hvs6_err {
	HVS6_OK = 0,
	HVS6_ERR_RANGE = -1,   /* start index outside the words passed in */
	HVS6_ERR_END = -2,     /* the list is empty: an END word at the start */
	HVS6_ERR_INVALID = -3, /* control word 0 without VALID */
	HVS6_ERR_SIZE = -4,    /* NEXT too short for a plane, or past the end */
	HVS6_ERR_FORMAT = -5,  /* a pixel format code no enum entry names */
	HVS6_ERR_TILED = -6,   /* not a linear (raster) buffer */
};

/* The channel registers decoded. */
struct hvs6_chan {
	bool enabled;   /* CTRL0.ENB */
	bool running;   /* STATUS.MODE is INIT or RUN */
	bool empty;     /* STATUS.EMPTY */
	uint32_t mode;  /* STATUS.MODE */
	uint32_t frcnt; /* STATUS.FRCNT */
	uint32_t yline; /* STATUS.YLINE */
	uint32_t fwidth, lines; /* CTRL0.FWIDTH, CTRL0.LINES */
	uint32_t head;   /* LPTRS.HEADE: the list start last installed */
	uint32_t active; /* DL.LACT: the list the channel runs; decode from here */
};

/* The first plane of a list. */
struct hvs6_plane {
	uint64_t addr; /* 40-bit fetch address of the first pointer */
	uint32_t pitch, width, height, format_code;
	bool rgb32;      /* RGBA8888 family (4 bytes, 8 bits a channel) */
	uint32_t x, y;   /* POS0 start on the screen */
	uint32_t order;  /* CTL0.ORDERRGBA */
	bool unity;      /* CTL0.UNITY (no POS1 word) */
	uint32_t nwords; /* CTL0.NEXT: the entry's length */
};

static inline enum hvs6_gen hvs6_gen_from_version(uint32_t version)
{
	switch (version & HVS6_VERSION_MASK) {
	case HVS6_VERSION_C0:
		return HVS6_GEN_C;
	case HVS6_VERSION_D0:
		return HVS6_GEN_D;
	default:
		return HVS6_GEN_UNKNOWN;
	}
}

/* Byte offsets of channel chan's CTRL0, LPTRS, STATUS and DL from the HVS
 * base; 0 for an unknown generation or channel (0 is VERSION, never one of
 * these). */
enum hvs6_chan_reg { HVS6_REG_CTRL0, HVS6_REG_LPTRS, HVS6_REG_STATUS, HVS6_REG_DL };

static inline uint32_t hvs6_chan_reg_offset(enum hvs6_gen gen, unsigned int chan,
					    enum hvs6_chan_reg reg)
{
	static const uint32_t c[4] = {HVS6C_DISP0_CTRL0, HVS6C_DISP0_LPTRS,
				      HVS6C_DISP0_STATUS, HVS6C_DISP0_DL};
	static const uint32_t d[4] = {HVS6D_DISP0_CTRL0, HVS6D_DISP0_LPTRS,
				      HVS6D_DISP0_STATUS, HVS6D_DISP0_DL};

	if (chan >= HVS6_CHANNELS || (unsigned int)reg > HVS6_REG_DL)
		return 0;
	if (gen == HVS6_GEN_C)
		return c[reg] + chan * HVS6C_DISP_STRIDE;
	if (gen == HVS6_GEN_D)
		return d[reg] + chan * HVS6D_DISP_STRIDE;
	return 0;
}

static inline void hvs6_chan_decode(uint32_t ctrl0, uint32_t lptrs, uint32_t status,
				    uint32_t dl, struct hvs6_chan *out)
{
	out->enabled = (ctrl0 & HVS6_CTRL0_ENB) != 0;
	out->fwidth = (ctrl0 >> HVS6_CTRL0_FWIDTH_SHIFT) & HVS6_CTRL0_FWIDTH_MASK;
	out->lines = ctrl0 & HVS6_CTRL0_LINES_MASK;
	out->mode = (status >> HVS6_STATUS_MODE_SHIFT) & HVS6_STATUS_MODE_MASK;
	out->running = out->mode == HVS6_STATUS_MODE_INIT || out->mode == HVS6_STATUS_MODE_RUN;
	out->empty = (status & HVS6_STATUS_EMPTY) != 0;
	out->frcnt = (status >> HVS6_STATUS_FRCNT_SHIFT) & HVS6_STATUS_FRCNT_MASK;
	out->yline = status & HVS6_STATUS_YLINE_MASK;
	out->head = lptrs & HVS6_LPTRS_HEADE_MASK;
	out->active = dl & HVS6_DL_LACT_MASK;
}

/* The name of a pixel format code, or NULL for one the enum does not name. */
static inline const char *hvs6_format_name(uint32_t code)
{
	static const char *const names[32] = {
		[HVS6_FMT_RGB332] = "RGB332",
		[HVS6_FMT_RGBA4444] = "RGBA4444",
		[HVS6_FMT_RGB555] = "RGB555",
		[HVS6_FMT_RGBA5551] = "RGBA5551",
		[HVS6_FMT_RGB565] = "RGB565",
		[HVS6_FMT_RGB888] = "RGB888",
		[HVS6_FMT_RGBA6666] = "RGBA6666",
		[HVS6_FMT_RGBA8888] = "RGBA8888",
		[HVS6_FMT_YUV420_3PLANE] = "YCBCR_YUV420_3PLANE",
		[HVS6_FMT_YUV420_2PLANE] = "YCBCR_YUV420_2PLANE",
		[HVS6_FMT_YUV422_3PLANE] = "YCBCR_YUV422_3PLANE",
		[HVS6_FMT_YUV422_2PLANE] = "YCBCR_YUV422_2PLANE",
		[HVS6_FMT_H264] = "H264",
		[HVS6_FMT_PALETTE] = "PALETTE",
		[HVS6_FMT_YUV444_RGB] = "YUV444_RGB",
		[HVS6_FMT_AYUV444_RGB] = "AYUV444_RGB",
		[HVS6_FMT_RGBA1010102] = "RGBA1010102",
		[HVS6_FMT_YCBCR_10BIT] = "YCBCR_10BIT",
		[HVS6_FMT_YUV420_15_6_2PLANE] = "YCBCR_YUV420_15_6_2PLANE",
		[HVS6_FMT_YUV420_15_6_3PLANE] = "YCBCR_YUV420_15_6_3PLANE",
		[HVS6_FMT_YUV420_13_4_2PLANE] = "YCBCR_YUV420_13_4_2PLANE",
		[HVS6_FMT_YUV420_13_4_3PLANE] = "YCBCR_YUV420_13_4_3PLANE",
		[HVS6_FMT_YUV420_11_2_2PLANE] = "YCBCR_YUV420_11_2_2PLANE",
		[HVS6_FMT_YUV420_11_2_3PLANE] = "YCBCR_YUV420_11_2_3PLANE",
		[HVS6_FMT_YUV420_9_0_2PLANE] = "YCBCR_YUV420_9_0_2PLANE",
		[HVS6_FMT_YUV420_9_0_3PLANE] = "YCBCR_YUV420_9_0_3PLANE",
	};

	return code < 32 ? names[code] : NULL;
}

/*
 * Decode the entry at word index start of the display-list memory dl
 * (nwords words, from HVS6_DLIST_OFFSET; at most CXM_SIZE). Fills *out and
 * returns HVS6_OK, or returns an hvs6_err with *out cleared.
 */
static inline int hvs6_dlist_first_plane(const uint32_t *dl, size_t nwords, uint32_t start,
					 struct hvs6_plane *out)
{
	uint32_t ctl0, n, ptr, pos0, pos2, fmt;

	*out = (struct hvs6_plane){0};
	if (dl == NULL || start >= nwords)
		return HVS6_ERR_RANGE;
	ctl0 = dl[start];
	if (ctl0 & HVS6_CTL0_END)
		return HVS6_ERR_END;
	if (!(ctl0 & HVS6_CTL0_VALID))
		return HVS6_ERR_INVALID;

	n = (ctl0 >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
	ptr = HVS6_W_PTR_UNITY + ((ctl0 & HVS6_CTL0_UNITY) ? 0u : 1u);
	if (n < ptr + HVS6_PTR_WORDS || (size_t)n > nwords - start)
		return HVS6_ERR_SIZE;

	fmt = ctl0 & HVS6_CTL0_FORMAT_MASK;
	if (hvs6_format_name(fmt) == NULL)
		return HVS6_ERR_FORMAT;
	if (((ctl0 >> HVS6_CTL0_ADDR_MODE_SHIFT) & HVS6_CTL0_ADDR_MODE_MASK) !=
	    HVS6_CTL0_ADDR_MODE_LINEAR)
		return HVS6_ERR_TILED;

	pos0 = dl[start + HVS6_W_POS0];
	pos2 = dl[start + ptr - 2u]; /* POS2, then POS3, then the pointers */
	out->format_code = fmt;
	out->rgb32 = fmt == HVS6_FMT_RGBA8888;
	out->order = (ctl0 >> HVS6_CTL0_ORDER_SHIFT) & HVS6_CTL0_ORDER_MASK;
	out->unity = (ctl0 & HVS6_CTL0_UNITY) != 0;
	out->nwords = n;
	out->x = pos0 & HVS6_POS0_X_MASK;
	out->y = (pos0 >> HVS6_POS0_Y_SHIFT) & HVS6_POS0_Y_MASK;
	out->width = (pos2 & HVS6_POS2_WIDTH_MASK) + 1u;
	out->height = ((pos2 >> HVS6_POS2_LINES_SHIFT) & HVS6_POS2_LINES_MASK) + 1u;
	out->addr = ((uint64_t)(dl[start + ptr] & HVS6_PTR0_UPPER_MASK) << 32) |
		    dl[start + ptr + 1u];
	out->pitch = dl[start + ptr + 2u] & HVS6_PTR2_PITCH_MASK;
	return HVS6_OK;
}

#endif /* YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_H */
