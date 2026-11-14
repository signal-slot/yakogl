/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The vertex PDS program with a per-stream instance divisor
 * (struct pvr_pds_render_stream.divisor, Mesa's multiplier/shift,
 * pvr_pipeline_pds.c:846-864); built and run by tests/es3_pvr_divisor_pds.rs.
 * Prints one line per divisor:
 *   d=<divisor> mult=<hex> dgrow=<n> cgrow=<n> same=<0|1> div_ok=<0|1>
 * where mult is the multiplier data word of stream 1 (its block starts at
 * dword 8, the multiplier is the constant after the stride), dgrow/cgrow the
 * data and code growth over divisor 0, same whether the words equal divisor
 * 0's, and div_ok whether (i * mult) >> (shift + 31) equals i / d for the
 * instance numbers 0..99999 and a sample of the range up to 2^32 - 1.
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

#include "pvr_mesa.h"
#include "pvr_pds_render_gen.h"

#define N 64u

struct prog {
   uint32_t data[N], code[N];
   struct pvr_pds_render_vertex_info info;
};

static int gen(struct prog *p, uint32_t divisor, uint32_t per_instance)
{
   const struct pvr_pds_render_stream st[2] = {
      { .address = 0x100000, .stride = 16, .dma_size = 16, .dst_reg = 0 },
      { .address = 0x200000, .stride = 8, .dma_size = 8, .dst_reg = 4,
        .per_instance = per_instance, .divisor = divisor },
   };

   memset(p, 0, sizeof(*p));
   return pvr_pds_render_vertex_program_ids(0x400, 2, st, 2, 0, PVR_PDS_NO_ID_REG, 6, p->data,
                                            p->code, &p->info);
}

static int same(const struct prog *a, const struct prog *b)
{
   return a->info.data_size_dw == b->info.data_size_dw &&
          a->info.code_size_dw == b->info.code_size_dw &&
          !memcmp(a->data, b->data, a->info.data_size_dw * 4) &&
          !memcmp(a->code, b->code, a->info.code_size_dw * 4);
}

static int model_ok(uint32_t d, uint32_t mult, uint32_t shift)
{
   for (uint64_t i = 0; i < 100000u + 70000u; i++) {
      uint64_t n = i < 100000u ? i : 0xffffffffull - (i - 100000u) * 61000u;

      if (((n * mult) >> (shift + 31u)) != n / d)
         return 0;
   }
   return 1;
}

int main(void)
{
   static const uint32_t divs[] = { 0, 1, 2, 3, 4, 0xffff };
   struct prog base, p, q;

   if (gen(&base, 0, 1)) {
      printf("generator failed\n");
      return 1;
   }
   for (unsigned k = 0; k < sizeof(divs) / sizeof(divs[0]); k++) {
      uint32_t d = divs[k], mult = 0, shift = 0;
      int div_ok = 1;

      if (gen(&p, d, 1)) {
         printf("generator failed\n");
         return 1;
      }
      if (d > 1 && (d & (d - 1))) {
         shift = (31u - (uint32_t)__builtin_clz(d - 1u)) + 1u;
         mult = p.data[8 + 3];
         div_ok = model_ok(d, mult, shift);
      }
      printf("d=%u mult=0x%x dgrow=%d cgrow=%d same=%d div_ok=%d\n", d, mult,
             (int)p.info.data_size_dw - (int)base.info.data_size_dw,
             (int)p.info.code_size_dw - (int)base.info.code_size_dw, same(&base, &p), div_ok);
   }
   /* The divisor of a stream that is not per-instance is ignored. */
   if (gen(&p, 0xffff, 0) || gen(&q, 0, 0)) {
      printf("generator failed\n");
      return 1;
   }
   printf("not_per_instance_ignored=%d\n", same(&p, &q));
   return 0;
}
