/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host test of ../src/swap_check.h. Prints "swap_check_test: N checks, F failed". */
#include <stdio.h>
#include <string.h>

#include "../src/swap_check.h"

static int checks, failed;
#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

static uint32_t st(uint32_t frcnt)
{
	return (frcnt & 0x3fu) << 16 | 0x4000u;
}

static struct swap_sample good(void)
{
	struct swap_sample s = { 0x0, 0x40, 0x40, st(10), st(11), 1, 1, 672, 10, 10 };

	return s;
}

static void test_judge(void)
{
	struct swap_sample s = good();

	CHECK(swap_judge(&s) == SWAP_OURS);
	s = good(); s.ptr_after = 0xf0000040u; /* bits above the word index ignored */
	CHECK(swap_judge(&s) == SWAP_OURS);
	s = good(); s.ptr_after = 0x0; /* never followed */
	CHECK(swap_judge(&s) == SWAP_NOT_LATCHED);
	s = good(); s.ptr_after = 0x41;
	CHECK(swap_judge(&s) == SWAP_NOT_LATCHED);
	s = good(); s.status_after = st(10);
	CHECK(swap_judge(&s) == SWAP_FRCNT_STALLED);
	s = good(); s.status_before = st(63); s.status_after = st(0); /* wrap */
	CHECK(swap_judge(&s) == SWAP_OURS);
	CHECK(swap_frames(st(63), st(0)) == 1);
	CHECK(swap_frames(st(5), st(5)) == 0);
	CHECK(swap_frames(st(62), st(1)) == 3);
	s = good(); s.status_before = st(63); s.status_after = st(63);
	CHECK(swap_judge(&s) == SWAP_FRCNT_STALLED);
	s = good(); s.pv_control = 0;
	CHECK(swap_judge(&s) == SWAP_VALVE_OFF);
	s = good(); s.pv_vcontrol = 0;
	CHECK(swap_judge(&s) == SWAP_VALVE_OFF);
	/* firmware at 672 len 10: ours at 32 len 10 is clear */
	s = good(); s.expected_word = 32; s.ptr_after = 32;
	CHECK(swap_judge(&s) == SWAP_OURS);
	/* word 0 is no longer special */
	s = good(); s.expected_word = 0; s.ptr_after = 0;
	CHECK(swap_judge(&s) == SWAP_OURS);
	/* overlap: start inside, end inside, covering */
	s = good(); s.expected_word = 675; s.ptr_after = 675;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.expected_word = 665; s.ptr_after = 665;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.expected_word = 600; s.our_words = 200; s.ptr_after = 600;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	/* touching exactly: ours ends at 672, or starts at 682 */
	s = good(); s.expected_word = 662; s.ptr_after = 662;
	CHECK(swap_judge(&s) == SWAP_OURS);
	s = good(); s.expected_word = 682; s.ptr_after = 682;
	CHECK(swap_judge(&s) == SWAP_OURS);
	s = good(); s.expected_word = 681; s.ptr_after = 681;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	/* the LPTRS field: ending at 4096 is fine, past it is not */
	s = good(); s.expected_word = 4086; s.ptr_after = 4086;
	CHECK(swap_judge(&s) == SWAP_OURS);
	s = good(); s.expected_word = 4087; s.ptr_after = 4087;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.expected_word = 0x1000; s.ptr_after = 0;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.expected_word = 0xfffffff0u; s.ptr_after = 0;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	/* unknown lengths are refused */
	s = good(); s.fw_words = 0;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.our_words = 0;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	/* placement is judged after FRCNT, before the pointer */
	s = good(); s.fw_words = 0; s.status_after = st(10);
	CHECK(swap_judge(&s) == SWAP_FRCNT_STALLED);
	s = good(); s.fw_words = 0; s.ptr_after = 0;
	CHECK(swap_judge(&s) == SWAP_BAD_PLACEMENT);
	s = good(); s.status_after = 0xffffffffu;
	CHECK(swap_judge(&s) == SWAP_UNCLOCKED_FAIL);
	s = good(); s.ptr_after = 0xffffffffu;
	CHECK(swap_judge(&s) == SWAP_UNCLOCKED_FAIL);
	s = good(); s.pv_control = 0xffffffffu;
	CHECK(swap_judge(&s) == SWAP_UNCLOCKED_FAIL);
	/* precedence: valve off beats a stalled FRCNT */
	s = good(); s.pv_control = 0; s.status_after = st(10);
	CHECK(swap_judge(&s) == SWAP_VALVE_OFF);
	CHECK(swap_ptr_word(40, true) == 45);
	CHECK(swap_ptr_word(40, false) == 46);
}

