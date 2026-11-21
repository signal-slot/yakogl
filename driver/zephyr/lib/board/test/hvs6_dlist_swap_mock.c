/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/hvs6_dlist_swap.h against a model of one D0 HVS
 * channel: list memory, LPTRS, STATUS (FRCNT, MODE), DL, CTRL1, and a frame
 * edge every few polls. The model logs every write, clean and barrier in one
 * sequence so that order (list, clean, barrier, LPTRS last) is checked.
 *
 * Cases: the pass path; LPTRS and LACT following only at the next FRCNT
 * edge; LPTRS never reading back (fails, named); an FRCNT stall (timeout,
 * named); FRCNT wrapping 63 to 0; a placement overlapping words 0..31
 * (refused); a placement on the firmware's list (refused); a list that does
 * not fit (refused); a restore with one changed word (fails, naming it);
 * CTRL1 never written, in every case.
 *
 * The board's layout (s32dumprun, Pi 5 D0: LPTRS = DL = 0x2a0): HEADE=672
 * with a 32-word firmware entry and the list END at 704 (swap and byte-equal
 * restore of exactly 672..704); a zero word where the next entry should be
 * (refused, nothing written, the five record lines); a target range holding
 * a non-zero word (refused, named; auto steps past it); a target range
 * overlapping 672..704 (refused); a range that does not fit (refused).
 *
 * No END in view (s32noend, the board's s32dumprun): (a) the window kept at
 * HEADE, clamped at 256 words; (b) clamped to mem_words near the memory's
 * end; (c) our list placed wholly before HEADE, swap and byte-equal restore
 * of the window; (d) no room before HEADE, or a start after the window:
 * PLACE_NOEND; (e) a non-zero word before HEADE: PLACE_USED (auto steps
 * past it, never past HEADE); (f) another channel's LPTRS.HEADE in our
 * range: PLACE_CHAN (auto steps past it); (g) with an END the walked list
 * and the old placement, the other channels not consulted; (h) NEXT 0 and an
 * entry past the memory still FW_WALK; (i) a changed word inside the window
 * named by the restore, one past it not compared.
 *
 * Hardware-written words (swap-hwword): the set's text form (parse, format,
 * refusals); an invalid set in cfg refused before any access; at the
 * restore, with the empty set a changed word 676 (entry 0 off 4) fails as
 * before and is named by a restore-diff line, with 4:skip it passes ("byte-
 * equal except hw words off 4"), with 4:expect it is reported, an undeclared
 * word beside it (677, off 5) fails named, the END word is in no entry, and
 * in a window every entry's offset 4 is covered; at our slot's read-back the
 * HVS writing word 36 (our entry off 4) back fails as LIST_READ with the
 * empty set and is skipped with 4:skip, off 5 still fails, our list END is
 * in no entry.
 *
 * Build: tools/v3d/check71.d/hvs6_dlist_swap.sh. Prints one line per check,
 * then "hvs6_dlist_swap_mock: N of N checks, 0 failed"; exits nonzero on a
 * failure.
 */
#include <stdio.h>
#include <string.h>

#include "hvs6_dlist_swap.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

/* ---- the channel model --------------------------------------------------- */

#define MEMW 4096u
#define LOGN 256u
#define CH 0u

enum lmode {
	L_NOW,   /* LPTRS reads back at once, LACT follows at the next edge */
	L_EDGE,  /* LPTRS and LACT both follow only at the next edge */
	L_NEVER, /* the register ignores writes */
};

enum ev { EV_WRITE, EV_CLEAN, EV_BARRIER };

struct event {
	enum ev kind;
	uint32_t off, val;
};

struct hvs {
	uint32_t mem[MEMW];
	uint32_t lptrs, pending, lact, ctrl1, frcnt;
	int has_pending;
	enum lmode lmode;
	int stall;            /* FRCNT never moves */
	uint32_t tpf;         /* polls per frame */
	uint32_t tick;
	int64_t corrupt_word; /* changed at the first edge showing our list */
	uint32_t our_start;
	struct event log[LOGN];
	uint32_t nlog;
	uint32_t ctrl1_writes, other_writes;
	uint32_t nreads, oob_reads; /* list-memory reads, and reads past MEMW */
	uint32_t olptrs[3];         /* LPTRS of the other channels (read only) */
	int64_t stick_word;         /* a write to this word stores stick_val (HVS write-back) */
	uint32_t stick_val;
};

static void logev(struct hvs *h, enum ev k, uint32_t off, uint32_t val)
{
	if (h->nlog < LOGN) {
		h->log[h->nlog].kind = k;
		h->log[h->nlog].off = off;
		h->log[h->nlog].val = val;
	}
	h->nlog++;
}

static uint32_t m_read(void *ctx, uint32_t off)
{
	struct hvs *h = ctx;

	if (off >= HVS6_DLIST_OFFSET) {
		uint32_t w = (off - HVS6_DLIST_OFFSET) / 4u;

		if (w >= MEMW) {
			h->oob_reads++;
			return 0;
		}
		h->nreads++;
		return h->mem[w];
	}
	if (off == hvs6s_lptrs_reg(CH))
		return h->lptrs;
	if (off == hvs6s_status_reg(CH))
		return (h->frcnt << 16) | (YD5_HVS6_STATUS_MODE_RUN << 13) | (h->tick & 0x7ffu);
	if (off == hvs6s_dl_reg(CH))
		return h->lact;
	if (off == hvs6s_ctrl1_reg(CH))
		return h->ctrl1;
	for (unsigned int ch = 0; ch < 3; ch++)
		if (ch != CH && off == hvs6s_lptrs_reg(ch))
			return h->olptrs[ch];
	return 0;
}

static void m_write(void *ctx, uint32_t off, uint32_t val)
{
	struct hvs *h = ctx;

	logev(h, EV_WRITE, off, val);
	if (off >= HVS6_DLIST_OFFSET) {
		uint32_t w = (off - HVS6_DLIST_OFFSET) / 4u;

		if (w < MEMW)
			h->mem[w] = (int64_t)w == h->stick_word ? h->stick_val : val;
		else
			h->other_writes++;
		return;
	}
	if (off == hvs6s_lptrs_reg(CH)) {
		if (h->lmode == L_NOW)
			h->lptrs = val;
		if (h->lmode != L_NEVER) {
			h->pending = val;
			h->has_pending = 1;
		}
		return;
	}
	if (off == hvs6s_ctrl1_reg(CH))
		h->ctrl1_writes++;
	else
		h->other_writes++;
}

static void m_clean(void *ctx, uint64_t addr, uint64_t len)
{
	logev(ctx, EV_CLEAN, (uint32_t)addr, (uint32_t)len);
}

static void m_barrier(void *ctx)
{
	logev(ctx, EV_BARRIER, 0, 0);
}

static void m_wait(void *ctx)
{
	struct hvs *h = ctx;

	h->tick++;
	if (h->stall || h->tick % h->tpf)
		return;
	h->frcnt = (h->frcnt + 1u) & 63u;
	if (h->has_pending) {
		h->lptrs = h->pending;
		h->has_pending = 0;
	}
	h->lact = h->lptrs & 0xfffu;
	if (h->corrupt_word >= 0 && h->lact == h->our_start) {
		h->mem[h->corrupt_word] ^= 0x00010000u;
		h->corrupt_word = -1;
	}
}

/* The firmware's state: a 10-word Linux-layout list at fw_head, a known
 * pattern in the rest of words 0..31, every other word zero, LPTRS =
 * fw_head. */
static void model_init(struct hvs *h, uint32_t fw_head)
{
	struct hvs6w_plane p = { HVS6W_RGB565, 0, 0, 1920, 1080, 3840, 0x3e000000ull };

	memset(h, 0, sizeof(*h));
	for (uint32_t i = 0; i < 32; i++)
		h->mem[i] = 0xdead0000u | i;
	(void)hvs6w_write_plane(&h->mem[fw_head], HVS6W_LIST_WORDS, &p);
	h->lptrs = fw_head;
	h->lact = fw_head;
	h->ctrl1 = YD5_HVS6_CTRL1_BGENB;
	h->frcnt = 10;
	h->tpf = 3;
	h->corrupt_word = -1;
	h->our_start = 0xffffffffu;
	h->stick_word = -1;
}

static struct hvs6s_io model_io(struct hvs *h)
{
	struct hvs6s_io io = { m_read, m_write, m_clean, m_barrier, m_wait, h };

	return io;
}

static struct hvs6s_cfg base_cfg(void)
{
	struct hvs6s_cfg c;

	memset(&c, 0, sizeof(c));
	c.chan = CH;
	c.mem_words = MEMW;
	c.start = HVS6S_AUTO;
	c.list_arm = 0x107c004000ull; /* HVS + 0x4000, a register address */
	c.cache_line = 64;
	c.max_polls = 40;
	c.plane = (struct hvs6w_plane){ HVS6W_XRGB8888, 0, 0, 1920, 1080, 7680, 0x20000000ull };
	return c;
}

/* ---- log helpers -------------------------------------------------------- */

static int count_writes(const struct hvs *h, uint32_t off)
{
	int n = 0;

	for (uint32_t i = 0; i < h->nlog && i < LOGN; i++)
		if (h->log[i].kind == EV_WRITE && h->log[i].off == off)
			n++;
	return n;
}

static int64_t first_write(const struct hvs *h, uint32_t off)
{
	for (uint32_t i = 0; i < h->nlog && i < LOGN; i++)
		if (h->log[i].kind == EV_WRITE && h->log[i].off == off)
			return i;
	return -1;
}

/* Every write is list memory in [start, start+len) or LPTRS; nothing else,
 * CTRL1 above all. */
static int writes_confined(const struct hvs *h, uint32_t start, uint32_t len)
{
	if (h->ctrl1_writes || h->other_writes || h->nlog > LOGN)
		return 0;
	for (uint32_t i = 0; i < h->nlog; i++) {
		const struct event *e = &h->log[i];

		if (e->kind != EV_WRITE || e->off == hvs6s_lptrs_reg(CH))
			continue;
		if (e->off < hvs6s_word_off(start) || e->off >= hvs6s_word_off(start + len))
			return 0;
	}
	return 1;
}

static void sink(void *ctx, const char *t)
{
	(void)ctx;
	printf("        %s\n", t);
}

static void print_record(const struct hvs6s_snap *s, const struct hvs6s_report *r)
{
	struct hvs6s_emit em = { sink, NULL };

	(void)hvs6s_lines(s, r, &em);
}

static void name_check(const char *tag, int ok, const char *what)
{
	char b[160];

	(void)snprintf(b, sizeof(b), "%s: %s", tag, what);
	check(ok, b);
}

/* ---- cases ---------------------------------------------------------------- */

static void case_pass(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	uint32_t want[HVS6W_LIST_WORDS];
	int64_t lp, lastlist = -1, clean = -1, bar = -1;
	const char *t = "pass";

	model_init(&h, 0);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_OK && r.err == HVS6S_OK, "run returns ok");
	name_check(t, s.taken && s.lptrs == 0 && s.fw_head == 0 && s.fw_len == 10 && s.fw_end_found,
		   "snapshot: LPTRS 0, firmware list 0..9 walked to its END by NEXT");
	name_check(t, r.start == 32 && r.len == 10, "placed at word 32, 10 words");
	(void)hvs6w_write_plane(want, HVS6W_LIST_WORDS, &c.plane);
	name_check(t, memcmp(&h.mem[32], want, sizeof(want)) == 0,
		   "list memory 32..41 holds hvs6_dlist_write's 10 words");
	name_check(t, want[0] >> 24 == 0x49u && want[2] == 0xfff0u && want[8] == HVS6_CTL0_END &&
		      want[9] == HVS6_CTL0_END,
		   "entry: VALID|NEXT=9, CTL2 0xfff0, inner END and list END");
	name_check(t, r.lptrs_written == 32 && r.lptrs_ok && r.lptrs_poll == 0 && r.lptrs_first == 32,
		   "result 1: LPTRS read back 32 at once");
	name_check(t, r.frcnt_ok && r.frcnt_polls > 0 && r.frcnt_polls <= 3,
		   "result 2: FRCNT advanced within one frame of polls");
	name_check(t, r.settle_ok, "settle: two advances past the write");
	name_check(t, hvs6s_frcnt(r.at_write.status) == 10 && hvs6s_mode(r.at_write.status) == 2 &&
		      r.at_write.lptrs == 0 && hvs6s_bgenb(r.at_write.ctrl1) == 1,
		   "at write: FRCNT 10, MODE RUN, LPTRS 0, BGENB 1 recorded");
	name_check(t, hvs6s_frcnt(r.after.status) == 11 && r.after.lptrs == 32,
		   "after: FRCNT 11, LPTRS 32 recorded");
	name_check(t, r.lact_follows, "LACT followed (record only)");
	name_check(t, r.list_bus == 0x107c004080ull, "bus address of word 32 recorded");
	name_check(t, r.cleaned.start == 0x107c004080ull && r.cleaned.end == 0x107c0040c0ull,
		   "cleaned the line-rounded 40 bytes of the list");

	lp = first_write(&h, hvs6s_lptrs_reg(CH));
	for (uint32_t i = 0; i < h.nlog; i++) {
		if (h.log[i].kind == EV_WRITE && h.log[i].off >= HVS6_DLIST_OFFSET)
			lastlist = i;
		if (h.log[i].kind == EV_CLEAN && clean < 0)
			clean = i;
		if (h.log[i].kind == EV_BARRIER && bar < 0)
			bar = i;
	}
	name_check(t, lastlist >= 0 && clean > lastlist && bar > clean && lp > bar,
		   "order: list words, clean, barrier, then LPTRS last");
	name_check(t, h.log[lp].val == 32, "the first LPTRS write is our start, as one write");
	name_check(t, count_writes(&h, hvs6s_lptrs_reg(CH)) == 2,
		   "LPTRS written twice in all: the swap and the restore");
	name_check(t, writes_confined(&h, 32, 10), "writes only to our list and LPTRS");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");
	name_check(t, r.restored && r.restore_frcnt_ok && r.restore_lptrs_ok && r.restore_equal &&
		      r.diff_word == -1 && h.lptrs == 0,
		   "restore: LPTRS 0 back, two advances, words equal");
}

