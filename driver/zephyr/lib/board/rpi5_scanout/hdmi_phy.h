/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * BCM2712 HDMI0 PHY: capture the values the firmware programmed, replay them.
 *
 * Route 3 (our own HVS6, pixel valve and HDMI) needs the HDMI PHY and its PLL
 * set up. Linux does it in vc6_hdmi_phy_init
 * (rpi:drivers/gpu/drm/vc4/vc4_hdmi_phy.c:954-1198, summarised in
 * driver/docs/rpi5-v3d71-facts.md section 10, "BCM2712 HDMI PHY
 * initialisation") from per-rate tables and fixed constants. Those are GPL
 * and are not copied here. Instead, when the firmware has already brought
 * HDMI0 up at the mode we want (HDMI_VID_CTL bit 31,
 * rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:3391-3399, bit at rpi:vc4_regs.h:1007),
 * the values it left in the PHY, PLL and RM registers are read
 * (yd5_hdmi_phy_capture) and written back in the order the facts file gives
 * (yd5_hdmi_phy_replay_plan), for example after the HDMI block was reset.
 * No value in this file comes from a Linux table: every register value of
 * the plan is either 0 or a captured value.
 *
 * Offsets are relative to the base of their block (yd5_hdmi_phy_block): the
 * hdmi0 node's "phy", "rm" and "hd" regions (facts section 10,
 * rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:370-397), from
 * vc6_hdmi_hdmi0_fields (rpi:drivers/gpu/drm/vc4/vc4_hdmi_regs.h:431,
 * rpi-6.12.y 5bd564aa); each carries its line. Steps of the plan whose
 * effect on the hardware is our own choice are marked "unverified".
 *
 * No MMIO happens here: reads go through an injected reader, the plan is a
 * list of {op, block, offset, value} entries, and yd5_hdmi_phy_run() replays
 * it through injected accessors (the probe passes MMIO, the mock a fake
 * register file).
 *
 * C11, header-only, no Zephyr includes.
 */
#ifndef YD5_HDMI_PHY_H
#define YD5_HDMI_PHY_H

#include <stddef.h>
#include <stdint.h>

/* --- blocks and registers ---------------------------------------------- */

enum yd5_hdmi_phy_block {
	YD5_HDMI_BLK_PHY = 0,	/* hdmi0 reg-name "phy" (bcm2712.dtsi:370-397) */
	YD5_HDMI_BLK_RM = 1,	/* hdmi0 reg-name "rm" */
	YD5_HDMI_BLK_HD = 2,	/* hdmi0 reg-name "hd" */
};

/* Offsets in the "phy" block (VC5_PHY_REG entries of vc6_hdmi_hdmi0_fields) */
#define YD5_HDMI_PHY_RESET_CTL		0x000u	/* rpi:vc4_hdmi_regs.h:478 */
#define YD5_HDMI_PHY_POWERUP_CTL	0x004u	/* rpi:vc4_hdmi_regs.h:479 */
#define YD5_HDMI_PHY_CTL_0		0x008u	/* rpi:vc4_hdmi_regs.h:480 */
#define YD5_HDMI_PHY_CTL_1		0x00cu	/* rpi:vc4_hdmi_regs.h:481 */
#define YD5_HDMI_PHY_CTL_2		0x010u	/* rpi:vc4_hdmi_regs.h:482 */
#define YD5_HDMI_PHY_CTL_CK		0x014u	/* rpi:vc4_hdmi_regs.h:483 */
#define YD5_HDMI_PHY_PLL_REFCLK		0x01cu	/* rpi:vc4_hdmi_regs.h:484 */
#define YD5_HDMI_PHY_PLL_POST_KDIV	0x028u	/* rpi:vc4_hdmi_regs.h:485 */
#define YD5_HDMI_PHY_PLL_VCOCLK_DIV	0x02cu	/* rpi:vc4_hdmi_regs.h:486 */
#define YD5_HDMI_PHY_PLL_CFG		0x044u	/* rpi:vc4_hdmi_regs.h:487 */
#define YD5_HDMI_PHY_TMDS_CLK_WORD_SEL	0x054u	/* rpi:vc4_hdmi_regs.h:488 */
#define YD5_HDMI_PHY_PLL_MISC_0		0x060u	/* rpi:vc4_hdmi_regs.h:489; _1.._8 at +4, :490-497 */
#define YD5_HDMI_PHY_PLL_MISC_N		9u	/* facts s.10 step 2 (:974-982) */
#define YD5_HDMI_PHY_PLL_RESET_CTL	0x190u	/* rpi:vc4_hdmi_regs.h:498 */
#define YD5_HDMI_PHY_PLL_POWERUP_CTL	0x194u	/* rpi:vc4_hdmi_regs.h:499 */
/* "rm" block (VC5_RM_REG) */
#define YD5_HDMI_RM_OFFSET		0x018u	/* rpi:vc4_hdmi_regs.h:502 */
/* "hd" block (VC4_HD_REG) */
#define YD5_HDMI_HD_VID_CTL		0x044u	/* rpi:vc4_hdmi_regs.h:438 */
#define YD5_HDMI_HD_VID_CTL_ENABLE	0x80000000u /* bit 31, rpi:vc4_regs.h:1007 */

