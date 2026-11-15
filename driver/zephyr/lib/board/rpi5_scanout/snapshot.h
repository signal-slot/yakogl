/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Register snapshot of the Raspberry Pi 5 (BCM2712) scanout path, for the
 * U3 probe (driver/docs/rpi5-display.md): a const table of the HVS6
 * (scaler6), pixel valve and HDMI0 registers worth reading, a reader
 * injected as a function pointer (the host mock and Zephyr's sys_read32
 * both fit), and a printer with a fixed line format that the host decoders
 * (dump-decode, the linux_ref output) parse.
 *
 * Header-only C11. Facts only; no Linux code is copied. A table entry's
 * `cite` is "rpi:<file>:<line>" (the Raspberry Pi Linux tree, as cited by
 * hvs6_dlist.h), "facts:<section>" (driver/docs/rpi5-v3d71-facts.md), or
 * "unverified" (from memory, not confirmed against a tree or the board).
 * The facts file (section 10) gives the HVS, pixel valve and HDMI base
 * addresses but no register offsets, so the offsets come from vc4_regs.h
 * as already cited in hvs6_dlist.h, or are unverified.
 *
 * Offsets are relative to the block's base; the base is the reader's
 * business (the injected reader maps block + offset to an address), so no
 * board address ever appears in a printed line.
 *
 * Printed line formats (stable; one line per register or word, no trailing
 * blanks, lower-case hex):
 *
 *   u3 reg <block> <name> +0x<off>=0x<val>
 *       <block> is hvs, pv or hdmi0; <name> is the table name; <off> is
 *       4 hex digits; <val> is 8 hex digits.
 *       e.g. "u3 reg hvs c_disp0_status +0x0044=0x00400000"
 *   u3 dlist <ch> [<i>]=0x<word>
 *       <ch> is the HVS channel in decimal, <i> the word's index from the
 *       start the caller gave, in decimal; <word> is 8 hex digits. At most
 *       SNAP_DLIST_MAX (64) words; the list stops after the first word
 *       equal to the END word (printed); the caller passes that value.
 *       e.g. "u3 dlist 0 [0]=0x80000000"
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_SNAPSHOT_H
#define YAKOGL_BOARD_RPI5_SCANOUT_SNAPSHOT_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

enum snap_block { SNAP_HVS = 0, SNAP_PV = 1, SNAP_HDMI0 = 2 };

struct snap_reg {
	enum snap_block block;
	uint16_t off;		/* byte offset from the block base */
	uint8_t width;		/* bits; only 32 is read */
	const char *name;	/* unique over the whole table */
	const char *cite;	/* see the file comment; never empty */
};

#define SNAP_DLIST_MAX 64u
/* The display-list memory starts here from the HVS base (rpi:vc4_regs.h:541,
 * rpi:vc4_hvs.c:2150-2151, as cited by hvs6_dlist.h HVS6_DLIST_OFFSET). */
#define SNAP_HVS_DLIST_OFFSET 0x4000u
/* The END word's value is a parameter. Linux writes END as bit 31
 * (rpi:vc4_regs.h:1359, as cited by hvs6_dlist.h HVS6_CTL0_END); a caller
 * passes this for a list written that way (the END word is that bit alone). */
#define SNAP_END_WORD_BIT31 0x80000000u

