/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * record71: the shader state record Mesa's gallium v3d driver emits for one
 * program at V3D 7.1.10 (the BCM2712 D0, "GL Shader State Record Draw Index"),
 * from the prog_data v3d_shc printed. Built and run by gen.py inside the Mesa
 * tree that built v3d_shc: Mesa's own v3d_compute_vpm_config (compiler/vir.c)
 * and Mesa's own generated 7.1 packer (cle/v3d_packet_v71_pack.h) do the work.
 *
 * The field assignments are those of emit_shader_state_record()
 * (src/gallium/drivers/v3d/v3dx_draw.c:477-546, Mesa 26.2.1), the draw-index
 * branch taken when v3d_device_has_draw_index() (ver 71, rev >= 10). The
 * three code and three uniform addresses are the draw's, packed here as 0.
 * Draw state that is not the program's: a triangle draw (point size in the
 * shaded vertex data off: info->mode is not MESA_PRIM_POINTS) and no geometry
 * shader (the GL ES 2.0/3.0 core has none).
 *
 * Arguments, all decimal integers, in this order:
 *   fs: threads single_seg writes_z disable_ez uses_center_w uses_pid
 *       lock_scoreboard_on_first_thrsw uses_implicit_point_line_varyings
 *       num_inputs
 *   vs: threads single_seg vpm_input_size vpm_output_size vcm_cache_size
 *       uses_vid uses_iid
 *   cs: the same seven as vs
 * Prints `name value` lines: the vpm_config values, every record field set,
 * and the 32-byte packed record as eight little-endian words.
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "common/v3d_device_info.h"
#include "compiler/v3d_compiler.h"

#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#include "cle/v3d_packet_v71_pack.h"

enum { NFS = 9, NVS = 7, NARGS = NFS + 2 * NVS };

