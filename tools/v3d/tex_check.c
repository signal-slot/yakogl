/* SPDX-License-Identifier: MIT
 *
 * tex_check: checks driver/src/v3d/v3d_tex.c, the V3D 4.2 texture layout,
 * tiling and state records the backend's textures use
 * (docs/online-shaderc/v3d-backend.md, "M6 textures"), against Mesa 26.2.1:
 *
 *  1. The level layout (v3d_tex_layout) against numbers derived by hand from
 *     Mesa's v3d_setup_slices (gallium/drivers/v3d/v3d_resource.c), which is
 *     static and cannot be linked. The cases reach every rule: each tiling,
 *     the power-of-two padding below level 1, ub_pad (both branches), UIF XOR,
 *     the page alignment of level 1, and the final 4 KiB alignment.
 *  2. The texel tiling (v3d_tex_store_level) byte for byte against Mesa's
 *     v3d_store_tiled_image (broadcom/common/v3d_tiling.c, linked from
 *     libv3d_neon.a): every level of a set of sizes, every cpp.
 *  3. The texture and sampler state records against Mesa's generated
 *     V3D42_TEXTURE_SHADER_STATE_pack and V3D42_SAMPLER_STATE_pack.
 *  4. Sub-image stores (v3d_tex_store_box, glTexSubImage2D) against Mesa's
 *     v3d_store_tiled_image with the same pipe_box: fixed boxes across utile,
 *     UIF-block and XOR-column edges plus seeded random ones, on every level
 *     of 9 sizes (a 512x512 single-level image among them) at every cpp,
 *     the whole image compared after every box; and the refusals.
 *  5. The source formats against Mesa's own format table
 *     (v3dx_format_table.c, compiled here; tex_check_mesa_fmt.c): the
 *     supported rows equal Mesa's, every other format is refused by name.
 *  6. Every GL ES 2.0 sampler (6 min x 2 mag filters x 3 x 3 wraps) and
 *     femtovg's four named ones against V3D42_SAMPLER_STATE_pack of what
 *     Mesa's v3d_upload_sampler_state_variant fills.
 *  7. Negative controls, each of which must be detected: XOR dropped, a box
 *     one texel off, a flipped record bit, swapped wraps, a wrong swizzle.
 *  8. The packers' refusals (a type no supported format has, every field
 *     that does not fit; nothing is masked) and the TMU P0/P1 uniform words
 *     against V3D42_TMU_CONFIG_PARAMETER_0/1_pack.
 *
 * Built and run against a Mesa 26.2.1 tree with build-v3d
 * (tools/v3d/README.md) by tools/v3d/tex_check.sh, which holds the build
 * command; tools/check.sh runs that script.
 *
 * Exit status 0 only when every check passed AND the number of checks run is
 * the number planned (and the boxes, formats and samplers each counted).
 */
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

/* Mesa's packers come through v3d_cl.h (v3d_tex.h), which sets their hooks
 * and compiles them with NDEBUG: it must be the first to include the
 * generated header. */
#include "v3d_tex.h"
#include "util/box.h"
#include "common/v3d_tiling.h"

#include "tex_check_mesa_fmt.h"
#include "tex_pixel_offset.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("tex_check: FAIL %s\n", what);
	}
}

/* ---- 1. the layout against Mesa's rules, derived by hand ----------------- */

struct want_slice {
	uint32_t offset, stride, padded_height, ub_pad;
	enum v3d_tex_tiling tiling;
};

static void layout_case(const char *name, uint32_t w, uint32_t h, uint32_t levels,
			uint32_t size, const struct want_slice *want)
{
	struct v3d_tex_layout l;
	char what[160];
	snprintf(what, sizeof(what), "%s: laid out", name);
	check(v3d_tex_layout(w, h, levels, 4, &l), what);
	snprintf(what, sizeof(what), "%s: size %u, want %u", name, l.size, size);
	check(l.size == size, what);
	for (uint32_t i = 0; i < levels; i++) {
		const struct v3d_tex_slice *s = &l.slices[i];
		const struct want_slice *x = &want[i];
		snprintf(what, sizeof(what),
			 "%s level %u: offset %u stride %u h %u pad %u tiling %d, want %u %u %u %u %d",
			 name, i, s->offset, s->stride, s->padded_height, s->ub_pad, s->tiling,
			 x->offset, x->stride, x->padded_height, x->ub_pad, x->tiling);
		check(s->offset == x->offset && s->stride == x->stride &&
			      s->padded_height == x->padded_height && s->ub_pad == x->ub_pad &&
			      s->tiling == x->tiling,
		      what);
	}
}

#define LT V3D_TEX_TILING_LINEARTILE
#define UB1 V3D_TEX_TILING_UBLINEAR_1_COLUMN
#define UB2 V3D_TEX_TILING_UBLINEAR_2_COLUMN
#define UIF V3D_TEX_TILING_UIF_NO_XOR
#define UIFX V3D_TEX_TILING_UIF_XOR

