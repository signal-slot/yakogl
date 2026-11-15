/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pv_hdmi.h: decodes the BCM2712 pixel valve and HDMI0 state a firmware
 * scanout leaves (Route A of the Pi 5 display work, driver/docs/rpi5-display.md).
 * Pure functions over register values the caller has read; there is no MMIO
 * here and nothing is written.
 *
 * Citations: "rpi:" is raspberrypi/linux rpi-6.12.y at
 * 5bd564aaa8dc4b1b8ed11733c19ec8ece63a72cc (driver/docs/rpi5-v3d71-facts.md,
 * sources table). Only facts (addresses, offsets, bit positions) are kept.
 * Addresses are ARM physical: the device tree's soc node maps child address
 * 0x0 to 0x10_0000_0000 (rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:189-191).
 */
#ifndef YAKOGL_RPI5_SCANOUT_PV_HDMI_H
#define YAKOGL_RPI5_SCANOUT_PV_HDMI_H

#include <stdbool.h>
#include <stdint.h>

/* ---- where the blocks are (ARM physical) -------------------------------- */

/* pixelvalve0: reg <0x7c410000 0x100> */
#define PV0_BASE 0x107c410000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:294 */
#define PV0_SIZE 0x100u          /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:294 */
/* pixelvalve1: reg <0x7c411000 0x100> */
#define PV1_BASE 0x107c411000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:300 */
#define PV1_SIZE 0x100u          /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:300 */
/* hdmi0 region "hdmi": first reg entry, 0x7c701400 size 0x300 */
#define HDMI0_HDMI_BASE 0x107c701400ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:372 */
#define HDMI0_HDMI_SIZE 0x300u          /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:372 */
/* hdmi0 region "hd": ninth reg entry, 0x7c720000 size 0x100 */
#define HDMI0_HD_BASE 0x107c720000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:380 */
#define HDMI0_HD_SIZE 0x100u          /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:380 */
/* position of "hdmi" and "hd" in reg-names */
#define HDMI0_REG_NAME_HDMI_IDX 0 /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:381 */
#define HDMI0_REG_NAME_HD_IDX 8   /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:389 */

/* ---- pixel valve register offsets (from PVn_BASE) ------------------------ */

#define PV_CONTROL 0x00   /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:145 */
#define PV_V_CONTROL 0x04 /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:171 */
#define PV_HORZA 0x0c     /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:184 */
#define PV_HORZB 0x10     /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:190 */
#define PV_VERTA 0x14     /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:196 */
#define PV_VERTB 0x18     /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:202 */

/* PV_CONTROL fields */
#define PV_CONTROL_EN (1u << 0)        /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:169 */
#define PV_CONTROL_FIFO_CLR (1u << 1)  /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:168 */
#define PV_CONTROL_CLK_SELECT_SHIFT 2  /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:167 */
#define PV_CONTROL_CLK_SELECT_MASK 0x3u /* bits 3:2, rpi:drivers/gpu/drm/vc4/vc4_regs.h:166 */
#define PV_CONTROL_PIXEL_REP_SHIFT 4   /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:162 */
#define PV_CONTROL_PIXEL_REP_MASK 0x3u /* bits 5:4, rpi:drivers/gpu/drm/vc4/vc4_regs.h:161 */
#define PV_CONTROL_WAIT_HSTART (1u << 12)       /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:160 */
#define PV_CONTROL_TRIGGER_UNDERFLOW (1u << 13) /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:159 */
#define PV_CONTROL_CLR_AT_START (1u << 14)      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:158 */
#define PV_CONTROL_FIFO_LEVEL_SHIFT 15   /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:157 */
#define PV_CONTROL_FIFO_LEVEL_MASK 0x3fu /* bits 20:15, rpi:drivers/gpu/drm/vc4/vc4_regs.h:156 */
#define PV_CONTROL_FORMAT_SHIFT 21       /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:149 */
#define PV_CONTROL_FORMAT_MASK 0x7u      /* bits 23:21, rpi:drivers/gpu/drm/vc4/vc4_regs.h:148 */
#define PV_CONTROL_FORMAT_24 0           /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:150 */
#define PV_CONTROL_FORMAT_DSIV_16 1      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:151 */
#define PV_CONTROL_FORMAT_DSIC_16 2      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:152 */
#define PV_CONTROL_FORMAT_DSIV_18 3      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:153 */
#define PV_CONTROL_FORMAT_DSIV_24 4      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:154 */
#define PV5_CONTROL_FIFO_LEVEL_HIGH_SHIFT 25  /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:147 */
#define PV5_CONTROL_FIFO_LEVEL_HIGH_MASK 0x3u /* bits 26:25, rpi:drivers/gpu/drm/vc4/vc4_regs.h:146 */

/* PV_V_CONTROL fields */
#define PV_VCONTROL_VIDEN (1u << 0)      /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:180 */
#define PV_VCONTROL_CONTINUOUS (1u << 1) /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:179 */
#define PV_VCONTROL_COMMAND (1u << 2)    /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:178 */
#define PV_VCONTROL_DSI (1u << 3)        /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:177 */
#define PV_VCONTROL_INTERLACE (1u << 4)  /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:176 */

/* Timing registers: two 16-bit fields each. A: back porch / sync; B: front
 * porch / active. Fields at bits 31:16 and 15:0
 * (rpi:drivers/gpu/drm/vc4/vc4_regs.h:185-188 HORZA, :191-194 HORZB,
 * :197-200 VERTA, :203-206 VERTB). */
