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
 * V3D 4.2 job builder (see v3d_job.h). Each step names the Mesa 26.2.1
 * function it follows; the packets are packed by Mesa's own generated
 * packers (v3d_packet_v42_pack.h, through v3d_cl.h's v3d_cl_emit).
 *
 * Three helpers here are twins of static ones in v3d_frame.c (the BCL
 * prologue, the viewport packets, the uniform writer), with the same
 * semantics. v3d_frame.c is left as it is until its QUNIFORM_UNIFORM board run
 * lands; then the twins move into one shared header (the lead's plan,
 * 2026-09-24), and frame_check covers both builders until then.
 */
#include "v3d_job.h"
#include "v3d_tex.h" /* v3d_tex_p0 / v3d_tex_p1 */

#define V3D_JOB_TILE_ALLOC_INITIAL_BLOCK V3D_TILE_ALLOC_BLOCK_128B

/* What v3d_job_end still writes after the last draw: FLUSH in the BCL, and
 * the generic tile list in the indirect buffer (TILE_COORDINATES_IMPLICIT 1,
 * LOAD_TILE_BUFFER_GENERAL 13, END_OF_LOADS 1, PRIM_LIST_FORMAT 2,
 * SET_INSTANCEID 5, BRANCH_TO_IMPLICIT_TILE_LIST 2, STORE_TILE_BUFFER_GENERAL
 * 13, CLEAR_TILE_BUFFERS 2, END_OF_TILE_MARKER 1, RETURN_FROM_SUB_LIST 1: 41
 * bytes). A draw is refused as FULL when it would leave less, so a job that
 * took a draw can always be ended. */
#define BCL_END_RESERVE 1u
#define IND_END_RESERVE 64u

bool v3d_job_pack_skip;

static struct v3d_job_pack_counts pack_totals;
static uint32_t pack_jobs;

void v3d_job_pack_totals(struct v3d_job_pack_counts *out, uint32_t *jobs)
{
	if (out)
		*out = pack_totals;
	if (jobs)
		*jobs = pack_jobs;
}

/* One draw's packet skip (v3d_job.h "Packet skip"): the groups it wrote,
 * with their inputs, and its counts. v3d_job_draw hands them to the job
 * only when the draw is recorded. */
struct pack_draw {
	uint32_t written; /* bit g: group g written by this draw */
	uint32_t key[V3D_PACK_GROUPS][V3D_PACK_KEY_MAX];
	struct v3d_job_pack_counts n;
};

/* With the knob on: whether group g, `packets` packets, must be written,
 * which is when it was not written yet in this job or `key` (its `words`
 * inputs) differs from what it was last written with. */
static bool pack_group(struct v3d_job *j, struct pack_draw *pd, uint32_t g, uint32_t packets,
		       const uint32_t *key, uint32_t words)
{
	if (words > V3D_PACK_KEY_MAX) {
		j->bcl->bad_field = true; /* a group this file lists wrongly */
		return true;
	}
	if (j->pack.valid & (1u << g)) {
		uint32_t diff = 0;
		for (uint32_t i = 0; i < words; i++)
			diff |= j->pack.key[g][i] ^ key[i];
		pd->n.compared++;
		if (!diff) {
			pd->n.skipped += packets;
			return false;
		}
		pd->n.mismatched++;
	}
	for (uint32_t i = 0; i < words; i++)
		pd->key[g][i] = key[i];
	pd->written |= 1u << g;
	pd->n.emitted += packets;
	return true;
}

/* Whether group `g` (V3D_PACK_*), `packets` packets, is written by this
 * draw. With the knob off: always, and the inputs after `packets` are not
 * even evaluated. With it on: pack_group() on those inputs, each one word
 * (a float as its bits, so the compare is exact). Every input a group's
 * packets are packed from must be listed, or a changed one would be missed;
 * each group has this one call site in emit_state(). Uses `j` and `pd`. */
#define PACK(g, packets, ...)                                                                   \
	(!j->pack.on ? (pd->n.emitted += (packets), true)                                       \
		     : pack_group(j, pd, (g), (packets), (const uint32_t[]){ __VA_ARGS__ },    \
				  sizeof((const uint32_t[]){ __VA_ARGS__ }) / sizeof(uint32_t)))

static uint32_t fb(float f)
{
	return v3d_cl_float_bits(f);
}

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

/* A float Mesa assigns to a uint32_t pixel coordinate (v3dx_emit.c:246-257):
 * C converts by truncation; a value outside [0, 2^16) or a NaN is refused. */
static bool to_pixel(float v, uint32_t *out)
{
	if (!(v >= 0.0f && v < 65536.0f))
		return false;
	*out = (uint32_t)v;
	return true;
}

/* The render targets the job configures: RT 0 and the target's extra ones. */
static uint32_t job_rts(const struct v3d_job_target *t)
{
	return t->rts > 1 ? t->rts : 1;
}

bool v3d_job_begin(struct v3d_job *j, const struct v3d_job_target *t, struct v3d_cl *bcl,
		   struct v3d_cl *rcl, struct v3d_cl *ind)
{
	*j = (struct v3d_job){ .bcl = bcl, .rcl = rcl, .ind = ind, .t = *t };
	j->pack.on = v3d_job_pack_skip;
	/* 4x with more than one RT: not written (the samples of RT 1..3 would
	 * need buffers of their own); refused. 4x with no colour buffer
	 * (address 0) is refused too: a depth-only framebuffer is never
	 * multisampled in ES 2.0, so the shape has no user. */
	if (t->width == 0 || t->height == 0 || t->rts > 4 || (t->msaa && t->rts > 1) ||
	    (t->msaa && !t->address)) {
		j->broken = true;
		return false;
	}

	/* v3dx_draw.c v3dX(start_binning), V3D_VERSION 42: the RT count and
	 * the max bpp decide the hardware's tile size (v3d_frame_tiling_rts) */
	v3d_cl_emit(bcl, TILE_BINNING_MODE_CFG, bin) {
		bin.width_in_pixels = t->width;
		bin.height_in_pixels = t->height;
		bin.number_of_render_targets = job_rts(t);
		bin.maximum_bpp_of_all_render_targets = V3D_INTERNAL_BPP_32;
		bin.multisample_mode_4x = t->msaa;
		bin.tile_allocation_initial_block_size = V3D_JOB_TILE_ALLOC_INITIAL_BLOCK;
		bin.tile_allocation_block_size = V3D_TILE_ALLOC_BLOCK_64B;
	}
	v3d_cl_emit(bcl, FLUSH_VCD_CACHE, flush);
	v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq);
	v3d_cl_emit(bcl, START_TILE_BINNING, start);

	/* the default attribute values, (0, 0, 0, 1.0) per vec4 for the 16 vec4
	 * slots of V3D_MAX_VS_INPUTS (v3dx_state.c:445-464; every attribute this
	 * builder takes is float, so w is fui(1.0) for all of them). Mesa keeps
	 * them in its own state buffer, one per vertex-elements state; one per
	 * job serves every draw here. */
	uint8_t *d = v3d_cl_reserve(ind, 16 * 16, 16);
	if (!d || !v3d_cl_ok(bcl)) {
		j->broken = true;
		return false;
	}
	j->defaults = v3d_cl_gpu_next(ind) - 16 * 16;
	for (int i = 0; i < 16; i++)
		put_u32(d + i * 16 + 12, v3d_cl_float_bits(1.0f));
	return true;
}

