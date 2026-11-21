/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_dispregs: a read-only look at the display state the firmware left on
 * the Pi 5 (docs/notes/rpi5-display.md section 4, Q3, Q5 and Q6). It writes no
 * display, HVS, pixel valve, HDMI or PHY register: the only writes are the PM
 * watchdog's (the board layer, rpi_wdog.c) and the mailbox's (the doorbell and
 * the request buffer). Every request tag is a GET tag.
 *
 * Steps, each printing its line(s) whatever happened before:
 *
 *   map     the mailbox, the HVS (0x1a000 bytes) and pixel valves 0 and 1
 *   hvs     VERSION, then CTRL0 (ENB, FWIDTH 28:16, LINES 12:0) and STATUS of
 *           channels 0 and 1: the size the HVS composes (Q3)
 *   pv<n>   PV_CONTROL, PV_V_CONTROL, PV_HORZA/B, PV_VERTA/B raw, then
 *           decoded with pv_hdmi.h (CLK_SELECT, PIXEL_REP, format) and
 *           pv_timing.h (active width and height, the VIC if the timing is
 *           one of its three modes). PV_MUX and PV_STAT are not read: no
 *           offset of them is in the repo (rpi5_scanout/pv_hdmi.h has
 *           PV_CONTROL, PV_V_CONTROL and the four timing words only).
 *   clocks  Q5: GET_CLOCK_STATE 0x00030001, GET_CLOCK_RATE 0x00030002,
 *           GET_CLOCK_RATE_MEASURED 0x00030047, GET_MAX_CLOCK_RATE 0x00030004
 *           (rpi5_scanout/fw_clk.h:29-35, citing raspberrypi-firmware.h) for
 *           clocks 4 (HVS core), 13 (HDMI HSM, m2mc), 14 (HDMI bvb) and 16
 *           (HVS disp): fw_clk.h's YD5_CLK_* names.
 *   disp    Q6: GET_NUM_DISPLAYS 0x00040013 and GET_DISPLAY_SETTINGS
 *           0x00040014 (a 64-byte answer, raw words; asked for the display
 *           the firmware has selected: SET_DISPLAY_NUM 0x00048013 is a setter
 *           and is never sent). Tag ids from driver/docs/rpi5-display.md:203
 *           (rpi:include/soc/bcm2835/raspberrypi-firmware.h:121-125).
 *   hdmi    Q6: HDMI_HOTPLUG (+0x1c8 of the hdmi block) and HD_VID_CTL of
 *           HDMI0 (hd + 0x44) and HDMI1 (hd + 0x48), read only when the HSM
 *           clock (13) answered state on and a measured rate other than 0.
 *           Otherwise the line says `skipped (hsm clock off)`: a read of an
 *           HDMI block with its clock off may hang the bus.
 *
 * The mailbox is rpi5_mbox's "clear" way (a clean before the doorbell, an
 * invalidate after the reply; CONFIG_CACHE_MANAGEMENT=y), a copy of
 * rpi_mbox.c's transact() that keeps the response code word the board layer
 * keeps to itself.
 *
 * The end line is `rpi5_dispregs: done (...)` or `rpi5_dispregs: FAIL
 * (<reason>)`; then the image stops feeding the PM watchdog (armed by the board
 * layer in its first PRE_KERNEL_1 hook, before main), which resets the SoC
 * within 10 s, and tryboot brings Linux back. verdicts.sh reads the log.
 */
#include <zephyr/cache.h>
#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>

#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "fw_clk.h"
#include "hvs6_chan.h"
#include "pv_hdmi.h"
#include "pv_timing.h"
#include "scanout_alive.h"

#include "rpi_wdog.h"

/* The mailbox: the board layer's values (rpi_mbox.c:27-40), as rpi5_mbox. */
#define MBOX_BASE      0x107c013880ull
#define MBOX_SIZE      0x40u
#define MAIL0_RD       0x00
#define MAIL0_STA      0x18
#define MAIL1_WRT      0x20
#define MAIL1_STA      0x38
#define MBOX_FULL      BIT(31)
#define MBOX_EMPTY     BIT(30)
#define CHAN_PROPERTY  8u
#define STATUS_SUCCESS 0x80000000u
#define TAG_RESPONSE   BIT(31)
#define FULL_TRIES     100000  /* rpi_mbox.c:70-74, 1 us apart */
#define REPLY_TRIES    1000000 /* rpi_mbox.c:77-86, 1 us apart */

/* GET tags of the display count and settings: driver/docs/rpi5-display.md:203. */
#define TAG_GET_NUM_DISPLAYS      0x00040013u
#define TAG_GET_DISPLAY_SETTINGS  0x00040014u
#define SETTINGS_BYTES            64u

