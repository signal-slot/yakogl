/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2022 Imagination Technologies Ltd.
 * The block and stream sequences follow Mesa 26.2.1
 * src/imagination/vulkan/rogue/pvr_clear.c, pvr_blit.c and
 * pvr_arch_cmd_buffer.c (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * pvr_vdm_ppp_gen.c -- the PPP state block and the VDM control stream on
 * PowerVR BXS-4-64 (BVNC 36.53.104.796): the GEOMETRY the TA tiles for a
 * TA + 3D render.
 *
 * Every word is packed by Mesa 26.2.1's csbgen (the ROGUE_*_pack() functions
 * of driver/src/pvr/generated/csbgen/rogue/{ppp,vdm,ipf}.h, generated from the
 * vendored XML). What is ours is which words, in which order, with which
 * values, following Mesa:
 *   rogue/pvr_clear.c   pvr_pack_clear_vdm_state(), pvr_emit_ppp_from_template()
 *                       and the static clear PPP templates (the block order,
 *                       the TA_WCLAMP value)
 *   pvr_blit.c          the six TA_STATE_PDS_* words of a clear
 *   pvr_arch_cmd_buffer.c pvr_emit_ppp_state() (word order with the viewport),
 *                       pvr_setup_isp_faces_and_control(),
 *                       pvr_arch_calculate_vertex_cam_size(),
 *                       pvr_emit_vdm_index_list()
 * and the checks a caller may get wrong: every field is checked against its
 * width in the XML before anything is written, and a value that does not fit
 * refuses the block (csbgen asserts in a debug build and masks in a release
 * one; a masked size is a smaller size the hardware uses without a word).
 *
 * The API (pvr_vdm_ppp_gen.h) is unchanged from the hand-packed encoder this
 * file replaced; tools/hostharness proves the words equal.
 */

#include <stdint.h>
#include <string.h>

#include "pvr_mesa.h"
#include "pvr_vdm_ppp_gen.h"

/* The header's values are the csbgen enums' (the C and Rust callers use the
 * header's names). */
_Static_assert(PVR_TA_OBJTYPE_TRIANGLE == ROGUE_TA_OBJTYPE_TRIANGLE, "objtype");
_Static_assert(PVR_TA_OBJTYPE_LINE == ROGUE_TA_OBJTYPE_LINE, "objtype");
_Static_assert(PVR_TA_OBJTYPE_SPRITE_01UV == ROGUE_TA_OBJTYPE_SPRITE_01UV, "objtype");
_Static_assert(PVR_TA_OBJTYPE_POINT_FILLED_TRIANGLE == ROGUE_TA_OBJTYPE_POINT_FILLED_TRIANGLE, "objtype");
_Static_assert(PVR_TA_CMPMODE_NEVER == ROGUE_TA_CMPMODE_NEVER, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_LESS == ROGUE_TA_CMPMODE_LESS, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_EQUAL == ROGUE_TA_CMPMODE_EQUAL, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_LESS_OR_EQUAL == ROGUE_TA_CMPMODE_LESS_OR_EQUAL, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_GREATER == ROGUE_TA_CMPMODE_GREATER, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_NOT_EQUAL == ROGUE_TA_CMPMODE_NOT_EQUAL, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_GREATER_OR_EQUAL == ROGUE_TA_CMPMODE_GREATER_OR_EQUAL, "cmpmode");
_Static_assert(PVR_TA_CMPMODE_ALWAYS == ROGUE_TA_CMPMODE_ALWAYS, "cmpmode");
_Static_assert(PVR_TA_STENCILOP_KEEP == ROGUE_TA_ISPB_STENCILOP_KEEP, "stencilop");
_Static_assert(PVR_TA_STENCILOP_ZERO == ROGUE_TA_ISPB_STENCILOP_ZERO, "stencilop");
_Static_assert(PVR_TA_STENCILOP_REPLACE == ROGUE_TA_ISPB_STENCILOP_REPLACE, "stencilop");
_Static_assert(PVR_TA_STENCILOP_INCR_SAT == ROGUE_TA_ISPB_STENCILOP_INCREMENT_SATURATE, "stencilop");
_Static_assert(PVR_TA_STENCILOP_DECR_SAT == ROGUE_TA_ISPB_STENCILOP_DECREMENT_SATURATE, "stencilop");
_Static_assert(PVR_TA_STENCILOP_INVERT == ROGUE_TA_ISPB_STENCILOP_INVERT, "stencilop");
_Static_assert(PVR_TA_STENCILOP_INCR == ROGUE_TA_ISPB_STENCILOP_INCREMENT, "stencilop");
_Static_assert(PVR_TA_STENCILOP_DECR == ROGUE_TA_ISPB_STENCILOP_DECREMENT, "stencilop");
_Static_assert(PVR_TA_CULLMODE_NONE == ROGUE_TA_CULLMODE_NO_CULLING, "cullmode");
_Static_assert(PVR_TA_CULLMODE_CW == ROGUE_TA_CULLMODE_CULL_CW, "cullmode");
_Static_assert(PVR_TA_CULLMODE_CCW == ROGUE_TA_CULLMODE_CULL_CCW, "cullmode");
_Static_assert(PVR_TA_CLIP_MODE_NONE == ROGUE_TA_CLIP_MODE_NO_FRONT_OR_REAR, "clip_mode");
_Static_assert(PVR_TA_CLIP_MODE_FRONT_REAR == ROGUE_TA_CLIP_MODE_FRONT_REAR, "clip_mode");
_Static_assert(PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH == ROGUE_TA_CLIP_MODE_FRONT_REAR_DEPTH, "clip_mode");
_Static_assert(PVR_TA_FLATSHADE_VERTEX_0 == ROGUE_TA_FLATSHADE_VTX_VERTEX_0, "flatshade");
_Static_assert(PVR_TA_FLATSHADE_VERTEX_1 == ROGUE_TA_FLATSHADE_VTX_VERTEX_1, "flatshade");
_Static_assert(PVR_TA_FLATSHADE_VERTEX_2 == ROGUE_TA_FLATSHADE_VTX_VERTEX_2, "flatshade");
_Static_assert(PVR_VDM_TOPOLOGY_POINT_LIST == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_POINT_LIST, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_LIST == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LIST, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_STRIP == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_STRIP, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_LINE_LOOP == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_LINE_LOOP, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_FAN == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_FAN, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_LIST == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_LIST, "topology");
_Static_assert(PVR_VDM_TOPOLOGY_TRI_STRIP == ROGUE_VDMCTRL_PRIMITIVE_TOPOLOGY_TRI_STRIP, "topology");
_Static_assert((int)PVR_PPP_PASSTYPE_OPAQUE == (int)ROGUE_TA_PASSTYPE_OPAQUE, "passtype");
_Static_assert((int)PVR_PPP_PASSTYPE_TRANSLUCENT == (int)ROGUE_TA_PASSTYPE_TRANSLUCENT, "passtype");
_Static_assert((int)PVR_PPP_PASSTYPE_PUNCH_THROUGH == (int)ROGUE_TA_PASSTYPE_PUNCH_THROUGH, "passtype");

