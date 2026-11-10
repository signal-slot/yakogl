// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- the dEQP-GLES3 negative_api and transform feedback lifetime cases ------
 * Included by gl_test.c after the instmix include (it uses that file's static
 * helpers: link_program, compile, check_gl_error, free_all_objects, pr_info,
 * pr_err) and is built wherever gl_test.c is, with no build-file change.
 * Not wired yet: no case number, no name, no dispatch (plan #343, item
 * gltest-negapi-inc; the follow-up item gltest-negapi adds them). Built, not
 * run: the boards are held, the checks go through the board-check issues.
 *
 * Every case asserts the ES 3.0 rule dEQP expects, not what the library does
 * today: a FAIL names the call, the error it got and the error it wanted, and
 * the case goes on to the next call (as dEQP does: it records the first wrong
 * error and carries on). Each case drains the error queue first, checks
 * glGetError after every call, and deletes what it made. SKIP (-3) on an ES
 * 2.0 library, which has none of these entry points.
 *
 * negapi_texsubimage2d   dEQP-GLES3.functional.negative_api.texture.texsubimage2d
 * negapi_texsubimage3d   dEQP-GLES3.functional.negative_api.texture.texsubimage3d
 *   Rule and calls: docs/notes/es3-negative-api.md section 1 (ES 3.0.6 3.8.5,
 *   table 3.2; es3fNegativeTextureApiTests.cpp). INVALID_ENUM for a target,
 *   format or type that is not one; INVALID_OPERATION for a type that does not
 *   pair with the texture's format (the packed 16-bit types with the wrong
 *   base format, RGBA_INTEGER/UNSIGNED_INT, and RGB/GL_FLOAT into an unsized
 *   RGBA texture). A FAIL on the last call alone is the open defect of that
 *   section: es3_sub_image_check accepts GL_FLOAT for an unsized format.
 * negapi_uniformui       dEQP-GLES3.functional.negative_api.shader.uniformui_incompatible_type
 * negapi_uniformuiv      dEQP-GLES3.functional.negative_api.shader.uniformuiv_incompatible_type
 *   Rule and calls: docs/notes/es3-negative-api.md section 2 (ES 3.0.6
 *   2.12.6; es3fNegativeShaderApiTests.cpp). glUniform{1..4}ui[v] on a
 *   uvec4 loads only with 4 (INVALID_OPERATION for 1, 2, 3), and on an ivec4,
 *   a vec4 or (1 value) a sampler2D it is INVALID_OPERATION in every form. A
 *   FAIL at glUniform4ui[v](ivec4) alone is the documented Qt deviation of
 *   that section (program.rs setter_accepts takes an int through the
 *   unsigned setter).
 * negapi_readpix_fmt     dEQP-GLES3.functional.negative_api.buffer.read_pixels_fbo_format_mismatch
 *   Rule and calls: docs/notes/es3-negative-api.md section 3 (ES 3.0.6 4.3.2;
 *   es3fNegativeBufferApiTests.cpp). glReadPixels(RGBA, FLOAT) from an RGBA8,
 *   RGBA32I and RGBA32UI colour attachment is INVALID_OPERATION; a 4-sample
 *   RGBA8 renderbuffer is NO_ERROR, makes a complete framebuffer with
 *   SAMPLE_BUFFERS 1, and glReadPixels(RGBA, UNSIGNED_BYTE) from it is
 *   INVALID_OPERATION. A FAIL at glRenderbufferStorageMultisample is RC8
 *   (plan #339, multisample renderbuffers), not a read rule.
 * tf_deleted_output      dEQP-GLES3.functional.lifetime.attach.deleted_output.buffer_transform_feedback
 *   Rule and sequence: docs/notes/es3-tf-lifetime.md sections 1, 2 and 5
 *   (ES 3.0 D.1.2 and 2.9; es3fLifetimeTests.cpp). A buffer attached to an
 *   unbound transform feedback object is deleted; its name is bound again
 *   WITHOUT glGenBuffers (binding an unused name makes a new object) and
 *   specified with 0xab bytes; the capture through the old transform
 *   feedback object must go to the deleted buffer's storage. The new buffer
 *   must read back all 0xab, glIsBuffer must be true, the TF binding must no
 *   longer name it, and the new buffer must survive the release of the TF
 *   binding. dEQP compares two rendered pictures; this reads the bytes
 *   (host dEQP reads black, so only the bytes decide). A FAIL is the
 *   defect of that note: glBindBuffer reuses the orphan's record. */

static bool na_expect(const char *what, GLenum want)
{
	GLenum got = glGetError();

	if (got != want) {
		pr_err("gl_test: negapi: %s: got GL error 0x%x, want 0x%x\n",
		       what, (unsigned)got, (unsigned)want);
		return false;
	}
	pr_info("gl_test:   %-52s 0x%04x  ok\n", what, (unsigned)want);
	return true;
}

static bool na_is_es2(const char *name)
{
	const char *ver = (const char *)glGetString(GL_VERSION);

	if (!ver || strncmp(ver, "OpenGL ES 2.", 12) == 0) {
		pr_info("gl_test: %s: SKIP (an ES 2.0 library has no ES 3.0 entry points)\n", name);
		return true;
	}
	return false;
}

/* ---- negative_api.texture.texsubimage2d (es3-negative-api.md section 1) --- */
static int case_negapi_texsubimage2d(void)
{
	GLuint tex = 0;
	bool ok = true;

	if (na_is_es2("negapi_texsubimage2d"))
		return -3;
	free_all_objects();
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 32, 32, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("glTexImage2D(RGBA, UNSIGNED_BYTE) 32x32", GL_NO_ERROR);

	glTexSubImage2D(0, 0, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("TexSubImage2D target 0", GL_INVALID_ENUM);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, 4, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("TexSubImage2D format 4", GL_INVALID_ENUM);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, 0, NULL);
	ok &= na_expect("TexSubImage2D RGB, type 0", GL_INVALID_ENUM);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, NULL);
	ok &= na_expect("TexSubImage2D RGBA, UNSIGNED_SHORT_5_6_5", GL_INVALID_OPERATION);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4, NULL);
	ok &= na_expect("TexSubImage2D RGB, UNSIGNED_SHORT_4_4_4_4", GL_INVALID_OPERATION);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, GL_UNSIGNED_SHORT_5_5_5_1, NULL);
	ok &= na_expect("TexSubImage2D RGB, UNSIGNED_SHORT_5_5_5_1", GL_INVALID_OPERATION);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA_INTEGER, GL_UNSIGNED_INT, NULL);
	ok &= na_expect("TexSubImage2D RGBA_INTEGER, UNSIGNED_INT", GL_INVALID_OPERATION);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, GL_FLOAT, NULL);
	ok &= na_expect("TexSubImage2D RGB, FLOAT into unsized RGBA", GL_INVALID_OPERATION);

	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(1, &tex);
	ok &= na_expect("negapi_texsubimage2d teardown", GL_NO_ERROR);
	return ok ? 0 : -1;
}

