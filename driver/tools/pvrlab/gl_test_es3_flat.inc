// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- ES 3.0 flat shading beyond case 89: lines, loops, wide lines, strips ----
 * Case bodies, not yet wired in: gl_test.c includes this file after
 * gl_test_instmix.inc and gives each function a case number. Everything here
 * is static and uses gl_test.c's helpers (link_program, read_topdown,
 * expect_px, check_gl_error, upload_vbo, free_all_objects, TW, TH).
 *
 * dEQP-GLES3.functional.rasterization.flatshading.* (docs/notes/es3-flatshading.md):
 * every vertex gets its own flat colour and each primitive must take the
 * colour of its PROVOKING vertex, the last vertex in ES 3.0 2.18:
 *
 *   GL_LINES          line i                 -> its second vertex (2i+1, 0-based)
 *   GL_LINE_STRIP     segment i              -> vertex i+1
 *   GL_LINE_LOOP      segment i < n-1        -> vertex i+1
 *                     closing segment (n-1,0)-> vertex 0
 *   GL_TRIANGLE_STRIP triangle i             -> vertex i+2
 *   GL_TRIANGLE_FAN   triangle i (hub 0)     -> vertex i+2, never the hub
 *
 * The first-vertex convention (D3D's) gives the first vertex of each pair,
 * the previous vertex's colour on the strips, and the hub on the fan; a
 * closing loop segment that took vertex n-1 shows the colour of the segment
 * before it.
 *
 * Window is 64x64, vertices are given in window pixels (GL, y up) and the
 * vertex shader maps them to NDC. Lines are axis-aligned and run through pixel
 * centres (x.5, y.5), so no diamond-exit or corner rule decides a sample.
 * The eight vertex colours (index -> R G B, alpha 255, background 0,0,0,255):
 *   0 255   0   0   1   0 255   0   2   0   0 255   3 255 255   0
 *   4   0 255 255   5 255   0 255   6 255 255 255   7 255 128   0
 * Sample pixels are given top-down (row = 63 - y). In every case a sample is
 * the centre of a segment (or of a triangle's centroid) and, on the wide forms,
 * the pixel on each side of it across the line.
 *
 * lines (flat_lines): 3 lines at y = 8.5 / 28.5 / 48.5, x 4.5 -> 59.5,
 *   colours (0,1) (2,3) (4,5): expected centre (32,55) = 1, (32,35) = 3,
 *   (32,15) = 5, and the same colour at x = 8 and x = 56 of each row.
 * line_strip (flat_line_strip): vertices 0 (8.5,8.5) 1 (56.5,8.5) 2 (56.5,56.5)
 *   3 (8.5,56.5), colours 0..3: bottom (32,55) = 1, right (56,32) = 2,
 *   top (32,7) = 3.
 * line_loop (flat_line_loop): the same four vertices, closed: bottom = 1,
 *   right = 2, top = 3 and the closing left side (8,32) = 0 (not 3).
 * *_wide: the same shapes at glLineWidth(5), or the largest width
 *   GL_ALIASED_LINE_WIDTH_RANGE allows when that is below 5 (a range whose
 *   maximum is 1 reports no wide line: the case then runs at width 1 and says
 *   so). The same expected colours at the same centres, and at one pixel
 *   either side across the line when the width is 3 or more.
 * triangle_strip (flat_tri_strip): 8 vertices, 6 triangles, vertex k at
 *   x = 4.5 + 8k, y = 4.5 (k even) / 59.5 (k odd): triangle i is the colour of
 *   vertex i+2, sampled at its centroid.
 * triangle_fan (flat_tri_fan): hub (32,32) and 7 rim points on a circle of
 *   radius 26 at 0, 50 ... 300 degrees, 6 triangles: triangle i is the colour
 *   of vertex i+2 (rim point i+1), sampled at its centroid.
 *
 * A case returns 0 on a pass, -1 on a mismatch or GL error. A draw the GL
 * refuses is a failure, not a skip. */
static const char es3flat_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec2 a_win;\n"
	"layout(location = 1) in vec3 a_col;\n"
	"flat out vec3 v_col;\n"
	"void main()\n"
	"{\n"
	"    gl_Position = vec4(a_win / 32.0 - 1.0, 0.0, 1.0);\n"
	"    v_col = a_col;\n"
	"}\n";

static const char es3flat_fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"flat in vec3 v_col;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    fragColor = vec4(v_col, 1.0);\n"
	"}\n";

static const uint8_t es3flat_pal[8][3] = {
	{ 255, 0, 0 }, { 0, 255, 0 }, { 0, 0, 255 }, { 255, 255, 0 },
	{ 0, 255, 255 }, { 255, 0, 255 }, { 255, 255, 255 }, { 255, 128, 0 },
};

struct es3flat_vtx { float x, y; uint8_t col; };

/* One flat-shaded draw of `n` vertices as `mode` at line width `width`, then
 * read_topdown(). Returns false on a GL error. */
static bool es3flat_draw(const char *name, GLenum mode, const struct es3flat_vtx *v, int n, float width)
{
	float data[8 * 5];
	GLuint prog, vbo;

	free_all_objects();
	prog = link_program(es3flat_vs_src, es3flat_fs_src);
	if (!prog)
		return false;
	for (int i = 0; i < n; i++) {
		data[i * 5 + 0] = v[i].x;
		data[i * 5 + 1] = v[i].y;
		for (int c = 0; c < 3; c++)
			data[i * 5 + 2 + c] = (float)es3flat_pal[v[i].col][c] / 255.0f;
	}
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_CULL_FACE);
	glDisable(GL_SCISSOR_TEST);
	vbo = upload_vbo(data, (size_t)n * 5 * sizeof(float));
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 5 * sizeof(float), (const void *)0);
	glVertexAttribPointer(1, 3, GL_FLOAT, GL_FALSE, 5 * sizeof(float), (const void *)(2 * sizeof(float)));
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glLineWidth(width);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(mode, 0, n);
	glFinish();
	glDisableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vbo);
	glLineWidth(1.0f);
	if (!check_gl_error(name))
		return false;
	read_topdown();
	return true;
}

