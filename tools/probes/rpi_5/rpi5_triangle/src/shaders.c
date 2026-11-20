/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The rpi5_triangle image's three programs, from one of the two generated
 * headers under tools/v3d/tri-shaders71/:
 *
 *   RPI5_TRI_OURS=0  mesa_passthrough_interp_color.h: Mesa 26.2.2's VS, CS
 *                    and FS out of the board's tools/v3d/pi5/triangle.clif,
 *                    with their uniform streams and record values (the
 *                    same filling as tools/v3d/frame_check.c --triangle71,
 *                    which triangle71.sh checks against that dump);
 *   RPI5_TRI_OURS=1  ours_passthrough_interp_color.h: our 7.1 online
 *                    compiler's pass-through VS + interp_color FS, its
 *                    prog_data as the header gives it
 *                    (docs/notes/v3d71-tri-ours-record.md).
 *
 * Either way the builder derives the record from the prog_data; main.c then
 * checks the record it wrote against the header (tri_shaders_rec_check)
 * before anything is submitted.
 */
#include <stddef.h>
#include <string.h>

#include "shaders.h"

#if RPI5_TRI_OURS
#include "ours_passthrough_interp_color.h"
#else
#include "mesa_passthrough_interp_color.h"
#endif

#if RPI5_TRI_OURS

static struct v3d_uniform vs_u[OURS71_TRI_VS_NUM_UNIFORMS], cs_u[OURS71_TRI_CS_NUM_UNIFORMS];

void tri_shaders_get(struct tri_shaders *s)
{
	static const uint8_t vs_sizes[16] = OURS71_TRI_VS_VATTR_SIZES;
	static const uint8_t cs_sizes[16] = OURS71_TRI_CS_VATTR_SIZES;

	memset(s, 0, sizeof(*s));
	s->name = "ours";
	s->code[0] = ours71_tri_fs_code;
	s->code[1] = ours71_tri_vs_code;
	s->code[2] = ours71_tri_cs_code;
	s->words[0] = OURS71_TRI_FS_NUM_WORDS;
	s->words[1] = OURS71_TRI_VS_NUM_WORDS;
	s->words[2] = OURS71_TRI_CS_NUM_WORDS;
	for (uint32_t i = 0; i < OURS71_TRI_VS_NUM_UNIFORMS; i++)
		vs_u[i] = (struct v3d_uniform){ ours71_tri_vs_uniforms[i][0], ours71_tri_vs_uniforms[i][1] };
	for (uint32_t i = 0; i < OURS71_TRI_CS_NUM_UNIFORMS; i++)
		cs_u[i] = (struct v3d_uniform){ ours71_tri_cs_uniforms[i][0], ours71_tri_cs_uniforms[i][1] };
	/* OURS71_TRI_FS_NUM_UNIFORMS is 0: no FS stream */
	s->fs = (struct v3d_fs_prog){
		.threads = OURS71_TRI_FS_THREADS,
		.single_seg = OURS71_TRI_FS_SINGLE_SEG,
		.num_inputs = OURS71_TRI_FS_NUM_INPUTS,
		.flat_shade_flags = OURS71_TRI_FS_FLAT_SHADE_FLAGS,
		.noperspective_flags = OURS71_TRI_FS_NOPERSPECTIVE_FLAGS,
		.centroid_flags = OURS71_TRI_FS_CENTROID_FLAGS,
		.writes_z = OURS71_TRI_FS_WRITES_Z,
		.disable_ez = OURS71_TRI_FS_DISABLE_EZ,
		.uses_center_w = OURS71_TRI_FS_USES_CENTER_W,
		.lock_scoreboard_on_first_thrsw = OURS71_TRI_FS_LOCK_SCOREBOARD_ON_FIRST_THRSW,
		.uses_implicit_point_line_varyings = OURS71_TRI_FS_USES_IMPLICIT_POINT_LINE_VARYINGS,
		.uses_pid = OURS71_TRI_FS_USES_PID,
	};
	s->vs = (struct v3d_vs_prog){
		.threads = OURS71_TRI_VS_THREADS,
		.single_seg = OURS71_TRI_VS_SINGLE_SEG,
		.vpm_input_size = OURS71_TRI_VS_VPM_INPUT_SIZE,
		.vpm_output_size = OURS71_TRI_VS_VPM_OUTPUT_SIZE,
		.separate_segments = OURS71_TRI_VS_SEPARATE_SEGMENTS,
		.vcm_cache_size = OURS71_TRI_VS_VCM_CACHE_SIZE,
		.uses_vid = OURS71_TRI_VS_USES_VID,
		.uses_iid = OURS71_TRI_VS_USES_IID,
		.uses_biid = OURS71_TRI_VS_USES_BIID,
		.uniforms = vs_u,
		.num_uniforms = OURS71_TRI_VS_NUM_UNIFORMS,
	};
	s->cs = (struct v3d_vs_prog){
		.threads = OURS71_TRI_CS_THREADS,
		.single_seg = OURS71_TRI_CS_SINGLE_SEG,
		.vpm_input_size = OURS71_TRI_CS_VPM_INPUT_SIZE,
		.vpm_output_size = OURS71_TRI_CS_VPM_OUTPUT_SIZE,
		.separate_segments = OURS71_TRI_CS_SEPARATE_SEGMENTS,
		.vcm_cache_size = OURS71_TRI_CS_VCM_CACHE_SIZE,
		.uses_vid = OURS71_TRI_CS_USES_VID,
		.uses_iid = OURS71_TRI_CS_USES_IID,
		.uses_biid = OURS71_TRI_CS_USES_BIID,
		.uniforms = cs_u,
		.num_uniforms = OURS71_TRI_CS_NUM_UNIFORMS,
	};
	memcpy(s->vs.vattr_sizes, vs_sizes, sizeof(vs_sizes));
	memcpy(s->cs.vattr_sizes, cs_sizes, sizeof(cs_sizes));
	/* both attributes are vec4 floats, tightly packed (the JSON's a_pos and
	 * a_color, records 0 and 1); the VS reads all four components of each */
	for (uint32_t i = 0; i < TRI_SHADER_ATTRS; i++) {
		s->attr_components[i] = vs_sizes[i];
		s->attr_stride[i] = 16;
	}
}

