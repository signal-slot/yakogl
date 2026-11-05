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
 *     supported rows equal Mesa's (with their block and sRGB-ness), every
 *     other format is refused by name.
 *  6. Every GL ES 2.0 sampler (6 min x 2 mag filters x 3 x 3 wraps) and
 *     femtovg's four named ones against V3D42_SAMPLER_STATE_pack of what
 *     Mesa's v3d_upload_sampler_state_variant fills.
 *  7. Negative controls, each of which must be detected: XOR dropped, a box
 *     one texel off, a flipped record bit, swapped wraps, a wrong swizzle.
 *  8. The packers' refusals (a type no supported format has, every field
 *     that does not fit; nothing is masked) and the TMU P0/P1 uniform words
 *     against V3D42_TMU_CONFIG_PARAMETER_0/1_pack.
 *  9. Layered images (cube maps): the layer stride, size and offsets by
 *     Mesa's cube_map_stride rule, every face x level tiled at Mesa's
 *     offsets, the depth-6 record.
 * 10. The TFU mipmap job (v3d_tex_tfu_pack_mipmap), field by field.
 * 11. 2D arrays (the cube path with the layer count: layouts, every layer x
 *     level tiled, the depth-N record) and 3D textures (v3d_tex_layout_3d:
 *     the per-level depth and the slice stacking against numbers derived by
 *     hand from v3d_setup_slices, every slice x level tiled at
 *     v3d_layer_offset's place, the record with level 0's slice stride, a
 *     control, the refusals; docs/notes/v3d-tex-layout-es3.md).
 * 12. ETC2 and EAC (v3d_tex_layout_compressed): the block-aware layouts
 *     derived by hand from v3d_setup_slices (odd, non-power-of-two and 1x1
 *     sizes, a full chain, ub_pad, XOR), a control without the block-aware
 *     padding, each of the ten formats tiled at every level against Mesa's
 *     v3d_store_tiled_image with Mesa's block counts, block boxes stored and
 *     loaded, each format's record (type, swizzle, sRGB bit) against Mesa's
 *     row and packer, and the refusals: the texel entry points and the TFU
 *     write nothing for a compressed layout.
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
	/* The render images (src/v3d/backend.rs RtImage: a render target the
	 * TMU samples as it is, stored by the TLB), at the sizes the Pi 4
	 * benchmarks render into, sampled at one level. 1600x1200, glmark2
	 * refract's colour target: 1600 is 50 columns of 4 UIF blocks (32
	 * texels), so no width pad; 1200 is 150 UIF block rows (8 rows each),
	 * 150 % 32 = 22, which is neither under 6 nor over 26: no ub_pad, and no
	 * XOR; stride 6400, 7,680,000 bytes -- the 7,684,096 of RESULTS'
	 * tiled-copy lines less the backend's 4 KiB of alignment slack. 800x600,
	 * glmark2's canvas and terrain's targets: 800 is 25 columns, 600 is 75
	 * rows (75 % 32 = 11): no pad, no XOR, stride 3200, 1,920,000 bytes.
	 * Refract's 11-level chain image is pinned by tests/v3d_rt_sample.rs. */
	layout_case("1600x1200 (refract's render target)", 1600, 1200, 1, 7680000,
		    (struct want_slice[]){ { 0, 6400, 1200, 0, UIF } });
	layout_case("800x600 (terrain's render targets)", 800, 600, 1, 1920000,
		    (struct want_slice[]){ { 0, 3200, 600, 0, UIF } });

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
 * format our table or Mesa's lacks fails the check (nothing is packed).
 * `srgb`: the record's sRGB bit, as v3dx_state.c:1023-1025 sets it for an
 * sRGB view (its texels the RGBA8 row's). */
static void state_case_srgb(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t base,
			    enum v3d_tex_format_id id, bool srgb)
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
		.srgb = srgb,
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
	t.srgb = srgb;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, mesa, &t);

	snprintf(what, sizeof(what), "texture state %s: our bytes are Mesa's", name);
	check(packed && memcmp(ours, mesa, sizeof(ours)) == 0, what);
	if (memcmp(ours, mesa, sizeof(ours)) != 0)
		for (int i = 0; i < V3D_TEX_STATE_BYTES; i++)
			printf("  %2d: ours %02x mesa %02x\n", i, ours[i], mesa[i]);
}

static void state_case(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t base,
		       enum v3d_tex_format_id id)
{
	state_case_srgb(name, w, h, levels, base, id, false);
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
	/* the sRGB class: the RGBA8 row with the sRGB bit, which sits in the
	 * base pointer's flag bits (bit 3) */
	state_case_srgb("64x64 x7 RGBA8 sRGB", 64, 64, 7, 0x00610000u, V3D_TEX_FORMAT_RGBA8, true);
	/* the ES 3.0 classes: cpp 2, 4, 8 and 16 layouts, integer and 32-bit types */
	state_case("13x7 x3 R16F", 13, 7, 3, 0x00620000u, V3D_TEX_FORMAT_R16F);
	state_case("64x32 x6 RGB9E5", 64, 32, 6, 0x00630000u, V3D_TEX_FORMAT_RGB9E5);
	state_case("256x256 x9 RGBA16F (UIF at level 0)", 256, 256, 9, 0x00700000u, V3D_TEX_FORMAT_RGBA16F);
	state_case("16x16 RGBA16UI", 16, 16, 1, 0x00800000u, V3D_TEX_FORMAT_RGBA16UI);
	state_case("64x64 x7 RGBA32F", 64, 64, 7, 0x00810000u, V3D_TEX_FORMAT_RGBA32F);
	state_case("13x7 RGBA32I", 13, 7, 1, 0x00900000u, V3D_TEX_FORMAT_RGBA32I);

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
		/* all but RGB8 (no 3-byte type), RGBA4 and RGB5_A1 (the GL core
		 * stores both as RGBA8) */
		bool want = id != V3D_TEX_FORMAT_RGB8 && id != V3D_TEX_FORMAT_RGBA4 &&
			    id != V3D_TEX_FORMAT_RGB5_A1;
		if (want) {
			formats_supported += got;
			snprintf(what, sizeof(what),
				 "format %s: ours type %u cpp %u swizzle %u%u%u%u return %u (and the "
				 "block, sRGB), Mesa's %s %u %u %u%u%u%u %u",
				 name, f.type, f.cpp, f.swizzle[0], f.swizzle[1], f.swizzle[2],
				 f.swizzle[3], f.return_size, m.in_table ? "row" : "(no row)", m.type,
				 m.cpp, m.swizzle[0], m.swizzle[1], m.swizzle[2], m.swizzle[3],
				 m.return_size);
			uint32_t bw = 0, bh = 0;
			bool block = v3d_tex_format_block((enum v3d_tex_format_id)id, &bw, &bh) &&
				     bw == m.block_w && bh == m.block_h &&
				     v3d_tex_format_is_srgb((enum v3d_tex_format_id)id) == m.srgb;
			check(got && m.in_table && f.type == m.type && f.cpp == m.cpp &&
				      memcmp(f.swizzle, m.swizzle, 4) == 0 &&
				      f.return_size == m.return_size && block,
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
	 * no_std (examples/nostd/). A Text item is an RGBA8
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

/* ---- part 10: the TFU mipmap job (v3d_tex_tfu_pack_mipmap) ---------------- */

/* The job that makes levels 1.. of a render image from its level 0 (the
 * V3D backend's v3d_generate_mipmap, Mesa's v3d_generate_mipmap through
 * v3dX(tfu) with for_mipmap): each field against an independent source --
 * the ICFG format against the layout's level 0 tiling (Mesa's
 * V3D33_TFU_ICFG_FORMAT_LINEARTILE 11 + the tiling's distance from
 * LINEARTILE), IIS against the padded height in UIF blocks, NUMMM against
 * the level count, OPAD against the layout's padding rows (v3dx_tfu.c:154-162
 * sets it for every UIF destination), IOA's format
 * (V3D33_TFU_IOA_FORMAT_LINEARTILE 3 + the distance) and DIMTW -- and the
 * refusals. */
static void mip_case(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t icfg_fmt,
		     uint32_t ioa_fmt)
{
	struct v3d_tex_layout l;
	struct v3d_tex_tfu_job j = { 0 };
	const uint32_t image = 0x01000000u;
	bool ok = v3d_tex_layout(w, h, levels, 4, &l) && v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, image, &j);
	char what[160];
	snprintf(what, sizeof(what), "mipmap job %s: packed", name);
	check(ok, what);
	const struct v3d_tex_slice *s = &l.slices[0];
	const uint32_t level0 = image + s->offset;
	const bool uif = s->tiling == V3D_TEX_TILING_UIF_NO_XOR || s->tiling == V3D_TEX_TILING_UIF_XOR;
	/* RGBA8: utiles 4 rows high, UIF blocks 8 */
	const uint32_t opad = uif ? (s->padded_height - (h + 7) / 8 * 8) / 8 : 0;
	snprintf(what, sizeof(what), "mipmap job %s: icfg format %u, type RGBA8, %u levels after the first, OPAD %u",
		 name, icfg_fmt, levels - 1, opad);
	check(ok && j.icfg == (opad << 22 | icfg_fmt << 18 | V3D_TFU_TTYPE_RGBA8 << 9 | (levels - 1) << 5), what);
	snprintf(what, sizeof(what), "mipmap job %s: iia level 0, iis %u", name, uif ? s->padded_height / 8 : 0);
	check(ok && j.iia == level0 && j.iis == (uif ? s->padded_height / 8 : 0), what);
	snprintf(what, sizeof(what), "mipmap job %s: ioa level 0 | format %u << 3 | DIMTW, ios h << 16 | w", name,
		 ioa_fmt);
	check(ok && j.ioa == (level0 | ioa_fmt << 3 | 1u) && j.ios == (h << 16 | w) && j.ica == 0 && j.iua == 0 &&
		      j.coef0 == 0,
	      what);
}

static void part_mipmap_job(void)
{
	/* refract's 1600x1200 x11 (UIF no XOR: 14 / 6), terrain's 800x600 x10,
	 * a 1024x1024 x11 whose level 0 Mesa lays out with the XOR (15 / 7: the
	 * render image drops it, backend.rs rt_layout, which the backend test
	 * pins; the packer takes the layout as given), the Pi 4 window's
	 * 1280x800 x11 whose level 0 Mesa pads by two UIF block rows (OPAD 2),
	 * a 16x16 x3 UB-linear level 0 (13 / 5, no height word) and a 4x4 x3 LT
	 * one (11 / 3) */
	mip_case("1600x1200 x11", 1600, 1200, 11, 14, 6);
	mip_case("800x600 x10", 800, 600, 10, 14, 6);
	mip_case("1024x1024 x11 (XOR as Mesa lays it out)", 1024, 1024, 11, 15, 7);
	mip_case("1280x800 x11 (two padding rows)", 1280, 800, 11, 14, 6);
	mip_case("16x16 x3", 16, 16, 3, 13, 5);
	mip_case("4x4 x3", 4, 4, 3, 11, 3);

	struct v3d_tex_layout l;
	struct v3d_tex_tfu_job j = { 0 };
	check(v3d_tex_layout(64, 64, 1, 4, &l) && !v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, 0x01000000u, &j),
	      "mipmap job: one level is refused");
	check(v3d_tex_layout(64, 64, 3, 4, &l) && !v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_R8, 0x01000000u, &j),
	      "mipmap job: a type of another texel size is refused");
	check(v3d_tex_layout(64, 64, 3, 1, &l) && v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_R8, 0x01000000u, &j) &&
		      (j.icfg >> 9 & 0x7f) == V3D_TFU_TTYPE_R8 && (j.icfg >> 5 & 0xf) == 2,
	      "mipmap job: an R8 layout takes the R8 type");
	const uint32_t packed_iia = j.iia;
	check(v3d_tex_layout(64, 64, 3, 4, &l) && !v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, 0x01000020u, &j),
	      "mipmap job: a level 0 off 64 bytes is refused");
	check(j.iia == packed_iia && packed_iia != 0, "mipmap job: a refusal leaves the job untouched");
}

