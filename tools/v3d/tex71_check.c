/* SPDX-License-Identifier: MIT
 *
 * tex71_check: driver/src/v3d/v3d_tex71.c, the V3D 7.1 texture shader state,
 * sampler state and TMU parameter 2, against Mesa 26.2.1's 7.1 packing of
 * the same inputs. Built and run by tools/v3d/check71.d/tex71.sh, which
 * compiles this file twice:
 *
 *  - with -DTEX71_MESA_SIDE: Mesa's side. Its packers are Mesa's own
 *    generated cle/v3d_packet_v71_pack.h from the reference build
 *    (tools/v3d/mesa_ref.sh, build/v3d-check/mesa/gen, or MESA_V3D_TREE's
 *    build-v3d), and the swizzle translation is Mesa's
 *    v3d_translate_pipe_swizzle (broadcom/common/v3d_util.c:211) linked
 *    from that build. The values are filled as Mesa's driver code fills
 *    them; that code is Gallium (and Vulkan) driver code, which the
 *    reference build cannot compile (it needs a pipe context), so it is
 *    transcribed here line for line, each rule with its Mesa 26.2.1
 *    file:line:
 *      gallium/drivers/v3d/v3dx_state.c:904-988, :1023-1034
 *          v3d_setup_texture_shader_state and the sampler view's state at
 *          V3D_VERSION >= 71 (transfer function, chroma fields)
 *      broadcom/vulkan/v3dvx_image.c:137-157
 *          the 7.1 R/B swap and reverse bits (Gallium never sets them)
 *      gallium/drivers/v3d/v3d_resource.c:757-779
 *          cube_map_stride and v3d_layer_offset
 *      gallium/drivers/v3d/v3dx_state.c:565-583, :586-767, :770-835
 *          translate_wrap, v3d_upload_sampler_state_variant,
 *          v3d_create_sampler_state's border variant
 *    (v3dx_state.c, v3dvx_image.c and v3d_resource.c are in the 26.2.1
 *    tarball third_party/mesa-26.2.1/UPSTREAM names, not vendored.)
 *  - without it: our side, v3d_tex71.c (at V3D_CL_VER 71, over the
 *    committed driver/src/v3d/generated/v3d_packet_v71_pack.h), with the
 *    layouts of v3d_tex.c (version-neutral, linked as the driver builds it,
 *    at 4.2) and v3d_tex.c's v3d_tex_p0/p1, which serve 7.1 unchanged.
 *
 * The two never share a translation unit: both packet headers define the
 * same V3D71_* names.
 *
 * Checks:
 *  1. texture shader state, every case of tex_cases[] (RGBA8 with and
 *     without sRGB, mip chains and a base level, cubes, a 2D array view
 *     from layer 1, a 3D texture, R/B swap and reverse, BGRA by swizzle,
 *     LT, UB-linear and UIF/XOR level 0s with ub_pad, R8, RGBA16F, RGBA32F,
 *     1x1 and 4096x4096): our 24 bytes equal Mesa's;
 *  2. the same records against words built from the 7.1 XML's bit
 *     positions (cle/v3d_packet.xml:1633-1707, each field with its line)
 *     for two cases written out field by field: an independent anchor that
 *     does not go through any generated packer;
 *  3. every sampler of 2 min x 3 mip x 2 mag filters x 5^3 wraps, LODs and
 *     biases off and on the u4.8/s8.8 grid, each compare function, the
 *     anisotropy steps, and the border variants 0000, 0001, 1111 and
 *     FOLLOWS (UNORM and 32-bit words, no f16 at 7.1): our bytes equal
 *     Mesa's;
 *  4. TMU parameters 0 and 1 (v3d_tex_p0/p1) against
 *     V3D71_TMU_CONFIG_PARAMETER_0/1_pack for every payload, and parameter 2
 *     (v3d_tex71_p2) against V3D71_TMU_CONFIG_PARAMETER_2_pack over every
 *     op, offsets and flag;
 *  5. the refusals, and negative controls that each must be detected.
 *
 * The last line is "tex71: N of N checks, 0 failed" when every check
 * passed and the number run is the number planned; exit 0 only then.
 */
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ---- shared between the two sides ------------------------------------------- */

enum tgt { T_2D, T_CUBE, T_ARRAY, T_3D };

/* Swizzles as Mesa's enum pipe_swizzle (util/format/u_formats.h:42-48). */
enum { SX, SY, SZ, SW, S0, S1 };

struct tcase {
	const char *name;
	enum tgt target;
	uint32_t w, h, d;  /* d: a 3D texture's depth or the layer count */
	uint32_t levels;   /* of the resource */
	uint32_t cpp, type;
	bool srgb;
	uint8_t swz[4];    /* the view's swizzle (pipe) */
	uint32_t first_level, last_level, first_layer, last_layer;
	bool swap_rb, reverse;
	uint32_t bo;       /* the BO's GPU address */
};

/* What Mesa's v3d_resource holds for the case, from the layout. */
struct mrsc {
	uint32_t off[16], size[16];
	int tiling0; /* Mesa's enum v3d_tiling_mode (v3d_tiling.h) */
	uint32_t ub_pad0;
};

/* Neutral wrap and filter names; each side maps them its own way. */
enum { W_REPEAT, W_CLAMP_TO_EDGE, W_MIRROR, W_BORDER, W_MIRROR_CLAMP_TO_EDGE, W_N };
enum { F_NEAREST, F_LINEAR };
enum { M_NONE, M_NEAREST, M_LINEAR };
/* The border variant of the view (v3d_context.h:105-134, the rows used). */
enum { BV_F16_UNORM = 4, BV_32 = 15 };

struct scase {
	int wrap[3];
	int min_img, mag_img, mip;
	float min_lod, max_lod, bias;
	bool compare;
	uint32_t compare_func;
	uint32_t max_aniso;
	float border[4];
	int view_variant; /* what v3d_create_sampler_view picks: BV_* */
};

#ifdef TEX71_MESA_SIDE
/* ============================================================================
 * Mesa's side
 * ============================================================================ */
#define NDEBUG
#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#define __gen_unpack_address(cl, s, e) ((uint32_t)(__gen_unpack_uint(cl, s, e) << (31 - ((e) - (s)))))
#include "cle/v3d_packet_v71_pack.h"
#include "util/format/u_formats.h"
#include "pipe/p_defines.h"

/* broadcom/common/v3d_util.h:73, without the header's libdrm include */
uint32_t v3d_translate_pipe_swizzle(enum pipe_swizzle swizzle);

/* Mesa's enum v3d_tiling_mode (broadcom/common/v3d_tiling.h) */
enum { V3D_TILING_UIF_NO_XOR = 4, V3D_TILING_UIF_XOR = 5 };

void mesa71_tex_state(const struct tcase *c, const struct mrsc *r, uint8_t out[24]);
void mesa71_sampler(const struct scase *c, uint8_t out[24]);
void mesa71_p0(uint32_t addr, uint32_t payload, uint8_t out[4]);
void mesa71_p1(uint32_t addr, uint32_t payload, uint8_t out[4]);
void mesa71_p2(uint32_t op, int s, int t, int rr, uint32_t flags, uint8_t out[4]);