/* `bytes` in units of `unit` bytes, rounded up, in 64 bits: a 32-bit
 * DIV_ROUND_UP wraps for a byte count near 2^32 and would pass a huge size
 * as a small one. */
static inline uint64_t pvr_units(uint64_t bytes, uint32_t unit)
{
   return (bytes + unit - 1u) / unit;
}

/* Fill *why (if given, and not yet filled): the first field that does not
 * fit is the one reported. */
static void pvr_refuse(struct pvr_vdm_field_refusal *why, const char *field,
                       uint64_t value, uint32_t max, uint32_t unit_bytes)
{
   if (why && !why->field) {
      why->field = field;
      why->value = value;
      why->max = max;
      why->unit_bytes = unit_bytes;
   }
}

/* Whether `value` (in units of `unit_bytes`; 0 = a plain value such as an
 * enum, a table index or a tile coordinate) fits a field of `bits` bits
 * (at most 32), the width the field has in the XML. When it does not, *why
 * names the field. */
static int pvr_field_fits(struct pvr_vdm_field_refusal *why, const char *field,
                          uint64_t value, unsigned bits, uint32_t unit_bytes)
{
   const uint32_t max = (uint32_t)((UINT64_C(1) << bits) - 1u);

   if (value <= max)
      return 1;
   pvr_refuse(why, field, value, max, unit_bytes);
   return 0;
}

/* Whether the bits of `addr` below `shift` are zero. An address field with
 * `shift` holds addr >> shift, and csbgen's __pvr_address (Mesa 26.2.1
 * pvr_packet_helpers.h) drops the low bits without a word: a misaligned
 * address becomes an earlier one. `field` names the bits (value = the low
 * bits, max 0). */
static int pvr_addr_aligned(struct pvr_vdm_field_refusal *why, const char *field,
                            uint64_t addr, unsigned shift)
{
   const uint64_t low = addr & ((UINT64_C(1) << shift) - 1u);

   if (!low)
      return 1;
   pvr_refuse(why, field, low, 0, 0);
   return 0;
}

/* ------------------------------------------------------------------------- *
 * VDM cam size: Mesa's pvr_arch_calculate_vertex_cam_size()
 * (pvr_arch_cmd_buffer.c), with the BXS-4-64's values from Mesa's device
 * information (vdm_cam_size, simple_internal_parameter_format[_v2]).
 * ------------------------------------------------------------------------- */
void pvr_vdm_calc_cam_size_bxs4_64(uint32_t vs_output_size_bytes,
                                   int raster_enable,
                                   uint32_t *cam_size_out,
                                   uint32_t *vs_max_instances_out)
{
   const struct pvr_device_info *dev_info = pvr_mesa_dev_info(false);
   /* vs_output_size is in VDM_STATE4 units. */
   const uint32_t vs_output_size =
      DIV_ROUND_UP(vs_output_size_bytes, ROGUE_VDMCTRL_VDM_STATE4_VS_OUTPUT_SIZE_UNIT_SIZE);
   const bool has_raster_enable = raster_enable != 0;
   const uint32_t vdm_cam_size = PVR_GET_FEATURE_VALUE(dev_info, vdm_cam_size, 32U);
   /* Size of a vertex in the UVS, *4 for column ordering. */
   const uint32_t uvs_vertex_vector_size_in_dwords =
      (vs_output_size + 1U + (has_raster_enable ? 4U : 0U)) * 4U;

   if (vdm_cam_size < 96U) {
      /* Override the cam size for the 8XE proxy. */
      if (uvs_vertex_vector_size_in_dwords < (14U * 4U)) {
         *cam_size_out = MIN2(31U, vdm_cam_size - 1U);
         *vs_max_instances_out = 16U;
      } else if (uvs_vertex_vector_size_in_dwords < (20U * 4U)) {
         *cam_size_out = 15U;
         *vs_max_instances_out = 16U;
      } else if (uvs_vertex_vector_size_in_dwords < (28U * 4U)) {
         *cam_size_out = 11U;
         *vs_max_instances_out = 12U;
      } else if (uvs_vertex_vector_size_in_dwords < (44U * 4U)) {
         *cam_size_out = 7U;
         *vs_max_instances_out = 8U;
      } else if (PVR_HAS_FEATURE(dev_info, simple_internal_parameter_format_v2) ||
                 uvs_vertex_vector_size_in_dwords < (64U * 4U)) {
         *cam_size_out = 7U;
         *vs_max_instances_out = 4U;
      } else {
         *cam_size_out = 3U;
         *vs_max_instances_out = 2U;
      }
   } else {
      /* Not this core (vdm_cam_size 64): Mesa's other branch is not taken. */
      *cam_size_out = vdm_cam_size - 1U;
      *vs_max_instances_out = 16U;
   }
}

/* ------------------------------------------------------------------------- *
 * (1) PPP state block -- parameter driven
 * ------------------------------------------------------------------------- */
int32_t pvr_ppp_build_block(const struct pvr_ppp_frag_inputs *in,
                            const struct pvr_ppp_state *st,
                            uint32_t *out,
                            uint32_t *out_count,
                            struct pvr_vdm_field_refusal *why)
{
   uint32_t *dw = out;
   const bool has_color = true;   /* a draw always binds a fragment PDS */
   /* Every size field of the block, in 64 bits, checked against its width
    * in Mesa 26.2.1 csbgen/rogue/ppp.xml before anything is written:
    * OUTPUT_SEL.vtxsize [24:31], STATE_VARYING0 f32_* [0:23],
    * STATE_PDS_SIZEINFO1, STATE_PDS_SIZEINFO2.usc_sharedsize [23:31].
    *
    * pds_texturestatesize counts the texture-state program's DATA segment
    * in units of 4 dwords (16 bytes). Mesa divides the segment's DWORD count
    * by the field's UNIT_SIZE 4: pvr_arch_cmd_buffer.c (and the
    * CR_PDS_BGRND3_SIZEINFO twin) divide pvr_pds_upload.data_size, which
    * pvr_device.c stores as bytes / 4. Our input is in bytes, so it goes to
    * dwords first. Temps (pds_tempsize) and shared registers
    * (usc_sharedsize) are counts divided by their UNIT_SIZE as Mesa does. */
   const uint64_t texstate =
      pvr_units(pvr_units(in->texture_program_data_size_bytes, 4u),
                ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEXTURESTATESIZE_UNIT_SIZE);
   const uint64_t tempsize =
      pvr_units(in->texture_program_temps, ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEMPSIZE_UNIT_SIZE);
   const uint64_t sharedsize =
      pvr_units(in->shared_reg_count, ROGUE_TA_STATE_PDS_SIZEINFO2_USC_SHAREDSIZE_UNIT_SIZE);

