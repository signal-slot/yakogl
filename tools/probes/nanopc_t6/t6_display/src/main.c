/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_display: step D2 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
 * section 2 route (a), section 5). Started by a crash kexec, so Linux's
 * display chain still runs: VP0 scans 1024x768 at 60 Hz from Cluster0
 * through the VOP's IOMMU. Before main() the board glue has armed WDT0
 * (nanopc_t6.c). This draws the section 5 pattern into a buffer of this
 * image (red, green / blue, yellow quadrants, a white 1-pixel border),
 * cleans it to the point of coherency, then yd_t6_display_adopt
 * (board::nanopc_t6_display in the drivers crate): what Linux left is read
 * and checked, every window but Esmart0 turned off, the IOMMU's paging
 * turned off, Esmart0 made VP0's only layer on the buffer. It prints what
 * it found and did, holds the picture for T6_DISPLAY_HOLD_S seconds with
 * the watchdog fed, prints the section 5 read-backs (yd_t6_display_status)
 * and PASS or FAIL, and resets back into Armbian. Nothing here touches the
 * GPU; the GIC has every SPI disabled by Zephyr's init, so the VOP's and the
 * HDMI controller's interrupt lines Linux left enabled reach no CPU.
 */
#include <zephyr/kernel.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/cache.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <errno.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#define VOP_PHYS      0xfdd90000UL
#define VOP_MAP       0x2000
#define VOP_MMU_PHYS  0xfdd97000UL /* the two sets at +0xe00, +0xf00 */
#define PMU_PHYS      0xfd8d8000UL
#define CRU_PHYS      0xfd7c0000UL
#define SYS_GRF_PHYS  0xfd58c000UL
#define PHY_GRF_PHYS  0xfd5e0000UL
#define HDMI_PHYS     0xfde80000UL

/* The mode the adopt requires (board::nanopc_t6_display::MODE). */
#define FB_W      1024
#define FB_H      768
#define FB_STRIDE (FB_W * 4)
#define HTOTAL    1344
#define VTOTAL    806

/* The picture is held this long for the person at the display. */
#define T6_DISPLAY_HOLD_S 30
/* The status: the frame check's wait, the frames timed. */
#define CHECK_WAIT_US 100000
#define TIMED_FRAMES  60

BUILD_ASSERT(T6_DISPLAY_HOLD_S + 10 < CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
	     "the run ends well inside the watchdog's lifetime");

static uint32_t t6_display_fb[FB_W * FB_H] __aligned(4096);

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static uint64_t now_us(void)
{
	return k_cyc_to_us_floor64(k_cycle_get_64());
}