/* ---- part 11: 2D arrays and 3D textures (docs/notes/v3d-tex-layout-es3.md) -- */

/* A 2D array is the cube path with another layer count (v3d_resource.c:
 * 757-764, :778-779): N whole mip trees cube_map_stride apart, the state's
 * image_depth the layer count (v3dx_state.c:937-938) and its array stride the
 * tree stride / 64 (:961). A 3D texture is v3d_tex_layout_3d: each level
 * holds level_depth slices (:609-611, :636-639) one slice size apart (:722,
 * :776-777), the state's image_depth is the level 0 depth (:935-936) and its
 * array stride level 0's slice size / 64 (:765-766, :961). Every number below
 * is derived by hand from v3d_setup_slices, as part 1's are. */

struct want_slice_3d {
	uint32_t offset, stride, padded_height, ub_pad;
	enum v3d_tex_tiling tiling;
	uint32_t depth; /* v3d_tex_level_depth */
};

static void layout3d_case(const char *name, uint32_t w, uint32_t h, uint32_t depth, uint32_t levels,
			  uint32_t size, const struct want_slice_3d *want)
{
	struct v3d_tex_layout l;
	char what[200];
	snprintf(what, sizeof(what), "3D %s: laid out", name);
	check(v3d_tex_layout_3d(w, h, depth, levels, 4, &l), what);
	snprintf(what, sizeof(what), "3D %s: size %u, want %u", name, l.size, size);
	check(l.size == size, what);
	for (uint32_t i = 0; i < levels; i++) {
		const struct v3d_tex_slice *s = &l.slices[i];
		const struct want_slice_3d *x = &want[i];
		uint32_t d = v3d_tex_level_depth(depth, i);
		snprintf(what, sizeof(what),
			 "3D %s level %u: offset %u stride %u h %u pad %u tiling %d depth %u, "
			 "want %u %u %u %u %d %u",
			 name, i, s->offset, s->stride, s->padded_height, s->ub_pad, s->tiling, d,
			 x->offset, x->stride, x->padded_height, x->ub_pad, x->tiling, x->depth);
		check(s->offset == x->offset && s->stride == x->stride &&
			      s->padded_height == x->padded_height && s->ub_pad == x->ub_pad &&
			      s->tiling == x->tiling && d == x->depth,
		      what);
	}
}

/* Every layer of every level of an N-layer array tiled by us at image +
 * layer * v3d_tex_layer_stride, against Mesa's v3d_store_tiled_image at its
 * slice offset + layer * cube_map_stride (v3d_layer_offset, :778-779; the
 * per-slice store with the same box is Mesa's own upload, v3d_resource.c:
 * 136-150 and :398-412, one z at a time). The buffer is sized by Mesa's rule
 * (:764), never by the function under test. */
static void array_tiling_case(uint32_t w, uint32_t h, uint32_t layers, uint32_t levels)
{
	char what[200];
	struct v3d_tex_layout l;
	uint32_t size = 0;
	v3d_tex_layout(w, h, levels, 4, &l);
	uint32_t stride = v3d_tex_layer_stride(&l), mstride = mesa_layer_stride(&l);
	uint32_t msize = l.size + mstride * (layers - 1);
	if (!(v3d_tex_layers_size(&l, layers, &size) && size == msize && stride == mstride)) {
		snprintf(what, sizeof(what), "array %ux%u x%u, %u layers: stride %u size %u, Mesa's rule gives %u and %u",
			 w, h, levels, layers, stride, size, mstride, msize);
		check(false, what);
		return;
	}
	uint8_t *ours = calloc(1, msize), *mesa = calloc(1, msize);
	for (uint32_t f = 0; f < layers; f++)
		for (uint32_t lv = 0; lv < levels; lv++) {
			uint32_t lw = w >> lv ? w >> lv : 1, lh = h >> lv ? h >> lv : 1;
			uint8_t *src = malloc((size_t)lw * lh * 4);
			for (uint32_t i = 0; i < lw * lh * 4; i++)
				src[i] = (uint8_t)rng();
			v3d_tex_store_level(ours + (size_t)f * stride, &l, lv, src, lw * 4);
			struct pipe_box box = { .x = 0, .y = 0, .z = 0, .width = (int32_t)lw,
						.height = (int32_t)lh, .depth = 1 };
			const struct v3d_tex_slice *s = &l.slices[lv];
			v3d_store_tiled_image(mesa + s->offset + (size_t)f * mstride, s->stride, src, lw * 4,
					      (enum v3d_tiling_mode)s->tiling, 4, s->padded_height, &box);
			free(src);
		}
	snprintf(what, sizeof(what), "array %ux%u x%u, %u layers: every layer x level, our bytes are Mesa's (%u bytes)",
		 w, h, levels, layers, msize);
	check(memcmp(ours, mesa, msize) == 0, what);
	free(ours);
	free(mesa);
}

