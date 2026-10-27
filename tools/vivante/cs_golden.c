/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * cs_golden.c - the command stream a working driver produces, without a GPU.
 *
 * Mesa's etnaviv driver runs on the host against its own drm-shim, which
 * answers as the i.MX8MP's GC7000UL and keeps every submit instead of
 * executing it. What this program draws is deliberately the shape of
 * gl_test's case 1: a vertex shader with no uniforms, a fragment shader whose
 * only input is one vec4 uniform, and a triangle. The stream it leaves behind
 * is what this backend's own stream is diffed against, so a disagreement
 * about what the hardware is told stops being a guess.
 *
 * The case number chooses the shape. **The numbering itself is defined once,
 * in cs_ours.c's "the mode numbers, in one place" list** -- both harnesses
 * take the same number and must agree, and mode 15 was once claimed by two
 * people in commits that merged without conflict. Add the row there first.
 * What follows is what each mode is worth reading *here*, in the golden.
 *   1  fragment uniform only  (the default; gl_test case 1)
 *   2  vertex uniform as well (both stages, so the split between them shows)
 *   3  two frames, the uniform changed between them, which is gl_test case 1
 *      in full -- the second frame's draw does not reload the shader, so it
 *      shows what a uniform write looks like with no shader block before it
 *   4  a textured quad, NEAREST and clamp-to-edge: gl_test case 2's shape,
 *      so the texture descriptor and the sampler state can be compared
 *   8  gl_test case 3's shape: a depth-tested draw, so the depth buffer's
 *      stride and layout can be compared -- the one open question in the
 *      notes that no other mode reaches
 *  12  case 13's first frame: the same depth-tested draw, but the depth
 *      clear is glClearDepthf(0.5) rather than the default 1.0. Case 13's
 *      first check is exactly that value being honoured, so this mode shows
 *      where the two drivers put it.
 *  14  case 3 in full: mode 8's first frame, then the depth test turned
 *      off, then GL_GREATER with glClearDepthf(0), then the two polygon
 *      offset frames. Case 3 hangs kicks on the board, and what a driver
 *      emits *between* two frames of changing depth state is the part no
 *      single-frame mode can show.
 *  15  a depth clear with no draw behind it, which GL has to carry out at
 *      the glFinish. Whether a kick comes out at all is the question.
 *   7  gl_test case 10's points: three GL_POINTS with gl_PointSize coming
 *      from a vertex attribute, so SH_SPECIALS' VS_PSIZE_OUT can be compared
 *  19  gl_test case 8 frame 1: a blended draw over an opaque one
 *  20  gl_test case 8 frames 5 and 6: glBlendColor's factors, then a colour
 *      mask that writes only green and alpha
 *  21  gl_test case 12: two-sided stencil, front reference 1 and back 2,
 *      drawn twice -- CCW fronts then CW -- because this pixel engine has no
 *      front-face register, so what moves between the two frames is how a
 *      driver names the faces
 *  22  gl_test case 23: INCR_WRAP / DECR_WRAP winding, then a NOTEQUAL cover
 *  23  gl_test case 34: two draws in one frame with different uniforms
 *  24  gl_test case 24: two samplers declared, only the second sampled -- a
 *      1x1 texture descriptor to compare, which no other mode has
 *  26  dEQP texture.vertex.2d.filtering.nearest_nearest_clamp's shape:
 *      texture2DLod in the vertex stage over two 128x128 RGB mipmapped
 *      textures, a minified and a magnified draw in one frame
 *   5  case 2's mip sub-test: a 4x4 texture whose levels are red, green and
 *      blue, NEAREST_MIPMAP_NEAREST, on a quad two pixels wide so the
 *      hardware has to pick level 1. That is the check the board fails.
 *  11  a cube map: six 8x8 faces, one colour each, sampled with textureCube.
 *      The descriptor's TYPE, SLICE and 3D_CONFIG are what to read here --
 *      the backend fills every texture in as a plain 2D one, so cases 41, 42
 *      and 48 have nothing to go on until this is compared.
 *  10  gl_test case 20's ETC1 texture: 16x8, four block columns by two block
 *      rows, each block a flat colour of its own. Where the blocks lie in
 *      memory and what the descriptor says about them is the whole question,
 *      so read this one with --tex-desc and --mem.
 *  16  case 20's ETC2 RGBA8 EAC texture. The branch is here, but ES 2.0 has
 *      no ETC2 and this context rejects the upload with GL_INVALID_ENUM, so
 *      there is no golden to be had: it stops with exit 3 and says why,
 *      rather than describing the default texture the binding kept.
 */
#include <EGL/egl.h>
#include <GLES2/gl2.h>

/* OES_packed_depth_stencil; not in the ES 2.0 headers. */
#ifndef GL_DEPTH24_STENCIL8_OES
#define GL_DEPTH24_STENCIL8_OES 0x88F0
#endif
/* OES_compressed_ETC1_RGB8_texture; not in the ES 2.0 headers either. */
#ifndef GL_ETC1_RGB8_OES
#define GL_ETC1_RGB8_OES 0x8D64
#endif
#ifndef GL_RGBA8_OES
#define GL_RGBA8_OES 0x8058
#endif
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char vs_plain[] =
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"varying vec4 v_color;\n"
	"void main() { gl_Position = a_pos; v_color = a_color; }\n";

static const char vs_mvp[] =
	"uniform mat4 u_mvp;\n"
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"varying vec4 v_color;\n"
	"void main() { gl_Position = u_mvp * a_pos; v_color = a_color; }\n";

static const char vs_uv[] =
	"attribute vec4 a_pos;\n"
	"attribute vec2 a_uv;\n"
	"varying vec2 v_uv;\n"
	"void main() { gl_Position = a_pos; v_uv = a_uv; }\n";

static const char vs_dir[] =
	"attribute vec4 a_pos;\n"
	"attribute vec2 a_uv;\n"
	"varying vec3 v_dir;\n"
	"void main() { gl_Position = a_pos; v_dir = vec3(a_uv, 1.0); }\n";

static const char fs_cube[] =
	"precision mediump float;\n"
	"uniform samplerCube u_cube;\n"
	"varying vec3 v_dir;\n"
	"void main() { gl_FragColor = textureCube(u_cube, v_dir); }\n";

static const char fs_tex[] =
	"precision mediump float;\n"
	"uniform sampler2D u_tex;\n"
	"varying vec2 v_uv;\n"
	"void main() { gl_FragColor = texture2D(u_tex, v_uv); }\n";