void mesa71_tex_state(const struct tcase *c, const struct mrsc *r, uint8_t out[24])
{
	/* v3d_resource.c:761-766 */
	uint32_t cube_map_stride = c->target != T_3D ? ((r->off[0] + r->size[0] + 63) & ~63u)
						     : r->size[0];
	/* v3d_layer_offset (v3d_resource.c:771-779) at level 0 */
	uint32_t layer_off = c->target == T_3D ? r->off[0] + c->first_layer * r->size[0]
					       : r->off[0] + c->first_layer * cube_map_stride;

	struct V3D71_TEXTURE_SHADER_STATE tex;
	memset(&tex, 0, sizeof tex);
	/* v3d_setup_texture_shader_state (v3dx_state.c:904-988); no MSAA, no
	 * 1D, not sampling a cube array */
	tex.image_width = c->w;
	tex.image_height = c->h;
	tex.image_width &= (1 << 14) - 1;
	tex.image_height &= (1 << 14) - 1;
	if (c->target == T_3D)
		tex.image_depth = c->d;
	else
		tex.image_depth = (c->last_layer - c->first_layer) + 1;
	tex.base_level = c->first_level;
	tex.max_level = c->last_level;
	const uint32_t base_offset = c->bo + layer_off;
	tex.texture_base_pointer = base_offset;
	tex.array_stride_64_byte_aligned = cube_map_stride / 64;
	/* :963-969 */
	tex.chroma_offset_x = 1;
	tex.chroma_offset_y = 1;
	tex.texture_base_pointer_cb = base_offset >> 6;
	tex.texture_base_pointer_cr = base_offset >> 6;
	/* :976-987 */
	tex.level_0_is_strictly_uif =
		(r->tiling0 == V3D_TILING_UIF_XOR || r->tiling0 == V3D_TILING_UIF_NO_XOR);
	tex.level_0_xor_enable = (r->tiling0 == V3D_TILING_UIF_XOR);
	if (tex.level_0_is_strictly_uif) {
		tex.level_0_ub_pad = r->ub_pad0;
		tex.extended = true;
	}
	/* :1023-1034 */
	tex.transfer_func = c->srgb ? TRANSFER_FUNC_SRGB : TRANSFER_FUNC_NONE;
	tex.swizzle_r = v3d_translate_pipe_swizzle((enum pipe_swizzle)c->swz[0]);
	tex.swizzle_g = v3d_translate_pipe_swizzle((enum pipe_swizzle)c->swz[1]);
	tex.swizzle_b = v3d_translate_pipe_swizzle((enum pipe_swizzle)c->swz[2]);
	tex.swizzle_a = v3d_translate_pipe_swizzle((enum pipe_swizzle)c->swz[3]);
	tex.texture_type = c->type;
	/* v3dvx_image.c:142-157: the view swizzle stays as it is when either
	 * bit is set (the case's swizzle is that view swizzle) */
	tex.reverse = c->reverse;
	tex.r_b_swap = c->swap_rb;
	V3D71_TEXTURE_SHADER_STATE_pack(NULL, out, &tex);
}

/* v3dx_state.c:565-583 */
static enum V3D71_Wrap_Mode translate_wrap(enum pipe_tex_wrap pipe_wrap)
{
	switch (pipe_wrap) {
	case PIPE_TEX_WRAP_REPEAT:
		return V3D_WRAP_MODE_REPEAT;
	case PIPE_TEX_WRAP_CLAMP_TO_EDGE:
		return V3D_WRAP_MODE_CLAMP;
	case PIPE_TEX_WRAP_MIRROR_REPEAT:
		return V3D_WRAP_MODE_MIRROR;
	case PIPE_TEX_WRAP_CLAMP_TO_BORDER:
		return V3D_WRAP_MODE_BORDER;
	case PIPE_TEX_WRAP_MIRROR_CLAMP_TO_EDGE:
		return V3D_WRAP_MODE_MIRROR_ONCE;
	default:
		return 7;
	}
}

static enum pipe_tex_wrap pipe_wrap(int w)
{
	static const enum pipe_tex_wrap m[W_N] = { PIPE_TEX_WRAP_REPEAT, PIPE_TEX_WRAP_CLAMP_TO_EDGE,
						   PIPE_TEX_WRAP_MIRROR_REPEAT,
						   PIPE_TEX_WRAP_CLAMP_TO_BORDER,
						   PIPE_TEX_WRAP_MIRROR_CLAMP_TO_EDGE };
	return m[w];
}

static float clampf(float v, float lo, float hi)
{
	return v < lo ? lo : (v > hi ? hi : v);
}

/* The variants of v3d_context.h:105-134 this check reaches */
enum { VAR_0000 = 0, VAR_0001 = 1, VAR_1111 = 2 };

void mesa71_sampler(const struct scase *c, uint8_t out[24])
{
	union {
		float f[4];
		uint32_t ui[4];
	} bc, border;
	memcpy(bc.f, c->border, sizeof bc.f);
	enum pipe_tex_wrap ws = pipe_wrap(c->wrap[0]), wt = pipe_wrap(c->wrap[1]),
			   wr = pipe_wrap(c->wrap[2]);
	unsigned min_img = c->min_img == F_NEAREST ? PIPE_TEX_FILTER_NEAREST : PIPE_TEX_FILTER_LINEAR;
	unsigned mag_img = c->mag_img == F_NEAREST ? PIPE_TEX_FILTER_NEAREST : PIPE_TEX_FILTER_LINEAR;
	unsigned min_mip = c->mip == M_NONE	? PIPE_TEX_MIPFILTER_NONE
			   : c->mip == M_NEAREST ? PIPE_TEX_MIPFILTER_NEAREST
						 : PIPE_TEX_MIPFILTER_LINEAR;

	/* v3d_create_sampler_state (v3dx_state.c:781-813) */
	bool uses_border_color = translate_wrap(ws) == V3D_WRAP_MODE_BORDER ||
				 translate_wrap(wt) == V3D_WRAP_MODE_BORDER ||
				 translate_wrap(wr) == V3D_WRAP_MODE_BORDER;
	bool border_color_variants = false;
	int border_variant = VAR_0000;
	if (uses_border_color) {
		if (bc.ui[0] == 0 && bc.ui[1] == 0 && bc.ui[2] == 0 && bc.ui[3] == 0)
			border_variant = VAR_0000;
		else if (bc.ui[0] == 0 && bc.ui[1] == 0 && bc.ui[2] == 0 && bc.ui[3] == 0x3F800000)
			border_variant = VAR_0001;
		else if (bc.ui[0] == 0x3F800000 && bc.ui[1] == 0x3F800000 && bc.ui[2] == 0x3F800000 &&
			 bc.ui[3] == 0x3F800000)
			border_variant = VAR_1111;
		else
			border_color_variants = true;
	}
	/* write_tmu_p1 (v3d_uniforms.c) picks the view's variant then */
	int variant = border_color_variants ? c->view_variant : border_variant;

	/* v3d_upload_sampler_state_variant (v3dx_state.c:586-767) */
	struct V3D71_SAMPLER_STATE sampler;
	memset(&sampler, 0, sizeof sampler);
	sampler.wrap_i_border = false;
	sampler.wrap_s = translate_wrap(ws);
	sampler.wrap_t = translate_wrap(wt);
	sampler.wrap_r = translate_wrap(wr);
	sampler.fixed_bias = c->bias;
	sampler.depth_compare_function = c->compare ? c->compare_func : V3D_COMPARE_FUNC_NEVER;
	sampler.min_filter_nearest = min_img == PIPE_TEX_FILTER_NEAREST;
	sampler.mag_filter_nearest = mag_img == PIPE_TEX_FILTER_NEAREST;
	sampler.mip_filter_nearest = min_mip != PIPE_TEX_MIPFILTER_LINEAR;
	sampler.min_level_of_detail = fminf(fmaxf(0, c->min_lod), 15);
	sampler.max_level_of_detail = fminf(fmaxf(c->max_lod, c->min_lod), 15);
	if (min_mip == PIPE_TEX_MIPFILTER_NONE) {
		sampler.min_level_of_detail = fminf(sampler.min_level_of_detail, 1.0 / 256.0);
		sampler.max_level_of_detail = fminf(sampler.max_level_of_detail, 1.0 / 256.0);
	}
	if (c->max_aniso) {
		sampler.anisotropy_enable = true;
		if (c->max_aniso > 8)
			sampler.maximum_anisotropy = 3;
		else if (c->max_aniso > 4)
			sampler.maximum_anisotropy = 2;
		else if (c->max_aniso > 2)
			sampler.maximum_anisotropy = 1;
	}
	if (variant == VAR_0000) {
		sampler.border_color_mode = V3D_BORDER_COLOR_0000;
	} else if (variant == VAR_0001) {
		sampler.border_color_mode = V3D_BORDER_COLOR_0001;
	} else if (variant == VAR_1111) {
		sampler.border_color_mode = V3D_BORDER_COLOR_1111;
	} else {
		sampler.border_color_mode = V3D_BORDER_COLOR_FOLLOWS;
		/* the reswizzle (:653-686) is the default branch for both
		 * variants here; the clamp (:689-747) for the UNORM one */
		border = bc;
		if (variant == BV_F16_UNORM)
			for (int i = 0; i < 4; i++)
				border.f[i] = clampf(border.f[i], 0, 1);
		/* :749-760 is V3D_VERSION == 42 only: no f16 at 7.1 */
		sampler.border_color_word_0 = border.ui[0];
		sampler.border_color_word_1 = border.ui[1];
		sampler.border_color_word_2 = border.ui[2];
		sampler.border_color_word_3 = border.ui[3];
	}
	V3D71_SAMPLER_STATE_pack(NULL, out, &sampler);
}

