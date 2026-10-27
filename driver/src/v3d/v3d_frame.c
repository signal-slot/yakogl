/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * Ported from Mesa 26.2.1 src/gallium/drivers/v3d/ (v3dx_draw.c, v3dx_emit.c,
 * v3dx_rcl.c, v3d_job.c) (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * V3D 4.2 frame builder (see v3d_frame.h). Each step names the Mesa 26.2.1
 * function it follows; the packets are packed by Mesa's own generated
 * packers (v3d_packet_v42_pack.h, through v3d_cl.h's v3d_cl_emit).
 */
#include "v3d_frame.h"

#define V3D_MAX_SUPERTILES 256               /* v3d_limits.h */
#define V3D_TILE_ALLOC_INITIAL_BLOCK 128     /* bytes, v3d_limits.h */

static uint32_t div_round_up(uint32_t a, uint32_t b)
{
	return (a + b - 1) / b;
}

static uint32_t align_up(uint32_t v, uint32_t a)
{
	return (v + a - 1) / a * a;
}

void v3d_frame_tiling(uint32_t width, uint32_t height, struct v3d_tiling *t)
{
	v3d_frame_tiling_rts(width, height, 1, t);
}

void v3d_frame_tiling_rts(uint32_t width, uint32_t height, uint32_t rts, struct v3d_tiling *t)
{
	v3d_frame_tiling_ms(width, height, rts, false, t);
}

void v3d_frame_tiling_ms(uint32_t width, uint32_t height, uint32_t rts, bool msaa, struct v3d_tiling *t)
{
	v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t);
}

void v3d_frame_tiling_bpp(uint32_t width, uint32_t height, uint32_t rts, bool msaa, uint32_t max_bpp,
			  struct v3d_tiling *t)
{
	/* v3d_choose_tile_size, the 4.x branch (v3d_util.c:122-200, Mesa
	 * b739817b): from 64x64, one step down the table for 2 render targets
	 * and two for 3-4, two more for 4x MSAA ("if (msaa) idx += 2"), no
	 * double buffer, and one more per step of the largest internal bpp
	 * ("tile_size_index += max_internal_bpp": 32 bpp adds nothing, 64 one,
	 * 128 two; Mesa's table runs down to 8x8). The TILE_BINNING_MODE_CFG
	 * packet carries only the RT count, the max bpp and the MSAA bit, and
	 * the hardware takes the tile size from them, so this rule is the
	 * hardware's, not a choice. */
	static const uint8_t sizes[][2] = { { 64, 64 }, { 64, 32 }, { 32, 32 }, { 32, 16 },
					    { 16, 16 }, { 16, 8 },  { 8, 8 } };
	uint32_t idx = (rts > 2 ? 2 : rts > 1 ? 1 : 0) + (msaa ? 2 : 0) + (max_bpp > 2 ? 2 : max_bpp);
	t->tile_w = sizes[idx][0];
	t->tile_h = sizes[idx][1];
	t->tiles_x = div_round_up(width, t->tile_w);
	t->tiles_y = div_round_up(height, t->tile_h);

	/* emit_render_layer: grow the supertile until the frame has at most
	 * V3D_MAX_SUPERTILES of them and neither side reaches the limit. */
	uint32_t sw = 1, sh = 1;
	for (;;) {
		uint32_t fw = div_round_up(t->tiles_x, sw);
		uint32_t fh = div_round_up(t->tiles_y, sh);
		if (fw < V3D_MAX_SUPERTILES && fh < V3D_MAX_SUPERTILES &&
		    fw * fh <= V3D_MAX_SUPERTILES) {
			t->frame_w_supertiles = fw;
			t->frame_h_supertiles = fh;
			break;
		}
		if (sw < sh)
			sw++;
		else
			sh++;
	}
	t->supertile_w = sw;
	t->supertile_h = sh;
}

void v3d_frame_alloc_sizes(const struct v3d_tiling *t, uint32_t draws, uint32_t page_size,
			   uint32_t *tile_alloc_size, uint32_t *tile_state_size)
{
	/* v3d_tile_alloc_sizes, one layer */
	uint32_t tiles_size = t->tiles_x * t->tiles_y * V3D_TILE_ALLOC_INITIAL_BLOCK;
	uint32_t size = align_up(tiles_size, 4096) + 8192;
	uint32_t pool = tiles_size * draws / 2;
	size += pool < 512 * 1024 ? pool : 512 * 1024;
	*tile_alloc_size = align_up(size, page_size);
	*tile_state_size = t->tiles_x * t->tiles_y * 256;
}

/* v3dx_draw.c v3dX(start_binning), V3D_VERSION 42: the BCL prologue. */
static void emit_start_binning(uint32_t width, uint32_t height, struct v3d_cl *bcl)
{
	v3d_cl_emit(bcl, TILE_BINNING_MODE_CFG, bin) {
		bin.width_in_pixels = width;
		bin.height_in_pixels = height;
		bin.number_of_render_targets = 1;
		bin.maximum_bpp_of_all_render_targets = V3D_INTERNAL_BPP_32;
		bin.tile_allocation_initial_block_size = V3D_TILE_ALLOC_BLOCK_128B;
		bin.tile_allocation_block_size = V3D_TILE_ALLOC_BLOCK_64B;
	}
	v3d_cl_emit(bcl, FLUSH_VCD_CACHE, flush);
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq);
	v3d_cl_emit(bcl, START_TILE_BINNING, start);
}

