/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: the decision logic of the long hold (docs/notes/rpi5-display.md
 * section 4, Q2: does the firmware redraw into its plane or change its display
 * list?). The probe writes its text and pattern, then reads without writing
 * for about 20 s; this file decides what the reads say. Pure functions on
 * values: stdint/stddef/stdbool only, no Zephyr, host-tested by
 * test/hold_check_test.c (hold_check_selftest.sh).
 *
 * A hold is a series of samples about HC_SAMPLE_MS apart. Sample 0 is the
 * baseline taken right after our write; every later sample is compared to it.
 * Per sample the first failing check names the reason, in the order of
 * enum hc_reason (unclocked, PV disabled, FRCNT stalled, pointer, list, plane);
 * across samples the first sample that fails wins.
 *
 * Register layouts are those of driver/zephyr/lib/board/rpi5_scanout/
 * scanout_alive.h (STATUS.FRCNT 21:16, :61-64; all ones is an unclocked block,
 * :19-23; PV_CONTROL.EN and PV_V_CONTROL.VIDEN, :78-82). They are repeated here
 * as numbers so this header stands alone; the test checks them against
 * scanout_alive.h.
 */
#ifndef RPI5_SCANOUT16_HOLD_CHECK_H
#define RPI5_SCANOUT16_HOLD_CHECK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define HC_SAMPLE_MS		250u	/* the spacing of the samples */
#define HC_WDOG_MARGIN_MS	10000u	/* the hold ends this long before the watchdog */
#define HC_FRCNT_SHIFT		16u
#define HC_FRCNT_MASK		0x3fu
#define HC_FRCNT_MOD		64u
#define HC_ALL_ONES		0xffffffffu
#define HC_PV_CONTROL_EN	(1u << 0)
#define HC_PV_VCONTROL_VIDEN	(1u << 0)

/* --- (a) FNV-1a, 32 bit (the checksum of pattern.h's pattern_report) ------ */

#define HC_FNV_BASIS		0x811c9dc5u
#define HC_FNV_PRIME		0x01000193u

static inline uint32_t hc_fnv1a_update(uint32_t h, const volatile uint8_t *p, size_t n)
{
	while (n--) {
		h ^= *p++;
		h *= HC_FNV_PRIME;
	}
	return h;
}

static inline uint32_t hc_fnv1a(const volatile void *buf, size_t n)
{
	return hc_fnv1a_update(HC_FNV_BASIS, (const volatile uint8_t *)buf, n);
}

/* --- (b) the strided-row hash --------------------------------------------- */

/* The row read as the i-th of nrows spread evenly over h rows (0 .. h-1). */
static inline uint32_t hc_row_at(uint32_t i, uint32_t nrows, uint32_t h)
{
	return nrows ? (uint32_t)(((uint64_t)i * h) / nrows) : 0u;
}

/* FNV-1a over nrows whole rows of pitch bytes, spread evenly over h rows. */
static inline uint32_t hc_rows_hash(const volatile uint8_t *plane, uint32_t pitch,
				    uint32_t h, uint32_t nrows)
{
	uint32_t hash = HC_FNV_BASIS;

	for (uint32_t i = 0; i < nrows; i++)
		hash = hc_fnv1a_update(hash, plane + (size_t)hc_row_at(i, nrows, h) * pitch,
				       pitch);
	return hash;
}

/* How many of the h rows differ between two planes (whole rows of pitch bytes). */
static inline uint32_t hc_rows_changed(const volatile uint8_t *a, const volatile uint8_t *b,
				       uint32_t pitch, uint32_t h)
{
	uint32_t rows = 0;

	for (uint32_t y = 0; y < h; y++) {
		const volatile uint8_t *ra = a + (size_t)y * pitch;
		const volatile uint8_t *rb = b + (size_t)y * pitch;

		for (uint32_t i = 0; i < pitch; i++) {
			if (ra[i] != rb[i]) {
				rows++;
				break;
			}
		}
	}
	return rows;
}

/* --- (c) mismatches against an expected-pixel callback -------------------- */

/* The raw pixel value expected at (x, y), e.g. pattern_pixel() inside a band
 * and the text band's rendering elsewhere. */
typedef uint32_t (*hc_expect_fn)(void *ctx, uint32_t x, uint32_t y);

