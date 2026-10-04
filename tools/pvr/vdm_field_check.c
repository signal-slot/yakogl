/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * vdm_field_check: the fields of the VDM emitters, pvr_ppp_build_block and
 * pvr_ppp_build_terminate_block (driver/src/pvr_vdm_ppp_gen.c) refuse a value
 * that does not fit instead of masking it. For every size field:
 *
 *  1. the largest value that fits is accepted, and lands in the field's bits
 *     (the whole word compared with the value built here from the widths);
 *  2. one byte more (the smallest input one unit past the bound) is refused:
 *     PVR_VDM_FIELD_REFUSED, nothing written at dw, and the refusal names the
 *     field, the value in the field's units and the field's max;
 *  3. a byte count near 2^32, which the old 32-bit round-up wrapped to a
 *     small size, is refused too.
 *
 * cam_size and vs_max_instances come from pvr_vdm_calc_cam_size_bxs4_64's
 * table (at most 31 and 16): no input reaches their bound, so the check is
 * that every output size from 0 to the VDM_STATE4 bound gives a value that
 * fits and lands where the table says.
 *
 * The widths are Mesa 26.2.1 src/imagination/csbgen/rogue/vdm.xml:
 *   PDS_STATE0  usc_common_size [16:24] 64 B, pds_temp_size [6:9] 16 B,
 *               pds_data_size [0:5] 16 B                       (:100-116)
 *   VDM_STATE0  cam_size [7:14]                                (:152-167)
 *   VDM_STATE4  vs_output_size [0:7] 4 B                       (:181-185)
 *   VDM_STATE5  vs_max_instances [25:29], vs_usc_unified_size [10:15] 16 B,
 *               vs_pds_temp_size [6:9] 16 B, vs_pds_data_size [0:5] 16 B
 *                                                              (:187-202)
 * and ppp.xml:
 *   OUTPUT_SEL      vtxsize [24:31] dwords                     (:195)
 *   STATE_VARYING0  f32_npc [16:23], f32_flat [8:15], f32_linear [0:7] (:222-226)
 *   STATE_PDS_SIZEINFO1 pds_texturestatesize [16:22] 4 dw of PDS data,
 *                   pds_varyingsize [10:15] 4 dw, usc_varyingsize [4:9] 16 dw,
 *                   pds_tempsize [0:3] 4 temps                 (:304-318)
 *   STATE_PDS_SIZEINFO2 usc_sharedsize [23:31] 16 registers    (:321-323)
 * For the PPP block, 4 also checks that a value in range gives the word the
 * old masked writer gave, for every value of every field: the stream is
 * unchanged for anything that fits. The exception is pds_texturestatesize:
 * the old writer divided BYTES by 4, Mesa divides DWORDS by 4
 * (pvr_arch_cmd_buffer.c:3667, pvr_device.c:949 data_size = bytes / 4), so
 * its sweep checks Mesa's rule and 4b pins the quarter.
 *
 * The non-size fields (6-9) refuse the same way, with unit 0 (a plain value):
 * for each, the largest value that fits is accepted and lands in its own
 * bits only (every other word as with the field at 0; the back-face ISPA
 * copies the front objtype, passtype, dcmpmode and pointlinewidth), one past it is
 * refused by name with nothing written, and every value in range gives the
 * word the old masked writer gave. The fields, widths from ppp.xml:
 *   STATE_ISPCTL upass [22:25] (:133); STATE_ISPA objtype [28:31],
 *   passtype [24:26], dcmpmode [17:19], pointlinewidth [8:15], sref [0:7]
 *   (:145-160); STATE_ISPB scmpmode [25:27], sop1-3 [22:24] [19:21] [16:18],
 *   scmpmask [8:15], swmask [0:7] (:162-169), both faces; STATE_ISPDBSC
 *   dbindex [16:31], scindex [0:15] (:189-191); REGION_CLIP0/1 left, right,
 *   top, bottom 9 bits (:174-182); STATE_PPP_CTRL clip_mode [10:11],
 *   flatshade_vtx [7:8], cullmode [0:1] (:328-340); STATE_TERMINATE0
 *   clip_right [18:26], clip_bottom [0:8] (:236-246); the PDS base
 *   addresses [4:31] shift 4 (:277-298: a 16-byte aligned offset);
 * and vdm.xml: PPP_STATE0 word_count [8:15], addrmsb [0:7] shift 32,
 *   PPP_STATE1 addrlsb [2:31] shift 2 (:90-98); PDS_STATE1/2 and
 *   VDM_STATE2/3 addresses [4:31] shift 4 (:118-128, :173-179);
 *   VDM_STATE0 flatshade_control [1:2] (:165); INDEX_LIST0
 *   primitive_topology [8:11], index_base_addrmsb [0:7] shift 32 (:218-219);
 *   INDEX_LIST1 bits 1:0 (4-byte aligned: our recipe, see the generator).
 * A field that is not emitted (the back face one-sided, the ISPDBSC indices
 * disabled, the clip rectangle off) is not checked.
 *
 * VDM_STATE5.vs_pds_temp_size counts 16-byte units of 32-bit PDS temps (Mesa
 * pvr_arch_cmd_buffer.c:8229: DIV_ROUND_UP(temps_required << 2, 16)); the
 * vertex PDS program the backend builds uses 2 temps for any stream count
 * and any USC temp count, so its field is 1 (pinned against the generator).
 *
 * The fragment kick-USC PDS program (pvr_pds_render_kick_usc_program) declares
 * the USC temps per instance in its DOUTU src0, the first two data dwords:
 * the temps field is bits [35:40] in 4-temp blocks (Mesa 26.2.1
 * pvr_rogue_pds_defs.h:1145,1172,1176, pvr_pds.c pvr_pds_setup_doutu), so a
 * count lands as DIV_ROUND_UP(temps, 4) and one past PVR_PDS_DOUTU_MAX_TEMPS
 * is refused with nothing written. PVR_BACKEND_OPT_FS_TEMPS_MIN reaches the
 * hardware through this word only.
 *
 * pvr_isp_build_scissor_word checks the same way: the frag kick's
 * ISP_SCISSOR_BASE table (a memory table the IPF reads, not a PPP/VDM
 * stream word, but built by a checked emitter here too) is two dwords of
 * 16-bit fields, widths from csbgen rogue/ipf.xml:150-166:
 *   SCISSOR_WORD_0 scw0_xmin [31:16], scw0_xmax [15:0];
 *   SCISSOR_WORD_1 scw1_ymin [31:16], scw1_ymax [15:0].
 * Each coordinate's bound is accepted and one past it is refused by name
 * with nothing written, same as every other field here.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src \
 *      tools/pvr/vdm_field_check.c driver/src/pvr_vdm_ppp_gen.c \
 *      driver/src/pvr_pds_render_gen.c -o build/pvr-check/vdm_field_check
 *
 * Exit status 0 only when every check passed AND the number run is the number
 * planned.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "pvr_pds_pixel_event_gen.h" /* PVR_PDS_DOUTU_MAX_TEMPS */
#include "pvr_pds_render_gen.h"
#include "pvr_vdm_ppp_gen.h"

#define PLANNED 459

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("FAIL: %s\n", what);
	}
}

#define POISON 0xa5a5a5a5u

static void poison(uint32_t *dw, int n)
{
	for (int i = 0; i < n; i++)
		dw[i] = POISON;
}

static bool untouched(const uint32_t *dw, int n)
{
	for (int i = 0; i < n; i++)
		if (dw[i] != POISON)
			return false;
	return true;
}