/* v3dx_rcl.c v3d_rcl_emit_generic_per_tile_list: written into `list` at its
 * current position; returns its start and end. */
static void emit_tile_list(const struct v3d_clear_frame *f, struct v3d_cl *list,
			   uint32_t *start, uint32_t *end)
{
	*start = v3d_cl_gpu_next(list);
	v3d_cl_emit(list, TILE_COORDINATES_IMPLICIT, coords);
	/* no loads: the whole target is cleared */
	v3d_cl_emit(list, END_OF_LOADS, end_loads);
	v3d_cl_emit(list, PRIM_LIST_FORMAT, fmt) {
		fmt.primitive_type = V3D_PRIM_LIST_TRIANGLES;
	}
	v3d_cl_emit(list, SET_INSTANCEID, iid);
	v3d_cl_emit(list, BRANCH_TO_IMPLICIT_TILE_LIST, branch);
	/* v3d_rcl_emit_stores -> store_general, RT0, raster RGBA8 */
	v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, st) {
		st.buffer_to_store = V3D_STORE_BUFFER_RT0;
		st.memory_format = V3D_MEMORY_FORMAT_RASTER;
		st.output_image_format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
		st.height_in_ub_or_stride = f->stride;
		st.address = f->target;
		st.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
	}
	/* job->clear_tlb: clear after the store, ready for the next tile */
	v3d_cl_emit(list, CLEAR_TILE_BUFFERS, clear) {
		clear.clear_all_render_targets = true;
		clear.clear_z_stencil_buffer = true;
	}
	v3d_cl_emit(list, END_OF_TILE_MARKER, eot);
	v3d_cl_emit(list, RETURN_FROM_SUB_LIST, ret);
	*end = v3d_cl_gpu_next(list);
}

/* v3dx_rcl.c v3dX(emit_rcl) (v42) and emit_render_layer, layer 0. The draw
 * area is the whole frame in both frames: v3d_tlb_clear sets draw_min/max to
 * the framebuffer (v3dx_draw.c:1787-1790), and a draw only widens it. */
