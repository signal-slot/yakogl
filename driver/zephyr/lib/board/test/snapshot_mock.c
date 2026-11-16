/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Host mock of rpi5_scanout/snapshot.h over a fake register file.
 * Built by tools/v3d/check71.d/snapshot.sh. */
#include <stdio.h>
#include <string.h>

#include "snapshot.h"

static int checks, failed;
#define CHECK(c)                                                                  \
	do {                                                                      \
		checks++;                                                         \
		if (!(c)) {                                                       \
			failed++;                                                 \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);       \
		}                                                                 \
	} while (0)

static uint32_t regs[3][0x8000 / 4];
static unsigned reads;

static uint32_t rd(void *ctx, enum snap_block b, uint32_t off)
{
	(void)ctx;
	reads++;
	return regs[b][off / 4];
}

static char lines[200][160];
static size_t nlines;
static void sink(void *ctx, const char *t)
{
	(void)ctx;
	if (nlines < 200)
		snprintf(lines[nlines++], sizeof(lines[0]), "%s", t);
}

int main(void)
{
	const struct snap_io io = { rd, NULL };
	const struct snap_emit em = { sink, NULL };
	uint32_t out[SNAP_COUNT];

	/* table: unique names, citations, 32-bit, aligned, valid blocks */
	for (size_t i = 0; i < SNAP_COUNT; i++) {
		CHECK(snap_table[i].cite && snap_table[i].cite[0]);
		CHECK(snap_table[i].name && snap_table[i].name[0]);
		CHECK(snap_table[i].width == 32 && (snap_table[i].off & 3u) == 0);
		CHECK(snap_table[i].block <= SNAP_HDMI0);
		for (size_t j = i + 1; j < SNAP_COUNT; j++) {
			CHECK(strcmp(snap_table[i].name, snap_table[j].name) != 0);
			CHECK(!(snap_table[i].block == snap_table[j].block &&
				snap_table[i].off == snap_table[j].off));
		}
	}

	/* take reads each entry once, in table order */
	for (size_t i = 0; i < SNAP_COUNT; i++)
		regs[snap_table[i].block][snap_table[i].off / 4] = 0x1000u + (uint32_t)i;
	CHECK(snapshot_take(&io, out) == SNAP_COUNT);
	CHECK(reads == SNAP_COUNT);
	for (size_t i = 0; i < SNAP_COUNT; i++)
		CHECK(out[i] == 0x1000u + i);
	CHECK(snapshot_take(NULL, out) == 0);

	/* exact line format */
	regs[SNAP_HVS][0x44 / 4] = 0x00400000u;
	regs[SNAP_HVS][0x20 / 4] = 0x80040000u; /* control: HVS_EN (bit 31) set */
	regs[SNAP_HVS][0x34 / 4] = 0x00000200u; /* c_disp0_ctrl1 */
	regs[SNAP_HVS][0x114 / 4] = 0x00000100u; /* d_disp0_cob */
	regs[SNAP_PV][0] = 0xdeadbeefu;
	regs[SNAP_HDMI0][0x44 / 4] = 0x80000001u;
	regs[SNAP_HDMI0][0x48 / 4] = 0x80000000u;
	snapshot_take(&io, out);
	CHECK(snapshot_print(out, &em) == SNAP_COUNT && nlines == SNAP_COUNT);
	int sh = 0, sp = 0, sd = 0, se = 0, sc = 0, s1 = 0, s2 = 0;
	for (size_t i = 0; i < nlines; i++) {
		CHECK(strncmp(lines[i], "u3 reg ", 7) == 0);
		sh |= !strcmp(lines[i], "u3 reg hvs c_disp0_status +0x0044=0x00400000");
		sc |= !strcmp(lines[i], "u3 reg hvs control +0x0020=0x80040000");
		s1 |= !strcmp(lines[i], "u3 reg hvs c_disp0_ctrl1 +0x0034=0x00000200");
		s2 |= !strcmp(lines[i], "u3 reg hvs d_disp0_cob +0x0114=0x00000100");
		sp |= !strcmp(lines[i], "u3 reg pv pv_control +0x0000=0xdeadbeef");
		sd |= !strcmp(lines[i], "u3 reg hdmi0 hdmi_vid_ctl +0x0044=0x80000001");
		se |= !strcmp(lines[i], "u3 reg hdmi0 hdmi1_vid_ctl +0x0048=0x80000000");
	}
	CHECK(sh && sp && sd && se && sc && s1 && s2);

	/* The HVS's identity words and the channel registers beyond the four the
	 * decoders need: Z1 (driver/docs/rpi5-display.md section 6) reads CTRL1,
	 * BGND and COB as well, and the table names them for both steppings. */
	{
		static const char *const sfx[7] = { "ctrl0", "ctrl1", "bgnd", "lptrs", "cob", "status",
						    "dl" };
		int ctl = 0;

		for (size_t i = 0; i < SNAP_COUNT; i++) {
			if (snap_table[i].block == SNAP_HVS && !strcmp(snap_table[i].name, "control")) {
				ctl |= snap_table[i].off == 0x20;
			}
		}
		CHECK(ctl);
		for (int g = 0; g < 2; g++) {
			for (unsigned ch = 0; ch < 3; ch++) {
				for (size_t k = 0; k < 7; k++) {
					char nm[32];
					int found = 0;

					(void)snprintf(nm, sizeof(nm), "%s_disp%u_%s", g ? "d" : "c", ch,
						       sfx[k]);
					for (size_t i = 0; i < SNAP_COUNT; i++) {
						found |= !strcmp(snap_table[i].name, nm);
					}
					CHECK(found);
				}
			}
		}
	}

	/* the 'hd' offsets are the BCM2712 tables' (vc6_hdmi_hdmi0_fields at
	 * rpi:vc4_hdmi_regs.h:431-439, vc6_hdmi_hdmi1_fields at :531-539) and not
	 * the legacy BCM2835 table's (vc4_hdmi_fields at :182-197: M_CTL 0x0c,
	 * VID_CTL 0x38, FRAME_COUNT 0x68; CEC_CNTRL_1 is what 0xe8 is there, at
	 * :221). A reader that used the legacy values reads another register. */
	int hv0 = 0, hv1 = 0, hf0 = 0, hf1 = 0, hd0 = 0;
	for (size_t i = 0; i < SNAP_COUNT; i++) {
		if (snap_table[i].block != SNAP_HDMI0)
			continue;
		hv0 |= !strcmp(snap_table[i].name, "hdmi_vid_ctl") && snap_table[i].off == 0x44;
		hv1 |= !strcmp(snap_table[i].name, "hdmi1_vid_ctl") && snap_table[i].off == 0x48;
		hf0 |= !strcmp(snap_table[i].name, "hdmi_frame_count") && snap_table[i].off == 0x60;
		hf1 |= !strcmp(snap_table[i].name, "hdmi1_frame_count") && snap_table[i].off == 0x64;
		hd0 |= !strcmp(snap_table[i].name, "hdmi_dvp_ctl") && snap_table[i].off == 0x00;
		CHECK(snap_table[i].off != 0x38 && snap_table[i].off != 0x0c && snap_table[i].off != 0xe8);
	}
	CHECK(hv0 && hv1 && hf0 && hf1 && hd0);

	/* channel strides: c ch2 ctrl0 = 0x30 + 2*0x20; d ch1 dl = 0x11c + 0x40; and the
	 * same distance for the registers the decoders do not read (c ch2 cob =
	 * 0x40 + 0x40, d ch1 cob = 0x114 + 0x40) */
	int c2 = 0, d1 = 0, c3 = 0, d3 = 0;
	for (size_t i = 0; i < SNAP_COUNT; i++) {
		c2 |= !strcmp(snap_table[i].name, "c_disp2_ctrl0") && snap_table[i].off == 0x70;
		d1 |= !strcmp(snap_table[i].name, "d_disp1_dl") && snap_table[i].off == 0x15c;
		c3 |= !strcmp(snap_table[i].name, "c_disp2_cob") && snap_table[i].off == 0x80;
		d3 |= !strcmp(snap_table[i].name, "d_disp1_cob") && snap_table[i].off == 0x154;
	}
	CHECK(c2 && d1 && c3 && d3);

	/* display list stops at END */
	nlines = 0;
	reads = 0;
	for (unsigned i = 0; i < 128; i++)
		regs[SNAP_HVS][SNAP_HVS_DLIST_OFFSET / 4 + 40 + i] = 0x11u + i;
	regs[SNAP_HVS][SNAP_HVS_DLIST_OFFSET / 4 + 40 + 5] = SNAP_END_WORD_BIT31;
	CHECK(snapshot_dlist(&io, &em, 1, 40, SNAP_END_WORD_BIT31) == 6);
	CHECK(nlines == 6 && reads == 6);
	CHECK(!strcmp(lines[0], "u3 dlist 1 [0]=0x00000011"));
	CHECK(!strcmp(lines[5], "u3 dlist 1 [5]=0x80000000"));

	/* ... and at 64 words with no END */
	nlines = 0;
	CHECK(snapshot_dlist(&io, &em, 0, 50, SNAP_END_WORD_BIT31) == 64 && nlines == 64);
	CHECK(!strcmp(lines[63], "u3 dlist 0 [63]=0x0000005a"));

	/* END at the first word; a data word with bit 31 set is not END */
	nlines = 0;
	regs[SNAP_HVS][SNAP_HVS_DLIST_OFFSET / 4 + 100] = SNAP_END_WORD_BIT31;
	CHECK(snapshot_dlist(&io, &em, 2, 100, SNAP_END_WORD_BIT31) == 1);
	nlines = 0;
	regs[SNAP_HVS][SNAP_HVS_DLIST_OFFSET / 4 + 100] = 0xc0001234u;
	regs[SNAP_HVS][SNAP_HVS_DLIST_OFFSET / 4 + 101] = SNAP_END_WORD_BIT31;
	CHECK(snapshot_dlist(&io, &em, 2, 100, SNAP_END_WORD_BIT31) == 2);
	CHECK(!strcmp(lines[0], "u3 dlist 2 [0]=0xc0001234"));

	printf("snapshot_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
