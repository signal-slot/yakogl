/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95_display.h -- the Verdin iMX95's screen, the Toradex 10.1" DSI
 * display on X4 (verdin_imx95_display.c; CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY),
 * for the board layer's board_show().
 */
#ifndef YAKOGL_VERDIN_IMX95_DISPLAY_H
#define YAKOGL_VERDIN_IMX95_DISPLAY_H

#include <stdint.h>

/* The panel: XRGB8888, top row first (yakogl_drivers.h YD_IMX95_DISPLAY_*). */
#define VERDIN_IMX95_SCREEN_W 1280u
#define VERDIN_IMX95_SCREEN_H 800u

/* Show the panel-sized XRGB8888 buffer at physical `fb`, rows `stride`
 * bytes apart. The first call brings the picture up (the I2C buses and the
 * expander first, then yd_imx95_display_up: the design note's twelve
 * steps); a later one flips to `fb` and returns once the DPU has taken it,
 * so the buffer shown before is free. 0, -ENOTSUP for a stride that is not
 * the first show's, -ENODEV once an up stopped (its lines say where; not
 * tried again), or the errno of the step that refused. Main only. */
int verdin_imx95_display_show(uint64_t fb, uint32_t stride);

/* The picture down (yd_imx95_display_down), if it is up; for the end of a
 * run. Main only. */
void verdin_imx95_display_down(void);

#endif /* YAKOGL_VERDIN_IMX95_DISPLAY_H */
