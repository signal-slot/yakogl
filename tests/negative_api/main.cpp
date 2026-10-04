/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * negative_api - run the dEQP-GLES2 functional.negative_api cases on the host.
 *
 * These 246 cases only check which GL error each bad call raises, so they need
 * no GPU: the linux OSAL gives the core memory and completes every submit at
 * once. Build with -DYAKOGL_OSAL=linux and run; each case prints PASS or FAIL
 * with the mismatching error. See tests/negative_api/README.md.
 */
#include "deqp_shim.hpp"

#include <EGL/egl.h>
#include <cstdlib>
#include <cstring>

const char *g_cur_group = "";
const char *g_cur_name = "";
int g_cur_fails;
int g_cur_checks;
bool g_verbose = true;
DeqpLogSink m_log;
DeqpContext m_context;
DeqpTestCtx m_testCtx;
DeqpResult g_cur_result;
tcu::TestLog::Tag tcu::TestLog::Message;
tcu::TestLog::Tag tcu::TestLog::EndMessage;
tcu::TestLog::Tag tcu::TestLog::EndSection;

const char *deqp_error_name(GLenum e)
{
	switch (e) {
	case GL_NO_ERROR:                      return "GL_NO_ERROR";
	case GL_INVALID_ENUM:                  return "GL_INVALID_ENUM";
	case GL_INVALID_VALUE:                 return "GL_INVALID_VALUE";
	case GL_INVALID_OPERATION:             return "GL_INVALID_OPERATION";
	case GL_OUT_OF_MEMORY:                 return "GL_OUT_OF_MEMORY";
	case GL_INVALID_FRAMEBUFFER_OPERATION: return "GL_INVALID_FRAMEBUFFER_OPERATION";
	default:                               return "an unknown error";
	}
}

#include "cases.inc"

int main(int argc, char **argv)
{
	const char *filter = argc > 1 ? argv[1] : NULL;
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLint major = 0, minor = 0;

	if (!eglInitialize(dpy, &major, &minor)) {
		fprintf(stderr, "eglInitialize failed\n");
		return 2;
	}

	static const EGLint cfg_attrs[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_DEPTH_SIZE, 16, EGL_STENCIL_SIZE, 8, EGL_NONE
	};
	EGLConfig cfg;
	EGLint ncfg = 0;

	if (!eglChooseConfig(dpy, cfg_attrs, &cfg, 1, &ncfg) || ncfg == 0) {
		fprintf(stderr, "eglChooseConfig found no config\n");
		return 2;
	}

	static const EGLint pb_attrs[] = { EGL_WIDTH, 128, EGL_HEIGHT, 128, EGL_NONE };
	EGLSurface surf = eglCreatePbufferSurface(dpy, cfg, pb_attrs);
	static const EGLint ctx_attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attrs);

	if (surf == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT ||
	    !eglMakeCurrent(dpy, surf, surf, ctx)) {
		fprintf(stderr, "no current context (egl error 0x%x)\n", eglGetError());
		return 2;
	}

	printf("GL_VERSION  = %s\n", (const char *)glGetString(GL_VERSION));
	printf("GL_SHADING_LANGUAGE_VERSION = %s\n",
	       (const char *)glGetString(GL_SHADING_LANGUAGE_VERSION));
	printf("GL_EXTENSIONS = %s\n\n", (const char *)glGetString(GL_EXTENSIONS));

	int pass = 0, fail = 0, skipped = 0;

	for (size_t i = 0; i < sizeof(g_deqp_cases) / sizeof(g_deqp_cases[0]); i++) {
		const DeqpCase *c = &g_deqp_cases[i];
		char full[128];

		snprintf(full, sizeof(full), "%s.%s", c->group, c->name);
		if (filter && !strstr(full, filter)) {
			skipped++;
			continue;
		}
		/* dEQP gives every case its own context. Do the same: a case that
		 * leaves an incomplete framebuffer bound (buffer.read_pixels does)
		 * would otherwise fail the next one. */
		eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
		eglDestroyContext(dpy, ctx);
		ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attrs);
		if (ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, surf, surf, ctx)) {
			fprintf(stderr, "%s: lost the context (egl error 0x%x)\n", full, eglGetError());
			return 2;
		}

		g_cur_group = c->group;
		g_cur_name = c->name;
		g_cur_fails = 0;
		g_cur_checks = 0;
		g_cur_result = QP_TEST_RESULT_PASS;
		while (glGetError() != GL_NO_ERROR)
			;
		c->run();
		if (g_cur_result == QP_TEST_RESULT_NOT_SUPPORTED) {
			pass++;
			printf("negative_api.%s: PASS (not supported)\n", full);
		} else if (g_cur_fails == 0 && g_cur_result == QP_TEST_RESULT_PASS) {
			pass++;
			printf("negative_api.%s: PASS (%d checks)\n", full, g_cur_checks);
		} else {
			fail++;
			printf("negative_api.%s: FAIL (%d of %d checks)\n", full, g_cur_fails ? g_cur_fails : 1, g_cur_checks);
		}
	}

	printf("\nnegative_api: %d passed, %d failed", pass, fail);
	if (skipped)
		printf(", %d not selected", skipped);
	printf("\n");

	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglTerminate(dpy);
	return fail ? 1 : 0;
}
