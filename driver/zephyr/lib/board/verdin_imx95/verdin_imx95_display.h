/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95_display.h -- the Verdin iMX95's screen for the board layer's
 * board_show() (CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY): the Toradex 10.1" LVDS
 * display on X12 (verdin_imx95_lvds.c, CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY_X12_LVDS,
 * the default) or the 10.1" DSI display on X4 (verdin_imx95_display.c,
 * CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY_X4_DSI). Both implement this header;
 * the image builds one.
 */
#ifndef YAKOGL_VERDIN_IMX95_DISPLAY_H
#define YAKOGL_VERDIN_IMX95_DISPLAY_H

#include <stdint.h>

/* The panel: XRGB8888, top row first (yakogl_drivers.h YD_IMX95_DISPLAY_*);
 * both displays are 1280x800. */
#define VERDIN_IMX95_SCREEN_W 1280u
#define VERDIN_IMX95_SCREEN_H 800u

/* Show the panel-sized XRGB8888 buffer at physical `fb`, rows `stride`
 * bytes apart. The first call brings the picture up (X12: the backlight's
 * pins, then yd_imx95_lvds_up, the design note's section 4(b'); X4: the I2C
 * buses and the expander first, then yd_imx95_display_up, its twelve
 * steps); a later one flips to `fb` and returns once the DPU has taken it,
 * so the buffer shown before is free. 0, -ENOTSUP for a stride that is not
 * the first show's, -ENODEV once an up stopped (its lines say where; not
 * tried again), or the errno of the step that refused. Main only. */
int verdin_imx95_display_show(uint64_t fb, uint32_t stride);

/* The picture down (yd_imx95_lvds_down or yd_imx95_display_down), if it is
 * up; for the end of a run. Main only. */
void verdin_imx95_display_down(void);

#endif /* YAKOGL_VERDIN_IMX95_DISPLAY_H */