static void part_layout(void)
{
	/* RGBA8: utile 4x4 (64 B), UIF block 8x8 (256 B) */
	/* 1x1: LT, a 4x4 utile */
	layout_case("1x1", 1, 1, 1, 64, (struct want_slice[]){ { 0, 16, 4, 0, LT } });
	/* 8x8: <= one UIF block wide: UB 1 column */
	layout_case("8x8", 8, 8, 1, 256, (struct want_slice[]){ { 0, 32, 8, 0, UB1 } });
	/* 16x16: <= two blocks wide: UB 2 columns */
	layout_case("16x16", 16, 16, 1, 1024, (struct want_slice[]){ { 0, 64, 16, 0, UB2 } });
	/* 64x64: UIF, height 8 UB rows (8 % 32 in 6..26): no pad, no XOR */
	layout_case("64x64", 64, 64, 1, 16384, (struct want_slice[]){ { 0, 256, 64, 0, UIF } });
	/* 64x264: 33 UB rows, 33 % 32 = 1 < 6 and 33 >= 32: pad 6 - 1 = 5,
	 * 38 rows (304 px); 38 % 32 != 0: no XOR */
	layout_case("64x264 (ub_pad)", 64, 264, 1, 256 * 304,
		    (struct want_slice[]){ { 0, 256, 304, 5, UIF } });
	/* 64x480: 60 rows, 60 % 32 = 28 > 26: pad 32 - 28 = 4, 64 rows (512 px);
	 * 64 % 32 == 0: XOR */
	layout_case("64x480 (pad to XOR)", 64, 480, 1, 256 * 512,
		    (struct want_slice[]){ { 0, 256, 512, 4, UIFX } });
	/* 256x256, 9 levels, smallest first. pot = 2 * next_pot(128) = 256.
	 *   8: 1x1 LT 64 @0     7: 2x2 LT 64 @64     6: 4x4 LT 64 @128
	 *   5: 8x8 UB1 256 @192 4: 16x16 UB2 1024 @448
	 *   3: 32x32 UIF (4 rows, < 32: no pad) 4096 @1472
	 *   2: 64x64 UIF 16384 @5568
	 *   1: 128x128 UIF (16 rows) 65536 @21952; 128 px is not > 26 rows:
	 *      no page alignment
	 *   0: 256x256, 32 rows, 32 % 32 == 0: XOR, 262144 @87488
	 * Level 0 to 4 KiB: 90112 - 87488 = 2624 added to every offset. */
	layout_case("256x256 x9", 256, 256, 9, 349632 + 2624,
		    (struct want_slice[]){
			    { 90112, 1024, 256, 0, UIFX }, { 24576, 512, 128, 0, UIF },
			    { 8192, 256, 64, 0, UIF },     { 4096, 128, 32, 0, UIF },
			    { 3072, 64, 16, 0, UB2 },      { 2816, 32, 8, 0, UB1 },
			    { 2752, 16, 4, 0, LT },        { 2688, 16, 4, 0, LT },
			    { 2624, 16, 4, 0, LT } });
	/* 80x656, 2 levels. Level 1: 40x328 -> 64 wide, 41 rows (9 mod 32: no
	 * pad, no XOR), 256 x 328 = 83968; 64 > 32 and 328 > 208 (26 rows): its
	 * total is page-aligned to 86016. Level 0: 96 wide, 82 rows (18: no pad,
	 * no XOR), 384 x 656 = 251904 @86016 (already 4 KiB aligned). */
	layout_case("80x656 x2 (level 1 page-aligned)", 80, 656, 2, 86016 + 251904,
		    (struct want_slice[]){ { 86016, 384, 656, 0, UIF }, { 0, 256, 328, 0, UIF } });
	/* 13x7 x3, non-power-of-two: pot = 2 * next_pot(6) = 16 wide, 2 *
	 * next_pot(3) = 8 high. Level 2: 16>>2 = 4 x 8>>2 = 2: LT 16 x 4 = 64
	 * @0. Level 1: 6x3 (minify of 13x7): 3 <= 4 rows: LT, 8 x 4 -> stride
	 * 32, 128 @64. Level 0: 13x7: 13 > 8 and <= 16: UB2, 16 x 8, stride 64,
	 * 512 @192. Level 0 to 4 KiB: +3904. */
	layout_case("13x7 x3 (non-pot)", 13, 7, 3, 704 + 3904,
		    (struct want_slice[]){ { 4096, 64, 8, 0, UB2 }, { 3968, 32, 4, 0, LT },
					   { 3904, 16, 4, 0, LT } });

	struct v3d_tex_layout l;
	check(!v3d_tex_layout(0, 4, 1, 4, &l), "an empty image is refused");
	check(!v3d_tex_layout(16384, 4, 1, 4, &l), "a width past 14 bits is refused");
	check(!v3d_tex_layout(4, 4, 16, 4, &l), "more levels than the state holds are refused");
	check(!v3d_tex_layout(4, 4, 1, 3, &l), "a texel size the TMU has no utile for is refused");

	/* 4x multisampled (v3d_resource.c:612-642: one level at 2w x 2h, UIF at
	 * the top whatever its size). 8x8 at 4x is 16x16 RGBA8: UIF forced,
	 * width to a 4-block column (32), height to a UIF block (16); 2 UB rows
	 * are under the 32-row page cache (PAGE_CACHE_UB_ROWS = 32768 / 1024),
	 * so no pad, and 2 % 32 != 0: no XOR. 32 x 4 = 128 bytes a row, 2048 in
	 * all. The same 16x16 laid out by size is UB 2 columns. */
	check(v3d_tex_layout_msaa(8, 8, 4, &l) && l.levels == 1 && l.width == 16 && l.height == 16,
	      "8x8 at 4x: laid out, one level of 16x16");
	check(l.size == 2048, "8x8 at 4x: 2048 bytes");
	check(l.slices[0].offset == 0 && l.slices[0].stride == 128 && l.slices[0].padded_height == 16 &&
		      l.slices[0].ub_pad == 0 && l.slices[0].tiling == UIF,
	      "8x8 at 4x: level 0 UIF (no XOR), stride 128, height 16, no pad");
	check(v3d_tex_layout(16, 16, 1, 4, &l) && l.slices[0].tiling == UB2,
	      "16x16 laid out by size is UB 2 columns: the 4x layout's UIF is uif_top's");
	check(!v3d_tex_layout_msaa(8192, 4, 4, &l), "a 4x width whose double passes 14 bits is refused");
}

/* ---- 2. the tiling against Mesa's v3d_store_tiled_image ------------------ */

/* Texel data must not repeat with any period the tiling can move data by,
 * or two different layouts give identical images. UIF XOR swaps macroblock
 * rows 16 apart, and that is a multiple of 256 bytes, and often of 64 KiB,
 * so two generators were blind to it here: i * 131 + 1 (period 256 bytes;
 * an XOR mask mutant 0x10 -> 0x08 went undetected at 512x512 cpp 1, 2, 4 and
 * 256x256 cpp 2, 4), and then bits 8..15 of an LCG mod 2^32 (period 65536:
 * the "XOR dropped" control read 0 differing bytes). xorshift32 has no short
 * period in any bit. */
static uint32_t rng_state = 12345;
static uint32_t rng(void)
{
	uint32_t x = rng_state;
	x ^= x << 13;
	x ^= x >> 17;
	x ^= x << 5;
	rng_state = x;
	return x;
}

static void tiling_case(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp)
{
	struct v3d_tex_layout l;
	char what[160];
	if (!v3d_tex_layout(w, h, levels, cpp, &l)) {
		snprintf(what, sizeof(what), "tiling %ux%u x%u cpp %u: laid out", w, h, levels, cpp);
		check(false, what);
		return;
	}
	uint8_t *ours = calloc(1, l.size), *mesa = calloc(1, l.size);
	for (uint32_t lv = 0; lv < levels; lv++) {
		uint32_t lw = w >> lv ? w >> lv : 1, lh = h >> lv ? h >> lv : 1;
		uint32_t stride = lw * cpp + 12; /* a source pitch wider than the level */
		uint8_t *src = malloc((size_t)stride * lh);
		for (uint32_t i = 0; i < stride * lh; i++)
			src[i] = (uint8_t)rng();
		v3d_tex_store_level(ours, &l, lv, src, stride);
		const struct v3d_tex_slice *s = &l.slices[lv];
		struct pipe_box box = { .x = 0, .width = (int32_t)lw, .y = 0, .height = (int32_t)lh,
					.z = 0, .depth = 1 };
		v3d_store_tiled_image(mesa + s->offset, s->stride, src, stride,
				      (enum v3d_tiling_mode)s->tiling, (int)cpp, s->padded_height, &box);
		free(src);
	}
	snprintf(what, sizeof(what), "tiling %ux%u x%u cpp %u: our bytes are Mesa's", w, h,
		 levels, cpp);
	check(memcmp(ours, mesa, l.size) == 0, what);
	free(ours);
	free(mesa);
}

static void part_tiling(void)
{
	/* our enum is Mesa's, value for value */
	check((int)V3D_TEX_TILING_LINEARTILE == (int)V3D_TILING_LINEARTILE &&
		      (int)V3D_TEX_TILING_UBLINEAR_1_COLUMN == (int)V3D_TILING_UBLINEAR_1_COLUMN &&
		      (int)V3D_TEX_TILING_UBLINEAR_2_COLUMN == (int)V3D_TILING_UBLINEAR_2_COLUMN &&
		      (int)V3D_TEX_TILING_UIF_NO_XOR == (int)V3D_TILING_UIF_NO_XOR &&
		      (int)V3D_TEX_TILING_UIF_XOR == (int)V3D_TILING_UIF_XOR,
	      "the tiling enum values are Mesa's");
	static const uint32_t sizes[][3] = {
		{ 1, 1, 1 },   { 3, 5, 1 },    { 8, 8, 1 },     { 16, 16, 1 },  { 13, 7, 3 },
		{ 37, 61, 6 }, { 64, 64, 7 },  { 64, 264, 1 },  { 64, 480, 1 }, { 100, 3, 2 },
		{ 256, 256, 9 }, { 80, 656, 2 }, { 512, 512, 10 },
	};
	static const uint32_t cpps[] = { 1, 2, 4, 8, 16 };
	for (unsigned i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++)
		for (unsigned c = 0; c < sizeof(cpps) / sizeof(cpps[0]); c++)
			tiling_case(sizes[i][0], sizes[i][1], sizes[i][2], cpps[c]);
}

/* ---- 3. the records against Mesa's packers --------------------------------- */

/* One record: ours from our format table's row and our layout at its cpp,
 * Mesa's from its own format table (tex_check_mesa_fmt.c) and its packer. A
 * format our table or Mesa's lacks fails the check (nothing is packed). */
