/* SPDX-License-Identifier: MIT */
/* SPDX-FileCopyrightText: 2014-2017 Broadcom */
/*
 * V3D 7.1 texture records (v3d_tex71.h): the 7.1 translation unit. Its
 * packers are Mesa's generated 7.1 ones (v3d_cl.h at V3D_CL_VER 71); the
 * values follow Mesa 26.2.1's gallium/drivers/v3d/v3dx_state.c at
 * V3D_VERSION >= 71, with the file:line of each rule below.
 *
 *   Copyright (c) 2014-2017 Broadcom
 */
#if defined(V3D_CL_VER) && V3D_CL_VER != 71
#error "v3d_tex71.c is the V3D 7.1 translation unit"
#endif
#ifndef V3D_CL_VER
#define V3D_CL_VER 71
#endif

#include "v3d_tex71.h"

#include <string.h>

#include "v3d_cl.h"
#include "v3d_tex.h" /* struct v3d_tex_layout, the type and swizzle values */

/* ---- the texture shader state ----------------------------------------------- */

void v3d_tex71_state_from_layout(struct v3d_tex71_state *s, const struct v3d_tex_layout *l)
{
	const struct v3d_tex_slice *s0 = &l->slices[0];

	/* v3dx_state.c:976-987: a UIF level 0 is always read as UIF, with its
	 * padding and the extended bit */
	s->level_0_is_strictly_uif = s0->tiling == V3D_TEX_TILING_UIF_XOR ||
				     s0->tiling == V3D_TEX_TILING_UIF_NO_XOR;
	s->level_0_xor_enable = s0->tiling == V3D_TEX_TILING_UIF_XOR;
	s->level_0_ub_pad = 0;
	s->extended = false;
	if (s->level_0_is_strictly_uif) {
		s->level_0_ub_pad = s0->ub_pad;
		s->extended = true;
	}
	/* v3d_resource.c:761-764 (cube_map_stride, programmed whatever the
	 * layer count, v3dx_state.c:961); 64 divides it */
	s->array_stride_64 = ((s0->offset + s0->size + 63u) & ~63u) / 64;
}

void v3d_tex71_state_from_layout_3d(struct v3d_tex71_state *s, const struct v3d_tex_layout *l)
{
	v3d_tex71_state_from_layout(s, l);
	/* v3d_resource.c:765-766: the stride between level 0's slices; a
	 * slice is whole 64-byte utiles */
	s->array_stride_64 = l->slices[0].size / 64;
}

/* The types v3d_tex.h names (the formats the GL core stores): the TMU would
 * read any other value as a different format, so it is refused. */
static bool type_is_known(uint32_t type)
{
	static const uint8_t types[] = {
		V3D_TEXTURE_DATA_FORMAT_R8,
		V3D_TEXTURE_DATA_FORMAT_RG8,
		V3D_TEXTURE_DATA_FORMAT_RGBA8,
		V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8,
		V3D_TEXTURE_DATA_FORMAT_R32F,
		V3D_TEXTURE_DATA_FORMAT_RGBA8_SNORM,
		V3D_TEXTURE_DATA_FORMAT_RGB565,
		V3D_TEXTURE_DATA_FORMAT_RGB10_A2,
		V3D_TEXTURE_DATA_FORMAT_R16F,
		V3D_TEXTURE_DATA_FORMAT_RGBA16F,
		V3D_TEXTURE_DATA_FORMAT_R11F_G11F_B10F,
		V3D_TEXTURE_DATA_FORMAT_RGB9_E5,
		V3D_TEXTURE_DATA_FORMAT_RGBA32F,
		V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2,
		V3D_TEXTURE_DATA_FORMAT_RGB8_PUNCHTHROUGH_ALPHA1,
		V3D_TEXTURE_DATA_FORMAT_R11_EAC,
		V3D_TEXTURE_DATA_FORMAT_SIGNED_R11_EAC,
		V3D_TEXTURE_DATA_FORMAT_RG11_EAC,
		V3D_TEXTURE_DATA_FORMAT_SIGNED_RG11_EAC,
		V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC,
		V3D_TEXTURE_DATA_FORMAT_RGBA8I,
		V3D_TEXTURE_DATA_FORMAT_RGBA8UI,
		V3D_TEXTURE_DATA_FORMAT_RGBA16I,
		V3D_TEXTURE_DATA_FORMAT_RGBA16UI,
		V3D_TEXTURE_DATA_FORMAT_RGBA32I,
		V3D_TEXTURE_DATA_FORMAT_RGBA32UI,
		V3D_TEXTURE_DATA_FORMAT_RGB10_A2UI,
	};
	for (unsigned i = 0; i < sizeof types / sizeof types[0]; i++)
		if (types[i] == type)
			return true;
	return false;
}

