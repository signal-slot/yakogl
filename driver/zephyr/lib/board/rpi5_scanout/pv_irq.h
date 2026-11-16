/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_irq: BCM2712 pixel valve interrupt enable and status (vblank, VFP start),
 * the decode of a status word, the write-1-to-clear acknowledge list and a
 * bounded poll for the next vblank. Header only, C11, no Zephyr, no MMIO: all
 * register access goes through struct pv_io (two function pointers and a
 * context), so the board layer passes MMIO accessors and the mock a fake file.
 * No interrupt is wired here; the poll reads PV_INTSTAT.
 *
 * Sources. driver/docs/rpi5-v3d71-facts.md section 10 is silent on the pixel
 * valve registers: it gives only the nodes (pixelvalve0 0x7c410000 SPI 101,
 * pixelvalve1 0x7c411000 SPI 110, citation bcm2712.dtsi:292-302). The offsets
 * and bits below are facts read from Linux rpi-6.12.y commit 5bd564aa (facts
 * file, sources table), drivers/gpu/drm/vc4/:
 *   vc4_regs.h:211  PV_INTEN   0x24
 *   vc4_regs.h:212  PV_INTSTAT 0x28
 *   vc4_regs.h:213-222 the bits, VID_IDLE bit 9 down to HSYNC_START bit 0
 *   vc4_regs.h:224  PV_STAT    0x2c
 *   vc4_crtc.c:806  vblank enable writes PV_INTEN = PV_INT_VFP_START
 *   vc4_crtc.c:825  vblank disable writes PV_INTEN = 0
 *   vc4_crtc.c:885-889 the handler reads PV_INTSTAT and writes the handled
 *                   bit back to PV_INTSTAT (write 1 to clear)
 *   vc4_crtc.c:1364-1365 the BCM2712 pixel valves bind this same code
 *                   (bcm2712_pv0_data, bcm2712_pv1_data), so the layout is
 *                   the one of the earlier chips.
 * Linux uses VFP_START (the start of the vertical front porch, the end of the
 * active lines) as its vblank event; this header does the same.
 *
 * unverified (no board read yet): that the BCM2712 pixel valve has exactly
 * these offsets and bits (Linux has one register map and no per-chip table
 * for them); that PV_INTSTAT bits are write-1-to-clear on it (implied by
 * vc4_crtc.c:889); that PV_INTSTAT is set while PV_INTEN is clear (the poll
 * relies on it; if not, write pv_irq_enable_write(PV_INT_VBLANK) first).
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_PV_IRQ_H
#define YAKOGL_BOARD_RPI5_SCANOUT_PV_IRQ_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Register offsets from the pixel valve base (vc4_regs.h:211,212,224). */
#define PV_IRQ_INTEN   0x24u
#define PV_IRQ_INTSTAT 0x28u
#define PV_IRQ_STAT    0x2cu

/* PV_INTEN / PV_INTSTAT bits (vc4_regs.h:213-222). */
#define PV_INT_VID_IDLE    (1u << 9)
#define PV_INT_VFP_END     (1u << 8)
#define PV_INT_VFP_START   (1u << 7)
#define PV_INT_VACT_START  (1u << 6)
#define PV_INT_VBP_START   (1u << 5)
#define PV_INT_VSYNC_START (1u << 4)
#define PV_INT_HFP_START   (1u << 3)
#define PV_INT_HACT_START  (1u << 2)
#define PV_INT_HBP_START   (1u << 1)
#define PV_INT_HSYNC_START (1u << 0)
#define PV_INT_ALL         0x3ffu

/* The vblank event: Linux's choice (vc4_crtc.c:806,888). */
#define PV_INT_VBLANK PV_INT_VFP_START

#define PV_IRQ_TIMEOUT (-1)

struct pv_io {
	uint32_t (*read)(void *ctx, uint32_t off);
	void (*write)(void *ctx, uint32_t off, uint32_t val);
	void *ctx;
};

