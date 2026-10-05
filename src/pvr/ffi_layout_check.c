/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * ffi_layout_check.c - pins src/pvr/ffi.rs to the driver's headers.
 *
 * The Rust backend calls the driver's C command generators
 * (pvr_vdm_ppp_gen.h, pvr_pds_render_gen.h, pvr_pds_pixel_event_gen.h) through
 * hand-mirrored #[repr(C)] structs, u32 constants and extern "C" prototypes in
 * ffi.rs. This file is compiled by the C side of the build against the REAL
 * driver headers and contains nothing but compile-time checks: the size of
 * every mirrored struct, the offset of every mirrored field, the value of
 * every mirrored constant, and a redeclaration of every mirrored function
 * (a changed signature is a "conflicting types" error). ffi.rs carries the
 * same sizes and offsets as `const _: () = assert!(...)`, so a drift on either
 * side fails the build instead of corrupting the GPU command stream.
 *
 * The numbers were computed on an x86-64 host by a probe program compiled
 * against the driver headers (powervr-bxs-am62p 09c2072); every mirrored type
 * is built from fixed-width integers (and C int, 32-bit on all supported
 * targets), so they are identical on aarch64. Nothing here depends on the pointer size.
 *
 * When a driver header changes: update the struct in ffi.rs, then the numbers
 * here AND the matching asserts at the end of ffi.rs.
 */
#include <stddef.h>
#include <stdint.h>

#include "pvr_vdm_ppp_gen.h"
#include "pvr_pds_render_gen.h"
#include "pvr_pds_pixel_event_gen.h"
#include "pvr_backend.h"

/* ffi.rs mirrors C `int` as i32 and the pvr_ppp_passtype / pvr_pds_coeff_interp
 * enumerators as u32 values. */
_Static_assert(sizeof(int) == 4, "ffi.rs mirrors int as i32");

/* The backend options a C client (gl_test.c, gl_test_user) sets by name are
 * backend.rs's numbers: the header had lost PVR_BACKEND_OPT_DBIAS_INT and
 * its COUNT read 11 against Rust's 12, unnoticed (2026-09-23). */
_Static_assert(PVR_BACKEND_OPT_RETIRED_NO_SUBMIT == 6, "backend.rs PVR_BACKEND_OPT_RETIRED_NO_SUBMIT (the retired slot keeps its number)");
_Static_assert(PVR_BACKEND_OPT_FORCE_MIP0 == 10, "backend.rs PVR_BACKEND_OPT_FORCE_MIP0");
_Static_assert(PVR_BACKEND_OPT_DBIAS_INT == 11, "backend.rs PVR_BACKEND_OPT_DBIAS_INT");
_Static_assert(PVR_BACKEND_OPT_PBE_TWIDDLE == 12, "backend.rs PVR_BACKEND_OPT_PBE_TWIDDLE");
_Static_assert(PVR_BACKEND_OPT_ONE_IN_FLIGHT == 13, "backend.rs PVR_BACKEND_OPT_ONE_IN_FLIGHT");
_Static_assert(PVR_BACKEND_OPT_SWAP_DROPS_ZS == 14, "backend.rs PVR_BACKEND_OPT_SWAP_DROPS_ZS");
_Static_assert(PVR_BACKEND_OPT_FS_TEMPS_MIN == 15, "backend.rs PVR_BACKEND_OPT_FS_TEMPS_MIN");
_Static_assert(PVR_BACKEND_OPT_IF_LOWERING == 16, "backend.rs PVR_BACKEND_OPT_IF_LOWERING");
_Static_assert(PVR_IF_LOWERING_DEFAULT == 0 && PVR_IF_LOWERING_PREDICATE == 1 && PVR_IF_LOWERING_BRANCH == 2,
	       "backend.rs PVR_IF_LOWERING_*");
_Static_assert(PVR_BACKEND_OPT_COUNT == 17, "backend.rs PVR_BACKEND_OPT_COUNT");

