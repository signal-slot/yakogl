/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95's screen on X12 (verdin_imx95_display.h;
 * CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY_X12_LVDS, the default): the Toradex
 * Capacitive Touch Display 10.1" LVDS on the Mallow's native LVDS
 * connector, 1280x800 XRGB8888, cold, in the order of
 * driver/docs/imx95-display.md section 4(b'). The register logic and the
 * SCMI requests are the drivers crate's (board::verdin_imx95_lvds through
 * yd_imx95_lvds_up / _flip / _down); this file is what is Zephyr's:
 *
 *   SCMI     the board's one raw channel (verdin_imx95_scmi.c), the GPU's too
 *   blocks   the x12_display node's six reg entries (app.overlay), each
 *            mapped as Device memory (device_map)
 *   pins     the backlight's PWM (GPIO_IO06 as TPM5 channel 0) and enable
 *            (XSPI1_DATA4 as GPIO5_IO04) from the x12_display node, through
 *            the System Manager's pin control (SCMI protocol 0x19)
 *   waits    k_busy_wait (yd_env_init)
 *
 * No I2C and no Zephyr GPIO driver: the panel has no bridge, and the
 * backlight's enable is driven as a register by the Rust side (the RGPIO's
 * set and direction registers). Touch (X13) is not part of the picture.
 *
 * Built and host-tested (the Rust side), not run on a board yet.
 */
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/pinctrl.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "verdin_imx95_display.h"
#include "verdin_imx95_scmi.h"

#define DISP_NODE DT_NODELABEL(x12_display)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(DISP_NODE, yakogl_imx95_lvds_display, okay),
	     "the board layer's app.overlay has the X12 display (x12_display)");
BUILD_ASSERT(VERDIN_IMX95_SCREEN_W == YD_IMX95_DISPLAY_WIDTH &&
		     VERDIN_IMX95_SCREEN_H == YD_IMX95_DISPLAY_HEIGHT,
	     "the screen is the panel the Rust side drives");

#define BLOCK_CHECK(name, PHYS, SIZE)                                                       \
	BUILD_ASSERT(DT_REG_ADDR_BY_NAME(DISP_NODE, name) == (PHYS) &&                      \
			     DT_REG_SIZE_BY_NAME(DISP_NODE, name) >= (SIZE),                \
		     "x12_display reg \"" #name "\" is " #PHYS ", at least " #SIZE " bytes")
BLOCK_CHECK(ldb, YD_IMX95_LDB_PHYS, YD_IMX95_LDB_SIZE);
BLOCK_CHECK(dpu, YD_IMX95_DPU_PHYS, YD_IMX95_DPU_SIZE);
BLOCK_CHECK(csr, YD_IMX95_DISPLAY_CSR_PHYS, YD_IMX95_DISPLAY_CSR_SIZE);
BLOCK_CHECK(interleaver, YD_IMX95_INTERLEAVER_PHYS, YD_IMX95_INTERLEAVER_SIZE);
BLOCK_CHECK(tpm, YD_IMX95_TPM5_PHYS, YD_IMX95_TPM5_SIZE);
BLOCK_CHECK(gpio, YD_IMX95_GPIO5_PHYS, YD_IMX95_GPIO5_SIZE);

PINCTRL_DT_DEFINE(DISP_NODE);

static struct yd_env vi95_env;
static struct yd_imx95_lvds_bases vi95_bases;
static struct yd_imx95_lvds vi95_disp;

#define MAP(field, name)                                                                     \
	device_map((mm_reg_t *)&vi95_bases.field, DT_REG_ADDR_BY_NAME(DISP_NODE, name),     \
		   DT_REG_SIZE_BY_NAME(DISP_NODE, name), K_MEM_CACHE_NONE)

static int vi95_display_up(uint64_t fb, uint32_t stride)
{
	int rc = pinctrl_apply_state(PINCTRL_DT_DEV_CONFIG_GET(DISP_NODE), PINCTRL_STATE_DEFAULT);

	if (rc != 0) {
		printk("verdin_imx95: display: the backlight's pins (TPM5 channel 0, GPIO5_IO04): "
		       "%d\n",
		       rc);
		vi95_disp.state = YD_IMX95_DISPLAY_FAILED;
		return -ENODEV;
	}
	MAP(dpu, dpu);
	MAP(csr, csr);
	MAP(interleaver, interleaver);
	MAP(ldb, ldb);
	MAP(tpm, tpm);
	MAP(gpio, gpio);
	yd_env_init(&vi95_env, NULL);

	printk("verdin_imx95: display: X12 LVDS, the Toradex 10.1\" LVDS panel\n");
	rc = yd_imx95_lvds_up(verdin_imx95_scmi, &vi95_bases, &vi95_env, fb, stride, &vi95_disp);
	if (rc == 0) {
		printk("verdin_imx95: display up: %ux%u XRGB8888 at 0x%llx stride %u, "
		       "pixel clock %u Hz (DISP level %u), FrameGen0 in sync after %u us, "
		       "LDB 0x%08x 0x%08x 0x%08x 0x%08x, backlight %u/255 (TPM5 at %u Hz)\n",
		       vi95_disp.width, vi95_disp.height, (unsigned long long)fb, stride,
		       vi95_disp.pix_hz[0], vi95_disp.level, vi95_disp.sync_us, vi95_disp.ldb[0],
		       vi95_disp.ldb[1], vi95_disp.ldb[2], vi95_disp.ldb[3], vi95_disp.backlight,
		       vi95_disp.tpm_hz);
	} else {
		printk("verdin_imx95: display stopped: rc %d, %s side (the line before says "
		       "where)\n",
		       rc,
		       vi95_disp.stop == YD_IMX95_LVDS_STOP_BACKLIGHT ? "backlight"
		       : vi95_disp.stop == YD_IMX95_LVDS_STOP_DPU     ? "DPU"
								      : "no");
		vi95_disp.state = YD_IMX95_DISPLAY_FAILED;
		rc = -ENODEV;
	}
	return rc;
}

int verdin_imx95_display_show(uint64_t fb, uint32_t stride)
{
	int rc;

	switch (vi95_disp.state) {
	case YD_IMX95_DISPLAY_NEW:
		return vi95_display_up(fb, stride);
	case YD_IMX95_DISPLAY_UP:
		break;
	default:
		return -ENODEV;
	}
	if (stride != vi95_disp.stride) {
		return -ENOTSUP;
	}
	rc = yd_imx95_lvds_flip(&vi95_bases, &vi95_env, fb, &vi95_disp);
	if (rc == -ETIMEDOUT) {
		printk("verdin_imx95: display: no shadow load for 0x%llx\n",
		       (unsigned long long)fb);
	}
	return rc;
}

void verdin_imx95_display_down(void)
{
	int rc;

	if (vi95_disp.state != YD_IMX95_DISPLAY_UP) {
		return;
	}
	rc = yd_imx95_lvds_down(verdin_imx95_scmi, &vi95_bases, &vi95_env, &vi95_disp);
	printk("verdin_imx95: display down: %d after %u flip(s)\n", rc, vi95_disp.flips);
}
