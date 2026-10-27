/* SPDX-License-Identifier: MIT
 *
 * job_check: checks the V3D 4.2 job builder (driver/src/v3d/v3d_job.c), the
 * multi-draw form of the frame builder the GL core's backend uses
 * (docs/online-shaderc/v3d-backend.md, "M6"). Seven parts, every check
 * counted:
 *
 *  1. Identity with the frames that drew on the board. One draw of a triangle
 *     list through the job builder must give the bytes
 *     v3d_frame_build_triangle() gives for the same program, state and clear
 *     (M5, 2026-09-24, every pixel read back), except the one field the job
 *     builder sets on purpose: TILE_RENDERING_MODE_CFG_COMMON's internal depth
 *     type (depth_24, for the Z/S the tile buffer keeps). The BCL and the
 *     indirect buffer must be identical; the RCL may differ only inside the
 *     COMMON packet, and there only in that field. The zero-draw job's BCL
 *     must equal v3d_frame_build_clear()'s (M4).
 *  2. A draw that does not go in leaves nothing: a job whose BCL is too small
 *     for a second draw answers FULL and its lists are byte for byte what
 *     they were; a refused draw (an unknown primitive, a uniform word outside the stage's
 *     storage, an unknown uniform contents, a program reading an attribute
 *     with no record) answers REFUSED the same way; the job still ends.
 *  3. Mesa's CLE decoder (clif_dump) reads a three-draw job that uses every
 *     packet the job builder adds (blend, two-sided stencil, depth offset,
 *     indexed, a scissor, a program with no attribute, flat shading, a loaded
 *     colour) with no error, and the packet census is what the draws ask for.
 *  7. Depth and stencil kept between jobs (M6 step C): a job whose clear
 *     names a Z/S buffer loads it (LOAD_TILE_BUFFER_GENERAL, Z+Stencil, UIF
 *     no XOR, D24S8, the height in UIF blocks, the address) and stores it
 *     (the same, after RT0's store, no clear bit), as Mesa's v3dx_rcl.c
 *     does for a zsbuf; store only emits no load; no buffer emits neither.
 *     Depth and stencil apart (s_address): STENCIL then Z loaded, Z then
 *     STENCIL stored, each at its own buffer, D24S8.
 * 17. A window that is a view into a wider buffer: RT0's LOAD and STORE
 *     carry the view's byte stride (W + 32 pixels a row), decoded; a packed
 *     target carries W * 4.
 * 16. A stencil-only framebuffer's S8 buffer (s_s8, zs_address 0): one LOAD
 *     and one STORE, buffer STENCIL, image format S8, nothing of Z; the two
 *     packets' bytes are the ones Mesa's packer gives for the fields its
 *     load_general / store_general set for a separate stencil
 *     (v3dx_rcl.c:85-108, :131-170). Mesa never emits this job shape (a
 *     stencil-only zsbuf is D24S8 there, v3d_screen.c:508-514), so only the
 *     packets are pinned to Mesa; the shape is unmeasured until the board.
 *  4. The clear quad (src/v3d/state.rs clear_quad_draw, M6 step B): a draw
 *     with Mesa's clear_with_quad state (st_cb_clear.c:194-309) -- a partial
 *     colour mask, depth ALWAYS with the write on, stencil ALWAYS/REPLACE with
 *     a partial write mask on both faces alike, a scissor -- decodes to the
 *     packets that state means: COLOR_WMASKS with the masked-off channels
 *     set, one STENCIL_CFG for both faces, the clip window of the box.
 *  5. The TMU configuration uniforms (M6 textures): a draw whose FS stream
 *     reads QUNIFORM_TMU_CONFIG_P0/P1 for two units gets each unit's texture
 *     and sampler state address with the compiler's bits ORed in
 *     (v3d_uniforms.c write_tmu_p0/p1), read back from the indirect buffer;
 *     a unit the draw did not bind and a misaligned record are REFUSED, with
 *     the lists as they were; the VS stream reads the vertex stage's own
 *     table (sampler bindings are numbered per stage). The size words
 *     (QUNIFORM_TEXTURE_WIDTH / _HEIGHT, v3d_uniforms.c:52-70, 290-298) read the
 *     base level's size the table carries for the binding; a binding the
 *     draw did not give and a size not given (0) are REFUSED.
 *  6. gl_FragCoord.y's transform (QUNIFORM_FB_WPOS_Y_SCALE/OFFSET, our own
 *     contents codes): a window's draw gets (-1.0, the target's height) and
 *     an FBO's (1.0, 0.0), read back from the FS stream; a zeroed (absent)
 *     transform, one made for another height, a scale with the other
 *     scale's offset and a data word that is not 0 are REFUSED with the
 *     lists as they were; the M5 frame builder writes the same words and
 *     refuses the zeroed one. gl_PointCoord.y's pair (QUNIFORM_FB_PNTC_Y_*)
 *     comes from the same transform: (-1.0, 1.0) for a window, (1.0, 0.0)
 *     for an FBO; data 1 and a zeroed transform are REFUSED.
 * 15. Packet skip (v3d_job.h): with the knob off, a 48-draw job whose every
 *     state input changes (pack_seq.h) is byte for byte what origin/main
 *     6ee82953 wrote (a pinned hash); with it on, the RCL and the indirect
 *     buffer are the same, the BCL is shorter, and the state the BCL holds
 *     at each draw (every packet kind's last packet, walked with Mesa's
 *     packet lengths) is the same as with it off. Two identical draws write
 *     the state packets once and the draw packets twice; a new depth range
 *     writes exactly the viewport group again, a new viewport x the clip
 *     window too; a draw refused after its state was packed leaves the last
 *     written inputs as they were; each of the 55 inputs the groups are
 *     packed from, changed alone, is written again; a new job writes every
 *     group.
 *
 * Built against a Mesa 26.2.1 tree with build-v3d (tools/v3d/README.md):
 *
 *   M=<mesa>; B=$M/build-v3d; cc -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H \
 *      -I$M/src -I$M/include -I$M/src/broadcom -I$B/src/broadcom -I$B/src -Idriver/src/v3d \
 *      tools/v3d/job_check.c driver/src/v3d/v3d_job.c driver/src/v3d/v3d_frame.c \
 *      driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
 *      $B/src/broadcom/libbroadcom_v3d.a $B/src/broadcom/cle/libbroadcom_cle.a \
 *      $B/src/broadcom/libv3d_neon.a $B/src/util/libmesa_util.a \
 *      -lexpat -lz -lzstd -lm -lpthread -o job_check
 *   ./job_check            (JOB_CHECK_DUMP=1 prints the decoded lists)
 *
 * Exit status 0 only when every check passed AND the number of checks run is
 * the number planned.
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "common/v3d_device_info.h"
#include "clif/clif_dump.h"
#include "drm-uapi/v3d_drm.h"
#include "util/log.h"

#include "cle/v3d_decoder.h"

/* Mesa's packers, for the reference bytes of part 16 (the S8 stencil's
 * load and store), come through v3d_cl.h (v3d_frame.h), which sets their
 * hooks and compiles them with NDEBUG: it must be the first to include the
 * generated header. */
#include "v3d_frame.h"
#include "v3d_job.h"
#include "pack_seq.h"

static int checks, failures;

static void check(int ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("job_check: FAIL %s\n", what);
	}
}

/* ---- a program, and the state M5 draws with ------------------------------ */

static const struct v3d_uniform fs_u[] = {
	{ V3D_QUNIFORM_CONSTANT, 0x3f800000u },
	{ V3D_QUNIFORM_UNIFORM, 1 },
};
static const struct v3d_uniform vs_u[] = {
	{ V3D_QUNIFORM_VIEWPORT_X_SCALE, 0 }, { V3D_QUNIFORM_VIEWPORT_Y_SCALE, 0 },
	{ V3D_QUNIFORM_VIEWPORT_Z_SCALE, 0 }, { V3D_QUNIFORM_VIEWPORT_Z_OFFSET, 0 },
	{ V3D_QUNIFORM_UNIFORM, 3 },
};
static const struct v3d_uniform cs_u[] = {
	{ V3D_QUNIFORM_VIEWPORT_X_SCALE, 0 }, { V3D_QUNIFORM_VIEWPORT_Y_SCALE, 0 },
	{ V3D_QUNIFORM_UNIFORM, 0 },
};
static const uint32_t vs_words[4] = { 0x3e800000u, 0, 0, 0x3f000000u };
static const uint32_t fs_words[2] = { 0, 0x3f400000u };

static struct v3d_fs_prog fs_prog(void)
{
	return (struct v3d_fs_prog){
		.code = 0x00200000u, .threads = 4, .single_seg = false, .num_inputs = 4,
		.uniforms = fs_u, .num_uniforms = 2,
	};
}

static struct v3d_vs_prog vs_prog(uint32_t code, const struct v3d_uniform *u, uint32_t n)
{
	return (struct v3d_vs_prog){
		.code = code, .threads = 4, .single_seg = true, .vpm_input_size = 0,
		.vpm_output_size = 2, .vcm_cache_size = 4, .vattr_sizes = { 4, 4 },
		.uniforms = u, .num_uniforms = n,
	};
}

#define BO_GPU     0x00100000u
#define BO_SIZE    0x00020000u
#define BCL_OFF    0x00000u
#define RCL_OFF    0x04000u
#define IND_OFF    0x08000u
#define TARGET     0x00400000u
#define TILE_ALLOC 0x00800000u
#define MS_BUF 0x00e00000u /* part 14's 4x colour buffer */
#define RT_IMG 0x00f00000u /* part 20's render image */
#define W 200u
#define H 100u

struct lists {
	uint8_t bo[BO_SIZE];
	struct v3d_cl bcl, rcl, ind;
};

static void lists_init(struct lists *l, uint32_t bcl_size)
{
	memset(l->bo, 0, sizeof(l->bo));
	v3d_cl_init(&l->bcl, l->bo + BCL_OFF, BO_GPU + BCL_OFF, bcl_size);
	v3d_cl_init(&l->rcl, l->bo + RCL_OFF, BO_GPU + RCL_OFF, IND_OFF - RCL_OFF);
	v3d_cl_init(&l->ind, l->bo + IND_OFF, BO_GPU + IND_OFF, BO_SIZE - IND_OFF);
}

static struct v3d_job_state m5_state(void)
{
	struct v3d_job_state st = {
		.front_ccw = true, .line_width = 1.0f, .depth_func = V3D_COMPARE_FUNC_ALWAYS,
		.colormask = 0xf,
	};
	v3d_viewport_gl(0, 0, (float)W, (float)H, 0.0f, 1.0f, true, H, &st.vp);
	return st;
}

/* ---- 1. identity with M5 and M4 ------------------------------------------ */

static void part_identity(void)
{
	static struct lists a, b;
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };

	/* M5 */
	lists_init(&a, RCL_OFF - BCL_OFF);
	struct v3d_triangle_frame f = {
		.clear = { .width = W, .height = H, .clear_rgba8 = 0xff202020u, .target = TARGET,
			   .stride = W * 4, .tile_alloc = TILE_ALLOC },
		.vs_storage = vst, .fs_storage = fst, .front_ccw = true,
		.fs = &fs, .vs = &vs, .cs = &cs, .attrs = attrs, .num_attrs = 2,
		.first_vertex = 0, .vertex_count = 3,
	};
	v3d_viewport_gl(0, 0, (float)W, (float)H, 0.0f, 1.0f, true, H, &f.vp);
	check(v3d_frame_build_triangle(&f, &a.bcl, &a.rcl, &a.ind, NULL), "M5 frame builds");

	/* the same through the job builder */
	lists_init(&b, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	check(v3d_job_begin(&j, &t, &b.bcl, &b.rcl, &b.ind), "job begins");
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3, .first = 0,
	};
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "the M5 draw is recorded");
	struct v3d_job_clear c = { .load_rt0 = false, .rgba8 = 0xff202020u, .z = 0.0f, .stencil = 0 };
	check(v3d_job_end(&j, &c, NULL, NULL), "job ends");

	check(a.bcl.used == b.bcl.used && memcmp(a.bcl.buf, b.bcl.buf, a.bcl.used) == 0,
	      "the job's BCL is the M5 frame's, byte for byte");
	check(a.ind.used == b.ind.used && memcmp(a.ind.buf, b.ind.buf, a.ind.used) == 0,
	      "the job's indirect buffer is the M5 frame's, byte for byte");
	/* the RCL: one length, and every differing byte inside the first packet
	 * (COMMON: an opcode and 8 bytes of fields), where only the internal
	 * depth type (field bits 47..50) may differ: 0 in M5, depth_24 here */
	int same_len = a.rcl.used == b.rcl.used;
	check(same_len, "the RCLs have one length");
	int outside = 0;
	for (uint32_t i = 9; same_len && i < a.rcl.used; i++)
		outside += a.rcl.buf[i] != b.rcl.buf[i];
	check(same_len && outside == 0, "the RCLs differ nowhere past the COMMON packet");
	uint64_t fa = 0, fb = 0;
	for (int k = 0; k < 8; k++) {
		fa |= (uint64_t)a.rcl.buf[1 + k] << (8 * k);
		fb |= (uint64_t)b.rcl.buf[1 + k] << (8 * k);
	}
	uint64_t depth_mask = 0xfull << 47;
	check((fa & ~depth_mask) == (fb & ~depth_mask), "COMMON differs only in the depth type");
	check(((fa & depth_mask) >> 47) == 0 && ((fb & depth_mask) >> 47) == V3D_INTERNAL_TYPE_DEPTH_24,
	      "COMMON's depth type: 0 in M5, depth_24 in the job");

	/* M4: the zero-draw job's BCL is the clear frame's */
	lists_init(&a, RCL_OFF - BCL_OFF);
	struct v3d_clear_frame cf = { .width = W, .height = H, .clear_rgba8 = 0xff00ff00u,
				      .target = TARGET, .stride = W * 4, .tile_alloc = TILE_ALLOC };
	check(v3d_frame_build_clear(&cf, &a.bcl, &a.rcl, &a.ind), "M4 frame builds");
	lists_init(&b, RCL_OFF - BCL_OFF);
	check(v3d_job_begin(&j, &t, &b.bcl, &b.rcl, &b.ind), "the clear job begins");
	struct v3d_job_clear cc = { .rgba8 = 0xff00ff00u, .z = 0.0f };
	check(v3d_job_end(&j, &cc, NULL, NULL), "the clear job ends");
	check(a.bcl.used == b.bcl.used && memcmp(a.bcl.buf, b.bcl.buf, a.bcl.used) == 0,
	      "the zero-draw job's BCL is the M4 frame's, byte for byte");
}

/* ---- 2. a draw that does not go in leaves nothing ------------------------ */

static struct lists rl;
static uint8_t snap_bcl[BO_SIZE], snap_ind[BO_SIZE];
static struct v3d_cl snap_b, snap_i;

static void snapshot(void)
{
	snap_b = rl.bcl;
	snap_i = rl.ind;
	memcpy(snap_bcl, rl.bcl.buf, rl.bcl.used);
	memcpy(snap_ind, rl.ind.buf, rl.ind.used);
}

static int unchanged(void)
{
	return rl.bcl.used == snap_b.used && rl.ind.used == snap_i.used && !rl.bcl.overflow &&
	       !rl.ind.overflow && !rl.bcl.bad_field && !rl.ind.bad_field &&
	       memcmp(snap_bcl, rl.bcl.buf, snap_b.used) == 0 &&
	       memcmp(snap_ind, rl.ind.buf, snap_i.used) == 0;
}

static void part_rollback(void)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	struct v3d_job_clear c = { .rgba8 = 0xff202020u, .z = 1.0f };

	/* the BCL size that holds the prologue, exactly one draw and the FLUSH:
	 * measured from a first job, then given to a second */
	lists_init(&rl, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	v3d_job_begin(&j, &t, &rl.bcl, &rl.rcl, &rl.ind);
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "a draw goes in");
	uint32_t one_draw = rl.bcl.used + 1;

	lists_init(&rl, one_draw);
	v3d_job_begin(&j, &t, &rl.bcl, &rl.rcl, &rl.ind);
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "the first draw fits the small BCL");
	snapshot();
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_FULL, "a second draw is FULL");
	check(unchanged(), "FULL left the lists as they were");

	/* refusals, each a copy of the good draw with one thing wrong */
	struct v3d_job_draw r = d;
	r.prim = V3D_PRIM_TRIANGLE_FAN + 1;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "an unknown primitive: REFUSED, nothing left");
	static const struct v3d_uniform bad_index[] = { { V3D_QUNIFORM_UNIFORM, 2 } };
	struct v3d_fs_prog fs_bad = fs;
	fs_bad.uniforms = bad_index;
	fs_bad.num_uniforms = 1;
	r = d;
	r.fs = &fs_bad;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a uniform word outside the storage: REFUSED, nothing left");
	static const struct v3d_uniform unknown[] = { { 77, 0 } };
	fs_bad.uniforms = unknown;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "an unknown uniform contents: REFUSED, nothing left");
	r = d;
	r.num_attrs = 1;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "an attribute read with no record: REFUSED, nothing left");
	check(j.draws == 1, "the job holds its one draw");
	check(v3d_job_end(&j, &c, NULL, NULL), "the job still ends after FULL and REFUSED");
	check(rl.bcl.used == one_draw, "the ended job used the BCL exactly");
}

