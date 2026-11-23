/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/hvs6_vblank.h against a scripted STATUS sequence:
 * a normal step, the 63->0 wrap, a step of 2, a timeout, budgets 0 and 1,
 * other STATUS bits changing without an FRCNT change, the time budget. The
 * io has no write function: the detector cannot write a register.
 *
 * Build: cc -std=c99 -Wall -Wextra -Werror -I../rpi5_scanout hvs6_vblank_mock.c
 * (tools/v3d/check71.d/hvs6_vblank.sh). Prints one line per check, then
 * "hvs6_vblank: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>

#include "hvs6_vblank.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

#define ST(frcnt, other) (((uint32_t)(frcnt) << 16) | (uint32_t)(other))

struct script {
	const uint32_t *v;
	unsigned int n, pos;
	uint32_t last_off;
	uint64_t clock; /* advances by tick per read */
	uint64_t tick;
};

static uint32_t rd(void *ctx, uint32_t off)
{
	struct script *s = ctx;
	uint32_t v = s->v[s->pos < s->n ? s->pos : s->n - 1];

	s->pos++;
	s->last_off = off;
	s->clock += s->tick;
	return v;
}

static uint64_t now(void *ctx)
{
	return ((struct script *)ctx)->clock;
}

static struct hvs6v_result run(struct script *s, const uint32_t *v, unsigned int n,
			       uint32_t max_polls, uint64_t tb, int timed)
{
	struct hvs6v_io io;
	struct hvs6v_budget b;

	io.read32 = rd;
	io.now = timed ? now : 0;
	io.ctx = s;
	b.max_polls = max_polls;
	b.time_budget = tb;
	s->v = v;
	s->n = n;
	s->pos = 0;
	s->clock = 0;
	s->tick = 10;
	return hvs6v_wait_frame(&io, &b);
}

