/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A stand-in for a sample's Rust archive, for selftest.sh: the four entry
 * points the two replay programs call, each drawing one triangle through the
 * get_proc it is given. FAKE_BAD=1 adds what the replay must report: a GL
 * error (glEnable with a bad enum) and a name YakoGL does not answer.
 */
#include <stdint.h>
#include <stdlib.h>

#include <GLES2/gl2.h>

typedef const void *(*get_proc_fn)(const char *);

static const char *vs = "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }";
static const char *fs = "precision mediump float; void main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }";

#define GL(name) ((__typeof__(&name))gp(#name))

static int frame(get_proc_fn gp)
{
	static GLuint prog, vbo;
	static const float tri[] = { -1, -1, 1, -1, 0, 1 };

	if (!prog) {
		GLuint v = GL(glCreateShader)(GL_VERTEX_SHADER), f = GL(glCreateShader)(GL_FRAGMENT_SHADER);
		GLint ok = 0;

		GL(glShaderSource)(v, 1, &vs, NULL);
		GL(glShaderSource)(f, 1, &fs, NULL);
		GL(glCompileShader)(v);
		GL(glCompileShader)(f);
		prog = GL(glCreateProgram)();
		GL(glAttachShader)(prog, v);
		GL(glAttachShader)(prog, f);
		GL(glLinkProgram)(prog);
		GL(glGetProgramiv)(prog, GL_LINK_STATUS, &ok);
		if (!ok)
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
		GL(glEnable)(0xdead);
		(void)gp("glNoSuchEntryPoint");
	}
	return 0;
}

int slint_cluster_draw_frame(get_proc_fn gp, uint32_t w, uint32_t h)
{
	(void)w;
	(void)h;
	return frame(gp);
}

int slint_cluster_advance(uint32_t n)
{
	(void)n;
	return 0;
}

int slint_sample_first_frame(get_proc_fn gp, uint32_t w, uint32_t h)
{
	return slint_cluster_draw_frame(gp, w, h);
}

int slint_sample_animate_frame(get_proc_fn gp, uint32_t w, uint32_t h, uint32_t t_ms, uint32_t n)
{
	(void)t_ms;
	(void)n;
	return slint_cluster_draw_frame(gp, w, h);
}