/*
 * v3d_uniforms.c v3d_write_uniforms for the contents this builder supports
 * (the twin of v3d_frame.c's write_uniforms). The stream starts 4-byte
 * aligned and one more word than it holds must fit behind it (the QPU
 * prefetches the next uniform, v3d_uniforms.c:228-236). Any other contents is
 * refused, never written as 0; so is a QUNIFORM_UNIFORM whose word is not in
 * the stage's storage.
 */
/* The records of sampler binding `unit` of this stage, or NULL when the
 * draw did not bind it. */
static const struct v3d_job_tmu *find_tmu(const struct v3d_job_tmu *tmu, uint32_t n, uint32_t unit)
{
	for (uint32_t i = 0; tmu && i < n; i++)
		if (tmu[i].unit == unit)
			return &tmu[i];
	return NULL;
}

static bool write_uniforms(struct v3d_cl *cl, const struct v3d_uniform *u, uint32_t n,
			   const struct v3d_viewport *vp, const struct v3d_uniform_storage *st,
			   const struct v3d_job_tmu *tmu, uint32_t num_tmu, const struct v3d_wpos *wpos,
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
		case V3D_QUNIFORM_TMU_CONFIG_P0:
		case V3D_QUNIFORM_TMU_CONFIG_P1: {
			/* v3d_uniforms.c write_tmu_p0 / write_tmu_p1, through the
			 * texture code's own helpers (v3d_tex.c), which refuse an
			 * unaligned record and bits outside the P0/P1 fields; the
			 * unit is the stage's sampler binding (bits 24..31) */
			const struct v3d_job_tmu *t = find_tmu(tmu, num_tmu, u[i].data >> 24);
			bool ok = t && (u[i].contents == V3D_QUNIFORM_TMU_CONFIG_P0
						? v3d_tex_p0(t->tex_state, u[i].data, &v)
						: v3d_tex_p1(t->sampler, u[i].data, &v));
			if (!ok) {
				cl->bad_field = true;
				return false;
			}
			break;
		}
		case V3D_QUNIFORM_TEXTURE_WIDTH:
		case V3D_QUNIFORM_TEXTURE_HEIGHT: {
			/* v3d_uniforms.c:52-70, 290-298: the base level's size of the
			 * binding `data` (not shifted); a binding the draw did
			 * not give, or one given without its size, is refused */
			const struct v3d_job_tmu *t = find_tmu(tmu, num_tmu, u[i].data);
			v = !t ? 0 : u[i].contents == V3D_QUNIFORM_TEXTURE_WIDTH ? t->width : t->height;
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
			/* the target's own height: a transform made for another
			 * size is refused */
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

/* v3dx_emit.c emit_varying_flags for word 0 (the only word struct
 * v3d_fs_prog carries: 24 varyings, more than MAX_VARYING_VECTORS asks for):
 * one packet that zeroes the higher words, or ZERO_ALL_* when it is 0. The
 * three kinds are three packets of one layout (V3DX(emit_varying_flags)'s
 * flag_emit_callback in Mesa). */
#define EMIT_VARYING_FLAGS(bcl, KIND, kind, w0)                                                    \
	do {                                                                                       \
		if (!(w0)) {                                                                       \
			v3d_cl_emit(bcl, ZERO_ALL_##KIND##_FLAGS, zero_all);                       \
		} else {                                                                           \
			v3d_cl_emit(bcl, KIND##_FLAGS, flags) {                                    \
				flags.varying_offset_v0 = 0;                                       \
				flags.action_for_##kind##_flags_of_lower_numbered_varyings =       \
					V3D_VARYING_FLAGS_ACTION_UNCHANGED;                        \
				flags.action_for_##kind##_flags_of_higher_numbered_varyings =      \
					V3D_VARYING_FLAGS_ACTION_ZEROED;                           \
				flags.kind##_flags_for_varyings_v024 = (w0);                       \
			}                                                                          \
		}                                                                                  \
	} while (0)

static void emit_stencil_face(struct v3d_cl *bcl, const struct v3d_job_stencil_face *f, bool front,
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

/* CLIP_WINDOW (v3dx_emit.c:229-266): the viewport, and the scissor or the
 * drawable */
static bool emit_clip_window(struct v3d_job *j, const struct v3d_job_state *st)
{
	struct v3d_cl *bcl = j->bcl;
	const struct v3d_viewport *vp = &st->vp;
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
	return true;
}

/* CFG_BITS (v3dx_emit.c:294-405). Points and lines are never culled as
 * forward facing; early Z stays off (the RCL disables it, see v3d_job_end),
 * so its two bits stay 0. */
static void emit_cfg_bits(struct v3d_job *j, const struct v3d_job_state *st, bool polygon)
{
	v3d_cl_emit(j->bcl, CFG_BITS, cfg) {
		cfg.enable_forward_facing_primitive = !polygon || !st->cull_front;
		cfg.enable_reverse_facing_primitive = !st->cull_back;
		cfg.clockwise_primitives = st->front_ccw; /* "seems backwards", v3dx_emit.c:315-319 */
		cfg.enable_depth_offset = st->offset_tri;
		cfg.depth_test_function = V3D_COMPARE_FUNC_ALWAYS;
		cfg.stencil_enable = st->stencil_enabled;
		cfg.blend_enable = st->blend_enabled;
		cfg.line_rasterization = V3D_LINE_RASTERIZATION_DIAMOND_EXIT;
		/* rasterizer oversample in a 4x job (v3dx_emit.c:326-334:
		 * multisample || sample_mask != 0xf; GL ES rasterizes a
		 * multisampled surface multisampled, and a single-sampled job
		 * leaves the mask at 0xf) */
		cfg.rasterizer_oversample_mode = j->t.msaa ? 1 : 0;
		if (st->depth_enabled) {
			cfg.z_updates_enable = st->depth_writemask;
			cfg.depth_test_function = st->depth_func;
		}
	}
}

/* the viewport (v3dx_emit.c:445-523) */
static bool emit_viewport(struct v3d_cl *bcl, const struct v3d_viewport *vp)
{
	v3d_cl_emit(bcl, CLIPPER_XY_SCALING, xy) {
		xy.viewport_half_width_in_1_256th_of_pixel = vp->scale[0] * 256.0f;
		xy.viewport_half_height_in_1_256th_of_pixel = vp->scale[1] * 256.0f;
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
		vo.fine_x = (float)fx / 256.0f; /* exact: fx < 2^22 */
		vo.coarse_x = coarse_x;
		vo.fine_y = (float)fy / 256.0f;
		vo.coarse_y = coarse_y;
	}
	return true;
}

/* BLEND_ENABLES and BLEND_CFG (v3dx_emit.c:525-568): one render target, no
 * independent blend, so Mesa's single emit_rt_blend with rt_mask
 * (1 << max_rts) - 1, max_rts 4; and blend_enables = RT0 */
static void emit_blend(struct v3d_cl *bcl, const struct v3d_job_state *st)
{
	if (!st->blend_enabled) {
		v3d_cl_emit(bcl, BLEND_ENABLES, off);
		return;
	}
	v3d_cl_emit(bcl, BLEND_ENABLES, on) {
		on.mask = 1;
	}
	v3d_cl_emit(bcl, BLEND_CFG, b) {
		b.render_target_mask = 0xf;
		b.color_blend_mode = st->rgb_func;
		b.color_blend_src_factor = st->rgb_src;
		b.color_blend_dst_factor = st->rgb_dst;
		b.alpha_blend_mode = st->alpha_func;
		b.alpha_blend_src_factor = st->alpha_src;
		b.alpha_blend_dst_factor = st->alpha_dst;
	}
}

/* v3dx_emit.c v3dX(emit_state) for the gallium state in `d->st`, in Mesa's
 * order: with the packet skip off, every packet, as with every dirty bit set
 * (v3d_job.c:436); with it on, the groups whose inputs changed (v3d_job.h
 * "Packet skip"). Each PACK() lists every input its group's packets are
 * packed from; an input that does not change the packet when another is off
 * (the scissor box without a scissor, the blend factors without blending) is
 * listed as 0 then, so it does not force a write. */
static bool emit_state(struct v3d_job *j, const struct v3d_job_draw *d, struct pack_draw *pd)
{
	struct v3d_cl *bcl = j->bcl;
	const struct v3d_job_state *st = d->st;
	const struct v3d_viewport *vp = &st->vp;

	const bool sc = st->scissor;
	if (PACK(V3D_PACK_CLIP, 1, fb(vp->scale[0]), fb(vp->scale[1]), fb(vp->translate[0]),
		 fb(vp->translate[1]), sc, sc ? st->scissor_minx : 0, sc ? st->scissor_miny : 0,
		 sc ? st->scissor_maxx : 0, sc ? st->scissor_maxy : 0) &&
	    !emit_clip_window(j, st))
		return false;

	const bool polygon = d->prim >= V3D_PRIM_TRIANGLES, de = st->depth_enabled;
	if (PACK(V3D_PACK_CFG, 1,
		 (uint32_t)polygon | (uint32_t)st->cull_front << 1 | (uint32_t)st->cull_back << 2 |
			 (uint32_t)st->front_ccw << 3 | (uint32_t)st->offset_tri << 4 |
			 (uint32_t)st->stencil_enabled << 5 | (uint32_t)st->blend_enabled << 6 |
			 (uint32_t)de << 7 | (uint32_t)(de && st->depth_writemask) << 8,
		 de ? st->depth_func : 0))
		emit_cfg_bits(j, st, polygon);

	/* DEPTH_OFFSET (v3dx_emit.c:407-420): the Z24 values (no Z16 buffer) */
	if (st->offset_tri && PACK(V3D_PACK_DEPTH_OFFSET, 1, fb(st->offset_scale),
				   fb(st->offset_units), fb(st->offset_clamp))) {
		/* factor and units are f187 fields: the packer keeps a float's
		 * top 16 bits, so they are given as the value the field holds */
		v3d_cl_emit(bcl, DEPTH_OFFSET, dof) {
			dof.depth_offset_factor = v3d_cl_f187(st->offset_scale);
			dof.depth_offset_units = v3d_cl_f187(st->offset_units);
			dof.limit = st->offset_clamp;
		}
	}

	/* POINT_SIZE on every 4.2 draw (v3dx_emit.c:422-437), never skipped: ES
	 * 2.0 has no glPointSize, so gallium's 1.0; LINE_WIDTH (v3dx_emit.c:439-443) */
	v3d_cl_emit(bcl, POINT_SIZE, ps) {
		ps.point_size = 1.0f;
	}
	pd->n.emitted++;
	if (PACK(V3D_PACK_LINE, 1, fb(st->line_width))) {
		v3d_cl_emit(bcl, LINE_WIDTH, lw) {
			lw.line_width = st->line_width;
		}
	}

	if (PACK(V3D_PACK_VIEWPORT, 4, fb(vp->scale[0]), fb(vp->scale[1]), fb(vp->scale[2]),
		 fb(vp->translate[0]), fb(vp->translate[1]), fb(vp->translate[2])) &&
	    !emit_viewport(bcl, vp))
		return false;

	const bool be = st->blend_enabled;
	if (PACK(V3D_PACK_BLEND, be ? 2 : 1, be, be ? st->rgb_func : 0, be ? st->rgb_src : 0,
		 be ? st->rgb_dst : 0, be ? st->alpha_func : 0, be ? st->alpha_src : 0,
		 be ? st->alpha_dst : 0))
		emit_blend(bcl, st);
	/* COLOR_WRITE_MASKS (v3dx_emit.c:571-585): translate_colormask per RT,
	 * a set bit disables the channel; without independent blend every RT
	 * takes RT0's mask */
	uint32_t off = (~st->colormask) & 0xf;
	if (PACK(V3D_PACK_COLOR_MASK, 1, off)) {
		v3d_cl_emit(bcl, COLOR_WRITE_MASKS, wm) {
			wm.mask = off | off << 4 | off << 8 | off << 12;
		}
	}
	const uint16_t *bc = st->blend_color_f16;
	if (PACK(V3D_PACK_BLEND_COLOR, 1, bc[0] | (uint32_t)bc[1] << 16, bc[2] | (uint32_t)bc[3] << 16)) {
		v3d_cl_emit(bcl, BLEND_CONSTANT_COLOR, color) {
			color.red_f16 = bc[0];
			color.green_f16 = bc[1];
			color.blue_f16 = bc[2];
			color.alpha_f16 = bc[3];
		}
	}

	/* STENCIL_CFG (v3dx_emit.c:600-619, v3dx_state.c:220-264): the front
	 * face configures the back too unless the back has its own */
	const bool sb = st->stencil_back_enabled;
	const struct v3d_job_stencil_face *sf = &st->stencil_front, *sk = &st->stencil_back;
	if (st->stencil_enabled &&
	    PACK(V3D_PACK_STENCIL, sb ? 2 : 1, sb, sf->func, sf->sfail_op, sf->zfail_op, sf->zpass_op,
		 sf->ref, sf->value_mask, sf->write_mask, sb ? sk->func : 0, sb ? sk->sfail_op : 0,
		 sb ? sk->zfail_op : 0, sb ? sk->zpass_op : 0, sb ? sk->ref : 0,
		 sb ? sk->value_mask : 0, sb ? sk->write_mask : 0)) {
		emit_stencil_face(bcl, sf, true, !sb);
		if (sb)
			emit_stencil_face(bcl, sk, false, true);
	}

	/* varying flags (v3dx_emit.c:621-643) */
	const struct v3d_fs_prog *fs = d->fs;
	if (PACK(V3D_PACK_VARYINGS, 3, fs->flat_shade_flags, fs->noperspective_flags,
		 fs->centroid_flags)) {
		EMIT_VARYING_FLAGS(bcl, FLAT_SHADE, flat_shade, fs->flat_shade_flags);
		EMIT_VARYING_FLAGS(bcl, NON_PERSPECTIVE, non_perspective, fs->noperspective_flags);
		EMIT_VARYING_FLAGS(bcl, CENTROID, centroid, fs->centroid_flags);
	}

	/* no transform feedback targets (v3dx_emit.c:648-677); the occlusion
	 * query's counter, or the zero-address "disable" (720-726: the address
	 * only while a query is active, `oq_addr` 0 otherwise; the address is
	 * a key word, so a change of query rewrites the group); SAMPLE_STATE
	 * (728-736): coverage 1.0, the mask the draw's in a 4x job, 0xf
	 * otherwise */
	const uint32_t mask = j->t.msaa ? st->sample_mask & 0xf : 0xf;
	if (PACK(V3D_PACK_FIXED, 3, mask, st->oq_addr)) {
		v3d_cl_emit(bcl, TRANSFORM_FEEDBACK_SPECS, tf);
		v3d_cl_emit(bcl, OCCLUSION_QUERY_COUNTER, oq) {
			oq.address = st->oq_addr;
		}
		v3d_cl_emit(bcl, SAMPLE_STATE, ss) {
			ss.coverage = 1.0f;
			ss.mask = mask;
		}
	}
	return true;
}

static bool threads_ok(uint32_t threads)
{
	/* v3d_compile gives 4, 2 or 1; 0 means the value was never filled */
	return threads == 1 || threads == 2 || threads == 4;
}

/* What this builder refuses before it writes anything. */
static bool draw_is_supported(const struct v3d_job_draw *d)
{
	if (!d->st || !d->fs || !d->vs || !d->cs)
		return false;
	if (!threads_ok(d->fs->threads) || !threads_ok(d->vs->threads) ||
	    !threads_ok(d->cs->threads))
		return false;
	/* word 0 of the varying flags only (24 varyings) */
	if (d->fs->num_inputs > 24)
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

/* v3dx_draw.c v3d_draw_vbo for one draw: the state, the shader state (the
 * uniform streams, the record and the attribute records in the indirect
 * buffer, v3d_emit_gl_shader_state), then the primitive. */
static bool record_draw(struct v3d_job *j, const struct v3d_job_draw *d,
			struct v3d_job_draw_layout *lo, struct pack_draw *pd)
{
	const struct v3d_fs_prog *fs = d->fs;
	const struct v3d_vs_prog *vs = d->vs, *cs = d->cs;
	struct v3d_cl *ind = j->ind, *bcl = j->bcl;

	/* the implicit point varyings (gl_PointCoord) exist for points only:
	 * any other primitive would read every declared varying two slots off
	 * (the backend refuses it by name first) */
	if (fs->uses_implicit_point_line_varyings && d->prim != V3D_PRIM_POINTS) {
		ind->bad_field = true;
		return false;
	}
	if (!emit_state(j, d, pd))
		return false;

	/* the uniform streams, FS first, then VS, then CS (v3dx_draw.c:695-716) */
	if (!write_uniforms(ind, fs->uniforms, fs->num_uniforms, &d->st->vp, &d->fs_storage,
			    d->tmu, d->num_tmu, &d->st->wpos, j->t.height, &lo->fs_uniforms) ||
	    !write_uniforms(ind, vs->uniforms, vs->num_uniforms, &d->st->vp, &d->vs_storage,
			    d->vs_tmu, d->num_vs_tmu, &d->st->wpos, j->t.height, &lo->vs_uniforms) ||
	    !write_uniforms(ind, cs->uniforms, cs->num_uniforms, &d->st->vp, &d->vs_storage,
			    d->vs_tmu, d->num_vs_tmu, &d->st->wpos, j->t.height, &lo->cs_uniforms))
		return false;

	/* the record, 32-byte aligned (v3dx_draw.c:749-756); v3d_compute_vpm_config
	 * without a GS (compiler/vir.c:3071-3079) */
	if (!v3d_cl_reserve(ind, 0, 32))
		return false;
	lo->record = v3d_cl_gpu_next(ind);
	v3d_cl_emit(ind, GL_SHADER_STATE_RECORD, r) {
		/* emit_shader_state_record, the 4.2 record (v3dx_draw.c:553-682) */
		r.enable_clipping = true;
		r.point_size_in_shaded_vertex_data = d->vs_writes_psiz;
		r.fragment_shader_does_z_writes = fs->writes_z;
		r.enable_sample_rate_shading = false;
		r.turn_off_early_z_test = fs->disable_ez;
		r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2 =
			fs->uses_center_w;
		r.any_shader_reads_hardware_written_primitive_id = fs->uses_pid;
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
		r.address_of_default_attribute_values = j->defaults;
		lo->rec = r;
	}

	/* the attribute records (v3dx_draw.c:798-864), with the GFXH-930 and
	 * GFXH-1602 rules for what the CS must load, and the GFXH-930 dummy
	 * record for a program that reads no attribute */
	lo->attr_records = v3d_cl_gpu_next(ind);
	uint32_t records = d->num_attrs;
	bool cs_loaded_any = false;
	const bool cs_uses_builtins = cs->uses_iid || cs->uses_biid || cs->uses_vid;
	for (uint32_t i = 0; i < d->num_attrs; i++) {
		v3d_cl_emit(ind, GL_SHADER_STATE_ATTRIBUTE_RECORD, a) {
			a.address = d->attrs[i].address;
			a.stride = d->attrs[i].stride;
			a.vec_size = d->attrs[i].components & 3; /* 0 means 4 */
			a.type = V3D_ATTRIBUTE_FLOAT;
			a.number_of_values_read_by_coordinate_shader = cs->vattr_sizes[i];
			a.number_of_values_read_by_vertex_shader = vs->vattr_sizes[i];
			a.maximum_index = 0xffffff;
			/* v3dx_state.c:388-389: MIN2(divisor, the field's 16 bits) */
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
		/* "Valid address of data whose value will be unused": the
		 * job's default attribute values */
		v3d_cl_emit(ind, GL_SHADER_STATE_ATTRIBUTE_RECORD, a) {
			a.address = j->defaults;
			a.type = V3D_ATTRIBUTE_FLOAT;
			a.stride = 0;
			a.vec_size = 1;
			a.number_of_values_read_by_coordinate_shader = 1;
			a.number_of_values_read_by_vertex_shader = 1;
		}
		records = 1;
	}

	/* back in the BCL (v3dx_draw.c:868-884, 1290-1378): no base vertex or
	 * base instance (ES 3.0 has neither: no BASE_VERTEX_BASE_INSTANCE), no
	 * primitive restart; more than one instance is the instanced packet of
	 * the same kind, which walks the instances itself */
	v3d_cl_emit(bcl, VCM_CACHE_SIZE, vcm) {
		vcm.number_of_16_vertex_batches_for_binning = cs->vcm_cache_size;
		vcm.number_of_16_vertex_batches_for_rendering = vs->vcm_cache_size;
	}
	v3d_cl_emit(bcl, GL_SHADER_STATE, state) {
		state.address = lo->record;
		state.number_of_attribute_arrays = records;
	}
	/* VCM_CACHE_SIZE, GL_SHADER_STATE, the primitive, and the index buffer */
	pd->n.emitted += d->indexed ? 4 : 3;
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
				prim.enable_primitive_restarts = false;
				prim.number_of_instances = d->instances;
				prim.index_offset = d->index_offset;
			}
		} else {
			v3d_cl_emit(bcl, INDEXED_PRIM_LIST, prim) {
				prim.mode = d->prim;
				prim.index_type = d->index_type;
				prim.length = d->count;
				prim.enable_primitive_restarts = false;
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

int v3d_job_draw(struct v3d_job *j, const struct v3d_job_draw *d, struct v3d_job_draw_layout *layout)
{
	struct v3d_job_draw_layout scratch;
	struct v3d_job_draw_layout *lo = layout ? layout : &scratch;

	if (j->broken || !draw_is_supported(d))
		return V3D_JOB_REFUSED;

	/* Where the lists stand: a draw that does not go in leaves nothing. */
	const struct v3d_cl bcl0 = *j->bcl, ind0 = *j->ind;
	struct pack_draw pd; /* the key rows are written before they are read */
	pd.written = 0;
	pd.n = (struct v3d_job_pack_counts){ 0 };
	bool ok = record_draw(j, d, lo, &pd);
	bool bad = j->bcl->bad_field || j->ind->bad_field;
	bool full = j->bcl->overflow || j->ind->overflow ||
		    j->bcl->size - j->bcl->used < BCL_END_RESERVE ||
		    j->ind->size - j->ind->used < IND_END_RESERVE;
	if (!ok || bad || full) {
		*j->bcl = bcl0;
		*j->ind = ind0;
		return (full && !bad) ? V3D_JOB_FULL : V3D_JOB_REFUSED;
	}
	/* the draw is in: what its groups were written with is what the list
	 * now holds */
	for (uint32_t g = 0; pd.written >> g; g++)
		if (pd.written & (1u << g))
			memcpy(j->pack.key[g], pd.key[g], sizeof(pd.key[g]));
	j->pack.valid |= pd.written;
	j->pack.n.emitted += pd.n.emitted;
	j->pack.n.skipped += pd.n.skipped;
	j->pack.n.compared += pd.n.compared;
	j->pack.n.mismatched += pd.n.mismatched;
	j->draws++;
	return V3D_JOB_OK;
}

/* v3dx_rcl.c v3d_rcl_emit_generic_per_tile_list, written into `list`. */
static void emit_tile_list(const struct v3d_job *j, const struct v3d_job_clear *c,
			   struct v3d_cl *list, uint32_t *start, uint32_t *end)
{
	/* address 0: no colour buffer (a depth/stencil-only framebuffer, Mesa's
	 * job with nr_cbufs 0): RT 0 is configured for the TLB, never loaded or
	 * stored */
	const bool rt0 = j->t.address != 0;
	uint32_t stores = 0;

	*start = v3d_cl_gpu_next(list);
	v3d_cl_emit(list, TILE_COORDINATES_IMPLICIT, coords);
	/* v3d_rcl_emit_loads -> load_general, RT0, raster RGBA8: the colour
	 * the target already holds, when this job did not clear it. */
	if (rt0 && c->load_rt0 && !j->t.msaa) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, ld) {
			ld.buffer_to_load = V3D_LOAD_BUFFER_RT0;
			ld.memory_format = V3D_MEMORY_FORMAT_RASTER;
			ld.input_image_format = j->t.rgb565 ? V3D_OUTPUT_IMAGE_FORMAT_BGR565
							     : V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			ld.height_in_ub_or_stride = j->t.stride;
			ld.address = j->t.address;
			ld.force_alpha_1 = c->force_alpha_1;
			ld.r_b_swap = c->r_b_swap;
			ld.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	/* at 4x, the samples an earlier job kept (v3dx_rcl.c:100-110: a
	 * multisampled surface loads with ALL_SAMPLES); v3d_job_end refuses a
	 * 4x load with no buffer. RGBA8 in the tile buffer throughout, so
	 * neither swap nor alpha forcing applies to the kept samples. */
	if (rt0 && c->load_rt0 && j->t.msaa) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, ld) {
			ld.buffer_to_load = V3D_LOAD_BUFFER_RT0;
			ld.memory_format = j->t.ms_memory_format;
			ld.input_image_format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			ld.height_in_ub_or_stride = j->t.ms_height_in_ub;
			ld.address = j->t.ms_address;
			ld.decimate_mode = V3D_DECIMATE_MODE_ALL_SAMPLES;
		}
	}
	/* then RT 1..3 (v3d_rcl_emit_loads, v3dx_rcl.c:198-212: RENDER_TARGET_0
	 * + i, in RT order), each that has a buffer and was not cleared */
	for (uint32_t n = 1; n < job_rts(&j->t); n++) {
		const struct v3d_job_rt *rt = &j->t.rt[n - 1];
		if (!rt->address || !c->load_rt[n - 1])
			continue;
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, ld) {
			ld.buffer_to_load = V3D_LOAD_BUFFER_RT0 + n;
			ld.memory_format = V3D_MEMORY_FORMAT_RASTER;
			ld.input_image_format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			ld.height_in_ub_or_stride = rt->stride;
			ld.address = rt->address;
			ld.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	/* then Z and S apart, stencil first (v3dx_rcl.c:220-234): each half
	 * from its own buffer; at 4x, 4x buffers with all their samples */
	bool split = c->s_address != 0;
	enum V3D42_Decimate_Mode zs_decimate =
		j->t.msaa ? V3D_DECIMATE_MODE_ALL_SAMPLES : V3D_DECIMATE_MODE_SAMPLE_0;
	if (split && c->s_load) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, sl) {
			sl.buffer_to_load = V3D_LOAD_BUFFER_STENCIL;
			sl.memory_format = c->s_memory_format;
			sl.input_image_format =
				c->s_s8 ? V3D_OUTPUT_IMAGE_FORMAT_S8 : V3D_OUTPUT_IMAGE_FORMAT_D24S8;
			sl.height_in_ub_or_stride = c->s_height_in_ub;
			sl.address = c->s_address;
			sl.decimate_mode = zs_decimate;
		}
	}
	/* or Z and S together from their buffer (load_general, ZSTENCIL,
	 * D24S8, v3dx_rcl.c:207-234): what an earlier job stored */
	if (c->zs_address && c->zs_load) {
		v3d_cl_emit(list, LOAD_TILE_BUFFER_GENERAL, zl) {
			zl.buffer_to_load = split ? V3D_LOAD_BUFFER_Z : V3D_LOAD_BUFFER_ZSTENCIL;
			zl.memory_format = c->zs_memory_format;
			zl.input_image_format = V3D_OUTPUT_IMAGE_FORMAT_D24S8;
			zl.height_in_ub_or_stride = c->zs_height_in_ub;
			zl.address = c->zs_address;
			zl.decimate_mode = zs_decimate;
		}
	}
	v3d_cl_emit(list, END_OF_LOADS, end_loads);
	v3d_cl_emit(list, PRIM_LIST_FORMAT, fmt) {
		fmt.primitive_type = V3D_PRIM_LIST_TRIANGLES;
	}
	v3d_cl_emit(list, SET_INSTANCEID, iid);
	v3d_cl_emit(list, BRANCH_TO_IMPLICIT_TILE_LIST, branch);
	/* at 4x, RT 0's samples for the next job first (store_general with
	 * ALL_SAMPLES, v3dx_rcl.c:157-158), then the resolve below */
	if (rt0 && j->t.msaa && j->t.ms_address) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, ms) {
			ms.buffer_to_store = V3D_STORE_BUFFER_RT0;
			ms.memory_format = j->t.ms_memory_format;
			ms.output_image_format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			ms.height_in_ub_or_stride = j->t.ms_height_in_ub;
			ms.address = j->t.ms_address;
			ms.decimate_mode = V3D_DECIMATE_MODE_ALL_SAMPLES;
		}
	}
	/* v3d_rcl_emit_stores -> store_general, RT0, raster RGBA8; at 4x the
	 * resolve, decimate 4X (v3dx_rcl.c:159-166) */
	if (rt0) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, st) {
			st.buffer_to_store = V3D_STORE_BUFFER_RT0;
			st.memory_format = V3D_MEMORY_FORMAT_RASTER;
			st.output_image_format = j->t.rgb565 ? V3D_OUTPUT_IMAGE_FORMAT_BGR565
							     : V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			st.height_in_ub_or_stride = j->t.stride;
			st.address = j->t.address;
			st.r_b_swap = c->r_b_swap;
			st.decimate_mode = j->t.msaa ? V3D_DECIMATE_MODE_4X : V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	/* RT 1..3 (v3d_rcl_emit_stores, v3dx_rcl.c:258-283, in RT order), each
	 * that has a buffer: a GL_NONE one is never stored */
	for (uint32_t n = 1; n < job_rts(&j->t); n++) {
		const struct v3d_job_rt *rt = &j->t.rt[n - 1];
		if (!rt->address)
			continue;
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, sr) {
			sr.buffer_to_store = V3D_STORE_BUFFER_RT0 + n;
			sr.memory_format = V3D_MEMORY_FORMAT_RASTER;
			sr.output_image_format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
			sr.height_in_ub_or_stride = rt->stride;
			sr.address = rt->address;
			sr.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
		}
	}
	/* store_general, ZSTENCIL, D24S8 (v3dx_rcl.c:285-309): for the next job */
	if (c->zs_address && c->zs_store) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, zs) {
			zs.buffer_to_store = split ? V3D_STORE_BUFFER_Z : V3D_STORE_BUFFER_ZSTENCIL;
			zs.memory_format = c->zs_memory_format;
			zs.output_image_format = V3D_OUTPUT_IMAGE_FORMAT_D24S8;
			zs.height_in_ub_or_stride = c->zs_height_in_ub;
			zs.address = c->zs_address;
			zs.decimate_mode = zs_decimate;
		}
	}
	/* the stencil's own buffer after Z (v3dx_rcl.c:288-304) */
	if (split && c->s_store) {
		stores++;
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, ss) {
			ss.buffer_to_store = V3D_STORE_BUFFER_STENCIL;
			ss.memory_format = c->s_memory_format;
			ss.output_image_format =
				c->s_s8 ? V3D_OUTPUT_IMAGE_FORMAT_S8 : V3D_OUTPUT_IMAGE_FORMAT_D24S8;
			ss.height_in_ub_or_stride = c->s_height_in_ub;
			ss.address = c->s_address;
			ss.decimate_mode = zs_decimate;
		}
	}
	/* a tile list with nothing to store still emits a store, of buffer
	 * NONE (v3dx_rcl.c:315-321, "we still need to emit some sort of
	 * store"): a job with no colour buffer whose depth is not kept */
	if (!stores)
		v3d_cl_emit(list, STORE_TILE_BUFFER_GENERAL, none) {
			none.buffer_to_store = V3D_STORE_BUFFER_NONE;
		}
	/* job->clear_tlb (v3dx_rcl.c:330-337, GFXH-1461/GFXH-1689): every tile
	 * starts from the clear values, Z/S included, which is what keeps Z/S
	 * per tile without a buffer in memory */
	v3d_cl_emit(list, CLEAR_TILE_BUFFERS, clear) {
		clear.clear_all_render_targets = true;
		clear.clear_z_stencil_buffer = true;
	}
	v3d_cl_emit(list, END_OF_TILE_MARKER, eot);
	v3d_cl_emit(list, RETURN_FROM_SUB_LIST, ret);
	*end = v3d_cl_gpu_next(list);
}