/* --- the capture list ---------------------------------------------------- */

/* Index of each captured register in yd5_hdmi_phy_regs[] and in a capture. */
enum yd5_hdmi_phy_idx {
	YD5_PHYI_RESET_CTL = 0,
	YD5_PHYI_POWERUP_CTL,
	YD5_PHYI_MISC_0,	/* .. YD5_PHYI_MISC_0 + 8 */
	YD5_PHYI_PLL_REFCLK = YD5_PHYI_MISC_0 + YD5_HDMI_PHY_PLL_MISC_N,
	YD5_PHYI_RM_OFFSET,
	YD5_PHYI_PLL_VCOCLK_DIV,
	YD5_PHYI_PLL_CFG,
	YD5_PHYI_PLL_POST_KDIV,
	YD5_PHYI_CTL_0,
	YD5_PHYI_CTL_1,
	YD5_PHYI_CTL_2,
	YD5_PHYI_CTL_CK,
	YD5_PHYI_TMDS_CLK_WORD_SEL,
	YD5_PHYI_PLL_POWERUP_CTL,
	YD5_PHYI_PLL_RESET_CTL,
	YD5_PHYI_VID_CTL,	/* read for the validity check, never replayed */
	YD5_PHYI_COUNT
};

struct yd5_hdmi_phy_reg {
	const char *name;
	uint8_t block;		/* enum yd5_hdmi_phy_block */
	uint16_t offset;
	const char *cite;	/* facts s.10 step and the vc4_hdmi_phy.c lines */
};

/* The registers vc6_hdmi_phy_init touches, in the order it first writes
 * them (facts file section 10, steps 1-11), plus HDMI_VID_CTL. */