#define HVS_BASE 0x107c580000ull /* rpi:bcm2712.dtsi:612-617, as rpi5_scanout16 */
#define HVS_SIZE 0x1a000u
/* HDMI1: driver/docs/rpi5-display.md:68 (bcm2712.dtsi:399-426), the same
 * register layout as HDMI0's hdmi block; its HD_VID_CTL is hd + 0x48
 * (driver/docs/rpi5-display.md:302, vc4_hdmi_regs.h:531-538). */
#define HDMI1_HDMI_BASE 0x107c706400ull
#define HDMI1_HD_VID_CTL 0x0048u
#define HDMI_HOTPLUG     0x1c8u /* hdmi_hpd.h:53; bit 0 connected */
#define ALL_ONES         0xffffffffu

#define BUF_WORDS 64u
static uint32_t buf[BUF_WORDS] __aligned(64);

static mm_reg_t mbox;
static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

struct reply {
	uint32_t status;   /* buffer response code word */
	uint32_t answered; /* tag response length and flag word */
	uint32_t w[16];    /* value words */
	int rc;
	const char *stage;
};

/* Map a block, print nothing; 0 when it did not map. */
static mm_reg_t map(uint64_t pa, size_t size)
{
	mm_reg_t va = 0;

	device_map(&va, (uintptr_t)pa, size, K_MEM_CACHE_NONE);
	return va;
}

static uint32_t rd(mm_reg_t base, uint32_t size, uint32_t off)
{
	return base != 0 && off < size ? sys_read32(base + off) : ALL_ONES;
}

/* One property request with one tag of `nbytes` value bytes; value word 0 is
 * `v0`. The transaction of rpi_mbox.c's transact() with the cache calls on. */
static void xact(uint32_t tag, uint32_t nbytes, uint32_t v0, struct reply *r)
{
	uint32_t nval = nbytes / 4u;
	uint64_t phys64 = k_mem_phys_addr(buf);
	uint32_t phys = (uint32_t)phys64;

	memset(r, 0, sizeof(*r));
	r->stage = "not sent";
	r->rc = -ENODEV;
	if (!mbox) {
		r->stage = "no mapping";
		return;
	}
	if (phys64 != phys) {
		r->stage = "buffer above 4 GiB";
		r->rc = -ERANGE;
		return;
	}
	memset(buf, 0, sizeof(buf));
	buf[0] = (6u + nval) * 4u;
	buf[2] = tag;
	buf[3] = nbytes;
	buf[5] = v0;
	sys_cache_data_flush_range(buf, sizeof(buf));
	for (int t = 0; sys_read32(mbox + MAIL1_STA) & MBOX_FULL; t++) {
		if (t > FULL_TRIES) {
			r->stage = "MAIL1 stayed full";
			r->rc = -ETIMEDOUT;
			return;
		}
		k_busy_wait(1);
	}
	sys_write32(phys | CHAN_PROPERTY, mbox + MAIL1_WRT);
	for (int t = 0;; t++) {
		if (!(sys_read32(mbox + MAIL0_STA) & MBOX_EMPTY)) {
			uint32_t msg = sys_read32(mbox + MAIL0_RD);

			if ((msg & 0xfu) == CHAN_PROPERTY)
				break;
		}
		if (t > REPLY_TRIES) {
			r->stage = "no reply";
			r->rc = -ETIMEDOUT;
			return;
		}
		k_busy_wait(1);
	}
	sys_cache_data_invd_range(buf, sizeof(buf));
	r->status = buf[1];
	r->answered = buf[4];
	for (uint32_t i = 0; i < nval && i < ARRAY_SIZE(r->w); i++)
		r->w[i] = buf[5u + i];
	r->stage = "replied";
	r->rc = (r->status == STATUS_SUCCESS && buf[2] == tag && (r->answered & TAG_RESPONSE)) ? 0 : -EIO;
}

/* A clock tag, value words (id, value): the line carries the clock and the
 * value word. Returns rc; *val is the value word (0 when not answered). */
static int clock_tag(const char *tname, uint32_t tag, uint32_t id, uint32_t *val)
{
	struct reply r;

	xact(tag, 8u, id, &r);
	*val = r.rc == 0 ? r.w[1] : 0;
	printk("rpi5_dispregs: mbox %s tag 0x%08x clock %u status 0x%08x answered 0x%08x value 0x%08x "
	       "rc %d stage %s\n",
	       tname, tag, id, r.status, r.answered, r.w[1], r.rc, r.stage);
	return r.rc;
}

