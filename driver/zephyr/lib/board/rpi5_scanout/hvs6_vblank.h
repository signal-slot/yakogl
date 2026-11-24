/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_vblank: find a new frame of BCM2712 HVS (scaler6, stepping D0)
 * channel 0 by reads only. Plan #390, item vblank-detect, for the vblank
 * list-pointer swap of docs/notes/rpi5-display.md (Q4): the caller writes
 * LPTRS just after hvs6v_wait_frame returns.
 *
 * The detector reads channel 0's STATUS once, then polls until STATUS.FRCNT
 * differs from that first value. The moment FRCNT changes is a frame edge
 * seen with the poll interval's resolution, no better.
 *
 * Header only, C99, no Zephyr includes. Register access goes through struct
 * hvs6v_io (offsets from the HVS base, as struct hvs6s_io in
 * hvs6_dlist_swap.h), so the host mock (driver/zephyr/lib/board/test/
 * hvs6_vblank_mock.c, tools/v3d/check71.d/hvs6_vblank.sh) runs the same code
 * against a scripted STATUS sequence. struct hvs6v_io has read32 and no
 * write32: this header writes no register. In particular it never touches
 * PV_INTSTAT (pv_wait_vblank clears it, a write) and does not use pv_irq.h.
 *
 * Facts and where they come from:
 *  - Channel 0 STATUS on D0 is DISP0_STATUS at 0x118
 *    (hvs6_dlist.h:77-82, HVS6D_DISP0_STATUS; hvs6_chan.h YD5_HVS6D_*).
 *  - FRCNT is STATUS bits 21:16, six bits, so it wraps at 64
 *    (rpi:vc4_regs.h:605; hvs6_dlist.h:98-104 HVS6_STATUS_FRCNT_*;
 *    hvs6_dlist_swap.h:190-192 HVS6S_FRCNT_*; scanout_alive.h:61-64
 *    SA_STATUS_FRCNT_*). It is the only frame or line field seen to move on
 *    the board (vblank-regs facts check of plan #390; rpi5-display.md).
 *  - Linux itself compares FRCNT to age a list (rpi:vc4_hvs.c:682-697,
 *    quoted at hvs6_dlist_swap.h:72-74): the same field, the same 6-bit wrap.
 *  - STATUS.YLINE (bits 12:0) is NOT used: its range and how it counts are
 *    unproven. The raw STATUS words seen at the step are reported unchanged
 *    (before_status, after_status) so a caller or probe can print them;
 *    nothing here decides on them. A change of any other STATUS bit without
 *    an FRCNT change does not count as a frame.
 *
 * A step of more than one (polls too far apart, or the caller was preempted)
 * is still a detected edge, but it is reported, not hidden: HVS6V_MISSED,
 * step > 1, missed = step - 1. hvs6v_found() is true for HVS6V_OK and
 * HVS6V_MISSED; the caller decides whether a late edge is good enough.
 *
 * Budget: max_polls reads after the first one (0 allows none: the first
 * read is taken, there is nothing to compare, HVS6V_TIMEOUT), and
 * time_budget in the units of io->now when io->now is set (0 = no time
 * bound). The first limit reached ends the wait with HVS6V_TIMEOUT. One
 * frame at 60 Hz is 16.7 ms: a budget of a few frames suffices.
 */
#ifndef HVS6_VBLANK_H
#define HVS6_VBLANK_H

#include <stdint.h>

/* DISP0_STATUS of channel 0, stepping D0 (hvs6_dlist.h:82). */
#define HVS6V_STATUS_OFF 0x0118u
/* FRCNT, STATUS bits 21:16 (rpi:vc4_regs.h:605). */
#define HVS6V_FRCNT_SHIFT 16u
#define HVS6V_FRCNT_MASK 0x3fu

struct hvs6v_io {
	uint32_t (*read32)(void *ctx, uint32_t off);
	/* Optional monotonic time, any unit (that of time_budget); NULL: no
	 * elapsed time, only max_polls bounds the wait. */
	uint64_t (*now)(void *ctx);
	void *ctx;
};

struct hvs6v_budget {
	uint32_t max_polls;   /* reads after the first; 0 allows none */
	uint64_t time_budget; /* in io->now units; 0 = no time bound */
};

enum hvs6v_code {
	HVS6V_OK = 0,   /* FRCNT stepped by exactly 1 */
	HVS6V_MISSED,   /* FRCNT stepped by 2..63: frames missed */
	HVS6V_TIMEOUT,  /* no FRCNT change within the budget */
	HVS6V_BADIO     /* no io, no read32 or no budget; nothing was read */
};

struct hvs6v_result {
	enum hvs6v_code code;
	uint32_t frcnt_before;  /* FRCNT of the first read */
	uint32_t frcnt_after;   /* FRCNT of the first read that differed; the
				 * first FRCNT on a timeout */
	uint32_t reads;         /* STATUS reads made, the first included */
	uint32_t step;          /* (after - before) & 63; 0 on a timeout */
	uint32_t missed;        /* step - 1 when step > 1, else 0 */
	uint32_t before_status; /* raw STATUS of the last read before the step
				 * (the last read on a timeout) */
	uint32_t after_status;  /* raw STATUS of the first read after it
				 * (the last read on a timeout) */
	int have_time;          /* elapsed is valid */
	uint64_t elapsed;       /* now() at the end - now() at the start */
};

static inline const char *hvs6v_name(enum hvs6v_code c)
{
	switch (c) {
	case HVS6V_OK: return "ok";
	case HVS6V_MISSED: return "frames missed";
	case HVS6V_TIMEOUT: return "timeout";
	case HVS6V_BADIO: return "bad io";
	}
	return "?";
}

/* 1 when the edge was seen (HVS6V_OK or HVS6V_MISSED). */
static inline int hvs6v_found(const struct hvs6v_result *r)
{
	return r->code == HVS6V_OK || r->code == HVS6V_MISSED;
}

static inline uint32_t hvs6v_frcnt(uint32_t status)
{
	return (status >> HVS6V_FRCNT_SHIFT) & HVS6V_FRCNT_MASK;
}

/* 6-bit wrap-aware step from a to b. */
static inline uint32_t hvs6v_step(uint32_t a, uint32_t b)
{
	return (b - a) & HVS6V_FRCNT_MASK;
}

/* Read STATUS, poll until FRCNT differs, within the budget. Reads only. */
static inline struct hvs6v_result hvs6v_wait_frame(const struct hvs6v_io *io,
						   const struct hvs6v_budget *bud)
{
	struct hvs6v_result r = {HVS6V_BADIO, 0, 0, 0, 0, 0, 0, 0, 0, 0};
	uint64_t t0 = 0;
	uint32_t st, first;

	if (!io || !io->read32 || !bud)
		return r;
	r.have_time = io->now != 0;
	if (r.have_time)
		t0 = io->now(io->ctx);

	st = io->read32(io->ctx, HVS6V_STATUS_OFF);
	r.reads = 1;
	first = hvs6v_frcnt(st);
	r.frcnt_before = first;
	r.frcnt_after = first;
	r.before_status = st;
	r.after_status = st;
	r.code = HVS6V_TIMEOUT;

	while (r.reads - 1 < bud->max_polls) {
		if (r.have_time && bud->time_budget &&
		    io->now(io->ctx) - t0 >= bud->time_budget)
			break;
		st = io->read32(io->ctx, HVS6V_STATUS_OFF);
		r.reads++;
		if (hvs6v_frcnt(st) != first) {
			r.after_status = st; /* before_status: the read before */
			r.frcnt_after = hvs6v_frcnt(st);
			r.step = hvs6v_step(first, r.frcnt_after);
			r.missed = r.step > 1 ? r.step - 1 : 0;
			r.code = r.step > 1 ? HVS6V_MISSED : HVS6V_OK;
			break;
		}
		r.before_status = st;
		r.after_status = st;
	}
	if (r.have_time)
		r.elapsed = io->now(io->ctx) - t0;
	return r;
}

#endif /* HVS6_VBLANK_H */
