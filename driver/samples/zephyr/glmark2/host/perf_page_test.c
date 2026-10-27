/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * perf_page_test.c: the frame-breakdown record's packing (src/gm_perf_page.h)
 * on the host. Known totals in, known words out, for the four pages, the
 * saturation, the empty scene, page 2's bin edges and a page that is not
 * one. Prints each page as a record line (yk-record: w0 w1 w2 w3), the form
 * yakogl-tools record reads; run.sh decodes them. Exit 1 on a difference.
 */
#include <stdio.h>
#include <string.h>

#include "gm_perf_page.h"

static int s_failed;

static void expect_words(const char *what, uint32_t page, const struct gm_perf_scene *s, uint32_t w0,
			 uint32_t w1, uint32_t w2, uint32_t w3, int print)
{
	uint32_t w[4] = {0u, 0u, 0u, 0u};

	if (gm_perf_page_words(page, s, w) != 0) {
		printf("FAIL %s: page %u refused\n", what, page);
		s_failed = 1;
		return;
	}
	if (w[0] != w0 || w[1] != w1 || w[2] != w2 || w[3] != w3) {
		printf("FAIL %s: got %08x %08x %08x %08x, want %08x %08x %08x %08x\n", what, w[0], w[1],
		       w[2], w[3], w0, w1, w2, w3);
		s_failed = 1;
		return;
	}
	if (print) {
		printf(YK_RECORD_LINE_FMT, w[0], w[1], w[2], w[3]);
	}
}

int main(void)
{
	struct gm_perf_scene s;
	static uint32_t hist[GM_PERF_BUCKETS];
	uint32_t w[4];

	/* page 0: 587 frames of 17 018 us (clear, view, before F1: the note's
	 * step 3), gl 100, swap 200, show 16 000, 586 intervals of 17 018 */
	memset(&s, 0, sizeof(s));
	s.frames = 587;
	s.frame_us = 587u * 17018u;
	s.gl_us = 587u * 100u;
	s.swap_us = 587u * 200u + 586u; /* rounded down: 200 */
	s.show_us = 587u * 16000u;
	s.intervals = 586;
	s.interval_us = 586u * 17018u;
	expect_words("page 0", 0, &s, 0x594b0c00u, 0x427a024bu, 0x00c80064u, 0x427a3e80u, 1);

	/* page 1: 100 frames; wait 1500 us, 50 waits, 3 sleeps, busy 2000 us,
	 * 100 kicks, submit 50 us per frame */
	memset(&s, 0, sizeof(s));
	s.frames = 100;
	s.wait_us = 150000;
	s.waits = 50;
	s.sleeps = 3;
	s.busy_us = 200000;
	s.kicks = 100;
	s.submit_us = 5000;
	expect_words("page 1", 1, &s, 0x594b0c01u, 0x003205dcu, 0x07d00003u, 0x00320064u, 1);

	/* page 2: T = 17 018 us. 580 intervals at 17 000 us (bucket 531),
	 * the edge of 1.5 T = 25 527 us: bucket 797 starts at 25 504 (bin 0),
	 * 798 at 25 536 (bin 1); 5 at 34 000 (bucket 1062, bin 1); the edge of
	 * 2.5 T = 42 545 us: bucket 1329 starts at 42 528 (bin 1), 1330 at
	 * 42 560 (bin 2); 1 at 60 000 us and 1 in the last bucket (bin 2) */
	memset(hist, 0, sizeof(hist));
	hist[17000u / GM_PERF_BUCKET_US] = 580;
	hist[797] = 1;
	hist[798] = 1;
	hist[34000u / GM_PERF_BUCKET_US] = 5;
	hist[1329] = 1;
	hist[1330] = 1;
	hist[60000u / GM_PERF_BUCKET_US] = 1;
	hist[GM_PERF_BUCKETS - 1u] = 1;
	memset(&s, 0, sizeof(s));
	gm_perf_bins(hist, GM_PERF_BUCKETS, GM_PERF_BUCKET_US, 17018u, s.bins);
	if (s.bins[0] != 581u || s.bins[1] != 7u || s.bins[2] != 3u) {
		printf("FAIL page 2 bins: got %u %u %u, want 581 7 3\n", s.bins[0], s.bins[1], s.bins[2]);
		s_failed = 1;
	}
	s.period_us = 17018;
	s.slow_presents = 587;
	s.ticks_per_sec = 10000;
	expect_words("page 2", 2, &s, 0x594b0c02u, 0x00070245u, 0x427a0003u, 0x2710024bu, 1);

	/* page 2 with T not measured: no bins */
	gm_perf_bins(hist, GM_PERF_BUCKETS, GM_PERF_BUCKET_US, 0u, s.bins);
	s.period_us = 0;
	expect_words("page 2, T 0", 2, &s, 0x594b0c02u, 0x00000000u, 0x00000000u, 0x2710024bu, 0);

	/* page 3: 586 intervals of 17 018 us at 800 MHz, a quarter busy;
	 * 2 MiB read and 1 920 000 bytes written per interval */
	memset(&s, 0, sizeof(s));
	s.intervals = 586;
	s.interval_us = 586u * 17018u;
	s.hi_total_cycles = 800ull * s.interval_us; /* 13 614 kcycles a frame */
	s.hi_idle_cycles = s.hi_total_cycles / 4u * 3u; /* 10 210 */
	s.read_bytes = 586ull * 2u * 1024u * 1024u;   /* 2048 KiB */
	s.write_bytes = 586ull * 1920000u;            /* 1875 KiB */
	expect_words("page 3", 3, &s, 0x594b0c03u, 0x27e2352eu, 0x07530800u, 0x00000320u, 1);

	/* saturation at 0xffff, and a scene without frames: every mean 0 */
	memset(&s, 0, sizeof(s));
	s.frames = 70000;
	s.frame_us = 70000ull * 70000u;
	s.show_us = 0;
	expect_words("page 0, saturated", 0, &s, 0x594b0c00u, 0xffffffffu, 0x00000000u, 0x00000000u, 0);
	memset(&s, 0, sizeof(s));
	s.wait_us = 12345;
	expect_words("page 1, no frames", 1, &s, 0x594b0c01u, 0u, 0u, 0u, 0);
	expect_words("page 3, no intervals", 3, &s, 0x594b0c03u, 0u, 0u, 0u, 0);

	/* not a page */
	if (gm_perf_page_words(4, &s, w) != -1) {
		printf("FAIL page 4: packed\n");
		s_failed = 1;
	}

	printf("perf_page_test: %s\n", s_failed ? "FAILED" : "ok");
	return s_failed;
}
