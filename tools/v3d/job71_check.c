/* SPDX-License-Identifier: MIT
 *
 * job71_check: the V3D 7.1 job builder (driver/src/v3d/v3d_job71.c, built
 * with -DV3D_CL_VER=71 over v3d_frame.c at 7.1) decoded by Mesa's own CLE
 * decoder (clif_dump for a version 71 device), against the jobs Mesa 26.2.2
 * submitted on the Raspberry Pi 5 (tools/v3d/pi5/clear.clif and
 * triangle.clif). tools/v3d/check71.d/job71.sh builds and runs it and
 * compares the CLIF text this writes with the board's through
 * tools/v3d/clif_cmp.py.
 *
 *   job71_check clear      the 64x64 clear to 0xffff0000 (no draw), every
 *                          buffer at clear.clif's address and under its name
 *   job71_check triangle   the same clear then Mesa's triangle with Mesa's
 *                          7.1 shaders (tools/v3d/tri-shaders71), at
 *                          triangle.clif's addresses
 *   job71_check multi      a 200x100 job of three draws (arrays, indexed,
 *                          instanced) into 8 render targets of mixed formats
 *                          and tilings (one GL_NONE) with a D24S8 buffer
 *                          loaded and stored: the decoder reads every list
 *                          in full, the packet census is the job's, and the
 *                          render-target packets are the expected bytes
 *   job71_check refusals   inputs the builder must refuse, each refused
 *
 * JOB71_CLIF=<file> stores the decoded CLIF text of clear, triangle or multi.
 * The last line is "job71_check: <mode> ...: ok" or ends "FAIL". Built with
 * the cc line of check71.d/job71.sh (Mesa's include paths, -DV3D_CL_VER=71,
 * -Itools/v3d/tri-shaders71, the reference build's libmesa_ref.a).
 */
#include <stdarg.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "common/v3d_device_info.h"
#include "clif/clif_dump.h"
#include "drm-uapi/v3d_drm.h"
#include "util/log.h"

#include "v3d_job71.h"

#include "mesa_passthrough_interp_color.h"

#if V3D_CL_VER != 71
#error "job71_check is built with -DV3D_CL_VER=71"
#endif

static int failures;

__attribute__((format(printf, 1, 2))) static void fail(const char *fmt, ...)
{
	va_list ap;
	failures++;
	printf("job71_check: FAIL ");
	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
	printf("\n");
}

/* One buffer of a dump: its name, address, size and contents. */
struct bo {
	const char *name;
	uint32_t gpu, size;
	uint8_t *cpu;
};

/* Decode the job with Mesa's clif_dump (the CLIF form V3D_DEBUG=clif
 * writes, not pretty) and return its text, malloc'd. */
static char *decode(const struct bo *bos, unsigned n, const struct drm_v3d_submit_cl *submit)
{
	const char *dir = getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp";
	char path[512];
	snprintf(path, sizeof(path), "%s/job71_check_%d.log", dir, (int)getpid());
	setenv("MESA_LOG_FILE", path, 1);
	setenv("MESA_LOG_LEVEL", "info", 1);
	setenv("MESA_LOG_PREFIX", "", 1);
	struct v3d_device_info devinfo = { .ver = 71, .has_accumulators = false, .cle_readahead = 256 };
	struct log_stream *stream = mesa_log_streami();
	struct clif_dump *clif = clif_dump_init(&devinfo, stream, false, false);
	for (unsigned i = 0; i < n; i++)
		clif_dump_add_bo(clif, bos[i].name, bos[i].gpu, bos[i].size, bos[i].cpu);
	struct drm_v3d_submit_cl s = *submit;
	clif_dump(clif, &s);
	clif_dump_destroy(clif);
	mesa_log_stream_destroy(stream);

	FILE *in = fopen(path, "r");
	if (!in)
		return NULL;
	fseek(in, 0, SEEK_END);
	long len = ftell(in);
	fseek(in, 0, SEEK_SET);
	char *text = malloc(len + 1);
	size_t got = fread(text, 1, len, in);
	text[got] = 0;
	fclose(in);
	unlink(path);
	const char *out_path = getenv("JOB71_CLIF");
	if (out_path) {
		FILE *out = fopen(out_path, "w");
		if (!out || fputs(text, out) < 0 || fclose(out) != 0)
			fail("cannot write %s", out_path);
	}
	return text;
}

/* Lines whose first word is exactly `name`. */
static int census(const char *text, const char *name)
{
	int n = 0;
	size_t len = strlen(name);
	for (const char *line = text; *line;) {
		const char *p = line;
		while (*p == ' ')
			p++;
		if (strncmp(p, name, len) == 0 && (p[len] == ' ' || p[len] == '\n' || p[len] == 0))
			n++;
		const char *nl = strchr(line, '\n');
		if (!nl)
			break;
		line = nl + 1;
	}
	return n;
}

/* The structural checks of every decode: no decoder error, every
 * unindented upper-case line a packet of the census (`total` of them), and
 * binary data only in buffers named resource_* and where `ind_bin` says
 * (a "[<indirect buffer>+<offset>]" prefix: the uniform streams). Returns
 * the number of control-list sections. */