static void case_edge(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "edge";

	model_init(&h, 0);
	h.lmode = L_EDGE;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_OK, "run returns ok");
	name_check(t, r.lptrs_first == 0, "LPTRS read at once is still the firmware's 0");
	name_check(t, r.lptrs_ok && r.lptrs_poll > 0 && r.lptrs_poll == r.frcnt_polls,
		   "LPTRS reads back at the poll where FRCNT moved");
	name_check(t, r.lact_follows && r.after.lptrs == 32, "LACT and LPTRS follow at the edge");
	name_check(t, r.settle_ok && r.restore_equal && h.lptrs == 0,
		   "settle and restore pass; LPTRS 0 after the restore's edge");
	name_check(t, h.ctrl1_writes == 0 && writes_confined(&h, 32, 10), "CTRL1 never written");
}

static void case_never(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "lptrs-never";

	model_init(&h, 0);
	h.lmode = L_NEVER;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_LPTRS_READ && r.err == HVS6S_ERR_LPTRS_READ,
		   "fails as HVS6S_ERR_LPTRS_READ");
	name_check(t, strcmp(hvs6s_err_name(r.err), "LPTRS did not read back") == 0,
		   "named 'LPTRS did not read back'");
	name_check(t, !r.lptrs_ok && r.frcnt_ok, "result 1 failed, result 2 (FRCNT) passed on its own");
	name_check(t, r.restored && r.restore_equal, "the restore still ran and compared equal");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");
}

