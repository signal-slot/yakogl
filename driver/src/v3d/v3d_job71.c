/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * Ported from Mesa 26.2.1 src/gallium/drivers/v3d/ (v3dx_draw.c, v3dx_emit.c,
 * v3dx_rcl.c, v3d_context.c, v3d_uniforms.c), the version 71 paths (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * V3D 7.1 job builder (see v3d_job71.h): the general form of
 * v3d_frame71_tri.c (the board-proven M5 job) and of v3d_frame.c's 7.1
 * clear, with v3d_job.c's draw loop. Each step names the Mesa 26.2.1
 * function it follows for V3D_VERSION 71; the packets are packed by Mesa's
 * own generated 7.1 packers (v3d_packet_v71_pack.h, through v3d_cl.h's
 * v3d_cl_emit). tools/v3d/check71.d/job71.sh compares the clear and the
 * triangle with Mesa's own jobs on the board (tools/v3d/pi5/clear.clif, triangle.clif),
 * and the jobs of the gl_test shapes with Mesa's on the drm-shim at 7.1.10
 * (tools/v3d/pi5/gl_*.clif), naming each difference that remains.
 *
 * At 4.2 this file is empty.
 */
#include "v3d_job71.h"

#if V3D_CL_VER == 71

/* devinfo->clipper_xy_granularity of a version 71 device (v3d_device_info.c) */
#define CLIPPER_XY_GRANULARITY 64.0f

/* What v3d_job71_end still writes after the last draw: FLUSH in the BCL, and
 * the generic tile list in the indirect buffer (TILE_COORDINATES_IMPLICIT 1,
 * up to 10 LOAD_TILE_BUFFER_GENERAL of 13, END_OF_LOADS 1, PRIM_LIST_FORMAT
 * 2, SET_INSTANCEID 5, BRANCH_TO_IMPLICIT_TILE_LIST 2, up to 10
 * STORE_TILE_BUFFER_GENERAL of 13, CLEAR_RENDER_TARGETS 1,
 * END_OF_TILE_MARKER 1, RETURN_FROM_SUB_LIST 1: 274 bytes). A draw is
 * refused as FULL when it would leave less, so a job that took a draw can
 * always be ended. */
#define BCL_END_RESERVE 1u
#define IND_END_RESERVE 288u

static void put_u32(uint8_t *p, uint32_t v)
{
	p[0] = (uint8_t)v;
	p[1] = (uint8_t)(v >> 8);
	p[2] = (uint8_t)(v >> 16);
	p[3] = (uint8_t)(v >> 24);
}

static float abs_f(float v)
{
	return v < 0.0f ? -v : v;
}

static float max_f(float a, float b)
{
	return a > b ? a : b;
}

static float min_f(float a, float b)
{
	return a < b ? a : b;
}

/* A float Mesa assigns to a uint32_t pixel coordinate (v3dx_emit.c): C
 * converts by truncation; a value outside [0, 2^16) or a NaN is refused. */
static bool to_pixel(float v, uint32_t *out)
{
	if (!(v >= 0.0f && v < 65536.0f))
		return false;
	*out = (uint32_t)v;
	return true;
}

/* The Log2 Tile Width/Height fields (v3d_util.h log2_tile_size): 8 pixels 0
 * .. 64 pixels 3. */
static uint32_t log2_tile(uint32_t edge)
{
	return edge == 64 ? 3 : edge == 32 ? 2 : edge == 16 ? 1 : 0;
}

/*
 * The tile buffer's internal type and bpp of one OUTPUT_IMAGE_FORMAT this
 * builder takes (v3dx_format_table.c v3dX(get_internal_type_bpp_for_output_format),
 * which has no version branch: the 4.2 builder's rt_internal rows), and the
 * 7.1 "type and clamping" value of v3dx_rcl.c clamp_for_format_and_type
 * (V3D_VERSION >= 71): the integer types clamped, 16F with NORM clamping for
 * an sRGB format, the others as they are. False for a format with no row.
 */
static bool rt71_internal(uint32_t fmt, uint32_t *bpp, uint32_t *type_clamp)
{
	switch (fmt) {
	case V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16F_CLAMP_NORM;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2:
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA16F:
	case V3D_OUTPUT_IMAGE_FORMAT_R16F:
	case V3D_OUTPUT_IMAGE_FORMAT_R11F_G11F_B10F:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16F;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA8:
	case V3D_OUTPUT_IMAGE_FORMAT_BGR565:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_8;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA8I:
	case V3D_OUTPUT_IMAGE_FORMAT_RG8I:
	case V3D_OUTPUT_IMAGE_FORMAT_R8I:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_8I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI:
	case V3D_OUTPUT_IMAGE_FORMAT_RG8UI:
	case V3D_OUTPUT_IMAGE_FORMAT_R8UI:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_8UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA16I:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RG16I:
	case V3D_OUTPUT_IMAGE_FORMAT_R16I:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI:
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RG16UI:
	case V3D_OUTPUT_IMAGE_FORMAT_R16UI:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_16UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA32I:
		*bpp = V3D_INTERNAL_BPP_128;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RG32I:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_R32I:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32I_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI:
		*bpp = V3D_INTERNAL_BPP_128;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RG32UI:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_R32UI:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32UI_CLAMPED;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RGBA32F:
		*bpp = V3D_INTERNAL_BPP_128;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32F;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_RG32F:
		*bpp = V3D_INTERNAL_BPP_64;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32F;
		return true;
	case V3D_OUTPUT_IMAGE_FORMAT_R32F:
		*bpp = V3D_INTERNAL_BPP_32;
		*type_clamp = V3D_RENDER_TARGET_TYPE_CLAMP_32F;
		return true;
	default:
		return false;
	}
}

/* v3d_util.c v3d_internal_bpp_words */
static uint32_t bpp_words(uint32_t bpp)
{
	return bpp == V3D_INTERNAL_BPP_128 ? 4 : bpp == V3D_INTERNAL_BPP_64 ? 2 : 1;
}

static bool memory_format_ok(uint32_t mf)
{
	return mf <= V3D_MEMORY_FORMAT_UIF_XOR;
}

static bool target_ok(const struct v3d_job71_target *t)
{
	if (t->width == 0 || t->height == 0 || t->width > V3D_JOB71_MAX_SIZE ||
	    t->height > V3D_JOB71_MAX_SIZE || t->rts > V3D_JOB71_MAX_RTS)
		return false;
	if (t->depth_type > V3D_INTERNAL_TYPE_DEPTH_16)
		return false;
	for (uint32_t n = 0; n < t->rts; n++) {
		const struct v3d_job71_rt *rt = &t->rt[n];
		uint32_t bpp, tc;
		if (rt->address &&
		    (!rt71_internal(rt->format, &bpp, &tc) || !memory_format_ok(rt->memory_format)))
			return false;
	}
	if (t->zs.address &&
	    ((t->zs.format != V3D_OUTPUT_IMAGE_FORMAT_D24S8 &&
	      t->zs.format != V3D_OUTPUT_IMAGE_FORMAT_D32F &&
	      t->zs.format != V3D_OUTPUT_IMAGE_FORMAT_D16) ||
	     !memory_format_ok(t->zs.memory_format)))
		return false;
	if (t->s.address &&
	    ((t->s.format != V3D_OUTPUT_IMAGE_FORMAT_S8 && t->s.format != V3D_OUTPUT_IMAGE_FORMAT_D24S8) ||
	     !memory_format_ok(t->s.memory_format)))
		return false;
	return true;
}

bool v3d_job71_begin(struct v3d_job71 *j, const struct v3d_job71_target *t, struct v3d_cl *bcl,
		     struct v3d_cl *rcl, struct v3d_cl *ind)
{
	*j = (struct v3d_job71){ .bcl = bcl, .rcl = rcl, .ind = ind, .t = *t };
	if (!target_ok(t)) {
		j->broken = true;
		return false;
	}

	/* v3d_context.c v3d_get_tile_buffer_size: the bytes per pixel of every
	 * RT with a buffer together (4 per internal bpp word); the tile size
	 * is the first that fits the 7.1 TLB (v3d_util.c v3d_choose_tile_size) */
	uint32_t total_bpp = 0;
	for (uint32_t n = 0; n < t->rts; n++) {
		uint32_t bpp, tc;
		if (t->rt[n].address && rt71_internal(t->rt[n].format, &bpp, &tc))
			total_bpp += 4 * bpp_words(bpp);
	}
	v3d_frame_tiling_gen(71, t->width, t->height, t->rts > 1 ? t->rts : 1, false, 0, total_bpp,
			     &j->tiling);

	/* v3dx_draw.c v3dX(start_binning), V3D_VERSION 71 */
	v3d_cl_emit(bcl, NUMBER_OF_LAYERS, layers) {
		layers.number_of_layers = 1;
	}
	v3d_cl_emit(bcl, TILE_BINNING_MODE_CFG, bin) {
		bin.width_in_pixels = t->width;
		bin.height_in_pixels = t->height;
		bin.log2_tile_width = log2_tile(j->tiling.tile_w);
		bin.log2_tile_height = log2_tile(j->tiling.tile_h);
		bin.tile_allocation_initial_block_size = V3D_TILE_ALLOC_BLOCK_128B;
		bin.tile_allocation_block_size = V3D_TILE_ALLOC_BLOCK_64B;
	}
	v3d_cl_emit(bcl, FLUSH_VCD_CACHE, flush);
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq);
	v3d_cl_emit(bcl, START_TILE_BINNING, start);
	if (!v3d_cl_ok(bcl)) {
		j->broken = true;
		return false;
	}
	return true;
}

