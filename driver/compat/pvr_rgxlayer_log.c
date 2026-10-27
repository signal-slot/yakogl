/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_rgxlayer_log.c -- RGXCommentLog and RGXErrorLog of the DDK's rgxlayer.h,
 * the two functions of that layer that are C-variadic (printf-style). Stable
 * Rust cannot define a variadic function, so these two are C; every other
 * rgxlayer.h function is driver/rust/src/gpu/pvr/rgxlayer.rs.
 *
 * Both format into a line and hand it to the log of the context's struct
 * yd_env: a comment at debug level (the DDK's own writes comments only into a
 * PDump), an error at error level (the DDK's: PVR_DPF(PVR_DBG_ERROR)). The
 * DDK's image utilities pass a NULL hPrivate in a few places; those lines are
 * dropped.
 */
#include <stdarg.h>
#include <stdio.h>

#include "pvr_ddk.h"
#include "rgxlayer.h"
#include <yakogl_drivers.h>

#define RGX_LOG_LINE 160 /* env::LINE_MAX on the Rust side */

static void rgx_vlog(const void *hPrivate, uint32_t level, const IMG_CHAR *fmt, va_list ap)
{
	const struct yd_pvr_layer *l = hPrivate;
	char line[RGX_LOG_LINE + 1];
	int n;

	if (l == NULL || l->env == NULL) {
		return;
	}
	n = vsnprintf(line, sizeof(line), fmt, ap);
	if (n < 0) {
		return;
	}
	if ((size_t)n >= sizeof(line)) {
		n = (int)sizeof(line) - 1;
	}
	while (n > 0 && line[n - 1] == '\n') {
		n--;
	}
	l->env->log(l->env->ctx, level, (const uint8_t *)line, (size_t)n);
}

void RGXCommentLog(const void *hPrivate, const IMG_CHAR *pszString, ...)
{
	va_list ap;

	va_start(ap, pszString);
	rgx_vlog(hPrivate, 4 /* debug */, pszString, ap);
	va_end(ap);
}

void RGXErrorLog(const void *hPrivate, const IMG_CHAR *pszString, ...)
{
	va_list ap;

	va_start(ap, pszString);
	rgx_vlog(hPrivate, 1 /* error */, pszString, ap);
	va_end(ap);
}
