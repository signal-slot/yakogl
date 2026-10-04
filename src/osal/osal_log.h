/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_log.h - the one line buffer of an OSAL log() that formats before it
 * prints (zephyr.c, zephyr_gc.c, zephyr_v3d.c hand the finished line to
 * LOG_INF). It is OSAL_LOG_LINE_MAX bytes (include/gles_osal.h), the size
 * the core formats its own lines in, so a core line reaches the log whole.
 * Plain C with no OS in it, so the host checks it
 * (tools/osal/osal_log_check.c).
 */
#ifndef OSAL_LOG_H
#define OSAL_LOG_H

#include <stdarg.h>
#include <stddef.h>

#include "gles_osal.h"

struct osal_log_line {
	char s[OSAL_LOG_LINE_MAX];
};

/* The formatter: vsnprintk on Zephyr, vsnprintf on the host. Both write at
 * most `size` bytes, NUL included. */
typedef int (*osal_vfmt_fn)(char *buf, size_t size, const char *fmt, va_list ap);

#if defined(__ZEPHYR__)
#include <zephyr/sys/printk.h>
/* Zephyr's formatter as a function: some configurations define vsnprintk as
 * a function-like macro (sys/printk.h), which cannot be passed by address. */
static inline int osal_vsnprintk(char *buf, size_t size, const char *fmt, va_list ap)
{
	return vsnprintk(buf, size, fmt, ap);
}
#endif

/* Format one line into `line` and return its text (always NUL-terminated). */
static inline const char *osal_log_format(struct osal_log_line *line, osal_vfmt_fn vfmt,
					  const char *fmt, va_list ap)
{
	line->s[0] = '\0';
	vfmt(line->s, sizeof(line->s), fmt, ap);
	line->s[sizeof(line->s) - 1] = '\0';
	return line->s;
}

#endif /* OSAL_LOG_H */
