/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_log_check.c - host check of src/osal/osal_log.h, the line buffer the
 * Zephyr OSALs' log() formats into (zephyr.c, zephyr_gc.c, zephyr_v3d.c; no
 * host build reaches them). vsnprintf stands in for Zephyr's vsnprintk: the
 * two have the same contract (write at most `size` bytes, NUL included).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Iinclude -Isrc/osal \
 *      tools/osal/osal_log_check.c -o build/osal-check/osal_log_check \
 *      && build/osal-check/osal_log_check
 *
 * The shapes:
 * 1. The buffer is OSAL_LOG_LINE_MAX bytes, the size glog! formats in.
 * 2. The longest line the core makes (OSAL_LOG_LINE_MAX - 1 bytes) comes out
 *    whole. The 192-byte buffer the Zephyr OSALs had is asserted to cut the
 *    same line, so the case is a real one.
 * 3. The PowerVR depth/stencil refusal line (src/ds_retention.rs, as the
 *    AM62P's Linux harness printed it in batch 81: 201 bytes) comes out
 *    whole, ending in "(GL_OUT_OF_MEMORY)". The old buffer kept 191 bytes,
 *    which end in "(GL_OUT_": the cut line the Zephyr image printed.
 * 4. A longer line is cut to OSAL_LOG_LINE_MAX - 1 bytes and NUL-terminated.
 * 5. A formatter that writes nothing still leaves an empty string.
 */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "osal_log.h"

#define PLANNED 11
#define OLD_ZEPHYR_BUF 192   /* src/osal/zephyr.c:438 before batch 83 */

static int checks, failures;

static void check(int ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("osal_log_check: FAIL %s\n", what);
	}
}

/* What zephyr.c's z_log does, with vsnprintf for vsnprintk. */
static const char *log_line(struct osal_log_line *line, const char *fmt, ...)
{
	const char *text;
	va_list ap;

	va_start(ap, fmt);
	text = osal_log_format(line, vsnprintf, fmt, ap);
	va_end(ap);
	return text;
}

/* The same through the old 192-byte buffer. */
static size_t old_len(const char *fmt, ...)
{
	char buf[OLD_ZEPHYR_BUF];
	va_list ap;

	va_start(ap, fmt);
	vsnprintf(buf, sizeof(buf), fmt, ap);
	va_end(ap);
	return strlen(buf);
}

static int write_nothing(char *buf, size_t size, const char *fmt, va_list ap)
{
	(void)buf;
	(void)size;
	(void)fmt;
	(void)ap;
	return -1;
}

static const char *nothing_line(struct osal_log_line *line, const char *fmt, ...)
{
	const char *text;
	va_list ap;

	va_start(ap, fmt);
	text = osal_log_format(line, write_nothing, fmt, ap);
	va_end(ap);
	return text;
}

int main(void)
{
	static const char refusal[] =
		"yakogl/pvr: depth/stencil refused for object 0x23: all 8 depth/stencil "
		"buffers hold contents GL keeps (none deleted, none cleared whole since its "
		"last draw); the draw is not recorded (GL_OUT_OF_MEMORY)";
	struct osal_log_line line;
	char longest[OSAL_LOG_LINE_MAX];      /* OSAL_LOG_LINE_MAX - 1 bytes + NUL */
	char too_long[OSAL_LOG_LINE_MAX + 64];
	const char *text;

	/* 1. the size */
	check(sizeof(line.s) == OSAL_LOG_LINE_MAX, "line buffer is not OSAL_LOG_LINE_MAX bytes");

	/* 2. the longest core line */
	memset(longest, 'a', sizeof(longest) - 1);
	longest[sizeof(longest) - 1] = '\0';
	text = log_line(&line, "%s", longest);
	check(strlen(text) == OSAL_LOG_LINE_MAX - 1, "a 255-byte line lost bytes");
	check(strcmp(text, longest) == 0, "a 255-byte line changed");
	check(old_len("%s", longest) == OLD_ZEPHYR_BUF - 1, "the old buffer did not cut a 255-byte line");

	/* 3. the refusal line the AM62P cut */
	check(sizeof(refusal) - 1 > OLD_ZEPHYR_BUF - 1 && sizeof(refusal) - 1 < OSAL_LOG_LINE_MAX,
	      "the refusal line is not between the old and the new size");
	text = log_line(&line, "%s", refusal);
	check(strcmp(text, refusal) == 0, "the refusal line changed");
	check(strlen(text) >= 18 && strcmp(text + strlen(text) - 18, "(GL_OUT_OF_MEMORY)") == 0,
	      "the refusal line lost its (GL_OUT_OF_MEMORY)");
	check(old_len("%s", refusal) < sizeof(refusal) - 1, "the old buffer did not cut the refusal line");

	/* 4. longer than the core ever makes: cut, terminated */
	memset(too_long, 'b', sizeof(too_long) - 1);
	too_long[sizeof(too_long) - 1] = '\0';
	text = log_line(&line, "%s", too_long);
	check(strlen(text) == OSAL_LOG_LINE_MAX - 1, "a long line was not cut to 255 bytes");
	check(strncmp(text, too_long, OSAL_LOG_LINE_MAX - 1) == 0, "a long line's first 255 bytes changed");

	/* 5. a formatter that writes nothing */
	memset(line.s, 'x', sizeof(line.s));
	text = nothing_line(&line, "%s", "ignored");
	check(text[0] == '\0', "a formatter that wrote nothing left garbage");

	printf("osal_log_check: %d checks run (planned %d), %d failed\n", checks, PLANNED, failures);
	return failures || checks != PLANNED;
}
