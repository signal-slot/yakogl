/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout32: stage 2 of docs/notes/rpi5-display.md (section 3, Q4). Our
 * own 32 bpp XRGB8888 plane, in a display list placed at or after word 32 of
 * the HVS6 list memory and outside the firmware's list, put on channel 0 by
 * swapping its list pointer under the firmware's running scanout, alternated
 * between two buffers A and B, then the firmware's list pointer and words
 * given back byte-equal.
 *
 * The firmware's list is wherever channel 0's LPTRS.HEADE names, read at run
 * time (on the board it was word 672, with no END within 256 words), never
 * assumed to be words 0..31.
 *
 *   S1  the HVS VERSION word and the stepping it names
 *   S2  channel 0's CTRL0, LPTRS, STATUS (FRCNT) and DL words, decoded, and
 *       STATUS read several times to show FRCNT advancing
 *   S3  the firmware's display list from LPTRS.HEADE: every word to its END,
 *       or the first HVS6S_FW_MAX words when no END is in view
 *   S4  pixel valve 0 (HDMI0's) PV_CONTROL and PV_V_CONTROL, read only
 *   S5  the snapshot (hvs6_dlist_swap.h): LPTRS, words 0..31 and the
 *       firmware's list, plus SNAP_MAX words from HEADE kept by the probe and
 *       hashed                                     -> u3 snapshot
 *   S6  our list pair placed (hvs6s_place): slot A and slot B, 10 words each
 *       (hvs6_dlist_write.h's entry), refused when it does not fit; then,
 *       before any write, the span read and refused unless every word reads
 *       zero twice, it lies outside the words read from HEADE (and before
 *       HEADE when the firmware's list has no END in view) and no channel's
 *       list pointer is inside it                   -> u3 dlist
 *   S7  buffers A and B filled (pattern.h, A as XRGB8888, B with red and
 *       blue exchanged) and cleaned to memory          -> u3 plane
 *   S8  SWAPS swaps, A, B, A, B ...: each one writes the slot's list and then
 *       LPTRS (hvs6s_swap), waits for FRCNT past it, judged by swap_check.h
 *                                          -> u3 swap <k>, u3 named
 *   S9  a sample of each buffer read back after a clean+invalidate
 *   S10 the restore (hvs6s_restore): the firmware's LPTRS written back, two
 *       frames, then LPTRS, words 0..31 and the SNAP_MAX words from HEADE
 *       compared with the snapshot through swap_check.h's swap_restore_judge,
 *       and our span written back to zero once channel 0 is off it
 *                                                        -> u3 restore
 *   then the dlswap record of swap 1 and the restore, and the end line.
 *
 * The restore runs whenever something was written, whatever failed after
 * it; when nothing was (placement refused), S10 only compares. The pixel
 * valve, HDMI and PHY are only read, never written. Buffers A
 * and B are .bss RAM in the image's own (cacheable) mapping: they are cleaned
 * with sys_cache_data_flush_range, never mapped a second time.
 *
 * The console lines named u3 and the end line are the formats of
 * verdicts.sh's header.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <stdbool.h>
#include <stdint.h>

#include "bus_addr.h"
#include "hvs6_dlist.h"
#include "hvs6_dlist_swap.h"
#include "hvs6_dlist_write.h"
#include "pattern.h"
#include "pv_hdmi.h"
#include "rpi_wdog.h"
#include "scanout_alive.h"
#include "swap_check.h"

#define HVS_BASE  0x107c580000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:612-617 */
#define HVS_SIZE  0x1a000u
#define ALL_ONES  0xffffffffu
#define CHAN      0u             /* the channel the firmware drives HDMI0 from */
#define DL_WORDS  ((HVS_SIZE - HVS6_DLIST_OFFSET) / 4u)
#define SNAP_MAX  256u           /* words kept: the bootloader's 32 and what follows */

/* Our buffers: the firmware's mode is 1920x1080; a smaller one is used whole. */
#define FB_MAX_W  1920u
#define FB_MAX_H  1080u
#define FB_BPP    4u
#define FB_BYTES  (FB_MAX_W * FB_MAX_H * FB_BPP)

#define SWAPS     8u    /* at least verdicts.sh's MIN_SWAPS (4) */
#define HOLD_MS   1000u /* each buffer stays on screen this long */
#define POLL_US   1000u /* one poll interval: well under a frame */
#define MAX_POLLS 200u  /* 200 ms per wait, about 12 frames: under 64 (FRCNT aliasing) */
#define CHECK_STEP 16u  /* readback grid: every 16th pixel in x and y */
#define SLOT_WORDS HVS6W_LIST_WORDS
#define PAIR_WORDS (2u * SLOT_WORDS)
#define FRCNT_READS  6u  /* S2: STATUS reads to show FRCNT advancing */
#define FRCNT_GAP_MS 20u /* about 1.2 frames at 60 Hz: well under 64 frames */

static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

static mm_reg_t hvs;
static mm_reg_t pv0;

static uint8_t fb_a[FB_BYTES] __aligned(4096);
static uint8_t fb_b[FB_BYTES] __aligned(4096);

/* One read of an HVS word; ALL_ONES when the block was not mapped. */
static uint32_t hvs_rd(uint32_t off)
{
	return hvs != 0 && off < HVS_SIZE ? sys_read32(hvs + off) : ALL_ONES;
}

static uint32_t pv_rd(uint32_t off)
{
	return pv0 != 0 && off < PV0_SIZE ? sys_read32(pv0 + off) : ALL_ONES;
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

/* ---- hvs6s_io over the mapped HVS ---------------------------------------- */

static uint32_t io_read32(void *ctx, uint32_t off)
{
	ARG_UNUSED(ctx);
	return hvs_rd(off);
}

static void io_write32(void *ctx, uint32_t off, uint32_t val)
{
	ARG_UNUSED(ctx);
	if (hvs != 0 && off < HVS_SIZE) {
		sys_write32(val, hvs + off);
	}
}

/* The list memory is device memory in the SoC window: no cache line holds it,
 * nothing to clean (hvs6_dlist_swap.h). Only a range outside that window,
 * which this probe never passes, would be RAM in Zephyr's identity map. */
static void io_clean(void *ctx, uint64_t addr, uint64_t len)
{
	ARG_UNUSED(ctx);
	if (bcm2712_addr_in_soc(addr)) {
		return;
	}
	(void)sys_cache_data_flush_range((void *)(uintptr_t)addr, (size_t)len);
}

static void io_barrier(void *ctx)
{
	ARG_UNUSED(ctx);
	barrier_dsync_fence_full();
}

static void io_wait(void *ctx)
{
	ARG_UNUSED(ctx);
	k_busy_wait(POLL_US);
}

static const struct hvs6s_io io = {
	.read32 = io_read32,
	.write32 = io_write32,
	.clean = io_clean,
	.barrier = io_barrier,
	.wait = io_wait,
	.ctx = NULL,
};

static void emit_line(void *ctx, const char *text)
{
	ARG_UNUSED(ctx);
	printk("%s\n", text);
}

static const struct hvs6s_emit emit = { emit_line, NULL };

/* FNV-1a over the bytes of n words, little-endian. */
static uint32_t words_hash(const uint32_t *w, uint32_t n)
{
	uint32_t h = 2166136261u;

	for (uint32_t i = 0; i < n; i++) {
		for (unsigned int b = 0; b < 4u; b++) {
			h = (h ^ ((w[i] >> (8u * b)) & 0xffu)) * 16777619u;
		}
	}
	return h;
}

/* The 40-bit address the plane entry at list word `start` names, read back
 * from list memory and decoded; 0 (and *rc an hvs6 error) when it does not
 * decode. */
static uint64_t slot_plane_addr(uint32_t start, int *rc)
{
	uint32_t w[SLOT_WORDS];
	struct hvs6_plane pl;

	for (uint32_t i = 0; i < SLOT_WORDS; i++) {
		w[i] = hvs_rd(HVS6_DLIST_OFFSET + (start + i) * 4u);
	}
	*rc = hvs6_dlist_first_plane(w, SLOT_WORDS, 0, &pl);
	return *rc == HVS6_OK ? pl.addr : 0;
}

static struct hvs6s_snap fwsnap;
static struct swap_step steps[SWAPS];

/* The firmware's words from LPTRS.HEADE on, SNAP_MAX of them, taken with the
 * snapshot and compared after the restore: the list the channel runs, not
 * words 0..31 (the board's list starts at word 672 and has no END in 256). */
static uint32_t ext[SNAP_MAX], ext_rb[SNAP_MAX];
static uint32_t ext_n;
/* Our span's words as they read before the first write (all zero, checked). */
static uint32_t pair_prev[PAIR_WORDS];

/*
 * Before any list word is written: the span [at, at + len) is read and
 * checked against everything the probe can see. NULL when it is safe;
 * otherwise the reason, *err the dlswap code and *bad the word that refused
 * it (when one did), and nothing may be written.
 */
static const char *place_check(const struct hvs6s_snap *s, uint32_t at, uint32_t len,
			       enum hvs6s_err *err, uint32_t *bad, uint32_t *bad_val)
{
	*err = HVS6S_ERR_PLACE_FW;
	*bad = 0;
	*bad_val = 0;
	if (at < HVS6S_KEPT) {
		*err = HVS6S_ERR_PLACE_KEPT;
		return "overlaps words 0..31";
	}
	if (len > PAIR_WORDS) {
		*err = HVS6S_ERR_PLACE_FIT;
		return "span longer than the probe's pair";
	}
	if (hvs6s_overlap(at, len, s->fw_head, s->fw_len)) {
		return "overlaps the firmware's walked list";
	}
	if (hvs6s_overlap(at, len, s->fw_head, ext_n)) {
		return "overlaps the words read from HEADE";
	}
	if (!s->fw_end_found && (uint64_t)at + len > s->fw_head) {
		return "reaches HEADE or past it while the firmware's list has no END in view";
	}
	for (unsigned int c = 0; c < YD5_HVS6_NUM_CHANNELS; c++) {
		uint32_t lp = hvs_rd(hvs6s_lptrs_reg(c)) & YD5_HVS6_LPTRS_HEADE_MASK;
		uint32_t la = hvs_rd(hvs6s_dl_reg(c)) & HVS6S_DL_LACT_MASK;

		if (hvs6s_overlap(at, len, lp, 1) || hvs6s_overlap(at, len, la, 1)) {
			*bad = hvs6s_overlap(at, len, lp, 1) ? lp : la;
			*bad_val = c;
			return "a channel's list pointer is inside it";
		}
	}
	/* Two reads a frame apart: every word zero both times. */
	for (unsigned int pass = 0; pass < 2u; pass++) {
		if (pass != 0) {
			k_msleep(FRCNT_GAP_MS);
		}
		for (uint32_t i = 0; i < len; i++) {
			uint32_t w = hvs_rd(hvs6s_word_off(at + i));

			if (w != 0) {
				*bad = at + i;
				*bad_val = w;
				return "a word in it is not zero";
			}
			pair_prev[i] = w;
		}
	}
	*err = HVS6S_OK;
	return NULL;
}

int main(void)
{
	enum hvs6_gen gen;
	struct hvs6_chan ch;
	struct hvs6_plane pl;
	uint32_t version, ctrl0, lptrs, status, dl, cxm;
	uint32_t start, got, i, next_entry, end_idx;
	bool saw_end = false;
	const char *fmt;
	int rc;

	struct hvs6s_cfg cfg = {0};
	struct hvs6s_report rec, rrest;
	enum hvs6s_err snap_err = HVS6S_ERR_ARG, place_err = HVS6S_ERR_ARG, first_err = HVS6S_OK;
	uint32_t pair = 0, slot[2] = {0, 0};
	uint32_t fw_w = FB_MAX_W, fw_h = FB_MAX_H, pitch;
	uint64_t phys[2] = {0, 0}, bus[2] = {0, 0};
	bool bufs_ok = false, placed = false, named = false, readback_ok = false;
	bool restore_pass = false, refused = false, wrote = false;
	unsigned int nsteps = 0, swaps_ok = 0;
	const char *fail = NULL;
	char failbuf[96];

	printk("rpi5_scanout32: build %s\n", RPI5_SCANOUT32_BUILD);
	rpi_wdog_announce();
	printk("rpi5_scanout32: stage 2: our XRGB8888 list in free words at or after word 32 "
	       "(read and checked first), %u swaps A/B on "
	       "channel %u, then the firmware's list restored; no pixel valve, HDMI or PHY write\n",
	       SWAPS, CHAN);

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
	cxm = hvs_rd(HVS6_CXM_SIZE);
	printk("rpi5_scanout32: S1 CXM_SIZE 0x%08x\n", cxm);

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

	/* S2: STATUS read several times, FRCNT shown advancing under the firmware. */
	if (hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_STATUS) == 0 || hvs == 0) {
		printk("rpi5_scanout32: S2 ch%u FRCNT series NOT read (HVS not mapped or unknown "
		       "stepping)\n", CHAN);
	} else {
		uint32_t fr[FRCNT_READS], adv = 0;

		for (i = 0; i < FRCNT_READS; i++) {
			if (i != 0) {
				k_msleep(FRCNT_GAP_MS);
			}
			fr[i] = swap_frcnt(hvs_rd(hvs6_chan_reg_offset(gen, CHAN, HVS6_REG_STATUS)));
			if (i != 0) {
				adv += (fr[i] - fr[i - 1]) & SWAP_FRCNT_MASK;
			}
		}
		printk("rpi5_scanout32: S2 ch%u FRCNT series", CHAN);
		for (i = 0; i < FRCNT_READS; i++) {
			printk(" %u", fr[i]);
		}
		printk(" (%u reads %u ms apart): %u frames, %s\n", FRCNT_READS, FRCNT_GAP_MS, adv,
		       adv != 0 ? "advancing" : "NOT advancing");
	}

	/* S3: from the list start the channel names (LPTRS.HEADE), never word 0. */
	start = ch.head;
	if (ch.active != ch.head) {
		printk("rpi5_scanout32: S3 note: DL.LACT %u differs from LPTRS.HEADE %u; the list "
		       "is read from HEADE\n", ch.active, ch.head);
	}
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

	/* One line per word: from HEADE to the END, or the first HVS6S_FW_MAX
	 * words when no END is in view (the board's list had none in 256). */
	for (i = 0; i < got; i++) {
		if (saw_end ? i > end_idx : i >= HVS6S_FW_MAX) {
			break;
		}
		printk("rpi5_scanout32: S3 dl[%3u] = 0x%08x\n", start + i, snap[i]);
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
			if (pl.width != 0 && pl.width <= FB_MAX_W && pl.height != 0 &&
			    pl.height <= FB_MAX_H) {
				fw_w = pl.width;
				fw_h = pl.height;
			}
		}
	}

	/* S4: the pixel valve, read only (swap_check.h's valve check). */
	device_map(&pv0, (uintptr_t)PV0_BASE, PV0_SIZE, K_MEM_CACHE_NONE);
	printk("rpi5_scanout32: S4 pv0 %s at 0x%llx: PV_CONTROL 0x%08x PV_V_CONTROL 0x%08x\n",
	       pv0 != 0 ? "mapped" : "NOT mapped", (unsigned long long)PV0_BASE,
	       pv_rd(SA_PV_CONTROL), pv_rd(SA_PV_V_CONTROL));

	/* S5: the snapshot. hvs6_dlist_swap.h's offsets are D0's. */
	cfg.chan = CHAN;
	cfg.mem_words = cxm == ALL_ONES ? 0u : MIN(cxm, DL_WORDS);
	cfg.start = HVS6S_AUTO;
	cfg.list_arm = HVS_BASE + HVS6_DLIST_OFFSET;
	cfg.cache_line = (uint32_t)sys_cache_data_line_size_get();
	cfg.max_polls = MAX_POLLS;
	if (hvs == 0) {
		printk("rpi5_scanout32: S5 dlswap snapshot FAIL: HVS not mapped\n");
	} else if (gen != HVS6_GEN_D) {
		printk("rpi5_scanout32: S5 dlswap snapshot FAIL: stepping is not D0 (the swap's "
		       "offsets are D0's)\n");
	} else {
		snap_err = hvs6s_snapshot(&io, &cfg, &fwsnap);
		if (snap_err == HVS6S_OK) {
			ext_n = snap_dlist(ext, fwsnap.fw_head, SNAP_MAX);
			if (ext_n > cfg.mem_words - fwsnap.fw_head) {
				ext_n = cfg.mem_words - fwsnap.fw_head;
			}
			if (ext_n == 0) {
				snap_err = HVS6S_ERR_SNAPSHOT;
			}
		}
		printk("rpi5_scanout32: S5 dlswap snapshot %s (%s) mem_words %u lptrs 0x%08x "
		       "fw_head %u fw_len %u end %s\n",
		       snap_err == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(snap_err),
		       cfg.mem_words, fwsnap.lptrs, fwsnap.fw_head, fwsnap.fw_len,
		       fwsnap.fw_end_found ? "found" : "capped");
	}
	if (snap_err == HVS6S_OK) {
		printk("rpi5_scanout32: S5 kept for the restore: LPTRS 0x%08x, words 0..%u, the "
		       "firmware's list words %u..%u (%u from HEADE)\n", fwsnap.lptrs,
		       HVS6S_KEPT - 1u, fwsnap.fw_head, fwsnap.fw_head + ext_n - 1u, ext_n);
		/* The hash is over the words from HEADE: the list the channel runs. */
		printk("u3 snapshot: list-ptr=0x%08x words=%u hash=0x%08x\n", fwsnap.lptrs, ext_n,
		       words_hash(ext, ext_n));
	} else {
		printk("u3 snapshot: list-ptr=0x%08x words=0 hash=0x00000000 FAIL no snapshot\n",
		       lptrs);
		first_err = snap_err;
		fail = "no snapshot of the firmware's list";
	}

	/* S6: one span for slot A and slot B, at or after word 32, outside the
	 * firmware's list; then read and checked before anything is written. */
	if (snap_err == HVS6S_OK) {
		place_err = hvs6s_place(&cfg, &fwsnap, PAIR_WORDS, &pair);
		placed = place_err == HVS6S_OK;
	}
	if (placed) {
		uint32_t bad, bad_val;
		const char *why;

		for (unsigned int c = 0; c < YD5_HVS6_NUM_CHANNELS; c++) {
			printk("rpi5_scanout32: S6 ch%u LPTRS 0x%08x DL 0x%08x\n", c,
			       hvs_rd(hvs6s_lptrs_reg(c)), hvs_rd(hvs6s_dl_reg(c)));
		}
		why = place_check(&fwsnap, pair, PAIR_WORDS, &place_err, &bad, &bad_val);
		if (why == NULL) {
			printk("rpi5_scanout32: S6 placement check ok: words %u..%u read zero twice, "
			       "outside words 0..31, the firmware's list from HEADE %u (%u words "
			       "read, END %s) and every channel's list pointer\n",
			       pair, pair + PAIR_WORDS - 1u, fwsnap.fw_head, ext_n,
			       fwsnap.fw_end_found ? "found" : "not in view");
		} else {
			placed = false;
			refused = true;
			printk("rpi5_scanout32: S6 placement check FAIL: words %u..%u %s (word/channel "
			       "%u value 0x%08x); nothing is written\n",
			       pair, pair + PAIR_WORDS - 1u, why, bad, bad_val);
			printk("u3 dlist: placed word 0 len 0 FAIL refused at word %u: %s\n", pair,
			       why);
			if (first_err == HVS6S_OK) {
				first_err = place_err;
				fail = "our list was refused: its words are not known free";
			}
		}
	}
	if (placed) {
		slot[0] = pair;
		slot[1] = pair + SLOT_WORDS;
		printk("rpi5_scanout32: S6 dlswap place ok: slot A word %u, slot B word %u, %u "
		       "words each\n", slot[0], slot[1], SLOT_WORDS);
		printk("u3 dlist: placed word %u len %u (slot A %u, slot B %u)\n", pair,
		       2u * SLOT_WORDS, slot[0], slot[1]);
	} else if (!refused) {
		printk("rpi5_scanout32: S6 dlswap place %s: %s\n",
		       snap_err == HVS6S_OK ? "FAIL" : "skip",
		       snap_err == HVS6S_OK ? hvs6s_err_name(place_err) : "no snapshot");
		printk("u3 dlist: placed word 0 len 0 FAIL %s\n",
		       snap_err == HVS6S_OK ? hvs6s_err_name(place_err) : "no snapshot");
		if (first_err == HVS6S_OK) {
			first_err = place_err;
			fail = "our list was not placed";
		}
	}

	/* S7: the buffers, in the image's own cacheable RAM, cleaned to memory. */
	pitch = fw_w * FB_BPP;
	phys[0] = k_mem_phys_addr(fb_a);
	phys[1] = k_mem_phys_addr(fb_b);
	{
		int fa = pattern_fill(fb_a, fw_w, fw_h, pitch, 32, PATTERN_XRGB8888);
		int fb = pattern_fill(fb_b, fw_w, fw_h, pitch, 32, PATTERN_XBGR8888);
		int ca = sys_cache_data_flush_range(fb_a, (size_t)pitch * fw_h);
		int cb = sys_cache_data_flush_range(fb_b, (size_t)pitch * fw_h);
		int ba = bcm2712_arm_to_bus(phys[0], &bus[0]);
		int bb = bcm2712_arm_to_bus(phys[1], &bus[1]);

		bufs_ok = fa == 0 && fb == 0 && ca == 0 && cb == 0 && ba == 0 && bb == 0;
		printk("rpi5_scanout32: S7 buffers %s: A phys 0x%010llx bus 0x%010llx fill %d "
		       "flush %d, B phys 0x%010llx bus 0x%010llx fill %d flush %d, %ux%u pitch %u\n",
		       bufs_ok ? "ok" : "FAIL", (unsigned long long)phys[0],
		       (unsigned long long)bus[0], fa, ca, (unsigned long long)phys[1],
		       (unsigned long long)bus[1], fb, cb, fw_w, fw_h, pitch);
	}
	printk("u3 plane: addr=0x%010llx fmt=XRGB8888 pitch=%u w=%u h=%u addr_b=0x%010llx "
	       "pattern_a=bars pattern_b=bars-red-blue-exchanged%s\n",
	       (unsigned long long)bus[0], pitch, fw_w, fw_h, (unsigned long long)bus[1],
	       bufs_ok ? "" : " FAIL buffers not ready");
	if (!bufs_ok && first_err == HVS6S_OK) {
		first_err = HVS6S_ERR_BUS;
		fail = "buffers A/B not ready";
	}

	/* S8: the swaps, A, B, A, B ... */
	hvs6s_report_init(&rec);
	cfg.plane.format = HVS6W_XRGB8888;
	cfg.plane.x = 0;
	cfg.plane.y = 0;
	cfg.plane.width = fw_w;
	cfg.plane.height = fw_h;
	cfg.plane.pitch = pitch;
	for (unsigned int k = 0; k < SWAPS; k++) {
		unsigned int b = k & 1u;
		struct hvs6s_report r;
		struct swap_step *st = &steps[k];
		enum hvs6s_err e;
		enum swap_reason why;
		uint32_t pre_status, pre_lptrs, post_status, post_lptrs;
		int prc;

		if (!placed || !bufs_ok) {
			printk("u3 swap %u: skipped (%s)\n", k + 1u,
			       !placed ? "not placed" : "buffers not ready");
			continue;
		}
		pre_status = hvs_rd(hvs6s_status_reg(CHAN));
		pre_lptrs = hvs_rd(hvs6s_lptrs_reg(CHAN));
		cfg.plane.addr = bus[b];
		hvs6s_report_init(&r);
		wrote = true; /* from here on the restore writes LPTRS back */
		e = hvs6s_swap(&io, &cfg, slot[b], &r);
		post_status = hvs_rd(hvs6s_status_reg(CHAN));
		post_lptrs = hvs_rd(hvs6s_lptrs_reg(CHAN));

		st->sample.ptr_before = r.swapped ? r.at_write.lptrs : pre_lptrs;
		st->sample.ptr_after = post_lptrs;
		st->sample.expected_word = slot[b];
		st->sample.status_before = r.swapped ? r.at_write.status : pre_status;
		st->sample.status_after = post_status;
		st->sample.pv_control = pv_rd(SA_PV_CONTROL);
		st->sample.pv_vcontrol = pv_rd(SA_PV_V_CONTROL);
		st->plane_addr = slot_plane_addr(slot[b], &prc);
		st->expected_addr = bus[b];
		nsteps = k + 1u;
		why = swap_judge(&st->sample);

		hvs6s_regs_line(&emit, "at-write", &r.at_write);
		hvs6s_regs_line(&emit, "after", &r.after);
		printk("dlswap swap%u %s %s lptrs-readback %s frcnt-advance %s settle %s "
		       "lact-follows %s (record only)\n",
		       k + 1u, e == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(e),
		       r.lptrs_ok ? "ok" : "FAIL", r.frcnt_ok ? "ok" : "FAIL",
		       r.settle_ok ? "ok" : (r.frcnt_ok ? "FAIL" : "skip"),
		       r.lact_follows ? "yes" : "no");
		printk("u3 swap %u: buf %c list-ptr=0x%08x frcnt %u -> %u judge=%s slot=%u "
		       "plane=0x%010llx%s\n",
		       k + 1u, b ? 'B' : 'A', post_lptrs, swap_frcnt(st->sample.status_before),
		       swap_frcnt(post_status), swap_reason_name(why), slot[b],
		       (unsigned long long)st->plane_addr,
		       prc != HVS6_OK ? " (entry did not decode)" : "");
		if (k == 0) {
			named = post_lptrs != ALL_ONES &&
				(post_lptrs & SWAP_LPTRS_MASK) == slot[0];
			printk("u3 named: %s LPTRS 0x%08x after swap 1, our list at word %u, "
			       "first read 0x%08x poll %u\n",
			       named ? "yes" : "no", post_lptrs, slot[0], r.lptrs_first,
			       r.lptrs_poll);
			rec = r;
		}
		if (e != HVS6S_OK && first_err == HVS6S_OK) {
			first_err = e;
		}
		if (e == HVS6S_OK && why == SWAP_OURS) {
			swaps_ok++;
		}
		k_msleep(HOLD_MS);
	}
	if (!placed || !bufs_ok) {
		printk("u3 named: no (no swap was made)\n");
	} else {
		struct swap_seq_verdict v = swap_seq_judge(steps, nsteps);

		printk("rpi5_scanout32: S8 sequence %s (%s) index %d swap %s, %u of %u swaps ok\n",
		       v.pass ? "ok" : "FAIL", swap_seq_reason_name(v.reason), v.index,
		       swap_reason_name(v.swap), swaps_ok, nsteps);
		if (!v.pass && fail == NULL) {
			snprintk(failbuf, sizeof(failbuf), "swap %d: %s (%s)", v.index + 1,
				 swap_seq_reason_name(v.reason), swap_reason_name(v.swap));
			fail = failbuf;
		}
		if (!named && fail == NULL) {
			fail = "channel 0 did not name our list after swap 1";
		}
	}

	/* S9: a sample of each buffer, from memory (clean+invalidate first). */
	if (bufs_ok) {
		struct pattern_report ra, rb;
		int ia = sys_cache_data_flush_and_invd_range(fb_a, (size_t)pitch * fw_h);
		int ib = sys_cache_data_flush_and_invd_range(fb_b, (size_t)pitch * fw_h);
		int ma = pattern_check(fb_a, fw_w, fw_h, pitch, 32, PATTERN_XRGB8888, CHECK_STEP,
				       &ra);
		int mb = pattern_check(fb_b, fw_w, fw_h, pitch, 32, PATTERN_XBGR8888, CHECK_STEP,
				       &rb);

		readback_ok = ma == 0 && mb == 0;
		printk("rpi5_scanout32: S9 readback A %s %u mismatches of %u (invd %d, sum 0x%08x)"
		       ", B %s %u mismatches of %u (invd %d, sum 0x%08x)\n",
		       ma == 0 ? "ok" : "FAIL", ra.mismatches, ra.checked, ia, ra.checksum,
		       mb == 0 ? "ok" : "FAIL", rb.mismatches, rb.checked, ib, rb.checksum);
		if (!readback_ok && fail == NULL) {
			fail = "a buffer did not read back its pattern";
		}
	} else {
		printk("rpi5_scanout32: S9 readback skip (buffers not ready)\n");
	}

	/* S10: the restore, whenever the snapshot was taken. When nothing was
	 * written (refused or skipped), no register or word is written here
	 * either: the compare alone runs. */
	hvs6s_report_init(&rrest);
	if (snap_err == HVS6S_OK) {
		uint32_t rb_kept[HVS6S_KEPT], rb_ptr, rb_n;
		struct swap_restore_verdict vk, vf;
		enum hvs6s_err e = HVS6S_OK;
		bool cleared = true;

		if (wrote) {
			e = hvs6s_restore(&io, &cfg, &fwsnap, &rrest);
		} else {
			printk("rpi5_scanout32: S10 restore write skip: nothing was written; "
			       "read-only compare\n");
		}
		rb_ptr = hvs_rd(hvs6s_lptrs_reg(CHAN));
		for (i = 0; i < HVS6S_KEPT; i++) {
			rb_kept[i] = hvs_rd(hvs6s_word_off(i));
		}
		rb_n = snap_dlist(ext_rb, fwsnap.fw_head, ext_n);
		vk = swap_restore_judge(fwsnap.kept, rb_kept, HVS6S_KEPT, fwsnap.lptrs, rb_ptr);
		vf = swap_restore_judge(ext, ext_rb, rb_n == ext_n ? ext_n : 0u, fwsnap.lptrs,
					rb_ptr);
		if (rb_n != ext_n) {
			vf.pass = false;
		}

		/* Our span back to what it read before the first write (zero), once
		 * channel 0 is back on the firmware's list and frames have passed. */
		if (wrote && placed) {
			bool back = rrest.restore_lptrs_ok && rrest.restore_frcnt_ok &&
				    !hvs6s_overlap(pair, PAIR_WORDS,
						   rb_ptr & YD5_HVS6_LPTRS_HEADE_MASK, 1);
			int bad = -1;

			if (back) {
				for (i = 0; i < PAIR_WORDS; i++) {
					io_write32(NULL, hvs6s_word_off(pair + i), pair_prev[i]);
				}
				barrier_dsync_fence_full();
				for (i = 0; i < PAIR_WORDS && bad < 0; i++) {
					if (hvs_rd(hvs6s_word_off(pair + i)) != pair_prev[i]) {
						bad = (int)i;
					}
				}
			}
			cleared = back && bad < 0;
			printk("rpi5_scanout32: S10 our words %u..%u %s\n", pair,
			       pair + PAIR_WORDS - 1u,
			       !back ? "left in place: channel 0 is not back on the firmware's list"
			       : bad < 0 ? "back to zero" : "NOT back to zero");
		}

		restore_pass = e == HVS6S_OK && vk.pass && vf.pass && cleared;
		printk("rpi5_scanout32: S10 dlswap restore %s (%s); words 0..31 %s index %d want "
		       "0x%08x got 0x%08x; fw %u..+%u %s index %d want 0x%08x got 0x%08x\n",
		       !wrote ? "skip" : e == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(e),
		       swap_restore_reason_name(vk.reason), vk.index, vk.want, vk.got,
		       fwsnap.fw_head, ext_n, swap_restore_reason_name(vf.reason),
		       vf.index, vf.want, vf.got);
		if (restore_pass) {
			printk("u3 restore: PASS byte-equal list-ptr=0x%08x words=%u+%u "
			       "hash=0x%08x%s\n",
			       rb_ptr, HVS6S_KEPT, ext_n, words_hash(ext_rb, ext_n),
			       wrote ? "" : " (nothing was written)");
		} else {
			printk("u3 restore: FAIL %s list-ptr=0x%08x want 0x%08x (%s / %s%s) "
			       "hash=0x%08x\n",
			       hvs6s_err_name(e), rb_ptr, fwsnap.lptrs,
			       swap_restore_reason_name(vk.reason),
			       swap_restore_reason_name(vf.reason),
			       cleared ? "" : " / our words not cleared", words_hash(ext_rb, rb_n));
		}
		if (e != HVS6S_OK && first_err == HVS6S_OK) {
			first_err = e;
		}
		if (!restore_pass && fail == NULL) {
			fail = "the firmware's list was not restored byte-equal";
		}
	} else {
		printk("rpi5_scanout32: S10 dlswap restore skip (no snapshot; nothing was written)\n");
		printk("u3 restore: FAIL not run (no snapshot; nothing was written)\n");
	}

	/* The dlswap record: placement, swap 1 and the restore, one line per step. */
	if (snap_err == HVS6S_OK) {
		rec.place_err = place_err;
		if (placed) {
			rec.start = pair;
			rec.len = 2u * SLOT_WORDS;
		}
		rec.restored = rrest.restored;
		rec.restore_frcnt_ok = rrest.restore_frcnt_ok;
		rec.restore_polls = rrest.restore_polls;
		rec.at_restore = rrest.at_restore;
		rec.after_restore = rrest.after_restore;
		rec.restore_lptrs_ok = rrest.restore_lptrs_ok;
		rec.restore_equal = rrest.restore_equal;
		rec.diff_word = rrest.diff_word;
		rec.diff_want = rrest.diff_want;
		rec.diff_got = rrest.diff_got;
		rec.err = first_err;
		(void)hvs6s_lines(&fwsnap, &rec, &emit);
	} else {
		printk("dlswap snapshot FAIL lptrs=0x%08x (%s)\n", lptrs, hvs6s_err_name(snap_err));
		printk("dlswap result FAIL %s\n", hvs6s_err_name(snap_err));
	}

	if (fail == NULL && first_err != HVS6S_OK) {
		fail = hvs6s_err_name(first_err);
	}
	if (fail == NULL) {
		printk("rpi5_scanout32: PASS (%u swaps alternating A/B, list at word %u, FRCNT "
		       "advanced, buffers read back, firmware list restored byte-equal)\n",
		       nsteps, pair);
	} else {
		printk("rpi5_scanout32: FAIL (%s)\n", fail);
	}

	finished = true;
	printk("rpi5_scanout32: feeding stopped, reset expected (the PM watchdog resets the SoC, "
	       "and tryboot returns the board to Linux)\n");
	for (;;) {
		k_sleep(K_FOREVER);
	}
	return 0;
}
