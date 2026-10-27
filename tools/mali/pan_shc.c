/* SPDX-License-Identifier: MIT
 *
 * pan_shc: YakoGL's reference shader tool for the Arm Mali-G610 (Valhall v10,
 * RK3588 / NanoPC-T6). It is an instrument, not a product path: YakoGL's own
 * compiler (src/shaderc/online/) is compared with what Mesa makes for the same
 * GL program. It is C only because it is built inside Mesa's tree and calls
 * Mesa's compiler, as tools/v3d/v3d_shc.c does; nothing here ships.
 *
 * Reads one GL program as two OpenGL-flavoured SPIR-V modules (made from GLSL
 * ES 1.00 by tools/mali/glsl2pan.py), turns them into NIR the way Mesa's GL
 * state tracker does for a GL_ARB_gl_spirv program, runs the gallium panfrost
 * driver's shader preparation and compile for both stages, and prints Mesa's
 * Valhall binaries with the facts a driver needs to run them.
 *
 *   pan_shc vs.spv fs.spv [--gpu-id HEX] [--gpu-variant N]
 *           [--prim triangles|lines|points] > out.json 2> disasm.txt
 *
 * stdout: the line "pan_shc: build <yakogl git describe> mesa <version>", then
 * one JSON object. stderr: the Valhall disassembly of every program region.
 *
 * Transcribed from Mesa 26.2.1 (MIT, Copyright Collabora, Arm, Intel and
 * others), each pass cited by file:line where it is called:
 *   - the GL front half, as in tools/v3d/v3d_shc.c, with panfrost's screen
 *     caps: src/mesa/main/glspirv.c _mesa_spirv_to_nir(),
 *     src/compiler/glsl/gl_nir_linker.c gl_nir_link_spirv(), preprocess_shader(),
 *     prelink_lowering(), gl_nir_lower_optimize_varyings(), gl_nir_opts(),
 *     src/mesa/state_tracker/st_glsl_to_nir.cpp st_link_glsl_to_nir(),
 *     st_glsl_to_nir_post_opts(), st_finalize_nir(), st_nir_lower_uniforms(),
 *     src/mesa/state_tracker/st_program.c st_create_common_variant() /
 *     st_create_fp_variant(), the uniform storage of
 *     src/compiler/glsl/gl_nir_link_uniforms.c add_parameter() and
 *     src/mesa/program/prog_parameter.c _mesa_add_parameter();
 *   - the screen caps: src/gallium/drivers/panfrost/pan_screen.c;
 *   - the driver: src/gallium/drivers/panfrost/pan_shader.c
 *     panfrost_create_shader_state() (:500-633), panfrost_link_shader()
 *     (:660-690), panfrost_default_shader_key() (:478-497),
 *     panfrost_build_fs_key() (:308-352) and panfrost_shader_compile() (:95-239).
 *     Three of the driver's passes are
 *     compiled from the driver's own files, unmodified (build_pan_shc.sh copies
 *     them; tools/mali/pan_shc-shim/ stands in for pan_context.h).
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/glsl_types.h"
#include "compiler/nir/nir.h"
#include "compiler/nir/nir_builder.h"
#include "compiler/spirv/nir_spirv.h"
#include "mesa/program/prog_statevars.h"
#include "util/ralloc.h"
#include "util/u_dynarray.h"
#include "util/u_math.h"

#include "bifrost/valhall/disassemble.h"
#include "lib/pan_earlyzs.h"
#include "panfrost/model/pan_model.h"
#include "pan_compiler.h"
#include "pan_context.h" /* tools/mali/pan_shc-shim */
#include "pan_nir.h"

#include "pan_shc_build.h" /* PAN_SHC_YAKOGL_DESCRIBE, written by build_pan_shc.sh */

/*
 * The panfrost screen caps that decide what the state tracker does to a
 * shader, for arch >= 6 (pan_screen.c:810-822, 697, 747, 853, 862, 869;
 * u_screen.c for those panfrost leaves at the default).
 */
#define PAN_FS_POSITION_IS_SYSVAL	 true  /* pan_screen.c:821, arch >= 6 */
#define PAN_FS_POINT_IS_SYSVAL		 true  /* pan_screen.c:822 */
#define PAN_FS_FACE_IS_INTEGER_SYSVAL	 true  /* pan_screen.c:820 */
#define PAN_FS_COORD_ORIGIN_UPPER_LEFT	 true  /* pan_screen.c:816 */
#define PAN_FS_COORD_ORIGIN_LOWER_LEFT	 false /* pan_screen.c:814 */
#define PAN_FS_COORD_CENTER_INTEGER	 true  /* pan_screen.c:810, arch >= 6 */
#define PAN_FS_COORD_CENTER_HALF_INTEGER false /* pan_screen.c:811 */

/*
 * The linker's limits for nir_opt_varyings_bulk (gl_nir_linker.c:1501-1503,
 * the minimum over the linked stages), as st_extensions.c:231-262 derives them
 * from the panfrost caps: max_const_buffer0_size is 16 * 1024 floats
 * (pan_screen.c:576), less 4 * MAX_CLIP_PLANES (8) in the VS because
 * clip_planes is 0 (pan_screen.c:862); point_size_fixed is LOWER_NEVER below
 * arch 13 (pan_screen.c:697), so no point size slot is reserved; the FS's 4
 * for alpha_test (false for arch > 5, pan_screen.c:853) is a larger limit.
 * PAN_MAX_CONST_BUFFERS (16) constant buffers, the first one being the default
 * uniform block (pan_device.h:42).
 */
#define PAN_GL_MAX_UNIFORM_COMPONENTS (16 * 1024 - 4 * 8)
#define PAN_GL_MAX_UNIFORM_BLOCKS     (16 - 1)

static const nir_shader_compiler_options *nir_options[MESA_SHADER_COMPUTE + 1];

static uint32_t *read_words(const char *path, size_t *nwords)
{
	FILE *f = fopen(path, "rb");
	if (!f) {
		perror(path);
		return NULL;
	}
	fseek(f, 0, SEEK_END);
	long n = ftell(f);
	fseek(f, 0, SEEK_SET);
	uint32_t *w = malloc(n + 4);
	if (fread(w, 1, n, f) != (size_t)n) {
		perror("read");
		fclose(f);
		free(w);
		return NULL;
	}
	fclose(f);
	*nwords = n / 4;
	return w;
}

/* gl_nir_linker.c:45-130, gl_nir_opts(); in libglsl, which this tool does not
 * link, so transcribed (as v3d_shc does). */