static void test_poll(void)
{
	uint32_t f = 99;
	/* pointer follows only at the next FRCNT edge: pass */
	uint32_t ptr1[3] = { 0x0, 0x0, 0x40 };
	uint32_t sta1[3] = { st(10), st(10), st(11) };
	int i = swap_poll_first(st(10), ptr1, sta1, 3, 0x40, &f);

	CHECK(i == 2 && f == 1);
	/* never follows: -1, frames = those elapsed */
	uint32_t ptr2[3] = { 0, 0, 0 };
	uint32_t sta2[3] = { st(10), st(11), st(12) };

	i = swap_poll_first(st(10), ptr2, sta2, 3, 0x40, &f);
	CHECK(i == -1 && f == 2);
	/* wrap while polling */
	uint32_t ptr3[2] = { 0, 0x40 };
	uint32_t sta3[2] = { st(63), st(0) };

	i = swap_poll_first(st(63), ptr3, sta3, 2, 0x40, &f);
	CHECK(i == 1 && f == 1);
	CHECK(swap_poll_first(st(1), ptr3, sta3, 0, 0x40, NULL) == -1);
	/* all-ones is not a hit even though it masks to 0xfff */
	uint32_t ptr4[1] = { 0xffffffffu };
	uint32_t sta4[1] = { st(1) };

	CHECK(swap_poll_first(st(0), ptr4, sta4, 1, 0xfff, &f) == -1);
}

static void test_seq(void)
{
	struct swap_step s[4];
	struct swap_seq_verdict v;

	for (int i = 0; i < 4; i++) {
		s[i].sample = good();
		s[i].sample.status_before = st(62 + i); /* 62,63,0,1 */
		s[i].sample.status_after = st(63 + i);  /* wraps 63 -> 0 */
		s[i].expected_addr = (i & 1) ? 0x30000000ull : 0x10000000ull;
		s[i].plane_addr = s[i].expected_addr;
	}
	v = swap_seq_judge(s, 4);
	CHECK(v.pass && v.reason == SWAP_SEQ_PASS && v.index == -1);
	s[2].plane_addr = 0x30000000ull; /* A expected, B named */
	v = swap_seq_judge(s, 4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_WRONG_BUFFER && v.index == 2);
	s[2].plane_addr = s[2].expected_addr;
	s[1].plane_addr = (1ull << 32) | 0x30000000ull; /* upper byte differs */
	v = swap_seq_judge(s, 4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_WRONG_BUFFER && v.index == 1);
	s[1].plane_addr = s[1].expected_addr;
	s[3].sample.status_after = s[3].sample.status_before;
	v = swap_seq_judge(s, 4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_SWAP_FAILED && v.index == 3 &&
	      v.swap == SWAP_FRCNT_STALLED);
	s[0].sample.ptr_after = 0;
	v = swap_seq_judge(s, 4);
	CHECK(v.index == 0 && v.swap == SWAP_NOT_LATCHED);
	v = swap_seq_judge(s, 0);
	CHECK(!v.pass && v.reason == SWAP_SEQ_EMPTY);
}

