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
 * hvs6_dlist_swap.h's hvs6s_snapshot walks it entry by entry (VALID-clear
 * entries included) over the whole list memory past HEADE (cfg.walk_words 0,
 * scanout32-fold) to its END; the snapshot, the S5 hash and every restore
 * compare cover that whole walked list, END included (on the board, from
 * rpi5_dlmap: HEADE 672, END at word 928, 257 words).
 *
 * Placement (selftests-s32b): the board's unused list memory holds a
 * 0xb0b0b0b0 fill, not zeros, and word 0 is a bare END that channel 1 runs
 * while enabled. The library's placement wants all-zero words, so the probe
 * places itself (swap_check.h's swap_place_after_end): only with an END
 * found, from the word after it, at or above word 32, inside the list memory
 * and the 12-bit LPTRS field, clear of the other channels' lists (each walked
 * to its END by hvs6s_snapshot; a channel not walked to an END refuses) and
 * of their LPTRS.HEADE and DL.LACT. No word value decides it: neither zeros
 * nor the fill are taken as free. The words of our pair are saved at S6,
 * checked unchanged before each slot's first write, and written back at
 * S10. No END in the walk: placement refused, nothing written.
 *
 *   S1  the HVS VERSION word and the stepping it names, CXM_SIZE
 *   S2  channel 0's CTRL0, LPTRS, STATUS (FRCNT) and DL words, decoded, and
 *       STATUS read several times to show FRCNT advancing
 *   S3  the firmware's display list from LPTRS.HEADE: every word to its END,
 *       or the first DUMP_MAX words when no END is in view
 *   S4  pixel valve 0 (HDMI0's) PV_CONTROL and PV_V_CONTROL, read only
 *   S5  the plan's snapshot (hvs6s_snapshot, read only): LPTRS and the
 *       firmware's list [HEADE, HEADE + fw_len), hashed   -> u3 snapshot
 *   S6  the plan's placement (read only): one span for slot A and slot B,
 *       10 words each (hvs6_dlist_write.h's entry), right after the END
 *       (above); then no channel's LPTRS.HEADE or DL.LACT inside it; the
 *       pair's words saved                        -> u3 walk, u3 dlist
 *   S7  buffers A and B filled (pattern.h, A as XRGB8888, B with red and
 *       blue exchanged) and cleaned to memory          -> u3 plane
 *   S8  swap 1 is run_swap1 at slot A (hvs6s_run's sequence): its own
 *       snapshot, the placement checked again and the pair's words unchanged,
 *       the swap to buffer A and its restore, with the dlswap record of that
 *       run. Then slot B checked the same way
 *       and swaps 2..SWAPS, B, A, B ... (hvs6s_swap), only while swap 1's
 *       DL.LACT picked our list up and came back to the firmware's head, and
 *       up to the first failed swap. Every swap judged by swap_check.h,
 *       DL.LACT pickup included          -> u3 swap <k>, u3 named
 *   S9  a sample of each buffer read back after a clean+invalidate
 *   S10 the restore (hvs6s_restore) after swaps 2..SWAPS: the firmware's
 *       LPTRS written back, two frames and DL.LACT back on the firmware's
 *       head; then LPTRS and the firmware's words compared with the snapshot
 *       through swap_check.h's swap_restore_judge, and our span written back
 *       to the words S6 saved once channel 0 is off it and DL.LACT is back
 *                                                        -> u3 restore
 *   S11 the main stack's size and its never-touched bytes (s32-nullfix,
 *       swap_check.h (h)); within the margin the end line is FAIL
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
 *
 * DL.LACT pickup (lact-probe-a, after swap-lact-lib): a swap counts only once
 * channel 0's DL.LACT read the head just written, and a restore only once
 * DL.LACT read the firmware's LPTRS.HEADE again (hvs6_dlist_swap.h's
 * lact_pickup and restore_lact_ok). Each swap prints
 *   dlswap lact-pickup ok|FAIL polls=N frcnt=A..B want=W lact=L
 * (swap 1's inside run_swap1's record, swaps 2.. after their dlswap swap<k>
 * line: the k-th such line of the log is swap k's), each restore
 *   dlswap restore-lact ok|FAIL polls=N frcnt=A..B want=W lact=L
 * (swap 1's restore inside the record, the final one after restore-frcnt).
 * 'u3 named: yes' needs swap 1's pickup; 'u3 restore: PASS' needs the
 * restore's DL.LACT back (the final restore's, or swap 1's own when only
 * swap 1 ran) and every swap made picked up; our list words are cleared only
 * once the restore's DL.LACT is back. verdicts.sh judges q4 by the same lines.
 *
 * Hardware-written words (swap-hwword): RPI5_SCANOUT32_HW_WORDS, the CMake
 * knob of the same name ("" by default, e.g. "4:skip" or "4:skip,7:expect"),
 * is parsed by hvs6s_hw_parse into cfg.hw (and swap_check.h's copy). Its
 * offsets, from the start of a plane entry, are left out of the hash and of
 * the byte-equal compares (SKIP) or compared and reported without failing
 * (EXPECT): in every entry of the firmware's span (S5's hash, the plan/run
 * snapshot compare, both restores) and in our slot (hvs6s_swap's read-back,
 * the slot read after each swap). Every differing word is printed as a
 * "dlswap restore-diff ..." or "dlswap readback-diff ..." line with its
 * count line; the u3 snapshot and u3 restore lines carry hwskip=<set>, and a
 * PASS with a non-empty set reads "byte-equal except hw words off <offs>".
 * A knob that does not parse refuses the run before anything is read or
 * written (the build refuses it first).
 *
 * Vblank-timed swap (swap-lact-probe, on swap-vblank-lib): every swap writes
 * LPTRS with cfg.write_at = HVS6S_AT_VBLANK, just after an FRCNT step of
 * channel 0's STATUS found by reads only (no pixel valve register is read
 * or written for it; pv_wait_vblank, which writes PV_INTSTAT, is not used),
 * and the DL.LACT pickup is timed by hvs6_dlist_swap.h's tight loop, with
 * io.now_ns from the cycle counter. Each swap prints the library's two
 * record lines
 *   dlswap vblank ok|FAIL mode=vblank reads=N ... invblank=yes|no
 *   dlswap lact-timing tight=ok|miss reads=K step=J ... boundary=yes|no
 * (swap 1's inside run_swap1's record, the others after their at-write and
 * after lines) and, after its 'u3 swap <k>' line, one summary line
 *   u3 vblank <k>: landed=yes|no why=<reason> frcnt <w> -> <p> steps=<n>
 *     invblank=yes|no tight=ok|miss pickup_reads=<n> pickup_us=<us|none>
 *     read_us=<us|none> boundary=yes|no since_us=<us|none> yline <a> -> <b>
 *     (yline record only)
 * judged by swap_check.h's swap_vblank_judge: the write in the vblank, the
 * pickup inside the tight loop, at most one FRCNT step from the write to
 * the pickup and, with the clock, within SWAP_VB_DT_US_MAX. boundary= and
 * YLINE are a record and decide nothing. The probe's own PASS and
 * verdicts.sh's 'q4: yes' need every swap landed. A swap that found no
 * FRCNT step does not write LPTRS (HVS6S_ERR_VBLANK) and stops the swaps.
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

#include "io_filter.h" /* needs HVS_SIZE and CHAN */

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
/* cfg.lact_frames: the HVS takes a list pointer at a frame start, so DL.LACT
 * should read the new head one frame after the write; a write just after a
 * frame edge and the 1 ms poll gap can add one more. Four frames leave margin
 * and stay well under MAX_POLLS' ~12 frames and FRCNT's 64-frame aliasing.
 * A pickup later than that is a FAIL, not a late pass. */
#define LACT_FRAMES 4u

/* s32-nullfix: main()'s record lines (hvs6s_lines -> printk) need more than
 * Zephyr's default 4096-byte main stack, which overflowed into the idle
 * thread's stack on the board (swap_check.h section (h)); prj.conf sets it. */
BUILD_ASSERT(CONFIG_MAIN_STACK_SIZE >= SWAP_MAIN_STACK_MIN,
	     "rpi5_scanout32: CONFIG_MAIN_STACK_SIZE below SWAP_MAIN_STACK_MIN (swap_check.h)");

/* The hardware-written words, from CMakeLists.txt (empty by default). */
#ifndef RPI5_SCANOUT32_HW_WORDS
#define RPI5_SCANOUT32_HW_WORDS ""
#endif

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

/* writes io_write32 refused (scanout32_write_ok); any is a FAIL */
static unsigned int refused_writes;

static void io_write32(void *ctx, uint32_t off, uint32_t val)
{
	ARG_UNUSED(ctx);
	if (!scanout32_write_ok(off)) {
		/* by construction: only list words >= HVS6S_KEPT and channel 0's LPTRS */
		printk("rpi5_scanout32: refused write off 0x%x\n", off);
		refused_writes++;
		return;
	}
	if (hvs != 0) {
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

/* The tight loop's clock (swap-vblank-lib): the system cycle counter in ns. */
static uint64_t io_now_ns(void *ctx)
{
	ARG_UNUSED(ctx);
	return k_cyc_to_ns_floor64(k_cycle_get_64());
}

static const struct hvs6s_io io = {
	.read32 = io_read32,
	.write32 = io_write32,
	.clean = io_clean,
	.barrier = io_barrier,
	.wait = io_wait,
	.ctx = NULL,
	.now_ns = io_now_ns,
};

static void emit_line(void *ctx, const char *text)
{
	ARG_UNUSED(ctx);
	printk("%s\n", text);
}

static const struct hvs6s_emit emit = { emit_line, NULL };

/* The hardware-written words: swap_check.h's copy of cfg.hw, and its text. */
static struct swap_hw_set shw;
static char hwtxt[48];
static char hwoffs[24];

static void to_swap_hw(const struct hvs6s_hw_set *h, struct swap_hw_set *o)
{
	*o = (struct swap_hw_set){ 0 };
	for (uint32_t i = 0; i < h->n && i < HVS6S_HW_MAX && i < SWAP_HW_MAX; i++) {
		o->w[i].off = h->w[i].off;
		o->w[i].mode = h->w[i].mode == HVS6S_HW_SKIP     ? SWAP_HW_SKIP
			       : h->w[i].mode == HVS6S_HW_EXPECT ? SWAP_HW_EXPECT
								 : SWAP_HW_NO;
		o->n = i + 1u;
	}
}

/* Every kept differing word as a "dlswap <kind>-diff ..." line, then the
 * count line; note is appended to each (the swap a read-back belongs to). */
static void print_diffs(const char *kind, const struct swap_diffs *d, const char *note)
{
	for (unsigned int i = 0; i < d->shown && i < SWAP_DIFF_MAX; i++) {
		const struct swap_diff *x = &d->d[i];
		char e[12], o[12];

		if (x->entry < 0) {
			snprintk(e, sizeof(e), "none");
			snprintk(o, sizeof(o), "none");
		} else {
			snprintk(e, sizeof(e), "%d", (int)x->entry);
			snprintk(o, sizeof(o), "%d", (int)x->off);
		}
		printk("dlswap %s-diff word %u entry %s off %s was 0x%08x now 0x%08x hw=%s%s\n", kind,
		       x->word, e, o, x->was, x->now, swap_hw_mode_name(x->hw), note);
	}
	printk("dlswap %s-diffs total=%u undeclared=%u skip=%u expect=%u shown=%u hwskip=%s%s\n",
	       kind, d->total, d->undeclared, d->skip, d->expect, d->shown, hwtxt, note);
}

/* Our slot read back after a swap: the words written (hvs6_dlist_write.h
 * for plane) against the words read from list memory, and the 40-bit address
 * the read entry names (0, and *rc an hvs6 error, when it does not decode).
 * Differing words are printed as readback-diff lines of swap k. */
static uint64_t slot_read(uint32_t start, const struct hvs6w_plane *plane, unsigned int k,
			  struct swap_slot *sl, int *rc)
{
	struct hvs6_plane pl;
	struct swap_diffs d;
	char note[24];

	*sl = (struct swap_slot){ 0 };
	sl->start = start;
	for (uint32_t i = 0; i < SLOT_WORDS; i++) {
		sl->got[i] = hvs_rd(HVS6_DLIST_OFFSET + (start + i) * 4u);
	}
	if (hvs6w_write_plane(sl->want, SWAP_SLOT_MAX, plane) == (int)SLOT_WORDS) {
		sl->n = SLOT_WORDS;
		(void)swap_diff_words(&shw, sl->want, sl->got, sl->n, start, &d, NULL);
		if (d.total != 0) {
			snprintk(note, sizeof(note), " (swap %u, after)", k + 1u);
			print_diffs("readback", &d, note);
		}
	}
	*rc = hvs6_dlist_first_plane(sl->got, SLOT_WORDS, 0, &pl);
	return *rc == HVS6_OK ? pl.addr : 0;
}

/* plansnap: the read-only plan's snapshot (S5/S6), before anything is
 * written. fwsnap: run_swap1's own (S8), the one every restore compares
 * against. */
static struct hvs6s_snap plansnap, fwsnap;
static struct swap_step steps[SWAPS];
static bool pickups[SWAPS]; /* DL.LACT reached swap k's head (report.lact_pickup) */
static struct swap_vblank vbs[SWAPS]; /* swap k's vblank record (swap_vblank_judge) */
static struct swap_slot slots[SWAPS];
static struct swap_diffs rdiffs;
static uint32_t fw_rb[HVS6S_FW_MAX];
/* another channel's list, walked for the placement (selftests-s32b) */
static struct hvs6s_snap chsnap;
/* the words of our pair as S6 read them, before any write: what S8 checks
 * is still there before each slot's first write, and what S10 writes back */
static uint32_t pair_saved[PAIR_WORDS];

/* The placement's input (swap_check.h's swap_place_after_end) from snapshot
 * s of channel CHAN's list and the other channels read now: each one's list
 * walked by hvs6s_snapshot to its END, its LPTRS.HEADE and its DL.LACT. Only
 * reads; no word value enters it. With verbose, one S6 line per channel. */
static void place_input(const struct hvs6s_cfg *c, const struct hvs6s_snap *s, uint32_t len,
			bool verbose, struct swap_place_in *in)
{
	*in = (struct swap_place_in){
		.mem_words = c->mem_words,
		.fw_head = s->fw_head,
		.fw_words = s->fw_len,
		.end_found = s->taken && s->fw_end_found && !s->fw_noend,
		.unknown_chan = -1,
		.len = len,
	};
	for (unsigned int ch = 0; ch < YD5_HVS6_NUM_CHANNELS; ch++) {
		struct hvs6s_cfg oc = *c;
		uint32_t lp, la;
		enum hvs6s_err e;

		if (ch == c->chan) {
			continue;
		}
		lp = hvs_rd(hvs6s_lptrs_reg(ch)) & YD5_HVS6_LPTRS_HEADE_MASK;
		la = hvs_rd(hvs6s_dl_reg(ch)) & HVS6S_DL_LACT_MASK;
		oc.chan = ch;
		e = hvs6s_snapshot(&io, &oc, &chsnap);
		if (in->nbusy + 3u > SWAP_PLACE_BUSY_MAX) {
			/* not reached with three channels: refuse rather than guess */
			if (in->unknown_chan < 0) {
				in->unknown_chan = (int)ch;
			}
			continue;
		}
		in->busy[in->nbusy++] = (struct swap_span){ lp, 1u };
		in->busy[in->nbusy++] = (struct swap_span){ la, 1u };
		if (e == HVS6S_OK && chsnap.fw_end_found && !chsnap.fw_noend) {
			in->busy[in->nbusy++] = (struct swap_span){ chsnap.fw_head, chsnap.fw_len };
		} else if (in->unknown_chan < 0) {
			in->unknown_chan = (int)ch;
		}
		if (verbose) {
			printk("rpi5_scanout32: S6 ch%u heade %u lact %u list %s %u..%u (%u entries, "
			       "walk stopped at word %u = 0x%08x%s)\n",
			       ch, lp, la,
			       e != HVS6S_OK ? "NOT walked:"
			       : chsnap.fw_end_found ? "END-walked"
						     : "NOT walked to an END:",
			       chsnap.fw_head, chsnap.fw_head + (chsnap.fw_len ? chsnap.fw_len - 1u : 0u),
			       chsnap.fw_entries, chsnap.walk_stop, chsnap.walk_word,
			       e != HVS6S_OK ? ", refused by the walk" : "");
		}
	}
}

/* The placement from input in, checked once more against the library's
 * geometry (words 0..31, the firmware's span, the memory). */
static enum swap_place_reason place_after_end(const struct hvs6s_cfg *c, const struct hvs6s_snap *s,
					      const struct swap_place_in *in, uint32_t *start)
{
	enum swap_place_reason why = swap_place_after_end(in, start);

	if (why == SWAP_PLACE_OK && hvs6s_check_span(c, s, *start, in->len) != HVS6S_OK) {
		why = SWAP_PLACE_NO_ROOM;
	}
	return why;
}

static enum hvs6s_err place_err_of(enum swap_place_reason why)
{
	switch (why) {
	case SWAP_PLACE_OK: return HVS6S_OK;
	case SWAP_PLACE_NO_END: return HVS6S_ERR_PLACE_NOEND;
	case SWAP_PLACE_OTHER_UNKNOWN: return HVS6S_ERR_PLACE_CHAN;
	case SWAP_PLACE_NO_ROOM: return HVS6S_ERR_PLACE_FIT;
	case SWAP_PLACE_BAD_ARG: break;
	}
	return HVS6S_ERR_ARG;
}

/* Before the first write to words [pair + from, pair + to): the placement,
 * computed again from snapshot s and the other channels read now, is still
 * pair, and those words still hold what S6 saved. HVS6S_OK, or the refusal
 * with a reason in why_txt and, for a changed word, the word in
 * r->place_bad_word/_val. Only reads. */
static enum hvs6s_err recheck_pair(const struct hvs6s_cfg *c, const struct hvs6s_snap *s,
				   uint32_t pair, uint32_t from, uint32_t to,
				   struct hvs6s_report *r, char *why_txt, size_t why_len)
{
	struct swap_place_in in;
	enum swap_place_reason pw;
	uint32_t st = 0;

	place_input(c, s, PAIR_WORDS, false, &in);
	pw = place_after_end(c, s, &in, &st);
	if (pw != SWAP_PLACE_OK) {
		snprintk(why_txt, why_len, "placement now refused: %s", swap_place_reason_name(pw));
		return place_err_of(pw);
	}
	if (st != pair) {
		snprintk(why_txt, why_len, "placement moved from word %u to word %u", pair, st);
		return HVS6S_ERR_PLACE_FW;
	}
	for (uint32_t i = from; i < to; i++) {
		uint32_t now = hvs_rd(hvs6s_word_off(pair + i));

		if (now != pair_saved[i]) {
			r->place_bad_word = pair + i;
			r->place_bad_val = now;
			snprintk(why_txt, why_len, "word %u changed since S6: saved 0x%08x now 0x%08x",
				 pair + i, pair_saved[i], now);
			return HVS6S_ERR_PLACE_USED;
		}
	}
	return HVS6S_OK;
}

/* Swap 1: hvs6s_run's sequence with the probe's placement in place of the
 * library's zero-word test (hvs6s_place_io refuses the board's 0xb0b0b0b0
 * fill): snapshot, the pair rechecked (recheck_pair), the swap to slot A and
 * the restore. The restore runs whenever the swap was called; nothing is
 * written when the snapshot or the recheck refused. *wrote: the swap was
 * called (our words may be written). Returns r->err. */
static enum hvs6s_err run_swap1(const struct hvs6s_cfg *c, struct hvs6s_snap *s, uint32_t pair,
				struct hvs6s_report *r, bool *wrote)
{
	enum hvs6s_err e;
	char why_txt[96];

	hvs6s_report_init(r);
	r->hw = c->hw;
	*wrote = false;
	e = hvs6s_snapshot(&io, c, s);
	if (e != HVS6S_OK) {
		hvs6s_fail(r, e);
		return r->err;
	}
	r->place_err = recheck_pair(c, s, pair, 0, PAIR_WORDS, r, why_txt, sizeof(why_txt));
	if (r->place_err != HVS6S_OK) {
		printk("rpi5_scanout32: S8 slot A place FAIL: %s; nothing is written\n", why_txt);
		hvs6s_fail(r, r->place_err);
		return r->err;
	}
	r->start = pair;
	r->len = SLOT_WORDS;
	*wrote = true;
	e = hvs6s_swap(&io, c, pair, r);
	if (e != HVS6S_OK) {
		hvs6s_fail(r, e);
	}
	e = hvs6s_restore(&io, c, s, r);
	if (e != HVS6S_OK) {
		hvs6s_fail(r, e);
	}
	return r->err;
}

/* The firmware's span in words for swap_check.h: the snapshot's fw_len, the
 * walked list to its END or, with no END in view, the fixed window, so that
 * the judge and the restore compare against the same span. */
static uint32_t fw_words_of(const struct hvs6s_snap *s)
{
	return s->fw_len;
}

static bool snaps_equal(const struct hvs6s_snap *a, const struct hvs6s_snap *b)
{
	if (!a->taken || !b->taken || a->lptrs != b->lptrs || a->fw_head != b->fw_head ||
	    a->fw_len != b->fw_len) {
		return false;
	}
	/* a declared hardware-written word may differ between the two */
	return swap_diff_words(&shw, a->fw, b->fw, a->fw_len, a->fw_head, NULL, NULL) == 0;
}

/* One swap's sample for swap_check.h, all ten fields. The registers are the
 * header's samples when LPTRS was written (at the write, and once read-back
 * and FRCNT were in), else the probe's own reads around the call. */
static void fill_step(struct swap_step *st, struct swap_slot *sl, unsigned int k,
		      const struct hvs6w_plane *plane, const struct hvs6s_report *r,
		      uint32_t pre_lptrs, uint32_t pre_status, uint32_t post_lptrs,
		      uint32_t post_status, uint32_t slot_word, uint64_t buf_bus,
		      const struct hvs6s_snap *s)
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
	st->plane_addr = slot_read(slot_word, plane, k, sl, &prc);
	st->expected_addr = buf_bus;
	if (prc != HVS6_OK) {
		printk("rpi5_scanout32: S8 slot word %u: plane entry did not decode (hvs6 error "
		       "%d)\n", slot_word, prc);
	}
}

/* Swap k's vblank record for swap_check.h, from hvs6s_swap's report. */
static void to_swap_vblank(const struct hvs6s_report *r, struct swap_vblank *v)
{
	*v = (struct swap_vblank){
		.tried = r->vb_tried,
		.found = r->vb_found,
		.swapped = r->swapped,
		.in_vblank = r->write_in_vblank,
		.tight_pickup = r->tight_pickup,
		.boundary = r->lact_boundary,
		.timed = r->timed,
		.frcnt_write = r->write_frcnt,
		.frcnt_pickup = r->pickup_frcnt,
		.pickup_reads = r->tight_reads,
		.dt_ns = r->lact_dt_ns,
		.read_ns = r->read_ns,
	};
}

/* The u3 vblank line of swap k (verdicts.sh's header). */
static void vblank_line(unsigned int k, const struct swap_vblank *v, const struct hvs6s_report *r)
{
	enum swap_vb_reason why = swap_vblank_judge(v);
	char dt[32], rd[32], since[32];

	printk("u3 vblank %u: landed=%s why=%s frcnt %u -> %u steps=%u invblank=%s tight=%s "
	       "pickup_reads=%u pickup_us=%s read_us=%s boundary=%s since_us=%s yline %u -> %u "
	       "(yline record only)\n",
	       k + 1u, why == SWAP_VB_LANDED ? "yes" : "no", swap_vb_reason_name(why),
	       v->frcnt_write, v->frcnt_pickup, swap_vb_steps(v), v->in_vblank ? "yes" : "no",
	       v->tight_pickup ? "ok" : "miss", v->pickup_reads,
	       hvs6s_us_text(v->timed && v->tight_pickup, v->dt_ns, dt, sizeof(dt)),
	       hvs6s_us_text(v->timed && v->pickup_reads, v->read_ns, rd, sizeof(rd)),
	       v->boundary ? "yes" : "no",
	       hvs6s_us_text(v->timed && v->swapped, r->vb_since_ns, since, sizeof(since)),
	       r->write_yline, r->pickup_yline);
}

/* The dlswap summary, DL.LACT pickup and u3 swap lines of swap k (and u3
 * named after swap 1); returns the judge's reason. Swap 1's pickup line is
 * in run_swap1's record (hvs6s_lines), so the k-th 'dlswap lact-pickup' line
 * of the log is swap k's. */
static enum swap_reason swap_lines(unsigned int k, const struct swap_step *st,
				   const struct hvs6s_report *r, enum hvs6s_err e, bool *named)
{
	enum swap_reason why = swap_judge(&st->sample);
	uint32_t slot_word = st->sample.expected_word;

	printk("dlswap swap%u %s %s lptrs-readback %s frcnt-advance %s settle %s\n",
	       k + 1u, e == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(e),
	       r->lptrs_ok ? "ok" : "FAIL", r->frcnt_ok ? "ok" : "FAIL",
	       r->settle_ok ? "ok" : (r->frcnt_ok ? "FAIL" : "skip"));
	if (k != 0) {
		hvs6s_lact_line(&emit, "lact-pickup", r->lact_pickup, r->lact_polls,
				r->lact_frcnt_write, r->lact_frcnt_pickup, r->lact_want,
				r->lact_seen);
	}
	printk("u3 swap %u: buf %c list-ptr=0x%08x frcnt %u -> %u judge=%s slot=%u "
	       "plane=0x%010llx fw=%u+%u ours=%u\n",
	       k + 1u, (k & 1u) ? 'B' : 'A', st->sample.ptr_after,
	       swap_frcnt(st->sample.status_before), swap_frcnt(st->sample.status_after),
	       swap_reason_name(why), slot_word, (unsigned long long)st->plane_addr,
	       st->sample.fw_start, st->sample.fw_words, st->sample.our_words);
	to_swap_vblank(r, &vbs[k]);
	vblank_line(k, &vbs[k], r);
	if (k == 0) {
		*named = swap_named(st->sample.ptr_after, slot_word, r->lact_pickup);
		printk("u3 named: %s LPTRS 0x%08x after swap 1, our list at word %u, "
		       "first read 0x%08x poll %u, DL.LACT pickup %s (want %u lact %u)\n",
		       *named ? "yes" : "no", st->sample.ptr_after, slot_word, r->lptrs_first,
		       r->lptrs_poll, r->lact_pickup ? "ok" : "FAIL", r->lact_want, r->lact_seen);
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
	/* static: each report holds two diff records, too large for the main stack */
	static struct hvs6s_report plan, runrep, rrest;
	enum hvs6s_err snap_err = HVS6S_ERR_ARG, place_err = HVS6S_ERR_ARG, first_err = HVS6S_OK;
	enum hvs6s_err run_err = HVS6S_ERR_ARG;
	uint32_t pair = 0, slot[2] = {0, 0};
	uint32_t fw_w = FB_MAX_W, fw_h = FB_MAX_H, pitch;
	uint64_t phys[2] = {0, 0}, bus[2] = {0, 0};
	bool bufs_ok = false, placed = false, named = false, readback_ok = false;
	bool restore_pass = false, ran = false, wrote_list = false, wrote_more = false;
	bool same_snap = false, hw_bad = false;
	unsigned int nsteps = 0, swaps_ok = 0;
	const char *fail = NULL;
	char failbuf[256];
	char why[192];

	printk("rpi5_scanout32: build %s\n", RPI5_SCANOUT32_BUILD);
	rpi_wdog_announce();
	printk("rpi5_scanout32: stage 2: our XRGB8888 list right after the END of the firmware's "
	       "list (walked from LPTRS.HEADE over the list memory), at or after word 32, its words "
	       "saved and written back, %u swaps A/B on channel %u, then the firmware's list "
	       "restored; no pixel valve, HDMI or PHY write\n",
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
		} else if (next_entry < got) {
			printk(" (entry walk stopped at list index %u, word %u = 0x%08x)\n",
			       next_entry, start + next_entry, snap[next_entry]);
		} else {
			printk(" (entry walk stopped at list index %u, word %u, past the %u words "
			       "read)\n", next_entry, start + next_entry, got);
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
	cfg.lact_polls = 0; /* the pickup's poll bound: max_polls */
	cfg.lact_frames = LACT_FRAMES;
	/* LPTRS just after an FRCNT step found by reads, the pickup timed by the
	 * tight loop; the read bounds and the vblank window are the library's
	 * defaults (HVS6S_VBLANK_READS_DEF, HVS6S_TIGHT_READS_DEF, 667 us) */
	cfg.write_at = HVS6S_AT_VBLANK;
	/* scanout32-fold: the walk covers the whole list memory past HEADE (the
	 * board's END is at word 928, 257 words from HEADE 672), and the snapshot
	 * is the whole walked list, END included */
	cfg.walk_words = 0;
	hvs6s_report_init(&plan);
	{
		int hrc = hvs6s_hw_parse(RPI5_SCANOUT32_HW_WORDS, &cfg.hw);

		to_swap_hw(&cfg.hw, &shw);
		(void)hvs6s_hw_format(&cfg.hw, hwtxt, sizeof(hwtxt));
		(void)hvs6s_hw_offs(&cfg.hw, hwoffs, sizeof(hwoffs));
		plan.hw = cfg.hw;
		printk("rpi5_scanout32: S5 hw words '%s': %s%s\n", RPI5_SCANOUT32_HW_WORDS,
		       hrc == 0 ? hwtxt : "do not parse",
		       hrc == 0 ? (cfg.hw.n ? " (left out of the hash and the byte-equal compares)"
				   : " (every word compared byte-equal)")
				: "; nothing is read or written");
		if (hrc != 0) {
			hw_bad = true;
		}
	}
	if (hw_bad) {
		snap_err = HVS6S_ERR_ARG;
		printk("rpi5_scanout32: S5 dlswap snapshot FAIL: the hardware-written word set does "
		       "not parse\n");
	} else if (hvs == 0) {
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
		       plansnap.fw_end_found ? "found"
		       : plansnap.fw_noend   ? "NOT found: fixed window kept, our list before HEADE"
					     : "NOT found");
	}
	if (snap_err == HVS6S_OK) {
		/* The hash is over the firmware's words [HEADE, HEADE + fw_len),
		 * END included: the list the channel runs, as the restore compares. */
		printk("u3 snapshot: list-ptr=0x%08x heade=%u words=%u hash=0x%08x%s hwskip=%s\n",
		       plansnap.lptrs, plansnap.fw_head, plansnap.fw_len,
		       swap_words_hash(&shw, plansnap.fw, plansnap.fw, plansnap.fw_len),
		       plansnap.fw_noend ? " window=noend" : "", hwtxt);
	} else {
		uint32_t sl = plansnap.lptrs != 0 ? plansnap.lptrs : lptrs;

		hvs6s_fail(&plan, snap_err);
		printk("u3 snapshot: list-ptr=0x%08x heade=%u words=0 hash=0x00000000 FAIL %s "
		       "(walk stopped at word %u, 0x%08x, after %u entries) hwskip=%s\n",
		       sl, sl & YD5_HVS6_LPTRS_HEADE_MASK,
		       hw_bad ? "hardware-written word set does not parse" : hvs6s_err_name(snap_err),
		       plansnap.walk_stop, plansnap.walk_word, plansnap.fw_entries,
		       hw_bad ? "bad" : hwtxt);
		first_err = snap_err;
		fail = hw_bad ? "the hardware-written word set does not parse"
			      : "no snapshot of the firmware's list";
	}

	/* S6: the plan's placement, read only (selftests-s32b): one span for
	 * slot A and slot B right after the END the walk found (swap_check.h's
	 * swap_place_after_end), at or above word 32, inside the list memory,
	 * clear of the other channels' lists, LPTRS.HEADE and DL.LACT. No word
	 * value decides it: on the board the words after the END hold a
	 * 0xb0b0b0b0 fill, and zeros below HEADE are not taken either. The words
	 * are saved here and written back at S10. */
	for (unsigned int c = 0; c < YD5_HVS6_NUM_CHANNELS; c++) {
		printk("rpi5_scanout32: S6 ch%u LPTRS 0x%08x DL 0x%08x\n", c,
		       hvs_rd(hvs6s_lptrs_reg(c)), hvs_rd(hvs6s_dl_reg(c)));
	}
	if (snap_err == HVS6S_OK) {
		struct swap_place_in pin;
		enum swap_place_reason pw;

		place_input(&cfg, &plansnap, PAIR_WORDS, true, &pin);
		pw = place_after_end(&cfg, &plansnap, &pin, &pair);
		place_err = place_err_of(pw);
		if (pw == SWAP_PLACE_OTHER_UNKNOWN) {
			snprintk(why, sizeof(why), "%s (channel %d)", swap_place_reason_name(pw),
				 pin.unknown_chan);
		} else {
			snprintk(why, sizeof(why), "%s", swap_place_reason_name(pw));
		}
		if (place_err == HVS6S_OK) {
			/* every channel's list pointer, channel 0's included, once more */
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
		}
		/* the walk's result: verdicts.sh's 'u3 walk' line */
		if (place_err == HVS6S_OK) {
			printk("u3 walk: heade=%u end=%u words=%u entries=%u invalid=%u mem=%u len=%u "
			       "place=%u\n",
			       plansnap.fw_head, plansnap.fw_head + plansnap.fw_len - 1u,
			       plansnap.fw_len, plansnap.fw_entries, plansnap.fw_invalid,
			       cfg.mem_words, PAIR_WORDS, pair);
		} else if (pin.end_found) {
			printk("u3 walk: heade=%u end=%u words=%u entries=%u invalid=%u mem=%u len=%u "
			       "place=refused (%s)\n",
			       plansnap.fw_head, plansnap.fw_head + plansnap.fw_len - 1u,
			       plansnap.fw_len, plansnap.fw_entries, plansnap.fw_invalid,
			       cfg.mem_words, PAIR_WORDS, why);
		} else {
			printk("u3 walk: heade=%u end=none words=%u entries=%u invalid=%u mem=%u len=%u "
			       "place=refused (%s, walk stopped at word %u = 0x%08x)\n",
			       plansnap.fw_head, plansnap.fw_len, plansnap.fw_entries,
			       plansnap.fw_invalid, cfg.mem_words, PAIR_WORDS, why,
			       plansnap.walk_stop, plansnap.walk_word);
		}
		plan.place_err = place_err;
		placed = place_err == HVS6S_OK;
	} else {
		snprintk(why, sizeof(why), "no snapshot: %s", hvs6s_err_name(snap_err));
		printk("u3 walk: heade=%u end=none words=0 entries=%u invalid=%u mem=%u len=%u "
		       "place=refused (%s)\n",
		       plansnap.fw_head, plansnap.fw_entries, plansnap.fw_invalid, cfg.mem_words,
		       PAIR_WORDS, why);
	}
	if (placed) {
		slot[0] = pair;
		slot[1] = pair + SLOT_WORDS;
		plan.start = pair;
		plan.len = PAIR_WORDS;
		for (i = 0; i < PAIR_WORDS; i++) {
			pair_saved[i] = hvs_rd(hvs6s_word_off(pair + i));
		}
		printk("rpi5_scanout32: S6 placement check ok: words %u..%u right after the "
		       "firmware's list %u..%u (%u entries, %u with VALID clear, END at word %u), "
		       "at or above word %u, inside the %u-word list memory, clear of every "
		       "channel's list and list pointer; their words are not assumed free: saved "
		       "and written back at S10\n",
		       pair, pair + PAIR_WORDS - 1u, plansnap.fw_head,
		       plansnap.fw_head + plansnap.fw_len - 1u, plansnap.fw_entries,
		       plansnap.fw_invalid, plansnap.walk_stop, HVS6S_KEPT, cfg.mem_words);
		printk("rpi5_scanout32: S6 saved words %u..%u:", pair, pair + PAIR_WORDS - 1u);
		for (i = 0; i < PAIR_WORDS; i++) {
			printk(" 0x%08x", pair_saved[i]);
		}
		printk("\n");
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
			fail = "our list was refused: no place after the firmware list's END";
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

	/* S8: swap 1 is run_swap1 at slot A (hvs6s_run's sequence: its own
	 * snapshot, the placement checked again with the saved words unchanged,
	 * the swap to buffer A and the restore); then slot B is checked the same
	 * way and swaps 2.. alternate B, A, B ... */
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

		ran = true;
		/* our list's words are written once the run's placement passed */
		run_err = run_swap1(&cfg, &fwsnap, slot[0], &runrep, &wrote_list);
		same_snap = snaps_equal(&plansnap, &fwsnap);
		printk("rpi5_scanout32: S8 swap 1 run %s (%s) at slot A word %u: swapped %d "
		       "restored %d; its snapshot %s the plan's\n",
		       run_err == HVS6S_OK ? "ok" : "FAIL", hvs6s_err_name(run_err), slot[0],
		       runrep.swapped, runrep.restored, same_snap ? "equals" : "DIFFERS from");
		(void)hvs6s_lines(&fwsnap, &runrep, &emit);
		fill_step(&steps[0], &slots[0], 0, &cfg.plane, &runrep, pre_lptrs, pre_status,
			  hvs_rd(hvs6s_lptrs_reg(CHAN)), hvs_rd(hvs6s_status_reg(CHAN)), slot[0],
			  bus[0], &fwsnap);
		nsteps = 1;
		pickups[0] = runrep.lact_pickup;
		w = swap_lines(0, &steps[0], &runrep, run_err, &named);
		if (run_err != HVS6S_OK && first_err == HVS6S_OK) {
			first_err = run_err;
		}
		if (run_err == HVS6S_OK && w == SWAP_OURS && runrep.lact_pickup) {
			swaps_ok++;
		}
		if (!same_snap && fail == NULL) {
			fail = "the firmware's list changed between the plan and the run";
		}
		k_msleep(HOLD_MS);
	}

	/* Slot B, read and checked before its first write, as slot A was. */
	{
		/* swaps 2.. only after swap 1's list was picked up and DL.LACT came
		 * back to the firmware's head at its restore */
		bool go = ran && runrep.swapped && runrep.lact_pickup && runrep.restore_lact_ok &&
			  runrep.restore_equal && same_snap;

		if (go) {
			static struct hvs6s_report bplace;
			char btxt[96];
			enum hvs6s_err be;

			hvs6s_report_init(&bplace);
			be = recheck_pair(&cfg, &fwsnap, pair, SLOT_WORDS, PAIR_WORDS, &bplace, btxt,
					  sizeof(btxt));
			if (be == HVS6S_OK) {
				printk("rpi5_scanout32: S8 slot B place ok: words %u..%u still after "
				       "the END and as saved at S6\n",
				       slot[1], slot[1] + SLOT_WORDS - 1u);
			} else {
				go = false;
				snprintk(why, sizeof(why), "slot B refused: %s: %s", hvs6s_err_name(be),
					 btxt);
				printk("rpi5_scanout32: S8 slot B place FAIL: %s; nothing more is "
				       "written\n", why);
				if (first_err == HVS6S_OK) {
					first_err = be;
					fail = "slot B was refused: its place or words changed since S6";
				}
			}
		} else if (ran) {
			snprintk(why, sizeof(why), "swap 1: %s",
				 !runrep.swapped ? "LPTRS was not written"
				 : !runrep.lact_pickup ? "DL.LACT did not reach the new head"
				 : !runrep.restore_lact_ok ? "DL.LACT not back on the firmware's head"
				 : !runrep.restore_equal ? "the run's restore was not byte-equal"
				 : "the run's snapshot differed from the plan's");
		} else {
			snprintk(why, sizeof(why), "%s", !placed ? "not placed" : "buffers not ready");
		}

		for (unsigned int k = ran ? 1u : 0u; k < SWAPS; k++) {
			unsigned int b = k & 1u;
			static struct hvs6s_report r;
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
			if (r.vb_tried) {
				/* the vblank wait and, once LPTRS was written, the tight
				 * pickup loop, as hvs6s_lines prints them for swap 1 */
				hvs6s_vblank_line(&emit, &r);
				if (r.swapped) {
					hvs6s_timing_line(&emit, &r);
				}
			}
			if (r.ldiff.total != 0) {
				/* hvs6s_swap's own read-back, before LPTRS */
				(void)hvs6s_diff_lines(&emit, "readback", &r.ldiff, &cfg.hw);
			}
			fill_step(&steps[k], &slots[k], k, &cfg.plane, &r, pre_lptrs, pre_status,
				  hvs_rd(hvs6s_lptrs_reg(CHAN)), hvs_rd(hvs6s_status_reg(CHAN)), slot[b],
				  bus[b], &fwsnap);
			nsteps = k + 1u;
			pickups[k] = r.lact_pickup;
			w = swap_lines(k, &steps[k], &r, e, &named);
			if (e != HVS6S_OK && first_err == HVS6S_OK) {
				first_err = e;
			}
			if (e != HVS6S_OK) {
				/* no more LPTRS writes; S10 restores at once */
				go = false;
				snprintk(why, sizeof(why), "swap %u failed", k + 1u);
			}
			if (e == HVS6S_OK && w == SWAP_OURS && r.lact_pickup) {
				swaps_ok++;
			}
			k_msleep(HOLD_MS);
		}
	}
	if (!ran) {
		printk("u3 named: no (no swap was made)\n");
	} else {
		struct swap_seq_verdict v = swap_seq_judge_lact(steps, slots, pickups, nsteps, &shw);

		printk("rpi5_scanout32: S8 sequence %s (%s) index %d swap %s, %u of %u swaps ok\n",
		       v.pass ? "ok" : "FAIL", swap_seq_reason_name(v.reason), v.index,
		       swap_reason_name(v.swap), swaps_ok, nsteps);
		if (v.reason == SWAP_SEQ_SLOT_DIFF) {
			printk("rpi5_scanout32: S8 swap %d: slot word %d (list word %u) wrote 0x%08x "
			       "read 0x%08x, not a declared hardware-written word (hwskip=%s)\n",
			       v.index + 1, v.word, slots[v.index].start + (uint32_t)v.word, v.want,
			       v.got, hwtxt);
		}
		if (!v.pass && fail == NULL) {
			if (v.reason == SWAP_SEQ_SLOT_DIFF) {
				snprintk(failbuf, sizeof(failbuf), "swap %d: %s (slot word %d)",
					 v.index + 1, swap_seq_reason_name(v.reason), v.word);
			} else {
				snprintk(failbuf, sizeof(failbuf), "swap %d: %s (%s)", v.index + 1,
					 swap_seq_reason_name(v.reason), swap_reason_name(v.swap));
			}
			fail = failbuf;
		}
		{
			enum swap_vb_reason vw = SWAP_VB_LANDED;
			int vi = swap_vblank_first_miss(vbs, nsteps, &vw);

			if (vi < 0) {
				printk("rpi5_scanout32: S8 vblank ok: every swap written in the "
				       "vblank and picked up within one frame step\n");
			} else {
				printk("rpi5_scanout32: S8 vblank FAIL: swap %d %s\n", vi + 1,
				       swap_vb_reason_name(vw));
			}
			if (vi >= 0 && fail == NULL) {
				snprintk(failbuf, sizeof(failbuf),
					 "swap %d did not land in vblank (%s)", vi + 1,
					 swap_vb_reason_name(vw));
				fail = failbuf;
			}
		}
		if (nsteps < SWAPS && fail == NULL) {
			snprintk(failbuf, sizeof(failbuf), "%u of %u swaps made (%s)", nsteps, SWAPS,
				 why);
			fail = failbuf;
		}
		if (!named && fail == NULL) {
			fail = "channel 0 did not name our list after swap 1 (LPTRS and DL.LACT)";
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
	 * was taken. Swap 1's restore is run_swap1's own. When nothing more
	 * was written, no register or word is written here: the compare alone
	 * runs (and our words go back to the words S6 saved). */
	{
		const struct hvs6s_snap *ref = fwsnap.taken ? &fwsnap : &plansnap;

		if (ref->taken) {
			enum hvs6s_err e = HVS6S_OK;
			struct swap_restore_verdict vf;
			uint32_t rb_ptr, rb_lact;
			bool cleared = true, settled, lact_ok, picked;

			if (wrote_more) {
				e = hvs6s_restore(&io, &cfg, &fwsnap, &rrest);
				hvs6s_regs_line(&emit, "restore-after", &rrest.after_restore);
				printk("dlswap restore-frcnt %s polls=%u\n",
				       rrest.restore_frcnt_ok ? "ok" : "FAIL", rrest.restore_polls);
				hvs6s_lact_line(&emit, "restore-lact", rrest.restore_lact_ok,
						rrest.restore_lact_polls, rrest.restore_frcnt_write,
						rrest.restore_frcnt_pickup, rrest.restore_lact_want,
						rrest.restore_lact_seen);
				settled = rrest.restore_frcnt_ok;
			} else if (ran) {
				printk("rpi5_scanout32: S10 restore write skip: no swap after swap 1; "
				       "swap 1's own restore %s, its DL.LACT %s; compare only\n",
				       runrep.restored ? "wrote the firmware's LPTRS back"
				       : "did not run",
				       runrep.restore_lact_ok ? "back on the firmware's head"
				       : "NOT back on the firmware's head");
				settled = runrep.restore_frcnt_ok;
				/* swap 1's own restore (its restore-lact line is in the
				 * record) decides the restore's DL.LACT here */
				if (!runrep.restore_lact_ok) {
					e = HVS6S_ERR_RESTORE_LACT;
				}
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
			vf = swap_restore_judge_hw(ref->fw, fw_rb, ref->fw_len, ref->lptrs, rb_ptr,
						   &shw, ref->fw_head, &rdiffs);
			print_diffs("restore", &rdiffs, "");

			/* Our span back to the words S6 saved (they were not assumed
			 * free: on the board the 0xb0b0b0b0 fill), once channel 0 is
			 * back on the firmware's list, frames have passed and neither
			 * its LPTRS nor DL.LACT is inside it. */
			lact_ok = swap_restore_lact(wrote_more, ran, rrest.restore_lact_ok,
						    runrep.restore_lact_ok);
			picked = swap_pickups_all(pickups, nsteps);
			if (wrote_list) {
				/* only once the restore's DL.LACT is back (a positive
				 * check), the frames passed and LPTRS is the firmware's */
				bool back = swap_clear_ok(settled, lact_ok, rb_ptr, rb_lact,
							  ref->fw_head, pair, PAIR_WORDS);
				int bad = -1;
				uint32_t badv = 0;

				if (back) {
					for (i = 0; i < PAIR_WORDS; i++) {
						io_write32(NULL, hvs6s_word_off(pair + i), pair_saved[i]);
					}
					barrier_dsync_fence_full();
					for (i = 0; i < PAIR_WORDS && bad < 0; i++) {
						uint32_t v = hvs_rd(hvs6s_word_off(pair + i));

						if (v != pair_saved[i]) {
							bad = (int)i;
							badv = v;
						}
					}
				}
				cleared = back && bad < 0;
				if (!back) {
					printk("rpi5_scanout32: S10 our words %u..%u left in place: "
					       "channel 0 is not back on the firmware's list (frames, "
					       "LPTRS or DL.LACT)\n", pair, pair + PAIR_WORDS - 1u);
				} else if (bad < 0) {
					printk("rpi5_scanout32: S10 our words %u..%u back to the words "
					       "saved at S6\n", pair, pair + PAIR_WORDS - 1u);
				} else {
					printk("rpi5_scanout32: S10 our words %u..%u NOT back to the "
					       "words saved at S6: word %u saved 0x%08x reads 0x%08x\n",
					       pair, pair + PAIR_WORDS - 1u, pair + (uint32_t)bad,
					       pair_saved[bad], badv);
				}
			}

			restore_pass = swap_restore_pass(e == HVS6S_OK, vf.pass, cleared, lact_ok,
							 picked);
			if (e == HVS6S_OK && !picked) {
				/* the restore went through, but some swap's list was
				 * never picked up by the HVS: not a PASS */
				e = HVS6S_ERR_LACT_PICKUP;
			}
			printk("rpi5_scanout32: S10 dlswap restore %s (%s); fw %u..+%u %s index %d "
			       "want 0x%08x got 0x%08x\n",
			       !wrote_more ? "skip" : e == HVS6S_OK ? "ok" : "FAIL",
			       hvs6s_err_name(e), ref->fw_head, ref->fw_len,
			       swap_restore_reason_name(vf.reason), vf.index, vf.want, vf.got);
			if (restore_pass && shw.n == 0) {
				printk("u3 restore: PASS byte-equal list-ptr=0x%08x heade=%u words=%u "
				       "hash=0x%08x hwskip=none%s\n",
				       rb_ptr, ref->fw_head, ref->fw_len,
				       swap_words_hash(&shw, ref->fw, fw_rb, ref->fw_len),
				       wrote_list ? "" : " (nothing was written)");
			} else if (restore_pass) {
				printk("u3 restore: PASS byte-equal except hw words off %s (%u differed: "
				       "%u skip, %u expect) list-ptr=0x%08x heade=%u words=%u "
				       "hash=0x%08x hwskip=%s%s\n",
				       hwoffs, rdiffs.skip + rdiffs.expect, rdiffs.skip, rdiffs.expect,
				       rb_ptr, ref->fw_head, ref->fw_len,
				       swap_words_hash(&shw, ref->fw, fw_rb, ref->fw_len), hwtxt,
				       wrote_list ? "" : " (nothing was written)");
			} else {
				printk("u3 restore: FAIL %s list-ptr=0x%08x want 0x%08x (%s%s%s%s) "
				       "hash=0x%08x hwskip=%s undeclared=%u\n",
				       hvs6s_err_name(e), rb_ptr, ref->lptrs,
				       swap_restore_reason_name(vf.reason),
				       lact_ok ? "" : " / DL.LACT not back",
				       picked ? "" : " / a swap's DL.LACT never reached its head",
				       cleared ? "" : " / our words not written back",
				       swap_words_hash(&shw, ref->fw, fw_rb, ref->fw_len), hwtxt,
				       rdiffs.undeclared);
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

	if (refused_writes != 0u && fail == NULL) {
		snprintk(failbuf, sizeof(failbuf), "%u write(s) refused by the io filter",
			 refused_writes);
		fail = failbuf;
	}

	/* The dlswap record of the plan when run_swap1 did not run (it printed
	 * its own record after swap 1): snapshot, placement, nothing written. */
	if (!ran) {
		hvs6s_fail(&plan, first_err);
		(void)hvs6s_lines(&plansnap, &plan, &emit);
	}

	/* the main stack's high-water mark (CONFIG_INIT_STACKS), after every
	 * record line was printed */
	{
		size_t unused = 0;
		int src = k_thread_stack_space_get(k_current_get(), &unused);
		bool sok = src == 0 && swap_stack_ok(CONFIG_MAIN_STACK_SIZE, (uint32_t)unused);

		printk("rpi5_scanout32: S11 main stack size %u unused %u (%s, need %u unused)\n",
		       (unsigned)CONFIG_MAIN_STACK_SIZE, (unsigned)unused,
		       src != 0 ? "unmeasured" : sok ? "ok" : "low", (unsigned)SWAP_STACK_MARGIN);
		if (!sok && fail == NULL) {
			fail = "the main stack came within its margin (S11)";
		}
	}

	if (fail == NULL && first_err != HVS6S_OK) {
		fail = hvs6s_err_name(first_err);
	}
	if (fail == NULL) {
		printk("rpi5_scanout32: PASS (%u swaps alternating A/B, list at word %u, each "
		       "written in the vblank and picked up within one frame step, DL.LACT "
		       "picked up each and came back, FRCNT advanced, buffers read back, firmware "
		       "list restored byte-equal%s%s)\n",
		       nsteps, pair, shw.n ? " except hw words off " : "", shw.n ? hwoffs : "");
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
