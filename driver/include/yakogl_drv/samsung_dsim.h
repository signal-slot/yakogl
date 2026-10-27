/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The driver-specific half of the Samsung DSIM device (yakogl,samsung-dsim;
 * driver/drivers/mipi_dsi/dsi_yakogl_samsung.c): what Zephyr's MIPI-DSI API
 * (attach / transfer / detach) has no call for.
 *
 * mipi_dsi_attach() leaves the data lanes held in stop state with the clock
 * lane in HS, so a bridge can be programmed before video flows; the data
 * lanes are released by yakogl_samsung_dsim_video_start(), called by the
 * LCDIF's blanking_off() (its dsi-host) or by the chain owner after the
 * LCDIF scans. Risk R5 of docs/source-form.md, decided in
 * driver/docs/imx8mp-display-cold.md.
 */
#ifndef YAKOGL_DRV_SAMSUNG_DSIM_H
#define YAKOGL_DRV_SAMSUNG_DSIM_H

#include <zephyr/device.h>
#include <yakogl_drivers.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The pixel clock the LCDIF really runs (the next attach turns the porches
 * into blanking word counts with it). */
int yakogl_samsung_dsim_set_pixel_clock(const struct device *dev, uint32_t hz);
/* Release the data lanes: video goes out as the LCDIF sends lines. -ENODEV
 * when not attached. */
int yakogl_samsung_dsim_video_start(const struct device *dev);
/* YD_SAMSUNG_DSIM_H_* bits, read from the hardware (the block must be
 * powered: the caller's power domain). */
uint32_t yakogl_samsung_dsim_health(const struct device *dev);
/* What the last attach programmed (PLL, rates, PHY timings, step, error). */
const struct yd_samsung_dsim_data *yakogl_samsung_dsim_data(const struct device *dev);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_DRV_SAMSUNG_DSIM_H */
