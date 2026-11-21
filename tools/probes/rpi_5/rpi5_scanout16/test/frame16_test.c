/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of ../src/frame16.h. Usage: frame16_test [outdir]; writes the
 * synthetic console logs good.log, readback_fail.log and missing.log to
 * outdir (default build/rpi5_scanout16), built only from the formatters
 * except lines [1]-[3] (literals of verdicts_selftest.sh). Prints
 * "frame16_test: N checks, F failed".
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../src/frame16.h"

static int checks, failed;
#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

#define W 1920u
#define H 1080u
#define PITCH 3840u
#define TEXT_ROWS 24u

static struct frame16_plane mk(void *base, enum pattern_fmt fmt)
{
	struct frame16_plane p = { base, PITCH, W, H, fmt };

	return p;
}

static void fill_junk(uint8_t *b, size_t n)
{
	for (size_t i = 0; i < n; i++)
		b[i] = (uint8_t)(i * 7u + 3u);
}

static int has(const char *s, const char *sub)
{
	return strstr(s, sub) != NULL;
}

static void emit(FILE *f, const char *line)
{
	fprintf(f, "%s\n", line);
}

/* A log: lines [1]..[3] literal, [4]..[11] given; NULL skips a line. */
static void write_log(const char *path, const char *l[8], const char *rb2_override)
{
	FILE *f = fopen(path, "w");

	if (!f) {
		perror(path);
		exit(2);
	}
	emit(f, "--- Zephyr rpi5_scanout16 on BCM2712 (frame16_test) ---");
	emit(f, "u3 scanout: running");
	emit(f, "u3 plane: addr=0x3c000000 fmt=RGB565 pitch=3840 w=1920 h=1080");
	emit(f, "u3 adopt-readback: PASS (32 KiB read back byte-equal)");
	for (int i = 0; i < 8; i++) {
		const char *s = (i == 3 && rb2_override) ? rb2_override : l[i];

		if (s)
			emit(f, s);
	}
	emit(f, "rpi5_scanout16: feeding stopped, reset expected");
	fclose(f);
}