   if (out_count)
      *out_count = 0;
   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "TA_OUTPUT_SEL.vtxsize", st->vtxsize, 8, 4) ||
       !pvr_field_fits(why, "TA_STATE_VARYING0.f32_linear", st->f32_linear, 8, 4) ||
       !pvr_field_fits(why, "TA_STATE_VARYING0.f32_flat", st->f32_flat, 8, 4) ||
       !pvr_field_fits(why, "TA_STATE_VARYING0.f32_npc", st->f32_npc, 8, 4) ||
       !pvr_field_fits(why, "TA_STATE_PDS_SIZEINFO1.pds_texturestatesize", texstate, 7,
                       ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEXTURESTATESIZE_UNIT_SIZE * 4u) ||
       !pvr_field_fits(why, "TA_STATE_PDS_SIZEINFO1.pds_tempsize", tempsize, 4,
                       ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_TEMPSIZE_UNIT_SIZE * 4u) ||
       !pvr_field_fits(why, "TA_STATE_PDS_SIZEINFO1.usc_varyingsize", st->usc_varyingsize, 6,
                       ROGUE_TA_STATE_PDS_SIZEINFO1_USC_VARYINGSIZE_UNIT_SIZE * 4u) ||
       !pvr_field_fits(why, "TA_STATE_PDS_SIZEINFO1.pds_varyingsize", st->pds_varyingsize, 6,
                       ROGUE_TA_STATE_PDS_SIZEINFO1_PDS_VARYINGSIZE_UNIT_SIZE * 4u) ||
       !pvr_field_fits(why, "TA_STATE_PDS_SIZEINFO2.usc_sharedsize", sharedsize, 9,
                       ROGUE_TA_STATE_PDS_SIZEINFO2_USC_SHAREDSIZE_UNIT_SIZE * 4u))
      return PVR_VDM_FIELD_REFUSED;
   /* Every other field the block writes, against its ppp.xml width, before
    * anything is written: the enums (a value past the width selects another
    * entry or spills into the next field), the stencil values, the
    * STATE_ISPDBSC table indices (read only when enabled), the region clip
    * tiles (only with clip_enable), the PDS base addresses (16-byte aligned
    * offsets, [4:31] shift 4) and PPP_CTRL. The back-face pair is checked
    * only when it is emitted (two_sided). */
   if (!pvr_field_fits(why, "TA_STATE_ISPCTL.upass", in->isp_userpass, 4, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPA.objtype", st->objtype, 4, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPA.passtype", in->passtype, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPA.dcmpmode", st->depth_cmp, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPA.pointlinewidth", st->point_line_width, 8, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPA.sref", st->stencil_ref, 8, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.scmpmode", st->stencil_cmp, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.sop1", st->stencil_sop_fail, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.sop2", st->stencil_sop_dfail, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.sop3", st->stencil_sop_pass, 3, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.scmpmask", st->stencil_cmp_mask, 8, 0) ||
       !pvr_field_fits(why, "TA_STATE_ISPB.swmask", st->stencil_write_mask, 8, 0) ||
       (st->two_sided &&
        (!pvr_field_fits(why, "TA_STATE_ISPA(back).sref", st->stencil_back_ref, 8, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).scmpmode", st->stencil_back_cmp, 3, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).sop1", st->stencil_back_sop_fail, 3, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).sop2", st->stencil_back_sop_dfail, 3, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).sop3", st->stencil_back_sop_pass, 3, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).scmpmask", st->stencil_back_cmp_mask, 8, 0) ||
         !pvr_field_fits(why, "TA_STATE_ISPB(back).swmask", st->stencil_back_write_mask, 8, 0))) ||
       (st->dbias_enable &&
        !pvr_field_fits(why, "TA_STATE_ISPDBSC.dbindex", st->dbias_index, 16, 0)) ||
       (st->scissor_enable &&
        !pvr_field_fits(why, "TA_STATE_ISPDBSC.scindex", st->scissor_index, 16, 0)) ||
       (st->clip_enable &&
        (!pvr_field_fits(why, "TA_REGION_CLIP0.left", st->clip_left, 9, 0) ||
         !pvr_field_fits(why, "TA_REGION_CLIP0.right", st->clip_right, 9, 0) ||
         !pvr_field_fits(why, "TA_REGION_CLIP1.top", st->clip_top, 9, 0) ||
         !pvr_field_fits(why, "TA_REGION_CLIP1.bottom", st->clip_bottom, 9, 0))) ||
       !pvr_addr_aligned(why, "TA_STATE_PDS_SHADERBASE.addr bits 3:0",
                         in->pixel_program_code_offset, 4) ||
       !pvr_addr_aligned(why, "TA_STATE_PDS_TEXUNICODEBASE.addr bits 3:0",
                         in->texture_program_code_offset, 4) ||
       !pvr_addr_aligned(why, "TA_STATE_PDS_VARYINGBASE.addr bits 3:0",
                         in->varying_program_code_offset, 4) ||
       !pvr_addr_aligned(why, "TA_STATE_PDS_TEXTUREDATABASE.addr bits 3:0",
                         in->texture_program_data_offset, 4) ||
       !pvr_field_fits(why, "TA_STATE_PPP_CTRL.clip_mode", st->clip_mode, 2, 0) ||
       !pvr_field_fits(why, "TA_STATE_PPP_CTRL.flatshade_vtx", st->flatshade_vtx, 2, 0) ||
       !pvr_field_fits(why, "TA_STATE_PPP_CTRL.cullmode", st->cullmode, 2, 0) ||
       (st->vis_test && !pvr_field_fits(why, "TA_STATE_ISPCTL.visreg", st->vis_reg, 14, 0)))
      return PVR_VDM_FIELD_REFUSED;

   /* TA_STATE_HEADER: the words below that are present -- the colour
    * template's (pvr_clear.c, requires_pds_state = has_color), the viewport
    * (one: view_port_count = count - 1 = 0), the back-face ISP pair and
    * ISPDBSC when they are emitted. */
   pvr_pack(dw, TA_STATE_HEADER, h) {
      h.pres_stream_out_size = true;
      h.pres_ppp_ctrl = true;
      h.pres_varying_word2 = true;
      h.pres_varying_word1 = true;
      h.pres_varying_word0 = true;
      h.pres_outselects = true;
      h.pres_wclamp = true;
      h.pres_viewport = st->viewport_enable != 0;
      h.pres_region_clip = true;
      h.pres_pds_state_ptr2 = has_color;
      h.pres_pds_state_ptr1 = has_color;
      h.pres_pds_state_ptr0 = has_color;
      h.pres_ispctl_dbsc = st->dbias_enable || st->scissor_enable;
      h.pres_ispctl_bb = st->two_sided != 0;
      h.pres_ispctl_ba = st->two_sided != 0;
      h.pres_ispctl_fb = true;
      h.pres_ispctl_fa = true;
      h.pres_ispctl = true;
   }
   dw++;

   /* TA_STATE_ISPCTL: tagwritedisable when there is no colour or the
    * caller's colour write is off. */
   pvr_pack(dw, TA_STATE_ISPCTL, c) {
      c.upass = in->isp_userpass;
      c.tagwritedisable = !has_color || st->color_write_disable;
      c.two_sided = st->two_sided != 0;
      c.bpres = true;
      c.dbenable = st->dbias_enable != 0;
      c.scenable = st->scissor_enable != 0;
      c.vistest = st->vis_test != 0;
      c.visreg = st->vis_test ? st->vis_reg : 0;
   }
   dw++;

   /* TA_STATE_ISPA / ISPB, front then (two_sided) back: Mesa
    * pvr_setup_isp_faces_and_control() (dcmpmode/dwritedisable from the
    * depth state, sref the stencil reference; "FA, BA have the ISPA format"
    * in pvr_emit_ppp_state). */
   pvr_pack(dw, TA_STATE_ISPA, a) {
      a.objtype = st->objtype;
      /* in->passtype is the PASSTYPE enum value (== the ISPA field). */
      a.passtype = in->passtype;
      a.dwritedisable = !st->depth_write;
      a.dcmpmode = st->depth_cmp;
      a.pointlinewidth = st->point_line_width;
      a.sref = st->stencil_ref;
   }
   dw++;
   pvr_pack(dw, TA_STATE_ISPB, b) {
      b.scmpmode = st->stencil_cmp;
      b.sop1 = st->stencil_sop_fail;
      b.sop2 = st->stencil_sop_dfail;
      b.sop3 = st->stencil_sop_pass;
      b.scmpmask = st->stencil_cmp_mask;
      b.swmask = st->stencil_write_mask;
   }
   dw++;
   if (st->two_sided) {
      pvr_pack(dw, TA_STATE_ISPA, a) {
         a.objtype = st->objtype;
         a.passtype = in->passtype;
         a.dwritedisable = !st->depth_write;
         a.dcmpmode = st->depth_cmp;
         a.pointlinewidth = st->point_line_width;
         a.sref = st->stencil_back_ref;
      }
      dw++;
      pvr_pack(dw, TA_STATE_ISPB, b) {
         b.scmpmode = st->stencil_back_cmp;
         b.sop1 = st->stencil_back_sop_fail;
         b.sop2 = st->stencil_back_sop_dfail;
         b.sop3 = st->stencil_back_sop_pass;
         b.scmpmask = st->stencil_back_cmp_mask;
         b.swmask = st->stencil_back_write_mask;
      }
      dw++;
   }

   /* TA_STATE_ISPDBSC: the depth bias and scissor table indices (each read
    * only when ISPCTL.dbenable / .scenable is set), after the ISP words and
    * before the PDS words (pvr_emit_ppp_state). */
   if (st->dbias_enable || st->scissor_enable) {
      pvr_pack(dw, TA_STATE_ISPDBSC, d) {
         d.dbindex = st->dbias_enable ? st->dbias_index : 0u;
         d.scindex = st->scissor_enable ? st->scissor_index : 0u;
      }
      dw++;
   }

   /* The six TA_STATE_PDS_* words of the fragment PDS binding, in
    * PVR_STATIC_CLEAR_PPP_PDS_TYPE_* order (pvr_clear.h), as pvr_blit.c packs
    * them. VARYINGBASE 0 is the dummy program at PDS offset 0 (only when
    * nothing is iterated). */
   pvr_pack(dw, TA_STATE_PDS_SHADERBASE, p) {
      p.addr = PVR_DEV_ADDR(in->pixel_program_code_offset);
   }
   dw++;
   pvr_pack(dw, TA_STATE_PDS_TEXUNICODEBASE, p) {
      p.addr = PVR_DEV_ADDR(in->texture_program_code_offset);
   }
   dw++;
   pvr_pack(dw, TA_STATE_PDS_SIZEINFO1, p) {
      p.pds_texturestatesize = (uint32_t)texstate;
      p.pds_varyingsize = st->pds_varyingsize;
      p.usc_varyingsize = st->usc_varyingsize;
      p.pds_tempsize = (uint32_t)tempsize;
   }
   dw++;
   pvr_pack(dw, TA_STATE_PDS_SIZEINFO2, p) {
      p.usc_sharedsize = (uint32_t)sharedsize;
      p.pds_tri_merge_disable = st->tri_merge_disable != 0;
   }
   dw++;
   pvr_pack(dw, TA_STATE_PDS_VARYINGBASE, p) {
      p.addr = PVR_DEV_ADDR(in->varying_program_code_offset);
   }
   dw++;
   pvr_pack(dw, TA_STATE_PDS_TEXTUREDATABASE, p) {
      p.addr = PVR_DEV_ADDR(in->texture_program_data_offset);
   }
   dw++;

   /* TA_REGION_CLIP0/1: tile granular, OUTSIDE mode; with no clip the whole
    * screen (left = top = 0, right = bottom = ROGUE_TA_REGION_CLIP_MAX). */
   pvr_pack(dw, TA_REGION_CLIP0, c) {
      c.mode = ROGUE_TA_REGION_CLIP_MODE_OUTSIDE;
      c.left = st->clip_enable ? st->clip_left : 0u;
      c.right = st->clip_enable ? st->clip_right : ROGUE_TA_REGION_CLIP_MAX;
   }
   dw++;
   pvr_pack(dw, TA_REGION_CLIP1, c) {
      c.top = st->clip_enable ? st->clip_top : 0u;
      c.bottom = st->clip_enable ? st->clip_bottom : ROGUE_TA_REGION_CLIP_MAX;
   }
   dw++;

   /* Viewport words (pvr_emit_ppp_state): a0 m0 a1 m1 a2 m2 = x_center
    * x_scale y_center y_scale z_center z_scale as f32 bits. csbgen has no
    * definition for these (Mesa: "none will be added"). */
   if (st->viewport_enable) {
      for (unsigned i = 0; i < 6; i++)
         *dw++ = st->viewport[i];
   }

   /* TA_WCLAMP = fui(1.0e-13f), the clear template's (pvr_clear.c). */
   pvr_pack(dw, TA_WCLAMP, w) {
      w.val = fui(1.0e-13f);
   }
   dw++;

   /* TA_OUTPUT_SEL: vtxsize = VS output dwords, tsp_unclamped_z_pres when
    * the FS iterates Z (pvr_arch_cmd_buffer.c, from uses.z),
    * psprite_size_pres when the VS writes gl_PointSize, rhw_pres when the FS
    * uses 1/w. */
   pvr_pack(dw, TA_OUTPUT_SEL, o) {
      o.vtxsize = st->vtxsize;
      o.tsp_unclamped_z_pres = st->z_pres != 0;
      o.psprite_size_pres = st->psprite_size_pres != 0;
      o.rhw_pres = st->rhw_pres != 0;
   }
   dw++;

   /* TA_STATE_VARYING0/1/2: the f32 varyings by interpolation; no f16, no
    * clip planes. */
   pvr_pack(dw, TA_STATE_VARYING0, v) {
      v.f32_linear = st->f32_linear;
      v.f32_flat = st->f32_flat;
      v.f32_npc = st->f32_npc;
   }
   dw++;
   pvr_pack(dw, TA_STATE_VARYING1, v) {
      (void)v;
   }
   dw++;
   pvr_pack(dw, TA_STATE_VARYING2, v) {
      (void)v;
   }
   dw++;

   /* TA_STATE_PPP_CTRL (pvr_setup_ppp_control()). */
   pvr_pack(dw, TA_STATE_PPP_CTRL, c) {
      c.clip_mode = st->clip_mode;
      c.drawclippededges = st->drawclippededges != 0;
      c.flatshade_vtx = st->flatshade_vtx;
      c.pretransform = st->pretransform != 0;
      c.wclampen = st->wclampen != 0;
      c.cullmode = st->cullmode;
   }
   dw++;

   /* TA_STATE_STREAM_OUT0: no stream out. */
   pvr_pack(dw, TA_STATE_STREAM_OUT0, s) {
      (void)s;
   }
   dw++;

   {
      uint32_t count = (uint32_t)(dw - out);   /* 19, or 25 with a viewport */
      if (out_count)
         *out_count = count;
      return (int32_t)count;
   }
}

