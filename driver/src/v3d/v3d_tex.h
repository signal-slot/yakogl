/* SPDX-License-Identifier: MIT */
/*
 * V3D 4.2 textures: the tiled level layout the TMU walks, the texel tiling,
 * and the texture and sampler state records the TMU reads through the P0/P1
 * uniforms (docs/online-shaderc/v3d-backend.md, "M6 textures").
 *
 * After Mesa 26.2.1 (MIT), with the functions each part follows:
 *   v3d_tex_layout      v3d_setup_slices, v3d_get_ub_pad,
 *                       v3d_get_dimension_mpad (gallium/drivers/v3d/v3d_resource.c),
 *                       transcribed: Gallium code, it needs pipe_resource
 *   v3d_tex_store_level, v3d_tex_store_box
 *                       call Mesa's own v3d_store_tiled_image
 *                       (broadcom/common/v3d_tiling.c, vendored unmodified in
 *                       third_party/, compiled by v3d_tiling_mesa.c)
 *   v3d_tex_format_get  the V3D 4.2 format table rows
 *                       (gallium/drivers/v3d/v3dx_format_table.c)
 *   v3d_tex_pack_state  v3d_setup_texture_shader_state's values, packed by
 *                       Mesa's generated TEXTURE_SHADER_STATE packer (v3d_cl.h)
 *                       (gallium/drivers/v3d/v3dx_state.c)
 *   v3d_tex_pack_sampler v3d_upload_sampler_state_variant, SAMPLER_STATE
 *   v3d_tex_p0, v3d_tex_p1 write_tmu_p0, write_tmu_p1
 *                       (gallium/drivers/v3d/v3d_uniforms.c)
 *
 *   Copyright (c) 2014-2017 Broadcom
 *   Copyright (C) 2012 Rob Clark <robclark@freedesktop.org>
 *
 * The cut is what GL ES 2.0 on this backend needs first: 2D, one layer, no
 * MSAA, uncompressed (block size 1), and a resource that is never shared
 * (Mesa's uif_top false). Checked against Mesa by tools/v3d/tex_check.c.
 *
 * No OS calls: the caller owns the memory.
 */
#ifndef V3D_TEX_H
#define V3D_TEX_H

#include <stdbool.h>
#include <stdint.h>

#include "v3d_cl.h" /* Mesa's packet enums (wrap and border colour modes) */

/* Mesa's enum v3d_tiling_mode order (v3d_tiling.h); raster is not a TMU
 * layout for a 2D texture, so it is not here. */
enum v3d_tex_tiling {
	V3D_TEX_TILING_LINEARTILE = 1,
	V3D_TEX_TILING_UBLINEAR_1_COLUMN = 2,
	V3D_TEX_TILING_UBLINEAR_2_COLUMN = 3,
	V3D_TEX_TILING_UIF_NO_XOR = 4,
	V3D_TEX_TILING_UIF_XOR = 5,
};

/* 4096x4096 has 13 levels; the state's max level is 4 bits. */
#define V3D_TEX_MAX_LEVELS 15

struct v3d_tex_slice {
	uint32_t offset;        /* from the start of the image */
	uint32_t stride;        /* bytes of one row of the padded level */
	uint32_t padded_height; /* rows of the padded level */
	uint32_t size;          /* stride * padded_height */
	uint32_t ub_pad;        /* UIF block rows of padding (UIF only) */
	enum v3d_tex_tiling tiling;
};

struct v3d_tex_layout {
	uint32_t width, height, levels, cpp;
	uint32_t size; /* bytes of the whole image, level 0 included */
	struct v3d_tex_slice slices[V3D_TEX_MAX_LEVELS];
};

/* The layout of a width x height image with `levels` levels (1 = no mip
 * chain) of `cpp` bytes a texel (1, 2, 4, 8 or 16). False for a size or a
 * count this cut does not lay out. */
bool v3d_tex_layout(uint32_t width, uint32_t height, uint32_t levels, uint32_t cpp,
		    struct v3d_tex_layout *l);

/* A 4x multisampled width x height resource as Mesa lays it out: one level
 * of 2 x width by 2 x height, UIF at the top (v3d_resource.c:612-642). The
 * TLB stores and loads it with decimate ALL_SAMPLES. False for a size this
 * cut does not lay out. */
bool v3d_tex_layout_msaa(uint32_t width, uint32_t height, uint32_t cpp, struct v3d_tex_layout *l);

/* Tile one level: `src` holds the level's width x height texels in raster
 * rows `src_stride` bytes apart; `image` is the start of the whole image
 * (the level is written at its slice offset). */
