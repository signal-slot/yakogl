/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gm_perf_page.h: the glmark2 image's frame-breakdown record, packed
 * (docs/notes/imx8mp-frame-floor.md, "The frame-breakdown record"). Pure C,
 * no Zephyr: main.cpp gathers a scene's totals into struct gm_perf_scene,
 * and the functions here turn them into one page of a kind PERF record.
 * host/perf_page_test.c packs known pages into known words on the host.
 *
 * The record: w0 = YK_RECORD_MAGIC | YK_RECORD_KIND_PERF << 8 | page
 * (experiment 0, so the tag is the page: 0x594b0c00 | page); w1..w3 = six
 * 16-bit values v0..v5, v0 in w1's low half (w1 = v1 << 16 | v0,
 * w2 = v3 << 16 | v2, w3 = v5 << 16 | v4). yakogl-tools record decodes it
 * as "experiment 0 page p: v0 v1 v2 v3 v4 v5". Every value saturates at
 * 0xffff. A mean whose count is 0 is 0. Means are rounded down.
 *
 *   page 0, the frame (us per frame, over the scene's frames):
 *     v0 frames, v1 frame (t2 - t0), v2 gl (t1 - t0), v3 swap, v4 show,
 *     v5 frame to frame (t0 to the next t0, over the intervals)
 *   page 1, the swap (per frame, or per 100 frames):
 *     v0 wait us, v1 waits per 100 frames, v2 gc_render_wait sleeps per 100
 *     frames, v3 GPU busy us, v4 kicks per 100 frames, v5 submit us
 *   page 2, the display (counts of frame-to-frame intervals):
 *     v0 under 1.5 T, v1 in [1.5 T, 2.5 T), v2 at 2.5 T or more, v3 T in us
 *     (0: not measured, and v0..v2 are 0), v4 frames whose present (swap +
 *     show) took 1000 us or more, v5 the kernel tick rate
 *   page 3, the GPU (per frame-to-frame interval):
 *     v0 HI total kcycles, v1 HI idle kcycles, v2 KiB read, v3 KiB written,
 *     v4 core MHz (total cycles / frame-to-frame us), v5 0
 */
#ifndef GM_PERF_PAGE_H
#define GM_PERF_PAGE_H

#include <stdint.h>

#include <yakogl_record.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GM_PERF_PAGES 4u

/* page 2's histogram of frame-to-frame times: bucket b holds the intervals
 * of b * GM_PERF_BUCKET_US us up to the next bucket; the last one holds
 * everything from its start on (65.5 ms: 2.5 T is 42.5 ms at 58.76 Hz) */
#define GM_PERF_BUCKET_US 32u
#define GM_PERF_BUCKETS 2048u

/* One scene's totals, as main.cpp gathers them. Times in microseconds. */
struct gm_perf_scene {
	/* page 0: over the scene's frames (each frame's t0 .. t2) */
	uint64_t frames;
	uint64_t frame_us, gl_us, swap_us, show_us;
	/* the frame-to-frame intervals (frames - 1 of them: the last frame has
	 * no next t0) and their sum */
	uint64_t intervals;
	uint64_t interval_us;
	/* page 1: the differences over the scene of the GC7000 OSAL's and
	 * driver's counters */
	uint64_t wait_us, waits, sleeps, busy_us, kicks, submit_us;
	/* page 2: gm_perf_bins' result, T, the slow presents, the tick rate */
	uint32_t bins[3];
	uint32_t period_us;
	uint64_t slow_presents;
	uint32_t ticks_per_sec;
	/* page 3: HI_PROFILE sums over the intervals; bytes = BYTES8 x 8 */
	uint64_t hi_total_cycles, hi_idle_cycles, read_bytes, write_bytes;
};

static inline uint32_t gm_perf_sat(uint64_t v)
{
	return v > 0xffffu ? 0xffffu : (uint32_t)v;
}

/* sum / n, rounded down; 0 for n = 0 */
static inline uint64_t gm_perf_mean(uint64_t sum, uint64_t n)
{
	return n == 0u ? 0u : sum / n;
}

/* Page 2's three bins from the histogram: an interval counts at its
 * bucket's start (t = b * bucket_us, so up to bucket_us early) and goes to
 * bin 0 when t < 1.5 T, bin 1 when t < 2.5 T, bin 2 otherwise. T = 0 (not
 * measured): all three 0. */
static inline void gm_perf_bins(const uint32_t *hist, uint32_t buckets, uint32_t bucket_us,
				uint32_t period_us, uint32_t bins[3])
{
	uint64_t n[3] = {0u, 0u, 0u};

	for (uint32_t b = 0; period_us != 0u && b < buckets; b++) {
		const uint64_t t2 = 2u * (uint64_t)b * bucket_us; /* 2 t */

		n[t2 < 3u * (uint64_t)period_us ? 0 : t2 < 5u * (uint64_t)period_us ? 1 : 2] += hist[b];
	}
	for (int i = 0; i < 3; i++) {
		bins[i] = n[i] > 0xffffffffu ? 0xffffffffu : (uint32_t)n[i];
	}
}

/* The six values of `page`. -1 for a page that is not 0..3. */
static inline int gm_perf_page_values(uint32_t page, const struct gm_perf_scene *s, uint32_t v[6])
{
	const uint64_t f = s->frames, k = s->intervals;
	uint64_t x[6] = {0u, 0u, 0u, 0u, 0u, 0u};

	switch (page) {
	case 0:
		x[0] = f;
		x[1] = gm_perf_mean(s->frame_us, f);
		x[2] = gm_perf_mean(s->gl_us, f);
		x[3] = gm_perf_mean(s->swap_us, f);
		x[4] = gm_perf_mean(s->show_us, f);
		x[5] = gm_perf_mean(s->interval_us, k);
		break;
	case 1:
		x[0] = gm_perf_mean(s->wait_us, f);
		x[1] = gm_perf_mean(s->waits * 100u, f);
		x[2] = gm_perf_mean(s->sleeps * 100u, f);
		x[3] = gm_perf_mean(s->busy_us, f);
		x[4] = gm_perf_mean(s->kicks * 100u, f);
		x[5] = gm_perf_mean(s->submit_us, f);
		break;
	case 2:
		x[0] = s->bins[0];
		x[1] = s->bins[1];
		x[2] = s->bins[2];
		x[3] = s->period_us;
		x[4] = s->slow_presents;
		x[5] = s->ticks_per_sec;
		break;
	case 3:
		x[0] = gm_perf_mean(s->hi_total_cycles / 1000u, k);
		x[1] = gm_perf_mean(s->hi_idle_cycles / 1000u, k);
		x[2] = gm_perf_mean(s->read_bytes / 1024u, k);
		x[3] = gm_perf_mean(s->write_bytes / 1024u, k);
		x[4] = gm_perf_mean(s->hi_total_cycles, s->interval_us); /* cycles per us = MHz */
		break;
	default:
		return -1;
	}
	for (int i = 0; i < 6; i++) {
		v[i] = gm_perf_sat(x[i]);
	}
	return 0;
}

/* The record's four words for `page`: w[0] as board_result_tag stores it
 * (tag = page), w[1..3] the values. -1 for a page that is not 0..3. */
static inline int gm_perf_page_words(uint32_t page, const struct gm_perf_scene *s, uint32_t w[4])
{
	uint32_t v[6];

	if (gm_perf_page_values(page, s, v) != 0) {
		return -1;
	}
	w[0] = YK_RECORD_MAGIC | YK_RECORD_KIND_PERF << 8 | page;
	w[1] = v[1] << 16 | v[0];
	w[2] = v[3] << 16 | v[2];
	w[3] = v[5] << 16 | v[4];
	return 0;
}

#ifdef __cplusplus
}
#endif

#endif /* GM_PERF_PAGE_H */