#define SNAP_R(b, o, n, c) { (b), (o), 32, (n), (c) }
#define SNAP_HVS_CH(g, p, c0, lp, st, dl, ch, cc)                                  \
	SNAP_R(SNAP_HVS, (c0) + (ch) * (p), g "_disp" #ch "_ctrl0", cc),           \
	SNAP_R(SNAP_HVS, (lp) + (ch) * (p), g "_disp" #ch "_lptrs", cc),           \
	SNAP_R(SNAP_HVS, (st) + (ch) * (p), g "_disp" #ch "_status", cc),          \
	SNAP_R(SNAP_HVS, (dl) + (ch) * (p), g "_disp" #ch "_dl", cc)
#define SNAP_HVS_C(ch) SNAP_HVS_CH("c", 0x20, 0x30, 0x3c, 0x44, 0x48, ch, \
	"rpi:vc4_regs.h:564,587,600,614 (stride :622)")
#define SNAP_HVS_D(ch) SNAP_HVS_CH("d", 0x40, 0x100, 0x110, 0x118, 0x11c, ch, \
	"rpi:vc4_regs.h:707,711,713,714 (stride :718)")

/* Generation C and D channel blocks are both listed (the generation is in
 * the version word): on the other generation the offsets read other
 * registers, so a decoder picks by the version word. */
static const struct snap_reg snap_table[] = {
	SNAP_R(SNAP_HVS, 0x0000, "version", "rpi:vc4_regs.h:543,676"),
	SNAP_R(SNAP_HVS, 0x0004, "cxm_size", "rpi:vc4_regs.h:547,677"),
	SNAP_HVS_C(0), SNAP_HVS_C(1), SNAP_HVS_C(2),
	SNAP_HVS_D(0), SNAP_HVS_D(1), SNAP_HVS_D(2),
	/* Pixel valve (facts:10: pixelvalve0/1 at bcm2712.dtsi:292-302; no
	 * register offsets in the facts file). These are the vc4 pixel valve
	 * layout from memory; that BCM2712's is the same is not confirmed. */
	SNAP_R(SNAP_PV, 0x00, "pv_control", "unverified"),
	SNAP_R(SNAP_PV, 0x04, "pv_v_control", "unverified"),
	SNAP_R(SNAP_PV, 0x0c, "pv_horza", "unverified"),
	SNAP_R(SNAP_PV, 0x10, "pv_horzb", "unverified"),
	SNAP_R(SNAP_PV, 0x14, "pv_verta", "unverified"),
	SNAP_R(SNAP_PV, 0x18, "pv_vertb", "unverified"),
	SNAP_R(SNAP_PV, 0x24, "pv_inten", "unverified"),
	SNAP_R(SNAP_PV, 0x28, "pv_intstat", "unverified"),
	SNAP_R(SNAP_PV, 0x2c, "pv_stat", "unverified"),
	/* HDMI0 'hd' region (facts:10: HDMI_VID_CTL bit 31 is
	 * VC4_HD_VID_CTL_ENABLE, vc4_hdmi.c:3391-3399, vc4_regs.h:1007). The
	 * offsets within the region are not in the facts file. */
	SNAP_R(SNAP_HDMI0, 0x038, "hdmi_vid_ctl", "unverified (offset; bit 31 ENABLE: facts:10)"),
	SNAP_R(SNAP_HDMI0, 0x00c, "hdmi_m_ctl", "unverified"),
	SNAP_R(SNAP_HDMI0, 0x0e8, "hdmi_frame_count", "unverified"),
};
#define SNAP_COUNT (sizeof(snap_table) / sizeof(snap_table[0]))

/* Injected 32-bit reader: block + byte offset from its base. */
struct snap_io {
	uint32_t (*read32)(void *ctx, enum snap_block block, uint32_t off);
	void *ctx;
};

/* Line sink: gets one NUL-terminated line, without a newline. */
struct snap_emit {
	void (*line)(void *ctx, const char *text);
	void *ctx;
};

static inline const char *snap_block_name(enum snap_block b)
{
	switch (b) {
	case SNAP_HVS: return "hvs";
	case SNAP_PV: return "pv";
	case SNAP_HDMI0: return "hdmi0";
	}
	return "?";
}

/* out[i] = the value of snap_table[i]; returns the count read (SNAP_COUNT;
 * 0 for a null io or out). Entries whose width is not 32 read as 0. */
static inline size_t snapshot_take(const struct snap_io *io, uint32_t out[])
{
	if (!io || !io->read32 || !out)
		return 0;
	for (size_t i = 0; i < SNAP_COUNT; i++)
		out[i] = snap_table[i].width == 32 ?
			io->read32(io->ctx, snap_table[i].block, snap_table[i].off) : 0;
	return SNAP_COUNT;
}

/* One 'u3 reg' line per table entry; returns the lines emitted. */
static inline size_t snapshot_print(const uint32_t out[], const struct snap_emit *emit)
{
	char buf[160];

	if (!out || !emit || !emit->line)
		return 0;
	for (size_t i = 0; i < SNAP_COUNT; i++) {
		(void)snprintf(buf, sizeof(buf), "u3 reg %s %s +0x%04x=0x%08x",
			       snap_block_name(snap_table[i].block), snap_table[i].name,
			       (unsigned)snap_table[i].off, (unsigned)out[i]);
		emit->line(emit->ctx, buf);
	}
	return SNAP_COUNT;
}

/* Read and print channel `ch`'s display list: words from `start_word` (an
 * index into the display-list memory; a caller takes it from the channel's
 * DL or LPTRS register), at most SNAP_DLIST_MAX, stopping after the first
 * word equal to `end_word`. Returns the lines emitted. */
static inline size_t snapshot_dlist(const struct snap_io *io, const struct snap_emit *emit,
				    unsigned ch, uint32_t start_word, uint32_t end_word)
{
	char buf[64];
	size_t n = 0;

	if (!io || !io->read32 || !emit || !emit->line)
		return 0;
	while (n < SNAP_DLIST_MAX) {
		uint32_t w = io->read32(io->ctx, SNAP_HVS,
					SNAP_HVS_DLIST_OFFSET + (start_word + (uint32_t)n) * 4u);

		(void)snprintf(buf, sizeof(buf), "u3 dlist %u [%u]=0x%08x", ch, (unsigned)n,
			       (unsigned)w);
		emit->line(emit->ctx, buf);
		n++;
		if (w == end_word)
			break;
	}
	return n;
}

#endif
