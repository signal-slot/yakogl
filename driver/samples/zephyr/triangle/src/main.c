/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * triangle: the one triangle sample, the same source on the Raspberry Pi 4
 * (V3D), the Verdin AM62P (PowerVR) and the Verdin iMX8M Plus (GC7000)
 * (driver/docs/one-method.md, section 5, step 3). The board is chosen by the
 * Zephyr board only; everything that differs per board is behind
 * yakogl_board.h.
 *
 * One Gouraud triangle (red top, green bottom left, blue bottom right) drawn
 * through the public OpenGL ES 2.0 API on YakoGL, into a 1024x600 window.
 * This file is the thin part: the window, EGL/GL, the screen and the
 * records. What is drawn and what is expected are the check crate's
 * (check/src/lib.rs, through rust/src/lib.rs), one definition for both:
 *
 *   triangle_scene()                     vertices, clear colour, shader
 *                                        texts, window size, fill words
 *   board_window(1024, 600 + guard rows) sentinel window, guard rows
 *   EGL up (RGBA8888, ES 2)              the window's 1024x600 only
 *   compile + link, one draw, eglSwapBuffers, board_show()
 *   triangle_check()                     every pixel classified (inside /
 *                                        outside / edge), guard rows,
 *                                        glReadPixels against memory, and
 *                                        PICTURE's words
 *   board_result(PICTURE)                gc_gl_triangle's packing
 *   board_result(APP, fail)              only after a failed check
 *   board_finish()
 *
 * The records: the last record of a run is its verdict (yakogl_board.h). A
 * checked frame stores PICTURE (centre pixel, column CRC, size; record.rs
 * kind 4). After a pass that is the last record. After a failed check an APP
 * record follows it: w1 = the check's failure code << 8 | 2 (fail), the codes
 * being check/src/lib.rs `Failure` (1 counts, 2 inside, 3 outside, 4 edge,
 * 5 guard, 6 glReadPixels); w2 = 1 frame presented; w3 = the GPU resets. A
 * run that never reached the check stores no PICTURE, and board_finish()
 * stores its APP fail record.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <stdint.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

/* Where a run that died stopped (kind PROGRESS, YK_RECORD_WHERE_STAGE | n). */
enum stage {
	STAGE_WINDOW = 1,
	STAGE_EGL,
	STAGE_PROGRAM,
	STAGE_DRAW,
	STAGE_SWAPPED,
	STAGE_CHECKED,
};

/* ---- rust/src/lib.rs ------------------------------------------------------ */

struct triangle_scene {
	float verts[24]; /* x, y, z, w, r, g, b, a per vertex */
	float clear[4];
	const char *vs, *fs; /* NUL-terminated GLSL ES 1.00 */
	uint32_t w, h, guard_rows, sentinel, guard;
};
BUILD_ASSERT(sizeof(struct triangle_scene) == 152, "rust/src/lib.rs Scene");

struct triangle_result {
	uint32_t status;  /* YK_RECORD_STATUS_PASS / _FAIL */
	uint32_t failure; /* check/src/lib.rs Failure, 0 for a pass */
	uint32_t picture[3];
};
BUILD_ASSERT(sizeof(struct triangle_result) == 20, "rust/src/lib.rs CheckResult");

typedef void (*triangle_read_pixels_fn)(GLint x, GLint y, GLsizei w, GLsizei h, GLenum format,
					GLenum type, void *pixels);
void triangle_scene(struct triangle_scene *out);
int triangle_check(const uint32_t *cpu, uint32_t w, uint32_t h, uint32_t guard_rows,
		   uint32_t format, triangle_read_pixels_fn read_pixels,
		   struct triangle_result *out);

/* ---- the run -------------------------------------------------------------- */

static struct triangle_scene s_scene;
static struct board_window s_win; /* the window as EGL and the screen see it */
static EGLDisplay s_dpy;
static EGLSurface s_surf;
static struct triangle_result s_result;
static bool s_checked;

/* Each EGL call is checked on its own: eglGetError reports the LAST call. */
static int egl_fail(const char *call)
{
	printk("triangle: %s failed: EGL error 0x%x\n", call, eglGetError());
	return -EIO;
}

static int window(void)
{
	const uint32_t w = s_scene.w, h = s_scene.h, guard_rows = s_scene.guard_rows;
	struct board_window full;
	int ret = board_window(w, h + guard_rows, &full);

	if (ret) {
		printk("triangle: board_window(%u, %u) = %d\n", w, h + guard_rows, ret);
		return ret;
	}
	if (full.stride != w * 4u) {
		printk("triangle: board_window gave stride %u, not %u\n", full.stride, w * 4u);
		return -EINVAL;
	}
	/* The window starts with the sentinel, the guard rows after it with
	 * the guard word, written through the CPU's view (Normal non-cacheable
	 * on every board, yakogl_board.h: no cache maintenance). */
	uint32_t *px = full.cpu;
	size_t win_px = (size_t)w * h, all_px = (size_t)w * (h + guard_rows);

	for (size_t i = 0; i < all_px; i++)
		px[i] = i < win_px ? s_scene.sentinel : s_scene.guard;
	s_win = full;
	s_win.h = h; /* EGL and the screen get the window, not the guard rows */
	printk("triangle: window %ux%u at phys 0x%lx, format 0x%x, %u guard rows\n", s_win.w,
	       s_win.h, (unsigned long)s_win.phys, s_win.format, guard_rows);
	return 0;
}

