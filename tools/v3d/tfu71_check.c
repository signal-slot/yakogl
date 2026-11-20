/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tfu71_check: driver/src/v3d/v3d_tfu71.c's V3D 7.1 TFU job against Mesa
 * 26.2.2's own 7.1 fill. tools/v3d/check71.d/tfu71.sh builds the other half,
 * mesa_tfu71.c: the lines of Mesa's src/gallium/drivers/v3d/v3dx_tfu.c that
 * compute the job (the IOS initializer, :98; IIA, IOA and IIS, :106-127; the
 * V3D_VERSION >= 71 branch, :165-202; the declines at :64-66 and :88-91),
 * copied verbatim from the Mesa tree with a small shim for the Gallium
 * resource, and compiled at V3D_VERSION 71 with Mesa's own v3d_tfu.h,
 * v3d_drm.h, enum v3d_tiling_mode (v3d_tiling.h:42-64), v3d_utile_height
 * (v3d_tiling.c:55-70), v3dX(tfu_supports_tex_format)
 * (v3dx_format_table.c:343-378) and the generated 7.1 Texture Data Formats.
 *
 * Checks: the constants (field shifts and codes, tiling order, data types,
 * the maximum size) equal Mesa's; v3d_tfu71_supports equals Mesa's for every
 * data type 0..127, with and without mipmaps; the named cases (RGBA8 raster
 * to UIF with and without mipmaps, the mipmap generation of a UIF image,
 * R8 and RGB565, a raster destination, LINEARTILE and UB-linear sides, the
 * declines) and a seeded sweep of valid inputs give Mesa's twelve register
 * words and Mesa's decision; the inputs our packer refuses on its own (a
 * value that would not fit or not describe the image) are refused with the
 * job untouched. One check per comparison; the last line is
 * "tfu71: N of N checks, 0 failed" (exit 0) or the count of failures (1).
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#include "v3d_tfu71.h"

/* mesa_tfu71.c (built by tfu71.sh from the Mesa tree) */
struct mesa_tfu71_in {
	uint32_t ttype, cpp, width, height, num_mm;
	uint32_t src_addr, src_tiling, src_sp;
	uint32_t dst_addr, dst_tiling, dst_sp;
};
struct mesa_tfu71_out {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef[4], ioc;
};
int mesa_tfu71(const struct mesa_tfu71_in *in, struct mesa_tfu71_out *out);
int mesa_tfu71_supports(uint32_t ttype, int for_mipmap);
/* Mesa's constants, in the order mesa_tfu71.c fills them */
extern const uint32_t mesa_tfu71_consts[];
extern const char *const mesa_tfu71_const_names[];
extern const unsigned mesa_tfu71_nconsts;

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