static void state_case(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t base,
		       enum v3d_tex_format_id id)
{
	struct v3d_tex_format f;
	struct mesa_fmt m;
	bool have = v3d_tex_format_get(id, &f) && mesa_v3d_format(v3d_tex_format_name(id), &m) &&
		    m.in_table;
	struct v3d_tex_layout l;
	uint8_t ours[V3D_TEX_STATE_BYTES], mesa[V3D_TEX_STATE_BYTES];
	char what[160];
	if (!have || !v3d_tex_layout(w, h, levels, f.cpp, &l)) {
		snprintf(what, sizeof(what), "texture state %s: our row, Mesa's row and a layout", name);
		check(false, what);
		return;
	}
	struct v3d_tex_state s = {
		.base = base + l.slices[0].offset,
		.width = w,
		.height = h,
		.depth = 1,
		.base_level = 0,
		.max_level = levels - 1,
		.type = f.type,
	};
	memcpy(s.swizzle, f.swizzle, 4);
	v3d_tex_state_from_layout(&s, &l);
	bool packed = v3d_tex_pack_state(ours, &s);

	/* v3d_setup_texture_shader_state + create_texture_shader_state_bo */
	struct V3D42_TEXTURE_SHADER_STATE t = { 0 };
	t.image_width = w;
	t.image_height = h;
	t.image_depth = 1;
	t.base_level = 0;
	t.max_level = levels - 1;
	t.texture_base_pointer = base + l.slices[0].offset;
	t.array_stride_64_byte_aligned = ((l.slices[0].offset + l.slices[0].size + 63) & ~63u) / 64;
	t.level_0_is_strictly_uif = l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR ||
				    l.slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR;
	t.level_0_xor_enable = l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR;
	if (t.level_0_is_strictly_uif) {
		t.level_0_ub_pad = l.slices[0].ub_pad;
		t.extended = true;
	}
	t.swizzle_r = m.swizzle[0];
	t.swizzle_g = m.swizzle[1];
	t.swizzle_b = m.swizzle[2];
	t.swizzle_a = m.swizzle[3];
	t.texture_type = m.type;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, mesa, &t);

	snprintf(what, sizeof(what), "texture state %s: our bytes are Mesa's", name);
	check(packed && memcmp(ours, mesa, sizeof(ours)) == 0, what);
	if (memcmp(ours, mesa, sizeof(ours)) != 0)
		for (int i = 0; i < V3D_TEX_STATE_BYTES; i++)
			printf("  %2d: ours %02x mesa %02x\n", i, ours[i], mesa[i]);
}

static void sampler_case(const char *name, const struct v3d_sampler *s,
			 const struct V3D42_SAMPLER_STATE *m)
{
	uint8_t ours[V3D_SAMPLER_STATE_BYTES], mesa[V3D_SAMPLER_STATE_BYTES];
	bool packed = v3d_tex_pack_sampler(ours, s);
	V3D42_SAMPLER_STATE_pack(NULL, mesa, m);
	char what[160];
	snprintf(what, sizeof(what), "sampler state %s: our bytes are Mesa's", name);
	check(packed && memcmp(ours, mesa, sizeof(ours)) == 0, what);
	if (memcmp(ours, mesa, sizeof(ours)) != 0)
		for (int i = 0; i < V3D_SAMPLER_STATE_BYTES; i++)
			printf("  %2d: ours %02x mesa %02x\n", i, ours[i], mesa[i]);
}

static void part_records(void)
{
	check(TEXTURE_DATA_FORMAT_RGBA8 == V3D_TEXTURE_DATA_FORMAT_RGBA8, "RGBA8's type is Mesa's");
	state_case("64x64 RGBA8", 64, 64, 1, 0x00400000u, V3D_TEX_FORMAT_RGBA8);
	state_case("256x256 x9 (UIF XOR at level 0)", 256, 256, 9, 0x00410000u, V3D_TEX_FORMAT_RGBA8);
	state_case("64x264 (ub_pad 5)", 64, 264, 1, 0x00480000u, V3D_TEX_FORMAT_RGBA8);
	state_case("13x7 x3 BGRA (UB2 at level 0)", 13, 7, 3, 0x00490000u, V3D_TEX_FORMAT_BGRA8);
	state_case("1x1 (LT)", 1, 1, 1, 0x004a0000u, V3D_TEX_FORMAT_RGBA8);
	/* the narrow formats: R8 and RG8, their swizzles, cpp 1 and 2 layouts */
	state_case("64x64 x7 L8", 64, 64, 7, 0x004b0000u, V3D_TEX_FORMAT_L8);
	state_case("512x512 x10 A8 (UIF at level 0)", 512, 512, 10, 0x00500000u, V3D_TEX_FORMAT_A8);
	state_case("13x7 x3 L8A8", 13, 7, 3, 0x00600000u, V3D_TEX_FORMAT_L8A8);

	/* GL's defaults: min_lod -1000, max_lod 1000, no bias
	 * (v3d_upload_sampler_state_variant with the clamps it applies) */
	struct v3d_sampler lin_mip = { .wrap_s = V3D_WRAP_MODE_REPEAT, .wrap_t = V3D_WRAP_MODE_CLAMP,
				       .wrap_r = V3D_WRAP_MODE_REPEAT, .min_lod = -1000.0f,
				       .max_lod = 1000.0f, .border_color_mode = V3D_BORDER_COLOR_0000 };
	struct V3D42_SAMPLER_STATE m1 = { .wrap_s = V3D_WRAP_MODE_REPEAT,
					  .wrap_t = V3D_WRAP_MODE_CLAMP,
					  .wrap_r = V3D_WRAP_MODE_REPEAT,
					  .min_level_of_detail = 0.0f,
					  .max_level_of_detail = 15.0f,
					  .border_color_mode = V3D_BORDER_COLOR_0000 };
	sampler_case("linear, linear mip, repeat/clamp", &lin_mip, &m1);

	struct v3d_sampler nearest = { .wrap_s = V3D_WRAP_MODE_MIRROR, .wrap_t = V3D_WRAP_MODE_MIRROR,
				       .min_nearest = true, .mag_nearest = true, .mip_nearest = true,
				       .mip_none = true, .min_lod = -1000.0f, .max_lod = 1000.0f };
	struct V3D42_SAMPLER_STATE m2 = { .wrap_s = V3D_WRAP_MODE_MIRROR,
					  .wrap_t = V3D_WRAP_MODE_MIRROR,
					  .min_filter_nearest = true,
					  .mag_filter_nearest = true,
					  .mip_filter_nearest = true,
					  /* MIN2(MIN2(MAX2(0, -1000), 15), 1/256) = 0 and
					   * MIN2(MIN2(MAX2(1000, -1000), 15), 1/256) = 1/256 */
					  .min_level_of_detail = 0.0f,
					  .max_level_of_detail = 1.0f / 256.0f };
	sampler_case("nearest, no mips (LOD 1/256), mirror", &nearest, &m2);

	struct v3d_sampler bias = { .min_nearest = false, .mag_nearest = true, .mip_nearest = true,
				    .min_lod = 2.0f, .max_lod = 5.5f, .lod_bias = -1.25f };
	struct V3D42_SAMPLER_STATE m3 = { .mag_filter_nearest = true,
					  .mip_filter_nearest = true,
					  .min_level_of_detail = 2.0f,
					  .max_level_of_detail = 5.5f,
					  .fixed_bias = -1.25f };
	sampler_case("LOD range and a negative bias", &bias, &m3);
}

/* ---- 4. sub-box stores (glTexSubImage2D) against Mesa -------------------- */

/* glTexSubImage2D: femtovg updates its 256x1 RGBA8 gradient texture this way
 * (docs/online-shaderc/v3d-backend.md, "M7"). Mesa's v3d_store_tiled_image
 * takes the same box. Both images start with the same
 * noise, and after EVERY box the whole image is compared, so a texel written
 * outside its box, or a wrong texel a later box overwrites, cannot hide. */

#define FIXED_BOXES 7
#define RANDOM_BOXES 16
static unsigned boxes_stored;
static uint64_t box_bytes_compared;

static uint32_t min_u32(uint32_t a, uint32_t b)
{
	return a < b ? a : b;
}

