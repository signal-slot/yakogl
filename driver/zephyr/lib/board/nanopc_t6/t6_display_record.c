/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The record of a display adopt on the NanoPC-T6, printed
 * (t6_display_print_adopt, nanopc_t6.h): the same lines for the t6_display
 * probe and for the board layer's first board_show(), each under its own tag.
 * driver/docs/nanopc-t6-display.md, section 4, says what the fields mean.
 */
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <stdbool.h>

#include <yakogl_drivers.h>

#include "nanopc_t6.h"

static const char *const stop_name[] = {
	[YD_T6_DISPLAY_STOP_NONE] = "none",
	[YD_T6_DISPLAY_STOP_VOP_DOMAIN] = "PD_VOP is not powered or not out of idle",
	[YD_T6_DISPLAY_STOP_VOP_CLOCK] = "a clock of PD_VOP or HCLK_VOP is gated",
	[YD_T6_DISPLAY_STOP_VERSION] = "VERSION_INFO is not the RK3588's",
	[YD_T6_DISPLAY_STOP_STANDBY] = "VP0 is in STANDBY: no picture to adopt",
	[YD_T6_DISPLAY_STOP_VP_STATE] = "VP0's DSP_CTRL is not HDMI RGB or its background not black",
	[YD_T6_DISPLAY_STOP_MODE] = "VP0's timing is neither 1024x768@60 nor 1920x1080@60",
	[YD_T6_DISPLAY_STOP_ROUTE] = "HDMI0 is not fed by VP0",
	[YD_T6_DISPLAY_STOP_ESMART_POWER] = "the Esmart windows' power is off",
	[YD_T6_DISPLAY_STOP_TAKE_OVER] = "the take-over of Linux's windows stopped",
	[YD_T6_DISPLAY_STOP_IOMMU] = "a vop_mmu set keeps paging",
	[YD_T6_DISPLAY_STOP_OVERLAY] = "the overlay select stopped",
	[YD_T6_DISPLAY_STOP_WINDOW] = "the buffer was refused",
	[YD_T6_DISPLAY_STOP_FRAME] = "no frame start after the window",
	[YD_T6_DISPLAY_STOP_SHOWN] = "a register reads other than written",
};

/* The step `step` ran (a stop at a later step, or none). */
static bool ran(uint32_t stop, uint32_t step)
{
	return stop == YD_T6_DISPLAY_STOP_NONE || stop > step;
}

void t6_display_print_adopt(const char *tag, int rc, const struct yd_t6_display_adopt *a)
{
	printk("%s: found PD_VOP PWR 0x%08x STATUS 0x%08x IDLE_REQ 0x%08x IDLE_ACK 0x%08x "
	       "IDLE_ST 0x%08x REPAIR 0x%08x CLKGATE_CON(52) 0x%08x\n",
	       tag, a->pd_vop[0], a->pd_vop[1], a->pd_vop[2], a->pd_vop[3], a->pd_vop[4],
	       a->pd_vop[5], a->cru_gate52);
	printk("%s: found VERSION_INFO 0x%08x DSP_CTRL 0x%08x 0x%08x 0x%08x 0x%08x "
	       "DSP_IF_EN 0x%08x SYS_PD_CTRL 0x%08x\n",
	       tag, a->version, a->dsp_ctrl[0], a->dsp_ctrl[1], a->dsp_ctrl[2], a->dsp_ctrl[3],
	       a->dsp_if_en, a->pd_ctrl);
	printk("%s: found VP0 timing 0x%08x 0x%08x 0x%08x 0x%08x OVL_LAYER_SEL 0x%08x "
	       "OVL_PORT_SEL 0x%08x\n",
	       tag, a->timing[0], a->timing[1], a->timing[2], a->timing[3], a->layer_sel,
	       a->port_sel);
	printk("%s: found Cluster0 0x%08x 0x%08x Esmart0 0x%08x vop_mmu STATUS 0x%08x "
	       "0x%08x DTE_ADDR 0x%08x 0x%08x\n",
	       tag, a->cluster0[0], a->cluster0[1], a->esmart0_ctrl, a->mmu_status[0],
	       a->mmu_status[1], a->mmu_dte[0], a->mmu_dte[1]);
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop >= YD_T6_DISPLAY_STOP_TAKE_OVER) {
		printk("%s: take-over: running ports 0x%x, windows off 0x%x\n", tag, a->running,
		       a->off);
	}
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop >= YD_T6_DISPLAY_STOP_IOMMU) {
		printk("%s: vop_mmu: STATUS 0x%08x 0x%08x after paging off, stall time-out "
		       "%u\n",
		       tag, a->mmu_status_now[0], a->mmu_status_now[1], a->mmu_timeout);
	}
	if (ran(a->stop, YD_T6_DISPLAY_STOP_OVERLAY)) {
		printk("%s: overlay: OVL_LAYER_SEL 0x%08x OVL_PORT_SEL 0x%08x written, in "
		       "force after %u us\n",
		       tag, a->ovl_layer_sel, a->ovl_port_sel, a->ovl_waited_us);
	}
	if (ran(a->stop, YD_T6_DISPLAY_STOP_FRAME)) {
		printk("%s: window: Esmart0 on 0x%08x stride %u; REG_CFG_DONE 0x%08x after "
		       "the write, 0x%08x after the frame start (%u us)\n",
		       tag, a->fb, a->stride, a->cfg_done_written, a->cfg_done_after,
		       a->frame_waited_us);
	}
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop == YD_T6_DISPLAY_STOP_SHOWN) {
		printk("%s: shown: YRGB_MST 0x%08x REGION0_CTRL 0x%08x OVL_PORT_SEL 0x%08x "
		       "OVL_LAYER_SEL 0x%08x\n",
		       tag, a->shown[0], a->shown[1], a->shown[2], a->shown[3]);
	}
	printk("%s: adopt rc %d stop %u (%s), refusal %u, index %u, detail 0x%08x\n", tag, rc,
	       a->stop, a->stop < ARRAY_SIZE(stop_name) ? stop_name[a->stop] : "unknown",
	       a->refusal, a->index, a->detail);
}
