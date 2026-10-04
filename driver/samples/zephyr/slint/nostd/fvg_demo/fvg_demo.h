/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* fvg_demo.h - C entry point for the no_std FemtoVG demo staticlib. */
#ifndef FVG_DEMO_H
#define FVG_DEMO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Get-proc-address hook, EGL/GLES style: given a NUL-terminated GL entry-point
 * name, return its address, or NULL if unknown.
 */
typedef const void *(*fvg_get_proc_fn)(const char *name);

/*
 * Draw one FemtoVG frame through the caller's *current* GL ES 2.0 context.
 *
 * The caller must have made a GL context current before calling. `width` and
 * `height` are the drawable size in pixels. Draws a dark clear, a filled
 * rounded rect, a linear-gradient rect and a stroked path, then flushes.
 *
 * Returns 0 on success, non-zero on failure.
 */
int32_t fvg_draw_frame(fvg_get_proc_fn get_proc, uint32_t width, uint32_t height);

#ifdef __cplusplus
}
#endif

#endif /* FVG_DEMO_H */
