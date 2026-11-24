/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The CPU picture of the NanoPC-T6 display probes (t6_picture.h), with the
 * probe's name as the prefix of each line. driver/tools/kexec-zephyr/
 * t6-verdict.sh judges the lines of both probes with the same templates.
 */
#include "t6_picture.h"

#include <zephyr/cache.h>
#include <zephyr/kernel.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <nanopc_t6.h>

/* The hold's rules, which t6-verdict.sh checks on the record as well: a
 * "held" line every 10 s up to the hold, at least the 30 s of the default,
 * and the run ends well inside the watchdog's lifetime (a longer hold is
 * built with CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S raised with it). */
BUILD_ASSERT(T6_PICTURE_HOLD_S % 10 == 0 && T6_PICTURE_HOLD_S >= 30,
	     "T6_PICTURE_HOLD_S: a multiple of 10, at least 30");
BUILD_ASSERT(T6_PICTURE_HOLD_S + 10 < CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
	     "the run ends well inside the watchdog's lifetime");

static uint32_t t6_picture_fb[T6_PICTURE_WIDTH * T6_PICTURE_HEIGHT] __aligned(4096);

uint64_t t6_picture_prepare(const char *probe)
{
	int prc, crc;
	uint64_t pa;

	prc = yd_t6_display_pattern(t6_picture_fb, T6_PICTURE_WIDTH, T6_PICTURE_HEIGHT,
				    T6_PICTURE_STRIDE);
	/* Zephyr's "flush" is a clean to the point of coherency (dc cvac). */
	crc = sys_cache_data_flush_range(t6_picture_fb, sizeof(t6_picture_fb));
	pa = k_mem_phys_addr(t6_picture_fb);
	printk("%s: frame buffer %p at PA 0x%llx, %ux%u XRGB8888 stride %u, pattern rc %d, "
	       "clean rc %d\n",
	       probe, (void *)t6_picture_fb, (unsigned long long)pa, T6_PICTURE_WIDTH,
	       T6_PICTURE_HEIGHT, T6_PICTURE_STRIDE, prc, crc);
	if (prc != 0 || crc != 0) {
		printk("%s: FAIL: the pattern was not drawn or not cleaned\n", probe);
		return 0;
	}
	return pa;
}

void t6_picture_map(struct yd_t6_display_bases *b, size_t cru_bytes, size_t hdmi_bytes)
{
	mm_reg_t vop, mmu, pmu, cru, sys_grf, phy_grf, hdmi;

	device_map(&vop, YD_T6_DISPLAY_VOP_PHYS, T6_PICTURE_VOP_MAP, K_MEM_CACHE_NONE);
	device_map(&mmu, YD_T6_DISPLAY_VOP_MMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, YD_T6_DISPLAY_PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&cru, YD_T6_DISPLAY_CRU_PHYS, cru_bytes, K_MEM_CACHE_NONE);
	device_map(&sys_grf, YD_T6_DISPLAY_SYS_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy_grf, YD_T6_DISPLAY_HDPTX0_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&hdmi, YD_T6_DISPLAY_HDMI0_PHYS, hdmi_bytes, K_MEM_CACHE_NONE);
	*b = (struct yd_t6_display_bases){
		.vop = vop,
		.vop_mmu = { mmu + 0xe00, mmu + 0xf00 },
		.pmu = pmu,
		.cru = cru,
		.sys_grf = sys_grf,
		.phy_grf = phy_grf,
		.hdmi = hdmi,
	};
}

void t6_picture_delay_us(uint32_t us)
{
	k_busy_wait(us);
}

uint64_t t6_picture_now_us(void)
{
	return k_cyc_to_us_floor64(k_cycle_get_64());
}

void t6_picture_hold(const char *probe)
{
	printk("%s: holding the picture for %d s: red top left, green top right, blue bottom left, "
	       "yellow bottom right, white 1-pixel border\n",
	       probe, T6_PICTURE_HOLD_S);
	for (int i = 1; i <= T6_PICTURE_HOLD_S; i++) {
		k_sleep(K_SECONDS(1));
		t6_watchdog_feed();
		if (i % 10 == 0) {
			printk("%s: held %d s\n", probe, i);
		}
	}
}

