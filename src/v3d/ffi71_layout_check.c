/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * No code: pins the layouts and constants src/v3d/ffi71.rs mirrors from the
 * V3D 7.1 builder's headers (driver/src/v3d/v3d_job71.h, v3d_tex71.h,
 * v3d_tfu71.h, and the v3d_frame.h / v3d_cl.h / v3d_tex.h types they take).
 * ffi71.rs asserts the same numbers on the Rust side; if an assertion fails
 * here, a header moved and ffi71.rs must move with it. Compile with
 * -DV3D_CL_VER=71 (v3d_job71.h declares nothing otherwise); tests/v3d71_ffi.rs
 * does, with -fsyntax-only. LP64 targets only.
 */
#include <stddef.h>
#include "v3d_job71.h"
#include "v3d_tex.h"
#include "v3d_tex71.h"
#include "v3d_tfu71.h"

#if defined(__LP64__)
#define SZ(t, n) _Static_assert(sizeof(struct t) == (n), #t)
#define OFF(t, f, n) _Static_assert(offsetof(struct t, f) == (n), #t "." #f)

/* v3d_frame.h and v3d_cl.h types (ffi71.rs writes them again) */
SZ(v3d_uniform, 8);
OFF(v3d_uniform, data, 4);
SZ(v3d_viewport, 24);
SZ(v3d_wpos, 8);
OFF(v3d_wpos, y_offset, 4);
SZ(v3d_uniform_storage, 16);
SZ(v3d_vertex_attr, 16);
OFF(v3d_vertex_attr, divisor, 12);
SZ(v3d_tiling, 32);
SZ(v3d_fs_prog, 64);
SZ(v3d_vs_prog, 64);
SZ(v3d_cl, 24);
OFF(v3d_cl, gpu, 8);
OFF(v3d_cl, used, 16);
OFF(v3d_cl, overflow, 20);
OFF(v3d_cl, bad_field, 21);

/* v3d_job71.h */
SZ(v3d_job71_rt, 20);
OFF(v3d_job71_rt, format, 4);
OFF(v3d_job71_rt, memory_format, 8);
OFF(v3d_job71_rt, height_in_ub_or_stride, 12);
OFF(v3d_job71_rt, r_b_swap, 16);
OFF(v3d_job71_rt, force_alpha_1, 17);
OFF(v3d_job71_rt, pad, 18);
SZ(v3d_job71_zs, 16);
OFF(v3d_job71_zs, format, 4);
OFF(v3d_job71_zs, height_in_ub, 12);
SZ(v3d_job71_target, 212);
OFF(v3d_job71_target, tile_alloc, 8);
OFF(v3d_job71_target, rts, 12);
OFF(v3d_job71_target, rt, 16);
OFF(v3d_job71_target, depth_type, 176);
OFF(v3d_job71_target, zs, 180);
OFF(v3d_job71_target, s, 196);
SZ(v3d_job71_stencil_face, 28);
SZ(v3d_job71_state, 188);
OFF(v3d_job71_state, scissor, 24);
OFF(v3d_job71_state, scissor_maxy, 40);
OFF(v3d_job71_state, front_ccw, 44);
OFF(v3d_job71_state, offset_tri, 47);
OFF(v3d_job71_state, offset_units, 48);
OFF(v3d_job71_state, line_width, 60);
OFF(v3d_job71_state, depth_enabled, 64);
OFF(v3d_job71_state, depth_func, 68);
OFF(v3d_job71_state, stencil_enabled, 72);
OFF(v3d_job71_state, stencil_front, 76);
OFF(v3d_job71_state, stencil_back, 104);
OFF(v3d_job71_state, blend_enabled, 132);
OFF(v3d_job71_state, rgb_func, 136);
OFF(v3d_job71_state, colormask, 160);
OFF(v3d_job71_state, blend_color_f16, 164);
OFF(v3d_job71_state, wpos, 172);
OFF(v3d_job71_state, sample_mask, 180);
OFF(v3d_job71_state, oq_addr, 184);
SZ(v3d_job71_tmu, 28);
OFF(v3d_job71_tmu, width, 12);
OFF(v3d_job71_tmu, array_size, 24);
SZ(v3d_job71_draw, 152);
OFF(v3d_job71_draw, fs, 8);
OFF(v3d_job71_draw, vs_storage, 32);
OFF(v3d_job71_draw, fs_storage, 48);
OFF(v3d_job71_draw, attrs, 64);
OFF(v3d_job71_draw, num_attrs, 72);
OFF(v3d_job71_draw, prim, 76);
OFF(v3d_job71_draw, indexed, 88);
OFF(v3d_job71_draw, index_type, 92);
OFF(v3d_job71_draw, index_offset, 104);
OFF(v3d_job71_draw, tmu, 112);
OFF(v3d_job71_draw, num_tmu, 120);
OFF(v3d_job71_draw, vs_tmu, 128);
OFF(v3d_job71_draw, num_vs_tmu, 136);
OFF(v3d_job71_draw, vs_writes_psiz, 140);
OFF(v3d_job71_draw, instances, 144);
SZ(v3d_job71_clear, 148);
OFF(v3d_job71_clear, color, 4);
OFF(v3d_job71_clear, z, 132);
OFF(v3d_job71_clear, stencil, 136);
OFF(v3d_job71_clear, zs_load, 140);
OFF(v3d_job71_clear, zs_cleared, 144);
OFF(v3d_job71_clear, zs_load_aspects, 145);
OFF(v3d_job71_clear, pad, 147);
SZ(v3d_job71, 280);
OFF(v3d_job71, t, 24);
OFF(v3d_job71, tiling, 236);
OFF(v3d_job71, draws, 268);
OFF(v3d_job71, broken, 272);
/* the draw layout ffi71.rs reads whole: 3 stream addresses, the record's
 * address, the attribute records', then the unpacked 96-byte record */
