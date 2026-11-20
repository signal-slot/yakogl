/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout32: decision logic of the list-pointer swap and the restore
 * (docs/notes/rpi5-display.md section 3 and Q4). Pure functions, no I/O, no
 * Zephyr: stdint/stddef/stdbool only, host-tested by test/swap_check_test.c.
 *
 * linuxdesk found no latch bit in the pinned tree: the HVS takes the channel's
 * list pointer at the next frame start, so a swap is judged by the pointer
 * word read back after FRCNT advanced, not by a latch flag.
 *
 * Constants are repeated from driver/zephyr/lib/board/rpi5_scanout/ so the
 * header stands alone:
 *   hvs6_chan.h:94     YD5_HVS6_LPTRS_HEADE_MASK 0xfff (list start, in words)
 *   hvs6_chan.h:107    YD5_HVS6_BOOTLOADER_DLIST_END 32 (firmware list words)
 *   scanout_alive.h:63 STATUS.FRCNT 21:16 (SA_STATUS_FRCNT_SHIFT/_MASK)
 *   scanout_alive.h:77 SA_PV_CONTROL_EN (bit 0)
 *   scanout_alive.h:79 SA_PV_VCONTROL_VIDEN (bit 0)
 *   hvs6_dlist.h:163   HVS6_W_PTR_UNITY 5 (PTR0 word index in a unity entry;
 *                      one more when POS1 is present), HVS6_PTR_WORDS 3
 * (the citefix item rechecks the line numbers of hvs6_chan.h/hvs6_dlist.h.)
 */
#ifndef RPI5_SCANOUT32_SWAP_CHECK_H
#define RPI5_SCANOUT32_SWAP_CHECK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SWAP_LPTRS_MASK 0xfffu
#define SWAP_FW_LIST_WORDS 32u
#define SWAP_FRCNT_SHIFT 16u
#define SWAP_FRCNT_MASK 0x3fu
#define SWAP_PV_CONTROL_EN (1u << 0)
#define SWAP_PV_VCONTROL_VIDEN (1u << 0)
#define SWAP_UNCLOCKED 0xffffffffu
#define SWAP_W_PTR_UNITY 5u

static inline uint32_t swap_frcnt(uint32_t status)
{
	return (status >> SWAP_FRCNT_SHIFT) & SWAP_FRCNT_MASK;
}

/* Frames between two STATUS reads, 6-bit wrap (63 -> 0 is one). */
static inline uint32_t swap_frames(uint32_t status_before, uint32_t status_after)
{
	return (swap_frcnt(status_after) - swap_frcnt(status_before)) & SWAP_FRCNT_MASK;
}

/* Word index of the plane's first pointer word (PTR0) within our list: the
 * entry start plus 5 (unity scale) or 6 (scaled, one POS1 more). */
static inline uint32_t swap_ptr_word(uint32_t entry_start, bool unity)
{
	return entry_start + SWAP_W_PTR_UNITY + (unity ? 0u : 1u);
}

/* (a) One swap. */
enum swap_reason {
	SWAP_OURS = 0,         /* pointer named ours (pass) */
	SWAP_UNCLOCKED_FAIL,   /* a register read all ones */
	SWAP_VALVE_OFF,        /* PV_CONTROL.EN or PV_V_CONTROL.VIDEN clear */
	SWAP_FRCNT_STALLED,    /* FRCNT did not advance */
	SWAP_BAD_PLACEMENT,    /* expected word inside the firmware's 32 words */
	SWAP_NOT_LATCHED,      /* pointer did not name ours after the wait */
};

struct swap_sample {
	uint32_t ptr_before;      /* LPTRS/DL word read before the write */
	uint32_t ptr_after;       /* the same register after the wait */
	uint32_t expected_word;   /* word offset of our list, >= 32 */
	uint32_t status_before;   /* DISPx_STATUS before the write */
	uint32_t status_after;    /* DISPx_STATUS after the wait */
	uint32_t pv_control;
	uint32_t pv_vcontrol;
};

static inline const char *swap_reason_name(enum swap_reason r)
{
	switch (r) {
	case SWAP_OURS:
		return "pointer-named-ours";
	case SWAP_UNCLOCKED_FAIL:
		return "unclocked";
	case SWAP_VALVE_OFF:
		return "valve-off";
	case SWAP_FRCNT_STALLED:
		return "frcnt-stalled";
	case SWAP_BAD_PLACEMENT:
		return "bad-placement";
	case SWAP_NOT_LATCHED:
		return "pointer-not-latched";
	}
	return "?";
}

/* Order: unclocked, valve, FRCNT, placement, pointer. The caller samples
 * after the wait; a pointer that follows only at the next FRCNT edge is
 * therefore read as ours, one that never follows is not-latched. */
static inline enum swap_reason swap_judge(const struct swap_sample *s)
{
	if (s->status_before == SWAP_UNCLOCKED || s->status_after == SWAP_UNCLOCKED ||
	    s->pv_control == SWAP_UNCLOCKED || s->pv_vcontrol == SWAP_UNCLOCKED ||
	    s->ptr_after == SWAP_UNCLOCKED)
		return SWAP_UNCLOCKED_FAIL;
	if (!(s->pv_control & SWAP_PV_CONTROL_EN) || !(s->pv_vcontrol & SWAP_PV_VCONTROL_VIDEN))
		return SWAP_VALVE_OFF;
	if (swap_frames(s->status_before, s->status_after) == 0)
		return SWAP_FRCNT_STALLED;
	if (s->expected_word < SWAP_FW_LIST_WORDS || s->expected_word > SWAP_LPTRS_MASK)
		return SWAP_BAD_PLACEMENT;
	if ((s->ptr_after & SWAP_LPTRS_MASK) != s->expected_word)
		return SWAP_NOT_LATCHED;
	return SWAP_OURS;
}

