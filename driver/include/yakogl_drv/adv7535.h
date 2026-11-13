/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The ADV7535 DSI-to-HDMI bridge as a Zephyr device (yakogl,adv7535;
 * driver/drivers/display/bridge_adv7535.c, register logic in
 * driver/rust/src/ip/adv7535.rs through yakogl_mp25_display.h).
 *
 * The device has no Zephyr class API: the chain owner (the LTDC driver) calls
 * these, in the order of yakogl_mp25_display.h:
 *
 *   adv7535_init()          chip probe, powered down (after the board layer
 *                           released reset and switched on the HDMI 5 V)
 *   adv7535_hpd()
 *   adv7535_read_edid()     block 0, before start
 *   adv7535_choose_mode()   the mode the LTDC and DSI host are set to
 *   (LTDC scanning, then the DSI host streaming video: mipi_dsi_attach())
 *   adv7535_start()         ONLY after the DSI host streams video with the
 *                           clock lane in HS; the part locks on that stream
 *   adv7535_stop()          before the DSI host's detach
 *
 * Calls are serialised by the device's mutex. Every call returns a negative
 * errno (-ENODEV for a device that is not a ready yakogl,adv7535); the
 * meaning of the others is that of the yd_mp25_adv7535_* call it wraps.
 */
#ifndef YAKOGL_DRV_ADV7535_H
#define YAKOGL_DRV_ADV7535_H

#include <stddef.h>
#include <stdint.h>
#include <zephyr/device.h>
#include <yakogl_mp25_display.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Configure reset-gpios (released), pd-gpios (woken: asserted for 5 ms, then
 * released, as adv7511_drv.c does) and hpd-gpios (input), then
 * yd_mp25_adv7535_init(). -EIO: the main map does not answer (reset, 5 V,
 * the I2C controller). May be called again after adv7535_stop(). */
int adv7535_init(const struct device *dev);

/* 1 plugged, 0 not. With hpd-gpios the line, else the chip's status
 * register. */
int adv7535_hpd(const struct device *dev);

/* EDID block 0 (YD_MP25_EDID_BLOCK = 128 bytes) into buf. -ENODEV with HPD
 * low, -EBUSY once started. */
int adv7535_read_edid(const struct device *dev, uint8_t *buf);

/* yd_mp25_choose_mode() with the node's lane count; YD_MP25_MODE_FROM_EDID,
 * _FALLBACK_NO_EDID or _FALLBACK_LIMIT (edid may be NULL). */
int adv7535_choose_mode(const struct device *dev, const uint8_t *edid, size_t len,
			struct yd_mp25_mode *mode, struct yd_mp25_hdmi_sink *sink);

/* Power the part up and set its mode. Legal only after the DSI host streams
 * video (mipi_dsi_attach() of yakogl,stm32-dsi returned 0). */
int adv7535_start(const struct device *dev, const struct yd_mp25_mode *mode,
		  const struct yd_mp25_hdmi_sink *sink);

/* Powered down, DSI receiver off. */
int adv7535_stop(const struct device *dev);

/* What the last init found (revision, HPD, started); NULL for a device that
 * is not a ready yakogl,adv7535. */
const struct yd_mp25_adv7535_data *adv7535_data(const struct device *dev);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_DRV_ADV7535_H */