SZ(v3d_job71_draw_layout, 116);
OFF(v3d_job71_draw_layout, record, 12);
OFF(v3d_job71_draw_layout, attr_records, 16);
OFF(v3d_job71_draw_layout, rec, 20);
_Static_assert(_Alignof(struct v3d_job71_draw_layout) == 4, "v3d_job71_draw_layout alignment");
_Static_assert(V3D_JOB71_MAX_RTS == 8 && V3D_JOB71_MAX_SIZE == 7680, "7.1 limits");
_Static_assert(V3D_QUNIFORM_TEXTURE_WIDTH == 44 && V3D_QUNIFORM_TEXTURE_HEIGHT == 45 &&
		       V3D_QUNIFORM_TEXTURE_DEPTH == 46 && V3D_QUNIFORM_TEXTURE_ARRAY_SIZE == 47,
	       "texture size uniforms");
_Static_assert(V3D_QUNIFORM_TMU_CONFIG_P0 == 40 && V3D_QUNIFORM_TMU_CONFIG_P1 == 41,
	       "TMU configuration uniforms");
_Static_assert(V3D_JOB71_ZS_DEPTH == 1 && V3D_JOB71_ZS_STENCIL == 2, "Z/S aspect bits");
_Static_assert(V3D_JOB71_OK == 0 && V3D_JOB71_FULL == 1 && V3D_JOB71_REFUSED == 2, "job results");

/* constants ffi71.rs carries as numbers */
_Static_assert(V3D_QUNIFORM_CONSTANT == 0 && V3D_QUNIFORM_UNIFORM == 1 &&
		       V3D_QUNIFORM_VIEWPORT_X_SCALE == 2 && V3D_QUNIFORM_VIEWPORT_Y_SCALE == 3 &&
		       V3D_QUNIFORM_VIEWPORT_Z_OFFSET == 4 && V3D_QUNIFORM_VIEWPORT_Z_SCALE == 5,
	       "quniform contents");
_Static_assert(V3D_QUNIFORM_FB_WPOS_Y_SCALE == 0x10000 && V3D_QUNIFORM_FB_WPOS_Y_OFFSET == 0x10001 &&
		       V3D_QUNIFORM_FB_PNTC_Y_SCALE == 0x10002 && V3D_QUNIFORM_FB_PNTC_Y_OFFSET == 0x10003,
	       "our own gl_FragCoord.y / gl_PointCoord.y contents");
_Static_assert(V3D_MAX_ATTRS == 16, "V3D_MAX_ATTRS");
_Static_assert(V3D_COMPARE_FUNC_NEVER == 0 && V3D_COMPARE_FUNC_LESS == 1 &&
		       V3D_COMPARE_FUNC_EQUAL == 2 && V3D_COMPARE_FUNC_LEQUAL == 3 &&
		       V3D_COMPARE_FUNC_GREATER == 4 && V3D_COMPARE_FUNC_NOTEQUAL == 5 &&
		       V3D_COMPARE_FUNC_GEQUAL == 6 && V3D_COMPARE_FUNC_ALWAYS == 7,
	       "compare functions");
_Static_assert(V3D_STENCIL_OP_ZERO == 0 && V3D_STENCIL_OP_KEEP == 1 &&
		       V3D_STENCIL_OP_REPLACE == 2 && V3D_STENCIL_OP_INCR == 3 &&
		       V3D_STENCIL_OP_DECR == 4 && V3D_STENCIL_OP_INVERT == 5 &&
		       V3D_STENCIL_OP_INCWRAP == 6 && V3D_STENCIL_OP_DECWRAP == 7,
	       "stencil ops");
