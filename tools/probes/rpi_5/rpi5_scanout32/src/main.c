/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout32, skeleton: stage 2 of docs/notes/rpi5-display.md. Later this
 * image places our own XRGB8888 plane after word 32 of the firmware's display
 * list and swaps channel 0's list pointer at vblank. This version only reads:
 *
 *   S1  the HVS VERSION word and the stepping it names
 *   S2  channel 0's CTRL0, LPTRS, STATUS (FRCNT) and DL words, decoded
 *   S3  the firmware's display list: every word from the list start through
 *       word 32 and on to the list's END, one line per word
 *
 * Every read prints a line, also when it cannot be made. Nothing is written:
 * no register, no list word, no buffer. The list is read into a snapshot
 * (snap_dlist) that the byte-equal restore check of the full probe reuses.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <stdbool.h>
#include <stdint.h>

#include "hvs6_dlist.h"
#include "rpi_wdog.h"

#define HVS_BASE  0x107c580000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:612-617 */
#define HVS_SIZE  0x1a000u
#define ALL_ONES  0xffffffffu
#define CHAN      0u             /* the channel the firmware drives HDMI0 from */
#define DL_WORDS  ((HVS_SIZE - HVS6_DLIST_OFFSET) / 4u)
#define SNAP_MAX  256u           /* words kept: the bootloader's 32 and what follows */

static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

static mm_reg_t hvs;

/* One read of an HVS word; ALL_ONES when the block was not mapped. */
static uint32_t hvs_rd(uint32_t off)
{
	return hvs != 0 ? sys_read32(hvs + off) : ALL_ONES;
}

/* The display-list memory's words [from, from + n) into dst, read word by
 * word (the memory is device memory, no cache line holds a copy). Returns
 * how many were read: n, or 0 when the HVS is not mapped. */
static uint32_t snap_dlist(uint32_t *dst, uint32_t from, uint32_t n)
{
	uint32_t i;

	if (hvs == 0 || from >= DL_WORDS) {
		return 0;
	}
	if (n > DL_WORDS - from) {
		n = DL_WORDS - from;
	}
	for (i = 0; i < n; i++) {
		dst[i] = sys_read32(hvs + HVS6_DLIST_OFFSET + (from + i) * 4u);
	}
	return n;
}

static uint32_t snap[SNAP_MAX];