static const char vs_points[] =
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"attribute float a_size;\n"
	"varying vec4 v_color;\n"
	"void main() { gl_Position = a_pos; gl_PointSize = a_size; v_color = a_color; }\n";

static const char fs_varying[] =
	"precision mediump float;\n"
	"varying vec4 v_color;\n"
	"void main() { gl_FragColor = v_color; }\n";

static const char fs_uniform[] =
	"precision mediump float;\n"
	"uniform vec4 u_color;\n"
	"void main() { gl_FragColor = u_color; }\n";

/* gl_test's two-varying pair: colour modulated by uv (colour_uv.frag). */
static const char vs_cuv[] =
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"attribute vec2 a_uv;\n"
	"varying vec4 v_color;\n"
	"varying vec2 v_uv;\n"
	"void main() { gl_Position = a_pos; v_color = a_color; v_uv = a_uv; }\n";

static const char fs_cuv[] =
	"precision mediump float;\n"
	"varying vec4 v_color;\n"
	"varying vec2 v_uv;\n"
	"void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }\n";

/* case 24's fragment shader: two samplers declared, one sampled. It pairs with
 * vs_uv above. */
static const char fs_compact[] =
	"precision mediump float;\n"
	"varying vec2 v_uv;\n"
	"uniform sampler2D u_tex0;\n"
	"uniform sampler2D u_tex1;\n"
	"void main() { gl_FragColor = texture2D(u_tex1, v_uv); }\n";

static GLuint compile(GLenum stage, const char *src)
{
	GLuint s = glCreateShader(stage);
	GLint ok = 0;

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		char log[1024] = { 0 };

		glGetShaderInfoLog(s, sizeof(log) - 1, NULL, log);
		fprintf(stderr, "compile failed: %s\n", log);
		exit(1);
	}
	return s;
}

/*
 * Modes 10..14: the same per-pixel state the matching cs_ours.c modes set, so
 * the blend, stencil and uniform register blocks can be diffed. Each mode
 * builds its own program and draws its own frame.
 */
struct cuv_g { float pos[4]; float color[4]; float uv[2]; };

static void cuv_set_g(struct cuv_g *v, float x, float y, float z,
		      float r, float g, float b)
{
	v->pos[0] = x; v->pos[1] = y; v->pos[2] = z; v->pos[3] = 1.0f;
	v->color[0] = r; v->color[1] = g; v->color[2] = b; v->color[3] = 1.0f;
	v->uv[0] = 1.0f; v->uv[1] = 1.0f;
}

static void cuv_attribs_g(void)
{
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_g), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_g), (const void *)16);
	glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_g), (const void *)32);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
}

static GLuint link_two_g(const char *vs, const char *fs, const char *a1, const char *a2)
{
	GLuint p = glCreateProgram();
	GLint ok = 0;

	glAttachShader(p, compile(GL_VERTEX_SHADER, vs));
	glAttachShader(p, compile(GL_FRAGMENT_SHADER, fs));
	glBindAttribLocation(p, 0, "a_pos");
	if (a1) {
		glBindAttribLocation(p, 1, a1);
	}
	if (a2) {
		glBindAttribLocation(p, 2, a2);
	}
	glLinkProgram(p);
	glGetProgramiv(p, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[1024] = { 0 };

		glGetProgramInfoLog(p, sizeof(log) - 1, NULL, log);
		fprintf(stderr, "link failed: %s\n", log);
		exit(1);
	}
	glUseProgram(p);
	return p;
}