/* write_tmu_p0 / write_tmu_p1 (v3d_uniforms.c): the record's address ORed
 * with the compiler's payload, as these structs pack it */
void mesa71_p0(uint32_t addr, uint32_t payload, uint8_t out[4])
{
	struct V3D71_TMU_CONFIG_PARAMETER_0 p = { .texture_state_address = addr,
						  .return_words_of_texture_data = payload };
	V3D71_TMU_CONFIG_PARAMETER_0_pack(NULL, out, &p);
}

void mesa71_p1(uint32_t addr, uint32_t payload, uint8_t out[4])
{
	struct V3D71_TMU_CONFIG_PARAMETER_1 p = { .sampler_state_address = addr,
						  .output_type_32_bit = payload & 1,
						  .unnormalized_coordinates = (payload >> 1) & 1,
						  .per_pixel_mask_enable = (payload >> 2) & 1 };
	V3D71_TMU_CONFIG_PARAMETER_1_pack(NULL, out, &p);
}

void mesa71_p2(uint32_t op, int s, int t, int rr, uint32_t flags, uint8_t out[4])
{
	struct V3D71_TMU_CONFIG_PARAMETER_2 p;
	memset(&p, 0, sizeof p);
	p.op = op;
	p.offset_s = s;
	p.offset_t = t;
	p.offset_r = rr;
	p.gather_mode = flags & 1;
	p.gather_component = (flags >> 1) & 3;
	p.coefficient_mode = (flags >> 3) & 1;
	p.sample_number = (flags >> 4) & 3;
	p.disable_autolod = (flags >> 6) & 1;
	p.offset_format_8 = (flags >> 7) & 1;
	p.lod_query = (flags >> 8) & 1;
	p.dim_query = (flags >> 9) & 1;
	p.write_conversion = (flags >> 10) & 1;
	V3D71_TMU_CONFIG_PARAMETER_2_pack(NULL, out, &p);
}

#else
/* ============================================================================
 * Our side and the checks
 * ============================================================================ */
#define V3D_CL_VER 71
#include "v3d_tex.h"
#include "v3d_tex71.h"

void mesa71_tex_state(const struct tcase *c, const struct mrsc *r, uint8_t out[24]);
void mesa71_sampler(const struct scase *c, uint8_t out[24]);
void mesa71_p0(uint32_t addr, uint32_t payload, uint8_t out[4]);
void mesa71_p1(uint32_t addr, uint32_t payload, uint8_t out[4]);
void mesa71_p2(uint32_t op, int s, int t, int rr, uint32_t flags, uint8_t out[4]);

static int run, failed;

static void check(bool ok, const char *what)
{
	run++;
	if (!ok) {
		failed++;
		if (failed <= 40)
			printf("FAIL: %s\n", what);
	}
}

static void hex(const char *tag, const uint8_t *b, int n)
{
	printf("  %s", tag);
	for (int i = 0; i < n; i++)
		printf(" %02x", b[i]);
	printf("\n");
}

#define RGBA { SX, SY, SZ, SW }
static const struct tcase tex_cases[] = {
	{ "rgba8 64x64", T_2D, 64, 64, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, false, false, 0x10000000 },
	{ "rgba8 srgb 64x64", T_2D, 64, 64, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, true, RGBA, 0, 0, 0, 0, false, false, 0x10000000 },
	{ "rgba8 13x7 lt", T_2D, 13, 7, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, false, false, 0x00400000 },
	{ "rgba8 1x1", T_2D, 1, 1, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, false, false, 0x00001000 },
	{ "rgba8 mip 256x128", T_2D, 256, 128, 1, 9, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 8, 0, 0, false, false, 0x20000000 },
	{ "rgba8 srgb mip 256x128", T_2D, 256, 128, 1, 9, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, true, RGBA, 0, 8, 0, 0, false, false, 0x20000000 },
	{ "rgba8 mip 300x200 levels 2..5", T_2D, 300, 200, 1, 9, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 2, 5, 0, 0, false, false, 0x20100000 },
	{ "rgba8 uif 1024x1000", T_2D, 1024, 1000, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, false, false, 0x30000000 },
	{ "rgba8 uif mip 1920x1080", T_2D, 1920, 1080, 1, 11, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, true, RGBA, 0, 10, 0, 0, false, false, 0x30000000 },
	{ "rgba8 4096x4096 13 levels", T_2D, 4096, 4096, 1, 13, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 12, 0, 0, false, false, 0x40000000 },
	{ "cube rgba8 64 mip", T_CUBE, 64, 64, 6, 7, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 6, 0, 5, false, false, 0x11000000 },
	{ "cube rgba8 srgb 128", T_CUBE, 128, 128, 6, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, true, RGBA, 0, 0, 0, 5, false, false, 0x11100000 },
	{ "cube rgba8 512 uif mip", T_CUBE, 512, 512, 6, 10, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 9, 0, 5, false, false, 0x11200000 },
	{ "array rgba8 32x32x4 layers 1..3", T_ARRAY, 32, 32, 4, 6, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 5, 1, 3, false, false, 0x12000000 },
	{ "3d rgba8 16x16x8 mip", T_3D, 16, 16, 8, 5, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 4, 0, 0, false, false, 0x13000000 },
	{ "bgra8 by swizzle", T_2D, 64, 32, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, { SZ, SY, SX, SW }, 0, 0, 0, 0, false, false, 0x14000000 },
	{ "bgra8 by r/b swap", T_2D, 64, 32, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, true, false, 0x14000000 },
	{ "abgr reverse", T_2D, 64, 32, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, RGBA, 0, 0, 0, 0, false, true, 0x14000000 },
	{ "argb reverse + swap srgb", T_2D, 64, 32, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, true, RGBA, 0, 0, 0, 0, true, true, 0x14000000 },
	{ "rgbx view swizzle 1", T_2D, 128, 64, 1, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, false, { SX, SY, SZ, S1 }, 0, 0, 0, 0, false, false, 0x15000000 },
	{ "r8 luminance", T_2D, 100, 60, 1, 7, 1, V3D_TEXTURE_DATA_FORMAT_R8, false, { SX, SX, SX, S1 }, 0, 6, 0, 0, false, false, 0x16000000 },
	{ "r8 alpha", T_2D, 33, 17, 1, 1, 1, V3D_TEXTURE_DATA_FORMAT_R8, false, { S0, S0, S0, SX }, 0, 0, 0, 0, false, false, 0x16100000 },
	{ "rgba16f 64x64 mip", T_2D, 64, 64, 1, 7, 8, V3D_TEXTURE_DATA_FORMAT_RGBA16F, false, RGBA, 0, 6, 0, 0, false, false, 0x17000000 },
	{ "rgba32f 40x24", T_2D, 40, 24, 1, 1, 16, V3D_TEXTURE_DATA_FORMAT_RGBA32F, false, RGBA, 0, 0, 0, 0, false, false, 0x17100000 },
};
#define N_TEX ((int)(sizeof tex_cases / sizeof tex_cases[0]))

