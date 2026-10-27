/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_render_regs.c -- render register words packed by Mesa 26.2.1's csbgen
 * (pvr_render_regs.h).
 */
#include <stdint.h>
#include <string.h>

#include "pvr_mesa.h"
#include "pvr_render_regs.h"

static uint64_t qword(const uint32_t dw[2])
{
   uint64_t q;

   memcpy(&q, dw, sizeof(q));
   return q;
}

uint32_t pvr_cr_event_pixel_pds_info(uint32_t num_rt)
{
   struct pvr_pds_event_program program;
   uint32_t w;

   /* The data segment's size, from Mesa's own sizing pass. */
   memset(&program, 0, sizeof(program));
   program.num_emit_word_pairs = num_rt;
   pvr_pds_generate_pixel_event(&program, NULL, PDS_GENERATE_SIZES, pvr_mesa_dev_info(false));
   pvr_pack(&w, CR_EVENT_PIXEL_PDS_INFO, v) {
      v.const_size =
         DIV_ROUND_UP(program.data_size, ROGUE_CR_EVENT_PIXEL_PDS_INFO_CONST_SIZE_UNIT_SIZE);
      v.temp_stride = 0;
      v.usc_sr_size =
         DIV_ROUND_UP(2u * num_rt, ROGUE_CR_EVENT_PIXEL_PDS_INFO_USC_SR_SIZE_UNIT_SIZE);
   }
   return w;
}

uint32_t pvr_cr_isp_aa(uint32_t samples)
{
   uint32_t w;

   pvr_pack(&w, CR_ISP_AA, v) {
      v.mode = samples == 4   ? ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X
               : samples == 2 ? ROGUE_CR_ISP_AA_MODE_TYPE_AA_2X
                              : ROGUE_CR_ISP_AA_MODE_TYPE_AA_NONE;
   }
   return w;
}

uint32_t pvr_cr_isp_ctl(bool skip_init_hdrs, bool process_empty_tiles, bool sample_pos,
                        uint32_t pipe_enable)
{
   uint32_t w;

   pvr_pack(&w, CR_ISP_CTL, v) {
      v.skip_init_hdrs = skip_init_hdrs;
      v.process_empty_tiles = process_empty_tiles;
      v.sample_pos = sample_pos;
      v.pipe_enable = pipe_enable;
   }
   return w;
}

uint32_t pvr_cr_usc_pixel_output_ctrl(uint32_t width_regs, uint32_t partitions)
{
   uint32_t w;

   pvr_pack(&w, CR_USC_PIXEL_OUTPUT_CTRL, v) {
      v.width = width_regs == 1   ? ROGUE_CR_PIXEL_WIDTH_1REGISTER
                : width_regs == 4 ? ROGUE_CR_PIXEL_WIDTH_4REGISTERS
                : width_regs == 8 ? ROGUE_CR_PIXEL_WIDTH_8REGISTERS
                                  : ROGUE_CR_PIXEL_WIDTH_2REGISTERS;
      /* Mesa pvr_arch_setup_tiles_in_flight(), usc_pixel_partition_mask */
      v.partition_mask = (1u << partitions) - 1u;
   }
   return w;
}

uint32_t pvr_cr_isp_bgobjvals(bool enablebgtag, bool mask, uint32_t stencil)
{
   uint32_t w;

   pvr_pack(&w, CR_ISP_BGOBJVALS, v) {
      v.enablebgtag = enablebgtag;
      v.mask = mask;
      v.stencil = stencil;
   }
   return w;
}

void pvr_pbe_reg_words_linear_rgba(uint32_t width, uint32_t height, uint32_t stride_dwords,
                                   uint64_t out[2])
{
   uint32_t w[2];

   pvr_pack(w, PBESTATE_REG_WORD0, r) {
      r.tilerelative = true;
      /* linestride counts pairs of dwords, less one (Mesa
       * pvr_arch_pbe_setup_emit) */
      r.linestride = (stride_dwords - 1u) / 2u;
      r.memlayout = ROGUE_PBESTATE_MEMLAYOUT_LINEAR;
      r.swiz_chan0 = ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0;
      r.swiz_chan1 = ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1;
      r.swiz_chan2 = ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2;
      r.swiz_chan3 = ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3;
   }
   out[0] = qword(w);
   pvr_pack(w, PBESTATE_REG_WORD1, r) {
      r.maxclip_x = width - 1u;
      r.maxclip_y = height - 1u;
   }
   out[1] = qword(w);
}

void pvr_cr_pds_bgnd(uint32_t shader_addr, uint32_t texunicode_addr, uint32_t texturedata_addr,
                     uint32_t usc_sharedsize, uint32_t pds_texturestatesize,
                     uint32_t pds_tempsize, uint64_t out[3])
{
   uint32_t w[2];

   pvr_pack(w, CR_PDS_BGRND0_BASE, b) {
      b.shader_addr = PVR_DEV_ADDR(shader_addr);
      b.texunicode_addr = PVR_DEV_ADDR(texunicode_addr);
   }
   out[0] = qword(w);
   pvr_pack(w, CR_PDS_BGRND1_BASE, b) {
      b.texturedata_addr = PVR_DEV_ADDR(texturedata_addr);
   }
   out[1] = qword(w);
   pvr_pack(w, CR_PDS_BGRND3_SIZEINFO, b) {
      b.usc_sharedsize = usc_sharedsize;
      b.pds_texturestatesize = pds_texturestatesize;
      b.pds_tempsize = pds_tempsize;
   }
   out[2] = qword(w);
}