/* the record values of ours_passthrough_interp_color.h; clipping on, no
 * point size, propagate NaNs clear and min input segments 1 are the frame
 * code's (v3d71-tri-ours-record.md section 1: Mesa's values, which run) */
#define REC(f) OURS71_TRI_REC_##f
#define REC_POINT_SIZE           0
#define REC_ENABLE_CLIPPING      1
#define REC_CS_MIN_OUT           0
#define REC_CS_MIN_IN            1
#define REC_VS_MIN_OUT           0
#define REC_VS_MIN_IN            1
#define REC_PROPAGATE_NANS       0
#define REC_FS_Z_WRITES          REC(FRAGMENT_SHADER_DOES_Z_WRITES)
#define REC_EARLY_Z_OFF          REC(TURN_OFF_EARLY_Z_TEST)
#define REC_CENTRE_W             REC(FRAGMENT_SHADER_USES_REAL_PIXEL_CENTRE_W_IN_ADDITION_TO_CENTROID_W2)
#define REC_READS_PRIM_ID        REC(ANY_SHADER_READS_HARDWARE_WRITTEN_PRIMITIVE_ID)
#define REC_INSERT_PRIM_ID       REC(INSERT_PRIMITIVE_ID_AS_FIRST_VARYING_TO_FRAGMENT_SHADER)
#define REC_WAIT_FIRST_THRSW     REC(DO_SCOREBOARD_WAIT_ON_FIRST_THREAD_SWITCH)
#define REC_DISABLE_IMPLICIT     REC(DISABLE_IMPLICIT_POINT_LINE_VARYINGS)
#define REC_VARYINGS             REC(NUMBER_OF_VARYINGS_IN_FRAGMENT_SHADER)
#define REC_CS_OUT               REC(COORDINATE_SHADER_OUTPUT_VPM_SEGMENT_SIZE)
#define REC_CS_IN                REC(COORDINATE_SHADER_INPUT_VPM_SEGMENT_SIZE)
#define REC_VS_OUT               REC(VERTEX_SHADER_OUTPUT_VPM_SEGMENT_SIZE)
#define REC_VS_IN                REC(VERTEX_SHADER_INPUT_VPM_SEGMENT_SIZE)
#define REC_FS_4WAY              REC(FRAGMENT_SHADER_4_WAY_THREADABLE)
#define REC_VS_4WAY              REC(VERTEX_SHADER_4_WAY_THREADABLE)
#define REC_CS_4WAY              REC(COORDINATE_SHADER_4_WAY_THREADABLE)
#define REC_FS_FINAL             REC(FRAGMENT_SHADER_START_IN_FINAL_THREAD_SECTION)
#define REC_VS_FINAL             REC(VERTEX_SHADER_START_IN_FINAL_THREAD_SECTION)
#define REC_CS_FINAL             REC(COORDINATE_SHADER_START_IN_FINAL_THREAD_SECTION)
#define REC_CS_VID               REC(VERTEX_ID_READ_BY_COORDINATE_SHADER)
#define REC_CS_IID               REC(INSTANCE_ID_READ_BY_COORDINATE_SHADER)
#define REC_VS_VID               REC(VERTEX_ID_READ_BY_VERTEX_SHADER)
#define REC_VS_IID               REC(INSTANCE_ID_READ_BY_VERTEX_SHADER)
#define REC_FS_NANS              REC_PROPAGATE_NANS
#define REC_VS_NANS              REC_PROPAGATE_NANS
#define REC_CS_NANS              REC_PROPAGATE_NANS
#define REC_SAMPLE_RATE          0

