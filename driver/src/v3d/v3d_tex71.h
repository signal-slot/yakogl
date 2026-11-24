/* SPDX-License-Identifier: MIT */
/* SPDX-FileCopyrightText: 2014-2017 Broadcom */
/*
 * V3D 7.1 (the Pi 5's V3D 7.1.10) texture records: the TEXTURE_SHADER_STATE
 * and SAMPLER_STATE the TMU reads through the P0/P1 uniforms, and the TMU
 * configuration parameter 2 where it differs from 4.2. The 4.2 records stay
 * in v3d_tex.c/.h (unchanged); this file only adds what 7.1 needs.
 *
 * After Mesa 26.2.1 (MIT), with the code each part follows:
 *   v3d_tex71_pack_state  v3d_setup_texture_shader_state and
 *                         v3dX(create_texture_shader_state_bo) at
 *                         V3D_VERSION >= 71
 *                         (gallium/drivers/v3d/v3dx_state.c:904-988,
 *                         :1023-1034), and for the R/B swap and reverse
 *                         bits, which gallium never sets, Vulkan's
 *                         pack_texture_shader_state_helper
 *                         (broadcom/vulkan/v3dvx_image.c:137-157); packed by
 *                         Mesa's generated V3D71_TEXTURE_SHADER_STATE packer
 *                         (driver/src/v3d/generated/v3d_packet_v71_pack.h,
 *                         through v3d_cl.h at V3D_CL_VER 71)
 *   v3d_tex71_pack_sampler v3d_upload_sampler_state_variant
 *                         (v3dx_state.c:586-767), SAMPLER_STATE
 *   v3d_tex71_p2          TMU_CONFIG_PARAMETER_2 for 7.1
 *                         (cle/v3d_packet.xml:1575-1590)
 *
 *   Copyright (c) 2014-2017 Broadcom
 *
 * What differs from 4.2 (cle/v3d_packet.xml:1592-1631 against :1633-1707):
 *  - the sRGB bit is gone; a 3-bit Transfer func takes its place (bits 2..4
 *    of word 0: none, sRGB, PQ, HLG, ...), with Reverse (bit 5) and the two
 *    flips (bits 0..1) in the same low bits of the base pointer;
 *  - R/B swap is bit 32 and the array stride moves to bits 33..56. 7.1.2
 *    has no swap bit and the stride at 32; Mesa (and this file) use the
 *    7.1.5 layout (v3d_packet.xml:1686-1690), right for the board's 7.1.10;
 *  - the record ends with the chroma fields (Cb, Cr base >> 6, offsets),
 *    which Mesa fills for every texture (v3dx_state.c:963-969);
 *  - in the sampler, the field layout is 4.2's, but a FOLLOWS border colour
 *    is four 32-bit words as given: the 7.x TMU converts them itself, where
 *    4.2 wants f16 values (v3dx_state.c:749-760);
 *  - TMU parameter 2 gains LOD query (4.2 has it too), DIM query and write
 *    conversion (bits 24..26). Parameters 0 and 1 are 4.2's, bit for bit
 *    (v3d_packet.xml:1534-1544 has one definition for both; Mesa's
 *    v3d_packing.c packs P1 with the 4.2 packer for both): v3d_tex_p0 and
 *    v3d_tex_p1 of v3d_tex.h serve 7.1 unchanged (tools/v3d/tex71_check.c
 *    compares them with V3D71_TMU_CONFIG_PARAMETER_0/1_pack).
 *
 * Reused from v3d_tex.h (version-neutral, not changed): struct
 * v3d_tex_layout and its layout functions, the V3D_TEXTURE_DATA_FORMAT_*
 * types (one Texture Data Formats enum for both, v3d_packet.xml:328) and the
 * V3D_SWIZZLE_* values (the same Swizzle encoding). Checked by
 * tools/v3d/check71.d/tex71.sh (tools/v3d/tex71_check.c) against Mesa's 7.1
 * packing.
 *
 * This header needs no V3D_CL_VER and includes no packet header: a 4.2 file
 * may include it. v3d_tex71.c is the 7.1 translation unit.
 *
 * No OS calls: the caller owns the memory.
 */
