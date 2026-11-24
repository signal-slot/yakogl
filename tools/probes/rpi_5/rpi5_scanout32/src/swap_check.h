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
 *   (the firmware's list is NOT the 32 words at word 0 that
 *   hvs6_chan.h:107 YD5_HVS6_BOOTLOADER_DLIST_END names: s32dumprun read it
 *   starting at LPTRS.HEADE, 672 on the board; fw_start/fw_words below)
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
	SWAP_BAD_PLACEMENT,    /* our span overlaps the firmware list, runs past
                                * the 12-bit LPTRS field, or a length is unknown */
	SWAP_NOT_LATCHED,      /* pointer did not name ours after the wait */
};

struct swap_sample {
	uint32_t ptr_before;      /* LPTRS/DL word read before the write */
	uint32_t ptr_after;       /* the same register after the wait */
	uint32_t expected_word;   /* word offset of our list */
	uint32_t status_before;   /* DISPx_STATUS before the write */
	uint32_t status_after;    /* DISPx_STATUS after the wait */
	uint32_t pv_control;
	uint32_t pv_vcontrol;
	/* Placement. The firmware's list spans [fw_start, fw_start+fw_words):
	 * fw_start is LPTRS.HEADE read at run time (a word offset), fw_words
	 * its measured length up to the END word. Ours spans
	 * [expected_word, expected_word+our_words). Zero fw_words or zero
	 * our_words means "unknown": the judge refuses the placement, since
	 * overlap cannot be excluded. Zero-initialised = refused. */
	uint32_t fw_start;
	uint32_t fw_words;
	uint32_t our_words;
};

/* True when our span is clear of the firmware's and inside the LPTRS field. */
static inline bool swap_placement_ok(const struct swap_sample *s)
{
	uint64_t o0 = s->expected_word, o1 = o0 + s->our_words;
	uint64_t f0 = s->fw_start, f1 = f0 + s->fw_words;

	if (s->fw_words == 0 || s->our_words == 0)
		return false;
	if (o1 > (uint64_t)SWAP_LPTRS_MASK + 1u)
		return false;
	return o1 <= f0 || o0 >= f1; /* touching the end exactly is clear */
}

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
	if (!swap_placement_ok(s))
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

/* (h) Hardware-written words (swap-hwword, plan #390), as hvs6_dlist_swap.h's
 * struct hvs6s_hw_set: offsets from the start of a plane entry the HVS may
 * write back while it scans the entry (a context or status word). Each is
 * SKIP (left out of the byte-equal compare and of the hash) or EXPECT
 * (compared, a change reported and counted, never failing). The empty set
 * (n == 0, or a NULL pointer) is the old byte-equal judgement. Entries are
 * found in the reference words (the snapshot's, or the words we wrote) by
 * CTL0.NEXT from the first word; the END word and words after a NEXT 0 are
 * in no entry and never declared. Repeated from hvs6_dlist.h:
 *   hvs6_dlist.h:123     HVS6_CTL0_END (bit 31)
 *   hvs6_dlist.h:125-126 HVS6_CTL0_NEXT_SHIFT 24, HVS6_CTL0_NEXT_MASK 0x3f */
#define SWAP_CTL0_END (1u << 31)
#define SWAP_CTL0_NEXT_SHIFT 24u
#define SWAP_CTL0_NEXT_MASK 0x3fu
#define SWAP_HW_MAX 4u    /* HVS6S_HW_MAX */
#define SWAP_DIFF_MAX 8u  /* differing words kept per compare (all counted) */
#define SWAP_SLOT_MAX 10u /* HVS6W_LIST_WORDS: our list, entry and END */

enum swap_hw_mode {
	SWAP_HW_NO = 0, /* not declared: a difference fails */
	SWAP_HW_SKIP,
	SWAP_HW_EXPECT,
};

struct swap_hw_word {
	uint32_t off;
	enum swap_hw_mode mode;
};

