/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/hdmi_phy.h: capture from a fake register file,
 * the replay plan's order and values, refused captures, and the bounded poll.
 * The register values are made up for the test (a pattern of block and
 * offset); none is a hardware or Linux value.
 *
 * Build: cc -std=c11 -Wall -Wextra -Werror -I../rpi5_scanout hdmi_phy_mock.c
 * (tools/v3d/check71.d/hdmi_phy.sh). Prints one line per check, then
 * "hdmi_phy_mock: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>
#include <string.h>

#include "hdmi_phy.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

/* --- fake register file ------------------------------------------------------ */

#define NBLK 3
#define NWORDS (0x200 / 4)
#define NLOG 64

struct fake {
	uint32_t r[NBLK][NWORDS];
	unsigned nreads;
	struct { unsigned block; uint32_t offset, value; } w[NLOG];
	unsigned nw;
	int stuck_pll;		/* PLL_RESET_CTL always reads 0 */
	unsigned bad_read;	/* reads of an unknown block or offset */
};

static uint32_t f_read(void *ctx, unsigned block, uint32_t offset)
{
	struct fake *f = ctx;

	f->nreads++;
	if (block >= NBLK || offset / 4 >= NWORDS || offset % 4) {
		f->bad_read++;
		return 0xdeadbeefu;
	}
	if (f->stuck_pll && block == YD5_HDMI_BLK_PHY &&
	    offset == YD5_HDMI_PHY_PLL_RESET_CTL)
		return 0;
	return f->r[block][offset / 4];
}

static void f_write(void *ctx, unsigned block, uint32_t offset, uint32_t value)
{
	struct fake *f = ctx;

	if (f->nw < NLOG) {
		f->w[f->nw].block = block;
		f->w[f->nw].offset = offset;
		f->w[f->nw].value = value;
	}
	f->nw++;
	if (block < NBLK && offset / 4 < NWORDS)
		f->r[block][offset / 4] = value;
}

static uint32_t pat(unsigned block, uint32_t offset)
{
	return 0x5a000000u | (block << 16) | offset | 1u;
}

/* A register file as the firmware might leave it with HDMI0 running. */
static void fake_running(struct fake *f)
{
	memset(f, 0, sizeof(*f));
	for (unsigned b = 0; b < NBLK; b++)
		for (unsigned i = 0; i < NWORDS; i++)
			f->r[b][i] = pat(b, i * 4);
	f->r[YD5_HDMI_BLK_HD][YD5_HDMI_HD_VID_CTL / 4] |= YD5_HDMI_HD_VID_CTL_ENABLE;
}

static int step_w(const struct yd5_hdmi_phy_step *s, unsigned blk, uint32_t off,
		  uint32_t val)
{
	return s->op == YD5_PHY_OP_WRITE && s->block == blk && s->offset == off &&
	       s->value == val;
}