/* Knob-driven wrapper: reproduces the historical single-draw block. */
int32_t pvr_ppp_build_clear_block(const struct pvr_ppp_frag_inputs *in,
                                  uint32_t *out,
                                  uint32_t *out_count,
                                  struct pvr_vdm_field_refusal *why)
{
   struct pvr_ppp_state st;
   struct pvr_ppp_frag_inputs fi = *in;

   memset(&st, 0, sizeof(st));

   /* Object type must match the VDM primitive topology. */
#if defined(PVR_PRIM_LINES)
   st.objtype = PVR_TA_OBJTYPE_LINE;
#elif defined(PVR_PRIM_POINTS)
   st.objtype = PVR_TA_OBJTYPE_POINT_FILLED_TRIANGLE;
#else
   st.objtype = PVR_TA_OBJTYPE_TRIANGLE;
#endif

   /* GL-ES depth test (PVR_DEPTH): LESS + write; else no depth. */
#if defined(PVR_DEPTH) && !defined(PVR_DEPTH_PAINTORDER)
   st.depth_cmp = PVR_TA_CMPMODE_LESS;
   st.depth_write = 1;
#else
   st.depth_cmp = PVR_TA_CMPMODE_ALWAYS;
   st.depth_write = 0;
#endif
   st.stencil_cmp = PVR_TA_CMPMODE_ALWAYS;
   st.stencil_sop_fail = st.stencil_sop_dfail = st.stencil_sop_pass = PVR_TA_STENCILOP_KEEP;

