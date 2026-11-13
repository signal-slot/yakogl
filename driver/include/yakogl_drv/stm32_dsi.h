/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The driver-specific half of the STM32MP25 DSI host (yakogl,stm32-dsi;
 * driver/drivers/mipi_dsi/dsi_yakogl_stm32.c): what struct mipi_dsi_device
 * has no field for. mipi_dsi_attach() brings the host up and starts video, so
 * the chain owner calls it with the LTDC already scanning (the pixel clock
 * running), then starts the bridge (adv7535_start()).
 */
#ifndef YAKOGL_DRV_STM32_DSI_H
#define YAKOGL_DRV_STM32_DSI_H

#include <stdint.h>
#include <zephyr/device.h>
#include <yakogl_mp25_display.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The pixel clock the LTDC really runs and the sync polarities
 * (YD_MP25_MODE_*) of the mode, for the next attach (the porches come from
 * the mipi_dsi_device's timings). Without it the node's pixel-clock-frequency
 * and active-high syncs are used. -EBUSY while attached. */
int stm32_dsi_set_mode(const struct device *dev, uint32_t pixel_clock_hz, uint32_t mode_flags);

/* What the last attach programmed (PLL, rates, failed step and errno); NULL
 * for a device that is not a ready yakogl,stm32-dsi. */
const struct yd_mp25_dsi_data *stm32_dsi_data(const struct device *dev);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_DRV_STM32_DSI_H */
