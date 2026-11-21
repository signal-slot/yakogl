// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- case 222 (dtpu): the outcome of checks 3..8 as record words ----------
 * Included by gl_test.c (or by dtpu_record_test.c on the host). Plain C99, no
 * GL calls. docs/notes/gc-depth-sample.md, "Reading the dtpu page".
 *
 * The CASE record keeps the first failing check only (gl_test_check), so an
 * X2 run that fails check 4 hides checks 5, 6 and 7. The case therefore keeps
 * every check's outcome and the run can end with a PERF page of them:
 * selector GC_RESULT_PERF_SEL | 0xb0 (0x10b0) on an image built for case 222.
 *
 * The page (w0 = the record's w1 ... w2 = w3; w0's low byte of the record is
 * the tag 0xb0):
 *   w0  bit 0           the page is valid (the arms ran)
 *       bit n, 3..8     check n failed (3 P0 off the ramp, 4 P1 != P0,
 *                       5 P1 beyond 255 of P0, 6 P2 not the word,
 *                       7 P3 off the deeper ramp, 8 a GL error in the arms)
 *       bits 12..15     verdict code (DTPU_V_*)
 *       bits 16..31     the first GL error of the arms (0 none)
 *   w1  bits 0..15      first differing top-down index for check 4 (P1),
 *       bits 16..31     for check 5 (P1); 4096 = none
 *   w2  bits 0..15      first differing top-down index for check 6 (P2),
 *       bits 16..31     for check 7 (P3); 4096 = none
 * Index i = y_top * 64 + x; GL y = 63 - y_top. Check 3's index is not kept
 * (a failure of the reference voids the rest; bit 3 says so). */
#include <stdint.h>
#include <string.h>

#define DTPU_PIXELS 4096u
#define DTPU_NONE DTPU_PIXELS
#define DTPU_PAGE_TAG 0xb0u

enum dtpu_verdict {
	DTPU_V_NO_REF = 0,   /* check 3 failed: no verdict */
	DTPU_V_EXACT = 1,    /* D24X8 exact (check 4 and 7 pass) */
	DTPU_V_STALE = 2,    /* D24X8 exact but stale without a wait (check 7) */
	DTPU_V_FEWBITS = 3,  /* D24X8 within 255 (check 5 passes, 4 fails) */
	DTPU_V_RAW = 4,      /* D24X8 off, the raw view is the word (check 6 passes) */
	DTPU_V_NONE = 5,     /* neither addresses the surface */
};

#if defined(__GNUC__)
#define DTPU_FN static __attribute__((unused))
#else
#define DTPU_FN static
#endif

DTPU_FN uint32_t dtpu_verdict(uint32_t bad3, uint32_t bad4, uint32_t bad5, uint32_t bad6, uint32_t bad7)
{
	if (bad3 != DTPU_NONE)
		return DTPU_V_NO_REF;
	if (bad4 == DTPU_NONE)
		return bad7 == DTPU_NONE ? DTPU_V_EXACT : DTPU_V_STALE;
	if (bad5 == DTPU_NONE)
		return DTPU_V_FEWBITS;
	return bad6 == DTPU_NONE ? DTPU_V_RAW : DTPU_V_NONE;
}

/* `bad8` is the first GL error of the arms (0 none). */
DTPU_FN void dtpu_encode(uint32_t bad3, uint32_t bad4, uint32_t bad5, uint32_t bad6, uint32_t bad7, uint32_t bad8,
			 uint32_t w[3])
{
	uint32_t m = 1u;

	m |= (bad3 != DTPU_NONE) << 3 | (bad4 != DTPU_NONE) << 4 | (bad5 != DTPU_NONE) << 5 |
	     (bad6 != DTPU_NONE) << 6 | (bad7 != DTPU_NONE) << 7 | (bad8 != 0u) << 8;
	m |= dtpu_verdict(bad3, bad4, bad5, bad6, bad7) << 12;
	m |= (bad8 & 0xffffu) << 16;
	w[0] = m;
	w[1] = (bad4 & 0xffffu) | (bad5 & 0xffffu) << 16;
	w[2] = (bad6 & 0xffffu) | (bad7 & 0xffffu) << 16;
}

static struct {
	uint32_t valid;
	uint32_t w[3];
} s_dtpu_rec;

/* The page of the last run of case 222: tag 0xb0, w[0..2] the record's
 * w1..w3. 0 if the arms ran, -1 if not (check 1 or 2 stopped the case). */
int gl_test_dtpu_page(unsigned int tag, uint32_t *w)
{
	w[0] = w[1] = w[2] = 0;
	if (!s_dtpu_rec.valid || tag != DTPU_PAGE_TAG)
		return -1;
	memcpy(w, s_dtpu_rec.w, sizeof(s_dtpu_rec.w));
	return 0;
}

DTPU_FN void dtpu_keep(const uint32_t w[3])
{
	memcpy(s_dtpu_rec.w, w, sizeof(s_dtpu_rec.w));
	s_dtpu_rec.valid = 1u;
}

DTPU_FN void dtpu_forget(void)
{
	s_dtpu_rec.valid = 0u;
}

/* ---- the checks over the four arms' read-back pixels (top-down, 64x64) ---- */

/* the decoded 24-bit depth of a P0 / P1 / P3 pixel: R * 65536 + G * 256 + B */
DTPU_FN uint32_t dtpu_kd(uint32_t px)
{
	return (px & 0xffu) << 16 | ((px >> 8) & 0xffu) << 8 | ((px >> 16) & 0xffu);
}

/* the depth bits of a P2 pixel (the raw word's): A * 65536 + R * 256 + G */
DTPU_FN uint32_t dtpu_word(uint32_t px)
{
	return (px >> 24) << 16 | (px & 0xffu) << 8 | ((px >> 8) & 0xffu);
}

DTPU_FN uint32_t dtpu_dist(uint32_t a, uint32_t b)
{
	return a > b ? a - b : b - a;
}

/* n = x + y + 1 at top-down index i (GL y = 63 - row) */
DTPU_FN uint32_t dtpu_n(uint32_t i)
{
	return i % 64u + (63u - i / 64u) + 1u;
}

/* The first failing top-down index of checks 3..7 (DTPU_NONE when none), each
 * on its own: a failure of one does not stop the others. */
DTPU_FN void dtpu_scan(const uint32_t *p0, const uint32_t *p1, const uint32_t *p2, const uint32_t *p3,
		       uint32_t bad[5])
{
	uint32_t c;

	for (c = 0; c < 5; c++)
		bad[c] = DTPU_NONE;
	for (uint32_t i = 0; i < DTPU_PIXELS; i++) {
		const uint32_t n = dtpu_n(i), k = n << 17, kd0 = dtpu_kd(p0[i]), w = dtpu_word(p2[i]);
		const uint32_t n3 = n + 32, k3 = n3 < 128 ? n3 << 17 : 0xffffffu;

		if (bad[0] == DTPU_NONE && dtpu_dist(kd0, k) > 3)
			bad[0] = i;
		if (bad[1] == DTPU_NONE && p1[i] != p0[i])
			bad[1] = i;
		if (bad[2] == DTPU_NONE && dtpu_dist(dtpu_kd(p1[i]), kd0) > 255)
			bad[2] = i;
		if (bad[3] == DTPU_NONE && ((w != k && w != k - 1) || dtpu_dist(w, kd0) > 2))
			bad[3] = i;
		if (bad[4] == DTPU_NONE && dtpu_dist(dtpu_kd(p3[i]), k3) > 3)
			bad[4] = i;
	}
}