static void gl_nir_opts(nir_shader *nir)
{
	bool progress;

	do {
		progress = false;

		NIR_PASS(_, nir, nir_lower_vars_to_ssa);
		NIR_PASS(progress, nir, nir_remove_dead_variables,
			 nir_var_function_temp | nir_var_shader_temp | nir_var_mem_shared, NULL);
		NIR_PASS(progress, nir, nir_opt_find_array_copies);
		NIR_PASS(progress, nir, nir_opt_copy_prop_vars);
		NIR_PASS(progress, nir, nir_opt_dead_write_vars);

		if (nir->options->lower_to_scalar) {
			NIR_PASS(_, nir, nir_lower_alu_to_scalar,
				 nir->options->lower_to_scalar_filter, NULL);
			NIR_PASS(_, nir, nir_lower_phis_to_scalar, NULL, NULL);
		}

		NIR_PASS(_, nir, nir_lower_alu);
		NIR_PASS(_, nir, nir_lower_pack);
		NIR_PASS(progress, nir, nir_opt_copy_prop);
		NIR_PASS(progress, nir, nir_opt_remove_phis);
		NIR_PASS(progress, nir, nir_opt_dce);

		bool opt_loop_progress = false;
		NIR_PASS(opt_loop_progress, nir, nir_opt_loop);
		if (opt_loop_progress) {
			progress = true;
			NIR_PASS(progress, nir, nir_opt_copy_prop);
			NIR_PASS(progress, nir, nir_opt_dce);
		}
		NIR_PASS(progress, nir, nir_opt_if, 0);
		NIR_PASS(progress, nir, nir_opt_dead_cf);
		NIR_PASS(progress, nir, nir_opt_cse);

		nir_opt_peephole_select_options peephole_select_options = {
			.limit = 8,
			.indirect_load_ok = true,
			.expensive_alu_ok = true,
		};
		NIR_PASS(progress, nir, nir_opt_peephole_select, &peephole_select_options);

		NIR_PASS(progress, nir, nir_opt_phi_precision);
		NIR_PASS(progress, nir, nir_opt_algebraic);
		NIR_PASS(progress, nir, nir_opt_constant_folding);

		NIR_PASS(progress, nir, nir_opt_undef);

		peephole_select_options = (nir_opt_peephole_select_options){
			.limit = 0,
			.discard_ok = true,
		};
		NIR_PASS(progress, nir, nir_opt_peephole_select, &peephole_select_options);
		if (nir->options->max_unroll_iterations ||
		    (nir->options->max_unroll_iterations_fp64 &&
		     (nir->options->lower_doubles_options & nir_lower_fp64_full_software)))
			NIR_PASS(progress, nir, nir_opt_loop_unroll);
	} while (progress);

	unsigned lower_flrp = (nir->options->lower_flrp16 ? 16 : 0) |
			      (nir->options->lower_flrp32 ? 32 : 0) |
			      (nir->options->lower_flrp64 ? 64 : 0);
	if (lower_flrp)
		NIR_PASS(progress, nir, nir_lower_flrp, lower_flrp, false);

	NIR_PASS(_, nir, nir_lower_var_copies);
}

static void gl_nir_opts_cb(nir_shader *nir, void *data)
{
	gl_nir_opts(nir);
}

/*
 * Stage 1, per shader: _mesa_spirv_to_nir(), glspirv.c:234-342. The
 * capabilities are spirv_to_nir's defaults (ES 2.0 needs none of the
 * optional ones) and there are no subgroups.
 */
static nir_shader *load_stage(const char *path, mesa_shader_stage stage)
{
	size_t nwords;
	uint32_t *spv = read_words(path, &nwords);
	if (!spv)
		return NULL;
	struct spirv_to_nir_options spirv_opts = {
		.environment = NIR_SPIRV_OPENGL,
		.ubo_addr_format = nir_address_format_32bit_index_offset,
		.ssbo_addr_format = nir_address_format_32bit_index_offset,
		.shared_addr_format = nir_address_format_32bit_offset,
	};
	nir_shader *nir = spirv_to_nir(spv, nwords, NULL, stage, "main", &spirv_opts,
				       nir_options[stage]);
	free(spv);
	if (!nir) {
		fprintf(stderr, "%s: spirv_to_nir failed\n", path);
		return NULL;
	}

	/* glspirv.c:303-311 with st_extensions.c:476-481: panfrost keeps
	 * gl_FragCoord, gl_PointCoord and gl_FrontFacing as system values
	 * (pan_screen.c:820-822), so only gl_PrimitiveID becomes an input. */
	const nir_lower_sysvals_to_varyings_options sysvals_to_varyings = {
		.frag_coord = !PAN_FS_POSITION_IS_SYSVAL,
		.point_coord = !PAN_FS_POINT_IS_SYSVAL,
		.front_face = !PAN_FS_FACE_IS_INTEGER_SYSVAL,
		.primitive_id = stage == MESA_SHADER_FRAGMENT,
	};
	NIR_PASS(_, nir, nir_lower_sysvals_to_varyings, &sysvals_to_varyings);

	/* glspirv.c:313-339 */
	NIR_PASS(_, nir, nir_lower_variable_initializers, nir_var_function_temp);
	NIR_PASS(_, nir, nir_lower_returns);
	NIR_PASS(_, nir, nir_inline_functions);
	NIR_PASS(_, nir, nir_opt_copy_prop);
	NIR_PASS(_, nir, nir_opt_deref);
	nir_remove_non_entrypoints(nir);
	NIR_PASS(_, nir, nir_lower_variable_initializers, ~0);
	NIR_PASS(_, nir, nir_split_var_copies);
	NIR_PASS(_, nir, nir_split_per_member_structs);
	NIR_PASS(_, nir, nir_lower_frexp);
	return nir;
}

/*
 * Stage 2, per shader: gl_nir_link_spirv() (gl_nir_linker.c:1529) up to the
 * varyings: remove_dead_varyings_pre_linking() (:1183, not SSO, so every
 * unused input and output goes), then prelink_lowering() (:1394) with
 * preprocess_shader() (:1310) for panfrost's options.
 *
 * Left out, each because an ES 2.0 shader cannot reach it:
 * gl_nir_lower_blend_equation_advanced (no framebuffer fetch), gl_nir_add_point_size
 * (PointSizeFixed is true: point_size_fixed is LOWER_NEVER, st_extensions.c:204),
 * gl_nir_zero_initialize_clip_distance (no gl_ClipDistance), gl_nir_lower_images
 * (no images; libglsl). nir_lower_mediump_vars does not run: panfrost sets
 * fp16 but not int16 (pan_screen.c:585-591, gl_nir_linker.c:1362-1365).
 * lower_to_scalar is not set in panfrost's options, so neither the
 * scalarisation (:1367-1374) nor nir_lower_load_const_to_scalar (:1429) runs.
 */
static void prelink_stage(nir_shader *nir)
{
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
	/* gl_nir_linker.c:1328-1346 */
	if (nir->info.stage == MESA_SHADER_VERTEX) {
		nir->info.next_stage = MESA_SHADER_FRAGMENT;
	} else {
		nir->info.prev_stage = MESA_SHADER_VERTEX;
		nir->info.prev_stage_has_xfb = false;
	}

	/* gl_nir_linker.c:1359-1360 */
	NIR_PASS(_, nir, nir_lower_global_vars_to_local);
	NIR_PASS(_, nir, nir_lower_var_copies);

	/* gl_nir_linker.c:1376, 1390 */
	NIR_PASS(_, nir, nir_opt_barrier_modes);
	NIR_PASS(_, nir, nir_opt_constant_folding);

	/* prelink_lowering(), gl_nir_linker.c:1443-1463: panfrost does not
	 * set compact_arrays, so the clip distances become vec4s */
	nir_opt_access_options opt_access_options = { .is_vulkan = false };
	NIR_PASS(_, nir, nir_opt_access, &opt_access_options);
	nir_gather_clip_cull_distance_sizes_from_vars(nir);
	if (!nir->options->compact_arrays) {
		NIR_PASS(_, nir, nir_lower_clip_cull_distance_to_vec4s);
		NIR_PASS(_, nir, nir_lower_tess_level_array_vars_to_vec);
	}
	NIR_PASS(_, nir, nir_merge_clip_cull_distance_vars);
}

/*
 * GLSL ES links varyings by name; glsl2pan.py numbers each stage's varyings on
 * its own (as glsl2vivante.py does), so give every FS input the location of
 * the VS output of the same name. A name with no VS output is a link error,
 * reported rather than guessed (v3d_shc.c link_varyings_by_name).
 */
static bool link_varyings_by_name(nir_shader *vs, nir_shader *fs)
{
	bool ok = true;
	nir_foreach_shader_in_variable(in, fs) {
		if (in->data.location < VARYING_SLOT_VAR0)
			continue;
		nir_variable *match = NULL;
		nir_foreach_shader_out_variable(out, vs) {
			if (out->name && in->name && strcmp(out->name, in->name) == 0)
				match = out;
		}
		if (!match) {
			fprintf(stderr, "link: FS input '%s' has no VS output of that name\n",
				in->name ? in->name : "(unnamed)");
			ok = false;
			continue;
		}
		in->data.location = match->data.location;
	}
	return ok;
}