static int egl_up(void)
{
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nw;
	EGLConfig cfg;
	EGLint n = 0;

	nw = (struct gles_native_window){
		.phys = s_win.phys, .width = s_win.w, .height = s_win.h,
		.stride_bytes = s_win.stride, .format = s_win.format,
	};
	/* every board: the Pi 4 board layer gives the V3D OSAL its clock
	 * switch through the OSAL's default-platform hook */
	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL))
		return egl_fail("eglInitialize");
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n))
		return egl_fail("eglChooseConfig");
	if (n != 1) {
		printk("triangle: eglChooseConfig: %d configs for RGBA8888\n", n);
		return -ENOENT;
	}
	s_surf = eglCreateWindowSurface(s_dpy, cfg, &nw, NULL);
	if (s_surf == EGL_NO_SURFACE)
		return egl_fail("eglCreateWindowSurface");
	EGLContext ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (ctx == EGL_NO_CONTEXT)
		return egl_fail("eglCreateContext");
	if (!eglMakeCurrent(s_dpy, s_surf, s_surf, ctx))
		return egl_fail("eglMakeCurrent");
	printk("triangle: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	return 0;
}

static GLuint shader(GLenum type, const char *src)
{
	GLuint s = glCreateShader(type);
	GLint ok = 0;

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		char log[160];

		glGetShaderInfoLog(s, sizeof(log), NULL, log);
		printk("triangle: compile (%s) failed: %s\n",
		       type == GL_VERTEX_SHADER ? "vs" : "fs", log);
		return 0;
	}
	return s;
}

static int program(GLint *a_pos, GLint *a_color)
{
	int64_t t0 = k_uptime_get();
	GLuint vs = shader(GL_VERTEX_SHADER, s_scene.vs);
	GLuint fs = shader(GL_FRAGMENT_SHADER, s_scene.fs);
	GLint ok = 0;

	if (!vs || !fs)
		return -EINVAL;
	GLuint prog = glCreateProgram();

	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[200];

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		printk("triangle: link failed: %s\n", log);
		return -EINVAL;
	}
	glUseProgram(prog);
	*a_pos = glGetAttribLocation(prog, "a_pos");
	*a_color = glGetAttribLocation(prog, "a_color");
	printk("triangle: compile + link %lld ms; a_pos at %d, a_color at %d\n",
	       (long long)(k_uptime_get() - t0), *a_pos, *a_color);
	if (*a_pos < 0 || *a_color < 0) {
		printk("triangle: an attribute has no location\n");
		return -EINVAL;
	}
	return 0;
}

/* The one frame: clear, the triangle, eglSwapBuffers (submits and waits). */
static int draw(GLint a_pos, GLint a_color)
{
	const float *c = s_scene.clear;
	GLuint vbo;

	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(s_scene.verts), s_scene.verts, GL_STATIC_DRAW);
	glVertexAttribPointer((GLuint)a_pos, 4, GL_FLOAT, GL_FALSE, 32, (const void *)0);
	glVertexAttribPointer((GLuint)a_color, 4, GL_FLOAT, GL_FALSE, 32, (const void *)16);
	glEnableVertexAttribArray((GLuint)a_pos);
	glEnableVertexAttribArray((GLuint)a_color);

	int64_t t0 = k_uptime_get();

	glViewport(0, 0, (GLsizei)s_win.w, (GLsizei)s_win.h);
	glClearColor(c[0], c[1], c[2], c[3]);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_DRAW);
	if (!eglSwapBuffers(s_dpy, s_surf))
		return egl_fail("eglSwapBuffers");
	printk("triangle: frame %lld ms\n", (long long)(k_uptime_get() - t0));
	GLenum err = glGetError();

	if (err != GL_NO_ERROR) {
		printk("triangle: GL error 0x%x during the frame\n", err);
		return -EIO;
	}
	return 0;
}

static int run(void)
{
	GLint a_pos, a_color;
	int ret;

	board_progress(YK_RECORD_WHERE_STAGE | STAGE_WINDOW);
	ret = window();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_EGL);
	ret = egl_up();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_PROGRAM);
	ret = program(&a_pos, &a_color);
	if (ret)
		return ret;
	ret = draw(a_pos, a_color);
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_SWAPPED);
	/* the check reads the window, not the screen: go on after an error */
	ret = board_show(&s_win);
	if (ret)
		printk("triangle: board_show = %d (the window is not on the screen)\n", ret);
	/* the guard rows follow the window in memory: s_win.cpu covers both */
	ret = triangle_check(s_win.cpu, s_win.w, s_win.h, s_scene.guard_rows, s_win.format,
			     glReadPixels, &s_result);
	s_checked = ret >= 0;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_CHECKED);
	if (ret < 0)
		return -EINVAL; /* not checked: the reason is printed */
	return ret == 0 ? 0 : -EBADMSG;
}

int main(void)
{
	int rc;

	triangle_scene(&s_scene);
	printk("triangle: OpenGL ES 2.0 through YakoGL, %ux%u window\n", s_scene.w, s_scene.h);
	rc = run();
	if (s_checked) {
		board_result(YK_RECORD_KIND_PICTURE, s_result.picture[0], s_result.picture[1],
			     s_result.picture[2]);
		if (s_result.status != YK_RECORD_STATUS_PASS)
			board_result(YK_RECORD_KIND_APP,
				     s_result.failure << 8 | YK_RECORD_STATUS_FAIL, 1u,
				     board_gpu_resets());
	}
	board_finish("triangle", rc);
}