/* The records of sampler binding `unit` of this stage, or NULL. */
static const struct v3d_job71_tmu *find_tmu(const struct v3d_job71_tmu *tmu, uint32_t n, uint32_t unit)
{
	for (uint32_t i = 0; tmu && i < n; i++)
		if (tmu[i].unit == unit)
			return &tmu[i];
	return NULL;
}

/*
 * v3d_uniforms.c v3d_write_uniforms for the contents this builder supports,
 * with the 7.1 clipper granularity (the viewport X and Y scales in 1/64
 * pixel). The stream starts 4-byte aligned and one more word than it holds
 * must fit behind it (the QPU prefetches); the next stream starts right
 * after its last word, so an empty stream has the next one's address, as in
 * the board's dump. Any other contents is refused, never written as 0.
 */
static bool write_uniforms(struct v3d_cl *cl, const struct v3d_uniform *u, uint32_t n,
			   const struct v3d_viewport *vp, const struct v3d_uniform_storage *st,
			   const struct v3d_job71_tmu *tmu, uint32_t num_tmu, const struct v3d_wpos *wpos,
			   uint32_t fb_height, uint32_t *addr)
{
	if (n && !u) {
		cl->bad_field = true;
		return false;
	}
	if (!v3d_cl_reserve(cl, 0, 4))
		return false;
	if ((uint64_t)(n + 1) * 4 > cl->size - cl->used) {
		cl->overflow = true;
		return false;
	}
	*addr = v3d_cl_gpu_next(cl);
	uint8_t *p = v3d_cl_reserve(cl, n * 4, 4);
	if (!p)
		return false;
	for (uint32_t i = 0; i < n; i++) {
		uint32_t v;
		switch (u[i].contents) {
		case V3D_QUNIFORM_CONSTANT:
			v = u[i].data;
			break;
		case V3D_QUNIFORM_UNIFORM:
			if (!st->words || u[i].data >= st->count) {
				cl->bad_field = true;
				return false;
			}
			v = st->words[u[i].data];
			break;
		case V3D_QUNIFORM_VIEWPORT_X_SCALE:
			v = v3d_cl_float_bits(vp->scale[0] * CLIPPER_XY_GRANULARITY);
			break;
		case V3D_QUNIFORM_VIEWPORT_Y_SCALE:
			v = v3d_cl_float_bits(vp->scale[1] * CLIPPER_XY_GRANULARITY);
			break;
		case V3D_QUNIFORM_VIEWPORT_Z_OFFSET:
			v = v3d_cl_float_bits(vp->translate[2]);
			break;
		case V3D_QUNIFORM_VIEWPORT_Z_SCALE:
			v = v3d_cl_float_bits(vp->scale[2]);
			break;
		case V3D_QUNIFORM_TEXTURE_WIDTH:
		case V3D_QUNIFORM_TEXTURE_HEIGHT:
		case V3D_QUNIFORM_TEXTURE_DEPTH:
		case V3D_QUNIFORM_TEXTURE_ARRAY_SIZE: {
			/* v3d_uniforms.c: the base level's size, depth or layer
			 * count of binding `data`; not given (0) is refused */
			const struct v3d_job71_tmu *t = find_tmu(tmu, num_tmu, u[i].data);
			v = !t ? 0
			       : u[i].contents == V3D_QUNIFORM_TEXTURE_WIDTH  ? t->width
			       : u[i].contents == V3D_QUNIFORM_TEXTURE_HEIGHT ? t->height
			       : u[i].contents == V3D_QUNIFORM_TEXTURE_DEPTH  ? t->depth
									     : t->array_size;
			if (v == 0) {
				cl->bad_field = true;
				return false;
			}
			break;
		}
		case V3D_QUNIFORM_FB_WPOS_Y_SCALE:
		case V3D_QUNIFORM_FB_WPOS_Y_OFFSET:
		case V3D_QUNIFORM_FB_PNTC_Y_SCALE:
		case V3D_QUNIFORM_FB_PNTC_Y_OFFSET:
			if (!v3d_wpos_uniform(wpos, fb_height, u[i].contents, u[i].data, &v)) {
				cl->bad_field = true;
				return false;
			}
			break;
		default:
			/* the TMU configuration words (40, 41) among them: the
			 * 7.1 texture state is not written yet */
			cl->bad_field = true;
			return false;
		}
		put_u32(p + i * 4, v);
	}
	return true;
}