/* ---- negative_api.texture.texsubimage3d (es3-negative-api.md section 1) --- */
static int case_negapi_texsubimage3d(void)
{
	GLuint tex = 0;
	bool ok = true;

	if (na_is_es2("negapi_texsubimage3d"))
		return -3;
	free_all_objects();
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_3D, tex);
	glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA, 4, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("glTexImage3D(RGBA, UNSIGNED_BYTE) 4x4x4", GL_NO_ERROR);

	glTexSubImage3D(0, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("TexSubImage3D target 0", GL_INVALID_ENUM);
	glTexSubImage3D(GL_TEXTURE_2D, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("TexSubImage3D target TEXTURE_2D", GL_INVALID_ENUM);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, 0, GL_UNSIGNED_BYTE, NULL);
	ok &= na_expect("TexSubImage3D format 0", GL_INVALID_ENUM);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGB, 0, NULL);
	ok &= na_expect("TexSubImage3D RGB, type 0", GL_INVALID_ENUM);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, NULL);
	ok &= na_expect("TexSubImage3D RGBA, UNSIGNED_SHORT_5_6_5", GL_INVALID_OPERATION);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4, NULL);
	ok &= na_expect("TexSubImage3D RGB, UNSIGNED_SHORT_4_4_4_4", GL_INVALID_OPERATION);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGB, GL_UNSIGNED_SHORT_5_5_5_1, NULL);
	ok &= na_expect("TexSubImage3D RGB, UNSIGNED_SHORT_5_5_5_1", GL_INVALID_OPERATION);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGBA_INTEGER, GL_UNSIGNED_INT, NULL);
	ok &= na_expect("TexSubImage3D RGBA_INTEGER, UNSIGNED_INT", GL_INVALID_OPERATION);
	glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGB, GL_FLOAT, NULL);
	ok &= na_expect("TexSubImage3D RGB, FLOAT into unsized RGBA", GL_INVALID_OPERATION);

	glBindTexture(GL_TEXTURE_3D, 0);
	glDeleteTextures(1, &tex);
	ok &= na_expect("negapi_texsubimage3d teardown", GL_NO_ERROR);
	return ok ? 0 : -1;
}