/* A refusal: the return, nothing written, and the refusal's four fields. */
static void refused(const char *what, int32_t rc, const uint32_t *dw, int n, const struct pvr_vdm_field_refusal *why,
		    const char *field, uint64_t value, uint32_t max, uint32_t unit)
{
	char m[160];

	snprintf(m, sizeof(m), "%s: refused (rc %d)", what, (int)rc);
	check(rc == PVR_VDM_FIELD_REFUSED, m);
	snprintf(m, sizeof(m), "%s: nothing written", what);
	check(untouched(dw, n), m);
	snprintf(m, sizeof(m), "%s: names %s (got %s)", what, field, why->field ? why->field : "(null)");
	check(why->field && strcmp(why->field, field) == 0, m);
	snprintf(m, sizeof(m), "%s: value %llu max %u unit %u (got %llu %u %u)", what, (unsigned long long)value, max,
		 unit, (unsigned long long)why->value, why->max, why->unit_bytes);
	check(why->value == value && why->max == max && why->unit_bytes == unit, m);
}

/* PDS_STATE0: block_type PDS_STATE_UPDATE (1) at 29. */
static uint32_t pds_state0(uint32_t common, uint32_t temp, uint32_t data)
{
	return (1u << 29) | (common << 16) | (temp << 6) | data;
}

static void pds_state(void)
{
	const struct pvr_vdm_pds_state ok = {
		.pds_data_offset = 0x100,
		.pds_code_offset = 0x200,
		.pds_data_size_bytes = 63 * 16,   /* 63 units: the 6-bit max */
		.pds_temps = 60,                  /* 240 bytes = 15 units: the 4-bit max */
		.usc_common_size_bytes = 511 * 64 /* 511 units: the 9-bit max */
	};
	struct pvr_vdm_field_refusal why;
	uint32_t dw[4];
	int32_t rc;

	/* 1: every field at its bound */
	poison(dw, 4);
	memset(&why, 0, sizeof(why));
	rc = pvr_vdm_emit_pds_state(dw, &ok, &why);
	check(rc == 3, "PDS_STATE0 at every bound: 3 dwords");
	check(dw[0] == pds_state0(511, 15, 63), "PDS_STATE0 at every bound: the word");
	check(dw[3] == POISON, "PDS_STATE0: writes 3 dwords, no more");
	check(why.field == NULL, "PDS_STATE0 accepted: no refusal named");

	/* 2: one byte past each bound */
	{
		struct pvr_vdm_pds_state s = ok;

		s.usc_common_size_bytes = 511 * 64 + 1;
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("usc_common_size 32705 B", rc, dw, 4, &why, "PDS_STATE0.usc_common_size", 512, 511, 64);
	}
	{
		struct pvr_vdm_pds_state s = ok;

		s.pds_temps = 61; /* 244 bytes: 16 units */
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("pds_temps 61", rc, dw, 4, &why, "PDS_STATE0.pds_temp_size", 16, 15, 16);
	}
	{
		struct pvr_vdm_pds_state s = ok;

		s.pds_data_size_bytes = 63 * 16 + 1;
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("pds_data_size 1009 B", rc, dw, 4, &why, "PDS_STATE0.pds_data_size", 64, 63, 16);
	}
	/* 3: near 2^32 (the old round-up wrapped 0xffffffff + 63 to 62, i.e. 0
	 * units); temps * 4 past 2^32 likewise. */
	{
		struct pvr_vdm_pds_state s = ok;

		s.usc_common_size_bytes = 0xffffffffu;
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("usc_common_size 0xffffffff B", rc, dw, 4, &why, "PDS_STATE0.usc_common_size", 67108864, 511,
			64);
	}
	{
		struct pvr_vdm_pds_state s = ok;

		s.pds_temps = 0x40000000u; /* * 4 = 2^32: 0 in 32 bits */
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("pds_temps 2^30", rc, dw, 4, &why, "PDS_STATE0.pds_temp_size", 268435456, 15, 16);
	}
	/* a refusal with no `why` still refuses */
	{
		struct pvr_vdm_pds_state s = ok;

		s.pds_data_size_bytes = 0xffffffffu;
		poison(dw, 4);
		rc = pvr_vdm_emit_pds_state(dw, &s, NULL);
		check(rc == PVR_VDM_FIELD_REFUSED && untouched(dw, 4), "PDS_STATE0 refuses with why == NULL");
	}
}

/* VDM_STATE5 as the widths place it. */
static uint32_t vdm_state5(uint32_t inst, uint32_t unified, uint32_t temp, uint32_t data)
{
	return (inst << 25) | (unified << 10) | (temp << 6) | data;
}

static void vdm_state(void)
{
	const struct pvr_vdm_vs_state ok = {
		.vs_pds_data_offset = 0x300,
		.vs_pds_code_offset = 0x400,
		.vs_pds_data_size_bytes = 63 * 16, /* 63 units */
		.vs_pds_temps = 60,                /* 240 bytes = 15 units: the 4-bit max */
		.vs_output_size_bytes = 255 * 4,   /* 255 units: the 8-bit max */
		.vs_input_size_bytes = 63 * 16,    /* 63 units */
		.flatshade_control = 0,
	};
	struct pvr_vdm_field_refusal why;
	uint32_t dw[6], cam, inst;
	int32_t rc;

	/* 1 */
	pvr_vdm_calc_cam_size_bxs4_64(ok.vs_output_size_bytes, 1, &cam, &inst);
	poison(dw, 6);
	memset(&why, 0, sizeof(why));
	rc = pvr_vdm_emit_vdm_state(dw, &ok, &why);
	check(rc == 5, "VDM_STATE at every bound: 5 dwords");
	check(dw[3] == 255, "VDM_STATE4 at its bound: 255");
	check(dw[4] == vdm_state5(inst, 63, 15, 63), "VDM_STATE5 at every bound: the word");
	check(((dw[0] >> 7) & 0xffu) == cam, "VDM_STATE0.cam_size as the table gives it");
	check(dw[5] == POISON, "VDM_STATE: writes 5 dwords, no more");
	check(why.field == NULL, "VDM_STATE accepted: no refusal named");

	/* 2 */
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_output_size_bytes = 255 * 4 + 1;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_output_size 1021 B", rc, dw, 6, &why, "VDM_STATE4.vs_output_size", 256, 255, 4);
	}
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_input_size_bytes = 63 * 16 + 1;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_input_size 1009 B", rc, dw, 6, &why, "VDM_STATE5.vs_usc_unified_size", 64, 63, 16);
	}
	{
		/* input 0 = the output size: an output of 1009..1020 bytes fits
		 * VDM_STATE4 but not vs_usc_unified_size */
		struct pvr_vdm_vs_state s = ok;

		s.vs_input_size_bytes = 0;
		s.vs_output_size_bytes = 63 * 16 + 4;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_input_size 0, output 1012 B", rc, dw, 6, &why, "VDM_STATE5.vs_usc_unified_size", 64, 63,
			16);
	}
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_pds_temps = 61; /* 244 bytes: 16 units */
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_pds_temps 61", rc, dw, 6, &why, "VDM_STATE5.vs_pds_temp_size", 16, 15, 16);
	}
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_pds_data_size_bytes = 63 * 16 + 1;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_pds_data_size 1009 B", rc, dw, 6, &why, "VDM_STATE5.vs_pds_data_size", 64, 63, 16);
	}
	/* 3 */
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_pds_data_size_bytes = 0xffffffffu;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_pds_data_size 0xffffffff B", rc, dw, 6, &why, "VDM_STATE5.vs_pds_data_size", 268435456,
			63, 16);
	}
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_output_size_bytes = 0xffffffffu;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("vs_output_size 0xffffffff B", rc, dw, 6, &why, "VDM_STATE4.vs_output_size", 1073741824, 255,
			4);
	}
	{
		struct pvr_vdm_vs_state s = ok;

		s.vs_pds_temps = 0xffffffffu;
		poison(dw, 6);
		rc = pvr_vdm_emit_vdm_state(dw, &s, NULL);
		check(rc == PVR_VDM_FIELD_REFUSED && untouched(dw, 6), "VDM_STATE refuses with why == NULL");
	}

	/* cam_size and vs_max_instances over every output size VDM_STATE4
	 * takes: in range, and where the table puts them (one check each). */
	{
		bool cam_ok = true, inst_ok = true;

		for (uint32_t units = 0; units <= 255; units++) {
			struct pvr_vdm_vs_state s = ok;

			s.vs_output_size_bytes = units * 4;
			pvr_vdm_calc_cam_size_bxs4_64(s.vs_output_size_bytes, 1, &cam, &inst);
			poison(dw, 6);
			rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
			cam_ok &= rc == 5 && cam <= 255 && ((dw[0] >> 7) & 0xffu) == cam;
			inst_ok &= rc == 5 && inst <= 31 && (dw[4] >> 25) == inst;
		}
		check(cam_ok, "VDM_STATE0.cam_size fits for every vs_output_size 0..255");
		check(inst_ok, "VDM_STATE5.vs_max_instances fits for every vs_output_size 0..255");
	}
}

