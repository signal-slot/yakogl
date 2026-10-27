/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The calls of driver/src/display/imx8mp_display.h that board_imx8mp.c
 * makes, for the host model: ../latch_model.c's simulated LCDIF. */
#ifndef LATCH_MODEL_IMX8MP_DISPLAY_H
#define LATCH_MODEL_IMX8MP_DISPLAY_H
#include <zephyr/kernel.h>

int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch);
int imx8mp_display_set_fb(uintptr_t fb_phys);
void imx8mp_display_geometry(uint32_t *width, uint32_t *height);
#endif
