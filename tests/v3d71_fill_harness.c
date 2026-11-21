/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * v3d71_fill_harness: test-only. Runs the V3D 7.1 job builder
 * (driver/src/v3d/v3d_job71.c over v3d_frame.c, both with -DV3D_CL_VER=71)
 * on the descriptors tests/v3d71_fill.rs fills with src/v3d/fill71.rs, and
 * prints the control lists it writes, for the test to decode and compare
 * with Mesa's 7.1 references (tools/v3d/pi5/gl_*.clif).
 *
 * Input (stdin): hexadecimal numbers separated by white space, the structs
 * of v3d_job71.h field by field in the order the readers below take them
 * (the test writes the same order; markers catch a drift). Output (stdout),
 * per job:
 *   job <k> <begin ok 0/1>
 *   draw <i> <V3D_JOB71_* result>
 *   end <ok 0/1>
 *   qms <tile allocation size at a 4096-byte granule>
 *   bcl|rcl|ind <gpu address> <bytes used> <the bytes as hex>
 *   tiles <tile list start> <tile list end>
 * Built by the test with $CC/cc; no OS call besides stdio.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_job71.h"

#if V3D_CL_VER != 71
#error "v3d71_fill_harness is built with -DV3D_CL_VER=71"
#endif

#define MARK_JOB 0xf171u
#define MARK_DRAW 0xd71u
#define MARK_END 0xe71u

#define H_BCL 0x100000u
#define H_RCL 0x110000u
#define H_IND 0x120000u
#define H_CL 0x8000u
#define H_PAGE 4096u /* the drm-shim's buffer granule, as job71_check's gl mode */

static uint8_t h_bcl[H_CL], h_rcl[H_CL], h_ind[H_CL];

static unsigned long long rd(void)
{
	unsigned long long v;
	if (scanf("%llx", &v) != 1) {
		fprintf(stderr, "v3d71_fill_harness: short input\n");
		exit(2);
	}
	return v;
}

static uint32_t u(void)
{
	return (uint32_t)rd();
}

static bool b(void)
{
	return rd() != 0;
}

static float f(void)
{
	uint32_t x = u();
	float v;
	memcpy(&v, &x, sizeof(v));
	return v;
}

static void mark(uint32_t m)
{
	uint32_t got = u();
	if (got != m) {
		fprintf(stderr, "v3d71_fill_harness: marker %x where %x was due\n", got, m);
		exit(2);
	}
}

static void rd_zs(struct v3d_job71_zs *z)
{
	z->address = u();
	z->format = u();
	z->memory_format = u();
	z->height_in_ub = u();
}

static void rd_target(struct v3d_job71_target *t)
{
	memset(t, 0, sizeof(*t));
	t->width = u();
	t->height = u();
	t->tile_alloc = u();
	t->rts = u();
	for (int n = 0; n < V3D_JOB71_MAX_RTS; n++) {
		struct v3d_job71_rt *rt = &t->rt[n];
		rt->address = u();
		rt->format = u();
		rt->memory_format = u();
		rt->height_in_ub_or_stride = u();
		rt->r_b_swap = b();
		rt->force_alpha_1 = b();
	}
	t->depth_type = u();
	rd_zs(&t->zs);
	rd_zs(&t->s);
}

static void rd_clear(struct v3d_job71_clear *c)
{
	memset(c, 0, sizeof(*c));
	c->load_rts = u();
	for (int n = 0; n < V3D_JOB71_MAX_RTS; n++)
		for (int w = 0; w < 4; w++)
			c->color[n][w] = u();
	c->z = f();
	c->stencil = u();
	c->zs_load = b();
	c->zs_store = b();
	c->s_load = b();
	c->s_store = b();
	c->zs_cleared = b();
	c->zs_load_aspects = (uint8_t)u();
	c->zs_store_aspects = (uint8_t)u();
}

static void rd_face(struct v3d_job71_stencil_face *s)
{
	s->func = u();
	s->sfail_op = u();
	s->zfail_op = u();
	s->zpass_op = u();
	s->ref = u();
	s->value_mask = u();
	s->write_mask = u();
}

static void rd_state(struct v3d_job71_state *s)
{
	memset(s, 0, sizeof(*s));
	for (int i = 0; i < 3; i++)
		s->vp.scale[i] = f();
	for (int i = 0; i < 3; i++)
		s->vp.translate[i] = f();
	s->scissor = b();
	s->scissor_minx = u();
	s->scissor_miny = u();
	s->scissor_maxx = u();
	s->scissor_maxy = u();
	s->front_ccw = b();
	s->cull_front = b();
	s->cull_back = b();
	s->offset_tri = b();
	s->offset_units = f();
	s->offset_scale = f();
	s->offset_clamp = f();
	s->line_width = f();
	s->depth_enabled = b();
	s->depth_writemask = b();
	s->depth_func = u();
	s->stencil_enabled = b();
	s->stencil_back_enabled = b();
	rd_face(&s->stencil_front);
	rd_face(&s->stencil_back);
	s->blend_enabled = b();
	s->rgb_func = u();
	s->rgb_src = u();
	s->rgb_dst = u();
	s->alpha_func = u();
	s->alpha_src = u();
	s->alpha_dst = u();
	s->colormask = u();
	for (int i = 0; i < 4; i++)
		s->blend_color_f16[i] = (uint16_t)u();
	s->wpos.y_scale = f();
	s->wpos.y_offset = f();
	s->sample_mask = u();
	s->oq_addr = u();
}

