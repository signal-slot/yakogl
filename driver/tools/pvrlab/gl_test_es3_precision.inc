// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- dEQP shaders.precision.{int,uint}: integer ALU into an integer target --
 * Not yet wired: gl_test.c includes this file after its other .inc files and
 * dispatches case_es3_precision_int / case_es3_precision_uint under case
 * numbers the planner assigns (the file defines none).
 *
 * The shape of dEQP-GLES3 shaders.precision.{int,uint}.{highp,mediump,lowp}_
 * {add,sub,mul}_fragment (glsShaderExecUtil): a vertex shader passes two
 * integer attributes on as flat varyings and the fragment shader writes
 *
 *     layout(location = 0) out PREC INT o_out;   o_out = v_in0 OP v_in1;
 *
 * to a scalar integer colour output. The PowerVR online back end refused these
 * programs ("a register read that nothing writes") until 3efd7b30; the cases
 * check that they link, draw and read back the right integer. The target is a
 * 4x4 RGBA32I (case _int) or RGBA32UI (case _uint) renderbuffer, cleared first
 * to a sentinel the draw must replace; one fullscreen triangle (gl_VertexID)
 * covers it, the operands are constant vertex attributes
 * (glVertexAttribI4[u]i), and the red channel is read back with
 * glReadPixels(GL_RGBA_INTEGER) at three pixels (G, B and A are undefined for
 * a one-component output and not compared).
 *
 * Expected R, computed in C (a OP b, the operands within the precision's
 * guaranteed range so the result is exact at any width the hardware uses):
 *
 *   int    prec    a        b      add         sub         mul
 *          highp   1234567  -89    1234478     1234656     -109876463
 *          mediump 3000     -7     2993        3007        -21000
 *          lowp    11       -7     4           18          -77
 *   uint   prec    a        b      add         sub         mul
 *          highp   4000000000 9    4000000009  3999999991  1640261632
 *          mediump 30000    7      30007       29993       210000
 *          lowp    200      5      205         195         1000
 *
 * (highp uint mul is 36e9 mod 2^32. Mediump and lowp operands and results sit past the precision's guaranteed
 * 16 and 9 bit ranges for some rows; hardware computing at 32 bits gives the
 * values above, hardware that really narrows may differ: compare the log, not
 * only the verdict.) Each program also needs its GL error to be clear. */
struct es3prec_row {
	const char *prec, *op;
	int64_t a, b;
};

static const struct es3prec_row es3prec_int_rows[] = {
	{ "highp", "+", 1234567, -89 }, { "highp", "-", 1234567, -89 }, { "highp", "*", 1234567, -89 },
	{ "mediump", "+", 3000, -7 }, { "mediump", "-", 3000, -7 }, { "mediump", "*", 3000, -7 },
	{ "lowp", "+", 11, -7 }, { "lowp", "-", 11, -7 }, { "lowp", "*", 11, -7 },
};

static const struct es3prec_row es3prec_uint_rows[] = {
	{ "highp", "+", 4000000000LL, 9 }, { "highp", "-", 4000000000LL, 9 }, { "highp", "*", 4000000000LL, 9 },
	{ "mediump", "+", 30000, 7 }, { "mediump", "-", 30000, 7 }, { "mediump", "*", 30000, 7 },
	{ "lowp", "+", 200, 5 }, { "lowp", "-", 200, 5 }, { "lowp", "*", 200, 5 },
};

