/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dEQP writes its test log (QPA) with fopen(); the board has no file system.
 * --wrap=fopen hands dEQP a picolibc stream (fdevopen) for the log name, and
 * that stream never stores the log: it reads it line by line and turns each
 * case into one result line on the console,
 *
 *     deqp: <status> <case name>
 *
 * from "#beginTestCaseResult <name>" and the case's <Result StatusCode="..">.
 * A case that begins and never reaches a result (a crash, a hang the watchdog
 * ends) has no line, and the summary's count says so: absence is its own
 * outcome, never folded into pass or fail.
 */
#include <stdio.h>
#include <string.h>
#include <zephyr/sys/printk.h>
#ifdef CONFIG_DEQP_QUIET_STAMPS
#include <zephyr/kernel.h>
#endif
#ifdef CONFIG_YAKOGL_GPU_MALI
/* A Mali board's watchdog (the NanoPC-T6's WDT0, the Verdin iMX95's WDOG3)
 * is fed only while the run says it makes progress: a case that begins or
 * ends does (src/mali_board.c). osal_zephyr_mali_board_feed() is the board
 * glue's feed, the one the OSAL calls in its waits. */
#include "zephyr_mali_board.h"
#endif

#define DEQP_LOG_NAME "deqp.qpa"

static char line[512];
static size_t len;
static char cur_case[256];
static unsigned int n_begun, n_result;

#ifdef CONFIG_DEQP_QUIET_STAMPS
/* Quiet console: no per-case line, a progress stamp every
 * CONFIG_DEQP_STAMP_EVERY finished cases and at the end. The format is
 * documented in tools/cts/mp2-case-times.py. */
static unsigned int n_pass, n_fail, n_other;
static long long last_stamp_done = -1;

static void print_stamp(void)
{
	printk("deqp: stamp uptime_ms=%llu case=%u done=%u pass=%u fail=%u other=%u\n",
	       (unsigned long long)k_uptime_get(), n_begun ? n_begun - 1 : 0, n_result, n_pass,
	       n_fail, n_other);
	last_stamp_done = n_result;
}
#endif

#ifdef CONFIG_GC_COMMON_RESULT_SNVS
/* deqp_snvs.c: the board keeps a running tally where a reset cannot lose it
 * (a status of NULL: a case began). Absent from the other boards' builds. */
void deqp_snvs_progress(const char *status, const char *name);
#endif

#ifdef CONFIG_DEQP_ECHO_QPA
/* 1 while the next piece continues a line longer than the buffer */
static int echo_cont;
#endif

static void take_line(void)
{
	static const char begin[] = "#beginTestCaseResult ";
	static const char result[] = "<Result StatusCode=\"";

	line[len] = 0;
#ifdef CONFIG_DEQP_ECHO_QPA
	/* "qpa: " starts a log line, "qpa+ " continues one the buffer split */
	printk("%s%s\n", echo_cont ? "qpa+ " : "qpa: ", line);
#endif
	if (strncmp(line, begin, sizeof(begin) - 1) == 0) {
		strncpy(cur_case, line + sizeof(begin) - 1, sizeof(cur_case) - 1);
		cur_case[sizeof(cur_case) - 1] = 0;
		n_begun++;
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
		deqp_snvs_progress(NULL, cur_case);
#endif
#ifdef CONFIG_YAKOGL_GPU_MALI
		osal_zephyr_mali_board_feed();
#endif
		return;
	}
	/* The info group's answers (GL_VERSION, GL_VENDOR, ...) are the only
	 * text worth a console line: they say which version the image reports,
	 * and nothing else shows it on a board without a file system. */
	if (strncmp(cur_case, "dEQP-GLES", 9) == 0 && strstr(cur_case, ".info.") != NULL) {
		const char *t = strstr(line, "<Text>");
		const char *e = t ? strstr(t, "</Text>") : NULL;

		if (t && e) {
			printk("deqp:   %.*s\n", (int)(e - t - 6), t + 6);
		}
	}
	const char *r = strstr(line, result);

	if (r && cur_case[0]) {
		char status[32];
		size_t i = 0;

		r += sizeof(result) - 1;
		while (r[i] && r[i] != '"' && i < sizeof(status) - 1) {
			status[i] = r[i];
			i++;
		}
		status[i] = 0;
#ifdef CONFIG_DEQP_QUIET_STAMPS
		n_result++;
		if (strcmp(status, "Pass") == 0)
			n_pass++;
		else if (strcmp(status, "Fail") == 0)
			n_fail++;
		else
			n_other++;
		if (n_result % CONFIG_DEQP_STAMP_EVERY == 0)
			print_stamp();
#else
		printk("deqp: %s %s\n", status, cur_case);
		n_result++;
#endif
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
		deqp_snvs_progress(status, cur_case);
#endif
#ifdef CONFIG_YAKOGL_GPU_MALI
		osal_zephyr_mali_board_feed();
#endif
		cur_case[0] = 0;
	}
}

static int qpa_put(char c, FILE *f)
{
	(void)f;
	if (c == '\n' || len == sizeof(line) - 1) {
		take_line();
		len = 0;
#ifdef CONFIG_DEQP_ECHO_QPA
		echo_cont = c != '\n';
#endif
		if (c == '\n')
			return (unsigned char)c;
	}
	line[len++] = c;
	return (unsigned char)c;
}

static int qpa_flush(FILE *f)
{
	(void)f;
	return 0;
}

FILE *__real_fopen(const char *path, const char *mode);
FILE *__wrap_fopen(const char *path, const char *mode)
{
	if (path && strcmp(path, DEQP_LOG_NAME) == 0)
		return fdevopen(qpa_put, NULL, qpa_flush);
	return __real_fopen(path, mode);
}

void deqp_log_summary(void)
{
#ifdef CONFIG_DEQP_QUIET_STAMPS
	if (last_stamp_done != (long long)n_result)
		print_stamp();
#endif
	printk("deqp: summary: %u cases begun, %u with a result, %u without\n", n_begun, n_result,
	       n_begun - n_result);
}