/* VDM_STATE5.vs_pds_temp_size is in 16-byte units of 32-bit temps: 4 temps
 * are 1 unit, 5 are 2 (the old DRU(temps, 16) gave 1: a quarter of the
 * temps). The backend's vertex PDS program: 2 temps, 1 unit, whatever the
 * stream count, the id registers and the USC temp count (the largest, the
 * DOUTU limit, included). */
static void vdm_temp_units(void)
{
	static const uint32_t temps[] = { 1, 4, 5, 8, 9, 60 }, units[] = { 1, 1, 2, 2, 3, 15 };
	struct pvr_vdm_vs_state s = {
		.vs_pds_data_offset = 0x300,
		.vs_pds_code_offset = 0x400,
		.vs_pds_data_size_bytes = 32,
		.vs_output_size_bytes = 16,
	};
	struct pvr_vdm_field_refusal why = { 0 };
	uint32_t dw[6];
	bool ok = true;

	for (unsigned i = 0; i < sizeof(temps) / sizeof(temps[0]); i++) {
		s.vs_pds_temps = temps[i];
		ok &= pvr_vdm_emit_vdm_state(dw, &s, &why) == 5 && ((dw[4] >> 6) & 0xfu) == units[i];
	}
	check(ok, "VDM_STATE5.vs_pds_temp_size = DRU(temps * 4, 16): 1 4 5 8 9 60 -> 1 1 2 2 3 15");

	{
		struct pvr_pds_render_stream st[PVR_PDS_VERTEX_MAX_STREAMS];
		struct pvr_pds_render_vertex_info vi;
		uint32_t data[8 * PVR_PDS_VERTEX_MAX_STREAMS + 8], code[PVR_PDS_VERTEX_MAX_STREAMS + 4];
		bool temps_ok = true, field_ok = true, info_ok = true;

		memset(st, 0, sizeof(st));
		for (unsigned i = 0; i < PVR_PDS_VERTEX_MAX_STREAMS; i++) {
			st[i].address = 0x10000u + i * 0x1000u;
			st[i].stride = 16;
			st[i].dma_size = 16;
			st[i].dst_reg = i * 4u;
		}
		for (uint32_t n = 0; n <= PVR_PDS_VERTEX_MAX_STREAMS; n++) {
			for (int ids = 0; ids < 2; ids++) {
				memset(&vi, 0, sizeof(vi));
				if (pvr_pds_render_vertex_program_ids(0x1240, PVR_PDS_DOUTU_MAX_TEMPS, st, n, 0,
								      ids ? 64u : PVR_PDS_NO_ID_REG,
								      ids ? 65u : PVR_PDS_NO_ID_REG, data, code, &vi) != 0) {
					temps_ok = false;
					continue;
				}
				temps_ok &= vi.temps_used == 2;
				info_ok &= vi.vdm_pds_temp_size_units == 1;
				s.vs_pds_temps = vi.temps_used;
				field_ok &= pvr_vdm_emit_vdm_state(dw, &s, &why) == 5 && ((dw[4] >> 6) & 0xfu) == 1;
			}
		}
		check(temps_ok, "vertex PDS program: 2 temps for 0..16 streams, with and without ids, USC temps at the DOUTU max");
		check(field_ok, "vertex PDS program: VDM_STATE5.vs_pds_temp_size 1");
		check(info_ok, "vertex PDS program: info.vdm_pds_temp_size_units 1 (the same rule)");
	}
}

static void kick_usc_temps(void)
{
	static const uint32_t temps[] = { 0, 3, 15, 16, 17, 30, 120, PVR_PDS_DOUTU_MAX_TEMPS };
	uint32_t data[8], code[1];
	struct pvr_pds_render_kick_usc_info info;
	bool ok = true;

	for (unsigned i = 0; i < sizeof(temps) / sizeof(temps[0]); i++) {
		memset(&info, 0, sizeof(info));
		ok &= pvr_pds_render_kick_usc_program(0x200000, temps[i], 0, data, code, &info) == 0 &&
		      info.doutu_offset == 0 && data[0] == 0x200000u && ((data[1] >> 3) & 0x3fu) == (temps[i] + 3u) / 4u;
	}
	check(ok, "kick USC DOUTU temps field = DRU(temps, 4): 0 3 15 16 17 30 120 252 -> 0 1 4 4 5 8 30 63");

	poison(data, 8);
	poison(code, 1);
	check(pvr_pds_render_kick_usc_program(0x200000, PVR_PDS_DOUTU_MAX_TEMPS + 1u, 0, data, code, &info) != 0 &&
		      untouched(data, 8) && untouched(code, 1),
	      "kick USC: PVR_PDS_DOUTU_MAX_TEMPS + 1 temps refused, nothing written");
}

/* The PPP block's size fields. */
#define PPP_N ((int)PVR_PPP_BLOCK_MAX_DWORDS)

static struct pvr_ppp_state ppp_ok_state(void)
{
	struct pvr_ppp_state st;

	memset(&st, 0, sizeof(st));
	st.vtxsize = 255;
	st.f32_linear = 255;
	st.f32_flat = 255;
	st.f32_npc = 255;
	st.usc_varyingsize = 63;
	st.pds_varyingsize = 63;
	return st;
}

static const struct pvr_ppp_frag_inputs ppp_ok_in = {
	.pixel_program_code_offset = 0x2000,
	.texture_program_code_offset = 0x2040,
	.texture_program_data_offset = 0x3000,
	.texture_program_data_size_bytes = 127 * 16, /* 127 units of 4 dwords: the 7-bit max */
	.texture_program_temps = 60,                /* 15 units of 4 temps: the 4-bit max */
	.shared_reg_count = 511 * 16,               /* 511 units of 16 registers: the 9-bit max */
};

/* The word indices of a block with no viewport, one-sided, no ISPDBSC. */
enum { PPP_SIZEINFO1 = 6, PPP_SIZEINFO2 = 7, PPP_OUTPUT_SEL = 13, PPP_VARYING0 = 14, PPP_WORDS = 19 };