struct swap_hw_set {
	unsigned int n;
	struct swap_hw_word w[SWAP_HW_MAX];
};

static inline const char *swap_hw_mode_name(enum swap_hw_mode m)
{
	switch (m) {
	case SWAP_HW_NO:
		return "no";
	case SWAP_HW_SKIP:
		return "skip";
	case SWAP_HW_EXPECT:
		return "expect";
	}
	return "?";
}

static inline enum swap_hw_mode swap_hw_lookup(const struct swap_hw_set *hw, uint32_t off)
{
	if (!hw)
		return SWAP_HW_NO;
	for (unsigned int i = 0; i < hw->n && i < SWAP_HW_MAX; i++)
		if (hw->w[i].off == off)
			return hw->w[i].mode;
	return SWAP_HW_NO;
}

/* Word i of ref[0..n) by CTL0.NEXT from ref[0]: true with its entry and its
 * offset in it; false for the END word, after a NEXT 0, or past n. */
static inline bool swap_entry_pos(const uint32_t *ref, unsigned int n, unsigned int i,
				  uint32_t *entry, uint32_t *off)
{
	unsigned int pos = 0;
	uint32_t e = 0;

	if (!ref || i >= n)
		return false;
	while (pos <= i) {
		uint32_t nx;

		if (ref[pos] & SWAP_CTL0_END)
			return false;
		nx = (ref[pos] >> SWAP_CTL0_NEXT_SHIFT) & SWAP_CTL0_NEXT_MASK;
		if (nx == 0)
			return false;
		if (i < pos + nx) {
			*entry = e;
			*off = i - pos;
			return true;
		}
		pos += nx;
		e++;
	}
	return false;
}

/* The mode of word i; *entry and *off -1 outside every entry. */
static inline enum swap_hw_mode swap_hw_of(const struct swap_hw_set *hw, const uint32_t *ref,
					   unsigned int n, unsigned int i, int32_t *entry,
					   int32_t *off)
{
	uint32_t e = 0, o = 0;

	*entry = -1;
	*off = -1;
	if (!swap_entry_pos(ref, n, i, &e, &o))
		return SWAP_HW_NO;
	*entry = (int32_t)e;
	*off = (int32_t)o;
	return swap_hw_lookup(hw, o);
}

/* One differing word; word is base + index, entry/off -1 outside entries. */
struct swap_diff {
	uint32_t word;
	int32_t entry, off;
	uint32_t was, now;
	enum swap_hw_mode hw;
};

struct swap_diffs {
	unsigned int total, undeclared, skip, expect;
	unsigned int hw_words; /* words at a declared offset, changed or not */
	unsigned int shown;    /* d[0..shown) */
	struct swap_diff d[SWAP_DIFF_MAX];
};

/*
 * Every word of got[0..n) against ref[0..n) (the layout and the expected
 * values), base the list word of ref[0]. Fills *d when not NULL. Returns the
 * undeclared differing words (the ones that fail); *first gets the index of
 * the first of them, -1 when none (first may be NULL).
 */
static inline unsigned int swap_diff_words(const struct swap_hw_set *hw, const uint32_t *ref,
					   const uint32_t *got, unsigned int n, uint32_t base,
					   struct swap_diffs *d, int *first)
{
	struct swap_diffs z = { 0 };
	struct swap_diffs *o = d ? d : &z;

	*o = z;
	if (first)
		*first = -1;
	for (unsigned int i = 0; i < n; i++) {
		int32_t e, f;
		enum swap_hw_mode m = swap_hw_of(hw, ref, n, i, &e, &f);

		if (m != SWAP_HW_NO)
			o->hw_words++;
		if (ref[i] == got[i])
			continue;
		o->total++;
		if (m == SWAP_HW_SKIP) {
			o->skip++;
		} else if (m == SWAP_HW_EXPECT) {
			o->expect++;
		} else {
			if (first && *first < 0)
				*first = (int)i;
			o->undeclared++;
		}
		if (o->shown < SWAP_DIFF_MAX) {
			o->d[o->shown] = (struct swap_diff){ base + i, e, f, ref[i], got[i], m };
			o->shown++;
		}
	}
	return o->undeclared;
}

