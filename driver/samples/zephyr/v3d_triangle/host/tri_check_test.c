/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of the v3d_triangle read-back check (src/tri_check.h): it must
 * pass a picture that is what the check describes and fail each kind of wrong
 * picture. The "correct" picture is made here from the same model (pixel
 * centres inside the triangle get the interpolated colour), so a pass says the
 * check is consistent with itself and can fail; it says nothing about what the
 * V3D draws.
 *
 *   cc -std=c11 -Wall -Wextra -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include \
 *      -Idriver/samples/zephyr/v3d_triangle/src \
 *      driver/samples/zephyr/v3d_triangle/host/tri_check_test.c \
 *      driver/src/v3d/v3d_frame.c -o tri_check_test
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tri_check.h"

#define W 1280
#define H 360
#define CLEAR 0xff202020u

static uint32_t img[H][W];

static void draw_model(const struct raster *r)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, l);
			img[y][x] = d >= 0 ? expected_rgba8(r, l) : CLEAR;
		}
}

/* The discard variant's picture, right or wrong. */
enum disc_fault { DISC_RIGHT, DISC_INVERTED, DISC_G_ONLY, DISC_B_ONLY };

static void draw_discard(const struct raster *r, enum disc_fault fault)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, l);
			bool g = interp_ch(l, 1) > 0.25f, b = interp_ch(l, 2) < 0.25f;
			bool gone = fault == DISC_G_ONLY ? g : fault == DISC_B_ONLY ? b : (g && b);
			if (fault == DISC_INVERTED)
				gone = !gone;
			img[y][x] = d < 0 || gone ? CLEAR : expected_rgba8(r, l);
		}
}

/* The gl_FragCoord variant's picture, right or with the window transform
 * wrong in one way. */
enum fc_fault { FC_RIGHT, FC_WRONG_FLIP, FC_NO_OFFSET, FC_X_FOR_Y };

static void draw_fragcoord(const struct raster *r, enum fc_fault fault)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, l);
			if (d < 0) {
				img[y][x] = CLEAR;
				continue;
			}
			float px = x + 0.5f, py = y + 0.5f;
			uint32_t v = fragcoord_colour(r, l, px, py, fault != FC_WRONG_FLIP);
			if (fault == FC_NO_OFFSET) /* scale -1, offset 0: y' <= 0 */
				v &= ~0x0000ff00u;
			if (fault == FC_X_FOR_Y)
				v = (v & ~0x0000ff00u) | (unorm8(px / TRI_FRAGCOORD_H) << 8);
			img[y][x] = v;
		}
}

/* The int variant's picture, right or with one operation wrong. */
enum int_fault { INT_RIGHT, INT_ROUNDED, INT_STALE_SELECT, INT_EQ_NE_SWAPPED, INT_NO_SIGN };

static void draw_int(const struct raster *r, enum int_fault fault)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, l);
			if (d < 0) {
				img[y][x] = CLEAR;
				continue;
			}
			int i, gs, b_low;
			int_sides(l, &i, &gs, &b_low);
			if (fault == INT_ROUNDED)
				i = (int)(interp_ch(l, 0) * 4.0f + 0.5f);
			if (fault == INT_STALE_SELECT)
				b_low = 0; /* the value before the conditional move: the else arm */
			if (fault == INT_NO_SIGN)
				gs = 0;
			uint32_t v = int_colour(i, gs, b_low);
			if (fault == INT_EQ_NE_SWAPPED) {
				float g = (i != 2 ? 1.0f : 0.0f) * 0.5f + (i == 1 ? 1.0f : 0.0f) * 0.25f + 0.125f;
				v = (v & ~0x0000ff00u) | (unorm8(g) << 8);
			}
			img[y][x] = v;
		}
}

/* A pixel whose class is `want_in` (1 inside, 0 outside), found by scanning. */
static void find(const struct raster *r, int want_in, uint32_t *px, uint32_t *py)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, l);
			if ((want_in && d >= EDGE_PX + 2) || (!want_in && d <= -EDGE_PX - 2)) {
				*px = x;
				*py = y;
				return;
			}
		}
	abort();
}

static int failures, cases;

