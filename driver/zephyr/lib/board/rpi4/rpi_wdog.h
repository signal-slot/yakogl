/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The bounded lifetime of a Raspberry Pi 4 test image (see rpi_wdog.c): the
 * watchdog arms itself at boot; the image only announces it.
 */
#ifndef RPI_WDOG_H
#define RPI_WDOG_H

#include <stdbool.h>

/* Seconds after boot during which the watchdog is fed: long enough to read the
 * screen, short enough that nobody has to reach for the power switch. */
#ifndef RPI_WDOG_HOLD_S
#define RPI_WDOG_HOLD_S 50
#endif

/* Print when the reset will come. */
void rpi_wdog_announce(void);

/* Whether the image is still making progress; the watchdog is fed only while
 * it says so (and only until RPI_WDOG_HOLD_S). rpi_wdog.c's weak default
 * always says yes; an image may override it. Called from a timer handler. */
bool rpi_wdog_alive(void);

#endif