int main(void)
{
	struct fake f;
	struct yd5_hdmi_phy_io io = { f_read, f_write, &f };
	struct yd5_hdmi_phy_capture c;
	struct yd5_hdmi_phy_step plan[YD5_HDMI_PHY_PLAN_LEN + 4];
	size_t n;
	char name[128];

	/* the register list */
	{
		int uniq = 1, aligned = 1, cited = 1;

		for (unsigned i = 0; i < YD5_PHYI_COUNT; i++) {
			const struct yd5_hdmi_phy_reg *a = &yd5_hdmi_phy_regs[i];

			if (a->offset % 4 || a->offset >= 0x200)
				aligned = 0;
			if (!a->name || !a->cite || !a->cite[0])
				cited = 0;
			for (unsigned j = i + 1; j < YD5_PHYI_COUNT; j++)
				if (a->block == yd5_hdmi_phy_regs[j].block &&
				    a->offset == yd5_hdmi_phy_regs[j].offset)
					uniq = 0;
		}
		check(YD5_PHYI_COUNT == 24, "list: 22 PHY + RM_OFFSET + HDMI_VID_CTL");
		check(uniq, "list: no block:offset twice");
		check(aligned, "list: offsets word aligned, inside 0x200");
		check(cited, "list: every entry named and cited");
		check(yd5_hdmi_phy_regs[YD5_PHYI_RM_OFFSET].block == YD5_HDMI_BLK_RM &&
		      yd5_hdmi_phy_regs[YD5_PHYI_VID_CTL].block == YD5_HDMI_BLK_HD,
		      "list: RM_OFFSET in rm, VID_CTL in hd");
		check(yd5_hdmi_phy_regs[YD5_PHYI_MISC_0 + 8].offset ==
		      YD5_HDMI_PHY_PLL_MISC_0 + 0x20u, "list: PLL_MISC_8 at MISC_0 + 0x20");
	}

	/* capture */
	fake_running(&f);
	yd5_hdmi_phy_capture(&io, &c);
	{
		int same = 1;

		for (unsigned i = 0; i < YD5_PHYI_COUNT; i++)
			if (c.v[i] != f.r[yd5_hdmi_phy_regs[i].block]
					 [yd5_hdmi_phy_regs[i].offset / 4])
				same = 0;
		check(same, "capture: every value equals the fake register");
	}
	check(f.nreads == YD5_PHYI_COUNT && f.bad_read == 0,
	      "capture: one read per register, none out of range");
	check(f.nw == 0, "capture: writes nothing");
	check(yd5_hdmi_phy_capture_valid(&c) == YD5_PHY_CAPTURE_OK,
	      "valid: running HDMI0 accepted");

	/* plan */
	n = yd5_hdmi_phy_replay_plan(&c, plan, sizeof(plan) / sizeof(plan[0]));
	check(n == YD5_HDMI_PHY_PLAN_LEN && n == 27, "plan: 27 entries");
	check(step_w(&plan[0], YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_RESET_CTL, 0) &&
	      step_w(&plan[1], YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_POWERUP_CTL, 0),
	      "plan 1: PHY reset and power-down to 0 first");
	{
		int ok = 1;

		for (unsigned i = 0; i < 9; i++)
			if (!step_w(&plan[2 + i], YD5_HDMI_BLK_PHY,
				    YD5_HDMI_PHY_PLL_MISC_0 + 4 * i,
				    pat(YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_MISC_0 + 4 * i)))
				ok = 0;
		check(ok, "plan 2: PLL_MISC_0..8 in order, captured values");
	}
	{
		static const struct { unsigned blk; uint32_t off; const char *what; } seq[] = {
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_REFCLK, "3: PLL_REFCLK" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_RESET_CTL, "4: PHY reset release" },
			{ YD5_HDMI_BLK_RM, YD5_HDMI_RM_OFFSET, "5: RM_OFFSET (rm block)" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_VCOCLK_DIV, "6: PLL_VCOCLK_DIV" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_CFG, "7: PLL_CFG" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_POST_KDIV, "7: PLL_POST_KDIV" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_CTL_0, "8: PHY_CTL_0" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_CTL_1, "8: PHY_CTL_1" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_CTL_2, "8: PHY_CTL_2" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_CTL_CK, "8: PHY_CTL_CK" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_TMDS_CLK_WORD_SEL, "9: TMDS_CLK_WORD_SEL" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_POWERUP_CTL, "10: PHY_POWERUP_CTL" },
			{ YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_POWERUP_CTL, "10: PLL_POWERUP_CTL" },
		};
		for (unsigned i = 0; i < sizeof(seq) / sizeof(seq[0]); i++) {
			snprintf(name, sizeof(name), "plan %s at %u, captured value",
				 seq[i].what, 11 + i);
			check(step_w(&plan[11 + i], seq[i].blk, seq[i].off,
				     pat(seq[i].blk, seq[i].off)), name);
		}
	}
	check(step_w(&plan[24], YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_RESET_CTL, 0) &&
	      step_w(&plan[25], YD5_HDMI_BLK_PHY, YD5_HDMI_PHY_PLL_RESET_CTL,
		     c.v[YD5_PHYI_PLL_RESET_CTL]),
	      "plan 11: PLL reset 0, then captured value");
	check(plan[26].op == YD5_PHY_OP_POLL && plan[26].block == YD5_HDMI_BLK_PHY &&
	      plan[26].offset == YD5_HDMI_PHY_PLL_RESET_CTL &&
	      plan[26].value == c.v[YD5_PHYI_PLL_RESET_CTL] &&
	      plan[26].max_polls == YD5_HDMI_PHY_LOCK_POLLS && plan[26].max_polls > 0,
	      "plan 12: last entry is a bounded poll");
	{
		int vid = 0;

		for (size_t i = 0; i < n; i++)
			if (plan[i].block == YD5_HDMI_BLK_HD)
				vid = 1;
		check(!vid, "plan: HDMI_VID_CTL is never written");
	}
	{
		/* every write value is 0 or a captured value: nothing else */
		int ok = 1;

		for (size_t i = 0; i < n; i++) {
			int found = plan[i].value == 0;

			for (unsigned j = 0; j < YD5_PHYI_COUNT; j++)
				if (plan[i].value == c.v[j])
					found = 1;
			if (!found)
				ok = 0;
		}
		check(ok, "plan: every value is 0 or captured");
	}
	check(yd5_hdmi_phy_replay_plan(&c, plan, YD5_HDMI_PHY_PLAN_LEN - 1) == 0,
	      "plan: refused into a short buffer");

	/* replay onto a reset register file */
	{
		struct yd5_hdmi_phy_step p2[YD5_HDMI_PHY_PLAN_LEN];
		size_t n2 = yd5_hdmi_phy_replay_plan(&c, p2, YD5_HDMI_PHY_PLAN_LEN);
		int r, same = 1, order = 1;

		memset(f.r, 0, sizeof(f.r));
		f.nw = 0;
		f.nreads = 0;
		r = yd5_hdmi_phy_run(p2, n2, &io);
		check(r == 0, "run: completes, poll satisfied");
		check(f.nw == 26, "run: 26 writes");
		check(f.nreads == 1, "run: poll satisfied on the first read");
		for (unsigned i = 0; i < YD5_PHYI_COUNT; i++) {
			if (i == YD5_PHYI_VID_CTL)
				continue;
			if (f.r[yd5_hdmi_phy_regs[i].block][yd5_hdmi_phy_regs[i].offset / 4] !=
			    c.v[i])
				same = 0;
		}
		check(same, "run: every PHY/PLL/RM register ends at its captured value");
		for (unsigned i = 0; i < 26 && i < NLOG; i++)
			if (f.w[i].block != p2[i].block || f.w[i].offset != p2[i].offset ||
			    f.w[i].value != p2[i].value)
				order = 0;
		check(order, "run: writes issued in plan order");
	}

	/* bounded poll times out */
	{
		struct yd5_hdmi_phy_step p3[YD5_HDMI_PHY_PLAN_LEN];
		size_t n3 = yd5_hdmi_phy_replay_plan(&c, p3, YD5_HDMI_PHY_PLAN_LEN);
		int r;

		p3[n3 - 1].max_polls = 7;
		f.stuck_pll = 1;
		f.nreads = 0;
		f.nw = 0;
		r = yd5_hdmi_phy_run(p3, n3, &io);
		check(r == -(int)n3, "poll: stuck PLL returns -(index + 1) of the poll");
		check(f.nreads == 7, "poll: exactly max_polls reads, then gives up");
		f.stuck_pll = 0;

		p3[n3 - 1].max_polls = 0;
		f.nreads = 0;
		r = yd5_hdmi_phy_run(p3, n3, &io);
		check(r == -(int)n3 && f.nreads == 0, "poll: max_polls 0 times out at once");
	}

	/* refused captures */
	{
		struct yd5_hdmi_phy_capture b;
		static const struct { unsigned idx; uint32_t v; enum yd5_hdmi_phy_valid want;
				      const char *what; } bad[] = {
			{ YD5_PHYI_VID_CTL, 0x0000ff00u, YD5_PHY_CAPTURE_HDMI_OFF,
			  "VID_CTL bit 31 clear: HDMI off" },
			{ YD5_PHYI_RESET_CTL, 0, YD5_PHY_CAPTURE_IN_RESET, "PHY in reset" },
			{ YD5_PHYI_POWERUP_CTL, 0, YD5_PHY_CAPTURE_POWERED_DOWN, "PHY powered down" },
			{ YD5_PHYI_PLL_POWERUP_CTL, 0, YD5_PHY_CAPTURE_POWERED_DOWN,
			  "PLL powered down" },
			{ YD5_PHYI_PLL_RESET_CTL, 0, YD5_PHY_CAPTURE_PLL_RESET, "PLL in reset" },
			{ YD5_PHYI_CTL_1, 0xffffffffu, YD5_PHY_CAPTURE_BUS_ERROR,
			  "all-ones read: bus error" },
		};
		for (unsigned i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
			b = c;
			b.v[bad[i].idx] = bad[i].v;
			snprintf(name, sizeof(name), "refused: %s, no plan", bad[i].what);
			check(yd5_hdmi_phy_capture_valid(&b) == bad[i].want &&
			      yd5_hdmi_phy_replay_plan(&b, plan, YD5_HDMI_PHY_PLAN_LEN) == 0, name);
		}
		/* the all-reset capture: a register file of zeros */
		memset(f.r, 0, sizeof(f.r));
		yd5_hdmi_phy_capture(&io, &b);
		check(yd5_hdmi_phy_capture_valid(&b) == YD5_PHY_CAPTURE_HDMI_OFF &&
		      yd5_hdmi_phy_replay_plan(&b, plan, YD5_HDMI_PHY_PLAN_LEN) == 0,
		      "refused: all-zero register file (HDMI never brought up)");
	}

	printf("hdmi_phy_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