#ifndef V3D_TEX71_H
#define V3D_TEX71_H

#include <stdbool.h>
#include <stdint.h>

struct v3d_tex_layout;

/* TEXTURE_SHADER_STATE at 7.1 (V3D71_TEXTURE_SHADER_STATE_length) and
 * SAMPLER_STATE (V3D71_SAMPLER_STATE_length). Both records are 32-byte
 * aligned, as Mesa allocates them (v3dx_state.c:816, :1003-1005): P0 and P1
 * OR their flags into the address's low bits. The record a TMU reads needs
 * less: a sampler is 8-byte aligned (P1's flags are bits 0..2), and Mesa's own
 * 7.1 references place theirs so (tools/v3d/pi5/gl_*.clif: sampler states at
 * 0x...78, 0x...e8, 0x...68), the texture state 32-byte (P0 flags 0..3 plus the
 * record's 24 bytes). Keeping 32 for both is the safe choice. */
#define V3D_TEX71_STATE_BYTES 24
#define V3D_TEX71_SAMPLER_BYTES 24
#define V3D_TEX71_RECORD_ALIGN 32

/* Transfer func (v3d_packet.xml:1697-1704) */
#define V3D_TEX71_TRANSFER_NONE 0
#define V3D_TEX71_TRANSFER_SRGB 1

struct v3d_tex71_state {
	/* GPU address of level 0 of the first layer, as v3d_tex.h's struct
	 * v3d_tex_state: image + l.slices[0].offset (+ the layer's offset,
	 * v3d_tex_layer_offset). 64-byte aligned: the low 6 bits carry the
	 * transfer function, reverse and the flips. Also written, >> 6, as the
	 * Cb and Cr base, with both chroma offsets 1 (v3dx_state.c:963-969). */
	uint32_t base;
	uint32_t width, height, depth; /* depth: 1, 6 for a cube, the layer
					* count, or a 3D texture's depth */
	uint32_t base_level, max_level;
	uint32_t type;      /* V3D_TEXTURE_DATA_FORMAT_* (v3d_tex.h) */
	uint8_t swizzle[4]; /* R, G, B, A: V3D_SWIZZLE_* (v3d_tex.h) */
	/* V3D_TEX71_TRANSFER_NONE, or _SRGB for an sRGB format
	 * (v3dx_state.c:1027-1029) */
	uint32_t transfer_func;
	/* The 7.1 channel order bits (v3dvx_image.c:137-157): with either
	 * set, `swizzle` is the view's own swizzle (the format's R/B swap or
	 * reverse is these bits, not composed into the swizzle), and a border
	 * colour follows them. Gallium never sets them (it composes the
	 * format's swizzle, as at 4.2); R/B swap needs 7.1.5 or later. */
	bool r_b_swap, reverse;
	/* from the layout's level 0 (v3d_tex71_state_from_layout) */
	bool level_0_is_strictly_uif, level_0_xor_enable, extended;
	uint32_t level_0_ub_pad;
	uint32_t array_stride_64; /* Mesa's cube_map_stride / 64 */
};

/* The layout-derived fields as v3d_setup_texture_shader_state sets them at
 * 7.1 (v3dx_state.c:961, :976-987; the same rules as 4.2): for a 2D
 * texture, a cube map or an array, array_stride_64 = the mip tree's stride
 * / 64 (v3d_resource.c:761-764). The caller sets everything else. */
void v3d_tex71_state_from_layout(struct v3d_tex71_state *s, const struct v3d_tex_layout *l);
/* The same for a 3D texture laid out by v3d_tex_layout_3d: array_stride_64
 * = level 0's slice size / 64 (v3d_resource.c:765-766). */
