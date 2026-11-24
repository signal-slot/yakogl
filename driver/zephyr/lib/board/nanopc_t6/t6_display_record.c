/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The records of the display on the NanoPC-T6, printed (nanopc_t6.h): of an
 * adopt (t6_display_print_adopt), of the chain from cold
 * (t6_display_print_cold) and of the read-backs over a picture that is up
 * (t6_display_read_back). The same lines for the t6_display and t6_cold
 * probes and for the board layer (its first board_show(), and the read-backs
 * at the end of a run), each under its own tag.
 * driver/docs/nanopc-t6-display.md, section 4, says what the adopt's fields
 * mean, section 5 what the read-backs prove.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

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

static const char *const cold_stage_name[] = {
	[0] = "none",
	[YD_T6_DISPLAY_COLD_STAGE_START] = "START",
	[YD_T6_DISPLAY_COLD_STAGE_POWER] = "POWER",
	[YD_T6_DISPLAY_COLD_STAGE_GRF] = "GRF",
	[YD_T6_DISPLAY_COLD_STAGE_PHY] = "PHY",
	[YD_T6_DISPLAY_COLD_STAGE_DCLK] = "DCLK",
	[YD_T6_DISPLAY_COLD_STAGE_CONTROLLER] = "CONTROLLER",
	[YD_T6_DISPLAY_COLD_STAGE_VOP] = "VOP",
	[YD_T6_DISPLAY_COLD_STAGE_VIDEO] = "VIDEO",
};

static const char *const cold_error_name[] = {
	[YD_T6_DISPLAY_COLD_ERROR_NONE] = "none",
	[YD_T6_DISPLAY_COLD_ERROR_STOPPED] = "stopped as asked",
	[YD_T6_DISPLAY_COLD_ERROR_MODE] = "the mode does not fit",
	[YD_T6_DISPLAY_COLD_ERROR_PHY] = "the PHY",
	[YD_T6_DISPLAY_COLD_ERROR_VOP] = "VOP2 refused",
	[YD_T6_DISPLAY_COLD_ERROR_FOUND_RUNNING] = "a port runs (force-cold not set)",
	[YD_T6_DISPLAY_COLD_ERROR_STANDBY] = "a running port did not stop",
	[YD_T6_DISPLAY_COLD_ERROR_DISPLAY] = "a clock, domain or gate",
	[YD_T6_DISPLAY_COLD_ERROR_GRF] = "a GRF word did not take",
	[YD_T6_DISPLAY_COLD_ERROR_GATE] = "a gate stays closed",
	[YD_T6_DISPLAY_COLD_ERROR_DCLK_MUX] = "DCLK_VOP0's mux did not take",
	[YD_T6_DISPLAY_COLD_ERROR_HDMI] = "the HDMI controller refused",
	[YD_T6_DISPLAY_COLD_ERROR_IOMMU] = "a vop_mmu set keeps paging",
	[YD_T6_DISPLAY_COLD_ERROR_VIDEO] = "a video check failed",
	[YD_T6_DISPLAY_COLD_ERROR_VOP_UNREADABLE] = "VOP2 is powered but cannot be read",
	[YD_T6_DISPLAY_COLD_ERROR_UNCLOCKED] = "a port has no pixel clock",
};

static const char *name_of(const char *const *names, size_t n, uint32_t v)
{
	return v < n && names[v] != NULL ? names[v] : "unknown";
}

const char *t6_display_cold_stage_name(uint32_t stage)
{
	return name_of(cold_stage_name, ARRAY_SIZE(cold_stage_name), stage);
}

void t6_display_print_cold(const char *tag, int rc, long long ms,
			   const struct yd_t6_display_cold *c)
{
	printk("%s: chain rc %d in %lld ms: stage %u (%s), error %u (%s), refusal %u, index %u, "
	       "detail 0x%08x\n",
	       tag, rc, ms, c->stage, t6_display_cold_stage_name(c->stage), c->error,
	       name_of(cold_error_name, ARRAY_SIZE(cold_error_name), c->error), c->refusal,
	       c->index, c->detail);
}

