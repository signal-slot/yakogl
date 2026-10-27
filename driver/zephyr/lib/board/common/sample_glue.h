/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Board glue shared by the Zephyr samples (samples/zephyr/common). */
#ifndef YAKOGL_SAMPLE_GLUE_H
#define YAKOGL_SAMPLE_GLUE_H

#include <stdint.h>
#include <stddef.h>

/* Scan an XRGB8888 buffer out as the DSS application plane
 * (CONFIG_YAKOGL_SAMPLE_SHOW_ON_DSS); without the DSS driver it returns 0 and
 * does nothing. */
int pvr_sample_show_on_dss(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes);

/* Re-point the application plane at another buffer of the same geometry
 * (double buffering: call after the frame in `phys` is complete). */
/* Vertical syncs seen by the display so far (0 without a display): a plane
 * flip takes effect at the next one, so a buffer flipped away is on screen
 * until the count advances. */
uint32_t pvr_sample_vsync_count(void);
int pvr_sample_flip_app_buffer(uintptr_t phys, uint32_t width, uint32_t height,
			       uint32_t stride_bytes);
/* What to OR into a GL window's format (struct gles_native_window.format):
 * GLES_NATIVE_WINDOW_BOTTOM_UP when the display can scan the buffer bottom row
 * first (the DSS driver with AM62P_DSS_PLANE_VFLIP), so GL draws the
 * window y-up as it draws an FBO; 0 otherwise. */
uint32_t pvr_sample_window_flags(void);
/* pvr_sample_show_on_dss / pvr_sample_flip_app_buffer for a GL window: the
 * scan order follows the window's own format (its GLES_NATIVE_WINDOW_BOTTOM_UP
 * bit), so the plane and the GL surface cannot disagree. -ENOTSUP for a
 * bottom-up window on a display that cannot scan one. */
int pvr_sample_show_window(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes, uint32_t window_format);
int pvr_sample_flip_window(uintptr_t phys, uint32_t width, uint32_t height,
			   uint32_t stride_bytes, uint32_t window_format);
/* Which of the n windows at phys[] the application plane may still scan,
 * bit k for phys[k] (am62p_dss_app_busy); 0 without the DSS. */
uint32_t pvr_sample_window_busy(const uint64_t *phys, size_t n);

/* Clean a buffer out of the CPU's data cache, so the GPU and the display
 * controller see what the CPU wrote. Zephyr's own cache calls are inline C,
 * which a Rust application cannot reach; this is that call by another name. */
void pvr_sample_flush_dcache(void *addr, size_t size);

/* Keep the picture for CONFIG_YAKOGL_SAMPLE_HOLD_SECONDS, then reset the SoC
 * when CONFIG_YAKOGL_SAMPLE_REBOOT is set. Returns rc otherwise. With
 * CONFIG_YAKOGL_SAMPLE_DISPLAY_FILL (AM62P, diagnosis) the DSS safety layer is
 * first filled with that opaque colour and the port enabled, so the panel
 * must show it whatever the application plane carries. */
int pvr_sample_finish(int rc);

/* The two halves of pvr_sample_finish around its hold, for a run that holds
 * elsewhere (the board layer's board_finish): before, the known-colour fill
 * and the DSS counters ("before hold", with the register dump when
 * configured); after, the counters again. Nothing without the DSS driver. */
void pvr_sample_before_hold(void);
void pvr_sample_after_hold(void);

/* CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP (AM62P): the display path's registers on
 * the console in the Linux dump's format (am62p/display_dump.c). */
void pvr_sample_display_dump(const char *when);

#endif