static void ppp_refused(const char *what, const struct pvr_ppp_frag_inputs *in, const struct pvr_ppp_state *st,
			const char *field, uint64_t value, uint32_t max, uint32_t unit)
{
	struct pvr_vdm_field_refusal why;
	uint32_t dw[PPP_N], n = 99;
	char m[160];
	int32_t rc;

	poison(dw, PPP_N);
	memset(&why, 0, sizeof(why));
	rc = pvr_ppp_build_block(in, st, dw, &n, &why);
	refused(what, rc, dw, PPP_N, &why, field, value, max, unit);
	snprintf(m, sizeof(m), "%s: *out_count 0 (got %u)", what, n);
	check(n == 0, m);
}

/* pds_texturestatesize by Mesa's rule: the data segment in dwords
 * (pvr_device.c:949), divided by UNIT_SIZE 4 (pvr_arch_cmd_buffer.c:3667). */
static uint32_t mesa_texstate_units(uint32_t tex_bytes)
{
	return (((tex_bytes + 3u) / 4u) + 3u) / 4u;
}

/* The old writer: the same fields masked to their bits; the texture-state
 * units come in already converted (see mesa_texstate_units). */
static uint32_t old_sizeinfo1(uint32_t tex_units, uint32_t temps, uint32_t usc_v, uint32_t pds_v)
{
	return (tex_units << 16) | ((temps + 3u) / 4u) | ((usc_v & 0x3fu) << 4) | ((pds_v & 0x3fu) << 10);
}

static void ppp_block(void)
{
	const struct pvr_ppp_state ok = ppp_ok_state();
	struct pvr_vdm_field_refusal why;
	uint32_t dw[PPP_N], n = 0;
	int32_t rc;

	/* 1 */
	poison(dw, PPP_N);
	memset(&why, 0, sizeof(why));
	rc = pvr_ppp_build_block(&ppp_ok_in, &ok, dw, &n, &why);
	check(rc == PPP_WORDS && n == PPP_WORDS, "PPP block at every bound: 19 dwords");
	check(dw[PPP_SIZEINFO1] == ((127u << 16) | (63u << 10) | (63u << 4) | 15u), "SIZEINFO1 at every bound: the word");
	check(dw[PPP_SIZEINFO2] == (511u << 23), "SIZEINFO2 at its bound: the word");
	check((dw[PPP_OUTPUT_SEL] >> 24) == 255, "OUTPUT_SEL.vtxsize at its bound: 255");
	check(dw[PPP_VARYING0] == 0x00ffffffu, "VARYING0 at every bound: the word");
	check(dw[PPP_WORDS] == POISON, "PPP block: writes 19 dwords, no more");
	check(why.field == NULL, "PPP block accepted: no refusal named");

	/* 2: one past each bound */
	{
		struct pvr_ppp_state st = ok;

		st.vtxsize = 256;
		ppp_refused("vtxsize 256", &ppp_ok_in, &st, "TA_OUTPUT_SEL.vtxsize", 256, 255, 4);
	}
	{
		struct pvr_ppp_state st = ok;

		st.f32_linear = 256;
		ppp_refused("f32_linear 256", &ppp_ok_in, &st, "TA_STATE_VARYING0.f32_linear", 256, 255, 4);
	}
	{
		struct pvr_ppp_state st = ok;

		st.f32_flat = 256;
		ppp_refused("f32_flat 256", &ppp_ok_in, &st, "TA_STATE_VARYING0.f32_flat", 256, 255, 4);
	}
	{
		struct pvr_ppp_state st = ok;

		st.f32_npc = 256;
		ppp_refused("f32_npc 256", &ppp_ok_in, &st, "TA_STATE_VARYING0.f32_npc", 256, 255, 4);
	}
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.texture_program_data_size_bytes = 127 * 16 + 1;
		ppp_refused("texture data 2033 B", &in, &ok, "TA_STATE_PDS_SIZEINFO1.pds_texturestatesize", 128, 127, 16);
	}
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.texture_program_temps = 61;
		ppp_refused("texture temps 61", &in, &ok, "TA_STATE_PDS_SIZEINFO1.pds_tempsize", 16, 15, 16);
	}
	{
		struct pvr_ppp_state st = ok;

		st.usc_varyingsize = 64;
		ppp_refused("usc_varyingsize 64", &ppp_ok_in, &st, "TA_STATE_PDS_SIZEINFO1.usc_varyingsize", 64, 63, 64);
	}
	{
		struct pvr_ppp_state st = ok;

		st.pds_varyingsize = 64;
		ppp_refused("pds_varyingsize 64", &ppp_ok_in, &st, "TA_STATE_PDS_SIZEINFO1.pds_varyingsize", 64, 63, 16);
	}
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.shared_reg_count = 511 * 16 + 1;
		ppp_refused("shared 8177 registers", &in, &ok, "TA_STATE_PDS_SIZEINFO2.usc_sharedsize", 512, 511, 64);
	}
	/* 3: near 2^32 (the old 32-bit round-up wrapped these to 0 units) */
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.texture_program_data_size_bytes = 0xffffffffu;
		ppp_refused("texture data 0xffffffff B", &in, &ok, "TA_STATE_PDS_SIZEINFO1.pds_texturestatesize",
			    268435456, 127, 16);
	}
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.shared_reg_count = 0xffffffffu;
		ppp_refused("shared 0xffffffff registers", &in, &ok, "TA_STATE_PDS_SIZEINFO2.usc_sharedsize", 268435456,
			    511, 64);
	}
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.texture_program_temps = 0xffffffffu;
		poison(dw, PPP_N);
		rc = pvr_ppp_build_block(&in, &ok, dw, NULL, NULL);
		check(rc == PVR_VDM_FIELD_REFUSED && untouched(dw, PPP_N), "PPP block refuses with why == NULL");
	}

	/* 4: every in-range value writes the word the old masked writer wrote */
	{
		bool vtx = true, var = true, tex = true, tmp = true, usc = true, pds = true, sh = true;
		struct pvr_ppp_frag_inputs in = ppp_ok_in;
		struct pvr_ppp_state st = ok;

		for (uint32_t v = 0; v <= 255; v++) {
			st = ok;
			st.vtxsize = v;
			st.f32_linear = v;
			st.f32_flat = 255 - v;
			st.f32_npc = v ^ 0x5au;
			vtx &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			       (dw[PPP_OUTPUT_SEL] >> 24) == (v & 0xffu);
			var &= dw[PPP_VARYING0] == ((v & 0xffu) | (((255 - v) & 0xffu) << 8) | (((v ^ 0x5au) & 0xffu) << 16));
		}
		st = ok;
		for (uint32_t b = 0; b <= 127 * 16; b++) {
			in = ppp_ok_in;
			in.texture_program_data_size_bytes = b;
			tex &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			       dw[PPP_SIZEINFO1] == old_sizeinfo1(mesa_texstate_units(b), in.texture_program_temps, 63, 63);
		}
		for (uint32_t t = 0; t <= 60; t++) {
			in = ppp_ok_in;
			in.texture_program_temps = t;
			tmp &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			       dw[PPP_SIZEINFO1] == old_sizeinfo1(127, t, 63, 63);
		}
		in = ppp_ok_in;
		for (uint32_t v = 0; v <= 63; v++) {
			st = ok;
			st.usc_varyingsize = v;
			usc &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			       dw[PPP_SIZEINFO1] == old_sizeinfo1(127, 60, v, 63);
			st = ok;
			st.pds_varyingsize = v;
			pds &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			       dw[PPP_SIZEINFO1] == old_sizeinfo1(127, 60, 63, v);
		}
		st = ok;
		for (uint32_t r = 0; r <= 511 * 16; r++) {
			in = ppp_ok_in;
			in.shared_reg_count = r;
			sh &= pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
			      dw[PPP_SIZEINFO2] == ((r + 15u) / 16u) << 23;
		}
		check(vtx, "OUTPUT_SEL: every vtxsize 0..255 as the old writer");
		check(var, "VARYING0: every f32 count 0..255 as the old writer");
		check(tex, "SIZEINFO1: every texture data size 0..2032 B by Mesa's dwords/4 rule");
		check(tmp, "SIZEINFO1: every texture temp count 0..60 as the old writer");
		check(usc, "SIZEINFO1: every usc_varyingsize 0..63 as the old writer");
		check(pds, "SIZEINFO1: every pds_varyingsize 0..63 as the old writer");
		check(sh, "SIZEINFO2: every shared register count 0..8176 as the old writer");
	}

	/* 4b: pds_texturestatesize is a quarter of the old bytes/4 value. The
	 * draw's common-store DMA program (pvr_pds_render_dma_program, the one
	 * src/pvr/backend.rs binds) gives the data size: 3 dwords per kick, so
	 * 1 kick = 12 B -> 1 unit (old 3), 8 kicks = 96 B -> 6 units (old 24). */
	{
		static const struct { uint32_t kicks, units; } q[] = { { 1, 1 }, { PVR_PDS_DMA_MAX_KICKS, 6 } };
		struct pvr_pds_dma_kick kicks[PVR_PDS_DMA_MAX_KICKS];
		uint32_t pdata[3 * PVR_PDS_DMA_MAX_KICKS], pcode[PVR_PDS_DMA_MAX_KICKS + 1];
		struct pvr_ppp_frag_inputs in = ppp_ok_in;
		const struct pvr_ppp_state st = ok;
		char m[120];

		for (unsigned k = 0; k < PVR_PDS_DMA_MAX_KICKS; k++) {
			kicks[k].address = 0x1000u * (k + 1u);
			kicks[k].dwords = 4;
			kicks[k].dest_offset = 4u * k;
		}
		for (unsigned i = 0; i < sizeof(q) / sizeof(q[0]); i++) {
			struct pvr_pds_render_color_upload_info info;

			memset(&info, 0, sizeof(info));
			rc = pvr_pds_render_dma_program(kicks, q[i].kicks, 0, pdata, pcode, &info);
			in.texture_program_data_size_bytes = info.data_size_dw * 4u; /* as backend.rs */
			snprintf(m, sizeof(m), "SIZEINFO1.pds_texturestatesize for %u kicks (%u B): %u units (old %u)",
				 q[i].kicks, in.texture_program_data_size_bytes, q[i].units,
				 in.texture_program_data_size_bytes / 4u);
			check(rc == 0 && pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_WORDS &&
				      ((dw[PPP_SIZEINFO1] >> 16) & 0x7fu) == q[i].units,
			      m);
		}
	}

	/* 5: the clear block passes a refusal on, and builds in range */
	{
		struct pvr_ppp_frag_inputs in = ppp_ok_in;

		in.shared_reg_count = 512 * 16;
		n = 99;
		memset(&why, 0, sizeof(why));
		rc = pvr_ppp_build_clear_block(&in, dw, &n, &why);
		check(rc == PVR_VDM_FIELD_REFUSED && n == 0 && why.field &&
			      strcmp(why.field, "TA_STATE_PDS_SIZEINFO2.usc_sharedsize") == 0,
		      "clear PPP block refused: names SIZEINFO2.usc_sharedsize, 0 dwords");
		in.shared_reg_count = 6;
		rc = pvr_ppp_build_clear_block(&in, dw, &n, &why);
		check(rc == PPP_WORDS && n == PPP_WORDS, "clear PPP block in range: 19 dwords");
	}
}