_Static_assert(V3D_BLEND_FACTOR_ZERO == 0 && V3D_BLEND_FACTOR_ONE == 1 &&
		       V3D_BLEND_FACTOR_SRC_COLOR == 2 && V3D_BLEND_FACTOR_INV_SRC_COLOR == 3 &&
		       V3D_BLEND_FACTOR_DST_COLOR == 4 && V3D_BLEND_FACTOR_INV_DST_COLOR == 5 &&
		       V3D_BLEND_FACTOR_SRC_ALPHA == 6 && V3D_BLEND_FACTOR_INV_SRC_ALPHA == 7 &&
		       V3D_BLEND_FACTOR_DST_ALPHA == 8 && V3D_BLEND_FACTOR_INV_DST_ALPHA == 9 &&
		       V3D_BLEND_FACTOR_CONST_COLOR == 10 && V3D_BLEND_FACTOR_INV_CONST_COLOR == 11 &&
		       V3D_BLEND_FACTOR_CONST_ALPHA == 12 && V3D_BLEND_FACTOR_INV_CONST_ALPHA == 13 &&
		       V3D_BLEND_FACTOR_SRC_ALPHA_SATURATE == 14,
	       "blend factors");
_Static_assert(V3D_BLEND_MODE_ADD == 0 && V3D_BLEND_MODE_SUB == 1 && V3D_BLEND_MODE_RSUB == 2 &&
		       V3D_BLEND_MODE_MIN == 3 && V3D_BLEND_MODE_MAX == 4,
	       "blend modes");
_Static_assert(V3D_PRIM_POINTS == 0 && V3D_PRIM_LINES == 1 && V3D_PRIM_LINE_LOOP == 2 &&
		       V3D_PRIM_LINE_STRIP == 3 && V3D_PRIM_TRIANGLES == 4 &&
		       V3D_PRIM_TRIANGLE_STRIP == 5 && V3D_PRIM_TRIANGLE_FAN == 6,
	       "primitives");
_Static_assert(V3D_INDEX_TYPE_8 == 0 && V3D_INDEX_TYPE_16 == 1 && V3D_INDEX_TYPE_32 == 2,
	       "index types");
_Static_assert(V3D_MEMORY_FORMAT_RASTER == 0 && V3D_MEMORY_FORMAT_LINEARTILE == 1 &&
		       V3D_MEMORY_FORMAT_UB_LINEAR_1_UIF_BLOCK_WIDE == 2 &&
		       V3D_MEMORY_FORMAT_UB_LINEAR_2_UIF_BLOCKS_WIDE == 3 &&
		       V3D_MEMORY_FORMAT_UIF_NO_XOR == 4 && V3D_MEMORY_FORMAT_UIF_XOR == 5,
	       "memory formats");
_Static_assert(V3D_INTERNAL_TYPE_DEPTH_32F == 0 && V3D_INTERNAL_TYPE_DEPTH_24 == 1 &&
		       V3D_INTERNAL_TYPE_DEPTH_16 == 2,
	       "depth internal types");
_Static_assert(V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8 == 0 && V3D_OUTPUT_IMAGE_FORMAT_BGR565 == 7 &&
		       V3D_OUTPUT_IMAGE_FORMAT_RGBA8 == 27 && V3D_OUTPUT_IMAGE_FORMAT_D32F == 40 &&
		       V3D_OUTPUT_IMAGE_FORMAT_D16 == 42 && V3D_OUTPUT_IMAGE_FORMAT_D24S8 == 43 &&
		       V3D_OUTPUT_IMAGE_FORMAT_S8 == 44,
	       "output image formats");

/* v3d_tex.h's layout, the argument of v3d_tex71_state_from_layout */
_Static_assert(V3D_TEX_MAX_LEVELS == 15, "V3D_TEX_MAX_LEVELS");
SZ(v3d_tex_slice, 24);
SZ(v3d_tex_layout, 20 + 24 * V3D_TEX_MAX_LEVELS);
OFF(v3d_tex_layout, slices, 20);

/* v3d_tex71.h */
_Static_assert(V3D_TEX71_STATE_BYTES == 24 && V3D_TEX71_SAMPLER_BYTES == 24 &&
		       V3D_TEX71_RECORD_ALIGN == 32,
	       "7.1 texture record sizes");
