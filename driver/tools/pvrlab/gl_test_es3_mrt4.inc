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
 *   part 3: every attachment is cleared again as in part 1, then
 *           glDrawBuffers({ATT0, GL_NONE, ATT2, ATT3}) and the same quad:
 *           attachment 1 keeps its part-1 clear, 0, 2 and 3 get the shader's
 *           colours (the GL_NONE slot must not write render target 1);
 *   part 4: with all four draw buffers on again, GL_SCISSOR_TEST on a
 *           sub-rectangle and glClearBufferfv(GL_COLOR, 3, magenta): inside
 *           the rectangle attachment 3 is magenta, outside it keeps part 3's
 *           yellow, and attachment 0 is untouched (red).
 * The one recorded check is a fail mask: bits 0-3 the clear, bits 4-7 the
 * draw, bit n of each group attachment n; bits 8-11 the GL_NONE draw,
 * attachment n at bit 8+n; bit 12 attachment 3 inside the scissored clear,
 * bit 13 attachment 3 outside it, bit 14 attachment 0 untouched. Lines to read:
 *   gl_test:   mrt4 clear|draw attachment N = 0x... want 0x...
 *   gl_test:   mrt4 none-slot draw attachment N = 0x... want 0x...
 *   gl_test:   mrt4 scissor clear attachment 3 inside|outside = 0x... want 0x...
 *   gl_test:   mrt4 scissor clear attachment 0 = 0x... want 0x...
 *   gl_test:   mrt4 fail mask (...)  = 0  want 0  ok  [check N]
 *   gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS N < 4)
 *   part 5 (es3mrt4_pairs, below, run after part 4 on its own framebuffers):
 *           the PowerVR register budget (docs/notes/pvr-mrt4-budget.md, four
 *           32-bit output registers, a 64-bit format takes two):
 *           (a) RGBA16F at attachment 0 and RGBA8 at 1-2 (2+1+1 = four
 *           registers) must be COMPLETE and the quad's red, green, blue
 *           come back from attachments 0-2 (0 read as GL_FLOAT);
 *           (b) RGBA16F at 0 and 1 and RGBA8 at 2 and 3 (2+2+1+1 = six
 *           registers) must be GL_FRAMEBUFFER_UNSUPPORTED on PowerVR; on
 *           V3D it must be COMPLETE and the quad's four colours drawn.
 *           Another GPU prints a "not checked" line for (b). Below GL_MAX_DRAW_BUFFERS 4
 *           both are skipped by the case's SKIP line, as every part is.
 *           New fail-mask bits (bits 0-14 above are unchanged):
 *             bit 15      (a) framebuffer not COMPLETE
 *             bits 16-18  (a) attachment 0-2 value wrong after the draw
 *             bit 19      (b) status wrong (PowerVR: not UNSUPPORTED; V3D: not COMPLETE)
 *             bits 20-23  (b) V3D only: attachment 0-3 value wrong after the draw
 *           Verdict lines:
 *   gl_test:   mrt4 pair 16F+8+8 status = 0x... want 0x...
 *   gl_test:   mrt4 pair 16F+8+8 attachment N = ... want ...
 *   gl_test:   mrt4 over-budget 16F+16F+8+8 status = 0x... want 0x...  (PowerVR|V3D)
 *   gl_test:   mrt4 over-budget 16F+16F+8+8 attachment N = 0x... want 0x...  (V3D)
 *   gl_test:   mrt4 over-budget 16F+16F+8+8: not checked (renderer "...")
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

static uint32_t es3mrt4_read_at(int n, int x, int y)
{
	uint8_t px[4] = { 0, 0, 0, 0 };

	glReadBuffer(GL_COLOR_ATTACHMENT0 + (GLenum)n);
	glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	return rgba_u32(px[0], px[1], px[2], px[3]);
}

/* Part 5. fmt[n]: 1 = RGBA16F texture, 0 = RGBA8. Returns the status. When the
 * framebuffer is complete and draw is set it draws the quad with all four draw
 * buffers on and leaves the results in outs (floats for RGBA16F, 8-bit
 * otherwise, both as an RGBA8 word). The caller deletes nothing: the helper
 * frees its own objects. */
static GLenum es3mrt4_pair_fb(GLuint prog, const int fmt[4], int draw, uint32_t out[4])
{
	static const GLenum all[4] = {
		GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3,
	};
	GLuint fbo = 0, tex[4] = { 0, 0, 0, 0 };
	GLenum status;

	glGenTextures(4, tex);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	for (int n = 0; n < 4; n++) {
		glBindTexture(GL_TEXTURE_2D, tex[n]);
		if (fmt[n])
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F, TW, TH, 0, GL_RGBA, GL_HALF_FLOAT, NULL);
		else
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glFramebufferTexture2D(GL_FRAMEBUFFER, all[n], GL_TEXTURE_2D, tex[n], 0);
	}
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status == GL_FRAMEBUFFER_COMPLETE && draw) {
		glDrawBuffers(4, all);
		glUseProgram(prog);
		glViewport(0, 0, TW, TH);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		for (int n = 0; n < 4; n++) {
			if (fmt[n]) {
				GLfloat f[4] = { 0.0f, 0.0f, 0.0f, 0.0f };

				glReadBuffer(all[n]);
				glReadPixels(TW / 2, TH / 2, 1, 1, GL_RGBA, GL_FLOAT, f);
				out[n] = rgba_u32((uint8_t)(f[0] * 255.0f + 0.5f), (uint8_t)(f[1] * 255.0f + 0.5f),
						  (uint8_t)(f[2] * 255.0f + 0.5f), (uint8_t)(f[3] * 255.0f + 0.5f));
			} else {
				out[n] = es3mrt4_read(n);
			}
		}
	}
	glReadBuffer(GL_COLOR_ATTACHMENT0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(4, tex);
	return status;
}