/* Every slice Mesa allocates (level_depth, the padding slices included) of
 * every level of a 3D texture tiled by us at image + slice * slices[level].size
 * (v3d_tex_slice_offset's rule), against Mesa's v3d_store_tiled_image at
 * slices[level].offset + slice * slices[level].size (v3d_layer_offset,
 * :776-777). The buffer is the layout's own size, Mesa's rsc->size (:739). */
static void tiling3d_case(uint32_t w, uint32_t h, uint32_t depth, uint32_t levels)
{
	char what[200];
	struct v3d_tex_layout l;
	if (!v3d_tex_layout_3d(w, h, depth, levels, 4, &l)) {
		snprintf(what, sizeof(what), "3D tiling %ux%ux%u x%u: laid out", w, h, depth, levels);
		check(false, what);
		return;
	}
	uint8_t *ours = calloc(1, l.size), *mesa = calloc(1, l.size);
	bool offsets_agree = true;
	for (uint32_t lv = 0; lv < levels; lv++) {
		const struct v3d_tex_slice *s = &l.slices[lv];
		uint32_t lw = w >> lv ? w >> lv : 1, lh = h >> lv ? h >> lv : 1;
		for (uint32_t k = 0; k < v3d_tex_level_depth(depth, lv); k++) {
			uint32_t off = 0;
			offsets_agree &= v3d_tex_slice_offset(&l, lv, k, depth, &off) &&
					 off == s->offset + k * s->size;
			uint8_t *src = malloc((size_t)lw * lh * 4);
			for (uint32_t i = 0; i < lw * lh * 4; i++)
				src[i] = (uint8_t)rng();
			v3d_tex_store_level(ours + (size_t)k * s->size, &l, lv, src, lw * 4);
			struct pipe_box box = { .x = 0, .y = 0, .z = 0, .width = (int32_t)lw,
						.height = (int32_t)lh, .depth = 1 };
			v3d_store_tiled_image(mesa + s->offset + (size_t)k * s->size, s->stride, src, lw * 4,
					      (enum v3d_tiling_mode)s->tiling, 4, s->padded_height, &box);
			free(src);
		}
	}
	snprintf(what, sizeof(what),
		 "3D tiling %ux%ux%u x%u: every slice x level at v3d_layer_offset's place, our bytes are Mesa's (%u bytes)",
		 w, h, depth, levels, l.size);
	check(offsets_agree && memcmp(ours, mesa, l.size) == 0, what);
	free(ours);
	free(mesa);
}

/* Mesa's packer for an N-deep record: `stride64` is the rule under test's
 * input restated (the tree stride / 64 for an array, level 0's slice size /
 * 64 for a 3D texture), `depth` the layer count or the level 0 depth. */
static void mesa_layered_record(uint8_t out[V3D_TEX_STATE_BYTES], const struct v3d_tex_layout *l,
				uint32_t base, uint32_t depth, uint32_t stride64)
{
	struct V3D42_TEXTURE_SHADER_STATE t = { 0 };
	t.image_width = l->width;
	t.image_height = l->height;
	t.image_depth = depth;
	t.max_level = l->levels - 1;
	t.texture_base_pointer = base + l->slices[0].offset;
	t.array_stride_64_byte_aligned = stride64;
	t.level_0_is_strictly_uif = l->slices[0].tiling == V3D_TEX_TILING_UIF_XOR ||
				    l->slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR;
	t.level_0_xor_enable = l->slices[0].tiling == V3D_TEX_TILING_UIF_XOR;
	if (t.level_0_is_strictly_uif) {
		t.level_0_ub_pad = l->slices[0].ub_pad;
		t.extended = true;
	}
	t.swizzle_r = 2;
	t.swizzle_g = 3;
	t.swizzle_b = 4;
	t.swizzle_a = 5;
	t.texture_type = TEXTURE_DATA_FORMAT_RGBA8;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, out, &t);
}