static void box_case(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp)
{
	char what[320];
	struct v3d_tex_layout l;
	if (!v3d_tex_layout(w, h, levels, cpp, &l)) {
		snprintf(what, sizeof(what), "boxes %ux%u x%u cpp %u: laid out", w, h, levels, cpp);
		check(false, what);
		return;
	}
	uint8_t *ours = malloc(l.size), *mesa = malloc(l.size);
	for (uint32_t i = 0; i < l.size; i++)
		ours[i] = mesa[i] = (uint8_t)(i * 29 + 7);
	const uint32_t uw = v3d_utile_width((int)cpp), uh = v3d_utile_height((int)cpp);
	const uint32_t bw = 2 * uw, bh = 2 * uh; /* one UIF block */
	char first_bad[160] = "";

	for (uint32_t lv = 0; lv < levels; lv++) {
		const struct v3d_tex_slice *s = &l.slices[lv];
		uint32_t lw = w >> lv ? w >> lv : 1, lh = h >> lv ? h >> lv : 1;
		/* x, y, w, h; clipped into the level below */
		uint32_t box[FIXED_BOXES + RANDOM_BOXES][4] = {
			{ 0, 0, 1, 1 },
			{ lw - 1, lh - 1, 1, 1 },
			{ uw - 1, uh - 1, 2, 2 },         /* across a utile corner */
			{ bw - 1, bh - 1, 2, 2 },         /* across a UIF block corner */
			{ 4 * bw - 1, 0, 2, lh },         /* across the column pair UIF XOR flips */
			{ 1, 1, lw > 2 ? lw - 2 : 1, lh > 2 ? lh - 2 : 1 },
			{ 0, 0, lw, lh },
		};
		for (int r = 0; r < RANDOM_BOXES; r++) {
			uint32_t bx = rng() % lw, by = rng() % lh;
			box[FIXED_BOXES + r][0] = bx;
			box[FIXED_BOXES + r][1] = by;
			box[FIXED_BOXES + r][2] = 1 + rng() % (lw - bx);
			box[FIXED_BOXES + r][3] = 1 + rng() % (lh - by);
		}
		for (int b = 0; b < FIXED_BOXES + RANDOM_BOXES; b++) {
			uint32_t bx = min_u32(box[b][0], lw - 1), by = min_u32(box[b][1], lh - 1);
			uint32_t bwid = min_u32(box[b][2], lw - bx), bhei = min_u32(box[b][3], lh - by);
			uint32_t stride = bwid * cpp + 5; /* a source pitch wider than the box */
			uint8_t *src = malloc((size_t)stride * bhei);
			for (uint32_t i = 0; i < stride * bhei; i++)
				src[i] = (uint8_t)rng();
			bool ok = v3d_tex_store_box(ours, &l, lv, bx, by, bwid, bhei, src, stride);
			struct pipe_box pb = { .x = (int32_t)bx, .width = (int32_t)bwid,
					       .y = (int32_t)by, .height = (int32_t)bhei,
					       .z = 0, .depth = 1 };
			v3d_store_tiled_image(mesa + s->offset, s->stride, src, stride,
					      (enum v3d_tiling_mode)s->tiling, (int)cpp,
					      s->padded_height, &pb);
			free(src);
			boxes_stored++;
			box_bytes_compared += l.size;
			if (!first_bad[0] && (!ok || memcmp(ours, mesa, l.size) != 0))
				snprintf(first_bad, sizeof(first_bad),
					 " (first difference: level %u box %u,%u %ux%u%s)", lv, bx,
					 by, bwid, bhei, ok ? "" : ", refused");
		}
	}
	snprintf(what, sizeof(what), "boxes %ux%u x%u cpp %u: our bytes are Mesa's after every box%s",
		 w, h, levels, cpp, first_bad);
	check(!first_bad[0], what);

	/* the other direction (a depth resolve reads the V3D's depth buffer this
	 * way): every level's fixed boxes loaded by v3d_tex_load_box out of our
	 * image against Mesa's v3d_load_tiled_image out of Mesa's, into rows
	 * wider than the box whose padding must stay as it was */
	char first_load_bad[160] = "";
	for (uint32_t lv = 0; lv < levels && !first_load_bad[0]; lv++) {
		const struct v3d_tex_slice *s = &l.slices[lv];
		uint32_t lw = w >> lv ? w >> lv : 1, lh = h >> lv ? h >> lv : 1;
		const uint32_t lbox[4][4] = {
			{ 0, 0, 1, 1 }, { uw - 1, uh - 1, 2, 2 }, { bw - 1, bh - 1, 2, 2 }, { 0, 0, lw, lh },
		};
		for (int b = 0; b < 4; b++) {
			uint32_t bx = min_u32(lbox[b][0], lw - 1), by = min_u32(lbox[b][1], lh - 1);
			uint32_t bwid = min_u32(lbox[b][2], lw - bx), bhei = min_u32(lbox[b][3], lh - by);
			uint32_t stride = bwid * cpp + 3;
			size_t n = (size_t)stride * bhei;
			uint8_t *a = malloc(n), *m = malloc(n);
			memset(a, 0x3c, n);
			memset(m, 0x3c, n);
			bool ok = v3d_tex_load_box(ours, &l, lv, bx, by, bwid, bhei, a, stride);
			struct pipe_box pb = { .x = (int32_t)bx, .width = (int32_t)bwid,
					       .y = (int32_t)by, .height = (int32_t)bhei,
					       .z = 0, .depth = 1 };
			v3d_load_tiled_image(m, stride, mesa + s->offset, s->stride,
					     (enum v3d_tiling_mode)s->tiling, (int)cpp, s->padded_height, &pb);
			if (!ok || memcmp(a, m, n) != 0)
				snprintf(first_load_bad, sizeof(first_load_bad),
					 " (first difference: level %u box %u,%u %ux%u%s)", lv, bx, by,
					 bwid, bhei, ok ? "" : ", refused");
			free(a);
			free(m);
			if (first_load_bad[0])
				break;
		}
	}
	snprintf(what, sizeof(what), "boxes %ux%u x%u cpp %u: load_box reads Mesa's v3d_load_tiled_image%s",
		 w, h, levels, cpp, first_load_bad);
	check(!first_load_bad[0], what);
	free(ours);
	free(mesa);
}

/* The refusals: nothing written, and the verdict GL needs. */
static void box_refusals(void)
{
	struct v3d_tex_layout l;
	v3d_tex_layout(64, 64, 1, 4, &l);
	uint8_t *img = malloc(l.size), *copy = malloc(l.size);
	for (uint32_t i = 0; i < l.size; i++)
		img[i] = copy[i] = (uint8_t)(i * 13 + 1);
	uint8_t src[65 * 4 * 2];
	memset(src, 0xa5, sizeof(src));
	static const struct {
		const char *name;
		uint32_t level, x, y, w, h, stride;
		bool want;
	} r[] = {
		{ "a box starting at the right edge", 0, 64, 0, 1, 1, 256, false },
		{ "a box one texel wider than the level", 0, 0, 0, 65, 1, 260, false },
		{ "a height that would wrap y + h", 0, 0, 1, 1, 0xffffffffu, 256, false },
		{ "an x that would wrap x + w", 0, 0xffffffffu, 0, 2, 1, 256, false },
		{ "a level the layout does not have", 1, 0, 0, 1, 1, 256, false },
		{ "a source pitch shorter than the box row", 0, 0, 0, 4, 2, 15, false },
		{ "an empty box inside the level (GL: no-op)", 0, 3, 3, 0, 5, 0, true },
		{ "an empty box at the right edge (GL: no-op)", 0, 64, 0, 0, 1, 0, true },
	};
	char what[160];
	for (unsigned i = 0; i < sizeof(r) / sizeof(r[0]); i++) {
		bool got = v3d_tex_store_box(img, &l, r[i].level, r[i].x, r[i].y, r[i].w, r[i].h,
					     src, r[i].stride);
		snprintf(what, sizeof(what), "store_box: %s: %s, nothing written", r[i].name,
			 r[i].want ? "accepted" : "refused");
		check(got == r[i].want && memcmp(img, copy, l.size) == 0, what);
		/* each case is judged on its own: undo a write that should not
		 * have happened */
		memcpy(img, copy, l.size);
	}
	/* the same table for load_box: the refusals write nothing into the rows
	 * (the source becomes the destination; the image is only read) */
	for (unsigned i = 0; i < sizeof(r) / sizeof(r[0]); i++) {
		uint8_t dst[65 * 4 * 2], untouched[65 * 4 * 2];
		memset(dst, 0x5c, sizeof(dst));
		memcpy(untouched, dst, sizeof(dst));
		bool got = v3d_tex_load_box(img, &l, r[i].level, r[i].x, r[i].y, r[i].w, r[i].h,
					    dst, r[i].stride);
		snprintf(what, sizeof(what), "load_box: %s: %s, nothing written", r[i].name,
			 r[i].want ? "accepted" : "refused");
		check(got == r[i].want && memcmp(dst, untouched, sizeof(dst)) == 0 &&
			      memcmp(img, copy, l.size) == 0,
		      what);
	}
	free(img);
	free(copy);
}