static void test_restore(void)
{
	uint32_t a[32], b[32];
	struct swap_restore_verdict v;

	for (unsigned int i = 0; i < 32; i++)
		a[i] = b[i] = 0xc0de0000u + i;
	v = swap_restore_judge(a, b, 32, 0x0, 0x0);
	CHECK(v.pass && v.reason == SWAP_RESTORE_EQUAL && v.index == -1);
	b[0] ^= 1;
	v = swap_restore_judge(a, b, 32, 0, 0);
	CHECK(!v.pass && v.reason == SWAP_RESTORE_WORD_DIFF && v.index == 0 &&
	      v.want == a[0] && v.got == b[0]);
	b[0] = a[0];
	b[31] = 0;
	v = swap_restore_judge(a, b, 32, 0, 0);
	CHECK(!v.pass && v.index == 31 && v.want == a[31] && v.got == 0);
	b[31] = a[31];
	/* only the pointer differs */
	v = swap_restore_judge(a, b, 32, 0x0, 0x40);
	CHECK(!v.pass && v.reason == SWAP_RESTORE_PTR_DIFF && v.index == -1 &&
	      v.want == 0 && v.got == 0x40);
	/* the first of several differences wins, words before the pointer */
	b[5] = 1; b[9] = 2;
	v = swap_restore_judge(a, b, 32, 0, 0x40);
	CHECK(v.reason == SWAP_RESTORE_WORD_DIFF && v.index == 5);
	v = swap_restore_judge(a, b, 0, 0, 0);
	CHECK(v.pass);
}

/* ---- hardware-written words -------------------------------------------- */

#define CTL0(next) (0x40000000u | ((uint32_t)(next) << 24))

/* Two firmware entries of 10 words each (CTL0 NEXT=10) and the END: 21 words. */
static void fw_list(uint32_t *w)
{
	for (unsigned int i = 0; i < 21; i++)
		w[i] = 0xc0de0000u + i;
	w[0] = CTL0(10);
	w[10] = CTL0(10);
	w[20] = SWAP_CTL0_END;
}

static uint32_t fnv(const uint32_t *w, unsigned int n)
{
	uint32_t h = 2166136261u;

	for (unsigned int i = 0; i < n; i++)
		for (unsigned int b = 0; b < 4u; b++)
			h = (h ^ ((w[i] >> (8u * b)) & 0xffu)) * 16777619u;
	return h;
}

static void test_hw_pos(void)
{
	uint32_t w[21], e = 99, o = 99;
	int32_t se, so;
	struct swap_hw_set hw = { 1, { { 4, SWAP_HW_SKIP } } };

	fw_list(w);
	CHECK(swap_entry_pos(w, 21, 0, &e, &o) && e == 0 && o == 0);
	CHECK(swap_entry_pos(w, 21, 14, &e, &o) && e == 1 && o == 4);
	CHECK(!swap_entry_pos(w, 21, 20, &e, &o)); /* the END word is in no entry */
	CHECK(!swap_entry_pos(w, 21, 21, &e, &o)); /* past n */
	CHECK(swap_hw_of(&hw, w, 21, 4, &se, &so) == SWAP_HW_SKIP && se == 0 && so == 4);
	CHECK(swap_hw_of(&hw, w, 21, 14, &se, &so) == SWAP_HW_SKIP && se == 1 && so == 4);
	CHECK(swap_hw_of(&hw, w, 21, 15, &se, &so) == SWAP_HW_NO && so == 5);
	CHECK(swap_hw_of(&hw, w, 21, 20, &se, &so) == SWAP_HW_NO && se == -1 && so == -1);
	CHECK(swap_hw_of(NULL, w, 21, 4, &se, &so) == SWAP_HW_NO && so == 4);
	w[10] = 0; /* NEXT 0: the second entry and what follows are in no entry */
	CHECK(!swap_entry_pos(w, 21, 14, &e, &o) && swap_entry_pos(w, 21, 9, &e, &o) && o == 9);
	/* the board's layout: one entry of NEXT=32 at HEADE, a window without END */
	{
		uint32_t b[256];

		for (unsigned int i = 0; i < 256; i++)
			b[i] = (i % 32u) ? 0x1000u + i : CTL0(32);
		CHECK(swap_entry_pos(b, 256, 255, &e, &o) && e == 7 && o == 31);
		CHECK(swap_entry_pos(b, 256, 32 * 3 + 4, &e, &o) && e == 3 && o == 4);
	}
}

