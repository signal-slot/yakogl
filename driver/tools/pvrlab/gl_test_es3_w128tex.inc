// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- case 244: two-plane RGBA32 textures on the GC7000UL (w128tex) ----------
 * Wired into gl_test.c as case 244 (include after gl_test_es3_divmix.inc,
 * `case 244: rc = case_w128tex();` in the dispatch, a row in the header list
 * and in case_names). Built, not run on a board. Uses gl_test.c's static
 * helpers (link_program, read_topdown, check_gl_error, free_all_objects,
 * puv_quad_setup, TW, TH) and the online shader path; no gl_test_shaders.h
 * change.
 *
 * The check of docs/notes/gc-128bit-swizzle.md section 6 ("Board checks"), for
 * the board session that first draws a 16-byte texture on the i.MX8MP: the
 * GC7000UL's texture unit reads RGBA32F/UI/I as two G32R32 planes (a second
 * sampler slot, joined by the compiler: docs/notes/gc-128bit-texture-
 * sampling.md). A 4x4 texture, NEAREST, sampled over the 64x64 target (16 x 16
 * pixels a texel); the fragment shader computes the texel's expected value
 * from the fragment's cell, u_base + u_step * (cx + 4 * cy), compares all four
 * channels exactly and writes vec4(channel equal): 0xffffffff when every
 * channel of the texel is right. The values are ones a 16-bit path loses
 * (above 2^24, beyond half-float range, negative, bit 31 set). Five
 * sub-checks, one verdict line each (all 16 cells must be white):
 *
 *   rgba32f    GL_RGBA32F  through sampler2D,  70000.5 / -1.5 / 2^-8 / 1e6+.25
 *   rgba32ui   GL_RGBA32UI through usampler2D, 0x80000001 / 16777217 / 4e9 / 0xfffffff0
 *   rgba32i    GL_RGBA32I  through isampler2D, -2e9 / 16777217 / -1 / -16777219
 *   rgb32f.a   GL_RGB32F   through sampler2D: rgb as uploaded, alpha exactly 1.0
 *              (the joined value reads plane 1's .y, so the descriptor's A=ONE
 *              never reaches it: the core must store alpha 1)
 *   swizzle    GL_RGBA32UI with TEXTURE_SWIZZLE_R = BLUE and _B = RED: the
 *              draw must be REFUSED while the guard of gc-128bit-swizzle.md
 *              5.1 stands: no pixel changes (the target keeps its clear
 *              colour 16,32,48,255) and glGetError is GL_NO_ERROR. The shader
 *              expects the swapped channels, so a draw that is made and right
 *              (the swizzle key of 5.2 landed, the guard is gone) writes
 *              white and FAILs here by name; then this sub-check becomes a
 *              swizzled-sampling pin and changes with that commit.
 *
 * Output: `gl_test:   <what> got 0x.. want 0x.. ok|MISMATCH [check N]` per
 * sub-check, then `gl_test: w128tex: N of 5 sub-checks ok`; the case returns 0
 * only if all five pass and no GL error was raised. On a back end whose
 * sampled-format mask has no 128-bit class (before the flip) the GL core stores
 * RGBA32F as RGBA16F and the sub-checks 1-4 FAIL: that is the expected record
 * for a pre-flip image. Skipped (-3) on every GPU but the GC7000 and in an ES
 * 2.0 library. Opt-in, by number 244 (the case selection of the board's
 * gl_test image, as every opt-in case). */
#define W128_CELLS 16

static const char w128_vs_src[] =
	"#version 300 es\n"
	"layout(location = 0) in vec2 pos;\n"
	"out vec2 v_xy;\n"
	"void main()\n"
	"{\n"
	"    v_xy = pos;\n"
	"    gl_Position = vec4(pos, 0.0, 1.0);\n"
	"}\n";

