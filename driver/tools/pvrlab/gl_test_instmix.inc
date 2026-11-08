/* ---- case 221: a mixed-divisor instanced draw with gl_InstanceID (instmix) --
 * Included by gl_test.c after case 220 (it uses that file's static helpers and
 * is built wherever gl_test.c is, with no build-file change).
 *
 * dEQP's instanced.*.mixed shape (es3fInstancedRenderingTests.cpp): one
 * per-vertex position (divisor 0), one attribute with divisor 1 and one with
 * divisor 3, 4 instances, as glDrawArraysInstanced and as
 * glDrawElementsInstanced. The vertex shader puts instance gl_InstanceID in
 * quadrant (id & 1, id >> 1), so each instance covers its own 32x32 region,
 * and colours it from the three sources together:
 *
 *   r = divisor-1 attribute  {1.00, 0.50, 0.25, 0.75}[instance]
 *   g = divisor-3 attribute  {0.25, 1.00}[instance / 3]  (0,1,2 -> 0; 3 -> 1)
 *   b = gl_InstanceID * 0.25
 *
 * Expected (top-down pixel, R G B A), a pass per instance and per draw kind:
 *   instance 0 (16,48) 255  64   0 255      instance 1 (48,48) 128  64  64 255
 *   instance 2 (16,16)  64  64 128 255      instance 3 (48,16) 191 255 191 255
 * Passes where the back end takes both divisors (the V3D; the GC7000UL once
 * its stream divisor, V1 of docs/notes/instanceid-per-draw.md, is on). On a
 * back end that takes divisor 1 only, the GL core splits the draw per instance:
 * gl_InstanceID reads 0 in every draw and the draw is refused (GC V0) or
 * drawn wrong (instance 0's colour, one region, the others clear). */
static const char instmix_vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec4 pos;\n"
	"layout(location = 1) in float a_d1;\n"
	"layout(location = 2) in float a_d3;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    float id = float(gl_InstanceID);\n"
	"    vec2 cell = vec2(mod(id, 2.0) - 0.5, floor(id * 0.5) - 0.5);\n"
	"    gl_Position = vec4(pos.xy * 0.5 + cell, 0.0, 1.0);\n"
	"    v_col = vec4(a_d1, a_d3, id * 0.25, 1.0);\n"
	"}\n";

static int case_instmix(void)
{
	static const float quad6[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	static const float quad4[4 * 4] = { -1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1 };
	static const uint16_t idx[6] = { 0, 1, 2, 0, 2, 3 };
	static const float d1[4] = { 1.0f, 0.5f, 0.25f, 0.75f };
	static const float d3[2] = { 0.25f, 1.0f };
	static const struct { const char *name; uint32_t x, y; uint32_t want; } inst[4] = {
		{ "instance 0", 16, 48, 0xff0040ffu },
		{ "instance 1", 48, 48, 0xff404080u },
		{ "instance 2", 16, 16, 0xff804040u },
		{ "instance 3", 48, 16, 0xffbfffbfu },
	};
	GLuint prog, vb6 = 0, vb4 = 0, ib = 0, b1 = 0, b3 = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(instmix_vs_src, gl_test_es300_vcolor_fs_src);
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
	glGenBuffers(1, &b1);
	glBindBuffer(GL_ARRAY_BUFFER, b1);
	glBufferData(GL_ARRAY_BUFFER, sizeof(d1), d1, GL_STATIC_DRAW);
	glGenBuffers(1, &b3);
	glBindBuffer(GL_ARRAY_BUFFER, b3);
	glBufferData(GL_ARRAY_BUFFER, sizeof(d3), d3, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	for (int pass = 0; pass < 2; pass++) {
		const char *kind = pass == 0 ? "arrays" : "elements";
		char what[40];

		glBindBuffer(GL_ARRAY_BUFFER, pass == 0 ? vb6 : vb4);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
		glEnableVertexAttribArray(0);
		glBindBuffer(GL_ARRAY_BUFFER, b1);
		glVertexAttribPointer(1, 1, GL_FLOAT, GL_FALSE, 4, (void *)0);
		glEnableVertexAttribArray(1);
		glVertexAttribDivisor(1, 1);
		glBindBuffer(GL_ARRAY_BUFFER, b3);
		glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, 4, (void *)0);
		glEnableVertexAttribArray(2);
		glVertexAttribDivisor(2, 3);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 4);
		else
			glDrawElementsInstanced(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (void *)0, 4);
		glFinish();
		snprintf(what, sizeof(what), "instmix: %s, divisors 1 and 3", kind);
		if (!check_gl_error(what))
			ok = false;
		read_topdown();
		for (int i = 0; i < 4; i++) {
			snprintf(what, sizeof(what), "%s: %s", kind, inst[i].name);
			ok &= expect_px(what, inst[i].x, inst[i].y, inst[i].want, 3);
		}
	}
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb6);
	glDeleteBuffers(1, &vb4);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &b1);
	glDeleteBuffers(1, &b3);
	return ok && check_gl_error("instmix teardown") ? 0 : -1;
}