/* The pixel at window (x, y) (y up) must be palette colour `col`. */
static bool es3flat_expect(const char *name, const char *what, int x, int y, uint8_t col)
{
	char label[48];
	uint32_t want = rgba_u32(es3flat_pal[col][0], es3flat_pal[col][1], es3flat_pal[col][2], 255);

	snprintf(label, sizeof(label), "%s: %s", name, what);
	return expect_px(label, (uint32_t)x, (uint32_t)(TH - 1 - (uint32_t)y), want, 2);
}

/* The width a wide-line case draws at: 5, clamped to the reported range. */
static float es3flat_wide_width(void)
{
	GLfloat range[2] = { 1.0f, 1.0f };
	float w = 5.0f;

	glGetFloatv(GL_ALIASED_LINE_WIDTH_RANGE, range);
	if (w > range[1])
		w = range[1];
	if (w < range[0])
		w = range[0];
	return w;
}

/* Samples across a segment: its centre and, on a wide line, one pixel on each
 * side. horizontal: the segment runs along x at row y, the offsets move in y. */
static bool es3flat_seg(const char *name, const char *what, int x, int y, bool horizontal, float width, uint8_t col)
{
	bool ok = es3flat_expect(name, what, x, y, col);

	if (width >= 3.0f) {
		ok &= es3flat_expect(name, what, horizontal ? x : x - 1, horizontal ? y - 1 : y, col);
		ok &= es3flat_expect(name, what, horizontal ? x : x + 1, horizontal ? y + 1 : y, col);
	}
	return ok;
}

static int es3flat_lines(const char *name, float width)
{
	static const struct es3flat_vtx v[6] = {
		{ 4.5f, 8.5f, 0 }, { 59.5f, 8.5f, 1 },
		{ 4.5f, 28.5f, 2 }, { 59.5f, 28.5f, 3 },
		{ 4.5f, 48.5f, 4 }, { 59.5f, 48.5f, 5 },
	};
	static const int rows[3] = { 8, 28, 48 };
	bool ok = true;

	if (!es3flat_draw(name, GL_LINES, v, 6, width))
		return -1;
	for (int i = 0; i < 3; i++) {
		const uint8_t col = v[2 * i + 1].col;

		ok &= es3flat_seg(name, "line, near its first vertex", 8, rows[i], true, width, col);
		ok &= es3flat_seg(name, "line, middle", 32, rows[i], true, width, col);
		ok &= es3flat_seg(name, "line, near its last vertex", 56, rows[i], true, width, col);
	}
	return ok ? 0 : -1;
}

