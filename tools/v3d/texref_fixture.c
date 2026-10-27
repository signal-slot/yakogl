/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * texref_fixture.c: the outputs of v3d-tex's C reference functions, as rows
 * src/shaderc/online/texref.rs compares bit for bit against its Rust
 * transcription (tests/fixtures/v3d_texref.txt). Written by
 * tools/v3d/texref_fixture.sh, which adds the headers' SHA-256 so that a
 * header that changes without the fixture being written again fails the
 * Rust test.
 *
 * Rows:
 *   cube <x> <y> <z> <face> <s> <t>   tfr_select_cube_face and
 *       tfr_project_to_face (tex_filter_ref.h) on every direction
 *       tex_cube_case.h's draws give (tcc_dir, 6 faces x 32 x 32 pixels),
 *       the hand cases of tex_filter_ref_check.c's part_cube, and the ties
 *       and degenerate directions; floats as their bit patterns in hex, the
 *       face in GL order.
 *   level <lo> <hi> <lo_level> <hi_level>   tmc_levels (tex_mip_case.h)
 *       for NEAREST_MIPMAP_NEAREST over its 7 levels, for LOD bounds
 *       [lod - e, lod + e] with e = tmc_fixed_error(8) and [lod, lod], lod
 *       from -1 to 8 in steps of 1/64.
 */
#include <stdio.h>
#include <string.h>

#include "tex_cube_case.h"
#include "tex_mip_case.h"

static unsigned bits(float f)
{
	unsigned u;
	memcpy(&u, &f, 4);
	return u;
}

static unsigned cube_rows;

static void cube(float x, float y, float z)
{
	float s, t;
	enum tfr_cube_face f = tfr_select_cube_face(x, y, z);
	tfr_project_to_face(f, x, y, z, &s, &t);
	printf("cube %08x %08x %08x %d %08x %08x\n", bits(x), bits(y), bits(z), (int)f, bits(s), bits(t));
	cube_rows++;
}

int main(void)
{
	unsigned level_rows = 0;
	for (int face = 0; face < 6; face++)
		for (int y = 0; y < TCC_SIZE; y++)
			for (int x = 0; x < TCC_SIZE; x++) {
				float d[3];
				tcc_dir(face, x, y, d);
				cube(d[0], d[1], d[2]);
			}
	/* part_cube's hand cases, the ties it checks, and the degenerate ones */
	cube(1.0f, 0.5f, -0.25f);
	cube(0.5f, -1.0f, 0.25f);
	cube(1.0f, 1.0f, 0.5f);
	cube(0.5f, -1.0f, -1.0f);
	cube(1.0f, 1.0f, 1.0f);
	cube(-1.0f, 1.0f, -1.0f);
	cube(0.25f, -0.5f, 0.5f);
	cube(0.0f, 0.0f, 0.0f);
	cube(-0.0f, 0.0f, -2.0f);
	cube(1e-8f, 0.0f, 0.0f);
	struct tmc_draw d = { TMC_NEAREST_MIPMAP_NEAREST, 0, false };
	const float e = tmc_fixed_error(8);
	for (int k = -64; k <= 8 * 64; k++) {
		float lod = (float)k / 64.0f;
		float bounds[2][2] = { { lod - e, lod + e }, { lod, lod } };
		for (int b = 0; b < 2; b++) {
			int lo, hi;
			tmc_levels(&d, bounds[b][0], bounds[b][1], &lo, &hi);
			printf("level %08x %08x %d %d\n", bits(bounds[b][0]), bits(bounds[b][1]), lo, hi);
			level_rows++;
		}
	}
	/* the counts, for the script to put at the head of the file */
	fprintf(stderr, "# cube rows: %u\n# level rows: %u\n# fixed_error(8): %08x\n", cube_rows, level_rows,
		bits(e));
	return 0;
}