bool v3d_job_end(struct v3d_job *j, const struct v3d_job_clear *c, uint32_t *tile_list_start,
		 uint32_t *tile_list_end)
{
	if (j->broken)
		return false;
	/* a 4x job that loads RT 0 needs the samples an earlier job kept: a
	 * resolved image loaded back would not be what a 4x surface holds */
	if (j->t.msaa && c->load_rt0 && !j->t.ms_address) {
		j->broken = true;
		return false;
	}

	/* v3dX(bcl_epilogue): no TF, no primitive counts */
	v3d_cl_emit(j->bcl, FLUSH, flush);

	struct v3d_tiling t;
	v3d_frame_tiling_ms(j->t.width, j->t.height, job_rts(&j->t), j->t.msaa, &t);
	uint32_t list_start, list_end;
	emit_tile_list(j, c, j->ind, &list_start, &list_end);
	if (tile_list_start)
		*tile_list_start = list_start;
	if (tile_list_end)
		*tile_list_end = list_end;

	/* v3dX(emit_rcl) (v42) and emit_render_layer, layer 0 */
	struct v3d_cl *rcl = j->rcl;
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_COMMON, common) {
		common.number_of_render_targets = job_rts(&j->t);
		common.image_width_pixels = j->t.width;
		common.image_height_pixels = j->t.height;
		common.maximum_bpp_of_all_render_targets = V3D_INTERNAL_BPP_32;
		/* early Z off (see v3d_job.h); the tile buffer's Z/S is D24S8:
		 * internal type depth_24, what a Z24S8 zsbuf gives */
		common.early_z_disable = true;
		common.internal_depth_type = V3D_INTERNAL_TYPE_DEPTH_24;
		common.multisample_mode_4x = j->t.msaa;
	}
	/* R8G8B8A8_UNORM clear colour, internal type 8: 32 bits (v3d_tlb_clear),
	 * one PART1 per RT in RT order (v3dx_rcl.c:833-839); then the COLOR
	 * packet with every configured RT RGBA8 (:898-913); an RT past the
	 * job's count stays 0, as v3d_setup_render_target leaves it */
	for (uint32_t n = 0; n < job_rts(&j->t); n++) {
		bool load = n == 0 ? c->load_rt0 : c->load_rt[n - 1];
		uint32_t rgba8 = n == 0 ? c->rgba8 : c->rgba8_rt[n - 1];
		v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1, clear) {
			clear.render_target_number = n;
			clear.clear_color_low_32_bits = load ? 0 : rgba8;
		}
	}
	const uint32_t rts = job_rts(&j->t);
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_COLOR, color) {
		color.render_target_0_internal_bpp = V3D_INTERNAL_BPP_32;
		color.render_target_0_internal_type = V3D_INTERNAL_TYPE_8;
		color.render_target_0_clamp = V3D_RENDER_TARGET_CLAMP_NONE;
		if (rts > 1) {
			color.render_target_1_internal_bpp = V3D_INTERNAL_BPP_32;
			color.render_target_1_internal_type = V3D_INTERNAL_TYPE_8;
			color.render_target_1_clamp = V3D_RENDER_TARGET_CLAMP_NONE;
		}
		if (rts > 2) {
			color.render_target_2_internal_bpp = V3D_INTERNAL_BPP_32;
			color.render_target_2_internal_type = V3D_INTERNAL_TYPE_8;
			color.render_target_2_clamp = V3D_RENDER_TARGET_CLAMP_NONE;
		}
		if (rts > 3) {
			color.render_target_3_internal_bpp = V3D_INTERNAL_BPP_32;
			color.render_target_3_internal_type = V3D_INTERNAL_TYPE_8;
			color.render_target_3_clamp = V3D_RENDER_TARGET_CLAMP_NONE;
		}
	}
	v3d_cl_emit(rcl, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, zs) {
		zs.z_clear_value = c->z;
		zs.stencil_clear_value = c->stencil;
	}
	v3d_cl_emit(rcl, TILE_LIST_INITIAL_BLOCK_SIZE, init) {
		init.size_of_first_block_in_chained_tile_lists = V3D_JOB_TILE_ALLOC_INITIAL_BLOCK;
		init.use_auto_chained_tile_lists = true;
	}

	v3d_cl_emit(rcl, MULTICORE_RENDERING_TILE_LIST_SET_BASE, base) {
		base.tile_list_set_number = 0;
		base.address = j->t.tile_alloc;
	}
	v3d_cl_emit(rcl, MULTICORE_RENDERING_SUPERTILE_CFG, sc) {
		sc.supertile_width_in_tiles = t.supertile_w;
		sc.supertile_height_in_tiles = t.supertile_h;
		sc.total_frame_width_in_supertiles = t.frame_w_supertiles;
		sc.total_frame_height_in_supertiles = t.frame_h_supertiles;
		sc.total_frame_width_in_tiles = t.tiles_x;
		sc.total_frame_height_in_tiles = t.tiles_y;
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
	uint32_t stw = t.tile_w * t.supertile_w, sth = t.tile_h * t.supertile_h;
	for (uint32_t y = 0; y <= (j->t.height - 1) / sth; y++) {
		for (uint32_t x = 0; x <= (j->t.width - 1) / stw; x++) {
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
	pack_totals.emitted += j->pack.n.emitted;
	pack_totals.skipped += j->pack.n.skipped;
	pack_totals.compared += j->pack.n.compared;
	pack_totals.mismatched += j->pack.n.mismatched;
	pack_jobs++;
	return true;
}
