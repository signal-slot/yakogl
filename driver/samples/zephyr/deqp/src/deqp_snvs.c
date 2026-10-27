/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The dEQP results in the SNVS record (gc_result.h), for a board whose
 * console cannot leave it (the Verdin iMX8M Plus on a Mallow carrier has no
 * UART). Rewritten on every case begin and result, so a run that hangs or
 * faults still says how far it got.
 *
 * Up to 32 cases the record is the DEQP_CASES bitmap (kind 9): every case's
 * own outcome, so one run names each failure, and the GPU driver's reset and
 * fence-timeout counters. Past 32 it becomes the DEQP tally (kind 8): counts,
 * and the first failing case's name check (gc_result.h).
 */
#include <stdlib.h>
#include <string.h>
#include <zephyr/sys/util.h>
#include <GLES2/gl2.h>
#include "gc_result.h"

/* The Vivante driver's counters (gc_render.h), declared here rather than
 * included: this file is built only on the GC7000 boards, and needs no more. */
uint32_t gc_render_resets(void);
void gc_render_counters(uint32_t *fence_timeouts, uint32_t *event_busy, uint32_t *wait_ms);

#ifdef CONFIG_DEQP_SNVS_ADDRESS
uint64_t yakogl_debug_default_target_gpu(void);
uint32_t yakogl_gpu_work_lost(void);
uint32_t gc_osal_no_reuse_leaked_kib(void);
#endif

static uint32_t s_pass, s_fail, s_nsup, s_other, s_begun;
static uint32_t s_first_fail = 0xffffu;
static uint32_t s_first_name = 0xffffu;   /* the first Fail's name check; none yet */

/* CRC-16/CCITT-FALSE: poly 0x1021, init 0xffff, no reflection */
static uint16_t crc16_name(const char *s)
{
	uint16_t crc = 0xffffu;

	for (; *s; s++) {
		crc ^= (uint16_t)((uint8_t)*s << 8);
		for (int b = 0; b < 8; b++) {
			crc = (crc & 0x8000u) ? (uint16_t)(crc << 1 ^ 0x1021u) : (uint16_t)(crc << 1);
		}
	}
	return crc;
}

static uint32_t gpu_counters(void)
{
	uint32_t timeouts = 0, busy = 0, wait_ms = 0;

	gc_render_counters(&timeouts, &busy, &wait_ms);
#ifdef CONFIG_DEQP_SNVS_ADDRESS
	/* Kconfig DEQP_SNVS_ADDRESS: the address A/B's own layout */
	return (uint32_t)(yakogl_debug_default_target_gpu() & 0xfffu) << 20 |
	       MIN(yakogl_gpu_work_lost(), 15u) << 16 | MIN(gc_render_resets(), 15u) << 12 |
	       MIN(timeouts, 15u) << 8 | MIN(gc_osal_no_reuse_leaked_kib() / 1024u, 255u);
#else
	return s_first_name << 16 | MIN(gc_render_resets(), 15u) << 12 | MIN(timeouts, 15u) << 8 |
	       MIN(s_first_fail, 0xffu);
#endif
}
/* sample_glue.c's fatal handler tags its FAULT record with this: a crash
 * still says which case it was in (the 1-based count of cases begun). */
uint8_t pvr_sample_fault_tag(void)
{
	return (uint8_t)MIN(s_begun, 255u);
}

static uint32_t s_bits[2];
static uint8_t s_seq;

/* Selector GC_RESULT_DEQP_SEL_FAILS | page << 8 | index: the page of Fails
 * to name (kind DEQP_FAILS), or -1 for the usual records. */
static int fails_page(void)
{
	int sel = gc_result_selector();

	return sel >= 0 && (sel & GC_RESULT_DEQP_SEL_FAILS) ? (sel >> 8) & 0x7f : -1;
}

static uint16_t s_page_names[GC_RESULT_DEQP_FAILS_PER_PAGE] = {
	0xffffu, 0xffffu, 0xffffu, 0xffffu, 0xffffu, 0xffffu,
};

static void write_fails_page(void)
{
	gc_result_write_tag(GC_RESULT_DEQP_FAILS, (uint8_t)MIN(s_fail, 255u),
			    s_page_names[0] | (uint32_t)s_page_names[1] << 16,
			    s_page_names[2] | (uint32_t)s_page_names[3] << 16,
			    s_page_names[4] | (uint32_t)s_page_names[5] << 16);
}

/* Selector GC_RESULT_DEQP_SEL_PROBE (gc_result.h): the first failing case's
 * picture, five pixels of it, from the thread that logged the result (the
 * test thread, its context current). */
static bool s_probed;

static uint16_t rgb565(const uint8_t *px)
{
	return (uint16_t)((px[0] >> 3) << 11 | (px[1] >> 2) << 5 | px[2] >> 3);
}

