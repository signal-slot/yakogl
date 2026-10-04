/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The i.MX 8M Plus LCDIFv3 device (yakogl,imx8mp-lcdif;
 * driver/drivers/display/display_yakogl_imx8mp_lcdif.c), for a chain owner
 * that runs the bring-up order itself (src/display/imx8mp_display.c) and for
 * code that wants to wait for vertical blanking. Zephyr's display API covers
 * the rest (write = flip, blanking on/off).
 */
#ifndef SAFEUI_IMX8MP_LCDIF_H
#define SAFEUI_IMX8MP_LCDIF_H

#include <zephyr/device.h>
#include <zephyr/kernel.h>
#include <yakogl_drivers.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Timing and the layer at fb_phys (XRGB8888, pitch 0 = width * 4); display
 * and layer stay off. -ENODEV on an adopt-only node. */
int yakogl_imx8mp_lcdif_configure(const struct device *dev, const struct yd_lcdif_mode *mode,
				  uint64_t fb_phys, uint32_t pitch);
/* Display and layer on (panic thresholds, the VS_BLANK interrupt when the
 * node has one); waits for the first frame boundary. */
int yakogl_imx8mp_lcdif_enable(const struct device *dev);
/* Cold: stop at a frame boundary, display off. Adopted: hand the previous
 * owner's layer back. */
int yakogl_imx8mp_lcdif_disable(const struct device *dev);
/* Take over a layer the previous kernel left running; fb_phys 0 writes
 * nothing (the first flip brings our layout). */
int yakogl_imx8mp_lcdif_adopt(const struct device *dev, uint64_t fb_phys, uint32_t pitch);
int yakogl_imx8mp_lcdif_set_fb(const struct device *dev, uint64_t fb_phys);
/* The next vertical blanking (needs the node's interrupt): 0, -EAGAIN on
 * timeout, -ENOTSUP without an interrupt. */
int yakogl_imx8mp_lcdif_wait_vblank(const struct device *dev, k_timeout_t timeout);
/* What the device did last (on, adopted, fb, pitch, size, last IRQ status). */
const struct yd_lcdif_data *yakogl_imx8mp_lcdif_data(const struct device *dev);

#ifdef __cplusplus
}
#endif

#endif /* SAFEUI_IMX8MP_LCDIF_H */