/* FNV-1a over the bytes (little-endian) of w[0..n), leaving out every word
 * at a declared offset (SKIP and EXPECT both: a hash cannot say "changed as
 * expected"), entries from ref. The empty set hashes every word: the hash
 * rpi5_scanout32 printed before. */
static inline uint32_t swap_words_hash(const struct swap_hw_set *hw, const uint32_t *ref,
				       const uint32_t *w, unsigned int n)
{
	uint32_t h = 2166136261u;
	bool declared = hw && hw->n;

	for (unsigned int i = 0; i < n; i++) {
		int32_t e, f;

		if (declared && swap_hw_of(hw, ref, n, i, &e, &f) != SWAP_HW_NO)
			continue;
		for (unsigned int b = 0; b < 4u; b++)
			h = (h ^ ((w[i] >> (8u * b)) & 0xffu)) * 16777619u;
	}
	return h;
}

/* Our slot read back after a swap: the words written and the words read. */
struct swap_slot {
	uint32_t start;              /* list word of want[0] */
	unsigned int n;              /* 0: not read, the slot is not judged */
	uint32_t want[SWAP_SLOT_MAX];
	uint32_t got[SWAP_SLOT_MAX];
};

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
	SWAP_SEQ_SLOT_DIFF,      /* an undeclared word of our slot changed */
	SWAP_SEQ_NO_PICKUP,      /* DL.LACT never reached the step's head (swap_seq_judge_lact) */
};

struct swap_seq_verdict {
	bool pass;
	enum swap_seq_reason reason;
	int index;               /* failing step, -1 for none */
	enum swap_reason swap;   /* the swap reason of that step */
	int word;                /* SLOT_DIFF: the slot's word index, -1 otherwise */
	uint32_t want, got;      /* SLOT_DIFF: written and read */
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
	case SWAP_SEQ_SLOT_DIFF:
		return "slot-word-differs";
	case SWAP_SEQ_NO_PICKUP:
		return "lact-no-pickup";
	}
	return "?";
}

/* Every swap must advance FRCNT and name its pointer, our slot (when slots
 * is not NULL and slots[i].n is not 0) must read back as written except at
 * hw's declared offsets, and the plane entry must hold the step's buffer
 * address. The first failing step names it. The address is judged as
 * before, also when a declared word is one of the pointer words. */