/* ---- 3. Mesa's decoder on a three-draw job ------------------------------- */

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

/* The value of `field` in the nth packet named `name` (counting from 0), as
 * the decoder prints it ("    Field: value"); -1 when the packet or the field
 * is absent, which no field checked below can legitimately be. */
static long field(const char *text, const char *name, int nth, const char *fname)
{
	size_t len = strlen(name), flen = strlen(fname);
	int seen = 0;
	for (const char *line = text; *line;) {
		const char *nl = strchr(line, '\n');
		if (strncmp(line, name, len) == 0 && (line[len] == '\n' || line[len] == ' ') &&
		    seen++ == nth) {
			/* the packet's own lines are the indented ones after it */
			for (const char *f = nl ? nl + 1 : ""; *f == ' ';) {
				const char *p = f;
				while (*p == ' ')
					p++;
				if (strncmp(p, fname, flen) == 0 && p[flen] == ':')
					return strtol(p + flen + 1, NULL, 0);
				const char *next = strchr(f, '\n');
				if (!next)
					break;
				f = next + 1;
			}
			return -1;
		}
		if (!nl)
			break;
		line = nl + 1;
	}
	return -1;
}

/* Mesa's log opens MESA_LOG_FILE once per process (util/log.c, at the first
 * message), so every decode writes to the same file: each call returns only
 * what that call added, and the file is removed at exit. */
static char log_path[512];
static long log_read;

static void remove_log(void)
{
	unlink(log_path);
}

static char *decode(const struct lists *l)
{
	if (!log_path[0]) {
		const char *dir = getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp";
		snprintf(log_path, sizeof(log_path), "%s/job_check_%d.log", dir, (int)getpid());
		setenv("MESA_LOG_FILE", log_path, 1);
		setenv("MESA_LOG_LEVEL", "info", 1);
		setenv("MESA_LOG_PREFIX", "", 1);
		atexit(remove_log);
	}

	struct v3d_device_info devinfo = { .ver = 42, .has_accumulators = true,
					   .cle_readahead = 256 };
	struct log_stream *stream = mesa_log_streami();
	struct clif_dump *clif = clif_dump_init(&devinfo, stream, true, false);
	clif_dump_add_bo(clif, "bo", BO_GPU, BO_SIZE, (void *)l->bo);
	/* referenced, not followed: registered so the decoder can name them */
	static uint8_t target[64], tile_alloc[64], code[0x3000], vtx[64], idx[64];
	clif_dump_add_bo(clif, "target", TARGET, sizeof(target), target);
	clif_dump_add_bo(clif, "tile_alloc", TILE_ALLOC, sizeof(tile_alloc), tile_alloc);
	clif_dump_add_bo(clif, "code", 0x00200000u, sizeof(code), code);
	clif_dump_add_bo(clif, "vtx", 0x00300000u, sizeof(vtx), vtx);
	clif_dump_add_bo(clif, "idx", 0x00310000u, sizeof(idx), idx);
	/* part 7's depth/stencil buffer (W x H D24S8, and room for padding) */
	static uint8_t zsbuf[W * H * 4 + 16384];
	clif_dump_add_bo(clif, "zsbuf", 0x00a00000u, sizeof(zsbuf), zsbuf);
	/* and the stencil's own buffer, depth and stencil apart */
	static uint8_t sbuf[W * H * 4 + 16384];
	clif_dump_add_bo(clif, "sbuf", 0x00b00000u, sizeof(sbuf), sbuf);
	/* part 12's render targets 1 and 3 (W x H RGBA8 raster each) */
	static uint8_t rt1buf[W * H * 4], rt3buf[W * H * 4];
	clif_dump_add_bo(clif, "rt1", 0x00c00000u, sizeof(rt1buf), rt1buf);
	clif_dump_add_bo(clif, "rt3", 0x00d00000u, sizeof(rt3buf), rt3buf);
	/* part 14's 4x colour buffer (2W x 2H RGBA8, UIF, and room for padding) */
	static uint8_t msbuf[4 * W * H * 4 + 65536];
	clif_dump_add_bo(clif, "msbuf", MS_BUF, sizeof(msbuf), msbuf);
	/* part 20's render image (W x H RGBA8, UIF, and room for padding) */
	static uint8_t rtimg[W * H * 4 + 65536];
	clif_dump_add_bo(clif, "rtimg", RT_IMG, sizeof(rtimg), rtimg);
	struct drm_v3d_submit_cl submit = {
		.bcl_start = l->bcl.gpu, .bcl_end = l->bcl.gpu + l->bcl.used,
		.rcl_start = l->rcl.gpu, .rcl_end = l->rcl.gpu + l->rcl.used,
	};
	clif_dump(clif, &submit);
	clif_dump_destroy(clif);
	mesa_log_stream_destroy(stream);

	FILE *in = fopen(log_path, "r");
	if (!in)
		return NULL;
	fseek(in, 0, SEEK_END);
	long end = ftell(in);
	if (end <= log_read) {
		fclose(in);
		return NULL; /* this decode wrote nothing */
	}
	fseek(in, log_read, SEEK_SET);
	char *text = malloc(end - log_read + 1);
	size_t got = fread(text, 1, end - log_read, in);
	text[got] = 0;
	fclose(in);
	log_read = end;
	return text;
}

static void part_decode(void)
{
	static struct lists l;
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };

	lists_init(&l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l.bcl, &l.rcl, &l.ind), "the decode job begins");

	/* draw 1: blend, two-sided stencil, depth test and offset, indexed u16 */
	struct v3d_job_state s1 = m5_state();
	s1.depth_enabled = true;
	s1.depth_writemask = true;
	s1.depth_func = V3D_COMPARE_FUNC_LESS;
	s1.offset_tri = true;
	s1.offset_scale = 1.0f;
	s1.offset_units = 4.0f;
	s1.stencil_enabled = true;
	s1.stencil_back_enabled = true;
	s1.stencil_front = (struct v3d_job_stencil_face){ V3D_COMPARE_FUNC_EQUAL, V3D_STENCIL_OP_KEEP,
							  V3D_STENCIL_OP_INCR, V3D_STENCIL_OP_REPLACE,
							  1, 0xff, 0xff };
	s1.stencil_back = (struct v3d_job_stencil_face){ V3D_COMPARE_FUNC_NOTEQUAL, V3D_STENCIL_OP_ZERO,
							 V3D_STENCIL_OP_DECR, V3D_STENCIL_OP_INVERT,
							 2, 0x0f, 0xf0 };
	s1.blend_enabled = true;
	s1.rgb_func = V3D_BLEND_MODE_ADD;
	s1.rgb_src = V3D_BLEND_FACTOR_SRC_ALPHA;
	s1.rgb_dst = V3D_BLEND_FACTOR_INV_SRC_ALPHA;
	s1.alpha_func = V3D_BLEND_MODE_ADD;
	s1.alpha_src = V3D_BLEND_FACTOR_ONE;
	s1.alpha_dst = V3D_BLEND_FACTOR_ZERO;
	s1.colormask = 0x7;
	struct v3d_job_draw d1 = {
		.st = &s1, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 6,
		.indexed = true, .index_type = V3D_INDEX_TYPE_16, .index_buffer = 0x00310000u,
		.index_buffer_bytes = 64, .index_offset = 4,
	};
	check(v3d_job_draw(&j, &d1, NULL) == V3D_JOB_OK, "draw 1 (blend, stencil, offset, indexed)");

	/* draw 2: a strip through a scissor, culling the back */
	struct v3d_job_state s2 = m5_state();
	s2.scissor = true;
	s2.scissor_minx = 10;
	s2.scissor_miny = 20;
	s2.scissor_maxx = 50;
	s2.scissor_maxy = 60;
	s2.cull_back = true;
	struct v3d_job_draw d2 = d1;
	d2.st = &s2;
	d2.indexed = false;
	d2.prim = V3D_PRIM_TRIANGLE_STRIP;
	d2.count = 4;
	d2.first = 2;
	check(v3d_job_draw(&j, &d2, NULL) == V3D_JOB_OK, "draw 2 (scissored strip)");

	/* draw 3: a program with no attribute and a flat varying, a line loop */
	struct v3d_fs_prog fs3 = fs;
	fs3.flat_shade_flags = 0x5;
	struct v3d_vs_prog vs3 = vs, cs3 = cs;
	memset(vs3.vattr_sizes, 0, sizeof(vs3.vattr_sizes));
	memset(cs3.vattr_sizes, 0, sizeof(cs3.vattr_sizes));
	struct v3d_job_state s3 = m5_state();
	struct v3d_job_draw d3 = {
		.st = &s3, .fs = &fs3, .vs = &vs3, .cs = &cs3, .vs_storage = vst, .fs_storage = fst,
		.num_attrs = 0, .prim = V3D_PRIM_LINE_LOOP, .count = 3,
	};
	check(v3d_job_draw(&j, &d3, NULL) == V3D_JOB_OK, "draw 3 (no attribute, flat, line loop)");

	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f, .stencil = 0 };
	check(v3d_job_end(&j, &c, NULL, NULL), "the decode job ends");

	char *text = decode(&l);
	check(text != NULL, "the decoder wrote its output");
	if (!text)
		return;
	if (getenv("JOB_CHECK_DUMP"))
		fputs(text, stdout);

	struct v3d_tiling tl;
	v3d_frame_tiling(W, H, &tl);
	int supertiles = (int)(((H - 1) / (tl.tile_h * tl.supertile_h) + 1) *
			       ((W - 1) / (tl.tile_w * tl.supertile_w) + 1));
	/* The names the decoder prints: the XML shortname upper-cased when
	 * there is one (v3d_decoder.c), else the packet name. */
	const struct { const char *name; int n; } want[] = {
		/* the prologue and the three draws' state */
		{ "TILE_BINNING_MODE_CFG", 1 },
		{ "CLEAR_VCD_CACHE", 2 },
		{ "OCCLUSION_QUERY_COUNTER_ENABLE", 4 }, /* start_binning + 3 x emit_state */
		{ "START_TILE_BINNING", 1 },
		{ "CLIP", 3 },
		{ "CFG_BITS", 3 },
		{ "DEPTH_OFFSET", 1 },
		{ "POINT_SIZE", 3 },
		{ "LINE_WIDTH", 3 },
		{ "CLIPPER_XY", 3 },
		{ "CLIPPER_Z", 3 },
		{ "CLIPZ", 3 },
		{ "VIEWPORT_OFFSET", 3 },
		{ "BLEND_ENABLES", 3 },
		{ "BLEND_CFG", 1 },
		{ "COLOR_WMASKS", 3 },
		{ "BLEND_CCOLOR", 3 },
		{ "STENCIL_CFG", 2 },
		{ "FLATSHADE_FLAGS", 1 },
		{ "ZERO_ALL_FLATSHADE_FLAGS", 2 },
		{ "ZERO_ALL_NOPERSPECTIVE_FLAGS", 3 },
		{ "ZERO_ALL_CENTROID_FLAGS", 3 },
		{ "TRANSFORM_FEEDBACK_SPECS", 3 },
		{ "SAMPLE_STATE", 3 },
		/* the shader state and the primitives */
		{ "VCM_CACHE_SIZE", 3 },
		{ "GL_SHADER", 3 },
		{ "INDEX_BUFFER_SETUP", 1 },
		{ "INDEXED_PRIM_LIST", 1 },
		{ "VERTEX_ARRAY_PRIMS", 2 },
		{ "FLUSH", 1 },
		/* the RCL and the tile list, with the load */
		{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
		{ "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 1 },
		{ "TILE_RENDERING_MODE_CFG_COLOR", 1 },
		{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 },
		{ "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
		{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 },
		{ "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
		{ "TILE_COORDS", 2 },
		{ "LOAD", 1 },
		{ "END_LOADS", 3 },
		{ "STORE", 3 },
		{ "CLEAR", 2 },
		{ "END_TILE", 3 },
		{ "GENERIC_TILE_LIST", 1 },
		{ "IMPLICIT_TILE_COORDS", 1 },
		{ "PRIM_LIST_FORMAT", 1 },
		{ "SET_INSTANCEID", 1 },
		{ "BRANCH_IMPLICIT_TILE", 1 },
		{ "RETURN", 1 },
		{ "SUPERTILE_COORDS", supertiles },
		{ "END_RENDER", 1 },
	};
	int total = 0;
	for (unsigned i = 0; i < sizeof(want) / sizeof(want[0]); i++) {
		int got = census(text, want[i].name);
		total += got;
		char what[128];
		snprintf(what, sizeof(what), "census %s: decoded %d, expected %d", want[i].name, got,
			 want[i].n);
		check(got == want[i].n, what);
	}
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "the decoder reported no error");
	check(total > 0, "the census found packets at all");

	/* The values the draws asked for, as the decoder reads them back. The
	 * packers are covered field by field by cl_golden; these check what the
	 * job builder DECIDES: which face a stencil packet configures, what
	 * culling and the scissor become, which primitive and indices go out. */
	const struct { const char *name; int nth; const char *field; long want; } fields[] = {
		/* draw 1: two-sided stencil, the front packet not claiming the back */
		{ "STENCIL_CFG", 0, "Front Config", 1 },
		{ "STENCIL_CFG", 0, "Back Config", 0 },
		{ "STENCIL_CFG", 0, "Stencil Test Function", V3D_COMPARE_FUNC_EQUAL },
		{ "STENCIL_CFG", 0, "Depth Test Fail Op", V3D_STENCIL_OP_INCR },
		{ "STENCIL_CFG", 0, "Stencil Pass Op", V3D_STENCIL_OP_REPLACE },
		{ "STENCIL_CFG", 0, "Stencil Ref Value", 1 },
		{ "STENCIL_CFG", 1, "Front Config", 0 },
		{ "STENCIL_CFG", 1, "Back Config", 1 },
		{ "STENCIL_CFG", 1, "Stencil Write Mask", 0xf0 },
		{ "STENCIL_CFG", 1, "Stencil Test Mask", 0x0f },
		{ "CFG_BITS", 0, "Depth-Test Function", V3D_COMPARE_FUNC_LESS },
		{ "CFG_BITS", 0, "Z updates enable", 1 },
		{ "CFG_BITS", 0, "Enable Depth Offset", 1 },
		{ "CFG_BITS", 0, "Stencil enable", 1 },
		{ "CFG_BITS", 0, "Blend enable", 1 },
		{ "BLEND_ENABLES", 0, "Mask", 1 },
		{ "BLEND_CFG", 0, "Color blend src factor", V3D_BLEND_FACTOR_SRC_ALPHA },
		{ "BLEND_CFG", 0, "Color blend dst factor", V3D_BLEND_FACTOR_INV_SRC_ALPHA },
		{ "BLEND_CFG", 0, "Alpha blend src factor", V3D_BLEND_FACTOR_ONE },
		{ "BLEND_CFG", 0, "Render Target Mask", 0xf },
		/* colormask 0x7 (alpha off): the alpha bit of each of the 4 RTs */
		{ "COLOR_WMASKS", 0, "Mask", 0x8888 },
		{ "INDEX_BUFFER_SETUP", 0, "Size", 64 },
		{ "INDEXED_PRIM_LIST", 0, "mode", 4 },
		{ "INDEXED_PRIM_LIST", 0, "Index type", V3D_INDEX_TYPE_16 },
		{ "INDEXED_PRIM_LIST", 0, "Length", 6 },
		{ "INDEXED_PRIM_LIST", 0, "Index Offset", 4 },
		/* draw 2: the scissor inside the viewport, the back culled */
		{ "CLIP", 1, "Clip Window Left Pixel Coordinate", 10 },
		{ "CLIP", 1, "Clip Window Bottom Pixel Coordinate", 20 },
		{ "CLIP", 1, "Clip Window Width in pixels", 40 },
		{ "CLIP", 1, "Clip Window Height in pixels", 40 },
		{ "CFG_BITS", 1, "Enable Forward Facing Primitive", 1 },
		{ "CFG_BITS", 1, "Enable Reverse Facing Primitive", 0 },
		{ "CFG_BITS", 1, "Depth-Test Function", V3D_COMPARE_FUNC_ALWAYS },
		{ "BLEND_ENABLES", 1, "Mask", 0 }, /* draw 2 does not blend */
		{ "VERTEX_ARRAY_PRIMS", 0, "mode", V3D_PRIM_TRIANGLE_STRIP },
		{ "VERTEX_ARRAY_PRIMS", 0, "Length", 4 },
		{ "VERTEX_ARRAY_PRIMS", 0, "Index of First Vertex", 2 },
		/* draw 3: the dummy attribute record, flat shading, a line loop */
		{ "GL_SHADER", 2, "number of attribute arrays", 1 },
		{ "FLATSHADE_FLAGS", 0, "Flat Shade Flags for varyings V0*24", 5 },
		{ "FLATSHADE_FLAGS", 0, "Action for Flat Shade Flags of higher numbered varyings",
		  V3D_VARYING_FLAGS_ACTION_ZEROED },
		{ "BLEND_ENABLES", 2, "Mask", 0 },
		{ "VERTEX_ARRAY_PRIMS", 1, "mode", V3D_PRIM_LINE_LOOP },
		/* the RCL: the colour loaded, the Z/S clear values, no early Z */
		{ "LOAD", 0, "Buffer to Load", V3D_LOAD_BUFFER_RT0 },
		{ "LOAD", 0, "Memory Format", V3D_MEMORY_FORMAT_RASTER },
		{ "LOAD", 0, "Input Image Format", V3D_OUTPUT_IMAGE_FORMAT_RGBA8 },
		{ "TILE_RENDERING_MODE_CFG_COMMON", 0, "Early-Z disable", 1 },
		{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 0, "Stencil Clear Value", 0 },
	};
	for (unsigned i = 0; i < sizeof(fields) / sizeof(fields[0]); i++) {
		long got = field(text, fields[i].name, fields[i].nth, fields[i].field);
		char what[200];
		snprintf(what, sizeof(what), "%s #%d %s: decoded %ld, expected %ld", fields[i].name,
			 fields[i].nth, fields[i].field, got, fields[i].want);
		check(got == fields[i].want, what);
	}
	printf("job_check: three-draw job, BCL %u RCL %u indirect %u bytes, %d packets decoded\n",
	       l.bcl.used, l.rcl.used, l.ind.used, total);
	free(text);
}

/* ---- 4. the clear quad ---------------------------------------------------- */

static void part_clear_quad(void)
{
	static struct lists l;
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 16, 4 }, { 0x00300000u, 16, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };

	lists_init(&l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l.bcl, &l.rcl, &l.ind), "the clear-quad job begins");

	/* what state.rs clear_quad_draw gives a scissored clear of R and A,
	 * depth 0.25 and stencil 0x5a under the write mask 0x0f */
	struct v3d_job_state q = m5_state();
	q.colormask = 0x9;
	q.depth_enabled = true;
	q.depth_writemask = true;
	q.depth_func = V3D_COMPARE_FUNC_ALWAYS;
	q.stencil_enabled = true;
	q.stencil_back_enabled = false; /* both faces alike: one packet */
	q.stencil_front = (struct v3d_job_stencil_face){ V3D_COMPARE_FUNC_ALWAYS, V3D_STENCIL_OP_REPLACE,
							 V3D_STENCIL_OP_REPLACE, V3D_STENCIL_OP_REPLACE,
							 0x5a, 0xff, 0x0f };
	q.scissor = true;
	q.scissor_minx = 8;
	q.scissor_miny = 4;
	q.scissor_maxx = 24;
	q.scissor_maxy = 36;
	struct v3d_job_draw d = {
		.st = &q, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 6,
	};
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "the clear quad goes in");
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f, .stencil = 0 };
	check(v3d_job_end(&j, &c, NULL, NULL), "the clear-quad job ends");

	char *text = decode(&l);
	check(text != NULL, "the decoder wrote its output");
	if (!text)
		return;
	if (getenv("JOB_CHECK_DUMP"))
		fputs(text, stdout);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "the decoder reported no error");
	check(census(text, "STENCIL_CFG") == 1, "one STENCIL_CFG for both faces");
	const struct { const char *name; const char *field; long want; } fields[] = {
		/* translate_colormask: a SET bit disables the channel, per RT */
		{ "COLOR_WMASKS", "Mask", 0x6666 },
		{ "BLEND_ENABLES", "Mask", 0 },
		{ "CFG_BITS", "Depth-Test Function", V3D_COMPARE_FUNC_ALWAYS },
		{ "CFG_BITS", "Z updates enable", 1 },
		{ "CFG_BITS", "Stencil enable", 1 },
		{ "CFG_BITS", "Blend enable", 0 },
		{ "STENCIL_CFG", "Front Config", 1 },
		{ "STENCIL_CFG", "Back Config", 1 },
		{ "STENCIL_CFG", "Stencil Test Function", V3D_COMPARE_FUNC_ALWAYS },
		{ "STENCIL_CFG", "Stencil Test Fail Op", V3D_STENCIL_OP_REPLACE },
		{ "STENCIL_CFG", "Depth Test Fail Op", V3D_STENCIL_OP_REPLACE },
		{ "STENCIL_CFG", "Stencil Pass Op", V3D_STENCIL_OP_REPLACE },
		{ "STENCIL_CFG", "Stencil Ref Value", 0x5a },
		{ "STENCIL_CFG", "Stencil Test Mask", 0xff },
		{ "STENCIL_CFG", "Stencil Write Mask", 0x0f },
		{ "CLIP", "Clip Window Left Pixel Coordinate", 8 },
		{ "CLIP", "Clip Window Bottom Pixel Coordinate", 4 },
		{ "CLIP", "Clip Window Width in pixels", 16 },
		{ "CLIP", "Clip Window Height in pixels", 32 },
	};
	for (unsigned i = 0; i < sizeof(fields) / sizeof(fields[0]); i++) {
		long got = field(text, fields[i].name, 0, fields[i].field);
		char what[200];
		snprintf(what, sizeof(what), "clear quad: %s %s: decoded %ld, expected %ld", fields[i].name,
			 fields[i].field, got, fields[i].want);
		check(got == fields[i].want, what);
	}
	free(text);
}

