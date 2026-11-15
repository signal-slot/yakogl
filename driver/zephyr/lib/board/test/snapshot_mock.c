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
	regs[SNAP_PV][0] = 0xdeadbeefu;
	regs[SNAP_HDMI0][0x38 / 4] = 0x80000001u;
	snapshot_take(&io, out);
	CHECK(snapshot_print(out, &em) == SNAP_COUNT && nlines == SNAP_COUNT);
	int sh = 0, sp = 0, sd = 0;
	for (size_t i = 0; i < nlines; i++) {
		CHECK(strncmp(lines[i], "u3 reg ", 7) == 0);
		sh |= !strcmp(lines[i], "u3 reg hvs c_disp0_status +0x0044=0x00400000");
		sp |= !strcmp(lines[i], "u3 reg pv pv_control +0x0000=0xdeadbeef");
		sd |= !strcmp(lines[i], "u3 reg hdmi0 hdmi_vid_ctl +0x0038=0x80000001");
	}
	CHECK(sh && sp && sd);

	/* channel strides: c ch2 ctrl0 = 0x30 + 2*0x20; d ch1 dl = 0x11c + 0x40 */
	int c2 = 0, d1 = 0;
	for (size_t i = 0; i < SNAP_COUNT; i++) {
		c2 |= !strcmp(snap_table[i].name, "c_disp2_ctrl0") && snap_table[i].off == 0x70;
		d1 |= !strcmp(snap_table[i].name, "d_disp1_dl") && snap_table[i].off == 0x15c;
	}
	CHECK(c2 && d1);

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