struct hc_mismatch {
	uint32_t checked;	/* pixels looked at */
	uint32_t count;		/* of those, differing from the expected value */
	uint32_t first_x;	/* of the first mismatch in row-major order, */
	uint32_t first_y;	/* valid only if count != 0 */
	uint32_t rows;		/* distinct rows with at least one mismatch */
};

/* bpp_bytes is 2 (RGB565) or 4; the pixels are little-endian. */
static inline struct hc_mismatch hc_mismatch_scan(const volatile uint8_t *plane, uint32_t pitch,
						  uint32_t w, uint32_t h, uint32_t bpp_bytes,
						  hc_expect_fn expect, void *ctx)
{
	struct hc_mismatch m = { 0, 0, 0, 0, 0 };

	for (uint32_t y = 0; y < h; y++) {
		const volatile uint8_t *row = plane + (size_t)y * pitch;
		bool hit = false;

		for (uint32_t x = 0; x < w; x++) {
			const volatile uint8_t *p = row + (size_t)x * bpp_bytes;
			uint32_t got = 0;

			for (uint32_t b = 0; b < bpp_bytes; b++)
				got |= (uint32_t)p[b] << (8u * b);
			m.checked++;
			if (got == expect(ctx, x, y))
				continue;
			if (!m.count) {
				m.first_x = x;
				m.first_y = y;
			}
			m.count++;
			hit = true;
		}
		if (hit)
			m.rows++;
	}
	return m;
}

/* --- (d) FRCNT progress --------------------------------------------------- */

static inline uint32_t hc_frcnt(uint32_t status)
{
	return (status >> HC_FRCNT_SHIFT) & HC_FRCNT_MASK;
}

/* Frames passed from status a to the later status b, modulo 64 (0 = stalled;
 * a whole 64 frames in one interval also reads 0, 250 ms is 15 frames at 60 Hz). */
static inline uint32_t hc_frcnt_delta(uint32_t a, uint32_t b)
{
	return (hc_frcnt(b) - hc_frcnt(a)) & (HC_FRCNT_MOD - 1u);
}

/* --- (e) (f) one register sample ------------------------------------------ */

/* All ones from a register: the block is unclocked or unpowered. */
static inline bool hc_unclocked(uint32_t reg)
{
	return reg == HC_ALL_ONES;
}

static inline bool hc_pv_running(uint32_t pv_control, uint32_t pv_vcontrol)
{
	return (pv_control & HC_PV_CONTROL_EN) && (pv_vcontrol & HC_PV_VCONTROL_VIDEN);
}

/* --- (g) the deadline clamp ----------------------------------------------- */

struct hc_clamp {
	uint32_t end_ms;	/* when the hold ends, on the same clock as start_ms */
	uint32_t hold_ms;	/* end_ms - start_ms */
	bool clamped;		/* the hold is shorter than wanted: report it */
};

/* hold_end_ms = min(start_ms + want_ms, hold_s * 1000 - 10000). The watchdog
 * deadline counts from boot (rpi_wdog.c:82-97), so start_ms is the uptime when
 * the hold starts. A limit at or before start_ms leaves no hold (end = start). */
static inline struct hc_clamp hc_clamp_hold(uint32_t start_ms, uint32_t want_ms, uint32_t hold_s)
{
	uint64_t want_end = (uint64_t)start_ms + want_ms;
	uint64_t total = (uint64_t)hold_s * 1000u;
	uint64_t limit = total > HC_WDOG_MARGIN_MS ? total - HC_WDOG_MARGIN_MS : 0u;
	uint64_t end = want_end < limit ? want_end : limit;
	struct hc_clamp c;

	if (end < start_ms)
		end = start_ms;
	c.end_ms = (uint32_t)end;
	c.hold_ms = (uint32_t)(end - start_ms);
	c.clamped = end < want_end;
	return c;
}

/* --- (h) the hold verdict ------------------------------------------------- */

enum hc_reason {
	HC_PASS = 0,
	HC_UNCLOCKED,		/* a register read all ones */
	HC_PV_DISABLED,		/* PV_CONTROL.EN or PV_V_CONTROL.VIDEN clear */
	HC_FRCNT_STALLED,	/* FRCNT did not advance since the sample before */
	HC_PTR_CHANGED,		/* the channel's list pointer is not the baseline's */
	HC_LIST_CHANGED,	/* the display list words are not the baseline's */
	HC_PLANE_CHANGED,	/* the plane hash is not the baseline's */
};