#define PV_TIMING_HI_SHIFT 16 /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:186 */
#define PV_TIMING_LO_MASK 0xffffu /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:187 */

/* ---- HDMI0 hd region ------------------------------------------------------ */

/* HDMI_VID_CTL offset in the hd region, BCM2712 hdmi0 table (variant at
 * rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:3538-3544). */
#define HD_VID_CTL 0x0044 /* rpi:drivers/gpu/drm/vc4/vc4_hdmi_regs.h:438 */

/* HD_VID_CTL bit 31: the firmware left HDMI running; vc4_hdmi_bind tests it
 * (rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:3391-3399). */
#define HD_VID_CTL_ENABLE (1u << 31) /* rpi:drivers/gpu/drm/vc4/vc4_regs.h:1007 */

/* ---- decoders ------------------------------------------------------------- */

struct pv_control {
	bool enabled;       /* PV_CONTROL_EN */
	bool fifo_clr;      /* PV_CONTROL_FIFO_CLR */
	bool clr_at_start;
	bool wait_hstart;
	bool trigger_underflow;
	uint8_t clk_select; /* 1 = DPI/SMI/HDMI */
	uint8_t pixel_rep;  /* the field, not the factor */
	uint8_t format;     /* PV_CONTROL_FORMAT_*; 0 = 24 bit */
	uint8_t fifo_level; /* low 6 bits; the high 2 bits are separate */
	uint8_t fifo_level_high;
};

static inline struct pv_control pv_decode_control(uint32_t v)
{
	struct pv_control c = {
		.enabled = (v & PV_CONTROL_EN) != 0,
		.fifo_clr = (v & PV_CONTROL_FIFO_CLR) != 0,
		.clr_at_start = (v & PV_CONTROL_CLR_AT_START) != 0,
		.wait_hstart = (v & PV_CONTROL_WAIT_HSTART) != 0,
		.trigger_underflow = (v & PV_CONTROL_TRIGGER_UNDERFLOW) != 0,
		.clk_select = (v >> PV_CONTROL_CLK_SELECT_SHIFT) & PV_CONTROL_CLK_SELECT_MASK,
		.pixel_rep = (v >> PV_CONTROL_PIXEL_REP_SHIFT) & PV_CONTROL_PIXEL_REP_MASK,
		.format = (v >> PV_CONTROL_FORMAT_SHIFT) & PV_CONTROL_FORMAT_MASK,
		.fifo_level = (v >> PV_CONTROL_FIFO_LEVEL_SHIFT) & PV_CONTROL_FIFO_LEVEL_MASK,
		.fifo_level_high =
			(v >> PV5_CONTROL_FIFO_LEVEL_HIGH_SHIFT) & PV5_CONTROL_FIFO_LEVEL_HIGH_MASK,
	};
	return c;
}

struct pv_vcontrol {
	bool viden;
	bool continuous;
	bool command;
	bool dsi;
	bool interlace;
};

static inline struct pv_vcontrol pv_decode_vcontrol(uint32_t v)
{
	struct pv_vcontrol c = {
		.viden = (v & PV_VCONTROL_VIDEN) != 0,
		.continuous = (v & PV_VCONTROL_CONTINUOUS) != 0,
		.command = (v & PV_VCONTROL_COMMAND) != 0,
		.dsi = (v & PV_VCONTROL_DSI) != 0,
		.interlace = (v & PV_VCONTROL_INTERLACE) != 0,
	};
	return c;
}

/* The pixel valve produces video: enabled and video enabled. */
static inline bool pv_running(uint32_t control, uint32_t vcontrol)
{
	return (control & PV_CONTROL_EN) != 0 && (vcontrol & PV_VCONTROL_VIDEN) != 0;
}

/* Timing in pixels and lines; totals are the sum of the four parts (per
 * field when interlaced). */
struct pv_timing {
	uint32_t hactive, hsync, hbp, hfp, htotal;
	uint32_t vactive, vsync, vbp, vfp, vtotal;
};

static inline struct pv_timing pv_decode_timing(uint32_t horza, uint32_t horzb, uint32_t verta,
						uint32_t vertb)
{
	struct pv_timing t;

	t.hbp = horza >> PV_TIMING_HI_SHIFT;
	t.hsync = horza & PV_TIMING_LO_MASK;
	t.hfp = horzb >> PV_TIMING_HI_SHIFT;
	t.hactive = horzb & PV_TIMING_LO_MASK;
	t.vbp = verta >> PV_TIMING_HI_SHIFT;
	t.vsync = verta & PV_TIMING_LO_MASK;
	t.vfp = vertb >> PV_TIMING_HI_SHIFT;
	t.vactive = vertb & PV_TIMING_LO_MASK;
	t.htotal = t.hsync + t.hbp + t.hactive + t.hfp;
	t.vtotal = t.vsync + t.vbp + t.vactive + t.vfp;
	return t;
}

/* Both active sizes non-zero. A pixel valve the firmware never set up reads
 * zero. */
static inline bool pv_timing_valid(const struct pv_timing *t)
{
	return t->hactive != 0 && t->vactive != 0;
}

static inline bool hd_vid_ctl_enabled(uint32_t hd_vid_ctl)
{
	return (hd_vid_ctl & HD_VID_CTL_ENABLE) != 0;
}

#endif /* YAKOGL_RPI5_SCANOUT_PV_HDMI_H */
