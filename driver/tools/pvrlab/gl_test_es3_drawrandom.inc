// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- ES 3.0 draw.random shapes (docs/notes/es3-draw-random.md) -------------
 * To be included by gl_test.c after its last .inc (it uses that file's static
 * helpers: link_program, free_all_objects, read_topdown, expect_px,
 * check_gl_error, TW, TH, gl_test_es300_vcolor_fs_src). Not wired yet: no case
 * number is assigned here.
 *
 * dEQP-GLES3.functional.draw.random shapes that failed on the AM62P at
 * ad2b5ed and that 6c9606f6 fixed. Every draw is followed by glGetError.
 * Pixels are top-down in the 64x64 target, 0xAABBGGRR, cleared to 0xff000000.
 *
 * case_drawrandom_packed (draw.random.118): glDrawElementsInstanced(
 * GL_LINE_STRIP, 2, GL_UNSIGNED_SHORT, 0, 2), indices {0, 1} in a buffer, one
 * attribute GL_UNSIGNED_INT_2_10_10_10_REV x4 normalized into a vec2, in an
 * 8-byte buffer (two 4-byte elements), divisor 0. It used to return
 * GL_INVALID_OPERATION (the array was sized as 2 x 4 bytes per element).
 * Elements: (0, 647) and (1023, 647) of 1023, so x = -1 and +1 and the line is
 * y = 647/1023 * 2 - 1 = 0.2649 in clip space; instance 1 is shifted by -0.5.
 * Expected: instance 0 red at (32,23), instance 1 green at (32,39), black at
 * (32,31) and (32,8). A raw read of the words as floats puts the line
 * elsewhere or nowhere.
 *
 * case_drawrandom_intfloat (draw.random 71/94/202 class A): a GL_INT x2 array
 * in a buffer through glVertexAttribPointer (not normalized) into a vec2, a
 * triangle strip (-500,-500) (500,-500) (-500,500) (500,500) drawn with
 * glDrawElements, GL_UNSIGNED_BYTE indices from a buffer; the shader scales
 * by 1/1000. Expected: yellow 0xff00ffff at (32,32), (20,20) and (44,44),
 * black at (4,4) and (60,60). Read as raw bits the values are denormals and
 * the quad collapses to a point: the centre stays black.
 *
 * case_drawrandom_intw (draw.random 36/42/151/207 class B): a GL_INT x3 array
 * through glVertexAttribIPointer into an ivec4, so w reads as the integer 1.
 * Same quad (strip, buffer, glDrawElements); the shader draws green when
 * a_pos.w == 1 and red otherwise. Expected: green 0xff00ff00 at (32,32) and
 * (20,44), black at (4,4). w = 1065353216 (the float 1.0's bits, the old
 * fill) draws red. */
static const char drawrandom_packed_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec2 a_pos;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    vec2 p = a_pos * 2.0 - 1.0;\n"
	"    p.y -= float(gl_InstanceID) * 0.5;\n"
	"    gl_Position = vec4(p, 0.0, 1.0);\n"
	"    v_col = gl_InstanceID == 0 ? vec4(1.0, 0.0, 0.0, 1.0)\n"
	"                               : vec4(0.0, 1.0, 0.0, 1.0);\n"
	"}\n";

static const char drawrandom_intfloat_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec2 a_pos;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    gl_Position = vec4(a_pos / 1000.0, 0.0, 1.0);\n"
	"    v_col = vec4(1.0, 1.0, 0.0, 1.0);\n"
	"}\n";

static const char drawrandom_intw_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"precision highp int;\n"
	"layout(location = 0) in ivec4 a_pos;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    gl_Position = vec4(vec2(a_pos.xy) / 1000.0, 0.0, 1.0);\n"
	"    v_col = a_pos.w == 1 ? vec4(0.0, 1.0, 0.0, 1.0)\n"
	"                         : vec4(1.0, 0.0, 0.0, 1.0);\n"
	"}\n";

