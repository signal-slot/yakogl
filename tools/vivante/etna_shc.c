/* SPDX-License-Identifier: MIT
 *
 * etna_shc: YakoGL's offline shader tool for the Vivante GC7000 (i.MX8MP).
 * Reads an OpenGL-flavoured SPIR-V module (glslangValidator -G), turns it into
 * NIR the way Mesa's GL state tracker would hand it to the etnaviv driver, runs
 * the driver's own compiler (etna_compile_shader) for the GPU named by the
 * hardware database, and prints the instruction words and the reflection
 * (input/output registers, uniform layout, texture units) as JSON. The GPU
 * specs come from the database entry alone: no device is needed.
 *
 *   etna_shc vert|frag in.spv > out.json
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/nir/nir.h"
#include "compiler/nir/nir_builder.h"
#include "compiler/glsl_types.h"
#include "compiler/spirv/nir_spirv.h"
#include "util/ralloc.h"

#include "etna_core_info.h"
#include "hwdb/etna_hwdb.h"
#include "etnaviv_screen.h"
#include "etnaviv_shader.h"
#include "etnaviv_compiler.h"
#include "etnaviv_context.h"
#include "etnaviv_nir.h"
#include "etnaviv_debug.h"

static uint32_t *read_words(const char *path, size_t *nwords)
{
	FILE *f = fopen(path, "rb");
	if (!f) { perror(path); return NULL; }
	fseek(f, 0, SEEK_END);
	long n = ftell(f);
	fseek(f, 0, SEEK_SET);
	uint32_t *w = malloc(n + 4);
	if (fread(w, 1, n, f) != (size_t)n) { perror("read"); fclose(f); return NULL; }
	fclose(f);
	*nwords = n / 4;
	return w;
}

/* GL-style location assignment: the driver reads var->data.driver_location
 * for inputs (attribute/varying register index), outputs and uniforms
 * (vec4 slots). The state tracker does this before handing NIR over. */
static int vec4_slots(const struct glsl_type *t, bool bindless)
{
	return glsl_count_vec4_slots(t, false, bindless);
}

/* vec4_slots in nir_lower_io's type: the state tracker's
 * st_unpacked_uniforms_type_size */
static unsigned uniform_slots(const struct glsl_type *t, bool bindless)
{
	return glsl_count_vec4_slots(t, false, bindless);
}

static void assign_locations(nir_shader *nir, nir_variable_mode mode, bool uniforms)
{
	/* stable order: by location */
	unsigned next = 0;
	for (int loc = 0; loc < 128; loc++) {
		nir_foreach_variable_with_modes(var, nir, mode) {
			if (var->data.location != loc)
				continue;
			if (uniforms && glsl_type_is_sampler(glsl_without_array(var->type))) {
				var->data.driver_location = var->data.binding;
				continue;
			}
			var->data.driver_location = next;
			next += uniforms ? vec4_slots(var->type, false) : 1;
		}
	}
}

