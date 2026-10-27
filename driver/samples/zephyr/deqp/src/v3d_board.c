/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_deqp's board side: what the deqp sample (../../deqp/src, used as is)
 * needs on a Raspberry Pi 4, and the result record the lead photographs.
 *
 *  - The HDMI console: the firmware framebuffer (1280x720) through the
 *    mailbox, printk drawn into it (driver/zephyr/lib/board/rpi4/fbcon.c). Set up
 *    before main(), so the run's lines show from the first case.
 *  - The EGL default display. dEQP's platform asks for EGL_DEFAULT_DISPLAY;
 *    YakoGL hands the native display to the OSAL's init as is, and the V3D
 *    OSAL needs the board's V3D clock switch there (struct
 *    osal_zephyr_v3d_init). --wrap=eglGetDisplay (CMakeLists.txt) gives it:
 *    on this board the default display is the V3D with this clock switch.
 *  - The record. Every printk line passes through tap(): the deqp sample's
 *    "deqp: <StatusCode> <case>" lines (deqp_log.c) become one letter per
 *    case, and YakoGL's refusal lines ("yakogl/v3d: draw refused: <why>",
 *    "yakogl: ... was not recorded") are tied to the case they appeared in
 *    (the lines between two verdict lines belong to the second case). At the
 *    end (pvr_sample_finish, which the deqp main calls), at the photo
 *    deadline if the run is still going, or on a fatal error, the screen is
 *    cleared and the record printed once, then frozen until the watchdog.
 *    dEQP's own stdout and stderr go to the UART only; of them the record
 *    keeps the "FATAL ERROR: ..." line of a run that dEQP gave up on, and
 *    the reason dEQP prints for each case, "  <Status> (<reason>)", which
 *    comes right after that case's verdict line (tcuTestSessionExecutor.cpp
 *    leaveTestCase: endCase, then print).
 *
 * For a shader that does not compile or link, dEQP's reason is only "Compile
 * failed"; the compiler's own message goes to the QPA log. dEQP reads it with
 * glGetShaderInfoLog / glGetProgramInfoLog after every compile and link
 * (gluShaderProgram.cpp), right after it asks for GL_COMPILE_STATUS or
 * GL_LINK_STATUS. --wrap on those four (CMakeLists.txt) keeps the first line
 * of the log of a shader or program whose status query answered GL_FALSE,
 * and only that: a warning from a build that succeeded is never kept. A case
 * that did not pass and in which a build failed shows that line ("cc: ...",
 * "cc(vs): ..." for YakoGL's refusal of a vertex shader) in place of dEQP's
 * reason, whatever dEQP's reason says ("Compile failed", "could not create
 * shader", "glGetError() returned GL_INVALID_OPERATION" after a draw with
 * the program that did not link).
 *
 * When more cases fail than the screen can name, the record still accounts for
 * all of them: the cases that did not pass are counted by group (the name
 * without its last part), with the letters and the first reason in each.
 *
 * A refusal is its own outcome: a case that did not pass and during which
 * the V3D backend refused a draw is R, never F. Whether it did is read from
 * the backend's own count, v3d_refused_draws() (src/v3d/backend.rs), taken
 * at every verdict: the count grew across the case. The log's refusal lines
 * ("yakogl/v3d: draw refused: <why>", once per cause per boot) only name the
 * causes; a refused case whose cause was logged in an earlier case is
 * tagged "r?".
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/fatal.h>
#include <zephyr/sys/atomic.h>
#include <zephyr/sys/libc-hooks.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/arch/arm64/lib_helpers.h> /* read_far_el1 */
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include "osal_internal.h" /* struct osal_zephyr_v3d_init */
#include "rpi_mbox.h"
#include "fbcon.h"
#include "rpi_wdog.h"
#include "sample_glue.h"

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define FB_W 1280
#define FB_H 720
/* The 8x16 font at scale 1: 45 rows of 160 columns. fbcon clears the row
 * after each newline, so a record uses 44 of them and the top one stays. */
#define ROWS (FB_H / 16 - 1)
#define NAME_COLS 48   /* a listed name */
#define TAG_COLS 32    /* its time, keep and carve-out figures, refusal causes */
#define REASON_COLS 70 /* its reason: 89 columns come before it, 159 in all */

#define MAX_CASES      12288 /* one letter each; the largest group is ~10k */
#define MAX_LISTED     256   /* cases that did not pass, kept by name */
#define NAME_LEN       96
#define REASON_LEN     100
/* A compiler line is kept whole (its first line, up to this), because what
 * it says last is the part that matters: "... refused this fragment shader:
 * Program(NotYet(..))". The record shows its tail when it is too long. */
#define INFO_LEN       240
#define MAX_GROUPS     24    /* groups of cases that did not pass */
#define MAX_BUCKETS    4     /* distinct reasons counted per group */
#define BUCKET_LEN     96    /* a reason's tail, where its cause is */
#define GROUP_LEN      64
#define MAX_CAUSES     12
#define CAUSE_LEN      120
#define MAX_DRIVER     3     /* other driver lines kept verbatim */
#define MAX_FAULT      6     /* MMU-fault and overwrite lines, kept whole */
#define FAULT_LEN      320
#define TEXT_LEN       150
#define PER_ROW        128   /* letters per record row */

/* ---- the V3D's EGL default display --------------------------------------- */

static int clk_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	int ret = rpi_clock_state(RPI_FW_CLK_V3D, on);

	if (ret)
		printk("v3d_deqp: fw clock 5 %s: %d (not fatal)\n", on ? "on" : "off", ret);
	return 0; /* as v3d_gl_triangle: which clock the toggle needs is a board question */
}

static struct osal_zephyr_v3d_init plat = { .clock_set = clk_set };

/* src/v3d/backend.rs: draws refused since init; wraps, so read differences. */
extern uint32_t v3d_refused_draws(void);
/* The online compiler's keep region (src/shaderc/online/store.rs): bytes
 * held now and the most ever held. Read at every verdict, so the record can
 * set the region's use beside the cases that did not pass. */
extern int shc_online_keep_stats(size_t *in_use, size_t *peak);
/* The V3D OSAL's GPU memory, the carve-out (src/osal/zephyr_v3d.c): bytes
 * used now, the most at once, the total. An OSAL older than that accessor
 * links this weak stand-in instead, and the record says it has no figures. */
static bool carve_known = true;

__weak void osal_v3d_mem_used(uint32_t *used, uint32_t *high, uint32_t *total)
{
	carve_known = false;
	if (used)
		*used = 0;
	if (high)
		*high = 0;
	if (total)
		*total = 0;
}