static void print_status(const struct yd_t6_display_status *s)
{
	const uint32_t *v = s->vop;

	printk("t6_display: frame check over %u ms: frame start %u, underflow %u, bus error %u %u, "
	       "VP0 INT_RAW_STATUS 0x%08x\n",
	       CHECK_WAIT_US / 1000, s->frame_start, s->underflow, s->bus_error[0], s->bus_error[1],
	       s->raw);
	if (s->frames != 0 && s->frames_us != 0) {
		uint64_t mhz = (uint64_t)s->frames * 1000000000ULL / s->frames_us;
		uint64_t khz = (uint64_t)HTOTAL * VTOTAL * s->frames * 1000ULL / s->frames_us;

		printk("t6_display: %u frames in %u us: %u mHz, pixel clock %u kHz\n", s->frames,
		       s->frames_us, (uint32_t)mhz, (uint32_t)khz);
	} else {
		printk("t6_display: no frame timed (refusal %u, detail 0x%08x)\n", s->refusal,
		       s->detail);
	}
	printk("t6_display: VOP VERSION_INFO 0x%08x SYS_PD_CTRL 0x%08x REG_CFG_DONE 0x%08x "
	       "DSP_IF_EN 0x%08x DSP_CTRL 0x%08x HTOTAL_HS_END 0x%08x VTOTAL_VS_END 0x%08x\n",
	       v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
	printk("t6_display: VOP OVL_PORT_SEL 0x%08x OVL_LAYER_SEL 0x%08x Esmart0 REGION0_CTRL "
	       "0x%08x YRGB_MST 0x%08x VIR 0x%08x INT_RAW_STATUS 0x%08x SYS0 0x%08x SYS1 0x%08x\n",
	       v[7], v[8], v[9], v[10], v[11], v[12], v[13], v[14]);
	printk("t6_display: HPD %u (SOC_STATUS1 0x%08x), PHY0 GRF STATUS 0x%08x\n",
	       (s->soc_status1 >> 16) & 1, s->soc_status1, s->phy_status);
	if (s->vo1_usable) {
		printk("t6_display: HDMI0 CMU_STATUS 0x%08x VIDQPCLK_FREQ %u LINKQPCLK_FREQ %u\n",
		       s->cmu[0], s->cmu[1], s->cmu[2]);
		printk("t6_display: HDMI0 VIDEO_MONITOR_STATUS0-6 0x%08x 0x%08x 0x%08x 0x%08x "
		       "0x%08x 0x%08x 0x%08x\n",
		       s->monitor[0], s->monitor[1], s->monitor[2], s->monitor[3], s->monitor[4],
		       s->monitor[5], s->monitor[6]);
	} else {
		printk("t6_display: HDMI0 not read: PD_VO1 not usable or PCLK_HDMITX0 gated "
		       "(CLKGATE_CON(60) 0x%08x)\n",
		       s->cru_gate60);
	}
}

int main(void)
{
	mm_reg_t vop, mmu, pmu, cru, sys_grf, phy_grf, hdmi;
	struct yd_t6_display_bases b;
	struct yd_t6_display_adopt a;
	struct yd_t6_display_status s;
	uint64_t fb_pa;
	int prc, crc, rc;

	printk("t6_display: build %s\n", T6_DISPLAY_BUILD);
	printk("t6_display: WDT0 period %u ms, fed until %u ms of uptime\n",
	       t6_watchdog_period_ms(), t6_watchdog_deadline_ms());

	prc = yd_t6_display_pattern(t6_display_fb, FB_W, FB_H, FB_STRIDE);
	/* Zephyr's "flush" is a clean to the point of coherency (dc cvac). */
	crc = sys_cache_data_flush_range(t6_display_fb, sizeof(t6_display_fb));
	fb_pa = k_mem_phys_addr(t6_display_fb);
	printk("t6_display: frame buffer %p at PA 0x%llx, %ux%u XRGB8888 stride %u, pattern rc %d, "
	       "clean rc %d\n",
	       (void *)t6_display_fb, (unsigned long long)fb_pa, FB_W, FB_H, FB_STRIDE, prc, crc);
	if (prc != 0 || crc != 0) {
		printk("t6_display: FAIL: the pattern was not drawn or not cleaned\n");
		t6_reset_to_linux("t6_display done");
	}

	device_map(&vop, VOP_PHYS, VOP_MAP, K_MEM_CACHE_NONE);
	device_map(&mmu, VOP_MMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&sys_grf, SYS_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy_grf, PHY_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&hdmi, HDMI_PHYS, 0x1000, K_MEM_CACHE_NONE);
	b = (struct yd_t6_display_bases){
		.vop = vop,
		.vop_mmu = { mmu + 0xe00, mmu + 0xf00 },
		.pmu = pmu,
		.cru = cru,
		.sys_grf = sys_grf,
		.phy_grf = phy_grf,
		.hdmi = hdmi,
	};
	t6_watchdog_feed();

	rc = yd_t6_display_adopt(&b, fb_pa, delay_us, &a);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_display: FAIL: yd_t6_display_adopt refused its arguments\n");
		t6_reset_to_linux("t6_display done");
	}
	t6_display_print_adopt("t6_display", rc, &a);
	if (rc != 0) {
		printk("t6_display: FAIL: adopt stop %u\n", a.stop);
		t6_reset_to_linux("t6_display done");
	}

	printk("t6_display: holding the picture for %d s: red top left, green top right, blue "
	       "bottom left, yellow bottom right, white 1-pixel border\n",
	       T6_DISPLAY_HOLD_S);
	for (int i = 1; i <= T6_DISPLAY_HOLD_S; i++) {
		k_sleep(K_SECONDS(1));
		t6_watchdog_feed();
		if (i % 10 == 0) {
			printk("t6_display: held %d s\n", i);
		}
	}

	rc = yd_t6_display_status(&b, CHECK_WAIT_US, TIMED_FRAMES, now_us, delay_us, &s);
	t6_watchdog_feed();
	print_status(&s);
	if (rc == 0 && s.frame_start && !s.underflow && !s.bus_error[0] && !s.bus_error[1] &&
	    s.vop[10] == a.fb) {
		printk("t6_display: PASS: Esmart0 of VP0 shows 0x%08x, frames run, no underflow, no "
		       "bus error\n",
		       a.fb);
	} else {
		printk("t6_display: FAIL: status rc %d, frame start %u, underflow %u, bus error %u "
		       "%u, YRGB_MST 0x%08x\n",
		       rc, s.frame_start, s.underflow, s.bus_error[0], s.bus_error[1], s.vop[10]);
	}
	t6_reset_to_linux("t6_display done");
}
