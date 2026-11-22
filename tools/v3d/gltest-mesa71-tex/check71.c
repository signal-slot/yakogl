/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * check71: the part of tools/v3d/check71.d/gltest_mesa71_tex.sh that needs no
 * Mesa compiler, only the vendored reference build (tools/v3d/mesa_ref.sh) or
 * a MESA_V3D_TREE's build-v3d. For each committed header of this directory:
 *
 *  - the shader state record's field macros (..._REC_<FIELD>), packed with
 *    Mesa's own generated 7.1 packer (GL Shader State Record Draw Index, the
 *    BCM2712 D0 layout), give the committed 32 bytes (..._rec[8]) and the
 *    24-bit flag word (..._REC_FLAGS);
 *  - Mesa's 7.1 QPU disassembler (v3d_qpu_disasm at 7.1.10.2) on the
 *    committed code words gives the lines v3d_shc printed into <name>.disasm:
 *    written to <out>/<name>.code in v3d_shc's own format (v3d_shc.c
 *    disasm()), the caller compares them with the committed file.
 *
 *   check71 <out dir>     exit 0 when every record equals, 1 otherwise
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "common/v3d_device_info.h"
#include "qpu/qpu_disasm.h"
#include "util/ralloc.h"

#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#include "cle/v3d_packet_v71_pack.h"

#include "texture.h"
#include "texture2.h"
#include "cube.h"

