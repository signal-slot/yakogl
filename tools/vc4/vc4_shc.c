/* SPDX-License-Identifier: MIT
 *
 * vc4_shc: YakoGL's offline shader tool for the Broadcom VideoCore IV (gallium
 * vc4, Raspberry Pi Zero 2 W). Reads one GL program as two OpenGL-flavoured
 * SPIR-V modules (glslangValidator -G), turns them into NIR the way Mesa's GL
 * state tracker would, and hands them to Mesa's own vc4 driver code, which
 * compiles them as it does for a draw (vc4_program.c vc4_update_compiled_fs and
 * vc4_update_compiled_vs, both through vc4_get_compiled_shader and
 * vc4_shader_ntq):
 *
 *   1. the fragment shader (FS),
 *   2. the vertex shader (VS), keyed by the FS's inputs,
 *   3. the coordinate shader (CS): the same VS with is_coord set, for binning.
 *
 * It prints the QPU code and the uniform stream of all three as JSON, and the
 * disassembly (Mesa's vc4_qpu_disasm) on stderr. No device and no drm-shim are
 * needed: vc4_shader_ntq and vc4_get_compiled_shader are static in
 * vc4_program.c, so the tool sets up the part of a vc4_context they read (a
 * zeroed context with the state objects below) and calls the driver's public
 * entry points: vc4_program_init (the shader caches and the create/bind
 * hooks), pipe_context create_vs_state/create_fs_state (vc4_shader_state_create:
 * the driver's own NIR preparation), bind_*_state, and
 * vc4_update_compiled_shaders. The one call that would reach the kernel,
 * vc4_bo_alloc_shader (DRM_IOCTL_VC4_CREATE_SHADER_BO), is replaced at link
 * time (-Wl,--wrap=vc4_bo_alloc_shader, mesa-vc4_shc.patch) by a copy into
 * memory.
 *
 *   vc4_shc vs.spv fs.spv [--prim triangles|lines|points]
 *           [--attr-components N,N,...] [--color-format rgba8|bgra8|rgb565]
 *           [--no-threaded-fs] > out.json
 *
 * --attr-components gives the number of float components of each vertex
 * attribute the VS reads, in VS input order (the vertex elements st/mesa
 * builds; key attr_formats R32..._FLOAT); default 4 for each. The VS code
 * depends on it (vc4_nir_lower_io.c fills the missing components).
 * --color-format is the render target's format in the FS key (default rgba8,
 * PIPE_FORMAT_R8G8B8A8_UNORM, what st/mesa picks on vc4 for a GL_RGBA
 * texture; tools/vc4/README.md). --no-threaded-fs compiles the FS as on a
 * kernel without DRM_VC4_PARAM_SUPPORTS_THREADED_FS (the drm-shim and the
 * Linux driver answer 1, so the default is threaded where it fits).
 *
 * Transcribed from Mesa 26.2.1 (MIT, Copyright Broadcom, Intel and others):
 *   - the GL front half, in the order st/mesa runs it for a GL_ARB_gl_spirv
 *     program, exactly as tools/v3d/v3d_shc.c does (its stages 1-5, each pass
 *     cited there by file:line), with vc4's screen caps in place of v3d's:
 *     src/gallium/drivers/vc4/vc4_screen.c vc4_init_screen_caps,
 *     vc4_init_shader_caps; the NIR options are the driver's own
 *     (vc4_program.c vc4_screen_get_compiler_options, called here)
 *   - the keys come from the driver's own vc4_update_compiled_fs/vs over the
 *     state set in main(), which mirrors a GLES 2 draw of one colour buffer
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compiler/glsl_types.h"
#include "compiler/nir/nir.h"
#include "compiler/nir/nir_builder.h"
#include "compiler/spirv/nir_spirv.h"
#include "mesa/program/prog_statevars.h"
#include "pipe/p_defines.h"
#include "pipe/p_state.h"
#include "util/log.h"
#include "util/ralloc.h"

#include "vc4_bufmgr.h"
#include "vc4_context.h"
#include "vc4_qir.h"
#include "vc4_qpu.h"
#include "vc4_screen.h"

/*
 * The screen caps of the gallium vc4 driver that decide what the GL state
 * tracker does to a shader before the driver sees it (vc4_screen.c
 * vc4_init_screen_caps): the same fragment conventions as v3d
 * (fs_coord_origin_upper_left, fs_coord_pixel_center_half_integer,
 * fs_face_is_integer_sysval), fs_position_is_sysval and fs_point_is_sysval
 * left false, and texrect false (st_finalize_nir then lowers rectangle
 * textures, which ES 2.0 has none of).
 */
