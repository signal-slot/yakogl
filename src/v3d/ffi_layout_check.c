/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * No code: pins the layouts and constants src/v3d/ffi.rs mirrors from the V3D
 * job builder's headers (driver/src/v3d/v3d_job.h, v3d_frame.h, v3d_cl.h).
 * ffi.rs asserts the same numbers on the Rust side; if an assertion fails
 * here, a header moved and ffi.rs must move with it. Compiled by
 * tools/check.sh and by the image build. LP64 targets only.
 */
#include <stddef.h>
#include "v3d_job.h"
#include "v3d_tex.h"

#if defined(__LP64__)
#define SZ(t, n) _Static_assert(sizeof(struct t) == (n), #t)
#define OFF(t, f, n) _Static_assert(offsetof(struct t, f) == (n), #t "." #f)

SZ(v3d_uniform, 8);
OFF(v3d_uniform, data, 4);
SZ(v3d_viewport, 24);
SZ(v3d_uniform_storage, 16);
SZ(v3d_vertex_attr, 16);
OFF(v3d_vertex_attr, divisor, 12);
SZ(v3d_tiling, 32);
SZ(v3d_fs_prog, 56);
OFF(v3d_fs_prog, threads, 4);
OFF(v3d_fs_prog, single_seg, 8);
OFF(v3d_fs_prog, num_inputs, 12);
OFF(v3d_fs_prog, flat_shade_flags, 16);
OFF(v3d_fs_prog, centroid_flags, 24);
OFF(v3d_fs_prog, writes_z, 28);
OFF(v3d_fs_prog, uses_pid, 33);
OFF(v3d_fs_prog, uniforms, 40);
OFF(v3d_fs_prog, num_uniforms, 48);
SZ(v3d_vs_prog, 64);
OFF(v3d_vs_prog, single_seg, 8);
OFF(v3d_vs_prog, vpm_input_size, 12);
OFF(v3d_vs_prog, separate_segments, 20);
OFF(v3d_vs_prog, vcm_cache_size, 24);
OFF(v3d_vs_prog, uses_biid, 30);
OFF(v3d_vs_prog, vattr_sizes, 31);
OFF(v3d_vs_prog, uniforms, 48);
OFF(v3d_vs_prog, num_uniforms, 56);
SZ(v3d_cl, 24);
OFF(v3d_cl, gpu, 8);
OFF(v3d_cl, used, 16);
OFF(v3d_cl, overflow, 20);
OFF(v3d_cl, bad_field, 21);
SZ(v3d_job_target, 68);
OFF(v3d_job_target, rgb565, 20);
OFF(v3d_job_target, rts, 24);
OFF(v3d_job_target, rt, 28);
OFF(v3d_job_target, msaa, 52);
OFF(v3d_job_target, ms_address, 56);
OFF(v3d_job_target, ms_memory_format, 60);
OFF(v3d_job_target, ms_height_in_ub, 64);
SZ(v3d_job_stencil_face, 28);
SZ(v3d_job_state, 188);
OFF(v3d_job_state, scissor, 24);
OFF(v3d_job_state, scissor_maxy, 40);
OFF(v3d_job_state, front_ccw, 44);
OFF(v3d_job_state, offset_tri, 47);
OFF(v3d_job_state, offset_units, 48);
OFF(v3d_job_state, line_width, 60);
OFF(v3d_job_state, depth_enabled, 64);
OFF(v3d_job_state, depth_func, 68);
OFF(v3d_job_state, stencil_enabled, 72);
OFF(v3d_job_state, stencil_front, 76);
OFF(v3d_job_state, stencil_back, 104);
OFF(v3d_job_state, blend_enabled, 132);
OFF(v3d_job_state, rgb_func, 136);
OFF(v3d_job_state, colormask, 160);
OFF(v3d_job_state, blend_color_f16, 164);
OFF(v3d_job_state, wpos, 172);
OFF(v3d_job_state, sample_mask, 180);
OFF(v3d_job_state, oq_addr, 184);
SZ(v3d_wpos, 8);
OFF(v3d_wpos, y_offset, 4);
SZ(v3d_job_draw, 152);
OFF(v3d_job_draw, instances, 144);
/* the MMU-fault record's view (ffi.rs V3dJobDrawLayout): the three stream
 * addresses first, 116 bytes in all, 4-byte aligned */