int main(int argc, char **argv)
{
	if (argc < 3) {
		fprintf(stderr, "usage: etna_shc vert|frag in.spv [model rev productid]\n");
		return 2;
	}
	mesa_shader_stage stage = strcmp(argv[1], "vert") == 0 ? MESA_SHADER_VERTEX : MESA_SHADER_FRAGMENT;
	size_t nwords;
	uint32_t *spv = read_words(argv[2], &nwords);
	if (!spv)
		return 1;

	/* The GPU: i.MX8MP's GC7000UL unless told otherwise (model rev product) */
	struct etna_core_info info = { .model = 0x7000, .revision = 0x6204, .product_id = 0x70003 };
	if (argc >= 6) {
		info.model = strtoul(argv[3], NULL, 0);
		info.revision = strtoul(argv[4], NULL, 0);
		info.product_id = strtoul(argv[5], NULL, 0);
	}
	if (!etna_query_feature_db(&info)) {
		fprintf(stderr, "hwdb: no entry for model 0x%x rev 0x%x product 0x%x\n", info.model,
			info.revision, info.product_id);
		return 1;
	}
	/* the gross architecture level, as src/etnaviv/drm/etnaviv_gpu.c derives it */
	info.halti = etna_core_has_feature(&info, ETNA_FEATURE_HALTI5) ? 5 :
		     etna_core_has_feature(&info, ETNA_FEATURE_HALTI4) ? 4 :
		     etna_core_has_feature(&info, ETNA_FEATURE_HALTI3) ? 3 :
		     etna_core_has_feature(&info, ETNA_FEATURE_HALTI2) ? 2 :
		     etna_core_has_feature(&info, ETNA_FEATURE_HALTI1) ? 1 :
		     etna_core_has_feature(&info, ETNA_FEATURE_HALTI0) ? 0 : -1;
	struct etna_screen *screen = calloc(1, sizeof(*screen));
	screen->info = &info;
	etna_get_specs(screen);
	screen->compiler = etna_compiler_create("etna_shc", &info);
	if (!screen->compiler) {
		fprintf(stderr, "etna_compiler_create failed\n");
		return 1;
	}

	glsl_type_singleton_init_or_ref();
	struct spirv_to_nir_options spirv_opts = {
		.environment = NIR_SPIRV_OPENGL,
	};
	nir_shader *nir = spirv_to_nir(spv, nwords, NULL, stage, "main", &spirv_opts,
				       etna_compiler_get_options(screen->compiler));
	if (!nir) {
		fprintf(stderr, "spirv_to_nir failed\n");
		return 1;
	}
	nir_validate_shader(nir, "after spirv_to_nir");
	/* the common front-end lowering (what vk/gl_nir do before a driver sees it) */
	NIR_PASS(_, nir, nir_lower_variable_initializers, nir_var_function_temp);
	NIR_PASS(_, nir, nir_lower_returns);
	NIR_PASS(_, nir, nir_inline_functions);
	NIR_PASS(_, nir, nir_opt_copy_prop);
	NIR_PASS(_, nir, nir_opt_deref);
	nir_remove_non_entrypoints(nir);
	NIR_PASS(_, nir, nir_lower_variable_initializers, ~0);
	NIR_PASS(_, nir, nir_split_var_copies);
	NIR_PASS(_, nir, nir_split_per_member_structs);
	NIR_PASS(_, nir, nir_remove_dead_variables,
		 nir_var_shader_in | nir_var_shader_out | nir_var_uniform | nir_var_system_value, NULL);
	NIR_PASS(_, nir, nir_lower_global_vars_to_local);
	NIR_PASS(_, nir, nir_split_var_copies);
	NIR_PASS(_, nir, nir_lower_var_copies);
	NIR_PASS(_, nir, nir_lower_vars_to_ssa);
	NIR_PASS(_, nir, nir_opt_copy_prop);
	NIR_PASS(_, nir, nir_opt_dce);
	NIR_PASS(_, nir, nir_lower_samplers);
	/* what the GL state tracker does before the driver (st_glsl_to_nir.cpp):
	 * system values become intrinsics (etnaviv reads load_frag_coord, not a
	 * gl_FragCoord variable) and texelFetch on a plain 2D texture stays txf */
	/* gl_PointCoord is a fragment INPUT at VARYING_SLOT_PNTC to the GL state
	 * tracker, and etnaviv reads it as one (etnaviv_compiler_nir.c, the
	 * VARYING_SLOT_PNTC case). SPIR-V says it is a system value, and
	 * nir_lower_system_values would make it a load_point_coord intrinsic the
	 * driver has no emitter for: the liveness pass then asserts on a source
	 * that was never defined. So make the variable the input it is. */
	if (stage == MESA_SHADER_FRAGMENT) {
		bool moved = false;

		nir_foreach_variable_with_modes(var, nir, nir_var_system_value) {
			if (var->data.location != SYSTEM_VALUE_POINT_COORD)
				continue;
			var->data.mode = nir_var_shader_in;
			var->data.location = VARYING_SLOT_PNTC;
			moved = true;
		}
		if (moved)
			nir_fixup_deref_modes(nir);
	}
	NIR_PASS(_, nir, nir_lower_system_values);
	/* the tail of gl_nir_opts (gl_nir_linker.c): flrp is lowered by the frontend,
	 * not by the driver (etnaviv sets lower_flrp32 and has no flrp emitter) */
	NIR_PASS(_, nir, nir_lower_flrp, 32, false);
	NIR_PASS(_, nir, nir_lower_var_copies);
	{
		struct nir_lower_tex_options tex_opts = { .lower_rect = true };
		NIR_PASS(_, nir, nir_lower_tex, &tex_opts);
	}
	/* A matrix or array varying is one variable over several slots. The GL
	 * linker hands the driver one variable per element (gl_nir_linker.c and
	 * the state tracker run this lowering); etnaviv's output list does not
	 * cope with the wide one -- a mat2 varying came out as two registers
	 * with slot 21905 and no components -- so split them the same way.
	 *
	 * Not the vertex shader's inputs, though: those are the attributes, and
	 * the pass would turn a matrix attribute into one input per column at
	 * location + k. The table keeps a matrix attribute as one input with its
	 * columns in vi_reg + k, which is what src/vivante/shader.rs binds, and
	 * a column at a location no declaration names cannot be reflected (the
	 * matattr rows, issue #22). Mesa's PowerVR compiler makes the same
	 * choice (pco_nir.c: outputs_only for MESA_SHADER_VERTEX). */
	NIR_PASS(_, nir, nir_lower_io_array_vars_to_elements_no_indirects,
	         stage == MESA_SHADER_VERTEX);
	/*
	 * The driver asserts that a FRAGMENT shader's input variables carry
	 * driver_locations 0..n-1 with no gaps: it removes the dead ones
	 * itself (etnaviv_compiler_nir.c, "Remove any dead in variables
	 * before we iterate over them") and then compares the number of
	 * inputs it counted with the highest location it saw. A dead input
	 * in the middle leaves a gap and the assertion fires.
	 *
	 * That is how femtovg's filter_image.nodiscard.frag failed to build:
	 * taking the discard out leaves the stroke mask unused, and with it
	 * `ftcoord` at location 0, so only `fpos` at location 1 survived.
	 *
	 * So do the same removal here, before the locations are handed out,
	 * and number what is left. The variables are still reached through
	 * derefs at this point (nir_lower_io runs inside the driver), so the
	 * dead loads have to go first for a variable to look unused. Only
	 * fragment inputs: the vertex branch of that loop has no assertion,
	 * and renumbering vertex attributes would move every attribute of
	 * every shader already in the table.
	 */
	if (stage == MESA_SHADER_FRAGMENT) {
		NIR_PASS(_, nir, nir_opt_dce);
		NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in, NULL);
	}

	assign_locations(nir, nir_var_shader_in, false);
	assign_locations(nir, nir_var_shader_out, false);
	assign_locations(nir, nir_var_uniform, true);
	/*
	 * Uniforms, as the GL state tracker hands them to the driver
	 * (st_glsl_to_nir.cpp st_nir_lower_uniforms): load_uniform in vec4
	 * slots, then -- when the driver's options ask, as etnaviv's do from
	 * HALTI2 on (etnaviv_compiler.c: lower_uniforms_to_ubo = halti >= 2) --
	 * every load_uniform becomes a load_ubo of UBO 0. etnaviv turns the
	 * constant-offset ones back into constant-file reads
	 * (etnaviv_nir_lower_ubo_to_uniform.c) and keeps a run-time index as a
	 * LOAD from the uniform block in memory.
	 *
	 * Without this, a run-time index stayed a load_uniform and became
	 * MOVAR + a relative constant-file read (etnaviv_compiler_nir.c, the
	 * load_uniform case), which reads element 0 whatever the index on the
	 * i.MX8MP's GC7000: gl_test 45 es3dyn read 3 x cols[0] (2026-10-04,
	 * record 594b010e 002d0102 bf990000 bf9900e6).
	 */
	if (nir->options->lower_uniforms_to_ubo) {
		unsigned slots = 0;

		nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
			if (glsl_type_is_sampler(glsl_without_array(var->type)))
				continue;
			slots = MAX2(slots, var->data.driver_location + vec4_slots(var->type, false));
		}
		nir->num_uniforms = slots;
		NIR_PASS(_, nir, nir_lower_io, nir_var_uniform, uniform_slots, (nir_lower_io_options)0);
		NIR_PASS(_, nir, nir_lower_uniforms_to_ubo, false, false);
	}
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
	if (getenv("ETNA_SHC_DUMP_NIR")) {
		nir_print_shader(nir, stderr);
		etna_mesa_debug |= ETNA_DBG_DUMP_SHADERS;	/* and the final NIR + ISA */
	}

	struct etna_shader shader = {
		.nir = nir, .info = &info, .specs = &screen->specs,
		.compiler = screen->compiler, .screen = screen,
	};
	struct etna_shader_variant v = { .shader = &shader };
	if (!etna_compile_shader(&v)) {
		fprintf(stderr, "etna_compile_shader failed\n");
		return 1;
	}

	printf("{\n  \"stage\": \"%s\",\n", stage == MESA_SHADER_VERTEX ? "vert" : "frag");
	printf("  \"gpu\": { \"model\": \"0x%x\", \"revision\": \"0x%x\", \"halti\": %d },\n",
	       info.model, info.revision, info.halti);
	printf("  \"code_size\": %u,\n  \"num_temps\": %u,\n", v.code_size, v.num_temps);
	printf("  \"vs_pos_out_reg\": %d,\n  \"vs_pointsize_out_reg\": %d,\n  \"vs_id_in_reg\": %d,\n",
	       v.vs_pos_out_reg, v.vs_pointsize_out_reg, v.vs_id_in_reg);
	printf("  \"ps_color_out_reg\": %d,\n  \"ps_depth_out_reg\": %d,\n  \"input_count_unk8\": %u,\n",
	       v.ps_color_out_reg[0], v.ps_depth_out_reg, v.input_count_unk8);
	printf("  \"uses_discard\": %s,\n  \"vs_load_balancing\": \"0x%08x\",\n",
	       v.uses_discard ? "true" : "false", v.vs_load_balancing);
	printf("  \"inputs\": [");
	for (unsigned i = 0; i < v.infile.num_reg; i++)
		printf("%s{ \"reg\": %d, \"slot\": %d, \"components\": %u, \"interp\": %u }",
		       i ? ", " : "", v.infile.reg[i].reg, v.infile.reg[i].slot,
		       v.infile.reg[i].num_components, v.infile.reg[i].interpolation);
	printf("],\n  \"outputs\": [");
	for (unsigned i = 0; i < v.outfile.num_reg; i++)
		printf("%s{ \"reg\": %d, \"slot\": %d, \"components\": %u }", i ? ", " : "",
		       v.outfile.reg[i].reg, v.outfile.reg[i].slot, v.outfile.reg[i].num_components);
	printf("],\n  \"uniforms\": [");
	for (unsigned i = 0; i < v.uniforms.count; i++)
		printf("%s{ \"contents\": %u, \"data\": \"0x%08x\" }", i ? ", " : "",
		       v.uniforms.contents[i], v.uniforms.data[i]);
	printf("],\n  \"uniform_names\": [");
	{
		unsigned n = 0;
		nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
			bool sampler = glsl_type_is_sampler(glsl_without_array(var->type));
			printf("%s{ \"name\": \"%s\", \"location\": %d, \"%s\": %u, \"vec4s\": %d }",
			       n++ ? ", " : "", var->name, var->data.location,
			       sampler ? "unit" : "slot", var->data.driver_location,
			       sampler ? 0 : vec4_slots(var->type, false));
		}
	}
	printf("],\n  \"code\": [");
	for (unsigned i = 0; i < v.code_size; i++)
		printf("%s\"0x%08x\"", i ? ", " : "", v.code[i]);
	printf("]\n}\n");
	fflush(stdout);
	etna_dump_shader(&v);   /* the disassembly, on stdout after the JSON */
	return 0;
}