   /* Region clip: PVR_SCISSOR (tile-granular scissor) / PVR_SMALL_CLIP diag. */
#if defined(PVR_SCISSOR)
#ifndef PVR_SCISSOR_L
#define PVR_SCISSOR_L 0u
#endif
#ifndef PVR_SCISSOR_T
#define PVR_SCISSOR_T 0u
#endif
#ifndef PVR_SCISSOR_R
#define PVR_SCISSOR_R 0u
#endif
#ifndef PVR_SCISSOR_B
#define PVR_SCISSOR_B 0u
#endif
   st.clip_enable = 1;
   st.clip_left = (uint32_t)(PVR_SCISSOR_L);
   st.clip_top = (uint32_t)(PVR_SCISSOR_T);
   st.clip_right = (uint32_t)(PVR_SCISSOR_R);
   st.clip_bottom = (uint32_t)(PVR_SCISSOR_B);
#elif defined(PVR_SMALL_CLIP)
   st.clip_enable = 1;
   st.clip_right = 1;
   st.clip_bottom = 1;
#endif

   /* Output select / varyings / varying sizes. */
#if defined(PVR_MULTIVARY)
   st.vtxsize = 10;                 /* V1: pos4 + color4 + uv2 */
   st.rhw_pres = 1;                 /* perspective W plane */
   st.f32_linear = 6;               /* both varyings smooth */
   st.usc_varyingsize = 2;          /* ceil(28/16) */
   st.pds_varyingsize = 2;          /* ceil(8/4) */
#else
   st.vtxsize = 8;                  /* 8-dword VS output (DDK contract) */
   st.rhw_pres = 0;
#if defined(PVR_GRADIENT) && !defined(PVR_GRAD_FLAT)
#ifndef PVR_VARYING0_VAL
#define PVR_VARYING0_VAL 0x00040000u   /* f32_npc = 4 */
#endif
   {
      /* the knob is a whole TA_STATE_VARYING0 word */
      const uint32_t varying0 = (uint32_t)(PVR_VARYING0_VAL);
      struct ROGUE_TA_STATE_VARYING0 v;

      ROGUE_TA_STATE_VARYING0_unpack(&varying0, &v);
      st.f32_linear = v.f32_linear;
      st.f32_flat = v.f32_flat;
      st.f32_npc = v.f32_npc;
   }
#else
   st.f32_flat = 4;                 /* DDK captured flat config (0x400) */
#endif
#if defined(PVR_GRADIENT)
   st.usc_varyingsize = 1;          /* one 4-dword iterated varying (0x410) */
   st.pds_varyingsize = 1;
#endif
#endif

   /* PPP_CTRL: pretransform (screen-space vertices) + cull mode. */
   st.pretransform = 1;
#if defined(PVR_CULL)
#ifndef PVR_CULL_MODE
#define PVR_CULL_MODE 1u   /* CULL_CW */
#endif
   st.cullmode = (uint32_t)(PVR_CULL_MODE);
#else
   st.cullmode = PVR_TA_CULLMODE_NONE;
#endif

#ifdef PVR_NO_COEFF_PROG
   fi.varying_program_code_offset = 0;
#endif
   return pvr_ppp_build_block(&fi, &st, out, out_count, why);
}

/* The trailing PPP block of a render: TA_STATE_HEADER with only
 * pres_terminate, TA_STATE_TERMINATE0 (the last 32-px block right/bottom,
 * top 0) and TA_STATE_TERMINATE1 (left 0, render target 0). */