static uint32_t es3mrt4_pairs(GLuint prog, const uint32_t want[4])
{
	static const int pair[4] = { 1, 0, 0, 0 };	/* 16F + 8 + 8: 2+1+1 registers */
	static const int over[4] = { 1, 1, 0, 0 };	/* 16F + 16F + 8 + 8: 2+2+1+1 */
	const char *rend = (const char *)glGetString(GL_RENDERER);
	uint32_t out[4] = { 0, 0, 0, 0 };
	uint32_t fail = 0;
	GLenum st;
	int pvr = rend && strstr(rend, "PowerVR") != NULL;
	int v3d = rend && strstr(rend, "V3D") != NULL;

	st = es3mrt4_pair_fb(prog, pair, 1, out);
	pr_info("gl_test:   mrt4 pair 16F+8+8 status = 0x%04x  want 0x%04x\n", (unsigned)st,
		(unsigned)GL_FRAMEBUFFER_COMPLETE);
	if (st != GL_FRAMEBUFFER_COMPLETE) {
		fail |= 1u << 15;
	} else {
		for (int n = 0; n < 3; n++) {
			pr_info("gl_test:   mrt4 pair 16F+8+8 attachment %d = 0x%08x  want 0x%08x\n", n, out[n],
				want[n]);
			if (chan_diff(out[n], want[n]) > 2)
				fail |= 1u << (16 + n);
		}
	}

	st = es3mrt4_pair_fb(prog, over, v3d, out);
	if (pvr) {
		pr_info("gl_test:   mrt4 over-budget 16F+16F+8+8 status = 0x%04x  want 0x%04x (PowerVR)\n",
			(unsigned)st, (unsigned)GL_FRAMEBUFFER_UNSUPPORTED);
		if (st != GL_FRAMEBUFFER_UNSUPPORTED)
			fail |= 1u << 19;
	} else if (v3d) {
		pr_info("gl_test:   mrt4 over-budget 16F+16F+8+8 status = 0x%04x  want 0x%04x (V3D)\n",
			(unsigned)st, (unsigned)GL_FRAMEBUFFER_COMPLETE);
		if (st != GL_FRAMEBUFFER_COMPLETE) {
			fail |= 1u << 19;
		} else {
			for (int n = 0; n < 4; n++) {
				pr_info("gl_test:   mrt4 over-budget 16F+16F+8+8 attachment %d = 0x%08x  want 0x%08x\n",
					n, out[n], want[n]);
				if (chan_diff(out[n], want[n]) > 2)
					fail |= 1u << (20 + n);
			}
		}
	} else {
		pr_info("gl_test:   mrt4 over-budget 16F+16F+8+8: not checked (renderer \"%s\")\n",
			rend ? rend : "?");
	}
	return fail;
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
	static const GLenum gap[4] = {
		GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3,
	};
	static const GLfloat magenta[4] = { 1.0f, 0.0f, 1.0f, 1.0f };
	const uint32_t none_want[4] = {
		rgba_u32(255, 0, 0, 255), rgba_u32(255, 128, 128, 255),
		rgba_u32(0, 0, 255, 255), rgba_u32(255, 255, 0, 255),
	};
	uint32_t got;
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

	/* part 3: a GL_NONE slot between written targets */
	for (int n = 0; n < 4; n++)
		glClearBufferfv(GL_COLOR, n, clr[n]);
	glDrawBuffers(4, gap);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	for (int n = 0; n < 4; n++) {
		got = es3mrt4_read(n);
		pr_info("gl_test:   mrt4 none-slot draw attachment %d = 0x%08x  want 0x%08x\n", n, got,
			none_want[n]);
		if (chan_diff(got, none_want[n]) > 2)
			fail |= 1u << (8 + n);
	}

	/* part 4: scissored clear of attachment 3 */
	glDrawBuffers(4, all);
	glEnable(GL_SCISSOR_TEST);
	glScissor(16, 16, 32, 32);
	glClearBufferfv(GL_COLOR, 3, magenta);
	glDisable(GL_SCISSOR_TEST);
	glFinish();
	got = es3mrt4_read_at(3, TW / 2, TH / 2);
	pr_info("gl_test:   mrt4 scissor clear attachment 3 inside = 0x%08x  want 0x%08x\n", got,
		rgba_u32(255, 0, 255, 255));
	if (chan_diff(got, rgba_u32(255, 0, 255, 255)) > 2)
		fail |= 1u << 12;
	got = es3mrt4_read_at(3, 4, 4);
	pr_info("gl_test:   mrt4 scissor clear attachment 3 outside = 0x%08x  want 0x%08x\n", got,
		draw_want[3]);
	if (chan_diff(got, draw_want[3]) > 2)
		fail |= 1u << 13;
	got = es3mrt4_read_at(0, TW / 2, TH / 2);
	pr_info("gl_test:   mrt4 scissor clear attachment 0 = 0x%08x  want 0x%08x\n", got, draw_want[0]);
	if (chan_diff(got, draw_want[0]) > 2)
		fail |= 1u << 14;

	glReadBuffer(GL_COLOR_ATTACHMENT0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(4, tex);

	/* part 5: the PowerVR register budget */
	fail |= es3mrt4_pairs(prog, draw_want);
	if (!check_gl_error("mrt4 teardown"))
		return -1;
	return expect_int("mrt4 fail mask (0-3 clear, 4-7 draw, 8-11 none slot, 12-14 scissor, 15-23 register budget)", (long)fail, 0) ? 0 : -1;
}