/* 6: the PPP block's non-size fields. The block with every optional word
 * present (two-sided, depth bias, scissor, region clip; no viewport). */
enum {
	PPP_FULL_ISPCTL = 1,
	PPP_FULL_ISPA = 2,
	PPP_FULL_ISPB = 3,
	PPP_FULL_ISPA_BACK = 4,
	PPP_FULL_ISPB_BACK = 5,
	PPP_FULL_ISPDBSC = 6,
	PPP_FULL_SHADERBASE = 7,
	PPP_FULL_TEXUNICODEBASE = 8,
	PPP_FULL_VARYINGBASE = 11,
	PPP_FULL_TEXTUREDATABASE = 12,
	PPP_FULL_CLIP0 = 13,
	PPP_FULL_CLIP1 = 14,
	PPP_FULL_PPP_CTRL = 20,
	PPP_FULL_WORDS = 22,
};

enum { IN_NONE, IN_UPASS, IN_PASSTYPE };

struct ppp_nonsize {
	const char *field;
	int word;
	int word2; /* a second word the field lands in (the back ISPA copies the front depth state), or 0 */
	unsigned shift, bits;
	int in_field;  /* IN_NONE: a uint32_t of pvr_ppp_state at st_off */
	size_t st_off;
};

#define ST(f) IN_NONE, offsetof(struct pvr_ppp_state, f)