/* Every field record71.c prints, in its order (the header's REC_ macros). */
#define REC_FIELDS(X, P)                                                                 \
	X(P, point_size_in_shaded_vertex_data, POINT_SIZE_IN_SHADED_VERTEX_DATA)           \
	X(P, enable_clipping, ENABLE_CLIPPING)                                             \
	X(P, vertex_id_read_by_coordinate_shader, VERTEX_ID_READ_BY_COORDINATE_SHADER)     \
	X(P, instance_id_read_by_coordinate_shader, INSTANCE_ID_READ_BY_COORDINATE_SHADER) \
	X(P, base_instance_id_read_by_coordinate_shader,                                   \
	  BASE_INSTANCE_ID_READ_BY_COORDINATE_SHADER)                                      \
	X(P, cs_basevertex, CS_BASEVERTEX)                                                 \
	X(P, cs_drawindex, CS_DRAWINDEX)                                                   \
	X(P, vertex_id_read_by_vertex_shader, VERTEX_ID_READ_BY_VERTEX_SHADER)             \
	X(P, instance_id_read_by_vertex_shader, INSTANCE_ID_READ_BY_VERTEX_SHADER)         \
	X(P, base_instance_id_read_by_vertex_shader, BASE_INSTANCE_ID_READ_BY_VERTEX_SHADER) \
	X(P, vs_basevertex, VS_BASEVERTEX)                                                 \
	X(P, vs_drawindex, VS_DRAWINDEX)                                                   \
	X(P, fragment_shader_does_z_writes, FRAGMENT_SHADER_DOES_Z_WRITES)                 \
	X(P, turn_off_early_z_test, TURN_OFF_EARLY_Z_TEST)                                 \
	X(P, fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2,          \
	  FRAGMENT_SHADER_USES_REAL_PIXEL_CENTRE_W_IN_ADDITION_TO_CENTROID_W2)             \
	X(P, enable_sample_rate_shading, ENABLE_SAMPLE_RATE_SHADING)                       \
	X(P, any_shader_reads_hardware_written_primitive_id,                               \
	  ANY_SHADER_READS_HARDWARE_WRITTEN_PRIMITIVE_ID)                                  \
	X(P, insert_primitive_id_as_first_varying_to_fragment_shader,                      \
	  INSERT_PRIMITIVE_ID_AS_FIRST_VARYING_TO_FRAGMENT_SHADER)                         \
	X(P, turn_off_scoreboard, TURN_OFF_SCOREBOARD)                                     \
	X(P, do_scoreboard_wait_on_first_thread_switch,                                    \
	  DO_SCOREBOARD_WAIT_ON_FIRST_THREAD_SWITCH)                                       \
	X(P, disable_implicit_point_line_varyings, DISABLE_IMPLICIT_POINT_LINE_VARYINGS)   \
	X(P, no_prim_pack, NO_PRIM_PACK)                                                   \
	X(P, never_defer_fep_depth_writes, NEVER_DEFER_FEP_DEPTH_WRITES)                   \
	X(P, number_of_varyings_in_fragment_shader, NUMBER_OF_VARYINGS_IN_FRAGMENT_SHADER) \
	X(P, coordinate_shader_output_vpm_segment_size,                                    \
	  COORDINATE_SHADER_OUTPUT_VPM_SEGMENT_SIZE)                                       \
	X(P, min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size, \
	  MIN_COORD_SHADER_OUTPUT_SEGMENTS_REQUIRED_IN_PLAY_IN_ADDITION_TO_VCM_CACHE_SIZE) \
	X(P, coordinate_shader_input_vpm_segment_size, COORDINATE_SHADER_INPUT_VPM_SEGMENT_SIZE) \
	X(P, min_coord_shader_input_segments_required_in_play,                             \
	  MIN_COORD_SHADER_INPUT_SEGMENTS_REQUIRED_IN_PLAY)                                \
	X(P, vertex_shader_output_vpm_segment_size, VERTEX_SHADER_OUTPUT_VPM_SEGMENT_SIZE) \
	X(P, min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size, \
	  MIN_VERTEX_SHADER_OUTPUT_SEGMENTS_REQUIRED_IN_PLAY_IN_ADDITION_TO_VCM_CACHE_SIZE) \
	X(P, vertex_shader_input_vpm_segment_size, VERTEX_SHADER_INPUT_VPM_SEGMENT_SIZE)   \
	X(P, min_vertex_shader_input_segments_required_in_play,                            \
	  MIN_VERTEX_SHADER_INPUT_SEGMENTS_REQUIRED_IN_PLAY)                               \
	X(P, fragment_shader_4_way_threadable, FRAGMENT_SHADER_4_WAY_THREADABLE)           \
	X(P, fragment_shader_start_in_final_thread_section,                                \
	  FRAGMENT_SHADER_START_IN_FINAL_THREAD_SECTION)                                   \
	X(P, fragment_shader_propagate_nans, FRAGMENT_SHADER_PROPAGATE_NANS)               \
	X(P, vertex_shader_4_way_threadable, VERTEX_SHADER_4_WAY_THREADABLE)               \
	X(P, vertex_shader_start_in_final_thread_section,                                  \
	  VERTEX_SHADER_START_IN_FINAL_THREAD_SECTION)                                     \
	X(P, vertex_shader_propagate_nans, VERTEX_SHADER_PROPAGATE_NANS)                   \
	X(P, coordinate_shader_4_way_threadable, COORDINATE_SHADER_4_WAY_THREADABLE)       \
	X(P, coordinate_shader_start_in_final_thread_section,                              \
	  COORDINATE_SHADER_START_IN_FINAL_THREAD_SECTION)                                 \
	X(P, coordinate_shader_propagate_nans, COORDINATE_SHADER_PROPAGATE_NANS)

#define SET_FIELD(P, field, FIELD) s.field = P##_REC_##FIELD;
#define ONE_FIELD(P, field, FIELD) +1
#define NUM_FIELDS (0 REC_FIELDS(ONE_FIELD, _))

/* The Pi 5's identity, as v3d_shc was run (--device 71 --rev 10 --compat-rev 2). */
static const struct v3d_device_info devinfo = { .ver = 71, .rev = 10, .compat_rev = 2,
						.vpm_size = 64 * 1024, .qpu_count = 16 };

