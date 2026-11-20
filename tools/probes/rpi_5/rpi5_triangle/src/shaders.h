/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The three programs of the rpi5_triangle image: Mesa's (the default) or our
 * 7.1 compiler's (RPI5_TRI_OURS=1, rpi5_triangle_ours), as the builder
 * v3d_frame_build_triangle() takes them. One translation unit includes the
 * one generated header under tools/v3d/tri-shaders71/.
 */
#ifndef RPI5_TRI_SHADERS_H
#define RPI5_TRI_SHADERS_H

#include <stdint.h>

#include "v3d_frame.h"

#define TRI_SHADER_ATTRS 2u /* position (vec4), colour (vec4) */

struct tri_shaders {
	const char *name;        /* "mesa" or "ours" */
	const uint64_t *code[3]; /* FS, VS, CS QPU words */
	uint32_t words[3];
	/* prog_data; the code addresses are 0 until the caller places the code */
	struct v3d_fs_prog fs;
	struct v3d_vs_prog vs, cs;
	/* the attribute records: components (float) and byte stride of each */
	uint32_t attr_components[TRI_SHADER_ATTRS], attr_stride[TRI_SHADER_ATTRS];
};

/* Fill `s` from the selected header. */
void tri_shaders_get(struct tri_shaders *s);

/*
 * The D0 shader record the builder wrote (struct v3d_triangle_layout.rec),
 * field by field against the selected header's record values (the
 * MESA71_TRI_REC_* or OURS71_TRI_REC_* fields that come from the programs,
 * plus the clipping, point-size and propagate-NaNs bits both references
 * show). Returns the number of fields that differ; *first names the first.
 */
uint32_t tri_shaders_rec_check(const struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX *rec,
			       const char **first);

#endif