/* ---- 5. the TMU configuration uniforms ---------------------------------- */

static uint32_t ind_word(uint32_t gpu)
{
	uint32_t v;
	memcpy(&v, rl.bo + (gpu - BO_GPU), 4);
	return v;
}

static void part_tmu(void)
{
	static const struct v3d_uniform fs_tex_u[] = {
		{ V3D_QUNIFORM_CONSTANT, 0x3f800000u },
		{ V3D_QUNIFORM_TMU_CONFIG_P0, (0u << 24) | 0x5 },
		{ V3D_QUNIFORM_TMU_CONFIG_P1, (0u << 24) | 0x3 },
		{ V3D_QUNIFORM_TMU_CONFIG_P0, (1u << 24) | 0x1 },
		{ V3D_QUNIFORM_TMU_CONFIG_P1, (1u << 24) | 0x0 },
	};
	struct v3d_fs_prog fs = fs_prog();
	fs.uniforms = fs_tex_u;
	fs.num_uniforms = 5;
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	/* unit 1 before unit 0: looked up by unit, not by position */
	const struct v3d_job_tmu tmu[2] = { { 1, 0x00500040u, 0x00500060u },
					    { 0, 0x00500000u, 0x00500020u } };
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
		.tmu = tmu, .num_tmu = 2,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };

	lists_init(&rl, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &rl.bcl, &rl.rcl, &rl.ind), "the textured job begins");
	struct v3d_job_draw_layout lo;
	check(v3d_job_draw(&j, &d, &lo) == V3D_JOB_OK, "a textured draw goes in");
	const uint32_t want[5] = { 0x3f800000u, 0x00500000u | 0x5, 0x00500020u | 0x3,
				   0x00500040u | 0x1, 0x00500060u };
	for (int i = 0; i < 5; i++) {
		char what[120];
		uint32_t got = ind_word(lo.fs_uniforms + 4 * (uint32_t)i);
		snprintf(what, sizeof(what), "FS uniform %d: 0x%08x, want 0x%08x", i, got, want[i]);
		check(got == want[i], what);
	}

	snapshot();
	struct v3d_job_draw r = d;
	r.tmu = &tmu[1]; /* unit 0 only */
	r.num_tmu = 1;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a unit the draw did not bind: REFUSED, nothing left");
	const struct v3d_job_tmu misaligned[2] = { { 1, 0x00500048u, 0x00500060u },
						   { 0, 0x00500000u, 0x00500020u } };
	r.tmu = misaligned;
	r.num_tmu = 2;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a record not 32-byte aligned: REFUSED, nothing left");
	/* the VS and CS streams read the vertex stage's own table */
	static const struct v3d_uniform vs_tex_u[] = { { V3D_QUNIFORM_TMU_CONFIG_P0, (0u << 24) | 0x2 } };
	struct v3d_vs_prog vst_prog = vs;
	vst_prog.uniforms = vs_tex_u;
	vst_prog.num_uniforms = 1;
	r = d;
	r.vs = &vst_prog;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a VS binding with only an FS table: REFUSED, nothing left");
	const struct v3d_job_tmu vs_table[1] = { { 0, 0x00600000u, 0x00600020u } };
	r.vs_tmu = vs_table;
	r.num_vs_tmu = 1;
	check(v3d_job_draw(&j, &r, &lo) == V3D_JOB_OK, "a VS binding with its own table goes in");
	check(ind_word(lo.vs_uniforms) == (0x00600000u | 0x2),
	      "the VS stream's P0 is the vertex table's binding 0, not the fragment table's");
	/* the size words (v3d_uniforms.c:52-70, 290-298, QUNIFORM_TEXTURE_WIDTH /
	 * _HEIGHT; data = the binding, not shifted): the base level's size
	 * the table carries, looked up by binding; a binding the draw did not
	 * give and a size of 0 (not given) are REFUSED */
	static const struct v3d_uniform fs_size_u[] = {
		{ V3D_QUNIFORM_TEXTURE_WIDTH, 1 }, { V3D_QUNIFORM_TEXTURE_HEIGHT, 1 },
		{ V3D_QUNIFORM_TEXTURE_WIDTH, 0 }, { V3D_QUNIFORM_TEXTURE_HEIGHT, 0 },
	};
	struct v3d_fs_prog fs_size = fs;
	fs_size.uniforms = fs_size_u;
	fs_size.num_uniforms = 4;
	const struct v3d_job_tmu sized[2] = { { 1, 0x00500040u, 0x00500060u, 15, 7 },
					      { 0, 0x00500000u, 0x00500020u, 64, 32 } };
	r = d;
	r.fs = &fs_size;
	r.tmu = sized;
	check(v3d_job_draw(&j, &r, &lo) == V3D_JOB_OK, "a draw reading texture sizes goes in");
	const uint32_t want_size[4] = { 15, 7, 64, 32 };
	for (int i = 0; i < 4; i++) {
		char what[120];
		uint32_t got = ind_word(lo.fs_uniforms + 4 * (uint32_t)i);
		snprintf(what, sizeof(what), "FS size word %d: %u, want %u", i, got, want_size[i]);
		check(got == want_size[i], what);
	}
	snapshot();
	r.tmu = &sized[1]; /* binding 0 only */
	r.num_tmu = 1;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a size word for a binding the draw did not give: REFUSED, nothing left");
	r.tmu = tmu; /* the same bindings, no size given */
	r.num_tmu = 2;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
	      "a size word for a binding given without its size: REFUSED, nothing left");
	struct v3d_job_clear c = { .rgba8 = 0xff202020u, .z = 1.0f };
	check(v3d_job_end(&j, &c, NULL, NULL), "the textured job ends");
}

/* ---- 6. gl_FragCoord.y's transform --------------------------------------- */

static uint32_t word_at(const struct lists *l, uint32_t gpu)
{
	uint32_t v;
	memcpy(&v, l->bo + (gpu - BO_GPU), 4);
	return v;
}

static uint32_t fbits(float f)
{
	uint32_t v;
	memcpy(&v, &f, 4);
	return v;
}

static void part_wpos(void)
{
	static const struct v3d_uniform fs_wpos_u[] = {
		{ V3D_QUNIFORM_FB_WPOS_Y_SCALE, 0 },
		{ V3D_QUNIFORM_FB_WPOS_Y_OFFSET, 0 },
	};
	struct v3d_fs_prog fs = fs_prog();
	fs.uniforms = fs_wpos_u;
	fs.num_uniforms = 2;
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };

	struct v3d_wpos w;
	v3d_wpos_gl(true, H, &w);
	check(w.y_scale == -1.0f && w.y_offset == (float)H, "v3d_wpos_gl: a window is (-1, height)");
	v3d_wpos_gl(false, H, &w);
	check(w.y_scale == 1.0f && w.y_offset == 0.0f, "v3d_wpos_gl: an FBO is (1, 0)");

	/* the job builder: the words read back from the FS stream */
	struct v3d_job_state st = m5_state();
	v3d_wpos_gl(true, H, &st.wpos);
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	lists_init(&rl, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &rl.bcl, &rl.rcl, &rl.ind), "the gl_FragCoord job begins");
	struct v3d_job_draw_layout lo;
	check(v3d_job_draw(&j, &d, &lo) == V3D_JOB_OK, "a window draw reading the transform goes in");
	check(ind_word(lo.fs_uniforms) == fbits(-1.0f) && ind_word(lo.fs_uniforms + 4) == fbits((float)H),
	      "a window's FS stream holds (-1.0, height)");
	struct v3d_job_state fbo = st;
	v3d_wpos_gl(false, H, &fbo.wpos);
	struct v3d_job_draw e = d;
	e.st = &fbo;
	check(v3d_job_draw(&j, &e, &lo) == V3D_JOB_OK, "an FBO draw reading the transform goes in");
	check(ind_word(lo.fs_uniforms) == fbits(1.0f) && ind_word(lo.fs_uniforms + 4) == fbits(0.0f),
	      "an FBO's FS stream holds (1.0, 0.0)");

	/* gl_PointCoord.y's pair from the same transform: (-1, 1) for a
	 * window, (1, 0) for an FBO (STATE_FB_PNTC_Y_TRANSFORM,
	 * prog_statevars.c:754-763, with ES's upper-left sprite origin) */
	static const struct v3d_uniform fs_pntc_u[] = {
		{ V3D_QUNIFORM_FB_PNTC_Y_SCALE, 0 },
		{ V3D_QUNIFORM_FB_PNTC_Y_OFFSET, 0 },
	};
	struct v3d_fs_prog pf = fs;
	pf.uniforms = fs_pntc_u;
	struct v3d_job_draw p = d;
	p.fs = &pf;
	check(v3d_job_draw(&j, &p, &lo) == V3D_JOB_OK, "a window draw reading the point-coord pair goes in");
	check(ind_word(lo.fs_uniforms) == fbits(-1.0f) && ind_word(lo.fs_uniforms + 4) == fbits(1.0f),
	      "a window's FS stream holds the point-coord pair (-1.0, 1.0)");
	p.st = &fbo;
	check(v3d_job_draw(&j, &p, &lo) == V3D_JOB_OK, "an FBO draw reading the point-coord pair goes in");
	check(ind_word(lo.fs_uniforms) == fbits(1.0f) && ind_word(lo.fs_uniforms + 4) == fbits(0.0f),
	      "an FBO's FS stream holds the point-coord pair (1.0, 0.0)");
	{
		struct v3d_uniform u[2] = { { V3D_QUNIFORM_FB_PNTC_Y_SCALE, 0 },
					    { V3D_QUNIFORM_FB_PNTC_Y_OFFSET, 1 } };
		struct v3d_fs_prog bf = fs;
		bf.uniforms = u;
		struct v3d_job_draw r = d;
		r.fs = &bf;
		snapshot();
		check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
		      "a point-coord word with data 1: REFUSED, nothing left");
		struct v3d_job_state zs = st;
		zs.wpos = (struct v3d_wpos){ 0.0f, 0.0f };
		struct v3d_uniform ok_u[2] = { { V3D_QUNIFORM_FB_PNTC_Y_SCALE, 0 },
					       { V3D_QUNIFORM_FB_PNTC_Y_OFFSET, 0 } };
		bf.uniforms = ok_u;
		r.st = &zs;
		snapshot();
		check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(),
		      "a point-coord pair from a zeroed (absent) transform: REFUSED, nothing left");
	}

	/* refused, and nothing left: absent, made for another height, a scale
	 * with the other one's offset, a data word that is not 0 */
	static const struct {
		float scale, offset;
		uint32_t data;
		const char *what;
	} bad[] = {
		{ 0.0f, 0.0f, 0, "a zeroed (absent) transform: REFUSED, nothing left" },
		{ -1.0f, (float)(H + 1), 0, "a transform for another height: REFUSED, nothing left" },
		{ 1.0f, (float)H, 0, "scale 1 with the window's offset: REFUSED, nothing left" },
		{ -1.0f, (float)H, 1, "data 1: REFUSED, nothing left" },
	};
	for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
		struct v3d_uniform u[2] = { { V3D_QUNIFORM_FB_WPOS_Y_SCALE, bad[i].data },
					    { V3D_QUNIFORM_FB_WPOS_Y_OFFSET, 0 } };
		struct v3d_fs_prog bf = fs;
		bf.uniforms = u;
		struct v3d_job_state bs = st;
		bs.wpos = (struct v3d_wpos){ bad[i].scale, bad[i].offset };
		struct v3d_job_draw r = d;
		r.st = &bs;
		r.fs = &bf;
		snapshot();
		check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && unchanged(), bad[i].what);
	}
	struct v3d_job_clear c = { .rgba8 = 0xff202020u, .z = 1.0f };
	check(v3d_job_end(&j, &c, NULL, NULL), "the gl_FragCoord job ends");

	/* the M5 frame builder writes the same words, and refuses a zeroed one */
	static struct lists fl;
	struct v3d_triangle_frame f = {
		.clear = { .width = W, .height = H, .clear_rgba8 = 0xff202020u, .target = TARGET,
			   .stride = W * 4, .tile_alloc = TILE_ALLOC },
		.vs_storage = vst, .fs_storage = fst, .front_ccw = true,
		.fs = &fs, .vs = &vs, .cs = &cs, .attrs = attrs, .num_attrs = 2,
		.first_vertex = 0, .vertex_count = 3,
	};
	v3d_viewport_gl(0, 0, (float)W, (float)H, 0.0f, 1.0f, true, H, &f.vp);
	lists_init(&fl, RCL_OFF - BCL_OFF);
	check(!v3d_frame_build_triangle(&f, &fl.bcl, &fl.rcl, &fl.ind, NULL) && fl.ind.bad_field,
	      "the M5 frame with a zeroed transform: refused (bad_field)");
	v3d_wpos_gl(true, H, &f.wpos);
	lists_init(&fl, RCL_OFF - BCL_OFF);
	struct v3d_triangle_layout tl;
	check(v3d_frame_build_triangle(&f, &fl.bcl, &fl.rcl, &fl.ind, &tl), "the M5 frame with a window's transform builds");
	check(word_at(&fl, tl.fs_uniforms) == fbits(-1.0f) && word_at(&fl, tl.fs_uniforms + 4) == fbits((float)H),
	      "the M5 frame's FS stream holds (-1.0, height)");
}