static void test_hw_restore(void)
{
	uint32_t a[21], b[21];
	struct swap_restore_verdict v, old;
	struct swap_diffs d;
	struct swap_hw_set none = { 0, { { 0, SWAP_HW_NO } } };
	struct swap_hw_set skip4 = { 1, { { 4, SWAP_HW_SKIP } } };
	struct swap_hw_set exp4 = { 1, { { 4, SWAP_HW_EXPECT } } };
	struct swap_hw_set end0 = { 1, { { 0, SWAP_HW_SKIP } } };

	/* empty set: the old verdicts, case by case */
	fw_list(a);
	memcpy(b, a, sizeof(a));
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &none, 672, &d);
	CHECK(v.pass && v.reason == SWAP_RESTORE_EQUAL && d.total == 0 && d.hw_words == 0);
	CHECK(swap_words_hash(&none, a, b, 21) == fnv(b, 21));
	CHECK(swap_words_hash(NULL, a, b, 21) == fnv(b, 21));
	b[14] ^= 0x100u;
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &none, 672, &d);
	old = swap_restore_judge(a, b, 21, 0x2a0, 0x2a0);
	CHECK(!v.pass && v.reason == old.reason && v.index == old.index && v.index == 14 &&
	      v.want == old.want && v.got == old.got);
	CHECK(d.total == 1 && d.undeclared == 1 && d.shown == 1 && d.d[0].word == 686 &&
	      d.d[0].entry == 1 && d.d[0].off == 4 && d.d[0].was == a[14] && d.d[0].now == b[14] &&
	      d.d[0].hw == SWAP_HW_NO);
	v = swap_restore_judge_hw(a, a, 21, 0x2a0, 0x20, NULL, 672, NULL);
	old = swap_restore_judge(a, a, 21, 0x2a0, 0x20);
	CHECK(!v.pass && v.reason == SWAP_RESTORE_PTR_DIFF && old.reason == v.reason &&
	      v.want == 0x2a0 && v.got == 0x20);

	/* a skipped word changing: PASS, named, out of the hash */
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &skip4, 672, &d);
	CHECK(v.pass && v.reason == SWAP_RESTORE_EQUAL && v.index == -1);
	CHECK(d.total == 1 && d.skip == 1 && d.undeclared == 0 && d.hw_words == 2 &&
	      d.d[0].word == 686 && d.d[0].off == 4 && d.d[0].hw == SWAP_HW_SKIP);
	CHECK(swap_words_hash(&skip4, a, b, 21) == swap_words_hash(&skip4, a, a, 21));
	CHECK(swap_words_hash(&none, a, b, 21) != swap_words_hash(&none, a, a, 21));
	CHECK(swap_words_hash(&skip4, a, a, 21) != fnv(a, 21));
	/* a skipped word with a bad pointer still fails on the pointer */
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x20, &skip4, 672, &d);
	CHECK(!v.pass && v.reason == SWAP_RESTORE_PTR_DIFF);

	/* EXPECT: compared and counted, never failing */
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &exp4, 672, &d);
	CHECK(v.pass && d.expect == 1 && d.skip == 0 && d.d[0].hw == SWAP_HW_EXPECT);
	CHECK(swap_words_hash(&exp4, a, b, 21) == swap_words_hash(&exp4, a, a, 21));

	/* an undeclared word changing beside a skipped one: FAIL, named */
	b[15] ^= 0x1u;
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &skip4, 672, &d);
	CHECK(!v.pass && v.reason == SWAP_RESTORE_WORD_DIFF && v.index == 15 && v.want == a[15] &&
	      v.got == b[15]);
	CHECK(d.total == 2 && d.skip == 1 && d.undeclared == 1 && d.shown == 2 &&
	      d.d[0].word == 686 && d.d[1].word == 687 && d.d[1].off == 5 &&
	      d.d[1].hw == SWAP_HW_NO);
	/* the END word is in no entry: off 0 declared does not cover it */
	memcpy(b, a, sizeof(a));
	b[20] = 0;
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &end0, 672, &d);
	CHECK(!v.pass && v.index == 20 && d.d[0].entry == -1 && d.d[0].off == -1);
	/* the cap: 12 words differ, 8 kept, all counted */
	memcpy(b, a, sizeof(a));
	for (unsigned int i = 1; i <= 12; i++)
		b[i] ^= 0x10000u;
	v = swap_restore_judge_hw(a, b, 21, 0x2a0, 0x2a0, &skip4, 672, &d);
	CHECK(!v.pass && v.index == 1 && d.total == 12 && d.shown == SWAP_DIFF_MAX &&
	      d.skip == 1 && d.undeclared == 11);
}