static const struct v3d_uniform *rd_uniforms(uint32_t *n)
{
	*n = u();
	struct v3d_uniform *list = calloc(*n ? *n : 1, sizeof(*list));
	for (uint32_t i = 0; i < *n; i++) {
		list[i].contents = u();
		list[i].data = u();
	}
	return list;
}

static void rd_fs(struct v3d_fs_prog *p)
{
	memset(p, 0, sizeof(*p));
	p->code = u();
	p->threads = u();
	p->single_seg = b();
	p->num_inputs = u();
	p->flat_shade_flags = u();
	p->noperspective_flags = u();
	p->centroid_flags = u();
	p->flat_shade_flags_hi = u();
	p->noperspective_flags_hi = u();
	p->centroid_flags_hi = u();
	p->writes_z = b();
	p->disable_ez = b();
	p->uses_center_w = b();
	p->lock_scoreboard_on_first_thrsw = b();
	p->uses_implicit_point_line_varyings = b();
	p->uses_pid = b();
	p->uniforms = rd_uniforms(&p->num_uniforms);
}

static void rd_vs(struct v3d_vs_prog *p)
{
	memset(p, 0, sizeof(*p));
	p->code = u();
	p->threads = u();
	p->single_seg = b();
	p->vpm_input_size = u();
	p->vpm_output_size = u();
	p->separate_segments = b();
	p->vcm_cache_size = u();
	p->uses_vid = b();
	p->uses_iid = b();
	p->uses_biid = b();
	for (int i = 0; i < V3D_MAX_ATTRS; i++)
		p->vattr_sizes[i] = (uint8_t)u();
	p->uniforms = rd_uniforms(&p->num_uniforms);
}

static void rd_storage(struct v3d_uniform_storage *s)
{
	s->count = u();
	uint32_t *w = calloc(s->count ? s->count : 1, sizeof(*w));
	for (uint32_t i = 0; i < s->count; i++)
		w[i] = u();
	s->words = s->count ? w : NULL;
}

static void put_cl(const char *name, const struct v3d_cl *cl)
{
	printf("%s %x %x ", name, cl->gpu, cl->used);
	for (uint32_t i = 0; i < cl->used; i++)
		printf("%02x", cl->buf[i]);
	printf("\n");
}

int main(void)
{
	uint32_t jobs = u();
	for (uint32_t k = 0; k < jobs; k++) {
		mark(MARK_JOB);
		struct v3d_job71_target t;
		struct v3d_job71_clear c;
		rd_target(&t);
		rd_clear(&c);
		uint32_t ndraws = u();
		struct v3d_cl bcl, rcl, ind;
		memset(h_bcl, 0, sizeof(h_bcl));
		memset(h_rcl, 0, sizeof(h_rcl));
		memset(h_ind, 0, sizeof(h_ind));
		v3d_cl_init(&bcl, h_bcl, H_BCL, H_CL);
		v3d_cl_init(&rcl, h_rcl, H_RCL, H_CL);
		v3d_cl_init(&ind, h_ind, H_IND, H_CL);
		struct v3d_job71 j;
		bool ok = v3d_job71_begin(&j, &t, &bcl, &rcl, &ind);
		printf("job %u %d\n", k, ok ? 1 : 0);
		for (uint32_t i = 0; i < ndraws; i++) {
			mark(MARK_DRAW);
			struct v3d_job71_state *st = calloc(1, sizeof(*st));
			struct v3d_fs_prog *fs = calloc(1, sizeof(*fs));
			struct v3d_vs_prog *vs = calloc(1, sizeof(*vs)), *cs = calloc(1, sizeof(*cs));
			struct v3d_job71_draw d;
			memset(&d, 0, sizeof(d));
			rd_state(st);
			rd_fs(fs);
			rd_vs(vs);
			rd_vs(cs);
			rd_storage(&d.vs_storage);
			rd_storage(&d.fs_storage);
			d.num_attrs = u();
			struct v3d_vertex_attr *attrs = calloc(d.num_attrs ? d.num_attrs : 1, sizeof(*attrs));
			for (uint32_t a = 0; a < d.num_attrs; a++) {
				attrs[a].address = u();
				attrs[a].stride = u();
				attrs[a].components = u();
				attrs[a].divisor = u();
			}
			d.st = st;
			d.fs = fs;
			d.vs = vs;
			d.cs = cs;
			d.attrs = d.num_attrs ? attrs : NULL;
			d.prim = u();
			d.count = u();
			d.first = u();
			d.indexed = b();
			d.index_type = u();
			d.index_buffer = u();
			d.index_buffer_bytes = u();
			d.index_offset = u();
			d.vs_writes_psiz = b();
			d.instances = u();
			int r = ok ? v3d_job71_draw(&j, &d, NULL) : V3D_JOB71_REFUSED;
			printf("draw %u %d\n", i, r);
		}
		mark(MARK_END);
		uint32_t start = 0, end = 0;
		bool done = ok && v3d_job71_end(&j, &c, &start, &end);
		printf("end %d\n", done ? 1 : 0);
		if (done) {
			uint32_t qms = 0, qts = 0;
			v3d_job71_alloc_sizes(&j, H_PAGE, &qms, &qts);
			printf("qms %u\n", qms);
			put_cl("bcl", &bcl);
			put_cl("rcl", &rcl);
			put_cl("ind", &ind);
			printf("tiles %x %x\n", start, end);
		}
	}
	return 0;
}