_Static_assert(sizeof(struct pvr_ppp_frag_inputs) == 36, "struct pvr_ppp_frag_inputs");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, pixel_program_code_offset) == 0, "pvr_ppp_frag_inputs.pixel_program_code_offset");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, texture_program_code_offset) == 4, "pvr_ppp_frag_inputs.texture_program_code_offset");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, texture_program_data_offset) == 8, "pvr_ppp_frag_inputs.texture_program_data_offset");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, texture_program_data_size_bytes) == 12, "pvr_ppp_frag_inputs.texture_program_data_size_bytes");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, texture_program_temps) == 16, "pvr_ppp_frag_inputs.texture_program_temps");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, shared_reg_count) == 20, "pvr_ppp_frag_inputs.shared_reg_count");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, isp_userpass) == 24, "pvr_ppp_frag_inputs.isp_userpass");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, passtype) == 28, "pvr_ppp_frag_inputs.passtype");
_Static_assert(offsetof(struct pvr_ppp_frag_inputs, varying_program_code_offset) == 32, "pvr_ppp_frag_inputs.varying_program_code_offset");

_Static_assert(sizeof(struct pvr_ppp_state) == 208, "struct pvr_ppp_state");
_Static_assert(offsetof(struct pvr_ppp_state, objtype) == 0, "pvr_ppp_state.objtype");
_Static_assert(offsetof(struct pvr_ppp_state, point_line_width) == 4, "pvr_ppp_state.point_line_width");
_Static_assert(offsetof(struct pvr_ppp_state, tri_merge_disable) == 8, "pvr_ppp_state.tri_merge_disable");
_Static_assert(offsetof(struct pvr_ppp_state, depth_cmp) == 12, "pvr_ppp_state.depth_cmp");
_Static_assert(offsetof(struct pvr_ppp_state, depth_write) == 16, "pvr_ppp_state.depth_write");
_Static_assert(offsetof(struct pvr_ppp_state, color_write_disable) == 20, "pvr_ppp_state.color_write_disable");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_cmp) == 24, "pvr_ppp_state.stencil_cmp");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_ref) == 28, "pvr_ppp_state.stencil_ref");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_sop_fail) == 32, "pvr_ppp_state.stencil_sop_fail");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_sop_dfail) == 36, "pvr_ppp_state.stencil_sop_dfail");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_sop_pass) == 40, "pvr_ppp_state.stencil_sop_pass");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_cmp_mask) == 44, "pvr_ppp_state.stencil_cmp_mask");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_write_mask) == 48, "pvr_ppp_state.stencil_write_mask");
_Static_assert(offsetof(struct pvr_ppp_state, two_sided) == 52, "pvr_ppp_state.two_sided");
_Static_assert(offsetof(struct pvr_ppp_state, dbias_enable) == 56, "pvr_ppp_state.dbias_enable");
_Static_assert(offsetof(struct pvr_ppp_state, dbias_index) == 60, "pvr_ppp_state.dbias_index");
_Static_assert(offsetof(struct pvr_ppp_state, scissor_enable) == 64, "pvr_ppp_state.scissor_enable");
_Static_assert(offsetof(struct pvr_ppp_state, scissor_index) == 68, "pvr_ppp_state.scissor_index");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_cmp) == 72, "pvr_ppp_state.stencil_back_cmp");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_ref) == 76, "pvr_ppp_state.stencil_back_ref");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_sop_fail) == 80, "pvr_ppp_state.stencil_back_sop_fail");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_sop_dfail) == 84, "pvr_ppp_state.stencil_back_sop_dfail");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_sop_pass) == 88, "pvr_ppp_state.stencil_back_sop_pass");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_cmp_mask) == 92, "pvr_ppp_state.stencil_back_cmp_mask");
_Static_assert(offsetof(struct pvr_ppp_state, stencil_back_write_mask) == 96, "pvr_ppp_state.stencil_back_write_mask");
_Static_assert(offsetof(struct pvr_ppp_state, clip_enable) == 100, "pvr_ppp_state.clip_enable");
_Static_assert(offsetof(struct pvr_ppp_state, clip_left) == 104, "pvr_ppp_state.clip_left");
_Static_assert(offsetof(struct pvr_ppp_state, clip_right) == 108, "pvr_ppp_state.clip_right");
_Static_assert(offsetof(struct pvr_ppp_state, clip_top) == 112, "pvr_ppp_state.clip_top");
_Static_assert(offsetof(struct pvr_ppp_state, clip_bottom) == 116, "pvr_ppp_state.clip_bottom");
_Static_assert(offsetof(struct pvr_ppp_state, viewport_enable) == 120, "pvr_ppp_state.viewport_enable");
_Static_assert(offsetof(struct pvr_ppp_state, viewport) == 124, "pvr_ppp_state.viewport");
_Static_assert(offsetof(struct pvr_ppp_state, vtxsize) == 148, "pvr_ppp_state.vtxsize");
_Static_assert(offsetof(struct pvr_ppp_state, rhw_pres) == 152, "pvr_ppp_state.rhw_pres");
_Static_assert(offsetof(struct pvr_ppp_state, psprite_size_pres) == 156, "pvr_ppp_state.psprite_size_pres");
_Static_assert(offsetof(struct pvr_ppp_state, f32_linear) == 160, "pvr_ppp_state.f32_linear");
_Static_assert(offsetof(struct pvr_ppp_state, f32_flat) == 164, "pvr_ppp_state.f32_flat");
_Static_assert(offsetof(struct pvr_ppp_state, f32_npc) == 168, "pvr_ppp_state.f32_npc");
_Static_assert(offsetof(struct pvr_ppp_state, usc_varyingsize) == 172, "pvr_ppp_state.usc_varyingsize");
_Static_assert(offsetof(struct pvr_ppp_state, pds_varyingsize) == 176, "pvr_ppp_state.pds_varyingsize");
_Static_assert(offsetof(struct pvr_ppp_state, pretransform) == 180, "pvr_ppp_state.pretransform");
_Static_assert(offsetof(struct pvr_ppp_state, cullmode) == 184, "pvr_ppp_state.cullmode");
_Static_assert(offsetof(struct pvr_ppp_state, clip_mode) == 188, "pvr_ppp_state.clip_mode");
_Static_assert(offsetof(struct pvr_ppp_state, drawclippededges) == 192, "pvr_ppp_state.drawclippededges");
_Static_assert(offsetof(struct pvr_ppp_state, wclampen) == 196, "pvr_ppp_state.wclampen");
_Static_assert(offsetof(struct pvr_ppp_state, flatshade_vtx) == 200, "pvr_ppp_state.flatshade_vtx");
_Static_assert(offsetof(struct pvr_ppp_state, z_pres) == 204, "pvr_ppp_state.z_pres");