/* Rows are 9 each; `is_uint` picks the type, the target format and the table. */
static int es3prec_run(bool is_uint)
{
	const struct es3prec_row *rows = is_uint ? es3prec_uint_rows : es3prec_int_rows;
	const char *ty = is_uint ? "uint" : "int";
	GLuint rb = 0, fbo = 0;
	bool ok = true;

	free_all_objects();
	glGenRenderbuffers(1, &rb);
	glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glRenderbufferStorage(GL_RENDERBUFFER, is_uint ? GL_RGBA32UI : GL_RGBA32I, 4, 4);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: precision %s: %s target incomplete\n", ty, is_uint ? "RGBA32UI" : "RGBA32I");
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &fbo);
		glDeleteRenderbuffers(1, &rb);
		return -1;
	}
	glViewport(0, 0, 4, 4);

	for (unsigned i = 0; i < 9; i++) {
		static const GLint px_at[3][2] = { { 0, 0 }, { 2, 1 }, { 3, 3 } };
		char vs[512], fs[512], what[48];
		GLuint prog;
		GLuint px[4 * 4 * 4];
		uint32_t want;

		snprintf(vs, sizeof(vs),
			 "#version 300 es\n"
			 "layout(location = 1) in highp %s a_in0;\n"
			 "layout(location = 2) in highp %s a_in1;\n"
			 "flat out %s %s v_in0;\n"
			 "flat out %s %s v_in1;\n"
			 "void main()\n"
			 "{\n"
			 "    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));\n"
			 "    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);\n"
			 "    v_in0 = a_in0;\n"
			 "    v_in1 = a_in1;\n"
			 "}\n",
			 ty, ty, rows[i].prec, ty, rows[i].prec, ty);
		snprintf(fs, sizeof(fs),
			 "#version 300 es\n"
			 "flat in %s %s v_in0;\n"
			 "flat in %s %s v_in1;\n"
			 "layout(location = 0) out %s %s o_out;\n"
			 "void main()\n"
			 "{\n"
			 "    o_out = v_in0 %s v_in1;\n"
			 "}\n",
			 rows[i].prec, ty, rows[i].prec, ty, rows[i].prec, ty, rows[i].op);
		/* The expected word, in C: 32-bit wraparound as the GPU does. */
		switch (rows[i].op[0]) {
		case '+': want = (uint32_t)(rows[i].a + rows[i].b); break;
		case '-': want = (uint32_t)(rows[i].a - rows[i].b); break;
		default: want = (uint32_t)((uint64_t)rows[i].a * (uint64_t)rows[i].b); break;
		}
		snprintf(what, sizeof(what), "%s %s %s", ty, rows[i].prec, rows[i].op);

		prog = link_program(vs, fs);
		if (!prog) {
			pr_err("gl_test: precision %s does not link\n", what);
			ok = false;
			continue;
		}
		if (is_uint) {
			static const GLuint clr[4] = { 0xdeadbeefu, 0, 0, 0 };

			glClearBufferuiv(GL_COLOR, 0, clr);
			glVertexAttribI4ui(1, (GLuint)rows[i].a, 0, 0, 0);
			glVertexAttribI4ui(2, (GLuint)rows[i].b, 0, 0, 0);
		} else {
			static const GLint clr[4] = { 0x5eadbee, 0, 0, 0 };

			glClearBufferiv(GL_COLOR, 0, clr);
			glVertexAttribI4i(1, (GLint)rows[i].a, 0, 0, 0);
			glVertexAttribI4i(2, (GLint)rows[i].b, 0, 0, 0);
		}
		glUseProgram(prog);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glReadPixels(0, 0, 4, 4, GL_RGBA_INTEGER, is_uint ? GL_UNSIGNED_INT : GL_INT, px);
		if (!check_gl_error(what))
			ok = false;
		for (unsigned k = 0; k < 3; k++) {
			GLuint got = px[((unsigned)px_at[k][1] * 4 + (unsigned)px_at[k][0]) * 4];
			bool same = got == want;

			if (is_uint)
				pr_info("gl_test:   %-22s (%d,%d) R = %u  want %u  %s  [check %d]\n", what,
					px_at[k][0], px_at[k][1], (unsigned)got, (unsigned)want,
					same ? "ok" : "MISMATCH", gl_test_check_no + 1);
			else
				pr_info("gl_test:   %-22s (%d,%d) R = %d  want %d  %s  [check %d]\n", what,
					px_at[k][0], px_at[k][1], (int)got, (int)want,
					same ? "ok" : "MISMATCH", gl_test_check_no + 1);
			ok &= gl_test_check(same, got, want);
		}
		glUseProgram(0);
		glDeleteProgram(prog);
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteRenderbuffers(1, &rb);
	glViewport(0, 0, TW, TH);
	return ok && check_gl_error("precision teardown") ? 0 : -1;
}

static int case_es3_precision_int(void)
{
	return es3prec_run(false);
}

static int case_es3_precision_uint(void)
{
	return es3prec_run(true);
}
