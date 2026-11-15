/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hdmi_ctrl.h: BCM2712 HDMI0 controller setup in DVI mode (RGB, 8 bits per
 * component, full range, no audio, no infoframes) as a list of register
 * writes. Header-only, C99, static inline. Nothing here touches hardware:
 * the caller (the probe, or a mock) plays the log.
 *
 * Facts only: offsets, fields and the order of the writes, read from the
 * Raspberry Pi Linux tree (branch rpi-6.12.y, "rpi:" below, all under
 * drivers/gpu/drm/vc4/) for `bcm2712_hdmi0_variant` (vc4_hdmi.c:3540-3546:
 * max pixel clock 600 MHz, register map `vc6_hdmi_hdmi0_fields`). No code is
 * copied.
 *
 * Out of scope (a separate item each):
 *  - the PHY (vc6_hdmi_phy_init, vc4_hdmi_phy.c) and its constant tables;
 *  - the firmware clocks (HSM/"hdmi" clock 13, "bvb" clock 14, pixel clock).
 *
 * Where they go in the order (Linux's, vc4_hdmi.c):
 *   1. clocks: HSM minimum rate, pixel clock rate and enable, BVB minimum
 *      rate and enable                                    (rpi:vc4_hdmi.c:1523-1561)
 *   2. PHY init (vc6_hdmi_phy_init)                       (rpi:vc4_hdmi.c:1565-1566)
 *   3. yd5_hdmi_dvi_configure(): scheduler control, timings (rpi:vc4_hdmi.c:1570-1578)
 *   4. yd5_hdmi_dvi_pre_enable(): CSC, FIFO master        (rpi:vc4_hdmi.c:1597-1621)
 *   5. START of the pixel valve (HVS channel and pixel valve are other items)
 *   6. yd5_hdmi_dvi_post_enable(): VID_CTL enable, DVI mode, FIFO recenter
 *                                                         (rpi:vc4_hdmi.c:1627-1703)
 * yd5_hdmi_dvi_enable() appends 3, 4 and 6 in that order; the caller runs
 * 1-2 before it and starts the pixel valve between 4 and 6 (use the three
 * phase functions to do so).
 *
 * Waits the Linux code does between writes (not entries of the log; the
 * caller supplies them): after the DVI-mode write, poll SCHEDULER_CONTROL
 * until HDMI_ACTIVE reads 0 (<= 1 s, rpi:vc4_hdmi.c:1673-1681); between the
 * two FIFO_CTL RECENTER pairs, 1 ms, and after them poll FIFO_CTL until
 * RECENTER_DONE reads 1 (<= 1 ms, rpi:vc4_hdmi.c:1439-1475). In
 * yd5_hdmi_disable(): 1 ms after the BLANKPIX write (rpi:vc4_hdmi.c:910).
 *
 * Read-modify-write registers: Linux reads the register and changes some
 * bits. A log entry carries `mask`: the bits the entry sets (to `value`'s
 * bits) while keeping the register's other bits; 0xffffffff is a plain
 * write. A player does `reg = (reg & ~mask) | (value & mask)`. FIFO_CTL's
 * recenter writes keep the register's other bits too (Linux masks the read
 * with 0xefff, rpi:vc4_regs.h:837), so they use mask = RECENTER only.
 *
 * Not done on purpose: scrambling and the SCDC setup (only above 340 MHz,
 * rpi:vc4_hdmi.c:3346; needs the DDC) and everything for HDMI mode
 * (infoframes, packet RAM, audio). The pixel clock limit is the 600 MHz of
 * the variant, but DVI is specified to 165 MHz and this header sets up no
 * scrambling: above 340 MHz the sink will not lock.
 */
#ifndef YAKOGL_RPI5_SCANOUT_HDMI_CTRL_H
#define YAKOGL_RPI5_SCANOUT_HDMI_CTRL_H

#include <stdint.h>

/* ---- the nine register regions of hdmi0 (bcm2712.dtsi:370-397) ---------- */
enum yd5_hdmi_region {
	YD5_HDMI_R_HDMI = 0, /* "hdmi" */
	YD5_HDMI_R_DVP,      /* "dvp" */
	YD5_HDMI_R_PHY,      /* "phy" */
	YD5_HDMI_R_RM,       /* "rm" */
	YD5_HDMI_R_PACKET,   /* "packet" */
	YD5_HDMI_R_METADATA, /* "metadata" */
	YD5_HDMI_R_CSC,      /* "csc" */
	YD5_HDMI_R_CEC,      /* "cec" */
	YD5_HDMI_R_HD,       /* "hd" */
	YD5_HDMI_R_COUNT
};

