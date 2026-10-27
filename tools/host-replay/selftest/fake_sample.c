/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A stand-in for a sample's Rust archive, for selftest.sh: the four entry
 * points the two replay programs call, each drawing one triangle through the
 * get_proc it is given. FAKE_BAD=1 adds what the replay must report: a GL
 * error (glEnable with a bad enum), a name YakoGL does not answer, and a
 * draw that samples an incomplete texture (one level under a mipmap filter:
 * it reads (0,0,0,1) with no GL error, ES 2.0 3.7.10, so only the counter
 * sees it). Not a texture without any image: the draw path skips a texture
 * that has no storage before its completeness check (src/gl/draw.rs,
 * "no texture: the backend binds a black 1x1"), so that one is not counted.
 */
#include <stdint.h>
#include <stdlib.h>

#include <GLES2/gl2.h>

typedef const void *(*get_proc_fn)(const char *);

static const char *vs = "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }";
static const char *fs = "precision mediump float; void main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }";
static const char *fs_tex = "precision mediump float; uniform sampler2D t; "
			    "void main() { gl_FragColor = texture2D(t, vec2(0.5, 0.5)); }";

#define GL(name) ((__typeof__(&name))gp(#name))

/* A linked program of the two texts, or 0. */
static GLuint build(get_proc_fn gp, const char *vs_text, const char *fs_text)
{
	GLuint v = GL(glCreateShader)(GL_VERTEX_SHADER), f = GL(glCreateShader)(GL_FRAGMENT_SHADER), prog;
	GLint ok = 0;

	GL(glShaderSource)(v, 1, &vs_text, NULL);
	GL(glShaderSource)(f, 1, &fs_text, NULL);
	GL(glCompileShader)(v);
	GL(glCompileShader)(f);
	prog = GL(glCreateProgram)();
	GL(glAttachShader)(prog, v);
	GL(glAttachShader)(prog, f);
	GL(glLinkProgram)(prog);
	GL(glGetProgramiv)(prog, GL_LINK_STATUS, &ok);
	return ok ? prog : 0;
}

static int frame(get_proc_fn gp)
{
	static GLuint prog, vbo;
	static const float tri[] = { -1, -1, 1, -1, 0, 1 };

	if (!prog) {
		prog = build(gp, vs, fs);
		if (!prog)
			return -1;
		GL(glGenBuffers)(1, &vbo);
		GL(glBindBuffer)(GL_ARRAY_BUFFER, vbo);
		GL(glBufferData)(GL_ARRAY_BUFFER, sizeof(tri), tri, GL_STATIC_DRAW);
	}
	GL(glUseProgram)(prog);
	GL(glBindBuffer)(GL_ARRAY_BUFFER, vbo);
	GL(glEnableVertexAttribArray)(0);
	GL(glVertexAttribPointer)(0, 2, GL_FLOAT, GL_FALSE, 0, NULL);
	GL(glClear)(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	GL(glDrawArrays)(GL_TRIANGLES, 0, 3);
	const char *bad = getenv("FAKE_BAD");

	if (bad && *bad) {
		static GLuint tex_prog, tex;

		if (!tex_prog) {
			static const unsigned char texels[2 * 2 * 4] = { 0 };

			tex_prog = build(gp, vs, fs_tex);
			if (!tex_prog)
				return -1;
			GL(glGenTextures)(1, &tex);
			GL(glBindTexture)(GL_TEXTURE_2D, tex);
			GL(glTexImage2D)(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
			/* level 0 only, under a filter that needs level 1 too */
			GL(glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
		}
		/* incomplete (ES 2.0 3.7.10): sampled as (0,0,0,1), no GL error */
		GL(glUseProgram)(tex_prog);
		GL(glActiveTexture)(GL_TEXTURE0);
		GL(glBindTexture)(GL_TEXTURE_2D, tex);
		GL(glUniform1i)(GL(glGetUniformLocation)(tex_prog, "t"), 0);
		GL(glDrawArrays)(GL_TRIANGLES, 0, 3);
		GL(glEnable)(0xdead);
		(void)gp("glNoSuchEntryPoint");
	}
	return 0;
}

int slint_sample_first_frame(get_proc_fn gp, uint32_t w, uint32_t h)
{
	(void)w;
	(void)h;
	return frame(gp);
}

int slint_sample_animate_frame(get_proc_fn gp, uint32_t w, uint32_t h, uint32_t t_ms, uint32_t n)
{
	(void)w;
	(void)h;
	(void)t_ms;
	(void)n;
	return frame(gp);
}