_Static_assert(sizeof(struct pvr_vdm_vs_state) == 28, "struct pvr_vdm_vs_state");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_pds_data_offset) == 0, "pvr_vdm_vs_state.vs_pds_data_offset");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_pds_code_offset) == 4, "pvr_vdm_vs_state.vs_pds_code_offset");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_pds_data_size_bytes) == 8, "pvr_vdm_vs_state.vs_pds_data_size_bytes");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_pds_temps) == 12, "pvr_vdm_vs_state.vs_pds_temps");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_output_size_bytes) == 16, "pvr_vdm_vs_state.vs_output_size_bytes");
_Static_assert(offsetof(struct pvr_vdm_vs_state, vs_input_size_bytes) == 20, "pvr_vdm_vs_state.vs_input_size_bytes");
_Static_assert(offsetof(struct pvr_vdm_vs_state, flatshade_control) == 24, "pvr_vdm_vs_state.flatshade_control");

_Static_assert(sizeof(struct pvr_vdm_pds_state) == 20, "struct pvr_vdm_pds_state");
_Static_assert(offsetof(struct pvr_vdm_pds_state, pds_data_offset) == 0, "pvr_vdm_pds_state.pds_data_offset");
_Static_assert(offsetof(struct pvr_vdm_pds_state, pds_code_offset) == 4, "pvr_vdm_pds_state.pds_code_offset");
_Static_assert(offsetof(struct pvr_vdm_pds_state, pds_data_size_bytes) == 8, "pvr_vdm_pds_state.pds_data_size_bytes");
_Static_assert(offsetof(struct pvr_vdm_pds_state, pds_temps) == 12, "pvr_vdm_pds_state.pds_temps");
_Static_assert(offsetof(struct pvr_vdm_pds_state, usc_common_size_bytes) == 16, "pvr_vdm_pds_state.usc_common_size_bytes");

