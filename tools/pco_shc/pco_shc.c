/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: Imagination Technologies Ltd.
 *
 * pco_shc.c -- general offline GLSL/SPIR-V -> PowerVR Rogue USC compiler
 * for BXS-4-64 (BVNC 36.53.104.796, TI AM62P).
 *
 * Route (a): read a SPIR-V binary + a stage flag, run spirv_to_nir with the
 * production PVR/PCO spirv+nir options, replicate the exact driver
 * (pvr_arch_pipeline.c) shader-data allocation of attribs/varyings/outputs/
 * push-constants, then run the production PCO backend passes and dump the
 * USC machine code + all driver-integration metadata.
 *
 * Usage:  pco_shc <vs|fs> <in.spv>
 *   env  PCO_SHC_OUT=f32x4   emit 4x f32 to po0..3 instead of packed RGBA8 po0
 *   env  PCO_DEBUG_PRINT=fs,vs,internal,verbose   disassemble
 *
 * This mirrors, function-for-function, the graphics compile in
 * src/imagination/vulkan/pvr_arch_pipeline.c (the alloc helpers are copied
 * verbatim from there so the register layout is bit-identical to the real
 * Vulkan driver), but without needing VkGraphicsPipelineCreateInfo/renderpass/
 * layout: for our bare-metal target we only support attributes, inter-stage
 * varyings, push-constant uniforms and a single colour output -- exactly what
 * the hand-written driver drives with DOUTD (uniforms->sh) + DOUTI (varyings->cf).
 */

#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "hwdef/rogue_hw_utils.h"
#include "hwdef/rogue_hw_defs.h"
#include "compiler/nir/nir.h"
#include "compiler/nir/nir_builder.h"
#include "compiler/glsl_types.h"
#include "compiler/spirv/nir_spirv.h"
#include "util/ralloc.h"
#include "util/u_math.h"
#include "util/bitset.h"

#include "pco/pco.h"
#include "pco/pco_data.h"
#include "pco_uscgen_programs.h"
#include "pco/pco_common.h"
#include "pvr_device_info.h"
#include "pvr_iface.h"              /* PVR_PCK_INFO_* (image descriptor meta) */
#include "device_info/bxs-4-64.h"   /* pvr_device_info_36_53_104_796 */

#ifndef MAX_VARYING
#define MAX_VARYING 16
#endif

/* ---- allocator helpers, copied verbatim from pvr_arch_pipeline.c ---- */

static void set_var(pco_range *allocation_list, unsigned to,
                    nir_variable *var, unsigned dwords_each)
{
   unsigned slots = glsl_count_dword_slots(var->type, false);
   allocation_list[var->data.location] = (pco_range){
      .start = to, .count = slots * dwords_each,
   };
}

static void allocate_var(pco_range *allocation_list, unsigned *counter,
                         nir_variable *var, unsigned dwords_each)
{
   unsigned slots = glsl_count_dword_slots(var->type, false);
   allocation_list[var->data.location] = (pco_range){
      .start = *counter, .count = slots * dwords_each,
   };
   *counter += slots * dwords_each;
}

static void try_allocate_var(pco_range *allocation_list, unsigned *counter,
                             nir_shader *nir, uint64_t bitset,
                             nir_variable_mode mode, int location,
                             unsigned dwords_each)
{
   if (!(bitset & BITFIELD64_BIT(location)))
      return;
   nir_variable *var = nir_find_variable_with_location(nir, mode, location);
   assert(var);
   allocate_var(allocation_list, counter, var, dwords_each);
}

static void allocate_val(pco_range *allocation_list, unsigned *counter,
                         unsigned location, unsigned dwords_each)
{
   allocation_list[location] = (pco_range){
      .start = *counter, .count = dwords_each,
   };
   *counter += dwords_each;
}

static void try_allocate_vars(pco_range *allocation_list, unsigned *counter,
                              nir_shader *nir, uint64_t *bitset,
                              nir_variable_mode mode, bool any, bool f16,
                              enum glsl_interp_mode interp_mode,
                              unsigned dwords_each)
{
   uint64_t skipped = 0;
   while (*bitset) {
      int location = u_bit_scan64(bitset);
      unsigned accum = 0;
      nir_foreach_variable_with_modes (var, nir, mode) {
         if (var->data.location != location)
            continue;
         if (!any && (glsl_type_is_16bit(
                         glsl_without_array_or_matrix(var->type)) != f16 ||
                      var->data.interpolation != interp_mode)) {
            skipped |= BITFIELD64_BIT(location);
            break;
         }
         accum += glsl_count_dword_slots(var->type, false);
      }
      if (skipped & BITFIELD64_BIT(location))
         continue;
      accum *= dwords_each;
      allocate_val(allocation_list, counter, location, accum);
   }
   *bitset |= skipped;
}

/* ---- VS/FS data allocation, mirroring pvr_postprocess_shader_data ---- */

static void alloc_vs_attribs(pco_data *data, nir_shader *nir)
{
   nir_foreach_shader_in_variable (var, nir)
      allocate_var(data->vs.attribs, &data->common.vtxins, var, 1);
}

/* Vertex/instance IDs and the draw parameters the shader reads land in
 * vtxin registers after the attributes, as pvr_alloc_vs_sysvals does; the
 * vertex fetch (PDS) writes them there. */
