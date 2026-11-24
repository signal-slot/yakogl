/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The vertex PDS program with many per-instance streams, each with its own
 * divisor (struct pvr_pds_render_stream.divisor, Mesa's multiplier/shift,
 * pvr_pipeline_pds.c:846-864); built and run by
 * tests/es3_pvr_divisor_pds_many.rs, one process per case so that an assert
 * of Mesa's generator (built without NDEBUG) shows as an abort of that case
 * only.
 *
 *   es3_pvr_divisor_pds_many <set> <n>
 *
 * set "mixed": the divisors 3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16, 19,
 * 23, 64, 29, 0x8000 (primes, powers of two and 0xffff); set "odd": 16
 * divisors that are none of them powers of two (every stream takes Mesa's
 * multiply, two temps each). The first n of the set are used, stream s
 * reading 16 bytes with stride 16 + 4 * s into vtxin 4 * s. The program is
 * generated the way src/pvr/backend.rs does for a draw that writes both
 * gl_VertexID and gl_InstanceID and has a base instance with
 * PVR_PDS_ID_BASE_TO_STREAMS (its largest program for that many streams).
 *
 * Prints
 *   sizes data=<dw> code=<dw> temps=<n> want_temps=<n>
 *   s=<s> d=<divisor> mult=0x<data word 8s+3> stride_ok=<0|1> code_ok=<0|1>
 * where stride_ok says the stride sits at data word 8s+2, and code_ok that
 * the code segment holds the stream's instance divide exactly as Mesa's
 * pvr_pds_vertex_shader emits it (two LIMM, the MAD with the multiplier
 * constant 8s+3 and two SFTLP64 by 31 and by shift; or one SFTLP32 by
 * log2(d) for a power of two), on the temps Mesa's allocator gives it
 * (pvr_pds_get_temps: IR0 128, IR1 129, then each stream in order, two
 * temps 64-bit aligned for a multiply, one for a shift). want_temps is that
 * allocator's count.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "pvr_mesa.h"
#include "pvr_pds_render_gen.h"

#define BUF_DW 512u
#define IR0 128u
#define IR1 129u

static const uint32_t mixed[16] = { 3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16, 19, 23, 64, 29, 0x8000 };
static const uint32_t odd[16] = { 3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 17, 19, 23, 0xfffe, 0xffff };

static int is_pow2(uint32_t d) { return (d & (d - 1)) == 0; }

static int has_seq(const uint32_t *code, uint32_t n, const uint32_t *seq, uint32_t m)
{
   for (uint32_t i = 0; i + m <= n; i++)
      if (!memcmp(&code[i], seq, m * 4))
         return 1;
   return 0;
}

int main(int argc, char **argv)
{
   static uint32_t data[BUF_DW], code[BUF_DW];
   struct pvr_pds_render_stream st[PVR_PDS_VERTEX_MAX_STREAMS];
   struct pvr_pds_render_vertex_info info;
   const uint32_t *divs;
   uint32_t n, next_temp = IR1 + 1, temps = 2, stream_temp[PVR_PDS_VERTEX_MAX_STREAMS];

   if (argc != 3 || (strcmp(argv[1], "mixed") && strcmp(argv[1], "odd"))) {
      fprintf(stderr, "usage: %s mixed|odd <n>\n", argv[0]);
      return 2;
   }
   divs = strcmp(argv[1], "mixed") ? odd : mixed;
   n = (uint32_t)atoi(argv[2]);
   if (n == 0 || n > PVR_PDS_VERTEX_MAX_STREAMS)
      return 2;

   memset(st, 0, sizeof(st));
   for (uint32_t s = 0; s < n; s++) {
      uint32_t need = is_pow2(divs[s]) ? 1u : 2u, t;

      st[s].address = 0x100000ull * (s + 1);
      st[s].stride = 16 + 4 * s;
      st[s].dma_size = 16;
      st[s].dst_reg = 4 * s;
      st[s].per_instance = 1;
      st[s].divisor = divs[s];
      /* pvr_pds_get_temps() */
      t = (need & 1) ? next_temp : ((next_temp + 1) & ~1u);
      temps += t + need - next_temp;
      next_temp = t + need;
      stream_temp[s] = t;
   }

   memset(data, 0, sizeof(data));
   memset(code, 0, sizeof(code));
   memset(&info, 0, sizeof(info));
   if (pvr_pds_render_vertex_program_ids_base(0x400, 8, st, n, 0, 4 * n,
                                              (4 * n + 1) | PVR_PDS_ID_BASE_TO_STREAMS, 5, data,
                                              code, &info)) {
      printf("generator refused\n");
      return 1;
   }
   printf("sizes data=%u code=%u temps=%u want_temps=%u\n", info.data_size_dw, info.code_size_dw,
          info.temps_used, temps);
   if (info.data_size_dw > BUF_DW || info.code_size_dw > BUF_DW)
      return 1;

   for (uint32_t s = 0; s < n; s++) {
      const uint32_t d = divs[s], t = stream_temp[s];
      uint32_t seq[5], m;

      if (is_pow2(d)) {
         const uint32_t shift = (uint32_t)__builtin_ctz(d);

         seq[0] = pvr_pds_inst_encode_sftlp32(1, 0, PVR_ROGUE_PDSINST_LOP_NONE, IR1, IR0,
                                              ~shift + 1u, t);
         m = 1;
      } else {
         const uint32_t shift = (31u - (uint32_t)__builtin_clz(d - 1u)) + 1u;
         const int32_t rest = -(int32_t)shift; /* -(shift + 31) + 31 */

         seq[0] = pvr_pds_inst_encode_limm(0, t, 0, 0);
         seq[1] = pvr_pds_inst_encode_limm(0, t + 1, 0, 0);
         seq[2] = pvr_rogue_inst_encode_mad(0, 0, 0, 8 * s + 3, IR1, t / 2, t / 2);
         seq[3] = pvr_pds_inst_encode_sftlp64(0, PVR_ROGUE_PDSINST_LOP_NONE, 1, t / 2, IR0,
                                              0xFFFE1, t / 2);
         seq[4] = pvr_pds_inst_encode_sftlp64(0, PVR_ROGUE_PDSINST_LOP_NONE, 1, t / 2, IR0,
                                              (uint32_t)rest, t / 2);
         m = 5;
      }
      printf("s=%u d=%u mult=0x%x stride_ok=%d code_ok=%d\n", s, d, data[8 * s + 3],
             data[8 * s + 2] == st[s].stride, has_seq(code, info.code_size_dw, seq, m));
   }
   return 0;
}