/* ---- 7. depth and stencil kept between jobs (M6 step C) ------------------ */

/* part 11 sets it: the FS reads the implicit point varyings (gl_PointCoord) */
static bool one_job_implicit;

/* One M5 draw in a job whose clear carries `zs`, its VS writing the point
 * size when `psiz`, decoded: the text. */
static char *one_job_on(struct lists *l, const struct v3d_job_clear *zs, bool psiz, uint32_t prim,
			bool rgb565, bool writes_z, bool disable_ez)
{
	struct v3d_fs_prog fs = fs_prog();
	fs.uses_implicit_point_line_varyings = one_job_implicit;
	/* the compiler's report: a discarding FS writes Z itself with early Z
	 * off (1, 1); one reading gl_FragCoord.z writes Z with early Z kept
	 * (1, 0) (Mesa nir_to_vir.c:2081-2098, vir.c:1011-1013) */
	fs.writes_z = writes_z;
	fs.disable_ez = disable_ez;
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = prim, .count = 3,
		.vs_writes_psiz = psiz,
	};
	struct v3d_job_target t = { W, H, TARGET, rgb565 ? W * 2 : W * 4, TILE_ALLOC, rgb565 };
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) &&
		      v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK && v3d_job_end(&j, zs, NULL, NULL),
	      "a Z/S job builds");
	return decode(l);
}

static char *one_job(struct lists *l, const struct v3d_job_clear *zs, bool psiz, uint32_t prim)
{
	return one_job_on(l, zs, psiz, prim, false, false, false);
}

static char *zs_job(struct lists *l, const struct v3d_job_clear *zs)
{
	return one_job(l, zs, false, V3D_PRIM_TRIANGLES);
}

/* Whether the nth `name` packet's `fname` line reads `needle`: an address
 * is decoded as "[bo+offset]" and a comment with the value, not a number. */
static bool field_has(const char *text, const char *name, int nth, const char *fname,
		      const char *needle)
{
	size_t len = strlen(name), flen = strlen(fname);
	int seen = 0;
	for (const char *line = text; *line;) {
		const char *nl = strchr(line, '\n');
		if (strncmp(line, name, len) == 0 && (line[len] == '\n' || line[len] == ' ') &&
		    seen++ == nth) {
			for (const char *f = nl ? nl + 1 : ""; *f == ' ';) {
				const char *p = f;
				while (*p == ' ')
					p++;
				const char *end = strchr(p, '\n');
				if (strncmp(p, fname, flen) == 0 && p[flen] == ':') {
					const char *hit = strstr(p, needle);
					return hit && (!end || hit < end);
				}
				if (!end)
					break;
				f = end + 1;
			}
			return false;
		}
		if (!nl)
			break;
		line = nl + 1;
	}
	return false;
}

/* The nth STORE whose buffer is `buffer`, or -1. */
static int store_of(const char *text, long buffer)
{
	for (int n = 0; n < 8; n++) {
		long b = field(text, "STORE", n, "Buffer to Store");
		if (b < 0)
			return -1;
		if (b == buffer)
			return n;
	}
	return -1;
}

/* The nth LOAD whose buffer is `buffer`, or -1. */
static int load_of(const char *text, long buffer)
{
	for (int n = 0; n < 8; n++) {
		long b = field(text, "LOAD", n, "Buffer to Load");
		if (b < 0)
			return -1;
		if (b == buffer)
			return n;
	}
	return -1;
}

static void part_zs(void)
{
	static struct lists l;
	/* loaded and stored: the buffer at 0xa00000, UIF (no XOR), 13 UIF blocks */
	struct v3d_job_clear c = { .load_rt0 = false, .rgba8 = 0xff202020u, .z = 1.0f,
				   .zs_address = 0x00a00000u,
				   .zs_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR,
				   .zs_height_in_ub = 13, .zs_load = true, .zs_store = true };
	char *text = zs_job(&l, &c);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "Z/S job: the decoder reported no error");
	check(census(text, "LOAD") == 1, "Z/S job: one LOAD, the Z/S buffer (the colour is cleared)");
	check(census(text, "STORE") == 4, "Z/S job: four STOREs (two dummy tiles, RT0, Z/S)");
	const struct { const char *field; long want; } ld[] = {
		{ "Buffer to Load", V3D_LOAD_BUFFER_ZSTENCIL },
		{ "Memory Format", V3D_MEMORY_FORMAT_UIF_NO_XOR },
		{ "Input Image Format", V3D_OUTPUT_IMAGE_FORMAT_D24S8 },
		{ "Height in UB or Stride", 13 },
	};
	check(field_has(text, "LOAD", 0, "Address", "[zsbuf+0x00000000]"),
	      "Z/S LOAD Address: the buffer's first byte (0x00a00000)");
	for (unsigned i = 0; i < sizeof(ld) / sizeof(ld[0]); i++) {
		long got = field(text, "LOAD", 0, ld[i].field);
		char what[160];
		snprintf(what, sizeof(what), "Z/S LOAD %s: decoded %ld, expected %ld", ld[i].field, got,
			 ld[i].want);
		check(got == ld[i].want, what);
	}
	int zs = store_of(text, V3D_STORE_BUFFER_ZSTENCIL);
	check(zs >= 0, "Z/S job: a STORE of buffer 11 (Z+Stencil)");
	const struct { const char *field; long want; } stf[] = {
		{ "Memory Format", V3D_MEMORY_FORMAT_UIF_NO_XOR },
		{ "Output Image Format", V3D_OUTPUT_IMAGE_FORMAT_D24S8 },
		{ "Height in UB or Stride", 13 },
		{ "Clear buffer being stored", 0 },
	};
	check(zs >= 0 && field_has(text, "STORE", zs, "Address", "[zsbuf+0x00000000]"),
	      "Z/S STORE Address: the buffer's first byte (0x00a00000)");
	for (unsigned i = 0; i < sizeof(stf) / sizeof(stf[0]); i++) {
		long got = zs < 0 ? -2 : field(text, "STORE", zs, stf[i].field);
		char what[160];
		snprintf(what, sizeof(what), "Z/S STORE %s: decoded %ld, expected %ld", stf[i].field, got,
			 stf[i].want);
		check(got == stf[i].want, what);
	}
	/* the RT0 store comes before the Z/S one in the tile, as Mesa's order */
	int rt0 = store_of(text, V3D_STORE_BUFFER_RT0);
	check(rt0 >= 0 && zs > rt0, "Z/S job: RT0 stored before Z/S (v3d_rcl_emit_stores)");

	/* stored only: no load */
	c.zs_load = false;
	text = zs_job(&l, &c);
	check(census(text, "LOAD") == 0 && store_of(text, V3D_STORE_BUFFER_ZSTENCIL) >= 0,
	      "Z/S job, store only: no LOAD, the Z/S STORE");
	/* no buffer (address 0): neither, whatever the flags say */
	c.zs_address = 0;
	c.zs_load = true;
	text = zs_job(&l, &c);
	check(census(text, "LOAD") == 0 && store_of(text, V3D_STORE_BUFFER_ZSTENCIL) < 0,
	      "no Z/S buffer: neither a Z/S LOAD nor a Z/S STORE");

	/* depth and stencil apart: each half from and to its own D24S8
	 * buffer, stencil loaded first and stored last (v3dx_rcl.c:220-234,
	 * :288-304), as buffer Z and buffer STENCIL */
	struct v3d_job_clear a = c;
	a.zs_address = 0x00a00000u;
	a.zs_load = a.zs_store = true;
	a.s_address = 0x00b00000u;
	a.s_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR;
	a.s_height_in_ub = 13;
	a.s_load = a.s_store = true;
	text = zs_job(&l, &a);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "apart: the decoder reported no error");
	check(census(text, "LOAD") == 2, "apart: two LOADs, stencil and Z");
	check(field(text, "LOAD", 0, "Buffer to Load") == V3D_LOAD_BUFFER_STENCIL &&
		      field_has(text, "LOAD", 0, "Address", "[sbuf+0x00000000]") &&
		      field(text, "LOAD", 0, "Input Image Format") == V3D_OUTPUT_IMAGE_FORMAT_D24S8 &&
		      field(text, "LOAD", 0, "Height in UB or Stride") == 13,
	      "apart: the first LOAD is the stencil, from its own buffer, D24S8");
	check(field(text, "LOAD", 1, "Buffer to Load") == V3D_LOAD_BUFFER_Z &&
		      field_has(text, "LOAD", 1, "Address", "[zsbuf+0x00000000]") &&
		      field(text, "LOAD", 1, "Input Image Format") == V3D_OUTPUT_IMAGE_FORMAT_D24S8,
	      "apart: the second LOAD is Z, from the depth's buffer, D24S8");
	int zst = store_of(text, V3D_STORE_BUFFER_Z), sst = store_of(text, V3D_STORE_BUFFER_STENCIL);
	check(zst >= 0 && sst > zst && store_of(text, V3D_STORE_BUFFER_ZSTENCIL) < 0 &&
		      field_has(text, "STORE", zst, "Address", "[zsbuf+0x00000000]") &&
		      field_has(text, "STORE", sst, "Address", "[sbuf+0x00000000]"),
	      "apart: Z stored to the depth's buffer, then the stencil to its own; no Z+Stencil store");
	check(field(text, "STORE", zst, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_D24S8 &&
		      field(text, "STORE", sst, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_D24S8 &&
		      field(text, "STORE", sst, "Height in UB or Stride") == 13,
	      "apart: both STOREs D24S8");
	a.zs_load = false;
	text = zs_job(&l, &a);
	check(census(text, "LOAD") == 1 && field(text, "LOAD", 0, "Buffer to Load") == V3D_LOAD_BUFFER_STENCIL,
	      "apart, the stencil loaded only: one LOAD, the stencil");
	a.zs_load = true;
	a.s_store = false;
	text = zs_job(&l, &a);
	check(store_of(text, V3D_STORE_BUFFER_Z) >= 0 && store_of(text, V3D_STORE_BUFFER_STENCIL) < 0,
	      "apart, Z written only: Z stored, the stencil not");
}

/* Part 17: a window that is a view into a wider buffer (src/v3d/backend.rs
 * Target::pitch, GlesDraw::target_pitch): a W-wide target whose rows are
 * W + 32 pixels apart. RT0's LOAD and STORE carry the view's byte stride in
 * "Height in UB or Stride" (raster: the stride in bytes, v3d_packet.xml),
 * read back through Mesa's decoder; the packed target carries W * 4. */
static char *view_job(struct lists *l, const struct v3d_job_clear *zs, uint32_t stride)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, stride, TILE_ALLOC, false };
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) &&
		      v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK && v3d_job_end(&j, zs, NULL, NULL),
	      "a view job builds");
	return decode(l);
}

static void part_view_stride(void)
{
	static struct lists l;
	/* the colour loaded (no clear), as a view's next frame is */
	struct v3d_job_clear c = { .load_rt0 = true, .rgba8 = 0xff202020u, .z = 1.0f };
	const long view = (long)(W + 32u) * 4, packed = (long)W * 4;
	char *text = view_job(&l, &c, (uint32_t)view);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "view job: the decoder reported no error");
	int ld = load_of(text, V3D_LOAD_BUFFER_RT0), st = store_of(text, V3D_STORE_BUFFER_RT0);
	char what[160];
	long got = ld < 0 ? -2 : field(text, "LOAD", ld, "Height in UB or Stride");
	snprintf(what, sizeof(what), "view job: RT0 LOAD stride decoded %ld, expected %ld", got, view);
	check(got == view, what);
	got = st < 0 ? -2 : field(text, "STORE", st, "Height in UB or Stride");
	snprintf(what, sizeof(what), "view job: RT0 STORE stride decoded %ld, expected %ld", got, view);
	check(got == view, what);
	text = view_job(&l, &c, (uint32_t)packed);
	st = store_of(text, V3D_STORE_BUFFER_RT0);
	got = st < 0 ? -2 : field(text, "STORE", st, "Height in UB or Stride");
	snprintf(what, sizeof(what), "packed job: RT0 STORE stride decoded %ld, expected %ld", got, packed);
	check(got == packed, what);
}

/* Part 16: a stencil-only framebuffer's S8 buffer (src/v3d/backend.rs
 * V3dState::s8_stencil). The field values Mesa's load_general and
 * store_general give a separate stencil (v3dx_rcl.c:85-108, :131-170):
 * buffer STENCIL, the S8 resource's tiling and UB height, image format S8,
 * no R/B swap, no force alpha 1, no clear, sample 0, no dither. */