int32_t pvr_ppp_build_terminate_block(uint32_t width, uint32_t height, uint32_t out[3],
                                      struct pvr_vdm_field_refusal *why)
{
   /* The last 32-px block, in 64 bits: a zero width or height has no last
    * block, its index wraps to 2^64 - 1 and is refused. clip_right and
    * clip_bottom are 9 bits (ppp.xml), so at most 16384 px. */
   const uint64_t right =
      pvr_units(width, ROGUE_TA_STATE_TERMINATE0_CLIP_RIGHT_BLOCK_SIZE_IN_PIXELS) - 1u;
   const uint64_t bottom =
      pvr_units(height, ROGUE_TA_STATE_TERMINATE0_CLIP_BOTTOM_BLOCK_SIZE_IN_PIXELS) - 1u;

   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "TA_STATE_TERMINATE0.clip_right", right, 9, 0) ||
       !pvr_field_fits(why, "TA_STATE_TERMINATE0.clip_bottom", bottom, 9, 0))
      return PVR_VDM_FIELD_REFUSED;
   pvr_pack(&out[0], TA_STATE_HEADER, h) {
      h.pres_terminate = true;
   }
   pvr_pack(&out[1], TA_STATE_TERMINATE0, t) {
      t.clip_right = (uint32_t)right;
      t.clip_top = 0;
      t.clip_bottom = (uint32_t)bottom;
   }
   pvr_pack(&out[2], TA_STATE_TERMINATE1, t) {
      t.clip_left = 0;
      t.render_target = 0;
   }
   return 3;
}

/* ------------------------------------------------------------------------- *
 * (2) VDM control stream -- emitters
 * ------------------------------------------------------------------------- */
int32_t pvr_vdm_emit_ppp_state(uint32_t *dw, uint64_t ppp_dev_addr, uint32_t word_count,
                               struct pvr_vdm_field_refusal *why)
{
   /* vdm.xml: word_count [8:15]; the address is 40 bits (addrmsb [0:7]
    * shift 32, addrlsb [2:31] shift 2), so 4-byte aligned and below 2^40.
    * Checked before anything is written. */
   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "VDMCTRL_PPP_STATE0.word_count", word_count, 8, 0) ||
       !pvr_field_fits(why, "VDMCTRL_PPP_STATE0.addrmsb (address bits 39:32)", ppp_dev_addr >> 32, 8, 0) ||
       !pvr_addr_aligned(why, "VDMCTRL_PPP_STATE1.addrlsb (address bits 1:0)", ppp_dev_addr, 2))
      return PVR_VDM_FIELD_REFUSED;
   pvr_pack(&dw[0], VDMCTRL_PPP_STATE0, s) {
      s.word_count = word_count;
      s.addrmsb = PVR_DEV_ADDR(ppp_dev_addr);
   }
   pvr_pack(&dw[1], VDMCTRL_PPP_STATE1, s) {
      s.addrlsb = PVR_DEV_ADDR(ppp_dev_addr);
   }
   return 2;
}

int32_t pvr_vdm_emit_pds_state(uint32_t *dw, const struct pvr_vdm_pds_state *ps,
                               struct pvr_vdm_field_refusal *why)
{
   /* VDMCTRL_PDS_STATE0..2: the VDM's own PDS program (dm_target VDM, usc
    * target ALL, no unified store), its sizes in the XML's units; a size
    * that does not fit its bits refuses the block (nothing written). */
   const uint64_t common =
      pvr_units(ps->usc_common_size_bytes, ROGUE_VDMCTRL_PDS_STATE0_USC_COMMON_SIZE_UNIT_SIZE);
   const uint64_t temp =
      pvr_units((uint64_t)ps->pds_temps * 4u, ROGUE_VDMCTRL_PDS_STATE0_PDS_TEMP_SIZE_UNIT_SIZE);
   const uint64_t data =
      pvr_units(ps->pds_data_size_bytes, ROGUE_VDMCTRL_PDS_STATE0_PDS_DATA_SIZE_UNIT_SIZE);

   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "PDS_STATE0.usc_common_size", common, 9,
                       ROGUE_VDMCTRL_PDS_STATE0_USC_COMMON_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "PDS_STATE0.pds_temp_size", temp, 4,
                       ROGUE_VDMCTRL_PDS_STATE0_PDS_TEMP_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "PDS_STATE0.pds_data_size", data, 6,
                       ROGUE_VDMCTRL_PDS_STATE0_PDS_DATA_SIZE_UNIT_SIZE) ||
       !pvr_addr_aligned(why, "PDS_STATE1.pds_data_addr bits 3:0", ps->pds_data_offset, 4) ||
       !pvr_addr_aligned(why, "PDS_STATE2.pds_code_addr bits 3:0", ps->pds_code_offset, 4))
      return PVR_VDM_FIELD_REFUSED;
   pvr_pack(&dw[0], VDMCTRL_PDS_STATE0, s) {
      s.dm_target = ROGUE_VDMCTRL_DM_TARGET_VDM;
      s.usc_target = ROGUE_VDMCTRL_USC_TARGET_ALL;
      s.usc_common_size = (uint32_t)common;
      s.usc_unified_size = 0;
      s.pds_temp_size = (uint32_t)temp;
      s.pds_data_size = (uint32_t)data;
   }
   pvr_pack(&dw[1], VDMCTRL_PDS_STATE1, s) {
      s.pds_data_addr = PVR_DEV_ADDR(ps->pds_data_offset);
      s.sd_type = ROGUE_VDMCTRL_SD_TYPE_NONE;
      s.sd_next_type = ROGUE_VDMCTRL_SD_TYPE_NONE;
   }
   pvr_pack(&dw[2], VDMCTRL_PDS_STATE2, s) {
      s.pds_code_addr = PVR_DEV_ADDR(ps->pds_code_offset);
   }
   return 3;
}

int32_t pvr_vdm_emit_vdm_state(uint32_t *dw, const struct pvr_vdm_vs_state *vs,
                               struct pvr_vdm_field_refusal *why)
{
   uint32_t cam_size, max_instances;
   const uint64_t vs_output_size =
      pvr_units(vs->vs_output_size_bytes, ROGUE_VDMCTRL_VDM_STATE4_VS_OUTPUT_SIZE_UNIT_SIZE);
   const uint32_t vs_input_size_bytes =
      vs->vs_input_size_bytes ? vs->vs_input_size_bytes : vs->vs_output_size_bytes;
   const uint64_t unified =
      pvr_units(vs_input_size_bytes, ROGUE_VDMCTRL_VDM_STATE5_VS_USC_UNIFIED_SIZE_UNIT_SIZE);
   /* vs_pds_temps counts 32-bit PDS temps; the field's unit is 16 bytes
    * (4 temps). Mesa's draw path: DIV_ROUND_UP(temps_required << 2, 16)
    * (pvr_arch_cmd_buffer.c), the same bytes rule as
    * PDS_STATE0.pds_temp_size. */
   const uint64_t temp =
      pvr_units((uint64_t)vs->vs_pds_temps * 4u, ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_TEMP_SIZE_UNIT_SIZE);
   const uint64_t data =
      pvr_units(vs->vs_pds_data_size_bytes, ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_DATA_SIZE_UNIT_SIZE);

