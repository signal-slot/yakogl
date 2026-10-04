/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_narrow_case.h: GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA sampled
 * narrow on the V3D, as data and checks a sample includes (gl_test 135
 * "texnarrow"; docs/online-shaderc/v3d-backend.md, "Narrow L8, A8 and LA8").
 * tools/v3d/tex_filter_ref_check.c checks it on the host.
 *
 * Why: the V3D keeps these formats at their own size (1 byte a texel for L8
 * and A8, 2 for LA8) and samples them as the TMU's R8 and RG8 with the
 * swizzles XXX1, 000X and XXXY, as Mesa does (v3dx_format_table.c:114-117).
 * The tiled layout depends on the texel size (a utile is 8x8 texels at 1
 * byte, 8x4 at 2, 4x4 at 4), so every tiling mode at 1 and 2 bytes is a new
 * path, and so are the GL core's narrow level offsets, the odd pitch and a
 * cube's layer stride. gl_test 132 (narrowla) samples 4x4 textures and one
 * small chain; this case reaches every level of a 64x64 chain (UIF down to
 * LT), a 13x7 level, and every face of a 16x16 cube at two levels.
 *
 * Per format (L8, A8, LA8), 20 draws, NEAREST filtering, CLAMP_TO_EDGE:
 *   0..6   a 64x64 chain of 7 levels, NEAREST_MIPMAP_NEAREST, level L drawn
 *          into a (64 >> L)-square viewport: one texel a pixel, LOD L
 *          exactly, so each pixel reads texel (x, y) of level L;
 *   7      a 13x7 texture (one level), drawn 13x7: texel (x, y);
 *   8..19  a 16x16 cube of 5 levels, NEAREST_MIPMAP_NEAREST, per face a quad
 *          whose direction is major + x * sx + y * ty (tex_cube_case.h's face
 *          table, x and y -1..1 over the viewport), drawn 16x16 (level 0) and
 *          4x4 (level 2): each pixel centre projects (dEQP's selectCubeFace
 *          and projectToFace) onto a texel centre of that level.
 * Every pixel of every draw is judged: it must be the texel's value as the
 * format samples, (L,L,L,1), (0,0,0,A) or (L,L,L,A) (ES 2.0 table 3.12),
 * within 1 step. The texel bytes are a hash of (texture, layer, level, x, y,
 * byte), so a texel read from anywhere else fails nearly every pixel;
 * tnc_controls() measures that for the wrong reads a tiling or layout defect
 * makes, before any draw.
 *
 * Header only, no OS calls and no libm.
 */
#ifndef TEX_NARROW_CASE_H
#define TEX_NARROW_CASE_H

#include <stdbool.h>
#include <stdint.h>

#include "tex_cube_case.h" /* tcc_faces; tex_filter_ref.h's cube face rules */

#define TNC_FORMATS 3
#define TNC_CHAIN 64 /* the 2D chain's level 0 */
#define TNC_CHAIN_LEVELS 7
#define TNC_ODD_W 13
#define TNC_ODD_H 7
#define TNC_CUBE 16 /* the cube's faces */
#define TNC_CUBE_LEVELS 5
#define TNC_DRAWS 20 /* per format */
#define TNC_TOL 1    /* steps a channel may be off */

enum tnc_format { TNC_L8, TNC_A8, TNC_LA8 };
enum tnc_kind { TNC_KIND_CHAIN, TNC_KIND_ODD, TNC_KIND_CUBE };

static const char *const tnc_format_names[TNC_FORMATS] = { "L8", "A8", "LA8" };

struct tnc_draw {
	enum tnc_kind kind;
	int face;           /* the cube's face drawn (GL order), 0 otherwise */
	int level;          /* the level every pixel reads */
	int size_w, size_h; /* the viewport */
};

static inline unsigned tnc_bpp(enum tnc_format f)
{
	return f == TNC_LA8 ? 2u : 1u;
}