_Static_assert(sizeof(struct pvr_vdm_field_refusal) == 24, "struct pvr_vdm_field_refusal");
_Static_assert(offsetof(struct pvr_vdm_field_refusal, field) == 0, "pvr_vdm_field_refusal.field");
_Static_assert(offsetof(struct pvr_vdm_field_refusal, value) == 8, "pvr_vdm_field_refusal.value");
_Static_assert(offsetof(struct pvr_vdm_field_refusal, max) == 16, "pvr_vdm_field_refusal.max");
_Static_assert(offsetof(struct pvr_vdm_field_refusal, unit_bytes) == 20, "pvr_vdm_field_refusal.unit_bytes");
_Static_assert(PVR_VDM_FIELD_REFUSED == -1, "PVR_VDM_FIELD_REFUSED");

_Static_assert(sizeof(struct pvr_pds_render_vertex_info) == 24, "struct pvr_pds_render_vertex_info");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, data_size_dw) == 0, "pvr_pds_render_vertex_info.data_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, code_size_dw) == 4, "pvr_pds_render_vertex_info.code_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, temps_used) == 8, "pvr_pds_render_vertex_info.temps_used");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, vdm_pds_data_size_units) == 12, "pvr_pds_render_vertex_info.vdm_pds_data_size_units");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, vdm_pds_temp_size_units) == 16, "pvr_pds_render_vertex_info.vdm_pds_temp_size_units");
_Static_assert(offsetof(struct pvr_pds_render_vertex_info, vdm_usc_unified_size_units) == 20, "pvr_pds_render_vertex_info.vdm_usc_unified_size_units");

_Static_assert(sizeof(struct pvr_pds_render_kick_usc_info) == 12, "struct pvr_pds_render_kick_usc_info");
_Static_assert(offsetof(struct pvr_pds_render_kick_usc_info, data_size_dw) == 0, "pvr_pds_render_kick_usc_info.data_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_kick_usc_info, code_size_dw) == 4, "pvr_pds_render_kick_usc_info.code_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_kick_usc_info, doutu_offset) == 8, "pvr_pds_render_kick_usc_info.doutu_offset");

