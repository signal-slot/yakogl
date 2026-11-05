/*
 * SPDX-License-Identifier: MIT
 *
 * capture_spm_load.c -- print the machine code and the metadata of Mesa's
 * SPM (partial render) background-object load shader for the BXS-4-64
 * (BVNC 36.53.104.796, TI AM62P), as Mesa 26.2.1's pco compiles it.
 *
 * The shader is pvr_uscgen_spm_load() (src/imagination/vulkan/pvr_usc.c):
 * the resumed render of a partial render runs it as the background object
 * so that every pixel's output registers start as the tile the partial
 * render stored to the SPM scratch buffer. It samples the scratch through
 * a texture (point sampled, non-normalised coordinates = the pixel's
 * position, lod 0) and writes the raw dwords to the output registers.
 * Mesa generates it at device creation (pvr_arch_spm.c
 * pvr_arch_device_init_spm_load_state); we have no pco on the target, so
 * it is generated once here and committed (tools/pco_shc/spm_load/).
 *
 * Built inside a Mesa tree (tools/pco_shc/README.md); run by
 * tools/pco_shc/gen_spm_load.py capture.
 *
 * usage:
 *   capture_spm_load <output_reg_count> <tile_buffer_count> <is_multisampled>
 *       struct pvr_spm_load_props (pvr_usc.h): output_reg_count 1, 2 or 4
 *       (dwords of the pixel's colour), tile_buffer_count 0..7,
 *       is_multisampled 0 or 1.
 *
 * Output (stdout), one "key value" line each, then the words:
 *   props, index, consts_dwords, smp_state_sh, tex_state_sh, texstate_dwords,
 *   usc_bytes, entry_offset, temps, shareds, coeffs, vtxins, interns, uses,
 *   fs_out, fs_in, words.
 * The shared-register slots are enum pvr_spm_load_data (common/pvr_iface.h):
 * the shader reads the sampler state from sh[smp_state_sh ..+texstate_dwords)
 * and the scratch texture's state from sh[tex_state_sh ..+texstate_dwords);
 * the PDS program that kicks it DMAs consts_dwords dwords there
 * (pvr_arch_spm.c pvr_arch_spm_init_bgobj_state).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/glsl_types.h"
#include "compiler/nir/nir.h"
#include "util/format/u_format.h"
#include "util/ralloc.h"

#include "hwdef/rogue_hw_defs.h"
#include "pco/pco.h"
#include "pco/pco_data.h"
#include "pvr_usc.h"
#include "pvr_device_info.h"
#include "device_info/bxs-4-64.h"
#include "pco_uscgen_programs.h"

static void range(const char *key, const char *reg, pco_range r)
{
   printf("%s %s.start=%u count=%u\n", key, reg, r.start, r.count);
}

int main(int argc, char **argv)
{
   if (argc != 4) {
      fprintf(stderr,
              "usage: capture_spm_load <output_reg_count> <tile_buffer_count> "
              "<is_multisampled>\n");
      return 2;
   }

   struct pvr_spm_load_props props = {
      .output_reg_count = (unsigned)strtoul(argv[1], NULL, 0),
      .tile_buffer_count = (unsigned)strtoul(argv[2], NULL, 0),
      .is_multisampled = strtoul(argv[3], NULL, 0) != 0,
   };

   if ((props.output_reg_count != 1 && props.output_reg_count != 2 &&
        props.output_reg_count != 4) ||
       props.tile_buffer_count > 7 ||
       (props.tile_buffer_count > 0 && props.output_reg_count != 4)) {
      fprintf(stderr, "capture_spm_load: no such SPM load program "
                      "(pvr_usc.h pvr_uscgen_spm_load_index)\n");
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

   printf("props output_reg_count=%u tile_buffer_count=%u is_multisampled=%u\n",
          props.output_reg_count, props.tile_buffer_count,
          props.is_multisampled ? 1 : 0);
   printf("index %u\n", pvr_uscgen_spm_load_index(&props));
   printf("consts_dwords %u\n", pvr_uscgen_spm_load_data_size(&props));
   printf("smp_state_sh %u\n", PVR_SPM_LOAD_DATA_SMP);
   printf("tex_state_sh %u\n", PVR_SPM_LOAD_DATA_REG_TEX);
   printf("texstate_dwords %u\n", ROGUE_NUM_TEXSTATE_DWORDS);
   printf("usc_bytes %u\n", len);
   printf("entry_offset %u\n", d->common.entry_offset);
   printf("temps %u\n", d->common.temps);
   printf("shareds %u\n", d->common.shareds);
   printf("coeffs %u\n", d->common.coeffs);
   printf("vtxins %u\n", d->common.vtxins);
   printf("interns %u\n", d->common.interns);
   printf("uses sample_shading=%d z=%d w=%d pntc=%d discard=%d usclib=%d "
          "point_sampler=%d\n",
          d->fs.uses.sample_shading, d->fs.uses.z, d->fs.uses.w,
          d->fs.uses.pntc, d->fs.uses.discard, d->common.uses.usclib,
          d->common.uses.point_sampler);
   for (unsigned i = 0; i < FRAG_RESULT_MAX; ++i) {
      if (!d->fs.outputs[i].count)
         continue;
      printf("fs_out res=%u po.start=%u count=%u fmt=%s\n", i,
             d->fs.outputs[i].start, d->fs.outputs[i].count,
             util_format_name(d->fs.output_formats[i]));
   }
   for (unsigned i = 0; i < VARYING_SLOT_MAX; ++i) {
      if (!d->fs.varyings[i].count)
         continue;
      printf("fs_in slot=%u coeff.start=%u count=%u\n", i,
             d->fs.varyings[i].start, d->fs.varyings[i].count);
   }
   if (d->fs.tile_buffers.count)
      range("tile_buffers", "sh", d->fs.tile_buffers);
   if (d->common.point_sampler.count)
      range("point_sampler", "sh", d->common.point_sampler);

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
