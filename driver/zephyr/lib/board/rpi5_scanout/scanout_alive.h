/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * "Is a scanout running?" from two raw register samples (BCM2712, HVS6 channel
 * plus pixel valve), for the U3 probe that has to decide, when a kernel= image
 * starts, whether the firmware's scanout can be adopted or must be stopped.
 *
 * No MMIO happens here. The caller reads the registers named below twice, some
 * milliseconds apart (one frame is 16.7 ms at 60 Hz; 20 ms or more), fills two
 * struct sa_sample and calls sa_judge(). sa_reg_*() give the offsets to read
 * (from the HVS base; the pixel valve ones from the PV base).
 *
 * Running needs: the channel enabled (CTRL0.ENB set, STATUS.MODE not
 * DISABLED), the pixel valve enabled (PV_CONTROL.EN and PV_V_CONTROL.VIDEN)
 * and the channel's frame count (STATUS.FRCNT, 6 bits, wraps) or line
 * (STATUS.YLINE) different between the samples. A read of all ones means the
 * block is unclocked or unpowered: stopped, reason "unclocked". The checks run
 * in the order of enum sa_reason; the first that fails names the reason.
 * The HDMI0 status is not used: HD VID_CTL only says the encoder is on, which
 * the pixel valve and the counter already imply.
 *
 * Offsets, bit positions and order are from the Raspberry Pi Linux kernel,
 * branch rpi-6.12.y, drivers/gpu/drm/vc4/, as "rpi:<file>:<line>". The
 * pixel valve registers are the Pi 4 layout; that the BCM2712 pixel valve
 * uses it is unverified (same caveat as fw_stop.h).
 *
 * Two HVS steppings exist: C0 (VERSION 0x53) and D0 (0x54), different
 * channel strides. C99, header-only, no Zephyr includes.
 */
#ifndef YD5_SCANOUT_ALIVE_H
#define YD5_SCANOUT_ALIVE_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

/* --- stepping (rpi:vc4_regs.h:543-546) ----------------------------------- */

#define SA_HVS_VERSION		0x00000000u	/* rpi:vc4_regs.h:543 */
#define SA_HVS_VERSION_MASK	0x000000ffu	/* bits 7:0, rpi:vc4_regs.h:543 */
#define SA_HVS_VERSION_C0	0x53u		/* rpi:vc4_regs.h:545 */
#define SA_HVS_VERSION_D0	0x54u		/* rpi:vc4_regs.h:546 */

#define SA_NUM_CHANNELS		3u		/* rpi:vc4_regs.h:630-637 (DISP0..2) */

/* --- channel registers: channel 0 offset and stride per stepping --------- */

/* C0, stride 0x20 (rpi:vc4_regs.h:564,622) */
#define SA_C0_CTRL0		0x00000030u	/* rpi:vc4_regs.h:564 */
#define SA_C0_STATUS		0x00000044u	/* rpi:vc4_regs.h:600 */
#define SA_C0_DL		0x00000048u	/* rpi:vc4_regs.h:614 */
#define SA_C0_STRIDE		0x00000020u	/* rpi:vc4_regs.h:622 */
/* D0, stride 0x40 (rpi:vc4_regs.h:701,718) */
#define SA_D0_CTRL0		0x00000100u	/* rpi:vc4_regs.h:701 */
#define SA_D0_STATUS		0x00000118u	/* rpi:vc4_regs.h:706 */
#define SA_D0_DL		0x0000011cu	/* rpi:vc4_regs.h:714 */
#define SA_D0_STRIDE		0x00000040u	/* rpi:vc4_regs.h:718 */

/* DISPx_CTRL0.ENB, bit 31 (rpi:vc4_regs.h:568; both steppings) */
#define SA_CTRL0_ENB		(1u << 31)
/* DISPx_STATUS: FRCNT 21:16 (rpi:vc4_regs.h:605), MODE 14:13 (:607) with
 * DISABLED 0 (:608), YLINE 12:0 (:612) */
#define SA_STATUS_FRCNT_SHIFT	16u
#define SA_STATUS_FRCNT_MASK	0x3fu
#define SA_STATUS_MODE_SHIFT	13u
#define SA_STATUS_MODE_MASK	0x3u
#define SA_STATUS_MODE_DISABLED	0u
#define SA_STATUS_YLINE_MASK	0x1fffu
/* DISPx_DL.LACT 11:0, the word index of the list being scanned (rpi:vc4_regs.h:619) */
#define SA_DL_LACT_MASK		0x00000fffu
/* display-list memory: HVS offset 0x4000, 32-bit words (rpi:vc4_regs.h:541) */
#define SA_DLIST_MEM_OFF	0x00004000u

/* --- pixel valve (offsets from its base) --------------------------------- */

#define SA_PV_CONTROL		0x00u		/* rpi:vc4_regs.h:145 */
#define SA_PV_CONTROL_EN	(1u << 0)	/* rpi:vc4_regs.h:169 */
#define SA_PV_V_CONTROL		0x04u		/* rpi:vc4_regs.h:171 */
#define SA_PV_VCONTROL_VIDEN	(1u << 0)	/* rpi:vc4_regs.h:180 */

/* --- samples and verdict -------------------------------------------------- */

struct sa_sample {
	uint32_t hvs_version;	/* HVS VERSION */
	uint32_t ctrl0;		/* DISPx_CTRL0 */
	uint32_t status;	/* DISPx_STATUS */
	uint32_t dl;		/* DISPx_DL */
	uint32_t pv_control;	/* PV_CONTROL */
	uint32_t pv_vcontrol;	/* PV_V_CONTROL */
};

