/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_cube_case.h: a cube-map board test, as data and checks a sample
 * includes (docs/online-shaderc/v3d-backend.md, "Cube maps"). The same shape
 * as tex_linear_case.h, and tools/v3d/tex_filter_ref_check.c checks it on the
 * host.
 *
 * The scene: a cube map whose six faces are 4x4 RGBA8 images, R a ramp
 * along s, G along t, and B the face (16 + 40 * face, so a wrong face shows
 * in every pixel). For each face and each filter (NEAREST, LINEAR; wrap
 * CLAMP_TO_EDGE, no mipmaps) a 32x32 quad is drawn whose vertex shader gives
 * the direction  major + x * sx + y * ty  (x, y the NDC position, -1..1), with
 * the face's three vectors from tcc_faces. The table is chosen so that on
 * every face the face coordinates are s = (x + 1) / 2 and t = (y + 1) / 2
 * (dEQP's projectToFace, checked on the host for every pixel), so pixel
 * (x, y) samples the face at u = (x + 0.5) / 8 texels, as the 2D test does:
 * every LINEAR weight is k/16.
 *
 * What is judged: NEAREST on every pixel, exactly; LINEAR on the interior
 * pixels (whose four texels all lie on the face) within dEQP's LINEAR bound.
 * The LINEAR edge pixels (4 pixels wide) are measured and printed, not
 * judged: there a seamless TMU blends in the next face while dEQP's
 * CLAMP_TO_EDGE reference does not, and GL ES 2.0 requires neither.
 *
 * What the sample does, per draw i in 0..TCC_DRAWS-1 (face = i / 2, LINEAR
 * when i is odd): upload the six faces once (tcc_make_faces(); face f to
 * GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, row 0 first); set the three uniforms
 * from tcc_faces[face], the min and mag filters, CLAMP_TO_EDGE; draw a
 * full-viewport quad into a 32x32 viewport; read back; tcc_check_pixel per
 * pixel (GL window coordinates from the lower-left corner); tcc_verdict.
 *
 * Header only, no OS calls, no libm.
 */
#ifndef TEX_CUBE_CASE_H
#define TEX_CUBE_CASE_H

#include <stdbool.h>
#include <stdint.h>

#include "tex_filter_ref.h"

#define TCC_TEX 4    /* each face is TCC_TEX x TCC_TEX */
#define TCC_SIZE 32  /* each draw is TCC_SIZE x TCC_SIZE pixels */
#define TCC_DRAWS 12 /* 6 faces x NEAREST, LINEAR */
#define TCC_EDGE 4   /* LINEAR pixels this close to a face edge read past it */

/* dEQP's LINEAR threshold for an RGBA8 target (tex_linear_case.h) */
#define TCC_LINEAR_TOL_LSB (255.0f * (float)((1u << 17) - 1u) / (float)(1u << 23))

/* direction = major + x * sx + y * ty, per face in GL's order */
struct tcc_face {
	float major[3], sx[3], ty[3];
};
static const struct tcc_face tcc_faces[6] = {
	{ { 1, 0, 0 }, { 0, 0, -1 }, { 0, -1, 0 } },  /* +X: sc = -z, tc = -y */
	{ { -1, 0, 0 }, { 0, 0, 1 }, { 0, -1, 0 } },  /* -X: sc = +z, tc = -y */
	{ { 0, 1, 0 }, { 1, 0, 0 }, { 0, 0, 1 } },    /* +Y: sc = +x, tc = +z */
	{ { 0, -1, 0 }, { 1, 0, 0 }, { 0, 0, -1 } },  /* -Y: sc = +x, tc = -z */
	{ { 0, 0, 1 }, { 1, 0, 0 }, { 0, -1, 0 } },   /* +Z: sc = +x, tc = -y */
	{ { 0, 0, -1 }, { -1, 0, 0 }, { 0, -1, 0 } }, /* -Z: sc = -x, tc = -y */
};

static uint8_t tcc_texels[6][TCC_TEX * TCC_TEX * 4];

static inline void tcc_make_faces(void)
{
	static const uint8_t ramp[4] = { 0, 85, 170, 255 };
	for (int f = 0; f < 6; f++)
		for (int j = 0; j < TCC_TEX; j++)
			for (int i = 0; i < TCC_TEX; i++) {
				uint8_t *p = &tcc_texels[f][(j * TCC_TEX + i) * 4];
				p[0] = ramp[i];
				p[1] = ramp[j];
				p[2] = (uint8_t)(16 + 40 * f);
				p[3] = 255;
			}
}

/* the NDC coordinate of pixel column/row p's centre */
static inline float tcc_ndc(int p)
{
	return ((float)p + 0.5f) * 2.0f / (float)TCC_SIZE - 1.0f;
}

/* the direction the vertex shader interpolates to pixel (x, y) of `face` */
static inline void tcc_dir(int face, int x, int y, float out[3])
{
	const struct tcc_face *f = &tcc_faces[face];
	float xn = tcc_ndc(x), yn = tcc_ndc(y);
	for (int c = 0; c < 3; c++)
		out[c] = f->major[c] + xn * f->sx[c] + yn * f->ty[c];
}

/* true when a LINEAR sample at pixel (x, y) reads past the face's edge */
static inline bool tcc_edge(int x, int y)
{
	return x < TCC_EDGE || x >= TCC_SIZE - TCC_EDGE || y < TCC_EDGE || y >= TCC_SIZE - TCC_EDGE;
}

/* dEQP's reference for pixel (x, y) of the draw for `face`: the face chosen
 * and projected by dEQP's own functions (not by the table), then sampled on
 * that face with CLAMP_TO_EDGE */
static inline void tcc_expected(int face, bool linear, int x, int y, float out[4])
{
	float d[3], s, t;
	tcc_dir(face, x, y, d);
	enum tfr_cube_face f = tfr_select_cube_face(d[0], d[1], d[2]);
	tfr_project_to_face(f, d[0], d[1], d[2], &s, &t);
	struct tfr_texture tex = { tcc_texels[f], TCC_TEX, TCC_TEX };
	float u = s * TCC_TEX, v = t * TCC_TEX;
	if (linear)
		tfr_linear(&tex, TFR_CLAMP_TO_EDGE, TFR_CLAMP_TO_EDGE, u, v, out);
	else
		tfr_nearest(&tex, TFR_CLAMP_TO_EDGE, TFR_CLAMP_TO_EDGE, u, v, out);
}

struct tcc_result {
	unsigned judged, over, edge;
	float max_lsb[4], edge_max_lsb;
	int first_over_x, first_over_y;
};

static inline void tcc_result_init(struct tcc_result *r)
{
	r->judged = r->over = r->edge = 0;
	for (int c = 0; c < 4; c++)
		r->max_lsb[c] = 0.0f;
	r->edge_max_lsb = 0.0f;
	r->first_over_x = r->first_over_y = -1;
}

static inline void tcc_check_pixel(int face, bool linear, int x, int y, const uint8_t px[4],
				   struct tcc_result *r)
{
	float ref[4], per[4];
	tcc_expected(face, linear, x, y, ref);
	float e = tfr_error_lsb(px, ref, per);
	if (linear && tcc_edge(x, y)) {
		r->edge++;
		if (e > r->edge_max_lsb)
			r->edge_max_lsb = e;
		return;
	}
	r->judged++;
	for (int c = 0; c < 4; c++)
		if (per[c] > r->max_lsb[c])
			r->max_lsb[c] = per[c];
	if (e > (linear ? TCC_LINEAR_TOL_LSB : 0.0f) + 0.001f) {
		if (r->over == 0) {
			r->first_over_x = x;
			r->first_over_y = y;
		}
		r->over++;
	}
}

typedef void (*tcc_print_fn)(const char *fmt, ...);

#define TCC_HUNDREDTHS(f) (int)(f), (int)((f) * 100.0f) % 100

/* the draw's line; true when every pixel was measured (judged + edge = the
 * quad) and none judged is over its tolerance */
static inline bool tcc_verdict(int face, bool linear, const struct tcc_result *r,
			       tcc_print_fn print)
{
	static const char *const names[6] = { "+X", "-X", "+Y", "-Y", "+Z", "-Z" };
	const unsigned area = TCC_SIZE * TCC_SIZE;
	const unsigned interior = (TCC_SIZE - 2 * TCC_EDGE) * (TCC_SIZE - 2 * TCC_EDGE);
	const unsigned want_judged = linear ? interior : area;
	bool ok = r->judged == want_judged && r->judged + r->edge == area && r->over == 0;
	print("tex_cube face %s %s: %u of %u judged, %u over %d.%02d; max error R %d.%02d "
	      "G %d.%02d B %d.%02d A %d.%02d; edge pixels %u, largest edge error %d.%02d "
	      "(not judged); %s\n",
	      names[face], linear ? "LINEAR" : "NEAREST", r->judged, want_judged, r->over,
	      TCC_HUNDREDTHS(linear ? TCC_LINEAR_TOL_LSB : 0.0f), TCC_HUNDREDTHS(r->max_lsb[0]),
	      TCC_HUNDREDTHS(r->max_lsb[1]), TCC_HUNDREDTHS(r->max_lsb[2]),
	      TCC_HUNDREDTHS(r->max_lsb[3]), r->edge, TCC_HUNDREDTHS(r->edge_max_lsb),
	      ok ? "PASS" : "FAIL");
	if (r->over)
		print("tex_cube face %s %s: first pixel over the tolerance at (%d, %d)\n",
		      names[face], linear ? "LINEAR" : "NEAREST", r->first_over_x, r->first_over_y);
	return ok;
}

/* The controls, printed; false if the probe cannot tell the cases apart or
 * the face table does not send each pixel where it should:
 * - for every face and pixel, dEQP's selectCubeFace picks that face and its
 *   projectToFace gives s = (x + 0.5) / 32, t = (y + 0.5) / 32;
 * - LINEAR and NEAREST differ over dEQP's LINEAR bound on at least a quarter
 *   of the interior;
 * - two neighbouring faces' expected colours differ on every pixel. */
static inline bool tcc_controls(tcc_print_fn print)
{
	unsigned wrong_face = 0, wrong_st = 0, lin_near = 0, faces_apart = 0;
	for (int face = 0; face < 6; face++)
		for (int y = 0; y < TCC_SIZE; y++)
			for (int x = 0; x < TCC_SIZE; x++) {
				float d[3], s, t;
				tcc_dir(face, x, y, d);
				enum tfr_cube_face f = tfr_select_cube_face(d[0], d[1], d[2]);
				tfr_project_to_face(f, d[0], d[1], d[2], &s, &t);
				float ws = ((float)x + 0.5f) / (float)TCC_SIZE;
				float wt = ((float)y + 0.5f) / (float)TCC_SIZE;
				wrong_face += (int)f != face;
				wrong_st += tfr_abs(s - ws) > 1e-6f || tfr_abs(t - wt) > 1e-6f;
				float a[4], b[4];
				uint8_t px[4];
				if (!tcc_edge(x, y)) {
					tcc_expected(face, true, x, y, a);
					tcc_expected(face, false, x, y, b);
					for (int c = 0; c < 4; c++)
						px[c] = (uint8_t)(a[c] * 255.0f + 0.5f);
					lin_near += tfr_error_lsb(px, b, (float *)0) > TCC_LINEAR_TOL_LSB;
				}
				tcc_expected(face, false, x, y, a);
				tcc_expected((face + 1) % 6, false, x, y, b);
				for (int c = 0; c < 4; c++)
					px[c] = (uint8_t)(a[c] * 255.0f + 0.5f);
				faces_apart += tfr_error_lsb(px, b, (float *)0) > TCC_LINEAR_TOL_LSB;
			}
	const unsigned all = 6 * TCC_SIZE * TCC_SIZE;
	const unsigned interior = 6 * (TCC_SIZE - 2 * TCC_EDGE) * (TCC_SIZE - 2 * TCC_EDGE);
	bool ok = wrong_face == 0 && wrong_st == 0 && lin_near >= interior / 4 && faces_apart == all;
	print("tex_cube controls: face table %u wrong faces, %u wrong (s, t) of %u pixels; "
	      "LINEAR vs NEAREST %u of %u interior pixels; neighbouring faces apart on %u of %u; "
	      "%s\n",
	      wrong_face, wrong_st, all, lin_near, interior, faces_apart, all,
	      ok ? "the probe can tell them apart" : "FAIL");
	return ok;
}

#endif