static int es3flat_square(const char *name, GLenum mode, float width)
{
	static const struct es3flat_vtx v[4] = {
		{ 8.5f, 8.5f, 0 }, { 56.5f, 8.5f, 1 }, { 56.5f, 56.5f, 2 }, { 8.5f, 56.5f, 3 },
	};
	bool ok;

	if (!es3flat_draw(name, mode, v, 4, width))
		return -1;
	ok = es3flat_seg(name, "bottom segment = vertex 1", 32, 8, true, width, 1);
	ok &= es3flat_seg(name, "right segment = vertex 2", 56, 32, false, width, 2);
	ok &= es3flat_seg(name, "top segment = vertex 3", 32, 56, true, width, 3);
	if (mode == GL_LINE_LOOP)
		ok &= es3flat_seg(name, "closing segment = vertex 0", 8, 32, false, width, 0);
	else
		/* The strip leaves the left side undrawn: it stays the clear colour. */
		ok &= expect_px("line_strip: no closing segment", 8, TH - 1 - 32, 0xff000000u, 0);
	return ok ? 0 : -1;
}

/* ---- flat_lines: GL_LINES, width 1 ------------------------------------- */
static int case_es3flat_lines(void) { return es3flat_lines("flat lines", 1.0f); }

/* ---- flat_lines_wide: GL_LINES, width 5 (clamped to the range) ---------- */
static int case_es3flat_lines_wide(void) { return es3flat_lines("flat lines_wide", es3flat_wide_width()); }

/* ---- flat_line_strip: GL_LINE_STRIP, width 1 --------------------------- */
static int case_es3flat_line_strip(void) { return es3flat_square("flat line_strip", GL_LINE_STRIP, 1.0f); }

/* ---- flat_line_strip_wide: GL_LINE_STRIP, width 5 ---------------------- */
static int case_es3flat_line_strip_wide(void)
{
	return es3flat_square("flat line_strip_wide", GL_LINE_STRIP, es3flat_wide_width());
}

/* ---- flat_line_loop: GL_LINE_LOOP, width 1, closing segment = vertex 0 --- */
static int case_es3flat_line_loop(void) { return es3flat_square("flat line_loop", GL_LINE_LOOP, 1.0f); }

/* ---- flat_line_loop_wide: GL_LINE_LOOP, width 5 ------------------------ */
static int case_es3flat_line_loop_wide(void)
{
	return es3flat_square("flat line_loop_wide", GL_LINE_LOOP, es3flat_wide_width());
}

/* Every triangle i of `v` (n vertices, mode strip or fan) must read the colour
 * of vertex i+2 at its centroid. */
static int es3flat_tris(const char *name, GLenum mode, const struct es3flat_vtx *v, int n)
{
	bool ok = true;

	if (!es3flat_draw(name, mode, v, n, 1.0f))
		return -1;
	for (int i = 0; i + 2 < n; i++) {
		const struct es3flat_vtx *a = mode == GL_TRIANGLE_FAN ? &v[0] : &v[i];
		const struct es3flat_vtx *b = &v[i + 1];
		const struct es3flat_vtx *c = &v[i + 2];
		char what[48];

		snprintf(what, sizeof(what), "triangle %d = vertex %d", i, i + 2);
		ok &= es3flat_expect(name, what, (int)((a->x + b->x + c->x) / 3.0f),
				     (int)((a->y + b->y + c->y) / 3.0f), c->col);
	}
	return ok ? 0 : -1;
}

/* ---- flat_tri_strip: GL_TRIANGLE_STRIP, 6 triangles ---------------------- */
static int case_es3flat_tri_strip(void)
{
	struct es3flat_vtx v[8];

	for (int k = 0; k < 8; k++) {
		v[k].x = 4.5f + 8.0f * (float)k;
		v[k].y = (k & 1) ? 59.5f : 4.5f;
		v[k].col = (uint8_t)k;
	}
	return es3flat_tris("flat triangle_strip", GL_TRIANGLE_STRIP, v, 8);
}

/* ---- flat_tri_fan: GL_TRIANGLE_FAN, hub + 7 rim points, 6 triangles ------ */
static int case_es3flat_tri_fan(void)
{
	static const struct es3flat_vtx v[8] = {
		{ 32.0f, 32.0f, 0 },
		{ 58.0f, 32.0f, 1 }, { 48.7f, 51.9f, 2 }, { 27.5f, 57.6f, 3 }, { 9.5f, 45.0f, 4 },
		{ 7.6f, 23.1f, 5 }, { 23.1f, 7.6f, 6 }, { 45.0f, 9.5f, 7 },
	};

	return es3flat_tris("flat triangle_fan", GL_TRIANGLE_FAN, v, 8);
}