static const struct yd5_hdmi_phy_reg yd5_hdmi_phy_regs[YD5_PHYI_COUNT] = {
	[YD5_PHYI_RESET_CTL] = { "PHY_RESET_CTL", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_RESET_CTL, "s.10 steps 1,4; :945-952, :988" },
	[YD5_PHYI_POWERUP_CTL] = { "PHY_POWERUP_CTL", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_POWERUP_CTL, "s.10 steps 1,10; :945-952, :1177-1184" },
	[YD5_PHYI_MISC_0 + 0] = { "PHY_PLL_MISC_0", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x00u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 1] = { "PHY_PLL_MISC_1", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x04u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 2] = { "PHY_PLL_MISC_2", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x08u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 3] = { "PHY_PLL_MISC_3", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x0cu, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 4] = { "PHY_PLL_MISC_4", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x10u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 5] = { "PHY_PLL_MISC_5", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x14u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 6] = { "PHY_PLL_MISC_6", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x18u, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 7] = { "PHY_PLL_MISC_7", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x1cu, "s.10 step 2; :974-982" },
	[YD5_PHYI_MISC_0 + 8] = { "PHY_PLL_MISC_8", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_MISC_0 + 0x20u, "s.10 step 2; :974-982" },
	[YD5_PHYI_PLL_REFCLK] = { "PHY_PLL_REFCLK", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_REFCLK, "s.10 step 3; :984-986" },
	[YD5_PHYI_RM_OFFSET] = { "RM_OFFSET", YD5_HDMI_BLK_RM,
		YD5_HDMI_RM_OFFSET, "s.10 step 5; :990-993" },
	[YD5_PHYI_PLL_VCOCLK_DIV] = { "PHY_PLL_VCOCLK_DIV", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_VCOCLK_DIV, "s.10 step 6; :995-998" },
	[YD5_PHYI_PLL_CFG] = { "PHY_PLL_CFG", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_CFG, "s.10 step 7; :1000-1005" },
	[YD5_PHYI_PLL_POST_KDIV] = { "PHY_PLL_POST_KDIV", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_POST_KDIV, "s.10 steps 1,7; :945-952, :1000-1005" },
	[YD5_PHYI_CTL_0] = { "PHY_CTL_0", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_CTL_0, "s.10 step 8; :1010-1169" },
	[YD5_PHYI_CTL_1] = { "PHY_CTL_1", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_CTL_1, "s.10 step 8; :1010-1169" },
	[YD5_PHYI_CTL_2] = { "PHY_CTL_2", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_CTL_2, "s.10 step 8; :1010-1169" },
	[YD5_PHYI_CTL_CK] = { "PHY_CTL_CK", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_CTL_CK, "s.10 step 8; :1010-1169" },
	[YD5_PHYI_TMDS_CLK_WORD_SEL] = { "PHY_TMDS_CLK_WORD_SEL",
		YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_TMDS_CLK_WORD_SEL,
		"s.10 step 9; :1171-1175" },
	[YD5_PHYI_PLL_POWERUP_CTL] = { "PHY_PLL_POWERUP_CTL", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_POWERUP_CTL, "s.10 step 10; :1186-1187" },
	[YD5_PHYI_PLL_RESET_CTL] = { "PHY_PLL_RESET_CTL", YD5_HDMI_BLK_PHY,
		YD5_HDMI_PHY_PLL_RESET_CTL, "s.10 step 11; :1189-1195" },
	[YD5_PHYI_VID_CTL] = { "HDMI_VID_CTL", YD5_HDMI_BLK_HD,
		YD5_HDMI_HD_VID_CTL, "s.10 table; vc4_hdmi.c:3391-3399" },
};

/* --- capture --------------------------------------------------------------- */

typedef uint32_t (*yd5_hdmi_phy_read_fn)(void *ctx, unsigned block,
					 uint32_t offset);
typedef void (*yd5_hdmi_phy_write_fn)(void *ctx, unsigned block,
				      uint32_t offset, uint32_t value);

struct yd5_hdmi_phy_io {
	yd5_hdmi_phy_read_fn read;
	yd5_hdmi_phy_write_fn write;	/* only yd5_hdmi_phy_run() writes */
	void *ctx;
};

struct yd5_hdmi_phy_capture {
	uint32_t v[YD5_PHYI_COUNT];	/* indexed by enum yd5_hdmi_phy_idx */
};

/* Reads every register of yd5_hdmi_phy_regs[] once, in list order. */
static inline void yd5_hdmi_phy_capture(const struct yd5_hdmi_phy_io *io,
					struct yd5_hdmi_phy_capture *out)
{
	for (unsigned i = 0; i < YD5_PHYI_COUNT; i++)
		out->v[i] = io->read(io->ctx, yd5_hdmi_phy_regs[i].block,
				     yd5_hdmi_phy_regs[i].offset);
}

/* --- validity -------------------------------------------------------------- */

enum yd5_hdmi_phy_valid {
	YD5_PHY_CAPTURE_OK = 0,
	YD5_PHY_CAPTURE_HDMI_OFF,	/* HDMI_VID_CTL bit 31 clear */
	YD5_PHY_CAPTURE_IN_RESET,	/* PHY_RESET_CTL 0: PHY held in reset */
	YD5_PHY_CAPTURE_POWERED_DOWN,	/* PHY or PLL power-up register 0 */
	YD5_PHY_CAPTURE_PLL_RESET,	/* PLL_RESET_CTL 0: PLL held in reset */
	YD5_PHY_CAPTURE_BUS_ERROR,	/* a register read all ones */
};

