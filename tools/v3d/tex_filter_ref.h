/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_filter_ref.h: the CPU reference for a board test of 2D texture
 * filtering (docs/online-shaderc/v3d-backend.md, "LINEAR on the board").
 *
 * The filters are dEQP's, because dEQP is what will judge the driver:
 * transcribed from VK-GL-CTS framework/common/tcuTexture.cpp at 93bca01
 * (Apache-2.0, "Copyright 2014 The Android Open Source Project"): wrap() and
 * mirror() (:2325-2376), sampleNearest2D (:2521-2539) and sampleLinear2D
 * (:2624-2660), for an RGBA8 UNORM texture, no border, no offset.
 *
 * Header only, no OS calls and no libm, so a Zephyr sample and a host test
 * include the same code. Texture coordinates are in TEXELS (dEQP's
 * unnormalised u = size * s). Results are floats in [0, 1].
 */
#ifndef TEX_FILTER_REF_H
#define TEX_FILTER_REF_H

#include <stddef.h>
#include <stdint.h>

enum tfr_wrap { TFR_CLAMP_TO_EDGE, TFR_REPEAT, TFR_MIRRORED_REPEAT };

struct tfr_texture {
	const uint8_t *texels; /* RGBA8, rows of `width` texels, row 0 is t = 0 */
	int width, height;
};

/* deFloorFloatToInt32, without libm */
static inline int tfr_floor(float f)
{
	int i = (int)f;
	return ((float)i > f) ? i - 1 : i;
}

/* deFloatFrac: f - floor(f) */
static inline float tfr_frac(float f)
{
	return f - (float)tfr_floor(f);
}

static inline int tfr_imod(int a, int b)
{
	int m = a % b;
	return m < 0 ? m + b : m;
}

static inline int tfr_mirror(int a)
{
	return a >= 0 ? a : -(1 + a);
}

/* tcuTexture.cpp wrap(), the three GL ES 2.0 modes */
static inline int tfr_wrap(enum tfr_wrap mode, int c, int size)
{
	switch (mode) {
	case TFR_CLAMP_TO_EDGE:
		return c < 0 ? 0 : (c > size - 1 ? size - 1 : c);
	case TFR_REPEAT:
		return tfr_imod(c, size);
	case TFR_MIRRORED_REPEAT:
	default:
		return (size - 1) - tfr_mirror(tfr_imod(c, 2 * size) - size);
	}
}

static inline void tfr_lookup(const struct tfr_texture *t, int i, int j, float out[4])
{
	const uint8_t *p = t->texels + ((size_t)j * (size_t)t->width + (size_t)i) * 4u;
	for (int c = 0; c < 4; c++)
		out[c] = (float)p[c] / 255.0f;
}

/* sampleNearest2D */
static inline void tfr_nearest(const struct tfr_texture *t, enum tfr_wrap ws, enum tfr_wrap wt,
			       float u, float v, float out[4])
{
	tfr_lookup(t, tfr_wrap(ws, tfr_floor(u), t->width), tfr_wrap(wt, tfr_floor(v), t->height),
		   out);
}

/* sampleLinear2D */
static inline void tfr_linear(const struct tfr_texture *t, enum tfr_wrap ws, enum tfr_wrap wt,
			      float u, float v, float out[4])
{
	int x0 = tfr_floor(u - 0.5f), x1 = x0 + 1;
	int y0 = tfr_floor(v - 0.5f), y1 = y0 + 1;
	int i0 = tfr_wrap(ws, x0, t->width), i1 = tfr_wrap(ws, x1, t->width);
	int j0 = tfr_wrap(wt, y0, t->height), j1 = tfr_wrap(wt, y1, t->height);
	float a = tfr_frac(u - 0.5f), b = tfr_frac(v - 0.5f);
	float p00[4], p10[4], p01[4], p11[4];
	tfr_lookup(t, i0, j0, p00);
	tfr_lookup(t, i1, j0, p10);
	tfr_lookup(t, i0, j1, p01);
	tfr_lookup(t, i1, j1, p11);
	for (int c = 0; c < 4; c++)
		out[c] = p00[c] * (1.0f - a) * (1.0f - b) + p10[c] * a * (1.0f - b) +
			 p01[c] * (1.0f - a) * b + p11[c] * a * b;
}

/* Cube maps: dEQP's selectCubeFace and projectToFace (tcuTexture.cpp
 * :3530-3631 at 93bca01), with the faces numbered in GL's order (+X, -X, +Y,
 * -Y, +Z, -Z = GL_TEXTURE_CUBE_MAP_POSITIVE_X + n), not dEQP's (which starts
 * at -X). */
enum tfr_cube_face { TFR_PX, TFR_NX, TFR_PY, TFR_NY, TFR_PZ, TFR_NZ };

static inline float tfr_abs(float f)
{
	return f < 0.0f ? -f : f;
}

static inline enum tfr_cube_face tfr_select_cube_face(float x, float y, float z)
{
	const float ax = tfr_abs(x), ay = tfr_abs(y), az = tfr_abs(z);
	if (ay < ax && az < ax)
		return x >= 0.0f ? TFR_PX : TFR_NX;
	if (ax < ay && az < ay)
		return y >= 0.0f ? TFR_PY : TFR_NY;
	if (ax < az && ay < az)
		return z >= 0.0f ? TFR_PZ : TFR_NZ;
	/* some components are equal: dEQP's tie-breaking rule */
	if (ax == ay)
		return ax < az ? (z >= 0.0f ? TFR_PZ : TFR_NZ) : (x >= 0.0f ? TFR_PX : TFR_NX);
	if (ax == az)
		return az < ay ? (y >= 0.0f ? TFR_PY : TFR_NY) : (z >= 0.0f ? TFR_PZ : TFR_NZ);
	if (ay == az)
		return ay < ax ? (x >= 0.0f ? TFR_PX : TFR_NX) : (y >= 0.0f ? TFR_PY : TFR_NY);
	return x >= 0.0f ? TFR_PX : TFR_NX;
}

/* The face coordinates (s, t) in [0, 1]; (0, 0) when the major axis is 0 */
static inline void tfr_project_to_face(enum tfr_cube_face face, float x, float y, float z,
				       float *s, float *t)
{
	float sc, tc, ma;
	switch (face) {
	case TFR_NX: sc = +z; tc = -y; ma = -x; break;
	case TFR_PX: sc = -z; tc = -y; ma = +x; break;
	case TFR_NY: sc = +x; tc = -z; ma = -y; break;
	case TFR_PY: sc = +x; tc = +z; ma = +y; break;
	case TFR_NZ: sc = -x; tc = -y; ma = -z; break;
	case TFR_PZ:
	default: sc = +x; tc = -y; ma = +z; break;
	}
	if (tfr_abs(ma) < 1.1920929e-7f) { /* FLT_EPSILON */
		*s = *t = 0.0f;
		return;
	}
	*s = ((sc / ma) + 1.0f) / 2.0f;
	*t = ((tc / ma) + 1.0f) / 2.0f;
}

/* How far a read-back RGBA8 pixel is from a reference colour, in 1/255 steps,
 * per channel (optional) and the largest of the four (returned). */
static inline float tfr_error_lsb(const uint8_t px[4], const float ref[4], float per_channel[4])
{
	float worst = 0.0f;
	for (int c = 0; c < 4; c++) {
		float d = (float)px[c] - ref[c] * 255.0f;
		if (d < 0.0f)
			d = -d;
		if (per_channel)
			per_channel[c] = d;
		if (d > worst)
			worst = d;
	}
	return worst;
}

#endif