bool v3d_tex71_pack_state(uint8_t dst[V3D_TEX71_STATE_BYTES], const struct v3d_tex71_state *s)
{
	const uint32_t max14 = (1u << 14) - 1;
	if (!dst || !s || !type_is_known(s->type) || (s->base & 63) != 0 ||
	    (s->transfer_func != V3D_TEX71_TRANSFER_NONE &&
	     s->transfer_func != V3D_TEX71_TRANSFER_SRGB) ||
	    s->width == 0 || s->width > max14 || s->height == 0 || s->height > max14 ||
	    s->depth == 0 || s->depth > max14 || s->max_level > 15 ||
	    s->base_level > s->max_level || s->level_0_ub_pad > 15 ||
	    s->array_stride_64 >= (1u << 24))
		return false;
	for (int i = 0; i < 4; i++)
		if (s->swizzle[i] > V3D_SWIZZLE_ALPHA)
			return false;

	struct V3D71_TEXTURE_SHADER_STATE t = { V3D71_TEXTURE_SHADER_STATE_header };
	/* The low bits of word 0 (flips, transfer function, reverse) are
	 * ORed in after the checked pack, as v3d_tex.c does with 4.2's sRGB
	 * bit: Mesa's unpacker reads the base pointer as bits 0..31 whole, so
	 * a record with any of them set would never unpack to the values it
	 * was packed from. The base's own bits 0..5 are zero (checked above),
	 * which is where Mesa's packer ORs them (cl[0] |= util_bitpack_uint(
	 * transfer_func, 2, 4) | util_bitpack_uint(reverse, 5, 5)). */
	t.texture_base_pointer = s->base;
	t.r_b_swap = s->r_b_swap;
	t.array_stride_64_byte_aligned = s->array_stride_64;
	t.image_width = s->width;
	t.image_height = s->height;
	t.image_depth = s->depth;
	t.texture_type = s->type;
	t.extended = s->extended;
	t.swizzle_r = s->swizzle[0];
	t.swizzle_g = s->swizzle[1];
	t.swizzle_b = s->swizzle[2];
	t.swizzle_a = s->swizzle[3];
	t.max_level = s->max_level;
	t.base_level = s->base_level;
	t.level_0_ub_pad = s->level_0_ub_pad;
	t.level_0_xor_enable = s->level_0_xor_enable;
	t.level_0_is_strictly_uif = s->level_0_is_strictly_uif;
	/* v3dx_state.c:963-969 */
	t.chroma_offset_x = 1;
	t.chroma_offset_y = 1;
	t.texture_base_pointer_cb = s->base >> 6;
	t.texture_base_pointer_cr = s->base >> 6;
	uint8_t out[V3D71_TEXTURE_SHADER_STATE_length];
	if (!v3d_pack_TEXTURE_SHADER_STATE(out, &t))
		return false;
	out[0] |= (uint8_t)(s->transfer_func << 2);
	if (s->reverse)
		out[0] |= 1u << 5;
	/* one store: `dst` may be uncached GPU memory (v3d_tex.c) */
	memcpy(dst, out, V3D_TEX71_STATE_BYTES);
	return true;
}

/* ---- the sampler state -------------------------------------------------------- */

/* llroundf: half away from zero, as util_bitpack_[us]fixed round */
static int64_t round_half_away(float v)
{
	return (int64_t)(v >= 0.0f ? v + 0.5f : v - 0.5f);
}

static float clampf(float v, float lo, float hi)
{
	return v < lo ? lo : (v > hi ? hi : v);
}

static bool border_mode_ok(uint32_t m)
{
	return m == V3D_TEX71_BORDER_0000 || m == V3D_TEX71_BORDER_0001 ||
	       m == V3D_TEX71_BORDER_1111 || m == V3D_TEX71_BORDER_FOLLOWS;
}