static inline struct swap_seq_verdict swap_seq_judge_hw(const struct swap_step *st,
							const struct swap_slot *slots,
							unsigned int n,
							const struct swap_hw_set *hw)
{
	struct swap_seq_verdict v = { n != 0, n ? SWAP_SEQ_PASS : SWAP_SEQ_EMPTY, -1, SWAP_OURS,
				      -1, 0, 0 };

	for (unsigned int i = 0; i < n; i++) {
		enum swap_reason r = swap_judge(&st[i].sample);

		if (r != SWAP_OURS) {
			v.pass = false;
			v.reason = SWAP_SEQ_SWAP_FAILED;
			v.index = (int)i;
			v.swap = r;
			return v;
		}
		if (slots && slots[i].n) {
			unsigned int k = slots[i].n < SWAP_SLOT_MAX ? slots[i].n : SWAP_SLOT_MAX;
			int w = -1;

			if (swap_diff_words(hw, slots[i].want, slots[i].got, k, slots[i].start, NULL,
					    &w) != 0) {
				v.pass = false;
				v.reason = SWAP_SEQ_SLOT_DIFF;
				v.index = (int)i;
				v.word = w;
				v.want = slots[i].want[w];
				v.got = slots[i].got[w];
				return v;
			}
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

/* The old judgement: no slots, the empty set. */
static inline struct swap_seq_verdict swap_seq_judge(const struct swap_step *st,
						     unsigned int n)
{
	return swap_seq_judge_hw(st, NULL, n, NULL);
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

/*
 * The restore with hardware-written words: every word compared, entries from
 * saved[] (the snapshot), base the list word of saved[0] (HEADE). Fails on
 * the first undeclared differing word (index, want, got name it), else on the
 * pointer. A declared word that differs is recorded in *d (when not NULL)
 * and never fails. With an empty set this is swap_restore_judge.
 */
static inline struct swap_restore_verdict swap_restore_judge_hw(const uint32_t *saved,
								const uint32_t *readback,
								unsigned int n,
								uint32_t saved_ptr,
								uint32_t read_ptr,
								const struct swap_hw_set *hw,
								uint32_t base,
								struct swap_diffs *d)
{
	struct swap_restore_verdict v = { true, SWAP_RESTORE_EQUAL, -1, 0, 0 };
	int first = -1;

	if (swap_diff_words(hw, saved, readback, n, base, d, &first) != 0) {
		v.pass = false;
		v.reason = SWAP_RESTORE_WORD_DIFF;
		v.index = first;
		v.want = saved[first];
		v.got = readback[first];
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

/* (l) DL.LACT pickup (lact-probe-a, plan #390). hvs6_dlist_swap.h judges a
 * swap by DISPx_DL.LACT reaching the head just written (report.lact_pickup,
 * HVS6S_ERR_LACT_PICKUP) and a restore by DL.LACT back on the firmware's
 * LPTRS.HEADE (report.restore_lact_ok, HVS6S_ERR_RESTORE_LACT): Linux
 * completes a flip only then (rpi:vc4_crtc.c:832-855). LPTRS reading back
 * as written does not show that the HVS scanned the list. The probe's
 * verdicts use these: */

/* 'u3 named: yes': LPTRS names our slot after swap 1 and DL.LACT picked the
 * slot up. */
static inline bool swap_named(uint32_t ptr_after, uint32_t slot_word, bool lact_pickup)
{
	return ptr_after != SWAP_UNCLOCKED && (ptr_after & SWAP_LPTRS_MASK) == slot_word &&
	       lact_pickup;
}

/* The sequence with each step's pickup: swap_seq_judge_hw, and step i fails
 * with SWAP_SEQ_NO_PICKUP when pickup[i] is false (after the swap reason,
 * before the slot and the address). pickup NULL is swap_seq_judge_hw. */
static inline struct swap_seq_verdict swap_seq_judge_lact(const struct swap_step *st,
							  const struct swap_slot *slots,
							  const bool *pickup, unsigned int n,
							  const struct swap_hw_set *hw)
{
	struct swap_seq_verdict v = { n != 0, n ? SWAP_SEQ_PASS : SWAP_SEQ_EMPTY, -1, SWAP_OURS,
				      -1, 0, 0 };

	if (!pickup)
		return swap_seq_judge_hw(st, slots, n, hw);
	for (unsigned int i = 0; i < n; i++) {
		struct swap_seq_verdict one;
		enum swap_reason r = swap_judge(&st[i].sample);

		if (r == SWAP_OURS && !pickup[i]) {
			v.pass = false;
			v.reason = SWAP_SEQ_NO_PICKUP;
			v.index = (int)i;
			return v;
		}
		one = swap_seq_judge_hw(&st[i], slots ? &slots[i] : NULL, 1, hw);
		if (!one.pass) {
			one.index = (int)i;
			return one;
		}
	}
	return v;
}

/* True when every one of the n swaps made picked up (n 0: true). */
static inline bool swap_pickups_all(const bool *pickup, unsigned int n)
{
	for (unsigned int i = 0; i < n; i++)
		if (!pickup || !pickup[i])
			return false;
	return true;
}

/* The restore's DL.LACT result that S10 goes by: the final hvs6s_restore's
 * after swaps 2.. (wrote_more), else swap 1's own restore inside hvs6s_run
 * (ran: only swap 1 ran), else true (nothing was written, nothing to come
 * back from). */
static inline bool swap_restore_lact(bool wrote_more, bool ran, bool final_lact_ok,
				     bool run_lact_ok)
{
	if (wrote_more)
		return final_lact_ok;
	if (ran)
		return run_lact_ok;
	return true;
}

/* Our list words go back to zero only once channel 0 is back on the
 * firmware's list: the restore's frames passed (settled), DL.LACT came back
 * to the firmware's head (lact_ok, a positive check), LPTRS.HEADE is the
 * firmware's head, and neither LPTRS.HEADE nor DL.LACT is inside our span
 * [pair, pair + words). */
static inline bool swap_clear_ok(bool settled, bool lact_ok, uint32_t lptrs, uint32_t lact,
				 uint32_t fw_head, uint32_t pair, uint32_t words)
{
	uint32_t head = lptrs & SWAP_LPTRS_MASK;
	uint64_t p0 = pair, p1 = p0 + words;

	if (!settled || !lact_ok || lptrs == SWAP_UNCLOCKED || head != fw_head)
		return false;
	if (head >= p0 && head < p1)
		return false;
	if (lact >= p0 && lact < p1)
		return false;
	return true;
}

/* 'u3 restore: PASS': the restore call passed (restore_ok), the firmware's
 * words and pointer judged equal (words_pass), our words cleared, DL.LACT
 * back on the firmware's head (lact_ok, swap_restore_lact), and every swap
 * made picked up our list (pickups_ok, swap_pickups_all): a restore from a
 * list the HVS never scanned shows nothing about giving the list back. */
static inline bool swap_restore_pass(bool restore_ok, bool words_pass, bool cleared, bool lact_ok,
				     bool pickups_ok)
{
	return restore_ok && words_pass && cleared && lact_ok && pickups_ok;
}

/* (v) Vblank landing (swap-lact-probe, on swap-vblank-lib). hvs6s_swap writes
 * LPTRS just after an FRCNT step found by reads (cfg.write_at
 * HVS6S_AT_VBLANK) and times the DL.LACT pickup with a tight loop. A swap
 * landed in vblank when
 *   - the vblank wait was made and found its FRCNT step (else NO_VBLANK:
 *     HVS6S_ERR_VBLANK, LPTRS not written; NOT_TRIED for a swap made
 *     without the wait),
 *   - the write was in the vblank (report.write_in_vblank: right after the
 *     step and, with a clock, within the vblank window; else NOT_IN_VBLANK),
 *   - DL.LACT reached the new head inside the tight loop (tight_pickup; else
 *     PICKUP_TIMEOUT: the loop ran out, whatever the slower poll found later),
 *   - FRCNT stepped at most SWAP_VB_MAX_STEPS times from the write to the
 *     pickup (6-bit, wrap aware) and, with a clock, the pickup came within
 *     SWAP_VB_DT_US_MAX of the write (else MISSED_FRAME; the time bound also
 *     catches a 6-bit FRCNT difference that aliased over 64 frames).
 * The tight loop's frame-boundary flag (report.lact_boundary) and YLINE are
 * printed as a record and decide nothing: the boundary depends on the
 * assumed 667 us vblank and on where in the frame LACT changes, which the
 * board has not shown yet; YLINE's range and direction are unproven. */
#define SWAP_VB_MAX_STEPS 1u
/* Two frames at 50 Hz (and so more than two at the firmware's 60 Hz): a
 * write just after a step and a pickup at the next one stay well inside. */
#define SWAP_VB_DT_US_MAX 40000u

enum swap_vb_reason {
	SWAP_VB_LANDED = 0,      /* in vblank, picked up within one frame step */
	SWAP_VB_NOT_TRIED,       /* no vblank wait was made (HVS6S_AT_ONCE) */
	SWAP_VB_NO_VBLANK,       /* no FRCNT step within the read bound; LPTRS not written */
	SWAP_VB_NOT_IN_VBLANK,   /* the write was not right after the step */
	SWAP_VB_PICKUP_TIMEOUT,  /* the tight loop ran out before DL.LACT reached the head */
	SWAP_VB_MISSED_FRAME,    /* more than SWAP_VB_MAX_STEPS frames to the pickup */
};

/* One swap's vblank record, copied from hvs6s_swap's report. */
struct swap_vblank {
	bool tried;            /* report.vb_tried */
	bool found;            /* report.vb_found */
	bool swapped;          /* report.swapped: LPTRS was written */
	bool in_vblank;        /* report.write_in_vblank */
	bool tight_pickup;     /* report.tight_pickup */
	bool boundary;         /* report.lact_boundary (record only) */
	bool timed;            /* report.timed: the _ns fields are set */
	uint32_t frcnt_write;  /* report.write_frcnt */
	uint32_t frcnt_pickup; /* report.pickup_frcnt (at the pickup, or the last iteration) */
	uint32_t pickup_reads; /* report.tight_reads (iterations, the pickup's index + 1) */
	uint64_t dt_ns;        /* report.lact_dt_ns, write to pickup */
	uint32_t read_ns;      /* report.read_ns, one register read in the tight loop */
};

static inline const char *swap_vb_reason_name(enum swap_vb_reason r)
{
	switch (r) {
	case SWAP_VB_LANDED:
		return "landed";
	case SWAP_VB_NOT_TRIED:
		return "not-tried";
	case SWAP_VB_NO_VBLANK:
		return "no-vblank";
	case SWAP_VB_NOT_IN_VBLANK:
		return "write-not-in-vblank";
	case SWAP_VB_PICKUP_TIMEOUT:
		return "pickup-timeout";
	case SWAP_VB_MISSED_FRAME:
		return "missed-frame";
	}
	return "?";
}

/* FRCNT steps from the write to the pickup, 6-bit. */
static inline uint32_t swap_vb_steps(const struct swap_vblank *v)
{
	return (v->frcnt_pickup - v->frcnt_write) & SWAP_FRCNT_MASK;
}

/* In the order of the header: tried, found, in the vblank, picked up in the
 * tight loop, within one frame step. */
static inline enum swap_vb_reason swap_vblank_judge(const struct swap_vblank *v)
{
	if (!v || !v->tried)
		return SWAP_VB_NOT_TRIED;
	if (!v->found || !v->swapped)
		return SWAP_VB_NO_VBLANK;
	if (!v->in_vblank)
		return SWAP_VB_NOT_IN_VBLANK;
	if (!v->tight_pickup)
		return SWAP_VB_PICKUP_TIMEOUT;
	if (swap_vb_steps(v) > SWAP_VB_MAX_STEPS ||
	    (v->timed && v->dt_ns > (uint64_t)SWAP_VB_DT_US_MAX * 1000u))
		return SWAP_VB_MISSED_FRAME;
	return SWAP_VB_LANDED;
}

/* The first of the n swaps made that did not land (its index), -1 when all
 * landed (n 0: -1); *why its reason (why may be NULL). */
static inline int swap_vblank_first_miss(const struct swap_vblank *v, unsigned int n,
					 enum swap_vb_reason *why)
{
	for (unsigned int i = 0; i < n; i++) {
		enum swap_vb_reason r = swap_vblank_judge(v ? &v[i] : NULL);

		if (r != SWAP_VB_LANDED) {
			if (why)
				*why = r;
			return (int)i;
		}
	}
	if (why)
		*why = SWAP_VB_LANDED;
	return -1;
}

#endif /* RPI5_SCANOUT32_SWAP_CHECK_H */