#define VC4_FS_POSITION_IS_SYSVAL	 false
#define VC4_FS_POINT_IS_SYSVAL		 false
#define VC4_FS_FACE_IS_INTEGER_SYSVAL	 true
#define VC4_FS_COORD_ORIGIN_UPPER_LEFT	 true
#define VC4_FS_COORD_ORIGIN_LOWER_LEFT	 false
#define VC4_FS_COORD_CENTER_INTEGER	 false
#define VC4_FS_COORD_CENTER_HALF_INTEGER true
#define VC4_TEXRECT			 false

/*
 * The linker's limits for nir_opt_varyings_bulk (gl_nir_linker.c:1501-1503),
 * as st_extensions.c derives them from the vc4 caps: max_const_buffer0_size is
 * 16384 floats (vc4_screen.c vc4_init_shader_caps), less 4 * MAX_CLIP_PLANES
 * (8) in the VS because clip_planes is 0, less 4 in the FS because alpha_test
 * is false; max_const_buffers is 1, so no uniform blocks.
 */
#define VC4_GL_MAX_UNIFORM_COMPONENTS (16384 - 4 * 8)
#define VC4_GL_MAX_UNIFORM_BLOCKS     (1 - 1)

static const nir_shader_compiler_options *vc4_nir_options;

/* The shader BO the driver would create: the code, kept in memory. */
struct vc4_bo *__wrap_vc4_bo_alloc_shader(struct vc4_screen *screen, const void *data,
					  uint32_t size);
struct vc4_bo *__wrap_vc4_bo_alloc_shader(struct vc4_screen *screen, const void *data,
					  uint32_t size)
{
	struct vc4_bo *bo = calloc(1, sizeof(*bo));
	if (!bo)
		abort();
	pipe_reference_init(&bo->reference, 1);
	bo->screen = screen;
	bo->name = "code";
	bo->size = size;
	bo->map = malloc(size);
	if (!bo->map)
		abort();
	memcpy(bo->map, data, size);
	return bo;
}

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
	if (!w || fread(w, 1, n, f) != (size_t)n) {
		perror("read");
		fclose(f);
		free(w);
		return NULL;
	}
	fclose(f);
	*nwords = n / 4;
	return w;
}

static unsigned type_size_vec4(const struct glsl_type *type, bool bindless)
{
	return glsl_count_vec4_slots(type, false, bindless);
}

/* gl_nir_opts(), gl_nir_linker.c:45-130 (tools/v3d/v3d_shc.c gl_nir_opts) */
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

	/* vc4 sets lower_flrp32 only (vc4_program.c nir_options) */
	NIR_PASS(progress, nir, nir_lower_flrp, 32, false);
	NIR_PASS(_, nir, nir_lower_var_copies);
}

static void gl_nir_opts_cb(nir_shader *nir, void *data)
{
	gl_nir_opts(nir);
}