/* draw i of a format (the table above) */
static inline struct tnc_draw tnc_draw_of(int i)
{
	struct tnc_draw d = { TNC_KIND_CHAIN, 0, 0, 0, 0 };
	if (i < TNC_CHAIN_LEVELS) {
		d.level = i;
		d.size_w = d.size_h = TNC_CHAIN >> i;
	} else if (i == TNC_CHAIN_LEVELS) {
		d.kind = TNC_KIND_ODD;
		d.size_w = TNC_ODD_W;
		d.size_h = TNC_ODD_H;
	} else {
		int j = i - TNC_CHAIN_LEVELS - 1;
		d.kind = TNC_KIND_CUBE;
		d.face = j / 2;
		d.level = (j & 1) ? 2 : 0;
		d.size_w = d.size_h = TNC_CUBE >> d.level;
	}
	return d;
}

/* the levels a texture kind has */
static inline int tnc_levels(enum tnc_kind kind)
{
	return kind == TNC_KIND_CHAIN ? TNC_CHAIN_LEVELS : kind == TNC_KIND_CUBE ? TNC_CUBE_LEVELS : 1;
}

/* Byte k of texel (x, y) of level `level` of layer `layer` of texture
 * `kind`: a hash, so that neighbouring texels, levels, faces, the textures
 * and the two bytes of an LA8 texel all differ in general (tnc_controls
 * measures how often they do not). */
static inline uint8_t tnc_byte(enum tnc_kind kind, int layer, int level, int x, int y, int k)
{
	uint32_t v = (uint32_t)x | (uint32_t)y << 7 | (uint32_t)level << 14 | (uint32_t)layer << 18 |
		     (uint32_t)kind << 21 | (uint32_t)k << 23;
	v ^= v >> 11;
	v *= 0x2c1b3c6du;
	v ^= v >> 15;
	v *= 0x297a2d39u;
	v ^= v >> 13;
	return (uint8_t)(v >> 8);
}

/* The level's width and height in texels of a texture kind */
static inline int tnc_level_w(enum tnc_kind kind, int level)
{
	int w = kind == TNC_KIND_CHAIN ? TNC_CHAIN : kind == TNC_KIND_ODD ? TNC_ODD_W : TNC_CUBE;
	w >>= level;
	return w > 0 ? w : 1;
}

static inline int tnc_level_h(enum tnc_kind kind, int level)
{
	int h = kind == TNC_KIND_CHAIN ? TNC_CHAIN : kind == TNC_KIND_ODD ? TNC_ODD_H : TNC_CUBE;
	h >>= level;
	return h > 0 ? h : 1;
}

/* The texel pixel (x, y) of draw d reads, GL window coordinates from the
 * viewport's lower-left corner (upload row 0 is t = 0, as tex_cube_case.h
 * assumes): its layer (the cube face dEQP selects), level, column and row. */
static inline void tnc_texel(const struct tnc_draw *d, int x, int y, int *layer, int *level, int *tx,
			     int *ty)
{
	*layer = 0;
	*level = d->level;
	*tx = x;
	*ty = y;
	if (d->kind != TNC_KIND_CUBE)
		return;
	const struct tcc_face *f = &tcc_faces[d->face];
	float xn = ((float)x + 0.5f) * 2.0f / (float)d->size_w - 1.0f;
	float yn = ((float)y + 0.5f) * 2.0f / (float)d->size_h - 1.0f;
	float dir[3], s, t;
	for (int c = 0; c < 3; c++)
		dir[c] = f->major[c] + xn * f->sx[c] + yn * f->ty[c];
	enum tfr_cube_face face = tfr_select_cube_face(dir[0], dir[1], dir[2]);
	tfr_project_to_face(face, dir[0], dir[1], dir[2], &s, &t);
	int lw = tnc_level_w(TNC_KIND_CUBE, d->level);
	int u = (int)(s * (float)lw), v = (int)(t * (float)lw);
	*layer = (int)face;
	*tx = u < 0 ? 0 : u >= lw ? lw - 1 : u;
	*ty = v < 0 ? 0 : v >= lw ? lw - 1 : v;
}

