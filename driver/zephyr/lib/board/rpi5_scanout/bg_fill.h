/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * HVS6 (BCM2712 "scaler6") channel background fill as a masked register-write
 * list: a picture without a framebuffer. With the channel running and an empty
 * display list, the HVS shows the channel's background colour on the pixel
 * valve and HDMI, so a solid colour on the screen shows that the HVS6 -> pixel
 * valve -> HDMI path is alive.
 *
 * No MMIO happens here. yd5_bgf_plan() returns {offset, value, mask} writes;
 * the caller applies each as read, clear the mask bits, OR (value & mask),
 * write, so every bit outside the mask stays as found. yd5_bgf_restore() turns
 * the register values read before the fill into full-width writes that put
 * them back. Offsets are relative to the HVS base. Each register carries its
 * source as "rpi:<file>:<line>" (branch rpi-6.12.y, drivers/gpu/drm/vc4/); a
 * value with no source is marked "unverified". The offsets are local on
 * purpose: this header does not include hvs6_chan.h.
 *
 * Not done here: starting the channel (CTRL0, size, display list pointer) and
 * the empty list itself; that is the channel setup's job, and the fill shows
 * only while the channel runs.
 *
 * C11, header-only, no Zephyr includes.
 */
#ifndef YD5_BG_FILL_H
#define YD5_BG_FILL_H

#include <stdint.h>

#define YD5_BGF_EINVAL		(-22)

#define YD5_BGF_NUM_CHANNELS	3u	/* rpi:vc4_regs.h:630-637 (DISP0..2) */

enum yd5_bgf_variant {
	YD5_BGF_C0 = 0,	/* SCALER6_VERSION 0x53, rpi:vc4_regs.h:544-546, :565 */
	YD5_BGF_D0 = 1,	/* SCALER6_VERSION 0x54 */
};

/* C0: channel 0 CTRL1 / BGND, stride 0x20 (rpi:vc4_regs.h:564,575,582,622) */
#define YD5_BGF_C0_CTRL1	0x00000034u
#define YD5_BGF_C0_BGND		0x00000038u
#define YD5_BGF_C0_STRIDE	0x00000020u
/* D0: channel 0 CTRL1 / BGND, stride 0x40 (rpi:vc4_regs.h:701-703,718) */
#define YD5_BGF_D0_CTRL1	0x00000104u
#define YD5_BGF_D0_BGND		0x00000108u
#define YD5_BGF_D0_STRIDE	0x00000040u

/* DISPx_CTRL1 bit 8: background fill enable (rpi:vc4_regs.h:579; used for
 * the fill at rpi:vc4_hvs.c:1499-1505) */
#define YD5_BGF_CTRL1_BGENB	(1u << 8)

/*
 * DISPx_BGND carries the colour in bits 23:0 as 0xRRGGBB. The kernel only
 * reads the register for its dump (rpi:vc4_hvs.c:83-99, :157-176) and defines
 * no field layout: the 8:8:8 order and the meaning of bits 31:24 are
 * unverified. The plan masks to 23:0 and leaves 31:24 as found.
 */
#define YD5_BGF_BGND_MASK	0x00ffffffu

#define YD5_BGF_MAX_WRITES	2u

struct yd5_bgf_write {
	uint32_t offset;	/* from the HVS base */
	uint32_t value;
	uint32_t mask;		/* bits to change; the others stay as found */
};

struct yd5_bgf_plan {
	unsigned int n;
	struct yd5_bgf_write w[YD5_BGF_MAX_WRITES];
};

static inline int yd5_bgf_regs(enum yd5_bgf_variant v, unsigned int ch,
			       uint32_t *ctrl1, uint32_t *bgnd)
{
	if (ch >= YD5_BGF_NUM_CHANNELS)
		return YD5_BGF_EINVAL;
	if (v == YD5_BGF_C0) {
		*ctrl1 = YD5_BGF_C0_CTRL1 + ch * YD5_BGF_C0_STRIDE;
		*bgnd = YD5_BGF_C0_BGND + ch * YD5_BGF_C0_STRIDE;
	} else if (v == YD5_BGF_D0) {
		*ctrl1 = YD5_BGF_D0_CTRL1 + ch * YD5_BGF_D0_STRIDE;
		*bgnd = YD5_BGF_D0_BGND + ch * YD5_BGF_D0_STRIDE;
	} else {
		return YD5_BGF_EINVAL;
	}
	return 0;
}

/*
 * Two writes in order: BGND colour (mask 23:0), then CTRL1 BGENB (mask bit
 * 8; INTLACE and every other bit stay). The colour goes first so the fill
 * never shows a stale colour. 0 on success; YD5_BGF_EINVAL with out->n = 0
 * for ch > 2, an unknown variant, rgb888 above 0xffffff or a null out.
 */
static inline int yd5_bgf_plan_v(enum yd5_bgf_variant v, unsigned int ch,
				 uint32_t rgb888, struct yd5_bgf_plan *out)
{
	uint32_t ctrl1, bgnd;

	if (!out)
		return YD5_BGF_EINVAL;
	out->n = 0;
	if (rgb888 > YD5_BGF_BGND_MASK || yd5_bgf_regs(v, ch, &ctrl1, &bgnd))
		return YD5_BGF_EINVAL;
	out->w[0].offset = bgnd;
	out->w[0].value = rgb888;
	out->w[0].mask = YD5_BGF_BGND_MASK;
	out->w[1].offset = ctrl1;
	out->w[1].value = YD5_BGF_CTRL1_BGENB;
	out->w[1].mask = YD5_BGF_CTRL1_BGENB;
	out->n = 2;
	return 0;
}

/* The stepping-C0 form. */
static inline int yd5_bgf_plan(unsigned int ch, uint32_t rgb888,
			       struct yd5_bgf_plan *out)
{
	return yd5_bgf_plan_v(YD5_BGF_C0, ch, rgb888, out);
}

/*
 * saved[i] is the whole register read at plan->w[i].offset before the plan
 * was applied. Returns, in reverse order, a full-width write (mask
 * 0xffffffff) of each saved value, so the register file is back byte for
 * byte. 0 on success, YD5_BGF_EINVAL for a null or aliased argument or a plan
 * longer than YD5_BGF_MAX_WRITES.
 */
static inline int yd5_bgf_restore(const struct yd5_bgf_plan *plan,
				  const uint32_t *saved,
				  struct yd5_bgf_plan *out)
{
	unsigned int i;

	if (!plan || !saved || !out || plan == out || plan->n > YD5_BGF_MAX_WRITES)
		return YD5_BGF_EINVAL;
	out->n = plan->n;
	for (i = 0; i < plan->n; i++) {
		out->w[i].offset = plan->w[plan->n - 1u - i].offset;
		out->w[i].value = saved[plan->n - 1u - i];
		out->w[i].mask = 0xffffffffu;
	}
	return 0;
}

/* One masked write applied to a register value (what the caller does per write). */
static inline uint32_t yd5_bgf_merge(uint32_t old, const struct yd5_bgf_write *w)
{
	return (old & ~w->mask) | (w->value & w->mask);
}

#endif /* YD5_BG_FILL_H */
