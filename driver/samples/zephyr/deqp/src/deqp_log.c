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

#define DEQP_LOG_NAME "deqp.qpa"

static char line[512];
static size_t len;
static char cur_case[256];
static unsigned int n_begun, n_result;

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
		printk("deqp: %s %s\n", status, cur_case);
		n_result++;
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
		deqp_snvs_progress(status, cur_case);
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
	printk("deqp: summary: %u cases begun, %u with a result, %u without\n", n_begun, n_result,
	       n_begun - n_result);
}
