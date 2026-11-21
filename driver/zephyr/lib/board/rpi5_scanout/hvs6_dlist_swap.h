/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_dlist_swap: put one display list of our own on a running BCM2712 HVS
 * (scaler6, stepping D0) channel by swapping the channel's list pointer, and
 * give the firmware its list back, byte-equal, afterwards. Stage 2 of
 * docs/notes/rpi5-display.md (Q4): our XRGB8888 plane in free list memory,
 * the pointer swap at vblank, the firmware's list restored.
 *
 * Header only, C11, no Zephyr includes. Register and list-memory access go
 * through struct hvs6s_io (offsets from the HVS base, as in snapshot.h and
 * hvs6_chan.h), so the host mock (driver/zephyr/lib/board/test/
 * hvs6_dlist_swap_mock.c, tools/v3d/check71.d/hvs6_dlist_swap.sh) runs the
 * same code against a model of the channel.
 *
 * API change of dlplace (plan #385), for the rpi5_scanout32 probe. The board
 * (s32dumprun at cc89a1cf, Pi 5 D0, HVS VERSION 0x2454) showed channel 0's
 * LPTRS = DL = 0x2a0: the firmware's live list starts at word 672, its first
 * entry is the firmware plane with NEXT=32, and a 256-entry walk saw no END.
 * So the firmware's list is no longer assumed to sit at words 0..31:
 *  - struct hvs6s_snap lost kept[] (words 0..31). The snapshot is exactly
 *    LPTRS and the firmware's words [HEADE, HEADE + fw_len), and the restore
 *    compares exactly those. New fields: fw_entries, walk_stop, walk_word.
 *  - hvs6s_snapshot refuses a list it cannot walk to its END within
 *    HVS6S_FW_MAX words (HVS6S_ERR_FW_NOEND), and a walk that meets an entry
 *    with NEXT 0 or one running past the list memory (HVS6S_ERR_FW_WALK).
 *    It used to keep a capped span and go on.
 *  - hvs6s_place_io (new, used by hvs6s_run) reads the target range first
 *    and refuses any non-zero word in it (HVS6S_ERR_PLACE_USED); with
 *    HVS6S_AUTO it searches past such words. hvs6s_place is kept, geometry
 *    only (no read), for planning A/B slots.
 *  - hvs6s_lines prints place, write and restore as "skip" after a refused
 *    snapshot (it used to stop after the snapshot line). New report fields:
 *    place_bad_word, place_bad_val, place_skipped.
 *  struct hvs6s_cfg is unchanged; the old error codes keep their values.
 *
 * Change of s32noend (plan #385): the board (s32dumprun; b4a9c159, 3ccf5f71)
 * showed HEADE=672, an entry with NEXT=32 there and no END at word 704, and
 * a desk check (enddesk) found that no register holds a list's length. So a
 * walk that reaches no END within its bound is no longer refused:
 *  - hvs6s_snapshot keeps a fixed window [HEADE, HEADE + min(HVS6S_FW_MAX,
 *    mem_words - HEADE)) as the firmware's span, fw_len its length, and sets
 *    fw_noend = 1 (fw_end_found stays 0). The walk ends without an END once
 *    it has covered the window: HVS6S_FW_MAX words, or exactly the words
 *    left in the memory. NEXT 0, or an entry running past the memory, still
 *    refuses as HVS6S_ERR_FW_WALK; HVS6S_ERR_FW_NOEND is no longer returned.
 *  - With fw_noend our list goes only wholly before HEADE: start >= 32,
 *    start + len <= HEADE, every word zero (HVS6S_ERR_PLACE_USED), and no
 *    other channel's LPTRS.HEADE inside it (HVS6S_ERR_PLACE_CHAN, the channel
 *    in r->place_chan). No room: HVS6S_ERR_PLACE_NOEND.
 *  - The restore compares LPTRS and the whole window, as a walked list.
 *  - hvs6s_lines adds " window=noend" to the "snapshot ok" line.
 *  With an END found, nothing changed.
 *
 * Facts, from the pinned tree (raspberrypi/linux rpi-6.12.y at 5bd564aa,
 * drivers/gpu/drm/vc4/, as rpi:<file>:<line>; taken from the linuxdesk desk
 * check of plan #385, not re-read for this header; no code copied):
 *  - Channel 0's list pointer on D0 is SCALER6D_DISP0_LPTRS at 0x110, +0x40
 *    per channel, one field HEADE in bits 11:0, in words
 *    (rpi:vc4_regs.h:588-591, :704). Offsets here are hvs6_chan.h's
 *    YD5_HVS6D_* macros.
 *  - Linux updates a live channel with one LPTRS write and no read-back or
 *    wait (rpi:vc4_hvs.c:1166-1169, called at :1343-1346), after writing the
 *    whole new list including its END word (:1316).
 *  - There is no latch or pending bit in the channel registers
 *    (rpi:vc4_regs.h:564-622, :701-733). Hence nothing here claims that the
 *    HVS latched the pointer: the two results below (read-back, FRCNT moved)
 *    do not prove it. When the pointer takes effect (frame start or at once)
 *    only the board can show, and only through what it scans out.
 *  - Linux frees an old list only once FRCNT moved past the frame of the
 *    swap, target (FRCNT + 1) & 63 (rpi:vc4_hvs.c:682-697). FRCNT is STATUS
 *    bits 21:16 (rpi:vc4_regs.h:605). Here: two advances, to be safe.
 *  - Linux's lists start at or after word 32, HVS_BOOTLOADER_DLIST_END
 *    (rpi:vc4_regs.h:1056, rpi:vc4_hvs.c:1576-1583). Words 0..31 stay
 *    refused for our list (HVS6S_ERR_PLACE_KEPT); the firmware's own list
 *    is found from LPTRS.HEADE, wherever it is.
 *  - CTRL1.BGENB is the background fill, toggled at EOF from needs_bg_fill
 *    (rpi:vc4_hvs.c:1484-1510). A swap does not need it. CTRL1 is never
 *    written here; it is read for the record only.
 *
 * The sequence (hvs6s_run, or the steps one by one):
 *  1. hvs6s_snapshot: the firmware's LPTRS, and its list from LPTRS.HEADE,
 *     walked entry by entry by CTL0.NEXT (the entry's size in words) to the
 *     list END word, at most HVS6S_FW_MAX words. With no END in that bound
 *     the span is the fixed window from HEADE (fw_noend). Refused, with
 *     nothing written and no restore needed, on NEXT 0 or an entry past the
 *     list memory.
 *  2. hvs6s_place_io: our list's start, the first fit at or after word 32
 *     (HVS6S_AUTO), or the caller's start checked. The range holds our
 *     whole list, HVS6W_LIST_WORDS (the 9-word entry with NEXT=9 and the
 *     list END, as hvs6_dlist_write.h writes). Refused by name: overlap with
 *     words 0..31, overlap with [HEADE, HEADE + fw_len), a list that does
 *     not fit the memory (cfg.mem_words, from CXM_SIZE) or the 12-bit HEADE,
 *     a non-zero word in the range (read before anything is written).
 *     With fw_noend also a range ending after HEADE, or one holding another
 *     channel's LPTRS.HEADE.
 *  3. hvs6s_swap: our entry from hvs6_dlist_write.h (10 words, Linux's
 *     layout), written and read back word by word; the cache cleaned over
 *     the list (fb_flush.h's line-rounded range) and a barrier; the bus
 *     address of the list from bus_addr.h recorded; then LPTRS, last, as
 *     one write. Then two results, each on its own: LPTRS read back as
 *     written, and FRCNT advanced (6-bit wrap aware), each within
 *     max_polls polls. FRCNT, STATUS.MODE, LPTRS, DL and CTRL1.BGENB are
 *     recorded at the write and after it. Then FRCNT must advance twice past
 *     the write before the old list is reused or checked.
 *  4. hvs6s_restore, always once the snapshot was taken, even after a
 *     failure in 2 or 3: the firmware's LPTRS written back, two FRCNT
 *     advances, then LPTRS and the firmware's words [HEADE, HEADE + fw_len)
 *     compared byte for byte with the snapshot; the first differing word is
 *     named. Our own list's words are left in list memory, not cleared.
 *
 * The list memory sits at HVS + 0x4000 and is device memory on the Pi 5's
 * Zephyr map; the clean is then a no-op, kept so that a staged list in
 * normal memory and the A/B buffers of the caller are treated alike. The
 * framebuffer's own clean is the caller's (fb_flush.h).
 *
 * The poll interval (io->wait) must be under 64 frames (about 1 s at 60 Hz),
 * or a 6-bit FRCNT difference aliases.
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_SWAP_H
#define YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_SWAP_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "bus_addr.h"
#include "dlist_alloc.h"
#include "fb_flush.h"
#include "hvs6_chan.h"
#include "hvs6_dlist_write.h"

/* Our list's start chosen here (first free fit from word 32). */
#define HVS6S_AUTO 0xffffffffu
/* The walk's hard bound: words of the firmware's list, from HEADE, that the
 * walk may cover and the snapshot keeps. No END within it: the snapshot keeps
 * the window [HEADE, HEADE + min(HVS6S_FW_MAX, mem_words - HEADE)). */
#define HVS6S_FW_MAX 256u
/* Words 0..31 Linux leaves to the bootloader (rpi:vc4_regs.h:1056). */
#define HVS6S_KEPT 32u
/* STATUS.FRCNT, bits 21:16 (rpi:vc4_regs.h:605). */
#define HVS6S_FRCNT_SHIFT 16u
#define HVS6S_FRCNT_MASK 0x3fu
/* DISPx_DL.LACT, bits 11:0, the list word being scanned (scanout_alive.h's
 * SA_DL_LACT_MASK, rpi:vc4_regs.h:619). Recorded, never a pass criterion. */
#define HVS6S_DL_LACT_MASK 0x00000fffu
/* FRCNT advances to wait past a pointer write before the old list is reused. */
#define HVS6S_SETTLE_FRAMES 2u
/* The record's prefix of every line hvs6s_lines prints. */
#define HVS6S_TAG "dlswap"

/* Register and list-memory access. Offsets are from the HVS base. */
struct hvs6s_io {
	uint32_t (*read32)(void *ctx, uint32_t off);
	void (*write32)(void *ctx, uint32_t off, uint32_t val);
	/* data cache clean of [addr, addr + len), ARM physical addresses */
	void (*clean)(void *ctx, uint64_t addr, uint64_t len);
	void (*barrier)(void *ctx); /* dsb: the list's writes before LPTRS */
	void (*wait)(void *ctx);    /* one poll interval, well under a frame */
	void *ctx;
};

struct hvs6s_cfg {
	unsigned int chan;     /* 0..2 */
	uint32_t mem_words;    /* list memory in words (CXM_SIZE, read by the caller) */
	uint32_t start;        /* HVS6S_AUTO, or the word our list starts at */
	uint64_t list_arm;     /* ARM physical address of list word 0 (HVS + 0x4000) */
	uint32_t cache_line;   /* bytes; 0 means fb_flush.h's 64 */
	uint32_t max_polls;    /* polls allowed per FRCNT wait and read-back */
	struct hvs6w_plane plane;
};

/* What failed, first failure only; HVS6S_OK when everything passed. */
enum hvs6s_err {
	HVS6S_OK = 0,
	HVS6S_ERR_ARG,          /* null io/cfg/callback, channel over 2, max_polls 0 */
	HVS6S_ERR_SNAPSHOT,     /* the firmware's LPTRS outside the list memory */
	HVS6S_ERR_PLACE_KEPT,   /* our list overlaps words 0..31 */
	HVS6S_ERR_PLACE_FW,     /* our list overlaps the firmware's list */
	HVS6S_ERR_PLACE_FIT,    /* our list does not fit memory or HEADE */
	HVS6S_ERR_PLANE,        /* hvs6_dlist_write.h refused the plane */
	HVS6S_ERR_BUS,          /* the list's address has no bus address */
	HVS6S_ERR_LIST_READ,    /* a written list word did not read back */
	HVS6S_ERR_LPTRS_READ,   /* LPTRS never read back as written */
	HVS6S_ERR_FRCNT,        /* FRCNT did not advance after the write */
	HVS6S_ERR_SETTLE,       /* FRCNT did not advance twice past the write */
	HVS6S_ERR_RESTORE_FRCNT,/* FRCNT did not advance twice after the restore */
	HVS6S_ERR_RESTORE_LPTRS,/* LPTRS differs from the snapshot after the restore */
	HVS6S_ERR_RESTORE_WORD, /* a list word differs from the snapshot */
	HVS6S_ERR_FW_NOEND,     /* no list END within HVS6S_FW_MAX words (no longer returned) */
	HVS6S_ERR_FW_WALK,      /* an entry with NEXT 0, or one past the list memory */
	HVS6S_ERR_PLACE_USED,   /* a word of our target range is not zero */
	HVS6S_ERR_PLACE_NOEND,  /* no END: our list does not fit wholly before HEADE */
	HVS6S_ERR_PLACE_CHAN,   /* no END: another channel's LPTRS.HEADE is in our range */
};

static inline const char *hvs6s_err_name(enum hvs6s_err e)
{
	switch (e) {
	case HVS6S_OK: return "ok";
	case HVS6S_ERR_ARG: return "bad argument";
	case HVS6S_ERR_SNAPSHOT: return "firmware LPTRS outside the list memory";
	case HVS6S_ERR_PLACE_KEPT: return "place: overlaps words 0..31";
	case HVS6S_ERR_PLACE_FW: return "place: overlaps the firmware's list";
	case HVS6S_ERR_PLACE_FIT: return "place: does not fit";
	case HVS6S_ERR_PLANE: return "plane: refused by hvs6_dlist_write";
	case HVS6S_ERR_BUS: return "list address has no bus address";
	case HVS6S_ERR_LIST_READ: return "list word did not read back";
	case HVS6S_ERR_LPTRS_READ: return "LPTRS did not read back";
	case HVS6S_ERR_FRCNT: return "FRCNT did not advance";
	case HVS6S_ERR_SETTLE: return "FRCNT did not advance twice";
	case HVS6S_ERR_RESTORE_FRCNT: return "restore: FRCNT did not advance twice";
	case HVS6S_ERR_RESTORE_LPTRS: return "restore: LPTRS differs";
	case HVS6S_ERR_RESTORE_WORD: return "restore: list word differs";
	case HVS6S_ERR_FW_NOEND: return "snapshot: no END within the walk's bound";
	case HVS6S_ERR_FW_WALK: return "snapshot: entry with NEXT 0 or past the list memory";
	case HVS6S_ERR_PLACE_USED: return "place: target range holds a non-zero word";
	case HVS6S_ERR_PLACE_NOEND: return "place: no END, no room wholly before HEADE";
	case HVS6S_ERR_PLACE_CHAN: return "place: another channel's LPTRS.HEADE in range";
	}
	return "?";
}

/* The firmware's state, taken before anything is written. */
struct hvs6s_snap {
	int taken;                      /* 1 when the walk reached the END or covered the window */
	uint32_t lptrs;                 /* the whole register */
	uint32_t fw_head, fw_len;       /* its span in words: the list, END included, or the window */
	int fw_end_found;
	int fw_noend;                   /* 1: no END seen, [fw_head, fw_head + fw_len) is a window */
	uint32_t fw_entries;            /* entries walked before the END */
	uint32_t walk_stop, walk_word;  /* last word the walk read, and its value */
	uint32_t fw[HVS6S_FW_MAX];      /* words fw_head .. fw_head + fw_len - 1 */
};

/* One sample of the channel registers. */
struct hvs6s_regs {
	uint32_t status, lptrs, dl, ctrl1;
};

static inline uint32_t hvs6s_frcnt(uint32_t status)
{
	return (status >> HVS6S_FRCNT_SHIFT) & HVS6S_FRCNT_MASK;
}

static inline uint32_t hvs6s_mode(uint32_t status)
{
	return (status >> YD5_HVS6_STATUS_MODE_SHIFT) & YD5_HVS6_STATUS_MODE_MASK;
}

static inline uint32_t hvs6s_bgenb(uint32_t ctrl1)
{
	return (ctrl1 & YD5_HVS6_CTRL1_BGENB) ? 1u : 0u;
}

/* Frames from a to b on the 6-bit counter. */
static inline uint32_t hvs6s_frdelta(uint32_t a, uint32_t b)
{
	return (b - a) & HVS6S_FRCNT_MASK;
}

/* D0 channel offsets (hvs6_chan.h). */
static inline uint32_t hvs6s_lptrs_reg(unsigned int ch)
{
	return yd5_hvs6_lptrs_reg(YD5_HVS6_D0, ch);
}

static inline uint32_t hvs6s_status_reg(unsigned int ch)
{
	return yd5_hvs6_status_reg(YD5_HVS6_D0, ch);
}

static inline uint32_t hvs6s_ctrl1_reg(unsigned int ch)
{
	return yd5_hvs6_ctrl1_reg(YD5_HVS6_D0, ch);
}

static inline uint32_t hvs6s_dl_reg(unsigned int ch)
{
	return YD5_HVS6D_DISP0_DL + ch * YD5_HVS6D_DISP_STRIDE;
}

static inline uint32_t hvs6s_word_off(uint32_t word)
{
	return HVS6_DLIST_OFFSET + word * 4u;
}

/* The whole record of one run. */
struct hvs6s_report {
	enum hvs6s_err err;          /* the first failure */

	/* place */
	enum hvs6s_err place_err;
	uint32_t start, len;         /* our list, in words */
	int64_t place_bad_word;      /* non-zero word that refused the start, -1 when none */
	uint32_t place_bad_val;
	uint32_t place_skipped;      /* HVS6S_AUTO: non-zero words the search stepped past */
	int place_chan;              /* PLACE_CHAN: the channel whose HEADE refused, -1 when none */
	uint64_t list_bus;           /* bus address of our list's first word */
	struct fb_range cleaned;     /* the cleaned range (ARM) */

	/* swap */
	int swapped;                 /* LPTRS was written with our start */
	uint32_t list_bad_word;      /* list word that did not read back (index in our list) */
	uint32_t lptrs_written;
	struct hvs6s_regs at_write;  /* sampled just before the LPTRS write */
	struct hvs6s_regs after;     /* sampled when both results were in, or at the timeout */
	int lptrs_ok;                /* result 1: LPTRS read back as written */
	uint32_t lptrs_first;        /* LPTRS read at once after the write */
	uint32_t lptrs_poll;         /* polls before it read back (0: at once) */
	int frcnt_ok;                /* result 2: FRCNT advanced */
	uint32_t frcnt_polls;        /* polls before it advanced */
	int lact_follows;            /* DL.LACT equals our start after (record only) */
	int settle_ok;               /* two advances past the write */
	uint32_t settle_polls;

	/* restore */
	int restored;                /* the firmware's LPTRS was written back */
	int restore_frcnt_ok;
	uint32_t restore_polls;
	struct hvs6s_regs at_restore, after_restore;
	int restore_lptrs_ok;
	int restore_equal;           /* LPTRS and every kept word equal the snapshot */
	int64_t diff_word;           /* first differing list word, -1 when none */
	uint32_t diff_want, diff_got;
};

static inline void hvs6s_report_init(struct hvs6s_report *r)
{
	*r = (struct hvs6s_report){0};
	r->diff_word = -1;
	r->place_bad_word = -1;
	r->place_chan = -1;
	r->start = HVS6S_AUTO;
}

static inline void hvs6s_fail(struct hvs6s_report *r, enum hvs6s_err e)
{
	if (r->err == HVS6S_OK)
		r->err = e;
}

static inline int hvs6s_io_ok(const struct hvs6s_io *io)
{
	return io && io->read32 && io->write32 && io->clean && io->barrier && io->wait;
}

static inline int hvs6s_cfg_ok(const struct hvs6s_cfg *c)
{
	return c && c->chan < YD5_HVS6_NUM_CHANNELS && c->max_polls > 0 &&
	       c->mem_words >= HVS6S_KEPT;
}

static inline void hvs6s_sample(const struct hvs6s_io *io, unsigned int ch, struct hvs6s_regs *s)
{
	s->status = io->read32(io->ctx, hvs6s_status_reg(ch));
	s->lptrs = io->read32(io->ctx, hvs6s_lptrs_reg(ch));
	s->dl = io->read32(io->ctx, hvs6s_dl_reg(ch));
	s->ctrl1 = io->read32(io->ctx, hvs6s_ctrl1_reg(ch));
}

/*
 * 1. The firmware's LPTRS and its list, from LPTRS.HEADE to the list END.
 * Entries are walked by CTL0.NEXT (the entry's size in words; an entry's
 * inner END, Linux's layout, is skipped by it). Words between entries are
 * kept too. When the walk has covered its window, min(HVS6S_FW_MAX,
 * mem_words - HEADE) words, without an END, the snapshot keeps that window
 * instead (fw_noend; walk_stop and walk_word are the last entry word read).
 * Only reads; on a refusal s->taken stays 0 and walk_stop and walk_word name
 * where the walk stopped.
 */
static inline enum hvs6s_err hvs6s_snapshot(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					    struct hvs6s_snap *s)
{
	uint32_t pos = 0, head, room, win;

	if (!s)
		return HVS6S_ERR_ARG;
	*s = (struct hvs6s_snap){0};
	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c))
		return HVS6S_ERR_ARG;
	s->lptrs = io->read32(io->ctx, hvs6s_lptrs_reg(c->chan));
	head = s->lptrs & YD5_HVS6_LPTRS_HEADE_MASK;
	s->fw_head = head;
	s->walk_stop = head;
	if (head >= c->mem_words)
		return HVS6S_ERR_SNAPSHOT;
	room = c->mem_words - head; /* words from HEADE to the memory's end */
	win = room < HVS6S_FW_MAX ? room : HVS6S_FW_MAX;
	for (;;) {
		uint32_t w, n;

		if (pos > room)
			return HVS6S_ERR_FW_WALK; /* the last entry ran past the memory */
		if (pos >= win) {
			/* no END in the window: keep the window itself */
			s->fw_len = win;
			s->fw_noend = 1;
			for (uint32_t i = 0; i < s->fw_len; i++)
				s->fw[i] = io->read32(io->ctx, hvs6s_word_off(head + i));
			s->taken = 1;
			return HVS6S_OK;
		}
		w = io->read32(io->ctx, hvs6s_word_off(head + pos));
		s->walk_stop = head + pos;
		s->walk_word = w;
		if (w & HVS6_CTL0_END)
			break;
		n = (w >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
		if (n == 0)
			return HVS6S_ERR_FW_WALK;
		s->fw_entries++;
		pos += n;
	}
	s->fw_len = pos + 1u;
	s->fw_end_found = 1;
	for (uint32_t i = 0; i < s->fw_len; i++)
		s->fw[i] = io->read32(io->ctx, hvs6s_word_off(head + i));
	s->taken = 1;
	return HVS6S_OK;
}

static inline int hvs6s_overlap(uint32_t a, uint32_t al, uint32_t b, uint32_t bl)
{
	return al && bl && (uint64_t)a < (uint64_t)b + bl && (uint64_t)b < (uint64_t)a + al;
}

/* A span of len words at start against the kept words, the firmware's list
 * and the memory, and, with fw_noend, against HEADE (the span must end at or
 * before it). Usable for each slot of an A/B pair. */
static inline enum hvs6s_err hvs6s_check_span(const struct hvs6s_cfg *c, const struct hvs6s_snap *s,
					      uint32_t start, uint32_t len)
{
	if (!c || !s || !len)
		return HVS6S_ERR_ARG;
	if (hvs6s_overlap(start, len, 0, HVS6S_KEPT))
		return HVS6S_ERR_PLACE_KEPT;
	if (hvs6s_overlap(start, len, s->fw_head, s->fw_len))
		return HVS6S_ERR_PLACE_FW;
	if ((uint64_t)start + len > c->mem_words || start > YD5_HVS6_LPTRS_HEADE_MASK)
		return HVS6S_ERR_PLACE_FIT;
	/* no END: the firmware's list may run on past the window; only words
	 * wholly before HEADE are known to be outside it */
	if (s->fw_noend && (uint64_t)start + len > s->fw_head)
		return HVS6S_ERR_PLACE_NOEND;
	return HVS6S_OK;
}

/* Our list's start by geometry only (no read): dlist_alloc.h's first fit,
 * or the caller's, checked. hvs6s_run uses hvs6s_place_io, which also reads
 * the range; this one stays for planning A/B slots. */
static inline enum hvs6s_err hvs6s_place(const struct hvs6s_cfg *c, const struct hvs6s_snap *s,
					 uint32_t len, uint32_t *start)
{
	uint32_t st;

	if (!c || !s || !s->taken || !start || !len)
		return HVS6S_ERR_ARG;
	if (c->start == HVS6S_AUTO) {
		struct dla_span fw = { s->fw_head, s->fw_len };
		uint32_t mem = c->mem_words;
		int64_t a;

		/* a start past HEADE cannot be pointed at */
		if (mem > YD5_HVS6_LPTRS_HEADE_MASK + len)
			mem = YD5_HVS6_LPTRS_HEADE_MASK + len;
		if (s->fw_noend)
			mem = s->fw_head; /* wholly before HEADE */
		a = dla_first_fit(mem, len, DLA_MIN_ALIGN, &fw, 1);
		if (a < 0)
			return s->fw_noend ? HVS6S_ERR_PLACE_NOEND : HVS6S_ERR_PLACE_FIT;
		st = (uint32_t)a;
	} else {
		st = c->start;
	}
	{
		enum hvs6s_err e = hvs6s_check_span(c, s, st, len);

		if (e != HVS6S_OK)
			return e;
	}
	*start = st;
	return HVS6S_OK;
}

/* The first non-zero word of [start, start + len), read in order; 1 if any. */
static inline int hvs6s_first_used(const struct hvs6s_io *io, uint32_t start, uint32_t len,
				   uint32_t *at, uint32_t *val)
{
	for (uint32_t i = 0; i < len; i++) {
		uint32_t w = io->read32(io->ctx, hvs6s_word_off(start + i));

		if (w != 0) {
			*at = start + i;
			*val = w;
			return 1;
		}
	}
	return 0;
}

/* With fw_noend: the first channel other than c->chan whose LPTRS.HEADE,
 * read now, is inside [start, start + len), its HEADE in *head; -1 when
 * none. */
static inline int hvs6s_other_head_in(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
				      uint32_t start, uint32_t len, uint32_t *head)
{
	for (unsigned int ch = 0; ch < YD5_HVS6_NUM_CHANNELS; ch++) {
		uint32_t h;

		if (ch == c->chan)
			continue;
		h = io->read32(io->ctx, hvs6s_lptrs_reg(ch)) & YD5_HVS6_LPTRS_HEADE_MASK;
		if (hvs6s_overlap(start, len, h, 1)) {
			*head = h;
			return (int)ch;
		}
	}
	return -1;
}

/*
 * 2. Our list's start, read and checked: the geometry of hvs6s_check_span
 * and every word of [start, start + len) zero. With fw_noend the range must
 * also end at or before HEADE (HVS6S_ERR_PLACE_NOEND, also when HVS6S_AUTO
 * finds no room there) and hold no other channel's LPTRS.HEADE
 * (HVS6S_ERR_PLACE_CHAN, the channel in r->place_chan and its HEADE in
 * r->place_bad_word; HVS6S_AUTO steps past it). HVS6S_AUTO searches from word
 * 32 for the first range that passes both, stepping past the firmware's
 * list and past each non-zero word it meets (r->place_skipped counts
 * those). A caller's start with a non-zero word is refused as
 * HVS6S_ERR_PLACE_USED, the word named in r->place_bad_word/_val. Only
 * reads. r may be NULL.
 */
static inline enum hvs6s_err hvs6s_place_io(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					    const struct hvs6s_snap *s, uint32_t len, uint32_t *start,
					    struct hvs6s_report *r)
{
	uint32_t st, at = 0, val = 0;
	enum hvs6s_err e;

	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c) || !s || !s->taken || !start || !len)
		return HVS6S_ERR_ARG;
	if (c->start == HVS6S_AUTO) {
		/* the last start that HEADE can point at, and the memory's end */
		uint64_t lim = c->mem_words;

		if (lim > (uint64_t)YD5_HVS6_LPTRS_HEADE_MASK + len)
			lim = (uint64_t)YD5_HVS6_LPTRS_HEADE_MASK + len;
		if (s->fw_noend)
			lim = s->fw_head; /* wholly before HEADE */
		st = HVS6S_KEPT;
		for (;;) {
			uint32_t oh = 0;

			if ((uint64_t)st + len > lim)
				return s->fw_noend ? HVS6S_ERR_PLACE_NOEND : HVS6S_ERR_PLACE_FIT;
			if (hvs6s_overlap(st, len, s->fw_head, s->fw_len)) {
				st = s->fw_head + s->fw_len;
				continue;
			}
			if (s->fw_noend && hvs6s_other_head_in(io, c, st, len, &oh) >= 0) {
				st = oh + 1u;
				continue;
			}
			if (hvs6s_first_used(io, st, len, &at, &val)) {
				if (r)
					r->place_skipped++;
				st = at + 1u;
				continue;
			}
			break;
		}
	} else {
		st = c->start;
	}
	e = hvs6s_check_span(c, s, st, len);
	if (e != HVS6S_OK)
		return e;
	if (s->fw_noend) {
		int och = hvs6s_other_head_in(io, c, st, len, &at);

		if (och >= 0) {
			if (r) {
				r->place_chan = och;
				r->place_bad_word = at;
				r->place_bad_val =
					io->read32(io->ctx, hvs6s_lptrs_reg((unsigned int)och));
			}
			return HVS6S_ERR_PLACE_CHAN;
		}
	}
	if (hvs6s_first_used(io, st, len, &at, &val)) {
		if (r) {
			r->place_bad_word = at;
			r->place_bad_val = val;
		}
		return HVS6S_ERR_PLACE_USED;
	}
	*start = st;
	return HVS6S_OK;
}

/*
 * Poll until FRCNT is at least `need` frames past `from`, adding the
 * frames between successive reads (6-bit wrap aware). *polls gets the
 * waits taken. Returns 1 when FRCNT got there, 0 after max_polls waits.
 */
static inline int hvs6s_wait_frames(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
				    uint32_t from, uint32_t need, uint32_t *polls)
{
	uint32_t last = from, total = 0;

	for (uint32_t i = 0;; i++) {
		uint32_t f = hvs6s_frcnt(io->read32(io->ctx, hvs6s_status_reg(c->chan)));

		total += hvs6s_frdelta(last, f);
		last = f;
		if (total >= need) {
			*polls = i;
			return 1;
		}
		if (i >= c->max_polls) {
			*polls = i;
			return 0;
		}
		io->wait(io->ctx);
	}
}

/* 3. Write our list at start, then LPTRS last; the two results; the settle. */
static inline enum hvs6s_err hvs6s_swap(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					uint32_t start, struct hvs6s_report *r)
{
	uint32_t list[HVS6W_LIST_WORDS];
	uint32_t f0, last, total = 0, lp_poll = 0, fr_poll = 0;
	int n, lp_ok = 0, fr_ok = 0;
	uint64_t arm, bus;
	uint32_t i;

	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c) || !r)
		return HVS6S_ERR_ARG;
	n = hvs6w_write_plane(list, HVS6W_LIST_WORDS, &c->plane);
	if (n != (int)HVS6W_LIST_WORDS)
		return HVS6S_ERR_PLANE;
	r->start = start;
	r->len = (uint32_t)n;
	arm = c->list_arm + (uint64_t)start * 4u;
	if (bcm2712_arm_to_bus(arm, &bus) != 0)
		return HVS6S_ERR_BUS;
	r->list_bus = bus;

	for (i = 0; i < r->len; i++)
		io->write32(io->ctx, hvs6s_word_off(start + i), list[i]);
	for (i = 0; i < r->len; i++) {
		if (io->read32(io->ctx, hvs6s_word_off(start + i)) != list[i]) {
			r->list_bad_word = i;
			return HVS6S_ERR_LIST_READ;
		}
	}
	r->cleaned = fb_flush_range(arm, (uint64_t)r->len * 4u,
				    c->cache_line ? c->cache_line : FB_FLUSH_LINE_A76);
	if (r->cleaned.end != r->cleaned.start)
		io->clean(io->ctx, r->cleaned.start, r->cleaned.end - r->cleaned.start);
	io->barrier(io->ctx);

	hvs6s_sample(io, c->chan, &r->at_write);
	f0 = hvs6s_frcnt(r->at_write.status);
	r->lptrs_written = dla_ptr(start) & YD5_HVS6_LPTRS_HEADE_MASK;
	io->write32(io->ctx, hvs6s_lptrs_reg(c->chan), r->lptrs_written); /* the one write */
	r->swapped = 1;

	/* Results 1 and 2, polled together, each kept on its own. */
	r->lptrs_first = io->read32(io->ctx, hvs6s_lptrs_reg(c->chan));
	last = f0;
	for (i = 0;; i++) {
		struct hvs6s_regs s;

		hvs6s_sample(io, c->chan, &s);
		if (!lp_ok && (s.lptrs & YD5_HVS6_LPTRS_HEADE_MASK) == r->lptrs_written) {
			lp_ok = 1;
			lp_poll = i;
		}
		total += hvs6s_frdelta(last, hvs6s_frcnt(s.status));
		last = hvs6s_frcnt(s.status);
		if (!fr_ok && total >= 1) {
			fr_ok = 1;
			fr_poll = i;
		}
		r->after = s;
		if ((lp_ok && fr_ok) || i >= c->max_polls)
			break;
		io->wait(io->ctx);
	}
	r->lptrs_ok = lp_ok;
	r->lptrs_poll = lp_poll;
	r->frcnt_ok = fr_ok;
	r->frcnt_polls = fr_ok ? fr_poll : i;
	r->lact_follows = (r->after.dl & HVS6S_DL_LACT_MASK) == start;

	/* Two advances past the write before the old list is touched. */
	if (fr_ok) {
		uint32_t p = 0;

		if (total >= HVS6S_SETTLE_FRAMES) {
			r->settle_ok = 1;
		} else {
			r->settle_ok = hvs6s_wait_frames(io, c, last, HVS6S_SETTLE_FRAMES - total, &p);
		}
		r->settle_polls = p;
	}
	if (!lp_ok)
		return HVS6S_ERR_LPTRS_READ;
	if (!fr_ok)
		return HVS6S_ERR_FRCNT;
	if (!r->settle_ok)
		return HVS6S_ERR_SETTLE;
	return HVS6S_OK;
}