EGLDisplay __real_eglGetDisplay(EGLNativeDisplayType id);
EGLDisplay __wrap_eglGetDisplay(EGLNativeDisplayType id)
{
	return __real_eglGetDisplay(id == EGL_DEFAULT_DISPLAY ? (EGLNativeDisplayType)&plat : id);
}

EGLDisplay __real_eglGetPlatformDisplay(EGLenum platform, void *native, const EGLAttrib *attribs);
EGLDisplay __wrap_eglGetPlatformDisplay(EGLenum platform, void *native, const EGLAttrib *attribs)
{
	return __real_eglGetPlatformDisplay(platform, native ? native : (void *)&plat, attribs);
}

/* ---- the compiler's message ------------------------------------------------ */

/* The first line of the first failed build's log in the case in progress. */
static char info_line[INFO_LEN];
/* The shader and program whose status query last answered GL_FALSE (0: none):
 * only their logs are kept. */
static GLuint failed_shader, failed_program;

static void take_info(const GLchar *log, GLsizei len)
{
	GLsizei n = 0;

	if (!log || len <= 0 || info_line[0])
		return; /* nothing, or the case already has its first message */
	/* YakoGL's refusal line (src/shaderc/online/backend.rs:437) is
	 * "online shaderc: the code generator refused this <stage> shader:
	 * <error>". A row has room for the error, so it becomes
	 * "cc(fs): <error>" or "cc(vs): <error>". Any other line is kept as is. */
	static const char fixed[] = "online shaderc: the code generator refused this ";
	static const struct {
		const char *stage, *tag;
	} stages[] = { { "fragment shader: ", "cc(fs): " }, { "vertex shader: ", "cc(vs): " } };
	const char *tag = "cc: ";

	if (len > (GLsizei)(sizeof(fixed) - 1) && strncmp(log, fixed, sizeof(fixed) - 1) == 0) {
		for (size_t i = 0; i < ARRAY_SIZE(stages); i++) {
			GLsizei k = (GLsizei)strlen(stages[i].stage);

			if (len >= (GLsizei)(sizeof(fixed) - 1) + k &&
			    strncmp(log + sizeof(fixed) - 1, stages[i].stage, (size_t)k) == 0) {
				log += sizeof(fixed) - 1 + (size_t)k;
				len -= (GLsizei)(sizeof(fixed) - 1) + k;
				tag = stages[i].tag;
				break;
			}
		}
	}
	while (n < len && log[n] && log[n] != '\n')
		n++;
	if (n)
		snprintf(info_line, sizeof(info_line), "%s%.*s", tag, (int)n, log);
}

void __real_glGetShaderiv(GLuint shader, GLenum pname, GLint *params);
void __wrap_glGetShaderiv(GLuint shader, GLenum pname, GLint *params)
{
	__real_glGetShaderiv(shader, pname, params);
	if (pname == GL_COMPILE_STATUS && params)
		failed_shader = *params == GL_FALSE ? shader : 0;
}

void __real_glGetProgramiv(GLuint program, GLenum pname, GLint *params);
void __wrap_glGetProgramiv(GLuint program, GLenum pname, GLint *params)
{
	__real_glGetProgramiv(program, pname, params);
	if (pname == GL_LINK_STATUS && params)
		failed_program = *params == GL_FALSE ? program : 0;
}

void __real_glGetShaderInfoLog(GLuint shader, GLsizei size, GLsizei *length, GLchar *log);
void __wrap_glGetShaderInfoLog(GLuint shader, GLsizei size, GLsizei *length, GLchar *log)
{
	GLsizei got = 0;

	__real_glGetShaderInfoLog(shader, size, &got, log);
	if (length)
		*length = got;
	if (shader && shader == failed_shader)
		take_info(log, got);
}

void __real_glGetProgramInfoLog(GLuint program, GLsizei size, GLsizei *length, GLchar *log);
void __wrap_glGetProgramInfoLog(GLuint program, GLsizei size, GLsizei *length, GLchar *log)
{
	GLsizei got = 0;

	__real_glGetProgramInfoLog(program, size, &got, log);
	if (length)
		*length = got;
	if (program && program == failed_program)
		take_info(log, got);
}

/* ---- the tally ------------------------------------------------------------- */

/* dEQP's status codes (framework/qphelper/qpTestLog.c) and their letters. */
static const struct {
	const char *code;
	char letter;
} statuses[] = {
	{ "Pass", 'P' },          { "Fail", 'F' },     { "QualityWarning", 'Q' },
	{ "CompatibilityWarning", 'C' },               { "NotSupported", 'N' },
	{ "ResourceError", 'E' }, { "InternalError", 'I' }, { "Crash", 'X' },
	{ "Timeout", 'T' },       { "Pending", '?' },  { "Waiver", 'W' },
	{ "DeviceLost", 'D' },
};
#define REFUSED 'R'
/* the order the record counts them in */
static const char letters[] = "PFRNQCEIXTWD?";

struct listed {
	uint32_t index;
	char letter;
	uint16_t causes; /* bit i: cause i was logged during the case */
	uint32_t refused; /* draws the backend refused during the case */
	uint32_t keep_kib, keep_peak_kib; /* the keep region at its verdict */
	uint32_t ms; /* from the verdict before it to its own */
	uint32_t carve_kib, carve_high_kib; /* the carve-out at its verdict */
	uint16_t slow_jobs; /* its jobs the OSAL called slow */
	bool hung;          /* a job of it did not end (or timed out) */
	char name[NAME_LEN];
	char reason[REASON_LEN]; /* dEQP's result description */
	char info[INFO_LEN];     /* the compiler's first line, if any */
};

/* The cases that did not pass, by group: every one counted, whether or not
 * its name fits on the screen. */
/* One distinct reason in a group and how many of its cases gave it. A
 * reason is the compiler's line when the case's build failed, else dEQP's
 * result text; it is keyed by its tail (the cause is at the end). */
struct bucket {
	char text[BUCKET_LEN];
	uint32_t n;
	bool is_n; /* an N's reason (dEQP's own NotSupported) */
};

struct group {
	char name[GROUP_LEN];
	uint32_t n, n_letter[ARRAY_SIZE(letters) - 1];
	struct bucket b[MAX_BUCKETS];
	uint32_t n_buckets;
	uint32_t n_other; /* cases whose reason found no free bucket */
};

