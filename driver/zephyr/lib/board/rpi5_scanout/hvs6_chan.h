/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * HVS6 (BCM2712 "scaler6") channel setup as a recorded register-write list.
 *
 * No MMIO happens here: yd5_hvs6_chan_enable() and yd5_hvs6_chan_disable()
 * append {offset, value} pairs to a log, in the order the Raspberry Pi Linux
 * kernel writes them, and the probe replays the log against the HVS register
 * block. Offsets are relative to the HVS base. Only register offsets, bit
 * positions and order are taken from the kernel; each carries its source as
 * "rpi:<file>:<line>" (branch rpi-6.12.y, drivers/gpu/drm/vc4/).
 *
 * Two HVS steppings exist, and Linux picks the macro set by them:
 * C0 (SCALER6_VERSION 0x53) and D0 (0x54), with different channel strides.
 * The functions without a variant argument use C0; the _v forms take it.
 *
 * Limits of a log: it holds values, not reads. Linux read-modify-writes
 * DISPx_CTRL1 and the second write of the stop sequence; here the bits that
 * the kernel's source defines are written and every other bit is 0.
 *
 * C99, header-only, no Zephyr includes.
 */
#ifndef YD5_HVS6_CHAN_H
#define YD5_HVS6_CHAN_H

#include <stdint.h>

/* --- stepping ----------------------------------------------------------- */

/* rpi:vc4_regs.h:543-546 */
#define YD5_HVS6_VERSION		0x00000000u
#define YD5_HVS6_VERSION_MASK		0x000000ffu	/* bits 7:0 */
#define YD5_HVS6_VERSION_C0		0x53u
#define YD5_HVS6_VERSION_D0		0x54u

enum yd5_hvs6_variant {
	YD5_HVS6_C0 = 0, /* rpi:vc4_regs.h:565 (VC4_GEN_6_C) */
	YD5_HVS6_D0 = 1, /* rpi:vc4_regs.h:565 (otherwise) */
};

/* -1 for a version byte that is neither C0 nor D0 (rpi:vc4_regs.h:544-546) */
static inline int yd5_hvs6_variant_from_version(uint32_t version_reg,
						enum yd5_hvs6_variant *out)
{
	uint32_t v = version_reg & YD5_HVS6_VERSION_MASK;

	if (v == YD5_HVS6_VERSION_C0) {
		*out = YD5_HVS6_C0;
		return 0;
	}
	if (v == YD5_HVS6_VERSION_D0) {
		*out = YD5_HVS6_D0;
		return 0;
	}
	return -1;
}

/* --- channel registers (offset from the HVS base) ----------------------- */

#define YD5_HVS6_NUM_CHANNELS		3u	/* rpi:vc4_regs.h:630-637 (DISP0..2) */

/* C0: channel 0 block, stride 0x20 (rpi:vc4_regs.h:564,622 -> 0x50-0x30) */
#define YD5_HVS6C_DISP0_CTRL0		0x00000030u	/* rpi:vc4_regs.h:564 */
#define YD5_HVS6C_DISP0_CTRL1		0x00000034u	/* rpi:vc4_regs.h:575 */
#define YD5_HVS6C_DISP0_BGND		0x00000038u	/* rpi:vc4_regs.h:582 */
#define YD5_HVS6C_DISP0_LPTRS		0x0000003cu	/* rpi:vc4_regs.h:587 */
#define YD5_HVS6C_DISP0_STATUS		0x00000044u	/* rpi:vc4_regs.h:600 */
#define YD5_HVS6C_DISP0_DL		0x00000048u	/* rpi:vc4_regs.h:614 */
#define YD5_HVS6C_DISP_STRIDE		0x00000020u	/* rpi:vc4_regs.h:622,564 */

