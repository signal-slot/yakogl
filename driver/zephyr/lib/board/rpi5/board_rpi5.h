/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Raspberry Pi 5's screen (BCM2712), route 2 of driver/docs/rpi5-display.md:
 * the framebuffer from the firmware's property mailbox, the console (rpi4/
 * fbcon.c) in it, and a window that is a view into it. The Pi 4's screen path
 * (rpi4/board_rpi4.c) for the Pi 5, with the same decisions
 * (driver/rust/src/board/rpi_fb.rs, board 5). It is not the whole board layer
 * (yakogl_board.h): there is no GPU platform or record screen here yet, so the
 * calls carry their own names.
 */
#ifndef BOARD_RPI5_H
#define BOARD_RPI5_H

#include <stdbool.h>
#include <stdint.h>

#include "yakogl_board.h" /* struct board_window */

/* True when the mailbox granted a usable 32 bpp framebuffer (the init hook
 * ran before main). Without one the console is the UART only. */
bool rpi5_screen_ready(void);

/* The screen as granted: width x height on screen, rows in memory, pitch in
 * bytes. Any pointer may be NULL; 0 each without a screen. */
void rpi5_screen_info(uint32_t *width, uint32_t *height, uint32_t *rows, uint32_t *pitch);

/*
 * A w x h window as a view into the framebuffer, at its (0,0) with its pitch
 * (format: the firmware's pixel order, with GLES_NATIVE_WINDOW_SCANOUT). Once.
 * 0; -EINVAL for a zero size; -ENODEV without a screen or without an answered
 * pixel order; -EFBIG when the window is wider than the granted screen or
 * taller than its rows in memory (refused by name, never clipped); -EINVAL
 * for any other window that is not a view; -ENOMEM on a second call. *out is
 * zeroed on error.
 */
int rpi5_window(uint32_t w, uint32_t h, struct board_window *out);

/* Show a window of rpi5_window(), or a part of it at a row boundary: the
 * screen is panned to its first row (SET_VIRTUAL_OFFSET; no call when it
 * starts there). -ENODEV without a screen, -EINVAL for a window that is not
 * a part of the view (nothing is copied), -EIO when the firmware did not
 * take the pan. */
int rpi5_show(const struct board_window *win);

#endif