void v3d_tex71_state_from_layout_3d(struct v3d_tex71_state *s, const struct v3d_tex_layout *l);

/* False, with `out` untouched, for a record the TMU would read as
 * something else: a type that is not one of v3d_tex.h's
 * V3D_TEXTURE_DATA_FORMAT_* values, a base that is not 64-byte aligned, a
 * transfer function other than NONE or sRGB, a size, level, swizzle, ub_pad
 * or array stride that does not fit its field, or base_level > max_level.
 * Nothing is masked to fit. */
bool v3d_tex71_pack_state(uint8_t out[V3D_TEX71_STATE_BYTES], const struct v3d_tex71_state *s);

/* SAMPLER_STATE. Wrap modes and border colour modes are the values of
 * Mesa's "Wrap Mode" and "Border Color Mode" enums (v3d_packet.xml:72-85):
 * REPEAT 0, CLAMP 1, MIRROR 2, BORDER 3, MIRROR_ONCE 4; 0000 0, 0001 1,
 * 1111 2, FOLLOWS 7. */
#define V3D_TEX71_WRAP_REPEAT 0
#define V3D_TEX71_WRAP_CLAMP 1
#define V3D_TEX71_WRAP_MIRROR 2
#define V3D_TEX71_WRAP_BORDER 3
#define V3D_TEX71_WRAP_MIRROR_ONCE 4
#define V3D_TEX71_BORDER_0000 0
#define V3D_TEX71_BORDER_0001 1
#define V3D_TEX71_BORDER_1111 2
#define V3D_TEX71_BORDER_FOLLOWS 7

struct v3d_tex71_sampler {
	uint32_t wrap_s, wrap_t, wrap_r; /* V3D_TEX71_WRAP_* */
	bool min_nearest, mag_nearest;
	bool mip_nearest; /* !(GL's LINEAR mip filter) */
	bool mip_none;    /* no mip filtering: the LOD is clamped to base */
	float min_lod, max_lod, lod_bias;
	uint32_t compare_func;   /* 0..7, GL order; NEVER (0) when compare is off */
	/* the sampler's max anisotropy as Gallium keeps it: 0 off; any other
	 * value turns it on, with 2, 4, 8 or 16 taps (v3dx_state.c:626-635) */
	uint32_t max_anisotropy;
	uint32_t border_color_mode; /* V3D_TEX71_BORDER_* */
	/* FOLLOWS only (0 otherwise): the four words as the TMU takes them,
	 * already in the channel order and clamp of Mesa's border variant
	 * (v3dx_state.c:646-747); 32-bit floats for a float or normalised
	 * format, no f16 conversion at 7.1. */
	uint32_t border_color[4];
};

/* False, with `out` untouched, for a wrap or border colour mode outside the
 * enums above, border words with a mode other than FOLLOWS, a compare
 * function above 7, a NaN LOD or bias, or a bias outside the s8.8 field
 * (-128 <= bias < 128). LODs clamp to 0..15 as Mesa's do. */
bool v3d_tex71_pack_sampler(uint8_t out[V3D_TEX71_SAMPLER_BYTES], const struct v3d_tex71_sampler *s);

/* TMU configuration parameter 2 at 7.1 (v3d_packet.xml:1575-1590): what the
 * compiler writes with a TMU lookup. 4.2's fields plus DIM query and write
 * conversion. Offsets are signed 4-bit texel offsets. */
struct v3d_tex71_p2 {
	uint32_t op;                       /* TMU Op, 0..15 */
	int32_t offset_s, offset_t, offset_r; /* -8..7 */
	bool gather_mode;
	uint32_t gather_component;         /* 0..3 */
	bool coefficient_mode;
	uint32_t sample_number;            /* 0..3 */
	bool disable_autolod, offset_format_8;
	bool lod_query, dim_query, write_conversion;
};

/* False, with *out untouched, for a value that does not fit its field. */
bool v3d_tex71_p2(const struct v3d_tex71_p2 *p, uint32_t *out);

#endif
