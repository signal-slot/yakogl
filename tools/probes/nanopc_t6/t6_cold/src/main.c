/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_cold: step D8 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
 * section 2 route (c), section 5, section 7). Started by a normal kexec, so
 * Linux's shutdown has turned the display off: VP0 in STANDBY, its pixel
 * and bus clocks gated, the IOMMU off. Before main() the board glue has
 * armed WDT0 (nanopc_t6.c). This draws the section 5 pattern into a buffer
 * of this image and cleans it (common/t6_picture.c, as t6_display), then
 * yd_t6_display_cold (board::nanopc_t6_display_cold over
 * soc::rk3588_display_chain): with power-cycle every display block is put
 * off and in reset, then power, clocks and resets, the GRF words, the PHY
 * at the mode's pixel clock, DCLK_VOP0 on the PHY, the HDMI controller
 * bound, VP0 started at the mode, the controller enabled in DVI on VP0's
 * video clock, Esmart0 on the buffer, and the video check. The
 * chain turns the VOP's IOMMU off itself and makes Esmart0 VP0's only
 * layer, so nothing of that is left to this file.
 *
 * The mode is the build's (T6_COLD_MODE): 1024x768@60 at 65 MHz, or
 * 1920x1080@60 at 148.5 MHz for a sink that shows no other mode (the HDMI
 * capture unit on the board's output). The buffer is 1024x768 in both: at
 * 1920x1080 it is the screen's top left and the rest is VP0's background,
 * black.
 *
 * Every line of the drivers crate goes to the UART at once (cold_log): what
 * the chain reads before it writes, one line per stage before the stage's
 * first access (a hang leaves its stage as the last line), and what each
 * stage did. Then the chain's rc. When the picture is up (the chain ran
 * through, or stopped after the VOP or VIDEO stage as asked) the picture is
 * held T6_PICTURE_HOLD_S seconds and the section 5 read-backs are printed;
 * then PASS or FAIL, or STOPPED for a stop as asked, and the reset back into
 * Armbian. Below the VOP stage nothing scans out and the read-backs would
 * read a VOP the chain may have left off: they are not taken.
 *
 * The watchdog: one t6_watchdog_feed() before the chain and one after it,
 * so the chain must end inside CONFIG_YAKOGL_NANOPC_T6_ALIVE_S (5 s). Its
 * polls are bounded: the PMU's at 10 ms each (at most 27 of them with
 * power-cycle: 6 to make an idle VOP readable, 9 to switch three domains
 * off, 12 to switch two on; each step is a 1 us wait and a register read,
 * so the time is longer than the bound), a frame at 50 ms (the overlay;
 * with force-cold up to 8 more for four running ports), the PHY's 5.5 ms,
 * the IOMMU's 3 ms and the video check's two frames (33 ms). That is
 * 0.36 s of bounds without force-cold and 0.76 s with it, about twice the
 * PMU's part in time: well inside 5 s. The run prints the time the chain
 * took. Nothing here touches the GPU; the GIC has every SPI disabled by
 * Zephyr's init.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <errno.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "t6_picture.h"

BUILD_ASSERT(T6_COLD_STOP_AFTER >= 0 && T6_COLD_STOP_AFTER <= YD_T6_DISPLAY_COLD_STAGE_VIDEO,
	     "T6_COLD_STOP_AFTER: 0 or a stage");
BUILD_ASSERT(T6_COLD_FORCE_COLD == 0 || T6_COLD_FORCE_COLD == 1, "T6_COLD_FORCE_COLD: 0 or 1");
BUILD_ASSERT(T6_COLD_MODE_1920X1080 == 0 || T6_COLD_MODE_1920X1080 == 1,
	     "T6_COLD_MODE_1920X1080: 0 or 1");

/* The CRU with the PMU1 CRU at +0x30000 (the PHY's reference clock and
 * resets), and the HDMI QP TX0 up to its interrupt words at 0x3000. */