int main(int argc, char **argv)
{
	const char *outdir = argc > 1 ? argv[1] : "build/rpi5_scanout16";
	size_t sz = (size_t)PITCH * H;
	uint8_t *plane = malloc(sz), *exp = malloc(sz);
	char path[512], l4[160], l5[160], l6[256], l7[256], l8[96], l9[96], l10[160], l11[160];
	uint32_t changed = 0, e_changed = 0;
	const char *why = NULL;
	bool drawn = false;
	struct frame16_plane P, E;
	struct frame16_rb rb;
	int e;

	if (!plane || !exp)
		return 2;
	P = mk(plane, PATTERN_RGB565);
	E = mk(exp, PATTERN_RGB565);

	/* geometry */
	CHECK(frame16_plane_ok(&P));
	CHECK(frame16_text_max_rows(&P) == 33); /* split_layout: 528 / 16 */
	CHECK(split_layout(W, H).pattern.y == 528);

	/* (a) text rows into a junk plane: every row changes bytes */
	fill_junk(plane, sz);
	e = frame16_draw_text(&P, TEXT_ROWS, &changed);
	CHECK(e == 0);
	CHECK(changed == TEXT_ROWS);
	/* drawing the same text again changes nothing */
	e = frame16_draw_text(&P, TEXT_ROWS, &changed);
	CHECK(e == 0 && changed == 0);
	/* into an all-black plane, a row of spaces-only would not change; ours has glyphs */
	memset(plane, 0, sz);
	e = frame16_draw_text(&P, 5, &changed);
	CHECK(e == 0 && changed == 5);
	/* too many rows for the layout, and a bad plane */
	e = frame16_draw_text(&P, 34, &changed);
	CHECK(e == FRAME16_E_FIT && changed == 0);
	{
		struct frame16_plane q = mk(plane, PATTERN_XRGB8888);

		e = frame16_draw_text(&q, 1, &changed);
		CHECK(e == FRAME16_E_PLANE && changed == 0);
		q = mk(NULL, PATTERN_RGB565);
		CHECK(frame16_draw_text(&q, 1, &changed) == FRAME16_E_PLANE);
	}
	CHECK(frame16_text_max_rows(NULL) == 0);

	/* text stays in its rows: the first pattern row is untouched by it */
	memset(plane, 0, sz);
	CHECK(frame16_draw_text(&P, TEXT_ROWS, &changed) == 0);
	{
		int clean = 1;

		for (size_t i = (size_t)TEXT_ROWS * 16u * PITCH; i < sz; i++)
			if (plane[i])
				clean = 0;
		CHECK(clean);
	}

	/* (b) pattern */
	memset(plane, 0, sz);
	CHECK(frame16_draw_pattern(&P, &why) && why == NULL);
	{
		struct pattern_report rep;
		struct pattern_rect pat = split_layout(W, H).pattern;
		struct pattern_rect top = { 0, 0, W, pat.y };

		CHECK(pattern_check_rect(plane, W, H, PITCH, 16, PATTERN_RGB565, pat, 1, &rep) == 0);
		CHECK(rep.checked == W * pat.h);
		for (size_t i = 0; i < (size_t)pat.y * PITCH; i++)
			if (plane[i]) {
				CHECK(0 && "pattern leaked above its band");
				break;
			}
		(void)top;
	}
	{
		struct frame16_plane q = mk(plane, PATTERN_XRGB8888);

		CHECK(!frame16_draw_pattern(&q, &why));
		CHECK(why && has(why, "not RGB565"));
		q = mk(plane, PATTERN_RGB565);
		q.w = 32;
		q.h = 32;
		CHECK(!frame16_draw_pattern(&q, &why));
		CHECK(why && has(why, "too small"));
		q = mk(NULL, PATTERN_RGB565);
		CHECK(!frame16_draw_pattern(&q, &why));
		CHECK(!frame16_draw_pattern(NULL, &why));
	}

	/* the frame as one call, plane and expected image */
	fill_junk(plane, sz);
	memcpy(exp, plane, sz); /* expected = the plane before the draw, then the same frame */
	e = frame16_render(&P, TEXT_ROWS, &changed, &drawn, &why);
	CHECK(e == 0 && drawn && changed == TEXT_ROWS);
	e = frame16_render(&E, TEXT_ROWS, &e_changed, &drawn, &why);
	CHECK(e == 0 && drawn);

	/* (c) readback: the plane has junk in the pitch padding? no: pitch == 2*w;
	 * junk in nothing visible means equal. */
	rb = frame16_readback(&P, &E);
	CHECK(rb.pass && rb.mismatch == 0 && !rb.bad_arg && rb.bytes == W * 2u * H);
	frame16_fmt_readback(l6, sizeof(l6), 1, &rb);
	CHECK(strcmp(l6, "u3 readback 1: PASS (4147200 bytes read back byte-equal)") == 0);

	/* one corrupted pixel (one low byte) */
	plane[(size_t)700 * PITCH + 101 * 2] ^= 0x04;
	rb = frame16_readback(&P, &E);
	CHECK(!rb.pass && rb.mismatch == 1 && rb.first_x == 101 && rb.first_y == 700 &&
	      rb.first_off == 202 && rb.got == (uint8_t)(rb.want ^ 0x04));
	frame16_fmt_readback(l7, sizeof(l7), 2, &rb);
	CHECK(has(l7, "u3 readback 2: FAIL (1 of 4147200 bytes differ, first at x=101 y=700 byte 202: want 0x"));
	/* a second corruption earlier in the row-major order moves "first" */
	plane[(size_t)3 * PITCH + 1] ^= 0xff;
	rb = frame16_readback(&P, &E);
	CHECK(rb.mismatch == 2 && rb.first_y == 3 && rb.first_x == 0 && rb.first_off == 1);
	plane[(size_t)3 * PITCH + 1] ^= 0xff;
	plane[(size_t)700 * PITCH + 101 * 2] ^= 0x04;
	CHECK(frame16_readback(&P, &E).pass);
	/* pitch padding is ignored: use a wider pitch for the plane only */
	{
		struct frame16_plane q = P;
		uint8_t *pad = malloc((size_t)(PITCH + 64) * H);
		struct frame16_plane PP;

		CHECK(pad != NULL);
		PP = (struct frame16_plane){ pad, PITCH + 64, W, H, PATTERN_RGB565 };
		memset(pad, 0xa5, (size_t)(PITCH + 64) * H);
		for (uint32_t y = 0; y < H; y++)
			memcpy(pad + (size_t)y * (PITCH + 64), exp + (size_t)y * PITCH, PITCH);
		/* exp is the rendered image: re-render over the same base so only the draw differs */
		CHECK(frame16_render(&PP, TEXT_ROWS, &changed, &drawn, &why) == 0);
		CHECK(frame16_readback(&PP, &E).pass);
		(void)q;
		free(pad);
	}
	/* not comparable */
	{
		struct frame16_plane q = E;

		q.h = H - 1;
		CHECK(frame16_readback(&P, &q).bad_arg);
		q = E;
		q.fmt = PATTERN_XRGB8888;
		rb = frame16_readback(&P, &q);
		CHECK(rb.bad_arg && !rb.pass);
		frame16_fmt_readback(l7, sizeof(l7), 1, &rb);
		CHECK(has(l7, "u3 readback 1: FAIL (not comparable"));
	}

	/* (d) formatters */
	frame16_fmt_text_rows(l4, sizeof(l4), TEXT_ROWS);
	CHECK(strcmp(l4, "rpi5_scanout16: text rows 24 changed") == 0);
	frame16_fmt_pattern(l5, sizeof(l5), true, NULL);
	CHECK(strcmp(l5, "rpi5_scanout16: pattern drawn (8 colour bars below the text)") == 0);
	frame16_fmt_pattern(l5, sizeof(l5), false, "plane is not RGB565");
	CHECK(strcmp(l5, "rpi5_scanout16: pattern not drawn (plane is not RGB565)") == 0);
	frame16_fmt_frcnt(l8, sizeof(l8), 17, 21);
	CHECK(strcmp(l8, "u3 frcnt: before 17 after 21") == 0);
	frame16_fmt_pv(l9, sizeof(l9), 0x00000a01, 0x1);
	CHECK(strcmp(l9, "u3 pv: control=0xa01 en=1 viden=1") == 0);
	frame16_fmt_pv(l9, sizeof(l9), 0x0, 0x0);
	CHECK(strcmp(l9, "u3 pv: control=0x0 en=0 viden=0") == 0);
	frame16_fmt_terminal(l11, sizeof(l11), true, "x");
	CHECK(strcmp(l11, "rpi5_scanout16: PASS (x)") == 0);
	frame16_fmt_terminal(l11, sizeof(l11), false, "readback 2 FAIL");
	CHECK(strcmp(l11, "rpi5_scanout16: FAIL (readback 2 FAIL)") == 0);

	/* hold line from hold_judge's verdicts */
	{
		struct hold_sample ref = { 0x00010000, 0x100, 1, 2, 0, 1, 1 }, s[2];
		struct hold_verdict v;

		s[0] = ref;
		s[0].status = 0x00020000;
		s[1] = ref;
		s[1].status = 0x00030000;
		v = hold_judge(&ref, s, 2, false);
		CHECK(v.pass);
		frame16_fmt_hold(l10, sizeof(l10), 60000, &v, 99, 99);
		CHECK(strcmp(l10, "u3 hold: 60000 ms plane=kept bytes=0 list=kept words=0") == 0);
		s[1].plane_hash = 7;
		s[1].rows_touched = 3;
		v = hold_judge(&ref, s, 2, false);
		frame16_fmt_hold(l10, sizeof(l10), 60000, &v, 5760, 99);
		CHECK(strcmp(l10, "u3 hold: 60000 ms plane=redrawn bytes=5760 list=kept words=0") == 0);
		s[1].plane_hash = 2;
		s[1].list_hash = 9;
		v = hold_judge(&ref, s, 2, false);
		frame16_fmt_hold(l10, sizeof(l10), 30000, &v, 99, 4);
		CHECK(strcmp(l10, "u3 hold: 30000 ms plane=kept bytes=0 list=changed words=4") == 0);
		s[1].dl_ptr = 0x200;
		v = hold_judge(&ref, s, 2, false);
		CHECK(v.reason == HOLD_POINTER_CHANGED);
		frame16_fmt_hold(l10, sizeof(l10), 30000, &v, 99, 1);
		CHECK(has(l10, "list=changed words=1"));
		/* redrawn in phase A (before our write), reason otherwise pass */
		s[1] = ref;
		s[1].status = 0x00030000;
		v = hold_judge(&ref, s, 2, true);
		frame16_fmt_hold(l10, sizeof(l10), 60000, &v, 12, 0);
		CHECK(strcmp(l10, "u3 hold: 60000 ms plane=redrawn bytes=12 list=kept words=0") == 0);
		v = hold_judge(&ref, s, 2, false);
		frame16_fmt_hold(l10, sizeof(l10), 60000, &v, 0, 0);
	}

	/* the synthetic logs: a good run, a readback FAIL, a missing line */
	{
		struct hold_sample ref = { 0x00010000, 0x100, 1, 2, 0, 1, 1 }, s[1];
		struct hold_verdict v;
		struct frame16_rb ok, bad;
		const char *lines[8];
		char r1[256], r2[256], rbad[256], t_ok[160], t_bad[160];
		char f9[96];

		s[0] = ref;
		s[0].status = 0x00020000;
		v = hold_judge(&ref, s, 1, false);
		fill_junk(plane, sz);
		memcpy(exp, plane, sz);
		CHECK(frame16_render(&E, TEXT_ROWS, &e_changed, &drawn, &why) == 0);
		CHECK(frame16_render(&P, TEXT_ROWS, &changed, &drawn, &why) == 0);
		ok = frame16_readback(&P, &E);
		CHECK(ok.pass);
		plane[(size_t)900 * PITCH + 5] ^= 1;
		bad = frame16_readback(&P, &E);
		CHECK(!bad.pass);
		plane[(size_t)900 * PITCH + 5] ^= 1;

		frame16_fmt_text_rows(l4, sizeof(l4), changed);
		frame16_fmt_pattern(l5, sizeof(l5), drawn, why);
		frame16_fmt_readback(r1, sizeof(r1), 1, &ok);
		frame16_fmt_readback(r2, sizeof(r2), 2, &ok);
		frame16_fmt_readback(rbad, sizeof(rbad), 2, &bad);
		frame16_fmt_frcnt(l8, sizeof(l8), hold_frcnt(ref.status), hold_frcnt(s[0].status));
		frame16_fmt_pv(f9, sizeof(f9), 0x1, 0x1);
		frame16_fmt_hold(l10, sizeof(l10), 60000, &v, 0, 0);
		frame16_fmt_terminal(t_ok, sizeof(t_ok), true, "plane adopted, both read-backs match");
		frame16_fmt_terminal(t_bad, sizeof(t_bad), false, "readback 2 FAIL");

		lines[0] = l4;
		lines[1] = l5;
		lines[2] = r1;
		lines[3] = r2;
		lines[4] = l8;
		lines[5] = f9;
		lines[6] = l10;
		lines[7] = t_ok;
		snprintf(path, sizeof(path), "%s/good.log", outdir);
		write_log(path, lines, NULL);
		lines[7] = t_bad;
		snprintf(path, sizeof(path), "%s/readback_fail.log", outdir);
		write_log(path, lines, rbad);
		lines[7] = t_ok;
		lines[4] = NULL; /* the frcnt line is missing */
		snprintf(path, sizeof(path), "%s/missing.log", outdir);
		write_log(path, lines, NULL);
	}

	free(plane);
	free(exp);
	printf("frame16_test: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