static struct {
	char filter[TEXT_LEN]; /* for the first row, in quotes (cut there to fit) */
	size_t prefix_len;     /* the filter's part before its last '.' ahead of the '*' */
	char verdict[MAX_CASES];
	uint32_t n_cases, n_letter[128], n_pass_refusal;
	struct listed listed[MAX_LISTED];
	uint32_t n_listed;
	struct group group[MAX_GROUPS];
	uint32_t n_groups, n_grouped_lost; /* not passed, past the group table */
	int last_group; /* the group of the case whose verdict came last, or -1 */
	char last_letter; /* and that case's letter */
	char last_info[INFO_LEN]; /* and its failed build's compiler line, if any */
	char cause[MAX_CAUSES][CAUSE_LEN];
	uint32_t cause_first[MAX_CAUSES]; /* the case that logged it */
	uint32_t n_causes, n_causes_lost;
	uint32_t refused_mark;  /* v3d_refused_draws() at the last verdict */
	uint32_t refused_draws; /* refused draws in all cases so far */
	/* The keep region, in bytes, read at each verdict: after the first case,
	 * the most held at a verdict (and which case), held at the verdict before
	 * the first case that did not pass, and the last reading. */
	size_t keep_first, keep_max, keep_before_bad, keep_last, keep_peak, keep_prev;
	uint32_t keep_max_at, first_bad;
	uint32_t slowest_ms, slowest_at; /* the longest case, verdict to verdict */
	/* the carve-out, bytes: after the first case, the last reading, the
	 * OSAL's own high-water, its total */
	uint32_t carve_first, carve_last, carve_high, carve_total;
	/* The OSAL's job lines, by case: "osal(v3d): job N ... (slow; ...)" and
	 * "... did not end ..." (an OSAL that tells slow from hung), or
	 * "osal(v3d): FIRST FAILURE: ..." (one that only reports its first
	 * timeout). Counted against the case in progress. */
	uint16_t pending_slow;
	bool pending_hung;
	uint32_t n_slow_cases, first_slow, first_hung;
	bool have_hung;
	char first_hung_line[TEXT_LEN];
	/* The driver's lines about a fault, whole: "... MMU fault ..." (the
	 * backend's texture-record and code checks, the OSAL's), the OSAL's
	 * stall line with the MMU's "vio ... vio_id ..." (the faulting address and
	 * client, read when the stage stopped: the job record's own MMU line
	 * needs the hub bits, which the wait has cleared by then) and "... written
	 * past ..." (a canary after a tiled image). The first ones, and a count. */
	char fault[MAX_FAULT][FAULT_LEN];
	uint32_t n_fault;
	char vio_line[FAULT_LEN]; /* the first stall line with vio_id: its own slot */
	bool keep_read, have_bad;
	/* since the last verdict line: the causes the case in progress logged */
	uint16_t pending_causes;
	char driver[MAX_DRIVER][TEXT_LEN];
	uint32_t n_driver;
	char done[TEXT_LEN], summary[TEXT_LEN], exception[TEXT_LEN], fatal[TEXT_LEN];
	/* The case that has begun ("Test case '<name>'..", dEQP's stdout) and has
	 * no verdict yet: the one a fatal stop or the deadline caught. */
	char current[NAME_LEN];
	int64_t t_start_ms, t_end_ms;
} t;

/* The image's case filter, whole: the deqp sample's main passes exactly
 * this (--deqp-case=CONFIG_DEQP_CASE). */
static const char case_filter[] = CONFIG_DEQP_CASE;

static char line[512];
static size_t line_len;
static printk_hook_fn_t fbcon_out, uart_out; /* uart_out: the hook before fbcon's */
enum tap_mode { TAP_RUN, TAP_RECORD, TAP_FROZEN };
static volatile enum tap_mode mode = TAP_RUN;
static atomic_t record_taken;
/* The record's screen rows, counted as fbcon moves to a new one (a newline,
 * or a character past the last column): for a block whose length the
 * record does not know, the OSAL's job record. */
static uint32_t rec_col, rec_rows;

static bool starts(const char *s, const char *prefix)
{
	return strncmp(s, prefix, strlen(prefix)) == 0;
}

static void keep(char *dst, const char *src)
{
	snprintf(dst, TEXT_LEN, "%.*s", TEXT_LEN - 1, src);
}

/* Count a case that did not pass in its group: its full name without the
 * last part, shown without the filter's prefix. A case right under the
 * prefix is in the group the prefix names ("dEQP-GLES2.a.color_clear." ->
 * "color_clear"). Returns the group's index, or -1 when the table is full. */
static int count_in_group(char letter, const char *name)
{
	const char *dot = strrchr(name, '.');
	size_t len = dot ? (size_t)(dot - name) : strlen(name);
	uint32_t g;

	if (t.prefix_len && strncmp(name, case_filter, t.prefix_len) == 0) {
		if (len >= t.prefix_len) { /* below the prefix */
			name += t.prefix_len;
			len -= t.prefix_len;
		} else { /* right under it: the prefix's last part */
			const char *start = name;

			for (const char *c = name; c < name + len; c++)
				if (*c == '.')
					start = c + 1;
			len -= (size_t)(start - name);
			name = start;
		}
	}
	if (len >= GROUP_LEN)
		len = GROUP_LEN - 1;
	for (g = 0; g < t.n_groups; g++)
		if (strlen(t.group[g].name) == len && strncmp(t.group[g].name, name, len) == 0)
			break;
	if (g == t.n_groups) {
		if (g == MAX_GROUPS) {
			t.n_grouped_lost++;
			return -1;
		}
		memcpy(t.group[g].name, name, len);
		t.group[g].name[len] = 0;
		t.n_groups++;
	}
	t.group[g].n++;
	const char *at = strchr(letters, letter);

	if (at)
		t.group[g].n_letter[at - letters]++;
	return (int)g;
}

/* One character to the UART only (qpa_uart.c's tee): the hook the console had
 * before fbcon's, so it neither reaches the screen nor passes the tap. */
void v3d_board_uart_putc(char c)
{
	if (uart_out)
		uart_out((unsigned char)c);
}

/* One line per case to the UART only, at its verdict: its index and letter,
 * its time, the keep region and the carve-out (KiB held/high-water), "H" if a
 * job of it hung (or timed out) and "S<n>" for n slow jobs, the draws the
 * backend refused in it, and its full name. It goes straight to the UART's
 * hook: this runs inside the printk hook, so printk here would recurse, and
 * the screen keeps its one-page record. */
