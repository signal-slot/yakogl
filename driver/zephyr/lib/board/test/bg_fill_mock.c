/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/bg_fill.h against a mock HVS register file:
 * each channel and stepping, masks that keep other bits, restore byte for
 * byte, and the refused inputs.
 *
 * Build: cc -std=c11 -Wall -Wextra -Werror -I../rpi5_scanout bg_fill_mock.c
 * (tools/v3d/check71.d/bg_fill.sh). Prints one line per check, then
 * "bg_fill_mock: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>
#include <string.h>

#include "bg_fill.h"

#define NREGS 256u	/* 0x400 bytes, covers every channel of both steppings */

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

static void fill_regs(uint32_t *r)
{
	unsigned int i;

	for (i = 0; i < NREGS; i++)
		r[i] = 0xa5a5a5a5u ^ (i * 0x01010101u);
}

static void apply(uint32_t *r, const struct yd5_bgf_plan *p)
{
	unsigned int i;

	for (i = 0; i < p->n; i++)
		r[p->w[i].offset / 4u] = yd5_bgf_merge(r[p->w[i].offset / 4u], &p->w[i]);
}

static void one(enum yd5_bgf_variant v, unsigned int ch, const char *tag)
{
	uint32_t regs[NREGS], orig[NREGS], saved[YD5_BGF_MAX_WRITES];
	struct yd5_bgf_plan p, rs;
	uint32_t ctrl1, bgnd;
	unsigned int i;
	char name[96];
	int diff = 0;

	yd5_bgf_regs(v, ch, &ctrl1, &bgnd);
	fill_regs(regs);
	regs[ctrl1 / 4u] = 0xffff0001u;	/* INTLACE set, BGENB clear, rest set */
	regs[bgnd / 4u] = 0xdeadbeefu;
	memcpy(orig, regs, sizeof(regs));

	snprintf(name, sizeof(name), "%s ch%u: plan ok, BGND then CTRL1", tag, ch);
	check(yd5_bgf_plan_v(v, ch, 0x123456u, &p) == 0 && p.n == 2 &&
	      p.w[0].offset == bgnd && p.w[1].offset == ctrl1, name);
	snprintf(name, sizeof(name), "%s ch%u: masks 23:0 and bit 8", tag, ch);
	check(p.w[0].mask == 0x00ffffffu && p.w[0].value == 0x123456u &&
	      p.w[1].mask == (1u << 8) && p.w[1].value == (1u << 8), name);

	for (i = 0; i < p.n; i++)
		saved[i] = regs[p.w[i].offset / 4u];
	apply(regs, &p);
	snprintf(name, sizeof(name), "%s ch%u: colour set, top byte kept", tag, ch);
	check(regs[bgnd / 4u] == 0xde123456u, name);
	snprintf(name, sizeof(name), "%s ch%u: BGENB set, other CTRL1 bits kept", tag, ch);
	check(regs[ctrl1 / 4u] == 0xffff0101u, name);
	for (i = 0; i < NREGS; i++)
		if (regs[i] != orig[i] && i != bgnd / 4u && i != ctrl1 / 4u)
			diff++;
	snprintf(name, sizeof(name), "%s ch%u: no other register changed", tag, ch);
	check(diff == 0, name);

	snprintf(name, sizeof(name), "%s ch%u: restore returns the file byte for byte", tag, ch);
	check(yd5_bgf_restore(&p, saved, &rs) == 0 && rs.n == 2 &&
	      rs.w[0].offset == ctrl1 && rs.w[1].offset == bgnd, name);
	apply(regs, &rs);
	check(memcmp(regs, orig, sizeof(regs)) == 0, name);
}

int main(void)
{
	static const char *const vn[] = { "C0", "D0" };
	struct yd5_bgf_plan p, rs;
	uint32_t saved[2] = { 1, 2 };
	uint32_t ctrl1, bgnd;
	unsigned int ch;
	int v;

	for (v = 0; v < 2; v++)
		for (ch = 0; ch < YD5_BGF_NUM_CHANNELS; ch++)
			one((enum yd5_bgf_variant)v, ch, vn[v]);

	check(yd5_bgf_plan(0, 0, &p) == 0 && p.w[0].offset == 0x38u && p.w[1].offset == 0x34u,
	      "C0 ch0: BGND 0x38, CTRL1 0x34");
	yd5_bgf_regs(YD5_BGF_D0, 2, &ctrl1, &bgnd);
	check(bgnd == 0x188u && ctrl1 == 0x184u, "D0 ch2: BGND 0x188, CTRL1 0x184");

	p.n = 7;
	check(yd5_bgf_plan(3, 0, &p) == YD5_BGF_EINVAL && p.n == 0, "channel 3 refused, plan empty");
	check(yd5_bgf_plan(0xffffffffu, 0, &p) == YD5_BGF_EINVAL, "huge channel refused");
	check(yd5_bgf_plan(0, 0x1000000u, &p) == YD5_BGF_EINVAL && p.n == 0, "rgb above 24 bits refused");
	check(yd5_bgf_plan_v((enum yd5_bgf_variant)2, 0, 0, &p) == YD5_BGF_EINVAL, "unknown stepping refused");
	check(yd5_bgf_plan(0, 0, NULL) == YD5_BGF_EINVAL, "null out refused");
	yd5_bgf_plan(0, 0, &p);
	check(yd5_bgf_restore(NULL, saved, &rs) == YD5_BGF_EINVAL &&
	      yd5_bgf_restore(&p, NULL, &rs) == YD5_BGF_EINVAL &&
	      yd5_bgf_restore(&p, saved, NULL) == YD5_BGF_EINVAL &&
	      yd5_bgf_restore(&p, saved, &p) == YD5_BGF_EINVAL,
	      "restore refuses null and aliased arguments");
	p.n = 3;
	check(yd5_bgf_restore(&p, saved, &rs) == YD5_BGF_EINVAL, "restore refuses an over-long plan");

	printf("bg_fill_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