static void emit_rcl(const struct v3d_clear_frame *f, const struct v3d_tiling *t,
		     uint32_t list_start, uint32_t list_end, struct v3d_cl *rcl)
{
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_COMMON, common) {
		common.number_of_render_targets = 1;
		common.image_width_pixels = f->width;
		common.image_height_pixels = f->height;
		common.maximum_bpp_of_all_render_targets = V3D_INTERNAL_BPP_32;
		/* no depth buffer: v3d_update_job_ez sets first_ez_state to
		 * V3D_EZ_DISABLED (v3dx_draw.c:941-945), and emit_rcl then
		 * disables early Z (v3dx_rcl.c:730-747) */
		common.early_z_disable = true;
	}
	/* R8G8B8A8_UNORM clear colour, internal type 8: 32 bits (v3d_tlb_clear) */
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1, clear) {
		clear.render_target_number = 0;
		clear.clear_color_low_32_bits = f->clear_rgba8;
	}
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_COLOR, color) {
		color.render_target_0_internal_bpp = V3D_INTERNAL_BPP_32;
		color.render_target_0_internal_type = V3D_INTERNAL_TYPE_8;
		color.render_target_0_clamp = V3D_RENDER_TARGET_CLAMP_NONE;
	}
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, zs) {
		zs.z_clear_value = 0.0f;
		zs.stencil_clear_value = 0;
	}
	v3d_cl_emit(rcl, TILE_LIST_INITIAL_BLOCK_SIZE, init) {
		init.size_of_first_block_in_chained_tile_lists = V3D_TILE_ALLOC_BLOCK_128B;
		init.use_auto_chained_tile_lists = true;
	}

	v3d_cl_emit(rcl, MULTICORE_RENDERING_TILE_LIST_SET_BASE, base) {
		base.tile_list_set_number = 0;
		base.address = f->tile_alloc;
	}
	v3d_cl_emit(rcl, MULTICORE_RENDERING_SUPERTILE_CFG, sc) {
		sc.supertile_width_in_tiles = t->supertile_w;
		sc.supertile_height_in_tiles = t->supertile_h;
		sc.total_frame_width_in_supertiles = t->frame_w_supertiles;
		sc.total_frame_height_in_supertiles = t->frame_h_supertiles;
		sc.total_frame_width_in_tiles = t->tiles_x;
		sc.total_frame_height_in_tiles = t->tiles_y;
		sc.number_of_bin_tile_lists = 1;
	}
	/* the initial clear and the GFXH-1742 workaround: two dummy tiles, the
	 * clear only in the first (no double-buffer mode) */
	for (int i = 0; i < 2; i++) {
		v3d_cl_emit(rcl, TILE_COORDINATES, coords);
		v3d_cl_emit(rcl, END_OF_LOADS, end_loads);
		v3d_cl_emit(rcl, STORE_TILE_BUFFER_GENERAL, none) {
			none.buffer_to_store = V3D_STORE_BUFFER_NONE;
		}
		if (i == 0) {
			v3d_cl_emit(rcl, CLEAR_TILE_BUFFERS, clear_tlb) {
				clear_tlb.clear_all_render_targets = true;
				clear_tlb.clear_z_stencil_buffer = true;
			}
		}
		v3d_cl_emit(rcl, END_OF_TILE_MARKER, eot);
	}
	v3d_cl_emit(rcl, FLUSH_VCD_CACHE, flush);
	v3d_cl_emit(rcl, START_ADDRESS_OF_GENERIC_TILE_LIST, gen) {
		gen.start = list_start;
		gen.end = list_end;
	}
	/* every supertile covering the frame */
	uint32_t stw = t->tile_w * t->supertile_w, sth = t->tile_h * t->supertile_h;
	for (uint32_t y = 0; y <= (f->height - 1) / sth; y++) {
		for (uint32_t x = 0; x <= (f->width - 1) / stw; x++) {
			v3d_cl_emit(rcl, SUPERTILE_COORDINATES, coords) {
				coords.column_number_in_supertiles = x;
				coords.row_number_in_supertiles = y;
			}
		}
	}
	v3d_cl_emit(rcl, END_OF_RENDERING, end);
}

bool v3d_frame_build_clear(const struct v3d_clear_frame *f, struct v3d_cl *bcl, struct v3d_cl *rcl,
			   struct v3d_cl *tile_list)
{
	if (f->width == 0 || f->height == 0)
		return false;

	struct v3d_tiling t;
	v3d_frame_tiling(f->width, f->height, &t);

	emit_start_binning(f->width, f->height, bcl);
	/* no draws; FLUSH makes the binner cap every tile list with a return
	 * (v3dx_job.c v3dX(bcl_epilogue)) */
	v3d_cl_emit(bcl, FLUSH, flush);

	uint32_t list_start, list_end;
	emit_tile_list(f, tile_list, &list_start, &list_end);
	emit_rcl(f, &t, list_start, list_end, rcl);