_Static_assert(sizeof(struct pvr_pds_render_color_upload_info) == 12, "struct pvr_pds_render_color_upload_info");
_Static_assert(offsetof(struct pvr_pds_render_color_upload_info, data_size_dw) == 0, "pvr_pds_render_color_upload_info.data_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_color_upload_info, code_size_dw) == 4, "pvr_pds_render_color_upload_info.code_size_dw");
_Static_assert(offsetof(struct pvr_pds_render_color_upload_info, temps_used) == 8, "pvr_pds_render_color_upload_info.temps_used");

_Static_assert(sizeof(struct pvr_pds_render_stream) == 24, "struct pvr_pds_render_stream");
_Static_assert(offsetof(struct pvr_pds_render_stream, address) == 0, "pvr_pds_render_stream.address");
_Static_assert(offsetof(struct pvr_pds_render_stream, stride) == 8, "pvr_pds_render_stream.stride");
_Static_assert(offsetof(struct pvr_pds_render_stream, dma_size) == 12, "pvr_pds_render_stream.dma_size");
_Static_assert(offsetof(struct pvr_pds_render_stream, dst_reg) == 16, "pvr_pds_render_stream.dst_reg");
_Static_assert(offsetof(struct pvr_pds_render_stream, per_instance) == 20, "pvr_pds_render_stream.per_instance");

_Static_assert(sizeof(struct pvr_pds_coeff_varying) == 12, "struct pvr_pds_coeff_varying");
_Static_assert(offsetof(struct pvr_pds_coeff_varying, components) == 0, "pvr_pds_coeff_varying.components");
_Static_assert(offsetof(struct pvr_pds_coeff_varying, interp) == 4, "pvr_pds_coeff_varying.interp");
_Static_assert(offsetof(struct pvr_pds_coeff_varying, vtxout_off) == 8, "pvr_pds_coeff_varying.vtxout_off");

_Static_assert(sizeof(struct pvr_pds_coeff_info) == 20, "struct pvr_pds_coeff_info");
_Static_assert(offsetof(struct pvr_pds_coeff_info, data_size_dw) == 0, "pvr_pds_coeff_info.data_size_dw");
_Static_assert(offsetof(struct pvr_pds_coeff_info, code_size_dw) == 4, "pvr_pds_coeff_info.code_size_dw");
_Static_assert(offsetof(struct pvr_pds_coeff_info, coeff_dwords) == 8, "pvr_pds_coeff_info.coeff_dwords");
_Static_assert(offsetof(struct pvr_pds_coeff_info, pds_varyingsize) == 12, "pvr_pds_coeff_info.pds_varyingsize");
_Static_assert(offsetof(struct pvr_pds_coeff_info, usc_varyingsize) == 16, "pvr_pds_coeff_info.usc_varyingsize");

_Static_assert(sizeof(struct pvr_pds_dma_kick) == 16, "struct pvr_pds_dma_kick");
_Static_assert(offsetof(struct pvr_pds_dma_kick, address) == 0, "pvr_pds_dma_kick.address");
_Static_assert(offsetof(struct pvr_pds_dma_kick, dwords) == 8, "pvr_pds_dma_kick.dwords");
_Static_assert(offsetof(struct pvr_pds_dma_kick, dest_offset) == 12, "pvr_pds_dma_kick.dest_offset");
/* ---- constants ---- */
_Static_assert(PVR_PPP_BLOCK_MAX_DWORDS == 40u, "PVR_PPP_BLOCK_MAX_DWORDS");
_Static_assert(PVR_TA_OBJTYPE_TRIANGLE == 0u, "PVR_TA_OBJTYPE_TRIANGLE");
_Static_assert(PVR_TA_OBJTYPE_LINE == 1u, "PVR_TA_OBJTYPE_LINE");
_Static_assert(PVR_TA_OBJTYPE_SPRITE_01UV == 4u, "PVR_TA_OBJTYPE_SPRITE_01UV");
_Static_assert(PVR_TA_CMPMODE_NEVER == 0u, "PVR_TA_CMPMODE_NEVER");
_Static_assert(PVR_TA_CMPMODE_LESS == 1u, "PVR_TA_CMPMODE_LESS");
_Static_assert(PVR_TA_CMPMODE_EQUAL == 2u, "PVR_TA_CMPMODE_EQUAL");
_Static_assert(PVR_TA_CMPMODE_LESS_OR_EQUAL == 3u, "PVR_TA_CMPMODE_LESS_OR_EQUAL");
_Static_assert(PVR_TA_CMPMODE_GREATER == 4u, "PVR_TA_CMPMODE_GREATER");
_Static_assert(PVR_TA_CMPMODE_NOT_EQUAL == 5u, "PVR_TA_CMPMODE_NOT_EQUAL");
_Static_assert(PVR_TA_CMPMODE_GREATER_OR_EQUAL == 6u, "PVR_TA_CMPMODE_GREATER_OR_EQUAL");
_Static_assert(PVR_TA_CMPMODE_ALWAYS == 7u, "PVR_TA_CMPMODE_ALWAYS");
_Static_assert(PVR_TA_STENCILOP_KEEP == 0u, "PVR_TA_STENCILOP_KEEP");
_Static_assert(PVR_TA_STENCILOP_ZERO == 1u, "PVR_TA_STENCILOP_ZERO");
_Static_assert(PVR_TA_STENCILOP_REPLACE == 2u, "PVR_TA_STENCILOP_REPLACE");
_Static_assert(PVR_TA_STENCILOP_INCR_SAT == 3u, "PVR_TA_STENCILOP_INCR_SAT");
_Static_assert(PVR_TA_STENCILOP_DECR_SAT == 4u, "PVR_TA_STENCILOP_DECR_SAT");
_Static_assert(PVR_TA_STENCILOP_INVERT == 5u, "PVR_TA_STENCILOP_INVERT");
_Static_assert(PVR_TA_STENCILOP_INCR == 6u, "PVR_TA_STENCILOP_INCR");
_Static_assert(PVR_TA_STENCILOP_DECR == 7u, "PVR_TA_STENCILOP_DECR");
_Static_assert(PVR_TA_CULLMODE_NONE == 0u, "PVR_TA_CULLMODE_NONE");
_Static_assert(PVR_TA_CULLMODE_CW == 1u, "PVR_TA_CULLMODE_CW");
_Static_assert(PVR_TA_CULLMODE_CCW == 2u, "PVR_TA_CULLMODE_CCW");
_Static_assert(PVR_TA_CLIP_MODE_NONE == 0u, "PVR_TA_CLIP_MODE_NONE");
_Static_assert(PVR_TA_CLIP_MODE_FRONT_REAR == 1u, "PVR_TA_CLIP_MODE_FRONT_REAR");
_Static_assert(PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH == 2u, "PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH");
_Static_assert(PVR_TA_FLATSHADE_VERTEX_0 == 1u, "PVR_TA_FLATSHADE_VERTEX_0");
_Static_assert(PVR_VDM_TOPOLOGY_POINT_LIST == 0u, "PVR_VDM_TOPOLOGY_POINT_LIST");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_LIST == 1u, "PVR_VDM_TOPOLOGY_LINE_LIST");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_STRIP == 3u, "PVR_VDM_TOPOLOGY_LINE_STRIP");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_LOOP == 5u, "PVR_VDM_TOPOLOGY_LINE_LOOP");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_LIST == 6u, "PVR_VDM_TOPOLOGY_TRI_LIST");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_STRIP == 9u, "PVR_VDM_TOPOLOGY_TRI_STRIP");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_FAN == 11u, "PVR_VDM_TOPOLOGY_TRI_FAN");
_Static_assert(PVR_PPP_PASSTYPE_OPAQUE == 0u, "PVR_PPP_PASSTYPE_OPAQUE");
_Static_assert(PVR_PPP_PASSTYPE_TRANSLUCENT == 1u, "PVR_PPP_PASSTYPE_TRANSLUCENT");
_Static_assert(PVR_PPP_PASSTYPE_PUNCH_THROUGH == 2u, "PVR_PPP_PASSTYPE_PUNCH_THROUGH");
_Static_assert(PVR_PDS_VERTEX_MAX_STREAMS == 16u, "PVR_PDS_VERTEX_MAX_STREAMS");
_Static_assert(PVR_PDS_NO_ID_REG == 255u, "PVR_PDS_NO_ID_REG");
_Static_assert(PVR_PDS_COEFF_SMOOTH == 0u, "PVR_PDS_COEFF_SMOOTH");
_Static_assert(PVR_PDS_COEFF_NPC == 1u, "PVR_PDS_COEFF_NPC");
_Static_assert(PVR_PDS_COEFF_FLAT == 2u, "PVR_PDS_COEFF_FLAT");
_Static_assert(PVR_PDS_COEFF_MAX_VARYINGS == 16u, "PVR_PDS_COEFF_MAX_VARYINGS");
_Static_assert(PVR_PDS_DOUTU_MAX_TEMPS == 252u, "PVR_PDS_DOUTU_MAX_TEMPS");
_Static_assert(PVR_PDS_DMA_MAX_KICKS == 8u, "PVR_PDS_DMA_MAX_KICKS");

