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
 * time (on the board it was word 672), never assumed to be words 0..31.
 * hvs6_dlist_swap.h (dlplace) finds it, walks it entry by entry to its END
 * and refuses a list with no END within HVS6S_FW_MAX words; it places our
 * list only in words that read zero, clear of words 0..31 and of the
 * firmware's span, within the list memory (mem_words, from CXM_SIZE).
 *
 *   S1  the HVS VERSION word and the stepping it names, CXM_SIZE
 *   S2  channel 0's CTRL0, LPTRS, STATUS (FRCNT) and DL words, decoded, and
 *       STATUS read several times to show FRCNT advancing
 *   S3  the firmware's display list from LPTRS.HEADE: every word to its END,
 *       or the first DUMP_MAX words when no END is in view
 *   S4  pixel valve 0 (HDMI0's) PV_CONTROL and PV_V_CONTROL, read only
 *   S5  the plan's snapshot (hvs6s_snapshot, read only): LPTRS and the
 *       firmware's list [HEADE, HEADE + fw_len), hashed   -> u3 snapshot
 *   S6  the plan's placement (hvs6s_place_io, read only): one span for slot
 *       A and slot B, 10 words each (hvs6_dlist_write.h's entry), every word
 *       read zero, clear of words 0..31 and the firmware's span; then no
 *       channel's LPTRS.HEADE or DL.LACT inside it            -> u3 dlist
 *   S7  buffers A and B filled (pattern.h, A as XRGB8888, B with red and
 *       blue exchanged) and cleaned to memory          -> u3 plane
 *   S8  swap 1 is hvs6s_run with cfg.start = slot A: its own snapshot, the
 *       placement checked again, the swap to buffer A and its restore, with
 *       the dlswap record of that run. Then slot B checked (hvs6s_place_io)
 *       and swaps 2..SWAPS, B, A, B ... (hvs6s_swap). Every swap judged by
 *       swap_check.h                          -> u3 swap <k>, u3 named
 *   S9  a sample of each buffer read back after a clean+invalidate
 *   S10 the restore (hvs6s_restore) after swaps 2..SWAPS: the firmware's
 *       LPTRS written back, two frames; then LPTRS and the firmware's words
 *       compared with the snapshot through swap_check.h's swap_restore_judge,
 *       and our span written back to zero once channel 0 is off it
 *                                                        -> u3 restore
 *   then the end line.
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
#define SNAP_MAX  HVS6S_FW_MAX   /* S3: words read from HEADE */
#define DUMP_MAX  64u            /* S3: words printed when no END is in view */

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

/* plansnap: the read-only plan's snapshot (S5/S6), before anything is
 * written. fwsnap: hvs6s_run's own (S8), the one every restore compares
 * against. */
static struct hvs6s_snap plansnap, fwsnap;
static struct swap_step steps[SWAPS];
static uint32_t fw_rb[HVS6S_FW_MAX];

/* The firmware's span in words for swap_check.h: the walked length to the
 * END, or, when the walk found none, the extent it read from HEADE. */
static uint32_t fw_words_of(const struct hvs6s_snap *s)
{
	if (s->fw_end_found) {
		return s->fw_len;
	}
	return s->walk_stop >= s->fw_head ? s->walk_stop - s->fw_head + 1u : 0u;
}

static bool snaps_equal(const struct hvs6s_snap *a, const struct hvs6s_snap *b)
{
	if (!a->taken || !b->taken || a->lptrs != b->lptrs || a->fw_head != b->fw_head ||
	    a->fw_len != b->fw_len) {
		return false;
	}
	for (uint32_t i = 0; i < a->fw_len; i++) {
		if (a->fw[i] != b->fw[i]) {
			return false;
		}
	}
	return true;
}

/* One swap's sample for swap_check.h, all ten fields. The registers are the
 * header's samples when LPTRS was written (at the write, and once read-back
 * and FRCNT were in), else the probe's own reads around the call. */
static void fill_step(struct swap_step *st, const struct hvs6s_report *r, uint32_t pre_lptrs,
		      uint32_t pre_status, uint32_t post_lptrs, uint32_t post_status,
		      uint32_t slot_word, uint64_t buf_bus, const struct hvs6s_snap *s)
{
	int prc;

	st->sample = (struct swap_sample){
		.ptr_before = r->swapped ? r->at_write.lptrs : pre_lptrs,
		.ptr_after = r->swapped ? r->after.lptrs : post_lptrs,
		.expected_word = slot_word,
		.status_before = r->swapped ? r->at_write.status : pre_status,
		.status_after = r->swapped ? r->after.status : post_status,
		.pv_control = pv_rd(SA_PV_CONTROL),
		.pv_vcontrol = pv_rd(SA_PV_V_CONTROL),
		.fw_start = s->fw_head & SWAP_LPTRS_MASK,
		.fw_words = fw_words_of(s),
		.our_words = SLOT_WORDS,
	};
	st->plane_addr = slot_plane_addr(slot_word, &prc);
	st->expected_addr = buf_bus;
	if (prc != HVS6_OK) {
		printk("rpi5_scanout32: S8 slot word %u: plane entry did not decode (hvs6 error "
		       "%d)\n", slot_word, prc);
	}
}

/* The dlswap summary and u3 swap lines of swap k (and u3 named after swap
 * 1); returns the judge's reason. */
static enum swap_reason swap_lines(unsigned int k, const struct swap_step *st,
				   const struct hvs6s_report *r, enum hvs6s_err e, bool *named)
{
	enum swap_reason why = swap_judge(&st->sample);
	uint32_t slot_word = st->sample.expected_word;

	printk("dlswap swap%u %s %s lptrs-readback %s frcnt-advance %s settle %s "
	       "lact-follows %s (record only)\n",
	       k + 1u, e == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(e),
	       r->lptrs_ok ? "ok" : "FAIL", r->frcnt_ok ? "ok" : "FAIL",
	       r->settle_ok ? "ok" : (r->frcnt_ok ? "FAIL" : "skip"),
	       r->lact_follows ? "yes" : "no");
	printk("u3 swap %u: buf %c list-ptr=0x%08x frcnt %u -> %u judge=%s slot=%u "
	       "plane=0x%010llx fw=%u+%u ours=%u\n",
	       k + 1u, (k & 1u) ? 'B' : 'A', st->sample.ptr_after,
	       swap_frcnt(st->sample.status_before), swap_frcnt(st->sample.status_after),
	       swap_reason_name(why), slot_word, (unsigned long long)st->plane_addr,
	       st->sample.fw_start, st->sample.fw_words, st->sample.our_words);
	if (k == 0) {
		*named = st->sample.ptr_after != ALL_ONES &&
			 (st->sample.ptr_after & SWAP_LPTRS_MASK) == slot_word;
		printk("u3 named: %s LPTRS 0x%08x after swap 1, our list at word %u, "
		       "first read 0x%08x poll %u\n",
		       *named ? "yes" : "no", st->sample.ptr_after, slot_word, r->lptrs_first,
		       r->lptrs_poll);
	}
	return why;
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
	struct hvs6s_report plan, runrep, rrest;
	enum hvs6s_err snap_err = HVS6S_ERR_ARG, place_err = HVS6S_ERR_ARG, first_err = HVS6S_OK;
	enum hvs6s_err run_err = HVS6S_ERR_ARG;
	uint32_t pair = 0, slot[2] = {0, 0};
	uint32_t fw_w = FB_MAX_W, fw_h = FB_MAX_H, pitch;
	uint64_t phys[2] = {0, 0}, bus[2] = {0, 0};
	bool bufs_ok = false, placed = false, named = false, readback_ok = false;
	bool restore_pass = false, ran = false, wrote_list = false, wrote_more = false;
	bool same_snap = false;
	unsigned int nsteps = 0, swaps_ok = 0;
	const char *fail = NULL;
	char failbuf[192];
	char why[96];

	printk("rpi5_scanout32: build %s\n", RPI5_SCANOUT32_BUILD);
	rpi_wdog_announce();
	printk("rpi5_scanout32: stage 2: our XRGB8888 list in zero words at or after word 32, "
	       "clear of the firmware's list at LPTRS.HEADE (read and checked first), %u swaps A/B on "
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

	/* One line per word: from HEADE to the END, or the first DUMP_MAX
	 * words when no END is in view. */
	for (i = 0; i < got; i++) {
		if (saw_end ? i > end_idx : i >= DUMP_MAX) {
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

	/* S5: the plan's snapshot, read only. hvs6_dlist_swap.h's offsets are
	 * D0's; the list memory's size is CXM_SIZE (in words), capped by the
	 * block mapped here. */
	cfg.chan = CHAN;
	cfg.mem_words = cxm == ALL_ONES ? 0u : MIN(cxm, DL_WORDS);
	cfg.start = HVS6S_AUTO;
	cfg.list_arm = HVS_BASE + HVS6_DLIST_OFFSET;
	cfg.cache_line = (uint32_t)sys_cache_data_line_size_get();
	cfg.max_polls = MAX_POLLS;
	hvs6s_report_init(&plan);
	if (hvs == 0) {
		printk("rpi5_scanout32: S5 dlswap snapshot FAIL: HVS not mapped\n");
	} else if (gen != HVS6_GEN_D) {
		printk("rpi5_scanout32: S5 dlswap snapshot FAIL: stepping is not D0 (the swap's "
		       "offsets are D0's)\n");
	} else {
		snap_err = hvs6s_snapshot(&io, &cfg, &plansnap);
		printk("rpi5_scanout32: S5 dlswap snapshot %s (%s) mem_words %u (CXM_SIZE 0x%08x) "
		       "lptrs 0x%08x fw_head %u fw_len %u entries %u, walk stopped at word %u "
		       "(0x%08x), END %s\n",
		       snap_err == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(snap_err),
		       cfg.mem_words, cxm, plansnap.lptrs, plansnap.fw_head, plansnap.fw_len,
		       plansnap.fw_entries, plansnap.walk_stop, plansnap.walk_word,
		       plansnap.fw_end_found ? "found" : "NOT found");
	}
	if (snap_err == HVS6S_OK) {
		/* The hash is over the firmware's words [HEADE, HEADE + fw_len),
		 * END included: the list the channel runs, as the restore compares. */
		printk("u3 snapshot: list-ptr=0x%08x heade=%u words=%u hash=0x%08x\n",
		       plansnap.lptrs, plansnap.fw_head, plansnap.fw_len,
		       words_hash(plansnap.fw, plansnap.fw_len));
	} else {
		uint32_t sl = plansnap.lptrs != 0 ? plansnap.lptrs : lptrs;

		hvs6s_fail(&plan, snap_err);
		printk("u3 snapshot: list-ptr=0x%08x heade=%u words=0 hash=0x00000000 FAIL %s "
		       "(walk stopped at word %u, 0x%08x, after %u entries)\n",
		       sl, sl & YD5_HVS6_LPTRS_HEADE_MASK, hvs6s_err_name(snap_err),
		       plansnap.walk_stop, plansnap.walk_word, plansnap.fw_entries);
		first_err = snap_err;
		fail = "no snapshot of the firmware's list";
	}

	/* S6: the plan's placement, read only: one span for slot A and slot B,
	 * every word zero, clear of words 0..31 and the firmware's span
	 * (hvs6s_place_io); then no channel's list pointer inside it. */
	for (unsigned int c = 0; c < YD5_HVS6_NUM_CHANNELS; c++) {
		printk("rpi5_scanout32: S6 ch%u LPTRS 0x%08x DL 0x%08x\n", c,
		       hvs_rd(hvs6s_lptrs_reg(c)), hvs_rd(hvs6s_dl_reg(c)));
	}
	if (snap_err == HVS6S_OK) {
		place_err = hvs6s_place_io(&io, &cfg, &plansnap, PAIR_WORDS, &pair, &plan);
		if (place_err == HVS6S_OK) {
			for (unsigned int c = 0; c < YD5_HVS6_NUM_CHANNELS; c++) {
				uint32_t lp = hvs_rd(hvs6s_lptrs_reg(c)) & YD5_HVS6_LPTRS_HEADE_MASK;
				uint32_t la = hvs_rd(hvs6s_dl_reg(c)) & HVS6S_DL_LACT_MASK;
				bool in_lp = hvs6s_overlap(pair, PAIR_WORDS, lp, 1);

				if (in_lp || hvs6s_overlap(pair, PAIR_WORDS, la, 1)) {
					place_err = HVS6S_ERR_PLACE_FW;
					snprintk(why, sizeof(why),
						 "channel %u's %s %u is inside words %u..%u", c,
						 in_lp ? "LPTRS.HEADE" : "DL.LACT", in_lp ? lp : la,
						 pair, pair + PAIR_WORDS - 1u);
					break;
				}
			}
		} else if (plan.place_bad_word >= 0) {
			snprintk(why, sizeof(why), "%s: word %lld reads 0x%08x",
				 hvs6s_err_name(place_err), (long long)plan.place_bad_word,
				 plan.place_bad_val);
		} else {
			snprintk(why, sizeof(why), "%s (%u non-zero words stepped past)",
				 hvs6s_err_name(place_err), plan.place_skipped);
		}
		plan.place_err = place_err;
		placed = place_err == HVS6S_OK;
	} else {
		snprintk(why, sizeof(why), "no snapshot: %s", hvs6s_err_name(snap_err));
	}
	if (placed) {
		slot[0] = pair;
		slot[1] = pair + SLOT_WORDS;
		plan.start = pair;
		plan.len = PAIR_WORDS;
		printk("rpi5_scanout32: S6 placement check ok: words %u..%u read zero, clear of "
		       "words 0..%u, the firmware's list %u..%u (%u entries, END at word %u) and "
		       "every channel's list pointer; %u non-zero word(s) stepped past\n",
		       pair, pair + PAIR_WORDS - 1u, HVS6S_KEPT - 1u, plansnap.fw_head,
		       plansnap.fw_head + plansnap.fw_len - 1u, plansnap.fw_entries,
		       plansnap.walk_stop, plan.place_skipped);
		printk("rpi5_scanout32: S6 dlswap place ok: slot A word %u, slot B word %u, %u "
		       "words each\n", slot[0], slot[1], SLOT_WORDS);
		printk("u3 dlist: placed word %u len %u (slot A %u, slot B %u)\n", pair, PAIR_WORDS,
		       slot[0], slot[1]);
	} else {
		printk("rpi5_scanout32: S6 placement check %s: %s; nothing is written\n",
		       snap_err == HVS6S_OK ? "FAIL" : "skip", why);
		printk("u3 dlist: refused %s\n", why);
		if (snap_err == HVS6S_OK) {
			hvs6s_fail(&plan, place_err);
		}
		if (first_err == HVS6S_OK) {
			first_err = place_err;
			fail = "our list was refused: its words are not known free";
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

	/* S8: swap 1 is hvs6s_run at slot A (its own snapshot, the placement
	 * checked again, the swap to buffer A and the restore); then slot B is
	 * checked the same way and swaps 2.. alternate B, A, B ... */
	hvs6s_report_init(&runrep);
	hvs6s_report_init(&rrest);
	cfg.plane.format = HVS6W_XRGB8888;
	cfg.plane.x = 0;
	cfg.plane.y = 0;
	cfg.plane.width = fw_w;
	cfg.plane.height = fw_h;
	cfg.plane.pitch = pitch;
	cfg.plane.addr = bus[0];
	if (placed && bufs_ok) {
		uint32_t pre_status = hvs_rd(hvs6s_status_reg(CHAN));
		uint32_t pre_lptrs = hvs_rd(hvs6s_lptrs_reg(CHAN));
		enum swap_reason w;

		cfg.start = slot[0];
		ran = true;
		run_err = hvs6s_run(&io, &cfg, &fwsnap, &runrep);
		/* our list's words are written once the run's placement passed */
		wrote_list = fwsnap.taken && runrep.place_err == HVS6S_OK;
		same_snap = snaps_equal(&plansnap, &fwsnap);
		printk("rpi5_scanout32: S8 hvs6s_run %s (%s) at slot A word %u: swapped %d "
		       "restored %d; its snapshot %s the plan's\n",
		       run_err == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(run_err), slot[0],
		       runrep.swapped, runrep.restored, same_snap ? "equals" : "DIFFERS from");
		(void)hvs6s_lines(&fwsnap, &runrep, &emit);
		fill_step(&steps[0], &runrep, pre_lptrs, pre_status,
			  hvs_rd(hvs6s_lptrs_reg(CHAN)), hvs_rd(hvs6s_status_reg(CHAN)), slot[0],
			  bus[0], &fwsnap);
		nsteps = 1;
		w = swap_lines(0, &steps[0], &runrep, run_err, &named);
		if (run_err != HVS6S_OK && first_err == HVS6S_OK) {
			first_err = run_err;
		}
		if (run_err == HVS6S_OK && w == SWAP_OURS) {
			swaps_ok++;
		}
		if (!same_snap && fail == NULL) {
			fail = "the firmware's list changed between the plan and the run";
		}
		k_msleep(HOLD_MS);
	}

	/* Slot B, read and checked before its first write, as slot A was. */
	{
		bool go = ran && runrep.swapped && runrep.restore_equal && same_snap;

		if (go) {
			struct hvs6s_report bplace;
			uint32_t sb = 0;
			enum hvs6s_err be;

			hvs6s_report_init(&bplace);
			cfg.start = slot[1];
			be = hvs6s_place_io(&io, &cfg, &fwsnap, SLOT_WORDS, &sb, &bplace);
			if (be == HVS6S_OK && sb != slot[1]) {
				be = HVS6S_ERR_PLACE_FIT;
			}
			if (be == HVS6S_OK) {
				printk("rpi5_scanout32: S8 slot B place ok: word %u..%u read zero\n",
				       slot[1], slot[1] + SLOT_WORDS - 1u);
			} else {
				go = false;
				if (bplace.place_bad_word >= 0) {
					snprintk(why, sizeof(why), "slot B refused: %s: word %lld reads "
						 "0x%08x", hvs6s_err_name(be),
						 (long long)bplace.place_bad_word, bplace.place_bad_val);
				} else {
					snprintk(why, sizeof(why), "slot B refused: %s",
						 hvs6s_err_name(be));
				}
				printk("rpi5_scanout32: S8 slot B place FAIL: %s; nothing more is "
				       "written\n", why);
				if (first_err == HVS6S_OK) {
					first_err = be;
					fail = "slot B was refused: its words are not known free";
				}
			}
		} else if (ran) {
			snprintk(why, sizeof(why), "swap 1: %s",
				 !runrep.swapped ? "LPTRS was not written"
				 : !runrep.restore_equal ? "the run's restore was not byte-equal"
				 : "the run's snapshot differed from the plan's");
		} else {
			snprintk(why, sizeof(why), "%s", !placed ? "not placed" : "buffers not ready");
		}

		for (unsigned int k = ran ? 1u : 0u; k < SWAPS; k++) {
			unsigned int b = k & 1u;
			struct hvs6s_report r;
			enum hvs6s_err e;
			enum swap_reason w;
			uint32_t pre_status, pre_lptrs;

			if (!go) {
				printk("u3 swap %u: skipped (%s)\n", k + 1u, why);
				continue;
			}
			pre_status = hvs_rd(hvs6s_status_reg(CHAN));
			pre_lptrs = hvs_rd(hvs6s_lptrs_reg(CHAN));
			cfg.plane.addr = bus[b];
			hvs6s_report_init(&r);
			wrote_more = true; /* from here on S10 writes LPTRS back */
			e = hvs6s_swap(&io, &cfg, slot[b], &r);
			hvs6s_regs_line(&emit, "at-write", &r.at_write);
			hvs6s_regs_line(&emit, "after", &r.after);
			fill_step(&steps[k], &r, pre_lptrs, pre_status, hvs_rd(hvs6s_lptrs_reg(CHAN)),
				  hvs_rd(hvs6s_status_reg(CHAN)), slot[b], bus[b], &fwsnap);
			nsteps = k + 1u;
			w = swap_lines(k, &steps[k], &r, e, &named);
			if (e != HVS6S_OK && first_err == HVS6S_OK) {
				first_err = e;
			}
			if (e == HVS6S_OK && w == SWAP_OURS) {
				swaps_ok++;
			}
			k_msleep(HOLD_MS);
		}
	}
	if (!ran) {
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
		if (nsteps < SWAPS && fail == NULL) {
			snprintk(failbuf, sizeof(failbuf), "%u of %u swaps made (%s)", nsteps, SWAPS,
				 why);
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

	/* S10: the restore after swaps 2.., and the compare whenever a snapshot
	 * was taken. Swap 1's restore is hvs6s_run's own. When nothing more
	 * was written, no register or word is written here: the compare alone
	 * runs (and our words go back to zero). */
	{
		const struct hvs6s_snap *ref = fwsnap.taken ? &fwsnap : &plansnap;

		if (ref->taken) {
			enum hvs6s_err e = HVS6S_OK;
			struct swap_restore_verdict vf;
			uint32_t rb_ptr, rb_lact;
			bool cleared = true, settled;

			if (wrote_more) {
				e = hvs6s_restore(&io, &cfg, &fwsnap, &rrest);
				hvs6s_regs_line(&emit, "restore-after", &rrest.after_restore);
				printk("dlswap restore-frcnt %s polls=%u\n",
				       rrest.restore_frcnt_ok ? "ok" : "FAIL", rrest.restore_polls);
				settled = rrest.restore_frcnt_ok;
			} else if (ran) {
				printk("rpi5_scanout32: S10 restore write skip: no swap after swap 1; "
				       "hvs6s_run's own restore %s; compare only\n",
				       runrep.restored ? "wrote the firmware's LPTRS back"
				       : "did not run");
				settled = runrep.restore_frcnt_ok;
			} else {
				printk("rpi5_scanout32: S10 restore write skip: nothing was written; "
				       "read-only compare\n");
				settled = true;
			}
			rb_ptr = hvs_rd(hvs6s_lptrs_reg(CHAN));
			rb_lact = hvs_rd(hvs6s_dl_reg(CHAN)) & HVS6S_DL_LACT_MASK;
			for (i = 0; i < ref->fw_len; i++) {
				fw_rb[i] = hvs_rd(hvs6s_word_off(ref->fw_head + i));
			}
			vf = swap_restore_judge(ref->fw, fw_rb, ref->fw_len, ref->lptrs, rb_ptr);

			/* Our span back to zero (it read zero before the first write),
			 * once channel 0 is back on the firmware's list, frames have
			 * passed and neither its LPTRS nor DL.LACT is inside it. */
			if (wrote_list) {
				bool back = settled &&
					    (rb_ptr & YD5_HVS6_LPTRS_HEADE_MASK) == ref->fw_head &&
					    !hvs6s_overlap(pair, PAIR_WORDS,
							   rb_ptr & YD5_HVS6_LPTRS_HEADE_MASK, 1) &&
					    !hvs6s_overlap(pair, PAIR_WORDS, rb_lact, 1);
				int bad = -1;

				if (back) {
					for (i = 0; i < PAIR_WORDS; i++) {
						io_write32(NULL, hvs6s_word_off(pair + i), 0u);
					}
					barrier_dsync_fence_full();
					for (i = 0; i < PAIR_WORDS && bad < 0; i++) {
						if (hvs_rd(hvs6s_word_off(pair + i)) != 0u) {
							bad = (int)i;
						}
					}
				}
				cleared = back && bad < 0;
				printk("rpi5_scanout32: S10 our words %u..%u %s\n", pair,
				       pair + PAIR_WORDS - 1u,
				       !back ? "left in place: channel 0 is not back on the firmware's "
					       "list"
				       : bad < 0 ? "back to zero" : "NOT back to zero");
			}

			restore_pass = e == HVS6S_OK && vf.pass && cleared;
			printk("rpi5_scanout32: S10 dlswap restore %s (%s); fw %u..+%u %s index %d "
			       "want 0x%08x got 0x%08x\n",
			       !wrote_more ? "skip" : e == HVS6S_OK ? "ok" : "FAIL",
			       hvs6s_err_name(e), ref->fw_head, ref->fw_len,
			       swap_restore_reason_name(vf.reason), vf.index, vf.want, vf.got);
			if (restore_pass) {
				printk("u3 restore: PASS byte-equal list-ptr=0x%08x heade=%u words=%u "
				       "hash=0x%08x%s\n",
				       rb_ptr, ref->fw_head, ref->fw_len,
				       words_hash(fw_rb, ref->fw_len),
				       wrote_list ? "" : " (nothing was written)");
			} else {
				printk("u3 restore: FAIL %s list-ptr=0x%08x want 0x%08x (%s%s) "
				       "hash=0x%08x\n",
				       hvs6s_err_name(e), rb_ptr, ref->lptrs,
				       swap_restore_reason_name(vf.reason),
				       cleared ? "" : " / our words not cleared",
				       words_hash(fw_rb, ref->fw_len));
			}
			if (e != HVS6S_OK && first_err == HVS6S_OK) {
				first_err = e;
			}
			if (!restore_pass && fail == NULL) {
				fail = "the firmware's list was not restored byte-equal";
			}
		} else {
			printk("rpi5_scanout32: S10 dlswap restore skip (no snapshot; nothing was "
			       "written)\n");
			printk("u3 restore: FAIL not run (no snapshot; nothing was written)\n");
		}
	}

	/* The dlswap record of the plan when hvs6s_run did not run (it printed
	 * its own record after swap 1): snapshot, placement, nothing written. */
	if (!ran) {
		hvs6s_fail(&plan, first_err);
		(void)hvs6s_lines(&plansnap, &plan, &emit);
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
