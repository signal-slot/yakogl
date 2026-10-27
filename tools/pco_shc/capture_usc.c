/*
 * SPDX-License-Identifier: MIT
 *
 * capture_usc.c -- print the machine code of one of Mesa's own USC programs
 * for the BXS-4-64 (BVNC 36.53.104.796), as Mesa 26.2.1's pco compiles it.
 * Built inside a Mesa tree (tools/pco_shc/README.md); run by
 * tools/pco_shc/gen_usc_blobs.py, which turns its output into the driver's
 * pvr_usc_*_blob.h.
 *
 * usage:
 *   capture_usc eot <emit_count> [<tb_emit> ...]
 *       pvr_usc_eot() with the transfer context's props
 *       (pvr_arch_job_context.c pvr_transfer_eot_shaders_init): shared_words,
 *       state_regs[i] = 2i. Each <tb_emit> names an emit whose data comes
 *       from a tile buffer (tile_buffer_addrs[tb_emit] = the sentinel
 *       0x0000beefe0770000, num_output_regs 4, msaa_samples 1), which the
 *       driver patches with the real address at upload.
 *   capture_usc clear <dword_count>
 *       pvr_uscgen_clear_attach() with { dword_count, offset 0, no tile
 *       buffer } (pvr_arch_device.c's clear-attach table).
 *   capture_usc precomp <index>
 *       pco_usclib_common[index], a program Mesa precompiles from usclib
 *       (pco_uscgen_programs.h names the indices).
 *
 * Output (stdout): "temps <n>" then "words <w0> <w1> ..." in hex.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/glsl_types.h"
#include "compiler/nir/nir.h"
#include "util/ralloc.h"

#include "pco/pco.h"
#include "pco/pco_data.h"
#include "pvr_usc.h"
#include "pvr_device_info.h"
#include "device_info/bxs-4-64.h"
#include "pco_uscgen_programs.h"

#define TB_SENTINEL UINT64_C(0x0000beefe0770000)

static void print(const uint8_t *bin, unsigned len, unsigned temps)
{
   printf("temps %u\nwords", temps);
   for (unsigned i = 0; i + 4 <= len; i += 4) {
      uint32_t dw;
      memcpy(&dw, bin + i, 4);
      printf(" 0x%08x", dw);
   }
   printf("\n");
}

int main(int argc, char **argv)
{
   if (argc < 3) {
      fprintf(stderr, "usage: capture_usc eot|clear|precomp <n> [...]\n");
      return 2;
   }
   void *mem_ctx = ralloc_context(NULL);
   const struct pvr_device_info *dev_info = &pvr_device_info_36_53_104_796;
   struct pvr_device_runtime_info rt_info = { 0 };
   pco_ctx *ctx = pco_ctx_create(dev_info, &rt_info, mem_ctx);

   pco_ctx_setup_usclib(ctx, pco_usclib_0_nir, sizeof(pco_usclib_0_nir));
   glsl_type_singleton_init_or_ref();

   unsigned n = (unsigned)strtoul(argv[2], NULL, 0);
   pco_shader *sh = NULL;

   if (!strcmp(argv[1], "eot")) {
      static unsigned regs[PVR_MAX_COLOR_ATTACHMENTS];
      struct pvr_eot_props props = {
         .emit_count = n,
         .shared_words = true,
         .state_regs = regs,
      };

      for (unsigned i = 0; i < PVR_MAX_COLOR_ATTACHMENTS; i++)
         regs[i] = i * 2;
      for (int a = 3; a < argc; a++) {
         unsigned t = (unsigned)strtoul(argv[a], NULL, 0);

         props.tile_buffer_addrs[t] = TB_SENTINEL;
         props.num_output_regs = 4;
         props.msaa_samples = 1;
      }
      sh = pvr_usc_eot(ctx, &props, dev_info);
   } else if (!strcmp(argv[1], "clear")) {
      struct pvr_clear_attach_props props = {
         .dword_count = n,
         .offset = 0,
         .uses_tile_buffer = false,
      };

      sh = pvr_uscgen_clear_attach(ctx, &props);
   } else if (!strcmp(argv[1], "precomp")) {
      const pco_precomp_data *p = (const pco_precomp_data *)pco_usclib_common[n];

      print((const uint8_t *)p->binary, p->size_dwords * 4, p->temps);
      ralloc_free(mem_ctx);
      return 0;
   } else {
      fprintf(stderr, "capture_usc: unknown program '%s'\n", argv[1]);
      return 2;
   }

   print(pco_shader_binary_data(sh), pco_shader_binary_size(sh),
         pco_shader_data(sh)->common.temps);
   ralloc_free(mem_ctx);
   return 0;
}
