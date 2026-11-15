/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fw_stop: the route-3 plan to stop a scanout the firmware may have left
 * running on the Pi 5 (BCM2712), before route 3 reprograms HVS6, the pixel
 * valve and HDMI0. Header only, C11, no Zephyr, no MMIO: the caller reads
 * the registers into a struct fw_stop_snap, fw_stop_plan() returns the
 * ordered register writes, and the caller performs them (read-modify-write:
 * new = (old & ~mask) | (value & mask)) and waits wait_us after each.
 *
 * Facts (own words; Linux is GPL, only file:line is kept; "rpi:" is
 * raspberrypi/linux rpi-6.12.y commit 5bd564aa under drivers/gpu/drm/vc4/,
 * "facts:" is driver/docs/rpi5-v3d71-facts.md):
 *  - The firmware can leave HDMI running: the Linux HDMI bind code tests
 *    whether the video control register has its enable bit set and then
 *    keeps the clocks on (rpi:vc4_hdmi.c:3391-3399; facts section 10; the
 *    bit VC4_HD_VID_CTL_ENABLE is bit 31, rpi:vc4_regs.h:1007).
 *  - When the firmware's own display driver is not the one in use, the DRM
 *    bind sends the firmware a "display done" notice to stop it
 *    (rpi:vc4_drv.c:397-400; facts section 10). That notice is a mailbox
 *    call, not part of this register plan: a caller that can send it does
 *    so first; this plan covers the registers.
 *  - Order: HDMI video off first (nothing consumes pixels), then the pixel
 *    valve off (nothing consumes the HVS FIFO), then the HVS channel off and
 *    its display list emptied (nothing produces). This consumer-first order
 *    is this design's choice; no Linux line prescribes it.
 *  - The HVS channel registers and the display-list base are those of
 *    hvs6_dlist.h, kept here as local copies, not included (rpi:vc4_regs.h:
 *    564 and 622 for generation C, :707 and :718 for D; CTRL0 enable bit 31
 *    at :568, Linux's own "FIFO enabled" test at rpi:vc4_hvs.c:739; list
 *    memory 0x4000 from the HVS base, rpi:vc4_regs.h:541).
 *  - Display-list words 0..31 hold the bootloader's setup and Linux never
 *    overwrites them (rpi:vc4_regs.h:1056, rpi:vc4_hvs.c:1579-1583; facts
 *    section 10). The firmware's list may sit there. Emptying it means
 *    writing the END word (bit 31 of control word 0, rpi:vc4_regs.h:1359)
 *    over the first word of the list, the word the channel's list-active
 *    field names.
 *
 * "unverified" marks a fact not confirmed for BCM2712: an offset or bit that
 * is the Pi 4 layout, or one not read (the HD VID_CTL offset). The
 * vc4_regs.h lines were checked against the file at rpi-6.12.y 5bd564aa;
 * the board's register dump must confirm the rest before route 3 relies on it.
 *
 * Idempotent: the plan for a snapshot taken after the plan was applied is
 * empty, and each write touches only a bit that is set in the snapshot.
 */
#ifndef YAKOGL_BOARD_RPI5_SCANOUT_FW_STOP_H
#define YAKOGL_BOARD_RPI5_SCANOUT_FW_STOP_H

#include <stddef.h>
#include <stdint.h>

/* Register blocks; offsets are bytes from the block's base. */
enum fw_stop_block {
	FW_STOP_HDMI_HD = 1, /* HDMI0 "hd" region (facts section 10) */
	FW_STOP_PV,          /* pixel valve (PV0 or PV1: the caller's base) */
	FW_STOP_HVS,         /* HVS registers and display-list memory */
};

/* VID_CTL offset in the hd region: unverified (Pi 4 layout, not read). */
#define FW_STOP_HD_VID_CTL 0x038u
/* ENABLE, bit 31: rpi:vc4_regs.h:1007 (facts section 10). */
#define FW_STOP_HD_VID_CTL_ENABLE (1u << 31)