static void blend_geometry_g(void)
{
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};
	struct cuv_g v[9];
	GLuint vbo;
	int i;

	for (i = 0; i < 6; i++) {
		cuv_set_g(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
	}
	cuv_set_g(&v[6], -0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set_g(&v[7], 0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set_g(&v[8], 0.0f, 0.75f, 0.0f, 0, 1, 0);
	for (i = 6; i < 9; i++) {
		v[i].color[3] = 0.5f;
	}
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
	cuv_attribs_g();
}

static int frag_mode_g(int mode)
{
	glViewport(0, 0, 64, 64);
	glClearColor(0, 0, 0, 1);

	if (mode == 19 || mode == 20) {
		link_two_g(vs_cuv, fs_cuv, "a_color", "a_uv");
		blend_geometry_g();
		glClear(GL_COLOR_BUFFER_BIT);
		glDisable(GL_BLEND);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glEnable(GL_BLEND);
		if (mode == 19) {
			glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
		} else {
			glBlendColor(0.25f, 0.5f, 0.75f, 1.0f);
			glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR);
		}
		glDrawArrays(GL_TRIANGLES, 6, 3);
		if (mode == 20) {
			glDisable(GL_BLEND);
			glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_TRUE);
			glDrawArrays(GL_TRIANGLES, 6, 3);
			glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		}
		glFinish();
		glDisable(GL_BLEND);
	} else if (mode == 21) {
		static const float c[6][2] = {
			{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
		};
		struct cuv_g v[18];
		GLuint vbo;
		int i;

		link_two_g(vs_cuv, fs_cuv, "a_color", "a_uv");
		for (i = 0; i < 6; i++) {
			cuv_set_g(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
			cuv_set_g(&v[12 + i], c[i][0], c[i][1], 0.0f, 0, 0, 1);
		}
		cuv_set_g(&v[6], -0.9f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_g(&v[7], -0.1f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_g(&v[8], -0.5f, 0.8f, 0.0f, 1, 1, 1);
		cuv_set_g(&v[9], 0.1f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_g(&v[10], 0.5f, 0.8f, 0.0f, 1, 1, 1);
		cuv_set_g(&v[11], 0.9f, -0.8f, 0.0f, 1, 1, 1);
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
		cuv_attribs_g();

		glClearStencil(0);
		glEnable(GL_STENCIL_TEST);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
		glStencilFuncSeparate(GL_FRONT, GL_ALWAYS, 1, 0xff);
		glStencilFuncSeparate(GL_BACK, GL_ALWAYS, 2, 0xff);
		glStencilOpSeparate(GL_FRONT_AND_BACK, GL_KEEP, GL_KEEP, GL_REPLACE);
		glDrawArrays(GL_TRIANGLES, 6, 6);
		glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		glStencilFunc(GL_EQUAL, 1, 0xff);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glStencilFunc(GL_EQUAL, 2, 0xff);
		glDrawArrays(GL_TRIANGLES, 12, 6);
		glFinish();

		/* frame 2: the same with clockwise fronts. The hardware has
		 * no front-face register, so whatever moves between the two
		 * frames is how a driver tells it which winding is front. */
		glFrontFace(GL_CW);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
		glStencilFuncSeparate(GL_FRONT, GL_ALWAYS, 1, 0xff);
		glStencilFuncSeparate(GL_BACK, GL_ALWAYS, 2, 0xff);
		glStencilOpSeparate(GL_FRONT_AND_BACK, GL_KEEP, GL_KEEP, GL_REPLACE);
		glDrawArrays(GL_TRIANGLES, 6, 6);
		glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		glStencilFunc(GL_EQUAL, 1, 0xff);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glStencilFunc(GL_EQUAL, 2, 0xff);
		glDrawArrays(GL_TRIANGLES, 12, 6);
		glFinish();
		glFrontFace(GL_CCW);
		glDisable(GL_STENCIL_TEST);
	} else if (mode == 22) {
		struct cuv_g v[6];
		GLuint vbo;

		link_two_g(vs_cuv, fs_cuv, "a_color", "a_uv");
		cuv_set_g(&v[0], -0.8f, -0.7f, 0.0f, 1, 1, 0);
		cuv_set_g(&v[1],  0.8f, -0.7f, 0.0f, 1, 1, 0);
		cuv_set_g(&v[2],  0.0f,  0.8f, 0.0f, 1, 1, 0);
		cuv_set_g(&v[3], -0.25f, -0.10f, 0.0f, 1, 1, 0);
		cuv_set_g(&v[4],  0.00f,  0.35f, 0.0f, 1, 1, 0);
		cuv_set_g(&v[5],  0.25f, -0.10f, 0.0f, 1, 1, 0);
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
		cuv_attribs_g();

		glFrontFace(GL_CCW);
		glDisable(GL_CULL_FACE);
		glEnable(GL_STENCIL_TEST);
		glClearStencil(0);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
		glStencilFunc(GL_ALWAYS, 0, 0xff);
		glStencilOpSeparate(GL_FRONT, GL_KEEP, GL_KEEP, GL_INCR_WRAP);
		glStencilOpSeparate(GL_BACK,  GL_KEEP, GL_KEEP, GL_DECR_WRAP);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		glStencilFunc(GL_NOTEQUAL, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		glDisable(GL_STENCIL_TEST);
	} else if (mode == 23) {
		static const float pos[6][2] = {
			{ -0.5f,  0.5f }, { -0.9f, -0.5f }, { -0.1f, -0.5f },
			{  0.5f,  0.5f }, {  0.1f, -0.5f }, {  0.9f, -0.5f },
		};
		struct { float pos[4]; float color[4]; } v[6];
		GLuint prog, vbo;
		GLint loc;
		int i;

		prog = link_two_g(vs_plain, fs_uniform, "a_color", NULL);
		for (i = 0; i < 6; i++) {
			v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
			v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
			v[i].color[0] = v[i].color[1] = 1.0f;
			v[i].color[2] = v[i].color[3] = 1.0f;
		}
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(v[0]), (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(v[0]), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);

		loc = glGetUniformLocation(prog, "u_color");
		glClear(GL_COLOR_BUFFER_BIT);
		glUniform4f(loc, 1.0f, 0.5f, 0.0f, 1.0f);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glUniform4f(loc, 0.0f, 1.0f, 1.0f, 1.0f);
		glDrawArrays(GL_TRIANGLES, 3, 3);
		glFinish();
	} else if (mode == 25) {
		/* gl_test 108 D: vec4 fcols[4] read at a uniform int (fidx 2) in a
		 * fragment stage with no varying input, under the ES 1.00
		 * passthrough. On the i.MX8MP it reads element 0 (108, 116, 117,
		 * 120-122 at 1eda74e5) while Mesa passes indexing.uniform_array on
		 * this GPU: the diff of the two streams is the constant file, the
		 * shader and the link state for that shape. Sources verbatim from
		 * tools/shaders/gl_test_es100/passthrough.vert and
		 * tools/shaders/es100/uniidx_d.frag. */
		static const char vs_d[] =
			"#version 100\n"
			"attribute vec4 a_pos;\n"
			"attribute vec4 a_color;\n"
			"varying vec4 v_color;\n"
			"void main() {\n"
			"    gl_Position = a_pos;\n"
			"    v_color = a_color;\n"
			"}\n";
		static const char fs_d[] =
			"precision mediump float;\n"
			"uniform vec4 fcols[4];\n"
			"uniform int fidx;\n"
			"void main() {\n"
			"    gl_FragColor = fcols[fidx];\n"
			"}\n";
		static const float fcols[16] = {
			0.0f, 0.0f, 0.0f, 1.0f,  1.0f, 0.0f, 0.0f, 1.0f,
			0.2f, 0.4f, 0.6f, 1.0f,  0.0f, 1.0f, 0.0f, 1.0f,
		};
		static const float c[6][2] = {
			{ -1, -1 }, { 1, -1 }, { -1, 1 }, { 1, -1 }, { 1, 1 }, { -1, 1 },
		};
		float q[6][8];
		GLuint prog, vbo;
		int i;

		prog = link_two_g(vs_d, fs_d, "a_color", NULL);
		for (i = 0; i < 6; i++) {
			q[i][0] = c[i][0]; q[i][1] = c[i][1]; q[i][2] = 0.0f; q[i][3] = 1.0f;
			q[i][4] = q[i][5] = q[i][6] = q[i][7] = 1.0f;
		}
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		glUniform4fv(glGetUniformLocation(prog, "fcols"), 4, fcols);
		glUniform1i(glGetUniformLocation(prog, "fidx"), 2);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
	} else if (mode == 26) {
		/* dEQP texture.vertex.2d.filtering.nearest_nearest_clamp's shape:
		 * its vertex shader verbatim (es2fVertexTextureTests.cpp), two
		 * 128x128 GL_RGB textures with all eight levels (a gradient and a
		 * 4-texel grid, as the case fills them), NEAREST/NEAREST, clamp,
		 * UNPACK_ALIGNMENT 1 as glu::Texture2D uploads, then the case's
		 * minified cell (texture 0, u_lod = log2(1.8)) and magnified one
		 * (texture 1, u_lod = log2(0.3)) in one frame. Every 2D case fails
		 * on the i.MX8MP while Mesa's r6204 fails list has none of them:
		 * the diff is the vertex-stage sampler, its descriptor and the
		 * texldl for this shape. */
		static const char vs_t[] =
			"attribute highp vec2 a_position;\n"
			"attribute highp vec2 a_texCoord;\n"
			"uniform highp sampler2D u_texture;\n"
			"uniform highp float u_lod;\n"
			"varying mediump vec4 v_color;\n"
			"\n"
			"void main()\n"
			"{\n"
			"    gl_Position = vec4(a_position, 0.0, 1.0);\n"
			"    v_color = texture2DLod(u_texture, a_texCoord, u_lod);\n"
			"}\n";
		static const char fs_t[] =
			"varying mediump vec4 v_color;\n"
			"\n"
			"void main()\n"
			"{\n"
			"    gl_FragColor = v_color;\n"
			"}\n";
		static uint8_t lvl[128 * 128 * 3];
		static const float c[6][2] = {
			{ -1, -1 }, { 1, -1 }, { -1, 1 }, { 1, -1 }, { 1, 1 }, { -1, 1 },
		};
		float q[12][4];
		GLuint prog, vbo, tex[2];
		int i, t, l, x, y;

		prog = link_two_g(vs_t, fs_t, "a_texCoord", NULL);
		glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
		glGenTextures(2, tex);
		for (t = 0; t < 2; t++) {
			glBindTexture(GL_TEXTURE_2D, tex[t]);
			for (l = 0; l < 8; l++) {
				int n = 128 >> l;

				for (y = 0; y < n; y++) {
					for (x = 0; x < n; x++) {
						uint8_t *p = &lvl[(y * n + x) * 3];
						float s = ((float)x + 0.5f) / (float)n;
						float u = ((float)y + 0.5f) / (float)n;

						if (t == 0) {
							/* -0.5..1.0, clamped by the unorm store */
							float v[3] = { -0.5f + 1.5f * s, -0.5f + 1.5f * u,
								       -0.5f + 1.5f * (s + u) * 0.5f };
							int k;

							for (k = 0; k < 3; k++)
								p[k] = (uint8_t)(v[k] <= 0.0f ? 0 : v[k] >= 1.0f ? 255 : v[k] * 255.0f + 0.5f);
						} else {
							uint32_t rgb = (0x00ffffffu / 8u) * (uint32_t)l;
							uint32_t col = ((x / 4 + y / 4) & 1) ? ~rgb : rgb;

							p[0] = (uint8_t)col; p[1] = (uint8_t)(col >> 8); p[2] = (uint8_t)(col >> 16);
						}
					}
				}
				glTexImage2D(GL_TEXTURE_2D, l, GL_RGB, n, n, 0, GL_RGB, GL_UNSIGNED_BYTE, lvl);
			}
		}
		/* two quads: the left half and the right half */
		for (i = 0; i < 12; i++) {
			const float *v = c[i % 6];
			float x0 = i < 6 ? -1.0f : 0.0f;

			q[i][0] = x0 + (v[0] + 1.0f) * 0.5f;
			q[i][1] = v[1];
			q[i][2] = (v[0] + 1.0f) * 0.5f;
			q[i][3] = (v[1] + 1.0f) * 0.5f;
		}
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
		glVertexAttribPointer(glGetAttribLocation(prog, "a_position"), 2, GL_FLOAT, GL_FALSE,
				      sizeof(q[0]), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)8);
		glEnableVertexAttribArray(glGetAttribLocation(prog, "a_position"));
		glEnableVertexAttribArray(1);
		glUniform1i(glGetUniformLocation(prog, "u_texture"), 0);
		glActiveTexture(GL_TEXTURE0);
		glClearColor(0.125f, 0.25f, 0.5f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		for (t = 0; t < 2; t++) {
			glBindTexture(GL_TEXTURE_2D, tex[t]);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
			glUniform1f(glGetUniformLocation(prog, "u_lod"), t == 0 ? 0.847997f : -1.736966f);
			glDrawArrays(GL_TRIANGLES, 6 * t, 6);
		}
		glFinish();
	} else if (mode == 27) {
		/* Mode 26 drawn as dEQP draws it: two SEPARATE client arrays
		 * (a_position and a_texCoord, both vec2, stride 0) and client
		 * ushort indices through glDrawElements over dEQP's own grid per
		 * cell (127x127 quads, four vertices each), no buffer object. Mode 26 below says the rest:
		 * dEQP texture.vertex.2d.filtering.nearest_nearest_clamp's shape:
		 * its vertex shader verbatim (es2fVertexTextureTests.cpp), two
		 * 128x128 GL_RGB textures with all eight levels (a gradient and a
		 * 4-texel grid, as the case fills them), NEAREST/NEAREST, clamp,
		 * UNPACK_ALIGNMENT 1 as glu::Texture2D uploads, then the case's
		 * minified cell (texture 0, u_lod = log2(1.8)) and magnified one
		 * (texture 1, u_lod = log2(0.3)) in one frame. Every 2D case fails
		 * on the i.MX8MP while Mesa's r6204 fails list has none of them:
		 * the diff is the vertex-stage sampler, its descriptor and the
		 * texldl for this shape. */
		static const char vs_t[] =
			"attribute highp vec2 a_position;\n"
			"attribute highp vec2 a_texCoord;\n"
			"uniform highp sampler2D u_texture;\n"
			"uniform highp float u_lod;\n"
			"varying mediump vec4 v_color;\n"
			"\n"
			"void main()\n"
			"{\n"
			"    gl_Position = vec4(a_position, 0.0, 1.0);\n"
			"    v_color = texture2DLod(u_texture, a_texCoord, u_lod);\n"
			"}\n";
		static const char fs_t[] =
			"varying mediump vec4 v_color;\n"
			"\n"
			"void main()\n"
			"{\n"
			"    gl_FragColor = v_color;\n"
			"}\n";
		static uint8_t lvl[128 * 128 * 3];
		static float pos[127 * 127 * 4][2], uv[127 * 127 * 4][2];
		static uint16_t idx[127 * 127 * 6];
		GLuint prog, tex[2];
		int i, t, l, x, y;

		prog = link_two_g(vs_t, fs_t, "a_texCoord", NULL);
		glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
		glGenTextures(2, tex);
		for (t = 0; t < 2; t++) {
			glBindTexture(GL_TEXTURE_2D, tex[t]);
			for (l = 0; l < 8; l++) {
				int n = 128 >> l;

				for (y = 0; y < n; y++) {
					for (x = 0; x < n; x++) {
						uint8_t *p = &lvl[(y * n + x) * 3];
						float s = ((float)x + 0.5f) / (float)n;
						float u = ((float)y + 0.5f) / (float)n;

						if (t == 0) {
							/* -0.5..1.0, clamped by the unorm store */
							float v[3] = { -0.5f + 1.5f * s, -0.5f + 1.5f * u,
								       -0.5f + 1.5f * (s + u) * 0.5f };
							int k;

							for (k = 0; k < 3; k++)
								p[k] = (uint8_t)(v[k] <= 0.0f ? 0 : v[k] >= 1.0f ? 255 : v[k] * 255.0f + 0.5f);
						} else {
							uint32_t rgb = (0x00ffffffu / 8u) * (uint32_t)l;
							uint32_t col = ((x / 4 + y / 4) & 1) ? ~rgb : rgb;

							p[0] = (uint8_t)col; p[1] = (uint8_t)(col >> 8); p[2] = (uint8_t)(col >> 16);
						}
					}
				}
				glTexImage2D(GL_TEXTURE_2D, l, GL_RGB, n, n, 0, GL_RGB, GL_UNSIGNED_BYTE, lvl);
			}
		}
		/* dEQP's own grid (es2fVertexTextureTests PosTexCoordQuadGrid):
		 * GRID_SIZE_2D = 127 quads a side, FOUR vertices a quad (none
		 * shared), positions and texcoords apart: 1,225,804 bytes of
		 * client arrays and indices, more than the GC7000 backend's
		 * 1 MiB frame arena */
		for (y = 0, i = 0; y < 127; y++) {
			for (x = 0; x < 127; x++) {
				const float fx0 = (float)x / 127.0f, fx1 = (float)(x + 1) / 127.0f;
				const float fy0 = (float)y / 127.0f, fy1 = (float)(y + 1) / 127.0f;
				const float q[4][2] = { { fx0, fy0 }, { fx1, fy0 }, { fx0, fy1 }, { fx1, fy1 } };
				const int v = (y * 127 + x) * 4;
				int k;

				for (k = 0; k < 4; k++) {
					pos[v + k][0] = q[k][0] * 2.0f - 1.0f;
					pos[v + k][1] = q[k][1] * 2.0f - 1.0f;
					uv[v + k][0] = q[k][0];
					uv[v + k][1] = q[k][1];
				}
				idx[i++] = (uint16_t)v; idx[i++] = (uint16_t)(v + 1); idx[i++] = (uint16_t)(v + 2);
				idx[i++] = (uint16_t)(v + 1); idx[i++] = (uint16_t)(v + 3); idx[i++] = (uint16_t)(v + 2);
			}
		}
		glBindBuffer(GL_ARRAY_BUFFER, 0);
		glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
		glVertexAttribPointer(glGetAttribLocation(prog, "a_position"), 2, GL_FLOAT, GL_FALSE, 0, pos);
		glVertexAttribPointer(glGetAttribLocation(prog, "a_texCoord"), 2, GL_FLOAT, GL_FALSE, 0, uv);
		glEnableVertexAttribArray(glGetAttribLocation(prog, "a_position"));
		glEnableVertexAttribArray(glGetAttribLocation(prog, "a_texCoord"));
		glUniform1i(glGetUniformLocation(prog, "u_texture"), 0);
		glActiveTexture(GL_TEXTURE0);
		glClearColor(0.125f, 0.25f, 0.5f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		for (t = 0; t < 2; t++) {
			glBindTexture(GL_TEXTURE_2D, tex[t]);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
			glUniform1f(glGetUniformLocation(prog, "u_lod"), t == 0 ? 0.847997f : -1.736966f);
			glDrawElements(GL_TRIANGLES, 127 * 127 * 6, GL_UNSIGNED_SHORT, idx);
		}
		glFinish();
	} else if (mode == 24) {
		/* case 24: bind red to unit 0 (which the shader does not read)
		 * and green to unit 1 (which it does). The compiler compacts the
		 * surviving sampler to register 0, so slot 0's descriptor has to
		 * be green. The 1x1 texture is the point: no other mode has one,
		 * and its stride and padding are what a descriptor diff shows. */
		static const uint8_t red[4]   = { 255, 0, 0, 255 };
		static const uint8_t green[4] = { 0, 255, 0, 255 };
		static const float c[6][4] = {
			{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
			{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
		};
		struct puv { float pos[4]; float uv[2]; } q[6];
		GLuint prog, t0 = 0, t1 = 0, vbo;
		int i;

		prog = link_two_g(vs_uv, fs_compact, "a_uv", NULL);
		for (i = 0; i < 6; i++) {
			q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1];
			q[i].pos[2] = 0.0f;    q[i].pos[3] = 1.0f;
			q[i].uv[0]  = c[i][2]; q[i].uv[1]  = c[i][3];
		}
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);

		glActiveTexture(GL_TEXTURE0);
		glGenTextures(1, &t0);
		glBindTexture(GL_TEXTURE_2D, t0);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, red);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glActiveTexture(GL_TEXTURE1);
		glGenTextures(1, &t1);
		glBindTexture(GL_TEXTURE_2D, t1);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, green);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glActiveTexture(GL_TEXTURE0);

		fprintf(stderr, "u_tex0 loc %d, u_tex1 loc %d\n",
			glGetUniformLocation(prog, "u_tex0"),
			glGetUniformLocation(prog, "u_tex1"));
		glUniform1i(glGetUniformLocation(prog, "u_tex0"), 0);
		glUniform1i(glGetUniformLocation(prog, "u_tex1"), 1);

		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
	} else {
		fprintf(stderr, "unknown mode %d\n", mode);
		return 1;
	}

	fprintf(stderr, "drew; gl error 0x%x\n", glGetError());
	return 0;
}

int main(int argc, char **argv)
{
	int mode = argc > 1 ? atoi(argv[1]) : 1;
	const int textured = mode == 4 || mode == 5 || mode == 10 || mode == 16;
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLint major, minor, n;
	EGLConfig cfg;
	EGLContext ctx;
	static const EGLint cfg_attrs[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_NONE
	};
	static const EGLint ctx_attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	GLuint prog, fbo, rb, vbo;
	GLint loc;
	static const float verts[] = {
		 0.0f,  0.75f, 0.0f, 1.0f,   1, 1, 1, 1,
		-0.75f, -0.75f, 0.0f, 1.0f,  1, 1, 1, 1,
		 0.75f, -0.75f, 0.0f, 1.0f,  1, 1, 1, 1,
	};

	/* A mode this harness does not implement must not quietly draw mode 1's
	 * shape. Diffed against the golden's real mode it looks exactly like a
	 * driver bug and is not one: measured on 2026-09-22, modes 2 and 3 gave
	 * mode 1's stream in cs_ours while cs_golden gave their own, so the diff
	 * showed a vertex uniform we were "missing" and never write. Refuse
	 * instead, and say what there is. */
	{
		/* Keep this in step with the `if (mode == N)` branches below: the
		 * guard is a second place the mode set is written down, so it can
		 * drift from the code the way the old duplicate mode list did. If
		 * you add a branch, add the number here. */
		static const int have[] = { 1, 2, 3, 4, 5, 7, 8, 10, 11, 12, 14, 15, 16, 17, 18,
					    19, 20, 21, 22, 23, 24, 25, 26, 27 };
		size_t k;
		int known = 0;

		for (k = 0; k < sizeof(have) / sizeof(have[0]); k++)
			if (have[k] == mode)
				known = 1;
		if (!known) {
			fprintf(stderr, "%s: no mode %d here. This harness has:", argv[0], mode);
			for (k = 0; k < sizeof(have) / sizeof(have[0]); k++)
				fprintf(stderr, " %d", have[k]);
			fprintf(stderr, "\n(cs_ours.c's mode list says what each one is)\n");
			return 2;
		}
	}

	if (!eglInitialize(dpy, &major, &minor)) {
		fprintf(stderr, "eglInitialize failed\n");
		return 1;
	}
	eglBindAPI(EGL_OPENGL_ES_API);
	if (!eglChooseConfig(dpy, cfg_attrs, &cfg, 1, &n) || n < 1) {
		fprintf(stderr, "eglChooseConfig failed\n");
		return 1;
	}
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attrs);
	if (ctx == EGL_NO_CONTEXT) {
		fprintf(stderr, "eglCreateContext failed\n");
		return 1;
	}
	/* No surface: the target is an FBO, which is all a stream needs. */
	if (!eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)) {
		fprintf(stderr, "eglMakeCurrent failed 0x%x\n", eglGetError());
		return 1;
	}
	fprintf(stderr, "renderer: %s\n", (const char *)glGetString(GL_RENDERER));

	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glGenRenderbuffers(1, &rb);
	glBindRenderbuffer(GL_RENDERBUFFER, rb);
	/* The depth modes compare depth state, and a 16-bit colour target puts
	 * a spurious difference into every line of that comparison: Mesa
	 * derives PE_COLOR_FORMAT and PE_LOGIC_OP's SINGLE_BUFFER from the
	 * colour format, so RGBA4 against this backend's RGBA8 makes those two
	 * registers disagree for a reason that has nothing to do with depth.
	 * Everything else keeps RGBA4, so the readings taken against it stand. */
	glRenderbufferStorage(GL_RENDERBUFFER,
			      mode == 8 || mode == 12 || mode == 14 || mode == 15 || mode == 18 ? GL_RGBA8_OES
								    : GL_RGBA4,
			      64, 64);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
	if (mode == 8 || mode == 12 || mode == 14 || mode == 15 || mode == 18 ||
	    mode == 21 || mode == 22) {
		/* a depth attachment, so the driver has a depth surface to
		 * describe: the only way to see what it tells the PE about it.
		 * The stencil modes need the stencil half of the same buffer. */
		GLuint drb;

		glGenRenderbuffers(1, &drb);
		glBindRenderbuffer(GL_RENDERBUFFER, drb);
		/* D24S8, because that is the one buffer this backend keeps and
		 * comparing it against a 16-bit one would only measure the
		 * harness. */
		glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, 64, 64);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT,
					  GL_RENDERBUFFER, drb);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT,
					  GL_RENDERBUFFER, drb);
		if (mode == 8) {
			glEnable(GL_DEPTH_TEST);
			glDepthFunc(GL_LESS);
		}
	}
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		fprintf(stderr, "fbo incomplete\n");
		return 1;
	}
	if (mode == 11) {
		/* the same cull state as ours, on an FBO target too */
		glEnable(GL_CULL_FACE);
		glFrontFace(GL_CCW);
		glCullFace(GL_BACK);
	}
	glViewport(0, 0, 64, 64);

	if (mode >= 19) {
		return frag_mode_g(mode);
	}

	prog = glCreateProgram();
	glAttachShader(prog, compile(GL_VERTEX_SHADER,
				     mode == 7 ? vs_points :
				     mode == 11 || mode == 17 ? vs_dir :
				     textured ? vs_uv : mode == 2 ? vs_mvp : vs_plain));
	glAttachShader(prog, compile(GL_FRAGMENT_SHADER,
				     mode == 7 ? fs_varying :
				     mode == 11 || mode == 17 ? fs_cube :
				     textured ? fs_tex : fs_uniform));
	glBindAttribLocation(prog, 0, "a_pos");
	glBindAttribLocation(prog, 1, textured ? "a_uv" : "a_color");
	if (mode == 7) {
		glBindAttribLocation(prog, 2, "a_size");
	}
	glLinkProgram(prog);
	glUseProgram(prog);

	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 32, (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 32, (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	if (mode == 2) {
		static const float mvp[16] = {
			0.5f, 0, 0, 0,  0, 0.5f, 0, 0,  0, 0, 1, 0,  0, 0, 0, 1
		};

		loc = glGetUniformLocation(prog, "u_mvp");
		glUniformMatrix4fv(loc, 1, GL_FALSE, mvp);
	}
	if (mode == 7) {
		/* gl_test case 10's frame 5: three points of 1, 3 and 5 px,
		 * the size coming from an attribute. */
		struct { float pos[4]; float color[4]; float size; } pv[3];
		static const float sz[3] = { 1.0f, 3.0f, 5.0f };
		int i;

		for (i = 0; i < 3; i++) {
			pv[i].pos[0] = -0.5f + 0.5f * (float)i;
			pv[i].pos[1] = 0.0f; pv[i].pos[2] = 0.0f; pv[i].pos[3] = 1.0f;
			pv[i].color[0] = 0.0f; pv[i].color[1] = 0.0f;
			pv[i].color[2] = 1.0f; pv[i].color[3] = 1.0f;
			pv[i].size = sz[i];
		}
		glBufferData(GL_ARRAY_BUFFER, sizeof(pv), pv, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(pv[0]),
				      (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(pv[0]),
				      (const void *)16);
		glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, sizeof(pv[0]),
				      (const void *)32);
		glEnableVertexAttribArray(2);
		goto draw;
	}
	if (mode == 5) {
		/* gl_test case 2's mip sub-test, exactly: a 4x4 texture whose
		 * levels are red, green and blue, and a quad two pixels wide,
		 * so two texels fall in each pixel and level 1 is the one the
		 * hardware should fetch. */
		static unsigned char red[4 * 4 * 4], green[2 * 2 * 4], blue[4];
		struct { float pos[4]; float uv[2]; } q[6];
		const float x0 = 30.0f, n = 2.0f;
		const float l = x0 / 64 * 2 - 1, r = (x0 + n) / 64 * 2 - 1;
		const float t = 1 - x0 / 64 * 2, b = 1 - (x0 + n) / 64 * 2;
		const float c[6][4] = {
			{ l, b, 0, 0 }, { r, b, 1, 0 }, { r, t, 1, 1 },
			{ l, b, 0, 0 }, { r, t, 1, 1 }, { l, t, 0, 1 },
		};
		GLuint tex;
		int i;

		for (i = 0; i < 16; i++) { red[i * 4] = 255; red[i * 4 + 3] = 255; }
		for (i = 0; i < 4; i++) { green[i * 4 + 1] = 255; green[i * 4 + 3] = 255; }
		blue[2] = 255; blue[3] = 255;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, red);
		glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA, 2, 2, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, green);
		glTexImage2D(GL_TEXTURE_2D, 2, GL_RGBA, 1, 1, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, blue);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER,
				GL_NEAREST_MIPMAP_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);

		for (i = 0; i < 6; i++) {
			q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1];
			q[i].pos[2] = 0.0f;    q[i].pos[3] = 1.0f;
			q[i].uv[0]  = c[i][2]; q[i].uv[1]  = c[i][3];
		}
		glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(q[0]),
				      (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(q[0]),
				      (const void *)16);
		goto draw;
	}
	if (mode == 11 || mode == 17) {
		/* Six 8x8 faces, one flat colour each: gl_test case 41's
		 * texture. What the descriptor says about a cube -- its TYPE,
		 * the SLICE that steps from one face to the next, 3D_CONFIG's
		 * depth -- is the whole point of this mode. */
		static const unsigned char face_rgb[6][3] = {
			{ 255, 0, 0 }, { 0, 255, 0 }, { 0, 0, 255 },
			{ 255, 255, 0 }, { 255, 0, 255 }, { 0, 255, 255 },
		};
		static unsigned char texels[8 * 8 * 4];
		GLuint tex;
		int f, i;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
		for (f = 0; f < 6; f++) {
			for (i = 0; i < 8 * 8; i++) {
				texels[i * 4 + 0] = face_rgb[f][0];
				texels[i * 4 + 1] = face_rgb[f][1];
				texels[i * 4 + 2] = face_rgb[f][2];
				texels[i * 4 + 3] = 255;
			}
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA,
				     8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
		}
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_cube"), 0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 32, (const void *)16);
		if (mode != 17)
			goto draw;

		/* Mode 17 is gl_test case 41's second half, which mode 11
		 * cannot reach: render INTO one cube face through an FBO, then
		 * sample the cube again. The interesting state is between the
		 * two kicks -- what the driver does when a face that was just a
		 * render target becomes a texture -- so this is one submit per
		 * frame, the shape mode 14 uses for case 3.
		 *
		 * Case 42 does the same with a 2D array layer, so whatever this
		 * settles about a layered render target applies there too. */
		{
			GLuint cfbo = 0;

			glGenFramebuffers(1, &cfbo);
			glBindFramebuffer(GL_FRAMEBUFFER, cfbo);
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
					       GL_TEXTURE_CUBE_MAP_POSITIVE_Y, tex, 0);
			if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
				fprintf(stderr, "cs_golden: mode 17: cube-face FBO incomplete\n");
				return 3;
			}
			/* frame 1: clear the +Y face white, no draw */
			glViewport(0, 0, 8, 8);
			glClearColor(1.0f, 1.0f, 1.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glFinish();

			/* frame 2: back to the window, sample +Y, which should
			 * now read white rather than its uploaded colour */
			glBindFramebuffer(GL_FRAMEBUFFER, fbo);
			glViewport(0, 0, 64, 64);
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
		}
		goto done;
	}
	if (mode == 16) {
		/* gl_test case 20's ETC2 RGBA8 EAC texture: 8x8, four 16-byte
		 * blocks, alpha 255/128/64/0 over red/green/blue/white. The ETC1
		 * texture of mode 10 already matches Mesa byte for byte; this is
		 * the half that has never been compared. */
		static const unsigned char alphas[4] = { 255, 128, 64, 0 };
		static const unsigned char cols[4][3] = {
			{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 },
		};
		static unsigned char etc2a[4 * 16];
		GLuint tex;
		int i;

		for (i = 0; i < 4; i++) {
			unsigned char *blk = &etc2a[i * 16];

			memset(blk, 0, 16);
			blk[0] = alphas[i];
			blk[8]  = (unsigned char)((cols[i][0] << 4) | cols[i][0]);
			blk[9]  = (unsigned char)((cols[i][1] << 4) | cols[i][1]);
			blk[10] = (unsigned char)((cols[i][2] << 4) | cols[i][2]);
		}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x9278 /* RGBA8_ETC2_EAC */,
				       8, 8, 0, sizeof(etc2a), etc2a);
		/* ETC2 is core in ES 3.00 and absent from this ES 2.0 context, so
		 * the upload is rejected and the binding keeps whatever default
		 * texture it had. Emitting a descriptor then would be the worst
		 * outcome: a plausible-looking golden -- SIZE 1x1, FORMAT 7, no
		 * compression -- that describes nothing and invites a diff
		 * against it. Stop here instead, loudly. The drm-shim GC7000UL
		 * offers no ES3 config, so asking for an ES 3.0 context does not
		 * rescue this either; ours is compared against Mesa's rules in
		 * etnaviv_texture_desc.c instead of against a stream. */
		{
			GLenum e = glGetError();

			if (e != GL_NO_ERROR) {
				fprintf(stderr, "cs_golden: mode 16 needs ETC2, which this "
					"ES 2.0 context rejected (gl error 0x%x). There is no "
					"golden for it; see VIVANTE_NOTES.md.\n", e);
				return 3;
			}
		}
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 32, (const void *)16);
		goto draw;
	}
	if (mode == 10) {
		/* gl_test case 20's ETC1 texture. Each 4x4 block is flat: both
		 * halves the same 4-bit colour, table 0, every index 0, so the
		 * texel is c * 17 + 2 and a block that lands in the wrong place
		 * says so in one colour. */
		static const unsigned char cols[8][3] = {
			{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 },
			{ 15, 15, 0 }, { 0, 15, 15 }, { 15, 0, 15 }, { 8, 8, 8 },
		};
		static unsigned char etc1[8 * 8];
		GLuint tex;
		int b;

		for (b = 0; b < 8; b++) {
			unsigned char *blk = &etc1[b * 8];

			blk[0] = (unsigned char)((cols[b][0] << 4) | cols[b][0]);
			blk[1] = (unsigned char)((cols[b][1] << 4) | cols[b][1]);
			blk[2] = (unsigned char)((cols[b][2] << 4) | cols[b][2]);
			blk[3] = blk[4] = blk[5] = blk[6] = blk[7] = 0;
		}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES,
				       16, 8, 0, sizeof(etc1), etc1);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 32, (const void *)16);
		goto draw;
	}
	if (mode == 4) {
		/* gl_test's quadrant texture: red | green on the t = 0 side,
		 * blue | white on the t = 1 side. Red and blue are what a
		 * channel-order mistake transposes, which is the point. */
		static uint8_t texels[16 * 16 * 4];
		GLuint tex;
		int x, y;

		for (y = 0; y < 16; y++) {
			for (x = 0; x < 16; x++) {
				uint8_t *q = &texels[(y * 16 + x) * 4];
				int right = x >= 8, top = y >= 8;

				q[0] = (!top && !right) || (top && right) ? 255 : 0;
				q[1] = (!top && right) || (top && right) ? 255 : 0;
				q[2] = top ? 255 : 0;
				q[3] = 255;
			}
		}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 16, 16, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, texels);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
		/* the second attribute is two components here, not four */
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 32, (const void *)16);
		goto draw;
	}

	loc = glGetUniformLocation(prog, "u_color");
	if (loc < 0) {
		fprintf(stderr, "u_color not found\n");
		return 1;
	}
	/* The very value gl_test case 1 sets for its first frame. */
	glUniform4f(loc, 1.0f, 0.5f, 0.0f, 1.0f);