static int structure(const char *text, int total, const char *ind_bin)
{
	if (strstr(text, "Failed") || strstr(text, "nknown"))
		fail("the decoder reported an error");
	int lists = 0, packets = 0;
	for (const char *line = text; *line;) {
		const char *nl = strchr(line, '\n');
		size_t len = nl ? (size_t)(nl - line) : strlen(line);
		if (len >= 16 && strncmp(line, "@format ctrllist", 16) == 0)
			lists++;
		if (len >= 14 && strncmp(line, "@format binary", 14) == 0) {
			char l[256];
			snprintf(l, sizeof(l), "%.*s", (int)(len < 255 ? len : 255), line);
			if (!strstr(l, "[resource_") && !strstr(l, ind_bin))
				fail("bytes of a list left undecoded: %s", l);
		}
		size_t k = 0;
		while (k < len && ((line[k] >= 'A' && line[k] <= 'Z') || line[k] == '_' ||
				   (k > 0 && line[k] >= '0' && line[k] <= '9')))
			k++;
		if (k > 0 && k == len)
			packets++;
		if (!nl)
			break;
		line = nl + 1;
	}
	if (packets != total)
		fail("%d packets decoded, the census has %d", packets, total);
	return lists;
}

struct want {
	const char *name;
	int n;
};

static int check_census(const char *text, const struct want *w, unsigned n)
{
	int total = 0;
	for (unsigned i = 0; i < n; i++) {
		int got = census(text, w[i].name);
		total += got;
		if (got != w[i].n)
			fail("%s: decoded %d, expected %d", w[i].name, got, w[i].n);
	}
	return total;
}

/* ------------------------------------------------------------------------ */
/* The board's jobs: 64x64 RGBA8, a UIF (no XOR) texture of 8 UIF blocks'
 * padded height, cleared to 0xffff0000 (RENDER_TARGET_PART1's "Clear Color
 * low bits" 4294901760); the Pi 5 kernel's 16 KiB buffer granule. */

#define W 64u
#define H 64u
#define CLEAR 0xffff0000u
#define PAGE 16384u

/* The buffers of a board dump (its @createbuf lines, in their order); the
 * sizes are the dump's buffer sizes (its blank spans). */
struct place {
	uint32_t code, code_size, target, target_size, bcl, vertex, vertex_size, indirect, tile_alloc,
		tile_alloc_size, tsda, tsda_size, rcl, cl_size;
};
/* tools/v3d/pi5/clear.clif: no code and no vertex buffer; the tile list in
 * its own CL buffer (CL_0x21000), our indirect buffer */
static const struct place CLEAR_PLACE = { 0, 0, 0x9000, 0x5000, 0x11000, 0, 0, 0x21000, 0x15000,
					  0x4000, 0x19000, 0x1000, 0x1d000, 0x4000 };
/* tools/v3d/pi5/triangle.clif (frame_check.c's TRI71_PLACES "pi5") */
static const struct place TRI_PLACE = { 0x1000, 0x5000, 0x9000, 0x5000, 0x11000, 0x15000, 0x101000,
					0x119000, 0x11d000, 0x4000, 0x121000, 0x1000, 0x125000, 0x4000 };

static uint8_t b_code[0x5000], b_target[0x5000], b_bcl[0x4000], b_vertex[0x101000], b_ind[0x4000],
	b_tile[0x4000], b_tsda[0x1000], b_rcl[0x4000];

static void board_target(const struct place *p, struct v3d_job71_target *t)
{
	memset(t, 0, sizeof(*t));
	t->width = W;
	t->height = H;
	t->tile_alloc = p->tile_alloc;
	t->rts = 1;
	t->rt[0] = (struct v3d_job71_rt){
		.address = p->target,
		.format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8,
		.memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR,
		.height_in_ub_or_stride = 8, /* 64 rows / (2 * a 4-row utile) */
	};
	t->depth_type = V3D_INTERNAL_TYPE_DEPTH_32F; /* no zsbuf */
}

static void board_clear(struct v3d_job71_clear *c)
{
	memset(c, 0, sizeof(*c));
	c->color[0][0] = CLEAR;
	c->z = 0.0f; /* the dump's Z clear value (glClear(COLOR) only) */
}

/* gl_ref.py's triangle (the dump's vertex buffer): positions, then colours */
static const float TRI_VERTICES[24] = {
	-0.75f, -0.75f, 0.0f, 1.0f, 0.75f, -0.75f, 0.0f, 1.0f, 0.0f, 0.75f, 0.0f, 1.0f,
	1.0f,   0.0f,   0.0f, 1.0f, 0.0f,  1.0f,   0.0f, 1.0f, 0.0f, 0.0f,  1.0f, 1.0f,
};
#define TRI_COLOR_OFFSET 0x30u

static struct v3d_uniform vs_u[MESA71_TRI_VS_UNIFORM_COUNT], cs_u[MESA71_TRI_CS_UNIFORM_COUNT];
static struct v3d_fs_prog tri_fs;
static struct v3d_vs_prog tri_vs, tri_cs;
static struct v3d_vertex_attr tri_attrs[MESA71_TRI_ATTR_COUNT];
static struct v3d_job71_state tri_st;

/* Mesa's shaders, as the header gives them, at the dump's code addresses
 * (FS, VS, CS back to back from code + 0x60), and the draw of the dump. */
