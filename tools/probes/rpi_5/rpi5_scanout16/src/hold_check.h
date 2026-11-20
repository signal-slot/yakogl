/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: the long hold's decision logic (docs/notes/rpi5-display.md
 * section 4, Q2: does the firmware redraw into its plane or change its display
 * list while we hold without writing?). Pure functions, no I/O, no Zephyr:
 * stdint/stddef/stdbool only, host-tested by test/hold_check_test.c.
 *
 * Register layout is that of driver/zephyr/lib/board/rpi5_scanout/
 * scanout_alive.h (STATUS.FRCNT 21:16, PV_CONTROL.EN bit 0, PV_V_CONTROL.VIDEN
 * bit 0); the constants are repeated here so this header stands alone.
 * pattern.h has no hash function (only a checksum field that names FNV-1a),
 * so the FNV-1a 32-bit hash is defined here.
 */
#ifndef RPI5_SCANOUT16_HOLD_CHECK_H
#define RPI5_SCANOUT16_HOLD_CHECK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define HOLD_FNV_BASIS 0x811c9dc5u
#define HOLD_FNV_PRIME 0x01000193u

#define HOLD_FRCNT_SHIFT 16u /* scanout_alive.h:61 */
#define HOLD_FRCNT_MASK 0x3fu
#define HOLD_PV_CONTROL_EN (1u << 0)
#define HOLD_PV_VCONTROL_VIDEN (1u << 0)
#define HOLD_UNCLOCKED 0xffffffffu /* scanout_alive.h:19 */
#define HOLD_WDOG_MARGIN_MS 10000u

/* (a) FNV-1a 32-bit, continuing from h (start with HOLD_FNV_BASIS). */
static inline uint32_t hold_fnv1a_n(uint32_t h, const volatile uint8_t *p, size_t n)
{
	for (size_t i = 0; i < n; i++) {
		h ^= p[i];
		h *= HOLD_FNV_PRIME;
	}
	return h;
}

static inline uint32_t hold_fnv1a(const volatile void *buf, size_t n)
{
	return hold_fnv1a_n(HOLD_FNV_BASIS, (const volatile uint8_t *)buf, n);
}

/* The row index of the i-th of n rows spread evenly over h rows. */
static inline uint32_t hold_row_at(uint32_t i, uint32_t n, uint32_t h)
{
	return n ? (uint32_t)(((uint64_t)i * h) / n) : 0u;
}

/* (b) Hash of n whole rows (pitch bytes each) spread evenly over h rows. */
static inline uint32_t hold_rows_hash(const volatile void *plane, uint32_t pitch,
				      uint32_t h, uint32_t n)
{
	const volatile uint8_t *b = (const volatile uint8_t *)plane;
	uint32_t acc = HOLD_FNV_BASIS;

	if (n > h)
		n = h;
	for (uint32_t i = 0; i < n; i++)
		acc = hold_fnv1a_n(acc, b + (size_t)hold_row_at(i, n, h) * pitch, pitch);
	return acc;
}

/* (c) Mismatches of a plane against an expected-pixel callback. */
typedef uint32_t (*hold_expect_fn)(uint32_t x, uint32_t y, void *ctx);

struct hold_mismatch {
	uint32_t count;   /* pixels differing */
	uint32_t first_x; /* of the first (row-major); valid if count > 0 */
	uint32_t first_y;
	uint32_t rows;    /* distinct rows with at least one mismatch */
};

static inline uint32_t hold_load_px(const volatile uint8_t *p, uint32_t bpp)
{
	uint32_t v = 0;

	for (uint32_t i = 0; i < bpp; i++)
		v |= (uint32_t)p[i] << (8u * i);
	return v;
}

