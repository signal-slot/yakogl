/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tfu71_check: driver/src/v3d/v3d_tfu71.c's V3D 7.1 TFU job against Mesa
 * 26.2.2's own 7.1 fill, in two ways (tools/v3d/check71.d/tfu71.sh):
 *
 *  - always: against the committed table tools/v3d/tfu71_golden.tsv, which
 *    holds Mesa's answers (constants, the supported data types, the twelve
 *    register words and the decision for every input below except the
 *    sweep's tail). The table is Mesa's output, written by this program's
 *    --emit when it is built with the Mesa half;
 *  - with -DTFU71_MESA (when a Mesa 26.2.2 tree is present): against the
 *    Mesa half itself, mesa_tfu71.c, which tfu71.sh makes from the lines of
 *    Mesa's src/gallium/drivers/v3d/v3dx_tfu.c that compute the job (the
 *    IOS initializer, :98; IIA, IOA and IIS, :106-127; the V3D_VERSION >= 71
 *    branch, :165-202; the declines at :64-66 and :88-91), copied verbatim
 *    with a small shim for the Gallium resource and compiled at V3D_VERSION
 *    71 with Mesa's own v3d_tfu.h, v3d_drm.h, enum v3d_tiling_mode
 *    (v3d_tiling.h:42-64), v3d_utile_height (v3d_tiling.c:55-70),
 *    v3dX(tfu_supports_tex_format) (v3dx_format_table.c:343-378) and the
 *    generated 7.1 Texture Data Formats; tfu71.sh then also requires the
 *    committed table to equal a fresh --emit byte for byte.
 *
 * Checks: the constants (field shifts and codes, tiling order, data types,
 * the maximum size) equal Mesa's; v3d_tfu71_supports equals Mesa's for every
 * data type 0..127, with and without mipmaps; the named cases (RGBA8 raster
 * to UIF with and without mipmaps, the mipmap generation of a UIF image,
 * R8 and RGB565, a raster destination, LINEARTILE and UB-linear sides, the
 * declines) and a seeded sweep of valid inputs (its first TABLE_SWEEP in the
 * table, all SWEEP live) give Mesa's twelve register words and Mesa's
 * decision; the inputs our packer refuses on its own (a value that would not
 * fit or not describe the image) are refused with the job untouched. One
 * check per comparison; the last line is "tfu71: N of N checks, 0 failed"
 * (exit 0) or the count of failures (1).
 *
 * Also, against Mesa 7.1's own glGenerateMipmap (tools/v3d/pi5/gl_mipgen.stdout,
 * made with drm-shim): the TFU job's register words are read from its
 * SUBMIT_TFU line, v3d_tfu71's inputs are filled from gl_mipgen's GL setup
 * (glTexStorage2D 7 levels, GL_RGBA8, 64x64: RGBA8, cpp 4, 64x64, 6 levels
 * after the base, source = destination = the texture's UIF level 0) and all
 * twelve words are compared. Addresses: drm-shim's BO offsets (0x11000, 4 KiB
 * aligned, where the board's are 16 KiB) are the check's input addresses, so
 * IIA and IOA are compared as an echo of the input, not as an answer. The
 * tiling, UIF_NO_XOR, is Mesa's choice for that buffer (read from the
 * reference's ICFG IFORMAT and IOC FORMAT, which must agree); the padded
 * height 64 rows is what a 64-row UIF level at 4 cpp has (IIS 8 UIF blocks,
 * checked). The shim runs no TFU job, so only values are compared.
 *
 * Usage: tfu71_check TABLE [GL_MIPGEN_STDOUT]   (both builds)
 *        tfu71_check --emit OUT       (the -DTFU71_MESA build: write the table)
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_tfu71.h"

/* One input, in the Mesa half's terms. */
struct tfu71_in {
	uint32_t ttype, cpp, width, height, num_mm;
	uint32_t src_addr, src_tiling, src_sp;
	uint32_t dst_addr, dst_tiling, dst_sp;
};

/* Mesa's answer for one input: the decision and, when it packs, the words
 * ICFG, IIA, IIS, ICA, IUA, IOA, IOS, COEF0..3, IOC. */
struct tfu71_ref {
	int ok;
	uint32_t w[12];
};

#define NCONSTS 20
static const char *const const_names[NCONSTS] = {
	"V3D71_TFU_IOC_DIMTW",
	"V3D71_TFU_IOC_FORMAT_SHIFT",
	"V3D71_TFU_IOC_FORMAT_RASTER",
	"V3D71_TFU_IOC_FORMAT_LINEARTILE",
	"V3D71_TFU_IOC_STRIDE_SHIFT",
	"V3D71_TFU_IOC_NUMMM_SHIFT",
	"V3D71_TFU_ICFG_OTYPE_SHIFT",
	"V3D71_TFU_ICFG_IFORMAT_SHIFT",
	"V3D71_TFU_ICFG_FORMAT_RASTER",
	"V3D71_TFU_ICFG_FORMAT_LINEARTILE",
	"V3D_TFU_MAX_DIM",
	"TEXTURE_DATA_FORMAT_R8",
	"TEXTURE_DATA_FORMAT_RGBA8",
	"TEXTURE_DATA_FORMAT_RGB565",
	"V3D_TILING_RASTER",
	"V3D_TILING_LINEARTILE",
	"V3D_TILING_UBLINEAR_1_COLUMN",
	"V3D_TILING_UBLINEAR_2_COLUMN",
	"V3D_TILING_UIF_NO_XOR",
	"V3D_TILING_UIF_XOR",
};
static const uint32_t our_consts[NCONSTS] = {
	V3D_TFU71_IOC_DIMTW,
	V3D_TFU71_IOC_FORMAT_SHIFT,
	V3D_TFU71_IOC_FORMAT_RASTER,
	V3D_TFU71_IOC_FORMAT_LINEARTILE,
	V3D_TFU71_IOC_STRIDE_SHIFT,
	V3D_TFU71_IOC_NUMMM_SHIFT,
	V3D_TFU71_ICFG_OTYPE_SHIFT,
	V3D_TFU71_ICFG_IFORMAT_SHIFT,
	V3D_TFU71_ICFG_FORMAT_RASTER,
	V3D_TFU71_ICFG_FORMAT_LINEARTILE,
	V3D_TFU71_MAX_DIM,
	V3D_TFU71_TTYPE_R8,
	V3D_TFU71_TTYPE_RGBA8,
	V3D_TFU71_TTYPE_RGB565,
	V3D_TFU71_TILING_RASTER,
	V3D_TFU71_TILING_LINEARTILE,
	V3D_TFU71_TILING_UBLINEAR_1_COLUMN,
	V3D_TFU71_TILING_UBLINEAR_2_COLUMN,
	V3D_TFU71_TILING_UIF_NO_XOR,
	V3D_TFU71_TILING_UIF_XOR,
};

#ifdef TFU71_MESA
/* mesa_tfu71.c (built by tfu71.sh from the Mesa tree) */
struct mesa_tfu71_out {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef[4], ioc;
};
int mesa_tfu71(const struct tfu71_in *in, struct mesa_tfu71_out *out);
int mesa_tfu71_supports(uint32_t ttype, int for_mipmap);
/* Mesa's constants, in the order of const_names */
extern const uint32_t mesa_tfu71_consts[];
extern const char *const mesa_tfu71_const_names[];
extern const unsigned mesa_tfu71_nconsts;

static struct tfu71_ref mesa_ref(const struct tfu71_in *in)
{
	struct mesa_tfu71_out o;
	struct tfu71_ref r;
	memset(&o, 0, sizeof(o));
	memset(&r, 0, sizeof(r));
	r.ok = mesa_tfu71(in, &o) != 0;
	if (r.ok) {
		const uint32_t v[12] = { o.icfg, o.iia,     o.iis,     o.ica,     o.iua,     o.ioa,
					 o.ios,  o.coef[0], o.coef[1], o.coef[2], o.coef[3], o.ioc };
		memcpy(r.w, v, sizeof(v));
	}
	return r;
}
#endif

static unsigned checks, failed;

static void check(int ok, const char *what)
{
	checks++;
	if (!ok) {
		failed++;
		if (failed <= 30)
			printf("FAIL: %s\n", what);
	}
}

static struct v3d_tfu71_args to_args(const struct tfu71_in *in)
{
	struct v3d_tfu71_args a = {
		.ttype = in->ttype,
		.cpp = in->cpp,
		.width = in->width,
		.height = in->height,
		.num_mm = in->num_mm,
		.src = { in->src_addr, (enum v3d_tfu71_tiling)in->src_tiling, in->src_sp },
		.dst = { in->dst_addr, (enum v3d_tfu71_tiling)in->dst_tiling, in->dst_sp },
	};
	return a;
}

static const char *const word_names[12] = { "icfg", "iia",   "iis",   "ica",   "iua",   "ioa",
					     "ios",  "coef0", "coef1", "coef2", "coef3", "ioc" };

/* One input against one reference answer (`who`: "Mesa" or "table"): the
 * decision, and when both packed, the twelve words. `verbose` prints the
 * words (the named cases). */
static void compare(const char *name, const char *who, const struct tfu71_in *in,
		    const struct tfu71_ref *r, int verbose)
{
	char what[320];
	const struct v3d_tfu71_args a = to_args(in);
	struct v3d_tfu71_job j;
	memset(&j, 0xa5, sizeof(j));
	const int ook = v3d_tfu71_pack(&a, &j);
	snprintf(what, sizeof(what), "%s: decision ours %d, %s %d", name, ook, who, r->ok);
	check(ook == r->ok, what);
	if (!ook || !r->ok) {
		if (verbose)
			printf("  %-34s declined (ours and %s)\n", name, who);
		return;
	}
	const uint32_t ow[12] = { j.icfg, j.iia,     j.iis,     j.ica,     j.iua,     j.ioa,
				  j.ios,  j.coef[0], j.coef[1], j.coef[2], j.coef[3], j.ioc };
	for (int i = 0; i < 12; i++) {
		snprintf(what, sizeof(what), "%s: %s ours 0x%08x, %s 0x%08x", name, word_names[i], ow[i],
			 who, r->w[i]);
		check(ow[i] == r->w[i], what);
	}
	if (verbose)
		printf("  %-34s icfg %08x iia %08x iis %08x ioa %08x ios %08x ioc %08x\n", name, ow[0], ow[1],
		       ow[2], ow[5], ow[6], ow[11]);
}

/* An input only ours refuses: refused, `j` untouched. */
static void refused(const char *name, const struct tfu71_in *in)
{
	char what[256];
	const struct v3d_tfu71_args a = to_args(in);
	struct v3d_tfu71_job j, before;
	memset(&j, 0x5a, sizeof(j));
	before = j;
	const int ook = v3d_tfu71_pack(&a, &j);
	snprintf(what, sizeof(what), "%s: refused", name);
	check(!ook, what);
	snprintf(what, sizeof(what), "%s: job untouched", name);
	check(memcmp(&j, &before, sizeof(j)) == 0, what);
}

static uint32_t uif_h(uint32_t cpp)
{
	/* 2 * v3d_utile_height (v3d_tiling.c:55-70) */
	return cpp == 1 ? 16u : (cpp <= 4 ? 8u : 4u);
}

static uint32_t align_up(uint32_t v, uint32_t a)
{
	return (v + a - 1u) / a * a;
}

enum { RA = 0, LT = 1, UB1 = 2, UB2 = 3, UIFN = 4, UIFX = 5 };

#define S 0x10000000u
#define D 0x20000000u
static const struct {
	const char *name;
	struct tfu71_in in;
} named[] = {
	/* RGBA8 raster to UIF: the texture upload */
	{ "rgba8 raster->uif_xor 64x64", { 4, 4, 64, 64, 0, S, RA, 256, D, UIFX, 64 } },
	{ "rgba8 raster->uif_xor 64x64 mm6", { 4, 4, 64, 64, 6, S, RA, 256, D, UIFX, 64 } },
	{ "rgba8 raster->uif_xor 100x60 pad", { 4, 4, 100, 60, 0, S, RA, 512, D, UIFX, 72 } },
	{ "rgba8 raster->uif_noxor 256x128", { 4, 4, 256, 128, 0, S, RA, 1024, D, UIFN, 128 } },
	{ "rgba8 raster->uif_xor 256x256 mm8", { 4, 4, 256, 256, 8, S, RA, 1024, D, UIFX, 256 } },
	/* the mipmap generation of a UIF image (src == dst, v3d_generate_mipmap) */
	{ "rgba8 uif_xor mipgen 256x256 mm8", { 4, 4, 256, 256, 8, D, UIFX, 256, D, UIFX, 256 } },
	{ "rgba8 uif_xor mipgen 128x32 mm7", { 4, 4, 128, 32, 7, D, UIFX, 32, D, UIFX, 32 } },
	{ "rgba8 ub2 mipgen 32x32 mm5", { 4, 4, 32, 32, 5, D, UB2, 0, D, UB2, 0 } },
	/* R8 and RGB565 */
	{ "r8 raster->uif_xor 100x37", { 0, 1, 100, 37, 0, S, RA, 128, D, UIFX, 48 } },
	{ "r8 raster->uif_xor 128x128 mm7", { 0, 1, 128, 128, 7, S, RA, 128, D, UIFX, 128 } },
	{ "r8 raster->uif_noxor 64x16", { 0, 1, 64, 16, 0, S, RA, 64, D, UIFN, 16 } },
	{ "rgb565 raster->uif_xor 33x17", { 6, 2, 33, 17, 0, S, RA, 66, D, UIFX, 24 } },
	{ "rgb565 raster->uif_xor 64x64 mm6", { 6, 2, 64, 64, 6, S, RA, 128, D, UIFX, 64 } },
	/* other tilings, a raster destination (7.1 has one) */
	{ "rgba8 raster->lineartile 4x4", { 4, 4, 4, 4, 0, S, RA, 16, D, LT, 0 } },
	{ "rgba8 raster->ublinear1 8x8", { 4, 4, 8, 8, 0, S, RA, 32, D, UB1, 0 } },
	{ "rgba8 lt->ublinear2 16x16", { 4, 4, 16, 16, 0, S, LT, 0, D, UB2, 0 } },
	{ "rgba8 uif_xor->raster 64x64", { 4, 4, 64, 64, 0, S, UIFX, 64, D, RA, 320 } },
	{ "rgba8 raster->raster 17x3", { 4, 4, 17, 3, 0, S, RA, 68, D, RA, 128 } },
	{ "r32f raster->uif_xor 64x64", { 29, 4, 64, 64, 0, S, RA, 256, D, UIFX, 64 } },
	{ "rgba16f raster->uif_xor 32x32 mm5", { 18, 8, 32, 32, 5, S, RA, 256, D, UIFX, 32 } },
	{ "rgba32f raster->uif_xor 16x16", { 31, 16, 16, 16, 0, S, RA, 256, D, UIFX, 16 } },
	{ "rgba8 16384x1 raster->raster", { 4, 4, 16384, 1, 0, S, RA, 65536, D, RA, 65536 } },
	/* Mesa's declines */
	{ "rgba32f mipmaps (declined)", { 31, 16, 16, 16, 4, S, RA, 256, D, UIFX, 16 } },
	{ "r32f mipmaps (declined)", { 29, 4, 16, 16, 4, S, RA, 64, D, UIFX, 16 } },
	{ "depth comp16 (declined)", { 21, 2, 16, 16, 0, S, RA, 32, D, UIFX, 16 } },
	{ "etc2 (declined)", { 32, 8, 16, 16, 0, S, RA, 128, D, UIFX, 16 } },
	{ "rgba8 mipmaps to raster (declined)", { 4, 4, 16, 16, 4, S, RA, 64, D, RA, 64 } },
};
#define NNAMED (sizeof(named) / sizeof(named[0]))

/* The seeded sweep: SWEEP valid inputs, the first TABLE_SWEEP of them in
 * the committed table. */
#define SWEEP 20000u
#define TABLE_SWEEP 1000u
#define NINPUTS (NNAMED + SWEEP)

static uint32_t rng_state = 0x7f4a7c15u;

static uint32_t rnd(uint32_t n)
{
	rng_state = rng_state * 1664525u + 1013904223u;
	return (uint32_t)(((uint64_t)(rng_state >> 8) * n) >> 24);
}

/* The next valid sweep input (the draws of the earlier check, with the
 * inputs whose raster destination stride would not fit 16 bits skipped). */
static struct tfu71_in sweep_next(void)
{
	static const uint32_t cpps[] = { 1, 2, 4, 8, 16 };
	static const uint32_t sizes[] = { 1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 100, 127, 128,
					  129, 255, 256, 257, 640, 1000, 1024, 1920, 2048, 4095, 4096, 8191,
					  8192, 16383, 16384 };
	const unsigned nsizes = sizeof(sizes) / sizeof(sizes[0]);
	for (;;) {
		struct tfu71_in in;
		in.ttype = rnd(40);
		in.cpp = cpps[rnd(5)];
		in.width = sizes[rnd(nsizes)];
		in.height = sizes[rnd(nsizes)];
		uint32_t m = in.width > in.height ? in.width : in.height, levels = 0;
		while (m > 1) {
			m >>= 1;
			levels++;
		}
		in.num_mm = rnd(3) == 0 ? 0 : rnd(levels + 1);
		in.src_addr = (rnd(0x10000) << 12) | (rnd(64) << 6);
		in.dst_addr = (rnd(0x10000) << 12) | (rnd(64) << 6);
		in.src_tiling = rnd(6);
		in.dst_tiling = rnd(6);
		const uint32_t sides[2] = { in.src_tiling, in.dst_tiling };
		uint32_t sp[2];
		for (int s = 0; s < 2; s++) {
			if (sides[s] == RA) {
				sp[s] = (in.width + rnd(5) * rnd(65)) * in.cpp;
				if (s == 1 && sp[s] / in.cpp > 0xffffu)
					sp[s] = in.width * in.cpp;
			} else if (sides[s] >= UIFN) {
				sp[s] = align_up(in.height, uif_h(in.cpp)) + rnd(4) * uif_h(in.cpp);
			} else {
				sp[s] = rnd(2) ? 0 : rnd(1u << 16);
			}
		}
		in.src_sp = sp[0];
		in.dst_sp = sp[1];
		if (in.dst_tiling == RA && in.dst_sp / in.cpp > 0xffffu)
			continue;
		return in;
	}
}

static struct tfu71_in inputs[NINPUTS];

static void make_inputs(void)
{
	for (unsigned i = 0; i < NNAMED; i++)
		inputs[i] = named[i].in;
	for (unsigned i = 0; i < SWEEP; i++)
		inputs[NNAMED + i] = sweep_next();
}

static void input_name(unsigned i, char *buf, size_t n)
{
	if (i < NNAMED) {
		snprintf(buf, n, "%s", named[i].name);
		return;
	}
	const struct tfu71_in *in = &inputs[i];
	snprintf(buf, n, "sweep %u (ttype %u cpp %u %ux%u mm %u src %u/%u dst %u/%u)", i - (unsigned)NNAMED,
		 in->ttype, in->cpp, in->width, in->height, in->num_mm, in->src_tiling, in->src_sp,
		 in->dst_tiling, in->dst_sp);
}

/* ---- the committed table ---------------------------------------------- */

#define TABLE_CASES (NNAMED + TABLE_SWEEP)

struct table {
	uint32_t consts[NCONSTS];
	int have_consts[NCONSTS];
	int supports[2][128]; /* -1: absent */
	struct tfu71_in in[TABLE_CASES];
	struct tfu71_ref ref[TABLE_CASES];
	unsigned ncases;
};

static int read_table(const char *path, struct table *t)
{
	FILE *f = fopen(path, "r");
	if (!f) {
		printf("tfu71: cannot open the table %s\n", path);
		return 0;
	}
	memset(t, 0, sizeof(*t));
	for (int m = 0; m < 2; m++)
		for (int i = 0; i < 128; i++)
			t->supports[m][i] = -1;
	char line[512];
	unsigned lineno = 0;
	int ok = 1;
	while (fgets(line, sizeof(line), f)) {
		lineno++;
		if (line[0] == '#' || line[0] == '\n')
			continue;
		char name[80];
		uint32_t v, m;
		char bits[160];
		struct tfu71_in in;
		struct tfu71_ref r;
		memset(&r, 0, sizeof(r));
		if (sscanf(line, "const %79s %" SCNu32, name, &v) == 2) {
			int found = 0;
			for (int i = 0; i < NCONSTS; i++)
				if (strcmp(name, const_names[i]) == 0) {
					t->consts[i] = v;
					t->have_consts[i] = 1;
					found = 1;
				}
			if (!found) {
				printf("tfu71: %s:%u: unknown constant %s\n", path, lineno, name);
				ok = 0;
			}
		} else if (sscanf(line, "supports %" SCNu32 " %159s", &m, bits) == 2) {
			if (m > 1 || strlen(bits) != 128) {
				printf("tfu71: %s:%u: bad supports line\n", path, lineno);
				ok = 0;
				continue;
			}
			for (int i = 0; i < 128; i++)
				t->supports[m][i] = bits[i] == '1';
		} else if (strncmp(line, "case ", 5) == 0) {
			int okv;
			int n = sscanf(line + 5,
				       "%" SCNu32 " %" SCNu32 " %" SCNu32 " %" SCNu32 " %" SCNu32 " %" SCNx32
				       " %" SCNu32 " %" SCNu32 " %" SCNx32 " %" SCNu32 " %" SCNu32 " %d"
				       " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32
				       " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32 " %" SCNx32,
				       &in.ttype, &in.cpp, &in.width, &in.height, &in.num_mm, &in.src_addr,
				       &in.src_tiling, &in.src_sp, &in.dst_addr, &in.dst_tiling, &in.dst_sp, &okv,
				       &r.w[0], &r.w[1], &r.w[2], &r.w[3], &r.w[4], &r.w[5], &r.w[6], &r.w[7],
				       &r.w[8], &r.w[9], &r.w[10], &r.w[11]);
			r.ok = okv;
			if (!((n == 12 && okv == 0) || (n == 24 && okv == 1)) || t->ncases >= TABLE_CASES) {
				printf("tfu71: %s:%u: bad case line\n", path, lineno);
				ok = 0;
				continue;
			}
			t->in[t->ncases] = in;
			t->ref[t->ncases] = r;
			t->ncases++;
		} else {
			printf("tfu71: %s:%u: unknown line\n", path, lineno);
			ok = 0;
		}
	}
	fclose(f);
	return ok;
}

static void check_table(const struct table *t)
{
	char what[320];
	for (int i = 0; i < NCONSTS; i++) {
		snprintf(what, sizeof(what), "constant %s: ours %u, table %s%u", const_names[i], our_consts[i],
			 t->have_consts[i] ? "" : "(absent) ", t->consts[i]);
		check(t->have_consts[i] && our_consts[i] == t->consts[i], what);
	}
	for (int m = 0; m < 2; m++)
		for (uint32_t ty = 0; ty < 128; ty++) {
			snprintf(what, sizeof(what), "supports(%u, mipmap %d): ours %d, table %d", ty, m,
				 (int)v3d_tfu71_supports(ty, m), t->supports[m][ty]);
			check((int)v3d_tfu71_supports(ty, m) == t->supports[m][ty], what);
		}
	snprintf(what, sizeof(what), "the table has %u cases, the check %u", t->ncases, (unsigned)TABLE_CASES);
	check(t->ncases == TABLE_CASES, what);
	for (unsigned i = 0; i < t->ncases && i < TABLE_CASES; i++) {
		char name[200];
		input_name(i, name, sizeof(name));
		snprintf(what, sizeof(what), "%s: the table's input is this check's input", name);
		check(memcmp(&t->in[i], &inputs[i], sizeof(inputs[i])) == 0, what);
		/* the named cases are compared (and printed) by main */
		if (i >= NNAMED)
			compare(name, "table", &inputs[i], &t->ref[i], 0);
	}
}

#ifdef TFU71_MESA
static int emit(const char *path)
{
	FILE *f = fopen(path, "w");
	if (!f) {
		printf("tfu71: cannot write %s\n", path);
		return 1;
	}
	fprintf(f, "# SPDX-License-Identifier: Apache-2.0\n"
		   "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n"
		   "# tools/v3d/tfu71_golden.tsv: Mesa 26.2.2's own V3D 7.1 TFU fill (v3dx_tfu.c at\n"
		   "# V3D_VERSION 71) on the inputs of tools/v3d/tfu71_check.c.\n"
		   "# Generated by tools/v3d/check71.d/tfu71.sh --update; do not edit.\n"
		   "# const NAME VALUE: Mesa's constant (v3d_tfu.h, v3d_tiling.h, the 7.1 data types)\n"
		   "# supports M BITS: v3dX(tfu_supports_tex_format)(type, for_mipmap M), types 0..127\n"
		   "# case TTYPE CPP W H NUM_MM SRC_ADDR SRC_TILING SRC_SP DST_ADDR DST_TILING DST_SP OK\n"
		   "#      [ICFG IIA IIS ICA IUA IOA IOS COEF0 COEF1 COEF2 COEF3 IOC]: one input, Mesa's\n"
		   "#      decision and (OK 1) its words; the %u named cases, then the sweep's first %u\n",
		(unsigned)NNAMED, (unsigned)TABLE_SWEEP);
	for (unsigned i = 0; i < mesa_tfu71_nconsts; i++)
		fprintf(f, "const %s %u\n", mesa_tfu71_const_names[i], mesa_tfu71_consts[i]);
	for (int m = 0; m < 2; m++) {
		fprintf(f, "supports %d ", m);
		for (uint32_t ty = 0; ty < 128; ty++)
			fputc(mesa_tfu71_supports(ty, m) ? '1' : '0', f);
		fputc('\n', f);
	}
	for (unsigned i = 0; i < TABLE_CASES; i++) {
		const struct tfu71_in *in = &inputs[i];
		const struct tfu71_ref r = mesa_ref(in);
		fprintf(f, "case %u %u %u %u %u %08x %u %u %08x %u %u %d", in->ttype, in->cpp, in->width,
			in->height, in->num_mm, in->src_addr, in->src_tiling, in->src_sp, in->dst_addr,
			in->dst_tiling, in->dst_sp, r.ok);
		if (r.ok)
			for (int k = 0; k < 12; k++)
				fprintf(f, " %08x", r.w[k]);
		fputc('\n', f);
	}
	return fclose(f) == 0 ? 0 : 1;
}
#endif

/* gl_mipgen's SUBMIT_TFU line: icfg iia iis ica iua ioa ios coef0..3 ioc */
static int read_mipgen_ref(const char *path, uint32_t w[12])
{
	FILE *f = fopen(path, "r");
	char line[512];
	int found = 0;
	if (!f) {
		printf("tfu71: cannot open the reference %s\n", path);
		return 0;
	}
	while (fgets(line, sizeof(line), f)) {
		const char *p = strstr(line, "SUBMIT_TFU icfg ");
		if (!p)
			continue;
		found = sscanf(p,
			       "SUBMIT_TFU icfg %" SCNx32 " iia %" SCNx32 " iis %" SCNx32 " ica %" SCNx32
			       " iua %" SCNx32 " ioa %" SCNx32 " ios %" SCNx32 " coef %" SCNx32 " %" SCNx32
			       " %" SCNx32 " %" SCNx32 " ioc %" SCNx32,
			       &w[0], &w[1], &w[2], &w[3], &w[4], &w[5], &w[6], &w[7], &w[8], &w[9], &w[10],
			       &w[11]) == 12;
		break;
	}
	fclose(f);
	if (!found)
		printf("tfu71: no SUBMIT_TFU line in %s\n", path);
	return found;
}

/* Our job for gl_mipgen's GL setup against the words Mesa 7.1 wrote. */
static void check_mipgen(const char *path)
{
	char what[320];
	uint32_t w[12];
	if (!read_mipgen_ref(path, w)) {
		check(0, "gl_mipgen's SUBMIT_TFU line reads");
		return;
	}
	check(1, "gl_mipgen's SUBMIT_TFU line reads");
	/* the reference's own tiling, UIF_NO_XOR, two ways: ICFG and IOC */
	const uint32_t ifmt = w[0] >> V3D_TFU71_ICFG_IFORMAT_SHIFT,
		       ofmt = (w[11] >> V3D_TFU71_IOC_FORMAT_SHIFT) & 0xf;
	const uint32_t tiling = V3D_TFU71_TILING_UIF_NO_XOR;
	const uint32_t code = V3D_TFU71_ICFG_FORMAT_LINEARTILE + (tiling - V3D_TFU71_TILING_LINEARTILE);
	check(ifmt == code && ofmt == code, "gl_mipgen: ICFG IFORMAT and IOC FORMAT are UIF_NO_XOR's code");
	/* GL setup -> inputs: RGBA8, cpp 4, 64x64, 7 levels = 6 after the base,
	 * source = destination (v3d_generate_mipmap), UIF with 64 rows; the
	 * address is drm-shim's, taken from the reference */
	const struct tfu71_in in = { V3D_TFU71_TTYPE_RGBA8, 4, 64, 64, 6, w[1], tiling, 64, w[5], tiling, 64 };
	const struct v3d_tfu71_args a = to_args(&in);
	struct v3d_tfu71_job j;
	memset(&j, 0xa5, sizeof(j));
	if (!v3d_tfu71_pack(&a, &j)) {
		check(0, "gl_mipgen: our job packs");
		return;
	}
	const uint32_t ow[12] = { j.icfg, j.iia,     j.iis,     j.ica,     j.iua,     j.ioa,
				  j.ios,  j.coef[0], j.coef[1], j.coef[2], j.coef[3], j.ioc };
	printf("gl_mipgen's TFU job, ours against Mesa 7.1 (%s), word by word:\n", path);
	for (int i = 0; i < 12; i++) {
		const int addr = i == 1 || i == 5;
		printf("  %-6s ours 0x%08x Mesa 0x%08x %s%s\n", word_names[i], ow[i], w[i],
		       ow[i] == w[i] ? "match" : "DIFFERENT",
		       addr ? " (an address: drm-shim's BO offset, fed from the reference as the input)" : "");
		snprintf(what, sizeof(what), "gl_mipgen: %s ours 0x%08x, Mesa 0x%08x", word_names[i], ow[i], w[i]);
		check(ow[i] == w[i], what);
	}
	check(ow[11] == 0x00086061u, "gl_mipgen: IOC is 0x00086061 (DIMTW 1, NUMMM 6, FORMAT 6, STRIDE 8)");
	printf("  IOC 0x%08x: DIMTW %u NUMMM %u FORMAT %u STRIDE %u\n", ow[11], ow[11] & 1u, (ow[11] >> 4) & 0xf,
	       (ow[11] >> 12) & 0xf, ow[11] >> 16);
}

int main(int argc, char **argv)
{
	char what[256];

	make_inputs();
#ifdef TFU71_MESA
	if (argc == 3 && strcmp(argv[1], "--emit") == 0)
		return emit(argv[2]);
#endif
	if ((argc != 2 && argc != 3) || argv[1][0] == '-') {
		fprintf(stderr, "usage: %s TABLE [GL_MIPGEN_STDOUT]\n", argv[0]);
		return 2;
	}

	/* 1. the committed table: constants, supports, the named cases and
	 * the sweep's first TABLE_SWEEP */
	static struct table t;
	check(read_table(argv[1], &t), "the table reads");
	check_table(&t);
	const unsigned after_table = checks;
	printf("named cases (ours, equal to Mesa's):\n");
	for (unsigned i = 0; i < NNAMED && i < t.ncases; i++)
		compare(named[i].name, "table", &named[i].in, &t.ref[i], 1);
	const unsigned after_named = checks;

	unsigned live = 0;
#ifdef TFU71_MESA
	/* 2. live against the Mesa half: constants, supports, every input */
	check(mesa_tfu71_nconsts == NCONSTS, "constant count");
	for (unsigned i = 0; i < mesa_tfu71_nconsts && i < NCONSTS; i++) {
		snprintf(what, sizeof(what), "constant %s: ours %u, Mesa %u (%s)", const_names[i],
			 our_consts[i], mesa_tfu71_consts[i], mesa_tfu71_const_names[i]);
		check(our_consts[i] == mesa_tfu71_consts[i] &&
			      strcmp(const_names[i], mesa_tfu71_const_names[i]) == 0,
		      what);
	}
	for (uint32_t ty = 0; ty < 128; ty++)
		for (int m = 0; m < 2; m++) {
			snprintf(what, sizeof(what), "supports(%u, mipmap %d) against Mesa", ty, m);
			check((int)v3d_tfu71_supports(ty, m) == (mesa_tfu71_supports(ty, m) != 0), what);
		}
	for (unsigned i = 0; i < NINPUTS; i++) {
		char name[200];
		input_name(i, name, sizeof(name));
		const struct tfu71_ref r = mesa_ref(&inputs[i]);
		compare(name, "Mesa", &inputs[i], &r, 0);
	}
	live = checks - after_named;
#endif

	/* 2b. gl_mipgen's TFU job from Mesa 7.1 under drm-shim */
	if (argc == 3)
		check_mipgen(argv[2]);

	/* 3. what only ours refuses: a value that would not fit or not
	 * describe the image (Mesa never asks) */
	const unsigned before_refusals = checks;
	const struct {
		const char *name;
		struct tfu71_in in;
	} ours_only[] = {
		{ "cpp 3", { 4, 3, 16, 16, 0, S, RA, 48, D, UIFX, 16 } },
		{ "cpp 0", { 4, 0, 16, 16, 0, S, RA, 64, D, UIFX, 16 } },
		{ "width 0", { 4, 4, 0, 16, 0, S, RA, 64, D, UIFX, 16 } },
		{ "height 0", { 4, 4, 16, 0, 0, S, RA, 64, D, UIFX, 16 } },
		{ "width 16385", { 4, 4, 16385, 1, 0, S, RA, 65540, D, RA, 65540 } },
		{ "height 16385", { 0, 1, 1, 16385, 0, S, RA, 1, D, UIFX, 16400 } },
		{ "16x16 with 5 levels after", { 4, 4, 16, 16, 5, S, RA, 64, D, UIFX, 16 } },
		{ "1x1 with 1 level after", { 4, 4, 1, 1, 1, S, RA, 4, D, UIFX, 8 } },
		{ "raster src stride not texels", { 4, 4, 16, 16, 0, S, RA, 66, D, UIFX, 16 } },
		{ "raster src stride short", { 4, 4, 16, 16, 0, S, RA, 60, D, UIFX, 16 } },
		{ "raster dst stride short", { 4, 4, 16, 16, 0, S, UIFX, 16, D, RA, 32 } },
		{ "uif dst padded height not blocks", { 4, 4, 16, 16, 0, S, RA, 64, D, UIFX, 20 } },
		{ "uif dst padded height low", { 4, 4, 16, 24, 0, S, RA, 64, D, UIFX, 16 } },
		{ "uif src padded height not blocks", { 0, 1, 16, 16, 0, S, UIFN, 24, D, UIFX, 16 } },
		{ "raster dst stride over 16 bits", { 4, 4, 16, 16, 0, S, RA, 64, D, RA, 0x10000u * 4u } },
		{ "tiling 6", { 4, 4, 16, 16, 0, S, 6, 0, D, UIFX, 16 } },
	};
	for (unsigned i = 0; i < sizeof(ours_only) / sizeof(ours_only[0]); i++)
		refused(ours_only[i].name, &ours_only[i].in);
	{
		struct v3d_tfu71_job j;
		check(!v3d_tfu71_pack(NULL, &j), "NULL args refused");
		const struct v3d_tfu71_args a = to_args(&named[0].in);
		check(!v3d_tfu71_pack(&a, NULL), "NULL job refused");
	}

	printf("tfu71: table %u checks (%u named + %u sweep cases), of them named %u, live against Mesa %u checks (%u inputs)%s, ours-only refusals %u\n",
	       after_named, (unsigned)NNAMED, (unsigned)TABLE_SWEEP, after_named - after_table, live,
	       live ? (unsigned)NINPUTS : 0u, live ? "" : " [no Mesa tree]", checks - before_refusals);
	(void)what;
	if (failed) {
		printf("tfu71: %u of %u checks failed\n", failed, checks);
		return 1;
	}
	printf("tfu71: %u of %u checks, 0 failed\n", checks, checks);
	return 0;
}