/* v3dx_emit.c emit_varying_flags (the 4.2 builder's EMIT_VARYING_FLAGS: the
 * 7.1 packets have the same layout). */
#define EMIT_VARYING_FLAGS(bcl, KIND, kind, lo, hi)                                                \
	do {                                                                                       \
		const uint64_t all_ = ((uint64_t)(hi) << 32) | (lo);                               \
		bool any_ = false;                                                                 \
		for (uint32_t k_ = 0; k_ < 3; k_++) {                                              \
			const uint32_t w_ = (uint32_t)(all_ >> (24 * k_)) & 0xffffffu;             \
			if (!w_)                                                                   \
				continue;                                                          \
			v3d_cl_emit(bcl, KIND##_FLAGS, flags) {                                    \
				flags.varying_offset_v0 = k_;                                      \
				flags.action_for_##kind##_flags_of_lower_numbered_varyings =       \
					(any_ || k_ == 0) ? V3D_VARYING_FLAGS_ACTION_UNCHANGED     \
							  : V3D_VARYING_FLAGS_ACTION_ZEROED;       \
				flags.action_for_##kind##_flags_of_higher_numbered_varyings =      \
					any_ ? V3D_VARYING_FLAGS_ACTION_UNCHANGED                  \
					     : V3D_VARYING_FLAGS_ACTION_ZEROED;                    \
				flags.kind##_flags_for_varyings_v024 = w_;                         \
			}                                                                          \
			any_ = true;                                                               \
		}                                                                                  \
		if (!any_)                                                                         \
			v3d_cl_emit(bcl, ZERO_ALL_##KIND##_FLAGS, zero_all);                       \
	} while (0)

static void emit_stencil_face(struct v3d_cl *bcl, const struct v3d_job71_stencil_face *f, bool front,
			      bool back)
{
	v3d_cl_emit(bcl, STENCIL_CFG, c) {
		c.stencil_ref_value = f->ref;
		c.stencil_test_mask = f->value_mask;
		c.stencil_write_mask = f->write_mask;
		c.stencil_test_function = f->func;
		c.stencil_test_fail_op = f->sfail_op;
		c.depth_test_fail_op = f->zfail_op;
		c.stencil_pass_op = f->zpass_op;
		c.front_config = front;
		c.back_config = back;
	}
}

/* v3dx_emit.c v3dX(emit_state), V3D_VERSION 71, with every dirty bit set:
 * every state packet on every draw, in Mesa's order. */
static bool emit_state(struct v3d_job71 *j, const struct v3d_job71_draw *d)
{
	struct v3d_cl *bcl = j->bcl;
	const struct v3d_job71_state *st = d->st;
	const struct v3d_viewport *vp = &st->vp;

	/* CLIP_WINDOW: the viewport, and the scissor or the drawable */
	float vp_minx = -abs_f(vp->scale[0]) + vp->translate[0];
	float vp_maxx = abs_f(vp->scale[0]) + vp->translate[0];
	float vp_miny = -abs_f(vp->scale[1]) + vp->translate[1];
	float vp_maxy = abs_f(vp->scale[1]) + vp->translate[1];
	float lo_x = 0, lo_y = 0, hi_x = (float)j->t.width, hi_y = (float)j->t.height;
	if (st->scissor) {
		lo_x = (float)st->scissor_minx;
		lo_y = (float)st->scissor_miny;
		hi_x = (float)st->scissor_maxx;
		hi_y = (float)st->scissor_maxy;
	}
	uint32_t minx, miny, maxx, maxy;
	if (!to_pixel(max_f(vp_minx, lo_x), &minx) || !to_pixel(max_f(vp_miny, lo_y), &miny) ||
	    !to_pixel(min_f(vp_maxx, hi_x), &maxx) || !to_pixel(min_f(vp_maxy, hi_y), &maxy)) {
		bcl->bad_field = true;
		return false;
	}
	uint32_t cw = 0, ch = 0;
	if (maxx > minx && maxy > miny) {
		cw = maxx - minx;
		ch = maxy - miny;
	}
	v3d_cl_emit(bcl, CLIP_WINDOW, clip) {
		clip.clip_window_left_pixel_coordinate = minx;
		clip.clip_window_bottom_pixel_coordinate = miny;
		clip.clip_window_width_in_pixels = cw;
		clip.clip_window_height_in_pixels = ch;
	}

	/* CFG_BITS at 7.1: no early-Z bits, the Z clipping mode -1..1 (GL's
	 * clip_halfz false, depth_clip_near/far true), no Z clamp. Points and
	 * lines are never culled as forward facing. */
	const bool polygon = d->prim >= V3D_PRIM_TRIANGLES;
	v3d_cl_emit(bcl, CFG_BITS, cfg) {
		cfg.enable_forward_facing_primitive = !polygon || !st->cull_front;
		cfg.enable_reverse_facing_primitive = !st->cull_back;
		cfg.clockwise_primitives = st->front_ccw; /* "seems backwards" */
		cfg.enable_depth_offset = st->offset_tri;
		cfg.depth_test_function = V3D_COMPARE_FUNC_ALWAYS;
		cfg.stencil_enable = st->stencil_enabled;
		cfg.blend_enable = st->blend_enabled;
		cfg.line_rasterization = V3D_LINE_RASTERIZATION_DIAMOND_EXIT;
		cfg.z_clipping_mode = V3D_Z_CLIP_MODE_MIN_ONE_TO_ONE;
		if (st->depth_enabled) {
			cfg.z_updates_enable = st->depth_writemask;
			cfg.depth_test_function = st->depth_func;
		}
	}

	/* DEPTH_OFFSET: f187 factor and units, given as the value the field holds */
	if (st->offset_tri) {
		v3d_cl_emit(bcl, DEPTH_OFFSET, dof) {
			dof.depth_offset_factor = v3d_cl_f187(st->offset_scale);
			dof.depth_offset_units = v3d_cl_f187(st->offset_units);
			dof.limit = st->offset_clamp;
		}
	}

	/* POINT_SIZE (ES has no glPointSize: gallium's 1.0) and LINE_WIDTH */
	v3d_cl_emit(bcl, POINT_SIZE, ps) {
		ps.point_size = 1.0f;
	}
	v3d_cl_emit(bcl, LINE_WIDTH, lw) {
		lw.line_width = st->line_width;
	}

	/* the viewport; the XY scaling in 1/64 pixel at 7.1. A |Z scale| under
	 * 0.01 is Mesa's CLIPPER_Z_SCALE_AND_OFFSET_NO_GUARDBAND, which this
	 * builder cannot write yet: refused. */
	if (!(abs_f(vp->scale[2]) >= 0.01f)) {
		bcl->bad_field = true;
		return false;
	}
	v3d_cl_emit(bcl, CLIPPER_XY_SCALING, xy) {
		xy.viewport_half_width_in_1_64th_of_pixel = vp->scale[0] * CLIPPER_XY_GRANULARITY;
		xy.viewport_half_height_in_1_64th_of_pixel = vp->scale[1] * CLIPPER_XY_GRANULARITY;
	}
	v3d_cl_emit(bcl, CLIPPER_Z_SCALE_AND_OFFSET, z) {
		z.viewport_z_scale_zc_to_zs = vp->scale[2];
		z.viewport_z_offset_zc_to_zs = vp->translate[2];
	}
	float za = vp->translate[2] - vp->scale[2], zb = vp->translate[2] + vp->scale[2];
	v3d_cl_emit(bcl, CLIPPER_Z_MIN_MAX_CLIPPING_PLANES, zmm) {
		zmm.minimum_zw = min_f(za, zb);
		zmm.maximum_zw = max_f(za, zb);
	}
	float fine_x = vp->translate[0], fine_y = vp->translate[1];
	int32_t coarse_x = 0, coarse_y = 0;
	if (fine_x < 0) {
		int32_t blocks = (int32_t)((abs_f(fine_x) + 64 - 1) / 64);
		fine_x += 64.0f * (float)blocks;
		coarse_x -= blocks;
	}
	if (fine_y < 0) {
		int32_t blocks = (int32_t)((abs_f(fine_y) + 64 - 1) / 64);
		fine_y += 64.0f * (float)blocks;
		coarse_y -= blocks;
	}
	uint32_t fx, fy;
	if (!v3d_float_to_u14_8(fine_x, &fx) || !v3d_float_to_u14_8(fine_y, &fy)) {
		bcl->bad_field = true;
		return false;
	}
	v3d_cl_emit(bcl, VIEWPORT_OFFSET, vo) {
		vo.fine_x = (float)fx / 256.0f;
		vo.coarse_x = coarse_x;
		vo.fine_y = (float)fy / 256.0f;
		vo.coarse_y = coarse_y;
	}

	/* BLEND_ENABLES and BLEND_CFG: RT 0's blend for all 8 RTs (no
	 * independent blend: v3d_create_blend_state's (1 << max_rts) - 1) */
	const uint32_t all_rts = (1u << V3D_JOB71_MAX_RTS) - 1;
	if (st->blend_enabled) {
		v3d_cl_emit(bcl, BLEND_ENABLES, on) {
			on.mask = all_rts;
		}
		v3d_cl_emit(bcl, BLEND_CFG, b) {
			b.render_target_mask = all_rts;
			b.color_blend_mode = st->rgb_func;
			b.color_blend_src_factor = st->rgb_src;
			b.color_blend_dst_factor = st->rgb_dst;
			b.alpha_blend_mode = st->alpha_func;
			b.alpha_blend_src_factor = st->alpha_src;
			b.alpha_blend_dst_factor = st->alpha_dst;
		}
	} else {
		v3d_cl_emit(bcl, BLEND_ENABLES, off);
	}
	/* COLOR_WRITE_MASKS: 4 "disable" bits per RT, RT 0's mask for all 8 */
	const uint32_t off = (~st->colormask) & 0xf;
	v3d_cl_emit(bcl, COLOR_WRITE_MASKS, wm) {
		for (uint32_t n = 0; n < V3D_JOB71_MAX_RTS; n++)
			wm.mask |= off << (4 * n);
	}
	const uint16_t *bc = st->blend_color_f16;
	v3d_cl_emit(bcl, BLEND_CONSTANT_COLOR, color) {
		color.red_f16 = bc[0];
		color.green_f16 = bc[1];
		color.blue_f16 = bc[2];
		color.alpha_f16 = bc[3];
	}

	/* STENCIL_CFG: the front face configures the back too unless the back
	 * has its own */
	if (st->stencil_enabled) {
		emit_stencil_face(bcl, &st->stencil_front, true, !st->stencil_back_enabled);
		if (st->stencil_back_enabled)
			emit_stencil_face(bcl, &st->stencil_back, false, true);
	}

	const struct v3d_fs_prog *fs = d->fs;
	EMIT_VARYING_FLAGS(bcl, FLAT_SHADE, flat_shade, fs->flat_shade_flags, fs->flat_shade_flags_hi);
	EMIT_VARYING_FLAGS(bcl, NON_PERSPECTIVE, non_perspective, fs->noperspective_flags,
			   fs->noperspective_flags_hi);
	EMIT_VARYING_FLAGS(bcl, CENTROID, centroid, fs->centroid_flags, fs->centroid_flags_hi);

	/* no transform feedback; the occlusion query's counter or the
	 * zero-address "disable"; SAMPLE_STATE coverage 1.0, mask 0xf (no 4x) */
	v3d_cl_emit(bcl, TRANSFORM_FEEDBACK_SPECS, tf);
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq) {
		oq.address = st->oq_addr;
	}
	v3d_cl_emit(bcl, SAMPLE_STATE, ss) {
		ss.coverage = 1.0f;
		ss.mask = 0xf;
	}
	return true;
}

