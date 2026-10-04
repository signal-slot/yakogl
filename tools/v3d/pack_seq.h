/* SPDX-License-Identifier: MIT
 *
 * pack_seq.h: one job of PACK_SEQ_DRAWS draws whose state changes in a fixed
 * pattern, for job_check's packet-skip part (15, v3d_job.h "Packet skip").
 * Every input of every packet group changes, alone and together with others:
 * the viewport (x, and z alone), the scissor, culling, the depth test and
 * depth offset, one- and two-sided stencil, blending and its factors, the
 * colour mask, the blend colour, the line width, flat shading, points, lines
 * and triangles, arrays and indexed. Each field has its own period, so the
 * groups change at different draws, and an input that matters only while
 * another is on (the scissor box, the depth function, the offset units, the
 * blend factors, the stencil reference) also changes while it stays on.
 *
 * Header-only, with no use of the packet-skip API, so the same job can be
 * built against another revision of driver/src/v3d/v3d_job.c: job_check.c's
 * pin of the knob-off bytes was taken that way (see part 15 there).
 */
#ifndef PACK_SEQ_H
#define PACK_SEQ_H

#include "v3d_job.h"

#define PACK_SEQ_DRAWS 48

static const struct v3d_uniform pack_seq_fs_u[] = { { V3D_QUNIFORM_CONSTANT, 0x3f800000u } };
static const struct v3d_uniform pack_seq_vs_u[] = {
	{ V3D_QUNIFORM_VIEWPORT_X_SCALE, 0 },
	{ V3D_QUNIFORM_VIEWPORT_Z_OFFSET, 0 },
};

/* Draw i's state, primitive, index use and flat-shade flags. */
static void pack_seq_state(uint32_t i, uint32_t w, uint32_t h, struct v3d_job_state *st,
			   uint32_t *prim, bool *indexed, uint32_t *flat)
{
	*st = (struct v3d_job_state){
		.front_ccw = true, .line_width = 1.0f, .depth_func = V3D_COMPARE_FUNC_ALWAYS,
		.colormask = 0xf, .sample_mask = 0xf,
	};
	float x0 = (i / 7) % 2 ? 8.0f : 0.0f;          /* CLIP and VIEWPORT */
	float z_near = (i / 5) % 2 ? 0.25f : 0.0f;     /* VIEWPORT alone */
	v3d_viewport_gl(x0, 0, (float)w - x0, (float)h, z_near, 1.0f, true, h, &st->vp);
	if ((i / 3) % 4 == 1) {                        /* CLIP alone */
		st->scissor = true;
		st->scissor_minx = 4;
		st->scissor_miny = 2;
		st->scissor_maxx = w - 4;
		st->scissor_maxy = h - 2 - i % 2;
	}
	st->cull_back = (i / 11) % 2;                  /* CFG */
	if ((i / 6) % 2) {                             /* CFG */
		st->depth_enabled = true;
		st->depth_writemask = i % 2 == 0;
		st->depth_func = (i / 3) % 2 ? V3D_COMPARE_FUNC_LESS : V3D_COMPARE_FUNC_LEQUAL;
	}
	if ((i / 8) % 2) {                             /* CFG and DEPTH_OFFSET */
		st->offset_tri = true;
		st->offset_scale = 1.0f;
		st->offset_units = (i / 2) % 2 ? 4.0f : 2.0f;
	}
	st->line_width = (i / 10) % 2 ? 2.0f : 1.0f;   /* LINE */
	if ((i / 2) % 3 != 0) {                        /* CFG and BLEND */
		st->blend_enabled = true;
		st->rgb_func = V3D_BLEND_MODE_ADD;
		st->alpha_func = V3D_BLEND_MODE_ADD;
		st->rgb_src = i % 2 ? V3D_BLEND_FACTOR_SRC_ALPHA : V3D_BLEND_FACTOR_ONE;
		st->rgb_dst = V3D_BLEND_FACTOR_INV_SRC_ALPHA;
		st->alpha_src = V3D_BLEND_FACTOR_ONE;
		st->alpha_dst = V3D_BLEND_FACTOR_INV_SRC_ALPHA;
	}
	st->colormask = (i / 9) % 2 ? 0x7 : 0xf;       /* COLOR_MASK */
	st->blend_color_f16[0] = (i / 17) % 2 ? 0x3c00 : 0; /* BLEND_COLOR */
	uint32_t stencil = (i / 4) % 3;                /* CFG and STENCIL */
	if (stencil) {
		st->stencil_enabled = true;
		st->stencil_front = (struct v3d_job_stencil_face){
			.func = V3D_COMPARE_FUNC_ALWAYS, .sfail_op = V3D_STENCIL_OP_REPLACE,
			.zfail_op = V3D_STENCIL_OP_REPLACE, .zpass_op = V3D_STENCIL_OP_REPLACE,
			.ref = 1 + i % 2, .value_mask = 0xff, .write_mask = 0xff,
		};
		if (stencil == 2) {
			st->stencil_back_enabled = true;
			st->stencil_back = st->stencil_front;
			st->stencil_back.write_mask = 0x0f;
		}
	}
	*flat = (i / 5) % 3 == 2 ? 0x3u : 0;           /* VARYINGS */
	*prim = i % 13 == 5 ? V3D_PRIM_POINTS : i % 13 == 6 ? V3D_PRIM_LINES : V3D_PRIM_TRIANGLES;
	*indexed = i % 4 == 3;
}