static uint8_t v3d_swz(uint8_t p)
{
	/* v3d_translate_pipe_swizzle's table, the caller's job in the driver */
	return p == S0 ? V3D_SWIZZLE_ZERO : p == S1 ? V3D_SWIZZLE_ONE : (uint8_t)(V3D_SWIZZLE_RED + p);
}

/* our side of one case: the layout (v3d_tex.c), the state, the record */
static bool ours_tex(const struct tcase *c, struct v3d_tex71_state *s, struct v3d_tex_layout *l,
		     uint8_t out[24])
{
	bool ok = c->target == T_3D ? v3d_tex_layout_3d(c->w, c->h, c->d, c->levels, c->cpp, l)
				    : v3d_tex_layout(c->w, c->h, c->levels, c->cpp, l);
	if (!ok)
		return false;
	memset(s, 0, sizeof *s);
	uint32_t off;
	if (c->target == T_3D) {
		v3d_tex71_state_from_layout_3d(s, l);
		if (!v3d_tex_slice_offset(l, 0, c->first_layer, c->d, &off))
			return false;
		s->depth = c->d;
	} else {
		v3d_tex71_state_from_layout(s, l);
		if (!v3d_tex_layer_offset(l, 0, c->first_layer, c->d, &off))
			return false;
		s->depth = c->last_layer - c->first_layer + 1;
	}
	s->base = c->bo + off;
	s->width = c->w;
	s->height = c->h;
	s->base_level = c->first_level;
	s->max_level = c->last_level;
	s->type = c->type;
	for (int i = 0; i < 4; i++)
		s->swizzle[i] = v3d_swz(c->swz[i]);
	s->transfer_func = c->srgb ? V3D_TEX71_TRANSFER_SRGB : V3D_TEX71_TRANSFER_NONE;
	s->r_b_swap = c->swap_rb;
	s->reverse = c->reverse;
	return v3d_tex71_pack_state(out, s);
}

static void mrsc_from(const struct v3d_tex_layout *l, struct mrsc *r)
{
	memset(r, 0, sizeof *r);
	for (uint32_t i = 0; i < l->levels; i++) {
		r->off[i] = l->slices[i].offset;
		r->size[i] = l->slices[i].size;
	}
	r->tiling0 = (int)l->slices[0].tiling; /* the same enum values */
	r->ub_pad0 = l->slices[0].ub_pad;
}

static int seen_uif, seen_xor, seen_ub_pad, seen_ublinear, seen_lt;

static void check_tex_cases(void)
{
	for (int i = 0; i < N_TEX; i++) {
		const struct tcase *c = &tex_cases[i];
		struct v3d_tex71_state s;
		struct v3d_tex_layout l;
		struct mrsc r;
		uint8_t ours[24], mesa[24];
		bool ok = ours_tex(c, &s, &l, ours);
		if (ok) {
			mrsc_from(&l, &r);
			mesa71_tex_state(c, &r, mesa);
			ok = memcmp(ours, mesa, 24) == 0;
			enum v3d_tex_tiling t = l.slices[0].tiling;
			seen_uif += t == V3D_TEX_TILING_UIF_NO_XOR || t == V3D_TEX_TILING_UIF_XOR;
			seen_xor += t == V3D_TEX_TILING_UIF_XOR;
			seen_ub_pad += s.level_0_ub_pad != 0;
			seen_ublinear += t == V3D_TEX_TILING_UBLINEAR_1_COLUMN ||
					 t == V3D_TEX_TILING_UBLINEAR_2_COLUMN;
			seen_lt += t == V3D_TEX_TILING_LINEARTILE;
		}
		char what[128];
		snprintf(what, sizeof what, "texture state %s equals Mesa's", c->name);
		check(ok, what);
		if (!ok) {
			hex("ours", ours, 24);
			hex("mesa", mesa, 24);
		}
	}
	/* the cases reach every level-0 tiling and a UIF pad */
	check(seen_lt && seen_ublinear && seen_uif && seen_xor && seen_ub_pad,
	      "the texture cases reach LT, UB-linear, UIF, XOR and ub_pad at level 0");
}

/* ---- 2. the 7.1 XML's bit positions, without a generated packer ------------ */

static void put(uint8_t *b, unsigned start, unsigned size, uint64_t v)
{
	for (unsigned i = 0; i < size; i++)
		if ((v >> i) & 1)
			b[(start + i) / 8] |= (uint8_t)(1u << ((start + i) % 8));
}

struct xml_rec {
	uint32_t base, stride64, w, h, d, type, swz[4], maxl, basel, ubpad;
	bool rbswap, reverse, ext, xor_en, uif;
	uint32_t tf;
};

static void xml_words(const struct xml_rec *x, uint8_t b[24])
{
	memset(b, 0, 24);
	/* cle/v3d_packet.xml (third_party/mesa-26.2.1), struct "Texture Shader
	 * State" min_ver 71, :1633-1707 */
	put(b, 0, 32, x->base);           /* :1694 Texture base pointer (address) */
	put(b, 2, 3, x->tf);              /* :1697 Transfer func */
	put(b, 5, 1, x->reverse);         /* :1696 Reverse */
	put(b, 32, 1, x->rbswap);         /* :1692 R/B swap */
	put(b, 33, 24, x->stride64);      /* :1691 Array Stride (64-byte aligned) */
	put(b, 58, 14, x->w);             /* :1684 Image Width */
	put(b, 72, 14, x->h);             /* :1683 Image Height */
	put(b, 86, 14, x->d);             /* :1682 Image Depth */
	put(b, 100, 7, x->type);          /* :1681 Texture type */
	put(b, 107, 1, x->ext);           /* :1679 Extended */
	put(b, 108, 3, x->swz[0]);        /* :1678 Swizzle R */
	put(b, 111, 3, x->swz[1]);        /* :1677 Swizzle G */
	put(b, 114, 3, x->swz[2]);        /* :1676 Swizzle B */
	put(b, 117, 3, x->swz[3]);        /* :1667 Swizzle A */
	put(b, 120, 4, x->maxl);          /* :1665 Max Level */
	put(b, 124, 4, x->basel);         /* :1664 Base Level */
	put(b, 128, 4, x->ubpad);         /* :1662 Level 0 UB_PAD */
	put(b, 132, 1, x->xor_en);        /* :1661 Level 0 XOR enable */
	put(b, 134, 1, x->uif);           /* :1660 Level 0 is strictly UIF */
	put(b, 136, 1, 1);                /* :1657 Chroma offset x (Mesa: 1) */
	put(b, 137, 1, 1);                /* :1656 Chroma offset y (Mesa: 1) */
	put(b, 138, 26, x->base >> 6);    /* :1655 texture base pointer Cb */
	put(b, 164, 26, x->base >> 6);    /* :1654 texture_base pointer_Cr */
}