/*
 * Stage 3, the program: gl_nir_lower_optimize_varyings() (gl_nir_linker.c:
 * 1480-1525, called at :1553), then the dead uniforms (:1555-1565; ES 2.0 has
 * no uniform blocks or initialisers, which can_remove_var would keep).
 */
static void link_program(nir_shader *vs, nir_shader *fs)
{
	nir_lower_io_passes(vs, true);
	nir_lower_io_passes(fs, true);
	nir_shader *shaders[2] = { vs, fs };
	nir_opt_varyings_bulk(shaders, 2, true, PAN_GL_MAX_UNIFORM_COMPONENTS,
			      PAN_GL_MAX_UNIFORM_BLOCKS, gl_nir_opts_cb, NULL);
	NIR_PASS(_, vs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
	NIR_PASS(_, fs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
}

/*
 * The default uniform block as st/mesa lays it out for panfrost, which sets
 * packed_uniforms (pan_screen.c:747, so PackedDriverUniformStorage,
 * st_context.c:524). gl_nir_link_uniforms() adds one parameter per leaf
 * vector (add_parameter, gl_nir_link_uniforms.c:733-790: an array or a matrix
 * gives one per element or column) of vector_elements dwords, packed with no
 * alignment (_mesa_add_parameter, prog_parameter.c:279-286: only a 64-bit
 * type is aligned, to 2 dwords, and ES 2.0 has none). A uniform's
 * driver_location is the ValueOffset of its first parameter
 * (st_nir_assign_uniform_locations, st_glsl_to_nir.cpp:135-173). A state
 * parameter (gl_FbWposYTransform) is added by st_nir_lower_wpos_ytransform
 * with _mesa_add_state_reference: 4 dwords, aligned to 4 (prog_parameter.c:
 * 477). Each stage has its own parameter list.
 */
struct param_list {
	unsigned values; /* NumParameterValues, in dwords */
};

static unsigned add_param(struct param_list *p, unsigned size, bool pad_and_align)
{
	unsigned off = p->values;
	if (pad_and_align)
		off = align(off, 4);
	p->values = off + (pad_and_align ? align(size, 4) : size);
	return off;
}

static void add_uniform_params(struct param_list *p, const struct glsl_type *type)
{
	if (glsl_type_is_struct_or_ifc(type)) {
		for (unsigned i = 0; i < glsl_get_length(type); i++)
			add_uniform_params(p, glsl_get_struct_field(type, i));
		return;
	}
	if (glsl_type_is_array(type) && glsl_type_is_struct_or_ifc(glsl_without_array(type))) {
		for (unsigned i = 0; i < glsl_get_aoa_size(type); i++)
			add_uniform_params(p, glsl_without_array(type));
		return;
	}
	unsigned n = MAX2(glsl_get_aoa_size(type), 1) *
		     glsl_get_matrix_columns(glsl_without_array(type));
	for (unsigned i = 0; i < n; i++)
		add_param(p, glsl_get_vector_elements(glsl_without_array(type)), false);
}

/* st_glsl_to_nir.cpp:175-209, filter_64_bit_instr() */
static bool def_is_64bit(nir_def *def, void *state)
{
	bool *lower = (bool *)state;
	if (def && def->bit_size == 64) {
		*lower = true;
		return false;
	}
	return true;
}

static bool src_is_64bit(nir_src *src, void *state)
{
	bool *lower = (bool *)state;
	if (src && nir_src_bit_size(*src) == 64) {
		*lower = true;
		return false;
	}
	return true;
}

static bool filter_64_bit_instr(const nir_instr *const_instr, const void *data)
{
	bool lower = false;
	nir_instr *instr = (nir_instr *)const_instr;
	nir_foreach_def(instr, def_is_64bit, &lower);
	if (lower)
		return true;
	nir_foreach_src(instr, src_is_64bit, &lower);
	return lower;
}

/*
 * Stage 4, per shader: the rest of st_link_glsl_to_nir() (st_glsl_to_nir.cpp:
 * 523-560) and st_glsl_to_nir_post_opts() (:236-381).
 *
 * Left out because ES 2.0 cannot reach them, and they live in libglsl or
 * st/mesa: the indirect lowering (:534-547; panfrost sets indirect_temp_addr
 * and indirect_const_addr for arch >= 6, pan_screen.c:579-580),
 * gl_nir_lower_buffers (:552, no UBOs or SSBOs), st_nir_lower_builtin (:283,
 * not for SPIR-V nor packed storage), gl_nir_lower_atomics (:287, no atomics).
 */
static bool postlink_stage(nir_shader *nir, struct param_list *params)
{
	/* st_glsl_to_nir.cpp:529 */
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	/* The user uniforms' parameters, which gl_nir_link_uniforms fills in
	 * its variable order. That order is the declaration order for a GLSL
	 * program (the GL path the board's driver takes for GLSL ES 1.00) and
	 * not for spirv_to_nir's variables, so the uniforms are taken in
	 * location order, which glsl2pan.py gives in declaration order (as
	 * v3d_shc.c assign_uniform_locations does). Checked against Mesa's GL
	 * driver on drm-shim with a mat4 and a float (tools/mali/README-pan_shc.md). */
	unsigned uniforms = 0, placed = 0;
	nir_foreach_variable_with_modes(var, nir, nir_var_uniform)
		uniforms += !glsl_contains_opaque(var->type) && !var->state_slots;
	for (int loc = 0; loc < 4096; loc++) {
		nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
			if (glsl_contains_opaque(var->type) || var->state_slots ||
			    var->data.location != loc)
				continue;
			var->data.driver_location = params->values;
			add_uniform_params(params, var->type);
			placed++;
		}
	}
	if (placed != uniforms) {
		fprintf(stderr, "pan_shc: %u of %u uniforms have no location in 0..4095 "
				"(glsl2pan.py gives every uniform one)\n", uniforms - placed, uniforms);
		return false;
	}

	/* st_nir_lower_wpos_ytransform(), st_glsl_to_nir.cpp:393-435, with
	 * panfrost's caps: upper-left origin and integer pixel centres
	 * (pan_screen.c:810-816), while GL's are lower left and half integer, so
	 * a gl_FragCoord read becomes a function of gl_FbWposYTransform. */
	if (nir->info.stage == MESA_SHADER_FRAGMENT) {
		static const gl_state_index16 wpos_state[STATE_LENGTH] = {
			STATE_FB_WPOS_Y_TRANSFORM
		};
		nir_lower_wpos_ytransform_options wpos_options = {
			.fs_coord_origin_upper_left = PAN_FS_COORD_ORIGIN_UPPER_LEFT,
			.fs_coord_origin_lower_left = PAN_FS_COORD_ORIGIN_LOWER_LEFT,
			.fs_coord_pixel_center_integer = PAN_FS_COORD_CENTER_INTEGER,
			.fs_coord_pixel_center_half_integer = PAN_FS_COORD_CENTER_HALF_INTEGER,
		};
		memcpy(wpos_options.state_tokens, wpos_state, sizeof(wpos_options.state_tokens));
		bool wpos = false, pntc = false;
		NIR_PASS(wpos, nir, nir_lower_wpos_ytransform, &wpos_options);
		static const gl_state_index16 pntc_state[1][STATE_LENGTH] = {
			{ STATE_FB_PNTC_Y_TRANSFORM }
		};
		NIR_PASS(pntc, nir, nir_lower_pntc_ytransform, pntc_state);
		/* _mesa_add_state_reference (:421, :430); the variable's
		 * driver_location is the parameter's ValueOffset
		 * (st_update_state_param_locations, :116-129) */
		unsigned wpos_off = wpos ? add_param(params, 4, true) : 0;
		unsigned pntc_off = pntc ? add_param(params, 4, true) : 0;
		nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
			if (!var->state_slots)
				continue;
			if (var->state_slots[0].tokens[0] == STATE_FB_WPOS_Y_TRANSFORM)
				var->data.driver_location = wpos_off;
			else if (var->state_slots[0].tokens[0] == STATE_FB_PNTC_Y_TRANSFORM)
				var->data.driver_location = pntc_off;
		}
	}

	/* st_glsl_to_nir.cpp:557-560 */
	struct nir_lower_compute_system_values_options cs_options = { 0 };
	NIR_PASS(_, nir, nir_lower_system_values);
	NIR_PASS(_, nir, nir_lower_compute_system_values, &cs_options);

	/* st_glsl_to_nir_post_opts(), :291-347: panfrost sets both
	 * lower_int64_options and lower_doubles_options (nir_lower_dmod) and not
	 * lower_to_scalar, so the 64-bit instructions and all phis are scalarised
	 * first; vectorize_vec2_16bit is set, so nothing is revectorised. */
	bool lowered_64bit_ops = false;
	bool revectorize = false;
	NIR_PASS(revectorize, nir, nir_lower_alu_to_scalar, filter_64_bit_instr, NULL);
	NIR_PASS(revectorize, nir, nir_lower_phis_to_scalar, NULL, NULL);
	NIR_PASS(lowered_64bit_ops, nir, nir_lower_frexp);
	NIR_PASS(lowered_64bit_ops, nir, nir_lower_doubles, NULL,
		 nir->options->lower_doubles_options);
	NIR_PASS(lowered_64bit_ops, nir, nir_lower_int64);
	if (revectorize || lowered_64bit_ops)
		gl_nir_opts(nir);

	/* :350, :352-364 (no atomic counters: max_hw_atomic_counters is 0 and
	 * nir_atomics_as_deref unset), :366, :369 */
	nir_remove_dead_variables(nir, nir_var_function_temp, NULL);
	NIR_PASS(_, nir, nir_lower_atomics_to_ssbo, 0);
	NIR_PASS(_, nir, nir_opt_intrinsics);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
	return true;
}