void v3d_tex_store_level(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			 const uint8_t *src, uint32_t src_stride);

/* Tile a w x h box of one level at (x, y), as glTexSubImage2D does (Mesa's
 * v3d_store_tiled_image with a pipe_box): `src` points at the box's first
 * texel, and its rows are `src_stride` bytes apart. Texels outside the box
 * are not touched. False, with nothing written, for a level the layout does
 * not have, a box that does not fit inside the level, or a src_stride shorter
 * than one row of the box. An empty box (w or h 0) inside the level writes
 * nothing and is not an error, as in GL. */
bool v3d_tex_store_box(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		       uint32_t x, uint32_t y, uint32_t w, uint32_t h, const uint8_t *src,
		       uint32_t src_stride);

/* The other direction (Mesa's v3d_load_tiled_image): the w x h box of one
 * level at (x, y) of the tiled `image` into raster rows `dst_stride` bytes
 * apart at `dst`. The same refusals as v3d_tex_store_box, nothing written. */
bool v3d_tex_load_box(const uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		      uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint8_t *dst,
		      uint32_t dst_stride);

/* Layered images: a cube map (6 layers, in GL's face order +X, -X, +Y, -Y,
 * +Z, -Z) or an array. Every layer is a whole mip tree laid out by
 * v3d_tex_layout, one after the other, `v3d_tex_layer_stride` bytes apart
 * (Mesa's cube_map_stride, v3d_resource.c:757-764: level 0's end, 64-byte
 * aligned; level 0 is the last and largest slice). The texture state takes
 * depth = the layer count (v3dx_state.c:936-938) and array_stride_64 =
 * stride / 64, which v3d_tex_state_from_layout already sets. To tile level L
 * of layer f, pass image + f * stride to v3d_tex_store_level/_box.
 * v3d_tex_layers_size and v3d_tex_layer_offset return false, and leave *out
 * untouched, for 0 layers, a level or layer out of range, or a size past
 * 32 bits. */
uint32_t v3d_tex_layer_stride(const struct v3d_tex_layout *l);
bool v3d_tex_layers_size(const struct v3d_tex_layout *l, uint32_t layers, uint32_t *out);
/* v3d_layer_offset (v3d_resource.c:771-779): the offset of level `level` of
 * layer `layer` from the start of the layered image. */
bool v3d_tex_layer_offset(const struct v3d_tex_layout *l, uint32_t level, uint32_t layer,
			  uint32_t layers, uint32_t *out);

/* TEXTURE_SHADER_STATE (V3D 4.2, 24 bytes). */
#define V3D_TEX_STATE_BYTES 24
/* Mesa aligns both records to 32 bytes: the P0/P1 uniforms OR their own bits
 * into the address's low bits. */
#define V3D_TEX_RECORD_ALIGN 32

/* Texture Data Formats (Mesa v3d_packet.xml, V3D 4.x): the ones a supported
 * format uses. */
#define V3D_TEXTURE_DATA_FORMAT_R8    0
#define V3D_TEXTURE_DATA_FORMAT_RG8   2
#define V3D_TEXTURE_DATA_FORMAT_RGBA8 4
#define V3D_TEXTURE_DATA_FORMAT_R32F  29

/* Swizzle values (the Swizzle fields of TEXTURE_SHADER_STATE). */
#define V3D_SWIZZLE_ZERO  0
#define V3D_SWIZZLE_ONE   1
#define V3D_SWIZZLE_RED   2
#define V3D_SWIZZLE_GREEN 3
#define V3D_SWIZZLE_BLUE  4
#define V3D_SWIZZLE_ALPHA 5

/* The source formats a GL ES 2.0 upload can name, with the GL format/type
 * pair and Mesa's pipe format of the same memory layout. RGBA8, BGRA8, L8, A8
 * and L8A8 are supported: v3d_tex_format_get refuses every other one, and
 * v3d_tex_format_name names it for the refusal. */