#define W128_FS(SAMPLER, VEC, CONV) \
	"#version 300 es\n" \
	"precision highp float;\n" \
	"precision highp int;\n" \
	"uniform highp " SAMPLER " u_tex;\n" \
	"uniform " VEC " u_base;\n" \
	"uniform " VEC " u_step;\n" \
	"in vec2 v_xy;\n" \
	"layout(location = 0) out vec4 fragColor;\n" \
	"void main()\n" \
	"{\n" \
	"    ivec2 c = ivec2(gl_FragCoord.xy) / 16;\n" \
	"    " VEC " want = u_base + u_step * " CONV "(c.x + 4 * c.y);\n" \
	"    " VEC " got = texture(u_tex, v_xy * 0.5 + 0.5);\n" \
	"    fragColor = vec4(equal(got, want));\n" \
	"}\n"

static const char w128_fs_f_src[] = W128_FS("sampler2D", "vec4", "float");
static const char w128_fs_u_src[] = W128_FS("usampler2D", "uvec4", "uint");
static const char w128_fs_i_src[] = W128_FS("isampler2D", "ivec4", "int");

/* One sub-check's verdict line, in the file's format. */
static bool w128_verdict(const char *what, bool ok, uint32_t got, uint32_t want)
{
	pr_info("gl_test:   %-28s got 0x%08x  want 0x%08x  %s  [check %d]\n", what, got, want,
		ok ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(ok, got, want);
}

/* Read the target back and count the 4x4 cells (each cell's centre pixel) that
 * equal `want`; `*first_bad` is the first other pixel (or `want`). */
static unsigned w128_cells_equal(uint32_t want, uint32_t *first_bad)
{
	const uint32_t *px;
	unsigned n = 0;

	glFinish();
	px = read_topdown();
	*first_bad = want;
	for (unsigned cy = 0; cy < 4; cy++) {
		for (unsigned cx = 0; cx < 4; cx++) {
			uint32_t p = px[(cy * 16 + 8) * TW + cx * 16 + 8];

			if (p == want)
				n++;
			else if (*first_bad == want)
				*first_bad = p;
		}
	}
	return n;
}

static void w128_texture(GLenum ifmt, GLenum fmt, GLenum type, const void *data)
{
	glTexImage2D(GL_TEXTURE_2D, 0, ifmt, 4, 4, 0, fmt, type, data);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
}

/* Clear, draw the quad with `prog` and the bound texture; the verdict is that
 * every cell is `want`. */
static bool w128_draw_check(const char *what, GLuint prog, uint32_t want, bool *ok)
{
	uint32_t bad;
	unsigned n;
	bool p;

	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
	puv_quad_setup();
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	if (!check_gl_error(what))
		*ok = false;
	n = w128_cells_equal(want, &bad);
	p = w128_verdict(what, n == W128_CELLS, bad, want);
	*ok &= p;
	return p;
}

static int case_w128tex(void)
{
#ifndef GL_TEST_VIVANTE
	pr_info("gl_test: case 244 w128tex: SKIP (the GC7000UL's two-plane RGBA32 texture class)\n");
	return -3;
#else
	static const float fbase[4] = { 70000.5f, -1.5f, 0.00390625f, 1000000.25f };
	static const float fstep[4] = { 1.0f, -0.25f, 0.001953125f, 0.0625f };
	static const GLuint ubase[4] = { 0x80000001u, 16777217u, 4000000000u, 0xfffffff0u };
	static const GLuint ustep[4] = { 3u, 1u, 7u, 1u };
	static const GLint ibase[4] = { -2000000000, 16777217, -1, -16777219 };
	static const GLint istep[4] = { 5, -3, -1, 2 };
	/* the swizzled draw's expectation: R and B swapped */
	static const GLuint sbase[4] = { 4000000000u, 16777217u, 0x80000001u, 0xfffffff0u };
	static const GLuint sstep[4] = { 7u, 1u, 3u, 1u };
	static const float b3[4] = { 70000.5f, -1.5f, 0.00390625f, 1.0f };
	static const float s3[4] = { 1.0f, -0.25f, 0.001953125f, 0.0f };
	static float f4[W128_CELLS * 4], f3[W128_CELLS * 3];
	static GLuint u4[W128_CELLS * 4];
	static GLint i4[W128_CELLS * 4];
	const char *ver = (const char *)glGetString(GL_VERSION);
	GLuint pf, pu, pi, tex = 0;
	bool ok = true;
	int passed = 0;

	if (!ver || strncmp(ver, "OpenGL ES 2.", 12) == 0) {
		pr_info("gl_test: case 244 w128tex: SKIP (an ES 2.0 library has no RGBA32 textures)\n");
		return -3;
	}
	free_all_objects();
	pf = link_program(w128_vs_src, w128_fs_f_src);
	pu = link_program(w128_vs_src, w128_fs_u_src);
	pi = link_program(w128_vs_src, w128_fs_i_src);
	if (!pf || !pu || !pi)
		return -1;
	for (unsigned t = 0; t < W128_CELLS; t++) {
		for (unsigned k = 0; k < 4; k++) {
			f4[t * 4 + k] = fbase[k] + fstep[k] * (float)t;
			u4[t * 4 + k] = ubase[k] + ustep[k] * t;
			i4[t * 4 + k] = ibase[k] + istep[k] * (GLint)t;
			if (k < 3)
				f3[t * 3 + k] = f4[t * 4 + k];
		}
	}
	glGenTextures(1, &tex);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, tex);
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glClearColor(0.0f, 0.0f, 0.0f, 0.0f);

	/* 1. RGBA32F */
	w128_texture(GL_RGBA32F, GL_RGBA, GL_FLOAT, f4);
	if (!check_gl_error("w128tex rgba32f upload"))
		return -1;
	glUseProgram(pf);
	glUniform4fv(glGetUniformLocation(pf, "u_base"), 1, fbase);
	glUniform4fv(glGetUniformLocation(pf, "u_step"), 1, fstep);
	passed += w128_draw_check("rgba32f sampler2D", pf, 0xffffffffu, &ok);

	/* 2. RGBA32UI */
	w128_texture(GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u4);
	if (!check_gl_error("w128tex rgba32ui upload"))
		return -1;
	glUseProgram(pu);
	glUniform4uiv(glGetUniformLocation(pu, "u_base"), 1, ubase);
	glUniform4uiv(glGetUniformLocation(pu, "u_step"), 1, ustep);
	passed += w128_draw_check("rgba32ui usampler2D", pu, 0xffffffffu, &ok);

	/* 3. RGBA32I */
	w128_texture(GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, i4);
	if (!check_gl_error("w128tex rgba32i upload"))
		return -1;
	glUseProgram(pi);
	glUniform4iv(glGetUniformLocation(pi, "u_base"), 1, ibase);
	glUniform4iv(glGetUniformLocation(pi, "u_step"), 1, istep);
	passed += w128_draw_check("rgba32i isampler2D", pi, 0xffffffffu, &ok);

	/* 4. RGB32F: rgb as uploaded, the sampled alpha exactly 1.0 */
	w128_texture(GL_RGB32F, GL_RGB, GL_FLOAT, f3);
	if (!check_gl_error("w128tex rgb32f upload"))
		return -1;
	glUseProgram(pf);
	glUniform4fv(glGetUniformLocation(pf, "u_base"), 1, b3);
	glUniform4fv(glGetUniformLocation(pf, "u_step"), 1, s3);
	passed += w128_draw_check("rgb32f alpha == 1.0", pf, 0xffffffffu, &ok);

	/* 5. RGBA32UI with R and B swapped by the texture's swizzle: refused,
	 * the target keeps its clear colour (0xAABBGGRR) and GL raises nothing */
	w128_texture(GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u4);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_BLUE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_RED);
	if (!check_gl_error("w128tex swizzle upload"))
		return -1;
	glUseProgram(pu);
	glUniform4uiv(glGetUniformLocation(pu, "u_base"), 1, sbase);
	glUniform4uiv(glGetUniformLocation(pu, "u_step"), 1, sstep);
	glClearColor(16.0f / 255, 32.0f / 255, 48.0f / 255, 1.0f);
	passed += w128_draw_check("swizzled rgba32ui refused", pu, 0xff302010u, &ok);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_RED);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_BLUE);

	pr_info("gl_test: w128tex: %d of 5 sub-checks ok\n", passed);
	glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(pf);
	glDeleteProgram(pu);
	glDeleteProgram(pi);
	return ok && check_gl_error("w128tex teardown") ? 0 : -1;
#endif
}