static bool threads_ok(uint32_t threads)
{
	return threads == 1 || threads == 2 || threads == 4;
}

/* What the 7.1 record can say of a VS or CS: no "separate VPM segments"
 * flag (7.1 has none: a non-zero input size is the separate case, written
 * as it is, as Mesa does), no base-instance read (Mesa 26.2.1 never sets
 * the D0 record's bits for it). */
static bool vs_ok71(const struct v3d_vs_prog *p)
{
	return threads_ok(p->threads) && !p->separate_segments && !p->uses_biid;
}

/* What this builder refuses before it writes anything. */
static bool draw_is_supported(const struct v3d_job71_draw *d)
{
	if (!d->st || !d->fs || !d->vs || !d->cs)
		return false;
	if (!threads_ok(d->fs->threads) || !vs_ok71(d->vs) || !vs_ok71(d->cs))
		return false;
	if (d->fs->num_inputs > 64)
		return false;
	if (d->prim > V3D_PRIM_TRIANGLE_FAN || d->count == 0)
		return false;
	if (d->num_attrs > V3D_MAX_ATTRS || (d->num_attrs && !d->attrs))
		return false;
	for (uint32_t i = d->num_attrs; i < V3D_MAX_ATTRS; i++)
		if (d->vs->vattr_sizes[i] || d->cs->vattr_sizes[i])
			return false; /* the program reads an attribute with no record */
	for (uint32_t i = 0; i < d->num_attrs; i++)
		if (d->attrs[i].components < 1 || d->attrs[i].components > 4)
			return false;
	if (d->indexed && d->index_type > V3D_INDEX_TYPE_32)
		return false;
	return true;
}