static const struct ppp_nonsize ppp_nonsize[] = {
	{ "TA_STATE_ISPCTL.upass", PPP_FULL_ISPCTL, 0, 22, 4, IN_UPASS, 0 },
	{ "TA_STATE_ISPA.objtype", PPP_FULL_ISPA, PPP_FULL_ISPA_BACK, 28, 4, ST(objtype) },
	{ "TA_STATE_ISPA.passtype", PPP_FULL_ISPA, PPP_FULL_ISPA_BACK, 24, 3, IN_PASSTYPE, 0 },
	{ "TA_STATE_ISPA.dcmpmode", PPP_FULL_ISPA, PPP_FULL_ISPA_BACK, 17, 3, ST(depth_cmp) },
	{ "TA_STATE_ISPA.pointlinewidth", PPP_FULL_ISPA, PPP_FULL_ISPA_BACK, 8, 8, ST(point_line_width) },
	{ "TA_STATE_ISPA.sref", PPP_FULL_ISPA, 0, 0, 8, ST(stencil_ref) },
	{ "TA_STATE_ISPB.scmpmode", PPP_FULL_ISPB, 0, 25, 3, ST(stencil_cmp) },
	{ "TA_STATE_ISPB.sop1", PPP_FULL_ISPB, 0, 22, 3, ST(stencil_sop_fail) },
	{ "TA_STATE_ISPB.sop2", PPP_FULL_ISPB, 0, 19, 3, ST(stencil_sop_dfail) },
	{ "TA_STATE_ISPB.sop3", PPP_FULL_ISPB, 0, 16, 3, ST(stencil_sop_pass) },
	{ "TA_STATE_ISPB.scmpmask", PPP_FULL_ISPB, 0, 8, 8, ST(stencil_cmp_mask) },
	{ "TA_STATE_ISPB.swmask", PPP_FULL_ISPB, 0, 0, 8, ST(stencil_write_mask) },
	{ "TA_STATE_ISPA(back).sref", PPP_FULL_ISPA_BACK, 0, 0, 8, ST(stencil_back_ref) },
	{ "TA_STATE_ISPB(back).scmpmode", PPP_FULL_ISPB_BACK, 0, 25, 3, ST(stencil_back_cmp) },
	{ "TA_STATE_ISPB(back).sop1", PPP_FULL_ISPB_BACK, 0, 22, 3, ST(stencil_back_sop_fail) },
	{ "TA_STATE_ISPB(back).sop2", PPP_FULL_ISPB_BACK, 0, 19, 3, ST(stencil_back_sop_dfail) },
	{ "TA_STATE_ISPB(back).sop3", PPP_FULL_ISPB_BACK, 0, 16, 3, ST(stencil_back_sop_pass) },
	{ "TA_STATE_ISPB(back).scmpmask", PPP_FULL_ISPB_BACK, 0, 8, 8, ST(stencil_back_cmp_mask) },
	{ "TA_STATE_ISPB(back).swmask", PPP_FULL_ISPB_BACK, 0, 0, 8, ST(stencil_back_write_mask) },
	{ "TA_STATE_ISPDBSC.dbindex", PPP_FULL_ISPDBSC, 0, 16, 16, ST(dbias_index) },
	{ "TA_STATE_ISPDBSC.scindex", PPP_FULL_ISPDBSC, 0, 0, 16, ST(scissor_index) },
	{ "TA_REGION_CLIP0.left", PPP_FULL_CLIP0, 0, 16, 9, ST(clip_left) },
	{ "TA_REGION_CLIP0.right", PPP_FULL_CLIP0, 0, 0, 9, ST(clip_right) },
	{ "TA_REGION_CLIP1.top", PPP_FULL_CLIP1, 0, 16, 9, ST(clip_top) },
	{ "TA_REGION_CLIP1.bottom", PPP_FULL_CLIP1, 0, 0, 9, ST(clip_bottom) },
	{ "TA_STATE_PPP_CTRL.clip_mode", PPP_FULL_PPP_CTRL, 0, 10, 2, ST(clip_mode) },
	{ "TA_STATE_PPP_CTRL.flatshade_vtx", PPP_FULL_PPP_CTRL, 0, 7, 2, ST(flatshade_vtx) },
	{ "TA_STATE_PPP_CTRL.cullmode", PPP_FULL_PPP_CTRL, 0, 0, 2, ST(cullmode) },
};

#undef ST

static void ppp_set(const struct ppp_nonsize *f, struct pvr_ppp_frag_inputs *in, struct pvr_ppp_state *st,
		    uint32_t v)
{
	switch (f->in_field) {
	case IN_UPASS:
		in->isp_userpass = v;
		break;
	case IN_PASSTYPE:
		in->passtype = (uint8_t)v; /* one past (8) still fits the uint8_t */
		break;
	default:
		memcpy((char *)st + f->st_off, &v, sizeof(v));
		break;
	}
}

static struct pvr_ppp_state ppp_full_state(void)
{
	struct pvr_ppp_state st = ppp_ok_state();

	st.two_sided = 1;
	st.dbias_enable = 1;
	st.scissor_enable = 1;
	st.clip_enable = 1;
	return st;
}

static struct pvr_ppp_frag_inputs ppp_full_in(void)
{
	struct pvr_ppp_frag_inputs in = ppp_ok_in;

	in.varying_program_code_offset = 0x2080; /* VARYINGBASE written from the offset */
	return in;
}

/* Whether the block built with the field at `v` is the base block (the field
 * at 0) with v << shift in the field's word: nothing else moved. */
static bool ppp_lands(const struct ppp_nonsize *f, uint32_t v, const uint32_t *base)
{
	struct pvr_ppp_frag_inputs in = ppp_full_in();
	struct pvr_ppp_state st = ppp_full_state();
	uint32_t dw[PPP_N], n = 0;

	ppp_set(f, &in, &st, v);
	if (pvr_ppp_build_block(&in, &st, dw, &n, NULL) != PPP_FULL_WORDS || n != PPP_FULL_WORDS)
		return false;
	for (int i = 0; i < PPP_FULL_WORDS; i++) {
		const bool in_word = i == f->word || (f->word2 && i == f->word2); /* no field is in word 0 */
		const uint32_t want = in_word ? base[i] | (v << f->shift) : base[i];

		if (dw[i] != want)
			return false;
	}
	return true;
}

static void ppp_nonsize_fields(void)
{
	struct pvr_ppp_frag_inputs in = ppp_full_in();
	struct pvr_ppp_state st = ppp_full_state();
	uint32_t base[PPP_N], dw[PPP_N], n = 0;
	char m[160];

	poison(base, PPP_N);
	check(pvr_ppp_build_block(&in, &st, base, &n, NULL) == PPP_FULL_WORDS && n == PPP_FULL_WORDS &&
		      base[PPP_FULL_WORDS] == POISON,
	      "PPP block with every optional word: 22 dwords");
	for (unsigned i = 0; i < sizeof(ppp_nonsize) / sizeof(ppp_nonsize[0]); i++) {
		const struct ppp_nonsize *f = &ppp_nonsize[i];
		const uint32_t max = (uint32_t)((1ull << f->bits) - 1u);
		bool sweep = true;

		snprintf(m, sizeof(m), "%s at its bound %u: accepted, in its own bits", f->field, max);
		check(ppp_lands(f, max, base), m);
		in = ppp_full_in();
		st = ppp_full_state();
		ppp_set(f, &in, &st, max + 1u);
		snprintf(m, sizeof(m), "%s %u", f->field, max + 1u);
		ppp_refused(m, &in, &st, f->field, max + 1u, max, 0);
		for (uint32_t v = 0; v <= max && sweep; v++)
			sweep = ppp_lands(f, v, base);
		snprintf(m, sizeof(m), "%s: every value 0..%u as the old masked writer", f->field, max);
		check(sweep, m);
	}

	/* the PDS base addresses: a 16-byte aligned offset is written as is, a
	 * misaligned one is refused (the old writer dropped its low bits) */
	{
		static const struct {
			const char *field;
			int word;
			size_t off;
		} a[] = {
			{ "TA_STATE_PDS_SHADERBASE.addr bits 3:0", PPP_FULL_SHADERBASE,
			  offsetof(struct pvr_ppp_frag_inputs, pixel_program_code_offset) },
			{ "TA_STATE_PDS_TEXUNICODEBASE.addr bits 3:0", PPP_FULL_TEXUNICODEBASE,
			  offsetof(struct pvr_ppp_frag_inputs, texture_program_code_offset) },
			{ "TA_STATE_PDS_VARYINGBASE.addr bits 3:0", PPP_FULL_VARYINGBASE,
			  offsetof(struct pvr_ppp_frag_inputs, varying_program_code_offset) },
			{ "TA_STATE_PDS_TEXTUREDATABASE.addr bits 3:0", PPP_FULL_TEXTUREDATABASE,
			  offsetof(struct pvr_ppp_frag_inputs, texture_program_data_offset) },
		};

		for (unsigned i = 0; i < sizeof(a) / sizeof(a[0]); i++) {
			uint32_t v = 0xfffffff0u;

			in = ppp_full_in();
			st = ppp_full_state();
			memcpy((char *)&in + a[i].off, &v, sizeof(v));
			snprintf(m, sizeof(m), "%s: offset 0xfffffff0 accepted, written as is", a[i].field);
			check(pvr_ppp_build_block(&in, &st, dw, &n, NULL) == PPP_FULL_WORDS && dw[a[i].word] == v, m);
			v = 0xfffffff8u;
			memcpy((char *)&in + a[i].off, &v, sizeof(v));
			snprintf(m, sizeof(m), "%s: offset 0xfffffff8", a[i].field);
			ppp_refused(m, &in, &st, a[i].field, 8, 0, 0);
		}
	}

	/* a field that is not emitted is not checked (and not written) */
	st = ppp_ok_state();
	st.stencil_back_ref = 0x1ff;
	st.dbias_index = 0x10000;
	st.scissor_index = 0x10000;
	st.clip_left = 0x200;
	check(pvr_ppp_build_block(&ppp_ok_in, &st, dw, &n, NULL) == PPP_WORDS,
	      "one-sided, no ISPDBSC, no clip: the unused fields are not checked");
}

