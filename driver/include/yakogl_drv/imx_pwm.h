/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The one thing the yakogl,imx-pwm device offers beyond Zephyr's PWM API:
 * whether the output is running, read from the hardware (PWMCR.EN with a
 * clock selected). imx8mp_display_health() reports it as BACKLIGHT_PWM.
 */
#ifndef YAKOGL_DRV_IMX_PWM_H
#define YAKOGL_DRV_IMX_PWM_H

#include <stdbool.h>
#include <zephyr/device.h>

/* false for a device that is not a yakogl,imx-pwm or is not ready. */
bool yakogl_imx_pwm_is_enabled(const struct device *dev);

#endif /* YAKOGL_DRV_IMX_PWM_H */