int main(void)
{
	struct script s = {0, 0, 0, 0, 0, 0};
	struct hvs6v_result r;

	{ /* normal step 5 -> 6 on the third read */
		const uint32_t v[] = {ST(5, 0x10), ST(5, 0x20), ST(6, 0x30)};
		r = run(&s, v, 3, 10, 0, 0);
		check(r.code == HVS6V_OK && hvs6v_found(&r), "normal step: ok");
		check(r.frcnt_before == 5 && r.frcnt_after == 6 && r.step == 1 && r.missed == 0,
		      "normal step: 5 -> 6, step 1, none missed");
		check(r.reads == 3, "normal step: 3 reads");
		check(r.before_status == v[1] && r.after_status == v[2],
		      "normal step: raw STATUS before and after the step");
		check(!r.have_time, "normal step: no time hook, no elapsed");
		check(s.last_off == 0x118u, "reads STATUS of channel 0 (0x118)");
	}
	{ /* wrap */
		const uint32_t v[] = {ST(63, 0), ST(0, 0)};
		r = run(&s, v, 2, 10, 0, 0);
		check(r.code == HVS6V_OK && r.step == 1 && r.frcnt_before == 63 && r.frcnt_after == 0,
		      "wrap 63 -> 0 is a step of 1");
	}
	{ /* missed frame */
		const uint32_t v[] = {ST(10, 0), ST(12, 0)};
		r = run(&s, v, 2, 10, 0, 0);
		check(r.code == HVS6V_MISSED && hvs6v_found(&r) && r.step == 2 && r.missed == 1,
		      "step of 2 is reported as frames missed (1)");
	}
	{ /* missed across the wrap */
		const uint32_t v[] = {ST(62, 0), ST(1, 0)};
		r = run(&s, v, 2, 10, 0, 0);
		check(r.code == HVS6V_MISSED && r.step == 3 && r.missed == 2,
		      "62 -> 1 is a step of 3, 2 missed");
	}
	{ /* timeout */
		const uint32_t v[] = {ST(7, 1)};
		r = run(&s, v, 1, 5, 0, 0);
		check(r.code == HVS6V_TIMEOUT && !hvs6v_found(&r), "no step: timeout");
		check(r.reads == 6, "timeout: first read plus max_polls reads");
		check(r.step == 0 && r.frcnt_before == 7 && r.frcnt_after == 7,
		      "timeout: no step, FRCNT unchanged");
	}
	{ /* budget 0 */
		const uint32_t v[] = {ST(7, 0), ST(8, 0)};
		r = run(&s, v, 2, 0, 0, 0);
		check(r.code == HVS6V_TIMEOUT && r.reads == 1, "budget 0: one read, timeout");
	}
	{ /* budget 1: the step is on the one allowed poll */
		const uint32_t v[] = {ST(7, 0), ST(8, 0)};
		r = run(&s, v, 2, 1, 0, 0);
		check(r.code == HVS6V_OK && r.reads == 2, "budget 1: step on the first poll is found");
	}
	{ /* budget 1: the step is one poll too late */
		const uint32_t v[] = {ST(7, 0), ST(7, 0), ST(8, 0)};
		r = run(&s, v, 3, 1, 0, 0);
		check(r.code == HVS6V_TIMEOUT && r.reads == 2, "budget 1: step one poll late times out");
	}
	{ /* other bits change, FRCNT does not */
		const uint32_t v[] = {ST(9, 0x0001), ST(9, 0x0fff), ST(9, 0x6000), ST(9, 0x0002)};
		const uint32_t w[] = {ST(9, 0x0001), ST(9, 0x0fff), ST(9, 0x4abc), ST(10, 0x0003)};

		r = run(&s, v, 4, 20, 0, 0);
		check(r.code == HVS6V_TIMEOUT && r.step == 0,
		      "YLINE/MODE/EMPTY bits changing without FRCNT is not a frame");
		check(r.reads == 21 && r.frcnt_after == 9, "other bits: polled to the budget");
		r = run(&s, w, 4, 20, 0, 0);
		check(r.code == HVS6V_OK && r.reads == 4, "other bits then a real step: found on the step");
		check(r.before_status == w[2] && r.after_status == w[3],
		      "other bits: raw STATUS before the step is the last pre-step read");
	}
	{ /* time hook */
		const uint32_t v[] = {ST(1, 0), ST(1, 0), ST(2, 0)};
		r = run(&s, v, 3, 100, 0, 1);
		check(r.have_time && r.elapsed == 30, "time hook: elapsed spans the reads (3 x tick)");
	}
	{ /* time budget ends the wait before max_polls */
		const uint32_t v[] = {ST(1, 0)};
		r = run(&s, v, 1, 1000, 45, 1);
		check(r.code == HVS6V_TIMEOUT && r.reads < 1000 && r.have_time,
		      "time budget: times out before max_polls");
		check(r.elapsed >= 45, "time budget: elapsed reached the budget");
	}
	{ /* time budget ignored without a hook */
		const uint32_t v[] = {ST(1, 0)};
		r = run(&s, v, 1, 4, 1, 0);
		check(r.code == HVS6V_TIMEOUT && r.reads == 5 && !r.have_time,
		      "time budget without a hook: max_polls bounds the wait");
	}
	{ /* bad io */
		struct hvs6v_io io = {0, 0, 0};
		struct hvs6v_budget b = {1, 0};

		r = hvs6v_wait_frame(&io, &b);
		check(r.code == HVS6V_BADIO && r.reads == 0, "no read32: bad io, nothing read");
	}
	check(hvs6v_step(63, 0) == 1 && hvs6v_step(0, 63) == 63 && hvs6v_step(5, 5) == 0,
	      "hvs6v_step is 6-bit wrap aware");
	check(hvs6v_frcnt(0x003fffffu) == 63 && hvs6v_frcnt(0xffc0ffffu) == 0,
	      "hvs6v_frcnt takes bits 21:16 only");
	check(hvs6v_name(HVS6V_OK) && hvs6v_name(HVS6V_MISSED) && hvs6v_name(HVS6V_TIMEOUT) &&
		      hvs6v_name(HVS6V_BADIO),
	      "names exist for every code");

	printf("hvs6_vblank: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