static void part_boxes(void)
{
	static const uint32_t sizes[][3] = {
		{ 512, 512, 1 }, /* a large single-level image (UIF XOR) */
		{ 1, 1, 1 },     { 13, 7, 3 },   { 37, 61, 6 },  { 64, 264, 1 }, { 64, 480, 1 },
		{ 256, 256, 9 }, { 80, 656, 2 }, { 100, 3, 2 },
	};
	static const uint32_t cpps[] = { 1, 2, 4, 8, 16 };
	for (unsigned i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++)
		for (unsigned c = 0; c < sizeof(cpps) / sizeof(cpps[0]); c++)
			box_case(sizes[i][0], sizes[i][1], sizes[i][2], cpps[c]);
	box_refusals();
}

/* ---- 5. the source formats against Mesa's format table -------------------- */

static unsigned formats_supported;

static void part_formats(void)
{
	char what[200];
	const char *names[V3D_TEX_FORMAT_COUNT];
	for (int id = 0; id < V3D_TEX_FORMAT_COUNT; id++) {
		const char *name = v3d_tex_format_name((enum v3d_tex_format_id)id);
		names[id] = name;
		struct mesa_fmt m;
		snprintf(what, sizeof(what), "format %s: Mesa knows the name", name);
		check(mesa_v3d_format(name, &m), what);

		struct v3d_tex_format f;
		memset(&f, 0x5a, sizeof(f));
		struct v3d_tex_format untouched = f;
		bool got = v3d_tex_format_get((enum v3d_tex_format_id)id, &f);
		bool want = id == V3D_TEX_FORMAT_RGBA8 || id == V3D_TEX_FORMAT_BGRA8 ||
			    id == V3D_TEX_FORMAT_L8 || id == V3D_TEX_FORMAT_A8 ||
			    id == V3D_TEX_FORMAT_L8A8 || id == V3D_TEX_FORMAT_R32F;
		if (want) {
			formats_supported += got;
			snprintf(what, sizeof(what),
				 "format %s: ours type %u cpp %u swizzle %u%u%u%u return %u, "
				 "Mesa's %s %u %u %u%u%u%u %u",
				 name, f.type, f.cpp, f.swizzle[0], f.swizzle[1], f.swizzle[2],
				 f.swizzle[3], f.return_size, m.in_table ? "row" : "(no row)", m.type,
				 m.cpp, m.swizzle[0], m.swizzle[1], m.swizzle[2], m.swizzle[3],
				 m.return_size);
			check(got && m.in_table && f.type == m.type && f.cpp == m.cpp &&
				      memcmp(f.swizzle, m.swizzle, 4) == 0 &&
				      f.return_size == m.return_size,
			      what);
		} else {
			snprintf(what, sizeof(what), "format %s: refused by name, out untouched", name);
			check(!got && memcmp(&f, &untouched, sizeof(f)) == 0, what);
			if (m.in_table)
				printf("tex_check: note: %s is refused; Mesa samples it as type %u, "
				       "swizzle %u%u%u%u, return %u, %u byte(s) a texel\n",
				       name, m.type, m.swizzle[0], m.swizzle[1], m.swizzle[2],
				       m.swizzle[3], m.return_size, m.cpp);
			else
				printf("tex_check: note: %s is refused; Mesa's V3D 4.2 table has no "
				       "row for it (%u byte(s) a texel)\n",
				       name, m.cpp);
		}
	}
	bool distinct = true;
	for (int a = 0; a < V3D_TEX_FORMAT_COUNT; a++)
		for (int b = a + 1; b < V3D_TEX_FORMAT_COUNT; b++)
			if (strcmp(names[a], names[b]) == 0)
				distinct = false;
	check(distinct, "every format has its own name");
	struct v3d_tex_format f;
	check(!v3d_tex_format_get(V3D_TEX_FORMAT_COUNT, &f) &&
		      strcmp(v3d_tex_format_name(V3D_TEX_FORMAT_COUNT), "unknown") == 0,
	      "an id out of range is refused and named unknown");
}

/* ---- 6. every GL ES 2.0 sampler against Mesa's rule ----------------------- */

/* v3d_upload_sampler_state_variant (gallium/drivers/v3d/v3dx_state.c:586-620)
 * and translate_wrap (:567-583), for the sampler a GL ES 2.0 state tracker
 * hands it: no anisotropy, no compare, the default LOD range -1000..1000, no
 * bias, the 0000 border variant. */
enum { MIP_NONE, MIP_NEAREST, MIP_LINEAR };
static const uint32_t gl_wraps[3] = { V3D_WRAP_MODE_REPEAT, V3D_WRAP_MODE_CLAMP,
				      V3D_WRAP_MODE_MIRROR };
static const char *const gl_wrap_names[3] = { "REPEAT", "CLAMP_TO_EDGE", "MIRRORED_REPEAT" };

static float max_f(float a, float b)
{
	return a > b ? a : b;
}

static float min_f(float a, float b)
{
	return a < b ? a : b;
}

static struct V3D42_SAMPLER_STATE mesa_sampler(bool min_nearest, bool mag_nearest, int mip,
					       uint32_t wrap_s, uint32_t wrap_t)
{
	const float min_lod = -1000.0f, max_lod = 1000.0f;
	struct V3D42_SAMPLER_STATE s = { 0 };
	s.wrap_s = wrap_s;
	s.wrap_t = wrap_t;
	s.wrap_r = V3D_WRAP_MODE_REPEAT; /* GL's TEXTURE_WRAP_R default */
	s.fixed_bias = 0.0f;
	s.min_filter_nearest = min_nearest;
	s.mag_filter_nearest = mag_nearest;
	s.mip_filter_nearest = mip != MIP_LINEAR;
	s.min_level_of_detail = min_f(max_f(0, min_lod), 15);
	s.max_level_of_detail = min_f(max_f(max_lod, min_lod), 15);
	if (mip == MIP_NONE) {
		s.min_level_of_detail = min_f(s.min_level_of_detail, 1.0f / 256.0f);
		s.max_level_of_detail = min_f(s.max_level_of_detail, 1.0f / 256.0f);
	}
	s.border_color_mode = V3D_BORDER_COLOR_0000;
	return s;
}

static struct v3d_sampler our_sampler(bool min_nearest, bool mag_nearest, int mip,
				      uint32_t wrap_s, uint32_t wrap_t)
{
	struct v3d_sampler s = { .wrap_s = wrap_s,
				 .wrap_t = wrap_t,
				 .wrap_r = V3D_WRAP_MODE_REPEAT,
				 .min_nearest = min_nearest,
				 .mag_nearest = mag_nearest,
				 .mip_nearest = mip != MIP_LINEAR,
				 .mip_none = mip == MIP_NONE,
				 .min_lod = -1000.0f,
				 .max_lod = 1000.0f,
				 .border_color_mode = V3D_BORDER_COLOR_0000 };
	return s;
}

static unsigned samplers_checked;

static void gl_sampler_case(const char *label, bool min_nearest, bool mag_nearest, int mip,
			    int ws, int wt)
{
	char name[160];
	snprintf(name, sizeof(name), "%s min %s mip %s, mag %s, wrap %s/%s", label,
		 min_nearest ? "NEAREST" : "LINEAR",
		 mip == MIP_NONE ? "none" : (mip == MIP_NEAREST ? "NEAREST" : "LINEAR"),
		 mag_nearest ? "NEAREST" : "LINEAR", gl_wrap_names[ws], gl_wrap_names[wt]);
	struct v3d_sampler ours = our_sampler(min_nearest, mag_nearest, mip, gl_wraps[ws],
					      gl_wraps[wt]);
	struct V3D42_SAMPLER_STATE m = mesa_sampler(min_nearest, mag_nearest, mip, gl_wraps[ws],
						    gl_wraps[wt]);
	sampler_case(name, &ours, &m);
	samplers_checked++;
}

