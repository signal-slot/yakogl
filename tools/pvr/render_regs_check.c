/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * render_regs_check.c -- the register words driver/src/pvr_render_regs.c
 * packs with Mesa's csbgen, against the literals the driver carried before
 * (pvr_render_regs.h's PVR_R_* values, and the self-test rig's words in
 * pvr_fw_boot.c, which came from the DDK capture), so that the swap to
 * csbgen is byte-for-byte the same programme for the GPU.
 *
 *   cc -std=c11 -Idriver/src <pvr_mesa include dirs> tools/pvr/render_regs_check.c \
 *      driver/src/pvr_render_regs.c driver/compat/pvr_mesa_pds.c
 */
#include <stdio.h>
#include <stdint.h>

#include "pvr_render_regs.h"

static unsigned checks, failed;

static void expect(const char *what, uint64_t got, uint64_t want)
{
   checks++;
   if (got != want) {
      failed++;
      printf("FAIL %s: 0x%016llx, want 0x%016llx\n", what, (unsigned long long)got,
             (unsigned long long)want);
   }
}

int main(void)
{
   enum { PLANNED = 15 };
   uint64_t pbe[2], bg[3];

   /* PVR_R_EVENT_PIXEL_PDS_INFO (const_size 2, usc_sr_size 1), and the
    * render service's three-target word ((3 * 3 + 1 + 3) / 4 | 1 << 9). */
   expect("CR_EVENT_PIXEL_PDS_INFO 1 RT", pvr_cr_event_pixel_pds_info(1), 0x00000202u);
   expect("CR_EVENT_PIXEL_PDS_INFO 2 RT", pvr_cr_event_pixel_pds_info(2), 0x00000202u);
   expect("CR_EVENT_PIXEL_PDS_INFO 3 RT", pvr_cr_event_pixel_pds_info(3), 0x00000203u);
   /* PVR_R_ISP_AA */
   expect("CR_ISP_AA 1x", pvr_cr_isp_aa(1), 0);
   expect("CR_ISP_AA 4x", pvr_cr_isp_aa(4), 2);
   /* the self-test's SKIP_INIT_HDRS | PROCESS_EMPTY | SAMPLE_POS | PIPE(5) */
   expect("CR_ISP_CTL", pvr_cr_isp_ctl(true, true, true, 5), 0x80035000u);
   expect("CR_ISP_CTL 4x", pvr_cr_isp_ctl(true, true, true, 3), 0x80033000u);
   /* the self-test's (and backend's) 0x0007fff9: four registers, sixteen
    * partitions */
   expect("CR_USC_PIXEL_OUTPUT_CTRL", pvr_cr_usc_pixel_output_ctrl(4, 16), 0x0007fff9u);
   /* cr.xml ISP_BGOBJVALS.mask */
   expect("CR_ISP_BGOBJVALS", pvr_cr_isp_bgobjvals(false, true, 0), 0x00000100u);
   /* the self-test's PBE words for a 64 x 64 target, 64 dwords a row */
   pvr_pbe_reg_words_linear_rgba(64, 64, 64, pbe);
   expect("PBESTATE_REG_WORD0", pbe[0], (1ull << 57) | ((uint64_t)((64 - 1) / 2) << 34) | 0x68800000ull);
   expect("PBESTATE_REG_WORD1", pbe[1], (uint64_t)(64 - 1) | ((uint64_t)(64 - 1) << 32));
   pvr_pbe_reg_words_linear_rgba(1024, 600, 1024, pbe);
   expect("PBESTATE_REG_WORD0 1024", pbe[0], (1ull << 57) | ((uint64_t)((1024 - 1) / 2) << 34) | 0x68800000ull);
   /* the self-test's background PDS words */
   pvr_cr_pds_bgnd(0x1230, 0x4560, 0x7890, 1, 3, 1, bg);
   expect("CR_PDS_BGRND0_BASE", bg[0], 0x1230ull | (0x4560ull << 32));
   expect("CR_PDS_BGRND1_BASE", bg[1], 0x7890ull << 32);
   expect("CR_PDS_BGRND3_SIZEINFO", bg[2], (1ull << 55) | (3ull << 16) | 1ull);

   printf("render_regs_check: %u checks run (planned %u), %u failed\n", checks, PLANNED, failed);
   return failed != 0 || checks != PLANNED;
}