/* ---- registers as {region, offset}; comments are rpi:file:line ---------- */
struct yd5_hdmi_reg {
	uint8_t region; /* enum yd5_hdmi_region */
	uint16_t offset;
};

/* all in rpi:vc4_hdmi_regs.h (vc6_hdmi_hdmi0_fields) at the line shown */
#define YD5_HDMI_VID_CTL             ((struct yd5_hdmi_reg){YD5_HDMI_R_HD, 0x044})   /* rpi:vc4_hdmi_regs.h:438 */
#define YD5_HDMI_FIFO_CTL            ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x07c}) /* rpi:vc4_hdmi_regs.h:441 */
#define YD5_HDMI_RAM_PACKET_CONFIG   ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0c4}) /* rpi:vc4_hdmi_regs.h:443 */
#define YD5_HDMI_SCHEDULER_CONTROL   ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0e8}) /* rpi:vc4_hdmi_regs.h:448 */
#define YD5_HDMI_HORZA               ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0ec}) /* rpi:vc4_hdmi_regs.h:449 */
#define YD5_HDMI_HORZB               ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0f0}) /* rpi:vc4_hdmi_regs.h:450 */
#define YD5_HDMI_VERTA0              ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0f4}) /* rpi:vc4_hdmi_regs.h:451 */
#define YD5_HDMI_VERTB0              ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x0f8}) /* rpi:vc4_hdmi_regs.h:452 */
#define YD5_HDMI_VERTA1              ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x100}) /* rpi:vc4_hdmi_regs.h:453 */
#define YD5_HDMI_VERTB1              ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x104}) /* rpi:vc4_hdmi_regs.h:454 */
#define YD5_HDMI_MISC_CONTROL        ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x114}) /* rpi:vc4_hdmi_regs.h:455 */
#define YD5_HDMI_DEEP_COLOR_CONFIG_1 ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x18c}) /* rpi:vc4_hdmi_regs.h:468 */
#define YD5_HDMI_GCP_CONFIG          ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x194}) /* rpi:vc4_hdmi_regs.h:469 */
#define YD5_HDMI_GCP_WORD_1          ((struct yd5_hdmi_reg){YD5_HDMI_R_HDMI, 0x198}) /* rpi:vc4_hdmi_regs.h:470 */
#define YD5_HDMI_CLOCK_STOP          ((struct yd5_hdmi_reg){YD5_HDMI_R_DVP, 0x0bc})  /* rpi:vc4_hdmi_regs.h:474 */
#define YD5_HDMI_VEC_INTERFACE_CFG   ((struct yd5_hdmi_reg){YD5_HDMI_R_DVP, 0x0f0})  /* rpi:vc4_hdmi_regs.h:475 */
#define YD5_HDMI_VEC_INTERFACE_XBAR  ((struct yd5_hdmi_reg){YD5_HDMI_R_DVP, 0x0f4})  /* rpi:vc4_hdmi_regs.h:476 */
#define YD5_HDMI_CSC_CTL             ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x000})  /* rpi:vc4_hdmi_regs.h:521 */
#define YD5_HDMI_CSC_12_11           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x004})  /* rpi:vc4_hdmi_regs.h:522 */
#define YD5_HDMI_CSC_14_13           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x008})  /* rpi:vc4_hdmi_regs.h:523 */
#define YD5_HDMI_CSC_22_21           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x00c})  /* rpi:vc4_hdmi_regs.h:524 */
#define YD5_HDMI_CSC_24_23           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x010})  /* rpi:vc4_hdmi_regs.h:525 */
#define YD5_HDMI_CSC_32_31           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x014})  /* rpi:vc4_hdmi_regs.h:526 */
#define YD5_HDMI_CSC_34_33           ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x018})  /* rpi:vc4_hdmi_regs.h:527 */
#define YD5_HDMI_CSC_CHANNEL_CTL     ((struct yd5_hdmi_reg){YD5_HDMI_R_CSC, 0x02c})  /* rpi:vc4_hdmi_regs.h:528 */