static void part_arrays_and_3d(void)
{
	struct v3d_tex_layout l, l2;
	uint32_t size = 0, off = 0;

	/* -- 2D arrays: the cube path with the layer count --
	 * 64x32 x7 RGBA8 (utile 4x4, UIF block 8x8; pot = 2 * next_pot(32) =
	 * 64 wide, 2 * next_pot(16) = 32 high), smallest first:
	 *   6: 1x1 LT 16x4 = 64 @0      5: 2x1 LT 64 @64     4: 4x2 LT 64 @128
	 *   3: 8x4 (4 rows <= utile) LT 32x4 = 128 @192
	 *   2: 16x8 UB2 64x8 = 512 @320
	 *   1: 32x16 UIF (2 UB rows: no pad, no XOR) 128x16 = 2048 @832;
	 *      32 wide is not > 4 blocks (32): no page alignment
	 *   0: 64x32 UIF (4 rows: no pad, no XOR) 256x32 = 8192 @2880
	 * 11072 in all; level 0 to 4 KiB: +1216 on every offset, 12288.
	 * Tree stride align(4096 + 8192, 64) = 12288; 4 layers 49152; level 2
	 * of layer 3 at 1536 + 3 * 12288 = 38400; the state's stride 192. */
	check(v3d_tex_layout(64, 32, 7, 4, &l) && l.size == 12288 && l.slices[0].offset == 4096 &&
		      l.slices[1].offset == 2048 && l.slices[2].offset == 1536 && l.slices[6].offset == 1216 &&
		      v3d_tex_layer_stride(&l) == 12288 && v3d_tex_layers_size(&l, 4, &size) &&
		      size == 49152 && v3d_tex_layer_offset(&l, 2, 3, 4, &off) && off == 38400,
	      "array 64x32 x7, 4 layers: tree stride 12288, 49152 bytes, level 2 of layer 3 at 38400");
	/* 13x7 x3 (part 1: level 0 UB2 512 @4096, 1 LT 128 @3968, 2 LT 64
	 * @3904, 4608 in all), 3 layers: stride align(4608, 64) = 4608, 13824
	 * bytes, level 1 of layer 2 at 3968 + 2 * 4608 = 13184 */
	check(v3d_tex_layout(13, 7, 3, 4, &l) && v3d_tex_layer_stride(&l) == 4608 &&
		      v3d_tex_layers_size(&l, 3, &size) && size == 13824 &&
		      v3d_tex_layer_offset(&l, 1, 2, 3, &off) && off == 13184,
	      "array 13x7 x3 (non-pot), 3 layers: stride 4608, 13824 bytes, level 1 of layer 2 at 13184");
	array_tiling_case(64, 32, 4, 7);
	array_tiling_case(13, 7, 3, 3);

	/* the array's record: depth 4 (the layer count), array stride 192 */
	v3d_tex_layout(64, 32, 7, 4, &l);
	struct v3d_tex_state st = { .base = 0x00400000u + l.slices[0].offset, .width = 64,
				    .height = 32, .depth = 4, .max_level = 6,
				    .type = V3D_TEXTURE_DATA_FORMAT_RGBA8, .swizzle = { 2, 3, 4, 5 } };
	v3d_tex_state_from_layout(&st, &l);
	uint8_t o[V3D_TEX_STATE_BYTES], m[V3D_TEX_STATE_BYTES];
	bool packed = v3d_tex_pack_state(o, &st);
	mesa_layered_record(m, &l, 0x00400000u, 4, 12288 / 64);
	check(packed && st.array_stride_64 == 192 && memcmp(o, m, sizeof(o)) == 0,
	      "array record 64x32 x7, 4 layers: depth 4, array stride 192, our bytes are Mesa's");

	/* -- 3D: a per-level depth, each level its slices one size apart --
	 * 32x32x8 x6 RGBA8: pot 32 wide, 32 high; pot_depth = 2 * next_pot(4)
	 * = 8, level i >= 1 has 8 >> i slices (at least 1):
	 *   5: 1x1 LT 64, 1 slice: 64 @0        4: 2x2 LT 64 x1 @64
	 *   3: 4x4 LT 64 x1 @128                 2: 8x8 UB1 32x8 = 256 x2 = 512 @192
	 *   1: 16x16 UB2 64x16 = 1024 x4 = 4096 @704 (16 wide: no page alignment)
	 *   0: 32x32 UIF (4 UB rows: no pad, no XOR) 128x32 = 4096 x8 = 32768 @4800
	 * 37568; level 0 to 4 KiB: +3392, 40960. Slice stride 4096 / 64 = 64. */
	layout3d_case("32x32x8 x6", 32, 32, 8, 6, 40960,
		      (struct want_slice_3d[]){ { 8192, 128, 32, 0, UIF, 8 }, { 4096, 64, 16, 0, UB2, 4 },
						{ 3584, 32, 8, 0, UB1, 2 },   { 3520, 16, 4, 0, LT, 1 },
						{ 3456, 16, 4, 0, LT, 1 },    { 3392, 16, 4, 0, LT, 1 } });
	/* 13x7x6 x4 (non-pot): pot 16 wide, 8 high (part 1's 13x7), pot_depth
	 * = 2 * next_pot(3) = 8: levels 1.. have 4, 2, 1 slices where GL has 3,
	 * 1, 1 (the rest is padding the backend owns).
	 *   3: 2x1 LT 64 x1 @0      2: 4x2 LT 64 x2 = 128 @64
	 *   1: 6x3 LT 32x4 = 128 x4 = 512 @192
	 *   0: 13x7 UB2 64x8 = 512 x6 = 3072 @704
	 * 3776; level 0 to 4 KiB: +3392, 7168. Slice stride 512 / 64 = 8. */
	layout3d_case("13x7x6 x4 (non-pot)", 13, 7, 6, 4, 7168,
		      (struct want_slice_3d[]){ { 4096, 64, 8, 0, UB2, 6 }, { 3584, 32, 4, 0, LT, 4 },
						{ 3456, 16, 4, 0, LT, 2 }, { 3392, 16, 4, 0, LT, 1 } });
	/* 4x4x16 x5: the depth outlasts the sides (the GL core's
	 * chain_levels_3d: 1x1 levels while the depth halves). pot 4 wide and
	 * high, pot_depth = 2 * next_pot(8) = 16: 16, 8, 4, 2, 1 slices.
	 *   4: 1x1 LT 64 x1 @0     3: 1x1 LT 64 x2 = 128 @64
	 *   2: 1x1 (4 >> 2) LT 64 x4 = 256 @192
	 *   1: 2x2 LT 64 x8 = 512 @448
	 *   0: 4x4 LT 64 x16 = 1024 @960
	 * 1984; level 0 to 4 KiB: +3136, 5120. Slice stride 64 / 64 = 1. */
	layout3d_case("4x4x16 x5 (depth outlasts the sides)", 4, 4, 16, 5, 5120,
		      (struct want_slice_3d[]){ { 4096, 16, 4, 0, LT, 16 }, { 3584, 16, 4, 0, LT, 8 },
						{ 3328, 16, 4, 0, LT, 4 },  { 3200, 16, 4, 0, LT, 2 },
						{ 3136, 16, 4, 0, LT, 1 } });
	/* 80x656x3 x2: part 1's level 1 (40x328 UIF 256x328 = 83968) is
	 * page-aligned; with pot_depth = 2 * next_pot(1) = 2 it has 1 slice,
	 * and the TOTAL is aligned, 86016 (:729-734). Level 0 (96 wide, 82
	 * rows, UIF 384x656 = 251904) has 3 slices: 755712 @86016; 841728 in
	 * all (86016 is 21 pages: no final shift). Slice stride 251904 / 64 =
	 * 3936. */
	layout3d_case("80x656x3 x2 (level 1 page-aligned)", 80, 656, 3, 2, 841728,
		      (struct want_slice_3d[]){ { 86016, 384, 656, 0, UIF, 3 }, { 0, 256, 328, 0, UIF, 1 } });

	/* level_depth by hand (:609-611, :636-639): depth 6 -> 6, 4, 2, 1;
	 * depth 5 -> 5, 2, 1 (next_pot(2) = 2, x2 = 4); depth 9 -> 9, 4, 2, 1
	 * (next_pot(4) = 4, x2 = 8); depth 1 -> 1 at every level */
	check(v3d_tex_level_depth(6, 0) == 6 && v3d_tex_level_depth(6, 1) == 4 &&
		      v3d_tex_level_depth(6, 2) == 2 && v3d_tex_level_depth(6, 3) == 1 &&
		      v3d_tex_level_depth(5, 1) == 2 && v3d_tex_level_depth(5, 2) == 1 &&
		      v3d_tex_level_depth(9, 1) == 4 && v3d_tex_level_depth(9, 2) == 2 &&
		      v3d_tex_level_depth(9, 3) == 1 && v3d_tex_level_depth(1, 0) == 1 &&
		      v3d_tex_level_depth(1, 1) == 1 && v3d_tex_level_depth(1, 9) == 1,
	      "level_depth: 6 -> 6,4,2,1; 5 -> 5,2,1; 9 -> 9,4,2,1; 1 -> 1");

	/* a depth of 1 is the 2D layout, byte for byte */
	bool same = true;
	static const uint32_t flat[][3] = { { 1, 1, 1 }, { 13, 7, 3 }, { 256, 256, 9 }, { 80, 656, 2 } };
	for (unsigned i = 0; i < sizeof(flat) / sizeof(flat[0]); i++)
		same &= v3d_tex_layout(flat[i][0], flat[i][1], flat[i][2], 4, &l) &&
			v3d_tex_layout_3d(flat[i][0], flat[i][1], 1, flat[i][2], 4, &l2) &&
			memcmp(&l, &l2, sizeof(l)) == 0;
	check(same, "3D with a depth of 1 is v3d_tex_layout's layout (4 sizes)");

	/* slice offsets of 32x32x8 x6 (:776-777): level 0 slice 7 at 8192 +
	 * 7 * 4096 = 36864, level 1 slice 3 at 4096 + 3 * 1024 = 7168, level 2
	 * slice 1 at 3584 + 256 = 3840 */
	v3d_tex_layout_3d(32, 32, 8, 6, 4, &l);
	uint32_t o0 = 0, o1 = 0, o2 = 0;
	check(v3d_tex_slice_offset(&l, 0, 7, 8, &o0) && o0 == 36864 && v3d_tex_slice_offset(&l, 1, 3, 8, &o1) &&
		      o1 == 7168 && v3d_tex_slice_offset(&l, 2, 1, 8, &o2) && o2 == 3840,
	      "3D 32x32x8 slice offsets: (0, 7) 36864, (1, 3) 7168, (2, 1) 3840");

	tiling3d_case(32, 32, 8, 6);
	tiling3d_case(13, 7, 6, 4);
	tiling3d_case(4, 4, 16, 5);
	tiling3d_case(80, 656, 3, 2);

	/* the 3D record: depth 8 (level 0's), array stride 4096 / 64 = 64 */
	v3d_tex_layout_3d(32, 32, 8, 6, 4, &l);
	struct v3d_tex_state s3 = { .base = 0x00400000u + l.slices[0].offset, .width = 32,
				    .height = 32, .depth = 8, .max_level = 5,
				    .type = V3D_TEXTURE_DATA_FORMAT_RGBA8, .swizzle = { 2, 3, 4, 5 } };
	v3d_tex_state_from_layout_3d(&s3, &l);
	packed = v3d_tex_pack_state(o, &s3);
	mesa_layered_record(m, &l, 0x00400000u, 8, l.slices[0].size / 64);
	check(packed && s3.array_stride_64 == 64 && memcmp(o, m, sizeof(o)) == 0,
	      "3D record 32x32x8 x6: depth 8, array stride 64, our bytes are Mesa's");

	/* control: the same record with the array's tree stride (what
	 * v3d_tex_state_from_layout gives, (8192 + 4096) / 64 = 192) is not
	 * Mesa's 3D record */
	struct v3d_tex_state wrong = s3;
	v3d_tex_state_from_layout(&wrong, &l);
	uint8_t w[V3D_TEX_STATE_BYTES];
	check(v3d_tex_pack_state(w, &wrong) && wrong.array_stride_64 == 192 && memcmp(w, m, sizeof(w)) != 0,
	      "control: a 3D record carrying the array's tree stride is detected");

	/* the refusals: nothing written */
	uint32_t sentinel = 0x12345678u, v = sentinel;
	memset(&l2, 0xcc, sizeof(l2));
	struct v3d_tex_layout untouched = l2;
	check(!v3d_tex_layout_3d(32, 32, 0, 1, 4, &l2) && memcmp(&l2, &untouched, sizeof(l2)) == 0,
	      "3D layout: a depth of 0 is refused, nothing written");
	check(!v3d_tex_layout_3d(32, 32, 16384, 1, 4, &l2) && memcmp(&l2, &untouched, sizeof(l2)) == 0,
	      "3D layout: a depth of 16384 is refused, nothing written");
	/* 8192x8192 at 16 bytes a texel is 2^30 a slice; 8 slices pass 32 bits */
	check(!v3d_tex_layout_3d(8192, 8192, 8, 1, 16, &l2) && memcmp(&l2, &untouched, sizeof(l2)) == 0,
	      "3D layout: a stack past 32 bits is refused, nothing written");
	check(!v3d_tex_slice_offset(&l, 2, 2, 8, &v) && v == sentinel,
	      "slice offset: slice 2 of level 2 (2 slices) is refused");
	check(!v3d_tex_slice_offset(&l, 6, 0, 8, &v) && v == sentinel, "slice offset: level 6 of 6 is refused");
}