static unsigned st_packed_uniforms_type_size(const struct glsl_type *type, bool bindless)
{
	return glsl_count_dword_slots(type, bindless);
}

/*
 * Stage 5, per shader: the variant st/mesa makes for the first draw with a
 * default key (st_create_common_variant for the VS, st_program.c:793;
 * st_create_fp_variant for the FS, st_program.c:1050). panfrost does not set
 * call_finalize_nir_in_linker and has no finalize_nir, so st_finalize_nir
 * runs here (st_program.c:854, :1220), and nir_io_has_intrinsics is not set,
 * so the IO goes back to variables (:866-880, :1248-1255).
 */
static void create_variant(nir_shader *nir, const struct param_list *params)
{
	/* st_finalize_nir(), st_glsl_to_nir.cpp:723-761: texrect is supported
	 * (u_screen.c:163) and texture_gather_offsets is not, so only the tg4
	 * offsets are lowered */
	struct nir_lower_tex_options tex_opts = { .lower_tg4_offsets = true };
	NIR_PASS(_, nir, nir_lower_tex, &tex_opts);

	/* st_nir_assign_uniform_locations (:135-173): driver_location was set
	 * in postlink_stage(); the samplers take their GL binding */
	nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
		if (glsl_type_is_sampler(glsl_without_array(var->type)))
			var->data.driver_location = var->data.binding;
	}
	nir->num_uniforms = DIV_ROUND_UP(params->values, 4);

	/* st_nir_lower_uniforms(), :701-716: packed, so dword slots, then the
	 * uniforms become UBO 0 (lower_uniforms_to_ubo is set, NativeIntegers) */
	NIR_PASS(_, nir, nir_lower_io, nir_var_uniform, st_packed_uniforms_type_size,
		 (nir_lower_io_options)0);
	NIR_PASS(_, nir, nir_lower_uniforms_to_ubo, true, false);

	/* st_nir_lower_samplers(): gl_nir_lower_samplers = the binding as the
	 * index (gl_nir_lower_samplers_as_deref) and nir_lower_samplers */
	NIR_PASS(_, nir, nir_lower_samplers);

	/* st_program.c:871-879 (VS), 1252-1254 (FS) */
	NIR_PASS(_, nir, nir_opt_dce);
	NIR_PASS(_, nir, nir_unlower_io_to_vars, false);
	gl_nir_opts(nir);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

/* pan_shader.c:472-475 */
static unsigned glsl_type_size(const struct glsl_type *type, bool bindless)
{
	return glsl_count_attribute_slots(type, false);
}

/* What panfrost_create_shader_state() keeps of a shader (pan_shader.c:500-633). */
struct uncompiled {
	nir_shader *nir;
	bool fragcolor_lowered;
	uint32_t noperspective_varyings;
	struct pan_varying_layout vs_varying_layout;
};

/*
 * Stage 6: panfrost_create_shader_state(), pan_shader.c:500-633, for a VS or
 * FS without pixel local storage or transform feedback (ES 2.0 has neither).
 */
static void driver_create(struct uncompiled *so, nir_shader *nir, uint64_t gpu_id)
{
	memset(so, 0, sizeof(*so));
	so->nir = nir;

	/* PAN_SHC_PRINT_NIR=1: the NIR the driver is handed (pipe->create_*_state) */
	if (getenv("PAN_SHC_PRINT_NIR"))
		nir_print_shader(nir, stderr);

	/* :555-562 */
	if (nir->info.stage == MESA_SHADER_FRAGMENT &&
	    nir->info.outputs_written & BITFIELD_BIT(FRAG_RESULT_COLOR)) {
		NIR_PASS(_, nir, nir_lower_fragcolor, nir->info.fs.color_is_dual_source ? 1 : 8);
		so->fragcolor_lowered = true;
	}

	/* :565-577 */
	pan_preprocess_nir(nir, gpu_id);
	NIR_PASS(_, nir, nir_lower_indirect_derefs_to_if_else_trees,
		 nir_var_shader_in | nir_var_shader_out, UINT32_MAX);
	NIR_PASS(_, nir, nir_lower_io, nir_var_shader_in | nir_var_shader_out, glsl_type_size,
		 nir_lower_io_use_interpolated_input_intrinsics);
	NIR_PASS(_, nir, nir_opt_constant_folding);

	/* :579-592 */
	if (nir->info.stage == MESA_SHADER_FRAGMENT)
		so->noperspective_varyings = pan_nir_collect_noperspective_varyings_fs(nir);
	if (nir->info.stage == MESA_SHADER_VERTEX) {
		pan_varying_collect_formats(&so->vs_varying_layout, nir, gpu_id, false, false);
		pan_build_varying_layout_compact(&so->vs_varying_layout, nir, gpu_id);
	}
}

/* pan_shader.c:75-91 */
static bool lower_sample_mask_writes(nir_builder *b, nir_intrinsic_instr *intrin, void *data)
{
	if (intrin->intrinsic != nir_intrinsic_store_output)
		return false;
	if (nir_intrinsic_io_semantics(intrin).location != FRAG_RESULT_SAMPLE_MASK)
		return false;
	b->cursor = nir_before_instr(&intrin->instr);
	nir_def *orig = nir_load_sample_mask(b);
	nir_def *new = nir_bcsel_pan(b, nir_load_multisampled_pan(b), intrin->src[0].ssa, orig);
	nir_src_rewrite(&intrin->src[0], new);
	return true;
}

