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
 *
 * With GM2_GCCOST=1 the library calls the OSAL through a table of this file
 * (the gc-cost section below), which counts what a GC7000 run would make the
 * board's CPU pay its GPU: the waits, the kicks and their streams decoded,
 * the cache maintenance bytes. The timing wrappers are on too (for the call
 * in progress and the swaps), without the profile's lines unless
 * GM2_PROFILE=1. Per kick, and at exit:
 *
 *   gm2: gc-kick <phase> <name> fence=<f> bytes=<b> draws=<d> box_px=<p>
 *        target=<w>x<h> stride=<s> rts=<n> rs=<n> rs_px=<p> [undecoded]
 *   gm2: gc-cost counted=<n> gpu=<lag|done> kicks= waits= swap_waits=
 *        invd=<calls>,<bytes> flush=<calls>,<bytes> foreign_bytes=
 *        stream_bytes= draws= box_px= rs= rs_px=
 *   gm2: gc-cost-entry <name> waits= kicks= invd=<c>,<b> flush=<c>,<b>
 *   gm2: gc-wait <name> <caller<chain> setup=<n> frames=<n> teardown=<n>
 *
 * gc-cost and gc-cost-entry count the counted frames, as gl-frames does.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <execinfo.h>
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
#include <gles_osal.h>

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
	/* GM2_GCCOST: the GC7000's waits, kicks and cache maintenance inside
	 * the calls, in the counted frames only (the gc-cost section below) */
	unsigned long gc_waits, gc_kicks, gc_invd_calls, gc_flush_calls;
	uint64_t gc_invd_bytes, gc_flush_bytes;
};

/* The timed call in progress (the outermost one), NULL between calls */
static struct gl_time *t_cur;
static void gc_mark(int last);

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

static void tick_begin(struct tick *k, struct gl_time *t)
{
	k->outer = t_depth++ == 0;
	if (!k->outer)
		return;
	t_cur = t;
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
	t_cur = NULL;
	if (t != t_swap)
		return;
	t_swaps++;
	if (t_swaps == 1) {
		f_wall0 = wall;
		f_cpu0 = cpu;
		f_gpu0 = g;
		draw_mark_now(&d_mark0);
		gc_mark(0);
		perf_say("enable\n");
	}
	if (t_swaps > 1 && k->phase == PH_FRAMES) {
		f_wall1 = wall;
		f_cpu1 = cpu;
		f_gpu1 = g;
		draw_mark_now(&d_mark1);
		gc_mark(1);
		if (t_swaps == t_frames)
			perf_say("disable\n");
	}
}

/* The wrappers: p_<name> times the call and forwards it to r_<name>, what
 * lookup() would have answered for <name> without the profile. */