static void uart_case_line(uint32_t i, char letter, uint32_t ms, size_t keep_now, size_t keep_peak,
			   uint32_t carve, uint32_t carve_high, uint32_t n_refused, const char *name)
{
	char b[320];
	int n;

	if (!uart_out)
		return;
	n = snprintf(b, sizeof(b), "v3d_deqp: case #%u %c %ums k%u/%u", i, letter, ms,
		     (unsigned)(keep_now / 1024), (unsigned)(keep_peak / 1024));
	if (carve_known && n > 0 && n < (int)sizeof(b))
		n += snprintf(b + n, sizeof(b) - n, " c%u/%u", carve / 1024, carve_high / 1024);
	if (t.pending_hung && n > 0 && n < (int)sizeof(b))
		n += snprintf(b + n, sizeof(b) - n, " H");
	if (t.pending_slow && n > 0 && n < (int)sizeof(b))
		n += snprintf(b + n, sizeof(b) - n, " S%u", t.pending_slow);
	if (n > 0 && n < (int)sizeof(b))
		n += snprintf(b + n, sizeof(b) - n, " r%u %.*s\n", n_refused, 240, name);
	if (n <= 0)
		return;
	if (n >= (int)sizeof(b)) { /* cut: still one line */
		n = (int)sizeof(b) - 1;
		b[n - 1] = '\n';
	}
	for (int k = 0; k < n; k++)
		uart_out((unsigned char)b[k]);
}

static void take_verdict(char letter, const char *name)
{
	uint32_t now = v3d_refused_draws();
	uint32_t n_refused = now - t.refused_mark; /* the count wraps */
	bool refused = n_refused > 0;
	uint32_t i = t.n_cases++;
	int64_t now_ms = k_uptime_get();
	uint32_t ms = (uint32_t)(now_ms - t.t_end_ms); /* since the last verdict */

	t.refused_mark = now;
	t.refused_draws += n_refused;

	size_t keep_now = 0, keep_peak = 0;

	shc_online_keep_stats(&keep_now, &keep_peak);
	if (!t.keep_read)
		t.keep_first = keep_now;
	if (!t.keep_read || keep_now > t.keep_max) {
		t.keep_max = keep_now;
		t.keep_max_at = i;
	}
	uint32_t carve = 0, carve_high = 0;

	osal_v3d_mem_used(&carve, &carve_high, &t.carve_total);
	if (!t.keep_read)
		t.carve_first = carve;
	t.carve_last = carve;
	t.carve_high = carve_high;
	t.keep_read = true;
	if (i == 0 || ms > t.slowest_ms) {
		t.slowest_ms = ms;
		t.slowest_at = i;
	}
	t.keep_last = keep_now;
	t.keep_peak = keep_peak;

	if (refused && letter != 'P')
		letter = REFUSED;
	else if (refused)
		t.n_pass_refusal++;
	t.n_letter[(unsigned char)letter & 0x7f]++;
	if (i < MAX_CASES)
		t.verdict[i] = letter;
	if (letter != 'P' && t.n_listed < MAX_LISTED) {
		struct listed *l = &t.listed[t.n_listed++];

		l->index = i;
		l->letter = letter;
		l->causes = t.pending_causes;
		l->refused = n_refused;
		l->keep_kib = (uint32_t)(keep_now / 1024);
		l->keep_peak_kib = (uint32_t)(keep_peak / 1024);
		l->ms = ms;
		l->carve_kib = carve / 1024;
		l->carve_high_kib = carve_high / 1024;
		l->slow_jobs = t.pending_slow;
		l->hung = t.pending_hung;
		/* the part the filter's prefix does not already say */
		const char *shown = name;

		if (t.prefix_len && strncmp(name, case_filter, t.prefix_len) == 0)
			shown += t.prefix_len;
		snprintf(l->name, sizeof(l->name), "%.*s", NAME_LEN - 1, shown);
	}
	if (letter != 'P' && !t.have_bad) {
		t.have_bad = true;
		t.first_bad = i;
		t.keep_before_bad = t.keep_prev; /* read at the verdict before it */
	}
	t.keep_prev = keep_now;
	t.last_group = letter == 'P' ? -1 : count_in_group(letter, name);
	t.last_letter = letter;
	if (letter != 'P' && info_line[0] && t.n_listed && t.listed[t.n_listed - 1].index == i)
		snprintf(t.listed[t.n_listed - 1].info, INFO_LEN, "%s", info_line);
	/* the reason line comes after the verdict: keep the compiler line for it */
	snprintf(t.last_info, sizeof(t.last_info), "%s", letter != 'P' ? info_line : "");
	info_line[0] = 0;
	uart_case_line(i, letter, ms, keep_now, keep_peak, carve, carve_high, n_refused, name);
	if (t.pending_slow) {
		if (!t.n_slow_cases++)
			t.first_slow = i;
	}
	if (t.pending_hung && !t.have_hung) {
		t.have_hung = true;
		t.first_hung = i;
	}
	t.pending_slow = 0;
	t.pending_hung = false;
	t.pending_causes = 0;
	t.current[0] = 0; /* the case in progress has its verdict */
	t.t_end_ms = now_ms;
}

static void take_cause(const char *why)
{
	uint32_t i;

	for (i = 0; i < t.n_causes; i++)
		if (strncmp(t.cause[i], why, CAUSE_LEN - 1) == 0)
			break;
	if (i == t.n_causes) {
		if (t.n_causes == MAX_CAUSES) {
			t.n_causes_lost++; /* the case is still R by the count */
			return;
		}
		snprintf(t.cause[i], CAUSE_LEN, "%.*s", CAUSE_LEN - 1, why);
		t.cause_first[i] = t.n_cases; /* the case in progress */
		t.n_causes++;
	}
	t.pending_causes |= (uint16_t)(1u << i);
}

/* The part every name the filter can match starts with, up to a '.':
 * "dEQP-GLES2.a.b.*" -> "dEQP-GLES2.a.b.", and for a list
 * "dEQP-GLES2.a.b.*,dEQP-GLES2.a.c.*" -> "dEQP-GLES2.a.". It ends at the
 * string's end (or a quote). */
static size_t common_prefix(const char *f)
{
	size_t len = SIZE_MAX, dot = 0;
	const char *p = f;

	for (;;) { /* each pattern: its length up to the first '*' */
		size_t n = strcspn(p, "*,'");
		size_t same = 0;

		while (same < n && same < len && p[same] == f[same])
			same++;
		len = same;
		p += strcspn(p, ",'");
		if (*p != ',')
			break;
		p++;
	}
	for (size_t i = 0; i < len && len != SIZE_MAX; i++)
		if (f[i] == '.')
			dot = i + 1;
	return dot;
}