/*
 * A capture can be replayed only when the firmware had HDMI0 running: with
 * HDMI off the PHY sits at its reset values, and replaying those programs
 * nothing. Reset and power-down are judged by the whole register being 0
 * (the state vc6_hdmi_reset_phy leaves, facts s.10 step 1, and the PLL
 * reset clear of step 11); the individual bit positions are not used here.
 * All-ones reads are taken as an unclocked or absent block (unverified: what
 * a read of an unclocked BCM2712 HDMI block returns is not known).
 */
static inline enum yd5_hdmi_phy_valid
yd5_hdmi_phy_capture_valid(const struct yd5_hdmi_phy_capture *c)
{
	for (unsigned i = 0; i < YD5_PHYI_COUNT; i++)
		if (c->v[i] == 0xffffffffu)
			return YD5_PHY_CAPTURE_BUS_ERROR;
	if (!(c->v[YD5_PHYI_VID_CTL] & YD5_HDMI_HD_VID_CTL_ENABLE))
		return YD5_PHY_CAPTURE_HDMI_OFF;
	if (c->v[YD5_PHYI_RESET_CTL] == 0)
		return YD5_PHY_CAPTURE_IN_RESET;
	if (c->v[YD5_PHYI_POWERUP_CTL] == 0 ||
	    c->v[YD5_PHYI_PLL_POWERUP_CTL] == 0)
		return YD5_PHY_CAPTURE_POWERED_DOWN;
	if (c->v[YD5_PHYI_PLL_RESET_CTL] == 0)
		return YD5_PHY_CAPTURE_PLL_RESET;
	return YD5_PHY_CAPTURE_OK;
}

/* --- replay plan ----------------------------------------------------------- */

enum yd5_hdmi_phy_op {
	YD5_PHY_OP_WRITE = 0,	/* write value to block:offset */
	YD5_PHY_OP_POLL = 1,	/* read until (r & mask) == value, max_polls */
};

struct yd5_hdmi_phy_step {
	uint8_t op;		/* enum yd5_hdmi_phy_op */
	uint8_t block;
	uint16_t offset;
	uint32_t value;
	uint32_t mask;		/* POLL only */
	uint32_t max_polls;	/* POLL only */
};

/* 2 reset + 9 misc + refclk + reset release + rm + vco + cfg + kdiv
 * + 4 channels + word sel + 2 power-up + 2 PLL reset + 1 poll */
#define YD5_HDMI_PHY_PLAN_LEN	27u
#define YD5_HDMI_PHY_LOCK_POLLS	1000u	/* default bound of the poll entry */

/*
 * The write list, in the order of facts file section 10:
 *
 *  1  PHY reset and power-down: PHY_RESET_CTL and PHY_POWERUP_CTL to 0.
 *     Linux also sets a bypass bit in PLL_POST_KDIV here; that bit's
 *     position is not taken over, so it is left out (unverified that the
 *     PLL tolerates skipping the bypass while it is reprogrammed).
 *  2  the nine PLL_MISC registers, captured values
 *  3  PLL_REFCLK, captured
 *  4  PHY_RESET_CTL, captured (Linux writes a fixed release value; the
 *     capture holds what the firmware left there)
 *  5  RM_OFFSET (rm block), captured
 *  6  PLL_VCOCLK_DIV, captured
 *  7  PLL_CFG, then PLL_POST_KDIV, captured
 *  8  PHY_CTL_0, _1, _2, _CK, captured
 *  9  TMDS_CLK_WORD_SEL, captured
 * 10  PHY_POWERUP_CTL, then PLL_POWERUP_CTL, captured
 * 11  PLL_RESET_CTL: 0 (PLL reset held), then the captured value (released).
 *     Writing the whole register as 0 rather than clearing one bit is
 *     unverified.
 * 12  a bounded poll. vc6_hdmi_phy_init has no lock wait (its last writes
 *     are the PLL reset at :1189-1195, then the unlock at :1197); this
 *     entry is ours, and since no lock-status bit is confirmed it reads back
 *     PLL_RESET_CTL until it equals the captured value (unverified as a lock
 *     indication). A later item may swap in a real lock bit: only the poll
 *     entry's block, offset, mask and value change.
 *
 * Returns the number of entries written (YD5_HDMI_PHY_PLAN_LEN), or 0 when
 * the capture is not valid or out is too small; out is untouched then.
 */