/* ---- part 12: ETC2 and EAC (docs/notes/v3d-tex-layout-es3.md section 4) -- */

/* A compressed level is laid out and tiled as a level of blocks, a block one
 * texel of the block's bytes (v3d_resource.c:646-647, :797), with the
 * power-of-two padding of the levels from 2 taken in whole blocks
 * (v3d_get_dimension_mpad with the block, :573-581, :596-608). The layouts
 * are derived by hand from v3d_setup_slices, as part 1's are; the formats'
 * rows, blocks and sRGB-ness are Mesa's built format table and util_format;
 * the tiling is Mesa's built v3d_store_tiled_image / v3d_load_tiled_image
 * with Mesa's block counts (util_format_get_nblocksx/y) and cpp = the
 * block's bytes. */

static void clayout_case(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp,
			 uint32_t size, const struct want_slice *want)
{
	struct v3d_tex_layout l;
	char what[200];
	snprintf(what, sizeof(what), "ETC2 %s: laid out", name);
	check(v3d_tex_layout_compressed(w, h, levels, 4, 4, cpp, &l) && l.width == w && l.height == h &&
		      l.cpp == cpp && l.levels == levels,
	      what);
	snprintf(what, sizeof(what), "ETC2 %s: size %u, want %u", name, l.size, size);
	check(l.size == size, what);
	for (uint32_t i = 0; i < levels; i++) {
		const struct v3d_tex_slice *s = &l.slices[i];
		const struct want_slice *x = &want[i];
		snprintf(what, sizeof(what),
			 "ETC2 %s level %u: offset %u stride %u h %u pad %u tiling %d, want %u %u %u %u %d",
			 name, i, s->offset, s->stride, s->padded_height, s->ub_pad, s->tiling, x->offset,
			 x->stride, x->padded_height, x->ub_pad, x->tiling);
		check(s->offset == x->offset && s->stride == x->stride &&
			      s->padded_height == x->padded_height && s->ub_pad == x->ub_pad &&
			      s->tiling == x->tiling && s->size == x->stride * x->padded_height,
		      what);
	}
}

static const enum v3d_tex_format_id etc_ids[10] = {
	V3D_TEX_FORMAT_ETC2_RGB8,      V3D_TEX_FORMAT_ETC2_SRGB8,     V3D_TEX_FORMAT_ETC2_RGB8A1,
	V3D_TEX_FORMAT_ETC2_SRGB8A1,   V3D_TEX_FORMAT_ETC2_RGBA8,     V3D_TEX_FORMAT_ETC2_SRGBA8,
	V3D_TEX_FORMAT_ETC2_R11_UNORM, V3D_TEX_FORMAT_ETC2_R11_SNORM, V3D_TEX_FORMAT_ETC2_RG11_UNORM,
	V3D_TEX_FORMAT_ETC2_RG11_SNORM,
};

/* Every level of a w x h x levels image of format `id`: ours laid out by our
 * row's block and tiled by v3d_tex_store_level_blocks, against Mesa's
 * v3d_store_tiled_image of Mesa's block count at the layout's slices. Mesa's
 * cpp and block are the ones the layout must have been given: a row that
 * disagrees fails here as well as in part 5. */
static void ctiling_case(enum v3d_tex_format_id id, uint32_t w, uint32_t h, uint32_t levels)
{
	const char *name = v3d_tex_format_name(id);
	char what[200];
	struct v3d_tex_format f;
	struct mesa_fmt m;
	uint32_t bw = 0, bh = 0;
	struct v3d_tex_layout l;
	if (!(v3d_tex_format_get(id, &f) && v3d_tex_format_block(id, &bw, &bh) && mesa_v3d_format(name, &m) &&
	      f.cpp == m.cpp && bw == m.block_w && bh == m.block_h &&
	      v3d_tex_layout_compressed(w, h, levels, bw, bh, f.cpp, &l))) {
		snprintf(what, sizeof(what), "ETC2 tiling %s %ux%u x%u: our row, Mesa's row and a layout", name, w,
			 h, levels);
		check(false, what);
		return;
	}
	uint8_t *ours = calloc(1, l.size), *mesa = calloc(1, l.size);
	bool ok = true;
	for (uint32_t lv = 0; lv < levels; lv++) {
		uint32_t mx = 0, my = 0, gx = 0, gy = 0;
		ok &= mesa_v3d_level_blocks(name, w, h, lv, &mx, &my) &&
		      v3d_tex_level_blocks(&l, lv, bw, bh, &gx, &gy) && gx == mx && gy == my;
		uint32_t stride = mx * m.cpp + 8; /* a source pitch wider than the row */
		uint8_t *src = malloc((size_t)stride * my);
		for (uint32_t i = 0; i < stride * my; i++)
			src[i] = (uint8_t)rng();
		ok &= v3d_tex_store_level_blocks(ours, &l, lv, bw, bh, src, stride);
		const struct v3d_tex_slice *s = &l.slices[lv];
		struct pipe_box box = { .x = 0, .y = 0, .z = 0, .width = (int32_t)mx, .height = (int32_t)my,
					.depth = 1 };
		v3d_store_tiled_image(mesa + s->offset, s->stride, src, stride, (enum v3d_tiling_mode)s->tiling,
				      (int)m.cpp, s->padded_height, &box);
		free(src);
	}
	snprintf(what, sizeof(what),
		 "ETC2 tiling %s %ux%u x%u: Mesa's block counts, every level, our bytes are Mesa's (%u bytes)", name,
		 w, h, levels, l.size);
	check(ok && memcmp(ours, mesa, l.size) == 0, what);
	free(ours);
	free(mesa);
}

static unsigned cboxes_stored;

/* Sub-boxes of blocks (glCompressedTexSubImage2D: whole blocks, Mesa's
 * u_box_pixels_to_blocks): fixed and seeded random boxes on every level,
 * the whole image compared with Mesa's after every box, and each box loaded
 * back with v3d_tex_load_blocks against v3d_load_tiled_image. */
