/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The V3D's power and clock on the BCM2837 (vc4.h, struct vc4_power), two
 * ways, to choose between on the board (docs/notes/pre-study-rpi-zero-2w.md
 * Q3; which one day one needs is a board answer):
 *
 *   the firmware's property mailbox, as the Zero 2 W's device tree routes
 *   it (Linux v6.18 drivers/pmdomain/bcm/raspberrypi-power.c and
 *   drivers/clk/bcm/clk-raspberrypi.c);
 *
 *   the PM block and the AXI async bridge directly, as Linux does where the
 *   device tree names the PM block (drivers/pmdomain/bcm/bcm2835-power.c).
 *
 * The order of the steps and the register bits are read from those files;
 * the code is ours.
 */
#include <errno.h>

#include "vc4.h"

/* ---- the firmware ---- */

static int fw_clock_tag(struct vc4_power_fw *f, uint32_t tag, uint32_t *val)
{
	/* {id, value, 0}: struct raspberrypi_firmware_prop (clk-raspberrypi.c:184-188) */
	uint32_t v[3] = { VC4_FW_CLK_V3D, *val, 0 };
	int ret = f->tag(tag, v, 3);
	if (ret == 0)
		*val = v[1];
	return ret;
}

static int fw_domain(struct vc4_power_fw *f, bool on)
{
	/* struct rpi_power_domain_packet {domain, state} (raspberrypi-power.c:42-45) */
	uint32_t v[2] = { VC4_FW_DOMAIN_V3D, on ? 1u : 0u };
	return f->tag(RPI_TAG_SET_DOMAIN_STATE, v, 2);
}

int vc4_fw_clock(void *ctx, bool on)
{
	uint32_t state = on ? RPI_CLOCK_STATE_ON : 0;
	return fw_clock_tag(ctx, RPI_TAG_SET_CLOCK_STATE, &state);
}

int vc4_power_fw_on(void *ctx)
{
	struct vc4_power_fw *f = ctx;
	int ret = fw_domain(f, true);
	if (ret)
		return ret;
	f->set_hz = 0;
	if (f->rate_hz) {
		uint32_t hz = f->rate_hz;
		if (hz == VC4_FW_RATE_MAX) {
			hz = 0;
			ret = fw_clock_tag(f, RPI_TAG_GET_MAX_CLOCK_RATE, &hz);
			if (ret)
				return ret;
			if (hz == 0)
				return -EIO; /* no maximum: the firmware has no such clock */
		}
		f->set_hz = hz;
		ret = fw_clock_tag(f, RPI_TAG_SET_CLOCK_RATE, &hz);
		if (ret)
			return ret;
	}
	return vc4_fw_clock(f, true);
}

int vc4_power_fw_off(void *ctx)
{
	struct vc4_power_fw *f = ctx;
	int ret = vc4_fw_clock(f, false);
	if (ret)
		return ret;
	return fw_domain(f, false);
}

int vc4_power_fw_state(void *ctx, bool *on)
{
	struct vc4_power_fw *f = ctx;
	uint32_t v[2] = { VC4_FW_DOMAIN_V3D, ~0u };
	int ret = f->tag(RPI_TAG_GET_DOMAIN_STATE, v, 2);
	if (ret)
		return ret;
	if (v[1] == ~0u)
		return -ENOTSUP;
	*on = (v[1] & 1u) != 0;
	return 0;
}

/* ---- the PM block and the bridge ---- */

static inline uint32_t rd(const struct vc4_io *io, uint32_t off)
{
	return io->read32(io->ctx, off);
}

/* PM and ASB writes both carry the password (bcm2835-power.c:110,176). */
static inline void wr_pw(const struct vc4_io *io, uint32_t off, uint32_t val)
{
	io->write32(io->ctx, off, PM_PASSWORD | val);
}

static inline void delay(const struct vc4_io *io, uint32_t us)
{
	if (io->delay_us)
		io->delay_us(io->ctx, us);
}

/* Poll `off` until (value & mask) == want: up to `us` + 1 reads 1 us apart. */
static bool poll(const struct vc4_io *io, uint32_t off, uint32_t mask, uint32_t want, uint32_t us)
{
	for (uint32_t i = 0;; i++) {
		if ((rd(io, off) & mask) == want)
			return true;
		if (i >= us)
			return false;
		delay(io, 1);
	}
}

/* bcm2835_asb_control (bcm2835-power.c:152-185): start (clear REQ_STOP, wait
 * for ACK to drop) or stop (set it, wait for ACK) one bridge port. */