static int case_drawrandom_packed(void)
{
	/* x = 0 / 1023, y = 647 / 1023, z = 0, w = 0 (2 bits) */
	static const uint32_t packed[2] = { 647u << 10, 1023u | (647u << 10) };
	static const uint16_t idx[2] = { 0, 1 };
	GLuint prog, vb = 0, ib = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(drawrandom_packed_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(packed), packed, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	glVertexAttribPointer(0, 4, GL_UNSIGNED_INT_2_10_10_10_REV, GL_TRUE, 0, (void *)0);
	glEnableVertexAttribArray(0);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawElementsInstanced(GL_LINE_STRIP, 2, GL_UNSIGNED_SHORT, (void *)0, 2);
	glFinish();
	if (!check_gl_error("drawrandom 118: glDrawElementsInstanced"))
		ok = false;
	read_topdown();
	ok &= expect_px("118: instance 0 line", 32, 23, 0xff0000ffu, 3);
	ok &= expect_px("118: instance 1 line", 32, 39, 0xff00ff00u, 3);
	ok &= expect_px("118: between the lines", 32, 31, 0xff000000u, 3);
	ok &= expect_px("118: above the lines", 32, 8, 0xff000000u, 3);
	glDisableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb);
	glDeleteBuffers(1, &ib);
	return ok && check_gl_error("drawrandom 118 teardown") ? 0 : -1;
}

/* The quad of the two integer cases: `comps` GL_INT components per vertex,
 * `integer` selects glVertexAttribIPointer, drawn as a strip by index. */
static int drawrandom_int_quad(const char *name, const char *vs_src, int comps,
			       bool integer, const uint32_t *wants, const uint32_t (*pts)[2],
			       int npts)
{
	static const GLint v2[4 * 2] = { -500, -500, 500, -500, -500, 500, 500, 500 };
	static const GLint v3[4 * 3] = { -500, -500, 0, 500, -500, 0, -500, 500, 0, 500, 500, 0 };
	static const uint8_t idx[4] = { 0, 1, 2, 3 };
	const void *data = comps == 2 ? (const void *)v2 : (const void *)v3;
	size_t bytes = comps == 2 ? sizeof(v2) : sizeof(v3);
	GLuint prog, vb = 0, ib = 0;
	bool ok = true;
	char what[48];

	free_all_objects();
	prog = link_program(vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, bytes, data, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	if (integer)
		glVertexAttribIPointer(0, comps, GL_INT, 0, (void *)0);
	else
		glVertexAttribPointer(0, comps, GL_INT, GL_FALSE, 0, (void *)0);
	glEnableVertexAttribArray(0);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawElements(GL_TRIANGLE_STRIP, 4, GL_UNSIGNED_BYTE, (void *)0);
	glFinish();
	snprintf(what, sizeof(what), "%s: glDrawElements", name);
	if (!check_gl_error(what))
		ok = false;
	read_topdown();
	for (int i = 0; i < npts; i++) {
		snprintf(what, sizeof(what), "%s: sample %d", name, i);
		ok &= expect_px(what, pts[i][0], pts[i][1], wants[i], 3);
	}
	glDisableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb);
	glDeleteBuffers(1, &ib);
	snprintf(what, sizeof(what), "%s teardown", name);
	return ok && check_gl_error(what) ? 0 : -1;
}

static int case_drawrandom_intfloat(void)
{
	static const uint32_t pts[5][2] = { { 32, 32 }, { 20, 20 }, { 44, 44 }, { 4, 4 }, { 60, 60 } };
	static const uint32_t wants[5] = { 0xff00ffffu, 0xff00ffffu, 0xff00ffffu, 0xff000000u, 0xff000000u };

	return drawrandom_int_quad("int->float", drawrandom_intfloat_vs_src, 2, false,
				   wants, pts, 5);
}

static int case_drawrandom_intw(void)
{
	static const uint32_t pts[3][2] = { { 32, 32 }, { 20, 44 }, { 4, 4 } };
	static const uint32_t wants[3] = { 0xff00ff00u, 0xff00ff00u, 0xff000000u };

	return drawrandom_int_quad("ivec4 w", drawrandom_intw_vs_src, 3, true, wants, pts, 3);
}