/* D0: channel 0 block, stride 0x40 (rpi:vc4_regs.h:701,718 -> 0x140-0x100) */
#define YD5_HVS6D_DISP0_CTRL0		0x00000100u	/* rpi:vc4_regs.h:701 */
#define YD5_HVS6D_DISP0_CTRL1		0x00000104u	/* rpi:vc4_regs.h:702 */
#define YD5_HVS6D_DISP0_BGND		0x00000108u	/* rpi:vc4_regs.h:703 */
#define YD5_HVS6D_DISP0_LPTRS		0x00000110u	/* rpi:vc4_regs.h:704 */
#define YD5_HVS6D_DISP0_STATUS		0x00000118u	/* rpi:vc4_regs.h:706 */
#define YD5_HVS6D_DISP0_DL		0x0000011cu	/* rpi:vc4_regs.h:714 */
#define YD5_HVS6D_DISP_STRIDE		0x00000040u	/* rpi:vc4_regs.h:718,701 */

/* DISPx_CTRL0 fields (the kernel uses the C0 masks for both steppings) */
#define YD5_HVS6_CTRL0_ENB		(1u << 31)	/* rpi:vc4_regs.h:568 */
#define YD5_HVS6_CTRL0_RESET		(1u << 30)	/* rpi:vc4_regs.h:569 */
#define YD5_HVS6_CTRL0_FWIDTH_SHIFT	16u		/* rpi:vc4_regs.h:570 (28:16) */
#define YD5_HVS6_CTRL0_FWIDTH_MASK	0x1fffu
#define YD5_HVS6_CTRL0_ONESHOT		(1u << 15)	/* rpi:vc4_regs.h:571 */
#define YD5_HVS6_CTRL0_LINES_SHIFT	0u		/* rpi:vc4_regs.h:573 (12:0) */
#define YD5_HVS6_CTRL0_LINES_MASK	0x1fffu

/* DISPx_CTRL1 fields */
#define YD5_HVS6_CTRL1_BGENB		(1u << 8)	/* rpi:vc4_regs.h:579 */
#define YD5_HVS6_CTRL1_INTLACE		(1u << 0)	/* rpi:vc4_regs.h:580 */

/* DISPx_LPTRS: display list head, in words (rpi:vc4_regs.h:591, 11:0) */
#define YD5_HVS6_LPTRS_HEADE_MASK	0x00000fffu

/* DISPx_STATUS mode field (rpi:vc4_regs.h:607-611, bits 14:13) */
#define YD5_HVS6_STATUS_MODE_SHIFT	13u
#define YD5_HVS6_STATUS_MODE_MASK	0x3u
#define YD5_HVS6_STATUS_MODE_DISABLED	0u
#define YD5_HVS6_STATUS_MODE_INIT	1u
#define YD5_HVS6_STATUS_MODE_RUN	2u
#define YD5_HVS6_STATUS_MODE_EOF	3u

/* First display list word the kernel's allocator hands out (rpi:vc4_regs.h:1056) */
#define YD5_HVS6_BOOTLOADER_DLIST_END	32u

/* --- errors ------------------------------------------------------------- */

#define YD5_HVS6_EINVAL			(-22)
#define YD5_HVS6_ENOSPC			(-28)
#define YD5_HVS6_EPIPE			(-32)	/* rpi:vc4_hvs.c:931 (-EPIPE) */

/* --- write log ---------------------------------------------------------- */

#define YD5_HVS6_LOG_MAX		16u

struct yd5_hvs6_write {
	uint32_t offset;	/* from the HVS base */
	uint32_t value;
};

struct yd5_hvs6_log {
	struct yd5_hvs6_write w[YD5_HVS6_LOG_MAX];
	uint32_t n;
};

static inline void yd5_hvs6_log_init(struct yd5_hvs6_log *log)
{
	log->n = 0;
}

static inline int yd5_hvs6_log_append(struct yd5_hvs6_log *log,
				      uint32_t offset, uint32_t value)
{
	if (log->n >= YD5_HVS6_LOG_MAX)
		return YD5_HVS6_ENOSPC;
	log->w[log->n].offset = offset;
	log->w[log->n].value = value;
	log->n++;
	return 0;
}