#define CRU_MAP  0x31000
#define HDMI_MAP 0x5000

/* The drivers crate's lines, on the UART at once and as they are: a line
 * printed before a hang must not wait in a log buffer. */
static void cold_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	ARG_UNUSED(ctx);
	ARG_UNUSED(level);
	printk("%.*s\n", (int)len, (const char *)buf);
}

int main(void)
{
	const uint32_t flags =
		YD_T6_DISPLAY_COLD_POWER_CYCLE |
		(T6_COLD_FORCE_COLD ? YD_T6_DISPLAY_COLD_FORCE_COLD : 0) |
		(T6_COLD_MODE_1920X1080 ? YD_T6_DISPLAY_COLD_MODE_1920X1080 : 0);
	struct yd_t6_display_cold_bases b;
	struct yd_t6_display_status s;
	struct yd_t6_display_cold c = { 0 };
	struct yd_env env;
	mm_reg_t vop_grf, vo1_grf, phy;
	uint64_t fb_pa;
	int64_t t0, ms;
	bool up;
	int rc;

	printk("t6_cold: build %s\n", T6_COLD_BUILD);
	printk("t6_cold: WDT0 period %u ms, fed until %u ms of uptime\n", t6_watchdog_period_ms(),
	       t6_watchdog_deadline_ms());
	printk("t6_cold: options: power-cycle 1, force-cold %d, stop after %d (%s)\n",
	       T6_COLD_FORCE_COLD, T6_COLD_STOP_AFTER,
	       T6_COLD_STOP_AFTER == 0 ? "run to the end" :
					 t6_display_cold_stage_name(T6_COLD_STOP_AFTER));
	printk("t6_cold: mode %s\n", T6_COLD_MODE_1920X1080 ? "1920x1080" : "1024x768");

	fb_pa = t6_picture_prepare("t6_cold");
	if (fb_pa == 0) {
		t6_reset_to_linux("t6_cold done");
	}
	t6_picture_map(&b.display, CRU_MAP, HDMI_MAP);
	device_map(&vop_grf, YD_T6_DISPLAY_VOP_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&vo1_grf, YD_T6_DISPLAY_VO1_GRF_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&phy, YD_T6_DISPLAY_HDPTX0_PHYS, 0x2000, K_MEM_CACHE_NONE);
	b.vop_grf = vop_grf;
	b.vo1_grf = vo1_grf;
	b.phy = phy;
	yd_env_init(&env, NULL);
	env.log = cold_log;
	t6_watchdog_feed();

	t0 = k_uptime_get();
	rc = yd_t6_display_cold(&b, &env, flags, T6_COLD_STOP_AFTER, fb_pa, &c);
	ms = k_uptime_get() - t0;
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_cold: FAIL: yd_t6_display_cold refused its arguments\n");
		t6_reset_to_linux("t6_cold done");
	}
	t6_display_print_cold("t6_cold", rc, (long long)ms, &c);
	if (rc != 0 && rc != -ECANCELED) {
		printk("t6_cold: FAIL: stage %u (%s) failed\n", c.stage,
		       t6_display_cold_stage_name(c.stage));
		t6_reset_to_linux("t6_cold done");
	}

	up = rc == 0 || c.stage >= YD_T6_DISPLAY_COLD_STAGE_VOP;
	if (up) {
		t6_picture_hold("t6_cold");
		rc = t6_display_read_back("t6_cold", &b.display, &s);
		if (c.error == YD_T6_DISPLAY_COLD_ERROR_NONE) {
			t6_picture_verdict("t6_cold", rc, &s, (uint32_t)fb_pa);
			t6_reset_to_linux("t6_cold done");
		}
	}
	printk("t6_cold: STOPPED: after stage %u (%s) as asked%s\n", c.stage,
	       t6_display_cold_stage_name(c.stage),
	       up ? "" : ", nothing scans out");
	t6_reset_to_linux("t6_cold done");
}