/* PV_CONTROL offset 0 (rpi:vc4_regs.h:145), EN bit 0 (:169): the Pi 4 PV layout; that BCM2712 uses it is unverified. */
#define FW_STOP_PV_CONTROL 0x00u
#define FW_STOP_PV_CONTROL_EN (1u << 0)
/* PV_V_CONTROL offset 4 (rpi:vc4_regs.h:171), VIDEN bit 0 (:180): same caveat. */
#define FW_STOP_PV_V_CONTROL 0x04u
#define FW_STOP_PV_VCONTROL_VIDEN (1u << 0)

/* HVS numbers: see the header comment for the citations. */
#define FW_STOP_HVS_DLIST_OFFSET 0x4000u
#define FW_STOP_HVS_CHANNELS 3u
#define FW_STOP_HVS_CTRL0_ENB (1u << 31)
#define FW_STOP_HVS_DL_LACT_MASK 0xfffu
#define FW_STOP_HVS_CTL0_END (1u << 31)

enum fw_stop_hvs_gen {
	FW_STOP_HVS_6C = 1, /* VERSION 0x53 */
	FW_STOP_HVS_6D,     /* VERSION 0x54 */
};

enum fw_stop_err {
	FW_STOP_ERR_ARG = -1,    /* NULL snapshot or output */
	FW_STOP_ERR_GEN = -2,    /* hvs_gen not 6C or 6D */
	FW_STOP_ERR_CHAN = -3,   /* channel over 2 */
	FW_STOP_ERR_BUFFER = -4, /* output too short for the plan */
};

/* One register write: new = (old & ~mask) | (value & mask), then wait. */
struct fw_stop_write {
	enum fw_stop_block block;
	uint32_t offset;
	uint32_t value;
	uint32_t mask;
	uint32_t wait_us;
	const char *cite; /* facts or rpi: file:line, or "unverified" */
};

#define FW_STOP_MAX_WRITES 5u

/* The registers as read. Fields the plan does not need stay 0. */
struct fw_stop_snap {
	uint32_t hdmi_vid_ctl;       /* HDMI hd region, VID_CTL */
	uint32_t pv_control;         /* PV_CONTROL */
	uint32_t pv_vcontrol;        /* PV_V_CONTROL */
	enum fw_stop_hvs_gen hvs_gen;
	uint32_t hvs_chan;           /* the channel feeding the pixel valve, 0..2 */
	uint32_t hvs_ctrl0;          /* DISPx_CTRL0 of that channel */
	uint32_t hvs_dl;             /* DISPx_DL of that channel (LACT in 11:0) */
	uint32_t hvs_dl_word0;       /* display-list word at LACT, read by the caller */
};

/* One 60 Hz frame is 16.7 ms: wait a frame and a bit after a stop. A design
 * number, not a Linux one. */
#define FW_STOP_WAIT_FRAME_US 20000u

static inline int fw_stop_hvs_running(const struct fw_stop_snap *s)
{
	return (s->hvs_ctrl0 & FW_STOP_HVS_CTRL0_ENB) != 0;
}

/* Does anything run? A pure judgement from the snapshot. */
static inline int fw_stop_is_needed(const struct fw_stop_snap *s)
{
	if (!s)
		return 0;
	return (s->hdmi_vid_ctl & FW_STOP_HD_VID_CTL_ENABLE) != 0 ||
	       (s->pv_control & FW_STOP_PV_CONTROL_EN) != 0 ||
	       (s->pv_vcontrol & FW_STOP_PV_VCONTROL_VIDEN) != 0 || fw_stop_hvs_running(s);
}

/* HVS CTRL0 offset of a channel (hvs6_dlist.h numbers). */
static inline int fw_stop_hvs_ctrl0_off(enum fw_stop_hvs_gen g, uint32_t ch, uint32_t *off)
{
	if (g != FW_STOP_HVS_6C && g != FW_STOP_HVS_6D)
		return FW_STOP_ERR_GEN;
	if (ch >= FW_STOP_HVS_CHANNELS)
		return FW_STOP_ERR_CHAN;
	if (g == FW_STOP_HVS_6C)
		*off = 0x0030u + ch * 0x0020u; /* rpi:vc4_regs.h:564,622 */
	else
		*off = 0x0100u + ch * 0x0040u; /* rpi:vc4_regs.h:707,718 */
	return 0;
}