static void case_stall(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "frcnt-stall";

	model_init(&h, 0);
	h.stall = 1;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_FRCNT, "fails as HVS6S_ERR_FRCNT");
	name_check(t, strcmp(hvs6s_err_name(r.err), "FRCNT did not advance") == 0,
		   "named 'FRCNT did not advance'");
	name_check(t, r.lptrs_ok && !r.frcnt_ok && r.frcnt_polls == c.max_polls,
		   "LPTRS read back; FRCNT timed out after max_polls");
	name_check(t, !r.settle_ok, "no settle claimed");
	name_check(t, r.restored && !r.restore_frcnt_ok && r.restore_polls == c.max_polls,
		   "the restore ran; its FRCNT wait timed out too");
	name_check(t, r.restore_lptrs_ok && r.restore_equal,
		   "the restore still compared: LPTRS and words equal");
	name_check(t, h.tick == 2u * c.max_polls,
		   "every wait bounded: max_polls in the swap, max_polls in the restore");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");
}

static void case_wrap(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "wrap";

	model_init(&h, 0);
	h.frcnt = 62;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_OK, "run returns ok across 63 -> 0");
	name_check(t, hvs6s_frcnt(r.at_write.status) == 62 && hvs6s_frcnt(r.after.status) == 63,
		   "write at 62, advanced to 63");
	name_check(t, r.settle_ok && h.frcnt != 62, "settle counted through 63 -> 0");
	name_check(t, hvs6s_frdelta(63, 0) == 1 && hvs6s_frdelta(62, 1) == 3 && hvs6s_frdelta(5, 5) == 0,
		   "frdelta: 63->0 is 1, 62->1 is 3, 5->5 is 0");
	name_check(t, r.restore_equal && h.ctrl1_writes == 0, "restore equal, CTRL1 never written");

	/* the wrap inside the restore's wait */
	model_init(&h, 0);
	h.frcnt = 61;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.restore_frcnt_ok && hvs6s_frcnt(r.after_restore.status) <= 2,
		   "restore's two advances counted across the wrap");
}

static void case_place_kept(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "place-kept";

	model_init(&h, 0);
	io = model_io(&h);
	c.start = 24; /* 24..33 */
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_KEPT && r.place_err == HVS6S_ERR_PLACE_KEPT,
		   "start 24 refused as HVS6S_ERR_PLACE_KEPT");
	name_check(t, strcmp(hvs6s_err_name(e), "place: overlaps words 0..31") == 0,
		   "named 'place: overlaps words 0..31'");
	name_check(t, !r.swapped && count_writes(&h, hvs6s_lptrs_reg(CH)) == 1 &&
		      writes_confined(&h, 0, 0),
		   "no list word written; the only write is the restore's LPTRS");
	name_check(t, r.restored && r.restore_equal, "the restore ran anyway and compared equal");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");
	name_check(t, hvs6s_check_span(&c, &s, 31, 1) == HVS6S_ERR_PLACE_KEPT &&
		      hvs6s_check_span(&c, &s, 32, 1) == HVS6S_OK,
		   "word 31 refused, word 32 accepted");
}

static void case_place_fw(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "place-fw";

	/* the firmware's list at 40..49 */
	model_init(&h, 40);
	io = model_io(&h);
	c.start = 45;
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, s.fw_head == 40 && s.fw_len == 10, "snapshot: firmware list 40..49");
	name_check(t, e == HVS6S_ERR_PLACE_FW, "start 45 refused as HVS6S_ERR_PLACE_FW");
	name_check(t, strcmp(hvs6s_err_name(e), "place: overlaps the firmware's list") == 0,
		   "named 'place: overlaps the firmware's list'");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && h.ctrl1_writes == 0,
		   "nothing but the restore's LPTRS written; CTRL1 never written");

	model_init(&h, 40);
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 50, "auto placement steps over it to word 50");
	name_check(t, r.restore_equal && h.lptrs == 40 && h.ctrl1_writes == 0,
		   "restore gives LPTRS 40 back; CTRL1 never written");
}

static void case_fit(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "fit";

	model_init(&h, 0);
	io = model_io(&h);
	c.mem_words = 40; /* 32..39 is 8 words, the list needs 10 */
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FIT, "auto in 40 words refused as HVS6S_ERR_PLACE_FIT");
	name_check(t, strcmp(hvs6s_err_name(e), "place: does not fit") == 0, "named 'place: does not fit'");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && h.ctrl1_writes == 0,
		   "nothing but the restore's LPTRS written; CTRL1 never written");

	model_init(&h, 0);
	io = model_io(&h);
	c.mem_words = MEMW;
	c.start = MEMW - 5;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FIT, "start 4091 (runs past 4096) refused");
	name_check(t, hvs6s_check_span(&c, &s, 4086, 10) == HVS6S_OK &&
		      hvs6s_check_span(&c, &s, 4087, 10) == HVS6S_ERR_PLACE_FIT,
		   "4086..4095 fits, 4087..4096 does not");
	c.mem_words = 8192;
	name_check(t, hvs6s_check_span(&c, &s, 4096, 10) == HVS6S_ERR_PLACE_FIT,
		   "start 4096 refused: over the 12-bit HEADE");
}

static void case_restore_word(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	struct hvs6s_emit em = { sink, NULL };
	enum hvs6s_err e;
	const char *t = "restore-word";

	uint32_t w3;

	model_init(&h, 0);
	w3 = h.mem[3];
	h.corrupt_word = 3;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	(void)hvs6s_lines(&s, &r, &em);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD, "fails as HVS6S_ERR_RESTORE_WORD");
	name_check(t, r.lptrs_ok && r.frcnt_ok && r.settle_ok, "the swap itself passed");
	name_check(t, r.diff_word == 3 && r.diff_want == w3 && r.diff_got == (w3 ^ 0x00010000u),
		   "names word 3, with the snapshot's and the read value");
	name_check(t, r.restore_lptrs_ok && !r.restore_equal, "LPTRS equal, words not");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");

	/* word 17 is outside the firmware's list 0..9: not in the snapshot */
	model_init(&h, 0);
	h.corrupt_word = 17;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.diff_word == -1 && h.mem[17] != (0xdead0000u | 17u),
		   "a changed word outside [HEADE, HEADE + fw_len) is not compared");

	/* a word of the firmware's own list past 31 */
	model_init(&h, 40);
	h.corrupt_word = 43;
	h.our_start = 50;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 43,
		   "a changed word of the firmware's list (43) is named too");
}

