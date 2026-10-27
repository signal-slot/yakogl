/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * CONFIG_V3D_DEQP_QPA_UART: dEQP's whole QPA log to the UART, for a filter
 * of a few cases whose pixels decide a question.
 *
 * The deqp sample's fopen wrapper (deqp_log.c) gives dEQP a picolibc stream
 * from fdevopen(qpa_put, ...), and qpa_put keeps one line per case. Linked
 * with --wrap=fdevopen, the stream's put function becomes a tee: each
 * character goes to the UART, then to qpa_put as before. deqp_log.c is not
 * changed. There is one such stream (deqp.qpa), opened once.
 *
 * The deqp sample's command line also says --deqp-log-images=disable and
 * --deqp-log-shader-sources=disable, which dEQP turns into the flags of
 * qpTestLog_createFileLog(). Linked with --wrap=qpTestLog_createFileLog,
 * this file clears those two flags, so the QPA carries the result, reference
 * and error-mask images and the shader sources: what a copy of it is for.
 */
#include <stdint.h>
#include <stdio.h>

/* v3d_board.c: a character to the UART only */
void v3d_board_uart_putc(char c);

FILE *__real_fdevopen(int (*put)(char, FILE *), int (*get)(FILE *), int (*flush)(FILE *));

static int (*inner_put)(char, FILE *);

static int tee_put(char c, FILE *f)
{
	v3d_board_uart_putc(c);
	return inner_put ? inner_put(c, f) : (unsigned char)c;
}

FILE *__wrap_fdevopen(int (*put)(char, FILE *), int (*get)(FILE *), int (*flush)(FILE *))
{
	if (!put || inner_put)
		return __real_fdevopen(put, get, flush); /* not a log stream, or a second one */
	inner_put = put;
	return __real_fdevopen(tee_put, get, flush);
}

/* qpTestLog.h's qpTestLogFlag values: the two this image wants off */
#define QPA_EXCLUDE_IMAGES         (1u << 0)
#define QPA_EXCLUDE_SHADER_SOURCES (1u << 1)

typedef struct qpTestLog_s qpTestLog;
qpTestLog *__real_qpTestLog_createFileLog(const char *fileName, uint32_t flags);
qpTestLog *__wrap_qpTestLog_createFileLog(const char *fileName, uint32_t flags)
{
	return __real_qpTestLog_createFileLog(fileName,
					      flags & ~(QPA_EXCLUDE_IMAGES | QPA_EXCLUDE_SHADER_SOURCES));
}