	return v3d_cl_ok(bcl) && v3d_cl_ok(rcl) && v3d_cl_ok(tile_list);
}

/* ------------------------------------------------------------------------ */
/* M5 */

void v3d_viewport_gl(float x, float y, float w, float h, float n, float f, bool y0_top,
		     uint32_t fb_height, struct v3d_viewport *vp)
{
	/* _mesa_get_viewport_xform, GL_LOWER_LEFT, GL_NEGATIVE_ONE_TO_ONE
	 * (main/viewport.c:538-556; n and f are doubles there) */
	float half_width = 0.5f * w, half_height = 0.5f * h;
	vp->scale[0] = half_width;
	vp->translate[0] = half_width + x;
	vp->scale[1] = half_height;
	vp->translate[1] = half_height + y;
	vp->scale[2] = (float)(0.5 * ((double)f - (double)n));
	vp->translate[2] = (float)(0.5 * ((double)n + (double)f));
	/* st_update_viewport: a window whose coordinate system is upside down */
	if (y0_top) {
		vp->scale[1] *= -1;
		vp->translate[1] = (float)fb_height - vp->translate[1];
	}
}

void v3d_wpos_gl(bool y0_top, uint32_t fb_height, struct v3d_wpos *w)
{
	/* STATE_FB_WPOS_Y_TRANSFORM's XY pair (prog_statevars.c:736-751), with
	 * y0_top standing for the draw buffer's FlipY */
	w->y_scale = y0_top ? -1.0f : 1.0f;
	w->y_offset = y0_top ? (float)fb_height : 0.0f;
}

bool v3d_wpos_uniform(const struct v3d_wpos *w, uint32_t fb_height, uint32_t contents,
		      uint32_t data, uint32_t *out)
{
	if (data != 0)
		return false;
	float offset;
	if (w->y_scale == 1.0f)
		offset = 0.0f;
	else if (w->y_scale == -1.0f)
		offset = (float)fb_height;
	else
		return false; /* zeroed (absent), or not a y transform */
	if (w->y_offset != offset)
		return false;
	if (contents == V3D_QUNIFORM_FB_WPOS_Y_SCALE)
		*out = v3d_cl_float_bits(w->y_scale);
	else if (contents == V3D_QUNIFORM_FB_WPOS_Y_OFFSET)
		*out = v3d_cl_float_bits(w->y_offset);
	else if (contents == V3D_QUNIFORM_FB_PNTC_Y_SCALE)
		*out = v3d_cl_float_bits(w->y_scale); /* flip_y ? -1 : 1 */
	else if (contents == V3D_QUNIFORM_FB_PNTC_Y_OFFSET)
		*out = v3d_cl_float_bits(w->y_scale == -1.0f ? 1.0f : 0.0f);
	else
		return false;
	return true;
}

static float abs_f(float v)
{
	return v < 0.0f ? -v : v;
}

/* A float that Mesa assigns to a uint32_t pixel coordinate (v3dx_emit.c:246-257).
 * C converts by truncation; a value outside [0, 2^16) (a 16-bit field) or a
 * NaN is refused rather than converted. */
static bool to_pixel(float v, uint32_t *out)
{
	if (!(v >= 0.0f && v < 65536.0f))
		return false;
	*out = (uint32_t)v;
	return true;
}

static void put_u32(uint8_t *p, uint32_t v)
{
	p[0] = (uint8_t)v;
	p[1] = (uint8_t)(v >> 8);
	p[2] = (uint8_t)(v >> 16);
	p[3] = (uint8_t)(v >> 24);
}

/*
 * v3d_uniforms.c v3d_write_uniforms for the contents M5 supports. The stream
 * starts 4-byte aligned; space for one more word than the stream holds must
 * exist behind it (the QPU prefetches the next uniform, v3d_uniforms.c:228-236), but the
 * next stream starts right after the last word, as in Mesa. Any other
 * contents is refused, never written as 0; so is a QUNIFORM_UNIFORM whose
 * word is not in the stage's storage.
 */