/* ---- fields ------------------------------------------------------------- */
/* HD_VID_CTL, rpi:vc4_regs.h:1007-1015 */
#define YD5_VID_CTL_ENABLE              (1u << 31) /* rpi:vc4_regs.h:1007 */
#define YD5_VID_CTL_UNDERFLOW_ENABLE    (1u << 30) /* rpi:vc4_regs.h:1008 */
#define YD5_VID_CTL_FRAME_COUNTER_RESET (1u << 29) /* rpi:vc4_regs.h:1009 */
#define YD5_VID_CTL_VSYNC_LOW           (1u << 28) /* rpi:vc4_regs.h:1010 */
#define YD5_VID_CTL_HSYNC_LOW           (1u << 27) /* rpi:vc4_regs.h:1011 */
#define YD5_VID_CTL_CLRRGB              (1u << 23) /* rpi:vc4_regs.h:1013 */
#define YD5_VID_CTL_BLANKPIX            (1u << 18) /* rpi:vc4_regs.h:1014 */
#define YD5_VID_CTL_BLANK_INSERT_EN     (1u << 16) /* rpi:vc4_regs.h:1015 */
/* FIFO_CTL, rpi:vc4_regs.h:827-836 */
#define YD5_FIFO_CTL_RECENTER_DONE      (1u << 14) /* rpi:vc4_regs.h:827 (read) */
#define YD5_FIFO_CTL_RECENTER           (1u << 6)  /* rpi:vc4_regs.h:830 */
#define YD5_FIFO_CTL_MASTER_SLAVE_N     (1u << 0)  /* rpi:vc4_regs.h:836 */
/* SCHEDULER_CONTROL, rpi:vc4_regs.h:839-843 */
#define YD5_SCHED_MANUAL_FORMAT         (1u << 15) /* rpi:vc4_regs.h:839 */
#define YD5_SCHED_IGNORE_VSYNC_PREDICTS (1u << 5)  /* rpi:vc4_regs.h:840 */
#define YD5_SCHED_HDMI_ACTIVE           (1u << 1)  /* rpi:vc4_regs.h:842 (read) */
#define YD5_SCHED_MODE_HDMI             (1u << 0)  /* rpi:vc4_regs.h:843; 0 = DVI */
/* RAM_PACKET_CONFIG, rpi:vc4_regs.h:796 */
#define YD5_RAM_PACKET_ENABLE           (1u << 16)
/* HORZA (BCM2712 layout), rpi:vc4_hdmi.c:65-70 */
#define YD5_HORZA_HFP_SHIFT 16 /* bits 28:16 */
#define YD5_HORZA_VPOS      (1u << 15)
#define YD5_HORZA_HPOS      (1u << 14)
#define YD5_HORZA_HAP_SHIFT 0 /* bits 13:0 */
/* HORZB, rpi:vc4_hdmi.c:72-75 */
#define YD5_HORZB_HBP_SHIFT 16 /* bits 26:16 */
#define YD5_HORZB_HSP_SHIFT 0  /* bits 10:0 */
/* VERTA, rpi:vc4_hdmi.c:77-82 */
#define YD5_VERTA_VSP_SHIFT 24 /* bits 28:24 */
#define YD5_VERTA_VFP_SHIFT 16 /* bits 22:16 */
#define YD5_VERTA_VAL_SHIFT 0  /* bits 12:0 */
/* VERTB, rpi:vc4_hdmi.c:84-85 and rpi:vc4_regs.h:859-860 */
#define YD5_VERTB_VSPO_SHIFT 16 /* bits 29:16 */
/* MISC_CONTROL, rpi:vc4_hdmi.c:87-90 */
#define YD5_MISC_PIXEL_REP_MASK 0xfu /* bits 3:0 */
/* DEEP_COLOR_CONFIG_1, rpi:vc4_hdmi.c:94-98 */
#define YD5_DEEPCOLOR_PACK_PHASE_SHIFT 8 /* bits 10:8 */
#define YD5_DEEPCOLOR_PACK_PHASE_MASK  (7u << 8)
#define YD5_DEEPCOLOR_DEPTH_MASK       0xfu /* bits 3:0; 0 = 8 bpc */
/* GCP_CONFIG / GCP_WORD_1, rpi:vc4_hdmi.c:100-108 */
#define YD5_GCP_CONFIG_ENABLE            (1u << 31)
#define YD5_GCP_WORD1_BYTE1_MASK         0xff00u
#define YD5_GCP_WORD1_BYTE0_MASK         0x00ffu
#define YD5_GCP_WORD1_BYTE0_CLEAR_AVMUTE (1u << 4)
/* CSC, rpi:vc4_regs.h:1040-1041 and rpi:vc4_hdmi.c:1210-1216 */
#define YD5_CSC_CTL_ENABLE      (1u << 2)
#define YD5_CSC_CTL_MODE_CUSTOM 3u /* bits 1:0 (rpi:vc4_regs.h:1030) */
/* identity coefficient, signed 2p13: rpi:vc4_hdmi.c:1032-1034 */
#define YD5_CSC_ONE 0x2000u
/* DVP interface crossbar for RGB, rpi:vc4_hdmi.c:1249 */
#define YD5_VEC_XBAR_RGB 0x354021u
/* pixel clock limit of the variant, rpi:vc4_hdmi.c:3544 */
#define YD5_HDMI_MAX_PIXEL_CLOCK_HZ 600000000u

