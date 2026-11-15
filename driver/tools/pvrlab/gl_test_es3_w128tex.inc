// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- case 244: two-plane RGBA32F/UI/I textures on the GC7000UL (w128tex) ----
 * Included by gl_test.c (after gl_test_es3_divmix.inc). The file uses that
 * file's static helpers (link_program, read_topdown, s_px, gl_test_check,
 * check_gl_error, free_all_objects, puv_quad_setup, TW, TH) and the online
 * shader path like instmix and divmix: its fragment shaders are its own
 * sources, the vertex shader is gl_test_es300_cube_vs_src. No
 * gl_test_shaders.h change.
 *
 * The board check of docs/notes/gc-128bit-swizzle.md section 6 ("Board
 * checks"), for the session that first draws a two-plane texture on the
 * i.MX8MP: the GC7000UL samples RGBA32F/UI/I as two G32R32 planes, a
 * companion sampler slot and the shader's two look-ups joined
 * (docs/notes/gc-128bit-texture-sampling.md). Five sub-checks, each a
 * full-window quad over the 64x64 RGBA8 target sampling a 4x4 texture
 * NEAREST with texture() at uv = v_xy * 0.5 + 0.5, so texel (tx, ty) (rows
 * bottom-up) covers 16x16 pixels:
 *
 *   1. rgba32f   GL_RGBA32F through a sampler2D;
 *   2. rgba32ui  GL_RGBA32UI through a usampler2D;
 *   3. rgba32i   GL_RGBA32I through an isampler2D;
 *   4. rgb32f    GL_RGB32F (uploaded GL_RGB) through a sampler2D: r, g, b as
 *                rgba32f's and a sampled alpha of exactly 1.0;
 *   5. swizzle   the rgba32ui texture with GL_TEXTURE_SWIZZLE_R = GL_BLUE and
 *                GL_TEXTURE_SWIZZLE_B = GL_RED through 2's program: the draw
 *                is refused by name (the interim guard of section 5.1 in
 *                wide128_select, "a 128-bit texture with a texture swizzle is
 *                not joined with it yet") until the swizzle key of 5.2 lands,
 *                so no pixel of the window changes and glGetError is
 *                GL_NO_ERROR. Once the key lands this sub-check must become a
 *                drawn one.
 *
 * Every texel has its own value in each lane, computed by the same formula in
 * C (w128_word) and in the shader, with i = ty * 4 + tx and lane c:
 *
 *   float  bits (0x3f800001 + i * 0x00012345 + c * 0x04000000)
 *               ^ (c odd ? 0x80000000 : 0)
 *          lane 0 ~ 1.0, lane 1 ~ -256, lane 2 ~ 65536 (above the half
 *          float's 65504), lane 3 ~ -2^24; every value has its low mantissa
 *          bits set, which a 16-bit store loses;
 *   uint   0x89000001 + i * 0x00010101 + c * 0x02000000 (all above 2^24);
 *   int    (0x01234567 + i * 0x00010101 + c * 0x00100000), negated in lanes
 *          0 and 2 (magnitudes above 2^24, negative values a 16-bit or
 *          unsigned path loses).
 *
 * The shader compares the four lanes it sampled with the formula's words,
 * bit for bit (floatBitsToUint for the float textures), and writes 1.0 in a
 * lane that matched and 0.0 in one that did not: each of the 16 texels' centre
 * pixels must read 0xffffffff (0xAABBGGRR, the R byte lowest). A byte 00 names
 * the lane that read a wrong value; 0xff000000 everywhere is a draw that did
 * not happen (the clear).
 *
 * Output: one `gl_test:   w128tex <sub>: texel (tx,ty) (x,y) = 0x... want
 * 0xffffffff  MISMATCH  [check N]` line per wrong texel, then one verdict
 * line per sub-check
 *
 *   gl_test: w128tex: rgba32f sampler2D: PASS (16 of 16 texels)
 *   gl_test: w128tex: swizzle refused: PASS (0 of 4096 pixels changed, GL_NO_ERROR)
 *
 * (FAIL with the count otherwise, ", GL error" added when the upload or the
 * draw raised one), and `gl_test: w128tex: N of 5 sub-checks pass`. The case
 * returns 0 only when all five pass.
 *
 * The checks are numbered for the CASE record of a board with no console
 * (first bad check, got, want), the same in every run: 1-16 rgba32f, 17-32
 * rgba32ui, 33-48 rgba32i, 49-64 rgb32f (texel i = ty * 4 + tx is check
 * base + i; got is the pixel, want 0xffffffff), 65 the swizzle draw's
 * glGetError (want 0), 66 the count of pixels it changed (want 0).
 *
 * On the i.MX8MP (tools/cts/imx8mp-gltest-build.sh, case alone: SEL = case
 * minus the image's first case):
 *   tools/cts/imx8mp-gltest-build.sh w128tex-<sha> 244
 *   SEL=0 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
 *     $IMX8MP_IMG_OUT/w128tex-<sha>/zephyr/zephyr.bin <logdir> w128tex
 *
 * Before gc-w128-flip (RGBA32F/UI/I not in GC_SAMPLED_TEXFMTS) the GL core
 * stores RGBA32F and RGB32F in RGBA16F on this back end, so 1 and 4 FAIL in
 * every lane, and refuses RGBA32UI/I, so 2 and 3 FAIL with the clear colour;
 * 5 then passes for the wrong reason (the class is refused, not the swizzle).
 * The case is for the image with the flip. The GC7000 only: SKIP by name on
 * the other GPUs (they sample RGBA32 in one plane and honour the swizzle, so
 * sub-check 5 does not apply) and in an ES 2.0 library. Opt-in, by number. */
#ifdef GL_TEST_VIVANTE
static const char w128tex_f_fs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"precision highp int;\n"
	"uniform highp sampler2D u_tex;\n"
	"uniform int u_rgb;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    vec2 uv = v_xy * 0.5 + 0.5;\n"
	"    ivec2 t = ivec2(floor(uv * 4.0));\n"
	"    uint i = uint(t.y * 4 + t.x);\n"
	"    uvec4 w = (uvec4(0x3f800001u + i * 0x00012345u) + uvec4(0u, 1u, 2u, 3u) * 0x04000000u)\n"
	"              ^ uvec4(0u, 0x80000000u, 0u, 0x80000000u);\n"
	"    if (u_rgb != 0)\n"
	"        w.w = 0x3f800000u;\n"
	"    fragColor = vec4(equal(floatBitsToUint(texture(u_tex, uv)), w));\n"
	"}\n";

static const char w128tex_u_fs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"precision highp int;\n"
	"uniform highp usampler2D u_tex;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    vec2 uv = v_xy * 0.5 + 0.5;\n"
	"    ivec2 t = ivec2(floor(uv * 4.0));\n"
	"    uint i = uint(t.y * 4 + t.x);\n"
	"    uvec4 w = uvec4(0x89000001u + i * 0x00010101u) + uvec4(0u, 1u, 2u, 3u) * 0x02000000u;\n"
	"    fragColor = vec4(equal(texture(u_tex, uv), w));\n"
	"}\n";

static const char w128tex_i_fs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"precision highp int;\n"
	"uniform highp isampler2D u_tex;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    vec2 uv = v_xy * 0.5 + 0.5;\n"
	"    ivec2 t = ivec2(floor(uv * 4.0));\n"
	"    int i = t.y * 4 + t.x;\n"
	"    ivec4 w = (ivec4(0x01234567 + i * 0x00010101) + ivec4(0, 1, 2, 3) * 0x00100000)\n"
	"              * ivec4(-1, 1, -1, 1);\n"
	"    fragColor = vec4(equal(texture(u_tex, uv), w));\n"
	"}\n";

enum w128tex_kind { W128_F, W128_U, W128_I };

/* Lane c of texel i, as the shaders compute it (the float's bits for W128_F). */
static uint32_t w128_word(enum w128tex_kind k, uint32_t i, uint32_t c)
{
	switch (k) {
	case W128_F:
		return (0x3f800001u + i * 0x00012345u + c * 0x04000000u) ^ ((c & 1) ? 0x80000000u : 0u);
	case W128_U:
		return 0x89000001u + i * 0x00010101u + c * 0x02000000u;
	default: {
		int32_t v = (int32_t)(0x01234567u + i * 0x00010101u + c * 0x00100000u);

		return (uint32_t)((c & 1) ? v : -v);
	}
	}
}

/* One sub-check's 16 texel centres in the last draw: each must read
 * 0xffffffff. Always 16 checks, so the numbering stays fixed. Prints the
 * wrong ones and the verdict; true when all match and `gl_ok` (no GL error at
 * the upload or the draw). */
static bool w128_cells(const char *sub, bool gl_ok)
{
	int good = 0;

	read_topdown();
	for (uint32_t ty = 0; ty < 4; ty++) {
		for (uint32_t tx = 0; tx < 4; tx++) {
			uint32_t x = tx * 16 + 8, y = (3 - ty) * 16 + 7;
			uint32_t got = s_px[y * TW + x];

			if (gl_test_check(got == 0xffffffffu, got, 0xffffffffu)) {
				good++;
				continue;
			}
			pr_info("gl_test:   w128tex %s: texel (%u,%u) (%2u,%2u) = 0x%08x  want 0xffffffff  MISMATCH  [check %d]\n",
				sub, tx, ty, x, y, got, gl_test_check_no);
		}
	}
	pr_info("gl_test: w128tex: %s: %s (%d of 16 texels%s)\n", sub, good == 16 && gl_ok ? "PASS" : "FAIL", good,
		gl_ok ? "" : ", GL error");
	return good == 16 && gl_ok;
}

/* A 4x4 texture of `ifmt` from the formula, NEAREST and clamped; `rgb` uploads
 * three lanes (GL_RGB). Bound to unit 0. */
static GLuint w128_texture(GLenum ifmt, enum w128tex_kind k, bool rgb)
{
	static uint32_t texels[16 * 4];
	uint32_t n = rgb ? 3 : 4;
	GLenum fmt = k == W128_F ? (rgb ? GL_RGB : GL_RGBA) : GL_RGBA_INTEGER;
	GLenum type = k == W128_F ? GL_FLOAT : k == W128_U ? GL_UNSIGNED_INT : GL_INT;
	GLuint tex = 0;

	for (uint32_t i = 0; i < 16; i++)
		for (uint32_t c = 0; c < n; c++)
			texels[i * n + c] = w128_word(k, i, c);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, ifmt, 4, 4, 0, fmt, type, texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	return tex;
}

/* Clear to opaque black and draw the quad with `prog` on unit 0. */
static bool w128_draw(GLuint prog, const char *sub)
{
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	return check_gl_error(sub);
}
#endif /* GL_TEST_VIVANTE */

static int case_w128tex(void)
{
#ifndef GL_TEST_VIVANTE
	pr_info("gl_test: case 244 w128tex: SKIP (the two-plane RGBA32 path is the GC7000's)\n");
	return -3;
#else
	static uint32_t before[TW * TH];
	const char *ver = (const char *)glGetString(GL_VERSION);
	GLuint fprog, uprog, iprog, tex;
	int passed = 0;
	bool gl;

	if (!ver || strncmp(ver, "OpenGL ES 2.", 12) == 0) {
		pr_info("gl_test: case 244 w128tex: SKIP (an ES 2.0 library has no 32-bit-channel textures)\n");
		return -3;
	}
	free_all_objects();
	fprog = link_program(gl_test_es300_cube_vs_src, w128tex_f_fs_src);
	uprog = link_program(gl_test_es300_cube_vs_src, w128tex_u_fs_src);
	iprog = link_program(gl_test_es300_cube_vs_src, w128tex_i_fs_src);
	if (!fprog || !uprog || !iprog)
		return -1;
	glViewport(0, 0, TW, TH);
	glActiveTexture(GL_TEXTURE0);

	/* 1. RGBA32F through sampler2D */
	tex = w128_texture(GL_RGBA32F, W128_F, false);
	glUseProgram(fprog);
	glUniform1i(glGetUniformLocation(fprog, "u_rgb"), 0);
	gl = check_gl_error("w128tex rgba32f upload");
	gl &= w128_draw(fprog, "w128tex rgba32f draw");
	passed += w128_cells("rgba32f sampler2D", gl);
	glDeleteTextures(1, &tex);

	/* 2. RGBA32UI through usampler2D */
	tex = w128_texture(GL_RGBA32UI, W128_U, false);
	gl = check_gl_error("w128tex rgba32ui upload");
	gl &= w128_draw(uprog, "w128tex rgba32ui draw");
	passed += w128_cells("rgba32ui usampler2D", gl);
	glDeleteTextures(1, &tex);

	/* 3. RGBA32I through isampler2D */
	tex = w128_texture(GL_RGBA32I, W128_I, false);
	gl = check_gl_error("w128tex rgba32i upload");
	gl &= w128_draw(iprog, "w128tex rgba32i draw");
	passed += w128_cells("rgba32i isampler2D", gl);
	glDeleteTextures(1, &tex);

	/* 4. RGB32F: r, g, b as rgba32f, alpha exactly 1.0 */
	tex = w128_texture(GL_RGB32F, W128_F, true);
	glUseProgram(fprog);
	glUniform1i(glGetUniformLocation(fprog, "u_rgb"), 1);
	gl = check_gl_error("w128tex rgb32f upload");
	gl &= w128_draw(fprog, "w128tex rgb32f draw");
	passed += w128_cells("rgb32f sampler2D, alpha 1.0", gl);
	glDeleteTextures(1, &tex);

	/* 5. 2's texture swizzled across the planes: refused, nothing drawn.
	 * The window as cleared is the reference. */
	tex = w128_texture(GL_RGBA32UI, W128_U, false);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_BLUE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_RED);
	{
		GLenum e;
		uint32_t changed = 0;
		bool ok;

		glUseProgram(uprog);
		glUniform1i(glGetUniformLocation(uprog, "u_tex"), 0);
		puv_quad_setup();
		glClearColor(0.25f, 0.5f, 0.75f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		memcpy(before, read_topdown(), sizeof(before));
		while (glGetError() != GL_NO_ERROR)
			;
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		e = glGetError();
		read_topdown();
		for (uint32_t p = 0; p < TW * TH; p++)
			changed += s_px[p] != before[p];
		ok = gl_test_check(e == GL_NO_ERROR, e, GL_NO_ERROR);
		ok &= gl_test_check(changed == 0, changed, 0);
		if (changed)
			pr_info("gl_test:   w128tex swizzle: (16,16) = 0x%08x, cleared 0x%08x\n", s_px[16 * TW + 16],
				before[16 * TW + 16]);
		pr_info("gl_test: w128tex: swizzle refused: %s (%u of %u pixels changed, %s 0x%x)\n",
			ok ? "PASS" : "FAIL", changed, TW * TH, e == GL_NO_ERROR ? "GL_NO_ERROR" : "GL error", (unsigned)e);
		passed += ok;
	}
	glDeleteTextures(1, &tex);

	pr_info("gl_test: w128tex: %d of 5 sub-checks pass\n", passed);
	glBindTexture(GL_TEXTURE_2D, 0);
	glUseProgram(0);
	glDeleteProgram(fprog);
	glDeleteProgram(uprog);
	glDeleteProgram(iprog);
	return passed == 5 ? 0 : -1;
#endif
}