static void probe_first_fail(void)
{
	const int w = CONFIG_DEQP_SURFACE_SIZE, h = CONFIG_DEQP_SURFACE_SIZE;
	int at[5][2] = {
		{ w / 2, h / 2 }, { w / 4, h / 4 }, { 3 * w / 4, h / 4 },
		{ w / 4, 3 * h / 4 }, { 3 * w / 4, 3 * h / 4 },
	};
	const char *pt = CONFIG_DEQP_PROBE_POINTS;
	uint8_t px[5][4];
	uint8_t rgb[15];
	uint16_t p[5], crc = 0xffffu;

	if (s_probed) {
		return;
	}
	s_probed = true;
	/* CONFIG_DEQP_PROBE_POINTS: "x,y;x,y;..." replaces the defaults in order */
	for (int i = 0; i < 5 && *pt; i++) {
		char *end;
		long x = strtol(pt, &end, 10);
		long y;

		if (end == pt || *end != ',') {
			break;
		}
		pt = end + 1;
		y = strtol(pt, &end, 10);
		if (end == pt) {
			break;
		}
		at[i][0] = (int)x;
		at[i][1] = (int)y;
		pt = *end == ';' ? end + 1 : end;
	}
	for (int i = 0; i < 5; i++) {
		memset(px[i], 0, 4);
		glReadPixels(at[i][0], at[i][1], 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px[i]);
		p[i] = rgb565(px[i]);
		memcpy(rgb + 3 * i, px[i], 3);
	}
	for (int i = 0; i < 15; i++) {
		crc ^= (uint16_t)(rgb[i] << 8);
		for (int b = 0; b < 8; b++) {
			crc = (crc & 0x8000u) ? (uint16_t)(crc << 1 ^ 0x1021u) : (uint16_t)(crc << 1);
		}
	}
	gc_result_write_tag(GC_RESULT_PROBES, 2u, p[0] | (uint32_t)p[1] << 16,
			    p[2] | (uint32_t)p[3] << 16, p[4] | (uint32_t)crc << 16);
}

void deqp_snvs_progress(const char *status, const char *name)
{
	const int page = fails_page();
	const int sel = gc_result_selector();

	if (sel >= 0 && (sel & GC_RESULT_DEQP_SEL_STATUS)) {
		static bool s_status_done;

		if (status == NULL) {
			s_begun++;
		} else if (!s_status_done && strcmp(status, "Pass") != 0 &&
			   strcmp(status, "Fail") != 0 && strcmp(status, "NotSupported") != 0) {
			uint32_t w[3] = { 0, 0, 0 };

			for (int i = 0; i < 12 && status[i]; i++) {
				w[i / 4] |= (uint32_t)(uint8_t)status[i] << (8 * (i % 4));
			}
			gc_result_write_tag(GC_RESULT_DEQP_STATUS,
					    (uint8_t)MIN(s_begun > 0u ? s_begun - 1u : 0u, 255u),
					    w[0], w[1], w[2]);
			s_status_done = true;
		}
		return;
	}
	if (sel >= 0 && (sel & GC_RESULT_DEQP_SEL_PROBE)) {
		if (status != NULL && strcmp(status, "Pass") != 0 &&
		    strcmp(status, "NotSupported") != 0) {
			probe_first_fail();
		}
		if (s_probed) {
			return; /* the probes record stays */
		}
	}

	if (page >= 0) {
		/* a Fail, or any status other than Pass and NotSupported (a
		 * crash, a timeout, an internal error: the ones that can end
		 * a run early) */
		if (status != NULL && strcmp(status, "Pass") != 0 &&
		    strcmp(status, "NotSupported") != 0) {
			uint32_t k = s_fail++;
			uint32_t first = (uint32_t)page * GC_RESULT_DEQP_FAILS_PER_PAGE;

			if (k >= first && k < first + GC_RESULT_DEQP_FAILS_PER_PAGE) {
				s_page_names[k - first] = crc16_name(name ? name : "");
			}
		}
		write_fails_page();
		return;
	}

	uint32_t st = 0;

	if (status == NULL) {
		s_begun++;
	} else if (strcmp(status, "Pass") == 0) {
		s_pass++;
		st = 1;
	} else if (strcmp(status, "Fail") == 0) {
		s_fail++;
		st = 2;
		if (s_first_fail == 0xffffu && s_begun > 0u) {
			s_first_fail = MIN(s_begun - 1u, 0xfffeu);
			s_first_name = crc16_name(name ? name : "");
		}
	} else if (strcmp(status, "NotSupported") == 0) {
		s_nsup++;
		st = 3;
	} else {
		s_other++;
		st = 3;
	}
	if (s_begun <= GC_RESULT_DEQP_CASES_MAX) {
		if (st != 0u && s_begun > 0u) {
			uint32_t i = s_begun - 1u;

			s_bits[i / 16u] |= st << (2u * (i % 16u));
		}
		gc_result_write_tag(GC_RESULT_DEQP_CASES, (uint8_t)s_begun, s_bits[0], s_bits[1],
				    gpu_counters());
		return;
	}
	gc_result_write_tag(GC_RESULT_DEQP, ++s_seq, MIN(s_pass, 0xffffu) | MIN(s_fail, 0xffffu) << 16,
			    MIN(s_nsup, 0xffffu) | MIN(s_other, 0xffffu) << 16,
			    MIN(s_begun, 0xffffu) | s_first_name << 16);
}