/* What a texel whose bytes are b0 (and b1, LA8) samples as, R, G, B, A */
static inline void tnc_sampled(enum tnc_format f, uint8_t b0, uint8_t b1, uint8_t out[4])
{
	switch (f) {
	case TNC_L8: out[0] = out[1] = out[2] = b0; out[3] = 255; break;
	case TNC_A8: out[0] = out[1] = out[2] = 0; out[3] = b0; break;
	default: out[0] = out[1] = out[2] = b0; out[3] = b1; break;
	}
}

/* The reference for pixel (x, y) of draw d of format f */
static inline void tnc_expected(enum tnc_format f, const struct tnc_draw *d, int x, int y, uint8_t out[4])
{
	int layer, level, tx, ty;
	tnc_texel(d, x, y, &layer, &level, &tx, &ty);
	tnc_sampled(f, tnc_byte(d->kind, layer, level, tx, ty, 0), tnc_byte(d->kind, layer, level, tx, ty, 1),
		    out);
}

static inline bool tnc_close(const uint8_t a[4], const uint8_t b[4])
{
	for (int c = 0; c < 4; c++) {
		int e = (int)a[c] - (int)b[c];
		if (e > TNC_TOL || e < -TNC_TOL)
			return false;
	}
	return true;
}

/* ---- the controls ---------------------------------------------------------
 * The wrong reads a defect would make, each a texel of the SAME texture read
 * in place of the right one (or the right texel's bytes misread):
 *   0 the next texel in x      1 the next row        2 x and y swapped
 *   3 the next level (the level's texel (x/2, y/2))   4 the next layer (cube)
 *   5 the bytes of texel 4i (rows read at 4 bytes a texel: the RGBA8 rule)
 *   6 the two bytes of an LA8 texel swapped (LA8 only)
 * For every draw, every pixel's reference must pass its own verdict, and
 * each wrong read may pass on at most 1/32 of the pixels it applies to. A
 * wrong read that does not apply (no next level, a texel that is its own
 * transpose) is not counted either way. */
#define TNC_WRONG 7

static inline bool tnc_wrong_read(enum tnc_format f, const struct tnc_draw *d, int w, int x, int y,
				  uint8_t out[4])
{
	int layer, level, tx, ty;
	tnc_texel(d, x, y, &layer, &level, &tx, &ty);
	int lw = tnc_level_w(d->kind, level), lh = tnc_level_h(d->kind, level);
	int k0 = 0, k1 = 1;
	switch (w) {
	case 0:
		if (lw == 1)
			return false; /* the only texel of its row */
		tx = (tx + 1) % lw;
		break;
	case 1:
		if (lh == 1)
			return false; /* the only row */
		ty = (ty + 1) % lh;
		break;
	case 2: {
		if (tx == ty || tx >= lh || ty >= lw)
			return false; /* no different texel to read */
		int s = tx;
		tx = ty;
		ty = s;
		break;
	}
	case 3:
		if (level + 1 >= tnc_levels(d->kind))
			return false; /* no next level */
		level++;
		tx /= 2;
		ty /= 2;
		break;
	case 4:
		if (d->kind != TNC_KIND_CUBE)
			return false;
		layer = (layer + 1) % 6;
		break;
	case 5: {
		int i = (ty * lw + tx) * 4 % (lw * lh);
		if (i == ty * lw + tx)
			return false; /* the texel reads itself */
		tx = i % lw;
		ty = i / lw;
		break;
	}
	default:
		if (f != TNC_LA8)
			return false;
		k0 = 1;
		k1 = 0;
		break;
	}
	tnc_sampled(f, tnc_byte(d->kind, layer, level, tx, ty, k0), tnc_byte(d->kind, layer, level, tx, ty, k1),
		    out);
	return true;
}

struct tnc_control_counts {
	unsigned refs, refs_ok;
	unsigned wrong[TNC_WRONG], wrong_passed[TNC_WRONG];
};