static void test_hw_seq(void)
{
	struct swap_step s[2];
	struct swap_slot sl[2];
	struct swap_seq_verdict v;
	struct swap_hw_set none = { 0, { { 0, SWAP_HW_NO } } };
	struct swap_hw_set skip4 = { 1, { { 4, SWAP_HW_SKIP } } };

	memset(s, 0, sizeof(s));
	memset(sl, 0, sizeof(sl));
	for (int i = 0; i < 2; i++) {
		s[i].sample = good();
		s[i].expected_addr = s[i].plane_addr = 0x10000000ull + (uint64_t)i;
		sl[i].start = 32 + 10u * (unsigned int)i;
		sl[i].n = SWAP_SLOT_MAX;
		for (unsigned int k = 0; k < SWAP_SLOT_MAX; k++)
			sl[i].want[k] = sl[i].got[k] = 0x5a000000u + k;
		sl[i].want[0] = sl[i].got[0] = CTL0(9);
		sl[i].want[8] = sl[i].got[8] = SWAP_CTL0_END;
		sl[i].want[9] = sl[i].got[9] = SWAP_CTL0_END;
	}
	v = swap_seq_judge_hw(s, sl, 2, &none);
	CHECK(v.pass && v.reason == SWAP_SEQ_PASS && v.word == -1);
	/* the HVS wrote word 4 of our entry (the POS3 context word) back */
	sl[1].got[4] = 0x12345678u;
	v = swap_seq_judge_hw(s, sl, 2, &none);
	CHECK(!v.pass && v.reason == SWAP_SEQ_SLOT_DIFF && v.index == 1 && v.word == 4 &&
	      v.want == sl[1].want[4] && v.got == 0x12345678u);
	v = swap_seq_judge_hw(s, sl, 2, &skip4);
	CHECK(v.pass && v.reason == SWAP_SEQ_PASS);
	/* an undeclared slot word beside it: named */
	sl[1].got[6] ^= 1u;
	v = swap_seq_judge_hw(s, sl, 2, &skip4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_SLOT_DIFF && v.index == 1 && v.word == 6);
	/* the old judge, and a slot not read (n 0), ignore the slot words */
	CHECK(swap_seq_judge(s, 2).pass);
	sl[1].n = 0;
	CHECK(swap_seq_judge_hw(s, sl, 2, &none).pass);
	CHECK(swap_seq_judge_hw(s, NULL, 2, NULL).pass);
	/* the swap judge still comes first */
	sl[1].n = SWAP_SLOT_MAX;
	s[1].sample.status_after = s[1].sample.status_before;
	v = swap_seq_judge_hw(s, sl, 2, &skip4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_SWAP_FAILED && v.swap == SWAP_FRCNT_STALLED);
	/* a wrong buffer with a clean slot is still WRONG_BUFFER */
	s[1].sample = good();
	sl[1].got[6] = sl[1].want[6];
	s[1].plane_addr = 0x99;
	v = swap_seq_judge_hw(s, sl, 2, &skip4);
	CHECK(!v.pass && v.reason == SWAP_SEQ_WRONG_BUFFER && v.index == 1);
}