static void expect(const char *name, bool want_ok, const struct raster *r)
{
	struct tri_result res;
	tri_check(r, (const uint8_t *)img, W * 4, W, H, CLEAR, &res);
	bool ok = tri_result_ok(&res);
	cases++;
	printf("%-34s %s: %u = %u in + %u out + %u edge, wrong %u/%u/%u, worst %u\n", name,
	       ok ? "pass" : "FAIL", res.total, res.n_in, res.n_out, res.n_edge, res.bad_in,
	       res.bad_out, res.bad_edge, res.worst_in);
	if (ok != want_ok) {
		printf("  ^ expected %s\n", want_ok ? "pass" : "FAIL");
		failures++;
	}
}

int main(void)
{
	struct v3d_viewport vp;
	v3d_viewport_gl(0, 0, W, H, 0.0f, 1.0f, true, H, &vp);
	struct raster r;
	raster_setup(&vp, &tri_identity, &r);
	uint32_t ix, iy, ox, oy;
	find(&r, 1, &ix, &iy);
	find(&r, 0, &ox, &oy);

	draw_model(&r);
	expect("the model picture", true, &r);

	draw_model(&r);
	img[iy][ix] ^= 0x00000010u; /* red off by 16 */
	expect("one inside pixel off by 16", false, &r);

	draw_model(&r);
	img[iy][ix] += 3; /* red +3: inside the tolerance */
	expect("one inside pixel off by 3 (tolerated)", true, &r);

	draw_model(&r);
	img[oy][ox] = 0xff202021u;
	expect("one outside pixel off by 1", false, &r);

	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++)
			img[y][x] = CLEAR;
	expect("nothing drawn (clear only)", false, &r);

	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++)
			img[y][x] = 0xffff00ffu;
	expect("nothing written (sentinel)", false, &r);

	/* the picture flipped upside down: what a wrong y orientation gives */
	draw_model(&r);
	for (uint32_t y = 0; y < H / 2; y++)
		for (uint32_t x = 0; x < W; x++) {
			uint32_t t = img[y][x];
			img[y][x] = img[H - 1 - y][x];
			img[H - 1 - y][x] = t;
		}
	expect("the picture upside down", false, &r);

	/* R and B swapped: what a wrong channel order gives */
	draw_model(&r);
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			uint32_t v = img[y][x];
			img[y][x] = (v & 0xff00ff00u) | ((v & 0xff) << 16) | ((v >> 16) & 0xff);
		}
	expect("R and B swapped", false, &r);

	/* The uniform variant: the check must pass the picture the uniforms
	 * make and fail the pictures a lost or misplaced uniform makes. */
	struct raster ru, wrong;
	raster_setup(&vp, &tri_uniform_values, &ru);
	draw_model(&ru);
	expect("uniforms: the model picture", true, &ru);

	draw_model(&r);
	expect("uniforms: none applied (identity)", false, &ru);

	struct tri_xform x = tri_uniform_values;
	x.offset[0] = x.offset[1] = 0.0f;
	raster_setup(&vp, &x, &wrong);
	draw_model(&wrong);
	expect("uniforms: VS offset read as 0", false, &ru);

	x = tri_uniform_values;
	for (int c = 0; c < 4; c++)
		x.bias[c] = 0.0f;
	raster_setup(&vp, &x, &wrong);
	draw_model(&wrong);
	expect("uniforms: FS bias read as 0", false, &ru);

	/* the FS storage read one word late: scale.yzw, bias.x, bias.yzw, then
	 * past the end (0) */
	x = tri_uniform_values;
	for (int c = 0; c < 4; c++) {
		float next[8];
		memcpy(next, tri_uniform_values.scale, sizeof(float) * 4);
		memcpy(next + 4, tri_uniform_values.bias, sizeof(float) * 4);
		x.scale[c] = next[c + 1];
		x.bias[c] = c + 5 < 8 ? next[c + 5] : 0.0f;
	}
	raster_setup(&vp, &x, &wrong);
	draw_model(&wrong);
	expect("uniforms: FS storage one word late", false, &ru);

	/* The if variant: the check must pass the picture the if makes, and
	 * fail a picture with no if, one with the arms swapped, and one where
	 * the then arm also wrote the else arm's lane (a lost "value from
	 * before the if"). */
	struct raster ri = r;
	ri.if_variant = true;
	draw_model(&ri);
	expect("if: the model picture", true, &ri);

	draw_model(&r);
	expect("if: no if (plain triangle)", false, &ri);

	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(&ri, x + 0.5f, y + 0.5f, l);
			img[y][x] = d >= 0 ? expected_side(&ri, l, !(interp_red(l) > 0.5f)) : CLEAR;
		}
	expect("if: the arms swapped", false, &ri);

	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			float l[3];
			float d = raster_at(&ri, x + 0.5f, y + 0.5f, l);
			uint32_t v = expected_rgba8(&ri, l);
			if (interp_red(l) > 0.5f) /* the then side, b forced to 0.25 too */
				v = (v & ~0x00ff0000u) | (unorm8(0.25f) << 16);
			img[y][x] = d >= 0 ? v : CLEAR;
		}
	expect("if: then arm lost b's old value", false, &ri);

	/* The int variant: the check must pass the picture the int shader makes,
	 * and fail each way its operations could go wrong. */
	struct raster rn = r;
	rn.int_variant = true;
	draw_int(&rn, INT_RIGHT);
	expect("int: the model picture", true, &rn);

	draw_model(&r);
	expect("int: no int shader (plain triangle)", false, &rn);

	draw_int(&rn, INT_ROUNDED);
	expect("int: int() rounds instead of truncating", false, &rn);

	draw_int(&rn, INT_STALE_SELECT);
	expect("int: the ternary read before its move", false, &rn);

	draw_int(&rn, INT_EQ_NE_SWAPPED);
	expect("int: i == 1 and i != 2 swapped", false, &rn);

	draw_int(&rn, INT_NO_SIGN);
	expect("int: sign() lost (0)", false, &rn);

	/* The discard variant: the check must pass the picture the discard makes
	 * and fail a picture with nothing discarded, the condition inverted, and
	 * only one of the two conditions applied. */
	struct raster rd = r;
	rd.discard_variant = true;
	rd.clear = CLEAR;
	draw_discard(&rd, DISC_RIGHT);
	expect("discard: the model picture", true, &rd);

	draw_model(&r);
	expect("discard: nothing discarded", false, &rd);

	draw_discard(&rd, DISC_INVERTED);
	expect("discard: the condition inverted", false, &rd);

	draw_discard(&rd, DISC_G_ONLY);
	expect("discard: g > 0.25 alone (the && lost)", false, &rd);

	draw_discard(&rd, DISC_B_ONLY);
	expect("discard: b < 0.25 alone (the && lost)", false, &rd);

	/* The gl_FragCoord variant: the check must pass the picture a right
	 * window transform makes, fail each wrong one, and say which flip the
	 * pixels match. */
	struct raster rf = r;
	rf.fragcoord_variant = true;
	rf.fb_h = (float)H;
	draw_fragcoord(&rf, FC_RIGHT);
	expect("fragcoord: the model picture", true, &rf);
	{
		struct tri_result res;
		tri_check(&rf, (const uint8_t *)img, W * 4, W, H, CLEAR, &res);
		cases++;
		bool ok = res.n_flip_right > 0 && res.n_flip_wrong == 0 && res.n_band > 0;
		printf("%-34s %s: right %u, wrong %u, band %u\n", "fragcoord: the counts", ok ? "pass" : "FAIL",
		       res.n_flip_right, res.n_flip_wrong, res.n_band);
		failures += !ok;
	}
	draw_fragcoord(&rf, FC_WRONG_FLIP);
	expect("fragcoord: the flip wrong", false, &rf);
	{
		struct tri_result res;
		tri_check(&rf, (const uint8_t *)img, W * 4, W, H, CLEAR, &res);
		cases++;
		bool ok = res.n_flip_wrong > 0 && res.n_flip_right == 0;
		printf("%-34s %s: right %u, wrong %u\n", "fragcoord: the wrong flip counted", ok ? "pass" : "FAIL",
		       res.n_flip_right, res.n_flip_wrong);
		failures += !ok;
	}
	draw_fragcoord(&rf, FC_NO_OFFSET);
	expect("fragcoord: the offset missing", false, &rf);
	draw_fragcoord(&rf, FC_X_FOR_Y);
	expect("fragcoord: x read for y", false, &rf);
	draw_model(&r);
	expect("fragcoord: the interpolated picture", false, &rf);

	printf("tri_check_test: %d cases run (planned 35), %d failed\n", cases, failures);
	return failures == 0 && cases == 35 ? 0 : 1;
}