/* 7: the VDM words' non-size fields. */
static void vdm_nonsize_fields(void)
{
	struct pvr_vdm_field_refusal why;
	uint32_t dw[8];
	int32_t rc;
	bool sweep;

	/* PPP_STATE0/1 */
	poison(dw, 8);
	memset(&why, 0, sizeof(why));
	rc = pvr_vdm_emit_ppp_state(dw, 0xfffffffffcull, 255, &why);
	check(rc == 2 && dw[0] == ((255u << 8) | 0xffu) && dw[1] == 0xfffffffcu && dw[2] == POISON && !why.field,
	      "PPP_STATE: word_count 255, address 2^40 - 4 accepted");
	poison(dw, 8);
	rc = pvr_vdm_emit_ppp_state(dw, 0x1000, 256, &why);
	refused("PPP_STATE word_count 256", rc, dw, 8, &why, "VDMCTRL_PPP_STATE0.word_count", 256, 255, 0);
	poison(dw, 8);
	rc = pvr_vdm_emit_ppp_state(dw, 0x10000000000ull, 19, &why);
	refused("PPP_STATE address 2^40", rc, dw, 8, &why, "VDMCTRL_PPP_STATE0.addrmsb (address bits 39:32)", 256,
		255, 0);
	poison(dw, 8);
	rc = pvr_vdm_emit_ppp_state(dw, 0x40108002ull, 19, &why);
	refused("PPP_STATE address 0x40108002", rc, dw, 8, &why, "VDMCTRL_PPP_STATE1.addrlsb (address bits 1:0)", 2, 0,
		0);
	sweep = true;
	for (uint32_t v = 0; v <= 255; v++)
		sweep &= pvr_vdm_emit_ppp_state(dw, 0x40108000ull, v, NULL) == 2 && dw[0] == (v << 8) &&
			 dw[1] == 0x40108000u;
	check(sweep, "PPP_STATE: every word_count 0..255 as the old masked writer");

	/* PDS_STATE1/2 */
	{
		struct pvr_vdm_pds_state s = { .pds_data_offset = 0xfffffff0u, .pds_code_offset = 0xfffffff0u };

		poison(dw, 8);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		check(rc == 3 && dw[1] == 0xfffffff0u && dw[2] == 0xfffffff0u, "PDS_STATE1/2: 16-byte aligned offsets accepted");
		s.pds_data_offset = 0x108;
		poison(dw, 8);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("PDS_STATE1 offset 0x108", rc, dw, 8, &why, "PDS_STATE1.pds_data_addr bits 3:0", 8, 0, 0);
		s.pds_data_offset = 0x100;
		s.pds_code_offset = 0x204;
		poison(dw, 8);
		rc = pvr_vdm_emit_pds_state(dw, &s, &why);
		refused("PDS_STATE2 offset 0x204", rc, dw, 8, &why, "PDS_STATE2.pds_code_addr bits 3:0", 4, 0, 0);
	}

	/* VDM_STATE0.flatshade_control, VDM_STATE2/3 */
	{
		const struct pvr_vdm_vs_state ok = {
			.vs_pds_data_offset = 0xfffffff0u,
			.vs_pds_code_offset = 0xfffffff0u,
			.vs_pds_data_size_bytes = 32,
			.vs_pds_temps = 2,
			.vs_output_size_bytes = 16,
			.flatshade_control = 3,
		};
		struct pvr_vdm_vs_state s = ok;

		poison(dw, 8);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		check(rc == 5 && ((dw[0] >> 1) & 3u) == 3 && dw[1] == 0xfffffff0u && dw[2] == 0xfffffff0u,
		      "VDM_STATE0.flatshade_control 3 and 16-byte aligned VDM_STATE2/3 accepted");
		s.flatshade_control = 4;
		poison(dw, 8);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("flatshade_control 4", rc, dw, 8, &why, "VDM_STATE0.flatshade_control", 4, 3, 0);
		s = ok;
		s.vs_pds_data_offset = 0x301;
		poison(dw, 8);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("VDM_STATE2 offset 0x301", rc, dw, 8, &why, "VDM_STATE2.vs_pds_data_base_addr bits 3:0", 1, 0, 0);
		s = ok;
		s.vs_pds_code_offset = 0x40c;
		poison(dw, 8);
		rc = pvr_vdm_emit_vdm_state(dw, &s, &why);
		refused("VDM_STATE3 offset 0x40c", rc, dw, 8, &why, "VDM_STATE3.vs_pds_code_base_addr bits 3:0", 12, 0, 0);
		sweep = true;
		for (uint32_t v = 0; v <= 3; v++) {
			s = ok;
			s.flatshade_control = v;
			sweep &= pvr_vdm_emit_vdm_state(dw, &s, NULL) == 5 && ((dw[0] >> 1) & 3u) == v &&
				 (dw[0] & 1u) == 0 && ((dw[0] >> 3) & 0xfu) == 0;
		}
		check(sweep, "VDM_STATE0: every flatshade_control 0..3 in its own bits");
	}

	/* INDEX_LIST0/1 */
	poison(dw, 8);
	memset(&why, 0, sizeof(why));
	rc = pvr_vdm_emit_index_list(dw, 15, 0xfffffffffcull, 3, &why);
	check(rc == 3 && ((dw[0] >> 8) & 0xfu) == 15 && (dw[0] & 0xffu) == 0xff && dw[1] == 0xfffffffcu && dw[2] == 3 &&
		      dw[3] == POISON && !why.field,
	      "INDEX_LIST: topology 15, address 2^40 - 4 accepted");
	poison(dw, 8);
	rc = pvr_vdm_emit_index_list(dw, 16, 0, 3, &why);
	refused("INDEX_LIST topology 16 (non-indexed)", rc, dw, 8, &why, "VDMCTRL_INDEX_LIST0.primitive_topology", 16,
		15, 0);
	poison(dw, 8);
	rc = pvr_vdm_emit_index_list(dw, 6, 0x10000000000ull, 3, &why);
	refused("INDEX_LIST address 2^40", rc, dw, 8, &why,
		"VDMCTRL_INDEX_LIST0.index_base_addrmsb (address bits 39:32)", 256, 255, 0);
	poison(dw, 8);
	rc = pvr_vdm_emit_index_list(dw, 6, 0x50000002ull, 3, &why);
	refused("INDEX_LIST address 0x50000002", rc, dw, 8, &why,
		"VDMCTRL_INDEX_LIST1.index_base_addrlsb (address bits 1:0)", 2, 0, 0);
	poison(dw, 8);
	rc = pvr_vdm_emit_index_list_instanced(dw, 16, 0x50000000ull, 3, 4, &why);
	refused("INDEX_LIST instanced topology 16", rc, dw, 8, &why, "VDMCTRL_INDEX_LIST0.primitive_topology", 16, 15, 0);
	sweep = true;
	for (uint32_t v = 0; v <= 15; v++) {
		sweep &= pvr_vdm_emit_index_list(dw, v, 0x50000000ull, 3, NULL) == 3 &&
			 dw[0] == ((3u << 29) | (1u << 28) | (1u << 27) | (1u << 17) | (v << 8)) && dw[1] == 0x50000000u;
		sweep &= pvr_vdm_emit_index_list(dw, v, 0, 3, NULL) == 2 && dw[0] == ((3u << 29) | (1u << 27) | (v << 8));
	}
	check(sweep, "INDEX_LIST: every topology 0..15 as the old masked writer");

	/* STATE_TERMINATE0: 32-px blocks, 9 bits */
	{
		uint32_t t[4];

		poison(t, 4);
		memset(&why, 0, sizeof(why));
		rc = pvr_ppp_build_terminate_block(16384, 16384, t, &why);
		check(rc == 3 && t[1] == ((511u << 18) | 511u) && t[3] == POISON && !why.field,
		      "TERMINATE: 16384 x 16384 accepted (511, 511)");
		rc = pvr_ppp_build_terminate_block(1024, 600, t, &why);
		check(rc == 3 && t[0] == 0x02000000u && t[1] == 0x007c0012u && t[2] == 0,
		      "TERMINATE: 1024 x 600 is the DDK capture 0x02000000 0x007c0012 0");
		poison(t, 4);
		rc = pvr_ppp_build_terminate_block(16385, 600, t, &why);
		refused("TERMINATE width 16385", rc, t, 4, &why, "TA_STATE_TERMINATE0.clip_right", 512, 511, 0);
		poison(t, 4);
		rc = pvr_ppp_build_terminate_block(1024, 16385, t, &why);
		refused("TERMINATE height 16385", rc, t, 4, &why, "TA_STATE_TERMINATE0.clip_bottom", 512, 511, 0);
		poison(t, 4);
		rc = pvr_ppp_build_terminate_block(0, 600, t, &why);
		refused("TERMINATE width 0", rc, t, 4, &why, "TA_STATE_TERMINATE0.clip_right", UINT64_MAX, 511, 0);
		sweep = true;
		for (uint32_t w = 1; w <= 16384; w++) {
			const uint32_t b = (w + 31u) / 32u - 1u;

			sweep &= pvr_ppp_build_terminate_block(w, w, t, NULL) == 3 && t[1] == ((b << 18) | b);
		}
		check(sweep, "TERMINATE: every size 1..16384 as the old masked writer");
	}
}

