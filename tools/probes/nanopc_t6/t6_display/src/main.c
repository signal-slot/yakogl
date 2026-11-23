/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_display: step D2 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
 * section 2 route (a), section 5). Started by a crash kexec, so Linux's
 * display chain still runs: VP0 scans 1024x768 or 1920x1080 at 60 Hz from
 * Cluster0 through the VOP's IOMMU. Before main() the board glue has armed
 * WDT0 (nanopc_t6.c). This draws the section 5 pattern into a buffer of this
 * image (red, green / blue, yellow quadrants, a white 1-pixel border),
 * cleans it to the point of coherency, then yd_t6_display_adopt
 * (board::nanopc_t6_display in the drivers crate): what Linux left is read
 * and checked, every window but Esmart0 turned off, the IOMMU's paging
 * turned off, Esmart0 made VP0's only layer on the buffer. It prints what
 * it found and did, holds the picture for T6_PICTURE_HOLD_S seconds with
 * the watchdog fed, prints the section 5 read-backs (yd_t6_display_status)
 * and PASS or FAIL, and resets back into Armbian. The pattern, the hold and
 * the read-backs are common/t6_picture.c's, shared with t6_cold (step D8).
 * Nothing here touches the GPU; the GIC has every SPI disabled by Zephyr's
 * init, so the VOP's and the HDMI controller's interrupt lines Linux left
 * enabled reach no CPU.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <errno.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#include "t6_picture.h"

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

static void print_adopt(const struct yd_t6_display_adopt *a)
{
	printk("t6_display: found PD_VOP PWR 0x%08x STATUS 0x%08x IDLE_REQ 0x%08x IDLE_ACK 0x%08x "
	       "IDLE_ST 0x%08x REPAIR 0x%08x CLKGATE_CON(52) 0x%08x\n",
	       a->pd_vop[0], a->pd_vop[1], a->pd_vop[2], a->pd_vop[3], a->pd_vop[4], a->pd_vop[5],
	       a->cru_gate52);
	printk("t6_display: found VERSION_INFO 0x%08x DSP_CTRL 0x%08x 0x%08x 0x%08x 0x%08x "
	       "DSP_IF_EN 0x%08x SYS_PD_CTRL 0x%08x\n",
	       a->version, a->dsp_ctrl[0], a->dsp_ctrl[1], a->dsp_ctrl[2], a->dsp_ctrl[3],
	       a->dsp_if_en, a->pd_ctrl);
	printk("t6_display: found VP0 timing 0x%08x 0x%08x 0x%08x 0x%08x OVL_LAYER_SEL 0x%08x "
	       "OVL_PORT_SEL 0x%08x\n",
	       a->timing[0], a->timing[1], a->timing[2], a->timing[3], a->layer_sel, a->port_sel);
	printk("t6_display: found Cluster0 0x%08x 0x%08x Esmart0 0x%08x vop_mmu STATUS 0x%08x "
	       "0x%08x DTE_ADDR 0x%08x 0x%08x\n",
	       a->cluster0[0], a->cluster0[1], a->esmart0_ctrl, a->mmu_status[0], a->mmu_status[1],
	       a->mmu_dte[0], a->mmu_dte[1]);
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop >= YD_T6_DISPLAY_STOP_TAKE_OVER) {
		printk("t6_display: take-over: running ports 0x%x, windows off 0x%x\n", a->running,
		       a->off);
	}
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop >= YD_T6_DISPLAY_STOP_IOMMU) {
		printk("t6_display: vop_mmu: STATUS 0x%08x 0x%08x after paging off, stall time-out "
		       "%u\n",
		       a->mmu_status_now[0], a->mmu_status_now[1], a->mmu_timeout);
	}
	if (ran(a->stop, YD_T6_DISPLAY_STOP_OVERLAY)) {
		printk("t6_display: overlay: OVL_LAYER_SEL 0x%08x OVL_PORT_SEL 0x%08x written, in "
		       "force after %u us\n",
		       a->ovl_layer_sel, a->ovl_port_sel, a->ovl_waited_us);
	}
	if (ran(a->stop, YD_T6_DISPLAY_STOP_FRAME)) {
		printk("t6_display: window: Esmart0 on 0x%08x stride %u; REG_CFG_DONE 0x%08x after "
		       "the write, 0x%08x after the frame start (%u us)\n",
		       a->fb, a->stride, a->cfg_done_written, a->cfg_done_after, a->frame_waited_us);
	}
	if (a->stop == YD_T6_DISPLAY_STOP_NONE || a->stop == YD_T6_DISPLAY_STOP_SHOWN) {
		printk("t6_display: shown: YRGB_MST 0x%08x REGION0_CTRL 0x%08x OVL_PORT_SEL 0x%08x "
		       "OVL_LAYER_SEL 0x%08x\n",
		       a->shown[0], a->shown[1], a->shown[2], a->shown[3]);
	}
}

int main(void)
{
	struct yd_t6_display_bases b;
	struct yd_t6_display_adopt a;
	struct yd_t6_display_status s;
	uint64_t fb_pa;
	int rc;

	printk("t6_display: build %s\n", T6_DISPLAY_BUILD);
	printk("t6_display: WDT0 period %u ms, fed until %u ms of uptime\n",
	       t6_watchdog_period_ms(), t6_watchdog_deadline_ms());

	fb_pa = t6_picture_prepare("t6_display");
	if (fb_pa == 0) {
		t6_reset_to_linux("t6_display done");
	}
	/* The adopt reads CLKGATE_CON(52) and (60) and the controller's CMU and
	 * video monitor words only: a page of each. */
	t6_picture_map(&b, 0x1000, 0x1000);
	t6_watchdog_feed();

	rc = yd_t6_display_adopt(&b, fb_pa, t6_picture_delay_us, &a);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_display: FAIL: yd_t6_display_adopt refused its arguments\n");
		t6_reset_to_linux("t6_display done");
	}
	print_adopt(&a);
	printk("t6_display: adopt rc %d stop %u (%s), refusal %u, index %u, detail 0x%08x\n", rc,
	       a.stop, a.stop < ARRAY_SIZE(stop_name) ? stop_name[a.stop] : "unknown", a.refusal,
	       a.index, a.detail);
	if (rc != 0) {
		printk("t6_display: FAIL: adopt stop %u\n", a.stop);
		t6_reset_to_linux("t6_display done");
	}

	t6_picture_hold("t6_display");
	rc = t6_picture_status("t6_display", &b, &s);
	t6_picture_verdict("t6_display", rc, &s, a.fb);
	t6_reset_to_linux("t6_display done");
}