static void part_s8(void)
{
	static struct lists l;
	struct v3d_job_clear c = { .load_rt0 = false, .rgba8 = 0xff202020u, .z = 1.0f,
				   .s_address = 0x00b00000u, .s_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR,
				   .s_height_in_ub = 4, .s_load = true, .s_store = true, .s_s8 = true };
	char *text = zs_job(&l, &c);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "S8: the decoder reported no error");
	check(census(text, "LOAD") == 1, "S8: one LOAD, the stencil (the colour is cleared)");
	check(field(text, "LOAD", 0, "Buffer to Load") == V3D_LOAD_BUFFER_STENCIL &&
		      field(text, "LOAD", 0, "Input Image Format") == V3D_OUTPUT_IMAGE_FORMAT_S8 &&
		      field(text, "LOAD", 0, "Memory Format") == V3D_MEMORY_FORMAT_UIF_NO_XOR &&
		      field(text, "LOAD", 0, "Height in UB or Stride") == 4 &&
		      field_has(text, "LOAD", 0, "Address", "[sbuf+0x00000000]"),
	      "S8: the LOAD is the stencil, S8, UIF, 4 UIF blocks, from its buffer");
	check(store_of(text, V3D_STORE_BUFFER_Z) < 0 && store_of(text, V3D_STORE_BUFFER_ZSTENCIL) < 0,
	      "S8: nothing of Z stored (no depth buffer)");
	int st = store_of(text, V3D_STORE_BUFFER_STENCIL);
	check(st >= 0 && field(text, "STORE", st, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_S8 &&
		      field(text, "STORE", st, "Height in UB or Stride") == 4 &&
		      field(text, "STORE", st, "Clear buffer being stored") == 0 &&
		      field_has(text, "STORE", st, "Address", "[sbuf+0x00000000]"),
	      "S8: the stencil STORE, S8, 4 UIF blocks, no clear, to its buffer");

	uint8_t ld[V3D42_LOAD_TILE_BUFFER_GENERAL_length];
	struct V3D42_LOAD_TILE_BUFFER_GENERAL ml = { V3D42_LOAD_TILE_BUFFER_GENERAL_header };
	ml.buffer_to_load = V3D_LOAD_BUFFER_STENCIL;
	ml.address = 0x00b00000u;
	ml.memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR; /* Mesa: the S8 slice's tiling; cl_golden pins the value */
	ml.input_image_format = V3D_OUTPUT_IMAGE_FORMAT_S8;
	ml.r_b_swap = false;
	ml.force_alpha_1 = false;
	ml.height_in_ub_or_stride = 4;
	ml.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
	V3D42_LOAD_TILE_BUFFER_GENERAL_pack(NULL, ld, &ml);
	check(memmem(l.bo, sizeof(l.bo), ld, sizeof(ld)) != NULL,
	      "S8: the LOAD's bytes are Mesa's load_general for a separate stencil");

	uint8_t sd[V3D42_STORE_TILE_BUFFER_GENERAL_length];
	struct V3D42_STORE_TILE_BUFFER_GENERAL ms = { V3D42_STORE_TILE_BUFFER_GENERAL_header };
	ms.buffer_to_store = V3D_STORE_BUFFER_STENCIL;
	ms.address = 0x00b00000u;
	ms.clear_buffer_being_stored = false;
	ms.output_image_format = V3D_OUTPUT_IMAGE_FORMAT_S8;
	ms.r_b_swap = false;
	ms.memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR; /* Mesa: the S8 slice's tiling; cl_golden pins the value */
	ms.height_in_ub_or_stride = 4;
	ms.decimate_mode = V3D_DECIMATE_MODE_SAMPLE_0;
	V3D42_STORE_TILE_BUFFER_GENERAL_pack(NULL, sd, &ms);
	check(memmem(l.bo, sizeof(l.bo), sd, sizeof(sd)) != NULL,
	      "S8: the STORE's bytes are Mesa's store_general for a separate stencil");
}

/* Part 18: a job with no colour buffer (target address 0, a depth/stencil-
 * only framebuffer; src/v3d/backend.rs DRAWS_WITHOUT_COLOUR). Mesa's job
 * with nr_cbufs 0 loads and stores no render target, and a tile list that
 * stores nothing stores buffer NONE (v3dx_rcl.c:315-321). (a) depth kept:
 * the Z/S LOAD and STORE, nothing of RT 0. (b) nothing kept: one NONE
 * STORE in the tile list, after the two dummy tiles' ones. (c) 4x with no
 * colour buffer is refused at begin. */
static char *no_colour_job(struct lists *l, const struct v3d_job_clear *c)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, 0, W * 4, TILE_ALLOC, false };
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK &&
		      v3d_job_end(&j, c, NULL, NULL),
	      "no colour: the job builds");
	return decode(l);
}

static void part_no_colour(void)
{
	static struct lists l;
	/* (a) loaded and stored, as part 7's buffer; load_rt0 set on purpose:
	 * with no colour buffer it must load nothing */
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f, .zs_address = 0x00a00000u,
				   .zs_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR, .zs_height_in_ub = 13,
				   .zs_load = true, .zs_store = true };
	char *text = no_colour_job(&l, &c);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "no colour, Z kept: the decoder reported no error");
	check(census(text, "LOAD") == 1 && load_of(text, V3D_LOAD_BUFFER_ZSTENCIL) == 0,
	      "no colour, Z kept: one LOAD, the Z/S buffer, nothing of RT 0");
	check(store_of(text, V3D_STORE_BUFFER_RT0) < 0, "no colour, Z kept: no RT 0 STORE");
	int zs = store_of(text, V3D_STORE_BUFFER_ZSTENCIL);
	check(census(text, "STORE") == 3 && zs == 2 && field_has(text, "STORE", zs, "Address", "[zsbuf+0x00000000]"),
	      "no colour, Z kept: three STOREs (two dummy tiles, Z/S), the Z/S one to its buffer");
	/* (b) nothing kept */
	struct v3d_job_clear n = { .load_rt0 = true, .z = 1.0f };
	text = no_colour_job(&l, &n);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "no colour, nothing kept: the decoder reported no error");
	check(census(text, "LOAD") == 0, "no colour, nothing kept: no LOAD");
	check(census(text, "STORE") == 3 && field(text, "STORE", 2, "Buffer to Store") == V3D_STORE_BUFFER_NONE,
	      "no colour, nothing kept: the tile list's one STORE is of buffer NONE (v3dx_rcl.c:315-321)");
	/* (c) 4x */
	struct v3d_job_target t4 = { W, H, 0, W * 4, TILE_ALLOC, false };
	t4.msaa = true;
	struct v3d_job j;
	lists_init(&l, RCL_OFF - BCL_OFF);
	check(!v3d_job_begin(&j, &t4, &l.bcl, &l.rcl, &l.ind), "no colour at 4x: refused at begin");
}

/* A target with no alpha (GL_RGB, GL_RGB565, stored as RGBA8): the colour
 * LOAD sets Force alpha 1, as Mesa's v3dx_rcl.c:99 does for a format whose
 * alpha swizzle is 1; any other target's does not. */
static void part_rt_alpha(void)
{
	static struct lists l;
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f, .force_alpha_1 = true };
	char *text = zs_job(&l, &c);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "RGB job: the decoder reported no error");
	check(census(text, "LOAD") == 1 && field(text, "LOAD", 0, "Buffer to Load") == V3D_LOAD_BUFFER_RT0,
	      "RGB job: one LOAD, the colour (render target 0)");
	check(field(text, "LOAD", 0, "Force alpha 1") == 1, "RGB job: the colour LOAD forces alpha 1");
	c.force_alpha_1 = false;
	text = zs_job(&l, &c);
	check(field(text, "LOAD", 0, "Force alpha 1") == 0, "RGBA job: the colour LOAD keeps alpha");
	check(field(text, "LOAD", 0, "R/B swap") == 0 &&
		      field(text, "STORE", store_of(text, V3D_STORE_BUFFER_RT0), "R/B swap") == 0,
	      "RGBA job: no R/B swap at the colour LOAD or STORE");
	/* an XRGB8888 target: R and B swapped both ways, alpha forced to 1 */
	c.force_alpha_1 = true;
	c.r_b_swap = true;
	text = zs_job(&l, &c);
	check(field(text, "LOAD", 0, "R/B swap") == 1 && field(text, "LOAD", 0, "Force alpha 1") == 1,
	      "XRGB job: the colour LOAD swaps R/B and forces alpha 1");
	check(field(text, "STORE", store_of(text, V3D_STORE_BUFFER_RT0), "R/B swap") == 1,
	      "XRGB job: the RT0 STORE swaps R/B");
	/* an RGB565 target: bgr565 both ways at 2 bytes a pixel, no swap
	 * (Mesa's B5G6R5_UNORM row, v3dx_format_table.c:81; v3d_formats.c:176) */
	c.r_b_swap = false;
	text = one_job_on(&l, &c, false, V3D_PRIM_TRIANGLES, true, false, false);
	int rt0 = store_of(text, V3D_STORE_BUFFER_RT0);
	check(field(text, "LOAD", 0, "Input Image Format") == V3D_OUTPUT_IMAGE_FORMAT_BGR565 &&
		      field(text, "LOAD", 0, "Height in UB or Stride") == W * 2,
	      "RGB565 job: the colour LOAD reads bgr565 at 2 bytes a pixel");
	check(field(text, "STORE", rt0, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_BGR565 &&
		      field(text, "STORE", rt0, "Height in UB or Stride") == W * 2 &&
		      field(text, "STORE", rt0, "R/B swap") == 0,
	      "RGB565 job: the RT0 STORE writes bgr565 at 2 bytes a pixel, no swap");
}

/* A job on `rts` render targets: RT 0 the usual target, RT 1..3 at
 * rt_addr[n - 1] (0: a GL_NONE draw buffer). */
static char *mrt_job(struct lists *l, const struct v3d_job_clear *c, uint32_t rts, const uint32_t rt_addr[3])
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	t.rts = rts;
	for (int n = 0; n < 3; n++)
		t.rt[n] = (struct v3d_job_rt){ rt_addr[n], W * 4 };
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) &&
		      v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK && v3d_job_end(&j, c, NULL, NULL),
	      "an MRT job builds");
	return decode(l);
}

static void part_mrt(void)
{
	static struct lists l;
	const uint32_t addr[3] = { 0x00c00000u, 0, 0x00d00000u };
	struct v3d_job_clear c = { .load_rt0 = false, .rgba8 = 0x11223344u, .z = 1.0f,
				   .load_rt = { true, false, false }, .rgba8_rt = { 0, 0x55667788u, 0x99aabbccu } };
	/* 3 RTs: RT 1 loaded (not cleared), RT 2 a GL_NONE buffer, cleared */
	char *text = mrt_job(&l, &c, 3, addr);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "MRT job: the decoder reported no error");
	check(field(text, "TILE_BINNING_MODE_CFG", 0, "Number of Render Targets") == 3 &&
		      field(text, "TILE_RENDERING_MODE_CFG_COMMON", 0, "Number of Render Targets") == 3,
	      "3 RTs: the binning and rendering configs say 3");
	/* v3d_choose_tile_size 4.x, 3-4 RTs: 32x32, so 200x100 is 7x4 tiles */
	check(field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Width in Tiles") == 7 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Height in Tiles") == 4,
	      "3 RTs: 32x32 tiles (7 x 4 for 200 x 100)");
	check(census(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1") == 3 &&
		      field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 0, "Render Target number") == 0 &&
		      field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 1, "Render Target number") == 1 &&
		      field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 2, "Render Target number") == 2,
	      "3 RTs: one CLEAR_COLORS_PART1 per RT, in RT order");
	check(field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 0, "Clear Color low 32 bits") == 0x11223344 &&
		      field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 1, "Clear Color low 32 bits") == 0 &&
		      field(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 2, "Clear Color low 32 bits") ==
			      0x55667788,
	      "3 RTs: each RT's clear colour, 0 for the loaded RT 1");
	check(field(text, "TILE_RENDERING_MODE_CFG_COLOR", 0, "Render Target 2 Internal Type") == V3D_INTERNAL_TYPE_8 &&
		      field(text, "TILE_RENDERING_MODE_CFG_COLOR", 0, "Render Target 3 Internal Type") == 0,
	      "3 RTs: COLOR configures RT 0..2 (type 8) and leaves RT 3 at 0");
	check(load_of(text, V3D_LOAD_BUFFER_RT0 + 1) >= 0 && load_of(text, V3D_LOAD_BUFFER_RT0 + 2) < 0,
	      "3 RTs: RT 1 loaded, RT 2 (GL_NONE) not");
	int s1 = store_of(text, V3D_STORE_BUFFER_RT0 + 1);
	check(store_of(text, V3D_STORE_BUFFER_RT0) >= 0 && s1 >= 0 && store_of(text, V3D_STORE_BUFFER_RT0 + 2) < 0,
	      "3 RTs: RT 0 and RT 1 stored, RT 2 (GL_NONE) not");
	check(field_has(text, "STORE", s1, "Address", "0x00c00000") &&
		      field(text, "STORE", s1, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_RGBA8 &&
		      field(text, "STORE", s1, "Height in UB or Stride") == W * 4,
	      "3 RTs: RT 1's STORE writes its own buffer, raster RGBA8");
	/* 2 RTs: 64x32 tiles, 4 x 4 for 200 x 100 */
	const uint32_t two[3] = { 0x00c00000u, 0, 0 };
	text = mrt_job(&l, &c, 2, two);
	check(field(text, "TILE_RENDERING_MODE_CFG_COMMON", 0, "Number of Render Targets") == 2 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Width in Tiles") == 4 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Height in Tiles") == 4,
	      "2 RTs: 64x32 tiles (4 x 4 for 200 x 100)");
	/* 1 RT (rts 0 and 1 alike): the job as before, 64x64 */
	text = mrt_job(&l, &c, 0, two);
	check(field(text, "TILE_RENDERING_MODE_CFG_COMMON", 0, "Number of Render Targets") == 1 &&
		      census(text, "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1") == 1 &&
		      store_of(text, V3D_STORE_BUFFER_RT0 + 1) < 0 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Width in Tiles") == 4 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Height in Tiles") == 2,
	      "rts 0: one RT, 64x64 tiles, RT 1 ignored");
	/* more than 4: refused at begin */
	struct v3d_job_target t5 = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	t5.rts = 5;
	struct v3d_job j5;
	lists_init(&l, RCL_OFF - BCL_OFF);
	check(!v3d_job_begin(&j5, &t5, &l.bcl, &l.rcl, &l.ind), "5 RTs: refused");
}

/* The record's Point size in shaded vertex data is the draw's
 * vs_writes_psiz (Mesa: set exactly when its VS writes the size, the points
 * variant, v3dx_draw.c:556-558). */
static void part_psiz(void)
{
	static struct lists l;
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f };
	const char *rec = "@format shadrec_gl_main";
	char *text = one_job(&l, &c, true, V3D_PRIM_TRIANGLES);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "psiz job: the decoder reported no error");
	/* a field the record always sets: the record was found and read */
	check(field(text, rec, 0, "Enable clipping") == 1, "psiz job: the record decoded (Enable clipping 1)");
	check(field(text, rec, 0, "Point size in shaded vertex data") == 1,
	      "psiz job: the record says the point size is in the shaded vertex data");
	text = one_job(&l, &c, false, V3D_PRIM_TRIANGLES);
	check(field(text, rec, 0, "Point size in shaded vertex data") == 0,
	      "no psiz: the record says the point size is not there");
	/* a points draw: recorded (the triangles program serves points, measured
	 * on the board by gl_test 110), with the bit, as VERTEX_ARRAY_PRIMS mode 0 */
	text = one_job(&l, &c, true, V3D_PRIM_POINTS);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "points job: the decoder reported no error");
	check(field(text, "VERTEX_ARRAY_PRIMS", 0, "mode") == V3D_PRIM_POINTS &&
		      field(text, "VERTEX_ARRAY_PRIMS", 0, "Length") == 3,
	      "points job: VERTEX_ARRAY_PRIMS mode 0 (points), 3 vertices");
	check(field(text, rec, 0, "Point size in shaded vertex data") == 1,
	      "points job: the record carries the point size");
}

/* A discarding FS (writes_z and disable_ez from the compiler): the record
 * says "Fragment shader does Z writes" and "Turn off early-z test", as Mesa's
 * does (v3dx_draw.c:560-575), so the TLB updates Z and stencil at the
 * shader's own Z write and a discarded fragment updates neither. Without
 * them the write is implicit and a discarded fragment still updates stencil
 * (femtovg's lost stroke fringe, confirmed on the board). */
static void part_discard(void)
{
	static struct lists l;
	const char *rec = "@format shadrec_gl_main";
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f };
	char *text = one_job_on(&l, &c, false, V3D_PRIM_TRIANGLES, false, true, true);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "discard job: the decoder reported no error");
	check(field(text, rec, 0, "Enable clipping") == 1, "discard job: the record decoded (Enable clipping 1)");
	check(field(text, rec, 0, "Fragment shader does Z writes") == 1 &&
		      field(text, rec, 0, "Turn off early-z test") == 1,
	      "discard job: the record says the FS writes Z and turns early Z off");
	text = one_job_on(&l, &c, false, V3D_PRIM_TRIANGLES, false, false, false);
	check(field(text, rec, 0, "Fragment shader does Z writes") == 0 &&
		      field(text, rec, 0, "Turn off early-z test") == 0,
	      "plain job: the record says neither");
	/* an FS reading gl_FragCoord.z (v3d-m5, aa391d2f): Z written, early Z
	 * kept; the only case that tells the two copies apart */
	text = one_job_on(&l, &c, false, V3D_PRIM_TRIANGLES, false, true, false);
	check(field(text, rec, 0, "Fragment shader does Z writes") == 1 &&
		      field(text, rec, 0, "Turn off early-z test") == 0,
	      "reads-Z job: the record says the FS writes Z, early Z not turned off");
}

