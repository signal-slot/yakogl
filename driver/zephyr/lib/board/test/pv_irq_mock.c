/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_irq_mock: a fake pixel valve register file (PV_INTEN, PV_INTSTAT with
 * write-1-to-clear) that raises vblank after k polls, never raises it, or
 * holds other status bits that must survive the acknowledge. Built by
 * tools/v3d/check71.d/pv_irq.sh.
 */
#include <stdio.h>
#include <string.h>

#include "pv_irq.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

#define OTHERS (PV_INT_VSYNC_START | PV_INT_HSYNC_START | PV_INT_VID_IDLE | 0x400u)

struct fake {
	uint32_t inten, intstat;
	uint32_t polls;       /* reads of INTSTAT */
	uint32_t raise_after; /* vblank raised on this read; 0 = never */
	uint32_t period;      /* and every period reads after it; 0 = once */
	uint32_t writes;
	uint32_t bad_off;
};

static uint32_t f_read(void *c, uint32_t off)
{
	struct fake *f = c;

	if (off == PV_IRQ_INTEN)
		return f->inten;
	if (off != PV_IRQ_INTSTAT) {
		f->bad_off++;
		return 0;
	}
	f->polls++;
	if (f->raise_after && f->polls >= f->raise_after &&
	    (f->polls == f->raise_after ||
	     (f->period && (f->polls - f->raise_after) % f->period == 0)))
		f->intstat |= PV_INT_VFP_START;
	return f->intstat;
}

static void f_write(void *c, uint32_t off, uint32_t v)
{
	struct fake *f = c;

	f->writes++;
	if (off == PV_IRQ_INTEN)
		f->inten = v;
	else if (off == PV_IRQ_INTSTAT)
		f->intstat &= ~v; /* write 1 to clear */
	else
		f->bad_off++;
}

int main(void)
{
	struct fake f;
	struct pv_io io = {f_read, f_write, &f};
	struct pv_irq_status s;
	struct pv_irq_write w[2];

	/* register map, as cited */
	CHECK(PV_IRQ_INTEN == 0x24 && PV_IRQ_INTSTAT == 0x28 && PV_IRQ_STAT == 0x2c);
	CHECK(PV_INT_VFP_START == 0x80 && PV_INT_VSYNC_START == 0x10 && PV_INT_VID_IDLE == 0x200);
	CHECK(PV_INT_ALL == (PV_INT_VID_IDLE | PV_INT_VFP_END | PV_INT_VFP_START |
			     PV_INT_VACT_START | PV_INT_VBP_START | PV_INT_VSYNC_START |
			     PV_INT_HFP_START | PV_INT_HACT_START | PV_INT_HBP_START |
			     PV_INT_HSYNC_START));

	/* decode */
	s = pv_irq_decode(0);
	CHECK(!s.vblank && !s.vsync_start && !s.vid_idle && !s.other && !s.unknown);
	s = pv_irq_decode(PV_INT_VFP_START);
	CHECK(s.vblank && !s.vsync_start && !s.other && s.raw == PV_INT_VFP_START);
	s = pv_irq_decode(PV_INT_VSYNC_START | PV_INT_HSYNC_START | 0x400u);
	CHECK(!s.vblank && s.vsync_start && s.other == PV_INT_HSYNC_START && s.unknown == 0x400u);
	s = pv_irq_decode(0xffffffffu);
	CHECK(s.vblank && s.vsync_start && s.vid_idle && s.unknown == ~PV_INT_ALL);

	/* ack list: only wanted bits that are set, one write, nothing else */
	CHECK(pv_irq_ack_list(0, PV_INT_VBLANK, w, 2) == 0);
	CHECK(pv_irq_ack_list(PV_INT_HSYNC_START, PV_INT_VBLANK, w, 2) == 0);
	CHECK(pv_irq_ack_list(PV_INT_VFP_START | PV_INT_HSYNC_START, PV_INT_VBLANK, w, 2) == 1);
	CHECK(w[0].off == PV_IRQ_INTSTAT && w[0].val == PV_INT_VFP_START);
	CHECK(pv_irq_ack_list(0xffffffffu, 0xffffffffu, w, 2) == 1 && w[0].val == PV_INT_ALL);
	CHECK(pv_irq_ack_list(PV_INT_VFP_START, PV_INT_VBLANK, w, 0) == 0);
	CHECK(pv_irq_ack_list(PV_INT_VFP_START, PV_INT_VBLANK, NULL, 2) == 0);
	CHECK(pv_irq_ack_list(0x400u, 0x400u, w, 2) == 0); /* not an interrupt bit */
	w[0] = pv_irq_enable_write(PV_INT_VBLANK);
	CHECK(w[0].off == PV_IRQ_INTEN && w[0].val == PV_INT_VFP_START);
	CHECK(pv_irq_enable_write(0xffffffffu).val == PV_INT_ALL);

	/* vblank after k polls */
	for (uint32_t k = 1; k <= 5; k++) {
		memset(&f, 0, sizeof(f));
		f.raise_after = k;
		CHECK(pv_wait_vblank(&io, 10) == 1);
		CHECK(f.polls == k);   /* stopped at the first sight */
		CHECK(f.intstat == 0); /* acknowledged */
		CHECK(f.writes == 1 && f.bad_off == 0);
	}
	/* one poll short of the limit, then enough */
	memset(&f, 0, sizeof(f));
	f.raise_after = 7;
	CHECK(pv_wait_vblank(&io, 6) == PV_IRQ_TIMEOUT);
	CHECK(f.polls == 6 && f.writes == 0);
	CHECK(pv_wait_vblank(&io, 1) == 1 && f.polls == 7);

	/* never raised: bounded, no write */
	memset(&f, 0, sizeof(f));
	CHECK(pv_wait_vblank(&io, 1000) == PV_IRQ_TIMEOUT);
	CHECK(f.polls == 1000 && f.writes == 0);
	CHECK(pv_wait_vblank(&io, 0) == PV_IRQ_TIMEOUT && f.polls == 1000);

	/* other bits set must not be cleared, with and without vblank */
	memset(&f, 0, sizeof(f));
	f.intstat = OTHERS;
	CHECK(pv_wait_vblank(&io, 20) == PV_IRQ_TIMEOUT);
	CHECK(f.intstat == OTHERS && f.writes == 0);
	f.raise_after = 1;
	f.polls = 0;
	CHECK(pv_wait_vblank(&io, 20) == 1);
	CHECK(f.intstat == OTHERS);
	CHECK(f.writes == 1 && f.inten == 0);

	/* count: vblank every 4th poll in 20 polls = 5 frames, others kept */
	memset(&f, 0, sizeof(f));
	f.intstat = PV_INT_VBP_START;
	f.raise_after = 4;
	f.period = 4;
	CHECK(pv_count_vblanks(&io, 20) == 5);
	CHECK(f.intstat == PV_INT_VBP_START && f.polls == 20);
	memset(&f, 0, sizeof(f));
	CHECK(pv_count_vblanks(&io, 20) == 0 && f.polls == 20);
	CHECK(pv_count_vblanks(&io, 0) == 0);

	/* a missing accessor never crashes */
	{
		struct pv_io bad = {NULL, f_write, &f};

		CHECK(pv_wait_vblank(&bad, 5) == PV_IRQ_TIMEOUT);
		CHECK(pv_wait_vblank(NULL, 5) == PV_IRQ_TIMEOUT);
		CHECK(pv_count_vblanks(&bad, 5) == 0);
	}

	printf("pv_irq_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