SZ(v3d_job_draw_layout, 116);
OFF(v3d_job_draw_layout, fs_uniforms, 0);
OFF(v3d_job_draw_layout, vs_uniforms, 4);
OFF(v3d_job_draw_layout, cs_uniforms, 8);
_Static_assert(_Alignof(struct v3d_job_draw_layout) == 4, "v3d_job_draw_layout alignment");
OFF(v3d_job_draw, vs_tmu, 128);
OFF(v3d_job_draw, num_vs_tmu, 136);
OFF(v3d_job_draw, vs_writes_psiz, 140);
OFF(v3d_job_draw, tmu, 112);
OFF(v3d_job_draw, num_tmu, 120);
SZ(v3d_job_tmu, 12);
SZ(v3d_tex_slice, 24);
SZ(v3d_tex_layout, 20 + 24 * V3D_TEX_MAX_LEVELS);
OFF(v3d_tex_layout, slices, 20);
SZ(v3d_tex_tfu_job, 32);
OFF(v3d_tex_tfu_job, ios, 24);
SZ(v3d_tex_state, 44);
OFF(v3d_tex_state, swizzle, 28);
OFF(v3d_tex_state, srgb, 32);
OFF(v3d_tex_state, extended, 35);
OFF(v3d_tex_state, level_0_ub_pad, 36);
OFF(v3d_tex_state, array_stride_64, 40);
SZ(v3d_sampler, 32);
OFF(v3d_sampler, min_nearest, 12);
OFF(v3d_sampler, mip_none, 15);
OFF(v3d_sampler, min_lod, 16);
OFF(v3d_sampler, border_color_mode, 28);
_Static_assert(V3D_QUNIFORM_TMU_CONFIG_P0 == 40 && V3D_QUNIFORM_TMU_CONFIG_P1 == 41, "TMU uniforms");
_Static_assert(V3D_TEX_MAX_LEVELS == 15 && V3D_TEX_STATE_BYTES == 24 && V3D_SAMPLER_STATE_BYTES == 24 &&
		       V3D_TEX_RECORD_ALIGN == 32 && V3D_TEXTURE_DATA_FORMAT_R8 == 0 &&
		       V3D_TEXTURE_DATA_FORMAT_RG8 == 2 && V3D_TEXTURE_DATA_FORMAT_RGBA8 == 4 &&
		       V3D_TEXTURE_DATA_FORMAT_R32F == 29,
	       "texture sizes");
_Static_assert(V3D_SWIZZLE_ZERO == 0 && V3D_SWIZZLE_ONE == 1 && V3D_SWIZZLE_RED == 2 && V3D_SWIZZLE_GREEN == 3 && V3D_SWIZZLE_BLUE == 4 &&
		       V3D_SWIZZLE_ALPHA == 5 && V3D_WRAP_MODE_REPEAT == 0 && V3D_WRAP_MODE_CLAMP == 1 &&
		       V3D_WRAP_MODE_MIRROR == 2 && V3D_BORDER_COLOR_0000 == 0,
	       "texture enums");
OFF(v3d_job_draw, vs_storage, 32);
OFF(v3d_job_draw, fs_storage, 48);
OFF(v3d_job_draw, attrs, 64);
OFF(v3d_job_draw, num_attrs, 72);
OFF(v3d_job_draw, prim, 76);
OFF(v3d_job_draw, indexed, 88);
OFF(v3d_job_draw, index_type, 92);
OFF(v3d_job_draw, index_offset, 104);
SZ(v3d_job_clear, 68);
OFF(v3d_job_clear, s_s8, 64);
OFF(v3d_job_clear, load_rt, 46);
OFF(v3d_job_clear, rgba8_rt, 52);
_Static_assert(V3D_TEX_TILING_LINEARTILE == 1 && V3D_TEX_TILING_UIF_NO_XOR == 4 &&
		       V3D_TEX_TILING_UIF_XOR == 5 &&
		       (int)V3D_TEX_TILING_UIF_NO_XOR == (int)V3D_MEMORY_FORMAT_UIF_NO_XOR,
	       "the tiling enum is the packets' memory format numbering");
