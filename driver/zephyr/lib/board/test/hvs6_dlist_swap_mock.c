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

	if (off >= HVS6_DLIST_OFFSET)
		return h->mem[(off - HVS6_DLIST_OFFSET) / 4u];
	if (off == hvs6s_lptrs_reg(CH))
		return h->lptrs;
	if (off == hvs6s_status_reg(CH))
		return (h->frcnt << 16) | (YD5_HVS6_STATUS_MODE_RUN << 13) | (h->tick & 0x7ffu);
	if (off == hvs6s_dl_reg(CH))
		return h->lact;
	if (off == hvs6s_ctrl1_reg(CH))
		return h->ctrl1;
	return 0;
}

static void m_write(void *ctx, uint32_t off, uint32_t val)
{
	struct hvs *h = ctx;

	logev(h, EV_WRITE, off, val);
	if (off >= HVS6_DLIST_OFFSET) {
		h->mem[(off - HVS6_DLIST_OFFSET) / 4u] = val;
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
 * pattern in the rest of words 0..31, LPTRS = fw_head. */
static void model_init(struct hvs *h, uint32_t fw_head)
{
	struct hvs6w_plane p = { HVS6W_RGB565, 0, 0, 1920, 1080, 3840, 0x3e000000ull };

	memset(h, 0, sizeof(*h));
	for (uint32_t i = 0; i < MEMW; i++)
		h->mem[i] = 0xdead0000u | i;
	(void)hvs6w_write_plane(&h->mem[fw_head], HVS6W_LIST_WORDS, &p);
	h->lptrs = fw_head;
	h->lact = fw_head;
	h->ctrl1 = YD5_HVS6_CTRL1_BGENB;
	h->frcnt = 10;
	h->tpf = 3;
	h->corrupt_word = -1;
	h->our_start = 0xffffffffu;
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

	model_init(&h, 0);
	h.corrupt_word = 17;
	h.our_start = 32;
	io = model_io(&h);
	e = hvs6s_run(&io, &c, &s, &r);
	(void)hvs6s_lines(&s, &r, &em);
	name_check(t, e == HVS6S_ERR_RESTORE_WORD, "fails as HVS6S_ERR_RESTORE_WORD");
	name_check(t, r.lptrs_ok && r.frcnt_ok && r.settle_ok, "the swap itself passed");
	name_check(t, r.diff_word == 17 && r.diff_want == (0xdead0000u | 17u) &&
		      r.diff_got == ((0xdead0000u | 17u) ^ 0x00010000u),
		   "names word 17, with the snapshot's and the read value");
	name_check(t, r.restore_lptrs_ok && !r.restore_equal, "LPTRS equal, words not");
	name_check(t, h.ctrl1_writes == 0, "CTRL1 never written");

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
	/* a firmware list without an END in reach is kept capped */
	model_init(&h, 0);
	for (uint32_t i = 0; i < 80; i++)
		h.mem[i] = HVS6_CTL0_VALID | (1u << HVS6_CTL0_NEXT_SHIFT);
	io = model_io(&h);
	c = base_cfg();
	check(hvs6s_run(&io, &c, &s, &r) == HVS6S_OK && !s.fw_end_found && s.fw_len == HVS6S_FW_MAX &&
	      r.start == HVS6S_FW_MAX,
	      "firmware list without END: 64 words kept, our list placed after them");
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
	printf("hvs6_dlist_swap_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