_Static_assert(V3D_TEX71_TRANSFER_NONE == 0 && V3D_TEX71_TRANSFER_SRGB == 1, "transfer func");
_Static_assert(V3D_TEX71_WRAP_REPEAT == 0 && V3D_TEX71_WRAP_CLAMP == 1 && V3D_TEX71_WRAP_MIRROR == 2 &&
		       V3D_TEX71_WRAP_BORDER == 3 && V3D_TEX71_WRAP_MIRROR_ONCE == 4 &&
		       V3D_TEX71_BORDER_0000 == 0 && V3D_TEX71_BORDER_0001 == 1 &&
		       V3D_TEX71_BORDER_1111 == 2 && V3D_TEX71_BORDER_FOLLOWS == 7,
	       "wrap and border modes");
SZ(v3d_tex71_state, 52);
OFF(v3d_tex71_state, width, 4);
OFF(v3d_tex71_state, type, 24);
OFF(v3d_tex71_state, swizzle, 28);
OFF(v3d_tex71_state, transfer_func, 32);
OFF(v3d_tex71_state, r_b_swap, 36);
OFF(v3d_tex71_state, reverse, 37);
OFF(v3d_tex71_state, level_0_is_strictly_uif, 38);
OFF(v3d_tex71_state, level_0_ub_pad, 44);
OFF(v3d_tex71_state, array_stride_64, 48);
SZ(v3d_tex71_sampler, 56);
OFF(v3d_tex71_sampler, min_nearest, 12);
OFF(v3d_tex71_sampler, min_lod, 16);
OFF(v3d_tex71_sampler, compare_func, 28);
OFF(v3d_tex71_sampler, max_anisotropy, 32);
OFF(v3d_tex71_sampler, border_color_mode, 36);
OFF(v3d_tex71_sampler, border_color, 40);
SZ(v3d_tex71_p2, 40);
OFF(v3d_tex71_p2, offset_s, 4);
OFF(v3d_tex71_p2, gather_mode, 16);
OFF(v3d_tex71_p2, gather_component, 20);
OFF(v3d_tex71_p2, coefficient_mode, 24);
OFF(v3d_tex71_p2, sample_number, 28);
OFF(v3d_tex71_p2, disable_autolod, 32);
OFF(v3d_tex71_p2, lod_query, 34);

/* v3d_tfu71.h */
_Static_assert(sizeof(enum v3d_tfu71_tiling) == 4, "enum v3d_tfu71_tiling is an int");
_Static_assert(V3D_TFU71_TILING_RASTER == 0 && V3D_TFU71_TILING_LINEARTILE == 1 &&
		       V3D_TFU71_TILING_UBLINEAR_1_COLUMN == 2 && V3D_TFU71_TILING_UBLINEAR_2_COLUMN == 3 &&
		       V3D_TFU71_TILING_UIF_NO_XOR == 4 && V3D_TFU71_TILING_UIF_XOR == 5,
	       "TFU tilings");
_Static_assert(V3D_TFU71_MAX_DIM == 16384u && V3D_TFU71_ICFG_OTYPE_SHIFT == 16u &&
		       V3D_TFU71_ICFG_IFORMAT_SHIFT == 23u && V3D_TFU71_ICFG_FORMAT_RASTER == 0u &&
		       V3D_TFU71_ICFG_FORMAT_LINEARTILE == 3u && V3D_TFU71_IOC_DIMTW == 1u &&
		       V3D_TFU71_IOC_NUMMM_SHIFT == 4u && V3D_TFU71_IOC_FORMAT_SHIFT == 12u &&
		       V3D_TFU71_IOC_FORMAT_RASTER == 0u && V3D_TFU71_IOC_FORMAT_LINEARTILE == 3u &&
		       V3D_TFU71_IOC_STRIDE_SHIFT == 16u,
	       "TFU register fields");
_Static_assert(V3D_TFU71_TTYPE_R8 == 0u && V3D_TFU71_TTYPE_RGBA8 == 4u && V3D_TFU71_TTYPE_RGB565 == 6u,
	       "TFU texture types");
SZ(v3d_tfu71_job, 48);
OFF(v3d_tfu71_job, coef, 28);
OFF(v3d_tfu71_job, ioc, 44);
SZ(v3d_tfu71_surface, 12);
OFF(v3d_tfu71_surface, tiling, 4);
OFF(v3d_tfu71_surface, stride_or_padded_height, 8);
SZ(v3d_tfu71_args, 44);
OFF(v3d_tfu71_args, cpp, 4);
OFF(v3d_tfu71_args, width, 8);
OFF(v3d_tfu71_args, num_mm, 16);
OFF(v3d_tfu71_args, src, 20);
OFF(v3d_tfu71_args, dst, 32);
#endif