enum v3d_tex_format_id {
	V3D_TEX_FORMAT_RGBA8,   /* RGBA / UNSIGNED_BYTE: R8G8B8A8_UNORM */
	V3D_TEX_FORMAT_BGRA8,   /* BGRA_EXT / UNSIGNED_BYTE: B8G8R8A8_UNORM */
	V3D_TEX_FORMAT_RGB8,    /* RGB / UNSIGNED_BYTE: no 3-byte TMU type */
	V3D_TEX_FORMAT_L8,      /* LUMINANCE / UNSIGNED_BYTE: L8_UNORM */
	V3D_TEX_FORMAT_A8,      /* ALPHA / UNSIGNED_BYTE: A8_UNORM */
	V3D_TEX_FORMAT_L8A8,    /* LUMINANCE_ALPHA / UNSIGNED_BYTE: L8A8_UNORM */
	V3D_TEX_FORMAT_RGB565,  /* RGB / UNSIGNED_SHORT_5_6_5: B5G6R5_UNORM */
	V3D_TEX_FORMAT_RGBA4,   /* RGBA / UNSIGNED_SHORT_4_4_4_4: A4B4G4R4_UNORM */
	V3D_TEX_FORMAT_RGB5_A1, /* RGBA / UNSIGNED_SHORT_5_5_5_1: A1B5G5R5_UNORM */
	V3D_TEX_FORMAT_R32F,    /* a depth texture as the GL core keeps it: R32_FLOAT */
	V3D_TEX_FORMAT_COUNT
};

/* What the texture state and the compiler need for one source format: the
 * row of Mesa's format table (gallium/drivers/v3d/v3dx_format_table.c), with
 * the swizzle in TEXTURE_SHADER_STATE's encoding (v3d_translate_pipe_swizzle).
 * The TMU applies the swizzle, so a program always reads R, G, B, A. */
struct v3d_tex_format {
	uint32_t type;        /* V3D_TEXTURE_DATA_FORMAT_* */
	uint32_t cpp;         /* bytes of one texel in memory */
	uint8_t swizzle[4];   /* V3D_SWIZZLE_*, for R, G, B, A */
	uint32_t return_size; /* 16: the TMU returns f16 pairs; 32: 32-bit words */
};

/* False for a format this cut does not support, or an id out of range; `out`
 * is then untouched. */
bool v3d_tex_format_get(enum v3d_tex_format_id id, struct v3d_tex_format *out);
/* The format's name ("RGBA8", ...), or "unknown" for an id out of range. */
const char *v3d_tex_format_name(enum v3d_tex_format_id id);

struct v3d_tex_state {
	/* GPU address of LEVEL 0: the image's address + l.slices[0].offset,
	 * which is not 0 (level 0 comes last, 4 KiB aligned). Mesa's
	 * texture_base_pointer is bo + v3d_layer_offset(prsc, 0, layer)
	 * (v3dx_state.c:958-960, v3d_resource.c:771-779); the TMU finds the
	 * smaller levels below it. Must be 64-byte aligned. */
	uint32_t base;
	uint32_t width, height, depth;
	uint32_t base_level, max_level;
	uint32_t type;       /* V3D_TEXTURE_DATA_FORMAT_* */
	uint8_t swizzle[4];  /* R, G, B, A: V3D_SWIZZLE_* */
	bool srgb;
	/* from the layout's level 0 (v3d_tex_state_from_layout) */
	bool level_0_is_strictly_uif, level_0_xor_enable, extended;
	uint32_t level_0_ub_pad;
	/* Mesa's cube_map_stride / 64, programmed whatever the layer count */
	uint32_t array_stride_64;
};

/* The layout-derived fields, as v3d_setup_texture_shader_state sets them. */
void v3d_tex_state_from_layout(struct v3d_tex_state *s, const struct v3d_tex_layout *l);
/* False, with `out` untouched, for a record the TMU would read as something
 * else: a type no supported format has (v3d_tex_format_get), a base address
 * that is not 64-byte aligned (flag bits share its low bits), a size, level,
 * swizzle, ub_pad or array stride that does not fit its field, or
 * base_level > max_level. Nothing is masked to fit. */
bool v3d_tex_pack_state(uint8_t out[V3D_TEX_STATE_BYTES], const struct v3d_tex_state *s);

/* SAMPLER_STATE (24 bytes). */
#define V3D_SAMPLER_STATE_BYTES 24

/* V3D_WRAP_MODE_* and V3D_BORDER_COLOR_* are Mesa's "Wrap Mode" and
 * "Border Color Mode" enums (v3d_packet_v42_pack.h, through v3d_cl.h). */

struct v3d_sampler {
	uint32_t wrap_s, wrap_t, wrap_r; /* V3D_WRAP_MODE_* */
	bool min_nearest, mag_nearest;
	bool mip_nearest;  /* !(GL's LINEAR mip filter) */
	bool mip_none;     /* no mip filtering: the LOD is clamped to base */
	float min_lod, max_lod, lod_bias;
	uint32_t border_color_mode; /* V3D_BORDER_COLOR_* */
};

/* False, with `out` untouched, for a wrap other than REPEAT, CLAMP or MIRROR
 * (BORDER needs border colour words this packer does not write), a border
 * colour mode other than 0000, 0001 or 1111, a NaN LOD, or a bias outside
 * the s8.8 field (-128 <= bias < 128). */