static void tri_draw(const struct place *p, struct v3d_job71_draw *d)
{
	uint32_t at = 0x60;
	const struct {
		const uint64_t *w;
		uint32_t n;
	} code[3] = {
		{ mesa71_tri_fs_code, MESA71_TRI_FS_CODE_COUNT },
		{ mesa71_tri_vs_code, MESA71_TRI_VS_CODE_COUNT },
		{ mesa71_tri_cs_code, MESA71_TRI_CS_CODE_COUNT },
	};
	uint32_t addr[3];
	memset(b_code, 0, sizeof(b_code));
	for (int s = 0; s < 3; s++) {
		addr[s] = p->code + at;
		for (uint32_t i = 0; i < code[s].n; i++, at += 8)
			for (int b = 0; b < 8; b++)
				b_code[at + b] = (uint8_t)(code[s].w[i] >> (8 * b));
	}
	for (uint32_t i = 0; i < MESA71_TRI_VS_UNIFORM_COUNT; i++)
		vs_u[i] = (struct v3d_uniform){
			mesa71_tri_vs_uniform_kind[i],
			mesa71_tri_vs_uniform_kind[i] == MESA71_TRI_UNIFORM_CONSTANT ? mesa71_tri_vs_uniform[i]
										    : 0,
		};
	for (uint32_t i = 0; i < MESA71_TRI_CS_UNIFORM_COUNT; i++)
		cs_u[i] = (struct v3d_uniform){
			mesa71_tri_cs_uniform_kind[i],
			mesa71_tri_cs_uniform_kind[i] == MESA71_TRI_UNIFORM_CONSTANT ? mesa71_tri_cs_uniform[i]
										    : 0,
		};
	tri_fs = (struct v3d_fs_prog){
		.code = addr[0],
		.threads = MESA71_TRI_FS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_FS_FINAL_THREAD_SECTION,
		.num_inputs = MESA71_TRI_REC_VARYINGS,
		.writes_z = MESA71_TRI_REC_FS_Z_WRITES,
		.disable_ez = MESA71_TRI_REC_EARLY_Z_OFF,
		.uses_center_w = MESA71_TRI_REC_FS_USES_REAL_CENTRE_W,
		.lock_scoreboard_on_first_thrsw = MESA71_TRI_REC_SCOREBOARD_WAIT_FIRST_THRSW,
		.uses_implicit_point_line_varyings = !MESA71_TRI_REC_DISABLE_IMPLICIT_PL_VARYINGS,
		.uses_pid = MESA71_TRI_REC_INSERT_PRIM_ID,
	};
	tri_vs = (struct v3d_vs_prog){
		.code = addr[1],
		.threads = MESA71_TRI_VS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_VS_FINAL_THREAD_SECTION,
		.vpm_input_size = MESA71_TRI_REC_VS_IN_VPM,
		.vpm_output_size = MESA71_TRI_REC_VS_OUT_VPM,
		.vcm_cache_size = MESA71_TRI_VCM_CACHE_BATCHES,
		.uniforms = vs_u,
		.num_uniforms = MESA71_TRI_VS_UNIFORM_COUNT,
	};
	tri_cs = (struct v3d_vs_prog){
		.code = addr[2],
		.threads = MESA71_TRI_CS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_CS_FINAL_THREAD_SECTION,
		.vpm_input_size = MESA71_TRI_REC_CS_IN_VPM,
		.vpm_output_size = MESA71_TRI_REC_CS_OUT_VPM,
		.vcm_cache_size = MESA71_TRI_VCM_CACHE_BATCHES,
		.uniforms = cs_u,
		.num_uniforms = MESA71_TRI_CS_UNIFORM_COUNT,
	};
	for (uint32_t i = 0; i < MESA71_TRI_ATTR_COUNT; i++) {
		tri_vs.vattr_sizes[i] = mesa71_tri_attr[i].vs_values;
		tri_cs.vattr_sizes[i] = mesa71_tri_attr[i].cs_values;
		tri_attrs[i] = (struct v3d_vertex_attr){
			.address = p->vertex + (i ? TRI_COLOR_OFFSET : 0),
			.stride = mesa71_tri_attr[i].stride,
			.components = mesa71_tri_attr[i].vec_size,
			.divisor = mesa71_tri_attr[i].divisor,
		};
	}
	memset(&tri_st, 0, sizeof(tri_st));
	v3d_viewport_gl(0, 0, (float)W, (float)H, 0, 1, false, H, &tri_st.vp);
	tri_st.front_ccw = false; /* the dump's CFG_BITS "Clockwise Primitives: 0" */
	tri_st.line_width = 1.0f;
	tri_st.colormask = 0xf;
	tri_st.depth_func = V3D_COMPARE_FUNC_LESS;
	tri_st.sample_mask = 0xf;

	memset(d, 0, sizeof(*d));
	d->st = &tri_st;
	d->fs = &tri_fs;
	d->vs = &tri_vs;
	d->cs = &tri_cs;
	d->attrs = tri_attrs;
	d->num_attrs = MESA71_TRI_ATTR_COUNT;
	d->prim = V3D_PRIM_TRIANGLES;
	d->count = 3;
}

static void init_lists(const struct place *p, struct v3d_cl *bcl, struct v3d_cl *rcl, struct v3d_cl *ind)
{
	memset(b_bcl, 0, sizeof(b_bcl));
	memset(b_rcl, 0, sizeof(b_rcl));
	memset(b_ind, 0, sizeof(b_ind));
	v3d_cl_init(bcl, b_bcl, p->bcl, p->cl_size);
	v3d_cl_init(rcl, b_rcl, p->rcl, p->cl_size);
	v3d_cl_init(ind, b_ind, p->indirect, p->cl_size);
}