static inline const char *hc_reason_name(enum hc_reason r)
{
	switch (r) {
	case HC_PASS:
		return "pass";
	case HC_UNCLOCKED:
		return "unclocked";
	case HC_PV_DISABLED:
		return "pv disabled";
	case HC_FRCNT_STALLED:
		return "frcnt stalled";
	case HC_PTR_CHANGED:
		return "pointer changed";
	case HC_LIST_CHANGED:
		return "list changed";
	case HC_PLANE_CHANGED:
		return "plane changed";
	}
	return "?";
}

/* What the probe reads at one point of the hold. */
struct hc_sample {
	uint32_t status;	/* DISPx_STATUS */
	uint32_t dl_ptr;	/* the channel's list pointer register (DISPx_LPTRS or _DL) */
	uint32_t pv_control;
	uint32_t pv_vcontrol;
	uint32_t list_hash;	/* FNV-1a of the display list words */
	uint32_t plane_hash;	/* hc_rows_hash() of the plane */
	uint32_t rows_touched;	/* rows that differ from the baseline (hc_rows_changed), 0 if unknown */
};

struct hc_hold {
	enum hc_reason reason;	/* HC_PASS, or the first failure */
	uint32_t index;		/* the sample that failed (0 is the baseline); 0 on pass */
	uint32_t rows_touched;	/* of that sample, for HC_PLANE_CHANGED */
};

/* Judge samples s[0..n-1]; s[0] is the baseline. */
static inline struct hc_hold hc_judge_hold(const struct hc_sample *s, uint32_t n)
{
	struct hc_hold r = { HC_PASS, 0, 0 };

	for (uint32_t i = 0; i < n; i++) {
		const struct hc_sample *c = &s[i];
		enum hc_reason why = HC_PASS;

		if (hc_unclocked(c->status) || hc_unclocked(c->dl_ptr) ||
		    hc_unclocked(c->pv_control) || hc_unclocked(c->pv_vcontrol))
			why = HC_UNCLOCKED;
		else if (!hc_pv_running(c->pv_control, c->pv_vcontrol))
			why = HC_PV_DISABLED;
		else if (i && !hc_frcnt_delta(s[i - 1].status, c->status))
			why = HC_FRCNT_STALLED;
		else if (i && c->dl_ptr != s[0].dl_ptr)
			why = HC_PTR_CHANGED;
		else if (i && c->list_hash != s[0].list_hash)
			why = HC_LIST_CHANGED;
		else if (i && c->plane_hash != s[0].plane_hash)
			why = HC_PLANE_CHANGED;
		if (why != HC_PASS) {
			r.reason = why;
			r.index = i;
			r.rows_touched = c->rows_touched;
			return r;
		}
	}
	return r;
}

/* Did the firmware redraw: 'A' between the adopt and our write (the plane's
 * hash read at the adopt differs from the one read just before our write, with
 * nothing of ours in between), 'B' during the hold (plane changed), else 0. A is
 * the earlier and wins when both happened. */
static inline char hc_redraw_phase(uint32_t hash_adopt, uint32_t hash_before_write,
				   enum hc_reason hold_reason)
{
	if (hash_adopt != hash_before_write)
		return 'A';
	return hold_reason == HC_PLANE_CHANGED ? 'B' : 0;
}

struct hc_verdict {
	bool pass;		/* the hold passed (a redraw in phase A does not fail it) */
	enum hc_reason reason;
	uint32_t index;
	uint32_t rows_touched;
	bool redraw;
	char redraw_phase;	/* 'A', 'B' or 0 */
};

static inline struct hc_verdict hc_verdict_make(const struct hc_sample *s, uint32_t n,
						uint32_t hash_adopt, uint32_t hash_before_write)
{
	struct hc_hold h = hc_judge_hold(s, n);
	struct hc_verdict v;

	v.pass = h.reason == HC_PASS;
	v.reason = h.reason;
	v.index = h.index;
	v.rows_touched = h.rows_touched;
	v.redraw_phase = hc_redraw_phase(hash_adopt, hash_before_write, h.reason);
	v.redraw = v.redraw_phase != 0;
	return v;
}

#endif /* RPI5_SCANOUT16_HOLD_CHECK_H */