/* ---- negative_api.shader.uniformui{,v}_incompatible_type (section 2) -------
 * One program for both cases: vec4_v, ivec4_f, uvec4_f and sampler_f, all
 * active, in use. */
static const char negapi_uni_vs_src[] =
	"#version 300 es\n"
	"in vec4 a_pos;\n"
	"void main() { gl_Position = a_pos; }\n";
static const char negapi_uni_fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"precision mediump int;\n"
	"precision mediump sampler2D;\n"
	"uniform vec4 vec4_v;\n"
	"uniform ivec4 ivec4_f;\n"
	"uniform uvec4 uvec4_f;\n"
	"uniform sampler2D sampler_f;\n"
	"out vec4 o_color;\n"
	"void main() {\n"
	"    o_color = vec4_v + vec4(ivec4_f) + vec4(uvec4_f) + texture(sampler_f, vec2(0.5));\n"
	"}\n";

/* glUniform<n>ui (vec == false) or glUniform<n>uiv with one element (vec) */
static void negapi_uniform_ui(int n, GLint loc, bool vec)
{
	static const GLuint v[4] = { 0, 0, 0, 0 };

	if (vec) {
		switch (n) {
		case 1: glUniform1uiv(loc, 1, v); break;
		case 2: glUniform2uiv(loc, 1, v); break;
		case 3: glUniform3uiv(loc, 1, v); break;
		default: glUniform4uiv(loc, 1, v); break;
		}
	} else {
		switch (n) {
		case 1: glUniform1ui(loc, 0); break;
		case 2: glUniform2ui(loc, 0, 0); break;
		case 3: glUniform3ui(loc, 0, 0, 0); break;
		default: glUniform4ui(loc, 0, 0, 0, 0); break;
		}
	}
}

static int negapi_uniformui_common(const char *name, bool vec)
{
	static const char *const uname[3] = { "uvec4_f", "ivec4_f", "vec4_v" };
	static const GLuint one[1] = { 0 };
	GLuint prog;
	GLint loc[3], samp;
	bool ok = true;

	if (na_is_es2(name))
		return -3;
	free_all_objects();
	prog = link_program(negapi_uni_vs_src, negapi_uni_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 3; i++) {
		loc[i] = glGetUniformLocation(prog, uname[i]);
		if (loc[i] < 0) {
			pr_err("gl_test: negapi: %s: uniform %s is not active\n", name, uname[i]);
			ok = false;
		}
	}
	samp = glGetUniformLocation(prog, "sampler_f");
	if (samp < 0) {
		pr_err("gl_test: negapi: %s: uniform sampler_f is not active\n", name);
		ok = false;
	}
	if (!ok || !check_gl_error("negapi uniform setup")) {
		glUseProgram(0);
		glDeleteProgram(prog);
		return -1;
	}

	for (int i = 0; i < 3; i++) {
		for (int n = 1; n <= 4; n++) {
			char what[64];
			/* only a uvec4 with 4 values is a match */
			GLenum want = (i == 0 && n == 4) ? GL_NO_ERROR : GL_INVALID_OPERATION;

			negapi_uniform_ui(n, loc[i], vec);
			snprintf(what, sizeof(what), "glUniform%dui%s on %s", n, vec ? "v" : "", uname[i]);
			ok &= na_expect(what, want);
		}
	}
	if (vec)
		glUniform1uiv(samp, 1, one);
	else
		glUniform1ui(samp, 0);
	ok &= na_expect(vec ? "glUniform1uiv on sampler_f" : "glUniform1ui on sampler_f",
			GL_INVALID_OPERATION);

	glUseProgram(0);
	glDeleteProgram(prog);
	ok &= na_expect("negapi uniform teardown", GL_NO_ERROR);
	return ok ? 0 : -1;
}

