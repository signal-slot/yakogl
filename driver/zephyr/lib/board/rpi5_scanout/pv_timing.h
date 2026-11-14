/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_timing.h: BCM2712 pixel valve register values for CEA-861 modes (Route C
 * of the Pi 5 display work, driver/docs/rpi5-display.md), and a comparison of
 * those values with a register snapshot. Pure functions; nothing is written.
 * Field layouts and addresses come from pv_hdmi.h, which is included and not
 * redefined (its struct pv_timing is the decoded form; this header's names
 * start with pvt_).
 *
 * Citations: "rpi:" is raspberrypi/linux rpi-6.12.y at
 * 5bd564aaa8dc4b1b8ed11733c19ec8ece63a72cc. Facts only. The function is
 * named instead of a line because the file was read as text without line
 * numbers; the register layout lines are in pv_hdmi.h.
 *
 * Linux's rule (rpi:drivers/gpu/drm/vc4/vc4_crtc.c, vc4_crtc_config_pv), for a
 * progressive HDMI mode with one pixel per clock and pixel_rep 1:
 *   HORZA = hbp << 16 | hsync,   hbp = htotal - hsync_end, hsync = hsync_end - hsync_start
 *   HORZB = hfp << 16 | hactive, hfp = hsync_start - hdisplay
 *   VERTA = vbp << 16 | vsync,   vbp = vtotal - vsync_end
 *   VERTB = vfp << 16 | vactive
 *   PV_V_CONTROL = CONTINUOUS (no INTERLACE, no ODD_DELAY); vc4_crtc_atomic_enable
 *     ORs in VIDEN after PV_CONTROL_EN.
 *   PV_CONTROL = FIFO_CLR | FIFO_LEVEL | FORMAT_24 | PIXEL_REP(pixel_rep - 1) |
 *     CLR_AT_START | TRIGGER_UNDERFLOW | WAIT_HSTART | CLK_SELECT(encoder);
 *     vc4_crtc_atomic_enable ORs in EN.
 * The FIFO level depends on the HVS FIFO depth and the clock select on the
 * encoder; neither is known here, so the values leave both zero and the
 * comparison ignores both. Pixels per clock above 1 (the register values are
 * divided by it) are not modelled; a firmware mode that uses it shows up as
 * a mismatch.
 */
#ifndef YAKOGL_RPI5_SCANOUT_PV_TIMING_H
#define YAKOGL_RPI5_SCANOUT_PV_TIMING_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "pv_hdmi.h"

/* A CEA-861 mode: timings in pixels and lines (CTA-861 mode tables). */
struct pvt_mode {
	uint8_t vic;
	const char *name;
	uint32_t pixel_clock_khz;
	uint32_t hactive, hfp, hsync, hbp;
	uint32_t vactive, vfp, vsync, vbp;
	bool hsync_positive; /* false = active low */
	bool vsync_positive;
};

static const struct pvt_mode pvt_modes[] = {
	{1, "640x480p60", 25175, 640, 16, 96, 48, 480, 10, 2, 33, false, false},
	{4, "1280x720p60", 74250, 1280, 110, 40, 220, 720, 5, 5, 20, true, true},
	{16, "1920x1080p60", 148500, 1920, 88, 44, 148, 1080, 4, 5, 36, true, true},
};
#define PVT_NUM_MODES (sizeof(pvt_modes) / sizeof(pvt_modes[0]))

/* NULL when the VIC is not in the table. */
static inline const struct pvt_mode *pvt_find_vic(uint8_t vic)
{
	for (size_t i = 0; i < PVT_NUM_MODES; i++) {
		if (pvt_modes[i].vic == vic) {
			return &pvt_modes[i];
		}
	}
	return NULL;
}

/* The register values of one mode. The polarities are not pixel valve
 * registers (they go to the HDMI block); they are kept beside the values. */
struct pvt_regs {
	uint32_t control;  /* without PV_CONTROL_EN, FIFO level and clock select */
	uint32_t vcontrol; /* without PV_VCONTROL_VIDEN */
	uint32_t horza, horzb, verta, vertb;
	bool hsync_positive, vsync_positive;
};

#define PVT_OK 0
#define PVT_ERR_NOT_FOUND (-1) /* no such VIC in the table */
#define PVT_ERR_ARG (-2)       /* NULL out pointer */

#define PVT_PACK(hi, lo) ((((uint32_t)(hi)) << PV_TIMING_HI_SHIFT) | (uint32_t)(lo))

static inline void pvt_regs_from_mode(const struct pvt_mode *m, struct pvt_regs *r)
{
	r->horza = PVT_PACK(m->hbp, m->hsync);
	r->horzb = PVT_PACK(m->hfp, m->hactive);
	r->verta = PVT_PACK(m->vbp, m->vsync);
	r->vertb = PVT_PACK(m->vfp, m->vactive);
	r->control = PV_CONTROL_FIFO_CLR | PV_CONTROL_CLR_AT_START | PV_CONTROL_TRIGGER_UNDERFLOW |
		     PV_CONTROL_WAIT_HSTART |
		     ((uint32_t)PV_CONTROL_FORMAT_24 << PV_CONTROL_FORMAT_SHIFT);
	r->vcontrol = PV_VCONTROL_CONTINUOUS;
	r->hsync_positive = m->hsync_positive;
	r->vsync_positive = m->vsync_positive;
}

/* Fills r for the VIC; returns PVT_OK, PVT_ERR_NOT_FOUND or PVT_ERR_ARG. */
static inline int pvt_regs_for_vic(uint8_t vic, struct pvt_regs *r)
{
	const struct pvt_mode *m;

	if (r == NULL) {
		return PVT_ERR_ARG;
	}
	m = pvt_find_vic(vic);
	if (m == NULL) {
		return PVT_ERR_NOT_FOUND;
	}
	pvt_regs_from_mode(m, r);
	return PVT_OK;
}

