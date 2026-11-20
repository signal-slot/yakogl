/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/route3.h: the route-3 bring-up plan of the Pi 5
 * screen. Checks the order of the stages (clocks before HDMI, the list
 * before the channel points at it, the HVS channel before the pixel valve as
 * Linux does, the pixel valve before HDMI's post-enable), the step counts
 * for 640x480, 720p and 1080p, the clock buffers against a firmware model,
 * the error cases, and a replay of plan and teardown against a register
 * model with the hardware's reactions to the polled bits.
 *
 * Build: cc -std=c99 -Wall -Wextra -Werror -I../rpi5_scanout route3_mock.c
 * (tools/v3d/check71.d/route3.sh). Prints one line per check, then
 * "route3_mock: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>
#include <string.h>

#include "route3.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

/* Expected counts: clk-query 1, dlist 10, hvs-chan 7, clk-hsm 1, clk-bvb 1,
 * phy-init 1, hdmi-configure 12, pv-reset 2, pv-config 9, pv-enable 1,
 * hdmi-pre-enable 11, pv-viden 1, hdmi-post-enable 7 writes + 3 waits. */
#define PLAN_STEPS (1 + 10 + 7 + 1 + 1 + 1 + 12 + 2 + 9 + 1 + 11 + 1 + 7 + 3)
/* pv-viden-off 1, wait-viden 1, wait-20ms 1, hdmi-disable 3, wait-1ms 1, pv-reset 2,
 * hvs-stop 2, wait-hvs-disabled 1, hdmi-powerdown 1, phy-disable 1 */
#define TEARDOWN_STEPS (1 + 1 + 1 + 3 + 1 + 2 + 2 + 1 + 1 + 1)

#define FB_ADDR 0x00c0000000ull
#define DL_OFF 48u

/* ---- searching a plan ------------------------------------------------------ */

static int is_write(const struct yd5_r3_step *s, uint8_t block, uint32_t off)
{
	return s->kind == YD5_R3_WRITE && s->w.block == block && s->w.offset == off;
}

/* First / last index of a step with the name, or -1. */
static int first_named(const struct yd5_route3 *p, const char *name)
{
	unsigned int i;

	for (i = 0; i < p->n; i++)
		if (strcmp(p->s[i].name, name) == 0)
			return (int)i;
	return -1;
}

static int last_named(const struct yd5_route3 *p, const char *name)
{
	int i;

	for (i = (int)p->n - 1; i >= 0; i--)
		if (strcmp(p->s[i].name, name) == 0)
			return i;
	return -1;
}

static int count_named(const struct yd5_route3 *p, const char *name)
{
	unsigned int i;
	int n = 0;

	for (i = 0; i < p->n; i++)
		n += strcmp(p->s[i].name, name) == 0;
	return n;
}

static int count_kind(const struct yd5_route3 *p, uint8_t kind)
{
	unsigned int i;
	int n = 0;

	for (i = 0; i < p->n; i++)
		n += p->s[i].kind == kind;
	return n;
}

/* First write that sets (value & mask & bits) == bits at {block, off}. */
static int first_setting(const struct yd5_route3 *p, uint8_t block, uint32_t off, uint32_t bits)
{
	unsigned int i;

	for (i = 0; i < p->n; i++)
		if (is_write(&p->s[i], block, off) && (p->s[i].w.mask & bits) == bits &&
		    (p->s[i].w.value & bits) == bits)
			return (int)i;
	return -1;
}

static int first_kind(const struct yd5_route3 *p, uint8_t kind, int after)
{
	unsigned int i;

	for (i = (unsigned int)(after + 1); i < p->n; i++)
		if (p->s[i].kind == kind)
			return (int)i;
	return -1;
}

static int first_hdmi_write(const struct yd5_route3 *p)
{
	unsigned int i;

	for (i = 0; i < p->n; i++)
		if (p->s[i].kind == YD5_R3_WRITE && p->s[i].w.block >= YD5_R3_BLK_HDMI0)
			return (int)i;
	return -1;
}

/* Any write in the plan that clears HD VID_CTL ENABLE. */
static int clears_vid_enable(const struct yd5_route3 *p)
{
	unsigned int i;

	for (i = 0; i < p->n; i++)
		if (is_write(&p->s[i], YD5_R3_BLK_HDMI(YD5_HDMI_VID_CTL.region),
			     YD5_HDMI_VID_CTL.offset) &&
		    (p->s[i].w.mask & YD5_VID_CTL_ENABLE) && !(p->s[i].w.value & YD5_VID_CTL_ENABLE))
			return 1;
	return 0;
}

/* ---- register and firmware model ---------------------------------------- */

#define HVS_WORDS (0x8000u / 4u)
#define PV_WORDS (0x100u / 4u)
#define HDMI_WORDS (0x400u / 4u)

struct model {
	uint32_t hvs[HVS_WORDS];
	uint32_t pv[PV_WORDS];
	uint32_t hdmi[YD5_HDMI_R_COUNT][HDMI_WORDS];
	uint32_t hsm_hz, bvb_hz;
	int placeholders;
	int clock_errors;
	enum yd5_hvs6_variant v;
};

static uint32_t *reg(struct model *m, uint8_t block, uint32_t off)
{
	if (off & 3u)
		return NULL;
	if (block == YD5_R3_BLK_HVS && off / 4u < HVS_WORDS)
		return &m->hvs[off / 4u];
	if (block == YD5_R3_BLK_PV && off / 4u < PV_WORDS)
		return &m->pv[off / 4u];
	if (block >= YD5_R3_BLK_HDMI0 && block < YD5_R3_BLK_HDMI0 + YD5_HDMI_R_COUNT &&
	    off / 4u < HDMI_WORDS)
		return &m->hdmi[block - YD5_R3_BLK_HDMI0][off / 4u];
	return NULL;
}

/* The firmware: answer a buffer the way the property channel does. */
static void firmware(uint32_t *buf, struct yd5_r3_step *s, struct model *m)
{
	uint32_t i;

	buf[1] = YD5_FWCLK_RESP_OK;
	if (s->c.op == YD5_R3_CLK_SET_RATE) {
		buf[4] = YD5_FWCLK_TAG_RESP | 8u;
		if (buf[5] == YD5_CLK_M2MC)
			m->hsm_hz = buf[6];
		if (buf[5] == YD5_CLK_PIXEL_BVB)
			m->bvb_hz = buf[6];
		return;
	}
	for (i = 2; i + 5u <= s->c.words - 1u; i += YD5_FWCLK_TAG_WORDS) {
		buf[i + 2] = YD5_FWCLK_TAG_RESP | 8u;
		buf[i + 4] = buf[i] == YD5_TAG_GET_CLOCK_STATE ? YD5_FWCLK_STATE_ON : 500000000u;
	}
}

/* The hardware's reactions to writes: RECENTER sets RECENTER_DONE, CTRL0 ENB
 * runs the channel and its absence disables it. */
static void react(struct model *m, const struct yd5_r3_step *s, uint32_t *r)
{
	if (s->w.block == YD5_R3_BLK_HDMI(YD5_HDMI_FIFO_CTL.region) &&
	    s->w.offset == YD5_HDMI_FIFO_CTL.offset && (*r & YD5_FIFO_CTL_RECENTER))
		*r |= YD5_FIFO_CTL_RECENTER_DONE;
	if (s->w.block == YD5_R3_BLK_HVS && s->w.offset == yd5_hvs6_ctrl0_reg(m->v, 0)) {
		uint32_t *st = reg(m, YD5_R3_BLK_HVS, yd5_hvs6_status_reg(m->v, 0));
		uint32_t mode = (*r & YD5_HVS6_CTRL0_ENB) ? YD5_HVS6_STATUS_MODE_RUN
							   : YD5_HVS6_STATUS_MODE_DISABLED;

		*st = (*st & ~(YD5_HVS6_STATUS_MODE_MASK << YD5_HVS6_STATUS_MODE_SHIFT)) |
		      (mode << YD5_HVS6_STATUS_MODE_SHIFT);
	}
}

/* Replay a plan; returns the index of the step that failed, or -1. */
static int replay(struct model *m, struct yd5_route3 *p)
{
	unsigned int i;

	for (i = 0; i < p->n; i++) {
		struct yd5_r3_step *s = &p->s[i];
		uint32_t *r, buf[YD5_R3_MBOX_WORDS], rate;
		struct yd5_fwclk_info info[YD5_R3_CLK_QUERY_IDS];

		switch (s->kind) {
		case YD5_R3_WRITE:
			r = reg(m, s->w.block, s->w.offset);
			if (r == NULL)
				return (int)i;
			*r = (*r & ~s->w.mask) | (s->w.value & s->w.mask);
			react(m, s, r);
			break;
		case YD5_R3_DELAY:
			if (s->d.us == 0)
				return (int)i;
			if (s->d.poll) {
				r = reg(m, s->d.block, s->d.offset);
				if (r == NULL || (*r & s->d.mask) != s->d.want)
					return (int)i;
			}
			break;
		case YD5_R3_CLOCK:
			memcpy(buf, s->c.buf, sizeof(buf));
			firmware(buf, s, m);
			if (s->c.op == YD5_R3_CLK_QUERY) {
				if (yd5_fwclk_parse_query(buf, s->c.nids, info) != 0 ||
				    !info[0].on || !info[1].on)
					m->clock_errors++;
			} else if (yd5_fwclk_parse_set_rate(buf, s->c.id, &rate) != 0 ||
				   rate != s->c.hz) {
				m->clock_errors++;
			}
			break;
		case YD5_R3_PLACEHOLDER:
			m->placeholders++;
			break;
		default:
			return (int)i;
		}
	}
	return -1;
}

/* ---- checks ------------------------------------------------------------------ */

static void check_mode(uint8_t vic, const char *tag, enum yd5_hvs6_variant v, uint32_t hsm,
		       uint32_t bvb)
{
	static struct yd5_route3 p, q, td;
	static struct model m;
	struct yd5_route3_mode mode;
	const struct pvt_mode *pm = pvt_find_vic(vic);
	struct pvt_regs pv;
	struct hvs6_plane pl;
	uint32_t pitch = pm->hactive * 4u + 64u, ctrl0, *vid;
	char name[128];
	int r, i_dl, i_lptrs, i_enb, i_pv_en, i_viden, i_vid, i_phy, i_fifo, i, j;
	unsigned int k;

	yd5_route3_mode_default(&mode, vic);
	mode.hvs = v;
	mode.dlist_offset_words = DL_OFF;
	mode.fifo_level = 0x20;
	pvt_regs_from_mode(pm, &pv);

	r = yd5_route3_plan(&mode, FB_ADDR, pitch, &p);
	snprintf(name, sizeof(name), "%s: plan returns 0 with %d steps (got %u)", tag, PLAN_STEPS,
		 p.n);
	check(r == 0 && p.n == PLAN_STEPS, name);

	snprintf(name, sizeof(name), "%s: kinds: 3 clock, 1 placeholder, 3 waits, rest writes",
		 tag);
	check(count_kind(&p, YD5_R3_CLOCK) == 3 && count_kind(&p, YD5_R3_PLACEHOLDER) == 1 &&
	      count_kind(&p, YD5_R3_DELAY) == 3 && count_kind(&p, YD5_R3_WRITE) == PLAN_STEPS - 7,
	      name);

	snprintf(name, sizeof(name), "%s: stage step counts", tag);
	check(count_named(&p, "dlist") == 10 && count_named(&p, "hvs-chan") == 7 &&
	      count_named(&p, "hdmi-configure") == 12 && count_named(&p, "pv-reset") == 2 &&
	      count_named(&p, "pv-config") == 9 && count_named(&p, "hdmi-pre-enable") == 11 &&
	      count_named(&p, "hdmi-post-enable") == 7,
	      name);

	/* clocks before HDMI */
	i_phy = first_named(&p, "phy-init");
	snprintf(name, sizeof(name), "%s: clocks (query, hsm, bvb) before phy-init before any "
		 "HDMI write", tag);
	check(first_named(&p, "clk-query") == 0 && first_named(&p, "clk-hsm") < i_phy &&
	      first_named(&p, "clk-bvb") < i_phy && i_phy >= 0 && i_phy < first_hdmi_write(&p) &&
	      first_kind(&p, YD5_R3_CLOCK, first_named(&p, "clk-bvb")) < 0,
	      name);
	snprintf(name, sizeof(name), "%s: the PHY step is the placeholder 'phy-init'", tag);
	check(i_phy >= 0 && p.s[i_phy].kind == YD5_R3_PLACEHOLDER, name);

	/* the list before the channel points at it */
	i_dl = last_named(&p, "dlist");
	i_lptrs = -1;
	for (k = 0; k < p.n; k++)
		if (is_write(&p.s[k], YD5_R3_BLK_HVS, yd5_hvs6_lptrs_reg(v, 0)))
			i_lptrs = (int)k;
	i_enb = first_setting(&p, YD5_R3_BLK_HVS, yd5_hvs6_ctrl0_reg(v, 0), YD5_HVS6_CTRL0_ENB |
			      ((pm->hactive - 1u) << YD5_HVS6_CTRL0_FWIDTH_SHIFT));
	snprintf(name, sizeof(name), "%s: display list written before LPTRS, LPTRS before ENB",
		 tag);
	check(i_dl >= 0 && i_lptrs > i_dl && p.s[i_lptrs].w.value == DL_OFF && i_enb > i_lptrs,
	      name);

	/* HVS channel, pixel valve, HDMI: Linux's order */
	i_pv_en = first_setting(&p, YD5_R3_BLK_PV, PV_CONTROL, PV_CONTROL_EN);
	i_viden = first_setting(&p, YD5_R3_BLK_PV, PV_V_CONTROL, PV_VCONTROL_VIDEN);
	i_vid = first_setting(&p, YD5_R3_BLK_HDMI(YD5_HDMI_VID_CTL.region),
			      YD5_HDMI_VID_CTL.offset, YD5_VID_CTL_ENABLE);
	i_fifo = first_setting(&p, YD5_R3_BLK_HDMI(YD5_HDMI_FIFO_CTL.region),
			       YD5_HDMI_FIFO_CTL.offset, YD5_FIFO_CTL_MASTER_SLAVE_N);
	snprintf(name, sizeof(name), "%s: HVS ENB < HDMI configure < PV config < PV EN", tag);
	check(i_enb >= 0 && i_enb < first_named(&p, "hdmi-configure") &&
	      last_named(&p, "hdmi-configure") < first_named(&p, "pv-reset") &&
	      last_named(&p, "pv-config") < i_pv_en,
	      name);
	snprintf(name, sizeof(name), "%s: PV EN < HDMI FIFO master < VIDEN < VID_CTL ENABLE", tag);
	check(i_pv_en >= 0 && i_pv_en < i_fifo && i_fifo < i_viden && i_viden < i_vid, name);
	snprintf(name, sizeof(name), "%s: the pixel valve is configured before any EN or VIDEN",
		 tag);
	check(first_named(&p, "pv-reset") < i_pv_en && first_named(&p, "pv-reset") < i_viden &&
	      p.s[first_named(&p, "pv-reset")].w.mask == PV_CONTROL_EN &&
	      p.s[first_named(&p, "pv-reset")].w.value == 0,
	      name);

	/* the waits */
	i = first_named(&p, "wait-hdmi-active");
	snprintf(name, sizeof(name), "%s: HDMI_ACTIVE poll right after the DVI-mode write", tag);
	check(i > 0 && p.s[i].d.poll && p.s[i].d.mask == YD5_SCHED_HDMI_ACTIVE &&
	      p.s[i].d.want == 0 && p.s[i].d.us == 1000000u &&
	      is_write(&p.s[i - 1], YD5_R3_BLK_HDMI(YD5_HDMI_SCHEDULER_CONTROL.region),
		       YD5_HDMI_SCHEDULER_CONTROL.offset) &&
	      p.s[i - 1].w.mask == YD5_SCHED_MODE_HDMI,
	      name);
	i = first_named(&p, "wait-1ms");
	j = last_named(&p, "wait-recenter");
	snprintf(name, sizeof(name), "%s: 1 ms between the RECENTER pairs, RECENTER_DONE last",
		 tag);
	check(i > 0 && !p.s[i].d.poll && p.s[i].d.us == 1000u && p.s[i - 1].w.value ==
	      YD5_FIFO_CTL_RECENTER && p.s[i + 1].w.value == 0 && j == (int)p.n - 1 &&
	      p.s[j].d.poll && p.s[j].d.want == YD5_FIFO_CTL_RECENTER_DONE,
	      name);

	/* clocks */
	i = first_named(&p, "clk-hsm");
	j = first_named(&p, "clk-bvb");
	snprintf(name, sizeof(name), "%s: HSM %u Hz, BVB %u Hz, SET_CLOCK_RATE buffers", tag, hsm,
		 bvb);
	check(p.s[i].c.op == YD5_R3_CLK_SET_RATE && p.s[i].c.id == YD5_CLK_M2MC &&
	      p.s[i].c.hz == hsm && p.s[i].c.buf[2] == YD5_TAG_SET_CLOCK_RATE &&
	      p.s[i].c.buf[5] == YD5_CLK_M2MC && p.s[i].c.buf[6] == hsm &&
	      p.s[i].c.words == YD5_FWCLK_SET_WORDS && p.s[j].c.id == YD5_CLK_PIXEL_BVB &&
	      p.s[j].c.hz == bvb && p.s[j].c.buf[6] == bvb,
	      name);
	snprintf(name, sizeof(name), "%s: query buffer for core, disp, m2mc, pixel_bvb", tag);
	check(p.s[0].c.op == YD5_R3_CLK_QUERY && p.s[0].c.nids == 4 &&
	      p.s[0].c.words == YD5_FWCLK_QUERY_WORDS(4) && p.s[0].c.ids[0] == YD5_CLK_CORE &&
	      p.s[0].c.ids[1] == YD5_CLK_DISP && p.s[0].c.buf[0] == YD5_FWCLK_QUERY_WORDS(4) * 4u,
	      name);

	snprintf(name, sizeof(name), "%s: no write clears VID_CTL ENABLE", tag);
	check(!clears_vid_enable(&p), name);

	/* determinism */
	r = yd5_route3_plan(&mode, FB_ADDR, pitch, &q);
	j = r == 0 && q.n == p.n;
	for (k = 0; j && k < p.n; k++)
		j = q.s[k].kind == p.s[k].kind && strcmp(q.s[k].name, p.s[k].name) == 0 &&
		    memcmp(&q.s[k].w, &p.s[k].w, sizeof(p.s[k].w)) == 0 &&
		    q.s[k].d.us == p.s[k].d.us && q.s[k].c.words == p.s[k].c.words &&
		    memcmp(q.s[k].c.buf, p.s[k].c.buf, sizeof(q.s[k].c.buf)) == 0;
	snprintf(name, sizeof(name), "%s: a second plan is the same", tag);
	check(j, name);

	/* replay */
	memset(&m, 0, sizeof(m));
	m.v = v;
	m.hdmi[YD5_HDMI_VID_CTL.region][YD5_HDMI_VID_CTL.offset / 4u] = YD5_VID_CTL_BLANKPIX;
	m.pv[YD5_R3_PV_VSYNCD_EVEN / 4] = 0xdeadbeefu;
	r = replay(&m, &p);
	snprintf(name, sizeof(name), "%s: replay runs every step, polls met, 1 placeholder", tag);
	check(r == -1 && m.placeholders == 1, name);
	snprintf(name, sizeof(name), "%s: firmware model accepts the clock buffers", tag);
	check(m.clock_errors == 0 && m.hsm_hz == hsm && m.bvb_hz == bvb, name);

	r = hvs6_dlist_first_plane(m.hvs + HVS6_DLIST_OFFSET / 4u, HVS_WORDS - HVS6_DLIST_OFFSET / 4u,
				   m.hvs[yd5_hvs6_lptrs_reg(v, 0) / 4u], &pl);
	snprintf(name, sizeof(name), "%s: the list LPTRS names decodes to the framebuffer", tag);
	check(r == HVS6_OK && pl.addr == FB_ADDR && pl.pitch == pitch && pl.width == pm->hactive &&
	      pl.height == pm->vactive && pl.unity && pl.rgb32,
	      name);
	ctrl0 = m.hvs[yd5_hvs6_ctrl0_reg(v, 0) / 4u];
	snprintf(name, sizeof(name), "%s: HVS CTRL0 = ENB | FWIDTH | LINES, STATUS run", tag);
	check(ctrl0 == (YD5_HVS6_CTRL0_ENB | ((pm->hactive - 1u) << 16) | (pm->vactive - 1u)) &&
	      ((m.hvs[yd5_hvs6_status_reg(v, 0) / 4u] >> YD5_HVS6_STATUS_MODE_SHIFT) & 3u) ==
		      YD5_HVS6_STATUS_MODE_RUN,
	      name);
	snprintf(name, sizeof(name), "%s: PV timing words match pv_timing.h, VIC found", tag);
	check(m.pv[PV_HORZA / 4] == pv.horza && m.pv[PV_HORZB / 4] == pv.horzb &&
	      m.pv[PV_VERTA / 4] == pv.verta && m.pv[PV_VERTB / 4] == pv.vertb &&
	      pvt_match_vic(m.pv[PV_CONTROL / 4], m.pv[PV_V_CONTROL / 4], m.pv[PV_HORZA / 4],
			    m.pv[PV_HORZB / 4], m.pv[PV_VERTA / 4], m.pv[PV_VERTB / 4]) == vic,
	      name);
	snprintf(name, sizeof(name), "%s: PV running: EN, VIDEN, CONTINUOUS, ODD_TIMING, CLK_SELECT 0, "
		 "level, VSYNCD_EVEN 0",
		 tag);
	check(pv_running(m.pv[PV_CONTROL / 4], m.pv[PV_V_CONTROL / 4]) &&
	      (m.pv[PV_V_CONTROL / 4] & PV_VCONTROL_CONTINUOUS) &&
	      (m.pv[PV_V_CONTROL / 4] & YD5_R3_PV_VCONTROL_ODD_TIMING) &&
	      ((m.pv[PV_CONTROL / 4] >> PV_CONTROL_CLK_SELECT_SHIFT) & 3u) == 0u &&
	      ((m.pv[PV_CONTROL / 4] >> PV_CONTROL_FIFO_LEVEL_SHIFT) & 0x3fu) == 0x20u &&
	      m.pv[YD5_R3_PV_VSYNCD_EVEN / 4] == 0,
	      name);
	vid = &m.hdmi[YD5_HDMI_VID_CTL.region][YD5_HDMI_VID_CTL.offset / 4u];
	snprintf(name, sizeof(name), "%s: VID_CTL enabled, BLANKPIX released, sync polarity", tag);
	check((*vid & YD5_VID_CTL_ENABLE) && !(*vid & YD5_VID_CTL_BLANKPIX) &&
	      !!(*vid & YD5_VID_CTL_HSYNC_LOW) == !pm->hsync_positive &&
	      !!(*vid & YD5_VID_CTL_VSYNC_LOW) == !pm->vsync_positive,
	      name);

	/* teardown after the plan */
	r = yd5_route3_teardown(&mode, &td);
	snprintf(name, sizeof(name), "%s: teardown returns 0 with %d steps", tag, TEARDOWN_STEPS);
	check(r == 0 && td.n == TEARDOWN_STEPS, name);
	snprintf(name, sizeof(name), "%s: teardown order: VIDEN off, 20 ms, HDMI blank, PV reset, HVS "
		 "stop, powerdown, phy-disable", tag);
	check(first_named(&td, "pv-viden-off") == 0 && first_named(&td, "wait-viden") == 1 &&
	      first_named(&td, "wait-20ms") == 2 && td.s[2].d.us == 20000u &&
	      first_named(&td, "hdmi-disable") == 3 &&
	      last_named(&td, "hdmi-disable") < first_named(&td, "wait-1ms") &&
	      first_named(&td, "wait-1ms") < first_named(&td, "pv-reset") &&
	      last_named(&td, "pv-reset") < first_named(&td, "hvs-stop") &&
	      last_named(&td, "hvs-stop") < first_named(&td, "wait-hvs-disabled") &&
	      first_named(&td, "wait-hvs-disabled") < first_named(&td, "hdmi-powerdown") &&
	      first_named(&td, "phy-disable") == (int)td.n - 1 &&
	      td.s[td.n - 1].kind == YD5_R3_PLACEHOLDER,
	      name);
	snprintf(name, sizeof(name), "%s: teardown has no clock step, keeps VID_CTL ENABLE", tag);
	check(count_kind(&td, YD5_R3_CLOCK) == 0 && !clears_vid_enable(&td), name);
	r = replay(&m, &td);
	snprintf(name, sizeof(name), "%s: teardown replays: PV off, HVS disabled, ENABLE kept",
		 tag);
	check(r == -1 && m.placeholders == 2 && !(m.pv[PV_CONTROL / 4] & PV_CONTROL_EN) &&
	      !(m.pv[PV_V_CONTROL / 4] & PV_VCONTROL_VIDEN) &&
	      !(m.hvs[yd5_hvs6_ctrl0_reg(v, 0) / 4u] & YD5_HVS6_CTRL0_ENB) &&
	      ((m.hvs[yd5_hvs6_status_reg(v, 0) / 4u] >> YD5_HVS6_STATUS_MODE_SHIFT) & 3u) ==
		      YD5_HVS6_STATUS_MODE_DISABLED &&
	      (*vid & YD5_VID_CTL_ENABLE) && (*vid & YD5_VID_CTL_BLANKPIX),
	      name);
}

static void check_errors(void)
{
	static struct yd5_route3 p;
	struct yd5_route3_mode mode;
	int r;

	yd5_route3_mode_default(&mode, 1);
	check(mode.fifo_level == 46u && mode.dlist_offset_words == 32u && mode.hvs == YD5_HVS6_C0 &&
	      mode.format == HVS6W_XRGB8888,
	      "default mode: FIFO level 46, list at word 32, C0, XRGB8888");
	check(yd5_route3_plan(&mode, FB_ADDR, 2560, NULL) == YD5_R3_EINVAL, "error: NULL out");
	p.n = 5;
	check(yd5_route3_plan(NULL, FB_ADDR, 2560, &p) == YD5_R3_EINVAL && p.n == 0,
	      "error: NULL mode, out->n = 0");

	mode.vic = 2;
	p.n = 5;
	r = yd5_route3_plan(&mode, FB_ADDR, 2560, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: VIC 2 is not in the table");

	yd5_route3_mode_default(&mode, 1);
	r = yd5_route3_plan(&mode, 1ull << 40, 2560, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: framebuffer address over 40 bits");
	r = yd5_route3_plan(&mode, FB_ADDR, 640u * 4u - 4u, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: pitch under one row");
	r = yd5_route3_plan(&mode, FB_ADDR, 2562, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: pitch not a multiple of 4");

	mode.dlist_offset_words = 31;
	r = yd5_route3_plan(&mode, FB_ADDR, 2560, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: list in the bootloader's words 0..31");
	mode.dlist_offset_words = 0xfffu - HVS6W_LIST_WORDS + 2u;
	r = yd5_route3_plan(&mode, FB_ADDR, 2560, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: list past the 12-bit head field");
	mode.dlist_offset_words = 0xfffu - HVS6W_LIST_WORDS + 1u;
	r = yd5_route3_plan(&mode, FB_ADDR, 2560, &p);
	check(r == 0 && p.n == PLAN_STEPS, "list ending at word 0xfff is accepted");

	yd5_route3_mode_default(&mode, 1);
	mode.fifo_level = 0x40;
	r = yd5_route3_plan(&mode, FB_ADDR, 2560, &p);
	check(r == YD5_R3_EINVAL && p.n == 0, "error: FIFO level over 6 bits");

	yd5_route3_mode_default(&mode, 1);
	mode.format = HVS6W_RGB565;
	r = yd5_route3_plan(&mode, FB_ADDR, 1280, &p);
	check(r == 0 && p.n == PLAN_STEPS, "RGB565 with a 1280-byte pitch plans");

	check(yd5_route3_teardown(NULL, &p) == YD5_R3_EINVAL && p.n == 0 &&
	      yd5_route3_teardown(&mode, NULL) == YD5_R3_EINVAL,
	      "error: teardown with NULL mode or out");
}

int main(void)
{
	check_mode(1, "640x480 C0", YD5_HVS6_C0, 120000000u, 75000000u);
	check_mode(16, "1080p C0", YD5_HVS6_C0, 149985000u, 75000000u);
	check_mode(16, "1080p D0", YD5_HVS6_D0, 149985000u, 75000000u);
	check_mode(4, "720p D0", YD5_HVS6_D0, 120000000u, 75000000u);
	check_errors();
	printf("route3_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