/* The parts of struct panfrost_shader_key (pan_context.h) an ES 2.0 draw can set. */
struct shader_key {
	uint32_t vs_noperspective_varyings;
	unsigned fs_nr_cbufs_for_fragcolor;
	unsigned fs_sprite_coord_enable;
	bool fs_line_smooth;
	bool fs_use_vs_layout;
};

struct compiled {
	struct util_dynarray binary;
	struct pan_shader_info info;
	struct panfrost_sysvals sysvals;
	struct pan_earlyzs_lut earlyzs;
};

/*
 * Stage 7: panfrost_shader_compile(), pan_shader.c:95-239, the variant
 * panfrost_link_shader() (:660-690, the FS with the VS's varying layout) and
 * panfrost_update_shader_variant() (the VS) compile, then pan_earlyzs_analyze
 * (panfrost_new_variant_locked, :389).
 */
static bool driver_compile(const struct uncompiled *so, const struct shader_key *key,
			   const struct pan_varying_layout *varying_layout, uint64_t gpu_id,
			   uint32_t gpu_variant, struct compiled *out)
{
	const unsigned arch = pan_arch(gpu_id);
	nir_shader *s = nir_shader_clone(NULL, so->nir);

	struct pan_compile_inputs inputs = {
		.gpu_id = gpu_id,
		.gpu_variant = gpu_variant,
	};

	/* :122-131: no transform feedback in ES 2.0, so IDVS stays on */
	if (s->info.stage == MESA_SHADER_VERTEX)
		inputs.no_idvs = s->info.has_transform_feedback_varyings;

	/* :134-137 */
	inputs.trust_varying_flat_highp_types = false;
	inputs.varying_layout = varying_layout;

	if (s->info.stage == MESA_SHADER_FRAGMENT) {
		/* :140-143 */
		if (key->fs_nr_cbufs_for_fragcolor)
			NIR_PASS(_, s, panfrost_nir_remove_fragcolor_stores,
				 key->fs_nr_cbufs_for_fragcolor);
		/* :145-151 */
		if (key->fs_sprite_coord_enable) {
			NIR_PASS(_, s, nir_lower_texcoord_replace_late, key->fs_sprite_coord_enable,
				 true);
			NIR_PASS(_, s, pan_nir_lower_var_special_pan);
		}
		/* :153-156: clip_plane_enable is 0 (ES 2.0 has no user clip
		 * planes); :158-164: line_smooth is never set in GLES */
		if (key->fs_line_smooth) {
			fprintf(stderr, "pan_shc: line smoothing is not a GLES state\n");
			return false;
		}
		/* :166-167 */
		NIR_PASS(_, s, nir_shader_intrinsics_pass, lower_sample_mask_writes,
			 nir_metadata_control_flow, NULL);
	}

	/* :180 (the arch <= 5 framebuffer lowering at :173-177 is Midgard's) */
	NIR_PASS(_, s, panfrost_nir_lower_res_indices, inputs.gpu_id);

	/* :182 */
	pan_postprocess_nir(s, &inputs, &out->info);

	/* :184-188 */
	if (s->info.stage == MESA_SHADER_VERTEX)
		NIR_PASS(_, s, nir_inline_sysval, nir_intrinsic_load_noperspective_varyings_pan,
			 key->vs_noperspective_varyings);

	/* :194 */
	NIR_PASS(_, s, panfrost_nir_lower_sysvals, arch, &out->sysvals);

	/* :196-207 */
	inputs.fau.pushable_ubos = BITFIELD_BIT(0);
	if (out->sysvals.sysval_count != 0)
		inputs.fau.pushable_ubos |= BITFIELD_BIT(PAN_UBO_SYSVALS);
	inputs.fau.promote_immediates = true;
	/* :209-213 */
	if (arch >= 9)
		inputs.robust_descriptors = true;

	/* :215-216: screen->vtbl.compile_shader is pan_shader_compile
	 * (pan_cmdstream.c:4806) */
	util_dynarray_init(&out->binary, NULL);
	pan_shader_compile(s, &inputs, &out->binary, &out->info);
	ralloc_free(s);

	out->earlyzs = pan_earlyzs_analyze(&out->info, arch);
	return out->binary.size > 0 || so->nir->info.stage == MESA_SHADER_VERTEX;
}

/* ---- printing ---------------------------------------------------------- */

static const char *b2s(bool b)
{
	return b ? "true" : "false";
}

static const char *sysval_name(unsigned type)
{
	switch (type) {
	case PAN_SYSVAL_VIEWPORT_SCALE: return "VIEWPORT_SCALE";
	case PAN_SYSVAL_VIEWPORT_OFFSET: return "VIEWPORT_OFFSET";
	case PAN_SYSVAL_TEXTURE_SIZE: return "TEXTURE_SIZE";
	case PAN_SYSVAL_SSBO: return "SSBO";
	case PAN_SYSVAL_NUM_WORK_GROUPS: return "NUM_WORK_GROUPS";
	case PAN_SYSVAL_SAMPLER: return "SAMPLER";
	case PAN_SYSVAL_LOCAL_GROUP_SIZE: return "LOCAL_GROUP_SIZE";
	case PAN_SYSVAL_WORK_DIM: return "WORK_DIM";
	case PAN_SYSVAL_IMAGE_SIZE: return "IMAGE_SIZE";
	case PAN_SYSVAL_SAMPLE_POSITIONS: return "SAMPLE_POSITIONS";
	case PAN_SYSVAL_MULTISAMPLED: return "MULTISAMPLED";
	case PAN_SYSVAL_RT_CONVERSION: return "RT_CONVERSION";
	case PAN_SYSVAL_VERTEX_INSTANCE_OFFSETS: return "VERTEX_INSTANCE_OFFSETS";
	case PAN_SYSVAL_DRAWID: return "DRAWID";
	case PAN_SYSVAL_BLEND_CONSTANTS: return "BLEND_CONSTANTS";
	case PAN_SYSVAL_XFB: return "XFB";
	case PAN_SYSVAL_NUM_VERTICES: return "NUM_VERTICES";
	case PAN_SYSVAL_PRINTF_BUFFER: return "PRINTF_BUFFER";
	case PAN_SYSVAL_IMAGE_SAMPLES: return "IMAGE_SAMPLES";
	default: return NULL;
	}
}

static const char *section_name(enum pan_varying_section s)
{
	switch (s) {
	case PAN_VARYING_SECTION_POSITION: return "position";
	case PAN_VARYING_SECTION_ATTRIBS: return "attribs";
	case PAN_VARYING_SECTION_SPECIAL: return "special";
	case PAN_VARYING_SECTION_GENERIC: return "generic";
	default: return "?";
	}
}

static void print_varying_layout(const char *key, const struct pan_varying_layout *l)
{
	printf("\"%s\": { \"count\": %u, \"format_known\": %s, \"layout_known\": %s, "
	       "\"generic_size_B\": %u, \"slots\": [",
	       key, l->count, b2s(l->known & PAN_VARYING_FORMAT_KNOWN),
	       b2s(l->known & PAN_VARYING_LAYOUT_KNOWN), l->generic_size_B);
	bool first = true;
	for (unsigned i = 0; i < l->count; i++) {
		const struct pan_varying_slot *s = &l->slots[i];
		if (pan_varying_slot_is_empty(s))
			continue;
		printf("%s{ \"index\": %u, \"location\": \"%s\", \"alu_type\": \"0x%x\", "
		       "\"ncomps\": %u, \"section\": \"%s\", \"offset\": %d }",
		       first ? "" : ", ", i, gl_varying_slot_name_for_stage(s->location,
									   MESA_SHADER_VERTEX),
		       s->alu_type, s->ncomps, section_name(s->section), s->offset);
		first = false;
	}
	printf("] }");
}