static const gl_system_value vs_sysvals[] = {
   SYSTEM_VALUE_VERTEX_ID,     SYSTEM_VALUE_INSTANCE_ID,
   SYSTEM_VALUE_BASE_INSTANCE, SYSTEM_VALUE_BASE_VERTEX,
   SYSTEM_VALUE_DRAW_ID,       SYSTEM_VALUE_VIEW_INDEX,
};

static void alloc_vs_sysvals(pco_data *data, nir_shader *nir)
{
   for (unsigned u = 0; u < ARRAY_SIZE(vs_sysvals); ++u) {
      if (BITSET_TEST(nir->info.system_values_read, vs_sysvals[u])) {
         nir_intrinsic_op op = nir_intrinsic_from_system_value(vs_sysvals[u]);
         unsigned dwords = nir_intrinsic_infos[op].dest_components;
         allocate_val(data->common.sys_vals, &data->common.vtxins, vs_sysvals[u], dwords);
      }
   }
}

static void alloc_vs_varyings(pco_data *data, nir_shader *nir)
{
   uint64_t vars_mask = nir->info.outputs_written &
                        BITFIELD64_RANGE(VARYING_SLOT_VAR0, MAX_VARYING);

   try_allocate_var(data->vs.varyings, &data->vs.vtxouts, nir,
                    nir->info.outputs_written, nir_var_shader_out,
                    VARYING_SLOT_POS, 1);

   nir_foreach_variable_with_modes (var, nir, nir_var_shader_out) {
      if (var->data.location < VARYING_SLOT_VAR0 ||
          var->data.location >= VARYING_SLOT_VAR0 + MAX_VARYING)
         continue;
      bool f16 = glsl_type_is_16bit(glsl_without_array_or_matrix(var->type));
      unsigned components = glsl_get_components(var->type);
      switch (var->data.interpolation) {
      case INTERP_MODE_NONE:
         var->data.interpolation = INTERP_MODE_SMOOTH;
         FALLTHROUGH;
      case INTERP_MODE_SMOOTH:
         if (f16) data->vs.f16_smooth += components;
         else     data->vs.f32_smooth += components;
         break;
      case INTERP_MODE_FLAT:
         if (f16) data->vs.f16_flat += components;
         else     data->vs.f32_flat += components;
         break;
      case INTERP_MODE_NOPERSPECTIVE:
         if (f16) data->vs.f16_npc += components;
         else     data->vs.f32_npc += components;
         break;
      default: UNREACHABLE("");
      }
   }

   for (unsigned f16 = 0; f16 <= 1; ++f16)
      for (enum glsl_interp_mode im = INTERP_MODE_SMOOTH;
           im <= INTERP_MODE_NOPERSPECTIVE; ++im)
         try_allocate_vars(data->vs.varyings, &data->vs.vtxouts, nir,
                           &vars_mask, nir_var_shader_out, false, f16, im, 1);

   const gl_varying_slot last_slots[] = {
      VARYING_SLOT_PSIZ, VARYING_SLOT_VIEWPORT, VARYING_SLOT_LAYER,
      VARYING_SLOT_CLIP_DIST0, VARYING_SLOT_CLIP_DIST1,
   };
   for (unsigned u = 0; u < ARRAY_SIZE(last_slots); ++u)
      try_allocate_var(data->vs.varyings, &data->vs.vtxouts, nir,
                       nir->info.outputs_written, nir_var_shader_out,
                       last_slots[u], 1);
}

static void alloc_fs_varyings(pco_data *data, nir_shader *nir)
{
   uint64_t vars_mask = nir->info.inputs_read &
                        BITFIELD64_RANGE(VARYING_SLOT_VAR0, MAX_VARYING);
   assert(!data->common.coeffs);

   unsigned zw_count = !!data->fs.uses.z + !!data->fs.uses.w;
   allocate_val(data->fs.varyings, &data->common.coeffs, VARYING_SLOT_POS,
                zw_count * ROGUE_USC_COEFFICIENT_SET_SIZE);

   nir_variable *pntc = nir_find_variable_with_location(nir, nir_var_shader_in,
                                                        VARYING_SLOT_PNTC);
   if (pntc) {
      allocate_var(data->fs.varyings, &data->common.coeffs, pntc,
                   ROGUE_USC_COEFFICIENT_SET_SIZE);
      data->fs.uses.pntc = true;
   }

   try_allocate_vars(data->fs.varyings, &data->common.coeffs, nir, &vars_mask,
                     nir_var_shader_in, true, false, 0,
                     ROGUE_USC_COEFFICIENT_SET_SIZE);
}

/* ---- post-spirv nir prep, mirroring vk_spirv_to_nir's common lowering ---- */