/* ---- 11. gl_PointCoord: points only -------------------------------------
 * A program reading the implicit point varyings (Mesa's
 * uses_implicit_point_line_varyings, v3d-m5's FsProg flag) clears the
 * record's "Disable implicit point/line varyings" and is drawn as points
 * only: a triangle or a line would read every declared varying two slots
 * off, so the job builder REFUSES it with the lists as they were (the
 * backend refuses it by name first). */
static void part_pntc(void)
{
	static struct lists l;
	const char *rec = "@format shadrec_gl_main";
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f };
	char *text;

	one_job_implicit = true;
	text = one_job_on(&l, &c, false, V3D_PRIM_POINTS, false, false, false);
	check(field(text, rec, 0, "Disable implicit point/line varyings") == 0,
	      "gl_PointCoord points: the record keeps the implicit varyings");
	one_job_implicit = false;
	text = one_job_on(&l, &c, false, V3D_PRIM_POINTS, false, false, false);
	check(field(text, rec, 0, "Disable implicit point/line varyings") == 1,
	      "plain points: the record disables the implicit varyings");

	struct v3d_fs_prog fs = fs_prog();
	fs.uses_implicit_point_line_varyings = true;
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	lists_init(&rl, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &rl.bcl, &rl.rcl, &rl.ind), "the gl_PointCoord job begins");
	static const struct {
		uint32_t prim;
		const char *what;
	} bad[] = {
		{ V3D_PRIM_TRIANGLES, "gl_PointCoord drawn as triangles: REFUSED, nothing left" },
		{ V3D_PRIM_LINES, "gl_PointCoord drawn as lines: REFUSED, nothing left" },
	};
	for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
		struct v3d_job_draw d = {
			.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
			.attrs = attrs, .num_attrs = 2, .prim = bad[i].prim, .count = 3,
		};
		snapshot();
		check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_REFUSED && unchanged(), bad[i].what);
	}
}

/* ---- 12. instanced draws ---------------------------------------------------
 * A draw of more than one instance is the instanced packet of its kind
 * (VERTEX_ARRAY_INSTANCED_PRIMS / INDEXED_INSTANCED_PRIM_LIST, Mesa
 * v3dx_draw.c:1322-1366), with the draw's count as the instance length, and
 * a per-instance array's record carries its Instance Divisor
 * (v3dx_state.c:388-389); one instance stays the plain packet. Read back by
 * Mesa's decoder. */
static char *inst_job(struct lists *l, uint32_t instances, bool indexed)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4, 0 }, { 0x00300010u, 16, 4, 1 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
		.instances = instances,
	};
	if (indexed) {
		d.indexed = true;
		d.index_type = V3D_INDEX_TYPE_16;
		d.index_buffer = 0x00310000u;
		d.index_buffer_bytes = 6;
	}
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f };
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	check(v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK &&
		      v3d_job_end(&j, &c, NULL, NULL),
	      "an instanced job builds");
	return decode(l);
}

static void part_instanced(void)
{
	static struct lists l;
	const char *attr = "@format shadrec_gl_attr";
	char *text = inst_job(&l, 4, false);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "4 instances, arrays: the decoder reported no error");
	check(census(text, "VERTEX_ARRAY_INSTANCED_PRIMS") == 1 && census(text, "VERTEX_ARRAY_PRIMS") == 0,
	      "4 instances, arrays: one VERTEX_ARRAY_INSTANCED_PRIMS, no VERTEX_ARRAY_PRIMS");
	check(field(text, "VERTEX_ARRAY_INSTANCED_PRIMS", 0, "Instance Length") == 3 &&
		      field(text, "VERTEX_ARRAY_INSTANCED_PRIMS", 0, "Number of Instances") == 4 &&
		      field(text, "VERTEX_ARRAY_INSTANCED_PRIMS", 0, "Index of First Vertex") == 0 &&
		      field(text, "VERTEX_ARRAY_INSTANCED_PRIMS", 0, "mode") == V3D_PRIM_TRIANGLES,
	      "4 instances, arrays: 3 vertices an instance, 4 instances, from vertex 0, triangles");
	check(field(text, attr, 0, "Instance Divisor") == 0 && field(text, attr, 1, "Instance Divisor") == 1,
	      "4 instances: record 0 per vertex (divisor 0), record 1 per instance (divisor 1)");
	text = inst_job(&l, 4, true);
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "4 instances, indexed: the decoder reported no error");
	check(census(text, "INDEXED_INSTANCED_PRIM_LIST") == 1 && census(text, "INDEXED_PRIM_LIST") == 0,
	      "4 instances, indexed: one INDEXED_INSTANCED_PRIM_LIST, no INDEXED_PRIM_LIST");
	check(field(text, "INDEXED_INSTANCED_PRIM_LIST", 0, "Instance Length") == 3 &&
		      field(text, "INDEXED_INSTANCED_PRIM_LIST", 0, "Number of Instances") == 4 &&
		      field(text, "INDEXED_INSTANCED_PRIM_LIST", 0, "Index type") == V3D_INDEX_TYPE_16,
	      "4 instances, indexed: 3 indices an instance, 4 instances, 16-bit indices");
	text = inst_job(&l, 1, false);
	check(census(text, "VERTEX_ARRAY_PRIMS") == 1 && census(text, "VERTEX_ARRAY_INSTANCED_PRIMS") == 0,
	      "1 instance: the plain VERTEX_ARRAY_PRIMS");
}

/* A job at 4x (msaa) or not, on the usual target, whose RT 0 samples are
 * kept at `ms` (0: not kept) and whose draw's sample mask is `mask`. */
static char *ms_job(struct lists *l, const struct v3d_job_clear *c, bool msaa, uint32_t ms, uint32_t mask, bool *ended)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	st.sample_mask = mask;
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	t.msaa = msaa;
	t.ms_address = ms;
	t.ms_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR;
	t.ms_height_in_ub = 26;
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	bool ok = v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind) && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK;
	*ended = ok && v3d_job_end(&j, c, NULL, NULL);
	return decode(l);
}

/* 4x multisampling (Mesa's job->msaa): the two config bits, the tile size,
 * the rasterizer oversample, the sample mask, RT 0's samples stored and
 * loaded with ALL_SAMPLES and the resolve stored with 4X, Z/S with
 * ALL_SAMPLES; and what is refused. */
static void part_msaa(void)
{
	static struct lists l;
	bool ended;
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f, .zs_address = 0x00a00000u,
				   .zs_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR, .zs_height_in_ub = 26,
				   .zs_load = true, .zs_store = true };
	char *text = ms_job(&l, &c, true, MS_BUF, 0x5, &ended);
	check(ended, "a 4x job builds");
	check(!strstr(text, "Failed") && !strstr(text, "nknown"), "4x job: the decoder reported no error");
	check(field(text, "TILE_BINNING_MODE_CFG", 0, "Multisample Mode (4x)") == 1 &&
		      field(text, "TILE_RENDERING_MODE_CFG_COMMON", 0, "Multisample Mode (4x)") == 1,
	      "4x job: the binning and rendering configs say 4x");
	/* v3d_choose_tile_size 4.x, 1 RT at 4x: idx 2, 32x32, so 200x100 is 7x4 */
	check(field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Width in Tiles") == 7 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Height in Tiles") == 4,
	      "4x job: 32x32 tiles (7 x 4 for 200 x 100)");
	check(field(text, "CFG_BITS", 0, "Rasterizer Oversample Mode") == 1, "4x job: rasterizer oversample on");
	check(field(text, "SAMPLE_STATE", 0, "Mask") == 0x5, "4x job: SAMPLE_STATE takes the draw's mask");
	int s0 = store_of(text, V3D_STORE_BUFFER_RT0);
	check(s0 >= 0 && field_has(text, "STORE", s0, "Address", "[msbuf+0x00000000]") &&
		      field(text, "STORE", s0, "Decimate mode") == V3D_DECIMATE_MODE_ALL_SAMPLES &&
		      field(text, "STORE", s0, "Memory Format") == V3D_MEMORY_FORMAT_UIF_NO_XOR,
	      "4x job: RT 0's samples stored first, ALL_SAMPLES into the UIF buffer");
	check(field(text, "STORE", s0 + 1, "Buffer to Store") == V3D_STORE_BUFFER_RT0 &&
		      field_has(text, "STORE", s0 + 1, "Address", "[target+0x00000000]") &&
		      field(text, "STORE", s0 + 1, "Decimate mode") == V3D_DECIMATE_MODE_4X &&
		      field(text, "STORE", s0 + 1, "Memory Format") == V3D_MEMORY_FORMAT_RASTER,
	      "4x job: then the resolve, 4X into the raster target");
	int l0 = load_of(text, V3D_LOAD_BUFFER_RT0);
	check(l0 >= 0 && field_has(text, "LOAD", l0, "Address", "[msbuf+0x00000000]") &&
		      field(text, "LOAD", l0, "Decimate mode") == V3D_DECIMATE_MODE_ALL_SAMPLES,
	      "4x job: RT 0 loaded from the kept samples, ALL_SAMPLES");
	int lz = load_of(text, V3D_LOAD_BUFFER_ZSTENCIL), sz = store_of(text, V3D_STORE_BUFFER_ZSTENCIL);
	check(lz >= 0 && sz >= 0 && field(text, "LOAD", lz, "Decimate mode") == V3D_DECIMATE_MODE_ALL_SAMPLES &&
		      field(text, "STORE", sz, "Decimate mode") == V3D_DECIMATE_MODE_ALL_SAMPLES,
	      "4x job: Z/S loaded and stored with ALL_SAMPLES");
	/* samples not kept: the resolve alone; a load of RT 0 is refused at end */
	struct v3d_job_clear cleared = { .load_rt0 = false, .rgba8 = 0xff0000ffu, .z = 1.0f };
	text = ms_job(&l, &cleared, true, 0, 0xf, &ended);
	check(ended && store_of(text, V3D_STORE_BUFFER_RT0) >= 0 &&
		      field(text, "STORE", store_of(text, V3D_STORE_BUFFER_RT0), "Decimate mode") == V3D_DECIMATE_MODE_4X &&
		      field(text, "STORE", store_of(text, V3D_STORE_BUFFER_RT0) + 1, "Buffer to Store") !=
			      V3D_STORE_BUFFER_RT0,
	      "4x, samples not kept: one RT 0 store, the resolve");
	struct v3d_job_clear loads = { .load_rt0 = true, .z = 1.0f };
	(void)ms_job(&l, &loads, true, 0, 0xf, &ended);
	check(!ended, "4x, samples not kept: a job that loads RT 0 is refused");
	/* 4x with two RTs: refused at begin */
	struct v3d_job_target t2 = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	t2.rts = 2;
	t2.msaa = true;
	struct v3d_job j2;
	lists_init(&l, RCL_OFF - BCL_OFF);
	check(!v3d_job_begin(&j2, &t2, &l.bcl, &l.rcl, &l.ind), "4x with 2 RTs: refused");
	/* a single-sampled job: none of it, whatever the draw's mask holds */
	text = ms_job(&l, &cleared, false, MS_BUF, 0x5, &ended);
	check(ended && field(text, "TILE_BINNING_MODE_CFG", 0, "Multisample Mode (4x)") == 0 &&
		      field(text, "CFG_BITS", 0, "Rasterizer Oversample Mode") == 0 &&
		      field(text, "SAMPLE_STATE", 0, "Mask") == 0xf &&
		      field(text, "STORE", store_of(text, V3D_STORE_BUFFER_RT0), "Decimate mode") == V3D_DECIMATE_MODE_SAMPLE_0 &&
		      field(text, "MULTICORE_RENDERING_SUPERTILE_CFG", 0, "Total Frame Width in Tiles") == 4,
	      "1x job: no 4x bit, no oversample, mask 0xf, sample 0 stored, 64x64 tiles");
}

/* ---- 20. the render image (v3d_job.h uif_*) ------------------------------ */

/* A single-sampled job on the usual target whose RT 0 is also stored into
 * the render image at `uif` (0: none), `rgb565` and `msaa` as given. */
static char *rt_job(struct lists *l, const struct v3d_job_clear *c, uint32_t uif, bool rgb565, bool msaa,
		    uint32_t address, bool *ended)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, address, rgb565 ? W * 2 : W * 4, TILE_ALLOC, rgb565 };
	t.msaa = msaa;
	t.uif_address = uif;
	t.uif_memory_format = V3D_MEMORY_FORMAT_UIF_NO_XOR;
	/* H = 100 rows: Mesa pads to 104 (13 UIF blocks of 8 rows) */
	t.uif_height_in_ub = 13;
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	bool ok = v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind);
	if (ok)
		ok = v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK;
	*ended = ok && v3d_job_end(&j, c, NULL, NULL);
	return ok ? decode(l) : NULL;
}

/* The render image (src/v3d/backend.rs RtImage): RT 0 stored a second time,
 * UIF into the image right after the raster store, sample 0, RGBA8; the
 * load still from the raster target; and the targets no image is made for
 * are refused at begin. */
static void part_rt_image(void)
{
	static struct lists l;
	bool ended;
	struct v3d_job_clear c = { .load_rt0 = true, .z = 1.0f };
	char *text = rt_job(&l, &c, RT_IMG, false, false, TARGET, &ended);
	check(ended && text, "render image: the job builds");
	check(text && !strstr(text, "Failed") && !strstr(text, "nknown"),
	      "render image: the decoder reported no error");
	int s0 = text ? store_of(text, V3D_STORE_BUFFER_RT0) : -1;
	check(s0 >= 0 && field_has(text, "STORE", s0, "Address", "[target+0x00000000]") &&
		      field(text, "STORE", s0, "Memory Format") == V3D_MEMORY_FORMAT_RASTER &&
		      field(text, "STORE", s0, "Decimate mode") == V3D_DECIMATE_MODE_SAMPLE_0,
	      "render image: RT 0 stored raster into the target first (as without an image)");
	check(s0 >= 0 && field(text, "STORE", s0 + 1, "Buffer to Store") == V3D_STORE_BUFFER_RT0 &&
		      field_has(text, "STORE", s0 + 1, "Address", "[rtimg+0x00000000]") &&
		      field(text, "STORE", s0 + 1, "Memory Format") == V3D_MEMORY_FORMAT_UIF_NO_XOR &&
		      field(text, "STORE", s0 + 1, "Height in UB or Stride") == 13 &&
		      field(text, "STORE", s0 + 1, "Output Image Format") == V3D_OUTPUT_IMAGE_FORMAT_RGBA8 &&
		      field(text, "STORE", s0 + 1, "Decimate mode") == V3D_DECIMATE_MODE_SAMPLE_0,
	      "render image: then RT 0 again, UIF (no XOR) into the image, 13 UB rows, RGBA8, sample 0");
	check(s0 >= 0 && field(text, "STORE", s0 + 2, "Buffer to Store") != V3D_STORE_BUFFER_RT0,
	      "render image: no third RT 0 store");
	int l0 = text ? load_of(text, V3D_LOAD_BUFFER_RT0) : -1;
	check(l0 >= 0 && field_has(text, "LOAD", l0, "Address", "[target+0x00000000]") &&
		      field(text, "LOAD", l0, "Memory Format") == V3D_MEMORY_FORMAT_RASTER &&
		      field(text, "LOAD", l0 + 1, "Buffer to Load") != V3D_LOAD_BUFFER_RT0,
	      "render image: RT 0 loaded from the raster target alone, never from the image");
	free(text);
	/* refused at begin: 4x, bgr565, no colour buffer */
	(void)rt_job(&l, &c, RT_IMG, false, true, TARGET, &ended);
	check(!ended, "render image at 4x: refused");
	(void)rt_job(&l, &c, RT_IMG, true, false, TARGET, &ended);
	check(!ended, "render image of a bgr565 target: refused");
	struct v3d_job_clear nc = { .load_rt0 = false, .z = 1.0f };
	(void)rt_job(&l, &nc, RT_IMG, false, false, 0, &ended);
	check(!ended, "render image with no colour buffer: refused");
}

/* ---- 15. packet skip (v3d_job.h "Packet skip") -------------------------- */

/* A BCL walked packet by packet with Mesa's own packet lengths and names
 * (v3d_packet.xml through cle/v3d_decoder.h: the packet's shortname when it
 * has one, as CLIP for CLIP_WINDOW, in upper case with "_" for anything not
 * a letter or digit): how many of each opcode, and
 * at every primitive a hash of the state the list holds then. The state is
 * the last packet of each kind (sticky, v3d_job.h), keyed by opcode, except
 * that STENCIL_CFG is keyed by the faces it sets and a ZERO_ALL_*_FLAGS
 * packet shares the key of the flags packet it zeroes. */
