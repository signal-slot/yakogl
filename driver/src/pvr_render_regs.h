/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_render_regs.h -- the render register words the C side of the driver
 * builds (the render service, pvr_render.c, and the self-test rig in
 * pvr_fw_boot.c), each packed by Mesa 26.2.1's csbgen
 * (driver/src/pvr/generated/csbgen/rogue/{cr,pbestate}.h) in
 * pvr_render_regs.c. The files that call these include the DDK and Zephyr
 * headers, which do not mix with Mesa's util headers, so the packing lives
 * behind plain functions. The Rust backend packs its own words through its
 * bindings to the same headers (src/pvr/).
 */
#ifndef PVR_RENDER_REGS_H
#define PVR_RENDER_REGS_H

#include <stdbool.h>
#include <stdint.h>

/* CR_EVENT_PIXEL_PDS_INFO for the pixel-event program of `num_rt` render
 * targets (pvr_pds_generate_pixel_event_tq): const_size its data segment in
 * the field's units, usc_sr_size the 2 * num_rt PBE state words in shared
 * registers, temp_stride 0 (Mesa pvr_arch_job_render.c,
 * pvr_frag_state_stream_init). */
uint32_t pvr_cr_event_pixel_pds_info(uint32_t num_rt);

/* CR_ISP_AA for `samples` per pixel (1: AA_NONE, 2: AA_2X, 4: AA_4X). */
uint32_t pvr_cr_isp_aa(uint32_t samples);

/* CR_ISP_CTL with the fields the self-test sets. */
uint32_t pvr_cr_isp_ctl(bool skip_init_hdrs, bool process_empty_tiles, bool sample_pos,
                        uint32_t pipe_enable);

/* CR_USC_PIXEL_OUTPUT_CTRL: `width_regs` output registers per pixel (1, 2,
 * 4 or 8), the partition mask for `partitions` partitions. */
uint32_t pvr_cr_usc_pixel_output_ctrl(uint32_t width_regs, uint32_t partitions);

/* CR_ISP_BGOBJVALS. */
uint32_t pvr_cr_isp_bgobjvals(bool enablebgtag, bool mask, uint32_t stencil);

/* The 64-bit PBESTATE_REG_WORD0/1 of a linear, tile-relative RGBA target
 * (identity swizzle) of `width` x `height` pixels and `stride_dwords` dwords
 * per row. */
void pvr_pbe_reg_words_linear_rgba(uint32_t width, uint32_t height, uint32_t stride_dwords,
                                   uint64_t out[2]);

/* CR_PDS_BGRND0_BASE, CR_PDS_BGRND1_BASE, CR_PDS_BGRND3_SIZEINFO: the
 * background object's PDS programs (PDS-heap offsets) and sizes in the
 * fields' units. */
void pvr_cr_pds_bgnd(uint32_t shader_addr, uint32_t texunicode_addr, uint32_t texturedata_addr,
                     uint32_t usc_sharedsize, uint32_t pds_texturestatesize,
                     uint32_t pds_tempsize, uint64_t out[3]);

#endif /* PVR_RENDER_REGS_H */