static void test_names(void)
{
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_SLOT_DIFF), "slot-word-differs"));
	CHECK(!strcmp(swap_hw_mode_name(SWAP_HW_NO), "no"));
	CHECK(!strcmp(swap_hw_mode_name(SWAP_HW_SKIP), "skip"));
	CHECK(!strcmp(swap_hw_mode_name(SWAP_HW_EXPECT), "expect"));
	CHECK(!strcmp(swap_reason_name(SWAP_OURS), "pointer-named-ours"));
	CHECK(!strcmp(swap_reason_name(SWAP_NOT_LATCHED), "pointer-not-latched"));
	CHECK(!strcmp(swap_reason_name(SWAP_FRCNT_STALLED), "frcnt-stalled"));
	CHECK(!strcmp(swap_reason_name(SWAP_VALVE_OFF), "valve-off"));
	CHECK(!strcmp(swap_reason_name(SWAP_UNCLOCKED_FAIL), "unclocked"));
	CHECK(!strcmp(swap_reason_name(SWAP_BAD_PLACEMENT), "bad-placement"));
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_PASS), "pass"));
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_SWAP_FAILED), "swap-failed"));
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_WRONG_BUFFER), "wrong-buffer"));
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_EMPTY), "empty"));
	CHECK(!strcmp(swap_restore_reason_name(SWAP_RESTORE_EQUAL), "byte-equal"));
	CHECK(!strcmp(swap_restore_reason_name(SWAP_RESTORE_WORD_DIFF), "word-differs"));
	CHECK(!strcmp(swap_restore_reason_name(SWAP_RESTORE_PTR_DIFF), "pointer-differs"));
	CHECK(!strcmp(swap_reason_name((enum swap_reason)99), "?"));
	CHECK(!strcmp(swap_seq_reason_name(SWAP_SEQ_NO_PICKUP), "lact-no-pickup"));
}

/* (l) DL.LACT pickup (lact-probe-a): named, the sequence, the restore. */
static void lact_steps(struct swap_step *s, unsigned int n)
{
	for (unsigned int i = 0; i < n; i++) {
		s[i].sample = good();
		s[i].sample.status_before = st(10 + i);
		s[i].sample.status_after = st(11 + i);
		s[i].expected_addr = (i & 1) ? 0x30000000ull : 0x10000000ull;
		s[i].plane_addr = s[i].expected_addr;
	}
}

static void test_lact_named(void)
{
	/* pickup yes: LPTRS names slot 32 and DL.LACT reached it */
	CHECK(swap_named(0x20, 32, true));
	CHECK(swap_named(0xf0000020u, 32, true)); /* bits above HEADE ignored */
	/* pickup no: LPTRS reads back as written, the HVS never took it */
	CHECK(!swap_named(0x20, 32, false));
	/* LPTRS not ours, or unclocked: no, whatever LACT says */
	CHECK(!swap_named(0x2a0, 32, true));
	CHECK(!swap_named(SWAP_UNCLOCKED, 0xfff, true));
}

static void test_lact_seq(void)
{
	struct swap_step s[4];
	bool yes[4] = { true, true, true, true };
	bool no2[4] = { true, true, false, true };
	bool no0[4] = { false, true, true, true };
	struct swap_seq_verdict v;

	lact_steps(s, 4);
	/* pickup yes on every swap: pass, as the old judge */
	v = swap_seq_judge_lact(s, NULL, yes, 4, NULL);
	CHECK(v.pass && v.reason == SWAP_SEQ_PASS && v.index == -1);
	/* NULL pickup: the old judge */
	v = swap_seq_judge_lact(s, NULL, NULL, 4, NULL);
	CHECK(v.pass && v.reason == SWAP_SEQ_PASS);
	/* pickup no at swap 3: fails there, although LPTRS named ours */
	v = swap_seq_judge_lact(s, NULL, no2, 4, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_NO_PICKUP && v.index == 2 && v.swap == SWAP_OURS);
	CHECK(swap_seq_judge_hw(s, NULL, 4, NULL).pass); /* the old judge did not see it */
	/* pickup no at swap 1 */
	v = swap_seq_judge_lact(s, NULL, no0, 4, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_NO_PICKUP && v.index == 0);
	/* swap-1-only: one step, picked up -> pass; not -> no pickup */
	v = swap_seq_judge_lact(s, NULL, yes, 1, NULL);
	CHECK(v.pass && v.index == -1);
	v = swap_seq_judge_lact(s, NULL, no0, 1, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_NO_PICKUP && v.index == 0);
	/* the swap's own reason comes first: stalled at swap 3 with no pickup */
	s[2].sample.status_after = s[2].sample.status_before;
	v = swap_seq_judge_lact(s, NULL, no2, 4, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_SWAP_FAILED && v.index == 2 &&
	      v.swap == SWAP_FRCNT_STALLED);
	lact_steps(s, 4);
	/* picked up, but the wrong buffer at swap 4: index is 3 */
	s[3].plane_addr = 0x10000000ull;
	v = swap_seq_judge_lact(s, NULL, yes, 4, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_WRONG_BUFFER && v.index == 3);
	v = swap_seq_judge_lact(s, NULL, yes, 0, NULL);
	CHECK(!v.pass && v.reason == SWAP_SEQ_EMPTY);
	CHECK(swap_pickups_all(yes, 4));
	CHECK(!swap_pickups_all(no2, 4));
	CHECK(swap_pickups_all(no2, 2)); /* only the swaps made count */
	CHECK(swap_pickups_all(NULL, 0));
	CHECK(!swap_pickups_all(NULL, 1));
}