/* 4. The firmware's LPTRS back, two advances, then the byte-equal compare. */
static inline enum hvs6s_err hvs6s_restore(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					   const struct hvs6s_snap *s, struct hvs6s_report *r)
{
	uint32_t i, got;

	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c) || !s || !s->taken || !r)
		return HVS6S_ERR_ARG;
	hvs6s_sample(io, c->chan, &r->at_restore);
	io->barrier(io->ctx);
	io->write32(io->ctx, hvs6s_lptrs_reg(c->chan), s->lptrs);
	r->restored = 1;
	r->restore_frcnt_ok = hvs6s_wait_frames(io, c, hvs6s_frcnt(r->at_restore.status),
						 HVS6S_SETTLE_FRAMES, &r->restore_polls);
	hvs6s_sample(io, c->chan, &r->after_restore);

	/* Compared even after a stall: the record says what was there. */
	r->restore_equal = 1;
	r->restore_lptrs_ok = r->after_restore.lptrs == s->lptrs;
	if (!r->restore_lptrs_ok)
		r->restore_equal = 0;
	/* exactly the snapshot's words, [HEADE, HEADE + fw_len) */
	for (i = 0; i < s->fw_len && r->diff_word < 0; i++) {
		got = io->read32(io->ctx, hvs6s_word_off(s->fw_head + i));
		if (got != s->fw[i]) {
			r->diff_word = s->fw_head + i;
			r->diff_want = s->fw[i];
			r->diff_got = got;
		}
	}
	if (r->diff_word >= 0)
		r->restore_equal = 0;
	if (!r->restore_frcnt_ok)
		return HVS6S_ERR_RESTORE_FRCNT;
	if (!r->restore_lptrs_ok)
		return HVS6S_ERR_RESTORE_LPTRS;
	if (r->diff_word >= 0)
		return HVS6S_ERR_RESTORE_WORD;
	return HVS6S_OK;
}

