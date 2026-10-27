/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * cs_ours.c - this backend's command stream for the same draw as cs_golden.c.
 *
 * The host OSAL gives the Vivante backend malloc'd "GPU" memory and keeps the
 * stream instead of running it, so the bytes this driver would hand the GC7000
 * can be read on the host and diffed against the ones Mesa produces. The GL
 * calls are gl_test's own and so are the shader sources, so the offline
 * table's SHA-256 lookup finds the same shaders the board runs.
 *
 * argv[1] picks the shape. The mode numbers and what each one means are
 * listed once, below, next to the rule for adding a new one.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <GLES2/gl2.h>

#include "gl_internal.h"
#include "gl_context.h"
#include "backend.h"
#include "osal_internal.h"

#include "gl_test_shaders.h"

/* The same 64x64 target gl_test uses, so the state the backend derives from
 * the surface matches too. */
#define TW 64
#define TH 64

/* ---- the mode numbers, in one place ------------------------------------
 *
 * Both harnesses take the same mode number and must agree on what it means,
 * and two of us add modes independently. Mode 15 was claimed twice in one
 * evening -- a depth clear and an ETC2 texture -- and the two commits did not
 * conflict textually, so the collision arrived silently and mode 15 quietly
 * did both. Add the row here first, then the code.
 *
 *   1     a fragment uniform only (gl_test case 1)
 *   2     a vertex uniform as well, so the split between stages shows
 *   3     two frames with the uniform changed between them: case 1 in full
 *   4     a textured quad, NEAREST + clamp (gl_test case 2)
 *   5     case 2's mip sub-test: a 4x4 chain, red/green/blue levels
 *   6     the same, from gl_test's glGenerateMipmap base -- left half red,
 *         right half blue, so level 2 has to average to purple
 *   7     points with gl_PointSize (gl_test case 10)
 *   8     a depth-tested draw against D24S8 (gl_test case 3's shape)
 *   9     render into a texture through an FBO: case 6's target, and the one
 *         case 4's check 5 uses. An FBO is not y-flipped, so everything the
 *         flip touches differs here. Modes 10 and 11 render this way too.
 *   10    case 20's ETC1 texture, 16x8, eight flat blocks
 *   11    a cube map: six 8x8 faces (cases 41/42/48)
 *   12    a depth clear to something other than 1.0
 *   13    ours only: case 48's 4x4x5 volume (no golden: ES 2.0 has no 3D)
 *   14    all of case 3 replayed, one submit per frame
 *   15    a depth clear on its own, no draw behind it
 *   16    case 20's ETC2 RGBA8 EAC texture (no golden: ES 2.0 has no ETC2)
 *   17    case 41's second half: render into a cube face through an FBO, then
 *         sample the cube again -- two frames, one submit each
 *   18    case 13's last frame: two draws in one frame under different
 *         glDepthRangef ranges. The clear is pending when the first
 *         draw takes it, so the clear quad inherits that draw's range
 *   19    case 8 frame 1: a blended draw over an opaque one
 *   20    case 8 frames 5 and 6: glBlendColor's factors, then a colour mask
 *         that writes only green and alpha
 *   21    case 12: two-sided stencil, front reference 1 and back 2, drawn
 *         twice -- once with CCW fronts and once with CW -- because this
 *         pixel engine has no front-face register and what moves between the
 *         two frames is how a driver names the faces
 *   22    case 23: INCR_WRAP / DECR_WRAP winding, then a NOTEQUAL cover
 *   23    case 34: two draws in one frame with the uniform changed between
 *   24    case 24: two samplers declared, only the second sampled, so the
 *         1x1 descriptor and the sampler slot can both be checked
 *   25    gl_test 108 D: vec4 fcols[4] read at a uniform int in a fragment
 *         stage with no varying input (reads element 0 on the i.MX8MP). Ours
 *         needs the online compiler: build_cs_ours.sh with SHADERC_ONLINE=1
 *   26    dEQP texture.vertex.2d.filtering.nearest_nearest_clamp's shape:
 *         texture2DLod in the vertex stage over two 128x128 RGB mipmapped
 *         textures, a minified and a magnified draw in one frame. Online too
 */
struct pc_vertex {
	float pos[4];
	float color[4];
};

static GLuint compile(GLenum stage, const char *src)
{
	GLuint s = glCreateShader(stage);
	GLint ok = 0;

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		char log[512] = { 0 };

		glGetShaderInfoLog(s, sizeof(log) - 1, NULL, log);
		fprintf(stderr, "compile failed: %s\n", log);
		exit(1);
	}
	return s;
}