static void cbox_case(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp)
{
	char what[240];
	struct v3d_tex_layout l;
	if (!v3d_tex_layout_compressed(w, h, levels, 4, 4, cpp, &l)) {
		snprintf(what, sizeof(what), "ETC2 boxes %ux%u x%u cpp %u: laid out", w, h, levels, cpp);
		check(false, what);
		check(false, what);
		return;
	}
	uint8_t *ours = malloc(l.size), *mesa = malloc(l.size);
	for (uint32_t i = 0; i < l.size; i++)
		ours[i] = mesa[i] = (uint8_t)(i * 31 + 3);
	char bad[160] = "", load_bad[160] = "";
	for (uint32_t lv = 0; lv < levels; lv++) {
		const struct v3d_tex_slice *s = &l.slices[lv];
		uint32_t gw = 1, gh = 1;
		v3d_tex_level_blocks(&l, lv, 4, 4, &gw, &gh);
		uint32_t box[10][4] = {
			{ 0, 0, 1, 1 }, { gw - 1, gh - 1, 1, 1 }, { 1, 1, gw, gh }, { 0, 0, gw, gh },
		};
		for (int r = 4; r < 10; r++) {
			uint32_t bx = rng() % gw, by = rng() % gh;
			box[r][0] = bx;
			box[r][1] = by;
			box[r][2] = 1 + rng() % (gw - bx);
			box[r][3] = 1 + rng() % (gh - by);
		}
		for (int b = 0; b < 10; b++) {
			uint32_t bx = min_u32(box[b][0], gw - 1), by = min_u32(box[b][1], gh - 1);
			uint32_t bwid = min_u32(box[b][2], gw - bx), bhei = min_u32(box[b][3], gh - by);
			uint32_t stride = bwid * cpp + 4;
			size_t n = (size_t)stride * bhei;
			uint8_t *src = malloc(n);
			for (size_t i = 0; i < n; i++)
				src[i] = (uint8_t)rng();
			bool ok = v3d_tex_store_blocks(ours, &l, lv, 4, 4, bx, by, bwid, bhei, src, stride);
			struct pipe_box pb = { .x = (int32_t)bx, .width = (int32_t)bwid, .y = (int32_t)by,
					       .height = (int32_t)bhei, .z = 0, .depth = 1 };
			v3d_store_tiled_image(mesa + s->offset, s->stride, src, stride,
					      (enum v3d_tiling_mode)s->tiling, (int)cpp, s->padded_height, &pb);
			cboxes_stored++;
			if (!bad[0] && (!ok || memcmp(ours, mesa, l.size) != 0))
				snprintf(bad, sizeof(bad), " (first difference: level %u box %u,%u %ux%u%s)", lv, bx,
					 by, bwid, bhei, ok ? "" : ", refused");
			/* back out of both images, into rows whose padding must
			 * stay as it was */
			uint8_t *a = malloc(n), *mm = malloc(n);
			memset(a, 0x3c, n);
			memset(mm, 0x3c, n);
			bool lok = v3d_tex_load_blocks(ours, &l, lv, 4, 4, bx, by, bwid, bhei, a, stride);
			v3d_load_tiled_image(mm, stride, mesa + s->offset, s->stride, (enum v3d_tiling_mode)s->tiling,
					     (int)cpp, s->padded_height, &pb);
			if (!load_bad[0] && (!lok || memcmp(a, mm, n) != 0))
				snprintf(load_bad, sizeof(load_bad), " (first difference: level %u box %u,%u %ux%u%s)",
					 lv, bx, by, bwid, bhei, lok ? "" : ", refused");
			free(a);
			free(mm);
			free(src);
		}
	}
	snprintf(what, sizeof(what), "ETC2 boxes %ux%u x%u cpp %u: store_blocks, our bytes are Mesa's after every box%s",
		 w, h, levels, cpp, bad);
	check(!bad[0], what);
	snprintf(what, sizeof(what), "ETC2 boxes %ux%u x%u cpp %u: load_blocks reads Mesa's v3d_load_tiled_image%s", w,
		 h, levels, cpp, load_bad);
	check(!load_bad[0], what);
	free(ours);
	free(mesa);
}

/* The record of a 36x36 x6 image of format `id` against Mesa's packer of
 * Mesa's row: its type, its swizzle, and the sRGB bit util_format_is_srgb
 * gives (v3dx_state.c:1023-1025); width and height in texels (prsc->width0,
 * v3dx_state.c:932-933). */
static void cstate_case(enum v3d_tex_format_id id, uint32_t base)
{
	const char *name = v3d_tex_format_name(id);
	char what[200];
	struct v3d_tex_format f;
	struct mesa_fmt m;
	uint32_t bw = 0, bh = 0;
	struct v3d_tex_layout l;
	if (!(v3d_tex_format_get(id, &f) && v3d_tex_format_block(id, &bw, &bh) && mesa_v3d_format(name, &m) &&
	      m.in_table && v3d_tex_layout_compressed(36, 36, 6, bw, bh, f.cpp, &l))) {
		snprintf(what, sizeof(what), "ETC2 texture state %s: our row, Mesa's row and a layout", name);
		check(false, what);
		return;
	}
	struct v3d_tex_state s = { .base = base + l.slices[0].offset, .width = 36, .height = 36, .depth = 1,
				   .max_level = 5, .type = f.type, .srgb = v3d_tex_format_is_srgb(id) };
	memcpy(s.swizzle, f.swizzle, 4);
	v3d_tex_state_from_layout(&s, &l);
	uint8_t ours[V3D_TEX_STATE_BYTES], mesa[V3D_TEX_STATE_BYTES];
	bool packed = v3d_tex_pack_state(ours, &s);
	struct V3D42_TEXTURE_SHADER_STATE t = { 0 };
	t.image_width = 36;
	t.image_height = 36;
	t.image_depth = 1;
	t.max_level = 5;
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
	t.srgb = m.srgb;
	V3D42_TEXTURE_SHADER_STATE_pack(NULL, mesa, &t);
	snprintf(what, sizeof(what), "ETC2 texture state %s 36x36 x6 (type %u, sRGB %d): our bytes are Mesa's", name,
		 m.type, m.srgb);
	check(packed && memcmp(ours, mesa, sizeof(ours)) == 0, what);
	if (memcmp(ours, mesa, sizeof(ours)) != 0)
		for (int i = 0; i < V3D_TEX_STATE_BYTES; i++)
			printf("  %2d: ours %02x mesa %02x\n", i, ours[i], mesa[i]);
}

