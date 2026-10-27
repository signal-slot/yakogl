/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * tfu_case_check: checks tools/v3d/tfu_case.h, the TFU probe of v3d_gl_test
 * case 140, on the host. The TFU itself is on the board only; what is checked
 * is everything the board run's verdict rests on:
 *
 *  1. The sizes reach every tiling (LT, UBLINEAR 1 and 2, UIF, UIF XOR) for
 *     each texel size the formats use, and UIF with padding (OPAD > 0).
 *  2. Every packed job of every format, size and variant: each field against
 *     an independent source (IIS * cpp against the stride, the IOA format
 *     against the layout's tiling, OPAD against v3d_tex_layout's ub_pad).
 *  3. One job as literals from Mesa's v3d_tfu.h constants (95x37 RGBA8 as
 *     R32F: UIF no XOR, format 6).
 *  4. The refusals: two levels, a short stride, a stride of part texels, a
 *     destination off 64 bytes, a source not 4-byte aligned.
 *  5. tfu_case_compare: a copy equal to the CPU's passes; one wrong byte, a
 *     texel left as the fill, a padding byte written, a whole image tiled
 *     at the wrong stride, and a stale source (another job's seed) are each
 *     found.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include -Itools/v3d \
 *      tools/v3d/tfu_case_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
 *      -lm -o build/tfu_case_check
 *
 * Exit status 0 only when every check passed AND the number run is the number
 * planned.
 */
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tfu_case.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("FAIL: %s\n", what);
	}
}

/* 1 */
static void coverage(void)
{
	static const uint32_t cpps[3] = { 1, 2, 4 };
	for (int c = 0; c < 3; c++) {
		unsigned seen = 0;
		bool padded_uif = false;
		for (int i = 0; i < TFU_CASE_SIZES; i++) {
			struct v3d_tex_layout l;
			if (!v3d_tex_layout(tfu_case_sizes[i][0], tfu_case_sizes[i][1], 1, cpps[c], &l))
				continue;
			seen |= 1u << l.slices[0].tiling;
			padded_uif |= l.slices[0].ub_pad > 0;
		}
		char what[96];
		snprintf(what, sizeof(what), "cpp %u: the sizes reach all five tilings (seen 0x%x)", cpps[c], seen);
		check(seen == (1u << V3D_TEX_TILING_LINEARTILE | 1u << V3D_TEX_TILING_UBLINEAR_1_COLUMN |
			       1u << V3D_TEX_TILING_UBLINEAR_2_COLUMN | 1u << V3D_TEX_TILING_UIF_NO_XOR |
			       1u << V3D_TEX_TILING_UIF_XOR),
		      what);
		snprintf(what, sizeof(what), "cpp %u: a UIF size with padding (OPAD > 0)", cpps[c]);
		check(padded_uif, what);
	}
	bool cpps_ok = true;
	for (int f = 0; f < TFU_CASE_FMTS; f++)
		cpps_ok &= tfu_case_fmts[f].cpp == 1 || tfu_case_fmts[f].cpp == 2 || tfu_case_fmts[f].cpp == 4;
	check(cpps_ok, "every format's texel size is 1, 2 or 4 bytes");
}