static bool write_uniforms(struct v3d_cl *cl, const struct v3d_uniform *u, uint32_t n,
			   const struct v3d_viewport *vp, const struct v3d_uniform_storage *st,
			   const struct v3d_wpos *wpos, uint32_t fb_height, uint32_t *addr)
{
	if (n && !u)
		return false;
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
			/* v3d_uniforms.c:251-252: gallium_uniforms[data] */
			if (!st->words || u[i].data >= st->count) {
				cl->bad_field = true;
				return false;
			}
			v = st->words[u[i].data];
			break;
		case V3D_QUNIFORM_VIEWPORT_X_SCALE:
			/* devinfo->clipper_xy_granularity is 256.0f on 4.2 */
			v = v3d_cl_float_bits(vp->scale[0] * 256.0f);
			break;
		case V3D_QUNIFORM_VIEWPORT_Y_SCALE:
			v = v3d_cl_float_bits(vp->scale[1] * 256.0f);
			break;
		case V3D_QUNIFORM_VIEWPORT_Z_OFFSET:
			v = v3d_cl_float_bits(vp->translate[2]);
			break;
		case V3D_QUNIFORM_VIEWPORT_Z_SCALE:
			v = v3d_cl_float_bits(vp->scale[2]);
			break;
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
			cl->bad_field = true;
			return false;
		}
		put_u32(p + i * 4, v);
	}
	return true;
}

/* v3dx_emit.c v3dX(emit_state) for the first draw of a job, where every
 * dirty bit is set (v3d_job.c:436). The GL state is the default one with no
 * depth/stencil buffer, no scissor, no blending, all channels written, no
 * culling, no transform feedback, no query, no MSAA. */