/* --- register addresses of a channel ------------------------------------ */

static inline uint32_t yd5_hvs6_reg(enum yd5_hvs6_variant v, unsigned int chan,
				    uint32_t c0_chan0, uint32_t d0_chan0)
{
	return v == YD5_HVS6_D0 ? d0_chan0 + chan * YD5_HVS6D_DISP_STRIDE
				: c0_chan0 + chan * YD5_HVS6C_DISP_STRIDE;
}

static inline uint32_t yd5_hvs6_ctrl0_reg(enum yd5_hvs6_variant v, unsigned int chan)
{
	return yd5_hvs6_reg(v, chan, YD5_HVS6C_DISP0_CTRL0, YD5_HVS6D_DISP0_CTRL0);
}

static inline uint32_t yd5_hvs6_ctrl1_reg(enum yd5_hvs6_variant v, unsigned int chan)
{
	return yd5_hvs6_reg(v, chan, YD5_HVS6C_DISP0_CTRL1, YD5_HVS6D_DISP0_CTRL1);
}

static inline uint32_t yd5_hvs6_bgnd_reg(enum yd5_hvs6_variant v, unsigned int chan)
{
	return yd5_hvs6_reg(v, chan, YD5_HVS6C_DISP0_BGND, YD5_HVS6D_DISP0_BGND);
}

static inline uint32_t yd5_hvs6_lptrs_reg(enum yd5_hvs6_variant v, unsigned int chan)
{
	return yd5_hvs6_reg(v, chan, YD5_HVS6C_DISP0_LPTRS, YD5_HVS6D_DISP0_LPTRS);
}

static inline uint32_t yd5_hvs6_status_reg(enum yd5_hvs6_variant v, unsigned int chan)
{
	return yd5_hvs6_reg(v, chan, YD5_HVS6C_DISP0_STATUS, YD5_HVS6D_DISP0_STATUS);
}

/* --- sequences ---------------------------------------------------------- */

/*
 * Stop: Linux writes CTRL0 with RESET set, then with ENB cleared
 * (rpi:vc4_hvs.c:1072-1096, writes at :1086-1090). The bits of CTRL0 other
 * than ENB/RESET are not known to a log; they are written as 0 here.
 */
static inline int yd5_hvs6_chan_disable_v(struct yd5_hvs6_log *log,
					  enum yd5_hvs6_variant v,
					  unsigned int chan)
{
	uint32_t ctrl0;
	int r;

	if (chan >= YD5_HVS6_NUM_CHANNELS)
		return YD5_HVS6_EINVAL;
	if (log->n + 2u > YD5_HVS6_LOG_MAX)
		return YD5_HVS6_ENOSPC;
	ctrl0 = yd5_hvs6_ctrl0_reg(v, chan);
	r = yd5_hvs6_log_append(log, ctrl0, YD5_HVS6_CTRL0_ENB | YD5_HVS6_CTRL0_RESET);
	if (r)
		return r;
	return yd5_hvs6_log_append(log, ctrl0, YD5_HVS6_CTRL0_RESET);
}

static inline int yd5_hvs6_chan_disable(struct yd5_hvs6_log *log, unsigned int chan)
{
	return yd5_hvs6_chan_disable_v(log, YD5_HVS6_C0, chan);
}

