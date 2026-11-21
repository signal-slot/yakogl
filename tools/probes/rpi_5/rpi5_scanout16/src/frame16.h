/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: the frame's hardware-independent logic (docs/notes/
 * rpi5-display.md section 3): fbcon 16 bpp text rows, the RGB565 pattern
 * band, the byte-equal readback and the formatters of the console lines
 * [4]..[11] that verdicts.sh parses. C11, static inline, no Zephyr, no I/O;
 * host-tested by test/frame16_test.c and frame16_selftest.sh (which pipes
 * the formatted lines through verdicts.sh).
 *
 * Layout: the text band is rows 0 .. n*16-1 (scale 1, 16 pixel rows per text
 * row); the pattern band is pattern.h's split_layout() bottom band. The text
 * must end at or above the pattern band's first row (frame16_text_max_rows).
 *
 * Readback choice: BYTE-EQUAL against an expected image of the same
 * geometry (the caller renders the same frame into a second buffer with
 * frame16_render). Only the w*2 visible bytes of each row are compared; the
 * pitch padding is never written by us and is ignored. The result counts
 * differing bytes and names the first one (row-major) as x (pixel), y and
 * the byte offset. No hash and no pattern_check grid: a hash cannot name a
 * mismatch, and a grid can miss a corrupted pixel.
 */
#ifndef RPI5_SCANOUT16_FRAME16_H
#define RPI5_SCANOUT16_FRAME16_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h> /* snprintf: the formatters only */

#include "../../../../../driver/zephyr/lib/board/rpi5_scanout/fbcon565.h"
#include "../../../../../driver/zephyr/lib/board/rpi5_scanout/pattern.h"
#include "hold_check.h"

#define FRAME16_TEXT_SCALE 1u
#define FRAME16_TEXT_ROW_PX (GC_FONT_H * FRAME16_TEXT_SCALE)
#define FRAME16_FG 0xffffu /* white */
#define FRAME16_BG 0x0000u

#define FRAME16_E_PLANE (-100) /* not an adopted RGB565 plane */
#define FRAME16_E_FIT (-101)   /* the text rows do not fit above the pattern */

/* An adopted plane: base, pitch in bytes, size in pixels, format. */
struct frame16_plane {
	void *base;
	uint32_t pitch;
	uint32_t w, h;
	enum pattern_fmt fmt;
};

static inline bool frame16_plane_ok(const struct frame16_plane *p)
{
	return p && p->base && p->fmt == PATTERN_RGB565 && p->w > 0 && p->h > 0 &&
	       p->pitch >= p->w * 2u && !(p->pitch & 1u);
}

static inline struct yd5_fc565 frame16_fc(const struct frame16_plane *p)
{
	struct yd5_fc565 f = { (uint16_t *)p->base, p->w, p->pitch, p->h };

	return f;
}

/* Number of text rows that fit above the pattern band (0 for a bad plane). */
static inline uint32_t frame16_text_max_rows(const struct frame16_plane *p)
{
	if (!frame16_plane_ok(p))
		return 0;
	return split_layout(p->w, p->h).pattern.y / FRAME16_TEXT_ROW_PX;
}

/* The text of row i: a fixed, row-unique string. */
static inline void frame16_text_line(uint32_t i, char *buf, size_t n)
{
	snprintf(buf, n, "rpi5_scanout16 text row %02u: the quick brown fox 0123456789",
		 (unsigned int)i);
}

/* Hash of the visible bytes of the pixel rows y0 .. y0+h-1. */
static inline uint32_t frame16_band_hash(const struct frame16_plane *p, uint32_t y0, uint32_t h)
{
	uint32_t acc = HOLD_FNV_BASIS;

	for (uint32_t y = y0; y < y0 + h && y < p->h; y++)
		acc = hold_fnv1a_n(acc, (const volatile uint8_t *)p->base + (size_t)y * p->pitch,
				   (size_t)p->w * 2u);
	return acc;
}

/*
 * (a) Draw n text rows; *changed receives the number of rows whose plane
 * bytes differ after the draw (line [4]). Returns 0, a YD5_FC565_E_* code,
 * FRAME16_E_PLANE or FRAME16_E_FIT. *changed is 0 on any error.
 */
static inline int frame16_draw_text(const struct frame16_plane *p, uint32_t n, uint32_t *changed)
{
	uint32_t hits = 0;
	struct yd5_fc565 f;

	if (changed)
		*changed = 0;
	if (!frame16_plane_ok(p))
		return FRAME16_E_PLANE;
	if (n > frame16_text_max_rows(p))
		return FRAME16_E_FIT;
	f = frame16_fc(p);
	for (uint32_t i = 0; i < n; i++) {
		char line[96];
		uint32_t y0 = i * FRAME16_TEXT_ROW_PX;
		uint32_t before = frame16_band_hash(p, y0, FRAME16_TEXT_ROW_PX);
		int e;

		frame16_text_line(i, line, sizeof(line));
		e = yd5_fc565_puts(&f, y0, FRAME16_TEXT_ROW_PX, 0, 0, line, FRAME16_TEXT_SCALE,
				   FRAME16_FG, FRAME16_BG, NULL);
		if (e)
			return e;
		hits += frame16_band_hash(p, y0, FRAME16_TEXT_ROW_PX) != before;
	}
	if (changed)
		*changed = hits;
	return 0;
}