/* ---- function prototypes as ffi.rs declares them --------------------------
 * (u32 = uint32_t, u64 = uint64_t, i32 = int; a mismatch with the header's
 * declaration is a compile error) */
int32_t pvr_ppp_build_block(const struct pvr_ppp_frag_inputs *in, const struct pvr_ppp_state *st,
			    uint32_t *out, uint32_t *out_count, struct pvr_vdm_field_refusal *why);
int32_t pvr_ppp_build_terminate_block(uint32_t width, uint32_t height, uint32_t out[3],
				      struct pvr_vdm_field_refusal *why);
int32_t pvr_isp_build_scissor_word(uint32_t xmin, uint32_t xmax, uint32_t ymin, uint32_t ymax,
				   uint32_t out[2], struct pvr_vdm_field_refusal *why);
int32_t pvr_vdm_emit_ppp_state(uint32_t *dw, uint64_t ppp_dev_addr, uint32_t word_count,
			       struct pvr_vdm_field_refusal *why);
int32_t pvr_vdm_emit_pds_state(uint32_t *dw, const struct pvr_vdm_pds_state *ps,
			       struct pvr_vdm_field_refusal *why);
int32_t pvr_vdm_emit_vdm_state(uint32_t *dw, const struct pvr_vdm_vs_state *vs,
			       struct pvr_vdm_field_refusal *why);