static void part_gl_samplers(void)
{
	/* the six GL ES 2.0 minifying filters x the two magnifying x 3 x 3 wraps */
	for (int mn = 0; mn < 2; mn++)
		for (int mip = 0; mip < 3; mip++)
			for (int mg = 0; mg < 2; mg++)
				for (int ws = 0; ws < 3; ws++)
					for (int wt = 0; wt < 3; wt++)
						gl_sampler_case("ES2", mn == 0, mg == 0, mip, ws, wt);
	/* The Slint path's own, by name: femtovg 0.26 and Slint patched for
	 * no_std (driver/samples/zephyr/slint/nostd/). A Text item is an RGBA8
	 * texture drawn with ImageRendering::Smooth, CLAMP (slint-nostd.patch
	 * :6872-6905), like a smooth image; a pixelated image is NEAREST; the
	 * gradient texture repeats on T; a tiled image repeats on both. No mips
	 * anywhere. */
	gl_sampler_case("Slint pixelated image:", true, true, MIP_NONE, 1, 1);
	gl_sampler_case("Slint text / smooth image:", false, false, MIP_NONE, 1, 1);
	gl_sampler_case("femtovg gradient (REPEAT_Y):", false, false, MIP_NONE, 1, 0);
	gl_sampler_case("femtovg tiled image:", false, false, MIP_NONE, 0, 0);
}

/* ---- 7. negative controls: each must be DETECTED ------------------------- */

static void part_controls(void)
{
	struct v3d_tex_layout l;
	uint8_t *a, *b;

	/* (a) level 0 of 256x256 x9 is UIF XOR; tiled as UIF without XOR it
	 * must not match Mesa's bytes */
	v3d_tex_layout(256, 256, 9, 4, &l);
	a = calloc(1, l.size);
	b = calloc(1, l.size);
	uint8_t *src = malloc(256 * 4 * 256);
	for (uint32_t i = 0; i < 256 * 4 * 256; i++)
		src[i] = (uint8_t)rng();
	const struct v3d_tex_slice *s0 = &l.slices[0];
	for (uint32_t y = 0; y < 256; y++)
		for (uint32_t x = 0; x < 256; x++)
			memcpy(a + s0->offset +
				       v3d_tex_pixel_offset(V3D_TEX_TILING_UIF_NO_XOR, 4,
							    s0->padded_height, x, y),
			       src + (y * 256 + x) * 4, 4);
	struct pipe_box full = { .x = 0, .y = 0, .z = 0, .width = 256, .height = 256, .depth = 1 };
	v3d_store_tiled_image(b + s0->offset, s0->stride, src, 256 * 4,
			      (enum v3d_tiling_mode)s0->tiling, 4, s0->padded_height, &full);
	check(s0->tiling == V3D_TEX_TILING_UIF_XOR && memcmp(a, b, l.size) != 0,
	      "control: UIF tiled without XOR where the layout says XOR is detected");

	/* (b) a box stored one texel to the right of Mesa's */
	memset(a, 0, l.size);
	memset(b, 0, l.size);
	struct pipe_box pb = { .x = 40, .y = 9, .z = 0, .width = 7, .height = 5, .depth = 1 };
	bool stored = v3d_tex_store_box(a, &l, 0, 41, 9, 7, 5, src, 7 * 4);
	v3d_store_tiled_image(b + s0->offset, s0->stride, src, 7 * 4,
			      (enum v3d_tiling_mode)s0->tiling, 4, s0->padded_height, &pb);
	check(stored && memcmp(a, b, l.size) != 0, "control: a box one texel off is detected");
	free(src);
	free(a);
	free(b);

	/* (c) one bit flipped in a packed texture state that matched Mesa's */
	struct v3d_tex_state st = { .base = 0x00400000u, .width = 64, .height = 64, .depth = 1,
				    .type = V3D_TEXTURE_DATA_FORMAT_RGBA8,
				    .swizzle = { 2, 3, 4, 5 } };
	v3d_tex_layout(64, 64, 1, 4, &l);
	v3d_tex_state_from_layout(&st, &l);
	uint8_t ours[V3D_TEX_STATE_BYTES], mesa[V3D_TEX_STATE_BYTES];
	bool st_packed = v3d_tex_pack_state(ours, &st);
	struct V3D42_TEXTURE_SHADER_STATE t = { 0 };
	t.image_width = 64;
	t.image_height = 64;
	t.image_depth = 1;
	t.texture_base_pointer = 0x00400000u;
	t.array_stride_64_byte_aligned = st.array_stride_64;
	t.level_0_is_strictly_uif = true;
	t.extended = true;
	t.swizzle_r = 2;
	t.swizzle_g = 3;
	t.swizzle_b = 4;
	t.swizzle_a = 5;
	t.texture_type = TEXTURE_DATA_FORMAT_RGBA8;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, mesa, &t);
	bool same_before = st_packed && memcmp(ours, mesa, sizeof(ours)) == 0;
	ours[12] ^= 0x10;
	check(same_before && memcmp(ours, mesa, sizeof(ours)) != 0,
	      "control: one flipped bit in a texture state is detected");

	/* (d) wrap S and T swapped in a sampler whose two wraps differ */
	uint8_t so[V3D_SAMPLER_STATE_BYTES], sm[V3D_SAMPLER_STATE_BYTES];
	struct v3d_sampler sw = our_sampler(false, false, MIP_NONE, V3D_WRAP_MODE_REPEAT,
					    V3D_WRAP_MODE_CLAMP);
	struct V3D42_SAMPLER_STATE ms = mesa_sampler(false, false, MIP_NONE, V3D_WRAP_MODE_CLAMP,
						     V3D_WRAP_MODE_REPEAT);
	bool sw_packed = v3d_tex_pack_sampler(so, &sw);
	V3D42_SAMPLER_STATE_pack(NULL, sm, &ms);
	check(sw_packed && memcmp(so, sm, sizeof(so)) != 0, "control: swapped S/T wraps are detected");

	/* (e) BGRA8's row against Mesa's RGBA8 row */
	struct v3d_tex_format f;
	struct mesa_fmt m;
	bool have = v3d_tex_format_get(V3D_TEX_FORMAT_BGRA8, &f) && mesa_v3d_format("RGBA8", &m);
	check(have && memcmp(f.swizzle, m.swizzle, 4) != 0,
	      "control: BGRA8's swizzle compared with RGBA8's is detected");
}

/* ---- 8. record refusals, and the P0/P1 uniforms against Mesa ------------- */