bool v3d_tex71_pack_sampler(uint8_t dst[V3D_TEX71_SAMPLER_BYTES], const struct v3d_tex71_sampler *s)
{
	if (!dst || !s || s->wrap_s > V3D_TEX71_WRAP_MIRROR_ONCE ||
	    s->wrap_t > V3D_TEX71_WRAP_MIRROR_ONCE || s->wrap_r > V3D_TEX71_WRAP_MIRROR_ONCE ||
	    !border_mode_ok(s->border_color_mode) || s->compare_func > 7 ||
	    s->min_lod != s->min_lod || s->max_lod != s->max_lod || s->lod_bias != s->lod_bias)
		return false;
	if (s->border_color_mode != V3D_TEX71_BORDER_FOLLOWS &&
	    (s->border_color[0] | s->border_color[1] | s->border_color[2] | s->border_color[3]))
		return false;
	/* Fixed Bias is s8.8 in 16 bits; the first test keeps the conversion
	 * to an integer defined */
	if (s->lod_bias < -129.0f || s->lod_bias > 129.0f)
		return false;
	int64_t bias = round_half_away(s->lod_bias * 256.0f);
	if (bias < -32768 || bias > 32767)
		return false;

	/* v3dx_state.c:608-611 (and 0 as the floor of the max, which Mesa's
	 * u4.8 field needs as well) */
	float min_lod = clampf(s->min_lod, 0.0f, 15.0f);
	float max_lod = s->max_lod > s->min_lod ? s->max_lod : s->min_lod;
	max_lod = clampf(max_lod, 0.0f, 15.0f);
	/* v3dx_state.c:613-624: without a mip filter only base level, but
	 * fractionally over it so that the HW picks min or mag */
	if (s->mip_none) {
		if (min_lod > 1.0f / 256.0f)
			min_lod = 1.0f / 256.0f;
		if (max_lod > 1.0f / 256.0f)
			max_lod = 1.0f / 256.0f;
	}

	struct V3D71_SAMPLER_STATE p = { V3D71_SAMPLER_STATE_header };
	p.wrap_i_border = false; /* v3dx_state.c:591 */
	p.wrap_s = s->wrap_s;
	p.wrap_t = s->wrap_t;
	p.wrap_r = s->wrap_r;
	p.mag_filter_nearest = s->mag_nearest;
	p.min_filter_nearest = s->min_nearest;
	p.mip_filter_nearest = s->mip_nearest;
	p.depth_compare_function = s->compare_func;
	/* the LODs u4.8 and the bias s8.8, given on the fields' grid */
	p.min_level_of_detail = (float)round_half_away(min_lod * 256.0f) / 256.0f;
	p.max_level_of_detail = (float)round_half_away(max_lod * 256.0f) / 256.0f;
	p.fixed_bias = (float)bias / 256.0f;
	/* v3dx_state.c:626-635 */
	if (s->max_anisotropy) {
		p.anisotropy_enable = true;
		if (s->max_anisotropy > 8)
			p.maximum_anisotropy = 3;
		else if (s->max_anisotropy > 4)
			p.maximum_anisotropy = 2;
		else if (s->max_anisotropy > 2)
			p.maximum_anisotropy = 1;
	}
	/* v3dx_state.c:637-765: the words as given, no f16 at 7.x (:749-760) */
	p.border_color_mode = s->border_color_mode;
	p.border_color_word_0 = s->border_color[0];
	p.border_color_word_1 = s->border_color[1];
	p.border_color_word_2 = s->border_color[2];
	p.border_color_word_3 = s->border_color[3];
	uint8_t out[V3D71_SAMPLER_STATE_length];
	if (!v3d_pack_SAMPLER_STATE(out, &p))
		return false;
	memcpy(dst, out, V3D_TEX71_SAMPLER_BYTES);
	return true;
}

/* ---- TMU configuration parameter 2 -------------------------------------------- */

bool v3d_tex71_p2(const struct v3d_tex71_p2 *p, uint32_t *out)
{
	if (!p || !out || p->op > 15 || p->gather_component > 3 || p->sample_number > 3 ||
	    p->offset_s < -8 || p->offset_s > 7 || p->offset_t < -8 || p->offset_t > 7 ||
	    p->offset_r < -8 || p->offset_r > 7)
		return false;
	struct V3D71_TMU_CONFIG_PARAMETER_2 v, back;
	memset(&v, 0, sizeof v);
	v.op = p->op;
	v.offset_s = p->offset_s;
	v.offset_t = p->offset_t;
	v.offset_r = p->offset_r;
	v.gather_mode = p->gather_mode;
	v.gather_component = p->gather_component;
	v.coefficient_mode = p->coefficient_mode;
	v.sample_number = p->sample_number;
	v.disable_autolod = p->disable_autolod;
	v.offset_format_8 = p->offset_format_8;
	v.lod_query = p->lod_query;
	v.dim_query = p->dim_query;
	v.write_conversion = p->write_conversion;
	/* the checked round trip of v3d_cl.h's packers (TMU parameter 2 is
	 * not in v3d_cl_packets.h's list) */
	uint8_t b[V3D71_TMU_CONFIG_PARAMETER_2_length];
	memcpy(&back, &v, sizeof back);
	V3D71_TMU_CONFIG_PARAMETER_2_pack(NULL, b, &v);
	V3D71_TMU_CONFIG_PARAMETER_2_unpack(b, &back);
	if (memcmp(&back, &v, sizeof v) != 0)
		return false;
	*out = (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
	return true;
}