/* Fields that can differ: one bit each, in the order of pvt_diff_names. */
#define PVT_DIFF_HACTIVE (1u << 0)
#define PVT_DIFF_HFP (1u << 1)
#define PVT_DIFF_HSYNC (1u << 2)
#define PVT_DIFF_HBP (1u << 3)
#define PVT_DIFF_VACTIVE (1u << 4)
#define PVT_DIFF_VFP (1u << 5)
#define PVT_DIFF_VSYNC (1u << 6)
#define PVT_DIFF_VBP (1u << 7)
#define PVT_DIFF_FORMAT (1u << 8)
#define PVT_DIFF_PIXEL_REP (1u << 9)
#define PVT_DIFF_INTERLACE (1u << 10)
#define PVT_DIFF_CONTINUOUS (1u << 11)
#define PVT_DIFF_COUNT 12

static const char *const pvt_diff_names[PVT_DIFF_COUNT] = {
	"hactive", "hfp",    "hsync",     "hbp",       "vactive",    "vfp",
	"vsync",   "vbp",    "format",    "pixel_rep", "interlace",  "continuous",
};

/*
 * Compares the registers of an expected mode with a snapshot (PV_CONTROL,
 * PV_V_CONTROL, HORZA, HORZB, VERTA, VERTB as read). Returns the PVT_DIFF_*
 * bits of the fields that differ; 0 when the snapshot is that mode. Compared:
 * the eight timing fields, format, pixel repeat, interlace and continuous.
 * Not compared: EN, VIDEN, FIFO strobe and level, clock select and the
 * trigger flags (they say whether it runs, not which mode it runs).
 */
static inline uint32_t pvt_compare(const struct pvt_regs *want, uint32_t control,
				   uint32_t vcontrol, uint32_t horza, uint32_t horzb,
				   uint32_t verta, uint32_t vertb)
{
	struct pv_timing w = pv_decode_timing(want->horza, want->horzb, want->verta, want->vertb);
	struct pv_timing g = pv_decode_timing(horza, horzb, verta, vertb);
	struct pv_control wc = pv_decode_control(want->control);
	struct pv_control gc = pv_decode_control(control);
	struct pv_vcontrol wv = pv_decode_vcontrol(want->vcontrol);
	struct pv_vcontrol gv = pv_decode_vcontrol(vcontrol);
	uint32_t d = 0;

	d |= (w.hactive != g.hactive) ? PVT_DIFF_HACTIVE : 0;
	d |= (w.hfp != g.hfp) ? PVT_DIFF_HFP : 0;
	d |= (w.hsync != g.hsync) ? PVT_DIFF_HSYNC : 0;
	d |= (w.hbp != g.hbp) ? PVT_DIFF_HBP : 0;
	d |= (w.vactive != g.vactive) ? PVT_DIFF_VACTIVE : 0;
	d |= (w.vfp != g.vfp) ? PVT_DIFF_VFP : 0;
	d |= (w.vsync != g.vsync) ? PVT_DIFF_VSYNC : 0;
	d |= (w.vbp != g.vbp) ? PVT_DIFF_VBP : 0;
	d |= (wc.format != gc.format) ? PVT_DIFF_FORMAT : 0;
	d |= (wc.pixel_rep != gc.pixel_rep) ? PVT_DIFF_PIXEL_REP : 0;
	d |= (wv.interlace != gv.interlace) ? PVT_DIFF_INTERLACE : 0;
	d |= (wv.continuous != gv.continuous) ? PVT_DIFF_CONTINUOUS : 0;
	return d;
}

/* Writes the names of the set bits of a diff mask, space separated, into buf
 * of n bytes (with the NUL); "none" for 0. Returns the length, or -1 when it
 * did not fit (buf is then a NUL terminated prefix; n == 0 writes nothing). */
static inline int pvt_diff_str(uint32_t diff, char *buf, size_t n)
{
	size_t len = 0;

	if (n == 0) {
		return -1;
	}
	buf[0] = '\0';
	for (unsigned int i = 0; i <= PVT_DIFF_COUNT; i++) {
		const char *s;
		size_t sl = 0;

		if (i == PVT_DIFF_COUNT) {
			if (diff != 0) {
				break;
			}
			s = "none";
		} else if (diff & (1u << i)) {
			s = pvt_diff_names[i];
		} else {
			continue;
		}
		while (s[sl]) {
			sl++;
		}
		if (len + (len ? 1u : 0u) + sl + 1 > n) {
			return -1;
		}
		if (len) {
			buf[len++] = ' ';
		}
		for (size_t k = 0; k < sl; k++) {
			buf[len++] = s[k];
		}
		buf[len] = '\0';
	}
	return (int)len;
}

/* The VIC of the table mode a snapshot equals, 0 when none does. */
static inline uint8_t pvt_match_vic(uint32_t control, uint32_t vcontrol, uint32_t horza,
				    uint32_t horzb, uint32_t verta, uint32_t vertb)
{
	for (size_t i = 0; i < PVT_NUM_MODES; i++) {
		struct pvt_regs r;

		pvt_regs_from_mode(&pvt_modes[i], &r);
		if (pvt_compare(&r, control, vcontrol, horza, horzb, verta, vertb) == 0) {
			return pvt_modes[i].vic;
		}
	}
	return 0;
}

#endif /* YAKOGL_RPI5_SCANOUT_PV_TIMING_H */