int main(void)
{
	enum hvs6_gen gen;
	struct hvs6_chan ch;
	struct hvs6_plane pl;
	uint32_t version, ctrl0, lptrs, status, dl;
	uint32_t start, got, i, next_entry, end_idx;
	bool saw_end = false;
	const char *fmt;
	int rc;

	printk("rpi5_scanout32: build %s\n", RPI5_SCANOUT32_BUILD);
	rpi_wdog_announce();
	printk("rpi5_scanout32: read-only skeleton: no register, list word or buffer is written\n");

	/* S1 */
	device_map(&hvs, (uintptr_t)HVS_BASE, HVS_SIZE, K_MEM_CACHE_NONE);
	if (hvs == 0) {
		printk("rpi5_scanout32: S1 hvs NOT mapped at 0x%llx (every word reads 0x%08x)\n",
		       (unsigned long long)HVS_BASE, ALL_ONES);
	} else {
		printk("rpi5_scanout32: S1 hvs mapped at 0x%llx, %u bytes\n",
		       (unsigned long long)HVS_BASE, HVS_SIZE);
	}
	version = hvs_rd(HVS6_VERSION);
	gen = hvs6_gen_from_version(version);
	printk("rpi5_scanout32: S1 VERSION 0x%08x (bits 7:0 0x%02x): stepping %s\n", version,
	       version & HVS6_VERSION_MASK,
	       gen == HVS6_GEN_D ? "D (D0)" : gen == HVS6_GEN_C ? "C (C0)" : "UNKNOWN");
	printk("rpi5_scanout32: S1 CXM_SIZE 0x%08x\n", hvs_rd(HVS6_CXM_SIZE));

	/* S2 */
	ctrl0 = hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_CTRL0);
	if (ctrl0 == 0) {
		printk("rpi5_scanout32: S2 channel %u registers NOT read: unknown stepping\n", CHAN);
		lptrs = status = dl = ALL_ONES;
		ctrl0 = ALL_ONES;
	} else {
		lptrs = hvs_rd(hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_LPTRS));
		status = hvs_rd(hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_STATUS));
		dl = hvs_rd(hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_DL));
		ctrl0 = hvs_rd(hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_CTRL0));
	}
	printk("rpi5_scanout32: S2 ch%u CTRL0 0x%08x LPTRS 0x%08x STATUS 0x%08x DL 0x%08x\n", CHAN,
	       ctrl0, lptrs, status, dl);
	hvs6_chan_decode(ctrl0, lptrs, status, dl, &ch);
	printk("rpi5_scanout32: S2 ch%u enabled %d mode %u running %d empty %d FRCNT %u YLINE %u "
	       "FWIDTH %u LINES %u\n",
	       CHAN, ch.enabled, ch.mode, ch.running, ch.empty, ch.frcnt, ch.yline, ch.fwidth,
	       ch.lines);
	printk("rpi5_scanout32: S2 ch%u list start (LPTRS.HEADE) %u, running list (DL.LACT) %u\n",
	       CHAN, ch.head, ch.active);

	/* S3: from the list the channel runs. */
	start = ch.active;
	got = snap_dlist(snap, start, SNAP_MAX);
	if (got == 0) {
		printk("rpi5_scanout32: S3 display list NOT read (HVS not mapped or start %u out of "
		       "range)\n", start);
	}
	/* Walk the entries to find the END word (bit 31 of a control word 0). */
	end_idx = 0;
	next_entry = 0;
	while (next_entry < got) {
		uint32_t w = snap[next_entry], n;

		if (w & HVS6_CTL0_END) {
			end_idx = next_entry;
			saw_end = true;
			break;
		}
		n = (w >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
		if (n == 0) { /* not a valid entry length: stop, report below */
			break;
		}
		next_entry += n;
	}
	if (got != 0) {
		printk("rpi5_scanout32: S3 list read from word %u: %u words; END %s",
		       start, got, saw_end ? "at word" : "NOT found within them");
		if (saw_end) {
			printk(" %u (list index %u)\n", start + end_idx, end_idx);
		} else {
			printk(" (entry walk stopped at list index %u)\n", next_entry);
		}
	}

	/* One line per word: from the start through word 32 and on to the END. */
	for (i = 0; i < got; i++) {
		uint32_t abs_idx = start + i;
		bool past_32 = abs_idx > HVS6_BOOTLOADER_DLIST_END;

		if (past_32 && (!saw_end || i > end_idx)) {
			break;
		}
		printk("rpi5_scanout32: S3 dl[%3u] = 0x%08x", abs_idx, snap[i]);
		if (abs_idx == HVS6_BOOTLOADER_DLIST_END) {
			printk("  <- word 32, where our entries go");
		}
		printk("\n");
	}

	/* The first plane through the decoder; it reads at the entry start. */
	if (got != 0) {
		rc = hvs6_dlist_first_plane(snap, got, 0, &pl);
		if (rc != HVS6_OK) {
			printk("rpi5_scanout32: S3 first plane NOT decoded: hvs6 error %d\n", rc);
		} else {
			fmt = hvs6_format_name(pl.format_code);
			printk("rpi5_scanout32: S3 first plane addr 0x%010llx pitch %u %ux%u at %u,%u "
			       "format %u (%s) rgb32 %d next %u\n",
			       (unsigned long long)pl.addr, pl.pitch, pl.width, pl.height, pl.x,
			       pl.y, pl.format_code, fmt != NULL ? fmt : "unnamed", pl.rgb32,
			       pl.nwords);
		}
	}

	printk("rpi5_scanout32: skeleton done\n");
	finished = true;
	return 0;
}
