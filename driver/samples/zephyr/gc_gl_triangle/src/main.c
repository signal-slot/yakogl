/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_gl_triangle - the i.MX8MP sibling of gl_triangle: the same Gouraud
 * triangle, drawn through the public OpenGL ES 2.0 API instead of through the
 * driver's own command builders.
 *
 * gc_triangle proves the driver can drive the 3D pipe. This proves the whole
 * stack above it: EGL picks a config and wraps a buffer, the GL core resolves
 * the state, YakoGL's Vivante backend turns it into a command stream, and the
 * Zephyr OSAL (yakogl/src/osal/zephyr_gc.c) hands that stream to the render
 * service. A failure here with gc_triangle passing is a failure in that stack,
 * not in the hardware bring-up.
 *
 * There is no display driver on this target yet, so the window is a linear
 * XRGB8888 buffer in the GPU carveout and the check is glReadPixels. The
 * vertices are red, green and blue, which separates the failures a
 * single-colour triangle cannot -- see gc_triangle's README for the table.
 *
 * Boot it from Torizon with tools/kexec-zephyr:
 *   kexec_zephyr zephyr.bin 0x42000000
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <errno.h>
#include <string.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <gles_offline.h>

#include <gc_render.h>

#include "gc_sample.h"
#include "gc_result.h"
#include <zephyr/sys/crc.h>
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
#include "imx8mp_display.h"
#endif
#ifdef CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD
#include <yakogl_drivers.h>
#endif

LOG_MODULE_REGISTER(gc_gl_triangle, LOG_LEVEL_INF);

#define W  CONFIG_GC_GL_TRIANGLE_WIDTH
#define H  CONFIG_GC_GL_TRIANGLE_HEIGHT
/* EGL requires the window's pitch to be exactly width * bytes-per-pixel. */
#define STRIDE (W * 4)

/* The clear colour, as the GL sees it and as glReadPixels should report it. */
#define CLEAR_R 0.0f
#define CLEAR_G 0.0f
#define CLEAR_B 0.25f
#define CLEAR_B8 64   /* 0.25 * 255 = 63.75, rounded */

/* x, y, z, w, r, g, b, a -- clip space, w = 1. */
static const float s_verts[3][8] = {
	{  0.0f,  0.8f, 0.0f, 1.0f,  1.0f, 0.0f, 0.0f, 1.0f },   /* red,   top */
	{ -0.8f, -0.8f, 0.0f, 1.0f,  0.0f, 1.0f, 0.0f, 1.0f },   /* green, bottom left */
	{  0.8f, -0.8f, 0.0f, 1.0f,  0.0f, 0.0f, 1.0f, 1.0f },   /* blue,  bottom right */
};

/* Clip space to GL window space: y grows upwards here, which is what
 * glReadPixels takes (the GL core flips for the top-down window surface). */
static GLint gx_of(float x) { return (GLint)((x + 1.0f) * 0.5f * (float)W); }
static GLint gy_of(float y) { return (GLint)((y + 1.0f) * 0.5f * (float)H); }

static void read_rgba(GLint x, GLint y, uint8_t px[4])
{
	px[0] = px[1] = px[2] = px[3] = 0;
	glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
}

/* The PICTURE record (gc_result.h): the centre pixel and a CRC of the centre
 * column, the same numbers the AM62P's gl_triangle computes, so the two boards'
 * pictures can be compared without a camera. */
static void record_picture(void)
{
	uint8_t px[4], col[4 * (GC_RESULT_PICTURE_LAST - GC_RESULT_PICTURE_FIRST + 1)];
	uint32_t centre;

	read_rgba(W / 2, H / 2, px);
	centre = (uint32_t)px[0] | (uint32_t)px[1] << 8 | (uint32_t)px[2] << 16 |
		 (uint32_t)px[3] << 24;
	for (int i = GC_RESULT_PICTURE_FIRST; i <= GC_RESULT_PICTURE_LAST; i++) {
		read_rgba(W / 2, (GLint)(i * H / GC_RESULT_PICTURE_ROWS),
			  &col[4 * (i - GC_RESULT_PICTURE_FIRST)]);
	}
	LOG_INF("  picture: centre 0x%08x, column crc 0x%08x (%ux%u)", centre,
		crc32_ieee(col, sizeof(col)), W, H);
	gc_result_write(GC_RESULT_PICTURE, centre, crc32_ieee(col, sizeof(col)),
			(uint32_t)W << 16 | (uint32_t)H);
}