static void check_xml_anchors(void)
{
	/* An sRGB RGBA8 64x32 single-level LT texture at 0x10001000 with an
	 * array stride of 0x123 x 64 and R/B swap: every field written by
	 * hand. */
	struct v3d_tex71_state s = {
		.base = 0x10001000, .width = 64, .height = 32, .depth = 1, .base_level = 0,
		.max_level = 0, .type = V3D_TEXTURE_DATA_FORMAT_RGBA8,
		.swizzle = { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA },
		.transfer_func = V3D_TEX71_TRANSFER_SRGB, .r_b_swap = true, .array_stride_64 = 0x123,
	};
	struct xml_rec x = { .base = 0x10001000, .stride64 = 0x123, .w = 64, .h = 32, .d = 1,
			     .type = 4, .swz = { 2, 3, 4, 5 }, .rbswap = true, .tf = 1 };
	uint8_t ours[24], want[24];
	xml_words(&x, want);
	bool ok = v3d_tex71_pack_state(ours, &s) && memcmp(ours, want, 24) == 0;
	check(ok, "XML anchor 1 (sRGB, R/B swap, LT) equals the 7.1 bit positions");
	if (!ok) {
		hex("ours", ours, 24);
		hex("xml ", want, 24);
	}
	/* The first words, written out: base | sRGB << 2; then R/B swap in
	 * bit 32, stride << 33, width << 58 */
	check(ours[0] == 0x04 && ours[1] == 0x10 && ours[2] == 0x00 && ours[3] == 0x10 &&
		      ours[4] == ((0x123 << 1) & 0xff) + 1,
	      "XML anchor 1: word 0 is 0x10001004, byte 4 is (stride << 1) | R/B swap");

	/* A UIF XOR level 0 with ub_pad 3, levels 2..9, reverse, a 3D-sized
	 * depth and the widest fields */
	struct v3d_tex71_state u = {
		.base = 0xfffc0040, .width = 16383, .height = 9000, .depth = 16383, .base_level = 2,
		.max_level = 9, .type = V3D_TEXTURE_DATA_FORMAT_RGBA32F,
		.swizzle = { V3D_SWIZZLE_ALPHA, V3D_SWIZZLE_ONE, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_BLUE },
		.reverse = true, .level_0_is_strictly_uif = true, .level_0_xor_enable = true,
		.extended = true, .level_0_ub_pad = 3, .array_stride_64 = 0xffffff,
	};
	struct xml_rec y = { .base = 0xfffc0040, .stride64 = 0xffffff, .w = 16383, .h = 9000,
			     .d = 16383, .type = 31, .swz = { 5, 1, 0, 4 }, .maxl = 9, .basel = 2,
			     .ubpad = 3, .reverse = true, .ext = true, .xor_en = true, .uif = true };
	xml_words(&y, want);
	ok = v3d_tex71_pack_state(ours, &u) && memcmp(ours, want, 24) == 0;
	check(ok, "XML anchor 2 (UIF XOR, ub_pad, reverse, widest fields) equals the 7.1 bit positions");
	if (!ok) {
		hex("ours", ours, 24);
		hex("xml ", want, 24);
	}
}

/* ---- 3. samplers ---------------------------------------------------------------- */

static void ours_sampler_from(const struct scase *c, struct v3d_tex71_sampler *s)
{
	/* the GL core's mapping, GL wrap to the 7.1 Wrap Mode (v3dx_state.c:565-583) */
	static const uint32_t wm[W_N] = { V3D_TEX71_WRAP_REPEAT, V3D_TEX71_WRAP_CLAMP,
					  V3D_TEX71_WRAP_MIRROR, V3D_TEX71_WRAP_BORDER,
					  V3D_TEX71_WRAP_MIRROR_ONCE };
	memset(s, 0, sizeof *s);
	s->wrap_s = wm[c->wrap[0]];
	s->wrap_t = wm[c->wrap[1]];
	s->wrap_r = wm[c->wrap[2]];
	s->min_nearest = c->min_img == F_NEAREST;
	s->mag_nearest = c->mag_img == F_NEAREST;
	s->mip_nearest = c->mip != M_LINEAR;
	s->mip_none = c->mip == M_NONE;
	s->min_lod = c->min_lod;
	s->max_lod = c->max_lod;
	s->lod_bias = c->bias;
	s->compare_func = c->compare ? c->compare_func : 0;
	s->max_anisotropy = c->max_aniso;
	bool border = c->wrap[0] == W_BORDER || c->wrap[1] == W_BORDER || c->wrap[2] == W_BORDER;
	const float *b = c->border;
	if (!border || (b[0] == 0 && b[1] == 0 && b[2] == 0 && b[3] == 0))
		s->border_color_mode = V3D_TEX71_BORDER_0000;
	else if (b[0] == 0 && b[1] == 0 && b[2] == 0 && b[3] == 1)
		s->border_color_mode = V3D_TEX71_BORDER_0001;
	else if (b[0] == 1 && b[1] == 1 && b[2] == 1 && b[3] == 1)
		s->border_color_mode = V3D_TEX71_BORDER_1111;
	else {
		s->border_color_mode = V3D_TEX71_BORDER_FOLLOWS;
		/* the cases' UNORM colours are in 0..1: the variant's clamp
		 * leaves them as they are */
		memcpy(s->border_color, b, sizeof s->border_color);
	}
}

static int n_samplers, sampler_bad;

static void one_sampler(const struct scase *c, const char *name)
{
	struct v3d_tex71_sampler s;
	uint8_t ours[24], mesa[24];
	ours_sampler_from(c, &s);
	bool ok = v3d_tex71_pack_sampler(ours, &s);
	if (ok) {
		mesa71_sampler(c, mesa);
		ok = memcmp(ours, mesa, 24) == 0;
	}
	n_samplers++;
	if (!ok && ++sampler_bad <= 20) {
		printf("FAIL: sampler %s (wrap %d %d %d, min %d mag %d mip %d, lod %g..%g bias %g, "
		       "compare %d/%u, aniso %u)\n",
		       name, c->wrap[0], c->wrap[1], c->wrap[2], c->min_img, c->mag_img, c->mip,
		       (double)c->min_lod, (double)c->max_lod, (double)c->bias, c->compare,
		       c->compare_func, c->max_aniso);
		hex("ours", ours, 24);
		hex("mesa", mesa, 24);
	}
}

#define N_LODS 8
static const float lods[N_LODS][3] = {
	{ 0, 1000, 0 },	   { 0, 15, 0 },	{ 0.3f, 2.7f, 0.5f }, { 1, 1, -1.37f },
	{ 2.5f, 0.5f, 3.999f }, { 0, 20, -127.5f }, { 14.99f, 15, 127.99f }, { 0.001f, 0.002f, 0.001f },
};

static void check_samplers(void)
{
	int planned = 0;
	/* every filter x wrap, at each LOD set */
	for (int li = 0; li < N_LODS; li++)
		for (int mi = 0; mi < 2; mi++)
			for (int mp = 0; mp < 3; mp++)
				for (int mg = 0; mg < 2; mg++)
					for (int w = 0; w < W_N * W_N * W_N; w++) {
						struct scase c = { { w % W_N, (w / W_N) % W_N, w / (W_N * W_N) },
								   mi, mg, mp, lods[li][0], lods[li][1],
								   lods[li][2], false, 0, 0, { 0, 0, 0, 0 },
								   BV_F16_UNORM };
						one_sampler(&c, "grid");
						planned++;
					}
	/* depth compare, each function, compare off */
	for (uint32_t f = 0; f < 8; f++)
		for (int on = 0; on < 2; on++) {
			struct scase c = { { W_CLAMP_TO_EDGE, W_CLAMP_TO_EDGE, W_CLAMP_TO_EDGE },
					   F_LINEAR, F_LINEAR, M_NONE, 0, 1000, 0, on, f, 0,
					   { 0, 0, 0, 0 }, BV_F16_UNORM };
			one_sampler(&c, "compare");
			planned++;
		}
	/* anisotropy */
	static const uint32_t an[] = { 0, 1, 2, 3, 4, 5, 8, 9, 16 };
	for (unsigned i = 0; i < sizeof an / sizeof an[0]; i++) {
		struct scase c = { { W_REPEAT, W_REPEAT, W_REPEAT }, F_LINEAR, F_LINEAR, M_LINEAR,
				   0, 1000, 0, false, 0, an[i], { 0, 0, 0, 0 }, BV_F16_UNORM };
		one_sampler(&c, "anisotropy");
		planned++;
	}
	/* border variants, with the border on each axis and off */
	static const float colours[][4] = {
		{ 0, 0, 0, 0 }, { 0, 0, 0, 1 }, { 1, 1, 1, 1 }, { 0.25f, 0.5f, 0.75f, 1 },
		{ 2, -1, 0.5f, 3 }, { 1, 0, 0, 1 },
	};
	static const int variants[] = { BV_F16_UNORM, BV_F16_UNORM, BV_F16_UNORM, BV_F16_UNORM,
					BV_32, BV_F16_UNORM };
	for (unsigned k = 0; k < sizeof colours / sizeof colours[0]; k++)
		for (int axis = 0; axis < 4; axis++) {
			struct scase c = { { W_REPEAT, W_MIRROR, W_CLAMP_TO_EDGE }, F_NEAREST,
					   F_LINEAR, M_NEAREST, 0, 1000, 0, false, 0, 0,
					   { colours[k][0], colours[k][1], colours[k][2], colours[k][3] },
					   variants[k] };
			if (axis < 3)
				c.wrap[axis] = W_BORDER;
			one_sampler(&c, "border");
			planned++;
		}
	check(n_samplers == planned, "every sampler case ran");
	char what[96];
	snprintf(what, sizeof what, "%d samplers equal Mesa's (%d differ)", n_samplers, sampler_bad);
	check(sampler_bad == 0, what);
	printf("tex71: %d texture records, %d samplers compared with Mesa's\n", N_TEX, n_samplers);
}