/* ---- timing (self-contained, progressive, pixel repetition 1) ----------- */
struct yd5_hdmi_timing {
	uint32_t pixel_clock_hz;
	uint16_t h_active, h_fp, h_sync, h_bp;
	uint16_t v_active, v_fp, v_sync, v_bp;
	uint8_t hsync_pos; /* nonzero: positive polarity */
	uint8_t vsync_pos;
};

/* ---- write log ----------------------------------------------------------- */
struct yd5_hdmi_write {
	uint8_t region; /* enum yd5_hdmi_region */
	uint16_t offset;
	uint32_t value;
	uint32_t mask; /* bits written; 0xffffffff = whole register */
};

#define YD5_HDMI_LOG_MAX 48

struct yd5_hdmi_log {
	struct yd5_hdmi_write w[YD5_HDMI_LOG_MAX];
	unsigned int n;
};

enum yd5_hdmi_err {
	YD5_HDMI_OK = 0,
	YD5_HDMI_E_ARG = -1,   /* null pointer or an empty timing */
	YD5_HDMI_E_CLOCK = -2, /* pixel clock above 600 MHz */
	YD5_HDMI_E_RANGE = -3, /* a field does not fit its register bits */
	YD5_HDMI_E_FULL = -4,  /* the log is full */
};

static inline void yd5_hdmi_log_init(struct yd5_hdmi_log *log)
{
	log->n = 0;
}

static inline int yd5_hdmi_put(struct yd5_hdmi_log *log, struct yd5_hdmi_reg r, uint32_t value,
			       uint32_t mask)
{
	if (log->n >= YD5_HDMI_LOG_MAX) {
		return YD5_HDMI_E_FULL;
	}
	log->w[log->n].region = r.region;
	log->w[log->n].offset = r.offset;
	log->w[log->n].value = value & mask;
	log->w[log->n].mask = mask;
	log->n++;
	return YD5_HDMI_OK;
}

#define YD5_HDMI_PUT_(log, r, v, m)                                                                \
	do {                                                                                       \
		int yd5_rc_ = yd5_hdmi_put((log), (r), (v), (m));                                  \
		if (yd5_rc_ != YD5_HDMI_OK) {                                                      \
			return yd5_rc_;                                                            \
		}                                                                                  \
	} while (0)

/* Does v fit in `bits` bits? */
static inline int yd5_hdmi_fits(uint32_t v, unsigned int bits)
{
	return v < (1u << bits);
}

/* Validate a timing: fields in their register widths, clock <= 600 MHz. */
static inline int yd5_hdmi_timing_check(const struct yd5_hdmi_timing *t)
{
	if (t == 0 || t->pixel_clock_hz == 0 || t->h_active == 0 || t->v_active == 0) {
		return YD5_HDMI_E_ARG;
	}
	if (t->pixel_clock_hz > YD5_HDMI_MAX_PIXEL_CLOCK_HZ) {
		return YD5_HDMI_E_CLOCK;
	}
	if (!yd5_hdmi_fits(t->h_active, 14) || !yd5_hdmi_fits(t->h_fp, 13) ||
	    !yd5_hdmi_fits(t->h_bp, 11) || !yd5_hdmi_fits(t->h_sync, 11) ||
	    !yd5_hdmi_fits(t->v_active, 13) || !yd5_hdmi_fits(t->v_fp, 7) ||
	    !yd5_hdmi_fits(t->v_sync, 5) || !yd5_hdmi_fits(t->v_bp, 9)) {
		return YD5_HDMI_E_RANGE;
	}
	/* VERTB1.VSPO = htotal / 2 (rpi:vc4_hdmi.c:1346), 14 bits */
	if (!yd5_hdmi_fits(((uint32_t)t->h_active + t->h_fp + t->h_sync + t->h_bp) >> 1, 14)) {
		return YD5_HDMI_E_RANGE;
	}
	return YD5_HDMI_OK;
}