int main(int argc, char **argv)
{
	long a[NARGS];

	if (argc != NARGS + 1) {
		fprintf(stderr, "record71: %d arguments wanted, %d given\n", NARGS, argc - 1);
		return 2;
	}
	for (int i = 0; i < NARGS; i++) {
		char *end;
		a[i] = strtol(argv[i + 1], &end, 10);
		if (*end || a[i] < 0) {
			fprintf(stderr, "record71: argument %d '%s' is not a count\n", i + 1, argv[i + 1]);
			return 2;
		}
	}
	const long *f = a, *v = a + NFS, *c = a + NFS + NVS;

	/* The Pi 5's identity (tools/v3d/pi5/v3d_ident.txt: 7.1.10.2), as v3d_shc
	 * was run; v3d_compute_vpm_config reads it only with a geometry shader. */
	struct v3d_device_info devinfo = { .ver = 71, .rev = 10, .compat_rev = 2,
					   .vpm_size = 64 * 1024, .qpu_count = 16 };
	if (!v3d_device_has_draw_index(&devinfo)) {
		fprintf(stderr, "record71: Mesa says 7.1.10 has no draw-index record\n");
		return 1;
	}

	struct v3d_vs_prog_data vs_bin, vs;
	memset(&vs_bin, 0, sizeof(vs_bin));
	memset(&vs, 0, sizeof(vs));
	vs.base.threads = v[0];
	vs.base.single_seg = v[1];
	vs.vpm_input_size = v[2];
	vs.vpm_output_size = v[3];
	vs.vcm_cache_size = v[4];
	vs.uses_vid = v[5];
	vs.uses_iid = v[6];
	vs_bin.base.threads = c[0];
	vs_bin.base.single_seg = c[1];
	vs_bin.vpm_input_size = c[2];
	vs_bin.vpm_output_size = c[3];
	vs_bin.vcm_cache_size = c[4];
	vs_bin.uses_vid = c[5];
	vs_bin.uses_iid = c[6];

	struct vpm_config vpm_cfg_bin, vpm_cfg;
	if (!v3d_compute_vpm_config(&devinfo, &vs_bin, &vs, NULL, NULL, &vpm_cfg_bin, &vpm_cfg)) {
		fprintf(stderr, "record71: v3d_compute_vpm_config refused the program\n");
		return 1;
	}
	printf("vpm_bin_As %u\nvpm_bin_Ve %u\nvpm_bin_Vc %u\n", vpm_cfg_bin.As, vpm_cfg_bin.Ve,
	       vpm_cfg_bin.Vc);
	printf("vpm_As %u\nvpm_Ve %u\nvpm_Vc %u\n", vpm_cfg.As, vpm_cfg.Ve, vpm_cfg.Vc);

	/* v3dx_draw.c:477-546, field for field */
	struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX shader = {
		V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_header
	};
	shader.enable_clipping = true;
	shader.point_size_in_shaded_vertex_data = false;
	shader.fragment_shader_does_z_writes = f[2];
	shader.turn_off_early_z_test = f[3];
	shader.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2 = f[4];
	shader.any_shader_reads_hardware_written_primitive_id = f[5];
	shader.insert_primitive_id_as_first_varying_to_fragment_shader = f[5];
	shader.do_scoreboard_wait_on_first_thread_switch = f[6];
	shader.disable_implicit_point_line_varyings = !f[7];
	shader.number_of_varyings_in_fragment_shader = f[8];
	shader.coordinate_shader_code_address = 0;
	shader.vertex_shader_code_address = 0;
	shader.fragment_shader_code_address = 0;
	shader.coordinate_shader_input_vpm_segment_size = vs_bin.vpm_input_size;
	shader.vertex_shader_input_vpm_segment_size = vs.vpm_input_size;
	shader.coordinate_shader_output_vpm_segment_size = vs_bin.vpm_output_size;
	shader.vertex_shader_output_vpm_segment_size = vs.vpm_output_size;
	shader.coordinate_shader_uniforms_address = 0;
	shader.vertex_shader_uniforms_address = 0;
	shader.fragment_shader_uniforms_address = 0;
	shader.min_coord_shader_input_segments_required_in_play = vpm_cfg_bin.As;
	shader.min_vertex_shader_input_segments_required_in_play = vpm_cfg.As;
	shader.min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size =
		vpm_cfg_bin.Ve;
	shader.min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size =
		vpm_cfg.Ve;
	shader.coordinate_shader_4_way_threadable = vs_bin.base.threads == 4;
	shader.vertex_shader_4_way_threadable = vs.base.threads == 4;
	shader.fragment_shader_4_way_threadable = f[0] == 4;
	shader.coordinate_shader_start_in_final_thread_section = vs_bin.base.single_seg;
	shader.vertex_shader_start_in_final_thread_section = vs.base.single_seg;
	shader.fragment_shader_start_in_final_thread_section = f[1];
	shader.vertex_id_read_by_coordinate_shader = vs_bin.uses_vid;
	shader.instance_id_read_by_coordinate_shader = vs_bin.uses_iid;
	shader.vertex_id_read_by_vertex_shader = vs.uses_vid;
	shader.instance_id_read_by_vertex_shader = vs.uses_iid;

#define P(field) printf("%s %u\n", #field, (unsigned)shader.field)
	P(point_size_in_shaded_vertex_data);
	P(enable_clipping);
	P(vertex_id_read_by_coordinate_shader);
	P(instance_id_read_by_coordinate_shader);
	P(base_instance_id_read_by_coordinate_shader);
	P(cs_basevertex);
	P(cs_drawindex);
	P(vertex_id_read_by_vertex_shader);
	P(instance_id_read_by_vertex_shader);
	P(base_instance_id_read_by_vertex_shader);
	P(vs_basevertex);
	P(vs_drawindex);
	P(fragment_shader_does_z_writes);
	P(turn_off_early_z_test);
	P(fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2);
	P(enable_sample_rate_shading);
	P(any_shader_reads_hardware_written_primitive_id);
	P(insert_primitive_id_as_first_varying_to_fragment_shader);
	P(turn_off_scoreboard);
	P(do_scoreboard_wait_on_first_thread_switch);
	P(disable_implicit_point_line_varyings);
	P(no_prim_pack);
	P(never_defer_fep_depth_writes);
	P(number_of_varyings_in_fragment_shader);
	P(coordinate_shader_output_vpm_segment_size);
	P(min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size);
	P(coordinate_shader_input_vpm_segment_size);
	P(min_coord_shader_input_segments_required_in_play);
	P(vertex_shader_output_vpm_segment_size);
	P(min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size);
	P(vertex_shader_input_vpm_segment_size);
	P(min_vertex_shader_input_segments_required_in_play);
	P(fragment_shader_4_way_threadable);
	P(fragment_shader_start_in_final_thread_section);
	P(fragment_shader_propagate_nans);
	P(vertex_shader_4_way_threadable);
	P(vertex_shader_start_in_final_thread_section);
	P(vertex_shader_propagate_nans);
	P(coordinate_shader_4_way_threadable);
	P(coordinate_shader_start_in_final_thread_section);
	P(coordinate_shader_propagate_nans);
#undef P

	uint8_t rec[V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_length];
	memset(rec, 0, sizeof(rec));
	V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_pack(NULL, rec, &shader);
	for (unsigned i = 0; i + 4 <= sizeof(rec); i += 4)
		printf("word%u 0x%08x\n", i / 4,
		       (unsigned)(rec[i] | rec[i + 1] << 8 | rec[i + 2] << 16 | (uint32_t)rec[i + 3] << 24));
	return 0;
}