static int case_negapi_uniformui(void)
{
	return negapi_uniformui_common("negapi_uniformui", false);
}

static int case_negapi_uniformuiv(void)
{
	return negapi_uniformui_common("negapi_uniformuiv", true);
}

/* ---- negative_api.buffer.read_pixels_fbo_format_mismatch (section 3) ------- */
static int case_negapi_readpix_fmt(void)
{
	static const struct { const char *name; GLint ifmt; GLenum fmt, type; } tex_fmt[3] = {
		{ "RGBA8", GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE },
		{ "RGBA32I", GL_RGBA32I, GL_RGBA_INTEGER, GL_INT },
		{ "RGBA32UI", GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT },
	};
	static uint8_t px[32 * 32 * 16];
	GLuint fbo = 0, tex = 0, rb = 0;
	GLint sample_buffers = -1;
	GLenum status;
	bool ok = true;

	if (na_is_es2("negapi_readpix_fmt"))
		return -3;
	free_all_objects();
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	for (int i = 0; i < 3; i++) {
		char what[64];

		glTexImage2D(GL_TEXTURE_2D, 0, tex_fmt[i].ifmt, 32, 32, 0, tex_fmt[i].fmt, tex_fmt[i].type, NULL);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
		snprintf(what, sizeof(what), "%s texture on the FBO", tex_fmt[i].name);
		ok &= na_expect(what, GL_NO_ERROR);
		status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
		if (status != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: negapi: %s FBO status 0x%x, want COMPLETE\n",
			       tex_fmt[i].name, (unsigned)status);
			ok = false;
		}
		glReadPixels(0, 0, 32, 32, GL_RGBA, GL_FLOAT, px);
		snprintf(what, sizeof(what), "ReadPixels RGBA, FLOAT from %s", tex_fmt[i].name);
		ok &= na_expect(what, GL_INVALID_OPERATION);
	}
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(1, &tex);

	glGenRenderbuffers(1, &rb);
	glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 32, 32);
	ok &= na_expect("RenderbufferStorageMultisample(4, RGBA8) 32x32", GL_NO_ERROR);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
	ok &= na_expect("FramebufferRenderbuffer", GL_NO_ERROR);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: negapi: multisample FBO status 0x%x, want COMPLETE\n", (unsigned)status);
		ok = false;
	}
	glGetIntegerv(GL_SAMPLE_BUFFERS, &sample_buffers);
	ok &= na_expect("GetIntegerv(SAMPLE_BUFFERS)", GL_NO_ERROR);
	if (sample_buffers <= 0) {
		pr_err("gl_test: negapi: SAMPLE_BUFFERS %d, want 1 (dEQP: Got invalid value)\n",
		       (int)sample_buffers);
		ok = false;
	} else {
		glReadPixels(0, 0, 32, 32, GL_RGBA, GL_UNSIGNED_BYTE, px);
		ok &= na_expect("ReadPixels RGBA, UNSIGNED_BYTE from a multisample FBO",
				GL_INVALID_OPERATION);
	}

	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
	glBindRenderbuffer(GL_RENDERBUFFER, 0);
	glDeleteRenderbuffers(1, &rb);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	ok &= na_expect("negapi_readpix_fmt teardown", GL_NO_ERROR);
	return ok ? 0 : -1;
}

/* ---- lifetime.attach.deleted_output.buffer_transform_feedback (es3-tf-lifetime.md) */
static const char negapi_tf_vs_src[] =
	"#version 300 es\n"
	"in vec4 a_pos;\n"
	"out vec4 v_tf;\n"
	"void main() { gl_Position = a_pos; v_tf = vec4(1.0, 2.0, 3.0, 4.0); }\n";
static const char negapi_tf_fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"out vec4 o_color;\n"
	"void main() { o_color = vec4(0.0); }\n";