/*
 * Phase 3: scheduler control and timings (vc5_hdmi_set_timings), in Linux's
 * order. rpi:vc4_hdmi.c:1570-1574 (scheduler) and :1332-1434 (timings).
 */
static inline int yd5_hdmi_dvi_configure(struct yd5_hdmi_log *log,
					 const struct yd5_hdmi_timing *t)
{
	uint32_t htotal, verta, vertb, vertb_even;
	int rc;

	if (log == 0) {
		return YD5_HDMI_E_ARG;
	}
	rc = yd5_hdmi_timing_check(t);
	if (rc != YD5_HDMI_OK) {
		return rc;
	}
	htotal = (uint32_t)t->h_active + t->h_fp + t->h_sync + t->h_bp;

	/* rpi:vc4_hdmi.c:1570-1574: |= MANUAL_FORMAT | IGNORE_VSYNC_PREDICTS */
	YD5_HDMI_PUT_(log, YD5_HDMI_SCHEDULER_CONTROL,
		      YD5_SCHED_MANUAL_FORMAT | YD5_SCHED_IGNORE_VSYNC_PREDICTS,
		      YD5_SCHED_MANUAL_FORMAT | YD5_SCHED_IGNORE_VSYNC_PREDICTS);

	/* rpi:vc4_hdmi.c:1365-1372 */
	YD5_HDMI_PUT_(log, YD5_HDMI_HORZA,
		      (t->vsync_pos ? YD5_HORZA_VPOS : 0u) | (t->hsync_pos ? YD5_HORZA_HPOS : 0u) |
			      ((uint32_t)t->h_active << YD5_HORZA_HAP_SHIFT) |
			      ((uint32_t)t->h_fp << YD5_HORZA_HFP_SHIFT),
		      0xffffffffu);
	/* rpi:vc4_hdmi.c:1374-1380 */
	YD5_HDMI_PUT_(log, YD5_HDMI_HORZB,
		      ((uint32_t)t->h_bp << YD5_HORZB_HBP_SHIFT) |
			      ((uint32_t)t->h_sync << YD5_HORZB_HSP_SHIFT),
		      0xffffffffu);

	/* rpi:vc4_hdmi.c:1341-1345 */
	verta = ((uint32_t)t->v_sync << YD5_VERTA_VSP_SHIFT) |
		((uint32_t)t->v_fp << YD5_VERTA_VFP_SHIFT) |
		((uint32_t)t->v_active << YD5_VERTA_VAL_SHIFT);
	/* rpi:vc4_hdmi.c:1346-1353 (pixel repetition 1, not interlaced) */
	vertb = ((htotal >> 1) << YD5_VERTB_VSPO_SHIFT) | (uint32_t)t->v_bp;
	vertb_even = (uint32_t)t->v_bp;

	/* rpi:vc4_hdmi.c:1382-1386 */
	YD5_HDMI_PUT_(log, YD5_HDMI_VERTA0, verta, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_VERTA1, verta, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_VERTB0, vertb_even, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_VERTB1, vertb, 0xffffffffu);

	/* rpi:vc4_hdmi.c:1409-1414: 8 bpc: gcp = 0, init pack phase 2 */
	YD5_HDMI_PUT_(log, YD5_HDMI_DEEP_COLOR_CONFIG_1, 2u << YD5_DEEPCOLOR_PACK_PHASE_SHIFT,
		      YD5_DEEPCOLOR_PACK_PHASE_MASK | YD5_DEEPCOLOR_DEPTH_MASK);
	/* rpi:vc4_hdmi.c:1416-1421: subpacket byte 1 = gcp (0), byte 0 = clear AVMUTE */
	YD5_HDMI_PUT_(log, YD5_HDMI_GCP_WORD_1, YD5_GCP_WORD1_BYTE0_CLEAR_AVMUTE,
		      YD5_GCP_WORD1_BYTE1_MASK | YD5_GCP_WORD1_BYTE0_MASK);
	/* rpi:vc4_hdmi.c:1423-1425 */
	YD5_HDMI_PUT_(log, YD5_HDMI_GCP_CONFIG, YD5_GCP_CONFIG_ENABLE, YD5_GCP_CONFIG_ENABLE);
	/* rpi:vc4_hdmi.c:1427-1430: pixel repetition 1 -> field 0 */
	YD5_HDMI_PUT_(log, YD5_HDMI_MISC_CONTROL, 0u, YD5_MISC_PIXEL_REP_MASK);
	/* rpi:vc4_hdmi.c:1432 */
	YD5_HDMI_PUT_(log, YD5_HDMI_CLOCK_STOP, 0u, 0xffffffffu);
	return YD5_HDMI_OK;
}