static void case_args(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;

	model_init(&h, 0);
	io = model_io(&h);
	io.wait = NULL;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0,
	      "args: no wait callback refused before any access");
	io = model_io(&h);
	c.chan = 3;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0, "args: channel 3 refused");
	c = base_cfg();
	c.max_polls = 0;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG, "args: max_polls 0 refused");
	c = base_cfg();
	c.mem_words = 1024;
	h.lptrs = 2000;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_SNAPSHOT && h.nlog == 0,
	      "snapshot: LPTRS past the memory refused, nothing written");
	model_init(&h, 0);
	io = model_io(&h);
	c = base_cfg();
	c.plane.width = 0;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_PLANE && !r.swapped && r.restore_equal &&
	      writes_confined(&h, 0, 0),
	      "plane refused by hvs6_dlist_write: nothing written, restore ran");
	model_init(&h, 0);
	io = model_io(&h);
	c = base_cfg();
	c.list_arm = 1ull << 40;
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_BUS && !r.swapped && writes_confined(&h, 0, 0),
	      "list address over 40 bits refused before any list write");
	/* a firmware list of 1-word entries at HEADE 0 without an END in reach:
	 * the 256-word window, and no room before HEADE 0 */
	model_init(&h, 0);
	for (uint32_t i = 0; i < 300; i++)
		h.mem[i] = HVS6_CTL0_VALID | (1u << HVS6_CTL0_NEXT_SHIFT);
	io = model_io(&h);
	c = base_cfg();
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_PLACE_NOEND && s.taken && s.fw_noend &&
	      s.fw_entries == 256 && s.fw_len == 256 && !r.swapped && writes_confined(&h, 0, 0) &&
	      r.restore_equal,
	      "1-word entries without END at HEADE 0: window 0..255 kept, no room before HEADE, "
	      "nothing but the restore's LPTRS written");
	check(hvs6s_snapshot(&io, &c, NULL) == HVS6S_ERR_ARG, "args: snapshot without a snap refused");
}

/* ---- the board's layout: the firmware's list at word 672 ------------------ */

#define BHEAD 672u /* LPTRS = DL = 0x2a0 on the Pi 5 D0 (s32dumprun) */

/* The firmware plane at head as the board shows it: RGB565 1920x1080, pitch
 * 3840, NEXT=32; words 10..31 of the entry non-zero filler; with end, the
 * list END at head + 32. LPTRS = LACT = head. */
static void model_board(struct hvs *h, uint32_t head, int end)
{
	struct hvs6w_plane p = { HVS6W_RGB565, 0, 0, 1920, 1080, 3840, 0x3f800000ull };

	model_init(h, 0);
	for (uint32_t i = 0; i < 10; i++) /* no list at word 0 */
		h->mem[i] = 0xdead0000u | i;
	if (head + 32u < MEMW) {
		(void)hvs6w_write_plane(&h->mem[head], HVS6W_LIST_WORDS, &p);
		h->mem[head] = (h->mem[head] & ~(HVS6_CTL0_NEXT_MASK << HVS6_CTL0_NEXT_SHIFT)) |
			       (32u << HVS6_CTL0_NEXT_SHIFT);
		for (uint32_t i = 10; i < 32; i++)
			h->mem[head + i] = 0xc0de0000u | i;
		if (end)
			h->mem[head + 32u] = HVS6_CTL0_END;
	}
	h->lptrs = head;
	h->lact = head;
}

static void case_board_672(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	uint32_t want[HVS6W_LIST_WORDS];
	int same = 1;
	const char *t = "heade672";

	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_OK && r.err == HVS6S_OK, "run returns ok");
	name_check(t, s.taken && s.lptrs == BHEAD && s.fw_head == BHEAD && s.fw_end_found,
		   "snapshot: the firmware's list starts at LPTRS.HEADE 672, not 0");
	name_check(t, s.fw_entries == 1 && s.fw_len == 33 && s.walk_stop == BHEAD + 32u &&
		      s.walk_word == HVS6_CTL0_END,
		   "walk: one entry of NEXT=32, the list END at 704, 33 words");
	for (uint32_t i = 0; i < 33; i++)
		if (s.fw[i] != h.mem[BHEAD + i])
			same = 0;
	name_check(t, same, "the snapshot holds words 672..704 exactly");
	name_check(t, r.start == 32 && r.len == 10 && r.place_skipped == 0,
		   "placed at word 32, zero words 32..41, 10 words");
	(void)hvs6w_write_plane(want, HVS6W_LIST_WORDS, &c.plane);
	name_check(t, memcmp(&h.mem[32], want, sizeof(want)) == 0 && want[0] >> 24 == 0x49u &&
		      want[9] == HVS6_CTL0_END,
		   "our 10 words (NEXT=9 and the list END) at 32..41");
	name_check(t, r.lptrs_written == 32 && r.lptrs_ok && r.frcnt_ok && r.settle_ok,
		   "swap: LPTRS 32 read back, FRCNT advanced, settled");
	name_check(t, writes_confined(&h, 32, 10) && count_writes(&h, hvs6s_lptrs_reg(CH)) == 2,
		   "writes only to 32..41 and LPTRS (swap and restore)");
	name_check(t, r.restored && r.restore_lptrs_ok && r.restore_equal && r.diff_word == -1 &&
		      h.lptrs == BHEAD,
		   "restore: LPTRS 672 back, words 672..704 byte-equal");
	name_check(t, h.oob_reads == 0 && h.ctrl1_writes == 0, "no read past the memory; CTRL1 never written");

	/* right after the END */
	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	c.start = BHEAD + 33u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 705 && writes_confined(&h, 705, 10) &&
		      r.restore_equal && h.lptrs == BHEAD,
		   "caller's start 705, right after the END: swap and byte-equal restore");

	/* the END word changed while ours was shown */
	model_board(&h, BHEAD, 1);
	h.corrupt_word = BHEAD + 32u;
	h.our_start = 32;
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 704 && r.diff_want == HVS6_CTL0_END,
		   "a changed END word (704) is named by the restore");
}

static void case_board_noend(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "noend";

	/* (h) the word after the firmware entry is zero: NEXT 0, no END */
	model_board(&h, BHEAD, 0);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_FW_WALK && s.walk_stop == 704 && s.walk_word == 0 &&
		      s.fw_entries == 1 && h.nlog == 0,
		   "a zero word at 704 (NEXT 0, no END): refused as HVS6S_ERR_FW_WALK at 704");
	name_check(t, strcmp(hvs6s_err_name(e), "snapshot: entry with NEXT 0 or past the list memory") == 0,
		   "named 'snapshot: entry with NEXT 0 or past the list memory'");

	/* an entry that runs past the list memory */
	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	c.mem_words = BHEAD + 20u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_FW_WALK && h.nlog == 0 && h.oob_reads == 0,
		   "NEXT=32 past a 692-word memory: refused, no read past it");
}

/* A collecting sink, to check the lines of a refused snapshot. */
static char lines_buf[32][256];
static size_t lines_n;

static void collect(void *ctx, const char *text)
{
	(void)ctx;
	if (lines_n < 32)
		(void)snprintf(lines_buf[lines_n], sizeof(lines_buf[0]), "%s", text);
	lines_n++;
	printf("        %s\n", text);
}

static int has_line(const char *prefix)
{
	for (size_t i = 0; i < lines_n && i < 32; i++)
		if (strncmp(lines_buf[i], prefix, strlen(prefix)) == 0)
			return 1;
	return 0;
}

