/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * tex_upload_check: the two ways the V3D backend fills a texture's tiled copy
 * give the same bytes (docs/online-shaderc/v3d-backend.md, "tile at upload").
 *
 *  - eager, at the upload (src/v3d/backend.rs v3d_texture_uploaded, called by
 *    the GL core's texture_uploaded hook for a whole level-0 GL_RGBA /
 *    GL_UNSIGNED_BYTE upload): v3d_tex_store_box from the CALLER's rows, rows
 *    `stride` bytes apart, where stride is the GL core's src_row_bytes
 *    (src/gl/texture.rs:824-831: UNPACK_ROW_LENGTH if set, else the width,
 *    times 4, rounded up to UNPACK_ALIGNMENT);
 *  - lazy, at a draw (src/v3d/texture.rs:161-168 tile): v3d_tex_store_level
 *    from the GL core's STORAGE, level 0 at tex_pitch(w) * 4 bytes a row
 *    (src/v3d/texture.rs:21-23: the width rounded up to a pair of texels).
 *
 * For every case the caller's rows are made with random texels and random
 * pad bytes, the storage is made from them as the GL core stores an RGBA
 * upload (w texels a row at tex_pitch, its pad texels random), and both
 * copies are tiled into images that start from the same fill. The WHOLE
 * images are compared, padding included. Cases: 19 sizes (every Slint texture
 * size the recorder measured, odd widths, one-wide and one-high) x
 * UNPACK_ALIGNMENT 1, 2, 4, 8 x UNPACK_ROW_LENGTH 0, w, w + 5.
 *
 * What this cannot see: the two rules above are mirrored here from the Rust,
 * and the GL core's storage layout is taken as "w texels a row at
 * tex_pitch". The backend's own tests pin those (gl_level against the GL
 * core's level_off; the rig's eager-equals-lazy test). What this adds is the
 * real C: the two calls give the same bytes for the same texels under every
 * unpack state.
 *
 * Controls, each of which must be detected wherever it can matter: the eager
 * path given w * 4 instead of the unpack stride (where the stride is not
 * w * 4), and the lazy path given w * 4 instead of tex_pitch (odd widths).
 * And the eager call refuses a stride one byte short of a row, writing
 * nothing.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include \
 *      tools/v3d/tex_upload_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
 *      -lm -o build/tex_upload_check
 *
 * Exit status 0 only when every check passed AND the number run is the number
 * planned.
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_tex.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("tex_upload_check: FAIL %s\n", what);
	}
}

static uint32_t rng_state = 99;
static uint32_t rng(void)
{
	uint32_t x = rng_state;
	x ^= x << 13;
	x ^= x >> 17;
	x ^= x << 5;
	rng_state = x;
	return x;
}

/* src/v3d/texture.rs:21-23 */
static uint32_t tex_pitch(uint32_t w)
{
	return (w + 1) & ~1u;
}

/* src/gl/texture.rs:824-831, for GL_RGBA / GL_UNSIGNED_BYTE (4 bytes) */
static uint32_t unpack_stride(uint32_t w, uint32_t row_length, uint32_t align)
{
	uint32_t n = (row_length ? row_length : w) * 4;
	return (n + align - 1) & ~(align - 1);
}

enum mutation { NONE, EAGER_PACKED_STRIDE, LAZY_PACKED_PITCH };

/* 0: the images are equal; 1: they differ; -1: the eager call refused. */
static int compare(uint32_t w, uint32_t h, uint32_t row_length, uint32_t align, enum mutation m)
{
	struct v3d_tex_layout l;
	if (!v3d_tex_layout(w, h, 1, 4, &l))
		return -2;
	uint32_t stride = unpack_stride(w, row_length, align), pitch = tex_pitch(w) * 4;
	uint8_t *caller = malloc((size_t)stride * h), *storage = malloc((size_t)pitch * h);
	uint8_t *eager = malloc(l.size), *lazy = malloc(l.size);
	for (size_t i = 0; i < (size_t)stride * h; i++)
		caller[i] = (uint8_t)rng();
	for (size_t i = 0; i < (size_t)pitch * h; i++)
		storage[i] = (uint8_t)rng();
	/* the GL core's store of an RGBA upload: w texels a row */
	for (uint32_t y = 0; y < h; y++)
		memcpy(storage + (size_t)y * pitch, caller + (size_t)y * stride, (size_t)w * 4);
	memset(eager, 0x5a, l.size);
	memset(lazy, 0x5a, l.size);
	bool ok = v3d_tex_store_box(eager, &l, 0, 0, 0, w, h, caller,
				    m == EAGER_PACKED_STRIDE ? w * 4 : stride);
	v3d_tex_store_level(lazy, &l, 0, storage, m == LAZY_PACKED_PITCH ? w * 4 : pitch);
	int r = !ok ? -1 : memcmp(eager, lazy, l.size) != 0;
	free(caller);
	free(storage);
	free(eager);
	free(lazy);
	return r;
}