static int asb_control(const struct vc4_io *asb, uint32_t reg, bool start)
{
	uint32_t v = rd(asb, reg);
	v = start ? (v & ~ASB_REQ_STOP) : (v | ASB_REQ_STOP);
	wr_pw(asb, reg, v);
	return poll(asb, reg, ASB_ACK, start ? 0 : ASB_ACK, 1) ? 0 : -ETIMEDOUT;
}

/* bcm2835_power_power_on for PM_GRAFX (bcm2835-power.c:217-285). */
static int grafx_on(const struct vc4_io *pm)
{
	if (rd(pm, PM_GRAFX) & PM_POWUP)
		return 0; /* the firmware powered it: leave it (:230-232) */

	/* Close the power switches with the inrush current limit stepped up
	 * from 3.5 mA to 20 mA, waiting for POWOK at each step. Linux's loop
	 * does not stop at the first POWOK (:239-251): every step is written
	 * and the last one's wait decides. That is the sequence that has run on
	 * boards, so it is kept as it is. */
	bool powok = false;
	for (uint32_t inrush = PM_INRUSH_3_5_MA; inrush <= PM_INRUSH_20_MA; inrush++) {
		wr_pw(pm, PM_GRAFX,
		      (rd(pm, PM_GRAFX) & ~PM_INRUSH_MASK) | (inrush << PM_INRUSH_SHIFT) | PM_POWUP);
		powok = poll(pm, PM_GRAFX, PM_POWOK, PM_POWOK, 3);
	}
	if (!powok) {
		wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~(PM_POWUP | PM_INRUSH_MASK));
		return -ETIMEDOUT;
	}
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) | PM_ISPOW);  /* electrical isolation off */
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) | PM_MEMREP); /* repair the memories */
	if (!poll(pm, PM_GRAFX, PM_MRDONE, PM_MRDONE, 1)) {
		wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_ISPOW);
		wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~(PM_POWUP | PM_INRUSH_MASK));
		return -ETIMEDOUT;
	}
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) | PM_ISFUNC); /* functional isolation off */
	return 0;
}

/* bcm2835_power_power_off for PM_GRAFX (bcm2835-power.c:197-215). */
static void grafx_off(const struct vc4_io *pm)
{
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_ISFUNC);
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_ISPOW);
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_POWUP);
}

int vc4_power_pm_on(void *ctx)
{
	const struct vc4_power_pm *p = ctx;
	const struct vc4_io *pm = p->pm, *asb = p->asb;
	int ret;

	if (rd(asb, ASB_AXI_BRDG_ID) != ASB_BRDG_ID)
		return -ENODEV;
	/* GRAFX is the V3D domain's parent (bcm2835-power.c:617,669) */
	ret = grafx_on(pm);
	if (ret)
		return ret;

	/* bcm2835_asb_power_on (:287-340): 32 clocks of reset under a running
	 * clock, then the reset released with the clock stopped */
	ret = p->clock(p->clock_ctx, true);
	if (ret)
		return ret;
	delay(pm, 1);
	/* a clock-off's answer is not checked, as clk_disable_unprepare
	 * returns none */
	p->clock(p->clock_ctx, false);
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) | PM_V3DRSTN);
	ret = p->clock(p->clock_ctx, true);
	if (ret)
		goto err_reset;
	ret = asb_control(asb, ASB_V3D_M_CTRL, true);
	if (ret)
		goto err_clock;
	ret = asb_control(asb, ASB_V3D_S_CTRL, true);
	if (ret) {
		asb_control(asb, ASB_V3D_M_CTRL, false);
		goto err_clock;
	}
	return 0;

err_clock:
	p->clock(p->clock_ctx, false);
err_reset:
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_V3DRSTN);
	return ret;
}

int vc4_power_pm_off(void *ctx)
{
	const struct vc4_power_pm *p = ctx;
	const struct vc4_io *pm = p->pm, *asb = p->asb;
	int ret;

	/* bcm2835_asb_power_off (:342-371) */
	ret = asb_control(asb, ASB_V3D_S_CTRL, false);
	if (ret)
		return ret;
	ret = asb_control(asb, ASB_V3D_M_CTRL, false);
	if (ret) {
		asb_control(asb, ASB_V3D_S_CTRL, true);
		return ret;
	}
	p->clock(p->clock_ctx, false);
	wr_pw(pm, PM_GRAFX, rd(pm, PM_GRAFX) & ~PM_V3DRSTN);
	/* then the parent island, once its only child is off */
	grafx_off(pm);
	return 0;
}