static void case_noend_lines(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	struct hvs6s_emit em = { collect, NULL };
	size_t n;
	const char *t = "noend-lines";

	model_board(&h, BHEAD, 0);
	io = model_io(&h);
	(void)hvs6s_run(&io, &c, &s, &r);
	lines_n = 0;
	n = hvs6s_lines(&s, &r, &em);
	name_check(t, n == 5 && lines_n == 5, "five lines after a refused snapshot");
	name_check(t, has_line("dlswap snapshot FAIL snapshot: entry with NEXT 0") &&
		      has_line("dlswap place skip") && has_line("dlswap write skip") &&
		      has_line("dlswap restore skip") && has_line("dlswap result FAIL"),
		   "snapshot FAIL, then place, write and restore skip, then the result");
	name_check(t, strstr(lines_buf[0], "fw_head=672") && strstr(lines_buf[0], "stop=704") &&
		      strstr(lines_buf[0], "word=0x00000000"),
		   "the snapshot line names HEADE 672 and where the walk stopped");
}

static void case_place_used(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "place-used";

	model_board(&h, BHEAD, 1);
	h.mem[36] = 0x12345678u;
	io = model_io(&h);
	c.start = 32; /* 32..41 holds word 36 */
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_USED && r.place_err == HVS6S_ERR_PLACE_USED,
		   "start 32 over a non-zero word refused as HVS6S_ERR_PLACE_USED");
	name_check(t, strcmp(hvs6s_err_name(e), "place: target range holds a non-zero word") == 0,
		   "named 'place: target range holds a non-zero word'");
	name_check(t, r.place_bad_word == 36 && r.place_bad_val == 0x12345678u,
		   "names word 36 and its value");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && h.mem[36] == 0x12345678u &&
		      count_writes(&h, hvs6s_lptrs_reg(CH)) == 1,
		   "no list word written; the only write is the restore's LPTRS");
	name_check(t, r.restored && r.restore_equal && h.lptrs == BHEAD,
		   "the restore ran and compared equal");

	model_board(&h, BHEAD, 1);
	h.mem[36] = 0x12345678u;
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 37 && r.place_skipped == 1 &&
		      writes_confined(&h, 37, 10),
		   "auto steps past word 36 to 37");

	/* everything from 32 to the firmware's list in use: auto goes past it */
	model_board(&h, BHEAD, 1);
	for (uint32_t i = 32; i < BHEAD; i++)
		h.mem[i] = 0x5a000000u | i;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == BHEAD + 33u && writes_confined(&h, 705, 10) &&
		      r.restore_equal,
		   "words 32..671 in use: auto places after the END, at 705");
}

static void case_place_overlap(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "place-overlap";
	static const uint32_t bad[] = { 663, 672, 690, 695, 704 };

	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	c.start = 700;
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FW, "start 700 (inside 672..704) refused as HVS6S_ERR_PLACE_FW");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && r.restore_equal,
		   "nothing but the restore's LPTRS written; restore equal");
	for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
		char b[96];

		(void)snprintf(b, sizeof(b), "check_span start %u refused (overlaps 672..704)",
			       (unsigned)bad[i]);
		name_check(t, hvs6s_check_span(&c, &s, bad[i], 10) == HVS6S_ERR_PLACE_FW, b);
	}
	name_check(t, hvs6s_check_span(&c, &s, 662, 10) == HVS6S_OK &&
		      hvs6s_check_span(&c, &s, 705, 10) == HVS6S_OK,
		   "662..671 and 705..714, just beside it, accepted");
}

static void case_place_nofit(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "place-nofit";

	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	c.start = MEMW - 6u; /* 4090..4099 */
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FIT && !r.swapped && writes_confined(&h, 0, 0),
		   "start 4090 with 10 words in 4096: refused as HVS6S_ERR_PLACE_FIT");

	/* every free word in use: no room anywhere */
	model_board(&h, BHEAD, 1);
	for (uint32_t i = 32; i < MEMW; i++)
		if (i < BHEAD || i > BHEAD + 32u)
			h.mem[i] = 0x5a000000u | i;
	/* a 9-word hole is still too small */
	for (uint32_t i = 3000; i < 3009; i++)
		h.mem[i] = 0;
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FIT && !r.swapped && writes_confined(&h, 0, 0) &&
		      r.restore_equal && h.oob_reads == 0,
		   "auto with no 10-word zero range (one 9-word hole): refused, nothing written");
}

/* ---- no END in view: the fixed window and placement before HEADE -------- */

/* The board's firmware entry at head (NEXT=32, no END after it), followed by
 * n - 1 more entry headers of NEXT=32, none of them END. */
static void model_noend(struct hvs *h, uint32_t head, uint32_t n)
{
	model_board(h, head, 0);
	for (uint32_t k = 1; k < n && head + 32u * k < MEMW; k++)
		h->mem[head + 32u * k] = h->mem[head];
}

/* (a) the window clamped at 256 words, and (c) our list before HEADE. */
static void case_noend_window(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	uint32_t want[HVS6W_LIST_WORDS];
	int same = 1;
	const char *t = "noend-window";

	model_noend(&h, BHEAD, 12);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, s.taken && s.fw_noend && !s.fw_end_found && s.fw_head == BHEAD,
		   "(a) no END within 256 words: snapshot taken as a window from HEADE 672");
	name_check(t, s.fw_len == HVS6S_FW_MAX && s.fw_entries == 8 && s.walk_stop == BHEAD + 224u &&
		      s.walk_word == h.mem[BHEAD],
		   "(a) window 672..927, clamped at 256 words; 8 entries walked, last at 896");
	for (uint32_t i = 0; i < s.fw_len; i++)
		if (s.fw[i] != h.mem[BHEAD + i])
			same = 0;
	name_check(t, same, "(a) the snapshot holds words 672..927 exactly");
	name_check(t, h.oob_reads == 0, "(a) no read past the memory");
	name_check(t, e == HVS6S_OK && r.err == HVS6S_OK, "(c) run returns ok");
	name_check(t, r.place_err == HVS6S_OK && r.start == 32 && r.start + r.len <= BHEAD,
		   "(c) placed at word 32, wholly before HEADE");
	(void)hvs6w_write_plane(want, HVS6W_LIST_WORDS, &c.plane);
	name_check(t, memcmp(&h.mem[32], want, sizeof(want)) == 0 && writes_confined(&h, 32, 10) &&
		      count_writes(&h, hvs6s_lptrs_reg(CH)) == 2,
		   "(c) our 10 words at 32..41; writes only there and LPTRS (swap, restore)");
	name_check(t, r.lptrs_ok && r.frcnt_ok && r.settle_ok, "(c) swap: read back, advanced, settled");
	name_check(t, r.restored && r.restore_lptrs_ok && r.restore_equal && r.diff_word == -1 &&
		      h.lptrs == BHEAD,
		   "(c) restore: LPTRS 672 back, window 672..927 byte-equal");
	name_check(t, h.ctrl1_writes == 0, "(c) CTRL1 never written");

	/* (c) a caller's start just before HEADE: 662..671 */
	model_noend(&h, BHEAD, 12);
	io = model_io(&h);
	c.start = BHEAD - 10u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 662 && writes_confined(&h, 662, 10) && r.restore_equal,
		   "(c) caller's start 662, ending at HEADE: swap and byte-equal restore");

	/* (g) the same words with an END at 704: the walked list, as before */
	model_board(&h, BHEAD, 1);
	for (uint32_t k = 2; k < 12; k++)
		h.mem[BHEAD + 32u * k] = h.mem[BHEAD];
	io = model_io(&h);
	c.start = BHEAD + 33u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && !s.fw_noend && s.fw_end_found && s.fw_len == 33 &&
		      r.start == 705,
		   "(g) END at 704: walked list of 33 words, a start after it still accepted");
}