static int case_tf_deleted_output(void)
{
	static const float tri[12] = { -1, -1, 0, 1,  1, -1, 0, 1,  0, 1, 0, 1 };
	static const char *const varyings[1] = { "v_tf" };
	enum { BYTES = 64 };
	uint8_t init[BYTES], fresh[BYTES];
	GLuint vs, fs, prog, vb = 0, vao = 0, tf = 0, x = 0;
	GLint ok_link = 0, bound = -1;
	const uint8_t *map;
	bool ok = true;

	if (na_is_es2("tf_deleted_output"))
		return -3;
	free_all_objects();
	memset(init, 0xcd, sizeof(init));
	memset(fresh, 0xab, sizeof(fresh));
	vs = compile(GL_VERTEX_SHADER, negapi_tf_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, negapi_tf_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "a_pos");
	glTransformFeedbackVaryings(prog, 1, varyings, GL_INTERLEAVED_ATTRIBS);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok_link);
	if (!ok_link) {
		pr_err("gl_test: negapi: tf_deleted_output: link failed\n");
		return -1;
	}
	glUseProgram(prog);
	glEnable(GL_RASTERIZER_DISCARD);
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(tri), tri, GL_STATIC_DRAW);
	glGenVertexArrays(1, &vao);
	glBindVertexArray(vao);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
	glEnableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);

	/* steps 1-2: the element X attached to the unbound TF object */
	glGenTransformFeedbacks(1, &tf);
	glGenBuffers(1, &x);
	glBindBuffer(GL_ARRAY_BUFFER, x);
	glBufferData(GL_ARRAY_BUFFER, BYTES, init, GL_DYNAMIC_READ);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
	glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
	ok &= na_expect("attach X to the TF object", GL_NO_ERROR);

	/* step 5: X deleted while the TF object (not bound) attaches it */
	glDeleteBuffers(1, &x);
	ok &= na_expect("glDeleteBuffers(X)", GL_NO_ERROR);
	if (glIsBuffer(x)) {
		pr_err("gl_test: negapi: glIsBuffer(X) true after the delete, want false\n");
		ok = false;
	}

	/* step 6: the same name bound without glGenBuffers is a new object */
	glBindBuffer(GL_ARRAY_BUFFER, x);
	glBufferData(GL_ARRAY_BUFFER, BYTES, fresh, GL_DYNAMIC_READ);
	ok &= na_expect("glBindBuffer(X) + glBufferData without glGen", GL_NO_ERROR);
	if (!glIsBuffer(x)) {
		pr_err("gl_test: negapi: glIsBuffer(X) false after the rebind, want true\n");
		ok = false;
	}
	glBindBuffer(GL_ARRAY_BUFFER, 0);

	/* step 8: the capture through the TF object must go to the old storage */
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
	glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &bound);
	if ((GLuint)bound == x) {
		pr_err("gl_test: negapi: the TF binding still names the new X (%d)\n", (int)bound);
		ok = false;
	} else {
		pr_info("gl_test:   TF binding 0 now names buffer %d, not the new X (%u)  ok\n",
			(int)bound, (unsigned)x);
	}
	glBeginTransformFeedback(GL_TRIANGLES);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glEndTransformFeedback();
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
	glFinish();
	ok &= na_expect("capture through the TF object", GL_NO_ERROR);

	/* the new X holds its own bytes */
	glBindBuffer(GL_ARRAY_BUFFER, x);
	map = glMapBufferRange(GL_ARRAY_BUFFER, 0, BYTES, GL_MAP_READ_BIT);
	if (!map) {
		pr_err("gl_test: negapi: map of the new X failed\n");
		ok = false;
	} else {
		for (int i = 0; i < BYTES; i++) {
			if (map[i] != 0xab) {
				pr_err("gl_test: negapi: new X byte %d is 0x%02x, want 0xab (the capture wrote to it)\n",
				       i, map[i]);
				ok = false;
				break;
			}
		}
		glUnmapBuffer(GL_ARRAY_BUFFER);
	}

	/* step 9: releasing the TF binding must not free the live X */
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
	glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
	glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
	if (!glIsBuffer(x)) {
		pr_err("gl_test: negapi: the new X was freed by the TF release\n");
		ok = false;
	}
	glBufferSubData(GL_ARRAY_BUFFER, 0, BYTES, init);
	ok &= na_expect("glBufferSubData on the new X after the release", GL_NO_ERROR);

	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &x);
	glDeleteBuffers(1, &vb);
	glDeleteTransformFeedbacks(1, &tf);
	glBindVertexArray(0);
	glDeleteVertexArrays(1, &vao);
	glDisable(GL_RASTERIZER_DISCARD);
	glUseProgram(0);
	glDeleteProgram(prog);
	ok &= na_expect("tf_deleted_output teardown", GL_NO_ERROR);
	return ok ? 0 : -1;
}