/* A point 80 per cent of the way from the centroid to vertex `i`: well inside
 * the triangle, and dominated by that vertex's colour. The barycentric weight
 * there is about 0.87, so the matching channel should read near 0xdd and the
 * other two near 0x11. */
static void probe_point(int i, GLint *x, GLint *y)
{
	float cx = (s_verts[0][0] + s_verts[1][0] + s_verts[2][0]) / 3.0f;
	float cy = (s_verts[0][1] + s_verts[1][1] + s_verts[2][1]) / 3.0f;

	*x = gx_of(cx + 0.8f * (s_verts[i][0] - cx));
	*y = gy_of(cy + 0.8f * (s_verts[i][1] - cy));
}

static bool is_clear_colour(const uint8_t px[4])
{
	return px[0] < 0x10 && px[1] < 0x10 &&
	       px[2] >= CLEAR_B8 - 8 && px[2] <= CLEAR_B8 + 8;
}

static bool check_vertex_colours(void)
{
	static const char *const name[3] = { "red (top)", "green (left)", "blue (right)" };
	bool ok = true;

	for (int i = 0; i < 3; i++) {
		uint8_t px[4];
		GLint x, y;

		probe_point(i, &x, &y);
		read_rgba(x, y, px);
		LOG_INF("  near %-13s (%3d,%3d) = %02x%02x%02x", name[i], x, y,
			px[0], px[1], px[2]);

		if (px[i] < 0x80) {
			LOG_ERR("  near %s: that channel is only 0x%02x", name[i], px[i]);
			ok = false;
		}
		for (int j = 0; j < 3; j++) {
			if (j != i && px[j] >= px[i]) {
				LOG_ERR("  near %s: channel %d (0x%02x) is not below "
					"channel %d (0x%02x)", name[i], j, px[j], i, px[i]);
				ok = false;
			}
		}
	}
	return ok;
}

#ifdef CONFIG_YAKOGL_SHADERC_ONLINE
/* The in-process compiler (the default back end) compiles GLSL ES, so the
 * sample hands it the ES 1.00 twins of the two shaders, byte for byte
 * tools/shaders/gl_test_es100/passthrough.vert and interp_color.frag (the
 * table's own texts are #version 450, which only Mesa compiled). A
 * measurement image that links the Mesa reference as its table
 * (CONFIG_YAKOGL_SHADERC_OFFLINE) takes the table's texts by name instead. */
static const char es100_vs_src[] =
	"#version 100\n"
	"/* Passthrough VS: position + one colour varying. ES 1.00 twin of\n"
	" * gl_test_passthrough_vs_src. Declaration order is the twin's location\n"
	" * order (a_pos 0, a_color 1). */\n"
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"varying vec4 v_color;\n"
	"void main() {\n"
	"    gl_Position = a_pos;\n"
	"    v_color = a_color;\n"
	"}\n";

static const char es100_fs_src[] =
	"#version 100\n"
	"/* Interpolated-colour FS: consumes the passthrough VS's v_color varying.\n"
	" * ES 1.00 twin of gl_test_interp_fs_src. */\n"
	"precision mediump float;\n"
	"varying vec4 v_color;\n"
	"void main() { gl_FragColor = v_color; }\n";
#endif

/* The offline shader table only accepts the GLSL it was built from, byte for
 * byte; the library hands the text out by name (passthrough.vert: position and
 * colour through, interp_color.frag: the interpolated colour). */
static GLuint build_program(void)
{
#ifdef CONFIG_YAKOGL_SHADERC_ONLINE
	const char *v = es100_vs_src;
	const char *f = es100_fs_src;
#else
	const char *v = gles_offline_shader_source("passthrough.vert");
	const char *f = gles_offline_shader_source("interp_color.frag");
#endif
	GLuint vs, fs, prog;
	GLint ok = 0;

	if (!v || !f) {
		LOG_ERR("shader sources missing from the Mesa reference table");
		return 0;
	}
	vs = glCreateShader(GL_VERTEX_SHADER);
	fs = glCreateShader(GL_FRAGMENT_SHADER);
	prog = glCreateProgram();

	glShaderSource(vs, 1, &v, NULL);
	glCompileShader(vs);
	glShaderSource(fs, 1, &f, NULL);
	glCompileShader(fs);
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[128];

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		LOG_ERR("link failed: %s", log);
		return 0;
	}
	return prog;
}