/*
 * Fill out[0..n) with the writes that stop the firmware's scanout, in
 * order; return n (0 when nothing runs) or a negative enum fw_stop_err.
 * Each write is made only when its bit is set in the snapshot:
 *   1 HDMI hd VID_CTL: clear ENABLE           (facts s10; rpi:vc4_hdmi.c:3391-3399)
 *   2 PV V_CONTROL: clear VIDEN               (rpi:vc4_regs.h:171,180; BCM2712 use unverified)
 *   3 PV CONTROL: clear EN                    (rpi:vc4_regs.h:145,169; BCM2712 use unverified)
 *   4 HVS CTRL0 of the channel: clear ENB     (rpi:vc4_regs.h:568, rpi:vc4_hvs.c:739)
 *   5 HVS list memory at the list-active word: the END word
 *                                             (rpi:vc4_regs.h:1359, rpi:vc4_hvs.c:1579-1583)
 * The END write is skipped when the word already is END. out needs room for
 * FW_STOP_MAX_WRITES when anything runs.
 */
static inline int fw_stop_plan(const struct fw_stop_snap *s, struct fw_stop_write *out,
			       size_t cap)
{
	uint32_t ctrl0_off = 0;
	size_t n = 0;
	int r;

	if (!s || !out)
		return FW_STOP_ERR_ARG;
	r = fw_stop_hvs_ctrl0_off(s->hvs_gen, s->hvs_chan, &ctrl0_off);
	if (r < 0)
		return r;
	if (!fw_stop_is_needed(s))
		return 0;
	if (cap < FW_STOP_MAX_WRITES)
		return FW_STOP_ERR_BUFFER;

	if (s->hdmi_vid_ctl & FW_STOP_HD_VID_CTL_ENABLE)
		out[n++] = (struct fw_stop_write){FW_STOP_HDMI_HD, FW_STOP_HD_VID_CTL, 0,
						  FW_STOP_HD_VID_CTL_ENABLE, FW_STOP_WAIT_FRAME_US,
						  "facts s10; rpi:vc4_hdmi.c:3391-3399; offset unverified"};
	if (s->pv_vcontrol & FW_STOP_PV_VCONTROL_VIDEN)
		out[n++] = (struct fw_stop_write){FW_STOP_PV, FW_STOP_PV_V_CONTROL, 0,
						  FW_STOP_PV_VCONTROL_VIDEN, 0, "rpi:vc4_regs.h:171,180; BCM2712 use unverified"};
	if (s->pv_control & FW_STOP_PV_CONTROL_EN)
		out[n++] = (struct fw_stop_write){FW_STOP_PV, FW_STOP_PV_CONTROL, 0,
						  FW_STOP_PV_CONTROL_EN, FW_STOP_WAIT_FRAME_US,
						  "rpi:vc4_regs.h:145,169; BCM2712 use unverified"};
	if (fw_stop_hvs_running(s)) {
		out[n++] = (struct fw_stop_write){FW_STOP_HVS, ctrl0_off, 0, FW_STOP_HVS_CTRL0_ENB,
						  FW_STOP_WAIT_FRAME_US,
						  "rpi:vc4_regs.h:568; rpi:vc4_hvs.c:739"};
		if (!(s->hvs_dl_word0 & FW_STOP_HVS_CTL0_END))
			out[n++] = (struct fw_stop_write){
				FW_STOP_HVS,
				FW_STOP_HVS_DLIST_OFFSET + 4u * (s->hvs_dl & FW_STOP_HVS_DL_LACT_MASK),
				FW_STOP_HVS_CTL0_END, 0xffffffffu, 0,
				"rpi:vc4_regs.h:1359,541; rpi:vc4_hvs.c:1579-1583"};
	}
	return (int)n;
}

#endif /* YAKOGL_BOARD_RPI5_SCANOUT_FW_STOP_H */