static bool emit_state(const struct v3d_triangle_frame *f, struct v3d_cl *bcl)
{
	const struct v3d_viewport *vp = &f->vp;

	/* CLIP_WINDOW (v3dx_emit.c:229-266), no scissor */
	float vp_minx = -abs_f(vp->scale[0]) + vp->translate[0];
	float vp_maxx = abs_f(vp->scale[0]) + vp->translate[0];
	float vp_miny = -abs_f(vp->scale[1]) + vp->translate[1];
	float vp_maxy = abs_f(vp->scale[1]) + vp->translate[1];
	float w = (float)f->clear.width, h = (float)f->clear.height;
	uint32_t minx, miny, maxx, maxy;
	if (!to_pixel(vp_minx > 0 ? vp_minx : 0, &minx) ||
	    !to_pixel(vp_miny > 0 ? vp_miny : 0, &miny) ||
	    !to_pixel(vp_maxx < w ? vp_maxx : w, &maxx) ||
	    !to_pixel(vp_maxy < h ? vp_maxy : h, &maxy))
		return false;
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

	/* CFG_BITS (v3dx_emit.c:294-405) */
	v3d_cl_emit(bcl, CFG_BITS, cfg) {
		cfg.enable_forward_facing_primitive = true; /* no culling, not discarding */
		cfg.enable_reverse_facing_primitive = true;
		cfg.clockwise_primitives = f->front_ccw; /* "seems backwards", v3dx_emit.c:315-319 */
		cfg.depth_test_function = V3D_COMPARE_FUNC_ALWAYS; /* depth test off */
		cfg.line_rasterization = V3D_LINE_RASTERIZATION_DIAMOND_EXIT;
		/* early_z_updates: ez_state is V3D_EZ_DISABLED without a
		 * depth buffer (v3dx_draw.c:941-945) */
	}

	/* POINT_SIZE on every 4.2 draw (v3dx_emit.c:422-437): GL's default 1.0, which
	 * v3d_rasterizer_state_create keeps (MAX2(size, .125f)) */
	v3d_cl_emit(bcl, POINT_SIZE, ps) {
		ps.point_size = 1.0f;
	}
	/* LINE_WIDTH (v3dx_emit.c:439-443): GL's default 1.0, no smoothing */
	v3d_cl_emit(bcl, LINE_WIDTH, lw) {
		lw.line_width = 1.0f;
	}

	/* the viewport (v3dx_emit.c:445-523) */
	v3d_cl_emit(bcl, CLIPPER_XY_SCALING, xy) {
		xy.viewport_half_width_in_1_256th_of_pixel = vp->scale[0] * 256.0f;
		xy.viewport_half_height_in_1_256th_of_pixel = vp->scale[1] * 256.0f;
	}
	v3d_cl_emit(bcl, CLIPPER_Z_SCALE_AND_OFFSET, z) {
		z.viewport_z_scale_zc_to_zs = vp->scale[2];
		z.viewport_z_offset_zc_to_zs = vp->translate[2];
	}
	/* util_viewport_zmin_zmax, clip_halfz false */
	float za = vp->translate[2] - vp->scale[2], zb = vp->translate[2] + vp->scale[2];
	v3d_cl_emit(bcl, CLIPPER_Z_MIN_MAX_CLIPPING_PLANES, zmm) {
		zmm.minimum_zw = za < zb ? za : zb;
		zmm.maximum_zw = za < zb ? zb : za;
	}
	float fine_x = vp->translate[0], fine_y = vp->translate[1];
	int32_t coarse_x = 0, coarse_y = 0;
	if (fine_x < 0) {
		/* DIV_ROUND_UP on a float, then truncated into an int32_t */
		int32_t blocks = (int32_t)((abs_f(fine_x) + 64 - 1) / 64);
		fine_x += 64.0f * blocks;
		coarse_x -= blocks;
	}
	if (fine_y < 0) {
		int32_t blocks = (int32_t)((abs_f(fine_y) + 64 - 1) / 64);
		fine_y += 64.0f * blocks;
		coarse_y -= blocks;
	}
	uint32_t fx, fy;
	if (!v3d_float_to_u14_8(fine_x, &fx) || !v3d_float_to_u14_8(fine_y, &fy)) {
		bcl->bad_field = true;
		return false;
	}
	v3d_cl_emit(bcl, VIEWPORT_OFFSET, vo) {
		vo.fine_x = (float)fx / 256.0f; /* exact: fx < 2^22 */
		vo.coarse_x = coarse_x;
		vo.fine_y = (float)fy / 256.0f;
		vo.coarse_y = coarse_y;
	}

	/* blending off (v3dx_emit.c:525-569), all channels written:
	 * translate_colormask of 0xf is 0 for each of the 4 RTs of 4.2
	 * (v3dx_emit.c:571-585) */
	v3d_cl_emit(bcl, BLEND_ENABLES, be);
	v3d_cl_emit(bcl, COLOR_WRITE_MASKS, wm);
	/* the blend colour, GL's default (0, 0, 0, 0) as half floats (v3dx_emit.c:587-598) */
	v3d_cl_emit(bcl, BLEND_CONSTANT_COLOR, bc);
	/* no stencil (v3dx_emit.c:600-619) */

	/* varying flags (v3dx_emit.c:621-643): M5 writes the all-zero case only */
	if (f->fs->flat_shade_flags || f->fs->noperspective_flags || f->fs->centroid_flags ||
	    f->fs->num_inputs > 24)
		return false;
	v3d_cl_emit(bcl, ZERO_ALL_FLAT_SHADE_FLAGS, zf);
	v3d_cl_emit(bcl, ZERO_ALL_NON_PERSPECTIVE_FLAGS, zn);
	v3d_cl_emit(bcl, ZERO_ALL_CENTROID_FLAGS, zc);

	/* no transform feedback targets (v3dx_emit.c:648-677) */
	v3d_cl_emit(bcl, TRANSFORM_FEEDBACK_SPECS, tf);
	/* no occlusion query (v3dx_emit.c:720-726) */
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq);
	/* SAMPLE_STATE (v3dx_emit.c:728-736): coverage 1.0, mask 0xf without MSAA */
	v3d_cl_emit(bcl, SAMPLE_STATE, ss) {
		ss.coverage = 1.0f;
		ss.mask = 0xf;
	}
	return v3d_cl_ok(bcl);
}

static bool threads_ok(uint32_t threads)
{
	/* v3d_compile gives 4, 2 or 1; 0 means the value was never filled */
	return threads == 1 || threads == 2 || threads == 4;
}

