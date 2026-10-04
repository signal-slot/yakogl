/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * slint_gl_rec: what the Slint sample's frames ask of a GL backend, measured on
 * the host. The one Slint sample's Rust archive (driver/samples/zephyr/slint/rust,
 * built for x86_64 from the same crate, UI and no_std Slint/femtovg trees as
 * the board images) is driven exactly as its main.c drives it (the sample's
 * own entry points, slint_sample_first_frame and slint_sample_animate_frame),
 * but its GL entry points come from this file: a
 * recorder that keeps the GL state femtovg sets and writes one line per
 * draw. Nothing is rendered and no shader is compiled (every compile and
 * link reports success), so this measures the call stream, which is what
 * the V3D backend will receive, not pixels.
 *
 * Per frame it prints: draws, vertices, the program (femtovg's
 * shader type, glyph texture, discard), the draws that sample a texture and
 * the textures they sample, the draws that test or write the stencil, the
 * clears, the render-target switches (glBindFramebuffer), the texture and
 * buffer uploads with their sizes, and every point where the V3D backend
 * must end a job (a target switch, glFlush/glFinish, glReadPixels) with the
 * stencil state across it.
 *
 * Frames: the sample's first frame (slint_sample_first_frame: the component
 * build and its default state at Slint clock 0, then demo_vehicle at t = 0 at
 * FRAME2_CLOCK_MS; the two are told apart by the crate's own "slint: frame 1"
 * line), then the sample's animation (slint_sample_animate_frame, the crate's
 * demo_vehicle) at 58 fps (the AM62P's rate) through the idle, the
 * acceleration and the cruise.
 *
 * Build (the archive as in docs/online-shaderc/v3d-backend.md, "M7"):
 *   cargo build --release -Z build-std=core,alloc --target x86_64-unknown-linux-gnu \
 *       --manifest-path <slint build>/rust/Cargo.toml   (with SLINT_UI_FILE, SAFEUI_LAYOUT_DIR)
 *   gcc -O1 -g -Wall -Wextra -o slint_gl_rec slint_gl_rec.c <that libslint_sample.a> -lm
 *   ./slint_gl_rec [draws.log]
 */
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ---- GL types and the enums used here (Khronos values) -------------------- */
typedef unsigned int GLenum, GLuint, GLbitfield;
typedef int GLint, GLsizei;
typedef unsigned char GLboolean, GLubyte;
typedef float GLfloat;
typedef intptr_t GLintptr, GLsizeiptr;
typedef char GLchar;

#define GL_NO_ERROR 0
#define GL_TRIANGLES 0x0004
#define GL_TRIANGLE_STRIP 0x0005
#define GL_CULL_FACE 0x0B44
#define GL_DEPTH_TEST 0x0B71
#define GL_STENCIL_TEST 0x0B90
#define GL_BLEND 0x0BE2
#define GL_SCISSOR_TEST 0x0C11
#define GL_ALWAYS 0x0207
#define GL_KEEP 0x1E00
#define GL_FRONT 0x0404
#define GL_BACK 0x0405
#define GL_FRONT_AND_BACK 0x0408
#define GL_VERSION 0x1F02
#define GL_VENDOR 0x1F00
#define GL_RENDERER 0x1F01
#define GL_EXTENSIONS 0x1F03
#define GL_SHADING_LANGUAGE_VERSION 0x8B8C
#define GL_NUM_EXTENSIONS 0x821D
#define GL_MAX_TEXTURE_SIZE 0x0D33
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_TEXTURE0 0x84C0
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_STENCIL_BUFFER_BIT 0x0400
#define GL_RGBA 0x1908
#define GL_RGB 0x1907
#define GL_LUMINANCE_ALPHA 0x190A

/* ---- the recorder's state -------------------------------------------------- */

#define MAXOBJ 65536
struct shader { GLenum type; char *src; };
struct program {
	GLuint fs;
	int type;      /* femtovg ShaderType (SELECT_SHADER), -1 if unknown */
	bool glyph;    /* ENABLE_GLYPH_TEXTURE */
	bool discard;  /* not NO_DISCARD, and a type with the stroke test */
};
struct texture { uint32_t w, h; };

static struct shader shaders[MAXOBJ];
static struct program programs[MAXOBJ];
static struct texture textures[MAXOBJ];
static GLuint next_name = 1;

static GLuint new_name(void)
{
	if (next_name >= MAXOBJ) {
		fprintf(stderr, "slint_gl_rec: more than %u GL objects\n", MAXOBJ);
		exit(2);
	}
	return next_name++;
}

static struct {
	GLuint program, fb, unit_tex[2], active;
	bool blend, stencil, scissor, cull, depth;
	GLenum sfunc[2], sfail[2], szfail[2], szpass[2];
	GLuint swmask;
	bool cmask;
} st = { .sfunc = { GL_ALWAYS, GL_ALWAYS }, .sfail = { GL_KEEP, GL_KEEP },
	 .szfail = { GL_KEEP, GL_KEEP }, .szpass = { GL_KEEP, GL_KEEP }, .swmask = 0xff, .cmask = true };

/* ---- per-frame counters ------------------------------------------------------ */

static const char *const type_name[10] = {
	"FillGradient", "FillImage", "Stencil", "FillImageGradient", "FilterImage",
	"FillColor", "TextureCopy", "?7", "FillGradientConic", "FillImageGradientConic",
};

struct frame_count {
	uint32_t draws, verts, by_type[10], tex_draws, glyph_draws, discard_draws, stencil_test_draws,
		stencil_write_draws, clears, clears_scissored, clears_stencil, fb_binds,
		job_ends, tex_images, tex_subs, tex_subs_partial, buf_uploads, mask_off_draws;
	uint64_t tex_bytes, sub_bytes, buf_bytes;
	uint32_t tex_seen[256], ntex_seen; /* distinct textures sampled */
	uint32_t upload_w[64], upload_h[64], nupload;
	/* a stencil-reading draw after a job end, with no stencil clear since a
	 * draw wrote the stencil: what depth/stencil between jobs must carry */
	uint32_t stencil_read_after_split;
};
static struct frame_count fc;
static uint64_t all_subs, all_subs_partial; /* the whole run */
static bool stencil_written_since_clear, split_since_write;
static FILE *draw_log;
static uint32_t frame_no;
uint32_t frame_no_for_digest(void);
uint32_t frame_no_for_digest(void) { return frame_no; }

static void seen_tex(GLuint t)
{
	for (uint32_t i = 0; i < fc.ntex_seen; i++)
		if (fc.tex_seen[i] == t)
			return;
	if (fc.ntex_seen < 256)
		fc.tex_seen[fc.ntex_seen++] = t;
}

static void job_end(const char *why)
{
	fc.job_ends++;
	if (stencil_written_since_clear)
		split_since_write = true;
	if (draw_log)
		fprintf(draw_log, "%u job-end %s\n", frame_no, why);
}

/* ---- the GL entry points ------------------------------------------------------ */

static GLenum r_GetError(void) { return GL_NO_ERROR; }
static const GLubyte *r_GetString(GLenum n)
{
	switch (n) {
	case GL_VERSION: return (const GLubyte *)"OpenGL ES 3.0 YakoGL"; /* what the board reports */
	case GL_RENDERER: return (const GLubyte *)"slint_gl_rec";
	case GL_VENDOR: return (const GLubyte *)"host";
	case GL_SHADING_LANGUAGE_VERSION: return (const GLubyte *)"OpenGL ES GLSL ES 3.00";
	case GL_EXTENSIONS: return (const GLubyte *)"";
	default: return (const GLubyte *)"";
	}
}
static const GLubyte *r_GetStringi(GLenum n, GLuint i) { (void)n; (void)i; return (const GLubyte *)""; }
static void r_GetIntegerv(GLenum n, GLint *v) { *v = n == GL_MAX_TEXTURE_SIZE ? 4096 : 0; }
static GLuint r_CreateShader(GLenum t)
{
	GLuint id = new_name();
	shaders[id].type = t;
	return id;
}
static void r_ShaderSource(GLuint s, GLsizei n, const GLchar *const *str, const GLint *len)
{
	size_t total = 0;
	for (GLsizei i = 0; i < n; i++)
		total += len && len[i] >= 0 ? (size_t)len[i] : strlen(str[i]);
	char *p = malloc(total + 1);
	size_t o = 0;
	for (GLsizei i = 0; i < n; i++) {
		size_t l = len && len[i] >= 0 ? (size_t)len[i] : strlen(str[i]);
		memcpy(p + o, str[i], l);
		o += l;
	}
	p[o] = 0;
	free(shaders[s].src);
	shaders[s].src = p;
}
static void r_CompileShader(GLuint s) { (void)s; }
static void r_GetShaderiv(GLuint s, GLenum n, GLint *v) { (void)s; *v = n == GL_COMPILE_STATUS ? 1 : 0; }
static void r_GetShaderInfoLog(GLuint s, GLsizei n, GLsizei *l, GLchar *log)
{
	(void)s;
	if (n > 0)
		log[0] = 0;
	if (l)
		*l = 0;
}
static void r_DeleteShader(GLuint s) { (void)s; }
static GLuint r_CreateProgram(void)
{
	GLuint id = new_name();
	programs[id].type = -1;
	return id;
}
static void r_AttachShader(GLuint p, GLuint s)
{
	if (shaders[s].type == GL_FRAGMENT_SHADER)
		programs[p].fs = s;
}
static void r_DetachShader(GLuint p, GLuint s) { (void)p; (void)s; }
static void r_BindAttribLocation(GLuint p, GLuint i, const GLchar *n) { (void)p; (void)i; (void)n; }
static void r_LinkProgram(GLuint p)
{
	const char *src = shaders[programs[p].fs].src;
	const char *d = src ? strstr(src, "#define SELECT_SHADER ") : NULL;
	programs[p].type = d ? atoi(d + strlen("#define SELECT_SHADER ")) : -1;
	programs[p].glyph = src && strstr(src, "#define ENABLE_GLYPH_TEXTURE");
	/* main-fs.glsl: the stroke-threshold discard unless NO_DISCARD, and
	 * never in TextureCopyUnclipped (SELECT_SHADER 6) */
	programs[p].discard = src && !strstr(src, "#define NO_DISCARD") && programs[p].type != 6;
}
static void r_GetProgramiv(GLuint p, GLenum n, GLint *v) { (void)p; *v = n == GL_LINK_STATUS ? 1 : 0; }
static void r_GetProgramInfoLog(GLuint p, GLsizei n, GLsizei *l, GLchar *log) { r_GetShaderInfoLog(p, n, l, log); }
static void r_DeleteProgram(GLuint p) { (void)p; }
static void r_UseProgram(GLuint p) { st.program = p; }
static GLint r_GetUniformLocation(GLuint p, const GLchar *n) { (void)p; return (GLint)(strlen(n) & 0xff); }
static void r_Uniform1i(GLint l, GLint v) { (void)l; (void)v; }
static void r_Uniform1f(GLint l, GLfloat v) { (void)l; (void)v; }
static void r_Uniform2fv(GLint l, GLsizei n, const GLfloat *v) { (void)l; (void)n; (void)v; }
/* The reference frame's digest: FNV-1a over everything it hands the GL that
 * decides pixels (vertex bytes, uniform values, texture data): two runs with
 * the same digest handed the GL the same frame. */
static uint64_t digest = 0xcbf29ce484222325ull;
static uint32_t digest_frame = 1;
static void dig(const void *p, size_t n)
{
	extern uint32_t frame_no_for_digest(void);
	if (frame_no_for_digest() != digest_frame || !p)
		return;
	const uint8_t *b = p;
	for (size_t i = 0; i < n; i++)
		digest = (digest ^ b[i]) * 0x100000001b3ull;
}
/* femtovg's frag[14] per program (glGetUniformLocation above answers
 * strlen(name), so "frag" is location 4) */
static float frag_of[MAXOBJ][14 * 4];
static void r_Uniform4fv(GLint l, GLsizei n, const GLfloat *v)
{
	dig(v, (size_t)n * 16);
	if (l == 4 && n == 14 && st.program < MAXOBJ)
		memcpy(frag_of[st.program], v, sizeof(frag_of[0]));
}
static void r_GenObjects(GLsizei n, GLuint *out)
{
	for (GLsizei i = 0; i < n; i++)
		out[i] = new_name();
}
static void r_DeleteObjects(GLsizei n, const GLuint *ids) { (void)n; (void)ids; }
static void r_BindVertexArray(GLuint a) { (void)a; }
static void r_BindBuffer(GLenum t, GLuint b) { (void)t; (void)b; }
/* the ARRAY_BUFFER's contents, for the coverage probe: femtovg uploads all
 * vertices of a frame in one glBufferData (x, y, u, v floats) */
static float *vbuf;
static size_t vbuf_floats;
static void r_BufferData(GLenum t, GLsizeiptr n, const void *d, GLenum u)
{
	(void)u;
	fc.buf_uploads++;
	fc.buf_bytes += (uint64_t)n;
	dig(d, (size_t)n);
	if (t == 0x8892 /* GL_ARRAY_BUFFER */ && d) {
		free(vbuf);
		vbuf = malloc((size_t)n);
		memcpy(vbuf, d, (size_t)n);
		vbuf_floats = (size_t)n / sizeof(float);
	}
}
static void r_BufferSubData(GLenum t, GLintptr o, GLsizeiptr n, const void *d)
{
	(void)t; (void)o; (void)d;
	fc.buf_uploads++;
	fc.buf_bytes += (uint64_t)n;
}
static void r_VertexAttribPointer(GLuint i, GLint s, GLenum t, GLboolean nm, GLsizei stride, const void *p)
{
	(void)i; (void)s; (void)t; (void)nm; (void)stride; (void)p;
}
static void r_EnableVertexAttribArray(GLuint i) { (void)i; }
static void r_ActiveTexture(GLenum u) { st.active = u - GL_TEXTURE0; }
static void r_BindTexture(GLenum t, GLuint id)
{
	(void)t;
	if (st.active < 2)
		st.unit_tex[st.active] = id;
}
static uint32_t bpp_of(GLenum f) { return f == GL_RGBA ? 4 : f == GL_RGB ? 3 : f == GL_LUMINANCE_ALPHA ? 2 : 1; }
static void r_TexImage2D(GLenum t, GLint l, GLint ifmt, GLsizei w, GLsizei h, GLint b, GLenum f, GLenum ty,
			 const void *px)
{
	(void)t; (void)ifmt; (void)b; (void)ty; (void)px;
	GLuint id = st.active < 2 ? st.unit_tex[st.active] : 0;
	if (l == 0 && id < MAXOBJ)
		textures[id] = (struct texture){ .w = (uint32_t)w, .h = (uint32_t)h };
	fc.tex_images++;
	fc.tex_bytes += (uint64_t)w * (uint64_t)h * bpp_of(f);
	if (fc.nupload < 64) {
		fc.upload_w[fc.nupload] = (uint32_t)w;
		fc.upload_h[fc.nupload++] = (uint32_t)h;
	}
}
static void r_TexSubImage2D(GLenum t, GLint l, GLint x, GLint y, GLsizei w, GLsizei h, GLenum f, GLenum ty,
			    const void *px)
{
	(void)t; (void)ty; (void)px;
	GLuint id = st.active < 2 ? st.unit_tex[st.active] : 0;
	/* a sub-box, not the whole level-0 image */
	bool partial = l != 0 || x != 0 || y != 0 || id >= MAXOBJ || (uint32_t)w != textures[id].w ||
		       (uint32_t)h != textures[id].h;
	fc.tex_subs++;
	fc.tex_subs_partial += partial;
	all_subs++;
	all_subs_partial += partial;
	fc.sub_bytes += (uint64_t)w * (uint64_t)h * bpp_of(f);
	dig(px, (size_t)w * (size_t)h * bpp_of(f));
}
static void r_TexParameteri(GLenum t, GLenum n, GLint v) { (void)t; (void)n; (void)v; }
static void r_PixelStorei(GLenum n, GLint v) { (void)n; (void)v; }
static void r_GenerateMipmap(GLenum t) { (void)t; }
static void r_BindFramebuffer(GLenum t, GLuint f)
{
	(void)t;
	if (f != st.fb) {
		fc.fb_binds++;
		job_end("target switch");
	}
	st.fb = f;
}
static void r_FramebufferTexture2D(GLenum t, GLenum a, GLenum tt, GLuint tex, GLint l)
{
	(void)t; (void)a; (void)tt; (void)tex; (void)l;
}
static void r_FramebufferRenderbuffer(GLenum t, GLenum a, GLenum rt, GLuint rb) { (void)t; (void)a; (void)rt; (void)rb; }
static void r_BindRenderbuffer(GLenum t, GLuint r) { (void)t; (void)r; }
static void r_RenderbufferStorage(GLenum t, GLenum f, GLsizei w, GLsizei h) { (void)t; (void)f; (void)w; (void)h; }
static GLenum r_CheckFramebufferStatus(GLenum t) { (void)t; return GL_FRAMEBUFFER_COMPLETE; }
static void set_cap(GLenum c, bool on)
{
	switch (c) {
	case GL_BLEND: st.blend = on; break;
	case GL_STENCIL_TEST: st.stencil = on; break;
	case GL_SCISSOR_TEST: st.scissor = on; break;
	case GL_CULL_FACE: st.cull = on; break;
	case GL_DEPTH_TEST: st.depth = on; break;
	default: break;
	}
}
static void r_Enable(GLenum c) { set_cap(c, true); }
static void r_Disable(GLenum c) { set_cap(c, false); }
static void r_BlendFuncSeparate(GLenum a, GLenum b, GLenum c, GLenum d) { (void)a; (void)b; (void)c; (void)d; }
static void r_BlendFunc(GLenum a, GLenum b) { (void)a; (void)b; }
static void r_ColorMask(GLboolean r, GLboolean g, GLboolean b, GLboolean a) { st.cmask = r || g || b || a; }
static void r_StencilMask(GLuint m) { st.swmask = m; }
static void r_StencilFunc(GLenum f, GLint r, GLuint m)
{
	(void)r; (void)m;
	st.sfunc[0] = st.sfunc[1] = f;
}
static void r_StencilOp(GLenum a, GLenum b, GLenum c)
{
	st.sfail[0] = st.sfail[1] = a;
	st.szfail[0] = st.szfail[1] = b;
	st.szpass[0] = st.szpass[1] = c;
}
static void r_StencilOpSeparate(GLenum face, GLenum a, GLenum b, GLenum c)
{
	for (int i = 0; i < 2; i++) {
		if (face == GL_FRONT_AND_BACK || face == (i ? GL_BACK : GL_FRONT)) {
			st.sfail[i] = a;
			st.szfail[i] = b;
			st.szpass[i] = c;
		}
	}
}
static void r_CullFace(GLenum m) { (void)m; }
static void r_FrontFace(GLenum m) { (void)m; }
static void r_Scissor(GLint x, GLint y, GLsizei w, GLsizei h) { (void)x; (void)y; (void)w; (void)h; }
static void r_Viewport(GLint x, GLint y, GLsizei w, GLsizei h) { (void)x; (void)y; (void)w; (void)h; }
static void r_ClearColor(GLfloat r, GLfloat g, GLfloat b, GLfloat a) { (void)r; (void)g; (void)b; (void)a; }
static void r_ClearStencil(GLint s) { (void)s; }
static void r_Clear(GLbitfield b)
{
	fc.clears++;
	fc.clears_scissored += st.scissor;
	if (b & GL_STENCIL_BUFFER_BIT) {
		fc.clears_stencil++;
		stencil_written_since_clear = false;
		split_since_write = false;
	}
	if (draw_log)
		fprintf(draw_log, "%u clear%s%s%s fb=%u\n", frame_no, b & GL_COLOR_BUFFER_BIT ? " colour" : "",
			b & GL_STENCIL_BUFFER_BIT ? " stencil" : "", st.scissor ? " scissored" : "", st.fb);
}
/* The coverage probe: pixel centres (canvas coordinates, y down, which is
 * the window's row order) and the frame whose draws are tested. */
#define MAXPROBE 8
static float probe_xy[MAXPROBE][2];
static int nprobe;
static uint32_t probe_frame = 1;
static uint32_t draw_no;

static float edge(const float *a, const float *b, float px, float py)
{
	return (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
}
static bool tri_covers(const float *a, const float *b, const float *c, float px, float py)
{
	float e0 = edge(a, b, px, py), e1 = edge(b, c, px, py), e2 = edge(c, a, px, py);
	return (e0 >= 0 && e1 >= 0 && e2 >= 0) || (e0 <= 0 && e1 <= 0 && e2 <= 0);
}

static void probe_draw(GLenum mode, GLint first, GLsizei n, const char *what, bool s_read, bool s_write)
{
	if (frame_no != probe_frame || !vbuf)
		return;
	for (int k = 0; k < nprobe; k++) {
		float px = probe_xy[k][0], py = probe_xy[k][1];
		uint32_t hits = 0;
		for (GLsizei i = 0; i + 2 < n; i += mode == GL_TRIANGLES ? 3 : 1) {
			size_t a = (size_t)(first + i) * 4, b = a + 4, c = a + 8;
			if (c + 1 >= vbuf_floats)
				break;
			if (tri_covers(&vbuf[a], &vbuf[b], &vbuf[c], px, py))
				hits++;
		}
		if (!hits)
			continue;
		const float *fr = frag_of[st.program];
		printf("probe (%.1f,%.1f) draw %u: %s %s, %u triangles cover it; innerCol %.0f,%.0f,%.0f,%.2f; "
		       "strokeMult %.2f strokeThr %.3f; stencil %s%s; cmask %d blend %d\n",
		       px, py, draw_no, what, mode == GL_TRIANGLES ? "tris" : "strip", hits, fr[24] * 255, fr[25] * 255,
		       fr[26] * 255, fr[27], fr[40], fr[41], s_read ? "read" : st.stencil ? "on" : "off",
		       s_write ? "+write" : "", st.cmask, st.blend);
	}
}

static void draw(GLenum mode, GLint first, GLsizei n)
{
	const struct program *p = &programs[st.program];
	int t = p->type;
	/* main-fs.glsl: the types that call texture2D(tex, ...) */
	bool samples_tex = t == 1 || t == 3 || t == 4 || t == 6 || t == 9;
	GLuint tex = samples_tex ? st.unit_tex[0] : 0, gtex = p->glyph ? st.unit_tex[1] : 0;
	bool s_read = st.stencil && (st.sfunc[0] != GL_ALWAYS || st.sfunc[1] != GL_ALWAYS);
	bool s_write = st.stencil && st.swmask &&
		       (st.sfail[0] != GL_KEEP || st.szfail[0] != GL_KEEP || st.szpass[0] != GL_KEEP ||
			st.sfail[1] != GL_KEEP || st.szfail[1] != GL_KEEP || st.szpass[1] != GL_KEEP);

	fc.draws++;
	fc.verts += (uint32_t)n;
	if (t >= 0 && t < 10)
		fc.by_type[t]++;
	if (tex) {
		fc.tex_draws++;
		seen_tex(tex);
	}
	if (gtex) {
		fc.glyph_draws++;
		seen_tex(gtex);
	}
	fc.discard_draws += p->discard;
	fc.stencil_test_draws += st.stencil;
	fc.stencil_write_draws += s_write;
	fc.mask_off_draws += !st.cmask;
	if (s_read && split_since_write)
		fc.stencil_read_after_split++;
	if (s_write)
		stencil_written_since_clear = true;
	draw_no++;
	{
		char what[64];
		snprintf(what, sizeof(what), "%s%s%s", t >= 0 && t < 10 ? type_name[t] : "?", p->glyph ? "+glyph" : "",
			 p->discard ? "+discard" : "");
		probe_draw(mode, first, n, what, s_read, s_write);
	}
	if (draw_log)
		fprintf(draw_log, "%u draw fb=%u %s%s%s %s n=%d tex=%u(%ux%u) glyph=%u stencil=%s%s cmask=%d blend=%d scissor=%d\n",
			frame_no, st.fb, t >= 0 && t < 10 ? type_name[t] : "?", p->glyph ? "+glyph" : "",
			p->discard ? "+discard" : "", mode == GL_TRIANGLES ? "tris" : mode == GL_TRIANGLE_STRIP ? "strip" : "other",
			n, tex, tex ? textures[tex].w : 0, tex ? textures[tex].h : 0, gtex,
			s_read ? "read" : st.stencil ? "on" : "off", s_write ? "+write" : "", st.cmask, st.blend, st.scissor);
}
static void r_DrawArrays(GLenum m, GLint f, GLsizei n) { draw(m, f, n); }
static void r_DrawElements(GLenum m, GLsizei n, GLenum t, const void *i) { (void)t; (void)i; draw(m, 0, n); }
static void r_Flush(void) { job_end("glFlush"); }
static void r_Finish(void) { job_end("glFinish"); }
static void r_ReadPixels(GLint x, GLint y, GLsizei w, GLsizei h, GLenum f, GLenum t, void *d)
{
	(void)x; (void)y; (void)f; (void)t;
	memset(d, 0, (size_t)w * (size_t)h * 4);
	job_end("glReadPixels");
}
static void r_Hint(GLenum a, GLenum b) { (void)a; (void)b; }

#define E(n) { "gl" #n, (void *)r_##n }
static const struct { const char *name; void *fn; } entries[] = {
	E(GetError), E(GetString), E(GetStringi), E(GetIntegerv), E(CreateShader), E(ShaderSource),
	E(CompileShader), E(GetShaderiv), E(GetShaderInfoLog), E(DeleteShader), E(CreateProgram),
	E(AttachShader), E(DetachShader), E(BindAttribLocation), E(LinkProgram), E(GetProgramiv),
	E(GetProgramInfoLog), E(DeleteProgram), E(UseProgram), E(GetUniformLocation), E(Uniform1i),
	E(Uniform1f), E(Uniform2fv), E(Uniform4fv), E(BindVertexArray), E(BindBuffer), E(BufferData),
	E(BufferSubData), E(VertexAttribPointer), E(EnableVertexAttribArray), E(ActiveTexture),
	E(BindTexture), E(TexImage2D), E(TexSubImage2D), E(TexParameteri), E(PixelStorei),
	E(GenerateMipmap), E(BindFramebuffer), E(FramebufferTexture2D), E(FramebufferRenderbuffer),
	E(BindRenderbuffer), E(RenderbufferStorage), E(CheckFramebufferStatus), E(Enable), E(Disable),
	E(BlendFuncSeparate), E(BlendFunc), E(ColorMask), E(StencilMask), E(StencilFunc), E(StencilOp),
	E(StencilOpSeparate), E(CullFace), E(FrontFace), E(Scissor), E(Viewport), E(ClearColor),
	E(ClearStencil), E(Clear), E(DrawArrays), E(DrawElements), E(Flush), E(Finish), E(ReadPixels),
	E(Hint),
	{ "glGenBuffers", (void *)r_GenObjects }, { "glGenTextures", (void *)r_GenObjects },
	{ "glGenVertexArrays", (void *)r_GenObjects }, { "glGenFramebuffers", (void *)r_GenObjects },
	{ "glGenRenderbuffers", (void *)r_GenObjects },
	{ "glDeleteBuffers", (void *)r_DeleteObjects }, { "glDeleteVertexArrays", (void *)r_DeleteObjects },
	{ "glDeleteFramebuffers", (void *)r_DeleteObjects }, { "glDeleteRenderbuffers", (void *)r_DeleteObjects },
	{ "glDeleteTextures", (void *)r_DeleteObjects },
	{ "glDisableVertexAttribArray", (void *)r_EnableVertexAttribArray },
};

/* Names Slint asked for that the recorder does not have, printed at the end.
 * glow resolves every name at context creation; a call to an unresolved one
 * panics in glow, so a run that finishes called none of them. */
static char missing[512][48];
static int nmissing;

static const void *get_proc(const char *name)
{
	for (size_t i = 0; i < sizeof(entries) / sizeof(entries[0]); i++)
		if (strcmp(name, entries[i].name) == 0)
			return entries[i].fn;
	if (nmissing < 512)
		snprintf(missing[nmissing++], sizeof(missing[0]), "%s", name);
	return NULL;
}

/* ---- what the sample's archive needs from C ------------------------------------- */

void *slint_heap_alloc(size_t alignment, size_t size);
void slint_heap_free(void *p);
int printk(const char *fmt, ...);

void *slint_heap_alloc(size_t alignment, size_t size)
{
	void *p = NULL;
	if (alignment < sizeof(void *))
		alignment = sizeof(void *);
	return posix_memalign(&p, alignment, size ? size : 1) ? NULL : p;
}
void slint_heap_free(void *p) { free(p); }
static void print_frame(const char *what, int rc);
static void frame1_done(void);

/* The crate prints each console line as printk("%s", line). Its "slint:
 * frame 1 (" line comes between the two draws of slint_sample_first_frame:
 * that is where frame 1's counts end and frame 2's begin. */
int printk(const char *fmt, ...)
{
	char buf[1024];
	va_list ap;
	va_start(ap, fmt);
	int n = vsnprintf(buf, sizeof(buf), fmt, ap);
	va_end(ap);
	fputs(buf, stdout);
	if (strncmp(buf, "slint: frame 1 (", 16) == 0)
		frame1_done();
	return n;
}

int slint_sample_first_frame(const void *(*get_proc)(const char *name), uint32_t width, uint32_t height);
int slint_sample_animate_frame(const void *(*get_proc)(const char *name), uint32_t width, uint32_t height,
			       uint32_t t_ms, uint32_t frame);

/* ---- the run ------------------------------------------------------------------ */

static void print_frame(const char *what, int rc)
{
	printf("%s: rc %d\n", what, rc);
	printf("  %u draws (%u vertices), %u with discard, %u with the colour mask off; by program:", fc.draws,
	       fc.verts, fc.discard_draws, fc.mask_off_draws);
	for (int t = 0; t < 10; t++)
		if (fc.by_type[t])
			printf(" %s %u", type_name[t], fc.by_type[t]);
	printf("\n  textures sampled: %u draws through tex, %u through glyphtex, %u distinct textures\n",
	       fc.tex_draws, fc.glyph_draws, fc.ntex_seen);
	printf("  stencil: %u draws with the test on, %u write it; %u stencil reads after a job end "
	       "with no stencil clear since the write\n",
	       fc.stencil_test_draws, fc.stencil_write_draws, fc.stencil_read_after_split);
	printf("  clears: %u (%u scissored, %u with stencil); target switches %u; job ends %u\n", fc.clears,
	       fc.clears_scissored, fc.clears_stencil, fc.fb_binds, fc.job_ends);
	printf("  uploads: %u glTexImage2D (%llu bytes), %u glTexSubImage2D (%llu bytes), %u buffer (%llu bytes)\n",
	       fc.tex_images, (unsigned long long)fc.tex_bytes, fc.tex_subs, (unsigned long long)fc.sub_bytes,
	       fc.buf_uploads, (unsigned long long)fc.buf_bytes);
	if (fc.nupload) {
		printf("  new textures:");
		for (uint32_t i = 0; i < fc.nupload; i++)
			printf(" %ux%u", fc.upload_w[i], fc.upload_h[i]);
		printf("\n");
	}
}

static int frame1_seen;

/* frame 1 ended (the crate's line): its counts, then frame 2's from zero */
static void frame1_done(void)
{
	frame1_seen = 1;
	print_frame("frame 1 (build + default state)", 0);
	frame_no = 1;
	draw_no = 0;
	memset(&fc, 0, sizeof(fc));
}

int main(int argc, char **argv)
{
	draw_log = fopen(argc > 1 ? argv[1] : "slint_draws.log", "w");
	/* probes: "x,y" pixel coordinates after the log file; each is tested at
	 * its pixel centre in the reference frame (frame 2) */
	for (int a = 2; a < argc && nprobe < MAXPROBE; a++) {
		int x, y;
		if (sscanf(argv[a], "%d,%d", &x, &y) == 2) {
			probe_xy[nprobe][0] = (float)x + 0.5f;
			probe_xy[nprobe][1] = (float)y + 0.5f;
			nprobe++;
		}
	}
	int rc;

	/* the first frame, as the sample's main.c draws it: frame 1 (build and
	 * default state, clock 0) and frame 2 (demo_vehicle t = 0, clock
	 * FRAME2_CLOCK_MS) in one call, split at the crate's "frame 1" line */
	memset(&fc, 0, sizeof(fc));
	rc = slint_sample_first_frame(get_proc, 1024, 600);
	r_Flush(); /* eglSwapBuffers ends the job */
	if (!frame1_seen) {
		print_frame("first frame: the crate printed no frame 1 line (it failed before)", rc);
		return 1;
	}
	print_frame("frame 2 (demo_vehicle t=0: the AM62P's reference frame)", rc);
	printf("frame 2 digest 0x%016llx\n", (unsigned long long)digest);
	if (getenv("SLINT_REC_REF_ONLY"))
		return 0; /* only the reference frame */

	/* the drive, at 58 fps: per phase, the per-frame spread */
	static const struct { const char *what; int64_t t0, t1; } phases[] = {
		{ "idle (0-3 s)", 0, 3000 },
		{ "accelerate (3-15 s)", 3000, 15000 },
		{ "cruise (15-22 s)", 15000, 22000 },
	};
	uint32_t frame = 2, total = 0;
	for (size_t ph = 0; ph < sizeof(phases) / sizeof(phases[0]); ph++) {
		uint32_t n = 0, dmin = UINT32_MAX, dmax = 0, umax = 0, frames_with_upload = 0, sras = 0, jmax = 0;
		uint64_t dsum = 0, bsum = 0, bmax = 0, usum = 0, tsum = 0;
		for (int64_t t = phases[ph].t0; t < phases[ph].t1; t += 1000 / 58) {
			frame_no = frame;
			memset(&fc, 0, sizeof(fc));
			rc = slint_sample_animate_frame(get_proc, 1024, 600, (uint32_t)t, frame++);
			r_Flush();
			if (rc) {
				print_frame("drive frame failed", rc);
				return 1;
			}
			n++;
			dsum += fc.draws;
			dmin = fc.draws < dmin ? fc.draws : dmin;
			dmax = fc.draws > dmax ? fc.draws : dmax;
			uint64_t b = fc.tex_bytes + fc.sub_bytes;
			bsum += b;
			bmax = b > bmax ? b : bmax;
			usum += fc.tex_images;
			umax = fc.tex_images > umax ? fc.tex_images : umax;
			frames_with_upload += fc.tex_images != 0;
			sras += fc.stencil_read_after_split;
			tsum += fc.ntex_seen;
			jmax = fc.job_ends > jmax ? fc.job_ends : jmax;
		}
		total += n;
		printf("drive %s: %u frames; draws %u..%u (mean %.1f); distinct textures sampled per frame mean %.1f; "
		       "new textures per frame mean %.2f, max %u, in %u of %u frames; texture bytes per frame mean %.0f, "
		       "max %llu (%.0f per second at 58 fps); job ends per frame max %u; stencil reads across a job end %u\n",
		       phases[ph].what, n, dmin, dmax, (double)dsum / n, (double)tsum / n, (double)usum / n, umax,
		       frames_with_upload, n, (double)bsum / n, (unsigned long long)bmax, (double)bsum / n * 58.0, jmax,
		       sras);
	}
	printf("drive frames in all: %u\n", total);
	printf("glTexSubImage2D in the whole run: %llu, of which %llu a sub-box (not the whole texture)\n",
	       (unsigned long long)all_subs, (unsigned long long)all_subs_partial);
	printf("names Slint asked for that the recorder does not have: %d\n", nmissing);
	for (int i = 0; i < nmissing; i++)
		printf("  %s\n", missing[i]);
	if (draw_log)
		fclose(draw_log);
	return 0;
}