static void take_line(void)
{
	line[line_len] = 0;
	if (starts(line, "deqp: ")) {
		const char *rest = line + sizeof("deqp: ") - 1;

		for (size_t s = 0; s < ARRAY_SIZE(statuses); s++) {
			size_t n = strlen(statuses[s].code);

			if (strncmp(rest, statuses[s].code, n) == 0 && rest[n] == ' ') {
				take_verdict(statuses[s].letter, rest + n + 1);
				return;
			}
		}
		/* "case filter '...'": the record takes the filter from
		 * CONFIG_DEQP_CASE instead (a long one does not fit one line) */
		if (starts(rest, "done: "))
			keep(t.done, rest);
		else if (starts(rest, "summary: "))
			keep(t.summary, rest);
		else if (starts(rest, "exception: "))
			keep(t.exception, rest);
		return;
	}
	if (starts(line, "yakogl/v3d: draw refused: ")) {
		take_cause(line + sizeof("yakogl/v3d: draw refused: ") - 1);
		return;
	}
	if (starts(line, "yakogl: ") && strstr(line, " was not recorded (")) {
		/* the GL core's echo of a failed draw, refusal or not (the arena,
		 * the GPU): R comes from the backend's count, not from this line */
		return;
	}
	/* a job the OSAL timed: slow (it ended) or hung (it did not) */
	if (starts(line, "osal(v3d): job ") && strstr(line, "(slow;")) {
		t.pending_slow++;
	} else if ((starts(line, "osal(v3d): job ") && strstr(line, " did not end ")) ||
		   starts(line, "osal(v3d): FIRST FAILURE")) {
		if (!t.pending_hung && !t.first_hung_line[0])
			keep(t.first_hung_line, line);
		t.pending_hung = true;
	}
	if (strstr(line, " vio_id ")) {
		if (!t.vio_line[0])
			snprintf(t.vio_line, FAULT_LEN, "%.*s", FAULT_LEN - 1, line);
	} else if (strstr(line, " MMU fault") || strstr(line, " written past ")) {
		if (t.n_fault < MAX_FAULT)
			snprintf(t.fault[t.n_fault], FAULT_LEN, "%.*s", FAULT_LEN - 1, line);
		t.n_fault++;
	}
	/* the rest of what the driver says, and Zephyr's LOG errors and warnings */
	if (starts(line, "yakogl") || starts(line, "osal(v3d)") || starts(line, "v3d") ||
	    starts(line, "E: ") || starts(line, "W: ")) {
		if (starts(line, "v3d_deqp"))
			return; /* our own */
		if (t.n_driver < MAX_DRIVER)
			keep(t.driver[t.n_driver], line);
		t.n_driver++;
	}
}

/* Every printk character: to the UART, to the screen, and a line at a time
 * to the tally. */
static int tap(int c)
{
	if (uart_out)
		uart_out(c);
	switch (mode) {
	case TAP_FROZEN:
		return c;
	case TAP_RECORD:
		/* fbcon's rule (driver/zephyr/lib/board/rpi4/fbcon.c out()): 8-pixel glyphs, as ROWS */
		if (c == '\n' || (c != '\r' && rec_col >= FB_W / 8)) {
			rec_col = 0;
			rec_rows++;
		}
		if (c != '\n' && c != '\r')
			rec_col++;
		return fbcon_out(c);
	case TAP_RUN:
		break;
	}
	if (c == '\n' || line_len == sizeof(line) - 1) {
		take_line();
		line_len = 0;
	}
	if (c != '\n' && c != '\r')
		line[line_len++] = (char)c;
	return fbcon_out(c);
}

/* dEQP's stdout and stderr: to the UART, and its fatal line to the record. */
static char out_line[TEXT_LEN];
static size_t out_len;

/* What to show for a case: the compiler's line when a build failed in it
 * (info is kept only for a failed compile or link), else dEQP's reason. */
static const char *why(const char *reason, const char *info)
{
	return info[0] ? info : reason;
}

#define SHOWN_BUCKETS 3 /* reasons shown per group */
#define GROUP_INDENT  84 /* 2 + 56 + 1 + 4 + 20 + 1: where a group's reasons start */

/* A group's reasons in the order they are shown: F/R reasons by count, then
 * N reasons by count. Returns how many. */
static uint32_t bucket_order(const struct group *g, const struct bucket *order[MAX_BUCKETS])
{
	uint32_t n = 0;

	for (int pass = 0; pass < 2; pass++) { /* 0: not N, 1: N */
		uint32_t first = n;

		for (uint32_t i = 0; i < g->n_buckets; i++)
			if (g->b[i].is_n == (pass == 1))
				order[n++] = &g->b[i];
		for (uint32_t i = first + 1; i < n; i++) /* insertion sort by count, desc */
			for (uint32_t j = i; j > first && order[j]->n > order[j - 1]->n; j--) {
				const struct bucket *tmp = order[j];

				order[j] = order[j - 1];
				order[j - 1] = tmp;
			}
	}
	return n;
}

/* The rows a group needs beyond its own: one per further shown reason, and
 * one for the cases no shown reason covers. */
static int group_extra_rows(const struct group *g)
{
	const struct bucket *order[MAX_BUCKETS];
	uint32_t n = bucket_order(g, order), shown = MIN(n, SHOWN_BUCKETS), counted = 0;

	for (uint32_t b = 0; b < shown; b++)
		counted += order[b]->n;
	return (shown ? (int)shown - 1 : 0) + (g->n > counted ? 1 : 0);
}

/* Count a case's reason in its group: the bucket with the same tail, or a
 * new one while there is room. */
static void count_reason(struct group *g, const char *why_text, bool is_n)
{
	size_t len = strlen(why_text);
	const char *key = len >= BUCKET_LEN ? why_text + len - (BUCKET_LEN - 1) : why_text;

	for (uint32_t i = 0; i < g->n_buckets; i++)
		if (g->b[i].is_n == is_n && strcmp(g->b[i].text, key) == 0) {
			g->b[i].n++;
			return;
		}
	if (g->n_buckets == MAX_BUCKETS) {
		g->n_other++;
		return;
	}
	struct bucket *b = &g->b[g->n_buckets++];

	snprintf(b->text, sizeof(b->text), "%.*s", BUCKET_LEN - 1, key);
	b->n = 1;
	b->is_n = is_n;
}