bool v3d_tex_pack_sampler(uint8_t out[V3D_SAMPLER_STATE_BYTES], const struct v3d_sampler *s);

/* The TMU configuration uniforms, as Mesa's write_tmu_p0 / write_tmu_p1
 * write them (gallium/drivers/v3d/v3d_uniforms.c:151-156, :208-215): the
 * record's address ORed with the low 24 bits of the compiler's uniform data
 * (bits 24..31 are the sampler binding, which the address replaces). The
 * fields are V3D42 TMU_CONFIG_PARAMETER_0 (return words, bits 0..3) and
 * TMU_CONFIG_PARAMETER_1 (output type 32-bit, unnormalised coordinates,
 * per-pixel mask enable: bits 0..2). False, with `out` untouched, for an
 * address that is not V3D_TEX_RECORD_ALIGN aligned or a payload bit outside
 * those fields. */
bool v3d_tex_p0(uint32_t tex_state_addr, uint32_t data, uint32_t *out);
bool v3d_tex_p1(uint32_t sampler_addr, uint32_t data, uint32_t *out);

/* The texture formatting unit (TFU): a job that tiles level 0 of a one-level
 * layout from a raster image in GPU memory. The registers of one job, in
 * struct v3d_tfu_job's order (v3d.h, which writes them). Packed as Mesa's
 * v3dX(tfu) does for V3D 4.2 (gallium/drivers/v3d/v3dx_tfu.c at b739817b,
 * lines 120-176; field positions from broadcom/common/v3d_tfu.h, V3D33_*).
 * The texture data types a job may name (cle/v3d_packet.xml:333-365). */
struct v3d_tex_tfu_job {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef0;
};
#define V3D_TFU_TTYPE_R8 0u
#define V3D_TFU_TTYPE_RG8 2u
#define V3D_TFU_TTYPE_RGBA8 4u
#define V3D_TFU_TTYPE_R16F 16u
#define V3D_TFU_TTYPE_R32F 29u

/* Pack the job that tiles level 0 of `l` from a raster source at GPU
 * address `src_gpu`, rows `src_stride` bytes apart, into the image at GPU
 * address `dst_gpu` (level 0 at its slice offset), with data type `ttype`.
 * False, with `j` untouched, for a layout of more than one level, a stride
 * that is not whole texels or is shorter than a row, a size of 16384 or
 * more, a source address that is not 4-byte aligned (the board measured
 * +0, +4 and +64 from a page, gl_test case 140; nothing narrower), or a
 * destination whose low 6 bits would fall into IOA's format and DIMTW
 * fields. The TFU writes the whole padded slice at the destination. */
bool v3d_tex_tfu_pack(const struct v3d_tex_layout *l, uint32_t ttype, uint32_t src_gpu, uint32_t src_stride,
		      uint32_t dst_gpu, struct v3d_tex_tfu_job *j);

/* Pack the job that generates levels 1 .. levels - 1 of the tiled image at
 * GPU address `image_gpu`, laid out as `l`, from its own level 0, with data
 * type `ttype` (the texture's real type, as Mesa's v3d_generate_mipmap
 * passes the resource's format: RGBA8 for an RGBA8 texture, so the filter
 * averages channels, not bytes). Mesa's v3dX(tfu) with for_mipmap, src ==
 * dst, src_level == base_level == 0 and last_level == levels - 1
 * (gallium/drivers/v3d/v3dx_tfu.c at b739817b): the source is level 0 in its
 * own tiling (ICFG's format field: LINEARTILE 11 + the tiling's distance from
 * LINEARTILE; IIS the padded height in UIF blocks for a UIF source, 0 for
 * LT and UB-linear); NUMMM (ICFG bits 5..) is the levels after the first;
 * IOA is level 0's address with the same tiling in its format field and
 * DIMTW (bit 0) set, which keeps the TFU from writing level 0 over itself
 * and sends the levels it makes to where Mesa's v3d_setup_slices lays them
 * (the layout `l` is, so the TMU finds them); a UIF level 0 carries its
 * padding rows in OPAD (ICFG bits 22..) as v3dx_tfu.c:154-162 does for every
 * UIF destination. False,
 * with `j` untouched, for a layout of one level, a texel size `ttype` does
 * not name (RGBA8 is 4 bytes), a size of 16384 or more, or a level 0 whose
 * address would fall into IOA's low 6 bits. */
bool v3d_tex_tfu_pack_mipmap(const struct v3d_tex_layout *l, uint32_t ttype, uint32_t image_gpu,
			     struct v3d_tex_tfu_job *j);

#endif