bool v3d_frame_build_triangle(const struct v3d_triangle_frame *f, struct v3d_cl *bcl,
			      struct v3d_cl *rcl, struct v3d_cl *indirect,
			      struct v3d_triangle_layout *layout)
{
	const struct v3d_clear_frame *c = &f->clear;
	const struct v3d_fs_prog *fs = f->fs;
	const struct v3d_vs_prog *vs = f->vs, *cs = f->cs;
	struct v3d_triangle_layout scratch;
	struct v3d_triangle_layout *lo = layout ? layout : &scratch;

	if (c->width == 0 || c->height == 0 || !fs || !vs || !cs)
		return false;
	if (!threads_ok(fs->threads) || !threads_ok(vs->threads) || !threads_ok(cs->threads))
		return false;
	/* u_trim_pipe_prim would drop a partial triangle; M5 refuses it */
	if (f->vertex_count == 0 || f->vertex_count % 3)
		return false;
	/* M5 writes real attribute records only: the GFXH-930 dummy record for
	 * a program without attributes is not written (v3dx_draw.c:849-864) */
	if (f->num_attrs == 0 || f->num_attrs > V3D_MAX_ATTRS || !f->attrs)
		return false;
	for (uint32_t i = f->num_attrs; i < V3D_MAX_ATTRS; i++)
		if (vs->vattr_sizes[i] || cs->vattr_sizes[i])
			return false; /* the program reads an attribute with no record */
	for (uint32_t i = 0; i < f->num_attrs; i++)
		if (f->attrs[i].components < 1 || f->attrs[i].components > 4)
			return false;

	struct v3d_tiling t;
	v3d_frame_tiling(c->width, c->height, &t);

	/* --- BCL: start_binning, then v3d_draw_vbo --- */
	emit_start_binning(c->width, c->height, bcl);
	if (!emit_state(f, bcl))
		return false;

	/* --- the indirect buffer: v3d_emit_gl_shader_state --- */
	/* the default attribute values, (0, 0, 0, 1.0) per vec4 for the 16
	 * vec4 slots of V3D_MAX_VS_INPUTS (v3dx_state.c:445-464; every M5
	 * attribute is float, so w is fui(1.0) for all of them) */
	uint8_t *d = v3d_cl_reserve(indirect, 16 * 16, 16);
	if (!d)
		return false;
	lo->defaults = v3d_cl_gpu_next(indirect) - 16 * 16;
	for (int i = 0; i < 16; i++)
		put_u32(d + i * 16 + 12, v3d_cl_float_bits(1.0f));

	/* the uniform streams, FS first, then VS, then CS (v3dx_draw.c:695-716) */
	if (!write_uniforms(indirect, fs->uniforms, fs->num_uniforms, &f->vp, &f->fs_storage,
			    &f->wpos, f->clear.height, &lo->fs_uniforms) ||
	    !write_uniforms(indirect, vs->uniforms, vs->num_uniforms, &f->vp, &f->vs_storage,
			    &f->wpos, f->clear.height, &lo->vs_uniforms) ||
	    !write_uniforms(indirect, cs->uniforms, cs->num_uniforms, &f->vp, &f->vs_storage,
			    &f->wpos, f->clear.height, &lo->cs_uniforms))
		return false;

	/* the record and the attribute records, 32-byte aligned (v3dx_draw.c:749-756) */
	if (!v3d_cl_reserve(indirect, 0, 32))
		return false;
	lo->record = v3d_cl_gpu_next(indirect);

