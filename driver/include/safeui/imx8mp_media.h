/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The i.MX 8M Plus display power domains (yakogl,imx8mp-media-pd under a
 * yakogl,imx8mp-media-blk-ctrl), beyond what Zephyr's power-domain model
 * offers: an explicit on / off / reset pulse for a consumer that drives the
 * domain itself (the i.MX8MP display facade, until the LCDIF and the DSIM
 * are devices of their own), the clock plan of the pixel and MIPI PHY
 * reference roots, and a status word for a record without a console.
 *
 * `pd` is a yakogl,imx8mp-media-pd device; the clock calls act on its
 * parent's roots. Errors are negative errno.
 */
#ifndef SAFEUI_IMX8MP_MEDIA_H
#define SAFEUI_IMX8MP_MEDIA_H

#include <stdint.h>
#include <zephyr/device.h>

int imx8mp_media_pd_on(const struct device *pd);
int imx8mp_media_pd_off(const struct device *pd);
/* Assert the domain's block resets, clocks on, release (the domain is on). */
int imx8mp_media_pd_reset(const struct device *pd);

/* VIDEO_PLL1 to video_pll1_hz (0: leave it), then the pixel clock root
 * (MEDIA_DISP1_PIX) at the rate nearest want_hz, which lands in *out_hz. */
int imx8mp_media_set_pixel_clock(const struct device *pd, uint32_t want_hz,
				 uint32_t video_pll1_hz, uint32_t *out_hz);
/* What the pixel clock root produces now, from its parent PLL's registers;
 * 0 when unknown. */
uint32_t imx8mp_media_pixel_clock(const struct device *pd);
/* MEDIA_MIPI_PHY1_REF = 24 MHz / n, at most hz. Returns the rate, 0 on error. */
uint32_t imx8mp_media_set_phy_ref_clock(const struct device *pd, uint32_t hz);

/* YD_MEDIA_ST_* (yakogl_drivers.h) of the first yakogl,imx8mp-media-blk-ctrl;
 * 0 when there is none. */
uint32_t imx8mp_media_status(void);

#endif /* SAFEUI_IMX8MP_MEDIA_H */
