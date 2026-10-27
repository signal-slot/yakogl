/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The TI SN65DSI83/84 DSI-to-LVDS bridge as a Zephyr device (yakogl,sn65dsi8x;
 * driver/drivers/display/bridge_sn65dsi8x.c, register logic in
 * driver/rust/src/ip/sn65dsi8x.rs).
 *
 * A node with a dsi-host phandle and a mode starts itself at init (attach
 * the DSI host, then the bridge; the AM62P). A node without one waits for
 * the display controller that owns the chain to call it (the i.MX8MP):
 *
 *   sn65dsi8x_claim_enable()  once, with the EN level it finds the chain at
 *   sn65dsi8x_start()         with the DSI clock lane in HS, no video yet
 *   (the controller starts the video)
 *   sn65dsi8x_check_link()    the error status after the video started
 *   sn65dsi8x_stop()          EN low
 *
 * Every call except sn65dsi8x_health() and sn65dsi8x_data() returns a
 * negative errno for a device that is not a ready yakogl,sn65dsi8x.
 */
#ifndef YAKOGL_DRV_SN65DSI8X_H
#define YAKOGL_DRV_SN65DSI8X_H

#include <stdbool.h>
#include <stdint.h>
#include <zephyr/device.h>
#include <yakogl_drivers.h>

/* Configure the EN pin as an output at `asserted`, without a transition:
 * a bridge found running stays running. */
int sn65dsi8x_claim_enable(const struct device *dev, bool asserted);

/* EN cycle, ID, CSRs, PLL, soft reset (data sheet Table 7-2, Init seq 3-7).
 * lane_rate_hz: the DSI bit rate per lane the host runs (0: pixel clock *
 * 24 / lanes). */
int sn65dsi8x_start(const struct device *dev, const struct yd_sn65dsi8x_mode *mode,
		    uint32_t lane_rate_hz);

/* Init seq 8-11: 5 ms, clear IRQ_STAT, 1 ms, read it. Returns IRQ_STAT (0 =
 * clean link) or a negative errno. */
int sn65dsi8x_check_link(const struct device *dev);

/* EN low for the reset time (the part forgets its CSRs). */
int sn65dsi8x_stop(const struct device *dev);

/* YD_SN65DSI8X_HEALTH_* read from the part (reads only); 0 for a device
 * that is not a ready yakogl,sn65dsi8x. */
uint32_t sn65dsi8x_health(const struct device *dev);

/* IRQ_STAT (0xE5) as it stands, read only: what latched since
 * sn65dsi8x_check_link() cleared it. 0..255, or a negative errno. */
int sn65dsi8x_irq_stat(const struct device *dev);

/* What the last start found and wrote; NULL as above. */
const struct yd_sn65dsi8x_data *sn65dsi8x_data(const struct device *dev);

#endif /* YAKOGL_DRV_SN65DSI8X_H */