static void test_lact_restore(void)
{
	/* which restore's DL.LACT decides: the final one after swaps 2.. */
	CHECK(swap_restore_lact(true, true, true, false));
	CHECK(!swap_restore_lact(true, true, false, true));
	/* swap-1-only path: hvs6s_run's own restore */
	CHECK(swap_restore_lact(false, true, false, true));
	CHECK(!swap_restore_lact(false, true, true, false));
	/* nothing written: nothing to come back from */
	CHECK(swap_restore_lact(false, false, false, false));

	/* u3 restore: PASS needs all five */
	CHECK(swap_restore_pass(true, true, true, true, true));
	CHECK(!swap_restore_pass(true, true, true, true, false)); /* pickup no */
	CHECK(!swap_restore_pass(true, true, true, false, true)); /* restore-lact failed */
	CHECK(!swap_restore_pass(false, true, true, true, true));
	CHECK(!swap_restore_pass(true, false, true, true, true));
	CHECK(!swap_restore_pass(true, true, false, true, true));
	/* swap-1-only, its restore's LACT not back: not PASS although the words
	 * and pointer are equal */
	CHECK(!swap_restore_pass(true, true, true, swap_restore_lact(false, true, false, false),
				 true));
	CHECK(swap_restore_pass(true, true, true, swap_restore_lact(false, true, false, true),
				true));

	/* our words (32..51) cleared only once DL.LACT is back on 672 */
	CHECK(swap_clear_ok(true, true, 0x2a0, 672, 672, 32, 20));
	CHECK(!swap_clear_ok(true, false, 0x2a0, 672, 672, 32, 20)); /* restore-lact failed */
	CHECK(!swap_clear_ok(false, true, 0x2a0, 672, 672, 32, 20)); /* frames */
	CHECK(!swap_clear_ok(true, true, 0x20, 672, 672, 32, 20));   /* LPTRS on ours */
	CHECK(!swap_clear_ok(true, true, 0x2a0, 40, 672, 32, 20));   /* LACT inside ours */
	CHECK(!swap_clear_ok(true, true, 0x2a0, 51, 672, 32, 20));
	CHECK(swap_clear_ok(true, true, 0x2a0, 52, 672, 32, 20));    /* just past ours */
	CHECK(!swap_clear_ok(true, true, 0x2a4, 672, 672, 32, 20));  /* not the fw head */
	CHECK(!swap_clear_ok(true, true, SWAP_UNCLOCKED, 672, 0xfff, 32, 20));
}

/* (v) Vblank landing (swap-lact-probe). */
static struct swap_vblank vb_in(void)
{
	/* written right after the step at FRCNT 10, picked up at the next step,
	 * 9 ms later, 1200 iterations of the tight loop at 120 ns a read */
	struct swap_vblank v = {
		.tried = true, .found = true, .swapped = true, .in_vblank = true,
		.tight_pickup = true, .boundary = true, .timed = true,
		.frcnt_write = 10, .frcnt_pickup = 11, .pickup_reads = 1200,
		.dt_ns = 9000000u, .read_ns = 120,
	};

	return v;
}

