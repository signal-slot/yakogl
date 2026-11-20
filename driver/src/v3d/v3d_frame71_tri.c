/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * Ported from Mesa 26.2.1 src/gallium/drivers/v3d/ (v3dx_draw.c, v3dx_emit.c,
 * v3d_uniforms.c) (MIT), the version 71 paths.
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * The V3D 7.1 clear-then-triangle frame (M5 on the Raspberry Pi 5), built
 * with -DV3D_CL_VER=71 next to v3d_frame.c (whose clear-frame steps it calls
 * through v3d_frame71_*). The byte reference is Mesa 26.2.2's own job on the
 * board, tools/v3d/pi5/triangle.clif; tools/v3d/check71.d/triangle71.sh
 * compares the two. The design, packet by packet, and what differs from the
 * 4.2 triangle: docs/notes/v3d71-triangle-frame.md.
 *
 * At 4.2 this file is empty, so a build may list it for both generations.
 */
#include "v3d_frame.h"

#if V3D_CL_VER == 71

/* devinfo->clipper_xy_granularity of a version 71 device (v3d_device_info.c) */
#define CLIPPER_XY_GRANULARITY 64.0f

static float abs_f(float v)
{
	return v < 0.0f ? -v : v;
}

/* A float that Mesa assigns to a uint32_t pixel coordinate (v3dx_emit.c):
 * truncated; outside [0, 2^16) or NaN is refused. */
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
 * v3d_uniforms.c v3d_write_uniforms, the 4.2 builder's rule (v3d_frame.c
 * write_uniforms) with the 7.1 clipper granularity: the viewport X and Y
 * scales are in 1/64 pixel (the board's VS stream starts 0x45000000, 32 x 64).
 * The stream starts 4-byte aligned with room for one more word behind it (the
 * QPU prefetches); the next stream starts right after its last word, so an
 * empty FS stream has the VS stream's address, as in the board's dump. Any
 * contents the 4.2 builder does not write is refused here too.
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

/* v3dx_emit.c v3dX(emit_state), version 71, for the first draw of a job with
 * the default GL state (no depth/stencil buffer, scissor, blending, culling,
 * transform feedback, query or MSAA). The packets and their order are the
 * board's binning list after START_TILE_BINNING (triangle.clif). */
static bool emit_state(const struct v3d_triangle_frame *f, struct v3d_cl *bcl)
{
	const struct v3d_viewport *vp = &f->vp;

	/* CLIP_WINDOW, no scissor: the viewport clamped to the frame */
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

	/* CFG_BITS at 7.1: no early-Z bits (the TLB decides), a 1-bit line
	 * rasterization (diamond exit, 0) and the Z clipping mode, -1..1 for
	 * GL's default clip_halfz false (the board's "Z Clipping mode: 1") */
	v3d_cl_emit(bcl, CFG_BITS, cfg) {
		cfg.enable_forward_facing_primitive = true;
		cfg.enable_reverse_facing_primitive = true;
		cfg.clockwise_primitives = f->front_ccw; /* as at 4.2 */
		cfg.depth_test_function = V3D_COMPARE_FUNC_ALWAYS;
		cfg.line_rasterization = V3D_LINE_RASTERIZATION_DIAMOND_EXIT;
		cfg.z_clipping_mode = V3D_Z_CLIP_MODE_MIN_ONE_TO_ONE;
	}
	v3d_cl_emit(bcl, POINT_SIZE, ps) {
		ps.point_size = 1.0f;
	}
	v3d_cl_emit(bcl, LINE_WIDTH, lw) {
		lw.line_width = 1.0f;
	}

	/* the viewport; the XY scaling is in 1/64 pixel at 7.1 */
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
		zmm.minimum_zw = za < zb ? za : zb;
		zmm.maximum_zw = za < zb ? zb : za;
	}
	float fine_x = vp->translate[0], fine_y = vp->translate[1];
	int32_t coarse_x = 0, coarse_y = 0;
	if (fine_x < 0) {
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
	/* the field is u14.8 as at 4.2; the CLIF prints its raw value (8192
	 * for 32 pixels) */
	v3d_cl_emit(bcl, VIEWPORT_OFFSET, vo) {
		vo.fine_x = (float)fx / 256.0f;
		vo.coarse_x = coarse_x;
		vo.fine_y = (float)fy / 256.0f;
		vo.coarse_y = coarse_y;
	}

	/* blending off for the 8 render targets of 7.1, all channels written
	 * (the write mask's bits are "disable"), the blend colour (0, 0, 0, 0) */
	v3d_cl_emit(bcl, BLEND_ENABLES, be);
	v3d_cl_emit(bcl, COLOR_WRITE_MASKS, wm);
	v3d_cl_emit(bcl, BLEND_CONSTANT_COLOR, bc);

	/* varying flags: the all-zero case only, as at 4.2 */
	if (f->fs->flat_shade_flags || f->fs->noperspective_flags || f->fs->centroid_flags ||
	    f->fs->flat_shade_flags_hi || f->fs->noperspective_flags_hi || f->fs->centroid_flags_hi ||
	    f->fs->num_inputs > 24)
		return false;
	v3d_cl_emit(bcl, ZERO_ALL_FLAT_SHADE_FLAGS, zf);
	v3d_cl_emit(bcl, ZERO_ALL_NON_PERSPECTIVE_FLAGS, zn);
	v3d_cl_emit(bcl, ZERO_ALL_CENTROID_FLAGS, zc);

	v3d_cl_emit(bcl, TRANSFORM_FEEDBACK_SPECS, tf);
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq);
	v3d_cl_emit(bcl, SAMPLE_STATE, ss) {
		ss.coverage = 1.0f;
		ss.mask = 0xf;
	}
	return v3d_cl_ok(bcl);
}