/* The order of the checks. SA_OK means running. */
enum sa_reason {
	SA_OK = 0,
	SA_BAD_INPUT,		/* channel out of range */
	SA_UNKNOWN_STEPPING,	/* VERSION is neither C0 nor D0, or differs between samples */
	SA_UNCLOCKED,		/* a register read all ones */
	SA_CHANNEL_OFF,
	SA_PV_OFF,
	SA_COUNTER_FROZEN
};

struct sa_verdict {
	int running;			/* 1 running, 0 stopped */
	enum sa_reason reason;		/* SA_OK when running */
	unsigned int channel;
	int d0;				/* 1 stepping D0, 0 C0 (valid past SA_UNKNOWN_STEPPING) */
	uint32_t dl_head_words;		/* DISPx_DL.LACT of the second sample */
	uint32_t dl_head_off;		/* the same as a byte offset from the HVS base */
	uint32_t frcnt_a, frcnt_b;	/* STATUS.FRCNT of the two samples */
	uint32_t yline_a, yline_b;	/* STATUS.YLINE of the two samples */
};

static inline int sa_is_d0(uint32_t version_reg)
{
	return (version_reg & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_D0;
}

/* Offsets from the HVS base; stepping from the VERSION read. */
static inline uint32_t sa_reg_ctrl0(int d0, unsigned int ch)
{
	return d0 ? SA_D0_CTRL0 + ch * SA_D0_STRIDE : SA_C0_CTRL0 + ch * SA_C0_STRIDE;
}

static inline uint32_t sa_reg_status(int d0, unsigned int ch)
{
	return d0 ? SA_D0_STATUS + ch * SA_D0_STRIDE : SA_C0_STATUS + ch * SA_C0_STRIDE;
}

static inline uint32_t sa_reg_dl(int d0, unsigned int ch)
{
	return d0 ? SA_D0_DL + ch * SA_D0_STRIDE : SA_C0_DL + ch * SA_C0_STRIDE;
}

static inline int sa_all_ones(const struct sa_sample *s)
{
	return s->ctrl0 == 0xffffffffu || s->status == 0xffffffffu ||
	       s->dl == 0xffffffffu || s->pv_control == 0xffffffffu ||
	       s->pv_vcontrol == 0xffffffffu;
}

static inline int sa_channel_on(const struct sa_sample *s)
{
	return (s->ctrl0 & SA_CTRL0_ENB) != 0 &&
	       ((s->status >> SA_STATUS_MODE_SHIFT) & SA_STATUS_MODE_MASK) !=
		       SA_STATUS_MODE_DISABLED;
}

static inline int sa_pv_on(const struct sa_sample *s)
{
	return (s->pv_control & SA_PV_CONTROL_EN) != 0 &&
	       (s->pv_vcontrol & SA_PV_VCONTROL_VIDEN) != 0;
}

/* a is the earlier sample, b the later one, both of channel `channel`. */
static inline struct sa_verdict sa_judge(const struct sa_sample *a,
					 const struct sa_sample *b,
					 unsigned int channel)
{
	struct sa_verdict v = {0};
	uint32_t va = a->hvs_version & SA_HVS_VERSION_MASK;
	uint32_t vb = b->hvs_version & SA_HVS_VERSION_MASK;

	v.channel = channel;
	v.frcnt_a = (a->status >> SA_STATUS_FRCNT_SHIFT) & SA_STATUS_FRCNT_MASK;
	v.frcnt_b = (b->status >> SA_STATUS_FRCNT_SHIFT) & SA_STATUS_FRCNT_MASK;
	v.yline_a = a->status & SA_STATUS_YLINE_MASK;
	v.yline_b = b->status & SA_STATUS_YLINE_MASK;
	v.dl_head_words = b->dl & SA_DL_LACT_MASK;
	v.dl_head_off = SA_DLIST_MEM_OFF + 4u * v.dl_head_words;

	if (channel >= SA_NUM_CHANNELS) {
		v.reason = SA_BAD_INPUT;
		return v;
	}
	if (sa_all_ones(a) || sa_all_ones(b)) {
		v.reason = SA_UNCLOCKED;
		return v;
	}
	if (va != vb || (vb != SA_HVS_VERSION_C0 && vb != SA_HVS_VERSION_D0)) {
		v.reason = SA_UNKNOWN_STEPPING;
		return v;
	}
	v.d0 = vb == SA_HVS_VERSION_D0;

	if (!sa_channel_on(b))
		v.reason = SA_CHANNEL_OFF;
	else if (!sa_pv_on(b))
		v.reason = SA_PV_OFF;
	else if (v.frcnt_a == v.frcnt_b && v.yline_a == v.yline_b)
		v.reason = SA_COUNTER_FROZEN;
	else
		v.running = 1;
	return v;
}

static inline const char *sa_reason_text(enum sa_reason r)
{
	switch (r) {
	case SA_OK:
		return "";
	case SA_BAD_INPUT:
		return "bad input";
	case SA_UNKNOWN_STEPPING:
		return "unknown stepping";
	case SA_UNCLOCKED:
		return "unclocked";
	case SA_CHANNEL_OFF:
		return "channel off";
	case SA_PV_OFF:
		return "pixel valve off";
	case SA_COUNTER_FROZEN:
		return "counter frozen";
	}
	return "bad input";
}

/*
 * The verdict line of tools/probes/rpi5_display/verdicts.sh, without newline:
 * "u3 scanout: running" or "u3 scanout: stopped <reason>". Returns the length
 * (snprintf's rule: n or more means truncated).
 */
static inline int sa_format(const struct sa_verdict *v, char *buf, size_t n)
{
	if (v->running)
		return snprintf(buf, n, "u3 scanout: running");
	return snprintf(buf, n, "u3 scanout: stopped %s", sa_reason_text(v->reason));
}

#endif /* YD5_SCANOUT_ALIVE_H */
