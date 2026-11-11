/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The vertex PDS program with a per-draw gl_InstanceID base
 * (pvr_pds_render_vertex_program_ids_base, Mesa's instance_id_modifier,
 * docs/notes/instanceid-per-draw.md fix P1); built and run by
 * tests/es3_pvr_instanceid_pds.rs. Prints
 * "base0_same=1 modifier_in_data=1 grow=<data>,<code> no_reg_ignored=1".
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

#include "pvr_mesa.h"
#include "pvr_pds_render_gen.h"

#define N 64u
#define BASE 0x12345u

struct prog {
   uint32_t data[N], code[N];
   struct pvr_pds_render_vertex_info info;
};

static int has(const struct prog *p, uint32_t w)
{
   for (uint32_t i = 0; i < p->info.data_size_dw; i++)
      if (p->data[i] == w)
         return 1;
   return 0;
}

static int same(const struct prog *a, const struct prog *b)
{
   return a->info.data_size_dw == b->info.data_size_dw &&
          a->info.code_size_dw == b->info.code_size_dw &&
          !memcmp(a->data, b->data, a->info.data_size_dw * 4) &&
          !memcmp(a->code, b->code, a->info.code_size_dw * 4);
}

int main(void)
{
   const struct pvr_pds_render_stream st[2] = {
      { .address = 0x100000, .stride = 16, .dma_size = 16, .dst_reg = 0 },
      { .address = 0x200000, .stride = 8, .dma_size = 8, .dst_reg = 4, .per_instance = 1 },
   };
   struct prog old, b0, bn, noreg_old, noreg_bn;
   int ok = 1;

   memset(&old, 0, sizeof(old));
   b0 = bn = noreg_old = noreg_bn = old;
   ok &= !pvr_pds_render_vertex_program_ids(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG, 6, old.data,
                                            old.code, &old.info);
   ok &= !pvr_pds_render_vertex_program_ids_base(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG, 6, 0,
                                                 b0.data, b0.code, &b0.info);
   ok &= !pvr_pds_render_vertex_program_ids_base(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG, 6, BASE,
                                                 bn.data, bn.code, &bn.info);
   ok &= !pvr_pds_render_vertex_program_ids(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG,
                                            PVR_PDS_NO_ID_REG, noreg_old.data, noreg_old.code,
                                            &noreg_old.info);
   ok &= !pvr_pds_render_vertex_program_ids_base(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG,
                                                 PVR_PDS_NO_ID_REG, BASE, noreg_bn.data,
                                                 noreg_bn.code, &noreg_bn.info);
   if (!ok) {
      printf("generator failed\n");
      return 1;
   }
   printf("base0_same=%d modifier_in_data=%d grow=%u,%u no_reg_ignored=%d\n", same(&old, &b0),
          has(&bn, BASE) && !has(&old, BASE),
          bn.info.data_size_dw - old.info.data_size_dw,
          bn.info.code_size_dw - old.info.code_size_dw, same(&noreg_old, &noreg_bn));
   return 0;
}