   pvr_vdm_calc_cam_size_bxs4_64(vs->vs_output_size_bytes, /*raster*/ 1,
                                 &cam_size, &max_instances);
   /* Every size field of the five words, widths from vdm.xml VDM_STATE0/4/5,
    * then flatshade_control (VDM_STATE0 [1:2]) and the two 16-byte aligned
    * PDS offsets (VDM_STATE2/3 [4:31] shift 4). cam_size and
    * vs_max_instances come from the table above (at most 31 and 16) and
    * cannot miss today; they are checked all the same, so a new table row is
    * refused rather than masked. Nothing is written on a refusal. */
   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "VDM_STATE0.cam_size", cam_size, 8, 1) ||
       !pvr_field_fits(why, "VDM_STATE4.vs_output_size", vs_output_size, 8,
                       ROGUE_VDMCTRL_VDM_STATE4_VS_OUTPUT_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "VDM_STATE5.vs_max_instances", max_instances, 5, 1) ||
       !pvr_field_fits(why, "VDM_STATE5.vs_usc_unified_size", unified, 6,
                       ROGUE_VDMCTRL_VDM_STATE5_VS_USC_UNIFIED_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "VDM_STATE5.vs_pds_temp_size", temp, 4,
                       ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_TEMP_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "VDM_STATE5.vs_pds_data_size", data, 6,
                       ROGUE_VDMCTRL_VDM_STATE5_VS_PDS_DATA_SIZE_UNIT_SIZE) ||
       !pvr_field_fits(why, "VDM_STATE0.flatshade_control", vs->flatshade_control, 2, 0) ||
       !pvr_addr_aligned(why, "VDM_STATE2.vs_pds_data_base_addr bits 3:0", vs->vs_pds_data_offset, 4) ||
       !pvr_addr_aligned(why, "VDM_STATE3.vs_pds_code_base_addr bits 3:0", vs->vs_pds_code_offset, 4))
      return PVR_VDM_FIELD_REFUSED;

   /* pvr_clear.c pvr_pack_clear_vdm_state(): the VS data address and the
    * other state present, the UVS scratch size FIVE. */
   pvr_pack(&dw[0], VDMCTRL_VDM_STATE0, s) {
      s.vs_data_addr_present = true;
      s.vs_other_present = true;
      s.cam_size = cam_size;
      s.uvs_scratch_size_select = ROGUE_VDMCTRL_UVS_SCRATCH_SIZE_SELECT_FIVE;
      s.flatshade_control = vs->flatshade_control;
   }
   pvr_pack(&dw[1], VDMCTRL_VDM_STATE2, s) {
      s.vs_pds_data_base_addr = PVR_DEV_ADDR(vs->vs_pds_data_offset);
   }
   pvr_pack(&dw[2], VDMCTRL_VDM_STATE3, s) {
      s.vs_pds_code_base_addr = PVR_DEV_ADDR(vs->vs_pds_code_offset);
   }
   pvr_pack(&dw[3], VDMCTRL_VDM_STATE4, s) {
      s.vs_output_size = (uint32_t)vs_output_size;
   }
   /* vs_usc_unified_size: the input vertex the PDS DMAs into the unified
    * store (Mesa pvr_arch_cmd_buffer.c vtxins, pvr_clear.c). */
   pvr_pack(&dw[4], VDMCTRL_VDM_STATE5, s) {
      s.vs_max_instances = max_instances;
      s.vs_usc_unified_size = (uint32_t)unified;
      s.vs_pds_temp_size = (uint32_t)temp;
      s.vs_pds_data_size = (uint32_t)data;
   }
   return 5;
}

/* VDMCTRL_INDEX_LIST0..3 of one draw, in the order Mesa's
 * pvr_emit_vdm_index_list() emits them: LIST0, then the address (LIST1),
 * the count (LIST2) and the instance count (LIST3) that LIST0 says are
 * present. */
static int32_t index_list(uint32_t *dw, uint32_t topology, uint64_t index_buf_dev_addr,
                          uint32_t index_count, uint32_t instance_count,
                          struct pvr_vdm_field_refusal *why)
{
   const bool indexed = index_buf_dev_addr != 0;
   const bool instanced = instance_count > 1;
   int32_t n = 0;

   /* vdm.xml: primitive_topology [8:11]; index_base_addrmsb [0:7] shift 32
    * (the address is below 2^40); INDEX_LIST1 carries address bits 31:0
    * with no shift. Our recipe records it 4-byte aligned
    * (driver/docs/gpu-recipes/multidraw_stencil_recipe.md); the GL layer
    * stages indices at other offsets, and a misaligned address is refused
    * here rather than masked (whether the VDM reads a 2-aligned base is not
    * measured). index_count and instance_count are 32 bits. */
   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "VDMCTRL_INDEX_LIST0.primitive_topology", topology, 4, 0) ||
       (indexed &&
        (!pvr_field_fits(why, "VDMCTRL_INDEX_LIST0.index_base_addrmsb (address bits 39:32)",
                         index_buf_dev_addr >> 32, 8, 0) ||
         !pvr_addr_aligned(why, "VDMCTRL_INDEX_LIST1.index_base_addrlsb (address bits 1:0)",
                           index_buf_dev_addr, 2))))
      return PVR_VDM_FIELD_REFUSED;
   /* Indexed draws use the on-silicon-proven encoding (the DDK
    * rgx_triangle_test's INDEX_LIST0 0x78020680): u16 indices, address and
    * count present. Non-indexed ones the Mesa clear-rect form: the count
    * only. */
   pvr_pack(&dw[n], VDMCTRL_INDEX_LIST0, l) {
      l.index_addr_present = indexed;
      l.index_count_present = true;
      l.index_instance_count_present = instanced;
      l.index_size = indexed ? ROGUE_VDMCTRL_INDEX_SIZE_B16 : ROGUE_VDMCTRL_INDEX_SIZE_B8;
      l.primitive_topology = topology;
      l.index_base_addrmsb = PVR_DEV_ADDR(index_buf_dev_addr);
   }
   n++;
   if (indexed) {
      pvr_pack(&dw[n], VDMCTRL_INDEX_LIST1, l) {
         l.index_base_addrlsb = PVR_DEV_ADDR(index_buf_dev_addr);
      }
      n++;
   }
   pvr_pack(&dw[n], VDMCTRL_INDEX_LIST2, l) {
      l.index_count = index_count;
   }
   n++;
   if (instanced) {
      pvr_pack(&dw[n], VDMCTRL_INDEX_LIST3, l) {
         l.instance_count = instance_count - 1u;
      }
      n++;
   }
   return n;
}

int32_t pvr_vdm_emit_index_list(uint32_t *dw, uint32_t topology,
                                uint64_t index_buf_dev_addr, uint32_t index_count,
                                struct pvr_vdm_field_refusal *why)
{
   return index_list(dw, topology, index_buf_dev_addr, index_count, 1, why);
}

int32_t pvr_vdm_emit_index_list_instanced(uint32_t *dw, uint32_t topology,
                                          uint64_t index_buf_dev_addr, uint32_t index_count,
                                          uint32_t instance_count,
                                          struct pvr_vdm_field_refusal *why)
{
   return index_list(dw, topology, index_buf_dev_addr, index_count, instance_count, why);
}