#define SIZES 19
static const uint32_t sizes[SIZES][2] = {
	{ 1, 1 },    { 2, 1 },    { 3, 5 },     { 7, 1 },     { 1, 9 },     { 13, 7 },   { 64, 64 },
	{ 100, 3 },  { 256, 1 },  { 60, 30 },   { 77, 21 },   { 320, 21 },  { 400, 21 }, { 400, 65 },
	{ 400, 147 }, { 330, 132 }, { 168, 67 }, { 513, 9 }, { 255, 255 },
};
static const uint32_t aligns[4] = { 1, 2, 4, 8 };

int main(void)
{
	/* one check per case; the two controls; the short-stride refusal */
	const int cases_planned = SIZES * 4 * 3;
	const int planned = cases_planned + 2 + 1;
	int cases = 0, c1_cases = 0, c1_seen = 0, c2_cases = 0, c2_seen = 0;
	char what[160];

	for (int s = 0; s < SIZES; s++)
		for (int a = 0; a < 4; a++)
			for (int k = 0; k < 3; k++) {
				uint32_t w = sizes[s][0], h = sizes[s][1];
				uint32_t rl = k == 0 ? 0 : (k == 1 ? w : w + 5);
				int r = compare(w, h, rl, aligns[a], NONE);
				cases++;
				snprintf(what, sizeof(what),
					 "%ux%u, UNPACK_ALIGNMENT %u, UNPACK_ROW_LENGTH %u: eager and lazy "
					 "copies are the same bytes (got %d)",
					 w, h, aligns[a], rl, r);
				check(r == 0, what);
				if (unpack_stride(w, rl, aligns[a]) != w * 4 && h > 1) {
					c1_cases++;
					c1_seen += compare(w, h, rl, aligns[a], EAGER_PACKED_STRIDE) != 0;
				}
				if ((w & 1) && h > 1) {
					c2_cases++;
					c2_seen += compare(w, h, rl, aligns[a], LAZY_PACKED_PITCH) != 0;
				}
			}
	snprintf(what, sizeof(what),
		 "control: the eager path given w*4 instead of the unpack stride differs in %d of %d",
		 c1_seen, c1_cases);
	check(c1_cases > 0 && c1_seen == c1_cases, what);
	snprintf(what, sizeof(what),
		 "control: the lazy path given w*4 instead of tex_pitch differs in %d of %d", c2_seen,
		 c2_cases);
	check(c2_cases > 0 && c2_seen == c2_cases, what);

	/* the eager call refuses a stride a byte short of a row, writing nothing */
	struct v3d_tex_layout l;
	v3d_tex_layout(13, 7, 1, 4, &l);
	uint8_t rows[13 * 4 * 7], *img = malloc(l.size), *copy = malloc(l.size);
	memset(rows, 1, sizeof(rows));
	memset(img, 0x5a, l.size);
	memcpy(copy, img, l.size);
	bool refused = !v3d_tex_store_box(img, &l, 0, 0, 0, 13, 7, rows, 13 * 4 - 1);
	check(refused && memcmp(img, copy, l.size) == 0,
	      "the eager call refuses a stride one byte short of a row, and writes nothing");
	free(img);
	free(copy);

	printf("tex_upload_check: %d cases (planned %d); controls %d of %d and %d of %d\n", cases,
	       cases_planned, c1_seen, c1_cases, c2_seen, c2_cases);
	printf("tex_upload_check: %d checks run (planned %d), %d failed\n", checks, planned,
	       failures);
	return (failures == 0 && checks == planned && cases == cases_planned) ? 0 : 1;
}