/* 8: the IPF scissor table entry (pvr_isp_build_scissor_word): two dwords,
 * four 16-bit fields (csbgen rogue/ipf.xml:150-166). A memory table the IPF
 * reads, not a PPP/VDM stream word, but the same checked-emitter contract:
 * every bound is accepted, one past each is refused by name with nothing
 * written. */
static void scissor_word(void)
{
	struct pvr_vdm_field_refusal why;
	uint32_t w[4];
	int32_t rc;
	bool sweep;

	/* 1: every field at its bound */
	poison(w, 4);
	memset(&why, 0, sizeof(why));
	rc = pvr_isp_build_scissor_word(0xffffu, 0xffffu, 0xffffu, 0xffffu, w, &why);
	check(rc == 2, "SCISSOR_WORD at every bound: 2 dwords");
	check(w[0] == 0xffffffffu && w[1] == 0xffffffffu, "SCISSOR_WORD at every bound: the words");
	check(w[2] == POISON, "SCISSOR_WORD: writes 2 dwords, no more");
	check(why.field == NULL, "SCISSOR_WORD accepted: no refusal named");

	/* 2: one past each bound, in field-check order (xmin, xmax, ymin, ymax) */
	poison(w, 4);
	rc = pvr_isp_build_scissor_word(0x10000u, 0, 0, 0, w, &why);
	refused("SCISSOR_WORD xmin 0x10000", rc, w, 4, &why, "IPF_SCISSOR_WORD_0.scw0_xmin", 0x10000, 0xffff, 0);
	poison(w, 4);
	rc = pvr_isp_build_scissor_word(0, 0x10000u, 0, 0, w, &why);
	refused("SCISSOR_WORD xmax 0x10000", rc, w, 4, &why, "IPF_SCISSOR_WORD_0.scw0_xmax", 0x10000, 0xffff, 0);
	poison(w, 4);
	rc = pvr_isp_build_scissor_word(0, 0, 0x10000u, 0, w, &why);
	refused("SCISSOR_WORD ymin 0x10000", rc, w, 4, &why, "IPF_SCISSOR_WORD_1.scw1_ymin", 0x10000, 0xffff, 0);
	poison(w, 4);
	rc = pvr_isp_build_scissor_word(0, 0, 0, 0x10000u, w, &why);
	refused("SCISSOR_WORD ymax 0x10000", rc, w, 4, &why, "IPF_SCISSOR_WORD_1.scw1_ymax", 0x10000, 0xffff, 0);

	/* 3: every value 0..0xffff, on the diagonal (xmin=xmax=ymin=ymax=v), as
	 * the old masked writer (`(v << 16) | (v & 0xffff)`) gave. */
	sweep = true;
	for (uint32_t v = 0; v <= 0xffffu; v++) {
		const uint32_t want = (v << 16) | v;

		sweep &= pvr_isp_build_scissor_word(v, v, v, v, w, NULL) == 2 && w[0] == want && w[1] == want;
	}
	check(sweep, "SCISSOR_WORD: every value 0..0xffff as the old masked writer");
}

/* The clear stream passes a refusal on: 0 bytes, *out_count 0, why named. */
static void clear_stream(void)
{
	struct pvr_vdm_inputs in = {
		.ppp_state_dev_addr = 0x40108000u,
		.ppp_word_count = 19,
		.vs_pds_data_offset = 0x1000,
		.vs_pds_code_offset = 0x1080,
		.vs_pds_data_size_bytes = 64 * 16, /* 64 units: one past */
		.vs_pds_temps = 4,
		.vs_output_size_bytes = 16,
		.index_count = 4,
	};
	struct pvr_vdm_field_refusal why = { 0 };
	uint32_t out[PVR_VDM_STREAM_MAX_DWORDS], n = 99;

	check(pvr_vdm_build_clear_stream(&in, out, &n, &why) == 0 && n == 0, "clear stream refused: 0 bytes, 0 dwords");
	check(why.field && strcmp(why.field, "VDM_STATE5.vs_pds_data_size") == 0, "clear stream refusal names the field");
	in.vs_pds_data_size_bytes = 64;
	check(pvr_vdm_build_clear_stream(&in, out, &n, &why) == n * 4u && n > 0, "clear stream in range: built");
	/* a non-size refusal from the index list is passed on the same way */
	in.index_buf_dev_addr = 0x50000002u;
	n = 99;
	memset(&why, 0, sizeof(why));
	check(pvr_vdm_build_clear_stream(&in, out, &n, &why) == 0 && n == 0 && why.field &&
		      strcmp(why.field, "VDMCTRL_INDEX_LIST1.index_base_addrlsb (address bits 1:0)") == 0,
	      "clear stream: a misaligned index address refused, named");
}

int main(void)
{
	pds_state();
	vdm_state();
	vdm_temp_units();
	kick_usc_temps();
	ppp_block();
	ppp_nonsize_fields();
	vdm_nonsize_fields();
	scissor_word();
	clear_stream();
	printf("vdm_field_check: %d checks run (planned %d), %d failed\n", checks, PLANNED, failures);
	return failures == 0 && checks == PLANNED ? 0 : 1;
}