draw:
	glClearColor(0, 0, 0, 1);
	/* Case 3 clears colour and depth together, and how the depth half of
	 * that clear is done -- a fast clear through the tile status, or a
	 * quad -- is one of the things this mode is here to show. */
	if (mode == 15) {
		/* A depth clear on its own, with no draw behind it. GL has to
		 * carry it out at the glFinish; the question this mode asks is
		 * whether a kick comes out at all. */
		glEnable(GL_DEPTH_TEST);
		glDepthFunc(GL_LESS);
		glClearDepthf(0.5f);
		glClear(GL_DEPTH_BUFFER_BIT);
		glFinish();
		glClearDepthf(1.0f);
		fprintf(stderr, "drew; gl error 0x%x\n", glGetError());
		return 0;
	}

	if (mode == 18) {
		/* Case 13's last frame, its only remaining failure. The far
		 * quad is drawn under glDepthRangef(0, 0.3) and the mid quad
		 * under the full range. The clear is still pending when the
		 * first draw takes it, so whatever range that draw carries is
		 * the range the clear quad is transformed by -- which is the
		 * thing to read here. */
		glClearDepthf(0.5f);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		glDepthRangef(0.0f, 0.3f);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glDepthRangef(0.0f, 1.0f);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		glClearDepthf(1.0f);
		fprintf(stderr, "drew; gl error 0x%x\n", glGetError());
		return 0;
	}

	/* Case 13 clears depth to 0.5 rather than the default 1.0, so whatever
	 * carries the clear value -- a tile-status word or a quad's z -- has to
	 * carry that value and not the default. */
	if (mode == 12)
		glClearDepthf(0.5f);
	glClear(mode == 8 || mode == 12 || mode == 14 || mode == 15 || mode == 18 ? (GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
					: GL_COLOR_BUFFER_BIT);
	if (mode == 7) {
		glDrawArrays(GL_POINTS, 0, 3);
	} else {
		glDrawArrays(GL_TRIANGLES, 0, mode == 5 ? 6 : 3);
	}
	glFinish();

	if (mode == 14) {
		/* Case 3's remaining frames, which is where it hangs on the
		 * board. Everything a single-frame mode cannot show lives
		 * between these kicks: what a driver emits when the depth
		 * state changes under it, and what it does with a depth clear
		 * that is not the default. */

		/* frame 2: the depth test off entirely, so draw order wins */
		glDisable(GL_DEPTH_TEST);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();

		/* frame 3: GREATER, and a depth clear of 0.0 -- the far end of
		 * the range rather than the near one, so a clear quad for it
		 * sits exactly on the near plane */
		glEnable(GL_DEPTH_TEST);
		glDepthFunc(GL_GREATER);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		glClearDepthf(0.0f);
		glClear(GL_DEPTH_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		glClearDepthf(1.0f);

		/* frames 4 and 5: the same geometry twice with a polygon
		 * offset, the one piece of depth state gl_test exercises that
		 * no other mode reaches */
		glDepthFunc(GL_LESS);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glEnable(GL_POLYGON_OFFSET_FILL);
		glPolygonOffset(0.0f, -4.0f);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		glPolygonOffset(0.0f, 4.0f);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		glDisable(GL_POLYGON_OFFSET_FILL);
	}


	if (mode == 3) {
		/* The same program, a different value: nothing about the shader
		 * changed, so this frame shows the uniform write on its own. */
		glUniform4f(loc, 0.0f, 1.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
	}

done:
	fprintf(stderr, "drew; gl error 0x%x\n", glGetError());
	return 0;
}
