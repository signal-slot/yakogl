/* SPDX-License-Identifier: MIT
 *
 * v3d_shc: YakoGL's offline shader tool for the Broadcom V3D 4.2 (Raspberry Pi 4).
 * Reads one GL program as two OpenGL-flavoured SPIR-V modules (glslangValidator -G),
 * turns them into NIR the way Mesa's GL state tracker and the gallium v3d driver would,
 * and runs Mesa's own v3d compiler three times, as the gallium driver does for a draw:
 *
 *   1. the fragment shader (FS),
 *   2. the vertex shader (VS), keyed by the FS's input slots,
 *   3. the coordinate shader (CS): the same VS with is_coord set, used for binning.
 *
 * It prints the QPU code and the reflection of all three as JSON, and the
 * disassembly on stderr. The GPU is described by a v3d_device_info filled in
 * here; no device is needed.
 *
 *   v3d_shc vs.spv fs.spv [--vpm-size BYTES] [--prim triangles|lines|points] > out.json
 *
 * --prim picks the variant: Mesa compiles the FS for the primitive being drawn
 * (v3d_program.c v3d_update_compiled_fs: is_points, is_lines) and a points draw
 * gets the VS point size (v3d_update_compiled_vs: per_vertex_point_size).
 * The default is triangles; the JSON names the variant in gpu.prim.
 *
 * Transcribed from Mesa 26.2.1 (MIT, Copyright Broadcom, Intel and others):
 *   - the GL front half, in the order st/mesa runs it for a GL_ARB_gl_spirv
 *     program (stages 1-5 below, each pass cited by file:line):
 *     src/mesa/main/glspirv.c _mesa_spirv_to_nir(),
 *     src/compiler/glsl/gl_nir_linker.c gl_nir_link_spirv() and gl_nir_opts(),
 *     src/mesa/state_tracker/st_glsl_to_nir.cpp st_link_glsl_to_nir(),
 *     st_glsl_to_nir_post_opts(), st_finalize_nir(), and
 *     src/mesa/state_tracker/st_program.c st_create_common_variant() /
 *     st_create_fp_variant()
 *   - the NIR compiler options: src/gallium/drivers/v3d/v3d_screen.c
 *     v3d_screen_get_compiler_options()
 *   - the shader preparation: src/gallium/drivers/v3d/v3d_program.c
 *     v3d_uncompiled_shader_create(), lower_uniform_offset_to_bytes_cb(),
 *     lower_textures_cb(), type_size()
 *   - the keys: v3d_program.c v3d_update_compiled_fs() / v3d_update_compiled_vs()
 *   - the device info for ver 42: src/broadcom/common/v3d_device_info.c
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
#include "pipe/p_defines.h"

#include "common/v3d_device_info.h"
#include "common/v3d_limits.h"
#include "compiler/v3d_compiler.h"
#include "qpu/qpu_disasm.h"
#include "mesa/program/prog_statevars.h"

/*
 * The NIR options the gallium v3d driver hands the state tracker
 * (v3d_screen.c, v3d_screen_get_compiler_options). lower_fsat is set for ver < 71
 * there, so it is true here.
 */
static const nir_shader_compiler_options v3d_nir_options = {
	.compact_arrays = true,
	.lower_uadd_sat = true,
	.lower_usub_sat = true,
	.lower_iadd_sat = true,
	.lower_extract_byte = true,
	.lower_extract_word = true,
	.lower_insert_byte = true,
	.lower_insert_word = true,
	.lower_bitfield_insert = true,
	.lower_bitfield_extract = true,
	.lower_bitfield_extract16 = true,
	.lower_bitfield_extract8 = true,
	.lower_bitfield_reverse = true,
	.lower_bit_count = true,
	.lower_cs_local_id_to_index = true,
	.lower_ffract = true,
	.lower_fmod = true,
	.lower_pack_unorm_2x16 = true,
	.lower_pack_snorm_2x16 = true,
	.lower_pack_unorm_4x8 = true,
	.lower_pack_snorm_4x8 = true,
	.lower_unpack_unorm_4x8 = true,
	.lower_unpack_snorm_4x8 = true,
	.lower_pack_half_2x16 = true,
	.lower_unpack_half_2x16 = true,
	.lower_pack_32_2x16 = true,
	.lower_pack_32_2x16_split = true,
	.lower_unpack_32_2x16_split = true,
	.lower_fdiv = true,
	.lower_find_lsb = true,
	.lower_flrp32 = true,
	.lower_fpow = true,
	.lower_fsat = true,
	.lower_fsqrt = true,
	.lower_ifind_msb = true,
	.lower_isign = true,
	.lower_hadd = true,
	.lower_fisnormal = true,
	.lower_mul_high = true,
	.lower_wpos_pntc = true,
	.lower_to_scalar = true,
	.lower_interpolate_at = true,
	.lower_int64_options = nir_lower_bcsel64 | nir_lower_bit_count64 | nir_lower_conv64 |
			       nir_lower_divmod64 | nir_lower_iabs64 | nir_lower_iadd64 |
			       nir_lower_icmp64 | nir_lower_imul_2x32_64 | nir_lower_imul64 |
			       nir_lower_ineg64 | nir_lower_logic64 | nir_lower_minmax64 |
			       nir_lower_shift64 | nir_lower_ufind_msb64,
	.lower_fquantize2f16 = true,
	.lower_ufind_msb = true,
	.has_fsub = true,
	.has_isub = true,
	.has_imul24 = true,
	.has_umul24 = true,
	.has_uclz = true,
	.divergence_analysis_options = nir_divergence_multiple_workgroup_per_compute_subgroup,
	.max_unroll_iterations = 16,
	.max_samples = 4,
	.force_indirect_unrolling_sampler = true,
	.scalarize_ddx = true,
	.max_varying_expression_cost = 4,
};

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
		return NULL;
	}
	fclose(f);
	*nwords = n / 4;
	return w;
}