static const uint32_t our_consts[] = {
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

static struct v3d_tfu71_args to_args(const struct mesa_tfu71_in *in)
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

static void words_of_ours(const struct v3d_tfu71_job *j, uint32_t w[12])
{
	const uint32_t v[12] = { j->icfg,    j->iia,     j->iis,     j->ica,     j->iua,     j->ioa,
				 j->ios,     j->coef[0], j->coef[1], j->coef[2], j->coef[3], j->ioc };
	memcpy(w, v, sizeof(v));
}

static void words_of_mesa(const struct mesa_tfu71_out *o, uint32_t w[12])
{
	const uint32_t v[12] = { o->icfg,    o->iia,     o->iis,     o->ica,     o->iua,     o->ioa,
				 o->ios,     o->coef[0], o->coef[1], o->coef[2], o->coef[3], o->ioc };
	memcpy(w, v, sizeof(v));
}

/* One input through both: the decision, and when both packed, the twelve
 * words. `verbose` prints the words (the named cases). */
static void compare(const char *name, const struct mesa_tfu71_in *in, int verbose)
{
	char what[256];
	struct mesa_tfu71_out mo;
	memset(&mo, 0, sizeof(mo));
	const int mok = mesa_tfu71(in, &mo);
	const struct v3d_tfu71_args a = to_args(in);
	struct v3d_tfu71_job j;
	memset(&j, 0xa5, sizeof(j));
	const int ook = v3d_tfu71_pack(&a, &j);
	snprintf(what, sizeof(what), "%s: decision ours %d, Mesa %d", name, ook, mok);
	check(ook == mok, what);
	if (!ook || !mok) {
		if (verbose)
			printf("  %-34s declined (ours and Mesa's)\n", name);
		return;
	}
	uint32_t ow[12], mw[12];
	words_of_ours(&j, ow);
	words_of_mesa(&mo, mw);
	for (int i = 0; i < 12; i++) {
		snprintf(what, sizeof(what), "%s: %s ours 0x%08x, Mesa 0x%08x", name, word_names[i], ow[i],
			 mw[i]);
		check(ow[i] == mw[i], what);
	}
	if (verbose)
		printf("  %-34s icfg %08x iia %08x iis %08x ioa %08x ios %08x ioc %08x\n", name, ow[0], ow[1],
		       ow[2], ow[5], ow[6], ow[11]);
}

/* An input only ours refuses: refused, `j` untouched. */
static void refused(const char *name, const struct mesa_tfu71_in *in)
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

static uint32_t rng_state = 0x7f4a7c15u;

static uint32_t rnd(uint32_t n)
{
	rng_state = rng_state * 1664525u + 1013904223u;
	return (uint32_t)(((uint64_t)(rng_state >> 8) * n) >> 24);
}

enum { RA = 0, LT = 1, UB1 = 2, UB2 = 3, UIFN = 4, UIFX = 5 };

int main(void)
{
	char what[256];

	/* 1. constants */
	check(mesa_tfu71_nconsts == sizeof(our_consts) / sizeof(our_consts[0]), "constant count");
	for (unsigned i = 0; i < mesa_tfu71_nconsts && i < sizeof(our_consts) / sizeof(our_consts[0]); i++) {
		snprintf(what, sizeof(what), "constant %s: ours %u, Mesa %u", mesa_tfu71_const_names[i],
			 our_consts[i], mesa_tfu71_consts[i]);
		check(our_consts[i] == mesa_tfu71_consts[i], what);
	}
	const unsigned after_consts = checks;

	/* 2. the supported data types */
	for (uint32_t t = 0; t < 128; t++)
		for (int m = 0; m < 2; m++) {
			snprintf(what, sizeof(what), "supports(%u, mipmap %d)", t, m);
			check((int)v3d_tfu71_supports(t, m) == (mesa_tfu71_supports(t, m) != 0), what);
		}
	const unsigned after_supports = checks;

	/* 3. named cases */
	printf("named cases (ours, equal to Mesa's):\n");
	const uint32_t S = 0x10000000u, D = 0x20000000u;
	const struct {
		const char *name;
		struct mesa_tfu71_in in;
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
	for (unsigned i = 0; i < sizeof(named) / sizeof(named[0]); i++)
		compare(named[i].name, &named[i].in, 1);
	const unsigned after_named = checks;

	/* 4. seeded sweep of valid inputs */
	static const uint32_t cpps[] = { 1, 2, 4, 8, 16 };
	static const uint32_t sizes[] = { 1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 100, 127, 128,
					  129, 255, 256, 257, 640, 1000, 1024, 1920, 2048, 4095, 4096, 8191,
					  8192, 16383, 16384 };
	const unsigned nsizes = sizeof(sizes) / sizeof(sizes[0]);
	unsigned sweep = 0;
	for (unsigned n = 0; n < 20000; n++) {
		struct mesa_tfu71_in in;
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
		snprintf(what, sizeof(what),
			 "sweep %u (ttype %u cpp %u %ux%u mm %u src %u/%u dst %u/%u)", n, in.ttype, in.cpp,
			 in.width, in.height, in.num_mm, in.src_tiling, in.src_sp, in.dst_tiling, in.dst_sp);
		compare(what, &in, 0);
		sweep++;
	}
	const unsigned after_sweep = checks;

	/* 5. what only ours refuses: a value that would not fit or not
	 * describe the image (Mesa never asks) */
	const struct {
		const char *name;
		struct mesa_tfu71_in in;
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

	printf("tfu71: constants %u, supports %u, named %u, sweep %u inputs (%u checks), ours-only refusals %u\n",
	       after_consts, after_supports - after_consts, after_named - after_supports, sweep,
	       after_sweep - after_named, checks - after_sweep);
	if (failed) {
		printf("tfu71: %u of %u checks failed\n", failed, checks);
		return 1;
	}
	printf("tfu71: %u of %u checks, 0 failed\n", checks, checks);
	return 0;
}