/*
 * The whole sequence: snapshot, place, swap, restore. The restore runs
 * whenever the snapshot was taken, whatever failed before it; a refused
 * snapshot writes nothing. r->err is the first failure (HVS6S_OK when all
 * passed), also returned.
 */
static inline enum hvs6s_err hvs6s_run(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
				       struct hvs6s_snap *s, struct hvs6s_report *r)
{
	enum hvs6s_err e;
	uint32_t start = 0;

	if (!r)
		return HVS6S_ERR_ARG;
	hvs6s_report_init(r);
	e = hvs6s_snapshot(io, c, s);
	if (e != HVS6S_OK) {
		hvs6s_fail(r, e);
		return r->err;
	}
	r->place_err = hvs6s_place_io(io, c, s, HVS6W_LIST_WORDS, &start, r);
	if (r->place_err != HVS6S_OK) {
		hvs6s_fail(r, r->place_err);
	} else {
		r->start = start;
		r->len = HVS6W_LIST_WORDS;
		e = hvs6s_swap(io, c, start, r);
		if (e != HVS6S_OK)
			hvs6s_fail(r, e);
	}
	e = hvs6s_restore(io, c, s, r);
	if (e != HVS6S_OK)
		hvs6s_fail(r, e);
	return r->err;
}

/* Line sink, as snapshot.h's: one NUL-terminated line without a newline. */
struct hvs6s_emit {
	void (*line)(void *ctx, const char *text);
	void *ctx;
};