#else /* Mesa's */

static struct v3d_uniform vs_u[MESA71_TRI_VS_UNIFORM_COUNT], cs_u[MESA71_TRI_CS_UNIFORM_COUNT];

/* the header's uniform kinds are Mesa's QUNIFORM_* contents numbers; only a
 * constant keeps its word, the viewport entries are the builder's */
static void mesa_uniforms(struct v3d_uniform *u, const uint32_t *word, const uint8_t *kind, uint32_t n)
{
	for (uint32_t i = 0; i < n; i++) {
		u[i].contents = kind[i];
		u[i].data = kind[i] == MESA71_TRI_UNIFORM_CONSTANT ? word[i] : 0;
	}
}

void tri_shaders_get(struct tri_shaders *s)
{
	memset(s, 0, sizeof(*s));
	s->name = "mesa";
	s->code[0] = mesa71_tri_fs_code;
	s->code[1] = mesa71_tri_vs_code;
	s->code[2] = mesa71_tri_cs_code;
	s->words[0] = MESA71_TRI_FS_CODE_COUNT;
	s->words[1] = MESA71_TRI_VS_CODE_COUNT;
	s->words[2] = MESA71_TRI_CS_CODE_COUNT;
	mesa_uniforms(vs_u, mesa71_tri_vs_uniform, mesa71_tri_vs_uniform_kind, MESA71_TRI_VS_UNIFORM_COUNT);
	mesa_uniforms(cs_u, mesa71_tri_cs_uniform, mesa71_tri_cs_uniform_kind, MESA71_TRI_CS_UNIFORM_COUNT);
	/* MESA71_TRI_FS_UNIFORM_COUNT is 0: no FS stream. A stage that is not
	 * 4-way threadable would be taken as 2-way (the record says only "4-way
	 * or not"); Mesa's are all 4-way here. */
	s->fs = (struct v3d_fs_prog){
		.threads = MESA71_TRI_FS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_FS_FINAL_THREAD_SECTION,
		.num_inputs = MESA71_TRI_REC_VARYINGS,
		.writes_z = MESA71_TRI_REC_FS_Z_WRITES,
		.disable_ez = MESA71_TRI_REC_EARLY_Z_OFF,
		.uses_center_w = MESA71_TRI_REC_FS_USES_REAL_CENTRE_W,
		.lock_scoreboard_on_first_thrsw = MESA71_TRI_REC_SCOREBOARD_WAIT_FIRST_THRSW,
		.uses_implicit_point_line_varyings = !MESA71_TRI_REC_DISABLE_IMPLICIT_PL_VARYINGS,
		.uses_pid = MESA71_TRI_REC_INSERT_PRIM_ID,
	};
	s->vs = (struct v3d_vs_prog){
		.threads = MESA71_TRI_VS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_VS_FINAL_THREAD_SECTION,
		.vpm_input_size = MESA71_TRI_REC_VS_IN_VPM,
		.vpm_output_size = MESA71_TRI_REC_VS_OUT_VPM,
		.vcm_cache_size = MESA71_TRI_VCM_CACHE_BATCHES,
		.uniforms = vs_u,
		.num_uniforms = MESA71_TRI_VS_UNIFORM_COUNT,
	};
	s->cs = (struct v3d_vs_prog){
		.threads = MESA71_TRI_CS_FOUR_THREADS ? 4 : 2,
		.single_seg = MESA71_TRI_CS_FINAL_THREAD_SECTION,
		.vpm_input_size = MESA71_TRI_REC_CS_IN_VPM,
		.vpm_output_size = MESA71_TRI_REC_CS_OUT_VPM,
		.vcm_cache_size = MESA71_TRI_VCM_CACHE_BATCHES,
		.uniforms = cs_u,
		.num_uniforms = MESA71_TRI_CS_UNIFORM_COUNT,
	};
	for (uint32_t i = 0; i < MESA71_TRI_ATTR_COUNT && i < TRI_SHADER_ATTRS; i++) {
		s->vs.vattr_sizes[i] = mesa71_tri_attr[i].vs_values;
		s->cs.vattr_sizes[i] = mesa71_tri_attr[i].cs_values;
		s->attr_components[i] = mesa71_tri_attr[i].vec_size;
		s->attr_stride[i] = mesa71_tri_attr[i].stride;
	}
}