uint32_t pvr_vdm_emit_terminate(uint32_t *dw)
{
   pvr_pack(&dw[0], VDMCTRL_STREAM_TERMINATE, t) {
      t.context = false;
   }
   return 1;
}

/* Knob-driven single-draw stream (silicon test rig). */
uint32_t pvr_vdm_build_clear_stream(const struct pvr_vdm_inputs *in,
                                    uint32_t *out,
                                    uint32_t *out_count,
                                    struct pvr_vdm_field_refusal *why)
{
   uint32_t *dw = out;
   struct pvr_vdm_vs_state vs = {
      .vs_pds_data_offset = in->vs_pds_data_offset,
      .vs_pds_code_offset = in->vs_pds_code_offset,
      .vs_pds_data_size_bytes = in->vs_pds_data_size_bytes,
      .vs_pds_temps = in->vs_pds_temps,
      .vs_output_size_bytes = in->vs_output_size_bytes,
      .vs_input_size_bytes = 0,   /* the clear quad: inputs == outputs */
#ifdef PVR_FLATSHADE_VTX
      .flatshade_control = (uint32_t)(PVR_FLATSHADE_VTX),
#else
      .flatshade_control = 0,
#endif
   };
   /* Primitive topology: default TRI_LIST(6). PVR_PRIM_LINES -> LINE_LIST(1),
    * PVR_PRIM_POINTS -> POINT_LIST(0). Must match the ISPA objtype in the PPP. */
#if defined(PVR_PRIM_LINES)
   const uint32_t topology = PVR_VDM_TOPOLOGY_LINE_LIST;
#elif defined(PVR_PRIM_POINTS)
   const uint32_t topology = PVR_VDM_TOPOLOGY_POINT_LIST;
#else
   const uint32_t topology = PVR_VDM_TOPOLOGY_TRI_LIST;
#endif

   /* Each emitter refuses a field that does not fit (nothing written);
    * the stream is then refused whole: 0 bytes, *out_count 0. */
#define PVR_STREAM_EMIT(call)                                                  \
   do {                                                                        \
      const int32_t n_ = (call);                                               \
      if (n_ < 0) {                                                            \
         if (out_count)                                                        \
            *out_count = 0;                                                    \
         return 0;                                                             \
      }                                                                        \
      dw += n_;                                                                \
   } while (0)

   PVR_STREAM_EMIT(pvr_vdm_emit_ppp_state(dw, in->ppp_state_dev_addr, in->ppp_word_count, why));
   PVR_STREAM_EMIT(pvr_vdm_emit_vdm_state(dw, &vs, why));

   if (in->index_buf_dev_addr)
      PVR_STREAM_EMIT(pvr_vdm_emit_index_list(dw, topology, in->index_buf_dev_addr,
                                              in->index_count, why));
   else
      PVR_STREAM_EMIT(pvr_vdm_emit_index_list(dw, PVR_VDM_TOPOLOGY_TRI_STRIP, 0,
                                              in->index_count, why));

   /* Optional 2nd draw call: a second INDEX_LIST block sharing the latched
    * PPP/VDM state (GL-ES multiple draws). */
   if (in->index_count2 && in->index_buf2_dev_addr)
      PVR_STREAM_EMIT(pvr_vdm_emit_index_list(dw, topology, in->index_buf2_dev_addr,
                                              in->index_count2, why));

#ifndef PVR_NO_VDM_2ND_PPP
   /* REQUIRED for the TA to tile (decisive fix 2026-09-12): the DDK draw emits a
    * 2nd PPP_STATE_UPDATE(3 words) after the index list, before terminate
    * (ddk_vdm[10]=0x00000380 hdr, [11]=PPP+0x4c). It references PPP words 19-21
    * (the trailing terminate state the caller appends at ppp+0x4c). */
   PVR_STREAM_EMIT(pvr_vdm_emit_ppp_state(dw, in->ppp_state_dev_addr + 0x4cu, 3, why));
#endif
#undef PVR_STREAM_EMIT
   dw += pvr_vdm_emit_terminate(dw);

   {
      uint32_t count = (uint32_t)(dw - out);
      if (out_count)
         *out_count = count;
      return count * (uint32_t)sizeof(uint32_t);
   }
}

/* ------------------------------------------------------------------------- *
 * (3) Clear-rect vertex buffer (pvr_clear_vertices_upload, pvr_clear.c:442)
 * ------------------------------------------------------------------------- */
uint32_t pvr_clear_vertices_fill(int32_t x0, int32_t y0,
                                 uint32_t w, uint32_t h,
                                 float depth,
                                 float out[PVR_CLEAR_VERTEX_COUNT]
                                          [PVR_CLEAR_VERTEX_COORDINATES])
{
   const float fx0 = (float)x0;
   const float fy0 = (float)y0;
   const float fx1 = (float)(x0 + (int32_t)w);
   const float fy1 = (float)(y0 + (int32_t)h);

   out[0][0] = fx0; out[0][1] = fy0; out[0][2] = depth;
   out[1][0] = fx0; out[1][1] = fy1; out[1][2] = depth;
   out[2][0] = fx1; out[2][1] = fy0; out[2][2] = depth;
   out[3][0] = fx1; out[3][1] = fy1; out[3][2] = depth;

   /* stride = 12 bytes, total = 48 bytes */
   return (uint32_t)sizeof(float) * PVR_CLEAR_VERTEX_COUNT *
          PVR_CLEAR_VERTEX_COORDINATES;
}

/* ------------------------------------------------------------------------- *
 * (4) IPF scissor table: IPF_SCISSOR_WORD_0/1 (csbgen rogue/ipf.xml)
 * ------------------------------------------------------------------------- */
int32_t pvr_isp_build_scissor_word(uint32_t xmin, uint32_t xmax,
                                   uint32_t ymin, uint32_t ymax,
                                   uint32_t out[2],
                                   struct pvr_vdm_field_refusal *why)
{
   if (why)
      why->field = 0;
   if (!pvr_field_fits(why, "IPF_SCISSOR_WORD_0.scw0_xmin", xmin, 16, 0) ||
       !pvr_field_fits(why, "IPF_SCISSOR_WORD_0.scw0_xmax", xmax, 16, 0) ||
       !pvr_field_fits(why, "IPF_SCISSOR_WORD_1.scw1_ymin", ymin, 16, 0) ||
       !pvr_field_fits(why, "IPF_SCISSOR_WORD_1.scw1_ymax", ymax, 16, 0))
      return PVR_VDM_FIELD_REFUSED;
   pvr_pack(&out[0], IPF_SCISSOR_WORD_0, w) {
      w.scw0_xmin = xmin;
      w.scw0_xmax = xmax;
   }
   pvr_pack(&out[1], IPF_SCISSOR_WORD_1, w) {
      w.scw1_ymin = ymin;
      w.scw1_ymax = ymax;
   }
   return 2;
}