static void test_vblank(void)
{
	struct swap_vblank v = vb_in(), seq[4];
	enum swap_vb_reason why = SWAP_VB_NOT_TRIED;

	/* in vblank: picked up at the next FRCNT step */
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED && swap_vb_steps(&v) == 1);
	/* picked up before any step (LACT followed at once), no clock */
	v = vb_in(); v.frcnt_pickup = 10; v.timed = false; v.dt_ns = 0;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED && swap_vb_steps(&v) == 0);
	/* across the 6-bit wrap: 63 -> 0 is one step */
	v = vb_in(); v.frcnt_write = 63; v.frcnt_pickup = 0;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED);
	/* the boundary flag is a record only */
	v = vb_in(); v.boundary = false;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED);

	/* a missed frame: two steps to the pickup, also across the wrap */
	v = vb_in(); v.frcnt_pickup = 12;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_MISSED_FRAME && swap_vb_steps(&v) == 2);
	v = vb_in(); v.frcnt_write = 63; v.frcnt_pickup = 1;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_MISSED_FRAME);
	/* one step by FRCNT, but 65 frames by the clock (an aliased count) */
	v = vb_in(); v.dt_ns = 1083000000ull;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_MISSED_FRAME);
	v = vb_in(); v.dt_ns = (uint64_t)SWAP_VB_DT_US_MAX * 1000u;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED); /* the bound itself is inside */
	/* without a clock the time bound is not applied */
	v = vb_in(); v.timed = false; v.dt_ns = 1083000000ull;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_LANDED);

	/* a pickup timeout: the tight loop ran out, whatever FRCNT says */
	v = vb_in(); v.tight_pickup = false; v.frcnt_pickup = 11;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_PICKUP_TIMEOUT);
	v.frcnt_pickup = 30;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_PICKUP_TIMEOUT);

	/* no step found: LPTRS not written */
	v = vb_in(); v.found = false; v.swapped = false; v.tight_pickup = false;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_NO_VBLANK);
	/* written late (outside the window) */
	v = vb_in(); v.in_vblank = false;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_NOT_IN_VBLANK);
	/* order: not in the vblank before the pickup */
	v.tight_pickup = false;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_NOT_IN_VBLANK);
	/* no wait made, or no record */
	v = vb_in(); v.tried = false;
	CHECK(swap_vblank_judge(&v) == SWAP_VB_NOT_TRIED);
	CHECK(swap_vblank_judge(NULL) == SWAP_VB_NOT_TRIED);

	/* the sequence: the first swap that did not land */
	for (unsigned int i = 0; i < 4; i++)
		seq[i] = vb_in();
	CHECK(swap_vblank_first_miss(seq, 4, &why) == -1 && why == SWAP_VB_LANDED);
	seq[2].frcnt_pickup = 13;
	seq[3].tight_pickup = false;
	CHECK(swap_vblank_first_miss(seq, 4, &why) == 2 && why == SWAP_VB_MISSED_FRAME);
	CHECK(swap_vblank_first_miss(seq, 2, NULL) == -1); /* only the swaps made */
	seq[2] = vb_in();
	CHECK(swap_vblank_first_miss(seq, 4, &why) == 3 && why == SWAP_VB_PICKUP_TIMEOUT);
	CHECK(swap_vblank_first_miss(NULL, 0, &why) == -1);
	CHECK(swap_vblank_first_miss(NULL, 1, &why) == 0 && why == SWAP_VB_NOT_TRIED);

	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_LANDED), "landed"));
	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_NOT_TRIED), "not-tried"));
	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_NO_VBLANK), "no-vblank"));
	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_NOT_IN_VBLANK), "write-not-in-vblank"));
	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_PICKUP_TIMEOUT), "pickup-timeout"));
	CHECK(!strcmp(swap_vb_reason_name(SWAP_VB_MISSED_FRAME), "missed-frame"));
	CHECK(!strcmp(swap_vb_reason_name((enum swap_vb_reason)99), "?"));
}

int main(void)
{
	test_judge();
	test_poll();
	test_seq();
	test_restore();
	test_hw_pos();
	test_hw_restore();
	test_hw_seq();
	test_names();
	test_lact_named();
	test_lact_seq();
	test_lact_restore();
	test_vblank();
	printf("swap_check_test: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