#define REC_POINT_SIZE       MESA71_TRI_REC_POINT_SIZE
#define REC_ENABLE_CLIPPING  MESA71_TRI_REC_ENABLE_CLIPPING
#define REC_CS_MIN_OUT       MESA71_TRI_REC_CS_MIN_OUT
#define REC_CS_MIN_IN        MESA71_TRI_REC_CS_MIN_IN
#define REC_VS_MIN_OUT       MESA71_TRI_REC_VS_MIN_OUT
#define REC_VS_MIN_IN        MESA71_TRI_REC_VS_MIN_IN
#define REC_FS_Z_WRITES      MESA71_TRI_REC_FS_Z_WRITES
#define REC_EARLY_Z_OFF      MESA71_TRI_REC_EARLY_Z_OFF
#define REC_CENTRE_W         MESA71_TRI_REC_FS_USES_REAL_CENTRE_W
#define REC_SAMPLE_RATE      MESA71_TRI_REC_SAMPLE_RATE_SHADING
#define REC_READS_PRIM_ID    MESA71_TRI_REC_INSERT_PRIM_ID /* the builder's rule: both are uses_pid */
#define REC_INSERT_PRIM_ID   MESA71_TRI_REC_INSERT_PRIM_ID
#define REC_WAIT_FIRST_THRSW MESA71_TRI_REC_SCOREBOARD_WAIT_FIRST_THRSW
#define REC_DISABLE_IMPLICIT MESA71_TRI_REC_DISABLE_IMPLICIT_PL_VARYINGS
#define REC_VARYINGS         MESA71_TRI_REC_VARYINGS
#define REC_CS_OUT           MESA71_TRI_REC_CS_OUT_VPM
#define REC_CS_IN            MESA71_TRI_REC_CS_IN_VPM
#define REC_VS_OUT           MESA71_TRI_REC_VS_OUT_VPM
#define REC_VS_IN            MESA71_TRI_REC_VS_IN_VPM
#define REC_FS_4WAY          MESA71_TRI_FS_FOUR_THREADS
#define REC_VS_4WAY          MESA71_TRI_VS_FOUR_THREADS
#define REC_CS_4WAY          MESA71_TRI_CS_FOUR_THREADS
#define REC_FS_FINAL         MESA71_TRI_FS_FINAL_THREAD_SECTION
#define REC_VS_FINAL         MESA71_TRI_VS_FINAL_THREAD_SECTION
#define REC_CS_FINAL         MESA71_TRI_CS_FINAL_THREAD_SECTION
#define REC_FS_NANS          MESA71_TRI_FS_PROPAGATE_NANS
#define REC_VS_NANS          MESA71_TRI_VS_PROPAGATE_NANS
#define REC_CS_NANS          MESA71_TRI_CS_PROPAGATE_NANS
/* the four id bits: MESA71_TRI_REC_FLAGS (0x00208002) has none of them */
#define REC_CS_VID           ((MESA71_TRI_REC_FLAGS >> 2) & 1u)
#define REC_CS_IID           ((MESA71_TRI_REC_FLAGS >> 3) & 1u)
#define REC_VS_VID           ((MESA71_TRI_REC_FLAGS >> 7) & 1u)
#define REC_VS_IID           ((MESA71_TRI_REC_FLAGS >> 8) & 1u)

