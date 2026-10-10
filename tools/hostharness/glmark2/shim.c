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
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

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
			return W[i].fn;
	return (void *)eglGetProcAddress(name);
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
