/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: Imagination Technologies Ltd.
 *
 * capture_smp_fs.c -- capture a minimal USC texture-SAMPLE fragment shader
 * for BXS-4-64 (BVNC 36.53.104.796, TI AM62P).
 *
 * Builds, via the real PCO backend, a fragment shader that:
 *   - reads an iterated 2D UV varying (VARYING_SLOT_VAR0.xy),
 *   - loads combined image/sampler state from shared regs
 *       tex_state = sh[0..3], smp_state = sh[4..7],
 *   - issues ONE SMP: dim=2D, fcnorm=1 (UNORM->float), nncoords=0
 *     (normalized coords), lod_mode=REPLACE with lod=0 (no gradients),
 *   - writes the resulting RGBA (4 components) to pixel output 0.
 *
 * This mirrors pvr_uscgen_tq()'s iterated-blit structure but forces the exact
 * SMP semantics requested (float result + explicit LOD replace 0), by calling
 * pco_emit_nir_smp() directly.
 */

#include <stdio.h>
#include <stdint.h>
#include <string.h>

#include "hwdef/rogue_hw_utils.h"
#include "compiler/nir/nir.h"
#include "compiler/nir/nir_builder.h"
#include "compiler/glsl_types.h"
#include "util/ralloc.h"

#include "pco/pco.h"
#include "pco/pco_data.h"
#include "pvr_device_info.h"
#include "device_info/bxs-4-64.h"   /* pvr_device_info_36_53_104_796 */

static pco_shader *build_shader(pco_ctx *ctx, nir_shader *nir, pco_data *data)
{
   pco_preprocess_nir(ctx, nir);
   pco_lower_nir(ctx, nir, data);
   pco_postprocess_nir(ctx, nir, data);

   pco_shader *shader = pco_trans_nir(ctx, nir, data, NULL);
   ralloc_steal(shader, nir);
   pco_process_ir(ctx, shader);
   pco_encode_ir(ctx, shader);

   return shader;
}

int main(void)
{
   void *mem_ctx = ralloc_context(NULL);

   const struct pvr_device_info *dev_info = &pvr_device_info_36_53_104_796;
   struct pvr_device_runtime_info rt_info = { 0 };
   pco_ctx *ctx = pco_ctx_create(dev_info, &rt_info, mem_ctx);

   nir_builder b = nir_builder_init_simple_shader(MESA_SHADER_FRAGMENT,
                                                  pco_nir_options(),
                                                  "smp_tex_fs");
   pco_data data = { 0 };

   /* Iterated 2D UV varying: VARYING_SLOT_VAR0.xy. */
   const struct glsl_type *var_type = glsl_vec_type(2);
   nir_variable *pos = nir_get_variable_with_location(b.shader,
                                                      nir_var_shader_in,
                                                      VARYING_SLOT_VAR0,
                                                      var_type);
   nir_def *coords_var = nir_load_var(&b, pos);
   nir_def *coords = nir_channels(&b, coords_var, nir_component_mask(2));

   /* Combined image/sampler state from shared regs (driver DMAs these to the
    * common store: image at cs dwords 0..3, sampler at cs dwords 4..7). */
   nir_def *tex_state = nir_load_preamble(&b, 4, 32, .base = 0);
   nir_def *smp_state = nir_load_preamble(&b, 4, 32, .base = 4);

   pco_smp_params params = {
      .tex_state = tex_state,
      .smp_state = smp_state,
      .dest_type = nir_type_float32,          /* fcnorm = 1 (UNORM->float) */
      .sampler_dim = GLSL_SAMPLER_DIM_2D,     /* dim = 2D */
      .nncoords = false,                      /* normalized 0..1 coords */
      .coords = coords,
      .lod_replace = nir_imm_float(&b, 0.0f), /* lod_mode = REPLACE, lod = 0 */
      .sample_components = 4,                  /* RGBA */
   };

   nir_intrinsic_instr *smp = pco_emit_nir_smp(&b, &params);
   nir_def *rgba = &smp->def;

   /* RGBA -> single packed pixel-output register po0 (RGBA8 UNORM target),
    * matching the single-po0 emit path of the existing interpolation FS.
    * Override via SMP_FS_OUT=f32x4 to emit 4x f32 to po0..3 instead. */
   const char *outsel = getenv("SMP_FS_OUT");
   bool out_f32x4 = outsel && !strcmp(outsel, "f32x4");

   if (out_f32x4) {
      data.fs.output_formats[FRAG_RESULT_DATA0] = PIPE_FORMAT_R32G32B32A32_FLOAT;
      data.fs.outputs[FRAG_RESULT_DATA0] = (pco_range){ .start = 0, .count = 4 };
      nir_store_output(&b, rgba, nir_imm_int(&b, 0),
                       .base = 0, .src_type = nir_type_float32,
                       .write_mask = BITFIELD_MASK(4),
                       .io_semantics.location = FRAG_RESULT_DATA0,
                       .io_semantics.num_slots = 1);
   } else {
      data.fs.output_formats[FRAG_RESULT_DATA0] = PIPE_FORMAT_R8G8B8A8_UNORM;
      data.fs.outputs[FRAG_RESULT_DATA0] = (pco_range){ .start = 0, .count = 1 };
      nir_store_output(&b, rgba, nir_imm_int(&b, 0),
                       .base = 0, .src_type = nir_type_float32,
                       .write_mask = BITFIELD_MASK(4),
                       .io_semantics.location = FRAG_RESULT_DATA0,
                       .io_semantics.num_slots = 1);
   }
   b.shader->info.outputs_written |= BITFIELD64_BIT(FRAG_RESULT_DATA0);

   nir_variable *var0 = nir_find_variable_with_location(b.shader,
                                                        nir_var_shader_in,
                                                        VARYING_SLOT_VAR0);
   if (var0) {
      var0->data.interpolation = INTERP_MODE_NOPERSPECTIVE;
      data.fs.varyings[VARYING_SLOT_VAR0] = (pco_range){
         .start = 0,
         .count = glsl_count_dword_slots(var0->type, false) *
                  ROGUE_USC_COEFFICIENT_SET_SIZE,
      };
   }

   nir_create_variable_with_location(b.shader,
                                     nir_var_shader_out,
                                     FRAG_RESULT_DATA0,
                                     glsl_vec_type(4));

   nir_jump(&b, nir_jump_return);

   pco_shader *fs = build_shader(ctx, b.shader, &data);

   const uint8_t *bin = pco_shader_binary_data(fs);
   unsigned       len = pco_shader_binary_size(fs);
   pco_data      *d   = pco_shader_data(fs);

   fprintf(stderr, "SMP_FS USC: %u bytes, temps=%u\n", len, d->common.temps);

   fwrite(bin, 1, len, stdout);

   fprintf(stderr, "dwords:");
   for (unsigned i = 0; i + 4 <= len; i += 4) {
      uint32_t dw;
      memcpy(&dw, bin + i, 4);
      fprintf(stderr, " 0x%08x", dw);
   }
   fprintf(stderr, "\n");

   ralloc_free(mem_ctx);
   return 0;
}