/* v3dx_draw.c v3d_draw_vbo for one draw at 7.1: the state, the shader state
 * (v3d_emit_gl_shader_state: the uniform streams, the D0 record and the
 * attribute records in the indirect buffer), then the primitive. */
static bool record_draw(struct v3d_job71 *j, const struct v3d_job71_draw *d,
			struct v3d_job71_draw_layout *lo)
{
	const struct v3d_fs_prog *fs = d->fs;
	const struct v3d_vs_prog *vs = d->vs, *cs = d->cs;
	struct v3d_cl *ind = j->ind, *bcl = j->bcl;

	/* the implicit point varyings exist for points only */
	if (fs->uses_implicit_point_line_varyings && d->prim != V3D_PRIM_POINTS) {
		ind->bad_field = true;
		return false;
	}
	if (!emit_state(j, d))
		return false;

	/* the uniform streams: FS, VS, CS (no default attribute values at 7.1) */
	if (!write_uniforms(ind, fs->uniforms, fs->num_uniforms, &d->st->vp, &d->fs_storage, d->tmu,
			    d->num_tmu, &d->st->wpos, j->t.height, &lo->fs_uniforms) ||
	    !write_uniforms(ind, vs->uniforms, vs->num_uniforms, &d->st->vp, &d->vs_storage,
			    d->vs_tmu, d->num_vs_tmu, &d->st->wpos, j->t.height, &lo->vs_uniforms) ||
	    !write_uniforms(ind, cs->uniforms, cs->num_uniforms, &d->st->vp, &d->vs_storage,
			    d->vs_tmu, d->num_vs_tmu, &d->st->wpos, j->t.height, &lo->cs_uniforms))
		return false;

	/* the record, 32-byte aligned: emit_shader_state_record's D0 branch
	 * (v3d_device_has_draw_index). No NaN propagation, no draw-index or
	 * base-vertex bits (Mesa 26.2.1 sets neither); v3d_compute_vpm_config
	 * without a GS: As 1, Ve 0. */
	if (!v3d_cl_reserve(ind, 0, 32))
		return false;
	lo->record = v3d_cl_gpu_next(ind);
	v3d_cl_emit(ind, GL_SHADER_STATE_RECORD_DRAW_INDEX, r) {
		r.enable_clipping = true;
		r.point_size_in_shaded_vertex_data = d->vs_writes_psiz;
		r.fragment_shader_does_z_writes = fs->writes_z;
		r.turn_off_early_z_test = fs->disable_ez;
		r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2 =
			fs->uses_center_w;
		r.any_shader_reads_hardware_written_primitive_id = fs->uses_pid;
		r.insert_primitive_id_as_first_varying_to_fragment_shader = fs->uses_pid;
		r.do_scoreboard_wait_on_first_thread_switch = fs->lock_scoreboard_on_first_thrsw;
		r.disable_implicit_point_line_varyings = !fs->uses_implicit_point_line_varyings;
		r.number_of_varyings_in_fragment_shader = fs->num_inputs;
		r.coordinate_shader_code_address = cs->code;
		r.vertex_shader_code_address = vs->code;
		r.fragment_shader_code_address = fs->code;
		r.coordinate_shader_input_vpm_segment_size = cs->vpm_input_size;
		r.vertex_shader_input_vpm_segment_size = vs->vpm_input_size;
		r.coordinate_shader_output_vpm_segment_size = cs->vpm_output_size;
		r.vertex_shader_output_vpm_segment_size = vs->vpm_output_size;
		r.coordinate_shader_uniforms_address = lo->cs_uniforms;
		r.vertex_shader_uniforms_address = lo->vs_uniforms;
		r.fragment_shader_uniforms_address = lo->fs_uniforms;
		r.min_coord_shader_input_segments_required_in_play = 1;
		r.min_vertex_shader_input_segments_required_in_play = 1;
		r.min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size = 0;
		r.min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size = 0;
		r.coordinate_shader_4_way_threadable = cs->threads == 4;
		r.vertex_shader_4_way_threadable = vs->threads == 4;
		r.fragment_shader_4_way_threadable = fs->threads == 4;
		r.coordinate_shader_start_in_final_thread_section = cs->single_seg;
		r.vertex_shader_start_in_final_thread_section = vs->single_seg;
		r.fragment_shader_start_in_final_thread_section = fs->single_seg;
		r.vertex_id_read_by_coordinate_shader = cs->uses_vid;
		r.instance_id_read_by_coordinate_shader = cs->uses_iid;
		r.vertex_id_read_by_vertex_shader = vs->uses_vid;
		r.instance_id_read_by_vertex_shader = vs->uses_iid;
		lo->rec = r;
	}