/*
 * Phase 4: colour space conversion for RGB full range (identity, custom
 * mode), the DVP crossbar, and the FIFO in master mode.
 * rpi:vc4_hdmi.c:1204-1266 (vc5_hdmi_csc_setup, RGB branch) and :1618 (the
 * FIFO_CTL write).
 */
static inline int yd5_hdmi_dvi_pre_enable(struct yd5_hdmi_log *log)
{
	if (log == 0) {
		return YD5_HDMI_E_ARG;
	}
	/* coefficients, rpi:vc4_hdmi.c:1149-1160 with the full-range unity
	 * matrix of :1023-1035; each word is (second << 16) | first */
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_12_11, YD5_CSC_ONE, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_14_13, 0u, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_22_21, YD5_CSC_ONE << 16, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_24_23, 0u, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_32_31, 0u, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_34_33, YD5_CSC_ONE, 0xffffffffu);
	/* rpi:vc4_hdmi.c:1258-1261: interface cfg 0, crossbar for RGB, channel
	 * control 0, CSC enabled in custom mode */
	YD5_HDMI_PUT_(log, YD5_HDMI_VEC_INTERFACE_CFG, 0u, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_VEC_INTERFACE_XBAR, YD5_VEC_XBAR_RGB, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_CHANNEL_CTL, 0u, 0xffffffffu);
	YD5_HDMI_PUT_(log, YD5_HDMI_CSC_CTL, YD5_CSC_CTL_ENABLE | YD5_CSC_CTL_MODE_CUSTOM,
		      0xffffffffu);
	/* rpi:vc4_hdmi.c:1618 */
	YD5_HDMI_PUT_(log, YD5_HDMI_FIFO_CTL, YD5_FIFO_CTL_MASTER_SLAVE_N, 0xffffffffu);
	return YD5_HDMI_OK;
}

/*
 * Phase 6 (after the pixel valve has started): VID_CTL enable with the sync
 * polarities, blanking released, DVI mode, FIFO recenter.
 * rpi:vc4_hdmi.c:1627-1703.
 */
static inline int yd5_hdmi_dvi_post_enable(struct yd5_hdmi_log *log,
					   const struct yd5_hdmi_timing *t)
{
	const uint32_t vid_set = YD5_VID_CTL_ENABLE | YD5_VID_CTL_CLRRGB |
				 YD5_VID_CTL_UNDERFLOW_ENABLE | YD5_VID_CTL_FRAME_COUNTER_RESET |
				 YD5_VID_CTL_BLANK_INSERT_EN;

	if (log == 0 || t == 0) {
		return YD5_HDMI_E_ARG;
	}
	/* rpi:vc4_hdmi.c:1648-1657: clear both polarity-low bits, then set
	 * ENABLE, CLRRGB, UNDERFLOW_ENABLE, FRAME_COUNTER_RESET,
	 * BLANK_INSERT_EN and the low bit of each negative sync */
	YD5_HDMI_PUT_(log, YD5_HDMI_VID_CTL,
		      vid_set | (t->vsync_pos ? 0u : YD5_VID_CTL_VSYNC_LOW) |
			      (t->hsync_pos ? 0u : YD5_VID_CTL_HSYNC_LOW),
		      vid_set | YD5_VID_CTL_VSYNC_LOW | YD5_VID_CTL_HSYNC_LOW);
	/* rpi:vc4_hdmi.c:1659-1660: release BLANKPIX */
	YD5_HDMI_PUT_(log, YD5_HDMI_VID_CTL, 0u, YD5_VID_CTL_BLANKPIX);
	/* rpi:vc4_hdmi.c:1673-1681 (DVI branch: MODE_HDMI cleared; then wait
	 * for HDMI_ACTIVE == 0) */
	YD5_HDMI_PUT_(log, YD5_HDMI_SCHEDULER_CONTROL, 0u, YD5_SCHED_MODE_HDMI);
	/* rpi:vc4_hdmi.c:1439-1475 (vc4_hdmi_recenter_fifo, called at :1698):
	 * RECENTER low, high, wait 1 ms, low, high, then poll RECENTER_DONE */
	YD5_HDMI_PUT_(log, YD5_HDMI_FIFO_CTL, 0u, YD5_FIFO_CTL_RECENTER);
	YD5_HDMI_PUT_(log, YD5_HDMI_FIFO_CTL, YD5_FIFO_CTL_RECENTER, YD5_FIFO_CTL_RECENTER);
	YD5_HDMI_PUT_(log, YD5_HDMI_FIFO_CTL, 0u, YD5_FIFO_CTL_RECENTER);
	YD5_HDMI_PUT_(log, YD5_HDMI_FIFO_CTL, YD5_FIFO_CTL_RECENTER, YD5_FIFO_CTL_RECENTER);
	return YD5_HDMI_OK;
}