/* the uniform names of a stage, by UBO 0 dword (driver_location) */
struct uniform_name {
	char *name;
	unsigned dword, dwords;
	int binding; /* >= 0 for a sampler */
	const char *state;
};
struct uniform_names {
	struct uniform_name u[64];
	unsigned n;
};

static void collect_uniforms(nir_shader *nir, struct uniform_names *un)
{
	un->n = 0;
	nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
		if (un->n == ARRAY_SIZE(un->u))
			break;
		struct uniform_name *u = &un->u[un->n++];
		u->name = strdup(var->name ? var->name : "");
		u->state = NULL;
		if (glsl_type_is_sampler(glsl_without_array(var->type))) {
			u->binding = var->data.binding;
			u->dword = u->dwords = 0;
			continue;
		}
		u->binding = -1;
		u->dword = var->data.driver_location;
		u->dwords = glsl_count_dword_slots(var->type, false);
		if (var->state_slots) {
			gl_state_index16 t = var->state_slots[0].tokens[0];
			u->state = t == STATE_FB_WPOS_Y_TRANSFORM   ? "FB_WPOS_Y_TRANSFORM"
				   : t == STATE_FB_PNTC_Y_TRANSFORM ? "FB_PNTC_Y_TRANSFORM"
								    : "unknown";
		}
	}
}

static void print_uniforms(const struct uniform_names *un)
{
	printf("\"uniforms\": [");
	for (unsigned i = 0; i < un->n; i++) {
		const struct uniform_name *u = &un->u[i];
		if (u->binding >= 0)
			printf("%s{ \"name\": \"%s\", \"sampler_binding\": %d }", i ? ", " : "",
			       u->name, u->binding);
		else
			printf("%s{ \"name\": \"%s\", \"ubo\": 0, \"dword\": %u, \"dwords\": %u%s%s%s }",
			       i ? ", " : "", u->name, u->dword, u->dwords,
			       u->state ? ", \"state\": \"" : "", u->state ? u->state : "",
			       u->state ? "\"" : "");
	}
	printf("]");
}

/* What a pushed UBO word holds: a sysval (UBO 1) or a user uniform (UBO 0). */
static void describe_ubo_word(const struct compiled *c, const struct uniform_names *un,
			      unsigned ubo, unsigned offset)
{
	if (ubo == PAN_UBO_SYSVALS) {
		unsigned idx = offset / 16;
		if (idx < c->sysvals.sysval_count) {
			unsigned sv = c->sysvals.sysvals[idx];
			const char *nm = sysval_name(PAN_SYSVAL_TYPE(sv));
			printf(", \"sysval\": \"%s\", \"sysval_id\": %u, \"component\": %u",
			       nm ? nm : "?", PAN_SYSVAL_ID(sv), (offset % 16) / 4);
		}
		return;
	}
	if (ubo == 0) {
		unsigned dw = offset / 4;
		for (unsigned i = 0; i < un->n; i++) {
			const struct uniform_name *u = &un->u[i];
			if (u->binding < 0 && dw >= u->dword && dw < u->dword + u->dwords) {
				printf(", \"uniform\": \"%s\", \"uniform_dword\": %u", u->name,
				       dw - u->dword);
				return;
			}
		}
	}
}

static void print_fau(const struct compiled *c, const struct uniform_names *un)
{
	const struct pan_fau_layout *f = &c->info.fau;
	printf("\"fau\": { \"reserved\": %u, \"count\": %u, \"max\": %u, \"words\": [",
	       f->reserved, f->count, f->max);
	for (unsigned i = f->reserved; i < f->count; i++) {
		printf("%s{ \"word\": %u, \"fau_slot\": %u, \"half\": %u", i == f->reserved ? "" : ", ",
		       i, i / 2, i % 2);
		if (BITSET_TEST(f->is_const, i)) {
			printf(", \"constant\": \"0x%08x\" }", f->words[i].constant);
		} else {
			printf(", \"ubo\": %u, \"offset\": %u", f->words[i].relocation.ubo,
			       f->words[i].relocation.offset);
			describe_ubo_word(c, un, f->words[i].relocation.ubo,
					  f->words[i].relocation.offset);
			printf(" }");
		}
	}
	printf("] }");
}

static void print_sysvals(const struct panfrost_sysvals *sv)
{
	printf("\"sysvals\": [");
	for (unsigned i = 0; i < sv->sysval_count; i++) {
		const char *nm = sysval_name(PAN_SYSVAL_TYPE(sv->sysvals[i]));
		printf("%s{ \"ubo\": %u, \"offset\": %u, \"sysval\": \"%s\", \"id\": %u }",
		       i ? ", " : "", PAN_UBO_SYSVALS, i * 16, nm ? nm : "?",
		       PAN_SYSVAL_ID(sv->sysvals[i]));
	}
	printf("]");
}

struct region {
	const char *name;
	unsigned offset, size;
};

static void print_binary(const struct util_dynarray *bin, const struct region *r, unsigned nr)
{
	printf("\"binary_size\": %u, \"regions\": [", bin->size);
	for (unsigned i = 0; i < nr; i++)
		printf("%s{ \"name\": \"%s\", \"offset\": %u, \"size\": %u }", i ? ", " : "",
		       r[i].name, r[i].offset, r[i].size);
	printf("],\n    \"code\": [");
	const uint64_t *w = bin->data;
	for (unsigned i = 0; i < bin->size / 8; i++)
		printf("%s\"0x%016" PRIx64 "\"", i ? ", " : "", w[i]);
	printf("]");
}

/* disassemble_valhall's verbose form prints each instruction's bytes, as
 * pandecode does, and stops at the first all-zero word (the padding). */
static void disasm(const char *stage, const struct util_dynarray *bin, const struct region *r,
		   unsigned nr)
{
	for (unsigned i = 0; i < nr; i++) {
		fprintf(stderr, "--- %s %s: offset %u, %u bytes\n", stage, r[i].name, r[i].offset,
			r[i].size);
		disassemble_valhall(stderr, (const uint8_t *)bin->data + r[i].offset, r[i].size,
				    true);
	}
}

/*
 * The program regions of a VS binary (bifrost_compile.c:4660-4752): the
 * position shader at 0; when the VS writes gl_PointSize, the same position
 * shader without the point size store at no_psiz_offset (packed right after
 * it, :4749); the varying shader at secondary_offset (packed last). Each
 * region ends where the next starts.
 */
static unsigned vs_regions(const struct compiled *c, struct region *r)
{
	unsigned n = 0, size = c->binary.size;
	const struct pan_shader_info *i = &c->info;
	unsigned ends[3] = { size, size, size };
	unsigned starts[3] = { 0, i->vs.no_psiz_offset, i->vs.secondary_offset };
	bool present[3] = { true, i->vs.no_psiz_offset != 0,
			    i->vs.idvs && i->vs.secondary_enable };
	for (unsigned a = 0; a < 3; a++)
		for (unsigned b = 0; b < 3; b++)
			if (present[a] && present[b] && starts[b] > starts[a] && starts[b] < ends[a])
				ends[a] = starts[b];
	const char *names[3] = { i->vs.idvs ? "position" : "vertex", "position_no_psiz",
				 "varying" };
	for (unsigned a = 0; a < 3; a++)
		if (present[a])
			r[n++] = (struct region){ names[a], starts[a], ends[a] - starts[a] };
	return n;
}

static void print_common(const struct compiled *c)
{
	const struct pan_shader_info *i = &c->info;
	printf("\"work_reg_count\": %u, \"preload\": \"0x%016" PRIx64 "\", \"tls_size\": %u, "
	       "\"ubo_count\": %u, \"ubo_mask\": \"0x%x\", \"sampler_count\": %u, "
	       "\"texture_count\": %u, \"ftz_fp16\": %s, \"ftz_fp32\": %s, "
	       "\"contains_barrier\": %s,\n    ",
	       i->work_reg_count, i->preload, i->tls_size, i->ubo_count, i->ubo_mask,
	       i->sampler_count, i->texture_count, b2s(i->ftz_fp16), b2s(i->ftz_fp32),
	       b2s(i->contains_barrier));
}