	/* v3d_compute_vpm_config without a GS (compiler/vir.c:3071-3079):
	 * As = 1, Ve = 0, Vc = vcm_cache_size for both the bin and render. */
	v3d_cl_emit(indirect, GL_SHADER_STATE_RECORD, r) {
		/* emit_shader_state_record, the 4.2 record (v3dx_draw.c:553-682) */
		r.enable_clipping = true;
		r.point_size_in_shaded_vertex_data = false; /* a triangle draw */
		r.fragment_shader_does_z_writes = fs->writes_z;
		r.enable_sample_rate_shading = false; /* no MSAA */
		r.turn_off_early_z_test = fs->disable_ez;
		r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2 =
			fs->uses_center_w;
		r.any_shader_reads_hardware_written_primitive_id = fs->uses_pid; /* no GS */
		r.insert_primitive_id_as_first_varying_to_fragment_shader = fs->uses_pid;
		r.do_scoreboard_wait_on_first_thread_switch = fs->lock_scoreboard_on_first_thrsw;
		r.disable_implicit_point_line_varyings = !fs->uses_implicit_point_line_varyings;
		r.number_of_varyings_in_fragment_shader = fs->num_inputs;
		r.fragment_shader_code_address = fs->code;
		r.vertex_shader_code_address = vs->code;
		r.coordinate_shader_code_address = cs->code;
		r.coordinate_shader_propagate_nans = true;
		r.vertex_shader_propagate_nans = true;
		r.fragment_shader_propagate_nans = true;
		r.coordinate_shader_has_separate_input_and_output_vpm_blocks = cs->separate_segments;
		r.vertex_shader_has_separate_input_and_output_vpm_blocks = vs->separate_segments;
		r.coordinate_shader_input_vpm_segment_size =
			cs->separate_segments ? cs->vpm_input_size : 1;
		r.vertex_shader_input_vpm_segment_size = vs->separate_segments ? vs->vpm_input_size : 1;
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
		/* Mesa's 4.2 record leaves the base-instance bits unset */
		r.address_of_default_attribute_values = lo->defaults;
		lo->rec = r;
	}

	/* the attribute records (v3dx_draw.c:798-847), with the GFXH-930 and GFXH-1602
	 * rules for what the CS must load */
	lo->attr_records = v3d_cl_gpu_next(indirect);
	bool cs_loaded_any = false;
	const bool cs_uses_builtins = cs->uses_iid || cs->uses_biid || cs->uses_vid;
	for (uint32_t i = 0; i < f->num_attrs; i++) {
		v3d_cl_emit(indirect, GL_SHADER_STATE_ATTRIBUTE_RECORD, a) {
			a.address = f->attrs[i].address;
			a.stride = f->attrs[i].stride;
			a.vec_size = f->attrs[i].components & 3; /* 0 means 4 */
			a.type = V3D_ATTRIBUTE_FLOAT;
			a.number_of_values_read_by_coordinate_shader = cs->vattr_sizes[i];
			a.number_of_values_read_by_vertex_shader = vs->vattr_sizes[i];
			a.maximum_index = 0xffffff;
			if (cs->vattr_sizes[i])
				cs_loaded_any = true;
			if (i == 0 && cs_uses_builtins && !cs_loaded_any) {
				a.number_of_values_read_by_coordinate_shader = 1;
				cs_loaded_any = true;
			} else if (i == f->num_attrs - 1 && !cs_loaded_any) {
				a.number_of_values_read_by_coordinate_shader = 1;
				cs_loaded_any = true;
			}
			lo->attr[i] = a;
		}
	}

	/* back in the BCL (v3dx_draw.c:868-884) */
	v3d_cl_emit(bcl, VCM_CACHE_SIZE, vcm) {
		vcm.number_of_16_vertex_batches_for_binning = cs->vcm_cache_size;
		vcm.number_of_16_vertex_batches_for_rendering = vs->vcm_cache_size;
	}
	v3d_cl_emit(bcl, GL_SHADER_STATE, state) {
		state.address = lo->record;
		state.number_of_attribute_arrays = f->num_attrs;
	}
	/* no base vertex / instance, no index buffer, one instance */
	v3d_cl_emit(bcl, VERTEX_ARRAY_PRIMS, prims) {
		prims.mode = V3D_PRIM_TRIANGLES;
		prims.length = f->vertex_count;
		prims.index_of_first_vertex = f->first_vertex;
	}
	/* v3dX(bcl_epilogue): no TF, no primitive counts */
	v3d_cl_emit(bcl, FLUSH, flush);

	/* --- the tile list (Mesa writes it into indirect at submit) and the RCL --- */
	emit_tile_list(c, indirect, &lo->tile_list_start, &lo->tile_list_end);
	emit_rcl(c, &t, lo->tile_list_start, lo->tile_list_end, rcl);

	return v3d_cl_ok(bcl) && v3d_cl_ok(rcl) && v3d_cl_ok(indirect);
}