/*
 * (b) Draw the pattern in the bottom band of split_layout(), below every
 * text row. Returns true when drawn; else false with *why a static reason.
 * A plane that is not RGB565, or smaller than 64x64, is not drawn.
 */
static inline bool frame16_draw_pattern(const struct frame16_plane *p, const char **why)
{
	const char *r = NULL;

	if (!p || !p->base)
		r = "no plane";
	else if (p->fmt != PATTERN_RGB565)
		r = "plane is not RGB565";
	else if (p->w < 64 || p->h < 64)
		r = "plane too small";
	else if (p->pitch < p->w * 2u || (p->pitch & 1u))
		r = "pitch too small";
	if (!r) {
		struct pattern_layout l = split_layout(p->w, p->h);

		if (pattern_fill_rect(p->base, p->w, p->h, p->pitch, 16, PATTERN_RGB565,
				      l.pattern) != 0)
			r = "pattern_fill_rect refused";
	}
	if (why)
		*why = r;
	return r == NULL;
}

/* The whole frame: text rows, then the pattern (attempted even when the text
 * failed). Used for the expected image too. Returns draw_text's code. */
static inline int frame16_render(const struct frame16_plane *p, uint32_t text_rows,
				 uint32_t *changed, bool *drawn, const char **why)
{
	int e = frame16_draw_text(p, text_rows, changed);
	bool d = frame16_draw_pattern(p, why);

	if (drawn)
		*drawn = d;
	return e;
}

/* (c) Byte-equal readback of the whole plane (visible bytes) against expected. */
struct frame16_rb {
	bool pass;
	uint32_t bytes;            /* bytes compared */
	uint32_t mismatch;         /* differing bytes */
	uint32_t first_x, first_y; /* of the first: pixel x, row y */
	uint32_t first_off;        /* its byte offset in the row */
	uint8_t want, got;         /* its two values */
	bool bad_arg;              /* geometry differs, or a plane is not RGB565 */
};

static inline struct frame16_rb frame16_readback(const struct frame16_plane *p,
						 const struct frame16_plane *exp)
{
	struct frame16_rb r = { false, 0, 0, 0, 0, 0, 0, 0, false };

	if (!frame16_plane_ok(p) || !frame16_plane_ok(exp) || p->w != exp->w || p->h != exp->h) {
		r.bad_arg = true;
		return r;
	}
	for (uint32_t y = 0; y < p->h; y++) {
		const volatile uint8_t *a = (const volatile uint8_t *)p->base + (size_t)y * p->pitch;
		const volatile uint8_t *b =
			(const volatile uint8_t *)exp->base + (size_t)y * exp->pitch;

		for (uint32_t i = 0; i < p->w * 2u; i++) {
			uint8_t g = a[i], w = b[i];

			if (g == w)
				continue;
			if (r.mismatch == 0) {
				r.first_x = i / 2u;
				r.first_y = y;
				r.first_off = i;
				r.want = w;
				r.got = g;
			}
			r.mismatch++;
		}
	}
	r.bytes = p->w * 2u * p->h;
	r.pass = r.mismatch == 0;
	return r;
}

/* ---- formatters: each writes the payload of one console line (no newline)
 * and returns snprintf's value. ---- */

/* [4] */
static inline int frame16_fmt_text_rows(char *b, size_t n, uint32_t changed)
{
	return snprintf(b, n, "rpi5_scanout16: text rows %u changed", (unsigned int)changed);
}

/* [5] */
static inline int frame16_fmt_pattern(char *b, size_t n, bool drawn, const char *why)
{
	if (drawn)
		return snprintf(b, n,
				"rpi5_scanout16: pattern drawn (8 colour bars below the text)");
	return snprintf(b, n, "rpi5_scanout16: pattern not drawn (%s)", why ? why : "unknown");
}

/* [6] (which = 1) and [7] (which = 2) */
static inline int frame16_fmt_readback(char *b, size_t n, int which, const struct frame16_rb *r)
{
	if (r->bad_arg)
		return snprintf(b, n,
				"u3 readback %d: FAIL (not comparable: plane or expected image invalid)",
				which);
	if (r->pass)
		return snprintf(b, n, "u3 readback %d: PASS (%u bytes read back byte-equal)", which,
				(unsigned int)r->bytes);
	return snprintf(b, n,
			"u3 readback %d: FAIL (%u of %u bytes differ, first at x=%u y=%u byte %u: want 0x%02x got 0x%02x)",
			which, (unsigned int)r->mismatch, (unsigned int)r->bytes,
			(unsigned int)r->first_x, (unsigned int)r->first_y,
			(unsigned int)r->first_off, r->want, r->got);
}