static inline size_t
yd5_hdmi_phy_replay_plan(const struct yd5_hdmi_phy_capture *c,
			 struct yd5_hdmi_phy_step *out, size_t cap)
{
	size_t n = 0;

	if (cap < YD5_HDMI_PHY_PLAN_LEN ||
	    yd5_hdmi_phy_capture_valid(c) != YD5_PHY_CAPTURE_OK)
		return 0;

#define YD5_PHY_W(idx, val) do { \
		out[n].op = YD5_PHY_OP_WRITE; \
		out[n].block = yd5_hdmi_phy_regs[idx].block; \
		out[n].offset = yd5_hdmi_phy_regs[idx].offset; \
		out[n].value = (val); \
		out[n].mask = 0; \
		out[n].max_polls = 0; \
		n++; \
	} while (0)
#define YD5_PHY_WC(idx) YD5_PHY_W(idx, c->v[idx])

	YD5_PHY_W(YD5_PHYI_RESET_CTL, 0u);			/* 1 */
	YD5_PHY_W(YD5_PHYI_POWERUP_CTL, 0u);
	for (unsigned i = 0; i < YD5_HDMI_PHY_PLL_MISC_N; i++)	/* 2 */
		YD5_PHY_WC(YD5_PHYI_MISC_0 + i);
	YD5_PHY_WC(YD5_PHYI_PLL_REFCLK);			/* 3 */
	YD5_PHY_WC(YD5_PHYI_RESET_CTL);				/* 4 */
	YD5_PHY_WC(YD5_PHYI_RM_OFFSET);				/* 5 */
	YD5_PHY_WC(YD5_PHYI_PLL_VCOCLK_DIV);			/* 6 */
	YD5_PHY_WC(YD5_PHYI_PLL_CFG);				/* 7 */
	YD5_PHY_WC(YD5_PHYI_PLL_POST_KDIV);
	YD5_PHY_WC(YD5_PHYI_CTL_0);				/* 8 */
	YD5_PHY_WC(YD5_PHYI_CTL_1);
	YD5_PHY_WC(YD5_PHYI_CTL_2);
	YD5_PHY_WC(YD5_PHYI_CTL_CK);
	YD5_PHY_WC(YD5_PHYI_TMDS_CLK_WORD_SEL);			/* 9 */
	YD5_PHY_WC(YD5_PHYI_POWERUP_CTL);			/* 10 */
	YD5_PHY_WC(YD5_PHYI_PLL_POWERUP_CTL);
	YD5_PHY_W(YD5_PHYI_PLL_RESET_CTL, 0u);			/* 11 */
	YD5_PHY_WC(YD5_PHYI_PLL_RESET_CTL);

#undef YD5_PHY_WC
#undef YD5_PHY_W

	out[n].op = YD5_PHY_OP_POLL;				/* 12 */
	out[n].block = YD5_HDMI_BLK_PHY;
	out[n].offset = YD5_HDMI_PHY_PLL_RESET_CTL;
	out[n].value = c->v[YD5_PHYI_PLL_RESET_CTL];
	out[n].mask = 0xffffffffu;
	out[n].max_polls = YD5_HDMI_PHY_LOCK_POLLS;
	n++;
	return n;
}

/* --- replay ---------------------------------------------------------------- */

/*
 * Replays a plan through io. A POLL entry reads at most max_polls times and
 * stops the replay when its condition never holds. Returns 0 when every
 * entry ran, else -(1 + index of the poll entry that timed out).
 */
static inline int yd5_hdmi_phy_run(const struct yd5_hdmi_phy_step *plan,
				   size_t n, const struct yd5_hdmi_phy_io *io)
{
	for (size_t i = 0; i < n; i++) {
		const struct yd5_hdmi_phy_step *s = &plan[i];

		if (s->op == YD5_PHY_OP_WRITE) {
			io->write(io->ctx, s->block, s->offset, s->value);
			continue;
		}
		uint32_t k = 0;

		while (k < s->max_polls &&
		       (io->read(io->ctx, s->block, s->offset) & s->mask) !=
			       (s->value & s->mask))
			k++;
		if (k == s->max_polls)
			return -(int)(i + 1);
	}
	return 0;
}

#endif /* YD5_HDMI_PHY_H */