/*
 * Start, in the order of the kernel's vc4_hvs_atomic_enable
 * (rpi:vc4_hvs.c:1218-1234): display list pointer (vc4_hvs_install_dlist,
 * rpi:vc4_hvs.c:1166-1169), then vc6_hvs_init_channel (rpi:vc4_hvs.c:1004-1036):
 * CTRL0 = RESET (:1020), CTRL1 without INTLACE (:1022-1025), CTRL0 = ENB |
 * FWIDTH(width-1) | LINES(height-1) (:1027-1033).
 *
 * The kernel's start path does not write DISPx_BGND: its only user is the
 * register dump (rpi:vc4_hvs.c:83-99, :157-176). The background fill is
 * switched through CTRL1.BGENB (rpi:vc4_hvs.c:1499-1505). Here bg_rgb is
 * written to BGND as given (bits 23:0, no field layout in the kernel) and
 * BGENB is set in CTRL1; both are a guess the board has to confirm.
 *
 * Writes, in order: stop pair (2), LPTRS, BGND, CTRL0 reset, CTRL1, CTRL0 =
 * enable. Errors, with nothing appended: chan > 2, zero or over 8192 width or
 * height (the fields hold size-1 in 13 bits), a list offset below
 * YD5_HVS6_BOOTLOADER_DLIST_END (rpi:vc4_hvs.c:1570-1583) or over the 12-bit
 * head field.
 */
static inline int yd5_hvs6_chan_enable_v(struct yd5_hvs6_log *log,
					 enum yd5_hvs6_variant v,
					 unsigned int chan, uint32_t width,
					 uint32_t height,
					 uint32_t dlist_offset_words,
					 uint32_t bg_rgb)
{
	uint32_t ctrl0;
	int r;

	if (chan >= YD5_HVS6_NUM_CHANNELS || width == 0 || height == 0 ||
	    width > YD5_HVS6_CTRL0_FWIDTH_MASK + 1u ||
	    height > YD5_HVS6_CTRL0_LINES_MASK + 1u ||
	    dlist_offset_words < YD5_HVS6_BOOTLOADER_DLIST_END ||
	    dlist_offset_words > YD5_HVS6_LPTRS_HEADE_MASK)
		return YD5_HVS6_EINVAL;
	if (log->n + 7u > YD5_HVS6_LOG_MAX)
		return YD5_HVS6_ENOSPC;

	ctrl0 = yd5_hvs6_ctrl0_reg(v, chan);
	r = yd5_hvs6_chan_disable_v(log, v, chan);
	if (!r)
		r = yd5_hvs6_log_append(log, yd5_hvs6_lptrs_reg(v, chan), dlist_offset_words);
	if (!r)
		r = yd5_hvs6_log_append(log, yd5_hvs6_bgnd_reg(v, chan), bg_rgb & 0x00ffffffu);
	if (!r)
		r = yd5_hvs6_log_append(log, ctrl0, YD5_HVS6_CTRL0_RESET);
	if (!r)
		r = yd5_hvs6_log_append(log, yd5_hvs6_ctrl1_reg(v, chan), YD5_HVS6_CTRL1_BGENB);
	if (!r)
		r = yd5_hvs6_log_append(log, ctrl0,
					YD5_HVS6_CTRL0_ENB |
					((width - 1u) << YD5_HVS6_CTRL0_FWIDTH_SHIFT) |
					((height - 1u) << YD5_HVS6_CTRL0_LINES_SHIFT));
	return r;
}

static inline int yd5_hvs6_chan_enable(struct yd5_hvs6_log *log, unsigned int chan,
				       uint32_t width, uint32_t height,
				       uint32_t dlist_offset_words, uint32_t bg_rgb)
{
	return yd5_hvs6_chan_enable_v(log, YD5_HVS6_C0, chan, width, height,
				      dlist_offset_words, bg_rgb);
}

/* --- output to FIFO ----------------------------------------------------- */

/*
 * HVS output (0..4) to FIFO (0..2) for the 6.x HVS (rpi:vc4_hvs.c:916-932):
 * 0 -> 0, 2 -> 2, 1, 3 and 4 -> 1, anything else -EPIPE.
 */
static inline int yd5_hvs6_fifo_from_output(unsigned int output)
{
	switch (output) {
	case 0:
		return 0;
	case 2:
		return 2;
	case 1:
	case 3:
	case 4:
		return 1;
	default:
		return YD5_HVS6_EPIPE;
	}
}

#endif /* YD5_HVS6_CHAN_H */