#define GM2_TIMED_VOID(name, params, args)                                                                   \
	static __typeof__(&name) r_##name;                                                                   \
	static struct gl_time t_##name = { #name };                                                          \
	static void p_##name params                                                                          \
	{                                                                                                    \
		struct tick k;                                                                               \
		tick_begin(&k, &t_##name);                                                                   \
		r_##name args;                                                                               \
		tick_end(&k, &t_##name);                                                                     \
	}
#define GM2_TIMED_RET(ret, name, params, args)                                                               \
	static __typeof__(&name) r_##name;                                                                   \
	static struct gl_time t_##name = { #name };                                                          \
	static ret p_##name params                                                                           \
	{                                                                                                    \
		struct tick k;                                                                               \
		tick_begin(&k, &t_##name);                                                                   \
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

/* GM2_FRAMES: the swap that ends the counted frames */
static void frames_from_env(void)
{
	const char *e = getenv("GM2_FRAMES");

	t_frames = e ? strtoul(e, NULL, 10) : 0;
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
	frames_from_env();
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

static int gccost_on(void);

/* fn, the entry point lookup() found for name, or its timing wrapper (the
 * profile's, and GM2_GCCOST's, which needs the call in progress and the
 * swaps) */
static void *timed(const char *name, void *fn)
{
	if (!fn || !(profile_on() | gccost_on()))
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

/* ---- the GC7000's costs (GM2_GCCOST=1) ------------------------------------ */

/* The OSAL table the library calls through (src/osal/mod.rs g_osal, set
 * with osal_set_ops). The hooks below are installed on every build; what
 * makes a run the GC7000's is its first kick descriptor (GC_KICK_MAGIC): a
 * PowerVR or V3D run never sets gc_seen, keeps linux.c's fences and prints
 * nothing. (The back end's own kick and wait counters, gc_counters in
 * src/vivante/backend.rs, are not exported, so the shim counts at the
 * OSAL.) */
extern void osal_set_ops(const struct osal_ops *ops);

/* gc_kick.h's descriptor (src/vivante/gc_kick.h), mirrored: the shim is
 * built once for every GPU and does not take the back end's headers */
#define GC_KICK_MAGIC 0x474b4b43u
struct gc_kick_desc {
	uint32_t magic, stream_bytes;
	uint64_t stream_gpu;
};

/* The registers the kick decode reads (src/vivante/regs.rs) */
enum {
	GC_OP_LOAD_STATE = 0x01, GC_OP_STALL = 0x09, GC_OP_DRAW_INSTANCED = 0x0c, /* the top five bits */
	GC_SE_SCISSOR_LEFT = 0x0c00, GC_SE_SCISSOR_TOP = 0x0c04,
	GC_SE_CLIP_RIGHT = 0x0c20, GC_SE_CLIP_BOTTOM = 0x0c24,
	GC_PE_COLOR_STRIDE = 0x1434, GC_PE_PIPE_COLOR_ADDR0 = 0x1460,
	GC_RS_KICKER = 0x1600, GC_RS_WINDOW_SIZE = 0x1620,
};

static const struct osal_ops *gc_real; /* the library's table, NULL when off */
static struct osal_ops gc_ops;         /* it, with the hooks below */
static int gc_lag = 1;                 /* GM2_GCCOST_GPU=done: 0 */
static int gc_seen;                    /* a GC7000 kick was submitted */
static int64_t gc_done;                /* the fences the lagging GPU has completed */

/* What the CPU paid the GPU so far (the frames' are the difference of the
 * values at the first swap and at the last counted one) */
struct gc_count {
	unsigned long waits, swap_waits, kicks, draws, rs, invd_calls, flush_calls;
	uint64_t invd_bytes, flush_bytes, foreign_bytes, stream_bytes, box_px, rs_px;
};
static struct gc_count gc_now, gc_m0, gc_m1;

/* The GPU memory the library holds: its CPU view, for the decode and to
 * tell a carve-out range from another */
struct gc_map {
	void *cpu;
	uint64_t gpu;
	size_t size;
};
static struct gc_map *gc_maps;
static size_t gc_nmaps, gc_capmaps;

/* The callers of the waits: the entry point and the library's frames */
enum { GC_WAIT_PCS = 6, GC_WAIT_SITES = 128 };
struct gc_wait_site {
	const char *entry;
	void *pc[GC_WAIT_PCS];
	unsigned long n[PH_N];
};
static struct gc_wait_site gc_sites[GC_WAIT_SITES];
static size_t gc_nsites;
static unsigned long gc_sites_lost;

/* Outside every timed call: GL work the library does on its own (none in
 * glmark2), or the init and shutdown */
static struct gl_time t_gc_none = { .name = "(none)" };

static struct gl_time *gc_entry(void)
{
	return t_cur ? t_cur : &t_gc_none;
}

static void gc_mark(int last)
{
	if (!gc_real)
		return;
	if (last)
		gc_m1 = gc_now;
	else
		gc_m0 = gc_now;
}

static const struct gc_map *gc_map_cpu(const void *p, size_t n)
{
	const uint8_t *b = p;

	for (size_t i = 0; i < gc_nmaps; i++) {
		const uint8_t *c = gc_maps[i].cpu;
		if (c && b >= c && b < c + gc_maps[i].size && n <= gc_maps[i].size - (size_t)(b - c))
			return &gc_maps[i];
	}
	return NULL;
}

static const uint32_t *gc_words_of(uint64_t gpu, size_t bytes)
{
	for (size_t i = 0; i < gc_nmaps; i++) {
		const struct gc_map *m = &gc_maps[i];
		if (m->cpu && gpu >= m->gpu && gpu < m->gpu + m->size && bytes <= m->size - (gpu - m->gpu))
			return (const uint32_t *)((uint8_t *)m->cpu + (gpu - m->gpu));
	}
	return NULL;
}

static int gc_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	int rc = gc_real->gpu_alloc(heap_base, size, flags, out);

	if (rc != 0 || !out)
		return rc;
	if (gc_nmaps == gc_capmaps) {
		gc_capmaps = gc_capmaps ? 2 * gc_capmaps : 256;
		gc_maps = realloc(gc_maps, gc_capmaps * sizeof(*gc_maps));
		if (!gc_maps)
			abort();
	}
	gc_maps[gc_nmaps++] = (struct gc_map){ out->cpu, out->gpu, out->size };
	return rc;
}

static int gc_free(struct osal_gpu_mem *mem)
{
	if (mem && mem->size) {
		for (size_t i = 0; i < gc_nmaps; i++) {
			if (gc_maps[i].gpu == mem->gpu && gc_maps[i].cpu == mem->cpu) {
				gc_maps[i] = gc_maps[--gc_nmaps];
				break;
			}
		}
	}
	return gc_real->gpu_free(mem);
}

/* A range the CPU writes back or drops: on the board each is DC range
 * maintenance over the carve-out, and the bytes stand for a CPU pass over
 * the memory around it (Normal non-cacheable there) */
static void gc_cache(int flush, void *cpu, size_t n)
{
	struct gl_time *t = gc_entry();

	if (!gc_map_cpu(cpu, n))
		gc_now.foreign_bytes += n;
	if (flush) {
		gc_now.flush_calls++;
		gc_now.flush_bytes += n;
	} else {
		gc_now.invd_calls++;
		gc_now.invd_bytes += n;
	}
	if (phase_now() == PH_FRAMES) {
		if (flush) {
			t->gc_flush_calls++;
			t->gc_flush_bytes += n;
		} else {
			t->gc_invd_calls++;
			t->gc_invd_bytes += n;
		}
	}
}

static void gc_cache_flush(void *cpu, size_t n)
{
	gc_cache(1, cpu, n);
	gc_real->cache_flush(cpu, n);
}

static void gc_cache_invd(void *cpu, size_t n)
{
	gc_cache(0, cpu, n);
	gc_real->cache_invd(cpu, n);
}

/* One kick's stream, decoded: the draws (DRAW_INSTANCED) and each one's
 * clip box (SE_SCISSOR_LEFT/TOP to SE_CLIP_RIGHT/BOTTOM in force at the
 * draw: the viewport and scissor box in the target's samples, an upper bound
 * of the pixels it covers, which the stream alone cannot give), the largest
 * box's extent as the target's size, the colour targets and their stride,
 * and the resolve engine's blits (RS_KICKER) with their windows. The back end
 * drops the state it already wrote within a kick (dedup.rs) and starts each
 * kick afresh, so the state is followed across the kick and reset at its
 * start. */
struct gc_kick_stats {
	unsigned long draws, rs, nodraw_box;
	uint64_t box_px, rs_px;
	uint32_t w, h, stride, rts;
};

static void gc_decode(const uint32_t *wd, size_t n, struct gc_kick_stats *s)
{
	uint32_t left = 0, top = 0, right = 0, bottom = 0, rs_win = 0, colour[8] = { 0 };
	int have_clip = 0;

	memset(s, 0, sizeof(*s));
	for (size_t i = 0; i < n;) {
		const uint32_t w = wd[i], op = w >> 27;

		if (op == GC_OP_LOAD_STATE) {
			uint32_t count = (w >> 16) & 0x3ff, reg = (w & 0xffff) * 4;

			if (count == 0)
				count = 0x400;
			for (uint32_t k = 0; k < count && i + 1 + k < n; k++, reg += 4) {
				const uint32_t v = wd[i + 1 + k];

				switch (reg) {
				case GC_SE_SCISSOR_LEFT: left = v >> 16; break;
				case GC_SE_SCISSOR_TOP: top = v >> 16; break;
				case GC_SE_CLIP_RIGHT: right = v >> 16; have_clip = 1; break;
				case GC_SE_CLIP_BOTTOM: bottom = v >> 16; have_clip = 1; break;
				case GC_PE_COLOR_STRIDE: s->stride = v; break;
				case GC_RS_WINDOW_SIZE: rs_win = v; break;
				case GC_RS_KICKER:
					s->rs++;
					s->rs_px += (uint64_t)(rs_win & 0xffff) * (rs_win >> 16);
					break;
				default:
					if (reg == GC_PE_PIPE_COLOR_ADDR0 && v) {
						int seen = 0;
						for (uint32_t r = 0; r < s->rts && r < 8; r++)
							seen |= colour[r] == v;
						if (!seen && s->rts < 8)
							colour[s->rts++] = v;
					}
				}
			}
			i += 1 + count + ((1 + count) & 1); /* padded to 64 bits with a NOP */
			continue;
		}
		if (op == GC_OP_DRAW_INSTANCED) {
			s->draws++;
			if (have_clip && right > left && bottom > top) {
				s->box_px += (uint64_t)(right - left) * (bottom - top);
				if (right > s->w)
					s->w = right;
				if (bottom > s->h)
					s->h = bottom;
			} else if (!have_clip) {
				s->nodraw_box++;
			}
			i += 4;
			continue;
		}
		i += op == GC_OP_STALL ? 2 : 1;
	}
}

static int64_t gc_submit(const void *desc, size_t len)
{
	const struct gc_kick_desc *k = desc;
	struct gl_time *t = gc_entry();
	struct gc_kick_stats s = { 0 };
	const uint32_t *wd = NULL;
	const int ph = phase_now();

	if (!k || len < sizeof(*k) || k->magic != GC_KICK_MAGIC)
		return gc_real->submit(desc, len);
	gc_seen = 1;
	wd = gc_words_of(k->stream_gpu, k->stream_bytes);
	if (wd)
		gc_decode(wd, k->stream_bytes / 4, &s);
	const int64_t fence = gc_real->submit(desc, len);
	if (fence < 0)
		return fence;
	gc_now.kicks++;
	gc_now.draws += s.draws;
	gc_now.box_px += s.box_px;
	gc_now.rs += s.rs;
	gc_now.rs_px += s.rs_px;
	gc_now.stream_bytes += wd ? k->stream_bytes : 0;
	if (ph == PH_FRAMES)
		t->gc_kicks++;
	fprintf(stderr,
		"gm2: gc-kick %s %s fence=%lld bytes=%u draws=%lu box_px=%llu target=%ux%u stride=%u rts=%u "
		"rs=%lu rs_px=%llu%s\n",
		phase_name[ph], t->name, (long long)fence, wd ? k->stream_bytes : 0, s.draws,
		(unsigned long long)s.box_px, s.w, s.h, s.stride, s.rts, s.rs, (unsigned long long)s.rs_px,
		wd ? "" : " undecoded");
	return fence;
}

/* GM2_GCCOST_GPU=lag (the default): the GPU completes a kick only when the
 * CPU waits for it, or for a later one (fences are ordered): what the
 * i.MX8MP does where its GPU takes longer than the CPU's recording, as in
 * terrain, shadow and refract. Every point where the back end checks a
 * fence then is a wait, so the count is the most waits the board's CPU can
 * pay for the frame. GM2_GCCOST_GPU=done keeps linux.c's answer (every
 * submit completed at once): no wait is reached, as without the switch. */
static int64_t gc_completed(void)
{
	const int64_t real = gc_real->completed();

	return gc_seen && gc_lag && gc_done < real ? gc_done : real;
}

static void gc_wait_site(void *const pc[GC_WAIT_PCS], const char *entry, int ph)
{
	for (size_t i = 0; i < gc_nsites; i++) {
		if (gc_sites[i].entry == entry && !memcmp(gc_sites[i].pc, pc, sizeof(gc_sites[i].pc))) {
			gc_sites[i].n[ph]++;
			return;
		}
	}
	if (gc_nsites == GC_WAIT_SITES) {
		gc_sites_lost++;
		return;
	}
	struct gc_wait_site *w = &gc_sites[gc_nsites++];
	w->entry = entry;
	memcpy(w->pc, pc, sizeof(w->pc));
	w->n[ph] = 1;
}

static int gc_wait(int64_t fence, int64_t timeout_us)
{
	struct gl_time *t = gc_entry();
	void *bt[GC_WAIT_PCS + 1] = { 0 };
	const int ph = phase_now();

	if (!gc_seen)
		return gc_real->wait(fence, timeout_us);
	/* the library's frames above this one (this one is the shim's) */
	backtrace(bt, GC_WAIT_PCS + 1);
	gc_wait_site(bt + 1, t->name, ph);
	gc_now.waits++;
	if (t == t_swap)
		gc_now.swap_waits++;
	if (ph == PH_FRAMES)
		t->gc_waits++;
	const int rc = gc_real->wait(fence, timeout_us);
	if (rc == 0 && fence > gc_done)
		gc_done = fence;
	return rc;
}

/* The library's function at a return address: addr2line over the program
 * (one process for every address), "+0x<offset>" where it has no name */
static void gc_resolve(char names[][GC_WAIT_PCS][96], size_t nsites)
{
	char cmd[16384];
	int n = snprintf(cmd, sizeof(cmd), "addr2line -f -C -e /proc/%d/exe", (int)getpid());
	uintptr_t base = 0;
	Dl_info di;

	if (dladdr((void *)gc_resolve, &di) && di.dli_fbase)
		base = (uintptr_t)di.dli_fbase;
	for (size_t i = 0; i < nsites; i++) {
		for (int k = 0; k < GC_WAIT_PCS; k++) {
			/* a return address is past its call: the call is one byte back */
			uintptr_t off = gc_sites[i].pc[k] ? (uintptr_t)gc_sites[i].pc[k] - base - 1 : 0;
			snprintf(names[i][k], sizeof(names[i][k]), "+0x%lx", (unsigned long)off);
			if (n < (int)sizeof(cmd) - 24)
				n += snprintf(cmd + n, sizeof(cmd) - n, " 0x%lx", (unsigned long)off);
		}
	}
	FILE *p = popen(cmd, "r");
	if (!p)
		return;
	char fn[1024], loc[1024];
	for (size_t i = 0; i < nsites; i++) {
		for (int k = 0; k < GC_WAIT_PCS; k++) {
			if (!fgets(fn, sizeof(fn), p) || !fgets(loc, sizeof(loc), p))
				goto done;
			fn[strcspn(fn, "\n")] = '\0';
			if (!strcmp(fn, "??") || !*fn)
				continue;
			/* yakogl::vivante::backend::frame_open::h0123456789abcdef: the
			 * last path segment, no hash, no spaces */
			char *h = strstr(fn, "::h");
			if (h && strlen(h) == 19)
				*h = '\0';
			char *last = fn;
			for (char *c = fn; (c = strstr(c, "::")); c += 2)
				last = c + 2;
			for (char *c = last; *c; c++)
				if (*c == ' ')
					*c = '_';
			snprintf(names[i][k], sizeof(names[i][k]), "%.95s", last);
		}
	}
done:
	pclose(p);
}

/* What the counted frames paid: a summary, the entry points' share, and the
 * waits' callers (every phase, frames apart). Formats in README.md. */
static void gccost_report(void)
{
	const unsigned long counted =
		t_swaps > 1 ? (t_frames && t_swaps > t_frames ? t_frames : t_swaps) - 1 : 0;
	const struct gc_count z = { 0 }, *a = counted ? &gc_m0 : &z, *b = counted ? &gc_m1 : &z;

	if (!gc_seen)
		return;
	fprintf(stderr,
		"gm2: gc-cost counted=%lu gpu=%s kicks=%lu waits=%lu swap_waits=%lu "
		"invd=%lu,%llu flush=%lu,%llu foreign_bytes=%llu stream_bytes=%llu draws=%lu box_px=%llu "
		"rs=%lu rs_px=%llu\n",
		counted, gc_lag ? "lag" : "done", b->kicks - a->kicks, b->waits - a->waits,
		b->swap_waits - a->swap_waits,
		b->invd_calls - a->invd_calls, (unsigned long long)(b->invd_bytes - a->invd_bytes),
		b->flush_calls - a->flush_calls, (unsigned long long)(b->flush_bytes - a->flush_bytes),
		(unsigned long long)(b->foreign_bytes - a->foreign_bytes),
		(unsigned long long)(b->stream_bytes - a->stream_bytes), b->draws - a->draws,
		(unsigned long long)(b->box_px - a->box_px), b->rs - a->rs, (unsigned long long)(b->rs_px - a->rs_px));
	for (size_t i = 0; i <= sizeof(T) / sizeof(T[0]); i++) {
		const struct gl_time *t = i < sizeof(T) / sizeof(T[0]) ? T[i].t : &t_gc_none;

		if (!t->gc_waits && !t->gc_kicks && !t->gc_invd_calls && !t->gc_flush_calls)
			continue;
		fprintf(stderr, "gm2: gc-cost-entry %s waits=%lu kicks=%lu invd=%lu,%llu flush=%lu,%llu\n", t->name,
			t->gc_waits, t->gc_kicks, t->gc_invd_calls, (unsigned long long)t->gc_invd_bytes,
			t->gc_flush_calls, (unsigned long long)t->gc_flush_bytes);
	}
	if (!gc_nsites)
		return;
	char (*names)[GC_WAIT_PCS][96] = calloc(gc_nsites, sizeof(*names));
	if (!names)
		return;
	gc_resolve(names, gc_nsites);
	for (size_t i = 0; i < gc_nsites; i++) {
		char chain[GC_WAIT_PCS * 97] = "";
		for (int k = 0; k < GC_WAIT_PCS; k++)
			snprintf(chain + strlen(chain), sizeof(chain) - strlen(chain), "%s%s", k ? "<" : "", names[i][k]);
		fprintf(stderr, "gm2: gc-wait %s %s setup=%lu frames=%lu teardown=%lu\n", gc_sites[i].entry, chain,
			gc_sites[i].n[PH_SETUP], gc_sites[i].n[PH_FRAMES], gc_sites[i].n[PH_TEARDOWN]);
	}
	if (gc_sites_lost)
		fprintf(stderr, "gm2: gc-wait (more than %d sites) lost=%lu\n", GC_WAIT_SITES, gc_sites_lost);
	free(names);
}

/* 1 when GM2_GCCOST=1: the library's OSAL table is then this section's,
 * installed before the library's first call (the constructor below), else 0 */
static int gccost_on(void)
{
	static int on = -1;

	if (on >= 0)
		return on;
	const char *e = getenv("GM2_GCCOST");

	on = e && strcmp(e, "1") == 0 && g_osal;
	if (!on)
		return on;
	e = getenv("GM2_GCCOST_GPU");
	gc_lag = !(e && strcmp(e, "done") == 0);
	frames_from_env();
	gc_real = g_osal;
	gc_ops = *gc_real;
	gc_ops.gpu_alloc = gc_alloc;
	gc_ops.gpu_free = gc_free;
	gc_ops.cache_flush = gc_cache_flush;
	gc_ops.cache_invd = gc_cache_invd;
	gc_ops.submit = gc_submit;
	gc_ops.wait = gc_wait;
	gc_ops.completed = gc_completed;
	osal_set_ops(&gc_ops);
	atexit(gccost_report);
	return on;
}

__attribute__((constructor)) static void gccost_init(void)
{
	gccost_on();
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