/* bpp is 2 or 4 bytes; pixels little-endian; bpp 2 compares the low 16 bits. */
static inline struct hold_mismatch hold_mismatch_scan(const volatile void *plane,
						      uint32_t pitch, uint32_t w, uint32_t h,
						      uint32_t bpp, hold_expect_fn expect,
						      void *ctx)
{
	const volatile uint8_t *b = (const volatile uint8_t *)plane;
	struct hold_mismatch m = { 0, 0, 0, 0 };

	for (uint32_t y = 0; y < h; y++) {
		bool hit = false;

		for (uint32_t x = 0; x < w; x++) {
			uint32_t got = hold_load_px(b + (size_t)y * pitch + (size_t)x * bpp, bpp);
			uint32_t want = expect(x, y, ctx);

			if (bpp < 4)
				want &= (1u << (8u * bpp)) - 1u;
			if (got == want)
				continue;
			if (m.count == 0) {
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

/* (d) FRCNT of a STATUS value. */
static inline uint32_t hold_frcnt(uint32_t status)
{
	return (status >> HOLD_FRCNT_SHIFT) & HOLD_FRCNT_MASK;
}

/* FRCNT must advance (mod 64) between every two samples. Returns -1 when it
 * does, else the index of the first sample equal to its predecessor's. */
static inline int hold_frcnt_stall(const uint32_t *status, unsigned int n)
{
	for (unsigned int i = 1; i < n; i++)
		if (hold_frcnt(status[i]) == hold_frcnt(status[i - 1]))
			return (int)i;
	return -1;
}

/* (e) An all-ones read is an unclocked or unpowered block. */
static inline bool hold_unclocked(uint32_t reg)
{
	return reg == HOLD_UNCLOCKED;
}

/* (f) PV enabled and its video enable set. */
static inline bool hold_pv_enabled(uint32_t pv_control, uint32_t pv_vcontrol)
{
	return (pv_control & HOLD_PV_CONTROL_EN) && (pv_vcontrol & HOLD_PV_VCONTROL_VIDEN);
}

/* (g) hold_end_ms = min(start + want, hold_s*1000 - 10000), the watchdog
 * deadline being counted from boot. *clamped is set when shorter than wanted.
 * A deadline at or before start gives start (no hold). */
static inline uint32_t hold_clamp_end(uint32_t start_ms, uint32_t want_ms, uint32_t hold_s,
				      bool *clamped)
{
	uint64_t want_end = (uint64_t)start_ms + want_ms;
	uint64_t wd = (uint64_t)hold_s * 1000u;
	uint64_t limit = wd > HOLD_WDOG_MARGIN_MS ? wd - HOLD_WDOG_MARGIN_MS : 0u;
	uint64_t end = want_end < limit ? want_end : limit;

	if (end < start_ms)
		end = start_ms;
	if (clamped)
		*clamped = end < want_end;
	return (uint32_t)end;
}

/* (h) The combined verdict. */
enum hold_reason {
	HOLD_PASS = 0,
	HOLD_UNCLOCKED_FAIL,  /* a register read all ones */
	HOLD_PV_DISABLED,     /* PV EN or VIDEN clear */
	HOLD_FRCNT_STALLED,   /* FRCNT did not advance */
	HOLD_POINTER_CHANGED, /* the channel's list pointer register moved */
	HOLD_LIST_CHANGED,    /* the display list's words changed */
	HOLD_PLANE_CHANGED,   /* the plane's bytes changed */
};

enum hold_redraw {
	HOLD_REDRAW_NO = 0,
	HOLD_REDRAW_A, /* the plane changed before our write */
	HOLD_REDRAW_B, /* the plane changed during the hold */
};

struct hold_sample {
	uint32_t status;       /* DISPx_STATUS */
	uint32_t dl_ptr;       /* the list pointer register read */
	uint32_t list_hash;    /* FNV-1a over the list words */
	uint32_t plane_hash;   /* hold_rows_hash of the plane */
	uint32_t rows_touched; /* rows differing from the reference, when plane_hash does */
	uint32_t pv_control;
	uint32_t pv_vcontrol;
};

struct hold_verdict {
	bool pass;
	enum hold_reason reason;
	int index;             /* failing sample, -1 for none */
	uint32_t rows_touched; /* for HOLD_PLANE_CHANGED */
	enum hold_redraw redraw;
};

static inline const char *hold_reason_name(enum hold_reason r)
{
	switch (r) {
	case HOLD_PASS:
		return "pass";
	case HOLD_UNCLOCKED_FAIL:
		return "unclocked";
	case HOLD_PV_DISABLED:
		return "pv-disabled";
	case HOLD_FRCNT_STALLED:
		return "frcnt-stalled";
	case HOLD_POINTER_CHANGED:
		return "pointer-changed";
	case HOLD_LIST_CHANGED:
		return "list-changed";
	case HOLD_PLANE_CHANGED:
		return "plane-changed";
	}
	return "?";
}

static inline const char *hold_redraw_name(enum hold_redraw r)
{
	return r == HOLD_REDRAW_A ? "yes-A" : r == HOLD_REDRAW_B ? "yes-B" : "no";
}

/* ref is the state right after our write; s[0..n) the hold's samples. Per
 * sample the checks run in the order unclocked, PV, FRCNT (against the
 * previous sample, the reference for sample 0), pointer, list, plane; the
 * first sample and check that fails name the verdict. before_write_changed:
 * the plane already differed from what the firmware left before our write
 * (redraw phase A). The redraw verdict is independent of the reason: A when
 * set, else B when any sample's plane hash differs from ref. */
static inline struct hold_verdict hold_judge(const struct hold_sample *ref,
					     const struct hold_sample *s, unsigned int n,
					     bool before_write_changed)
{
	struct hold_verdict v = { true, HOLD_PASS, -1, 0, HOLD_REDRAW_NO };
	bool plane_moved = false;

	for (unsigned int i = 0; i < n; i++) {
		const struct hold_sample *prev = i ? &s[i - 1] : ref;
		enum hold_reason r = HOLD_PASS;

		if (s[i].plane_hash != ref->plane_hash)
			plane_moved = true;
		if (!v.pass)
			continue;
		if (hold_unclocked(s[i].status) || hold_unclocked(s[i].pv_control) ||
		    hold_unclocked(s[i].pv_vcontrol) || hold_unclocked(s[i].dl_ptr))
			r = HOLD_UNCLOCKED_FAIL;
		else if (!hold_pv_enabled(s[i].pv_control, s[i].pv_vcontrol))
			r = HOLD_PV_DISABLED;
		else if (hold_frcnt(s[i].status) == hold_frcnt(prev->status))
			r = HOLD_FRCNT_STALLED;
		else if (s[i].dl_ptr != ref->dl_ptr)
			r = HOLD_POINTER_CHANGED;
		else if (s[i].list_hash != ref->list_hash)
			r = HOLD_LIST_CHANGED;
		else if (s[i].plane_hash != ref->plane_hash)
			r = HOLD_PLANE_CHANGED;
		if (r != HOLD_PASS) {
			v.pass = false;
			v.reason = r;
			v.index = (int)i;
			v.rows_touched = s[i].rows_touched;
		}
	}
	v.redraw = before_write_changed ? HOLD_REDRAW_A
		   : plane_moved       ? HOLD_REDRAW_B
				       : HOLD_REDRAW_NO;
	return v;
}

#endif /* RPI5_SCANOUT16_HOLD_CHECK_H */