static void part_compressed(void)
{
	/* By hand from v3d_setup_slices with 4x4 blocks. cpp 8: utile 4x2
	 * blocks, UIF block 8x4; cpp 16: utile 2x2, UIF block 4x4.
	 *
	 * 17x9 x5, cpp 8: pot_width = 2 * 4 * next_pot(ceil(8 / 4)) = 16,
	 * pot_height = 2 * 4 * next_pot(ceil(4 / 4)) = 8. Level 0: 5x3 blocks,
	 * 5 > 4 and 3 > 2, 5 <= 8: UB1, 8x4, stride 64, 256 bytes. Level 1:
	 * 8x4 texels, 2x1 blocks: LT 4x2, stride 32, 64. Levels 2..4: 16 >> i
	 * x 8 >> i texels (4x2, 2x1, 1x1): one block each, LT 64. Smallest
	 * first: 4 @0, 3 @64, 2 @128, 1 @192, 0 @256; 512 in all, level 0 to
	 * 4 KiB: +3840. */
	clayout_case("17x9 x5 (cpp 8)", 17, 9, 5, 8, 4352,
		     (struct want_slice[]){ { 4096, 64, 4, 0, UB1 }, { 4032, 32, 2, 0, LT },
					    { 3968, 32, 2, 0, LT }, { 3904, 32, 2, 0, LT },
					    { 3840, 32, 2, 0, LT } });
	/* ... cpp 16: level 0's 5x3 blocks: 5 > 4, <= 8: UB2, 8x4, stride 128,
	 * 512 bytes; the rest LT 2x2 utiles of 64 bytes: +3840 as above */
	clayout_case("17x9 x5 (cpp 16)", 17, 9, 5, 16, 4608,
		     (struct want_slice[]){ { 4096, 128, 4, 0, UB2 }, { 4032, 32, 2, 0, LT },
					    { 3968, 32, 2, 0, LT }, { 3904, 32, 2, 0, LT },
					    { 3840, 32, 2, 0, LT } });
	/* 36x36 x6, cpp 8 (the note's example): pot = 2 * 4 * next_pot(ceil(18
	 * / 4) = 5) = 64. Level 0: 9x9 blocks, 9 > 8, <= 16: UB2, 16x12, stride
	 * 128, 1536. Level 1: 18 texels, 5x5 blocks: UB1, 8x8, stride 64, 512.
	 * Level 2: 64 >> 2 = 16 texels, 4x4 blocks (36 >> 2 = 9 texels would be
	 * 3): LT, 4x4, stride 32, 128. Levels 3..5: 8, 4, 2 texels, 2, 1, 1
	 * blocks: LT 4x2, 64 each. 5 @0, 4 @64, 3 @128, 2 @192, 1 @320, 0 @832;
	 * 2368, level 0 to 4 KiB: +3264. */
	clayout_case("36x36 x6 (cpp 8)", 36, 36, 6, 8, 5632,
		     (struct want_slice[]){ { 4096, 128, 12, 0, UB2 }, { 3584, 64, 8, 0, UB1 },
					    { 3456, 32, 4, 0, LT },   { 3392, 32, 2, 0, LT },
					    { 3328, 32, 2, 0, LT },   { 3264, 32, 2, 0, LT } });
	/* ... cpp 16: level 0 9x9 blocks > 8: UIF, 16 wide (4 UIF blocks), 12
	 * high (3 UB rows: no pad, no XOR), stride 256, 3072. Level 1 5x5: UB2
	 * 8x8, stride 128, 1024. Level 2 4x4 blocks: UB1 4x4, stride 64, 256.
	 * Levels 3..5 LT 2x2, 64 each. 5 @0, 4 @64, 3 @128, 2 @192, 1 @448, 0
	 * @1472; 4544, level 0 to 4 KiB: +2624. */
	clayout_case("36x36 x6 (cpp 16)", 36, 36, 6, 16, 7168,
		     (struct want_slice[]){ { 4096, 256, 12, 0, UIF }, { 3072, 128, 8, 0, UB2 },
					    { 2816, 64, 4, 0, UB1 },  { 2752, 32, 2, 0, LT },
					    { 2688, 32, 2, 0, LT },   { 2624, 32, 2, 0, LT } });
	/* 1x1: one block in one utile, 64 bytes at 0 */
	clayout_case("1x1 (cpp 8)", 1, 1, 1, 8, 64, (struct want_slice[]){ { 0, 32, 2, 0, LT } });
	clayout_case("1x1 (cpp 16)", 1, 1, 1, 16, 64, (struct want_slice[]){ { 0, 32, 2, 0, LT } });
	/* 256x256 x9, cpp 16: 64, 32, 16 blocks wide at levels 0..2: UIF (16
	 * rows, 8, 4: no pad, no XOR; level 1 is 32 blocks wide, > 16, but its
	 * 32 rows are not > 26 UB rows (104): no page alignment); level 3 8
	 * blocks UB2, 4 UB1, then LT. 8..5 @0, 64, 128, 192; 4 @256 (256), 3
	 * @512 (1024), 2 @1536 (4096), 1 @5632 (16384), 0 @22016 (65536);
	 * level 0 to 4 KiB: +2560. */
	clayout_case("256x256 x9 (cpp 16)", 256, 256, 9, 16, 90112,
		     (struct want_slice[]){ { 24576, 1024, 64, 0, UIF }, { 8192, 512, 32, 0, UIF },
					    { 4096, 256, 16, 0, UIF },   { 3072, 128, 8, 0, UB2 },
					    { 2816, 64, 4, 0, UB1 },     { 2752, 32, 2, 0, LT },
					    { 2688, 32, 2, 0, LT },      { 2624, 32, 2, 0, LT },
					    { 2560, 32, 2, 0, LT } });
	/* 1024x1056, cpp 16: 256x264 blocks, UIF; 66 UB rows, 66 % 32 = 2 < 6
	 * and 66 >= 32: ub_pad 4, 70 rows (280), no XOR; stride 4096 */
	clayout_case("1024x1056 (cpp 16, ub_pad 4)", 1024, 1056, 1, 16, 4096 * 280,
		     (struct want_slice[]){ { 0, 4096, 280, 4, UIF } });
	/* 64x960, cpp 16: 16x240 blocks, UIF; 60 UB rows, 60 % 32 = 28 > 26:
	 * pad 4, 64 rows (256), 64 % 32 == 0: XOR; stride 256 */
	clayout_case("64x960 (cpp 16, UIF XOR)", 64, 960, 1, 16, 65536,
		     (struct want_slice[]){ { 0, 256, 256, 4, UIFX } });

	/* control: level 0's block count minified (36x36 laid out as an
	 * uncompressed 9x9 of cpp 8) misses the block-aware padding: its level
	 * 1 is 4 blocks wide where Mesa's is 5, and its level 2 2 rows high
	 * where Mesa allocates 4 */
	struct v3d_tex_layout l, naive;
	uint32_t gw = 0, gh = 0;
	check(v3d_tex_layout_compressed(36, 36, 6, 4, 4, 8, &l) && v3d_tex_layout(9, 9, 6, 8, &naive) &&
		      l.slices[2].padded_height == 4 && naive.slices[2].padded_height == 2 &&
		      memcmp(&l.slices[1], &naive.slices[1], sizeof(l.slices[1])) != 0,
	      "control: 36x36 laid out as level 0's 9x9 blocks is detected (level 2: 2 rows, Mesa 4)");
	/* GL's blocks of 36x36 level 2 (9 texels: 3x3) sit in Mesa's 4x4 */
	check(v3d_tex_level_blocks(&l, 2, 4, 4, &gw, &gh) && gw == 3 && gh == 3 &&
		      l.slices[2].stride / 8 == 4 && l.slices[2].padded_height == 4,
	      "36x36 level 2: GL's 3x3 blocks in Mesa's padded 4x4");
	/* a 1x1 block is the uncompressed layout */
	bool same = true;
	static const uint32_t flat[][3] = { { 1, 1, 1 }, { 13, 7, 3 }, { 256, 256, 9 }, { 80, 656, 2 } };
	for (unsigned i = 0; i < sizeof(flat) / sizeof(flat[0]); i++)
		for (uint32_t cpp = 8; cpp <= 16; cpp += 8)
			same &= v3d_tex_layout(flat[i][0], flat[i][1], flat[i][2], cpp, &l) &&
				v3d_tex_layout_compressed(flat[i][0], flat[i][1], flat[i][2], 1, 1, cpp, &naive) &&
				memcmp(&l, &naive, sizeof(l)) == 0;
	check(same, "a 1x1 block is v3d_tex_layout's layout (4 sizes x cpp 8, 16)");

	/* every format, odd, non-power-of-two and 1x1 sizes */
	for (unsigned i = 0; i < 10; i++) {
		ctiling_case(etc_ids[i], 17, 9, 5);
		ctiling_case(etc_ids[i], 36, 36, 6);
		ctiling_case(etc_ids[i], 1, 1, 1);
	}
	cbox_case(36, 36, 6, 8);
	cbox_case(36, 36, 6, 16);
	cbox_case(256, 256, 9, 8);
	cbox_case(256, 256, 9, 16);
	for (unsigned i = 0; i < 10; i++)
		cstate_case(etc_ids[i], 0x00a00000u + i * 0x10000u);

	/* the refusals: nothing written */
	struct v3d_tex_layout untouched;
	memset(&naive, 0xcc, sizeof(naive));
	untouched = naive;
	check(!v3d_tex_layout_compressed(36, 36, 1, 0, 4, 8, &naive) && memcmp(&naive, &untouched, sizeof(naive)) == 0,
	      "ETC2 layout: a block width of 0 is refused, nothing written");
	check(!v3d_tex_layout_compressed(36, 36, 1, 4, V3D_TEX_MAX_BLOCK_DIM + 1, 8, &naive) &&
		      memcmp(&naive, &untouched, sizeof(naive)) == 0,
	      "ETC2 layout: a block height past V3D_TEX_MAX_BLOCK_DIM is refused, nothing written");
	check(!v3d_tex_layout_compressed(36, 36, 1, 4, 4, 4, &naive) && memcmp(&naive, &untouched, sizeof(naive)) == 0,
	      "ETC2 layout: a block of 4 bytes is refused, nothing written");
	check(!v3d_tex_layout_compressed(16384, 4, 1, 4, 4, 8, &naive) && memcmp(&naive, &untouched, sizeof(naive)) == 0,
	      "ETC2 layout: a width past 14 bits is refused, nothing written");

	/* The texel entry points on a compressed layout: level 0 of 36x36 cpp 8
	 * is 9x9 blocks in a slice 16 blocks (128 bytes) wide and 12 high; a
	 * texel box of the level is 36 x 8 = 288 bytes a row. Each is refused
	 * and writes nothing, in the image or past it: the image sits in a
	 * buffer with a guard after it. */
	check(v3d_tex_layout_compressed(36, 36, 6, 4, 4, 8, &l), "ETC2 36x36 x6 cpp 8: laid out for the refusals");
	const size_t guard = 64 * 1024, rows = 36 * 8 * 36 + 64;
	uint8_t *img = malloc(l.size + guard), *copy = malloc(l.size + guard);
	for (size_t i = 0; i < l.size + guard; i++)
		img[i] = copy[i] = (uint8_t)(i * 7 + 5);
	uint8_t *src = malloc(rows), *dst = malloc(rows), *dst0 = malloc(rows);
	memset(src, 0xa5, rows);
	memset(dst, 0x5c, rows);
	memcpy(dst0, dst, rows);
	/* each case on its own: the buffers are put back after it */
	enum { STORE_LEVEL, STORE_BOX, LOAD_BOX, STORE_BLOCKS, LOAD_BLOCKS };
	static const struct {
		const char *name;
		int fn;
		uint32_t level, bw, x, y, w, h, stride;
	} r[] = {
		{ "v3d_tex_store_level of level 0 (texels as blocks) stops", STORE_LEVEL, 0, 0, 0, 0, 0, 0,
		  36 * 8 },
		{ "v3d_tex_store_box of level 0's texels is refused", STORE_BOX, 0, 0, 0, 0, 36, 36, 36 * 8 },
		{ "v3d_tex_store_box of level 1's texels (18 x 8 > the 64-byte stride) is refused", STORE_BOX, 1, 0, 0,
		  0, 18, 18, 18 * 8 },
		{ "v3d_tex_store_box at row 12 of a 12-row slice is refused", STORE_BOX, 0, 0, 0, 12, 1, 1, 8 },
		{ "v3d_tex_load_box of level 0's texels is refused", LOAD_BOX, 0, 0, 0, 0, 36, 36, 36 * 8 },
		{ "store_blocks one block wider than the level's 9 is refused", STORE_BLOCKS, 0, 4, 0, 0, 10, 1, 80 },
		{ "store_blocks with a block width of 0 is refused", STORE_BLOCKS, 0, 0, 0, 0, 1, 1, 8 },
		{ "load_blocks past GL's 3 blocks of level 2 is refused", LOAD_BLOCKS, 2, 4, 3, 0, 1, 1, 8 },
	};
	char what[200];
	for (unsigned i = 0; i < sizeof(r) / sizeof(r[0]); i++) {
		bool got = false;
		switch (r[i].fn) {
		case STORE_LEVEL:
			v3d_tex_store_level(img, &l, r[i].level, src, r[i].stride);
			break;
		case STORE_BOX:
			got = v3d_tex_store_box(img, &l, r[i].level, r[i].x, r[i].y, r[i].w, r[i].h, src,
						r[i].stride);
			break;
		case LOAD_BOX:
			got = v3d_tex_load_box(img, &l, r[i].level, r[i].x, r[i].y, r[i].w, r[i].h, dst,
					       r[i].stride);
			break;
		case STORE_BLOCKS:
			got = v3d_tex_store_blocks(img, &l, r[i].level, r[i].bw, 4, r[i].x, r[i].y, r[i].w, r[i].h,
						   src, r[i].stride);
			break;
		default:
			got = v3d_tex_load_blocks(img, &l, r[i].level, r[i].bw, 4, r[i].x, r[i].y, r[i].w, r[i].h,
						  dst, r[i].stride);
			break;
		}
		snprintf(what, sizeof(what), "ETC2: %s, nothing written", r[i].name);
		check(!got && memcmp(img, copy, l.size + guard) == 0 && memcmp(dst, dst0, rows) == 0, what);
		memcpy(img, copy, l.size + guard);
		memcpy(dst, dst0, rows);
	}
	struct v3d_tex_tfu_job j;
	memset(&j, 0xcc, sizeof(j));
	struct v3d_tex_tfu_job j0 = j;
	check(v3d_tex_layout_compressed(36, 36, 1, 4, 4, 8, &l) &&
		      !v3d_tex_tfu_pack(&l, V3D_TFU_TTYPE_RGBA8, 0x01000000u, 36 * 8, 0x02000000u, &j) &&
		      memcmp(&j, &j0, sizeof(j)) == 0,
	      "ETC2: a TFU job of a compressed layout (texels past its slice) is refused");
	free(img);
	free(copy);
	free(src);
	free(dst);
	free(dst0);
}