static int run_board(bool triangle)
{
	const struct place *p = triangle ? &TRI_PLACE : &CLEAR_PLACE;
	struct v3d_cl bcl, rcl, ind;
	init_lists(p, &bcl, &rcl, &ind);
	struct v3d_job71_target t;
	board_target(p, &t);
	struct v3d_job71 j;
	if (!v3d_job71_begin(&j, &t, &bcl, &rcl, &ind)) {
		fail("v3d_job71_begin refused the board's target");
		return 1;
	}
	struct v3d_job71_draw_layout lo = { 0 };
	if (triangle) {
		struct v3d_job71_draw d;
		tri_draw(p, &d);
		memset(b_vertex, 0, sizeof(b_vertex));
		memcpy(b_vertex, TRI_VERTICES, sizeof(TRI_VERTICES));
		/* a draw refused after it wrote its state (a TMU configuration
		 * uniform in the VS stream) leaves nothing: the job that
		 * follows is still the board's */
		struct v3d_uniform bad_u[MESA71_TRI_VS_UNIFORM_COUNT];
		struct v3d_vs_prog bad_vs = tri_vs;
		memcpy(bad_u, vs_u, sizeof(bad_u));
		bad_u[MESA71_TRI_VS_UNIFORM_COUNT - 1].contents = 40;
		bad_vs.uniforms = bad_u;
		struct v3d_job71_draw bad = d;
		bad.vs = &bad_vs;
		if (v3d_job71_draw(&j, &bad, NULL) != V3D_JOB71_REFUSED || j.draws != 0)
			fail("a draw with a TMU configuration uniform was not refused");
		int r = v3d_job71_draw(&j, &d, &lo);
		if (r != V3D_JOB71_OK) {
			fail("v3d_job71_draw: %d for Mesa's triangle", r);
			return 1;
		}
	}
	struct v3d_job71_clear c;
	board_clear(&c);
	uint32_t ls, le;
	if (!v3d_job71_end(&j, &c, &ls, &le)) {
		fail("v3d_job71_end refused the board's job");
		return 1;
	}
	uint32_t qms, qts_size;
	v3d_job71_alloc_sizes(&j, PAGE, &qms, &qts_size);
	struct drm_v3d_submit_cl submit = {
		.bcl_start = bcl.gpu, .bcl_end = bcl.gpu + bcl.used,
		.rcl_start = rcl.gpu, .rcl_end = rcl.gpu + rcl.used,
		.qma = p->tile_alloc, .qms = qms, .qts = p->tsda,
	};

	char names[8][32];
	struct bo bos[8];
	unsigned n = 0;
#define ADD(fmt, gpu, size, cpu)                                                                  \
	do {                                                                                       \
		snprintf(names[n], 32, fmt, (gpu));                                                \
		bos[n] = (struct bo){ names[n], (gpu), (size), (cpu) };                            \
		n++;                                                                               \
	} while (0)
	/* in the dump's order (its @createbuf lines) */
	if (triangle)
		ADD("resource_0x%x", p->code, p->code_size, b_code);
	ADD("resource_0x%x", p->target, p->target_size, b_target);
	ADD("CL_0x%x", p->bcl, p->cl_size, b_bcl);
	if (triangle) {
		ADD("resource_0x%x", p->vertex, p->vertex_size, b_vertex);
		ADD("CL_0x%x", p->indirect, p->cl_size, b_ind);
	}
	ADD("tile_alloc_0x%x", p->tile_alloc, p->tile_alloc_size, b_tile);
	ADD("TSDA_0x%x", p->tsda, p->tsda_size, b_tsda);
	ADD("CL_0x%x", p->rcl, p->cl_size, b_rcl);
	if (!triangle)
		ADD("CL_0x%x", p->indirect, p->cl_size, b_ind);
#undef ADD
	char *text = decode(bos, n, &submit);
	if (!text) {
		fail("no decoder output");
		return 1;
	}

	/* the uniform streams sit in front of the record, at the indirect
	 * buffer's start */
	char ind_name[64];
	snprintf(ind_name, sizeof(ind_name), "[CL_0x%x+0x00000000]", p->indirect);
	int lists;
	if (!triangle) {
		/* the board's clear, packet for packet (clear.clif) */
		static const struct want w[] = {
			{ "NUMBER_OF_LAYERS", 1 }, { "TILE_BINNING_MODE_CFG", 1 },
			{ "CLEAR_VCD_CACHE", 2 }, { "OCCLUSION_QUERY_COUNTER_ENABLE", 1 },
			{ "START_TILE_BINNING", 1 }, { "FLUSH", 1 },
			{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
			{ "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1", 1 },
			{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 }, { "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
			{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 }, { "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
			{ "TILE_COORDS", 2 }, { "END_LOADS", 3 }, { "STORE", 3 }, { "CLEAR_RT", 2 },
			{ "END_TILE", 3 }, { "GENERIC_TILE_LIST", 1 }, { "IMPLICIT_TILE_COORDS", 1 },
			{ "PRIM_LIST_FORMAT", 1 }, { "SET_INSTANCEID", 1 }, { "BRANCH_IMPLICIT_TILE", 1 },
			{ "RETURN", 1 }, { "SUPERTILE_COORDS", 1 }, { "END_RENDER", 1 },
		};
		lists = structure(text, check_census(text, w, sizeof(w) / sizeof(w[0])), ind_name);
		if (lists != 3)
			fail("%d control-list sections, expected 3", lists);
		if (bcl.used != 19 || rcl.used != 96 || ls != ind.gpu || le - ls != 27)
			fail("not the board's list sizes: BCL %u RCL %u tile list %u", bcl.used, rcl.used,
			     le - ls);
	} else {
		/* the board's triangle, packet for packet (triangle.clif) */
		static const struct want w[] = {
			{ "NUMBER_OF_LAYERS", 1 }, { "TILE_BINNING_MODE_CFG", 1 },
			{ "CLEAR_VCD_CACHE", 2 }, { "OCCLUSION_QUERY_COUNTER_ENABLE", 2 },
			{ "START_TILE_BINNING", 1 }, { "CLIP", 1 }, { "CFG_BITS", 1 }, { "POINT_SIZE", 1 },
			{ "LINE_WIDTH", 1 }, { "CLIPPER_XY", 1 }, { "CLIPPER_Z", 1 }, { "CLIPZ", 1 },
			{ "VIEWPORT_OFFSET", 1 }, { "BLEND_ENABLES", 1 }, { "COLOR_WMASKS", 1 },
			{ "BLEND_CCOLOR", 1 }, { "ZERO_ALL_FLATSHADE_FLAGS", 1 },
			{ "ZERO_ALL_NOPERSPECTIVE_FLAGS", 1 }, { "ZERO_ALL_CENTROID_FLAGS", 1 },
			{ "TRANSFORM_FEEDBACK_SPECS", 1 }, { "SAMPLE_STATE", 1 }, { "VCM_CACHE_SIZE", 1 },
			{ "GL_SHADER", 1 }, { "VERTEX_ARRAY_PRIMS", 1 }, { "FLUSH", 1 },
			{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
			{ "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1", 1 },
			{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 }, { "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
			{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 }, { "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
			{ "TILE_COORDS", 2 }, { "END_LOADS", 3 }, { "STORE", 3 }, { "CLEAR_RT", 2 },
			{ "END_TILE", 3 }, { "GENERIC_TILE_LIST", 1 }, { "IMPLICIT_TILE_COORDS", 1 },
			{ "PRIM_LIST_FORMAT", 1 }, { "SET_INSTANCEID", 1 }, { "BRANCH_IMPLICIT_TILE", 1 },
			{ "RETURN", 1 }, { "SUPERTILE_COORDS", 1 }, { "END_RENDER", 1 },
		};
		lists = structure(text, check_census(text, w, sizeof(w) / sizeof(w[0])), ind_name);
		/* the BCL (reopened by the decoder), the tile list, the RCL */
		if (lists != 4)
			fail("%d control-list sections, expected 4", lists);
		if (census(text, "@format shadrec_gl_main") != 1 ||
		    census(text, "@format shadrec_gl_attr") != (int)MESA71_TRI_ATTR_COUNT)
			fail("not one shader record and %u attribute records", MESA71_TRI_ATTR_COUNT);
		/* the board's indirect layout: uniforms +0x00, record +0x60,
		 * attribute records +0x80, tile list +0xa0..+0xbb */
		if (bcl.used != 126 || rcl.used != 96 || lo.record != ind.gpu + 0x60 ||
		    lo.attr_records != ind.gpu + 0x80 || ls != ind.gpu + 0xa0 || le != ind.gpu + 0xbb ||
		    lo.fs_uniforms != ind.gpu || lo.vs_uniforms != ind.gpu || lo.cs_uniforms != ind.gpu + 0x40)
			fail("not the board's layout: BCL %u RCL %u record +0x%x attrs +0x%x tile list "
			     "+0x%x..+0x%x",
			     bcl.used, rcl.used, lo.record - ind.gpu, lo.attr_records - ind.gpu,
			     ls - ind.gpu, le - ind.gpu);
		/* the record's flag dword at the D0 layout, as the board's */
		const uint8_t *rb = b_ind + (lo.record - ind.gpu);
		uint32_t flags = rb[0] | (uint32_t)rb[1] << 8 | (uint32_t)rb[2] << 16;
		if (flags != MESA71_TRI_REC_FLAGS)
			fail("the D0 flag bits 0x%06x, the board's 0x%06x", flags, MESA71_TRI_REC_FLAGS);
	}
	printf("job71_check: %s %ux%u, BCL %u RCL %u indirect %u bytes (tile list +0x%x..+0x%x), "
	       "tile allocation %u, %d lists: %s\n",
	       triangle ? "triangle" : "clear", W, H, bcl.used, rcl.used, ind.used, ls - ind.gpu,
	       le - ind.gpu, qms, lists, failures ? "FAIL" : "ok");
	free(text);
	return failures ? 1 : 0;
}

/* ------------------------------------------------------------------------ */
/* multi: the general job */

#define M_W 200u
#define M_H 100u
#define M_BCL 0x100000u
#define M_RCL 0x110000u
#define M_IND 0x120000u
#define M_CL 0x8000u
#define M_CODE 0x200000u
#define M_VTX 0x300000u
#define M_IDX 0x340000u
#define M_TILE 0x400000u
#define M_TSDA 0x500000u
#define M_RT(n) (0x1000000u + (n) * 0x100000u)
#define M_ZS 0x2000000u

static uint8_t m_bcl[M_CL], m_rcl[M_CL], m_ind[M_CL];

static int run_multi(void)
{
	struct v3d_cl bcl, rcl, ind;
	v3d_cl_init(&bcl, m_bcl, M_BCL, M_CL);
	v3d_cl_init(&rcl, m_rcl, M_RCL, M_CL);
	v3d_cl_init(&ind, m_ind, M_IND, M_CL);

	/* 8 RTs: raster RGBA8, UIF RGBA16F, raster RGBA32F, UIF-XOR R32UI,
	 * GL_NONE, raster sRGB, UIF RGBA8UI, raster R16F */
	static const struct {
		uint32_t fmt, mf, hs;
		bool none;
	} rts[8] = {
		{ V3D_OUTPUT_IMAGE_FORMAT_RGBA8, V3D_MEMORY_FORMAT_RASTER, M_W * 4, false },
		{ V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, V3D_MEMORY_FORMAT_UIF_NO_XOR, 14, false },
		{ V3D_OUTPUT_IMAGE_FORMAT_RGBA32F, V3D_MEMORY_FORMAT_RASTER, M_W * 16, false },
		{ V3D_OUTPUT_IMAGE_FORMAT_R32UI, V3D_MEMORY_FORMAT_UIF_XOR, 14, false },
		{ 0, 0, 0, true },
		{ V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8, V3D_MEMORY_FORMAT_RASTER, M_W * 4, false },
		{ V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI, V3D_MEMORY_FORMAT_UIF_NO_XOR, 14, false },
		{ V3D_OUTPUT_IMAGE_FORMAT_R16F, V3D_MEMORY_FORMAT_RASTER, M_W * 2, false },
	};
	struct v3d_job71_target t = { .width = M_W, .height = M_H, .tile_alloc = M_TILE, .rts = 8 };
	for (int n = 0; n < 8; n++)
		if (!rts[n].none)
			t.rt[n] = (struct v3d_job71_rt){ M_RT(n), rts[n].fmt, rts[n].mf, rts[n].hs,
							 n == 0, n == 0, { 0, 0 } };
	t.depth_type = V3D_INTERNAL_TYPE_DEPTH_24;
	t.zs = (struct v3d_job71_zs){ M_ZS, V3D_OUTPUT_IMAGE_FORMAT_D24S8, V3D_MEMORY_FORMAT_UIF_XOR, 14 };
	struct v3d_job71 j;
	if (!v3d_job71_begin(&j, &t, &bcl, &rcl, &ind)) {
		fail("multi: v3d_job71_begin refused the target");
		return 1;
	}
	/* 4 + 8 + 16 + 4 + 0 + 8 (sRGB at 64 bpp) + 4 + 8 (R16F at 64 bpp):
	 * 52 bytes a pixel, so 32x16 tiles (v3d_choose_tile_size, 7.x) */
	if (j.tiling.tile_w != 32 || j.tiling.tile_h != 16)
		fail("multi: tiles %ux%u, expected 32x16 for 52 bytes a pixel", j.tiling.tile_w,
		     j.tiling.tile_h);

	static uint8_t code[0x1000];
	struct v3d_job71_draw d;
	tri_draw(&(struct place){ .code = M_CODE, .vertex = M_VTX }, &d);
	memcpy(code, b_code, sizeof(code));
	static struct v3d_job71_state st;
	st = tri_st;
	v3d_viewport_gl(0, 0, (float)M_W, (float)M_H, 0, 1, true, M_H, &st.vp);
	st.depth_enabled = true;
	st.depth_writemask = true;
	st.stencil_enabled = true;
	st.stencil_front = (struct v3d_job71_stencil_face){ V3D_COMPARE_FUNC_ALWAYS, 0, 0, 2, 1, 0xff, 0xff };
	st.blend_enabled = true;
	st.rgb_func = st.alpha_func = 0;
	st.rgb_src = st.alpha_src = 1;
	st.rgb_dst = st.alpha_dst = 0;
	st.offset_tri = true;
	st.offset_units = 1.0f;
	st.offset_scale = 2.0f;
	st.colormask = 0x7;
	st.scissor = true;
	st.scissor_minx = 10;
	st.scissor_miny = 5;
	st.scissor_maxx = 150;
	st.scissor_maxy = 90;
	d.st = &st;
	int r[3];
	r[0] = v3d_job71_draw(&j, &d, NULL);
	d.indexed = true;
	d.index_type = V3D_INDEX_TYPE_16;
	d.index_buffer = M_IDX;
	d.index_buffer_bytes = 64;
	d.prim = V3D_PRIM_TRIANGLE_STRIP;
	d.count = 5;
	r[1] = v3d_job71_draw(&j, &d, NULL);
	d.indexed = false;
	d.prim = V3D_PRIM_TRIANGLES;
	d.count = 3;
	d.instances = 4;
	r[2] = v3d_job71_draw(&j, &d, NULL);
	for (int k = 0; k < 3; k++)
		if (r[k] != V3D_JOB71_OK)
			fail("multi: draw %d returned %d", k, r[k]);

	struct v3d_job71_clear c = { .load_rts = 0x09, .z = 1.0f, .zs_load = true, .zs_store = true };
	for (int n = 0; n < 8; n++)
		for (int k = 0; k < 4; k++)
			c.color[n][k] = 0x01010101u * (uint32_t)(n + 1) + (uint32_t)k;
	uint32_t ls, le;
	if (!v3d_job71_end(&j, &c, &ls, &le)) {
		fail("multi: v3d_job71_end refused the job (overflow/bad field: BCL %d/%d RCL %d/%d "
		     "indirect %d/%d)",
		     bcl.overflow, bcl.bad_field, rcl.overflow, rcl.bad_field, ind.overflow, ind.bad_field);
		return 1;
	}
	uint32_t qms, qts_size;
	v3d_job71_alloc_sizes(&j, 4096, &qms, &qts_size);
	struct drm_v3d_submit_cl submit = {
		.bcl_start = bcl.gpu, .bcl_end = bcl.gpu + bcl.used,
		.rcl_start = rcl.gpu, .rcl_end = rcl.gpu + rcl.used,
		.qma = M_TILE, .qms = qms, .qts = M_TSDA,
	};
	static uint8_t vtx[0x100], idx[0x100], tile[0x100], tsda[0x100], rt_bo[8][0x100], zs_bo[0x100];
	char names[8][32];
	struct bo bos[24];
	unsigned n = 0;
	bos[n++] = (struct bo){ "CL_bcl", M_BCL, M_CL, m_bcl };
	bos[n++] = (struct bo){ "CL_rcl", M_RCL, M_CL, m_rcl };
	bos[n++] = (struct bo){ "CL_ind", M_IND, M_CL, m_ind };
	bos[n++] = (struct bo){ "resource_code", M_CODE, sizeof(code), code };
	bos[n++] = (struct bo){ "resource_vtx", M_VTX, sizeof(vtx), vtx };
	bos[n++] = (struct bo){ "resource_idx", M_IDX, sizeof(idx), idx };
	bos[n++] = (struct bo){ "tile_alloc", M_TILE, sizeof(tile), tile };
	bos[n++] = (struct bo){ "TSDA", M_TSDA, sizeof(tsda), tsda };
	for (int k = 0; k < 8; k++) {
		snprintf(names[k], 32, "resource_rt%d", k);
		bos[n++] = (struct bo){ names[k], M_RT(k), sizeof(rt_bo[k]), rt_bo[k] };
	}
	bos[n++] = (struct bo){ "resource_zs", M_ZS, sizeof(zs_bo), zs_bo };
	char *text = decode(bos, n, &submit);
	if (!text) {
		fail("multi: no decoder output");
		return 1;
	}
	/* the census: 3 draws of the full state (stencil, blend, depth offset),
	 * RT 0, RT 3 and Z/S loaded, 7 RTs and Z/S stored, 8 RT configurations
	 * (PART2 for the 4 at 64 bpp and more, PART3 for the one at 128) */
	const int supertiles = (int)(j.tiling.frame_w_supertiles * j.tiling.frame_h_supertiles);
	const struct want w[] = {
		{ "NUMBER_OF_LAYERS", 1 }, { "TILE_BINNING_MODE_CFG", 1 }, { "CLEAR_VCD_CACHE", 2 },
		{ "OCCLUSION_QUERY_COUNTER_ENABLE", 4 }, { "START_TILE_BINNING", 1 }, { "CLIP", 3 },
		{ "CFG_BITS", 3 }, { "DEPTH_OFFSET", 3 }, { "POINT_SIZE", 3 }, { "LINE_WIDTH", 3 },
		{ "CLIPPER_XY", 3 }, { "CLIPPER_Z", 3 }, { "CLIPZ", 3 }, { "VIEWPORT_OFFSET", 3 },
		{ "BLEND_ENABLES", 3 }, { "BLEND_CFG", 3 }, { "COLOR_WMASKS", 3 }, { "BLEND_CCOLOR", 3 },
		{ "STENCIL_CFG", 3 }, { "ZERO_ALL_FLATSHADE_FLAGS", 3 },
		{ "ZERO_ALL_NOPERSPECTIVE_FLAGS", 3 }, { "ZERO_ALL_CENTROID_FLAGS", 3 },
		{ "TRANSFORM_FEEDBACK_SPECS", 3 }, { "SAMPLE_STATE", 3 }, { "VCM_CACHE_SIZE", 3 },
		{ "GL_SHADER", 3 }, { "VERTEX_ARRAY_PRIMS", 1 }, { "INDEX_BUFFER_SETUP", 1 },
		{ "INDEXED_PRIM_LIST", 1 }, { "VERTEX_ARRAY_INSTANCED_PRIMS", 1 }, { "FLUSH", 1 },
		{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
		{ "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1", 8 },
		{ "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART2", 4 },
		{ "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART3", 1 },
		{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 }, { "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
		{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 }, { "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
		{ "TILE_COORDS", 2 }, { "END_LOADS", 3 }, { "LOAD", 3 }, { "STORE", 10 },
		{ "CLEAR_RT", 2 }, { "END_TILE", 3 }, { "GENERIC_TILE_LIST", 1 },
		{ "IMPLICIT_TILE_COORDS", 1 }, { "PRIM_LIST_FORMAT", 1 }, { "SET_INSTANCEID", 1 },
		{ "BRANCH_IMPLICIT_TILE", 1 }, { "RETURN", 1 }, { "SUPERTILE_COORDS", supertiles },
		{ "END_RENDER", 1 },
	};
	/* three draws' uniform streams: binary anywhere in the indirect buffer */
	structure(text, check_census(text, w, sizeof(w) / sizeof(w[0])), "[CL_ind+");
	if (census(text, "@format shadrec_gl_main") != 3)
		fail("multi: not three shader records");

	/* The render-target packets byte by byte: Mesa's decoder (like its
	 * unpacker) reads no field wider than 32 bits right, so PART2's 40
	 * colour bits and PART3's 56 are checked here against v3d_packet.xml's
	 * layout (opcode 121; byte 1: sub-id 2 / 3 / 4 in bits 0..2, the RT
	 * number in 3..5; PART2's bits from byte 4, PART3's from byte 2, little
	 * endian), packet after packet behind the 9-byte common config. */
	uint32_t at = 9, parts = 0;
	for (uint32_t k = 0; k < 8; k++) {
		const bool none = rts[k].none, load = (c.load_rts >> k) & 1u;
		uint32_t words = 1;
		if (rts[k].fmt == V3D_OUTPUT_IMAGE_FORMAT_RGBA32F)
			words = 4;
		else if (rts[k].fmt == V3D_OUTPUT_IMAGE_FORMAT_RGBA16F ||
			 rts[k].fmt == V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8 ||
			 rts[k].fmt == V3D_OUTPUT_IMAGE_FORMAT_R16F)
			words = 2;
		const uint32_t *cw = c.color[k];
		const uint64_t w1 = load ? 0 : cw[1], w2 = load ? 0 : cw[2], w3 = load ? 0 : cw[3];
		const uint64_t want[3] = { 0, w1 | ((w2 & 0xff) << 32), ((w2 & 0xffffff00u) >> 8) | (w3 << 24) };
		const uint32_t nparts = none ? 1 : words == 4 ? 3 : words == 2 ? 2 : 1;
		for (uint32_t part = 0; part < nparts; part++, at += 9, parts++) {
			const uint8_t *p = m_rcl + at;
			if (p[0] != 121 || (p[1] & 7) != 2 + part || ((p[1] >> 3) & 7) != k) {
				fail("multi: RCL +%u: not RENDER_TARGET_PART%u of RT %u (%u, 0x%02x)", at,
				     part + 1, k, p[0], p[1]);
				continue;
			}
			if (part == 0)
				continue; /* PART1's fields are the decoder's, compared above */
			uint64_t v = 0;
			const uint32_t from = part == 1 ? 4 : 2, bytes = part == 1 ? 5 : 7;
			for (uint32_t b = 0; b < bytes; b++)
				v |= (uint64_t)p[from + b] << (8 * b);
			if (v != want[part] || (part == 1 && (p[2] | p[3])))
				fail("multi: RT %u PART%u clear colour bits 0x%llx, expected 0x%llx", k, part + 1,
				     (unsigned long long)v, (unsigned long long)want[part]);
		}
	}
	if (parts != 8 + 4 + 1)
		fail("multi: %u render-target packets, expected 13", parts);
	printf("job71_check: multi %ux%u, 3 draws, 8 RTs, tiles %ux%u, BCL %u RCL %u indirect %u "
	       "bytes, %d supertiles: %s\n",
	       M_W, M_H, j.tiling.tile_w, j.tiling.tile_h, bcl.used, rcl.used, ind.used, supertiles,
	       failures ? "FAIL" : "ok");
	free(text);
	return failures ? 1 : 0;
}

/* ------------------------------------------------------------------------ */
/* refusals: each a copy of the board's triangle job changed in one way */

#define REFUSALS 18

static bool refused(int which, const char **what)
{
	static uint8_t b[0x1000], r[0x1000], i[0x1000];
	struct v3d_cl bcl, rcl, ind;
	uint32_t cap = sizeof(b);
	struct v3d_job71_target t;
	board_target(&TRI_PLACE, &t);
	struct v3d_job71_draw d;
	tri_draw(&TRI_PLACE, &d);
	struct v3d_job71_state st = tri_st;
	struct v3d_fs_prog fs = tri_fs;
	struct v3d_vs_prog vs = tri_vs, cs = tri_cs;
	struct v3d_uniform u[MESA71_TRI_VS_UNIFORM_COUNT];
	memcpy(u, vs_u, sizeof(u));
	vs.uniforms = u;
	d.st = &st;
	d.fs = &fs;
	d.vs = &vs;
	d.cs = &cs;
	struct v3d_job71_clear c;
	board_clear(&c);
	switch (which) {
	case 0: *what = "width 0"; t.width = 0; break;
	case 1: *what = "width 7681"; t.width = 7681; break;
	case 2: *what = "height 7681"; t.height = 7681; break;
	case 3: *what = "9 render targets"; t.rts = 9; break;
	case 4: *what = "a depth format as a colour RT"; t.rt[0].format = V3D_OUTPUT_IMAGE_FORMAT_D24S8; break;
	case 5: *what = "an S8 depth buffer"; t.zs = (struct v3d_job71_zs){ 0x40000, V3D_OUTPUT_IMAGE_FORMAT_S8, 0, 0 }; break;
	case 6: *what = "a Z/S store without a Z/S buffer"; c.zs_store = true; break;
	case 7: *what = "a TMU configuration uniform (7.1 texture state not yet)"; u[3].contents = 40; break;
	case 8: *what = "a viewport Z scale of 0 (NO_GUARDBAND)"; st.vp.scale[2] = 0.0f; break;
	case 9: *what = "separate VPM segments"; vs.separate_segments = true; break;
	case 10: *what = "a CS reading the base instance"; cs.uses_biid = true; break;
	case 11: *what = "3 FS threads"; fs.threads = 3; break;
	case 12: *what = "the VS reads attribute 2, which has no record"; vs.vattr_sizes[2] = 1; break;
	case 13: *what = "a QUNIFORM_UNIFORM with no storage"; u[3].contents = V3D_QUNIFORM_UNIFORM; break;
	case 14: *what = "implicit point varyings on a triangle"; fs.uses_implicit_point_line_varyings = true; break;
	case 15: *what = "lists of 64 bytes"; cap = 64; break;
	case 16: *what = "an 8-bit index type past 32-bit"; d.indexed = true; d.index_type = 3; break;
	case 17: *what = "a depth type past DEPTH_16"; t.depth_type = 3; break;
	default: return false;
	}
	v3d_cl_init(&bcl, b, TRI_PLACE.bcl, cap);
	v3d_cl_init(&rcl, r, TRI_PLACE.rcl, cap);
	v3d_cl_init(&ind, i, TRI_PLACE.indirect, cap);
	struct v3d_job71 j;
	if (!v3d_job71_begin(&j, &t, &bcl, &rcl, &ind))
		return true;
	if (v3d_job71_draw(&j, &d, NULL) != V3D_JOB71_OK)
		return true;
	return !v3d_job71_end(&j, &c, NULL, NULL);
}

static int run_refusals(void)
{
	int n = 0;
	for (int k = 0; k < REFUSALS; k++) {
		const char *what = "";
		if (refused(k, &what))
			n++;
		else
			fail("built a job with %s", what);
	}
	printf("job71_check: refusals %d of %d refused: %s\n", n, REFUSALS, failures ? "FAIL" : "ok");
	return failures ? 1 : 0;
}

int main(int argc, char **argv)
{
	const char *mode = argc > 1 ? argv[1] : "";
	if (strcmp(mode, "clear") == 0)
		return run_board(false);
	if (strcmp(mode, "triangle") == 0)
		return run_board(true);
	if (strcmp(mode, "multi") == 0)
		return run_multi();
	if (strcmp(mode, "refusals") == 0)
		return run_refusals();
	printf("job71_check: FAIL usage: job71_check clear|triangle|multi|refusals\n");
	return 1;
}