static int check_record(const char *name, const struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX *s,
			const uint32_t want[8], uint32_t want_flags)
{
	uint8_t b[V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_length];
	int bad = 0;

	if (sizeof(b) != 32) {
		printf("FAIL: %s: Mesa's draw-index record is %zu bytes, not 32\n", name, sizeof(b));
		return 1;
	}
	memset(b, 0, sizeof(b));
	V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_pack(NULL, b, s);
	for (unsigned i = 0; i < 8; i++) {
		uint32_t w = b[4 * i] | b[4 * i + 1] << 8 | b[4 * i + 2] << 16 | (uint32_t)b[4 * i + 3] << 24;
		if (w != want[i]) {
			printf("FAIL: %s: record word %u packs to 0x%08x, the header has 0x%08x\n", name, i,
			       w, want[i]);
			bad = 1;
		}
		if (i == 0 && (w & 0xffffff) != want_flags) {
			printf("FAIL: %s: REC_FLAGS 0x%08x, the packed flag word is 0x%08x\n", name,
			       want_flags, w & 0xffffff);
			bad = 1;
		}
	}
	if (!bad)
		printf("%s: the %u record fields pack, with Mesa's 7.1 packer, to the committed 32 bytes "
		       "and flag word 0x%06x\n",
		       name, (unsigned)NUM_FIELDS, want_flags);
	return bad;
}

static int disasm_stage(FILE *f, const char *stage, const uint64_t *code, unsigned n)
{
	fprintf(f, "--- %s: %u instructions\n", stage, n);
	for (unsigned i = 0; i < n; i++) {
		const char *s = v3d_qpu_disasm(&devinfo, code[i]);
		if (!s)
			return 1;
		fprintf(f, "%4u 0x%016llx %s\n", i, (unsigned long long)code[i], s);
		ralloc_free((void *)s);
	}
	return 0;
}

static int disasm(const char *out, const char *name, const uint64_t *fs, unsigned nfs,
		  const uint64_t *vs, unsigned nvs, const uint64_t *cs, unsigned ncs)
{
	char path[4096];
	snprintf(path, sizeof(path), "%s/%s.code", out, name);
	FILE *f = fopen(path, "w");
	if (!f) {
		printf("FAIL: cannot write %s\n", path);
		return 1;
	}
	int bad = disasm_stage(f, "fs", fs, nfs) | disasm_stage(f, "vs", vs, nvs) |
		  disasm_stage(f, "cs", cs, ncs);
	if (fclose(f) || bad) {
		printf("FAIL: %s: the disassembly of the committed words was not written\n", name);
		return 1;
	}
	return 0;
}

#define CHECK_PROGRAM(P, p, name)                                                              \
	do {                                                                                   \
		struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX s = {                           \
			V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_header                         \
		};                                                                             \
		REC_FIELDS(SET_FIELD, P)                                                       \
		fail |= check_record(name, &s, p##_rec, P##_REC_FLAGS);                        \
		fail |= disasm(argv[1], name, p##_fs_code, P##_FS_CODE_COUNT, p##_vs_code,     \
			       P##_VS_CODE_COUNT, p##_cs_code, P##_CS_CODE_COUNT);             \
		programs++;                                                                    \
	} while (0)

int main(int argc, char **argv)
{
	int fail = 0;
	unsigned programs = 0;

	if (argc != 2) {
		fprintf(stderr, "usage: check71 <out dir>\n");
		return 2;
	}
	if (!v3d_device_has_draw_index(&devinfo)) {
		printf("FAIL: Mesa says 7.1.10 has no draw-index record\n");
		return 1;
	}
	CHECK_PROGRAM(MESA71_TEXTURE, mesa71_texture, "texture");
	CHECK_PROGRAM(MESA71_TEXTURE2, mesa71_texture2, "texture2");
	CHECK_PROGRAM(MESA71_CUBE, mesa71_cube, "cube");
	printf("check71: %u programs\n", programs);
	return fail;
}
