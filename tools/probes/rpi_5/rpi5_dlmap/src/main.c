/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_dlmap: a read-only map of the HVS6 display-list memory the firmware
 * left on the Pi 5, and of where its list really ends (docs/notes/
 * rpi5-display.md section 4, Q4). rpi5_scanout32 walked the firmware's list
 * by CTL0.NEXT for 256 words from channel 0's HEADE (672), went through 8
 * entries and found no END; this image reads the whole memory and walks
 * without that bound.
 *
 * It writes no HVS register and no list word; the only writes are the PM
 * watchdog's (the board layer, rpi_wdog.c). The list memory is copied once
 * into RAM with 32-bit reads and everything below works on the copy.
 *
 * Output (every line prefixed `rpi5_dlmap: `; verdicts.sh parses them):
 *
 *   hvs version 0x.. stepping D0|C0|unknown cxm_size 0x..
 *   mem_words <n> (words 0..n-1 of the list memory at HVS + 0x4000)
 *   ch<n> ctrl0 0x.. lptrs 0x.. heade <w> dl 0x.. status 0x..
 *   span <start>..<end> n=<count>      a maximal run of non-zero words
 *   zero <start>..<end>                a maximal run of at least 20 zero words
 *   end-word <i> 0x<val>               a word with bit 31 set
 *   walk ch<n> entry <k> word <i> ctl0 0x.. valid <0|1> next <n>
 *   walk ch<n> stop END at word <W> | NEXT 0 at word <W> |
 *                    past memory at word <W> | step bound at word <W>
 *   after-end zero <start>..<end> | after-end none (<reason>)
 *   done (...) | FAIL (<reason>)
 *
 * Span, zero and end-word lines are capped (`... more <n>` says how many
 * were left out), so the output stays well under 600 lines.
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>

#include <stdbool.h>
#include <stdint.h>

#include "hvs6_chan.h"
#include "hvs6_dlist.h"

#include "rpi_wdog.h"

#define HVS_BASE 0x107c580000ull /* rpi:bcm2712.dtsi:612-617, as rpi5_scanout16 */
#define HVS_SIZE 0x1a000u
#define ALL_ONES 0xffffffffu

/* The list memory is from HVS6_DLIST_OFFSET to the end of the mapping. */
#define MAX_WORDS ((HVS_SIZE - HVS6_DLIST_OFFSET) / 4u)

#define ZERO_MIN      20u /* shortest zero span printed */
#define MAX_SPANS     150u
#define MAX_ZEROS     80u
#define MAX_ENDWORDS  80u
#define WALK_STEPS    64u /* entries followed (and printed) per channel */

static uint32_t dl[MAX_WORDS];
static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

static mm_reg_t hvs;

static uint32_t hvs_rd(uint32_t off)
{
	return hvs != 0 && off < HVS_SIZE ? sys_read32(hvs + off) : ALL_ONES;
}

/* Walk channel `ch` from `head` by CTL0.NEXT. Returns the word of the END
 * that stopped it, or -1 when it stopped some other way. */