static void print_earlyzs(const struct pan_earlyzs_lut *lut)
{
	/* pan_earlyzs.h: states[writes_zs_or_oq][alpha_to_coverage]
	 * [zs_always_passes][zs_tilebuf_read]; update and kill are
	 * enum pan_earlyzs (0 early, 1 weak early, 2 late) */
	printf("\"earlyzs\": [");
	bool first = true;
	for (unsigned a = 0; a < 2; a++)
		for (unsigned b = 0; b < 2; b++)
			for (unsigned c = 0; c < 2; c++)
				for (unsigned d = 0; d < PAN_EARLYZS_ZS_TILEBUF_MODE_COUNT; d++) {
					struct pan_earlyzs_state s = lut->states[a][b][c][d];
					printf("%s{ \"writes_zs_or_oq\": %u, \"alpha_to_coverage\": %u, "
					       "\"zs_always_passes\": %u, \"zs_tilebuf_read\": %u, "
					       "\"update\": %u, \"kill\": %u, \"shader_readonly_zs\": %s }",
					       first ? "" : ", ", a, b, c, d, s.update, s.kill,
					       b2s(s.shader_readonly_zs));
					first = false;
				}
	printf("]");
}

static int usage(void)
{
	fprintf(stderr, "usage: pan_shc vs.spv fs.spv [--gpu-id HEX] [--gpu-variant N] "
			"[--prim triangles|lines|points]\n");
	return 2;
}