static void prep_spirv_nir(nir_shader *nir)
{
   /* Faithful copy of the common runtime's vk_spirv_to_nir() lowering
    * (src/vulkan/runtime/vk_nir.c), which is what
    * vk_pipeline_shader_stage_to_nir() runs before the PVR driver's own
    * passes.  Keeping this identical is what makes route (a) match the
    * real Vulkan pipeline. */
   NIR_PASS(_, nir, nir_lower_variable_initializers, nir_var_function_temp);
   NIR_PASS(_, nir, nir_lower_returns);
   NIR_PASS(_, nir, nir_inline_functions);
   NIR_PASS(_, nir, nir_opt_copy_prop);
   NIR_PASS(_, nir, nir_opt_constant_folding);
   NIR_PASS(_, nir, nir_opt_deref);

   nir_remove_non_entrypoints(nir);

   NIR_PASS(_, nir, nir_lower_variable_initializers, ~0);

   NIR_PASS(_, nir, nir_split_var_copies);
   NIR_PASS(_, nir, nir_split_per_member_structs);

   NIR_PASS(_, nir, nir_remove_dead_variables,
            nir_var_shader_in | nir_var_shader_out | nir_var_system_value,
            NULL);

   nir_gather_clip_cull_distance_sizes_from_vars(nir);
   NIR_PASS(_, nir, nir_merge_clip_cull_distance_vars);

   NIR_PASS(_, nir, nir_propagate_invariant, false);

   nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

/* Map an fs/vs input variable's component count to a float pipe format. */
static enum pipe_format float_format_for(unsigned comps)
{
   switch (comps) {
   case 1: return PIPE_FORMAT_R32_FLOAT;
   case 2: return PIPE_FORMAT_R32G32_FLOAT;
   case 3: return PIPE_FORMAT_R32G32B32_FLOAT;
   default: return PIPE_FORMAT_R32G32B32A32_FLOAT;
   }
}

/* ---- descriptor sets: combined image samplers ----
 *
 * The hand driver DMAs one pvr_combined_image_sampler_descriptor per sampler
 * into the shared registers, in binding order, BEFORE the push-constant
 * uniforms: sh[0..19] = binding 0 (image 4 dw, image meta 4, sampler 4,
 * sampler meta 4, gather 4), sh[20..39] = binding 1, ... The compiler only
 * needs to know the binding -> shared-register mapping (pco_binding_data), the
 * same way pvr_setup_descriptors() derives it from the pipeline layout. */
#define COMBINED_IMG_SMP_DWORDS \
   (ROGUE_NUM_TEXSTATE_DWORDS * 3 + PCO_IMAGE_META_COUNT + PCO_SAMPLER_META_COUNT)

static void setup_descriptor_sets(pco_data *data, nir_shader *nir)
{
   unsigned max_binding[PVR_MAX_DESCRIPTOR_SETS];
   for (unsigned u = 0; u < PVR_MAX_DESCRIPTOR_SETS; ++u)
      max_binding[u] = 0;
   bool any = false;

   nir_foreach_variable_with_modes (var, nir, nir_var_uniform) {
      if (!glsl_type_is_sampler(glsl_without_array(var->type)))
         continue;
      if (var->data.descriptor_set >= PVR_MAX_DESCRIPTOR_SETS) {
         fprintf(stderr, "descriptor set %u out of range\n", var->data.descriptor_set);
         exit(1);
      }
      if (var->data.binding + 1 > max_binding[var->data.descriptor_set])
         max_binding[var->data.descriptor_set] = var->data.binding + 1;
      any = true;
   }
   if (!any)
      return;

   for (unsigned set = 0; set < PVR_MAX_DESCRIPTOR_SETS; ++set) {
      if (!max_binding[set])
         continue;
      pco_descriptor_set_data *dsd = &data->common.desc_sets[set];
      dsd->binding_count = max_binding[set];
      dsd->bindings = rzalloc_array_size(NULL, sizeof(*dsd->bindings), dsd->binding_count);
   }
   nir_foreach_variable_with_modes (var, nir, nir_var_uniform) {
      if (!glsl_type_is_sampler(glsl_without_array(var->type)))
         continue;
      pco_descriptor_set_data *dsd = &data->common.desc_sets[var->data.descriptor_set];
      pco_binding_data *bd = &dsd->bindings[var->data.binding];
      bd->is_img_smp = true;
      /* range.count doubles as the element count until allocation */
      bd->range.count = glsl_type_is_array(var->type) ? glsl_get_length(var->type) : 1;
   }
}

/* Post-lower: give every used descriptor set its shared registers, sets in
 * order, bindings in order, one COMBINED_IMG_SMP_DWORDS slot per element. */
static void alloc_descriptor_sets(pco_data *data)
{
   for (unsigned set = 0; set < PVR_MAX_DESCRIPTOR_SETS; ++set) {
      pco_descriptor_set_data *dsd = &data->common.desc_sets[set];
      if (!dsd->bindings)
         continue;
      if (!dsd->used) {
         for (unsigned b = 0; b < dsd->binding_count; ++b)
            dsd->bindings[b].range.count = 0;
         continue;
      }
      dsd->range.start = data->common.shareds;
      for (unsigned b = 0; b < dsd->binding_count; ++b) {
         pco_binding_data *bd = &dsd->bindings[b];
         unsigned elems = bd->range.count;
         if (!bd->is_img_smp) {
            bd->range = (pco_range){ 0 };
            continue;
         }
         bd->range = (pco_range){
            .start = data->common.shareds,
            .count = elems * COMBINED_IMG_SMP_DWORDS,
            .stride = COMBINED_IMG_SMP_DWORDS,
         };
         data->common.shareds += elems * COMBINED_IMG_SMP_DWORDS;
      }
      dsd->range.count = data->common.shareds - dsd->range.start;
   }
}

/* ---- blend state: PCO_SHC_BLEND=srgb,drgb[,sa,da[,eqrgb[,eqa]]] ----
 * Factor names: zero one src_color inv_src_color src_alpha inv_src_alpha
 * dst_alpha inv_dst_alpha dst_color inv_dst_color src_alpha_saturate
 * const_color inv_const_color const_alpha inv_const_alpha.
 * Equations: add sub rsub min max. Unset = replace (no blending). */
static const struct { const char *name; enum pipe_blendfactor f; } blend_factors[] = {
   { "zero", PIPE_BLENDFACTOR_ZERO },
   { "one", PIPE_BLENDFACTOR_ONE },
   { "src_color", PIPE_BLENDFACTOR_SRC_COLOR },
   { "inv_src_color", PIPE_BLENDFACTOR_INV_SRC_COLOR },
   { "src_alpha", PIPE_BLENDFACTOR_SRC_ALPHA },
   { "inv_src_alpha", PIPE_BLENDFACTOR_INV_SRC_ALPHA },
   { "dst_alpha", PIPE_BLENDFACTOR_DST_ALPHA },
   { "inv_dst_alpha", PIPE_BLENDFACTOR_INV_DST_ALPHA },
   { "dst_color", PIPE_BLENDFACTOR_DST_COLOR },
   { "inv_dst_color", PIPE_BLENDFACTOR_INV_DST_COLOR },
   { "src_alpha_saturate", PIPE_BLENDFACTOR_SRC_ALPHA_SATURATE },
   { "const_color", PIPE_BLENDFACTOR_CONST_COLOR },
   { "inv_const_color", PIPE_BLENDFACTOR_INV_CONST_COLOR },
   { "const_alpha", PIPE_BLENDFACTOR_CONST_ALPHA },
   { "inv_const_alpha", PIPE_BLENDFACTOR_INV_CONST_ALPHA },
};
static const struct { const char *name; enum pipe_blend_func f; } blend_funcs[] = {
   { "add", PIPE_BLEND_ADD }, { "sub", PIPE_BLEND_SUBTRACT },
   { "rsub", PIPE_BLEND_REVERSE_SUBTRACT }, { "min", PIPE_BLEND_MIN },
   { "max", PIPE_BLEND_MAX },
};

static enum pipe_blendfactor parse_factor(const char *tok)
{
   for (unsigned u = 0; u < ARRAY_SIZE(blend_factors); ++u)
      if (!strcmp(tok, blend_factors[u].name))
         return blend_factors[u].f;
   fprintf(stderr, "PCO_SHC_BLEND: unknown blend factor '%s'\n", tok);
   exit(1);
}

static enum pipe_blend_func parse_func(const char *tok)
{
   for (unsigned u = 0; u < ARRAY_SIZE(blend_funcs); ++u)
      if (!strcmp(tok, blend_funcs[u].name))
         return blend_funcs[u].f;
   fprintf(stderr, "PCO_SHC_BLEND: unknown blend equation '%s'\n", tok);
   exit(1);
}

/* "rgba" subset -> PIPE_MASK_* bits; unset/empty = all channels. */
static uint8_t parse_colormask(const char *spec)
{
   uint8_t mask = 0;

   if (!spec || !*spec)
      return 0xf;
   for (const char *p = spec; *p; ++p) {
      switch (*p) {
      case 'r': mask |= PIPE_MASK_R; break;
      case 'g': mask |= PIPE_MASK_G; break;
      case 'b': mask |= PIPE_MASK_B; break;
      case 'a': mask |= PIPE_MASK_A; break;
      default:
         fprintf(stderr, "PCO_SHC_COLORMASK: unknown channel '%c'\n", *p);
         exit(1);
      }
   }
   return mask;
}

/* Fills rt[0] from the env var; returns true when blending was requested. */
static bool setup_blend(nir_lower_blend_rt *rt, const char *spec)
{
   char buf[128];
   char *tok[6] = { 0 };
   unsigned n = 0;

   if (!spec || !*spec)
      return false;
   snprintf(buf, sizeof(buf), "%s", spec);
   for (char *p = strtok(buf, ","); p && n < 6; p = strtok(NULL, ","))
      tok[n++] = p;
   if (n < 2) {
      fprintf(stderr, "PCO_SHC_BLEND: need at least srgb,drgb\n");
      exit(1);
   }
   rt->rgb.func = tok[4] ? parse_func(tok[4]) : PIPE_BLEND_ADD;
   rt->rgb.src_factor = parse_factor(tok[0]);
   rt->rgb.dst_factor = parse_factor(tok[1]);
   rt->alpha.func = tok[5] ? parse_func(tok[5]) : rt->rgb.func;
   rt->alpha.src_factor = tok[2] ? parse_factor(tok[2]) : rt->rgb.src_factor;
   rt->alpha.dst_factor = tok[3] ? parse_factor(tok[3]) : rt->rgb.dst_factor;
   return true;
}

static uint32_t *read_file(const char *path, size_t *out_len)
{
   FILE *f = fopen(path, "rb");
   if (!f) { fprintf(stderr, "cannot open %s\n", path); exit(1); }
   fseek(f, 0, SEEK_END);
   long n = ftell(f);
   fseek(f, 0, SEEK_SET);
   uint32_t *buf = malloc(n);
   if (fread(buf, 1, n, f) != (size_t)n) { fprintf(stderr, "read fail\n"); exit(1); }
   fclose(f);
   *out_len = n;
   return buf;
}

int main(int argc, char **argv)
{
   if (argc < 3) {
      fprintf(stderr, "usage: %s <vs|fs> <in.spv>\n", argv[0]);
      return 2;
   }
   mesa_shader_stage stage;
   if (!strcmp(argv[1], "vs")) stage = MESA_SHADER_VERTEX;
   else if (!strcmp(argv[1], "fs")) stage = MESA_SHADER_FRAGMENT;
   else { fprintf(stderr, "stage must be vs|fs\n"); return 2; }

   void *mem_ctx = ralloc_context(NULL);
   const struct pvr_device_info *dev_info = &pvr_device_info_36_53_104_796;
   struct pvr_device_runtime_info rt_info = { 0 };
   pco_ctx *ctx = pco_ctx_create(dev_info, &rt_info, mem_ctx);
   /* texelFetch/textureSize/array lookups lower through the USC library. */
   pco_ctx_setup_usclib(ctx, pco_usclib_0_nir, sizeof(pco_usclib_0_nir));

   glsl_type_singleton_init_or_ref();

   size_t spv_len;
   uint32_t *spv = read_file(argv[2], &spv_len);

   nir_shader *nir = spirv_to_nir(spv, spv_len / 4, NULL, stage, "main",
                                  pco_spirv_options(), pco_nir_options());
   if (!nir) { fprintf(stderr, "spirv_to_nir failed\n"); return 1; }
   ralloc_steal(mem_ctx, nir);
   nir_validate_shader(nir, "after spirv_to_nir");

   prep_spirv_nir(nir);

   /* Lean mode (DEFAULT): mark the shader "internal" so the PCO backend skips
    * the runtime-gated fragment epilogue (alpha-to-coverage / color-write-
    * enable / alpha-test / ISP feedback that a full Vulkan FS reads from a
    * per-draw "meta" control word) and, for a VS, the implicit point-size /
    * clip-cull outputs.  This yields a minimal shader in the exact style of the
    * hand driver's ddk_interp_fs / ddk_vs.  Set PCO_SHC_FULL=1 for the faithful
    * full-Vulkan-pipeline output (larger; reads a meta control word). */
   bool lean = !getenv("PCO_SHC_FULL");
   /* discard/demote is only implemented by the full (!internal) fragment
    * epilogue (the savmsk coverage kill lives in lower_isp_fb).  A lean shader
    * that uses discard would silently drop the kill, so force full mode. */
   if (lean && stage == MESA_SHADER_FRAGMENT && nir->info.fs.uses_discard) {
      lean = false;
      fprintf(stderr, "note: shader uses discard -> forcing FULL mode "
                      "(reads a per-draw meta control word at sh[meta])\n");
   }
   if (lean)
      nir->info.internal = true;

   pco_data data = { 0 };
   bool blended = false;
   /* fragment outputs that did not fit in the pixel output registers */
   unsigned num_tile_buffers = 0;

   /* --- pre-lower stage init (mirrors pvr_preprocess_shader_data) --- */
   setup_descriptor_sets(&data, nir);
   if (stage == MESA_SHADER_VERTEX) {
      /* attrib formats: infer float32 vecN from the input variable types. */
      nir_foreach_shader_in_variable (var, nir) {
         unsigned comps = glsl_get_components(glsl_without_array(var->type));
         data.vs.attrib_formats[var->data.location] = float_format_for(comps);
      }
   } else {
      /* single colour output. */
      /* PCO_SHC_OUT: the render target format the colour output is packed
       * for (the output register count is the format's bytes / 4, as
       * pvr_arch_pipeline sizes an mrt resource): default RGBA8 unorm,
       * f16x4 (RGBA16F), f16 (R16F), f32 (R32F), f32x4. */
      const char *outsel = getenv("PCO_SHC_OUT");
      enum pipe_format ofmt = PIPE_FORMAT_R8G8B8A8_UNORM;
      unsigned ocount = 1;
      if (outsel && !strcmp(outsel, "f32x4")) { ofmt = PIPE_FORMAT_R32G32B32A32_FLOAT; ocount = 4; }
      else if (outsel && !strcmp(outsel, "f16x4")) { ofmt = PIPE_FORMAT_R16G16B16A16_FLOAT; ocount = 2; }
      else if (outsel && !strcmp(outsel, "f16")) { ofmt = PIPE_FORMAT_R16_FLOAT; ocount = 1; }
      else if (outsel && !strcmp(outsel, "f32")) { ofmt = PIPE_FORMAT_R32_FLOAT; ocount = 1; }
      data.fs.z_replicate = ~0u;

      /* Passthrough (replace) blend for RT0, colormask = RGBA -- mirrors
       * pvr_init_fs_blend for a pipeline with blending disabled.  Leaving
       * blend_opts zeroed makes nir_lower_blend synthesise a broken shader
       * (colormask=0 -> writes nothing -> spurious kill/discard). */
      const nir_lower_blend_channel replace = {
         .func = PIPE_BLEND_ADD,
         .src_factor = PIPE_BLENDFACTOR_ONE,
         .dst_factor = PIPE_BLENDFACTOR_ZERO,
      };
      /* Every colour output the shader writes, not just the first: a G-buffer
       * pass declares layout(location = 0..2) out, and a compile that keeps
       * only output 0 leaves the end-of-tile program emitting the same
       * register into every attachment. Output i takes its own registers. */
      {
         unsigned reg = 0, nrt = 0, ntb = 0;
         /* A pixel has four output registers on this core (BXS-4-64 has no
          * eight_output_registers), and a 64/128-bit source starts at an even
          * one (pbe2_in_xe, pvr_mrt_alloc_from_buffer). A shader whose outputs
          * do not all fit -- three RGBA16F attachments want six registers --
          * would have the USC write past them, which faults the GPU, so the
          * outputs that do not fit share register 0: the driver emits only the
          * attachments that fit (pvr_backend.c). */
         const unsigned max_out_regs = 4;

         for (unsigned i = FRAG_RESULT_DATA0; i < FRAG_RESULT_MAX; ++i) {
            if (!(nir->info.outputs_written & BITFIELD64_BIT(i)))
               continue;
            if (ocount >= 2 && (reg & 1))
               reg++;
            if (reg + ocount > max_out_regs) {
               /* The output goes to a tile buffer: the shader stores it to
                * memory (the address comes from shared registers, the offset
                * within the tile from a special register), and the
                * end-of-tile program loads it back into the pixel output
                * registers before emitting that attachment (Mesa
                * pvr_arch_mrt.c, USC_MRT_RESOURCE_TYPE_MEMORY). One tile
                * buffer per spilled output, each used from its dword 0. */
               fprintf(stderr,
                       "note: fragment output %u (%u register(s)) does not fit "
                       "in %u pixel output registers: tile buffer %u\n",
                       i - FRAG_RESULT_DATA0, ocount, max_out_regs, ntb);
               data.fs.output_tile_buffers |= BITFIELD_BIT(i - FRAG_RESULT_DATA0);
               data.fs.output_formats[i] = ofmt;
               data.fs.outputs[i] = (pco_range){ .start = ntb, .offset = 0,
                                                 .count = ocount };
               ntb++;
               if (nrt < ARRAY_SIZE(data.fs.blend_opts.rt)) {
                  data.fs.blend_opts.rt[nrt].format = ofmt;
                  data.fs.blend_opts.rt[nrt].rgb = replace;
                  data.fs.blend_opts.rt[nrt].alpha = replace;
                  data.fs.blend_opts.rt[nrt].colormask = 0xf;
               }
               nrt++;
               continue;
            }
            data.fs.output_formats[i] = ofmt;
            data.fs.outputs[i] = (pco_range){ .start = reg, .count = ocount };
            if (nrt < ARRAY_SIZE(data.fs.blend_opts.rt)) {
               data.fs.blend_opts.rt[nrt].format = ofmt;
               data.fs.blend_opts.rt[nrt].rgb = replace;
               data.fs.blend_opts.rt[nrt].alpha = replace;
               data.fs.blend_opts.rt[nrt].colormask = 0xf;
            }
            reg += ocount;
            nrt++;
         }
         num_tile_buffers = ntb;
         if (!nrt) {   /* a shader that writes none: keep the old single slot */
            data.fs.output_formats[FRAG_RESULT_DATA0] = ofmt;
            data.fs.outputs[FRAG_RESULT_DATA0] = (pco_range){ .start = 0, .count = ocount };
            data.fs.blend_opts.rt[0].format = ofmt;
            data.fs.blend_opts.rt[0].rgb = replace;
            data.fs.blend_opts.rt[0].alpha = replace;
            data.fs.blend_opts.rt[0].colormask = 0xf;
         }
      }
      /* GL blending is lowered into the FS (nir_lower_blend: fb-fetch of the
       * on-chip PIXOUT + the blend arithmetic); the state is fixed per binary. */
      blended = setup_blend(&data.fs.blend_opts.rt[0], getenv("PCO_SHC_BLEND"));
      /* glColorMask is lowered the same way (nir_color_mask: fb-fetch of the
       * masked channels). PCO_SHC_COLORMASK = any of "rgba" (unset = rgba). */
      data.fs.blend_opts.rt[0].colormask = parse_colormask(getenv("PCO_SHC_COLORMASK"));
   }

   pco_preprocess_nir(ctx, nir);

   /* The Vulkan driver links the stages (pco_rev_link_nir) between
    * preprocess and lower, which also fixes up FS input interpolation:
    * gl_FragCoord (an input variable only after preprocess) and the point
    * coord are noperspective, unqualified inputs smooth. Stages are compiled
    * alone here, so mirror that consumer-side fixup. */
   if (stage == MESA_SHADER_FRAGMENT) {
      nir_foreach_shader_in_variable (in_var, nir) {
         if (in_var->data.location == VARYING_SLOT_POS ||
             in_var->data.location == VARYING_SLOT_PNTC)
            in_var->data.interpolation = INTERP_MODE_NOPERSPECTIVE;
         else if (in_var->data.interpolation == INTERP_MODE_NONE)
            in_var->data.interpolation = INTERP_MODE_SMOOTH;
      }
   }

   pco_lower_nir(ctx, nir, &data);
   pco_postprocess_nir(ctx, nir, &data);

   /* --- post-lower alloc (mirrors pvr_postprocess_shader_data) --- */

   /* Descriptor sets (combined image samplers) first: the hand driver DMAs
    * the texture descriptors at sh0.. and the uniforms after them. */
   alloc_descriptor_sets(&data);

   /* Push-constant uniforms -> shared regs (DOUTD common store), right after
    * the descriptors (sh0.. when the shader samples nothing). */
   if (data.common.push_consts.used == ~0U) {
      /* A load with a non-constant offset (dynamic indexing into a uniform
       * array, a vector component picked at run time) makes the gather pass
       * give up on the size. The whole push-constant block is DMA'd anyway,
       * so size the range from the block declaration; the translator turns
       * dynamic offsets into pco_mov_offset reads of the shared registers. */
      unsigned dwords = 0;
      nir_foreach_variable_with_modes(var, nir, nir_var_mem_push_const) {
         unsigned sz = glsl_get_explicit_size(var->type, false);
         dwords = MAX2(dwords, (var->data.driver_location + sz + 3) / 4);
      }
      data.common.push_consts.used = dwords;
   }
   if (data.common.push_consts.used > 0 && data.common.push_consts.used != ~0U) {
      data.common.push_consts.range = (pco_range){
         .start = data.common.shareds,
         .count = data.common.push_consts.used,
      };
      data.common.shareds += data.common.push_consts.used;
   }

   /* Blend constants (CONST_COLOR/CONST_ALPHA factors): shared regs right
    * after the push constants, DMA'd by the driver (pvr_setup_fs_blend). */
   if (stage == MESA_SHADER_FRAGMENT && data.fs.blend_consts_needed) {
      data.fs.blend_consts = (pco_range){
         .start = data.common.shareds,
         .count = util_bitcount(data.fs.blend_consts_needed),
      };
      data.common.shareds += data.fs.blend_consts.count;
   }

   if (stage == MESA_SHADER_VERTEX) {
      alloc_vs_attribs(&data, nir);
      alloc_vs_sysvals(&data, nir);
      alloc_vs_varyings(&data, nir);
   } else {
      /* fs meta register (ISP feedback / sample mask control word).  Only the
       * full-Vulkan path reads it; lean shaders don't, so don't reserve it. */
      if (!lean) {
         data.fs.meta = (pco_range){ .start = data.common.shareds, .count = 1 };
         ++data.common.shareds;
      }
      /* gl_FrontFacing: the front-face op the driver writes into a shared
       * register (Mesa pvr_alloc_fs_sysvals + PVR_BUFFER_TYPE_FRONT_FACE_OP). */
      if (BITSET_TEST(nir->info.system_values_read, SYSTEM_VALUE_FRONT_FACE)) {
         data.common.sys_vals[SYSTEM_VALUE_FRONT_FACE] =
            (pco_range){ .start = data.common.shareds, .count = 1 };
         ++data.common.shareds;
      }

      /* The tile buffer addresses the shader stores its spilled outputs to:
       * two shared registers each (Mesa PVR_BUFFER_TYPE_TILE_BUFFERS), after
       * everything else the shader reads from shared memory, so the driver
       * appends them to the block it already DMAs. */
      if (num_tile_buffers) {
         data.fs.num_tile_buffers = num_tile_buffers;
         data.fs.tile_buffers = (pco_range){ .start = data.common.shareds,
                                             .count = 2 * num_tile_buffers,
                                             .stride = 2 };
         data.common.shareds += 2 * num_tile_buffers;
      }

      alloc_fs_varyings(&data, nir);
   }

   pco_shader *sh = pco_trans_nir(ctx, nir, &data, mem_ctx);
   if (!sh) { fprintf(stderr, "pco_trans_nir failed\n"); return 1; }
   pco_process_ir(ctx, sh);
   pco_encode_ir(ctx, sh);

   const uint8_t *bin = pco_shader_binary_data(sh);
   unsigned len = pco_shader_binary_size(sh);
   pco_data *d = pco_shader_data(sh);

   /* ---- machine-readable metadata to stderr ---- */
   fprintf(stderr, "=== pco_shc metadata (%s, %s) ===\n",
           stage == MESA_SHADER_VERTEX ? "VS" : "FS",
           lean ? "lean" : "full-vulkan");
   fprintf(stderr, "usc_bytes=%u usc_dwords=%u\n", len, len / 4);
   fprintf(stderr, "temps=%u vtxins=%u coeffs=%u shareds=%u internal_regs=%u\n",
           d->common.temps, d->common.vtxins, d->common.coeffs,
           d->common.shareds, d->common.interns);
   fprintf(stderr, "entry_offset=%u\n", d->common.entry_offset);

   if (stage == MESA_SHADER_VERTEX) {
      fprintf(stderr, "vtxouts(vs_output_size)=%u\n", d->vs.vtxouts);
      for (unsigned u = 0; u < ARRAY_SIZE(vs_sysvals); ++u)
         if (d->common.sys_vals[vs_sysvals[u]].count > 0)
            fprintf(stderr, "sysval[%s]: vtxin.start=%u count=%u\n",
                    gl_system_value_name(vs_sysvals[u]),
                    d->common.sys_vals[vs_sysvals[u]].start,
                    d->common.sys_vals[vs_sysvals[u]].count);
      fprintf(stderr, "f32_smooth=%u f32_flat=%u f32_npc=%u\n",
              d->vs.f32_smooth, d->vs.f32_flat, d->vs.f32_npc);
      for (unsigned i = 0; i < VERT_ATTRIB_MAX; ++i)
         if (d->vs.attribs[i].count)
            fprintf(stderr, "attrib[loc=%u gen=%d]: vtxin.start=%u count=%u fmt=%u\n",
                    i, (int)i - VERT_ATTRIB_GENERIC0, d->vs.attribs[i].start,
                    d->vs.attribs[i].count, d->vs.attrib_formats[i]);
      for (unsigned i = 0; i < VARYING_SLOT_MAX; ++i)
         if (d->vs.varyings[i].count)
            fprintf(stderr, "vs_out[slot=%u %s]: vtxout.start=%u count=%u\n",
                    i, i == VARYING_SLOT_POS ? "POS" :
                       (i >= VARYING_SLOT_VAR0 ? "VARn" : "?"),
                    d->vs.varyings[i].start, d->vs.varyings[i].count);
   } else {
      fprintf(stderr, "uses: z=%d w=%d discard=%d pntc=%d fbfetch=%d\n",
              d->fs.uses.z, d->fs.uses.w, d->fs.uses.discard, d->fs.uses.pntc,
              d->fs.uses.fbfetch);
      fprintf(stderr, "blend=%s\n", blended ? getenv("PCO_SHC_BLEND") : "replace");
      fprintf(stderr, "colormask=0x%x\n", d->fs.blend_opts.rt[0].colormask);
      if (d->fs.blend_consts.count)
         fprintf(stderr, "blend_consts: sh.start=%u count=%u\n",
                 d->fs.blend_consts.start, d->fs.blend_consts.count);
      if (d->common.sys_vals[SYSTEM_VALUE_FRONT_FACE].count)
         fprintf(stderr, "front_face_op: sh.start=%u count=%u\n",
                 d->common.sys_vals[SYSTEM_VALUE_FRONT_FACE].start,
                 d->common.sys_vals[SYSTEM_VALUE_FRONT_FACE].count);
      {
         /* The image-descriptor PCK_INFO meta word the driver must supply for
          * an RGBA8 UNORM texture (pvr_arch_tex_state.c setup_pck_info). */
         bool scale = false, roundzero = false, split = false;
         enum pco_pck_format pf = pco_pipe_to_pck_format(PIPE_FORMAT_R8G8B8A8_UNORM,
                                                          &scale, &roundzero, &split);
         uint32_t pck = (pf == ~0u) ? PVR_PCK_FORMAT_INVALID : (uint32_t)pf;
         if (split) pck |= PVR_PCK_INFO_SPLIT_BIT;
         if (scale) pck |= PVR_PCK_INFO_SCALE_BIT;
         if (roundzero) pck |= PVR_PCK_INFO_ROUNDZERO_BIT;
         pck |= BITFIELD_BIT(PVR_PCK_INFO_UNORM_OFFSET);
         fprintf(stderr, "pck_info(rgba8_unorm)=0x%x\n", pck);
      }
      for (unsigned i = 0; i < VARYING_SLOT_MAX; ++i)
         if (d->fs.varyings[i].count)
            fprintf(stderr, "fs_in[slot=%u %s]: coeff.start=%u count=%u (cf%u.., %u coeff-sets)\n",
                    i, i == VARYING_SLOT_POS ? "POS(z/w)" :
                       (i >= VARYING_SLOT_VAR0 ? "VARn" : "?"),
                    d->fs.varyings[i].start, d->fs.varyings[i].count,
                    d->fs.varyings[i].start,
                    d->fs.varyings[i].count / ROGUE_USC_COEFFICIENT_SET_SIZE);
      for (unsigned i = 0; i < FRAG_RESULT_MAX; ++i)
         if (d->fs.outputs[i].count) {
            bool tb = i >= FRAG_RESULT_DATA0 &&
                      (d->fs.output_tile_buffers &
                       BITFIELD_BIT(i - FRAG_RESULT_DATA0));

            fprintf(stderr, "fs_out[res=%u]: %s.start=%u count=%u fmt=%u\n",
                    i, tb ? "tilebuf" : "po",
                    d->fs.outputs[i].start, d->fs.outputs[i].count,
                    d->fs.output_formats[i]);
         }
      if (d->fs.tile_buffers.count)
         fprintf(stderr, "tile_buffers (driver-supplied addresses): "
                         "sh.start=%u count=%u stride=%u buffers=%u\n",
                 d->fs.tile_buffers.start, d->fs.tile_buffers.count,
                 d->fs.tile_buffers.stride, d->fs.num_tile_buffers);
   }
   for (unsigned set = 0; set < PVR_MAX_DESCRIPTOR_SETS; ++set) {
      const pco_descriptor_set_data *dsd = &d->common.desc_sets[set];
      if (!dsd->bindings || !dsd->used)
         continue;
      for (unsigned b = 0; b < dsd->binding_count; ++b)
         if (dsd->bindings[b].range.count)
            fprintf(stderr, "desc[set=%u binding=%u]: sh.start=%u count=%u stride=%u img_smp=%d used=%d\n",
                    set, b, dsd->bindings[b].range.start, dsd->bindings[b].range.count,
                    dsd->bindings[b].range.stride, dsd->bindings[b].is_img_smp,
                    dsd->bindings[b].used);
   }
   if (d->common.push_consts.range.count)
      fprintf(stderr, "push_consts (uniform/DOUTD): sh.start=%u count=%u dwords\n",
              d->common.push_consts.range.start, d->common.push_consts.range.count);
   if (stage == MESA_SHADER_FRAGMENT && d->fs.meta.count)
      fprintf(stderr, "meta (ISP-feedback control word, driver-supplied): "
                      "sh.start=%u count=%u\n",
              d->fs.meta.start, d->fs.meta.count);

   /* USC dwords */
   fprintf(stderr, "dwords:");
   for (unsigned i = 0; i + 4 <= len; i += 4) {
      uint32_t dw; memcpy(&dw, bin + i, 4);
      fprintf(stderr, " 0x%08x", dw);
   }
   fprintf(stderr, "\n");

   /* raw binary to stdout (suppress with PCO_SHC_NOBIN=1 so PCO_DEBUG_PRINT
    * disassembly on stdout stays clean). */
   if (!getenv("PCO_SHC_NOBIN"))
      fwrite(bin, 1, len, stdout);

   ralloc_free(mem_ctx);
   return 0;
}