static int32_t walk(unsigned int ch, uint32_t head, uint32_t mem)
{
	uint32_t i = head;

	for (uint32_t k = 0; k < WALK_STEPS; k++) {
		uint32_t ctl0, next;

		if (i >= mem) {
			printk("rpi5_dlmap: walk ch%u stop past memory at word %u\n", ch, i);
			return -1;
		}
		ctl0 = dl[i];
		next = (ctl0 >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
		printk("rpi5_dlmap: walk ch%u entry %u word %u ctl0 0x%08x valid %u next %u\n", ch, k, i, ctl0,
		       (ctl0 & HVS6_CTL0_VALID) != 0, next);
		if (ctl0 & HVS6_CTL0_END) {
			printk("rpi5_dlmap: walk ch%u stop END at word %u\n", ch, i);
			return (int32_t)i;
		}
		if (next == 0) {
			printk("rpi5_dlmap: walk ch%u stop NEXT 0 at word %u\n", ch, i);
			return -1;
		}
		i += next;
	}
	printk("rpi5_dlmap: walk ch%u stop step bound at word %u\n", ch, i);
	return -1;
}

int main(void)
{
	enum yd5_hvs6_variant var = YD5_HVS6_D0;
	uint32_t version, cxm, mem = 0, heade[YD5_HVS6_NUM_CHANNELS] = { 0 };
	int32_t end0 = -1;
	const char *fail = NULL;

	printk("rpi5_dlmap: build %s\n", RPI5_DLMAP_BUILD);
	printk("rpi5_dlmap: read-only: no HVS register and no list word is written\n");

	device_map(&hvs, (uintptr_t)HVS_BASE, HVS_SIZE, K_MEM_CACHE_NONE);
	if (!hvs) {
		fail = "the HVS did not map";
		goto out;
	}

	version = hvs_rd(HVS6_VERSION);
	cxm = hvs_rd(HVS6_CXM_SIZE);
	if (yd5_hvs6_variant_from_version(version, &var) != 0) {
		printk("rpi5_dlmap: hvs version 0x%08x stepping unknown cxm_size 0x%08x\n", version, cxm);
		fail = "the HVS stepping is neither C0 nor D0";
		goto out;
	}
	printk("rpi5_dlmap: hvs version 0x%08x stepping %s cxm_size 0x%08x\n", version,
	       var == YD5_HVS6_D0 ? "D0" : "C0", cxm);

	/* (a) the channels */
	for (unsigned int ch = 0; ch < YD5_HVS6_NUM_CHANNELS; ch++) {
		uint32_t lptrs = hvs_rd(yd5_hvs6_lptrs_reg(var, ch));

		heade[ch] = lptrs & YD5_HVS6_LPTRS_HEADE_MASK;
		printk("rpi5_dlmap: ch%u ctrl0 0x%08x lptrs 0x%08x heade %u dl 0x%08x status 0x%08x\n", ch,
		       hvs_rd(yd5_hvs6_ctrl0_reg(var, ch)), lptrs, heade[ch],
		       hvs_rd(yd5_hvs6_reg(var, ch, YD5_HVS6C_DISP0_DL, YD5_HVS6D_DISP0_DL)),
		       hvs_rd(yd5_hvs6_status_reg(var, ch)));
	}

	/* (b) the memory size */
	if (cxm == ALL_ONES || cxm == 0) {
		fail = "CXM_SIZE unreadable or zero";
		goto out;
	}
	mem = MIN(cxm, MAX_WORDS);
	printk("rpi5_dlmap: mem_words %u (CXM_SIZE %u, mapped list words %u)\n", mem, cxm, MAX_WORDS);

	for (uint32_t i = 0; i < mem; i++)
		dl[i] = sys_read32(hvs + HVS6_DLIST_OFFSET + 4u * i);

	/* (c) non-zero spans and long zero spans */
	{
		uint32_t i = 0, nspan = 0, nzero = 0;

		while (i < mem) {
			uint32_t s = i;
			bool nz = dl[i] != 0;

			while (i < mem && (dl[i] != 0) == nz)
				i++;
			if (nz) {
				if (nspan++ < MAX_SPANS)
					printk("rpi5_dlmap: span %u..%u n=%u\n", s, i - 1, i - s);
			} else if (i - s >= ZERO_MIN) {
				if (nzero++ < MAX_ZEROS)
					printk("rpi5_dlmap: zero %u..%u\n", s, i - 1);
			}
		}
		if (nspan > MAX_SPANS)
			printk("rpi5_dlmap: span ... more %u\n", nspan - MAX_SPANS);
		if (nzero > MAX_ZEROS)
			printk("rpi5_dlmap: zero ... more %u\n", nzero - MAX_ZEROS);
	}

	/* (d) words with the END bit */
	{
		uint32_t n = 0;

		for (uint32_t i = 0; i < mem; i++) {
			if (dl[i] & HVS6_CTL0_END) {
				if (n++ < MAX_ENDWORDS)
					printk("rpi5_dlmap: end-word %u 0x%08x\n", i, dl[i]);
			}
		}
		if (n > MAX_ENDWORDS)
			printk("rpi5_dlmap: end-word ... more %u\n", n - MAX_ENDWORDS);
	}

	/* (e) the walks */
	for (unsigned int ch = 0; ch < YD5_HVS6_NUM_CHANNELS; ch++) {
		int32_t e = walk(ch, heade[ch], mem);

		if (ch == 0)
			end0 = e;
	}

	/* (f) the first zero span of at least ZERO_MIN words after channel 0's
	 * END, clamped to start at the first word Linux uses (32). */
	if (end0 < 0) {
		printk("rpi5_dlmap: after-end none (channel 0's walk found no END)\n");
	} else {
		uint32_t i = (uint32_t)end0 + 1u, found = 0;

		if (i < HVS6_BOOTLOADER_DLIST_END)
			i = HVS6_BOOTLOADER_DLIST_END;
		while (i < mem && !found) {
			uint32_t s = i;

			while (i < mem && dl[i] == 0)
				i++;
			if (i - s >= ZERO_MIN) {
				printk("rpi5_dlmap: after-end zero %u..%u\n", s, i - 1);
				found = 1;
			}
			while (i < mem && dl[i] != 0)
				i++;
		}
		if (!found)
			printk("rpi5_dlmap: after-end none (no zero span of %u words after word %d)\n", ZERO_MIN,
			       end0);
	}

out:
	if (fail)
		printk("rpi5_dlmap: FAIL (%s)\n", fail);
	else
		printk("rpi5_dlmap: done (mem_words %u, channel 0 END %s)\n", mem, end0 >= 0 ? "found" : "not found");

	finished = true;
	printk("rpi5_dlmap: feeding stopped, reset expected (the PM watchdog resets the SoC within "
	       "10 s, and tryboot returns the board to Linux)\n");
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
