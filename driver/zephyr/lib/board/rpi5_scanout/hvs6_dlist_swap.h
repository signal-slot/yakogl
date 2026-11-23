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
 *    (rpi:vc4_regs.h:564-622, :701-733). The read-back of LPTRS and FRCNT
 *    moving do not show that the HVS took the pointer. Linux completes a flip
 *    only once DISPx_DL.LACT equals the new head (rpi:vc4_crtc.c:832-855,
 *    driver/docs/rpi5-v3d71-facts.md, "HVS-written words of a plane entry, and
 *    list-pointer latching"), so that is the pickup criterion here; when in
 *    the frame the pointer takes effect is not assumed (no vblank-only latch).
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
 *     one write. Then three results, each on its own: LPTRS read back as
 *     written and FRCNT advanced (6-bit wrap aware), each within max_polls
 *     polls, and the pickup, DL.LACT equal to our start, within
 *     hvs6s_lact_limit polls (cfg.lact_polls, 0: max_polls) and, with
 *     cfg.lact_frames set, before FRCNT moved more than that many frames
 *     past the write (HVS6S_ERR_LACT_PICKUP otherwise). FRCNT, STATUS.MODE,
 *     LPTRS, DL and CTRL1.BGENB are recorded at the write and after it, the
 *     pickup's polls and FRCNT at the write and at the pickup. Then FRCNT
 *     must advance twice past the write before the old list is reused or
 *     checked.
 *  4. hvs6s_restore, always once the snapshot was taken, even after a
 *     failure in 2 or 3: the firmware's LPTRS written back, two FRCNT
 *     advances and DL.LACT back on the firmware's LPTRS.HEADE (a positive
 *     check, bounded as in 3; HVS6S_ERR_RESTORE_LACT otherwise), then LPTRS
 *     and the firmware's words [HEADE, HEADE + fw_len)
 *     compared byte for byte with the snapshot; the first differing word is
 *     named. Our own list's words are left in list memory, not cleared.
 *
 * Hardware-written words (swap-hwword, plan #390): the HVS may write a word
 * of a plane entry back while it scans it (a context or status word; Linux
 * writes POS3 as the context word 0xc0c0c0c0, rpi:vc4_plane.c:2184-2185, and
 * rpi5_scanout16 saw one word of the firmware's list change under a running
 * scanout). A byte-equal restore would then fail however correct the swap
 * was. cfg.hw (struct hvs6s_hw_set, at most HVS6S_HW_MAX entries) names
 * offsets from the start of a plane entry, each SKIP (left out of the
 * byte-equal compare and of the hash) or EXPECT (compared, a change reported
 * but not failing). It is empty by default: a zeroed cfg behaves as before.
 * Entries are found by CTL0.NEXT from the start of the reference words (the
 * snapshot's, or the words we wrote); the END word and words after a NEXT 0
 * are in no entry and never declared. The set applies to every entry of the
 * firmware's span at the restore and to our own entry at hvs6s_swap's
 * read-back. Every differing word is recorded (r->rdiff, r->ldiff:
 * HVS6S_DIFF_MAX kept, all counted) and printed by hvs6s_lines as
 *   dlswap restore-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<m>
 *   dlswap readback-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<m>
 * (<m> skip, expect or no; entry and off "none" outside any entry), then
 *   dlswap restore-diffs total=.. undeclared=.. skip=.. expect=.. shown=.. hwskip=<set>
 * (readback-diffs likewise, only when a word differed). Only an undeclared
 * word fails (HVS6S_ERR_RESTORE_WORD, HVS6S_ERR_LIST_READ); diff_word and
 * list_bad_word name the first undeclared one. Both compares now read every
 * word instead of stopping at the first difference. The set's text form
 * (hvs6s_hw_parse, hvs6s_hw_format) is "<off>:skip|expect[,...]", with ""
 * or "none" for the empty set. cfg.hw is checked by hvs6s_cfg_ok (a bad set
 * is HVS6S_ERR_ARG before anything is read or written).
 *
 * LACT pickup (swap-lact-lib, plan #390): a swap and a restore pass only
 * once DL.LACT reads the head just written (above, 3 and 4). Only reads were
 * added; the writes are the same. New: cfg.lact_polls, cfg.lact_frames
 * (zeroed: max_polls polls, no frame bound), HVS6S_ERR_LACT_PICKUP and
 * HVS6S_ERR_RESTORE_LACT (appended, the old codes keep their values), the
 * report's lact_pickup/_polls/_frcnt_write/_frcnt_pickup/_want/_seen and
 * restore_lact_ok/_polls/_want/_seen, restore_frcnt_write/_pickup. The old
 * lact_follows is kept and now equals lact_pickup. hvs6s_lines prints
 *   dlswap lact-pickup ok|FAIL polls=N frcnt=A..B want=W lact=L
 * where it printed "lact-follows ... (record only, not a latch proof)", and
 *   dlswap restore-lact ok|FAIL polls=N frcnt=A..B want=W lact=L
 * after restore-lptrs.
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
/* DISPx_DL.LACT, bits 11:0, the list the channel is scanning (scanout_alive.h's
 * SA_DL_LACT_MASK, rpi:vc4_regs.h:619). The pickup criterion of a swap and of
 * the restore (swap-lact-lib): LACT equal to the head just written. */
#define HVS6S_DL_LACT_MASK 0x00000fffu
/* FRCNT advances to wait past a pointer write before the old list is reused. */
#define HVS6S_SETTLE_FRAMES 2u
/* The record's prefix of every line hvs6s_lines prints. */
#define HVS6S_TAG "dlswap"

/* Hardware-written words: at most this many offsets in a set. */
#define HVS6S_HW_MAX 4u
/* The largest offset in an entry: CTL0.NEXT is 6 bits, entries are <= 63 words. */
#define HVS6S_HW_OFF_MAX 62u
/* Differing words kept (and printed) per compare; all are counted. */
#define HVS6S_DIFF_MAX 8u

enum hvs6s_hw_mode {
	HVS6S_HW_NO = 0, /* not declared: a difference fails */
	HVS6S_HW_SKIP,   /* left out of the compare and the hash */
	HVS6S_HW_EXPECT, /* compared, a change reported, never failing */
};

struct hvs6s_hw_word {
	uint32_t off; /* words from the start of a plane entry */
	enum hvs6s_hw_mode mode;
};

/* The declared hardware-written words; n == 0 is the empty set. */
struct hvs6s_hw_set {
	uint32_t n;
	struct hvs6s_hw_word w[HVS6S_HW_MAX];
};

/* One differing word. entry and off are -1 for a word in no entry. */
struct hvs6s_diff {
	uint32_t word; /* list-memory word */
	int32_t entry, off;
	uint32_t was, now;
	enum hvs6s_hw_mode hw;
};

struct hvs6s_diffs {
	uint32_t total;      /* differing words */
	uint32_t undeclared; /* of those, at no declared offset: these fail */
	uint32_t skip, expect;
	uint32_t hw_words;   /* words of the span at a declared offset, changed or not */
	uint32_t shown;      /* d[0..shown) */
	struct hvs6s_diff d[HVS6S_DIFF_MAX];
};

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
	struct hvs6s_hw_set hw; /* hardware-written words; zeroed = empty */
	/* LACT pickup (swap-lact-lib): polls from the LPTRS write allowed for
	 * DL.LACT to reach the new head; 0 means max_polls. */
	uint32_t lact_polls;
	/* and frames: the pickup fails once FRCNT moved more than this many
	 * frames past the write without it; 0 means no frame bound. */
	uint32_t lact_frames;
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
	HVS6S_ERR_LACT_PICKUP,  /* DL.LACT never reached our new head within the bound */
	HVS6S_ERR_RESTORE_LACT, /* restore: DL.LACT never came back to the firmware's head */
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
	case HVS6S_ERR_LACT_PICKUP: return "DL.LACT did not reach the new head";
	case HVS6S_ERR_RESTORE_LACT: return "restore: DL.LACT not back on the firmware's head";
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
	struct hvs6s_hw_set hw;      /* cfg.hw as the run used it (snapshot, swap, restore copy it) */

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
	uint32_t list_bad_word;      /* first undeclared word that did not read back (index in our list) */
	struct hvs6s_diffs ldiff;    /* every word of our list that did not read back as written */
	uint32_t lptrs_written;
	struct hvs6s_regs at_write;  /* sampled just before the LPTRS write */
	struct hvs6s_regs after;     /* sampled when both results were in, or at the timeout */
	int lptrs_ok;                /* result 1: LPTRS read back as written */
	uint32_t lptrs_first;        /* LPTRS read at once after the write */
	uint32_t lptrs_poll;         /* polls before it read back (0: at once) */
	int frcnt_ok;                /* result 2: FRCNT advanced */
	uint32_t frcnt_polls;        /* polls before it advanced */
	int lact_follows;            /* = lact_pickup, the old name kept for callers */
	int settle_ok;               /* two advances past the write */
	uint32_t settle_polls;
	/* result 3, a pass criterion: DL.LACT reached our start (the pickup) */
	int lact_pickup;
	uint32_t lact_polls;         /* polls from the write to the pickup, or spent without it */
	uint32_t lact_frcnt_write;   /* FRCNT sampled just before the LPTRS write */
	uint32_t lact_frcnt_pickup;  /* FRCNT at the pickup, or at the last poll without it */
	uint32_t lact_want;          /* the head LACT had to reach (our start) */
	uint32_t lact_seen;          /* DL.LACT at the pickup, or the last value read */

	/* restore */
	int restored;                /* the firmware's LPTRS was written back */
	int restore_frcnt_ok;
	uint32_t restore_polls;
	struct hvs6s_regs at_restore, after_restore;
	int restore_lptrs_ok;
	int restore_equal;           /* LPTRS and every kept word equal the snapshot */
	int64_t diff_word;           /* first undeclared differing list word, -1 when none */
	uint32_t diff_want, diff_got;
	struct hvs6s_diffs rdiff;    /* every differing word of the firmware's span */
	/* the restore's pickup, a pass criterion: DL.LACT back on the firmware's head */
	int restore_lact_ok;
	uint32_t restore_lact_polls;  /* polls from the write to the pickup, or spent */
	uint32_t restore_frcnt_write; /* FRCNT sampled just before the write */
	uint32_t restore_frcnt_pickup;/* FRCNT at the pickup, or at the last poll */
	uint32_t restore_lact_want;   /* the firmware's LPTRS.HEADE */
	uint32_t restore_lact_seen;   /* DL.LACT at the pickup, or the last value read */
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

static inline const char *hvs6s_hw_mode_name(enum hvs6s_hw_mode m)
{
	switch (m) {
	case HVS6S_HW_NO: return "no";
	case HVS6S_HW_SKIP: return "skip";
	case HVS6S_HW_EXPECT: return "expect";
	}
	return "?";
}

/* A set is valid with at most HVS6S_HW_MAX entries, each offset at most
 * HVS6S_HW_OFF_MAX, mode SKIP or EXPECT, no offset twice. NULL is empty. */
static inline int hvs6s_hw_ok(const struct hvs6s_hw_set *hw)
{
	if (!hw)
		return 1;
	if (hw->n > HVS6S_HW_MAX)
		return 0;
	for (uint32_t i = 0; i < hw->n; i++) {
		if (hw->w[i].off > HVS6S_HW_OFF_MAX ||
		    (hw->w[i].mode != HVS6S_HW_SKIP && hw->w[i].mode != HVS6S_HW_EXPECT))
			return 0;
		for (uint32_t j = 0; j < i; j++)
			if (hw->w[j].off == hw->w[i].off)
				return 0;
	}
	return 1;
}

/* The mode declared for an entry offset; HVS6S_HW_NO when none (or hw NULL). */
static inline enum hvs6s_hw_mode hvs6s_hw_lookup(const struct hvs6s_hw_set *hw, uint32_t off)
{
	if (!hw)
		return HVS6S_HW_NO;
	for (uint32_t i = 0; i < hw->n && i < HVS6S_HW_MAX; i++)
		if (hw->w[i].off == off)
			return hw->w[i].mode;
	return HVS6S_HW_NO;
}

/*
 * Word i of the n reference words w[], walked from w[0] entry by entry by
 * CTL0.NEXT: 1 with its entry's index and its offset in that entry; 0 for
 * the END word, a word after a NEXT 0, or i past n.
 */
static inline int hvs6s_entry_pos(const uint32_t *w, uint32_t n, uint32_t i, uint32_t *entry,
				  uint32_t *off)
{
	uint32_t pos = 0, e = 0;

	if (!w || i >= n)
		return 0;
	while (pos <= i) {
		uint32_t nx;

		if (w[pos] & HVS6_CTL0_END)
			return 0; /* the END word, or past it */
		nx = (w[pos] >> HVS6_CTL0_NEXT_SHIFT) & HVS6_CTL0_NEXT_MASK;
		if (nx == 0)
			return 0;
		if (i < pos + nx) {
			*entry = e;
			*off = i - pos;
			return 1;
		}
		pos += nx;
		e++;
	}
	return 0;
}

/* The mode of word i of the reference words; *entry and *off its place, -1
 * outside every entry. */
static inline enum hvs6s_hw_mode hvs6s_hw_of(const struct hvs6s_hw_set *hw, const uint32_t *ref,
					     uint32_t n, uint32_t i, int32_t *entry, int32_t *off)
{
	uint32_t e = 0, o = 0;

	*entry = -1;
	*off = -1;
	if (!hvs6s_entry_pos(ref, n, i, &e, &o))
		return HVS6S_HW_NO;
	*entry = (int32_t)e;
	*off = (int32_t)o;
	return hvs6s_hw_lookup(hw, o);
}

static inline void hvs6s_diffs_init(struct hvs6s_diffs *d)
{
	*d = (struct hvs6s_diffs){0};
}

/*
 * Word i of a compare: ref[0..n) the reference words (their entries and the
 * expected values), now the value read, base the list-memory word of ref[0].
 * A differing word is counted and kept in d. Returns its mode when it
 * differs, -1 when it is equal.
 */
static inline int hvs6s_diff_note(const struct hvs6s_hw_set *hw, const uint32_t *ref, uint32_t n,
				  uint32_t i, uint32_t base, uint32_t now, struct hvs6s_diffs *d)
{
	int32_t e = -1, o = -1;
	enum hvs6s_hw_mode m = HVS6S_HW_NO;
	int declared = hw && hw->n;

	if (declared) {
		m = hvs6s_hw_of(hw, ref, n, i, &e, &o);
		if (m != HVS6S_HW_NO)
			d->hw_words++;
	}
	if (ref[i] == now)
		return -1;
	if (!declared)
		(void)hvs6s_hw_of(hw, ref, n, i, &e, &o); /* its place, for the record */
	d->total++;
	if (m == HVS6S_HW_SKIP)
		d->skip++;
	else if (m == HVS6S_HW_EXPECT)
		d->expect++;
	else
		d->undeclared++;
	if (d->shown < HVS6S_DIFF_MAX) {
		d->d[d->shown] = (struct hvs6s_diff){ base + i, e, o, ref[i], now, m };
		d->shown++;
	}
	return (int)m;
}

/* The set as text into b: "none", or "<off>:<mode>" items joined by ','.
 * Returns b; 48 bytes hold any valid set ("62:expect" four times). */
static inline const char *hvs6s_hw_format(const struct hvs6s_hw_set *hw, char *b, size_t len)
{
	size_t at = 0;

	if (!b || !len)
		return "";
	b[0] = 0;
	if (!hw || !hw->n) {
		(void)snprintf(b, len, "none");
		return b;
	}
	for (uint32_t i = 0; i < hw->n && i < HVS6S_HW_MAX && at < len; i++) {
		int k = snprintf(b + at, len - at, "%s%u:%s", i ? "," : "", (unsigned)hw->w[i].off,
				 hvs6s_hw_mode_name(hw->w[i].mode));

		if (k < 0)
			break;
		at += (size_t)k;
	}
	return b;
}

/* Only the offsets of the set, "4,5" ("none" for the empty set). */
static inline const char *hvs6s_hw_offs(const struct hvs6s_hw_set *hw, char *b, size_t len)
{
	size_t at = 0;

	if (!b || !len)
		return "";
	b[0] = 0;
	if (!hw || !hw->n) {
		(void)snprintf(b, len, "none");
		return b;
	}
	for (uint32_t i = 0; i < hw->n && i < HVS6S_HW_MAX && at < len; i++) {
		int k = snprintf(b + at, len - at, "%s%u", i ? "," : "", (unsigned)hw->w[i].off);

		if (k < 0)
			break;
		at += (size_t)k;
	}
	return b;
}

static inline int hvs6s_hw_word_is(const char *t, const char *w)
{
	size_t i = 0;

	for (; w[i]; i++)
		if (t[i] != w[i])
			return 0;
	return 1;
}

/*
 * The set from its text: "" or "none" (empty), or "<off>:skip|expect" items
 * joined by ',', decimal offsets, no spaces. 0 with *out set; -1 with *out
 * empty for anything else (a bad item, an offset over HVS6S_HW_OFF_MAX, an
 * offset twice, more than HVS6S_HW_MAX items).
 */
static inline int hvs6s_hw_parse(const char *t, struct hvs6s_hw_set *out)
{
	struct hvs6s_hw_set h = {0};

	if (!out)
		return -1;
	*out = h;
	if (!t)
		return -1;
	if (t[0] == 0 || (hvs6s_hw_word_is(t, "none") && t[4] == 0))
		return 0;
	for (;;) {
		uint32_t off = 0;
		int digits = 0;

		while (*t >= '0' && *t <= '9') {
			off = off * 10u + (uint32_t)(*t - '0');
			if (off > HVS6S_HW_OFF_MAX)
				return -1;
			t++;
			digits++;
		}
		if (!digits || *t != ':' || h.n >= HVS6S_HW_MAX)
			return -1;
		t++;
		if (hvs6s_hw_word_is(t, "skip")) {
			h.w[h.n].mode = HVS6S_HW_SKIP;
			t += 4;
		} else if (hvs6s_hw_word_is(t, "expect")) {
			h.w[h.n].mode = HVS6S_HW_EXPECT;
			t += 6;
		} else {
			return -1;
		}
		h.w[h.n].off = off;
		h.n++;
		if (*t == 0)
			break;
		if (*t != ',')
			return -1;
		t++;
	}
	if (!hvs6s_hw_ok(&h))
		return -1;
	*out = h;
	return 0;
}

static inline int hvs6s_cfg_ok(const struct hvs6s_cfg *c)
{
	return c && c->chan < YD5_HVS6_NUM_CHANNELS && c->max_polls > 0 &&
	       c->mem_words >= HVS6S_KEPT && hvs6s_hw_ok(&c->hw);
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

/* The polls allowed for DL.LACT to reach a new head: cfg.lact_polls, or
 * max_polls when it is 0. */
static inline uint32_t hvs6s_lact_limit(const struct hvs6s_cfg *c)
{
	return c->lact_polls ? c->lact_polls : c->max_polls;
}

/* 3. Write our list at start, then LPTRS last; the three results (LPTRS read
 * back, FRCNT advanced, DL.LACT on our start); the settle. */
static inline enum hvs6s_err hvs6s_swap(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					uint32_t start, struct hvs6s_report *r)
{
	uint32_t list[HVS6W_LIST_WORDS];
	uint32_t f0, last, total = 0, lp_poll = 0, fr_poll = 0, la_poll = 0, la_lim;
	int n, lp_ok = 0, fr_ok = 0, la_ok = 0;
	uint64_t arm, bus;
	uint32_t i;

	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c) || !r)
		return HVS6S_ERR_ARG;
	r->hw = c->hw;
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
	/* every word read back; a declared hardware-written word that differs
	 * is recorded, only an undeclared one refuses the swap */
	hvs6s_diffs_init(&r->ldiff);
	{
		int bad = 0;

		for (i = 0; i < r->len; i++) {
			uint32_t got = io->read32(io->ctx, hvs6s_word_off(start + i));

			if (hvs6s_diff_note(&c->hw, list, r->len, i, start, got, &r->ldiff) ==
				    (int)HVS6S_HW_NO &&
			    !bad) {
				r->list_bad_word = i;
				bad = 1;
			}
		}
		if (bad)
			return HVS6S_ERR_LIST_READ;
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

	/* Results 1, 2 and 3, polled together, each kept on its own and each
	 * with its own bound: LPTRS and FRCNT max_polls, the LACT pickup
	 * hvs6s_lact_limit polls and, when set, lact_frames frames. */
	r->lptrs_first = io->read32(io->ctx, hvs6s_lptrs_reg(c->chan));
	r->lact_want = r->lptrs_written;
	r->lact_frcnt_write = f0;
	la_lim = hvs6s_lact_limit(c);
	last = f0;
	for (i = 0;; i++) {
		struct hvs6s_regs s;
		int lpfr_done, la_done;

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
		r->lact_seen = s.dl & HVS6S_DL_LACT_MASK;
		if (!la_ok && r->lact_seen == r->lact_want) {
			la_ok = 1;
			la_poll = i;
			r->lact_frcnt_pickup = last;
		}
		r->after = s;
		lpfr_done = (lp_ok && fr_ok) || i >= c->max_polls;
		la_done = la_ok || i >= la_lim || (c->lact_frames && total > c->lact_frames);
		if (lpfr_done && la_done)
			break;
		io->wait(io->ctx);
	}
	r->lptrs_ok = lp_ok;
	r->lptrs_poll = lp_poll;
	r->frcnt_ok = fr_ok;
	r->frcnt_polls = fr_ok ? fr_poll : i;
	r->lact_pickup = la_ok;
	r->lact_polls = la_ok ? la_poll : i;
	if (!la_ok)
		r->lact_frcnt_pickup = last;
	r->lact_follows = la_ok; /* the old field, now the pickup itself */

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
	if (!la_ok)
		return HVS6S_ERR_LACT_PICKUP;
	if (!r->settle_ok)
		return HVS6S_ERR_SETTLE;
	return HVS6S_OK;
}

/* 4. The firmware's LPTRS back; two advances and DL.LACT back on the
 * firmware's head, polled together, each with its own bound; then the
 * byte-equal compare. */
static inline enum hvs6s_err hvs6s_restore(const struct hvs6s_io *io, const struct hvs6s_cfg *c,
					   const struct hvs6s_snap *s, struct hvs6s_report *r)
{
	uint32_t i, got, last, total = 0, fr_poll = 0, la_lim;
	int fr_ok = 0, la_ok = 0;

	if (!hvs6s_io_ok(io) || !hvs6s_cfg_ok(c) || !s || !s->taken || !r)
		return HVS6S_ERR_ARG;
	hvs6s_sample(io, c->chan, &r->at_restore);
	io->barrier(io->ctx);
	io->write32(io->ctx, hvs6s_lptrs_reg(c->chan), s->lptrs);
	r->hw = c->hw;
	r->restored = 1;
	r->restore_lact_want = s->lptrs & YD5_HVS6_LPTRS_HEADE_MASK;
	r->restore_frcnt_write = hvs6s_frcnt(r->at_restore.status);
	la_lim = hvs6s_lact_limit(c);
	last = r->restore_frcnt_write;
	for (i = 0;; i++) {
		uint32_t f = hvs6s_frcnt(io->read32(io->ctx, hvs6s_status_reg(c->chan)));
		int fr_done, la_done;

		total += hvs6s_frdelta(last, f);
		last = f;
		if (!fr_ok && total >= HVS6S_SETTLE_FRAMES) {
			fr_ok = 1;
			fr_poll = i;
		}
		r->restore_lact_seen = io->read32(io->ctx, hvs6s_dl_reg(c->chan)) & HVS6S_DL_LACT_MASK;
		if (!la_ok && r->restore_lact_seen == r->restore_lact_want) {
			la_ok = 1;
			r->restore_lact_polls = i;
			r->restore_frcnt_pickup = f;
		}
		fr_done = fr_ok || i >= c->max_polls;
		la_done = la_ok || i >= la_lim || (c->lact_frames && total > c->lact_frames);
		if (fr_done && la_done)
			break;
		io->wait(io->ctx);
	}
	r->restore_frcnt_ok = fr_ok;
	r->restore_polls = fr_ok ? fr_poll : i;
	r->restore_lact_ok = la_ok;
	if (!la_ok) {
		r->restore_lact_polls = i;
		r->restore_frcnt_pickup = last;
	}
	hvs6s_sample(io, c->chan, &r->after_restore);

	/* Compared even after a stall: the record says what was there. */
	r->restore_equal = 1;
	r->restore_lptrs_ok = r->after_restore.lptrs == s->lptrs;
	if (!r->restore_lptrs_ok)
		r->restore_equal = 0;
	/* exactly the snapshot's words, [HEADE, HEADE + fw_len), every one read;
	 * the entries are the snapshot's, the declared offsets cfg.hw's */
	hvs6s_diffs_init(&r->rdiff);
	for (i = 0; i < s->fw_len; i++) {
		got = io->read32(io->ctx, hvs6s_word_off(s->fw_head + i));
		if (hvs6s_diff_note(&c->hw, s->fw, s->fw_len, i, s->fw_head, got, &r->rdiff) ==
			    (int)HVS6S_HW_NO &&
		    r->diff_word < 0) {
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
	if (!r->restore_lact_ok)
		return HVS6S_ERR_RESTORE_LACT;
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
	if (c)
		r->hw = c->hw;
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
 * One pickup result, step "lact-pickup" (the swap) or "restore-lact":
 *   dlswap <step> ok polls=<n> frcnt=<a>..<b> want=<head> lact=<seen>
 *   dlswap <step> FAIL polls=<n> frcnt=<a>..<b> want=<head> lact=<seen>
 * polls from the LPTRS write to the pickup (or spent without it), FRCNT at
 * the write and at the pickup (or at the last poll), the head LACT had to
 * reach and the LACT value read there (or last).
 */
static inline void hvs6s_lact_line(const struct hvs6s_emit *em, const char *step, int ok,
				   uint32_t polls, uint32_t fr_write, uint32_t fr_pick, uint32_t want,
				   uint32_t seen)
{
	char b[160];

	(void)snprintf(b, sizeof(b), HVS6S_TAG " %s %s polls=%u frcnt=%u..%u want=%u lact=%u", step,
		       ok ? "ok" : "FAIL", (unsigned)polls, (unsigned)fr_write, (unsigned)fr_pick,
		       (unsigned)want, (unsigned)seen);
	em->line(em->ctx, b);
}

/*
 * The differing words of one compare, kind "restore" or "readback": one
 * "dlswap <kind>-diff word ..." line per kept word, then the count line
 * "dlswap <kind>-diffs total=.. undeclared=.. skip=.. expect=.. shown=..
 * hwskip=<set>". Returns the lines emitted.
 */
static inline size_t hvs6s_diff_lines(const struct hvs6s_emit *em, const char *kind,
				      const struct hvs6s_diffs *d, const struct hvs6s_hw_set *hw)
{
	char b[256], set[48], e[12], o[12];
	size_t n = 0;

	for (uint32_t i = 0; i < d->shown && i < HVS6S_DIFF_MAX; i++) {
		const struct hvs6s_diff *x = &d->d[i];

		if (x->entry < 0) {
			(void)snprintf(e, sizeof(e), "none");
			(void)snprintf(o, sizeof(o), "none");
		} else {
			(void)snprintf(e, sizeof(e), "%d", (int)x->entry);
			(void)snprintf(o, sizeof(o), "%d", (int)x->off);
		}
		(void)snprintf(b, sizeof(b),
			       HVS6S_TAG " %s-diff word %u entry %s off %s was 0x%08x now 0x%08x hw=%s", kind,
			       (unsigned)x->word, e, o, (unsigned)x->was, (unsigned)x->now,
			       hvs6s_hw_mode_name(x->hw));
		em->line(em->ctx, b);
		n++;
	}
	(void)snprintf(b, sizeof(b),
		       HVS6S_TAG " %s-diffs total=%u undeclared=%u skip=%u expect=%u shown=%u hwskip=%s",
		       kind, (unsigned)d->total, (unsigned)d->undeclared, (unsigned)d->skip,
		       (unsigned)d->expect, (unsigned)d->shown, hvs6s_hw_format(hw, set, sizeof(set)));
	em->line(em->ctx, b);
	return n + 1;
}

/*
 * The record as lines, one per step, each printed whether the step passed,
 * failed or did not run ("skip"). The read-back's differing words follow the
 * write line when one differed; the restore's, with their count line, follow
 * restore-lptrs. r->hw, the set the run used, is named on the snapshot,
 * restore-diffs and restore-words lines. Returns the lines emitted.
 */
static inline size_t hvs6s_lines(const struct hvs6s_snap *s, const struct hvs6s_report *r,
				 const struct hvs6s_emit *em)
{
	char b[256], set[48], offs[24];
	size_t n = 0;

	if (!s || !r || !em || !em->line)
		return 0;
	(void)hvs6s_hw_format(&r->hw, set, sizeof(set));
	if (s->taken)
		(void)snprintf(b, sizeof(b),
			       HVS6S_TAG " snapshot ok lptrs=0x%08x fw_head=%u fw_len=%u entries=%u end=%u%s hwskip=%s",
			       (unsigned)s->lptrs, (unsigned)s->fw_head, (unsigned)s->fw_len,
			       (unsigned)s->fw_entries, (unsigned)s->walk_stop,
			       s->fw_noend ? " window=noend" : "", set);
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
		hvs6s_lact_line(em, "lact-pickup", r->lact_pickup, r->lact_polls, r->lact_frcnt_write,
				r->lact_frcnt_pickup, r->lact_want, r->lact_seen);
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
	if (r->ldiff.total)
		n += hvs6s_diff_lines(em, "readback", &r->ldiff, &r->hw);
	if (r->restored) {
		hvs6s_regs_line(em, "restore-after", &r->after_restore);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-frcnt %s polls=%u",
			       r->restore_frcnt_ok ? "ok" : "FAIL", (unsigned)r->restore_polls);
		em->line(em->ctx, b);
		(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-lptrs %s want=0x%08x got=0x%08x",
			       r->restore_lptrs_ok ? "ok" : "FAIL", (unsigned)s->lptrs,
			       (unsigned)r->after_restore.lptrs);
		em->line(em->ctx, b);
		hvs6s_lact_line(em, "restore-lact", r->restore_lact_ok, r->restore_lact_polls,
				r->restore_frcnt_write, r->restore_frcnt_pickup, r->restore_lact_want,
				r->restore_lact_seen);
		n += hvs6s_diff_lines(em, "restore", &r->rdiff, &r->hw);
		if (r->diff_word < 0 && !r->hw.n)
			(void)snprintf(b, sizeof(b), HVS6S_TAG " restore-words ok fw %u..%u equal",
				       (unsigned)s->fw_head, (unsigned)(s->fw_head + s->fw_len - 1u));
		else if (r->diff_word < 0)
			(void)snprintf(b, sizeof(b),
				       HVS6S_TAG " restore-words ok fw %u..%u byte-equal except hw words off %s "
				       "(%u differed) hwskip=%s",
				       (unsigned)s->fw_head, (unsigned)(s->fw_head + s->fw_len - 1u),
				       hvs6s_hw_offs(&r->hw, offs, sizeof(offs)),
				       (unsigned)(r->rdiff.skip + r->rdiff.expect), set);
		else
			(void)snprintf(b, sizeof(b),
				       HVS6S_TAG " restore-words FAIL word %u want=0x%08x got=0x%08x hwskip=%s",
				       (unsigned)r->diff_word, (unsigned)r->diff_want, (unsigned)r->diff_got, set);
		em->line(em->ctx, b);
		n += 5;
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
