/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * shim.c - what glmark2 asks of the operating system on the host run, and
 * the recording of its GL stream (README.md).
 *
 * glmark2 opens libEGL / libGLESv2 with dlopen and takes every entry point
 * with dlsym (shared-library.cpp), then asks eglGetProcAddress for the rest
 * (gl-state-egl.cpp, load_proc). run.sh renames dlopen, dlsym, dlclose and
 * dlerror in shared-library.o only (objcopy --redefine-sym) to the gm2_*
 * below, so YakoGL, linked in statically, is "the library", as on the
 * Zephyr image (driver/samples/zephyr/glmark2/src/platform.c). Every symbol
 * resolves through lookup(): a wrapper of this file where there is one, else
 * what YakoGL's eglGetProcAddress answers.
 *
 * The wrappers record, on stderr (where src/osal/linux.c logs the library's
 * own lines, so the order is kept), one "gm2:" line per event:
 *
 *   gm2: shader <id> <vert|frag> <hash>     a source handed to glShaderSource
 *                                            and compiled (the text is written
 *                                            to $GM2_CAPTURE/<hash>.<stage>)
 *   gm2: compile-failed <id> <hash>: <log>  glCompileShader left COMPILE_STATUS false
 *   gm2: link-failed <prog>: <log>          glLinkProgram left LINK_STATUS false
 *   gm2: glGetError 0x<e>                   the client read a non-zero error
 *   gm2: eglGetError 0x<e>                  ... a non-EGL_SUCCESS one
 *   gm2: fb-incomplete 0x<status>           glCheckFramebufferStatus said so
 *   gm2: egl-config id=... (eglCreateWindowSurface / eglCreatePbufferSurface)
 *   gm2: draw <n> fb <fb> kick <k> submitted <s>
 *                                            with GM2_KICKS=1 on a PowerVR
 *                                            build, after every draw: the
 *                                            framebuffer it rendered into, the
 *                                            kick holding it (the backend's
 *                                            kick_id() after the draw), and
 *                                            the kicks submitted so far
 *                                            (pvr_backend_counters): the
 *                                            order of a scene's passes
 *
 * The GL errors themselves, with the entry point that raised them, are the
 * library's own lines ("GL error 0x502 in glGenerateMipmap"), every one of
 * them: run.sh sets YAKOGL_ERROR_LOG, which linux.c hands to the GL core.
 *
 * With GM2_PROFILE=1 every entry point lookup() answers is wrapped once more
 * in a timing wrapper (gm2-timed.h, which run.sh generates with
 * gen_timed.py from YakoGL's GLES2 and EGL headers), and the process ends
 * with one line per entry point that was called and one for the frames:
 *
 *   gm2: gl-time <name> setup=<calls>,<wall ns>,<cpu ns> frames=... teardown=...
 *   gm2: gl-frames swaps=<s> counted=<n> wall_ns=<w> cpu_ns=<c> gl_wall_ns=<g>
 *
 * The phases are cut at eglSwapBuffers (glmark2 ends each frame with one,
 * main-loop.cpp MainLoop::draw): "setup" is every call before the first
 * swap returned (the scene's setup and its first frame, with the shader
 * compiles and first-use builds), "frames" every call after it up to the
 * end of swap GM2_FRAMES (the counted frames, n = GM2_FRAMES - 1), and
 * "teardown" what comes after (the scene's teardown). Wall time is
 * CLOCK_MONOTONIC, cpu time CLOCK_THREAD_CPUTIME_ID; on the host they agree
 * unless the library sleeps or waits, as linux.c's submits complete at
 * once. gl-frames gives the counted frames' wall and cpu time, glmark2's
 * own work included, and the wall time spent in GL and EGL calls during
 * them. A call made inside another timed call (none in glmark2) is counted
 * in the outer one only.
 *
 * The GPU work linux.c models on the CPU (the V3D's TFU jobs, src/osal/
 * linux.c "the TFU"; osal_linux_gpu_time) is the board's GPU time, not the
 * call's: it is taken out of the call's wall and cpu time, and out of the
 * frames' (gl-frames wall_ns, cpu_ns, gl_wall_ns), and given apart:
 *
 *   gm2: gpu-time <name> setup=<jobs>,<wall ns> frames=... teardown=...
 *   (gl-frames) ... gpu_jobs=<n> gpu_ns=<wall ns>
 *
 * a gpu-time line for each entry point that ran a job. The library's
 * per-step laps of a draw (src/draw_steps.rs; the OSAL adds them up, linux.c
 * osal_linux_draw_stats) over the counted frames follow, one line a run:
 *
 *   gm2: draw-steps counted=<n> scopes=<s> untimed=<u> ns=<12 comma-separated ns>
 *
 * the steps in draw_steps.rs Step order, scopes the timed scopes reported
 * (a draw each, a nested clear draw its own), untimed the laps the clock could
 * not read (GM2_NOCLK=1 refuses it: all laps untimed, the ns zero).
 * The line is printed when the library reported any scope. On a V3D build the
 * back end's jobs submitted and CPU waits for a job (v3d_counters) over the
 * counted frames follow, and the waits made inside each entry point:
 *
 *   gm2: v3d-frames counted=<n> kicks=<k> waits=<w>
 *   gm2: v3d-waits <name> setup=<w> frames=<w> teardown=<w>
 *
 * linux.c's submits complete at once, so a wait costs the host nearly
 * nothing: on the host the wait count, not the milliseconds, is what a
 * change to the V3D's job ordering moves.
 *
 * With GM2_PERF_CTL (and GM2_PERF_ACK) naming perf record's control and ack
 * fifos (perf record --control fifo:<ctl>,<ack> -D -1, run.sh GM2_PERF=1),
 * the end of the first swap writes "enable" and the end of swap GM2_FRAMES
 * "disable", so perf samples the counted frames only.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define EGL_EGLEXT_PROTOTYPES
#define GL_GLEXT_PROTOTYPES
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>

#include "shim.h"

/* ---- the shaders ---------------------------------------------------------- */

struct shader {
	GLuint id;
	GLenum type;
	char *text;
};
static struct shader *S;
static size_t nS, capS;

static struct shader *shader_of(GLuint id, int add)
{
	for (size_t i = 0; i < nS; i++)
		if (S[i].id == id)
			return &S[i];
	if (!add)
		return NULL;
	if (nS == capS) {
		capS = capS ? 2 * capS : 64;
		S = realloc(S, capS * sizeof(*S));
		if (!S)
			abort();
	}
	S[nS] = (struct shader){ id, 0, NULL };
	return &S[nS++];
}

static uint64_t fnv1a(const char *s)
{
	uint64_t h = 0xcbf29ce484222325ull;

	for (; *s; s++)
		h = (h ^ (unsigned char)*s) * 0x100000001b3ull;
	return h;
}

static const char *stage_name(GLenum type)
{
	return type == GL_VERTEX_SHADER ? "vert" : type == GL_FRAGMENT_SHADER ? "frag" : "unknown";
}

/* The text, once per hash: $GM2_CAPTURE/<hash>.<stage> */
static void capture(uint64_t h, const char *stage, const char *text)
{
	const char *dir = getenv("GM2_CAPTURE");
	char path[4096];
	struct stat st;
	FILE *f;

	if (!dir || !*dir)
		return;
	snprintf(path, sizeof(path), "%s/%016llx.%s", dir, (unsigned long long)h, stage);
	if (stat(path, &st) == 0)
		return;
	f = fopen(path, "w");
	if (!f) {
		fprintf(stderr, "gm2: cannot write %s\n", path);
		return;
	}
	fputs(text, f);
	fclose(f);
}

static void one_line(char *s)
{
	for (; *s; s++)
		if (*s == '\n' || *s == '\r')
			*s = ' ';
}

static GLuint w_glCreateShader(GLenum type)
{
	GLuint id = glCreateShader(type);

	if (id)
		shader_of(id, 1)->type = type;
	return id;
}

static void w_glShaderSource(GLuint id, GLsizei count, const GLchar *const *string, const GLint *length)
{
	glShaderSource(id, count, string, length);
	if (count < 0 || !string)
		return;
	size_t n = 0;
	for (GLsizei i = 0; i < count; i++)
		if (string[i])
			n += (length && length[i] >= 0) ? (size_t)length[i] : strlen(string[i]);
	char *t = malloc(n + 1), *p = t;
	if (!t)
		abort();
	for (GLsizei i = 0; i < count; i++) {
		if (!string[i])
			continue;
		size_t k = (length && length[i] >= 0) ? (size_t)length[i] : strlen(string[i]);
		memcpy(p, string[i], k);
		p += k;
	}
	*p = '\0';
	struct shader *s = shader_of(id, 1);
	free(s->text);
	s->text = t;
}

static void w_glCompileShader(GLuint id)
{
	struct shader *s = shader_of(id, 0);
	GLint ok = 0;
	uint64_t h = 0;

	glCompileShader(id);
	glGetShaderiv(id, GL_COMPILE_STATUS, &ok);
	if (s && s->text) {
		h = fnv1a(s->text);
		capture(h, stage_name(s->type), s->text);
		fprintf(stderr, "gm2: shader %u %s %016llx\n", id, stage_name(s->type), (unsigned long long)h);
	}
	if (!ok) {
		char log[2048] = "";
		glGetShaderInfoLog(id, sizeof(log), NULL, log);
		one_line(log);
		fprintf(stderr, "gm2: compile-failed %u %016llx: %s\n", id, (unsigned long long)h, log);
	}
}

static void w_glDeleteShader(GLuint id)
{
	struct shader *s = shader_of(id, 0);

	glDeleteShader(id);
	if (s) {
		free(s->text);
		*s = S[--nS];
	}
}

static void w_glLinkProgram(GLuint prog)
{
	GLint ok = 0;

	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[2048] = "";
		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		one_line(log);
		fprintf(stderr, "gm2: link-failed %u: %s\n", prog, log);
	}
}

/* ---- errors, framebuffers, draws ------------------------------------------ */

static GLenum w_glGetError(void)
{
	GLenum e = glGetError();

	if (e != GL_NO_ERROR)
		fprintf(stderr, "gm2: glGetError 0x%x\n", e);
	return e;
}

static EGLint w_eglGetError(void)
{
	EGLint e = eglGetError();

	if (e != EGL_SUCCESS)
		fprintf(stderr, "gm2: eglGetError 0x%x\n", e);
	return e;
}

static GLenum w_glCheckFramebufferStatus(GLenum target)
{
	GLenum st = glCheckFramebufferStatus(target);

	if (st != GL_FRAMEBUFFER_COMPLETE)
		fprintf(stderr, "gm2: fb-incomplete 0x%x\n", st);
	return st;
}

static unsigned long n_draws;

unsigned long gm2_draws(void)
{
	return n_draws;
}

/* The PowerVR back end's kick id and counters (src/pvr/backend.rs), weak:
 * the other GPUs' libraries have none, and GM2_KICKS then logs nothing. */
extern uint32_t pvr_backend_kick_id(void) __attribute__((weak));
extern void pvr_backend_counters(uint32_t *kicks, uint32_t *waits) __attribute__((weak));

/* GM2_KICKS=1: the "gm2: draw" line after every draw (above). glmark2
 * shadow's frame then reads: the depth pass in kick k with k - 1 submitted
 * (its draw ended the window clear's frame, so it is not in the kick that was
 * being recorded when it began: pvr-depth-resolve), then the ground in k + 1
 * with k submitted (the depth resolve's, or the draw's own, submit). */
static void log_kick(void)
{
	static int on = -1;

	if (on < 0) {
		const char *e = getenv("GM2_KICKS");
		on = e && strcmp(e, "1") == 0 && pvr_backend_kick_id && pvr_backend_counters;
	}
	if (!on)
		return;
	GLint fb = 0;
	uint32_t kicks = 0;

	glGetIntegerv(GL_FRAMEBUFFER_BINDING, &fb);
	pvr_backend_counters(&kicks, NULL);
	fprintf(stderr, "gm2: draw %lu fb %d kick %u submitted %u\n", n_draws, (int)fb, pvr_backend_kick_id(), kicks);
}

static void w_glDrawArrays(GLenum mode, GLint first, GLsizei count)
{
	n_draws++;
	glDrawArrays(mode, first, count);
	log_kick();
}

static void w_glDrawElements(GLenum mode, GLsizei count, GLenum type, const void *indices)
{
	n_draws++;
	glDrawElements(mode, count, type, indices);
	log_kick();
}

/* ---- the EGL config ------------------------------------------------------- */

static void log_config(const char *what, EGLDisplay dpy, EGLConfig cfg)
{
	static const struct { EGLint a; const char *n; } A[] = {
		{ EGL_CONFIG_ID, "id" },       { EGL_BUFFER_SIZE, "buf" },     { EGL_RED_SIZE, "r" },
		{ EGL_GREEN_SIZE, "g" },       { EGL_BLUE_SIZE, "b" },         { EGL_ALPHA_SIZE, "a" },
		{ EGL_DEPTH_SIZE, "depth" },   { EGL_STENCIL_SIZE, "stencil" }, { EGL_SAMPLES, "samples" },
		{ EGL_SURFACE_TYPE, "surface" }, { EGL_RENDERABLE_TYPE, "renderable" },
		{ EGL_NATIVE_VISUAL_ID, "visual" },
	};
	char line[512];
	int n = snprintf(line, sizeof(line), "gm2: egl-config %s", what);

	for (size_t i = 0; i < sizeof(A) / sizeof(A[0]) && n < (int)sizeof(line); i++) {
		EGLint v = -1;
		eglGetConfigAttrib(dpy, cfg, A[i].a, &v);
		n += snprintf(line + n, sizeof(line) - n, " %s=0x%x", A[i].n, v);
	}
	fprintf(stderr, "%s\n", line);
}

static EGLSurface w_eglCreateWindowSurface(EGLDisplay dpy, EGLConfig cfg, EGLNativeWindowType win,
					   const EGLint *attrib)
{
	log_config("window", dpy, cfg);
	return eglCreateWindowSurface(dpy, cfg, win, attrib);
}

static EGLSurface w_eglCreatePbufferSurface(EGLDisplay dpy, EGLConfig cfg, const EGLint *attrib)
{
	log_config("pbuffer", dpy, cfg);
	return eglCreatePbufferSurface(dpy, cfg, attrib);
}

/* ---- the profile (GM2_PROFILE=1) ------------------------------------------ */

enum { PH_SETUP, PH_FRAMES, PH_TEARDOWN, PH_N };
static const char *const phase_name[PH_N] = { "setup", "frames", "teardown" };

/* The GPU work linux.c modelled on the CPU so far (src/osal/linux.c, in
 * every build of it; zeros where it models none) */
void osal_linux_gpu_time(uint64_t *wall_ns, uint64_t *cpu_ns, uint32_t *jobs);

/* The library's draw-step totals so far (src/osal/linux.c, every build): ns
 * a step in src/draw_steps.rs Step order, the laps the clock could not time,
 * the timed scopes reported. False when none was reported. */
enum { DRAW_STEPS = 12 };   /* the STEPS == 12 assert in src/draw_steps.rs */
bool osal_linux_draw_stats(uint64_t ns[DRAW_STEPS], uint32_t *untimed, uint64_t *scopes);

/* The totals at the first swap and at the last counted one */
struct draw_mark {
	uint64_t ns[DRAW_STEPS], scopes;
	uint32_t untimed;
	bool any;
};
static struct draw_mark d_mark0, d_mark1;

static void draw_mark_now(struct draw_mark *m)
{
	m->any = osal_linux_draw_stats(m->ns, &m->untimed, &m->scopes);
}

/* One entry point's calls and time, per phase; wall and cpu without the
 * modelled GPU work, which is gpu (wall ns) in gpu_jobs jobs */
struct gl_time {
	const char *name;
	unsigned long calls[PH_N];
	uint64_t wall[PH_N], cpu[PH_N], gpu[PH_N];
	unsigned long gpu_jobs[PH_N];
	unsigned long v3d_waits[PH_N]; /* the V3D's CPU waits for a job inside the calls */
};

/* The V3D back end's jobs submitted and CPU waits for one (src/v3d/
 * backend.rs), weak: the other GPUs' libraries have none, and the
 * v3d-frames line is then not printed. */
extern void v3d_counters(uint32_t *kicks, uint32_t *waits) __attribute__((weak));

/* The modelled GPU work at one moment, and the V3D's counters then */
struct gpu_mark {
	uint64_t wall, cpu;
	uint32_t jobs;
	uint32_t kicks, waits;
};

/* One timed call: the clocks at its start and the phase it began in */
struct tick {
	uint64_t wall, cpu;
	struct gpu_mark gpu;
	int phase, outer;
};

static int t_depth;            /* timed calls in progress */
static unsigned long t_swaps;  /* eglSwapBuffers calls returned */
static unsigned long t_frames; /* GM2_FRAMES: the swap that ends the counted frames; 0 = none set */
static uint64_t f_wall0, f_cpu0, f_wall1, f_cpu1, f_gl_wall;
static struct gpu_mark f_gpu0, f_gpu1;

static struct gpu_mark gpu_now(void)
{
	struct gpu_mark m = { 0 };

	osal_linux_gpu_time(&m.wall, &m.cpu, &m.jobs);
	if (v3d_counters)
		v3d_counters(&m.kicks, &m.waits);
	return m;
}
static struct gl_time *t_swap; /* eglSwapBuffers' record */
static int perf_ctl = -1, perf_ack = -1;

static uint64_t now_ns(clockid_t id)
{
	struct timespec ts;

	clock_gettime(id, &ts);
	return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}

static int phase_now(void)
{
	if (t_swaps == 0)
		return PH_SETUP;
	return t_frames && t_swaps >= t_frames ? PH_TEARDOWN : PH_FRAMES;
}

/* "enable" or "disable" to perf record's control fifo, and wait for its ack,
 * so the samples start and end at the swap */
static void perf_say(const char *what)
{
	char ack[8];

	if (perf_ctl < 0)
		return;
	if (write(perf_ctl, what, strlen(what)) < 0)
		fprintf(stderr, "gm2: perf control: write failed\n");
	else if (perf_ack >= 0 && read(perf_ack, ack, sizeof(ack)) <= 0)
		fprintf(stderr, "gm2: perf control: no ack\n");
}

static void tick_begin(struct tick *k)
{
	k->outer = t_depth++ == 0;
	if (!k->outer)
		return;
	k->phase = phase_now();
	k->gpu = gpu_now();
	k->cpu = now_ns(CLOCK_THREAD_CPUTIME_ID);
	k->wall = now_ns(CLOCK_MONOTONIC);
}

static void tick_end(const struct tick *k, struct gl_time *t)
{
	t_depth--;
	if (!k->outer)
		return;
	uint64_t wall = now_ns(CLOCK_MONOTONIC), cpu = now_ns(CLOCK_THREAD_CPUTIME_ID);
	const struct gpu_mark g = gpu_now();
	/* the modelled GPU work ran inside the call, so inside both clocks */
	const uint64_t gw = g.wall - k->gpu.wall, gc = g.cpu - k->gpu.cpu;
	const uint64_t dwall = wall - k->wall, dcpu = cpu - k->cpu;
	const uint64_t net_wall = dwall > gw ? dwall - gw : 0;

	t->calls[k->phase]++;
	t->wall[k->phase] += net_wall;
	t->cpu[k->phase] += dcpu > gc ? dcpu - gc : 0;
	t->gpu[k->phase] += gw;
	t->gpu_jobs[k->phase] += g.jobs - k->gpu.jobs;
	t->v3d_waits[k->phase] += g.waits - k->gpu.waits;
	if (k->phase == PH_FRAMES)
		f_gl_wall += net_wall;
	if (t != t_swap)
		return;
	t_swaps++;
	if (t_swaps == 1) {
		f_wall0 = wall;
		f_cpu0 = cpu;
		f_gpu0 = g;
		draw_mark_now(&d_mark0);
		perf_say("enable\n");
	}
	if (t_swaps > 1 && k->phase == PH_FRAMES) {
		f_wall1 = wall;
		f_cpu1 = cpu;
		f_gpu1 = g;
		draw_mark_now(&d_mark1);
		if (t_swaps == t_frames)
			perf_say("disable\n");
	}
}

/* The wrappers: p_<name> times the call and forwards it to r_<name>, what
 * lookup() would have answered for <name> without the profile. */
#define GM2_TIMED_VOID(name, params, args)                                                                   \
	static __typeof__(&name) r_##name;                                                                   \
	static struct gl_time t_##name = { #name, { 0 }, { 0 }, { 0 }, { 0 }, { 0 } };                         \
	static void p_##name params                                                                          \
	{                                                                                                    \
		struct tick k;                                                                               \
		tick_begin(&k);                                                                              \
		r_##name args;                                                                               \
		tick_end(&k, &t_##name);                                                                     \
	}
#define GM2_TIMED_RET(ret, name, params, args)                                                               \
	static __typeof__(&name) r_##name;                                                                   \
	static struct gl_time t_##name = { #name, { 0 }, { 0 }, { 0 }, { 0 }, { 0 } };                         \
	static ret p_##name params                                                                           \
	{                                                                                                    \
		struct tick k;                                                                               \
		tick_begin(&k);                                                                              \
		ret v = r_##name args;                                                                       \
		tick_end(&k, &t_##name);                                                                     \
		return v;                                                                                    \
	}
#include "gm2-timed.h"
#undef GM2_TIMED_VOID
#undef GM2_TIMED_RET

static const struct timed_entry {
	const char *name;
	void **real;
	void *fn;
	struct gl_time *t;
} T[] = {
#define GM2_TIMED_VOID(name, params, args) { #name, (void **)&r_##name, (void *)p_##name, &t_##name },
#define GM2_TIMED_RET(ret, name, params, args) { #name, (void **)&r_##name, (void *)p_##name, &t_##name },
#include "gm2-timed.h"
#undef GM2_TIMED_VOID
#undef GM2_TIMED_RET
};

static void profile_report(void)
{
	for (size_t i = 0; i < sizeof(T) / sizeof(T[0]); i++) {
		const struct gl_time *t = T[i].t;
		char line[512];
		int n = 0;

		if (!t->calls[PH_SETUP] && !t->calls[PH_FRAMES] && !t->calls[PH_TEARDOWN])
			continue;
		n = snprintf(line, sizeof(line), "gm2: gl-time %s", t->name);
		for (int p = 0; p < PH_N && n < (int)sizeof(line); p++)
			n += snprintf(line + n, sizeof(line) - n, " %s=%lu,%llu,%llu", phase_name[p], t->calls[p],
				      (unsigned long long)t->wall[p], (unsigned long long)t->cpu[p]);
		fprintf(stderr, "%s\n", line);
		if (t->v3d_waits[PH_SETUP] || t->v3d_waits[PH_FRAMES] || t->v3d_waits[PH_TEARDOWN])
			fprintf(stderr, "gm2: v3d-waits %s setup=%lu frames=%lu teardown=%lu\n", t->name,
				t->v3d_waits[PH_SETUP], t->v3d_waits[PH_FRAMES], t->v3d_waits[PH_TEARDOWN]);
		if (!t->gpu_jobs[PH_SETUP] && !t->gpu_jobs[PH_FRAMES] && !t->gpu_jobs[PH_TEARDOWN])
			continue;
		n = snprintf(line, sizeof(line), "gm2: gpu-time %s", t->name);
		for (int p = 0; p < PH_N && n < (int)sizeof(line); p++)
			n += snprintf(line + n, sizeof(line) - n, " %s=%lu,%llu", phase_name[p], t->gpu_jobs[p],
				      (unsigned long long)t->gpu[p]);
		fprintf(stderr, "%s\n", line);
	}
	unsigned long counted = t_swaps > 1 ? (t_frames && t_swaps > t_frames ? t_frames : t_swaps) - 1 : 0;
	/* the frames' clocks without the modelled GPU work between the swaps */
	uint64_t gw = counted ? f_gpu1.wall - f_gpu0.wall : 0, gc = counted ? f_gpu1.cpu - f_gpu0.cpu : 0;
	fprintf(stderr,
		"gm2: gl-frames swaps=%lu counted=%lu wall_ns=%llu cpu_ns=%llu gl_wall_ns=%llu gpu_jobs=%lu "
		"gpu_ns=%llu\n",
		t_swaps, counted, (unsigned long long)(counted ? f_wall1 - f_wall0 - gw : 0),
		(unsigned long long)(counted ? f_cpu1 - f_cpu0 - gc : 0), (unsigned long long)f_gl_wall,
		(unsigned long)(counted ? f_gpu1.jobs - f_gpu0.jobs : 0), (unsigned long long)gw);
	if (counted && d_mark1.any) {
		/* the counted frames only: the totals between the two swaps. A mark
		 * taken before the first draw reported has no totals yet (zeros). */
		fprintf(stderr, "gm2: draw-steps counted=%lu scopes=%llu untimed=%lu ns=", counted,
			(unsigned long long)(d_mark1.scopes - d_mark0.scopes),
			(unsigned long)(d_mark1.untimed - d_mark0.untimed));
		for (int k = 0; k < DRAW_STEPS; k++)
			fprintf(stderr, "%s%llu", k ? "," : "", (unsigned long long)(d_mark1.ns[k] - d_mark0.ns[k]));
		fputc('\n', stderr);
	}
	if (v3d_counters)
		fprintf(stderr, "gm2: v3d-frames counted=%lu kicks=%lu waits=%lu\n", counted,
			(unsigned long)(counted ? f_gpu1.kicks - f_gpu0.kicks : 0),
			(unsigned long)(counted ? f_gpu1.waits - f_gpu0.waits : 0));
}

/* 1 once GM2_PROFILE=1 is read (and the report is registered), else 0 */
static int profile_on(void)
{
	static int on = -1;

	if (on >= 0)
		return on;
	const char *e = getenv("GM2_PROFILE");

	on = e && strcmp(e, "1") == 0;
	if (!on)
		return on;
	e = getenv("GM2_FRAMES");
	t_frames = e ? strtoul(e, NULL, 10) : 0;
	e = getenv("GM2_PERF_CTL");
	if (e && *e) {
		perf_ctl = open(e, O_WRONLY | O_CLOEXEC);
		if (perf_ctl < 0)
			fprintf(stderr, "gm2: perf control: cannot open %s\n", e);
		e = getenv("GM2_PERF_ACK");
		if (perf_ctl >= 0 && e && *e)
			perf_ack = open(e, O_RDONLY | O_CLOEXEC);
	}
	atexit(profile_report);
	return on;
}

/* fn, the entry point lookup() found for name, or its timing wrapper */
static void *timed(const char *name, void *fn)
{
	if (!fn || !profile_on())
		return fn;
	for (size_t i = 0; i < sizeof(T) / sizeof(T[0]); i++) {
		if (strcmp(T[i].name, name))
			continue;
		*T[i].real = fn;
		if (!strcmp(name, "eglSwapBuffers"))
			t_swap = T[i].t;
		return T[i].fn;
	}
	return fn;
}

/* ---- dlopen / dlsym / eglGetProcAddress ----------------------------------- */

static __eglMustCastToProperFunctionPointerType w_eglGetProcAddress(const char *name);

static const struct { const char *name; void *fn; } W[] = {
	{ "glCreateShader", (void *)w_glCreateShader },
	{ "glShaderSource", (void *)w_glShaderSource },
	{ "glCompileShader", (void *)w_glCompileShader },
	{ "glDeleteShader", (void *)w_glDeleteShader },
	{ "glLinkProgram", (void *)w_glLinkProgram },
	{ "glGetError", (void *)w_glGetError },
	{ "glCheckFramebufferStatus", (void *)w_glCheckFramebufferStatus },
	{ "glDrawArrays", (void *)w_glDrawArrays },
	{ "glDrawElements", (void *)w_glDrawElements },
	{ "eglGetError", (void *)w_eglGetError },
	{ "eglCreateWindowSurface", (void *)w_eglCreateWindowSurface },
	{ "eglCreatePbufferSurface", (void *)w_eglCreatePbufferSurface },
	{ "eglGetProcAddress", (void *)w_eglGetProcAddress },
};

static void *lookup(const char *name)
{
	for (size_t i = 0; i < sizeof(W) / sizeof(W[0]); i++)
		if (!strcmp(W[i].name, name))
			return timed(name, W[i].fn);
	return timed(name, (void *)eglGetProcAddress(name));
}

static __eglMustCastToProperFunctionPointerType w_eglGetProcAddress(const char *name)
{
	return (__eglMustCastToProperFunctionPointerType)lookup(name);
}

static int s_lib;
static char s_dlerror[160];
static int s_dlerror_set;

void *gm2_dlopen(const char *name, int flags)
{
	(void)flags;
	/* the names gl-state-egl.cpp tries: libEGL.so[.1], libGLESv2.so[.2] */
	if (name && (!strncmp(name, "libEGL.so", 9) || !strncmp(name, "libGLESv2.so", 12)))
		return &s_lib;
	snprintf(s_dlerror, sizeof(s_dlerror), "%s: not YakoGL", name ? name : "(main program)");
	s_dlerror_set = 1;
	return NULL;
}

void *gm2_dlsym(void *handle, const char *symbol)
{
	void *f;

	if (handle != &s_lib) {
		snprintf(s_dlerror, sizeof(s_dlerror), "dlsym: not a handle of dlopen");
		s_dlerror_set = 1;
		return NULL;
	}
	f = lookup(symbol);
	if (!f) {
		snprintf(s_dlerror, sizeof(s_dlerror), "%s: undefined symbol", symbol);
		s_dlerror_set = 1;
	}
	return f;
}

int gm2_dlclose(void *handle)
{
	return handle == &s_lib ? 0 : -1;
}

char *gm2_dlerror(void)
{
	if (!s_dlerror_set)
		return NULL;
	s_dlerror_set = 0;
	return s_dlerror;
}