void t6_delay_us(uint32_t us)
{
	k_busy_wait(us);
}

uint64_t t6_now_us(void)
{
	return k_cyc_to_us_floor64(k_cycle_get_64());
}

int t6_display_read_back(const char *tag, const struct yd_t6_display_bases *b,
			 struct yd_t6_display_status *s)
{
	const uint32_t *v = s->vop;
	int rc;

	rc = yd_t6_display_status(b, T6_DISPLAY_CHECK_WAIT_US, T6_DISPLAY_TIMED_FRAMES, t6_now_us,
				  t6_delay_us, s);
	t6_watchdog_feed();
	printk("%s: frame check over %u ms: frame start %u, underflow %u, bus error %u %u, "
	       "VP0 INT_RAW_STATUS 0x%08x\n",
	       tag, T6_DISPLAY_CHECK_WAIT_US / 1000, s->frame_start, s->underflow,
	       s->bus_error[0], s->bus_error[1], s->raw);
	if (s->frames != 0 && s->frames_us != 0) {
		/* htotal and vtotal of the mode VP0 runs: the upper halves of
		 * DSP_HTOTAL_HS_END and DSP_VTOTAL_VS_END as read back */
		uint64_t htotal = v[5] >> 16, vtotal = v[6] >> 16;
		uint64_t mhz = (uint64_t)s->frames * 1000000000ULL / s->frames_us;
		uint64_t khz = htotal * vtotal * s->frames * 1000ULL / s->frames_us;

		printk("%s: %u frames in %u us: %u mHz, pixel clock %u kHz\n", tag, s->frames,
		       s->frames_us, (uint32_t)mhz, (uint32_t)khz);
	} else {
		printk("%s: no frame timed (refusal %u, detail 0x%08x)\n", tag, s->refusal,
		       s->detail);
	}
	printk("%s: VOP VERSION_INFO 0x%08x SYS_PD_CTRL 0x%08x REG_CFG_DONE 0x%08x "
	       "DSP_IF_EN 0x%08x DSP_CTRL 0x%08x HTOTAL_HS_END 0x%08x VTOTAL_VS_END 0x%08x\n",
	       tag, v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
	printk("%s: VOP OVL_PORT_SEL 0x%08x OVL_LAYER_SEL 0x%08x Esmart0 REGION0_CTRL "
	       "0x%08x YRGB_MST 0x%08x VIR 0x%08x INT_RAW_STATUS 0x%08x SYS0 0x%08x SYS1 0x%08x\n",
	       tag, v[7], v[8], v[9], v[10], v[11], v[12], v[13], v[14]);
	printk("%s: HPD %u (SOC_STATUS1 0x%08x), PHY0 GRF STATUS 0x%08x\n", tag,
	       (s->soc_status1 >> 16) & 1, s->soc_status1, s->phy_status);
	if (s->vo1_usable) {
		printk("%s: HDMI0 CMU_STATUS 0x%08x VIDQPCLK_FREQ %u LINKQPCLK_FREQ %u\n", tag,
		       s->cmu[0], s->cmu[1], s->cmu[2]);
		printk("%s: HDMI0 VIDEO_MONITOR_STATUS0-6 0x%08x 0x%08x 0x%08x 0x%08x "
		       "0x%08x 0x%08x 0x%08x\n",
		       tag, s->monitor[0], s->monitor[1], s->monitor[2], s->monitor[3],
		       s->monitor[4], s->monitor[5], s->monitor[6]);
	} else {
		printk("%s: HDMI0 not read: PD_VO1 not usable or PCLK_HDMITX0 gated "
		       "(CLKGATE_CON(60) 0x%08x)\n",
		       tag, s->cru_gate60);
	}
	return rc;
}