static const char *draw(uint64_t window_phys)
{
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_DEPTH_SIZE, 16, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	struct gles_native_window win = {
		.phys = (uintptr_t)window_phys, .width = W, .height = H,
		.stride_bytes = STRIDE, .format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface surf;
	EGLContext ctx;
	EGLint n = 0;
	GLuint prog, vbo;
	GLenum err;
	uint8_t px[4];

	gc_result_stage(GC_RESULT_STAGE_DRAW, 0u);
	if (!eglInitialize(dpy, NULL, NULL)) {
		LOG_ERR("eglInitialize: 0x%x", eglGetError());
		return "EGL did not initialise (the GPU or the OSAL)";
	}
	if (!eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1) {
		LOG_ERR("eglChooseConfig: 0x%x", eglGetError());
		return "no EGL config matches an RGBA8888 window";
	}
	surf = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (surf == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT ||
	    !eglMakeCurrent(dpy, surf, surf, ctx)) {
		gc_result_stage(GC_RESULT_STAGE_EGL_CURRENT, 0u);
		LOG_ERR("EGL surface/context: 0x%x", eglGetError());
		return "the EGL surface or context could not be made current";
	}

	gc_result_stage(GC_RESULT_STAGE_EGL_CURRENT, 1u);

	prog = build_program();
	gc_result_stage(GC_RESULT_STAGE_PROGRAM, prog != 0 ? 1u : 0u);
	if (!prog) {
		return "the shader program did not link";
	}
	glUseProgram(prog);
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(s_verts), s_verts, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 32, (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 32, (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	glViewport(0, 0, W, H);
	glClearColor(CLEAR_R, CLEAR_G, CLEAR_B, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);

	gc_sample_service_wdog();
	/* Submits the frame and waits for its fence, so every read below sees
	 * finished pixels. */
	eglSwapBuffers(dpy, surf);
	gc_sample_service_wdog();

	err = glGetError();
	gc_result_stage(GC_RESULT_STAGE_SWAPPED, err);
	if (err != GL_NO_ERROR) {
		LOG_ERR("GL error 0x%x", err);
		return "the GL reported an error during the frame";
	}
	record_picture();

	/* The bottom-left corner is outside the triangle and must still be the
	 * clear colour: the proof that the draw was bounded, and that the clear
	 * ran at all. */
	read_rgba(1, 1, px);
	LOG_INF("  corner        (  1,  1) = %02x%02x%02x (clear is 0000%02x)",
		px[0], px[1], px[2], CLEAR_B8);
	if (!is_clear_colour(px)) {
		return "the corner is not the clear colour";
	}

	if (!check_vertex_colours()) {
		return "the vertex colours did not interpolate";
	}

	/* The centroid has all three vertices about equally, so every channel
	 * has to be present and none may dominate. A flat-shaded triangle fails
	 * here even when the three vertex probes look plausible. */
	read_rgba(gx_of(0.0f), gy_of(-0.2667f), px);
	LOG_INF("  centroid              = %02x%02x%02x", px[0], px[1], px[2]);
	if (px[0] < 0x20 || px[1] < 0x20 || px[2] < 0x20 ||
	    px[0] > 0xe0 || px[1] > 0xe0 || px[2] > 0xe0) {
		return "the centroid is not a blend of all three vertex colours";
	}

	err = glGetError();
	if (err != GL_NO_ERROR) {
		LOG_ERR("GL error 0x%x after the read-back", err);
		return "glReadPixels reported an error";
	}
	return NULL;
}

/* The window buffer, kept so that main() can hand it to the display. */
static struct gc_bo s_win;

static const char *run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo win;
	int rc;

	if (!device_is_ready(gpu)) {
		gc_result_stage(GC_RESULT_STAGE_GPU_READY, 0u);
		return "the GPU device did not initialise";
	}
	gc_result_stage(GC_RESULT_STAGE_GPU_READY, 1u);
	/* The window has to exist before EGL wraps it, so bring the render
	 * service up here rather than leaving it to eglInitialize (which calls
	 * the same idempotent function through the OSAL). */
	rc = gc_render_init(gpu);
	gc_result_stage(GC_RESULT_STAGE_RENDER_INIT_DONE, (uint32_t)rc);
	if (rc) {
		LOG_ERR("gc_render_init: %d", rc);
		return "the render service did not come up";
	}
	rc = gc_render_alloc((size_t)STRIDE * H, 64, &win);
	gc_result_stage(GC_RESULT_STAGE_ALLOC_DONE, (uint32_t)rc);
	if (rc) {
		LOG_ERR("gc_render_alloc: %d", rc);
		return "no carveout memory for the window buffer";
	}
	LOG_INF("window %ux%u stride %u at GPU 0x%08x", W, H, STRIDE, win.gpu);
	s_win = win;

	/* A CPU-drawn pattern the GPU must overwrite: if the clear never ran,
	 * the probes read this instead of black, which says "nothing drew"
	 * rather than "it drew the wrong thing". The carveout is Normal
	 * non-cacheable, so the GPU sees these stores without a flush. */
	memset(win.cpu, 0x7f, (size_t)STRIDE * H);

	return draw(win.phys);
}

#ifdef CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD
/* The DISPLAY record (record.rs, kind DISPLAY): the chain as it scans the
 * window, after a second of it, so the bridge's IRQ_STAT has had 60 frames
 * to latch a link error. Issue #23: the panel showed 1.5 copies of the frame
 * a line while every record said PASS. */
static void record_display(void)
{
	const struct imx8mp_display_info *info = imx8mp_display_info();
	int irq;
	struct yd_record_display d;
	uint32_t w[3];

	for (int i = 0; i < 10; i++) {
		k_msleep(100);
		gc_sample_service_wdog();
	}
	irq = imx8mp_display_bridge_irq_stat();
	d = (struct yd_record_display){
		.adopted = info->adopted ? 1u : 0u,
		.pixel_clock_hz = info->pixel_clock_hz,
		.lane_rate_hz = info->dsi_hs_clock_hz,
		.irq_now = irq >= 0 ? (uint32_t)irq : YK_RECORD_DISPLAY_IRQ_NOT_READ,
		.irq_at_start = info->bridge_irq_stat,
		.health = imx8mp_display_health(),
		.bpp_code = (imx8mp_display_layer_control() >> 24) & 0xfu,
		.pitch = info->pitch,
	};
	yd_record_display_words(&d, w);
	LOG_INF("display: %s, pixel clock %u Hz, DSI %u Hz, IRQ_STAT 0x%02x (start 0x%02x), "
		"health 0x%02x, CTRLDESCL0_5 0x%08x, pitch %u",
		info->adopted ? "adopted" : "cold", info->pixel_clock_hz, info->dsi_hs_clock_hz,
		d.irq_now, d.irq_at_start, d.health, imx8mp_display_layer_control(), d.pitch);
	gc_result_write(GC_RESULT_DISPLAY, w[0], w[1], w[2]);
}
#endif

/* Microseconds on the SoC's system counter. On the direct-boot route
 * (tools/cts/log/x2-direct-boot.md) U-Boot SPL starts that counter early in its own
 * start-up and nothing restarts it, so this is close to "time since power-on"
 * minus the Boot ROM; on the kexec route it is Linux's uptime and means
 * little. CNTFRQ_EL0 is the 8 MHz SPL programs. */
static uint64_t syscnt_us(void)
{
	uint64_t cnt, frq;

	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(cnt));
	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(frq));
	return frq != 0u ? cnt / (frq / 1000000u) : 0u;
}