/* 2 */
static void fields(void)
{
	const uint32_t dst = 0x00100000u, src = 0x00200000u;
	int packed = 0, wrong = 0;
	for (int f = 0; f < TFU_CASE_FMTS; f++)
		for (int i = 0; i < TFU_CASE_SIZES; i++)
			for (int v = 0; v < TFU_CASE_VARIANTS; v++) {
				const struct tfu_case_fmt *fm = &tfu_case_fmts[f];
				const struct tfu_case_variant *va = &tfu_case_variants[v];
				const uint32_t w = tfu_case_sizes[i][0], h = tfu_case_sizes[i][1];
				struct v3d_tex_layout l;
				struct tfu_case_job j;
				if (!v3d_tex_layout(w, h, 1, fm->cpp, &l))
					continue;
				const uint32_t stride = tfu_case_stride(w, fm->cpp, va->aligned_stride);
				if (!tfu_pack(&l, fm->ttype, src + va->src_offset, stride, dst, &j))
					continue;
				packed++;
				const struct v3d_tex_slice *s = &l.slices[0];
				const bool uif = s->tiling == V3D_TEX_TILING_UIF_NO_XOR ||
						 s->tiling == V3D_TEX_TILING_UIF_XOR;
				bool ok = j.ios == (h << 16 | w) && j.iia == src + va->src_offset &&
					  j.iis * fm->cpp == stride && (j.ioa & ~0x3fu) == dst + s->offset &&
					  (j.ioa & 7u) == 0 && ((j.ioa >> 3) & 7u) == (uint32_t)s->tiling + 2u &&
					  ((j.icfg >> 9) & 0x7fu) == fm->ttype && ((j.icfg >> 18) & 0xfu) == 0 &&
					  ((j.icfg >> 5) & 0xfu) == 0 && (j.icfg & 0x1fu) == 0 &&
					  (j.icfg >> 22) == (uif ? s->ub_pad : 0u) && j.ica == 0 && j.iua == 0 &&
					  j.coef0 == 0;
				if (!ok) {
					wrong++;
					printf("  %s %ux%u %s: icfg 0x%08x iis %u ioa 0x%08x ios 0x%08x ub_pad %u\n", fm->name,
					       w, h, va->name, j.icfg, j.iis, j.ioa, j.ios, s->ub_pad);
				}
			}
	char what[96];
	snprintf(what, sizeof(what), "every job packed (%d of %d)", packed,
		 TFU_CASE_FMTS * TFU_CASE_SIZES * TFU_CASE_VARIANTS);
	check(packed == TFU_CASE_FMTS * TFU_CASE_SIZES * TFU_CASE_VARIANTS, what);
	snprintf(what, sizeof(what), "every packed field matches its source (%d wrong)", wrong);
	check(wrong == 0, what);
}

/* 3: Mesa v3d_tfu.h: V3D33_TFU_IOA_FORMAT_UIF_NO_XOR 6, TTYPE shift 9,
 * R32F 29; 95x37 RGBA8 is UIF no XOR, padded to 40 rows (5 UIF blocks of 8),
 * OPAD 0; stride 384 bytes = 96 texels */
static void literal(void)
{
	struct v3d_tex_layout l;
	struct tfu_case_job j = { 0 };
	bool ok = v3d_tex_layout(95, 37, 1, 4, &l) &&
		  tfu_pack(&l, TFU_TTYPE_R32F, 0x00200000u, 384, 0x00100000u, &j);
	check(ok && l.slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR && l.slices[0].padded_height == 40,
	      "95x37 RGBA8 is UIF no XOR, 40 rows");
	check(ok && j.ios == 0x0025005fu && j.iis == 96 && j.icfg == 29u << 9 && j.ioa == (0x00100000u | 6u << 3) &&
		      j.iia == 0x00200000u,
	      "95x37 RGBA8 as R32F: ios 0x0025005f, iis 96, icfg 0x3a00, ioa 0x100030");
}

/* 4 */
static void refusals(void)
{
	struct v3d_tex_layout l, l2;
	struct tfu_case_job j;
	v3d_tex_layout(95, 37, 1, 4, &l);
	v3d_tex_layout(95, 37, 2, 4, &l2);
	check(!tfu_pack(&l2, TFU_TTYPE_R32F, 0x200000u, 384, 0x100000u, &j), "refused: two levels");
	check(!tfu_pack(&l, TFU_TTYPE_R32F, 0x200000u, 95 * 4 - 4, 0x100000u, &j), "refused: a stride short of a row");
	check(!tfu_pack(&l, TFU_TTYPE_R32F, 0x200000u, 95 * 4 + 2, 0x100000u, &j), "refused: part texels");
	check(!tfu_pack(&l, TFU_TTYPE_R32F, 0x200000u, 384, 0x100004u, &j), "refused: a destination off 64 bytes");
	check(!tfu_pack(&l, TFU_TTYPE_R32F, 0x200001u, 384, 0x100000u, &j),
	      "refused: a source not 4-byte aligned (unmeasured on the board)");
}