struct pv_irq_status {
	bool vblank;      /* VFP_START */
	bool vsync_start; /* VSYNC_START */
	bool vid_idle;
	uint32_t raw;     /* status as read */
	uint32_t other;   /* set interrupt bits that are none of the three above */
	uint32_t unknown; /* set bits outside PV_INT_ALL (not interrupt bits) */
};

struct pv_irq_write {
	uint32_t off;
	uint32_t val;
};

static inline struct pv_irq_status pv_irq_decode(uint32_t status)
{
	struct pv_irq_status s;

	s.raw = status;
	s.vblank = (status & PV_INT_VFP_START) != 0;
	s.vsync_start = (status & PV_INT_VSYNC_START) != 0;
	s.vid_idle = (status & PV_INT_VID_IDLE) != 0;
	s.other = status & PV_INT_ALL &
		  ~(PV_INT_VFP_START | PV_INT_VSYNC_START | PV_INT_VID_IDLE);
	s.unknown = status & ~PV_INT_ALL;
	return s;
}

/*
 * The write list that acknowledges the bits of `want` that are set in
 * `status`: one write of PV_INTSTAT with exactly those bits (write 1 to
 * clear, so no other bit is touched). Bits outside PV_INT_ALL are never
 * written. Returns the number of writes put into out (0 or 1); 0 when nothing
 * wanted is set or max is 0.
 */
static inline size_t pv_irq_ack_list(uint32_t status, uint32_t want, struct pv_irq_write *out,
				     size_t max)
{
	uint32_t bits = status & want & PV_INT_ALL;

	if (!bits || !out || max < 1)
		return 0;
	out[0].off = PV_IRQ_INTSTAT;
	out[0].val = bits;
	return 1;
}

/* The write that enables the given interrupt bits only (vc4_crtc.c:806). */
static inline struct pv_irq_write pv_irq_enable_write(uint32_t bits)
{
	struct pv_irq_write w = {PV_IRQ_INTEN, bits & PV_INT_ALL};

	return w;
}

static inline void pv_irq_apply(const struct pv_io *io, const struct pv_irq_write *w, size_t n)
{
	for (size_t i = 0; i < n; i++)
		io->write(io->ctx, w[i].off, w[i].val);
}

/*
 * Poll PV_INTSTAT at most max_polls times for the next vblank. On the first
 * poll that shows VFP_START the bit (and only it) is acknowledged and 1 is
 * returned. PV_IRQ_TIMEOUT when it never shows (also for max_polls 0 or a
 * missing accessor). A poll is one register read and takes no set time: the
 * caller sizes max_polls for a frame (16.7 ms at 60 Hz).
 */
static inline int pv_wait_vblank(const struct pv_io *io, uint32_t max_polls)
{
	if (!io || !io->read || !io->write)
		return PV_IRQ_TIMEOUT;
	for (uint32_t i = 0; i < max_polls; i++) {
		struct pv_irq_write w[1];
		uint32_t st = io->read(io->ctx, PV_IRQ_INTSTAT);

		if (pv_irq_ack_list(st, PV_INT_VBLANK, w, 1)) {
			pv_irq_apply(io, w, 1);
			return 1;
		}
	}
	return PV_IRQ_TIMEOUT;
}

/*
 * Poll exactly max_polls times and acknowledge each vblank seen; returns the
 * number of polls that saw one (at most one frame per poll, so it undercounts
 * when polls are slower than frames). 0 means none; it is never a timeout
 * value.
 */
static inline int pv_count_vblanks(const struct pv_io *io, uint32_t max_polls)
{
	int n = 0;

	if (!io || !io->read || !io->write)
		return 0;
	for (uint32_t i = 0; i < max_polls; i++) {
		struct pv_irq_write w[1];
		uint32_t st = io->read(io->ctx, PV_IRQ_INTSTAT);

		if (pv_irq_ack_list(st, PV_INT_VBLANK, w, 1)) {
			pv_irq_apply(io, w, 1);
			n++;
		}
	}
	return n;
}

#endif /* YAKOGL_BOARD_RPI5_SCANOUT_PV_IRQ_H */
