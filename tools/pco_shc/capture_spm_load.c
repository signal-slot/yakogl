/*
 * SPDX-License-Identifier: MIT
 *
 * capture_spm_load.c -- print the machine code and the register metadata of
 * Mesa's SPM background-load shader for the BXS-4-64 (BVNC 36.53.104.796,
 * TI AM62P), as Mesa 26.2.1's pco compiles it. Built inside a Mesa tree
 * (tools/pco_shc/README.md); run by tools/pco_shc/gen_spm_load.py, which
 * turns its output into tools/pco_shc/spm_load/ (the committed words and
 * their metadata).
 *
 * The shader is pvr_uscgen_spm_load() (src/imagination/vulkan/pvr_usc.c):
 * the fragment program the resumed render of a partial render runs as its
 * background object, which samples the tiles the PR's end-of-tile program
 * stored to the SPM scratch back into the output registers. Mesa builds it
 * at device init for every configuration (pvr_arch_spm.c
 * pvr_arch_device_init_spm_load_state); the driver has no pco on the
 * target, so this prints the one configuration it needs.
 *
 * usage:
 *   capture_spm_load <output_reg_count> <tile_buffer_count> <is_multisampled>
 *       pvr_uscgen_spm_load() with struct pvr_spm_load_props { output regs,
 *       tile buffers, ms }: 1 0 0 is one output dword (RGBA8), no tile
 *       buffer, one sample (pvr_uscgen_spm_load_index() 0).
 *
 * Output (stdout), one "key value" line each, then "words <w0> <w1> ..." in
 * hex (gen_spm_load.py documents the keys in the file it writes).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/glsl_types.h"
#include "compiler/nir/nir.h"
#include "compiler/shader_enums.h"
#include "util/format/u_format.h"
#include "util/ralloc.h"

#include "hwdef/rogue_hw_defs.h"
#include "pco/pco.h"
#include "pco/pco_data.h"
#include "pvr_iface.h"
#include "pvr_usc.h"
#include "pvr_device_info.h"
#include "device_info/bxs-4-64.h"
#include "pco_uscgen_programs.h"

int main(int argc, char **argv)
{
   if (argc != 4) {
      fprintf(stderr, "usage: capture_spm_load <output_reg_count> <tile_buffer_count> <is_multisampled>\n");
      return 2;
   }
   struct pvr_spm_load_props props = {
      .output_reg_count = (unsigned)strtoul(argv[1], NULL, 0),
      .tile_buffer_count = (unsigned)strtoul(argv[2], NULL, 0),
      .is_multisampled = strtoul(argv[3], NULL, 0) != 0,
   };
   if (props.output_reg_count != 1 && props.output_reg_count != 2 && props.output_reg_count != 4) {
      fprintf(stderr, "capture_spm_load: output_reg_count must be 1, 2 or 4\n");
      return 2;
   }
   if (props.tile_buffer_count > 7 || (props.tile_buffer_count && props.output_reg_count != 4)) {
      fprintf(stderr, "capture_spm_load: tile buffers (up to 7) only with 4 output regs (pvr_uscgen_spm_load_index)\n");
      return 2;
   }

   void *mem_ctx = ralloc_context(NULL);
   const struct pvr_device_info *dev_info = &pvr_device_info_36_53_104_796;
   struct pvr_device_runtime_info rt_info = { 0 };
   pco_ctx *ctx = pco_ctx_create(dev_info, &rt_info, mem_ctx);

   pco_ctx_setup_usclib(ctx, pco_usclib_0_nir, sizeof(pco_usclib_0_nir));
   glsl_type_singleton_init_or_ref();

   pco_shader *sh = pvr_uscgen_spm_load(ctx, &props);
   const uint8_t *bin = pco_shader_binary_data(sh);
   unsigned len = pco_shader_binary_size(sh);
   const pco_data *d = pco_shader_data(sh);
   const pco_range *pos = &d->fs.varyings[VARYING_SLOT_POS];
   const pco_range *out = &d->fs.outputs[FRAG_RESULT_DATA0];

   printf("mesa 26.2.1\n");
   printf("generator pvr_uscgen_spm_load\n");
   printf("device BXS-4-64 36.53.104.796\n");
   printf("output_reg_count %u\n", props.output_reg_count);
   printf("tile_buffer_count %u\n", props.tile_buffer_count);
   printf("is_multisampled %u\n", props.is_multisampled ? 1 : 0);
   printf("index %u\n", pvr_uscgen_spm_load_index(&props));
   printf("size_bytes %u\n", len);
   printf("entry_offset %u\n", d->common.entry_offset);
   printf("temps %u\n", d->common.temps);
   printf("shareds %u\n", d->common.shareds);
   printf("coeffs %u\n", d->common.coeffs);
   printf("vtxins %u\n", d->common.vtxins);
   printf("interns %u\n", d->common.interns);
   printf("spilled_temps %u\n", d->common.spilled_temps);
   printf("scratch %u\n", d->common.scratch);
   /* The shared registers the shader reads, as pvr_uscgen_spm_load() lays
    * them out (common/pvr_iface.h enum pvr_spm_load_data): the sampler state
    * at PVR_SPM_LOAD_DATA_SMP and the image state of the output-register
    * scratch at PVR_SPM_LOAD_DATA_REG_TEX, ROGUE_NUM_TEXSTATE_DWORDS each;
    * then per tile buffer its image state and its address (lo, hi). */
   printf("data_dwords %u\n", pvr_uscgen_spm_load_data_size(&props));
   printf("smp_state sh%u count %u\n", PVR_SPM_LOAD_DATA_SMP, ROGUE_NUM_TEXSTATE_DWORDS);
   printf("tex_state sh%u count %u\n", PVR_SPM_LOAD_DATA_REG_TEX, ROGUE_NUM_TEXSTATE_DWORDS);
   for (unsigned b = 0; b < props.tile_buffer_count; b++)
      printf("tile_buffer%u tex_state sh%u count %u addr sh%u count 2\n", b,
             pvr_uscgen_spm_buffer_data(b, false), ROGUE_NUM_TEXSTATE_DWORDS,
             pvr_uscgen_spm_buffer_data(b, true));
   printf("pos_coeffs cf%u count %u\n", pos->start, pos->count);
   printf("uses_z %u\n", d->fs.uses.z ? 1 : 0);
   printf("uses_w %u\n", d->fs.uses.w ? 1 : 0);
   printf("sample_shading %u\n", d->fs.uses.sample_shading ? 1 : 0);
   printf("point_sampler %u\n", d->common.uses.point_sampler ? 1 : 0);
   printf("usclib %u\n", d->common.uses.usclib ? 1 : 0);
   printf("output po%u count %u format %s\n", out->start, out->count,
          util_format_short_name(d->fs.output_formats[FRAG_RESULT_DATA0]));
   printf("words");
   for (unsigned i = 0; i + 4 <= len; i += 4) {
      uint32_t dw;
      memcpy(&dw, bin + i, 4);
      printf(" 0x%08x", dw);
   }
   printf("\n");
   ralloc_free(mem_ctx);
   return 0;
}