static const char *stepping(uint32_t version)
{
	return version == ALL_ONES                                       ? "unreadable"
	       : (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_D0   ? "D0"
	       : (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_C0   ? "C0"
								       : "unknown";
}

/* Q3, the HVS half. Returns the number of channels whose CTRL0 was read. */
static void read_hvs(mm_reg_t hvs)
{
	uint32_t version = rd(hvs, HVS_SIZE, SA_HVS_VERSION);
	int d0 = sa_is_d0(version);

	printk("rpi5_dispregs: hvs %s version 0x%08x stepping %s\n", hvs ? "mapped" : "NOT mapped",
	       version, stepping(version));
	for (unsigned int ch = 0; ch < 2; ch++) {
		uint32_t c = rd(hvs, HVS_SIZE, sa_reg_ctrl0(d0, ch));
		uint32_t s = rd(hvs, HVS_SIZE, sa_reg_status(d0, ch));

		printk("rpi5_dispregs: hvs ch%u ctrl0 0x%08x enb %u fwidth %u lines %u status 0x%08x mode %u "
		       "frcnt %u\n",
		       ch, c, (c & YD5_HVS6_CTRL0_ENB) != 0,
		       (c >> YD5_HVS6_CTRL0_FWIDTH_SHIFT) & YD5_HVS6_CTRL0_FWIDTH_MASK,
		       c & YD5_HVS6_CTRL0_LINES_MASK, s, (s >> SA_STATUS_MODE_SHIFT) & SA_STATUS_MODE_MASK,
		       (s >> SA_STATUS_FRCNT_SHIFT) & SA_STATUS_FRCNT_MASK);
	}
}

/* Q3, the valve half. Returns 1 when the valve mapped. */
static int read_pv(unsigned int n, uint64_t pa, uint32_t size)
{
	mm_reg_t va = map(pa, size);
	uint32_t ctl = rd(va, size, PV_CONTROL), vctl = rd(va, size, PV_V_CONTROL);
	uint32_t ha = rd(va, size, PV_HORZA), hb = rd(va, size, PV_HORZB);
	uint32_t va_ = rd(va, size, PV_VERTA), vb = rd(va, size, PV_VERTB);
	struct pv_control c = pv_decode_control(ctl);
	struct pv_vcontrol vc = pv_decode_vcontrol(vctl);
	struct pv_timing t = pv_decode_timing(ha, hb, va_, vb);

	printk("rpi5_dispregs: pv%u %s\n", n, va ? "mapped" : "NOT mapped");
	printk("rpi5_dispregs: pv%u regs control 0x%08x vcontrol 0x%08x horza 0x%08x horzb 0x%08x verta "
	       "0x%08x vertb 0x%08x\n",
	       n, ctl, vctl, ha, hb, va_, vb);
	printk("rpi5_dispregs: pv%u decode en %u viden %u clk_select %u pixel_rep %u format %u hactive %u "
	       "hfp %u hsync %u hbp %u vactive %u vfp %u vsync %u vbp %u vic %u\n",
	       n, c.enabled, vc.viden, c.clk_select, c.pixel_rep, c.format, t.hactive, t.hfp, t.hsync,
	       t.hbp, t.vactive, t.vfp, t.vsync, t.vbp, pvt_match_vic(ctl, vctl, ha, hb, va_, vb));
	return va != 0;
}

static void read_hdmi_port(unsigned int n, mm_reg_t hdmi, mm_reg_t hd, uint32_t vid_off)
{
	uint32_t hp = rd(hdmi, 0x300u, HDMI_HOTPLUG);
	uint32_t vid = rd(hd, HDMI0_HD_SIZE, vid_off);

	printk("rpi5_dispregs: hdmi%u hotplug 0x%08x connected %u hd_vid_ctl 0x%08x enable %u\n", n, hp,
	       (hp & 1u) != 0, vid, hd_vid_ctl_enabled(vid));
}

int main(void)
{
	static const struct {
		uint32_t id;
		const char *name;
	} clks[] = {
		{ YD5_CLK_CORE, "core" },
		{ YD5_CLK_M2MC, "hsm" },
		{ YD5_CLK_PIXEL_BVB, "bvb" },
		{ YD5_CLK_DISP, "disp" },
	};
	unsigned int n_pv = 0, n_clk = 0, n_tags = 0;
	bool hsm_on = false;
	uint32_t hsm_measured = 0;
	struct reply r;

	printk("rpi5_dispregs: build %s\n", RPI5_DISPREGS_BUILD);
	printk("rpi5_dispregs: cache management %s\n", IS_ENABLED(CONFIG_CACHE_MANAGEMENT) ? "on" : "off");
	printk("rpi5_dispregs: read-only: no display, HVS, pixel valve, HDMI or PHY register is written\n");

	mbox = map(MBOX_BASE, MBOX_SIZE);
	printk("rpi5_dispregs: mailbox %s\n", mbox ? "mapped" : "NOT mapped");
	mm_reg_t hvs = map(HVS_BASE, HVS_SIZE);

	/* Q3 */
	read_hvs(hvs);
	n_pv += read_pv(0, PV0_BASE, PV0_SIZE);
	n_pv += read_pv(1, PV1_BASE, PV1_SIZE);
	printk("rpi5_dispregs: pv mux/stat not read (no offset of them in the repo)\n");

	/* Q5: four tags per clock. */
	for (unsigned int i = 0; i < ARRAY_SIZE(clks); i++) {
		uint32_t state, rate, measured, max;
		int rs, rr, rm, rx;

		printk("rpi5_dispregs: clock %u %s\n", clks[i].id, clks[i].name);
		rs = clock_tag("state", YD5_TAG_GET_CLOCK_STATE, clks[i].id, &state);
		rr = clock_tag("rate", YD5_TAG_GET_CLOCK_RATE, clks[i].id, &rate);
		rm = clock_tag("measured", YD5_TAG_GET_CLOCK_RATE_MEASURED, clks[i].id, &measured);
		rx = clock_tag("max", YD5_TAG_GET_MAX_CLOCK_RATE, clks[i].id, &max);
		n_tags += (rs == 0) + (rr == 0) + (rm == 0) + (rx == 0);
		n_clk += (rs == 0 && rr == 0 && rm == 0 && rx == 0);
		if (clks[i].id == YD5_CLK_M2MC)
			hsm_on = rs == 0 && rm == 0 && (state & YD5_FWCLK_STATE_ON) && measured != 0;
		if (clks[i].id == YD5_CLK_M2MC)
			hsm_measured = measured;
	}

	/* Q6: the display count and settings, then the HDMI words. */
	xact(TAG_GET_NUM_DISPLAYS, 4u, 0, &r);
	printk("rpi5_dispregs: mbox num_displays tag 0x%08x status 0x%08x answered 0x%08x value 0x%08x rc "
	       "%d stage %s\n",
	       TAG_GET_NUM_DISPLAYS, r.status, r.answered, r.w[0], r.rc, r.stage);
	xact(TAG_GET_DISPLAY_SETTINGS, SETTINGS_BYTES, 0, &r);
	printk("rpi5_dispregs: mbox display_settings tag 0x%08x status 0x%08x answered 0x%08x value "
	       "0x%08x rc %d stage %s\n",
	       TAG_GET_DISPLAY_SETTINGS, r.status, r.answered, r.w[0], r.rc, r.stage);
	printk("rpi5_dispregs: display_settings words");
	for (unsigned int i = 0; i < SETTINGS_BYTES / 4u; i++)
		printk(" 0x%08x", r.w[i]);
	printk("\n");

	if (hsm_on) {
		mm_reg_t hd = map(HDMI0_HD_BASE, HDMI0_HD_SIZE);
		mm_reg_t h0 = map(HDMI0_HDMI_BASE, HDMI0_HDMI_SIZE);
		mm_reg_t h1 = map(HDMI1_HDMI_BASE, HDMI0_HDMI_SIZE);

		printk("rpi5_dispregs: hdmi reading (hsm clock on, measured %u Hz)\n", hsm_measured);
		read_hdmi_port(0, h0, hd, HD_VID_CTL);
		read_hdmi_port(1, h1, hd, HDMI1_HD_VID_CTL);
	} else {
		printk("rpi5_dispregs: hdmi skipped (hsm clock off)\n");
	}

	if (!mbox)
		printk("rpi5_dispregs: FAIL (no mailbox mapping)\n");
	else if (!hvs)
		printk("rpi5_dispregs: FAIL (the HVS did not map)\n");
	else if (n_pv == 0)
		printk("rpi5_dispregs: FAIL (no pixel valve mapped)\n");
	else
		printk("rpi5_dispregs: done (pv %u of 2, clock tags %u of 16, hdmi %s)\n", n_pv, n_tags,
		       hsm_on ? "read" : "skipped");
	(void)n_clk;

	finished = true;
	printk("rpi5_dispregs: feeding stopped, reset expected (the PM watchdog resets the SoC within "
	       "10 s, and tryboot returns the board to Linux)\n");
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