/* 5 */
static void compare(void)
{
	const uint32_t w = 95, h = 37, cpp = 4, stride = tfu_case_stride(w, cpp, true);
	struct v3d_tex_layout l;
	struct tfu_case_cmp c;
	v3d_tex_layout(w, h, 1, cpp, &l);
	uint8_t *src = malloc((size_t)stride * h), *ref = malloc(l.size), *got = malloc(l.size);
	if (!src || !ref || !got) {
		check(false, "memory");
		free(src);
		free(ref);
		free(got);
		return;
	}
	memset(src, 0, (size_t)stride * h);
	tfu_case_fill_src(src, w, h, cpp, stride, 1);
	memset(ref, TFU_CASE_FILL, l.size);
	v3d_tex_store_level(ref, &l, 0, src, stride);
	memcpy(got, ref, l.size);
	tfu_case_compare(ref, got, &l, &c);
	check(c.texels == w * h && c.texel_wrong == 0 && c.pad_written == 0 &&
		      c.pad_bytes == l.slices[0].size - w * h * cpp && c.first_x == UINT32_MAX,
	      "an equal copy: every texel compared, none wrong, padding counted");

	const struct v3d_tex_slice *s = &l.slices[0];
	const uint32_t o = s->offset + v3d_tex_pixel_offset(s->tiling, cpp, s->padded_height, 40, 17);
	got[o + 2] ^= 0x01;
	tfu_case_compare(ref, got, &l, &c);
	check(c.texel_wrong == 1 && c.texel_unwritten == 0 && c.first_x == 40 && c.first_y == 17,
	      "one wrong byte: texel (40, 17)");
	memset(got + o, TFU_CASE_FILL, cpp);
	tfu_case_compare(ref, got, &l, &c);
	check(c.texel_wrong == 1 && c.texel_unwritten == 1, "a texel left as the fill: unwritten");
	memcpy(got, ref, l.size);
	/* a padding byte: the first byte of the slice the reference left as the fill */
	uint32_t p = s->offset;
	while (p < s->offset + s->size && ref[p] != TFU_CASE_FILL)
		p++;
	got[p] = 0;
	tfu_case_compare(ref, got, &l, &c);
	check(p < s->offset + s->size && c.texel_wrong == 0 && c.pad_written == 1,
	      "a padding byte written: a finding, no texel wrong");
	/* the whole image tiled at the tight stride: the rows shear */
	memset(got, TFU_CASE_FILL, l.size);
	v3d_tex_store_level(got, &l, 0, src, w * cpp);
	tfu_case_compare(ref, got, &l, &c);
	check(c.texel_wrong > w * h / 2, "the image tiled at the wrong stride: most texels wrong");
	/* the previous job's source (another seed) read instead of this one's */
	uint8_t *stale = malloc((size_t)stride * h);
	if (stale) {
		memset(stale, 0, (size_t)stride * h);
		tfu_case_fill_src(stale, w, h, cpp, stride, 2);
		memset(got, TFU_CASE_FILL, l.size);
		v3d_tex_store_level(got, &l, 0, stale, stride);
		tfu_case_compare(ref, got, &l, &c);
	}
	check(stale && c.texel_wrong == w * h, "a stale source (the previous seed): every texel wrong");
	free(stale);
	free(src);
	free(ref);
	free(got);
}

int main(void)
{
	const int planned = 3 * 2 + 1 + 2 + 2 + 5 + 6;
	coverage();
	fields();
	literal();
	refusals();
	compare();
	printf("tfu_case_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return failures == 0 && checks == planned ? 0 : 1;
}
