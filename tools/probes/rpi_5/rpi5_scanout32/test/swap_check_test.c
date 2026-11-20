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

static void test_names(void)
{
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
}

int main(void)
{
	test_judge();
	test_poll();
	test_seq();
	test_restore();
	test_names();
	printf("swap_check_test: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