/* "  <Status> (<reason>)": the reason for the case whose verdict came last,
 * kept with that case and counted in its group. */
static void take_reason(const char *s)
{
	struct listed *l = t.n_listed ? &t.listed[t.n_listed - 1] : NULL;
	struct group *g = t.last_group >= 0 && (uint32_t)t.last_group < t.n_groups ?
				  &t.group[t.last_group] : NULL;

	if (l && (l->index != t.n_cases - 1 || l->reason[0]))
		l = NULL; /* not that case's entry, or it has its reason */
	if (!l && !g)
		return; /* that case passed */
	for (size_t i = 0; i < ARRAY_SIZE(statuses); i++) {
		size_t n = strlen(statuses[i].code);

		if (strncmp(s, statuses[i].code, n) == 0 && strncmp(s + n, " (", 2) == 0) {
			char r[REASON_LEN];
			int len = (int)strlen(s + n + 2);

			if (len && s[n + 2 + len - 1] == ')')
				len--;
			snprintf(r, sizeof(r), "%.*s", len, s + n + 2);
			if (l)
				snprintf(l->reason, sizeof(l->reason), "%s", r);
			if (g) {
				count_reason(g, why(r, t.last_info), t.last_letter == 'N');
				t.last_group = -1; /* one reason per case */
			}
			return;
		}
	}
}

static int stdout_tap(int c)
{
	if (uart_out)
		uart_out(c);
	if (c == '\n' || out_len == sizeof(out_line) - 1) {
		out_line[out_len] = 0;
		if (starts(out_line, "FATAL ERROR") && !t.fatal[0])
			keep(t.fatal, out_line);
		else if (mode == TAP_RUN && starts(out_line, "Test case '")) {
			const char *n = out_line + sizeof("Test case '") - 1;
			size_t len = strcspn(n, "'");

			snprintf(t.current, sizeof(t.current), "%.*s", (int)MIN(len, NAME_LEN - 1),
				 n);
		}
		else if (mode == TAP_RUN && starts(out_line, "  "))
			take_reason(out_line + 2);
		out_len = 0;
	}
	if (c != '\n' && c != '\r')
		out_line[out_len++] = (char)c;
	return c;
}

/* Print text in cols columns: whole when it fits, else "~" and its tail,
 * as long case names are shown (the end says the most). */
static void print_tail(const char *text, int cols)
{
	int n = (int)strlen(text);

	if (n <= cols)
		printk("%s", text);
	else
		printk("~%s", text + n - (cols - 1));
}

/* ---- the record ------------------------------------------------------------- */

static struct rpi_fb fb;
static bool have_screen;

static uint32_t n_of(char letter)
{
	return t.n_letter[(unsigned char)letter & 0x7f];
}

/* One screen, 45 rows of 160 columns: the build, the totals, a letter per
 * case, the refusal causes, then the cases that did not pass by name for as
 * many rows as are left. */