_Static_assert(V3D_STORE_BUFFER_Z == 9 && V3D_STORE_BUFFER_STENCIL == 10 && V3D_LOAD_BUFFER_Z == 9 &&
		       V3D_LOAD_BUFFER_STENCIL == 10 && V3D_STORE_BUFFER_ZSTENCIL == 11 &&
		       V3D_LOAD_BUFFER_ZSTENCIL == 11 &&
		       V3D_OUTPUT_IMAGE_FORMAT_D24S8 == 43,
	       "the Z/S buffer's packet values");
OFF(v3d_job_clear, zs_address, 16);
OFF(v3d_job_clear, zs_height_in_ub, 24);
OFF(v3d_job_clear, zs_load, 28);
OFF(v3d_job_clear, zs_store, 29);
OFF(v3d_job_clear, force_alpha_1, 30);
OFF(v3d_job_clear, r_b_swap, 31);
OFF(v3d_job_clear, s_address, 32);
OFF(v3d_job_clear, s_height_in_ub, 40);
OFF(v3d_job_clear, s_load, 44);
OFF(v3d_job_clear, s_store, 45);
OFF(v3d_job_clear, rgba8, 4);
OFF(v3d_job_clear, z, 8);
SZ(v3d_job, 832);
OFF(v3d_job, pack, 104);
SZ(v3d_job_pack, 728);
OFF(v3d_job_pack, valid, 4);
OFF(v3d_job_pack, key, 8);
OFF(v3d_job_pack, n, 712);
_Static_assert(V3D_PACK_GROUPS == 11 && V3D_PACK_KEY_MAX == 16, "ffi.rs carries these");
OFF(v3d_job, t, 24);
OFF(v3d_job, draws, 92);
OFF(v3d_job, broken, 100);

/* the constants ffi.rs carries as numbers */
_Static_assert(V3D_QUNIFORM_CONSTANT == 0 && V3D_QUNIFORM_UNIFORM == 1 &&
	       V3D_QUNIFORM_VIEWPORT_X_SCALE == 2 && V3D_QUNIFORM_VIEWPORT_Y_SCALE == 3 &&
	       V3D_QUNIFORM_VIEWPORT_Z_OFFSET == 4 && V3D_QUNIFORM_VIEWPORT_Z_SCALE == 5,
	       "quniform contents");
_Static_assert(V3D_QUNIFORM_FB_WPOS_Y_SCALE == 0x10000 && V3D_QUNIFORM_FB_WPOS_Y_OFFSET == 0x10001,
	       "our own gl_FragCoord.y contents");
_Static_assert(V3D_QUNIFORM_FB_PNTC_Y_SCALE == 0x10002 && V3D_QUNIFORM_FB_PNTC_Y_OFFSET == 0x10003,
	       "our own gl_PointCoord.y contents");
_Static_assert(V3D_MAX_ATTRS == 16, "V3D_MAX_ATTRS");
_Static_assert(V3D_COMPARE_FUNC_NEVER == 0 && V3D_COMPARE_FUNC_LESS == 1 &&
	       V3D_COMPARE_FUNC_EQUAL == 2 && V3D_COMPARE_FUNC_LEQUAL == 3 &&
	       V3D_COMPARE_FUNC_GREATER == 4 && V3D_COMPARE_FUNC_NOTEQUAL == 5 &&
	       V3D_COMPARE_FUNC_GEQUAL == 6 && V3D_COMPARE_FUNC_ALWAYS == 7,
	       "compare functions (the GLES_CMP_* order)");
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
	       "blend modes (the shc_blend_func order)");
_Static_assert(V3D_PRIM_POINTS == 0 && V3D_PRIM_LINES == 1 && V3D_PRIM_LINE_LOOP == 2 &&
	       V3D_PRIM_LINE_STRIP == 3 && V3D_PRIM_TRIANGLES == 4 &&
	       V3D_PRIM_TRIANGLE_STRIP == 5 && V3D_PRIM_TRIANGLE_FAN == 6,
	       "primitives");
_Static_assert(V3D_INDEX_TYPE_8 == 0 && V3D_INDEX_TYPE_16 == 1 && V3D_INDEX_TYPE_32 == 2,
	       "index types");
_Static_assert(V3D_JOB_OK == 0 && V3D_JOB_FULL == 1 && V3D_JOB_REFUSED == 2, "job results");
#endif