/*
 * Modes 10..14: the per-pixel state gl_test's blend, stencil and uniform
 * cases set. Each builds its own program and draws its own frame, so none of
 * them disturbs the shapes above; the point is the register block, not the
 * geometry.
 *
 *  10  gl_test case 8 frame 1: an opaque red quad, then a half-alpha green
 *      triangle blended SRC_ALPHA / ONE_MINUS_SRC_ALPHA
 *  11  gl_test case 8 frames 5 and 6: the same with CONSTANT_COLOR /
 *      ONE_MINUS_CONSTANT_COLOR and glBlendColor, then a colour mask of G+A
 *  12  gl_test case 12: two-sided stencil -- front and back reference values
 *      written by one draw, then two covers that test for each of them
 *  13  gl_test case 23: FemtoVG's fill -- INCR_WRAP on the front face and
 *      DECR_WRAP on the back, then a cover pass of NOTEQUAL 0
 *  14  gl_test case 34: two draws in one frame with the uniform changed
 *      between them, so the stream shows whether each draw carries its own
 */
struct cuv_vertex_h { float pos[4]; float color[4]; float uv[2]; };

static void cuv_set_h(struct cuv_vertex_h *v, float x, float y, float z,
		      float r, float g, float b)
{
	v->pos[0] = x; v->pos[1] = y; v->pos[2] = z; v->pos[3] = 1.0f;
	v->color[0] = r; v->color[1] = g; v->color[2] = b; v->color[3] = 1.0f;
	v->uv[0] = 1.0f; v->uv[1] = 1.0f;
}

static void cuv_attribs_h(void)
{
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex_h), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex_h), (const void *)16);
	glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex_h), (const void *)32);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
}

static GLuint link_two(const char *vs, const char *fs, const char *a1, const char *a2)
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
		char log[512] = { 0 };

		glGetProgramInfoLog(p, sizeof(log) - 1, NULL, log);
		fprintf(stderr, "link failed: %s\n", log);
		exit(1);
	}
	glUseProgram(p);
	return p;
}

/* The quad-and-triangle geometry cases 8 and 12 share: 0..5 a full-screen red
 * quad, 6..8 a green triangle at half alpha. */
