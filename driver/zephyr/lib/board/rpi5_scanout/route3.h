/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * route3.h: the route-3 bring-up of the Pi 5 (BCM2712) screen as one ordered
 * plan a probe replays: firmware clocks, the HVS6 display list and channel,
 * pixel valve 0 and HDMI0 in DVI mode (driver/docs/rpi5-display.md section
 * 5). Header-only, C99, no Zephyr, no MMIO. It puts together the reviewed
 * pieces and redefines none of their registers:
 *
 *   hvs6_dlist_write.h  the display list of one unscaled plane
 *   hvs6_chan.h         the HVS6 channel start and stop writes
 *   pv_timing.h         the pixel valve timing words of a CEA mode (with
 *                       pv_hdmi.h's register layout)
 *   hdmi_ctrl.h         the HDMI0 controller phases and its disable
 *   fw_clk.h            the firmware clock mailbox buffers and rate rules
 *
 * A plan is a list of steps of four kinds:
 *   YD5_R3_CLOCK        a property-mailbox buffer built by fw_clk.h; the
 *                       caller sends it and runs the matching parse function
 *                       (yd5_fwclk_parse_query / yd5_fwclk_parse_set_rate)
 *   YD5_R3_WRITE        {block, offset, value, mask}: the caller does
 *                       reg = (reg & ~mask) | (value & mask); mask 0xffffffff
 *                       is a plain write
 *   YD5_R3_DELAY        wait `us` microseconds; with `poll` set, read
 *                       {block, offset} until (reg & mask) == want, giving up
 *                       (the plan fails) after `us`
 *   YD5_R3_PLACEHOLDER  a named step this header cannot do (see "Open")
 *
 * Order (Linux's, "rpi:" = raspberrypi/linux rpi-6.12.y 5bd564aa under
 * drivers/gpu/drm/vc4/; facts and file:line only, no code copied; the lines
 * are those of driver/docs/rpi5-display.md, checked by the linux-cites item;
 * vc4_crtc.c, vc4_regs.h and the vc4_hdmi.c/vc4_hvs.c enable paths were
 * re-read at rpi-6.12.y ab437207 and the lines below are the same there):
 *
 *  1. clk-query    GET state/rate/measured of clocks core, disp, m2mc (HSM)
 *                  and pixel_bvb: the HVS clocks Linux holds on from bind
 *                  (rpi:vc4_hvs.c:2111-2135) must read on
 *  2. dlist        the plane's words into the list memory, from word
 *                  dlist_offset_words up (rpi:vc4_hvs.c:1151-1175)
 *  3. hvs-chan     the channel start of vc4_hvs_atomic_enable, which points
 *                  DISPx_LPTRS at the list (rpi:vc4_hvs.c:1218-1237): the
 *                  first thing vc4_crtc_atomic_enable does
 *                  (rpi:vc4_crtc.c:680-700)
 *  4. encoder pre_crtc_configure (rpi:vc4_hdmi.c:1481-1595):
 *     clk-hsm      SET_CLOCK_RATE m2mc = max(120 MHz, 1.01 x TMDS)
 *                  (rpi:vc4_hdmi.c:1523-1526)
 *     clk-bvb      SET_CLOCK_RATE pixel_bvb = 75/150/300 MHz
 *                  (rpi:vc4_hdmi.c:1546-1559); there is no pixel clock on
 *                  the BCM2712 (rpi:vc4_hdmi.c:3162,3168,3186)
 *     phy-init     PLACEHOLDER for vc6_hdmi_phy_init (rpi:vc4_hdmi.c:1566)
 *     hdmi-configure  yd5_hdmi_dvi_configure (rpi:vc4_hdmi.c:1570-1578)
 *  5. pv-reset, pv-config: vc4_crtc_config_pv: EN cleared then FIFO_CLR
 *                  (rpi:vc4_crtc.c:329-343), HORZA/HORZB (:387-397),
 *                  V_CONTROL (:442-446), VSYNCD_EVEN = 0 (:447),
 *                  VERTA/VERTB (:450-455), MUX_CFG
 *                  (:460-463), PIPE_INIT_CTRL (:465-469), CONTROL (:471-480)
 *     pv-enable    PV_CONTROL |= EN (rpi:vc4_crtc.c:688)
 *  6. hdmi-pre-enable  yd5_hdmi_dvi_pre_enable (rpi:vc4_hdmi.c:1597-1625)
 *  7. pv-viden     PV_V_CONTROL |= VIDEN (rpi:vc4_crtc.c:696-697)
 *  8. hdmi-post-enable  yd5_hdmi_dvi_post_enable (rpi:vc4_hdmi.c:1627-1700)
 *                  with its waits: HDMI_ACTIVE == 0 after the DVI-mode write
 *                  (<= 1 s, :1673-1681), 1 ms between the two RECENTER pairs
 *                  and RECENTER_DONE after them (<= 1 ms, :1439-1475)
 *
 * The HVS channel is started before the pixel valve, as Linux does: the
 * channel fills its output FIFO and waits; the pixel valve starts pulling
 * when EN and VIDEN are set.
 *
 * Teardown (vc4_crtc_disable, rpi:vc4_crtc.c:502-551: VIDEN off and wait
 * :516-519, 20 ms :537, the encoder's post_crtc_disable :539-540, the pixel
 * valve reset :542, the HVS channel stop :543 and rpi:vc4_hvs.c:1072-1097,
 * the encoder's post_crtc_powerdown :545-546 and rpi:vc4_hdmi.c:882-960):
 * VIDEN off and wait, 20 ms, yd5_hdmi_disable's post_crtc_disable writes
 * and 1 ms, pixel valve reset, channel stop and
 * STATUS.MODE == DISABLED, the powerdown write, then phy-disable. VID_CTL
 * ENABLE is never cleared (BCM2712 lock-up, rpi:vc4_hdmi.c:912-921).
 *
 * Open (named placeholders or values left to the board):
 *  - phy-init: vc6_hdmi_phy_init (rpi:vc4_hdmi.c:1566,
 *    rpi:vc4_hdmi_phy.c:954-1198) is NOT implemented. Its constants are GPL
 *    tables; the project takes them from the board's Linux register dump
 *    (hdmi_phy.h's capture-and-replay plan). A probe replays that capture at
 *    this step or stops here.
 *  - phy-disable: vc6_hdmi_phy_disable in the teardown, likewise not done.
 *  - Clock enables and releases: fw_clk.h has no SET_CLOCK_STATE builder, so
 *    the plan only sets rates (the firmware keeps the clocks of a running
 *    scanout on) and the teardown releases none.
 *  - HVS global init (CONTROL, PRI_MAP, CSC; rpi:vc4_hvs.c:1909-1960), the
 *    UPM allocation in PTR0 and the COB: left as the firmware set them.
 *  - HDMI power-up (DVP reset pulse, DVP_CTL = 0; rpi:vc4_hdmi.c:3239-3268):
 *    not in hdmi_ctrl.h; the plan assumes the firmware left HDMI0 powered.
 *  - PV_CONTROL FIFO_LEVEL: the mode carries it; the default is Linux's
 *    46 for pixel valve 0 of the BCM2712 (see YD5_R3_PV_FIFO_LEVEL_24BPP).
 *    The board's register dump confirms it; values above 6 bits (the
 *    PV5_CONTROL_FIFO_LEVEL_HIGH bits) are refused.
 *  - The pixel valve values (ODD_TIMING, MUX_CFG, PIPE_INIT_CTRL, CLK_SELECT
 *    0) are read from the Linux source and must match Linux's register dump.
 */
#ifndef YAKOGL_RPI5_SCANOUT_ROUTE3_H
#define YAKOGL_RPI5_SCANOUT_ROUTE3_H

#include <stddef.h>
#include <stdint.h>

#include "fw_clk.h"
#include "hdmi_ctrl.h"
#include "hvs6_chan.h"
#include "hvs6_dlist_write.h"
#include "pv_timing.h"

/* ---- pixel valve registers no included header carries -------------------- */
/* rpi:vc4_regs.h:172; set on the 6.x HVS with one pixel per clock
 * (rpi:vc4_crtc.c:444-445; BCM2712 pixel valve 0: rpi:vc4_crtc.c:1337) */
#define YD5_R3_PV_VCONTROL_ODD_TIMING (1u << 29)
/* PV_VSYNCD_EVEN, written 0 for a progressive mode (rpi:vc4_regs.h:182,
 * rpi:vc4_crtc.c:447) */
#define YD5_R3_PV_VSYNCD_EVEN      0x08u
/* PV_MUX_CFG, RGB_PIXEL_MUX_MODE bits 5:2, 8 = no swap (rpi:vc4_regs.h:228-231) */
#define YD5_R3_PV_MUX_CFG          0x34u
#define YD5_R3_PV_MUX_RGB_SHIFT    2u
#define YD5_R3_PV_MUX_RGB_NO_SWAP  8u
/* PV_PIPE_INIT_CTRL: WIDTH 11:8, IDLE 7:4, EN 0 (rpi:vc4_regs.h:233-236) */
#define YD5_R3_PV_PIPE_INIT_CTRL   0x94u
#define YD5_R3_PV_PIPE_INIT_VALUE  ((1u << 8) | (1u << 4) | 1u)
/* PV_CONTROL CLK_SELECT for HDMI0 on the BCM2712: 0. Linux writes the index
 * of the encoder in the pixel valve's encoder_types table
 * (rpi:vc4_crtc.c:478-479,1391), and BCM2712 pixel valve 0 lists HDMI0 at
 * index 0 (rpi:vc4_crtc.c:1339-1341); the BCM2711's 1 (DPI_SMI_HDMI,
 * rpi:vc4_regs.h:164) does not apply. */
#define YD5_R3_PV_CLK_SELECT_HDMI  0u
/* FIFO full level for 24-bit output: fifo_depth 64 (rpi:vc4_crtc.c:1336)
 * less 3 x HVS_FIFO_LATENCY_PIX 6 (rpi:vc4_crtc.c:52,292) */
#define YD5_R3_PV_FIFO_LEVEL_24BPP 46u

/* ---- steps ----------------------------------------------------------------- */
enum yd5_r3_kind {
	YD5_R3_CLOCK = 1,
	YD5_R3_WRITE,
	YD5_R3_DELAY,
	YD5_R3_PLACEHOLDER,
};

/* Register blocks. HDMI0 has nine (hdmi_ctrl.h's enum yd5_hdmi_region): block
 * YD5_R3_BLK_HDMI0 + region. Offsets are from the block's base. */
enum yd5_r3_block {
	YD5_R3_BLK_HVS = 1, /* HVS registers and list memory */
	YD5_R3_BLK_PV,      /* pixel valve 0 (feeds HDMI0) */
	YD5_R3_BLK_HDMI0,   /* + enum yd5_hdmi_region */
};
#define YD5_R3_BLK_HDMI(region) ((uint8_t)(YD5_R3_BLK_HDMI0 + (region)))

enum yd5_r3_clk_op {
	YD5_R3_CLK_QUERY = 1, /* parse with yd5_fwclk_parse_query(buf, nids, ...) */
	YD5_R3_CLK_SET_RATE,  /* parse with yd5_fwclk_parse_set_rate(buf, id, ...) */
};

#define YD5_R3_CLK_QUERY_IDS 4u
#define YD5_R3_MBOX_WORDS    64u /* >= YD5_FWCLK_QUERY_WORDS(4) = 63 */

struct yd5_r3_step {
	uint8_t kind;     /* enum yd5_r3_kind */
	const char *name; /* stage label, a string literal */
	struct {
		uint8_t block; /* enum yd5_r3_block */
		uint32_t offset, value, mask;
	} w;
	struct {
		uint32_t us;
		uint8_t poll; /* nonzero: poll {block, offset} for (reg & mask) == want */
		uint8_t block;
		uint32_t offset, mask, want;
	} d;
	struct {
		uint8_t op;      /* enum yd5_r3_clk_op */
		uint32_t nids;   /* QUERY: clocks in the buffer (ids[]) */
		uint32_t ids[YD5_R3_CLK_QUERY_IDS];
		uint32_t id, hz; /* SET_RATE: clock and requested rate */
		uint32_t words;  /* buffer length in words */
		uint32_t buf[YD5_R3_MBOX_WORDS];
	} c;
};

#define YD5_R3_MAX_STEPS 80u

struct yd5_route3 {
	struct yd5_r3_step s[YD5_R3_MAX_STEPS];
	unsigned int n;
};

/* The mode and the choices the plan cannot make itself. */
struct yd5_route3_mode {
	uint8_t vic;                  /* 1, 4 or 16 (pv_timing.h's table) */
	enum yd5_hvs6_variant hvs;    /* from the HVS VERSION register */
	uint32_t dlist_offset_words;  /* >= YD5_HVS6_BOOTLOADER_DLIST_END */
	enum hvs6w_format format;     /* the framebuffer's pixel format */
	uint32_t fifo_level;          /* PV_CONTROL FIFO_LEVEL (6 bits) */
	uint32_t bg_rgb;              /* HVS channel background */
};

#define YD5_R3_EINVAL (-22)
#define YD5_R3_ENOSPC (-28)

/* HVS channel and output of pixel valve 0 / HDMI0 (rpi:vc4_crtc.c:1331-1357,
 * rpi:vc4_hvs.c:916-930: output 0 is FIFO 0). */
#define YD5_R3_HVS_OUTPUT 0u

/* Waits (rpi:vc4_hdmi.c:1673-1681, :1439-1475, :910; rpi:vc4_crtc.c:516-519) */
#define YD5_R3_WAIT_HDMI_ACTIVE_US 1000000u
#define YD5_R3_WAIT_1MS_US         1000u
#define YD5_R3_WAIT_20MS_US        20000u
/* HVS stop: one frame at 24 Hz or faster is under 50 ms */
#define YD5_R3_WAIT_HVS_STOP_US    50000u

static inline void yd5_route3_mode_default(struct yd5_route3_mode *m, uint8_t vic)
{
	m->vic = vic;
	m->hvs = YD5_HVS6_C0;
	m->dlist_offset_words = YD5_HVS6_BOOTLOADER_DLIST_END;
	m->format = HVS6W_XRGB8888;
	m->fifo_level = YD5_R3_PV_FIFO_LEVEL_24BPP;
	m->bg_rgb = 0;
}

/* ---- appenders -------------------------------------------------------------- */
static inline struct yd5_r3_step *yd5_r3_new_(struct yd5_route3 *p, uint8_t kind,
					      const char *name)
{
	struct yd5_r3_step *s;
	struct yd5_r3_step zero = {0};

	if (p->n >= YD5_R3_MAX_STEPS)
		return NULL;
	s = &p->s[p->n++];
	*s = zero;
	s->kind = kind;
	s->name = name;
	return s;
}

static inline int yd5_r3_write_(struct yd5_route3 *p, const char *name, uint8_t block,
				uint32_t offset, uint32_t value, uint32_t mask)
{
	struct yd5_r3_step *s = yd5_r3_new_(p, YD5_R3_WRITE, name);

	if (s == NULL)
		return YD5_R3_ENOSPC;
	s->w.block = block;
	s->w.offset = offset;
	s->w.value = value & mask;
	s->w.mask = mask;
	return 0;
}

static inline int yd5_r3_delay_(struct yd5_route3 *p, const char *name, uint32_t us)
{
	struct yd5_r3_step *s = yd5_r3_new_(p, YD5_R3_DELAY, name);

	if (s == NULL)
		return YD5_R3_ENOSPC;
	s->d.us = us;
	return 0;
}

static inline int yd5_r3_poll_(struct yd5_route3 *p, const char *name, uint32_t us,
			       uint8_t block, uint32_t offset, uint32_t mask, uint32_t want)
{
	struct yd5_r3_step *s = yd5_r3_new_(p, YD5_R3_DELAY, name);

	if (s == NULL)
		return YD5_R3_ENOSPC;
	s->d.us = us;
	s->d.poll = 1;
	s->d.block = block;
	s->d.offset = offset;
	s->d.mask = mask;
	s->d.want = want & mask;
	return 0;
}

static inline int yd5_r3_placeholder_(struct yd5_route3 *p, const char *name)
{
	return yd5_r3_new_(p, YD5_R3_PLACEHOLDER, name) ? 0 : YD5_R3_ENOSPC;
}

static inline int yd5_r3_set_rate_(struct yd5_route3 *p, const char *name, uint32_t id,
				   uint32_t hz)
{
	struct yd5_r3_step *s = yd5_r3_new_(p, YD5_R3_CLOCK, name);

	if (s == NULL)
		return YD5_R3_ENOSPC;
	s->c.op = YD5_R3_CLK_SET_RATE;
	s->c.id = id;
	s->c.hz = hz;
	s->c.words = (uint32_t)yd5_fwclk_build_set_rate(s->c.buf, YD5_R3_MBOX_WORDS, id, hz, 1);
	return s->c.words ? 0 : YD5_R3_EINVAL;
}

static inline int yd5_r3_query_(struct yd5_route3 *p, const char *name)
{
	static const uint32_t ids[YD5_R3_CLK_QUERY_IDS] = {
		YD5_CLK_CORE, YD5_CLK_DISP, YD5_CLK_M2MC, YD5_CLK_PIXEL_BVB,
	};
	struct yd5_r3_step *s = yd5_r3_new_(p, YD5_R3_CLOCK, name);
	unsigned int i;

	if (s == NULL)
		return YD5_R3_ENOSPC;
	s->c.op = YD5_R3_CLK_QUERY;
	s->c.nids = YD5_R3_CLK_QUERY_IDS;
	for (i = 0; i < YD5_R3_CLK_QUERY_IDS; i++)
		s->c.ids[i] = ids[i];
	s->c.words = (uint32_t)yd5_fwclk_build_query(s->c.buf, YD5_R3_MBOX_WORDS, ids,
						     YD5_R3_CLK_QUERY_IDS);
	return s->c.words ? 0 : YD5_R3_EINVAL;
}

/* Append entries [from, to) of an HDMI log as writes. */
static inline int yd5_r3_hdmi_log_(struct yd5_route3 *p, const char *name,
				   const struct yd5_hdmi_log *log, unsigned int from,
				   unsigned int to)
{
	unsigned int i;
	int r;

	for (i = from; i < to; i++) {
		r = yd5_r3_write_(p, name, YD5_R3_BLK_HDMI(log->w[i].region), log->w[i].offset,
				  log->w[i].value, log->w[i].mask);
		if (r)
			return r;
	}
	return 0;
}

static inline int yd5_r3_hvs_log_(struct yd5_route3 *p, const char *name,
				  const struct yd5_hvs6_log *log)
{
	uint32_t i;
	int r;

	for (i = 0; i < log->n; i++) {
		r = yd5_r3_write_(p, name, YD5_R3_BLK_HVS, log->w[i].offset, log->w[i].value,
				  0xffffffffu);
		if (r)
			return r;
	}
	return 0;
}

static inline void yd5_r3_hdmi_timing_(const struct pvt_mode *m, struct yd5_hdmi_timing *t)
{
	t->pixel_clock_hz = m->pixel_clock_khz * 1000u;
	t->h_active = (uint16_t)m->hactive;
	t->h_fp = (uint16_t)m->hfp;
	t->h_sync = (uint16_t)m->hsync;
	t->h_bp = (uint16_t)m->hbp;
	t->v_active = (uint16_t)m->vactive;
	t->v_fp = (uint16_t)m->vfp;
	t->v_sync = (uint16_t)m->vsync;
	t->v_bp = (uint16_t)m->vbp;
	t->hsync_pos = m->hsync_positive ? 1 : 0;
	t->vsync_pos = m->vsync_positive ? 1 : 0;
}

/* ---- the plan ----------------------------------------------------------------- */

/*
 * Fill *out with the bring-up of `mode` scanning out the framebuffer at bus
 * address fb_addr (40 bits; bus_addr.h translates) with `pitch` bytes a row.
 * Returns 0, or YD5_R3_EINVAL / YD5_R3_ENOSPC with out->n = 0 (out NULL or
 * mode NULL: EINVAL). Checked before anything is appended: the VIC is in
 * pv_timing.h's table, the plane is writable (hvs6w_write_plane), the list
 * fits under the 12-bit head field, the FIFO level fits 6 bits, the HDMI
 * timing passes yd5_hdmi_timing_check.
 */
static inline int yd5_route3_plan(const struct yd5_route3_mode *mode, uint64_t fb_addr,
				  uint32_t pitch, struct yd5_route3 *out)
{
	const struct pvt_mode *m;
	struct pvt_regs pv;
	struct hvs6w_plane pl;
	struct yd5_hdmi_timing t;
	struct yd5_hdmi_log hl;
	struct yd5_hvs6_log vl;
	struct yd5_hdmi_clocks clk;
	uint32_t dl[HVS6W_LIST_WORDS];
	uint32_t control, i, chan;
	unsigned int cfg_end, pre_end, lows;
	int r, fifo;

	if (out == NULL)
		return YD5_R3_EINVAL;
	out->n = 0;
	if (mode == NULL)
		return YD5_R3_EINVAL;
	m = pvt_find_vic(mode->vic);
	if (m == NULL || mode->fifo_level > PV_CONTROL_FIFO_LEVEL_MASK ||
	    mode->dlist_offset_words + HVS6W_LIST_WORDS - 1u > YD5_HVS6_LPTRS_HEADE_MASK)
		return YD5_R3_EINVAL;
	fifo = yd5_hvs6_fifo_from_output(YD5_R3_HVS_OUTPUT);
	if (fifo < 0)
		return YD5_R3_EINVAL;
	chan = (uint32_t)fifo;

	pl.format = mode->format;
	pl.x = 0;
	pl.y = 0;
	pl.width = m->hactive;
	pl.height = m->vactive;
	pl.pitch = pitch;
	pl.addr = fb_addr;
	if (hvs6w_write_plane(dl, HVS6W_LIST_WORDS, &pl) != (int)HVS6W_LIST_WORDS)
		return YD5_R3_EINVAL;

	yd5_hvs6_log_init(&vl);
	if (yd5_hvs6_chan_enable_v(&vl, mode->hvs, chan, m->hactive, m->vactive,
				   mode->dlist_offset_words, mode->bg_rgb) != 0)
		return YD5_R3_EINVAL;

	yd5_r3_hdmi_timing_(m, &t);
	yd5_hdmi_log_init(&hl);
	if (yd5_hdmi_dvi_configure(&hl, &t) != YD5_HDMI_OK)
		return YD5_R3_EINVAL;
	cfg_end = hl.n;
	if (yd5_hdmi_dvi_pre_enable(&hl) != YD5_HDMI_OK)
		return YD5_R3_EINVAL;
	pre_end = hl.n;
	if (yd5_hdmi_dvi_post_enable(&hl, &t) != YD5_HDMI_OK)
		return YD5_R3_EINVAL;

	pvt_regs_from_mode(m, &pv);
	clk = yd5_hdmi_clocks((uint64_t)t.pixel_clock_hz, 8);
	control = pv.control | (mode->fifo_level << PV_CONTROL_FIFO_LEVEL_SHIFT) |
		  (YD5_R3_PV_CLK_SELECT_HDMI << PV_CONTROL_CLK_SELECT_SHIFT);

	/* 1. clocks the HVS needs, read back (rpi:vc4_hvs.c:2111-2135) */
	r = yd5_r3_query_(out, "clk-query");
	/* 2. the list, before anything points at it */
	for (i = 0; !r && i < HVS6W_LIST_WORDS; i++)
		r = yd5_r3_write_(out, "dlist", YD5_R3_BLK_HVS,
				  HVS6_DLIST_OFFSET + 4u * (mode->dlist_offset_words + i), dl[i],
				  0xffffffffu);
	/* 3. HVS channel (rpi:vc4_crtc.c:680-700, rpi:vc4_hvs.c:1218-1237) */
	if (!r)
		r = yd5_r3_hvs_log_(out, "hvs-chan", &vl);
	/* 4. encoder pre_crtc_configure (rpi:vc4_hdmi.c:1481-1595) */
	if (!r)
		r = yd5_r3_set_rate_(out, "clk-hsm", YD5_CLK_M2MC, (uint32_t)clk.hsm_min_hz);
	if (!r)
		r = yd5_r3_set_rate_(out, "clk-bvb", YD5_CLK_PIXEL_BVB, clk.bvb_hz);
	if (!r)
		r = yd5_r3_placeholder_(out, "phy-init");
	if (!r)
		r = yd5_r3_hdmi_log_(out, "hdmi-configure", &hl, 0, cfg_end);
	/* 5. vc4_crtc_config_pv, then EN (rpi:vc4_crtc.c:329-343,387-480,688) */
	if (!r)
		r = yd5_r3_write_(out, "pv-reset", YD5_R3_BLK_PV, PV_CONTROL, 0, PV_CONTROL_EN);
	if (!r)
		r = yd5_r3_write_(out, "pv-reset", YD5_R3_BLK_PV, PV_CONTROL, PV_CONTROL_FIFO_CLR,
				  PV_CONTROL_FIFO_CLR);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_HORZA, pv.horza, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_HORZB, pv.horzb, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_V_CONTROL,
				  pv.vcontrol | YD5_R3_PV_VCONTROL_ODD_TIMING, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, YD5_R3_PV_VSYNCD_EVEN, 0,
				  0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_VERTA, pv.verta, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_VERTB, pv.vertb, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, YD5_R3_PV_MUX_CFG,
				  YD5_R3_PV_MUX_RGB_NO_SWAP << YD5_R3_PV_MUX_RGB_SHIFT, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, YD5_R3_PV_PIPE_INIT_CTRL,
				  YD5_R3_PV_PIPE_INIT_VALUE, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-config", YD5_R3_BLK_PV, PV_CONTROL, control, 0xffffffffu);
	if (!r)
		r = yd5_r3_write_(out, "pv-enable", YD5_R3_BLK_PV, PV_CONTROL, PV_CONTROL_EN,
				  PV_CONTROL_EN);
	/* 6. encoder pre_crtc_enable (rpi:vc4_hdmi.c:1597-1625) */
	if (!r)
		r = yd5_r3_hdmi_log_(out, "hdmi-pre-enable", &hl, cfg_end, pre_end);
	/* 7. video on (rpi:vc4_crtc.c:696-697) */
	if (!r)
		r = yd5_r3_write_(out, "pv-viden", YD5_R3_BLK_PV, PV_V_CONTROL, PV_VCONTROL_VIDEN,
				  PV_VCONTROL_VIDEN);
	/* 8. encoder post_crtc_enable with its waits (rpi:vc4_hdmi.c:1627-1700):
	 * a wait goes after the scheduler write, between the RECENTER pairs and
	 * at the end */
	for (i = pre_end, lows = 0; !r && i < hl.n; i++) {
		const struct yd5_hdmi_write *w = &hl.w[i];

		/* the second RECENTER-low write starts the second pair */
		if (w->region == YD5_HDMI_FIFO_CTL.region && w->offset == YD5_HDMI_FIFO_CTL.offset &&
		    w->mask == YD5_FIFO_CTL_RECENTER && w->value == 0 && ++lows == 2u)
			r = yd5_r3_delay_(out, "wait-1ms", YD5_R3_WAIT_1MS_US);
		if (!r)
			r = yd5_r3_hdmi_log_(out, "hdmi-post-enable", &hl, i, i + 1u);
		if (!r && w->region == YD5_HDMI_SCHEDULER_CONTROL.region &&
		    w->offset == YD5_HDMI_SCHEDULER_CONTROL.offset)
			r = yd5_r3_poll_(out, "wait-hdmi-active", YD5_R3_WAIT_HDMI_ACTIVE_US,
					 YD5_R3_BLK_HDMI(w->region), w->offset,
					 YD5_SCHED_HDMI_ACTIVE, 0);
	}
	if (!r)
		r = yd5_r3_poll_(out, "wait-recenter", YD5_R3_WAIT_1MS_US,
				 YD5_R3_BLK_HDMI(YD5_HDMI_FIFO_CTL.region), YD5_HDMI_FIFO_CTL.offset,
				 YD5_FIFO_CTL_RECENTER_DONE, YD5_FIFO_CTL_RECENTER_DONE);
	if (r)
		out->n = 0;
	return r;
}

/*
 * Fill *out with the stop of what yd5_route3_plan started (only mode->hvs is
 * read). Returns 0, or YD5_R3_EINVAL / YD5_R3_ENOSPC with out->n = 0.
 */
static inline int yd5_route3_teardown(const struct yd5_route3_mode *mode, struct yd5_route3 *out)
{
	struct yd5_hdmi_log hl;
	struct yd5_hvs6_log vl;
	unsigned int i, split;
	uint32_t chan;
	int r, fifo;

	if (out == NULL)
		return YD5_R3_EINVAL;
	out->n = 0;
	if (mode == NULL)
		return YD5_R3_EINVAL;
	fifo = yd5_hvs6_fifo_from_output(YD5_R3_HVS_OUTPUT);
	if (fifo < 0)
		return YD5_R3_EINVAL;
	chan = (uint32_t)fifo;
	yd5_hvs6_log_init(&vl);
	if (yd5_hvs6_chan_disable_v(&vl, mode->hvs, chan) != 0)
		return YD5_R3_EINVAL;
	yd5_hdmi_log_init(&hl);
	if (yd5_hdmi_disable(&hl) != YD5_HDMI_OK)
		return YD5_R3_EINVAL;
	/* post_crtc_disable ends with its first BLANKPIX write (rpi:vc4_hdmi.c:904-910);
	 * what follows is post_crtc_powerdown (:947-948) */
	split = hl.n;
	for (i = 0; i < hl.n; i++)
		if (hl.w[i].region == YD5_HDMI_VID_CTL.region &&
		    hl.w[i].offset == YD5_HDMI_VID_CTL.offset &&
		    hl.w[i].mask == YD5_VID_CTL_BLANKPIX) {
			split = i + 1u;
			break;
		}

	/* rpi:vc4_crtc.c:516-519 */
	r = yd5_r3_write_(out, "pv-viden-off", YD5_R3_BLK_PV, PV_V_CONTROL, 0, PV_VCONTROL_VIDEN);
	if (!r)
		r = yd5_r3_poll_(out, "wait-viden", YD5_R3_WAIT_1MS_US, YD5_R3_BLK_PV, PV_V_CONTROL,
				 PV_VCONTROL_VIDEN, 0);
	/* rpi:vc4_crtc.c:537 */
	if (!r)
		r = yd5_r3_delay_(out, "wait-20ms", YD5_R3_WAIT_20MS_US);
	if (!r)
		r = yd5_r3_hdmi_log_(out, "hdmi-disable", &hl, 0, split);
	if (!r)
		r = yd5_r3_delay_(out, "wait-1ms", YD5_R3_WAIT_1MS_US);
	/* pixel valve reset (rpi:vc4_crtc.c:329-343) */
	if (!r)
		r = yd5_r3_write_(out, "pv-reset", YD5_R3_BLK_PV, PV_CONTROL, 0, PV_CONTROL_EN);
	if (!r)
		r = yd5_r3_write_(out, "pv-reset", YD5_R3_BLK_PV, PV_CONTROL, PV_CONTROL_FIFO_CLR,
				  PV_CONTROL_FIFO_CLR);
	/* HVS channel stop (rpi:vc4_hvs.c:1072-1097) */
	if (!r)
		r = yd5_r3_hvs_log_(out, "hvs-stop", &vl);
	if (!r)
		r = yd5_r3_poll_(out, "wait-hvs-disabled", YD5_R3_WAIT_HVS_STOP_US, YD5_R3_BLK_HVS,
				 yd5_hvs6_status_reg(mode->hvs, chan),
				 YD5_HVS6_STATUS_MODE_MASK << YD5_HVS6_STATUS_MODE_SHIFT,
				 YD5_HVS6_STATUS_MODE_DISABLED << YD5_HVS6_STATUS_MODE_SHIFT);
	/* post_crtc_powerdown (rpi:vc4_hdmi.c:932-960) */
	if (!r)
		r = yd5_r3_hdmi_log_(out, "hdmi-powerdown", &hl, split, hl.n);
	if (!r)
		r = yd5_r3_placeholder_(out, "phy-disable");
	if (r)
		out->n = 0;
	return r;
}

#endif /* YAKOGL_RPI5_SCANOUT_ROUTE3_H */