int t6_picture_status(const char *probe, const struct yd_t6_display_bases *b,
		      struct yd_t6_display_status *s)
{
	const uint32_t *v = s->vop;
	int rc;

	rc = yd_t6_display_status(b, T6_PICTURE_CHECK_WAIT_US, T6_PICTURE_TIMED_FRAMES,
				  t6_picture_now_us, t6_picture_delay_us, s);
	t6_watchdog_feed();
	printk("%s: frame check over %u ms: frame start %u, underflow %u, bus error %u %u, "
	       "VP0 INT_RAW_STATUS 0x%08x\n",
	       probe, T6_PICTURE_CHECK_WAIT_US / 1000, s->frame_start, s->underflow,
	       s->bus_error[0], s->bus_error[1], s->raw);
	if (s->frames != 0 && s->frames_us != 0) {
		/* htotal and vtotal of the mode VP0 runs: the upper halves of
		 * DSP_HTOTAL_HS_END and DSP_VTOTAL_VS_END as read back */
		uint64_t htotal = v[5] >> 16, vtotal = v[6] >> 16;
		uint64_t mhz = (uint64_t)s->frames * 1000000000ULL / s->frames_us;
		uint64_t khz = htotal * vtotal * s->frames * 1000ULL / s->frames_us;

		printk("%s: %u frames in %u us: %u mHz, pixel clock %u kHz\n", probe, s->frames,
		       s->frames_us, (uint32_t)mhz, (uint32_t)khz);
	} else {
		printk("%s: no frame timed (refusal %u, detail 0x%08x)\n", probe, s->refusal,
		       s->detail);
	}
	printk("%s: VOP VERSION_INFO 0x%08x SYS_PD_CTRL 0x%08x REG_CFG_DONE 0x%08x "
	       "DSP_IF_EN 0x%08x DSP_CTRL 0x%08x HTOTAL_HS_END 0x%08x VTOTAL_VS_END 0x%08x\n",
	       probe, v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
	printk("%s: VOP OVL_PORT_SEL 0x%08x OVL_LAYER_SEL 0x%08x Esmart0 REGION0_CTRL "
	       "0x%08x YRGB_MST 0x%08x VIR 0x%08x INT_RAW_STATUS 0x%08x SYS0 0x%08x SYS1 0x%08x\n",
	       probe, v[7], v[8], v[9], v[10], v[11], v[12], v[13], v[14]);
	printk("%s: HPD %u (SOC_STATUS1 0x%08x), PHY0 GRF STATUS 0x%08x\n", probe,
	       (s->soc_status1 >> 16) & 1, s->soc_status1, s->phy_status);
	if (s->vo1_usable) {
		printk("%s: HDMI0 CMU_STATUS 0x%08x VIDQPCLK_FREQ %u LINKQPCLK_FREQ %u\n", probe,
		       s->cmu[0], s->cmu[1], s->cmu[2]);
		printk("%s: HDMI0 VIDEO_MONITOR_STATUS0-6 0x%08x 0x%08x 0x%08x 0x%08x "
		       "0x%08x 0x%08x 0x%08x\n",
		       probe, s->monitor[0], s->monitor[1], s->monitor[2], s->monitor[3],
		       s->monitor[4], s->monitor[5], s->monitor[6]);
	} else {
		printk("%s: HDMI0 not read: PD_VO1 not usable or PCLK_HDMITX0 gated "
		       "(CLKGATE_CON(60) 0x%08x)\n",
		       probe, s->cru_gate60);
	}
	return rc;
}

bool t6_picture_verdict(const char *probe, int rc, const struct yd_t6_display_status *s,
			uint32_t fb)
{
	if (rc == 0 && s->frame_start && !s->underflow && !s->bus_error[0] && !s->bus_error[1] &&
	    s->vop[10] == fb) {
		printk("%s: PASS: Esmart0 of VP0 shows 0x%08x, frames run, no underflow, no "
		       "bus error\n",
		       probe, fb);
		return true;
	}
	printk("%s: FAIL: status rc %d, frame start %u, underflow %u, bus error %u %u, "
	       "YRGB_MST 0x%08x\n",
	       probe, rc, s->frame_start, s->underflow, s->bus_error[0], s->bus_error[1],
	       s->vop[10]);
	return false;
}