/* (b) the window clamped to the memory. */
static void case_noend_memend(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "noend-memend";

	/* HEADE 4032, entries at 4032 and 4064 fill the memory to 4095 */
	model_noend(&h, MEMW - 64u, 2);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, s.taken && s.fw_noend && s.fw_head == MEMW - 64u && s.fw_len == 64u &&
		      s.fw_entries == 2,
		   "(b) HEADE 4032: window clamped to mem_words, 4032..4095 (64 words)");
	name_check(t, h.oob_reads == 0, "(b) no read past the memory");
	name_check(t, e == HVS6S_OK && r.start == 32 && r.restore_equal && h.lptrs == MEMW - 64u,
		   "(b) placed at 32, swap and byte-equal restore");

	/* a smaller mem_words: HEADE 672 with 64 words left */
	model_noend(&h, BHEAD, 2);
	io = model_io(&h);
	c.mem_words = BHEAD + 64u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, s.taken && s.fw_noend && s.fw_len == 64u && e == HVS6S_OK && r.start == 32 &&
		      r.restore_equal && h.oob_reads == 0,
		   "(b) mem_words 736: window 672..735, placed at 32, restore equal");

	/* an entry past the memory is still FW_WALK, not a window */
	model_noend(&h, BHEAD, 2);
	io = model_io(&h);
	c.mem_words = BHEAD + 50u;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_FW_WALK && !s.taken && h.nlog == 0,
		   "(h) entry at 704 running past a 722-word memory: still FW_WALK, nothing written");
}

/* (d) no room before HEADE, and (e) a non-zero word before HEADE. */
static void case_noend_place(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "noend-place";

	/* HEADE 40: words 32..39 are 8, our list needs 10 */
	model_noend(&h, 40, 8);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, s.taken && s.fw_noend && s.fw_len == 256 && e == HVS6S_ERR_PLACE_NOEND &&
		      r.place_err == HVS6S_ERR_PLACE_NOEND,
		   "(d) HEADE 40, no END: auto refused as HVS6S_ERR_PLACE_NOEND");
	name_check(t, strcmp(hvs6s_err_name(e), "place: no END, no room wholly before HEADE") == 0,
		   "(d) named 'place: no END, no room wholly before HEADE'");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && count_writes(&h, hvs6s_lptrs_reg(CH)) == 1 &&
		      r.restored && r.restore_equal && h.lptrs == 40,
		   "(d) no list word written; the restore's LPTRS only, equal");
	c.start = 32;
	model_noend(&h, 40, 8);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FW && writes_confined(&h, 0, 0),
		   "(d) caller's start 32 (32..41 crosses HEADE 40 into the window): PLACE_FW");
	name_check(t, hvs6s_check_span(&c, &s, 32, 8) == HVS6S_OK &&
		      hvs6s_check_span(&c, &s, 296, 10) == HVS6S_ERR_PLACE_NOEND,
		   "(d) check_span: 32..39 fits before HEADE 40, 296..305 after the window refused");

	/* HEADE 672: a start after the window is refused too (no END: unknown) */
	model_noend(&h, BHEAD, 12);
	io = model_io(&h);
	c.start = 1000;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_NOEND && !r.swapped && writes_confined(&h, 0, 0),
		   "(d) caller's start 1000, after the window: refused as HVS6S_ERR_PLACE_NOEND");
	c.start = 700;
	model_noend(&h, BHEAD, 12);
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_FW && writes_confined(&h, 0, 0),
		   "(d) caller's start 700, inside the window: refused as HVS6S_ERR_PLACE_FW");
	name_check(t, hvs6s_check_span(&c, &s, 662, 10) == HVS6S_OK &&
		      hvs6s_check_span(&c, &s, 663, 10) == HVS6S_ERR_PLACE_FW &&
		      hvs6s_check_span(&c, &s, 928, 10) == HVS6S_ERR_PLACE_NOEND,
		   "(d) check_span: 662..671 ok, 663..672 overlaps, 928..937 after the window refused");
	{
		uint32_t st = 0;

		c.start = HVS6S_AUTO;
		name_check(t, hvs6s_place(&c, &s, 10, &st) == HVS6S_OK && st == 32,
			   "(d) geometry-only hvs6s_place: word 32");
		name_check(t, hvs6s_place(&c, &s, 700, &st) == HVS6S_ERR_PLACE_NOEND,
			   "(d) geometry-only hvs6s_place: 700 words do not fit before 672");
	}

	/* (e) a non-zero word before HEADE */
	model_noend(&h, BHEAD, 12);
	h.mem[36] = 0x12345678u;
	io = model_io(&h);
	c.start = 32;
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_USED && r.place_bad_word == 36 &&
		      r.place_bad_val == 0x12345678u && !r.swapped && writes_confined(&h, 0, 0) &&
		      r.restore_equal,
		   "(e) no END, word 36 non-zero, start 32: PLACE_USED naming word 36, nothing written");
	model_noend(&h, BHEAD, 12);
	h.mem[36] = 0x12345678u;
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 37 && r.place_skipped == 1 &&
		      writes_confined(&h, 37, 10) && r.restore_equal,
		   "(e) auto steps past word 36 to 37, still before HEADE");
	/* every word 32..671 in use: no END, so not after the window either */
	model_noend(&h, BHEAD, 12);
	for (uint32_t i = 32; i < BHEAD; i++)
		h.mem[i] = 0x5a000000u | i;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_NOEND && !r.swapped && writes_confined(&h, 0, 0) &&
		      r.restore_equal,
		   "(e) words 32..671 in use: PLACE_NOEND, not placed after the window");
}

/* (f) another channel's HEADE inside our range. */
static void case_noend_chan(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	struct hvs6s_emit em = { collect, NULL };
	enum hvs6s_err e;
	const char *t = "noend-chan";

	model_noend(&h, BHEAD, 12);
	h.olptrs[1] = 36;
	io = model_io(&h);
	c.start = 32;
	e = hvs6s_run(&io, &c, &s, &r);
	lines_n = 0;
	(void)hvs6s_lines(&s, &r, &em);
	name_check(t, e == HVS6S_ERR_PLACE_CHAN && r.place_err == HVS6S_ERR_PLACE_CHAN &&
		      r.place_chan == 1 && r.place_bad_word == 36,
		   "(f) channel 1's HEADE 36 inside 32..41: refused as HVS6S_ERR_PLACE_CHAN, ch 1");
	name_check(t, strcmp(hvs6s_err_name(e), "place: another channel's LPTRS.HEADE in range") == 0,
		   "(f) named 'place: another channel's LPTRS.HEADE in range'");
	name_check(t, !r.swapped && writes_confined(&h, 0, 0) && r.restore_equal && h.lptrs == BHEAD,
		   "(f) nothing but the restore's LPTRS written; restore equal");
	name_check(t, has_line("dlswap snapshot ok") && strstr(lines_buf[0], "window=noend") &&
		      strstr(lines_buf[0], "fw_len=256") && has_line("dlswap place FAIL place: another") &&
		      strstr(lines_buf[1], "ch=1 heade=36"),
		   "(f) lines: snapshot ok with window=noend, place FAIL naming ch=1 heade=36");

	model_noend(&h, BHEAD, 12);
	h.olptrs[2] = 0x1000u | 40u; /* bits above HEADE ignored */
	io = model_io(&h);
	c.start = 32;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_PLACE_CHAN && r.place_chan == 2 && r.place_bad_word == 40 &&
		      r.place_bad_val == (0x1000u | 40u),
		   "(f) channel 2's HEADE 40 inside 32..41: refused, ch 2, its LPTRS named");

	model_noend(&h, BHEAD, 12);
	h.olptrs[1] = 36;
	h.olptrs[2] = 40;
	io = model_io(&h);
	c.start = HVS6S_AUTO;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 41 && writes_confined(&h, 41, 10) && r.restore_equal,
		   "(f) auto steps past channel 1's 36 and channel 2's 40 to word 41");

	/* (g) with an END found the other channels are not consulted */
	model_board(&h, BHEAD, 1);
	h.olptrs[1] = 36;
	io = model_io(&h);
	c.start = 32;
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.start == 32 && !s.fw_noend,
		   "(g) END at 704: start 32 accepted as before (no other-channel check)");
}

