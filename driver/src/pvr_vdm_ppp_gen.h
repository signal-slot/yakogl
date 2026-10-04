/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2022 Imagination Technologies Ltd.
 * The block and stream sequences follow Mesa 26.2.1
 * src/imagination/vulkan/rogue/pvr_clear.c, pvr_blit.c and
 * src/imagination/vulkan/pvr_arch_cmd_buffer.c (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * pvr_vdm_ppp_gen.h -- portable builders for the PPP state block and the VDM
 * control stream on PowerVR BXS-4-64 (BVNC 36.53.104.796). This is the
 * GEOMETRY the TA tiles for a TA+3D render, ported from Mesa 26.2.1
 * src/imagination/vulkan/rogue/pvr_clear.c (state block + stream) and
 * pvr_arch_cmd_buffer.c (the per-draw state words).
 *
 * Two levels of API:
 *   - pvr_ppp_build_block() / pvr_vdm_emit_*(): parameter-driven, used by the
 *     YakoGL backend to emit one PPP block + VDM state + index list per draw
 *     and to compose many draws into one control stream.
 *   - pvr_ppp_build_clear_block() / pvr_vdm_build_clear_stream(): the original
 *     single-draw API whose output is selected by the PVR_* compile-time knobs
 *     (the silicon test rig in pvr_fw_boot.c). They are thin wrappers over the
 *     parameter-driven functions and produce the same dwords as before.
 *
 * This header needs only <stdint.h>; the .c packs every word with Mesa's
 * csbgen (driver/compat/pvr_mesa.h).
 */
#ifndef PVR_VDM_PPP_GEN_H
#define PVR_VDM_PPP_GEN_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Max dword counts (worst case). */
#define PVR_PPP_BLOCK_MAX_DWORDS   40u   /* 19 (clear) + 6 viewport + 2 back-face ISP + 3 terminate + spare */
#define PVR_VDM_STREAM_MAX_DWORDS  16u   /* single-draw clear stream */

/* Clear-rect vertex geometry (Mesa PVR_CLEAR_VERTEX_*). */
#define PVR_CLEAR_VERTEX_COUNT        4u
#define PVR_CLEAR_VERTEX_COORDINATES  3u   /* x, y, z(depth) -- screen space */

/* ------------------------------------------------------------------ *
 * csbgen enum values used by callers (rogue/ppp.xml, rogue/vdm.xml)
 * ------------------------------------------------------------------ */
#define PVR_TA_OBJTYPE_TRIANGLE              0u
#define PVR_TA_OBJTYPE_LINE                  1u
#define PVR_TA_OBJTYPE_SPRITE_01UV           4u   /* points (Mesa pvr_ta_objtype) */
#define PVR_TA_OBJTYPE_POINT_FILLED_TRIANGLE 6u

#define PVR_TA_CMPMODE_NEVER            0u
#define PVR_TA_CMPMODE_LESS             1u
#define PVR_TA_CMPMODE_EQUAL            2u
#define PVR_TA_CMPMODE_LESS_OR_EQUAL    3u
#define PVR_TA_CMPMODE_GREATER          4u
#define PVR_TA_CMPMODE_NOT_EQUAL        5u
#define PVR_TA_CMPMODE_GREATER_OR_EQUAL 6u
#define PVR_TA_CMPMODE_ALWAYS           7u

#define PVR_TA_STENCILOP_KEEP           0u
#define PVR_TA_STENCILOP_ZERO           1u
#define PVR_TA_STENCILOP_REPLACE        2u
#define PVR_TA_STENCILOP_INCR_SAT       3u
#define PVR_TA_STENCILOP_DECR_SAT       4u
#define PVR_TA_STENCILOP_INVERT         5u
#define PVR_TA_STENCILOP_INCR           6u
#define PVR_TA_STENCILOP_DECR           7u

#define PVR_TA_CULLMODE_NONE            0u
#define PVR_TA_CULLMODE_CW              1u
#define PVR_TA_CULLMODE_CCW             2u

#define PVR_TA_CLIP_MODE_NONE             0u   /* NO_FRONT_OR_REAR */
#define PVR_TA_CLIP_MODE_FRONT_REAR       1u
#define PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH 2u

#define PVR_TA_FLATSHADE_VERTEX_0       1u
#define PVR_TA_FLATSHADE_VERTEX_1       2u
#define PVR_TA_FLATSHADE_VERTEX_2       3u

/* VDMCTRL_INDEX_LIST0 primitive topology (vdm.xml:117-130). */
#define PVR_VDM_TOPOLOGY_POINT_LIST     0u
#define PVR_VDM_TOPOLOGY_LINE_LIST      1u
#define PVR_VDM_TOPOLOGY_LINE_STRIP     3u   /* vdm.xml PRIMITIVE_TOPOLOGY (2 = LINE_LIST_ADJ) */
#define PVR_VDM_TOPOLOGY_LINE_LOOP      5u
#define PVR_VDM_TOPOLOGY_TRI_FAN        11u
#define PVR_VDM_TOPOLOGY_TRI_LIST       6u
#define PVR_VDM_TOPOLOGY_TRI_STRIP      9u

/* ------------------------------------------------------------------ *
 * (1) PPP state block
 * ------------------------------------------------------------------ */

/*
 * Fragment-PDS binding inputs for the PPP TA_STATE_PDS_* block. These come
 * from the render (non-transfer) fragment PDS generators:
 *
 *  pixel_program_code_offset  : PDS-heap-relative byte offset of the
 *        fragment "kick-USC" PDS program (DATA start; the code follows it).
 *        -> TA_STATE_PDS_SHADERBASE. 16-byte aligned.
 *  texture_program_code_offset: PDS-heap-relative byte offset of the
 *        texture-state / uniform DMA PDS code -> TA_STATE_PDS_TEXUNICODEBASE.
 *  texture_program_data_offset: its DATA segment -> TA_STATE_PDS_TEXTUREDATABASE.
 *  texture_program_data_size_bytes : that data segment's size in bytes
 *        (a multiple of 4). SIZEINFO1.pds_texturestatesize gets it in units
 *        of 4 dwords: DRU(bytes / 4, 4), Mesa's rule.
 *  texture_program_temps           : PDS temps used by that program.
 *  shared_reg_count : number of shared-register dwords the fragment USC reads
 *        (DMAed into the common store). Feeds TA_STATE_PDS_SIZEINFO2.
 *  varying_program_code_offset: PDS-heap-relative byte offset of the
 *        coefficient-loading (DOUTI) program -> TA_STATE_PDS_VARYINGBASE
 *        (DATA start; code follows). 0 = the dummy program at PDS offset 0
 *        (only valid when no varying is iterated).
 */
/* TA_STATE_ISPA.passtype [24:26]: the Rogue PASSTYPE enum (csbgen
 * rogue/ppp.xml:89-97). PUNCH_THROUGH is required for a fragment shader that
 * kills pixels (discard/texkill) so the ISP resolves tag/visibility feedback;
 * emitting such a shader as TRANSLUCENT deadlocks the ISP (3D DM HWR). */
enum pvr_ppp_passtype {
   PVR_PPP_PASSTYPE_OPAQUE        = 0,
   PVR_PPP_PASSTYPE_TRANSLUCENT   = 1,
   PVR_PPP_PASSTYPE_PUNCH_THROUGH = 2,
};

struct pvr_ppp_frag_inputs {
   uint32_t pixel_program_code_offset;
   uint32_t texture_program_code_offset;
   uint32_t texture_program_data_offset;
   uint32_t texture_program_data_size_bytes;
   uint32_t texture_program_temps;
   uint32_t shared_reg_count;
   uint32_t isp_userpass;          /* TA_STATE_ISPCTL.upass; 0 for bring-up */
   uint8_t  passtype;              /* enum pvr_ppp_passtype (ISPA.passtype) */
   uint32_t varying_program_code_offset;
};

/*
 * Per-draw render state for the PPP block (everything the PVR_* compile-time
 * knobs used to select). Zero-initialise, then set what differs from the
 * defaults documented per field.
 */
struct pvr_ppp_state {
   uint32_t objtype;             /* PVR_TA_OBJTYPE_* (default TRIANGLE) */
   /* ISPA.pointlinewidth: line / point size in 4.4 fixed point minus 1
    * (0 = 1/16 px, 15 = 1 px, 255 = 16 px; Mesa pvr_setup_isp_faces_and_control). */
   uint32_t point_line_width;
   /* SIZEINFO2.pds_tri_merge_disable: required for lines and punch-through
    * objects (Mesa pvr_setup_pds_state_size_info). */
   int      tri_merge_disable;

   /* ISPA depth: compare + write. Zero is NOT a valid GL state; set
    * depth_cmp = PVR_TA_CMPMODE_ALWAYS + depth_write = 0 for "no depth". */
   uint32_t depth_cmp;           /* PVR_TA_CMPMODE_* */
   int      depth_write;         /* 1 = write depth */

   /* ISPCTL.tagwritedisable: 1 = the object updates depth/stencil only, no
    * colour (GL: glColorMask all false, stencil mask passes). */
   int      color_write_disable;

   /* ISPB stencil for the front face (both faces unless two_sided).
    * Default: ALWAYS / KEEP / masks 0. */
   uint32_t stencil_cmp;
   uint32_t stencil_ref;         /* ISPA.sref */
   uint32_t stencil_sop_fail, stencil_sop_dfail, stencil_sop_pass;
   uint32_t stencil_cmp_mask, stencil_write_mask;
   /* Two-sided stencil: two_sided != 0 sets ISPCTL.two_sided and emits a
    * second ISPA/ISPB pair (the back-face words) after the front pair. The
    * front words apply to triangles that are clockwise in the TA's winding
    * (the winding PVR_TA_CULLMODE_CW culls), the back words to
    * counter-clockwise ones (Mesa pvr_setup_isp_faces_and_control swaps the
    * pairs for VK_FRONT_FACE_COUNTER_CLOCKWISE). Only depth/stencil differ
    * between the pairs; the back ISPA copies the front depth state. */
   int      two_sided;
   /* Depth bias (glPolygonOffset): ISPCTL.dbenable + a STATE_ISPDBSC word
    * naming the entry of the frag kick's ISP_DBIAS_BASE table (3 floats:
    * constant factor, slope factor, clamp) this object uses. Triangles only. */
   int      dbias_enable;
   uint32_t dbias_index;
   /* Pixel scissor (GL scissor test): ISPCTL.scenable + the scindex half of
    * the STATE_ISPDBSC word, naming a 2-dword entry of the frag kick's
    * ISP_SCISSOR_BASE table (IPF_SCISSOR_WORD_0/1). Use with the region clip,
    * which only drops whole tiles. */
   int      scissor_enable;
   uint32_t scissor_index;
   uint32_t stencil_back_cmp;
   uint32_t stencil_back_ref;
   uint32_t stencil_back_sop_fail, stencil_back_sop_dfail, stencil_back_sop_pass;
   uint32_t stencil_back_cmp_mask, stencil_back_write_mask;

   /* TA_REGION_CLIP (tile granular, inclusive tile indices). clip_enable=0 =>
    * whole screen (OUTSIDE mode with max bounds, as the clear template). */
   int      clip_enable;
   uint32_t clip_left, clip_right, clip_top, clip_bottom;

   /* Hardware viewport transform (6 f32 bit patterns: x_center, x_scale,
    * y_center, y_scale, z_center, z_scale). viewport_enable=0 => not emitted
    * (positions are pre-transformed screen space, PPP_CTRL.pretransform=1). */
   int      viewport_enable;
   uint32_t viewport[6];

   /* TA_OUTPUT_SEL + TA_STATE_VARYING0 */
   uint32_t vtxsize;             /* VS output vector size in dwords */
   int      rhw_pres;
   int      psprite_size_pres;   /* OUTPUT_SEL: the VS emits a point size (sprites) */            /* 1/w plane present (FS uses w) */
   uint32_t f32_linear, f32_flat, f32_npc;   /* varying f32 component counts */

   /* TA_STATE_PDS_SIZEINFO1 varying sizes (units: usc 16 dw, pds 4 dw). */
   uint32_t usc_varyingsize, pds_varyingsize;

   /* TA_STATE_PPP_CTRL */
   int      pretransform;        /* 1 = screen-space input (clear template) */
   uint32_t cullmode;            /* PVR_TA_CULLMODE_* */
   uint32_t clip_mode;           /* PVR_TA_CLIP_MODE_* */
   int      drawclippededges;
   int      wclampen;
   uint32_t flatshade_vtx;       /* PVR_TA_FLATSHADE_* or 0 */

   /* TA_OUTPUT_SEL tsp_unclamped_z_pres: the FS iterates the Z plane
    * (gl_FragCoord.z). Last, so no earlier field moved when it was added. */
   int      z_pres;
};

struct pvr_vdm_field_refusal; /* below, with the VDM emitters */

/*
 * Emit the full TA_STATE_* PPP state block (every word present).
 *   out        : dword sink, must hold >= PVR_PPP_BLOCK_MAX_DWORDS.
 *   *out_count : number of dwords written -> VDMCTRL_PPP_STATE0.word_count.
 * Returns the dword count, or PVR_VDM_FIELD_REFUSED when a field does not
 * fit its bits: a size (OUTPUT_SEL.vtxsize, STATE_VARYING0.f32_*,
 * STATE_PDS_SIZEINFO1/2), an enum, a stencil value, an ISPDBSC index, a
 * region clip tile, or a PDS offset that is not 16-byte aligned. Nothing is
 * written, *out_count is 0 and `*why` (when not NULL) names the field.
 */
int32_t pvr_ppp_build_block(const struct pvr_ppp_frag_inputs *in,
                            const struct pvr_ppp_state *st,
                            uint32_t *out,
                            uint32_t *out_count,
                            struct pvr_vdm_field_refusal *why);

/* The knob-driven single-draw variant (silicon test rig); same output as
 * before the parameterisation, and the same refusal. */
int32_t pvr_ppp_build_clear_block(const struct pvr_ppp_frag_inputs *in,
                                  uint32_t *out,
                                  uint32_t *out_count,
                                  struct pvr_vdm_field_refusal *why);

/*
 * The 3-word PPP "terminate" block the DDK emits at the end of a draw stream
 * (referenced by a trailing PPP_STATE_UPDATE): header(pres_terminate) +
 * TERMINATE0(clip_right/top/bottom in 32-px blocks) + TERMINATE1(clip_left,
 * render_target). Sized for a w x h render target: 3, or
 * PVR_VDM_FIELD_REFUSED (nothing written) when the last block does not fit
 * clip_right/clip_bottom (9 bits: 1..16384 px).
 */
int32_t pvr_ppp_build_terminate_block(uint32_t width, uint32_t height, uint32_t out[3],
                                      struct pvr_vdm_field_refusal *why);

/* ------------------------------------------------------------------ *
 * (2) VDM control stream
 * ------------------------------------------------------------------ */

/* Vertex pipeline state for VDMCTRL_VDM_STATE0/2/3/4/5. */
struct pvr_vdm_vs_state {
   uint32_t vs_pds_data_offset;     /* PDS-heap byte offset of the vertex PDS DATA (16-aligned) */
   uint32_t vs_pds_code_offset;     /* ... and CODE */
   uint32_t vs_pds_data_size_bytes;
   uint32_t vs_pds_temps;
   uint32_t vs_output_size_bytes;   /* VS output vector size in bytes */
   /* VS input vertex size in bytes: every vtxin register the vertex PDS
    * program DMAs into the unified store (highest register + 1, times 4).
    * VDM_STATE5.vs_usc_unified_size is sized from this (Mesa
    * pvr_arch_cmd_buffer.c: "the size of the input vertex"), not from the
    * outputs: with 9 input dwords and 8 output dwords, sizing it from the
    * outputs made vertex instances past the first USC task lose their last
    * input (Qt Quick's batchable shaders, Sept 2026). 0 = same as the
    * output size (the bring-up quads, where the two coincide). */
   uint32_t vs_input_size_bytes;
   uint32_t flatshade_control;      /* VDM_STATE0.flatshade_control (0 = VERTEX_0) */
};

/* Each emitter writes at `dw` and returns the number of dwords written. */
/* A PDS program the VDM runs on a state update to fill the vertex shader's
 * common store (shared registers) -- vertex uniforms via DOUTD
 * (VDMCTRL_PDS_STATE0/1/2, Mesa pvr_emit_dirty_pds_state). */
struct pvr_vdm_pds_state {
   uint32_t pds_data_offset;        /* PDS-heap byte offset of the DATA (16-aligned) */
   uint32_t pds_code_offset;        /* ... and CODE */
   uint32_t pds_data_size_bytes;
   uint32_t pds_temps;              /* PDS temps the program uses (dwords) */
   uint32_t usc_common_size_bytes;  /* shared registers the DMA fills (bytes) */
};

/* Why an emitter refused: the first field whose value does not fit its
 * bits. `value` and `max` are in the field's own units (`unit_bytes` bytes
 * each), as the hardware reads them; unit_bytes 0 is a plain value (an
 * enum, a table index, a tile coordinate, a word count, or the low bits of
 * a misaligned address, whose max is 0). `field` is a static string such as
 * "VDM_STATE5.vs_pds_data_size". The widths are Mesa 26.2.1's
 * src/imagination/csbgen/rogue/vdm.xml and ppp.xml (the PPP block builder
 * uses the same record). */
struct pvr_vdm_field_refusal {
   const char *field;
   uint64_t value;
   uint32_t max;
   uint32_t unit_bytes;
};

/* The return of a size-checked emitter that refused: nothing was written at
 * `dw`, and `*why` (when not NULL) names the field. A field write never
 * masks a value that does not fit: the masked value is a different size,
 * which the hardware would use without any error. */
#define PVR_VDM_FIELD_REFUSED (-1)

/* 2, or PVR_VDM_FIELD_REFUSED (word_count past 8 bits, an address at or
 * past 2^40 or not 4-byte aligned) */
int32_t pvr_vdm_emit_ppp_state(uint32_t *dw, uint64_t ppp_dev_addr, uint32_t word_count,
                               struct pvr_vdm_field_refusal *why);
/* 3, or PVR_VDM_FIELD_REFUSED */
int32_t pvr_vdm_emit_pds_state(uint32_t *dw, const struct pvr_vdm_pds_state *ps,
                               struct pvr_vdm_field_refusal *why);
/* 5, or PVR_VDM_FIELD_REFUSED */
int32_t pvr_vdm_emit_vdm_state(uint32_t *dw, const struct pvr_vdm_vs_state *vs,
                               struct pvr_vdm_field_refusal *why);
/* index_buf_dev_addr != 0: indexed u16 draw (3 dwords); 0: non-indexed (2).
 * PVR_VDM_FIELD_REFUSED (nothing written) for a topology past 4 bits or an
 * index address at or past 2^40 or not 4-byte aligned. */
int32_t pvr_vdm_emit_index_list(uint32_t *dw, uint32_t topology,
                                uint64_t index_buf_dev_addr, uint32_t index_count,
                                struct pvr_vdm_field_refusal *why);
/* As above for instance_count instances (> 1 adds INDEX_LIST3 = count - 1). */
int32_t pvr_vdm_emit_index_list_instanced(uint32_t *dw, uint32_t topology,
                                          uint64_t index_buf_dev_addr, uint32_t index_count,
                                          uint32_t instance_count,
                                          struct pvr_vdm_field_refusal *why);
uint32_t pvr_vdm_emit_terminate(uint32_t *dw);                                            /* 1 */

/*
 * Legacy single-draw inputs (knob-driven test rig).
 */
struct pvr_vdm_inputs {
   uint64_t ppp_state_dev_addr;
   uint32_t ppp_word_count;
   uint32_t vs_pds_data_offset;
   uint32_t vs_pds_code_offset;
   uint32_t vs_pds_data_size_bytes;
   uint32_t vs_pds_temps;
   uint32_t vs_output_size_bytes;
   uint32_t index_count;
   uint64_t index_buf_dev_addr;  /* GPU VA of a u16 index buffer; 0 => non-indexed */
   uint32_t index_count2;
   uint64_t index_buf2_dev_addr;
};

/*
 * Emit the single-draw stream:
 *   PPP_STATE0/1 -> VDM_STATE0/2/3/4/5 -> INDEX_LIST [-> INDEX_LIST #2]
 *   -> trailing PPP_STATE(3 words at ppp+0x4c) -> STREAM_TERMINATE.
 * Returns the byte size of the stream (out_count * 4), or 0 with
 * *out_count = 0 when a size field does not fit (`*why` names it).
 */
uint32_t pvr_vdm_build_clear_stream(const struct pvr_vdm_inputs *in,
                                    uint32_t *out,
                                    uint32_t *out_count,
                                    struct pvr_vdm_field_refusal *why);

/* ------------------------------------------------------------------ *
 * (3) Clear-rect vertex buffer
 * ------------------------------------------------------------------ */
uint32_t pvr_clear_vertices_fill(int32_t x0, int32_t y0,
                                 uint32_t w, uint32_t h,
                                 float depth,
                                 float out[PVR_CLEAR_VERTEX_COUNT]
                                          [PVR_CLEAR_VERTEX_COORDINATES]);

/* Compute VDM cam_size / vs_max_instances for BXS-4-64. */
void pvr_vdm_calc_cam_size_bxs4_64(uint32_t vs_output_size_bytes,
                                   int raster_enable,
                                   uint32_t *cam_size_out,
                                   uint32_t *vs_max_instances_out);

/* ------------------------------------------------------------------ *
 * (4) IPF scissor table
 * ------------------------------------------------------------------ */

/*
 * One 2-dword entry of the frag kick's ISP_SCISSOR_BASE table (named by
 * TA_STATE_ISPDBSC.scindex, see pvr_ppp_state::scissor_index above): a
 * memory table the IPF reads directly, not a PPP/VDM stream word, but with
 * the same 16-bit hardware bound on each coordinate as every other checked
 * field here (Mesa 26.2.1 src/imagination/csbgen/rogue/ipf.xml:150-166,
 * SCISSOR_WORD_0 scw0_xmin[31:16]/scw0_xmax[15:0], SCISSOR_WORD_1
 * scw1_ymin[31:16]/scw1_ymax[15:0]). Returns 2, or PVR_VDM_FIELD_REFUSED
 * (nothing written) when a coordinate does not fit 16 bits (`*why` names
 * it, same as every other emitter here).
 */
int32_t pvr_isp_build_scissor_word(uint32_t xmin, uint32_t xmax,
                                   uint32_t ymin, uint32_t ymax,
                                   uint32_t out[2],
                                   struct pvr_vdm_field_refusal *why);

#ifdef __cplusplus
}
#endif

#endif /* PVR_VDM_PPP_GEN_H */
