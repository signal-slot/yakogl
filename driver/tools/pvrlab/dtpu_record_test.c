// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* Host selftest of gl_test_dtpu.inc (case 222's checks 3..8 and their PERF
 * page). Build and run:
 *
 *   gcc -std=c99 -Wall -Werror -O2 driver/tools/pvrlab/dtpu_record_test.c -o $TMPDIR/drt && $TMPDIR/drt
 *
 * Prints "dtpu_record_test: N ok, 0 failed" and exits 0 when all pass.
 */
#include <stdio.h>

#include "gl_test_dtpu.inc"

static int ok, failed;

#define CHECK(cond)                                                          \
	do {                                                                 \
		if (cond) {                                                  \
			ok++;                                                \
		} else {                                                     \
			failed++;                                            \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
		}                                                            \
	} while (0)

static uint32_t p0[DTPU_PIXELS], p1[DTPU_PIXELS], p2[DTPU_PIXELS], p3[DTPU_PIXELS];

/* the decoding shader's pixel: R hi, G mid, B lo, A 255 */
static uint32_t kd_px(uint32_t k)
{
	return (k >> 16) | ((k >> 8) & 0xffu) << 8 | (k & 0xffu) << 16 | 0xffu << 24;
}

/* P2, the raw word: A = bits 23..16, R = 15..8, G = 7..0, B = stencil */
static uint32_t raw_px(uint32_t k, uint32_t stencil)
{
	return (k >> 16) << 24 | ((k >> 8) & 0xffu) | (k & 0xffu) << 8 | stencil << 16;
}

/* all four arms as a correct board would return them */
static void good(void)
{
	for (uint32_t i = 0; i < DTPU_PIXELS; i++) {
		const uint32_t n = dtpu_n(i), n3 = n + 32;

		p0[i] = p1[i] = kd_px(n << 17);
		p2[i] = raw_px(n << 17, 0);
		p3[i] = kd_px(n3 < 128 ? n3 << 17 : 0xffffffu);
	}
}

static void scan(uint32_t bad[5], uint32_t w[3])
{
	dtpu_scan(p0, p1, p2, p3, bad);
	dtpu_encode(bad[0], bad[1], bad[2], bad[3], bad[4], 0, w);
}

int main(void)
{
	uint32_t bad[5], w[3], r[3];

	/* the pixel helpers: index 0 is GL (0, 63), n 64 */
	CHECK(dtpu_n(0) == 64 && dtpu_n(63) == 127 && dtpu_n(4032) == 1 && dtpu_n(4095) == 64);
	CHECK(dtpu_kd(kd_px(0xabcdefu)) == 0xabcdefu);
	CHECK(dtpu_word(raw_px(0xabcdefu, 0x5a)) == 0xabcdefu);

	/* everything right: valid bit only, verdict EXACT, all indices 4096 */
	good();
	scan(bad, w);
	CHECK(w[0] == (1u | DTPU_V_EXACT << 12));
	CHECK(w[1] == (4096u | 4096u << 16) && w[2] == (4096u | 4096u << 16));

	/* P1 differs from P0 at index 1 by 200 (within 255): check 4 alone */
	good();
	p1[1] = kd_px((dtpu_n(1) << 17) + 200);
	scan(bad, w);
	CHECK(w[0] == (1u | 1u << 4 | DTPU_V_FEWBITS << 12));
	CHECK((w[1] & 0xffffu) == 1 && (w[1] >> 16) == 4096 && w[2] == (4096u | 4096u << 16));

	/* P1 garbage from index 5, P2 fine: checks 4 and 5 fail, raw works */
	good();
	for (uint32_t i = 5; i < DTPU_PIXELS; i++)
		p1[i] = kd_px(0);
	scan(bad, w);
	CHECK(w[0] == (1u | 1u << 4 | 1u << 5 | DTPU_V_RAW << 12));
	CHECK((w[1] & 0xffffu) == 5 && (w[1] >> 16) == 5 && (w[2] & 0xffffu) == 4096);

	/* P1 and P2 both wrong at different first indices; the stencil byte
	 * is not part of the word; k - 1 is accepted */
	good();
	p1[7] = kd_px(0);
	p2[9] = raw_px(dtpu_n(9) << 17, 0xff);
	p2[10] = raw_px((dtpu_n(10) << 17) - 1, 0);
	p2[11] = raw_px((dtpu_n(11) << 17) + 5, 0);
	scan(bad, w);
	CHECK(w[0] == (1u | 1u << 4 | 1u << 5 | 1u << 6 | DTPU_V_NONE << 12));
	CHECK((w[1] & 0xffffu) == 7 && (w[1] >> 16) == 7);
	CHECK((w[2] & 0xffffu) == 11 && (w[2] >> 16) == 4096);

	/* P3 stale (the shallow ramp): check 7 only, verdict STALE */
	good();
	p3[100] = kd_px(dtpu_n(100) << 17);
	scan(bad, w);
	CHECK(w[0] == (1u | 1u << 7 | DTPU_V_STALE << 12));
	CHECK((w[2] >> 16) == 100 && (w[2] & 0xffffu) == 4096);

	/* P0 off the ramp: bit 3, no verdict */
	good();
	p0[3] = kd_px(0);
	scan(bad, w);
	CHECK((w[0] & (1u << 3)) && ((w[0] >> 12) & 0xfu) == DTPU_V_NO_REF);

	/* a GL error is bit 8, its code in the high half */
	good();
	dtpu_scan(p0, p1, p2, p3, bad);
	dtpu_encode(bad[0], bad[1], bad[2], bad[3], bad[4], 0x0502u, w);
	CHECK(w[0] == (1u | 1u << 8 | DTPU_V_EXACT << 12 | 0x0502u << 16));

	/* the page: absent until kept, then tag 0xb0 only */
	dtpu_forget();
	CHECK(gl_test_dtpu_page(0xb0, r) == -1);
	dtpu_keep(w);
	CHECK(gl_test_dtpu_page(0xb0, r) == 0 && r[0] == w[0] && r[1] == w[1] && r[2] == w[2]);
	CHECK(gl_test_dtpu_page(0xb1, r) == -1 && r[0] == 0);
	dtpu_forget();
	CHECK(gl_test_dtpu_page(0xb0, r) == -1);

	printf("dtpu_record_test: %d ok, %d failed\n", ok, failed);
	return failed ? 1 : 0;
}