static void part_refusals_and_p0p1(void)
{
	char what[160];

	/* texture state: a valid record, then one field at a time out of range */
	struct v3d_tex_layout l;
	v3d_tex_layout(64, 64, 1, 4, &l);
	struct v3d_tex_state good = { .base = 0x00400000u, .width = 64, .height = 64, .depth = 1,
				      .type = V3D_TEXTURE_DATA_FORMAT_RGBA8,
				      .swizzle = { 2, 3, 4, 5 } };
	v3d_tex_state_from_layout(&good, &l);
	uint8_t out[V3D_TEX_STATE_BYTES], sentinel[V3D_TEX_STATE_BYTES];
	check(v3d_tex_pack_state(out, &good), "texture state: the valid record is packed");
	static const char *const bad_state_names[] = {
		"type 1 (R8 SNORM, no supported format has it)", "type 127", "a base not 64-byte aligned",
		"width 0", "width 16384", "height 16384", "depth 0", "max level 16",
		"base level above max level", "swizzle 6", "ub_pad 16", "array stride 2^26",
	};
	for (unsigned i = 0; i < sizeof(bad_state_names) / sizeof(bad_state_names[0]); i++) {
		struct v3d_tex_state s = good;
		switch (i) {
		case 0: s.type = 1; break;
		case 1: s.type = 127; break;
		case 2: s.base += 32; break;
		case 3: s.width = 0; break;
		case 4: s.width = 16384; break;
		case 5: s.height = 16384; break;
		case 6: s.depth = 0; break;
		case 7: s.max_level = 16; break;
		case 8: s.max_level = 2; s.base_level = 3; break;
		case 9: s.swizzle[2] = 6; break;
		case 10: s.level_0_ub_pad = 16; break;
		default: s.array_stride_64 = 1u << 26; break;
		}
		memset(out, 0xcc, sizeof(out));
		memset(sentinel, 0xcc, sizeof(sentinel));
		snprintf(what, sizeof(what), "texture state: %s is refused, nothing written",
			 bad_state_names[i]);
		check(!v3d_tex_pack_state(out, &s) && memcmp(out, sentinel, sizeof(out)) == 0, what);
	}

	/* sampler state: the bias field's two ends are Mesa's bytes; past them,
	 * and every value the packer cannot write, is refused */
	uint8_t so[V3D_SAMPLER_STATE_BYTES], sm[V3D_SAMPLER_STATE_BYTES];
	static const float edge_bias[2] = { -128.0f, 127.99609375f };
	for (int i = 0; i < 2; i++) {
		struct v3d_sampler s = { .min_lod = -1000.0f, .max_lod = 1000.0f,
					 .lod_bias = edge_bias[i] };
		struct V3D42_SAMPLER_STATE m = { .min_level_of_detail = 0.0f,
						 .max_level_of_detail = 15.0f,
						 .fixed_bias = edge_bias[i] };
		bool packed = v3d_tex_pack_sampler(so, &s);
		V3D42_SAMPLER_STATE_pack(NULL, sm, &m);
		snprintf(what, sizeof(what), "sampler state: bias %g packs as Mesa's",
			 (double)edge_bias[i]);
		check(packed && memcmp(so, sm, sizeof(so)) == 0, what);
	}
	static const char *const bad_sampler_names[] = {
		"wrap S BORDER (3)", "wrap T MIRROR_ONCE (4)", "wrap R 7", "border colour FOLLOWS (7)",
		"a NaN min LOD", "a NaN bias", "bias 128", "bias -128.01", "bias 1e30",
	};
	for (unsigned i = 0; i < sizeof(bad_sampler_names) / sizeof(bad_sampler_names[0]); i++) {
		struct v3d_sampler s = { .min_lod = -1000.0f, .max_lod = 1000.0f };
		volatile float zero = 0.0f;
		switch (i) {
		case 0: s.wrap_s = 3; break;
		case 1: s.wrap_t = 4; break;
		case 2: s.wrap_r = 7; break;
		case 3: s.border_color_mode = 7; break;
		case 4: s.min_lod = zero / zero; break;
		case 5: s.lod_bias = zero / zero; break;
		case 6: s.lod_bias = 128.0f; break;
		case 7: s.lod_bias = -128.01f; break;
		default: s.lod_bias = 1e30f; break;
		}
		uint8_t sen[V3D_SAMPLER_STATE_BYTES];
		memset(so, 0xcc, sizeof(so));
		memset(sen, 0xcc, sizeof(sen));
		snprintf(what, sizeof(what), "sampler state: %s is refused, nothing written",
			 bad_sampler_names[i]);
		check(!v3d_tex_pack_sampler(so, &s) && memcmp(so, sen, sizeof(so)) == 0, what);
	}

	/* P0 and P1 against V3D42_TMU_CONFIG_PARAMETER_0/1_pack: every payload
	 * the fields hold, at three record addresses, with a binding in the top
	 * byte of the uniform data (it must not reach the word) */
	static const uint32_t addrs[3] = { 0x00000020u, 0x00400000u, 0xffffffe0u };
	unsigned p0_cases = 0, p0_bad = 0, p1_cases = 0, p1_bad = 0;
	for (int a = 0; a < 3; a++) {
		for (uint32_t words = 0; words < 16; words++) {
			struct V3D42_TMU_CONFIG_PARAMETER_0 m = { .texture_state_address = addrs[a],
								  .return_words_of_texture_data = words };
			uint8_t b[4];
			V3D42_TMU_CONFIG_PARAMETER_0_pack(NULL, b, &m);
			uint32_t want = b[0] | b[1] << 8 | b[2] << 16 | (uint32_t)b[3] << 24, got = 0;
			bool ok = v3d_tex_p0(addrs[a], (uint32_t)(a * 5 + 1) << 24 | words, &got);
			p0_cases++;
			p0_bad += !(ok && got == want);
		}
		for (uint32_t bits = 0; bits < 8; bits++) {
			struct V3D42_TMU_CONFIG_PARAMETER_1 m = {
				.sampler_state_address = addrs[a],
				.output_type_32_bit = bits & 1,
				.unnormalized_coordinates = (bits >> 1) & 1,
				.per_pixel_mask_enable = (bits >> 2) & 1,
			};
			uint8_t b[4];
			V3D42_TMU_CONFIG_PARAMETER_1_pack(NULL, b, &m);
			uint32_t want = b[0] | b[1] << 8 | b[2] << 16 | (uint32_t)b[3] << 24, got = 0;
			bool ok = v3d_tex_p1(addrs[a], (uint32_t)(a * 5 + 2) << 24 | bits, &got);
			p1_cases++;
			p1_bad += !(ok && got == want);
		}
	}
	snprintf(what, sizeof(what), "P0: %u of %u words are Mesa's (planned 48)",
		 p0_cases - p0_bad, p0_cases);
	check(p0_cases == 48 && p0_bad == 0, what);
	snprintf(what, sizeof(what), "P1: %u of %u words are Mesa's (planned 24)",
		 p1_cases - p1_bad, p1_cases);
	check(p1_cases == 24 && p1_bad == 0, what);
	uint32_t w = 0x12345678u;
	check(!v3d_tex_p0(0x00400010u, 3, &w) && w == 0x12345678u,
	      "P0: a record address not 32-byte aligned is refused");
	w = 0x12345678u;
	check(!v3d_tex_p0(0x00400000u, 0x10, &w) && w == 0x12345678u,
	      "P0: a payload bit above the return-words field is refused");
	w = 0x12345678u;
	check(!v3d_tex_p1(0x00400000u, 0x8, &w) && w == 0x12345678u,
	      "P1: a payload bit above the three flag bits is refused");
}

/* ---- 9. layered images: cube maps ---------------------------------------- */

/* Mesa's cube_map_stride (v3d_resource.c:762-763), restated here from the
 * slices so the per-face tiling below places Mesa's faces by Mesa's rule */
static uint32_t mesa_layer_stride(const struct v3d_tex_layout *l)
{
	return (l->slices[0].offset + l->slices[0].size + 63u) & ~63u;
}