	/* the attribute records, with the GFXH-930 and GFXH-1602 rules for what
	 * the CS must load, and the GFXH-930 dummy record for a program that
	 * reads no attribute: at the indirect buffer's first byte ("valid
	 * address of data whose value will be unused"), as Mesa's 7.1 driver
	 * points it (there is no default-attribute buffer at 7.1) */
	lo->attr_records = v3d_cl_gpu_next(ind);
	uint32_t records = d->num_attrs;
	bool cs_loaded_any = false;
	const bool cs_uses_builtins = cs->uses_iid || cs->uses_vid;
	for (uint32_t i = 0; i < d->num_attrs; i++) {
		v3d_cl_emit(ind, GL_SHADER_STATE_ATTRIBUTE_RECORD, a) {
			a.address = d->attrs[i].address;
			a.stride = d->attrs[i].stride;
			a.vec_size = d->attrs[i].components & 3; /* 0 means 4 */
			a.type = V3D_ATTRIBUTE_FLOAT;
			a.number_of_values_read_by_coordinate_shader = cs->vattr_sizes[i];
			a.number_of_values_read_by_vertex_shader = vs->vattr_sizes[i];
			a.maximum_index = 0xffffff;
			a.instance_divisor = d->attrs[i].divisor > 0xffffu ? 0xffffu : d->attrs[i].divisor;
			if (cs->vattr_sizes[i])
				cs_loaded_any = true;
			if (i == 0 && cs_uses_builtins && !cs_loaded_any) {
				a.number_of_values_read_by_coordinate_shader = 1;
				cs_loaded_any = true;
			} else if (i == d->num_attrs - 1 && !cs_loaded_any) {
				a.number_of_values_read_by_coordinate_shader = 1;
				cs_loaded_any = true;
			}
		}
	}
	if (records == 0) {
		v3d_cl_emit(ind, GL_SHADER_STATE_ATTRIBUTE_RECORD, a) {
			a.address = ind->gpu;
			a.type = V3D_ATTRIBUTE_FLOAT;
			a.stride = 0;
			a.vec_size = 1;
			a.number_of_values_read_by_coordinate_shader = 1;
			a.number_of_values_read_by_vertex_shader = 1;
		}
		records = 1;
	}

	/* back in the BCL: no base vertex or instance, no primitive restart */
	v3d_cl_emit(bcl, VCM_CACHE_SIZE, vcm) {
		vcm.number_of_16_vertex_batches_for_binning = cs->vcm_cache_size;
		vcm.number_of_16_vertex_batches_for_rendering = vs->vcm_cache_size;
	}
	v3d_cl_emit(bcl, GL_SHADER_STATE, state) {
		state.address = lo->record;
		state.number_of_attribute_arrays = records;
	}
	if (d->indexed) {
		v3d_cl_emit(bcl, INDEX_BUFFER_SETUP, ib) {
			ib.address = d->index_buffer;
			ib.size = d->index_buffer_bytes;
		}
		if (d->instances > 1) {
			v3d_cl_emit(bcl, INDEXED_INSTANCED_PRIM_LIST, prim) {
				prim.mode = d->prim;
				prim.index_type = d->index_type;
				prim.instance_length = d->count;
				prim.number_of_instances = d->instances;
				prim.index_offset = d->index_offset;
			}
		} else {
			v3d_cl_emit(bcl, INDEXED_PRIM_LIST, prim) {
				prim.mode = d->prim;
				prim.index_type = d->index_type;
				prim.length = d->count;
				prim.index_offset = d->index_offset;
			}
		}
	} else if (d->instances > 1) {
		v3d_cl_emit(bcl, VERTEX_ARRAY_INSTANCED_PRIMS, prim) {
			prim.mode = d->prim;
			prim.instance_length = d->count;
			prim.number_of_instances = d->instances;
			prim.index_of_first_vertex = d->first;
		}
	} else {
		v3d_cl_emit(bcl, VERTEX_ARRAY_PRIMS, prim) {
			prim.mode = d->prim;
			prim.length = d->count;
			prim.index_of_first_vertex = d->first;
		}
	}
	return true;
}

int v3d_job71_draw(struct v3d_job71 *j, const struct v3d_job71_draw *d,
		   struct v3d_job71_draw_layout *layout)
{
	struct v3d_job71_draw_layout scratch;
	struct v3d_job71_draw_layout *lo = layout ? layout : &scratch;

	if (j->broken || !draw_is_supported(d))
		return V3D_JOB71_REFUSED;

	/* where the lists stand: a draw that does not go in leaves nothing */
	const struct v3d_cl bcl0 = *j->bcl, ind0 = *j->ind;
	bool ok = record_draw(j, d, lo);
	bool bad = j->bcl->bad_field || j->ind->bad_field;
	bool full = j->bcl->overflow || j->ind->overflow ||
		    j->bcl->size - j->bcl->used < BCL_END_RESERVE ||
		    j->ind->size - j->ind->used < IND_END_RESERVE;
	if (!ok || bad || full) {
		*j->bcl = bcl0;
		*j->ind = ind0;
		return (full && !bad) ? V3D_JOB71_FULL : V3D_JOB71_REFUSED;
	}
	j->draws++;
	return V3D_JOB71_OK;
}

/*
 * RENDER_TARGET_PART2 and PART3, packed by Mesa's packers without v3d_cl.h's
 * pack-then-unpack check: their clear-colour fields are 40 and 56 bits
 * wide, and Mesa 26.2.1's __gen_unpack_uint (cle/v3d_packet_helpers.h:51-62)
 * reads at most 32 (an int shift and a 32-bit mask), so the check refuses
 * every value. The range check here is the one the round trip would make:
 * the render target number in 3 bits, the colour bits in their field.
 */
static void emit_rt_part2(struct v3d_cl *cl, uint32_t n, uint64_t mid_bits)
{
	struct V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2 p = {
		V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2_header
	};
	p.clear_color_mid_bits = mid_bits;
	p.render_target_number = n;
	uint8_t b[V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2_length];
	V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2_pack(NULL, b, &p);
	v3d_cl_put(cl, b, sizeof(b), n < V3D_JOB71_MAX_RTS && mid_bits >> 40 == 0);
}

static void emit_rt_part3(struct v3d_cl *cl, uint32_t n, uint64_t top_bits)
{
	struct V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3 p = {
		V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3_header
	};
	p.clear_color_top_bits = top_bits;
	p.render_target_number = n;
	uint8_t b[V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3_length];
	V3D71_TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3_pack(NULL, b, &p);
	v3d_cl_put(cl, b, sizeof(b), n < V3D_JOB71_MAX_RTS && top_bits >> 56 == 0);
}

/* The buffer number a load or store of `aspects` (V3D_JOB71_ZS_* bits, 0:
 * every aspect) of the target's depth buffer moves (v3dx_rcl.c
 * zs_buffer_from_pipe_bits: DEPTH | STENCIL ZSTENCIL, DEPTH Z, STENCIL
 * STENCIL). Only a D24S8 `zs` without a separate stencil holds a stencil.
 * False for an aspect the buffer does not hold. The load and store buffer
 * numbers are the same values (v3d_packet.xml "Load/Store buffer"). */