int32_t pvr_vdm_emit_index_list(uint32_t *dw, uint32_t topology, uint64_t index_buf_dev_addr,
				uint32_t index_count, struct pvr_vdm_field_refusal *why);
int32_t pvr_vdm_emit_index_list_instanced(uint32_t *dw, uint32_t topology,
					  uint64_t index_buf_dev_addr, uint32_t index_count,
					  uint32_t instance_count, struct pvr_vdm_field_refusal *why);
uint32_t pvr_vdm_emit_terminate(uint32_t *dw);
int pvr_pds_render_kick_usc_program(uint64_t fs_usc_exec_offset, uint32_t fs_temps, uint32_t sample_rate,
				    uint32_t *data_out, uint32_t *code_out,
				    struct pvr_pds_render_kick_usc_info *info);
int pvr_pds_render_color_upload_program(uint64_t color_buf_addr, uint32_t dma_dword_count,
					uint32_t dest_offset, int has_slc_mcu_cache_controls,
					uint32_t *data_out, uint32_t *code_out,
					struct pvr_pds_render_color_upload_info *info);
int pvr_pds_render_vertex_program_ids(uint64_t vs_usc_exec_offset, uint32_t vs_temps,
				      const struct pvr_pds_render_stream *streams, uint32_t num_streams,
				      int has_slc_mcu_cache_controls, uint32_t vertex_id_reg,
				      uint32_t instance_id_reg, uint32_t *data_out, uint32_t *code_out,
				      struct pvr_pds_render_vertex_info *info);
int pvr_pds_render_coeff_program(int uses_w, int uses_z, int uses_pntc,
				 const struct pvr_pds_coeff_varying *varyings,
				 uint32_t num_varyings, uint32_t *data_out, uint32_t *code_out,
				 struct pvr_pds_coeff_info *info);
int pvr_pds_render_dma_program(const struct pvr_pds_dma_kick *kicks, uint32_t num_kicks,
			       int has_slc_mcu_cache_controls, uint32_t *data_out, uint32_t *code_out,
			       struct pvr_pds_render_color_upload_info *info);
int pvr_pds_generate_pixel_event_tq(uint64_t doutu_src0, const uint32_t *emit_words, uint32_t num_rt,
				    int has_slc_mcu_cache_controls, uint32_t *data_out, uint32_t *code_out,
				    uint32_t *data_size_dw, uint32_t *code_size_dw);