int main(int argc, char **argv)
{
	/* Every instrument here says what it is before what it measured. */
	printf("pan_shc: build %s mesa %s\n", PAN_SHC_YAKOGL_DESCRIBE, PACKAGE_VERSION);
	fflush(stdout);

	if (argc < 3)
		return usage();

	/* The GPU: GPU_ID as panthor reports it (DRM_PANTHOR_DEV_QUERY_GPU_INFO
	 * gpu_id). 0xa8670005 is the Mali-G610 of the RK3588 (arch 10,
	 * pan_model.c:88). gpu_variant is core_features & 0xff
	 * (panthor_kmod.c:162); the compiler only reads it to pick the model for
	 * its statistics (bifrost_compile.c:4038-4045, 4943), so it does not
	 * change a binary. */
	uint64_t gpu_id = 0xa8670005;
	uint32_t gpu_variant = 0;
	const char *variant_source = "assumed";
	const char *prim = "triangles";
	for (int i = 3; i < argc; i++) {
		if (strcmp(argv[i], "--gpu-id") == 0 && i + 1 < argc) {
			gpu_id = strtoull(argv[++i], NULL, 16);
		} else if (strcmp(argv[i], "--gpu-variant") == 0 && i + 1 < argc) {
			gpu_variant = strtoul(argv[++i], NULL, 0);
			variant_source = "given";
		} else if (strcmp(argv[i], "--prim") == 0 && i + 1 < argc) {
			prim = argv[++i];
		} else {
			fprintf(stderr, "pan_shc: unknown argument '%s'\n", argv[i]);
			return usage();
		}
	}
	const bool points = strcmp(prim, "points") == 0;
	const bool lines = strcmp(prim, "lines") == 0;
	if (!points && !lines && strcmp(prim, "triangles") != 0) {
		fprintf(stderr, "pan_shc: --prim must be triangles, lines or points, not '%s'\n",
			prim);
		return 2;
	}
	const unsigned arch = pan_arch(gpu_id);
	if (arch < 9) {
		fprintf(stderr, "pan_shc: gpu id 0x%" PRIx64 " is arch %u; this tool prints "
				"Valhall (arch >= 9) programs only\n", gpu_id, arch);
		return 2;
	}
	const struct pan_model *model = pan_get_model(gpu_id, gpu_variant);

	glsl_type_singleton_init_or_ref();
	/* pan_screen.c:1142-1144 */
	for (unsigned s = 0; s <= MESA_SHADER_COMPUTE; s++)
		nir_options[s] = pan_get_nir_shader_compiler_options(arch, s, false);

	nir_shader *vs = load_stage(argv[1], MESA_SHADER_VERTEX);
	nir_shader *fs = load_stage(argv[2], MESA_SHADER_FRAGMENT);
	if (!vs || !fs)
		return 1;
	prelink_stage(vs);
	prelink_stage(fs);
	if (!link_varyings_by_name(vs, fs))
		return 1;

	/* the attributes as declared, read while they are still variables
	 * (as v3d_shc does) */
	struct {
		char *name;
		int location;
		unsigned slots;
	} attrs[PAN_MAX_ATTRIBUTE];
	unsigned num_attrs = 0;
	nir_foreach_shader_in_variable(var, vs) {
		if (num_attrs == ARRAY_SIZE(attrs)) {
			fprintf(stderr, "more than %u vertex attributes\n", num_attrs);
			return 1;
		}
		attrs[num_attrs].name = strdup(var->name ? var->name : "");
		attrs[num_attrs].location = var->data.location - VERT_ATTRIB_GENERIC0;
		attrs[num_attrs].slots = glsl_count_attribute_slots(var->type, false);
		num_attrs++;
	}

	link_program(vs, fs);
	struct param_list vs_params = { 0 }, fs_params = { 0 };
	if (!postlink_stage(vs, &vs_params) || !postlink_stage(fs, &fs_params))
		return 1;
	create_variant(vs, &vs_params);
	create_variant(fs, &fs_params);

	/* the names are read before the driver lowers the variables away */
	struct uniform_names vs_un, fs_un;
	collect_uniforms(vs, &vs_un);
	collect_uniforms(fs, &fs_un);
	int attr_index[PAN_MAX_ATTRIBUTE][4];
	for (unsigned a = 0; a < num_attrs; a++)
		for (unsigned c = 0; c < attrs[a].slots && c < 4; c++) {
			attr_index[a][c] = -1;
			nir_foreach_shader_in_variable(var, vs) {
				if (var->data.location ==
				    (int)(VERT_ATTRIB_GENERIC0 + attrs[a].location + c))
					attr_index[a][c] = var->data.driver_location;
			}
		}

	struct uncompiled vso, fso;
	driver_create(&vso, vs, gpu_id);
	driver_create(&fso, fs, gpu_id);

	/* panfrost_link_shader (pan_shader.c:660-690): the FS takes the VS's
	 * varying layout, and its variant uses the default key
	 * (panfrost_default_shader_key, :478-497): one colour buffer for
	 * gl_FragColor. The draw's key (panfrost_build_fs_key, :308-352) is the
	 * same for ES 2.0 on one colour buffer, except for points:
	 * sprite_coord_enable is the rasterizer's, which st/mesa sets to
	 * CoordReplace (0 in GLES 2, points.c:235) because panfrost sets
	 * tgsi_texcoord (pan_screen.c:817, st_atom_rasterizer.c:217-223); and
	 * line_smooth, which GLES does not have. So --prim does not change the
	 * FS key; it says which VS region the draw runs (below). */
	fso.vs_varying_layout = vso.vs_varying_layout;
	struct shader_key fs_key = {
		.fs_nr_cbufs_for_fragcolor = fso.fragcolor_lowered ? 1 : 0,
		.fs_sprite_coord_enable = 0,
		.fs_line_smooth = false,
		.fs_use_vs_layout = true,
	};
	/* panfrost_build_vs_key (:297-305) */
	struct shader_key vs_key = { .vs_noperspective_varyings = fso.noperspective_varyings };

	struct compiled vsc, fsc;
	memset(&vsc, 0, sizeof(vsc));
	memset(&fsc, 0, sizeof(fsc));
	/* :260-264: the VS and a FS whose key knows the layout use it */
	if (!driver_compile(&vso, &vs_key, &vso.vs_varying_layout, gpu_id, gpu_variant, &vsc)) {
		fprintf(stderr, "pan_shc: VS compile made no binary\n");
		return 1;
	}
	if (!driver_compile(&fso, &fs_key,
			    fso.vs_varying_layout.known ? &fso.vs_varying_layout : NULL, gpu_id,
			    gpu_variant, &fsc)) {
		fprintf(stderr, "pan_shc: FS compile made no binary\n");
		return 1;
	}

	struct region vr[3], fr[1] = { { "fragment", 0, fsc.binary.size } };
	unsigned nvr = vs_regions(&vsc, vr);

	/* Which VS region a draw of --prim runs as its position shader: the
	 * driver packs three program descriptors, the position shader, the
	 * same at no_psiz_offset (0 when the VS does not write gl_PointSize)
	 * and the varying shader (pan_cmdstream.c:4502-4555), and a draw that
	 * is not of points takes the second (panfrost_get_position_shader,
	 * pan_cmdstream.h:238-249). */
	const char *pos_region =
		(vsc.info.vs.no_psiz_offset && !points) ? "position_no_psiz" : vr[0].name;

	printf("{\n  \"gpu\": { \"gpu_id\": \"0x%" PRIx64 "\", \"arch\": %u, \"model\": \"%s\", "
	       "\"gpu_variant\": %u, \"gpu_variant_source\": \"%s\", \"prim\": \"%s\" },\n",
	       gpu_id, arch, model ? model->name : "unknown", gpu_variant, variant_source, prim);

	printf("  \"attributes\": [");
	for (unsigned a = 0; a < num_attrs; a++) {
		printf("%s{ \"name\": \"%s\", \"location\": %d, \"records\": [", a ? ", " : "",
		       attrs[a].name, attrs[a].location);
		for (unsigned c = 0; c < attrs[a].slots && c < 4; c++) {
			if (attr_index[a][c] < 0)
				printf("%snull", c ? ", " : "");
			else
				printf("%s{ \"index\": %d, \"handle\": \"0x%08x\" }", c ? ", " : "",
				       attr_index[a][c],
				       pan_res_handle(PAN_TABLE_ATTRIBUTE, attr_index[a][c]));
		}
		printf("] }");
		free(attrs[a].name);
	}
	printf("],\n");

	/* VS */
	const struct pan_shader_info *vi = &vsc.info;
	printf("  \"vs\": { ");
	print_common(&vsc);
	printf("\"idvs\": %s, \"secondary_enable\": %s, \"secondary_offset\": %u, "
	       "\"secondary_work_reg_count\": %u, \"secondary_preload\": \"0x%016" PRIx64 "\", "
	       "\"no_psiz_offset\": %u, \"writes_point_size\": %s, \"needs_extended_fifo\": %s, "
	       "\"draw_position_region\": \"%s\", \"attribute_count\": %u, "
	       "\"attributes_read\": \"0x%x\",\n    ",
	       b2s(vi->vs.idvs), b2s(vi->vs.secondary_enable), vi->vs.secondary_offset,
	       vi->vs.secondary_work_reg_count, vi->vs.secondary_preload, vi->vs.no_psiz_offset,
	       b2s(vi->vs.writes_point_size), b2s(vi->vs.needs_extended_fifo), pos_region,
	       vi->attribute_count, vi->attributes_read);
	print_varying_layout("varying_layout", &vso.vs_varying_layout);
	printf(",\n    ");
	print_uniforms(&vs_un);
	printf(",\n    ");
	print_sysvals(&vsc.sysvals);
	printf(",\n    ");
	print_fau(&vsc, &vs_un);
	printf(",\n    ");
	print_binary(&vsc.binary, vr, nvr);
	printf(" },\n");

	/* FS */
	const struct pan_shader_info *fi = &fsc.info;
	printf("  \"fs\": { ");
	print_common(&fsc);
	printf("\"reads_frag_coord\": %s, \"reads_point_coord\": %s, \"reads_primitive_id\": %s, "
	       "\"reads_face\": %s, \"can_discard\": %s, \"writes_depth\": %s, "
	       "\"writes_stencil\": %s, \"writes_coverage\": %s, \"sidefx\": %s, "
	       "\"sample_shading\": %s, \"early_fragment_tests\": %s, \"can_early_z\": %s, "
	       "\"can_fpk\": %s, \"untyped_color_outputs\": %s, \"outputs_read\": \"0x%x\", "
	       "\"outputs_written\": \"0x%" PRIx64 "\",\n    ",
	       b2s(fi->fs.reads_frag_coord), b2s(fi->fs.reads_point_coord),
	       b2s(fi->fs.reads_primitive_id), b2s(fi->fs.reads_face), b2s(fi->fs.can_discard),
	       b2s(fi->fs.writes_depth), b2s(fi->fs.writes_stencil), b2s(fi->fs.writes_coverage),
	       b2s(fi->fs.sidefx), b2s(fi->fs.sample_shading), b2s(fi->fs.early_fragment_tests),
	       b2s(fi->fs.can_early_z), b2s(fi->fs.can_fpk), b2s(fi->fs.untyped_color_outputs),
	       fi->fs.outputs_read, fi->outputs_written);
	printf("\"hsr\": { \"ld_tile\": %s, \"wait_or_tile_access_before_atest_zsemit\": %s, "
	       "\"rasterizer_coverage_read\": %s, \"centroid_interpolation\": %s, "
	       "\"varying_before_atest_zsemit\": %s },\n    ",
	       b2s(fi->fs.hsr.ld_tile), b2s(fi->fs.hsr.wait_or_tile_access_before_atest_zsemit),
	       b2s(fi->fs.hsr.rasterizer_coverage_read), b2s(fi->fs.hsr.centroid_interpolation),
	       b2s(fi->fs.hsr.varying_before_atest_zsemit));
	printf("\"blend\": [");
	for (unsigned i = 0; i < 8; i++)
		printf("%s{ \"rt\": %u, \"type\": \"0x%x\", \"return_offset\": %u }", i ? ", " : "", i,
		       fi->bifrost.blend[i].type, fi->bifrost.blend[i].return_offset);
	printf("], \"blend_src1_type\": \"0x%x\", \"wait_6\": %s, \"wait_7\": %s, "
	       "\"uses_flat_shading\": %s, \"uses_ld_var\": %s, \"noperspective\": \"0x%x\",\n    ",
	       fi->bifrost.blend_src1_type, b2s(fi->bifrost.wait_6), b2s(fi->bifrost.wait_7),
	       b2s(fi->bifrost.uses_flat_shading), b2s(fi->bifrost.uses_ld_var),
	       fi->varyings.noperspective);
	print_varying_layout("varying_formats", &fi->varyings.formats);
	printf(",\n    ");
	print_earlyzs(&fsc.earlyzs);
	printf(",\n    ");
	print_uniforms(&fs_un);
	printf(",\n    ");
	print_sysvals(&fsc.sysvals);
	printf(",\n    ");
	print_fau(&fsc, &fs_un);
	printf(",\n    ");
	print_binary(&fsc.binary, fr, 1);
	printf(" }\n}\n");
	fflush(stdout);

	disasm("vs", &vsc.binary, vr, nvr);
	disasm("fs", &fsc.binary, fr, 1);

	for (unsigned i = 0; i < vs_un.n; i++)
		free(vs_un.u[i].name);
	for (unsigned i = 0; i < fs_un.n; i++)
		free(fs_un.u[i].name);
	util_dynarray_fini(&vsc.binary);
	util_dynarray_fini(&fsc.binary);
	return 0;
}