/* ---- 3b. Mesa's 7.1 references (tools/v3d/pi5/gl_tex2d, gl_texfmt, gl_cube) ------ */

/* tex71.sh reads the records from tools/v3d/pi5/gl_*.clif (through
 * pi5/src/texstate71.py) into a file of one line per texture job:
 * "<shape> <job> <P0> <P1> <6 texture words> <6 sampler words>", all hex.
 * Each shape's GL setup (pi5/README.md, "The GL calls") is filled below
 * into our structs the way the GL core would fill them, packed, and
 * compared with those words. A reference line our table has no shape for,
 * and a table shape the file lacks, both fail. gl_mipgen's texture and
 * sampler are included (a mip-chain case; its TFU is tfu71ref's). */
struct rshape {
	const char *shape;
	int job;
	/* the texture's GL setup */
	uint32_t w, h, levels, cpp, type, layers;
	bool srgb;
	uint8_t swz[4]; /* pipe swizzle of the format's view */
	uint32_t last_level;
	/* the sampler's GL setup */
	int wrap, min_img, mag_img, mip;
	float max_lod;
};

static const struct rshape rshapes[] = {
	/* gl_tex2d: GL_RGBA8 64x64, REPEAT, NEAREST then LINEAR */
	{ "gl_tex2d", 1, 64, 64, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 1, false, RGBA, 0, W_REPEAT, F_NEAREST, F_NEAREST, M_NONE, 1000 },
	{ "gl_tex2d", 2, 64, 64, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 1, false, RGBA, 0, W_REPEAT, F_LINEAR, F_LINEAR, M_NONE, 1000 },
	/* gl_texfmt: GL_SRGB8_ALPHA8, then GL_RGB565 (swizzle R G B 1) */
	{ "gl_texfmt", 1, 64, 64, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 1, true, RGBA, 0, W_REPEAT, F_NEAREST, F_NEAREST, M_NONE, 1000 },
	{ "gl_texfmt", 2, 64, 64, 1, 2, V3D_TEXTURE_DATA_FORMAT_RGB565, 1, false, { SX, SY, SZ, S1 }, 0, W_REPEAT, F_NEAREST, F_NEAREST, M_NONE, 1000 },
	/* gl_mipgen: glTexStorage2D 7 levels, LINEAR_MIPMAP_LINEAR / LINEAR */
	{ "gl_mipgen", 1, 64, 64, 7, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 1, false, RGBA, 6, W_REPEAT, F_LINEAR, F_LINEAR, M_LINEAR, 1000 },
	/* gl_cube: six 4x4 faces as layers, wrap CLAMP_TO_EDGE (the state
	 * tracker's seamless forcing makes R CLAMP too); NEAREST in odd jobs,
	 * LINEAR in even ones */
	{ "gl_cube", 1, 4, 4, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 6, false, RGBA, 0, W_CLAMP_TO_EDGE, F_NEAREST, F_NEAREST, M_NONE, 1000 },
	{ "gl_cube", 2, 4, 4, 1, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8, 6, false, RGBA, 0, W_CLAMP_TO_EDGE, F_LINEAR, F_LINEAR, M_NONE, 1000 },
};
#define N_RSHAPES ((int)(sizeof rshapes / sizeof rshapes[0]))

static const struct rshape *rshape_for(const char *shape, int job)
{
	int want = job;
	if (!strcmp(shape, "gl_cube"))
		want = (job & 1) ? 1 : 2;
	for (int i = 0; i < N_RSHAPES; i++)
		if (!strcmp(rshapes[i].shape, shape) && rshapes[i].job == want)
			return &rshapes[i];
	return NULL;
}

static void words_to_bytes(const uint32_t w[6], uint8_t b[24])
{
	for (int i = 0; i < 6; i++)
		for (int k = 0; k < 4; k++)
			b[i * 4 + k] = (uint8_t)(w[i] >> (8 * k));
}

