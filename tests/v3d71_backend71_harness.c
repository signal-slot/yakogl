/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Test-only harness of tests/v3d71_backend71.rs: the driver's V3D 7.1 job
 * builder (driver/src/v3d/v3d_job71.c over v3d_frame.c, -DV3D_CL_VER=71)
 * run in its own process on what src/v3d/backend71.rs hands it. The pointer
 * fields of the structs cannot cross a pipe, so the Rust side sends every
 * struct's raw bytes (the src/v3d/ffi71.rs mirrors) with the arrays after
 * them, and this file points the structs at its own copies.
 *
 * stdin, native byte order:
 *   u32 magic 0x37314a42 ("BJ17"), u32 draws
 *   struct v3d_job71_target, struct v3d_job71_clear
 *   u32 bcl_gpu, rcl_gpu, ind_gpu, cl_size, page_size
 *   per draw: struct v3d_job71_state, struct v3d_fs_prog, struct
 *     v3d_vs_prog vs, cs; u32 prim, count, first; u32 n_fs_u, n_vs_u,
 *     n_cs_u, n_fs_gl, n_vs_gl, n_attrs; then the arrays in that order
 *     (struct v3d_uniform, u32 words, struct v3d_vertex_attr)
 * stdout:
 *   i32 status: 0 ok, 1 v3d_job71_begin false, 2 a draw not recorded, 3
 *     v3d_job71_end false; u32 the draw index, i32 v3d_job71_draw's result
 *   struct v3d_tiling (after begin)
 *   per recorded draw: u32 fs_uniforms, vs_uniforms, cs_uniforms, record,
 *     attr_records (struct v3d_job71_draw_layout's addresses)
 *   u32 tile_list_start, tile_list_end, tile_alloc_size, tile_state_size
 *   u32 bcl used, the bytes; rcl used, the bytes; ind used, the bytes
 * Exit 0 when the output is complete (whatever the status), 2 for a short
 * or bad input. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_job71.h"

#define MAX_DRAWS 16
#define MAX_WORDS 256

struct in_draw {
	struct v3d_job71_state st;
	struct v3d_fs_prog fs;
	struct v3d_vs_prog vs, cs;
	struct v3d_uniform fs_u[MAX_WORDS], vs_u[MAX_WORDS], cs_u[MAX_WORDS];
	uint32_t fs_gl[MAX_WORDS], vs_gl[MAX_WORDS];
	struct v3d_vertex_attr attrs[V3D_MAX_ATTRS];
	struct v3d_job71_draw d;
};

static struct in_draw draws[MAX_DRAWS];

static int get(void *p, size_t n)
{
	return n == 0 || fread(p, n, 1, stdin) == 1;
}

static void put(const void *p, size_t n)
{
	if (n)
		fwrite(p, n, 1, stdout);
}

static void put32(uint32_t v)
{
	put(&v, 4);
}

int main(void)
{
	uint32_t magic, n, bcl_gpu, rcl_gpu, ind_gpu, cl_size, page_size;
	static struct v3d_job71_target t;
	static struct v3d_job71_clear c;

	if (!get(&magic, 4) || magic != 0x37314a42u || !get(&n, 4) || n > MAX_DRAWS || !get(&t, sizeof t) ||
	    !get(&c, sizeof c) || !get(&bcl_gpu, 4) || !get(&rcl_gpu, 4) || !get(&ind_gpu, 4) || !get(&cl_size, 4) ||
	    !get(&page_size, 4))
		return 2;
	for (uint32_t i = 0; i < n; i++) {
		struct in_draw *x = &draws[i];
		uint32_t prim, count, first, nfu, nvu, ncu, nfg, nvg, na;
		if (!get(&x->st, sizeof x->st) || !get(&x->fs, sizeof x->fs) || !get(&x->vs, sizeof x->vs) ||
		    !get(&x->cs, sizeof x->cs) || !get(&prim, 4) || !get(&count, 4) || !get(&first, 4) ||
		    !get(&nfu, 4) || !get(&nvu, 4) || !get(&ncu, 4) || !get(&nfg, 4) || !get(&nvg, 4) || !get(&na, 4))
			return 2;
		if (nfu > MAX_WORDS || nvu > MAX_WORDS || ncu > MAX_WORDS || nfg > MAX_WORDS || nvg > MAX_WORDS ||
		    na > V3D_MAX_ATTRS)
			return 2;
		if (!get(x->fs_u, nfu * sizeof x->fs_u[0]) || !get(x->vs_u, nvu * sizeof x->vs_u[0]) ||
		    !get(x->cs_u, ncu * sizeof x->cs_u[0]) || !get(x->fs_gl, nfg * 4) || !get(x->vs_gl, nvg * 4) ||
		    !get(x->attrs, na * sizeof x->attrs[0]))
			return 2;
		x->fs.uniforms = x->fs_u;
		x->fs.num_uniforms = nfu;
		x->vs.uniforms = x->vs_u;
		x->vs.num_uniforms = nvu;
		x->cs.uniforms = x->cs_u;
		x->cs.num_uniforms = ncu;
		memset(&x->d, 0, sizeof x->d);
		x->d.st = &x->st;
		x->d.fs = &x->fs;
		x->d.vs = &x->vs;
		x->d.cs = &x->cs;
		x->d.fs_storage = (struct v3d_uniform_storage){ x->fs_gl, nfg };
		x->d.vs_storage = (struct v3d_uniform_storage){ x->vs_gl, nvg };
		x->d.attrs = x->attrs;
		x->d.num_attrs = na;
		x->d.prim = prim;
		x->d.count = count;
		x->d.first = first;
	}

	uint8_t *mem = calloc(3, cl_size);
	if (!mem)
		return 2;
	struct v3d_cl bcl, rcl, ind;
	v3d_cl_init(&bcl, mem, bcl_gpu, cl_size);
	v3d_cl_init(&rcl, mem + cl_size, rcl_gpu, cl_size);
	v3d_cl_init(&ind, mem + 2 * cl_size, ind_gpu, cl_size);

	struct v3d_job71 j;
	struct v3d_job71_draw_layout lo[MAX_DRAWS];
	int32_t status = 0;
	uint32_t at = 0, recorded = 0, ls = 0, le = 0, qms = 0, qts = 0;
	int32_t code = 0;
	memset(&j, 0, sizeof j);
	if (!v3d_job71_begin(&j, &t, &bcl, &rcl, &ind)) {
		status = 1;
	} else {
		for (; recorded < n; recorded++) {
			memset(&lo[recorded], 0, sizeof lo[recorded]);
			code = v3d_job71_draw(&j, &draws[recorded].d, &lo[recorded]);
			if (code != V3D_JOB71_OK) {
				status = 2;
				at = recorded;
				break;
			}
		}
		if (status == 0) {
			if (!v3d_job71_end(&j, &c, &ls, &le))
				status = 3;
			else
				v3d_job71_alloc_sizes(&j, page_size, &qms, &qts);
		}
	}

	put(&status, 4);
	put32(at);
	put(&code, 4);
	put(&j.tiling, sizeof j.tiling);
	put32(recorded);
	for (uint32_t i = 0; i < recorded; i++) {
		put32(lo[i].fs_uniforms);
		put32(lo[i].vs_uniforms);
		put32(lo[i].cs_uniforms);
		put32(lo[i].record);
		put32(lo[i].attr_records);
	}
	put32(ls);
	put32(le);
	put32(qms);
	put32(qts);
	put32(bcl.used);
	put(bcl.buf, bcl.used);
	put32(rcl.used);
	put(rcl.buf, rcl.used);
	put32(ind.used);
	put(ind.buf, ind.used);
	free(mem);
	return fflush(stdout) == 0 ? 0 : 2;
}