static inline void tnc_count_controls(struct tnc_control_counts *c)
{
	*c = (struct tnc_control_counts){ 0 };
	for (int f = 0; f < TNC_FORMATS; f++)
		for (int i = 0; i < TNC_DRAWS; i++) {
			struct tnc_draw d = tnc_draw_of(i);
			for (int y = 0; y < d.size_h; y++)
				for (int x = 0; x < d.size_w; x++) {
					uint8_t ref[4], bad[4];
					tnc_expected((enum tnc_format)f, &d, x, y, ref);
					c->refs++;
					c->refs_ok += tnc_close(ref, ref);
					for (int w = 0; w < TNC_WRONG; w++) {
						if (!tnc_wrong_read((enum tnc_format)f, &d, w, x, y, bad))
							continue;
						c->wrong[w]++;
						c->wrong_passed[w] += tnc_close(bad, ref);
					}
				}
		}
}

typedef void (*tnc_print_fn)(const char *fmt, ...);

/* the pixels of all 60 draws: 3 x (64^2 + 32^2 + ... + 1 + 91 + 6 x (256 + 16)) */
#define TNC_ALL_PIXELS (3u * (5461u + 91u + 6u * 272u))

static inline bool tnc_controls(tnc_print_fn print)
{
	static const char *const names[TNC_WRONG] = { "next x", "next row", "transposed", "next level",
						      "next face", "4-byte rows", "LA8 bytes swapped" };
	struct tnc_control_counts c;
	tnc_count_controls(&c);
	bool ok = c.refs == TNC_ALL_PIXELS && c.refs_ok == c.refs;
	print("tex_narrow controls: %u of %u reference pixels accepted", c.refs_ok, c.refs);
	for (int w = 0; w < TNC_WRONG; w++) {
		bool fine = c.wrong[w] > 0 && c.wrong_passed[w] * 32u <= c.wrong[w];
		ok &= fine;
		print("; %s %u of %u accepted", names[w], c.wrong_passed[w], c.wrong[w]);
	}
	print("; %s\n", ok ? "the check can tell them apart" : "FAIL: the check cannot tell them apart");
	return ok;
}

/* ---- the verdict ---------------------------------------------------------- */

struct tnc_result {
	unsigned judged, bad;
	int first_x, first_y;
	uint8_t first_got[4], first_want[4];
};

static inline void tnc_result_init(struct tnc_result *r)
{
	*r = (struct tnc_result){ 0, 0, -1, -1, { 0 }, { 0 } };
}

static inline void tnc_check_pixel(enum tnc_format f, const struct tnc_draw *d, int x, int y, const uint8_t px[4],
				   struct tnc_result *r)
{
	uint8_t want[4];
	tnc_expected(f, d, x, y, want);
	r->judged++;
	if (!tnc_close(px, want) && r->bad++ == 0) {
		r->first_x = x;
		r->first_y = y;
		for (int c = 0; c < 4; c++) {
			r->first_got[c] = px[c];
			r->first_want[c] = want[c];
		}
	}
}

/* the draw's line; true when every pixel of the viewport was judged and none
 * was wrong */
static inline bool tnc_verdict(enum tnc_format f, int i, const struct tnc_result *r, tnc_print_fn print)
{
	static const char *const faces[6] = { "+X", "-X", "+Y", "-Y", "+Z", "-Z" };
	struct tnc_draw d = tnc_draw_of(i);
	unsigned area = (unsigned)(d.size_w * d.size_h);
	bool ok = r->judged == area && r->bad == 0;
	if (d.kind == TNC_KIND_CUBE)
		print("tex_narrow %s cube %s level %d (%dx%d)", tnc_format_names[f], faces[d.face], d.level,
		      d.size_w, d.size_h);
	else
		print("tex_narrow %s %s level %d (%dx%d)", tnc_format_names[f],
		      d.kind == TNC_KIND_ODD ? "odd" : "chain", d.level, d.size_w, d.size_h);
	print(": %u of %u pixels judged, %u wrong; %s\n", r->judged, area, r->bad, ok ? "PASS" : "FAIL");
	if (r->bad)
		print("tex_narrow: first wrong pixel (%d, %d) = %d,%d,%d,%d want %d,%d,%d,%d\n", r->first_x,
		      r->first_y, r->first_got[0], r->first_got[1], r->first_got[2], r->first_got[3],
		      r->first_want[0], r->first_want[1], r->first_want[2], r->first_want[3]);
	return ok;
}

#endif /* TEX_NARROW_CASE_H */