static void check_references(const char *path)
{
	FILE *f = fopen(path, "r");
	if (!f) {
		check(false, "the reference records file opens");
		check(false, "every shape of the table is in the references");
		check(false, "every reference record equals ours");
		return;
	}
	char line[1024];
	int n = 0, bad = 0, unknown = 0;
	int seen[16] = { 0 };
	int tex_ok = 0, smp_ok = 0, p0_ok = 0, p1_ok = 0;
	while (fgets(line, sizeof line, f)) {
		char *tok[20];
		int nt = 0;
		for (char *t = strtok(line, " \t\r\n"); t && nt < 20; t = strtok(NULL, " \t\r\n"))
			tok[nt++] = t;
		if (nt != 16)
			continue;
		const struct rshape *r = rshape_for(tok[0], atoi(tok[1]));
		if (!r) {
			printf("FAIL: reference %s job %s: no shape in this table\n", tok[0], tok[1]);
			unknown++;
			continue;
		}
		seen[r - rshapes] = 1;
		uint32_t p0 = (uint32_t)strtoul(tok[2], NULL, 16), p1 = (uint32_t)strtoul(tok[3], NULL, 16);
		uint32_t tw[6], sw[6];
		for (int i = 0; i < 6; i++) {
			tw[i] = (uint32_t)strtoul(tok[4 + i], NULL, 16);
			sw[i] = (uint32_t)strtoul(tok[10 + i], NULL, 16);
		}
		uint8_t want_t[24], want_s[24], ours[24] = { 0 };
		words_to_bytes(tw, want_t);
		words_to_bytes(sw, want_s);
		n++;

		/* texture: layout and fields as the GL core fills them. Only
		 * the base address is Mesa's (the shim's 4 KiB buffers): taken
		 * from the reference; the Cb and Cr base fields our packer
		 * derives from it (>> 6) are still compared. */
		struct v3d_tex_layout l;
		struct v3d_tex71_state s;
		uint32_t off = 0;
		bool ok = v3d_tex_layout(r->w, r->h, r->levels, r->cpp, &l);
		if (ok) {
			memset(&s, 0, sizeof s);
			v3d_tex71_state_from_layout(&s, &l);
			ok = v3d_tex_layer_offset(&l, 0, 0, r->layers, &off);
		}
		bool tex_same = false;
		if (ok) {
			s.base = tw[0] & ~0x3fu;
			s.width = r->w;
			s.height = r->h;
			s.depth = r->layers;
			s.max_level = r->last_level;
			s.type = r->type;
			for (int i = 0; i < 4; i++)
				s.swizzle[i] = v3d_swz(r->swz[i]);
			s.transfer_func = r->srgb ? V3D_TEX71_TRANSFER_SRGB : V3D_TEX71_TRANSFER_NONE;
			tex_same = v3d_tex71_pack_state(ours, &s) && memcmp(ours, want_t, 24) == 0;
		}
		if (!tex_same) {
			printf("FAIL: %s job %s: texture record differs\n", tok[0], tok[1]);
			hex("ours", ours, 24);
			hex("ref ", want_t, 24);
		}
		tex_ok += tex_same;

		struct scase sc = { { r->wrap, r->wrap, r->wrap }, r->min_img, r->mag_img, r->mip, 0,
				    r->max_lod, 0, false, 0, 0, { 0, 0, 0, 0 }, BV_F16_UNORM };
		struct v3d_tex71_sampler ss;
		ours_sampler_from(&sc, &ss);
		memset(ours, 0, sizeof ours);
		bool smp_same = v3d_tex71_pack_sampler(ours, &ss) && memcmp(ours, want_s, 24) == 0;
		if (!smp_same) {
			printf("FAIL: %s job %s: sampler record differs\n", tok[0], tok[1]);
			hex("ours", ours, 24);
			hex("ref ", want_s, 24);
		}
		smp_ok += smp_same;

		/* P0 and P1: the record address taken from the reference, the
		 * flag bits (return words, per-pixel mask, ...) compared */
		uint32_t w0 = 0, w1 = 0;
		bool a = v3d_tex_p0(p0 & ~0xfu, 0x05000000u | 3, &w0) && w0 == p0;
		/* Mesa's sampler records sit 8-byte aligned in its state buffer
		 * (0x...78, 0x...e8, 0x...68 in the references; the field takes
		 * bits 3 up), where v3d_tex_p1 (a 4.2 file, left unchanged)
		 * wants V3D_TEX_RECORD_ALIGN (32): the flags are compared with a
		 * 32-aligned stand-in address, and the address replaced by the
		 * reference's. */
		bool b = v3d_tex_p1(0x1000, 0x03000000u | (p1 & 7u), &w1) && w1 == (0x1000u | (p1 & 7u));
		if (!a || !b)
			printf("FAIL: %s job %s: P0 %08x/%08x P1 %08x/%08x\n", tok[0], tok[1], p0, w0, p1, w1);
		p0_ok += a;
		p1_ok += b;
		bad += !(tex_same && smp_same && a && b);
		printf("  %s job %s: texture %s, sampler %s, P0 %s, P1 %s\n", tok[0], tok[1],
		       tex_same ? "equal" : "DIFFERS", smp_same ? "equal" : "DIFFERS",
		       a ? "equal" : "DIFFERS", b ? "equal" : "DIFFERS");
	}
	fclose(f);
	int shapes_seen = 0;
	for (int i = 0; i < N_RSHAPES; i++)
		shapes_seen += seen[i];
	check(unknown == 0 && n > 0, "every reference record has a shape in the table");
	check(shapes_seen == N_RSHAPES, "every shape of the table is in the references");
	check(bad == 0, "every reference texture record, sampler, P0 and P1 equals ours (0 differences)");
	printf("tex71: reference jobs %d: %d texture, %d sampler, %d P0, %d P1 equal; skipped, "
	       "taken from the reference: the texture base address (word 0 bits 6.., the Cb/Cr base "
	       "fields are derived from it and compared) and the P0/P1 record addresses\n",
	       n, tex_ok, smp_ok, p0_ok, p1_ok);
	printf("tex71: Mesa's choice, not a bug: its sampler records are 8-byte aligned (P1 address "
	       "bits 3 up), v3d_tex_p1 accepts only 32-byte aligned ones; every P1 flag word "
	       "(per-pixel mask) equals with a 32-aligned stand-in address\n");
	printf("tex71: not in the references: TMU P2 (each fragment shader has only the P0 and P1 "
	       "uniforms; P2 is the compiler's word, not Mesa's state); R/B swap and reverse "
	       "(Gallium never sets them: 0 in every record)\n");
}

/* ---- 4. TMU parameters ---------------------------------------------------------- */