static bool zs_buffer(const struct v3d_job71_target *t, uint32_t aspects, uint32_t *buffer)
{
	const bool has_stencil = !t->s.address && t->zs.format == V3D_OUTPUT_IMAGE_FORMAT_D24S8;
	if (aspects == 0)
		aspects = V3D_JOB71_ZS_DEPTH | (has_stencil ? V3D_JOB71_ZS_STENCIL : 0);
	if (aspects > (V3D_JOB71_ZS_DEPTH | V3D_JOB71_ZS_STENCIL) ||
	    ((aspects & V3D_JOB71_ZS_STENCIL) && !has_stencil))
		return false;
	if (aspects == (V3D_JOB71_ZS_DEPTH | V3D_JOB71_ZS_STENCIL))
		*buffer = V3D_LOAD_BUFFER_ZSTENCIL;
	else if (aspects == V3D_JOB71_ZS_DEPTH)
		*buffer = V3D_LOAD_BUFFER_Z;
	else
		*buffer = V3D_LOAD_BUFFER_STENCIL;
	return true;
}

/* v3dx_rcl.c v3d_rcl_emit_generic_per_tile_list, written into `list`: the
 * loads (v3d_rcl_emit_loads: the RTs in order, then a separate stencil, then
 * Z, Z/S or the D24S8's stencil: `zs_load_buf`), the branch to the binned
 * primitives, the stores (v3d_rcl_emit_stores: the RTs in order, then Z, Z/S
 * or stencil (`zs_store_buf`), then the separate stencil; a list with nothing
 * to store stores buffer NONE), and the 7.1 clear of the tile buffer for the
 * next tile, CLEAR_RENDER_TARGETS. */
static void emit_tile_list(const struct v3d_job71 *j, const struct v3d_job71_clear *c,
			   uint32_t zs_load_buf, uint32_t zs_store_buf, struct v3d_cl *list,
			   uint32_t *start, uint32_t *end)
{
	const struct v3d_job71_target *t = &j->t;
	*start = v3d_cl_gpu_next(list);
	v3d_cl_emit(list, TILE_COORDINATES_IMPLICIT, coords);
	for (uint32_t n = 0; n < t->rts; n++) {
		const struct v3d_job71_rt *rt = &t->rt[n];
		if (!rt->address || !(c->load_rts & (1u << n)))
			continue;
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, ld) {
			ld.buffer_to_load = V3D_LOAD_BUFFER_RT0 + n;
			ld.memory_format = rt->memory_format;
			ld.input_image_format = rt->format;
			ld.height_in_ub_or_stride = rt->height_in_ub_or_stride;
			ld.address = rt->address;
			ld.r_b_swap = rt->r_b_swap;
			ld.force_alpha_1 = rt->force_alpha_1;
			ld.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	if (t->s.address && c->s_load) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, sl) {
			sl.buffer_to_load = V3D_LOAD_BUFFER_STENCIL;
			sl.memory_format = t->s.memory_format;
			sl.input_image_format = t->s.format;
			sl.height_in_ub_or_stride = t->s.height_in_ub;
			sl.address = t->s.address;
			sl.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	if (t->zs.address && c->zs_load) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, zl) {
			zl.buffer_to_load = zs_load_buf;
			zl.memory_format = t->zs.memory_format;
			zl.input_image_format = t->zs.format;
			zl.height_in_ub_or_stride = t->zs.height_in_ub;
			zl.address = t->zs.address;
			zl.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	v3d_cl_emit(list, END_OF_LOADS, end_loads);
	v3d_cl_emit(list, PRIM_LIST_FORMAT, fmt) {
		fmt.primitive_type = V3D_PRIM_LIST_TRIANGLES;
	}
	v3d_cl_emit(list, SET_INSTANCEID, iid);
	v3d_cl_emit(list, BRANCH_TO_IMPLICIT_TILE_LIST, branch);

	uint32_t stores = 0;
	for (uint32_t n = 0; n < t->rts; n++) {
		const struct v3d_job71_rt *rt = &t->rt[n];
		if (!rt->address)
			continue;
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, st) {
			st.buffer_to_store = V3D_STORE_BUFFER_RT0 + n;
			st.memory_format = rt->memory_format;
			st.output_image_format = rt->format;
			st.height_in_ub_or_stride = rt->height_in_ub_or_stride;
			st.address = rt->address;
			st.r_b_swap = rt->r_b_swap;
			st.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
			st.clear_buffer_being_stored = false;
		}
	}
	if (t->zs.address && c->zs_store) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, zs) {
			zs.buffer_to_store = zs_store_buf;
			zs.memory_format = t->zs.memory_format;
			zs.output_image_format = t->zs.format;
			zs.height_in_ub_or_stride = t->zs.height_in_ub;
			zs.address = t->zs.address;
			zs.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	if (t->s.address && c->s_store) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, ss) {
			ss.buffer_to_store = V3D_STORE_BUFFER_STENCIL;
			ss.memory_format = t->s.memory_format;
			ss.output_image_format = t->s.format;
			ss.height_in_ub_or_stride = t->s.height_in_ub;
			ss.address = t->s.address;
			ss.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	if (!stores)
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, none) {
			none.buffer_to_store = V3D_STORE_BUFFER_NONE;
		}
	/* job->clear_tlb: every tile starts from the clear values. Written
	 * whatever the job loads: a loaded buffer is loaded over it. */
	v3d_cl_emit(list, CLEAR_RENDER_TARGETS, clear);
	v3d_cl_emit(list, END_OF_TILE_MARKER, eot);
	v3d_cl_emit(list, RETURN_FROM_SUB_LIST, ret);
	*end = v3d_cl_gpu_next(list);
}