static void blend_geometry(void)
{
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};
	struct cuv_vertex_h v[9];
	GLuint vbo;
	int i;

	for (i = 0; i < 6; i++) {
		cuv_set_h(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
	}
	cuv_set_h(&v[6], -0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set_h(&v[7], 0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set_h(&v[8], 0.0f, 0.75f, 0.0f, 0, 1, 0);
	for (i = 6; i < 9; i++) {
		v[i].color[3] = 0.5f;
	}
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
	cuv_attribs_h();
}

static int frag_mode(int mode)
{
	glViewport(0, 0, TW, TH);
	glClearColor(0, 0, 0, 1);

	if (mode == 19 || mode == 20) {
		link_two(gl_test_vs_src, gl_test_fs_src, "a_color", "a_uv");
		blend_geometry();
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
			/* frame 6: no blending, only the G and A channels written */
			glDisable(GL_BLEND);
			glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_TRUE);
			glDrawArrays(GL_TRIANGLES, 6, 3);
			glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		}
		glFinish();
		glDisable(GL_BLEND);
	} else if (mode == 21) {
		/* case 12's stencil2_frame: one draw writes 1 on front faces
		 * and 2 on back faces, then two covers read them back. */
		static const float c[6][2] = {
			{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
		};
		struct cuv_vertex_h v[18];
		GLuint vbo;
		int i;

		link_two(gl_test_vs_src, gl_test_fs_src, "a_color", "a_uv");
		for (i = 0; i < 6; i++) {
			cuv_set_h(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
			cuv_set_h(&v[12 + i], c[i][0], c[i][1], 0.0f, 0, 0, 1);
		}
		cuv_set_h(&v[6], -0.9f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_h(&v[7], -0.1f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_h(&v[8], -0.5f, 0.8f, 0.0f, 1, 1, 1);
		cuv_set_h(&v[9], 0.1f, -0.8f, 0.0f, 1, 1, 1);
		cuv_set_h(&v[10], 0.5f, 0.8f, 0.0f, 1, 1, 1);
		cuv_set_h(&v[11], 0.9f, -0.8f, 0.0f, 1, 1, 1);
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
		cuv_attribs_h();

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
		/* case 23: the winding fill. An outer CCW triangle and an
		 * inner CW one, INCR_WRAP / DECR_WRAP, then NOTEQUAL 0. */
		struct cuv_vertex_h v[6];
		GLuint vbo;

		link_two(gl_test_vs_src, gl_test_fs_src, "a_color", "a_uv");
		cuv_set_h(&v[0], -0.8f, -0.7f, 0.0f, 1, 1, 0);
		cuv_set_h(&v[1],  0.8f, -0.7f, 0.0f, 1, 1, 0);
		cuv_set_h(&v[2],  0.0f,  0.8f, 0.0f, 1, 1, 0);
		cuv_set_h(&v[3], -0.25f, -0.10f, 0.0f, 1, 1, 0);
		cuv_set_h(&v[4],  0.00f,  0.35f, 0.0f, 1, 1, 0);
		cuv_set_h(&v[5],  0.25f, -0.10f, 0.0f, 1, 1, 0);
		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
		cuv_attribs_h();

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
		/* case 34: two draws, one frame, the uniform changed between
		 * them. Both triangles must keep their own colour. */
		static const float pos[6][2] = {
			{ -0.5f,  0.5f }, { -0.9f, -0.5f }, { -0.1f, -0.5f },
			{  0.5f,  0.5f }, {  0.1f, -0.5f }, {  0.9f, -0.5f },
		};
		struct pc_vertex v[6];
		GLuint prog, vbo;
		GLint loc;
		int i;

		prog = link_two(gl_test_passthrough_vs_src, gl_test_uniform_fs_src,
				"a_color", NULL);
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

		prog = link_two(vs_d, fs_d, "a_color", NULL);
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

		prog = link_two(vs_t, fs_t, "a_texCoord", NULL);
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

		prog = link_two(vs_t, fs_t, "a_texCoord", NULL);
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
		/* case 24: two samplers declared, only u_tex1 sampled. The
		 * compiler drops binding 0 and compacts binding 1 to shared
		 * register 0, so the descriptor in slot 0 has to be the one
		 * bound to texture unit 1 -- green, not red. */
		static const uint8_t red[4]   = { 255, 0, 0, 255 };
		static const uint8_t green[4] = { 0, 255, 0, 255 };
		static const float c[6][4] = {
			{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
			{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
		};
		struct puv { float pos[4]; float uv[2]; } q[6];
		GLuint prog, t0 = 0, t1 = 0, vbo;
		int i;

		prog = link_two(gl_test_uv_vs_src, gl_test_compact_fs_src, "a_uv", NULL);
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
	const int textured = mode == 4 || mode == 5 || mode == 6 || mode == 10 || mode == 16;
	const int cube = mode == 11 || mode == 17;
	GLuint cube_tex = 0;   /* mode 17 attaches one of its faces to an FBO */
	const int vol = mode == 13;
	const int etc2 = mode == 16;
	struct pc_vertex verts[3];
	GLuint prog, vbo;
	GLint loc, ok = 0;
	int i;

	/* A mode this harness does not implement must not quietly draw mode 1's
	 * shape. Diffed against the golden's real mode it looks exactly like a
	 * driver bug and is not one: measured on 2026-09-22, modes 2 and 3 gave
	 * mode 1's stream in cs_ours while cs_golden gave their own, so the diff
	 * showed a vertex uniform we were "missing" and never write. Refuse
	 * instead, and say what there is. */
	{
		static const int have[] = { 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
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

	/* Exactly gl_test_run()'s bring-up for this GPU. */
	if (g_osal->init(NULL) != 0) {
		fprintf(stderr, "osal init failed\n");
		return 1;
	}
	if (yakogl_backend_init() != 0) {
		fprintf(stderr, "backend init failed\n");
		return 1;
	}
	if (!gl_context_create(TW, TH) || !gl_default_surface()) {
		fprintf(stderr, "context/surface creation failed\n");
		return 1;
	}

	if (mode >= 19) {
		return frag_mode(mode);
	}

	prog = glCreateProgram();
	glAttachShader(prog, compile(GL_VERTEX_SHADER,
				     mode == 7 ? gl_test_points_vs_src :
				     cube || vol ? gl_test_es300_cube_vs_src :
				     textured ? gl_test_uv_vs_src : gl_test_passthrough_vs_src));
	glAttachShader(prog, compile(GL_FRAGMENT_SHADER,
				     mode == 7 ? gl_test_interp_fs_src :
				     vol ? gl_test_es300_vol_fs_src :
				     cube ? gl_test_es300_cube_fs_src :
				     textured ? gl_test_texture_fs_src : gl_test_uniform_fs_src));
	glBindAttribLocation(prog, 0, "a_pos");
	glBindAttribLocation(prog, 1, textured || cube || vol ? "a_uv" : "a_color");
	if (mode == 7) {
		glBindAttribLocation(prog, 2, "a_size");
	}
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[512] = { 0 };

		glGetProgramInfoLog(prog, sizeof(log) - 1, NULL, log);
		fprintf(stderr, "link failed: %s\n", log);
		return 1;
	}
	glUseProgram(prog);

	for (i = 0; i < 3; i++) {
		static const float p[3][2] = {
			{ 0.0f, 0.75f }, { -0.75f, -0.75f }, { 0.75f, -0.75f }
		};

		verts[i].pos[0] = p[i][0];
		verts[i].pos[1] = p[i][1];
		verts[i].pos[2] = 0.0f;
		verts[i].pos[3] = 1.0f;
		verts[i].color[0] = verts[i].color[1] = 1.0f;
		verts[i].color[2] = verts[i].color[3] = 1.0f;
	}
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex),
			      (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex),
			      (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	if (mode == 7) {
		/* gl_test case 10's frame 5: three points of 1, 3 and 5 px,
		 * the size coming from an attribute. */
		struct pv { float pos[4]; float color[4]; float size; } pv[3];
		static const float sz[3] = { 1.0f, 3.0f, 5.0f };

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
	} else if (mode == 5 || mode == 6) {
		/* gl_test case 2's mip sub-test, exactly: a 4x4 texture whose
		 * levels are red, green and blue, and a quad two pixels wide
		 * so that two texels fall in each pixel and level 1 is the one
		 * the hardware should fetch. */
		static uint8_t red[4 * 4 * 4], green[2 * 2 * 4], blue[4];
		GLuint tex;
		int i;

		for (i = 0; i < 16; i++) { red[i * 4] = 255; red[i * 4 + 3] = 255; }
		for (i = 0; i < 4; i++) { green[i * 4 + 1] = 255; green[i * 4 + 3] = 255; }
		blue[2] = 255; blue[3] = 255;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		if (mode == 6) {
			/* gl_test's glGenerateMipmap base: left half red, right
			 * half blue, so level 2 must average to purple. */
			static uint8_t base[4 * 4 * 4];

			for (i = 0; i < 16; i++) {
				base[i * 4 + 0] = (i % 4) < 2 ? 255 : 0;
				base[i * 4 + 2] = (i % 4) < 2 ? 0 : 255;
				base[i * 4 + 3] = 255;
			}
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, base);
			glGenerateMipmap(GL_TEXTURE_2D);
		} else {
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, red);
			glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA, 2, 2, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, green);
			glTexImage2D(GL_TEXTURE_2D, 2, GL_RGBA, 1, 1, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, blue);
		}
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER,
				GL_NEAREST_MIPMAP_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);

		/* the 2x2-pixel quad at pixels 30..31, as gl_test draws it */
		{
			const float x0 = 30.0f, n = 2.0f;
			const float l = x0 / TW * 2 - 1, r = (x0 + n) / TW * 2 - 1;
			const float t = 1 - x0 / TH * 2, b = 1 - (x0 + n) / TH * 2;
			const float c[6][4] = {
				{ l, b, 0, 0 }, { r, b, 1, 0 }, { r, t, 1, 1 },
				{ l, b, 0, 0 }, { r, t, 1, 1 }, { l, t, 0, 1 },
			};
			struct puv_vertex { float pos[4]; float uv[2]; } q[6];

			for (i = 0; i < 6; i++) {
				q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1];
				q[i].pos[2] = 0.0f;    q[i].pos[3] = 1.0f;
				q[i].uv[0]  = c[i][2]; q[i].uv[1]  = c[i][3];
			}
			glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
			glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE,
					      sizeof(q[0]), (const void *)0);
			glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
					      sizeof(q[0]), (const void *)16);
		}
	} else if (vol) {
		/* gl_test case 48's volume: 4x4x5 RGBA8, each slice a flat colour.
		 * A 3D texture used to be laid out as a grid of slices here, which
		 * this hardware cannot address; it is stacked now, and the point of
		 * this mode is to read back what the descriptor claims -- TYPE 3,
		 * SIZE 4x4, SLICE = stride * height, 3D_CONFIG's depth 5 -- and
		 * check it against where the slices actually lie. */
		static uint8_t vox[4 * 4 * 5 * 4];
		GLuint tex;
		int k, i;

		for (k = 0; k < 5; k++)
			for (i = 0; i < 16; i++) {
				uint8_t *v = &vox[(k * 16 + i) * 4];

				v[0] = (uint8_t)(50 * k); v[1] = (uint8_t)(20 * k);
				v[2] = (uint8_t)(40 * k); v[3] = 255;
			}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_3D, tex);
		glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8, 4, 4, 5, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, vox);
		glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "vol"), 0);
		glUniform1f(glGetUniformLocation(prog, "zz"), 2.5f / 5.0f);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
				      sizeof(struct pc_vertex), (const void *)16);
	} else if (cube) {
		/* gl_test case 41's cube: six 8x8 faces, one flat colour each.
		 * What the descriptor says about the stack -- TYPE, SIZE,
		 * SLICE, 3D_CONFIG -- is what this mode is for. */
		static const uint8_t face_rgb[6][3] = {
			{ 255, 0, 0 }, { 0, 255, 0 }, { 0, 0, 255 },
			{ 255, 255, 0 }, { 255, 0, 255 }, { 0, 255, 255 },
		};
		static uint8_t texels[8 * 8 * 4];
		GLuint tex;
		int f, k;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
		for (f = 0; f < 6; f++) {
			for (k = 0; k < 8 * 8; k++) {
				texels[k * 4 + 0] = face_rgb[f][0];
				texels[k * 4 + 1] = face_rgb[f][1];
				texels[k * 4 + 2] = face_rgb[f][2];
				texels[k * 4 + 3] = 255;
			}
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA,
				     8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
		}
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		cube_tex = tex;
		glUniform1i(glGetUniformLocation(prog, "cube"), 0);
		glUniform3f(glGetUniformLocation(prog, "axis"), 0.0f, 0.0f, 1.0f);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
				      sizeof(struct pc_vertex), (const void *)16);
	} else if (etc2) {
		/* gl_test case 20's second texture: 8x8 ETC2 RGBA8 EAC, four
		 * 16-byte blocks, alpha 255/128/64/0 over red/green/blue/white.
		 * The EAC half is the first eight bytes (base alpha, multiplier 0,
		 * table 0, all indices 0) and the colour half the second eight, a
		 * flat ETC1 block. Checks 12-15 of that case are these four, and
		 * unlike the ETC1 texture this descriptor has never been diffed. */
		static const uint8_t alphas[4] = { 255, 128, 64, 0 };
		static const uint8_t cols[4][3] = {
			{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 },
		};
		static uint8_t etc2a[4 * 16];
		GLuint tex;
		int i;

		for (i = 0; i < 4; i++) {
			uint8_t *blk = &etc2a[i * 16];

			memset(blk, 0, 16);
			blk[0] = alphas[i];
			blk[8]  = (uint8_t)((cols[i][0] << 4) | cols[i][0]);
			blk[9]  = (uint8_t)((cols[i][1] << 4) | cols[i][1]);
			blk[10] = (uint8_t)((cols[i][2] << 4) | cols[i][2]);
		}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x9278 /* RGBA8_ETC2_EAC */,
				       8, 8, 0, sizeof(etc2a), etc2a);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
				      sizeof(struct pc_vertex), (const void *)16);
	} else if (mode == 10) {
		/* gl_test case 20's ETC1 texture: 16x8, eight flat 4x4 blocks.
		 * Where each block lands in memory, and what LINEAR_STRIDE
		 * says, is the whole question. */
		static const uint8_t cols[8][3] = {
			{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 },
			{ 15, 15, 0 }, { 0, 15, 15 }, { 15, 0, 15 }, { 8, 8, 8 },
		};
		static uint8_t etc1[8 * 8];
		GLuint tex;
		int b;

		for (b = 0; b < 8; b++) {
			uint8_t *blk = &etc1[b * 8];

			blk[0] = (uint8_t)((cols[b][0] << 4) | cols[b][0]);
			blk[1] = (uint8_t)((cols[b][1] << 4) | cols[b][1]);
			blk[2] = (uint8_t)((cols[b][2] << 4) | cols[b][2]);
			blk[3] = blk[4] = blk[5] = blk[6] = blk[7] = 0;
		}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x8D64 /* GL_ETC1_RGB8_OES */,
				       16, 8, 0, sizeof(etc1), etc1);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
				      sizeof(struct pc_vertex), (const void *)16);
	} else if (mode == 4) {
		/* gl_test's quadrant texture, at the size case 2 uses. */
		static uint8_t texels[16 * 16 * 4];
		GLuint tex;
		int tx, ty;

		for (ty = 0; ty < 16; ty++) {
			for (tx = 0; tx < 16; tx++) {
				uint8_t *q = &texels[(ty * 16 + tx) * 4];
				int right = tx >= 8, top = ty >= 8;

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
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE,
				      sizeof(struct pc_vertex), (const void *)16);
	} else {
		loc = glGetUniformLocation(prog, "u_color");
		if (loc < 0) {
			fprintf(stderr, "u_color not found\n");
			return 1;
		}
		glUniform4f(loc, 1.0f, 0.5f, 0.0f, 1.0f);
	}

	if (mode == 8 || mode == 12 || mode == 14 || mode == 15 || mode == 18) {
		glEnable(GL_DEPTH_TEST);
		glDepthFunc(GL_LESS);
	}
	if (mode == 9 || mode == 10 || mode == 11) {   /* not 17: it uses a cube-face FBO below */
		/* render into a texture through an FBO: case 6's target, and
		 * the one case 4's check 5 uses. An FBO is not y-flipped, so
		 * everything the flip touches differs here. */
		GLuint fbo = 0, tex = 0;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, NULL);
		glGenFramebuffers(1, &fbo);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
				       GL_TEXTURE_2D, tex, 0);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
			fprintf(stderr, "fbo incomplete\n");
	}
	if (mode == 11) {
		/* case 4's check 5: an FBO target with CCW fronts and back-face
		 * culling, where nothing should be culled. */
		glEnable(GL_CULL_FACE);
		glFrontFace(GL_CCW);
		glCullFace(GL_BACK);
	}
	glViewport(0, 0, TW, TH);
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
		glDrawArrays(GL_TRIANGLES, 0, textured && mode != 4 && mode != 10 && mode != 16 ? 6 : 3);
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


	if (mode == 10) {
		/* back to the window, and sample what was just rendered: this
		 * is case 6's second half, and the descriptor's format has to
		 * come back A8R8G8B8 rather than A8B8G8R8. */
		GLuint prog2 = glCreateProgram();
		struct puv { float pos[4]; float uv[2]; } q[6];
		static const float c[6][4] = {
			{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
			{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
		};

		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glAttachShader(prog2, compile(GL_VERTEX_SHADER, gl_test_uv_vs_src));
		glAttachShader(prog2, compile(GL_FRAGMENT_SHADER, gl_test_texture_fs_src));
		glBindAttribLocation(prog2, 0, "a_pos");
		glBindAttribLocation(prog2, 1, "a_uv");
		glLinkProgram(prog2);
		glUseProgram(prog2);
		glUniform1i(glGetUniformLocation(prog2, "u_tex"), 0);
		for (i = 0; i < 6; i++) {
			q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1];
			q[i].pos[2] = 0.0f;    q[i].pos[3] = 1.0f;
			q[i].uv[0]  = c[i][2]; q[i].uv[1]  = c[i][3];
		}
		glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(q[0]), (const void *)16);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
	}

	if (mode == 17) {
		/* gl_test case 41's second half, which mode 11 cannot reach:
		 * render INTO one cube face through an FBO, then sample the
		 * cube again. What matters is the state BETWEEN the two kicks
		 * -- what happens when a face that was just a render target
		 * becomes a texture -- so this is one submit per frame, the
		 * shape mode 14 uses for case 3.
		 *
		 * Case 42 does the same with a 2D array layer, so whatever
		 * this settles about a layered render target carries there. */
		GLuint cfbo = 0;

		glGenFramebuffers(1, &cfbo);
		glBindFramebuffer(GL_FRAMEBUFFER, cfbo);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
				       GL_TEXTURE_CUBE_MAP_POSITIVE_Y, cube_tex, 0);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			fprintf(stderr, "cs_ours: mode 17: cube-face FBO incomplete\n");
			return 3;
		}
		/* frame 1: clear the +Y face white, no draw */
		glViewport(0, 0, 8, 8);
		glClearColor(1.0f, 1.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glFinish();

		/* frame 2: back to the window, sample +Y, which should now read
		 * white rather than the colour it was uploaded with */
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
	}

	fprintf(stderr, "drew; gl error 0x%x\n", glGetError());
	return 0;
}