/* Stage 1: _mesa_spirv_to_nir(), glspirv.c:235-343 (v3d_shc.c load_stage) */
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
				       vc4_nir_options);
	free(spv);
	if (!nir) {
		fprintf(stderr, "%s: spirv_to_nir failed\n", path);
		return NULL;
	}

	/* glspirv.c:304-311: gl_FragCoord and gl_PointCoord become inputs (vc4
	 * sets neither fs_position_is_sysval nor fs_point_is_sysval);
	 * gl_FrontFacing stays a system value (fs_face_is_integer_sysval) */
	const nir_lower_sysvals_to_varyings_options sysvals_to_varyings = {
		.frag_coord = !VC4_FS_POSITION_IS_SYSVAL,
		.point_coord = !VC4_FS_POINT_IS_SYSVAL,
		.front_face = !VC4_FS_FACE_IS_INTEGER_SYSVAL,
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

/* Stage 2: the pre-link part of gl_nir_link_spirv() (v3d_shc.c prelink_stage;
 * vc4 sets lower_to_scalar and compact_arrays is unset, which only matters for
 * clip distances, which ES 2.0 has none of) */
static void prelink_stage(nir_shader *nir)
{
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
	if (nir->info.stage == MESA_SHADER_VERTEX) {
		nir->info.next_stage = MESA_SHADER_FRAGMENT;
	} else {
		nir->info.prev_stage = MESA_SHADER_VERTEX;
		nir->info.prev_stage_has_xfb = false;
	}

	NIR_PASS(_, nir, nir_lower_global_vars_to_local);
	NIR_PASS(_, nir, nir_lower_var_copies);

	NIR_PASS(_, nir, nir_remove_dead_variables,
		 nir_var_function_temp | nir_var_shader_temp | nir_var_mem_shared, NULL);
	NIR_PASS(_, nir, nir_opt_copy_prop_vars);
	NIR_PASS(_, nir, nir_lower_alu_to_scalar, nir->options->lower_to_scalar_filter, NULL);

	NIR_PASS(_, nir, nir_opt_barrier_modes);
	NIR_PASS(_, nir, nir_opt_constant_folding);

	NIR_PASS(_, nir, nir_lower_load_const_to_scalar);

	nir_opt_access_options opt_access_options = { .is_vulkan = false };
	NIR_PASS(_, nir, nir_opt_access, &opt_access_options);
	nir_gather_clip_cull_distance_sizes_from_vars(nir);
	NIR_PASS(_, nir, nir_merge_clip_cull_distance_vars);
}

/* GLSL ES links varyings by name (v3d_shc.c link_varyings_by_name) */
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

/* Stage 3: gl_nir_lower_optimize_varyings() (v3d_shc.c link_program) */
static void link_program(nir_shader *vs, nir_shader *fs)
{
	nir_lower_io_passes(vs, true);
	nir_lower_io_passes(fs, true);
	nir_shader *shaders[2] = { vs, fs };
	nir_opt_varyings_bulk(shaders, 2, true, VC4_GL_MAX_UNIFORM_COMPONENTS,
			      VC4_GL_MAX_UNIFORM_BLOCKS, gl_nir_opts_cb, NULL);
	NIR_PASS(_, vs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
	NIR_PASS(_, fs, nir_remove_dead_variables, nir_var_uniform | nir_var_image, NULL);
}

/* Stage 4: st_link_glsl_to_nir() and st_glsl_to_nir_post_opts()
 * (v3d_shc.c postlink_stage; vc4 has the same fragment conventions) */
static void postlink_stage(nir_shader *nir)
{
	NIR_PASS(_, nir, nir_remove_dead_variables, nir_var_shader_in | nir_var_shader_out, NULL);

	if (nir->info.stage == MESA_SHADER_FRAGMENT) {
		static const gl_state_index16 wpos_state[STATE_LENGTH] = {
			STATE_FB_WPOS_Y_TRANSFORM
		};
		nir_lower_wpos_ytransform_options wpos_options = {
			.fs_coord_origin_upper_left = VC4_FS_COORD_ORIGIN_UPPER_LEFT,
			.fs_coord_origin_lower_left = VC4_FS_COORD_ORIGIN_LOWER_LEFT,
			.fs_coord_pixel_center_integer = VC4_FS_COORD_CENTER_INTEGER,
			.fs_coord_pixel_center_half_integer = VC4_FS_COORD_CENTER_HALF_INTEGER,
		};
		memcpy(wpos_options.state_tokens, wpos_state, sizeof(wpos_options.state_tokens));
		NIR_PASS(_, nir, nir_lower_wpos_ytransform, &wpos_options);

		static const gl_state_index16 pntc_state[1][STATE_LENGTH] = {
			{ STATE_FB_PNTC_Y_TRANSFORM }
		};
		NIR_PASS(_, nir, nir_lower_pntc_ytransform, pntc_state);
	}

	struct nir_lower_compute_system_values_options cs_options = { 0 };
	NIR_PASS(_, nir, nir_lower_system_values);
	NIR_PASS(_, nir, nir_lower_compute_system_values, &cs_options);

	/* vc4 sets no lower_int64_options: nir_lower_int64 has nothing to do */
	bool lowered_64bit_ops = false;
	NIR_PASS(lowered_64bit_ops, nir, nir_lower_int64);
	if (lowered_64bit_ops)
		gl_nir_opts(nir);

	nir_remove_dead_variables(nir, nir_var_function_temp, NULL);
	NIR_PASS(_, nir, nir_opt_intrinsics);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

/* GL-style uniform locations, the state uniforms last (v3d_shc.c) */
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

/* Stage 5: the variant st/mesa makes at the first draw (v3d_shc.c
 * create_variant). st_finalize_nir: texrect is false on vc4 and
 * texture_gather_offsets too, so both lowerings are on. vc4 sets neither
 * packed_uniforms nor nir_io_has_intrinsics. */
static void create_variant(nir_shader *nir)
{
	struct nir_lower_tex_options tex_opts = {
		.lower_rect = !VC4_TEXRECT,
		.lower_tg4_offsets = true,
	};
	NIR_PASS(_, nir, nir_lower_tex, &tex_opts);

	assign_uniform_locations(nir);
	NIR_PASS(_, nir, nir_lower_io, nir_var_uniform, type_size_vec4, (nir_lower_io_options)0);
	NIR_PASS(_, nir, nir_lower_samplers);

	NIR_PASS(_, nir, nir_opt_dce);
	NIR_PASS(_, nir, nir_unlower_io_to_vars, false);
	gl_nir_opts(nir);
	nir_shader_gather_info(nir, nir_shader_get_entrypoint(nir));
}

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

/* enum quniform_contents (vc4_qir.h), by name */
static const char *const quniform_names[] = {
	[QUNIFORM_CONSTANT] = "CONSTANT",
	[QUNIFORM_UNIFORM] = "UNIFORM",
	[QUNIFORM_VIEWPORT_X_SCALE] = "VIEWPORT_X_SCALE",
	[QUNIFORM_VIEWPORT_Y_SCALE] = "VIEWPORT_Y_SCALE",
	[QUNIFORM_VIEWPORT_Z_OFFSET] = "VIEWPORT_Z_OFFSET",
	[QUNIFORM_VIEWPORT_Z_SCALE] = "VIEWPORT_Z_SCALE",
	[QUNIFORM_TEXTURE_CONFIG_P0] = "TEXTURE_CONFIG_P0",
	[QUNIFORM_TEXTURE_CONFIG_P1] = "TEXTURE_CONFIG_P1",
	[QUNIFORM_TEXTURE_CONFIG_P2] = "TEXTURE_CONFIG_P2",
	[QUNIFORM_TEXTURE_FIRST_LEVEL] = "TEXTURE_FIRST_LEVEL",
	[QUNIFORM_TEXTURE_MSAA_ADDR] = "TEXTURE_MSAA_ADDR",
	[QUNIFORM_UBO0_ADDR] = "UBO0_ADDR",
	[QUNIFORM_UBO1_ADDR] = "UBO1_ADDR",
	[QUNIFORM_TEXRECT_SCALE_X] = "TEXRECT_SCALE_X",
	[QUNIFORM_TEXRECT_SCALE_Y] = "TEXRECT_SCALE_Y",
	[QUNIFORM_TEXTURE_BORDER_COLOR] = "TEXTURE_BORDER_COLOR",
	[QUNIFORM_BLEND_CONST_COLOR_X] = "BLEND_CONST_COLOR_X",
	[QUNIFORM_BLEND_CONST_COLOR_Y] = "BLEND_CONST_COLOR_Y",
	[QUNIFORM_BLEND_CONST_COLOR_Z] = "BLEND_CONST_COLOR_Z",
	[QUNIFORM_BLEND_CONST_COLOR_W] = "BLEND_CONST_COLOR_W",
	[QUNIFORM_BLEND_CONST_COLOR_RGBA] = "BLEND_CONST_COLOR_RGBA",
	[QUNIFORM_BLEND_CONST_COLOR_AAAA] = "BLEND_CONST_COLOR_AAAA",
	[QUNIFORM_STENCIL] = "STENCIL",
	[QUNIFORM_SAMPLE_MASK] = "SAMPLE_MASK",
	[QUNIFORM_UNIFORMS_ADDRESS] = "UNIFORMS_ADDRESS",
};

static void print_shader(const char *name, const struct vc4_compiled_shader *sh, bool frag,
			 bool last)
{
	const uint64_t *code = sh->bo ? sh->bo->map : NULL;
	unsigned n = sh->bo ? sh->bo->size / 8 : 0;

	printf("  \"%s\": { \"failed\": %s, \"instructions\": %u, ", name,
	       sh->failed ? "true" : "false", n);
	if (frag) {
		printf("\"threaded\": %s, \"disable_early_z\": %s, \"color_inputs\": \"0x%08x\", "
		       "\"num_inputs\": %u, \"input_slots\": [",
		       sh->fs_threaded ? "true" : "false", sh->disable_early_z ? "true" : "false",
		       sh->color_inputs, sh->num_inputs);
		for (unsigned i = 0; i < sh->fs_inputs->num_inputs; i++)
			printf("%s{ \"slot\": %u, \"swizzle\": %u }", i ? ", " : "",
			       sh->fs_inputs->input_slots[i].slot,
			       sh->fs_inputs->input_slots[i].swizzle);
		printf("],\n    ");
	} else {
		printf("\"num_inputs\": %u, \"vattrs_live\": \"0x%02x\", \"vattr_offsets\": [",
		       sh->num_inputs, sh->vattrs_live);
		for (unsigned i = 0; i < 9; i++)
			printf("%s%u", i ? ", " : "", sh->vattr_offsets[i]);
		printf("],\n    ");
	}
	printf("\"num_texture_samples\": %u, \"uniforms\": [", sh->uniforms.num_texture_samples);
	for (unsigned i = 0; i < sh->uniforms.count; i++) {
		unsigned c = sh->uniforms.contents[i];
		printf("%s{ \"contents\": %u, \"name\": \"%s\", \"data\": \"0x%08x\" }", i ? ", " : "",
		       c, c < ARRAY_SIZE(quniform_names) ? quniform_names[c] : "?",
		       sh->uniforms.data[i]);
	}
	printf("],\n    \"code\": [");
	for (unsigned i = 0; i < n; i++)
		printf("%s\"0x%016llx\"", i ? ", " : "", (unsigned long long)code[i]);
	printf("] }%s\n", last ? "" : ",");
}

/* Mesa's disassembler, one line per instruction, on stderr */
static void disasm(const char *name, const struct vc4_compiled_shader *sh)
{
	const uint64_t *code = sh->bo ? sh->bo->map : NULL;
	unsigned n = sh->bo ? sh->bo->size / 8 : 0;
	fprintf(stderr, "--- %s: %u instructions\n", name, n);
	for (unsigned i = 0; i < n; i++) {
		struct log_stream *s = _mesa_log_stream_create(MESA_LOG_INFO, "vc4_shc");
		vc4_qpu_disasm(s, &code[i], 1);
		fprintf(stderr, "%4u 0x%016llx %s\n", i, (unsigned long long)code[i], s->msg);
		ralloc_free(s); /* not mesa_log_stream_destroy: printed above */
	}
}

static const enum pipe_format float_formats[5] = {
	PIPE_FORMAT_NONE,
	PIPE_FORMAT_R32_FLOAT,
	PIPE_FORMAT_R32G32_FLOAT,
	PIPE_FORMAT_R32G32B32_FLOAT,
	PIPE_FORMAT_R32G32B32A32_FLOAT,
};

int main(int argc, char **argv)
{
	if (argc < 3) {
		fprintf(stderr, "usage: vc4_shc vs.spv fs.spv [--prim triangles|lines|points] "
				"[--attr-components N,N,...] [--color-format rgba8|bgra8|rgb565] "
				"[--no-threaded-fs]\n");
		return 2;
	}
	const char *prim = "triangles";
	const char *color = "rgba8";
	const char *attr_list = NULL;
	bool threaded_fs = true;
	for (int i = 3; i < argc; i++) {
		if (strcmp(argv[i], "--prim") == 0 && i + 1 < argc) {
			prim = argv[++i];
		} else if (strcmp(argv[i], "--color-format") == 0 && i + 1 < argc) {
			color = argv[++i];
		} else if (strcmp(argv[i], "--attr-components") == 0 && i + 1 < argc) {
			attr_list = argv[++i];
		} else if (strcmp(argv[i], "--no-threaded-fs") == 0) {
			threaded_fs = false;
		} else {
			fprintf(stderr, "vc4_shc: unknown argument '%s'\n", argv[i]);
			return 2;
		}
	}
	enum mesa_prim prim_mode;
	if (strcmp(prim, "triangles") == 0)
		prim_mode = MESA_PRIM_TRIANGLES;
	else if (strcmp(prim, "lines") == 0)
		prim_mode = MESA_PRIM_LINES;
	else if (strcmp(prim, "points") == 0)
		prim_mode = MESA_PRIM_POINTS;
	else {
		fprintf(stderr, "vc4_shc: --prim must be triangles, lines or points, not '%s'\n",
			prim);
		return 2;
	}
	enum pipe_format color_format;
	if (strcmp(color, "rgba8") == 0)
		color_format = PIPE_FORMAT_R8G8B8A8_UNORM;
	else if (strcmp(color, "bgra8") == 0)
		color_format = PIPE_FORMAT_B8G8R8A8_UNORM;
	else if (strcmp(color, "rgb565") == 0)
		color_format = PIPE_FORMAT_B5G6R5_UNORM;
	else {
		fprintf(stderr, "vc4_shc: --color-format must be rgba8, bgra8 or rgb565, not '%s'\n",
			color);
		return 2;
	}

	glsl_type_singleton_init_or_ref();
	vc4_nir_options = vc4_screen_get_compiler_options(NULL, MESA_SHADER_VERTEX);

	nir_shader *vs = load_stage(argv[1], MESA_SHADER_VERTEX);
	nir_shader *fs = load_stage(argv[2], MESA_SHADER_FRAGMENT);
	if (!vs || !fs)
		return 1;
	prelink_stage(vs);
	prelink_stage(fs);
	if (!link_varyings_by_name(vs, fs))
		return 1;

	/* the attributes as declared, read while they are variables */
	struct {
		char *name;
		int location;
	} attrs[8];
	unsigned num_attrs = 0;
	nir_foreach_shader_in_variable(var, vs) {
		if (num_attrs == ARRAY_SIZE(attrs)) {
			fprintf(stderr, "more than %u vertex attributes\n", num_attrs);
			return 1;
		}
		if (glsl_count_attribute_slots(var->type, false) != 1) {
			fprintf(stderr, "attribute %s: a matrix attribute is not supported here\n",
				var->name);
			return 1;
		}
		attrs[num_attrs].name = strdup(var->name ? var->name : "");
		attrs[num_attrs].location = var->data.location - VERT_ATTRIB_GENERIC0;
		num_attrs++;
	}

	link_program(vs, fs);
	postlink_stage(vs);
	postlink_stage(fs);
	create_variant(vs);
	create_variant(fs);

	/* the names, before the driver lowers the variables away */
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
				fprintf(nj,
					"%s{ \"stage\": \"%s\", \"name\": \"%s\", \"%s\": %u, "
					"\"vec4s\": %u",
					n++ ? ", " : "", stage_names[s], var->name,
					sampler ? "binding" : "slot",
					sampler ? (unsigned)var->data.binding : var->data.driver_location,
					sampler ? 0 : type_size_vec4(var->type, false));
				if (var->state_slots && !state_name(var->state_slots[0].tokens[0])) {
					fprintf(stderr, "uniform %s: state %d is not one this tool adds\n",
						var->name, var->state_slots[0].tokens[0]);
					return 1;
				}
				if (var->state_slots)
					fprintf(nj, ", \"state\": \"%s\"",
						state_name(var->state_slots[0].tokens[0]));
				fprintf(nj, " }");
			}
		}
		/* per attribute, the VS input (vertex element) it is fetched as, or
		 * null when the linked VS does not read it */
		fprintf(nj, "],\n  \"attributes\": [");
		for (unsigned a = 0; a < num_attrs; a++) {
			int rec = -1;
			nir_foreach_shader_in_variable(var, vs) {
				if (var->data.location == (int)(VERT_ATTRIB_GENERIC0 + attrs[a].location))
					rec = var->data.driver_location;
			}
			fprintf(nj, "%s{ \"name\": \"%s\", \"location\": %d, \"element\": ",
				a ? ", " : "", attrs[a].name, attrs[a].location);
			if (rec < 0)
				fprintf(nj, "null }");
			else
				fprintf(nj, "%d }", rec);
			free(attrs[a].name);
		}
		fprintf(nj, "],\n");
	}
	fclose(nj);

	/* the vertex elements, one per VS input in driver_location order */
	unsigned num_elements = 0;
	nir_foreach_shader_in_variable(var, vs) {
		if (var->data.driver_location + 1 > num_elements)
			num_elements = var->data.driver_location + 1;
	}
	unsigned comps[8] = { 4, 4, 4, 4, 4, 4, 4, 4 };
	if (attr_list) {
		unsigned k = 0;
		for (const char *p = attr_list; *p; p++) {
			if (*p == ',')
				continue;
			if (*p < '1' || *p > '4' || k == 8) {
				fprintf(stderr, "vc4_shc: --attr-components wants up to 8 of 1..4\n");
				return 2;
			}
			comps[k++] = *p - '0';
		}
	}

	/*
	 * The driver. A zeroed screen with the features the drm-shim and the
	 * Linux driver report (vc4_screen.c vc4_screen_create: branches and,
	 * unless --no-threaded-fs, threaded FS), and a context holding the state
	 * of one GLES 2 draw: one colour buffer of --color-format, no depth or
	 * stencil, blending off with all channels written, no MSAA, no user clip
	 * planes, the sample mask all ones, and the vertex elements above.
	 */
	vc4_mesa_debug = 0;
	struct vc4_screen *screen = rzalloc(NULL, struct vc4_screen);
	screen->has_control_flow = true;
	screen->has_threaded_fs = threaded_fs;
	struct vc4_context *vc4 = rzalloc(NULL, struct vc4_context);
	struct pipe_context *pctx = &vc4->base;
	pctx->screen = &screen->base;
	vc4->screen = screen;
	vc4_program_init(pctx);

	struct vc4_job job;
	memset(&job, 0, sizeof(job));
	vc4->job = &job;
	struct vc4_rasterizer_state rast;
	memset(&rast, 0, sizeof(rast));
	rast.base.point_size_per_vertex = true; /* GLES: gl_PointSize from the VS */
	rast.base.sprite_coord_enable = 0;
	vc4->rasterizer = &rast;
	struct pipe_blend_state blend;
	memset(&blend, 0, sizeof(blend));
	blend.rt[0].colormask = PIPE_MASK_RGBA;
	vc4->blend = &blend;
	struct vc4_depth_stencil_alpha_state zsa;
	memset(&zsa, 0, sizeof(zsa));
	vc4->zsa = &zsa;
	struct pipe_resource color_buffer;
	memset(&color_buffer, 0, sizeof(color_buffer));
	color_buffer.format = color_format;
	vc4->framebuffer.nr_cbufs = 1;
	vc4->framebuffer.cbufs[0].texture = &color_buffer;
	vc4->framebuffer.cbufs[0].format = color_format;
	vc4->sample_mask = (1 << VC4_MAX_SAMPLES) - 1;
	struct vc4_vertex_stateobj vtx;
	memset(&vtx, 0, sizeof(vtx));
	vtx.num_elements = num_elements;
	for (unsigned i = 0; i < num_elements && i < 8; i++)
		vtx.pipe[i].src_format = float_formats[comps[i]];
	vc4->vtx = &vtx;

	/* A texture per sampler the stage uses: RGBA8, identity swizzle, the
	 * default sampler state (vc4_setup_shared_precompile_key's format) */
	struct pipe_resource tex_res;
	memset(&tex_res, 0, sizeof(tex_res));
	tex_res.format = PIPE_FORMAT_R8G8B8A8_UNORM;
	tex_res.width0 = tex_res.height0 = 1;
	tex_res.nr_samples = 1;
	static struct vc4_sampler_view views[VC4_MAX_TEXTURE_SAMPLERS];
	static struct pipe_sampler_state samplers[VC4_MAX_TEXTURE_SAMPLERS];
	for (unsigned i = 0; i < VC4_MAX_TEXTURE_SAMPLERS; i++) {
		views[i].base.format = PIPE_FORMAT_R8G8B8A8_UNORM;
		views[i].base.swizzle_r = PIPE_SWIZZLE_X;
		views[i].base.swizzle_g = PIPE_SWIZZLE_Y;
		views[i].base.swizzle_b = PIPE_SWIZZLE_Z;
		views[i].base.swizzle_a = PIPE_SWIZZLE_W;
		views[i].base.texture = &tex_res;
		views[i].texture = &tex_res;
		vc4->fragtex.textures[i] = &views[i].base;
		vc4->fragtex.samplers[i] = &samplers[i];
		vc4->verttex.textures[i] = &views[i].base;
		vc4->verttex.samplers[i] = &samplers[i];
	}
	vc4->verttex.num_textures = vs->info.num_textures;
	vc4->fragtex.num_textures = fs->info.num_textures;

	struct pipe_shader_state vs_cso = { .type = PIPE_SHADER_IR_NIR, .ir.nir = vs };
	struct pipe_shader_state fs_cso = { .type = PIPE_SHADER_IR_NIR, .ir.nir = fs };
	void *vs_so = pctx->create_vs_state(pctx, &vs_cso);
	void *fs_so = pctx->create_fs_state(pctx, &fs_cso);
	if (!vs_so || !fs_so) {
		fprintf(stderr, "vc4_shc: the driver refused the program%s%s\n",
			vs_cso.error_message ? ": " : "",
			vs_cso.error_message ? vs_cso.error_message : "");
		return 1;
	}
	pctx->bind_vs_state(pctx, vs_so);
	pctx->bind_fs_state(pctx, fs_so);
	vc4->dirty = ~0u;

	bool ok = vc4_update_compiled_shaders(vc4, prim_mode);
	if (!ok)
		fprintf(stderr, "vc4_shc: a stage failed to compile (register allocation)\n");

	printf("{\n  \"gpu\": { \"name\": \"VC4 2.1\", \"prim\": \"%s\", \"color_format\": \"%s\", "
	       "\"threaded_fs_supported\": %s, \"attr_components\": [",
	       prim, color, threaded_fs ? "true" : "false");
	for (unsigned i = 0; i < num_elements; i++)
		printf("%s%u", i ? ", " : "", comps[i]);
	printf("] },\n");
	fputs(names_json, stdout);
	print_shader("fs", vc4->prog.fs, true, false);
	print_shader("vs", vc4->prog.vs, false, false);
	print_shader("cs", vc4->prog.cs, false, true);
	printf("}\n");
	fflush(stdout);

	disasm("fs", vc4->prog.fs);
	disasm("vs", vc4->prog.vs);
	disasm("cs", vc4->prog.cs);
	free(names_json);
	return ok ? 0 : 1;
}