int main(void)
{
	/* part 1: 11 cases (2 + levels each: 3,3,3,3,3,3,11,4,5,3,3) + 4
	 * refusals + 5 for the 4x layout;
	 * part 2: 1 + 13 sizes x 5 cpp; part 3: 1 + 15 + 3;
	 * part 4: 9 sizes x 5 cpp x 2 (store, load) + 8 refusals x 2; part 5: 25 formats x 2 + 2;
	 * part 6: 2 x 3 x 2 x 3 x 3 + 4 femtovg; part 7: 5 controls;
	 * part 8: 1 + 12 state refusals + 2 bias edges + 9 sampler refusals +
	 * P0 + P1 + 3 P0/P1 refusals; part 9: 3 hand cube layouts + 4 cubes
	 * tiled + 1 cube record + 4 refusals; part 10: 6 mipmap jobs x 4 + 5;
	 * part 11: 2 hand array layouts + 2 arrays tiled + 1 array record + 4
	 * 3D layouts (2 + levels each: 8, 6, 7, 4) + level_depth + depth 1 +
	 * slice offsets + 4 3D tilings + 1 3D record + 1 control + 5 refusals;
	 * part 12: 9 compressed layouts (2 + levels each: 5,5,6,6,1,1,9,1,1) +
	 * 1 control + 1 level-2 grid + 1 block-1 equality + 10 formats x 3
	 * sizes tiled + 4 box cases x 2 (store, load) + 10 records + 4 layout
	 * refusals + 1 layout + 8 texel/block refusals + 1 TFU refusal;
	 * and the 4 population checks below */
	const int part1 = (3 + 3 + 3 + 3 + 3 + 3) + (2 + 9) + (2 + 2) + (2 + 3) + (3 + 3) + 4 + 5;
	const int part11 = (2 + 2 + 1) + (8 + 6 + 7 + 4) + 1 + 1 + 1 + 4 + 1 + 1 + 5;
	const int part12 = (7 + 7 + 8 + 8 + 3 + 3 + 11 + 3 + 3) + 1 + 1 + 1 + 10 * 3 + 4 * 2 + 10 + 4 + 1 +
			   8 + 1;
	const int planned = part1 + (1 + 13 * 5) + (1 + 15 + 3) + (9 * 5 * 2 + 8 * 2) +
			    (V3D_TEX_FORMAT_COUNT * 2 + 2) + (108 + 4) + 5 +
			    (1 + 12 + 2 + 9 + 2 + 3) + (3 + 4 + 1 + 4) + (6 * 4 + 5) + part11 + part12 + 4;
	/* the boxes part 4 must have stored: levels 1+1+3+6+1+1+9+2+2 = 26 of
	 * the 9 sizes, x 5 cpp, x (7 fixed + 16 random) boxes a level */
	const unsigned planned_boxes = 26 * 5 * (FIXED_BOXES + RANDOM_BOXES);
	/* part 12's: levels 6 + 6 + 9 + 9 = 30, 10 boxes a level */
	const unsigned planned_cboxes = 30 * 10;
	part_layout();
	part_tiling();
	part_records();
	part_boxes();
	part_formats();
	part_gl_samplers();
	part_controls();
	part_refusals_and_p0p1();
	part_layers();
	part_mipmap_job();
	part_arrays_and_3d();
	part_compressed();

	char what[200];
	snprintf(what, sizeof(what), "population: %u boxes stored, planned %u", boxes_stored,
		 planned_boxes);
	check(boxes_stored == planned_boxes, what);
	snprintf(what, sizeof(what),
		 "population: %u formats supported, planned 32 (RGBA8, BGRA8, L8, A8, L8A8, RGB565, "
		 "R32F, D24S8, the 14 ES 3.0 classes and the 10 ETC2/EAC formats)",
		 formats_supported);
	check(formats_supported == 32, what);
	snprintf(what, sizeof(what), "population: %u compressed boxes stored, planned %u", cboxes_stored,
		 planned_cboxes);
	check(cboxes_stored == planned_cboxes, what);
	snprintf(what, sizeof(what), "population: %u GL samplers packed, planned 112",
		 samplers_checked);
	check(samplers_checked == 112, what);

	printf("tex_check: %u sub-image boxes stored, %llu image bytes compared with Mesa's\n",
	       boxes_stored, (unsigned long long)box_bytes_compared);
	printf("tex_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
