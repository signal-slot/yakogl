// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- case 243: instanced draw, divisors 2, 3 and 4 together (divmix) ---------
 * The case number is PROVISIONAL: it is the highest case number of gl_test.c on
 * origin/main (242) plus one, taken when this file was written. Item gltest-wire
 * takes the number that is free then. Not wired in, built and not run.
 *
 * Wiring (gl_test.c, item gltest-wire): after the last include of the .inc files
 * (`#include "gl_test_es3_precision.inc"` on origin/main when this was written)
 *
 *   #include "gl_test_es3_divmix.inc"
 *
 * and in the dispatch switch (next to `case 221: rc = case_instmix(); break;`)
 *
 *   case 243: rc = case_divmix(); break;
 *
 * and one line for it in the case list of gl_test.c's header comment. The file
 * uses that file's static helpers (link_program, read_topdown, expect_px,
 * check_gl_error, free_all_objects, TW, TH) and the online shader path like
 * instmix (gl_test_es300_vcolor_fs_src); no gl_test_shaders.h change.
 *
 * P3 of docs/notes/instanceid-per-draw.md (PowerVR hardware instance divisors):
 * one per-vertex position (divisor 0) and three per-instance float attributes
 * with divisors 2, 3 and 4, 12 instances, as glDrawArraysInstanced and as
 * glDrawElementsInstanced. Instance i is drawn in cell (i & 3, i >> 2) of a
 * 4 x 3 grid over the 64x64 target (16 x 21.3 pixels a cell), placed from
 * gl_InstanceID, and coloured from the three sources, one channel each:
 *
 *   r = divisor-2 attribute  {0, 51, 102, 153, 204, 255}/255   [i / 2]
 *   g = divisor-3 attribute  {0, 85, 170, 255}/255             [i / 3]
 *   b = divisor-4 attribute  {40, 120, 200}/255                [i / 4]
 *
 * Every element differs from its neighbours, so a wrong divisor on any stream
 * gives a wrong channel in some cell. Expected, a pass per instance and per
 * draw kind (the same table for both). Pixel (x,y) top-down in the 64x64
 * target, the centre of the cell; colour 8-bit R G B A, tolerance 3 as instmix:
 *
 *   instance  pixel     R   G   B   A    0xAABBGGRR
 *    0        ( 8,53)    0   0  40 255    0xff280000
 *    1        (24,53)    0   0  40 255    0xff280000
 *    2        (40,53)   51   0  40 255    0xff280033
 *    3        (56,53)   51  85  40 255    0xff285533
 *    4        ( 8,31)  102  85 120 255    0xff785566
 *    5        (24,31)  102  85 120 255    0xff785566
 *    6        (40,31)  153 170 120 255    0xff78aa99
 *    7        (56,31)  153 170 120 255    0xff78aa99
 *    8        ( 8,10)  204 170 200 255    0xffc8aacc
 *    9        (24,10)  204 255 200 255    0xffc8ffcc
 *   10        (40,10)  255 255 200 255    0xffc8ffff
 *   11        (56,10)  255 255 200 255    0xffc8ffff
 *
 * Output: one `gl_test:   <kind>: instance N (x,y) = 0x... want 0x... ok` line
 * per instance and draw kind (24 lines, kind `arrays` then `elements`), then
 * `divmix: 24 of 24 instances ok` (or `divmix: N of 24 instances ok`); the case
 * returns 0 only if all 24 pass and no GL error was raised.
 *
 * A back end that takes divisor 1 only (the PowerVR before P3; the GL core
 * splits the draw per instance): each of the 12 draws has gl_InstanceID 0, so
 * every instance lands in cell 0, pixel (8,53), the later over the earlier;
 * that pixel ends as instance 11's colour 255 255 200 255 (0xffc8ffff) and the
 * other 11 cells stay at the clear colour 0 0 0 255 (0xff000000). All 24 checks
 * fail (none of the expected values above is the clear colour; instance 0's
 * (8,53) shows instance 11's colour), for arrays and for elements. A back end
 * with a wrong multiplier/shift places every cell but gives a wrong channel in
 * some: a divisor 3 stream stepping like divisor 2 gives g wrong from
 * instance 3 on, a divisor 4 stream stepping like divisor 2 reads b past its
 * three elements from instance 6 on. */
static const char divmix_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec4 pos;\n"
	"layout(location = 1) in float a_d2;\n"
	"layout(location = 2) in float a_d3;\n"
	"layout(location = 3) in float a_d4;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    float id = float(gl_InstanceID);\n"
	"    float col = mod(id, 4.0);\n"
	"    float row = floor(id * 0.25);\n"
	"    vec2 cell = vec2(col * 0.5 - 0.75, row * (2.0 / 3.0) - (2.0 / 3.0));\n"
	"    gl_Position = vec4(pos.xy * vec2(0.25, 1.0 / 3.0) + cell, 0.0, 1.0);\n"
	"    v_col = vec4(a_d2, a_d3, a_d4, 1.0);\n"
	"}\n";

static int case_divmix(void)
{
	static const float quad6[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	static const float quad4[4 * 4] = { -1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1 };
	static const uint16_t idx[6] = { 0, 1, 2, 0, 2, 3 };
	static const float d2[6] = { 0.0f, 51.0f / 255, 102.0f / 255, 153.0f / 255,
				     204.0f / 255, 1.0f };
	static const float d3[4] = { 0.0f, 85.0f / 255, 170.0f / 255, 1.0f };
	static const float d4[3] = { 40.0f / 255, 120.0f / 255, 200.0f / 255 };
	static const struct { const char *name; uint32_t x, y; uint32_t want; } inst[12] = {
		{ "instance 0", 8, 53, 0xff280000u },
		{ "instance 1", 24, 53, 0xff280000u },
		{ "instance 2", 40, 53, 0xff280033u },
		{ "instance 3", 56, 53, 0xff285533u },
		{ "instance 4", 8, 31, 0xff785566u },
		{ "instance 5", 24, 31, 0xff785566u },
		{ "instance 6", 40, 31, 0xff78aa99u },
		{ "instance 7", 56, 31, 0xff78aa99u },
		{ "instance 8", 8, 10, 0xffc8aaccu },
		{ "instance 9", 24, 10, 0xffc8ffccu },
		{ "instance 10", 40, 10, 0xffc8ffffu },
		{ "instance 11", 56, 10, 0xffc8ffffu },
	};
	GLuint prog, vb6 = 0, vb4 = 0, ib = 0, b2 = 0, b3 = 0, b4 = 0;
	bool ok = true;
	int passed = 0;

	free_all_objects();
	prog = link_program(divmix_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb6);
	glBindBuffer(GL_ARRAY_BUFFER, vb6);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad6), quad6, GL_STATIC_DRAW);
	glGenBuffers(1, &vb4);
	glBindBuffer(GL_ARRAY_BUFFER, vb4);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad4), quad4, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &b2);
	glBindBuffer(GL_ARRAY_BUFFER, b2);
	glBufferData(GL_ARRAY_BUFFER, sizeof(d2), d2, GL_STATIC_DRAW);
	glGenBuffers(1, &b3);
	glBindBuffer(GL_ARRAY_BUFFER, b3);
	glBufferData(GL_ARRAY_BUFFER, sizeof(d3), d3, GL_STATIC_DRAW);
	glGenBuffers(1, &b4);
	glBindBuffer(GL_ARRAY_BUFFER, b4);
	glBufferData(GL_ARRAY_BUFFER, sizeof(d4), d4, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	for (int pass = 0; pass < 2; pass++) {
		const char *kind = pass == 0 ? "arrays" : "elements";
		char what[40];

		glBindBuffer(GL_ARRAY_BUFFER, pass == 0 ? vb6 : vb4);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
		glEnableVertexAttribArray(0);
		glBindBuffer(GL_ARRAY_BUFFER, b2);
		glVertexAttribPointer(1, 1, GL_FLOAT, GL_FALSE, 4, (void *)0);
		glEnableVertexAttribArray(1);
		glVertexAttribDivisor(1, 2);
		glBindBuffer(GL_ARRAY_BUFFER, b3);
		glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, 4, (void *)0);
		glEnableVertexAttribArray(2);
		glVertexAttribDivisor(2, 3);
		glBindBuffer(GL_ARRAY_BUFFER, b4);
		glVertexAttribPointer(3, 1, GL_FLOAT, GL_FALSE, 4, (void *)0);
		glEnableVertexAttribArray(3);
		glVertexAttribDivisor(3, 4);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 12);
		else
			glDrawElementsInstanced(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (void *)0, 12);
		glFinish();
		snprintf(what, sizeof(what), "divmix: %s, divisors 2, 3, 4", kind);
		if (!check_gl_error(what))
			ok = false;
		read_topdown();
		for (int i = 0; i < 12; i++) {
			bool p;

			snprintf(what, sizeof(what), "%s: %s", kind, inst[i].name);
			p = expect_px(what, inst[i].x, inst[i].y, inst[i].want, 3);
			passed += p;
			ok &= p;
		}
	}
	pr_info("gl_test: divmix: %d of 24 instances ok\n", passed);
	for (int a = 1; a <= 3; a++) {
		glVertexAttribDivisor(a, 0);
		glDisableVertexAttribArray(a);
	}
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb6);
	glDeleteBuffers(1, &vb4);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &b2);
	glDeleteBuffers(1, &b3);
	glDeleteBuffers(1, &b4);
	return ok && check_gl_error("divmix teardown") ? 0 : -1;
}