static void print_record(const char *ending)
{
	uint32_t shown = MIN(t.n_cases, MAX_CASES);
	uint32_t not_passed = t.n_cases - n_of('P');
	int rows = 0;

	/* one row: the filter gets what the rest leaves, and is cut with "..." */
	{
		char head[160];
		const char *filter = t.filter[0] ? t.filter : "(not printed)";
		int used = snprintf(head, sizeof(head), "v3d_deqp %s: build %s, filter ", ending,
				    V3D_BUILD);
		int room = (int)sizeof(head) - 1 - MIN(used, (int)sizeof(head) - 1);
		int len = (int)strlen(filter);

		if (len > room)
			printk("%s%.*s...\n", head, MAX(room - 3, 0), filter);
		else
			printk("%s%s\n", head, filter);
	}
	printk("  %u cases in %lld ms, record at %lld s after boot, reset at ~%d s\n", t.n_cases,
	       (long long)(t.t_end_ms - t.t_start_ms), (long long)(k_uptime_get() / 1000),
	       CONFIG_V3D_DEQP_LIFETIME_S + 10);
	printk("  totals:");
	for (const char *l = letters; *l; l++)
		if (n_of(*l))
			printk(" %c %u", *l, n_of(*l));
	printk("  (R: not passed, a draw was refused in it; passed with one refused: %u)\n",
	       t.n_pass_refusal);
	rows += 3;
	if (t.done[0])
		printk("  %s\n", t.done);
	if (t.summary[0])
		printk("  %s\n", t.summary);
	if (t.exception[0])
		printk("  %s\n", t.exception);
	if (t.fatal[0])
		printk("  dEQP: %s\n", t.fatal);
	if (t.current[0]) /* begun, no verdict: what a stop caught */
		printk("  in progress: %.*s\n", 140, t.current);
	rows += !!t.done[0] + !!t.summary[0] + !!t.exception[0] + !!t.fatal[0] + !!t.current[0];

	/* The rows after the letters: the refusals, the driver lines, at least
	 * two rows of names, the "more" line and the last line. The letters get
	 * the rest; a group too large for that is cut short (and too long for
	 * one image anyway: see V3D_DEQP_LIFETIME_S). */
	int driver_rows = (int)MIN(t.n_driver, MAX_DRIVER) + (t.n_driver > MAX_DRIVER);
	int refusal_rows = (t.n_causes || t.refused_draws) ? 1 + (int)t.n_causes : 0;
	int group_rows = 0;

	if (t.n_groups) {
		group_rows = 1; /* the header */
		for (uint32_t i = 0; i < t.n_groups; i++)
			group_rows += 1 + group_extra_rows(&t.group[i]);
	}
	int after = refusal_rows + driver_rows + group_rows + 2 + 2;

	if (t.keep_read) {
		printk("  keep region (KiB, of %u): after #0 %u, most %u at #%u, last %u, high-water %u",
		       CONFIG_YAKOGL_SHADERC_KEEP_BYTES / 1024, (unsigned)(t.keep_first / 1024),
		       (unsigned)(t.keep_max / 1024), t.keep_max_at, (unsigned)(t.keep_last / 1024),
		       (unsigned)(t.keep_peak / 1024));
		if (t.have_bad)
			printk(", before #%u (first not passed) %u", t.first_bad,
			       (unsigned)(t.keep_before_bad / 1024));
		printk("; slowest #%u %u ms\n", t.slowest_at, t.slowest_ms);
		if (carve_known)
			printk("  carve-out (KiB, of %u): after #0 %u, last %u, high-water %u\n",
			       t.carve_total / 1024, t.carve_first / 1024, t.carve_last / 1024,
			       t.carve_high / 1024);
		else
			printk("  carve-out: this OSAL has no osal_v3d_mem_used\n");
		rows += 2;
	}
	/* which case the first slow and the first hung job belong to: the hung
	 * one is the cause, the cases after it may only be its victims */
	printk("  jobs: slow in %u cases", t.n_slow_cases);
	if (t.n_slow_cases)
		printk(" (first #%u)", t.first_slow);
	if (t.have_hung)
		printk("; the first hung or timed-out job in #%u", t.first_hung);
	else
		printk("; no job hung or timed out");
	printk("\n");
	rows++;
	if (t.first_hung_line[0]) {
		printk("  %.*s\n", 156, t.first_hung_line);
		rows++;
	}
	/* The OSAL's job record: the first completed job, the first that failed
	 * (and in a newer OSAL the MMU fault, slow and hung jobs, the carve-out).
	 * Its length depends on the OSAL, so its rows are counted as printed. */
	uint32_t before = rec_rows;

	osal_v3d_diag_print();
	rows += (int)(rec_rows - before);

	/* The fault lines, whole, in the rows the rest leaves: the case-order
	 * header, one row of letters and everything after them come first. A
	 * line of n characters, indented by 2, takes (n + 2 + 159) / 160 rows. */
	if (t.n_fault || t.vio_line[0]) {
		int budget = ROWS - rows - 2 - after - 1; /* - 1: this block's header */
		uint32_t shown_fault = 0;

		if (budget > 0) {
			printk("fault lines (%u, and the MMU's address and client first):\n",
			       t.n_fault);
			rows++;
			if (t.vio_line[0]) {
				int cost = (int)((strlen(t.vio_line) + 2 + FB_W / 8 - 1) / (FB_W / 8));

				if (cost <= budget) {
					printk("  %s\n", t.vio_line);
					budget -= cost;
					rows += cost;
				}
			}
			for (; shown_fault < MIN(t.n_fault, MAX_FAULT); shown_fault++) {
				int cost = (int)((strlen(t.fault[shown_fault]) + 2 + FB_W / 8 - 1) /
						 (FB_W / 8));

				if (cost > budget)
					break;
				printk("  %s\n", t.fault[shown_fault]);
				budget -= cost;
				rows += cost;
			}
			if (shown_fault < t.n_fault && budget > 0) {
				printk("  ... %u more (UART)\n", t.n_fault - shown_fault);
				rows++;
			}
		}
	}

	uint32_t verdict_rows = (shown + PER_ROW - 1) / PER_ROW;
	uint32_t max_rows = (uint32_t)MAX(ROWS - rows - 1 - after, 1);

	if (verdict_rows > max_rows) {
		verdict_rows = max_rows;
		shown = verdict_rows * PER_ROW;
	}
	printk("case order: P pass F fail R refused N not supported Q quality C compat "
	       "E resource I internal X crash T timeout%s\n",
	       shown < t.n_cases ? " (letters stop before the last cases)" : "");
	rows++;
	for (uint32_t r = 0; r < verdict_rows; r++) {
		uint32_t from = r * PER_ROW, n = MIN(PER_ROW, shown - from);

		printk("%5u %.*s\n", from, (int)n, &t.verdict[from]);
	}
	rows += (int)verdict_rows;

	if (t.n_causes || t.refused_draws) {
		printk("refusals: %u draws refused by the V3D backend (v3d_refused_draws), causes:%s\n",
		       t.refused_draws, t.n_causes_lost ? " (more than the table; r?)" : "");
		for (uint32_t i = 0; i < t.n_causes; i++)
			printk("  r%u (first #%u) %s\n", i, t.cause_first[i], t.cause[i]);
	}
	for (uint32_t i = 0; i < MIN(t.n_driver, MAX_DRIVER); i++)
		printk("driver: %s\n", t.driver[i]);
	if (t.n_driver > MAX_DRIVER)
		printk("driver: ... %u more lines (UART)\n", t.n_driver - MAX_DRIVER);
	rows += refusal_rows + driver_rows;

	/* every case that did not pass, counted by group */
	if (t.n_groups) {
		printk("not passed, by group (count, letters; reasons by count, F/R before N)%s:\n",
		       t.n_grouped_lost ? " (more groups than the table: see the totals)" : "");
		for (uint32_t i = 0; i < t.n_groups; i++) {
			const struct group *g = &t.group[i];
			const struct bucket *order[MAX_BUCKETS];
			uint32_t n_order = bucket_order(g, order);
			uint32_t shown = MIN(n_order, SHOWN_BUCKETS), counted = 0;
			char counts[48] = "";
			int k = 0;

			for (size_t c = 0; c < ARRAY_SIZE(g->n_letter) && k < (int)sizeof(counts) - 8; c++)
				if (g->n_letter[c])
					k += snprintf(counts + k, sizeof(counts) - k, " %c%u", letters[c],
						      g->n_letter[c]);
			/* 84 columns (2 + 56 + 1 + 4 + 20 + 1), then "%5ux " (7) and the
			 * reason's tail (68): 159. Further reasons go on their own rows,
			 * indented to the same 84 columns. */
			printk("  %-56.56s %4u%-20.20s ", g->name, g->n, counts);
			for (uint32_t b = 0; b < shown; b++) {
				if (b)
					printk("%*s", GROUP_INDENT, "");
				printk("%5ux ", order[b]->n);
				print_tail(order[b]->text, 68);
				printk("\n");
				counted += order[b]->n;
			}
			if (!shown)
				printk("(no reason line)\n");
			if (g->n > counted)
				printk("%*s%5u more: other reasons, or none printed\n", GROUP_INDENT, "",
				       g->n - counted);
		}
		rows += group_rows;
	}

	/* the cases that did not pass, one to a row: index, letter, name,
	 * refusal causes, dEQP's reason (cut at the screen's edge) */
	int room = ROWS - rows - 2; /* the "more" line and the last line */
	uint32_t listed = 0;

	for (; listed < t.n_listed && room > 0; room--, listed++) {
		const struct listed *l = &t.listed[listed];
		char tag[48] = "";
		/* its time, the keep region and the carve-out at its verdict (KiB
		 * held/high-water), then the refusal causes */
		int k = snprintf(tag, sizeof(tag), " %ums k%u/%u", l->ms, l->keep_kib,
				 l->keep_peak_kib);

		if (carve_known)
			k += snprintf(tag + k, sizeof(tag) - k, " c%u/%u", l->carve_kib,
				      l->carve_high_kib);
		if (l->hung)
			k += snprintf(tag + k, sizeof(tag) - k, " H");
		else if (l->slow_jobs)
			k += snprintf(tag + k, sizeof(tag) - k, " S%u", l->slow_jobs);

		for (uint32_t c = 0; c < t.n_causes && k < (int)sizeof(tag) - 4; c++)
			if (l->causes & (1u << c))
				k += snprintf(tag + k, sizeof(tag) - k, " r%u", c);
		if (l->refused && !l->causes && k < (int)sizeof(tag) - 4)
			snprintf(tag + k, sizeof(tag) - k, " r?"); /* a cause logged before */
		size_t n = strlen(l->name);
		const char *shown_name = n > NAME_COLS ? l->name + n - (NAME_COLS - 1) : l->name;

		/* 5 + 1 + 1 + 1 + NAME_COLS + TAG_COLS + 1 = 89 columns before the reason */
		printk("%5u %c %s%-*s%-*.*s ", l->index, l->letter, n > NAME_COLS ? "~" : "",
		       n > NAME_COLS ? NAME_COLS - 1 : NAME_COLS, shown_name, TAG_COLS, TAG_COLS, tag);
		print_tail(why(l->reason, l->info), REASON_COLS);
		printk("\n");
	}
	if (listed < not_passed)
		printk("  ... %u more not passed (the UART has every line)\n", not_passed - listed);
	printk("v3d_deqp: build %s %s: %u cases, P %u F %u R %u N %u other %u\n", V3D_BUILD,
	       ending, t.n_cases, n_of('P'), n_of('F'), n_of('R'), n_of('N'),
	       t.n_cases - n_of('P') - n_of('F') - n_of('R') - n_of('N'));
}

