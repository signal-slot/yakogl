/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Raspberry Pi 5's GPU platform for the V3D 7.1 OSAL
 * (src/osal/zephyr_v3d71.c): the V3D clock is the firmware's clock 5
 * (driver/docs/rpi5-v3d71-facts.md), switched through the property mailbox
 * (rpi4/rpi_mbox.c built for CONFIG_SOC_BCM2712), as the rpi_5 probes do it.
 * This is what osal_zephyr_v3d_default_platform() answers, so an image that
 * passes EGL_DEFAULT_DISPLAY gets the clock switch without a board argument,
 * as the Pi 4's board layer (rpi4/board_rpi4.c) gives it. Built into the GL
 * library with CONFIG_YAKOGL_GPU_V3D71 (zephyr/CMakeLists.txt).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <stdbool.h>
#include <stddef.h>

#include "osal_internal.h" /* struct osal_zephyr_v3d_init */
#include "rpi_mbox.h"

static int v3d71_clock_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	return rpi_clock_state(RPI_FW_CLK_V3D, on);
}

static const struct osal_zephyr_v3d_init s_v3d71_platform = { .clock_set = v3d71_clock_set };

/* The mailbox mapped once (rpi_mbox_init maps it at every call); NULL, with
 * the reason printed, when it cannot be: the OSAL then refuses its init. */
const struct osal_zephyr_v3d_init *osal_zephyr_v3d_default_platform(void)
{
	static bool mapped;

	if (!mapped) {
		const int rc = rpi_mbox_init();

		if (rc != 0) {
			printk("board: rpi5 mailbox not mapped (%d): no V3D clock switch\n", rc);
			return NULL;
		}
		mapped = true;
	}
	return &s_v3d71_platform;
}
