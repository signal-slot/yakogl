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
	t6_display_print_adopt("t6_display", rc, &a);
	if (rc != 0) {
		printk("t6_display: FAIL: adopt stop %u\n", a.stop);
		t6_reset_to_linux("t6_display done");
	}

	t6_picture_hold("t6_display");
	rc = t6_picture_status("t6_display", &b, &s);
	t6_picture_verdict("t6_display", rc, &s, a.fb);
	t6_reset_to_linux("t6_display done");
}