static uint32_t le32(const uint8_t *b)
{
	return (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
}

static void check_tmu(void)
{
	static const uint32_t addrs[] = { 0, 0x20, 0x10000000, 0xffffffe0, 0x12345660 };
	bool ok = true;
	for (unsigned a = 0; a < sizeof addrs / sizeof addrs[0]; a++) {
		for (uint32_t p = 0; p < 16; p++) {
			uint32_t w;
			uint8_t m[4];
			mesa71_p0(addrs[a], p, m);
			/* data's top byte is the unit; the address replaces it */
			ok &= v3d_tex_p0(addrs[a], 0x05000000u | p, &w) && w == le32(m);
		}
		for (uint32_t p = 0; p < 8; p++) {
			uint32_t w;
			uint8_t m[4];
			mesa71_p1(addrs[a], p, m);
			ok &= v3d_tex_p1(addrs[a], 0x03000000u | p, &w) && w == le32(m);
		}
	}
	check(ok, "TMU P0 and P1 (v3d_tex_p0/p1) equal V3D71_TMU_CONFIG_PARAMETER_0/1 for every payload");

	ok = true;
	int n = 0;
	static const int offs[] = { -8, -3, 0, 1, 7 };
	for (uint32_t op = 0; op < 16; op++)
		for (int si = 0; si < 5; si++)
			for (int ti = 0; ti < 5; ti++)
				for (int ri = 0; ri < 5; ri++) {
					uint32_t flags = (op * 131u + (uint32_t)(si * 7 + ti * 3 + ri)) & 0x7ff;
					struct v3d_tex71_p2 p = {
						.op = op, .offset_s = offs[si], .offset_t = offs[ti],
						.offset_r = offs[ri], .gather_mode = flags & 1,
						.gather_component = (flags >> 1) & 3,
						.coefficient_mode = (flags >> 3) & 1,
						.sample_number = (flags >> 4) & 3,
						.disable_autolod = (flags >> 6) & 1,
						.offset_format_8 = (flags >> 7) & 1,
						.lod_query = (flags >> 8) & 1, .dim_query = (flags >> 9) & 1,
						.write_conversion = (flags >> 10) & 1,
					};
					uint8_t m[4];
					uint32_t w;
					mesa71_p2(op, offs[si], offs[ti], offs[ri], flags, m);
					ok &= v3d_tex71_p2(&p, &w) && w == le32(m);
					n++;
				}
	/* each new 7.1 bit alone */
	for (int bit = 8; bit <= 10; bit++) {
		struct v3d_tex71_p2 p = { .lod_query = bit == 8, .dim_query = bit == 9,
					  .write_conversion = bit == 10 };
		uint8_t m[4];
		uint32_t w;
		mesa71_p2(0, 0, 0, 0, 1u << bit, m);
		ok &= v3d_tex71_p2(&p, &w) && w == le32(m) && w == 1u << (bit + 16);
		n++;
	}
	char what[96];
	snprintf(what, sizeof what, "TMU P2 (v3d_tex71_p2) equals V3D71_TMU_CONFIG_PARAMETER_2 (%d words)", n);
	check(ok, what);
}

/* ---- 5. refusals and controls ------------------------------------------------- */

static void check_refusals(void)
{
	const struct v3d_tex71_state good = {
		.base = 0x10000000, .width = 64, .height = 64, .depth = 1, .max_level = 3,
		.type = V3D_TEXTURE_DATA_FORMAT_RGBA8,
		.swizzle = { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA },
		.array_stride_64 = 4,
	};
	uint8_t out[24], keep[24];
	check(v3d_tex71_pack_state(out, &good), "the refusal base record packs");
	struct v3d_tex71_state b;
	bool all = true;
	int n = 0;
#define REFUSE(stmt)                                                                               \
	do {                                                                                       \
		b = good;                                                                          \
		stmt;                                                                              \
		memset(out, 0xa5, 24);                                                             \
		memcpy(keep, out, 24);                                                             \
		bool r = !v3d_tex71_pack_state(out, &b) && memcmp(out, keep, 24) == 0;             \
		if (!r)                                                                            \
			printf("  not refused: %s\n", #stmt);                                      \
		all &= r;                                                                          \
		n++;                                                                               \
	} while (0)
	REFUSE(b.base = 0x10000020);
	REFUSE(b.base = 0x10000001);
	REFUSE(b.transfer_func = 2);
	REFUSE(b.transfer_func = 7);
	REFUSE(b.width = 0);
	REFUSE(b.width = 16384);
	REFUSE(b.height = 16384);
	REFUSE(b.depth = 0);
	REFUSE(b.depth = 16384);
	REFUSE(b.type = 3);
	REFUSE(b.type = 128);
	REFUSE(b.max_level = 16);
	REFUSE(b.base_level = 4);
	REFUSE(b.level_0_ub_pad = 16);
	REFUSE(b.array_stride_64 = 1u << 24);
	REFUSE(b.swizzle[2] = 6);
#undef REFUSE
	char what[64];
	snprintf(what, sizeof what, "texture state refusals (%d), nothing written", n);
	check(all, what);

	const struct v3d_tex71_sampler sg = { .min_lod = 0, .max_lod = 1000 };
	struct v3d_tex71_sampler sb;
	all = v3d_tex71_pack_sampler(out, &sg);
	n = 0;
#define REFUSE(stmt)                                                                               \
	do {                                                                                       \
		sb = sg;                                                                           \
		stmt;                                                                              \
		memset(out, 0xa5, 24);                                                             \
		memcpy(keep, out, 24);                                                             \
		bool r = !v3d_tex71_pack_sampler(out, &sb) && memcmp(out, keep, 24) == 0;          \
		if (!r)                                                                            \
			printf("  not refused: %s\n", #stmt);                                      \
		all &= r;                                                                          \
		n++;                                                                               \
	} while (0)
	REFUSE(sb.wrap_s = 5);
	REFUSE(sb.wrap_t = 5);
	REFUSE(sb.wrap_r = 7);
	REFUSE(sb.border_color_mode = 3);
	REFUSE(sb.border_color_mode = 6);
	REFUSE(sb.border_color[1] = 1);
	REFUSE(sb.compare_func = 8);
	REFUSE(sb.min_lod = NAN);
	REFUSE(sb.max_lod = NAN);
	REFUSE(sb.lod_bias = NAN);
	REFUSE(sb.lod_bias = 128.0f);
	REFUSE(sb.lod_bias = -128.01f);
	REFUSE(sb.lod_bias = 1e30f);
#undef REFUSE
	snprintf(what, sizeof what, "sampler refusals (%d), nothing written", n);
	check(all, what);

	struct v3d_tex71_p2 pg = { 0 }, pb;
	uint32_t w = 0x5a5a5a5a;
	all = v3d_tex71_p2(&pg, &w) && w == 0;
	static const int bad[] = { 0, 1, 2, 3, 4, 5, 6, 7 };
	for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; i++) {
		pb = pg;
		switch (bad[i]) {
		case 0: pb.op = 16; break;
		case 1: pb.offset_s = 8; break;
		case 2: pb.offset_t = -9; break;
		case 3: pb.offset_r = 8; break;
		case 4: pb.gather_component = 4; break;
		case 5: pb.sample_number = 4; break;
		case 6: pb.offset_s = -9; break;
		case 7: pb.op = 0xffffffffu; break;
		}
		w = 0x5a5a5a5a;
		all &= !v3d_tex71_p2(&pb, &w) && w == 0x5a5a5a5a;
	}
	check(all, "TMU P2 refusals (8), nothing written");
	uint32_t p;
	check(!v3d_tex_p0(0x10000010, 0, &p) && !v3d_tex_p0(0x10000000, 0x10, &p) &&
		      !v3d_tex_p1(0x10000008, 0, &p) && !v3d_tex_p1(0x10000000, 0x8, &p),
	      "TMU P0/P1 refusals (misaligned record, a payload bit outside the fields)");
}

/* Each control changes one input on our side only: the compare must see it. */
static void check_controls(void)
{
	const struct tcase *c = &tex_cases[1]; /* rgba8 srgb 64x64 */
	struct v3d_tex71_state s;
	struct v3d_tex_layout l;
	struct mrsc r;
	uint8_t ours[24], mesa[24], alt[24];
	bool ok = ours_tex(c, &s, &l, ours);
	mrsc_from(&l, &r);
	mesa71_tex_state(c, &r, mesa);
	ok = ok && memcmp(ours, mesa, 24) == 0;
	int caught = 0, n = 0;
	struct v3d_tex71_state t;
#define CONTROL(stmt)                                                                              \
	do {                                                                                       \
		t = s;                                                                             \
		stmt;                                                                              \
		caught += v3d_tex71_pack_state(alt, &t) && memcmp(alt, mesa, 24) != 0;             \
		n++;                                                                               \
	} while (0)
	CONTROL(t.transfer_func = V3D_TEX71_TRANSFER_NONE);
	CONTROL(t.r_b_swap = true);
	CONTROL(t.reverse = true);
	CONTROL(t.array_stride_64++);
	CONTROL(t.swizzle[0] = V3D_SWIZZLE_BLUE);
	CONTROL(t.base += 64);
	CONTROL(t.extended = !t.extended);
#undef CONTROL
	check(ok && caught == n, "texture controls: sRGB off, R/B swap, reverse, stride, swizzle, base, extended each differ from Mesa");

	struct scase sc = { { W_REPEAT, W_CLAMP_TO_EDGE, W_MIRROR }, F_LINEAR, F_NEAREST, M_LINEAR,
			    0.5f, 4, 1, false, 0, 0, { 0, 0, 0, 0 }, BV_F16_UNORM };
	struct v3d_tex71_sampler ss, st;
	ours_sampler_from(&sc, &ss);
	mesa71_sampler(&sc, mesa);
	ok = v3d_tex71_pack_sampler(ours, &ss) && memcmp(ours, mesa, 24) == 0;
	caught = n = 0;
#define CONTROL(stmt)                                                                              \
	do {                                                                                       \
		st = ss;                                                                           \
		stmt;                                                                              \
		caught += v3d_tex71_pack_sampler(alt, &st) && memcmp(alt, mesa, 24) != 0;          \
		n++;                                                                               \
	} while (0)
	CONTROL(st.wrap_s = V3D_TEX71_WRAP_CLAMP);
	CONTROL(st.wrap_t = st.wrap_s);
	CONTROL(st.mag_nearest = false);
	CONTROL(st.mip_nearest = true);
	CONTROL(st.lod_bias += 1.0f / 256.0f);
	CONTROL(st.max_anisotropy = 2);
#undef CONTROL
	/* a FOLLOWS colour converted to f16, as 4.2 wants: not 7.1's words */
	struct scase fc = { { W_BORDER, W_BORDER, W_BORDER }, F_LINEAR, F_LINEAR, M_NONE, 0, 1000,
			    0, false, 0, 0, { 0.25f, 0.5f, 0.75f, 1 }, BV_F16_UNORM };
	ours_sampler_from(&fc, &st);
	mesa71_sampler(&fc, mesa);
	st.border_color[0] = 0x3400; /* 0.25 as f16 */
	st.border_color[1] = 0x3800;
	st.border_color[2] = 0x3a00;
	st.border_color[3] = 0x3c00;
	caught += v3d_tex71_pack_sampler(alt, &st) && memcmp(alt, mesa, 24) != 0;
	n++;
	check(ok && caught == n, "sampler controls: wraps, filters, bias, anisotropy, f16 border words each differ from Mesa");
}

int main(int argc, char **argv)
{
	check_tex_cases();
	check_xml_anchors();
	check_samplers();
	check_references(argc > 1 ? argv[1] : "/nonexistent");
	check_tmu();
	check_refusals();
	check_controls();
	/* N_TEX + 1 coverage + 3 anchors + 2 sampler summary + 2 TMU +
	 * 5 refusal checks + 2 controls */
	const int planned = N_TEX + 1 + 3 + 2 + 3 + 2 + 5 + 2;
	if (run != planned) {
		printf("FAIL: %d checks run, %d planned\n", run, planned);
		printf("tex71: %d of %d checks, %d failed\n", run - failed, planned, failed + 1);
		return 1;
	}
	printf("tex71: %d of %d checks, %d failed\n", run - failed, planned, failed);
	return failed != 0;
}
#endif