bool v3d_job71_end(struct v3d_job71 *j, const struct v3d_job71_clear *c, uint32_t *tile_list_start,
		   uint32_t *tile_list_end)
{
	if (j->broken || !c)
		return false;
	const struct v3d_job71_target *t = &j->t;
	/* a load or store of a buffer the target does not have is a caller's
	 * mistake, not a no-op */
	if ((c->zs_load || c->zs_store) && !t->zs.address) {
		j->broken = true;
		return false;
	}
	if ((c->s_load || c->s_store) && !t->s.address) {
		j->broken = true;
		return false;
	}
	/* the Z/S buffer numbers of the load and the store: an aspect the
	 * buffer does not hold is the same kind of mistake */
	uint32_t zs_load_buf = 0, zs_store_buf = 0;
	if ((c->zs_load && !zs_buffer(t, c->zs_load_aspects, &zs_load_buf)) ||
	    (c->zs_store && !zs_buffer(t, c->zs_store_aspects, &zs_store_buf))) {
		j->broken = true;
		return false;
	}

	/* v3dX(bcl_epilogue): no TF, no primitive counts */
	v3d_cl_emit(j->bcl, FLUSH, flush);

	uint32_t list_start, list_end;
	emit_tile_list(j, c, zs_load_buf, zs_store_buf, j->ind, &list_start, &list_end);
	if (tile_list_start)
		*tile_list_start = list_start;
	if (tile_list_end)
		*tile_list_end = list_end;

	/* v3dX(emit_rcl), V3D_VERSION 71, and emit_render_layer, layer 0 */
	struct v3d_cl *rcl = j->rcl;
	const struct v3d_tiling *ti = &j->tiling;
	const bool zs_kept = c->zs_load || c->zs_store || c->s_load || c->s_store;
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_COMMON, common) {
		common.number_of_render_targets = t->rts > 1 ? t->rts : 1;
		common.image_width_pixels = t->width;
		common.image_height_pixels = t->height;
		common.log2_tile_width = log2_tile(ti->tile_w);
		common.log2_tile_height = log2_tile(ti->tile_h);
		common.internal_depth_type = t->depth_type;
		/* early Z off (see v3d_job71.h) */
		common.early_z_disable = true;
		common.early_depth_stencil_clear = c->zs_cleared && !zs_kept;
	}
	/* The render targets (v3dx_rcl.c:842-889): each with a buffer gets its
	 * clear colour's low 32 bits, internal bpp, type and clamping, its row
	 * stride in 128-bit units (v3d_compute_rt_row_row_stride_128_bits) and
	 * its TLB base, the bases packed one after the other; PART2 and PART3
	 * carry the rest of the clear colour at 64 and 128 bpp. An RT with no
	 * buffer, and RT 0 of a job with none, is configured unused (stride
	 * 1). */
	if (t->rts == 0) {
		v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1, rt) {
			rt.stride = 1;
		}
	}
	uint32_t base = 0;
	for (uint32_t n = 0; n < t->rts; n++) {
		if (!t->rt[n].address) {
			v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1, rt) {
				rt.render_target_number = n;
				rt.stride = 1;
			}
			continue;
		}
		uint32_t bpp = 0, tc = 0;
		rt71_internal(t->rt[n].format, &bpp, &tc); /* checked at begin */
		const bool load = (c->load_rts >> n) & 1u;
		const uint32_t *w = c->color[n];
		const uint32_t w0 = load ? 0 : w[0], w1 = load ? 0 : w[1];
		const uint32_t w2 = load ? 0 : w[2], w3 = load ? 0 : w[3];
		const uint32_t stride = ti->tile_w * bpp_words(bpp) / 2;
		v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1, rt) {
			rt.clear_color_low_bits = w0;
			rt.internal_bpp = bpp;
			rt.internal_type_and_clamping = tc;
			rt.stride = stride;
			rt.base_address = base;
			rt.render_target_number = n;
		}
		base += ti->tile_h * stride / 8;
		/* 40 bits (32 + 8) and 56 bits (24 + 32) */
		if (bpp >= V3D_INTERNAL_BPP_64)
			emit_rt_part2(rcl, n, (uint64_t)w1 | ((uint64_t)(w2 & 0xff) << 32));
		if (bpp >= V3D_INTERNAL_BPP_128)
			emit_rt_part3(rcl, n, ((uint64_t)(w2 & 0xffffff00u) >> 8) | ((uint64_t)w3 << 24));
	}
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, zs) {
		zs.z_clear_value = c->z;
		zs.stencil_clear_value = c->stencil;
	}
	v3d_cl_emit(rcl, TILE_LIST_INITIAL_BLOCK_SIZE, init) {
		init.size_of_first_block_in_chained_tile_lists = V3D_TILE_ALLOC_BLOCK_128B;
		init.use_auto_chained_tile_lists = true;
	}
	v3d_cl_emit(rcl, MULTICORE_RENDERING_TILE_LIST_SET_BASE, lbase) {
		lbase.tile_list_set_number = 0;
		lbase.address = t->tile_alloc;
	}
	v3d_cl_emit(rcl, MULTICORE_RENDERING_SUPERTILE_CFG, sc) {
		sc.supertile_width_in_tiles = ti->supertile_w;
		sc.supertile_height_in_tiles = ti->supertile_h;
		sc.total_frame_width_in_supertiles = ti->frame_w_supertiles;
		sc.total_frame_height_in_supertiles = ti->frame_h_supertiles;
		sc.total_frame_width_in_tiles = ti->tiles_x;
		sc.total_frame_height_in_tiles = ti->tiles_y;
		sc.number_of_bin_tile_lists = 1;
	}
	/* the initial clear and the GFXH-1742 workaround: two dummy tiles with
	 * a store of buffer NONE each, the clear only in the first (no
	 * double-buffer mode) */
	for (int i = 0; i < 2; i++) {
		v3d_cl_emit(rcl, TILE_COORDINATES, coords);
		v3d_cl_emit(rcl, END_OF_LOADS, end_loads);
		v3d_cl_emit(rcl, STORE_TILE_BUFFER_GENERAL, none) {
			none.buffer_to_store = V3D_STORE_BUFFER_NONE;
		}
		if (i == 0)
			v3d_cl_emit(rcl, CLEAR_RENDER_TARGETS, clear_rt);
		v3d_cl_emit(rcl, END_OF_TILE_MARKER, eot);
	}
	v3d_cl_emit(rcl, FLUSH_VCD_CACHE, flush);
	v3d_cl_emit(rcl, START_ADDRESS_OF_GENERIC_TILE_LIST, gen) {
		gen.start = list_start;
		gen.end = list_end;
	}
	/* every supertile covering the frame */
	uint32_t stw = ti->tile_w * ti->supertile_w, sth = ti->tile_h * ti->supertile_h;
	for (uint32_t y = 0; y <= (t->height - 1) / sth; y++) {
		for (uint32_t x = 0; x <= (t->width - 1) / stw; x++) {
			v3d_cl_emit(rcl, SUPERTILE_COORDINATES, coords) {
				coords.column_number_in_supertiles = x;
				coords.row_number_in_supertiles = y;
			}
		}
	}
	v3d_cl_emit(rcl, END_OF_RENDERING, end);

	if (!v3d_cl_ok(j->bcl) || !v3d_cl_ok(rcl) || !v3d_cl_ok(j->ind)) {
		j->broken = true;
		return false;
	}
	return true;
}

void v3d_job71_alloc_sizes(const struct v3d_job71 *j, uint32_t page_size, uint32_t *tile_alloc_size,
			   uint32_t *tile_state_size)
{
	v3d_frame_alloc_sizes(&j->tiling, j->draws, page_size, tile_alloc_size, tile_state_size);
}

#endif /* V3D_CL_VER == 71 */