static inline void hvs6s_regs_line(const struct hvs6s_emit *em, const char *step,
				   const struct hvs6s_regs *g)
{
	char b[160];

	(void)snprintf(b, sizeof(b),
		       HVS6S_TAG " %s frcnt=%u mode=%u lptrs=0x%08x lact=%u bgenb=%u status=0x%08x",
		       step, (unsigned)hvs6s_frcnt(g->status), (unsigned)hvs6s_mode(g->status),
		       (unsigned)g->lptrs, (unsigned)(g->dl & HVS6S_DL_LACT_MASK),
		       (unsigned)hvs6s_bgenb(g->ctrl1), (unsigned)g->status);
	em->line(em->ctx, b);
}

/*
 * The record as lines, one per step, each printed whether the step passed,
 * failed or did not run ("skip"). Returns the lines emitted.
 */
static inline size_t hvs6s_lines(const struct hvs6s_snap *s, const struct hvs6s_report *r,
				 const struct hvs6s_emit *em)
{
	char b[200];
	size_t n = 0;

	if (!s || !r || !em || !em->line)
		return 0;
	if (s->taken)
		(void)snprintf(b, sizeof(b),
			       HVS6S_TAG " snapshot ok lptrs=0x%08x fw_head=%u fw_len=%u entries=%u end=%u%s",
			       (unsigned)s->lptrs, (unsigned)s->fw_head, (unsigned)s->fw_len,
			       (unsigned)s->fw_entries, (unsigned)s->walk_stop,
			       s->fw_noend ? " window=noend" : "");
	else
		(void)snprintf(b, sizeof(b),
			       HVS6S_TAG " snapshot FAIL %s lptrs=0x%08x fw_head=%u entries=%u stop=%u word=0x%08x",
			       hvs6s_err_name(r->err), (unsigned)s->lptrs, (unsigned)s->fw_head,
			       (unsigned)s->fw_entries, (unsigned)s->walk_stop, (unsigned)s->walk_word);
	em->line(em->ctx, b);
	n++;
	if (!s->taken) {
		/* nothing placed, written or restored: each step says so */
		(void)snprintf(b, sizeof(b), HVS6S_TAG " place skip (no snapshot)");
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " write skip (not placed)");
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore skip (nothing written)");
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " result FAIL %s", hvs6s_err_name(r->err));
		em->line(em->ctx, b);
		return n + 4;
	}
	if (r->place_err == HVS6S_OK)
		(void)snprintf(b, sizeof(b), HVS6S_TAG " place ok start=%u len=%u skipped=%u",
			       (unsigned)r->start, (unsigned)r->len, (unsigned)r->place_skipped);
	else if (r->place_chan >= 0)
		(void)snprintf(b, sizeof(b), HVS6S_TAG " place FAIL %s ch=%d heade=%u lptrs=0x%08x",
			       hvs6s_err_name(r->place_err), r->place_chan, (unsigned)r->place_bad_word,
			       (unsigned)r->place_bad_val);
	else if (r->place_bad_word >= 0)
		(void)snprintf(b, sizeof(b), HVS6S_TAG " place FAIL %s word=%u val=0x%08x",
			       hvs6s_err_name(r->place_err), (unsigned)r->place_bad_word,
			       (unsigned)r->place_bad_val);
	else
		(void)snprintf(b, sizeof(b), HVS6S_TAG " place FAIL %s", hvs6s_err_name(r->place_err));
	em->line(em->ctx, b);
	n++;
	if (r->swapped) {
		(void)snprintf(b, sizeof(b),
			       HVS6S_TAG " write ok lptrs=0x%03x bus=0x%010llx clean=0x%llx+0x%llx",
			       (unsigned)r->lptrs_written, (unsigned long long)r->list_bus,
			       (unsigned long long)r->cleaned.start,
			       (unsigned long long)(r->cleaned.end - r->cleaned.start));
		em->line(em->ctx, b);
		hvs6s_regs_line(em, "at-write", &r->at_write);
		hvs6s_regs_line(em, "after", &r->after);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " lptrs-readback %s first=0x%08x poll=%u",
			       r->lptrs_ok ? "ok" : "FAIL", (unsigned)r->lptrs_first,
			       (unsigned)r->lptrs_poll);
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " frcnt-advance %s polls=%u", r->frcnt_ok ? "ok" : "FAIL",
			       (unsigned)r->frcnt_polls);
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " lact-follows %s (record only, not a latch proof)",
			       r->lact_follows ? "yes" : "no");
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " settle %s polls=%u",
			       r->settle_ok ? "ok" : (r->frcnt_ok ? "FAIL" : "skip"),
			       (unsigned)r->settle_polls);
		em->line(em->ctx, b);
		n += 7;
	} else {
		(void)snprintf(b, sizeof(b), HVS6S_TAG " write %s %s",
			       r->place_err == HVS6S_OK ? "FAIL" : "skip",
			       r->place_err == HVS6S_OK ? hvs6s_err_name(r->err) : "(not placed)");
		em->line(em->ctx, b);
		n++;
	}
	if (r->restored) {
		hvs6s_regs_line(em, "restore-after", &r->after_restore);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-frcnt %s polls=%u",
			       r->restore_frcnt_ok ? "ok" : "FAIL", (unsigned)r->restore_polls);
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-lptrs %s want=0x%08x got=0x%08x",
			       r->restore_lptrs_ok ? "ok" : "FAIL", (unsigned)s->lptrs,
			       (unsigned)r->after_restore.lptrs);
		em->line(em->ctx, b);
		if (r->diff_word < 0)
			(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-words ok fw %u..%u equal",
				       (unsigned)s->fw_head, (unsigned)(s->fw_head + s->fw_len - 1u));
		else
			(void)snprintf(b, sizeof(b),
				       HVS6S_TAG " restore-words FAIL word %u want=0x%08x got=0x%08x",
				       (unsigned)r->diff_word, (unsigned)r->diff_want, (unsigned)r->diff_got);
		em->line(em->ctx, b);
		n += 4;
	} else {
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore FAIL not run");
		em->line(em->ctx, b);
		n++;
	}
	(void)snprintf(b, sizeof(b), HVS6S_TAG " result %s %s", r->err == HVS6S_OK ? "ok" : "FAIL",
		       hvs6s_err_name(r->err));
	em->line(em->ctx, b);
	return n + 1;
}

#endif /* YAKOGL_BOARD_RPI5_SCANOUT_HVS6_DLIST_SWAP_H */