#define WALK_PRIMS 64
struct walk {
	bool ok;         /* every packet known to the spec, none longer than 16 */
	uint32_t packets, prims;
	uint32_t count[256];
	char name[256][48];
	uint64_t state[WALK_PRIMS];
};

static struct v3d_spec *walk_spec;

static void walk(const struct v3d_cl *bcl, struct walk *w)
{
	static uint8_t last[256 + 4][16];
	static uint8_t last_len[256 + 4];
	memset(w, 0, sizeof(*w));
	memset(last_len, 0, sizeof(last_len));
	if (!walk_spec) {
		struct v3d_device_info devinfo = { .ver = 42, .has_accumulators = true,
						   .cle_readahead = 256 };
		walk_spec = v3d_spec_load(&devinfo);
	}
	w->ok = walk_spec != NULL;
	for (uint32_t at = 0; w->ok && at < bcl->used;) {
		const uint8_t *p = bcl->buf + at;
		struct v3d_group *g = v3d_spec_find_instruction(walk_spec, p);
		int len = g ? v3d_group_get_length(g) : 0;
		if (len <= 0 || len > 16 || at + (uint32_t)len > bcl->used) {
			w->ok = false;
			break;
		}
		const char *xml = v3d_group_get_name(g);
		char *name = w->name[p[0]];
		size_t n = 0;
		for (; xml[n] && n + 1 < sizeof(w->name[0]); n++) {
			char ch = xml[n];
			bool alnum = (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
				     (ch >= '0' && ch <= '9');
			name[n] = !alnum ? '_' : (ch >= 'a' && ch <= 'z') ? (char)(ch - 'a' + 'A') : ch;
		}
		name[n] = 0;
		w->count[p[0]]++;
		w->packets++;
		if (strcmp(name, "VERTEX_ARRAY_PRIMS") == 0 || strcmp(name, "INDEXED_PRIM_LIST") == 0 ||
		    strcmp(name, "VERTEX_ARRAY_INSTANCED_PRIMS") == 0 ||
		    strcmp(name, "INDEXED_INSTANCED_PRIM_LIST") == 0) {
			uint64_t h = 0xcbf29ce484222325ull;
			for (uint32_t k = 0; k < 256 + 4; k++) {
				if (!last_len[k])
					continue;
				h = (h ^ k) * 0x100000001b3ull;
				for (uint32_t b = 0; b < last_len[k]; b++)
					h = (h ^ last[k][b]) * 0x100000001b3ull;
			}
			if (w->prims < WALK_PRIMS)
				w->state[w->prims] = h;
			w->prims++;
		} else {
			uint32_t key = p[0];
			if (p[0] == 80) /* STENCIL_CFG: its front and back bits (field bits 28, 29) */
				key = 256 + ((p[4] >> 4) & 3);
			else if (p[0] == V3D42_ZERO_ALL_FLAT_SHADE_FLAGS_opcode)
				key = V3D42_FLAT_SHADE_FLAGS_opcode;
			else if (p[0] == V3D42_ZERO_ALL_NON_PERSPECTIVE_FLAGS_opcode)
				key = V3D42_NON_PERSPECTIVE_FLAGS_opcode;
			else if (p[0] == V3D42_ZERO_ALL_CENTROID_FLAGS_opcode)
				key = V3D42_CENTROID_FLAGS_opcode;
			memcpy(last[key], p, (size_t)len);
			last_len[key] = (uint8_t)len;
		}
		at += (uint32_t)len;
	}
}

/* How many packets the walk saw with this name (Mesa's). */
static uint32_t wcount(const struct walk *w, const char *name)
{
	for (int op = 0; op < 256; op++)
		if (w->count[op] && strcmp(w->name[op], name) == 0)
			return w->count[op];
	return 0;
}

/* The packets v3d_job_begin and v3d_job_end write around the draws:
 * TILE_BINNING_MODE_CFG, FLUSH_VCD_CACHE, OCCLUSION_QUERY_COUNTER,
 * START_TILE_BINNING, and FLUSH. */
#define JOB_FRAME_PACKETS 5

/* The state packets of an m5_state() triangle draw, each once, and the
 * packets written on every draw. */
static const char *const m5_state_packets[] = {
	"CLIP", "CFG_BITS", "LINE_WIDTH", "CLIPPER_XY",
	"CLIPPER_Z", "CLIPZ", "VIEWPORT_OFFSET",
	"BLEND_ENABLES", "COLOR_WMASKS", "BLEND_CCOLOR", "ZERO_ALL_FLATSHADE_FLAGS",
	"ZERO_ALL_NOPERSPECTIVE_FLAGS", "ZERO_ALL_CENTROID_FLAGS", "TRANSFORM_FEEDBACK_SPECS",
	"SAMPLE_STATE",
};
static const char *const draw_packets[] = {
	"POINT_SIZE", "VCM_CACHE_SIZE", "GL_SHADER", "VERTEX_ARRAY_PRIMS",
};

/* Every name in `names` seen exactly n times; the first that is not goes
 * into `bad`. */
static bool all_n(const struct walk *w, const char *const *names, size_t count, uint32_t n,
		  const char **bad)
{
	for (size_t i = 0; i < count; i++)
		if (wcount(w, names[i]) != n) {
			*bad = names[i];
			return false;
		}
	return true;
}

static bool counts_are(const struct v3d_job_pack_counts *c, uint32_t emitted, uint32_t skipped,
		       uint32_t compared, uint32_t mismatched)
{
	bool ok = c->emitted == emitted && c->skipped == skipped && c->compared == compared &&
		  c->mismatched == mismatched;
	if (!ok)
		printf("job_check: counts emitted %u skipped %u compared %u mismatched %u, "
		       "want %u %u %u %u\n", c->emitted, c->skipped, c->compared, c->mismatched,
		       emitted, skipped, compared, mismatched);
	return ok;
}

/* Input k of a packet group, alone: the state (and the FS's varying flags,
 * the primitive, and whether the job is 4x) of a draw where it matters,
 * changed when `change` is set. False past the last input. Every input
 * v3d_job.c's PACK() lists; a flag whose group's other words can all be 0
 * (the scissor, blending, the depth test, the stencil back face) is changed
 * where they are, so only the flag tells. */
static bool one_input(int k, bool change, struct v3d_job_state *st, struct v3d_fs_prog *fs,
		      uint32_t *prim, bool *msaa, const char **name)
{
	*st = m5_state();
	*msaa = false;
	*fs = fs_prog();
	*prim = V3D_PRIM_TRIANGLES;
	const bool c = change;
	struct v3d_job_stencil_face face = { V3D_COMPARE_FUNC_EQUAL, V3D_STENCIL_OP_KEEP,
					     V3D_STENCIL_OP_INCR, V3D_STENCIL_OP_REPLACE, 1, 0xff, 0xff };
	st->stencil_front = st->stencil_back = face;
	st->rgb_func = st->alpha_func = V3D_BLEND_MODE_ADD;
	st->rgb_src = st->alpha_src = V3D_BLEND_FACTOR_SRC_ALPHA;
	st->rgb_dst = st->alpha_dst = V3D_BLEND_FACTOR_INV_SRC_ALPHA;
	st->scissor_minx = 4, st->scissor_miny = 2, st->scissor_maxx = W - 4, st->scissor_maxy = H - 2;
	st->offset_scale = 1.0f, st->offset_units = 2.0f, st->offset_clamp = 0.5f;
	st->depth_func = V3D_COMPARE_FUNC_LESS;
	uint32_t *sf[7] = { &st->stencil_front.func, &st->stencil_front.sfail_op,
			    &st->stencil_front.zfail_op, &st->stencil_front.zpass_op,
			    &st->stencil_front.ref, &st->stencil_front.value_mask,
			    &st->stencil_front.write_mask };
	uint32_t *sk[7] = { &st->stencil_back.func, &st->stencil_back.sfail_op,
			    &st->stencil_back.zfail_op, &st->stencil_back.zpass_op,
			    &st->stencil_back.ref, &st->stencil_back.value_mask,
			    &st->stencil_back.write_mask };
	uint32_t *bl[6] = { &st->rgb_func, &st->rgb_src, &st->rgb_dst,
			    &st->alpha_func, &st->alpha_src, &st->alpha_dst };
	static const char *const face_names[7] = { "func", "sfail", "zfail", "zpass", "ref",
						   "value mask", "write mask" };
	static const char *const blend_names[6] = { "rgb func", "rgb src", "rgb dst",
						    "alpha func", "alpha src", "alpha dst" };
	static char buf[48];
	switch (k) {
	case 0: *name = "viewport x scale"; if (c) st->vp.scale[0] *= 0.5f; return true;
	case 1: *name = "viewport y scale"; if (c) st->vp.scale[1] *= 0.5f; return true;
	case 2: *name = "viewport z scale"; if (c) st->vp.scale[2] *= 0.5f; return true;
	case 3: *name = "viewport x offset"; if (c) st->vp.translate[0] += 4.0f; return true;
	case 4: *name = "viewport y offset"; if (c) st->vp.translate[1] += 4.0f; return true;
	case 5: *name = "viewport z offset"; if (c) st->vp.translate[2] += 0.125f; return true;
	case 6:
		/* an empty box at 0: its words are 0 whether it is on or not */
		*name = "scissor on";
		st->scissor_minx = st->scissor_miny = st->scissor_maxx = st->scissor_maxy = 0;
		st->scissor = c;
		return true;
	case 7: *name = "scissor min x"; st->scissor = true; if (c) st->scissor_minx++; return true;
	case 8: *name = "scissor min y"; st->scissor = true; if (c) st->scissor_miny++; return true;
	case 9: *name = "scissor max x"; st->scissor = true; if (c) st->scissor_maxx--; return true;
	case 10: *name = "scissor max y"; st->scissor = true; if (c) st->scissor_maxy--; return true;
	case 11: *name = "primitive (a polygon or not)"; if (c) *prim = V3D_PRIM_LINES; return true;
	case 12: *name = "cull front"; st->cull_front = c; return true;
	case 13: *name = "cull back"; st->cull_back = c; return true;
	case 14: *name = "front ccw"; st->front_ccw = !c; return true;
	case 15: *name = "depth offset on"; st->offset_tri = c; return true;
	case 16: *name = "depth offset scale"; st->offset_tri = true; if (c) st->offset_scale = 2.0f; return true;
	case 17: *name = "depth offset units"; st->offset_tri = true; if (c) st->offset_units = 4.0f; return true;
	case 18: *name = "depth offset clamp"; st->offset_tri = true; if (c) st->offset_clamp = 0.25f; return true;
	case 19: *name = "line width"; if (c) st->line_width = 2.0f; return true;
	case 20:
		/* NEVER is 0 and the write is off: only the flag tells */
		*name = "depth test on";
		st->depth_func = V3D_COMPARE_FUNC_NEVER;
		st->depth_enabled = c;
		return true;
	case 21: *name = "depth write"; st->depth_enabled = true; st->depth_writemask = c; return true;
	case 22: *name = "depth func"; st->depth_enabled = true; if (c) st->depth_func = V3D_COMPARE_FUNC_GEQUAL; return true;
	case 23: *name = "stencil on"; st->stencil_enabled = c; return true;
	case 24:
		/* a back face of all zeros: the back words of the key are 0
		 * whether it is on or not, so only the flag tells */
		*name = "stencil back on";
		st->stencil_enabled = true;
		st->stencil_back = (struct v3d_job_stencil_face){ 0 };
		st->stencil_back_enabled = c;
		return true;
	case 25:
		/* ADD with ZERO factors, all 0: only the flag tells */
		*name = "blend on";
		st->rgb_func = st->rgb_src = st->rgb_dst = 0;
		st->alpha_func = st->alpha_src = st->alpha_dst = 0;
		st->blend_enabled = c;
		return true;
	case 26: *name = "colour mask"; if (c) st->colormask = 0x7; return true;
	case 27: case 28: case 29: case 30:
		*name = "blend colour"; if (c) st->blend_color_f16[k - 27] = 0x3c00; return true;
	case 31: *name = "flat shade flags"; if (c) fs->flat_shade_flags = 1; return true;
	case 32: *name = "non-perspective flags"; if (c) fs->noperspective_flags = 1; return true;
	case 33: *name = "centroid flags"; if (c) fs->centroid_flags = 1; return true;
	case 34 + 7 + 7 + 6:
		/* SAMPLE_STATE's mask: a 4x job writes the draw's */
		*name = "sample mask (a 4x job)";
		*msaa = true;
		st->sample_mask = c ? 0x7 : 0xf;
		return true;
	}
	if (k >= 34 && k < 34 + 7) {
		snprintf(buf, sizeof(buf), "stencil front %s", face_names[k - 34]);
		*name = buf;
		st->stencil_enabled = true;
		if (c)
			*sf[k - 34] ^= 1;
		return true;
	}
	if (k >= 41 && k < 41 + 7) {
		snprintf(buf, sizeof(buf), "stencil back %s", face_names[k - 41]);
		*name = buf;
		st->stencil_enabled = st->stencil_back_enabled = true;
		if (c)
			*sk[k - 41] ^= 1;
		return true;
	}
	if (k >= 48 && k < 48 + 6) {
		snprintf(buf, sizeof(buf), "blend %s", blend_names[k - 48]);
		*name = buf;
		st->blend_enabled = true;
		if (c)
			*bl[k - 48] ^= 1;
		return true;
	}
	return false;
}
#define ONE_INPUTS 55

/* Two draws, the second with input k changed, in a job of its own; the
 * walk of its BCL and its counts. */
static void one_input_job(int k, struct walk *w, struct v3d_job_pack_counts *n)
{
	static struct lists l;
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st;
	struct v3d_fs_prog fs;
	uint32_t prim;
	bool msaa;
	const char *name;
	one_input(k, false, &st, &fs, &prim, &msaa, &name);
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	t.msaa = msaa;
	struct v3d_job j;
	bool ok = true;
	lists_init(&l, RCL_OFF - BCL_OFF);
	ok = v3d_job_begin(&j, &t, &l.bcl, &l.rcl, &l.ind);
	for (int change = 0; change < 2; change++) {
		one_input(k, change, &st, &fs, &prim, &msaa, &name);
		struct v3d_job_draw d = {
			.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
			.attrs = attrs, .num_attrs = 2, .prim = prim, .count = 3,
		};
		ok = ok && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK;
	}
	walk(&l.bcl, w);
	w->ok = w->ok && ok;
	*n = j.pack.n;
}

static void part_pack_skip(void)
{
	static struct lists off, on, l;
	struct v3d_job j;
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC };
	struct v3d_job_pack_counts tot0, tot1;
	uint32_t jobs0, jobs1;
	v3d_job_pack_totals(&tot0, &jobs0);

	/* (a) the knob off writes today's bytes. Set explicitly, not read from
	 * CONFIG_YAKOGL_V3D_PACK_SKIP (default y since batch 94's board
	 * evidence): this host binary never defines that macro, so both arms
	 * below are pinned independently of the Kconfig default. The pin:
	 * pack_seq.h's job built with origin/main 6ee82953's driver/src/v3d
	 * (git archive into a scratch directory, a 20-line program that calls
	 * pack_seq_build on a 0x4000/0x4000/0x18000 BCL/RCL/indirect split and
	 * prints the three lists' FNV-1a 64 in that order): 0x986c4c5405743e7a,
	 * BCL 5985 bytes. The same split here. */
	v3d_job_pack_skip = false;
	check(!v3d_job_pack_skip, "the knob is off (set explicitly for this arm)");
	struct v3d_cl bcl, rcl, ind;
	lists_init(&off, RCL_OFF - BCL_OFF);
	v3d_cl_init(&bcl, off.bo, BO_GPU, 0x4000);
	v3d_cl_init(&rcl, off.bo + 0x4000, BO_GPU + 0x4000, 0x4000);
	v3d_cl_init(&ind, off.bo + 0x8000, BO_GPU + 0x8000, 0x18000);
	check(pack_seq_build(&j, &t, &bcl, &rcl, &ind), "knob off: the 48-draw sequence builds");
	uint64_t h = pack_seq_fnv(&ind, pack_seq_fnv(&rcl, pack_seq_fnv(&bcl, 0xcbf29ce484222325ull)));
	check(h == 0x986c4c5405743e7aull && bcl.used == 5985,
	      "knob off: the lists are origin/main 6ee82953's, byte for byte (pinned hash)");
	struct v3d_job_pack_counts c_off = j.pack.n;
	struct walk w_off;
	walk(&bcl, &w_off);
	check(w_off.ok && c_off.skipped == 0 && c_off.compared == 0 && c_off.mismatched == 0 &&
		      c_off.emitted == w_off.packets - JOB_FRAME_PACKETS,
	      "knob off: every packet counted as emitted, nothing compared or skipped");

	/* (b) the knob on: the same sequence, the same state at every draw */
	struct v3d_cl bcl_on, rcl_on, ind_on;
	lists_init(&on, RCL_OFF - BCL_OFF);
	v3d_cl_init(&bcl_on, on.bo, BO_GPU, 0x4000);
	v3d_cl_init(&rcl_on, on.bo + 0x4000, BO_GPU + 0x4000, 0x4000);
	v3d_cl_init(&ind_on, on.bo + 0x8000, BO_GPU + 0x8000, 0x18000);
	v3d_job_pack_skip = true;
	check(pack_seq_build(&j, &t, &bcl_on, &rcl_on, &ind_on), "knob on: the sequence builds");
	struct v3d_job_pack_counts c_on = j.pack.n;
	check(rcl_on.used == rcl.used && memcmp(rcl_on.buf, rcl.buf, rcl.used) == 0 &&
		      ind_on.used == ind.used && memcmp(ind_on.buf, ind.buf, ind.used) == 0,
	      "knob on: the RCL and the indirect buffer are the knob-off job's");
	struct walk w_on;
	walk(&bcl_on, &w_on);
	check(w_on.ok && bcl_on.used < bcl.used && c_on.emitted == w_on.packets - JOB_FRAME_PACKETS &&
		      c_on.emitted + c_on.skipped == c_off.emitted,
	      "knob on: a shorter BCL; emitted is what it holds, emitted + skipped the knob-off count");
	check(c_on.compared > 0 && c_on.mismatched > 0 && c_on.mismatched < c_on.compared,
	      "knob on: groups compared, some differed, not all");
	check(w_off.prims == PACK_SEQ_DRAWS && w_on.prims == PACK_SEQ_DRAWS,
	      "both arms: one primitive per draw");
	int same = w_off.prims == w_on.prims;
	for (uint32_t i = 0; same && i < w_on.prims && i < WALK_PRIMS; i++)
		same = w_off.state[i] == w_on.state[i];
	check(same, "the state the BCL holds at each of the 48 draws is the same in both arms");
	if (getenv("JOB_CHECK_DUMP"))
		printf("job_check: pack seq: BCL %u -> %u bytes, packets %u -> %u; "
		       "emitted %u skipped %u compared %u mismatched %u\n", bcl.used, bcl_on.used,
		       w_off.packets, w_on.packets, c_on.emitted, c_on.skipped, c_on.compared,
		       c_on.mismatched);
	/* Mesa's decoder reads it (the lists at part 3's split) */
	lists_init(&l, RCL_OFF - BCL_OFF);
	check(pack_seq_build(&j, &t, &l.bcl, &l.rcl, &l.ind), "knob on: the sequence builds again");
	char *text = decode(&l);
	check(text && census(text, "VERTEX_ARRAY_PRIMS") + census(text, "INDEXED_PRIM_LIST") ==
			      PACK_SEQ_DRAWS,
	      "knob on: Mesa's decoder reads the job, every primitive");
	free(text);
	v3d_job_pack_totals(&tot1, &jobs1);
	check(jobs1 == jobs0 + 3 && tot1.emitted == tot0.emitted + c_off.emitted + 2 * c_on.emitted &&
		      tot1.skipped == tot0.skipped + 2 * c_on.skipped &&
		      tot1.compared == tot0.compared + 2 * c_on.compared,
	      "the totals add the three ended jobs' counts");

	/* (c) two identical draws: the state packets once, the draw packets twice */
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct walk w;
	const char *bad = "";
	lists_init(&l, RCL_OFF - BCL_OFF);
	check(v3d_job_begin(&j, &t, &l.bcl, &l.rcl, &l.ind) && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK &&
		      v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK,
	      "knob on: two identical draws go in");
	/* the first: 16 state packets + POINT_SIZE + 3 draw packets; the
	 * second: POINT_SIZE + 3, the 16 skipped, its 9 groups compared */
	check(counts_are(&j.pack.n, 24, 16, 9, 0), "two identical draws: counts 24/16/9/0");
	walk(&l.bcl, &w);
	check(w.ok && all_n(&w, m5_state_packets, 15, 1, &bad) &&
		      wcount(&w, "OCCLUSION_QUERY_COUNTER_ENABLE") == 2 /* the prologue's and the draw's */,
	      "two identical draws: every state packet once");
	if (bad[0]) {
		printf("job_check: %s seen %u times; the walk (ok %d):", bad, wcount(&w, bad), w.ok);
		for (int op = 0; op < 256; op++)
			if (w.count[op])
				printf(" %s(%d)x%u", w.name[op], op, w.count[op]);
		printf("\n");
	}
	check(all_n(&w, draw_packets, 4, 2, &bad), "two identical draws: the draw packets twice");

	/* (d) the depth range alone changes: the viewport group again, alone */
	struct v3d_job_state st3 = st;
	v3d_viewport_gl(0, 0, (float)W, (float)H, 0.25f, 1.0f, true, H, &st3.vp);
	d.st = &st3;
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "a draw with another depth range goes in");
	check(counts_are(&j.pack.n, 24 + 8, 16 + 12, 18, 1),
	      "a new depth range: the 4 viewport packets written, 12 skipped, 1 group of 9 differed");
	walk(&l.bcl, &w);
	check(wcount(&w, "CLIPPER_XY") == 2 && wcount(&w, "CLIPPER_Z") == 2 &&
		      wcount(&w, "CLIPZ") == 2 &&
		      wcount(&w, "VIEWPORT_OFFSET") == 2 && wcount(&w, "CLIP") == 1 &&
		      wcount(&w, "CFG_BITS") == 1 && wcount(&w, "SAMPLE_STATE") == 1,
	      "a new depth range: exactly the viewport group written again");

	/* (e) the viewport's x changes: the clip window and the viewport */
	struct v3d_job_state st4 = st;
	v3d_viewport_gl(8, 0, (float)W - 8, (float)H, 0.25f, 1.0f, true, H, &st4.vp);
	d.st = &st4;
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK &&
		      counts_are(&j.pack.n, 32 + 9, 28 + 11, 27, 3),
	      "a new viewport x: the clip window and the viewport written, 2 groups differed");

	/* (f) a refused draw leaves the last written inputs as they were: blend
	 * turned on in a draw that fails after its state was packed (a
	 * uniform outside the storage), then in one that goes in */
	struct v3d_job_state st5 = st4;
	st5.blend_enabled = true;
	st5.rgb_func = st5.alpha_func = V3D_BLEND_MODE_ADD;
	st5.rgb_src = st5.alpha_src = V3D_BLEND_FACTOR_SRC_ALPHA;
	st5.rgb_dst = st5.alpha_dst = V3D_BLEND_FACTOR_INV_SRC_ALPHA;
	static const struct v3d_uniform bad_index[] = { { V3D_QUNIFORM_UNIFORM, 2 } };
	struct v3d_fs_prog fs_bad = fs;
	fs_bad.uniforms = bad_index;
	fs_bad.num_uniforms = 1;
	struct v3d_job_draw r = d;
	r.st = &st5;
	r.fs = &fs_bad;
	struct v3d_job_pack before = j.pack;
	uint32_t bcl_used = l.bcl.used;
	check(v3d_job_draw(&j, &r, NULL) == V3D_JOB_REFUSED && l.bcl.used == bcl_used &&
		      j.pack.valid == before.valid &&
		      memcmp(j.pack.key, before.key, sizeof(before.key)) == 0 &&
		      counts_are(&j.pack.n, before.n.emitted, before.n.skipped, before.n.compared,
				 before.n.mismatched),
	      "a refused draw: the list, the last inputs and the counts as they were");
	d.st = &st5;
	check(v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK, "the blended draw goes in");
	walk(&l.bcl, &w);
	check(wcount(&w, "BLEND_CFG") == 1 && wcount(&w, "BLEND_ENABLES") == 2 &&
		      wcount(&w, "CFG_BITS") == 2 && counts_are(&j.pack.n, 41 + 7, 39 + 14, 36, 5),
	      "after the refusal, blending is written: BLEND_ENABLES, BLEND_CFG and CFG_BITS");
	struct v3d_job_clear c = { .rgba8 = 0xff202020u, .z = 1.0f };
	check(v3d_job_end(&j, &c, NULL, NULL), "the job ends");

	/* (g) every input of every group, changed alone from one draw to the
	 * next, is written again: the state the BCL holds at the second draw
	 * is the knob-off job's, and one group or more differed */
	int inputs = 0, written = 0;
	for (int k = 0; k < ONE_INPUTS + 1; k++) {
		struct v3d_job_state s0;
		struct v3d_fs_prog f0;
		uint32_t p0;
		bool m0;
		const char *name;
		if (!one_input(k, false, &s0, &f0, &p0, &m0, &name))
			break;
		inputs++;
		struct walk w0, w1;
		struct v3d_job_pack_counts n0, n1;
		v3d_job_pack_skip = false;
		one_input_job(k, &w0, &n0);
		v3d_job_pack_skip = true;
		one_input_job(k, &w1, &n1);
		bool same = w0.ok && w1.ok && w0.prims == 2 && w1.prims == 2 &&
			    w0.state[1] == w1.state[1] && n1.mismatched >= 1;
		written += same;
		if (!same)
			printf("job_check: input \"%s\" changed alone: not written again\n", name);
	}
	check(inputs == ONE_INPUTS, "55 inputs listed");
	check(written == inputs, "every input changed alone is written again (the knob-off state)");

	/* (h) a new job starts from nothing: its first draw writes every group */
	lists_init(&l, RCL_OFF - BCL_OFF);
	d.st = &st;
	check(v3d_job_begin(&j, &t, &l.bcl, &l.rcl, &l.ind) && v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK &&
		      counts_are(&j.pack.n, 20, 0, 0, 0),
	      "a new job's first draw writes every group, compares none");
	v3d_job_pack_skip = false;
}