int main(void)
{
	const char *reason;

	gc_result_stage(GC_RESULT_STAGE_MAIN, 0u);
	LOG_INF("gc_gl_triangle: %ux%u window, OpenGL ES 2.0 through YakoGL", W, H);
	LOG_INF("system counter at main(): %llu us", (unsigned long long)syscnt_us());
	gc_sample_service_wdog();
	reason = run();
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
	/* Show the window on the panel, whatever the read-back said: a
	 * picture is worth more than a verdict on a board with no console.
	 * The window is panel-sized (prj.conf), so the LCDIF scans it as is. */
	if (s_win.phys != 0 && W == 1280 && H == 800) {
		int drc = imx8mp_display_on((uintptr_t)s_win.phys, STRIDE);

		gc_result_stage(GC_RESULT_STAGE_DISPLAY_ON, (uint32_t)drc);
		LOG_INF("display: imx8mp_display_on(window) = %d", drc);
#ifdef CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD
		if (drc == 0) {
			record_display();
		}
#endif
	}
#endif
	LOG_INF("system counter at the verdict: %llu us", (unsigned long long)syscnt_us());
	/* Dropped when the PICTURE record was stored (gc_result.h): this is
	 * the record only of a run that ended before its picture. */
	gc_result_stage(GC_RESULT_STAGE_ENDED, reason == NULL ? 1u : 0u);
	gc_sample_verdict("gc_gl_triangle", reason == NULL, reason);
	if (reason != NULL && gc_render_ready()) {
		gc_render_stall_dump("gc_gl_triangle failed");
	}
	gc_sample_finish();
	return 0;
}