/* (i) a changed word inside the window, found by the restore. */
static void case_noend_restore(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	uint32_t w;
	const char *t = "noend-restore";

	model_noend(&h, BHEAD, 12);
	w = h.mem[BHEAD + 200u];
	h.corrupt_word = BHEAD + 200u;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	print_record(&s, &r);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.lptrs_ok && r.settle_ok && r.restore_lptrs_ok &&
		      !r.restore_equal,
		   "(i) a changed word in the window: RESTORE_WORD after a good swap");
	name_check(t, r.diff_word == BHEAD + 200 && r.diff_want == w && r.diff_got == (w ^ 0x00010000u),
		   "(i) names word 872 with the snapshot's and the read value");

	model_noend(&h, BHEAD, 12);
	h.corrupt_word = BHEAD + 255u;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == BHEAD + 255,
		   "(i) the window's last word (927) is compared too");

	model_noend(&h, BHEAD, 12);
	h.corrupt_word = BHEAD + 256u;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	name_check(t, e == HVS6S_OK && r.diff_word == -1, "(i) word 928, past the window, is not compared");
}

/* ---- hardware-written words (swap-hwword) ------------------------------ */

static int has_sub(const char *sub)
{
	for (size_t i = 0; i < lines_n && i < 32; i++)
		if (strstr(lines_buf[i], sub))
			return 1;
	return 0;
}

static void run_lines(struct hvs *h, struct hvs6s_cfg *c, struct hvs6s_snap *s,
		      struct hvs6s_report *r, enum hvs6s_err *e)
{
	struct hvs6s_io io = model_io(h);
	struct hvs6s_emit em = { collect, NULL };

	*e = hvs6s_run(&io, c, s, r);
	lines_n = 0;
	(void)hvs6s_lines(s, r, &em);
}

/* The set's text form. */
static void case_hw_parse(void)
{
	struct hvs6s_hw_set h;
	char b[48];
	const char *t = "hw-parse";
	static const char *const bad[] = {
		"4", "4:", ":skip", "x:skip", "4:skipx", "4:skip,", ",4:skip", " 4:skip", "4:Skip",
		"63:skip", "4:skip,4:expect", "1:skip,2:skip,3:skip,4:skip,5:skip", "none,4:skip",
		"99999999999:skip",
	};
	int all_bad = 1;

	name_check(t, hvs6s_hw_parse("", &h) == 0 && h.n == 0, "'' is the empty set");
	name_check(t, hvs6s_hw_parse("none", &h) == 0 && h.n == 0, "'none' is the empty set");
	name_check(t, hvs6s_hw_parse("4:skip", &h) == 0 && h.n == 1 && h.w[0].off == 4 &&
		      h.w[0].mode == HVS6S_HW_SKIP,
		   "'4:skip': offset 4 SKIP");
	name_check(t, hvs6s_hw_parse("0:expect,62:skip,7:expect,5:skip", &h) == 0 && h.n == 4 &&
		      h.w[0].off == 0 && h.w[0].mode == HVS6S_HW_EXPECT && h.w[1].off == 62 &&
		      h.w[3].mode == HVS6S_HW_SKIP,
		   "four items, offsets 0 and 62 the bounds");
	name_check(t, strcmp(hvs6s_hw_format(&h, b, sizeof(b)), "0:expect,62:skip,7:expect,5:skip") == 0,
		   "format gives the same text back");
	h.n = 0;
	name_check(t, strcmp(hvs6s_hw_format(&h, b, sizeof(b)), "none") == 0 &&
		      strcmp(hvs6s_hw_format(NULL, b, sizeof(b)), "none") == 0,
		   "the empty set formats as 'none'");
	for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
		h.n = 3;
		if (hvs6s_hw_parse(bad[i], &h) != -1 || h.n != 0) {
			all_bad = 0;
			printf("        accepted: '%s'\n", bad[i]);
		}
	}
	name_check(t, all_bad, "malformed, out of range, duplicate and five-item texts refused, set empty");
	name_check(t, hvs6s_hw_parse(NULL, &h) == -1 && hvs6s_hw_parse("4:skip", NULL) == -1,
		   "NULL text or set refused");
}

/* An invalid set in cfg refuses the run before any access. */
static void case_hw_badcfg(void)
{
	struct hvs h;
	struct hvs6s_io io;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	const char *t = "hw-badcfg";

	model_board(&h, BHEAD, 1);
	io = model_io(&h);
	c.hw.n = HVS6S_HW_MAX + 1u;
	name_check(t, hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0 && h.nreads == 0,
		   "five offsets: HVS6S_ERR_ARG, nothing read or written");
	c.hw = (struct hvs6s_hw_set){ 2, { { 4, HVS6S_HW_SKIP }, { 4, HVS6S_HW_EXPECT } } };
	name_check(t, hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0,
		   "an offset twice: HVS6S_ERR_ARG, nothing written");
	c.hw = (struct hvs6s_hw_set){ 1, { { 63, HVS6S_HW_SKIP } } };
	name_check(t, hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0,
		   "offset 63 (past any entry): HVS6S_ERR_ARG");
	c.hw = (struct hvs6s_hw_set){ 1, { { 4, HVS6S_HW_NO } } };
	name_check(t, hvs6s_run(&io, &c, &s, &r) == HVS6S_ERR_ARG && h.nlog == 0,
		   "mode NO in the set: HVS6S_ERR_ARG");
}