/* Poll helper: reads taken in order after the write. Returns the index of the
 * first read whose pointer names expected_word (LPTRS_MASK applied), or -1.
 * *frames gets the FRCNT edges from status_before to that read (or to the last
 * read when none matched) so a caller can bound the wait to N frames. */
static inline int swap_poll_first(uint32_t status_before, const uint32_t *ptr,
				  const uint32_t *status, unsigned int n,
				  uint32_t expected_word, uint32_t *frames)
{
	uint32_t f = 0;

	for (unsigned int i = 0; i < n; i++) {
		f = swap_frames(status_before, status[i]);
		if (ptr[i] != SWAP_UNCLOCKED && (ptr[i] & SWAP_LPTRS_MASK) == expected_word) {
			if (frames)
				*frames = f;
			return (int)i;
		}
	}
	if (frames)
		*frames = f;
	return -1;
}

/* (b) The sequence over buffers A/B. Per step: the swap sample, and the
 * address read back from the plane entry's pointer words (PTR0 low word plus
 * its upper byte) against the buffer expected for that step. */
struct swap_step {
	struct swap_sample sample;
	uint64_t plane_addr;     /* read from the plane entry */
	uint64_t expected_addr;  /* buffer A or B for this step */
};

enum swap_seq_reason {
	SWAP_SEQ_PASS = 0,
	SWAP_SEQ_SWAP_FAILED,    /* a swap_judge reason other than ours */
	SWAP_SEQ_WRONG_BUFFER,   /* the plane entry names another buffer */
	SWAP_SEQ_EMPTY,          /* no steps */
};

struct swap_seq_verdict {
	bool pass;
	enum swap_seq_reason reason;
	int index;               /* failing step, -1 for none */
	enum swap_reason swap;   /* the swap reason of that step */
};

static inline const char *swap_seq_reason_name(enum swap_seq_reason r)
{
	switch (r) {
	case SWAP_SEQ_PASS:
		return "pass";
	case SWAP_SEQ_SWAP_FAILED:
		return "swap-failed";
	case SWAP_SEQ_WRONG_BUFFER:
		return "wrong-buffer";
	case SWAP_SEQ_EMPTY:
		return "empty";
	}
	return "?";
}

/* Every swap must advance FRCNT and name its pointer, and the plane entry
 * must hold the step's buffer address. The first failing step names it. */
static inline struct swap_seq_verdict swap_seq_judge(const struct swap_step *st,
						     unsigned int n)
{
	struct swap_seq_verdict v = { n != 0, n ? SWAP_SEQ_PASS : SWAP_SEQ_EMPTY, -1, SWAP_OURS };

	for (unsigned int i = 0; i < n; i++) {
		enum swap_reason r = swap_judge(&st[i].sample);

		if (r != SWAP_OURS) {
			v.pass = false;
			v.reason = SWAP_SEQ_SWAP_FAILED;
			v.index = (int)i;
			v.swap = r;
			return v;
		}
		if (st[i].plane_addr != st[i].expected_addr) {
			v.pass = false;
			v.reason = SWAP_SEQ_WRONG_BUFFER;
			v.index = (int)i;
			return v;
		}
	}
	return v;
}

/* (c) Restore of the firmware's list. */
enum swap_restore_reason {
	SWAP_RESTORE_EQUAL = 0,   /* list words byte-equal and pointer restored */
	SWAP_RESTORE_WORD_DIFF,   /* first differing list word */
	SWAP_RESTORE_PTR_DIFF,    /* list equal, pointer word not the saved one */
};

struct swap_restore_verdict {
	bool pass;
	enum swap_restore_reason reason;
	int index;       /* first differing word, -1 when none */
	uint32_t want;   /* saved value (word or pointer) */
	uint32_t got;    /* read back */
};

static inline const char *swap_restore_reason_name(enum swap_restore_reason r)
{
	switch (r) {
	case SWAP_RESTORE_EQUAL:
		return "byte-equal";
	case SWAP_RESTORE_WORD_DIFF:
		return "word-differs";
	case SWAP_RESTORE_PTR_DIFF:
		return "pointer-differs";
	}
	return "?";
}

static inline struct swap_restore_verdict swap_restore_judge(const uint32_t *saved,
							     const uint32_t *readback,
							     unsigned int n,
							     uint32_t saved_ptr,
							     uint32_t read_ptr)
{
	struct swap_restore_verdict v = { true, SWAP_RESTORE_EQUAL, -1, 0, 0 };

	for (unsigned int i = 0; i < n; i++)
		if (saved[i] != readback[i]) {
			v.pass = false;
			v.reason = SWAP_RESTORE_WORD_DIFF;
			v.index = (int)i;
			v.want = saved[i];
			v.got = readback[i];
			return v;
		}
	if (saved_ptr != read_ptr) {
		v.pass = false;
		v.reason = SWAP_RESTORE_PTR_DIFF;
		v.want = saved_ptr;
		v.got = read_ptr;
	}
	return v;
}

#endif /* RPI5_SCANOUT32_SWAP_CHECK_H */
