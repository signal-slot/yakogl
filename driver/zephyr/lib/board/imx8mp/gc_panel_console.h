/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_panel_console.h -- the console of last resort on the Verdin iMX8M Plus.
 *
 * The Mallow carrier leaves the debug header X11 unassembled, so a Zephyr
 * image kexec'd out of Torizon prints nowhere: there is no reachable UART.
 * Every other bulk sink we measured on this board is gone by the time Zephyr
 * runs (SRC_GPR8/9 belong to TF-A, OCRAM is rewritten by ATF and U-Boot, and
 * DDR does not survive the SoC reset either). The one exception is 16 bytes:
 * the SNVS LPGPR do keep what is written to them across the reset (checked
 * 2026-09-23; an earlier note here said they ignore writes, which was wrong),
 * and gc_result.h uses them for a result record.
 *
 * What does survive is the picture. Linux leaves the LCDIFv3 at 0x32e80000
 * scanning a linear XRGB8888 framebuffer out to the 10-inch DSI panel, and it
 * keeps scanning it after the kexec jump. This driver reads that scan-out
 * configuration, puts a framebuffer of its own in front of it, and renders
 * printk() into it with a bitmap font. A photograph of the panel is then the
 * console log.
 *
 * It installs itself through __printk_hook_install() at PRE_KERNEL_1, so
 * nothing has to call anything here; the header exists for the banner and for
 * the geometry, which a sample may want to report.
 *
 * Compile-tested only. No i.MX8MP board has run this.
 */
#ifndef GC_PANEL_CONSOLE_H
#define GC_PANEL_CONSOLE_H

#include <stdbool.h>
#include <stdint.h>

/* True once the console is rendering. False means the LCDIF was not scanning
 * anything we understood, and printk() output goes nowhere. */
#ifdef CONFIG_GC_COMMON_PANEL_CONSOLE
bool gc_panel_console_ready(void);

/* Paint slot 0..7 of the bar along the bottom edge in 0xRRGGBB: the stage a
 * boot reached, readable from across the room. No-op without a console. */
void gc_panel_console_stage(unsigned int slot, uint32_t rgb);

/* The text grid, 0 when the console is not ready. */
uint16_t gc_panel_console_cols(void);
uint16_t gc_panel_console_rows(void);

/* The surface the console renders into, so a sample can draw on the same
 * picture. The caller must clean its writes out of the data cache itself
 * (sys_cache_data_flush_range); the LCDIF reads DDR directly. */
struct gc_panel_surface {
	uint8_t *fb;
	uint32_t width;
	uint32_t height;
	uint32_t pitch;      /* bytes per scan line */
	uint32_t bytes_pp;
};

/* False (and *s untouched) when the console is not rendering. */
bool gc_panel_console_surface(struct gc_panel_surface *s);

#else
/* No panel console in this image: everything that would draw is a no-op. */
static inline bool gc_panel_console_ready(void) { return false; }
static inline void gc_panel_console_stage(unsigned int slot, uint32_t rgb)
{
	(void)slot; (void)rgb;
}
static inline uint16_t gc_panel_console_cols(void) { return 0; }
static inline uint16_t gc_panel_console_rows(void) { return 0; }
#endif /* CONFIG_GC_COMMON_PANEL_CONSOLE */
#endif /* GC_PANEL_CONSOLE_H */