static void part_layers(void)
{
	char what[200];
	struct v3d_tex_layout l;
	uint32_t size = 0, off = 0;

	/* by hand, with Mesa's rules (RGBA8):
	 * 1x1 x1: one LT utile, 64 bytes at 0: stride 64, 6 layers 64 + 5 * 64.
	 * 64x64 x7: levels 6..1 at 0, 64, 128, 192 (UB1), 448 (UB2), 1472 (UIF,
	 *   4096), level 0 16384 at 5568; 21952 in all, level 0 moved to 8192
	 *   (+2624): level 0 ends at 24576 = the stride; 6 layers 147456; level 2
	 *   (448 + 2624 = 3072) of layer 5: 3072 + 5 * 24576 = 125952.
	 * 256x256 x9 (part 1): level 0 262144 at 90112, ends at 352256 = the
	 *   stride; 6 layers 2113536; level 0 of layer 1: 90112 + 352256. */
	v3d_tex_layout(1, 1, 1, 4, &l);
	check(v3d_tex_layer_stride(&l) == 64 && v3d_tex_layers_size(&l, 6, &size) && size == 384,
	      "cube 1x1 x1: stride 64, 6 layers 384 bytes");
	v3d_tex_layout(64, 64, 7, 4, &l);
	snprintf(what, sizeof(what), "cube 64x64 x7: stride %u (24576), size %u (147456)",
		 v3d_tex_layer_stride(&l), v3d_tex_layers_size(&l, 6, &size) ? size : 0);
	check(v3d_tex_layer_stride(&l) == 24576 && v3d_tex_layers_size(&l, 6, &size) &&
		      size == 147456 && v3d_tex_layer_offset(&l, 2, 5, 6, &off) && off == 125952,
	      what);
	v3d_tex_layout(256, 256, 9, 4, &l);
	check(v3d_tex_layer_stride(&l) == 352256 && v3d_tex_layers_size(&l, 6, &size) &&
		      size == 2113536 && v3d_tex_layer_offset(&l, 0, 1, 6, &off) &&
		      off == 90112 + 352256,
	      "cube 256x256 x9: stride 352256, 6 layers 2113536, layer 1 level 0 at 442368");

	/* every face and level tiled by us at image + face * stride, against
	 * Mesa's v3d_store_tiled_image at its slice offset + face * its stride */
	static const uint32_t cubes[][2] = { { 1, 1 }, { 15, 1 }, { 16, 5 }, { 64, 7 } };
	for (unsigned c = 0; c < sizeof(cubes) / sizeof(cubes[0]); c++) {
		uint32_t n = cubes[c][0], levels = cubes[c][1];
		v3d_tex_layout(n, n, levels, 4, &l);
		uint32_t stride = v3d_tex_layer_stride(&l), mstride = mesa_layer_stride(&l);
		/* the buffers are sized by Mesa's rule (rsc->size, :764), never by
		 * the function under test, and nothing is tiled when our stride or
		 * size disagrees: a wrong answer fails the check, it cannot
		 * overrun the buffer */
		uint32_t msize = l.size + mstride * 5;
		bool sized = v3d_tex_layers_size(&l, 6, &size) && size == msize && stride == mstride;
		if (!sized) {
			snprintf(what, sizeof(what),
				 "cube %ux%u x%u: stride %u size %u, Mesa's rule gives %u and %u", n, n,
				 levels, stride, size, mstride, msize);
			check(false, what);
			continue;
		}
		uint8_t *ours = calloc(1, msize), *mesa = calloc(1, msize);
		for (uint32_t f = 0; f < 6; f++)
			for (uint32_t lv = 0; lv < levels; lv++) {
				uint32_t w = n >> lv ? n >> lv : 1;
				uint8_t *src = malloc((size_t)w * w * 4);
				for (uint32_t i = 0; i < w * w * 4; i++)
					src[i] = (uint8_t)rng();
				v3d_tex_store_level(ours + (size_t)f * stride, &l, lv, src, w * 4);
				struct pipe_box box = { .x = 0, .y = 0, .z = 0, .width = (int32_t)w,
							.height = (int32_t)w, .depth = 1 };
				const struct v3d_tex_slice *s = &l.slices[lv];
				v3d_store_tiled_image(mesa + s->offset + (size_t)f * mstride, s->stride, src,
						      w * 4, (enum v3d_tiling_mode)s->tiling, 4,
						      s->padded_height, &box);
				free(src);
			}
		snprintf(what, sizeof(what),
			 "cube %ux%u x%u: 6 faces x %u levels, our bytes are Mesa's (%u bytes)", n, n,
			 levels, levels, msize);
		check(memcmp(ours, mesa, msize) == 0, what);
		free(ours);
		free(mesa);
	}

	/* the record of a cube: depth 6 (the layer count), array stride = the
	 * layer stride / 64 = 384 for 64x64 x7, against Mesa's packer */
	v3d_tex_layout(64, 64, 7, 4, &l);
	struct v3d_tex_state st = { .base = 0x00400000u + l.slices[0].offset, .width = 64,
				    .height = 64, .depth = 6, .max_level = 6,
				    .type = V3D_TEXTURE_DATA_FORMAT_RGBA8, .swizzle = { 2, 3, 4, 5 } };
	v3d_tex_state_from_layout(&st, &l);
	uint8_t o[V3D_TEX_STATE_BYTES], m[V3D_TEX_STATE_BYTES];
	bool packed = v3d_tex_pack_state(o, &st);
	struct V3D42_TEXTURE_SHADER_STATE t = { 0 };
	t.image_width = 64;
	t.image_height = 64;
	t.image_depth = 6;
	t.max_level = 6;
	t.texture_base_pointer = 0x00400000u + l.slices[0].offset;
	t.array_stride_64_byte_aligned = 24576 / 64;
	t.level_0_is_strictly_uif = true;
	t.extended = true;
	t.swizzle_r = 2;
	t.swizzle_g = 3;
	t.swizzle_b = 4;
	t.swizzle_a = 5;
	t.texture_type = TEXTURE_DATA_FORMAT_RGBA8;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, m, &t);
	check(packed && st.array_stride_64 == 384 && memcmp(o, m, sizeof(o)) == 0,
	      "cube record 64x64 x7: depth 6, array stride 384, our bytes are Mesa's");

	/* the refusals: *out untouched */
	uint32_t sentinel = 0x12345678u, v = sentinel;
	check(!v3d_tex_layer_offset(&l, 0, 6, 6, &v) && v == sentinel,
	      "layer offset: layer 6 of 6 is refused");
	v = sentinel;
	check(!v3d_tex_layer_offset(&l, 7, 0, 6, &v) && v == sentinel,
	      "layer offset: level 7 of 7 is refused");
	v = sentinel;
	check(!v3d_tex_layers_size(&l, 0, &v) && v == sentinel, "layers size: 0 layers is refused");
	v = sentinel;
	check(!v3d_tex_layers_size(&l, 0x100000u, &v) && v == sentinel,
	      "layers size: a size past 32 bits is refused");
}

int main(void)
{
	/* part 1: 9 cases (2 + levels each: 3,3,3,3,3,3,11,4,5) + 4 refusals
	 * + 5 for the 4x layout;
	 * part 2: 1 + 13 sizes x 5 cpp; part 3: 1 + 8 + 3;
	 * part 4: 9 sizes x 5 cpp x 2 (store, load) + 8 refusals x 2; part 5: 10 formats x 2 + 2;
	 * part 6: 2 x 3 x 2 x 3 x 3 + 4 femtovg; part 7: 5 controls;
	 * part 8: 1 + 12 state refusals + 2 bias edges + 9 sampler refusals +
	 * P0 + P1 + 3 P0/P1 refusals; part 9: 3 hand cube layouts + 4 cubes
	 * tiled + 1 cube record + 4 refusals; and the 3 population checks below */
	const int part1 = (3 + 3 + 3 + 3 + 3 + 3) + (2 + 9) + (2 + 2) + (2 + 3) + 4 + 5;
	const int planned = part1 + (1 + 13 * 5) + (1 + 8 + 3) + (9 * 5 * 2 + 8 * 2) +
			    (V3D_TEX_FORMAT_COUNT * 2 + 2) + (108 + 4) + 5 +
			    (1 + 12 + 2 + 9 + 2 + 3) + (3 + 4 + 1 + 4) + 3;
	/* the boxes part 4 must have stored: levels 1+1+3+6+1+1+9+2+2 = 26 of
	 * the 9 sizes, x 5 cpp, x (7 fixed + 16 random) boxes a level */
	const unsigned planned_boxes = 26 * 5 * (FIXED_BOXES + RANDOM_BOXES);
	part_layout();
	part_tiling();
	part_records();
	part_boxes();
	part_formats();
	part_gl_samplers();
	part_controls();
	part_refusals_and_p0p1();
	part_layers();

	char what[160];
	snprintf(what, sizeof(what), "population: %u boxes stored, planned %u", boxes_stored,
		 planned_boxes);
	check(boxes_stored == planned_boxes, what);
	snprintf(what, sizeof(what),
		 "population: %u formats supported, planned 6 (RGBA8, BGRA8, L8, A8, L8A8, R32F)",
		 formats_supported);
	check(formats_supported == 6, what);
	snprintf(what, sizeof(what), "population: %u GL samplers packed, planned 112",
		 samplers_checked);
	check(samplers_checked == 112, what);

	printf("tex_check: %u sub-image boxes stored, %llu image bytes compared with Mesa's\n",
	       boxes_stored, (unsigned long long)box_bytes_compared);
	printf("tex_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