/*
 * The controller writes of the enable, in Linux's order: phases 3, 4 and 6.
 * Refuses a bad timing or a pixel clock above 600 MHz (E_CLOCK) before
 * appending anything; on any error the log is left as it was. The caller
 * runs the clocks and the PHY init first and starts the pixel valve between
 * phase 4 and 6 (then call the phase functions instead).
 */
static inline int yd5_hdmi_dvi_enable(struct yd5_hdmi_log *log, const struct yd5_hdmi_timing *t)
{
	unsigned int start;
	int rc;

	if (log == 0) {
		return YD5_HDMI_E_ARG;
	}
	rc = yd5_hdmi_timing_check(t);
	if (rc != YD5_HDMI_OK) {
		return rc;
	}
	start = log->n;
	rc = yd5_hdmi_dvi_configure(log, t);
	if (rc == YD5_HDMI_OK) {
		rc = yd5_hdmi_dvi_pre_enable(log);
	}
	if (rc == YD5_HDMI_OK) {
		rc = yd5_hdmi_dvi_post_enable(log, t);
	}
	if (rc != YD5_HDMI_OK) {
		log->n = start;
	}
	return rc;
}

/*
 * The controller writes of the disable: rpi:vc4_hdmi.c:882-930
 * (post_crtc_disable) and :932-960 (post_crtc_powerdown). VID_CTL's ENABLE
 * is NOT cleared on the BCM2712: Linux skips it because it locks the system
 * up (rpi:vc4_hdmi.c:912-921). After the BLANKPIX write Linux waits 1 ms
 * (:910); after the last write it runs the PHY disable (vc6_hdmi_phy_disable)
 * and releases the clocks, which are not here. Scrambling is not touched.
 */
static inline int yd5_hdmi_disable(struct yd5_hdmi_log *log)
{
	if (log == 0) {
		return YD5_HDMI_E_ARG;
	}
	/* rpi:vc4_hdmi.c:900 */
	YD5_HDMI_PUT_(log, YD5_HDMI_RAM_PACKET_CONFIG, YD5_RAM_PACKET_ENABLE, 0xffffffffu);
	/* rpi:vc4_hdmi.c:902 */
	YD5_HDMI_PUT_(log, YD5_HDMI_VID_CTL, YD5_VID_CTL_CLRRGB, YD5_VID_CTL_CLRRGB);
	/* rpi:vc4_hdmi.c:904-906 (BCM2712 only) */
	YD5_HDMI_PUT_(log, YD5_HDMI_VID_CTL, YD5_VID_CTL_BLANKPIX, YD5_VID_CTL_BLANKPIX);
	/* rpi:vc4_hdmi.c:947-948 (post_crtc_powerdown) */
	YD5_HDMI_PUT_(log, YD5_HDMI_VID_CTL, YD5_VID_CTL_BLANKPIX, YD5_VID_CTL_BLANKPIX);
	return YD5_HDMI_OK;
}

#endif /* YAKOGL_RPI5_SCANOUT_HDMI_CTRL_H */