/* The firmware's span at the restore: empty set, SKIP, EXPECT, undeclared. */
static void case_hw_restore(void)
{
	struct hvs h;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	uint32_t w;
	const char *t = "hw-restore";

	/* empty set: word 676 (entry 0 off 4) changing fails, named */
	model_board(&h, BHEAD, 1);
	w = h.mem[BHEAD + 4u];
	h.corrupt_word = BHEAD + 4u;
	h.our_start = 32;
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 676 && r.diff_want == w &&
		      r.rdiff.total == 1 && r.rdiff.undeclared == 1 && r.rdiff.hw_words == 0,
		   "empty set: a changed word 676 is RESTORE_WORD, as before");
	name_check(t, has_line("dlswap restore-diff word 676 entry 0 off 4 was 0xc0c0c0c0 now 0xc0c1c0c0 hw=no") &&
		      has_line("dlswap restore-diffs total=1 undeclared=1 skip=0 expect=0 shown=1 hwskip=none") &&
		      has_line("dlswap restore-words FAIL word 676 want=0xc0c0c0c0 got=0xc0c1c0c0 hwskip=none"),
		   "empty set: restore-diff names word 676, entry 0, off 4, hw=no; count line");
	name_check(t, has_sub("snapshot ok") && has_sub("hwskip=none"), "snapshot line carries hwskip=none");

	/* SKIP: the same change passes, named and counted */
	model_board(&h, BHEAD, 1);
	h.corrupt_word = BHEAD + 4u;
	h.our_start = 32;
	(void)hvs6s_hw_parse("4:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && r.restore_equal && r.diff_word == -1 && r.rdiff.total == 1 &&
		      r.rdiff.skip == 1 && r.rdiff.undeclared == 0 && r.rdiff.hw_words == 1,
		   "4:skip: word 676 changing passes the restore");
	name_check(t, has_line("dlswap restore-diff word 676 entry 0 off 4 was 0xc0c0c0c0 now 0xc0c1c0c0 hw=skip") &&
		      has_line("dlswap restore-diffs total=1 undeclared=0 skip=1 expect=0 shown=1 hwskip=4:skip") &&
		      has_line("dlswap restore-words ok fw 672..704 byte-equal except hw words off 4 (1 differed) hwskip=4:skip") &&
		      has_line("dlswap result ok"),
		   "4:skip: the word named hw=skip, PASS says 'byte-equal except hw words off 4'");
	name_check(t, has_sub("snapshot ok") && has_sub("hwskip=4:skip"), "snapshot line carries hwskip=4:skip");

	/* SKIP declared, nothing changes: still says what was left out */
	model_board(&h, BHEAD, 1);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && r.rdiff.total == 0 &&
		      has_line("dlswap restore-words ok fw 672..704 byte-equal except hw words off 4 (0 differed)"),
		   "4:skip, no change: 'byte-equal except hw words off 4 (0 differed)'");

	/* EXPECT: compared, reported, not failing */
	model_board(&h, BHEAD, 1);
	h.corrupt_word = BHEAD + 4u;
	h.our_start = 32;
	(void)hvs6s_hw_parse("4:expect", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && r.rdiff.expect == 1 && r.rdiff.skip == 0 &&
		      has_sub("restore-diff word 676 entry 0 off 4") && has_sub("hw=expect"),
		   "4:expect: the change reported hw=expect, the run ok");

	/* undeclared word beside a declared one: FAIL, named */
	model_board(&h, BHEAD, 1);
	h.corrupt_word = BHEAD + 5u;
	h.our_start = 32;
	(void)hvs6s_hw_parse("4:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 677 && r.rdiff.undeclared == 1 &&
		      has_sub("restore-diff word 677 entry 0 off 5") && has_sub("off 5 was") &&
		      has_line("dlswap restore-words FAIL word 677"),
		   "4:skip, word 677 (off 5) changed: RESTORE_WORD naming 677, hw=no");

	/* the END word is in no entry: 0:skip does not cover it */
	model_board(&h, BHEAD, 1);
	h.corrupt_word = BHEAD + 32u;
	h.our_start = 32;
	(void)hvs6s_hw_parse("0:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 704 &&
		      has_line("dlswap restore-diff word 704 entry none off none"),
		   "0:skip: a changed END word (704, in no entry) still fails, 'entry none off none'");

	/* every entry of a window: entry 3's offset 4 (word 772) */
	model_noend(&h, BHEAD, 12);
	h.corrupt_word = BHEAD + 32u * 3u + 4u;
	h.our_start = 32;
	(void)hvs6s_hw_parse("4:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && s.fw_noend && r.rdiff.skip == 1 && r.rdiff.hw_words == 8 &&
		      has_sub("restore-diff word 772 entry 3 off 4"),
		   "no END: word 772 (entry 3 off 4 of the window) skipped; 8 entries' word 4 declared");
	model_noend(&h, BHEAD, 12);
	h.corrupt_word = BHEAD + 32u * 3u + 5u;
	h.our_start = 32;
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD && r.diff_word == 773,
		   "no END: word 773 (entry 3 off 5) is not declared: named");
}

/* Our slot's read-back in hvs6s_swap: the HVS writes word 4 of our entry. */
static void case_hw_readback(void)
{
	struct hvs h;
	struct hvs6s_cfg c = base_cfg();
	struct hvs6s_snap s;
	struct hvs6s_report r;
	enum hvs6s_err e;
	const char *t = "hw-readback";

	model_board(&h, BHEAD, 1);
	h.stick_word = 36;
	h.stick_val = 0x0badc0deu;
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_LIST_READ && r.list_bad_word == 4 && !r.swapped &&
		      r.ldiff.total == 1 && r.ldiff.undeclared == 1,
		   "empty set: word 36 (our entry off 4) reads back otherwise: LIST_READ, word 4");
	name_check(t, count_writes(&h, hvs6s_lptrs_reg(CH)) == 1 && r.restored && r.restore_equal,
		   "empty set: LPTRS never swapped, the restore ran and is equal");
	name_check(t, has_line("dlswap readback-diff word 36 entry 0 off 4 was 0xc0c0c0c0 now 0x0badc0de hw=no") &&
		      has_line("dlswap readback-diffs total=1 undeclared=1"),
		   "empty set: readback-diff names word 36, entry 0, off 4, hw=no");

	model_board(&h, BHEAD, 1);
	h.stick_word = 36;
	h.stick_val = 0x0badc0deu;
	(void)hvs6s_hw_parse("4:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && r.swapped && r.lptrs_ok && r.settle_ok && r.ldiff.skip == 1 &&
		      r.ldiff.undeclared == 0 && r.restore_equal,
		   "4:skip: the written-back word 36 does not stop the swap");
	name_check(t, has_line("dlswap readback-diff word 36 entry 0 off 4 was 0xc0c0c0c0 now 0x0badc0de hw=skip") &&
		      has_line("dlswap readback-diffs total=1 undeclared=0 skip=1 expect=0 shown=1 hwskip=4:skip"),
		   "4:skip: readback-diff hw=skip and its count line");

	model_board(&h, BHEAD, 1);
	h.stick_word = 37;
	h.stick_val = 0x77u;
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_LIST_READ && r.list_bad_word == 5 && !r.swapped &&
		      has_sub("readback-diff word 37 entry 0 off 5"),
		   "4:skip, word 37 (off 5) differs: LIST_READ naming word 5");

	/* our list END (word 41) is in no entry; the inner END (40) is off 8 */
	model_board(&h, BHEAD, 1);
	h.stick_word = 41;
	h.stick_val = 0;
	(void)hvs6s_hw_parse("0:skip,8:skip", &c.hw);
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_ERR_LIST_READ && r.list_bad_word == 9 &&
		      has_sub("readback-diff word 41 entry none off none"),
		   "0:skip,8:skip: our list END (41, no entry) differing still fails");
	model_board(&h, BHEAD, 1);
	h.stick_word = 40;
	h.stick_val = 0;
	run_lines(&h, &c, &s, &r, &e);
	name_check(t, e == HVS6S_OK && r.ldiff.skip == 1 && has_sub("readback-diff word 40 entry 0 off 8"),
		   "0:skip,8:skip: the inner END (40, off 8) differing is skipped");
}

int main(void)
{
	case_pass();
	case_edge();
	case_never();
	case_stall();
	case_wrap();
	case_place_kept();
	case_place_fw();
	case_fit();
	case_restore_word();
	case_args();
	case_board_672();
	case_board_noend();
	case_noend_lines();
	case_place_used();
	case_place_overlap();
	case_place_nofit();
	case_noend_window();
	case_noend_memend();
	case_noend_place();
	case_noend_chan();
	case_noend_restore();
	case_hw_parse();
	case_hw_badcfg();
	case_hw_restore();
	case_hw_readback();
	printf("hvs6_dlist_swap_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