/* ---- 19. occlusion queries: OCCLUSION_QUERY_COUNTER's address ------------- */

/* A job of four draws whose state differs only in oq_addr (`addrs`), with
 * packet skip `skip`. */
static char *oq_job(struct lists *l, const uint32_t *addrs, int n, bool skip)
{
	struct v3d_fs_prog fs = fs_prog();
	struct v3d_vs_prog vs = vs_prog(0x00201000u, vs_u, 5), cs = vs_prog(0x00202000u, cs_u, 3);
	struct v3d_vertex_attr attrs[2] = { { 0x00300000u, 32, 4 }, { 0x00300010u, 32, 4 } };
	struct v3d_uniform_storage vst = { vs_words, 4 }, fst = { fs_words, 2 };
	struct v3d_job_state st = m5_state();
	struct v3d_job_draw d = {
		.st = &st, .fs = &fs, .vs = &vs, .cs = &cs, .vs_storage = vst, .fs_storage = fst,
		.attrs = attrs, .num_attrs = 2, .prim = V3D_PRIM_TRIANGLES, .count = 3,
	};
	struct v3d_job_target t = { W, H, TARGET, W * 4, TILE_ALLOC, false };
	struct v3d_job_clear c = { 0 };
	bool was = v3d_job_pack_skip;
	v3d_job_pack_skip = skip;
	lists_init(l, RCL_OFF - BCL_OFF);
	struct v3d_job j;
	bool ok = v3d_job_begin(&j, &t, &l->bcl, &l->rcl, &l->ind);
	for (int i = 0; ok && i < n; i++) {
		st.oq_addr = addrs[i];
		ok = v3d_job_draw(&j, &d, NULL) == V3D_JOB_OK;
	}
	ok = ok && v3d_job_end(&j, &c, NULL, NULL);
	v3d_job_pack_skip = was;
	check(ok, skip ? "occlusion: the four-draw job builds (packet skip on)"
		       : "occlusion: the four-draw job builds (packet skip off)");
	return decode(l);
}

/* The address of the nth OCCLUSION_QUERY_COUNTER_ENABLE packet as the
 * decoder prints it: "[null]" for 0, and for an address in no registered
 * BO a comment ("XXX: BO unknown") then 0x...; -1 when the packet is
 * absent. */
static long oq_address(const char *text, int nth)
{
	const char *name = "OCCLUSION_QUERY_COUNTER_ENABLE\n";
	const char *p = text;
	for (int i = 0; p && i <= nth; i++) {
		p = strstr(p, name);
		if (p && i < nth)
			p += strlen(name);
	}
	if (!p)
		return -1;
	p += strlen(name);
	const char *nl = strchr(p, '\n');
	size_t len = nl ? (size_t)(nl - p) : strlen(p);
	char line[128];
	snprintf(line, sizeof(line), "%.*s", (int)len, p);
	if (!strstr(line, "address:"))
		return -1;
	if (strstr(line, "[null]"))
		return 0;
	const char *hex = strrchr(line, 'x');
	return hex && hex > line ? strtol(hex - 1, NULL, 16) : -1;
}

/* Mesa's v3dx_emit.c:720-726: the counter's address while a query is active,
 * the zero-address packet otherwise; the job's prologue disables it
 * (v3dx_draw.c:107-108). With packet skip the address is part of the FIXED
 * group's key: a draw with the same address writes nothing, another
 * address (0 included) rewrites the group. */
static void part_occlusion(void)
{
	static struct lists l;
	static const uint32_t addrs[4] = { 0x00400040u, 0x00400040u, 0, 0x00400080u };
	const char *oq = "OCCLUSION_QUERY_COUNTER_ENABLE";
	char *text = oq_job(&l, addrs, 4, false);
	check(text && census(text, oq) == 5, "occlusion, skip off: the prologue's packet and one per draw");
	check(text && oq_address(text, 0) == 0 && oq_address(text, 1) == 0x400040 &&
		      oq_address(text, 2) == 0x400040 && oq_address(text, 3) == 0 &&
		      oq_address(text, 4) == 0x400080,
	      "occlusion, skip off: 0 (prologue), the counter twice, 0, the other counter");
	free(text);
	text = oq_job(&l, addrs, 4, true);
	check(text && census(text, oq) == 4, "occlusion, skip on: the same address twice is written once");
	check(text && oq_address(text, 0) == 0 && oq_address(text, 1) == 0x400040 &&
		      oq_address(text, 2) == 0 && oq_address(text, 3) == 0x400080,
	      "occlusion, skip on: 0 (prologue), the counter, 0, the other counter");
	free(text);
}

int main(void)
{
	/* part 1: 14 checks; part 2: 11; part 3: 6 + 51 census + 2 + 47 fields;
	 * part 4: 3 + 3 + 19 fields; part 5: 2 + 5 words + 2 refusals + 3 per
	 * stage + 1 + 1 + 4 size words + 2 size refusals; part 6: 2 + 1 + 2 + 2 + 4 point-coord + 2 point-coord
	 * refusals + 4 refusals + 1 + 3 (the M5 frame);
	 * part 7: 3 builds + 3 + 5 load fields + 1 + 5 store fields + 1 + 2 +
	 * 3 builds + 8 apart;
	 * part 8: 4 builds + 9; part 9: 3 builds + 7; part 10: 3 builds + 5;
	 * part 11: 2 builds + 2 fields + 1 + 2 refusals;
	 * part 12 (multiple render targets): 3 builds + 12;
	 * part 13 (instanced draws): 3 builds + 8;
	 * part 14 (4x multisampling): 14;
	 * part 15 (packet skip): 4 knob off + 9 knob on + 4 identical draws +
	 * 3 depth range + 1 viewport x + 4 refusal + 2 one input + 1 new job;
	 * part 16 (S8 stencil): 1 build + 7;
	 * part 17 (a view's stride): 2 builds + 1 decode + 3 strides;
	 * part 18 (no colour buffer): 2 builds + 4 + 3 + 1 refusal;
	 * part 19 (occlusion queries): 2 builds + 4;
	 * part 20 (the render image): 1 build + 5 + 3 refusals */
	const int planned = 14 + 11 + 6 + 51 + 2 + 47 + 3 + 3 + 19 + 20 + 21 + 31 + 13 + 10 + 8 + 7 + 15 + 11 + 14 +
			    28 + 8 + 6 + 10 + 6 + 9;
	part_identity();
	part_rollback();
	part_decode();
	part_clear_quad();
	part_tmu();
	part_wpos();
	part_zs();
	part_s8();
	part_view_stride();
	part_no_colour();
	part_rt_alpha();
	part_psiz();
	part_discard();
	part_pntc();
	part_mrt();
	part_instanced();
	part_msaa();
	part_rt_image();
	part_pack_skip();
	part_occlusion();
	printf("job_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