/* Build the job into the three lists: every draw must go in and the job
 * must end. */
static bool pack_seq_build(struct v3d_job *j, const struct v3d_job_target *t, struct v3d_cl *bcl,
			   struct v3d_cl *rcl, struct v3d_cl *ind)
{
	static const uint32_t vs_words[1] = { 0 };
	struct v3d_vs_prog vs = {
		.code = 0x00201000u, .threads = 4, .single_seg = true, .vpm_output_size = 2,
		.vcm_cache_size = 4, .vattr_sizes = { 4, 4 }, .uniforms = pack_seq_vs_u,
		.num_uniforms = 2,
	};
	struct v3d_vs_prog cs = vs;
	cs.code = 0x00202000u;
	struct v3d_vertex_attr attrs[2] = {
		{ .address = 0x00300000u, .stride = 32, .components = 4 },
		{ .address = 0x00300010u, .stride = 32, .components = 4 },
	};
	bool ok = v3d_job_begin(j, t, bcl, rcl, ind);
	for (uint32_t i = 0; ok && i < PACK_SEQ_DRAWS; i++) {
		struct v3d_job_state st;
		uint32_t prim, flat;
		bool indexed;
		pack_seq_state(i, t->width, t->height, &st, &prim, &indexed, &flat);
		struct v3d_fs_prog fs = {
			.code = 0x00200000u, .threads = 4, .num_inputs = 4, .uniforms = pack_seq_fs_u,
			.num_uniforms = 1, .flat_shade_flags = flat,
		};
		struct v3d_job_draw d = {
			.st = &st, .fs = &fs, .vs = &vs, .cs = &cs,
			.vs_storage = { vs_words, 1 }, .attrs = attrs, .num_attrs = 2, .prim = prim,
			.count = 3, .first = i, .indexed = indexed, .index_type = V3D_INDEX_TYPE_16,
			.index_buffer = 0x00310000u, .index_buffer_bytes = 64,
		};
		ok = v3d_job_draw(j, &d, NULL) == V3D_JOB_OK;
	}
	struct v3d_job_clear c = { .rgba8 = 0xff202020u, .z = 1.0f };
	return ok && v3d_job_end(j, &c, NULL, NULL);
}

/* FNV-1a 64 of the bytes a list holds, continuing from `h`. */
static uint64_t pack_seq_fnv(const struct v3d_cl *cl, uint64_t h)
{
	for (uint32_t i = 0; i < cl->used; i++)
		h = (h ^ cl->buf[i]) * 0x100000001b3ull;
	return h;
}

#endif