#endif /* RPI5_TRI_OURS */

uint32_t tri_shaders_rec_check(const struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX *rec,
			       const char **first)
{
	const struct {
		const char *name;
		uint32_t got, want;
	} f[] = {
#define F(field, want) { #field, (uint32_t)rec->field, (uint32_t)(want) }
		F(point_size_in_shaded_vertex_data, REC_POINT_SIZE),
		F(enable_clipping, REC_ENABLE_CLIPPING),
		F(vertex_id_read_by_coordinate_shader, REC_CS_VID),
		F(instance_id_read_by_coordinate_shader, REC_CS_IID),
		F(base_instance_id_read_by_coordinate_shader, 0),
		F(cs_basevertex, 0),
		F(cs_drawindex, 0),
		F(vertex_id_read_by_vertex_shader, REC_VS_VID),
		F(instance_id_read_by_vertex_shader, REC_VS_IID),
		F(base_instance_id_read_by_vertex_shader, 0),
		F(vs_basevertex, 0),
		F(vs_drawindex, 0),
		F(fragment_shader_does_z_writes, REC_FS_Z_WRITES),
		F(turn_off_early_z_test, REC_EARLY_Z_OFF),
		F(fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2, REC_CENTRE_W),
		F(enable_sample_rate_shading, REC_SAMPLE_RATE),
		F(any_shader_reads_hardware_written_primitive_id, REC_READS_PRIM_ID),
		F(insert_primitive_id_as_first_varying_to_fragment_shader, REC_INSERT_PRIM_ID),
		F(turn_off_scoreboard, 0),
		F(do_scoreboard_wait_on_first_thread_switch, REC_WAIT_FIRST_THRSW),
		F(disable_implicit_point_line_varyings, REC_DISABLE_IMPLICIT),
		F(no_prim_pack, 0),
		F(never_defer_fep_depth_writes, 0),
		F(number_of_varyings_in_fragment_shader, REC_VARYINGS),
		F(coordinate_shader_output_vpm_segment_size, REC_CS_OUT),
		F(min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size,
		  REC_CS_MIN_OUT),
		F(coordinate_shader_input_vpm_segment_size, REC_CS_IN),
		F(min_coord_shader_input_segments_required_in_play, REC_CS_MIN_IN),
		F(vertex_shader_output_vpm_segment_size, REC_VS_OUT),
		F(min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size,
		  REC_VS_MIN_OUT),
		F(vertex_shader_input_vpm_segment_size, REC_VS_IN),
		F(min_vertex_shader_input_segments_required_in_play, REC_VS_MIN_IN),
		F(fragment_shader_4_way_threadable, REC_FS_4WAY),
		F(fragment_shader_start_in_final_thread_section, REC_FS_FINAL),
		F(fragment_shader_propagate_nans, REC_FS_NANS),
		F(vertex_shader_4_way_threadable, REC_VS_4WAY),
		F(vertex_shader_start_in_final_thread_section, REC_VS_FINAL),
		F(vertex_shader_propagate_nans, REC_VS_NANS),
		F(coordinate_shader_4_way_threadable, REC_CS_4WAY),
		F(coordinate_shader_start_in_final_thread_section, REC_CS_FINAL),
		F(coordinate_shader_propagate_nans, REC_CS_NANS),
#undef F
	};
	uint32_t bad = 0;

	*first = NULL;
	for (size_t i = 0; i < sizeof(f) / sizeof(f[0]); i++) {
		if (f[i].got != f[i].want && bad++ == 0)
			*first = f[i].name;
	}
	return bad;
}