/* v3d_program.c type_size(): the driver counts IO in attribute slots */
static unsigned type_size(const struct glsl_type *type, bool bindless)
{
	return glsl_count_attribute_slots(type, false);
}

static unsigned type_size_vec4(const struct glsl_type *type, bool bindless)
{
	return glsl_count_vec4_slots(type, false, bindless);
}

/* v3d_program.c lower_uniform_offset_to_bytes_cb(): vec4 units to bytes */
static bool lower_uniform_offset_to_bytes_cb(nir_builder *b, nir_intrinsic_instr *intr,
					     void *data)
{
	if (intr->intrinsic != nir_intrinsic_load_uniform)
		return false;
	b->cursor = nir_before_instr(&intr->instr);
	nir_intrinsic_set_base(intr, nir_intrinsic_base(intr) * 16);
	nir_src_rewrite(&intr->src[0], nir_ishl_imm(b, intr->src[0].ssa, 4));
	return true;
}

/* v3d_program.c lower_textures_cb(): in GL the texture index is the sampler index */
static bool lower_textures_cb(nir_builder *b, nir_tex_instr *tex, void *data)
{
	if (nir_tex_instr_need_sampler(tex))
		return false;
	tex->backend_flags = tex->texture_index;
	return true;
}

/*
 * The screen caps of the gallium v3d driver that decide what the GL state
 * tracker does to a shader before the driver sees it. v3d_screen.c sets only
 * fs_face_is_integer_sysval (v3d_screen.c:258) and the fs_coord_* conventions
 * (v3d_screen.c:313-316); fs_position_is_sysval and fs_point_is_sysval are left
 * at false (u_init_pipe_screen_caps, u_screen.c, does not set them either).
 */
#define V3D_FS_POSITION_IS_SYSVAL	 false
#define V3D_FS_POINT_IS_SYSVAL		 false
#define V3D_FS_FACE_IS_INTEGER_SYSVAL	 true
#define V3D_FS_COORD_ORIGIN_UPPER_LEFT	 true
#define V3D_FS_COORD_ORIGIN_LOWER_LEFT	 false
#define V3D_FS_COORD_CENTER_INTEGER	 false
#define V3D_FS_COORD_CENTER_HALF_INTEGER true

/*
 * The linker's limits for nir_opt_varyings_bulk (gl_nir_linker.c:1501-1503, the
 * minimum over the linked stages), as st_extensions.c:231-269 derives them from
 * the v3d caps: max_const_buffer0_size is 16384 floats (v3d_screen.c:164), less
 * 4 * MAX_CLIP_PLANES (8) in the VS because clip_planes is 0 (v3d_screen.c:370),
 * less 4 in the FS because alpha_test is false (v3d_screen.c:344); 16 constant
 * buffers (v3d_screen.c:165), the first being the default uniform block.
 */
#define V3D_GL_MAX_UNIFORM_COMPONENTS (16384 - 4 * 8)
#define V3D_GL_MAX_UNIFORM_BLOCKS     (16 - 1)

/*
 * gl_nir_opts(), gl_nir_linker.c:45-130: the optimisation loop the GL linker and
 * the state tracker run between their lowering passes. It lives in libglsl, which
 * this tool does not link, so it is transcribed here.
 */
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
		if (nir->options->max_unroll_iterations)
			NIR_PASS(progress, nir, nir_opt_loop_unroll);
	} while (progress);

	/* v3d sets lower_flrp32 only (v3d_nir_options above) */
	NIR_PASS(progress, nir, nir_lower_flrp, 32, false);
	NIR_PASS(_, nir, nir_lower_var_copies);
}

static void gl_nir_opts_cb(nir_shader *nir, void *data)
{
	gl_nir_opts(nir);
}

/*
 * Stage 1, per shader: _mesa_spirv_to_nir(), glspirv.c:235-343, which st/mesa
 * calls for a GL_ARB_gl_spirv program (st_glsl_to_nir.cpp:479).
 */
