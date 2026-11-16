// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- ES 3.0 four colour targets: GL_MAX_DRAW_BUFFERS 4 (case 245 mrt4) ----
 * Case bodies, wired in: gl_test.c includes this file after
 * gl_test_es3_divmix.inc and gives the function case number 245. Everything
 * here is static and uses gl_test.c's helpers (link_program, check_gl_error,
 * upload_vbo, free_all_objects, expect_int, rgba_u32, chan_diff, TW, TH).
 *
 * docs/notes/mrt4-v3d-pvr.md section 4: the board case of the V3D 4.2 and
 * PowerVR BXS-4-64 fourth draw target (plan #368, RC14). The body compiles in
 * every image and is inert without the knob: a GL_MAX_DRAW_BUFFERS or
 * GL_MAX_COLOR_ATTACHMENTS below 4 prints the SKIP line and returns -3
 * (RC_SKIPPED, as case 106 does for 3).
 *
 * Four 64x64 GL_RGBA8 texture attachments, glDrawBuffers(4, ATT0..ATT3).
 *   part 1: glClearBufferfv of each draw buffer with its own colour
 *           (grey, light red, light green, light blue), read back through
 *           glReadBuffer + glReadPixels at the centre;
 *   part 2: one fullscreen quad of a four-output ES 3.00 fragment shader,
 *           location 0..3 = red, green, blue, yellow: attachment 3 yellow is
 *           the board's proof of the fourth target.
 * The one recorded check is a fail mask: bits 0-3 the clear, bits 4-7 the
 * draw, bit n of each group attachment n. Lines to read:
 *   gl_test:   mrt4 clear|draw attachment N = 0x... want 0x...
 *   gl_test:   mrt4 fail mask (...)  = 0  want 0  ok  [check N]
 *   gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS N < 4)
 * The per-case PASS/FAIL line is the harness's. A case returns 0 on a pass,
 * -1 on a mismatch, GL error or incomplete framebuffer. */
static const char es3mrt4_vs_src[] =
	"#version 300 es\n"
	"layout(location = 0) in vec4 a_pos;\n"
	"void main()\n"
	"{\n"
	"    gl_Position = a_pos;\n"
	"}\n";

static const char es3mrt4_fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"layout(location = 0) out vec4 o0;\n"
	"layout(location = 1) out vec4 o1;\n"
	"layout(location = 2) out vec4 o2;\n"
	"layout(location = 3) out vec4 o3;\n"
	"void main()\n"
	"{\n"
	"    o0 = vec4(1.0, 0.0, 0.0, 1.0);\n"
	"    o1 = vec4(0.0, 1.0, 0.0, 1.0);\n"
	"    o2 = vec4(0.0, 0.0, 1.0, 1.0);\n"
	"    o3 = vec4(1.0, 1.0, 0.0, 1.0);\n"
	"}\n";

static uint32_t es3mrt4_read(int n)
{
	uint8_t px[4] = { 0, 0, 0, 0 };

	glReadBuffer(GL_COLOR_ATTACHMENT0 + (GLenum)n);
	glReadPixels(TW / 2, TH / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	return rgba_u32(px[0], px[1], px[2], px[3]);
}

static int case_es3mrt4(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const GLenum all[4] = {
		GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3,
	};
	static const GLfloat clr[4][4] = {
		{ 128.0f / 255.0f, 128.0f / 255.0f, 128.0f / 255.0f, 1.0f },
		{ 1.0f, 128.0f / 255.0f, 128.0f / 255.0f, 1.0f },
		{ 128.0f / 255.0f, 1.0f, 128.0f / 255.0f, 1.0f },
		{ 128.0f / 255.0f, 128.0f / 255.0f, 1.0f, 1.0f },
	};
	const uint32_t clear_want[4] = {
		rgba_u32(128, 128, 128, 255), rgba_u32(255, 128, 128, 255),
		rgba_u32(128, 255, 128, 255), rgba_u32(128, 128, 255, 255),
	};
	const uint32_t draw_want[4] = {
		rgba_u32(255, 0, 0, 255), rgba_u32(0, 255, 0, 255),
		rgba_u32(0, 0, 255, 255), rgba_u32(255, 255, 0, 255),
	};
	GLuint prog, fbo = 0, tex[4] = { 0, 0, 0, 0 };
	GLint maxdb = 0, maxca = 0;
	uint32_t fail = 0;

	glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxdb);
	glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &maxca);
	if (maxdb < 4 || maxca < 4) {
		pr_info("gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS %d < 4)\n",
			(int)(maxdb < 4 ? maxdb : maxca));
		return -3;
	}
	free_all_objects();
	prog = link_program(es3mrt4_vs_src, es3mrt4_fs_src);
	if (!prog)
		return -1;
	glGenTextures(4, tex);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	for (int n = 0; n < 4; n++) {
		glBindTexture(GL_TEXTURE_2D, tex[n]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glFramebufferTexture2D(GL_FRAMEBUFFER, all[n], GL_TEXTURE_2D, tex[n], 0);
	}
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: mrt4: framebuffer incomplete\n");
		return -1;
	}
	glDrawBuffers(4, all);
	if (!check_gl_error("mrt4 setup"))
		return -1;
	glUseProgram(prog);
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);

	for (int n = 0; n < 4; n++)
		glClearBufferfv(GL_COLOR, n, clr[n]);
	glFinish();
	for (int n = 0; n < 4; n++) {
		uint32_t got = es3mrt4_read(n);

		pr_info("gl_test:   mrt4 clear attachment %d = 0x%08x  want 0x%08x\n", n, got, clear_want[n]);
		if (chan_diff(got, clear_want[n]) > 2)
			fail |= 1u << n;
	}

	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	for (int n = 0; n < 4; n++) {
		uint32_t got = es3mrt4_read(n);

		pr_info("gl_test:   mrt4 draw attachment %d = 0x%08x  want 0x%08x\n", n, got, draw_want[n]);
		if (chan_diff(got, draw_want[n]) > 2)
			fail |= 1u << (4 + n);
	}

	glReadBuffer(GL_COLOR_ATTACHMENT0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(4, tex);
	if (!check_gl_error("mrt4 teardown"))
		return -1;
	return expect_int("mrt4 fail mask (0-3 clear, 4-7 draw)", (long)fail, 0) ? 0 : -1;
}
