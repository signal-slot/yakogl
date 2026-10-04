/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A printk console drawn into the firmware framebuffer, for a Raspberry Pi 4
 * whose only output we can read is HDMI (on the lab's capture card). The same
 * idea as the i.MX8MP panel console (driver/zephyr/lib/board/imx8mp), which reads
 * the LCDIF and cannot be reused here, with the same font.
 */
#ifndef FBCON_H
#define FBCON_H

#include <stdint.h>

/* Render printk() into rows [0, rows_px) of the framebuffer from now on. */
void fbcon_attach(uint8_t *fb, uint32_t width, uint32_t pitch, uint32_t rows_px,
		  uint32_t scale);

#endif