static nir_shader *load_stage(const char *path, mesa_shader_stage stage)
{
	size_t nwords;
	uint32_t *spv = read_words(path, &nwords);
	if (!spv)
		return NULL;
	/* glspirv.c:266-277; the capabilities are left to spirv_to_nir's
	 * defaults and there is no subgroup size (no subgroups in ES 2.0) */
	struct spirv_to_nir_options spirv_opts = {
		.environment = NIR_SPIRV_OPENGL,
		.ubo_addr_format = nir_address_format_32bit_index_offset,
		.ssbo_addr_format = nir_address_format_32bit_index_offset,
		.shared_addr_format = nir_address_format_32bit_offset,
	};
	nir_shader *nir =
		spirv_to_nir(spv, nwords, NULL, stage, "main", &spirv_opts, &v3d_nir_options);
	free(spv);
	if (!nir) {
		fprintf(stderr, "%s: spirv_to_nir failed\n", path);
		return NULL;
	}

	/*
	 * glspirv.c:304-311: gl_FragCoord and gl_PointCoord become input
	 * variables at VARYING_SLOT_POS / VARYING_SLOT_PNTC because v3d does not
	 * set fs_position_is_sysval / fs_point_is_sysval (st_extensions.c:476-481
	 * turns the caps into GLSLFragCoordIsSysVal / GLSLPointCoordIsSysVal).
	 * The v3d compiler reads them only as variables (nir_to_vir.c
	 * ntq_setup_fs_inputs: emit_fragcoord_input for VARYING_SLOT_POS, point_x/y
	 * for VARYING_SLOT_PNTC) and has no emitter for load_frag_coord or
	 * load_point_coord. gl_FrontFacing stays a system value
	 * (fs_face_is_integer_sysval, v3d_screen.c:258).
	 */
	const nir_lower_sysvals_to_varyings_options sysvals_to_varyings = {
		.frag_coord = !V3D_FS_POSITION_IS_SYSVAL,
		.point_coord = !V3D_FS_POINT_IS_SYSVAL,
		.front_face = !V3D_FS_FACE_IS_INTEGER_SYSVAL,
		.primitive_id = stage == MESA_SHADER_FRAGMENT,
	};
	NIR_PASS(_, nir, nir_lower_sysvals_to_varyings, &sysvals_to_varyings);

	/* glspirv.c:317-339 */
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
 * Stage 2, per shader, the part of gl_nir_link_spirv() (gl_nir_linker.c:1529)
 * that comes before varyings are linked: remove_dead_varyings_pre_linking()
 * (gl_nir_linker.c:1544, not SSO, so every unused input and output goes) and
 * prelink_lowering() (gl_nir_linker.c:1393) with preprocess_shader()
 * (gl_nir_linker.c:1309).
 *
 * Left out, each for a reason the corpus cannot reach: gl_nir_add_point_size
 * (PointSizeFixed is true for v3d: st_extensions.c:204, point_size_fixed left
 * at u_screen.c:151's PIPE_POINT_SIZE_LOWER_NEVER), gl_nir_zero_initialize_
 * clip_distance (ES 2.0 has no gl_ClipDistance), nir_lower_mediump_vars (v3d
 * has no fp16/int16 shader caps), gl_nir_lower_images and
 * gl_nir_lower_blend_equation_advanced (no images or framebuffer fetch in
 * ES 2.0; both live in libglsl, which this tool does not link).
 */
static void prelink_stage(nir_shader *nir)
{
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
	/* gl_nir_linker.c:1328-1346: the stage hints of a VS + FS program */
	if (nir->info.stage == MESA_SHADER_VERTEX) {
		nir->info.next_stage = MESA_SHADER_FRAGMENT;
	} else {
		nir->info.prev_stage = MESA_SHADER_VERTEX;
		nir->info.prev_stage_has_xfb = false;
	}

	/* gl_nir_linker.c:1359-1360 */
	NIR_PASS(_, nir, nir_lower_global_vars_to_local);
	NIR_PASS(_, nir, nir_lower_var_copies);

	/* gl_nir_linker.c:1367-1374 (v3d sets lower_to_scalar) */
	NIR_PASS(_, nir, nir_remove_dead_variables,
		 nir_var_function_temp | nir_var_shader_temp | nir_var_mem_shared, NULL);
	NIR_PASS(_, nir, nir_opt_copy_prop_vars);
	NIR_PASS(_, nir, nir_lower_alu_to_scalar, nir->options->lower_to_scalar_filter, NULL);

	/* gl_nir_linker.c:1376, 1390 */
	NIR_PASS(_, nir, nir_opt_barrier_modes);
	NIR_PASS(_, nir, nir_opt_constant_folding);

	/* prelink_lowering(), gl_nir_linker.c:1429-1431 */
	NIR_PASS(_, nir, nir_lower_load_const_to_scalar);

	/* gl_nir_linker.c:1443-1463; v3d sets compact_arrays, so the clip
	 * distances are not turned into vec4s */
	nir_opt_access_options opt_access_options = { .is_vulkan = false };
	NIR_PASS(_, nir, nir_opt_access, &opt_access_options);
	nir_gather_clip_cull_distance_sizes_from_vars(nir);
	NIR_PASS(_, nir, nir_merge_clip_cull_distance_vars);
}

/*
 * GLSL ES links varyings by name; the v3d compiler matches a VS output to an FS
 * input by varying slot (the FS's input_slots become the VS key's used_outputs).
 * glsl2vivante.py numbers each stage's varyings on its own, so give every FS
 * input the location of the VS output with the same name. A name with no VS
 * output is a link error, reported rather than guessed. It runs after
 * prelink_stage(), so an input the FS never reads is gone by then, as the ES
 * 1.00 rule (only a statically used varying must be declared in the VS) wants.
 */
static bool link_varyings_by_name(nir_shader *vs, nir_shader *fs)
{
	bool ok = true;
	nir_foreach_shader_in_variable(in, fs) {
		if (in->data.location < VARYING_SLOT_VAR0)
			continue; /* built-ins (gl_FragCoord, gl_PointCoord) */
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
 * Stage 3, the program: gl_nir_lower_optimize_varyings()
 * (gl_nir_linker.c:1480-1525, called from gl_nir_link_spirv at :1553).
 *
 * nir_lower_io_passes() (nir_lower_io.c:1418) is what makes the shader the
 * v3d compiler expects:
 *   - nir_lower_io_vars_to_temporaries on the outputs moves every output store
 *     to the end of the shader, out of any control flow (the v3d compiler's
 *     nir_move_output_stores_to_end asserts that no store is inside one);
 *   - nir_lower_io with type_size_vec4 gives each slot of a matrix attribute its
 *     own load_input, and nir_unlower_io_to_vars later makes one vector
 *     variable per slot (ntq_setup_vs_inputs asserts that no VS input is an
 *     array or a matrix: "No VS attribute array support");
 *   - with renumber_vs_inputs the VS inputs are numbered 0..n-1 over the slots
 *     the shader reads, which is the vertex element order st/mesa builds.
 * Then nir_opt_varyings_bulk links the two stages (dead varyings, constants and
 * uniform expressions moved across, compaction).
 */
static void link_program(nir_shader *vs, nir_shader *fs)
{
	nir_lower_io_passes(vs, true);
	nir_lower_io_passes(fs, true);
	nir_shader *shaders[2] = { vs, fs };
	nir_opt_varyings_bulk(shaders, 2, true, V3D_GL_MAX_UNIFORM_COMPONENTS,
			      V3D_GL_MAX_UNIFORM_BLOCKS, gl_nir_opts_cb, NULL);

	/* gl_nir_linker.c:1555-1565: unused uniforms go (ES 2.0 has no uniform
	 * blocks or uniform initialisers, which can_remove_var would keep) */
	NIR_PASS(_, vs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
	NIR_PASS(_, fs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
}

/*
 * Stage 4, per shader: the rest of st_link_glsl_to_nir() (st_glsl_to_nir.cpp:
 * 524-560) and st_glsl_to_nir_post_opts() (st_glsl_to_nir.cpp:236-381).
 *
 * The indirect-addressing lowering (:534-547) does nothing for v3d
 * (indirect_temp_addr and indirect_const_addr are set, v3d_screen.c:166-167);
 * gl_nir_lower_buffers (:552), st_nir_lower_builtin (:283, not for SPIR-V) and
 * the atomic lowerings (:287-288, :352-364) have nothing to do in an ES 2.0
 * shader and live in libglsl/st, which this tool does not link.
 */
static void postlink_stage(nir_shader *nir)
{
	/* st_glsl_to_nir.cpp:529: the IO is lowered, the variables are unused */
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	/*
	 * st_nir_lower_wpos_ytransform(), st_glsl_to_nir.cpp:393-435. v3d's
	 * fragment coordinate origin is upper left (v3d_screen.c:313-316), GL's
	 * is lower left, so a gl_FragCoord.y read becomes a function of the
	 * state uniform gl_FbWposYTransform, and a gl_PointCoord.y read one of
	 * gl_FbPntcYTransform. The runtime supplies both (prog_statevars.c:736-763,
	 * STATE_FB_WPOS_Y_TRANSFORM, STATE_FB_PNTC_Y_TRANSFORM); the JSON names
	 * them with their slot.
	 */
	if (nir->info.stage == MESA_SHADER_FRAGMENT) {
		static const gl_state_index16 wpos_state[STATE_LENGTH] = {
			STATE_FB_WPOS_Y_TRANSFORM
		};
		nir_lower_wpos_ytransform_options wpos_options = {
			.fs_coord_origin_upper_left = V3D_FS_COORD_ORIGIN_UPPER_LEFT,
			.fs_coord_origin_lower_left = V3D_FS_COORD_ORIGIN_LOWER_LEFT,
			.fs_coord_pixel_center_integer = V3D_FS_COORD_CENTER_INTEGER,
			.fs_coord_pixel_center_half_integer = V3D_FS_COORD_CENTER_HALF_INTEGER,
		};
		memcpy(wpos_options.state_tokens, wpos_state, sizeof(wpos_options.state_tokens));
		NIR_PASS(_, nir, nir_lower_wpos_ytransform, &wpos_options);

		static const gl_state_index16 pntc_state[1][STATE_LENGTH] = {
			{ STATE_FB_PNTC_Y_TRANSFORM }
		};
		NIR_PASS(_, nir, nir_lower_pntc_ytransform, pntc_state);
	}

	/* st_glsl_to_nir.cpp:557-560 */
	struct nir_lower_compute_system_values_options cs_options = { 0 };
	NIR_PASS(_, nir, nir_lower_system_values);
	NIR_PASS(_, nir, nir_lower_compute_system_values, &cs_options);

	/* st_glsl_to_nir_post_opts(): 64-bit integer lowering (:293-347; v3d
	 * sets lower_int64_options and no lower_doubles_options) */
	bool lowered_64bit_ops = false;
	NIR_PASS(lowered_64bit_ops, nir, nir_lower_int64);
	if (lowered_64bit_ops)
		gl_nir_opts(nir);

	/* st_glsl_to_nir.cpp:350, 366, 369 */
	nir_remove_dead_variables(nir, nir_var_function_temp, NULL);
	NIR_PASS(_, nir, nir_opt_intrinsics);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

/*
 * GL-style uniform locations, in location order (as etna_shc). The state
 * uniforms the state tracker adds (gl_FbWposYTransform, ...) come after the
 * user's, as _mesa_add_state_reference appends them to the parameter list
 * (st_update_state_param_locations, st_glsl_to_nir.cpp:116).
 */
static void assign_uniform_locations(nir_shader *nir)
{
	unsigned next = 0;
	for (int loc = 0; loc < 256; loc++) {
		nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
			if (var->state_slots || var->data.location != loc)
				continue;
			if (glsl_type_is_sampler(glsl_without_array(var->type))) {
				var->data.driver_location = var->data.binding;
				continue;
			}
			var->data.driver_location = next;
			next += type_size_vec4(var->type, false);
		}
	}
	nir_foreach_variable_with_modes(var, nir, nir_var_uniform) {
		if (!var->state_slots)
			continue;
		var->data.driver_location = next;
		next += type_size_vec4(var->type, false);
	}
}

/*
 * Stage 5, per shader: the shader variant st/mesa creates at the first draw
 * (st_create_common_variant for the VS, st_program.c:792; st_create_fp_variant
 * for the FS, st_program.c:1050), with a default key. v3d does not set
 * call_finalize_nir_in_linker, so st_finalize_nir runs here
 * (st_program.c:854, :1220), and v3d does not set nir_io_has_intrinsics, so
 * the lowered IO goes back to variables (st_program.c:863-876, :1245-1255).
 */
static void create_variant(nir_shader *nir)
{
	/* st_finalize_nir(), st_glsl_to_nir.cpp:723-761. texrect is supported
	 * and texture_gather_offsets is not (u_screen.c defaults), so only the
	 * tg4 offsets are lowered. */
	struct nir_lower_tex_options tex_opts = { .lower_tg4_offsets = true };
	NIR_PASS(_, nir, nir_lower_tex, &tex_opts);

	assign_uniform_locations(nir);

	/* st_nir_lower_uniforms(), st_glsl_to_nir.cpp:701-716: no packed
	 * uniforms (v3d_screen.c:280), so vec4 units, which the driver turns into
	 * bytes (lower_uniform_offset_to_bytes_cb) */
	NIR_PASS(_, nir, nir_lower_io, nir_var_uniform, type_size_vec4, (nir_lower_io_options)0);

	/* st_nir_lower_samplers(), st_glsl_to_nir.cpp:669-686: the sampler
	 * index is the SPIR-V binding here, which gl_nir_lower_samplers would
	 * take from the uniform storage */
	NIR_PASS(_, nir, nir_lower_samplers);

	/* st_program.c:871-879 (VS), 1252-1254 (FS) */
	NIR_PASS(_, nir, nir_opt_dce);
	NIR_PASS(_, nir, nir_unlower_io_to_vars, false);
	gl_nir_opts(nir);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

/*
 * Stage 6: v3d_uncompiled_shader_create() (v3d_program.c:329), for the VS and
 * FS paths. Its nir_lower_io of the FS inputs and outputs is kept: after
 * nir_unlower_io_to_vars the FS reads variables again, as it does in Mesa.
 */
static void driver_prepare(nir_shader *s)
{
	if (s->info.stage == MESA_SHADER_FRAGMENT &&
	    s->info.outputs_written & BITFIELD_BIT(FRAG_RESULT_COLOR))
		NIR_PASS(_, s, nir_lower_fragcolor, V3D_MAX_DRAW_BUFFERS);

	if (s->info.stage != MESA_SHADER_VERTEX)
		NIR_PASS(_, s, nir_lower_io, nir_var_shader_in | nir_var_shader_out, type_size,
			 (nir_lower_io_options)0);

	NIR_PASS(_, s, nir_normalize_cubemap_coords);
	NIR_PASS(_, s, nir_lower_load_const_to_scalar);
	v3d_optimize_nir(NULL, s);
	NIR_PASS(_, s, nir_lower_var_copies);
	v3d_optimize_nir(NULL, s);
	NIR_PASS(_, s, nir_remove_dead_variables, nir_var_function_temp, NULL);
	NIR_PASS(_, s, nir_shader_intrinsics_pass, lower_uniform_offset_to_bytes_cb,
		 nir_metadata_control_flow, NULL);
	NIR_PASS(_, s, nir_shader_tex_pass, lower_textures_cb, nir_metadata_control_flow, NULL);
	nir_sweep(s);
	nir_shader_gather_info(s, nir_shader_get_entrypoint(s));
}

/* the state uniforms postlink_stage() can add, as the JSON names them */
static const char *state_name(gl_state_index16 token)
{
	switch (token) {
	case STATE_FB_WPOS_Y_TRANSFORM:
		return "FB_WPOS_Y_TRANSFORM";
	case STATE_FB_PNTC_Y_TRANSFORM:
		return "FB_PNTC_Y_TRANSFORM";
	default:
		return NULL;
	}
}

static void debug_output(const char *msg, void *data)
{
	fprintf(stderr, "v3d: %s\n", msg);
}

static void print_uniforms(const struct v3d_uniform_list *u)
{
	printf("\"uniforms\": [");
	for (unsigned i = 0; i < u->count; i++)
		printf("%s{ \"contents\": %u, \"data\": \"0x%08x\" }", i ? ", " : "",
		       u->contents[i], u->data[i]);
	printf("]");
}

static void print_code(const uint64_t *code, uint32_t bytes)
{
	printf("\"code\": [");
	for (uint32_t i = 0; i < bytes / 8; i++)
		printf("%s\"0x%016llx\"", i ? ", " : "", (unsigned long long)code[i]);
	printf("]");
}

static void print_common(const struct v3d_prog_data *p)
{
	/* tmu_dirty_rcl decides the RCL's TMU cache flush (v3dx_draw.c) */
	printf("\"threads\": %u, \"single_seg\": %s, \"spill_size\": %u, "
	       "\"tmu_dirty_rcl\": %s, ",
	       p->threads, p->single_seg ? "true" : "false", p->spill_size,
	       p->tmu_dirty_rcl ? "true" : "false");
}

static void disasm(const struct v3d_device_info *devinfo, const char *name,
		   const uint64_t *code, uint32_t bytes)
{
	fprintf(stderr, "--- %s: %u instructions\n", name, bytes / 8);
	for (uint32_t i = 0; i < bytes / 8; i++) {
		const char *s = v3d_qpu_disasm(devinfo, code[i]);
		fprintf(stderr, "%4u 0x%016llx %s\n", i, (unsigned long long)code[i], s);
		ralloc_free((void *)s);
	}
}

int main(int argc, char **argv)
{
	if (argc < 3) {
		fprintf(stderr, "usage: v3d_shc vs.spv fs.spv [--vpm-size BYTES] "
				"[--prim triangles|lines|points]\n");
		return 2;
	}

	/*
	 * The GPU. The compiler reads ver, has_accumulators, vpm_size and
	 * page_size. ver 42 is the BCM2711 (linux bcm2711.dtsi "brcm,2711-v3d",
	 * Mesa v3d_device_info.c case 42). vpm_size is IDENT1.VPM_SIZE * 8192 on the
	 * board; until it has been read there it is an assumption, and the JSON
	 * says which.
	 */
	struct v3d_device_info devinfo = {
		.ver = 42,
		.has_accumulators = true,
		.vpm_size = 16 * 1024,
		.qpu_count = 8,
		.page_size = 4096,
		.clipper_xy_granularity = 256.0f,
		.cle_readahead = 256u,
		.max_render_targets = V3D_MAX_RENDER_TARGETS(42),
		.max_framebuffer_size = V3D_MAX_FRAMEBUFFER_SIZE(42),
	};
	const char *vpm_source = "assumed";
	const char *prim = "triangles";
	for (int i = 3; i < argc; i++) {
		if (strcmp(argv[i], "--vpm-size") == 0 && i + 1 < argc) {
			devinfo.vpm_size = strtol(argv[++i], NULL, 0);
			vpm_source = "given";
		} else if (strcmp(argv[i], "--prim") == 0 && i + 1 < argc) {
			prim = argv[++i];
		} else {
			fprintf(stderr, "v3d_shc: unknown argument '%s'\n", argv[i]);
			return 2;
		}
	}
	const bool points = strcmp(prim, "points") == 0;
	const bool lines = strcmp(prim, "lines") == 0;
	if (!points && !lines && strcmp(prim, "triangles") != 0) {
		fprintf(stderr, "v3d_shc: --prim must be triangles, lines or points, not '%s'\n",
			prim);
		return 2;
	}

	glsl_type_singleton_init_or_ref();
	const struct v3d_compiler *compiler = v3d_compiler_init(&devinfo, 0);
	if (!compiler) {
		fprintf(stderr, "v3d_compiler_init failed\n");
		return 1;
	}

	nir_shader *vs = load_stage(argv[1], MESA_SHADER_VERTEX);
	nir_shader *fs = load_stage(argv[2], MESA_SHADER_FRAGMENT);
	if (!vs || !fs)
		return 1;
	prelink_stage(vs);
	prelink_stage(fs);
	if (!link_varyings_by_name(vs, fs))
		return 1;

	/*
	 * The attributes as the program declares them, read while they are still
	 * variables: the GL location (the SPIR-V Location, VERT_ATTRIB_GENERIC0
	 * based) and how many slots it takes (a matN takes N). Which vertex
	 * attribute record each slot becomes is known only after the IO has been
	 * lowered and renumbered, below.
	 */
	struct {
		char *name;
		int location;
		unsigned slots;
	} attrs[V3D_MAX_VS_INPUTS / 4];
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
	postlink_stage(vs);
	postlink_stage(fs);
	create_variant(vs);
	create_variant(fs);

	/* the names are read before the driver lowers the variables away */
	char *names_json = NULL;
	size_t names_len = 0;
	FILE *nj = open_memstream(&names_json, &names_len);
	{
		nir_shader *stages[2] = { vs, fs };
		const char *stage_names[2] = { "vert", "frag" };
		unsigned n = 0;
		fprintf(nj, "  \"uniform_names\": [");
		for (int s = 0; s < 2; s++) {
			nir_foreach_variable_with_modes(var, stages[s], nir_var_uniform) {
				bool sampler = glsl_type_is_sampler(glsl_without_array(var->type));
				/* A sampler's texture index, the one its code carries in
				 * TMU config P0 bits 24..31, is var->data.binding after
				 * gl_nir_lower_samplers_as_deref (gl_nir_lower_samplers_as_deref.c:388:
				 * the uniform storage's opaque[stage].index); its
				 * driver_location is not that index. */
				fprintf(nj,
					"%s{ \"stage\": \"%s\", \"name\": \"%s\", \"%s\": %u, "
					"\"vec4s\": %u",
					n++ ? ", " : "", stage_names[s], var->name,
					sampler ? "binding" : "slot",
					sampler ? (unsigned)var->data.binding : var->data.driver_location,
					sampler ? 0 : type_size_vec4(var->type, false));
				/* a state uniform the runtime computes (postlink_stage) */
				if (var->state_slots && !state_name(var->state_slots[0].tokens[0])) {
					fprintf(stderr, "uniform %s: state %d is not one this tool "
						"adds\n", var->name, var->state_slots[0].tokens[0]);
					return 1;
				}
				if (var->state_slots)
					fprintf(nj, ", \"state\": \"%s\"",
						state_name(var->state_slots[0].tokens[0]));
				fprintf(nj, " }");
			}
		}
		/*
		 * Per attribute, per slot: the index of the vertex attribute record
		 * (the driver_location the v3d compiler indexes vattr_sizes with), or
		 * null for a slot the shader does not read.
		 */
		fprintf(nj, "],\n  \"attributes\": [");
		for (unsigned a = 0; a < num_attrs; a++) {
			fprintf(nj, "%s{ \"name\": \"%s\", \"location\": %d, \"records\": [",
				a ? ", " : "", attrs[a].name, attrs[a].location);
			for (unsigned c = 0; c < attrs[a].slots; c++) {
				int rec = -1;
				nir_foreach_shader_in_variable(var, vs) {
					if (var->data.location ==
					    (int)(VERT_ATTRIB_GENERIC0 + attrs[a].location + c))
						rec = var->data.driver_location;
				}
				if (rec < 0)
					fprintf(nj, "%snull", c ? ", " : "");
				else
					fprintf(nj, "%s%d", c ? ", " : "", rec);
			}
			fprintf(nj, "] }");
			free(attrs[a].name);
		}
		fprintf(nj, "],\n");
	}
	fclose(nj);

	driver_prepare(vs);
	driver_prepare(fs);

	/* 1. FS (v3d_update_compiled_fs, v3d_program.c:582): one colour buffer, no
	 * MSAA, no logic op, blending in the hardware, and the primitive of
	 * --prim. The compiler fetches the point coordinate only when is_points
	 * is set (nir_to_vir.c:4869); in the triangle variant a gl_PointCoord
	 * read is undefined, which GL allows, so a point draw needs --prim points.
	 * Line smoothing stays off (GLES has no GL_LINE_SMOOTH). */
	struct v3d_fs_key fs_key;
	memset(&fs_key, 0, sizeof(fs_key));
	fs_key.base.is_last_geometry_stage = true;
	fs_key.ignore_sample_mask = true;
	fs_key.cbufs = 1;
	fs_key.logicop_func = PIPE_LOGICOP_COPY;
	fs_key.can_earlyz_with_discard = fs->info.fs.uses_discard;
	fs_key.is_points = points;
	fs_key.is_lines = lines;
	struct v3d_prog_data *fs_data = NULL;
	uint32_t fs_bytes = 0;
	uint64_t *fs_code = v3d_compile(compiler, &fs_key.base, &fs_data, fs, debug_output, NULL,
					0, 0, &fs_bytes);
	if (!fs_code) {
		fprintf(stderr, "v3d_compile FS failed\n");
		return 1;
	}
	struct v3d_fs_prog_data *fsd = (struct v3d_fs_prog_data *)fs_data;

	/* 2. VS, keyed by what the FS reads (v3d_update_compiled_vs) */
	struct v3d_vs_key vs_key;
	memset(&vs_key, 0, sizeof(vs_key));
	vs_key.base.is_last_geometry_stage = true;
	vs_key.num_used_outputs = fsd->num_inputs;
	memcpy(vs_key.used_outputs, fsd->input_slots, sizeof(vs_key.used_outputs));
	/* v3d_program.c:864-866: a point draw whose rasterizer takes the size per
	 * vertex; in GLES the only point size is gl_PointSize, so that is a VS
	 * that writes it. The CS copies this key. */
	vs_key.per_vertex_point_size =
		points && (vs->info.outputs_written & BITFIELD64_BIT(VARYING_SLOT_PSIZ));
	struct v3d_prog_data *vs_data = NULL;
	uint32_t vs_bytes = 0;
	uint64_t *vs_code = v3d_compile(compiler, &vs_key.base, &vs_data, vs, debug_output, NULL,
					1, 0, &vs_bytes);
	if (!vs_code) {
		fprintf(stderr, "v3d_compile VS failed\n");
		return 1;
	}

	/* 3. CS: the same VS for binning; with no transform feedback it writes no
	 * varyings */
	struct v3d_vs_key cs_key = vs_key;
	cs_key.is_coord = true;
	cs_key.num_used_outputs = 0;
	memset(cs_key.used_outputs, 0, sizeof(cs_key.used_outputs));
	struct v3d_prog_data *cs_data = NULL;
	uint32_t cs_bytes = 0;
	uint64_t *cs_code = v3d_compile(compiler, &cs_key.base, &cs_data, vs, debug_output, NULL,
					1, 1, &cs_bytes);
	if (!cs_code) {
		fprintf(stderr, "v3d_compile CS failed\n");
		return 1;
	}

	printf("{\n  \"gpu\": { \"ver\": %u, \"vpm_size\": %d, \"vpm_size_source\": \"%s\", "
	       "\"prim\": \"%s\", \"per_vertex_point_size\": %s },\n",
	       devinfo.ver, devinfo.vpm_size, vpm_source, prim,
	       vs_key.per_vertex_point_size ? "true" : "false");
	fputs(names_json, stdout);

	printf("  \"fs\": { ");
	print_common(fs_data);
	printf("\"num_inputs\": %u, \"input_slots\": [", fsd->num_inputs);
	for (unsigned i = 0; i < fsd->num_inputs; i++)
		printf("%s{ \"slot\": %u, \"component\": %u }", i ? ", " : "",
		       v3d_slot_get_slot(fsd->input_slots[i]),
		       v3d_slot_get_component(fsd->input_slots[i]));
	printf("], \"flat_shade_flags\": \"0x%08x\", \"noperspective_flags\": \"0x%08x\", "
	       "\"centroid_flags\": \"0x%08x\", ",
	       fsd->flat_shade_flags[0], fsd->noperspective_flags[0], fsd->centroid_flags[0]);
	printf("\"writes_z\": %s, \"disable_ez\": %s, \"uses_center_w\": %s, "
	       "\"lock_scoreboard_on_first_thrsw\": %s, \"uses_discard\": %s, "
	       "\"uses_implicit_point_line_varyings\": %s, \"uses_pid\": %s, "
	       "\"force_per_sample_msaa\": %s,\n    ",
	       fsd->writes_z ? "true" : "false", fsd->disable_ez ? "true" : "false",
	       fsd->uses_center_w ? "true" : "false",
	       fsd->lock_scoreboard_on_first_thrsw ? "true" : "false",
	       fs->info.fs.uses_discard ? "true" : "false",
	       fsd->uses_implicit_point_line_varyings ? "true" : "false",
	       fsd->uses_pid ? "true" : "false",
	       fsd->force_per_sample_msaa ? "true" : "false");
	print_uniforms(&fs_data->uniforms);
	printf(",\n    ");
	print_code(fs_code, fs_bytes);
	printf(" },\n");

	struct v3d_prog_data *vcs[2] = { vs_data, cs_data };
	const uint64_t *vcs_code[2] = { vs_code, cs_code };
	uint32_t vcs_bytes[2] = { vs_bytes, cs_bytes };
	const char *vcs_name[2] = { "vs", "cs" };
	for (int i = 0; i < 2; i++) {
		struct v3d_vs_prog_data *d = (struct v3d_vs_prog_data *)vcs[i];
		printf("  \"%s\": { ", vcs_name[i]);
		print_common(vcs[i]);
		printf("\"vpm_input_size\": %u, \"vpm_output_size\": %u, "
		       "\"separate_segments\": %s, \"vcm_cache_size\": %u, "
		       "\"uses_vid\": %s, \"uses_iid\": %s, \"uses_biid\": %s, "
		       "\"writes_psiz\": %s, \"vattr_sizes\": [",
		       d->vpm_input_size, d->vpm_output_size,
		       d->separate_segments ? "true" : "false", d->vcm_cache_size,
		       d->uses_vid ? "true" : "false", d->uses_iid ? "true" : "false",
		       d->uses_biid ? "true" : "false", d->writes_psiz ? "true" : "false");
		for (unsigned a = 0; a < V3D_MAX_VS_INPUTS / 4; a++)
			printf("%s%u", a ? ", " : "", d->vattr_sizes[a]);
		printf("],\n    ");
		print_uniforms(&vcs[i]->uniforms);
		printf(",\n    ");
		print_code(vcs_code[i], vcs_bytes[i]);
		printf(" }%s\n", i == 0 ? "," : "");
	}
	printf("}\n");
	fflush(stdout);

	disasm(&devinfo, "fs", fs_code, fs_bytes);
	disasm(&devinfo, "vs", vs_code, vs_bytes);
	disasm(&devinfo, "cs", cs_code, cs_bytes);
	free(names_json);
	return 0;
}