/* Clear the screen, print the record, freeze. Once, whoever comes first. */
static void take_record(const char *ending)
{
	if (!atomic_cas(&record_taken, 0, 1) || !have_screen)
		return;
	mode = TAP_RECORD;
	rec_col = 0; /* fbcon_attach starts at the top left */
	fbcon_attach(fb.cpu, fb.width, fb.pitch, fb.height, 1);
	__printk_hook_install(tap);
	print_record(ending);
	mode = TAP_FROZEN;
}

static void photo_deadline(struct k_work *w)
{
	ARG_UNUSED(w);
	take_record("INCOMPLETE (the photo deadline came first)");
}

static K_WORK_DELAYABLE_DEFINE(deadline_work, photo_deadline);

/* The deqp main's last call (rc: 0 all passed, -1 not, -2 dEQP threw). */
int pvr_sample_finish(int rc)
{
	/* dEQP ends a session early after a fatal result (a ResourceError, such
	 * as a glGetError() of GL_OUT_OF_MEMORY: tcuTestSessionExecutor.cpp:384),
	 * and then says complete=0: say so, because "DONE" would hide the cases
	 * that never began. */
	bool stopped = strstr(t.done, "complete=0") != NULL;

	/* dEQP's rc is 0 when nothing failed, which includes NotSupported and
	 * warnings: "all passed" only when every case was a Pass */
	bool all_passed = t.n_cases && n_of('P') == t.n_cases;

	take_record(rc == 0 && all_passed ? "DONE, all passed" :
		    rc == 0          ? "DONE, none failed" :
		    rc == -2         ? "DONE, dEQP threw" :
		    stopped          ? "STOPPED EARLY by dEQP (complete=0; later cases never began)" :
				       "DONE");
	for (;;)
		k_sleep(K_FOREVER); /* the watchdog ends the image */
	return rc;
}

/* A fault: record what there is, with where it happened (the faulting PC
 * and LR from the exception frame, and FAR), for addr2line on the image's
 * ELF. ESR is not shown: by the time this runs, a later FP-access trap has
 * replaced it (QEMU: EC 0x7 read here, where the fault was EC 0x25).
 * Returning aborts the faulting thread; for a panic or a fault in an ISR the
 * kernel halts instead, and the watchdog then resets within 10 s, so the
 * record is on the screen for a few seconds only. */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	char ending[112];

	snprintf(ending, sizeof(ending),
		 "FATAL (reason %u, pc 0x%llx lr 0x%llx far 0x%llx)", reason,
		 esf ? (unsigned long long)esf->elr : 0ULL,
		 esf ? (unsigned long long)esf->lr : 0ULL, (unsigned long long)read_far_el1());
	take_record(ending);
}

/* ---- bring-up, before main() -------------------------------------------- */

static int v3d_deqp_board_init(void)
{
	int ret = rpi_mbox_init();

	if (ret == 0)
		ret = rpi_fb_alloc(FB_W, FB_H, &fb);
	if (ret)
		return 0; /* no screen: the UART still has every line */
	have_screen = true;
	uart_out = __printk_get_hook();
	fbcon_attach(fb.cpu, fb.width, fb.pitch, fb.height, 1);
	fbcon_out = __printk_get_hook();
	__printk_hook_install(tap);
	__stdout_hook_install(stdout_tap);

	printk("v3d_deqp: build %s\n", V3D_BUILD);
#ifdef V3D_NO_RPI_WDOG
	printk("board: NO watchdog (a QEMU build, V3D_RPI_WDOG=OFF)\n");
#else
	rpi_wdog_announce();
#endif
	printk("v3d_deqp: the record at %d s after boot at the latest\n",
	       CONFIG_V3D_DEQP_LIFETIME_S - CONFIG_V3D_DEQP_PHOTO_S);

	/* the V3D at its highest clock, as v3d_gl_triangle */
	uint32_t hz = 0, max_hz = 0;

	if (rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz) == 0 && max_hz &&
	    rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz != max_hz) {
		ret = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		printk("V3D clock %u -> %u Hz (%d)\n", hz, max_hz, ret);
	}
	k_work_schedule(&deadline_work,
			K_TIMEOUT_ABS_MS((int64_t)(CONFIG_V3D_DEQP_LIFETIME_S -
						   CONFIG_V3D_DEQP_PHOTO_S) * 1000));
	t.t_start_ms = t.t_end_ms = k_uptime_get();
	t.last_group = -1; /* no case yet */
	t.refused_mark = v3d_refused_draws(); /* 0 before EGL starts the backend */
	snprintf(t.filter, sizeof(t.filter), "'%.*s'", (int)sizeof(t.filter) - 3, case_filter);
	t.prefix_len = common_prefix(case_filter);
	return 0;
}

SYS_INIT(v3d_deqp_board_init, APPLICATION, 99);