static bool threads_ok(uint32_t threads)
{
	return threads == 1 || threads == 2 || threads == 4;
}

/* What the 7.1 record can say of a VS or CS: shared VPM segments with no
 * input size (Mesa's 7.1 compiler always shares them, and the record has no
 * "separate segments" flag), no base-instance read (not in a 7.1 reference). */
static bool vs_ok71(const struct v3d_vs_prog *p)
{
	return threads_ok(p->threads) && !p->separate_segments && p->vpm_input_size == 0 &&
	       !p->uses_biid;
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
	if (!threads_ok(fs->threads) || !vs_ok71(vs) || !vs_ok71(cs))
		return false;
	if (f->vertex_count == 0 || f->vertex_count % 3)
		return false;
	if (f->num_attrs == 0 || f->num_attrs > V3D_MAX_ATTRS || !f->attrs)
		return false;
	for (uint32_t i = f->num_attrs; i < V3D_MAX_ATTRS; i++)
		if (vs->vattr_sizes[i] || cs->vattr_sizes[i])
			return false;
	for (uint32_t i = 0; i < f->num_attrs; i++)
		if (f->attrs[i].components < 1 || f->attrs[i].components > 4 ||
		    f->attrs[i].divisor != 0)
			return false;

	/* one RGBA8 target, 4 bytes a pixel: the clear frame's tiling */
	struct v3d_tiling t;
	v3d_frame_tiling_gen(71, c->width, c->height, 1, false, V3D_INTERNAL_BPP_32, 4, &t);

	/* --- BCL: start_binning, then v3d_draw_vbo --- */
	v3d_frame71_start_binning(c->width, c->height, &t, bcl);
	if (!emit_state(f, bcl))
		return false;

	/* --- the indirect buffer: v3d_emit_gl_shader_state. No default
	 * attribute values at 7.1: the uniform streams come first (FS, VS, CS) --- */
	if (!write_uniforms(indirect, fs->uniforms, fs->num_uniforms, &f->vp, &f->fs_storage,
			    &f->wpos, c->height, &lo->fs_uniforms) ||
	    !write_uniforms(indirect, vs->uniforms, vs->num_uniforms, &f->vp, &f->vs_storage,
			    &f->wpos, c->height, &lo->vs_uniforms) ||
	    !write_uniforms(indirect, cs->uniforms, cs->num_uniforms, &f->vp, &f->vs_storage,
			    &f->wpos, c->height, &lo->cs_uniforms))
		return false;

	/* the record, 32-byte aligned, then the attribute records */
	if (!v3d_cl_reserve(indirect, 0, 32))
		return false;
	lo->record = v3d_cl_gpu_next(indirect);

	/* The BCM2712 D0 layout (v3d_device_has_draw_index, rev >= 10). The
	 * values are the 4.2 record's where the field exists in both, but: no
	 * default attribute values and no "separate VPM segments" flags (the
	 * input segment size is the program's, 0 when shared), the NaN
	 * propagation bits clear (the board's record has them clear for all
	 * three stages), and the D0's draw-index and base-vertex bits clear (our
	 * programs report neither; Mesa sets them only for a program reading
	 * gl_DrawID or gl_BaseVertex). */
	v3d_cl_emit(indirect, GL_SHADER_STATE_RECORD_DRAW_INDEX, r) {
		r.point_size_in_shaded_vertex_data = false; /* a triangle draw */
		r.enable_clipping = true;
		r.vertex_id_read_by_coordinate_shader = cs->uses_vid;
		r.instance_id_read_by_coordinate_shader = cs->uses_iid;
		r.vertex_id_read_by_vertex_shader = vs->uses_vid;
		r.instance_id_read_by_vertex_shader = vs->uses_iid;
		r.fragment_shader_does_z_writes = fs->writes_z;
		r.turn_off_early_z_test = fs->disable_ez;
		r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2 =
			fs->uses_center_w;
		r.enable_sample_rate_shading = false; /* no MSAA */
		r.any_shader_reads_hardware_written_primitive_id = fs->uses_pid; /* no GS */
		r.insert_primitive_id_as_first_varying_to_fragment_shader = fs->uses_pid;
		r.do_scoreboard_wait_on_first_thread_switch = fs->lock_scoreboard_on_first_thrsw;
		r.disable_implicit_point_line_varyings = !fs->uses_implicit_point_line_varyings;
		r.number_of_varyings_in_fragment_shader = fs->num_inputs;
		r.coordinate_shader_output_vpm_segment_size = cs->vpm_output_size;
		r.min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size = 0;
		r.coordinate_shader_input_vpm_segment_size = cs->vpm_input_size;
		r.min_coord_shader_input_segments_required_in_play = 1;
		r.vertex_shader_output_vpm_segment_size = vs->vpm_output_size;
		r.min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size = 0;
		r.vertex_shader_input_vpm_segment_size = vs->vpm_input_size;
		r.min_vertex_shader_input_segments_required_in_play = 1;
		r.fragment_shader_code_address = fs->code;
		r.fragment_shader_4_way_threadable = fs->threads == 4;
		r.fragment_shader_start_in_final_thread_section = fs->single_seg;
		r.fragment_shader_uniforms_address = lo->fs_uniforms;
		r.vertex_shader_code_address = vs->code;
		r.vertex_shader_4_way_threadable = vs->threads == 4;
		r.vertex_shader_start_in_final_thread_section = vs->single_seg;
		r.vertex_shader_uniforms_address = lo->vs_uniforms;
		r.coordinate_shader_code_address = cs->code;
		r.coordinate_shader_4_way_threadable = cs->threads == 4;
		r.coordinate_shader_start_in_final_thread_section = cs->single_seg;
		r.coordinate_shader_uniforms_address = lo->cs_uniforms;
		lo->rec = r;
	}

	/* the attribute records, with the 4.2 builder's GFXH-930 / GFXH-1602
	 * rule for what the CS must load (the board's: 4 and 0 values for the
	 * position and the colour) */
	lo->attr_records = v3d_cl_gpu_next(indirect);
	bool cs_loaded_any = false;
	const bool cs_uses_builtins = cs->uses_iid || cs->uses_vid;
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

	/* back in the BCL */
	v3d_cl_emit(bcl, VCM_CACHE_SIZE, vcm) {
		vcm.number_of_16_vertex_batches_for_binning = cs->vcm_cache_size;
		vcm.number_of_16_vertex_batches_for_rendering = vs->vcm_cache_size;
	}
	v3d_cl_emit(bcl, GL_SHADER_STATE, state) {
		state.address = lo->record;
		state.number_of_attribute_arrays = f->num_attrs;
	}
	v3d_cl_emit(bcl, VERTEX_ARRAY_PRIMS, prims) {
		prims.mode = V3D_PRIM_TRIANGLES;
		prims.length = f->vertex_count;
		prims.index_of_first_vertex = f->first_vertex;
	}
	v3d_cl_emit(bcl, FLUSH, flush);

	/* --- the tile list behind the records, and the RCL: the clear frame's --- */
	v3d_frame71_tile_list(c, indirect, &lo->tile_list_start, &lo->tile_list_end);
	v3d_frame71_rcl(c, &t, lo->tile_list_start, lo->tile_list_end, rcl);

	return v3d_cl_ok(bcl) && v3d_cl_ok(rcl) && v3d_cl_ok(indirect);
}

#endif /* V3D_CL_VER == 71 */