/* [8]: FRCNT values (0..63; the caller passes hold_frcnt() of STATUS). */
static inline int frame16_fmt_frcnt(char *b, size_t n, uint32_t before, uint32_t after)
{
	return snprintf(b, n, "u3 frcnt: before %u after %u", (unsigned int)before,
			(unsigned int)after);
}

/* [9] */
static inline int frame16_fmt_pv(char *b, size_t n, uint32_t pv_control, uint32_t pv_vcontrol)
{
	return snprintf(b, n, "u3 pv: control=0x%x en=%u viden=%u", (unsigned int)pv_control,
			(unsigned int)(pv_control & HOLD_PV_CONTROL_EN),
			(unsigned int)(pv_vcontrol & HOLD_PV_VCONTROL_VIDEN));
}

/* [10]: from hold_judge's verdict. plane=redrawn when its redraw is not NO or
 * its reason is plane-changed; list=changed when the reason is list-changed
 * or pointer-changed. bytes and words are the caller's counts (printed as 0
 * when kept). */
static inline int frame16_fmt_hold(char *b, size_t n, uint32_t ms, const struct hold_verdict *v,
				   uint32_t bytes, uint32_t words)
{
	bool redrawn = v->redraw != HOLD_REDRAW_NO || v->reason == HOLD_PLANE_CHANGED;
	bool changed = v->reason == HOLD_LIST_CHANGED || v->reason == HOLD_POINTER_CHANGED;

	return snprintf(b, n, "u3 hold: %u ms plane=%s bytes=%u list=%s words=%u", (unsigned int)ms,
			redrawn ? "redrawn" : "kept", (unsigned int)(redrawn ? bytes : 0),
			changed ? "changed" : "kept", (unsigned int)(changed ? words : 0));
}

/* Q2 list-word lines. entry_off < 0 prints 'none'. */
static inline int frame16_fmt_q2_word(char *b, size_t n, uint32_t index, uint32_t head, int entry_off,
				      uint32_t was, uint32_t now)
{
	if (entry_off < 0)
		return snprintf(b, n, "rpi5_scanout16: q2 word %u off %u entry-off none was 0x%08x "
				"now 0x%08x", (unsigned int)index, (unsigned int)(index - head),
				(unsigned int)was, (unsigned int)now);
	return snprintf(b, n, "rpi5_scanout16: q2 word %u off %u entry-off %d was 0x%08x now 0x%08x",
			(unsigned int)index, (unsigned int)(index - head), entry_off,
			(unsigned int)was, (unsigned int)now);
}

static inline int frame16_fmt_q2_more(char *b, size_t n, uint32_t left)
{
	return snprintf(b, n, "rpi5_scanout16: q2 %u more changed words left out", (unsigned int)left);
}

/* The entry the offsets refer to: its length and the POS3 context offset. */
static inline int frame16_fmt_q2_entry(char *b, size_t n, uint32_t nwords, uint32_t ctx_off)
{
	return snprintf(b, n, "rpi5_scanout16: q2 entry words %u ctx-off %u", (unsigned int)nwords,
			(unsigned int)ctx_off);
}

/* u3 q2 summary: changed (word given) or none. */
static inline int frame16_fmt_q2(char *b, size_t n, bool changed, uint32_t index, int entry_off,
				 uint32_t was, uint32_t now, bool per_frame)
{
	if (!changed)
		return snprintf(b, n, "u3 q2: no list word changed");
	if (entry_off < 0)
		return snprintf(b, n, "u3 q2: word %u entry-off none was 0x%08x now 0x%08x per-frame %s",
				(unsigned int)index, (unsigned int)was, (unsigned int)now,
				per_frame ? "yes" : "no");
	return snprintf(b, n, "u3 q2: word %u entry-off %d was 0x%08x now 0x%08x per-frame %s",
			(unsigned int)index, entry_off, (unsigned int)was, (unsigned int)now,
			per_frame ? "yes" : "no");
}

/* [11]: a result line; reason is free text (PASS: what held; FAIL: why). */
static inline int frame16_fmt_terminal(char *b, size_t n, bool pass, const char *reason)
{
	return snprintf(b, n, "rpi5_scanout16: %s (%s)", pass ? "PASS" : "FAIL",
			reason ? reason : "");
}

/* The planned watchdog ending, printed after [11]; not a verdict line. */
static inline int frame16_fmt_feeding_stopped(char *b, size_t n)
{
	return snprintf(b, n, "rpi5_scanout16: feeding stopped, reset expected");
}

#endif /* RPI5_SCANOUT16_FRAME16_H */
